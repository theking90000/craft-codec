use crate::{Compression, Error, Result};

// Keep codec state inline so conversion requires no extra allocation.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CompressionState {
    None,
    #[cfg(feature = "lz4")]
    Lz4(Lz4),
}

impl CompressionState {
    pub(crate) fn new(parameters: Compression) -> Result<Self> {
        match parameters {
            Compression::None => Ok(Self::None),
            #[cfg(feature = "lz4")]
            Compression::Lz4 => Ok(Self::Lz4(Lz4::new())),
            #[cfg(not(feature = "lz4"))]
            Compression::Lz4 => Err(Error::CompressionUnavailable),
        }
    }

    pub(crate) fn encode(&mut self, _buffer: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::None => Ok(()),
            #[cfg(feature = "lz4")]
            Self::Lz4(codec) => codec.encode(_buffer),
        }
    }

    pub(crate) fn decode(&mut self, _buffer: &mut Vec<u8>, _raw_len: usize) -> Result<()> {
        match self {
            Self::None => Err(Error::DecompressionFailed),
            #[cfg(feature = "lz4")]
            Self::Lz4(codec) => codec.decode(_buffer, _raw_len),
        }
    }
}

/// Independent LZ4 blocks with reusable scratch and a lazily initialized hash table.
/// Decoder-only instances never allocate a compression table. No size prefix,
/// dictionary, checksum, or LZ4 frame wrapper is stored in the data stream.
#[cfg(feature = "lz4")]
#[derive(Default)]
pub(crate) struct Lz4 {
    scratch: Vec<u8>,
    table: Option<lz4_flex::block::CompressTable>,
}

#[cfg(feature = "lz4")]
impl Lz4 {
    /// Create a codec without allocating scratch buffers.
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

#[cfg(feature = "lz4")]
impl Lz4 {
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
