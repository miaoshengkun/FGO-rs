use fgo_rs::{adb::Adb, control::Control};
use std::{
    io::Write,
    time::{Duration, Instant},
};

#[test]
fn adb_drains_large_output_without_pipe_deadlock() {
    let adb = Adb {
        executable: std::env::current_exe().unwrap(),
        timeout: Duration::from_secs(10),
    };
    let bytes = adb
        .command(
            None,
            &["--exact", "child_large_output", "--ignored", "--nocapture"],
            &Control::new(Duration::from_secs(15)),
        )
        .unwrap();
    assert!(bytes.len() > 256 * 1024);
}

#[test]
fn adb_timeout_kills_and_reaps_the_child() {
    let adb = Adb {
        executable: std::env::current_exe().unwrap(),
        timeout: Duration::from_millis(200),
    };
    let begin = Instant::now();
    let error = adb
        .command(
            None,
            &["--exact", "child_hangs", "--ignored", "--nocapture"],
            &Control::new(Duration::from_secs(10)),
        )
        .unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error}");
    assert!(begin.elapsed() < Duration::from_secs(3));
}

// Executed only in child processes by the tests above.
#[test]
#[ignore = "subprocess fixture"]
fn child_large_output() {
    std::io::stdout()
        .write_all(&vec![b'x'; 300 * 1024])
        .unwrap();
    std::io::stderr()
        .write_all(&vec![b'y'; 300 * 1024])
        .unwrap();
}

#[test]
#[ignore = "subprocess fixture"]
fn child_hangs() {
    std::thread::sleep(Duration::from_secs(30));
}
