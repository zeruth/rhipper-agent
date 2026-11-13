//! The cheap roles of a run: the summary of the worker's report, the run's line in the digest,
//! and whether an untagged look-back is the machine's. Each is a [`Role`] with no tools, routed
//! by [`ra_providers::route::role`]: the `[role]` table's model, else the local server when it
//! answers a ping, else no call. Every call lands in the ledger under the model that took it,
//! tagged with its role, so `rc usage` lists the roles' calls beside the worker's.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ra_providers::local::Role;
use ra_providers::{ModelProvider, ProviderError};

use crate::agent::Worker;
use crate::router::RoleConfig;

/// What the summary role is told: the report condensed for the next run of the same card.
pub(crate) const SUMMARY_INSTRUCTION: &str = "Condense this worker's report for the next run of the same card, in at most eight plain lines and no heading: what changed (which files), what was verified, what is left or failed. Keep the exact names, paths and commands; drop everything else.";

/// What the digest role is told: the run's line for the other projects on the machine.
pub(crate) const DIGEST_INSTRUCTION: &str = "In one line of at most 120 characters and no markdown, say what this run did for the people and the agents working on other projects on the same machine: the card, the outcome, and the crates or files it touched. The first line is how the run ended; the rest are the notes of the card.";

/// The cheap roles of a run, each with the model that takes it and the provider behind it; a
/// role with none runs its rule-based path and calls nothing. The local server answered a ping
/// when the run began; once a call finds it gone, every role left in the run is off too, with
/// no call and no frontier model in its place.
#[derive(Clone, Default)]
pub struct Roles {
    routes: Vec<(Role, String, Arc<dyn ModelProvider>)>,
    down: Arc<AtomicBool>,
}

impl Roles {
    /// Every role on `model` through `provider`; the tests hand in a scripted local one.
    #[must_use]
    pub fn preset(model: impl Into<String>, provider: Arc<dyn ModelProvider>) -> Self {
        let model = model.into();
        let shared = std::iter::repeat_n(provider, Role::ALL.len());
        Self {
            routes: Role::ALL.into_iter().zip(shared).map(|(role, provider)| (role, model.clone(), provider)).collect(),
            down: Arc::default(),
        }
    }

    /// The model of each role that has one: the table's word, else `local` when the server
    /// answered, else none; the roles the table turns off are left out.
    #[must_use]
    pub fn resolve(config: &RoleConfig, local: Option<&str>) -> Vec<(Role, String)> {
        Role::ALL.into_iter().filter_map(|role| ra_providers::route::role(config.model(role), local).map(|model| (role, model))).collect()
    }

    /// The model and the provider of `role`, when it has one.
    pub(crate) fn of(&self, role: Role) -> Option<(&str, &Arc<dyn ModelProvider>)> {
        self.routes.iter().find(|(r, _, _)| *r == role).map(|(_, model, provider)| (model.as_str(), provider))
    }

    /// The model of `role`, when it has one.
    #[must_use]
    pub fn model(&self, role: Role) -> Option<&str> {
        self.of(role).map(|(model, _)| model)
    }

    /// The resolution of every role for the start line, once per run: `roles: summary
    /// local/qwen3-8b · digest off · classify off`, so the ledger's rows are read against what
    /// the run said it would call.
    #[must_use]
    pub fn summary(&self) -> String {
        let parts: Vec<String> =
            Role::ALL.into_iter().map(|role| format!("{} {}", role.name(), self.model(role).unwrap_or("off"))).collect();
        format!("roles: {}", parts.join(" · "))
    }
}

impl Worker {
    /// The cheap roles of a run in `root`: the ones a test set, else each as the `[role]` table
    /// of `rhipper.toml` names it, with a key left out going to the local server when it answers
    /// a ping (once per run, whatever the number of roles) and to no model at all when it does
    /// not. A role whose model has no provider is off, with a warning; so is one that resolved
    /// to a Claude model the table did not name, since the cheap roles never fall to one.
    pub(crate) async fn roles(&self, root: &Path) -> Roles {
        if let Some(preset) = &self.roles {
            return preset.clone();
        }
        let config = match crate::router::Router::load(root) {
            Ok(router) => router.role,
            Err(e) => {
                tracing::warn!("the role table is off: {e:#}");
                RoleConfig::default()
            }
        };
        // the local default, pinged only when a role is left to it; the tests never ping
        let needs_local = Role::ALL.into_iter().any(|role| config.model(role).is_none());
        let local = if needs_local && !cfg!(test) { ra_providers::local::default_model().await } else { None };
        let mut roles = Roles::default();
        for (role, model) in Roles::resolve(&config, local.as_deref()) {
            if crate::agent::Route::of(&model) == crate::agent::Route::ClaudeCode && config.model(role).is_none() {
                tracing::warn!("the {} role is off: {model} is a Claude model the [role] table did not name", role.name());
                continue;
            }
            match self.provider_for(&model) {
                Ok(provider) => roles.routes.push((role, model, provider)),
                Err(e) => tracing::warn!("the {} role is off: {e:#}", role.name()),
            }
        }
        roles
    }

    /// One call of `role` on its model, booked under that model and the role's tag against
    /// `node`; `None` when the role has no model, the text is empty or the call failed, and the
    /// rule-based path stands. A server found unreachable or silent is logged once and asked
    /// nothing more this run.
    pub(crate) async fn ask_role(&self, roles: &Roles, role: Role, node: Option<i64>, instruction: &str, text: &str) -> Option<String> {
        let (model, provider) = roles.of(role)?;
        if text.trim().is_empty() || roles.down.load(Ordering::Relaxed) {
            return None;
        }
        match ra_providers::local::ask(provider.as_ref(), model, role, instruction, text).await {
            Ok((answer, usage)) => {
                self.record_as(model, Some(role.name()), node, &usage, 1).await;
                (!answer.is_empty()).then_some(answer)
            }
            Err(ProviderError::Unavailable(reason)) => {
                roles.down.store(true, Ordering::Relaxed);
                tracing::warn!("{model} does not answer, the cheap roles are off for this run: {reason}");
                None
            }
            Err(e) => {
                tracing::warn!("the {} role failed on {model}: {e}", role.name());
                None
            }
        }
    }
}
