# weevil

A tiny toy implementation of core [TigerBeetle](https://tigerbeetle.com) concepts in Rust. Built as a learning exercise in systems programming — zero-copy I/O, non-blocking network, and append-only durable storage.

## What it is

Weevil is a single-threaded TCP server that accepts financial transactions from concurrent clients, appends them to per-account binary log files, and responds with account balance information. It is not a real financial system.

## Concepts explored

- **Zero-copy wire protocol** — all messages are fixed 32-byte `#[repr(C)]` structs cast directly from network buffers using `bytemuck`. No serialization layer.
- **Non-blocking I/O** — a single-threaded `mio` event loop handles multiple concurrent connections without threads.
- **Append-only durable storage** — transactions are written as raw bytes to per-account `.log` files and flushed with `fdatasync` at the end of each event loop batch.
- **Batch commit** — transactions accumulate across one poll iteration and are flushed together, amortising the cost of `fdatasync` across multiple writes.
- **Balance replay** — on startup, each account's balance is reconstructed by replaying its log file 32 bytes at a time.

## Protocol

Two message types, both 32 bytes, distinguished by the final byte (`message_kind`):

| Byte offset | Field | Type |
|---|---|---|
| **Transaction** | | |
| 0–15 | amount | u128 |
| 16–23 | account_id | u64 |
| 24 | transaction_kind (0=deposit, 1=withdrawal) | u8 |
| 25–30 | padding | [u8; 6] |
| 31 | message_kind = 1 | u8 |
| **Account** | | |
| 0–7 | account_id | u64 |
| 8–30 | padding | [u8; 23] |
| 31 | message_kind = 0 | u8 |

## Running

```sh
# start the server
cargo run --bin server

# run the test client (4 concurrent threads)
cargo run --bin client
```

The client registers each account, sends a series of random deposits and withdrawals, then queries the final balance. Log files are written to `./data_files/` and persist across restarts.

## What it is not

Weevil omits most of what makes TigerBeetle production-worthy: response ordering after fsync, explicit error types, `O_DIRECT`, checksums, a WAL, cluster replication, and anything resembling fault tolerance. It is a learning artifact.
