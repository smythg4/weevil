use bytemuck::{Pod, Zeroable};

use crate::WeevilError;
use crate::transfer::Transfer;
use crate::{MessageKind, crc32, crc32_chained};

const _: () = assert!(std::mem::size_of::<Account>() == 64);
const _: () = assert!(std::mem::offset_of!(Account, checksum) == 8);
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
        let bytes = bytemuck::bytes_of(self);
        let checksum = crc32_chained(&[&bytes[..8], &[0u8; 4], &bytes[12..]]);
        if checksum == self.checksum {
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

pub struct AccountEntry {
    pub account_id: u64,
    pub debit_balance: u128,
    pub credit_balance: u128,
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
    pub fn new(account_id: u64, debit_balance: u128, credit_balance: u128) -> Self {
        AccountEntry {
            account_id,
            debit_balance,
            credit_balance,
        }
    }

    pub fn apply_transaction(&mut self, tx: &Transfer) -> Result<(), WeevilError> {
        if self.account_id == tx.debit_account_id {
            self.debit_balance += tx.amount;
        } else if self.account_id == tx.credit_account_id {
            self.credit_balance += tx.amount;
        } else {
            return Err(WeevilError::InvalidAccountId(
                tx.debit_account_id,
                tx.credit_account_id,
            ));
        }
        Ok(())
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
const _: () = assert!(std::mem::offset_of!(AccountResponse, checksum) == 40);
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
        let bytes = bytemuck::bytes_of(self);
        let checksum = crc32_chained(&[&bytes[..40], &[0u8; 4], &bytes[44..]]);
        if checksum == self.checksum {
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

const _: () = assert!(std::mem::size_of::<CheckpointRecord>() == 64);
const _: () = assert!(std::mem::offset_of!(CheckpointRecord, checksum) == 40);
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct CheckpointRecord {
    pub debit_balance: u128,
    pub credit_balance: u128,
    pub account_id: u64,
    pub checksum: u32,
    _pad: [u8; 20],
}

impl CheckpointRecord {
    pub fn verify(&self) -> Result<(), WeevilError> {
        let bytes = bytemuck::bytes_of(self);
        let checksum = crc32_chained(&[&bytes[..40], &[0u8; 4], &bytes[44..]]);
        if checksum == self.checksum {
            return Ok(());
        }
        Err(WeevilError::ChecksumFailed)
    }
}

impl From<&AccountEntry> for CheckpointRecord {
    fn from(acct_entry: &AccountEntry) -> Self {
        let mut cr = CheckpointRecord {
            debit_balance: acct_entry.debit_balance,
            credit_balance: acct_entry.credit_balance,
            account_id: acct_entry.account_id,
            checksum: 0,
            _pad: [0u8; 20],
        };
        let checksum = crc32(bytemuck::bytes_of(&cr));
        cr.checksum = checksum;
        cr
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
