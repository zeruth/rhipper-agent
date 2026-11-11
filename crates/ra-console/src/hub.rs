//! One window for every project. The first `rc-gui` to start takes the hub's lock, a file held
//! open for exclusive use under the user's local data, and becomes the window; anyone else who
//! wants a project opened — `rc open <dir>`, a second `rc-gui`, the desktop shortcut, a paired
//! session — drops the project's path into the hub's mailbox and leaves, and the window opens it
//! as a tab on its next tick. The window keeps the list of open projects, so the next start
//! brings them all back.

use std::path::{Path, PathBuf};

use anyhow::Context;

/// The hub's directory: `%LOCALAPPDATA%\rhipper\agent` on Windows (the IDE keeps its own
/// directories beside it under `rhipper`), `~/.local/share/rhipper/agent` elsewhere.
#[must_use]
pub fn dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("rhipper"), |d| PathBuf::from(d).join("rhipper")).join("agent")
    } else {
        std::env::var_os("HOME")
            .map_or_else(|| PathBuf::from(".rhipper"), |d| PathBuf::from(d).join(".local").join("share").join("rhipper"))
            .join("agent")
    }
}

/// The lock a running window holds. Dropping it lets the next window take over.
pub struct Lock(#[allow(dead_code)] std::fs::File);

/// Takes the hub's lock: `None` when a window holds it already. On Windows the file is opened
/// with no sharing, which a second opener cannot do; elsewhere a pid file stands in, and a dead
/// pid is overwritten.
#[must_use]
pub fn try_lock() -> Option<Lock> {
    let dir = dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("gui.lock");
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false).share_mode(0).open(&path).ok()?;
        Some(Lock(file))
    }
    #[cfg(not(windows))]
    {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(pid) = text.trim().parse::<u32>()
            && pid != std::process::id()
            && Path::new(&format!("/proc/{pid}")).exists()
        {
            return None;
        }
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&path).ok()?;
        use std::io::Write;
        let mut file = file;
        let _ = write!(file, "{}", std::process::id());
        Some(Lock(file))
    }
}

/// Whether a window holds the lock right now.
#[must_use]
pub fn window_running() -> bool {
    try_lock().is_none()
}

/// Asks the running window to open `project` as a tab: a file in the mailbox with the path.
///
/// # Errors
/// When the mailbox cannot be written.
pub fn request_open(project: &Path) -> anyhow::Result<()> {
    let mailbox = dir().join("open");
    std::fs::create_dir_all(&mailbox)?;
    let project = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    let name = format!("{}-{}.txt", unix_nanos(), std::process::id());
    std::fs::write(mailbox.join(name), project.display().to_string()).context("cannot write to the hub's mailbox")
}

/// The projects asked for since the last call, oldest first; each request is consumed.
#[must_use]
pub fn take_requests() -> Vec<PathBuf> {
    let mailbox = dir().join("open");
    let Ok(entries) = std::fs::read_dir(&mailbox) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "txt")).collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|file| {
            let text = std::fs::read_to_string(&file).ok()?;
            let _ = std::fs::remove_file(&file);
            let path = PathBuf::from(text.trim().trim_start_matches(r"\\?\"));
            (!path.as_os_str().is_empty()).then_some(path)
        })
        .collect()
}

/// The projects the window had open last, oldest first.
#[must_use]
pub fn recent() -> Vec<PathBuf> {
    std::fs::read_to_string(dir().join("open.txt"))
        .map(|text| text.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default()
}

/// Remembers the projects the window has open, for the next start.
pub fn remember(projects: &[PathBuf]) {
    let dir = dir();
    let _ = std::fs::create_dir_all(&dir);
    let text: Vec<String> = projects.iter().map(|p| p.display().to_string().trim_start_matches(r"\\?\").to_owned()).collect();
    let _ = std::fs::write(dir.join("open.txt"), text.join("\n"));
}

/// Asks the running window to quit whole, its tabs kept for the next start.
///
/// # Errors
/// When the request cannot be written.
pub fn request_quit() -> anyhow::Result<()> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("quit"), "").context("cannot write the hub's quit request")
}

/// Whether the window was asked to quit whole; the request is consumed.
#[must_use]
pub fn quit_requested() -> bool {
    let quit = dir().join("quit");
    quit.exists() && std::fs::remove_file(quit).is_ok()
}

/// Starts the installed window on `project`, detached, or hands the project to the running
/// one. Says which.
///
/// # Errors
/// When neither can be done.
pub fn open(project: &Path) -> anyhow::Result<String> {
    let shown = std::fs::canonicalize(project)
        .map_or_else(|_| project.display().to_string(), |p| p.display().to_string().trim_start_matches(r"\\?\").to_owned());
    if window_running() {
        request_open(project)?;
        return Ok(format!("{shown} opens as a tab of the running window"));
    }
    let gui = launch(Some(project))?;
    Ok(format!("{shown} opens in a new window ({}), with the tabs of last session", gui.display()))
}

/// Quits the running window, waits for it to go, and starts the installed one again on the
/// tabs it had: the way a new build comes up. Says what happened.
///
/// # Errors
/// When the window will not quit or the installed one cannot start.
pub fn restart() -> anyhow::Result<String> {
    let was_running = window_running();
    if was_running {
        request_quit()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
        while window_running() {
            anyhow::ensure!(std::time::Instant::now() < deadline, "the window did not quit in time");
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    }
    let gui = launch(None)?;
    let tabs = recent().len();
    Ok(if was_running {
        format!("the window restarted on {} with its {tabs} tab(s)", gui.display())
    } else {
        format!("no window was running; {} started with {tabs} tab(s)", gui.display())
    })
}

/// Starts the installed window, detached, on `project` when one is named, else on the tabs of
/// last session. Returns the binary it started.
fn launch(project: Option<&Path>) -> anyhow::Result<PathBuf> {
    let gui = crate::install::default_dir().join(format!("rc-gui{}", std::env::consts::EXE_SUFFIX));
    let gui = if gui.is_file() {
        gui
    } else {
        // not installed: the one beside this binary, a build
        std::env::current_exe()?.with_file_name(format!("rc-gui{}", std::env::consts::EXE_SUFFIX))
    };
    // detached: the window outlives the shell that asked for it and holds none of its pipes
    let mut command = std::process::Command::new(&gui);
    if let Some(project) = project {
        command.arg(project).current_dir(project);
    }
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        // the shell's pipes came to this process as inheritable handles, and a child inherits
        // every inheritable handle whatever its own stdio is set to; the shell would then wait
        // on the window for the pipe to close. They are made private before the spawn.
        keep_std_handles_private();
    }
    command.spawn().with_context(|| format!("cannot start {}", gui.display()))?;
    Ok(gui)
}

/// Clears the inherit flag on this process's standard handles, so a detached child does not
/// take the shell's pipes along.
#[cfg(windows)]
fn keep_std_handles_private() {
    use std::os::windows::io::AsRawHandle;
    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    }
    let handles = [std::io::stdin().as_raw_handle(), std::io::stdout().as_raw_handle(), std::io::stderr().as_raw_handle()];
    for handle in handles {
        if !handle.is_null() {
            // SAFETY: a valid handle of this process; the call only changes its inherit flag
            unsafe {
                SetHandleInformation(handle.cast(), HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

fn unix_nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}
