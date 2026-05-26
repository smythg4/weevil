use rand::prelude::*;
use std::io::Read;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::Relaxed;
use std::thread;
use std::{io::Write, net::TcpStream};
use weevil::WeevilError;
use weevil::account::{Account, AccountResponse};
use weevil::transfer::Transfer;

// const NUM_THREADS: usize = 250;
// const NUM_TRANSACTIONS: usize = 2000;
static TX_COUNT: AtomicUsize = AtomicUsize::new(0);

fn handle_round_trip(conn: &mut TcpStream, outbound: &[u8]) -> Result<(), WeevilError> {
    conn.write_all(outbound)?;
    let mut buffer = [0u8; 64];
    conn.read_exact(&mut buffer)?;
    let response: &AccountResponse = bytemuck::from_bytes(&buffer);
    response.verify()?;
    //println!("[SERVER] {response}");
    Ok(())
}

fn client_connection(account_id: u64, num_transactions: usize) -> Result<(), WeevilError> {
    let mut conn = TcpStream::connect("127.0.0.1:3333")?;
    conn.set_nodelay(true)?;

    let acct = Account::new(account_id);
    //println!("[CLIENT] {acct}");
    handle_round_trip(&mut conn, bytemuck::bytes_of(&acct))?;
    let acct = Account::new((account_id + 1) % (num_transactions as u64));
    //println!("[CLIENT] {acct}");
    handle_round_trip(&mut conn, bytemuck::bytes_of(&acct))?;

    let mut rng = rand::rng();

    for _ in 0..num_transactions {
        let amt = rng.random_range(1000u128..=1_000_000);
        let tx = Transfer::new(
            amt,
            account_id,
            (account_id + 1) % (num_transactions as u64),
        );
        //println!("[CLIENT] {tx}");
        handle_round_trip(&mut conn, bytemuck::bytes_of(&tx))?;
        TX_COUNT.fetch_add(1, Relaxed);
    }

    let acct = Account::new(account_id);
    //println!("[CLIENT] {acct}");
    handle_round_trip(&mut conn, bytemuck::bytes_of(&acct))?;

    Ok(())
}

fn main() {
    let now = std::time::Instant::now();
    let mut handles = Vec::new();

    let args: Vec<String> = std::env::args().collect();
    let num_threads: usize = args[1].parse().unwrap();
    let num_transactions: usize = args[2].parse().unwrap();

    for i in 0..num_threads {
        let handle = thread::spawn(move || {
            if let Err(e) = client_connection(i as u64, num_transactions) {
                eprintln!("Thread {i} error: {e}");
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.join();
    }
    let elapsed = now.elapsed();
    let total_transactions = TX_COUNT.load(Relaxed);
    let tps = total_transactions as f64 / elapsed.as_secs_f64();
    println!(
        "Threads: {}. Transactions per thread: {}",
        num_threads, num_transactions
    );
    println!(
        "Transactions Processed: {} / {}",
        total_transactions,
        num_threads * num_transactions
    );
    println!("Total time: {:?}", elapsed);
    println!("TPS: {}", tps)
}
