//! A `clang-cl` piped into `grep`, `head` or `tail` is refused with the form that passes for a
//! tool without output flags: the command alone, its output kept under `var/`.

use ra_gotchas::{Catalogue, Conditions, Verdict};

#[test]
fn a_piped_clang_cl_is_told_to_run_alone_and_grep_the_kept_file() {
    let alone = "clang-cl -fdump-record-layouts x.cpp";
    for filter in ["grep Foo", "head -n 40", "tail -n 40"] {
        let command = format!("{alone} 2>&1 | {filter}");
        let Verdict::Deny { id, reason } = Catalogue::seed().command(&command, Conditions::default()) else {
            panic!("{command} is refused with a hint");
        };
        assert_eq!(id, "piped-or-chained", "{command}");
        assert!(reason.contains(&format!("`{alone}`")), "{reason}");
        assert!(reason.contains("var/"), "{reason}");
        assert!(reason.contains("Grep"), "{reason}");
        assert!(!reason.contains("own flags"), "{reason}");
    }
}
