//! A plain outline of a roadmap: the format a person types, a model answers with, and `rc init`
//! and `rc import` read into the store. It is Markdown anyone can write without knowing the
//! ids and the fields of `rhipper.md`:
//!
//! ```text
//! # The project
//! ## A milestone {c3}
//! objective: what it achieves
//! acceptance: how it is checked
//! ### A section
//! - A task {c2}
//!   objective: ...
//!   acceptance: `cargo test -p x` passes
//!   files: src/a.rs src/b.rs
//!   waits: above, #12, start of another title
//!   - A subtask [done]
//! ```
//!
//! `{cN}` is the complexity, 3 unless said; `[done]` marks an item accepted already. A field
//! line belongs to the item above it; `accept:` reads as `acceptance:`. An indented line with
//! no field name is the objective of the item above it, so a plain outline needs no field names
//! at all. Lines that are none of these are prose and stay out.

use anyhow::Context;

use crate::model::NodeKind;

/// One item of an outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: NodeKind,
    pub title: String,
    pub complexity: u8,
    pub done: bool,
    pub objective: Option<String>,
    pub acceptance: Option<String>,
    pub files: Vec<String>,
    /// What the item waits for: `above`, `#N`, or the start of another title in the outline.
    pub waits: Vec<String>,
}

/// An outline:the project's title when the text names one, and its items in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    pub title: Option<String>,
    pub items: Vec<Item>,
}

impl Outline {
    /// How many items of each kind, as `3 milestones, 2 sections, 14 tasks`.
    pub fn summary(&self) -> String {
        let count = |kind: NodeKind| self.items.iter().filter(|i| i.kind == kind).count();
        let mut parts = Vec::new();
        for (kind, name) in
            [(NodeKind::Milestone, "milestone"), (NodeKind::Section, "section"), (NodeKind::Task, "task"), (NodeKind::Subtask, "subtask")]
        {
            let n = count(kind);
            if n > 0 {
                parts.push(format!("{n} {name}{}", if n == 1 { "" } else { "s" }));
            }
        }
        parts.join(", ")
    }

    /// The items as an import would nest them, one line each, indented by depth, with the
    /// objective and the acceptance beneath: what `rc import --dry` prints.
    pub fn preview(&self) -> Vec<String> {
        let mut lines = Vec::new();
        // the depth of the last milestone, section and task, as the import nests
        let (mut section, mut task) = (None, None);
        for item in &self.items {
            let depth = match item.kind {
                NodeKind::Project | NodeKind::Milestone => 0,
                NodeKind::Section => 1,
                NodeKind::Task => section.map_or(1, |d| d + 1),
                NodeKind::Subtask => task.or(section).map_or(1, |d| d + 1),
            };
            match item.kind {
                NodeKind::Project | NodeKind::Milestone => (section, task) = (None, None),
                NodeKind::Section => (section, task) = (Some(depth), None),
                NodeKind::Task => task = Some(depth),
                NodeKind::Subtask => {}
            }
            let pad = "  ".repeat(depth);
            let kind = if item.kind == NodeKind::Project { NodeKind::Milestone } else { item.kind };
            let done = if item.done { " [accepted]" } else { "" };
            lines.push(format!("{pad}{kind} {} {{c{}}}{done}", item.title, item.complexity));
            if let Some(objective) = &item.objective {
                lines.push(format!("{pad}    objective: {objective}"));
            }
            if let Some(acceptance) = &item.acceptance {
                lines.push(format!("{pad}    acceptance: {acceptance}"));
            }
            if !item.files.is_empty() {
                lines.push(format!("{pad}    files: {}", item.files.join(" ")));
            }
        }
        lines
    }
}

/// Parses an outline. A code fence around it, as a model may add, is ignored.
///
/// # Errors
/// When a field line has no item above it, or the text holds no item at all.
pub fn parse(text: &str) -> anyhow::Result<Outline> {
    let mut outline = Outline::default();
    for (number, raw) in text.lines().enumerate() {
        let number = number + 1;
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if let Some(title) = trimmed.strip_prefix("# ") {
            outline.title.get_or_insert_with(|| title.trim().to_owned());
            continue;
        }
        let heading = if let Some(t) = trimmed.strip_prefix("### ") {
            Some((NodeKind::Section, t))
        } else if let Some(t) = trimmed.strip_prefix("## ") {
            Some((NodeKind::Milestone, t))
        } else if let Some(t) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            Some((if indent >= 2 { NodeKind::Subtask } else { NodeKind::Task }, t))
        } else {
            None
        };
        if let Some((kind, rest)) = heading {
            let (title, complexity, done) = title_of(rest);
            // a task line that is itself a field of a list is not a title
            if title.is_empty() {
                continue;
            }
            outline.items.push(Item {
                kind,
                title,
                complexity,
                done,
                objective: None,
                acceptance: None,
                files: Vec::new(),
                waits: Vec::new(),
            });
            continue;
        }
        if let Some((field, value)) = trimmed.split_once(':')
            && matches!(field.trim().to_ascii_lowercase().as_str(), "objective" | "acceptance" | "accept" | "files" | "waits")
        {
            let item = outline.items.last_mut().with_context(|| format!("line {number}: a field with no item above it"))?;
            let value = value.trim();
            match field.trim().to_ascii_lowercase().as_str() {
                "objective" => item.objective = Some(value.to_owned()),
                "acceptance" | "accept" => item.acceptance = Some(value.to_owned()),
                "waits" => item.waits = value.split(',').map(|w| w.trim().to_owned()).filter(|w| !w.is_empty()).collect(),
                _ => item.files = value.split_whitespace().map(|f| f.trim_matches('`').to_owned()).collect(),
            }
            continue;
        }
        // an indented line under an item is its objective, several lines joining as one; a line
        // at the margin is prose
        if indent > 0
            && let Some(item) = outline.items.last_mut()
        {
            match &mut item.objective {
                Some(objective) => {
                    objective.push(' ');
                    objective.push_str(trimmed);
                }
                None => item.objective = Some(trimmed.to_owned()),
            }
        }
    }
    anyhow::ensure!(!outline.items.is_empty(), "the outline holds no item: no `## milestone` or `- task` line");
    Ok(outline)
}

/// The title of a heading line without its marks: `{cN}` sets the complexity, `[done]` the
/// state; the marks may come in any order at the end.
fn title_of(rest: &str) -> (String, u8, bool) {
    let mut title = rest.trim().to_owned();
    let mut complexity = 3;
    let mut done = false;
    loop {
        let t = title.trim_end();
        if let Some(before) = t.strip_suffix("[done]").or_else(|| t.strip_suffix("[accepted]")) {
            done = true;
            title = before.to_owned();
        } else if let Some((before, mark)) = t.rsplit_once("{c")
            && let Some(digit) = mark.strip_suffix('}').and_then(|d| d.parse::<u8>().ok())
        {
            complexity = digit.clamp(1, 5);
            title = before.to_owned();
        } else {
            break;
        }
    }
    (title.trim().trim_end_matches(':').to_owned(), complexity, done)
}

#[cfg(test)]
mod tests;
