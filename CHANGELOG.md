# Changelog

## 0.1.0

- Synchronous frame encoding and decoding with fixed or variable logical sizes.
- Optional AES-256-GCM and independent LZ4 compression with raw fallback.
- Compact external metadata, validated restoration and allocation-free range mapping.
- Reusable buffers, empty output on codec errors and bounded AES frame slots.
- Standalone CARBON integration with immutable write inputs and partial-write state.
