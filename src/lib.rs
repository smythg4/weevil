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
