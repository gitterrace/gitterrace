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

/// Thousands separators: 12345 -> "12,345".
pub(crate) fn group(n: u64) -> String {
    let s = n.to_string();
    let mut o = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            o.push(',');
        }
        o.push(c);
    }
    o
}

/// 1234 -> "1.2k", `2_500_000` -> "2.5M".
pub(crate) fn compact(n: u64) -> String {
    let f = |v: f64, s: &str| {
        let t = format!("{v:.1}");
        format!("{}{s}", t.strip_suffix(".0").unwrap_or(&t))
    };
    if n >= 1_000_000 {
        f(n as f64 / 1e6, "M")
    } else if n >= 1_000 {
        f(n as f64 / 1e3, "k")
    } else {
        n.to_string()
    }
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

/// Rough text width in px for sans-serif at `size`, good enough to size badges.
pub(crate) fn text_width(s: &str, size: f64) -> f64 {
    s.chars()
        .map(|c| match c {
            'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '|' | '\'' | '!' | 'I' | '1' => 0.31,
            ' ' | 't' | 'f' | 'r' | '-' | '/' | '(' | ')' => 0.38,
            'm' | 'w' | 'M' | 'W' | '%' => 0.88,
            c if c.is_ascii_uppercase() => 0.68,
            c if c.is_ascii_digit() => 0.6,
            _ => 0.56,
        })
        .sum::<f64>()
        * size
}

/// Relative age such as "3 days ago".
pub(crate) fn ago(now: i64, then: i64) -> String {
    let d = (now - then).max(0);
    let (n, unit) = match d {
        0..=59 => return "just now".to_owned(),
        60..=3599 => (d / 60, "minute"),
        3600..=86_399 => (d / 3600, "hour"),
        86_400..=2_591_999 => (d / 86_400, "day"),
        2_592_000..=31_535_999 => (d / 2_592_000, "month"),
        _ => (d / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
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
        .map(|(i, v)| (x + step * i as f64, y + h - f64::from(*v) / max * h))
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
