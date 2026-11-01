//! Small formatting helpers shared by view models.

use std::fmt::Write as _;

/// Civil date from days since the epoch, proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (year, month as u32, day as u32)
}

/// `YYYY-MM-DD HH:MM` in the commit's own offset.
pub(crate) fn date(time: i64, offset: i32) -> String {
    let local = time + i64::from(offset);
    let (y, m, d) = civil(local.div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60
    )
}

/// `YYYY-MM-DD` in UTC.
pub(crate) fn day(time: i64) -> String {
    date(time, 0)[..10].to_owned()
}

thread_local! {
    /// `display.dates = "absolute"` for the request rendered on this thread.
    static ABSOLUTE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` with [`ago`] answering a date instead of a distance when `absolute`.
pub(crate) fn with_dates<T>(absolute: bool, f: impl FnOnce() -> T) -> T {
    let prev = ABSOLUTE.with(|a| a.replace(absolute));
    let out = f();
    ABSOLUTE.with(|a| a.set(prev));
    out
}

/// Coarse relative time such as `3 days ago`, or the date under `display.dates = "absolute"`.
pub(crate) fn ago(time: i64, now: i64) -> String {
    if ABSOLUTE.with(std::cell::Cell::get) {
        return day(time);
    }
    let secs = now - time;
    if secs < 60 {
        return "just now".to_owned();
    }
    let (n, unit) = match secs {
        s if s < 3600 => (s / 60, "minute"),
        s if s < 86_400 => (s / 3600, "hour"),
        s if s < 86_400 * 30 => (s / 86_400, "day"),
        s if s < 86_400 * 365 => (s / (86_400 * 30), "month"),
        s => (s / (86_400 * 365), "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Percent-encodes a URL path segment or path; `/` survives only in paths.
pub(crate) fn enc(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

thread_local! {
    /// URL prefix of the repository being rendered on this thread; empty for a single repo.
    static BASE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

thread_local! {
    /// Ref chosen with `?rev=` for this request; `HEAD` when absent.
    static REV: std::cell::RefCell<String> = std::cell::RefCell::new("HEAD".to_owned());
}

tokio::task_local! {
    /// `?rev=` of the request being served, copied into [`REV`] by `app::blocking`.
    pub(crate) static REQUEST_REV: String;
}

/// The one place absolute in-app URLs are built: prefixes the current repository's base and,
/// on pages that follow the selected ref, carries `?rev=` along.
pub(crate) fn href(path: &str) -> String {
    let path = with_selected(path);
    BASE.with(|b| format!("{}{path}", b.borrow()))
}

/// Pages whose content is computed from the selected ref rather than a ref in the path.
fn follows_rev(path: &str) -> bool {
    let p = path.split(['?', '#']).next().unwrap_or_default();
    p == "/" || p == "/stats" || p.starts_with("/stats/") || p.starts_with("/history/")
}

fn with_selected(path: &str) -> std::borrow::Cow<'_, str> {
    let rev = selected();
    if rev == "HEAD" || !follows_rev(path) {
        return path.into();
    }
    let (head, frag) = path
        .split_once('#')
        .map_or((path, None), |(h, f)| (h, Some(f)));
    if head.split(['?', '&']).any(|p| p.starts_with("rev=")) {
        return path.into();
    }
    let sep = if head.contains('?') { '&' } else { '?' };
    let mut out = format!("{head}{sep}rev={}", enc(&rev, false));
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out.into()
}

/// The ref chosen with `?rev=`, `HEAD` when none was given.
pub(crate) fn selected() -> String {
    REV.with(|r| r.borrow().clone())
}

/// `selected` percent-encoded as one path segment, for `/log/{rev}`-style links.
pub(crate) fn selected_seg() -> String {
    enc(&selected(), false)
}

/// Runs `f` with [`selected`] answering `rev`.
pub(crate) fn with_rev<T>(rev: &str, f: impl FnOnce() -> T) -> T {
    let prev = REV.with(|r| r.replace(rev.to_owned()));
    let out = f();
    REV.with(|r| *r.borrow_mut() = prev);
    out
}

/// `?rev=` of a raw query string, empty values and absence reading as `HEAD`.
pub(crate) fn rev_param(query: Option<&str>) -> String {
    query
        .unwrap_or_default()
        .split('&')
        .find_map(|p| p.strip_prefix("rev="))
        .map(|v| {
            let v = v.replace('+', " ");
            percent_decode(&v)
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "HEAD".to_owned())
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| char::from(c).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (
                b.get(i + 1).copied().and_then(hex),
                b.get(i + 2).copied().and_then(hex),
            )
        {
            #[allow(clippy::cast_possible_truncation)]
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Prefix `href` adds on this thread, for templates and the JS meta tag.
pub(crate) fn base() -> String {
    BASE.with(|b| b.borrow().clone())
}

/// Runs `f` with `href` prefixing `base`, restoring the previous prefix afterwards.
pub(crate) fn with_base<T>(base: &str, f: impl FnOnce() -> T) -> T {
    let prev = BASE.with(|b| b.replace(base.to_owned()));
    let out = f();
    BASE.with(|b| *b.borrow_mut() = prev);
    out
}

/// `13972` as `13,972`.
pub(crate) fn thousands(n: impl std::fmt::Display) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Day header such as `Today`, `Yesterday` or `Oct 29`, in the commit's own offset.
pub(crate) fn day_label(time: i64, offset: i32, now: i64) -> String {
    let d = (time + i64::from(offset)).div_euclid(86_400);
    let today = (now + i64::from(offset)).div_euclid(86_400);
    match today - d {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        _ => {
            let (y, m, day) = civil(d);
            let (ty, _, _) = civil(today);
            let mon = MONTHS[(m as usize).saturating_sub(1).min(11)];
            if y == ty {
                format!("{mon} {day}")
            } else {
                format!("{mon} {day}, {y}")
            }
        }
    }
}

pub(crate) fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Escapes element content only: quotes stay as they are, which keeps JSON-heavy sources small.
pub(crate) fn esc_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use crate::fmt::{ago, date, day_label, enc, size, thousands, with_dates};

    #[test]
    fn formats_epoch_and_offset() {
        assert_eq!(date(0, 0), "1970-01-01 00:00");
        assert_eq!(date(1_700_000_000, 3600), "2023-11-14 23:13");
        assert_eq!(date(951_782_400, 0), "2000-02-29 00:00");
    }

    #[test]
    fn relative_and_size() {
        assert_eq!(ago(0, 90), "1 minute ago");
        assert_eq!(ago(0, 86_400 * 3), "3 days ago");
        assert_eq!(
            with_dates(true, || ago(86_400 * 40, 86_400 * 43)),
            "1970-02-10"
        );
        assert_eq!(
            ago(0, 90),
            "1 minute ago",
            "the mode does not leak out of its scope"
        );
        assert_eq!(size(1536), "1.5 KiB");
    }

    #[test]
    fn groups_digits_and_days() {
        assert_eq!(thousands(3_522_647_u64), "3,522,647");
        assert_eq!(thousands(12_u32), "12");
        assert_eq!(day_label(100, 0, 150), "Today");
        assert_eq!(day_label(0, 0, 86_400), "Yesterday");
        assert_eq!(day_label(0, 0, 86_400 * 10), "Jan 1");
        assert_eq!(day_label(0, 0, 86_400 * 400), "Jan 1, 1970");
    }

    #[test]
    fn encodes_paths() {
        assert_eq!(enc("a b/c#", true), "a%20b/c%23");
        assert_eq!(enc("a/b", false), "a%2Fb");
    }
}
