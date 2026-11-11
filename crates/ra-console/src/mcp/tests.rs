//! The tests of the MCP server: the handshake, the tool table and the roadmap tools, over the
//! message handler and a temp store. No test talks to a model.

use super::*;

async fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhipper.md"), "Rhipper").await.unwrap();
    (dir, store)
}

async fn send(store: &Store, worker: &mut Worker, message: Value) -> Option<Value> {
    handle(&message.to_string(), store, worker).await
}

fn call(id: i64, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
}

fn text_of(reply: &Value) -> &str {
    reply["result"]["content"][0]["text"].as_str().unwrap()
}

#[tokio::test]
async fn the_handshake_names_the_server_and_notifications_get_no_reply() {
    let (_dir, store) = store().await;
    let mut worker = Worker::new("claude-opus-5-5");
    let reply =
        send(&store, &mut worker, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}))
            .await
            .unwrap();
    assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(reply["result"]["serverInfo"]["name"], "rhipper");
    assert!(send(&store, &mut worker, json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await.is_none());
    let reply = send(&store, &mut worker, json!({"jsonrpc": "2.0", "id": 2, "method": "nope"})).await.unwrap();
    assert_eq!(reply["error"]["code"], -32601);
    let reply = handle("{not json", &store, &mut worker).await.unwrap();
    assert_eq!(reply["error"]["code"], -32700);
}

#[tokio::test]
async fn the_tool_table_lists_every_tool_with_a_schema() {
    let (_dir, store) = store().await;
    let mut worker = Worker::new("claude-opus-5-5");
    let reply = send(&store, &mut worker, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await.unwrap();
    let names: Vec<&str> = reply["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["show", "add", "details", "set", "depend", "run", "reassess", "model", "ping", "usage"]);
    assert!(reply["result"]["tools"].as_array().unwrap().iter().all(|t| t["inputSchema"]["type"] == "object"));
}

#[tokio::test]
async fn the_roadmap_tools_shape_the_tree() {
    let (_dir, store) = store().await;
    let mut worker = Worker::new("claude-opus-5-5");
    let reply =
        send(&store, &mut worker, call(1, "add", json!({"kind": "task", "title": "Decode nop", "parent": 1, "objective": "one byte"})))
            .await
            .unwrap();
    assert_eq!(text_of(&reply), "#2 Decode nop");
    let reply =
        send(&store, &mut worker, call(2, "details", json!({"id": 2, "acceptance": "`cargo test` passes", "model": "z-ai/glm-5.3-flash"})))
            .await
            .unwrap();
    assert_eq!(text_of(&reply), "#2: details, model set");
    let node = store.read(2).await.unwrap();
    assert_eq!(node.acceptance.as_deref(), Some("`cargo test` passes"));
    assert_eq!(node.assignee.as_ref().map(|a| a.model.as_str()), Some("z-ai/glm-5.3-flash"));
    let reply = send(&store, &mut worker, call(3, "set", json!({"id": 2, "state": "ready"}))).await.unwrap();
    assert_eq!(text_of(&reply), "#2 is now ready");
    let reply = send(&store, &mut worker, call(4, "show", json!({}))).await.unwrap();
    assert!(text_of(&reply).contains("#2 Decode nop [ready] ← implementer:z-ai/glm-5.3-flash"), "{}", text_of(&reply));
    // a bad call is an error answer, not a protocol error
    let reply = send(&store, &mut worker, call(5, "add", json!({"kind": "epic", "title": "x"}))).await.unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert!(text_of(&reply).starts_with("the kind is"), "{}", text_of(&reply));
}

#[tokio::test]
async fn the_add_tool_takes_the_files_the_model_and_the_dependencies() {
    let (_dir, store) = store().await;
    let mut worker = Worker::new("claude-opus-5-5");
    let reply = send(&store, &mut worker, call(1, "add", json!({"kind": "task", "title": "A", "parent": 1}))).await.unwrap();
    assert_eq!(text_of(&reply), "#2 A");
    let args = json!({"kind": "task", "title": "B", "parent": 1, "files": ["src/b.rs"], "waits_on": [2], "model": "local/qwen"});
    let reply = send(&store, &mut worker, call(2, "add", args)).await.unwrap();
    assert_eq!(text_of(&reply), "#3 B");
    let node = store.read(3).await.unwrap();
    assert_eq!(node.files, ["src/b.rs"]);
    assert_eq!(node.depends_on, [2]);
    assert_eq!(node.assignee.as_ref().map(|a| a.model.as_str()), Some("local/qwen"));
    // a dependency on a card that is not there is refused, and nothing is added
    let reply =
        send(&store, &mut worker, call(3, "add", json!({"kind": "task", "title": "C", "parent": 1, "waits_on": [9]}))).await.unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert!(text_of(&reply).contains("no node 9"), "{}", text_of(&reply));
    assert_eq!(store.all().await.unwrap().len(), 3);
}

#[tokio::test]
async fn the_model_tool_swaps_the_badge() {
    let (_dir, store) = store().await;
    let mut worker = Worker::new("claude-opus-5-5");
    let reply = send(&store, &mut worker, call(1, "model", json!({"id": "z-ai/glm-5.3-flash"}))).await.unwrap();
    assert_eq!(text_of(&reply), "the badge model is z-ai/glm-5.3-flash (OpenRouter)");
    assert_eq!(worker.model, "z-ai/glm-5.3-flash");
    let reply = send(&store, &mut worker, call(2, "model", json!({}))).await.unwrap();
    assert!(text_of(&reply).contains("z-ai/glm-5.3-flash"));
}
