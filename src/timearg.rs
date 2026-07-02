//! Parsing of `--since` / `--until` values: ISO dates/datetimes or
//! relative offsets like `7d`, `24h`, `90m`.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};

pub fn parse_time_arg(arg: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let arg = arg.trim();
    // Relative: <n><d|h|m>
    if let Some(unit) = arg.chars().last() {
        if matches!(unit, 'd' | 'h' | 'm') {
            if let Ok(n) = arg[..arg.len() - 1].parse::<i64>() {
                let delta = match unit {
                    'd' => Duration::days(n),
                    'h' => Duration::hours(n),
                    _ => Duration::minutes(n),
                };
                return Some(now - delta);
            }
        }
    }
    // Full RFC3339
    if let Ok(dt) = DateTime::parse_from_rfc3339(arg) {
        return Some(dt.with_timezone(&Utc));
    }
    // Datetime without offset — interpret as local time
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(arg, fmt) {
            return chrono::Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|dt| dt.with_timezone(&Utc));
        }
    }
    // Bare date — local midnight
    if let Ok(date) = NaiveDate::parse_from_str(arg, "%Y-%m-%d") {
        let naive = date.and_hms_opt(0, 0, 0)?;
        return chrono::Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|dt| dt.with_timezone(&Utc));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap()
    }

    #[test]
    fn relative_offsets() {
        assert_eq!(
            parse_time_arg("7d", now()).unwrap(),
            Utc.with_ymd_and_hms(2026, 6, 25, 12, 0, 0).unwrap()
        );
        assert_eq!(
            parse_time_arg("24h", now()).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap()
        );
        assert_eq!(
            parse_time_arg("90m", now()).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 2, 10, 30, 0).unwrap()
        );
    }

    #[test]
    fn rfc3339_passthrough() {
        assert_eq!(
            parse_time_arg("2026-01-01T00:00:00Z", now()).unwrap(),
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn bare_date_parses() {
        assert!(parse_time_arg("2026-01-01", now()).is_some());
    }

    #[test]
    fn garbage_rejected() {
        assert!(parse_time_arg("yesterday", now()).is_none());
        assert!(parse_time_arg("7w", now()).is_none());
    }
}
