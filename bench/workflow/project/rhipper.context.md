# Tally

Two crates. `crates/tally` is the library: `amount.rs` (text to cents and back), `parse.rs` (a
line to an `Entry` or a `ParseError`), `report.rs` (sums and the printed report). `lib.rs`
re-exports every public item of every module. `crates/tally-cli` is the command line, one file.

`docs/format.md` is the specification of the ledger format.

Tests sit in a `tests` module at the foot of the file they test and are named as sentences.
Public functions that return a value carry `#[must_use]`; one that returns a `Result` has an
`# Errors` section. No dependencies beyond the standard library.
