use bytemuck::{Pod, Zeroable};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

use crate::GenericError;
use crate::MessageKind;
use crate::transaction::{Transaction, TransactionKind};

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct Account {
    pub account_id: u64,
    _pad: [u8; 23],
    message_kind: u8,
}

impl Account {
    pub fn new(account_id: u64) -> Self {
        Account {
            account_id,
            _pad: [0u8; 23],
            message_kind: MessageKind::Account as u8,
        }
    }
}

impl std::fmt::Display for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] Account", self.account_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_cast() {
        let mut bytes = [0u8; 32];
        // account_id: 42 as u64, little-endian at offset 0
        bytes[0..8].copy_from_slice(&42u64.to_le_bytes());
        // message_kind = 0 (Account)
        bytes[31] = 0;

        let acct: &Account = bytemuck::from_bytes(&bytes);
        assert_eq!(acct.account_id, 42);
        assert_eq!(acct.message_kind, MessageKind::Account as u8);
    }
}

const MAX_BATCH: usize = 100;

pub struct AccountEntry {
    account_id: u64,
    file_backing: File,
    pub cached_balance: i128,
    pending_transactions: [Transaction; MAX_BATCH],
    len: usize,
    dirty: bool,
}

impl std::fmt::Display for AccountEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] Balance: ${:.2}",
            self.account_id,
            self.cached_balance as f64 / 1000.0
        )
    }
}

impl AccountEntry {
    pub fn new(account_id: u64) -> Result<Self, GenericError> {
        let path = format!("./data_files/{account_id}.log");
        let mut f = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)?;
        let mut cached_balance = 0;
        let mut buf = [0u8; 32];
        loop {
            match f.read_exact(&mut buf) {
                Ok(_) => {
                    if buf[31] != MessageKind::Transaction as u8 {
                        return Err(String::from("invalid message kind byte").into());
                    }
                    let tx = bytemuck::pod_read_unaligned::<Transaction>(&buf);
                    match tx.kind() {
                        TransactionKind::Deposit => cached_balance += tx.amount as i128,
                        TransactionKind::Withdrawal => cached_balance -= tx.amount as i128,
                    };
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            };
        }
        Ok(AccountEntry {
            account_id,
            file_backing: f,
            cached_balance,
            pending_transactions: [Transaction::default(); MAX_BATCH],
            len: 0,
            dirty: false,
        })
    }

    pub fn add_transaction(&mut self, tx: Transaction) -> Result<(), GenericError> {
        if self.len >= MAX_BATCH {
            return Err("pending_transactions full".into());
        }
        self.dirty = true;
        self.pending_transactions[self.len] = tx;
        self.len += 1;
        Ok(())
    }

    pub fn write(&mut self) -> Result<(), GenericError> {
        for tx in &self.pending_transactions[0..self.len] {
            self.file_backing.write_all(bytemuck::bytes_of(tx))?;
            match tx.kind() {
                TransactionKind::Deposit => self.cached_balance += tx.amount as i128,
                TransactionKind::Withdrawal => self.cached_balance -= tx.amount as i128,
            };
        }
        self.len = 0;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<(), GenericError> {
        if self.dirty {
            self.file_backing.sync_data()?;
            self.dirty = false;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), GenericError> {
        self.write()?;
        self.sync()
    }

    pub fn response(&self) -> AccountResponse {
        AccountResponse {
            cached_balance: self.cached_balance,
            account_id: self.account_id,
            _pad: [0u8; 7],
            status: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable, PartialEq, Eq)]
pub struct AccountResponse {
    cached_balance: i128,
    account_id: u64,
    _pad: [u8; 7],
    status: u8,
}

pub const NOT_FOUND: AccountResponse = AccountResponse {
    cached_balance: 0,
    account_id: 0,
    _pad: [0u8; 7],
    status: 1,
};

impl std::fmt::Display for AccountResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.status != 0 {
            return write!(f, "Account not found");
        }
        if self.cached_balance >= 0 {
            write!(
                f,
                "[{}] ${:.2}",
                self.account_id,
                self.cached_balance as f64 / 1000.0
            )
        } else {
            write!(
                f,
                "[{}] -${:.2}",
                self.account_id,
                self.cached_balance.abs() as f64 / 1000.0
            )
        }
    }
}
