//! One window for every project. The first `rc-gui` to start takes the hub's lock, a file held
//! open for exclusive use under the user's local data, and becomes the window; anyone else who
//! wants a project opened — `rc open <dir>`, a second `rc-gui`, the desktop shortcut, a paired
//! session — drops the project's path into the hub's mailbox and leaves, and the window opens it
//! as a tab on its next tick. The window keeps the list of open projects, so the next start
//! brings them all back.

use std::path::{Path, PathBuf};

use anyhow::Context;

/// The hub's directory: `%LOCALAPPDATA%\rhipper\agent` on Windows (the IDE keeps its own
/// directories beside it under `rhipper`), `~/.local/share/rhipper/agent` elsewhere. `RHIPPER_HUB`
/// names another, for the tests of the crates that build this one without `cfg(test)`; this
/// crate's own tests get a directory of their own under the temp directory, one a process, so a
/// test's digest, note, lock or mailbox never reaches the machine's hub and every project's runs.
#[must_use]
pub fn dir() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("rhipper-test-hub-{}", std::process::id()));
    }
    dir_from(std::env::var_os("RHIPPER_HUB").as_deref())
}

/// [`dir`] given the value of `RHIPPER_HUB`: the machine's hub without one.
fn dir_from(hub_var: Option<&std::ffi::OsStr>) -> PathBuf {
    if let Some(hub) = hub_var.filter(|h| !h.is_empty()) {
        return PathBuf::from(hub);
    }
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

/// A project's digest, under `digest/<project>.txt`: what it is doing, for the runs of the other
/// projects to read as one line rather than its roadmap or its files. A run's end and a walk's
/// start and end write it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Digest {
    /// The project's directory name, which names the file too.
    pub name: String,
    pub path: String,
    /// The process that wrote it, and when: a dead pid on a digest older than a day is stale.
    pub pid: u32,
    pub when: u64,
    /// The live run or walk, when one is going: `#12 title · pid 4242 · started 10:04`.
    pub live: Option<String>,
    /// The last card that ended and how, as the paired feed's `end` line says it.
    pub last: Option<String>,
    pub ready: usize,
    /// The files the last run changed.
    pub changed: Vec<String>,
    /// The uncommitted leftovers: the files changed by cards that need revision and still dirty
    /// in the tree, each as `path (#id …)`, so a later card knows the tree it inherits.
    pub leftovers: Vec<String>,
}

/// The longest a digest line gets in a system prompt: about fifty tokens a project.
pub const DIGEST_WIDTH: usize = 160;

/// A digest older than this whose writer is gone is dropped unread.
const DIGEST_STALE: u64 = 24 * 60 * 60;

/// The digests' directory under the hub.
fn digest_dir() -> PathBuf {
    dir().join("digest")
}

/// Writes `digest` for its project. A digest that cannot be written is nobody's failure: the
/// run goes on, the other projects read the last one.
pub fn write_digest(digest: &Digest) {
    if let Err(e) = write_digest_in(&digest_dir(), digest) {
        tracing::warn!("the project's digest was not written: {e:#}");
    }
}

/// [`write_digest`] into the directory named, for the tests.
///
/// # Errors
/// When the directory or the file cannot be written.
pub fn write_digest_in(dir: &Path, digest: &Digest) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    use std::fmt::Write;
    let one_line = |s: &str| s.replace(['\t', '\n'], " ");
    let mut text =
        format!("name: {}\npath: {}\npid: {}\nwhen: {}\n", one_line(&digest.name), one_line(&digest.path), digest.pid, digest.when);
    if let Some(live) = &digest.live {
        let _ = writeln!(text, "live: {}", one_line(live));
    }
    if let Some(last) = &digest.last {
        let _ = writeln!(text, "last: {}", one_line(last));
    }
    let _ = writeln!(text, "ready: {}", digest.ready);
    if !digest.changed.is_empty() {
        let _ = writeln!(text, "changed: {}", digest.changed.join(" "));
    }
    if !digest.leftovers.is_empty() {
        let _ = writeln!(text, "leftovers: {}", digest.leftovers.join(", "));
    }
    std::fs::write(dir.join(format!("{}.txt", digest.name)), text).context("cannot write the project's digest")
}

/// Reads the digest `root` last wrote, so a walk's start keeps the last run's end and files.
#[must_use]
pub fn own_digest(root: &Path) -> Option<Digest> {
    own_digest_in(&digest_dir(), root)
}

/// [`own_digest`] from the directory named, for the tests.
#[must_use]
pub fn own_digest_in(dir: &Path, root: &Path) -> Option<Digest> {
    parse_digest(&std::fs::read_to_string(dir.join(format!("{}.txt", project_name(root)))).ok()?)
}

fn parse_digest(text: &str) -> Option<Digest> {
    let mut digest = Digest::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(": ").or_else(|| line.strip_suffix(':').map(|k| (k, ""))) else { continue };
        let value = value.trim();
        match key {
            "name" => value.clone_into(&mut digest.name),
            "path" => value.clone_into(&mut digest.path),
            "pid" => digest.pid = value.parse().ok()?,
            "when" => digest.when = value.parse().ok()?,
            "live" => digest.live = Some(value.to_owned()),
            "last" => digest.last = Some(value.to_owned()),
            "ready" => digest.ready = value.parse().ok()?,
            "changed" => digest.changed = value.split_whitespace().map(str::to_owned).collect(),
            "leftovers" => digest.leftovers = value.split(", ").map(str::to_owned).collect(),
            _ => {}
        }
    }
    (!digest.name.is_empty()).then_some(digest)
}

/// The other projects' digests as one line each, clipped at [`DIGEST_WIDTH`], for the system
/// prompt of a run in `root`: the project's own is left out, and a stale one, whose writer is
/// gone and which is older than a day, or whose project is no longer on disk, is dropped. Never
/// their files or notes.
#[must_use]
pub fn digest_lines(root: &Path) -> Vec<String> {
    digest_lines_with(&digest_dir(), root, pid_alive, unix_now())
}

/// [`digest_lines`] with the directory, the liveness check and the time named, for the tests.
#[must_use]
pub fn digest_lines_with(dir: &Path, root: &Path, alive: impl Fn(u32) -> bool, now: u64) -> Vec<String> {
    let this = project_name(root);
    digests_in(dir, alive, now).iter().filter(|digest| digest.name != this).map(digest_line).collect()
}

/// Every project's digest under the hub, this project's among them, by name; a stale one is
/// dropped as in [`digest_lines`].
#[must_use]
pub fn digests() -> Vec<Digest> {
    digests_in(&digest_dir(), pid_alive, unix_now())
}

/// [`digests`] from the directory named, with the liveness check and the time, for the tests.
#[must_use]
pub fn digests_in(dir: &Path, alive: impl Fn(u32) -> bool, now: u64) -> Vec<Digest> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "txt")).collect();
    files.sort();
    files
        .iter()
        .filter_map(|file| parse_digest(&std::fs::read_to_string(file).ok()?))
        .filter(|digest| {
            // a project that is gone, a temp directory a test ran in say, has nothing to report
            let stale = !Path::new(&digest.path).exists() || (now.saturating_sub(digest.when) > DIGEST_STALE && !alive(digest.pid));
            if stale {
                let _ = std::fs::remove_file(dir.join(format!("{}.txt", digest.name)));
            }
            !stale
        })
        .collect()
}

/// One digest as a line: the name and path, then the live run, the last end, the ready count and
/// the changed files, whatever is there, clipped at the width.
fn digest_line(digest: &Digest) -> String {
    let mut parts = vec![format!("{} ({})", digest.name, digest.path)];
    if let Some(live) = &digest.live {
        parts.push(format!("live: {live}"));
    }
    if let Some(last) = &digest.last {
        parts.push(format!("last: {last}"));
    }
    parts.push(format!("{} ready", digest.ready));
    if !digest.changed.is_empty() {
        parts.push(format!("changed: {}", digest.changed.join(" ")));
    }
    if !digest.leftovers.is_empty() {
        parts.push(format!("leftovers: {}", digest.leftovers.join(", ")));
    }
    let line = parts.join(" · ");
    if line.chars().count() <= DIGEST_WIDTH {
        return line;
    }
    let mut clipped: String = line.chars().take(DIGEST_WIDTH - 1).collect();
    clipped.push('…');
    clipped
}

/// How a project is named in the hub: its directory's name.
#[must_use]
pub fn project_name(root: &Path) -> String {
    std::fs::canonicalize(root)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| String::from("-"))
}

/// One project's state in a read, as `rc status` prints it and the window's all-projects band
/// shows it: the run the paired feed has open, judged by its pid, the last end, the ready count
/// of the roadmap and today's calls from the ledger. Nothing of it is written anywhere.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub name: String,
    pub path: String,
    pub run: Run,
    /// The last card that ended and how, as the feed's `end` line says it.
    pub last: Option<String>,
    /// None without a roadmap to read.
    pub ready: Option<usize>,
    pub calls_today: u64,
    pub spend_today: Option<f64>,
}

/// What the paired feed has open in a project.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Run {
    #[default]
    Idle,
    /// A start with no end whose process lives, or carries no pid to judge: the start's text.
    Live(String),
    /// A start with no end whose process is gone: the window or the shell took the run with it.
    Orphaned(String),
}

/// The most ledger rows a status reads back for today's calls: more than a day writes.
const TODAY_ROWS: u32 = 5000;

/// The status of the project at `root`, its ledger read when it has one.
pub async fn status(root: &Path) -> Status {
    // a project without a ledger gets none opened for it: the open would create the file
    let ledger = root.join("var/usage.db");
    let spends = if ledger.is_file() {
        match ra_usage::UsageStore::open(&ledger).await {
            Ok(ledger) => ledger.recent(0, TODAY_ROWS).await.unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    status_with(root, pid_alive, &spends, &day_of(unix_now()))
}

/// The status of every project the hub remembers, this one first: the window's open tabs and
/// the digests' projects, each once, a directory that is gone left out.
pub async fn statuses(root: &Path) -> Vec<Status> {
    let mut roots: Vec<PathBuf> = vec![root.to_path_buf()];
    roots.extend(recent());
    roots.extend(digests().into_iter().map(|d| PathBuf::from(d.path)));
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut rows = Vec::new();
    for root in roots {
        let Ok(canonical) = std::fs::canonicalize(&root) else { continue };
        if seen.contains(&canonical) {
            continue;
        }
        seen.push(canonical);
        rows.push(status(&root).await);
    }
    rows
}

/// [`status`] with the liveness check, the ledger's rows and the day named, for the tests.
#[must_use]
pub fn status_with(root: &Path, alive: impl Fn(u32) -> bool, spends: &[ra_usage::Spend], today: &str) -> Status {
    let run = match crate::pair::live_start(root) {
        Some((text, Some(pid))) if !alive(pid) => Run::Orphaned(text),
        Some((text, _)) => Run::Live(text),
        None => Run::Idle,
    };
    let ready = std::fs::read_to_string(root.join("rhipper.md")).ok().and_then(|text| {
        let nodes = ra_roadmap::markdown::parse(&text).ok()?;
        Some(ra_roadmap::tree::effective_states(&nodes).values().filter(|s| **s == ra_roadmap::model::TaskState::Ready).count())
    });
    let (calls_today, spend_today) = spend_on(spends, today);
    let path = std::fs::canonicalize(root)
        .map_or_else(|_| root.display().to_string(), |p| p.display().to_string().trim_start_matches(r"\\?\").to_owned());
    Status { name: project_name(root), path, run, last: crate::pair::last_end(root), ready, calls_today, spend_today }
}

/// The calls and the dollars of the ledger rows written on `today` (`YYYY-MM-DD`, UTC, as the
/// ledger stamps them); the cost is `None` when no call of the day reported one.
#[must_use]
pub fn spend_on(spends: &[ra_usage::Spend], today: &str) -> (u64, Option<f64>) {
    spends
        .iter()
        .filter(|s| s.at.starts_with(today))
        .fold((0, None), |(calls, cost), s| (calls + 1, s.cost_usd.map_or(cost, |c| Some(cost.unwrap_or(0.0) + c))))
}

/// The width of the run column of a status line.
const RUN_WIDTH: usize = 44;

impl Status {
    /// The line of the status: the project, the run (or `idle`, or `orphaned: …`), the ready
    /// count, today's calls and dollars, and the last end.
    #[must_use]
    pub fn line(&self) -> String {
        let run = match &self.run {
            Run::Idle => String::from("idle"),
            Run::Live(text) => clip(text, RUN_WIDTH),
            Run::Orphaned(text) => clip(&format!("orphaned: {text}"), RUN_WIDTH),
        };
        let ready = self.ready.map_or(String::from("—"), |n| n.to_string());
        let today = match (self.calls_today, self.spend_today) {
            (0, _) => String::from("—"),
            (calls, Some(cost)) => format!("{calls} calls ${cost:.2}"),
            (calls, None) => format!("{calls} calls"),
        };
        // the end line's head says the card and how it ended; its tail is the cost, which `today` has
        let last = self.last.as_deref().map_or("—", |last| last.split_once(" · ").map_or(last, |(head, _)| head));
        format!("{:<20} {run:<RUN_WIDTH$} {ready:>5} {today:>16}  {last}", clip(&self.name, 20))
    }

    /// The header the lines go under.
    #[must_use]
    pub fn header() -> String {
        format!("{:<20} {:<RUN_WIDTH$} {:>5} {:>16}  {}", "project", "run", "ready", "today", "last")
    }
}

/// `text` within `width` characters, the last one `…` when it was longer.
fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(width - 1).collect();
    clipped.push('…');
    clipped
}

/// A unix time as its UTC day, `YYYY-MM-DD`, the way the ledger stamps a row.
#[must_use]
pub fn day_of(when: u64) -> String {
    // days to a civil date (Hinnant), the era of 400 years first
    let z = i64::try_from(when / 86_400).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A lock under `locks/<name>.lock` in the hub, shared by every project on the machine: the
/// cargo target two projects build in, around a check; the install directory, around
/// `rc install` and `rc restart`. Dropping it, or the holder's death, releases it.
pub struct Guard(#[allow(dead_code)] std::fs::File);

/// The locks' directory under the hub.
fn locks_dir() -> PathBuf {
    dir().join("locks")
}

/// Takes the hub's lock `name` once: `Ok(None)` while another holder, in this process or
/// another, has it.
///
/// # Errors
/// When the lock file cannot be made or opened.
pub fn try_guard(name: &str) -> anyhow::Result<Option<Guard>> {
    try_guard_in(&locks_dir(), name)
}

/// [`try_guard`] in the directory named, for the tests.
///
/// # Errors
/// When the lock file cannot be made or opened.
pub fn try_guard_in(dir: &Path, name: &str) -> anyhow::Result<Option<Guard>> {
    std::fs::create_dir_all(dir).with_context(|| format!("cannot make {}", dir.display()))?;
    let path = dir.join(format!("{name}.lock"));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    // an OS lock on the open file, not the open itself: it goes with the holder, dead or done
    match file.try_lock() {
        Ok(()) => Ok(Some(Guard(file))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(anyhow::Error::new(e).context(format!("cannot lock {}", path.display()))),
    }
}

/// Takes the hub's lock `name`, waiting on whoever holds it, `poll` between tries; `waited(n)`
/// hears of the `n`th try that found it held, and a `false` from it gives up with `Ok(None)`.
///
/// # Errors
/// When the lock file cannot be made or opened.
pub fn guard_in(dir: &Path, name: &str, poll: std::time::Duration, mut waited: impl FnMut(u32) -> bool) -> anyhow::Result<Option<Guard>> {
    let mut tries = 0;
    loop {
        if let Some(guard) = try_guard_in(dir, name)? {
            return Ok(Some(guard));
        }
        tries += 1;
        if !waited(tries) {
            return Ok(None);
        }
        std::thread::sleep(poll);
    }
}

/// Takes the hub's lock `name`, waiting on whoever holds it. `None` only when the lock cannot be
/// opened at all, which is said once and is nobody's failure: the caller goes on unguarded.
#[must_use]
pub fn guard(name: &str) -> Option<Guard> {
    match guard_in(&locks_dir(), name, std::time::Duration::from_millis(200), |_| true) {
        Ok(guard) => guard,
        Err(e) => {
            tracing::warn!("the hub's lock {name} was not taken: {e:#}");
            None
        }
    }
}

/// The lock `rc install` and `rc restart` take: two of them at once would set the same binary
/// aside twice.
pub const INSTALL_LOCK: &str = "install";

/// The file under the hub that orders the projects by priority, one directory name a line,
/// the first served first: the whole pool of writers and builders goes to the first project
/// with work, the next takes what that one leaves, and so on down. The person's rule of
/// 2026-11-12: "agent -> runelite -> asm … all 12 going to whichever project has priority
/// first … knock out the last of runelite then automatically move all resources to asm".
const PRIORITY_FILE: &str = "priority.txt";

/// The projects in priority order, by directory name; empty when none was set, and then
/// the pool is shared by shares as before.
#[must_use]
pub fn priority() -> Vec<String> {
    std::fs::read_to_string(dir().join(PRIORITY_FILE))
        .map(|text| text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Sets the priority order; an empty list clears it.
///
/// # Errors
/// When the hub's file cannot be written.
pub fn set_priority(names: &[String]) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir()).with_context(|| format!("cannot make {}", dir().display()))?;
    let path = dir().join(PRIORITY_FILE);
    if names.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    std::fs::write(&path, format!("{}\n", names.join("\n"))).with_context(|| format!("cannot write {}", path.display()))
}

/// A project's rank in the priority order: 0 first; a project not in the list comes after
/// every listed one; `None` when no order is set.
#[must_use]
pub fn rank_of(root: &Path) -> Option<usize> {
    let order = priority();
    if order.is_empty() {
        return None;
    }
    let name = project_name(&project_root_of(root));
    Some(order.iter().position(|n| n.eq_ignore_ascii_case(&name)).unwrap_or(order.len()))
}

/// Takes the hub's lock `name` once, as [`try_guard`], and notes who holds it in a file
/// beside the lock, so the other projects can count a project's slots ([`held_by`]).
///
/// # Errors
/// When the lock file cannot be made or opened.
pub fn try_guard_as(name: &str, who: &str) -> anyhow::Result<Option<Guard>> {
    let guard = try_guard(name)?;
    if guard.is_some() {
        let _ = std::fs::write(locks_dir().join(format!("{name}.who")), who);
    }
    Ok(guard)
}

/// How many of the locks named `prefix-<n>`, `n` under `count`, are held right now by `who`:
/// a held lock whose note names them. A note beside a free lock is stale and counts for nothing.
#[must_use]
pub fn held_by(prefix: &str, count: usize, who: &str) -> usize {
    (0..count)
        .filter(|slot| {
            let name = format!("{prefix}-{slot}");
            let held = matches!(try_guard(&name), Ok(None));
            held && std::fs::read_to_string(locks_dir().join(format!("{name}.who"))).is_ok_and(|w| w.trim().eq_ignore_ascii_case(who))
        })
        .count()
}

/// The builds the machine runs at once, across every project's workers and checks: four
/// builders beside the twelve writers. With a priority order set, the first project walking
/// with work may use all four and a project below it the last two; without one, a project
/// has two of its own and the two spare when it walks alone. The person's rules of
/// 2026-11-12: "12 opus researchers, and 4 runners overall", "all 12 … and 4 builders [to]
/// whichever project has priority first".
pub const BUILD_SLOTS: usize = 4;

/// The build slots a project keeps for itself without a priority order, and the slots a
/// project below the first in the order may use.
pub const BUILD_SHARE: usize = 2;

/// The project a directory works in: itself, or the project above a card's worktree
/// (`<root>/var/worktrees/<id>`), whose build slots the worktree's builds take.
#[must_use]
pub fn project_root_of(dir: &Path) -> PathBuf {
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let under_worktrees = dir.parent().and_then(Path::file_name).is_some_and(|n| n == "worktrees")
        && dir.parent().and_then(Path::parent).and_then(Path::file_name).is_some_and(|n| n == "var");
    if under_worktrees && let Some(root) = dir.ancestors().nth(3) {
        return root.to_path_buf();
    }
    dir
}

/// The name of the hub's lock on one of the machine's build slots.
#[must_use]
pub fn build_lock_name(slot: usize) -> String {
    format!("build-{slot}")
}

/// Takes one of the machine's [`BUILD_SLOTS`] once: `Ok(None)` while every slot the project
/// may take is held. With a priority order, the first project walking may take any of the
/// four and a project behind a walking one only the last [`BUILD_SHARE`]; without an order,
/// a project takes its own two (the first two for the project first in the hub's open list,
/// the last two for the next) and any of the four when it walks alone.
///
/// # Errors
/// When a lock file cannot be made or opened.
pub fn try_build_slot(root: &Path) -> anyhow::Result<Option<Guard>> {
    let project_root = project_root_of(root);
    let project = project_name(&project_root);
    // the slot's note names the card too when the build is a worktree's, for the window's
    // builders row: `runelite #42`
    let card = std::fs::canonicalize(root)
        .ok()
        .filter(|dir| dir.parent().and_then(Path::file_name).is_some_and(|n| n == "worktrees"))
        .and_then(|dir| dir.file_name().map(|n| n.to_string_lossy().into_owned()));
    let who = match &card {
        Some(card) => format!("{project} #{card}"),
        None => project.clone(),
    };
    let higher = crate::agent::higher_walking(&project_root);
    let alone = crate::agent::other_walks(&project_root) == 0;
    let slots: Vec<usize> = match rank_of(&project_root) {
        _ if alone => (0..BUILD_SLOTS).collect(),
        Some(_) if higher.is_empty() => (0..BUILD_SLOTS).collect(),
        Some(_) => (BUILD_SLOTS - BUILD_SHARE..BUILD_SLOTS).collect(),
        None => {
            // no order: the first project of the hub's open list has the first two slots
            let mine = recent().iter().position(|d| project_name(d).eq_ignore_ascii_case(&project)).unwrap_or(0);
            if mine == 0 { (0..BUILD_SHARE).collect() } else { (BUILD_SLOTS - BUILD_SHARE..BUILD_SLOTS).collect() }
        }
    };
    for slot in slots {
        if let Some(guard) = try_guard_as(&build_lock_name(slot), &who)? {
            return Ok(Some(guard));
        }
    }
    Ok(None)
}

/// The machine's build slots as the window shows them: for each, who holds it (`runelite #42`)
/// and since when (unix seconds), or `None` when it is free. A slot is asked by taking its lock
/// and letting it go at once; a builder that tries in that instant tries again on its next poll.
#[must_use]
pub fn build_slots() -> Vec<Option<(String, u64)>> {
    (0..BUILD_SLOTS)
        .map(|slot| {
            let name = build_lock_name(slot);
            if !matches!(try_guard(&name), Ok(None)) {
                return None;
            }
            let note = locks_dir().join(format!("{name}.who"));
            let who = std::fs::read_to_string(&note).map(|w| w.trim().to_owned()).unwrap_or_default();
            let since = std::fs::metadata(&note)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            Some((if who.is_empty() { "a build".to_owned() } else { who }, since))
        })
        .collect()
}

/// The cargo target a check in `root` builds in: `CARGO_TARGET_DIR` when set (a relative one
/// under `root`), else `target` under the outermost workspace above `root`, or `root` itself.
#[must_use]
pub fn cargo_target(root: &Path) -> PathBuf {
    cargo_target_with(root, std::env::var_os("CARGO_TARGET_DIR").as_deref())
}

/// [`cargo_target`] with `CARGO_TARGET_DIR` as named, for the tests.
#[must_use]
pub fn cargo_target_with(root: &Path, target_dir: Option<&std::ffi::OsStr>) -> PathBuf {
    if let Some(dir) = target_dir {
        let dir = PathBuf::from(dir);
        return if dir.is_absolute() { dir } else { root.join(dir) };
    }
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let is_workspace =
        |dir: &Path| std::fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|t| t.lines().any(|l| l.trim() == "[workspace]"));
    root.ancestors().filter(|dir| is_workspace(dir)).last().unwrap_or(&root).join("target")
}

/// The name of the hub's lock on a cargo target: `target-<hash of the path>`, the same from
/// every project that builds there.
#[must_use]
pub fn target_lock_name(target: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let target = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let mut text = target.display().to_string().trim_start_matches(r"\\?\").replace('\\', "/");
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    let mut hasher = std::hash::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("target-{:016x}", hasher.finish())
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
    // one install or restart at a time across the projects: the binary set aside once
    let _install = guard(INSTALL_LOCK);
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
    detach(&mut command);
    command.spawn().with_context(|| format!("cannot start {}", gui.display()))?;
    Ok(gui)
}

/// Makes a command run on its own: no stdio of this process, its own process group, and on
/// Windows no console and none of this process's inheritable handles, so the shell that asked
/// does not wait on it and a window or a session that started it can go while it keeps going.
/// The window starts a walk this way, and `rc open` the window.
pub fn detach(command: &mut std::process::Command) {
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // a hidden console rather than none: a console child of a process without one (the
        // harness under a walk the window started) would open an empty console window of its
        // own; under a hidden one it inherits that and shows nothing
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
        // the shell's pipes came to this process as inheritable handles, and a child inherits
        // every inheritable handle whatever its own stdio is set to; the shell would then wait
        // on the child for the pipe to close. They are made private before the spawn.
        keep_std_handles_private();
    }
}

/// Whether a process with this pid is running: on Windows asked of the process itself, elsewhere
/// of `/proc`. A pid that cannot be asked counts as alive, so nothing is called gone on a doubt.
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const STILL_ACTIVE: u32 = 259;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn GetExitCodeProcess(handle: *mut std::ffi::c_void, code: *mut u32) -> i32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        // SAFETY: a handle opened here is queried and closed here; a null one is not used
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                // no such process, or one this process may not look at: the latter is alive
                return std::io::Error::last_os_error().raw_os_error() != Some(87);
            }
            let mut code = 0u32;
            let asked = GetExitCodeProcess(handle, &raw mut code) != 0;
            CloseHandle(handle);
            !asked || code == STILL_ACTIVE
        }
    }
    #[cfg(not(windows))]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
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

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests;
