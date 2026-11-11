//! The cheap routes. A [`local::Role`](crate::local::Role) is one call with no tools, and
//! [`role`] picks its model: the `[role]` table's, else the local server's, else none, since a
//! missing server must degrade to no call and never to the frontier model. A fix pass on a
//! trivially rejected card (a lint warning, a format diff, a doc comment) needs the files and
//! the checks, but no frontier model either; [`cheap`] picks its model the same way, except that
//! the smallest Claude on the subscription takes it when no server answers. [`TABLE`] lists the
//! roles by their `[role]` key with what each falls to, and [`routed`] resolves one by name: the
//! look-back (`retro`) and the digest only summarise, so they take the local server whenever it
//! answers and the run's default model only when it does not.

use crate::local::default_model;

/// The smallest Claude: what a fix pass falls to when no local server answers.
pub const HAIKU: &str = "claude-haiku-4-5-20251001";

/// What a role falls to when the table names nothing and no local server answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// No call: the rule-based path stands.
    Silent,
    /// The smallest Claude on the subscription.
    Haiku,
    /// The run's default model, the one the card runs on.
    Default,
}

/// The role table: each cheap role by its `[role]` key, with where it falls without a server.
/// The roles that only summarise (`retro`, `digest`) still get an answer from the default route;
/// the ones whose rule-based path is as good (`summary`, `classify`) make no call.
pub const TABLE: &[(&str, Fallback)] = &[
    ("summary", Fallback::Silent),
    ("digest", Fallback::Default),
    ("classify", Fallback::Silent),
    ("retro", Fallback::Default),
    ("fix", Fallback::Haiku),
];

/// The fallback of `role` in [`TABLE`]; a role the table does not know falls to the default.
#[must_use]
pub fn fallback(role: &str) -> Fallback {
    TABLE.iter().find(|(name, _)| *name == role).map_or(Fallback::Default, |(_, fallback)| *fallback)
}

/// The model of `role`: `named` when the table names one (`Some("")` turns the role off), else
/// `local` when the server answered, else what [`fallback`] says, with `default` the run's model.
#[must_use]
pub fn routed(role: &str, named: Option<&str>, local: Option<&str>, default: &str) -> Option<String> {
    match named.map(str::trim) {
        Some("") => None,
        Some(model) => Some(model.to_owned()),
        None => local.map(str::to_owned).or_else(|| match fallback(role) {
            Fallback::Silent => None,
            Fallback::Haiku => Some(HAIKU.to_owned()),
            Fallback::Default => Some(default.to_owned()),
        }),
    }
}

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
