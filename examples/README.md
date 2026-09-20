# Examples

The crate contains no I/O traits or runtime dependency.

`cargo run --example memory --no-default-features` encodes an in-memory object
and reads the logical range `3..19` using synchronous I/O.

The standalone CARBON example expects a sibling `../carbon-io` checkout:

```sh
cargo run --manifest-path examples/carbon/Cargo.toml
cargo test --manifest-path examples/carbon/Cargo.toml
```

It implements CARBON's existing `ReadFile`, `WriteFile` and borrowing
`FrameWriter` traits outside CRAFT. Writes copy the immutable input once,
encode once, and retain their state across simulated partial transport writes.
Reads return owned buffers with a selected slice, without copying that slice.
The example uses a fixed test key and an in-memory backend, not a production
key source or HTTP client. Production keys must be independent per object.
Replays under one key must reproduce exactly the same payload at each index.

The example's allocation and transport policies belong to the application.
The library's sync codec API is identical in both examples.
