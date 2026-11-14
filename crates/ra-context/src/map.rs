//! The map of a card's work: the items of the card's files ranked against what the card says,
//! so the prompt points at the few a worker needs rather than the first sixty in file order.
//! An item the card names comes first, then what a point names and what the last run changed,
//! the test the check runs, an item whose name holds a rare word of the card, a caller of a
//! named item, and what the last session read; the best that fit the room, in file order, and
//! one line a file for the rest.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::Path;

use crate::{ChangedLines, clip, item_name, items, read_entry};

/// An item the card names, in backticks or by a `file:line` that falls in it.
const NAMED: usize = 100;
/// An item a point names: a review's, a failure's or a warning's.
const POINTED: usize = 90;
/// An item the last run changed.
const CHANGED: usize = 80;
/// A test whose name holds the filter of the card's `cargo test`.
const FILTER: usize = 70;
/// The most a word of the card in an item's name scores, for a word only that item holds.
const RARE: usize = 40;
/// An item that calls one the card names.
const CALLER: usize = 30;
/// An item the last session read: a row, never quoted, so a rerun opens on what its points
/// name and not on everything the last session looked at.
const READ: usize = 20;
/// Why an item only read is pointed at.
const READ_WHY: &str = "read by the last session";

/// Words too common in a card's prose to point at anything.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "its", "are", "not", "but", "has", "have", "was", "were", "all", "any",
    "one", "two", "out", "than", "then", "them", "they", "there", "what", "when", "which", "who", "will", "can", "does", "under", "over",
    "only", "also", "each", "every", "per", "pass", "passes", "where", "their",
];

/// How many items of one name outside the card's files the map points at.
const OUTSIDE_EACH: usize = 3;

/// The extensions of a tracked file the map looks in for a name the card's files lack.
const SOURCES: [&str; 8] = ["rs", "py", "ps1", "js", "ts", "go", "kt", "java"];

/// What a card asks, as the map reads it: its text (title, objective and acceptance), the
/// points a review, a failed check or a lint left on it, the read set of its last session as
/// [`crate::read_set`] has it, and the project's sources, where a name the card's files do not
/// define is looked for.
#[derive(Debug, Clone, Default)]
pub struct Ask<'a> {
    pub card: &'a str,
    pub points: &'a [String],
    pub read: &'a [String],
    pub sources: &'a [String],
}

/// One item the map points at: its file, its inclusive 1-based range, the item as the outline
/// shows it, why it is pointed at and how much that weighs, and whether its text may be quoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pointer {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub item: String,
    pub why: String,
    pub score: usize,
    pub quote: bool,
}

impl Pointer {
    /// The line the map prints for the item, a Read hint and its reason.
    #[must_use]
    pub fn line(&self) -> String {
        format!("{} {}-{}  {}  · {}", self.file, self.start, self.end, clip(&self.item, 110), self.why)
    }
}

/// The map of a card's files: the items pointed at, in file order with those found outside the
/// card's files after them, one line a file for what is not shown, and the names the card puts
/// in backticks that nothing in the project defines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Map {
    pub pointers: Vec<Pointer>,
    pub rest: Vec<String>,
    pub unresolved: Vec<String>,
}

impl Map {
    /// The map as the prompt carries it, with the numbered text of the pointed-at items under
    /// its rows, read through `read`: the highest score first, inside `quote` tokens, and an
    /// item that does not fit, or was only read by the last session, keeps its row alone.
    #[must_use]
    pub fn render(&self, read: impl Fn(&str) -> Option<String>, quote: usize) -> String {
        let mut out = String::new();
        for line in self.pointers.iter().map(Pointer::line).chain(self.rest.iter().cloned()) {
            let _ = writeln!(out, "{line}");
        }
        for name in &self.unresolved {
            let _ = writeln!(out, "nothing defines: {name}; treat it as new");
        }
        let mut order: Vec<&Pointer> = self.pointers.iter().filter(|p| p.quote).collect();
        order.sort_by_key(|p| std::cmp::Reverse(p.score));
        let mut left = quote;
        let mut bodies: HashMap<(&str, usize), String> = HashMap::new();
        for p in order {
            let Some(text) = read(&p.file) else { continue };
            let mut body = format!("{} {}-{}:\n", p.file, p.start, p.end);
            for (i, line) in text.lines().enumerate().take(p.end).skip(p.start.saturating_sub(1)) {
                let _ = writeln!(body, "{:>4}  {line}", i + 1);
            }
            let cost = tokens(&body);
            if cost <= left {
                left -= cost;
                bodies.insert((p.file.as_str(), p.start), body);
            }
        }
        for p in &self.pointers {
            if let Some(body) = bodies.get(&(p.file.as_str(), p.start)) {
                out.push_str(body);
            }
        }
        out
    }
}

/// The source files git tracks in `workspace`, as `git ls-files` lists them: where the map
/// looks for a name the card's files do not define. None outside a git repository.
#[must_use]
pub fn tracked_sources(workspace: &Path) -> Vec<String> {
    let listed = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(workspace)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    listed
        .lines()
        .map(|line| line.trim().trim_matches('"'))
        .filter(|path| Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| SOURCES.contains(&e)))
        .map(str::to_owned)
        .collect()
}

/// A file as the map reads it: its text and its items with the lines each owns.
type Read = (String, Vec<(usize, usize, String)>);

/// The tokens a line of the prompt costs, by the usual four characters a token.
#[must_use]
pub fn tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// The map of `files`, each read through `read` (`None` for a file not there yet): every item
/// scored against `ask` and the lines the last run `changed`, the best whose lines fit `room`
/// tokens kept, a tie going to the earlier item.
#[must_use]
pub fn map_of(read: impl Fn(&str) -> Option<String>, files: &[String], ask: &Ask, changed: &ChangedLines, room: usize) -> Map {
    let entries: Vec<Option<Read>> = files
        .iter()
        .map(|file| {
            read(file).map(|text| {
                let items = items(Path::new(file), &text);
                (text, items)
            })
        })
        .collect();
    let named = backticked(ask.card);
    let card_refs = refs_of(ask.card);
    let pointed: HashSet<String> = ask.points.iter().flat_map(|p| backticked(p).into_iter().chain(failed_name(p))).collect();
    let point_refs: Vec<(String, usize)> = ask.points.iter().flat_map(|p| refs_of(p)).collect();
    let filters = filters_of(ask.card);
    let words = words_of(ask.card);

    // how many items' names hold each word, across every file, so a word most items hold weighs little
    let mut spread: HashMap<String, usize> = HashMap::new();
    let mut named_names: HashSet<String> = HashSet::new();
    for (file, (_, items)) in files.iter().zip(&entries).filter_map(|(f, r)| r.as_ref().map(|r| (f, r))) {
        for (start, end, item) in items {
            for part in name_parts(name_of(item)).into_iter().collect::<HashSet<_>>() {
                *spread.entry(part).or_default() += 1;
            }
            if let Some(name) = item_name(item)
                && (named.contains(name) || refers(&card_refs, file, *start, *end))
            {
                named_names.insert(name.to_owned());
            }
        }
    }

    let mut candidates: Vec<(usize, Pointer)> = Vec::new();
    for (index, (file, entry)) in files.iter().zip(&entries).enumerate() {
        let Some((text, items)) = entry else { continue };
        let lines: Vec<&str> = text.lines().collect();
        let marks = changed.get(file).map_or(&[][..], Vec::as_slice);
        for (start, end, item) in items {
            let (start, end) = (*start, *end);
            let name = item_name(item);
            let mut reasons: Vec<(usize, String)> = Vec::new();
            if name.is_some_and(|n| named.contains(n)) || refers(&card_refs, file, start, end) {
                reasons.push((NAMED, "named by the card".to_owned()));
            }
            if marks.iter().any(|&(from, to)| from <= end && to >= start) {
                reasons.push((CHANGED, "changed by the last run".to_owned()));
            }
            if name.is_some_and(|n| pointed.contains(n)) || refers(&point_refs, file, start, end) {
                reasons.push((POINTED, "named by a point".to_owned()));
            }
            if was_read(ask.read, file, start, end) {
                reasons.push((READ, READ_WHY.to_owned()));
            }
            if let Some(n) = name.filter(|_| item.contains("fn "))
                && let Some(filter) = filters.iter().find(|f| n.contains(f.as_str()))
            {
                reasons.push((FILTER, format!("holds the check's filter `{filter}`")));
            }
            if let Some((score, word)) = name_parts(name_of(item))
                .into_iter()
                .filter(|part| words.contains(part))
                .map(|part| (RARE / spread.get(&part).copied().unwrap_or(1).max(1), part))
                .max_by_key(|(score, _)| *score)
                .filter(|(score, _)| *score > 0)
            {
                reasons.push((score, format!("its name holds `{word}`")));
            }
            let body = lines.get(start - 1..end.min(lines.len())).unwrap_or(&[]);
            if let Some(callee) = body.iter().copied().flat_map(identifiers).find(|word| named_names.contains(*word) && Some(*word) != name)
            {
                reasons.push((CALLER, format!("calls `{callee}`")));
            }
            // the strongest reason, the first given on a tie
            if let Some((score, why)) = reasons.into_iter().reduce(|best, next| if next.0 > best.0 { next } else { best }) {
                let quote = why != READ_WHY;
                candidates.push((index, Pointer { file: file.clone(), start, end, item: item.clone(), why, score, quote }));
            }
        }
    }

    let unresolved = look_outside(&read, files, &entries, ask, &mut candidates);

    candidates.sort_by(|(a, p), (b, q)| q.score.cmp(&p.score).then(a.cmp(b)).then(p.start.cmp(&q.start)));
    let mut used = 0;
    let mut kept: Vec<(usize, Pointer)> = Vec::new();
    for (index, pointer) in candidates {
        let cost = tokens(&pointer.line());
        if used + cost <= room {
            used += cost;
            kept.push((index, pointer));
        }
    }
    kept.sort_by_key(|(index, pointer)| (*index, pointer.start));

    let mut rest = Vec::new();
    for (index, (file, entry)) in files.iter().zip(&entries).enumerate() {
        let Some((text, items)) = entry else {
            rest.push(format!("{file} (not in the workspace yet)"));
            continue;
        };
        let lines = text.lines().count();
        let shown = kept.iter().filter(|(i, _)| *i == index).count();
        if shown == 0 {
            rest.push(format!("{file} ({lines} lines): {} items, none the card points at", items.len()));
        } else if items.len() > shown {
            rest.push(format!("{file} ({lines} lines): {} more items not shown", items.len() - shown));
        }
    }
    Map { pointers: kept.into_iter().map(|(_, pointer)| pointer).collect(), rest, unresolved }
}

/// The names of the card no item of its files declares, each looked for in the project's
/// sources: an item found there joins `candidates` as named by the card, and a name found
/// nowhere is returned, in the card's order.
fn look_outside(
    read: &impl Fn(&str) -> Option<String>,
    files: &[String],
    entries: &[Option<Read>],
    ask: &Ask,
    candidates: &mut Vec<(usize, Pointer)>,
) -> Vec<String> {
    let defined: HashSet<&str> =
        entries.iter().flatten().flat_map(|(_, items)| items.iter().filter_map(|(_, _, item)| item_name(item))).collect();
    let missing: Vec<(String, Vec<String>)> =
        code_names(ask.card).into_iter().filter(|(_, segments)| !segments.iter().any(|s| defined.contains(s.as_str()))).collect();
    if missing.is_empty() {
        return Vec::new();
    }
    let wanted: HashSet<&str> = missing.iter().flat_map(|(_, segments)| segments.iter().map(String::as_str)).collect();
    let own: HashSet<String> = files.iter().map(|f| f.replace('\\', "/")).collect();
    let mut found: HashMap<String, usize> = HashMap::new();
    for (offset, source) in ask.sources.iter().enumerate() {
        if own.contains(&source.replace('\\', "/")) {
            continue;
        }
        let Some(text) = read(source) else { continue };
        for (start, end, item) in items(Path::new(source), &text) {
            let Some(name) = item_name(&item).filter(|n| wanted.contains(n)) else { continue };
            let seen = found.entry(name.to_owned()).or_default();
            if *seen < OUTSIDE_EACH {
                *seen += 1;
                let why = "named by the card, outside its files".to_owned();
                let pointer = Pointer { file: source.clone(), start, end, item, why, score: NAMED, quote: true };
                candidates.push((files.len() + offset, pointer));
            }
        }
    }
    missing.into_iter().filter(|(_, segments)| !segments.iter().any(|s| found.contains_key(s))).map(|(name, _)| name).collect()
}

/// The names a card puts in backticks as code, each with the segments of its path, in the
/// card's order: `map_of`, `Map::unresolved` or `render()`, not a command, a path or a file.
fn code_names(card: &str) -> Vec<(String, Vec<String>)> {
    let mut names: Vec<(String, Vec<String>)> = Vec::new();
    for span in card.split('`').skip(1).step_by(2) {
        let name = span.trim().trim_start_matches(['&', '*']).trim_end_matches("()").trim_end_matches('!');
        let segments: Vec<String> = name.split("::").map(str::to_owned).collect();
        let code = segments
            .iter()
            .all(|s| s.len() > 2 && !s.starts_with(|c: char| c.is_ascii_digit()) && s.chars().all(|c| c.is_alphanumeric() || c == '_'));
        if code && !names.iter().any(|(known, _)| known == name) {
            names.push((name.to_owned(), segments));
        }
    }
    names
}

/// The name an item declares, or the item itself for one that declares none, a heading say.
fn name_of(item: &str) -> &str {
    item_name(item).unwrap_or(item)
}

/// The identifiers of a text: its runs of letters, digits and `_`.
fn identifiers(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty())
}

/// The identifiers a text puts in backticks, where a card names code.
fn backticked(text: &str) -> HashSet<String> {
    text.split('`').skip(1).step_by(2).flat_map(identifiers).filter(|w| w.len() > 1).map(str::to_owned).collect()
}

/// The `file:line` references of a text, `lib.rs:349` say.
fn refs_of(text: &str) -> Vec<(String, usize)> {
    text.split(|c: char| c.is_whitespace() || "`(),;".contains(c))
        .filter_map(|token| {
            let (mut path, mut line) = token.trim_end_matches(['.', ':']).rsplit_once(':')?;
            // a `file:line:column`, as a panic, a compiler error or a lint gives it
            if let Some((file, row)) = path.rsplit_once(':')
                && row.parse::<usize>().is_ok()
            {
                (path, line) = (file, row);
            }
            let line = line.parse().ok()?;
            path.contains('.').then(|| (path.replace('\\', "/"), line))
        })
        .collect()
}

/// Whether a reference falls in the range of an item of `file`.
fn refers(refs: &[(String, usize)], file: &str, start: usize, end: usize) -> bool {
    let file = file.replace('\\', "/");
    refs.iter().any(|(path, line)| file.ends_with(path.as_str()) && (start..=end).contains(line))
}

/// The function a failure names, the last segment of the test path its first line is
/// (`third` of `tests::third`); `None` for a point whose first line is prose.
fn failed_name(point: &str) -> Option<String> {
    let first = point.lines().next()?.trim();
    let name = first.rsplit("::").next()?;
    let path = !first.contains(char::is_whitespace) && first.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ':');
    (path && name.len() > 1).then(|| name.to_owned())
}

/// Whether the last session read any line of the item at `start..=end` of `file`.
fn was_read(read: &[String], file: &str, start: usize, end: usize) -> bool {
    let file = file.replace('\\', "/");
    read.iter().any(|entry| {
        let (path, range) = read_entry(entry);
        path.replace('\\', "/") == file && range.is_none_or(|(from, to)| from <= end && to >= start)
    })
}

/// The filters of the `cargo test` commands a card puts in backticks: the first word after
/// `test` that is neither a flag nor a flag's value.
fn filters_of(card: &str) -> Vec<String> {
    const VALUED: [&str; 9] = ["-p", "--package", "--test", "--bin", "--features", "-F", "--manifest-path", "--target", "-j"];
    card.split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| span.contains("cargo test"))
        .filter_map(|span| {
            let mut words = span.split_whitespace().skip_while(|w| *w != "test").skip(1);
            while let Some(word) = words.next() {
                if word == "--" {
                    return None;
                }
                if VALUED.contains(&word) {
                    words.next();
                } else if !word.starts_with('-') {
                    return Some(word.to_owned());
                }
            }
            None
        })
        .collect()
}

/// A word reduced so `ranks` meets `rank`.
fn stem(word: &str) -> String {
    let word = word.to_lowercase();
    match word.strip_suffix('s') {
        Some(stem) if word.len() > 3 && !stem.ends_with('s') => stem.to_owned(),
        _ => word,
    }
}

/// The words of a card that can point at an item.
fn words_of(card: &str) -> HashSet<String> {
    card.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_lowercase)
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .map(|w| stem(&w))
        .collect()
}

/// The words of a name, split at `_`, punctuation and a lower-to-upper case change.
fn name_parts(name: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for run in name.split(|c: char| !c.is_alphanumeric()) {
        let mut part = String::new();
        let mut last_lower = false;
        for c in run.chars() {
            if c.is_uppercase() && last_lower && !part.is_empty() {
                parts.push(std::mem::take(&mut part));
            }
            last_lower = c.is_lowercase() || c.is_ascii_digit();
            part.push(c);
        }
        parts.push(part);
    }
    parts.into_iter().filter(|p| p.len() > 2).map(|p| stem(&p)).collect()
}

#[cfg(test)]
mod tests;
