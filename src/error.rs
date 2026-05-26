use thiserror::Error;

#[derive(Debug, Error)]
pub enum WeevilError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid message kind: {0}")]
    InvalidMessageKind(u8),

    #[error("pending transactions full")]
    PendingTransactionsFull,

    #[error("checksum failed")]
    ChecksumFailed,

    #[error("invalid account_id debit: {0} or credit: {1}")]
    InvalidAccountId(u64, u64), // (debit id, credit id)
}
