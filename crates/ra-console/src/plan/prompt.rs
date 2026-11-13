//! What the planner is told and what it reads: the instruction that asks for small cards, each
//! citing the evidence line it comes from and naming what it saves and the check that proves
//! it, and the brief that numbers the evidence lines so a card can cite one.

use std::fmt::Write;

use super::evidence::Line;

/// The fewest cards a plan holds; fewer is a refusal.
pub const MIN_CARDS: usize = 4;

/// The most cards a plan holds; more is a refusal, the planner did not pick.
pub const MAX_CARDS: usize = 8;

/// What the planner is told: the cards come from the numbered evidence and from nothing else,
/// as a plain outline with two fields of its own beside the store's three.
pub const INSTRUCTION: &str = "You plan the next cards of a development agent from the evidence of its own runs: the numbered lines you get are the costliest cards of the last day, the cards that needed more than one run, the look-backs, the refused commands and what the other projects on the machine are doing. Propose between 4 and 8 small cards, each a change one worker finishes in one run under the turn cap. Answer with a plain outline and nothing else: one `- Title {cN}` line per card, N the complexity 1..3, followed by indented `evidence:`, `saves:`, `objective:`, `acceptance:` and `files:` lines. `evidence:` names the numbers of the lines the card comes from, as `#3 #5`; `saves:` says what the card saves in one phrase: turns, tokens, a refused command, a person's intervention; the acceptance is the command that checks the card in backticks and what it prints, nothing a reviewer must judge in prose; what only a person, CI or an emulator can give goes on a `person:` line, which the window shows as a note and no review asks for; the files are the paths the change lands in, separated by spaces. A card you cannot tie to a numbered line is not a card: leave it out. No headings, no prose.";

/// The brief: the evidence lines numbered from 1, one per line, as the cards cite them.
#[must_use]
pub fn brief(lines: &[Line]) -> String {
    let mut text = String::from("# Evidence\n");
    for (index, line) in lines.iter().enumerate() {
        let _ = writeln!(text, "{}. {line}", index + 1);
    }
    text
}
