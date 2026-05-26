pub mod account;
pub mod account_cache;
pub mod error;
pub mod transfer;

pub use error::WeevilError;

// Maximum number of connections the server will accept
pub const MAX_CONNECTIONS: usize = 256;

// Maximum number of accounts to handle
pub const MAX_ACCOUNTS: usize = 256;

// Maximum size of the WAL file before a snapshot
const MAX_WAL_SIZE: u64 = 1024 * 1024; // 1MB max

// Maximum number of transfers to store in memory before flushing to disk
const MAX_BATCH: usize = 1000;

#[repr(u8)]
pub enum MessageKind {
    Account,
    Transaction,
}

pub fn crc32(input: &[u8]) -> u32 {
    // Consider upgrading the table lookup version if speed is a concern
    !crc32_update(0xFFFFFFFF, input)
}

pub fn crc32_update(state: u32, input: &[u8]) -> u32 {
    let mut crc: u32 = state;
    for byte in input {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB88320 & mask);
        }
    }
    crc
}
