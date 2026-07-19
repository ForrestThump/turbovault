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
| `tasks_update`   | write | edit fields and re-render the line in the vault's dialect |
| `tasks_delete`   | write | remove a task line |
| `tasks_config`   | read  | show the settings the module tuned itself to |

All vault access goes through the curated `VaultApi`; every write is
compare-and-swap (`Match(version)`) — the module cannot blind-overwrite a note.

## Round trip

Reads parse task lines with core's `turbovault_parser::parse_tasks`. Writes that
change fields (`tasks_update`) re-render through this crate's own renderer
([`render::to_markdown_line`] / `render_body`) — the write half of the round
trip, owned here because emitting a task requires choosing one canonical dialect
(an opinion), whereas parsing is tolerant and belongs to the kernel. The
invariant the renderer guarantees is semantic: `parse_tasks(render(task)) ==
task` for every field the parser models.

`tasks_complete`/`tasks_delete` still use lossless line surgery — they don't need
to re-render.

## Self-tuning to your Obsidian Tasks settings

The module matches your vault instead of requiring separate configuration. It
resolves two settings, in priority order:

1. **Authoritative** — `.obsidian/plugins/obsidian-tasks-plugin/data.json`, read
   through the curated `VaultApi::read_config` (a read-only capability scoped to
   `.obsidian/`). It picks up `taskFormat` (emoji vs dataview) and `globalFilter`.
2. **Heuristic** — when the settings file is absent or the host declines config
   reads, it infers the metadata dialect from the vault's existing task lines.
3. **Default** — emoji, no global filter.

`tasks_config` reports the resolved settings and their `source`
(`obsidian-data` | `heuristic` | `default`). The global filter, when set, scopes
every read to the tasks your Tasks plugin would consider real.

> **Note:** `VaultApi::read_config` is a fork-local addition to the plugin API
> (a non-breaking default method, `None` where a host declines it). It is the
> only door onto the vault's non-note config space and is flagged for upstream
> review before it is proposed to turbovault-core.

## Deferred (follow-ups)

- **Recurrence-spawn on `complete`**: emitting the next occurrence line is pure
  `to_markdown_line`; it needs a recurrence-rule date engine (`every week`,
  `every weekday`, …), deferred to keep that logic correct rather than hasty.
