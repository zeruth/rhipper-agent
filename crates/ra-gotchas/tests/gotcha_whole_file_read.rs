//! A whole read of any file over [`LONG_FILE`] lines is refused, named by the card or not, and
//! a shell's `cat` or `Get-Content` of a whole file is that read too.

use ra_gotchas::{Catalogue, LONG_FILE, ReadCall, Verdict, whole_read};

#[test]
fn a_long_file_no_outline_names_is_refused_and_its_outline_named_when_known() {
    let catalogue = Catalogue::seed();
    let unknown = ReadCall { file: "src/psp/prx.rs", ranged: false, lines: LONG_FILE + 1, items: None };
    let Verdict::Deny { id, reason } = catalogue.read(unknown) else { panic!("a long file the card does not name is refused") };
    assert_eq!(id, "whole-file-read");
    assert!(!reason.contains("its items"), "{reason}");
    let items = ["parse_prx 12-90".to_owned()];
    let Verdict::Deny { reason, .. } = catalogue.read(ReadCall { items: Some(&items), ..unknown }) else { panic!("refused") };
    assert!(reason.contains("its items: parse_prx 12-90"), "{reason}");
    assert_eq!(catalogue.read(ReadCall { lines: LONG_FILE, ..unknown }), Verdict::Pass);
}

#[test]
fn a_shell_print_of_a_whole_file_is_a_whole_read() {
    assert_eq!(whole_read("cat crates/x/src/lib.rs").as_deref(), Some("crates/x/src/lib.rs"));
    assert_eq!(whole_read("Get-Content -Path crates\\x\\src\\lib.rs").as_deref(), Some("crates\\x\\src\\lib.rs"));
    assert_eq!(whole_read("Get-Content crates/x/src/lib.rs | Select-Object -First 40"), None);
    assert_eq!(whole_read("Get-Content crates/x/src/lib.rs -TotalCount 40"), None);
}
