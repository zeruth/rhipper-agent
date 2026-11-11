//! The `rhipper.md` format: one numbered line per node, and the detail fields indented beneath it.
//! The file is the store, so a human or an agent reads and edits it with a plain text editor and
//! a git diff stays readable.
//!
//! A node line holds everything a row shows inline: the id, the kind, the title, the complexity,
//! the state and the assignee. The detail lines carry the longer fields. Dependencies name ids
//! only, so cross-links survive when lines move.
//!
//! ```text
//! # Rhipper Agent
//!
//! - 1 ‹project› Phase 1: Foundation {c3} [defined]
//!     objective: kickstart the engine and the providers end to end.
//!     - 4 ‹task› Roadmap storage layer {c2} [accepted] [implementer:z-ai/glm-5.3-flash]
//!         objective: ...
//!         acceptance: ...
//!         files: crates/ra-roadmap/src/store.rs crates/ra-console/src/agent.rs
//!         waits-on: 2 7
//!         effort: high
//!         notes: ...
//!         assessed: 9f3a0c41d27b6e58
//! ```
//!
//! `files:` lists the workspace-relative paths a task touches, space-separated, so a worker reads
//! them first. A path holds no space.
//!
//! `effort:` is `low`, `medium` or `high`, what a run asks of the model; a card without the line
//! runs at medium, so the line is written only for another level.
//!
//! `assessed:` holds what the last reassessment saw: a fingerprint of its inputs, or the word
//! `final`, which no reassessment rewrites.

use anyhow::Context;

use crate::model::{Assignment, Effort, Node, NodeKind, TaskState};
use crate::tree::depths;

/// The indent of one hierarchy level, in spaces.
const INDENT: usize = 2;
/// The extra indent of a detail line under its node line.
const DETAIL_INDENT: usize = 4;

/// Escapes a detail value onto one line: a newline becomes `\n` and a backslash becomes `\\`.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', "\\n")
}

/// Reverses [`escape`].
fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Renders the nodes as the text of a `rhipper.md`. The nodes come in tree order: a parent before
/// its children.
pub fn render(nodes: &[Node]) -> String {
    let depth = depths(nodes);
    let mut out = String::new();
    let title = nodes.iter().find(|n| n.kind == NodeKind::Project).map_or("Rhipper Agent", |n| n.title.as_str());
    out.push_str("# ");
    out.push_str(title);
    out.push_str("\n\n");
    for node in nodes {
        let level = depth.get(&node.id).copied().unwrap_or(0);
        out.push_str(&node_line(node, level));
        for detail in detail_lines(node, level) {
            out.push('\n');
            out.push_str(&detail);
        }
        out.push('\n');
    }
    out
}

/// The line of one node: the bullet, the id, the kind, the title and the inline fields.
fn node_line(node: &Node, level: usize) -> String {
    let mut line = format!("{}- {} ‹{}› {}", " ".repeat(level * INDENT), node.id, node.kind.serde_name(), node.title);
    line.push_str(&format!(" {{c{}}}", node.complexity));
    line.push_str(&format!(" [{}]", node.state.serde_name()));
    if let Some(assignee) = &node.assignee {
        line.push_str(&format!(" [{}:{}]", assignee.role, assignee.model));
    }
    line
}

/// The detail lines of one node, each indented under its node line. An empty field writes no line,
/// so the diff of a task that grows its notes stays small.
fn detail_lines(node: &Node, level: usize) -> Vec<String> {
    let indent = " ".repeat(level * INDENT + DETAIL_INDENT);
    let mut lines = Vec::new();
    for (field, value) in [
        ("objective", node.objective.as_deref()),
        ("acceptance", node.acceptance.as_deref()),
        ("notes", node.notes.as_deref()),
        ("assessed", node.assessed.as_deref()),
    ] {
        if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
            lines.push(format!("{indent}{field}: {}", escape(value)));
        }
    }
    if !node.files.is_empty() {
        lines.push(format!("{indent}files: {}", node.files.join(" ")));
    }
    if let Some(session) = node.session.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // the fingerprint and the time follow the id, so a run tells a stale session from a live one
        let line = match (node.session_of.as_deref(), node.session_at) {
            (Some(of), Some(at)) => format!("{indent}session: {session} {of} {at}"),
            _ => format!("{indent}session: {session}"),
        };
        lines.push(line);
    }
    if !node.depends_on.is_empty() {
        let ids: Vec<String> = node.depends_on.iter().map(|id| id.to_string()).collect();
        lines.push(format!("{indent}waits-on: {}", ids.join(" ")));
    }
    if node.effort != Effort::default() {
        lines.push(format!("{indent}effort: {}", node.effort));
    }
    lines
}

/// Parses the text of a `rhipper.md` into the nodes. It fails on a line it cannot read and names
/// the line number.
pub fn parse(text: &str) -> anyhow::Result<Vec<Node>> {
    let mut nodes: Vec<Node> = Vec::new();
    // The open node levels: (id, level). A node line closes every level at or below its own.
    let mut stack: Vec<(i64, usize)> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_end();
        let number = index + 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("- ") {
            let level = (raw.len() - raw.trim_start().len()) / INDENT;
            let node = node_of_line(rest).with_context(|| format!("line {number}: {}", line.trim_start()))?;
            while stack.last().is_some_and(|(_, open)| *open >= level) {
                stack.pop();
            }
            let parent = stack.last().map(|(id, _)| *id);
            stack.push((node.id, level));
            nodes.push(Node { parent, ..node });
            continue;
        }
        // A detail line belongs to the node above it.
        let (field, value) = line.trim_start().split_once(':').with_context(|| format!("line {number} reads no field"))?;
        let value = unescape(value.trim());
        let node = nodes.last_mut().with_context(|| format!("line {number} has no node above it"))?;
        match field.trim() {
            "objective" => node.objective = Some(value),
            "acceptance" => node.acceptance = Some(value),
            "notes" => node.notes = Some(value),
            "assessed" => node.assessed = Some(value),
            "files" => node.files = value.split_whitespace().map(str::to_owned).collect(),
            "session" => {
                let mut words = value.split_whitespace();
                node.session = words.next().map(str::to_owned);
                node.session_of = words.next().map(str::to_owned);
                node.session_at = words
                    .next()
                    .map(str::parse::<u64>)
                    .transpose()
                    .with_context(|| format!("line {number}: the session time is a number"))?;
            }
            "effort" => node.effort = value.parse().with_context(|| format!("line {number}"))?,
            "waits-on" => {
                node.depends_on = value
                    .split_whitespace()
                    .map(|id| id.parse::<i64>())
                    .collect::<Result<Vec<i64>, _>>()
                    .with_context(|| format!("line {number}: a dependency is a node id"))?;
            }
            other => anyhow::bail!("line {number}: unknown field {other}"),
        }
    }
    Ok(nodes)
}

/// One node line: the id, the kind, the title and the inline fields. The inline fields sit at the
/// end of the line, so the parser strips them from the right and leaves the title alone.
fn node_of_line(rest: &str) -> anyhow::Result<Node> {
    let (id_text, rest) = rest.split_once(char::is_whitespace).context("the line names no id")?;
    let id = id_text.parse::<i64>().context("the id is a number")?;
    let rest = rest.trim_start();
    let (kind_text, title_and_fields) = rest.strip_prefix('‹').and_then(|r| r.split_once('›')).context("the kind sits in ‹ ›")?;
    let kind: NodeKind = kind_text.parse().with_context(|| format!("unknown kind {kind_text}"))?;

    let mut title = title_and_fields.trim().to_owned();
    let mut assignee = None;
    let mut state = TaskState::Defined;
    let mut complexity = 3u8;
    // The fields read from the right: the assignee, then the state, then the complexity. A bracket
    // that holds none of them stays in the title.
    loop {
        let Some((before, field)) = title.rsplit_once(" [") else { break };
        let field = field.strip_suffix(']').unwrap_or(field);
        if let Some((role, model)) = field.split_once(':') {
            if !role.contains('‹') && role.parse::<TaskState>().is_err() {
                assignee = Some(Assignment { role: role.to_owned(), model: model.to_owned() });
                title = before.trim_end().to_owned();
                continue;
            }
        }
        if let Ok(parsed) = field.parse::<TaskState>() {
            state = parsed;
            title = before.trim_end().to_owned();
            continue;
        }
        break;
    }
    if let Some((before, field)) = title.rsplit_once(" {c") {
        if let Some(digits) = field.strip_suffix('}') {
            if let Ok(parsed) = digits.parse::<u8>() {
                complexity = parsed;
                title = before.trim_end().to_owned();
            }
        }
    }
    anyhow::ensure!(!title.is_empty(), "the title is empty");
    Ok(Node {
        id,
        parent: None,
        kind,
        title,
        objective: None,
        acceptance: None,
        files: Vec::new(),
        session: None,
        session_at: None,
        session_of: None,
        depends_on: Vec::new(),
        state,
        assignee,
        complexity,
        effort: Effort::default(),
        notes: None,
        assessed: None,
    })
}

#[cfg(test)]
mod tests;
