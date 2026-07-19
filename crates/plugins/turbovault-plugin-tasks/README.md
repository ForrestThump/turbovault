# turbovault-plugin-tasks

Obsidian-tasks vertical for TurboVault, shipped as a **compiled-in, default-off
plugin module**. Enable it with the host `tasks` feature:

```bash
cargo run -p turbovault --features tasks
```

Tools are advertised namespaced under the plugin id `tasks`:

| Tool | Kind | Purpose |
|---|---|---|
| `tasks_list`     | read  | list tasks (status + tag filters) |
| `tasks_overdue`  | read  | pending tasks past their due date |
| `tasks_tags`     | read  | distinct inline task tags |
| `tasks_complete` | write | flip checkbox to `[x]` + stamp `✅ <date>` |
| `tasks_delete`   | write | remove a task line |

All vault access goes through the curated `VaultApi`; every write is
compare-and-swap (`Match(version)`) — the module cannot blind-overwrite a note.

## Deferred (follow-ups)

- `tasks_update` (arbitrary field edits) and recurrence-spawn on complete need
  the `TaskItem` → markdown renderer (`to_markdown_line`) from the fork's task
  branch. Port it into `turbovault-core` (or here) rather than re-implementing
  lossily.
