# CRAFT Codec

Cryptographic Random-Access Framing Toolkit.

With CRAFT, a Rust library, data streams can be compressed and encrypted as they
are written. They can then be read from any byte position without earlier data
being read or decoded.

- Data is written sequentially.
- A read starts at a chosen position and continues sequentially.
- A new read can be started to access another position.

Compression and encryption are optional. 

## How it works

Data is split into independent blocks called **frames**. Each frame can be
compressed, encrypted, and read on its own.

Separate metadata records where the frames are stored. To read part of the
original data, only the frames containing that part are retrieved and decoded.

For example, with 64 KiB frames, reading 4 KiB from the middle of a file requires
decoding only one or two frames. Everything before them can be skipped.

The metadata must be saved for later reads. It contains the frame index, codec
parameters and encryption key, so it must be kept in trusted, confidential storage.

## Installation

In `Cargo.toml`:

```toml
[dependencies]
craft-codec = { version = "0.2", features = ["lz4", "serde"] }
```

Requires Rust 1.85 or newer. AES-256-GCM encryption is enabled by default;
the `lz4` feature adds LZ4 compression. The optional `serde` feature implements
`Serialize` and `Deserialize` for `Metadata`.

## Basic usage

`Metadata` contains the complete configuration for an object. `Encoder` and
`Decoder` are constructed from it with `TryInto`, without separate codec or key
arguments:

```rust
use craft_codec::{Compression, Config, Decoder, Encoder, Encryption, Framing, Metadata};

let mut metadata = Metadata::new(Config::new(
    Framing::Fixed(8),
    Compression::None,
    Encryption::None,
)?);
let mut encoder: Encoder = (&metadata).try_into()?;
let mut buffer = b"abcdefgh".to_vec();
let sizes = encoder.encode_frame(0, &mut buffer)?;
// Write the complete encoded frame successfully before recording its sizes.
metadata.push(sizes)?;

let (_stored_range, frames) = metadata.range(3, Some(7))?;
let mut decoder: Decoder = (&metadata).try_into()?;
for frame in frames {
    decoder.decode_frame(frame.spec, &mut buffer)?;
    let selected = buffer.get(frame.selected).ok_or(craft_codec::Error::InvalidRange)?;
    assert_eq!(selected, b"defg");
}
# Ok::<(), craft_codec::Error>(())
```

`Compression::Lz4` enables compression. `Encryption::Aes256Gcm { key }` stores
all encryption initialization data in the metadata. Deserializing `Metadata`
from a database restores everything needed by `TryInto`. Codecs own their
initialized state and retain no borrow of the metadata.

Read positions refer to the original data, before compression or encryption.
For example, this selects bytes 3 through 18:

```ignore
let (stored_range, frames) = metadata.range(3, Some(19))?;
```

The result identifies the stored range to read and the frames to decode.
`metadata.range(start, None)` selects everything from `start` to the end.

The [complete example](examples/memory.rs) writes data to memory and reads a
selected range with compression and encryption disabled.

```sh
cargo run --example memory --no-default-features
```

The [examples guide](examples/README.md) also covers integration with CARBON I/O
for concurrent reads and writes.

## Frame size and compression

Frames can have a fixed size or vary up to a chosen maximum.
Smaller frames reduce the extra data decoded for a small read. Larger frames
reduce the space used by metadata and encryption tags.

Compression happens before encryption. If compression does not make a frame
smaller, the original bytes are kept and encrypted if encryption is enabled.

## Encryption and data safety

Each file or blob must have a fresh independent secret key. Once encrypted,
its contents must remain unchanged. Different data must never be encrypted
with the same key and frame index. Retries may resend the same encrypted bytes;
changed data requires a new key.

Metadata includes the encryption key and must be kept in trusted, confidential
storage, linked to the correct file or blob. Encryption detects changes to the
frames being read,
but does not protect metadata or detect missing final frames on its own.
Expected lengths and frame counts must also be checked by the application.

Without encryption, CRAFT does not detect data tampering. Data lengths and
compression ratios remain visible even with encryption. Sensitive data is not
securely erased from memory by CRAFT.

The format has not undergone a security audit. Encryption limits and detailed
requirements are covered in the [format specification](docs/specification.md).

## Metadata schema

Metadata carries a mandatory `schema_version`, currently `2`, separate from the
frame format version `1`. Only schema `2`, introduced in craft-codec `0.2.0`, is
supported. Metadata from releases before `0.2.0`, missing schema versions and
unknown schema versions are rejected. No legacy decoder or migration is provided.

The metadata enums can be deserialized even when a codec feature is disabled.
Conversion to `Encoder` or `Decoder` then returns `CompressionUnavailable` or
`EncryptionUnavailable` if that codec is required.

## More information

- [Examples and I/O integration](examples/README.md)
- [Format, metadata storage, and API details](docs/specification.md)
- [Benchmarks](docs/benchmarks.md)

MIT licensed.
