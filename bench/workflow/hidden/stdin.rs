use std::io::Write as _;
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_tally-cli");

fn piped(ledger: &str) -> Output {
    let mut child = Command::new(BIN)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(ledger.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn a_piped_ledger_prints_its_report() {
    let out = piped("food 1\nrent 2.50\nfood 2\n");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "food 3.00\nrent 2.50\ntotal 5.50\n");
}

#[test]
fn a_piped_bad_line_exits_one_and_names_the_line() {
    let out = piped("food 1\nrent\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("line 2:"));
}

#[test]
fn a_file_still_reads() {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hidden-stdin.ledger");
    std::fs::write(&path, "tea 0.5\n").unwrap();
    let out = Command::new(BIN).arg(&path).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "tea 0.50\ntotal 0.50\n");
}

#[test]
fn the_fixture_landed_and_agrees_with_itself() {
    let ledger = include_str!("fixtures/week.ledger");
    assert!(!ledger.trim().is_empty());
    assert_eq!(String::from_utf8_lossy(&piped(ledger).stdout), include_str!("fixtures/week.report"));
}
