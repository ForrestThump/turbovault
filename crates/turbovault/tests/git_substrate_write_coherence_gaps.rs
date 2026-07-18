//! Write-coherence gaps between the git substrate and the *other* write paths on a
//! `write_backend = Git` vault.
//!
//! `turbovault-git`'s own module docs state the intent plainly: *"This crate replaces the legacy
//! `VaultManager` write path."* The substrate's truth is the commit graph; the working tree is a
//! materialized *view* of HEAD, and commits are built by plumbing from an explicit tree (HEAD's tree
//! + the changeset), never by staging the working tree. Materialization then resyncs the working
//! tree to HEAD.
//!
//! The gap these tests document: on a git-backed vault **nothing prevents a non-substrate write to
//! the working tree** — neither the still-public `VaultManager::write_file` (the legacy path, which
//! `crates/vault`-style in-process callers use) nor a raw human edit in Obsidian. Such a write:
//!
//!   1. **escapes git history** — it produces no commit, so the substrate's audit/rollback log has
//!      no record of it, and
//!   2. **wedges the substrate** — because the working tree now differs from HEAD, the *next*
//!      substrate write to that path aborts with `"differs from HEAD"`, with no ingestion path to
//!      reconcile the stray change into a commit first.
//!
//! Both are demonstrated below against the real APIs: the substrate (`VaultRepo` + `Changeset`) is
//! the git writer; `VaultManager::write_file` is the legacy writer; `std::fs` stands in for Obsidian.
//! The tests **pass** — they assert the current (hazardous) behavior, as a reproduction to attach to
//! the issue, not the desired behavior.

use std::path::Path;

use serial_test::serial;
use tempfile::TempDir;
use turbovault_core::config::{ServerConfig, VaultConfig, VaultGitConfig, WriteBackend};
use turbovault_git::{Changeset, VaultRepo};
use turbovault_vault::VaultManager;

/// A real git repo with a born (non-empty-history) HEAD — the substrate needs a baseline commit.
fn init_git_vault() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let mut opts = git2::RepositoryInitOptions::new();
    opts.initial_head("main");
    let repo = git2::Repository::init_opts(tmp.path(), &opts).unwrap();
    let tree_oid = {
        let mut idx = repo.index().unwrap();
        idx.write_tree().unwrap()
    };
    let tree = repo.find_tree(tree_oid).unwrap();
    let sig = git2::Signature::now("Init", "init@example").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    tmp
}

/// A `VaultManager` bound to `root` and *configured as a git vault* — the exact shape an in-process
/// caller (e.g. the daemon's `liberado-vault` adapter) holds. `write_backend = Git` is set, yet
/// `write_file` still writes the filesystem directly: there is no guard.
fn git_vault_manager(root: &Path) -> VaultManager {
    let mut config = ServerConfig::new();
    config.vaults.push(
        VaultConfig::builder("gap", root)
            .as_default()
            .write_backend(WriteBackend::Git)
            .git(VaultGitConfig::default())
            .build()
            .unwrap(),
    );
    VaultManager::new(config).unwrap()
}

fn head_oid(root: &Path) -> Option<git2::Oid> {
    VaultRepo::open(root).ok().and_then(|r| r.head_oid())
}

/// The content git *believes* is at `rel` (the blob in HEAD's tree), independent of the working tree.
fn head_blob(root: &Path, rel: &str) -> Option<String> {
    let repo = git2::Repository::open(root).ok()?;
    let commit = repo.head().ok()?.peel_to_commit().ok()?;
    let tree = commit.tree().ok()?;
    let entry = tree.get_path(Path::new(rel)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(String::from_utf8_lossy(blob.content()).into_owned())
}

fn worktree(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

/// GAP 1 — the legacy `VaultManager::write_file` path is reachable on a `write_backend = Git` vault,
/// silently escapes git history, and wedges the substrate.
#[tokio::test]
#[serial]
async fn vault_manager_write_on_git_vault_escapes_git_and_wedges_the_substrate() {
    let tmp = init_git_vault();
    let root = tmp.path();

    // Baseline: one note, committed through the substrate (the real git writer).
    let repo = VaultRepo::open(root).unwrap();
    let seed = repo
        .commit_changeset(&Changeset::new("seed note").create("note.md", "committed"))
        .unwrap();
    assert_eq!(head_oid(root), Some(seed.commit));
    assert_eq!(head_blob(root, "note.md").as_deref(), Some("committed"));
    assert_eq!(worktree(root, "note.md").as_deref(), Some("committed"));

    // A legacy write to the SAME git-backed vault. No error, no guard — write_file just writes the
    // filesystem (+ optional audit), never touching the substrate.
    let manager = git_vault_manager(root);
    manager
        .write_file(Path::new("note.md"), "legacy overwrite", None)
        .await
        .expect("legacy write_file succeeds unguarded on a git vault");

    // The bytes changed on disk...
    assert_eq!(worktree(root, "note.md").as_deref(), Some("legacy overwrite"));
    // ...but produced NO commit: the write is invisible to git history / audit / rollback.
    assert_eq!(
        head_oid(root),
        Some(seed.commit),
        "GAP: legacy write produced no commit — it escaped git history"
    );
    // ...and git's truth is unchanged, so the working tree now diverges from HEAD.
    assert_eq!(
        head_blob(root, "note.md").as_deref(),
        Some("committed"),
        "GAP: HEAD still holds the pre-legacy content; working tree silently diverged"
    );

    // GAP 2: the escaped write now WEDGES the substrate. A subsequent substrate update to the note
    // aborts on working-tree drift — even though the caller's git pre-image (the committed blob) is
    // still valid — and there is no ingestion path to reconcile the stray change first.
    let committed_blob = VaultRepo::blob_oid_of(b"committed").unwrap();
    let result =
        repo.commit_changeset(&Changeset::new("agent update").update("note.md", "v2", committed_blob));
    let err = result.expect_err("GAP: substrate write must abort after the working tree drifted");
    assert!(
        format!("{err}").contains("differs from HEAD"),
        "aborts with a working-tree-drift error, got: {err}"
    );
    // The agent's update never lands; HEAD is stuck at the seed commit until a human reconciles.
    assert_eq!(head_oid(root), Some(seed.commit));
}

/// GAP 3 — a raw working-tree edit (an Obsidian save) on a git-backed vault is never ingested into a
/// commit and likewise wedges the substrate. This is the human-edit reconciliation gap.
#[tokio::test]
#[serial]
async fn manual_worktree_edit_on_git_vault_is_never_ingested_and_wedges_the_substrate() {
    let tmp = init_git_vault();
    let root = tmp.path();

    let repo = VaultRepo::open(root).unwrap();
    let seed = repo
        .commit_changeset(&Changeset::new("seed").create("note.md", "committed"))
        .unwrap();

    // A human edits the note in Obsidian: a raw working-tree write, outside the substrate.
    std::fs::write(root.join("note.md"), "human edit in obsidian").unwrap();

    // The edit is never ingested: HEAD is unchanged and git history has no record of it.
    assert_eq!(
        head_oid(root),
        Some(seed.commit),
        "GAP: human edit produced no commit"
    );
    assert_eq!(
        head_blob(root, "note.md").as_deref(),
        Some("committed"),
        "GAP: git history never saw the human edit"
    );
    assert_eq!(
        worktree(root, "note.md").as_deref(),
        Some("human edit in obsidian")
    );

    // And it wedges the substrate: an agent write to the note now aborts on drift, with no path to
    // first fold the human's change into a commit.
    let committed_blob = VaultRepo::blob_oid_of(b"committed").unwrap();
    let result =
        repo.commit_changeset(&Changeset::new("agent update").update("note.md", "v2", committed_blob));
    let err = result.expect_err("GAP: substrate write must abort after an external working-tree edit");
    assert!(
        format!("{err}").contains("differs from HEAD"),
        "aborts with a working-tree-drift error, got: {err}"
    );
}
