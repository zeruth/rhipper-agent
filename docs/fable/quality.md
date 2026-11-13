# Quality pipeline and pass rate: findings, design, roadmap

Written 2026-11-13 from the code at `787be7f`, the agent's feed (`var/pair.log`), the retro logs and the handoffs under `var/context/`. The runelite and asm feeds live in the hub directory, which this session could not read; their numbers are the ones in the brief plus the cases the code's own comments cite.

How to import the roadmap: a dry `rc import` of this file (`apps/rc/src/main.rs:1486`) reads Part 2 only. Part 1 is written with no `##`, `###` or dash-bullet lines, which is all `outline::parse` (`crates/ra-roadmap/src/outline.rs:103`) takes as items; everything else is prose to it.

# Part 1: Findings and design

#### The short version

A card fails its first review because four parties hold four different ideas of "done", and nothing forces them to agree before the run starts:

| Party | What it is told "done" is | Where |
|---|---|---|
| The check | the backticked spans of the acceptance that `is_command` takes, else a fallback | `agent.rs:6386-6453` `checks_of`, `ra-roadmap/src/check.rs` |
| The worker | the acceptance as prose; "do not build … right by reading is the normal case" and also "Run them before reporting" | `agent.rs:8499` `SYSTEM`, `ra-context/src/lib.rs:224` |
| The lint | pedantic clippy plus `unreachable_pub` on every changed line, a new file whole | `agent.rs:6127-6196`, `Cargo.toml:41-45` |
| The reviewer | "whether the diff meets the acceptance and whether the handoff tells the truth about it", read from prose, 6000 chars of notes and a 60k diff, in four turns with no workspace set | `agent.rs:1149`, `agent.rs:1546-1559`, `agent.rs:1702-1795` |

Each gap between two rows is a class of rerun. The design below closes them; the rows are in the order of the sections that follow, and the rank is the expected effect on first-review pass rate and reruns over the three projects:

| Rank | Change | Rerun class it removes | Feed evidence | Measure |
|---|---|---|---|---|
| 1 | One admission gate on every way a card enters, and a planner that sees the workspace | cards that cannot pass as written | #240, #241, #245 of the last walk (0 of 3 landed); asm #131/#132 (270 and 234 runs on one bad argument, `agent.rs:4834-4836`) | cards blocked at the gate; share of first runs whose check is the card's own command |
| 3 | Lint and a failed check settled inside the run that caused them | "checks failed" reruns: agent 49 | #219: nine runs, 142 turns (`var/pair.log:846-976`); #240 sent back on lint alone | reruns by cause; continuations a card; turns a landed card |
| 2 | One contract rendered to worker and reviewer; a verdict that is parsed strictly and whose points must cite | substantive review rejections: agent 14, asm 173, runelite 35 | 19-point and 28-point "verdicts" (`var/pair.log:850`, `:958`); #185 rejected 9 times (`:497-543`) | first-review pass rate; points a rejection, by kind; malformed verdicts |
| 4 | Loops that end on lack of progress, not on lack of a changed file | absolute-card and three-strike churn | asm #80, 109 rounds; six cards blocked then started again | rounds a card; blocks that held |
| 0 | The measures themselves, and a replay set for reviewer changes | (none; it makes 1-4 provable) | PASS RATE is reviews accepted over reviews plus failed checks (`apps/rc-gui/src/metrics.rs:168-172`); no figure says whether a card passed first time or why it was rerun | `rc quality` |

Rank 0 is built first because nothing else can be shown to work without it.

One number in the brief needs reading before it is used to rank. asm's "checks failed 507" looks like the largest target on the table. The code's own comment counts 270 runs of asm #131 and 234 of #132 "in an afternoon, a turn each" on a check cargo rejected on its arguments (`agent.rs:4834-4836`): 504 runs. If those are the same runs, asm's check failures were two cards with one malformed command, which the preflight (#219) and the fingerprint hold already stop, and asm's live problem is the other line: 173 review rejections against 16 passes. That is why the contract and the verdict rank above the lint, and why the gate ranks first: a card that cannot pass as written feeds both columns.

The effect sizes below are estimates from one project's feed and two projects' totals. Each is stated with what it is derived from; none is measured until Section A of the roadmap lands.

#### Finding 1. Cards enter the roadmap by five doors and only one has a guard

`tree::lint` and `definition_of_done` (`crates/ra-roadmap/src/tree.rs:189-266`) and the preflight (`agent.rs:6459-6548`) are the card-quality rules. They run in one place: `rc check` (`apps/rc/src/main.rs:1445-1460`), which a person chooses to run. The doors that do not run them:

1. `rc import` (`apps/rc/src/main.rs:1486`): `outline::parse` then `store.import`.
2. `rc add` (`apps/rc/src/main.rs:1462`).
3. The planner (`crates/ra-console/src/commands/plan.rs`, `plan_to` → `insert_card`). Its only acceptance test is `has_check` in `crates/ra-console/src/plan/propose.rs`, which is true for any non-empty backticked span. `is_command` (`crates/ra-roadmap/src/check.rs`) is the real rule and the planner does not call it.
4. The look-back pass every 10 landed cards (`crates/ra-console/src/lookback.rs`, `EVERY = 10`), which calls the same `plan`.
5. `split_points` (`agent.rs:1261`): each `[card]` point of a review becomes a card with a title of 80 chars and an objective, with no acceptance and no files.

The planner and the look-back write cards in one turn with no tools and no workspace (`propose.rs`, `agent.rs:1857-1903`); the brief (`plan/prompt.rs` `brief`) holds evidence lines and open titles, not one path of the source tree. So they invent. The last walk of the agent ran three look-back cards and landed none (`var/pair.log:1052-1096`):

| Card | What the planner wrote | What happened |
|---|---|---|
| #240 | acceptance "`rc wait` with one background job running prints `waiting <id>`…": not a command a machine runs | check fell back to `cargo test -p rc -q`, which passed and proved nothing; the worker "added no tests"; sent back on 5 lint warnings in the new file (`var/context/240.md`), 15 turns |
| #241 | acceptance "`rc handoff --dry-run <card>` prints a `machine:` block": no such subcommand; files `crates/`, `(handoff/prompt`, `builder)`, `docs/context.md`: none exists | check fell back to `cargo test --workspace -q` and timed out after 1200 s (`var/context/241.md:25`); the worker wrote "The acceptance check will fail as written", 12 turns |
| #245 | acceptance "`cargo test -p <review-crate> relocate_points`": a placeholder for a crate | 13 turns, not landed |

Forty turns and twenty minutes of a check slot went to three cards a string comparison could have refused. The same class in the code's own comments: asm #131 and #132 ran 270 and 234 times on a check cargo rejected on its arguments (`agent.rs:4834-4836`, `agent.rs:5569-5571`); #128 and #145 named files that did not exist (retro logs).

A fourth planned card shows how an unrunnable acceptance turns into a review rejection (`rhipper.md:1372-1377` in the main checkout). Its acceptance was four `rc gate "…"` lines that "each print `allow`"; `rc` is not one of the 34 `CHECK_PROGRAMS`, the subcommand did not exist, and its files were `crates/ (shell allowlist/gate) .claude/settings.json`. The worker built the subcommand, passed `cargo test -p ra-gotchas -q`, and reported "I did not run `rc gate` itself on the four acceptance commands" (the hooks refuse a live `rc.exe`; retros of #43 and #49 say the same). The reviewer sent it back: "run the four acceptance commands through `rc gate` and report what they print". Worker and reviewer were both right by what each was told. The card was wrong: it asked for proof no party in the pipeline is able to produce.

One hole below the gate makes it worse: a filtered `cargo test -q <filter>` that matches no test exits 0. Nothing in the tree reads `running 0 tests` (a search of every `.rs` file finds no handling). A card whose acceptance names a test the worker never wrote passes its check.

#### Design 1. `admit`: one gate, called by every door

A new module `crates/ra-roadmap/src/admit.rs`, pure and synchronous so every caller can afford it:

```rust
/// What the gate knows of the workspace; ra-console fills it once a pass.
pub struct Workspace {
    pub root: PathBuf,
    pub crates: Vec<String>,        // cargo members when a Cargo.toml is there, else empty
    pub named_checks: Vec<String>,  // the project's `[worker] check` names
}

pub enum Fault {
    NoCommand,                         // acceptance has no span `is_command` takes
    Placeholder { command: String, mark: String },   // `<card>`, `<review-crate>`, `…`
    UnknownCrate { command: String, name: String },  // `-p x` and x is no member
    NoFilter { command: String },      // `cargo test --workspace` on a card of < 3 crates
    MissingScript { command: String, path: String }, // `python tools/x.py`, no such file
    NotAPath { token: String },        // `(handoff/prompt`, `builder)`
    MissingFile { path: String },      // no file, and its parent directory is not there either
    Unproven { clause: usize },        // a clause no command proves (see Design 3)
}

pub fn faults(node: &Node, workspace: &Workspace) -> Vec<Fault>;
pub fn repaired(node: &Node, workspace: &Workspace) -> Option<Node>;
```

Rules that hold for every project come first (placeholder, path shape, file or parent exists, script exists, `is_command`); the cargo rules run only where `crates` is not empty, so a Gradle or Python project gets the gate without them. `MissingFile` allows a new file: the path is fine when its parent directory exists.

`repaired` is the deterministic part and needs no model: a `-p` crate derived from the files with `check::crate_of`, path tokens trimmed of brackets and backticks, a `--workspace` narrowed to the crates of the files.

The flow, the same for all five doors:

1. Build the node as today.
2. `faults(&node, &workspace)`; when not empty, try `repaired` and gate once more.
3. Still faulty and the door is a model (planner, look-back): one repair turn (Design 1b).
4. Still faulty: the card is inserted `Blocked`, its notes headed by one `gate:` line a fault, and the feed says `#N held at the gate: <first fault>`. The walk never starts it. `rc check` prints the same lines, so there is one vocabulary.

A card held at the gate costs zero worker turns. That is the whole point: #241 cost 12 turns and a 20-minute check to say what `NoCommand` says for nothing.

What moves: `definition_of_done` and the per-card half of `tree::lint` become callers of `admit::faults`; `rc check` keeps its output. What it removes: `has_check` in `propose.rs`; the fallback to `cargo test --workspace -q` for a new card that named no command (`checks_of` keeps the fallback only for cards that predate the gate).

**1b. A planner that sees the workspace.** The planner's brief gains a fixed block, built without a model, of under 60 lines: the crates and their test files (`crates/ra-console/src/agent/tests.rs` and the like), the paths the cited evidence lines name with whether each exists, the project's named checks, and for a cargo project the convention in two lines ("a test is named as a sentence in `tests.rs` beside the code; the acceptance is `cargo test -p <crate> -q <words of the test's name>`; what a command of the project's own binary prints is asserted in such a test, never by running the binary"). The planner's instruction (`plan/prompt.rs` `INSTRUCTION`) already says "the command that checks the card in backticks and what it prints"; it lacks the facts to obey it. When a card of the answer fails the gate, the faults go back in one more turn ("card 3: `-p review-crate` is no crate; the crates are …"), and only what still fails is held. This is a one-turn, no-tool role and fits the local model.

**1c. A filter that runs no test fails.** After a check whose command is `cargo test` with a filter, the orchestrator sums the `running N tests` lines; a sum of 0 is `check failed: the filter <words> matched no test`. The rerun prompt then says the one actionable thing: write the test the acceptance names.

Measured by: `gate held` events against cards entered, by door; first runs whose check was a fallback (target 0); cards of a look-back pass that land on their first run (last walk: 0 of 3).

Expected effect: on the agent, the three look-back cards of the last walk are 3 of its 7 runs. On asm, about 504 of the 507 check-failed reruns look like two cards with one unrunnable command (above); the preflight now stops that case at the first dispatch, and the gate stops it, with its cousins the preflight cannot see (a placeholder, a missing file, a test that does not exist), before any dispatch. On runelite, the 35 rejections "on points only a person or CI could settle" are cards whose acceptance asks for what no machine gives: `definition_of_done` names them, and only under `rc check`. This is ranked first because it is the cheapest change, it has a known floor, and every later section assumes a card that can pass.

#### Finding 2. The lint is a second, unannounced reviewer, and its findings cost a whole rerun

When the check passes, `run_task` lints the changed lines and a single warning turns the run to needs revision (`agent.rs:5595-5620`: `passed = false`). The workspace lints are `clippy::pedantic` and `unreachable_pub` (`Cargo.toml:41-45`), and a file git does not know counts whole (`agent.rs:6176-6181`). The worker is told "do not hunt warnings yourself" and "do not build at all when reading the code … tells you it is right" (`agent.rs:8499`). So the common first run of a card that adds a file is: right by reading, check passed, sent back for `pub` that should be `pub(crate)` (#240: four `unreachable_pub` and one `single_match`).

A lint send-back is counted as a check failure against `CHECK_RETRIES = 2` (`agent.rs:2884`), runs in a fresh session unless the card is absolute (`agent.rs:8250` `rerun_resume`), and so re-reads the code at 60-110k tokens a turn. The reruns of one walk in the feed: #222 5 turns at 80k, #220 4 turns at 89k, #219 4 turns at 63k, #226 3 turns at 60k, #223 5 turns at 112k, each ending needs revision again, and all of #219-#223 and #226 "blocked: sent back 3 times in a row in this walk" (`var/pair.log:841-929`). The retro logs carry about 25 lines of the same friction: the warnings were only in the notes, the hook refused clippy before a change, the build budget of two counted `cargo fmt`, "76 warnings in old code sent the card back twice", "the rerun prompt repeated all 89 lint warnings". #221's last answer: "I never found out which of the 80 warnings in the changed files this card added".

One card shows the whole cost. #219, a c2 card started one rung down, has nine run-end lines between `var/pair.log:846` and `:976`: 55, 3, 34, 7, 4, 9, 7, 7 and 16 turns, 142 in all, about 5.4M input tokens. It was rejected once on a 19-point haiku answer, blocked twice on three send-backs (`:856`, `:896`), started again both times, and accepted by a sonnet reviewer in 764 output tokens (`:975`). The feed does not say what each of the other send-backs was for, a failing test or a warning; the card's own retro lines are about lint. That the feed cannot say is the first card of the roadmap.

The lint fix pass (`agent.rs:1346-1411`) would take this off the worker, and is off unless a `[role] fix` is named or a local server answers: "the fix role is off: the lint warnings wait for the person".

#232 and #245 work on which lines the lint blames. They do not change what a warning costs: a rerun.

The same shape holds for the check itself. The worker is told two things: `SYSTEM` says build at most twice and normally not at all; the context says of the acceptance commands "Run them before reporting" (`ra-context/src/lib.rs:224`). A worker that obeys the first learns of a failed test from the next run's prompt, in a new session.

#### Design 2. Settle lint and a failed check inside the run

Three steps, each cheaper than the one after it, all before the card's state is set:

**2a. Apply the fixes clippy already wrote.** Run the lint with `--message-format=json`. A diagnostic whose primary span lies in the card's changed ranges (`ra_tools::changed_ranges`, `crates/ra-tools/src/lib.rs:383`) and whose suggestion is `MachineApplicable` is applied by the orchestrator, last span first within a file. `unreachable_pub`, `single_match`, `uninlined_format_args`, `must_use_candidate` and most of what the retros list are of this kind. Then lint once more. No model, no turn. A new function in `ra-tools`:

```rust
pub struct Suggestion { pub file: String, pub start: (u32, u32), pub end: (u32, u32), pub text: String, pub lint: String }
pub fn suggestions_within(json: &str, ranges: &[(String, u32, u32)]) -> Vec<Suggestion>;
pub fn apply(root: &Path, suggestions: &[Suggestion]) -> std::io::Result<usize>;
```

**2b. A primer so the worker writes it right.** The handoff already keeps a run's `warnings`. From the last 50 handoffs of the project the agent counts lint names and writes the top eight, one line each with the fix, into the worker's context ("`unreachable_pub`: 31 times; in a binary crate write `pub(crate)`"). It is derived for each project from its own handoffs, so asm and runelite get their own from their own linters' names; nobody maintains it. About 8 lines of context against a rerun.

**2c. One continuation in the warm session.** What 2a leaves, and a failed check that is not a setup failure, not red on main and not a timeout, goes back once into the session that just reported, as a message of only the failure (the run already does this once for an empty report, `agent.rs:5245-5266`, with `resume: Some(session)` and `REPORT_TURNS`): the failed command, the first error, the failing tests' output, or the warnings with their source lines. Cap 8 turns. Then check and lint run again and the card's state is set from that.

```rust
struct Continuation { cause: Cause, text: String, turns: u32 }   // Cause: Check | Lint
const CONTINUE_TURNS: u32 = 8;
const CONTINUE_UNDER: u64 = 90_000;   // tokens of the session's last turn; over it, the fix role or a rerun
```

This goes against a decision already in the code, and the reason must be met, not ignored. `rerun_resume` (`agent.rs:8246-8255`) starts every rerun of a card that is not absolute in a fresh session because a resumed one "paid its whole transcript every turn of a one-line answer (#210: about 990 heavy-turn lines at 26k to 86k tokens a turn)". Three things make the continuation a different case. It is bounded: one continuation, 8 turns, and only when the session's last turn was under `CONTINUE_UNDER`; #210's cost came from resuming without a bound. Its message is the failure alone, not the rerun block. And the fresh reruns it replaces are not cheap either: the feed's reruns of #219 to #226 ran 3 to 5 turns at 60-112k tokens a turn, because a fresh session reads the code in again before it can touch the one line. Whether the bound holds is a number, not an argument: `rc quality` prints tokens a continuation beside tokens a rerun, and if the first is not well under the second the threshold drops or the card is reverted.

A continuation is not a rerun and does not spend `CHECK_RETRIES`; the feed says `#N continued: lint, 2 turns` so the measure stays honest: turns and tokens a landed card must fall, not only the rerun rate.

**2d. One rule for the worker.** `SYSTEM` keeps "right by reading"; the context line 224 loses "Run them before reporting" and gains "the orchestrator runs them when you report and returns a failure to you once, in this session". The two texts then agree, and the worker's build budget stops being spent on what the orchestrator runs anyway.

What it removes: lint warnings as a cause of `NeedsRevision` on a first run in the common case; the dependence of lint fixing on a configured fix model; the contradiction in the worker's instructions.

Measured by: reruns by cause (lint, check, review, merge, restart) from the feed; continuations a card and their turns; average turns and tokens a landed card. Expected effect: the agent's "checks failed 49 (many on main itself)" splits between #235 and this, and how it splits between a failing test and a warning is not recorded anywhere that lasts. The walk's send-back line says "its check failed" for both (`agent.rs:2797-2805`), the dashboard counts both as a failed check (`apps/rc-gui/src/metrics.rs:108-111`), and the handoffs that hold the difference are archived when a card lands. Of the 21 handoffs still under `var/context/`, 3 are a passed check sent back on lint and 5 a failed check. The retro logs give the other bound: about 25 lines of lint friction over #207, #210, #219, #220, #221, #223, #226 and #238. Section A's first card puts the cause on the feed line so the next walk can say. On asm and runelite the effect is unknown until their feeds carry the cause; a failing test the worker would have fixed on seeing its output becomes a continuation of a few cached turns in any project.

#### Finding 3. The reviewer judges something the worker was not told, and its answer is parsed too kindly

**The instruction.** `REVIEW_INSTRUCTION` (`agent.rs:1149`): "Judge whether the diff meets the acceptance and whether the handoff tells the truth about it." The acceptance in the brief is the card's prose, whole (`agent.rs:1702-1795`). The check ran only its backticked commands. Every clause of prose no command proves is the reviewer's to judge from a diff, and "the handoff tells the truth" invites a point for each sentence of a report. The planner's own rule says the acceptance holds "nothing a reviewer must judge in prose" (`plan/prompt.rs`); nothing enforces it and the reviewer is not told it.

**The brief.** The reviewer gets "Notes of the card", the first 6000 chars (`DIGEST_NOTES`). Notes hold old reports, pasted clippy output (the notes of #92 hold one clippy run three times) and earlier rounds. That is material for points that have nothing to do with the diff.

**The parser.** `verdict_of` (`agent.rs:1166-1200`): an answer that starts with neither word falls through, and every line of it becomes a point. The feed shows what that makes: #219 "19 points" from an answer of 15,034 output tokens (`var/pair.log:850`), #226 "28 points" (`:958`). `max_tokens: Some(1500)` (`agent.rs:1550`) is not enforced on the Claude Code route; accepted haiku reviews ran 6.5-9.8k output tokens. A point needs no file, no line, no tie to the acceptance; an untagged point counts as work.

**The weight.** `classify` (`crates/ra-console/src/commands/review.rs`) calls a rejection trivial only when every point matches one of 13 words and none of 15, and the 15 include `test`, `fail`, `return`, `wrong`. All 27 `review ✗` lines of the agent's feed are "(substantive)". The trivial path has, on this evidence, never fired.

**Repeats.** #185 was rejected nine times running (`var/pair.log:497-543`); asm #53's points went 5, 5, 6, 4. #221 gave the reviewer its last verdict and asks it to keep numbers, and #238 took card and environment points off the rerun. Neither stops a reviewer from finding fresh points in lines it already passed.

**What is already fixed, and stays out of this design:** the haiku reviewer (#228 floors it at Sonnet; the five sonnet reviews of the last walk all accepted at 260-595 output tokens and $0.04-0.07); all-ticks verdicts (#233); kinds and the check output in the brief (#221, #238).

#### Design 3. One contract, a strict verdict, points that must cite

**3a. The contract.** A new `crates/ra-roadmap/src/contract.rs`, derived from the card and nothing else:

```rust
pub struct Contract {
    pub checks: Vec<String>,          // the commands, in order: what `checks_of` runs
    pub criteria: Vec<Criterion>,     // the acceptance, one clause each, numbered from 1
    pub scope: Vec<String>,           // the card's files
    pub person: Vec<String>,          // clauses and `person:` lines no machine or reviewer judges
}
pub struct Criterion { pub number: u8, pub text: String, pub proof: Proof }
pub enum Proof {
    Check(usize),   // the clause holds a command: check N proves it
    Reading,        // no command: the reviewer judges it from the diff
}
pub fn of(node: &Node) -> Contract;
impl Contract { pub fn render(&self, results: &[CheckResult]) -> String; }
```

`of` splits the acceptance into clauses at `;` and at sentence ends outside backticks. A clause with a command is `Check`; a clause that `HUMAN_CLAUSES` matches moves to `person`; the rest are `Reading`. The gate reports each `Reading` clause as `Unproven` (a warning to the one who wrote the card, not a block, at first; the second card of Section C) so that the stock of prose-only criteria shrinks at the source.

`render` is the one function both briefs call. The worker sees it without results; the reviewer sees the same lines with each check's exit beside it:

```text
CONTRACT of #245
Checks, run by the orchestrator in this order; exit 0 is the pass:
  1. `cargo test -p ra-console -q relocate_points`        passed (3 tests)
Criteria:
  C1. the points are matched by text after a merge        proved by check 1
  C2. a warning point quotes the flagged line             by reading the diff
Scope: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
For the person, not for you: (none)
```

**3b. What the reviewer is asked.** The instruction becomes, in substance: for each criterion, is it met? A criterion proved by a passed check is met unless the diff shows the test does not test it, and then cite the test's line. A criterion by reading is met when the diff does it; cite the line that does not. Anything else you see is a note: it is recorded and rejects nothing. What is listed for the person is not yours. The handoff is context; a claim in it that the diff contradicts is a note unless it bears on a criterion.

The review brief drops "Notes of the card". It keeps: the contract with results, the handoff's report (clipped), the last verdict and the points that left (#221, #238), the diff.

**3c. The verdict, parsed strictly.**

```rust
pub enum Verdict { Accepted { notes: Vec<String> }, Rejected { points: Vec<Point>, notes: Vec<String> }, Malformed(String) }
pub struct Point { pub number: u8, pub kind: PointKind, pub criterion: Option<u8>, pub file: Option<String>, pub line: Option<u32>, pub what: String }
// PointKind gains Nit beside Work, Card, Environment
```

The line form: `N. [work|card|environment|nit] C<n> path:line: what`, with `C<n>` or `path:line` required for a work point.

Rules, applied by the parser and not by trust:

1. First line `accepted` or `needs_revision`, else `Malformed`. A malformed answer is asked again once, in the same session, with the form and nothing else. Malformed twice is "no verdict": the card stays in review for the next pass, as a reviewer that ran out of turns does today. It is never points.
2. A work point with neither a criterion of the contract nor a file of the diff is a note.
3. `[nit]` points never reject. They go to the fix pass when one is on, else to the notes.
4. More than five work points: the first five stand, the feed counts the rest. A worker cannot act on 19.
5. `needs_revision` with no work, card or environment point left is `Accepted` with notes: the rule #233 made for ticks, widened.

**3d. A second review is a delta review.** The handoff's verdict stores the commit it judged. On the next review the brief leads with the diff since that commit, and the parser holds a new point (a number above the last verdict's highest) to it: its `path:line` must lie in lines changed since, or cite a criterion the last verdict did not rule on. Otherwise it is a note. A reviewer can keep a point, tick it, or object to what the rerun changed; it cannot start over. That ends rotation by construction.

What it removes: `classify`, `TRIVIAL` and `SUBSTANTIVE` (the whole of `commands/review.rs`, 75 lines), since the reviewer now says `[nit]` itself; the fall-through arm of `verdict_of`; the notes section of `review_brief`; "whether the handoff tells the truth" as a ground to reject.

The risk is a stricter parser that passes bad work. Three guards: the merged-tree check still gates the landing (#231); notes are kept on the card where the person reads verdicts; and the replay set (Design 0) runs any change of instruction or parser over stored briefs before a walk depends on it.

Measured by: first-review pass rate (cards accepted at their first review over cards reviewed; the dashboard's PASS RATE is reviews accepted over reviews plus failed checks, which mixes two causes and counts a card's fifth review like its first); points a rejection by kind; notes a review; malformed verdicts; rejections after which the rerun changed no file (a proxy for a rejection with nothing actionable in it).

Expected effect: of the agent's 27 rejections, 2 were format failures counted as 19 and 28 points, and 8 were repeats on #185; rules 1 and 3d reach those 10 for certain. For runelite's 35 rejections "on points only a person or CI could settle", #238 routes such points after the review; the contract tells the reviewer before it, so the review can pass.

#### Finding 4. Loops end on whether a file changed, not on whether the card got closer

Three counters decide when a card stops being rerun, and all three read a changed file as progress:

1. Inside a walk, `CHECK_RETRIES = 2` and `REVIEW_RETRIES = 1` (`agent.rs:2884-2886`); past them the card is parked until the next walk (`agent.rs:2794-2815`). The counts live in the walk's own map, so every new walk grants them again.
2. Across walks, a card is blocked only at three rounds running "without changing a file" (`agent.rs:4834-4865`, `idle_rounds`).
3. An absolute card is rerun at once while its last handoff changed a file (`agent.rs:2806-2809`).

A run sent back for lint or a failing test nearly always changes a file. So a card that comes back with the same kind of failure each time is never stuck by these rules; it is rerun twice a walk for as many walks as there are. That is #219's nine runs. The router does not help it either: a worker goes up a rung on `rejections / up_after` (`router.rs:603-609`), and `rejections` counts only reviewer send-backs (`agent.rs:2898-2907`), so a card that fails its check five times stays on the model that failed it.

**Absolute cards.** The loop at `agent.rs:1071-1115` reruns while the card is absolute and needs revision, and stops at `idle_rounds() >= RERUNS_A_WALK` (3, `agent.rs:4247`). Idle means no file changed. A card that changes a file every round and gets no closer never stops; asm #80 went 109 rounds on fixtures. The person's rule (`crates/ra-roadmap/src/model.rs:318-325`) is that absolute cards "work until done, no limits unless there is an actual full stop". A round that leaves the same tests failing is not work toward done.

**Three strikes.** #219-#223 and #226 were each "blocked: sent back 3 times in a row in this walk; a person sets it defined again", and each was started again within lines (#219: blocked at `var/pair.log:856`, started at `:863`, blocked at `:896`, started at `:949`). Those lines are from an earlier wording of rule 2; the feed does not say whether the person set the cards defined or a walk took them. Either way the block bought nothing, because nothing about the card or the route changed between tries.

**Follow-up cards.** `split_points` writes cards with no acceptance and no files. Under Design 1 they are held at the gate, which is right, and they need a way out that is not the person.

#### Design 4. Progress as the signal, and a hold that says why

**4a. Progress.** Each handoff's check line carries three counts: failing tests, standing work points, lint warnings.

```rust
#[derive(PartialEq, PartialOrd)]
pub struct Standing { pub failing: u32, pub points: u32, pub warnings: u32 }
```

A round made progress when `Standing` fell in lexical order or the check's fingerprint changed to a different first error. `stalled_rounds()` sits beside `idle_rounds()` in `ra-context` and counts the rounds running without progress off the handoffs, so it holds across walks and restarts as the idle count does.

One rule then replaces the three counters' reading of "moved". A card is rerun while its last round made progress, within the walk's retries as today. At two stalled rounds the stall counts as a rejection for the router, so the next run is a rung up in a fresh session with the standing failures alone; no second ladder is built. At four it is held with `no progress in 4 rounds: the same N tests fail`. The same rule runs an absolute card: it works on, without a cap, for as long as each round gets closer. Reading four rounds without progress as the "actual full stop" of the person's rule is this document's reading, and the person's to confirm; the card that builds it says so.

**4b. Before a stalled card is held, a diagnosis.** Before a card is held on four stalled rounds, one cheap call (local model, one turn, no tools) reads the contract and the stalled rounds' check lines or verdicts and answers one word: `card` (the criteria contradict the tree or each other, the check cannot pass as written) or `work`. `card` sends the card to the gate's repair turn with the diagnosis and holds it; `work` gives it one run on the next rung up with a fresh session. Only after that does the block for the person stand, and its line says which of the two it was. This is the lowest-confidence part of the design; it is last in the roadmap and worth building only if Section A's numbers show blocks still common after Sections B to D.

**4c. Follow-up cards.** `split_points` gives a follow-up the files the point cites and passes it through the gate; the planner's repair turn (1b) writes its acceptance.

Measured by: runs a landed card at the 90th percentile (the tail is where #219 and asm #80 live); stalled rounds a walk; cards held with a diagnosis against cards held without; follow-up cards that left the gate without the person.

#### Design 0. The measures, and a replay set

**`rc quality`.** One module (`crates/ra-console/src/quality.rs`, new) reads the feed and the handoffs and gives the figures every section above names. The dashboard already parses the same feed lines for RERUN RATE and PASS RATE (`apps/rc-gui/src/metrics.rs:108-121`: `end ✗ … needs revision on`, `review ✓`, `landed #`); the module takes that parse as its starting point, and the dashboard can read the struct later so the two never disagree.

```rust
pub struct Quality {
    pub cards_reviewed: u32, pub passed_first_review: u32,
    pub landed: u32, pub runs: u32, pub continuations: u32,
    pub reruns: Causes,            // check, lint, review, merge, restart
    pub rejections: u32, pub points: Kinds,   // work, card, environment, nit
    pub notes: u32, pub malformed: u32, pub no_verdict: u32,
    pub held_at_gate: u32, pub fallback_checks: u32,
    pub turns_a_landed_card: f32, pub tokens_a_landed_card: u64,
}
```

**Replay.** Each review writes its brief and the answer to `var/reviews/<id>-<n>.md`. `rc review replay --model <m>` sends the stored briefs to a model with the current instruction and parser and prints, against what happened to each card afterwards: agreed, would have accepted what was later changed, would have rejected what later landed unchanged. A change to `REVIEW_INSTRUCTION` or a trial of the local model as reviewer is then judged in minutes on a hundred stored reviews, not by a walk. It is also how the question "can a local model review c1 and c2 cards" gets an answer with a number on it.

#### What on the local model

Every role this design adds is one turn, no tools, a short fixed form: the planner's repair turn, the verdict re-ask, the third-strike diagnosis, the follow-up card's acceptance. The machine-applicable lint fixes and the gate need no model at all. The reviewer on c1 and c2 cards is the candidate with the largest saving, and the replay set is how to qualify it.

#### What this does not touch

Phase AA: red main (#235), the conflict pass (#236), same-file cards (#237), kinds off the rerun (#238). Also #227, #228, #231, #232, #233, #219, #220, #221, #224, #225 and the open look-back cards #240-#247. Where a card below stands on one of these, its objective says which.

#### Two cards from the feed, run through the design

**#241** (12 turns, a 1200 s check, not landed). At the look-back's insert, `faults` gives `NoCommand` (`rc handoff --dry-run <card>` starts with no check program) and `NotAPath` for `(handoff/prompt` and `builder)`. `repaired` has nothing to derive a crate from. The repair turn gets the three faults and the workspace block, and has what it needs to write `cargo test -p ra-console -q <the test's words>` with files `crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs`, which is where the worker ended up after searching (its handoff: changed `agent.rs`, read `commands/tests.rs`). The gate does not catch `docs/context.md`, a file that passes as new because `docs/` exists; the worker's "Faster next time" line did, and that is the residue the gate leaves. The run then has a filtered test for a check in place of the workspace suite, 1c makes sure the test exists, and the reviewer reads "C1, proved by check 1, passed".

**#219** (nine runs, 142 turns, landed). The 19-point answer is `Malformed` and asked again, not a rejection. The send-backs with a passing check and warnings go through 2a and 2c inside the run. If the card still came back twice without `Standing` falling, the third run would be a rung up; in the feed it reached opus only "after 1 rejection" by the reviewer, the one that was a format failure. How many of the nine runs remain is not knowable from the feed; the first-run count in `rc quality` is what will say.

#### What the design removes

| Removed | Where | Replaced by |
|---|---|---|
| `has_check` (any backticked span is a check) | `plan/propose.rs` | `admit::faults` |
| The workspace-suite fallback for a new card with no command | `agent.rs:6386-6453`, `check.rs:8` | the gate holds the card |
| `classify`, `TRIVIAL`, `SUBSTANTIVE` | `commands/review.rs`, whole | the reviewer's `[nit]` |
| The fall-through arm that makes points of every line | `agent.rs:1166-1200` | `Verdict::Malformed`, one re-ask |
| "Notes of the card" in the review brief | `agent.rs:1702-1795` | the contract with check results |
| "whether the handoff tells the truth" as a ground to reject | `agent.rs:1149` | a nit unless it bears on a criterion |
| "Run them before reporting" | `ra-context/src/lib.rs:224` | the continuation |
| A fresh rerun for a first lint warning or failed test | `agent.rs:5595-5665` | fixes applied, then one continuation |
| "A file changed" as the test of progress | `agent.rs:2808`, `:4843` | `Standing` and `stalled_rounds` |

#### Questions that are the person's

1. Is four rounds without progress a full stop for an absolute card (Design 4a)? The rule of 2026-11-12 says no limits short of one.
2. Should a clause no command proves hold a card at the gate, or only warn? The design starts with a warning, because points carry their criterion and `rc quality` can then count first-review rejections on by-reading criteria; if they are most of what is left, the warning becomes a hold.
3. The handoff's truth stops being a ground to reject. The person reads verdicts, not diffs, so a false report matters; the notes of a review are kept on the card for that reason. If they go unread, the alternative is a `[handoff]` kind that sends the report back to the session for a correction without a rerun of the work.
4. Design 2a edits a card's files with no model reading the result. It applies only what clippy marks machine-applicable, inside the card's own lines, and the handoff says `lint fixes applied: N`. If that is still too free a hand, 2c alone does the job at a few turns a card.
5. This session could not read the asm and runelite feeds. Section A's `rc quality` run in each project is the first thing to do with it: the split of asm's 507 check failures between unrunnable acceptance, lint and real test failures decides whether Section B or Section D goes first there.

# Part 2: Roadmap

Twenty-six cards in five sections, in build order. Each acceptance is one filtered `cargo test` and holds no other backticked span and no clause for a human; the worker names its tests so the filter matches. A path listed that is not in the tree yet is named as new in the card's objective. The scripted provider the tests use is `says(...)` in `crates/ra-console/src/commands/tests.rs` and `Worker::with_script`.

## Phase AB: a card passes its first review

### A. Measure the first review

- `rc quality` prints the first-review pass rate, runs a landed card and reruns by cause {c3}
  objective: PASS RATE on the dashboard is reviews accepted over reviews plus failed checks (apps/rc-gui/src/metrics.rs:168), which hides whether a card passed first time, and no figure splits reruns into check, lint, review, merge and restart. Add `crates/ra-console/src/quality.rs` (new file) with a `Quality` struct computed from the feed lines of `var/pair.log` (the parse at apps/rc-gui/src/metrics.rs:108-121 shows the line shapes), and an `rc quality` subcommand that prints it. The walk's send-back line says "its check failed" for a failing test and for a lint warning alike (agent.rs:2797), and a landed card's handoffs are archived, so make `pair::sent_back` name the cause as one of check, lint, review, merge. Every later card of this phase is measured by a figure of this struct.
  acceptance: `cargo test -p ra-console -q quality_counts` passes, its tests feeding a scripted feed of three cards and asserting 1 of 3 passed the first review, 2.0 runs a landed card, and one rerun each under check, lint and review.
  files: crates/ra-console/src/quality.rs crates/ra-console/src/lib.rs crates/ra-console/src/pair.rs crates/ra-console/src/agent.rs apps/rc/src/main.rs

- Each review keeps its brief and its answer under `var/reviews` {c2}
  objective: a change to the review instruction can only be tried on a live walk today. `judge` (agent.rs:1528) writes the brief it sent and the answer it got to `var/reviews/<id>-<n>.md`, n counting the card's reviews, so stored reviews can be replayed. Nothing reads the files yet.
  acceptance: `cargo test -p ra-console -q keeps_the_review_brief` passes, its test running one scripted review and asserting the file holds the brief and the answer.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- `rc review replay` judges the stored briefs with a named model and prints agreement {c3}
  objective: with briefs stored by the card before, replay sends each to a model under the current instruction and parser and prints three counts against what the card did next: agreed, accepted what was later changed, rejected what later landed unchanged. This is the offline test for every reviewer change of section C and for a local model as reviewer. New file `crates/ra-console/src/replay.rs`.
  acceptance: `cargo test -p ra-console -q replay_agreement` passes, its test replaying two stored reviews through a scripted provider and asserting the three counts.
  files: crates/ra-console/src/replay.rs crates/ra-console/src/lib.rs apps/rc/src/main.rs

### B. Cards that can pass: one gate on every door

- `admit::faults` names what makes one card unable to pass {c3}
  objective: look-back cards #240, #241 and #245 named a command that does not exist, files that do not exist and a placeholder crate, and cost 40 turns and a 1200 s check for no landing. New file `crates/ra-roadmap/src/admit.rs` with `Workspace`, `Fault` and `faults(node, workspace)`: no command by `check::is_command`, a `<placeholder>` in a command, a `-p` crate that is no member, a `--workspace` test on a card of under three crates, a script path that is missing, a files token that is no path, a file whose parent directory is missing. The cargo rules apply only when the workspace lists crates. Pure, no I/O beyond path checks.
  acceptance: `cargo test -p ra-roadmap -q admit_faults` passes, one test a fault built from the text of #240, #241 and #245, and one asserting a new file under an existing directory is no fault.
  files: crates/ra-roadmap/src/admit.rs crates/ra-roadmap/src/lib.rs crates/ra-roadmap/src/check.rs

- `admit::repaired` fixes what needs no model {c2}
  objective: a crate after `-p` can be derived from the card's files with `check::crate_of`, a files token can lose its brackets and backticks, a `--workspace` can narrow to the crates of the files. `repaired(node, workspace)` returns the node with those applied when the result has fewer faults, so the common slips never reach a model or the card's owner.
  acceptance: `cargo test -p ra-roadmap -q admit_repairs` passes, asserting a card with files in one crate and a workspace-wide test comes back with that crate named, and a card with no command comes back unchanged.
  files: crates/ra-roadmap/src/admit.rs crates/ra-roadmap/src/tree/tests.rs

- `rc check`, `rc import` and `rc add` report the gate's faults {c2}
  objective: `tree::lint` and `definition_of_done` run only under `rc check` (apps/rc/src/main.rs:1445); import and add insert unchecked. All three call `admit::faults` after `repaired`: check prints them with the lint's, import and add insert the card blocked with one `gate:` line a fault at the head of its notes and print the count.
  acceptance: `cargo test -p rc -q gate_on_import` passes, its test importing an outline with one sound card and one naming a missing crate and asserting the second is blocked and its notes start with the gate's line.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs

- The planner and the look-back insert through the gate {c3}
  objective: `plan_to` (crates/ra-console/src/commands/plan.rs) inserts what the planner answered after `has_check`, which any backticked span satisfies. Replace `has_check` with the gate: a card with faults after `repaired` is inserted blocked with `gate:` lines and the feed says `#N held at the gate`, so the walk spends no turn on it.
  acceptance: `cargo test -p ra-console -q held_at_the_gate` passes, its test scripting a planner answer of two cards, one with acceptance naming no command, and asserting one defined and one blocked with the gate's line.
  files: crates/ra-console/src/commands/plan.rs crates/ra-console/src/plan/propose.rs crates/ra-console/src/plan/tests.rs

- The planner's brief lists the crates, their test files and the named checks {c2}
  objective: the planner writes in one turn with no tools and its brief (plan/prompt.rs `brief`) holds no path of the tree, so it invents commands and files. Add a block of under 60 lines built without a model: the workspace's crates with their test files, each path the cited evidence names with whether it exists, the project's named checks, and for a cargo project the two-line test-naming convention.
  acceptance: `cargo test -p ra-console -q brief_lists_the_crates` passes, asserting the brief for a workspace of two crates names both and marks a cited path that is missing.
  files: crates/ra-console/src/plan/prompt.rs crates/ra-console/src/commands/plan.rs crates/ra-console/src/plan/tests.rs

- A planned card that fails the gate gets one repair turn {c3}
  objective: a card the planner wrote with faults is held for its owner today even when the fault is a wrong crate name. Send the faulty cards back once with their faults and the workspace block, re-gate the answer, and hold only what still fails. One turn, no tools: a role for the local model.
  acceptance: `cargo test -p ra-console -q repair_turn` passes, its test scripting a first answer with an unknown crate and a second with the right one and asserting the card is defined and the provider was called twice.
  files: crates/ra-console/src/plan/propose.rs crates/ra-console/src/commands/plan.rs crates/ra-console/src/plan/tests.rs

- A filtered test command that runs no test fails the check {c2}
  objective: `cargo test -q <filter>` exits 0 when the filter matches nothing, and nothing reads `running 0 tests`, so a card whose worker never wrote the named test passes its check (#240 passed `cargo test -p rc -q` having added no test). After a cargo test check with a filter, sum the `running N tests` lines; zero is a failed check whose first error is `the filter <words> matched no test`.
  acceptance: `cargo test -p ra-console -q matched_no_test` passes, asserting an output of three blocks that each ran 0 tests fails with that first error and an output with one test run passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

### C. One contract for the worker and the reviewer

- `Contract` splits a card's acceptance into checks and numbered criteria {c3}
  objective: the check runs the backticked commands and the reviewer judges the whole prose, so every clause no command proves becomes a review point (agent 14, asm 173, runelite 35 substantive rejections). New file `crates/ra-roadmap/src/contract.rs`: `Contract { checks, criteria, scope, person }`, `of(node)` splitting the acceptance at `;` and sentence ends outside backticks, a clause with a command proved by that check, a clause matching `HUMAN_CLAUSES` moved to `person`, the rest judged by reading; and `render(results)`.
  acceptance: `cargo test -p ra-roadmap -q contract_of` passes, asserting an acceptance of one command clause, one prose clause and one clause of the human kind gives one check, two criteria and one line for the owner, and that render numbers the criteria C1 and C2.
  files: crates/ra-roadmap/src/contract.rs crates/ra-roadmap/src/lib.rs crates/ra-roadmap/src/tree.rs

- `rc check` names each criterion no command proves, as a warning {c1}
  objective: the planner's rule is that an acceptance holds nothing a reviewer must judge in prose, and nothing shows the writer of a card where it breaks the rule. With the contract of the card before, add `Fault::Unproven` to `admit` for each criterion judged by reading; `rc check` prints it as a warning line and still counts the roadmap clean, and no door holds a card for it. Whether it becomes a hold is decided from the count of first-review rejections that cite such a criterion.
  acceptance: `cargo test -p rc -q unproven_criteria` passes, asserting a card with one command clause and one prose clause gives one warning that names criterion 2 and no fault that fails the check.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs crates/ra-roadmap/src/admit.rs

- The worker's brief and the review brief print the same contract {c3}
  objective: the worker reads "Acceptance criteria" as one line of prose (agent.rs:8525) and the reviewer reads it again as prose plus 6000 chars of card notes that hold old reports and pasted lint output. Both briefs call `Contract::render`, the reviewer's with each check's result; `review_brief` (agent.rs:1702) drops the notes section and keeps the handoff, the last verdict, the points that left and the files section of crates/ra-console/src/review.rs.
  acceptance: `cargo test -p ra-console -q same_contract` passes, asserting the contract block of the worker's prompt and of the review brief are equal but for the results, and the review brief holds no section of the card's notes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/review.rs crates/ra-console/src/agent/tests.rs

- The review instruction asks for a ruling a criterion {c2}
  objective: `REVIEW_INSTRUCTION` (agent.rs:1149) asks whether the diff meets the acceptance and whether the handoff tells the truth, which invites a point a sentence. Rewrite it around the contract: rule on each criterion; one proved by a passed check is met unless the diff shows the test does not test it; a work point is `N. [work] C<n> path:line: what`; `[nit]` for what rejects nothing; a handoff claim the diff contradicts is a nit unless it bears on a criterion; what is listed for the owner is not judged.
  acceptance: `cargo test -p ra-console -q instruction_names_the_criteria` passes, asserting the instruction holds the point form with a criterion number, names the kind nit, and no longer asks whether the handoff tells the truth.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- An answer in no form is asked again once and is never points {c3}
  objective: `verdict_of` (agent.rs:1166) turns every line of an off-form answer into a point; the feed has verdicts of 19 and 28 points from answers of up to 15,034 tokens (var/pair.log:850, :958). Return `Malformed` for an answer whose first line is neither word; `judge` asks once more in the same session with the form alone; malformed twice is no verdict and the card stays in review, as when a reviewer runs out of turns.
  acceptance: `cargo test -p ra-console -q malformed_verdict` passes, asserting a prose answer followed by a formed one gives the formed verdict with two provider calls, and two prose answers leave the card in review with no point on it.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- A work point cites a criterion or a changed file, and five stand at most {c3}
  objective: a point needs no file and no tie to the acceptance today, so a worker is sent back on points it cannot act on. Parse each point into `Point { number, kind, criterion, file, line, what }`; a work point with neither a criterion of the contract nor a file of the diff becomes a note kept on the card; past five work points the rest are counted in the feed and dropped; a rejection left with no work, card or environment point is accepted with notes.
  acceptance: `cargo test -p ra-console -q points_must_cite` passes, asserting a verdict of one cited and one uncited point rejects on one, a verdict of only uncited points accepts with two notes, and seven cited points leave five.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- The reviewer's `[nit]` replaces the keyword weight {c2}
  objective: `classify` (crates/ra-console/src/commands/review.rs) calls a rejection trivial by word lists whose substantive side holds `test`, `fail` and `return`; all 27 rejections of the agent's feed are substantive and the trivial path has not fired. Add `PointKind::Nit`: nits never reject and go to the fix pass when one is on, else to the notes. Delete `classify`, `TRIVIAL` and `SUBSTANTIVE` and their callers in `judged`.
  acceptance: `cargo test -p ra-console -q nits_never_reject` passes, asserting a verdict of one nit is accepted with the nit on the notes and a verdict of one work point and one nit rejects on the work point alone.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/review.rs crates/ra-console/src/commands.rs crates/ra-console/src/agent/tests.rs

- A second review may add points only on lines changed since the first {c3}
  objective: #185 was rejected nine times running and asm #53's points went 5, 5, 6, 4: a reviewer finds fresh points in lines it already passed. Stands on #221, which shows the last verdict. Store the commit a verdict judged on the handoff; the next brief leads with the diff since it; a point numbered above the last verdict's highest must cite a line changed since or a criterion not ruled on, else it is a note.
  acceptance: `cargo test -p ra-console -q delta_review` passes, asserting a second verdict with one kept point, one new point on a changed line and one new point on an unchanged line rejects on two and notes one.
  files: crates/ra-console/src/agent.rs crates/ra-context/src/lib.rs crates/ra-console/src/agent/tests.rs

### D. Lint and a failed check settled inside the run

- Machine-applicable lint fixes are applied to the card's lines before the lint judges {c3}
  objective: one warning on a changed line sends a passed card back for a whole run (agent.rs:5611), and #219 to #223 and #226 were each sent back three times largely on lint. `ra_tools::suggestions_within(json, ranges)` reads clippy's JSON for suggestions marked machine-applicable inside the changed ranges and `apply` writes them, last span first; `lint_changed` (agent.rs:6127) applies them and lints once more. No model and no turn. Stands on #232 for which lines are the card's.
  acceptance: `cargo test -p ra-tools -q suggestions_within` passes, asserting two suggestions on one line of a fixture file are both applied and a suggestion outside the ranges is left.
  files: crates/ra-tools/src/lib.rs crates/ra-tools/src/tests.rs crates/ra-console/src/agent.rs

- A failed check or a leftover warning returns once to the session that reported {c3}
  objective: a first run that fails its check or leaves a lint warning is rerun in a fresh session at 60 to 110k tokens a turn (agent 49 and asm 507 reruns on checks). Before the state is set in `run_task`, when the failure is no setup failure, not red on main and no timeout, and the session's last turn was under 90k tokens, resume the session once with the failure alone, cap 8 turns, then check and lint again. It spends no `CHECK_RETRIES` and the feed says `#N continued: check, 2 turns`.
  acceptance: `cargo test -p ra-console -q continued_in_session` passes, its test scripting a run whose check fails then passes after one resumed turn and asserting the card is implemented, one run is counted and the feed holds the continued line.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- The worker is told once who runs the check {c1}
  objective: `SYSTEM` (agent.rs:8499) says to build at most twice and normally not at all; the context (crates/ra-context/src/lib.rs:224) says of the acceptance commands `Run them before reporting`. With the continuation of the card before, replace that sentence with: the orchestrator runs them when you report and returns a failure to you once, in this session.
  acceptance: `cargo test -p ra-context -q who_runs_the_check` passes, asserting the context says a failure returns to the session once and no longer says to run the commands before reporting.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs

- The worker's context carries the project's eight commonest lints {c2}
  objective: the retro logs repeat the same lessons run after run (`pub(crate)` in a binary crate, `writeln!` over `push_str(&format!)`, `#[must_use]`, an Errors section). Count lint names over the warnings of the project's last 50 handoffs and print the top eight with their counts in the worker's brief, so each project gets its own list from its own linter with nothing to maintain.
  acceptance: `cargo test -p ra-context -q lint_primer` passes, asserting handoffs with three unreachable_pub warnings and one single_match warning give a primer that leads with unreachable_pub and the count 3.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs crates/ra-console/src/agent.rs

### E. Loops that end, and a third strike that changes something

- Each handoff carries what still stands, and the context counts stalled rounds {c2}
  objective: every rule that ends a rerun loop asks whether a file changed (agent.rs:2808, agent.rs:4843), and a run sent back for lint or a failing test always changes one; #219 ran nine times and asm #80 went 109 rounds. Write `Standing { failing, points, warnings }` on the handoff's check line, and add `stalled_rounds()` beside `idle_rounds()` (crates/ra-context/src/lib.rs:1060): the rounds running in which `Standing` did not fall and the check's first error stayed the same. Nothing acts on the count yet.
  acceptance: `cargo test -p ra-context -q stalled_rounds` passes, asserting three handoffs with 2, 2 and 2 failing tests count two stalled rounds and handoffs with 3, 2 and 1 count none.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs crates/ra-console/src/agent.rs

- A card is rerun while it gets closer, goes a rung up at two stalled rounds and is held at four {c3}
  objective: with the count of the card before, replace `moved` (agent.rs:2808) and the idle test (agent.rs:4843) by progress. Two stalled rounds add one to the `rejections` the router escalates on (agent.rs:2901, router.rs:606), which today counts only reviewer send-backs, and the next run starts a fresh session; four hold the card with `no progress in 4 rounds` and what still fails. An absolute card runs under the same rule with no other cap. This reads the rule of 2026-11-12 (work until done unless a full stop) as counting four rounds without progress as a full stop, and the card's handoff asks its owner to confirm that reading.
  acceptance: `cargo test -p ra-console -q no_progress_holds` passes, asserting four scripted rounds that each change a file and leave the same two tests failing end with one escalation and the card held, and four rounds with one test fewer failing each time end with no hold.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/router.rs crates/ra-console/src/agent/tests.rs

- A follow-up card from a review point carries files and passes the gate {c2}
  objective: `split_points` (agent.rs:1261) makes a card of each `[card]` point with a title and an objective and neither acceptance nor files. Give it the file the point cites and the parent's files, pass it through `admit`, and send it to the planner's repair turn for its acceptance; what still fails is held at the gate like any card.
  acceptance: `cargo test -p ra-console -q follow_up_card` passes, asserting a card point citing a file makes a card with that file and, with a scripted repair answer, an acceptance that names a command.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- Before a stalled card is held, one cheap call says card or work {c3}
  objective: six cards were blocked on three send-backs and started again unchanged (var/pair.log:841-929); a block changes nothing about the card or the route. Build this only if `rc quality` still shows held cards after sections B to D. One turn, no tools, on the fix or local model: given the contract and the stalled rounds' check lines or verdicts it answers `card` or `work`; `card` holds the card with the diagnosis for the repair turn, `work` gives one run a rung up in a fresh session; the hold's line then says which.
  acceptance: `cargo test -p ra-console -q third_strike` passes, asserting a scripted answer of card blocks the card with the diagnosis in its notes and a scripted answer of work starts one run on the next rung.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/router.rs crates/ra-console/src/agent/tests.rs
