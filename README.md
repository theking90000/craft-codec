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

The metadata must be saved with the data. Both are needed for later reads.

## Installation

In `Cargo.toml`:

```toml
[dependencies]
craft-codec = { version = "0.1", features = ["lz4", "serde"] }
```

Requires Rust 1.85 or newer. AES-256-GCM encryption is enabled by default;
the `lz4` feature adds LZ4 compression. The optional `serde` feature implements
`Serialize` and `Deserialize` for `Metadata`.

## Basic usage

The API follows the same steps as the data:

1. `Encoder` compresses and encrypts each frame.
2. `Metadata` records each frame after it has been written successfully.
3. `Decoder` restores the frames needed for a read.

Read positions refer to the original data, before compression or encryption.
For example, this selects bytes 3 through 18:

```ignore
let (stored_range, frames) = metadata.range(3, Some(19))?;
```

The result identifies the stored range to read and the frames to decode.
`metadata.range(start, None)` selects everything from `start` to the end.

The [complete example](examples/memory.rs) writes data to memory and reads a
selected range. It uses `Identity` to leave compression and encryption disabled.
`Lz4` and `Aes256Gcm` enable those steps.

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

Metadata and keys must be kept in trusted storage, with the metadata linked to
the correct file or blob. Encryption detects changes to the frames being read,
but does not protect metadata or detect missing final frames on its own.
Expected lengths and frame counts must also be checked by the application.

Without encryption, CRAFT does not detect data tampering. Data lengths and
compression ratios remain visible even with encryption. Sensitive data is not
securely erased from memory by CRAFT.

The format has not undergone a security audit. Encryption limits and detailed
requirements are covered in the [format specification](docs/specification.md).

## More information

- [Examples and I/O integration](examples/README.md)
- [Format, metadata storage, and API details](docs/specification.md)
- [Benchmarks](docs/benchmarks.md)

MIT licensed.
