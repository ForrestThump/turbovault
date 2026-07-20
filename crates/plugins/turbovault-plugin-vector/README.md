# turbovault-plugin-vector (prototype)

Semantic vector search for TurboVault as a **compiled-in, default-off plugin
module**. Wraps the embedded `turbovault-vector` engine (fastembed + usearch +
SQLite) behind the plugin boundary.

```bash
cargo run -p turbovault --features vector
```

Tools (advertised namespaced under `vector`):

| Tool | Purpose |
|---|---|
| `vector_search`  | semantic search; returns similar note chunks |
| `vector_reindex` | force a full re-scan into the index |
| `vector_status`  | model, dimensions, indexed-note count, chunking |
| `vector_config`  | resolved configuration + source |

The index persists in the plugin's per-vault state dir
(`.turbovault/plugins/vector/`, via `VaultApi::plugin_state_dir`). Notes are read
through `VaultApi`; the index is kept current by an mtime reconcile
(`VaultApi::list_notes_meta` → `update_note`), which re-embeds only changed
chunks.

**Prototype scope:** reconcile-on-demand (before search / on reindex) rather than
a real-time hook-subscription task; single active vault; default config. Depends
on the proposed `plugin_state_dir` (#42) and `list_notes_meta`/change-feed (#43)
capabilities.
