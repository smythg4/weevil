use bytemuck::{Pod, Zeroable};

use crate::{MessageKind, WeevilError, crc32};

// DEAD CODE
#[derive(Debug, Copy, Clone)]
#[repr(u8)]
pub enum TransactionKind {
    Debit,
    Credit,
}

// DEAD CODE
impl std::fmt::Display for TransactionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionKind::Debit => write!(f, "DEBIT"),
            TransactionKind::Credit => write!(f, "CREDIT"),
        }
    }
}

const _: () = assert!(std::mem::size_of::<Transfer>() == 64);

// TODO: Add a txid field for idempotency purposes
// Ex: Client sents tx with id, responses are acknowldged
// with the same id
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod, Zeroable)]
pub struct Transfer {
    pub amount: u128,
    pub debit_account_id: u64,
    pub credit_account_id: u64,
    pub checksum: u32,
    _pad: [u8; 27],
    message_kind: u8,
}

impl Transfer {
    pub fn new(amount: u128, debit_account_id: u64, credit_account_id: u64) -> Self {
        let mut tx = Transfer {
            amount,
            debit_account_id,
            credit_account_id,
            checksum: 0,
            _pad: [0u8; 27],
            message_kind: MessageKind::Transaction as u8,
        };
        let checksum = crc32(bytemuck::bytes_of(&tx));
        tx.checksum = checksum;
        tx
    }

    pub fn verify(&self) -> Result<(), WeevilError> {
        // TODO: Find a way to avoid this copy
        let mut copy = *self;
        let old_checksum = copy.checksum;
        copy.checksum = 0;
        let checksum = crc32(bytemuck::bytes_of(&copy));
        if checksum == old_checksum {
            return Ok(());
        }
        Err(WeevilError::ChecksumFailed)
    }
}

impl std::fmt::Display for Transfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = self.amount as f64 / 1000.0;
        write!(
            f,
            "[{}] Debit: ${:.2}, [{}] Credit: ${:.2}",
            self.debit_account_id, value, self.credit_account_id, value
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_cast() {
        let mut bytes = [0u8; 64];
        // amount: 1000 as u128, little-endian at offset 0
        bytes[0..16].copy_from_slice(&1000u128.to_le_bytes());
        // debit_account_id: 42 as u64, little-endian at offset 16
        bytes[16..24].copy_from_slice(&42u64.to_le_bytes());
        // credit_account_id: 9 as u64, little-endian at offset 16
        bytes[24..32].copy_from_slice(&9u64.to_le_bytes());
        // message_kind = 1 (Transaction)
        bytes[63] = 1;

        let tx: Transfer = bytemuck::pod_read_unaligned(&bytes);
        assert_eq!(tx.amount, 1000);
        assert_eq!(tx.debit_account_id, 42);
        assert_eq!(tx.credit_account_id, 9);
        assert_eq!(tx.message_kind, MessageKind::Transaction as u8);
    }
}
