//! Small formatting helpers shared by view models.

use gitterrace_git::{civil_from_days as civil, percent_decode};

/// The one percent-encoding rule: `enc_path` keeps `/`, `enc_seg` encodes it.
pub(crate) use gitterrace_git::{
    percent_encode_path as enc_path, percent_encode_segment as enc_seg,
};

/// The favicon `<link>`, the same file the page templates include, for pages built as strings.
pub(crate) const FAVICON: &str = include_str!("../templates/_favicon.html");

/// How a commit time renders when it is outside years 1 to 9999 (a corrupt or hostile
/// timestamp, such as `i64::MIN`): never a wrapped or invented date.
pub(crate) const UNKNOWN_DATE: &str = "unknown date";

/// First and last second of years 1 to 9999, the range a date can be written in.
const FIRST_SECOND: i64 = -62_135_596_800;
const LAST_SECOND: i64 = 253_402_300_799;

/// `time` shifted into its offset, or `None` when the sum overflows or leaves years 1 to
/// 9999. Every date helper goes through this, so none does unchecked time arithmetic.
fn local(time: i64, offset: i32) -> Option<i64> {
    time.checked_add(i64::from(offset))
        .filter(|t| (FIRST_SECOND..=LAST_SECOND).contains(t))
}

/// `YYYY-MM-DD HH:MM` in the commit's own offset.
pub(crate) fn date(time: i64, offset: i32) -> String {
    let Some(local) = local(time, offset) else {
        return UNKNOWN_DATE.to_owned();
    };
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
    let d = date(time, 0);
    d.get(..10)
        .filter(|_| d != UNKNOWN_DATE)
        .unwrap_or(&d)
        .to_owned()
}

thread_local! {
    /// `display.dates = "absolute"` for the request rendered on this thread.
    static ABSOLUTE: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
}

/// Runs `f` with the thread-local `key` set to `value`. A drop guard restores the previous
/// value, so a panic inside `f` cannot leave one request's state on a pooled thread.
fn scoped<V: 'static, T>(
    key: &'static std::thread::LocalKey<std::cell::RefCell<V>>,
    value: V,
    f: impl FnOnce() -> T,
) -> T {
    struct Restore<V: 'static> {
        key: &'static std::thread::LocalKey<std::cell::RefCell<V>>,
        prev: Option<V>,
    }
    impl<V: 'static> Drop for Restore<V> {
        fn drop(&mut self) {
            if let Some(prev) = self.prev.take() {
                self.key.with(|c| *c.borrow_mut() = prev);
            }
        }
    }
    let _restore = Restore {
        key,
        prev: Some(key.with(|c| c.replace(value))),
    };
    f()
}

/// Runs `f` with [`ago`] answering a date instead of a distance when `absolute`.
pub(crate) fn with_dates<T>(absolute: bool, f: impl FnOnce() -> T) -> T {
    scoped(&ABSOLUTE, absolute, f)
}

/// Coarse relative time such as `3 days ago`, or the date under `display.dates = "absolute"`.
pub(crate) fn ago(time: i64, now: i64) -> String {
    if ABSOLUTE.with(|a| *a.borrow()) {
        return day(time);
    }
    if local(time, 0).is_none() {
        return UNKNOWN_DATE.to_owned();
    }
    // A time in range and a `now` anywhere: saturate, a far future time reads as just now.
    let secs = now.saturating_sub(time);
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
    // The equivalence oracle freezes the clock so "3 days ago" does not move between runs.
    if let Some(t) = std::env::var("GITTERRACE_TEST_NOW")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return t;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

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
    let mut out = format!("{head}{sep}rev={}", enc_seg(&rev));
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
    enc_seg(&selected())
}

/// Runs `f` with [`selected`] answering `rev`.
pub(crate) fn with_rev<T>(rev: &str, f: impl FnOnce() -> T) -> T {
    scoped(&REV, rev.to_owned(), f)
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

/// Prefix `href` adds on this thread, for templates and the JS meta tag.
pub(crate) fn base() -> String {
    BASE.with(|b| b.borrow().clone())
}

/// Runs `f` with `href` prefixing `base`, restoring the previous prefix afterwards.
pub(crate) fn with_base<T>(base: &str, f: impl FnOnce() -> T) -> T {
    scoped(&BASE, base.to_owned(), f)
}

pub(crate) use gitterrace_index::{count, thousands};

/// The counted noun for `n`: `commit` for 1, `commits` otherwise. Spelled out, never `+s`.
pub(crate) fn noun(
    n: impl std::fmt::Display,
    one: &'static str,
    many: &'static str,
) -> &'static str {
    if n.to_string() == "1" { one } else { many }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Day header such as `Today`, `Yesterday` or `Oct 29`, in the commit's own offset.
pub(crate) fn day_label(time: i64, offset: i32, now: i64) -> String {
    let Some(t) = local(time, offset) else {
        return UNKNOWN_DATE.to_owned();
    };
    let d = t.div_euclid(86_400);
    // `now` comes from the clock (or a test), clamped into range so the subtraction is exact.
    let today = now
        .saturating_add(i64::from(offset))
        .clamp(FIRST_SECOND, LAST_SECOND)
        .div_euclid(86_400);
    match today - d {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        _ => {
            let (y, m, day) = civil(d);
            let (ty, _, _) = civil(today);
            let mon = MONTHS[usize::try_from(m - 1).unwrap_or(0).min(11)];
            if y == ty {
                format!("{mon} {day}")
            } else {
                format!("{mon} {day}, {y}")
            }
        }
    }
}

/// The one rendering of a commit id: a link to its commit page showing the short form, the
/// full id on hover. Returns finished HTML (ids are hex, still escaped), `""` for no id.
pub(crate) fn sha_link(full: &str) -> String {
    sha_link_at(&base(), full)
}

/// [`sha_link`] for code that runs outside a request (the watcher), with the URL prefix given.
pub(crate) fn sha_link_at(base: &str, full: &str) -> String {
    if full.is_empty() {
        return String::new();
    }
    format!(
        r#"<a class="sha" href="{}/commit/{}" data-tip="{}">{}</a>"#,
        esc(base),
        esc(full),
        esc(full),
        esc(gitterrace_git::short_id(full)),
    )
}

/// Escapes element content only: quotes stay as they are, which keeps JSON-heavy sources small.
pub(crate) fn esc_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escapes text for an HTML attribute or element: `& < > "`.
pub fn esc(s: &str) -> String {
    esc_text(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use crate::fmt::{UNKNOWN_DATE, ago, date, day, day_label, size, thousands, with_dates};

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
    fn count_agrees_with_its_noun() {
        use crate::fmt::{count, noun};
        assert_eq!(count(0_u32, "commit", "commits"), "0 commits");
        assert_eq!(count(1_u32, "commit", "commits"), "1 commit");
        assert_eq!(count(2_usize, "commit", "commits"), "2 commits");
        assert_eq!(count(1000_i64, "commit", "commits"), "1,000 commits");
        assert_eq!(count(1_u64, "repository", "repositories"), "1 repository");
        assert_eq!(count(3_u64, "repository", "repositories"), "3 repositories");
        assert_eq!(count(2_u64, "person", "people"), "2 people");
        assert_eq!(noun(1_u8, "match", "matches"), "match");
        assert_eq!(noun(11_u8, "match", "matches"), "matches");
    }

    #[test]
    fn request_state_is_restored_when_the_handler_panics() {
        use crate::fmt::{base, selected, with_base, with_rev};
        let caught = std::panic::catch_unwind(|| {
            with_dates(true, || {
                with_base("/r/x", || with_rev("dev", || panic!("handler")))
            })
        });
        assert!(caught.is_err());
        assert_eq!(selected(), "HEAD");
        assert_eq!(base(), "");
        assert_eq!(ago(0, 90), "1 minute ago");
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

    /// A commit can carry any i64 time and any i32 offset (the adversarial fixture has
    /// `i64::MIN`). No date helper may overflow, and a time outside years 1 to 9999 renders as
    /// [`UNKNOWN_DATE`], never as a wrapped or garbage date.
    #[test]
    fn extreme_times_never_overflow_and_read_as_unknown() {
        let times = [i64::MIN, i64::MIN + 1, -1, 0, i64::MAX - 1, i64::MAX];
        let offsets = [i32::MIN, -1, 0, 1, i32::MAX];
        for &t in &times {
            for &o in &offsets {
                for &now in &times {
                    let _ = (date(t, o), day(t), day_label(t, o, now), ago(t, now));
                    let _ = with_dates(true, || ago(t, now));
                }
            }
        }
        for t in [i64::MIN, i64::MAX] {
            assert_eq!(date(t, 0), UNKNOWN_DATE);
            assert_eq!(day(t), UNKNOWN_DATE);
            assert_eq!(day_label(t, 0, 1_700_000_000), UNKNOWN_DATE);
            assert_eq!(ago(t, 1_700_000_000), UNKNOWN_DATE);
        }
        // The edges of the range still read as dates.
        assert_eq!(day(-62_135_596_800), "0001-01-01");
        assert_eq!(day(253_402_300_799), "9999-12-31");
        assert_eq!(day(253_402_300_800), UNKNOWN_DATE);
    }
}
