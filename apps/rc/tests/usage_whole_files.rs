//! The usage line names the files read whole with their line counts, largest first, at most three.

use ra_console::agent::largest_whole_files;

fn file(path: &str, lines: u64) -> (String, u64) {
    (path.to_owned(), lines)
}

#[test]
fn the_usage_line_names_the_three_largest_whole_files() {
    let files = [file("src/mid.rs", 300), file("src/big.rs", 2400), file("src/small.rs", 20), file("src/next.rs", 900)];
    assert_eq!(largest_whole_files(&files), " (src/big.rs 2400 lines, src/next.rs 900 lines, src/mid.rs 300 lines)");
}

#[test]
fn no_whole_files_add_nothing_to_the_line() {
    assert_eq!(largest_whole_files(&[]), "");
}
