//! Waktu: semua epoch ms UTC. Hari = UTC (reset 00:00 UTC = 07:00 WIB).
use crate::types::Ms;
use chrono::{NaiveDate, TimeZone, Utc};

pub const DAY_MS: Ms = 86_400_000;

pub fn utc_day(ms: Ms) -> i64 {
    ms.div_euclid(DAY_MS)
}

pub fn day_start(ms: Ms) -> Ms {
    utc_day(ms) * DAY_MS
}

pub fn now_ms() -> Ms {
    Utc::now().timestamp_millis()
}

/// "YYYY-MM-DD" → epoch ms 00:00 UTC.
pub fn parse_ymd(s: &str) -> Option<Ms> {
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let dt = d.and_hms_opt(0, 0, 0)?;
    Some(Utc.from_utc_datetime(&dt).timestamp_millis())
}

pub fn fmt_ymd(ms: Ms) -> String {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

pub fn fmt_utc(ms: Ms) -> String {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn day_math() {
        let t = parse_ymd("2026-11-05").unwrap();
        assert_eq!(fmt_ymd(t), "2026-11-05");
        assert_eq!(day_start(t + 5), t);
        assert_eq!(utc_day(t - 1) + 1, utc_day(t));
    }
}
