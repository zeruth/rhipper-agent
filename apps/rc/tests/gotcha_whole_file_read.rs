//! A whole-file Read of a long file is refused with a gotcha naming `offset` and `limit`.

use ra_gotchas::{Catalogue, LONG_FILE, ReadCall, Verdict};

fn call(lines: usize, ranged: bool, items: Option<&[String]>) -> ReadCall<'_> {
    ReadCall { file: "apps/rc/src/main.rs", ranged, lines, items }
}

#[test]
fn a_whole_read_of_a_large_file_names_its_line_count_and_the_ranged_form() {
    let items: Vec<String> = Vec::new();
    let Verdict::Deny { id, reason } = Catalogue::seed().read(call(2253, false, Some(&items))) else {
        panic!("a whole read of a large file is refused");
    };
    assert_eq!(id, "whole-file-read");
    assert!(reason.contains("2253 lines"), "{reason}");
    assert!(reason.contains("offset"), "{reason}");
    assert!(reason.contains("limit"), "{reason}");
}

#[test]
fn a_ranged_read_and_a_small_file_pass() {
    let items: Vec<String> = Vec::new();
    let catalogue = Catalogue::seed();
    assert_eq!(catalogue.read(call(2253, true, Some(&items))), Verdict::Pass);
    assert_eq!(catalogue.read(call(LONG_FILE, false, Some(&items))), Verdict::Pass);
}
