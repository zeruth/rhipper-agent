# The workflow benchmark

A small Rust project with eight cards, each built on one way the walk fails today, run by the real walk on an API key in a directory that holds nothing of the person's machine, judged by checks the worker never sees, and written down as one hashed line a later run can be compared with.

How this was produced: by reading the source at `787be7f`, the five reviews beside this file and the live feed. Nothing was run. This session had no shell, so the Rust in sections 2 and 4 has never met a compiler; the first builder cards compile it, and where rustfmt or clippy disagree with this page, they win. Costs and times are estimates from prices in the live feed, to be replaced by the first batch. Four behaviours of Claude Code are recalled, not verified, and are marked "to verify" where they matter.

To import the cards: Part 2 alone. Part 1 holds a roadmap file in a fence whose lines an import would read as cards. Copy everything from the line `## Phase AC` to the end into a file and run `rc import <file> --under <project id> --dry` first.

# Part 1: Design

#### 0. The short version

| Piece | What it is | Size |
|---|---|---|
| The project | `bench/workflow/project/`: `tally`, a two-crate ledger summer with no dependencies, a long format specification, a roadmap of eight cards | about 150 lines of Rust, one 20 KB document |
| The hidden checks | `bench/workflow/hidden/`: one Rust integration test file per card, copied in after the walk ends, plus six checks on the landed tree | about 200 lines |
| The solved tree | `bench/workflow/solved/`: the project with all eight cards done, which proves the hidden checks can pass | about 250 lines |
| The runner | `rc bench run`: scratch directory, clean environment, the real `rc run --all`, hidden checks, one record | one crate, `ra-bench` |
| The record | One line in `bench/results/ledger.log`: a SHA-256, a tab, a JSON record that names the hash of the record before it | about 4 KB a run |
| The comparison | `rc bench compare a b`: an exact permutation test over whole runs; `rc bench gate` exits 1 on a regression | no statistics library |
| Lockstep | The benchmark is embedded in the `rc` binary; `rc bench check` (no key, no dollars) fails when the agent's parser, admission gate, check resolution or feed no longer fit it | in CI on every push |

The three numbers a run produces, in the order they are judged:

1. **Verified first pass**: the share of cards that landed after one round with the worker and one accepting review, and whose hidden checks pass. This is the thing that matters.
2. **Dollars per verified card**: everything the walk spent, divided by the cards that landed and passed their hidden checks.
3. **First-run tokens in**: the input tokens of each card's first worker run. It has the least noise of the three, so it is where a change to prompts or reading shows first.

One finding on the way, which is a card in Part 2 whatever becomes of the benchmark: nothing in the agent sets or removes `ANTHROPIC_API_KEY`. The harness child inherits `rc`'s whole environment less `CLAUDECODE` (`claude_code.rs`, `run`). A key exported in the shell that starts a walk therefore goes to `claude`, and (to verify) a `claude -p` with a key in its environment bills the key, not the Max login. The benchmark needs the key to reach its runs and no others, so the adapter has to make that choice on purpose.

#### 1. Where it lives

```
bench/
  workflow/
    bench.toml            the manifest: version, the eight cards, their pinned hashes
    project/              what a worker sees; copied to scratch, byte for byte
      .gitignore
      Cargo.toml
      rhipper.md
      rhipper.toml
      rhipper.context.md
      docs/format.md
      crates/tally/Cargo.toml
      crates/tally/src/lib.rs amount.rs parse.rs report.rs
      crates/tally-cli/Cargo.toml
      crates/tally-cli/src/main.rs
    hidden/               never copied before the walk has ended
      top.rs words.rs names.rs display.rs stdin.rs share.rs negative.rs comments.rs
    solved/               the files of project/ that the eight cards change, as they end up
  results/
    ledger.log            one line a run, tracked; the person commits it
crates/ra-bench/          bundle, environment, setup, observe, verdicts, record, ledger, compare
apps/rc/src/bench.rs      the subcommand: check, run, compare, gate, verify, pin
```

`ra-bench`'s `build.rs` walks `bench/workflow` and embeds every file in the binary. So an `rc` carries the benchmark of its own commit: there is no way to run agent version N against benchmark version M by accident, and `rc bench run` works on a machine that has the binary and not the repository. `--bench-dir <path>` reads a directory instead, for writing cards.

A new `.gitattributes` holds two lines: `bench/workflow/** text eol=lf`, so a Windows checkout embeds the same bytes as a Linux one, and `bench/results/ledger.log merge=union`, so two branches that each appended a run merge without a conflict (section 6 says why the chain survives that).

The project is its own Cargo workspace. The agent's `members = ["crates/*", "apps/*"]` does not reach into `bench/`, so `cargo test --workspace` at the root never builds it.

#### 2. The project

Tally reads lines of `category amount` and prints a sum per category and a total. It was chosen for four properties: it has no dependencies, so cargo needs no network and a cold build is seconds; its crates sit under `crates/<name>/src`, so `crate_of`, `cargo test -p` and `cargo clippy -p` take the same paths they take in a real project; it has a latent bug (negative amounts) and an unimplemented paragraph of its specification, so two cards are repairs and not inventions; and every behaviour is a pure function or one process, so every hidden check is a plain Rust test.

The lints are named one by one, not as `clippy::pedantic`, because a named lint means the same thing across toolchains and a group does not.

`Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/tally", "crates/tally-cli"]

[workspace.package]
edition = "2024"
version = "0.1.0"
publish = false

[workspace.lints.rust]
unreachable_pub = "warn"

[workspace.lints.clippy]
cast_possible_truncation = "warn"
cast_possible_wrap = "warn"
cast_precision_loss = "warn"
cast_sign_loss = "warn"
missing_errors_doc = "warn"
must_use_candidate = "warn"
```

`crates/tally/Cargo.toml` (and `crates/tally-cli/Cargo.toml` the same with its name and `[dependencies] tally = { path = "../tally" }`):

```toml
[package]
name = "tally"
edition.workspace = true
version.workspace = true
publish.workspace = true

[lints]
workspace = true
```

`.gitignore`: four lines, `var/`, `rhipper.md`, `/target`, `Cargo.lock`. The first two are what the agent's own tests give a project (`commands/tests.rs` writes `var/\nrhipper.md\n`) and how the live repository is kept: the roadmap changes state all through a walk and must not dirty the tree. `Cargo.lock` is ignored because its format version follows the cargo that wrote it, and a rewritten lock would dirty the tree on some machines and not others.

`crates/tally/src/lib.rs`:

```rust
//! Tally reads a ledger of `category amount` lines and sums it by category. An amount is whole
//! cents in an `i64`. `docs/format.md` is the specification of the format.

mod amount;
mod parse;
mod report;

pub use amount::*;
pub use parse::*;
pub use report::*;
```

The glob re-exports are deliberate: a card that adds a public function to a module does not touch `lib.rs`, so only the card that adds a module does.

`crates/tally/src/amount.rs`:

```rust
//! Amounts as text and as cents.

/// The cents of `12`, `12.5` or `12.50`: whole units and at most two decimals.
pub(crate) fn cents_of(text: &str) -> Option<i64> {
    let (units, decimals) = text.split_once('.').unwrap_or((text, "0"));
    if decimals.is_empty() || decimals.len() > 2 || !decimals.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let units: i64 = units.parse().ok()?;
    let mut fraction: i64 = decimals.parse().ok()?;
    if decimals.len() == 1 {
        fraction *= 10;
    }
    Some(units * 100 + fraction)
}

/// Cents as units with two decimals: `1250` is `12.50`.
#[must_use]
pub fn money(cents: i64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_amount_is_units_and_up_to_two_decimals() {
        assert_eq!(cents_of("12.50"), Some(1250));
        assert_eq!(cents_of("800"), Some(80000));
        assert_eq!(cents_of("0.5"), Some(50));
        assert_eq!(cents_of("1.234"), None);
        assert_eq!(cents_of("1."), None);
    }

    #[test]
    fn cents_print_with_two_decimals() {
        assert_eq!(money(1250), "12.50");
        assert_eq!(money(5), "0.05");
    }
}
```

The bug is in both functions: `-1.50` parses as -50 cents, `-0.50` as +50, and `money(-150)` prints `-1.-50`.

`crates/tally/src/parse.rs`:

```rust
//! One line of a ledger into an [`Entry`].

use crate::amount::cents_of;

/// One line of a ledger: a category and an amount in cents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub category: String,
    pub cents: i64,
}

/// Why a line is not an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The line has no words.
    Empty,
    /// The line has a category and nothing after it.
    NoAmount,
    /// The second word is not an amount; it is carried as written.
    BadAmount(String),
}

/// Parses `category amount`: `food 12.50` is twelve and a half units of `food`.
///
/// # Errors
///
/// A [`ParseError`] that says which part of the line is missing or malformed.
pub fn parse_line(line: &str) -> Result<Entry, ParseError> {
    let mut words = line.split_whitespace();
    let category = words.next().ok_or(ParseError::Empty)?;
    let amount = words.next().ok_or(ParseError::NoAmount)?;
    let cents = cents_of(amount).ok_or_else(|| ParseError::BadAmount(amount.to_owned()))?;
    Ok(Entry {
        category: category.to_owned(),
        cents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_a_category_and_an_amount() {
        let entry = parse_line("  food   12.50 ").unwrap();
        assert_eq!((entry.category.as_str(), entry.cents), ("food", 1250));
    }

    #[test]
    fn a_line_without_its_parts_says_which_is_missing() {
        assert_eq!(parse_line("   "), Err(ParseError::Empty));
        assert_eq!(parse_line("food"), Err(ParseError::NoAmount));
        assert_eq!(parse_line("food x"), Err(ParseError::BadAmount("x".to_owned())));
    }
}
```

`crates/tally/src/report.rs`:

```rust
//! Sums of entries and the report printed from them.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::amount::money;
use crate::parse::Entry;

/// The sum of every entry, in cents.
#[must_use]
pub fn total(entries: &[Entry]) -> i64 {
    entries.iter().map(|entry| entry.cents).sum()
}

/// The sum of each category, in the order of the category names.
#[must_use]
pub fn by_category(entries: &[Entry]) -> BTreeMap<String, i64> {
    let mut sums = BTreeMap::new();
    for entry in entries {
        *sums.entry(entry.category.clone()).or_insert(0) += entry.cents;
    }
    sums
}

/// The report: a line for each category, then the total.
#[must_use]
pub fn render(entries: &[Entry]) -> String {
    let mut out = String::new();
    for (category, cents) in by_category(entries) {
        let _ = writeln!(out, "{category} {}", money(cents));
    }
    let _ = writeln!(out, "total {}", money(total(entries)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(category: &str, cents: i64) -> Entry {
        Entry {
            category: category.to_owned(),
            cents,
        }
    }

    #[test]
    fn the_report_sums_each_category_and_ends_with_the_total() {
        let entries = [entry("food", 1250), entry("rent", 80000), entry("food", 250)];
        assert_eq!(render(&entries), "food 15.00\nrent 800.00\ntotal 815.00\n");
    }
}
```

`crates/tally-cli/src/main.rs`:

```rust
//! `tally-cli <file>` prints the report of a ledger file.

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: tally-cli <file>");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::from(2);
        }
    };
    match run(&text) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// The report of a ledger's text, or the first line that is not an entry.
fn run(text: &str) -> Result<String, String> {
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        match tally::parse_line(line) {
            Ok(entry) => entries.push(entry),
            Err(error) => return Err(format!("line {}: {error:?}", index + 1)),
        }
    }
    Ok(tally::render(&entries))
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn a_ledger_becomes_its_report() {
        assert_eq!(run("food 1\nfood 2.50\n"), Ok("food 3.50\ntotal 3.50\n".to_owned()));
    }

    #[test]
    fn a_bad_line_is_named_by_its_number() {
        assert_eq!(run("food 1\nfood\n"), Err("line 2: NoAmount".to_owned()));
    }
}
```

`rhipper.context.md`:

```markdown
# Tally

Two crates. `crates/tally` is the library: `amount.rs` (text to cents and back), `parse.rs` (a
line to an `Entry` or a `ParseError`), `report.rs` (sums and the printed report). `lib.rs`
re-exports every public item of every module. `crates/tally-cli` is the command line, one file.

`docs/format.md` is the specification of the ledger format.

Tests sit in a `tests` module at the foot of the file they test and are named as sentences.
Public functions that return a value carry `#[must_use]`; one that returns a `Result` has an
`# Errors` section. No dependencies beyond the standard library.
```

`docs/format.md` is the one large file, and it is large on purpose: between 16 and 24 KB, twelve numbered sections of prose and examples (1 Purpose, 2 Lines, 3 Categories, 4 Amounts, 5 Whitespace, 6 Comments and blank lines, 7 Errors, 8 The report, 9 Totals, 10 Exit codes, 11 Worked examples, 12 History of the format). Sections 3, 4 and 7 must agree with cards 4, 5, 6 and 9 below. Section 6 is the only one a card needs, and it reads, in full:

> A line whose first character that is not whitespace is `#` is a comment. A line that is empty or only whitespace is blank. A reader skips both. A `#` anywhere else is not a comment: `food 1#2` is an entry with a malformed amount. A skipped line still has a number: the line number in an error counts every line of the file, skipped or not.

`rhipper.toml` holds the model fixed so that the agent is the only thing that differs between two runs:

```toml
# One implementer, one reviewer, no escalation.
[worker]
auth = "api_key"
parallel = 4
turns = 40
check_timeout = 300
borrow = false

[[route]]
model = "claude-sonnet-5-5"

[ladder]
models = ["claude-sonnet-5-5"]
start = []
up_after = 0

[role]
reviewer = "claude-sonnet-5-5"
fix = ""
```

`auth` is the new key of section 5. The rest follows `router.rs` as it stands. A rule with no condition fits every card: `Rule::fits` holds each of complexity, kind, files, words and rejections only when the rule names it, and a rule with no `role` is the implementer's. A rule's model is the ceiling of the cards it routes (`Ladder::step`), so the route alone keeps a rejected card from climbing to opus. The one-rung ladder with no `start` keeps a c1 or c2 card from starting a rung down on haiku, which the default ladder would do. `[role] reviewer` is read before any rule or rung (`Router::reviewer`). A lockstep test still loads this file with the agent's own reader and asserts that all eight cards run on sonnet at zero, one and two rejections and that the reviewer is sonnet "by role", so a change to routing is caught. The primary measure is untouched by escalation in any case, since a first pass ends before one. Another model or a real ladder is another settings file given with `--settings`; its hash is in the record, and records with different settings are never compared.

`fix = ""` turns the fix pass off by name. Without the line the role is off anyway on a machine with no local server (`fix_model` never falls to a Claude model the table did not name), but a benchmark should not depend on what is listening on a port. So a run whose check passed and whose lint did not is sent back to its own worker, which is the dearer of the two paths and the one a change to the lint has to beat. Whether a fix pass on haiku is cheaper than that is a question the benchmark answers in one comparison: a settings file with `fix = "claude-haiku-4-5"` against this one.

No `[worker] check` and no `[worker] lint`: each card names its check, and the lint is the agent's default (`cargo clippy -p` on the crates of the changed files, warnings on changed lines), which is the thing under test.

#### 3. The eight cards

Ten nodes, so the look-back pass (every ten landed cards) never fires.

| # | Key | The failure mode it is built on | Where the trap is |
|---|---|---|---|
| 3 | `top` | A check that is easy to misread | The filter `top_` selects tests by name. The project names tests as sentences, so a worker following the house style writes tests the filter does not match, and a filter that matches nothing exits 0 today (testing.md F5). "Largest" is also by sum, not by size of number, and ties have an order. |
| 4 | `words` | A conflict with a sibling | Cards 4 and 5 each add a variant at the end of `ParseError`, a test in `parse_line` and tests at the foot of `parse.rs`. Run together they conflict; held apart (the dispatcher of card #237 does this for declared overlaps) they cost wall time. Either way the pair has a price, and the record shows which. |
| 5 | `names` | The sibling | As above. |
| 6 | `display` | A card that depends on others | `Display` needs an arm for the two variants cards 4 and 5 add, and neither card fixes the variant's name. The worker learns the names from the landed code or the dependency notes in its prompt; a worktree cut early does not compile at the merge. |
| 7 | `stdin` | An acceptance that needs a fixture | The test must run the built binary, which only an integration test can find (`CARGO_BIN_EXE_`), pipe a file into it, and compare with a second file. Four files, two of them data whose bytes matter. |
| 8 | `share` | A lint trap | The natural body is `(sum as f64 / total as f64 * 100.0).round() as u8`: four cast lints. A new file is linted whole. The function also needs `#[must_use]`. The check passes and the lint does not. |
| 9 | `negative` | A card a reviewer tends to reject | Three clauses, and the check proves one. `-0.50` survives the obvious repair as +50. The obvious repair (strip a `-`, parse the rest) makes `--1.50` parse. A helper made `pub` breaks the last clause, which only a reader can check. |
| 10 | `comments` | A card that tempts reading too much | The change is three lines in one function. The card points at a 20 KB specification of which one paragraph is needed. Reading it whole puts about 5k tokens into every later turn of a run whose floor is 17 to 21k. |

`rhipper.md`:

```markdown
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
```

The cards are written the way a person writes them, faults included, because that is what the workflow meets. One rule keeps them usable: a card must be admitted by the agent's own gate (`rc check` today, `admit::faults` when quality.md's gate lands). When an upgrade makes the gate refuse a bench card, `rc bench check` fails in that change with the gate's own words, and the answer is to rewrite the card and bump its version: the failure mode is then closed at the door, which is a better result than any score.

Overlaps were kept to the ones that are meant. Cards 4, 5 and 6 share `parse.rs` (the pair, then the dependent). Cards 7 and 10 share `main.rs` in two functions twenty lines apart. No other two cards share a file.

How a walk of it should go at four slots. Seven cards are ready at the start. The dispatcher takes cards whose files do not overlap a running card's, so 3, 4, 7 and 8 start, 9 takes the first slot that frees, and 5 and 10 are held behind 4 and 7. When a slot is free and every ready card overlaps a running one, the first runs anyway: if card 4 is still in a rerun when that happens, card 5 starts beside it and the conflict is met. So whether the pair costs time or a conflict pass depends on how card 4's own run went, which is exactly how it goes in the agent's repository. Card 6 starts when 4 and 5 have landed. Three waves, the last of one card.

What each card is there to show, in terms of the changes the five reviews propose. If a change lands and its card does not move, either the change does not do what it says or the card needs recalibrating; both are worth knowing.

| Card | Should move when | Read in the record as |
|---|---|---|
| `top` | A filter that matches no test fails the check (testing.md F5); the contract shows the worker its filter (quality.md) | `the_check_of_the_card_selects_two_tests` goes to ok; fewer review send-backs |
| `words`, `names` | The landing and the conflict pass get cheaper (orchestration.md); the dispatcher's holding gets smarter | `sent_back.conflict` and the pair's seconds |
| `display` | A dependent's prompt carries what its dependencies landed | Its `first_run_in` and turns: the variant names found without a search |
| `stdin` | Check failures name their cause and reach the worker once (testing.md D3) | `sent_back.check` |
| `share` | The lint runs with the check and inside the session (testing.md D4, quality.md's continuation) | `sent_back.lint` falls to zero and `rounds` stays 1 |
| `negative` | The worker and the reviewer read one contract; the reviewer reads proof lines; verdicts are strict (quality.md, testing.md D9) | `rejections`, and `the_public_surface_is_as_it_was` |
| `comments` | The run's carried context shrinks: a map of the file, reads by range (efficiency.md) | Its `first_run_in` and dollars, with first pass unchanged |
| all eight | A smaller floor (efficiency.md) | `T` |

The solved tree's two bodies that are not obvious, as a starting point for the card that writes it. `share`: with `sum` the category's sum and `total` the ledger's, return 0 when `total <= 0` or `sum <= 0`, else `u8::try_from((sum * 200 + total) / (total * 2)).map_or(100, |percent| percent.min(100))`, which is integer arithmetic with halves up and no cast. `cents_of`: strip one leading `-`, refuse a remainder that is empty or starts with `-` or `+`, parse the remainder as before, and negate the whole number of cents; `money` prints `cents.unsigned_abs()` in the two-part form behind a `-` when `cents < 0`.

#### 4. The hidden checks

A hidden check is a Rust integration test. After the walk has ended, the runner clones the landed `main` into a second directory, copies `hidden/<key>.rs` to `crates/<crate>/tests/hidden_<key>.rs` there, and runs `cargo test -p <crate> --test hidden_<key>` once for each card. Each file is its own test binary, so a card that never landed fails to compile alone and takes no other card with it. Each `#[test]` is one verdict, read from cargo's `test <name> ... ok` lines; a file that does not compile gives every verdict of its card as `compile`.

Three properties make them hold whatever the worker chose. They name only what a card's text fixes: function names and signatures, the three variants of the base, exact sentences. Where a card leaves a name free (the two new variants), the test reaches the value through behaviour (`parse_line(..).unwrap_err()`). And no test of one card uses a line that another card changes the meaning of, so a verdict does not depend on which other cards landed, with the one exception of `display`, which is the card that waits.

`hidden/top.rs`:

```rust
use tally::{Entry, top};

fn entry(category: &str, cents: i64) -> Entry {
    Entry { category: category.to_owned(), cents }
}

fn ledger() -> Vec<Entry> {
    vec![entry("food", 200), entry("rent", 300), entry("tea", 500), entry("food", 100), entry("refund", -900)]
}

#[test]
fn largest_first_and_equal_sums_by_name() {
    assert_eq!(top(&ledger(), 2), [("tea".to_owned(), 500), ("food".to_owned(), 300)]);
}

#[test]
fn fewer_than_n_returns_them_all() {
    let names: Vec<String> = top(&ledger(), 10).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["tea", "food", "rent", "refund"]);
    assert!(top(&ledger(), 0).is_empty());
}

#[test]
fn the_check_of_the_card_selects_two_tests() {
    let source = include_str!("../src/report.rs");
    let selected = source.lines().map(str::trim_start).filter(|line| {
        line.starts_with("fn ") && line.split('(').next().is_some_and(|name| name.contains("top_"))
    });
    assert!(selected.count() >= 2, "fewer than two tests match the filter top_");
}
```

`hidden/words.rs`:

```rust
use tally::{ParseError, parse_line};

fn is_new(error: &ParseError) -> bool {
    !matches!(error, ParseError::Empty | ParseError::NoAmount | ParseError::BadAmount(_))
}

#[test]
fn a_third_word_is_a_new_error() {
    assert!(is_new(&parse_line("food 12.50 lunch").unwrap_err()));
    assert!(is_new(&parse_line("food 1 2 3").unwrap_err()));
}

#[test]
fn two_words_still_parse() {
    assert_eq!(parse_line("  food   12.50  ").unwrap().cents, 1250);
}
```

`hidden/names.rs`:

```rust
use tally::{ParseError, parse_line};

fn is_new(error: &ParseError) -> bool {
    !matches!(error, ParseError::Empty | ParseError::NoAmount | ParseError::BadAmount(_))
}

#[test]
fn a_name_outside_the_alphabet_is_a_new_error_that_carries_it() {
    for name in ["Food", "12.50", "a_b", "caf\u{e9}"] {
        let error = parse_line(&format!("{name} 1")).unwrap_err();
        assert!(is_new(&error), "{name}: {error:?}");
        assert!(format!("{error:?}").contains(name), "{name} is not in {error:?}");
    }
}

#[test]
fn lowercase_digits_and_dashes_pass() {
    assert_eq!(parse_line("rent-2026 800").unwrap().category, "rent-2026");
}
```

`hidden/display.rs`:

```rust
use tally::parse_line;

fn said(line: &str) -> String {
    parse_line(line).unwrap_err().to_string()
}

#[test]
fn every_variant_reads_as_its_sentence() {
    assert_eq!(said(""), "empty line");
    assert_eq!(said("food"), "no amount after the category");
    assert_eq!(said("food 1.234"), "not an amount: 1.234");
    assert_eq!(said("food 1 lunch"), "more than two words");
    assert_eq!(said("Food 1"), "not a category name: Food");
}

#[test]
fn a_parse_error_is_an_error() {
    fn is_error<E: std::error::Error>(_: &E) {}
    is_error(&parse_line("").unwrap_err());
}
```

`hidden/stdin.rs` (into `crates/tally-cli/tests/`):

```rust
use std::io::Write as _;
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_tally-cli");

fn piped(ledger: &str) -> Output {
    let mut child = Command::new(BIN)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(ledger.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn a_piped_ledger_prints_its_report() {
    let out = piped("food 1\nrent 2.50\nfood 2\n");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "food 3.00\nrent 2.50\ntotal 5.50\n");
}

#[test]
fn a_piped_bad_line_exits_one_and_names_the_line() {
    let out = piped("food 1\nrent\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("line 2:"));
}

#[test]
fn a_file_still_reads() {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hidden-stdin.ledger");
    std::fs::write(&path, "tea 0.5\n").unwrap();
    let out = Command::new(BIN).arg(&path).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "tea 0.50\ntotal 0.50\n");
}

#[test]
fn the_fixture_landed_and_agrees_with_itself() {
    let ledger = include_str!("fixtures/week.ledger");
    assert!(!ledger.trim().is_empty());
    assert_eq!(String::from_utf8_lossy(&piped(ledger).stdout), include_str!("fixtures/week.report"));
}
```

`hidden/share.rs`:

```rust
use tally::{Entry, share};

fn entry(category: &str, cents: i64) -> Entry {
    Entry { category: category.to_owned(), cents }
}

#[test]
fn a_share_is_a_whole_percent_with_halves_up() {
    let thirds = [entry("a", 100), entry("b", 200)];
    assert_eq!((share(&thirds, "a"), share(&thirds, "b")), (33, 67));
    let eighths = [entry("a", 125), entry("b", 875)];
    assert_eq!((share(&eighths, "a"), share(&eighths, "b")), (13, 88));
}

#[test]
fn the_edges_are_zero_or_one_hundred() {
    assert_eq!(share(&[], "a"), 0);
    assert_eq!(share(&[entry("a", 100)], "b"), 0);
    assert_eq!(share(&[entry("a", 100)], "a"), 100);
    assert_eq!(share(&[entry("a", 100), entry("b", -100)], "a"), 0);
    let refund = [entry("a", 300), entry("b", -100)];
    assert_eq!((share(&refund, "a"), share(&refund, "b")), (100, 0));
}
```

`hidden/negative.rs`:

```rust
use tally::{money, parse_line};

fn cents(amount: &str) -> Option<i64> {
    parse_line(&format!("x {amount}")).ok().map(|entry| entry.cents)
}

#[test]
fn a_leading_minus_negates_the_amount() {
    assert_eq!(cents("-1.50"), Some(-150));
    assert_eq!(cents("-0.50"), Some(-50));
    assert_eq!(cents("-0.05"), Some(-5));
    assert_eq!(cents("-12"), Some(-1200));
    assert_eq!(cents("12.5"), Some(1250));
}

#[test]
fn money_prints_one_sign() {
    assert_eq!(money(-150), "-1.50");
    assert_eq!(money(-50), "-0.50");
    assert_eq!(money(-5), "-0.05");
    assert_eq!(money(0), "0.00");
    assert_eq!(money(1250), "12.50");
}

#[test]
fn what_failed_before_still_fails() {
    for amount in ["--1.50", "-", "-.5", "1.-5", "1.234", "-1.234", "1e2"] {
        assert_eq!(cents(amount), None, "{amount}");
    }
}

#[test]
fn the_public_surface_is_as_it_was() {
    let source = include_str!("../src/amount.rs");
    let public: Vec<&str> = source.lines().map(str::trim_start).filter(|line| line.starts_with("pub ")).collect();
    assert_eq!(public, ["pub fn money(cents: i64) -> String {"]);
}
```

`hidden/comments.rs` (into `crates/tally-cli/tests/`):

```rust
use std::process::{Command, Output};

fn run(name: &str, ledger: &str) -> Output {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, ledger).unwrap();
    Command::new(env!("CARGO_BIN_EXE_tally-cli")).arg(&path).output().unwrap()
}

#[test]
fn comments_and_blank_lines_are_skipped() {
    let out = run("hidden-skip.ledger", "# week 3\n\nfood 1\n   \n  # indented\n#food 9\nfood 2\n");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "food 3.00\ntotal 3.00\n");
}

#[test]
fn an_error_counts_every_line_of_the_file() {
    let out = run("hidden-count.ledger", "food 1\n\n# note\nfood\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("line 4:"));
}

#[test]
fn a_hash_inside_a_line_is_not_a_comment() {
    assert_eq!(run("hidden-hash.ledger", "food 1#2\n").status.code(), Some(1));
}
```

Six more checks run on the clone before any hidden file is copied in. They judge the tree the walk left, and each failure is charged to the cards whose `files:` name the file at fault.

| Verdict | Command or scan | It fails when |
|---|---|---|
| `tests` | `cargo test --workspace -q` | The landed tree's own tests fail, the workers' tests included |
| `clippy` | `cargo clippy --workspace --all-targets --message-format=short -- -D warnings` | Any warning; charged by the path at the head of each line |
| `fmt` | `cargo fmt --all --check` | The landing left unformatted code |
| `scope` | `git diff --name-only <base> main` | A changed path is in no card's `files:` |
| `quiet` | a scan of every `.rs` file | A lint attribute exists: `#[allow(`, `#![allow(`, `#[expect(` or `#![expect(`. The base has none, so a lint silenced is not a lint passed |
| `whole` | a scan of every tracked file | A line starts with seven `<`, `=` or `>` (a conflict marker) |

A card is **verified** when it landed, every test of its hidden file passed, and none of the six names one of its files.

How hidden the checks are: they are never in the scratch directory while a worker runs, and no command of `rc` prints them. They are, though, on the same disk (in the repository, if there is one) and inside the `rc` binary. A worker that went looking could find them. The runner does not pretend otherwise; it looks: section 5's scan reads every tool call of every run for a path outside the scratch directory, and a run with one is marked tainted and left out of every comparison.

`solved/` is the answer key: the nine files the cards change or add, as a clean solution leaves them. `rc bench check` copies the project to a temporary directory and proves two things with it. On the base, every hidden file fails or does not compile (a check that passes before the work is no check). With `solved/` laid over the base, all eight hidden files and all six tree checks pass (a check nothing can pass is no check either). This is the test that stops the benchmark from rotting when someone edits a card.

#### 5. The runner

`rc bench run [--repeats N] [--parallel N] [--settings FILE] [--rc PATH] [--versus PATH] [--max-usd X] [--pass-env NAME] [--label TEXT] [--keep]`

One run, in order. Steps 1 to 4 and 8 to 11 spend nothing.

1. **Refuse early.** `ANTHROPIC_API_KEY` must be set and not empty, or the command exits 2 with `rc bench runs on an API key and never on a login: set ANTHROPIC_API_KEY`. `git`, `cargo`, `cargo clippy` and `claude` must answer `--version`. The ledger must verify; a broken chain is not appended to.
2. **Scratch.** A short path under the system temp directory, `rb-<8 hex>/`, with `p/` (the project), `home/`, `claude/`, `hub/`, `s/`, `tmp/`, `v/` (the clone for the hidden checks) and `e/` (evidence). Short, because a worktree's target directory nests deep and Windows paths end at 260 characters.
3. **Project.** The embedded files of `project/` are written byte for byte to `p/`. Then `git init -b main`, `git add -A`, and one commit with a fixed name, mail, message and date. Same bytes and same commit fields give the same commit hash on every machine; the record carries it as `bench.base`, and two records with the same benchmark hash and different bases mean a copy went wrong.
4. **Environment.** The child's environment starts empty and gets exactly the table below. Nothing else of the person's shell reaches the walk.
5. **Probe** (under a cent). One `claude -p` on haiku with `--max-turns 1`, no tools, in that environment, `--output-format stream-json`. The runner reads the `system` `init` event's `apiKeySource` (to verify: the field's name and values) and refuses to go on if the harness says it is not on the key from the environment. The probe also records `claude --version`.
6. **Walk.** `<rc> run --all --parallel 4 --no-debug` with `p/` as its directory, where `<rc>` is the running binary or the one given with `--rc`. The parallel walk is also the path that does not hand over to an installed build (`run_all` does; `run_parallel` does not), and with `LOCALAPPDATA` and `HOME` pointed at scratch there is no installed build to hand over to.
7. **Watch.** The runner follows `p/var/pair.log` to its `walk ran N card(s)` line. It stops the walk and records why when the dollars in `p/var/usage.db` pass `--max-usd` (default 15), the clock passes 40 minutes, or one card is started a sixth time. The third stop is there because the parallel walk has no count of its own for a card that keeps changing files: it blocks a card after three rounds in a row that changed nothing (`RERUNS_A_WALK`), and the limits of two check failures and one rejection (`CHECK_RETRIES`, `REVIEW_RETRIES`) are the sequential walk's alone. A card and a reviewer that disagree can go round until the money is gone, and the benchmark should measure that once, not pay for it five times.
8. **Collect.** Into `e/`: the feed, the usage rows as a tab-separated file, `var/history.log`, `var/gotchas.log`, the final `rhipper.md`, every run's stream and system prompt from `var/runs/`, the walk's own output, and `git bundle create landed.bundle main`. Also `var/checks.tsv` and `var/events.jsonl` on the day those exist.
9. **Judge.** The six tree checks and the eight hidden files, in `v/`, with a target directory of their own.
10. **Scan.** Three scans over `e/`. The key's value: any occurrence is replaced by `[key]` and counted. The person's real home directory, user name and default hub path, resolved before step 4: any occurrence is a leak into the run and is counted. Every tool call's path arguments: any path outside `rb-<8 hex>/` is counted. A run with a nonzero second or third count is tainted. A `rate_limit_event` with plan windows in any stream, or a `var/limits.json`, means a login was used: the run is void and says so.
11. **Record.** Section 6. The line is appended to the ledger, `e/` moves to `var/bench/<run id>/` beside the ledger (untracked), and the scratch directory is removed unless `--keep`.

`--repeats N` does this N times and gives the N records one batch id. `--versus PATH` alternates the two binaries, A B A B, so both meet the same hour, the same API load and the same model snapshot.

A benchmark run can go on beside a real walk on the same machine. It has its own hub, so it takes no walk lock, no build slot and no mailbox of the person's; it is on the key, so it draws on no plan window. It does share the processor, which is one more reason wall time is recorded and not judged.

The environment of the walk, complete:

| Variable | Value | Why |
|---|---|---|
| `ANTHROPIC_API_KEY` | the caller's | The only credential in the run |
| `CLAUDE_CONFIG_DIR` | `claude/` | An empty harness home: no login to fall back on, no user settings, hooks, memory or MCP servers (to verify: that the variable moves credentials too; on macOS the keychain may still hold a login, which the probe and the plan-window scan catch) |
| `HOME` `USERPROFILE` `APPDATA` `LOCALAPPDATA` `XDG_CONFIG_HOME` `XDG_DATA_HOME` | under `home/` | Whatever a tool looks for in a home directory, it finds nothing; `rc`'s install directory and default hub are derived from these |
| `RHIPPER_HUB` | `hub/` | No machine notes, no lines learned by other projects, no digests of other projects in the system prompt; no shared build slots or flaky list |
| `RHIPPER_SCRATCH` | `s/` | Cold targets every run, the same for both sides of a comparison |
| `RHIPPER_LOCAL_URL` | `http://127.0.0.1:9/v1` | No local model answers, so the cheap roles call nothing (`local.rs`), on every machine alike |
| `RHIPPER_DEBUG` | `off` | No retrospective question after a run |
| `RHIPPER_KEEP_STREAMS` | `1` | New: every run's stream is kept, not only the last (Part 2) |
| `RHIPPER_BENCH` | the run id | New: lets a feed line or a hook know, and nothing depend on it |
| `TMP` `TEMP` `TMPDIR` | `tmp/` | |
| `GIT_CONFIG_GLOBAL` `GIT_CONFIG_NOSYSTEM` `GIT_TERMINAL_PROMPT` | a file in scratch with a fixed name and mail, `core.autocrlf=false`, `commit.gpgsign=false`; `1`; `0` | The person's git identity, hooks path, signing and line-ending settings stay out |
| `CARGO_HOME` `RUSTUP_HOME` | the caller's real ones, resolved | The toolchain has to be found; nothing is downloaded |
| `CARGO_NET_OFFLINE` `CARGO_BUILD_RUSTC_WRAPPER` `CARGO_TERM_COLOR` | `true`, empty, `never` | No network for cargo, no compiler cache of the person's, plain output |
| `PATH` | the caller's | To find git, cargo and claude; the adapter puts `rc`'s own directory first |
| `SystemRoot` `SystemDrive` `windir` `ComSpec` `PATHEXT` `NUMBER_OF_PROCESSORS` `PROCESSOR_ARCHITECTURE` | the caller's (Windows only) | Processes do not start without them |
| `LANG` `LC_ALL` `TZ` `NO_COLOR` | `C.UTF-8`, `C.UTF-8`, `UTC`, `1` | |
| any `--pass-env NAME` | the caller's | For a proxy or a certificate bundle; the names (never the values) are in the record |

Not in the table and so not in the run: `ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, the Bedrock and Vertex switches, `OPENROUTER_API_KEY`, every other `RHIPPER_*`, `CLAUDECODE`, `CARGO_TARGET_DIR`.

The adapter already keeps the person's own harness settings out: `--setting-sources project,local`, `--strict-mcp-config`, a fixed `--tools` list. The project has no `.claude/` directory and no `CLAUDE.md`. What cannot be kept out is a system-wide managed policy of Claude Code; the record's `claude` version and the prompt hash of section 6 are where its effect would show.

**The key, in both directions.** "Never the subscription" is steps 1, 4, 5 and 10: the key is required, the harness home is empty, the harness is asked what it is using before a dollar is spent, and plan windows in a stream void the run. "Only to these runs" needs the adapter to stop inheriting the key by accident. A new setting, `[worker] auth`:

| Value | The adapter does |
|---|---|
| `subscription` (the default) | removes `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` from the harness's environment |
| `api_key` | refuses to start a run without `ANTHROPIC_API_KEY`, and passes it |

The benchmark's `rhipper.toml` says `api_key`. The default changes today's behaviour for anyone who starts walks from a shell with a key exported and means to be billed on it: they add one line. For the person this is the safe direction, since Max is the default worker and a stray key would otherwise turn a free walk into an invoice without a word in the feed.

The key still reaches the worker's shell tools, because the harness passes its environment down. A worker could `echo` it into a stream. Step 10 scrubs and counts that. `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1` is recalled as a harness switch that strips credentials from tool subprocesses; if it exists (to verify) it joins the table.

#### 6. The record

A line of `bench/results/ledger.log` is a SHA-256 in hex, a tab, and a JSON object on one line, the same shape as the feed's `time<TAB>text`. The hash is of the bytes of the JSON exactly as written, so verifying needs no canonical form and no agreement on how a float prints.

```json
{
  "schema": 1,
  "kind": "run",
  "run": "20261113T180422Z-3f9a1c07",
  "batch": "20261113T180422Z",
  "prev": "9d1e…",
  "at": "2026-11-13T18:04:22Z",
  "agent": {"version": "0.2.412", "commit": "787be7f0…", "dirty": false, "prompt": "sha256:5b0c…"},
  "bench": {"version": 1, "hash": "sha256:c41a…", "base": "6e02…", "settings": "sha256:77fe…",
            "cards": {"top": 1, "words": 1, "names": 1, "display": 1, "stdin": 1, "share": 1, "negative": 1, "comments": 1}},
  "models": {"implementer": ["claude-sonnet-5-5"], "reviewer": ["claude-sonnet-5-5"], "other": []},
  "machine": {"label": "tyler-desktop", "os": "windows 11 10.0.26200", "arch": "x86_64", "cpus": 16, "ram_gb": 64,
              "rustc": "1.91.0", "cargo": "1.91.0", "clippy": "0.1.91", "git": "2.47.0", "claude": "2.1.34"},
  "auth": {"source": "ANTHROPIC_API_KEY", "probe": "ok", "plan_windows": 0, "key_scrubbed": 0},
  "leak": {"home_paths": 0, "outside_paths": 0, "env_passed": []},
  "walk": {"parallel": 4, "seconds": 742, "stop": "no card is ready", "landed": 7, "tree": "a3f1…"},
  "cards": [
    {"id": 3, "key": "top", "v": 1, "landed": true, "rounds": 1, "reviews": 1, "rejections": 0,
     "sent_back": {"check": 0, "lint": 0, "review": 0, "conflict": 0, "merged": 0, "other": 0},
     "first_pass": true, "verified": true,
     "turns": 9, "in": 148210, "cached": 121877, "out": 3120, "first_run_in": 148210,
     "usd": 0.2371, "review_usd": 0.0512, "seconds": 171,
     "hidden": {"largest_first_and_equal_sums_by_name": "ok", "fewer_than_n_returns_them_all": "ok",
                "the_check_of_the_card_selects_two_tests": "ok"},
     "tree": []}
  ],
  "tree": {"tests": "ok", "clippy": "ok", "fmt": "ok", "scope": "ok", "quiet": "ok", "whole": "ok"},
  "totals": {"usd": 3.82, "unattributed_usd": 0.0, "in": 1492300, "cached": 1203114, "out": 31877, "turns": 104},
  "score": {"cards": 8, "verified_first_pass": 5, "verified": 7, "usd_per_verified": 0.5457, "first_run_in": 1101200},
  "evidence": "sha256:0be7…",
  "observe": "feed-1",
  "void": null
}
```

What each field is read from, today:

| Field | Source |
|---|---|
| `agent.version`, `agent.commit`, `agent.dirty` | `<rc> --version`, which must come to print the commit (a card: `build.rs` embeds only the commit count today) |
| `agent.prompt` | SHA-256 over the system prompts of each card's first worker run, in card order, with the scratch path replaced by `<root>`. With an empty hub and an empty home, two machines on one commit must get the same hash; a different one means something of a machine reached a prompt. The prompt file is `var/runs/system-<pid>.md` today, named by `rc`'s own process and so shared and overwritten by every run of a parallel walk; the card that keeps streams keeps each run's prompt under the run's name |
| `bench.hash` | SHA-256 over every embedded file in path order: path, a zero byte, the bytes with CRLF folded to LF, a zero byte |
| `bench.settings` | SHA-256 of the `rhipper.toml` the walk ran with |
| `models` | The distinct `model` values of the usage rows, by role |
| a card's `rounds` | The feed's `start #id` lines plus its `↻ #id … continues` lines: every time the card was handed to a worker, in a new session or the same one. Not the usage rows, since one run can write two (the second ask for a report is recorded apart) |
| `reviews`, `rejections` | Rows with role `reviewer`; `review ✗` lines of the feed |
| `sent_back` | Six counters, each from a line as the source writes it at `787be7f`. `review`: a `review ✗ #id` line. `check` and `lint`: an `end ✗ #id … needs revision on …` line; it is `lint` when the walk's own output has `check passed: … · lint: N warning(s)` under that run's line, and `check` otherwise, because the feed's line does not say which (a gap observability.md's events close). `conflict`: `#id not reviewed: … merged into … with conflicts` or `#id not landed: … conflicts …`. `merged`: `#id not landed: the merged tree fails …` or `#id not reviewed: the merged tree fails …`. `other`: `#id blocked: …`, `#id parked: …`, `#id failed and waits for a person`, `end ✗ #id … failed: …`. The `rerun #id` and `waits #id` lines of `pair::sent_back` are written by the sequential walk only and repeat a cause already counted, so they are read for nothing |
| `turns`, `in`, `cached`, `out`, `usd` | Sums over the card's usage rows: `turns`, `prompt_tokens`, `cached_tokens`, `completion_tokens`, `cost_usd`. `in` includes `cached`. `review_usd` is the reviewer rows' part of `usd` |
| `first_run_in` | The input tokens in the `result` event of the card's first kept stream; from a binary that keeps no streams, the `in N` of the card's first `end` line |
| `seconds` | First `start #id` line to the `landed #id` line; to the walk's end if it never landed |
| `first_pass` | `landed`, `rounds` is 1, `rejections` is 0 and every `sent_back` counter is 0: one `start`, one `end ✓`, one `review ✓`, one `landed`, and nothing between |
| `verified` | Section 4 |
| `walk.tree` | `git rev-parse main^{tree}` of the landed repository |
| `evidence` | SHA-256 over the files of `e/` in path order, after the scrub |
| `walk.stop` | The words after `walk ran N card(s) ·` on the feed, or `budget`, `timeout` or `rounds` when the runner stopped the walk. A stopped walk is a result, not a fault: its unlanded cards are unverified and its dollars count |
| `void` | Null, or why the measurement itself cannot be trusted: `login` (a plan window was seen), `tainted` (a leak count is not zero), `observe` (an invariant failed) |

All of this reading is one module, `observe`, and it is the one place that knows the feed's wording. It also checks itself: the dollars of all usage rows must equal the cards' dollars plus `unattributed_usd`; every card whose state ended `accepted` must have a `landed` line; every `start` must have an `end`. A run that fails one is recorded with `void: "observe"` and the gate fails loudly. That is the difference between a feed change being caught and a feed change quietly producing a wrong number. When `var/events.jsonl` and `var/checks.tsv` exist, `observe` reads them instead and `observe` in the record says `events-1`.

A turn here is what the usage ledger calls a turn. efficiency.md shows that figure steps once a tool call and is not a model call; the record keeps the ledger's number and the name, so it moves when the ledger's meaning is fixed, and records on both sides of that fix are not compared on turns.

**The chain.** `prev` is the hash of the last line of the ledger at the time of writing, or sixty-four zeros. `rc bench verify` checks that each line's hash is the hash of its JSON and that each `prev` is the hash of some earlier line. "Some earlier line", not "the line before", so that a union merge of two branches' appended lines still verifies: the chain is allowed to fork and rejoin, as git's is.

What this proves and does not. Changing any field of any record breaks its hash. Recomputing the hash orphans every record that named it. So the middle of the ledger cannot be edited without `verify` saying where. The last line can: a chain protects history, not its tip. The tip is protected by the thing the person already has, a pushed commit on a branch that is never force-pushed, and `rc bench verify --anchored` adds one check for it: every line of the ledger at `origin/main` is still a line of the working copy. A signature is the stronger form and an optional last card: `ssh-keygen -Y sign` over a record's hash, kept in a second file, since a hash cannot cover its own signature.

What it does not prove is that the numbers were measured honestly, only that they were not changed after. The evidence directory is the answer to that, as far as there is one. `rc bench verify --replay var/bench/<run id>` recomputes `evidence`, unbundles `landed.bundle`, runs the hidden checks again on it, and re-derives every card's tokens and dollars from the kept streams (each ends in the harness's own `result` event with its usage and `total_cost_usd`). A record whose replay agrees was produced by that tree and those streams. Someone set on it could still forge streams; the message ids in them are Anthropic's, and that is as far as a file can go.

#### 7. Comparing two runs

One run proves nothing. The same commit run twice gives different numbers, because the model does.

**The unit is a whole run**, never a card. Cards inside a run are not independent (the pair and its dependent rise and fall together; a slow API hour touches all eight), so the test shuffles runs.

**The three scores of a run** are section 0's: `F`, verified first passes over cards; `C`, the walk's dollars over its verified cards (a run with none has no `C` and is counted as the worst); `T`, the sum of `first_run_in`.

**The test** is an exact permutation test. Given batch A of a runs and batch B of b runs, take the difference of the batch means of a score. Then form every way of dealing the a+b runs into groups of a and b (252 ways for 5 and 5; 184,756 for 10 and 10; above a million, 100,000 deals drawn with a seed made from the two batch ids, so the answer is the same every time it is asked). The p-value is the share of deals whose difference is at least as large as the one observed. No distribution is assumed, no library is needed, and the smallest p at 5 and 5 is 1 in 252.

**The noise floor** falls out of the same deals: the 95th percentile of the absolute difference across them is the size of difference that chance produces at this number of repeats. `compare` prints it beside every verdict. `rc bench compare --aa <batch>` deals one batch against itself and prints only the floor, which is the measured answer to "how much does the model wobble on this benchmark".

**What may be compared.** Two records are comparable when `schema`, `bench.version`, `bench.settings`, `models`, `walk.parallel`, the major and minor of `machine.claude` and `observe` are equal, neither is void, and they are at most 14 days apart, because the model behind an id is retrained under it. Within that, the scores are computed over the cards whose `key` and `v` are equal on both sides, and `compare` says how many those are. A different `os` or toolchain does not forbid a comparison of `F` and `C`, but is printed. Wall time is never compared across machines.

**The verdicts.**

| Verdict | Rule |
|---|---|
| Cannot judge | A run on either side is void, or fewer than three comparable runs a side, or no card version in common. The reason is printed. A void for `observe` on the new side is the benchmark catching a change to the feed, and is fixed in `observe`, not argued with |
| Worse | `F` is down with p below 0.05; or `C` is up by 10% or more with p below 0.05 |
| Better | Not worse, and: `F` is up with p below 0.05; or `C` is down by 10% or more with p below 0.05; or `T` is down by 5% or more with p below 0.05 while `F` and `C` are inside their floors |
| No difference at this size | Anything else. Printed with the floors |

A walk the runner stopped for budget, time or rounds is not void. Its cards count as they stood, which is badly, and that is the right score for an agent that loops.

A verdict of "worse" is checked once before it is believed: a second batch of B, and worse again. At 5% each, a false alarm twice running is one gate in 400.

What `compare` prints, with invented numbers:

```
baseline 20261113T180422Z  0.2.412 (787be7f0a1c2)  10 runs      last 20261120T091500Z  0.2.431 (c3d9e1b77a04)  5 runs
bench 1 · 8 of 8 cards in common · settings 77fe · claude-sonnet-5-5 / claude-sonnet-5-5 · claude 2.1

card       verified first pass     median $ a card     sent back for (baseline, last)
top          3/10      4/5           0.31    0.24      check 2 review 6 | review 1
words        9/10      5/5           0.12    0.11
names        7/10      5/5           0.19    0.12      conflict 3 |
display      6/10      3/5           0.22    0.25      review 4 | review 2
stdin        2/10      2/5           0.61    0.48      check 5 review 4 | check 2 review 1
share        1/10      4/5           0.44    0.21      lint 9 | lint 1
negative     2/10      1/5           0.52    0.55      review 9 | review 4
comments     8/10      5/5           0.18    0.11

score                        baseline     last      change    p        floor
verified first pass (F)      0.475        0.725     +0.250    0.012    0.20
dollars a verified card (C)  0.71         0.44      -38%      0.008    21%
first-run tokens in (T)      1.31M        1.02M     -22%      0.004    6%

better: first pass up 0.25 (floor 0.20), dollars a verified card down 38% (floor 21%)
```

The per-card rows are for the reader, not the verdict: at five runs a card's own count proves nothing, but a row that goes from 1 of 10 to 4 of 5 with its lint send-backs gone says which change did the work.

**How many repeats.** With eight cards, a run's `F` moves in steps of 0.125, and its spread from run to run should be near 0.12 to 0.18 (a guess from eight unequal coin flips, to be replaced by the first `--aa`). On that guess:

| Repeats a side | Smallest shift in `F` likely to be seen | In cards a run | Estimated cost of both sides |
|---|---|---|---|
| 3 | about 0.33 | 2.6 | $20 to $45 |
| 5 (the default) | about 0.25 | 2 | $35 to $75 |
| 10 | about 0.17 | 1.4 | $70 to $150 |

Against a stored baseline only one side is paid for. Today's figures (reruns on 69 to 97% of cards, first reviews passing 8 to 71%) leave room for changes worth two cards a run, so five repeats is the right start. As the agent improves, shifts shrink, and the finer instrument is `T`, whose spread should be a few percent, and then `C`.

A full run is estimated at $3 to $8 and 10 to 20 minutes on sonnet: eight worker runs at $0.10 to $0.40 and eight reviews at $0.04 to $0.07 when everything passes once, and two to three times that at today's rerun rates.

**Calibration.** A card that passes first time in every run, or in none, tells nothing in either direction. After the first batch of ten, any card with a verified first pass of 0 of 10 or 10 of 10 is looked at: made harder or easier, with its version bumped. The aim is a baseline `F` between 0.3 and 0.7, where both an improvement and a regression have room to show. This is the one part of the design that only running it can finish.

#### 8. Lockstep

Four mechanisms, from free to paid.

**1. One binary.** The benchmark is embedded, so agent and benchmark cannot be of different commits.

**2. `rc bench check`** needs no key and spends nothing. It runs in CI on every push (`.github/workflows/build.yml`) and is what any card that touches prompts, the roadmap format, admission or checks will meet at its own landing, because the cheap parts are plain tests of `ra-bench`. It fails, with the reason, when:

| Part | It fails when | So this change to the agent is caught |
|---|---|---|
| Parse | The embedded `rhipper.md` does not load with the current parser, or loading and saving it does not give back the same bytes | A change to the roadmap format: the fix is to migrate the bench file in the same commit |
| Admit | `rc check` on it reports a fault, or `preflight_faults` says a card's check is rejected before any build | A stricter admission gate or a new hook rule that would refuse a bench card or its command |
| Resolve | A card's checks, as the walk would derive them, differ from the `check` list pinned in `bench.toml` | A change to `is_command`, `named_checks`, the fallbacks or the acceptance grammar |
| Settings | The embedded `rhipper.toml` does not load, or a card routes somewhere else | A change to routing, the ladder or the setting names |
| Pins | A card's content hash differs from the one pinned for its version | An edit to a card, its files or its hidden check without a version bump |
| Red and green | A hidden file passes on the base, or fails on the solved tree; the base or the solved tree is not clean under fmt and clippy | A broken benchmark, or a toolchain whose lints moved |
| Observe | A scripted walk over the bench roadmap (the scripted provider of `commands/tests.rs`) leaves a feed and a usage ledger that `observe` cannot read with every invariant holding | A change to the feed's wording, the usage ledger's columns or the order of a card's events |

The red-and-green part runs cargo on a copy of the project and takes a minute or two cold. It is a test marked ignored and run by name, in the sense of testing.md's live tier. Everything else is milliseconds.

**3. Versions.** `bench.toml` has a `version` for the whole benchmark and a `v` for each card, each pinned with a hash:

```toml
version = 1
repeats = 5
max_usd = 15.0
max_minutes = 40
max_rounds = 5        # a card's sixth start stops the walk
base = "sha256:…"     # every file of project/ except rhipper.md, and the six tree checks

[[card]]
id = 3
key = "top"
v = 1
crate = "tally"
check = ["cargo test -p tally -q top_"]
sha = "sha256:…"      # title, objective, acceptance, files and waits-on as parsed, and hidden/top.rs
```

A card's hash is taken over the parsed card, not the text of the file, so a migration of the roadmap's syntax changes no card. Changing a card's words or its hidden check changes `sha`; the Pins part then fails until `v` is bumped and `rc bench pin` rewrites the hash. Changing the base code changes `base` and requires bumping `version`, which makes every older record incomparable, so it should happen about never: the base is finished the day it is calibrated. `compare` uses only the cards whose `key` and `v` match on both sides. That is how an old record stays comparable on seven cards after the eighth was rewritten.

`schema` is the record's own version. A new field is added without a bump; a field whose meaning changes bumps it.

**4. The gate.** `rc bench gate [--repeats N]` runs a batch, compares it with the baseline, prints the table and exits 0 for better or no difference, 1 for worse, 2 when it could not judge (no key, no comparable baseline, a baseline older than 14 days). The baseline is the newest batch that carries a `baseline` mark, which is itself a ledger line (`"kind": "baseline"`, naming the batch) written by `rc bench run --baseline`, so moving the baseline is in the chain too. When the baseline is stale, exit 2 says how to renew it: build the baseline commit in a worktree and run `rc bench run --versus <that rc> --baseline`.

Where the gate runs. Not in the walk's landing: at $20 and 20 minutes it cannot guard every card. Three places fit:

1. By hand, before `rc install` puts a new build in charge of real walks. `rc install` prints one line when the build it installs has no record in the ledger, or a "worse".
2. In CI by hand: a `bench.yml` workflow with `workflow_dispatch` and the key as a secret.
3. From the walk, as the acceptance of a milestone's last card when the milestone changed prompts, review or checks. This needs `rc` to be a check program, which it is not today, or a one-line script; it is left to the person.

#### 9. Portability

What a machine needs: git, a Rust toolchain of 1.85 or later with clippy and rustfmt, Claude Code on the path, and `ANTHROPIC_API_KEY`. No network beyond the API, no Python, no shell scripts (the hidden checks are Rust; the runner is `rc`), no repository (the benchmark is in the binary).

What is the same everywhere: the bytes of the project and so `bench.base`; the environment of section 5; the prompt, which `agent.prompt` proves; the settings.

What differs and is recorded: `machine`. Wall time depends on it; first pass and tokens should not, and the first batch run on two machines is the test of that claim.

What the record deliberately lacks: any path, the key, any value of a passed variable.

Known limits. Clippy's lints move between toolchains even when named, so the lint card may differ across a toolchain upgrade; `machine.clippy` is printed when it differs between two batches. A corporate proxy needs `--pass-env`. A managed Claude Code policy cannot be turned off from here. On macOS the keychain is outside `CLAUDE_CONFIG_DIR`; the probe is the guard.

#### 10. What it does not do, and what is unverified

Left out on purpose: a second benchmark project in another language (the agent's failure modes here are in its workflow, not in Rust); a score for the reviewer's accuracy apart from the outcome (a wrong acceptance already shows as landed and not verified, a wrong rejection as a lost first pass); wall time as a judged score; a leaderboard.

To verify before the runner is trusted, each one a line in a builder card's acceptance:

1. That `claude -p` with `ANTHROPIC_API_KEY` set and an empty `CLAUDE_CONFIG_DIR` runs without any interactive step, bills the key, and says so in its `init` event.
2. That `CLAUDE_CONFIG_DIR` also moves the credentials file on Windows and Linux.
3. That a run on a key emits no `rate_limit_event` with plan windows.
4. That `rc run` itself does not call the login check of `rc doctor` (it appears not to; `claude auth status` is in `doctor` only).
5. That the walk's own output still carries the `check passed: … · lint: N warning(s)` line under a run's line when it is captured from a detached process, since that line is all that tells a lint send-back from a check send-back today.
6. That the harness has a switch that keeps credentials out of its tool subprocesses.
7. That the code of sections 2 and 4 compiles and is clean under rustfmt and the seven named lints. A base amount with a leading `+` parses today (`i64`'s own parser takes it); the solved tree and section 4 of the specification must agree on whether it stays, and the hidden checks do not ask.

What can be done by hand today, before any card, to learn the most for a few dollars: copy sections 2 and 3 into a directory, set the variables of section 5's table in one PowerShell session, run `rc run --all --parallel 4 --no-debug` there, and read `rc usage --cards`. That one walk answers items 1, 3, 4 and 5, shows whether a card goes round without end, and gives the first real cost. Until the `auth` card lands, leave that line out of the hand-made `rhipper.toml`; the key reaches the harness by inheritance, which is the finding of section 0 put to use once.

# Part 2: Roadmap

Twenty-four cards in build order. The outline has no way to say that one card waits on another, so the order is the dependency: a section needs the sections above it, and inside a section a card needs the cards above it unless it says otherwise. Section 2 depends on nothing and can run first or alongside section 1.

Card 1 creates every module file of `ra-bench` and card 15 creates every verb of `rc bench`, so that no later card edits `crates/ra-bench/src/lib.rs` or `apps/rc/src/main.rs` and no two of these cards meet in a file. Each module keeps its tests in its own `tests.rs` (`crates/ra-bench/src/ledger.rs` and `crates/ra-bench/src/ledger/tests.rs`).

## Phase AC: a benchmark the workflow runs against

### 1. The benchmark in the tree

- A crate that carries the benchmark inside the binary {c3}
  objective: The benchmark has to be of the same commit as the agent that runs it, and has to work where there is no repository. A new crate crates/ra-bench with a build.rs that embeds every file under bench/workflow, a `Bundle` that lists them by path and hashes them (path, zero byte, bytes with CRLF folded to LF, zero byte, in path order, SHA-256), and one empty module file each for project, env, setup, observe, verdict, record, ledger, compare, scan, pin and check, declared in lib.rs so later cards add no line there. New files: bench/workflow/bench.toml with the version, limits and an empty card list as in section 8 of docs/fable/workflow-bench.md, and .gitattributes with the two lines of section 1. `sha2` joins the workspace dependencies.
  acceptance: `cargo test -p ra-bench -q bundle_` passes: the bundle lists bench.toml, the hash of a file set is the same with LF and with CRLF line endings and differs when a byte of content differs or a path is renamed, and a directory given at run time is read in place of the embedded one.
  files: Cargo.toml .gitattributes bench/workflow/bench.toml crates/ra-bench/Cargo.toml crates/ra-bench/build.rs crates/ra-bench/src/lib.rs crates/ra-bench/src/bundle.rs crates/ra-bench/src/bundle/tests.rs crates/ra-bench/src/project.rs crates/ra-bench/src/check.rs crates/ra-bench/src/env.rs crates/ra-bench/src/setup.rs crates/ra-bench/src/observe.rs crates/ra-bench/src/verdict.rs crates/ra-bench/src/record.rs crates/ra-bench/src/ledger.rs crates/ra-bench/src/compare.rs crates/ra-bench/src/scan.rs crates/ra-bench/src/pin.rs

- The benchmark project: the tally workspace {c2}
  objective: The walk needs a project small enough to run for dollars and real enough to fail the way real projects do. The files of section 2 of docs/fable/workflow-bench.md, new under bench/workflow/project: the workspace manifest with its named lints, the two crates, .gitignore, rhipper.context.md and rhipper.toml without its auth line (a later card adds the setting). The code on that page was never compiled; where rustfmt or clippy disagree with it they win, and the two latent faults it describes (negative amounts in both directions, no handling of blank lines) must survive. The project module of ra-bench gains a test, ignored by default because it runs cargo, that writes the embedded project to a temporary directory and proves it.
  acceptance: `cargo test -p ra-bench -q project_is_clean -- --ignored` passes: on a copy of the embedded project, cargo test of the workspace passes offline, cargo clippy with warnings denied prints none, cargo fmt with its check flag changes nothing, and money of minus 150 cents still prints the faulty text.
  files: bench/workflow/project/Cargo.toml bench/workflow/project/.gitignore bench/workflow/project/rhipper.context.md bench/workflow/project/rhipper.toml bench/workflow/project/crates/tally/Cargo.toml bench/workflow/project/crates/tally/src/lib.rs bench/workflow/project/crates/tally/src/amount.rs bench/workflow/project/crates/tally/src/parse.rs bench/workflow/project/crates/tally/src/report.rs bench/workflow/project/crates/tally-cli/Cargo.toml bench/workflow/project/crates/tally-cli/src/main.rs crates/ra-bench/src/project.rs

- The ledger format's specification, long on purpose {c2}
  objective: One card of the benchmark tempts a worker to read a large file for one paragraph. New at bench/workflow/project/docs/format.md: twelve numbered sections as listed in section 2 of docs/fable/workflow-bench.md, between 16 and 24 KB, with section 6 word for word as that page gives it, and sections 3, 4 and 7 agreeing with the benchmark's cards on names, amounts and the five error sentences.
  acceptance: `cargo test -p ra-bench -q specification_` passes: the embedded document is between 16 and 24 KB, has twelve numbered section headings in order, its section 6 holds the five sentences on comments and blank lines exactly, and the five error sentences each appear in section 7.
  files: bench/workflow/project/docs/format.md crates/ra-bench/src/project.rs

- The benchmark's roadmap of eight cards and their manifest {c3}
  objective: The cards are the benchmark. New at bench/workflow/project/rhipper.md: the roadmap of section 3 of docs/fable/workflow-bench.md, word for word, ten nodes with card 6 waiting on 4 and 5. bench/workflow/bench.toml gains its eight card entries (id, key, v, crate, check). The pin module computes a card's hash from the parsed card and its hidden file, and the base hash from the rest of the project, and the manifest reader refuses a manifest whose cards do not match the roadmap's task ids.
  acceptance: `cargo test -p ra-bench -q roadmap_` passes: the embedded roadmap loads with the current parser and saves back to the same bytes, has eight tasks under one milestone, card 6 waits on 4 and 5 and no other card waits, the roadmap's own lint reports no fault, each manifest entry names a task that exists, and a card's hash is unchanged by reindenting the file and changed by one word of its acceptance.
  files: bench/workflow/project/rhipper.md bench/workflow/bench.toml crates/ra-bench/src/pin.rs crates/ra-bench/src/pin/tests.rs

- The hidden checks and the solved tree that proves them {c3}
  objective: A pass has to be verified by something the worker never saw, and the checks themselves have to be shown able to fail and able to pass. New under bench/workflow/hidden: the eight test files of section 4 of docs/fable/workflow-bench.md. New under bench/workflow/solved: the files the eight cards change or add, as a clean solution leaves them (amount.rs, parse.rs, report.rs, share.rs, lib.rs, main.rs, tests/cli.rs and its two fixtures). The verdict module copies a hidden file into its crate's tests directory, runs it alone, and reads one verdict a test from cargo's output, with a file that does not compile giving every verdict as compile; it also runs the six tree checks of section 4.
  acceptance: `cargo test -p ra-bench -q hidden_checks -- --ignored` passes: on a copy of the base every hidden file has at least one verdict that is not ok, with the solved tree laid over it every verdict of all eight files and all six tree checks is ok, and a conflict marker, an allow attribute or a file outside every card's list each turn exactly their own tree check.
  files: bench/workflow/hidden/top.rs bench/workflow/hidden/words.rs bench/workflow/hidden/names.rs bench/workflow/hidden/display.rs bench/workflow/hidden/stdin.rs bench/workflow/hidden/share.rs bench/workflow/hidden/negative.rs bench/workflow/hidden/comments.rs bench/workflow/solved/crates/tally/src/amount.rs bench/workflow/solved/crates/tally/src/parse.rs bench/workflow/solved/crates/tally/src/report.rs bench/workflow/solved/crates/tally/src/share.rs bench/workflow/solved/crates/tally/src/lib.rs bench/workflow/solved/crates/tally-cli/src/main.rs bench/workflow/solved/crates/tally-cli/tests/cli.rs bench/workflow/solved/crates/tally-cli/tests/fixtures/week.ledger bench/workflow/solved/crates/tally-cli/tests/fixtures/week.report crates/ra-bench/src/verdict.rs crates/ra-bench/src/verdict/tests.rs

### 2. A run nothing leaks into

- The harness gets an API key only when the project asks for one {c3}
  objective: The harness child inherits the whole environment of rc, so a key exported in the shell that starts a walk reaches claude and can turn a walk on the Max login into a bill, with no word in the feed. A new setting, auth, in the worker table: subscription, the default, removes ANTHROPIC_API_KEY and ANTHROPIC_AUTH_TOKEN from the harness's environment; api_key refuses to start a run without ANTHROPIC_API_KEY and passes it. The start line of the feed says on a key when it is. The benchmark's rhipper.toml gains its auth line.
  acceptance: `cargo test -p ra-providers -q auth_` passes: with the default the command built for the harness removes both variables though the parent has them, with api_key and the key set it keeps the key, with api_key and no key the run fails before any process starts and the error names the variable, and a rhipper.toml with an unknown auth value is refused by name.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/tests.rs crates/ra-console/src/router.rs crates/ra-console/src/agent.rs bench/workflow/project/rhipper.toml

- Every run's stream is kept when the environment asks {c2}
  objective: The raw stream of a run is the only first-hand record of its tokens, dollars and tool calls, and var/runs/last.jsonl is overwritten by the next run. With RHIPPER_KEEP_STREAMS set to 1 the adapter also writes each run's stream to var/runs under a name made of the card's id and a counter, and keeps each system prompt file beside it; without the variable nothing changes.
  acceptance: `cargo test -p ra-providers -q keeps_streams` passes: two scripted runs of one card with the variable set leave two stream files and two prompt files whose names carry the card's id and 1 and 2, last.jsonl still holds the second, and without the variable only last.jsonl exists.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/tests.rs

- The version names the commit {c1}
  objective: A benchmark record has to say exactly which agent produced it, and the version today is a count of commits that two branches can share. The build script also embeds the commit's full hash and whether crates, apps or bench were dirty, and the version flag of rc prints the count, then the first twelve characters of the hash in brackets, then a plus sign when dirty; a build outside a git checkout prints unknown for the hash.
  acceptance: `cargo test -p rc -q version_names_the_commit` passes: the version text holds the count, a bracketed hash of twelve hex characters or the word unknown, and a parser in ra-bench's record module reads the three parts back from that text and from the old text with no hash.
  files: crates/ra-console/build.rs crates/ra-console/src/lib.rs apps/rc/src/main.rs apps/rc/src/tests.rs crates/ra-bench/src/record.rs

- The clean environment of a benchmark walk {c2}
  objective: Nothing of the person's machine may reach a benchmark run except the tools on the path and the key. The env module builds the walk's environment from nothing: exactly the table of section 5 of docs/fable/workflow-bench.md, with the home, harness, hub, scratch and temp directories under one scratch root, a git config file written there, the caller's cargo and rustup homes resolved before the home is moved, and any variable named by a pass list copied through. Its debug form never prints the key's value.
  acceptance: `cargo test -p ra-bench -q clean_env` passes: built from a parent environment holding a hub path, an OpenRouter key, a base URL and a target directory, the result holds none of them, holds every name of the table for the platform, points every home variable inside the scratch root, carries a passed variable by name, and its debug text holds the key's name and not its value.
  files: crates/ra-bench/src/env.rs crates/ra-bench/src/env/tests.rs

- The project is set up in scratch the same way everywhere {c2}
  objective: Two machines must start a run from the same bytes and be able to prove it. The setup module makes the scratch root with its eight directories under a short name in the system temp directory, writes the embedded project byte for byte, optionally replaces rhipper.toml with a given settings file, and makes one commit on main with a fixed name, mail, message and date using the scratch git config, returning the commit and the tree.
  acceptance: `cargo test -p ra-bench -q setup_` passes: two setups in two directories give the same commit hash, the working tree's files equal the bundle's bytes with no carriage return added, a settings file changes the tree and the commit, and the scratch path is at most forty characters longer than the temp directory's.
  files: crates/ra-bench/src/setup.rs crates/ra-bench/src/setup/tests.rs

### 3. From a walk to a record

- The record and the chained ledger {c3}
  objective: A result has to be machine-readable and impossible to edit without trace. The record module holds the types of section 6 of docs/fable/workflow-bench.md and the three scores computed from the cards. The ledger module appends a line of hash, tab and JSON whose prev is the hash of the last line, and verifies a file: each hash is the hash of its JSON as written, each prev is the hash of an earlier line or sixty-four zeros, and a baseline mark names a batch that exists.
  acceptance: `cargo test -p ra-bench -q ledger_` passes: three appended records verify, changing one digit of the second names line 2, recomputing that line's hash names line 3 as an orphan, two ledgers that each appended to a shared start verify after their lines are concatenated, a record with a field this version does not know still verifies and reads, and the scores of a record with five verified first passes of eight and 3.82 dollars are 0.625 and the dollars over its verified cards.
  files: crates/ra-bench/src/record.rs crates/ra-bench/src/record/tests.rs crates/ra-bench/src/ledger.rs crates/ra-bench/src/ledger/tests.rs

- What a walk did, read from its ledger, feed and history {c3}
  objective: Every number of a record comes from what the walk left behind, and the feed's wording is known in too many places already. The observe module is the one reader the benchmark has: from a project directory it returns, for each card, rounds, reviews, rejections, the six sent-back counters, turns, tokens in, cached and out, the first run's tokens in, dollars, review dollars, seconds and whether it landed, per the table of section 6 of docs/fable/workflow-bench.md, and the walk's stop line. It checks itself with three invariants (the dollars add up, an accepted card has a landed line, every start has an end) and returns an error that names the broken one.
  acceptance: `cargo test -p ra-bench -q observe_` passes: from a usage ledger, a feed and a walk's output written in the test with the shapes of the live ones, a card with one start, one accepting review and one landing is a first pass though its run wrote two usage rows, a card that continued in its session once is two rounds, a card with a lint send-back, a conflict and a rejection counts one of each, a card title holding the words in and out does not shift a number, dollars with no card are unattributed, and each of the three invariants, broken in turn, is the error returned.
  files: crates/ra-bench/src/observe.rs crates/ra-bench/src/observe/tests.rs

- The scans for the key, the machine and the outside {c2}
  objective: A run that used a login, read outside its directory or carried the person's paths must not count, and a key must never rest in a kept file. The scan module walks an evidence directory: replaces every occurrence of a given secret and counts them, counts occurrences of given machine strings (home path, user name, hub path), counts tool calls in the stream files whose path arguments leave a given root, counts rate limit events that carry plan windows, and reads the init event's key source from a stream.
  acceptance: `cargo test -p ra-bench -q scan_` passes: a stream holding the key twice is rewritten without it and reports 2, a system prompt holding the home path reports 1, a Read call on a path above the root and a Grep call with a relative path inside it report 1 between them, a rate limit event with a seven day window reports 1, and the init event's key source is returned as written.
  files: crates/ra-bench/src/scan.rs crates/ra-bench/src/scan/tests.rs

- The benchmark's own check, with no key and no dollars {c3}
  objective: A change to the agent that no longer fits the benchmark must be caught in that change, not at the next paid run. The check module's one function runs the parts of section 8 of docs/fable/workflow-bench.md that need only this crate (parse, pins, red and green) and returns one line a part with its reason on failure. The pin module's rewrite updates the manifest's hashes after a deliberate edit, and refuses when a card's hash changed and its version did not.
  acceptance: `cargo test -p ra-bench -q check_` passes: on the embedded benchmark the cheap parts pass, a bundle with one word of a card changed fails the pins part and names the card, the rewrite refuses it until the card's version is raised and then pins the new hash, and a changed base file is refused until the benchmark's version is raised.
  files: crates/ra-bench/src/check.rs crates/ra-bench/src/check/tests.rs crates/ra-bench/src/pin.rs crates/ra-bench/src/pin/tests.rs bench/workflow/bench.toml

- One command runs the benchmark and writes its record {c4}
  objective: The pieces become the command the person asked for. A new subcommand of rc, bench, with the verbs check, run, compare, gate, verify and pin, all declared here and handled in a new file apps/rc/src/bench.rs; compare, gate and verify answer not built yet until their cards. Run follows the eleven steps of section 5 of docs/fable/workflow-bench.md through two traits, one that starts the walk and one that probes the harness, so the flow is tested with neither a model nor a key: refuse without the key, scratch, setup, clean environment, probe, walk with a watch on dollars, minutes and a card's rounds, collect into evidence, judge, scan, record, append, move the evidence beside the ledger. Repeats share a batch id. Check prints the parts and exits 1 on a failure.
  acceptance: `cargo test -p rc -q bench_run` passes: without the key the command exits 2 and names the variable before anything is created, with a scripted walk that writes a canned feed and usage ledger and lands a canned tree the ledger gains one verifying line whose cards and scores match the canned walk, a probe that reports a login stops the run before the walk, a scripted walk that passes the dollar limit is stopped and recorded with budget as its stop and its unlanded cards unverified, a scripted stream that carries a plan window makes the record void for a login, two repeats share a batch, and no file of the evidence holds the key.
  files: apps/rc/src/main.rs apps/rc/src/bench.rs apps/rc/src/bench/tests.rs apps/rc/Cargo.toml

### 4. Two runs compared

- An exact permutation test over whole runs {c3}
  objective: The same commit gives different numbers twice, so a difference means nothing until it is larger than chance makes. The compare module takes two batches of records, keeps the comparable ones and the cards whose key and version match on both sides, computes the three scores of each run, and for each score the difference of the batch means, its p-value over every deal of the runs into two groups (sampled with a seed made from the batch ids above a million deals) and the noise floor as the 95th percentile of the absolute difference over the deals. It returns cannot judge, better, worse or no difference by the table of section 7 of docs/fable/workflow-bench.md, and for one batch against itself only the floors.
  acceptance: `cargo test -p ra-bench -q compare_` passes: five runs at 0.75 against five at 0.25 is better with a p-value of 1 in 252, the same batches swapped is worse, two batches drawn from one set of values is no difference and prints a floor, a batch with a void run cannot be judged and the reason names the run, a run stopped for budget is scored as it stood, records with different settings hashes are refused with the reason, a card whose version differs is left out and counted, and the answer for a sampled case is the same on two calls.
  files: crates/ra-bench/src/compare.rs crates/ra-bench/src/compare/tests.rs

- The compare and gate commands, and the baseline mark {c3}
  objective: The gate is what fails when a change makes the agent worse. Compare takes two selectors (a batch id, a commit, the word baseline or the word last), prints one row a card with first passes and median dollars on each side, the three scores with their differences, p-values and floors, and the verdict on one line; with one batch and the aa flag it prints the floors. Run gains a baseline flag that appends a baseline line naming its batch. Gate runs a batch, compares it with the newest marked baseline, and exits 0 for better or no difference, 1 for worse, 2 when it cannot judge, saying how to renew a baseline older than 14 days.
  acceptance: `cargo test -p rc -q bench_gate` passes: over a ledger written in the test and a scripted walk, a batch worse than the baseline exits 1 and prints the score that fell, an equal one exits 0, a ledger with no baseline exits 2, a baseline fifteen days old exits 2 and names the versus flag, and the baseline line verifies in the chain.
  files: apps/rc/src/bench.rs apps/rc/src/bench/tests.rs

- Two builds measured in the same hour {c2}
  objective: A baseline from last week met a different model snapshot and a different API hour. The versus flag of run takes the path of a second rc and alternates the two builds for the given repeats, each side in its own batch, then prints the comparison. The record's agent block comes from the version text of the binary that walked, which may be an old one with no commit in it.
  acceptance: `cargo test -p rc -q bench_versus` passes: three repeats with a second scripted walker give six records in the order first, second, first, second, first, second, in two batches of three, each record naming its own walker's version, followed by one verdict line.
  files: apps/rc/src/bench.rs apps/rc/src/bench/tests.rs

- Verify a ledger, its anchor and a run's evidence {c3}
  objective: A result is worth what can be checked about it later. Verify checks the chain and prints the first faulty line with its reason. With the anchored flag it also checks that every line of the ledger at the remote's main branch is still in the working copy. With the replay flag and an evidence directory it recomputes the evidence hash, unbundles the landed tree, runs the hidden checks on it again, re-derives each card's tokens and dollars from the kept streams' result events, and names every field of the record that disagrees.
  acceptance: `cargo test -p rc -q bench_verify` passes: a good ledger exits 0, an edited one exits 1 and names the line, a ledger missing a line that a scripted remote copy has is named under the anchored flag, a replay of evidence written by a scripted run agrees, and the same evidence with one stream's cost changed names that card's dollars.
  files: apps/rc/src/bench.rs apps/rc/src/bench/tests.rs crates/ra-bench/src/ledger.rs crates/ra-bench/src/ledger/tests.rs

### 5. Lockstep with the agent

- The agent's own walk, scripted over the benchmark's roadmap, stays readable {c3}
  objective: The parts of the benchmark's check that need the walk cannot live in ra-bench: whether the agent admits the cards, derives their checks, routes them, and leaves a feed observe can read. ra-console gains ra-bench as a dev-dependency and four tests on the embedded project: the admission lint reports no fault and the preflight rejects no card's command; each card's checks as the walk derives them equal the manifest's check list; the embedded rhipper.toml loads and routes all eight cards to the manifest's model at zero and at one rejection with the reviewer as named; and a walk with the scripted provider that lands all eight leaves a project directory observe reads with every invariant holding and eight first passes.
  acceptance: `cargo test -p ra-console -q bench_lockstep` passes: the four tests named above, each reporting on failure the card and the difference.
  files: crates/ra-console/Cargo.toml crates/ra-console/src/commands/tests.rs

- The benchmark's check runs on every push {c1}
  objective: Lockstep is only as good as how often it is checked. The build workflow gains a step after the tests that runs the two ignored tests of ra-bench by name, and a new workflow, bench.yml, started by hand, installs Claude Code, takes the key from a secret and runs the gate with a repeats input, uploading the ledger and the evidence as an artifact.
  acceptance: `cargo test -p ra-bench -q workflows_` passes: the build workflow's text names both ignored tests, and the bench workflow's text has a manual trigger, reads the key from a secret into the environment of the one step that runs the gate and of no other, and uploads the results.
  files: .github/workflows/build.yml .github/workflows/bench.yml crates/ra-bench/src/check/tests.rs

- An install says what the benchmark knows about the build {c2}
  objective: The moment a worse agent starts to cost money is the moment it is installed. The install command looks for the build's commit in the ledger of the repository it installs from and adds one line to what it prints: no benchmark record for this build, or the newest verdict a batch of this commit got against its baseline with the three scores. It never refuses.
  acceptance: `cargo test -p ra-console -q install_names_the_benchmark` passes: with a ledger holding a batch of the build's commit the line carries its verdict and first pass score, with a ledger that lacks the commit the line says there is no record, and with no ledger the install prints what it printed before.
  files: crates/ra-console/src/install.rs crates/ra-console/src/commands/tests.rs crates/ra-console/Cargo.toml

- The first baseline and the calibration it shows {c2}
  objective: The design's guesses (cost, spread, which cards are too easy or too hard) end with the first real batch. This card is run by the person with the key: ten repeats marked as the baseline, the aa floors read, and any card with a verified first pass in none or in all ten runs noted for rework. What it adds to the tree is a short page, docs/bench.md, that says how to run, compare, gate and verify, what the three scores mean, and the measured floors and cost in place of this design's estimates.
  acceptance: `cargo test -p ra-bench -q page_` passes: the page names the six verbs, the three scores and the variable the key comes from, and holds no line that says estimate.
  files: docs/bench.md crates/ra-bench/src/check/tests.rs

- A record can be signed {c2}
  objective: A chain shows that history was not edited; a signature shows who wrote the tip. With a sign flag and the path of an SSH key, run signs each record's hash with ssh-keygen under the namespace rhipper-bench and appends hash, tab and signature to bench/results/ledger.sig; verify with a signers file checks every signature present and names a record whose signature fails, and a ledger with no signatures verifies as before.
  acceptance: `cargo test -p rc -q bench_sign` passes: with a scripted signer a run leaves one signature line for its record, verify accepts it, a signature moved to another record's hash is named, and a ledger with no signature file exits 0.
  files: apps/rc/src/bench.rs apps/rc/src/bench/tests.rs crates/ra-bench/src/ledger.rs crates/ra-bench/src/ledger/tests.rs
