//! Percent-encoding shared by every crate that builds URLs.

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Percent-encodes every byte outside RFC 3986 unreserved (`A-Za-z0-9-._~`), upper-case hex.
/// `/` survives only when `keep_slash`, so a path stays a path and a segment stays one.
pub fn percent_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 15)]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::percent_encode;

    #[test]
    fn pins_old_outputs() {
        let cases = [
            ("", "", ""),
            ("AZaz09-._~", "AZaz09-._~", "AZaz09-._~"),
            ("a b/c#", "a%20b/c%23", "a%20b%2Fc%23"),
            ("/", "/", "%2F"),
            ("\u{e9}\u{65e5}", "%C3%A9%E6%97%A5", "%C3%A9%E6%97%A5"),
            ("!*()%'+", "%21%2A%28%29%25%27%2B", "%21%2A%28%29%25%27%2B"),
        ];
        for (s, keep, all) in cases {
            assert_eq!(percent_encode(s, true), keep, "{s:?}");
            assert_eq!(percent_encode(s, false), all, "{s:?}");
        }
    }
}
