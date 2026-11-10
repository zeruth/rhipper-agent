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
//!         waits-on: 2 7
//!         notes: ...
//! ```

use std::collections::HashMap;

use anyhow::Context;

use crate::model::{Assignment, Node, NodeKind, TaskState};

/// The indent of one hierarchy level, in spaces.
const INDENT: usize = 2;
/// The extra indent of a detail line under its node line.
const DETAIL_INDENT: usize = 4;

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
    for (field, value) in
        [("objective", node.objective.as_deref()), ("acceptance", node.acceptance.as_deref()), ("notes", node.notes.as_deref())]
    {
        if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
            lines.push(format!("{indent}{field}: {value}"));
        }
    }
    if !node.depends_on.is_empty() {
        let ids: Vec<String> = node.depends_on.iter().map(|id| id.to_string()).collect();
        lines.push(format!("{indent}waits-on: {}", ids.join(" ")));
    }
    lines
}

/// The depth of every node: the distance from its project root.
fn depths(nodes: &[Node]) -> HashMap<i64, usize> {
    let by_id: HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    let mut depth = HashMap::new();
    for node in nodes {
        let mut level = 0;
        let mut parent = node.parent;
        while let Some(id) = parent {
            parent = by_id.get(&id).and_then(|n| n.parent);
            level += 1;
        }
        depth.insert(node.id, level);
    }
    depth
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
        let value = value.trim();
        let node = nodes.last_mut().with_context(|| format!("line {number} has no node above it"))?;
        match field.trim() {
            "objective" => node.objective = Some(value.to_owned()),
            "acceptance" => node.acceptance = Some(value.to_owned()),
            "notes" => node.notes = Some(value.to_owned()),
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
    let kind = match kind_text {
        "project" => NodeKind::Project,
        "milestone" => NodeKind::Milestone,
        "section" => NodeKind::Section,
        "task" => NodeKind::Task,
        "subtask" => NodeKind::Subtask,
        other => anyhow::bail!("unknown kind {other}"),
    };

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
            if !role.contains('‹') && TaskState::parse(role).is_none() {
                assignee = Some(Assignment { role: role.to_owned(), model: model.to_owned() });
                title = before.trim_end().to_owned();
                continue;
            }
        }
        if let Some(parsed) = TaskState::parse(field) {
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
        depends_on: Vec::new(),
        state,
        assignee,
        complexity,
        notes: None,
    })
}

impl TaskState {
    /// The state of its store name, for the parser.
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "defined" => Self::Defined,
            "ready" => Self::Ready,
            "assigned" => Self::Assigned,
            "in_progress" => Self::InProgress,
            "implemented" => Self::Implemented,
            "testing" => Self::Testing,
            "review" => Self::Review,
            "accepted" => Self::Accepted,
            "blocked" => Self::Blocked,
            "failed" => Self::Failed,
            "needs_revision" => Self::NeedsRevision,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests;
