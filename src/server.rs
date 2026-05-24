use mio::event::Event;
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Registry, Token};
use std::io::{Read, Write};

use weevil::WeevilError;
use weevil::account::{Account, AccountEntry, AccountResponse, CACHE_FULL, NOT_FOUND};
use weevil::transaction::Transaction;

const SERVER: Token = Token(0);

const MAX_CONNECTIONS: usize = 65;
const MAX_ACCOUNTS: usize = 257;

enum ParsedMessage {
    Account(Account),
    Transaction(Transaction),
    Incomplete,
    Closing,
}

enum SessionStatus {
    Reading,
    AwaitingCommit([u8; 64]),
    Writing([u8; 64]),
    Closing,
}

#[repr(C, align(16))]
struct AlignedBuf([u8; 64]);

impl Default for AlignedBuf {
    fn default() -> Self {
        Self([0u8; 64])
    }
}

struct Session {
    stream: TcpStream,
    read_buf: AlignedBuf,
    offset: usize,
    status: SessionStatus,
    token: Token,
}

impl Session {
    fn process_event(
        &mut self,
        event: &Event,
        registry: &Registry,
    ) -> Result<ParsedMessage, WeevilError> {
        match self.status {
            SessionStatus::Reading if event.is_readable() => self.read_message(),
            SessionStatus::Writing(_) if event.is_writable() => self.write_response(registry),
            SessionStatus::Closing => Ok(ParsedMessage::Closing),
            SessionStatus::AwaitingCommit(_) => Ok(ParsedMessage::Incomplete), // nothing to do until we hit the disk
            _ => Ok(ParsedMessage::Incomplete), // false trigger on event polling, just try again
        }
    }

    fn read_message(&mut self) -> Result<ParsedMessage, WeevilError> {
        loop {
            assert!(self.offset < 64);
            match self.stream.read(&mut self.read_buf.0[self.offset..]) {
                Ok(0) => {
                    self.status = SessionStatus::Closing;
                    return Ok(ParsedMessage::Closing);
                }
                Ok(n) => {
                    self.offset += n;
                    if self.offset == 64 {
                        let result = match self.read_buf.0[63] {
                            0 => {
                                let acct: &Account = bytemuck::from_bytes(&self.read_buf.0);
                                ParsedMessage::Account(*acct)
                            }
                            1 => {
                                let tx: &Transaction = bytemuck::from_bytes(&self.read_buf.0);
                                ParsedMessage::Transaction(*tx)
                            }
                            _ => return Err(WeevilError::InvalidMessageKind(self.read_buf.0[63])),
                        };
                        self.offset = 0;
                        return Ok(result);
                    }
                }
                Err(e) if would_block(&e) => return Ok(ParsedMessage::Incomplete),
                Err(e) if interrupted(&e) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                    return Ok(ParsedMessage::Closing);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn write_response(&mut self, registry: &Registry) -> Result<ParsedMessage, WeevilError> {
        let old = std::mem::replace(&mut self.status, SessionStatus::Reading);
        let SessionStatus::Writing(data) = old else {
            panic!("session status must be `SessionStatus::Writing(data)`")
        };
        match self.stream.write_all(&data) {
            Ok(()) => {
                registry.reregister(&mut self.stream, self.token, Interest::READABLE)?;
            }
            Err(e) if would_block(&e) || interrupted(&e) => {
                self.status = SessionStatus::Writing(data);
            }
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                self.status = SessionStatus::Closing;
                return Ok(ParsedMessage::Incomplete);
            }
            Err(e) => return Err(e.into()),
        }
        Ok(ParsedMessage::Incomplete)
    }

    fn stage_response(&mut self, response: [u8; 64]) {
        self.status = SessionStatus::AwaitingCommit(response);
    }
}

fn main() -> Result<(), WeevilError> {
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(128);

    let addr = "127.0.0.1:3333".parse().expect("invalid server address");
    let mut server = TcpListener::bind(addr)?;

    poll.registry()
        .register(&mut server, SERVER, Interest::READABLE)?;

    const EMPTY_SESSION: Option<Session> = None;
    let mut connections = [EMPTY_SESSION; MAX_CONNECTIONS];

    let mut account_entries = AccountEntryCache::new();

    println!("Waiting to receive Weevil messages on {addr}...");

    loop {
        if let Err(err) = poll.poll(&mut events, Some(std::time::Duration::from_micros(500))) {
            if interrupted(&err) {
                continue;
            }
            return Err(err.into());
        }

        for event in &events {
            match event.token() {
                SERVER => accept_connections(&mut server, poll.registry(), &mut connections)?,
                token => {
                    if let Some(Some(session)) = connections.get_mut(token.0) {
                        match session.process_event(event, poll.registry()) {
                            Ok(ParsedMessage::Closing) => {
                                if let Ok(addr) = session.stream.peer_addr() {
                                    println!("[{}] Disconnected", addr);
                                } else {
                                    println!("[UNKNOWN CLIENT] Disconnected");
                                }
                                poll.registry().deregister(&mut session.stream)?;
                                connections[token.0] = None;
                            }
                            Ok(ParsedMessage::Incomplete) => continue,
                            Ok(ParsedMessage::Account(acct)) => {
                                process_account(acct, session, &mut account_entries)?
                            }
                            Ok(ParsedMessage::Transaction(tx)) => {
                                process_transaction(tx, session, &mut account_entries)?
                            }
                            Err(e) => {
                                // log the error
                                eprintln!("ERROR: {e}");
                                // close the session
                                session.status = SessionStatus::Closing;
                            }
                        }
                    } else {
                        eprintln!("Unknown token: {token:?}");
                    }
                }
            }
        }

        // now that we've collected all our inputs, we push them to disk
        for entry in account_entries.iter_mut().flatten() {
            entry.flush()?; // has internal check before the syscall
        }

        // find all the AwaitingCommit sessions
        // now we notify the client that their transaction is durable on disk
        for s in connections
            .iter_mut()
            .flatten()
            .filter(|s| matches!(s.status, SessionStatus::AwaitingCommit(_)))
        {
            let old = std::mem::replace(&mut s.status, SessionStatus::Reading);
            if let SessionStatus::AwaitingCommit(data) = old {
                s.status = SessionStatus::Writing(data);
            } else {
                unreachable!();
            }
            poll.registry()
                .reregister(&mut s.stream, s.token, Interest::WRITABLE)?;
        }
    }
}

fn accept_connections(
    server: &mut TcpListener,
    registry: &Registry,
    connections: &mut [Option<Session>; MAX_CONNECTIONS],
) -> Result<(), WeevilError> {
    loop {
        match server.accept() {
            Ok((mut stream, address)) if let Some(token) = next_token(connections) => {
                println!("[{}] Connection received", address);
                stream.set_nodelay(true)?;
                registry.register(&mut stream, token, Interest::READABLE)?;
                connections[token.0] = Some(Session {
                    stream,
                    read_buf: AlignedBuf::default(),
                    offset: 0,
                    status: SessionStatus::Reading,
                    token,
                });
            }
            Ok((_, _)) => {
                eprintln!("Connections pool full, dropping...");
            }
            Err(e) if would_block(&e) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn process_account(
    acct: Account,
    session: &mut Session,
    account_entries: &mut AccountEntryCache,
) -> Result<(), WeevilError> {
    if let Some(a) = account_entries.get(acct.account_id) {
        println!("{a}");
        // our write_buf is staged, now we wait until fsync is complete before sending
        session.stage_response(cast_response(a.response()));
    } else if !account_entries.has_capacity(acct.account_id) {
        println!("account cache full");
        session.stage_response(cast_response(CACHE_FULL));
    } else {
        let entry = AccountEntry::new(acct.account_id)?;
        println!("Registering account: {entry}...");
        // guaranteed to succeed — slot was confirmed above
        let entry = account_entries
            .insert(entry)
            .expect("slot vanished after has_capacity");
        println!("Success: {entry}");
        session.stage_response(cast_response(entry.response()));
    }
    Ok(())
}

fn process_transaction(
    tx: Transaction,
    session: &mut Session,
    account_entries: &mut AccountEntryCache,
) -> Result<(), WeevilError> {
    if let Some(a) = account_entries.get_mut(tx.account_id) {
        println!("Pushing transaction to {a}...");
        a.add_transaction(tx)?;
        // our write_buf is staged, now we wait until fsync is complete before sending
        session.stage_response(cast_response(a.response()));
    } else {
        eprintln!("Account [{}] not found...", tx.account_id);
        session.stage_response(cast_response(NOT_FOUND));
    }
    Ok(())
}

fn next_token(connections: &[Option<Session>; MAX_CONNECTIONS]) -> Option<Token> {
    // reserve slot 0 for the SERVER connection
    let (next, _) = connections
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, s)| s.is_none())?;
    Some(Token(next))
}

fn would_block(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::WouldBlock
}

fn interrupted(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::Interrupted
}

fn cast_response(r: AccountResponse) -> [u8; 64] {
    bytemuck::cast::<AccountResponse, [u8; 64]>(r)
}
struct AccountEntryCache {
    entries: [Option<AccountEntry>; MAX_ACCOUNTS],
}

impl AccountEntryCache {
    pub fn new() -> Self {
        const EMPTY_ACCOUNT_ENTRY: Option<AccountEntry> = None;
        AccountEntryCache {
            entries: [EMPTY_ACCOUNT_ENTRY; MAX_ACCOUNTS],
        }
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

    fn get(&self, acct_id: u64) -> Option<&AccountEntry> {
        if let Some(idx) = self.get_account_idx(acct_id) {
            return self.entries[idx].as_ref();
        }
        None
    }

    fn get_mut(&mut self, acct_id: u64) -> Option<&mut AccountEntry> {
        if let Some(idx) = self.get_account_idx(acct_id) {
            return self.entries[idx].as_mut();
        }
        None
    }

    fn insert(&mut self, acct_entry: AccountEntry) -> Option<&AccountEntry> {
        let id = acct_entry.account_id;
        if let Some(idx) = self.find_free_slot(id) {
            self.entries[idx] = Some(acct_entry)
        }
        self.get(id)
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Option<AccountEntry>> {
        self.entries.iter_mut()
    }

    pub fn has_capacity(&self, acct_id: u64) -> bool {
        self.find_free_slot(acct_id).is_some()
    }
}
