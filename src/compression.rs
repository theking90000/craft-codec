use crate::{Compression, Identity, Result};

/// Sealed compression implementation used for static dispatch.
/// Applications normally pass [`Identity`] or, with feature `lz4`, `Lz4`.
pub trait CompressionCodec: crate::sealed::Sealed {
    /// Metadata format implemented by this codec.
    const FORMAT: Compression;
    /// Compress one block, retaining the original if compression is not smaller.
    #[doc(hidden)]
    fn encode(&mut self, buffer: &mut Vec<u8>) -> Result<()>;
    /// Decode a compressed block to exactly `raw_len` bytes.
    #[doc(hidden)]
    fn decode(&mut self, buffer: &mut Vec<u8>, raw_len: usize) -> Result<()>;
}

impl CompressionCodec for Identity {
    const FORMAT: Compression = Compression::None;
    fn encode(&mut self, _: &mut Vec<u8>) -> Result<()> {
        Ok(())
    }
    fn decode(&mut self, _: &mut Vec<u8>, _: usize) -> Result<()> {
        Err(crate::Error::DecompressionFailed)
    }
}

/// Independent LZ4 blocks with reusable scratch and a lazily initialized hash table.
/// Decoder-only instances never allocate a compression table. No size prefix,
/// dictionary, checksum, or LZ4 frame wrapper is stored in the data stream.
#[cfg(feature = "lz4")]
#[derive(Default)]
pub struct Lz4 {
    scratch: Vec<u8>,
    table: Option<lz4_flex::block::CompressTable>,
}

#[cfg(feature = "lz4")]
impl Lz4 {
    /// Create a codec without allocating scratch buffers.
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(feature = "lz4")]
impl crate::sealed::Sealed for Lz4 {}

#[cfg(feature = "lz4")]
impl CompressionCodec for Lz4 {
    const FORMAT: Compression = Compression::Lz4;

    fn encode(&mut self, buffer: &mut Vec<u8>) -> Result<()> {
        use lz4_flex::block::{CompressTable, compress_into_with_table, get_maximum_output_size};
        // The profile limits raw lengths so the codec's bound cannot overflow.
        crate::error::resize(&mut self.scratch, get_maximum_output_size(buffer.len()))?;
        let table = self.table.get_or_insert_with(CompressTable::large);
        let written = compress_into_with_table(buffer, &mut self.scratch, table)
            .map_err(|_| crate::Error::CompressionFailed)?;
        if written < buffer.len() {
            self.scratch.truncate(written);
            std::mem::swap(buffer, &mut self.scratch);
        }
        Ok(())
    }

    fn decode(&mut self, buffer: &mut Vec<u8>, raw_len: usize) -> Result<()> {
        crate::error::resize(&mut self.scratch, raw_len)?;
        // The block API consumes the complete input, not a prefix of a stream.
        let written = lz4_flex::block::decompress_into(buffer, &mut self.scratch)
            .map_err(|_| crate::Error::DecompressionFailed)?;
        if written != raw_len {
            return Err(crate::Error::DecompressionFailed);
        }
        std::mem::swap(buffer, &mut self.scratch);
        Ok(())
    }
}
