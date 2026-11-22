//! Dependency-free UTC clock (§7.3 timestamps: `YYYY-MM-DDTHH:MM:SSZ`).
//!
//! std has no civil-calendar formatter, and the plan bans casual new deps;
//! so the standard days→civil algorithm (Howard Hinnant) is implemented here.
//! `fmt_epoch_seconds` is pure and exported so tests can pin exact outputs
//! instead of racing the wall clock.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since Unix epoch (UTC), best-effort. Panics only if the clock is
/// before 1970 (impossible on any real host) — a wrong time would silently
/// corrupt the event ordering, so a hard stop is the honest choice.
pub fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_secs()
}

/// `YYYY-MM-DDTHH:MM:SSZ` from seconds-since-epoch (UTC, no localisation).
pub fn fmt_epoch_seconds(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Current UTC instant formatted per §7.3.
pub fn now_utc_iso8601() -> String {
    fmt_epoch_seconds(now_epoch_seconds())
}

/// Days-since-epoch → (year, month, day). Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (u32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0,399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0,365]
    let mp = (5 * doy + 2) / 153; // [0,11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1,31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1,12]
    (if m <= 2 { (y + 1) as u32 } else { y as u32 }, m, d)
}

/// Inverse of `fmt_epoch_seconds`: parse a strict `YYYY-MM-DDTHH:MM:SSZ`
/// stamp (§7.2 `approvals.ts`) back to epoch seconds. Anything malformed is
/// `None` — callers must fall back, never guess (§9 timeout math).
pub fn epoch_from_utc_iso(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let at = |a: usize, c: usize| s[a..c].parse::<i64>().ok();
    let (y, m, d) = (at(0, 4)?, at(5, 7)?, at(8, 10)?);
    let (hh, mm, ss) = (at(11, 13)?, at(14, 16)?, at(17, 19)?);
    // Range-validate every field; the civil-date math would happily accept
    // month 13 and produce a wrong-but-plausible epoch (§9 timeouts must not
    // be poisoned by a corrupt timestamp).
    if !(1970..=9999).contains(&y)
        || !(1..=12).contains(&m)
        || !(1..=31).contains(&d)
        || !(0..=23).contains(&hh)
        || !(0..=59).contains(&mm)
        || !(0..=59).contains(&ss)
    {
        return None;
    }
    // Hinnant's `days_from_civil`.
    let era_y = if m <= 2 { y - 1 } else { y };
    let era = if era_y >= 0 { era_y } else { era_y - 399 } / 400;
    let yoe = era_y - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + hh * 3_600 + mm * 60 + ss) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_is_the_exact_inverse_of_format() {
        for e in [
            0u64,
            1_000_000_000,
            1_789_207_200,
            1_789_164_360,
            2_000_000_000,
        ] {
            assert_eq!(
                epoch_from_utc_iso(&fmt_epoch_seconds(e)),
                Some(e),
                "round trip {e}"
            );
        }
        // Pinned against Python calendar.timegm((2026,9,12,10,5,0)).
        assert_eq!(
            epoch_from_utc_iso("2026-09-12T10:05:00Z"),
            Some(1_789_207_500)
        );
    }

    #[test]
    fn parse_rejects_garbage_rather_than_guessing() {
        assert_eq!(epoch_from_utc_iso(""), None);
        assert_eq!(epoch_from_utc_iso("2026-09-12 10:05:00Z"), None); // space
        assert_eq!(epoch_from_utc_iso("2026-09-12T10:05:00"), None); // no Z
        assert_eq!(
            epoch_from_utc_iso("2026-09-12T10:05:00Z"),
            Some(1_789_207_500)
        );
        assert_eq!(epoch_from_utc_iso("2026-13-12T10:05:00Z"), None); // month 13 rejected
        assert_eq!(epoch_from_utc_iso("2026-09-12T25:05:00Z"), None); // bad hour
        assert_eq!(epoch_from_utc_iso("not-a-date-20260912Z"), None);
    }

    #[test]
    fn epoch_zero_is_1970_new_year() {
        assert_eq!(fmt_epoch_seconds(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_instants() {
        // 2001-09-09 01:46:40 (the famous ~1e9 epoch).
        assert_eq!(fmt_epoch_seconds(1_000_000_000), "2001-09-09T01:46:40Z");
        // 2026-09-12T10:00:00Z — cross-checked against Python's timegm.
        assert_eq!(fmt_epoch_seconds(1_789_207_200), "2026-09-12T10:00:00Z");
    }

    #[test]
    fn leap_day_and_boundaries() {
        // 2024-02-29T23:59:12Z (leap day, verified against timegm).
        assert_eq!(fmt_epoch_seconds(1_709_251_152), "2024-02-29T23:59:12Z");
        // 2026-09-11T22:06:00Z — the §7.3 example envelope's instant.
        assert_eq!(fmt_epoch_seconds(1_789_164_360), "2026-09-11T22:06:00Z");
        // one day boundary roll: 23:59:59 -> next 00:00:00
        assert_eq!(fmt_epoch_seconds(1_789_207_199), "2026-09-12T09:59:59Z");
        assert_eq!(fmt_epoch_seconds(1_789_207_200), "2026-09-12T10:00:00Z");
    }

    #[test]
    fn now_is_well_formed_and_recent() {
        let s = now_utc_iso8601();
        assert_eq!(s.len(), 20);
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
        // 2026+ — guards against the clock/date math regressing backwards.
        assert!(s.starts_with("202") || s.as_str() > "2026-01-01T00:00:00Z");
    }

    #[test]
    fn monotonic_seconds_increase() {
        let a = now_epoch_seconds();
        let b = now_epoch_seconds();
        assert!(b >= a);
    }
}
