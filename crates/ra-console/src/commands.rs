//! What can be typed at the console. The commands read and shape the roadmap; running a task
//! goes through the orchestrator, which the console drives through the same store.

use anyhow::Context;
use ra_roadmap::model::{Effort, Gambit, GambitState, Node, NodeKind, TaskState};
use ra_roadmap::store::{Edit, Store};
use ra_roadmap::tree;
use ra_usage::Totals;

use crate::agent::{self, Worker};

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
    Spec {
        name: "edit",
        usage: "<id> <title|objective|acceptance|complexity|files|session> <text>",
        help: "rewrites one field of a card and nothing else; `session clear` forgets the harness session",
    },
    Spec { name: "depend", usage: "<id> <id>...", help: "makes the first node wait for the others" },
    Spec {
        name: "files",
        usage: "<id> [path...]",
        help: "the workspace-relative paths a task touches, read first by its worker; no path clears them",
    },
    Spec { name: "set", usage: "<id> <state>", help: "sets the state: the twelve lifecycle names; cancelled removes the card" },
    Spec {
        name: "remove",
        usage: "<id> [tree]",
        help: "removes a card and every waits-on reference to it; tree takes what is under it too",
    },
    Spec {
        name: "model",
        usage: "<id>",
        help: "the model of the next worker session: a claude-* id runs on Claude Code, local/<name> on the local server, vendor/model on OpenRouter",
    },
    Spec { name: "assign", usage: "<id> <model|none>", help: "the model one node runs on, over the badge; none clears it" },
    Spec { name: "budgets", usage: "[on|off]", help: "whether a run is capped in dollars by its card's complexity; off unless turned on" },
    Spec { name: "effort", usage: "<id> <low|medium|high>", help: "the effort a run of the card asks of its model; medium when unset" },
    Spec { name: "priority", usage: "<id> <1-5|none>", help: "the person's priority on a card, taken first by the walk; none clears it" },
    Spec { name: "gambits", usage: "", help: "the gambits: the standing directions that order the walk, with weight and state" },
    Spec {
        name: "gambit",
        usage: "add <text> [w<n>] | <id> park|activate|done | <id> weight <n> | <id> focus [words]",
        help: "adds a gambit (weight 3 unless w<n> ends the line) or changes one: its state, its weight 1 to 5, or the words a card matches on",
    },
    Spec { name: "key", usage: "[<paste> | clear]", help: "stores, shows the state of, or clears the OpenRouter key" },
    Spec { name: "ping", usage: "", help: "one round trip to the badge model" },
    Spec {
        name: "usage",
        usage: "[id]",
        help: "the ledger by card, summed up the tree, for the subtree of a node; the whole roadmap without an id",
    },
    Spec {
        name: "context",
        usage: "[id]",
        help: "the context a run of the card would send, or the project context without an id; folded, Enter on the empty prompt unfolds it",
    },
    Spec {
        name: "run",
        usage: "<id> [dry]",
        help: "executes one roadmap task on the badge model; dry prints what it would send and calls nothing",
    },
    Spec {
        name: "review",
        usage: "<id>",
        help: "a second model reads the card's acceptance, handoff and git diff and writes its verdict into the notes: accepted, or needs revision with the points",
    },
    Spec { name: "reassess", usage: "<id>", help: "the model revises the objective and acceptance of one node" },
    Spec {
        name: "reassess-tree",
        usage: "<id>",
        help: "reassess for the node and its whole subtree; on a milestone it also proposes the cards its objective lacks",
    },
    Spec { name: "proposals", usage: "<id>", help: "the cards a reassess-tree proposed under a milestone, waiting for a word" },
    Spec { name: "accept", usage: "<id> <n>", help: "adds proposal n of the milestone as a task under it" },
    Spec { name: "decline", usage: "<id> <n>", help: "drops proposal n of the milestone; it is remembered and not proposed again" },
    Spec { name: "final", usage: "<id> [off]", help: "marks a node final, so no reassessment rewrites it; off clears the mark" },
    Spec { name: "seed", usage: "", help: "fills the roadmap with the improvement plan of the agent itself" },
    Spec {
        name: "install",
        usage: "[dir] [desktop]",
        help: "installs this build (rc and rc-gui) into a directory on the path; desktop adds a shortcut to the window when there is none",
    },
    Spec {
        name: "note",
        usage: "<text>",
        help: "an observation of yours for the next runs: what slowed one, a quirk, a direction; the workers read the latest",
    },
    Spec {
        name: "filter",
        usage: "[state...]",
        help: "hides the tasks in those states from the tree, and the structure left empty; no state shows everything again",
    },
    Spec {
        name: "log",
        usage: "[id]",
        help: "saves the full stream of the last run, or of the last run of card id, from the activity log and the harness's events to var/runs/<id>-<time>.log",
    },
    Spec { name: "clear", usage: "", help: "clears the log view" },
    Spec { name: "debug", usage: "", help: "toggles the trace lines in the log; the file copy always runs" },
    Spec { name: "quit", usage: "", help: "stops the console (also exit, or Ctrl+C twice)" },
];

#[derive(Debug)]
pub enum Outcome {
    Continue,
    Clear,
    Quit,
    /// The console adopts the model on the badge: `model <id>` set it.
    Model(String),
    /// The console shows or hides the trace lines: `debug` toggled them.
    ToggleDebug,
    /// The console turns the complexity budgets on or off: `budgets` set it.
    Budgets(bool),
    /// The lines fold under their first one, a header, until the person opens them: `context`
    /// printed a whole file.
    Fold,
    /// The tree hides the tasks in these states: `filter` named them; empty shows everything.
    Filter(Vec<TaskState>),
}

/// Command names that start with `prefix`.
pub fn complete(prefix: &str) -> Vec<&'static str> {
    COMMANDS.iter().map(|c| c.name).filter(|n| n.starts_with(prefix)).collect()
}

/// What Tab makes of a line.
#[derive(Debug, PartialEq, Eq)]
pub enum Completion {
    /// Nothing matches what is typed.
    None,
    /// One match: the line to put in place of the typed one, ending in a space.
    Line(String),
    /// Several matches, to show under the prompt.
    Choices(Vec<String>),
}

/// Completes the word under the cursor of a console line: a command name on the first word; a
/// card id on the first argument of a command that takes one, from the id typed so far or from
/// the words of a title (`run key` finds the one card whose title contains "key"); a state on
/// the second argument of `set`. The leading `/` or `.` stays as typed.
pub fn complete_line(line: &str, nodes: &[Node]) -> Completion {
    let (mark, text) = match line.strip_prefix('/') {
        Some(rest) => ("/", rest),
        None => match line.strip_prefix('.') {
            Some(rest) => (".", rest),
            None => ("", line),
        },
    };
    let Some((name, rest)) = text.split_once(char::is_whitespace) else {
        return match complete(text).as_slice() {
            [] => Completion::None,
            [one] => Completion::Line(format!("{mark}{one} ")),
            many => Completion::Choices(many.iter().map(|&n| n.to_owned()).collect()),
        };
    };
    let Some(spec) = COMMANDS.iter().find(|c| c.name == name) else {
        return Completion::None;
    };
    if !(spec.usage.starts_with("<id>") || spec.usage.starts_with("[id]")) {
        return Completion::None;
    }
    let rest = rest.trim_start();
    let first = rest.split_whitespace().next().unwrap_or_default();
    if first.parse::<i64>().is_ok() && rest.len() > first.len() {
        // the id is in place; only `set` has a word to complete after it
        if name != "set" {
            return Completion::None;
        }
        let typed = rest[first.len()..].trim_start();
        let states: Vec<&str> = TaskState::ALL.iter().map(|s| s.serde_name()).filter(|s| s.starts_with(typed)).collect();
        return match states.as_slice() {
            [] => Completion::None,
            [one] => Completion::Line(format!("{mark}{name} {first} {one} ")),
            many => Completion::Choices(many.iter().map(|&s| s.to_owned()).collect()),
        };
    }
    let query = rest.trim_end();
    if query.is_empty() {
        return Completion::None;
    }
    let lower = query.to_lowercase();
    let found: Vec<&Node> =
        nodes.iter().filter(|n| n.id.to_string().starts_with(query) || n.title.to_lowercase().contains(&lower)).collect();
    match found.as_slice() {
        [] => Completion::None,
        [one] => Completion::Line(format!("{mark}{name} {} ", one.id)),
        many => Completion::Choices(many.iter().map(|n| format!("#{} {}", n.id, n.title)).collect()),
    }
}

/// Runs one line against the store; the lines to show, or an error to show. `worker` holds the
/// badge model and the provider, which `ping`, `run` and the reassessments talk to.
pub async fn run(line: &str, store: &Store, worker: &Worker) -> anyhow::Result<(Vec<String>, Outcome)> {
    let raw = line.trim();
    let mut out = Vec::new();
    if raw.is_empty() {
        return Ok((out, Outcome::Continue));
    }
    // a command starts with a slash; anything else typed is a note for the next runs, the
    // person's own look back, kept with the workers' ("stop after this task" once closed the
    // window as a command)
    let Some(line) = raw.strip_prefix('/').or_else(|| raw.strip_prefix('.')) else {
        crate::retro::record_pair(&agent::workspace_of(store), raw)?;
        out.push("noted; the next runs read it · a command starts with /".to_owned());
        return Ok((out, Outcome::Continue));
    };
    let line = line.trim_start();
    let (name, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let args = rest.trim();
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
                    let subtree = tree::subtree(&nodes, root);
                    anyhow::ensure!(!subtree.is_empty(), "no node {root}");
                    subtree
                }
                Err(_) => nodes.iter().collect(),
            };
            for node in &picked {
                out.push(format!(
                    "{}#{} {} [{}]{}{}",
                    "  ".repeat(tree::depth(&nodes, node.id)),
                    node.id,
                    node.title,
                    node.state,
                    if node.is_final() { " [final]" } else { "" },
                    match &node.assignee {
                        Some(a) => format!(" ← {}:{}", a.role, a.model),
                        None => String::new(),
                    }
                ));
                if let Some(objective) = &node.objective {
                    out.push(format!("    {objective}"));
                }
                if !node.files.is_empty() {
                    out.push(format!("    files: {}", node.files.join(" ")));
                }
            }
            if picked.is_empty() {
                out.push("the roadmap is empty: add a project first".into());
            }
        }
        "add" => {
            let (kind_text, rest) = args.split_once(char::is_whitespace).context("usage: add <kind> <title>")?;
            let kind: NodeKind = kind_text.parse()?;
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
        "edit" => {
            const USAGE: &str = "usage: edit <id> <title|objective|acceptance|complexity|files|session> <text>";
            let (id_text, rest) = args.split_once(char::is_whitespace).context(USAGE)?;
            let id = id_text.parse::<i64>().context("the id is a node id")?;
            let (field, text) = rest.trim().split_once(char::is_whitespace).unwrap_or((rest.trim(), ""));
            let text = text.trim();
            let files: Vec<String>;
            let mut edit = Edit::default();
            match field.to_ascii_lowercase().as_str() {
                "title" => edit.title = Some(text),
                "objective" => edit.objective = Some(text),
                "acceptance" => edit.acceptance = Some(text),
                "complexity" => edit.complexity = Some(text.parse().context("complexity is 1 to 5")?),
                "files" => {
                    // a path is stored with forward slashes, so the file reads the same on every platform
                    files = text.split_whitespace().map(|p| p.replace('\\', "/")).collect();
                    edit.files = Some(&files);
                }
                "session" => {
                    anyhow::ensure!(text == "clear", "usage: edit <id> session clear");
                    edit.clear_session = true;
                }
                _ => anyhow::bail!("{USAGE}"),
            }
            anyhow::ensure!(field == "session" || field == "files" || !text.is_empty(), "{USAGE}");
            store.edit(id, &edit).await?;
            out.push(if edit.clear_session { format!("#{id} forgets its session") } else { format!("#{id} {field} rewritten") });
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
        "files" => {
            let mut words = args.split_whitespace();
            let id = words.next().context("usage: files <id> [path...]")?.parse::<i64>().context("the id is a node id")?;
            // a path is stored with forward slashes, so the file reads the same on every platform
            let files: Vec<String> = words.map(|p| p.replace('\\', "/")).collect();
            store.set_files(id, &files).await?;
            out.push(if files.is_empty() {
                format!("#{id} names no files")
            } else {
                format!("#{id} names {} files: {}", files.len(), files.join(" "))
            });
        }
        "set" => {
            let (id_text, state_text) = args.split_once(char::is_whitespace).context("usage: set <id> <state>")?;
            let id = id_text.parse::<i64>().context("the id is a node id")?;
            let state: TaskState = state_text.parse()?;
            store.set_state(id, state).await?;
            if state == TaskState::Cancelled {
                out.push(format!("#{id} removed: a cancelled card does not stay on the roadmap"));
            } else {
                out.push(format!("#{id} is now {state}"));
            }
        }
        "effort" => {
            let (id_text, level_text) = args.split_once(char::is_whitespace).context("usage: effort <id> <low|medium|high>")?;
            let id = id_text.parse::<i64>().context("the id is a node id")?;
            let effort: Effort = level_text.parse()?;
            store.set_effort(id, effort).await?;
            out.push(format!("#{id} runs at {effort} effort"));
        }
        "priority" => {
            const USAGE: &str = "usage: priority <id> <1-5|none>";
            let (id_text, value) = args.split_once(char::is_whitespace).context(USAGE)?;
            let id = id_text.parse::<i64>().context("the id is a node id")?;
            let priority = match value.trim() {
                "none" | "clear" => None,
                n => Some(n.parse::<u8>().context(USAGE)?),
            };
            store.set_priority(id, priority).await?;
            out.push(match priority {
                Some(p) => format!("#{id} has priority {p}"),
                None => format!("#{id} has no priority: the file and the gambits order it"),
            });
        }
        "gambits" => {
            anyhow::ensure!(args.is_empty(), "usage: gambits");
            out.extend(gambit_lines(&store.gambits().await?));
        }
        "gambit" => {
            const USAGE: &str =
                "usage: gambit add <text> [w<n>] | gambit <id> park|activate|done | gambit <id> weight <n> | gambit <id> focus [words]";
            let (first, rest) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
            let rest = rest.trim();
            if first == "add" {
                // a trailing w<n> is the weight; the words before it are the text
                let (text, weight) = match rest.rsplit_once(char::is_whitespace) {
                    Some((text, last)) if last.len() > 1 && last.starts_with('w') && last[1..].chars().all(|c| c.is_ascii_digit()) => {
                        (text.trim(), last[1..].parse::<u8>().context("the weight is 1 to 5")?)
                    }
                    _ => (rest, 3),
                };
                anyhow::ensure!(!text.is_empty(), "{USAGE}");
                let id = store.add_gambit(text, weight, &[]).await?;
                out.push(format!("g{id} {text} {{w{weight}}} [active]"));
                return Ok((out, Outcome::Continue));
            }
            let id = gambit_id(first).context(USAGE)?;
            let (action, words) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let words = words.trim();
            match action {
                "park" | "activate" | "done" => {
                    anyhow::ensure!(words.is_empty(), "{USAGE}");
                    let state = match action {
                        "park" => GambitState::Parked,
                        "activate" => GambitState::Active,
                        _ => GambitState::Done,
                    };
                    store.set_gambit_state(id, state).await?;
                    out.push(format!("g{id} is now {state}"));
                }
                "weight" => {
                    let weight = words.parse::<u8>().context("usage: gambit <id> weight <1-5>")?;
                    store.set_gambit_weight(id, weight).await?;
                    out.push(format!("g{id} weighs {weight}"));
                }
                "focus" => {
                    let focus: Vec<String> = words.split_whitespace().map(str::to_owned).collect();
                    store.set_gambit_focus(id, &focus).await?;
                    out.push(if focus.is_empty() {
                        format!("g{id} matches every card")
                    } else {
                        format!("g{id} focuses on {}", focus.join(" "))
                    });
                }
                _ => anyhow::bail!("{USAGE}"),
            }
        }
        "remove" | "rm" => {
            let (id_text, rest) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
            let id = id_text.parse::<i64>().context("usage: remove <id> [tree]")?;
            let removed = store.remove(id, rest.trim() == "tree").await?;
            out.push(format!("removed {}", removed.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(" ")));
        }
        "seed" => {
            anyhow::ensure!(args.is_empty(), "usage: seed");
            let ids = seed(store).await?;
            out.push(format!("planted the improvement roadmap: {} nodes under the project", ids.len()));
        }
        "key" => {
            if args == "clear" {
                crate::secret::clear()?;
                worker.forget();
                out.push("the stored key is gone; the environment variable still wins when it is set".into());
            } else if args.is_empty() {
                if crate::secret::has_key() {
                    out.push(format!("a key is stored ({}); the environment variable overrides it when set", crate::secret::masked()?));
                } else {
                    out.push("no key stored; set one with key <paste>".into());
                }
            } else {
                crate::secret::set_key(args)?;
                // the next call connects with the new key
                worker.forget();
                out.push(format!("the key is in the credential store ({})", crate::secret::masked()?));
            }
        }
        "ping" => {
            anyhow::ensure!(args.is_empty(), "usage: ping");
            return Ok((agent::ping(worker).await?, Outcome::Continue));
        }
        "run" => {
            let (id_text, flag) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
            let id = id_text.parse::<i64>().context("usage: run <id> [dry]")?;
            let dry = match flag.trim() {
                "" => false,
                "dry" => true,
                _ => anyhow::bail!("usage: run <id> [dry]"),
            };
            // a node with a model of its own runs on it; the badge is the default
            let mut runner = worker.clone();
            if let Some(assignment) = store.read(id).await?.assignee {
                runner.model = agent::resolve(&assignment.model);
            }
            let lines = if dry { agent::dry_run(store, id, &runner).await? } else { agent::run(store, id, &runner).await? };
            return Ok((lines, Outcome::Continue));
        }
        "assign" => {
            let (id_text, model) = args.split_once(char::is_whitespace).context("usage: assign <id> <model|none>")?;
            let id = id_text.parse::<i64>().context("the id is a node id")?;
            let model = agent::resolve(model);
            let assignment = (model != "none").then(|| ra_roadmap::model::Assignment { model, role: "implementer".to_owned() });
            store.assign(id, assignment.as_ref()).await?;
            out.push(match &assignment {
                Some(a) => format!("#{id} runs on {} through {}", a.model, agent::Route::of(&a.model).name()),
                None => format!("#{id} runs on the badge model"),
            });
        }
        "review" => {
            let id = args.parse::<i64>().context("usage: review <id>")?;
            return Ok((agent::review(store, id, worker).await?, Outcome::Continue));
        }
        "reassess" | "reassess-tree" => {
            let id = args.parse::<i64>().context(format!("usage: {name} <id>"))?;
            return Ok((agent::reassess(store, id, worker, name == "reassess-tree").await?, Outcome::Continue));
        }
        "proposals" => {
            let id = args.parse::<i64>().context("usage: proposals <id>")?;
            let proposals = agent::proposals_of(store, id);
            if proposals.is_empty() {
                out.push(format!("no proposals wait on #{id}; a reassess-tree of a milestone makes them"));
            } else {
                out.extend(agent::proposal_lines(id, &proposals));
            }
        }
        "accept" | "decline" => {
            let usage = format!("usage: {name} <id> <n>");
            let (id_text, n_text) = args.split_once(char::is_whitespace).context(usage.clone())?;
            let id = id_text.parse::<i64>().context(usage.clone())?;
            let n = n_text.trim().parse::<usize>().context(usage)?;
            let lines =
                if name == "accept" { agent::accept_proposal(store, id, n).await? } else { agent::decline_proposal(store, id, n).await? };
            return Ok((lines, Outcome::Continue));
        }
        "final" => {
            let (id_text, flag) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
            let id = id_text.parse::<i64>().context("usage: final <id> [off]")?;
            // the declined proposals on the line stay either way
            let node = store.read(id).await?;
            match flag.trim() {
                "" => {
                    store.set_assessed(id, agent::assessed_line(&node, Some(ra_roadmap::model::FINAL)).as_deref()).await?;
                    out.push(format!("#{id} is final: no reassessment rewrites it"));
                }
                "off" => {
                    // clearing the mark drops the fingerprint too, so the next reassessment calls
                    store.set_assessed(id, agent::assessed_line(&node, None).as_deref()).await?;
                    out.push(format!("#{id} is no longer final"));
                }
                _ => anyhow::bail!("usage: final <id> [off]"),
            }
        }
        "install" => {
            // `install [dir] [desktop]`: the word desktop adds the shortcut, anything else is the directory
            let words: Vec<&str> = args.split_whitespace().collect();
            let desktop = words.iter().any(|w| w.eq_ignore_ascii_case("desktop"));
            let dir = words.iter().find(|w| !w.eq_ignore_ascii_case("desktop")).map(std::path::PathBuf::from);
            out.extend(crate::install::install(dir.as_deref())?);
            if desktop {
                let dir = dir.unwrap_or_else(crate::install::default_dir);
                out.push(crate::install::desktop_shortcut(&dir, &agent::workspace_of(store)));
            }
        }
        "note" => {
            crate::retro::record_pair(&agent::workspace_of(store), args)?;
            out.push("noted; the next runs read it with the workers' own look-backs".to_owned());
        }
        "log" => {
            let id = if args.is_empty() { None } else { Some(args.parse::<i64>().context("usage: log [id]")?) };
            let saved = save_run_log(&agent::workspace_of(store), id)?;
            out.push(format!(
                "#{} · {} lines{} · {}",
                saved.id,
                saved.lines,
                if saved.events { "" } else { " · no harness events: the stream kept is a newer run's" },
                saved.path.display()
            ));
        }
        "clear" | "cls" => return Ok((out, Outcome::Clear)),
        "debug" => {
            anyhow::ensure!(args.is_empty(), "usage: debug");
            return Ok((out, Outcome::ToggleDebug));
        }
        "filter" => {
            let states = args.split_whitespace().map(str::parse::<TaskState>).collect::<Result<Vec<_>, _>>()?;
            out.push(if states.is_empty() {
                "filter cleared: the tree shows every card".to_owned()
            } else {
                format!("the tree hides {}", states.iter().map(|s| s.serde_name()).collect::<Vec<_>>().join(", "))
            });
            return Ok((out, Outcome::Filter(states)));
        }
        "model" => {
            anyhow::ensure!(!args.is_empty(), "usage: model <id>, such as opus, claude-opus-5-5, local/qwen3 or z-ai/glm-5.3-flash");
            let model = agent::resolve(args);
            out.push(format!("the next worker session runs on {model} through {}", agent::Route::of(&model).name()));
            return Ok((out, Outcome::Model(model)));
        }
        "budgets" => {
            return match args {
                "" => {
                    out.push(format!(
                        "budgets are {}",
                        if worker.budgets { "on: a run is capped by its card's complexity" } else { "off" }
                    ));
                    Ok((out, Outcome::Continue))
                }
                "on" | "off" => {
                    let on = args == "on";
                    out.push(format!(
                        "budgets {}",
                        if on { "on: a run is capped by its card's complexity" } else { "off: a run spends what it needs" }
                    ));
                    Ok((out, Outcome::Budgets(on)))
                }
                _ => anyhow::bail!("usage: budgets [on|off]"),
            };
        }
        "context" => {
            // the header names what follows and how long it is; the body is the text, line by line
            let (what, text) = if let Ok(id) = args.parse::<i64>() {
                (format!("context #{id}"), agent::context_preview(store, id).await?)
            } else {
                anyhow::ensure!(args.is_empty(), "usage: context [id]");
                let text = agent::project_context(store);
                anyhow::ensure!(!text.is_empty(), "no {} beside the roadmap yet; a run drafts it", agent::PROJECT_FILE);
                (agent::PROJECT_FILE.to_owned(), text)
            };
            let body: Vec<String> = text.trim_end().lines().map(str::to_owned).collect();
            out.push(format!("{what} · {} lines", body.len()));
            out.extend(body);
            return Ok((out, Outcome::Fold));
        }
        "usage" => {
            let nodes = store.all().await?;
            let root = if let Ok(id) = args.parse::<i64>() {
                anyhow::ensure!(nodes.iter().any(|n| n.id == id), "no node {id}");
                id
            } else {
                anyhow::ensure!(args.is_empty(), "usage: usage [id]");
                nodes.first().context("the roadmap is empty")?.id
            };
            let by_node = worker.by_node().await.context("no usage ledger is open")?;
            let rows = usage_by_card(&nodes, root, &by_node);
            if rows.is_empty() {
                out.push("the ledger is empty: no completion has run yet".into());
            }
            // one line per card, so the log gives each its own lane color
            for (node, t) in rows {
                let cost = t.cost_usd.map_or_else(|| "—".to_owned(), |c| format!("${c:.4}"));
                out.push(match node {
                    Some(node) => format!(
                        "{}#{} {} · {} calls · {} turns · {} in · {} out · {} cached · {cost}",
                        "  ".repeat(tree::depth(&nodes, node.id)),
                        node.id,
                        node.title,
                        t.calls,
                        t.turns,
                        t.prompt_tokens,
                        t.completion_tokens,
                        t.cached_tokens
                    ),
                    None => format!(
                        "outside any card · {} calls · {} turns · {} in · {} out · {} cached · {cost}",
                        t.calls, t.turns, t.prompt_tokens, t.completion_tokens, t.cached_tokens
                    ),
                });
            }
        }
        // the whole line, or nothing: `stop after this task` once closed the window
        "quit" | "exit" | "shutdown" if args.is_empty() => return Ok((out, Outcome::Quit)),
        "quit" | "exit" | "shutdown" => anyhow::bail!("`{name}` takes no words; `stop` asks the walk to stop after the current task"),
        "stop" => {
            out.push(agent::stop_walk(&agent::workspace_of(store)));
        }
        other => anyhow::bail!("no command /{other}; /help lists them"),
    }
    Ok((out, Outcome::Continue))
}

/// The gambits as the console, the CLI and the MCP server list them: one line each as the file
/// writes it, the focus words under it; a word when there are none.
#[must_use]
pub fn gambit_lines(gambits: &[Gambit]) -> Vec<String> {
    if gambits.is_empty() {
        return vec!["no gambits yet: gambit add <text> [w<n>] states a direction".to_owned()];
    }
    let mut out = Vec::new();
    for gambit in gambits {
        out.push(format!("g{} {} {{w{}}} [{}]", gambit.id, gambit.text, gambit.weight, gambit.state));
        if !gambit.focus.is_empty() {
            out.push(format!("    focus: {}", gambit.focus.join(" ")));
        }
    }
    out
}

/// The number of a gambit as typed, `g2` or `2`.
fn gambit_id(text: &str) -> Option<u32> {
    text.strip_prefix('g').unwrap_or(text).parse().ok()
}

/// One run cut out of the activity log: from its start marker to its end marker, or to the end
/// of the log when it is still going.
#[derive(Debug, PartialEq, Eq)]
struct RunBlock {
    /// The card the run was of.
    id: i64,
    /// The index of the start marker in the log, so the newest run is known from any other.
    start: usize,
    /// The progress lines, markers included.
    lines: Vec<String>,
}

/// The last run in the activity text, or the last run of card `id`; none when no such run started.
fn last_run_of(activity: &str, id: Option<i64>) -> Option<RunBlock> {
    let lines: Vec<&str> = activity.lines().collect();
    let card_of = |line: &str| -> Option<i64> {
        let rest = line.strip_prefix(crate::logs::RUN_START)?.strip_prefix('#')?;
        rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
    };
    let start = lines.iter().rposition(|line| card_of(line).is_some_and(|card| id.is_none_or(|id| id == card)))?;
    let end = lines[start..].iter().position(|line| line.starts_with(crate::logs::RUN_END)).map_or(lines.len(), |i| start + i + 1);
    Some(RunBlock { id: card_of(lines[start])?, start, lines: lines[start..end].iter().map(|l| (*l).to_owned()).collect() })
}

/// What `log` wrote: the card, the file, how many progress lines, and whether the harness's
/// events went with them.
struct SavedLog {
    id: i64,
    path: std::path::PathBuf,
    lines: usize,
    events: bool,
}

/// Writes the last run under `root`, or the last run of card `id`, to `var/runs/<id>-<time>.log`:
/// its progress lines from the activity log, then the harness's event stream when that stream
/// is this run's; the stream kept is only the newest run's.
fn save_run_log(root: &std::path::Path, id: Option<i64>) -> anyhow::Result<SavedLog> {
    let activity = std::fs::read_to_string(root.join(crate::ui::ACTIVITY_FILE)).unwrap_or_default();
    let run = last_run_of(&activity, id).with_context(|| match id {
        Some(id) => format!("no run of #{id} in {}; the console's own runs are not in it", crate::ui::ACTIVITY_FILE),
        None => format!("no run in {}; the console's own runs are not in it", crate::ui::ACTIVITY_FILE),
    })?;
    let newest = last_run_of(&activity, None).is_some_and(|n| n.start == run.start);
    let stream =
        if newest { std::fs::read_to_string(root.join(ra_providers::claude_code::STREAM_FILE)).unwrap_or_default() } else { String::new() };
    let mut text = run.lines.join("\n");
    text.push('\n');
    if !stream.is_empty() {
        text.push_str("\n--- harness events ---\n");
        text.push_str(&stream);
    }
    let dir = root.join("var/runs");
    std::fs::create_dir_all(&dir)?;
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let path = dir.join(format!("{}-{when}.log", run.id));
    std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(SavedLog { id: run.id, path, lines: run.lines.len(), events: !stream.is_empty() })
}

/// The ledger by card under `root`: every node of its subtree whose own subtree has calls, in
/// tree order, with the sums of that subtree (a milestone is its cards, the project the total),
/// as the window's ledger tab reads it. The project also takes the calls booked to a node the
/// roadmap lost; the calls that served no node close the list when `root` is the project.
fn usage_by_card<'a>(nodes: &'a [Node], root: i64, by_node: &[(Option<i64>, Totals)]) -> Vec<(Option<&'a Node>, Totals)> {
    let own = |id: i64| by_node.iter().find(|(n, _)| *n == Some(id)).map(|(_, t)| *t);
    let mut rows: Vec<(Option<&Node>, Totals)> = Vec::new();
    for node in tree::subtree(nodes, root) {
        let mut sum = Totals::default();
        for member in tree::subtree(nodes, node.id) {
            if let Some(t) = own(member.id) {
                sum.merge(&t);
            }
        }
        if node.kind == NodeKind::Project {
            for (id, t) in by_node {
                if id.is_some_and(|id| !nodes.iter().any(|n| n.id == id)) {
                    sum.merge(t);
                }
            }
        }
        if sum.calls > 0 {
            rows.push((Some(node), sum));
        }
    }
    let at_root = nodes.iter().any(|n| n.id == root && n.kind == NodeKind::Project);
    if at_root && let Some((_, t)) = by_node.iter().find(|(n, _)| n.is_none()) {
        rows.push((None, *t));
    }
    rows
}

/// Plants the improvement roadmap of the agent itself under the first project, as milestone,
/// section and task nodes with acceptance criteria. It refuses when the project already holds
/// nodes, so a seed never doubles.
async fn seed(store: &Store) -> anyhow::Result<Vec<i64>> {
    let nodes = store.all().await?;
    let project = nodes.first().with_context(|| "the roadmap is empty: the store always holds one project")?;
    anyhow::ensure!(nodes.len() == 1, "the project already holds nodes; seed runs on an empty roadmap");
    let root = project.id;
    let mut ids = Vec::new();
    let milestone = store
        .insert(
            Some(root),
            NodeKind::Milestone,
            "Phase A: a working loop",
            Some("one task goes from the roadmap to a model and back, end to end."),
            Some("`run <id>` completes on a real model, and the answer lands in the log."),
            3,
        )
        .await?;
    ids.push(milestone);
    let providers = store
        .insert(
            Some(milestone),
            NodeKind::Section,
            "Providers",
            Some("the console talks to OpenRouter with the key from the environment."),
            Some("a `ping` command rounds a hello through the chat endpoint."),
            2,
        )
        .await?;
    ids.push(providers);
    let t1 = store
        .insert(
            Some(providers),
            NodeKind::Task,
            "Load OPENROUTER_API_KEY",
            Some("the console reads the key and refuses plainly without one."),
            Some("`run` without the key prints a refusal that names it."),
            1,
        )
        .await?;
    ids.push(t1);
    let t2 = store
        .insert(
            Some(providers),
            NodeKind::Task,
            "A ping through the provider",
            Some("a `ping` command sends one message and prints the answer and the usage."),
            Some("`ping` shows the answer, then in, out and cached token counts."),
            2,
        )
        .await?;
    ids.push(t2);
    let runner = store
        .insert(
            Some(milestone),
            NodeKind::Section,
            "Task runner",
            Some("`run <id>` executes one roadmap task on the badge model."),
            Some("the state walks defined to implemented, and the answer lands in the log."),
            3,
        )
        .await?;
    ids.push(runner);
    let t3 = store
        .insert(
            Some(runner),
            NodeKind::Task,
            "run <id> builds the context",
            Some("the runner reads the node, its ancestors and its dependencies into one prompt."),
            Some("a task with an objective and acceptance produces a prompt that names both."),
            2,
        )
        .await?;
    ids.push(t3);
    let t4 = store
        .insert(
            Some(runner),
            NodeKind::Task,
            "run <id> records the answer",
            Some("the runner writes the answer to the log and the usage to the ledger."),
            Some("after a run, the token panel shows real counts and the node holds notes."),
            2,
        )
        .await?;
    ids.push(t4);
    store.depend_on(t2, t1).await?;
    store.depend_on(t3, t1).await?;
    store.depend_on(t4, t2).await?;
    store.depend_on(t4, t3).await?;
    Ok(ids)
}

#[cfg(test)]
mod tests;
