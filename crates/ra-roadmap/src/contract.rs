//! The contract of a card: its acceptance read once into what a program proves, what a reader
//! judges and what waits for the owner, so the worker, the check and the review hold the same
//! criteria.

use std::fmt::Write;

use crate::check::is_command;

/// Words that mark a clause only a person or a device can settle.
const OWNER_MARKERS: [&str; 9] =
    ["by hand", "manually", "the owner", "the person", "on a device", "on the device", "visually", "by eye", "in the window"];

/// An acceptance split into its parts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contract {
    /// The commands the acceptance names, in the order written.
    pub checks: Vec<String>,
    /// The clauses a program or a reader settles: a command clause proves its own criterion.
    pub criteria: Vec<String>,
    /// The clauses only a person or a device can settle.
    pub owner: Vec<String>,
}

/// Reads an acceptance into its contract.
#[must_use]
pub fn contract_of(acceptance: &str) -> Contract {
    let mut contract = Contract::default();
    for clause in clauses(acceptance) {
        let commands: Vec<String> = spans(&clause).into_iter().filter(|s| is_command(s)).collect();
        if !commands.is_empty() {
            contract.checks.extend(commands);
            contract.criteria.push(clause);
        } else if is_owner(&clause) {
            contract.owner.push(clause);
        } else {
            contract.criteria.push(clause);
        }
    }
    contract
}

impl Contract {
    /// The contract as text: criteria numbered C1, C2 and so on, then the owner's lines.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (i, criterion) in self.criteria.iter().enumerate() {
            let _ = writeln!(out, "C{}. {criterion}", i + 1);
        }
        if !self.owner.is_empty() {
            out.push_str("For the owner:\n");
            for line in &self.owner {
                let _ = writeln!(out, "- {line}");
            }
        }
        out
    }
}

fn is_owner(clause: &str) -> bool {
    let lower = clause.to_lowercase();
    OWNER_MARKERS.iter().any(|m| lower.contains(m))
}

/// The backticked spans of a clause.
fn spans(clause: &str) -> Vec<String> {
    clause.split('`').skip(1).step_by(2).map(|s| s.trim().to_string()).collect()
}

/// The clauses of an acceptance: split at newlines, semicolons and sentence ends outside backticks.
fn clauses(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_code = false;
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '`' {
            in_code = !in_code;
        }
        let sentence_end = c == '.' && chars.get(i + 1).is_none_or(|n| n.is_whitespace());
        if !in_code && (c == '\n' || c == ';' || sentence_end) {
            if sentence_end {
                current.push(c);
            }
            push_clause(&mut out, &current);
            current.clear();
        } else {
            current.push(c);
        }
    }
    push_clause(&mut out, &current);
    out
}

fn push_clause(out: &mut Vec<String>, raw: &str) {
    let clause = raw.trim().trim_start_matches(['-', '*']).trim();
    if clause.chars().any(char::is_alphanumeric) {
        out.push(clause.to_string());
    }
}

#[cfg(test)]
mod tests;
