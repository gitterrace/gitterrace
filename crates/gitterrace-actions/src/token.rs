//! The per-process action token. Pages embed it, the run route demands it back: only a
//! same-origin page can read a gitterrace response, so only it can echo the token.

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
        Ok(Self(base64url(&raw)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant time in the content: every byte is compared even after a mismatch.
    pub fn verify(&self, offered: &str) -> bool {
        let (a, b) = (self.0.as_bytes(), offered.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    /// The answer to a challenge: proves the holder knows the secret without sending it.
    pub fn prove(&self, nonce: &str) -> String {
        proof(&self.0, nonce)
    }
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

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[(n >> (18 - 6 * i)) as usize & 63]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::Token;
    use crate::token::{base64url, proof};

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
    fn base64url_matches_known_vectors() {
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
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
