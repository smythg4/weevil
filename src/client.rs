use rand::prelude::*;
use std::io::Read;
use std::thread;
use std::{io::Write, net::TcpStream};
use weevil::WeevilError;
use weevil::account::{Account, AccountResponse};
use weevil::transaction::{Transaction, TransactionKind};

const NUM_THREADS: usize = 10;
const NUM_TRANSACTIONS: usize = 1000;

fn handle_round_trip(conn: &mut TcpStream, outbound: &[u8]) -> Result<(), WeevilError> {
    conn.write_all(outbound)?;
    let mut buffer = [0u8; 64];
    conn.read_exact(&mut buffer)?;
    let response: &AccountResponse = bytemuck::from_bytes(&buffer);
    response.verify()?;
    println!("[SERVER] {response}");
    Ok(())
}

fn client_connection(account_id: u64) -> Result<(), WeevilError> {
    let mut conn = TcpStream::connect("127.0.0.1:3333")?;
    conn.set_nodelay(true)?;

    let acct = Account::new(account_id);
    println!("[CLIENT] {acct}");
    handle_round_trip(&mut conn, bytemuck::bytes_of(&acct))?;

    let mut rng = rand::rng();

    for i in 0..NUM_TRANSACTIONS {
        let kind = if i % 2 == 0 {
            TransactionKind::Debit
        } else {
            TransactionKind::Credit
        };
        let tx = Transaction::new(rng.random_range(1000u128..=1_000_000), account_id, kind);
        println!("[CLIENT] {tx}");
        handle_round_trip(&mut conn, bytemuck::bytes_of(&tx))?;
    }

    let acct = Account::new(account_id);
    println!("[CLIENT] {acct}");
    handle_round_trip(&mut conn, bytemuck::bytes_of(&acct))?;

    Ok(())
}

fn main() {
    let mut handles = Vec::new();

    for i in 0..NUM_THREADS {
        let handle = thread::spawn(move || {
            if let Err(e) = client_connection(i as u64) {
                eprintln!("Thread {i} error: {e}");
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.join();
    }
}
