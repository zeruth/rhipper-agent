//! A listing of the running project's target root passes the shape judge; running from it, writing
//! there and another project's target stay refused.

use std::path::{Path, PathBuf};

use ra_gotchas::shape::{Shape, Shell, judge_reading};

fn judged(command: &str, shell: Shell) -> Shape {
    let own = vec![PathBuf::from("D:/rhipper/target/project/wt-805")];
    let reads = vec![PathBuf::from("D:/rhipper/target/project")];
    judge_reading(command, shell, Path::new("C:/work/wt"), &|_| false, &own, &reads)
}

#[test]
fn ls_own_target_root_lists_but_does_not_run_or_write() {
    for command in ["ls D:/rhipper/target/project/wt-804/debug", "ls D:/rhipper/target/project/wt-805/debug"] {
        assert_eq!(judged(command, Shell::Bash), Shape::Allowed, "{command}");
    }
    let test = r"Test-Path D:\rhipper\target\project\wt-804\debug\rc.exe";
    assert_eq!(judged(test, Shell::PowerShell), Shape::Allowed, "{test}");
    for command in [
        "ls D:/rhipper/target/other/wt-1/debug",
        "D:/rhipper/target/project/wt-804/debug/rc.exe --version",
        "cat D:/rhipper/target/project/wt-804/debug/x.txt",
        "ls D:/rhipper/target/project/wt-804 > D:/rhipper/target/project/out.txt",
    ] {
        assert_ne!(judged(command, Shell::Bash), Shape::Allowed, "{command}");
    }
}
