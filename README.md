# CRAFT I/O

Cryptographic Random-Access Framing Toolkit.

`craft-io` encodes independent frames and maps logical byte ranges to their
stored representation. Its single synchronous API works on reusable `Vec<u8>`
buffers. The caller supplies I/O, scheduling, retries and trusted metadata storage.

```text
raw frame → optional compression → optional encryption → stored frame
stored frame → authentication/decryption → decompression → raw frame
```

## Encode an object

Use `Identity` for either disabled transformation. Choose fixed frames with
a possibly shorter final frame, or variable frames with an explicit maximum.

```rust
use craft_io::{Decoder, Encoder, Framing, Identity, Metadata};

let mut encoder = Encoder::new(Framing::Fixed(8), Identity, Identity)?;
let mut metadata = Metadata::new(encoder.config());
let original = b"small independent frames";
let mut object = Vec::new();
let mut buffer = Vec::new();

for (index, raw) in original.chunks(8).enumerate() {
    buffer.clear();
    buffer.extend_from_slice(raw);
    let sizes = encoder.encode_frame(index as u64, &mut buffer)?;
    object.extend_from_slice(&buffer); // Replace with your backend's write.
    metadata.push(sizes)?;            // Append only after a successful write.
}

// Persist metadata separately and publish it with the completed object.
let metadata = Metadata::from_parts(metadata.into_parts())?;

// Read exactly the physical frames covering logical bytes [3, 19).
let (query, frames) = metadata.range(3, Some(19))?;
let mut source = &object[query.start as usize..query.end as usize];
let mut decoder = Decoder::new(metadata.config(), Identity, Identity)?;
let mut result = Vec::new();

for frame in frames {
    buffer.resize(frame.spec.stored_len(), 0);
    std::io::Read::read_exact(&mut source, &mut buffer)?;
    decoder.decode_frame(frame.spec, &mut buffer)?;
    result.extend_from_slice(&buffer[frame.selected]);
}
assert_eq!(result, original[3..19]);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`range(start, None)` reads through logical EOF. End bounds are exclusive.
Reversed or out-of-bounds ranges fail; empty ranges produce an empty query
and iterator and should not open a backend.

For an async backend, open once with
`open(query.start, Some(query.end - query.start))`, await complete frame reads,
then call the same synchronous decoder. The codec sees the original frame
index and sizes, never the logical slice selected by the caller.

## Enable compression and encryption

```toml
[dependencies]
craft-io = { version = "0.1", features = ["lz4"] }
```

The `aes-gcm` feature is enabled by default; `lz4` is optional. With
`default-features = false`, identity framing and metadata have no runtime
dependencies. All eight fixed/variable, compressed/raw, encrypted/plain
combinations use the same API.

```rust
# #[cfg(all(feature = "aes-gcm", feature = "lz4"))]
# {
use craft_io::{Aes256Gcm, Decoder, Encoder, Framing, Lz4};

// Example bytes only. Supply an independent secret key for each real object.
let key = [42; 32];
let encoder = Encoder::new(
    Framing::Variable(65_536),
    Lz4::new(),
    Aes256Gcm::new(&key),
)?;
let decoder = Decoder::new(encoder.config(), Lz4::new(), Aes256Gcm::new(&key))?;
# let _ = decoder;
# }
# Ok::<(), craft_io::Error>(())
```

LZ4 blocks are independent and retained only if strictly smaller than the raw
frame. Otherwise the raw payload is stored. Compression uses reusable scratch;
encryption modifies the chosen payload in place and appends its tag.
The buffer allocation can change through a scratch swap. Preserve a borrowed
input by copying it into a reusable work buffer before calling the encoder.

On a transformation error, the supplied buffer is cleared. This does not
promise erasure of plaintext from spare capacity or compression scratch.
Decryption authenticates the complete frame before decompression or delivery.

## Stored format and metadata

Version 1 stores concatenated `[payload][optional 16-byte tag]` records, with
no headers, padding, stored nonces or terminal record. Empty objects contain
no frames; individual frames must be nonempty.

| Logical framing | Compression | Per-frame metadata |
| --- | --- | --- |
| Fixed | None | None; only the final raw length is stored separately |
| Variable | None | Raw lengths |
| Fixed | LZ4 | Payload lengths, excluding tags |
| Variable | LZ4 | Raw and payload lengths |

`Lengths` selects `u16`, `u32` or `u64` from the configured maximum. Entries
encode `length - 1`, so a 65,536-byte frame fits in a `u16`. The last variable
frame's size is already in its index. `MetadataParts` exposes the version,
profile and compact arrays for application-owned persistence; no serialization
framework is required. `from_parts` validates structure, sizes and overflow.
It does not authenticate the metadata. Codec identifiers are `0 = identity`,
`1 = LZ4` for compression and `0 = identity`, `1 = AES-256-GCM` for encryption.

There is no prefix-offset table. Preparing a bounded variable-size query scans
metadata up to its last selected frame. Iteration then takes constant work per
frame and constant auxiliary memory. Fixed uncompressed queries use arithmetic.
Only logical and physical scalar totals are cached.

The iterator's `remaining()` reports the exact frame count without collecting
it. Metadata growth during encoding is proportional to frame count for indexed
profiles. Codec buffers and metadata allocations are distinct costs.

## Key and object contract

AES-256-GCM uses `nonce = [0; 4] || frame_index.to_be_bytes()`, empty AAD,
and the full 16-byte authentication tag. The original object index is used
even for reads starting in the middle. **Never encrypt different payloads
under the same key and frame index.** Use a fresh independent key for each
immutable object. Key generation and storage remain outside the crate.

Re-reading frames is unrestricted. Replaying the same ciphertext is allowed;
re-encoding under an existing key/index must reproduce exactly the same
payload. A new attempt that can change the payload needs a new key. The codec
does not maintain a nonce registry across calls or instances.

The version 1 AES usage policy caps configured frames at 16 MiB and permits
at most `min(2^32, 64 GiB / max_frame_len)` frame slots per object/key. This
reserves the maximum raw length for each index, also in variable mode. A
64 KiB configuration therefore allows 1,048,576 frames. These are conservative
library limits, not a claim of 128-bit security at the maximum volume or a
security audit of the composed format. Cipher operations come from RustCrypto.

Metadata, its association with the object, and the key must be trusted.
Truncation of entire final frames is detected by the caller expecting the
declared frame lengths/count, not by the previous frame's tag. Unread frames
are not verified. Unencrypted modes provide no integrity guarantee. No padding
hides logical lengths or compression ratios.

## Integration and development

See [the examples](examples/README.md) for memory I/O and a standalone CARBON
adapter using its unchanged borrowing `FrameWriter` interface. CRAFT adds no
async traits, buffer pool, tasks or retry policy. Heavy CPU work can run in an
application-owned worker without changing the codec API.

```sh
cargo test --all-features
cargo test --no-default-features
cargo test --no-default-features --features lz4
cargo clippy --all-features --all-targets -- -D warnings
cargo run --example memory --no-default-features
cargo bench --all-features --bench frames
```

Rust 1.85 or later. The crate forbids unsafe code. See the
[specification](docs/specification.md) and [benchmark notes](docs/benchmarks.md).

MIT licensed.
