//! Calendar arithmetic shared by every crate.

/// Days since 1970-01-01 to `(year, month, day)`, proleptic Gregorian (Hinnant).
/// Valid for negative days too.
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Inverse of `civil_from_days`.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// First and last second of years 1 to 9999, the range a date can be written in.
pub const FIRST_SECOND: i64 = -62_135_596_800;
/// See [`FIRST_SECOND`].
pub const LAST_SECOND: i64 = 253_402_300_799;

/// `YYYY-MM-DD` of a day number since the epoch.
pub fn ymd(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DDTHH:MM:SSZ` of epoch seconds, `None` outside years 1 to 9999 so a hostile time
/// never prints a wrapped or invented date.
pub fn iso_utc(secs: i64) -> Option<String> {
    if !(FIRST_SECOND..=LAST_SECOND).contains(&secs) {
        return None;
    }
    let rem = secs.rem_euclid(86_400);
    Some(format!(
        "{}T{:02}:{:02}:{:02}Z",
        ymd(secs.div_euclid(86_400)),
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    ))
}

#[cfg(test)]
mod tests {
    use crate::{iso_utc, ymd};

    #[test]
    fn iso_utc_and_ymd_write_known_points_and_refuse_out_of_range() {
        assert_eq!(iso_utc(0).as_deref(), Some("1970-01-01T00:00:00Z"));
        assert_eq!(
            iso_utc(951_782_400).as_deref(),
            Some("2000-02-29T00:00:00Z")
        );
        assert_eq!(iso_utc(-1).as_deref(), Some("1969-12-31T23:59:59Z"));
        assert_eq!(iso_utc(i64::MAX), None);
        assert_eq!(iso_utc(i64::MIN), None);
        assert_eq!(ymd(19_782), "2024-02-29");
    }

    use crate::{civil_from_days, days_from_civil};

    #[test]
    fn civil_round_trip() {
        for d in [-1000, 0, 19_723, 19_782, 20_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
    }

    #[test]
    fn pins_known_dates() {
        let cases = [
            (0, (1970, 1, 1)),
            (59, (1970, 3, 1)),
            (11_016, (2000, 2, 29)),
            (19_782, (2024, 2, 29)),
            (-1, (1969, 12, 31)),
            (-25_567, (1900, 1, 1)),
            (-719_468, (0, 3, 1)),
            (-719_528, (0, 1, 1)),
            (2_932_896, (9999, 12, 31)),
        ];
        for (days, want) in cases {
            assert_eq!(civil_from_days(days), want, "day {days}");
        }
    }
}
