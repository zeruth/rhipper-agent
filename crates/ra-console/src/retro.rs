//! The worker's own look back, in debug mode: a run ends its report with a short `Faster next
//! time` section, at most three one-line bullets, each what cost turns or time and the one
//! change that would have avoided it, `[machine]` when the cause is this machine. It costs no
//! second call: the ask rides in the system prompt, the bullets ride in the report. The
//! orchestrator keeps them in `var/retro.log`, names them in the run's result, and hands the
//! latest to the next runs as machine notes, so one worker's hindsight is the next one's
//! foresight. A `[machine]` bullet is about the machine, not the project, so it is promoted to
//! the shared store under the hub directory too, and every project's runs read it: a lesson
//! about this machine is paid for once.

use std::path::{Path, PathBuf};

use crate::plan::evidence::Line;

/// Where the bullets are kept, under the workspace: `time<TAB>node<TAB>machine<TAB>text`.
pub const RETRO_FILE: &str = "var/retro.log";

/// The shared store of the `[machine]` bullets of every project on this machine, under the hub
/// directory: `time<TAB>project<TAB>kind<TAB>text`. The tests' hub is a temp directory of
/// their own, so their notes never reach the machine's.
#[must_use]
pub fn shared_file() -> PathBuf {
    crate::hub::dir().join("notes.log")
}

/// The heading the worker writes, matched without its marks and its case.
pub const HEADING: &str = "faster next time";

/// How many of the latest bullets the next run reads.
pub const SHOWN: usize = 5;

/// What the system prompt asks for in debug mode.
pub const PROMPT: &str = "End your report with a section headed `## Faster next time`: at most three bullets of one line each, naming what cost this run turns or time (a refused command, a file you had to search for, a check you ran twice, a quirk of this machine) and the one change that would have avoided it; start a bullet with [machine] when the cause is this machine rather than the task. Nothing else in that section; leave it out when nothing cost anything.";

/// The look-back's key in the role table of [`ra_providers::route::TABLE`]: a role that only
/// summarises, so it takes the local server whenever it answers.
pub const ROLE: &str = "retro";

/// What the classify role is told about a bullet the worker left untagged: the workers forget
/// the tag more often than they misplace it, so the cheap model is asked, never the frontier one.
pub const CLASSIFY: &str = "Answer with one word, machine or task. Is this note about the machine it ran on (its shells, hooks, installed programs, paths, running processes, network) rather than about the task and its code?";

/// The verdict of a classify answer: `Some(true)` for the machine, `Some(false)` for the task,
/// `None` when the answer says neither.
#[must_use]
pub fn classified(answer: &str) -> Option<bool> {
    let word = answer.trim().trim_matches(['*', '`', '"', '\'', '.', '!', ' ']).to_ascii_lowercase();
    match word.split(|c: char| !c.is_ascii_alphabetic()).next()? {
        "machine" => Some(true),
        "task" => Some(false),
        _ => None,
    }
}

/// One bullet of a look back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// When the run ended, as seconds since the Unix epoch; zero for a bullet not yet kept.
    pub when: u64,
    pub node: Option<i64>,
    /// Whether the worker marked the cause as this machine's.
    pub machine: bool,
    /// Whether the note came from the person or a paired session (`rc note`, the `note` tool)
    /// rather than from a run.
    pub pair: bool,
    pub text: String,
}

/// Keeps an observation of the person or a paired session, for the next runs to read with
/// the workers' own; one that starts with `[machine]` is shared with every project.
///
/// # Errors
/// When the text is empty or the file cannot be written.
pub fn record_pair(root: &Path, text: &str) -> anyhow::Result<()> {
    let (machine, text) = match text.trim().strip_prefix("[machine]") {
        Some(rest) => (true, rest.trim()),
        None => (false, text.trim()),
    };
    anyhow::ensure!(!text.is_empty(), "a note says something");
    let note = Note { when: 0, node: None, machine, pair: true, text: text.chars().take(400).collect() };
    crate::pair::event(root, &format!("note {}{}", if machine { "[machine] " } else { "" }, note.text));
    record(root, None, &[note])
}

/// The bullets of a report's `Faster next time` section, in order; none without the section.
/// A bullet keeps one line, loses its `[machine]` tag into the flag, and clips at 240 characters.
#[must_use]
pub fn parse(report: &str) -> Vec<Note> {
    let mut notes = Vec::new();
    let mut inside = false;
    for line in report.lines() {
        let trimmed = line.trim();
        let bare = trimmed.trim_start_matches(['#', '*', '_', ' ']).trim_end_matches(['*', '_', ':', ' ']).to_ascii_lowercase();
        if bare == HEADING {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if trimmed.starts_with('#') {
            break;
        }
        let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")).or_else(|| trimmed.strip_prefix("• ")) else {
            if !trimmed.is_empty() && !notes.is_empty() {
                break;
            }
            continue;
        };
        let (machine, text) = match item.trim().strip_prefix("[machine]") {
            Some(rest) => (true, rest.trim()),
            None => (false, item.trim()),
        };
        if text.is_empty() {
            continue;
        }
        let text: String = text.chars().take(240).collect();
        notes.push(Note { when: 0, node: None, machine, pair: false, text });
        if notes.len() == 3 {
            break;
        }
    }
    notes
}

/// Keeps the bullets of a run of `node`, newest last; the `[machine]` ones go to the shared
/// store too.
///
/// # Errors
/// When the file cannot be written.
pub fn record(root: &Path, node: Option<i64>, notes: &[Note]) -> anyhow::Result<()> {
    record_with(root, &shared_file(), node, notes)
}

/// `record` with the shared store named, for the tests.
///
/// # Errors
/// When the project's file cannot be written; a shared store that cannot be is only logged,
/// since the project's own lesson is kept either way.
pub fn record_with(root: &Path, shared: &Path, node: Option<i64>, notes: &[Note]) -> anyhow::Result<()> {
    if notes.is_empty() {
        return Ok(());
    }
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let node = node.map_or(String::from("-"), |n| n.to_string());
    append(&root.join(RETRO_FILE), notes.iter().map(|note| format!("{when}\t{node}\t{}\t{}", kind_of(note), clean(&note.text))))?;
    let project = project_name(root);
    let general =
        notes.iter().filter(|note| note.machine).map(|note| format!("{when}\t{project}\t{}\t{}", kind_of(note), clean(&note.text)));
    if let Err(e) = append(shared, general) {
        tracing::warn!("the shared machine notes were not written: {e:#}");
    }
    Ok(())
}

/// The tag of the line a walk writes when it reads a planned card's claimed saving back.
pub const SAVING: &str = "[saving]";

/// Keeps the walk's verdict on a planned card's claim, `[saving] #id held: …` or `missed`,
/// under the card, so the next plan reads it as evidence; the line as written.
///
/// # Errors
/// When the file cannot be written.
pub fn record_saving(root: &Path, node: i64, held: bool, why: &str) -> anyhow::Result<String> {
    let text = format!("{SAVING} #{node} {}: {why}", if held { "held" } else { "missed" });
    record(root, Some(node), &[Note { when: 0, node: Some(node), machine: false, pair: false, text: text.clone() }])?;
    Ok(text)
}

/// How many landed cards a card is measured against: the median of the last twenty.
pub const MEDIAN_OF: u32 = 20;

/// The walk's verdict on a card's claim the moment it lands, measured: the card's own turns,
/// tokens a turn and cost from the ledger against the median tokens a turn of the last
/// [`MEDIAN_OF`] cards the ledger wrote up to `mark` (the newest row before the run), the card
/// itself left out; held when the card's are fewer. One `[saving]` line under the card in
/// `var/retro.log`, the card's own claim quoted beside the figures, returned too; nothing when
/// the card did not land, claims no saving, was judged already, or the ledger has no rows on
/// one side (the day-later reading of `plan::read_back` takes that claim). A claim that holds
/// across projects, its evidence cited from another project's `notes.log`, is a `[machine]`
/// note too: it reaches the hub, and every project's planner reads it through [`evidence`] as
/// a lesson that paid off twice; a worker's prompt ([`machine_lines`]) leaves every verdict out.
pub async fn landed(root: &Path, card: &ra_roadmap::model::Node, ledger: &ra_usage::UsageStore, mark: i64) -> Vec<String> {
    use ra_roadmap::model::TaskState;
    let Some(saving) = card.saving.as_deref() else { return Vec::new() };
    if !matches!(card.state, TaskState::Implemented | TaskState::Accepted) || saving_read(root, card.id) {
        return Vec::new();
    }
    let (Ok(own), Ok(others)) = (ledger.card_cost(card.id).await, ledger.recent_cards(mark, MEDIAN_OF + 1).await) else {
        return Vec::new();
    };
    let Some(median) = median_tokens_per_turn(others.iter().filter(|c| c.node != card.id).take(MEDIAN_OF as usize)) else {
        return Vec::new();
    };
    let per_turn = own.tokens_per_turn();
    if per_turn == 0 {
        return Vec::new();
    }
    let held = per_turn < median;
    let across = card.evidence.as_deref().is_some_and(|e| e.contains("notes.log"));
    let cost = own.totals.cost_usd.map(|c| format!(", ${c:.2}")).unwrap_or_default();
    let text = format!(
        "{SAVING} #{} {}: {} turns, {per_turn} tokens/turn{cost} vs median {median}; claimed \"{saving}\"",
        card.id,
        if held { "held" } else { "missed" },
        own.totals.turns
    );
    let note = Note { when: 0, node: Some(card.id), machine: held && across, pair: false, text: text.clone() };
    match record(root, Some(card.id), &[note]) {
        Ok(()) => vec![text],
        Err(e) => {
            tracing::warn!("the saving of #{} was not recorded: {e:#}", card.id);
            Vec::new()
        }
    }
}

/// The median tokens a turn over `cards`, the lower middle of an even count; `None` when there
/// are none or none had a turn, so a first card is not judged against nothing.
fn median_tokens_per_turn<'a>(cards: impl Iterator<Item = &'a ra_usage::CardCost>) -> Option<u64> {
    let mut rates: Vec<u64> = cards.map(ra_usage::CardCost::tokens_per_turn).filter(|&rate| rate > 0).collect();
    rates.sort_unstable();
    rates.get((rates.len().checked_sub(1)?) / 2).copied()
}

/// Whether the log already holds the walk's verdict on the card's claim: a claim is read back
/// once.
#[must_use]
pub fn saving_read(root: &Path, node: i64) -> bool {
    parse_log(&root.join(RETRO_FILE)).iter().any(|(_, _, note)| note.node == Some(node) && note.text.starts_with(SAVING))
}

/// Appends `lines` to `path`, making its directory; nothing is touched when there are none.
fn append(path: &Path, lines: impl Iterator<Item = String>) -> anyhow::Result<()> {
    use std::io::Write;
    let lines: Vec<String> = lines.collect();
    if lines.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    for line in lines {
        writeln!(file, "{line}")?;
    }
    Ok(())
}

/// The third column: 0 a worker's, 1 a worker's about this machine, 2 the person's, 3 the
/// person's about this machine.
fn kind_of(note: &Note) -> u8 {
    u8::from(note.machine) + if note.pair { 2 } else { 0 }
}

fn clean(text: &str) -> String {
    text.replace(['\t', '\n'], " ")
}

/// How the project is named in the shared store: its directory's name.
fn project_name(root: &Path) -> String {
    std::fs::canonicalize(root)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| String::from("-"))
}

/// The latest `limit` bullets, newest first; none without a file.
#[must_use]
pub fn recent(root: &Path, limit: usize) -> Vec<Note> {
    let mut notes: Vec<Note> = parse_log(&root.join(RETRO_FILE)).into_iter().map(|(_, _, note)| note).collect();
    notes.reverse();
    notes.truncate(limit);
    notes
}

/// The latest `limit` shared bullets of the other projects on this machine, newest first, each
/// with the project that learned it; none without a store.
#[must_use]
pub fn shared_recent(root: &Path, shared: &Path, limit: usize) -> Vec<(String, Note)> {
    let this = project_name(root);
    let mut notes: Vec<(String, Note)> =
        parse_log(shared).into_iter().filter(|(_, project, _)| *project != this).map(|(_, project, note)| (project, note)).collect();
    notes.reverse();
    notes.truncate(limit);
    notes
}

/// The lines of a log, oldest first, each with its line number (from 1) and its second column
/// (the node of a project's log, the project of the shared one).
fn parse_log(path: &Path) -> Vec<(usize, String, Note)> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let mut cols = line.split('\t');
            let (when, second, kind, text) = (cols.next()?, cols.next()?, cols.next()?, cols.next()?);
            let note = Note {
                when: when.parse().ok()?,
                node: second.parse().ok(),
                machine: matches!(kind, "1" | "3"),
                pair: matches!(kind, "2" | "3"),
                text: text.to_owned(),
            };
            Some((index + 1, second.to_owned(), note))
        })
        .collect()
}

/// How many of the latest lines of each log the planner reads as evidence.
pub const EVIDENCE_LINES: usize = 10;

/// The look-backs as the planner's evidence: the latest lines of the project's log, then the
/// latest shared notes of the other projects, each oldest first and cited by file and line
/// number (`retro.log:12`, `notes.log:3`), so a planned card names the line it comes from and
/// a reader can open it. None without a log.
#[must_use]
pub fn evidence(root: &Path) -> Vec<Line> {
    evidence_with(root, &shared_file())
}

/// `evidence` with the shared store named, for the tests.
#[must_use]
pub fn evidence_with(root: &Path, shared: &Path) -> Vec<Line> {
    let this = project_name(root);
    let own = parse_log(&root.join(RETRO_FILE)).into_iter().map(|(line, _, note)| {
        let who = match (note.pair, note.node) {
            (true, _) => "the person".to_owned(),
            (false, Some(node)) => format!("card #{node}"),
            (false, None) => "a run".to_owned(),
        };
        Line { source: format!("retro.log:{line}"), text: format!("{who}: {}", tagged(&note)) }
    });
    let shared = parse_log(shared)
        .into_iter()
        .filter(|(_, project, _)| *project != this)
        .map(|(line, project, note)| Line { source: format!("notes.log:{line}"), text: format!("({project}) {}", tagged(&note)) });
    tail(own.collect()).into_iter().chain(tail(shared.collect())).collect()
}

/// A note's text with its `[machine]` tag back on, so the planner tells a machine lesson.
fn tagged(note: &Note) -> String {
    if note.machine { format!("[machine] {}", note.text) } else { note.text.clone() }
}

/// The last `EVIDENCE_LINES` of `all`, in their order.
fn tail<T>(mut all: Vec<T>) -> Vec<T> {
    let skip = all.len().saturating_sub(EVIDENCE_LINES);
    all.drain(..skip);
    all
}

/// The latest bullets as a block for the system prompt of the next run, each once, and under
/// it what the other projects on this machine learned about it; empty without any.
#[must_use]
pub fn machine_lines(root: &Path) -> String {
    machine_lines_with(root, &shared_file())
}

/// `machine_lines` with the shared store named, for the tests.
#[must_use]
pub fn machine_lines_with(root: &Path, shared: &Path) -> String {
    let mut seen: Vec<String> = Vec::new();
    for note in recent(root, SHOWN * 2) {
        // the walk's verdicts on the plan's claims are the planner's to read, not a worker's
        if note.text.starts_with(SAVING) {
            continue;
        }
        // the person's notes are told from the workers' own
        let line = if note.pair { format!("(the person) {}", note.text) } else { note.text };
        if seen.len() == SHOWN || seen.contains(&line) {
            continue;
        }
        seen.push(line);
    }
    let mut blocks = Vec::new();
    if !seen.is_empty() {
        let lines: Vec<String> = seen.iter().map(|t| format!("- {t}")).collect();
        blocks.push(format!("What earlier runs here, and the person, said would have made them faster:\n{}", lines.join("\n")));
    }
    // a lesson this project already holds is not repeated from another's
    let mut elsewhere: Vec<String> = Vec::new();
    for (project, note) in shared_recent(root, shared, SHOWN * 2) {
        let known = seen.iter().any(|s| s.ends_with(&note.text));
        // a verdict another project shared is the planner's too
        if note.text.starts_with(SAVING) || elsewhere.len() == SHOWN || known || elsewhere.iter().any(|s| s.ends_with(&note.text)) {
            continue;
        }
        elsewhere.push(format!("- ({project}) {}", note.text));
    }
    if !elsewhere.is_empty() {
        blocks.push(format!("What runs in other projects on this machine learned about it:\n{}", elsewhere.join("\n")));
    }
    blocks.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_section_parses_to_its_bullets_and_nothing_else() {
        let report = "Done. Added the thing.\n\n## Faster next time\n- [machine] cargo test relinks rc.exe while the window runs: test with -p\n- the card named no files; a files line would have saved the search\n* a third, with **marks**\n- a fourth that is one too many\n\n## Other\n- not this\n";
        let notes = parse(report);
        assert_eq!(notes.len(), 3);
        assert!(notes[0].machine);
        assert_eq!(notes[0].text, "cargo test relinks rc.exe while the window runs: test with -p");
        assert!(!notes[1].machine);
        assert_eq!(notes[2].text, "a third, with **marks**");
        // the heading in other dress, and no heading at all
        assert_eq!(parse("**Faster next time:**\n- one\n").len(), 1);
        assert_eq!(parse("faster next time\n\n- after a blank\n")[0].text, "after a blank");
        assert!(parse("Done.\n- a bullet of the report\n").is_empty());
        assert!(parse("## Faster next time\n\nnothing cost anything\n").is_empty());
    }

    #[test]
    fn a_classify_answer_reads_as_machine_task_or_neither() {
        assert_eq!(classified("machine"), Some(true));
        assert_eq!(classified(" **Machine**.\n"), Some(true));
        assert_eq!(classified("task: the card named no files"), Some(false));
        assert_eq!(classified("I cannot tell"), None);
        assert_eq!(classified(""), None);
    }

    #[test]
    fn the_log_keeps_the_bullets_and_the_next_run_reads_the_latest_once() {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("hub").join("notes.log");
        assert!(recent(dir.path(), 5).is_empty());
        assert_eq!(machine_lines_with(dir.path(), &shared), "");
        let first = parse("## Faster next time\n- [machine] one\n- two\n");
        record_with(dir.path(), &shared, Some(7), &first).unwrap();
        record_with(dir.path(), &shared, Some(8), &parse("## Faster next time\n- two\n- three\n")).unwrap();
        let notes = recent(dir.path(), 10);
        assert_eq!(notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["three", "two", "two", "one"]);
        assert_eq!(notes[3].node, Some(7));
        assert!(notes[3].machine && !notes[0].machine);
        let lines = machine_lines_with(dir.path(), &shared);
        assert!(lines.starts_with("What earlier runs here, and the person, said"), "{lines}");
        assert_eq!(lines.matches("\n- ").count(), 3, "each once: {lines}");
        // nothing to record writes nothing
        record_with(dir.path(), &shared, None, &[]).unwrap();
        assert_eq!(recent(dir.path(), 10).len(), 4);
        // the person's note is kept apart and read first, being newest
        record_pair(dir.path(), "  the tooltip of the left panel is slow to show  ").unwrap();
        assert!(record_pair(dir.path(), "   ").is_err());
        let latest = &recent(dir.path(), 1)[0];
        assert!(latest.pair && !latest.machine && latest.node.is_none());
        assert_eq!(latest.text, "the tooltip of the left panel is slow to show");
        let lines = machine_lines_with(dir.path(), &shared);
        assert!(lines.contains("- (the person) the tooltip"), "{lines}");
        // a project's own machine note is not read back to it as another project's
        assert!(!lines.contains("other projects"), "{lines}");
    }

    #[test]
    fn a_machine_note_is_shared_with_the_other_projects_and_read_there_once() {
        let hub = tempfile::tempdir().unwrap();
        let shared = hub.path().join("notes.log");
        let alpha = tempfile::tempdir().unwrap();
        let beta = tempfile::tempdir().unwrap();
        let notes = parse(
            "## Faster next time\n- [machine] Select-String after a native pipe needs approval; grep in Bash works\n- the card named no files\n",
        );
        record_with(alpha.path(), &shared, Some(3), &notes).unwrap();
        // only the machine bullet crosses; the project's own stays home
        let text = std::fs::read_to_string(&shared).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains(&format!("\t{}\t1\tSelect-String", alpha.path().file_name().unwrap().to_string_lossy())), "{text}");
        assert!(recent(alpha.path(), 10).len() == 2 && recent(beta.path(), 10).is_empty());
        // the other project reads it under its own block, named by where it was learned
        let lines = machine_lines_with(beta.path(), &shared);
        assert!(lines.starts_with("What runs in other projects on this machine learned about it:\n- ("), "{lines}");
        assert!(lines.contains(") Select-String after a native pipe") && !lines.contains("named no files"), "{lines}");
        assert_eq!(shared_recent(beta.path(), &shared, 5).len(), 1);
        assert!(shared_recent(alpha.path(), &shared, 5).is_empty());
        // the person's [machine] note from beta goes the same way, as the person's
        record_pair(beta.path(), "[machine] the window holds rc-gui.exe; build with -p").unwrap();
        record_with(beta.path(), &shared, None, &recent(beta.path(), 1)).unwrap();
        let (project, note) = &shared_recent(alpha.path(), &shared, 5)[0];
        assert_eq!(project, &beta.path().file_name().unwrap().to_string_lossy());
        assert!(note.pair && note.machine);
        // once beta holds a lesson itself, the shared copy is not repeated under it
        let lines = machine_lines_with(beta.path(), &shared);
        assert_eq!(lines.matches("rc-gui.exe").count(), 1, "{lines}");
        assert!(lines.contains("- (the person) the window holds") && lines.contains(") Select-String"), "{lines}");
    }
}
