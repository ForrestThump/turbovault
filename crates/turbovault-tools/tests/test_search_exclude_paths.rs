//! Tests for server-side `search.exclude_paths` filtering across search tools.
//!
//! Verifies that path-prefix exclusions are applied to `search`, `advanced_search`
//! (additively with the tool parameter), and `semantic_search`.

use std::sync::Arc;
use tempfile::TempDir;
use turbovault_core::{ConfigProfile, VaultConfig};
use turbovault_tools::{SearchEngine, SearchQuery, SimilarityEngine};
use turbovault_vault::VaultManager;

/// Build a vault with notes in the root, `Archive/`, `.trash/`, and `Drafts/`.
async fn setup_vault() -> (TempDir, Arc<VaultManager>) {
    let temp_dir = TempDir::new().expect("temp dir");
    let vault_path = temp_dir.path();

    std::fs::create_dir_all(vault_path.join("Archive")).unwrap();
    std::fs::create_dir_all(vault_path.join(".trash")).unwrap();
    std::fs::create_dir_all(vault_path.join("Drafts")).unwrap();

    std::fs::write(
        vault_path.join("active.md"),
        "# Active\n\nThe quick brown fox jumps over apple orchard notes.",
    )
    .unwrap();
    std::fs::write(
        vault_path.join("Archive/old.md"),
        "# Old\n\nThe quick brown fox in an archived apple orchard note.",
    )
    .unwrap();
    std::fs::write(
        vault_path.join(".trash/deleted.md"),
        "# Deleted\n\nA trashed quick brown apple orchard fox note.",
    )
    .unwrap();
    std::fs::write(
        vault_path.join("Drafts/draft.md"),
        "# Draft\n\nA draft quick brown apple orchard fox note.",
    )
    .unwrap();

    let mut config = ConfigProfile::Development.create_config();
    config
        .vaults
        .push(VaultConfig::builder("test", vault_path).build().unwrap());

    let manager = VaultManager::new(config).unwrap();
    manager.initialize().await.unwrap();

    (temp_dir, Arc::new(manager))
}

fn paths(results: &[turbovault_tools::SearchResultInfo]) -> Vec<String> {
    results.iter().map(|r| r.path.replace('\\', "/")).collect()
}

#[tokio::test]
async fn search_excludes_configured_prefixes() {
    let (_temp, manager) = setup_vault().await;
    let engine = SearchEngine::with_exclusions(
        manager,
        vec!["Archive/".to_string(), ".trash/".to_string()],
    )
    .await
    .unwrap();

    let results = engine.search("apple orchard").await.unwrap();
    let found = paths(&results);

    assert!(
        found.iter().any(|p| p.ends_with("active.md")),
        "root note should be returned, got {found:?}"
    );
    assert!(
        found.iter().any(|p| p.ends_with("Drafts/draft.md")),
        "non-excluded folder should be returned, got {found:?}"
    );
    assert!(
        !found.iter().any(|p| p.contains("/Archive/")),
        "Archive/ notes must be excluded, got {found:?}"
    );
    assert!(
        !found.iter().any(|p| p.contains("/.trash/")),
        ".trash/ notes must be excluded, got {found:?}"
    );
}

#[tokio::test]
async fn no_exclusions_returns_everything() {
    let (_temp, manager) = setup_vault().await;
    let engine = SearchEngine::new(manager).await.unwrap();

    let results = engine.search("apple orchard").await.unwrap();
    let found = paths(&results);

    assert!(found.iter().any(|p| p.contains("/Archive/")));
    assert!(found.iter().any(|p| p.contains("/.trash/")));
}

#[tokio::test]
async fn advanced_search_param_is_additive_with_config() {
    let (_temp, manager) = setup_vault().await;
    // Config excludes Archive/; the tool param additionally excludes Drafts/.
    let engine = SearchEngine::with_exclusions(manager, vec!["Archive/".to_string()])
        .await
        .unwrap();

    let query = SearchQuery::new("apple orchard")
        .exclude(vec!["Drafts/".to_string()])
        .limit(50);
    let results = engine.advanced_search(query).await.unwrap();
    let found = paths(&results);

    assert!(found.iter().any(|p| p.ends_with("active.md")));
    assert!(
        !found.iter().any(|p| p.contains("/Archive/")),
        "config exclusion must still apply, got {found:?}"
    );
    assert!(
        !found.iter().any(|p| p.contains("/Drafts/")),
        "tool-param exclusion must apply, got {found:?}"
    );
    // .trash/ was not excluded by either source, so it should appear.
    assert!(found.iter().any(|p| p.contains("/.trash/")));
}

#[tokio::test]
async fn semantic_search_excludes_configured_prefixes() {
    let (_temp, manager) = setup_vault().await;
    let engine = SimilarityEngine::with_exclusions(
        manager,
        vec!["Archive/".to_string(), ".trash/".to_string()],
    )
    .await
    .unwrap();

    let results = engine.semantic_search("quick brown apple orchard fox", 50);
    let found: Vec<String> = results
        .iter()
        .map(|r| r.path.replace('\\', "/"))
        .collect();

    assert!(
        !found.iter().any(|p| p.starts_with("Archive/")),
        "Archive/ notes must be excluded from semantic search, got {found:?}"
    );
    assert!(
        !found.iter().any(|p| p.starts_with(".trash/")),
        ".trash/ notes must be excluded from semantic search, got {found:?}"
    );
    assert!(
        found.iter().any(|p| p.starts_with("Drafts/") || p == "active.md"),
        "non-excluded notes should still rank, got {found:?}"
    );
}
