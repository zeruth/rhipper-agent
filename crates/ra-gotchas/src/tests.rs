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
    // the read-only commands of both shells pass; a change of directory and a runtime still teach
    assert_eq!(catalogue.command("ls -la crates", Conditions::default()), Verdict::Pass);
    assert_eq!(catalogue.command("wc -l src/*.rs", Conditions::default()), Verdict::Pass);
    assert_eq!(catalogue.command("Get-ChildItem -Recurse crates", Conditions::default()), Verdict::Pass);
    for command in ["cd foo", "Set-Location crates", "python x.py", "sed -i 's/a/b/' x.rs"] {
        match catalogue.command(command, Conditions::default()) {
            Verdict::Deny { id, reason } => {
                assert_eq!(id, "shell-outside-allowlist", "{command}");
                assert!(reason.contains("Read"), "{reason}");
            }
            other => panic!("{command}: {other:?}"),
        }
    }
    assert_eq!(catalogue.command("git status --short", Conditions::default()), Verdict::Pass);
    // a pipe through tail keeps the output short; the harness judges its parts
    assert_eq!(catalogue.command("cargo test -p ra-gotchas 2>&1 | tail -15", Conditions::default()), Verdict::Pass);
    assert_eq!(catalogue.command("cargo test -p ra-gotchas", Conditions::default()), Verdict::Pass);
    // a shell inside a shell is no way around the allowlist
    for command in ["powershell.exe -NoProfile -File x.ps1", "sh var/check.sh", "bash -c 'cargo test'"] {
        assert!(matches!(catalogue.command(command, Conditions::default()), Verdict::Deny { .. }), "{command}");
    }
    // the git state is the person's, wherever in a chain the write sits
    for command in ["git stash -q && cargo clippy", "git add -A", "cargo test && git commit -m x", "git branch -D card"] {
        assert!(
            matches!(catalogue.command(command, Conditions::default()), Verdict::Deny { id, .. } if id == "git-writes"),
            "{command}"
        );
    }
    assert_eq!(catalogue.command("git diff --stat", Conditions::default()), Verdict::Pass);
    assert_eq!(catalogue.command("git branch --show-current", Conditions::default()), Verdict::Pass);
}

#[test]
fn a_workspace_build_is_rewritten_only_while_a_console_runs() {
    let catalogue = Catalogue::seed();
    let running = Conditions { console_running: true };
    match catalogue.command("cargo test -q", running) {
        Verdict::Rewrite { id, command, .. } => {
            assert_eq!(id, "binary-locked");
            assert_eq!(command, "cargo test --workspace --exclude rc --exclude rc-gui -q");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(catalogue.command("cargo test -p ra-roadmap", running), Verdict::Pass);
    assert_eq!(catalogue.command("cargo test -q", Conditions::default()), Verdict::Pass);
    match catalogue.command("cargo build", running) {
        Verdict::Rewrite { command, .. } => assert_eq!(command, "cargo build --workspace --exclude rc --exclude rc-gui"),
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
    // an edit that missed, and a worker that says it put one in the wrong place
    assert_eq!(catalogue.scan(Where::Edit, "Error: String to replace not found in file.")[0].id, "edit-missed");
    assert_eq!(catalogue.scan(Where::Edit, "Found 3 matches of the string to replace, but replace_all is false")[0].id, "edit-missed");
    assert!(catalogue.scan(Where::Edit, "The file x.rs has been updated successfully.").is_empty());
    assert_eq!(catalogue.scan(Where::Text, "Whoops, that edit landed in the wrong place; let me redo that edit.")[0].id, "misplaced-edit");
    assert!(catalogue.scan(Where::Text, "Now the edits to main.rs.").is_empty());
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
fn the_journal_lists_the_refused_commands_no_gotcha_names_oldest_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gotchas.log");
    let journal = Journal::at(&path);
    assert!(journal.unmatched().is_empty(), "no journal yet, no commands");
    let refused = |command: &str| Hit { id: UNMATCHED.into(), message: "refused".into(), excerpt: command.into() };
    journal.record(Some(3), Where::Command, &refused("Get-ChildItem -Recurse")).unwrap();
    journal.record(Some(3), Where::Command, &Hit { id: "mojibake".into(), message: "m".into(), excerpt: "Ã©".into() }).unwrap();
    journal.record(None, Where::Command, &refused("Select-String foo")).unwrap();
    assert_eq!(journal.unmatched(), ["Get-ChildItem -Recurse", "Select-String foo"]);
    // the unmatched hits still count among the gotchas, under their own id
    assert_eq!(journal.counts(), [(UNMATCHED.to_owned(), 2), ("mojibake".to_owned(), 1)]);
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
