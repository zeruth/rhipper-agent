# Rhipper Agent

**One roadmap from the five reviews.** The heading above is the project's own title and nothing more, because `Store::import` gives the project the outline's title (store.rs:272-276). This file merges `orchestration.md`, `efficiency.md`, `testing.md`, `quality.md` and `observability.md` into the one roadmap the walk executes next. It imports whole with `rc import`: everything outside the outline is prose at the margin with no list marks, so the parser in `crates/ra-roadmap/src/outline.rs` skips it. The cards run in the order written; inside a section each card builds on the ones above it.

Where the summary or a card gives a line number, the claim was read in the code at commit 787be7f. Where it gives only a review's name, the claim is that review's and was not checked again. The effects are estimates from the reviews' evidence, not measurements; each milestone names the measure that will show whether they held.

#### Where the three projects stand

| Measure | agent | runelite | asm |
|---|---|---|---|
| Runs that were reruns | 69% | 85% | 97% |
| First reviews passed | 71% | 14% | 8% |
| Median tokens a turn | 55k | 55k | 55k |

#### The five biggest changes, in order

| Order | The change | What it is expected to do | Milestone |
|---|---|---|---|
| 1 | A card is admitted only when a machine can run its check; a failed check goes to whoever can fix it; a loop ends when a round changes nothing; an off-form review is asked again, not turned into points. | Rerun rate. asm's 97% is mostly two cards (#131, #132) that ran 270 and 234 times on a check cargo rejected on its arguments; a hold after four rounds without progress bounds each at four. Three look-back cards on the live roadmap (#240, #241, #245) spent 40 turns on acceptances no machine can run. Target: asm at or under 50%, runelite 60%, agent 50%. | No run is spent on a card that cannot pass |
| 2 | A build fits in one tool call: the hook stops dropping the timeout, a worktree leases a warm target, and the worker proves its tree once with `rc prove`, a proof the orchestrator reuses. | Turns a run. A build costs 2 to 5 turns today at about 55k tokens each, and a card's first build is 6 to 15 minutes cold. Target: no turn spent waiting on a build, first build at or under 120 s, one suite run fewer for every run. | A build fits in one call |
| 3 | The prompt carries a map: the ranges the card will change, ranked and partly quoted, read from the worktree, on a smaller fixed floor. | Tokens a turn and turns a run. 68 of 151 tool calls came before the first edit, and read-in is 36 to 45% of the input. Target: median turn 55k to 35k, a c3 first run from 47–65 turns to 25 or fewer. | Reading in takes four calls |
| 4 | One numbered contract from the card to the verdict: the worker and the reviewer read the same criteria, the reviewer reads what the check proved, and lint is settled inside the run. | First-review pass rate. runelite passed 10 of 72 reviews and asm 16 of 202; #219 took nine runs and about 5.4M tokens on lint alone. Target: agent 85%, runelite and asm 50%. | One contract from the card to the verdict |
| 5 | A tree is proven once, by its hash, and a card lands from a landing worktree by fast-forward. | Wall time a landed card and reruns after a landing. The suite runs 3 to 5 times for one clean pass, and eight landings waited 2043 s on one another. Target: two suite runs or fewer a landed card, 60 s or less from the run's end to landed. | A tree is proven once and lands from its own worktree |

The four milestones after these are what the first five unlock: a walk that survives its own death, a record the window reads causes from, a hermetic suite with the big files carved, and one walk with one planner.

The whole is nine milestones, 30 sections and 151 cards: 12 at c1, 69 at c2, 70 at c3, none above. The first two milestones are 39 cards and carry most of the effect on the rerun rate and the turns of a run; they are also the ones that help runelite and asm as much as the agent, since nothing in them is about the agent's own code. A walk takes ready cards by priority and then in the order of the tree (`ready_cards`, agent.rs:3106) and the import sets no waits, so the last section of this file lists the waits to set with `rc depend` before the first walk.

The carving of agent.rs comes eighth, not first, for a reason worth stating. Almost every card here names agent.rs, so the overlap hold-back of #237 runs the agent's project close to one card at a time until the file is carved; carving first would buy width. It would also move every function the first seven milestones cite, before the map of the third milestone makes a function's place cheap to find. The order here spends wall time to keep runs cheap. If the first two milestones show seats idle behind agent.rs for longer than their cards run, move "Files a card can read whole" up: its carves need nothing but `rc carve` and the ratchet above them. Each carve's acceptance also names one test of an earlier milestone, to show the moved code still runs; a carve moved ahead of that milestone needs that second test swapped for one that exists by then.

#### One design for each problem the reviews shared

| Problem | Taken | Set aside |
|---|---|---|
| A card nobody can run | quality's `admit` gate at every door into the roadmap | efficiency's check-that-cannot-run card, observability's hole check: both are faults of the same gate |
| A filter that matches no test | one card (quality and testing wrote the same one) | the duplicate |
| Whose failure a failed check is | testing's `classify` table in the new pure crate `ra-check`; the run's cause in the record carries its blame | a second cause type in the event log |
| A card that loops | quality's progress rule (`Standing`, stalled rounds read off the handoffs, so it holds across restarts) | orchestration's fixed budgets by cause; its one good part is kept: rounds blamed on the machine or on main do not count |
| A proof made twice | testing's ledger `var/checks.tsv`: one row a command, keyed by the tree's hash; it is the cache and the measure at once | orchestration's `var/proofs` store and efficiency's stamp file |
| The worker's own check | one verb, `rc prove`, with efficiency's joining and exit 124 | the name `rc try` |
| A cold target | orchestration's leased pool (a target outlives its card and stays warm); efficiency's warm-up build beside the first turns | efficiency's `seed_target` copy (its own review calls it unproven) and testing's rename-into-pool |
| Landing | orchestration's stage, prove, fast-forward in a landing worktree, then a queue and a batch | testing's train that keeps the stash; both c4 cards are split in two |
| Where the tree is formatted | at the checkpoint, before the proof (both reviews agree once stated) | fmt after the proof at landing |
| Lint | fixes rustc marks machine-applicable are applied with no model; the rest continue the warm session; lint runs beside a failed check | a fresh run for lint alone (today's `lint_fix`, agent.rs:1374, when the session is still warm) |
| Resuming a session | efficiency's `worth_resuming`: only inside the provider's cache life | resuming cold (one turn cost $1.52 against $0.10) |
| The waiting line of `rc hold` | a line a minute, and ranks in place of the build that runs without a slot after 15 minutes (agent.rs:2291) | observability's line that announced the unguarded build |
| The measures | the tally on the end line now; the typed event log and its board as the one record later | `rc quality` and the walk's own report as separate counters: both become views of the board |
| Splitting the big files | efficiency's `rc carve` moves items by machine; new code goes to new files from the first milestone on | hand moves of a thousand lines by a worker |

#### Decisions taken where a review asked the person

1. An absolute card is held after four rounds without progress like any other; `rc run <id>` takes it up again.
2. A criterion no command proves is a warning of `rc check`, not a hold.
3. A lint fix rustc marks machine-applicable is applied with no model, inside the lines the card changed, and the end line says so.
4. A landing never touches a checkout that is dirty in the card's files: it waits and says which file.
5. Live card #241 tells the worker to start builds in the background because of the 120 s limit. The second milestone removes that limit, so #241's text should change to "prove with `rc prove`" before it runs again.
6. The gate of the first milestone will hold live cards #244, #246 and #247 as they are written: their acceptances name `rc gate` and `rc check --dry-run <card>`, which do not exist. That is the gate working; they need a rewritten acceptance.

## No run is spent on a card that cannot pass
objective: Moves the rerun rate of all three projects and the count of runs that end with nothing a worker could fix. Target on each project's next walk: agent 50% or under (69% now), runelite 60% (85%), asm 50% (97%); no card dispatched whose acceptance a machine cannot run; no card sent back more than four rounds running without progress; the window's count of failed runs equals the feed's.
acceptance: Every child accepted, and the next walk of each project meets the targets above.

### The count of failed runs is true
- A run that ends sent back writes the ✗ end marker, and only a run that ended implemented writes ✓ {c1}
  objective: `run` writes the ✓ marker for every `Ok` outcome, whatever state the card ended in (agent.rs:1116-1124), so the log the window reads held 30 failed ends against the feed's 100 and the window showed 70 failed runs as passed (observability, finding 1). Every rate below is read off that marker. The marker follows the card's state after the run: ✓ for implemented or accepted, ✗ with the first line of the reason otherwise.
  acceptance: `cargo test -p ra-console -q end_marker` passes, with a scripted run whose check fails writing the ✗ marker and a run that ends implemented writing ✓.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- The run tally counts the first turn's tokens, the tool calls before the first edit and the turns that only waited on a build {c2}
  objective: The reading-in and the build waits are known only from one review's hand count of four cards (efficiency, D8: 68 of 151 calls before the first edit; 2 to 5 turns a build). `TurnTally` in the provider gains `first` (input tokens of turn one), `first_edit` (calls before the first Edit or Write), `waits` (turns whose only calls were a wait) and `written` (bytes written), so the targets of the next two milestones can be read from every run.
  acceptance: `cargo test -p ra-providers -q tally_counts` passes over a scripted stream of four turns with one wait and an edit on the third call.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs
- The end line of a run says the tally's four numbers {c1}
  objective: The feed's end line is where the person, the paired session and the look-back read a run (efficiency, D8). `tokens_line` (agent.rs:7267) adds `first 14k · first edit after 6 calls · 2 waits`, leaving the words the dashboard parses where they are.
  acceptance: `cargo test -p ra-console -q tokens_line_says` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- `rc usage --runs` prints the medians of a project's runs by complexity {c2}
  objective: A target such as "a c3 run in 25 turns" needs one command that prints it (efficiency, D8). The usage rows gain the tally's fields, and `rc usage --runs` prints, for each project and complexity, the runs, the median turns, tokens a turn, first-turn tokens, calls before the first edit and waits.
  acceptance: `cargo test -p ra-usage -q run_medians` passes over six scripted rows of two complexities.
  files: crates/ra-usage/src/lib.rs crates/ra-usage/src/query.rs crates/ra-usage/src/tests.rs apps/rc/src/main.rs

### A card is admitted only when a machine can run its check
- A new crate, `ra-check`, reads a test run's output: the tests run, the tests failed, the first compile error {c2}
  objective: What a check's output means is read today by substring lists inside agent.rs (6642-6644, 7159), a file of 8,600 lines every card about checks has to open (testing, D0; efficiency, F6). A small pure crate holds that knowledge from here on. `output.rs` (new) parses cargo and gradle output into `Ran { tests, failed: Vec<String>, compile_error: Option<String> }`, summed over every test binary of the run.
  acceptance: `cargo test -p ra-check -q output` passes over four saved outputs: a pass, a failure, a compile error, and a filtered run of nothing.
  files: Cargo.toml crates/ra-check/Cargo.toml crates/ra-check/src/lib.rs crates/ra-check/src/output.rs crates/ra-check/src/output/tests.rs
- A filtered test command that ran no test fails the check as "matched no test" {c2}
  objective: `cargo test -p x -q a_filter` exits 0 when the filter matches nothing, and nothing in the code counts the tests run, so a card whose test was never written, or was named differently, passes its check (quality, design 1; testing, F5: the same card in both). When a command carries a filter and `ra_check::output` counts no test, the check fails with "matched no test: <filter>" and the worker is told the name the card expects.
  acceptance: `cargo test -p ra-console -q matched_no_test` passes, with a filtered command that ran nothing failing the check and an unfiltered run of an empty crate passing.
  files: crates/ra-console/src/agent.rs crates/ra-console/Cargo.toml crates/ra-console/src/commands/tests.rs
- `admit::faults` names what makes a card's acceptance impossible to run {c3}
  objective: Every backticked span of an acceptance is run as the check, and nothing reads it first. On the live roadmap #241 asks for `rc handoff --dry-run <card>` (no such subcommand; its check fell back to the workspace suite and timed out after 1200 s), #245 for `cargo test -p <review-crate>` (cargo refused the name) and #240 backticks the words a command prints; none of the three landed, in 40 turns (quality, design 1, checked in rhipper.md). `admit.rs` (new) is a pure function over the card and a `Workspace` (its packages, the binaries' subcommands, its scripts): `faults` returns `NoCommand`, `Placeholder`, `UnknownCrate`, `UnknownVerb`, `NotACommand` and `MissingScript`, each with the span.
  acceptance: `cargo test -p ra-roadmap -q admit_faults` passes, with the acceptances of #240, #241 and #245 as fixtures and one clean card that has no fault.
  files: crates/ra-roadmap/src/admit.rs crates/ra-roadmap/src/admit/tests.rs crates/ra-roadmap/src/lib.rs
- `admit::repaired` mends what a machine can mend {c2}
  objective: Most faults have one right answer (quality, design 1): a span that is a printed word and not a command loses its backticks, `cargo test` with no `-p` takes the package of the card's files when they sit in one crate, and `-q` is added. `repaired` returns the mended acceptance and the faults that remain, so the gate holds a card only for what needs a person or the planner.
  acceptance: `cargo test -p ra-roadmap -q admit_repairs` passes.
  files: crates/ra-roadmap/src/admit.rs crates/ra-roadmap/src/admit/tests.rs
- `rc import`, `rc add` and `rc check` run the gate {c2}
  objective: Five doors lead into the roadmap and only `rc check` looks at a card (quality, design 1). The three the person uses read the workspace (`cargo metadata --no-deps`, the `rc` subcommands, the scripts on disk), mend what `repaired` mends and print each remaining fault with its card; `rc import` and `rc add` still write the card, as defined, with the fault in its notes.
  acceptance: `cargo test -p rc -q gate_on_import` passes, importing an outline with one placeholder acceptance and printing its fault.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs crates/ra-roadmap/src/check.rs
- The planner, the look-back and a review's follow-up cards pass the gate, and a faulted card is never dispatched {c3}
  objective: The cards that failed came through the doors nobody guards: the look-back wrote #240 to #247 and `split_points` writes follow-ups (quality, design 1). Each door calls the gate before it writes; the walk's ready list leaves out a card with a fault and says once "#N held at the gate: <fault>". The check of files that exist stays live card #216's and is called through the same gate when it lands.
  acceptance: `cargo test -p ra-console -q held_at_the_gate` passes, with a scripted look-back card naming an unknown crate that is written defined, not dispatched, and named on the feed once.
  files: crates/ra-console/src/plan/propose.rs crates/ra-console/src/lookback.rs crates/ra-console/src/agent.rs crates/ra-console/src/plan/tests.rs
- The planning brief lists the workspace's crates, the `rc` subcommands and the form of an acceptance {c2}
  objective: The planner and the look-back invent `rc gate`, `<review-crate>` and `crates/ (handoff/prompt builder)` because their brief never says what exists (quality, design 1; efficiency, F7). The brief gains a block of at most 30 lines: the package names, the subcommands of `rc`, the test-naming rule and one example of an acceptance the gate admits.
  acceptance: `cargo test -p ra-console -q brief_lists_the_crates` passes.
  files: crates/ra-console/src/plan/prompt.rs crates/ra-console/src/plan/tests.rs
- A plan with faulted cards gets one repair turn {c2}
  objective: A fault found at the gate costs a person's attention unless the model that wrote the card is asked while it still holds the evidence (quality, design 1). The planner and the look-back get one more turn with only the faults and the workspace block; cards still faulted after it are written held.
  acceptance: `cargo test -p ra-console -q repair_turn` passes, with a scripted planner that mends one of two faulted cards.
  files: crates/ra-console/src/plan/propose.rs crates/ra-console/src/lookback.rs crates/ra-console/src/plan/tests.rs

### A failed check goes to whoever can fix it
- `ra_check::classify` decides whose a failed check is, from one table {c3}
  objective: Since #235 a test that fails on main too parks the card, but the other causes still go to the worker: a check that timed out waiting for a slot, a command the build tool rejected, a test that passes when run alone, a failure in a crate the card did not touch (testing, F6 and D3). asm's 507 failed checks were almost all #131 and #132 on one rejected command. `cause.rs` (new) holds `Blame { Card, Main, Machine, Flaky, Setup, Outside }` and `classify(&Evidence)`, a pure table of seven rows over the exit, the parsed output, the base run, the rerun alone and the files changed.
  acceptance: `cargo test -p ra-check -q cause` passes with one test for each row of the table.
  files: crates/ra-check/src/cause.rs crates/ra-check/src/cause/tests.rs crates/ra-check/src/lib.rs
- The walk acts on the blame: only a failure blamed on the card goes back to its worker {c3}
  objective: A rerun the worker cannot win is the most expensive run there is (testing, D3). After a failed check the walk gathers the evidence it already has (`main_failures`, `rerun_alone`, `is_setup_failure`, `outside_failure`) and asks `classify`: Machine is retried once with no worker run, Setup holds the card for the person with the tool's own error line, Outside becomes a follow-up card through the gate, Main waits as #235 has it, Flaky passes with a note, and Card reruns as now. The send-back line names the blame.
  acceptance: `cargo test -p ra-console -q names_its_cause` passes, with a scripted check the tool rejects holding the card with no rerun and a scripted timeout retried once.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- A failed check that is not cargo or gradle is run whole on the base {c2}
  objective: The base run that tells main's failure from the card's exists only for cargo and gradle tests (testing, F6), so a failing `python tools/x.py` or `.\gw.ps1 :task` always goes to the worker. Such a command is run whole in the base worktree, once for each base commit; a failure there is main's.
  acceptance: `cargo test -p ra-console -q whole_command_on_base` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- A flaky test is remembered across walks and projects, and its third sighting makes a card {c2}
  objective: A test that fails and then passes alone is forgiven and forgotten, so the same test costs a rerun in every walk (testing, F6). `flaky.rs` (new) appends each sighting to `flaky.tsv` in the hub, keyed by project and test; a known flaky test never sends a card back, and the third sighting writes one card through the gate that names the test and its three outputs.
  acceptance: `cargo test -p ra-check -q flaky` passes; `cargo test -p ra-console -q third_sighting` passes.
  files: crates/ra-check/src/flaky.rs crates/ra-check/src/flaky/tests.rs crates/ra-check/src/lib.rs crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

### A loop ends when a round changes nothing
- A handoff records where the card stands, and `stalled_rounds` counts the rounds that moved nothing {c2}
  objective: The walk ends a loop only when a run changed no file (`idle_rounds`, ra-context lib.rs:1060), so a card that edits something every round never stops: asm #80 ran 109 rounds, #185 was rejected nine times, #223 took eight runs (quality, design 4; orchestration, row 5). A handoff gains `Standing { failing, points, warnings }`; a round made progress when the standing fell or the check's first error changed. `stalled_rounds` sits beside `idle_rounds` and reads the handoffs, so it holds across walks and restarts.
  acceptance: `cargo test -p ra-context -q stalled_rounds` passes, over handoffs whose standing falls, holds for two rounds, and falls again.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs
- After two stalled rounds a card goes one rung up; after four it is held, absolute or not {c3}
  objective: One rule replaces the idle count for sent-back cards and the unbounded loop of an absolute card (agent.rs:1100-1106, which ends only on three rounds that changed no file) (quality, design 4). Two stalled rounds route the next run to the next model of the router; four hold the card for the person with its standing on the feed. A round blamed on the machine, on main or on a merge is not a round (orchestration's budgets by cause, folded in).
  acceptance: `cargo test -p ra-console -q no_progress_holds` passes, with a scripted card whose failing count stays at two for four rounds held after the fourth, and one whose count falls each round still running.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- A held card says why in one line, and a cause in the card's own text becomes a follow-up card {c2}
  objective: "Held" with no reason sends the person to the diff, which is what they want to stop reading (quality, design 4). On a hold, one call to the review role gets the standing of the last four rounds and answers `work`, `card` or `environment` with one sentence; the sentence goes on the card as a `person:` line, and a `card` answer writes a follow-up through the gate as #238 does for a review's card points.
  acceptance: `cargo test -p ra-console -q third_strike` passes: a scripted diagnosis that blames the card's text writes the one line on the card and one follow-up card through the gate.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

### A verdict is a verdict
- A review answer that is off the form is asked again once and is never turned into points {c2}
  objective: When the first line is neither word, `verdict_of` makes every line of the answer a point (agent.rs:1196-1198), so a reviewer that wrote an essay produced verdicts of 19 and 28 points and the worker was sent to settle prose (quality, design 3). `verdict_of` returns `Malformed` for such an answer; `judge` asks once more with only the form, and a second off-form answer leaves the card implemented with "no verdict" for the next walk's reviewer, not sent back.
  acceptance: `cargo test -p ra-console -q malformed_verdict` passes, with an essay answer re-asked and a second essay leaving the card implemented.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/commands/tests.rs
- A point cites a path and line, a nit never rejects, and five work points are the most {c3}
  objective: Of 27 rejections, 27 were classed substantive: the trivial path of `classify` (commands/review.rs:66) never fired, and points such as "consider renaming" rode reruns beside real ones (quality, design 3). The instruction asks for `N. [work|card|environment|nit] path:line: what`; a point with no place is asked for once and dropped if it still has none; a list of only nits is an acceptance with the nits kept in the notes; past five work points the reviewer is asked for its five worst.
  acceptance: `cargo test -p ra-console -q points_must_cite` passes; `cargo test -p ra-console -q nits_never_reject` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/review.rs crates/ra-console/src/agent/tests.rs

## A build fits in one call
objective: Moves the turns a run spends on builds and the wall time of a card's first build. Today a build costs 2 to 5 turns, at least six cards lost a result past the tool's 120 s limit, and a first build in a new worktree takes 6 to 15 minutes (#214, #242, #243). Target: the tally's waits at 0 for nine runs in ten; first build of a card 120 s or less on a leased target; one suite run fewer for every run (the worker's proof reused); no scratch target left behind by a landed card.
acceptance: Every child accepted, and the run medians and the checks report of the next agent walk show the targets above.

### The shell's limit is the build's, not 120 seconds
- The hook's rewrite keeps the whole tool input and sets the call's timeout to ten minutes {c1}
  objective: The pre-tool hook answers a rewritten build with `"updatedInput": {"command": command}` (apps/rc/src/main.rs:998), which drops the `timeout` the worker asked for, so every build under `rc hold` runs at the tool's default of 120 s and goes to the background or is lost (efficiency, F2). The rewrite returns the call's whole `tool_input` with the command replaced and `timeout` set to 600000 when it is absent or lower.
  acceptance: `cargo test -p rc -q rewrite_keeps` passes, with a call carrying a description and a timeout of 300000 coming back with both and the higher timeout.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs
- A worker's process starts with the shell's default and maximum timeouts raised {c1}
  objective: A build the hook does not rewrite (a script, a gradle wrapper) still meets the 120 s default (efficiency, D2). The spawn of the worker sets `BASH_DEFAULT_TIMEOUT_MS` and `BASH_MAX_TIMEOUT_MS` to ten minutes; neither name appears in the code today.
  acceptance: `cargo test -p ra-providers -q shell_timeout` passes.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs
- `rc hold` prints the failures and the last lines of what it ran, and keeps the whole output under var/ {c2}
  objective: Workers pipe builds through `tail` and `grep` by hand to keep a turn small, lose the failure and run the build again (efficiency, D2: #215 ran test, check, test). `rc hold` keeps the whole output in `var/jobs/<pid>.log` and prints the parsed failures (`ra_check::output`) and the last 40 lines; a pass prints one line. Live card #243 allows the pipe; this removes the need for it.
  acceptance: `cargo test -p ra-console -q hold_prints` passes, with a scripted failing command printing its failing test and the log's path.
  files: crates/ra-console/src/agent.rs crates/ra-console/Cargo.toml crates/ra-console/src/agent/tests.rs
- `rc hold` says where it stands every minute {c1}
  objective: A waiting `rc hold` prints one line and then nothing (agent.rs:2295-2302), so a worker that waited 12 minutes on #213 could not tell a queue from a hang (orchestration, D9; observability's hold line: the same card). Every minute it prints the time waited and the builds running, as `rc hold: 3:00 waiting · 4 of 4 building`.
  acceptance: `cargo test -p ra-console -q wait_line` passes with a scripted clock.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- Build slots have ranks, and no build runs without a slot {c3}
  objective: After 15 minutes `rc hold` runs its command with no slot (agent.rs:2291-2294), which is the moment the machine is most loaded, and a landing's proof queues behind worker builds (orchestration, D9). A waiter writes `build-want-<project>-<rank>-<pid>` in the hub; a taker stands back while a live waiter of a better rank exists (a landing's proof, then a worker's build, then a check), a dead waiter's file is ignored, and the wait has no slotless end.
  acceptance: `cargo test -p ra-console -q slot_rank` passes, with a landing taking the freed slot before a waiting worker, a dead waiter ignored, and a long wait never running without a slot.
  files: crates/ra-console/src/hub.rs crates/ra-console/src/hub/tests.rs crates/ra-console/src/agent.rs

### A worktree builds warm
- The cleanup asks the provider where a worktree builds, and a walk's start removes the targets of cards that have no worktree {c2}
  objective: `clean_after` removes `<root>/target/wt-<id>` (agent.rs:4374), but with a scratch drive the target is `<scratch>/target/<project>/wt-<id>` (`shared_target`, claude_code.rs:1042), so on this machine every landed card leaves gigabytes behind and nothing says so (orchestration, D10; testing, D8: the same bug). The cleanup and `hub::build_target` call the provider's function; the walk's start lists `wt-*` targets whose card has no worktree, removes them and says how many bytes.
  acceptance: `cargo test -p ra-console -q target_of_the_worktree` passes with a scratch root set in the test.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/hub.rs crates/ra-console/src/agent/tests.rs
- A worktree's target can be a lease from the project's pool {c3}
  objective: Every card compiles the whole workspace from nothing in `wt-<id>`, 6 to 15 minutes before its first test (orchestration, D10; efficiency, F2; testing, F8). A pool of targets `wt-pool-<n>` outlives the cards: `lease_target` takes a free one or makes one, marking it with a `.lease` file made with `create_new` that holds the card's id; a lease whose card has no worktree is free, so a crash frees itself; `shared_target` answers the lease when there is one. Cargo rebuilds only what the card's files changed.
  acceptance: `cargo test -p ra-providers -q lease` passes: two cards never get one target, a released target is taken next, and a lease left by a card with no worktree is reused.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs
- The walk leases a target before it checks a card out and returns it when the card lands or is dropped {c2}
  objective: The lease has to exist before `git worktree add` writes the files, or their fresh mtimes make cargo rebuild everything (orchestration, D10). `worktree_add` leases first; `clean_after` releases instead of deleting; the pool is capped at the walk's width plus one, and a target past the cap is removed.
  acceptance: `cargo test -p ra-console -q leased_target` passes, with a second card taking the target the first one returned.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- The preflight builds nothing: a check's package is looked up, not compiled {c2}
  objective: Before a run the preflight dry-runs each check, and for a test command that means `--no-run` (agent.rs:6456-6469): a whole compile, in series, before the worker's first turn, to learn only whether the package exists (testing, D8). The preflight asks `admit::faults` with the workspace instead and runs `--help` for other tools.
  acceptance: `cargo test -p ra-console -q preflight_builds_nothing` passes, with a scripted toolbox that records no cargo test call.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- A run's build starts when the run does, beside its first turns {c2}
  objective: The worker reads for several turns and then waits for a build that could have run meanwhile (efficiency, D2). When a run starts, the orchestrator starts the card's first check with `--no-run` in the card's target on a build slot, not awaited; the worker's first build finds the dependencies compiled. With a leased target this is seconds.
  acceptance: `cargo test -p ra-console -q warm_at_start` passes, with the warm-up recorded before the worker's first turn and the run not waiting for it.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

### A tree is checked once, by the worker, in one call
- `ra-check` keeps a ledger of every check: one row a command, keyed by the hash of the tree {c3}
  objective: Nobody knows how often or how long the suite runs: one review counted 3 to 5 runs for a clean pass and had no shell to time them (testing, D0). `ledger.rs` (new) appends a `Row` (when, card, stage, tree, base, command, toolchain stamp, outcome, seconds, tests, blame) to `var/checks.tsv`; `tree_of` hashes the working tree through a throwaway index (`git write-tree`), leaving the real index alone; `proven(tree, command, stamp)` answers the latest pass. The one store serves as the measure and as the cache; orchestration's `var/proofs` and efficiency's stamp file are not built.
  acceptance: `cargo test -p ra-check -q ledger` passes: a row written and found again, a changed file changing the tree, and a different toolchain stamp not proven.
  files: crates/ra-check/src/ledger.rs crates/ra-check/src/ledger/tests.rs crates/ra-check/src/lib.rs
- Every check the orchestrator runs writes its row {c2}
  objective: The ledger is only true if every stage writes to it (testing, D0). The run's check, the lint, the catch-up's proof, the landing's proof and the base run each write a row with their stage and seconds, and the feed gets one `check` line for each.
  acceptance: `cargo test -p ra-console -q check_rows` passes, with a scripted card that lands leaving rows for its run, its catch-up and its landing.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- `rc checks` prints the suite runs and the seconds a landed card, by stage and by blame {c2}
  objective: The target of this milestone and of the landing milestone is read here (testing, D0): suite runs for each landed card, seconds by stage (median and the 90th percentile), failures by blame, and the proofs reused.
  acceptance: `cargo test -p ra-check -q report` passes; `cargo test -p rc -q rc_checks_prints` passes.
  files: crates/ra-check/src/report.rs crates/ra-check/src/report/tests.rs crates/ra-check/src/lib.rs apps/rc/src/main.rs apps/rc/src/tests.rs
- `rc prove` runs the card's own checks in its worktree in one blocking call {c3}
  objective: A worker chooses a filter, a pipe and a timeout by hand and runs two or three builds a session; it cannot tell main's failure or a flaky test from its own (efficiency, D2 as `rc try`; testing, D1 as `rc prove`: one verb). `rc prove`, run in a worktree, runs the card's acceptance commands under one slot, prints ✓ or ✗ for each with the parsed failures of the first that failed and its blame, writes the ledger rows, and exits with the result.
  acceptance: `cargo test -p rc -q rc_prove` passes with a scripted card of two commands, the second failing.
  files: apps/rc/src/main.rs apps/rc/src/prove.rs apps/rc/src/tests.rs
- `rc prove` run again joins the check that is still running {c2}
  objective: A proof longer than the call would be lost as builds are lost today (efficiency, D2). The proof runs as a job with its output in `var/jobs/`; nine minutes in, `rc prove` exits 124 with "still running: run rc prove again to join", and a second call on the same tree joins the job instead of starting another build.
  acceptance: `cargo test -p rc -q prove_joins` passes with a scripted slow command.
  files: apps/rc/src/prove.rs apps/rc/src/tests.rs
- The run's check takes a passed proof of the same tree as its answer {c2}
  objective: A run that proved itself is compiled a second time by the orchestrator, on a slot other workers queue for (efficiency, D2; testing, D1). The check asks `proven` for each command on the tree's hash: proven, its line says "proved by the worker's own run" and nothing is built; anything else runs as now.
  acceptance: `cargo test -p ra-console -q reuses_the_workers_proof` passes, with a scripted toolbox that records no second run.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- The worker is told once who runs the check: itself, with `rc prove`, before it reports {c1}
  objective: The context tells the worker "Run them before reporting" (ra-context lib.rs:224) while the orchestrator runs the same commands after every run, and workers answer both ways: some run the suite twice, some report "I did not build" (quality, design 2; visible in the notes of #213, #214, #238). One sentence replaces it: prove with `rc prove` before reporting; a pass is not run again.
  acceptance: `cargo test -p ra-context -q who_runs_the_check` passes.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs rhipper.context.md
- The end line times a card's first build and its wait for a slot {c2}
  objective: The cost of a first build is known only from the workers' own look-backs (orchestration, D9). The first slot a card's run holds is timed, with its wait, and the end line says `first build 84 s after 12 s`; the next walk judges the pool and the ranks by it.
  acceptance: `cargo test -p ra-console -q first_build` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/hub.rs crates/ra-console/src/agent/tests.rs

## Reading in takes four calls
objective: Moves the tokens of a turn and the turns of a run. Today the median turn is 55k tokens, 68 of 151 tool calls come before the first edit, a c3 first run in agent.rs takes 47 to 65 turns and reads 0.8 to 1.5M tokens before it edits, and a rerun there starts at 45 to 50k. Target, read from `rc usage --runs`: first turn 14k or less (18k on a rerun); first edit after 4 calls or fewer on c1 and c2 cards and 8 on c3; a c3 run in 25 turns or fewer at 40k a turn or less; the map's hit rate 0.8 or more; median turn 35k.
acceptance: Every child accepted, and the next agent walk meets the targets above.

### A smaller floor under every turn
- A worker gets the tools its card can use {c2}
  objective: Every fresh run reads about 9.5k tokens of tool schemas before its prompt, and `WORKER_TOOLS` (claude_code.rs:32) names eleven tools for every card where the 151 calls of seven streams used six (efficiency, F3). `tools_for` gives one shell when the project's `[worker] shell` names it, the web tools only to an investigation or under `[worker] web = true`, `NotebookEdit` only to a card with an `.ipynb` file, and no `TodoWrite`.
  acceptance: `cargo test -p ra-providers -q tools_for` passes: a plain card's list has no web, notebook or todo tool, an investigation has the web tools, and a project whose shell is bash has no PowerShell.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs crates/ra-console/src/router.rs
- The system prompt puts what a walk's runs share first {c2}
  objective: `harness_messages` (agent.rs:5911) places the notes chosen for the card before the prompts that are the same for every run, so each run writes its whole 7 to 12k system prompt to the cache (efficiency, D3). The order becomes: the system text, the commit and retro prompts, the shell shapes, the other projects' digests, the project's context; what is the card's opens the task prompt. Two runs of one walk then share the prefix and the second reads it from the cache.
  acceptance: `cargo test -p ra-console -q system_prompt_shares` passes: the system prompts of two cards of different crates in one project are equal up to the project's context heading.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- A line learned on another project is shown only where it names something this project has {c2}
  objective: The system prompt of #245's run carried runelite's `third-party/jdk8u` line twice and two of asm's, none of use to an agent card (efficiency, D3). `machine_notes` (agent.rs:7226) keeps another project's line when a path it names exists here or a command it names is among this project's checks, and drops a line whose words another kept line already holds.
  acceptance: `cargo test -p ra-console -q machine_notes_keep` passes: a line naming a path absent here is dropped, one naming this project's build command is kept, and two lines differing by one word print once.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- The brief skips an ancestor with no objective and cuts the others to a sentence {c1}
  objective: Every prompt of every card carries "Under #1 Rhipper Agent: no objective" and a 100-token paragraph for its phase (efficiency, D3). An ancestor with no objective prints nothing; one with an objective prints its first sentence.
  acceptance: `cargo test -p ra-console -q brief_of_ancestors` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

### The prompt carries a map of the card's work
- Outline rows carry a start and an end, and the outline is read from the tree the worker acts in {c2}
  objective: A row gives a start line only, so one function costs a 400-line read; and `prepare` builds the outline from the project's root (agent.rs:5867 and :6219) where the regions seven lines on are read from the worktree (agent.rs:5874), so a rerun gets line numbers that have moved (efficiency, F1). Rows print `start-end`, an item's range opens on its doc comment and attributes, and `prepare` passes the worker's directory.
  acceptance: `cargo test -p ra-context -q outline_rows` passes; `cargo test -p ra-console -q outline_of_the_worktree` passes, with a worktree whose file differs from the root's outlined as the worktree has it.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- `map_of` ranks a file's items against the card's words {c3}
  objective: `outline_marked` shows the first 60 items in file order (ra-context lib.rs:349, `take(OUTLINE_ROOM)`): for agent.rs that is lines 28 to 646 of 8,600, none of them a card's subject, and c3 cards there read 0.8 to 1.5M tokens before their first edit (efficiency, F1 and D1). `map.rs` (new) holds `map_of(read, files, ask, changed, room)`, returning `Pointer { file, start, end, item, why, score }`: 100 for an item the card names, 80 for one the last run changed or a point names, 70 for a test holding the acceptance's filter, up to 40 for a rare word of the card in the item's name, 30 for a caller of a named item; the best that fit `room` tokens, in file order, then one line a file for what is not shown.
  acceptance: `cargo test -p ra-context -q map_ranks` passes five tests: a named item outranks an early one, a changed item is kept, a common word scores under a rare one, the room holds across two files, and a file with nothing pointed prints its one line.
  files: crates/ra-context/src/map.rs crates/ra-context/src/map/tests.rs crates/ra-context/src/lib.rs
- A worker's prompt carries the map in place of the outline of each file {c2}
  objective: The ranked map replaces `outline_files` in `prepare`, with the card's title, objective and acceptance as the ask and a room of 1,500 tokens; `OUTLINE_ROOM` goes (efficiency, D1). `var/context/<id>.md` shows the map as the worker got it.
  acceptance: `cargo test -p ra-console -q prompt_carries_the_map` passes: a card naming a function at the end of a file of 100 items has that function's row in its prompt and no row of the file's first items.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-context/src/lib.rs
- A name the card uses that its files do not define is found once across the project {c3}
  objective: #219's worker made about eight searches because the handler was in `apps/rc/src/main.rs` and the card listed `crates/` only (efficiency, D1). For each identifier of the card that no item of its files matches, `map_of` searches the project's tracked sources for an item of that name and adds it as a pointer; a name found nowhere is returned in `Map::unresolved`, and the prompt says "nothing defines: <name>; treat it as new".
  acceptance: `cargo test -p ra-context -q map_finds_outside` passes: an item the card names, defined outside its files, is pointed at with its path, and a name defined nowhere is listed unresolved.
  files: crates/ra-context/src/map.rs crates/ra-context/src/map/tests.rs
- The items a card names are quoted in its first prompt {c2}
  objective: A first run spends its first turns fetching what the card already names: #241 made nine of eleven calls before its two edits (efficiency, D1). `render(map, read, quote)` prints under the map's rows the numbered text of the named pointers, the highest score first, inside 6,000 tokens; an item that does not fit keeps its row.
  acceptance: `cargo test -p ra-context -q render_quotes` passes: a named 20-line function has its numbered body under the rows, one longer than the room has the row and no body, and of two that do not fit together the higher-scored is quoted.
  files: crates/ra-context/src/map.rs crates/ra-context/src/map/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- A rerun is handed the items its points name, not the first 800 lines the last run read {c3}
  objective: Reruns of c3 cards in agent.rs ran at 47 to 49k tokens from their first turn (#236, #235, #238: $0.34 to $0.43) where reruns with no read set run at 18 to 21k ($0.10 to $0.12): `regions` quotes up to 800 lines in the order the last run read them (`REGIONS_ROOM`, ra-context lib.rs:837) (efficiency, F9). On a rerun the map's ask carries the failures, the warnings and the review's points; the quote holds the items they name, then the items the last run changed, inside the same 6,000 tokens. `regions` and its two constants go.
  acceptance: `cargo test -p ra-console -q rerun_quotes` passes: a rerun whose failure names one of five functions the last run read quotes that one and lists the other four as rows, in under a third of the length the regions gave the same fixture.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-context/src/lib.rs crates/ra-context/src/map.rs
- The prompt says where the card's test goes and quotes the nearest one {c3}
  objective: An acceptance is a filtered test, and the worker pages through `commands/tests.rs` (5,000 lines) to learn the fixtures (efficiency, D1). From the acceptance the prompt gains: whether a test matching the filter exists and where; the `tests.rs` beside the card's first file with the name and line of its last test; and the existing test that best matches the card's words, quoted when under 40 lines.
  acceptance: `cargo test -p ra-context -q test_recipe` passes: a filter no test matches gives "no test matches yet" with the tests file and its last test, a filter that matches gives the test's range, and the quoted test is the one sharing the card's words.
  files: crates/ra-context/src/map.rs crates/ra-context/src/map/tests.rs crates/ra-console/src/agent.rs
- The task prompt ends on the first move {c1}
  objective: Turn one of a fresh run is usually one read of one page (efficiency, D1). The prompt's last line is built from the map: the ranges to read in one message (the three best pointers not quoted), "write the test first", and `rc prove` as the one check.
  acceptance: `cargo test -p ra-console -q prompt_ends_on` passes: the last line names the best pointers as path and range and the prove command, and a card with an empty map ends on the prove command alone.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- The end line says how much of the change the map pointed at {c2}
  objective: The map's scoring needs a number no model has to read (efficiency, D8). At a run's end the changed ranges are set against the pointers the prompt carried: `map 4/5` is the changed items a pointer overlapped over the changed items, and the usage row keeps both counts.
  acceptance: `cargo test -p ra-console -q map_hit` passes: a run that changed two pointed items and one unpointed prints the hit as 2 of 3, and a run with no map prints nothing.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-usage/src/lib.rs
- `rc map --eval` replays landed cards and prints the hit rate {c3}
  objective: The weights should be tuned on history, on all three projects, before workers pay for a bad guess (efficiency, D1). A landing writes the card's base and landing commits to the usage row; for each card with the pair the map is built from the card's text and the files at the base, and set against the ranges the landing changed. The command prints a row a card and the mean; `rc map <id>` prints one card's map as its prompt would carry it.
  acceptance: `cargo test -p ra-console -q map_eval` passes: in a fixture repository with two landed cards, one whose changed function the card names and one not, the rows are 1 of 1 and 0 of 1 and the mean 0.5.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-usage/src/lib.rs apps/rc/src/main.rs

### A run that wanders or went cold costs one line, not a session
- A run ten calls in with no edit is shown its first move again {c2}
  objective: Between a good run and the 160k cap there is no signal: #242 made 20 calls and #243 made 19 before their first edit (efficiency, F5). The hook keeps a count for each session in `var/runs/calls-<session>.json`; on the tenth tool call of a session with no Edit or Write the post hook adds "ten calls without an edit" and the map's first-move line, once.
  acceptance: `cargo test -p rc -q nudge_at_ten` passes: the answer on the tenth call of a session with no edit carries the line, the eleventh does not, and a session that edited at its third call never gets it.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs
- Every run that stops hands on what it read {c1}
  objective: The read set reaches the handoff only from a capped run or a failed check; a run stopped by a limit, a kill or a blocked card leaves the next one to read it all again (efficiency, D5). Every handoff of a run that did not pass carries the read set.
  acceptance: `cargo test -p ra-console -q handoff_carries_reads` passes: a run that ends blocked has its read ranges in the handoff.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- Every resume asks the age and the size of the session first {c2}
  objective: A one-turn run on a resumed session cost $1.52 (#216) and $1.87 (#208) against $0.10 to $0.12 for three fresh turns: past the provider's cache life a resume writes the whole transcript again (efficiency, F4). `worth_resuming(held, now, fresh_floor)` is true inside `CACHE_LIFE` (five minutes) of the session's last turn, or when that turn was under twice a fresh start; the report ask, an absolute card's next run and a continued run all go through it, and a run that does not resume starts on the rerun block as #210's reruns do (`rerun_resume`, agent.rs:8250).
  acceptance: `cargo test -p ra-console -q worth_resuming` passes: a 90k session two minutes old resumes, the same session an hour old does not, a 20k session an hour old does.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- A missing report is written by the `report` role from the diff and the run's last words {c3}
  objective: "Report asked twice" (#196, #209, #215, #220, #233) resumes the session for up to three turns at its full context (efficiency, D4). A `report` role (the local model when its server answers, else the cheapest) gets the diff stat, the changed items, the run's last three texts and the check's output, and writes the report in the handoff's shape; the session is asked again only when the role is off.
  acceptance: `cargo test -p ra-console -q report_role` passes: a run that ends without a report gets one from the scripted role with no second worker call.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-providers/src/route.rs crates/ra-console/src/router.rs

## One contract from the card to the verdict
objective: Moves the first-review pass rate and the runs spent on lint. Today 71% of first reviews pass on the agent, 14% on runelite (10 of 72) and 8% on asm (16 of 202); #219 took nine runs, 142 turns and about 5.4M tokens on lint; #185 was rejected nine times running. Target on each project's next walk: agent 85%, runelite 50%, asm 50%; no card sent back for lint alone; no second review that adds a point on a line it already passed; a replayed review agrees with itself nine times in ten.
acceptance: Every child accepted, and the next walk of each project meets the targets above.

### The worker and the reviewer read the same criteria
- `contract_of` splits an acceptance into its checks, its numbered criteria and what waits for the owner {c2}
  objective: Four parties hold four ideas of done: the card's prose, the worker's reading of it, the check's commands and the reviewer's judgement (quality, design 3). `contract.rs` (new) parses an acceptance once into `Contract { checks, criteria, owner }`: a command clause is a check and the criterion it proves, a prose clause is a criterion judged by reading, and a clause only a person or a device can settle is listed for the owner. `render` numbers the criteria C1, C2 and so on.
  acceptance: `cargo test -p ra-roadmap -q contract_of` passes: an acceptance of one command clause, one prose clause and one clause for a person gives one check, two criteria and one owner line.
  files: crates/ra-roadmap/src/contract.rs crates/ra-roadmap/src/contract/tests.rs crates/ra-roadmap/src/lib.rs
- `rc check` names each criterion no command proves, as a warning {c1}
  objective: The planner's rule is that an acceptance holds nothing a reviewer must judge in prose, and nothing shows a card's writer where it breaks the rule (quality, design 3). `admit` gains `Unproven` for each criterion judged by reading; `rc check` prints it as a warning and still counts the roadmap clean, and no door holds a card for it.
  acceptance: `cargo test -p rc -q unproven_criteria` passes: a card with one command clause and one prose clause gives one warning naming criterion 2 and no fault.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs crates/ra-roadmap/src/admit.rs
- The worker's brief and the review brief print the same contract {c3}
  objective: The worker reads the acceptance as one line of prose; the reviewer reads it again as prose with 6,000 characters of the card's notes, which hold old reports and pasted lint output (quality, design 3; the notes of live #213 and #215 show what that is). Both briefs call `Contract::render`, the reviewer's with each check's result; `review_brief` (agent.rs:1702) drops the notes and keeps the handoff, the last verdict, the points that left the card and the files section. Live card #213 owns the diff the review reads; this card leaves it alone.
  acceptance: `cargo test -p ra-console -q same_contract` passes: the contract block of the worker's prompt and of the review brief are equal but for the results, and the review brief holds no section of the card's notes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/review.rs crates/ra-console/src/agent/tests.rs
- The review instruction asks for a ruling on each criterion {c2}
  objective: `REVIEW_INSTRUCTION` (agent.rs:1149) asks "whether the diff meets the acceptance and whether the handoff tells the truth about it", which invites a point a sentence (quality, design 3). It is rewritten around the contract: rule on each criterion; one proved by a passed check is met unless the diff shows the test does not test it; a work point is `N. [work] C<n> path:line: what`; a handoff claim the diff contradicts is a nit unless it bears on a criterion; what is listed for the owner is not judged.
  acceptance: `cargo test -p ra-console -q instruction_names_the_criteria` passes: the instruction holds the point form with a criterion number and no longer asks whether the handoff tells the truth.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- A point may cite a criterion of the contract, and a point on a file outside the diff is a note {c2}
  objective: The first milestone made a point cite a path and a line; with the contract a point can be tied to the acceptance itself (quality, design 3). Each point parses into `Point { number, kind, criterion, file, line, what }`; a work point stands when it names a criterion of the contract or a file the diff changed, and one that names neither becomes a note on the card; a rejection left with no standing point is an acceptance with notes.
  acceptance: `cargo test -p ra-console -q points_cite_criteria` passes: a point citing criterion 2 with no line stands, a point on a file the diff did not change becomes a note, and a rejection of only such points accepts with the notes kept.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

### The reviewer reads what the check proved
- `ra-check` names the tests a diff added and whether the check ran them {c2}
  objective: A reviewer asks for "a run of the test" or doubts a test exists because the brief shows that a command passed, not what it ran (testing, D9). `new_tests(diff)` lists the test functions a diff adds; set against the names the run printed, it answers which new tests the check ran.
  acceptance: `cargo test -p ra-check -q new_tests` passes over a diff adding two tests and an output that ran one of them.
  files: crates/ra-check/src/output.rs crates/ra-check/src/output/tests.rs
- The review brief carries one proof line for each command {c2}
  objective: With the ledger's row and `new_tests`, the brief says for each command of the contract: the tree it ran on, the tests run, the new tests of the diff among them, the seconds (testing, D9). A filter that ran only tests older than the card says so, which is the case the reviewer most needs to see.
  acceptance: `cargo test -p ra-console -q proof_lines` passes: a scripted card whose check ran two tests, one of them new, has a brief saying 2 tests and 1 new.
  files: crates/ra-console/src/review.rs crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- A second review may add points only on lines changed since the first {c3}
  objective: #185 was rejected nine times running and asm #53's points went 5, 5, 6, 4: a reviewer finds fresh points in lines it already passed (quality, design 3). The handoff stores the commit a verdict judged; the next brief leads with the diff since it; a point numbered above the last verdict's highest must cite a line changed since or a criterion not yet ruled on, else it is a note.
  acceptance: `cargo test -p ra-console -q delta_review` passes: a second verdict with one kept point, one new point on a changed line and one new point on an unchanged line rejects on two and notes one.
  files: crates/ra-console/src/agent.rs crates/ra-context/src/lib.rs crates/ra-console/src/agent/tests.rs
- Every review keeps its brief and its answer, and `rc review replay` asks again and prints the agreement {c2}
  objective: A change to the review instruction is judged today by the next walk's pass rate, days later and mixed with everything else (quality, design 0). Each review writes `var/reviews/<id>-<n>.md` (the brief and the answer); `rc review replay` sends the kept briefs to the review role again and prints how many verdicts agree, so an instruction is tested on fifty past reviews before a walk pays for it.
  acceptance: `cargo test -p ra-console -q replay_agreement` passes: three kept reviews replayed through a scripted reviewer that changes one verdict print 2 of 3.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/review.rs crates/ra-console/src/agent/tests.rs apps/rc/src/main.rs

### Lint and a failed check are settled inside the run
- The lint runs beside a failed check, so a rerun gets both lists at once {c2}
  objective: The lint runs only after the checks pass, so a card fixes its test in one rerun and meets its lint warnings in the next (testing, F7; quality, design 2). The lint of the changed lines runs when the check fails too, and the send-back carries both.
  acceptance: `cargo test -p ra-console -q lint_beside_a_failed_check` passes: a scripted run with a failing test and one warning is sent back once, with both in the rerun's prompt.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- `ra-check` applies the lint fixes the compiler marks machine-applicable inside the card's lines {c3}
  objective: One warning on a changed line costs a model pass: `lint_fix` (agent.rs:1374) starts a whole run on the fix model for it, and #219 to #223 and #226 were each sent back three times largely on lint (quality, design 2; testing, D4: the same card, placed here in the pure crate). `fixes.rs` (new) reads clippy's JSON messages, keeps the suggestions marked machine-applicable whose span lies inside the changed ranges, and `apply` writes them, last span first. Nothing in the code reads that JSON today.
  acceptance: `cargo test -p ra-check -q fixes` passes: two suggestions on one line of a fixture are both applied and one outside the ranges is left.
  files: crates/ra-check/src/fixes.rs crates/ra-check/src/fixes/tests.rs crates/ra-check/src/lib.rs
- The lint applies those fixes and lints once more before it judges {c2}
  objective: With `fixes`, `lint_changed` (agent.rs:6147) runs clippy with JSON output, applies what is machine-applicable in the card's lines, formats, and lints again; only what is left is a warning. No model and no turn; the end line says "lint: 3 fixed by the machine".
  acceptance: `cargo test -p ra-console -q lint_fixed_by_the_machine` passes with a scripted toolbox whose first lint offers one fix and whose second is clean.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- The worker's context carries the project's eight commonest lints {c2}
  objective: The look-backs repeat the same lessons run after run: `pub(crate)` in a binary crate (live #240 was sent back on five such warnings), `writeln!` over `push_str(&format!)`, `#[must_use]` (quality, design 2). The lint names over the warnings of the project's last 50 handoffs are counted and the top eight printed with their counts in the worker's brief: each project gets its own list from its own linter with nothing to maintain.
  acceptance: `cargo test -p ra-context -q lint_primer` passes: handoffs with three unreachable_pub warnings and one single_match warning give a primer that leads with unreachable_pub and the count 3.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs crates/ra-console/src/agent.rs
- A failed check or a leftover warning returns once to the session that reported it, while the session is warm {c3}
  objective: A first run that fails its check or leaves a warning is rerun in a fresh session, which reads the card in again; the lint's own pass is a whole run (quality, design 2). Before the state is set in `run_task`, when the blame is the card's and `worth_resuming` says the session is warm, the session is resumed once with the failure alone under a cap of 8 turns, then checked and linted again; the feed says "#N continued: check, 2 turns" and one run is counted. A cold session takes the fresh rerun as now, and `lint_fix` stays for that case only.
  acceptance: `cargo test -p ra-console -q continued_in_session` passes: a scripted run whose check fails and then passes after one resumed turn ends implemented with one run counted and the continued line on the feed.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

## A tree is proven once and lands from its own worktree
objective: Moves the wall time of a landed card and the reruns that follow a landing. Today the suite runs 3 to 5 times for one clean pass, in two targets; eight landings waited 2043 s on one another (255 s each, one card at a time in the person's checkout); a card that passed its filtered check and its review can fail the crate's whole suite at the landing; 20 conflicts went to workers and #211 and #216 settled the same conflict three times each. Target, read from `rc checks`: two suite runs or fewer a landed card; 60 s or less from a run's end to landed when nothing else moved; no landing that changes or stashes a file of the person's checkout; half the conflicts settled with no model.
acceptance: Every child accepted, and the next agent walk meets the targets above.

### Every stage proves the same commands
- One function says which commands prove a card, and the run, the catch-up and the landing all call it {c3}
  objective: The run's check proves the card's own commands; the landing adds the whole suite of each changed crate (`merged_suite`, agent.rs:3614), reading only paths under `crates/` and no dependent crate. So a card is reviewed on one proof and lands, or fails to, on another, after the review is paid for (testing, F4; orchestration, D1). `proof_set(card, changed)` returns the one list; the worker's `rc prove` runs it too.
  acceptance: `cargo test -p ra-console -q one_proof_set` passes: the commands of the run's check, the catch-up and the landing of one scripted card are equal.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs apps/rc/src/prove.rs
- `ra_check::select` picks the suites from the crate graph: the changed crates and their dependents, apps included {c3}
  objective: `merged_suite` skips a change under `apps/` and every crate that depends on a changed one, so a card that breaks `rc` by changing `ra-roadmap` lands green (testing, D5). `select.rs` (new) reads `cargo metadata` once into a graph and returns, for a list of changed paths, the packages that own them and the packages that depend on those, in build order.
  acceptance: `cargo test -p ra-check -q select` passes over a saved metadata document of three packages: a change in the leaf selects all three, a change in an app selects the app.
  files: crates/ra-check/src/select.rs crates/ra-check/src/select/tests.rs crates/ra-check/src/lib.rs
- The proof set takes its suites from the graph {c2}
  objective: With `select`, `proof_set` drops the reading of `Cargo.toml` by hand in `merged_suite` and names the suites of the changed packages and their dependents (testing, D5).
  acceptance: `cargo test -p ra-console -q suites_from_the_graph` passes: a card changing a library of a fixture workspace proves the app that depends on it.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- The checkpoint formats the tree before anything proves it {c2}
  objective: The landing formats after the squash and before its check, so the tree that lands is not the tree that was proven, and a proof by hash can never be reused (testing, D1; orchestration, D1: both place fmt before the proof). `checkpoint_and_prove` (agent.rs:4008) runs the project's formatter on the changed files before it commits; the landing only checks that the format holds.
  acceptance: `cargo test -p ra-console -q formats_before_the_check` passes: a scripted run leaving an unformatted file has it formatted in its checkpoint commit.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

### A proven tree is not proven again
- The catch-up and the landing ask the ledger before they build {c3}
  objective: The same tree is compiled and tested at the run's end, after the catch-up merge and at the landing, even when nothing moved between them (testing, F3; orchestration, D2). Each stage hashes its tree and asks `proven` for each command of the proof set; a proven command writes a row marked reused and builds nothing.
  acceptance: `cargo test -p ra-console -q proven_tree` passes: a scripted card whose base did not move lands with one run of each command, and one whose base moved runs them once more.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- The catch-up proves the tree the landing will make {c2}
  objective: The catch-up merges main into the card's branch and proves the branch; the landing squashes onto main and proves that. The two trees hold the same files, so with the format settled their hashes are equal and the landing finds its proof in the ledger (orchestration, D2). The catch-up hashes the tree the squash will have and proves the whole proof set on it.
  acceptance: `cargo test -p ra-console -q catch_up_proves` passes: after a catch-up the landing of the same scripted card runs no command.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

### A card lands from a landing worktree
- `walk::landing::stage` builds the landing commit in a worktree of its own {c3}
  objective: `land` squashes into the person's checkout, stashes their changes around it, commits, checks, and undoes with `git reset --merge HEAD~1` when the check fails (agent.rs:4113-4203): the person's tree is the workbench, and one landing at a time can use it (orchestration, D1; testing, D1). `walk/landing.rs` (new, the first file of a `walk` module) holds `stage(root, id)`: in `var/worktrees/land` it resets to main's head, squashes the card's branch, commits with the landing message, and returns the commit and its tree's hash. The person's checkout is not read or written.
  acceptance: `cargo test -p ra-console -q landing_stage` passes on a fixture repository: the staged commit holds the card's change on main's head and the main checkout's files are untouched.
  files: crates/ra-console/src/walk.rs crates/ra-console/src/walk/landing.rs crates/ra-console/src/walk/landing/tests.rs crates/ra-console/src/lib.rs
- `walk::landing::advance` moves main to a staged commit by fast-forward only {c3}
  objective: Moving a branch is the one step of a landing that must touch the person's checkout, and it can be made a step that cannot half-happen (orchestration, D1). `advance(root, commit)` fast-forwards the project's branch and its checkout; it returns `Refused::Moved` when main is no longer the commit's parent and `Refused::Dirty(path)` when the checkout has an uncommitted change in a file the commit changes, and changes nothing in either case.
  acceptance: `cargo test -p ra-console -q landing_advance` passes: a clean advance, a refusal on a moved main, and a refusal naming the dirty file with the file left as it was.
  files: crates/ra-console/src/walk/landing.rs crates/ra-console/src/walk/landing/tests.rs
- `land` stages, proves and advances {c3}
  objective: With the two functions, `land` (agent.rs:4039) becomes: stage, ask the ledger for the staged tree (proving in the landing worktree only what is not proven), advance. A refusal on a moved main stages again; a refusal on a dirty file waits and says "#N waits: <path> has changes in your checkout" (orchestration, D1).
  acceptance: `cargo test -p ra-console -q lands_on_its_proof` passes: a scripted card lands with the main checkout's untracked and modified files untouched, and a failed proof leaves main where it was with no reset.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- The squash in the person's checkout, its stash and its undo are deleted {c2}
  objective: Once `land` goes through the landing worktree, the old path (the stash push and pop, the two `reset --merge` calls, the undo of a landed commit) is dead code inside the file every card reads (orchestration, table of deletions). It is removed with the tests that pinned it.
  acceptance: `cargo test -p ra-console -q never_stashed` passes, with a test that a landing beside a dirty checkout runs no stash command in the scripted git.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- The walk lands through a queue, so a run's seat is free while its card lands {c3}
  objective: A run holds its seat through its landing, and landings wait on one lock: 2043 s over eight landings in which the runner did nothing (orchestration, row 1). A finished run puts its card on a landing queue and frees its seat; one task takes the queue in order and lands each card, and the walk's end waits for the queue to empty.
  acceptance: `cargo test -p ra-console -q landing_queue` passes: with two scripted cards finishing together, a third starts before the second has landed, and all three land.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/landing.rs crates/ra-console/src/commands/tests.rs
- Cards waiting in the queue land under one proof, and a failed batch falls back to one at a time {c3}
  objective: When three cards wait, three proofs run in series though one proof of the three together would do (orchestration, D3; testing's train). The queue stages every waiting card in order on one base and proves the last tree; a pass advances main to it, a failure proves them one at a time and sends back only the card whose own proof fails.
  acceptance: `cargo test -p ra-console -q landing_batch` passes: three scripted cards land with one proof, and with the second one failing the first and third land and the second is sent back.
  files: crates/ra-console/src/walk/landing.rs crates/ra-console/src/walk/landing/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

### A conflict settled once is not settled again
- The worktrees turn on git's rerere, so a resolution is replayed {c1}
  objective: #211 and #216 met the same conflict three times each, and each time a model resolved it afresh (orchestration, row 4). `rerere.enabled` and `rerere.autoupdate` are set in the project's git config when the walk starts; a conflict the conflict pass resolved once is resolved by git on the next catch-up. The word rerere appears nowhere in the code today.
  acceptance: `cargo test -p ra-console -q rerere` passes on a fixture repository: the same conflict met a second time is resolved with no provider call.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- `walk::merge::settle_added` joins two sides that only added lines {c3}
  objective: Most of the agent's conflicts are two cards adding a test at the end of `commands/tests.rs` or an arm to one list in agent.rs: both sides added, neither changed the other's lines (orchestration, D4). `walk/merge.rs` (new) reads a conflicted file's hunks; a hunk whose two sides are pure additions to the base is settled as main's lines and then the card's; any other hunk is left for the conflict pass of #236.
  acceptance: `cargo test -p ra-console -q walk::merge` passes: a both-added hunk is joined in order, a hunk where one side changed a base line is left conflicted, and a file with both kinds settles only the first.
  files: crates/ra-console/src/walk/merge.rs crates/ra-console/src/walk/merge/tests.rs crates/ra-console/src/walk.rs
- A conflict goes to `settle_added` first and to the conflict pass only for what is left {c2}
  objective: With `settle_added`, a catch-up or landing merge that conflicts is settled by the machine where it can be, and proven by the proof set; the conflict pass of #236 gets only the hunks left, and the feed says how many were settled with no model (orchestration, D4).
  acceptance: `cargo test -p ra-console -q settled_without_a_model` passes: a scripted merge whose only conflict is two added tests lands with no provider call.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

## A walk survives its own death and stops only for a reason
objective: Moves the runs lost to a restart and the walks ended by one card. Today a restart sends every run in flight back to a full rerun (runelite: 23 restart reruns; #201 lost 220 s and its session), the window restarts 21 to 31 times a day, a walk ends on one card's error text holding the word "limit", and a walk that dies writes no line. Target on the next walk of each project: no worker turn spent on a card whose check had passed before a restart; no walk ended by one card; every walk's last feed line says why it ended and why each card left is left.
acceptance: Every child accepted, and the next walk of each project meets the targets above.

### The roadmap and the workers outlive nothing they should not
- Roadmap writes are a temp file and a rename under a cross-process lock {c3}
  objective: `Store::write` is a plain `std::fs::write` (store.rs:654-657) and every mutation is load, change, write with no lock, while the walk, the window, `rc`, the MCP server and the look-back all write the file: a concurrent write loses an update and a reader mid-write sees a cut file, which ends a walk at agent.rs:4670 (orchestration, F4). `Store::mutate` holds an OS lock on `rhipper.md.lock` around load, change and write; `write` renames a temp file over the roadmap with a short retry.
  acceptance: `cargo test -p ra-roadmap -q store_writes_whole` passes: two stores setting different cards at once lose neither, and a reader never sees a half-written roadmap.
  files: crates/ra-roadmap/src/store.rs crates/ra-roadmap/src/store/tests.rs
- A card whose check passed but was never reviewed is reviewed, not landed {c2}
  objective: `run_task` sets `Implemented` when the check passes and the review leaves it there on an accept, so a walk killed between the two leaves a card the next walk lands unreviewed (orchestration, F3; line numbers the review's). The walk's start and `land` ask for an accepting verdict on the card's last handoff and send a card without one to the review.
  acceptance: `cargo test -p ra-console -q never_reviewed` passes: an implemented card with no verdict is reviewed before it lands, and the landing refuses it.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- Worker processes die with the walk that started them {c3}
  objective: The claude child is spawned with no `kill_on_drop` and no job that closes with its owner (only `ra-tools` commands.rs:88 sets it; cpu.rs uses a job for the CPU cap alone), so a dead walk leaves workers writing in worktrees the next walk hands to a second writer (orchestration, F3). `ra_tools::cpu::own_children()` makes one job with kill-on-close on Windows and each spawned child joins it; every spawn sets `kill_on_drop(true)`.
  acceptance: `cargo test -p ra-tools -q dies_with_its_owner` passes: a child in the owning job is gone when the job closes.
  files: crates/ra-tools/src/cpu.rs crates/ra-tools/src/tests.rs crates/ra-providers/src/claude_code.rs

### A run's stage is on disk
- The run ledger: one record a card in flight, in `var/runs` {c3}
  objective: What the walk knows of its runs is rebuilt from text markers in `var/activity.log` and from memory (the `parked` set), so a new walk knows a card was running and nothing more (orchestration, F3 and D1). `walk/ledger.rs` (new) holds `Run { card, walk, stage, session, touched, sent_back, parked }`, `Stage { Dispatching, Working, Checking, Checked, Reviewing, Queued, Landing, Landed }`, and `put` (temp file and rename), `get`, `all`, `remove` over `var/runs/<id>.json`, reading `var/runs/<id>.child` for the child's pid and session when it exists. No caller yet.
  acceptance: `cargo test -p ra-console -q walk::ledger` passes: a record put is the record read back, a put cut short leaves the last whole record, and the child file fills the pid and the session.
  files: crates/ra-console/src/walk/ledger.rs crates/ra-console/src/walk/ledger/tests.rs crates/ra-console/src/walk.rs
- The provider records the claude child's pid and session as the run starts {c3}
  objective: The pid on a run's start marker is the walk's, and the session id reaches the roadmap only when the run returns, so a killed run has no child to find and no session to resume (orchestration, F3). `claude_code::run` writes `var/runs/<card>.child` (the pid, then the session id when the stream's first event names it) for a workspace `project_of` (claude_code.rs:1084) resolves to a card, and removes it when the child exits.
  acceptance: `cargo test -p ra-providers -q child_file` passes: a run in a card's worktree writes its pid and session beside the ledger, and a run outside a worktree writes no file.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs
- A run writes its stage to the ledger at every boundary {c3}
  objective: With the ledger, the run records `Working`, `Checking`, `Checked`, `Reviewing` and `Queued` as it passes them, the landing records `Landing` and `Landed`, and `clean_after` (agent.rs:4357) removes the record: a restart can tell a run that needs a worker from one that needs only a review or a landing (orchestration, D1).
  acceptance: `cargo test -p ra-console -q stage_is_on_disk` passes: a scripted run leaves checked, then reviewing, then queued, and a landed card leaves no record.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/ledger.rs crates/ra-console/src/commands/tests.rs
- Recovery is a plan read off the ledger {c2}
  objective: `requeue_orphans` (agent.rs:2579) sends every orphan back to `Defined` for a full rerun whatever it had finished; #201 lost 220 s and a session to one restart (orchestration, D1). The pure `ledger::recover(records, alive)` answers for each record one of `Resume::{Queue, Session, Rerun, Check, Review, Land, Cleanup}` by its stage, and leaves the run of a live walk alone.
  acceptance: `cargo test -p ra-console -q ledger_recover` passes: a checked run resumes at the review, a working run with a session resumes the session, and a run of a live walk is left alone.
  files: crates/ra-console/src/walk/ledger.rs crates/ra-console/src/walk/ledger/tests.rs
- A new walk sends a run that had finished its work to the stage it died before {c3}
  objective: The first half of orchestration's one c4 card. At the head of `run_parallel` (agent.rs:4616) the recovery plan replaces `requeue_orphans` for records at `Checking` or later: the worktree is checkpointed, `Checked` and `Reviewing` go to the review, `Queued` and `Landing` to the landing queue, and no worker is started for them. Runelite's 23 restart reruns are the cost today.
  acceptance: `cargo test -p ra-console -q resumes_at_its_stage` passes: a scripted walk killed after the check spends no worker turn on restart and lands the card.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/ledger.rs crates/ra-console/src/commands/tests.rs
- A new walk resumes a run killed mid-work in its session when the session is worth it {c3}
  objective: The second half. A record at `Working` with a session goes through `worth_resuming`: a warm or small session is resumed with a three-line prompt (what was interrupted, the checkpoint, the prove command); a cold large one starts fresh on the rerun block with the checkpointed work in its worktree. Either way the work on disk is kept, and `interrupted` is counted on the record.
  acceptance: `cargo test -p ra-console -q resumes_its_session` passes: a walk killed mid-run resumes the recorded session when it is two minutes old, and starts a fresh one on the kept worktree when it is an hour old and large.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/ledger.rs crates/ra-console/src/commands/tests.rs
- The window reads live runs from the ledger and no longer re-queues {c3}
  objective: `pair::window_up` adopts runs and ends in a roadmap write from the window's process beside a walk writing the same file (orchestration, F3). `window_up` lists the ledger's records with a live walk pid, starts a walk when records remain and none runs, and `adopt`, `requeue_orphans`, `requeue_orphans_with`, `pair::orphaned`, `pair::open_runs` and `pair::adoptable` are deleted.
  acceptance: `cargo test -p ra-console -q window_coming_up` passes: a window coming up shows the ledger's live runs and writes nothing to the roadmap.
  files: crates/ra-console/src/pair.rs crates/ra-console/src/pair/tests.rs crates/ra-console/src/agent.rs apps/rc-gui/src/main.rs

### No card can end the walk
- Dispatch runs inside the run's future, and a git failure parks one card {c3}
  objective: `worktree_add(&root, id).await?` runs in the walk's loop (agent.rs:4720): no run is polled meanwhile, and the `?` ends the walk and drops every run in flight (orchestration, F5). The worktree and its catch-up move into the run's future as the `Dispatching` stage; a failure parks that card with the error in words and the walk goes on.
  acceptance: `cargo test -p ra-console -q dispatch` passes: a card whose worktree cannot be made is parked and the others run.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- Only a typed provider stop ends a walk {c2}
  objective: A walk stops when a card's error text contains "unreachable", "limit", "week", "logged in" or "authentication" (agent.rs:4877-4879), read after `map_error` (agent.rs:8569) has turned the typed error into words; one card's spawn error stopped a walk at zero cards (orchestration, F5). `ProviderError::UsageOut` joins `Auth` and `Unavailable`; `map_error` wraps the three stopping kinds in a `Stop` error; the walk stops on that type alone and the substring list is deleted.
  acceptance: `cargo test -p ra-console -q typed_stop` passes: a card error that says limit does not stop the walk, one run's spawn failure parks that card only, and usage running out stops the walk after the runs in flight.
  files: crates/ra-console/src/agent.rs crates/ra-providers/src/lib.rs crates/ra-providers/src/claude_code.rs crates/ra-console/src/commands/tests.rs
- A roadmap read that fails is tried again, and every way out of the walk says why and names each card left {c3}
  objective: `store.load().await?` on each tick (agent.rs:4670) ends the walk on one bad read, the error returns write no feed line, and a walk that ends well says "no card is ready" straight after five cards were blocked (orchestration, F5 and F8). The walk keeps the last good roadmap for a tick and says the failed read once; every return from `run_parallel` writes an end line with the cause; and the end line of a walk with cards left names each with one reason (waits on a card, held at the gate, held for no progress, yours, main is red).
  acceptance: `cargo test -p ra-console -q walk_says_why` passes: one unreadable roadmap does not end the walk, a walk that fails leaves its reason on the feed, and a walk that ends with two cards left names both with their reasons.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- A question about a lock never takes the lock {c2}
  objective: `walk_running` takes and drops the walk's lock and is called for every other project by every walk, so a walk starting in that instant bails; `hub::build_slots` probes slots the same way (orchestration, F9; line numbers the review's). Both are answered from the holder's `.who` file and `pid_alive`, and `take_walk_lock` retries for half a second.
  acceptance: `cargo test -p ra-console -q asks_without_taking` passes: asking whether a walk runs leaves the lock free, and counting build slots takes none.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/hub.rs crates/ra-console/src/hub/tests.rs crates/ra-console/src/agent/tests.rs
- A stale marker stops nothing {c2}
  objective: A leftover `var/walk.stop` stops the next walk after one card, and `lookback::running` is true while its marker file exists (lookback.rs:38-40) though the pass that wrote its pid there (lookback.rs:57) is dead, which holds every later pass (orchestration, F9). The stop file carries the pid of the walk it is for and a stop for another pid is ignored; `running` asks `pid_alive`.
  acceptance: `cargo test -p ra-console -q stale_marker` passes: a stop left for a dead walk does not stop the next, and a look-back marker of a dead pass holds nothing.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/lookback.rs crates/ra-console/src/agent/tests.rs
- The window starts a walk when what the last one waited for has happened {c3}
  objective: A card parked on a red main is looked at again only when someone starts a walk (orchestration, D7). A walk that ends with such cards writes `var/walk.wake.json` naming main's commit; the window's tick, beside its `walk_requested` check, starts a walk when the branch's tip is no longer that commit. `walk/wake.rs` (new) holds the record and the pure test of it.
  acceptance: `cargo test -p ra-console -q walk::wake` passes: a moved main wakes a walk that waited on it, and an unmoved main wakes nothing.
  files: crates/ra-console/src/walk/wake.rs crates/ra-console/src/walk/wake/tests.rs crates/ra-console/src/walk.rs crates/ra-console/src/agent.rs apps/rc-gui/src/main.rs

## Every run has a cause the window can show
objective: Moves what the person can read without opening a diff. Today the metrics row is folded from the feed's prose (metrics.rs:80-183): it takes the first " in " of a line and loses the input of 62 lines, counts a failed review twice, and cuts sessions by time gaps across all projects, so the live roadmap's own target (rerun under 30%, pass over 70%) cannot be read from it; a ready card that does not run has no reason anywhere; 18% of the feed is the window going up and down; the window computes on its drawing thread at 25 frames a second. Target: every failed run of the next walk has a typed cause on its card; `rc why <id>` answers why a card is not landed; the six tiles are computed by walk and by project from events; no file is read and no query run on the drawing thread.
acceptance: Every child accepted, and after the next walk the why command names a cause for any card that did not land.

### One typed event log
- `events.rs` holds the Event record, its JSON line and its prose line {c2}
  objective: Four readers parse the prose of `var/pair.log` by substring and get it wrong (observability, section 2). The `events` module (new) holds `Event { at, card, kind }` with the `Kind`, `Phase`, `Outcome` and `Cause` enums of that section, a run's cause carrying `ra_check::Blame` so the walk and the window share one word for one failure; `emit(root, card, kind)` appends one JSON line to `var/events.jsonl` and, for every kind but `Window`, the prose line to the feed; `read(root)` skips a line it cannot parse. No call site changes.
  acceptance: `cargo test -p ra-console -q events::` passes: every kind survives its JSON line, a start writes one line to each file, a window event writes none to the feed, and a torn last line is skipped.
  files: crates/ra-console/src/events.rs crates/ra-console/src/events/tests.rs crates/ra-console/src/lib.rs
- A run's start, review, end, landing and send-back are written as events with their cause {c3}
  objective: "end ✗ … needs revision" names no cause, an implemented end is followed by a conflict send-back four times in the feed's last hundred lines, and a rerun is a plain start (observability, section 2). The feed calls at the run's start, the review, the end, the landing and the send-back become `events::emit` of `Start`, `Review`, `End`, `Landed`, `SentBack` and `Parked`, each with the cause the code holds at that point; the prose keeps the words the feed's readers match, so nothing else changes yet.
  acceptance: `cargo test -p ra-console -q run_events` passes: a failed check ends sent back with its command in the cause, a timeout is marked timed out, a conflict after an implemented end writes a send-back with the conflict cause, and the prose of an end still starts with its mark.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/pair.rs crates/ra-console/src/events.rs crates/ra-console/src/agent/tests.rs
- Phase changes are written as events, the slot wait with its card {c3}
  objective: A live run shows a clock and a step count: #241 spent 1,200 of 1,246 s in a timed-out check and a landing takes 135 to 438 s with nothing shown; the feed's 30 wait lines name no card (observability, section 3). A `Phase` event with the card is emitted at the run's start, at the entry and exit of `hold_build` (agent.rs:2079), at each acceptance command, the lint, the review, the catch-up and the landing queue; `hold_build` takes the card as an argument. A phase writes no prose unless it is a slot wait over 60 s.
  acceptance: `cargo test -p ra-console -q phase_events` passes: a held build emits a slot wait with the card and then the earlier phase, a check emits its command, and a short slot wait adds no feed line.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/events.rs crates/ra-console/src/agent/tests.rs
- Walks, look-back passes and window starts are events, and the window's lines leave the feed {c2}
  objective: 201 of the feed's 1,096 lines are the window going up or down, and they glue the metrics row's sessions together; the feed has no line for a walk's start, and a look-back pass leaves one sentence and no record of the cards it planned (observability, section 2). `WalkStart`, `WalkStop { why, ran, landed, left }`, `Lookback { evidence, planned, usd }` and `Window { up, build, pid }` are emitted where those happen; the window's lines stop going to the feed, and its restart count is read from the events.
  acceptance: `cargo test -p ra-console -q window_events` passes: a window coming up adds no feed line, the restart count over 24 hours is read from the event log, and a look-back pass emits the ids it planned.
  files: crates/ra-console/src/pair.rs crates/ra-console/src/lookback.rs crates/ra-console/src/agent.rs crates/ra-console/src/pair/tests.rs
- `events::Board` folds the log into runs, live runs, walks and passes, and `Tail` follows it by offset {c3}
  objective: Every reader needs the same answers: which attempt this is, what the last one died of, how long each phase took, what a walk cost and what its seats waited on (observability, sections 2 and 7; this is also where orchestration's walk report counts go, so there is one record of a walk). `events/board.rs` (new) holds `Board`, `RunRecord`, `LiveRun`, `WalkRecord` and `Pass`, `Board::apply(&Event)`, `yield_of(pass, board)`, and `Tail { offset, board }` whose `poll` reads only the bytes added since the last one.
  acceptance: `cargo test -p ra-console -q board_` passes: three runs of one card read back as first, second and third with causes check, conflict and none, phase spans sum to the run's seconds, a walk's cost not landed is the cost of its runs that did not land, and a poll after an append applies only the new events.
  files: crates/ra-console/src/events.rs crates/ra-console/src/events/board.rs crates/ra-console/src/events/tests.rs

### Why a card waits
- `hub::why` gives every open card and every project one reason {c3}
  objective: `hub::waiting_of` explains three states by searching feed sentences; a ready card that does not run and a project with no run have no reason anywhere (observability, section 4). `hub/why.rs` (new) holds `Why { Running, Landing, Again, Parked, Yours, WaitsOn, Behind, NoWalk, WalkStopped, NoRunner, Next }`, `why(nodes, board, slots, id)` and `project_why`, with a short form and a line; `waiting_of` keeps its signature and is rebuilt on it.
  acceptance: `cargo test -p ra-console -q why_` passes eight tests, one each for running, landing, again, yours, waits-on, behind, walk-stopped and next, each from a small roadmap and a board made of events.
  files: crates/ra-console/src/hub.rs crates/ra-console/src/hub/why.rs crates/ra-console/src/hub/tests.rs
- `rc status` prints each live run's phase and each waiting card's reason, and `rc why` prints one card's runs {c2}
  objective: `rc status` prints a header, one line, the walk's stop and three kinds of wait, reading the whole feed three times for it (observability, section 4). It builds one board from the event log and prints each live run with its phase and clock, each card in the landing queue with its age, the project's reasons, and the last walk with its runs, landings, reruns by cause and cost a landed card. `rc why <id>` prints the card's reason, one line a run with model, turns, time, cost and outcome, and the cost that did not land.
  acceptance: `cargo test -p rc -q why_lines` passes: a card whose runs failed a check, conflicted and landed prints three run lines and the cost that did not land, and a live run's line names its phase.
  files: apps/rc/src/main.rs apps/rc/src/status.rs apps/rc/src/tests.rs crates/ra-console/src/hub.rs

### The window reads and does not compute
- A Snapshot holds what a project's dashboard card draws, and a builder makes it {c3}
  objective: Each frame and each project the dashboard clones the roadmap and recomputes the walk's order, depths and waits, and once a second reads the whole feed (observability, section 8; line numbers the review's). `snapshot.rs` (new) holds `Snapshot { version, nodes, order, depths, why, project, board, changes }` and `build(…, prev)`, which reuses the parts of the last snapshot whose input did not change. No call site changes beyond the module line.
  acceptance: `cargo test -p rc-gui -q snapshot::` passes: the order equals the walk's order on the same nodes, every open card has a reason, and a build on the same nodes reuses the last order.
  files: apps/rc-gui/src/snapshot.rs apps/rc-gui/src/snapshot/tests.rs apps/rc-gui/src/main.rs
- A refresher thread loads the roadmap and builds snapshots, and frames read them {c3}
  objective: The window's tick runs every frame for every tab and, on a roadmap change, parses the whole roadmap with `block_on(self.store.load())` on the drawing thread (rc-gui main.rs:2140). One thread a window stats each open project's roadmap and polls its event tail every 250 ms, builds a snapshot when either moved, and asks for a repaint; the dashboard card, the queue and the card view read the snapshot. The loop's body is `refresh_once`, so a test can drive it.
  acceptance: `cargo test -p rc-gui -q refresher_` passes: a pass with no file change builds nothing, touching the roadmap builds one snapshot with a higher version, and an appended event does the same.
  files: apps/rc-gui/src/snapshot.rs apps/rc-gui/src/snapshot/tests.rs apps/rc-gui/src/main.rs
- The ledger, the token gauges and the diff refresh on the refresher thread {c3}
  objective: Still on the drawing thread after the card before: the usage queries of the gauges and the ledger (rc-gui main.rs:561, :2026) and the diff (observability, section 8). The snapshot gains the ledger and the diff, filled on the refresher when the usage file's time or the board moved; after this card no `block_on` is reachable from a frame.
  acceptance: `cargo test -p rc-gui -q ledger_snapshot` passes: the ledger part is rebuilt when the usage file's time moves and reused when it does not.
  files: apps/rc-gui/src/snapshot.rs apps/rc-gui/src/snapshot/tests.rs apps/rc-gui/src/main.rs
- The window repaints on change, at 10 frames a second while a run is live and at 1 without focus {c1}
  objective: The window asks for a repaint every 40 ms while any run is live (rc-gui main.rs:1365): 25 frames a second for as long as any of three projects walks (observability, section 8). A pure `cadence(live, focused)` gives 100 ms live and focused, 1 s live and unfocused, nothing idle; a frame is asked for at once when a snapshot or a sink moved.
  acceptance: `cargo test -p rc-gui -q cadence_` passes three tests, one for each case.
  files: apps/rc-gui/src/main.rs apps/rc-gui/src/tests.rs
- The sink lends a run instead of cloning it and forgets old runs {c2}
  objective: `Sink::run` clones a run with all its steps and the window calls it for each live card each frame; the map of runs is never pruned (observability, section 8; line numbers the review's). `Sink::with_run(id, f)` lends the run; ended runs beyond the newest 200 are dropped and a run keeps its newest 2,000 steps with a count of the earlier ones.
  acceptance: `cargo test -p ra-console -q sink_` passes: the 201st ended run evicts the oldest and a live run is never evicted, and a run of 2,500 steps holds 2,000 and reports 500 earlier.
  files: crates/ra-console/src/logs.rs apps/rc-gui/src/console.rs apps/rc-gui/src/main.rs

### The views
- The metrics row counts by walk and by project from the board {c3}
  objective: The row merges all projects into sessions cut by gaps, loses the input of the lines whose title holds " in " (`number_after`, metrics.rs:179) and counts a failed review twice, so no target of this roadmap can be read from it (observability, section 6). `metrics.rs` is rebuilt on the board's walks: FIRST-RUN PASS, RERUN RATE, REVIEW PASS, COST / LANDED, TOKENS / TURN, WALL / LANDED, the last ten walks as the sparkline, a scope of all or one project, the rerun causes on hover. `sessions`, `number_after` and `SESSION_GAP` are deleted.
  acceptance: `cargo test -p rc-gui -q metrics` passes: a walk of 7 runs over 4 cards with 3 reruns reads 43%, a review counted once gives 2 of 3 as 67%, one project's scope ignores another's walks, and 3.08 over 4 landed is 0.77.
  files: apps/rc-gui/src/metrics.rs apps/rc-gui/src/main.rs
- The report lists every finished card with verdict, cause, runs, cost not landed and wall time {c3}
  objective: The report keeps implemented and accepted cards only, so it cannot say where the money went: on #235, #236, #207 and #238, $8.61 of $9.93 was spent on runs that did not land (observability, section 6). `report.rs` (new) takes the report view out of main.rs and builds one row a card with a run ended in the chosen span, grouped as landed, sent back and yours, with the reviewer's first line or the cause as its sentence.
  acceptance: `cargo test -p rc-gui -q report::` passes: a card with three runs costing 2.54, 0.41 and 0.30 gives one row with 3 runs and 2.95 not landed, a card with only a failed run is in the sent-back group with its cause, and the span drops older runs.
  files: apps/rc-gui/src/report.rs apps/rc-gui/src/report/tests.rs apps/rc-gui/src/main.rs
- A since-you-looked band on each project's card, with a seen mark {c3}
  objective: Nothing in the window says what changed since the last look (observability, section 5). `since.rs` (new) holds `Since { landed, landed_usd, failed_first, failed_usd, conflicts, yours, planned }` counted from the board's runs ended after the mark in `var/seen`, and the band that draws it under each project's header; opening the project's tab writes the mark. What waits for the person does not clear on seen.
  acceptance: `cargo test -p rc-gui -q since::` passes: runs ended before the mark are not counted, a first-run failure adds its cost, what is yours counts whatever the mark, and the mark survives a round trip.
  files: apps/rc-gui/src/since.rs apps/rc-gui/src/since/tests.rs apps/rc-gui/src/main.rs
- The card view shows the card's runs and the whole verdict {c2}
  objective: The card view shows the first line of the last verdict and no history of attempts (observability, section 6). It gains a RUNS card, one line a run with its number, model, turns, time, cost, outcome and cause and a foot line with the cost not landed, and shows the verdict whole, folded past eight lines, with git's line-ending warnings filtered by a pure `verdict_lines`.
  acceptance: `cargo test -p rc-gui -q verdict_lines` passes: the line-ending warnings are dropped and the other lines keep their order.
  files: apps/rc-gui/src/main.rs apps/rc-gui/src/tests.rs
- A run card shows its phase strip, its run number, the last cause and a clock that survives a restart {c3}
  objective: A run card's status is a clock and a step count, and the clock is the window's own, reset by each of about 30 restarts a day (observability, section 3). `phase.rs` (new) holds `strip(live, now)` (the share of each phase), the phase's colour from the theme, and the status line; the title draws the run's number, "after ✗ <cause>" for a rerun, the strip, the clock from the run's start, and a warning past 120 s of work with no step.
  acceptance: `cargo test -p rc-gui -q phase::` passes: the strip's shares sum to one, a run started 600 s ago reads ten minutes whatever the window's uptime, a rerun after a failed check says so, and 130 quiet seconds of work are flagged.
  files: apps/rc-gui/src/phase.rs apps/rc-gui/src/phase/tests.rs apps/rc-gui/src/console.rs apps/rc-gui/src/main.rs
- A worker's turns and context so far reach its run card {c2}
  objective: #235's first run went 65 turns and 28 minutes before it failed, and nothing on screen counted (observability, section 3). The provider's turn tally publishes one progress line a turn with the turn count and the last turn's input; the sink folds it into the live run's `turns` and `context` in place of a step.
  acceptance: `cargo test -p ra-console -q turn_line` passes: the line for 12 turns at 61k sets both on the live run and adds no step; `cargo test -p ra-providers -q turn_progress` passes: a stream of three turns, one in two events, gives the line for 3.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs crates/ra-console/src/logs.rs
- Queue rows carry their reason, and an idle project says why it is idle {c3}
  objective: A project with no live run is one header line and a ready card shows nothing about why it is not running (observability, section 4). From the snapshot each queue row ends with the short reason and shows the line on hover; a card in the landing queue has a row with its age; an idle project draws three lines (its walk stopped when and why, or no walk; what is yours, with the command; what is next).
  acceptance: `cargo test -p rc-gui -q idle_lines` passes: a stopped walk gives a first line saying so, a card waiting for its owner prints its command, and the reason of a card behind another names that card.
  files: apps/rc-gui/src/main.rs apps/rc-gui/src/tests.rs
- The look-back badge shows the yield of the last pass {c2}
  objective: The badge counts landings toward the next pass while the last pass's first three cards (#240, #241, #245) all failed their first run (observability, section 7). From the board's last pass: K first-run landings of M planned cards run so far, in the warning colour when K is under half of M, with each planned card's outcome on hover.
  acceptance: `cargo test -p rc-gui -q lookback_badge` passes: 0 of 3 is flagged, 3 of 4 is not, and no pass yet gives the plain count.
  files: apps/rc-gui/src/main.rs apps/rc-gui/src/tests.rs
- Toasts belong to the window, show on every view and expire {c2}
  objective: Toasts are drawn only from a project's tab, so the dashboard and the report show none, and a failed toast is kept for ever (observability, section 5; line numbers the review's). The list moves to the window, each tab hands it its newly ended runs, and a pure `toast_alive(failed, age)` expires a failed toast after 30 s.
  acceptance: `cargo test -p rc-gui -q toast_` passes: a failed toast is alive at 29 s and gone at 31 s.
  files: apps/rc-gui/src/main.rs apps/rc-gui/src/tests.rs

## The agent's own suite is hermetic and its big files are carved
objective: Moves the seconds of the agent's own check and the reading a card in agent.rs costs. The evidence for the suite is inferred from the code and nothing was timed (testing, F1): 44 uses of real-cargo fixtures and elapsed-time assertions in commands/tests.rs, 2 s polls on the wall clock, a kill test whose ceiling was raised twice after misses under load (#179, #190). So this milestone comes after the check ledger, which times it. Target, read from `rc checks`: `cargo test -p ra-console -q` in 60 s or less on a warm target; no failure of the agent's suite blamed on load; agent.rs at 4,500 lines or fewer (8,600 today) and commands/tests.rs holding only module lines (4,900); conflict reruns on the agent halved (20 today).
acceptance: Every child accepted, and the checks report after the next agent walk shows the suite's median under the target.

### A suite that starts nothing it did not script
- The fixtures of the command tests move to a test kit both test trees use {c3}
  objective: The scripted worker, the scripted answers and the store fixture are private to commands/tests.rs, so agent/tests.rs grew its own copies (testing, D2). `testkit.rs` (new, compiled for tests only) holds them with one git helper and `Tools`, a toolbox script built from prefix rules that records what it was asked and answers cargo from the recorded outputs of `ra-check`. No test changes what it asserts.
  acceptance: `cargo test -p ra-console -q testkit::` passes: the toolbox script answers a passing and a failing cargo run from the recorded files and records the commands in order.
  files: crates/ra-console/src/lib.rs crates/ra-console/src/testkit.rs crates/ra-console/src/testkit/tests.rs crates/ra-console/src/commands/tests.rs crates/ra-console/src/agent/tests.rs
- A worker in a test refuses a real command unless the test says live {c3}
  objective: A worker with no script builds a real toolbox (agent.rs:250), so a test that forgets its script compiles a probe project with real cargo (testing, F1). Under test, a toolbox with no script answers every command with an error naming the command and the fix; `Worker::live()` opts out; each test the change breaks is marked live and otherwise unchanged; and a ratchet test counts the live calls in the test sources and fails above the count this card leaves.
  acceptance: `cargo test -p ra-console -q unscripted_command` passes; `cargo test -p ra-console -q the_live_tier_holds_at_most` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/commands/tests.rs crates/ra-console/src/testkit.rs
- The single-run tests answer their check from the kit instead of compiling a probe {c3}
  objective: The tests around `checked_run` build a real Cargo project and run cargo on it to learn how the agent treats a passing or a failing check; a recorded output teaches the same (testing, D2). Each is converted to a `Tools` script with its assertions kept, and the ratchet's count falls by the number converted.
  acceptance: `cargo test -p ra-console -q the_live_tier_holds_at_most` passes with the lowered count; `cargo test -p ra-console -q check_leaves_the_node` passes.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/testkit.rs
- The walk tests answer their checks from the kit {c3}
  objective: The three-card fixture feeds a walk that checks and lands each card with real cargo, several builds a test, all waiting on one target lock in 2 s polls (testing, D2). The walk and relaunch tests are converted to `Tools` scripts, keeping real git only where the test is about the landing's git, and the ratchet's count falls.
  acceptance: `cargo test -p ra-console -q the_live_tier_holds_at_most` passes with the lowered count; `cargo test -p ra-console -q run_all` passes.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/testkit.rs
- The worktree, crate-card and red-main tests answer their checks from the kit {c3}
  objective: The remaining families that let a real check run: the card-branch and crate-card projects and the red-main project (testing, D2). They are converted, keeping real git, and the ratchet falls to the tests that are about a real process, listed by name in its comment.
  acceptance: `cargo test -p ra-console -q the_live_tier_holds_at_most` passes with the lowered count; `cargo test -p ra-console -q red_main` passes.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/testkit.rs
- The live tests are an ignored tier named `live_` that the default suite skips {c2}
  objective: The tests that must run a real process (the kill of a check past its budget, a build in a worktree's target, a real cargo failure parsed) are the ones that stretch and fail under load (testing, D2). Each takes the prefix and the ignore attribute; one live test for each recorded cargo output asserts that a real run gives the same failed names and count as the file; CI runs the tier.
  acceptance: `cargo test -p ra-console -q the_default_suite_holds_no_live_test` passes: a test that reads the sources and fails when a test calling live lacks the attribute or the prefix.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/agent/tests.rs crates/ra-tools/src/tests.rs .github/workflows/build.yml rhipper.context.md

### No test waits on the wall clock
- The agent's waits run on tokio's clock and their tests on paused time {c3}
  objective: The landing guard, the target guard and `hold_build` poll every 2 s of real time, and the workspace's tokio has no `test-util` (the feature appears in no manifest), so a test of a wait waits for real and asserts on elapsed seconds (testing, F2). The feature is enabled for ra-console's tests, the three waits and the walk's tick use tokio's clock, and their tests start paused and assert what happened, never how long it took.
  acceptance: `cargo test -p ra-console -q on_paused_time` passes: a target wait ends at its limit naming the holder and a held slot is taken when freed, neither sleeping for real.
  files: crates/ra-console/Cargo.toml crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/commands/tests.rs
- The kill tests assert that the process ended, not how fast {c2}
  objective: The test of a check killed past its budget asserts elapsed under 90 s around a 15 s budget, raised twice after misses under load (#179, #190), and the toolbox's limit test sleeps 3 s (testing, F2). Each gets a child that blocks on stdin, a 1 s budget, and assertions on the timeout's text and on the child being gone; a ratchet test fails on any assertion comparing elapsed time with less-than.
  acceptance: `cargo test -p ra-tools -q past_the_toolboxs_limit` passes; `cargo test -p ra-console -q no_elapsed_ceiling` passes.
  files: crates/ra-tools/src/tests.rs crates/ra-console/src/commands/tests.rs crates/ra-console/src/agent/tests.rs apps/rc/src/tests.rs
- Stamps come from one clock a test can pin, and a ratchet stops new wall-clock calls {c3}
  objective: 142 uses of sleep, `Instant` and `SystemTime` sit in 25 files, and a test once failed as "6d 0h" rolled to "5d 23h" (testing, F2; the count is the review's). `clock.rs` (new) holds `unix_now` and a pin for tests; the `SystemTime::now` calls of agent.rs and pair.rs go through it; a test holds a table of each file's count of wall-clock calls and fails when one rises.
  acceptance: `cargo test -p ra-console -q wall_clock_ratchet` passes; `cargo test -p ra-console -q pinned_clock` passes: a feed line written under a pinned clock carries the pinned second.
  files: crates/ra-console/src/lib.rs crates/ra-console/src/clock.rs crates/ra-console/src/clock/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/pair.rs

### Files a card can read whole
- `rc carve` moves named items to a new file with no model writing the moved text {c3}
  objective: A split done by a worker costs output tokens for every moved line and a review of text that did not change; both the efficiency and the testing review ask for a move by script (efficiency, D7; testing, D7). `carve.rs` (new) holds `carve(read, from, to, items)`: it cuts each named item with its doc comment and attributes from one file, writes them to the new one under the `use` lines they need, and leaves a `mod` line and a re-export, so no caller changes; `rc carve <file> <new file> <item>…` runs it and `--dry` prints what would move.
  acceptance: `cargo test -p ra-context -q carve_moves` passes: two functions and their doc comments move to the new file, the old file re-exports them, and the dry run changes nothing.
  files: crates/ra-context/src/carve.rs crates/ra-context/src/carve/tests.rs crates/ra-context/src/lib.rs apps/rc/src/main.rs
- `rc doctor` names the files over the line budget, and a ratchet holds agent.rs to its ceiling {c2}
  objective: Nothing says a file has grown past what a worker can read: agent.rs is 8,613 lines and commands/tests.rs 4,998, and every card on either pays for it in reading and in conflicts (efficiency, D7; testing, F9). `rc doctor` lists the tracked sources over 1,500 lines with their length; a test `agent_rs_holds_at_most` reads the file's length and fails above a ceiling each carve lowers. The ceilings of the five carves below count from the 8,613 lines of today; if the file has grown by then, each carve lowers the ceiling by the same step from where it stands (700, 400, 1,000, 1,000 and 1,000 lines).
  acceptance: `cargo test -p rc -q doctor_names_long_files` passes; `cargo test -p ra-console -q agent_rs_holds_at_most` passes.
  files: apps/rc/src/doctor.rs apps/rc/src/tests.rs crates/ra-console/src/agent/tests.rs
- The review, the judge and the lint pass move to `agent/review.rs` {c2}
  objective: The first carve, made with `rc carve` and no retyping: `verdict_of`, `judge`, `review_brief`, `lint_fix` and what only they call leave agent.rs for a file a review card can read whole (efficiency, D7). Re-exported from `agent`, so no caller changes.
  acceptance: `cargo test -p ra-console -q agent_rs_holds_at_most` passes with the ceiling lowered to 7,900; `cargo test -p ra-console -q malformed_verdict` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/review.rs crates/ra-console/src/agent/tests.rs
- The build slots, the holds and the waits move to `agent/builds.rs` {c2}
  objective: The second carve: `hold_build`, `hold`, `wait` and the build allowance (about agent.rs:2070-2400) (efficiency, D7).
  acceptance: `cargo test -p ra-console -q agent_rs_holds_at_most` passes with the ceiling lowered to 7,500; `cargo test -p ra-console -q wait_line` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/builds.rs crates/ra-console/src/agent/tests.rs
- The prompt's assembly moves to `agent/prompt.rs` {c2}
  objective: The third carve: `prepare`, `harness_messages`, the task text, the digest block, `machine_notes` and the rerun text, which every card of the map milestone edits (efficiency, D7).
  acceptance: `cargo test -p ra-console -q agent_rs_holds_at_most` passes with the ceiling lowered to 6,500; `cargo test -p ra-console -q prompt_carries_the_map` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/prompt.rs crates/ra-console/src/agent/tests.rs
- The landing, the catch-up and the merge functions move into the walk module {c3}
  objective: The fourth carve: `catch_up`, `catch_up_for_review`, the conflict pass, `prove_tree`, `checkpoint_and_prove`, `land`, `land_or_say` and `clean_after` go to `walk/landing.rs` and `walk/merge.rs`, re-exported from `agent` (orchestration, D8).
  acceptance: `cargo test -p ra-console -q agent_rs_holds_at_most` passes with the ceiling lowered to 5,500; `cargo test -p ra-console -q lands_on_its_proof` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/landing.rs crates/ra-console/src/walk/merge.rs crates/ra-console/src/agent/tests.rs
- The parallel walk, the pool and the walk's lock files move out of agent.rs {c3}
  objective: The fifth carve: `run_parallel`, the pool and its seats, the walk's lock, `spawn_walk`, `stop_walk` and `relaunch` go to `walk.rs` and `walk/pool.rs` (new), re-exported from `agent` (orchestration, D8).
  acceptance: `cargo test -p ra-console -q agent_rs_holds_at_most` passes with the ceiling lowered to 4,500; `cargo test -p ra-console -q walk_says_why` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk.rs crates/ra-console/src/walk/pool.rs crates/ra-console/src/agent/tests.rs
- The run and check tests move to their own files under `commands/tests` {c3}
  objective: commands/tests.rs is 5,000 lines that every card adding a command test reads and appends to, which the overlap hold-back of #237 turns into serial runs (testing, F9). With the fixtures in the kit, the tests of single runs and of checks move with `rc carve` to `commands/tests/run.rs` and `checks.rs`; test names keep their last segment, so existing filters match.
  acceptance: `cargo test -p ra-console -q commands::tests::run::` passes; `cargo test -p ra-console -q commands::tests::checks::` passes.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/commands/tests/run.rs crates/ra-console/src/commands/tests/checks.rs
- The walk, landing and red-main tests move to their own files {c3}
  objective: The second third of the split: the tests of the walk, of landings and worktrees, and of red main (testing, D7).
  acceptance: `cargo test -p ra-console -q commands::tests::walk::` passes; `cargo test -p ra-console -q commands::tests::land::` passes; `cargo test -p ra-console -q commands::tests::red_main::` passes.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/commands/tests/walk.rs crates/ra-console/src/commands/tests/land.rs crates/ra-console/src/commands/tests/red_main.rs
- The review, plan and role tests move, and tests.rs keeps only its module lines {c3}
  objective: The last third; what is left in tests.rs is the list of modules, and the context file's line on where tests live names the directory and the kit (testing, D7).
  acceptance: `cargo test -p ra-console -q commands::tests::review::` passes; `cargo test -p ra-console -q tests_rs_holds_only_modules` passes.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/commands/tests/review.rs crates/ra-console/src/commands/tests/plan.rs crates/ra-console/src/commands/tests/roles.rs rhipper.context.md
- `rc checks` counts the conflict reruns by file {c2}
  objective: The agent had 20 conflict reruns and nobody knows how many were in the shared test file, so the split's effect cannot be read (testing, D7). The conflict pass and the send-back name the conflicted files, and the report counts conflict reruns by file.
  acceptance: `cargo test -p ra-check -q conflicts_by_file` passes; `cargo test -p ra-console -q conflict_names_its_files` passes.
  files: crates/ra-check/src/report.rs crates/ra-check/src/report/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

## One walk, with a planner that has a reason for every wait
objective: Moves the seconds a free seat waits and the code every orchestration card reads. Today the walk that runs (`spawn_walk` always passes `--parallel`, agent.rs:2994) never reads back, plans next, splits a due card or runs the retro: those calls are only in `run_all` (agent.rs:2688, :2714, :2750, :2781), which no walk runs but most walk tests exercise. The scheduler prefers and does not plan: a free seat can stay empty behind a held-back card, and one name (agent.rs) serialises the agent's own roadmap. Target: free-seat seconds with a ready card that overlaps nothing at zero on the board's walk record; one walk in the code; `run_all` and its 250 lines gone.
acceptance: Every child accepted, and the next agent walk's record shows no free-seat time against a ready card that overlapped nothing.

### One walk
- The parallel walk reads back, plans next, splits a due card and runs the retro {c3}
  objective: The four calls are made only from `run_all` (agent.rs:2688-2781), so the walk that runs does none of them and the roadmap is planned only by the look-back pass (orchestration, F6). `run_parallel` calls each at the matching point: the read-back at its start, the retro at each landing, the split and the next plan when the planner starts nothing and nothing is in flight.
  acceptance: `cargo test -p ra-console -q parallel_walk_plans` passes: a parallel walk with nothing ready plans the next cards, and one splits a card that is due.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs crates/ra-console/src/plan/tests.rs
- A running walk moves to a newer install at the next stage boundary {c3}
  objective: Only `run_all` hands its lock to a newer install; #215's conflict went back to its worker six seconds after the conflict pass landed, on a walk still running the old code (orchestration, F6). When the hub names a newer install the walk starts no new card, waits for its runs to reach a stage boundary, and relaunches; the run ledger resumes them.
  acceptance: `cargo test -p ra-console -q newer_install` passes: a parallel walk relaunches once its runs reach a boundary, and the relaunched walk resumes them from the ledger.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
- `rc run --all` is the parallel walk at width one {c3}
  objective: Two walks share one file and the tests exercise the one that does not run (orchestration, F6). `rc run --all` goes to `run_parallel` at width 1, and the tests of `run_all` move to a helper that calls it.
  acceptance: `cargo test -p ra-console -q run_all_walks` passes, the existing test of the walk's order now going through the parallel walk; `cargo test -p ra-console -q no_test_calls_the_old_walk` passes: a test that reads the test sources and fails on a call of the old walk.
  files: apps/rc/src/main.rs crates/ra-console/src/commands/tests.rs crates/ra-console/src/plan/tests.rs
- `run_all` and what only it called are deleted {c2}
  objective: With every caller on `run_parallel`, the one-at-a-time walk (agent.rs:2651-2850), its runner, its handed lock and its retry constants are about 250 dead lines in the file every orchestration card edits (orchestration, D8).
  acceptance: `cargo test -p ra-console -q agent_rs_holds_at_most` passes with the ceiling lowered by 250; `cargo test -p ra-console -q run_all_walks` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands.rs crates/ra-console/src/agent/tests.rs

### A planner with a reason for every wait
- The planner says which cards start now and why each other one waits {c3}
  objective: `spread` (agent.rs:4573) only prefers: a free seat can stay empty behind a held-back card, and ready cards run in roadmap order whatever they unblock (orchestration, F7 and D5). `walk/schedule.rs` (new) holds the pure `plan`: order by warm session, then cards unblocked, then roadmap order; start every card that fits; give each other card one `Wait`, in the words of `hub::Why`.
  acceptance: `cargo test -p ra-console -q walk::schedule` passes: a free seat goes to the next card that overlaps nothing, the card that unblocks most starts first, and every card not started has one reason.
  files: crates/ra-console/src/walk/schedule.rs crates/ra-console/src/walk/schedule/tests.rs crates/ra-console/src/walk.rs
- The walk dispatches from the planner and reads claimed files from the ledger {c3}
  objective: `claimed_files` (agent.rs:4592) runs a git diff for every running card on every 3 s tick (orchestration, F7). Each checkpoint records the touched files on the run's ledger record; the walk feeds `plan` from the roadmap and the ledger and starts what it names; `claimed_files` and the hold-back branch of the loop are deleted.
  acceptance: `cargo test -p ra-console -q planned_dispatch` passes: the walk starts the cards the plan names, and a tick with nothing new runs no git.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/schedule.rs crates/ra-console/src/commands/tests.rs
- A wide file takes more than one card, and a log says how each merge ended {c2}
  objective: Overlap is by file name and most orchestration cards name agent.rs; in the last walk 5 of 8 cards on it merged clean and 3 conflicted (orchestration, D5). A file is claimed by one card under 1,500 lines and one more for each further 3,000; every catch-up and landing appends the card, the file and the outcome (clean, recorded, added, pass, worker) to `var/merges.log`, so the rule can be corrected from what happened.
  acceptance: `cargo test -p ra-console -q width` passes: a short file holds the second card back, a wide file takes three, and a merge writes its outcome to the log.
  files: crates/ra-console/src/walk/schedule.rs crates/ra-console/src/walk/schedule/tests.rs crates/ra-console/src/agent.rs
- The pool counts a project ahead of this one no higher than its runners, and a walk without a seat says so {c2}
  objective: The pool subtracts the held seats and the ready cards of each walking project ahead in the order (agent.rs:5052-5060) with no cap at that project's own runners, so a project with 4 runners and 30 ready cards leaves the ones behind it no seat while seats are free, and the starved walk sleeps without a word (orchestration, F11). Each project's demand is capped at its runners, and a walk that first waits for a seat writes one feed line naming the projects that hold them.
  acceptance: `cargo test -p ra-console -q pool_demand` passes: a project ahead reserves no more than its runners, and a walk with no seat says who holds them.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

#### What was left out and why

Each row is a card or a design one of the five reviews proposed and this roadmap does not carry.

| Left out | From | Why |
|---|---|---|
| A card names files that exist (`brief_names_the_missing`; the `MissingFile` and `NotAPath` faults) | efficiency, quality | Live card #216 (in progress) is this card: `rc check` and the planner flag a path that is not there and not marked new. The gate here leaves file existence to it. |
| An acceptance command with a hole (`command_with_a_hole`), a check that cannot run (`check_that_cannot_run`) | observability, efficiency | Not dropped but merged: both are the `Placeholder` and `UnknownVerb` faults of the one gate in the first milestone. |
| Every merge and checkpoint is proven clean | testing, orchestration | Live card #215; the code at 787be7f already holds `prove_tree` and `checkpoint_and_prove`. |
| The worker's context states the build rules once | efficiency, quality | Live card #241. Its text must change once the shell's limit is 600 s (decision 5 above), but it is one card and it is on the roadmap. |
| `rc wait` finds the newest background job; `rc wait` takes its timings as values | efficiency, testing | Live card #240 rewrites `rc wait` and its tests (`apps/rc/src/wait.rs`). The timings change belongs in that card's rerun, not in a second card on the same lines. |
| The check's timeout scales with the load (`wait_ran_out`) | testing, orchestration | Live card #247. Its acceptance names a command that does not exist; the gate will hold it until that is fixed. |
| A piped `cargo test` is allowed; writing a file through the shell is a named gotcha | efficiency | Live card #243. |
| Read-only git is on the allowlist | efficiency | Live card #242. |
| Lint and review points quote the flagged line | quality | Live card #245. |
| The review and the handoff see the card's whole work | quality, testing | Live card #213 (in progress). `delta_review` and `same_contract` here are written to leave its diff alone. |
| A filtered test and the full run count as one check in the build budget | efficiency | Live card #244; `rc prove` makes most of it moot, and #244 stays the owner of the budget rule. |
| A worktree reads the main checkout's `third-party/` | efficiency | Live card #246. |
| Card-level points become follow-up cards; red main parks; the conflict pass; the overlap hold-back | quality, orchestration | Landed as #238, #235, #236 and #237. The reviews' cards that build on them are kept; the ones that restate them are not. |
| Send-back budgets by cause (check 3, review 2, conflict 2, landing 2, idle 2) | orchestration | One design a problem: quality's progress rule replaces fixed counts, since a card that halves its failures each round should go on and one that repeats itself should stop at two. |
| The proof cache in `var/proofs`; a stamp file beside the target | orchestration, efficiency | One design: testing's check ledger is the proof and the measure in one file. |
| `seed_target`, copying a built target into a new worktree | efficiency | Its own review says the copy's speed and cargo's acceptance of it are unproven; the leased pool gives a warm target with no copy. |
| Renaming a finished target into the pool; a landing train that keeps the stash | testing | One design: orchestration's lease file and landing worktree. The stash is what the landing milestone deletes. |
| The walk report's own counters and file (`var/walks.log`, `first_build_secs` in a report struct); `rc quality` | orchestration, quality | One record: the end line's tally now, the event board's walk record later. Three counters of one walk would disagree. |
| "runs unguarded at 15:00" on the hold line | observability | Contradicted by the slot ranks, under which no wait ends in a build without a slot. |
| Scout cards: an investigator reads in for a c4 or c5 card and leaves a map | efficiency | Conditional by its own review. The agent's project has never run an investigator; build it only if the map's hit rate stays under 0.8 after `rc map --eval`. |
| A Read of the root's copy from a worktree is answered from the worktree | efficiency | The evidence is one stream of eight. |
| A card whose check already passes on its base goes to review with no run (`already_passes`) | efficiency | One case (#201), and a passing filter on the base more often means the test does not test the card; the review's proof lines show that case instead. |
| A dependent starts on a dependency that is queued to land (`stacked`) | orchestration | A c4 its own review builds only if the walk's record shows free seats waiting on queued dependencies. |
| `[[check.contract]]` tables for what the crate graph cannot see | testing | No failure in the evidence was of this kind; `select` from the graph covers the ones that were. |
| Timing the suite by hand before any card runs | testing | The ledger's first rows are that timing, written by the walk. |
| The third-strike card's wider form (a model ruling on every held card) | quality | Kept in its narrow form in the first milestone: one turn, no tools, only when a card is held for no progress. |

#### What this roadmap asks of the live one

1. Cards #244, #246 and #247 name commands that do not exist (`rc gate`, `rc check --dry-run <card>`). Once the gate lands they are held until their acceptance is rewritten; rewriting them first costs three edits and saves three failed runs.
2. Card #241's text on build timeouts contradicts the 600 s shell limit of the second milestone. Rerun it after that milestone's first section lands.
3. The live Phase AA target (rerun under 30%, pass over 70% on the next agent walk) is read today from tiles that miscount. Until the metrics card of "Every run has a cause the window can show" lands, read it from `rc usage --runs` and `rc checks`.

#### After the import: the waits to set

Check the outline with `rc import docs/fable/agent-roadmap.md --under 1 --dry`, then import it without `--dry`. The outline format carries no waits, and a walk starts every ready card it has a seat for, so set these with `rc depend <id> <id>...` once the ids are known. A card is named below by the first test filter of its acceptance; where a later card runs the same filter again, the name means the card that comes first in this file.

"Chain" means each card of the section waits on the card above it. "Any order" means the section's cards wait on nothing inside it.

| Milestone | Section | Inside | Waits from outside the section |
|---|---|---|---|
| 1 | The count of failed runs is true | chain | none |
| 1 | A card is admitted only when a machine can run its check | chain | none |
| 1 | A failed check goes to whoever can fix it | chain | `output` (the crate `ra-check` exists) |
| 1 | A loop ends when a round changes nothing | chain | `no_progress_holds` on `names_its_cause`; `third_strike` on `held_at_the_gate` |
| 1 | A verdict is a verdict | chain | none |
| 2 | The shell's limit is the build's, not 120 seconds | chain | `hold_prints` on `output` |
| 2 | A worktree builds warm | chain | `preflight_builds_nothing` on `admit_faults` |
| 2 | A tree is checked once, by the worker, in one call | chain | `ledger` on `output`; `rc_prove` on `cause`; `first_build` on `tokens_line_says` |
| 3 | A smaller floor under every turn | any order | none |
| 3 | The prompt carries a map of the card's work | chain | `prompt_ends_on` on `rc_prove` |
| 3 | A run that wanders or went cold costs one line, not a session | any order | `nudge_at_ten` on `prompt_ends_on` |
| 4 | The worker and the reviewer read the same criteria | chain | `unproven_criteria` on `admit_faults`; `points_cite_criteria` on `points_must_cite` |
| 4 | The reviewer reads what the check proved | chain | `new_tests` on `output`; `proof_lines` on `check_rows`; `delta_review` on live card #213 |
| 4 | Lint and a failed check are settled inside the run | chain | `continued_in_session` on `names_its_cause` and `worth_resuming` |
| 5 | Every stage proves the same commands | chain | `one_proof_set` on `rc_prove` |
| 5 | A proven tree is not proven again | chain | `proven_tree` on `check_rows` and `formats_before_the_check` |
| 5 | A card lands from a landing worktree | chain | `landing_stage` on `catch_up_proves` |
| 5 | A conflict settled once is not settled again | chain | `walk::merge` on `landing_stage` (the `walk` module exists) |
| 6 | The roadmap and the workers outlive nothing they should not | any order | none |
| 6 | A run's stage is on disk | chain | `walk::ledger` on `landing_queue`; `resumes_its_session` on `worth_resuming` |
| 6 | No card can end the walk | any order | `dispatch` on `stage_is_on_disk`; `walk_says_why` on `no_progress_holds`; `walk::wake` on `walk::ledger` |
| 7 | One typed event log | chain | `events::` on `cause` |
| 7 | Why a card waits | chain | `why_` on `board_` |
| 7 | The window reads and does not compute | chain | `snapshot::` on `why_` |
| 7 | The views | any order | each on `refresher_`, but for `turn_line` and `toast_`, which wait on nothing |
| 8 | A suite that starts nothing it did not script | chain | `testkit::` on `output` |
| 8 | No test waits on the wall clock | any order | none |
| 8 | Files a card can read whole | chain | each carve on the card whose filter its acceptance names second; the three moves of the test file on `the_default_suite_holds_no_live_test`; `conflicts_by_file` on `rc_checks_prints` |
| 9 | One walk | chain | `newer_install` on `resumes_at_its_stage` |
| 9 | A planner with a reason for every wait | chain | `walk::schedule` on `why_`; `planned_dispatch` on `stage_is_on_disk` |

Milestones 6 to 9 wait on the first five only where the table says so; where seats are free they can run beside one another.
