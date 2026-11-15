//! The hook lets a worker run a script it wrote under `var/` with plain arguments, and keeps
//! refusing a python with no file, a script on stdin and a script outside `var/`.

use ra_gotchas::shape::{Shape, Shell};

fn judged(command: &str) -> Shape {
    let dir = tempfile::tempdir().expect("a tempdir");
    ra_gotchas::shape::judge(command, Shell::Bash, dir.path(), &|_| false)
}

#[test]
fn a_python_script_under_var_passes_the_hook() {
    for command in ["python var/scan.py", "python var/scan.py --out var/report.json 12", "python3 ./var/scan.py a b"] {
        assert_eq!(judged(command), Shape::Allowed, "{command}");
    }
}

#[test]
fn python_without_a_file_or_with_a_script_outside_var_stays_refused() {
    for command in [
        "python",
        "python -",
        "python - <<EOF",
        "python -c \"print(1)\"",
        "python tools/scan.py",
        "python var/../tools/scan.py",
        "python var/scan.txt",
        "python C:/elsewhere/var/scan.py",
        "python var/scan.py; rm -rf var",
    ] {
        assert_ne!(judged(command), Shape::Allowed, "{command}");
    }
}
