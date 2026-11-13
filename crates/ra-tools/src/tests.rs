//! The tests of the toolbox: each tool over a temp workspace, the confinement and the allowlist.

use super::commands::{MAX_LINES, MAX_OUTPUT, clip, split};
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
fn a_quoted_argument_stays_one_argument() {
    assert_eq!(split(r#"python -c "import time; time.sleep(9)""#).unwrap(), ["python", "-c", "import time; time.sleep(9)"]);
    assert_eq!(split("git log --format='%h %s' -n 1").unwrap(), ["git", "log", "--format=%h %s", "-n", "1"]);
    assert_eq!(split(r#"echo "a \"b\" c" '' d\ e"#).unwrap(), ["echo", "a \"b\" c", "", "d e"]);
    assert_eq!(split("  cargo   test  ").unwrap(), ["cargo", "test"]);
    let err = split(r#"python -c "import time"#).unwrap_err().to_string();
    assert!(err.contains("unbalanced \" quote"), "{err}");
}

#[test]
fn a_backslash_before_a_path_character_is_the_separator() {
    // runelite #9: `.\gw.ps1 :android-awt:stubReport` reached powershell as `-File .gw.ps1`
    assert_eq!(split(r".\gw.ps1 :android-awt:stubReport").unwrap(), [r".\gw.ps1", ":android-awt:stubReport"]);
    assert_eq!(split(r"python tools\run.py C:\Users\me\a.txt").unwrap(), ["python", r"tools\run.py", r"C:\Users\me\a.txt"]);
    assert_eq!(split(r"echo d\ e a\\b").unwrap(), ["echo", "d e", r"a\b"]);
    let err = split(r"echo a\").unwrap_err().to_string();
    assert!(err.contains("ends in a backslash"), "{err}");
}

#[tokio::test]
async fn a_check_passed_as_words_keeps_its_backslash() {
    let (_dir, tools) = workspace().await;
    let seen = Arc::new(std::sync::Mutex::new(String::new()));
    let log = seen.clone();
    let tools = tools.with_script(Arc::new(move |line: &str| {
        line.clone_into(&mut log.lock().unwrap());
        Ok((0, "ok\n".to_owned()))
    }));
    let words = words(r".\gw.ps1 :contract").unwrap();
    assert_eq!(words, [r".\gw.ps1", ":contract"]);
    let (passed, out) = tools.check_args(&words).await.unwrap();
    assert!(passed, "{out}");
    assert_eq!(*seen.lock().unwrap(), r".\gw.ps1 :contract");
    assert!(tools.check_args(&[]).await.unwrap_err().to_string().contains("empty"));
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
    assert!(out.contains("… 801 lines dropped …"), "{out}");
    assert!(out.starts_with("   Compiling crate-1 v0.1.0\n"), "the head stays");
    assert!(out.ends_with("   Compiling crate-1000 v0.1.0\nexit code 0"), "the tail and the verdict stay");
    // a check is the lint's, not a turn's: it reads the whole output
    assert_eq!(tools.check("cargo build").await.unwrap().1.lines().count(), 1001);
}

#[tokio::test]
async fn a_heavy_tool_result_is_clipped_to_its_head_and_tail_with_the_dropped_count() {
    let (_dir, tools) = workspace().await;
    let log: String = (1..=5000).map(|n| format!("test case_{n} ... ok\n")).collect();
    let tools = tools.with_script(Arc::new(move |_: &str| Ok((0, log.clone()))));
    let out = tools.execute(&call("run", json!({"command": "cargo test"}))).await;
    // the 5,000 lines and the exit line, less the 200 the marker stands between
    assert!(out.contains("… 4801 lines dropped …"), "{out}");
    assert!(out.lines().count() <= MAX_LINES + 1, "{} lines", out.lines().count());
    assert!(out.chars().count() <= MAX_OUTPUT + 1, "{} chars", out.chars().count());
    assert!(out.starts_with("test case_1 ... ok\n"), "the head stays");
    assert!(out.ends_with("test case_5000 ... ok\nexit code 0"), "the tail and the verdict stay");
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
    let (dir, tools) = workspace().await;
    // the quoted program is one argument, so the script keeps its spaces; the child blocks on an
    // event nothing sets, so only the limit ends it, and what it printed before the limit stays
    let pid_file = dir.path().join("child.pid");
    let pid_path = pid_file.display().to_string().replace('\\', "/");
    let block = format!(
        r#"python -c "import os, threading; open(r'{pid_path}', 'w').write(str(os.getpid())); print('started', flush=True); threading.Event().wait()""#
    );
    let (passed, text) = tools.clone().with_timeout(Duration::from_secs(1)).check_acceptance(&block).await.unwrap();
    assert!(!passed);
    assert_eq!(text.replace("\r\n", "\n"), "started\ntimed out after 1s");
    let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
    assert!(process_is_gone(pid), "the child {pid} outlived the limit");
    // under a longer limit a command that ends finishes
    let tools = tools.with_timeout(Duration::from_secs(1800));
    assert_eq!(tools.timeout(), Duration::from_secs(1800));
    assert!(tools.check_acceptance(r#"python -c "print('ok')""#).await.unwrap().0);
}

/// Whether no process answers to `pid` any more.
fn process_is_gone(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().unwrap();
        !String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    }
    #[cfg(not(windows))]
    {
        !std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().is_ok_and(|s| s.success())
    }
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
