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
    /// The fingerprint of the acceptance of the card whose run wrote the note, as
    /// [`acceptance_print`] has it: a note about a check the card has since changed retires.
    pub against: Option<String>,
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
    let note = Note { when: 0, node: None, machine, pair: true, text: text.chars().take(400).collect(), against: None };
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
        notes.push(Note { when: 0, node: None, machine, pair: false, text, against: None });
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
    let own = notes.iter().map(|note| {
        let against = note.against.as_deref().map(|print| format!("\t{print}")).unwrap_or_default();
        format!("{when}\t{node}\t{}\t{}{against}", kind_of(note), clean(&note.text))
    });
    append(&root.join(RETRO_FILE), own)?;
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
    record(root, Some(node), &[Note { when: 0, node: Some(node), machine: false, pair: false, text: text.clone(), against: None }])?;
    Ok(text)
}

/// The tag of the line a run writes when its turns were heavier than the ledger's ceiling.
pub const HEAVY: &str = "[heavy]";

/// The ledger's ceiling on the tokens a turn: a run whose mean turn passes it carried more
/// context than the work needed, and is noted. About the 90th percentile of the Claude Code runs
/// of 2026-11-12 (83k a turn): at 40k, under their median of 55k, nearly every run was noted.
pub const TURN_CEILING: u64 = 90_000;

/// Keeps `[heavy] #id N tokens/turn over the ceiling of C; biggest tool result: … (M chars)`
/// under the card when a run's mean turn passes [`TURN_CEILING`], naming the tool call whose
/// answer was the largest, so the next walk reads which read or command to make smaller; the
/// line as written. `biggest` is `None` when the harness ran the tools out of sight.
///
/// # Errors
/// When the file cannot be written.
pub fn record_heavy(root: &Path, node: i64, per_turn: u64, biggest: Option<(&str, usize)>) -> anyhow::Result<String> {
    let result = match biggest {
        Some((call, chars)) => format!("biggest tool result: {call} ({chars} chars)"),
        None => "the tool results were the harness's, out of sight".to_owned(),
    };
    let text = format!("{HEAVY} #{node} {per_turn} tokens/turn over the ceiling of {TURN_CEILING}; {result}");
    record(root, Some(node), &[Note { when: 0, node: Some(node), machine: false, pair: false, text: text.clone(), against: None }])?;
    Ok(text)
}

/// How many landed cards a card is measured against: the median of the last twenty.
pub const MEDIAN_OF: u32 = 20;

/// The walk's verdict on a card's claim the moment it lands, measured: the card's own turns,
/// tokens a turn and cost from the ledger against the median tokens a turn of the last
/// [`MEDIAN_OF`] cards the ledger wrote up to `mark` (the newest row before the run), the card
/// itself left out; held when the card's are fewer. One `[saving]` line under the card in
/// `var/retro.log`, the card's claim quoted beside the figures, returned too: its `saving:`
/// field, else the `[saving]`/`Saving:` sentence of its objective, else `claimed by estimate`
/// with the complexity's tokens a turn, so every landed card is judged; nothing when the card
/// did not land, was judged already, or the ledger has no rows on one side. A claim that holds
/// across projects, its evidence cited from another project's `notes.log`, is a `[machine]`
/// note too: it reaches the hub, and every project's planner reads it through [`evidence`] as
/// a lesson that paid off twice; a worker's prompt ([`machine_lines`]) leaves every verdict out.
pub async fn landed(root: &Path, card: &ra_roadmap::model::Node, ledger: &ra_usage::UsageStore, mark: i64) -> Vec<String> {
    use ra_roadmap::model::TaskState;
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
    let claimed = match card.saving.clone().or_else(|| card.objective.as_deref().and_then(claim_in)) {
        Some(saving) => format!("claimed \"{saving}\""),
        None => format!("claimed by estimate of {} tokens/turn", crate::agent::estimate_of(card).a_turn()),
    };
    let text = format!(
        "{SAVING} #{} {}: {} turns, {per_turn} tokens/turn{cost} vs median {median}; {claimed}",
        card.id,
        if held { "held" } else { "missed" },
        own.totals.turns
    );
    let note = Note { when: 0, node: Some(card.id), machine: held && across, pair: false, text: text.clone(), against: None };
    match record(root, Some(card.id), &[note]) {
        Ok(()) => vec![text],
        Err(e) => {
            tracing::warn!("the saving of #{} was not recorded: {e:#}", card.id);
            Vec::new()
        }
    }
}

/// The saving an objective claims in its own text, as the Phase P/Q cards wrote it: the sentence
/// after `[saving]` or `Saving:`, to its full stop or line end; `None` when it claims none.
fn claim_in(objective: &str) -> Option<String> {
    let (at, tag) = [SAVING, "Saving:"].iter().filter_map(|tag| objective.find(tag).map(|at| (at, tag.len()))).min()?;
    let rest = objective[at + tag..].trim_start_matches([':', ' ']);
    let end = [rest.find(". "), rest.find('\n')].into_iter().flatten().min().unwrap_or(rest.len());
    let claim = rest[..end].trim().trim_end_matches('.').trim();
    (!claim.is_empty()).then(|| claim.to_owned())
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
/// (the node of a project's log, the project of the shared one). A [`RETIRED`] line keeps its
/// number but is no note.
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
                against: cols.next().filter(|print| !print.is_empty()).map(str::to_owned),
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
    machine_lines_against(root, &shared_file(), None)
}

/// `machine_lines` for a run of `card`, its id and its acceptance: a note its run wrote about a
/// check the acceptance has since changed does not ride.
#[must_use]
pub fn machine_lines_for(root: &Path, card: Option<(i64, Option<&str>)>) -> String {
    machine_lines_against(root, &shared_file(), card)
}

/// `machine_lines` with the shared store named, for the tests.
#[must_use]
pub fn machine_lines_with(root: &Path, shared: &Path) -> String {
    machine_lines_against(root, shared, None)
}

/// How long a note rides: a lesson older than a week was about a machine or a project that has
/// moved on since.
pub const RIDES_FOR: u64 = 7 * 24 * 60 * 60;

/// The fingerprint of a card's acceptance a run's notes carry, so a later run tells whether the
/// checks they were written against still stand.
#[must_use]
pub fn acceptance_print(acceptance: Option<&str>) -> String {
    let text = acceptance.unwrap_or("").replace("\r\n", "\n");
    let hash = text.trim().bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{hash:016x}")
}

/// Whether a note names a command, a check or a config key: the kind of lesson a change of the
/// project's `rhipper.toml` or of the card's acceptance makes stale (#9 spent a run on one about
/// `cargo test --workspace` written before the project's check was configured).
#[must_use]
pub fn names_a_command(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    text.contains('`') || ["cargo ", "check", ".toml", " = ", " --", "[worker]", "[role]", "[plan]"].iter().any(|word| lower.contains(word))
}

/// Whether a note written at `when` is past [`RIDES_FOR`] at `now`.
fn stale(when: u64, now: u64) -> bool {
    now.saturating_sub(when) > RIDES_FOR
}

/// Whether a line of the project's log still rides into a run: young enough, and, when it names
/// a command, written after the last change of `rhipper.toml` (`config`, its mtime) and against
/// the acceptance the card has now (`card`, its id and [`acceptance_print`]).
fn rides(note: &Note, now: u64, config: Option<u64>, card: Option<(i64, &str)>) -> bool {
    if stale(note.when, now) {
        return false;
    }
    if !names_a_command(&note.text) {
        return true;
    }
    let config_moved = config.is_some_and(|at| at > note.when);
    let card_moved = card.is_some_and(|(id, print)| note.node == Some(id) && note.against.as_deref().is_some_and(|was| was != print));
    !(config_moved || card_moved)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// When `path` was last written, in unix seconds; `None` without the file.
fn modified(path: &Path) -> Option<u64> {
    let at = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    at.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// The latest `limit` lines of the project's log with their line numbers, newest first: what
/// `rc note --retire <n>` names.
#[must_use]
pub fn numbered(root: &Path, limit: usize) -> Vec<(usize, Note)> {
    let mut lines: Vec<(usize, Note)> = parse_log(&root.join(RETRO_FILE)).into_iter().map(|(line, _, note)| (line, note)).collect();
    lines.reverse();
    lines.truncate(limit);
    lines
}

/// The first column of a line retired by hand: the line stays in its log, so a `[machine]`
/// lesson is not lost, but no longer parses as a note and so rides into no run.
pub const RETIRED: &str = "retired";

/// Retires line `n` of the project's log by hand, and its copy in the shared store when it was
/// a machine note; the line's text, its tag back on.
///
/// # Errors
/// When the log has no such line or cannot be written.
pub fn retire(root: &Path, n: usize) -> anyhow::Result<String> {
    retire_with(root, &shared_file(), n)
}

/// `retire` with the shared store named, for the tests.
///
/// # Errors
/// When the log has no such line or cannot be written.
pub fn retire_with(root: &Path, shared: &Path, n: usize) -> anyhow::Result<String> {
    let path = root.join(RETRO_FILE);
    let Some((_, _, note)) = parse_log(&path).into_iter().find(|(line, _, _)| *line == n) else {
        anyhow::bail!("{RETRO_FILE} has no note on line {n}");
    };
    mark_lines(&path, |index, _| index + 1 == n)?;
    if note.machine {
        let copy = format!("{}\t{}\t{}\t{}", note.when, project_name(root), kind_of(&note), note.text);
        if let Err(e) = mark_lines(shared, |_, line| line == copy) {
            tracing::warn!("the shared copy of the note was not retired: {e:#}");
        }
    }
    Ok(tagged(&note))
}

/// Rewrites `path` with the lines `gone` picks, by their index from 0 and their text, marked
/// [`RETIRED`].
fn mark_lines(path: &Path, gone: impl Fn(usize, &str) -> bool) -> anyhow::Result<()> {
    let Ok(text) = std::fs::read_to_string(path) else { return Ok(()) };
    let kept: String = text
        .lines()
        .enumerate()
        .map(|(index, line)| if gone(index, line) { format!("{RETIRED}\t{line}\n") } else { format!("{line}\n") })
        .collect();
    std::fs::write(path, kept)?;
    Ok(())
}

/// `machine_lines` with the shared store and the card named: every note that still [`rides`],
/// and the other projects' that are not [`stale`].
#[must_use]
pub fn machine_lines_against(root: &Path, shared: &Path, card: Option<(i64, Option<&str>)>) -> String {
    let now = unix_now();
    let config = modified(&root.join(crate::router::ROUTES_FILE));
    let print = card.map(|(id, acceptance)| (id, acceptance_print(acceptance)));
    let card = print.as_ref().map(|(id, print)| (*id, print.as_str()));
    let mut own: Vec<Note> =
        parse_log(&root.join(RETRO_FILE)).into_iter().map(|(_, _, note)| note).filter(|note| rides(note, now, config, card)).collect();
    own.reverse();
    let mut seen: Vec<String> = Vec::new();
    for note in own.into_iter().take(SHOWN * 2) {
        // the walk's verdicts on the plan's claims and on a run's weight are the planner's to
        // read, not a worker's
        if note.text.starts_with(SAVING) || note.text.starts_with(HEAVY) {
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
    let this = project_name(root);
    let mut theirs: Vec<(String, Note)> = parse_log(shared)
        .into_iter()
        .filter(|(_, project, note)| *project != this && !stale(note.when, now))
        .map(|(_, project, note)| (project, note))
        .collect();
    theirs.reverse();
    for (project, note) in theirs.into_iter().take(SHOWN * 2) {
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
