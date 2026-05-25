use crate::MAX_ACCOUNTS;
use crate::WeevilError;
use crate::account::{AccountEntry, CheckpointRecord};
use crate::transaction::{Transaction, TransactionKind};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

pub struct AccountEntryCache {
    entries: [Option<AccountEntry>; MAX_ACCOUNTS],
    file_backing: File,
}

impl Default for AccountEntryCache {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountEntryCache {
    pub fn new() -> Self {
        let f = OpenOptions::new()
            .append(true)
            .create(true)
            .read(true)
            .open("./data_files/wal.log")
            .expect("error loading wal file");
        const EMPTY_ACCOUNT_ENTRY: Option<AccountEntry> = None;
        let mut cache = AccountEntryCache {
            entries: [EMPTY_ACCOUNT_ENTRY; MAX_ACCOUNTS],
            file_backing: f,
        };
        cache.replay().expect("transaction replay failed");
        cache
    }

    fn find_free_slot(&self, acct_id: u64) -> Option<usize> {
        let base = (acct_id % MAX_ACCOUNTS as u64) as usize;
        (base..base + MAX_ACCOUNTS)
            .map(|i| i % MAX_ACCOUNTS)
            .find(|&i| self.entries[i].is_none())
    }

    fn get_account_idx(&self, acct_id: u64) -> Option<usize> {
        let base = (acct_id % MAX_ACCOUNTS as u64) as usize;
        (base..base + MAX_ACCOUNTS)
            .map(|i| i % MAX_ACCOUNTS)
            .find(|&i| matches!(&self.entries[i], Some(ae) if ae.account_id == acct_id))
    }

    pub fn get(&self, acct_id: u64) -> Option<&AccountEntry> {
        if let Some(idx) = self.get_account_idx(acct_id) {
            return self.entries[idx].as_ref();
        }
        None
    }

    pub fn get_mut(&mut self, acct_id: u64) -> Option<&mut AccountEntry> {
        if let Some(idx) = self.get_account_idx(acct_id) {
            return self.entries[idx].as_mut();
        }
        None
    }

    pub fn insert(&mut self, acct_entry: AccountEntry) -> Option<&AccountEntry> {
        let id = acct_entry.account_id;
        if let Some(idx) = self.find_free_slot(id) {
            self.entries[idx] = Some(acct_entry)
        }
        self.get(id)
    }

    pub fn has_capacity(&self, acct_id: u64) -> bool {
        self.find_free_slot(acct_id).is_some()
    }

    pub fn checkpoint(&mut self) -> Result<(), WeevilError> {
        let mut temp_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open("./data_files/checkpoint.tmp")?;
        let crs = self.entries.iter().flatten().map(CheckpointRecord::from);
        for cr in crs {
            temp_file.write_all(bytemuck::bytes_of(&cr))?;
        }
        temp_file.sync_data()?;
        std::fs::rename("./data_files/checkpoint.tmp", "./data_files/checkpoint")?;
        self.file_backing.set_len(0)?;
        self.file_backing.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    fn replay(&mut self) -> Result<(), WeevilError> {
        let mut buf = [0u8; 64];

        // check the checkpoint file if it exists
        match OpenOptions::new()
            .read(true)
            .open("./data_files/checkpoint")
        {
            Ok(mut checkpoint_file) => {
                // TODO: I'm probably swallowing errors silently
                while checkpoint_file.read_exact(&mut buf).is_ok() {
                    let cr: CheckpointRecord = bytemuck::pod_read_unaligned(&buf);
                    cr.verify()?;
                    let acct_entry =
                        AccountEntry::new(cr.account_id, cr.debit_balance, cr.credit_balance)?;
                    self.insert(acct_entry).expect("ran out of space");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }

        // replay the wal for transactions that didn't make it yet
        self.file_backing.seek(SeekFrom::Start(0))?;

        // TODO: I'm probably swallowing errors silently
        while self.file_backing.read_exact(&mut buf).is_ok() {
            let tx: Transaction = bytemuck::pod_read_unaligned(&buf);
            tx.verify()?;
            if let Some(entry) = self.get_mut(tx.account_id) {
                entry.apply_transaction(tx)?;
            } else {
                let (db, cb) = match tx.kind()? {
                    TransactionKind::Debit => (tx.amount, 0),
                    TransactionKind::Credit => (0, tx.amount),
                };
                let acct_entry = AccountEntry::new(tx.account_id, db, cb)?;
                self.insert(acct_entry).expect("ran out of space");
            }
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), WeevilError> {
        let mut needs_sync = false;
        self.entries.iter_mut().flatten().try_for_each(|ae| {
            if ae.is_dirty() {
                needs_sync = true;
            }
            ae.write(&mut self.file_backing)
        })?;
        if needs_sync {
            self.file_backing.sync_data()?;
        }
        if self.file_backing.metadata()?.len() > 1_000_000 {
            self.checkpoint()?;
        }
        Ok(())
    }
}
