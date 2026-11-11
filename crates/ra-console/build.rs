//! The build number: `0.1.<n>`, counted up from zero in `var/build-number` every time the
//! workspace sources change, so a running console names the build it is and two builds never
//! share a name.

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let counter = root.join("var/build-number");
    let current: u64 = std::fs::read_to_string(&counter).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let _ = std::fs::create_dir_all(root.join("var"));
    let _ = std::fs::write(&counter, (current + 1).to_string());
    println!("cargo:rustc-env=RA_VERSION=0.1.{current}");
    // any source change in the workspace is a new build
    for dir in ["crates", "apps"] {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
    }
}
