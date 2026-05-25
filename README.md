# weevil

A tiny toy implementation of core [TigerBeetle](https://tigerbeetle.com) concepts in Rust. Built as a learning exercise in systems programming — zero-copy I/O, non-blocking network, append-only durable storage, and static memory allocation.

## What it is

Weevil is a single-threaded TCP server that accepts financial transactions from concurrent clients, appends them to per-account binary log files, and responds with account balance information. It is not a real financial system.

## Concepts explored

- **Zero-copy wire protocol** — all messages are fixed 64-byte `#[repr(C)]` structs cast directly from network buffers using `bytemuck`. No serialization layer. Responses are the same: a fixed 64-byte `AccountResponse` struct cast directly to the wire.
- **Non-blocking I/O** — a single-threaded `mio` event loop handles multiple concurrent connections without threads.
- **Append-only durable storage** — transactions are written as raw bytes to per-account `.log` files and flushed with `fdatasync` at the end of each event loop batch.
- **Batch commit with response ordering** — transactions accumulate across one poll iteration. At the end of each iteration, dirty account logs are written and flushed with `fdatasync`. Only after the flush completes are sessions promoted from `AwaitingCommit` to `Writing`, guaranteeing no client receives a response before its transaction is durable on disk.
- **Separate debit and credit accumulators** — `AccountEntry` tracks `debit_balance: u128` and `credit_balance: u128` as independent unsigned accumulators rather than a single signed balance. Unsigned types cannot go negative, transaction volume is preserved in both directions, and the net balance is derived by comparison and safe subtraction at display time. Matches the TigerBeetle model. ([64-Bit Bank Balances 'Ought to be Enough for Anybody'?](https://tigerbeetle.com/blog/2023-09-19-64-bit-bank-balances-ought-to-be-enough-for-anybody))
- **Batched disk writes** — all pending transactions for an account are written in a single `write_all(bytemuck::cast_slice(...))` call rather than one syscall per transaction. `bytemuck::cast_slice` reinterprets the contiguous `[Transaction; N]` array as a flat `&[u8]` with no copying.
- **Balance replay** — on startup, each account's balance is reconstructed by replaying its log file 64 bytes at a time.
- **Static connection table** — connections are stored in a fixed `[Option<Session>; MAX_CONNECTIONS]` array. The mio `Token` is a direct array index. No `HashMap`, no hashing, no pointer chasing — O(1) lookup by design.
- **Static account cache with open addressing** — accounts are stored in a fixed `[Option<AccountEntry>; MAX_ACCOUNTS]` array. Slot selection uses modulo hashing with linear probing and full wrap-around — no `HashMap`, no heap allocation. `MAX_ACCOUNTS` is prime (257) to reduce probe clustering.
- **Static pending transaction buffer** — each `AccountEntry` holds a `[Transaction; MAX_BATCH]` array with a `len` counter. No `Vec`, no heap growth. When the batch is full, `add_transaction` returns an error rather than flushing inline, preserving the batch commit guarantee.
- **Type-state response buffer** — `SessionStatus::AwaitingCommit([u8; 64])` and `Writing([u8; 64])` carry the response payload inside the state. The type system enforces that a session cannot be in `Writing` state without a response ready to send. No separate `write_buf` field, no `Option` to unwrap.
- **CRC32 checksums** — every wire message and every log record carries a CRC32 checksum in repurposed padding bytes. Computed in `new()` with the checksum field zeroed, verified on network ingress and during startup log replay. Implemented using the Hacker's Delight bitwise approach — reflected polynomial `0xEDB88320`, table-free, no dependencies.

## Protocol

All messages are 64 bytes. Client-to-server messages are distinguished by the final byte (`message_kind`). Server-to-client responses are always `AccountResponse`.

### Client → Server

| Byte offset | Field | Type |
|---|---|---|
| **Transaction** (`message_kind = 1`) | | |
| 0–15 | amount | u128 |
| 16–23 | account_id | u64 |
| 24 | transaction_kind (0=debit, 1=credit) | u8 |
| 25–62 | padding | [u8; 38] |
| 63 | message_kind = 1 | u8 |
| **Account** (`message_kind = 0`) | | |
| 0–7 | account_id | u64 |
| 8–62 | padding | [u8; 55] |
| 63 | message_kind = 0 | u8 |

### Server → Client

| Byte offset | Field | Type |
|---|---|---|
| 0–15 | debit_balance | u128 |
| 16–31 | credit_balance | u128 |
| 32–39 | account_id | u64 |
| 40–62 | padding | [u8; 23] |
| 63 | status | u8 |

The response reflects the committed balances at the previous flush boundary — the pending transaction has been accepted into the batch but balances are updated when the batch is written to disk, not at enqueue time.

| `status` | Meaning |
|---|---|
| 0 | Success |
| 1 | Account not found |
| 2 | Account cache full |

## Running

```sh
# start the server (creates ./data_files/ automatically on first run)
cargo run --bin server

# run the test client (NUM_THREADS concurrent connections, NUM_TRANSACTIONS each)
cargo run --bin client
```

The client registers each account, sends a series of random debits and credits, then queries the final balance. Log files are written to `./data_files/` and persist across restarts.

## What it is not

Weevil omits most of what makes TigerBeetle production-worthy: `O_DIRECT`, checksums, a WAL, cluster replication, and anything resembling fault tolerance. It is a learning artifact.

## Next Steps

- **Single WAL file** — currently performing N fsyncs per event loop iteration, one per dirty account. Consolidating all transactions into a single append-only `wal.log` reduces that to one fsync per batch regardless of how many accounts were touched. Will require reworking the startup replay logic and removing the per-account file handle from `AccountEntry`.

