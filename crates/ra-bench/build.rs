//! Embeds every file under `bench/workflow`, so a binary carries the benchmark of its own commit.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let root = manifest.join("../../bench/workflow");
    println!("cargo:rerun-if-changed={}", root.display());

    let mut files = Vec::new();
    walk(&root, "", &mut files);
    files.sort();

    let mut out = String::from("pub(crate) static EMBEDDED: &[(&str, &[u8])] = &[\n");
    for (rel, path) in &files {
        let path = path.to_str().expect("a bench path is UTF-8");
        writeln!(out, "    ({rel:?}, include_bytes!({path:?})),").expect("writing to a String");
    }
    out.push_str("];\n");

    let dest = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("embedded.rs");
    fs::write(dest, out).expect("write embedded.rs");
}

/// Collects every file below `dir` as its `/`-separated path relative to the bench root.
fn walk(dir: &Path, prefix: &str, files: &mut Vec<(String, PathBuf)>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("a directory entry");
        let name = entry.file_name().into_string().expect("a bench file name is UTF-8");
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let path = entry.path();
        if entry.file_type().expect("a file type").is_dir() {
            walk(&path, &rel, files);
        } else {
            files.push((rel, path));
        }
    }
}
