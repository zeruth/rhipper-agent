//! `llvm-objdump -s` ignores the address flags; the gotcha names the grep form to use instead.

use ra_gotchas::{Catalogue, Conditions, Verdict};

#[test]
fn an_objdump_s_with_an_address_matches_its_gotcha() {
    let catalogue = Catalogue::seed();
    for command in [
        "llvm-objdump -s --start-address=0x10066b90 --stop-address=0x10066bb0 a.exe",
        "llvm-objdump --full-contents --start-address 0x10066b90 a.exe",
        "llvm-objdump --stop-address=0x10 -s a.exe",
    ] {
        let id = catalogue.matching(command).map(|g| g.id.as_str());
        assert_eq!(id, Some("objdump-s-address"), "{command}");
    }
}

#[test]
fn the_gotcha_names_the_grep_form() {
    let catalogue = Catalogue::seed();
    let command = "llvm-objdump -s --start-address=0x10066b90 a.exe";
    let Verdict::Deny { id, reason } = catalogue.command(command, Conditions::default()) else {
        panic!("the command is refused with a hint");
    };
    assert_eq!(id, "objdump-s-address");
    assert!(reason.contains("ignored"), "{reason}");
    assert!(reason.contains("grep -A2 \"^ 10066b90\""), "{reason}");
}

#[test]
fn an_objdump_d_with_an_address_is_not_named() {
    let catalogue = Catalogue::seed();
    for command in ["llvm-objdump -d --start-address=0x10066b90 a.exe", "llvm-objdump -s a.exe"] {
        let id = catalogue.matching(command).map(|g| g.id.as_str());
        assert_ne!(id, Some("objdump-s-address"), "{command}");
    }
}
