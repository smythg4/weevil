//! src/lib.rs

pub mod account;
pub mod account_cache;
pub mod error;
pub mod transfer;

pub use error::WeevilError;

// Maximum number of connections the server will accept
pub const MAX_CONNECTIONS: usize = 256;

// Maximum number of accounts to handle
pub const MAX_ACCOUNTS: usize = 257; // prime number to reduce probe clustering

// Maximum size of the WAL file before a snapshot
pub const MAX_WAL_SIZE: u64 = 1024 * 1024; // 1MB max

// Maximum number of transfers to store in memory before flushing to disk
pub const MAX_BATCH: usize = 1000;

// The file path of the WAL file
pub const WAL_PATH: &str = "./data_files/wal.log";

// The file path of the checkpoint file which stores account balances
pub const CHECKPOINT_PATH: &str = "./data_files/checkpoint";
// The file path of a temp file to hold the checkpoint file data before
// copying it over to the actual checkpoint file and maintain atomicity
pub const TEMP_CHECKPOINT_PATH: &str = "./data_files/checkpoint.tmp";

/// Wire discriminate byte found at the last byte of a Weevil wire message
/// determines how the server interprets the preceding 63 bytes
#[repr(u8)]
pub enum MessageKind {
    Account,
    Transfer,
}

const TABLE: [u32; 256] = compute_crc32_table();

/// precomputes CRC32 lookup table to enable processing one byte at a time instead of
/// one bit at a time 
const fn compute_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut byte = 0;
    while byte < 256 {
        let mut crc = byte as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB88320 & mask);
            bit += 1;
        }
        table[byte as usize] = crc;
        byte += 1;
    }

    table
}

/// calculates the crc32 checksum of a byte slice
pub fn crc32(input: &[u8]) -> u32 {
    !crc32_update(0xFFFFFFFF, input)
}

/// takes a slice of byte slices and calculates CRC32 as if it was a continuous sequence
/// of bytes
pub fn crc32_chained(input: &[&[u8]]) -> u32 {
    !input
        .iter()
        .fold(0xFFFFFFFF, |state, chunk| crc32_update(state, chunk))
}

/// calculates an intermediate crc32 checksum given an initial state
/// and a new byte slice. used to support `crc32_chained`
fn crc32_update(state: u32, input: &[u8]) -> u32 {
    input.iter().fold(state, |crc, byte| {
        (crc >> 8) ^ TABLE[((crc ^ *byte as u32) & 0xFF) as usize]
    })
}
