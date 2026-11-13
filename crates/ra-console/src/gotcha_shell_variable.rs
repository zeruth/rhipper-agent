//! The `shell-variable` gotcha: a command the hook refused because a variable in it, `$?` most
//! often, is only known to the shell that runs it.

use ra_gotchas::{Catalogue, Hit, Where};

/// The gotcha's id in the catalogue.
pub const ID: &str = "shell-variable";

/// The hit a refused command's `result` earns from the seed catalogue, when it is this refusal.
#[must_use]
pub fn hit(result: &str) -> Option<Hit> {
    Catalogue::seed().scan(Where::Result, result).into_iter().find(|h| h.id == ID)
}
