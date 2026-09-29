# Timed vault reminders

TurboVault can be the source of truth for **when** an Obsidian Tasks line is eligible to fire. It does not send the reminder. There is no Telegram delivery, no scheduler, and no cron tick in this repository. A consumer (for example Liberado) reads the parsed tasks and decides the local minute itself.

Full Calendar notes are not part of this API. TurboVault does not parse `date`, `startTime`, `endTime`, `daysOfWeek` (`UMTWRFS`), `startRecur`, or `endRecur`. Event reminders have to keep using whatever reads those notes until a later change extends the parser. This surface is Tasks only.

## Two gates

Both must pass before a task shows up in the eligible list.

1. **Vault config** `timed_reminders` (default `false`). Parsing still records the per-task flag when this is off. [`remind_eligible_tasks`](../../crates/turbovault-core/src/reminders.rs) returns an empty list until the flag is true.
2. **Task opt-in**, in the trailing metadata of that line:
   - inline tag `#remind` (ASCII case-insensitive), or
   - Dataview `[remind:: …]` or `[reminder:: …]` (parentheses work too: `(remind:: …)`).

Truthy Dataview values, after trimming and one layer of matching quotes, are `true`, `yes`, `y`, `1`, and `on` (ASCII case-insensitive). Anything else, including `false`, `no`, `off`, and `0`, is an explicit opt-out. The rightmost remind tag or field wins, same as other task metadata.

These do **not** opt a task in:

- `#reminder`, `#remind/morning`, or any other tag
- the word "remind" in the description
- note frontmatter (`remind: true` on the file)
- a Full Calendar property

## What a consumer gets

On each [`TaskItem`](../../crates/turbovault-core/src/models.rs):

| Field | Meaning |
| --- | --- |
| `remind` | Item opted in |
| `due_date` / `due_time` | Due date, and clock time when the source had one |
| `scheduled_date` / `scheduled_time` | Scheduled date and optional time |
| `start_date` / `start_time` | Start date and optional time |
| `recurrence` | Tasks rule text such as `every weekday`. Not expanded into occurrences |
| `is_completed`, `status_marker` | Checkbox. `x` / `X` is done, `-` is cancelled, `/` stays open |
| `done_date`, `cancelled_date` | Done or cancelled even when the checkbox was not updated |

`TaskItem::reminder_info()` projects those into lifecycle (`open`, `done`, `cancelled`) plus the three anchors. `TaskItem::is_remind_eligible()` is the item-level check and ignores the vault toggle.

A task is eligible only when it opted in, is still **open**, and has at least one of due, scheduled, or start. Recurrence alone is not enough: there is no date to put on a clock. Done and cancelled tasks are skipped even with `#remind`.

Preferred fire anchor, in order: **due**, else **scheduled**, else **start**. That is `ReminderFire` on each [`RemindEligibleTask`](../../crates/turbovault-core/src/reminders.rs).

### Clock time

`LocalMinute.time` is set when the source included a time:

- Dataview: `[due:: 2026-05-01 09:30]`, `[due:: 2026-05-01T09:30]`, or with seconds
- Emoji: `📅 2026-05-01T09:30` or `📅 2026-05-01 09:30` (`HH:MM` or `HH:MM:SS` in the next token)

If `time` is `None`, the value was date-only. TurboVault does not invent 00:00 or a morning default. The consumer picks the local minute.

`RemindEligibleTask` also carries `content`, source `position` (pair it with the note path yourself), and `id` / `block_ref` when the task had them.

## Config

Per vault, in the TurboVault YAML `vaults:` entry:

```yaml
vaults:
  - name: personal
    path: /path/to/vault
    is_default: true
    timed_reminders: true
```

Or the builder:

```rust
let vault = VaultConfig::builder("personal", path)
    .timed_reminders(true)
    .build()?;
```

Leave it false (the default) and no consumer query should treat the vault as a reminder source.

## Calling the API

Parse notes with the existing task parser. Do not keep a second copy of the Tasks grammar.

```rust
use turbovault_core::remind_eligible_tasks;
use turbovault_parser::parse_tasks;

let tasks = parse_tasks(&markdown);
// `vault.timed_reminders` is the feature gate.
let due = remind_eligible_tasks(&tasks, vault.timed_reminders);
for item in due {
    let when = item.fire.when;
    // when.date is the local calendar day.
    // when.time is Some(HH:MM[:SS]) or None when the line was date-only.
    // item.info.recurrence is the raw rule, if any.
    // item.position is the line inside this note.
}
```

The same list is empty when `vault.timed_reminders` is false, and it omits unmarked, done, cancelled, and dateless tasks when the toggle is on.

Schedule the next occurrence from `recurrence` in the consumer. TurboVault only preserves the text (`every day`, `every week on Sunday`, `every weekday`, and the other Tasks phrases the parser already accepts, including Dataview `repeat::` / `recurrence::`).
