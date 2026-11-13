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
//! ## Gambits
//!
//! - g1 ‹gambit› Local execution before dollars {w4} [active]
//!     focus: provider local ra-providers
//!
//! - 1 ‹project› Phase 1: Foundation {c3} [defined]
//!     objective: kickstart the engine and the providers end to end.
//!     - 4 ‹task› Roadmap storage layer {c2} [accepted] [implementer:z-ai/glm-5.3-flash]
//!         objective: ...
//!         acceptance: ...
//!         files: crates/ra-roadmap/src/store.rs crates/ra-console/src/agent.rs
//!         waits-on: 2 7
//!         effort: high
//!         estimate: 40k tokens, 12 turns
//!         priority: 5
//!         notes: ...
//!         assessed: 9f3a0c41d27b6e58
//!         evidence: var/retro.log:41
//!         saving: two turns a run
//! ```
//!
//! The gambits come first, since the walk reads the file in order until the gambits decide it:
//! one line each, the weight `{w1}` to `{w5}`, the state `active`, `parked` or `done`, and an
//! optional `focus:` line of the words a card matches on, and an optional `plan-next: true` line
//! that lets the planner fill the roadmap when no card is ready. A done gambit stays in the file.
//!
//! `priority:` is the person's priority on a card, 1 to 5, apart from its complexity; a card
//! without the line has none.
//!
//! `files:` lists the workspace-relative paths a task touches, space-separated, so a worker reads
//! them first. A path holds no space.
//!
//! `effort:` is `low`, `medium` or `high`, what a run asks of the model; a card without the line
//! runs at medium, so the line is written only for another level.
//!
//! `estimate:` is what a run of the card is expected to spend, the tokens then the turns; a card
//! without the line takes the estimate of its complexity.
//!
//! `assessed:` holds what the last reassessment saw: a fingerprint of its inputs, or the word
//! `final`, which no reassessment rewrites; after it, `-` and a hash per proposed card the person
//! declined.
//!
//! `evidence:` is the line a planned card came from, `source:line`, and `saving:` what it claims
//! to save; a card the person wrote has neither.

use std::fmt::Write as _;

use anyhow::Context;

use crate::model::{Assignment, Effort, Gambit, GambitState, Node, NodeKind, Roadmap, TaskState, WEIGHT_RANGE};
use crate::tree::{self, depths};

/// The indent of one hierarchy level, in spaces.
const INDENT: usize = 2;
/// The extra indent of a detail line under its node line.
const DETAIL_INDENT: usize = 4;
/// The heading over the gambit lines.
const GAMBITS_HEADING: &str = "## Gambits";

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

/// Renders the nodes alone as the text of a `rhipper.md`, without gambits. The nodes come in
/// tree order: a parent before its children.
pub fn render(nodes: &[Node]) -> String {
    render_file(&Roadmap { gambits: Vec::new(), nodes: nodes.to_vec() })
}

/// Renders the whole file: the title, the gambits under their heading when there are any, then
/// the tree.
pub fn render_file(roadmap: &Roadmap) -> String {
    let nodes = &roadmap.nodes;
    let depth = depths(nodes);
    let mut out = String::new();
    let title = nodes.iter().find(|n| n.kind == NodeKind::Project).map_or("Rhipper Agent", |n| n.title.as_str());
    out.push_str("# ");
    out.push_str(title);
    out.push_str("\n\n");
    if !roadmap.gambits.is_empty() {
        out.push_str(GAMBITS_HEADING);
        out.push_str("\n\n");
        for gambit in &roadmap.gambits {
            out.push_str(&gambit_line(gambit));
            out.push('\n');
            if !gambit.focus.is_empty() {
                let _ = writeln!(out, "{}focus: {}", " ".repeat(DETAIL_INDENT), gambit.focus.join(" "));
            }
            if gambit.plan_next {
                let _ = writeln!(out, "{}plan-next: true", " ".repeat(DETAIL_INDENT));
            }
        }
        out.push('\n');
    }
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

/// Renders the tree, or the subtree under `root`, as a plain Markdown checklist for a README or
/// an issue: `- [x]` for an accepted card, `- [ ]` otherwise, nested by level, the objective
/// indented as a continuation line. A subtree starts at column 0, as if it were the whole
/// roadmap. The state is the effective one, so a parent whose children are all accepted ticks.
pub fn checklist(nodes: &[Node], root: Option<i64>) -> String {
    let picked: Vec<&Node> = match root {
        Some(root) => tree::subtree(nodes, root),
        None => nodes.iter().collect(),
    };
    let depth = depths(nodes);
    let base = picked.first().map_or(0, |n| depth.get(&n.id).copied().unwrap_or(0));
    let states = tree::effective_states(nodes);
    let mut lines = Vec::new();
    for node in picked {
        let level = depth.get(&node.id).copied().unwrap_or(0).saturating_sub(base);
        let indent = " ".repeat(level * INDENT);
        let mark = if states.get(&node.id).copied().unwrap_or(node.state) == TaskState::Accepted { 'x' } else { ' ' };
        lines.push(format!("{indent}- [{mark}] {}\n", node.title));
        if let Some(objective) = node.objective.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
            lines.extend(objective.lines().map(|line| format!("{indent}  {line}\n")));
        }
    }
    lines.concat()
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
        ("evidence", node.evidence.as_deref()),
        ("saving", node.saving.as_deref()),
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
    if let Some(estimate) = node.estimate {
        lines.push(format!("{indent}estimate: {estimate}"));
    }
    if let Some(priority) = node.priority {
        lines.push(format!("{indent}priority: {priority}"));
    }
    if node.absolute {
        lines.push(format!("{indent}absolute: true"));
    }
    lines
}

/// The line of one gambit: the bullet, the `g` id, the text, the weight and the state.
fn gambit_line(gambit: &Gambit) -> String {
    format!("- g{} ‹gambit› {} {{w{}}} [{}]", gambit.id, gambit.text, gambit.weight, gambit.state.serde_name())
}

/// Parses the text of a `rhipper.md` into the nodes, leaving the gambits aside. It fails on a
/// line it cannot read and names the line number.
pub fn parse(text: &str) -> anyhow::Result<Vec<Node>> {
    parse_file(text).map(|roadmap| roadmap.nodes)
}

/// Parses the whole file: the gambits and the nodes. It fails on a line it cannot read and names
/// the line number.
pub fn parse_file(text: &str) -> anyhow::Result<Roadmap> {
    let mut gambits: Vec<Gambit> = Vec::new();
    let mut nodes: Vec<Node> = Vec::new();
    // The open node levels: (id, level). A node line closes every level at or below its own.
    let mut stack: Vec<(i64, usize)> = Vec::new();
    // Whether the last bullet was a gambit, so a detail line knows what it belongs to.
    let mut under_gambit = false;
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_end();
        let number = index + 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("- ") {
            if rest.starts_with('g') && rest.contains("‹gambit›") {
                gambits.push(gambit_of_line(rest).with_context(|| format!("line {number}: {}", line.trim_start()))?);
                under_gambit = true;
                continue;
            }
            under_gambit = false;
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
        // A detail line belongs to the bullet above it.
        let (field, value) = line.trim_start().split_once(':').with_context(|| format!("line {number} reads no field"))?;
        let value = unescape(value.trim());
        if under_gambit {
            let gambit = gambits.last_mut().with_context(|| format!("line {number} has no gambit above it"))?;
            match field.trim() {
                "focus" => gambit.focus = value.split_whitespace().map(str::to_owned).collect(),
                "plan-next" => {
                    gambit.plan_next = match value.as_str() {
                        "true" => true,
                        "false" => false,
                        other => anyhow::bail!("line {number}: plan-next is true or false, not {other}"),
                    };
                }
                other => anyhow::bail!("line {number}: unknown gambit field {other}"),
            }
            continue;
        }
        let node = nodes.last_mut().with_context(|| format!("line {number} has no node above it"))?;
        match field.trim() {
            "objective" => node.objective = Some(value),
            "acceptance" => node.acceptance = Some(value),
            "notes" => node.notes = Some(value),
            "assessed" => node.assessed = Some(value),
            "evidence" => node.evidence = Some(value),
            "saving" => node.saving = Some(value),
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
            "estimate" => node.estimate = Some(value.parse().with_context(|| format!("line {number}"))?),
            "priority" => node.priority = Some(weight_of(&value, "priority").with_context(|| format!("line {number}"))?),
            "absolute" => node.absolute = matches!(value.trim(), "true" | "yes"),
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
    Ok(Roadmap { gambits, nodes })
}

/// A priority or a weight: a digit in [`WEIGHT_RANGE`]; `what` names it in the error.
fn weight_of(text: &str, what: &str) -> anyhow::Result<u8> {
    text.trim()
        .parse::<u8>()
        .ok()
        .filter(|w| WEIGHT_RANGE.contains(w))
        .with_context(|| format!("the {what} is {} to {}", WEIGHT_RANGE.start(), WEIGHT_RANGE.end()))
}

/// One gambit line: the `g` id, the text, the weight and the state. The weight and the state are
/// not optional, since the walk reads them; the text keeps any other bracket.
fn gambit_of_line(rest: &str) -> anyhow::Result<Gambit> {
    let (id_text, rest) = rest.split_once(char::is_whitespace).context("the line names no id")?;
    let id = id_text.strip_prefix('g').and_then(|digits| digits.parse::<u32>().ok()).context("a gambit id is g then a number")?;
    let text_and_fields = rest.trim_start().strip_prefix("‹gambit›").context("the kind sits in ‹ ›")?;
    let (text, state) = text_and_fields.trim().rsplit_once(" [").context("the gambit names no state in [ ]")?;
    let state: GambitState = state.strip_suffix(']').unwrap_or(state).parse()?;
    let (text, weight) = text.trim_end().rsplit_once(" {w").context("the gambit names no weight as {w3}")?;
    let weight = weight_of(weight.strip_suffix('}').unwrap_or(weight), "weight")?;
    let text = text.trim().to_owned();
    anyhow::ensure!(!text.is_empty(), "the gambit text is empty");
    Ok(Gambit { id, text, weight, state, focus: Vec::new(), plan_next: false })
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
        estimate: None,
        notes: None,
        assessed: None,
        priority: None,
        absolute: false,
        evidence: None,
        saving: None,
    })
}

#[cfg(test)]
mod tests;
