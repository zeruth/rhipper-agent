//! Rhipper Agent: the roadmap-driven multi-model development orchestrator. The binary parses the
//! command line and hands over to the subcommands; the work lives in the crates.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use ra_console::agent::Worker;
use ra_roadmap::history::Source;
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::{Card, Edit, Store};
use ra_roadmap::{history, markdown, tree};
use ra_usage::UsageStore;

#[cfg(test)]
mod tests;

mod doctor;
mod status;

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
        /// Prints a plain Markdown checklist instead, for a README or an issue.
        #[arg(long)]
        markdown: bool,
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
        /// The workspace-relative files the worker reads first.
        #[arg(long, num_args = 1..)]
        files: Vec<String>,
        /// The cards this one waits on.
        #[arg(long, num_args = 1..)]
        waits_on: Vec<i64>,
        /// The model that runs the card: a claude-* id on Claude Code, local/<name> on the local
        /// server, vendor/model on OpenRouter; left out, the default route.
        #[arg(long)]
        model: Option<String>,
    },
    /// Rewrites the named fields of a card and nothing else: the notes, the state and the session stay.
    Edit {
        id: i64,
        /// The title, one line.
        #[arg(long)]
        title: Option<String>,
        /// The objective.
        #[arg(long)]
        objective: Option<String>,
        /// The acceptance criteria.
        #[arg(long)]
        acceptance: Option<String>,
        /// Complexity 1..=5.
        #[arg(long)]
        complexity: Option<u8>,
        /// The workspace-relative files the worker reads first; replaces the list.
        #[arg(long, num_args = 1..)]
        files: Option<Vec<String>>,
        /// Forgets the harness session, so the next run starts fresh instead of resuming.
        #[arg(long)]
        clear_session: bool,
    },
    /// Prints the ready card the walk takes next: by priority, then the active gambits whose
    /// focus matches it, then the order of the file.
    Next {
        /// Prints every ready card in that order, with its score and the gambits that moved it.
        #[arg(long)]
        why: bool,
    },
    /// Lints the roadmap: placeholder text, a card without acceptance or files, a waits-on that names
    /// no card or comes back to its own, a complexity outside 1..5; one line each, exit code 1 when any.
    Check,
    /// Lists the gambits: the standing directions that order the walk, each with its weight, its state and its focus.
    Gambits,
    /// Adds a gambit or changes one: its state, its weight or the words a card matches on.
    Gambit {
        #[command(subcommand)]
        action: GambitAction,
    },
    /// Sets the person's priority on a card, 1 to 5, which the walk takes first; `--clear` leaves the order to the file and the gambits.
    Priority {
        id: i64,
        /// The priority, 1 to 5.
        #[arg(required_unless_present = "clear", conflicts_with = "clear")]
        priority: Option<u8>,
        /// Clears the priority.
        #[arg(long)]
        clear: bool,
    },
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
    /// The state changes so far, oldest first: when, the card, from what to what, and by whom
    /// (console, window, cli, mcp or run); of one card with an id.
    History {
        /// The card; left out, every card.
        id: Option<i64>,
    },
    /// Removes a card; every waits-on reference to it goes with it.
    Remove {
        id: i64,
        /// The card and everything under it; a card with children is refused without it.
        #[arg(long)]
        tree: bool,
    },
    /// Moves a card with everything under it to another parent; never under itself.
    Move {
        id: i64,
        /// The new parent.
        #[arg(long)]
        under: i64,
        /// The sibling under the new parent it goes after; last when left out.
        #[arg(long)]
        after: Option<i64>,
    },
    /// Adds a plain Markdown outline under a node: `##` headings as milestones, `###` as sections, bullets as tasks,
    /// an indented line as the objective, `accept:` (or `acceptance:`) as the acceptance; no model call.
    Import {
        /// The outline file.
        file: PathBuf,
        /// The node the outline goes under: the project for milestones, a milestone for sections and tasks.
        #[arg(long)]
        under: i64,
        /// Prints what it would add and adds nothing.
        #[arg(long)]
        dry: bool,
    },
    /// Checks what a run needs, one line each: `claude` on the path and logged in, the hooks, the settings a
    /// worker loads, git, the local server, the OpenRouter key, the roadmap parsing, the ledger opening; exit
    /// code 1 when a required one fails.
    Doctor,
    /// One round trip to a model: the answer and the usage.
    Ping {
        /// The model: a claude-* id runs on Claude Code, local/<name> on the local server at
        /// RHIPPER_LOCAL_URL, vendor/model on OpenRouter.
        #[arg(long, default_value = ra_console::ui::DEFAULT_MODEL)]
        model: String,
    },
    /// The usage ledger: the calls, the tokens and the cost of every model, all time.
    Usage,
    /// This project's state in one line: the live run (orphaned when its process is gone), the ready count, today's calls and the last end.
    Status {
        /// One line per project the hub remembers (the window's tabs, the digests), this one first.
        #[arg(long)]
        all: bool,
    },
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
        /// The model for this run; left out, each card's own model, else the rule of rhipper.toml
        /// that fits it, else the default.
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
        /// Runs the card in var/worktrees/<id> on a branch card/<id> made from HEAD; the handoff
        /// names the branch, and merging it is yours (`rc worktree list|drop`).
        #[arg(long, conflicts_with_all = ["dry", "next", "all"])]
        worktree: bool,
        /// With --all: runs that many ready cards at once, each in its own worktree.
        #[arg(long, requires = "all", value_name = "N")]
        parallel: Option<usize>,
        /// Spends the plan past its week threshold (90%, or `[plan] threshold` of rhipper.toml):
        /// without it a run on a claude-* model falls to the local route when one is up and is
        /// refused with the room left and the reset otherwise.
        #[arg(long)]
        force: bool,
    },
    /// The worktrees the runs made under var/worktrees, one per card on its branch card/<id>.
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },
    /// Has a second model review the card: it reads the acceptance, the last handoff and the git diff
    /// of the card's files, and writes its verdict into the notes: accepted, or needs revision with
    /// the points first. The reviewer is the `reviewer` rule of rhipper.toml that fits the card, else
    /// the default model.
    Review {
        /// The node id.
        id: i64,
    },
    /// Without an id, plans the next cards from the evidence of the runs themselves: the ledger's costliest
    /// cards, the cards run more than once, the look-backs, the gotcha journal and the other projects'
    /// digests. Each card that cites a line and names a saving and a check lands under the last open
    /// milestone with its evidence and saving, one run each under the turn cap; the next day's walk reads
    /// the saving back into var/retro.log. With an id, asks the architect model to split that milestone or
    /// section into cards, shows them, and adds them on confirmation.
    Plan {
        /// The milestone or section to split; left out, the plan comes from the evidence.
        id: Option<i64>,
        /// Adds the cards of a split without asking.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Shows the evidence plan and adds nothing.
        #[arg(long, conflicts_with = "id")]
        dry: bool,
        /// The milestone or section the evidence plan's cards go under; the last open milestone without it.
        #[arg(long, conflicts_with = "id")]
        under: Option<i64>,
    },
    /// Prints the model a run of the card would take and why: the card's own, the `[[route]]` rule of
    /// rhipper.toml that fits its complexity and kind, or the default. No model call.
    Route {
        /// The node id.
        id: i64,
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
    /// The link to a paired session: the latest events of var/pair.log (a run's start and end, a walk's end, a note), which a pairing session watches.
    Pair {
        /// Prints the events after this unix time instead of the last twenty, stamped, so a watch that expired catches up.
        #[arg(long)]
        since: Option<u64>,
        /// Keeps printing each new event as it lands, flushed, following the file by name across a rotation, until killed.
        #[arg(long)]
        follow: bool,
    },
    /// Opens a project in the window: as a tab of the one running, or in a new window from the install directory.
    Open {
        /// The project directory; this one when left out.
        dir: Option<PathBuf>,
    },
    /// Quits the window and starts the installed one again on the same tabs: how a new build comes up after `rc install`.
    Restart,
    /// Starts the walk (`rc run --all`) as a process of its own: through the window that has the project, else detached from here. `--stop` ends a walk after its current card.
    Walk {
        /// The project directory; this one when left out.
        dir: Option<PathBuf>,
        /// Ends the running walk after the card it is in.
        #[arg(long)]
        stop: bool,
        /// Runs that many ready cards at once, each in its own worktree; one when left out.
        #[arg(long)]
        parallel: Option<u32>,
    },
    /// An observation for the next runs: what slowed one, a quirk of the machine, a direction. The workers read the latest with their own look-backs.
    Note {
        /// The note, one line.
        text: Vec<String>,
    },
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
        /// Also puts a shortcut to the window on the desktop when there is none, opening this project.
        #[arg(long)]
        desktop: bool,
    },
    /// The context a run carries: the project's (rhipper.context.md), or a task's with the outlines of its files and its handoffs.
    Context {
        /// The task; left out, the project context.
        id: Option<i64>,
        /// Drafts rhipper.context.md from the workspace when there is none; an existing file is kept.
        #[arg(long)]
        init: bool,
        /// The evidence block the milestone planner reads: the look-backs, the shared notes, the ledger's costliest cards, the other projects' digests.
        #[arg(long, conflicts_with_all = ["id", "init"])]
        plan: bool,
    },
}

/// What `rc gambit` does with a gambit; the id is `g2` or `2`.
#[derive(Subcommand)]
enum GambitAction {
    /// Adds a gambit, active.
    Add {
        /// The text, one line.
        text: Vec<String>,
        /// The weight, 1 to 5.
        #[arg(long, default_value_t = 3)]
        weight: u8,
        /// The words a card matches on, in its title, its milestone's title or its files; none matches every card.
        #[arg(long, num_args = 1..)]
        focus: Vec<String>,
    },
    /// Parks a gambit: it waits and steers nothing.
    Park { id: String },
    /// Makes a gambit active again.
    Activate { id: String },
    /// Marks a gambit done; it stays in the file as a record.
    Done { id: String },
    /// Sets the weight of a gambit, 1 to 5.
    Weight { id: String, weight: u8 },
    /// Replaces the focus words of a gambit; none makes it match every card.
    Focus { id: String, words: Vec<String> },
}

/// What `rc worktree` does with the worktrees of the cards.
#[derive(Subcommand)]
enum WorktreeAction {
    /// Lists them: the card, the branch and the path.
    List,
    /// Removes the worktree of a card, changes and all; its branch goes too when it is merged and stays otherwise.
    Drop {
        /// The node id.
        id: i64,
    },
}

fn main() -> anyhow::Result<()> {
    // the commands' futures are one state machine, and in a debug build it outgrew the main
    // thread's stack (one megabyte on Windows) once the walk, the review, the plan and the
    // parallel runs were in it: every command overflowed before it began. The work runs on a
    // thread with room.
    let worker = std::thread::Builder::new().name("rc".to_owned()).stack_size(256 << 20).spawn(|| {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
        runtime.block_on(run_main())
    })?;
    worker.join().map_err(|_| anyhow::anyhow!("the command's thread panicked"))?
}

async fn run_main() -> anyhow::Result<()> {
    let options = Options::parse();
    match options.command {
        Command::Show { id, markdown } => {
            plain_tracing()?;
            if markdown { show_markdown(id).await } else { show(id).await }
        }
        Command::Add { kind, title, parent, objective, acceptance, complexity, files, waits_on, model } => {
            plain_tracing()?;
            let card = Card {
                objective: objective.as_deref(),
                acceptance: acceptance.as_deref(),
                complexity,
                files: &files,
                waits_on: &waits_on,
                model: model.as_deref(),
                ..Card::new()
            };
            add(kind, &title, parent, &card).await
        }
        Command::Edit { id, title, objective, acceptance, complexity, files, clear_session } => {
            plain_tracing()?;
            // a path is stored with forward slashes, so the file reads the same on every platform
            let files: Option<Vec<String>> = files.map(|files| files.iter().map(|p| p.replace('\\', "/")).collect());
            let edit = Edit {
                title: title.as_deref(),
                objective: objective.as_deref(),
                acceptance: acceptance.as_deref(),
                complexity,
                files: files.as_deref(),
                clear_session,
            };
            edit_card(id, &edit).await
        }
        Command::Next { why } => {
            plain_tracing()?;
            next(why).await
        }
        Command::Check => {
            plain_tracing()?;
            check().await
        }
        Command::Gambits => {
            plain_tracing()?;
            for line in ra_console::commands::gambit_lines(&open_store().await?.gambits().await?) {
                println!("{line}");
            }
            Ok(())
        }
        // the console's line parser is the one parser of the gambit commands; the CLI only
        // shapes the words
        Command::Gambit { action } => {
            plain_tracing()?;
            let line = match action {
                GambitAction::Add { text, weight, focus } => {
                    let text = text.join(" ");
                    anyhow::ensure!(!text.trim().is_empty(), "a gambit is not empty");
                    let store = open_store().await?;
                    let id = store.add_gambit(&text, weight, &focus).await?;
                    println!("g{id} {} {{w{weight}}} [active]", text.trim());
                    if !focus.is_empty() {
                        println!("    focus: {}", focus.join(" "));
                    }
                    return Ok(());
                }
                GambitAction::Park { id } => format!("/gambit {id} park"),
                GambitAction::Activate { id } => format!("/gambit {id} activate"),
                GambitAction::Done { id } => format!("/gambit {id} done"),
                GambitAction::Weight { id, weight } => format!("/gambit {id} weight {weight}"),
                GambitAction::Focus { id, words } => format!("/gambit {id} focus {}", words.join(" ")),
            };
            console_line(&line).await
        }
        Command::Priority { id, priority, clear } => {
            plain_tracing()?;
            let value = match priority {
                Some(p) if !clear => p.to_string(),
                _ => "none".to_owned(),
            };
            console_line(&format!("/priority {id} {value}")).await
        }
        Command::Depend { id, waits_for } => {
            plain_tracing()?;
            depend(id, &waits_for).await
        }
        Command::Set { id, state } => {
            plain_tracing()?;
            set(id, state).await
        }
        Command::History { id } => {
            plain_tracing()?;
            show_history(id).await
        }
        Command::Remove { id, tree } => {
            plain_tracing()?;
            let removed = open_store().await?.remove(id, tree).await?;
            println!("removed {}", removed.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(" "));
            Ok(())
        }
        Command::Move { id, under, after } => {
            plain_tracing()?;
            open_store().await?.move_node(id, under, after).await?;
            match after {
                Some(after) => println!("#{id} moved under #{under}, after #{after}"),
                None => println!("#{id} moved under #{under}"),
            }
            Ok(())
        }
        Command::Import { file, under, dry } => {
            plain_tracing()?;
            import(&file, under, dry).await
        }
        Command::Doctor => {
            plain_tracing()?;
            doctor::run(Path::new(".")).await
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
        // a read of files only, no trace: it runs from any directory, a project or not
        Command::Status { all } => {
            use ra_console::hub::Status;
            let rows =
                if all { ra_console::hub::statuses(Path::new(".")).await } else { vec![ra_console::hub::status(Path::new(".")).await] };
            println!("{}", Status::header());
            for row in &rows {
                println!("{}", row.line());
                // with --all, the cards each project has in hand go under its row, costed
                if all {
                    for line in status::card_lines_of(Path::new(&row.path)).await {
                        println!("{line}");
                    }
                }
            }
            Ok(())
        }
        Command::Run { id, next, all, stop_on_revision, model, budget, turns, no_debug, effort, dry, worktree, parallel, force } => {
            plain_tracing()?;
            if let Some(budget) = budget {
                anyhow::ensure!(budget.is_finite() && budget > 0.0, "the budget is a positive number of dollars");
            }
            anyhow::ensure!(turns != Some(0), "the turn cap is at least one");
            anyhow::ensure!(!(dry && (next || all)), "--dry takes a node id: a dry run sends nothing and picks nothing");
            let store = open_store().await?;
            // a given model pins every run of the command; without one each card picks its own,
            // through the rules of rhipper.toml, with the default last
            let worker = match model {
                Some(model) => Worker::new(model),
                None => Worker::new(ra_console::ui::DEFAULT_MODEL).with_routing(true),
            };
            let mut worker =
                worker.with_ledger(open_ledger().await?).with_budget(budget).with_turns(turns).with_effort(effort).with_force(force);
            if no_debug {
                worker = worker.with_debug(false);
            }
            let lines = match id {
                Some(id) if dry => ra_console::agent::dry_run(&store, id, &worker).await?,
                Some(id) if worktree => ra_console::agent::run_in_worktree(&store, id, &worker).await?,
                Some(id) => ra_console::agent::run(&store, id, &worker).await?,
                None if all => match parallel {
                    Some(width) => ra_console::agent::run_parallel(&store, &worker, width, stop_on_revision).await?,
                    None => ra_console::agent::run_all(&store, &worker, stop_on_revision).await?,
                },
                None => ra_console::agent::run_next(&store, &worker).await?,
            };
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        Command::Worktree { action } => {
            let store = open_store().await?;
            let root = ra_console::agent::workspace_of(&store);
            let lines = match action {
                WorktreeAction::List => ra_console::agent::worktree_list(&root)?,
                WorktreeAction::Drop { id } => ra_console::agent::worktree_drop(&root, id).await?,
            };
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        Command::Review { id } => {
            plain_tracing()?;
            let store = open_store().await?;
            let worker = Worker::new(ra_console::ui::DEFAULT_MODEL).with_ledger(open_ledger().await?);
            for line in ra_console::agent::review(&store, id, &worker).await? {
                println!("{line}");
            }
            Ok(())
        }
        Command::Plan { id, yes, dry, under } => {
            plain_tracing()?;
            let store = open_store().await?;
            let worker = Worker::new(ra_console::ui::DEFAULT_MODEL).with_ledger(open_ledger().await?);
            if let Some(id) = id {
                return plan(&store, id, &worker, yes).await;
            }
            for line in ra_console::commands::plan::plan(&store, &worker, under, dry).await? {
                println!("{line}");
            }
            Ok(())
        }
        Command::Route { id } => route(id).await,
        Command::Stop => {
            println!("{}", ra_console::instance::stop_running(Path::new(".")).unwrap_or_else(|| "no console is running".to_owned()));
            Ok(())
        }
        Command::Mcp { roadmap } => {
            plain_tracing()?;
            ensure_roadmap(&roadmap)?;
            let store = Store::open(&roadmap, "Rhipper Agent").await?.with_source(Source::Mcp);
            let worker = Worker::new(ra_console::ui::DEFAULT_MODEL).with_ledger(open_ledger().await?);
            ra_console::mcp::serve(store, worker).await
        }
        Command::Console { roadmap, replace } => console(roadmap, replace).await,
        // a hook answers fast and stays silent on anything it cannot judge; no trace file either
        Command::Hook { event } => hook(&event),
        Command::Gotchas => gotchas(),
        Command::Open { dir } => {
            let dir = dir.unwrap_or_else(|| PathBuf::from("."));
            println!("{}", ra_console::hub::open(&dir)?);
            Ok(())
        }
        Command::Restart => {
            println!("{}", ra_console::hub::restart()?);
            Ok(())
        }
        Command::Walk { dir, stop, parallel } => {
            let dir = dir.unwrap_or_else(|| PathBuf::from("."));
            let dir = std::fs::canonicalize(&dir).map_or(dir, |p| PathBuf::from(p.display().to_string().trim_start_matches(r"\\?\")));
            if stop {
                println!("{}", ra_console::agent::stop_walk(&dir));
            } else if ra_console::agent::walk_running(&dir) {
                println!("a walk is already running in {}; `rc walk --stop` ends it after its current card", dir.display());
            } else if ra_console::instance::claimed(&dir) && ra_console::hub::window_running() {
                // the window has the project: it starts the walk, so the tab shows it from its
                // first line and the walk is nobody's child but the system's
                println!("{}", ra_console::agent::request_walk(&dir, parallel));
            } else {
                let pid = ra_console::agent::spawn_walk(&dir, true, parallel)?;
                println!("walk started in {} (pid {pid}); `rc pair` and var/activity.log follow it", dir.display());
            }
            Ok(())
        }
        Command::Pair { since, follow } => {
            if follow {
                let mut out = std::io::stdout().lock();
                ra_console::pair::follow(Path::new("."), since.unwrap_or(0), &mut out)?;
                return Ok(());
            }
            if let Some(since) = since {
                for event in ra_console::pair::since(Path::new("."), since) {
                    println!("{event}");
                }
                return Ok(());
            }
            let events = ra_console::pair::recent(Path::new("."), 20);
            if events.is_empty() {
                println!("no events yet in {}; a run writes its start and its end there", ra_console::pair::PAIR_FILE);
            }
            for event in events {
                println!("{event}");
            }
            println!("\na pairing session follows it with `rc pair --follow --since <last seen>` and is told of each line");
            Ok(())
        }
        Command::Note { text } => {
            ra_console::retro::record_pair(Path::new("."), &text.join(" "))?;
            println!("noted; the next runs read it with the workers' own look-backs");
            Ok(())
        }
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
        Command::Install { dir, desktop } => {
            for line in ra_console::install::install(dir.as_deref())? {
                println!("{line}");
            }
            if desktop {
                let dir = dir.unwrap_or_else(ra_console::install::default_dir);
                println!("{}", ra_console::install::desktop_shortcut(&dir, Path::new(".")));
            }
            Ok(())
        }
        Command::Context { id, init, plan } => {
            plain_tracing()?;
            if plan {
                return plan_context().await;
            }
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

/// Prints the evidence block the milestone planner reads, as `rc plan <id>` sends it.
async fn plan_context() -> anyhow::Result<()> {
    let worker = Worker::new(ra_console::ui::DEFAULT_MODEL).with_ledger(open_ledger().await?);
    let block = ra_console::agent::plan_evidence(Path::new("."), &worker).await;
    if block.is_empty() {
        println!("no evidence yet: no look-back, shared note, ledger row or other project's digest");
    } else {
        print!("{block}");
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
    // a file the worker read is not the machine talking: no gotcha reads it, but a whole long
    // file read at once gets the outline instead, catalogue or not
    if call.get("tool_name").and_then(serde_json::Value::as_str) == Some("Read") {
        if event == "post"
            && let Some(hint) = ra_console::agent::read_hint(&cwd, &call)
        {
            println!("{}", serde_json::json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": hint}}));
        }
        return Ok(());
    }
    // the seed's new entries reach a project's catalogue here too: a worker started from an
    // installed rc, not the console, has only the hook to bring them
    let Ok((catalogue, _)) = ra_gotchas::Catalogue::refresh(&cwd.join("var/gotchas.toml")) else { return Ok(()) };
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
            // a command the project's `[worker] allow` names is the project's own: the shell
            // allowlist gotcha stands aside for it, the other gotchas do not
            let own = ra_console::router::Router::load(&cwd).is_ok_and(|r| r.worker.allows(command));
            let verdict = match catalogue.command(command, conditions) {
                ra_gotchas::Verdict::Deny { id, .. } | ra_gotchas::Verdict::Rewrite { id, .. }
                    if own && id == "shell-outside-allowlist" =>
                {
                    ra_gotchas::Verdict::Pass
                }
                other => other,
            };
            match verdict {
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
    let later = std::fs::read_to_string("var/gotchas.toml").map_or(0, |text| ra_gotchas::Catalogue::from_seed_later(&text));
    let seeded = match later {
        0 => String::new(),
        n => format!(", {n} of them from the seed since the file was written"),
    };
    let journal = ra_gotchas::Journal::at(Path::new("var/gotchas.log"));
    let counts = journal.counts();
    if counts.is_empty() {
        println!("no gotcha has fired yet; the catalogue holds {} entries in var/gotchas.toml{seeded}", catalogue.entries.len());
        return Ok(());
    }
    println!("the catalogue holds {} entries in var/gotchas.toml{seeded}", catalogue.entries.len());
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

/// Prints the ledger by model and role, the costliest first, and the total: a cheap role's
/// calls (`local/qwen3-8b · summary`) listed beside the worker's.
async fn usage() -> anyhow::Result<()> {
    // the plan's windows first: the room left matters more than the dollars already spent
    let limits = ra_providers::limits::Limits::load(Path::new(ra_providers::claude_code::LIMITS_FILE));
    if limits.fresh() {
        println!("Claude plan ({}): {}", limits.status, limits.summary().join(" · "));
        // the threshold the runs steer at, so the line says how much of the week is still a run's
        let threshold = ra_console::agent::plan_threshold(Path::new("."));
        match limits.week_past(threshold) {
            Some(_) => println!("week past the {:.0}% threshold: claude-* runs go local or need --force", threshold * 100.0),
            None => println!("claude-* runs steer at {:.0}% of the week", threshold * 100.0),
        }
    } else {
        println!("Claude plan: no fresh report; a Claude Code run brings one");
    }
    println!();
    let ledger = open_ledger().await?;
    let rows = ledger.by_role().await?;
    if rows.is_empty() {
        println!("the ledger is empty: no completion has run yet");
        return Ok(());
    }
    let cost = |c: Option<f64>| c.map_or("—".to_owned(), |c| format!("${c:.4}"));
    println!("{:<34} {:>6} {:>10} {:>10} {:>10} {:>10}", "model", "calls", "in", "out", "cached", "cost");
    for (model, role, t) in &rows {
        let model = match role {
            Some(role) => format!("{model} · {role}"),
            None => model.clone(),
        };
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

/// Runs one console line over the roadmap and prints what it says; for the commands that call
/// no model, so the worker is bare.
async fn console_line(line: &str) -> anyhow::Result<()> {
    let store = open_store().await?;
    let worker = Worker::new(ra_console::ui::DEFAULT_MODEL);
    for line in ra_console::commands::run(line, &store, &worker).await?.0 {
        println!("{line}");
    }
    Ok(())
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
    let roadmap = store.load().await?;
    let nodes = roadmap.nodes;
    // the gambits come first, as in the file: the direction before the work
    if id.is_none() && !roadmap.gambits.is_empty() {
        for gambit in &roadmap.gambits {
            for line in ra_console::commands::gambit_lines(std::slice::from_ref(gambit)) {
                println!("{line}");
            }
        }
        println!();
    }
    let root = store_path().parent().filter(|p| !p.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    for line in show_lines(&nodes, id, &root)? {
        println!("{line}");
    }
    Ok(())
}

/// The lines `rc show` prints for the roadmap, or the subtree under `id`: each node with its
/// state, then its objective, files and dependencies indented under it. A node asked for by id
/// also shows the reviewer's verdict on its last run, from its context under `root`, so
/// acceptance is one glance; the whole roadmap does not open a file per card.
fn show_lines(nodes: &[ra_roadmap::model::Node], id: Option<i64>, root: &Path) -> anyhow::Result<Vec<String>> {
    let picked: Vec<_> = match id {
        Some(id) => {
            let subtree = tree::subtree(nodes, id);
            if subtree.is_empty() {
                anyhow::bail!("no node {id}");
            }
            subtree
        }
        None => nodes.iter().collect(),
    };
    let states = tree::effective_states(nodes);
    let mut lines = Vec::new();
    for node in picked {
        let indent = "  ".repeat(tree::depth(nodes, node.id));
        let assignee = match &node.assignee {
            Some(a) => format!(" ← {}:{}", a.role, a.model),
            None => String::new(),
        };
        let mark = if node.is_final() { " [final]" } else { "" };
        let priority = node.priority.map(|p| format!(" {{p{p}}}")).unwrap_or_default();
        lines.push(format!(
            "{}#{} {}{priority} [{}]{mark}{assignee}",
            indent,
            node.id,
            node.title,
            state_label(states[&node.id], node.state)
        ));
        if id.is_some()
            && let Some(verdict) = last_verdict(root, node.id)
        {
            lines.push(format!("{indent}    verdict: {verdict}"));
        }
        if let Some(objective) = &node.objective {
            lines.push(format!("{indent}    {objective}"));
        }
        if !node.files.is_empty() {
            lines.push(format!("{indent}    files: {}", node.files.join(" ")));
        }
        if !node.depends_on.is_empty() {
            lines
                .push(format!("{indent}    waits on: {}", node.depends_on.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(" ")));
        }
    }
    Ok(lines)
}

/// The first line of the reviewer's verdict on the last run of `id`, from its context file
/// under `root`; `None` for a card never run or never judged.
fn last_verdict(root: &Path, id: i64) -> Option<String> {
    let handoff = ra_context::TaskContext::open(root, id).handoffs.pop()?;
    handoff.verdict.as_deref()?.lines().next().map(|line| line.trim().to_owned())
}

/// Prints the roadmap, or the subtree under `id`, as a Markdown checklist.
async fn show_markdown(id: Option<i64>) -> anyhow::Result<()> {
    let store = open_store().await?;
    let nodes = store.load().await?.nodes;
    if let Some(id) = id
        && !nodes.iter().any(|n| n.id == id)
    {
        anyhow::bail!("no node {id}");
    }
    print!("{}", markdown::checklist(&nodes, id));
    Ok(())
}

/// The computed state with the stored one when they differ, `ready (defined)`, else the one.
fn state_label(computed: TaskState, stored: TaskState) -> String {
    if computed == stored { stored.to_string() } else { format!("{computed} ({stored})") }
}

/// Prints the card the walk takes next; with `why`, every ready card in the walk's order with
/// its score, the picked one first.
async fn next(why: bool) -> anyhow::Result<()> {
    let roadmap = open_store().await?.load().await?;
    let ranked = tree::ranked(&roadmap.nodes, &roadmap.gambits);
    if ranked.is_empty() {
        println!("no card is ready");
    } else if why {
        for score in &ranked {
            println!("{}", score.line());
        }
    } else {
        let node = ranked[0].node;
        println!("#{} {}", node.id, node.title);
        if let Some(objective) = &node.objective {
            println!("    {objective}");
        }
    }
    Ok(())
}

/// Prints the model a run of the card would take, the effort when a rule sets one, and the
/// reason: the card's own model, the rule that fits, or the default with no rule to name.
async fn route(id: i64) -> anyhow::Result<()> {
    use ra_console::router::{IMPLEMENTER, ROUTES_FILE, Router, Why};
    let store = open_store().await?;
    let node = store.read(id).await?;
    let root = ra_console::agent::workspace_of(&store);
    let router = Router::load(&root)?;
    let choice = router.choose(&node, IMPLEMENTER, ra_console::ui::DEFAULT_MODEL);
    let effort = choice.effort.map_or_else(String::new, |effort| format!(" (effort {effort})"));
    println!("#{} {} → {}{effort}", node.id, node.title, choice.model);
    let reason = match choice.why {
        Why::Card => "the card's own model".to_owned(),
        Why::Rule(place) => format!("rule {place} of {ROUTES_FILE}: {}", router.rules[place - 1]),
        Why::Default if router.rules.is_empty() => format!("the default: no {ROUTES_FILE} beside the roadmap"),
        Why::Default => {
            format!("the default: none of the {} rules of {ROUTES_FILE} fits c{} {}", router.rules.len(), node.complexity, node.kind)
        }
    };
    println!("    {reason}");
    Ok(())
}

/// Prints every fault the lint finds and exits 1 when there is one, so a script or a hook can
/// gate on a clean roadmap.
async fn check() -> anyhow::Result<()> {
    let nodes = open_store().await?.all().await?;
    let faults = tree::lint(&nodes);
    if faults.is_empty() {
        println!("the roadmap is clean");
        return Ok(());
    }
    for line in &faults {
        println!("{line}");
    }
    let count = faults.len();
    eprintln!("{count} fault{} on the roadmap", if count == 1 { "" } else { "s" });
    std::process::exit(1)
}

async fn add(kind: NodeKind, title: &str, parent: Option<i64>, card: &Card<'_>) -> anyhow::Result<()> {
    if card.complexity == 0 || card.complexity > 5 {
        anyhow::bail!("complexity is 1 to 5");
    }
    let store = open_store().await?;
    if kind == NodeKind::Project && parent.is_some() {
        anyhow::bail!("a project has no parent");
    }
    if kind != NodeKind::Project && parent.is_none() {
        anyhow::bail!("{}{kind} needs --parent", if kind == NodeKind::Milestone { "a" } else { "a " });
    }
    let id = store.insert_card(parent, kind, title, card).await?;
    println!("#{id} {title}");
    if !card.files.is_empty() {
        println!("    files: {}", card.files.join(" "));
    }
    if !card.waits_on.is_empty() {
        println!("    waits on: {}", card.waits_on.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(" "));
    }
    Ok(())
}

/// Reads a plain outline and adds it under `under`; with `dry`, prints the items as they would
/// nest and touches nothing.
async fn import(file: &Path, under: i64, dry: bool) -> anyhow::Result<()> {
    use anyhow::Context;
    let text = std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let outline = ra_roadmap::outline::parse(&text)?;
    let store = open_store().await?;
    let root = store.read(under).await?;
    if dry {
        println!("would add {} under #{under} {}:", outline.summary(), root.title);
        for line in outline.preview() {
            println!("  {line}");
        }
        return Ok(());
    }
    let ids = store.import(under, &outline).await?;
    let nodes = store.all().await?;
    for id in &ids {
        if let Some(node) = nodes.iter().find(|n| n.id == *id) {
            println!("#{id} {} {}", node.kind, node.title);
        }
    }
    println!("added {} under #{under} {}", outline.summary(), root.title);
    Ok(())
}

/// Shows the planner's cards for `id` and adds them when `yes` or the person answers `y`.
async fn plan(store: &Store, id: i64, worker: &Worker, yes: bool) -> anyhow::Result<()> {
    use std::io::Write as _;
    let plan = ra_console::agent::plan(store, id, worker).await?;
    for line in plan.preview() {
        println!("{line}");
    }
    if !yes {
        print!("add them under #{id}? [y/N] ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("nothing added");
            return Ok(());
        }
    }
    for line in plan.add(store).await? {
        println!("{line}");
    }
    Ok(())
}

async fn edit_card(id: i64, edit: &Edit<'_>) -> anyhow::Result<()> {
    anyhow::ensure!(
        !edit.is_empty(),
        "nothing to edit: name a field (--title, --objective, --acceptance, --complexity, --files, --clear-session)"
    );
    let store = open_store().await?;
    store.edit(id, edit).await?;
    let node = store.read(id).await?;
    println!("#{id} {}", node.title);
    let changed: Vec<&str> = [
        edit.title.map(|_| "title"),
        edit.objective.map(|_| "objective"),
        edit.acceptance.map(|_| "acceptance"),
        edit.complexity.map(|_| "complexity"),
        edit.files.map(|_| "files"),
        edit.clear_session.then_some("session cleared"),
    ]
    .into_iter()
    .flatten()
    .collect();
    println!("    edited: {}", changed.join(", "));
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

/// Prints the history, one change a line as the log holds it; a card without any says so.
async fn show_history(id: Option<i64>) -> anyhow::Result<()> {
    let store = open_store().await?;
    if let Some(id) = id {
        store.read(id).await?;
    }
    let changes = store.history(id)?;
    if changes.is_empty() {
        println!(
            "{}",
            id.map_or_else(|| "no state change recorded yet".to_owned(), |id| format!("#{id} has no state change recorded yet"))
        );
    }
    for change in &changes {
        println!("{}", change.line());
    }
    if let Some(last) = changes.last() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        println!("last change {} ago", history::age(now.saturating_sub(last.at)));
    }
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
    let store = Store::open(&roadmap, "Rhipper Agent").await?.with_source(Source::Console);
    if !ra_console::ui::is_terminal() {
        anyhow::bail!("the console needs a terminal");
    }
    let root = roadmap.parent().filter(|p| !p.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let replaced = if replace { ra_console::instance::stop_running(&root) } else { None };
    ra_console::instance::claim(&root)?;
    let sink = ra_console::logs::Sink::new();
    ra_console::trace::init(log, Some(sink.clone()), None);
    if let Some(what) = replaced {
        tracing::info!("{what}");
    }
    let worker = Worker::new(ra_console::ui::DEFAULT_MODEL).with_ledger(open_ledger().await?);
    let result = ra_console::ui::run(store, worker, sink).await;
    ra_console::instance::release(&root);
    result
}
