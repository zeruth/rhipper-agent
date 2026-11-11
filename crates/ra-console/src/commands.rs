//! What can be typed at the console. The commands read and shape the roadmap; running a task
//! goes through the orchestrator, which the console drives through the same store.

use anyhow::Context;
use ra_roadmap::model::{NodeKind, TaskState};
use ra_roadmap::store::Store;
use ra_roadmap::tree;

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
    Spec { name: "depend", usage: "<id> <id>...", help: "makes the first node wait for the others" },
    Spec {
        name: "files",
        usage: "<id> [path...]",
        help: "the workspace-relative paths a task touches, read first by its worker; no path clears them",
    },
    Spec { name: "set", usage: "<id> <state>", help: "sets the state: the twelve lifecycle names" },
    Spec {
        name: "model",
        usage: "<id>",
        help: "the model of the next worker session: a claude-* id runs on Claude Code, vendor/model on OpenRouter",
    },
    Spec { name: "assign", usage: "<id> <model|none>", help: "the model one node runs on, over the badge; none clears it" },
    Spec { name: "budgets", usage: "[on|off]", help: "whether a run is capped by its card's complexity; off on a subscription" },
    Spec { name: "key", usage: "[<paste> | clear]", help: "stores, shows the state of, or clears the OpenRouter key" },
    Spec { name: "ping", usage: "", help: "one round trip to the badge model" },
    Spec { name: "run", usage: "<id>", help: "executes one roadmap task on the badge model" },
    Spec { name: "reassess", usage: "<id>", help: "the model revises the objective and acceptance of one node" },
    Spec { name: "reassess-tree", usage: "<id>", help: "reassess for the node and its whole subtree" },
    Spec { name: "final", usage: "<id> [off]", help: "marks a node final, so no reassessment rewrites it; off clears the mark" },
    Spec { name: "seed", usage: "", help: "fills the roadmap with the improvement plan of the agent itself" },
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
}

/// Command names that start with `prefix`.
pub fn complete(prefix: &str) -> Vec<&'static str> {
    COMMANDS.iter().map(|c| c.name).filter(|n| n.starts_with(prefix)).collect()
}

/// Runs one line against the store; the lines to show, or an error to show. `worker` holds the
/// badge model and the provider, which `ping`, `run` and the reassessments talk to.
pub async fn run(line: &str, store: &Store, worker: &Worker) -> anyhow::Result<(Vec<String>, Outcome)> {
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
            out.push(format!("#{id} is now {state}"));
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
            let id = args.parse::<i64>().context("usage: run <id>")?;
            // a node with a model of its own runs on it; the badge is the default
            let mut runner = worker.clone();
            if let Some(assignment) = store.read(id).await?.assignee {
                runner.model = assignment.model;
            }
            return Ok((agent::run(store, id, &runner).await?, Outcome::Continue));
        }
        "assign" => {
            let (id_text, model) = args.split_once(char::is_whitespace).context("usage: assign <id> <model|none>")?;
            let id = id_text.parse::<i64>().context("the id is a node id")?;
            let model = model.trim();
            let assignment =
                (model != "none").then(|| ra_roadmap::model::Assignment { model: model.to_owned(), role: "implementer".to_owned() });
            store.assign(id, assignment.as_ref()).await?;
            out.push(match &assignment {
                Some(a) => format!("#{id} runs on {} through {:?}", a.model, agent::Route::of(&a.model)),
                None => format!("#{id} runs on the badge model"),
            });
        }
        "reassess" | "reassess-tree" => {
            let id = args.parse::<i64>().context(format!("usage: {name} <id>"))?;
            return Ok((agent::reassess(store, id, worker, name == "reassess-tree").await?, Outcome::Continue));
        }
        "final" => {
            let (id_text, flag) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
            let id = id_text.parse::<i64>().context("usage: final <id> [off]")?;
            match flag.trim() {
                "" => {
                    store.set_assessed(id, Some(ra_roadmap::model::FINAL)).await?;
                    out.push(format!("#{id} is final: no reassessment rewrites it"));
                }
                "off" => {
                    // clearing the mark drops the fingerprint too, so the next reassessment calls
                    store.set_assessed(id, None).await?;
                    out.push(format!("#{id} is no longer final"));
                }
                _ => anyhow::bail!("usage: final <id> [off]"),
            }
        }
        "clear" | "cls" => return Ok((out, Outcome::Clear)),
        "debug" => {
            anyhow::ensure!(args.is_empty(), "usage: debug");
            return Ok((out, Outcome::ToggleDebug));
        }
        "model" => {
            anyhow::ensure!(!args.is_empty(), "usage: model <id>, such as claude-opus-5-5 or z-ai/glm-5.3-flash");
            let route = match agent::Route::of(args) {
                agent::Route::ClaudeCode => "Claude Code",
                agent::Route::OpenRouter => "OpenRouter",
            };
            out.push(format!("the next worker session runs on {args} through {route}"));
            return Ok((out, Outcome::Model(args.to_owned())));
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
        "quit" | "exit" | "shutdown" | "stop" => return Ok((out, Outcome::Quit)),
        other => anyhow::bail!("no command {other}; help lists them"),
    }
    Ok((out, Outcome::Continue))
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
