use bytemuck::{Pod, Zeroable};

use crate::MessageKind;

#[repr(u8)]
pub enum TransactionKind {
    Deposit,
    Withdrawal,
}

// TODO: Add a txid field for idempotency purposes
// Ex: Client sents tx with id, responses are acknowldged
// with the same id
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct Transaction {
    pub amount: u128,
    pub account_id: u64,
    transaction_kind: u8,
    _pad: [u8; 6],
    message_kind: u8,
}

impl Transaction {
    pub fn new(amount: u128, account_id: u64, transaction_kind: TransactionKind) -> Self {
        Transaction {
            amount,
            account_id,
            transaction_kind: transaction_kind as u8,
            message_kind: MessageKind::Transaction as u8,
            _pad: [0u8; 6],
        }
    }

    pub fn kind(&self) -> TransactionKind {
        match self.transaction_kind {
            0 => TransactionKind::Deposit,
            1 => TransactionKind::Withdrawal,
            _ => unreachable!(),
        }
    }
}

impl std::fmt::Display for Transaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sign = match self.kind() {
            TransactionKind::Deposit => "",
            TransactionKind::Withdrawal => "-",
        };
        let value = self.amount as f64 / 1000.0;
        write!(f, "[{}] {:2}${:.2}", self.account_id, sign, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_cast() {
        let mut bytes = [0u8; 32];
        // amount: 1000 as u128, little-endian at offset 0
        bytes[0..16].copy_from_slice(&1000u128.to_le_bytes());
        // account_id: 42 as u64, little-endian at offset 16
        bytes[16..24].copy_from_slice(&42u64.to_le_bytes());
        // kind: 1 (Withdrawal) at offset 24
        bytes[24] = 1;
        // message_kind = 1 (Transaction)
        bytes[31] = 1;

        let tx: &Transaction = bytemuck::from_bytes(&bytes);
        assert_eq!(tx.amount, 1000);
        assert_eq!(tx.account_id, 42);
        assert_eq!(tx.transaction_kind, 1);
        assert_eq!(tx.message_kind, MessageKind::Transaction as u8);
    }
}
