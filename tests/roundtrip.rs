//! Roundtrips through all framing and transform combinations.
use craft_codec::{
    CompressionCodec, Decoder, Encoder, EncryptionCodec, Framing, Identity, Metadata,
};

fn data(len: usize, mut seed: u64) -> Vec<u8> {
    (0..len)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect()
}

fn exercise<C: CompressionCodec, E: EncryptionCodec>(
    compression: impl Fn() -> C,
    encryption: impl Fn() -> E,
) {
    for framing in [Framing::Fixed(256), Framing::Variable(256)] {
        let lengths = match framing {
            Framing::Fixed(_) => [256, 256, 256, 17],
            _ => [17, 128, 256, 13],
        };
        let frames: Vec<Vec<u8>> = lengths
            .into_iter()
            .enumerate()
            .map(|(i, n)| {
                if i % 2 == 0 {
                    vec![b'A' + i as u8; n]
                } else {
                    data(n, i as u64 + 1)
                }
            })
            .collect();
        let original = frames.concat();
        let mut encoder = Encoder::new(framing, compression(), encryption()).unwrap();
        let mut metadata = Metadata::new(encoder.config());
        let mut stored = Vec::new();
        for (i, frame) in frames.into_iter().enumerate() {
            let mut buffer = frame;
            let sizes = encoder.encode_frame(i as u64, &mut buffer).unwrap();
            stored.extend_from_slice(&buffer);
            metadata.push(sizes).unwrap();
        }
        let metadata = Metadata::from_parts(metadata.into_parts()).unwrap();
        assert_eq!(stored.len() as u64, metadata.stored_len());
        let mut decoder = Decoder::new(metadata.config(), compression(), encryption()).unwrap();
        let len = original.len();
        for (start, end) in [
            (0, len),
            (1, len - 1),
            (16, 18),
            (17, 17),
            (3, 9),
            (len - 1, len),
            (len, len),
        ] {
            let (query, frames) = metadata.range(start as u64, Some(end as u64)).unwrap();
            let mut offset = query.start as usize;
            let mut result = Vec::new();
            for frame in frames {
                let end = offset + frame.spec.stored_len();
                let mut buffer = stored[offset..end].to_vec();
                offset = end;
                decoder.decode_frame(frame.spec, &mut buffer).unwrap();
                result.extend_from_slice(&buffer[frame.selected]);
            }
            assert_eq!(offset as u64, query.end);
            assert_eq!(result, original[start..end]);
        }
    }
}

#[test]
fn raw_fixed_and_variable() {
    exercise(|| Identity, || Identity);
}

#[cfg(feature = "aes-gcm")]
#[test]
fn aes_fixed_and_variable() {
    exercise(|| Identity, || craft_codec::Aes256Gcm::new(&[17; 32]));
}

#[cfg(feature = "lz4")]
#[test]
fn compression_fixed_and_variable() {
    exercise(craft_codec::Lz4::new, || Identity);
}

#[cfg(all(feature = "aes-gcm", feature = "lz4"))]
#[test]
fn compression_and_aes_fixed_and_variable() {
    exercise(craft_codec::Lz4::new, || {
        craft_codec::Aes256Gcm::new(&[23; 32])
    });
}

#[cfg(feature = "lz4")]
#[test]
fn compression_fallback_and_reused_contexts_are_independent() {
    use craft_codec::Lz4;
    let mut encoder = Encoder::new(Framing::Variable(4096), Lz4::new(), Identity).unwrap();
    let mut metadata = Metadata::new(encoder.config());
    let mut records = Vec::new();
    for (i, raw) in [vec![0; 4096], vec![42], data(4096, 42), vec![3; 4096]]
        .into_iter()
        .enumerate()
    {
        let mut buffer = raw.clone();
        let sizes = encoder.encode_frame(i as u64, &mut buffer).unwrap();
        if i == 0 || i == 3 {
            assert!(sizes.payload_len() < sizes.raw_len());
        }
        if i == 1 || i == 2 {
            assert_eq!(sizes.payload_len(), sizes.raw_len());
        }
        metadata.push(sizes).unwrap();
        records.push((raw, buffer));
    }
    for (i, (raw, mut buffer)) in records.into_iter().enumerate().rev() {
        let mut decoder = Decoder::new(metadata.config(), Lz4::new(), Identity).unwrap();
        decoder
            .decode_frame(metadata.frame(i as u64).unwrap(), &mut buffer)
            .unwrap();
        assert_eq!(buffer, raw);
    }
}
