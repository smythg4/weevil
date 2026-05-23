# weevil

A tiny toy implementation of core [TigerBeetle](https://tigerbeetle.com) concepts in Rust. Built as a learning exercise in systems programming — zero-copy I/O, non-blocking network, and append-only durable storage.

## What it is

Weevil is a single-threaded TCP server that accepts financial transactions from concurrent clients, appends them to per-account binary log files, and responds with account balance information. It is not a real financial system.

## Concepts explored

- **Zero-copy wire protocol** — all messages are fixed 32-byte `#[repr(C)]` structs cast directly from network buffers using `bytemuck`. No serialization layer.
- **Non-blocking I/O** — a single-threaded `mio` event loop handles multiple concurrent connections without threads.
- **Append-only durable storage** — transactions are written as raw bytes to per-account `.log` files and flushed with `fdatasync` at the end of each event loop batch.
- **Batch commit with response ordering** — transactions accumulate across one poll iteration. At the end of each iteration, dirty account logs are written and flushed with `fdatasync`. Only after the flush completes are sessions promoted from `AwaitingCommit` to `Writing`, guaranteeing no client receives a response before its transaction is durable on disk.
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
mkdir data_files
# start the server
cargo run --bin server

# run the test client (4 concurrent threads)
cargo run --bin client
```

The client registers each account, sends a series of random deposits and withdrawals, then queries the final balance. Log files are written to `./data_files/` and persist across restarts.

## What it is not

Weevil omits most of what makes TigerBeetle production-worthy: explicit error types, `O_DIRECT`, checksums, a WAL, cluster replication, and anything resembling fault tolerance. It is a learning artifact.

## Next Steps

- **Static memory allocation** — replace `HashMap`, `Vec`, and `Box<dyn Error>` with fixed-size arrays and explicit error enums allocated once at startup. TODOs are peppered throughout the code. ([A Database Without Dynamic Memory Allocation](https://tigerbeetle.com/blog/2022-10-12-a-database-without-dynamic-memory/))

- **Separate credits and debits** — replace `cached_balance: i128` with `credits_posted: u128` and `debits_posted: u128`. Signed balance hides transaction volume and introduces sign ambiguity. Also eliminate `as f64` in `Display` — use integer arithmetic for all money formatting. ([64-Bit Bank Balances 'Ought to be Enough for Anybody'?](https://tigerbeetle.com/blog/2023-09-19-64-bit-bank-balances-ought-to-be-enough-for-anybody))

- **Copy hunting** — `ParsedMessage::Transaction(*tx)` copies 32 bytes out of the aligned read buffer on every message. `format!(...).into_bytes()` allocates on every response. Use LLVM IR to find these systematically and eliminate them. ([Copy Hunting](https://tigerbeetle.com/blog/2023-07-26-copy-hunting/))

- **Assertion discipline** — external data panics are largely addressed: `message_kind` validation in replay returns an error, `handle_write` asserts `write_buf` is populated as an internal invariant. Remaining: `tx.kind()` in the replay loop has `unreachable!()` on a corrupt `transaction_kind` byte from disk — external data that should be a soft error. ([Asserting Implications](https://tigerbeetle.com/blog/2025-05-26-asserting-implications/))

- **Naming discipline** — `bytes_read` in `Session` is a byte offset into a fixed 32-byte buffer; the invariant `bytes_read < 32` should be explicit. Apply index/count/offset/size naming conventions throughout. ([Index, Count, Offset, Size](https://tigerbeetle.com/blog/2026-02-16-index-count-offset-size))

- **CRC32 checksums** — repurpose 4 bytes of padding in `Transaction` and `Account` into a `checksum: u32` field. Compute over the remaining bytes of the struct; verify on the way in (network) and on the way out (log replay in `AccountEntry::new`). Expanding to 64-byte structs first resolves the alignment constraint for fitting a `u32` cleanly without displacing `message_kind`. Implement using the Hacker's Delight bitwise CRC32 approach — table-free, branch-light, no dependencies.

- **io_uring** — `mio` uses a readiness model (kernel signals fd is ready, userland makes the syscall). `io_uring` uses a completion model (userland submits I/O, kernel does the syscall). Eliminates the context switch on the syscall itself. Significant architectural change but the direction TigerBeetle went. ([A Programmer-Friendly I/O Abstraction Over io_uring and kqueue](https://tigerbeetle.com/blog/2022-11-23-a-friendly-abstraction-over-iouring-and-kqueue))