use crate::route::{self, Fallback, HAIKU};

const DEFAULT: &str = "claude-fable-5-1";

#[test]
fn the_retro_role_goes_to_the_local_provider_when_it_answers_and_to_the_default_when_it_is_absent() {
    // the server answered: the look-back's role takes it, whatever the default
    assert_eq!(route::routed("retro", None, Some("local/qwen3-8b"), DEFAULT).as_deref(), Some("local/qwen3-8b"));
    // no server: the role falls to the run's default, never to no call
    assert_eq!(route::routed("retro", None, None, DEFAULT).as_deref(), Some(DEFAULT));
    // the table's word wins either way, and an empty one turns the role off
    assert_eq!(route::routed("retro", Some("z-ai/glm-5.3-flash"), Some("local/qwen3-8b"), DEFAULT).as_deref(), Some("z-ai/glm-5.3-flash"));
    assert_eq!(route::routed("retro", Some(" "), Some("local/qwen3-8b"), DEFAULT), None);
}

#[test]
fn the_table_sends_the_digest_to_the_local_server_and_keeps_each_roles_own_fallback() {
    assert_eq!(route::routed("digest", None, Some("local/qwen3-8b"), DEFAULT).as_deref(), Some("local/qwen3-8b"));
    assert_eq!(route::routed("digest", None, None, DEFAULT).as_deref(), Some(DEFAULT));
    // the summary and the classify have a rule-based path as good: no call without a server
    assert_eq!(route::routed("summary", None, None, DEFAULT), None);
    assert_eq!(route::routed("classify", None, None, DEFAULT), None);
    // the fix pass keeps haiku, and a role the table does not know falls to the default
    assert_eq!(route::routed("fix", None, None, DEFAULT).as_deref(), Some(HAIKU));
    assert_eq!(route::fallback("unknown"), Fallback::Default);
    assert_eq!(route::TABLE.iter().map(|(name, _)| *name).collect::<Vec<_>>(), ["summary", "digest", "classify", "retro", "fix"]);
}
