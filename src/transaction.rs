use bytemuck::{Pod, Zeroable};

use crate::{MessageKind, WeevilError, crc32};

#[repr(u8)]
pub enum TransactionKind {
    Debit,
    Credit,
}

impl std::fmt::Display for TransactionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionKind::Debit => write!(f, "DEBIT"),
            TransactionKind::Credit => write!(f, "CREDIT"),
        }
    }
}

const _: () = assert!(std::mem::size_of::<Transaction>() == 64);

// TODO: Add a txid field for idempotency purposes
// Ex: Client sents tx with id, responses are acknowldged
// with the same id
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod, Zeroable)]
pub struct Transaction {
    pub amount: u128,
    pub account_id: u64,
    transaction_kind: u8,
    _pad: [u8; 3],
    pub checksum: u32,
    _pad2: [u8; 31],
    message_kind: u8,
}

impl Transaction {
    pub fn new(amount: u128, account_id: u64, transaction_kind: TransactionKind) -> Self {
        let mut tx = Transaction {
            amount,
            account_id,
            transaction_kind: transaction_kind as u8,
            message_kind: MessageKind::Transaction as u8,
            _pad: [0u8; 3],
            checksum: 0,
            _pad2: [0u8; 31],
        };
        let checksum = crc32(bytemuck::bytes_of(&tx));
        tx.checksum = checksum;
        tx
    }

    pub fn kind(&self) -> Result<TransactionKind, WeevilError> {
        match self.transaction_kind {
            0 => Ok(TransactionKind::Debit),
            1 => Ok(TransactionKind::Credit),
            _ => Err(WeevilError::InvalidMessageKind(self.transaction_kind)),
        }
    }

    pub fn verify(&self) -> Result<(), WeevilError> {
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

impl std::fmt::Display for Transaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = self.amount as f64 / 1000.0;
        write!(f, "[{}] ", self.account_id)?;
        if let Ok(k) = self.kind() {
            write!(f, "{k} ")?;
        } else {
            write!(f, "UNKNOWN ")?;
        };
        write!(f, "${:.2}", value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_cast() {
        let mut bytes = [0u8; 64];
        // amount: 1000 as u128, little-endian at offset 0
        bytes[0..16].copy_from_slice(&1000u128.to_le_bytes());
        // account_id: 42 as u64, little-endian at offset 16
        bytes[16..24].copy_from_slice(&42u64.to_le_bytes());
        // kind: 1 (Credit) at offset 24
        bytes[24] = 1;
        // message_kind = 1 (Transaction)
        bytes[63] = 1;

        let tx: Transaction = bytemuck::pod_read_unaligned(&bytes);
        assert_eq!(tx.amount, 1000);
        assert_eq!(tx.account_id, 42);
        assert_eq!(tx.transaction_kind, 1);
        assert_eq!(tx.message_kind, MessageKind::Transaction as u8);
    }
}
