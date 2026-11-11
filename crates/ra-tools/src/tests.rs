//! The tests of the toolbox: each tool over a temp workspace, the confinement and the allowlist.

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

#[test]
fn the_specs_name_every_tool_once() {
    let names: Vec<String> = Toolbox::specs().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["read_file", "write_file", "edit_file", "list_dir", "search", "run"]);
}
