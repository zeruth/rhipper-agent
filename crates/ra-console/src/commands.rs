//! What can be typed at the console. The commands read and shape the roadmap; running a task
//! goes through the orchestrator, which the console drives through the same store.

use anyhow::Context;
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::Store;

pub struct Spec {
    pub name: &'static str,
    pub usage: &'static str,
    pub help: &'static str,
}

pub const COMMANDS: &[Spec] = &[
    Spec { name: "help", usage: "", help: "this list" },
    Spec { name: "show", usage: "[id]", help: "the roadmap, or one node with its subtree" },
    Spec {
        name: "add",
        usage: "<project|milestone|section|task|subtask> <title> [parent <id>]",
        help: "adds a node under its parent; a project takes no parent",
    },
    Spec { name: "depend", usage: "<id> <id>...", help: "makes the first node wait for the others" },
    Spec { name: "set", usage: "<id> <state>", help: "sets the state: the twelve lifecycle names" },
    Spec { name: "model", usage: "<id>", help: "the model of the next worker session, an OpenRouter id" },
    Spec { name: "clear", usage: "", help: "clears the log view" },
    Spec { name: "quit", usage: "", help: "stops the console (also exit, or Ctrl+C twice)" },
];

#[derive(Debug)]
pub enum Outcome {
    Continue,
    Clear,
    Quit,
    /// The console adopts the model on the badge: `model <id>` set it.
    Model(String),
}

/// Command names that start with `prefix`.
pub fn complete(prefix: &str) -> Vec<&'static str> {
    COMMANDS.iter().map(|c| c.name).filter(|n| n.starts_with(prefix)).collect()
}

/// Runs one line against the store; the lines to show, or an error to show.
pub async fn run(line: &str, store: &Store) -> anyhow::Result<(Vec<String>, Outcome)> {
    let line = line.trim().trim_start_matches(['/', '.']);
    let (name, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let args = rest.trim();
    let mut out = Vec::new();
    match name.to_ascii_lowercase().as_str() {
        "" => {}
        "help" | "?" => {
            for c in COMMANDS {
                let usage = if c.usage.is_empty() { String::new() } else { format!(" {}", c.usage) };
                out.push(format!("{}{usage}  —  {}", c.name, c.help));
            }
        }
        "show" => {
            let nodes = store.all().await?;
            let picked: Vec<&ra_roadmap::model::Node> = match args.parse::<i64>() {
                Ok(root) => {
                    let subtree = collect_subtree(&nodes, root);
                    anyhow::ensure!(!subtree.is_empty(), "no node {root}");
                    subtree
                }
                Err(_) => nodes.iter().collect(),
            };
            for node in &picked {
                out.push(format!(
                    "{}#{} {} [{}]{}",
                    "  ".repeat(u64::from(depth(&nodes, node)) as usize),
                    node.id,
                    node.title,
                    node.state.serde_name(),
                    match &node.assignee {
                        Some(a) => format!(" ← {}:{}", a.role, a.model),
                        None => String::new(),
                    }
                ));
                if let Some(objective) = &node.objective {
                    out.push(format!("    {objective}"));
                }
            }
            if picked.is_empty() {
                out.push("the roadmap is empty: add a project first".into());
            }
        }
        "add" => {
            let (kind_text, rest) = args.split_once(char::is_whitespace).context("usage: add <kind> <title>")?;
            let kind = kind_of(kind_text).context("the kind is project, milestone, section, task or subtask")?;
            let (title, parent) = match rest.split_once(" parent ") {
                Some((title, parent_text)) => {
                    let parent = parent_text.trim().parse::<i64>().context("the parent is a node id")?;
                    (title.trim(), Some(parent))
                }
                None => (rest.trim(), None),
            };
            anyhow::ensure!(!title.is_empty(), "usage: add <kind> <title> [parent <id>]");
            anyhow::ensure!(kind != NodeKind::Project || parent.is_none(), "a project has no parent");
            anyhow::ensure!(kind == NodeKind::Project || parent.is_some(), "a node below a project needs parent <id>");
            let id = store.insert(parent, kind, title, None, None, 3).await?;
            out.push(format!("#{id} {title}"));
        }
        "depend" => {
            let mut ids = args.split_whitespace().map(str::parse::<i64>);
            let id = ids.next().context("usage: depend <id> <id>...")??;
            let mut count = 0;
            for other in ids {
                store.depend_on(id, other?).await?;
                count += 1;
            }
            anyhow::ensure!(count > 0, "usage: depend <id> <id>...");
            out.push(format!("#{id} now waits for {count} more"));
        }
        "set" => {
            let (id_text, state_text) = args.split_once(char::is_whitespace).context("usage: set <id> <state>")?;
            let id = id_text.parse::<i64>().context("the id is a node id")?;
            let state = state_of(state_text).context("that is no state of the lifecycle")?;
            store.set_state(id, state).await?;
            out.push(format!("#{id} is now {}", state.serde_name()));
        }
        "clear" | "cls" => return Ok((out, Outcome::Clear)),
        "model" => {
            anyhow::ensure!(!args.is_empty(), "usage: model <openrouter id>, such as z-ai/glm-5.3-flash");
            out.push(format!("the next worker session runs on {args}"));
            return Ok((out, Outcome::Model(args.to_owned())));
        }
        "quit" | "exit" | "shutdown" | "stop" => return Ok((out, Outcome::Quit)),
        other => anyhow::bail!("no command {other}; help lists them"),
    }
    Ok((out, Outcome::Continue))
}

fn kind_of(text: &str) -> Option<NodeKind> {
    match text {
        "project" => Some(NodeKind::Project),
        "milestone" => Some(NodeKind::Milestone),
        "section" => Some(NodeKind::Section),
        "task" => Some(NodeKind::Task),
        "subtask" => Some(NodeKind::Subtask),
        _ => None,
    }
}

fn state_of(text: &str) -> Option<TaskState> {
    let states = [
        ("defined", TaskState::Defined),
        ("ready", TaskState::Ready),
        ("assigned", TaskState::Assigned),
        ("in_progress", TaskState::InProgress),
        ("implemented", TaskState::Implemented),
        ("testing", TaskState::Testing),
        ("review", TaskState::Review),
        ("accepted", TaskState::Accepted),
        ("blocked", TaskState::Blocked),
        ("failed", TaskState::Failed),
        ("needs_revision", TaskState::NeedsRevision),
        ("cancelled", TaskState::Cancelled),
    ];
    states.iter().find(|(name, _)| *name == text).map(|(_, state)| *state)
}

fn collect_subtree<'a>(nodes: &'a [ra_roadmap::model::Node], root: i64) -> Vec<&'a ra_roadmap::model::Node> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if let Some(node) = nodes.iter().find(|n| n.id == id) {
            out.push(node);
            for child in nodes.iter().filter(|n| n.parent == Some(id)).map(|n| n.id) {
                stack.push(child);
            }
        }
    }
    out.sort_by_key(|n| n.id);
    out
}

fn depth(nodes: &[ra_roadmap::model::Node], node: &ra_roadmap::model::Node) -> u64 {
    let mut depth = 0u64;
    let mut parent = node.parent;
    while let Some(id) = parent {
        parent = nodes.iter().find(|n| n.id == id).and_then(|n| n.parent);
        depth += 1;
    }
    depth
}

#[cfg(test)]
mod tests;
