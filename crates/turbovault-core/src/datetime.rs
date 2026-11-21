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
/// `NaiveDate::parse_from_str(raw, "%Y-%m-%d")`: one- or two-digit months and
/// days are accepted and leading whitespace is ignored. `None` is returned
/// for anything else, including trailing garbage.
#[must_use]
pub fn parse_iso_date(raw: &str) -> Option<Date> {
    let trimmed = raw.trim_start();
    let (negative, rest) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed),
    };

    let mut parts = rest.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: i32 = parts.next()?.parse().ok()?;
    let day: i32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let year = if negative { -year } else { year };
    Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()
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
