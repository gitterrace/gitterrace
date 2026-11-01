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

#[cfg(test)]
mod tests {
    use crate::civil_from_days;

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
