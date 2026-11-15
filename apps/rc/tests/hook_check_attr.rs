//! `git check-attr` reads attributes and nothing else: both shells' allowlists pass it.

use std::path::Path;

use ra_gotchas::shape::{Shape, Shell, judge_in};

fn shape(command: &str, shell: Shell) -> Shape {
    judge_in(command, shell, Path::new("C:/work/wt"), &|_| false, None)
}

#[test]
fn a_check_attr_of_a_path_is_allowed_in_both_shells() {
    for shell in [Shell::Bash, Shell::PowerShell] {
        for command in ["git check-attr diff -- apps/rc/src/main.rs", "git check-attr -a -- Cargo.toml"] {
            assert!(matches!(shape(command, shell), Shape::Allowed), "{command}");
        }
    }
}
