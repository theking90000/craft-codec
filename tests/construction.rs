//! Metadata is the only codec construction input, with or without Serde.
use craft_codec::{
    Compression, Config, Decoder, Encoder, Encryption, Error, FrameSizes, Framing,
    METADATA_SCHEMA_VERSION, Metadata,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn borrowed_and_owned_conversions_retain_no_metadata_borrow() -> TestResult {
    let (mut encoder, mut decoder, spec) = {
        let mut metadata = Metadata::new(Config::new(
            Framing::Fixed(8),
            Compression::None,
            Encryption::None,
        )?);
        metadata.push(FrameSizes::new(8, 8)?)?;
        let encoder: Encoder = metadata.clone().try_into()?;
        let decoder: Decoder = (&metadata).try_into()?;
        let spec = metadata.frame(0).ok_or(Error::InvalidMetadata)?;
        (encoder, decoder, spec)
    };
    let mut buffer = b"abcdefgh".to_vec();
    encoder.encode_frame(0, &mut buffer)?;
    decoder.decode_frame(spec, &mut buffer)?;
    assert_eq!(buffer, b"abcdefgh");
    Ok(())
}

#[test]
fn metadata_from_previous_or_unknown_schemas_is_explicitly_rejected() -> TestResult {
    let metadata = Metadata::new(Config::new(
        Framing::Fixed(8),
        Compression::None,
        Encryption::None,
    )?);
    assert_eq!(metadata.parts().schema_version, METADATA_SCHEMA_VERSION);
    for version in [0, 1, 3, u16::MAX] {
        let mut parts = metadata.clone().into_parts();
        parts.schema_version = version;
        assert_eq!(
            Metadata::from_parts(parts),
            Err(Error::UnsupportedSchemaVersion)
        );
    }
    Ok(())
}

#[cfg(not(feature = "lz4"))]
#[test]
fn compression_feature_is_required_only_at_conversion() -> TestResult {
    let metadata = Metadata::new(Config::new(
        Framing::Fixed(8),
        Compression::Lz4,
        Encryption::None,
    )?);
    let restored = Metadata::from_parts(metadata.into_parts())?;
    assert!(matches!(
        Encoder::try_from(&restored),
        Err(Error::CompressionUnavailable)
    ));
    assert!(matches!(
        Decoder::try_from(restored),
        Err(Error::CompressionUnavailable)
    ));
    Ok(())
}

#[cfg(not(feature = "aes-gcm"))]
#[test]
fn encryption_feature_is_required_only_at_conversion() -> TestResult {
    let metadata = Metadata::new(Config::new(
        Framing::Fixed(8),
        Compression::None,
        Encryption::Aes256Gcm { key: [17; 32] },
    )?);
    let restored = Metadata::from_parts(metadata.into_parts())?;
    assert!(matches!(
        Encoder::try_from(&restored),
        Err(Error::EncryptionUnavailable)
    ));
    assert!(matches!(
        Decoder::try_from(restored),
        Err(Error::EncryptionUnavailable)
    ));
    Ok(())
}
