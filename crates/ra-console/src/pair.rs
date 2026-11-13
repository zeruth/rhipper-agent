//! The link to a paired session: an external agent (a Claude Code session, say) that drives
//! and reviews this agent. The agent cannot push into such a session, but the session can
//! watch a file; so the agent keeps `var/pair.log`, one line per event — a run's start, its
//! end with the outcome, a walk's end, a note — and the pairing skill arms a watch on it first
//! thing. `rc pair` prints the latest; `rc pair --since <ts>` the ones after a time, and
//! `--follow` keeps printing them, so a watch that expired catches up with what it missed.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The feed, under the workspace: `time<TAB>text`.
pub const PAIR_FILE: &str = "var/pair.log";

/// Appends one event. A feed that cannot be written is nobody's failure: the run goes on.
pub fn event(root: &Path, text: &str) {
    let path = root.join(PAIR_FILE);
    let _ = std::fs::create_dir_all(root.join("var"));
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{when}\t{}", text.replace(['\t', '\n'], " "));
    }
}

/// What a walk did with card `id`, which a run left needing revision because of `why`: reran it
/// by itself, `n` of the `of` reruns it gives (`rerun`), or left it for the person. The event
/// goes on the feed, so the paired session knows a re-queue is its to make; the line is the walk's.
#[must_use]
pub fn sent_back(root: &Path, id: i64, title: &str, why: &str, rerun: Option<(usize, usize)>) -> String {
    let text = match rerun {
        Some((n, of)) => format!("rerun #{id} {title} · {why} · the walk takes it up again on its session ({n} of {of})"),
        None => format!("waits #{id} {title} · {why} · waits for the person: `rc set {id} defined` re-queues it"),
    };
    event(root, &text);
    text
}

/// The latest `limit` events, oldest first; none without a feed.
#[must_use]
pub fn recent(root: &Path, limit: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join(PAIR_FILE)) else { return Vec::new() };
    let lines: Vec<String> = text.lines().map(|l| l.split_once('\t').map_or(l, |(_, t)| t).to_owned()).collect();
    lines[lines.len().saturating_sub(limit)..].to_vec()
}

/// The latest `end` event without its word: the last card that ended and how, for the
/// project's digest; none while no run has ended.
#[must_use]
pub fn last_end(root: &Path) -> Option<String> {
    recent(root, 50).into_iter().rev().find_map(|line| line.strip_prefix("end ").map(str::trim).map(str::to_owned))
}

/// The run the feed has open: the latest `start` event with no `end` (or `re-queued`, or the
/// older `run orphaned`) after it, without its word, and the pid its marker carries last; none
/// while nothing runs. A start without a pid is from before the marker carried one, and its
/// liveness cannot be judged.
#[must_use]
pub fn live_start(root: &Path) -> Option<(String, Option<u32>)> {
    let open = recent(root, 50).into_iter().rev().find_map(|line| {
        if line.starts_with("end ") || line.starts_with("re-queued ") || line.starts_with("run orphaned") {
            return Some(None);
        }
        line.strip_prefix("start ").map(|text| Some(text.trim().to_owned()))
    })??;
    let pid = open.rsplit_once(" · pid ").and_then(|(_, pid)| pid.trim().parse().ok());
    Some((open, pid))
}

/// The lines written after `since` (a unix time, exclusive), oldest first, as `time<TAB>text`
/// so the reader knows where it got to.
#[must_use]
pub fn since(root: &Path, since: u64) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join(PAIR_FILE)) else { return Vec::new() };
    text.lines().filter(|l| after(l, since)).map(str::to_owned).collect()
}

/// Whether a feed line is stamped later than `since`; an unstamped line counts as new.
fn after(line: &str, since: u64) -> bool {
    line.split_once('\t').and_then(|(t, _)| t.parse::<u64>().ok()).is_none_or(|t| t > since)
}

/// A reader that follows the feed by name: each poll yields the whole lines written since the
/// last one, and a file that shrank or went away (a rotation by rename) is read again from its
/// start, so the new file's lines are not missed.
pub struct Follower {
    path: PathBuf,
    since: u64,
    offset: u64,
    partial: Vec<u8>,
}

impl Follower {
    /// Starts after `since`: the first poll replays what the file holds after that time.
    #[must_use]
    pub fn new(root: &Path, since: u64) -> Self {
        Self { path: root.join(PAIR_FILE), since, offset: 0, partial: Vec::new() }
    }

    /// The whole lines appended since the last poll, stamped, later than `since`.
    #[must_use]
    pub fn poll(&mut self) -> Vec<String> {
        let Ok(mut file) = std::fs::File::open(&self.path) else { return Vec::new() };
        let len = file.metadata().map_or(0, |m| m.len());
        if len < self.offset {
            // the file was replaced: a rename rotation, read the new one whole
            self.offset = 0;
            self.partial.clear();
        }
        if std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        let Ok(read) = file.read_to_end(&mut bytes) else { return Vec::new() };
        self.offset += read as u64;
        self.partial.extend_from_slice(&bytes);
        let mut lines = Vec::new();
        while let Some(end) = self.partial.iter().position(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(&self.partial[..end]).trim_end_matches('\r').to_owned();
            self.partial.drain(..=end);
            if after(&line, self.since) {
                lines.push(line);
            }
        }
        lines
    }
}

/// Prints the lines after `since`, then every new one as it lands, flushed, until the process
/// is killed. Polls four times a second: a line shows within that of its write.
///
/// # Errors
/// When `out` cannot be written: a closed pipe, which is how the follow ends.
pub fn follow(root: &Path, since: u64, out: &mut impl Write) -> std::io::Result<()> {
    let mut follower = Follower::new(root, since);
    loop {
        for line in follower.poll() {
            writeln!(out, "{line}")?;
        }
        out.flush()?;
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// The window has come up on this project: `window up <build> pid <pid> tabs <n>`, then the
/// runs the window took down with it go back on the queue through
/// [`crate::agent::requeue_orphans`], one `re-queued …` line for all of them. A run whose
/// process lives is left alone, the window owns no run's lifetime.
pub async fn window_up(store: &ra_roadmap::store::Store, root: &Path, build: &str, tabs: usize) {
    window_up_with(store, root, build, tabs, crate::hub::pid_alive).await;
}

/// [`window_up`] with the liveness check given, for the tests.
pub async fn window_up_with(store: &ra_roadmap::store::Store, root: &Path, build: &str, tabs: usize, alive: impl Fn(u32) -> bool) {
    event(root, &format!("window up {build} pid {} tabs {tabs}", std::process::id()));
    crate::agent::requeue_orphans_with(store, root, alive).await;
}

/// The cards of the runs in the activity log that started, never ended, and whose process is
/// gone, each once in the order they started, however many killed walks started it. Every such
/// run gets its end in the activity log, so its bubble closes and the next restart finds none
/// of them: a restart lists an orphan once.
#[must_use]
pub fn orphaned(root: &Path, alive: impl Fn(u32) -> bool) -> Vec<i64> {
    let activity_path = root.join(crate::ui::ACTIVITY_FILE);
    let activity = std::fs::read_to_string(&activity_path).unwrap_or_default();
    let gone = orphans(&activity, alive);
    let mut log = std::fs::OpenOptions::new().append(true).open(&activity_path).ok();
    let mut ids: Vec<i64> = Vec::new();
    for (id, pid) in gone {
        if let Some(log) = log.as_mut() {
            let _ = writeln!(log, "{}✗ orphaned #{id}: process {pid} is gone", crate::logs::RUN_END);
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// The window has gone from this project: `window down <reason>`, the reason `restart` under
/// `rc restart`, `close` when the person closed it.
pub fn window_down(root: &Path, reason: &str) {
    event(root, &format!("window down {reason}"));
}

/// The runs in the activity text that started with a pid, were not ended, and whose process is
/// gone by `alive`: `(card, pid)` in the order they started. An end marker closes the newest
/// open run, as the log pane does, unless it is an orphan's end, which names its card; a start
/// without a pid cannot be judged and is not.
fn orphans(activity: &str, alive: impl Fn(u32) -> bool) -> Vec<(i64, u32)> {
    let mut open: Vec<(i64, Option<u32>)> = Vec::new();
    for line in activity.lines() {
        // a walk of several cards tags each line with its card; the markers sit after the tag
        let (tag, line) = crate::logs::untag_card(line);
        if let Some(title) = line.strip_prefix(crate::logs::RUN_START) {
            let pid = title.rsplit_once(" · pid ").and_then(|(_, pid)| pid.trim().parse().ok());
            if let Some(id) = crate::logs::card_of(title) {
                open.push((id, pid));
            }
        } else if let Some(outcome) = line.strip_prefix(crate::logs::RUN_END) {
            if let Some(at) = crate::logs::end_closes(&open, outcome, tag, |(id, pid)| (Some(*id), *pid)) {
                open.remove(at);
            }
        }
    }
    open.into_iter().filter_map(|(id, pid)| pid.map(|pid| (id, pid))).filter(|(_, pid)| !alive(*pid)).collect()
}

#[cfg(test)]
mod tests;
