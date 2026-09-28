use crate::{Error, Result};

/// Compact positive lengths, stored as `length - 1`.
///
/// The width is selected from the configured maximum, not the object length.
/// Public variants allow external persistence without enabling `serde`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Lengths {
    /// Lengths up to and including 65,536 bytes.
    U16(Vec<u16>),
    /// Lengths up to and including 4,294,967,296 bytes.
    U32(Vec<u32>),
    /// Larger lengths. An encoded `u64::MAX` is invalid.
    U64(Vec<u64>),
}

impl Lengths {
    fn empty(max_length: u64) -> Self {
        match max_length {
            ..=65_536 => Self::U16(Vec::new()),
            65_537..=4_294_967_296 => Self::U32(Vec::new()),
            _ => Self::U64(Vec::new()),
        }
    }

    /// Create an empty table with the width required by `max_length`.
    pub fn new(max_length: u64) -> Result<Self> {
        if max_length == 0 {
            return Err(Error::InvalidFrameSize);
        }
        Ok(Self::empty(max_length))
    }

    pub(crate) fn for_config(config: crate::Config) -> Self {
        Self::empty(config.max_frame_len())
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        match self {
            Self::U16(v) => v.len(),
            Self::U32(v) => v.len(),
            Self::U64(v) => v.len(),
        }
    }
    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Bytes per entry, excluding the vector's spare capacity.
    pub fn width(&self) -> usize {
        match self {
            Self::U16(_) => 2,
            Self::U32(_) => 4,
            Self::U64(_) => 8,
        }
    }
    /// Decode an entry. Returns `None` for an absent or overflowing entry.
    pub fn get(&self, index: usize) -> Option<u64> {
        match self {
            Self::U16(v) => v.get(index).map(|v| u64::from(*v) + 1),
            Self::U32(v) => v.get(index).map(|v| u64::from(*v) + 1),
            Self::U64(v) => v.get(index).and_then(|v| v.checked_add(1)),
        }
    }
    /// Append a positive length that fits this table's width.
    pub fn push(&mut self, length: u64) -> Result<()> {
        let encoded = length.checked_sub(1).ok_or(Error::InvalidFrameSize)?;
        match self {
            Self::U16(v) => {
                let value = u16::try_from(encoded).map_err(|_| Error::Overflow)?;
                v.try_reserve(1).map_err(|_| Error::Allocation)?;
                v.push(value);
            }
            Self::U32(v) => {
                let value = u32::try_from(encoded).map_err(|_| Error::Overflow)?;
                v.try_reserve(1).map_err(|_| Error::Allocation)?;
                v.push(value);
            }
            Self::U64(v) => {
                v.try_reserve(1).map_err(|_| Error::Allocation)?;
                v.push(encoded);
            }
        }
        Ok(())
    }
    pub(crate) fn reserve_one(&mut self) -> Result<()> {
        match self {
            Self::U16(v) => v.try_reserve(1),
            Self::U32(v) => v.try_reserve(1),
            Self::U64(v) => v.try_reserve(1),
        }
        .map_err(|_| Error::Allocation)
    }
}
