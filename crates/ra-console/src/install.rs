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
/// path when it is not, and says what it did, one line each. The build is `debug` under the
/// target directory cargo names for the working directory (`CARGO_TARGET_DIR` when set) when
/// it holds the binaries, else the directory of the running binary; the copied rc must answer
/// the version the sources name;
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
    let beside = std::env::current_exe()?.parent().context("the running binary has no directory")?.to_path_buf();
    let cwd = std::env::current_dir().ok();
    let target = std::env::var_os("CARGO_TARGET_DIR").filter(|t| !t.is_empty());
    // cargo's own answer covers a subdirectory of the workspace and a target set in its config
    let build = cwd.as_deref().and_then(|d| build_source(&cargo_target_dir(d).unwrap_or_else(|| target_dir(d, target.as_deref()))));
    // an install from a checkout builds both binaries first: `cargo run -p rc -- install`
    // builds rc alone, and the window went out three times on a stale binary before anyone
    // noticed (the person: "install should install the window too")
    if let Some(build) = &build
        && let Some(root) = cwd.as_deref()
    {
        // the binary running this install cannot be rebuilt under itself (`target\debug\rc
        // install` is the freshly built rc): the others are
        let running = std::env::current_exe().ok().and_then(|exe| exe.file_stem().map(|s| s.to_string_lossy().into_owned()));
        let from_build = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|p| p.to_path_buf())).is_some_and(|p| {
            match (std::fs::canonicalize(&p), std::fs::canonicalize(build)) {
                (Ok(a), Ok(b)) => a == b,
                _ => p == *build,
            }
        });
        let packages: Vec<&str> = BINARIES.iter().copied().filter(|stem| !(from_build && running.as_deref() == Some(stem))).collect();
        build_packages(root, &packages)?;
    }
    // the version the copied rc must answer: the one the sources name for a build, this
    // binary's own when the install copies the directory it runs from
    let expected = match (&build, cwd.as_deref()) {
        (Some(_), Some(root)) => sources_version(root),
        _ => Some(crate::VERSION.to_owned()),
    };
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
    lines.extend(copy_binaries(&source, &dir, expected.as_deref(), version_of)?);
    lines.push(on_path(&dir));
    Ok(lines)
}

/// The build an install copies: `debug` under the target directory the build wrote; `None`
/// when it holds no rc.
fn build_source(target: &Path) -> Option<PathBuf> {
    let suffix = std::env::consts::EXE_SUFFIX;
    Some(target.join("debug")).filter(|d| d.join(format!("rc{suffix}")).is_file())
}

/// The target directory cargo builds the workspace around `cwd` into, as `cargo metadata`
/// names it (`CARGO_TARGET_DIR`, the config's, else the workspace root's `target`); `None`
/// outside a workspace or when cargo does not answer.
fn cargo_target_dir(cwd: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps", "--offline"])
        .current_dir(cwd)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    let metadata: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    metadata.get("target_directory")?.as_str().map(PathBuf::from)
}

/// The version the sources at `root` name, as the build script counts it: `0.2.<commits>`, and
/// a `+` when `crates` or `apps` holds uncommitted changes; `None` without git.
fn sources_version(root: &Path) -> Option<String> {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let commits = git(&["rev-list", "--count", "HEAD"])?.parse::<u64>().ok()?;
    let dirty = git(&["status", "--porcelain", "--untracked-files=no", "--", "crates", "apps"]).is_some_and(|s| !s.is_empty());
    Some(format!("0.2.{commits}{}", if dirty { "+" } else { "" }))
}

/// Copies the binaries of `source` into `dir` and says what it copied, one line each. The rc
/// copied is asked its version through `version` and refused, with the binary set aside put
/// back, when it is not `expected`, the version the build was made at.
///
/// # Errors
/// When a binary cannot be set aside or copied, or the copied rc is not the build's.
fn copy_binaries(
    source: &Path,
    dir: &Path,
    expected: Option<&str>,
    version: impl Fn(&Path) -> Option<String>,
) -> anyhow::Result<Vec<String>> {
    let suffix = std::env::consts::EXE_SUFFIX;
    let mut lines = Vec::new();
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
        // the copy's own version against the sources': a stale build copied from the wrong
        // directory answers an older one; only rc is asked, the window would open
        let copied = if stem == "rc" { version(&to) } else { None };
        if let Err(e) = check_versions(expected.filter(|_| stem == "rc"), copied.as_deref()) {
            // nothing of this install stays: the binary set aside comes back
            let _ = std::fs::remove_file(&to);
            if let Some(aside) = &aside {
                let _ = std::fs::rename(aside, &to);
            }
            return Err(e);
        }
        let version = copied.unwrap_or_else(|| crate::VERSION.to_owned());
        lines.push(format!("{name} {version} → {}", to.display()));
        // a running walk reads it between cards and moves to the new rc
        if stem == "rc" {
            let _ = std::fs::write(dir.join(crate::hub::INSTALLED_FILE), format!("{version} {}\n", unix_now()));
        }
    }
    Ok(lines)
}

/// The target directory of a build from `cwd` when cargo does not say: `target`
/// (`CARGO_TARGET_DIR`, relative to `cwd` as cargo reads it) when set, else `<cwd>/target`.
fn target_dir(cwd: &Path, target: Option<&std::ffi::OsStr>) -> PathBuf {
    cwd.join(target.unwrap_or_else(|| std::ffi::OsStr::new("target")))
}

/// What `binary --version` prints last, the version; `None` when it does not run.
fn version_of(binary: &Path) -> Option<String> {
    let out = std::process::Command::new(binary).arg("--version").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).split_whitespace().last().map(str::to_owned)).flatten()
}

/// Refuses an install whose copied rc is not the build's, naming both. The `+` of a tree
/// with changes is not compared: the sources' count does not carry it.
///
/// # Errors
/// When both versions are known and differ.
fn check_versions(built: Option<&str>, copied: Option<&str>) -> anyhow::Result<()> {
    if let (Some(built), Some(copied)) = (built, copied) {
        anyhow::ensure!(
            built.trim_end_matches('+') == copied.trim_end_matches('+'),
            "the build's rc is {built} but the copied rc is {copied}; nothing was installed"
        );
    }
    Ok(())
}

/// Builds `packages` in the workspace at `root`, so an install from a checkout installs what
/// the sources say and not what happened to be built last. Cargo's own output goes to the
/// terminal; a failed build is the install's failure.
///
/// # Errors
/// When cargo cannot start or the build fails.
fn build_packages(root: &Path, packages: &[&str]) -> anyhow::Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let mut command = std::process::Command::new("cargo");
    command.arg("build").current_dir(root);
    for package in packages {
        command.args(["-p", package]);
    }
    let status = command.status().with_context(|| format!("cannot start cargo to build {}", packages.join(", ")))?;
    anyhow::ensure!(status.success(), "the build of {} failed; nothing was installed", packages.join(" and "));
    Ok(())
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
mod tests;
