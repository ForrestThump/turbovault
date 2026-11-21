# chrono → jiff migration

TurboVault used [`chrono`](https://docs.rs/chrono) for every date and timestamp.
This document records the migration to [`jiff`](https://docs.rs/jiff) `0.2.35`
(the version the consumer pins), the public API changes it caused, the wire
format guarantees that were preserved, and the places where `chrono` is still
present in the dependency tree.

* Workspace dependency: `jiff = { version = "=0.2.35", features = ["serde"] }`
  (exact pin, `serde` enabled).
* `chrono` is no longer a direct dependency of any crate in this workspace.

## Public API changes

### `turbovault-core`

| Before | After |
|---|---|
| `TaskItem::created_date: Option<chrono::NaiveDate>` | `Option<jiff::civil::Date>` |
| `TaskItem::scheduled_date: Option<chrono::NaiveDate>` | `Option<jiff::civil::Date>` |
| `TaskItem::start_date: Option<chrono::NaiveDate>` | `Option<jiff::civil::Date>` |
| `TaskItem::due_date: Option<chrono::NaiveDate>` | `Option<jiff::civil::Date>` |
| `TaskItem::done_date: Option<chrono::NaiveDate>` | `Option<jiff::civil::Date>` |
| `TaskItem::cancelled_date: Option<chrono::NaiveDate>` | `Option<jiff::civil::Date>` |

New public module `turbovault_core::datetime` (also re-exported from the
crate root):

* `format_timestamp(jiff::Timestamp) -> String` — RFC 3339 UTC rendering that
  matches the shape `chrono`'s `DateTime<Utc>` serde impl produced.
* `format_now() -> String` — the current instant in that same shape.
* `parse_timestamp(&str) -> Result<jiff::Timestamp, jiff::Error>` — RFC 3339
  parsing that accepts everything `chrono`'s `DateTime::parse_from_rfc3339`
  accepted.
* `parse_iso_date(&str) -> Option<jiff::civil::Date>` — ISO `YYYY-MM-DD`
  parsing with `chrono`'s lenient `%Y-%m-%d` semantics (one- or two-digit
  month/day, leading whitespace tolerated).
* `datetime::serde_compat` — serde adapter (`serialize` / `deserialize`) that
  pins the `chrono` wire shape for any `jiff::Timestamp` field.

### `turbovault-audit`

| Before | After |
|---|---|
| `AuditFilter::since: Option<chrono::DateTime<Utc>>` | `Option<jiff::Timestamp>` |
| `AuditFilter::until: Option<chrono::DateTime<Utc>>` | `Option<jiff::Timestamp>` |
| `AuditFilter::with_since(chrono::DateTime<Utc>)` | `with_since(jiff::Timestamp)` |

`AuditEntry::timestamp` stays a `String`; only the rendering changed (see
*Wire format* below).

### `turbovault-plugin-tasks`

| Before | After |
|---|---|
| `recurrence::compute_next_occurrence(… Option<chrono::NaiveDate> …)` | `… Option<jiff::civil::Date> …` |

`compute_next_occurrence` is `pub` inside the crate but the module is private,
so this is an internal signature change. `render::to_markdown_line` and
`render::render_body` keep their signatures (`&TaskItem`), so the renderer is
unaffected.

### `turbovault-tools`, `turbovault-export`, `turbovault-batch`,
`turbovault-vector`

No public API change. All of them only used `chrono` internally (or, for
`-batch` and `-vector`, not at all — the unused dependency was dropped).

## Wire format

Every serialized timestamp keeps the shape `chrono` produced:

```text
2026-11-20T22:15:03.123456Z
```

* RFC 3339, UTC, `Z` suffix.
* Fractional seconds in *SI precision*, exactly like `chrono`'s
  `DateTime<Utc>` serde impl: omitted for a whole second, otherwise 3, 6, or
  9 digits (`...T22:15:03.100Z`, `...T22:15:03.123456Z`,
  `...T22:15:03.123456789Z`).

`jiff`'s own `Timestamp` rendering trims trailing zeros instead
(`...T22:15:03.1Z`), so `turbovault_core::datetime::format_timestamp` is used
for every timestamp that reaches a serde/API/disk boundary (audit log JSONL,
export reports, tool responses, note metadata). `datetime::serde_compat` does
the same for `Timestamp` fields that are serialized directly.

One deliberate difference: `chrono`'s `DateTime::to_rfc3339()` (used by the old
code for the string fields) rendered the offset as `+00:00` instead of `Z`.
The new rendering uses `Z`, matching the shape `chrono`'s *serde* impl
produced and the format documented above. Both spellings are valid RFC 3339
and are parsed by `chrono` and `jiff` alike, so existing logs keep working;
the parser still accepts `+00:00` and `-05:00` offsets with or without
fractional seconds.

Calendar dates are unaffected: `chrono::NaiveDate` and `jiff::civil::Date` both
serialize as `YYYY-MM-DD`. Deserialization keeps `chrono`'s leniency through
`turbovault_core::datetime::parse_iso_date`: one- or two-digit months and days
(`2026-1-31`, `2026-01-1`) and a signed year (`+2026-01-31`) are accepted,
while the spellings `chrono` rejected (`2026-011-02`, `2026-01-031`,
`2026-01-+1`) are rejected as well. `TaskItem`'s date fields go through that
parser (see `datetime::iso_date`), so their JSON accepts exactly what
`chrono`'s `NaiveDate` serde impl accepted.

## Known deltas vs `chrono`

| Topic | `chrono` | Here (`jiff`) |
|---|---|---|
| Leap second (`:60`) | accepted; the parsed value is the instant right after `23:59:59` | same: accepted, and mapped to the following second (`2016-12-31T23:59:60Z` → `2017-01-01T00:00:00Z`) |
| Leap second, *rendered* | kept the `23:59:60` spelling | `jiff`'s `Timestamp` cannot hold a leap second, so the equivalent instant is rendered (`2017-01-01T00:00:00Z`) |
| Fraction with more than 9 digits | truncated to nanosecond precision | same: truncated to nanosecond precision |
| Offset without a colon (`+0000`), hour-only offset (`+00`), offset with a seconds field (`+00:00:00`) | rejected | rejected as well — `jiff` accepts them, so the shape is checked before parsing |
| Time without a seconds field (`2025-11-20T21:35Z`) | rejected | rejected as well — `jiff` accepts them |
| Year outside `0000`..=`9999` | rendered as a signed five digit year (`-0001-01-01T00:00:00Z`) | rendered as a signed six digit year (`-000001-01-01T00:00:00Z`) |
| Range of representable instants | roughly `-262144`..=`262144` years | `jiff`'s `Timestamp` range, roughly `-009999-01-02`..`9999-12-30` |
| Whitespace before a numeric date field (`2026-01- 1`) | skipped, so accepted | rejected — the month and the day must be one or two digits |

The **supported range** of the timestamp helpers is the RFC 3339 year range,
`0000`..=`9999`; inside it the rendering and the parsing are identical to
`chrono`'s.

## Behavior preserved

* **Local "today"**: `chrono::Local::now().date_naive()` became
  `jiff::Zoned::now().date()` — the civil date in the host time zone.
* **Lenient date parsing**: `chrono`'s `%Y-%m-%d` accepted one- or two-digit
  months and days, an explicitly signed year, and leading whitespace; `jiff`'s
  civil parser does not, so `turbovault_core::datetime::parse_iso_date` keeps
  the old leniency (used by task metadata parsing, the tasks plugin tool
  arguments, and the OKF log-entry date).
* **Recurrence math**: `every N days/weeks/months/years/weekdays/weekends` is
  still calendar math with the same clamping (Jan 31 + 1 month → Feb 28/29,
  Feb 29 + 1 year → Feb 28). The month arithmetic reproduces `chrono`'s
  clamping exactly.
* **Audit filtering**: the `since`/`until` window is still exclusive on both
  ends (`entry_time < since` / `entry_time > until` are the only skips).

Two edge cases of the recurrence arithmetic changed:

* **A negative count is not an error.** `every -1 month` from 2026-03-15
  yields 2026-02-15, a date in the past, and `every -3 days` yields
  2026-03-12. `chrono` computed the same dates, but its `u32` month
  arithmetic overflowed (and panicked in debug builds) for a negative month
  count.
* **The range of the calendar arithmetic is narrower.** `jiff` covers roughly
  the years `-9999`..`9999`, `chrono` covered more, so an interval of more
  than about 7,304,484 days (roughly 20,000 years) has no next occurrence
  (`None`) here. `every 8000000 days` is such an interval; `chrono` could
  still express a date for it.

## Tests added

* `turbovault-core::datetime`: RFC 3339 rendering that matches `chrono`'s
  serde output for every SI-precision case, round-trips, parsing of the strings
  `chrono` produced (`+00:00`, `-05:00`, `Z`, with and without fractional
  seconds), rejection of non-RFC-3339 input, lenient ISO date parsing, and the
  serde adapter. Later rounds added: the pre-epoch sub-second normalization
  (`1969-12-31T23:59:59.500Z`), the tightened month/day field widths, the
  signed year, the leap second, the over-long fraction, the offset spellings
  `jiff` accepts, and the RFC 3339 year range.
* `turbovault-core::models`: a `TaskItem` JSON round-trip with the unpadded
  and signed date spellings `chrono` accepted, the zero padded spelling on
  the way out, and the rejection of a spelling `chrono` rejected.
* `turbovault-audit`: the audit log timestamp is RFC 3339 UTC with a `Z`
  suffix, `since`/`until` filtering works with `jiff::Timestamp`, and entries
  written by the `chrono` era (`+00:00` offsets) are still read and filtered.
* `turbovault-plugin-tasks::recurrence`: month-end and leap-day arithmetic
  (Jan 31 → Feb 28, Feb 29 2028 → Mar 29, Feb 29 2028 + 1 year → Feb 28
  2029, + 4 years → Feb 29 2032), daylight saving transitions
  (America/Chicago spring-forward 2026-03-08 and fall-back 2026-11-01: the
  recurrence advances by exactly one calendar day while the elapsed time is 23
  resp. 25 hours), the disambiguation of the nonexistent 02:30 and the
  ambiguous 01:30 local times, and the recurrence itself across both
  transition days.

## `chrono` still in the dependency tree

`cargo tree -i chrono --workspace --all-features -e normal,build,dev --target all`
after this migration reports `chrono` only through third-party crates:

| Path | Note |
|---|---|
| `turbomcp-core 3.1.5` → `chrono` | The crates.io `turbomcp` release enables `chrono` through its `rich-errors` feature. A separate PR migrates the `turbomcp` fork; no `[patch]` is added here. |
| `gluesql-core 0.19.0` → `chrono` | Pulled in by `gluesql`, which `turbovault-sql` uses for the optional `sql` feature (`turbovault-tools` → `turbovault`). Upstream dependency, not replaced here. |
| `schemars 1.2.1` → `chrono` | `schemars` keeps an optional `chrono04` dependency, and the crates.io `turbomcp 3.1.5` release activates that feature. This workspace does **not** enable it — it enables `schemars`' `jiff02` feature instead, so `jiff` types get their JSON schema impls. |

No crate in this workspace depends on `chrono` directly any more.
