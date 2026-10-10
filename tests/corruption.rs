//! Authentication, compressed-data failures and codec limits.
use craft_codec::{
    Compression, Config, Decoder, Encoder, Encryption, Error, FrameSizes, Framing, Metadata,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn object(
    framing: Framing,
    compression: Compression,
    encryption: Encryption,
) -> craft_codec::Result<(Encoder, Metadata)> {
    let metadata = Metadata::new(Config::new(framing, compression, encryption)?);
    Ok(((&metadata).try_into()?, metadata))
}

#[test]
fn failed_transforms_clear_output_and_reject_wrong_profiles() -> TestResult {
    let (mut encoder, mut metadata) =
        object(Framing::Variable(4), Compression::None, Encryption::None)?;
    for mut input in [vec![], vec![0; 5]] {
        assert_eq!(
            encoder.encode_frame(0, &mut input),
            Err(Error::InvalidFrameSize)
        );
        assert!(input.is_empty());
    }
    metadata.push(FrameSizes::new(4, 4)?)?;
    let spec = metadata.frame(0).ok_or(Error::InvalidMetadata)?;
    let mut decoder: Decoder = (&metadata).try_into()?;
    for mut buffer in [vec![1; 3], vec![1; 5]] {
        assert_eq!(
            decoder.decode_frame(spec.clone(), &mut buffer),
            Err(Error::InvalidStoredLength)
        );
        assert!(buffer.is_empty());
    }
    let other = Metadata::new(Config::new(
        Framing::Fixed(4),
        Compression::None,
        Encryption::None,
    )?);
    let mut decoder: Decoder = other.try_into()?;
    let mut buffer = vec![1; 4];
    assert_eq!(
        decoder.decode_frame(spec, &mut buffer),
        Err(Error::ProfileMismatch)
    );
    assert!(buffer.is_empty());
    Ok(())
}

#[cfg(feature = "aes-gcm")]
#[test]
fn aes256_nist_known_answer_with_suffix_tag() -> TestResult {
    // NIST AES-256-GCM: zero key, zero 96-bit IV, 16 zero plaintext bytes, no AAD.
    let expected = [
        0xce, 0xa7, 0x40, 0x3d, 0x4d, 0x60, 0x6b, 0x6e, 0x07, 0x4e, 0xc5, 0xd3, 0xba, 0xf3, 0x9d,
        0x18, 0xd0, 0xd1, 0xc8, 0xa7, 0x99, 0x99, 0x6b, 0xf0, 0x26, 0x5b, 0x98, 0xb5, 0xd4, 0x8a,
        0xb9, 0x19,
    ];
    let (mut encoder, mut metadata) = object(
        Framing::Fixed(16),
        Compression::None,
        Encryption::Aes256Gcm { key: [0; 32] },
    )?;
    let mut buffer = vec![0; 16];
    let sizes = encoder.encode_frame(0, &mut buffer)?;
    assert_eq!(buffer, expected);
    metadata.push(sizes)?;
    let mut decoder: Decoder = (&metadata).try_into()?;
    decoder.decode_frame(
        metadata.frame(0).ok_or(Error::InvalidMetadata)?,
        &mut buffer,
    )?;
    assert_eq!(buffer, [0; 16]);
    Ok(())
}

#[cfg(feature = "aes-gcm")]
#[test]
fn frame_nonce_is_big_endian_and_does_not_reset_at_range_start() -> TestResult {
    use aes_gcm::{KeyInit, aead::AeadInOut};
    let key = [71; 32];
    let (mut encoder, _) = object(
        Framing::Fixed(16),
        Compression::None,
        Encryption::Aes256Gcm { key },
    )?;
    let mut actual = vec![9; 16];
    encoder.encode_frame(0x01020304, &mut actual)?;
    let reference = aes_gcm::Aes256Gcm::new((&key).into());
    let nonce = [0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4];
    let mut expected = vec![9; 16];
    let tag = reference
        .encrypt_inout_detached(&nonce.into(), b"", expected.as_mut_slice().into())
        .map_err(|_| Error::EncryptionFailed)?;
    expected.extend_from_slice(&tag);
    assert_eq!(actual, expected);
    Ok(())
}

#[cfg(feature = "aes-gcm")]
#[test]
fn ciphertext_tag_key_and_index_substitution_all_fail_closed() -> TestResult {
    let (mut encoder, mut metadata) = object(
        Framing::Fixed(16),
        Compression::None,
        Encryption::Aes256Gcm { key: [1; 32] },
    )?;
    let mut good = vec![5; 16];
    let sizes = encoder.encode_frame(0, &mut good)?;
    metadata.push(sizes)?;
    metadata.push(sizes)?;
    let mut decoder: Decoder = (&metadata).try_into()?;
    for byte in 0..good.len() {
        let mut corrupted = good.clone();
        *corrupted.get_mut(byte).ok_or(Error::InvalidStoredLength)? ^= 1;
        assert_eq!(
            decoder.decode_frame(
                metadata.frame(0).ok_or(Error::InvalidMetadata)?,
                &mut corrupted
            ),
            Err(Error::AuthenticationFailed)
        );
        assert!(corrupted.is_empty());
    }
    let mut duplicate = good.clone();
    assert_eq!(
        decoder.decode_frame(
            metadata.frame(1).ok_or(Error::InvalidMetadata)?,
            &mut duplicate
        ),
        Err(Error::AuthenticationFailed)
    );
    assert!(duplicate.is_empty());
    let mut parts = metadata.into_parts();
    parts.config = Config::new(
        Framing::Fixed(16),
        Compression::None,
        Encryption::Aes256Gcm { key: [2; 32] },
    )?;
    let wrong_metadata = Metadata::from_parts(parts)?;
    let mut wrong_key: Decoder = (&wrong_metadata).try_into()?;
    assert_eq!(
        wrong_key.decode_frame(
            wrong_metadata.frame(0).ok_or(Error::InvalidMetadata)?,
            &mut good
        ),
        Err(Error::AuthenticationFailed)
    );
    assert!(good.is_empty());
    Ok(())
}

#[cfg(feature = "aes-gcm")]
#[test]
fn aes_slot_and_frame_limits_are_checked_before_encryption() -> TestResult {
    use craft_codec::{AES_MAX_BYTES, AES_MAX_FRAME_LEN};
    let (mut encoder, metadata) = object(
        Framing::Variable(65_536),
        Compression::None,
        Encryption::Aes256Gcm { key: [5; 32] },
    )?;
    let limit = metadata.config().max_frames();
    assert_eq!(limit, AES_MAX_BYTES / 65_536);
    let mut buffer = vec![1];
    assert_eq!(
        encoder.encode_frame(limit, &mut buffer),
        Err(Error::UsageLimit)
    );
    assert!(buffer.is_empty());
    assert!(
        object(
            Framing::Fixed(AES_MAX_FRAME_LEN.checked_add(1).ok_or(Error::Overflow)?),
            Compression::None,
            Encryption::Aes256Gcm { key: [5; 32] }
        )
        .is_err()
    );
    let mut parts = metadata.into_parts();
    parts.frame_count = limit.checked_add(1).ok_or(Error::Overflow)?;
    assert_eq!(Metadata::from_parts(parts), Err(Error::UsageLimit));
    Ok(())
}

#[cfg(feature = "lz4")]
#[test]
fn malformed_compression_wrong_lengths_and_trailing_data_fail() -> TestResult {
    let (mut encoder, metadata) =
        object(Framing::Variable(256), Compression::Lz4, Encryption::None)?;
    let mut encoded = vec![4; 128];
    let sizes = encoder.encode_frame(0, &mut encoded)?;
    assert!(sizes.payload_len() < 127);
    for raw_len in [127, 129] {
        let mut metadata = Metadata::new(metadata.config().clone());
        metadata.push(FrameSizes::new(raw_len, sizes.payload_len())?)?;
        let mut decoder: Decoder = (&metadata).try_into()?;
        let mut buffer = encoded.clone();
        assert_eq!(
            decoder.decode_frame(
                metadata.frame(0).ok_or(Error::InvalidMetadata)?,
                &mut buffer
            ),
            Err(Error::DecompressionFailed)
        );
        assert!(buffer.is_empty());
    }
    for mut buffer in [
        vec![0xff; encoded.len()],
        [encoded.as_slice(), &[0xff]].concat(),
    ] {
        let mut metadata = Metadata::new(metadata.config().clone());
        metadata.push(FrameSizes::new(128, buffer.len() as u64)?)?;
        let mut decoder: Decoder = (&metadata).try_into()?;
        assert_eq!(
            decoder.decode_frame(
                metadata.frame(0).ok_or(Error::InvalidMetadata)?,
                &mut buffer
            ),
            Err(Error::DecompressionFailed)
        );
        assert!(buffer.is_empty());
    }
    Ok(())
}

#[cfg(all(feature = "aes-gcm", feature = "lz4"))]
#[test]
fn authentication_precedes_decompression() -> TestResult {
    let (mut encoder, mut metadata) = object(
        Framing::Fixed(128),
        Compression::Lz4,
        Encryption::Aes256Gcm { key: [8; 32] },
    )?;
    let mut buffer = vec![4; 128];
    let sizes = encoder.encode_frame(0, &mut buffer)?;
    metadata.push(sizes)?;
    *buffer.first_mut().ok_or(Error::InvalidStoredLength)? ^= 0xff;
    let mut decoder: Decoder = (&metadata).try_into()?;
    assert_eq!(
        decoder.decode_frame(
            metadata.frame(0).ok_or(Error::InvalidMetadata)?,
            &mut buffer
        ),
        Err(Error::AuthenticationFailed)
    );
    assert!(buffer.is_empty());
    Ok(())
}
