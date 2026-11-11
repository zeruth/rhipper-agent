//! The tests of the context: the outline of a file, the draft of a project, a task's file
//! across runs, and what a run changed.

use std::path::Path;

use super::*;

const SOURCE: &str = "//! The store over a file.\n//! More.\n\nuse std::path::Path;\n\npub struct Store {\n    path: PathBuf,\n}\n\nimpl Store {\n    pub async fn open(path: &Path) -> anyhow::Result<Self> {\n        todo!()\n    }\n\n    fn write(&self) {}\n}\n\npub(crate) fn helper<T>(x: T) -> T\nwhere\n    T: Clone,\n{\n    x\n}\n\nconst ROOM: usize = 3;\n\n#[cfg(test)]\nmod tests;\n";

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
    assert_eq!(outline(Path::new("README.md"), "# Title\n\ntext\n\n## Part\n"), "README.md (5 lines)\n      1  # Title\n      5  ## Part\n");
    assert_eq!(outline(Path::new("Cargo.toml"), "[package]\nname = \"x\"\n\n[dependencies]\n"), "Cargo.toml (4 lines)\n      1  [package]\n      4  [dependencies]\n");
    assert_eq!(outline(Path::new("run.ps1"), "function Go {\n  x\n}\n"), "run.ps1 (3 lines)\n      1  function Go\n");
    let many: String = (0..70).map(|i| format!("fn f{i}() {{}}\n")).collect();
    let text = outline(Path::new("a.rs"), &many);
    assert_eq!(text.lines().count(), 1 + 60 + 1, "{text}");
    assert!(text.ends_with("… and 10 more\n"), "{text}");
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
    task.outlines = outline_files(root, &["src/lib.rs".to_owned(), "src/missing.rs".to_owned()]);
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
            changed: vec!["src/lib.rs".to_owned()],
            report: format!("report {i}\nsecond line\n## Handoff 9 · fake · header in a report"),
        });
    }
    task.save(root).unwrap();
    let again = TaskContext::open(root, 7);
    assert_eq!(again.handoffs.len(), 5);
    assert_eq!(again.handoffs[4].when, 1004);
    assert_eq!(again.handoffs[4].outcome, "implemented");
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
        task.record(Handoff { when: 1000 + i, model: "m".into(), outcome: "failed".into(), check: "c".into(), changed: vec![], report: "r".into() });
    }
    assert_eq!(task.handoffs.len(), 10);
    assert_eq!(task.handoffs[0].when, 1005);
    // a long report is clipped in the file, and the clip is not read back as report text
    let mut long = TaskContext::open(root, 8);
    long.record(Handoff { when: 1, model: "m".into(), outcome: "implemented".into(), check: "c".into(), changed: vec![], report: (0..50).map(|i| format!("line {i}\n")).collect() });
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
fn the_changed_paths_are_the_status_lines_a_run_added() {
    let before = " M apps/a.rs\n?? var/x\n";
    let after = " M apps/a.rs\n?? var/x\n M crates/b.rs\nR  old.rs -> new.rs\n?? \"with space.rs\"\n";
    assert_eq!(changed_between(before, after), ["crates/b.rs", "new.rs", "with space.rs"]);
    assert!(changed_between(after, after).is_empty());
    // a file that was dirty and changed again shows the same line: not counted, by design
    assert!(changed_between(" M a.rs\n", " M a.rs\n").is_empty());
}
