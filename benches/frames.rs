//! Reproducible standalone codec benchmark; no benchmark framework dependency.
use craft_codec::{
    CompressionCodec, Decoder, Encoder, EncryptionCodec, FrameSizes, Framing, Identity, Metadata,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::{
    alloc::System,
    hint::black_box,
    time::{Duration, Instant},
};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn measure(mut f: impl FnMut()) -> (u64, Duration) {
    let start = Instant::now();
    let mut count = 0;
    loop {
        for _ in 0..16 {
            f();
            count += 1;
        }
        if start.elapsed() >= Duration::from_millis(25) {
            return (count, start.elapsed());
        }
    }
}

fn report(
    profile: &str,
    corpus: &str,
    operation: &str,
    size: usize,
    stored: usize,
    samples: (u64, Duration),
    allocation_counts: (usize, usize),
) {
    let (iterations, elapsed) = samples;
    let (allocations, reallocations) = allocation_counts;
    let ns = elapsed.as_secs_f64() * 1e9 / iterations as f64;
    let mib = size as f64 * iterations as f64 / elapsed.as_secs_f64() / 1_048_576.0;
    println!(
        "{profile},{corpus},{operation},{size},{stored},{iterations},{ns:.0},{mib:.2},{allocations},{reallocations}"
    );
}

fn bench<C: CompressionCodec, E: EncryptionCodec>(
    profile: &str,
    corpus: &str,
    raw: &[u8],
    compression: impl Fn() -> C,
    encryption: impl Fn() -> E,
) {
    let mut encoder = Encoder::new(
        Framing::Fixed(raw.len() as u64),
        compression(),
        encryption(),
    )
    .unwrap();
    let mut decoder = Decoder::new(encoder.config(), compression(), encryption()).unwrap();
    let mut buffer = raw.to_vec();
    let sizes = encoder.encode_frame(0, &mut buffer).unwrap();
    let encoded = buffer.clone();
    let stored = encoded.len();
    let mut metadata = Metadata::new(encoder.config());
    metadata.push(sizes).unwrap();
    let spec = metadata.frame(0).unwrap();
    // Repeated encryption in this benchmark has identical key, nonce and payload.
    // Production code must assign a distinct slot to every different plaintext.
    let mut encode = || {
        buffer.clear();
        buffer.extend_from_slice(raw);
        black_box(encoder.encode_frame(0, black_box(&mut buffer)).unwrap());
    };
    for _ in 0..8 {
        encode();
    }
    let region = Region::new(GLOBAL);
    let samples = measure(encode);
    let stats = region.change();
    report(
        profile,
        corpus,
        "encode_with_input_copy",
        raw.len(),
        stored,
        samples,
        (stats.allocations, stats.reallocations),
    );
    let mut decode = || {
        buffer.clear();
        buffer.extend_from_slice(&encoded);
        decoder.decode_frame(spec, black_box(&mut buffer)).unwrap();
        black_box(&buffer);
    };
    for _ in 0..8 {
        decode();
    }
    let region = Region::new(GLOBAL);
    let samples = measure(decode);
    let stats = region.change();
    report(
        profile,
        corpus,
        "decode_with_input_copy",
        raw.len(),
        stored,
        samples,
        (stats.allocations, stats.reallocations),
    );
}

fn direct(corpus: &str, raw: &[u8]) {
    #[cfg(feature = "aes-gcm")]
    {
        use aes_gcm::{KeyInit, aead::AeadInOut};
        let cipher = aes_gcm::Aes256Gcm::new((&[3u8; 32]).into());
        let mut buffer = raw.to_vec();
        let region = Region::new(GLOBAL);
        let samples = measure(|| {
            buffer.copy_from_slice(raw);
            black_box(
                cipher
                    .encrypt_inout_detached(
                        &[0u8; 12].into(),
                        b"",
                        black_box(buffer.as_mut_slice()).into(),
                    )
                    .unwrap(),
            );
        });
        report(
            "direct_aes",
            corpus,
            "encode_with_input_copy",
            raw.len(),
            raw.len() + 16,
            samples,
            (region.change().allocations, region.change().reallocations),
        );
    }
    #[cfg(feature = "lz4")]
    {
        use lz4_flex::block::{CompressTable, compress_into_with_table, get_maximum_output_size};
        let mut table = CompressTable::large();
        let mut output = vec![0; get_maximum_output_size(raw.len())];
        let written = compress_into_with_table(raw, &mut output, &mut table).unwrap();
        let region = Region::new(GLOBAL);
        let samples = measure(|| {
            black_box(compress_into_with_table(black_box(raw), &mut output, &mut table).unwrap());
        });
        report(
            "direct_lz4",
            corpus,
            "compress_only_no_copy",
            raw.len(),
            written,
            samples,
            (region.change().allocations, region.change().reallocations),
        );
    }
    let _ = (corpus, raw);
}

fn main() {
    println!(
        "profile,corpus,operation,raw_bytes,stored_bytes,iterations,ns_per_frame,mib_per_sec,allocations,reallocations"
    );
    for size in [4096, 16_384, 65_536, 262_144, 1_048_576] {
        let mut random = 42u64;
        let noise: Vec<u8> = (0..size)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                random as u8
            })
            .collect();
        for (corpus, raw) in [
            ("repeated", vec![b'A'; size]),
            ("noise", noise.clone()),
            (
                "mixed",
                [vec![b'A'; size / 2], noise[..size / 2].to_vec()].concat(),
            ),
        ] {
            bench("identity", corpus, &raw, || Identity, || Identity);
            #[cfg(feature = "aes-gcm")]
            bench(
                "aes",
                corpus,
                &raw,
                || Identity,
                || craft_codec::Aes256Gcm::new(&[1; 32]),
            );
            #[cfg(feature = "lz4")]
            bench("lz4", corpus, &raw, craft_codec::Lz4::new, || Identity);
            #[cfg(all(feature = "aes-gcm", feature = "lz4"))]
            bench("lz4_aes", corpus, &raw, craft_codec::Lz4::new, || {
                craft_codec::Aes256Gcm::new(&[2; 32])
            });
            direct(corpus, &raw);
        }
    }
    let config = craft_codec::Config::new(
        Framing::Variable(65_536),
        craft_codec::Compression::Lz4,
        craft_codec::Encryption::None,
    )
    .unwrap();
    let mut metadata = Metadata::new(config);
    for _ in 0..16_384 {
        metadata
            .push(FrameSizes::new(65_536, 1000).unwrap())
            .unwrap();
    }
    let region = Region::new(GLOBAL);
    let samples = measure(|| {
        black_box(metadata.range(900 * 1_048_576, Some(900 * 1_048_576 + 16))).unwrap();
    });
    let stats = region.change();
    report(
        "metadata",
        "16384_frames",
        "range_near_end",
        0,
        0,
        samples,
        (stats.allocations, stats.reallocations),
    );
}
