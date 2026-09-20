use crate::{
    CompressionCodec, Config, EncryptionCodec, Error, FrameSizes, FrameSpec, Framing, Result,
};

/// Zero-sized identity transform, usable for compression, encryption, or both.
#[derive(Clone, Copy, Debug, Default)]
pub struct Identity;
impl crate::sealed::Sealed for Identity {}

/// Synchronous, statically dispatched frame encoder with reusable codec storage.
///
/// The caller owns the frame index and must never encrypt different payloads
/// with the same key/index. Use a fresh key for each immutable object. This
/// type performs no I/O, scheduling, implicit indexing, or retries.
pub struct Encoder<C = Identity, E = Identity> {
    config: Config,
    compression: C,
    encryption: E,
}

impl<C: CompressionCodec, E: EncryptionCodec> Encoder<C, E> {
    /// Create an encoder. Codec types determine the metadata profile.
    pub fn new(framing: Framing, compression: C, encryption: E) -> Result<Self> {
        let config = Config::new(framing, C::FORMAT, E::FORMAT)?;
        Ok(Self {
            config,
            compression,
            encryption,
        })
    }

    /// Settings to use when constructing the object's metadata.
    pub fn config(&self) -> Config {
        self.config
    }

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
pub struct Decoder<C = Identity, E = Identity> {
    config: Config,
    compression: C,
    encryption: E,
}

impl<C: CompressionCodec, E: EncryptionCodec> Decoder<C, E> {
    /// Create a decoder matching the object's metadata and the supplied key.
    pub fn new(config: Config, compression: C, encryption: E) -> Result<Self> {
        if config.compression() != C::FORMAT || config.encryption() != E::FORMAT {
            return Err(Error::ProfileMismatch);
        }
        Ok(Self {
            config,
            compression,
            encryption,
        })
    }

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
