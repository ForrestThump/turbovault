//! Next-occurrence date math for recurring tasks, following the Obsidian Tasks
//! plugin convention. Ported from the historical task tooling; lives in the
//! module (not core) for the same reason as the renderer — recurrence is a
//! Tasks-plugin convention and its only consumer is `tasks_complete`.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

/// Compute `(due, scheduled, start)` for the next instance of a recurring task,
/// offset by the same interval as the due date. Returns `None` when the rule is
/// not recognized.
///
/// Supported units: `day(s)`, `week(s)`, `month(s)`, `year(s)`, `weekday(s)`,
/// `weekend(s)`, with an optional leading count (e.g. `every 2 weeks`). A leading
/// `!` ("when done") is accepted and ignored, matching the reference behavior.
pub fn compute_next_occurrence(
    recurrence: &str,
    due_date: Option<NaiveDate>,
    scheduled_date: Option<NaiveDate>,
    start_date: Option<NaiveDate>,
    done_date: NaiveDate,
) -> Option<(Option<NaiveDate>, Option<NaiveDate>, Option<NaiveDate>)> {
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

    let next_due: NaiveDate = match unit {
        "day" | "days" => reference + Duration::days(count),
        "week" | "weeks" => reference + Duration::days(count * 7),
        "month" | "months" => advance_by_months(reference, count as u32)?,
        "year" | "years" => advance_by_months(reference, (count * 12) as u32)?,
        "weekday" | "weekdays" => {
            let mut day = reference + Duration::days(1);
            while matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
                day += Duration::days(1);
            }
            day
        }
        "weekend" | "weekends" => {
            let mut day = reference + Duration::days(1);
            while !matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
                day += Duration::days(1);
            }
            day
        }
        _ => return None,
    };

    let offset = next_due.signed_duration_since(reference);
    let next_scheduled = scheduled_date.map(|date| date + offset);
    let next_start = start_date.map(|date| date + offset);
    Some((Some(next_due), next_scheduled, next_start))
}

/// Advance a date by `months` calendar months, clamping the day to the last
/// valid day of the target month (so Jan 31 + 1 month → Feb 28/29).
fn advance_by_months(date: NaiveDate, months: u32) -> Option<NaiveDate> {
    let total = date.month0() + months;
    let new_year = date.year() + (total / 12) as i32;
    let new_month = total % 12 + 1;
    let last_day = days_in_month(new_year, new_month);
    NaiveDate::from_ymd_opt(new_year, new_month, date.day().min(last_day))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|date| date.pred_opt())
        .map(|date| date.day())
        .unwrap_or(28)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn every_day_advances_the_due_date() {
        let (due, _, _) =
            compute_next_occurrence("every day", Some(date("2026-07-19")), None, None, date("2026-07-19"))
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
        let (due, _, _) =
            compute_next_occurrence("every month", Some(date("2026-01-31")), None, None, date("2026-01-31"))
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
        let (due, _, _) =
            compute_next_occurrence("every weekday", Some(date("2026-07-17")), None, None, date("2026-07-17"))
                .unwrap();
        assert_eq!(due, Some(date("2026-07-20")));
    }

    #[test]
    fn unrecognized_rule_is_none() {
        assert!(
            compute_next_occurrence("every blue moon", Some(date("2026-07-19")), None, None, date("2026-07-19"))
                .is_none()
        );
    }
}
