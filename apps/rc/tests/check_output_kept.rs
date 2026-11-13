//! `rc check <card>` keeps a check's whole output in `var/checks/<card>-<run>.log`, prints that
//! path last, and runs the check with `TEMP` and `TMP` outside the project.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn rc(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rc")).args(args).env_remove("RHIPPER_SCRATCH").current_dir(dir).output().expect("rc runs")
}

/// A project with one task whose acceptance is `acceptance`; the task's id.
fn project_with(dir: &Path, acceptance: &str) -> String {
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
    id_of(&["add", "task", "T", "--parent", &milestone, "--acceptance", acceptance])
}

fn last_line(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).lines().last().expect("a line").to_owned()
}

#[test]
fn check_output_kept_whole_in_the_log_named_on_the_last_line() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let card = project_with(dir.path(), "`cargo no-such-subcommand-xyz`");
    let out = rc(dir.path(), &["check", &card]);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let log = PathBuf::from(last_line(&out));
    assert!(log.starts_with(dir.path().join("var").join("checks")), "{}", log.display());
    assert_eq!(log.file_name().and_then(|n| n.to_str()), Some(format!("{card}-1.log").as_str()));
    let kept = std::fs::read_to_string(&log).expect("the log");
    assert!(kept.contains("no such command"), "{kept}");
    assert!(kept.trim_end().ends_with("exit code 101"), "{kept}");

    let again = rc(dir.path(), &["check", &card]);
    assert!(last_line(&again).ends_with(&format!("{card}-2.log")), "{}", last_line(&again));
}

#[test]
fn check_output_kept_run_has_temp_and_tmp_outside_the_project() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let (script, body) = if cfg!(windows) {
        ("probe.ps1", "Write-Output \"TEMP=$env:TEMP\"\nWrite-Output \"TMP=$env:TMP\"\n")
    } else {
        ("probe.sh", "echo \"TEMP=$TEMP\"\necho \"TMP=$TMP\"\n")
    };
    std::fs::write(dir.path().join(script), body).expect("a probe");
    let card = project_with(dir.path(), &format!("`{script}`"));
    let out = rc(dir.path(), &["check", &card]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let kept = std::fs::read_to_string(last_line(&out)).expect("the log");
    let project = std::fs::canonicalize(dir.path()).expect("the project");
    for key in ["TEMP=", "TMP="] {
        let value = kept.lines().find_map(|l| l.strip_prefix(key)).unwrap_or_else(|| panic!("{key} in {kept}")).trim();
        assert!(!value.is_empty(), "{kept}");
        let value = std::fs::canonicalize(value).unwrap_or_else(|_| PathBuf::from(value));
        assert!(!value.starts_with(&project), "{key}{} is inside {}", value.display(), project.display());
    }
}
