//! Date and timestamp helpers shared across the workspace.
//!
//! # Why this module exists
//!
//! TurboVault used `chrono` for every date and timestamp before the `jiff`
//! migration (see `JIFF_MIGRATION.md` at the repository root). Two wire-level
//! details changed with the swap, and they are pinned down here so JSON,
//! JSONL, and CSV payloads stay byte-for-byte compatible with the `chrono`
//! era:
//!
//! * `chrono`'s `DateTime<Utc>` serde impl wrote RFC 3339 with a `Z` suffix
//!   and *SI-precision* fractional seconds: no fraction at all for a whole
//!   second, otherwise 3, 6, or 9 digits (`...T22:15:03.100Z`). `jiff`'s own
//!   `Timestamp` rendering trims trailing zeros instead
//!   (`...T22:15:03.1Z`), so [`format_timestamp`] reproduces the old shape.
//! * `chrono`'s `NaiveDate::parse_from_str(raw, "%Y-%m-%d")` accepted one- or
//!   two-digit months and days and skipped leading whitespace, while `jiff`'s
//!   civil date parser requires zero-padded fields. [`parse_iso_date`] keeps
//!   the lenient behavior so hand-written dates still parse.
//!
//! Everything else in the workspace uses `jiff` types directly
//! ([`jiff::Timestamp`] for instants, [`jiff::civil::Date`] for calendar
//! dates).

use jiff::Timestamp;
use jiff::civil::Date;

/// Format a timestamp the way chrono's `DateTime<Utc>` serde impl did.
///
/// The output is RFC 3339 in UTC with a `Z` suffix and SI-precision
/// fractional seconds: omitted when the timestamp is a whole second,
/// otherwise 3, 6, or 9 digits.
///
/// # Examples
///
/// ```
/// use jiff::Timestamp;
/// use turbovault_core::datetime::format_timestamp;
///
/// let ts = Timestamp::from_second(1_763_674_503).unwrap();
/// assert_eq!(format_timestamp(ts), "2025-11-20T21:35:03Z");
/// ```
#[must_use]
pub fn format_timestamp(timestamp: Timestamp) -> String {
    // `jiff` reports a *negative* sub-second component for instants before the
    // Unix epoch: its second is truncated towards zero instead of floored, so
    // `1969-12-31T23:59:59.5Z` is stored as second `0` with `-500_000_000`
    // nanoseconds. Normalize to the non-negative nanosecond within the second,
    // which is what `chrono` reported.
    let raw_nanos = timestamp.subsec_nanosecond();
    let nanos = if raw_nanos < 0 {
        raw_nanos + 1_000_000_000
    } else {
        raw_nanos
    };
    let fraction = if nanos == 0 {
        String::new()
    } else if nanos % 1_000_000 == 0 {
        format!(".{:03}", nanos / 1_000_000)
    } else if nanos % 1_000 == 0 {
        format!(".{:06}", nanos / 1_000)
    } else {
        format!(".{nanos:09}")
    };

    // `Timestamp` always renders in UTC with a `Z` suffix. Drop the fraction
    // jiff rendered (it trims trailing zeros) and append the chrono-shaped one.
    let rendered = timestamp.to_string();
    let stem = rendered.strip_suffix('Z').unwrap_or(&rendered);
    let stem = stem.split_once('.').map_or(stem, |(head, _)| head);
    format!("{stem}{fraction}Z")
}

/// Format the current instant the way chrono's `DateTime<Utc>` serde impl did.
///
/// See [`format_timestamp`] for the exact shape.
#[must_use]
pub fn format_now() -> String {
    format_timestamp(Timestamp::now())
}

/// Parse an RFC 3339 timestamp.
///
/// Accepts everything chrono's `DateTime::parse_from_rfc3339` accepted: a `Z`
/// suffix or a numeric UTC offset (`+00:00`, `-05:00`), with or without
/// fractional seconds.
pub fn parse_timestamp(raw: &str) -> Result<Timestamp, jiff::Error> {
    raw.parse()
}

/// Parse an ISO `YYYY-MM-DD` calendar date.
///
/// This keeps the lenient behavior of chrono's
/// `NaiveDate::parse_from_str(raw, "%Y-%m-%d")`:
///
/// * the year is one to four digits, optionally signed (`+2026-01-31` and
///   `-2026-01-31` are accepted, like chrono's signed `%Y`),
/// * the month and the day are one or two digits without a sign
///   (`2026-1-31` and `2026-01-1` are accepted, `2026-011-02`,
///   `2026-01-031` and `2026-01-+1` are not),
/// * leading whitespace is ignored.
///
/// `None` is returned for anything else, including trailing garbage.
#[must_use]
pub fn parse_iso_date(raw: &str) -> Option<Date> {
    let trimmed = raw.trim_start();
    let (negative, rest) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };

    let mut parts = rest.split('-');
    let year_text = parts.next()?;
    let month_text = parts.next()?;
    let day_text = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    // An unsigned year is at most four digits wide (chrono's `%Y` width); a
    // signed one may carry more digits, which are range checked below.
    let year_digits = year_text.as_bytes();
    if year_digits.is_empty() || !year_digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if !negative && year_digits.len() > 4 {
        return None;
    }
    let year: i32 = year_text.parse().ok()?;

    let month = parse_two_digits(month_text)?;
    let day = parse_two_digits(day_text)?;

    let year = if negative { -year } else { year };
    Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()
}

/// Parse a month or day field: one or two ASCII digits, without a sign.
///
/// This is the width chrono's `%m` and `%d` accepted — `2026-011-02` and
/// `2026-01-031` were rejected there, and so are they here.
fn parse_two_digits(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > 2 {
        return None;
    }
    if !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    text.parse().ok()
}

/// Serde adapter that keeps the `chrono` wire shape for [`Timestamp`].
///
/// `jiff`'s built-in `Timestamp` impls render fractional seconds with trimmed
/// trailing zeros (`...T22:15:03.1Z`). Applying this module to a field keeps
/// the historical `chrono` shape (`...T22:15:03.100Z`):
///
/// ```
/// use jiff::Timestamp;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct Entry {
///     #[serde(with = "turbovault_core::datetime::serde_compat")]
///     timestamp: Timestamp,
/// }
///
/// let ts = Timestamp::from_second(1_763_674_503).unwrap();
/// let json = serde_json::to_string(&Entry { timestamp: ts }).unwrap();
/// assert_eq!(json, r#"{"timestamp":"2025-11-20T21:35:03Z"}"#);
/// ```
pub mod serde_compat {
    use jiff::Timestamp;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serialize a timestamp in the chrono-compatible RFC 3339 shape.
    ///
    /// # Errors
    ///
    /// Returns an error if the serializer fails.
    pub fn serialize<S: Serializer>(
        timestamp: &Timestamp,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::format_timestamp(*timestamp))
    }

    /// Deserialize a timestamp from an RFC 3339 string.
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not a valid RFC 3339 timestamp.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::SignedDuration;

    fn timestamp(nanos: i64) -> Timestamp {
        Timestamp::from_second(1_763_674_503)
            .unwrap()
            .checked_add(SignedDuration::from_nanos(nanos))
            .unwrap()
    }

    #[test]
    fn formats_timestamps_like_chrono_serde() {
        // Expected values are what chrono's `DateTime<Utc>` serde impl wrote.
        assert_eq!(format_timestamp(timestamp(0)), "2025-11-20T21:35:03Z");
        assert_eq!(
            format_timestamp(timestamp(123_456_000)),
            "2025-11-20T21:35:03.123456Z"
        );
        assert_eq!(
            format_timestamp(timestamp(123_000_000)),
            "2025-11-20T21:35:03.123Z"
        );
        assert_eq!(
            format_timestamp(timestamp(123_456_789)),
            "2025-11-20T21:35:03.123456789Z"
        );
        assert_eq!(
            format_timestamp(timestamp(123_400_000)),
            "2025-11-20T21:35:03.123400Z"
        );
        assert_eq!(
            format_timestamp(timestamp(100_000_000)),
            "2025-11-20T21:35:03.100Z"
        );
        assert_eq!(
            format_timestamp(timestamp(1_000_000)),
            "2025-11-20T21:35:03.001Z"
        );
        assert_eq!(
            format_timestamp(timestamp(100)),
            "2025-11-20T21:35:03.000000100Z"
        );
    }

    #[test]
    fn formats_pre_epoch_timestamps_like_chrono_serde() {
        // Before 1970 `jiff` reports a negative sub-second component; the
        // rendering must still match what chrono wrote.
        let half_second_before_epoch = Timestamp::from_second(-1)
            .unwrap()
            .checked_add(SignedDuration::from_nanos(500_000_000))
            .unwrap();
        assert_eq!(
            format_timestamp(half_second_before_epoch),
            "1969-12-31T23:59:59.500Z"
        );

        let quarter_second_before_epoch = Timestamp::from_second(-2)
            .unwrap()
            .checked_add(SignedDuration::from_nanos(250_000_000))
            .unwrap();
        assert_eq!(
            format_timestamp(quarter_second_before_epoch),
            "1969-12-31T23:59:58.250Z"
        );

        let whole_second_before_epoch = Timestamp::from_second(-1).unwrap();
        assert_eq!(
            format_timestamp(whole_second_before_epoch),
            "1969-12-31T23:59:59Z"
        );
    }

    #[test]
    fn round_trips_pre_epoch_timestamps() {
        for timestamp in [
            Timestamp::from_second(-1).unwrap(),
            Timestamp::from_second(-1)
                .unwrap()
                .checked_add(SignedDuration::from_nanos(500_000_000))
                .unwrap(),
            Timestamp::from_second(-2)
                .unwrap()
                .checked_add(SignedDuration::from_nanos(250_000_000))
                .unwrap(),
        ] {
            let formatted = format_timestamp(timestamp);
            assert_eq!(parse_timestamp(&formatted).unwrap(), timestamp);
        }
    }

    #[test]
    fn round_trips_through_the_formatted_shape() {
        for nanos in [0, 1, 100, 1_000_000, 123_400_000, 123_456_789] {
            let ts = timestamp(nanos);
            let formatted = format_timestamp(ts);
            assert_eq!(parse_timestamp(&formatted).unwrap(), ts);
        }
    }

    #[test]
    fn parses_the_strings_chrono_produced() {
        // `+00:00` offset, `Z` suffix, with and without fractional seconds.
        let expected = timestamp(123_456_000);
        assert_eq!(
            parse_timestamp("2025-11-20T21:35:03.123456+00:00").unwrap(),
            expected
        );
        assert_eq!(
            parse_timestamp("2025-11-20T21:35:03.123456Z").unwrap(),
            expected
        );
        assert_eq!(
            parse_timestamp("2025-11-20T16:35:03.123456-05:00").unwrap(),
            expected
        );
        assert_eq!(
            parse_timestamp("2025-11-20T21:35:03+00:00").unwrap(),
            timestamp(0)
        );
        assert_eq!(
            parse_timestamp("2025-11-20T21:35:03Z").unwrap(),
            timestamp(0)
        );
    }

    #[test]
    fn rejects_non_rfc3339_timestamps() {
        assert!(parse_timestamp("2025-11-20").is_err());
        assert!(parse_timestamp("2025-11-20T21:35:03").is_err());
        assert!(parse_timestamp("not a timestamp").is_err());
    }

    #[test]
    fn parses_iso_dates_like_chrono_did() {
        let expected = Date::new(2026, 1, 31).unwrap();
        // Zero padded, single digit, and leading whitespace were all accepted
        // by chrono's `%Y-%m-%d` parser.
        assert_eq!(parse_iso_date("2026-01-31"), Some(expected));
        assert_eq!(parse_iso_date("2026-1-31"), Some(expected));
        assert_eq!(
            parse_iso_date("2026-01-1"),
            Some(Date::new(2026, 1, 1).unwrap())
        );
        assert_eq!(parse_iso_date(" 2026-01-31"), Some(expected));
        assert_eq!(parse_iso_date("2026-02-29"), None);
        assert_eq!(parse_iso_date("2026-13-01"), None);
        assert_eq!(parse_iso_date("2026-01-31T00:00:00"), None);
        assert_eq!(parse_iso_date("2026-01-31 "), None);
        assert_eq!(parse_iso_date(""), None);
    }

    #[test]
    fn rejects_month_and_day_fields_that_chrono_rejected() {
        // Chrono's `%m` and `%d` were at most two digits wide and carried no
        // sign, so these were rejected there and must be rejected here too.
        assert_eq!(parse_iso_date("2026-011-02"), None);
        assert_eq!(parse_iso_date("2026-01-031"), None);
        assert_eq!(parse_iso_date("2026-01-+1"), None);
        assert_eq!(parse_iso_date("2026-01- 1"), None);
        // A leap day in a leap year is still accepted.
        assert_eq!(
            parse_iso_date("2024-02-29"),
            Some(Date::new(2024, 2, 29).unwrap())
        );
        // The lenient single digit spellings stay accepted.
        assert_eq!(
            parse_iso_date("2026-1-31"),
            Some(Date::new(2026, 1, 31).unwrap())
        );
        assert_eq!(
            parse_iso_date("2026-01-1"),
            Some(Date::new(2026, 1, 1).unwrap())
        );
    }

    #[test]
    fn parses_signed_years_like_chrono_did() {
        // Chrono's `%Y` accepted an explicit sign.
        assert_eq!(
            parse_iso_date("+2026-01-31"),
            Some(Date::new(2026, 1, 31).unwrap())
        );
        assert_eq!(
            parse_iso_date("-2026-01-31"),
            Some(Date::new(-2026, 1, 31).unwrap())
        );
        // An unsigned year is at most four digits wide.
        assert_eq!(parse_iso_date("12345-01-31"), None);
        assert_eq!(
            parse_iso_date("0000-01-31"),
            Some(Date::new(0, 1, 31).unwrap())
        );
        // Out of range years are rejected, like chrono did.
        assert_eq!(parse_iso_date("+99999999-01-31"), None);
    }

    #[test]
    fn serde_adapter_keeps_the_chrono_shape() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Entry {
            #[serde(with = "serde_compat")]
            timestamp: Timestamp,
        }

        let entry = Entry {
            timestamp: timestamp(100_000_000),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert_eq!(json, r#"{"timestamp":"2025-11-20T21:35:03.100Z"}"#);

        let parsed: Entry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.timestamp, entry.timestamp);

        let from_chrono: Entry =
            serde_json::from_str(r#"{"timestamp":"2025-11-20T21:35:03.100+00:00"}"#).unwrap();
        assert_eq!(from_chrono.timestamp, entry.timestamp);
    }
}
