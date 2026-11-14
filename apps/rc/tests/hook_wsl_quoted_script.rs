//! A script quoted to `wsl -- bash -lc` is one command of WSL's: the hook judges it whole, never
//! by the `;` and `|` inside the quotes. Where the project allows WSL the whole script runs, as
//! written; where it does not, the whole command is refused with the form to use instead.

use std::path::Path;

use ra_gotchas::shape::{self, Shell};
use ra_gotchas::{Answer, Catalogue, Conditions};

/// The hook's answer to `command` in Bash, as `rc hook pre` composes it: the catalogue's verdict
/// and the command's shape, with WSL the project's own when `allowed`.
fn answer(command: &str, allowed: bool) -> Answer {
    let catalogue = Catalogue::seed();
    let verdict = catalogue.command(command, Conditions { allowed, ..Conditions::default() });
    let own = |part: &str| allowed && part.trim_start().starts_with("wsl ");
    Answer::of(verdict, shape::judge(command, Shell::Bash, Path::new("."), &own))
}

#[test]
fn a_quoted_script_runs_whole_where_the_project_allows_wsl() {
    // a chain, a pipe, a git write and a kill, all WSL's; the newline and the double space stay
    let command = "wsl -d Ubuntu-24.04 -- bash -lc \"cd testdata/psx && make;  git add -n . | tail -n 3\ntaskkill x\"";
    match answer(command, true) {
        Answer::Rewrite { command: rewritten, .. } => assert_eq!(rewritten, format!("rc hold -- {command}")),
        other => panic!("{command}: {other:?}"),
    }
}

#[test]
fn a_quoted_script_is_refused_whole_where_wsl_is_not_allowed() {
    let command = "wsl -- bash -lc 'make -C testdata; ls build | tail -n 5'";
    match answer(command, false) {
        Answer::Deny { id, reason } => {
            assert_eq!(id, "wsl-quoted-script");
            assert!(reason.contains("var/") && reason.contains("by path"), "{reason}");
        }
        other => panic!("{command}: {other:?}"),
    }
}

#[test]
fn the_command_around_the_script_is_still_judged() {
    // a variable and a loop in the script are WSL's, the tail after the quotes is the hook's
    let command = "wsl -e sh -c \"for f in *.s; do echo $f; done; echo $?\" 2>&1 | tail -n 20";
    match answer(command, true) {
        Answer::Rewrite { command: rewritten, .. } => assert_eq!(rewritten, format!("rc hold -- {command}")),
        other => panic!("{command}: {other:?}"),
    }
    // a refused part outside the quotes still refuses the whole
    let chained = "wsl -- bash -lc \"make\"; git commit -m x";
    assert!(matches!(answer(chained, true), Answer::Deny { .. }), "{chained}");
}
