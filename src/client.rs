use rand::prelude::*;
use std::io::{BufRead, BufReader};
use std::thread;
use std::{io::Write, net::TcpStream};
use weevil::GenericError;
use weevil::account::Account;
use weevil::transaction::{Transaction, TransactionKind};

const NUM_THREADS: usize = 8;
const NUM_TRANSACTIONS: usize = 100;

fn client_connection(account_id: u64) -> Result<(), GenericError> {
    let mut conn = TcpStream::connect("127.0.0.1:3333")?;
    let mut reader = BufReader::new(conn.try_clone()?);

    let acct = Account::new(account_id);
    conn.write_all(bytemuck::bytes_of(&acct)).unwrap();
    let mut response = String::new();
    reader.read_line(&mut response)?;
    print!("{response}");

    let mut rng = rand::rng();

    for i in 0..NUM_TRANSACTIONS {
        let kind = if i % 2 == 0 {
            TransactionKind::Deposit
        } else {
            TransactionKind::Withdrawal
        };
        let tx = Transaction::new(rng.random_range(1000u128..=1_000_000), account_id, kind);
        conn.write_all(bytemuck::bytes_of(&tx))?;

        let mut response = String::new();
        reader.read_line(&mut response)?;
        print!("{response}");
    }

    let acct = Account::new(account_id);
    conn.write_all(bytemuck::bytes_of(&acct))?;

    let mut response = String::new();
    reader.read_line(&mut response)?;

    print!("{response}");

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
