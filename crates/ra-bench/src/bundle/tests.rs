use std::fs;

use super::{Bundle, hash_of};

fn set<'a>(files: &[(&'a str, &'a str)]) -> String {
    hash_of(files.iter().map(|(path, text)| (*path, text.as_bytes())))
}

#[test]
fn bundle_embedded_lists_bench_toml() {
    let bundle = Bundle::embedded();
    assert!(bundle.paths().any(|path| path == "bench.toml"), "{:?}", bundle.paths().collect::<Vec<_>>());
    assert!(bundle.get("bench.toml").is_some_and(|bytes| bytes.starts_with(b"version = ")));
}

#[test]
fn bundle_hash_is_the_same_with_lf_and_crlf() {
    let lf = set(&[("a.rs", "fn a() {}\nfn b() {}\n"), ("b/c.toml", "x = 1\n")]);
    let crlf = set(&[("a.rs", "fn a() {}\r\nfn b() {}\r\n"), ("b/c.toml", "x = 1\r\n")]);
    assert_eq!(lf, crlf);
    assert!(lf.starts_with("sha256:") && lf.len() == "sha256:".len() + 64, "{lf}");
}

#[test]
fn bundle_hash_does_not_depend_on_the_order_given() {
    assert_eq!(set(&[("a", "1"), ("b", "2")]), set(&[("b", "2"), ("a", "1")]));
}

#[test]
fn bundle_hash_differs_when_a_byte_of_content_differs() {
    assert_ne!(set(&[("a.rs", "fn a() {}\n")]), set(&[("a.rs", "fn b() {}\n")]));
}

#[test]
fn bundle_hash_differs_when_a_path_is_renamed() {
    assert_ne!(set(&[("a.rs", "fn a() {}\n")]), set(&[("b.rs", "fn a() {}\n")]));
    // The zero byte keeps a path from running into its content.
    assert_ne!(set(&[("ab", "c")]), set(&[("a", "bc")]));
}

#[test]
fn bundle_reads_a_directory_given_at_run_time_in_place_of_the_embedded_one() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("project/src")).unwrap();
    fs::write(dir.path().join("other.toml"), "version = 2\n").unwrap();
    fs::write(dir.path().join("project/src/lib.rs"), "pub fn f() {}\r\n").unwrap();

    let bundle = Bundle::load(Some(dir.path())).unwrap();
    assert_eq!(bundle.paths().collect::<Vec<_>>(), ["other.toml", "project/src/lib.rs"]);
    assert_eq!(bundle.get("project/src/lib.rs"), Some(&b"pub fn f() {}\r\n"[..]));
    assert_eq!(bundle.hash(), set(&[("other.toml", "version = 2\n"), ("project/src/lib.rs", "pub fn f() {}\n")]));

    assert_eq!(Bundle::load(None).unwrap(), Bundle::embedded());
}
