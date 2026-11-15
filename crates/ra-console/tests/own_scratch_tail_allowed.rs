//! A worker may `tail`, `head` or `cat` a file under its own run's scratch directory, and no
//! other run's.

use ra_gotchas::shape::{Shape, Shell, judge};
use std::path::Path;

#[test]
fn a_worker_reads_its_own_runs_scratch_and_no_other() {
    let scratch = std::env::temp_dir().join("own-scratch").join("tmp").join("proj");
    let own = scratch.join("run-797");
    let other = scratch.join("run-796");
    // SAFETY: the only test of this binary, so no other thread reads the environment
    unsafe { std::env::set_var("TEMP", &own) };
    let workspace = Path::new("workspace-root");
    let deny = |_: &str| false;
    let say = |dir: &Path, program: &str| {
        let file = dir.join("task.output").to_string_lossy().replace('\\', "/");
        judge(&format!("{program} -n 60 {file}"), Shell::Bash, workspace, &deny)
    };
    for program in ["tail", "head", "cat"] {
        assert_eq!(say(&own, program), Shape::Allowed, "{program} of the run's own scratch");
        assert_eq!(say(&other, program), Shape::Unknown, "{program} of another run's scratch");
    }
}
