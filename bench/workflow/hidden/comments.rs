use std::process::{Command, Output};

fn run(name: &str, ledger: &str) -> Output {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, ledger).unwrap();
    Command::new(env!("CARGO_BIN_EXE_tally-cli")).arg(&path).output().unwrap()
}

#[test]
fn comments_and_blank_lines_are_skipped() {
    let out = run("hidden-skip.ledger", "# week 3\n\nfood 1\n   \n  # indented\n#food 9\nfood 2\n");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "food 3.00\ntotal 3.00\n");
}

#[test]
fn an_error_counts_every_line_of_the_file() {
    let out = run("hidden-count.ledger", "food 1\n\n# note\nfood\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("line 4:"));
}

#[test]
fn a_hash_inside_a_line_is_not_a_comment() {
    assert_eq!(run("hidden-hash.ledger", "food 1#2\n").status.code(), Some(1));
}
