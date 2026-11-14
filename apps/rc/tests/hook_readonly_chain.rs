//! The hook judges each part of a chain joined by `;`, `|`, `||` or `2>&1` on its own: a chain of
//! the read-only commands inside the workspace passes, and a chain with a refused part is refused
//! naming that part.

use std::path::Path;

use ra_gotchas::shape::{self, Shell};
use ra_gotchas::{Answer, Catalogue, Conditions};

/// The hook's answer to a Bash command run from `workspace`: the catalogue and the shape table
/// weighed together, as `rc hook` weighs them.
fn answer(workspace: &Path, command: &str) -> Answer {
    let shape = shape::judge(command, Shell::Bash, workspace, &|_| false);
    Answer::of(Catalogue::seed().command(command, Conditions::default()), shape)
}

#[test]
fn a_chain_of_read_only_commands_passes_the_hook() {
    let workspace = tempfile::tempdir().unwrap();
    for command in [
        "cat Cargo.toml | head -n 20; sed -n '1,20p' src/main.rs",
        "sed -n 1,5p Cargo.toml | grep name || echo none",
        "grep -n fn src/lib.rs 2>&1 | sort | uniq | tr a-z A-Z",
        "ls crates; rustfmt --check src/main.rs 2>&1 | tail -n 20",
        "rg -n judge crates | head -n 40",
        "echo start; wc -l src/main.rs | sort -n",
    ] {
        let answer = answer(workspace.path(), command);
        assert!(answer.allows(), "`{command}`: {answer:?}");
    }
}

#[test]
fn a_sed_that_only_prints_passes_and_one_that_writes_is_refused() {
    let workspace = tempfile::tempdir().unwrap();
    for command in ["sed -n '/fn main/,/^}/p' src/main.rs", "cargo metadata 2>&1 | sed -n 1,40p"] {
        let answer = answer(workspace.path(), command);
        assert!(answer.allows(), "`{command}`: {answer:?}");
    }
    for command in ["sed -i 's/a/b/' src/main.rs", "ls; sed -n '1w out.txt' src/main.rs", "cat x | sed -n -i 1p src/main.rs"] {
        let answer = answer(workspace.path(), command);
        assert!(matches!(answer, Answer::Deny { .. }), "`{command}`: {answer:?}");
    }
}

#[test]
fn a_chain_with_a_refused_part_names_that_part() {
    let workspace = tempfile::tempdir().unwrap();
    for (command, part) in [
        ("ls crates; rm -rf target", "`rm -rf target`"),
        ("cat Cargo.toml | tee var/out.txt", "`tee var/out.txt`"),
        ("grep -n x src/main.rs | head -n 5; sed s/a/b/ src/main.rs", "`sed s/a/b/ src/main.rs`"),
        ("echo a || cd crates", "`cd crates`"),
    ] {
        match answer(workspace.path(), command) {
            Answer::Deny { reason, .. } => assert!(reason.contains(part), "`{command}` is refused without naming {part}: {reason}"),
            other => panic!("`{command}` is not refused: {other:?}"),
        }
    }
}

#[test]
fn a_chain_leaving_the_workspace_is_not_allowed_by_the_hook() {
    let workspace = tempfile::tempdir().unwrap();
    for command in ["cat ../secret.txt | head -n 5", "ls; cat C:/Windows/win.ini", "sed -n 1p ../x.rs"] {
        let answer = answer(workspace.path(), command);
        assert!(!answer.allows(), "`{command}`: {answer:?}");
    }
}
