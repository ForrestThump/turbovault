# Tasks timing fields

Obsidian Tasks emoji metadata and Dataview inline fields are parsed onto `TaskItem`. Dates stay `YYYY-MM-DD`. When the source includes a clock time, it is stored separately so the date is not dropped.

| Source | `due_date` / `scheduled_date` / `start_date` | `due_time` / `scheduled_time` / `start_time` |
| --- | --- | --- |
| `📅 2026-05-01` or `[due:: 2026-05-01]` | `2026-05-01` | absent |
| `📅 2026-05-01T09:30` or `[due:: 2026-05-01T09:30]` | `2026-05-01` | `09:30:00` |
| `📅 2026-05-01 09:30` or `[due:: 2026-05-01 09:30]` | `2026-05-01` | `09:30:00` |
| same forms with seconds (`09:30:15`) | the date | `09:30:15` |

The same forms apply to scheduled (`⏳` / `scheduled::`) and start (`🛫` / `start::`). `HH:MM` and `HH:MM:SS` use two digits. A token that is not a real clock time, such as `99:99`, is left in the description.

`recurrence` remains the Tasks rule text (`🔁 every weekday`, `[repeat:: every week]`, `[recurrence:: …]`). It is not expanded into occurrences.

`status_marker` is the checkbox character: space, `x` / `X`, `/` (in progress), or `-` (cancelled). `is_completed` is still true only for `x` and `X`. Older JSON without `status_marker` infers `x` when `is_completed` is set and a space otherwise. `checkbox_marker()` applies that fallback.

Full Calendar note fields (`date`, `startTime`, `daysOfWeek`, `startRecur`, `endRecur`) are not parsed.
