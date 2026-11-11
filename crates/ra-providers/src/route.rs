//! The cheap routes. A [`local::Role`](crate::local::Role) is one call with no tools, and
//! [`role`] picks its model: the `[role]` table's, else the local server's, else none, since a
//! missing server must degrade to no call and never to the frontier model. A fix pass on a
//! trivially rejected card (a lint warning, a format diff, a doc comment) needs the files and
//! the checks, but no frontier model either; [`cheap`] picks its model the same way, except that
//! the smallest Claude on the subscription takes it when no server answers.

use crate::local::default_model;

/// The smallest Claude: what a fix pass falls to when no local server answers.
pub const HAIKU: &str = "claude-haiku-4-5-20251001";

/// The model of a no-tools role: `named` when the table names one (`Some("")` turns the role
/// off), else `local` when the server answered, else `None`: the role makes no call.
#[must_use]
pub fn role(named: Option<&str>, local: Option<&str>) -> Option<String> {
    match named.map(str::trim) {
        Some("") => None,
        Some(model) => Some(model.to_owned()),
        None => local.map(str::to_owned),
    }
}

/// The model of a fix pass: `named` when the table names one (`Some("")` turns the pass off),
/// else `local` when the server answered, else [`HAIKU`].
#[must_use]
pub fn cheap(named: Option<&str>, local: Option<&str>) -> Option<String> {
    match named.map(str::trim) {
        Some("") => None,
        Some(model) => Some(model.to_owned()),
        None => Some(local.unwrap_or(HAIKU).to_owned()),
    }
}

/// [`cheap`], pinging the local server for the default when the table names nothing.
pub async fn cheap_model(named: Option<&str>) -> Option<String> {
    match named.map(str::trim) {
        Some("") => None,
        Some(model) => Some(model.to_owned()),
        None => Some(default_model().await.unwrap_or_else(|| HAIKU.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fix_pass_takes_the_named_model_else_the_local_one_else_haiku_and_an_empty_name_turns_it_off() {
        assert_eq!(cheap(Some("z-ai/glm-5.3-flash"), Some("local/qwen3-8b")).as_deref(), Some("z-ai/glm-5.3-flash"));
        assert_eq!(cheap(None, Some("local/qwen3-8b")).as_deref(), Some("local/qwen3-8b"));
        assert_eq!(cheap(None, None).as_deref(), Some(HAIKU));
        assert_eq!(cheap(Some(" "), Some("local/qwen3-8b")), None);
    }

    #[test]
    fn a_no_tools_role_takes_the_named_model_else_the_local_one_and_makes_no_call_without_a_server() {
        assert_eq!(role(Some("z-ai/glm-5.3-flash"), Some("local/qwen3-8b")).as_deref(), Some("z-ai/glm-5.3-flash"));
        assert_eq!(role(None, Some("local/qwen3-8b")).as_deref(), Some("local/qwen3-8b"));
        // no server: no call, and never haiku
        assert_eq!(role(None, None), None);
        assert_eq!(role(Some(""), Some("local/qwen3-8b")), None);
    }
}
