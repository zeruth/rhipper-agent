//! `rc install`: the build beside the running binary goes into a directory on the path, so the
//! console and the window run from there and `target/` is never locked by a running instance.
//! A running installed binary cannot be overwritten, but it can be moved: the old one is set
//! aside as `.old` first, the new one copied in, and the set-aside files of earlier installs
//! are removed once nothing runs them.

use std::path::{Path, PathBuf};

use anyhow::Context;

/// The binaries an install carries, by stem.
pub const BINARIES: [&str; 2] = ["rc", "rc-gui"];

/// Where the binaries go unless told otherwise: `%LOCALAPPDATA%\Programs\rhipper` on Windows,
/// `~/.local/bin` elsewhere.
pub fn default_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("rhipper"), |d| PathBuf::from(d).join("Programs").join("rhipper"))
    } else {
        std::env::var_os("HOME").map_or_else(|| PathBuf::from(".local/bin"), |d| PathBuf::from(d).join(".local").join("bin"))
    }
}

/// Installs the binaries beside the running one into `dir` (the default when `None`), puts the
/// directory on the user's path when it is not, and says what it did, one line each.
///
/// # Errors
/// When the directory cannot be made or a binary cannot be copied.
pub fn install(dir: Option<&Path>) -> anyhow::Result<Vec<String>> {
    let dir = dir.map_or_else(default_dir, Path::to_path_buf);
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot make {}", dir.display()))?;
    let source = std::env::current_exe()?.parent().context("the running binary has no directory")?.to_path_buf();
    let suffix = std::env::consts::EXE_SUFFIX;
    let mut lines = Vec::new();
    sweep(&dir, &mut lines);
    for stem in BINARIES {
        let name = format!("{stem}{suffix}");
        let from = source.join(&name);
        if !from.exists() {
            lines.push(format!("{name}: not built beside {}; skipped", source.display()));
            continue;
        }
        let to = dir.join(&name);
        if to.exists() {
            // a running binary refuses to be overwritten, never to be moved
            let aside = dir.join(format!("{stem}.old.{}{suffix}", unix_now()));
            std::fs::rename(&to, &aside).with_context(|| format!("cannot set {} aside", to.display()))?;
        }
        std::fs::copy(&from, &to).with_context(|| format!("cannot copy {} to {}", from.display(), to.display()))?;
        lines.push(format!("{name} {} → {}", crate::VERSION, to.display()));
    }
    lines.push(on_path(&dir));
    Ok(lines)
}

/// Removes the set-aside binaries of earlier installs that nothing runs any more.
fn sweep(dir: &Path, lines: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut gone = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains(".old.") && std::fs::remove_file(entry.path()).is_ok() {
            gone += 1;
        }
    }
    if gone > 0 {
        lines.push(format!("{gone} set-aside binar{} of earlier installs removed", if gone == 1 { "y" } else { "ies" }));
    }
}

/// Puts `dir` on the user's path when it is not, and says which; the current shell keeps the
/// path it started with either way.
fn on_path(dir: &Path) -> String {
    let wanted = dir.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    let listed =
        |path: &str| path.split(if cfg!(windows) { ';' } else { ':' }).any(|p| p.trim_end_matches(['\\', '/']).to_lowercase() == wanted);
    if std::env::var("PATH").is_ok_and(|p| listed(&p)) {
        return format!("{} is on the path", dir.display());
    }
    if !cfg!(windows) {
        return format!("add {} to your PATH", dir.display());
    }
    // the user's own path in the registry, not the session's: setx would clip a long one
    let script = format!(
        "$user = [Environment]::GetEnvironmentVariable('Path', 'User'); if (($user -split ';') -notcontains '{0}') {{ [Environment]::SetEnvironmentVariable('Path', ($user.TrimEnd(';') + ';{0}'), 'User'); 'added' }} else {{ 'present' }}",
        dir.display()
    );
    match std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).output() {
        Ok(out) if out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "added" => {
            format!("{} added to the user's path; a new terminal sees it", dir.display())
        }
        Ok(out) if out.status.success() => format!("{} is on the user's path; a new terminal sees it", dir.display()),
        _ => format!("add {} to your PATH; the registry could not be written", dir.display()),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_dir_is_under_the_user_and_a_sweep_removes_the_set_aside_binaries() {
        let dir = default_dir();
        assert!(dir.ends_with(if cfg!(windows) { "Programs/rhipper" } else { ".local/bin" }), "{}", dir.display());
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("rc.old.1.exe"), b"x").unwrap();
        std::fs::write(tmp.path().join("rc.exe"), b"x").unwrap();
        let mut lines = Vec::new();
        sweep(tmp.path(), &mut lines);
        assert_eq!(lines, ["1 set-aside binary of earlier installs removed"]);
        assert!(tmp.path().join("rc.exe").exists());
        assert!(!tmp.path().join("rc.old.1.exe").exists());
    }
}
