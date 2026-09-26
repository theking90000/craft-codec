//! Authentication, compressed-data failures and codec limits.
use craft_codec::{
    Compression, Config, Decoder, Encoder, Encryption, Error, FrameSizes, Framing, Identity,
    Metadata,
};

#[test]
fn failed_transforms_clear_output_and_reject_wrong_profiles() {
    let mut encoder = Encoder::new(Framing::Variable(4), Identity, Identity).unwrap();
    for mut input in [vec![], vec![0; 5]] {
        assert_eq!(
            encoder.encode_frame(0, &mut input),
            Err(Error::InvalidFrameSize)
        );
        assert!(input.is_empty());
    }
    let mut metadata = Metadata::new(encoder.config());
    metadata.push(FrameSizes::new(4, 4).unwrap()).unwrap();
    let spec = metadata.frame(0).unwrap();
    let mut decoder = Decoder::new(metadata.config(), Identity, Identity).unwrap();
    for mut buffer in [vec![1; 3], vec![1; 5]] {
        assert_eq!(
            decoder.decode_frame(spec, &mut buffer),
            Err(Error::InvalidStoredLength)
        );
        assert!(buffer.is_empty());
    }
    let other = Config::new(Framing::Fixed(4), Compression::None, Encryption::None).unwrap();
    let mut decoder = Decoder::new(other, Identity, Identity).unwrap();
    let mut buffer = vec![1; 4];
    assert_eq!(
        decoder.decode_frame(spec, &mut buffer),
        Err(Error::ProfileMismatch)
    );
    assert!(buffer.is_empty());
}

#[cfg(feature = "aes-gcm")]
#[test]
fn aes256_nist_known_answer_with_suffix_tag() {
    use craft_codec::Aes256Gcm;
    // NIST AES-256-GCM: zero key, zero 96-bit IV, 16 zero plaintext bytes, no AAD.
    let expected = [
        0xce, 0xa7, 0x40, 0x3d, 0x4d, 0x60, 0x6b, 0x6e, 0x07, 0x4e, 0xc5, 0xd3, 0xba, 0xf3, 0x9d,
        0x18, 0xd0, 0xd1, 0xc8, 0xa7, 0x99, 0x99, 0x6b, 0xf0, 0x26, 0x5b, 0x98, 0xb5, 0xd4, 0x8a,
        0xb9, 0x19,
    ];
    let mut encoder = Encoder::new(Framing::Fixed(16), Identity, Aes256Gcm::new(&[0; 32])).unwrap();
    let mut buffer = vec![0; 16];
    let sizes = encoder.encode_frame(0, &mut buffer).unwrap();
    assert_eq!(buffer, expected);
    let mut metadata = Metadata::new(encoder.config());
    metadata.push(sizes).unwrap();
    let mut decoder = Decoder::new(metadata.config(), Identity, Aes256Gcm::new(&[0; 32])).unwrap();
    decoder
        .decode_frame(metadata.frame(0).unwrap(), &mut buffer)
        .unwrap();
    assert_eq!(buffer, [0; 16]);
}

#[cfg(feature = "aes-gcm")]
#[test]
fn frame_nonce_is_big_endian_and_does_not_reset_at_range_start() {
    use aes_gcm::{KeyInit, aead::AeadInOut};
    let key = [71; 32];
    let mut encoder = Encoder::new(
        Framing::Fixed(16),
        Identity,
        craft_codec::Aes256Gcm::new(&key),
    )
    .unwrap();
    let mut actual = vec![9; 16];
    encoder.encode_frame(0x01020304, &mut actual).unwrap();
    let reference = aes_gcm::Aes256Gcm::new((&key).into());
    let nonce = [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4];
    let mut expected = vec![9; 16];
    let tag = reference
        .encrypt_inout_detached(&nonce.into(), b"", expected.as_mut_slice().into())
        .unwrap();
    expected.extend_from_slice(&tag);
    assert_eq!(actual, expected);
}

#[cfg(feature = "aes-gcm")]
#[test]
fn ciphertext_tag_key_and_index_substitution_all_fail_closed() {
    use craft_codec::Aes256Gcm;
    let mut encoder = Encoder::new(Framing::Fixed(16), Identity, Aes256Gcm::new(&[1; 32])).unwrap();
    let mut good = vec![5; 16];
    let sizes = encoder.encode_frame(0, &mut good).unwrap();
    let mut metadata = Metadata::new(encoder.config());
    metadata.push(sizes).unwrap();
    metadata.push(sizes).unwrap();
    let mut decoder = Decoder::new(metadata.config(), Identity, Aes256Gcm::new(&[1; 32])).unwrap();
    for byte in 0..good.len() {
        let mut corrupted = good.clone();
        corrupted[byte] ^= 1;
        assert_eq!(
            decoder.decode_frame(metadata.frame(0).unwrap(), &mut corrupted),
            Err(Error::AuthenticationFailed)
        );
        assert!(corrupted.is_empty());
    }
    let mut duplicate = good.clone();
    assert_eq!(
        decoder.decode_frame(metadata.frame(1).unwrap(), &mut duplicate),
        Err(Error::AuthenticationFailed)
    );
    assert!(duplicate.is_empty());
    let mut wrong_key =
        Decoder::new(metadata.config(), Identity, Aes256Gcm::new(&[2; 32])).unwrap();
    assert_eq!(
        wrong_key.decode_frame(metadata.frame(0).unwrap(), &mut good),
        Err(Error::AuthenticationFailed)
    );
    assert!(good.is_empty());
}

#[cfg(feature = "aes-gcm")]
#[test]
fn aes_slot_and_frame_limits_are_checked_before_encryption() {
    use craft_codec::{AES_MAX_BYTES, AES_MAX_FRAME_LEN, Aes256Gcm};
    let mut encoder = Encoder::new(
        Framing::Variable(65_536),
        Identity,
        Aes256Gcm::new(&[5; 32]),
    )
    .unwrap();
    assert_eq!(encoder.config().max_frames(), AES_MAX_BYTES / 65_536);
    let mut buffer = vec![1];
    assert_eq!(
        encoder.encode_frame(encoder.config().max_frames(), &mut buffer),
        Err(Error::UsageLimit)
    );
    assert!(buffer.is_empty());
    assert!(
        Encoder::new(
            Framing::Fixed(AES_MAX_FRAME_LEN + 1),
            Identity,
            Aes256Gcm::new(&[5; 32])
        )
        .is_err()
    );
    let mut parts = Metadata::new(encoder.config()).into_parts();
    parts.frame_count = encoder.config().max_frames() + 1;
    assert_eq!(Metadata::from_parts(parts), Err(Error::UsageLimit));
}

#[cfg(feature = "lz4")]
#[test]
fn malformed_compression_wrong_lengths_and_trailing_data_fail() {
    use craft_codec::Lz4;
    let mut encoder = Encoder::new(Framing::Variable(256), Lz4::new(), Identity).unwrap();
    let mut encoded = vec![4; 128];
    let sizes = encoder.encode_frame(0, &mut encoded).unwrap();
    assert!(sizes.payload_len() < 127);
    for raw_len in [127, 129] {
        let mut metadata = Metadata::new(encoder.config());
        metadata
            .push(FrameSizes::new(raw_len, sizes.payload_len()).unwrap())
            .unwrap();
        let mut decoder = Decoder::new(metadata.config(), Lz4::new(), Identity).unwrap();
        let mut buffer = encoded.clone();
        assert_eq!(
            decoder.decode_frame(metadata.frame(0).unwrap(), &mut buffer),
            Err(Error::DecompressionFailed)
        );
        assert!(buffer.is_empty());
    }
    for mut buffer in [
        vec![0xff; encoded.len()],
        [encoded.as_slice(), &[0xff]].concat(),
    ] {
        let mut metadata = Metadata::new(encoder.config());
        metadata
            .push(FrameSizes::new(128, buffer.len() as u64).unwrap())
            .unwrap();
        let mut decoder = Decoder::new(metadata.config(), Lz4::new(), Identity).unwrap();
        assert_eq!(
            decoder.decode_frame(metadata.frame(0).unwrap(), &mut buffer),
            Err(Error::DecompressionFailed)
        );
        assert!(buffer.is_empty());
    }
}

#[cfg(all(feature = "aes-gcm", feature = "lz4"))]
#[test]
fn authentication_precedes_decompression() {
    use craft_codec::{Aes256Gcm, Lz4};
    let mut encoder =
        Encoder::new(Framing::Fixed(128), Lz4::new(), Aes256Gcm::new(&[8; 32])).unwrap();
    let mut buffer = vec![4; 128];
    let sizes = encoder.encode_frame(0, &mut buffer).unwrap();
    let mut metadata = Metadata::new(encoder.config());
    metadata.push(sizes).unwrap();
    buffer[0] ^= 0xff;
    let mut decoder =
        Decoder::new(metadata.config(), Lz4::new(), Aes256Gcm::new(&[8; 32])).unwrap();
    assert_eq!(
        decoder.decode_frame(metadata.frame(0).unwrap(), &mut buffer),
        Err(Error::AuthenticationFailed)
    );
    assert!(buffer.is_empty());
}
