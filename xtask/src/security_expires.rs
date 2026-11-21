//! The `Expires` of security.txt, derived from the changelog so no release step sets it.
//!
//! RFC 9116 makes `Expires` mandatory and wants it under a year out. The newest released
//! section's date plus `SPAN_DAYS` gives a date that moves with every release. A date that has
//! passed tells reporters the contact may be dead, so a test fails then and the fix is a release.

use anyhow::{Context as _, Result};

/// Days from the newest release to `Expires`: under RFC 9116's one year, with room before the
/// next release.
const SPAN_DAYS: i64 = 300;

/// Days since 1970-01-01 of a proleptic Gregorian date.
const fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of `days_from_civil`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The day number of the newest `## [x.y.z] - YYYY-MM-DD` heading of `changelog`.
fn newest_release_day(changelog: &str) -> Result<i64> {
    let line = changelog
        .lines()
        .find(|l| l.starts_with("## [") && l.contains("] - "))
        .context("no released section in the changelog")?;
    let date = line.rsplit("] - ").next().unwrap_or_default().trim();
    let mut parts = date.splitn(3, '-').map(str::parse::<i64>);
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Ok(y)), Some(Ok(m)), Some(Ok(d)))
            if (1..=12).contains(&m) && (1..=31).contains(&d) =>
        {
            Ok(days_from_civil(y, m, d))
        }
        _ => anyhow::bail!("bad release date {date:?} in the changelog"),
    }
}

/// The `Expires` value: the newest release's date plus `SPAN_DAYS`, at midnight UTC.
pub fn expires(changelog: &str) -> Result<String> {
    let (y, m, d) = civil_from_days(newest_release_day(changelog)? + SPAN_DAYS);
    Ok(format!("{y:04}-{m:02}-{d:02}T00:00:00Z"))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const LOG: &str =
        "# Changelog\n\n## [Unreleased]\n\n## [0.2.0] - 2026-11-21\n\n## [0.1.0] - 2026-11-01\n";

    #[test]
    fn expires_is_the_newest_release_plus_the_span() {
        assert_eq!(expires(LOG).unwrap(), "2027-09-17T00:00:00Z");
    }

    #[test]
    fn a_changelog_without_a_release_is_refused() {
        assert!(expires("# Changelog\n").is_err());
    }

    /// The real clock on purpose: this guard is about the calendar, not a rendered page.
    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    }

    #[test]
    fn the_span_stays_under_rfc_9116s_year() {
        const { assert!(SPAN_DAYS < 365) };
    }

    #[test]
    fn the_real_changelog_gives_a_date_in_the_future_and_within_a_year() {
        let log = include_str!("../../CHANGELOG.md");
        let exp = expires(log).unwrap();
        let day = newest_release_day(&format!("## [x] - {}\n", &exp[..10])).unwrap();
        let today = i64::try_from(now() / 86_400).unwrap();
        assert!(day > today, "security.txt Expires {exp} has passed: cut a release");
        assert!(day <= today + 365, "security.txt Expires {exp} is over a year out");
    }
}
