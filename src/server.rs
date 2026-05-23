use mio::event::Event;
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Registry, Token};
use std::collections::HashMap;
use std::io::{Read, Write};

use weevil::GenericError;
use weevil::account::{Account, AccountEntry};
use weevil::transaction::Transaction;

const SERVER: Token = Token(0);

enum ParsedMessage {
    Account(Account),
    Transaction(Transaction),
    Incomplete,
    Closing,
}

// TODO: Consider storing the pending write buffer in `AwaitingCommit` and `Writing`
#[derive(Eq, PartialEq)]
enum SessionStatus {
    Reading,
    AwaitingCommit,
    Writing,
    Closing,
}

#[repr(C, align(16))]
struct AlignedBuf([u8; 32]);

struct Session {
    stream: TcpStream,
    read_buf: AlignedBuf,
    bytes_read: usize,
    write_buf: Option<Vec<u8>>,
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
            SessionStatus::Writing if event.is_writable() => self.handle_write(registry),
            SessionStatus::Closing => Ok(ParsedMessage::Closing),
            SessionStatus::AwaitingCommit => Ok(ParsedMessage::Incomplete), // nothing to do until we hit the disk
            _ => Ok(ParsedMessage::Incomplete), // false trigger on event polling, just try again
        }
    }

    fn handle_read(&mut self) -> Result<ParsedMessage, GenericError> {
        let mut result = ParsedMessage::Incomplete;
        loop {
            match self.stream.read(&mut self.read_buf.0) {
                Ok(0) => {
                    self.status = SessionStatus::Closing;
                    return Ok(ParsedMessage::Closing);
                }
                Ok(n) => {
                    self.bytes_read += n;
                    if self.bytes_read == 32 {
                        result = match self.read_buf.0[31] {
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
                        self.bytes_read = 0;
                        return Ok(result);
                    }
                }
                Err(e) if would_block(&e) => return Ok(result),
                Err(e) if interrupted(&e) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return Ok(result),
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn handle_write(&mut self, registry: &Registry) -> Result<ParsedMessage, GenericError> {
        let data = self
            .write_buf
            .take()
            .expect("write_buf must be Some in Writing state");
        match self.stream.write_all(&data) {
            Ok(()) => {
                self.status = SessionStatus::Reading;
                registry.reregister(&mut self.stream, self.token, Interest::READABLE)?;
            }
            Err(e) if would_block(&e) || interrupted(&e) => self.write_buf = Some(data),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                self.status = SessionStatus::Closing;
                return Ok(ParsedMessage::Incomplete);
            }
            Err(e) => return Err(e.into()),
        }
        Ok(ParsedMessage::Incomplete)
    }
}

fn main() -> Result<(), GenericError> {
    let mut poll = Poll::new()?;
    let mut events = Events::with_capacity(128);

    let addr = "127.0.0.1:3333".parse()?;
    let mut server = TcpListener::bind(addr)?;

    poll.registry()
        .register(&mut server, SERVER, Interest::READABLE)?;

    // TODO: Replace HashMap with [Option<Session>; MAX_CONNECTIONS] and use the token value as a
    // direct array index
    let mut connections = HashMap::new();
    let mut unique_token = Token(SERVER.0 + 1);

    // TODO: Replace HashMap with [Option<AccountEntry>; MAX_ACCOUNTS] and use a hash to establish
    // direct array index. Probably needs cache eviction policy so this might be heavy.
    let mut account_entries = HashMap::new();

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
                SERVER => accept_connections(
                    &mut server,
                    poll.registry(),
                    &mut connections,
                    &mut unique_token,
                )?,
                token => {
                    if let Some(session) = connections.get_mut(&token) {
                        match session.handle(event, poll.registry()) {
                            Ok(ParsedMessage::Closing) => {
                                if let Ok(addr) = session.stream.peer_addr() {
                                    println!("[{}] Disconnected", addr);
                                } else {
                                    println!("[UNKNOWN CLIENT] Disconnected");
                                }
                                poll.registry().deregister(&mut session.stream)?;
                                connections.remove(&token);
                            }
                            Ok(ParsedMessage::Incomplete) => continue,
                            Ok(ParsedMessage::Account(acct)) => {
                                if let Some(a) = account_entries.get(&acct.account_id) {
                                    // TODO: Establish fixed sized response format
                                    let response = format!("[SERVER] {a}\n");
                                    print!("{response}");
                                    session.write_buf = Some(response.into_bytes());
                                } else {
                                    let entry = AccountEntry::new(acct.account_id)?;
                                    let response =
                                        format!("[SERVER] Registering account: {entry}...\n");
                                    account_entries.insert(acct.account_id, entry);
                                    print!("{response}");
                                    session.write_buf = Some(response.into_bytes())
                                }
                                // our write_buf is staged, now we wait until fsync is complete before sending
                                session.status = SessionStatus::AwaitingCommit;
                            }
                            Ok(ParsedMessage::Transaction(tx)) => {
                                let id = tx.account_id;
                                if let Some(a) = account_entries.get_mut(&id) {
                                    let response =
                                        format!("[SERVER] Pushing transaction to {a}...\n");
                                    print!("{response}");
                                    session.write_buf = Some(response.into_bytes());
                                    a.add_transaction(tx);
                                } else {
                                    let response = format!("Account [{id}] not found...\n");
                                    print!("{response}");
                                    session.write_buf = Some(response.into_bytes());
                                }
                                // our write_buf is staged, now we wait until fsync is complete before sending
                                session.status = SessionStatus::AwaitingCommit;
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

        for entry in account_entries.values_mut() {
            entry.write()?;
            entry.sync()?;
        }

        // find all the AwaitingCommit sessions
        for (t, s) in connections
            .iter_mut()
            .filter(|(_, s)| s.status == SessionStatus::AwaitingCommit)
        {
            s.status = SessionStatus::Writing;
            poll.registry()
                .reregister(&mut s.stream, *t, Interest::WRITABLE)?;
        }
    }
}

fn accept_connections(
    server: &mut TcpListener,
    registry: &Registry,
    connections: &mut HashMap<Token, Session>,
    unique_token: &mut Token,
) -> Result<(), GenericError> {
    loop {
        match server.accept() {
            Ok((mut stream, address)) => {
                println!("[{}] Connection received", address);
                let token = next_token(unique_token);
                registry.register(&mut stream, token, Interest::READABLE)?;
                connections.insert(
                    token,
                    Session {
                        stream,
                        read_buf: AlignedBuf([0u8; 32]),
                        bytes_read: 0,
                        write_buf: None,
                        status: SessionStatus::Reading,
                        token,
                    },
                );
            }
            Err(e) if would_block(&e) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn next_token(current: &mut Token) -> Token {
    let next = current.0;
    current.0 += 1;
    Token(next)
}

fn would_block(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::WouldBlock
}

fn interrupted(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::Interrupted
}
