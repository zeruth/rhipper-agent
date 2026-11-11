//! The worker's own look back, in debug mode: a run ends its report with a short `Faster next
//! time` section, at most three one-line bullets, each what cost turns or time and the one
//! change that would have avoided it, `[machine]` when the cause is this machine. It costs no
//! second call: the ask rides in the system prompt, the bullets ride in the report. The
//! orchestrator keeps them in `var/retro.log`, names them in the run's result, and hands the
//! latest to the next runs as machine notes, so one worker's hindsight is the next one's
//! foresight.

use std::path::Path;

/// Where the bullets are kept, under the workspace: `time<TAB>node<TAB>machine<TAB>text`.
pub const RETRO_FILE: &str = "var/retro.log";

/// The heading the worker writes, matched without its marks and its case.
pub const HEADING: &str = "faster next time";

/// How many of the latest bullets the next run reads.
pub const SHOWN: usize = 5;

/// What the system prompt asks for in debug mode.
pub const PROMPT: &str = "End your report with a section headed `## Faster next time`: at most three bullets of one line each, naming what cost this run turns or time (a refused command, a file you had to search for, a check you ran twice, a quirk of this machine) and the one change that would have avoided it; start a bullet with [machine] when the cause is this machine rather than the task. Nothing else in that section; leave it out when nothing cost anything.";

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
/// the workers' own.
///
/// # Errors
/// When the text is empty or the file cannot be written.
pub fn record_pair(root: &Path, text: &str) -> anyhow::Result<()> {
    let text = text.trim();
    anyhow::ensure!(!text.is_empty(), "a note says something");
    let note = Note { when: 0, node: None, machine: false, pair: true, text: text.chars().take(400).collect() };
    crate::pair::event(root, &format!("note {}", note.text));
    record(root, None, &[note])
}

/// The bullets of a report's `Faster next time` section, in order; none without the section.
/// A bullet keeps one line, loses its `[machine]` tag into the flag, and clips at 240 characters.
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

/// Keeps the bullets of a run of `node`, newest last.
///
/// # Errors
/// When the file cannot be written.
pub fn record(root: &Path, node: Option<i64>, notes: &[Note]) -> anyhow::Result<()> {
    use std::io::Write;
    if notes.is_empty() {
        return Ok(());
    }
    let path = root.join(RETRO_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    for note in notes {
        let node = node.map_or(String::from("-"), |n| n.to_string());
        // the third column: 0 a worker's, 1 a worker's about this machine, 2 the person's
        let kind = if note.pair { 2 } else { u8::from(note.machine) };
        writeln!(file, "{when}\t{node}\t{kind}\t{}", note.text.replace(['\t', '\n'], " "))?;
    }
    Ok(())
}

/// The latest `limit` bullets, newest first; none without a file.
pub fn recent(root: &Path, limit: usize) -> Vec<Note> {
    let Ok(text) = std::fs::read_to_string(root.join(RETRO_FILE)) else { return Vec::new() };
    let mut notes: Vec<Note> = text
        .lines()
        .filter_map(|line| {
            let mut cols = line.split('\t');
            let (when, node, machine, text) = (cols.next()?, cols.next()?, cols.next()?, cols.next()?);
            Some(Note {
                when: when.parse().ok()?,
                node: node.parse().ok(),
                machine: machine == "1",
                pair: machine == "2",
                text: text.to_owned(),
            })
        })
        .collect();
    notes.reverse();
    notes.truncate(limit);
    notes
}

/// The latest bullets as a block for the system prompt of the next run, each once; empty
/// without any.
pub fn machine_lines(root: &Path) -> String {
    let mut seen: Vec<String> = Vec::new();
    for note in recent(root, SHOWN * 2) {
        // the person's notes are told from the workers' own
        let line = if note.pair { format!("(the person) {}", note.text) } else { note.text };
        if seen.len() == SHOWN || seen.contains(&line) {
            continue;
        }
        seen.push(line);
    }
    if seen.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = seen.iter().map(|t| format!("- {t}")).collect();
    format!("What earlier runs here, and the person, said would have made them faster:\n{}", lines.join("\n"))
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
    fn the_log_keeps_the_bullets_and_the_next_run_reads_the_latest_once() {
        let dir = tempfile::tempdir().unwrap();
        assert!(recent(dir.path(), 5).is_empty());
        assert_eq!(machine_lines(dir.path()), "");
        let first = parse("## Faster next time\n- [machine] one\n- two\n");
        record(dir.path(), Some(7), &first).unwrap();
        record(dir.path(), Some(8), &parse("## Faster next time\n- two\n- three\n")).unwrap();
        let notes = recent(dir.path(), 10);
        assert_eq!(notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["three", "two", "two", "one"]);
        assert_eq!(notes[3].node, Some(7));
        assert!(notes[3].machine && !notes[0].machine);
        let lines = machine_lines(dir.path());
        assert!(lines.starts_with("What earlier runs here, and the person, said"), "{lines}");
        assert_eq!(lines.matches("\n- ").count(), 3, "each once: {lines}");
        // nothing to record writes nothing
        record(dir.path(), None, &[]).unwrap();
        assert_eq!(recent(dir.path(), 10).len(), 4);
        // the person's note is kept apart and read first, being newest
        record_pair(dir.path(), "  the tooltip of the left panel is slow to show  ").unwrap();
        assert!(record_pair(dir.path(), "   ").is_err());
        let latest = &recent(dir.path(), 1)[0];
        assert!(latest.pair && !latest.machine && latest.node.is_none());
        assert_eq!(latest.text, "the tooltip of the left panel is slow to show");
        assert!(machine_lines(dir.path()).contains("- (the person) the tooltip"), "{}", machine_lines(dir.path()));
    }
}
