//! The tests of the catalogue: the seed parses, the verdicts, the scans and the journal.

use super::*;

#[test]
fn the_seed_parses_and_names_every_entry_once() {
    let catalogue = Catalogue::seed();
    let mut ids: Vec<&str> = catalogue.entries.iter().map(|g| g.id.as_str()).collect();
    assert!(ids.len() >= 6, "{ids:?}");
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), catalogue.entries.len(), "duplicate ids");
}

#[test]
fn a_command_outside_the_allowlist_is_denied_with_the_reason() {
    let catalogue = Catalogue::seed();
    // the read-only tools pass now; a change of directory and the PowerShell cmdlets still teach
    assert_eq!(catalogue.command("ls -la crates", Conditions::default()), Verdict::Pass);
    assert_eq!(catalogue.command("wc -l src/*.rs", Conditions::default()), Verdict::Pass);
    for command in ["cd foo", "Get-ChildItem -Recurse", "python x.py"] {
        match catalogue.command(command, Conditions::default()) {
            Verdict::Deny { id, reason } => {
                assert_eq!(id, "shell-outside-allowlist", "{command}");
                assert!(reason.contains("Read"), "{reason}");
            }
            other => panic!("{command}: {other:?}"),
        }
    }
    assert_eq!(catalogue.command("git status --short", Conditions::default()), Verdict::Pass);
}

#[test]
fn a_workspace_build_is_rewritten_only_while_a_console_runs() {
    let catalogue = Catalogue::seed();
    let running = Conditions { console_running: true };
    match catalogue.command("cargo test -q", running) {
        Verdict::Rewrite { id, command, .. } => {
            assert_eq!(id, "binary-locked");
            assert_eq!(command, "cargo test --workspace --exclude rc -q");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(catalogue.command("cargo test -p ra-roadmap", running), Verdict::Pass);
    assert_eq!(catalogue.command("cargo test -q", Conditions::default()), Verdict::Pass);
    match catalogue.command("cargo build", running) {
        Verdict::Rewrite { command, .. } => assert_eq!(command, "cargo build --workspace --exclude rc"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_result_and_an_answer_are_scanned_for_their_gotchas() {
    let catalogue = Catalogue::seed();
    let result =
        "   Compiling rc v0.1.0\nerror: failed to remove file `C:\\x\\target\\debug\\rc.exe`\nCaused by: Access is denied. (os error 5)";
    let hits = catalogue.scan(Where::Result, result);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "binary-locked-result");
    assert!(hits[0].excerpt.starts_with("error: failed to remove"), "{}", hits[0].excerpt);
    let hits = catalogue.scan(Where::Result, "Command name is a dynamic expression which is not supported");
    assert_eq!(hits[0].id, "powershell-not-allowed");
    assert_eq!(catalogue.scan(Where::Result, "test result: ok. 12 passed"), []);
    let hits = catalogue.scan(Where::Answer, "I've made the changes, but I couldn't run any tests.");
    assert_eq!(hits[0].id, "untested-claim");
    // a command pattern does not fire on a result
    assert!(catalogue.scan(Where::Result, "ls -la").is_empty());
}

#[test]
fn the_journal_counts_and_the_notes_name_each_gotcha_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gotchas.log");
    let journal = Journal::at(&path);
    assert!(journal.counts().is_empty());
    let catalogue = Catalogue::seed();
    let hit = |id: &str| Hit { id: id.into(), message: catalogue.get(id).unwrap().message.clone(), excerpt: "x\ty".into() };
    journal.record(Some(15), Where::Command, &hit("shell-outside-allowlist")).unwrap();
    journal.record(Some(15), Where::Command, &hit("shell-outside-allowlist")).unwrap();
    journal.record(None, Where::Result, &hit("mojibake")).unwrap();
    assert_eq!(journal.counts(), [("shell-outside-allowlist".to_owned(), 2), ("mojibake".to_owned(), 1)]);
    assert_eq!(journal.recent(2), ["mojibake", "shell-outside-allowlist"]);
    let notes = catalogue.notes(&journal.recent(10));
    assert!(notes.starts_with("About this machine"), "{notes}");
    assert_eq!(notes.matches("\n- ").count(), 2, "{notes}");
    assert_eq!(catalogue.notes(&[]), "");
    // a tab in the excerpt does not break the columns
    assert!(std::fs::read_to_string(&path).unwrap().lines().all(|l| l.split('\t').count() == 5));
}

#[test]
fn a_load_writes_the_seed_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("var/gotchas.toml");
    let first = Catalogue::load(&path).unwrap();
    assert_eq!(first.entries.len(), Catalogue::seed().entries.len());
    std::fs::write(
        &path,
        "[[gotcha]]\nid = \"only\"\nwhere = \"answer\"\npattern = \"x\"\nmessage = \"m\"\nremedy = { kind = \"hint\" }\n",
    )
    .unwrap();
    assert_eq!(Catalogue::load(&path).unwrap().entries.len(), 1);
    std::fs::write(&path, "[[gotcha]]\nid = \"bad\"\nwhere = \"answer\"\npattern = \"(\"\nmessage = \"m\"\nremedy = { kind = \"hint\" }\n")
        .unwrap();
    assert!(Catalogue::load(&path).is_err());
}
