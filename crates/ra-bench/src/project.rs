//! The benchmark's project: the files of `bench/workflow/project`, written out as a directory a
//! run can build, test and put its cards on.

use std::io;
use std::path::Path;

use crate::Bundle;

/// Where the project's files sit in the bundle.
pub const DIR: &str = "project/";

/// Writes every file of the project in `bundle` below `dest`, by its path relative to the project.
///
/// # Errors
/// When a directory or file cannot be written.
pub fn write(bundle: &Bundle, dest: &Path) -> io::Result<()> {
    for path in bundle.paths() {
        let Some(rel) = path.strip_prefix(DIR) else { continue };
        let Some(bytes) = bundle.get(path) else { continue };
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
