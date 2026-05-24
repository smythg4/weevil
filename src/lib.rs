pub mod account;
pub mod error;
pub mod transaction;

#[repr(u8)]
pub enum MessageKind {
    Account,
    Transaction,
}

pub use error::WeevilError;
