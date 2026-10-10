//! Self-contained binary persistence and explicit schema compatibility.
#![cfg(feature = "serde")]

use craft_codec::{
    Compression, Config, Decoder, Encoder, Encryption, Error, FrameSizes, Framing,
    METADATA_SCHEMA_VERSION, Metadata, MetadataParts,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn persisted_object(
    compression: Compression,
    encryption: Encryption,
    framing: Framing,
) -> TestResult {
    let mut metadata = Metadata::new(Config::new(framing, compression, encryption)?);
    let mut encoder: Encoder = (&metadata).try_into()?;
    let original = [vec![7; 64], vec![9; 19]].concat();
    let mut stored = Vec::new();
    for (index, raw) in original.chunks(64).enumerate() {
        let mut buffer = raw.to_vec();
        let sizes = encoder.encode_frame(index as u64, &mut buffer)?;
        stored.extend_from_slice(&buffer);
        metadata.push(sizes)?;
    }
    let saved = postcard::to_allocvec(&metadata)?;
    let restored: Metadata = postcard::from_bytes(&saved)?;
    assert_eq!(restored, metadata);
    drop(encoder);
    drop(metadata);

    // Range selection and conversion share immutable metadata; the key is restored from DB bytes.
    let (query, frames) = restored.range(3, Some(79))?;
    let mut decoder: Decoder = (&restored).try_into()?;
    let mut offset = usize::try_from(query.start)?;
    let mut selected = Vec::new();
    for frame in frames {
        let end = offset
            .checked_add(frame.spec.stored_len())
            .ok_or(Error::Overflow)?;
        let mut buffer = stored
            .get(offset..end)
            .ok_or(Error::InvalidStoredLength)?
            .to_vec();
        decoder.decode_frame(frame.spec, &mut buffer)?;
        selected.extend_from_slice(buffer.get(frame.selected).ok_or(Error::InvalidRange)?);
        offset = end;
    }
    assert_eq!(offset as u64, query.end);
    assert_eq!(
        selected.as_slice(),
        original.get(3..79).ok_or(Error::InvalidRange)?
    );

    // Both conversions produce owned state that remains usable after dropping metadata.
    let spec = restored.frame(0).ok_or(Error::InvalidMetadata)?;
    let mut rebuilt_encoder: Encoder = (&restored).try_into()?;
    let mut rebuilt_decoder: Decoder = restored.try_into()?;
    let mut buffer = original.get(..64).ok_or(Error::InvalidRange)?.to_vec();
    rebuilt_encoder.encode_frame(0, &mut buffer)?;
    assert_eq!(
        buffer.as_slice(),
        stored
            .get(..spec.stored_len())
            .ok_or(Error::InvalidStoredLength)?
    );
    rebuilt_decoder.decode_frame(spec, &mut buffer)?;
    assert_eq!(
        buffer.as_slice(),
        original.get(..64).ok_or(Error::InvalidRange)?
    );
    Ok(())
}

#[test]
fn database_roundtrip_restores_every_available_codec_combination() -> TestResult {
    for framing in [Framing::Fixed(64), Framing::Variable(64)] {
        persisted_object(Compression::None, Encryption::None, framing)?;
        #[cfg(feature = "lz4")]
        persisted_object(Compression::Lz4, Encryption::None, framing)?;
        #[cfg(feature = "aes-gcm")]
        persisted_object(
            Compression::None,
            Encryption::Aes256Gcm { key: [31; 32] },
            framing,
        )?;
        #[cfg(all(feature = "lz4", feature = "aes-gcm"))]
        persisted_object(
            Compression::Lz4,
            Encryption::Aes256Gcm { key: [47; 32] },
            framing,
        )?;
    }
    Ok(())
}

#[test]
fn schema_is_mandatory_and_only_version_two_is_supported() -> TestResult {
    let metadata = Metadata::new(Config::new(
        Framing::Fixed(8),
        Compression::None,
        Encryption::None,
    )?);
    assert_eq!(metadata.parts().schema_version, METADATA_SCHEMA_VERSION);
    for version in [0, 1, 3, u16::MAX] {
        let mut parts = metadata.clone().into_parts();
        parts.schema_version = version;
        let saved = postcard::to_allocvec(&parts)?;
        let error = postcard::from_bytes::<Metadata>(&saved);
        assert!(error.is_err());
        assert!(postcard::from_bytes::<MetadataParts>(&saved).is_err());
        assert_eq!(
            Metadata::from_parts(parts),
            Err(Error::UnsupportedSchemaVersion)
        );
    }
    assert!(postcard::from_bytes::<Metadata>(&[]).is_err());
    // 0.1.x postcard fixture: format 1, Fixed(8), no codecs, empty frame index.
    let legacy = [1, 0, 8, 0, 0, 0, 0, 0, 0];
    assert!(postcard::from_bytes::<Metadata>(&legacy).is_err());
    Ok(())
}

#[test]
fn serialized_metadata_still_validates_indexes_and_configuration() -> TestResult {
    let mut metadata = Metadata::new(Config::new(
        Framing::Variable(64),
        Compression::None,
        Encryption::None,
    )?);
    metadata.push(FrameSizes::new(19, 19)?)?;
    let mut parts = metadata.into_parts();
    parts.frame_count = 2;
    assert!(postcard::from_bytes::<Metadata>(&postcard::to_allocvec(&parts)?).is_err());
    // Valid schema and format, followed by invalid Fixed(0) configuration.
    let invalid_config = [2, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(postcard::from_bytes::<Metadata>(&invalid_config).is_err());
    Ok(())
}

#[test]
fn disabled_codecs_can_be_deserialized_but_cannot_be_instantiated() -> TestResult {
    for (compression, encryption) in [
        (Compression::Lz4, Encryption::None),
        (Compression::None, Encryption::Aes256Gcm { key: [59; 32] }),
    ] {
        let metadata = Metadata::new(Config::new(Framing::Fixed(64), compression, encryption)?);
        let restored: Metadata = postcard::from_bytes(&postcard::to_allocvec(&metadata)?)?;
        assert_eq!(restored, metadata);
        #[cfg(not(feature = "lz4"))]
        if restored.config().compression() == Compression::Lz4 {
            assert!(matches!(
                Encoder::try_from(&restored),
                Err(Error::CompressionUnavailable)
            ));
            assert!(matches!(
                Decoder::try_from(restored),
                Err(Error::CompressionUnavailable)
            ));
            continue;
        }
        #[cfg(not(feature = "aes-gcm"))]
        if matches!(restored.config().encryption(), Encryption::Aes256Gcm { .. }) {
            assert!(matches!(
                Encoder::try_from(&restored),
                Err(Error::EncryptionUnavailable)
            ));
            assert!(matches!(
                Decoder::try_from(restored),
                Err(Error::EncryptionUnavailable)
            ));
        }
    }
    Ok(())
}
