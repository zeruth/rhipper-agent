//! Rhipper Agent: the roadmap-driven multi-model development orchestrator. The binary parses the
//! command line and hands over to the subcommands; the work lives in the crates.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use ra_console::agent::Worker;
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::Store;
use ra_roadmap::tree;
use ra_usage::UsageStore;

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
        kind: NodeKind,
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
    Set {
        id: i64,
        /// The state: one of the twelve lifecycle names.
        state: TaskState,
    },
    /// One round trip to a model: the answer and the usage.
    Ping {
        /// The model: a claude-* id runs on Claude Code, vendor/model on OpenRouter.
        #[arg(long, default_value = ra_console::ui::DEFAULT_MODEL)]
        model: String,
    },
    /// The usage ledger: the calls, the tokens and the cost of every model, all time.
    Usage,
    /// Starts the console over the orchestrator.
    Console {
        /// The roadmap file.
        #[arg(long, default_value = "rhipper.md")]
        roadmap: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let options = Options::parse();
    match options.command {
        Command::Show { id } => {
            plain_tracing()?;
            show(id).await
        }
        Command::Add { kind, title, parent, objective, acceptance, complexity } => {
            plain_tracing()?;
            add(kind, &title, parent, objective.as_deref(), acceptance.as_deref(), complexity).await
        }
        Command::Depend { id, waits_for } => {
            plain_tracing()?;
            depend(id, &waits_for).await
        }
        Command::Set { id, state } => {
            plain_tracing()?;
            set(id, state).await
        }
        Command::Ping { model } => {
            plain_tracing()?;
            let worker = Worker::new(model).with_ledger(open_ledger().await?);
            for line in ra_console::agent::ping(&worker).await? {
                println!("{line}");
            }
            Ok(())
        }
        Command::Usage => {
            plain_tracing()?;
            usage().await
        }
        Command::Console { roadmap } => console(roadmap).await,
    }
}

/// Opens the usage ledger under `var/`.
async fn open_ledger() -> anyhow::Result<UsageStore> {
    std::fs::create_dir_all("var")?;
    UsageStore::open(Path::new("var/usage.db")).await
}

/// Prints the ledger by model, the costliest first, and the total.
async fn usage() -> anyhow::Result<()> {
    let ledger = open_ledger().await?;
    let rows = ledger.by_model().await?;
    if rows.is_empty() {
        println!("the ledger is empty: no completion has run yet");
        return Ok(());
    }
    let cost = |c: Option<f64>| c.map_or("—".to_owned(), |c| format!("${c:.4}"));
    println!("{:<34} {:>6} {:>10} {:>10} {:>10} {:>10}", "model", "calls", "in", "out", "cached", "cost");
    for (model, t) in &rows {
        println!(
            "{model:<34} {:>6} {:>10} {:>10} {:>10} {:>10}",
            t.calls,
            t.prompt_tokens,
            t.completion_tokens,
            t.cached_tokens,
            cost(t.cost_usd)
        );
    }
    let all = ledger.totals().await?;
    println!(
        "{:<34} {:>6} {:>10} {:>10} {:>10} {:>10}",
        "total",
        all.calls,
        all.prompt_tokens,
        all.completion_tokens,
        all.cached_tokens,
        cost(all.cost_usd)
    );
    Ok(())
}

/// The default roadmap file; the commands open it in the working directory.
fn store_path() -> PathBuf {
    PathBuf::from("rhipper.md")
}

/// Opens the trace file and initializes the subscriber for the plain subcommands; they hold no
/// console, so the file is the only destination.
fn plain_tracing() -> anyhow::Result<()> {
    std::fs::create_dir_all("var")?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open("var/console.log")?;
    ra_console::trace::init(log, None);
    Ok(())
}

async fn open_store() -> anyhow::Result<Store> {
    Store::open(&store_path(), "Rhipper Agent").await
}

async fn show(id: Option<i64>) -> anyhow::Result<()> {
    let store = open_store().await?;
    let nodes = store.all().await?;
    let picked: Vec<_> = match id {
        Some(id) => {
            let subtree = tree::subtree(&nodes, id);
            if subtree.is_empty() {
                anyhow::bail!("no node {id}");
            }
            subtree
        }
        None => nodes.iter().collect(),
    };
    for node in picked {
        let indent = "  ".repeat(tree::depth(&nodes, node.id));
        let assignee = match &node.assignee {
            Some(a) => format!(" ← {}:{}", a.role, a.model),
            None => String::new(),
        };
        println!("{}#{} {} [{}]{assignee}", indent, node.id, node.title, node.state);
        if let Some(objective) = &node.objective {
            println!("{}    {}", indent, objective);
        }
    }
    Ok(())
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
        anyhow::bail!("{}{kind} needs --parent", if kind == NodeKind::Milestone { "a" } else { "a " });
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
    println!("#{id} is now {state}");
    Ok(())
}

async fn console(roadmap: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all("var")?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open("var/console.log")?;
    let store = Store::open(&roadmap, "Rhipper Agent").await?;
    if !ra_console::ui::is_terminal() {
        anyhow::bail!("the console needs a terminal");
    }
    let sink = ra_console::logs::Sink::new();
    ra_console::trace::init(log, Some(sink.clone()));
    let worker = Worker::new(ra_console::ui::DEFAULT_MODEL).with_ledger(open_ledger().await?);
    ra_console::ui::run(store, worker, sink).await
}
