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

const PROJECT: &str = "# P\n\nA project.\n\n## Layout\n\n- `crates/ra-roadmap` — the roadmap\n- `crates/ra-providers` — the providers\n- `crates/ra-tools` — the tools a bare model gets\n- `crates/ra-usage` — the ledger\n- `crates/ra-gotchas` — the catalogue\n- `crates/ra-context` — the context; `ra-tools` reads it for the outline\n- `apps/rc` — the CLI\n- the hub directory (`hub::dir()`) is shared by every project\n\n## Build and checks\n\n- `cargo test -p x`\n\n## Conventions\n\n- match the code\n\n## The window\n\nThe egui window in `apps/rc-gui`.\n\n## The toolbox\n\nWhat `ra-tools` allows.\n\n## The person's standing direction\n- g1 Local first {w3}\n";

#[test]
fn the_project_context_is_cut_to_the_card_s_crates_and_a_card_without_files_keeps_it_whole() {
    assert_eq!(cut_to(PROJECT, &[]), PROJECT);
    let cut = cut_to(PROJECT, &["crates/ra-tools/src/lib.rs".to_owned()]);
    // eight layout lines, three kept: the crate's own, the one that names it and the one with no path
    let layout: Vec<&str> =
        cut.lines().filter(|l| l.starts_with("- `crates") || l.starts_with("- `apps") || l.starts_with("- the hub")).collect();
    assert_eq!(
        layout,
        [
            "- `crates/ra-tools` — the tools a bare model gets",
            "- `crates/ra-context` — the context; `ra-tools` reads it for the outline",
            "- the hub directory (`hub::dir()`) is shared by every project"
        ],
        "{cut}"
    );
    // the lead, the build, the conventions and the direction ride whatever the card names
    for kept in [
        "# P\n\nA project.\n",
        "## Build and checks\n\n- `cargo test -p x`",
        "## Conventions\n\n- match the code",
        "## The person's standing direction\n- g1 Local first {w3}\n",
    ] {
        assert!(cut.contains(kept), "{cut}");
    }
    // another section rides when it names the crate and not otherwise
    assert!(cut.contains("## The toolbox\n\nWhat `ra-tools` allows."), "{cut}");
    assert!(!cut.contains("## The window"), "{cut}");
    // a file at the root names no crate and keeps the whole layout
    assert!(cut_to(PROJECT, &["gw.ps1".to_owned()]).contains("- `crates/ra-usage`"));
}

#[test]
fn the_machine_notes_drop_what_the_project_context_or_an_earlier_note_already_says() {
    let project = "## Build\n- Bash and PowerShell both run cargo, the read-only git commands and the read-only listing commands; pipe a long output through `tail`.\n";
    let notes = "About this machine, from earlier runs:\n- Bash and PowerShell both run cargo, the read-only git commands and the read-only listing commands; pipe a long output through tail\n- an edit missed: Read the lines first\n\nWhat earlier runs here said:\n- an edit missed: Read the lines first\n- (the person) the card named no files";
    assert_eq!(
        dedupe_notes(notes, project),
        "About this machine, from earlier runs:\n- an edit missed: Read the lines first\n\nWhat earlier runs here said:\n- (the person) the card named no files"
    );
    // a heading whose every line went goes with them, and nothing known leaves nothing
    assert_eq!(
        dedupe_notes(
            "About this machine:\n- Bash and PowerShell both run cargo, the read-only git commands and the read-only listing commands; pipe a long output through tail",
            project
        ),
        ""
    );
    assert_eq!(dedupe_notes("", project), "");
    // a short bullet inside a longer line of the context is not said by it
    assert_eq!(dedupe_notes("About this machine:\n- run cargo", project), "About this machine:\n- run cargo");
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
            read: vec![],
            failures: vec![],
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
            read: vec![],
            failures: vec![],
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
        read: vec![],
        failures: vec![],
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
fn a_rerun_after_a_reject_quotes_the_review_block_and_a_fresh_card_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let notes = "review by z-ai/glm-5.3-flash: needs revision (substantive)\n- hello.txt: the greeting lacks a newline\n- no test covers it\n\ncheck passed: cargo test -q · exit code 0\nok\n\nwrote hello.txt";
    let review = review_of(notes).expect("the notes open with a review");
    assert_eq!(
        review,
        "review by z-ai/glm-5.3-flash: needs revision (substantive)\n- hello.txt: the greeting lacks a newline\n- no test covers it"
    );
    // a card whose notes hold no verdict has no review
    assert_eq!(review_of("check passed: cargo test -q · exit code 0\nok"), None);
    assert_eq!(review_of(""), None);
    // the fresh card's prompt has no section
    let mut task = TaskContext::open(root, 9);
    task.brief = "Task #9: a thing\n".to_owned();
    assert!(!task.prompt().contains(REVIEW_HEADING), "{}", task.prompt());
    // the rejected card's prompt quotes the block verbatim, and its file keeps it
    task.review = Some(review);
    let prompt = task.prompt();
    assert!(prompt.contains("## Review\n"), "{prompt}");
    assert!(
        prompt.contains(
            "review by z-ai/glm-5.3-flash: needs revision (substantive)\n- hello.txt: the greeting lacks a newline\n- no test covers it\n"
        ),
        "{prompt}"
    );
    task.record(Handoff {
        when: 1,
        model: "m".into(),
        outcome: "needs revision (substantive)".into(),
        check: "check passed: cargo test -q".into(),
        verdict: Some("needs revision (substantive) · 2 points by z".into()),
        changed: vec!["hello.txt".into()],
        report: "wrote hello.txt".into(),
        warnings: vec![],
        read: vec![],
        failures: vec![],
    });
    task.save(root).unwrap();
    let file = std::fs::read_to_string(TaskContext::path(root, 9)).unwrap();
    assert!(file.contains("\n## Review\n\nreview by z-ai/glm-5.3-flash: needs revision (substantive)\n- hello.txt"), "{file}");
    // the section does not read back as a handoff, nor into one; it reads back as the review,
    // which the reviewer of the next round reads as its last verdict
    let again = TaskContext::open(root, 9);
    assert_eq!(again.handoffs.len(), 1);
    assert_eq!(again.handoffs[0].report, "wrote hello.txt");
    assert_eq!(again.review.as_deref(), task.review.as_deref());
}

#[test]
fn the_check_output_is_kept_clipped_to_its_tail_and_reads_back_apart_from_the_handoffs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut task = TaskContext::open(root, 4);
    task.brief = "Task #4\n".to_owned();
    task.set_check_output("$ cargo test -q · exit code 0\n## not a heading\ntest result: ok. 3 passed");
    task.record(Handoff {
        when: 1,
        model: "m".into(),
        outcome: "implemented".into(),
        check: "check passed: cargo test -q".into(),
        verdict: None,
        changed: vec![],
        report: "done".into(),
        warnings: vec![],
        read: vec![],
        failures: vec![],
    });
    task.save(root).unwrap();
    let again = TaskContext::open(root, 4);
    assert_eq!(again.check_output, task.check_output);
    assert_eq!(again.handoffs.len(), 1);
    assert_eq!(again.handoffs[0].report, "done");
    let long = format!("{}the end", "x".repeat(CHECK_OUTPUT_ROOM * 2));
    task.set_check_output(&long);
    assert!(task.check_output.ends_with("the end") && task.check_output.chars().count() < CHECK_OUTPUT_ROOM + 50);
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
        read: vec![],
        failures: vec![],
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
        read: vec![],
        failures: vec![],
    });
    assert!(task.last_warnings().is_empty());
    task.save(root).unwrap();
    let again = TaskContext::open(root, 9);
    assert_eq!(again.handoffs[0].warnings.len(), 2);
    assert!(again.handoffs[1].warnings.is_empty());
    assert_eq!(again.handoffs[1].report, "r");
}

#[test]
fn the_failures_of_a_check_are_every_failing_test_and_error_with_its_lines_and_read_back_from_the_file() {
    let cargo = "running 3 tests\ntest a::one ... ok\ntest a::two ... FAILED\ntest a::three ... FAILED\n\nfailures:\n\n---- a::two stdout ----\nsome print\n\nthread 'a::two' panicked at src/a.rs:4:5:\nassertion `left == right` failed\n  left: 1\n right: 2\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n\n---- a::three stdout ----\nthread 'a::three' panicked at src/a.rs:9:5:\nno greeting\n\nfailures:\n    a::two\n    a::three\n\ntest result: FAILED. 1 passed; 2 failed\nerror: test failed, to rerun pass `-p a --lib`\nexit code 101";
    let failures = failures_of(cargo);
    assert_eq!(
        failures,
        [
            "a::two\nthread 'a::two' panicked at src/a.rs:4:5:\nassertion `left == right` failed\n  left: 1\n right: 2",
            "a::three\nthread 'a::three' panicked at src/a.rs:9:5:\nno greeting"
        ]
    );
    let compile = "   Compiling a v0.1.0\nerror[E0308]: mismatched types\n --> src/a.rs:3:5\n  |\n3 |     1\n  |     ^ expected `String`\n\nerror: could not compile `a` (lib) due to 1 previous error\nexit code 101";
    assert_eq!(failures_of(compile), ["error[E0308]: mismatched types\n --> src/a.rs:3:5\n  |\n3 |     1\n  |     ^ expected `String`"]);
    let gradle = "> Task :app:test\n\nGreeterTest > greetsByName() FAILED\n    org.opentest4j.AssertionFailedError: expected: <hi> but was: <ho>\n        at app//GreeterTest.greetsByName(GreeterTest.java:12)\n\nGreeterTest > greetsNobody() FAILED\n    java.lang.NullPointerException\n\n3 tests completed, 2 failed\n\n> Task :app:test FAILED\n\nFAILURE: Build failed with an exception.\n\n* What went wrong:\nExecution failed for task ':app:test'.\n\nexit code 1";
    let failures = failures_of(gradle);
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(failures[0].starts_with("GreeterTest > greetsByName()\n    org.opentest4j.AssertionFailedError"), "{failures:?}");
    assert_eq!(failures[1], "GreeterTest > greetsNobody()\n    java.lang.NullPointerException");
    // a build that names no test still says what went wrong; an output that names nothing gives none
    assert_eq!(
        failures_of("FAILURE: x\n\n* What went wrong:\nCould not resolve all files.\n\nexit code 1"),
        ["What went wrong:\nCould not resolve all files."]
    );
    assert!(failures_of("ok\nexit code 1").is_empty());
    // the base's own failures mark theirs
    let marked = marked_on_base(&failures, &[failures[1].clone()]);
    assert_eq!(marked[0], failures[0]);
    assert_eq!(marked[1], format!("GreeterTest > greetsNobody(){ON_BASE}\n    java.lang.NullPointerException"));
    // the handoff keeps them whole, apart from the report, and reads them back
    let dir = tempfile::tempdir().unwrap();
    let mut task = TaskContext::open(dir.path(), 3);
    task.record(Handoff {
        when: 1,
        model: "m".into(),
        outcome: "needs revision".into(),
        check: "check failed: gradle test".into(),
        verdict: None,
        changed: vec![],
        report: "did it".into(),
        warnings: vec![],
        read: vec![],
        failures: marked.clone(),
    });
    task.save(dir.path()).unwrap();
    let again = TaskContext::open(dir.path(), 3);
    assert_eq!(again.handoffs[0].failures, marked);
    assert_eq!(again.handoffs[0].report, "did it");
}

#[test]
fn a_read_set_merges_each_file_s_ranges_and_a_whole_read_stands_for_the_file() {
    let reads: Vec<String> =
        ["a.rs:10-40", "D:/x.rs", "a.rs:30-60", "b.rs:5-9", "a.rs:61-70", "a.rs:200-210", "b.rs", "D:/x.rs:1-5"].map(str::to_owned).into();
    assert_eq!(read_set(&reads), ["a.rs:10-70", "a.rs:200-210", "D:/x.rs", "b.rs"]);
    assert_eq!(read_entry("D:/x.rs"), ("D:/x.rs", None));
    assert_eq!(read_entry("D:/x.rs:3-9"), ("D:/x.rs", Some((3, 9))));
}

#[test]
fn the_handoff_names_the_read_set_and_the_next_prompt_opens_with_its_regions_clipped_to_items() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    // three functions of 100 lines each: a read of 1-300 is clipped after the second
    let body: String =
        (0..3).map(|f| format!("fn f{f}() {{\n{}}}\n", (0..98).map(|i| format!("    let _ = {i};\n")).collect::<String>())).collect();
    std::fs::write(root.join("src/big.rs"), &body).unwrap();
    std::fs::write(root.join("src/small.rs"), "pub fn one() {}\npub fn two() {}\n").unwrap();
    let mut task = TaskContext::open(root, 11);
    task.brief = "Task #11: a thing\n".to_owned();
    task.record(Handoff {
        when: 1,
        model: "m".into(),
        outcome: "continues".into(),
        check: "no check: the context cap ended the run".into(),
        verdict: None,
        changed: vec![],
        report: "half way".into(),
        warnings: vec![],
        read: read_set(&["src/big.rs:1-300".to_owned(), "src/small.rs".to_owned(), "src/gone.rs:1-9".to_owned()]),
        failures: vec![],
    });
    task.save(root).unwrap();
    let text = std::fs::read_to_string(TaskContext::path(root, 11)).unwrap();
    assert!(text.contains("read: src/big.rs:1-300 src/small.rs src/gone.rs:1-9\n"), "{text}");
    let mut again = TaskContext::open(root, 11);
    assert_eq!(again.handoffs[0].read, ["src/big.rs:1-300", "src/small.rs", "src/gone.rs:1-9"]);
    assert_eq!(again.handoffs[0].report, "half way");
    again.brief = "Task #11: a thing\n".to_owned();
    again.regions = regions(root, &again.handoffs[0].read);
    let prompt = again.prompt();
    assert!(prompt.starts_with(REGIONS_HEADING), "{prompt}");
    assert!(prompt.find(REGIONS_HEADING).unwrap() < prompt.find("Task #11").unwrap());
    // the outline of the region, then its lines up to the end of f1, then what is left out
    assert!(
        prompt.contains("src/big.rs lines 1-300 (of 300)\n      1-100   fn f0()\n    101-200   fn f1()\n    201-300   fn f2()\n"),
        "{prompt}"
    );
    assert!(prompt.contains("     1  fn f0() {\n"), "{prompt}");
    assert!(prompt.contains("   200  }\n     … lines 201-300 left out; read them by range: f2 201-300\n"), "{prompt}");
    assert!(!prompt.contains("   201  fn f2"), "{prompt}");
    assert!(prompt.contains("src/small.rs lines 1-2 (of 2)\n") && prompt.contains("     2  pub fn two() {}\n"), "{prompt}");
    // a file no longer there is passed over; only the handoff still names it
    assert!(!again.regions.contains("gone.rs"), "{}", again.regions);
    // a run with no read set opens on its brief
    assert!(TaskContext::open(root, 12).prompt().is_empty());
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
        read: vec![],
        failures: vec![],
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

#[test]
fn the_context_lists_untracked_sources() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for d in ["crates/ra-tools/src", "crates/ra-usage/src", "tools", "target"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("crates/ra-tools/src/lib.rs"), "pub mod commands;\n").unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    git_ok(root, &["init", "-q"]);
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-q", "-m", "base"]);
    // a clean tree: nothing to list, and the context stands as it is
    assert!(untracked_sources(root, &[]).is_empty());
    assert_eq!(with_untracked("# P\n", &[]), "# P\n");
    // the new source under the card's crate, one in another crate, a script, a note and an ignored file
    std::fs::write(root.join("crates/ra-tools/src/commands.rs"), "pub fn run() {}\n").unwrap();
    std::fs::write(root.join("crates/ra-usage/src/new.rs"), "fn n() {}\n").unwrap();
    std::fs::write(root.join("tools/check.py"), "print(1)\n").unwrap();
    std::fs::write(root.join("gw.ps1"), "echo 1\n").unwrap();
    std::fs::write(root.join("NOTES.md"), "notes\n").unwrap();
    std::fs::write(root.join("target/out.rs"), "fn built() {}\n").unwrap();
    let all = untracked_sources(root, &[]);
    assert_eq!(all, ["crates/ra-tools/src/commands.rs", "crates/ra-usage/src/new.rs", "gw.ps1", "tools/check.py"]);
    // the card's files narrow the list to their crates
    let card = vec!["crates/ra-tools/src/lib.rs".to_owned()];
    assert_eq!(untracked_sources(root, &card), ["crates/ra-tools/src/commands.rs"]);
    assert_eq!(
        with_untracked("# P\n\nA project.\n", &untracked_sources(root, &card)),
        "# P\n\nA project.\n\n## Untracked sources\n- crates/ra-tools/src/commands.rs\n"
    );
    // a long list is clipped and the rest counted
    let many: Vec<String> = (0..23).map(|i| format!("src/f{i}.rs")).collect();
    let text = with_untracked("", &many);
    assert!(text.starts_with(UNTRACKED_HEADING));
    assert!(text.contains("- src/f19.rs\n… and 3 more\n"));
    assert!(!text.contains("src/f20.rs"));
}

#[test]
fn the_plan_context_carries_one_clipped_line_from_each_source_of_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("alpha");
    let hub = dir.path().join("hub");
    std::fs::create_dir_all(root.join("var")).unwrap();
    std::fs::create_dir_all(hub.join(plan::DIGEST_DIR)).unwrap();
    // no file of any source: no block at all
    let mut sources = plan::Sources::under(&root, &hub);
    assert_eq!(sources.this, "alpha");
    assert_eq!(plan::block(&sources), "");
    // the project's log: a card's line and one longer than the clip, oldest first, only the tail
    let long = "x".repeat(plan::CLIP + 40);
    let mut retro = String::new();
    for n in 0..plan::LINES_A_SOURCE + 2 {
        retro.push_str(&format!("100\t7\t0\tnote {n}\n"));
    }
    retro.push_str(&format!("100\t-\t2\t{long}\n"));
    std::fs::write(&sources.retro, retro).unwrap();
    // the shared notes: another project's and this project's own, which is left out
    std::fs::write(&sources.notes, "100\tbeta\t1\tsed is refused\n100\talpha\t1\tmine\n").unwrap();
    std::fs::write(
        sources.digests.join("beta.txt"),
        "name: beta\npath: C:/code/beta\npid: 1\nwhen: 100\nlast: ✓ #11 Shared notes\nready: 3\n",
    )
    .unwrap();
    std::fs::write(sources.digests.join("alpha.txt"), "name: alpha\npath: C:/code/alpha\npid: 1\nwhen: 100\nready: 1\n").unwrap();
    sources.ledger = vec![plan::LedgerCard { node: 12, cost_usd: Some(0.5), turns: 16, tokens_per_turn: 1200 }];
    let block = plan::block(&sources);
    assert!(block.starts_with("# Evidence\n\n## Look-backs"), "{block}");
    assert!(block.contains("- retro.log:4 card #7: note 3\n"), "{block}");
    assert!(!block.contains("note 2\n"), "{block}");
    assert!(block.contains("- notes.log:1 (beta) sed is refused\n"), "{block}");
    assert!(!block.contains("mine"), "{block}");
    assert!(block.contains("- usage.db #12: card #12 cost $0.50 over 16 turn(s), 1200 tokens a turn\n"), "{block}");
    assert!(block.contains("- beta · last: ✓ #11 Shared notes\n"), "{block}");
    assert!(!block.contains("alpha ·"), "{block}");
    // every line fits the clip, and the long one ends in an ellipsis
    assert!(block.lines().all(|l| l.chars().count() <= plan::CLIP + 2), "{block}");
    assert!(block.lines().any(|l| l.starts_with("- retro.log:8 x") && l.ends_with('…')), "{block}");
    assert_eq!(plan::clip("short"), "short");
}

#[test]
fn the_tree_since_the_last_run_reads_as_nothing_landed_merged_or_reset() {
    assert_eq!(since_line("abc123", "abc123def", true, ""), None);
    let landed = since_line("abc123", "def456", true, "def456 abc123\tCheckpoint: a card\n").unwrap();
    assert!(landed.starts_with("landed: 1 commit(s)") && landed.contains("Checkpoint: a card"), "{landed}");
    let merged = since_line("abc123", "def456", true, "def456 aaa111 bbb222\tMerge branch 'main' into card/7\naaa111 abc123\tx\n").unwrap();
    assert!(merged.starts_with("merged: Merge branch 'main' into card/7") && merged.contains("2 commit(s)"), "{merged}");
    let reset = since_line("abc123", "def456", false, "").unwrap();
    assert!(reset.starts_with("reset: the tree no longer holds abc123"), "{reset}");
}

fn standing_run(when: u64, failing: usize, points: usize) -> Handoff {
    Handoff {
        when,
        model: "m".to_owned(),
        outcome: "needs revision".to_owned(),
        check: "cargo test".to_owned(),
        verdict: (points > 0).then(|| format!("needs revision (substantive) · {points} point(s) by r")),
        changed: vec!["a.rs".to_owned()],
        report: String::new(),
        warnings: vec![],
        read: vec![],
        failures: (0..failing).map(|_| "test x\nsame error".to_owned()).collect(),
    }
}

#[test]
fn stalled_rounds_count_the_runs_since_the_standing_last_fell() {
    let dir = tempfile::tempdir().unwrap();
    let mut task = TaskContext::open(dir.path(), 9);
    assert_eq!(task.stalled_rounds(), 0);
    for (when, failing, points) in [(1, 5, 0), (2, 3, 1), (3, 3, 1), (4, 3, 1)] {
        task.record(standing_run(when, failing, points));
    }
    assert_eq!(task.stalled_rounds(), 2);
    task.record(standing_run(5, 1, 0));
    assert_eq!(task.stalled_rounds(), 0);
}

#[test]
fn a_handoff_head_names_the_tip_the_branch_s_commits_what_happened_and_what_is_open() {
    let tree = Tree {
        tip: "abc123".to_owned(),
        branch: Some("card/7".to_owned()),
        commits: vec!["c2 Checkpoint: run 2".to_owned(), "c1 Checkpoint: run 1".to_owned()],
        since: Some("merged: Merge branch 'main'".to_owned()),
    };
    let head = tree.head(&["cargo test -p x".to_owned()]);
    assert_eq!(
        head,
        "tree: at abc123 on card/7 · 2 commit(s) on the branch since its base: c2 Checkpoint: run 2; c1 Checkpoint: run 1\nsince the last run: merged: Merge branch 'main'\nopen: `cargo test -p x`\n"
    );
    // the head rides the report through the file, and the next run reads its tip back
    let report = format!("{head}the worker's words\nmore");
    let handoff = Handoff {
        when: 1,
        model: "m".to_owned(),
        outcome: "continues".to_owned(),
        check: "no check".to_owned(),
        verdict: None,
        changed: vec![],
        report: report.clone(),
        warnings: vec![],
        read: vec![],
        failures: vec![],
    };
    assert_eq!(tip_of(&handoff), Some("abc123"));
    assert_eq!(without_head(&report), "the worker's words\nmore");
    assert_eq!(without_head("plain report"), "plain report");
    assert_eq!(Tree::default().head(&[]), "");
    // the project's own tree has no branch and no commits to name
    let own = Tree { tip: "abc".to_owned(), ..Tree::default() };
    assert_eq!(own.head(&[]), "tree: at abc\n");
}
