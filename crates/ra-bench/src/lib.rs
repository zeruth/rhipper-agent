//! The workflow benchmark: a small project, its cards and their hidden checks, embedded in the
//! binary so an agent always runs the benchmark of its own commit, even with no repository at
//! hand. Every module the benchmark needs is declared here once; later work fills them in.

pub mod bundle;
pub mod check;
pub mod compare;
pub mod env;
pub mod ledger;
pub mod observe;
pub mod pin;
pub mod project;
pub mod record;
pub mod scan;
pub mod setup;
pub mod verdict;

pub use bundle::Bundle;
