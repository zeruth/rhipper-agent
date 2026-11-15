//! An `ls` of the project's own tmp directory is allowed; any other absolute path outside the
//! worktree names the `ls-outside-worktree` gotcha, in both drive spellings.

use std::path::Path;

use ra_gotchas::Catalogue;
use ra_gotchas::shape::{Shape, Shell, judge_in};

const TMP: &str = "D:/rhipper/tmp/rhipper-agent";

fn shape(command: &str) -> Shape {
    judge_in(command, Shell::Bash, Path::new("C:/work/wt"), &|_| false, Some(Path::new(TMP)))
}

#[test]
fn an_ls_of_the_project_tmp_is_allowed_in_both_spellings() {
    for command in [
        "ls D:/rhipper/tmp/rhipper-agent/run-732",
        "ls -la /d/rhipper/tmp/rhipper-agent",
        "ls D:/rhipper/tmp/rhipper-agent/run-1 /d/rhipper/tmp/rhipper-agent/run-2",
    ] {
        assert!(matches!(shape(command), Shape::Allowed), "{command}");
    }
}

#[test]
fn an_ls_of_any_other_outside_path_matches_the_gotcha() {
    let catalogue = Catalogue::seed();
    for command in [
        "ls D:/other/place",
        "ls /d/other/place",
        "ls -la C:/Users/tyler",
        "ls /c/Users/tyler /d/other/place",
        "ls D:/other/place D:/rhipper/tmp/rhipper-agent",
    ] {
        let id = catalogue.matching(command).map(|g| g.id.as_str());
        assert_eq!(id, Some("ls-outside-worktree"), "{command}");
        assert!(!matches!(shape(command), Shape::Allowed), "{command}");
    }
}
