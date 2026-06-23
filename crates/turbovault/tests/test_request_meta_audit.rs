//! Integration test: the `write_note` MCP tool records request `_meta` on the audit log.
//!
//! This is the end of the provenance wire — a request's `_meta` (surfaced into the handler
//! context by the router under `REQUEST_META_KEY`) must land on the write's audit entry, so a
//! separate process (e.g. a reactive daemon) can re-open the persisted log and attribute the
//! change to its originator. We drive the real server handler and then re-open the persisted
//! JSONL audit log from disk — the same cross-instance read the production loop relies on.

use tempfile::TempDir;
use turbomcp::{McpHandler, REQUEST_META_KEY, RequestContext};
use turbovault::ObsidianMcpServer;
use turbovault_audit::{AuditFilter, AuditLog};
use turbovault_core::VaultConfig;

#[tokio::test]
async fn write_note_records_request_meta_on_the_audit_log() {
    let temp = TempDir::new().expect("temp dir");
    let vault_path = temp.path();

    let server = ObsidianMcpServer::new().expect("server");
    let config = VaultConfig::builder("test", vault_path.to_str().unwrap())
        .build()
        .expect("vault config");
    server
        .multi_vault()
        .add_vault(config)
        .await
        .expect("add vault");
    server
        .multi_vault()
        .set_active_vault("test")
        .await
        .expect("set active");

    // Provenance carried exactly as a client would send it: an opaque object under `_meta`.
    let provenance = serde_json::json!({
        "_liberado_provenance": { "source": "tasks-mcp", "correlation_id": "corr-1" }
    });
    let ctx = RequestContext::new().with_metadata(REQUEST_META_KEY, provenance.clone());

    server
        .call_tool(
            "write_note",
            serde_json::json!({ "path": "note.md", "content": "# Hello" }),
            &ctx,
        )
        .await
        .expect("write_note");

    // Re-open the persisted audit log (a fresh instance, like another process would) and confirm
    // the write carried our `_meta` through to the audit entry's metadata.
    let audit = AuditLog::new(vault_path).await.expect("open audit log");
    let entries = audit
        .query(&AuditFilter::new().with_limit(100))
        .await
        .expect("query audit");

    let entry = entries
        .iter()
        .find(|e| e.metadata.get("_liberado_provenance").is_some())
        .unwrap_or_else(|| {
            panic!(
                "no audit entry carried the request _meta; entries: {:?}",
                entries
            )
        });
    assert_eq!(
        entry.metadata, provenance,
        "audit metadata should equal the request _meta verbatim"
    );
}
