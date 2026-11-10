//! Rhipper Agent: the roadmap-driven multi-model development orchestrator. The binary parses the
//! command line and hands over to the subcommands; the work lives in the crates.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::Store;

#[derive(Parser)]
#[command(name = "rc", version, about = "Rhipper Agent: the roadmap-driven multi-model development orchestrator")]
struct Options {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Shows the roadmap, or one node with its subtree.
    Show {
        /// The id of the node; the whole roadmap when left out.
        id: Option<i64>,
    },
    /// Adds a node to the roadmap.
    Add {
        /// The kind: project, milestone, section, task or subtask.
        kind: NodeKindArg,
        /// The title.
        title: String,
        /// The parent node; a project takes none.
        #[arg(long)]
        parent: Option<i64>,
        /// The objective.
        #[arg(long)]
        objective: Option<String>,
        /// The acceptance criteria.
        #[arg(long)]
        acceptance: Option<String>,
        /// Complexity 1..=5.
        #[arg(long, default_value_t = 3)]
        complexity: u8,
    },
    /// Links a node to the nodes it waits for.
    Depend {
        /// The node that waits.
        id: i64,
        /// The nodes it waits for.
        waits_for: Vec<i64>,
    },
    /// Sets the state of a node.
    Set { id: i64, state: TaskStateArg },
    /// Starts the console over the orchestrator.
    Console {
        /// The roadmap database.
        #[arg(long, default_value = "var/roadmap.db")]
        db: PathBuf,
    },
}

/// A kind on the command line; clap parses it from its snake-case name.
#[derive(Clone, Copy)]
struct NodeKindArg(NodeKind);

/// A state on the command line; clap parses it from its snake-case name.
#[derive(Clone, Copy)]
struct TaskStateArg(TaskState);

impl std::str::FromStr for NodeKindArg {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "project" => Ok(Self(NodeKind::Project)),
            "milestone" => Ok(Self(NodeKind::Milestone)),
            "section" => Ok(Self(NodeKind::Section)),
            "task" => Ok(Self(NodeKind::Task)),
            "subtask" => Ok(Self(NodeKind::Subtask)),
            _ => Err("the kind is project, milestone, section, task or subtask".into()),
        }
    }
}

impl std::fmt::Display for NodeKindArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.serde_name())
    }
}

impl std::str::FromStr for TaskStateArg {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
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
        states
            .iter()
            .find(|(name, _)| *name == text)
            .map(|(_, state)| Self(*state))
            .ok_or_else(|| "the state is one of: defined, ready, assigned, in_progress, implemented, testing, review, accepted, blocked, failed, needs_revision, cancelled".to_owned())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let options = Options::parse();
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    match options.command {
        Command::Show { id } => show(id).await,
        Command::Add { kind, title, parent, objective, acceptance, complexity } => {
            add(kind.0, &title, parent, objective.as_deref(), acceptance.as_deref(), complexity).await
        }
        Command::Depend { id, waits_for } => depend(id, &waits_for).await,
        Command::Set { id, state } => set(id, state.0).await,
        Command::Console { db } => console(db).await,
    }
}

/// The default store location; the console subcommand creates it.
fn store_path() -> PathBuf {
    PathBuf::from("var/roadmap.db")
}

async fn open_store() -> anyhow::Result<Store> {
    Store::open(&store_path()).await
}

async fn show(id: Option<i64>) -> anyhow::Result<()> {
    let store = open_store().await?;
    let nodes = store.all().await?;
    let nodes = match id {
        Some(id) => {
            let subtree: Vec<_> = collect_subtree(&nodes, id);
            if subtree.is_empty() {
                anyhow::bail!("no node {id}");
            }
            subtree
        }
        None => nodes,
    };
    for node in &nodes {
        let indent = "  ".repeat(u32::try_from(depth(&nodes, node)).unwrap_or(0) as usize);
        let assignee = match &node.assignee {
            Some(a) => format!(" ← {}:{}", a.role, a.model),
            None => String::new(),
        };
        println!("{}#{} {} [{:?}]{assignee}", indent, node.id, node.title, node.state);
        if let Some(objective) = &node.objective {
            println!("{}    {}", indent, objective);
        }
    }
    Ok(())
}

fn collect_subtree(nodes: &[ra_roadmap::model::Node], root: i64) -> Vec<ra_roadmap::model::Node> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if let Some(node) = nodes.iter().find(|n| n.id == id) {
            out.push(node.clone());
            for child in nodes.iter().filter(|n| n.parent == Some(id)).map(|n| n.id) {
                stack.push(child);
            }
        }
    }
    out.sort_by_key(|n| n.id);
    out
}

fn depth(nodes: &[ra_roadmap::model::Node], node: &ra_roadmap::model::Node) -> i64 {
    let mut depth = 0;
    let mut parent = node.parent;
    while let Some(id) = parent {
        parent = nodes.iter().find(|n| n.id == id).and_then(|n| n.parent);
        depth += 1;
    }
    depth
}

async fn add(
    kind: NodeKind,
    title: &str,
    parent: Option<i64>,
    objective: Option<&str>,
    acceptance: Option<&str>,
    complexity: u8,
) -> anyhow::Result<()> {
    if complexity == 0 || complexity > 5 {
        anyhow::bail!("complexity is 1 to 5");
    }
    let store = open_store().await?;
    if kind == NodeKind::Project && parent.is_some() {
        anyhow::bail!("a project has no parent");
    }
    if kind != NodeKind::Project && parent.is_none() {
        anyhow::bail!("{}{} needs --parent", if kind == NodeKind::Milestone { "a" } else { "a " }, kind.serde_name());
    }
    let id = store.insert(parent, kind, title, objective, acceptance, complexity).await?;
    println!("#{id} {title}");
    Ok(())
}

async fn depend(id: i64, waits_for: &[i64]) -> anyhow::Result<()> {
    let store = open_store().await?;
    for other in waits_for {
        store.depend_on(id, *other).await?;
    }
    println!("#{id} waits for {:?}", waits_for);
    Ok(())
}

async fn set(id: i64, state: TaskState) -> anyhow::Result<()> {
    let store = open_store().await?;
    store.set_state(id, state).await?;
    println!("#{id} is now {}", state.serde_name());
    Ok(())
}

async fn console(db: PathBuf) -> anyhow::Result<()> {
    let store = Store::open(&db).await?;
    let _nodes = store.all().await?;
    println!("console is not wired yet; the store at {} opens", db.display());
    Ok(())
}
