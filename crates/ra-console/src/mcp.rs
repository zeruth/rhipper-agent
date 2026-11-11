//! The MCP server over the roadmap and the workers: newline-delimited JSON-RPC 2.0 on stdin and
//! stdout, the way an MCP client such as Claude Code starts a local server. It speaks the small
//! part of the protocol a tool server needs: `initialize`, `ping`, `tools/list` and
//! `tools/call`. Nothing but the protocol goes to stdout; the traces go to the file.
//!
//! The tools are the console's commands with typed arguments, so a client iterates on the
//! roadmap, runs tasks and reads the ledger without a terminal.

use anyhow::Context;
use ra_roadmap::model::NodeKind;
use ra_roadmap::store::{Card, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use crate::agent::{self, Worker};
use crate::commands;

/// The protocol version answered when the client names none.
const PROTOCOL: &str = "2025-06-18";

/// Serves until stdin closes. `worker` carries the badge model; the `model` tool swaps it.
pub async fn serve(store: Store, worker: Worker) -> anyhow::Result<()> {
    let mut worker = worker;
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = handle(&line, &store, &mut worker).await else { continue };
        out.write_all(reply.to_string().as_bytes()).await?;
        out.write_all(b"\n").await?;
        out.flush().await?;
    }
    Ok(())
}

/// One message in, at most one message out: a notification gets none.
async fn handle(line: &str, store: &Store, worker: &mut Worker) -> Option<Value> {
    // a byte-order mark at the start of a piped file is not part of the message
    let message: Value = match serde_json::from_str(line.trim_start_matches('\u{feff}')) {
        Ok(message) => message,
        Err(e) => return Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
    };
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    tracing::debug!("mcp: {method}");
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "rhipper", "version": crate::VERSION},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => Ok(call(&params, store, worker).await),
        m if m.starts_with("notifications/") => return None,
        other => Err((-32601, format!("no method {other}"))),
    };
    // a notification has no id and gets no reply, even on error
    let id = id?;
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, text)) => error(id, code, &text),
    })
}

fn error(id: Value, code: i64, text: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": text}})
}

/// The tool table: the name, what it does, and the JSON schema of its arguments.
fn tools() -> Vec<Value> {
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
        json!({
            "name": name,
            "description": description,
            "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false},
        })
    };
    let id = json!({"type": "integer", "description": "The node id."});
    vec![
        tool(
            "show",
            "The roadmap, or one node with its subtree: id, title, state, assignee and objective per line.",
            json!({"id": id}),
            &[],
        ),
        tool(
            "add",
            "Adds a node under its parent, complete in one call, and returns its id. A project takes no parent.",
            json!({
                "kind": {"type": "string", "enum": ["project", "milestone", "section", "task", "subtask"]},
                "title": {"type": "string"},
                "parent": {"type": "integer", "description": "The parent node id."},
                "objective": {"type": "string", "description": "What the node must achieve."},
                "acceptance": {"type": "string", "description": "How acceptance is checked: concrete, command-checkable."},
                "complexity": {"type": "integer", "minimum": 1, "maximum": 5},
                "files": {"type": "array", "items": {"type": "string"}, "description": "The workspace-relative files the worker reads first."},
                "waits_on": {"type": "array", "items": {"type": "integer"}, "description": "The cards this one waits on."},
                "model": {"type": "string", "description": "The model that runs the card: a claude-* id on Claude Code, local/<name> on the local server, vendor/model on OpenRouter."}
            }),
            &["kind", "title"],
        ),
        tool(
            "details",
            "Sets the objective, the acceptance, the notes or the model of a node; a field left out keeps its text. The model is per-node routing: a claude-* id runs on Claude Code, local/<name> on the local OpenAI-compatible server, vendor/model on OpenRouter; an empty string clears it.",
            json!({
                "id": id,
                "objective": {"type": "string"},
                "acceptance": {"type": "string"},
                "notes": {"type": "string"},
                "model": {"type": "string"}
            }),
            &["id"],
        ),
        tool(
            "set",
            "Sets the state of a node.",
            json!({"id": id, "state": {"type": "string", "enum": ["defined", "ready", "assigned", "in_progress", "implemented", "testing", "review", "accepted", "blocked", "failed", "needs_revision", "cancelled"]}}),
            &["id", "state"],
        ),
        tool(
            "depend",
            "Makes a node wait for others.",
            json!({"id": id, "waits_for": {"type": "array", "items": {"type": "integer"}}}),
            &["id", "waits_for"],
        ),
        tool(
            "run",
            "Executes one task: the worker acts in the project directory and the answer lands in the node's notes. Takes as long as the work does. Runs on `model` here, else the node's own model, else the badge model.",
            json!({"id": id, "model": {"type": "string", "description": "A model for this run only."}}),
            &["id"],
        ),
        tool(
            "reassess",
            "The model revises the objective and the acceptance of a node for the work still open; with tree, its whole subtree. Finished nodes are verified, not rewritten.",
            json!({"id": id, "tree": {"type": "boolean"}}),
            &["id"],
        ),
        tool(
            "model",
            "Sets or shows the badge model of this server's worker.",
            json!({"id": {"type": "string", "description": "The model id; left out, shows the current one."}}),
            &[],
        ),
        tool("ping", "One round trip to the badge model: the answer, the tokens and the cost.", json!({}), &[]),
        tool("usage", "The ledger by model: calls, tokens and cost, all time, and the session tally.", json!({}), &[]),
        tool(
            "context",
            "The context a run of a card would send, without running: the brief, the file outlines and the handoffs so far. Without an id, the project context file beside the roadmap.",
            json!({"id": id}),
            &[],
        ),
        tool("gotchas", "The gotchas that fired, by how often, and the refused commands no gotcha names yet.", json!({}), &[]),
        tool(
            "handoffs",
            "The handoffs of a card's earlier runs, oldest first: when, model, outcome, the check, the files changed, the report and the lint warnings.",
            json!({"id": id}),
            &["id"],
        ),
    ]
}

/// Runs one tool; the answer is text either way, with `isError` on a failure.
async fn call(params: &Value, store: &Store, worker: &mut Worker) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    match dispatch(name, &args, store, worker).await {
        Ok(lines) => json!({"content": [{"type": "text", "text": lines.join("\n")}], "isError": false}),
        Err(e) => json!({"content": [{"type": "text", "text": format!("{e:#}")}], "isError": true}),
    }
}

async fn dispatch(name: &str, args: &Value, store: &Store, worker: &mut Worker) -> anyhow::Result<Vec<String>> {
    let int = |key: &str| args.get(key).and_then(Value::as_i64);
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    let need_id = || int("id").context("id is required");
    match name {
        "show" => {
            let line = int("id").map_or_else(|| "/show".to_owned(), |id| format!("/show {id}"));
            Ok(commands::run(&line, store, worker).await?.0)
        }
        "add" => {
            let kind: NodeKind = text("kind").context("kind is required")?.parse()?;
            let title = text("title").context("title is required")?;
            let complexity = u8::try_from(int("complexity").unwrap_or(3)).context("complexity is 1 to 5")?;
            anyhow::ensure!((1..=5).contains(&complexity), "complexity is 1 to 5");
            let parent = int("parent");
            anyhow::ensure!(kind != NodeKind::Project || parent.is_none(), "a project has no parent");
            anyhow::ensure!(kind == NodeKind::Project || parent.is_some(), "a {kind} needs a parent");
            let files: Vec<String> = match args.get("files") {
                None => Vec::new(),
                Some(list) => list
                    .as_array()
                    .context("files is a list of paths")?
                    .iter()
                    .map(|f| f.as_str().map(str::to_owned).context("files is a list of paths"))
                    .collect::<anyhow::Result<_>>()?,
            };
            let waits_on: Vec<i64> = match args.get("waits_on") {
                None => Vec::new(),
                Some(list) => list
                    .as_array()
                    .context("waits_on is a list of ids")?
                    .iter()
                    .map(|i| i.as_i64().context("waits_on is a list of ids"))
                    .collect::<anyhow::Result<_>>()?,
            };
            let card = Card {
                objective: text("objective"),
                acceptance: text("acceptance"),
                complexity,
                files: &files,
                waits_on: &waits_on,
                model: text("model"),
            };
            let id = store.insert_card(parent, kind, title, &card).await?;
            Ok(vec![format!("#{id} {title}")])
        }
        "details" => {
            let id = need_id()?;
            let mut changed = Vec::new();
            if text("objective").is_some() || text("acceptance").is_some() {
                store.set_details(id, text("objective"), text("acceptance")).await?;
                changed.push("details");
            }
            if let Some(notes) = text("notes") {
                store.set_notes(id, notes).await?;
                changed.push("notes");
            }
            if let Some(model) = text("model") {
                let assignment =
                    (!model.is_empty()).then(|| ra_roadmap::model::Assignment { model: model.to_owned(), role: "implementer".to_owned() });
                store.assign(id, assignment.as_ref()).await?;
                changed.push("model");
            }
            anyhow::ensure!(!changed.is_empty(), "nothing to set: pass objective, acceptance, notes or model");
            Ok(vec![format!("#{id}: {} set", changed.join(", "))])
        }
        "set" => {
            let line = format!("/set {} {}", need_id()?, text("state").context("state is required")?);
            Ok(commands::run(&line, store, worker).await?.0)
        }
        "depend" => {
            let ids: Vec<String> = args
                .get("waits_for")
                .and_then(Value::as_array)
                .context("waits_for is a list of ids")?
                .iter()
                .filter_map(Value::as_i64)
                .map(|i| i.to_string())
                .collect();
            let line = format!("/depend {} {}", need_id()?, ids.join(" "));
            Ok(commands::run(&line, store, worker).await?.0)
        }
        "run" => {
            let id = need_id()?;
            // the model named for this run, else the node's own, else the badge
            let node = store.read(id).await?;
            let picked = text("model").map(str::to_owned).or_else(|| node.assignee.map(|a| a.model));
            let mut runner = worker.clone();
            if let Some(model) = picked {
                runner.model = agent::resolve(&model);
            }
            agent::run(store, id, &runner).await
        }
        "reassess" => {
            let tree = args.get("tree").and_then(Value::as_bool).unwrap_or(false);
            agent::reassess(store, need_id()?, worker, tree).await
        }
        "model" => match text("id") {
            Some(model) => {
                worker.model = agent::resolve(model);
                Ok(vec![format!("the badge model is {} ({:?})", worker.model, worker.route())])
            }
            None => Ok(vec![format!("the badge model is {} ({:?})", worker.model, worker.route())]),
        },
        "ping" => agent::ping(worker).await,
        "usage" => {
            let mut out = Vec::new();
            let session = worker.tally();
            out.push(format!(
                "session: {} call(s), in {} out {} cached {}, cost {}",
                session.calls,
                session.prompt_tokens,
                session.completion_tokens,
                session.cached_tokens,
                session.cost_usd.map_or("—".to_owned(), |c| format!("${c:.4}"))
            ));
            if let Some(all) = worker.all_time().await {
                out.push(format!(
                    "all time: {} call(s), in {} out {} cached {}, cost {}",
                    all.calls,
                    all.prompt_tokens,
                    all.completion_tokens,
                    all.cached_tokens,
                    all.cost_usd.map_or("—".to_owned(), |c| format!("${c:.4}"))
                ));
            }
            Ok(out)
        }
        "context" => {
            let line = int("id").map_or_else(|| "/context".to_owned(), |id| format!("/context {id}"));
            Ok(commands::run(&line, store, worker).await?.0)
        }
        "gotchas" => Ok(gotchas(&agent::workspace_of(store))),
        "handoffs" => {
            let id = need_id()?;
            // the node is read for its existence: a missing card is an error, a card without
            // runs is an empty list
            store.read(id).await?;
            Ok(handoffs(id, &ra_context::TaskContext::open(&agent::workspace_of(store), id).handoffs))
        }
        other => anyhow::bail!("no tool {other}"),
    }
}

/// The gotchas of `root` by how often they fired, and the refused commands none names yet: the
/// lines `rc gotchas` prints.
fn gotchas(root: &std::path::Path) -> Vec<String> {
    let catalogue = ra_gotchas::Catalogue::load(&root.join("var/gotchas.toml")).ok();
    let journal_path = root.join("var/gotchas.log");
    let journal = ra_gotchas::Journal::at(&journal_path);
    let counts = journal.counts();
    if counts.is_empty() {
        let held = catalogue.as_ref().map_or(0, |c| c.entries.len());
        return vec![format!("no gotcha has fired yet; the catalogue holds {held} entries in var/gotchas.toml")];
    }
    let mut out = vec![format!("{:<26} {:>5}  message", "gotcha", "hits")];
    for (id, count) in &counts {
        let message =
            catalogue.as_ref().and_then(|c| c.get(id)).map_or("a refused command no gotcha names; see below", |g| g.message.as_str());
        out.push(format!("{id:<26} {count:>5}  {message}"));
    }
    let unmatched = journal.unmatched();
    if !unmatched.is_empty() {
        out.push("refused commands no gotcha names, newest last:".to_owned());
        out.extend(unmatched.iter().rev().take(20).rev().map(|command| format!("  {command}")));
    }
    out
}

/// The handoffs of card `id` as lines: a header with the count, then each run's block.
fn handoffs(id: i64, handoffs: &[ra_context::Handoff]) -> Vec<String> {
    let mut out = vec![format!("#{id} · {} handoff(s)", handoffs.len())];
    for handoff in handoffs {
        out.push(format!("## Handoff {} · {} · {}", handoff.when, handoff.model, handoff.outcome));
        out.push(format!("check: {}", handoff.check.trim()));
        if !handoff.changed.is_empty() {
            out.push(format!("changed: {}", handoff.changed.join(" ")));
        }
        out.extend(handoff.report.lines().map(str::to_owned));
        if !handoff.warnings.is_empty() {
            out.push("### Warnings".to_owned());
            out.extend(handoff.warnings.iter().cloned());
        }
    }
    out
}

#[cfg(test)]
mod tests;
