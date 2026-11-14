//! A refused `sed -n` and a refused `python` each match a gotcha whose text names what to use.

use ra_gotchas::{Catalogue, Conditions, Verdict};

fn refusal(command: &str, id: &str) -> String {
    let Verdict::Deny { id: got, reason } = Catalogue::seed().command(command, Conditions::default()) else {
        panic!("{command} is refused with a hint");
    };
    assert_eq!(got, id, "{command}");
    reason
}

#[test]
fn sed_n_points_to_read_with_offset_and_limit() {
    let reason = refusal("sed -n '1,20p' src/main.rs", "sed-n");
    assert!(reason.contains("Read tool"), "{reason}");
    assert!(reason.contains("offset"), "{reason}");
    assert!(reason.contains("limit"), "{reason}");
}

#[test]
fn python_points_to_wsl_bash_with_python3_inside() {
    let reason = refusal("python tools/x.py", "python");
    assert!(reason.contains("wsl -- bash -l"), "{reason}");
    assert!(reason.contains("python3"), "{reason}");
}
