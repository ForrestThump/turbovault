//! Next-occurrence date math for recurring tasks, following the Obsidian Tasks
//! plugin convention. Ported from the historical task tooling; lives in the
//! module (not core) for the same reason as the renderer — recurrence is a
//! Tasks-plugin convention and its only consumer is `tasks_complete`.
//!
//! The math is calendar (civil) math: every rule advances by whole calendar
//! days or months, never by a fixed number of seconds, so a daylight saving
//! transition never shifts a due date. `jiff`'s civil date arithmetic is used
//! directly; `chrono`'s `NaiveDate` semantics are preserved exactly (see the
//! `JIFF_MIGRATION.md` notes at the repository root).

use jiff::Span;
use jiff::civil::{Date, Weekday};

/// Compute `(due, scheduled, start)` for the next instance of a recurring task,
/// offset by the same interval as the due date. Returns `None` when the rule is
/// not recognized.
///
/// Supported units: `day(s)`, `week(s)`, `month(s)`, `year(s)`, `weekday(s)`,
/// `weekend(s)`, with an optional leading count (e.g. `every 2 weeks`). A leading
/// `!` ("when done") is accepted and ignored, matching the reference behavior.
pub fn compute_next_occurrence(
    recurrence: &str,
    due_date: Option<Date>,
    scheduled_date: Option<Date>,
    start_date: Option<Date>,
    done_date: Date,
) -> Option<(Option<Date>, Option<Date>, Option<Date>)> {
    let lower = recurrence.trim().to_lowercase();
    let rest = lower.strip_prefix("every ").unwrap_or(&lower).trim();
    let rest = rest.trim_start_matches('!').trim();

    let (count, unit): (i64, &str) = match rest.find(' ') {
        Some(pos) => match rest[..pos].parse::<i64>() {
            Ok(count) => (count, rest[pos + 1..].trim()),
            Err(_) => (1, rest),
        },
        None => (1, rest),
    };

    // The due date anchors the interval; fall back to the completion date.
    let reference = due_date.unwrap_or(done_date);

    let next_due: Date = match unit {
        "day" | "days" => add_days(reference, count)?,
        "week" | "weeks" => add_days(reference, count.checked_mul(7)?)?,
        "month" | "months" => add_months(reference, count)?,
        "year" | "years" => add_months(reference, count.checked_mul(12)?)?,
        "weekday" | "weekdays" => next_weekday(reference)?,
        "weekend" | "weekends" => next_weekend(reference)?,
        _ => return None,
    };

    // The scheduled and start dates shift by the same interval as the due
    // date. The interval is a whole number of days (calendar math), so the
    // shift is exact even across daylight saving transitions.
    let offset = next_due.duration_since(reference);
    let next_scheduled = scheduled_date.and_then(|date| date.checked_add(offset).ok());
    let next_start = start_date.and_then(|date| date.checked_add(offset).ok());
    Some((Some(next_due), next_scheduled, next_start))
}

/// Advance a date by `days` calendar days.
///
/// Returns `None` when the interval is out of range for a calendar date.
fn add_days(date: Date, days: i64) -> Option<Date> {
    let span = Span::new().try_days(days).ok()?;
    date.checked_add(span).ok()
}

/// Advance a date by `months` calendar months, clamping the day to the last
/// valid day of the target month (so Jan 31 + 1 month → Feb 28/29).
///
/// This mirrors the clamping the `chrono` implementation performed: the year
/// and month are advanced first, then the day is clamped to the length of the
/// target month.
fn add_months(date: Date, months: i64) -> Option<Date> {
    let total = i64::from(date.month()) - 1 + months;
    let year = i64::from(date.year()) + total.div_euclid(12);
    let month = total.rem_euclid(12) + 1;
    let year = i16::try_from(year).ok()?;
    let month = i8::try_from(month).ok()?;
    let first = Date::new(year, month, 1).ok()?;
    let day = date.day().min(first.last_of_month().day());
    Date::new(year, month, day).ok()
}

/// The next weekday (Monday through Friday) strictly after `reference`.
fn next_weekday(reference: Date) -> Option<Date> {
    let mut day = add_days(reference, 1)?;
    while matches!(day.weekday(), Weekday::Saturday | Weekday::Sunday) {
        day = add_days(day, 1)?;
    }
    Some(day)
}

/// The next weekend day (Saturday or Sunday) strictly after `reference`.
fn next_weekend(reference: Date) -> Option<Date> {
    let mut day = add_days(reference, 1)?;
    while !matches!(day.weekday(), Weekday::Saturday | Weekday::Sunday) {
        day = add_days(day, 1)?;
    }
    Some(day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::DateTime;
    use jiff::tz::TimeZone;

    fn date(text: &str) -> Date {
        text.parse().unwrap()
    }

    /// The time zone the daylight saving tests use.
    ///
    /// It is built from a POSIX rule (`CST6CDT` with the United States
    /// transition rules — daylight saving starts on the second Sunday in
    /// March and ends on the first Sunday in November) instead of a named
    /// zone, so the tests do not depend on a system time zone database being
    /// installed. For 2026 the transitions are 2026-03-08 and 2026-11-01, the
    /// same days as `America/Chicago`.
    fn posix_zone() -> TimeZone {
        TimeZone::posix("CST6CDT,M3.2.0,M11.1.0").expect("the POSIX time zone rule is valid")
    }

    #[test]
    fn every_day_advances_the_due_date() {
        let (due, _, _) = compute_next_occurrence(
            "every day",
            Some(date("2026-07-19")),
            None,
            None,
            date("2026-07-19"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-07-20")));
    }

    #[test]
    fn honors_a_leading_count() {
        let (due, _, _) = compute_next_occurrence(
            "every 2 weeks",
            Some(date("2026-07-01")),
            None,
            None,
            date("2026-07-01"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-07-15")));
    }

    #[test]
    fn month_clamps_to_last_valid_day() {
        let (due, _, _) = compute_next_occurrence(
            "every month",
            Some(date("2026-01-31")),
            None,
            None,
            date("2026-01-31"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-02-28")));
    }

    #[test]
    fn scheduled_and_start_shift_by_the_same_offset() {
        let (due, scheduled, start) = compute_next_occurrence(
            "every week",
            Some(date("2026-07-10")),
            Some(date("2026-07-08")),
            Some(date("2026-07-07")),
            date("2026-07-10"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-07-17")));
        assert_eq!(scheduled, Some(date("2026-07-15")));
        assert_eq!(start, Some(date("2026-07-14")));
    }

    #[test]
    fn weekday_skips_the_weekend() {
        // 2026-07-17 is a Friday; the next weekday is Monday the 20th.
        let (due, _, _) = compute_next_occurrence(
            "every weekday",
            Some(date("2026-07-17")),
            None,
            None,
            date("2026-07-17"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-07-20")));
    }

    #[test]
    fn unrecognized_rule_is_none() {
        assert!(
            compute_next_occurrence(
                "every blue moon",
                Some(date("2026-07-19")),
                None,
                None,
                date("2026-07-19")
            )
            .is_none()
        );
    }

    #[test]
    fn weekend_skips_the_workweek() {
        // 2026-07-17 is a Friday; the next weekend day is Saturday the 18th.
        let (due, _, _) = compute_next_occurrence(
            "every weekend",
            Some(date("2026-07-17")),
            None,
            None,
            date("2026-07-17"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-07-18")));
    }

    #[test]
    fn falls_back_to_the_done_date_when_there_is_no_due_date() {
        let (due, _, _) =
            compute_next_occurrence("every day", None, None, None, date("2026-07-19")).unwrap();
        assert_eq!(due, Some(date("2026-07-20")));
    }

    #[test]
    fn month_end_arithmetic_clamps_to_the_target_month() {
        // Jan 31 + 1 month → Feb 28 (2026 is not a leap year).
        let (due, _, _) = compute_next_occurrence(
            "every month",
            Some(date("2026-01-31")),
            None,
            None,
            date("2026-01-31"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-02-28")));

        // Mar 31 + 1 month → Apr 30.
        let (due, _, _) = compute_next_occurrence(
            "every month",
            Some(date("2026-03-31")),
            None,
            None,
            date("2026-03-31"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-04-30")));

        // Dec 31 + 1 month rolls into the next year.
        let (due, _, _) = compute_next_occurrence(
            "every month",
            Some(date("2026-12-31")),
            None,
            None,
            date("2026-12-31"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2027-01-31")));
    }

    #[test]
    fn leap_day_arithmetic_clamps_like_chrono_did() {
        // Feb 29 2028 (a leap day) + 1 month → Mar 29.
        let (due, _, _) = compute_next_occurrence(
            "every month",
            Some(date("2028-02-29")),
            None,
            None,
            date("2028-02-29"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2028-03-29")));

        // Feb 29 2028 + 1 year → Feb 28 2029 (2029 is not a leap year).
        let (due, _, _) = compute_next_occurrence(
            "every year",
            Some(date("2028-02-29")),
            None,
            None,
            date("2028-02-29"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2029-02-28")));

        // Feb 29 2028 + 4 years keeps the leap day.
        let (due, _, _) = compute_next_occurrence(
            "every 4 years",
            Some(date("2028-02-29")),
            None,
            None,
            date("2028-02-29"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2032-02-29")));
    }

    #[test]
    fn day_arithmetic_crosses_a_leap_day() {
        let (due, _, _) = compute_next_occurrence(
            "every day",
            Some(date("2028-02-28")),
            None,
            None,
            date("2028-02-28"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2028-02-29")));

        let (due, _, _) = compute_next_occurrence(
            "every day",
            Some(date("2028-02-29")),
            None,
            None,
            date("2028-02-29"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2028-03-01")));
    }

    /// Daylight saving transitions must not move a due date: the recurrence
    /// is calendar math, so a task due on the spring-forward day is still due
    /// on the spring-forward day.
    #[test]
    fn recurrence_is_calendar_math_across_the_spring_forward() {
        // 2026-03-08 is the America/Chicago spring-forward day (02:00 → 03:00).
        let (due, scheduled, start) = compute_next_occurrence(
            "every day",
            Some(date("2026-03-07")),
            Some(date("2026-03-07")),
            Some(date("2026-03-07")),
            date("2026-03-07"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-03-08")));
        assert_eq!(scheduled, Some(date("2026-03-08")));
        assert_eq!(start, Some(date("2026-03-08")));

        // The same holds for the week rule and for the day after the
        // transition.
        let (due, _, _) = compute_next_occurrence(
            "every week",
            Some(date("2026-03-01")),
            None,
            None,
            date("2026-03-01"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-03-08")));

        let (due, _, _) = compute_next_occurrence(
            "every day",
            Some(date("2026-03-08")),
            None,
            None,
            date("2026-03-08"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-03-09")));
    }

    /// The same holds across the fall-back day, where a local clock time
    /// happens twice.
    #[test]
    fn recurrence_is_calendar_math_across_the_fall_back() {
        // 2026-11-01 is the America/Chicago fall-back day (02:00 → 01:00).
        let (due, _, _) = compute_next_occurrence(
            "every day",
            Some(date("2026-10-31")),
            None,
            None,
            date("2026-10-31"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-11-01")));

        let (due, _, _) = compute_next_occurrence(
            "every day",
            Some(date("2026-11-01")),
            None,
            None,
            date("2026-11-01"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-11-02")));
    }

    /// The daylight saving transitions themselves are visible in the instant
    /// timeline, but not in the calendar arithmetic the recurrence uses: the
    /// spring-forward day is 23 hours long and the fall-back day is 25 hours
    /// long, while the recurrence still advances by exactly one day.
    ///
    /// The zone is built from a POSIX rule (`CST6CDT` with the United States
    /// transition rules) instead of a named zone, so the test does not need a
    /// system time zone database.
    #[test]
    fn daylight_saving_transitions_change_the_elapsed_time_not_the_day_count() {
        let tz = posix_zone();

        let before = DateTime::new(2026, 3, 8, 0, 0, 0, 0)
            .unwrap()
            .to_zoned(tz.clone())
            .unwrap();
        let after = DateTime::new(2026, 3, 9, 0, 0, 0, 0)
            .unwrap()
            .to_zoned(tz.clone())
            .unwrap();
        let elapsed = after.timestamp().duration_since(before.timestamp());
        assert_eq!(
            elapsed.as_hours(),
            23,
            "spring forward day is 23 hours long"
        );

        let (due, _, _) = compute_next_occurrence(
            "every day",
            Some(date("2026-03-08")),
            None,
            None,
            date("2026-03-08"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-03-09")));

        let before = DateTime::new(2026, 11, 1, 0, 0, 0, 0)
            .unwrap()
            .to_zoned(tz.clone())
            .unwrap();
        let after = DateTime::new(2026, 11, 2, 0, 0, 0, 0)
            .unwrap()
            .to_zoned(tz.clone())
            .unwrap();
        let elapsed = after.timestamp().duration_since(before.timestamp());
        assert_eq!(elapsed.as_hours(), 25, "fall back day is 25 hours long");

        let (due, _, _) = compute_next_occurrence(
            "every day",
            Some(date("2026-11-01")),
            None,
            None,
            date("2026-11-01"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-11-02")));
    }

    /// 02:30 does not exist on the spring-forward day and 01:30 happens twice
    /// on the fall-back day. Neither affects the calendar arithmetic, but the
    /// disambiguation is pinned here so a future switch to instant-based
    /// recurrence cannot silently change a due date.
    ///
    /// The zone is built from a POSIX rule instead of a named zone, so the
    /// test does not need a system time zone database.
    #[test]
    fn ambiguous_and_nonexistent_local_times_are_disambiguated() {
        let tz = posix_zone();

        // 2026-03-08 02:30 does not exist: the clock jumps from 02:00 to 03:00.
        let gap = DateTime::new(2026, 3, 8, 2, 30, 0, 0).unwrap();
        let zoned = tz
            .clone()
            .into_ambiguous_zoned(gap)
            .compatible()
            .expect("compatible disambiguation always resolves");
        assert_eq!(zoned.to_string(), "2026-03-08T03:30:00-05:00[-05:00]");

        // 2026-11-01 01:30 happens twice: the earlier (daylight saving) one is
        // picked by the compatible disambiguation.
        let ambiguous = DateTime::new(2026, 11, 1, 1, 30, 0, 0).unwrap();
        let zoned = tz
            .clone()
            .into_ambiguous_zoned(ambiguous)
            .compatible()
            .expect("compatible disambiguation always resolves");
        assert_eq!(zoned.to_string(), "2026-11-01T01:30:00-05:00[-05:00]");
        let later = tz
            .clone()
            .into_ambiguous_zoned(DateTime::new(2026, 11, 1, 1, 30, 0, 0).unwrap())
            .later()
            .expect("later disambiguation always resolves");
        assert_eq!(later.to_string(), "2026-11-01T01:30:00-06:00[-06:00]");

        // The plugin's own arithmetic is unaffected by either transition: the
        // due date and the scheduled and start dates move by exactly one
        // calendar day.
        let (due, scheduled, start) = compute_next_occurrence(
            "every day",
            Some(date("2026-03-08")),
            Some(date("2026-03-08")),
            Some(date("2026-03-08")),
            date("2026-03-08"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-03-09")));
        assert_eq!(scheduled, Some(date("2026-03-09")));
        assert_eq!(start, Some(date("2026-03-09")));

        let (due, scheduled, start) = compute_next_occurrence(
            "every day",
            Some(date("2026-11-01")),
            Some(date("2026-11-01")),
            Some(date("2026-11-01")),
            date("2026-11-01"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-11-02")));
        assert_eq!(scheduled, Some(date("2026-11-02")));
        assert_eq!(start, Some(date("2026-11-02")));
    }

    /// The recurrence itself is calendar math, so a task due on a transition
    /// day is still due on that day — and the scheduled and start dates move
    /// with it.
    #[test]
    fn recurrence_survives_the_dst_gap_and_fold() {
        // Spring forward: 2026-03-08 (the day 02:30 does not exist on).
        let (due, scheduled, start) = compute_next_occurrence(
            "every day",
            Some(date("2026-03-08")),
            Some(date("2026-03-08")),
            Some(date("2026-03-08")),
            date("2026-03-08"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-03-09")));
        assert_eq!(scheduled, Some(date("2026-03-09")));
        assert_eq!(start, Some(date("2026-03-09")));

        // A weekly rule that lands exactly on the transition day.
        let (due, _, _) = compute_next_occurrence(
            "every week",
            Some(date("2026-03-01")),
            None,
            None,
            date("2026-03-01"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-03-08")));

        // Fall back: 2026-11-01 (the day 01:30 happens twice on).
        let (due, scheduled, start) = compute_next_occurrence(
            "every day",
            Some(date("2026-11-01")),
            Some(date("2026-11-01")),
            Some(date("2026-11-01")),
            date("2026-11-01"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-11-02")));
        assert_eq!(scheduled, Some(date("2026-11-02")));
        assert_eq!(start, Some(date("2026-11-02")));

        // A weekly rule that lands exactly on the transition day.
        let (due, _, _) = compute_next_occurrence(
            "every week",
            Some(date("2026-10-25")),
            None,
            None,
            date("2026-10-25"),
        )
        .unwrap();
        assert_eq!(due, Some(date("2026-11-01")));
    }
}
