//! FNV-1a: a hash that is stable across runs and Rust releases, unlike the std hasher. For
//! names and fingerprints that reach disk or a manifest, never for anything adversarial.

const OFFSET_64: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME_64: u64 = 0x0100_0000_01b3;
const OFFSET_32: u32 = 0x811c_9dc5;
const PRIME_32: u32 = 0x0100_0193;

/// FNV-1a 64 over `bytes`, in order.
pub fn fnv1a64(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(OFFSET_64, |h, b| (h ^ u64::from(b)).wrapping_mul(PRIME_64))
}

/// FNV-1a 32 over `bytes`, in order.
pub fn fnv1a32(bytes: impl IntoIterator<Item = u8>) -> u32 {
    bytes.into_iter().fold(OFFSET_32, |h, b| (h ^ u32::from(b)).wrapping_mul(PRIME_32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_test_vectors() {
        assert_eq!(fnv1a64([]), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(*b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(*b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(fnv1a32([]), 0x811c_9dc5);
        assert_eq!(fnv1a32(*b"a"), 0xe40c_292c);
        assert_eq!(fnv1a32(*b"foobar"), 0xbf9c_f968);
    }
}
