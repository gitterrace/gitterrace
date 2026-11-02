pub(crate) const SANS: &str =
    "ui-sans-serif,system-ui,-apple-system,'Segoe UI',Roboto,'Helvetica Neue',Arial,sans-serif";
pub(crate) const MONO: &str =
    "ui-monospace,'SF Mono',SFMono-Regular,Menlo,Consolas,'Liberation Mono',monospace";

pub(crate) fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if c.is_control() => {}
            c => o.push(c),
        }
    }
    o
}

pub(crate) use gitterrace_index::count;

/// Thousands separators: 12345 -> "12,345".
pub(crate) fn group(n: u64) -> String {
    gitterrace_index::thousands(n)
}

/// Shortens `s` to at most `max` characters with an ellipsis.
pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut o: String = s.chars().take(max.saturating_sub(1)).collect();
        o.push('\u{2026}');
        o
    }
}

/// Formats a float coordinate compactly and deterministically.
pub(crate) fn n(v: f64) -> String {
    let t = format!("{v:.2}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    if t.is_empty() || t == "-" || t == "-0" {
        "0".to_owned()
    } else {
        t.to_owned()
    }
}

/// Filled area path plus a stroke path for a series scaled into a box.
pub(crate) fn area(values: &[u32], x: f64, y: f64, w: f64, h: f64) -> (String, String) {
    if values.is_empty() {
        return (String::new(), String::new());
    }
    let max = f64::from(values.iter().copied().max().unwrap_or(1).max(1));
    let step = if values.len() > 1 {
        w / (values.len() - 1) as f64
    } else {
        0.0
    };
    let pts: Vec<(f64, f64)> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            (
                x + step * i as f64,
                (f64::from(*v) / max).mul_add(-h, y + h),
            )
        })
        .collect();
    let mut line = format!("M{} {}", n(pts[0].0), n(pts[0].1));
    for p in &pts[1..] {
        line.push_str(&format!("L{} {}", n(p.0), n(p.1)));
    }
    let fill = format!(
        "{line}L{} {}L{} {}Z",
        n(pts[pts.len() - 1].0),
        n(y + h),
        n(pts[0].0),
        n(y + h)
    );
    (fill, line)
}

#[cfg(test)]
mod tests {
    use crate::svg::count;

    #[test]
    fn count_agrees_with_its_noun() {
        assert_eq!(count(0, "day", "days"), "0 days");
        assert_eq!(count(1, "day", "days"), "1 day");
        assert_eq!(count(1000, "line", "lines"), "1,000 lines");
    }
}
