//! The tests of the catalogue: the seed parses, the verdicts, the scans and the journal.

use super::*;

/// A build command as the hook sends it on: under `rc hold`, the last of the rewrites.
fn held(command: &str) -> String {
    format!("rc hold -- {command}")
}

/// Whether a verdict is the build-slot rewrite alone of `command`, by either slot entry: what a
/// build command that no other gotcha touches comes to.
fn only_held(verdict: &Verdict, command: &str) -> bool {
    let slot = |id: &str| matches!(id, "build-slot" | "contract-runner-slot" | "heavy-work-slot");
    matches!(verdict, Verdict::Rewrite { id, command: rewritten, .. } if slot(id.as_str()) && *rewritten == held(command))
}

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
    // a pipe through tail keeps the output short; the harness judges its parts; the build
    // still takes its slot
    let piped = "cargo test -p ra-gotchas 2>&1 | tail -15";
    assert!(only_held(&catalogue.command(piped, Conditions::default()), piped));
    assert!(matches!(catalogue.command("cargo test -p ra-gotchas", Conditions::default()), Verdict::Rewrite { .. }));
    // a shell inside a shell is no way around the allowlist
    for command in ["powershell.exe -NoProfile -File x.ps1", "sh var/check.sh", "bash -c 'cargo test'"] {
        assert!(matches!(catalogue.command(command, Conditions::default()), Verdict::Deny { .. }), "{command}");
    }
    // the git state is the person's, wherever in a chain the write sits
    for command in ["git stash -q && cargo clippy", "git add -A", "cargo test && git commit -m x", "git branch -D card"] {
        assert!(matches!(catalogue.command(command, Conditions::default()), Verdict::Deny { id, .. } if id == "git-writes"), "{command}");
    }
    assert_eq!(catalogue.command("git diff --stat", Conditions::default()), Verdict::Pass);
    assert_eq!(catalogue.command("git branch --show-current", Conditions::default()), Verdict::Pass);
}

#[test]
fn a_whole_read_of_a_long_outlined_file_is_denied_with_its_items_and_the_rest_pass() {
    let catalogue = Catalogue::seed();
    let items = ["parse 149-151".to_owned(), "command 217-248".to_owned()];
    let bare = ReadCall { file: "crates/x/src/lib.rs", ranged: false, lines: 1200, items: Some(&items) };
    match catalogue.read(bare) {
        Verdict::Deny { id, reason } => {
            assert_eq!(id, "whole-file-read");
            assert!(reason.contains("crates/x/src/lib.rs is 1200 lines; its items: parse 149-151, command 217-248"), "{reason}");
            // the harness hands the reason back as the Read's result, and the stream counts it
            let hits = catalogue.refused_read(&reason);
            assert_eq!(hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), ["whole-file-read"]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(catalogue.read(ReadCall { ranged: true, ..bare }), Verdict::Pass);
    assert_eq!(catalogue.read(ReadCall { lines: 300, ..bare }), Verdict::Pass);
    assert_eq!(catalogue.read(ReadCall { items: None, ..bare }), Verdict::Pass);
    // a file's own text is no refusal
    assert!(catalogue.refused_read("fn main() {}\n").is_empty());
}

#[test]
fn a_kill_by_pid_is_handed_to_the_person_and_a_process_listing_passes() {
    let catalogue = Catalogue::seed();
    for command in [
        "Stop-Process -Id 13656 -Force",
        "taskkill /PID 13656 /F",
        "Get-CimInstance Win32_Process -Filter \"ProcessId = 13656\"",
        "Get-Process rc | Stop-Process",
    ] {
        match catalogue.command(command, Conditions::default()) {
            Verdict::Deny { id, reason } => {
                assert_eq!(id, "stuck-process", "{command}");
                assert!(reason.contains("name the pid in the report"), "{reason}");
            }
            other => panic!("{command}: {other:?}"),
        }
    }
    assert_eq!(catalogue.command("Get-Process rc", Conditions::default()), Verdict::Pass);
    assert_eq!(catalogue.command("Get-Process | Select-Object -First 5", Conditions::default()), Verdict::Pass);
}

#[test]
fn a_workspace_build_is_rewritten_only_while_a_console_runs() {
    let catalogue = Catalogue::seed();
    let running = Conditions { console_running: true, ..Conditions::default() };
    // the tail rewrite composes on the end of it; the first id names the verdict
    match catalogue.command("cargo test -q", running) {
        Verdict::Rewrite { id, command, reason } => {
            assert_eq!(id, "binary-locked");
            assert_eq!(command, held("cargo test --workspace --exclude rc --exclude rc-gui -q 2>&1 | tail -n 80"));
            assert!(reason.contains("holds its binary") && reason.contains("; a cargo test"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    let narrow = "cargo test -p ra-roadmap 2>&1 | tail -20";
    assert!(only_held(&catalogue.command(narrow, running), narrow));
    match catalogue.command("cargo test -q", Conditions::default()) {
        Verdict::Rewrite { id, command, .. } => {
            assert_eq!(id, "long-output");
            assert_eq!(command, held("cargo test -q 2>&1 | tail -n 80"));
        }
        other => panic!("{other:?}"),
    }
    match catalogue.command("cargo build", running) {
        Verdict::Rewrite { command, .. } => {
            assert_eq!(command, held("cargo build --workspace --exclude rc --exclude rc-gui 2>&1 | tail -n 80"))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_full_suite_runs_the_changed_crates_only_while_the_check_is_the_suite() {
    let catalogue = Catalogue::seed();
    let changed = ["ra-asm-emit".to_owned(), "ra-gotchas".to_owned()];
    let suite = Conditions { suite_check: true, changed_crates: &changed, ..Conditions::default() };
    for (command, expected) in [
        ("cargo test -q 2>&1 | tail -n 80", "cargo test -p ra-asm-emit -p ra-gotchas -q 2>&1 | tail -n 80"),
        ("cargo test --workspace -q", "cargo test -p ra-asm-emit -p ra-gotchas -q 2>&1 | tail -n 80"),
        (
            "cargo test --workspace --exclude rc --exclude=rc-gui -- --nocapture",
            "cargo test -p ra-asm-emit -p ra-gotchas -- --nocapture 2>&1 | tail -n 80",
        ),
    ] {
        match catalogue.command(command, suite) {
            Verdict::Rewrite { id, command, reason } => {
                assert_eq!(id, "worker-full-suite");
                assert_eq!(command, held(expected));
                assert!(reason.contains("runs after the report"), "{reason}");
            }
            other => panic!("{command}: {other:?}"),
        }
    }
    // a console running brings no workspace flags back onto the narrowed command
    let running = Conditions { console_running: true, ..suite };
    assert!(
        matches!(catalogue.command("cargo test -q", running), Verdict::Rewrite { command, .. } if command == held("cargo test -p ra-asm-emit -p ra-gotchas -q 2>&1 | tail -n 80"))
    );
    // a command that names its crate runs as it is, on its slot
    let named = "cargo test -p ra-asm-emit -q 2>&1 | tail -n 80";
    assert!(only_held(&catalogue.command(named, suite), named));
    // a project whose check is not the suite, or a run that changed no crate, keeps the command
    for conditions in
        [Conditions { changed_crates: &changed, ..Conditions::default() }, Conditions { suite_check: true, ..Conditions::default() }]
    {
        let tailed = "cargo test -q 2>&1 | tail -n 80";
        assert!(only_held(&catalogue.command(tailed, conditions), tailed));
    }
    assert!(is_suite("cargo test -q") && is_suite("cargo test --workspace -q"));
    assert!(!is_suite("cargo test -p ra-gotchas -q") && !is_suite(".\\gw.ps1 :core:test") && !is_suite("cargo build"));
}

#[test]
fn a_changed_path_falls_in_the_package_of_its_nearest_manifest() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/a\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("crates/a/src")).unwrap();
    std::fs::write(dir.path().join("crates/a/Cargo.toml"), "[package]\nname = \"a-crate\"\n").unwrap();
    assert_eq!(package_of(dir.path(), Path::new("crates/a/src/lib.rs")).as_deref(), Some("a-crate"));
    // the workspace manifest is no package
    assert_eq!(package_of(dir.path(), Path::new("rhipper.toml")), None);
}

#[test]
fn the_hooks_full_suite_in_a_work_tree_runs_the_crate_its_changes_fall_in() {
    let dir = tempfile::tempdir().unwrap();
    let git = std::process::Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status();
    assert!(git.is_ok_and(|s| s.success()), "git init");
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/a\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("crates/a/src")).unwrap();
    std::fs::write(dir.path().join("crates/a/Cargo.toml"), "[package]\nname = \"a-crate\"\n").unwrap();
    std::fs::write(dir.path().join("crates/a/src/lib.rs"), "").unwrap();
    assert_eq!(changed_crates(dir.path()), ["a-crate"]);
    let catalogue = Catalogue::seed();
    match catalogue.command_in(dir.path(), "cargo test -q 2>&1 | tail -n 80", false, true, false) {
        Verdict::Rewrite { id, command, .. } => {
            assert_eq!(id, "worker-full-suite");
            assert_eq!(command, held("cargo test -p a-crate -q 2>&1 | tail -n 80"));
        }
        other => panic!("{other:?}"),
    }
    // a per-crate command, or a project whose check is not the suite, runs as written, on its slot
    let per_crate = "cargo test -p b -q 2>&1 | tail -n 80";
    assert!(only_held(&catalogue.command_in(dir.path(), per_crate, false, true, false), per_crate));
    let tailed = "cargo test -q 2>&1 | tail -n 80";
    assert!(only_held(&catalogue.command_in(dir.path(), tailed, false, false, false), tailed));
}

#[test]
fn a_cargo_command_without_a_tail_gets_one_on_its_end() {
    let catalogue = Catalogue::seed();
    let quiet = Conditions::default();
    for (command, expected) in [
        ("cargo test -p ra-gotchas", "cargo test -p ra-gotchas 2>&1 | tail -n 80"),
        ("cargo clippy --workspace -- -D warnings", "cargo clippy --workspace -- -D warnings 2>&1 | tail -n 80"),
        ("cargo build -p rc ", "cargo build -p rc 2>&1 | tail -n 80"),
        ("cargo check -p ra-console", "cargo check -p ra-console 2>&1 | tail -n 80"),
    ] {
        match catalogue.command(command, quiet) {
            Verdict::Rewrite { id, command, reason } => {
                assert_eq!(id, "long-output");
                assert_eq!(command, held(expected));
                assert!(reason.contains("tail"), "{reason}");
            }
            other => panic!("{command}: {other:?}"),
        }
    }
    // a command that already pipes, or redirects to a file, gets no tail; so does cargo run;
    // each still builds, so each takes its slot
    for command in [
        "cargo test -p ra-gotchas 2>&1 | tail -n 40",
        "cargo test -p ra-gotchas | Select-Object -Last 30",
        "cargo build 2>&1 | grep -E 'error|warning'",
        "cargo test > var/test.log 2>&1",
        "cargo run -p rc -- doctor",
    ] {
        assert!(only_held(&catalogue.command(command, quiet), command), "{command}");
    }
    // a format is heavy work too: it runs on a worker slot (it does not count as a build)
    assert!(only_held(&catalogue.command("cargo fmt -p ra-gotchas", quiet), "cargo fmt -p ra-gotchas"));
}

#[test]
fn a_build_runs_on_a_build_slot_and_a_held_one_is_left_alone() {
    let catalogue = Catalogue::seed();
    let quiet = Conditions::default();
    for command in [
        ".\\gw.ps1 :android-awt:compileDebugJavaWithJavac",
        "./gw.ps1 :core:test 2>&1 | tail -n 40",
        "./gradlew build",
        "gradlew.bat :app:assembleDebug",
        // the AWT contract runner is a build too: a JVM per contract
        "python -u tools-src/run-awt-contracts.py --java-home \"C:/Program Files/Java/jdk-21.0.10\" > build/awt140/out.txt 2>&1",
        "python3 tools-src/run-awt-contracts.py",
    ] {
        assert!(only_held(&catalogue.command(command, quiet), command), "{command}");
    }
    // the other heavy work, in a project whose allowlist runs it: scripts, a JVM, recursive
    // searches, the WSL toolchains; one the project does not allow stays denied
    let allowed = Conditions { allowed: true, ..Conditions::default() };
    assert!(matches!(catalogue.command("python tools-src/scan.py", quiet), Verdict::Deny { .. }));
    for command in [
        "python -u tools-src/hub-compat-scan.py --out build/report.json",
        ".\\testdata\\build.ps1 -Name hello64",
        "powershell -NoProfile -File tools/regen.ps1",
        "node scripts/gen.mjs",
        "java -cp build/classes Main",
        "rg -n \"isValidateRoot\" android-awt",
        "grep -rn \"class Foo\" src",
        "Get-ChildItem -Recurse -Filter *.java | Select-String isValidateRoot",
        "wsl -d Ubuntu-24.04 -- bash -lc \"make -C testdata/psx\"",
    ] {
        assert!(only_held(&catalogue.command(command, allowed), command), "{command}");
    }
    // light commands stay as they are: a one-line python, a plain listing, a grep of one file
    for command in ["git status", "Get-ChildItem src", "Get-ChildItem -Recurse crates", "grep -n foo src/lib.rs"] {
        assert!(!matches!(catalogue.command(command, quiet), Verdict::Rewrite { ref id, .. } if id.ends_with("-slot")), "{command}");
    }
    // the gradle wrapper under a bash-ish `&` too; a command already under `rc hold` stays
    assert!(only_held(&catalogue.command("& .\\gw.ps1 :core:test", quiet), "& .\\gw.ps1 :core:test"));
    // a worker that typed `rc hold` itself is judged on the command inside: a build gets its
    // slot as the hook would have put it there, anything else is told to run as written
    let verdict = catalogue.command("rc hold --shell bash -- cargo check -p ra-console", quiet);
    assert!(
        matches!(&verdict, Verdict::Rewrite { command, .. } if command.starts_with("rc hold -- cargo check -p ra-console")),
        "{verdict:?}"
    );
    assert!(
        matches!(catalogue.command("rc hold -- cargo check -p ra-console", quiet), Verdict::Rewrite { ref command, .. } if command.starts_with("rc hold -- cargo check -p ra-console"))
    );
    assert!(matches!(catalogue.command("rc hold -- git status", quiet), Verdict::Deny { ref id, .. } if id == "self-held"));
    assert!(matches!(catalogue.command("rc hold -- rm -rf src", quiet), Verdict::Deny { .. }));
    assert_eq!(insert_after("cargo build", 0, "rc hold --"), "rc hold -- cargo build");
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
fn the_summary_counts_the_days_hits_by_gotcha_and_tallies_each_unmatched_command_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gotchas.log");
    assert_eq!(journal::summary(&path), journal::Summary::default(), "no journal yet, nothing to say");
    let journal = Journal::at(&path);
    let hit = |id: &str, excerpt: &str| Hit { id: id.into(), message: "m".into(), excerpt: excerpt.into() };
    journal.record(Some(3), Where::Command, &hit("mojibake", "Ã©")).unwrap();
    journal.record(Some(3), Where::Command, &hit("shell-outside-allowlist", "sed -n 1p x")).unwrap();
    journal.record(Some(4), Where::Result, &hit("mojibake", "Ã¨")).unwrap();
    journal.record(Some(4), Where::Command, &hit(UNMATCHED, "Get-ChildItem -Recurse")).unwrap();
    journal.record(None, Where::Command, &hit(UNMATCHED, "Select-String foo")).unwrap();
    journal.record(None, Where::Command, &hit(UNMATCHED, "Get-ChildItem -Recurse")).unwrap();
    let summary = journal::summary(&path);
    assert_eq!(summary.counts, [("mojibake".to_owned(), 2), ("shell-outside-allowlist".to_owned(), 1)]);
    assert_eq!(summary.unmatched, [("Get-ChildItem -Recurse".to_owned(), 2), ("Select-String foo".to_owned(), 1)]);
    assert_eq!(
        summary.lines(),
        [
            "gotcha mojibake fired 2 times in the last day",
            "gotcha shell-outside-allowlist fired once in the last day",
            "refused command `Get-ChildItem -Recurse` matched no gotcha, 2 times in the last day",
            "refused command `Select-String foo` matched no gotcha, once in the last day",
        ]
    );
    // a hit older than the window is not counted
    let later = journal::unix_now() + 10;
    assert_eq!(journal::summary_since(&path, later), journal::Summary::default());
}

#[test]
fn a_refused_command_back_a_third_time_across_projects_is_drafted_with_its_hook() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a/var/gotchas.log"), dir.path().join("b/var/gotchas.log"));
    let refused = |command: &str| Hit { id: UNMATCHED.into(), message: "refused".into(), excerpt: command.into() };
    // the same command word three times, from two projects, with different arguments
    Journal::at(&a).record(Some(1), Where::Command, &refused("Select-String -Pattern foo src")).unwrap();
    Journal::at(&b).record(None, Where::Command, &refused("Select-String foo")).unwrap();
    Journal::at(&a).record(Some(2), Where::Command, &refused("Select-String bar *.rs")).unwrap();
    // three times in one project is that project's habit; twice across two is not yet a third time
    for _ in 0..3 {
        Journal::at(&a).record(Some(2), Where::Command, &refused("Measure-Object -Line")).unwrap();
    }
    Journal::at(&a).record(Some(3), Where::Command, &refused("rm -rf build")).unwrap();
    Journal::at(&b).record(Some(3), Where::Command, &refused("rm -rf target")).unwrap();
    let lines = Journal::refused_across(&[("a", &a), ("b", &b)]);
    assert_eq!(lines.len(), 8);
    let drafts = catalog::draft(&lines);
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    assert_eq!(
        (drafts[0].id.as_str(), drafts[0].hits, drafts[0].projects.as_slice()),
        ("refused-select-string", 3, ["a".to_owned(), "b".to_owned()].as_slice())
    );

    // the catalogue gains the entry under its mark, and its pre-hook denies the command with the fix
    let path = dir.path().join("a/var/gotchas.toml");
    let (catalogue, drafted) = Catalogue::promote(&path, &lines).unwrap();
    assert_eq!(drafted, ["refused-select-string"]);
    assert_eq!(catalogue.entries.len(), Catalogue::seed().entries.len() + 1);
    match catalogue.command("Select-String baz crates", Conditions::default()) {
        Verdict::Deny { id, reason } => {
            assert_eq!(id, "refused-select-string");
            assert!(reason.contains("3 times across 2 projects (a, b)") && reason.contains("Grep"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(catalogue.command("ls | Select-String x", Conditions::default()), Verdict::Deny { .. }));
    assert_eq!(catalogue.command("ls crates", Conditions::default()), Verdict::Pass);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(&format!("\n{}1 refused in a, b\n", catalog::DRAFT_MARK)), "{text}");
    assert!(text.ends_with(&drafts[0].block()), "{text}");

    // drafted once: a second pass finds the entry in place and leaves the file alone
    let (again, drafted) = Catalogue::promote(&path, &lines).unwrap();
    assert!(drafted.is_empty());
    assert_eq!(again.entries.len(), catalogue.entries.len());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
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

#[test]
fn a_refresh_brings_the_seeds_new_entries_and_keeps_the_files_own() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("var/gotchas.toml");
    // a project written from an older seed: no long-output entry, and a message of its own
    let seed = Catalogue::seed();
    let long_output = seed.get("long-output").unwrap();
    let (_, block) = seed_blocks().into_iter().find(|(id, _)| id == "long-output").unwrap();
    // a checkout may end the seed's lines in `\r\n`; the blocks end theirs in `\n`
    let seed_text = SEED.replace("\r\n", "\n");
    assert!(seed_text.contains(&block), "the seed's blocks are its own text");
    let own = seed_text.replacen(&format!("{block}\n"), "", 1).replace("a console or a window holds its binary", "the window holds rc.exe");
    assert!(Catalogue::parse(&own).unwrap().get("long-output").is_none(), "{own}");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &own).unwrap();

    let (merged, added) = Catalogue::refresh(&path).unwrap();
    assert_eq!(added, ["long-output"]);
    assert_eq!(merged.entries.len(), seed.entries.len());
    assert_eq!(merged.get("long-output").unwrap().message, long_output.message);
    // the project's entries stay its own, changed or not, byte for byte
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with(&own), "the file's own text was rewritten");
    assert!(merged.get("binary-locked").unwrap().message.starts_with("the window holds rc.exe"));
    assert!(text[own.len()..].starts_with(&format!("\n{SEED_MARK}")), "{}", &text[own.len()..]);
    assert_eq!(Catalogue::from_seed_later(&text), 1);
    assert_eq!(Catalogue::from_seed_later(&own), 0);

    // nothing missing: the file is not touched
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    let (again, added) = Catalogue::refresh(&path).unwrap();
    assert!(added.is_empty());
    assert_eq!(again.entries.len(), seed.entries.len());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
}
