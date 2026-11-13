//! A run's `TEMP` and `TMP` sit outside the worktree: a test that creates a project in temp must
//! not find the branch's tree above it.

use std::path::PathBuf;

use ra_providers::claude_code::ClaudeCode;
use ra_providers::{CompletionRequest, Message};

fn request(workspace: PathBuf) -> CompletionRequest {
    CompletionRequest {
        model: "claude-opus-5-5".to_owned(),
        messages: vec![Message::user("Reply with the single word: pong.")],
        temperature: None,
        max_tokens: None,
        tools: Vec::new(),
        workspace: Some(workspace),
        budget_usd: None,
        resume: None,
        max_turns: None,
        max_context: None,
        effort: None,
        read_only: false,
    }
}

#[test]
fn a_runs_temp_is_outside_its_worktree() {
    let root = tempfile::tempdir().unwrap();
    let worktree = root.path().join("var").join("worktrees").join("660");
    std::fs::create_dir_all(&worktree).unwrap();
    let env = ClaudeCode::new().environment(&request(worktree.clone()));
    for key in ["TEMP", "TMP"] {
        let (_, value) = env.iter().find(|(k, _)| k == key).unwrap_or_else(|| panic!("{key} is set: {env:?}"));
        let temp = PathBuf::from(value);
        assert!(temp.is_absolute(), "{key} is absolute: {value}");
        assert!(!temp.starts_with(&worktree), "{key} is outside the worktree: {value}");
        assert!(!temp.starts_with(root.path()), "{key} is outside the project: {value}");
    }
}
