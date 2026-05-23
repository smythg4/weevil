pub mod account;
pub mod transaction;

#[repr(u8)]
pub enum MessageKind {
    Account,
    Transaction,
}
