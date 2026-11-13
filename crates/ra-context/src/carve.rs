//! A split of a Rust file by script: named items move to a new file with their doc comments
//! and attributes, under the `use` lines they need, and the old file keeps a `mod` line and a
//! re-export, so no caller changes. No model writes the moved text, so a split costs no output
//! tokens and its review has no unchanged text to read.

use std::path::Path;

use anyhow::{Context, bail};

use crate::{item_name, rust_item};

/// What a carve leaves: the old file's text, the new file's, and each moved item's name with
/// the lines it held in the old file, 1-based and inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carve {
    pub from: String,
    pub to: String,
    pub moved: Vec<(String, usize, usize)>,
}

/// A top-level item of the old file: its name, its visibility, whether it is an `impl`, and its
/// lines from its doc comment to its end, 0-based and inclusive.
struct Item {
    name: String,
    visibility: String,
    is_impl: bool,
    start: usize,
    end: usize,
}

/// A top-level `use` statement: its lines, 0-based and inclusive, its text and the names it
/// brings in.
struct Use {
    first: usize,
    last: usize,
    statement: String,
    names: Vec<String>,
}

/// Traits a file uses by their methods or macros alone, with the text that shows the use.
const TRAIT_USES: [(&str, &[&str]); 5] = [
    ("Write", &["write!(", "writeln!("]),
    ("Context", &[".context(", ".with_context("]),
    ("Read", &[".read(", ".read_to_string(", ".read_to_end(", ".read_exact("]),
    ("BufRead", &[".lines()", ".read_line("]),
    ("FromStr", &["::from_str("]),
];

/// Cuts `names` out of `from` into the new file `to`, both read through `read` by the paths as
/// given. `to` must not exist yet and must be where a `mod` line in `from` finds it: beside a
/// `lib.rs`, `main.rs` or `mod.rs`, under the directory named after any other file.
///
/// # Errors
/// When `from` cannot be read, `to` exists or is not a child module's place, or a name is not a
/// top-level item of `from`.
pub fn carve(read: impl Fn(&str) -> Option<String>, from: &str, to: &str, names: &[String]) -> anyhow::Result<Carve> {
    let module = module_of(from, to)?;
    if read(to).is_some() {
        bail!("{to} exists already");
    }
    let text = read(from).with_context(|| format!("cannot read {from}"))?;
    let lines: Vec<&str> = text.lines().collect();
    let items = top_items(&lines);
    let mut moved: Vec<&Item> = Vec::new();
    for name in names {
        let item = items.iter().find(|i| &i.name == name).with_context(|| format!("{name} is not a top-level item of {from}"))?;
        if !moved.iter().any(|m| m.start == item.start) {
            moved.push(item);
        }
    }
    moved.sort_by_key(|i| i.start);
    let cut = |n: usize| moved.iter().any(|i| (i.start..=i.end).contains(&n));

    // the new file: the old file's imports its items name, then the old file's own items they use
    let moved_code = code_of(moved.iter().flat_map(|i| lines[i.start..=i.end].iter().copied()));
    let mut head: Vec<String> = uses_of(&lines)
        .into_iter()
        .filter(|u| u.names.iter().any(|n| n == "*" || mentions(&moved_code, n)))
        .map(|u| rerooted(&u.statement))
        .collect();
    let siblings: Vec<&str> =
        items.iter().filter(|i| !i.is_impl && !cut(i.start) && mentions(&moved_code, &i.name)).map(|i| i.name.as_str()).collect();
    if !siblings.is_empty() {
        head.push(format!("use super::{};", braced(&siblings)));
    }
    let body = moved
        .iter()
        .map(|i| lines[i.start..=i.end].iter().map(|l| widened(l, i)).collect::<Vec<_>>().join("\n") + "\n")
        .collect::<Vec<_>>()
        .join("\n");
    let to_text = if head.is_empty() { body } else { format!("{}\n\n{body}", head.join("\n")) };

    // the old file: the moved lines gone, the imports only they used gone, a `mod` line and a re-export
    let kept: Vec<String> = lines.iter().enumerate().filter(|(n, _)| !cut(*n)).map(|(_, l)| (*l).to_owned()).collect();
    let mut out = without_unused_uses(&collapsed(kept));
    let mut exports: Vec<(String, Vec<&str>)> = Vec::new();
    for item in moved.iter().filter(|i| !i.is_impl) {
        match exports.iter_mut().find(|(v, _)| *v == item.visibility) {
            Some((_, names)) => names.push(&item.name),
            None => exports.push((item.visibility.clone(), vec![&item.name])),
        }
    }
    let mut inserted = vec![format!("mod {module};")];
    inserted.extend(exports.iter().map(|(visibility, names)| format!("{visibility}use {module}::{};", braced(names))));
    let at = anchor(&out);
    // the lines stand with the file's `mod` lines, or as a group of their own
    if at > 0 && !out[at - 1].trim().is_empty() && !is_mod_line(&out[at - 1]) {
        inserted.insert(0, String::new());
    }
    if out.get(at).is_some_and(|l| !l.trim().is_empty() && !is_mod_line(l)) {
        inserted.push(String::new());
    }
    out.splice(at..at, inserted);
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    let from_text = out.join("\n") + "\n";

    let moved = moved.iter().map(|i| (i.name.clone(), i.start + 1, i.end + 1)).collect();
    Ok(Carve { from: from_text, to: to_text, moved })
}

/// Writes a carve under `root`: the old file over itself, the new file beside it, its
/// directory made when it is the first file there.
///
/// # Errors
/// When a file cannot be written.
pub fn write(root: &Path, from: &str, to: &str, carve: &Carve) -> anyhow::Result<()> {
    let to = root.join(to);
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot make {}", dir.display()))?;
    }
    std::fs::write(&to, &carve.to).with_context(|| format!("cannot write {}", to.display()))?;
    let from = root.join(from);
    std::fs::write(&from, &carve.from).with_context(|| format!("cannot write {}", from.display()))
}

/// The name a `mod` line gives `to`, after checking `to` is where that line finds it.
fn module_of(from: &str, to: &str) -> anyhow::Result<String> {
    let (from, to) = (Path::new(from), Path::new(to));
    if to.extension().and_then(|e| e.to_str()) != Some("rs") {
        bail!("{} is not a Rust file", to.display());
    }
    let stem = from.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent = from.parent().unwrap_or(Path::new(""));
    let home = if ["lib", "main", "mod"].contains(&stem) { parent.to_path_buf() } else { parent.join(stem) };
    if !to.parent().unwrap_or(Path::new("")).components().eq(home.components()) {
        bail!("{} is not where a `mod` line in {} finds it; it goes under {}", to.display(), from.display(), home.display());
    }
    let module = to.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if module.is_empty() || ["lib", "main", "mod", "tests"].contains(&module) {
        bail!("{} cannot be a carved module", to.display());
    }
    Ok(module.to_owned())
}

/// The items at the top level of a file, each from its doc comment and attributes to the line
/// its body or declaration closes on.
fn top_items(lines: &[&str]) -> Vec<Item> {
    let mut items = Vec::new();
    let mut n = 0;
    while n < lines.len() {
        let line = lines[n];
        let Some(declared) = (!line.starts_with([' ', '\t'])).then(|| rust_item(line)).flatten() else {
            n += 1;
            continue;
        };
        let mut start = n;
        while start > 0 && ["///", "#["].iter().any(|p| lines[start - 1].starts_with(p)) {
            start -= 1;
        }
        let end = end_of(lines, n);
        if let Some(name) = item_name(&declared) {
            let visibility = visibility_of(line);
            let rest = &line[visibility.len()..];
            let is_impl = rest.starts_with("impl") || rest.starts_with("unsafe impl");
            items.push(Item { name: name.to_owned(), visibility, is_impl, start, end });
        }
        n = end + 1;
    }
    items
}

/// The line an item opened on `first` closes on: where its braces close, or its `;` outside any.
fn end_of(lines: &[&str], first: usize) -> usize {
    let mut depth = 0usize;
    for (n, line) in lines.iter().enumerate().skip(first) {
        let chars: Vec<char> = line.chars().collect();
        let mut closed = false;
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '/' if chars.get(i + 1) == Some(&'/') => break,
                '"' => i = skip_string(&chars, i + 1),
                'r' if matches!(chars.get(i + 1), Some('"' | '#')) && (i == 0 || !is_word(chars[i - 1])) => i = skip_raw(&chars, i + 1),
                '\'' => i = skip_char(&chars, i),
                '{' | '(' | '[' => depth += 1,
                '}' | ')' | ']' => {
                    depth = depth.saturating_sub(1);
                    closed |= depth == 0 && chars[i] == '}';
                }
                ';' if depth == 0 => return n,
                _ => {}
            }
            i += 1;
        }
        if closed && depth == 0 {
            return n;
        }
    }
    lines.len().saturating_sub(1)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The index of a string's closing quote, the string opening before `i`.
fn skip_string(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return i,
            _ => i += 1,
        }
    }
    i
}

/// The index of a raw string's last closing `#` or its quote, `i` on its first `#` or its quote.
fn skip_raw(chars: &[char], mut i: usize) -> usize {
    let mut hashes = 0;
    while chars.get(i) == Some(&'#') {
        hashes += 1;
        i += 1;
    }
    if chars.get(i) != Some(&'"') {
        return i.saturating_sub(1);
    }
    i += 1;
    while i < chars.len() {
        if chars[i] == '"' && (1..=hashes).all(|h| chars.get(i + h) == Some(&'#')) {
            return i + hashes;
        }
        i += 1;
    }
    i
}

/// The index of a char literal's closing quote, or `i` itself for a lifetime.
fn skip_char(chars: &[char], i: usize) -> usize {
    match (chars.get(i + 1), chars.get(i + 2)) {
        (Some('\\'), _) => (i + 3..chars.len()).find(|&j| chars[j] == '\'').unwrap_or(i),
        (Some(_), Some('\'')) => i + 2,
        _ => i,
    }
}

/// The visibility a line opens with, `pub(crate) ` with its space, empty for a private item.
fn visibility_of(line: &str) -> String {
    let Some(after) = line.strip_prefix("pub") else { return String::new() };
    match after.strip_prefix('(').and_then(|s| s.split_once(')')) {
        Some((scope, _)) => format!("pub({scope}) "),
        None if after.starts_with(' ') => "pub ".to_owned(),
        None => String::new(),
    }
}

/// A moved item's line as the new file has it: a private item still visible to the old file,
/// a `pub(super)` one still visible to the old file's parent.
fn widened(line: &str, item: &Item) -> String {
    let declares = !line.starts_with(['#', '/', ' ', '\t', '}', ')', ']']) && rust_item(line).is_some();
    if !declares || item.is_impl {
        return line.to_owned();
    }
    match item.visibility.as_str() {
        "" => format!("pub(super) {line}"),
        "pub(super) " => format!("pub(in super::super) {}", &line[item.visibility.len()..]),
        _ => line.to_owned(),
    }
}

/// The `use` statements at the top level of a file.
fn uses_of<S: AsRef<str>>(lines: &[S]) -> Vec<Use> {
    let mut uses = Vec::new();
    let mut n = 0;
    while n < lines.len() {
        let line = lines[n].as_ref();
        if !line[visibility_of(line).len()..].starts_with("use ") {
            n += 1;
            continue;
        }
        let first = n;
        let mut statement = line.to_owned();
        while !statement.trim_end().ends_with(';') && n + 1 < lines.len() {
            n += 1;
            statement.push('\n');
            statement.push_str(lines[n].as_ref());
        }
        let tree = statement.split_once("use ").map_or("", |(_, t)| t).trim().trim_end_matches(';');
        let mut names = Vec::new();
        leaves(tree, "", &mut names);
        uses.push(Use { first, last: n, statement, names });
        n += 1;
    }
    uses
}

/// The names a use tree brings in: each path's last segment or its alias, `self` as the path
/// it stands in, `*` for a glob.
fn leaves(tree: &str, parent: &str, out: &mut Vec<String>) {
    let mut depth = 0;
    let mut part_start = 0;
    let mut parts = Vec::new();
    for (i, c) in tree.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&tree[part_start..i]);
                part_start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&tree[part_start..]);
    for part in parts.into_iter().map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(open) = part.find('{') {
            let prefix = part[..open].trim().trim_end_matches("::");
            let inner = part[open + 1..].trim_end().strip_suffix('}').unwrap_or(&part[open + 1..]);
            leaves(inner, prefix.rsplit("::").next().filter(|p| !p.is_empty()).unwrap_or(parent), out);
            continue;
        }
        let name = match part.split_once(" as ") {
            Some((_, alias)) => alias.trim(),
            None => match part.rsplit_once("::") {
                Some((before, "self")) => before.rsplit("::").next().unwrap_or(before),
                Some((_, last)) => last,
                None if part == "self" => parent,
                None => part,
            },
        };
        out.push(name.trim().to_owned());
    }
}

/// The lines of `lines` that are code, comments left out, as one text.
fn code_of<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    lines.filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

/// Whether `code` uses `name`: as a word, or as a trait by the methods or macros it brings.
fn mentions(code: &str, name: &str) -> bool {
    if TRAIT_USES.iter().any(|(t, shows)| *t == name && shows.iter().any(|s| code.contains(s))) {
        return true;
    }
    code.match_indices(name).any(|(at, _)| {
        let before = code[..at].chars().next_back();
        let after = code[at + name.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

/// A use statement of the old file as the new one, a module deeper, has it: private, and its
/// `self` and `super` paths one level up.
fn rerooted(statement: &str) -> String {
    let private = &statement[visibility_of(statement).len()..];
    private.replacen("use super::", "use super::super::", 1).replacen("use self::", "use super::", 1)
}

fn braced(names: &[&str]) -> String {
    match names {
        [one] => (*one).to_owned(),
        _ => format!("{{{}}}", names.join(", ")),
    }
}

/// Where the `mod` line goes: after the file's last top-level `mod` declaration, or its last
/// `use` statement, or its inner doc comment, ahead of its first other item.
fn anchor(lines: &[String]) -> usize {
    let first_item =
        lines.iter().position(|l| !l.starts_with([' ', '\t']) && rust_item(l).is_some() && !is_mod_line(l)).unwrap_or(lines.len());
    let head = &lines[..first_item];
    if let Some(last) = head.iter().rposition(|l| is_mod_line(l)) {
        return last + 1;
    }
    if let Some(last) = uses_of(head).last() {
        return last.last + 1;
    }
    head.iter().position(|l| !l.starts_with("//!") && !l.trim().is_empty()).unwrap_or(head.len())
}

fn is_mod_line(line: &str) -> bool {
    let rest = &line[visibility_of(line).len()..];
    rest.starts_with("mod ") && rest.trim_end().ends_with(';') && !rest.starts_with("mod tests")
}

/// The lines with no blank line at the start and none doubled, as a cut leaves them.
fn collapsed(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        let blank = line.trim().is_empty();
        if blank && out.last().is_none_or(|l| l.trim().is_empty()) {
            continue;
        }
        out.push(line);
    }
    out
}

/// The old file without the `use` statements only its moved items needed; a glob stays.
fn without_unused_uses(lines: &[String]) -> Vec<String> {
    let uses = uses_of(lines);
    let in_use = |n: usize| uses.iter().any(|u| (u.first..=u.last).contains(&n));
    let code = code_of(lines.iter().enumerate().filter(|(n, _)| !in_use(*n)).map(|(_, l)| l.as_str()));
    let unused: Vec<&Use> =
        uses.iter().filter(|u| !u.names.is_empty() && u.names.iter().all(|n| n != "*" && !mentions(&code, n))).collect();
    let kept = lines.iter().enumerate().filter(|(n, _)| !unused.iter().any(|u| (u.first..=u.last).contains(n))).map(|(_, l)| l.clone());
    collapsed(kept.collect())
}

#[cfg(test)]
mod tests;
