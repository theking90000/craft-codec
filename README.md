# CRAFT Codec

Cryptographic Random-Access Framing Toolkit.

CRAFT is a Rust library for compressing and encrypting data in independent
blocks called frames. It lets you read a portion of the original data without
reading or decoding the entire file.

Your application splits the data into frames and stores the encoded frames in
order. CRAFT records their sizes in separate metadata. When you request a byte
range, it uses that metadata to find the stored frames you need to read.
Each frame can be decrypted and decompressed independently.

For example, with 64 KiB frames, reading 4 KiB from the middle of a file requires
decoding only the one or two frames that contain those bytes. Earlier frames
do not need to be read or decoded, even when compression and encryption are
enabled.

- `Encoder` compresses and encrypts one frame at a time.
- `Metadata` maps a byte range in the original data to complete stored frames.
- `Decoder` restores each frame so you can extract the requested bytes.

Compression and encryption are optional. The API is synchronous and works on
reusable `Vec<u8>` buffers. Your application handles file or network I/O,
concurrency, retries, and metadata storage.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
craft-codec = { version = "0.1", features = ["lz4"] }
```

Requires Rust 1.85 or newer.

| Feature | Default | Provides |
| --- | --- | --- |
| `aes-gcm` | Enabled | AES-256-GCM authenticated encryption |
| `lz4` | Disabled | LZ4 block compression |

With `default-features = false`, framing and metadata have no runtime
dependencies. Use `Identity` in place of a compressor or cipher to disable that
transformation.

## Encoding data and reading a range

This example stores frames in memory, then reads bytes `3..19` of the original
data. It uses eight-byte frames and disables compression and encryption to show
the write and read steps.

```rust
use craft_codec::{Decoder, Encoder, Framing, Identity, Metadata};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = Encoder::new(Framing::Fixed(8), Identity, Identity)?;
    let mut metadata = Metadata::new(encoder.config());
    let original = b"small independent frames";
    let mut object = Vec::new();
    let mut buffer = Vec::new();

    for (index, raw) in original.chunks(8).enumerate() {
        buffer.clear();
        buffer.extend_from_slice(raw);
        let sizes = encoder.encode_frame(index as u64, &mut buffer)?;
        object.extend_from_slice(&buffer);
        metadata.push(sizes)?;
    }

    // Locate the complete stored frames containing bytes 3..19.
    let (stored_range, frames) = metadata.range(3, Some(19))?;
    let mut source = &object[stored_range.start as usize..stored_range.end as usize];
    let mut decoder = Decoder::new(metadata.config(), Identity, Identity)?;
    let mut result = Vec::new();

    for frame in frames {
        buffer.resize(frame.spec.stored_len(), 0);
        std::io::Read::read_exact(&mut source, &mut buffer)?;
        decoder.decode_frame(frame.spec, &mut buffer)?;
        result.extend_from_slice(&buffer[frame.selected]);
    }

    assert_eq!(result, original[3..19]);
    Ok(())
}
```

For a file or storage service, replace `object.extend_from_slice` with your
write operation. Append the frame's sizes to `metadata` only after the write
succeeds. Save the metadata separately and make it available with the completed
object. Here, an object means the full sequence of stored frames for one file
or blob.

### Original byte ranges and stored byte ranges

A logical byte range refers to the original data, before compression or
encryption. A stored byte range refers to the encoded bytes in storage.
Compression changes frame sizes, and encryption adds an authentication tag to
each frame, so the two ranges can have different offsets and lengths.

`metadata.range(start, end)` returns a stored byte range and an iterator over
the frames to decode. Read each complete frame, pass its `spec` to the decoder,
and then take `frame.selected` from the decoded buffer. The first and last
frames may contain bytes outside the requested range.

End bounds are exclusive. `range(start, None)` selects everything through the
end of the original data. Invalid ranges return an error. An empty range returns
an empty stored range and iterator, so no I/O is needed.

Fixed-size frames without compression allow CRAFT to calculate offsets directly.
For compressed or variable-size frames, it scans the size metadata to locate
the range. This scan does not read or decode the stored data.

## Choosing frame sizes

Your application chooses where to split the input:

| Configuration | Input frames |
| --- | --- |
| `Framing::Fixed(size)` | Every frame has `size` bytes, except the last, which may be shorter |
| `Framing::Variable(max)` | Each frame may have a different size, up to `max` bytes |

Individual frames must be nonempty. An empty object has no frames.
Smaller frames reduce the amount of extra data decoded for a small range read.
Larger frames reduce the number of metadata entries and authentication tags.

## Enabling compression and encryption

Replace `Identity` with `Lz4` and `Aes256Gcm`. The write and range-read steps
remain the same. This setup uses variable-size frames of at most 64 KiB:

```rust
fn main() -> Result<(), craft_codec::Error> {
    # #[cfg(all(feature = "aes-gcm", feature = "lz4"))]
    # {
    use craft_codec::{Aes256Gcm, Decoder, Encoder, Framing, Lz4};

    // Example only. Generate a fresh secret key for each real object.
    let key = [42; 32];
    let encoder = Encoder::new(
        Framing::Variable(65_536),
        Lz4::new(),
        Aes256Gcm::new(&key),
    )?;
    let decoder = Decoder::new(encoder.config(), Lz4::new(), Aes256Gcm::new(&key))?;
    # let _ = decoder;
    # }
    Ok(())
}
```

CRAFT compresses each frame before encrypting it. It keeps the compressed bytes
only when they are smaller than the original frame. Otherwise it stores the
original bytes, encrypted if encryption is enabled.

AES-256-GCM adds a 16-byte authentication tag to each frame. The decoder verifies
that tag before decompressing or returning the frame's contents.

### Keys and data integrity

Use a fresh independent secret key for each immutable object. CRAFT derives each
frame's nonce from its original index in that object. Never encrypt different
payloads with the same key and frame index. Key generation and storage are your
application's responsibility.

A retry may write the same encoded bytes again. If a new attempt can change the
payload, use a new key. CRAFT does not track key or nonce reuse across calls.

Keep the metadata, its association with the object, and the key in trusted
storage. Frame authentication does not authenticate the metadata. Your reader
must detect missing final frames by checking the expected lengths and frame
count. A range read verifies only the frames it decodes. Without encryption,
CRAFT provides no integrity guarantee. The format does not hide data lengths
or compression ratios.

The V1 encryption limits are 16 MiB per configured frame and at most
`min(2^32, 64 GiB / max_frame_len)` frames per key. With 64 KiB frames, that is
1,048,576 frames. Variable-size frames use the configured maximum for this
limit. These are conservative library limits. The composed format has not
undergone a security audit; cipher operations use RustCrypto.

## Storing metadata

An encoded object contains consecutive frame payloads, each followed by its
authentication tag when encryption is enabled. It has no frame headers or
embedded metadata. You need the separate metadata to decode it.

`Metadata::into_parts()` returns the configuration, frame count, and size
information for your application to serialize. Restore it with
`Metadata::from_parts(parts)`, which checks its structure and sizes but does
not authenticate it. CRAFT does not require a serialization framework.

Fixed-size frames without compression need only the final frame's length in
addition to the configuration and frame count. Variable-size frames need their
original lengths; compressed frames also need their stored payload lengths.
CRAFT stores these lengths in compact integer arrays.

See the [format specification](docs/specification.md) for the V1 format and
metadata rules.

## Integrating with I/O

For an async reader, fetch the stored range and await each complete frame before
calling the synchronous decoder. Keep the original frame index and sizes from
`frame.spec`, including when a read starts in the middle of an object. Your
application can move CPU work to a worker thread when needed.

Encoding and decoding modify the supplied buffer. Copy borrowed input into a
reusable work buffer if you need to preserve it for retries. Compression uses
reusable scratch space and may swap allocations with that buffer. A
transformation error clears the buffer, but does not securely erase its spare
capacity or the compression scratch space.

See the [examples guide](examples/README.md) for memory I/O and an adapter for
CARBON I/O, which schedules concurrent reads and writes. The adapter keeps
encoded bytes in its own buffer across partial writes and retries.

## Development

```sh
cargo test --all-features
cargo test --no-default-features
cargo test --no-default-features --features lz4
cargo clippy --all-features --all-targets -- -D warnings
cargo run --example memory --no-default-features
cargo bench --all-features --bench frames
```

The crate forbids unsafe code. See the [benchmark notes](docs/benchmarks.md) for
performance measurements.

MIT licensed.
