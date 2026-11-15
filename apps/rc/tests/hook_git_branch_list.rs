//! `git branch` with only listing flags passes the shell gate; a form that creates, deletes or
//! renames a branch stays refused.

use std::path::Path;

use ra_gotchas::shape::{Shape, Shell, judge};

fn gate(command: &str, shell: Shell) -> Shape {
    judge(command, shell, Path::new("."), &|_| false)
}

#[test]
fn a_branch_listing_is_allowed() {
    for command in [
        "git branch",
        "git branch -a",
        "git branch -r",
        "git branch -v",
        "git branch --list",
        "git branch --list 'card/*'",
        "git branch -a --format=%(refname:short)",
        "git branch -a --format='%(refname:short)'",
        "git branch --format=%(HEAD)%(refname:short) -r",
        "git branch --format=refname -a",
        "git branch --contains HEAD",
        "git branch --merged main",
    ] {
        assert_eq!(gate(command, Shell::Bash), Shape::Allowed, "{command}");
        assert_eq!(gate(command, Shell::PowerShell), Shape::Allowed, "{command}");
    }
}

#[test]
fn a_branch_that_creates_deletes_or_renames_stays_refused() {
    for command in
        ["git branch -D x", "git branch -d x", "git branch new", "git branch -m old new", "git branch -a new", "git branch -f x HEAD"]
    {
        assert!(matches!(gate(command, Shell::Bash), Shape::Denied(_)), "{command}");
    }
}

#[test]
fn an_unquoted_group_that_is_no_format_atom_is_not_allowed() {
    // PowerShell runs an unquoted `(calc)` as a command
    for command in ["git branch --format=%(calc)", "git branch -a --format=%(refname:short", "git branch -a x(refname)"] {
        assert_ne!(gate(command, Shell::PowerShell), Shape::Allowed, "{command}");
        assert_ne!(gate(command, Shell::Bash), Shape::Allowed, "{command}");
    }
}
