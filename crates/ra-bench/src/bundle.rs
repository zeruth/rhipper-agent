//! The benchmark's files, by path: the embedded set of the build's commit, or a directory read at
//! run time in its place for writing cards.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::{fs, io};

use sha2::{Digest, Sha256};

include!(concat!(env!("OUT_DIR"), "/embedded.rs"));

/// The files of `bench/workflow`, keyed by their `/`-separated path relative to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    files: BTreeMap<String, Vec<u8>>,
}

impl Bundle {
    /// The benchmark embedded at build time.
    #[must_use]
    pub fn embedded() -> Self {
        Self::from_files(EMBEDDED.iter().map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec())))
    }

    /// The files under `dir`, read now, in place of the embedded ones.
    ///
    /// # Errors
    /// When `dir` or anything below it cannot be read, or a name is not UTF-8.
    pub fn from_dir(dir: &Path) -> io::Result<Self> {
        let mut files = BTreeMap::new();
        read_dir(dir, "", &mut files)?;
        Ok(Self { files })
    }

    /// The directory given at run time when there is one, the embedded benchmark otherwise.
    ///
    /// # Errors
    /// As [`Bundle::from_dir`].
    pub fn load(dir: Option<&Path>) -> io::Result<Self> {
        dir.map_or_else(|| Ok(Self::embedded()), Self::from_dir)
    }

    /// A bundle of the given files.
    #[must_use]
    pub fn from_files(files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Self {
        Self { files: files.into_iter().collect() }
    }

    /// Every path, in order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// The bytes of one file, as stored.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// The hash of the whole bundle, as [`hash_of`] gives it.
    #[must_use]
    pub fn hash(&self) -> String {
        hash_of(self.files.iter().map(|(path, bytes)| (path.as_str(), bytes.as_slice())))
    }
}

/// `sha256:` and the hex digest over each file in path order: its path, a zero byte, its bytes
/// with CRLF folded to LF, a zero byte. Folding makes a Windows checkout hash as a Linux one.
#[must_use]
pub fn hash_of<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut files: Vec<_> = files.into_iter().collect();
    files.sort_unstable_by_key(|(path, _)| *path);
    let mut hasher = Sha256::new();
    for (path, bytes) in files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(fold_crlf(bytes));
        hasher.update([0]);
    }
    let mut out = String::from("sha256:");
    for byte in hasher.finalize() {
        write!(out, "{byte:02x}").expect("writing to a String");
    }
    out
}

fn fold_crlf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut iter = bytes.iter().peekable();
    while let Some(&byte) = iter.next() {
        if byte == b'\r' && iter.peek() == Some(&&b'\n') {
            continue;
        }
        out.push(byte);
    }
    out
}

fn read_dir(dir: &Path, prefix: &str, files: &mut BTreeMap<String, Vec<u8>>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| io::Error::new(io::ErrorKind::InvalidData, format!("not UTF-8: {}", name.display())))?;
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if entry.file_type()?.is_dir() {
            read_dir(&entry.path(), &rel, files)?;
        } else {
            files.insert(rel, fs::read(entry.path())?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
