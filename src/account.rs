use bytemuck::{Pod, Zeroable};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

use crate::WeevilError;
use crate::transaction::{Transaction, TransactionKind};
use crate::{MessageKind, crc32};

const _: () = assert!(std::mem::size_of::<Account>() == 64);
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct Account {
    pub account_id: u64,
    pub checksum: u32,
    _pad: [u8; 31],
    _pad2: [u8; 20],
    message_kind: u8,
}

impl Account {
    pub fn new(account_id: u64) -> Self {
        let mut a = Account {
            account_id,
            checksum: 0,
            _pad: [0u8; 31],
            _pad2: [0u8; 20],
            message_kind: MessageKind::Account as u8,
        };
        let checksum = crc32(bytemuck::bytes_of(&a));
        a.checksum = checksum;
        a
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

impl std::fmt::Display for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] Account", self.account_id)
    }
}

const MAX_BATCH: usize = 100;
pub struct AccountEntry {
    pub account_id: u64,
    file_backing: File,
    pub debit_balance: u128,
    pub credit_balance: u128,
    pending_transactions: [Transaction; MAX_BATCH],
    len: usize,
    dirty: bool,
}

impl std::fmt::Display for AccountEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.debit_balance > self.credit_balance {
            write!(
                f,
                "[{}] Debit Balance: ${:.2}",
                self.account_id,
                (self.debit_balance - self.credit_balance) as f64 / 1000.0,
            )
        } else {
            write!(
                f,
                "[{}] Credit Balance: ${:.2}",
                self.account_id,
                (self.credit_balance - self.debit_balance) as f64 / 1000.0,
            )
        }
    }
}

impl AccountEntry {
    pub fn new(account_id: u64) -> Result<Self, WeevilError> {
        let path = format!("./data_files/{account_id}.log");
        let mut f = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)?;
        let mut credit_balance = 0;
        let mut debit_balance = 0;
        let mut buf = [0u8; 64];
        loop {
            match f.read_exact(&mut buf) {
                Ok(_) => {
                    if buf[63] != MessageKind::Transaction as u8 {
                        return Err(WeevilError::InvalidMessageKind(buf[63]));
                    }
                    let tx = bytemuck::pod_read_unaligned::<Transaction>(&buf);
                    tx.verify()?;
                    match tx.kind()? {
                        TransactionKind::Debit => debit_balance += tx.amount,
                        TransactionKind::Credit => credit_balance += tx.amount,
                    };
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            };
        }
        Ok(AccountEntry {
            account_id,
            file_backing: f,
            debit_balance,
            credit_balance,
            pending_transactions: [Transaction::default(); MAX_BATCH],
            len: 0,
            dirty: false,
        })
    }

    pub fn add_transaction(&mut self, tx: Transaction) -> Result<(), WeevilError> {
        if self.len >= MAX_BATCH {
            return Err(WeevilError::PendingTransactionsFull);
        }
        self.dirty = true;
        self.pending_transactions[self.len] = tx;
        self.len += 1;
        Ok(())
    }

    pub fn write(&mut self) -> Result<(), WeevilError> {
        self.file_backing.write_all(bytemuck::cast_slice(
            &self.pending_transactions[0..self.len],
        ))?;
        for tx in &self.pending_transactions[0..self.len] {
            match tx.kind()? {
                TransactionKind::Debit => self.debit_balance += tx.amount,
                TransactionKind::Credit => self.credit_balance += tx.amount,
            };
        }
        self.len = 0;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<(), WeevilError> {
        if self.dirty {
            self.file_backing.sync_data()?;
            self.dirty = false;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), WeevilError> {
        self.write()?;
        self.sync()
    }

    pub fn response(&self) -> AccountResponse {
        let mut ar = AccountResponse {
            debit_balance: self.debit_balance,
            credit_balance: self.credit_balance,
            account_id: self.account_id,
            checksum: 0,
            _pad: [0u8; 19],
            status: 0,
        };
        let checksum = crc32(bytemuck::bytes_of(&ar));
        ar.checksum = checksum;
        ar
    }
}

const _: () = assert!(std::mem::size_of::<AccountResponse>() == 64);

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable, PartialEq, Eq)]
pub struct AccountResponse {
    debit_balance: u128,
    credit_balance: u128,
    account_id: u64,
    pub checksum: u32,
    _pad: [u8; 19],
    status: u8,
}

impl AccountResponse {
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

// NOTE: These checksums will fail to check if the struct changes
// Consider lazy static initialization or functions to return them.
pub const NOT_FOUND: AccountResponse = AccountResponse {
    debit_balance: 0u128,
    credit_balance: 0u128,
    account_id: 0,
    checksum: 0x028A53A0,
    _pad: [0u8; 19],
    status: 1,
};

pub const CACHE_FULL: AccountResponse = AccountResponse {
    debit_balance: 0u128,
    credit_balance: 0u128,
    account_id: 0,
    checksum: 0x9B83021A,
    _pad: [0u8; 19],
    status: 2,
};
// End checksum note

impl std::fmt::Display for AccountResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.status == 1 {
            return write!(f, "Account not found");
        } else if self.status == 2 {
            return write!(f, "Server account cache full");
        }

        if self.debit_balance > self.credit_balance {
            write!(
                f,
                "[{}] Debit Balance: ${:.2}",
                self.account_id,
                (self.debit_balance - self.credit_balance) as f64 / 1000.0,
            )
        } else {
            write!(
                f,
                "[{}] Credit Balance: ${:.2}",
                self.account_id,
                (self.credit_balance - self.debit_balance) as f64 / 1000.0,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_cast() {
        let mut bytes = [0u8; 64];
        // account_id: 42 as u64, little-endian at offset 0
        bytes[0..8].copy_from_slice(&42u64.to_le_bytes());
        // message_kind = 0 (Account)
        bytes[63] = 0;

        let acct: Account = bytemuck::pod_read_unaligned(&bytes);
        assert_eq!(acct.account_id, 42);
        assert_eq!(acct.message_kind, MessageKind::Account as u8);
    }

    #[test]
    fn test_not_found_checksum() {
        let response: &AccountResponse = bytemuck::from_bytes(bytemuck::bytes_of(&NOT_FOUND));
        assert!(response.verify().is_ok());
    }

    #[test]
    fn test_cache_full_checksum() {
        let response: &AccountResponse = bytemuck::from_bytes(bytemuck::bytes_of(&CACHE_FULL));
        assert!(response.verify().is_ok());
    }
}
