# Changelog

## 0.2.0 - 2026-10-10

- Breaking: construct non-generic `Encoder` and `Decoder` through `TryInto` from self-contained metadata.
- Store all codec initialization parameters, including the AES-256-GCM key, in serializable metadata.
- Remove separate codec instances, key arguments and public codec traits from the construction API.
- Require metadata schema version 2. Metadata from releases before 0.2.0 is explicitly unsupported; missing and unknown schemas are rejected.
- Keep frame format version 1 and the encoded frame layout unchanged.
- Deserialize metadata independently of enabled codecs; report unavailable codecs at conversion.
- Update examples, benchmarks and behavior tests; add binary persistence coverage with postcard.

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
