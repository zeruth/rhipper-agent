//! The route a run will use, settled before the worker starts: a route whose key is missing
//! falls back to Claude Code, which needs only its own login, so the run does not die on its first
//! turn. The handoff names the outcome on a `route:` line.

use std::fmt::Write;

use crate::agent::{Route, resolve};

/// The model a run falls back to when its route has no key.
const FALLBACK: &str = "sonnet";

/// What a run will use: the model, the route it goes through, and the model it left when the key
/// of that route was missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    pub model: String,
    pub route: Route,
    /// The model first chosen, when the run fell back from it.
    pub fell_back_from: Option<String>,
}

impl Chosen {
    /// The `route:` line of the handoff.
    #[must_use]
    pub fn line(&self) -> String {
        let mut line = format!("route: {} · {}", self.route.name(), self.model);
        if let Some(from) = &self.fell_back_from {
            let _ = write!(line, " · fell back from {from}: no key");
        }
        line
    }

    /// The note a log line carries for a fallback; `None` when the run kept its route.
    #[must_use]
    pub fn note(&self) -> Option<String> {
        self.fell_back_from.as_ref().map(|from| format!("{from} has no key, fell back to {}", self.model))
    }
}

/// Whether `route` has what it needs to start: the `OpenRouter` key from the environment or the
/// credential store; Claude Code its login and the local server a listener, neither of which is
/// a key.
#[must_use]
pub fn key_present(route: Route) -> bool {
    match route {
        Route::OpenRouter => crate::secret::api_key().is_ok(),
        Route::ClaudeCode | Route::Local => true,
    }
}

/// The route of `model` when `has_key` says whether its key is there: the model's own, or Claude
/// Code's when the key is missing.
pub fn choose(model: &str, has_key: impl Fn(Route) -> bool) -> Chosen {
    let route = Route::of(model);
    if has_key(route) {
        return Chosen { model: model.to_owned(), route, fell_back_from: None };
    }
    Chosen { model: resolve(FALLBACK), route: Route::ClaudeCode, fell_back_from: Some(model.to_owned()) }
}
