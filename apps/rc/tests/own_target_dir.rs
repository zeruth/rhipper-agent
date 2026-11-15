//! A card's own target dir counts as inside the workspace for listing and running what is built
//! there; another card's stays outside it.

use std::path::Path;

use ra_gotchas::Catalogue;
use ra_gotchas::shape::{Shape, Shell, judge_in};

const OWN: &str = "D:/rhipper/target/project/wt-265";

fn judged(command: &str, shell: Shell) -> Shape {
    judge_in(command, shell, Path::new("C:/work/wt"), &|_| false, Some(Path::new(OWN)))
}

#[test]
fn card_265_may_run_and_list_what_its_own_target_dir_holds() {
    let run = "D:/rhipper/target/project/wt-265/debug/ras.exe emit x --out out/y --quiet";
    assert_eq!(judged(run, Shell::Bash), Shape::Allowed, "{run}");
    let list = r"Get-ChildItem D:\rhipper\target\project\wt-265";
    assert_eq!(judged(list, Shell::PowerShell), Shape::Allowed, "{list}");
}

#[test]
fn card_265_may_not_list_card_238s_target_dir() {
    let list = r"Get-ChildItem D:\rhipper\target\project\wt-238";
    assert_ne!(judged(list, Shell::PowerShell), Shape::Allowed, "{list}");
    // a listing meets the listing's own outside-path gotcha, ahead of the general one
    let gotcha = Catalogue::seed().matching(list).map(|g| g.id.clone());
    assert!(matches!(gotcha.as_deref(), Some("ls-outside-worktree" | "path-outside-workspace")), "{list}: {gotcha:?}");
    let run = "D:/rhipper/target/project/wt-238/debug/ras.exe emit x --out out/y --quiet";
    assert_ne!(judged(run, Shell::Bash), Shape::Allowed, "{run}");
    let gotcha = Catalogue::seed().matching(run).map(|g| g.id.clone());
    assert_eq!(gotcha.as_deref(), Some("path-outside-workspace"), "{run}");
}
