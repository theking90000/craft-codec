//! One serial test measures allocations after warming both scratch buffers.
use craft_codec::{
    CompressionCodec, Decoder, Encoder, EncryptionCodec, Framing, Identity, Metadata,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn exercise<C: CompressionCodec, E: EncryptionCodec>(
    compression: impl Fn() -> C,
    encryption: impl Fn() -> E,
) {
    let mut encoder = Encoder::new(Framing::Fixed(65_536), compression(), encryption()).unwrap();
    let mut decoder = Decoder::new(encoder.config(), compression(), encryption()).unwrap();
    let mut buffer = vec![0; 65_536];
    let mut metadata = Metadata::new(encoder.config());
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
    let region = Region::new(GLOBAL);
    for index in 4..132 {
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

#[test]
fn codecs_and_ranges_allocate_nothing_after_warmup() {
    exercise(|| Identity, || Identity);
    #[cfg(feature = "aes-gcm")]
    exercise(|| Identity, || craft_codec::Aes256Gcm::new(&[12; 32]));
    #[cfg(feature = "lz4")]
    exercise(craft_codec::Lz4::new, || Identity);
    #[cfg(all(feature = "aes-gcm", feature = "lz4"))]
    exercise(craft_codec::Lz4::new, || {
        craft_codec::Aes256Gcm::new(&[13; 32])
    });
}
