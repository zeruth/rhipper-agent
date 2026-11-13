//! The agent's own planning: the next cards come from the runs' own evidence, not from a
//! person's list. [`evidence`] gathers the lines a planned card may cite, each naming where it
//! was read; a card without one is not planned. [`prompt`] numbers the lines and tells the
//! planner what a card must name; [`propose`] makes the call and judges the cards; [`home`]
//! says where each goes, the project's Look-back milestone or the agent's.

pub mod evidence;
pub mod home;
pub mod prompt;
pub mod propose;
pub mod split;

#[cfg(test)]
mod tests;
