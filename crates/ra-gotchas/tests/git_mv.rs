use ra_gotchas::shape::{Shape, Shell, judge};
use std::path::Path;

fn judged(command: &str) -> Shape {
    judge(command, Shell::Bash, Path::new("."), &|_| false)
}

#[test]
fn git_mv_inside_the_workspace_is_allowed() {
    assert_eq!(judged("git mv src/a.rs src/b.rs"), Shape::Allowed);
}

#[test]
fn git_mv_out_of_the_workspace_is_refused_naming_the_path() {
    for (command, path) in [("git mv a.rs ../b.rs", "../b.rs"), ("git mv /etc/a.rs b.rs", "/etc/a.rs")] {
        match judged(command) {
            Shape::Denied(reason) => assert!(reason.contains(path), "{reason}"),
            other => panic!("{command}: {other:?}"),
        }
    }
}
