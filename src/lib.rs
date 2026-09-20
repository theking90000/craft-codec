#![doc = include_str!("../README.md")]

mod codec;
mod compression;
mod config;
mod crypto;
mod error;
mod index;
mod metadata;

pub use codec::{Decoder, Encoder, Identity};
pub use compression::CompressionCodec;
#[cfg(feature = "lz4")]
pub use compression::Lz4;
pub use config::{
    AES_MAX_BYTES, AES_MAX_FRAME_LEN, AES_MAX_FRAMES, Compression, Config, Encryption,
    FORMAT_VERSION, Framing, TAG_LEN,
};
#[cfg(feature = "aes-gcm")]
pub use crypto::Aes256Gcm;
pub use crypto::EncryptionCodec;
pub use error::{Error, Result};
pub use index::Lengths;
pub use metadata::{FrameIter, FrameRead, FrameSizes, FrameSpec, Metadata, MetadataParts};

mod sealed {
    pub trait Sealed {}
}
