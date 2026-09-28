# Changelog

## 0.1.2 - 2026-09-28

- Replace panic-prone metadata and AES buffer handling with checked operations and structured errors.
- Propagate failures from the in-memory and CARBON examples instead of unwrapping results.

## 0.1.1 - 2026-09-27

- Add optional Serde support for configurations, metadata and compact length tables.
- Validate deserialized metadata and recalculate its logical and stored lengths.

## 0.1.0

- Synchronous frame encoding and decoding with fixed or variable logical sizes.
- Optional AES-256-GCM and independent LZ4 compression with raw fallback.
- Compact external metadata, validated restoration and allocation-free range mapping.
- Reusable buffers, empty output on codec errors and bounded AES frame slots.
- Standalone CARBON integration with immutable write inputs and partial-write state.
