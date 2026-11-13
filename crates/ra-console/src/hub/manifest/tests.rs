//! The tests of the machine's manifest: a probe of fake tools, the file it keeps, the lines a
//! card's prompt gets, the run's PATH, and the build budget the card that brought the manifest
//! put right. No test probes the machine itself.

use super::*;

/// A directory holding an empty file for each of `names`, as the probe resolves them.
fn fake_tools(dir: &Path, names: &[&str]) {
    std::fs::create_dir_all(dir).unwrap();
    let ext = if cfg!(windows) { ".exe" } else { "" };
    for name in names {
        std::fs::write(dir.join(format!("{name}{ext}")), "").unwrap();
    }
}

/// What the fake tools print: a version for each, and clang's back ends.
fn fake_ask(program: &Path, args: &[&str]) -> Option<String> {
    let name = program.file_stem()?.to_string_lossy().into_owned();
    match args {
        ["--version"] => Some(format!("{name} version 9.9.9\nmore text")),
        ["-print-targets"] => Some(String::from("  Registered Targets:\n    x86    - 32-bit X86\n    mips   - MIPS (32-bit big endian)\n")),
        _ => None,
    }
}

fn fake_manifest(base: &Path) -> Manifest {
    fake_tools(&base.join("llvm"), &["clang"]);
    fake_tools(&base.join("tools"), &["ninja", "rc"]);
    let names: Vec<String> = ["clang", "ninja", "rc", "nowhere"].map(str::to_owned).to_vec();
    probe_with(&names, &[base.join("tools"), base.join("llvm")], &fake_ask, "2026-11-12")
}

#[test]
fn a_manifest_of_fake_tools_resolves_each_and_reads_its_version_and_targets() {
    let base = tempfile::tempdir().unwrap();
    let manifest = fake_manifest(base.path());
    let clang = manifest.tool("clang").unwrap();
    assert!(clang.path.as_ref().unwrap().starts_with(base.path().join("llvm")));
    assert_eq!(clang.version, "clang version 9.9.9");
    assert_eq!(clang.targets, vec!["mips", "x86"]);
    assert!(manifest.tool("nowhere").unwrap().path.is_none());
    // the file reads back as it was written
    save_in(base.path(), &manifest).unwrap();
    let back = load_in(base.path()).unwrap();
    assert_eq!(Manifest { sdks: Vec::new(), ..back }, Manifest { sdks: Vec::new(), ..manifest.clone() });
    assert!(manifest.lines().iter().any(|l| l == "nowhere: not found on this machine"));
}

#[test]
fn the_manifest_directories_of_fake_tools_reach_the_run_s_path_once_each() {
    let base = tempfile::tempdir().unwrap();
    let manifest = fake_manifest(base.path());
    let dirs = manifest.dirs();
    assert_eq!(dirs, vec![base.path().join("llvm"), base.path().join("tools")]);
    let harness = ra_providers::claude_code::ClaudeCode::new().with_tool_dirs(dirs.clone());
    let env = harness.run_env(None);
    let path = env.iter().find(|(k, _)| k == "PATH").map(|(_, v)| v.clone()).unwrap();
    let on_path: Vec<PathBuf> = std::env::split_paths(&path).collect();
    for dir in &dirs {
        assert_eq!(on_path.iter().filter(|d| *d == dir).count(), 1, "{} on {on_path:?}", dir.display());
    }
    // the shell's own tools still come first
    let first_tool = on_path.iter().position(|d| d == &dirs[0]).unwrap();
    assert!(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).count() <= first_tool);
    if cfg!(windows) {
        assert!(env.iter().any(|(k, v)| k == "MSYS_NO_PATHCONV" && v == "1"));
    }
}

#[test]
fn the_manifest_lines_of_the_tools_a_card_names_reach_its_prompt_and_no_others() {
    let base = tempfile::tempdir().unwrap();
    let manifest = fake_manifest(base.path());
    let mut task = ra_context::TaskContext::open(base.path(), 7);
    task.brief = String::from("Task #7: compile the MIPS sample with clang, then run it");
    let messages = crate::agent::harness_messages(&task, None, None, "", base.path(), false, Some(&manifest));
    let system: String = messages.iter().filter(|m| m.role == "system").map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n");
    let clang = manifest.tool("clang").unwrap();
    assert!(system.contains(&format!("- clang: {}", clang.path.as_ref().unwrap().display())), "{system}");
    assert!(system.contains("targets: mips, x86"), "{system}");
    assert!(!system.contains("- ninja:"), "{system}");
    // a card that names no tool carries no block
    assert_eq!(manifest.block_for("Task #8: reword the readme"), "");
}

#[test]
fn a_manifest_names_the_project_s_own_programs_but_not_its_scripts() {
    assert_eq!(program_of("Bash(ninja *)").as_deref(), Some("ninja"));
    assert_eq!(program_of("rhtest").as_deref(), Some("rhtest"));
    assert_eq!(program_of("PowerShell(.\\gw.ps1 *)"), None);
    assert_eq!(program_of("& .\\gw.ps1 :core:test"), None);
}

#[test]
fn a_refused_build_spends_nothing_of_the_session_budget_the_manifest_card_set() {
    let tree = tempfile::tempdir().unwrap();
    // a tree with a change, so the budget and not the empty tree is what answers
    let init = std::process::Command::new("git").args(["init", "-q"]).current_dir(tree.path()).status();
    assert!(init.is_ok_and(|s| s.success()));
    std::fs::write(tree.path().join("changed.rs"), "fn main() {}\n").unwrap();
    let build = "rc hold -- cargo test -p ra-console";
    let refused = ra_gotchas::Verdict::Deny { id: String::from("shell-shape"), reason: String::from("no") };
    // refused by the catalogue, and no build at all: neither spends
    assert_eq!(crate::agent::budget_build(tree.path(), "s1", build, refused.clone()), refused);
    assert_eq!(crate::agent::budget_build(tree.path(), "s1", "git status", ra_gotchas::Verdict::Pass), ra_gotchas::Verdict::Pass);
    assert_eq!(crate::agent::builds_spent(tree.path(), "s1"), 0);
    // the builds that start spend the session's two, and the third is refused
    for spent in 1..=crate::agent::BUILDS_A_SESSION {
        assert_eq!(crate::agent::budget_build(tree.path(), "s1", build, ra_gotchas::Verdict::Pass), ra_gotchas::Verdict::Pass);
        assert_eq!(crate::agent::builds_spent(tree.path(), "s1"), spent);
    }
    let over = crate::agent::budget_build(tree.path(), "s1", build, ra_gotchas::Verdict::Pass);
    assert!(matches!(over, ra_gotchas::Verdict::Deny { ref id, .. } if id == "build-budget"));
    assert_eq!(crate::agent::builds_spent(tree.path(), "s1"), crate::agent::BUILDS_A_SESSION);
    // the budget is the session's: another session of the tree has its own
    assert_eq!(crate::agent::budget_build(tree.path(), "s2", build, ra_gotchas::Verdict::Pass), ra_gotchas::Verdict::Pass);
}
