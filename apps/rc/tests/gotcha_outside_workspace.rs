//! A refused `ls` of a run's scratch directory outside the workspace names the `outside-workspace`
//! gotcha, which says the run's files are under `var/`; an `ls` inside the workspace does not.

use ra_gotchas::{Catalogue, Conditions, Verdict};

#[test]
fn an_ls_of_a_scratch_run_directory_matches_the_gotcha() {
    let catalogue = Catalogue::seed();
    for command in ["ls -la D:/rhipper/tmp/rhipper-asm/run-209", "cat /d/rhipper/tmp/rhipper-asm/run-209/log.txt"] {
        let id = catalogue.matching(command).map(|g| g.id.as_str());
        assert_eq!(id, Some("outside-workspace"), "{command}");
    }
}

#[test]
fn the_gotcha_says_the_runs_files_are_under_var() {
    let catalogue = Catalogue::seed();
    let Verdict::Deny { id, reason } = catalogue.command("ls -la D:/rhipper/tmp/rhipper-asm/run-209", Conditions::default()) else {
        panic!("the ls is refused with a hint");
    };
    assert_eq!(id, "outside-workspace");
    assert!(reason.contains("var/"), "{reason}");
}

#[test]
fn an_ls_inside_the_workspace_is_not_named() {
    let catalogue = Catalogue::seed();
    for command in ["ls -la var/", "ls crates/ra-gotchas/src", "cat var/notes.txt"] {
        let id = catalogue.matching(command).map(|g| g.id.as_str());
        assert_ne!(id, Some("outside-workspace"), "{command}");
    }
}
