use std::fmt;

/// A format, resource, or transformation failure. No I/O is performed by CRAFT.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// A zero or unsupported maximum frame length.
    InvalidFrameSize,
    /// A length, count, profile, or index is inconsistent.
    InvalidMetadata,
    /// This metadata format version is not supported.
    UnsupportedVersion,
    /// The logical range is reversed or outside the object.
    InvalidRange,
    /// A full frame was submitted after a short final frame.
    FrameAfterFinal,
    /// The buffer does not have the expected stored length.
    InvalidStoredLength,
    /// The descriptor and the decoder use different configurations.
    ProfileMismatch,
    /// The configured cryptographic usage limit was reached.
    UsageLimit,
    /// An integer or platform buffer limit would be exceeded.
    Overflow,
    /// A buffer or index allocation failed.
    Allocation,
    /// The compression operation failed.
    CompressionFailed,
    /// Invalid compressed bytes or an incorrect decoded length.
    DecompressionFailed,
    /// Encryption failed.
    EncryptionFailed,
    /// The frame did not authenticate.
    AuthenticationFailed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidFrameSize => "invalid or unsupported frame size",
            Self::InvalidMetadata => "inconsistent frame metadata",
            Self::UnsupportedVersion => "unsupported metadata version",
            Self::InvalidRange => "logical range outside the object or reversed",
            Self::FrameAfterFinal => "frame submitted after a short final frame",
            Self::InvalidStoredLength => "incorrect stored frame length",
            Self::ProfileMismatch => "frame and decoder configurations differ",
            Self::UsageLimit => "cryptographic usage limit exceeded",
            Self::Overflow => "length exceeds integer or platform limits",
            Self::Allocation => "buffer allocation failed",
            Self::CompressionFailed => "compression failed",
            Self::DecompressionFailed => "invalid compressed frame",
            Self::EncryptionFailed => "encryption failed",
            Self::AuthenticationFailed => "frame authentication failed",
        })
    }
}

impl std::error::Error for Error {}

/// Result of a CRAFT operation.
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn buffer_len(len: u64) -> Result<usize> {
    let len = usize::try_from(len).map_err(|_| Error::Overflow)?;
    if len > isize::MAX as usize {
        return Err(Error::Overflow);
    }
    Ok(len)
}

#[cfg(feature = "lz4")]
pub(crate) fn resize(buffer: &mut Vec<u8>, len: usize) -> Result<()> {
    if len > isize::MAX as usize {
        return Err(Error::Overflow);
    }
    if len > buffer.len() {
        buffer
            .try_reserve(len - buffer.len())
            .map_err(|_| Error::Allocation)?;
    }
    buffer.resize(len, 0);
    Ok(())
}
