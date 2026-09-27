use crate::{Error, Result, error::buffer_len};

/// Version of the headerless data format and its external metadata.
pub const FORMAT_VERSION: u8 = 1;
/// AES-GCM authentication tag length in bytes.
pub const TAG_LEN: usize = 16;

/// Upper bound on raw bytes covered by the AES profile's frame slots per key.
/// This is a library usage policy, not a claim of 128-bit security at this limit.
pub const AES_MAX_BYTES: u64 = 1 << 36;
/// Maximum number of AES-GCM frame slots per object/key.
pub const AES_MAX_FRAMES: u64 = 1 << 32;
/// Maximum configured raw frame length for the AES-GCM profile, 16 MiB.
pub const AES_MAX_FRAME_LEN: u64 = 1 << 24;

/// Logical frame boundaries, before compression or encryption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Framing {
    /// All frames have this size, except a possibly shorter final frame.
    Fixed(u64),
    /// Each nonempty frame may have any size up to this limit.
    Variable(u64),
}

impl Framing {
    /// Maximum number of raw bytes in a frame.
    pub const fn max_frame_len(self) -> u64 {
        match self {
            Self::Fixed(size) | Self::Variable(size) => size,
        }
    }
}

/// Compression identifier in trusted external metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum Compression {
    /// Store raw bytes.
    None = 0,
    /// Independent LZ4 blocks, retained only when strictly smaller.
    Lz4 = 1,
}

/// Encryption identifier in trusted external metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum Encryption {
    /// No authentication or encryption.
    None = 0,
    /// AES-256-GCM with a 96-bit frame-index nonce and a 16-byte suffix tag.
    Aes256Gcm = 1,
}

impl Encryption {
    /// Stored suffix length in bytes.
    pub const fn tag_len(self) -> usize {
        match self {
            Self::None => 0,
            Self::Aes256Gcm => TAG_LEN,
        }
    }
}

/// Validated object-wide settings. Contains no key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Config {
    framing: Framing,
    compression: Compression,
    encryption: Encryption,
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Config {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Fields {
            framing: Framing,
            compression: Compression,
            encryption: Encryption,
        }

        let fields = <Fields as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(fields.framing, fields.compression, fields.encryption)
            .map_err(serde::de::Error::custom)
    }
}

impl Config {
    /// Validate a profile, including platform, codec and AES usage limits.
    /// Metadata for disabled codecs may still be inspected with this type.
    pub fn new(framing: Framing, compression: Compression, encryption: Encryption) -> Result<Self> {
        let max = framing.max_frame_len();
        if max == 0 {
            return Err(Error::InvalidFrameSize);
        }
        buffer_len(
            max.checked_add(encryption.tag_len() as u64)
                .ok_or(Error::Overflow)?,
        )?;
        // LZ4 block offsets/tables use 32-bit positions. Leave room for its bound.
        if compression == Compression::Lz4 && max > 0x7e00_0000 {
            return Err(Error::InvalidFrameSize);
        }
        if encryption == Encryption::Aes256Gcm && max > AES_MAX_FRAME_LEN {
            return Err(Error::InvalidFrameSize);
        }
        Ok(Self {
            framing,
            compression,
            encryption,
        })
    }

    /// Logical framing rule.
    pub const fn framing(self) -> Framing {
        self.framing
    }
    /// Compression format.
    pub const fn compression(self) -> Compression {
        self.compression
    }
    /// Encryption format.
    pub const fn encryption(self) -> Encryption {
        self.encryption
    }
    /// Maximum raw length, in bytes.
    pub const fn max_frame_len(self) -> u64 {
        self.framing.max_frame_len()
    }

    /// Maximum frame slots for this profile. AES reserves the configured
    /// maximum length for each slot, even if an actual frame is shorter.
    pub fn max_frames(self) -> u64 {
        match self.encryption {
            Encryption::None => u64::MAX,
            Encryption::Aes256Gcm => AES_MAX_FRAMES.min(AES_MAX_BYTES / self.max_frame_len()),
        }
    }

    pub(crate) fn validate_frame(self, index: u64, raw: u64, payload: u64) -> Result<()> {
        if raw == 0 || raw > self.max_frame_len() {
            return Err(Error::InvalidFrameSize);
        }
        if payload == 0
            || payload > raw
            || (self.compression == Compression::None && payload != raw)
        {
            return Err(Error::InvalidMetadata);
        }
        if index >= self.max_frames() {
            return Err(Error::UsageLimit);
        }
        Ok(())
    }
}
