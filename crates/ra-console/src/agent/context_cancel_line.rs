//! The line of a worker's context that says what cancelling a card does.

/// Cancelling a card removes it and its record, so a read of it after a cancel finds nothing.
pub(super) const CANCEL_LINE: &str =
    "Cancelling a card deletes the card and its record: write no test or command that reads a card after a cancel.\n";

#[cfg(test)]
mod tests;
