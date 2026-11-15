//! What a check's output means, read without running anything, and the ledger of every check
//! a project ran.

pub mod ledger;
pub mod output;

pub use output::{Ran, parse};
