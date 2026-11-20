//! Percent-encoding shared by every crate that builds URLs.

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Percent-encodes every byte outside RFC 3986 unreserved (`A-Za-z0-9-._~`), upper-case hex.
/// `/` is encoded too, so the result is one path segment.
pub fn percent_encode_segment(s: &str) -> String {
    encode(s, false)
}

/// Like `percent_encode_segment`, but `/` survives so a path stays a path.
pub fn percent_encode_path(s: &str) -> String {
    encode(s, true)
}

fn encode(s: &str, keep_slash: bool) -> String {
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

/// Inverse of `percent_encode`: `%XX` back to bytes, then UTF-8 (lossy). A malformed escape
/// stays literal, so decoding never fails.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| char::from(c).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) =
                (b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex))
        {
            out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use crate::{percent_decode, percent_encode_path, percent_encode_segment};

    #[test]
    fn percent_decode_inverts_encode_and_keeps_malformed() {
        for s in ["a b/c#", "\u{e9}\u{65e5}", "!*()%'+"] {
            assert_eq!(percent_decode(&percent_encode_path(s)), s);
        }
        assert_eq!(percent_decode("bad%zz%4"), "bad%zz%4");
    }

    /// RFC 3986: unreserved bytes pass through, the path form also keeps `/`, every other byte
    /// (UTF-8 included) becomes uppercase `%XX`.
    #[test]
    fn percent_encoding_follows_rfc_3986() {
        let cases = [
            ("", "", ""),
            ("AZaz09-._~", "AZaz09-._~", "AZaz09-._~"),
            ("a b/c#", "a%20b/c%23", "a%20b%2Fc%23"),
            ("/", "/", "%2F"),
            ("\u{e9}\u{65e5}", "%C3%A9%E6%97%A5", "%C3%A9%E6%97%A5"),
            ("!*()%'+", "%21%2A%28%29%25%27%2B", "%21%2A%28%29%25%27%2B"),
        ];
        for (s, keep, all) in cases {
            assert_eq!(percent_encode_path(s), keep, "{s:?}");
            assert_eq!(percent_encode_segment(s), all, "{s:?}");
        }
    }
}
