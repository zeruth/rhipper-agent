//! The roadmap engine. It holds what needs to be done and who did what: projects, milestones,
//! sections, tasks and subtasks, with dependencies, acceptance criteria, execution states and
//! worker assignments. The store is the `rhipper.md` file of the project; nothing here talks to a
//! model provider.
//!
//! The roadmap owns the work. Models are replaceable workers. The orchestrator owns coordination
//! and acceptance; it lives in another crate.

pub mod check;
pub mod contract;
pub mod history;
pub mod markdown;
pub mod missing_machinery;
pub mod model;
pub mod outline;
pub mod store;
pub mod tree;

#[cfg(test)]
mod tests;
