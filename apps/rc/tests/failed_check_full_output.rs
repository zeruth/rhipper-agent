//! A failed check's whole output is kept uncut in a file, whose path `rc check` prints last.

use std::path::Path;
use std::process::{Command, Output};

fn rc(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).env_remove("RHIPPER_SCRATCH").current_dir(dir).output().expect("rc runs")
}

/// A project whose one test prints far more than a tail keeps, then fails; the card's id.
fn project_with_a_long_failure(dir: &Path) -> String {
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n")
        .expect("a manifest");
    std::fs::create_dir_all(dir.join("src")).expect("src");
    let lib = "#[test]\nfn it_fails_loudly() {\n    for n in 0..400 {\n        println!(\"noise line {n}\");\n    }\n    panic!(\"the end of it\");\n}\n";
    std::fs::write(dir.join("src").join("lib.rs"), lib).expect("a lib");
    let out = rc(dir, &["init", "--empty"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let id_of = |args: &[&str]| {
        let out = rc(dir, args);
        assert!(out.status.success(), "rc {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines().next().and_then(|l| l.strip_prefix('#')).and_then(|l| l.split_whitespace().next()).expect("an id").to_owned()
    };
    let project = id_of(&["add", "project", "Alpha"]);
    let milestone = id_of(&["add", "milestone", "M", "--parent", &project]);
    id_of(&["add", "task", "T", "--parent", &milestone, "--acceptance", "`cargo test it_fails_loudly -- --nocapture`"])
}

#[test]
fn a_failed_check_keeps_its_whole_output_in_a_file() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let card = project_with_a_long_failure(dir.path());
    let out = rc(dir.path(), &["check", &card]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    let path = stdout.lines().last().expect("a line");
    let kept = std::fs::read_to_string(path).expect("the log");
    assert!(kept.contains("noise line 0\n"), "{kept}");
    assert!(kept.contains("noise line 399\n"), "{kept}");
    assert!(kept.contains("the end of it"), "{kept}");
}
