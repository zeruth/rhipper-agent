//! The tests of the CLI's pure parts: what `rc show` prints, the card lines of `rc status --all`,
//! the doctor's probe of the local route against a stub server.

use std::io::{Read, Write};
use std::net::TcpListener;

use super::*;
use ra_context::{Handoff, TaskContext};
use ra_usage::{CardCost, Totals};

/// A server that answers the requests in turn, one connection each, with the status and body
/// given; it reads each request whole before answering, as a closing server mid-request resets
/// the client.
fn serve(answers: Vec<(u16, &'static str)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let url = format!("http://127.0.0.1:{}/v1", listener.local_addr().expect("an address").port());
    std::thread::spawn(move || {
        for (status, body) in answers {
            let Ok((mut socket, _)) = listener.accept() else { return };
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = socket.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
        }
    });
    url
}

const MODELS: &str = r#"{"object":"list","data":[{"id":"qwen3-8b"},{"id":"llama-3.2-3b"}]}"#;

#[tokio::test]
async fn the_doctor_names_the_answering_local_model_and_its_latency() {
    let url = serve(vec![
        (200, MODELS),
        (200, r#"{"choices":[{"message":{"role":"assistant","content":"ok"}}],"usage":{"prompt_tokens":5,"completion_tokens":1}}"#),
    ]);
    let outcome = ra_providers::local::probe_at(&url).await;
    let probe = outcome.clone().expect("the server answers");
    assert_eq!(probe.model, "qwen3-8b");
    assert!(probe.latency < std::time::Duration::from_secs(10));
    let line = doctor::local_line(&outcome);
    assert!(line.starts_with("✓  local route     answering: qwen3-8b in "), "{line}");
    assert!(line.ends_with("ms") || line.ends_with('s'), "{line}");
}

#[tokio::test]
async fn the_doctor_prints_why_the_local_route_refused() {
    // a server that lists a model but cannot complete on it
    let url = serve(vec![(200, MODELS), (500, r#"{"error":{"message":"model not loaded"}}"#)]);
    let outcome = ra_providers::local::probe_at(&url).await;
    let line = doctor::local_line(&outcome);
    assert!(line.starts_with("–  local route     refused: qwen3-8b refused the one-token request: "), "{line}");
    assert!(line.contains("model not loaded"), "{line}");
    // a server with nothing loaded, and no server at all
    let empty = serve(vec![(200, r#"{"object":"list","data":[]}"#)]);
    assert_eq!(ra_providers::local::probe_at(&empty).await, Err(format!("{empty} answers but lists no model")));
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let closed = format!("http://127.0.0.1:{}/v1", listener.local_addr().expect("an address").port());
    drop(listener);
    let line = doctor::local_line(&ra_providers::local::probe_at(&closed).await);
    assert!(line.starts_with("–  local route     refused: provider unavailable: cannot reach "), "{line}");
}

fn roadmap() -> Vec<ra_roadmap::model::Node> {
    markdown::parse(
        "- 1 ‹project› Rhipper {c3} [defined]\n\
         \x20   - 2 ‹milestone› Phase A {c3} [defined]\n\
         \x20       - 3 ‹task› First {c2} [implemented]\n\
         \x20           objective: do the thing\n\
         \x20           files: src/a.rs\n\
         \x20       - 4 ‹task› Second {c2} [defined]\n\
         \x20           waits-on: 3\n",
    )
    .expect("a roadmap")
}

/// A context file for `id` whose last run carries `verdict`.
fn judged(root: &Path, id: i64, verdict: &str) {
    let mut context = TaskContext::open(root, id);
    context.record(Handoff {
        when: 1,
        model: "m".to_owned(),
        outcome: "implemented".to_owned(),
        check: "cargo test -q · passed".to_owned(),
        verdict: Some(verdict.to_owned()),
        changed: vec!["src/a.rs".to_owned()],
        report: "done".to_owned(),
        warnings: Vec::new(),
    });
    context.save(root).expect("the context saves");
}

#[test]
fn show_of_a_card_prints_its_verdict_under_the_title() {
    let dir = tempfile::tempdir().expect("a tempdir");
    judged(dir.path(), 3, "accepted by reviewer-model");
    let lines = show_lines(&roadmap(), Some(2), dir.path()).expect("the subtree");
    assert_eq!(
        lines,
        [
            "  #2 Phase A [defined]",
            "    #3 First [implemented]",
            "        verdict: accepted by reviewer-model",
            "        do the thing",
            "        files: src/a.rs",
            "    #4 Second [defined]",
            "        waits on: #3",
        ]
    );
}

#[test]
fn show_of_the_roadmap_leaves_the_verdicts_out_and_a_missing_node_is_named() {
    let dir = tempfile::tempdir().expect("a tempdir");
    judged(dir.path(), 3, "needs revision (trivial) · 2 point(s) by reviewer-model");
    let lines = show_lines(&roadmap(), None, dir.path()).expect("the roadmap");
    assert!(lines.iter().all(|l| !l.contains("verdict:")), "{lines:?}");
    assert_eq!(lines.len(), 7);
    let missing = show_lines(&roadmap(), Some(9), dir.path()).expect_err("no node 9");
    assert_eq!(missing.to_string(), "no node 9");
}

#[test]
fn a_card_without_a_judged_run_shows_no_verdict() {
    let dir = tempfile::tempdir().expect("a tempdir");
    let lines = show_lines(&roadmap(), Some(4), dir.path()).expect("the card");
    assert_eq!(lines, ["    #4 Second [defined]", "        waits on: #3"]);
}

#[test]
fn plan_without_an_id_is_the_evidence_plan_and_with_one_the_split_and_dry_takes_no_id() {
    let Options { command: Command::Plan { id, yes, dry, under } } = Options::try_parse_from(["rc", "plan", "--dry"]).expect("parses")
    else {
        panic!("not a plan")
    };
    assert_eq!((id, yes, dry, under), (None, false, true, None));
    let Options { command: Command::Plan { id, yes, dry, under } } =
        Options::try_parse_from(["rc", "plan", "--under", "9"]).expect("parses")
    else {
        panic!("not a plan")
    };
    assert_eq!((id, yes, dry, under), (None, false, false, Some(9)));
    let Options { command: Command::Plan { id, yes, dry, under } } = Options::try_parse_from(["rc", "plan", "4", "-y"]).expect("parses")
    else {
        panic!("not a plan")
    };
    assert_eq!((id, yes, dry, under), (Some(4), true, false, None));
    // a dry run plans from the evidence; a split has nothing dry about it
    assert!(Options::try_parse_from(["rc", "plan", "4", "--dry"]).is_err());
}

#[test]
fn a_status_card_line_carries_the_cost_and_the_tokens_a_turn_and_is_blank_before_a_run() {
    let nodes = roadmap();
    let ran = CardCost {
        node: 3,
        totals: Totals { calls: 2, prompt_tokens: 100_000, completion_tokens: 23_000, cached_tokens: 0, cost_usd: Some(0.4249), turns: 10 },
    };
    let lines = status::card_lines(&nodes, &[ran]);
    assert_eq!(lines, ["  #3 First [implemented]  $0.42 · 12.3k tok/turn"]);
    // the defined card is not in hand, so it has no line; an unseen card in hand gets a blank one
    let mut nodes = nodes;
    nodes[3].state = TaskState::NeedsRevision;
    let lines = status::card_lines(&nodes, &[ran]);
    assert_eq!(lines[1], "  #4 Second [needs_revision]");
    // a card the ledger saw without a cost reported still tells its tokens
    let free = CardCost { node: 4, totals: Totals { calls: 1, prompt_tokens: 900, completion_tokens: 50, turns: 1, ..Totals::default() } };
    assert_eq!(status::card_line(&nodes[3], Some(&free)), "  #4 Second [needs_revision]  $— · 950 tok/turn");
}
