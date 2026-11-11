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
#[command(name = "rc", version = ra_console::VERSION, about = "Rhipper Agent: the roadmap-driven multi-model development orchestrator")]
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
    /// Prints the first ready card: a defined task whose waits-on are all accepted.
    Next,
    /// Links a node to the nodes it waits for.
    Depend {
        /// The node that waits.
        id: i64,
        /// The nodes it waits for.
        waits_for: Vec<i64>,
    },
    /// Sets the state of a node; cancelled removes the card, since a cancelled card does not stay on the roadmap.
    Set {
        id: i64,
        /// The state: one of the twelve lifecycle names.
        state: TaskState,
    },
    /// Removes a card; every waits-on reference to it goes with it.
    Remove {
        id: i64,
        /// The card and everything under it; a card with children is refused without it.
        #[arg(long)]
        tree: bool,
    },
    /// One round trip to a model: the answer and the usage.
    Ping {
        /// The model: a claude-* id runs on Claude Code, local/<name> on the local server at
        /// RHIPPER_LOCAL_URL, vendor/model on OpenRouter.
        #[arg(long, default_value = ra_console::ui::DEFAULT_MODEL)]
        model: String,
    },
    /// The usage ledger: the calls, the tokens and the cost of every model, all time.
    Usage,
    /// Executes one task: the worker acts in the project directory and the answer lands in the node's notes.
    Run {
        /// The node id; left out with --next or --all.
        #[arg(required_unless_present_any = ["next", "all"])]
        id: Option<i64>,
        /// Runs the first ready card, the one `rc next` names.
        #[arg(long, conflicts_with_all = ["id", "all"])]
        next: bool,
        /// Runs the ready cards one after another, reading readiness again after each, until none
        /// is ready or a run fails; a card implemented by the batch counts as met for the cards
        /// that wait on it.
        #[arg(long, conflicts_with = "id")]
        all: bool,
        /// With --all: a card that needs revision stops the batch too.
        #[arg(long, requires = "all")]
        stop_on_revision: bool,
        /// The model for this run; left out, the node's own model, else the default.
        #[arg(long)]
        model: Option<String>,
        /// The most dollars the run may spend; none unless given. With budgets on (RHIPPER_BUDGETS=on, or
        /// `budgets on` in the console) the node's complexity caps a run (c1 0.25 … c5 5.00) instead.
        #[arg(long)]
        budget: Option<f64>,
        /// The most turns the run may take; none unless given.
        #[arg(long)]
        turns: Option<u32>,
        /// Out of debug mode: the worker is not asked what would have made the run faster. RHIPPER_DEBUG=off
        /// does the same everywhere.
        #[arg(long)]
        no_debug: bool,
        /// The effort the run asks of the model: low, medium or high; left out, the card's own,
        /// medium when it names none.
        #[arg(long)]
        effort: Option<ra_roadmap::model::Effort>,
        /// Prints what the run would send (the system messages, the task context, the harness
        /// command line) and calls no model; the card keeps its state and its session.
        #[arg(long)]
        dry: bool,
    },
    /// Starts the console over the orchestrator.
    Console {
        /// The roadmap file.
        #[arg(long, default_value = "rhipper.md")]
        roadmap: PathBuf,
        /// Tells a running console to quit first, and kills it when it does not.
        #[arg(long)]
        replace: bool,
    },
    /// Tells a running console to quit, without starting one.
    Stop,
    /// Serves the roadmap and the workers over MCP on stdin and stdout, for a client such as Claude Code.
    Mcp {
        /// The roadmap file.
        #[arg(long, default_value = "rhipper.md")]
        roadmap: PathBuf,
    },
    /// A Claude Code tool hook: reads the event on stdin and answers from the gotchas, with no model call.
    Hook {
        /// `pre` refuses or rewrites a command about to run; `post` adds the hint a result calls for.
        event: String,
    },
    /// The gotchas that fired, by id, and the refused commands no gotcha names yet.
    Gotchas,
    /// Makes this directory a project: drafts rhipper.md with a strong model from what the directory shows, imports a roadmap-shaped file, or starts empty.
    Init {
        /// A roadmap-shaped file (ROADMAP.md, TODO.md, a plan) the model converts, keeping every item.
        #[arg(long)]
        import: Option<PathBuf>,
        /// The bare project and nothing else, no model call.
        #[arg(long)]
        empty: bool,
        /// The model that drafts or converts.
        #[arg(long, default_value = ra_console::setup::SETUP_MODEL)]
        model: String,
    },
    /// Installs this build (rc and rc-gui) into a directory on the path, so the running instances never lock target/.
    Install {
        /// The directory; left out, %LOCALAPPDATA%\Programs\rhipper on Windows, ~/.local/bin elsewhere.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// The context a run carries: the project's (rhipper.context.md), or a task's with the outlines of its files and its handoffs.
    Context {
        /// The task; left out, the project context.
        id: Option<i64>,
        /// Drafts rhipper.context.md from the workspace when there is none; an existing file is kept.
        #[arg(long)]
        init: bool,
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
        Command::Next => {
            plain_tracing()?;
            next().await
        }
        Command::Depend { id, waits_for } => {
            plain_tracing()?;
            depend(id, &waits_for).await
        }
        Command::Set { id, state } => {
            plain_tracing()?;
            set(id, state).await
        }
        Command::Remove { id, tree } => {
            plain_tracing()?;
            let removed = open_store().await?.remove(id, tree).await?;
            println!("removed {}", removed.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(" "));
            Ok(())
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
        Command::Run { id, next, all, stop_on_revision, model, budget, turns, no_debug, effort, dry } => {
            plain_tracing()?;
            if let Some(budget) = budget {
                anyhow::ensure!(budget.is_finite() && budget > 0.0, "the budget is a positive number of dollars");
            }
            anyhow::ensure!(turns != Some(0), "the turn cap is at least one");
            anyhow::ensure!(!(dry && (next || all)), "--dry takes a node id: a dry run sends nothing and picks nothing");
            let store = open_store().await?;
            // a card named by id runs on its own model unless one is given; --next and --all run
            // on the given model, else the default
            let picked = match (model, id) {
                (Some(model), _) => model,
                (None, Some(id)) => store.read(id).await?.assignee.map_or_else(|| ra_console::ui::DEFAULT_MODEL.to_owned(), |a| a.model),
                (None, None) => ra_console::ui::DEFAULT_MODEL.to_owned(),
            };
            let mut worker =
                Worker::new(picked).with_ledger(open_ledger().await?).with_budget(budget).with_turns(turns).with_effort(effort);
            if no_debug {
                worker = worker.with_debug(false);
            }
            let lines = match id {
                Some(id) if dry => ra_console::agent::dry_run(&store, id, &worker).await?,
                Some(id) => ra_console::agent::run(&store, id, &worker).await?,
                None if all => ra_console::agent::run_all(&store, &worker, stop_on_revision).await?,
                None => ra_console::agent::run_next(&store, &worker).await?,
            };
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        Command::Stop => {
            println!("{}", ra_console::instance::stop_running().unwrap_or_else(|| "no console is running".to_owned()));
            Ok(())
        }
        Command::Mcp { roadmap } => {
            plain_tracing()?;
            ensure_roadmap(&roadmap)?;
            let store = Store::open(&roadmap, "Rhipper Agent").await?;
            let worker = Worker::new(ra_console::ui::DEFAULT_MODEL).with_ledger(open_ledger().await?);
            ra_console::mcp::serve(store, worker).await
        }
        Command::Console { roadmap, replace } => console(roadmap, replace).await,
        // a hook answers fast and stays silent on anything it cannot judge; no trace file either
        Command::Hook { event } => hook(&event),
        Command::Gotchas => gotchas(),
        Command::Init { import, empty, model } => {
            plain_tracing()?;
            let mode = match (import, empty) {
                (Some(file), _) => ra_console::setup::Mode::Import(file),
                (None, true) => ra_console::setup::Mode::Empty,
                (None, false) => ra_console::setup::Mode::Draft,
            };
            let worker = Worker::new(model).with_ledger(open_ledger().await?);
            for line in ra_console::setup::init(Path::new("."), &worker, mode).await? {
                println!("{line}");
            }
            Ok(())
        }
        Command::Install { dir } => {
            for line in ra_console::install::install(dir.as_deref())? {
                println!("{line}");
            }
            Ok(())
        }
        Command::Context { id, init } => {
            plain_tracing()?;
            context(id, init).await
        }
    }
}

/// Prints the project context, drafting it with `--init`, or a task's context as a run would
/// send it.
async fn context(id: Option<i64>, init: bool) -> anyhow::Result<()> {
    let root = Path::new(".");
    match id {
        Some(id) => {
            let store = open_store().await?;
            print!("{}", ra_console::agent::context_preview(&store, id).await?);
        }
        None => match ra_context::Project::load(root) {
            Some(text) => print!("{text}"),
            None if init => {
                print!("{}", ra_context::Project::ensure(root)?);
                eprintln!("\n{} written; edit it by hand, every run reads it", ra_context::PROJECT_FILE);
            }
            None => println!("no {}; `rc context --init` drafts one from the workspace", ra_context::PROJECT_FILE),
        },
    }
    Ok(())
}

/// Answers one Claude Code tool hook from the gotchas. `pre` prints a deny with the reason, or
/// an allow with the rewritten command; `post` prints the hints the result calls for. Anything
/// the hook cannot judge, a missing catalogue included, gets no answer at all: the run goes on.
fn hook(event: &str) -> anyhow::Result<()> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    // a byte-order mark at the start of a piped file is not part of the event
    let Ok(call) = serde_json::from_str::<serde_json::Value>(input.trim_start_matches('\u{feff}')) else { return Ok(()) };
    let cwd =
        call.get("cwd").and_then(serde_json::Value::as_str).map_or_else(|| std::env::current_dir().unwrap_or_default(), PathBuf::from);
    let Ok(catalogue) = ra_gotchas::Catalogue::load(&cwd.join("var/gotchas.toml")) else { return Ok(()) };
    // a running console or window holds its binary only while it runs from target/; an
    // installed one leaves the build free, so the lock itself is asked, not the pid file
    let locked = |name: &str| {
        let path = cwd.join("target").join("debug").join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        path.exists() && std::fs::OpenOptions::new().append(true).open(&path).is_err()
    };
    let conditions = ra_gotchas::Conditions { console_running: locked("rc") || locked("rc-gui") };
    let answer = match event {
        "pre" => {
            let command = call.pointer("/tool_input/command").and_then(serde_json::Value::as_str).unwrap_or("");
            match catalogue.command(command, conditions) {
                ra_gotchas::Verdict::Pass => None,
                ra_gotchas::Verdict::Deny { reason, .. } => Some(serde_json::json!({
                    "hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason}
                })),
                ra_gotchas::Verdict::Rewrite { command, reason, .. } => Some(serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "permissionDecisionReason": reason,
                        "updatedInput": {"command": command}
                    }
                })),
            }
        }
        "post" => {
            // what a shell said is read one way, what an edit tool said another
            let place = match call.get("tool_name").and_then(serde_json::Value::as_str) {
                Some("Edit" | "Write" | "MultiEdit" | "NotebookEdit") => ra_gotchas::Where::Edit,
                _ => ra_gotchas::Where::Result,
            };
            let response = call.get("tool_response").cloned().unwrap_or_default();
            let text = match &response {
                serde_json::Value::String(s) => s.clone(),
                other => ["stdout", "stderr"]
                    .iter()
                    .filter_map(|k| other.get(k).and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            let hits = catalogue.scan(place, &text);
            (!hits.is_empty()).then(|| {
                let context = hits.iter().map(|h| h.message.as_str()).collect::<Vec<_>>().join("; ");
                serde_json::json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": context}})
            })
        }
        _ => None,
    };
    if let Some(answer) = answer {
        println!("{answer}");
    }
    Ok(())
}

/// Prints the gotchas by how often they fired, and the refused commands no gotcha names yet.
fn gotchas() -> anyhow::Result<()> {
    let catalogue = ra_gotchas::Catalogue::load(Path::new("var/gotchas.toml"))?;
    let journal = ra_gotchas::Journal::at(Path::new("var/gotchas.log"));
    let counts = journal.counts();
    if counts.is_empty() {
        println!("no gotcha has fired yet; the catalogue holds {} entries in var/gotchas.toml", catalogue.entries.len());
        return Ok(());
    }
    println!("{:<26} {:>5}  message", "gotcha", "hits");
    for (id, count) in &counts {
        let message = catalogue.get(id).map_or("a refused command no gotcha names; see below", |g| g.message.as_str());
        println!("{id:<26} {count:>5}  {message}");
    }
    let unmatched = journal.unmatched();
    if !unmatched.is_empty() {
        println!("\nrefused commands no gotcha names, newest last:");
        for command in unmatched.iter().rev().take(20).rev() {
            println!("  {command}");
        }
    }
    Ok(())
}

/// Opens the usage ledger under `var/`.
async fn open_ledger() -> anyhow::Result<UsageStore> {
    std::fs::create_dir_all("var")?;
    UsageStore::open(Path::new("var/usage.db")).await
}

/// Prints the ledger by model, the costliest first, and the total.
async fn usage() -> anyhow::Result<()> {
    // the plan's windows first: the room left matters more than the dollars already spent
    let limits = ra_providers::limits::Limits::load(Path::new(ra_providers::claude_code::LIMITS_FILE));
    if limits.fresh() {
        println!("Claude plan ({}): {}", limits.status, limits.summary().join(" · "));
    } else {
        println!("Claude plan: no fresh report; a Claude Code run brings one");
    }
    println!();
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
    // the progress of a worker goes to the activity file, which a running console tails
    let activity = std::fs::OpenOptions::new().create(true).append(true).open("var/activity.log")?;
    ra_console::trace::init(log, None, Some(activity));
    Ok(())
}

async fn open_store() -> anyhow::Result<Store> {
    ensure_roadmap(&store_path())?;
    Store::open(&store_path(), "Rhipper Agent").await
}

/// A command over a roadmap needs one: a directory without it is told what `rc init` does,
/// rather than getting a bare file on the quiet.
fn ensure_roadmap(roadmap: &Path) -> anyhow::Result<()> {
    if roadmap.is_file() {
        return Ok(());
    }
    let root = roadmap.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    anyhow::bail!("{}", ra_console::setup::hint(root).join("\n"))
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
    let states = tree::effective_states(&nodes);
    for node in picked {
        let indent = "  ".repeat(tree::depth(&nodes, node.id));
        let assignee = match &node.assignee {
            Some(a) => format!(" ← {}:{}", a.role, a.model),
            None => String::new(),
        };
        let mark = if node.is_final() { " [final]" } else { "" };
        println!("{}#{} {} [{}]{mark}{assignee}", indent, node.id, node.title, state_label(states[&node.id], node.state));
        if let Some(objective) = &node.objective {
            println!("{indent}    {objective}");
        }
        if !node.files.is_empty() {
            println!("{indent}    files: {}", node.files.join(" "));
        }
    }
    Ok(())
}

/// The computed state with the stored one when they differ, `ready (defined)`, else the one.
fn state_label(computed: TaskState, stored: TaskState) -> String {
    if computed == stored { stored.to_string() } else { format!("{computed} ({stored})") }
}

async fn next() -> anyhow::Result<()> {
    let nodes = open_store().await?.all().await?;
    match tree::next_ready(&nodes) {
        Some(node) => {
            println!("#{} {}", node.id, node.title);
            if let Some(objective) = &node.objective {
                println!("    {objective}");
            }
        }
        None => println!("no card is ready"),
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
    if state == TaskState::Cancelled {
        println!("#{id} removed: a cancelled card does not stay on the roadmap");
    } else {
        println!("#{id} is now {state}");
    }
    Ok(())
}

async fn console(roadmap: PathBuf, replace: bool) -> anyhow::Result<()> {
    std::fs::create_dir_all("var")?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open("var/console.log")?;
    ensure_roadmap(&roadmap)?;
    let store = Store::open(&roadmap, "Rhipper Agent").await?;
    if !ra_console::ui::is_terminal() {
        anyhow::bail!("the console needs a terminal");
    }
    let replaced = if replace { ra_console::instance::stop_running() } else { None };
    ra_console::instance::claim()?;
    let sink = ra_console::logs::Sink::new();
    ra_console::trace::init(log, Some(sink.clone()), None);
    if let Some(what) = replaced {
        tracing::info!("{what}");
    }
    let worker = Worker::new(ra_console::ui::DEFAULT_MODEL).with_ledger(open_ledger().await?);
    let result = ra_console::ui::run(store, worker, sink).await;
    ra_console::instance::release();
    result
}
