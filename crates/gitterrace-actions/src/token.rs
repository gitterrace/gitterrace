//! The per-process owner secret, never shown in a page.
//!
//! A browser the owner opened through the launch link holds a value derived from it (`session`)
//! in an `HttpOnly` cookie, and every write demands that value back. Another OS user can fetch
//! any page over loopback, but cannot read the 0600 file the secret lives in.

use std::fmt;

use sha2::{Digest, Sha256};

/// Random bytes in the token. 32 is beyond guessing at any request rate.
const TOKEN_BYTES: usize = 32;

/// Held in memory only; `Debug` redacts it so it never reaches a log.
pub struct Token(String);

impl Token {
    /// A fresh token from the OS RNG, base64url without padding.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut raw = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut raw)?;
        Ok(Self(base64(&raw, Base64::UrlSafe)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant time in the content: every byte is compared even after a mismatch.
    pub fn verify(&self, offered: &str) -> bool {
        ct_eq(self.0.as_bytes(), offered.as_bytes())
    }

    /// What a browser holds after the launch link: a hash of the secret, so the cookie jar
    /// and a browser history never hold the secret from the 0600 file itself.
    pub fn session(&self) -> String {
        proof(&self.0, "session")
    }

    /// Whether `offered` is this secret's `session` value.
    pub fn verify_session(&self, offered: &str) -> bool {
        ct_eq(self.session().as_bytes(), offered.as_bytes())
    }

    /// The cookie's name. Cookies ignore ports, so two servers on `localhost` must not
    /// overwrite each other's: the name carries a few hex digits of a hash of the secret.
    pub fn cookie_name(&self) -> String {
        format!("gt-owner-{}", &proof(&self.0, "cookie-name")[..8])
    }

    /// The answer to a challenge: proves the holder knows the secret without sending it.
    pub fn prove(&self, nonce: &str) -> String {
        proof(&self.0, nonce)
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// SHA-256 over `secret:nonce`, lowercase hex.
///
/// A client that read the secret from its 0600 file checks a server's answer before telling
/// it anything, so a foreign listener that only mimics the replies never learns a path.
pub fn proof(secret: &str, nonce: &str) -> String {
    let digest = Sha256::new()
        .chain_update(secret.as_bytes())
        .chain_update(b":")
        .chain_update(nonce.as_bytes())
        .finalize();
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        s.push(char::from(b"0123456789abcdef"[usize::from(b >> 4)]));
        s.push(char::from(b"0123456789abcdef"[usize::from(b & 15)]));
        s
    })
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

/// The two base64 forms the product writes: a token travels in a URL, a font in a data URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base64 {
    /// RFC 4648 section 4 alphabet with `=` padding.
    Standard,
    /// RFC 4648 section 5 alphabet (`-` and `_`) without padding.
    UrlSafe,
}

/// `bytes` as base64 in the given form.
pub fn base64(bytes: &[u8], form: Base64) -> String {
    let tail: &[u8; 2] = match form {
        Base64::Standard => b"+/",
        Base64::UrlSafe => b"-_",
    };
    let digit = |n: u32| match n as u8 & 63 {
        v @ 0..=25 => char::from(b'A' + v),
        v @ 26..=51 => char::from(b'a' + v - 26),
        v @ 52..=61 => char::from(b'0' + v - 52),
        v => char::from(tail[usize::from(v - 62)]),
    };
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(digit(n >> (18 - 6 * i)));
            } else if form == Base64::Standard {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::Token;
    use crate::token::{Base64, base64, proof};

    #[test]
    fn proof_is_sha256_of_secret_and_nonce() {
        // printf 'abc:n1' | shasum -a 256
        assert_eq!(
            proof("abc", "n1"),
            "4fd321db886a203ea5ddbf0c3b79a25315577abda03f8cb75d3a09d32b18eef0"
        );
        assert_ne!(proof("abc", "n1"), proof("abx", "n1"));
        assert_ne!(proof("abc", "n1"), proof("abc", "n2"));
    }

    #[test]
    fn base64_matches_the_rfc_vectors_in_both_forms() {
        let cases: [(&[u8], &str, &str); 5] = [
            (b"", "", ""),
            (b"f", "Zg==", "Zg"),
            (b"fo", "Zm8=", "Zm8"),
            (b"foo", "Zm9v", "Zm9v"),
            (b"foob", "Zm9vYg==", "Zm9vYg"),
        ];
        for (raw, std, url) in cases {
            assert_eq!(base64(raw, Base64::Standard), std);
            assert_eq!(base64(raw, Base64::UrlSafe), url);
        }
        assert_eq!(base64(&[0xfb, 0xff], Base64::UrlSafe), "-_8");
        assert_eq!(base64(&[0xfb, 0xff], Base64::Standard), "+/8=");
    }

    #[test]
    fn session_and_cookie_name_derive_from_the_secret_without_revealing_it() {
        let (a, b) = (Token::generate().expect("rng"), Token::generate().expect("rng"));
        assert!(a.verify_session(&a.session()));
        assert!(!a.verify_session(&b.session()));
        assert!(!a.verify_session(a.as_str()), "the secret itself is not the session");
        assert!(!a.session().contains(a.as_str()));
        assert_ne!(a.cookie_name(), b.cookie_name());
        assert!(a.cookie_name().starts_with("gt-owner-"));
    }

    #[test]
    fn tokens_are_fresh_and_verify_exactly() {
        let t = Token::generate().expect("rng");
        assert_eq!(t.as_str().len(), 43);
        assert_ne!(t.as_str(), Token::generate().expect("rng").as_str());
        assert!(t.verify(t.as_str()));
        let mut off = t.as_str().as_bytes().to_vec();
        off[10] = if off[10] == b'A' { b'B' } else { b'A' };
        assert!(!t.verify(std::str::from_utf8(&off).expect("ascii")));
        assert!(!t.verify(""));
        assert!(!t.verify(&t.as_str()[1..]));
        assert_eq!(format!("{t:?}"), "Token(..)");
    }
}
