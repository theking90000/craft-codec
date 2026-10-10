#![doc = include_str!("../README.md")]

mod codec;
mod compression;
mod config;
mod crypto;
mod error;
mod index;
mod metadata;

pub use codec::{Decoder, Encoder};
pub use config::{
    AES_MAX_BYTES, AES_MAX_FRAME_LEN, AES_MAX_FRAMES, Compression, Config, Encryption,
    FORMAT_VERSION, Framing, METADATA_SCHEMA_VERSION, TAG_LEN,
};
pub use error::{Error, Result};
pub use index::Lengths;
pub use metadata::{FrameIter, FrameRead, FrameSizes, FrameSpec, Metadata, MetadataParts};
