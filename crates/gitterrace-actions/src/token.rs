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
        tag(&self.0, Purpose::Session, "")
    }

    /// Whether `offered` is this secret's `session` value.
    pub fn verify_session(&self, offered: &str) -> bool {
        ct_eq(self.session().as_bytes(), offered.as_bytes())
    }

    /// The cookie's name. Cookies ignore ports, so two servers on `localhost` must not
    /// overwrite each other's: the name carries a few hex digits of a hash of the secret.
    pub fn cookie_name(&self) -> String {
        format!("gt-owner-{}", &tag(&self.0, Purpose::CookieName, "")[..8])
    }

    /// The answer to a challenge: proves the holder knows the secret without sending it.
    pub fn prove(&self, nonce: &str) -> String {
        proof(&self.0, nonce)
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// What a value derived from the secret is for. Every purpose has a key of its own, so a
/// value made for one can never be a value of another: the challenge answers whatever nonce a
/// client picks, and no nonce reaches the session or the cookie name.
#[derive(Clone, Copy)]
enum Purpose {
    /// The answer to a client's nonce, given to any local program that asks.
    Challenge,
    /// The cookie value of a launched browser.
    Session,
    /// The cookie name of a launched browser.
    CookieName,
}

impl Purpose {
    #[cfg(test)]
    const ALL: [Self; 3] = [Self::Challenge, Self::Session, Self::CookieName];

    const fn label(self) -> &'static [u8] {
        match self {
            Self::Challenge => b"challenge",
            Self::Session => b"session",
            Self::CookieName => b"cookie-name",
        }
    }
}

/// SHA-256 over `parts`, each preceded by its length, so no two lists of parts hash the same
/// bytes and a part cannot borrow from its neighbor.
fn framed(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for part in parts {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    h.finalize().into()
}

/// The key of `purpose`: unrelated to the secret and to every other purpose's key, and one
/// way, so a value made with it reveals none of them.
fn key(secret: &str, purpose: Purpose) -> [u8; 32] {
    framed(&[b"gitterrace key v2", purpose.label(), secret.as_bytes()])
}

/// The value of `purpose` over `data`, lowercase hex. The purpose is in the message as well
/// as in the key, so a value of one purpose is wrong for another even by a key collision.
fn tag(secret: &str, purpose: Purpose, data: &str) -> String {
    let k = key(secret, purpose);
    let digest = framed(&[b"gitterrace tag v2", purpose.label(), &k, data.as_bytes()]);
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        s.push(char::from(b"0123456789abcdef"[usize::from(b >> 4)]));
        s.push(char::from(b"0123456789abcdef"[usize::from(b & 15)]));
        s
    })
}

/// The answer to a challenge `nonce`: the one value of the secret that leaves the process.
///
/// A client that read the secret from its 0600 file checks a server's answer before telling
/// it anything, so a foreign listener that only mimics the replies never learns a path. Any
/// local program may ask, so the answer is made with a key that makes nothing else.
pub fn proof(secret: &str, nonce: &str) -> String {
    tag(secret, Purpose::Challenge, nonce)
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
    use crate::token::{Base64, Purpose, base64, key, proof, tag};

    #[test]
    fn values_match_the_documented_derivation() {
        // Length-prefixed (u64 big endian) SHA-256 of ("gitterrace key v2", purpose, secret),
        // then of ("gitterrace tag v2", purpose, key, data): see `framed`.
        assert_eq!(
            proof("abc", "n1"),
            "29fdbe6e162fdef8356ca5ff2fc1b6803a0b83b38c09f397c8d1f0cbec260937"
        );
        assert_eq!(
            tag("abc", Purpose::Session, ""),
            "c8605d5eb9aba59d2987dde0b8f64638928a93c45f70c311590e6484aec7f877"
        );
        assert_eq!(&tag("abc", Purpose::CookieName, "")[..8], "3921fc9d");
        assert_ne!(proof("abc", "n1"), proof("abx", "n1"));
        assert_ne!(proof("abc", "n1"), proof("abc", "n2"));
    }

    /// Class: a value made for one purpose is never a value of another, whatever data a client
    /// can pick. The challenge answers any nonce, so no nonce may reach the session or the
    /// cookie name. Also catches a purpose added to `ALL` that reuses a label, and so a key.
    #[test]
    fn no_challenge_answer_is_a_session_or_cookie_name() {
        let t = Token::generate().expect("rng");
        let s = t.as_str();
        let (session, name) = (t.session(), t.cookie_name());
        let picked = [
            "session",
            "cookie-name",
            "challenge",
            "",
            "gt-owner-",
            &session,
            &name,
            s,
            &format!("{s}:session"),
            &format!("{s}:cookie-name"),
            &session[..8],
        ];
        for nonce in picked {
            let answer = t.prove(nonce);
            assert_ne!(answer, session, "{nonce:?}");
            assert_ne!(&answer[..8], &name["gt-owner-".len()..], "{nonce:?}");
        }
        for (i, a) in Purpose::ALL.iter().enumerate() {
            for b in &Purpose::ALL[i + 1..] {
                assert_ne!(a.label(), b.label());
                assert_ne!(key(s, *a), key(s, *b), "two purposes share a key");
                for data in picked {
                    assert_ne!(tag(s, *a, data), tag(s, *b, data), "{data:?}");
                }
            }
        }
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
