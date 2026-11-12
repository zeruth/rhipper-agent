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

/// Installs the build into `dir` (the default when `None`), puts the directory on the user's
/// path when it is not, and says what it did, one line each. The build is `target/debug` under
/// the working directory when it holds the binaries, else the directory of the running binary;
/// an installed binary asked to install from its own directory refuses, rather than setting
/// itself aside and copying nothing (which once left no `rc` at all).
///
/// # Errors
/// When the directory cannot be made, the source is the destination, or a binary cannot be
/// copied; a binary set aside for a copy that failed is put back.
pub fn install(dir: Option<&Path>) -> anyhow::Result<Vec<String>> {
    // one install or restart at a time across the projects: two would set the same binary aside twice
    let _install = crate::hub::guard(crate::hub::INSTALL_LOCK);
    let dir = dir.map_or_else(default_dir, Path::to_path_buf);
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot make {}", dir.display()))?;
    let suffix = std::env::consts::EXE_SUFFIX;
    let beside = std::env::current_exe()?.parent().context("the running binary has no directory")?.to_path_buf();
    let build = std::env::current_dir().ok().map(|d| d.join("target").join("debug")).filter(|d| d.join(format!("rc{suffix}")).is_file());
    let source = build.unwrap_or(beside);
    let same = |a: &Path, b: &Path| match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    anyhow::ensure!(
        !same(&source, &dir),
        "{} holds the installed binaries, not a build: run `rc install` from the project's directory after `cargo build -p rc -p rc-gui`, or `.\\target\\debug\\rc install`",
        dir.display()
    );
    let mut lines = Vec::new();
    sweep(&dir, &mut lines);
    for stem in BINARIES {
        let name = format!("{stem}{suffix}");
        let from = source.join(&name);
        if !from.is_file() {
            lines.push(format!("{name}: not built in {}; skipped", source.display()));
            continue;
        }
        let to = dir.join(&name);
        // a running binary refuses to be overwritten, never to be moved: the old one goes aside
        // and comes back if the copy fails
        let aside = to.exists().then(|| dir.join(format!("{stem}.old.{}{suffix}", unix_now())));
        if let Some(aside) = &aside {
            std::fs::rename(&to, aside).with_context(|| format!("cannot set {} aside", to.display()))?;
        }
        if let Err(e) = std::fs::copy(&from, &to) {
            if let Some(aside) = &aside {
                let _ = std::fs::rename(aside, &to);
            }
            return Err(anyhow::anyhow!(e).context(format!("cannot copy {} to {}", from.display(), to.display())));
        }
        // the copy's own version, which is the build's and not this binary's when they differ;
        // only rc is asked, the window would open
        let version = (stem == "rc")
            .then(|| std::process::Command::new(&to).arg("--version").output().ok())
            .flatten()
            .filter(|out| out.status.success())
            .and_then(|out| String::from_utf8_lossy(&out.stdout).split_whitespace().last().map(str::to_owned))
            .unwrap_or_else(|| crate::VERSION.to_owned());
        lines.push(format!("{name} {version} → {}", to.display()));
        // a running walk reads it between cards and moves to the new rc
        if stem == "rc" {
            let _ = std::fs::write(dir.join(crate::hub::INSTALLED_FILE), format!("{version} {}\n", unix_now()));
        }
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
    // the user's own path in the registry, not the session's. A read first, through `reg`,
    // which answers at once; PowerShell starts slowly and is only worth it for the write, which
    // setx would clip on a long path
    let registry = std::process::Command::new("reg").args(["query", r"HKCU\Environment", "/v", "Path"]).output();
    if let Ok(out) = &registry
        && out.status.success()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        let value = text.lines().find_map(|l| l.trim_start().strip_prefix("Path")).map(|rest| {
            // `Path    REG_EXPAND_SZ    <value>`
            rest.trim_start().split_once(char::is_whitespace).map_or("", |(_, v)| v.trim())
        });
        if value.is_some_and(listed) {
            return format!("{} is on the user's path; a new terminal sees it", dir.display());
        }
    }
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

/// Puts a shortcut to the installed window on the desktop when there is none: `Rhipper
/// Agent.lnk`, opening `project` as the working directory, so the window starts on this
/// project. Says what it did, or why not.
pub fn desktop_shortcut(dir: &Path, project: &Path) -> String {
    if !cfg!(windows) {
        return "a desktop shortcut is a Windows thing; put a launcher of your own on the path".to_owned();
    }
    let target = dir.join(format!("rc-gui{}", std::env::consts::EXE_SUFFIX));
    let project =
        std::fs::canonicalize(project).map_or_else(|_| project.display().to_string(), |p| p.display().to_string().replace("\\\\?\\", ""));
    // the desktop as Windows resolves it (a OneDrive desktop is not under the profile), and the
    // shortcut through the shell object: no crate, one PowerShell start, only when it is missing
    let script = format!(
        "$desktop = [Environment]::GetFolderPath('Desktop'); $path = Join-Path $desktop 'Rhipper Agent.lnk'; if (Test-Path $path) {{ 'present ' + $path }} else {{ $s = (New-Object -ComObject WScript.Shell).CreateShortcut($path); $s.TargetPath = '{0}'; $s.WorkingDirectory = '{1}'; $s.IconLocation = '{0},0'; $s.Description = 'Rhipper Agent, the workspace'; $s.Save(); 'added ' + $path }}",
        target.display(),
        project
    );
    match std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).output() {
        Ok(out) if out.status.success() => {
            let said = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            match said.split_once(' ') {
                Some(("added", path)) => format!("desktop shortcut added: {path} → {} in {project}", target.display()),
                Some(("present", path)) => format!("desktop shortcut already there: {path}"),
                _ => format!("desktop shortcut: {said}"),
            }
        }
        Ok(out) => format!("no desktop shortcut: {}", String::from_utf8_lossy(&out.stderr).trim()),
        Err(e) => format!("no desktop shortcut: {e}"),
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
