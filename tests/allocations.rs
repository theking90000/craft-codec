//! Measure allocations after warming both scratch buffers.
//! Run without libtest: its runner can allocate concurrently with the test,
//! and the global allocator counts allocations from every thread.
use craft_codec::{Compression, Config, Decoder, Encoder, Encryption, Framing, Metadata};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn exercise(compression: Compression, encryption: Encryption) {
    let mut metadata =
        Metadata::new(Config::new(Framing::Fixed(65_536), compression, encryption).unwrap());
    let mut encoder = Encoder::try_from(&metadata).unwrap();
    let mut decoder = Decoder::try_from(&metadata).unwrap();
    let mut buffer = vec![0; 65_536];
    // Keep the same plaintext; encode once per original object index.
    for index in 0..4 {
        let sizes = encoder.encode_frame(index, &mut buffer).unwrap();
        metadata.push(sizes).unwrap();
        decoder
            .decode_frame(metadata.frame(index).unwrap(), &mut buffer)
            .unwrap();
    }
    let sizes =
        craft_codec::FrameSizes::new(65_536, metadata.frame(0).unwrap().payload_len() as u64)
            .unwrap();
    // Prebuild descriptors so compact-index growth is outside the codec measurement.
    for _ in 4..132 {
        metadata.push(sizes).unwrap();
    }
    // Warm the complete steady-state path, including range selection.
    for index in 4..8 {
        encoder.encode_frame(index, &mut buffer).unwrap();
        decoder
            .decode_frame(metadata.frame(index).unwrap(), &mut buffer)
            .unwrap();
        let (_, frames) = metadata.range(50, Some(50_213)).unwrap();
        assert_eq!(frames.remaining(), 1);
    }
    let region = Region::new(GLOBAL);
    for index in 8..132 {
        encoder.encode_frame(index, &mut buffer).unwrap();
        decoder
            .decode_frame(metadata.frame(index).unwrap(), &mut buffer)
            .unwrap();
        let (_, frames) = metadata.range(50, Some(50_213)).unwrap();
        assert_eq!(frames.remaining(), 1);
    }
    let stats = region.change();
    assert_eq!(stats.allocations, 0, "{stats:?}");
    assert_eq!(stats.reallocations, 0, "{stats:?}");
    assert!(buffer.iter().all(|&byte| byte == 0));
}

fn main() {
    exercise(Compression::None, Encryption::None);
    #[cfg(feature = "aes-gcm")]
    exercise(Compression::None, Encryption::Aes256Gcm { key: [12; 32] });
    #[cfg(feature = "lz4")]
    exercise(Compression::Lz4, Encryption::None);
    #[cfg(all(feature = "aes-gcm", feature = "lz4"))]
    exercise(Compression::Lz4, Encryption::Aes256Gcm { key: [13; 32] });
}
