//! A refused `python <file>.py` matches `python-in-bash`, whose text names the `.sh` run under WSL.

use ra_gotchas::{Catalogue, Conditions, Verdict};

#[test]
fn a_python_script_points_to_a_sh_run_under_wsl_bash() {
    let command = "python tools/x.py";
    let Verdict::Deny { id, reason } = Catalogue::seed().command(command, Conditions::default()) else {
        panic!("{command} is refused with a hint");
    };
    assert_eq!(id, "python-in-bash");
    assert!(reason.contains("wsl -d Ubuntu-24.04 -- bash -l"), "{reason}");
    assert!(reason.contains(".sh"), "{reason}");
}
