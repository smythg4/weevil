pub mod account;
pub mod error;
pub mod transaction;

pub use error::WeevilError;

#[repr(u8)]
pub enum MessageKind {
    Account,
    Transaction,
}

pub fn crc32(input: &[u8]) -> u32 {
    // Consider upgrading the table lookup version if speed is a concern
    let mut crc: u32 = 0xFFFFFFFF;
    for byte in input {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB88320 & mask);
        }
    }
    !crc
}
