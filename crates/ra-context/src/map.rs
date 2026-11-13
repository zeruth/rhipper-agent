//! The map of a card's work: the items of the card's files ranked against what the card says,
//! so the prompt points at the few a worker needs rather than the first sixty in file order.
//! An item the card names comes first, then what the last run changed or a point names, the
//! test the check runs, an item whose name holds a rare word of the card, and a caller of a
//! named item; the best that fit the room, in file order, and one line a file for the rest.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::Path;

use crate::{ChangedLines, clip, item_name, items};

/// An item the card names, in backticks or by a `file:line` that falls in it.
const NAMED: usize = 100;
/// An item the last run changed, or one a point names.
const CHANGED: usize = 80;
/// A test whose name holds the filter of the card's `cargo test`.
const FILTER: usize = 70;
/// The most a word of the card in an item's name scores, for a word only that item holds.
const RARE: usize = 40;
/// An item that calls one the card names.
const CALLER: usize = 30;

/// Words too common in a card's prose to point at anything.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "its", "are", "not", "but", "has", "have", "was", "were", "all", "any",
    "one", "two", "out", "than", "then", "them", "they", "there", "what", "when", "which", "who", "will", "can", "does", "under", "over",
    "only", "also", "each", "every", "per", "pass", "passes", "where", "their",
];

/// What a card asks, as the map reads it: its text (title, objective and acceptance) and the
/// points a review or a failed check left on it.
#[derive(Debug, Clone, Default)]
pub struct Ask<'a> {
    pub card: &'a str,
    pub points: &'a [String],
}

/// One item the map points at: its file, its inclusive 1-based range, the item as the outline
/// shows it, why it is pointed at and how much that weighs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pointer {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub item: String,
    pub why: String,
    pub score: usize,
}

impl Pointer {
    /// The line the map prints for the item, a Read hint and its reason.
    #[must_use]
    pub fn line(&self) -> String {
        format!("{} {}-{}  {}  · {}", self.file, self.start, self.end, clip(&self.item, 110), self.why)
    }
}

/// The map of a card's files: the items pointed at, in file order, and one line a file for
/// what is not shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Map {
    pub pointers: Vec<Pointer>,
    pub rest: Vec<String>,
}

impl Map {
    /// The map as the prompt carries it.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in self.pointers.iter().map(Pointer::line).chain(self.rest.iter().cloned()) {
            let _ = writeln!(out, "{line}");
        }
        out
    }
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
    let read: Vec<Option<Read>> = files
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
    let pointed: HashSet<String> = ask.points.iter().flat_map(|p| backticked(p)).collect();
    let point_refs: Vec<(String, usize)> = ask.points.iter().flat_map(|p| refs_of(p)).collect();
    let filters = filters_of(ask.card);
    let words = words_of(ask.card);

    // how many items' names hold each word, across every file, so a word most items hold weighs little
    let mut spread: HashMap<String, usize> = HashMap::new();
    let mut named_names: HashSet<String> = HashSet::new();
    for (file, (_, items)) in files.iter().zip(&read).filter_map(|(f, r)| r.as_ref().map(|r| (f, r))) {
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
    for (index, (file, entry)) in files.iter().zip(&read).enumerate() {
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
                reasons.push((CHANGED, "named by a point".to_owned()));
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
                candidates.push((index, Pointer { file: file.clone(), start, end, item: item.clone(), why, score }));
            }
        }
    }

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
    for (index, (file, entry)) in files.iter().zip(&read).enumerate() {
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
    Map { pointers: kept.into_iter().map(|(_, pointer)| pointer).collect(), rest }
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
            let (path, line) = token.trim_end_matches(['.', ':']).rsplit_once(':')?;
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
