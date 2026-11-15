use ra_console::agent::Route;
use ra_console::route::choose;

#[test]
fn a_route_with_its_key_is_kept_and_named() {
    let chosen = choose("z-ai/glm-5.3-flash", |_| true);
    assert_eq!(chosen.route, Route::OpenRouter);
    assert_eq!(chosen.model, "z-ai/glm-5.3-flash");
    assert_eq!(chosen.line(), "route: OpenRouter · z-ai/glm-5.3-flash");
    assert!(chosen.note().is_none());
}

#[test]
fn a_missing_key_falls_back_to_claude_code_before_the_worker_starts() {
    let chosen = choose("z-ai/glm-5.3-flash", |route| route != Route::OpenRouter);
    assert_eq!(chosen.route, Route::ClaudeCode);
    assert_eq!(Route::of(&chosen.model), Route::ClaudeCode);
    assert_eq!(chosen.fell_back_from.as_deref(), Some("z-ai/glm-5.3-flash"));
    assert!(chosen.line().starts_with("route: Claude Code · "));
    assert!(chosen.line().contains("fell back from z-ai/glm-5.3-flash"));
    assert!(chosen.note().is_some());
}

#[test]
fn claude_code_needs_no_key() {
    let chosen = choose("claude-sonnet-5-5", |route| route != Route::OpenRouter);
    assert_eq!(chosen.route, Route::ClaudeCode);
    assert_eq!(chosen.model, "claude-sonnet-5-5");
    assert!(chosen.fell_back_from.is_none());
}
