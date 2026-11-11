//! The tests of the context: the outline of a file, the draft of a project, a task's file
//! across runs, and what a run changed.

use std::path::Path;

use super::*;

const SOURCE: &str = "//! The store over a file.\n//! More.\n\nuse std::path::Path;\n\npub struct Store {\n    path: PathBuf,\n}\n\nimpl Store {\n    pub async fn open(path: &Path) -> anyhow::Result<Self> {\n        todo!()\n    }\n\n    fn write(&self) {}\n}\n\npub(crate) fn helper<T>(x: T) -> T\nwhere\n    T: Clone,\n{\n    x\n}\n\nconst ROOM: usize = 3;\n\n#[cfg(test)]\nmod tests;\n";

#[test]
fn the_standing_direction_rides_after_the_project_context() {
    let direction = vec!["g1 Local first {w3} · focus: local".to_owned(), "g2 Ship small {w1}".to_owned()];
    assert_eq!(with_direction("# P\n\nA project.\n", &[]), "# P\n\nA project.\n");
    assert_eq!(
        with_direction("# P\n\nA project.\n", &direction),
        "# P\n\nA project.\n\n## The person's standing direction\n- g1 Local first {w3} · focus: local\n- g2 Ship small {w1}\n"
    );
    // no project file yet: the direction stands alone
    assert!(with_direction("", &direction).starts_with(DIRECTION_HEADING));
}

#[test]
fn a_rust_outline_names_the_items_with_their_lines() {
    let text = outline(Path::new("crates/x/src/store.rs"), SOURCE);
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("crates/x/src/store.rs (28 lines) — The store over a file."));
    let rest: Vec<&str> = lines.map(str::trim).collect();
    assert_eq!(
        rest,
        [
            "6  pub struct Store",
            "10  impl Store",
            "11  pub async fn open(path: &Path) -> anyhow::Result<Self>",
            "15  fn write(&self)",
            "18  pub(crate) fn helper<T>(x: T) -> T",
            "25  const ROOM: usize = 3",
        ]
    );
    // a body two levels in is not an item; a use is not; the tests module is not
    assert!(!text.contains("todo"));
    assert!(!text.contains("use std"));
    assert!(!text.contains("mod tests"));
    // an inline tests module hides its functions, and the file goes on after it
    let inline = "fn real() {}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn a_test() {}\n}\n\nfn after() {}\n";
    assert_eq!(outline(Path::new("x.rs"), inline), "x.rs (11 lines)\n      1  fn real()\n     11  fn after()\n");
}

#[test]
fn other_files_outline_their_headings_tables_or_definitions_and_a_long_one_is_counted() {
    assert_eq!(
        outline(Path::new("README.md"), "# Title\n\ntext\n\n## Part\n"),
        "README.md (5 lines)\n      1  # Title\n      5  ## Part\n"
    );
    assert_eq!(
        outline(Path::new("Cargo.toml"), "[package]\nname = \"x\"\n\n[dependencies]\n"),
        "Cargo.toml (4 lines)\n      1  [package]\n      4  [dependencies]\n"
    );
    assert_eq!(outline(Path::new("run.ps1"), "function Go {\n  x\n}\n"), "run.ps1 (3 lines)\n      1  function Go\n");
    let many: String = (0..70).map(|i| format!("fn f{i}() {{}}\n")).collect();
    let text = outline(Path::new("a.rs"), &many);
    assert_eq!(text.lines().count(), 1 + 60 + 1, "{text}");
    assert!(text.ends_with("… and 10 more\n"), "{text}");
}

#[test]
fn an_outline_marks_the_items_that_hold_a_changed_line_and_no_others() {
    // `open` owns lines 11-14, `write` 15-17, `helper` 18-24, `ROOM` 25 to the end
    let text = outline_marked(Path::new("store.rs"), SOURCE, &[(12, 12), (25, 27)]);
    let marked: Vec<&str> = text.lines().filter(|l| l.starts_with('*')).map(str::trim).collect();
    assert_eq!(marked, ["*    11  pub async fn open(path: &Path) -> anyhow::Result<Self>", "*    25  const ROOM: usize = 3"], "{text}");
    assert!(text.contains("\n     15  fn write(&self)\n"), "{text}");
    assert_eq!(outline_marked(Path::new("store.rs"), SOURCE, &[]), outline(Path::new("store.rs"), SOURCE));
}

#[test]
fn a_diff_gives_the_changed_lines_of_each_file() {
    let diff = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3 +3 @@ fn x()\n-old\n+new\n@@ -10,2 +10,4 @@\n+a\n+b\n@@ -20,1 +21,0 @@\n-gone\ndiff --git a/b.md b/b.md\n--- a/b.md\n+++ b/b.md\n@@ -0,0 +1,2 @@\n+# T\n+text\n";
    let changed = changed_lines(diff);
    assert_eq!(changed["src/a.rs"], [(3, 3), (10, 13), (21, 21)]);
    assert_eq!(changed["b.md"], [(1, 2)]);
    assert_eq!(changed.len(), 2);
}

#[test]
fn a_draft_reads_the_workspace_and_ensure_writes_it_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("crates/ra-one/src")).unwrap();
    std::fs::create_dir_all(root.join("crates/ra-two/src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n[workspace.package]\nedition = \"2024\"\n[workspace.lints.clippy]\npedantic = \"warn\"\n",
    )
    .unwrap();
    std::fs::write(root.join("crates/ra-one/Cargo.toml"), "[package]\nname = \"ra-one\"\ndescription = \"The first crate\"\n").unwrap();
    std::fs::write(root.join("crates/ra-two/Cargo.toml"), "[package]\nname = \"ra-two\"\n").unwrap();
    std::fs::write(root.join("crates/ra-two/src/lib.rs"), "//! The second crate, by its doc line.\n").unwrap();
    std::fs::write(root.join("README.md"), "# Demo\n\nA demo of the draft.\nIt spans two lines.\n\nNot this paragraph.\n").unwrap();
    let draft = Project::draft(root);
    // a workspace without a package takes the README's title
    assert!(draft.starts_with("# Demo\n\n"), "{draft}");
    assert!(draft.contains("A demo of the draft. It spans two lines.\n"), "{draft}");
    assert!(!draft.contains("Not this paragraph"), "{draft}");
    assert!(draft.contains("- `crates/ra-one` — The first crate\n"), "{draft}");
    assert!(draft.contains("- `crates/ra-two` — The second crate, by its doc line\n"), "{draft}");
    assert!(draft.contains("pedantic"), "{draft}");
    assert!(draft.contains("edition 2024"), "{draft}");
    // ensure writes the draft, then reads the file as it is
    assert!(Project::load(root).is_none());
    assert_eq!(Project::ensure(root).unwrap(), draft);
    std::fs::write(root.join(PROJECT_FILE), "# Edited by hand\n").unwrap();
    assert_eq!(Project::ensure(root).unwrap(), "# Edited by hand\n");
    // an empty workspace still drafts something
    let empty = tempfile::tempdir().unwrap();
    assert!(Project::draft(empty.path()).starts_with("# "));
}

#[test]
fn a_task_file_keeps_its_handoffs_across_runs_and_the_prompt_shows_the_last_ones() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn one() {}\n").unwrap();
    let mut task = TaskContext::open(root, 7);
    assert!(task.handoffs.is_empty());
    task.brief = "Task #7: a thing\nObjective: do it\n".to_owned();
    task.outlines = outline_files(root, &["src/lib.rs".to_owned(), "src/missing.rs".to_owned()], &ChangedLines::new());
    assert!(task.outlines.contains("src/lib.rs (1 lines)\n      1  pub fn one()\n"), "{}", task.outlines);
    assert!(task.outlines.contains("src/missing.rs (not in the workspace yet)"), "{}", task.outlines);
    // the first prompt carries no handoff
    assert!(!task.prompt().contains("Earlier runs"));
    for i in 0..5 {
        task.record(Handoff {
            when: 1000 + i,
            model: "claude-fable-5-1".to_owned(),
            outcome: if i == 4 { "implemented".to_owned() } else { "needs revision".to_owned() },
            check: format!("check failed: cargo test -p x · run {i}"),
            verdict: (i == 4).then(|| "accepted by m".to_owned()),
            changed: vec!["src/lib.rs".to_owned()],
            report: format!("report {i}\nsecond line\n## Handoff 9 · fake · header in a report"),
            warnings: vec![],
        });
    }
    task.save(root).unwrap();
    let again = TaskContext::open(root, 7);
    assert_eq!(again.handoffs.len(), 5);
    assert_eq!(again.handoffs[4].when, 1004);
    assert_eq!(again.handoffs[4].outcome, "implemented");
    // the verdict reads back on the one handoff that has it
    assert_eq!(again.handoffs[4].verdict.as_deref(), Some("accepted by m"));
    assert_eq!(again.handoffs[3].verdict, None);
    assert_eq!(again.handoffs[0].check, "check failed: cargo test -p x · run 0");
    assert_eq!(again.handoffs[0].changed, ["src/lib.rs"]);
    // a header-looking line inside a report does not start a handoff
    assert_eq!(again.handoffs[2].report, "report 2\nsecond line");
    // the prompt shows the last three, oldest first
    let prompt = task.prompt();
    assert!(prompt.contains("Earlier runs of this task"), "{prompt}");
    assert!(!prompt.contains("report 1\n"), "{prompt}");
    assert!(prompt.contains("report 2\n") && prompt.contains("report 4\n"), "{prompt}");
    assert!(prompt.find("report 2").unwrap() < prompt.find("report 4").unwrap());
    // the file keeps ten at most
    for i in 5..15 {
        task.record(Handoff {
            when: 1000 + i,
            model: "m".into(),
            outcome: "failed".into(),
            check: "c".into(),
            verdict: None,
            changed: vec![],
            report: "r".into(),
            warnings: vec![],
        });
    }
    assert_eq!(task.handoffs.len(), 10);
    assert_eq!(task.handoffs[0].when, 1005);
    // a long report is clipped in the file, and the clip is not read back as report text
    let mut long = TaskContext::open(root, 8);
    long.record(Handoff {
        when: 1,
        model: "m".into(),
        outcome: "implemented".into(),
        check: "c".into(),
        verdict: None,
        changed: vec![],
        report: (0..50).map(|i| format!("line {i}\n")).collect(),
        warnings: vec![],
    });
    long.save(root).unwrap();
    let back = TaskContext::open(root, 8);
    assert_eq!(back.handoffs[0].report.lines().count(), 40);
    // pruning keeps the open cards' files
    assert_eq!(TaskContext::prune(root, &[8]), 1);
    assert!(!TaskContext::path(root, 7).exists());
    assert!(TaskContext::path(root, 8).exists());
    assert_eq!(TaskContext::prune(Path::new("no/such/dir"), &[]), 0);
}

#[test]
fn a_handoff_with_warnings_keeps_them_under_their_heading_and_the_next_prompt_carries_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let unused =
        "warning: unused variable: `x`\n --> src/extra.rs:1:22\n  |\n1 | pub fn extra() { let x = 1; }\n  |                      ^";
    let docs = "warning: docs for function returning `Result` missing `# Errors` section\n  --> src/extra.rs:9:1";
    let mut task = TaskContext::open(root, 9);
    task.brief = "Task #9: a thing\n".to_owned();
    task.record(Handoff {
        when: 1,
        model: "m".into(),
        outcome: "needs revision".into(),
        check: "check passed: cargo test · lint: 2 warning(s)".into(),
        verdict: None,
        changed: vec!["src/extra.rs".into()],
        // a long report: the clip does not reach the warnings, and a heading in it is dropped
        report: (0..50).map(|i| format!("line {i}\n")).collect::<String>() + "### Warnings\nnot a section",
        warnings: vec![unused.to_owned(), docs.to_owned()],
    });
    task.save(root).unwrap();
    let text = std::fs::read_to_string(TaskContext::path(root, 9)).unwrap();
    let section = text.split("### Warnings\n").nth(1).expect("the heading");
    assert_eq!(text.matches("### Warnings").count(), 1, "{text}");
    assert_eq!(section, format!("{unused}\n\n{docs}\n"), "{text}");
    assert!(text.find("more lines in the notes").unwrap() < text.find("### Warnings").unwrap(), "{text}");
    // the file reads back whole, and the next prompt carries the warnings
    let again = TaskContext::open(root, 9);
    assert_eq!(again.handoffs[0].warnings, [unused, docs]);
    assert_eq!(again.handoffs[0].report.lines().count(), 40, "{}", again.handoffs[0].report);
    assert_eq!(again.last_warnings(), [unused, docs]);
    assert!(again.prompt().contains(&format!("### Warnings\n{unused}\n\n{docs}\n")), "{}", again.prompt());
    // a later clean run leaves nothing to fix first
    task.record(Handoff {
        when: 2,
        model: "m".into(),
        outcome: "implemented".into(),
        check: "c".into(),
        verdict: None,
        changed: vec![],
        report: "r".into(),
        warnings: vec![],
    });
    assert!(task.last_warnings().is_empty());
    task.save(root).unwrap();
    let again = TaskContext::open(root, 9);
    assert_eq!(again.handoffs[0].warnings.len(), 2);
    assert!(again.handoffs[1].warnings.is_empty());
    assert_eq!(again.handoffs[1].report, "r");
}

#[test]
fn a_notes_for_line_leaves_the_report_and_lands_as_a_handoff_on_the_other_card() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let report = "done\n- notes-for #9: the seam is in prepare\nNotes-for #9: workspace is overridden\nnotes-for #10: hi\nnotes-for nobody: stays\nnotes-for #11:\n";
    let (rest, notes) = notes_for(report);
    assert_eq!(rest, "done\nnotes-for nobody: stays\nnotes-for #11:\n");
    assert_eq!(notes, vec![(9, "the seam is in prepare\nworkspace is overridden".to_owned()), (10, "hi".to_owned())]);
    // a report without one is untouched
    assert_eq!(notes_for("plain\n").0, "plain\n");
    TaskContext::leave_note(root, 9, 7, "m", 5, &notes[0].1).unwrap();
    let other = TaskContext::open(root, 9);
    assert_eq!(other.handoffs.len(), 1);
    let note = &other.handoffs[0];
    assert_eq!(note.when, 5);
    assert_eq!(note.outcome, "note from #7");
    assert_eq!(note.report, "the seam is in prepare\nworkspace is overridden");
    assert!(other.prompt().contains("## Handoff 5 · m · note from #7\n"), "{}", other.prompt());
}

#[test]
fn the_changed_paths_are_the_status_lines_a_run_added() {
    let before = " M apps/a.rs\n?? var/x\n";
    let after = " M apps/a.rs\n?? var/x\n M crates/b.rs\nR  old.rs -> new.rs\n?? \"with space.rs\"\n";
    assert_eq!(changed_between(before, after), ["crates/b.rs", "new.rs", "with space.rs"]);
    assert!(changed_between(after, after).is_empty());
    // a file that was dirty and changed again shows the same line: not counted, by design
    assert!(changed_between(" M a.rs\n", " M a.rs\n").is_empty());
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn the_leftovers_are_the_files_of_a_rejected_card_still_dirty_and_a_clean_tree_has_no_section() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        std::fs::write(root.join("src").join(name), "fn x() {}\n").unwrap();
    }
    git_ok(root, &["init", "-q"]);
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-q", "-m", "base"]);
    // card 7 came back needs-revision having changed three files; c.rs was committed since
    let mut task = TaskContext::open(root, 7);
    task.record(Handoff {
        when: 1000,
        model: "m".to_owned(),
        outcome: "needs revision".to_owned(),
        check: "check failed: cargo test -p x".to_owned(),
        verdict: None,
        changed: vec!["src/a.rs".to_owned(), "src/b.rs".to_owned(), "src/c.rs".to_owned()],
        report: "did it".to_owned(),
        warnings: vec![],
    });
    task.save(root).unwrap();
    // a clean tree names nothing, whatever the card changed
    assert!(digest::leftovers(root, &[7]).is_empty());
    assert_eq!(digest::section(&digest::leftovers(root, &[7])), "");
    std::fs::write(root.join("src/a.rs"), "fn x() { changed }\n").unwrap();
    std::fs::write(root.join("src/b.rs"), "fn y() {}\n").unwrap();
    // an unrelated dirty file, and a card not under revision, are not leftovers
    std::fs::write(root.join("src/d.rs"), "fn d() {}\n").unwrap();
    let found = digest::leftovers(root, &[7]);
    assert_eq!(found.keys().collect::<Vec<_>>(), ["src/a.rs", "src/b.rs"]);
    assert_eq!(found["src/a.rs"], [7]);
    assert!(digest::leftovers(root, &[]).is_empty());
    assert_eq!(digest::section(&found), "## Uncommitted leftovers\n- src/a.rs (#7)\n- src/b.rs (#7)\n");
    // two cards on one file name both, once each, and a renamed path reads by its new name
    let cards = vec![(7, vec!["x.rs".to_owned()]), (9, vec!["x.rs".to_owned(), "new.rs".to_owned()]), (9, vec!["x.rs".to_owned()])];
    let found = digest::leftovers_in(" M x.rs\nR  old.rs -> new.rs\n", &cards);
    assert_eq!(found["x.rs"], [7, 9]);
    assert_eq!(found["new.rs"], [9]);
}
