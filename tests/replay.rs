use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;
use weevil::account::{Account, AccountResponse};
use weevil::transaction::{Transaction, TransactionKind};

const SERVER_ADDR: &str = "127.0.0.1:3333";
const TEST_ACCOUNT: u64 = 99;

fn clean_data_files() {
    let _ = fs::remove_file("./data_files/wal.log");
    let _ = fs::remove_file("./data_files/checkpoint");
    let _ = fs::create_dir_all("./data_files");
}

fn spawn_server() -> Child {
    Command::new("cargo")
        .args(["run", "--bin", "server"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn server")
}

fn wait_for_server() {
    for _ in 0..50 {
        if TcpStream::connect(SERVER_ADDR).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("server did not become ready");
}

fn round_trip(conn: &mut TcpStream, bytes: &[u8]) -> AccountResponse {
    conn.write_all(bytes).unwrap();
    let mut buf = [0u8; 64];
    conn.read_exact(&mut buf).unwrap();
    bytemuck::pod_read_unaligned(&buf)
}

#[test]
fn test_balance_survives_restart() {
    clean_data_files();

    // --- first server run ---
    let mut server = spawn_server();
    wait_for_server();

    let mut conn = TcpStream::connect(SERVER_ADDR).unwrap();
    conn.set_nodelay(true).unwrap();

    // register account
    round_trip(&mut conn, bytemuck::bytes_of(&Account::new(TEST_ACCOUNT)));

    // send known transactions
    round_trip(
        &mut conn,
        bytemuck::bytes_of(&Transaction::new(
            10_000,
            TEST_ACCOUNT,
            TransactionKind::Debit,
        )),
    );
    round_trip(
        &mut conn,
        bytemuck::bytes_of(&Transaction::new(
            3_000,
            TEST_ACCOUNT,
            TransactionKind::Credit,
        )),
    );
    round_trip(
        &mut conn,
        bytemuck::bytes_of(&Transaction::new(
            5_000,
            TEST_ACCOUNT,
            TransactionKind::Debit,
        )),
    );

    // query to get committed final balance
    let before = round_trip(&mut conn, bytemuck::bytes_of(&Account::new(TEST_ACCOUNT)));

    drop(conn);
    server.kill().unwrap();
    server.wait().unwrap();

    // --- restart ---
    let mut server = spawn_server();
    wait_for_server();

    let mut conn = TcpStream::connect(SERVER_ADDR).unwrap();
    conn.set_nodelay(true).unwrap();

    let after = round_trip(&mut conn, bytemuck::bytes_of(&Account::new(TEST_ACCOUNT)));

    drop(conn);
    server.kill().unwrap();
    server.wait().unwrap();

    assert_eq!(before, after);

    clean_data_files();
}
