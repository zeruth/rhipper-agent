//! The build version: `0.1.<commits>` from the git history, with the short hash and a `.dirty`
//! mark when the tree has uncommitted changes, so a running console names the build it is.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    let count = git(&["rev-list", "--count", "HEAD"]).unwrap_or_else(|| "0".to_owned());
    let hash = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "nogit".to_owned());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let version = format!("0.1.{count}+{hash}{}", if dirty { ".dirty" } else { "" });
    println!("cargo:rustc-env=RA_VERSION={version}");
    // the version moves with the history and the index; a plain edit shows on the next commit
    for path in ["../../.git/HEAD", "../../.git/index", "../../.git/refs/heads"] {
        println!("cargo:rerun-if-changed={path}");
    }
}
