use std::{iter::FusedIterator, ops::Range};

use crate::{Compression, Config, Error, FORMAT_VERSION, Framing, Lengths, Result};

/// Lengths returned by an encoder, before the authentication tag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameSizes {
    pub(crate) raw: u64,
    pub(crate) payload: u64,
}

impl FrameSizes {
    /// Validate two positive lengths. Object-specific checks happen on insertion.
    pub fn new(raw: u64, payload: u64) -> Result<Self> {
        if raw == 0 || payload == 0 || payload > raw {
            return Err(Error::InvalidMetadata);
        }
        Ok(Self { raw, payload })
    }
    /// Original length, in bytes.
    pub const fn raw_len(self) -> u64 {
        self.raw
    }
    /// Stored payload length excluding the tag.
    pub const fn payload_len(self) -> u64 {
        self.payload
    }
}

/// Codec input derived from validated metadata, independent of logical slicing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameSpec {
    pub(crate) config: Config,
    pub(crate) index: u64,
    pub(crate) sizes: FrameSizes,
}

impl FrameSpec {
    /// Frame index in the original object, also used to derive its nonce.
    pub const fn index(self) -> u64 {
        self.index
    }
    /// Complete raw buffer length.
    pub fn raw_len(self) -> usize {
        self.sizes.raw as usize
    }
    /// Complete stored length, including the tag.
    pub fn stored_len(self) -> usize {
        self.sizes.payload as usize + self.config.encryption().tag_len()
    }
    /// Payload size excluding the tag.
    pub fn payload_len(self) -> usize {
        self.sizes.payload as usize
    }
}

/// One complete frame to decode, followed by a slice selected by the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameRead {
    /// Descriptor to pass to the decoder.
    pub spec: FrameSpec,
    /// Half-open range inside the fully decoded frame.
    pub selected: Range<usize>,
}

/// External metadata representation. It contains no key and is not self-authenticating.
/// Persist it in trusted storage and restore with [`Metadata::from_parts`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MetadataParts {
    /// Format identifier; currently [`FORMAT_VERSION`].
    pub version: u8,
    /// Object profile.
    pub config: Config,
    /// Exact number of nonempty frames.
    pub frame_count: u64,
    /// Only for a nonempty fixed-framing object. Full final frames are allowed.
    pub last_frame_len: Option<u64>,
    /// Required only for variable logical framing; includes the last frame.
    pub raw_lengths: Option<Lengths>,
    /// Required only with compression; excludes tags, includes raw fallbacks.
    pub payload_lengths: Option<Lengths>,
}

/// Validated compact frame metadata, with cached scalar totals only.
/// Append sizes after the corresponding frame has been written successfully.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Metadata {
    parts: MetadataParts,
    logical_len: u64,
    stored_len: u64,
}

#[cfg(feature = "serde")]
impl serde::Serialize for Metadata {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(&self.parts, serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Metadata {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let parts = <MetadataParts as serde::Deserialize>::deserialize(deserializer)?;
        Self::from_parts(parts).map_err(serde::de::Error::custom)
    }
}

impl Metadata {
    /// Start an empty object. This allocates no index entries.
    pub fn new(config: Config) -> Self {
        let table = || Lengths::new(config.max_frame_len()).expect("validated maximum");
        Self {
            parts: MetadataParts {
                version: FORMAT_VERSION,
                config,
                frame_count: 0,
                last_frame_len: None,
                raw_lengths: matches!(config.framing(), Framing::Variable(_)).then(table),
                payload_lengths: (config.compression() != Compression::None).then(table),
            },
            logical_len: 0,
            stored_len: 0,
        }
    }

    /// Restore and validate trusted external metadata without copying its arrays.
    /// Fixed uncompressed metadata is checked in O(1); indexed modes in O(N).
    pub fn from_parts(parts: MetadataParts) -> Result<Self> {
        if parts.version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion);
        }
        let config = parts.config;
        let count = parts.frame_count;
        if count > config.max_frames() {
            return Err(Error::UsageLimit);
        }
        let width = Lengths::new(config.max_frame_len())?.width();
        let check_table = |table: &Option<Lengths>, required: bool| -> Result<()> {
            match (table, required) {
                (None, false) => Ok(()),
                (Some(table), true) if table.len() as u64 == count && table.width() == width => {
                    Ok(())
                }
                _ => Err(Error::InvalidMetadata),
            }
        };
        check_table(
            &parts.raw_lengths,
            matches!(config.framing(), Framing::Variable(_)),
        )?;
        check_table(
            &parts.payload_lengths,
            config.compression() != Compression::None,
        )?;
        match (config.framing(), count, parts.last_frame_len) {
            (Framing::Fixed(max), 1.., Some(last)) if last > 0 && last <= max => {}
            (Framing::Fixed(_), 0, None) | (Framing::Variable(_), _, None) => {}
            _ => return Err(Error::InvalidMetadata),
        }
        let mut result = Self {
            parts,
            logical_len: 0,
            stored_len: 0,
        };
        if let Framing::Fixed(size) = config.framing() {
            if config.compression() == Compression::None {
                let logical = if count == 0 {
                    0
                } else {
                    (count - 1)
                        .checked_mul(size)
                        .and_then(|n| n.checked_add(result.parts.last_frame_len.unwrap()))
                        .ok_or(Error::Overflow)?
                };
                result.logical_len = logical;
                result.stored_len = count
                    .checked_mul(config.encryption().tag_len() as u64)
                    .and_then(|tags| logical.checked_add(tags))
                    .ok_or(Error::Overflow)?;
                return Ok(result);
            }
        }
        for i in 0..count {
            let raw = match config.framing() {
                Framing::Fixed(size) if i + 1 < count => size,
                Framing::Fixed(_) => result.parts.last_frame_len.unwrap(),
                Framing::Variable(_) => result
                    .parts
                    .raw_lengths
                    .as_ref()
                    .unwrap()
                    .get(i as usize)
                    .ok_or(Error::InvalidMetadata)?,
            };
            let payload = match &result.parts.payload_lengths {
                Some(table) => table.get(i as usize).ok_or(Error::InvalidMetadata)?,
                None => raw,
            };
            config.validate_frame(i, raw, payload)?;
            result.logical_len = result.logical_len.checked_add(raw).ok_or(Error::Overflow)?;
            result.stored_len = result
                .stored_len
                .checked_add(payload)
                .and_then(|n| n.checked_add(config.encryption().tag_len() as u64))
                .ok_or(Error::Overflow)?;
        }
        Ok(result)
    }

    /// Borrow the validated persistence representation.
    pub fn parts(&self) -> &MetadataParts {
        &self.parts
    }
    /// Transfer the persistence representation without copying.
    pub fn into_parts(self) -> MetadataParts {
        self.parts
    }
    /// Object-wide configuration.
    pub fn config(&self) -> Config {
        self.parts.config
    }
    /// Exact number of frames.
    pub fn frame_count(&self) -> u64 {
        self.parts.frame_count
    }
    /// Logical object length, in bytes.
    pub fn logical_len(&self) -> u64 {
        self.logical_len
    }
    /// Stored object length including tags, in bytes.
    pub fn stored_len(&self) -> u64 {
        self.stored_len
    }
    /// Last raw frame length; absent for an empty object.
    pub fn last_frame_len(&self) -> Option<u64> {
        self.frame_count().checked_sub(1).map(|i| self.sizes(i).raw)
    }

    /// Append one successfully stored frame. Failures leave metadata unchanged.
    /// Once a short fixed frame is appended, no further frames may follow it.
    pub fn push(&mut self, sizes: FrameSizes) -> Result<()> {
        let config = self.config();
        config.validate_frame(self.frame_count(), sizes.raw, sizes.payload)?;
        if let Framing::Fixed(size) = config.framing() {
            if self.parts.last_frame_len.is_some_and(|last| last != size) {
                return Err(Error::FrameAfterFinal);
            }
        }
        let count = self.frame_count().checked_add(1).ok_or(Error::Overflow)?;
        let logical = self
            .logical_len
            .checked_add(sizes.raw)
            .ok_or(Error::Overflow)?;
        let stored = self
            .stored_len
            .checked_add(sizes.payload)
            .and_then(|n| n.checked_add(config.encryption().tag_len() as u64))
            .ok_or(Error::Overflow)?;
        // Reserve both arrays before changing either logical contents.
        if let Some(table) = &mut self.parts.raw_lengths {
            table.reserve_one()?;
        }
        if let Some(table) = &mut self.parts.payload_lengths {
            table.reserve_one()?;
        }
        if let Some(table) = &mut self.parts.raw_lengths {
            table.push(sizes.raw)?;
        }
        if let Some(table) = &mut self.parts.payload_lengths {
            table.push(sizes.payload)?;
        }
        if matches!(config.framing(), Framing::Fixed(_)) {
            self.parts.last_frame_len = Some(sizes.raw);
        }
        self.parts.frame_count = count;
        self.logical_len = logical;
        self.stored_len = stored;
        Ok(())
    }

    /// Look up a codec descriptor by its original frame index in O(1).
    pub fn frame(&self, index: u64) -> Option<FrameSpec> {
        (index < self.frame_count()).then(|| FrameSpec {
            config: self.config(),
            index,
            sizes: self.sizes(index),
        })
    }

    fn sizes(&self, index: u64) -> FrameSizes {
        let raw = match self.config().framing() {
            Framing::Fixed(size) if index + 1 < self.frame_count() => size,
            Framing::Fixed(_) => self.parts.last_frame_len.unwrap(),
            Framing::Variable(_) => self
                .parts
                .raw_lengths
                .as_ref()
                .unwrap()
                .get(index as usize)
                .unwrap(),
        };
        let payload = self
            .parts
            .payload_lengths
            .as_ref()
            .map_or(raw, |v| v.get(index as usize).unwrap());
        FrameSizes { raw, payload }
    }

    /// Map logical `[start, end)` bytes to a physical range of complete frames.
    /// `None` means logical EOF. Empty ranges return `0..0` and an empty iterator.
    /// No allocation, I/O, or decoding occurs. Variable-size layouts scan their
    /// compact index to the last selected frame; fixed uncompressed layouts use arithmetic.
    pub fn range(&self, start: u64, end: Option<u64>) -> Result<(Range<u64>, FrameIter<'_>)> {
        let end = end.unwrap_or(self.logical_len);
        if start > end || end > self.logical_len {
            return Err(Error::InvalidRange);
        }
        let iter = |first, stop, logical| FrameIter {
            metadata: self,
            next: first,
            stop,
            logical,
            selection: start..end,
        };
        if start == end {
            return Ok((0..0, iter(0, 0, 0)));
        }
        if let Framing::Fixed(size) = self.config().framing() {
            let first = start / size;
            let stop = (end - 1) / size + 1;
            if self.config().compression() == Compression::None {
                let stride = size + self.config().encryption().tag_len() as u64;
                let physical_start = first * stride;
                let physical_end = if stop == self.frame_count() {
                    self.stored_len
                } else {
                    stop * stride
                };
                return Ok((
                    physical_start..physical_end,
                    iter(first, stop, first * size),
                ));
            }
            let mut physical = 0;
            for i in 0..first {
                physical += self.sizes(i).payload + self.config().encryption().tag_len() as u64;
            }
            let physical_start = physical;
            if stop == self.frame_count() {
                physical = self.stored_len;
            } else {
                for i in first..stop {
                    physical += self.sizes(i).payload + self.config().encryption().tag_len() as u64;
                }
            }
            return Ok((physical_start..physical, iter(first, stop, first * size)));
        }
        let mut logical = 0;
        let mut physical = 0;
        let mut first = 0;
        while logical + self.sizes(first).raw <= start {
            let sizes = self.sizes(first);
            logical += sizes.raw;
            physical += sizes.payload + self.config().encryption().tag_len() as u64;
            first += 1;
        }
        let logical_start = logical;
        let physical_start = physical;
        let mut stop = first;
        if end == self.logical_len {
            stop = self.frame_count();
            physical = self.stored_len;
        } else {
            while logical < end {
                let sizes = self.sizes(stop);
                logical += sizes.raw;
                physical += sizes.payload + self.config().encryption().tag_len() as u64;
                stop += 1;
            }
        }
        Ok((physical_start..physical, iter(first, stop, logical_start)))
    }
}

/// Borrowing, allocation-free iterator over a selected logical range.
#[derive(Clone, Debug)]
pub struct FrameIter<'a> {
    metadata: &'a Metadata,
    next: u64,
    stop: u64,
    logical: u64,
    selection: Range<u64>,
}

impl FrameIter<'_> {
    /// Exact remaining frame count, including partial boundary frames.
    pub fn remaining(&self) -> u64 {
        self.stop - self.next
    }
}

impl Iterator for FrameIter<'_> {
    type Item = FrameRead;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.stop {
            return None;
        }
        let spec = self.metadata.frame(self.next).unwrap();
        let logical_end = self.logical + spec.sizes.raw;
        let selected = (self.selection.start.saturating_sub(self.logical) as usize)
            ..((self.selection.end.min(logical_end) - self.logical) as usize);
        self.logical = logical_end;
        self.next += 1;
        Some(FrameRead { spec, selected })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match usize::try_from(self.remaining()) {
            Ok(n) => (n, Some(n)),
            Err(_) => (usize::MAX, None),
        }
    }
}

impl FusedIterator for FrameIter<'_> {}
