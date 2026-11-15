//! The built command line run on the fixture ledger, given as a file and piped to it.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_tally-cli");
const LEDGER: &str = include_str!("fixtures/week.ledger");
const REPORT: &str = include_str!("fixtures/week.report");

#[test]
fn a_ledger_file_prints_its_report() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/week.ledger");
    let out = Command::new(BIN).arg(path).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), REPORT);
}

#[test]
fn a_piped_ledger_prints_the_same_report() {
    let mut child = Command::new(BIN)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(LEDGER.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), REPORT);
}
