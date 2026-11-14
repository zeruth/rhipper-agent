//! A refused `Test-Path`, `Get-ChildItem`, `gci` or `dir` of a path outside the worktree names the
//! same gotcha as a refused `ls` there, so it is not left unmatched.

use ra_gotchas::{Catalogue, Conditions, Verdict};

const OUTSIDE: [&str; 3] = ["../../target/debug/ras.exe", "C:/Windows/SysWOW64/msvcrt.dll", "../../target/debug"];

#[test]
fn a_test_path_outside_the_worktree_matches_the_ls_gotcha() {
    let catalogue = Catalogue::seed();
    for path in OUTSIDE {
        let command = format!("Test-Path {path}");
        let id = catalogue.matching(&command).map(|g| g.id.as_str());
        assert_eq!(id, Some("ls-outside-worktree"), "{command}");
    }
}

#[test]
fn a_listing_form_outside_the_worktree_matches_the_ls_gotcha() {
    let catalogue = Catalogue::seed();
    for form in ["Get-ChildItem", "gci", "dir"] {
        for path in OUTSIDE {
            let command = format!("{form} {path}");
            let id = catalogue.matching(&command).map(|g| g.id.as_str());
            assert_eq!(id, Some("ls-outside-worktree"), "{command}");
        }
    }
}

#[test]
fn the_powershell_forms_print_the_same_advice_as_ls() {
    let catalogue = Catalogue::seed();
    let Verdict::Deny { reason: expected, .. } = catalogue.command("ls ../../target/debug/ras.exe", Conditions::default()) else {
        panic!("the ls is refused with a hint");
    };
    for command in ["Test-Path ../../target/debug/ras.exe", "Get-ChildItem C:/Windows/SysWOW64", "gci ../../target", "dir ../.."] {
        let Verdict::Deny { id, reason } = catalogue.command(command, Conditions::default()) else {
            panic!("{command} is refused with a hint");
        };
        assert_eq!(id, "ls-outside-worktree", "{command}");
        assert_eq!(reason, expected, "{command}");
    }
}

#[test]
fn a_powershell_form_inside_the_worktree_is_not_named() {
    let catalogue = Catalogue::seed();
    for command in ["Test-Path target/debug", "Get-ChildItem crates/ra-gotchas/src", "gci apps/rc/tests", "dir crates"] {
        let id = catalogue.matching(command).map(|g| g.id.as_str());
        assert_ne!(id, Some("ls-outside-worktree"), "{command}");
    }
}
