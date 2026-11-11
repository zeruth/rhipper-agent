//! The tests of the toolbox: each tool over a temp workspace, the confinement and the allowlist.

use super::commands::{MAX_LINES, MAX_OUTPUT, clip};
use super::*;

fn call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall { id: "c".into(), name: name.into(), arguments: args.to_string() }
}

async fn workspace() -> (tempfile::TempDir, Toolbox) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    std::fs::write(dir.path().join("README.md"), "# Demo\n").unwrap();
    let toolbox = Toolbox::new(dir.path()).unwrap();
    (dir, toolbox)
}

#[tokio::test]
async fn read_file_numbers_the_lines_and_windows_long_files() {
    let (_dir, tools) = workspace().await;
    let out = tools.execute(&call("read_file", json!({"path": "src/main.rs"}))).await;
    assert!(out.starts_with("    1│fn main() {\n    2│"), "{out}");
    let out = tools.execute(&call("read_file", json!({"path": "src/main.rs", "offset": 2, "limit": 1}))).await;
    assert!(out.starts_with("    2│    println!"), "{out}");
    assert!(out.contains("read on from line 3"), "{out}");
    let out = tools.execute(&call("read_file", json!({"path": "nope.rs"}))).await;
    assert!(out.starts_with("error: cannot read nope.rs"), "{out}");
}

#[tokio::test]
async fn write_and_edit_change_the_file() {
    let (dir, tools) = workspace().await;
    let out = tools.execute(&call("write_file", json!({"path": "docs/new.md", "content": "hello\n"}))).await;
    assert_eq!(out, "wrote docs/new.md (6 bytes)");
    assert_eq!(std::fs::read_to_string(dir.path().join("docs/new.md")).unwrap(), "hello\n");
    let out = tools.execute(&call("edit_file", json!({"path": "src/main.rs", "old": "\"hi\"", "new": "\"hello\""}))).await;
    assert_eq!(out, "edited src/main.rs");
    assert!(std::fs::read_to_string(dir.path().join("src/main.rs")).unwrap().contains("println!(\"hello\")"));
    let out = tools.execute(&call("edit_file", json!({"path": "src/main.rs", "old": "n", "new": "m"}))).await;
    assert!(out.contains("times; include more context"), "{out}");
    let out = tools.execute(&call("edit_file", json!({"path": "src/main.rs", "old": "absent", "new": ""}))).await;
    assert!(out.contains("does not contain"), "{out}");
}

#[tokio::test]
async fn list_and_search_walk_the_workspace() {
    let (_dir, tools) = workspace().await;
    assert_eq!(tools.execute(&call("list_dir", json!({}))).await, "README.md\nsrc/");
    assert_eq!(tools.execute(&call("list_dir", json!({"path": "src"}))).await, "main.rs");
    let out = tools.execute(&call("search", json!({"text": "println"}))).await;
    assert_eq!(out, "src/main.rs:2: println!(\"hi\");");
    let out = tools.execute(&call("search", json!({"text": "zzz"}))).await;
    assert_eq!(out, "no line contains \"zzz\"");
}

#[tokio::test]
async fn every_path_stays_inside_the_workspace() {
    let (_dir, tools) = workspace().await;
    for path in ["../outside.txt", "src/../../outside.txt", "C:/Windows/win.ini", "/etc/passwd"] {
        let out = tools.execute(&call("read_file", json!({"path": path}))).await;
        assert!(out.starts_with("error:") && (out.contains("leaves the workspace") || out.contains("not relative")), "{path}: {out}");
    }
    // a dot-dot that stays inside is fine
    let out = tools.execute(&call("read_file", json!({"path": "src/../README.md"}))).await;
    assert!(out.contains("# Demo"), "{out}");
}

#[tokio::test]
async fn run_keeps_to_the_allowlist() {
    let (_dir, tools) = workspace().await;
    let out = tools.execute(&call("run", json!({"command": "rm -rf src"}))).await;
    assert!(out.contains("rm is not allowed"), "{out}");
    let out = tools.execute(&call("run", json!({"command": "git push origin main"}))).await;
    assert!(out.contains("git is not allowed"), "{out}");
    let out = tools.execute(&call("run", json!({"command": "cargo --version"}))).await;
    assert!(out.starts_with("cargo "), "{out}");
    assert!(out.trim_end().ends_with("exit code 0"), "{out}");
}

#[tokio::test]
async fn a_bad_tool_or_bad_arguments_answer_in_words() {
    let (_dir, tools) = workspace().await;
    assert!(tools.execute(&call("fly", json!({}))).await.starts_with("error: no tool fly"));
    assert!(tools.execute(&call("read_file", json!({"file": "x"}))).await.contains("do not fit the tool"));
    assert!(
        tools
            .execute(&ToolCall { id: "c".into(), name: "read_file".into(), arguments: "{not json".into() })
            .await
            .contains("not a JSON object")
    );
}

#[test]
fn clip_keeps_the_head_and_the_tail() {
    let long = "a".repeat(MAX_OUTPUT + 100);
    let out = clip(&long);
    assert!(out.contains("… 100 characters cut …"), "{}", out.len());
    assert_eq!(clip("short"), "short");
}

#[tokio::test]
async fn a_long_command_output_reaches_the_agent_clipped_to_its_head_and_tail() {
    let (_dir, tools) = workspace().await;
    let build: String = (1..=1000).map(|n| format!("   Compiling crate-{n} v0.1.0\n")).collect();
    let tools = tools.with_script(Arc::new(move |_: &str| Ok((0, build.clone()))));
    let out = tools.execute(&call("run", json!({"command": "cargo build"}))).await;
    // the lines of the output and the exit line, less what the marker stands for
    assert!(out.lines().count() <= MAX_LINES + 1, "{} lines", out.lines().count());
    assert!(out.contains("… 801 lines clipped …"), "{out}");
    assert!(out.starts_with("   Compiling crate-1 v0.1.0\n"), "the head stays");
    assert!(out.ends_with("   Compiling crate-1000 v0.1.0\nexit code 0"), "the tail and the verdict stay");
    // a check is the lint's, not a turn's: it reads the whole output
    assert_eq!(tools.check("cargo build").await.unwrap().1.lines().count(), 1001);
}

#[test]
fn the_specs_name_every_tool_once() {
    let names: Vec<String> = Toolbox::specs().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["read_file", "write_file", "edit_file", "list_dir", "search", "run"]);
}

const CLIPPY: &str = "    Checking probe v0.1.0\nwarning: unused variable: `x`\n --> crates/probe/src/lib.rs:2:9\n  |\n2 |     let x = 1;\n  |         ^\n\nwarning: this could be a `let` chain\n  --> crates/other/src/lib.rs:9:5\n\nwarning: `probe` (lib) generated 1 warning\n    Finished `dev` profile\n";

#[test]
fn warnings_at_keeps_the_diagnostics_of_the_named_files_whole() {
    let hits = warnings_at(CLIPPY, &["crates/probe/src/lib.rs".to_owned()]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].starts_with("warning: unused variable: `x`\n --> crates/probe/src/lib.rs:2:9"), "{}", hits[0]);
    assert!(hits[0].ends_with('^'), "the paragraph is whole: {}", hits[0]);
    assert!(warnings_at(CLIPPY, &["crates/probe/src/other.rs".to_owned()]).is_empty());
    // a Windows path from git and a cargo path meet on their separators
    assert_eq!(warnings_at(CLIPPY, &["crates\\other\\src\\lib.rs".to_owned()]).len(), 1);
}

#[test]
fn crate_of_is_the_nearest_package_above_the_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("crates/probe/src")).unwrap();
    std::fs::write(
        dir.path().join("crates/probe/Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\n\n[dependencies]\nname = \"not me\"\n",
    )
    .unwrap();
    assert_eq!(crate_of(dir.path(), "crates/probe/src/lib.rs").as_deref(), Some("probe"));
    assert_eq!(crate_of(dir.path(), "README.md"), None, "the workspace root is no package");
}

#[tokio::test]
async fn a_scripted_toolbox_answers_a_command_from_the_script() {
    let (_dir, tools) = workspace().await;
    let tools =
        tools.with_script(Arc::new(|command: &str| Ok((if command == "cargo test" { 0 } else { 101 }, format!("ran {command}\n")))));
    assert_eq!(tools.check("cargo test").await.unwrap(), (true, "ran cargo test\nexit code 0".to_owned()));
    assert!(!tools.check("cargo clippy").await.unwrap().0, "exit code 101 is a failed check");
    assert!(tools.check("rm -rf /").await.is_err(), "the allowlist comes before the script");
    // an acceptance's own check is the person's command: it runs past the allowlist
    assert_eq!(tools.check_acceptance("python tools/x.py").await.unwrap(), (false, "ran python tools/x.py\nexit code 101".to_owned()));
}

#[tokio::test]
async fn a_command_past_the_toolboxs_limit_is_stopped_and_names_the_limit() {
    let (_dir, tools) = workspace().await;
    // the command splits on whitespace, so the program sleeps without a space
    let sleep = "python -c __import__('time').sleep(3)";
    let error = tools.clone().with_timeout(Duration::from_secs(1)).check_acceptance(&sleep).await.unwrap_err();
    assert!(error.to_string().contains("ran past 1 s and was stopped"), "{error:#}");
    // under a longer limit the same command finishes
    let tools = tools.with_timeout(Duration::from_secs(1800));
    assert_eq!(tools.timeout(), Duration::from_secs(1800));
    assert!(tools.check_acceptance(&sleep.replace('3', "0")).await.unwrap().0);
}

#[test]
fn the_changed_ranges_come_from_the_hunks_and_only_their_warnings_count() {
    let diff = "diff --git a/crates/probe/src/lib.rs b/crates/probe/src/lib.rs\n--- a/crates/probe/src/lib.rs\n+++ b/crates/probe/src/lib.rs\n@@ -1,0 +2,3 @@\n+a\n+b\n+c\n@@ -40,2 +43 @@\n-x\n-y\n+z\n@@ -60,1 +62,0 @@\n-gone\ndiff --git a/new.rs b/new.rs\n--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1,2 @@\n+n\n+m\n";
    assert_eq!(
        changed_ranges(diff),
        [
            ("crates/probe/src/lib.rs".to_owned(), 2, 4),
            ("crates/probe/src/lib.rs".to_owned(), 43, 43),
            ("crates/probe/src/lib.rs".to_owned(), 62, 62),
            ("new.rs".to_owned(), 1, 2),
        ]
    );
    // the warning at line 2 is in a hunk; the one at line 9 of another file is not
    let ranges = changed_ranges(diff);
    let hits = warnings_within(CLIPPY, &ranges);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].contains("lib.rs:2:9"));
    // the same warning outside every hunk stays out, slack included
    assert!(warnings_within(CLIPPY, &[("crates/probe/src/lib.rs".to_owned(), 10, 20)]).is_empty());
    assert_eq!(warnings_within(CLIPPY, &[("crates/probe/src/lib.rs".to_owned(), 4, 20)]).len(), 1, "two lines of slack");
}
