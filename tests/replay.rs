use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use weevil::account::{Account, AccountResponse, NOT_FOUND};
use weevil::transfer::Transfer;

const SERVER_ADDR: &str = "127.0.0.1:3333";
const TEST_ACCOUNT1: u64 = 99;
const TEST_ACCOUNT2: u64 = 42;

// use to serialize tests that are affected by multithreaded test harness
static LOCK: Mutex<()> = Mutex::new(());

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
    let _guard = LOCK.lock().unwrap();

    clean_data_files();

    // --- first server run ---
    let mut server = spawn_server();
    wait_for_server();

    let mut conn = TcpStream::connect(SERVER_ADDR).unwrap();
    conn.set_nodelay(true).unwrap();

    // register accounts
    round_trip(&mut conn, bytemuck::bytes_of(&Account::new(TEST_ACCOUNT1)));
    round_trip(&mut conn, bytemuck::bytes_of(&Account::new(TEST_ACCOUNT2)));

    // send known transfers
    round_trip(
        &mut conn,
        bytemuck::bytes_of(&Transfer::new(10_000, TEST_ACCOUNT1, TEST_ACCOUNT2).unwrap()),
    );
    round_trip(
        &mut conn,
        bytemuck::bytes_of(&Transfer::new(3_000, TEST_ACCOUNT2, TEST_ACCOUNT1).unwrap()),
    );
    round_trip(
        &mut conn,
        bytemuck::bytes_of(&Transfer::new(5_000, TEST_ACCOUNT1, TEST_ACCOUNT2).unwrap()),
    );

    // query to get committed final balance
    let before1 = round_trip(&mut conn, bytemuck::bytes_of(&Account::new(TEST_ACCOUNT1)));
    let before2 = round_trip(&mut conn, bytemuck::bytes_of(&Account::new(TEST_ACCOUNT2)));

    drop(conn);
    server.kill().unwrap();
    server.wait().unwrap();

    // --- restart ---
    let mut server = spawn_server();
    wait_for_server();

    let mut conn = TcpStream::connect(SERVER_ADDR).unwrap();
    conn.set_nodelay(true).unwrap();

    let after1 = round_trip(&mut conn, bytemuck::bytes_of(&Account::new(TEST_ACCOUNT1)));
    let after2 = round_trip(&mut conn, bytemuck::bytes_of(&Account::new(TEST_ACCOUNT2)));

    drop(conn);
    server.kill().unwrap();
    server.wait().unwrap();

    assert_eq!(before1, after1);
    assert_eq!(before2, after2);

    clean_data_files();
}

#[test]
fn test_duplicate_account_registration_returns_account_balance() {
    let _guard = LOCK.lock().unwrap();

    clean_data_files();
    // --- start the server ---
    let mut server = spawn_server();
    wait_for_server();

    let mut conn = TcpStream::connect(SERVER_ADDR).unwrap();
    conn.set_nodelay(true).unwrap();

    // register accounts
    let account1 = Account::new(TEST_ACCOUNT1);
    let account2 = Account::new(TEST_ACCOUNT2);
    let response1 = round_trip(&mut conn, bytemuck::bytes_of(&account1));
    let _ = round_trip(&mut conn, bytemuck::bytes_of(&account2));

    assert_eq!((0, 0), (response1.debit_balance, response1.credit_balance));

    // send a transaction
    let tx = Transfer::new(1000, TEST_ACCOUNT1, TEST_ACCOUNT2).unwrap();
    let _ = round_trip(&mut conn, bytemuck::bytes_of(&tx));

    // register account1 again
    let response2 = round_trip(&mut conn, bytemuck::bytes_of(&account1));

    assert_eq!(
        (1000, 0),
        (response2.debit_balance, response2.credit_balance)
    );
    // shut down server
    drop(conn);
    server.kill().unwrap();
    server.wait().unwrap();

    clean_data_files();
}

#[test]
fn test_not_found_on_unregistered_account() {
    let _guard = LOCK.lock().unwrap();

    clean_data_files();
    // --- start the server ---
    let mut server = spawn_server();
    wait_for_server();

    let mut conn = TcpStream::connect(SERVER_ADDR).unwrap();
    conn.set_nodelay(true).unwrap();

    // make transfer request for unregistered accounts
    let tx = Transfer::new(1000, TEST_ACCOUNT1, TEST_ACCOUNT2).unwrap();
    let response = round_trip(&mut conn, bytemuck::bytes_of(&tx));

    assert_eq!(response, NOT_FOUND);

    // shut down server
    drop(conn);
    server.kill().unwrap();
    server.wait().unwrap();

    clean_data_files();
}
