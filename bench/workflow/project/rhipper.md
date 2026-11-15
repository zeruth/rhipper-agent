# Tally

- 1 ‹project› Tally {c2} [defined]
  - 2 ‹milestone› A ledger worth reading {c2} [defined]
    - 3 ‹task› The largest categories of a ledger {c2} [defined]
        objective: A long ledger has a few categories that matter. `top(entries: &[Entry], n: usize) -> Vec<(String, i64)>` in the report module returns the n categories with the largest sums, each with its sum, largest first and equal sums in name order; a ledger with fewer than n categories returns them all.
        acceptance: `cargo test -p tally -q top_` passes with at least two tests, one of them on equal sums.
        files: crates/tally/src/report.rs
    - 4 ‹task› A line with a third word is an error {c1} [defined]
        objective: `food 12.50 lunch` parses today as if the third word were not there, so a mistyped ledger sums in silence. `parse_line` returns a new variant of `ParseError` for a line of more than two words.
        acceptance: `cargo test -p tally -q third_word` passes.
        files: crates/tally/src/parse.rs
    - 5 ‹task› A category name is lowercase letters, digits and dashes {c1} [defined]
        objective: `Food` and `food` sum apart today, and a line with its two words swapped fails on the wrong word. `parse_line` returns a new variant of `ParseError` that carries the name when a category has any character outside a to z, 0 to 9 and the dash.
        acceptance: `cargo test -p tally -q category_name` passes.
        files: crates/tally/src/parse.rs
    - 6 ‹task› A parse error reads as a sentence {c2} [defined]
        objective: A caller that shows an error to a person has only its debug form. `ParseError` implements `Display` and `std::error::Error`. The sentences, exactly: `empty line`; `no amount after the category`; `not an amount: ` followed by the word as written; `more than two words`; `not a category name: ` followed by the name. This card leaves the command line as it is.
        acceptance: `cargo test -p tally -q error_reads` passes with a test that covers every variant.
        files: crates/tally/src/parse.rs
        waits-on: 4 5
    - 7 ‹task› The command line reads a ledger from standard input {c3} [defined]
        objective: A ledger piped from another program has no file. `tally-cli -` reads the ledger from standard input and otherwise behaves as it does for a file: the report on standard output and exit 0, or the first bad line on standard error and exit 1. An integration test, new at crates/tally-cli/tests/cli.rs, runs the built binary both ways on a fixture ledger, new at crates/tally-cli/tests/fixtures/week.ledger, and compares what it prints with week.report beside it.
        acceptance: `cargo test -p tally-cli -q --test cli` passes with one test that gives the file and one that pipes it.
        files: crates/tally-cli/src/main.rs crates/tally-cli/tests/cli.rs crates/tally-cli/tests/fixtures/week.ledger crates/tally-cli/tests/fixtures/week.report
    - 8 ‹task› A category's share of the total in whole percent {c2} [defined]
        objective: The report says what was spent and not how it divides. `share(entries: &[Entry], category: &str) -> u8`, in a new module at crates/tally/src/share.rs, returns the category's sum as a percentage of the total, rounded to the nearest whole percent with halves up. It is 0 when the total is zero or less, when the category is absent and when its sum is negative, and never more than 100.
        acceptance: `cargo test -p tally -q share_` passes.
        files: crates/tally/src/share.rs crates/tally/src/lib.rs
    - 9 ‹task› A negative amount keeps its sign {c2} [defined]
        objective: A refund is a negative amount, and today `-1.50` parses as minus fifty cents and prints as `-1.-50`. An amount with a leading minus parses to the negative of the amount without it, and `money` prints a negative number of cents as one minus, the units and two decimals; `-0.50` goes both ways as itself. Nothing else about amounts changes: what failed to parse before still fails.
        acceptance: `cargo test -p tally -q negative_` passes. No public item is added, removed or renamed.
        files: crates/tally/src/amount.rs
    - 10 ‹task› Comments and blank lines are skipped {c1} [defined]
        objective: docs/format.md is the specification of the ledger format, and its section on comments is not implemented: a blank line stops the report with an error. The command line skips the lines that section says to skip, and the line number in an error still counts the way that section says.
        acceptance: `cargo test -p tally-cli -q skips_` passes.
        files: crates/tally-cli/src/main.rs
