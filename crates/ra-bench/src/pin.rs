//! The pins of `bench.toml`: the hash of each card, taken over the card as parsed and its hidden
//! check, and the hash of the base, every other file of the project. A card's words or its
//! hidden check change its hash; a migration of the roadmap's syntax does not.

use std::collections::BTreeSet;

use ra_roadmap::model::{Node, NodeKind};
use serde::Deserialize;

use crate::Bundle;
use crate::bundle::hash_of;
use crate::project::DIR;

/// Where the manifest sits in the bundle.
pub const MANIFEST: &str = "bench.toml";
/// Where the roadmap of the cards sits in the bundle.
pub const ROADMAP: &str = "project/rhipper.md";

/// The manifest: the benchmark's version, the limits of a run and one entry a card.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub repeats: u32,
    pub max_usd: f64,
    pub max_minutes: u32,
    pub max_rounds: u32,
    /// The pinned hash of the base, once one is pinned.
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default, rename = "card")]
    pub cards: Vec<Card>,
}

/// One card's entry: the task it is, the key its hidden check and its records go by, its
/// version, the crate its hidden check lands in and the checks the walk derives for it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Card {
    pub id: i64,
    pub key: String,
    pub v: u32,
    #[serde(rename = "crate")]
    pub krate: String,
    pub check: Vec<String>,
    /// The pinned hash of the card, once one is pinned.
    #[serde(default)]
    pub sha: Option<String>,
}

/// The path of a card's hidden check in the bundle.
#[must_use]
pub fn hidden_path(key: &str) -> String {
    format!("hidden/{key}.rs")
}

/// The roadmap of the bundle, as the current parser reads it.
///
/// # Errors
/// When the bundle has no roadmap, it is not UTF-8 or the parser refuses it.
pub fn roadmap(bundle: &Bundle) -> Result<Vec<Node>, String> {
    let bytes = bundle.get(ROADMAP).ok_or_else(|| format!("no {ROADMAP} in the benchmark"))?;
    let text = std::str::from_utf8(bytes).map_err(|e| format!("{ROADMAP} is not UTF-8: {e}"))?;
    ra_roadmap::markdown::parse(text).map_err(|e| format!("{ROADMAP}: {e:#}"))
}

/// The hash of one card: its title, objective, acceptance, files and waits-on as parsed, and the
/// bytes of its hidden check when there is one.
#[must_use]
pub fn card_hash(node: &Node, hidden: Option<&[u8]>) -> String {
    let files = node.files.join("\n");
    let waits: Vec<String> = node.depends_on.iter().map(i64::to_string).collect();
    let waits = waits.join(" ");
    let mut fields: Vec<(&str, &[u8])> = vec![
        ("title", node.title.as_bytes()),
        ("objective", node.objective.as_deref().unwrap_or_default().as_bytes()),
        ("acceptance", node.acceptance.as_deref().unwrap_or_default().as_bytes()),
        ("files", files.as_bytes()),
        ("waits-on", waits.as_bytes()),
    ];
    if let Some(hidden) = hidden {
        fields.push(("hidden", hidden));
    }
    hash_of(fields)
}

/// The hash of the card `card` names in `nodes`, with its hidden check from `bundle`; `None`
/// when the roadmap has no such task.
#[must_use]
pub fn card_hash_in(bundle: &Bundle, nodes: &[Node], card: &Card) -> Option<String> {
    let node = nodes.iter().find(|n| n.id == card.id && n.kind == NodeKind::Task)?;
    Some(card_hash(node, bundle.get(&hidden_path(&card.key))))
}

/// The hash of the base: every file of the project but its roadmap, so that a card's edit moves
/// only that card's pin.
#[must_use]
pub fn base_hash(bundle: &Bundle) -> String {
    hash_of(
        bundle
            .paths()
            .filter(|path| path.starts_with(DIR) && *path != ROADMAP)
            .filter_map(|path| bundle.get(path).map(|bytes| (path, bytes))),
    )
}

/// Reads the manifest `text` against the roadmap's `nodes`.
///
/// # Errors
/// When the text is not a manifest, or its cards are not the roadmap's tasks one for one: an
/// entry names no task, two entries name one, or a task has no entry.
pub fn read(text: &str, nodes: &[Node]) -> Result<Manifest, String> {
    let manifest: Manifest = toml::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let tasks: BTreeSet<i64> = nodes.iter().filter(|n| n.kind == NodeKind::Task).map(|n| n.id).collect();
    let mut named = BTreeSet::new();
    for card in &manifest.cards {
        if !tasks.contains(&card.id) {
            return Err(format!("{MANIFEST}: card {} ({}) is not a task of the roadmap", card.id, card.key));
        }
        if !named.insert(card.id) {
            return Err(format!("{MANIFEST}: card {} is named twice", card.id));
        }
    }
    let missing: Vec<String> = tasks.difference(&named).map(i64::to_string).collect();
    if !missing.is_empty() {
        return Err(format!("{MANIFEST}: the roadmap's task {} has no card", missing.join(", ")));
    }
    Ok(manifest)
}

/// The manifest of the bundle, read against the bundle's roadmap as [`read`] reads it.
///
/// # Errors
/// As [`roadmap`] and [`read`], and when the bundle has no manifest or it is not UTF-8.
pub fn manifest(bundle: &Bundle) -> Result<Manifest, String> {
    let nodes = roadmap(bundle)?;
    let bytes = bundle.get(MANIFEST).ok_or_else(|| format!("no {MANIFEST} in the benchmark"))?;
    let text = std::str::from_utf8(bytes).map_err(|e| format!("{MANIFEST} is not UTF-8: {e}"))?;
    read(text, &nodes)
}

#[cfg(test)]
mod tests;
