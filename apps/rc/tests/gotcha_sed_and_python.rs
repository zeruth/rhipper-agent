//! A refused `sed -n` and a refused `python` each match a gotcha whose text names what to use.

use ra_gotchas::{Catalogue, Conditions, Verdict};

fn refusal_under(command: &str, id: &str, conditions: Conditions) -> String {
    let Verdict::Deny { id: got, reason } = Catalogue::seed().command(command, conditions) else {
        panic!("{command} is refused with a hint");
    };
    assert_eq!(got, id, "{command}");
    reason
}

fn refusal(command: &str, id: &str) -> String {
    refusal_under(command, id, Conditions::default())
}

#[test]
fn sed_n_points_to_read_with_offset_and_limit() {
    // the allowlist hint stands aside where the project allows its shell, and a sed that
    // writes is still no read
    let allowed = Conditions { allowed: true, ..Conditions::default() };
    let reason = refusal_under("sed -n -i 1p src/main.rs", "sed-n", allowed);
    assert!(reason.contains("Read tool"), "{reason}");
    assert!(reason.contains("offset"), "{reason}");
    assert!(reason.contains("limit"), "{reason}");
}

#[test]
fn a_sed_n_that_only_prints_is_not_named_by_the_sed_gotcha() {
    assert_eq!(Catalogue::seed().command("sed -n '1,20p' src/main.rs", Conditions::default()), Verdict::Pass);
}

#[test]
fn python_points_to_wsl_bash_with_python3_inside() {
    let reason = refusal("python -c \"print(1)\"", "python");
    assert!(reason.contains("wsl -- bash -l"), "{reason}");
    assert!(reason.contains("python3"), "{reason}");
}
