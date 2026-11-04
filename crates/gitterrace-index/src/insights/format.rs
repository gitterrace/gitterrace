//! Numbers as text: thousands separators and a count with its noun.

/// `13972` as `13,972`.
pub fn thousands(n: impl std::fmt::Display) -> String {
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

/// `1 commit`, `2 commits`, `1,000 commits`: the number and its noun, always agreeing.
pub fn count(n: impl std::fmt::Display + Copy, one: &str, many: &str) -> String {
    let word = if n.to_string() == "1" { one } else { many };
    format!("{} {word}", thousands(n))
}
