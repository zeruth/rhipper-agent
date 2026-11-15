//! The session may list and run from its worktree's cargo target and its kept scratch, and the
//! handoff names both paths.

use std::path::Path;

use ra_gotchas::shape::{Shape, Shell, judge_in};
use ra_providers::claude_code::{session_dirs_note, shared_target};

#[test]
fn the_handoff_names_the_target_and_the_kept_scratch_of_a_worktree() {
    let worktree = Path::new("var").join("worktrees").join("711");
    let note = session_dirs_note(&worktree);
    let target = shared_target(&worktree).unwrap();
    assert!(note.contains(&target.display().to_string()), "{note}");
    assert!(note.contains("run-711"), "{note}");
}

#[test]
fn a_workspace_that_is_no_worktree_has_no_such_note() {
    assert_eq!(session_dirs_note(Path::new("project")), "");
}

#[test]
fn the_target_and_the_scratch_can_be_listed_and_run() {
    for root in ["D:/rhipper/target/rhipper-agent/wt-711", "D:/rhipper/tmp/rhipper-agent/run-711"] {
        for command in [format!("ls {root}"), format!("{root}/debug/rc.exe --version")] {
            let shape = judge_in(&command, Shell::Bash, Path::new("C:/work/wt"), &|_| false, Some(Path::new(root)));
            assert_eq!(shape, Shape::Allowed, "{command}");
        }
    }
}
