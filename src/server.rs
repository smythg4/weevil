use mio::event::Event;
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Registry, Token};
use std::io::{Read, Write};

use weevil::GenericError;
use weevil::account::{Account, AccountEntry, AccountResponse, NOT_FOUND};
use weevil::transaction::Transaction;

const SERVER: Token = Token(0);

const MAX_CONNECTIONS: usize = 32;
const MAX_ACCOUNTS: usize = 1024;

enum ParsedMessage {
    Account(Account),
    Transaction(Transaction),
    Incomplete,
    Closing,
}

enum SessionStatus {
    Reading,
    AwaitingCommit([u8; 32]),
    Writing([u8; 32]),
    Closing,
}

#[derive(Default)]
#[repr(C, align(16))]
struct AlignedBuf([u8; 32]);

struct Session {
    stream: TcpStream,
    read_buf: AlignedBuf,
    offset: usize,
    status: SessionStatus,
    token: Token,
}

impl Session {
    fn handle(
        &mut self,
        event: &Event,
        registry: &Registry,
    ) -> Result<ParsedMessage, GenericError> {
        match self.status {
            SessionStatus::Reading if event.is_readable() => self.handle_read(),
            SessionStatus::Writing(_) if event.is_writable() => self.handle_write(registry),
            SessionStatus::Closing => Ok(ParsedMessage::Closing),
            SessionStatus::AwaitingCommit(_) => Ok(ParsedMessage::Incomplete), // nothing to do until we hit the disk
            _ => Ok(ParsedMessage::Incomplete), // false trigger on event polling, just try again
        }
    }

    fn handle_read(&mut self) -> Result<ParsedMessage, GenericError> {
        loop {
            match self.stream.read(&mut self.read_buf.0) {
                Ok(0) => {
                    self.status = SessionStatus::Closing;
                    return Ok(ParsedMessage::Closing);
                }
                Ok(n) => {
                    self.offset += n;
                    if self.offset == 32 {
                        let result = match self.read_buf.0[31] {
                            0 => {
                                let acct: &Account = bytemuck::from_bytes(&self.read_buf.0);
                                ParsedMessage::Account(*acct)
                            }
                            1 => {
                                let tx: &Transaction = bytemuck::from_bytes(&self.read_buf.0);
                                ParsedMessage::Transaction(*tx)
                            }
                            _ => return Err(String::from("invalid message type byte").into()),
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

    fn handle_write(&mut self, registry: &Registry) -> Result<ParsedMessage, GenericError> {
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

    fn stage_response(&mut self, response: [u8; 32]) {
        self.status = SessionStatus::AwaitingCommit(response);
    }
}

fn main() -> Result<(), GenericError> {
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(128);

    let addr = "127.0.0.1:3333".parse()?;
    let mut server = TcpListener::bind(addr)?;

    poll.registry()
        .register(&mut server, SERVER, Interest::READABLE)?;

    const EMPTY_SESSION: Option<Session> = None;
    let mut connections = [EMPTY_SESSION; MAX_CONNECTIONS];

    const EMPTY_ACCOUNT_ENTRY: Option<AccountEntry> = None;
    let mut account_entries: [Option<AccountEntry>; MAX_ACCOUNTS] =
        [EMPTY_ACCOUNT_ENTRY; MAX_ACCOUNTS];

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
                        match session.handle(event, poll.registry()) {
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
                                let idx = get_account_idx(acct.account_id);
                                if let Some(Some(a)) = account_entries.get(idx) {
                                    println!("{a}");
                                    // our write_buf is staged, now we wait until fsync is complete before sending
                                    session.stage_response(bytemuck::cast::<
                                        AccountResponse,
                                        [u8; 32],
                                    >(
                                        a.response()
                                    ));
                                } else {
                                    let entry = AccountEntry::new(acct.account_id)?;
                                    // our write_buf is staged, now we wait until fsync is complete before sending
                                    session.stage_response(bytemuck::cast::<
                                        AccountResponse,
                                        [u8; 32],
                                    >(
                                        entry.response()
                                    ));
                                    println!("Registering account: {entry}...");
                                    let idx = get_account_idx(acct.account_id);
                                    account_entries[idx] = Some(entry);
                                }
                            }
                            Ok(ParsedMessage::Transaction(tx)) => {
                                let idx = get_account_idx(tx.account_id);
                                if let Some(Some(a)) = account_entries.get_mut(idx) {
                                    println!("Pushing transaction to {a}...");
                                    a.add_transaction(tx)?;
                                    // our write_buf is staged, now we wait until fsync is complete before sending
                                    session.stage_response(bytemuck::cast::<
                                        AccountResponse,
                                        [u8; 32],
                                    >(
                                        a.response()
                                    ));
                                } else {
                                    eprintln!("Account [{}] not found...", tx.account_id);
                                    session.stage_response(bytemuck::cast::<
                                        AccountResponse,
                                        [u8; 32],
                                    >(
                                        NOT_FOUND
                                    ));
                                }
                            }
                            Err(e) => {
                                // log the error
                                eprintln!("ERROR: {e}");
                                // close the session
                                session.status = SessionStatus::Closing;
                            }
                        }
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
) -> Result<(), GenericError> {
    loop {
        match server.accept() {
            Ok((mut stream, address)) if let Some(token) = next_token(connections) => {
                println!("[{}] Connection received", address);
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

fn get_account_idx(acct_id: u64) -> usize {
    // TODO: this is ripe for collisions and will need to be addressed with
    // linear probing
    (acct_id % MAX_ACCOUNTS as u64) as usize
}
