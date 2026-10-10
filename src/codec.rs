use crate::{Config, Error, FrameSizes, FrameSpec, Metadata, Result};
use crate::{compression::CompressionState, crypto::EncryptionState};

/// Synchronous frame encoder constructed entirely from metadata with reusable codec storage.
///
/// The caller owns the frame index and must never encrypt different payloads
/// with the same key/index. Use a fresh key for each immutable object. This
/// type performs no I/O, scheduling, implicit indexing, or retries.
pub struct Encoder {
    config: Config,
    compression: CompressionState,
    encryption: EncryptionState,
}

impl TryFrom<&Metadata> for Encoder {
    type Error = Error;

    fn try_from(metadata: &Metadata) -> Result<Self> {
        let config = metadata.config().clone();
        Ok(Self {
            compression: CompressionState::new(config.compression())?,
            encryption: EncryptionState::new(config.encryption())?,
            config,
        })
    }
}

impl TryFrom<Metadata> for Encoder {
    type Error = Error;

    fn try_from(metadata: Metadata) -> Result<Self> {
        Self::try_from(&metadata)
    }
}

impl Encoder {
    /// Replace raw bytes with stored bytes, returning lengths to append to metadata.
    /// Empty/oversized frames and out-of-policy indices are refused.
    /// On any error the buffer is cleared, but its allocation is retained.
    ///
    /// This does not append metadata: the caller does that after successful I/O.
    /// The metadata builder enforces that a short fixed frame is the last frame.
    pub fn encode_frame(&mut self, index: u64, buffer: &mut Vec<u8>) -> Result<FrameSizes> {
        let result = self.encode_inner(index, buffer);
        if result.is_err() {
            buffer.clear();
        }
        result
    }

    /// Encode raw bytes into an owned buffer, leaving the input unchanged.
    /// Empty/oversized frames and out-of-policy indices are refused.
    /// Allocation and encoding failures are returned as errors.
    pub fn encode_frame_alloc(&mut self, index: u64, buffer: &[u8]) -> Result<Vec<u8>> {
        let capacity = buffer
            .len()
            .checked_add(self.encryption.overhead())
            .ok_or(Error::Overflow)?;
        let mut encoded = Vec::new();
        encoded
            .try_reserve(capacity)
            .map_err(|_| Error::Allocation)?;
        encoded.extend_from_slice(buffer);
        self.encode_frame(index, &mut encoded)?;
        Ok(encoded)
    }

    fn encode_inner(&mut self, index: u64, buffer: &mut Vec<u8>) -> Result<FrameSizes> {
        let raw = buffer.len() as u64;
        self.config.validate_frame(index, raw, raw)?;
        self.compression.encode(buffer)?;
        let payload = buffer.len() as u64;
        self.encryption.seal(index, buffer)?;
        Ok(FrameSizes { raw, payload })
    }
}

/// Synchronous frame decoder. It knows no logical ranges or I/O offsets.
/// Authentication precedes decompression. Reuse one instance and the caller's
/// buffer to amortize allocations. Compression may swap the buffer allocation.
pub struct Decoder {
    config: Config,
    compression: CompressionState,
    encryption: EncryptionState,
}

impl TryFrom<&Metadata> for Decoder {
    type Error = Error;

    fn try_from(metadata: &Metadata) -> Result<Self> {
        let config = metadata.config().clone();
        Ok(Self {
            compression: CompressionState::new(config.compression())?,
            encryption: EncryptionState::new(config.encryption())?,
            config,
        })
    }
}

impl TryFrom<Metadata> for Decoder {
    type Error = Error;

    fn try_from(metadata: Metadata) -> Result<Self> {
        Self::try_from(&metadata)
    }
}

impl Decoder {
    /// Replace one complete stored frame with its complete decoded bytes.
    /// On failure the buffer is cleared. No unauthenticated bytes are returned.
    /// Clearing the buffer is not a promise to zero its allocation or scratch.
    pub fn decode_frame(&mut self, spec: FrameSpec, buffer: &mut Vec<u8>) -> Result<()> {
        let result = self.decode_inner(spec, buffer);
        if result.is_err() {
            buffer.clear();
        }
        result
    }

    fn decode_inner(&mut self, spec: FrameSpec, buffer: &mut Vec<u8>) -> Result<()> {
        if spec.config != self.config {
            return Err(Error::ProfileMismatch);
        }
        if buffer.len() != spec.stored_len() {
            return Err(Error::InvalidStoredLength);
        }
        self.encryption.open(spec.index(), buffer)?;
        if spec.payload_len() < spec.raw_len() {
            self.compression.decode(buffer, spec.raw_len())?;
        }
        if buffer.len() != spec.raw_len() {
            return Err(Error::DecompressionFailed);
        }
        Ok(())
    }
}
