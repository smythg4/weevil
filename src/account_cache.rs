use crate::account::{AccountEntry, CheckpointRecord};
use crate::transfer::Transfer;
use crate::{CHECKPOINT_PATH, TEMP_CHECKPOINT_PATH, WAL_PATH, WeevilError};
use crate::{MAX_ACCOUNTS, MAX_BATCH, MAX_WAL_SIZE};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

pub struct AccountEntryCache {
    entries: [Option<AccountEntry>; MAX_ACCOUNTS],
    pending_transactions: [Transfer; MAX_BATCH],
    pt_len: usize,
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
            .open(WAL_PATH)
            .expect("error loading wal file");
        const EMPTY_ACCOUNT_ENTRY: Option<AccountEntry> = None;
        let mut cache = AccountEntryCache {
            entries: [EMPTY_ACCOUNT_ENTRY; MAX_ACCOUNTS],
            pending_transactions: [Transfer::default(); MAX_BATCH],
            pt_len: 0,
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
            assert!(self.entries[idx].is_none());
            self.entries[idx] = Some(acct_entry)
        }
        self.get(id)
    }

    pub fn has_capacity(&self, acct_id: u64) -> bool {
        self.find_free_slot(acct_id).is_some()
    }

    pub fn add_transaction(&mut self, tx: Transfer) -> Result<(), WeevilError> {
        if self.pt_len >= MAX_BATCH {
            return Err(WeevilError::PendingTransactionsFull);
        }
        self.pending_transactions[self.pt_len] = tx;
        self.pt_len += 1;
        Ok(())
    }

    pub fn checkpoint(&mut self) -> Result<(), WeevilError> {
        let mut temp_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(TEMP_CHECKPOINT_PATH)?;
        let (debit_sum, credit_sum) = self
            .entries
            .iter()
            .flatten()
            .fold((0u128, 0u128), |(d, c), ae| {
                (d + ae.debit_balance, c + ae.credit_balance)
            });
        assert_eq!(credit_sum, debit_sum);
        let crs = self.entries.iter().flatten().map(CheckpointRecord::from);
        for cr in crs {
            temp_file.write_all(bytemuck::bytes_of(&cr))?;
        }
        temp_file.sync_data()?;
        std::fs::rename(TEMP_CHECKPOINT_PATH, CHECKPOINT_PATH)?;
        self.file_backing.set_len(0)?;
        self.file_backing.seek(SeekFrom::Start(0))?;
        assert_eq!(self.file_backing.metadata()?.len(), 0);
        Ok(())
    }

    fn replay(&mut self) -> Result<(), WeevilError> {
        let mut buf = [0u8; 64];

        // check the checkpoint file if it exists
        match OpenOptions::new().read(true).open(CHECKPOINT_PATH) {
            Ok(mut checkpoint_file) => {
                // TODO: I'm probably swallowing errors silently
                while checkpoint_file.read_exact(&mut buf).is_ok() {
                    let cr: CheckpointRecord = bytemuck::pod_read_unaligned(&buf);
                    cr.verify()?;
                    let acct_entry =
                        AccountEntry::new(cr.account_id, cr.debit_balance, cr.credit_balance);
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
            let tx: Transfer = bytemuck::pod_read_unaligned(&buf);
            tx.verify()?;
            // update debit balances
            if let Some(entry) = self.get_mut(tx.debit_account_id) {
                entry.apply_transaction(&tx);
            } else {
                let acct_entry = AccountEntry::new(tx.debit_account_id, tx.amount, 0);
                self.insert(acct_entry).expect("ran out of space");
            }
            // update credit balances
            if let Some(entry) = self.get_mut(tx.credit_account_id) {
                entry.apply_transaction(&tx);
            } else {
                let acct_entry = AccountEntry::new(tx.credit_account_id, 0, tx.amount);
                self.insert(acct_entry).expect("ran out of space");
            }
        }
        let (debit_sum, credit_sum) = self
            .entries
            .iter()
            .flatten()
            .fold((0u128, 0u128), |(d, c), ae| {
                (d + ae.debit_balance, c + ae.credit_balance)
            });
        assert_eq!(credit_sum, debit_sum);
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), WeevilError> {
        if self.pt_len == 0 {
            return Ok(());
        }
        println!("Flushing {} transfers...", self.pt_len);
        self.file_backing.write_all(bytemuck::cast_slice(
            &self.pending_transactions[0..self.pt_len],
        ))?;
        self.file_backing.sync_data()?;
        for tx in &self.pending_transactions[0..self.pt_len] {
            let debit_idx = self.get_account_idx(tx.debit_account_id);
            let credit_idx = self.get_account_idx(tx.credit_account_id);
            if let Some(idx) = debit_idx
                && let Some(ae) = self.entries[idx].as_mut()
            {
                ae.apply_transaction(tx);
            }
            if let Some(idx) = credit_idx
                && let Some(ae) = self.entries[idx].as_mut()
            {
                ae.apply_transaction(tx);
            }
        }
        self.pt_len = 0;
        if self.file_backing.metadata()?.len() > MAX_WAL_SIZE {
            self.checkpoint()?;
        }
        Ok(())
    }
}
