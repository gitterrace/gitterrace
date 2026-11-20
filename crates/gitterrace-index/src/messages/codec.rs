//! Block compression for the message column: one zstd frame per block.
//!
//! zstd over the deflate the tree already has: on rust's 345k messages deflate made the column
//! 3% smaller and every search 1.4 to 2.5 times slower (a miss 81 to 140 ms against 52 to 59).

/// zstd level: 3 is its default, fast to write and quick to read back.
const LEVEL: i32 = 3;

pub(crate) fn pack(raw: &[u8]) -> std::io::Result<Vec<u8>> {
    zstd::bulk::compress(raw, LEVEL)
}

pub(crate) struct Unpacker(zstd::bulk::Decompressor<'static>);

impl Unpacker {
    pub(crate) fn new() -> std::io::Result<Self> {
        zstd::bulk::Decompressor::new().map(Self)
    }

    /// One block, `None` when it is not one whole frame or inflates past `max`: a block bigger
    /// than the writer makes is damage.
    pub(crate) fn unpack(&mut self, packed: &[u8], max: usize) -> Option<Vec<u8>> {
        self.0.decompress(packed, max).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{Unpacker, pack};

    #[test]
    fn a_block_round_trips_and_damage_is_refused() {
        let raw: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let z = pack(&raw).expect("pack");
        let mut u = Unpacker::new().expect("decoder");
        assert!(u.unpack(&z, 1 << 20) == Some(raw.clone()), "round trip");
        let small = b"short".to_vec();
        assert!(u.unpack(&pack(&small).expect("pack"), 100) == Some(small), "small");
        assert!(u.unpack(&z, 1000).is_none(), "over the bound");
        assert!(u.unpack(&z[..z.len() / 2], 1 << 20).is_none(), "cut");
        assert!(u.unpack(&[0xff; 32], 1 << 20).is_none(), "not a frame");
        assert!(u.unpack(&z, 1 << 20) == Some(raw), "reusable after damage");
    }
}
