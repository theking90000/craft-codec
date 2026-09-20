# Benchmark protocol

Run `cargo bench --all-features --bench frames`. The executable writes CSV to
stdout and uses only the standard library plus the existing allocation counter.

It measures 4, 16, 64, 256 KiB and 1 MiB frames on deterministic repeated,
pseudorandom and half-repeated inputs. Each case warms its buffers and runs
for at least 25 ms, in batches of 16 calls. These are single-process smoke
measurements, not confidence intervals or cross-platform guarantees.

Encoder and decoder rows include restoring their input buffer before each
operation, as a borrowing integration would. They report raw MiB/s, average
nanoseconds per frame, stored size, allocations and reallocations. They do not
include I/O, key initialization, metadata construction or task scheduling.

The direct AES baseline includes the input copy and detached encryption but
does not append its returned tag. The direct LZ4 baseline compresses straight
from the input slice, without the input copy or CRAFT's fallback selection.
Those differences must be considered when comparing the rows. Direct codec
baselines currently cover encoding only.

The metadata row maps a short selection near the end of 16,384 variable frames
with compressed lengths. Its MiB/s column is zero; compare its latency and
allocation counts. Compact arrays consume two bytes per raw length and two
bytes per payload length at a configured maximum of 64 KiB. There is no
additional per-frame offset array.

`tests/allocations.rs` separately asserts zero allocations and reallocations
after warming reusable codec buffers. It excludes growing metadata and owned
output frames retained by consumers. No allocator pool is hidden in the crate.

Record the machine, Rust version, enabled features and build profile alongside
any saved result. Do not extrapolate these CPU measurements to network latency
or automatically choose a runtime offload threshold from frame size alone.
