pub mod account;
pub mod transaction;

pub type GenericError = Box<dyn std::error::Error>;

#[repr(u8)]
pub enum MessageKind {
    Account,
    Transaction,
}
