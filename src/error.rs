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

    #[error("self transfer: {0}")]
    SelfTransfer(u64), // account_id
}
