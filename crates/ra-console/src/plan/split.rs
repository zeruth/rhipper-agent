//! A card the context cap ended [`CAP_RUNS`] times in a row is split, not continued again: the
//! planner reads the card and the last handoff and proposes smaller cards for the work that is
//! left, and the walk blocks the card with the proposal on the notes and the feed. An absolute
//! card is exempt, as it is from the walk's other limits.

use std::path::Path;

use ra_context::TaskContext;
use ra_providers::{CompletionRequest, Message};
use ra_roadmap::history::Source;
use ra_roadmap::model::{Effort, Node, TaskState};
use ra_roadmap::store::Store;

use crate::agent::{self, Worker};

/// The context-cap continuations in a row after which the next one is a split.
pub const CAP_RUNS: usize = 3;

/// What the planner is told when it splits a card the context cap keeps ending.
const INSTRUCTION: &str = "You split a development card that kept filling the worker's context before it finished. You get the card and the last handoff, which says what was done and what is left. Propose between 2 and 4 smaller cards for the work that is left only, each a change one worker finishes in one run, none repeating what was done. Answer with a plain outline and nothing else: one `- Title {cN}` line per card, N the complexity 1..3, followed by indented `objective:`, `acceptance:` and `files:` lines; the acceptance names the command that checks the card in backticks; the files are the paths the change lands in, separated by spaces. No headings, no prose.";

/// How many of the card's latest handoffs, in a row, are runs the context cap ended.
#[must_use]
pub fn cap_runs(task: &TaskContext) -> usize {
    task.handoffs.iter().rev().take_while(|h| h.outcome == "continues" && h.check.contains("the context cap")).count()
}

/// Whether the walk splits `node` instead of running it again: its last [`CAP_RUNS`] runs ended
/// on the context cap and it is not absolute.
#[must_use]
pub fn due(root: &Path, node: &Node) -> bool {
    !node.is_absolute() && cap_runs(&TaskContext::open(root, node.id)) >= CAP_RUNS
}

/// Asks the planner to split what is left of card `id` and blocks the card with the proposal in
/// its notes. The planner's model is the `planner` rule that fits the card, else the worker's
/// own. Returns the line the walk shows and the feed carries; a planner that fails leaves the
/// card blocked all the same, with the reason, for the person to split.
///
/// # Errors
/// The card cannot be read or blocked.
pub async fn split(store: &Store, root: &Path, id: i64, worker: &Worker) -> anyhow::Result<String> {
    let node = store.read(id).await?;
    let task = TaskContext::open(root, id);
    let handoff = task.handoffs.last().map_or("", |h| h.report.as_str());
    let proposal = match ask(root, worker, &node, handoff).await {
        Ok(outline) => format!("split proposed:\n{}", outline.trim()),
        Err(e) => format!("split by hand: the planner's proposal failed: {e:#}"),
    };
    store
        .set_notes(id, &format!("{CAP_RUNS} runs in a row ended on the context cap; the card is blocked, not continued again.\n{proposal}"))
        .await?;
    store.set_state_by(id, TaskState::Blocked, Source::Run).await?;
    let text = format!("split #{id} {}: {CAP_RUNS} runs ended on the context cap · {proposal}", node.title);
    crate::pair::event(root, &text);
    Ok(text)
}

/// The planner's outline for the card's remaining work.
async fn ask(root: &Path, worker: &Worker, node: &Node, handoff: &str) -> anyhow::Result<String> {
    let model = match crate::router::Router::load(root)?.pick(node, crate::router::PLANNER, agent::rejections(root, node.id)) {
        Some((_, rule)) => agent::resolve(&rule.model),
        None => worker.model.clone(),
    };
    let provider = worker.provider_for(&model)?;
    let brief = format!(
        "# Card #{} {}\nobjective: {}\nacceptance: {}\nfiles: {}\n\n# Last handoff\n{handoff}\n",
        node.id,
        node.title,
        node.objective.as_deref().unwrap_or("none"),
        node.acceptance.as_deref().unwrap_or("none"),
        node.files.join(" ")
    );
    let answer = provider
        .complete(CompletionRequest {
            model: model.clone(),
            messages: vec![Message::system(INSTRUCTION), Message::user(brief)],
            temperature: Some(0.2),
            max_tokens: Some(2000),
            tools: Vec::new(),
            workspace: None,
            budget_usd: None,
            resume: None,
            max_turns: Some(1),
            max_context: None,
            effort: Some(Effort::default().to_string()),
            read_only: false,
        })
        .await
        .map_err(agent::map_error)?;
    worker.record_as(&model, Some("planner"), Some(node.id), &answer.usage, answer.turns).await;
    anyhow::ensure!(!answer.text.trim().is_empty(), "the planner answered nothing");
    Ok(answer.text)
}
