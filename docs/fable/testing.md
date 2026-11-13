# Efficient testing: the agent's own suite and how a card is checked

Review of 2026-11-13, on the tree at `787be7f` (branch `overseer/history`) and the live state of `rhipper-agent` (`rhipper.md`, `var/pair.log`, the retro logs under `var/archive/`).

**How this was produced.** I had read and search tools only, no shell. Nothing here was run or timed by me. Every number is from the feed, the retros or the brief, and is cited. Every mechanism is read from the code and cited by `file:line`. Where I explain *why* something is slow I say "inferred": the code shows the mechanism, but nobody has measured its share. The first section of the roadmap exists to turn those inferences into numbers before the larger cards run.

Phase AA (#234 to #238), #244 and #247 are taken as done or decided. Nothing below re-proposes red-main parking, the conflict pass, the overlap hold-back, the split review points, the shared build budget or the load-scaled timeout. The design starts where they stop.

---

# Part 1: Findings and design

## The short version

A card on `ra-console` that is accepted on its first pass still pays for the whole `ra-console` suite three to five times, in two different cargo targets, and the last of those runs happens under the lock that every other landing waits on.

| Stage | Where it builds | What it runs | Evidence |
|---|---|---|---|
| Dispatch preflight | base tree, a build slot | the check with `--no-run`, never cached | `agent.rs:6540-6548` |
| The worker's own runs | worktree target | the suite, up to twice | `BUILDS_A_SESSION = 2`, `agent.rs:2113` |
| Post-run check | worktree target | named commands, then the suite | `agent.rs:5423-5640` |
| Lint | worktree target, clippy profile | clippy on the changed crates, only if the check passed | `agent.rs:5600-5620`, `6147-6196` |
| Merge proof | worktree target | a compile after each catch-up or checkpoint | `prove_tree`, `agent.rs:3787-3840` |
| Catch-up check (main moved) | worktree target | named commands again | `agent.rs:3560-3586` |
| Landing check | **the project's target**, under the landing lock | named commands plus every changed crate's suite | `agent.rs:4190-4213` |

The suite itself takes 80 to 120 s alone and 150 to 240 s while other cards build (`rhipper.context.md:26`; retro, second archive, lines 57 and 59). So one clean pass spends somewhere between 4 and 20 minutes of machine time (three to five runs of 80 to 240 s) re-proving the same tree, and a card that reruns (69% of the agent's runs) pays it again.

Four things follow from the code and the feed:

1. **The landing is the most expensive duplicate.** The seven landings of the last walk took 141 to 326 s each from `review ✓` to `landed` (median 214 s), one at a time, 28 minutes end to end (`var/pair.log:1042-1086`). When main has not moved since the catch-up, the tree being checked is byte for byte the tree already proven in the worktree, apart from a `cargo fmt` the landing applies itself.
2. **The suite is slow because many of its tests are not unit tests** (inferred). A `Worker` with no script runs real commands. The fixture that writes a real Cargo project appears 40 times in `commands/tests.rs`, and wherever the test's worker has no script, the run checks that project with a real `cargo test`. The context file says a test "never compiles a temp project for real" (`rhipper.context.md:37`); the code does otherwise.
3. **A failure is attributed late, coarsely, or not at all.** A wait that ran out on a locked target is sent to the worker as a failed check (`agent.rs:5453-5468`). A filter that matches no test passes. A test that fails under load and passes alone leaves no record outside one card's notes. A failing command with no parseable test names is never tried on the base.
4. **Nothing records what a check cost.** The feed has no check duration, no cause and no cache state. Every proposal below would be argued from anecdote without it.

Ranked by expected effect. The order is the rank; the number is the design section (D1 to D9, D0 for the ledger):

| # | Change | Moves | Main evidence |
|---|---|---|---|
| 1 | Prove a tree once; land by tree hash; landing train | wall-clock, reruns after review | landings 141-326 s serialized; `not landed: the merged tree fails` after a paid review (`pair.log:461, 713, 720`) |
| 2 | Hermetic default for the suite, live tier apart | wall-clock of every stage, reruns from timing tests | 80-240 s suite; retros #179, #190, #207 |
| 3 | A cause on every failure; machine, flaky and setup causes never reach a worker | rerun rate on all three projects | agent 49 check failures "many on main itself"; asm's 507, which match two cards looping on one setup fault (F6) |
| 9 | The reviewer reads the orchestrator's proof, not the worker's report | reviews passed, most on runelite and asm | reviews asked again for a check's result (`agent.rs:6558-6561`); #240 rejected for adding no tests. Rank is a guess: I could not read those projects' verdicts |
| 4 | Lint with the check, not after it; machine fixes first | reruns, tokens | #236 and #238 each spent a whole session on 1-2 warnings ($0.34, $0.43) |
| 5 | Selection from the workspace graph; contracts; the zero-test guard | correctness of the proof, build time | `merged_suite` skips `apps/` and dependents (`agent.rs:3614-3639`) |
| 6 | No wall clock in tests | determinism | 15 s, 13 s, 12 s, 3 s and 1 s real waits, listed below |
| 7 | Split `commands/tests.rs`; new check logic in a small pure crate | tokens a turn, merge conflicts | 4,900-line test file, 8,600-line `agent.rs` |
| 8 | Warm targets: the pool, a preflight that builds nothing, the cleanup bug | wall-clock, disk | cold target about 6 min (retro, second archive, line 123) |
| 0 | The check ledger (built first: it measures the rest) | attention | no duration or cause on the feed |

## Findings

### F1. The suite runs real cargo, real git and real python (inferred from the code)

- `Worker.script` is optional (`agent.rs:130`); `toolbox()` installs it only when set (`agent.rs:250-256`). Without it `ra_tools::Toolbox::run_words` spawns the process (`ra-tools/src/commands.rs:62-146`; the script short-circuit is lines 70-72).
- `cargo_project(dir, passing)` writes a real one-crate project (`commands/tests.rs:1221`). The name appears 40 times in the file; `with_script(` appears 39 times. `checked_run` (`commands/tests.rs:1289-1297`) is the pattern: a real project, a `Scripted` provider, no script on the toolbox, then `/run 2`. The check that follows is a real `cargo test --workspace -q` on the probe.
- The walk tests are heavier: `three_cards` (`commands/tests.rs:1311`) feeds `run_all`, which checks and lands each card, so one test is several cargo runs plus real git landings. The fixtures that build a repository or a Cargo project around a card (`three_cards`, `project_with_a_card_branch`, `project_with_a_crate_card`, `red_main_project`, `linted_run`, `empty_milestone`) appear 31 times. Not all of them let a real check run (`linted_run` scripts its check and its lint, `commands/tests.rs:3900-3902`), and I could not count exactly which do without running the suite; the guard card of section 3 produces that list as its first output.
- `preflight` defaults to on exactly when no script is set (`agent.rs:133`, `6503`), so those tests also pay a `--no-run` build first.
- The probes share one inherited `CARGO_TARGET_DIR` (comment at `commands/tests.rs:1217-1220`). `guard_target` therefore serializes them on one target lock and polls every 2 s (`agent.rs:2034-2066`). `hold_build` polls every 2 s too (`agent.rs:2079-2107`), and in tests the hub is one directory per process (`hub.rs:20-25`), so every test in the binary competes for the same 8 slots.
- Under a walk the test binary itself runs on the builders' CPU share (`ra-tools/src/commands.rs:102-103`), with up to 12 workers building.

What this predicts: a suite whose wall time is dominated by a few dozen tests queueing behind each other in 2 s steps, and which stretches by 2x or more under load. That matches the 80-120 s alone and 150-240 s loaded figures, and the early retro that the workspace took "2 s alone" before these tests existed (`var/archive/2026-11-12/retro.log:172`). It is still an inference. Two things measure it: the ledger's seconds for the suite before and after each conversion card of section 3, and, sooner, the list of unscripted tests the guard card of that section prints on its first run.

### F2. Tests wait on the wall clock

| Test | Where | Real wait | Assertion |
|---|---|---|---|
| `a_check_that_outlives_its_budget_is_killed` | `commands/tests.rs:4155-4181` | python sleeping 120 s, killed at 15 s | elapsed under 90 s |
| `a_wait_on_a_target_ends_at_the_check_timeout_naming_the_holder` | `commands/tests.rs:4221-4254` | 1 s plus a 2 s poll | elapsed under 120 s |
| `rc_wait_on_a_live_pid_outlasts_ten_silent_seconds…` | `apps/rc/src/tests.rs:263-274` | `ping -n 14`, about 13 s | elapsed at least 11 s |
| `rc_wait_on_an_empty_file_does_not_settle_on_silence` | `apps/rc/src/tests.rs:276-284` | 12 s | none on time, but it waits |
| `a_command_past_the_toolboxs_limit_is_stopped…` | `ra-tools/src/tests.rs:214-226` | python sleeping 3 s, 1 s limit | the text of the limit |

The comment on the first one records its own history: a 1 s budget missed (#179), then 5 s missed under the builders' CPU share (#190), so it is now 15 s. Each fix bought determinism with seconds. Retros #207 and #190 name the class ("a test that needs a process to print within a 1-second limit fails when the whole suite runs at once"; `var/archive/2026-11-12/retro.log:473, 506`).

The cause is structural. The waits in the agent are real time with constants: `guard_landing`, `guard_target` and `hold_build` poll at 2 s; `wait` polls at 500 ms with a 10 s settle (`agent.rs:2340-2354`); `WALK_TICK` is 3 s (`agent.rs:3015`, slept at `4800`); `LOCK_WAIT` 2 s (`agent.rs:3416`). There are 142 uses of `sleep`, `Instant` or `SystemTime` across 25 files (32 in `agent.rs`). Tokio's `test-util` feature is not enabled (`Cargo.toml`, workspace tokio features), so no test can pause time.

### F3. The landing re-proves a proven tree, in a colder place, under the lock

`land` (`agent.rs:4039-4225`) takes the landing lock (`4050`), squashes (`4111`), runs `cargo fmt --all` on main (`4158-4173`), commits (`4174`), then runs the card's commands and `merged_suite` in the project's own target (`4190-4213`), and resets the commit if they fail.

- When main has not moved since `catch_up_for_review`, the squash of a branch that already contains main is that branch's tree (`git merge --squash`, `agent.rs:4111`). The landing check repeats the review-time check on identical sources. The code's own comment says as much: "A reviewed card ran it on the caught-up tree already; this is the last guard, for a project that moved on since the review" (`agent.rs:4186-4189`). Nothing asks whether the project did move on; the guard runs every time.
- It builds in the project's target, which no worker keeps warm and which a running `rc.exe` or `rc-gui.exe` can lock (`rhipper.context.md:23`).
- `cargo fmt` at landing means the landed tree may differ from every tree that was checked or reviewed.
- Main's HEAD holds an unproven commit for the length of the check, and a dispatch can branch from it. A new worktree is cut from the project's `HEAD` (`agent.rs:3151`), and the only caller of `guard_landing` is `land` itself (`agent.rs:4050`), so nothing holds a dispatch back while a landing is being checked. If that landing then fails and resets, the new card stands on a commit main no longer has, and its own squash would carry the rejected card's changes along with it. This is read from the code, not seen in the feed, and I did not find or rule out a guard elsewhere. It deserves one test either way (the landing-worktree card in section 6 holds it), and the design below removes the window.
- A failure here comes after the review was paid for: `#182`, `#207` and `#179` each ended "not landed: the merged tree fails the check `cargo test -p ra-console -q`" (`pair.log:461, 713, 720`).
- Measured: 171, 203, 257, 214, 141, 326 and 293 s from `review ✓` to `landed` for #237, #212, #207, #208, #238, #236 and #235. #236's 326 s includes queueing behind #238.

### F4. The stages do not run the same set of commands

- Post-run and catch-up run `checks_of` (`agent.rs:5426`, `3560`). The landing adds `merged_suite` (`agent.rs:3614-3639`). A card can pass every earlier stage and meet a command for the first time at the landing.
- `merged_suite` covers only changed files under `crates/`. A card that changes `apps/rc/src/main.rs` gets no suite; a card that changes `ra-tools` does not run `ra-console`, which depends on it.
- With no named command, the default is `ra_roadmap::check::commands` (`ra-roadmap/src/check.rs:44-55`): a crate per file, else the whole workspace. Look-back cards without a backticked command got "project default cargo test --workspace -q" (#241, #242, #243 on the feed), the most expensive check there is, including the 25 s of `rc` wait tests.

### F5. A filter that matches nothing passes

Nothing in the crates reads `running 0 tests` or `filtered out` (searched). `cargo test -p ra-console -q some_name` exits 0 when no test has that name. The card's named check then proves nothing, and only the reviewer can notice (#240 was rejected because "the worker added no tests"; that verdict cost a review). `CheckRun` already carries a `tests` field (`agent.rs:6598-6619`), so the count is at hand.

### F6. Attribution has five gaps

What exists is good: `rerun_alone` (`agent.rs:7074-7089`), `main_failures` with its cache by base sha (`6866-6915`), `outside_failure` (`3647-3673`), `is_setup_failure` (`6659`), the fingerprint hold (`6669`, `6725`). The gaps:

1. **Machine causes go to the worker.** A `guard_target` wait that ran out becomes a failed `CheckRun` with `setup: false` (`agent.rs:5453-5468`); in the catch-up it becomes "the merged tree fails" (`agent.rs:3582`). The card goes to needs revision and a session is spent on a locked directory. The feed shows the slots saturated at exactly those moments ("the check waits for a worker slot, 8 of the machine's 8 are running", `pair.log:1087, 1090, 1091`).
2. **A flaky pass is forgotten.** "load-sensitive, passed alone" goes into the card's notes and nowhere else. No count per test, no card to fix it, nothing another project's run can read.
3. **Only cargo and gradle failures are tried on the base.** `rerun_words` (`agent.rs:7038-7069`) knows those two. A python contract runner or a script that fails with no test names is the card's by default, so the red-main parking of #235 does not reach the checks asm and runelite lean on most.
4. **Setup detection is a string list** (`SETUP_FAILURES`, `agent.rs:6623-6640`). #245's `cargo test -p <review-crate> relocate_points` failed with "invalid character `<` in package name", matched nothing on the list and was sent to a worker.
5. **A compile error is nobody's but the card's.** `outside_failure` gives up on any compile error (`agent.rs:3647-3673`), the commonest failure after a catch-up merge, and `depends_on_any` reads one level of `Cargo.toml` by line prefix (`agent.rs:3691`), so a transitive dependency counts as "outside".

**What asm's 507 says.** The brief counts 507 failed checks for asm. The comment on the preflight says asm #131 and #132 "ran 272 and 235 times on one malformed argument" (`agent.rs:6499-6502`), and 272 + 235 is 507. I could not open asm's feed (the session had no permission for that directory), so I cannot confirm they are the same runs. If they are, two things follow. First, asm's check failures were one setup fault looping, which the preflight and the first-sight hold have since stopped; that is the strongest evidence in the code for deciding the cause before a worker is spent, and it cost 507 sessions to learn. Second, without those runs asm has about 216 runs and 173 review rejections: its cost, like runelite's (10 of 72 reviews passed), is the reviewer, not the check. The part of this design that reaches those two projects is therefore the cause on every failure, the base run for whole commands, the contract table, and giving the reviewer the proof instead of the worker's report (D9).

### F7. The lint is a second round by construction

The lint runs only after the check passed (`agent.rs:5600-5620`), and a warning fails the run. A lint-only failure gets `lint_fix` (`agent.rs:1374-1411`), which is a whole `run_task`: a model session, then the checks and the lint again. So a card with one failing test and one warning needs at least three rounds: tests fail, tests pass and lint fails, fix pass. #236's last pass was 3 turns and $0.34, #238's 9 turns and $0.43, each to change one or two lines that clippy had already located and, for many lints, already written the replacement for.

### F8. Targets are cold where they need not be, and leak

- A fresh worktree target compiles every dependency, about 6 minutes (retro, second archive, line 123; also `var/archive/2026-11-12/retro.log:424`). `[profile.dev.package."*"] opt-level = 2` makes that slower than a default dev build.
- `clean_after` removes `root/target/wt-<id>` (`agent.rs:4374`). With a scratch root the target is at `<scratch>/target/<project>/wt-<id>` (`ra-providers/src/claude_code.rs:1029-1046`), which is never removed. The feed agrees: an early landing says "target removed" (`pair.log:744`), the later ones only "worktree removed · branch removed" (`pair.log:1036-1086`). Each leaked target is a full debug build; I could not measure the disk.
- Preflight compiles the base tree's tests at every dispatch (`agent.rs:6540-6548`; the dry run of a cargo test is `--no-run`, `agent.rs:6467-6469`), to learn only whether the tool rejects the command.

### F9. The test file and `agent.rs` are a cost in themselves

`commands/tests.rs` is about 4,900 lines with 30-odd private fixtures; `agent.rs` is about 8,600. A card that adds one test reads fixtures spread over the whole file and appends to the end of it, where every other card is appending too. The feed shows 20 merge-conflict reruns for the agent; I did not attribute them by file, and the ledger will not either, so card 5.4 counts them from git. Retro `2026-11-12:212` notes that `linted_run`, `Scripted` and `cargo_project` are private to that file, so `agent/tests.rs` grew its own copies (`scripted_checks`, `scripted`, `git_ok`, `git_in`; `agent/tests.rs:7, 730, 708, 352`).

### F10. Nothing measures a check

An `end` line carries turns, tokens, cached share and cost. No line carries how long a check ran, which stage ran it, whether it compiled from cold, or why it failed. `run_checks` (`agent.rs:6747-6816`) records no duration. The rerun causes in the brief were counted by reading text.

## Design

### D0. The check ledger: one row per check, which is also the proof

One append-only file per project, `var/checks.tsv`, written by every stage that runs a command. It is the measurement and the cache in one structure, so they cannot disagree.

```rust
// crates/ra-check/src/ledger.rs  (new crate, no dependency on ra-console)
pub struct Row {
    pub when: u64,            // unix seconds at the end of the run
    pub card: i64,
    pub stage: Stage,         // Preflight | Run | Lint | Merge | CatchUp | Land | Base
    pub tree: String,         // git tree hash of the sources the command ran on
    pub base: String,         // the main commit the card stood on
    pub command: String,
    pub stamp: String,        // toolchain stamp: `rustc -V` for cargo, "" otherwise
    pub result: Outcome,      // Pass | Fail | Waited | TimedOut
    pub cause: Option<Cause>, // set on anything but Pass (D3)
    pub seconds: u32,
    pub tests: u32,           // tests that ran, summed over the binaries
    pub new_tests: u32,       // of them, new in the card's diff (D9; 0 until section 10)
    pub reused: bool,         // true when this row stands for an earlier one and nothing ran
}

pub fn append(root: &Path, row: &Row) -> std::io::Result<()>;
pub fn proven(root: &Path, tree: &str, command: &str, stamp: &str) -> Option<Row>; // latest Pass
```

The tree hash is of the working tree, not of HEAD, since a worker never commits: `git read-tree HEAD`, `git add -A`, `git write-tree` against a throwaway `GIT_INDEX_FILE`. Three git calls, no build, and the real index is untouched. Ignored files (`var/`, targets) are outside the hash by design.

Each row also goes to the feed as one `check` line (`check #235 land · cargo test -p ra-console -q · pass · 187 s · 281 tests`), so the paired session and `rc status` see it. `rc checks` reads the file and prints the measures every later card is judged by:

- seconds by stage (median, p90) and their sum per landed card;
- whole-suite runs per landed card;
- share of rows reused;
- failures by cause, per project;
- the slowest commands.

Only a pass is ever reused. A failure is always run again (the fingerprint hold already stops the same failure twice).

**Measured by:** it is the measure. Its own acceptance is that the seven-stage table at the top of this document can be printed from data.

### D1. Prove a tree once; land by tree hash

**The rule:** a command that passed on tree T with toolchain S is not run again on T. The stages become lookups first.

What changes:

1. **One proof set.** A single function, `proof_set(root, node, changed) -> Vec<String>`, returns the commands for the post-run check, the catch-up check and the landing: the named commands, then the affected suites of D5. Today's three call sites (`agent.rs:5426`, `3560`, `4190`) call it. A command can no longer appear for the first time at the landing.
2. **Format before the proof.** The `cargo fmt --all` of `land` (`agent.rs:4158-4173`) moves to the end of the run, in the worktree, before the post-run check. `land` keeps `cargo fmt --all -- --check`, which builds nothing; if it differs the landing takes the unproven path below.
3. **Lookups.** The post-run check, the catch-up check and the landing each compute the tree and ask `proven` for every command of the set. A hit writes a `reused` row and runs nothing.
4. **The landing with an unmoved main** squashes, sees the squash's tree equal to the proven one, commits and is done. No build, no slot, a lock held for a few git calls.
5. **The landing with a moved main never builds in the project's target.** A standing landing worktree (`var/worktrees/land`, branch `land/next`, target `wt-land`, kept between landings so it is always warm) is reset to main, the card is squashed onto it there, and the proof set runs there. On a pass, main fast-forwards to that commit in the project root. On a failure nothing has touched main, so there is nothing to reset. Main moves only to a commit whose proof set passed, or whose only failures are main's already (cause `Main` or `Outside` in D3, which today also land with a note, `agent.rs:4199-4200`): a card never makes main redder than it found it. The fast-forward in the project root (`git merge --ff-only land/next`) meets the same working tree the squash meets today, so `land`'s handling of the person's own files stays as it is and moves to that call: identical untracked files are cleared, named changed files are stashed and popped after (`agent.rs:4117-4133`, `4181`). What goes is only the commit-check-reset in the root.
6. **The train.** When several cards are waiting to land, which is the normal end of a walk, they are squashed onto the landing worktree in priority order, one commit each, and the union of their proof sets runs once on the last commit. On a pass main fast-forwards over all of them. On a failure the classifier of D3 names the card: the one whose diff holds the failing test's file or crate. That card is dropped (to needs revision, with the output and "on main with #a #b"), the rest are rebuilt onto a clean reset and proven again. When no single card is named, the train halves. A conflict while squashing drops that card to the conflict pass of #236 (`resolve_landing`, reached from `land_or_say` at `agent.rs:4283-4293`) and the train goes on without it; the resolved card joins the next train.

What it removes: the landing build in the project's target; the 141-326 s lock hold; the reset path of `land` (`agent.rs:4204` and the `git reset --merge HEAD~1` around it); the window in which main holds an unproven commit; relink failures against a running `rc.exe` at landing.

Expected effect, from the last walk: seven landings in 28 minutes become one train of about one suite run. With the suite at its present 80-240 s that is 2 to 4 minutes; with D2 done, well under one.

The train depends on D2. With timing tests in the suite a train's failure is ambiguous, and seven cards wait on one flaky test. That is why the roadmap lands the tree-hash reuse (steps 1 to 5) before the hermetic work finishes, and the train (step 6) after it.

**Soundness.** A proof by tree hash is wrong only when a check reads something outside the tree: ignored state, the network, the clock. Projects whose checks do that (asm's `third-party/`, carried into worktrees per #246) can add its hash to the stamp or switch reuse off with `[check] reuse = false` in `rhipper.toml`. Default on.

**The worker's own run.** The worker is told to run the checks before reporting (`rhipper.context.md:20`), then the orchestrator runs them on the same tree. A new `rc prove` makes them one run: it runs the card's proof set from inside the worktree through the same runner, prints the same condensed failure the rerun prompt would hold (including "fails on main too" and "passed alone"), and writes the ledger row. If the worker changes nothing afterwards, the post-run check is a lookup. The worker also learns inside its session that a failure is not its own, instead of chasing it. (`rc check` is taken: it lints the roadmap, `apps/rc/src/main.rs:1445`.)

**Measured by:** `rc checks`: whole-suite runs per landed card (today 3 to 5 by the table above; target at most 2, the worker's and one proof); share of rows reused; seconds at stage `land` (today median 214 s; target under 5 s with an unmoved main); count of "not landed" feed lines after a `review ✓` (today 3 in the feed; target 0).

### D2. A hermetic suite by default, and a small live tier

**The rule:** a test in the default suite starts no process the test itself did not script, and waits on no real time. Tests that need the real tool are a named tier, few, serial, and run when what they guard changes.

1. **A test kit.** `crates/ra-console/src/testkit.rs`, `#[cfg(test)] pub(crate)`. It takes the fixtures out of `commands/tests.rs` so both test trees share them: `Scripted`, `says`, `says_at`, `store`, `worker`, and a script builder for the toolbox:

   ```rust
   /// A toolbox script built from rules, recording what it was asked.
   pub(crate) struct Tools { rules: Vec<(String, i32, String)>, seen: Arc<Mutex<Vec<String>>> }
   impl Tools {
       pub(crate) fn new() -> Self;
       /// Commands starting with `prefix` answer `code` and `text`.
       pub(crate) fn on(self, prefix: &str, code: i32, text: &str) -> Self;
       pub(crate) fn cargo_passes(self, tests: u32) -> Self;      // a recorded passing output
       pub(crate) fn cargo_fails(self, names: &[&str]) -> Self;   // a recorded failing output
       pub(crate) fn script(&self) -> ra_tools::Script;
       pub(crate) fn seen(&self) -> Vec<String>;
   }
   ```

   The canned outputs are recorded files (`crates/ra-check/fixtures/*.txt`: cargo passing, failing, a compile error, zero tests, a linker lock, clippy short format, a gradle failure), so every fake speaks the real tool's words.

2. **Hermetic by construction.** In `#[cfg(test)]`, `Worker::toolbox()` with no script returns a script that fails the test: "this test ran `cargo test --workspace -q` for real; script it or mark the worker `live()`". A new `Worker::live()` opts out. The first run of that card produces the exact list of offenders, which I could only estimate. A ratchet test counts `.live()` in the sources and fails when the count rises.

3. **Conversion by fixture family.** `checked_run` and its kin, the `three_cards` walks, and the crate-card and red-main families each become scripted. What those tests assert is the agent's behaviour given a check result (state changes, notes, feed lines), which a canned result exercises fully. Real git stays where the test is about git (worktrees, squash, conflict): it is local, deterministic and has no shared lock. Its cost is unmeasured; if the ledger shows the suite still slow after section 3, one repository template built once per process and copied per test is the fix.

4. **The live tier.** What must stay real: the toolbox spawns, times out and kills a process tree; a real cargo failure parses to the names the fixtures promise; a check builds in the worktree's own target. These keep their tests, marked `#[ignore = "live"]` and prefixed `live_`, so the default `cargo test -p ra-console -q` skips them. They are the **contract tests** of the fakes: each one runs the real tool and asserts the same facts the hermetic tests assert from the recorded file. When a toolchain update changes cargo's words, the live tier fails once, on the card or the person who updated, instead of every card inheriting a wrong fake.

5. **When the live tier runs** is the contract table of D5: when the diff touches the toolbox, the hooks or the lockfile, and in CI on every push (`.github/workflows/build.yml:52` gains `-- --include-ignored`).

Expected effect: unmeasured, so stated as a target. The early suite ran in 2 s alone. The hermetic tier should run in well under 20 s alone and stretch little under load, since it holds no lock and starts no process. That time is paid at every stage of the top table, so the saving multiplies by three to five per pass.

**Measured by:** `rc checks` median seconds of `cargo test -p ra-console -q` rows (today 80-240 s); the `.live()` count in the ratchet; rows with cause `flaky` naming an `ra-console` test (target 0).

### D3. Every failure gets a cause, and only one cause reaches a worker

```rust
// crates/ra-check/src/cause.rs
pub enum Cause {
    Card,            // fails on the tree and not on its base: the worker's
    Main,            // fails on the base commit too
    Machine(Why),    // TargetWait | SlotWait | TimedOutUnderLoad | FileLocked | DiskFull
    Flaky,           // failed in the suite, passed alone
    Setup,           // the command cannot run as written: the card's text
    Outside(String), // a panic in a crate the card neither changed nor depends on
}

pub struct Evidence<'a> {
    pub ran: bool,               // false when a wait ran out before the process started
    pub timed_out: bool,
    pub live_builds: usize,      // slots held when it started
    pub output: &'a str,
    pub tests: u32,
    pub filtered: bool,          // the command names a test filter
    pub changed: &'a [String],   // the card's files
    pub passed_alone: &'a [String],
    pub fails_on_base: Option<bool>,
}
pub fn classify(e: &Evidence<'_>) -> Cause;
```

The decision order, cheapest evidence first:

| # | Evidence | Cause | What the agent does |
|---|---|---|---|
| 1 | No process ran: a target or slot wait ran out | Machine | The check is queued again when a slot frees. The card's state does not change. No model call. |
| 2 | Output matches a file-lock, linker-lock or disk-full line (`LNK1104`, `os error 32`, `os error 5`, `os error 112`) | Machine | One retry. A second sight parks the card with a `[machine]` note in the hub's `notes.log`. |
| 3 | The command cannot run (the setup list, plus cargo's `error: invalid character`, `package ID specification … did not match`) or a filtered command ran 0 tests | Setup | Held for the person on first sight, as setup failures are today. |
| 4 | Timed out with other builds live | Machine | One retry when the slots drop below half. A second timeout on the same tree is `Card`: a hang is real. (#247 scales the limit; this is what happens past it.) |
| 5 | Compile error | Card when an error's `--> file` is one of the card's files. Otherwise Main when the base fails to compile, Card when it compiles. | The base is compiled once per sha (cached). A `Card` verdict on a file the card did not change says "breaks `<file>`, which it did not change". |
| 6 | Failed tests | Flaky for those that pass alone. Main for those that fail on the base. Outside for a panic path outside the card's crates and their dependents. Card for the rest. | `rerun_alone` and `main_failures` as today. A check whose only failures are flaky passes. |
| 7 | A failing command with no test names and no compile error | Main when the whole command fails on the base, else Card. | The command runs on the base once per (sha, command). |

Rows 1, 2, 4, 5 and 7 are new. Rows 3 and 6 exist and gain the zero-test rule and the dependents graph of D5 in place of `depends_on_any`. Row 7 is what gives a python or gradle project the red-main parking that #235 gave cargo test names.

**The flaky ledger.** `flaky.tsv` in the hub directory, shared by every project: project, command, test, sightings, last seen, last base sha. `rerun_alone` appends on every pass-alone. At three sightings in seven days the agent files one look-back note ("`<test>` failed in the suite and passed alone 3 times: #a #b #c"), which becomes one card to fix the test. A known flaky test is never skipped: skipping hides the regression it might one day catch. The ledger buys attention, not speed: one card instead of a failure nobody counts.

**The cause is written everywhere a failure is.** The ledger row, the `check` feed line, the rerun line ("rerun #221 · check failed · cause card"), the first line of the rerun prompt. The person reads the verdict and the cause, not the output.

**Measured by:** `rc checks` failures by cause and project. The target is structural: zero worker sessions started for a cause other than `Card`. The size of the win is the number of rows that turn out not to be `Card`, which nobody knows today; for the agent the brief says "many" of 49.

### D4. Lint with the check, and let the machine fix what it can

1. **Run both, report both.** The lint runs whether or not the tests passed, and the rerun prompt holds the failing tests and the warnings together. One round instead of two. The lint needs only clippy's check build, which is cheaper than the test build and does not wait for it.
2. **Machine fixes before any model.** Clippy's JSON output marks suggestions `MachineApplicable` and gives the byte span and the replacement. For warnings inside the card's changed lines (the ranges `lint_changed` already computes, `agent.rs:6147-6196`), the agent applies those replacements itself, formats, and lints again. Only what remains goes to `lint_fix`. Scoping to the card's own lines keeps the change inside the diff the reviewer sees; `cargo clippy --fix` over the crate would not.
3. **The fix pass proves only what it could have broken.** After a lint-only change the proof set runs as usual; with D1 the earlier proof is of a different tree, so it runs once, on the final tree, and that row is the one the landing reuses.

**Measured by:** runs on the feed whose line says "lint fixed on" or "lint fix pass on" (today at least two of the last seven landed cards); rounds per landed card.

### D5. Select from the graph; contracts for what the graph cannot see

1. **The workspace graph.** `cargo metadata --no-deps --format-version 1`, read once and cached by the hash of `Cargo.lock` and the member manifests, gives packages, their directories and their dependencies on each other.

   ```rust
   // crates/ra-check/src/select.rs
   pub struct Graph { /* package -> (dir, workspace dependencies) */ }
   impl Graph {
       pub fn from_metadata(json: &str) -> anyhow::Result<Self>;
       pub fn package_of(&self, file: &str) -> Option<&str>;        // nearest manifest above the file
       pub fn dependents(&self, package: &str) -> Vec<&str>;        // transitive, within the workspace
       /// The packages whose tests can change with these files.
       pub fn affected(&self, files: &[String], test_only: &dyn Fn(&str) -> bool) -> Vec<String>;
   }
   pub fn suite_command(packages: &[String]) -> Option<String>;     // one `cargo test -q -p a -p b`
   ```

   Rules: a changed package and its transitive dependents are affected. A file that is only tests (`tests.rs`, anything under a `tests/` directory) affects its own package alone, since no dependent can see it. A file outside every package (docs, `rhipper.toml`) affects nothing. The suites run as one cargo invocation, so shared dependencies compile once and cargo schedules the binaries.

   This replaces `merged_suite`'s path test and `ra_roadmap::check::commands`' crate-per-file guess. It fixes both directions of today's error: `apps/rc` gets a suite when it changes, `ra-console` runs when `ra-tools` changes, and a docs-only card stops running the workspace.

2. **I do not propose selecting tests inside a crate.** It needs coverage data, goes stale, and its failure mode is a silent false pass. D2 makes the largest crate's suite cheap enough to run whole; that is the better trade.

3. **Contracts.** A table in `rhipper.toml`, for what no graph knows:

   ```toml
   [[check.contract]]
   when = ["crates/ra-tools/**", "crates/ra-gotchas/**", "Cargo.lock"]
   run  = "cargo test -p ra-tools -p ra-console -q -- --ignored live_"
   ```

   `proof_set` appends the `run` of every entry whose `when` matches a changed file. For the agent this is the live tier. For asm and runelite it is the general lever: a glob maps to the gradle task or the contract-runner invocation that proves that part, so a card there stops running everything or nothing. The mechanism lives in the agent; each project writes only its table.

4. **The zero-test guard** (F5) is row 3 of the cause table.

**Measured by:** `rc checks` seconds at stage `run` for cards whose files are outside `ra-console`; count of `Setup` rows with "matched no test"; after-landing failures on main in a crate the landed card did not run (target 0; today not knowable).

### D6. No wall clock in a test

1. **Tokio's clock everywhere the agent waits.** `tokio` gains `test-util` as a dev-dependency feature of `ra-console`. Deadlines and polls in the agent use `tokio::time::{Instant, sleep, timeout}`, never `std::thread::sleep` or `std::time::Instant`. A hermetic test is `#[tokio::test(start_paused = true)]`: a 2 s poll or a 15-minute `HOLD_WAIT` passes in microseconds and in a fixed order.

   One caution that shapes the tiers: paused time auto-advances whenever the runtime is idle, including while it waits on a real child process. So paused time is for hermetic tests only, and a live test never pauses. The hermetic guard of D2 enforces the first half.

2. **Stamps through one function.** `unix_now()` already exists in `agent.rs`; it and the three `SystemTime::now` in `commands/tests.rs` (`2819, 2839, 3299`) go through a small `clock` module a test can pin, so "6d 0h rolling to 5d 23h" (`var/archive/2026-11-12/retro.log:139`) cannot recur.

3. **Waits take their timings as values.** `agent::wait` gets a `Timings { poll, settle, limit }` argument with today's constants as the default. The two `rc wait` tests pass milliseconds and lose their 25 s.

4. **Live timing tests assert outcomes, not durations.** The kill test keeps a real process, but one that blocks on stdin instead of sleeping, with a 1 s budget, asserting the text of the timeout and that the child is gone. No upper bound on elapsed time appears in any assertion: a ceiling is what fails under load, and the harness timeout already bounds a hang.

5. **A ratchet.** One test counts `thread::sleep`, `Instant::now` and `SystemTime::now` per source file against a table in the test, and fails when a count rises. The table only goes down.

**Measured by:** the ratchet's totals; `Flaky` rows naming a test of the agent; the `rc` suite's seconds.

### D7. Smaller files for the tests, a small crate for the check logic

1. **`crates/ra-check`**, new and pure: the ledger, the cause classifier, the selection graph, the output parsers that today sit in `agent.rs` (`failed_test_names` `7014`, `first_error` `6648`, `is_setup_failure` `6659`, `check_fingerprint` `6669`, `rerun_words` `7038`), and the recorded outputs they are tested against. No tokio, no git, no dependency on `ra-console`. Its suite is string in, value out, and should run in about a second. Every card below that adds check logic adds it here and is proven by `cargo test -p ra-check -q …`; a second, smaller card wires it into `agent.rs`. A worker on the first kind reads a few hundred lines instead of 8,600.

2. **Split `commands/tests.rs` by topic** once the kit holds the fixtures: `commands/tests/{run,checks,walk,land,review,red_main,plan,roles}.rs` under the existing `tests.rs`, which keeps only the `mod` lines. Test names do not change, so every existing filter still matches. Two cards on different topics stop appending to the same file, and the #237 hold-back, which serializes cards by overlapping files, stops serializing every card that adds a command test. The move is by line range with a script, not by retyping 4,900 lines through a model.

**Measured by:** average tokens a turn on cards whose files are under `crates/ra-check` against the 40-63k of the agent today; conflict reruns naming `commands/tests.rs`, counted from the feed and `git log` before and after (card 5.4 prints the before).

### D8. Warm targets

1. **Fix the cleanup path.** `clean_after` asks `shared_target` for the path instead of building `root/target/wt-<id>`. One line of behaviour, and it stops the leak of F8.
2. **A preflight that builds nothing.** The preflight exists to catch a command the tool rejects before a worker is spent on it, and it earns its keep (asm #131 and #132). But for a cargo test its dry run is `--no-run` (`dry_words`, `agent.rs:6459-6482`), a full compile of the base tree's tests in the project's target, to learn something that needs no compile: the arguments parse (`--help` after them fails first on a bad one, as `dry_words` already does for other cargo commands) and the `-p` names exist (a lookup in the graph of D5). Both together answer in milliseconds, and `rc check` (`preflight_faults`, `agent.rs:6523`) gets the same answer when the card is written instead of when it is dispatched. Whether the base compiles is not the preflight's question; `main_failures` and row 5 of the cause table answer it when it matters.
3. **A target pool.** When a card lands, its target is not deleted but renamed into a small pool (`<target root>/pool-<n>`, at most `[worker] parallel` of them, oldest dropped). A new worktree takes a pool target as its `wt-<id>` when one is free. Registry dependencies, which are the bulk of the cold 6 minutes and are built at `opt-level = 2`, are keyed by package and profile, not by the workspace's path, so they should be reused; the workspace's own crates rebuild as they would anyway. I expect this to hold but have not run it; the stage `Run` first-row seconds in the ledger will show it, and the card's own test covers only the pool's bookkeeping.

**Measured by:** `rc checks` seconds of each card's first cargo row (cold today about 360 s); seconds at stage `Preflight` (target: none above 1 s); the number of `wt-*` directories under the scratch target after a walk.

### D9. The reviewer reads the proof, not the report

The reviewer reads the objective, the acceptance, the last handoff and the diff (`agent.rs:1413-1419`). What it knows about the checks is mostly what the worker reported. The code records the price: "the reviews of runelite #31, #51, #73 and #152 each asked again for the contract runner's result" (`agent.rs:6558-6561`), and #240 was rejected because the worker "added no tests", a fact the check could have stated for nothing.

With the ledger the orchestrator can hand the reviewer facts in place of claims. The review prompt gains a few proof lines, built from the rows of the reviewed tree:

```text
proven on tree 3f2a91c (main 787be7f merged in), by the orchestrator:
  cargo test -p ra-console -q red_main    pass   4 tests, 3 of them new in this diff   2 s
  cargo test -q -p ra-console -p rc       pass   301 tests                            14 s
  lint over the card's lines              clean
```

- **Tests new in the diff.** After a filtered cargo test passes, the same command with `-- --list` prints the matched test names without building again. A name whose last segment appears as `fn <name>` on an added line of the card's diff is new. For gradle and scripts the line carries the count when the output gives one, and nothing otherwise.
- **A named check that ran no new test is stated, not failed.** A refactor is rightly proven by tests that already existed. The line says "none new in this diff" and the reviewer weighs it.
- **The review instruction says what the lines are:** established by the orchestrator on the tree under review. A point that only asks for evidence the lines already give is not a point.

This is the cheapest lever I can see on the two projects whose cost is review rejections, and it is a guess until measured: I did not read their verdicts.

**Measured by:** reviews passed per project (today 56 of 79, 10 of 72, 16 of 202); rejection points that ask whether a check ran or whether tests were added, counted from the verdicts before and after.

### One pass, before and after

| Step | Today | After |
|---|---|---|
| Dispatch | the named check compiled with `--no-run` on the base | packages looked up in the graph, arguments tried with `--help`; no build |
| The worker verifies | the suite once or twice, 80-240 s each | `rc prove`: one run of a hermetic suite, the row written, the cause printed |
| Post-run check | the suite again; the lint only if it passed | a lookup; the lint ran beside the tests |
| A lint warning | a model session, then the checks and the lint again | the machine applies clippy's own fix; a model only for what is left |
| Catch-up, main moved | merge, compile proof, the named checks | merge, format, the whole proof set once, in the warm worktree target |
| Review | the handoff and the diff | the proof lines and the diff |
| Landing, main unmoved | squash, format, commit, the suite in the project's target under the lock, 141-326 s | the squash's tree is proven: commit; seconds |
| Landing, main moved | the same, and a reset on failure | one train for every waiting card, in the landing worktree; main moves only on a pass |
| A failure | the worker's, unless a test fails on main or panics outside | the cause first; only `Card` reaches a worker |

### The other two projects

Nothing above is specific to cargo except the graph of D5 and the fixes of D4, and both sit behind the same interfaces a gradle or python project uses.

- **The ledger and reuse by tree** work for any command. A project whose check reads outside the tree names what it reads in the stamp, or switches reuse off.
- **The cause table** is tool-neutral from row 1 to row 4 and row 7. Rows 5 and 6 need a parser per tool; cargo and gradle have one today.
- **The contract table** is how asm and runelite get selection: their owners know which glob needs which task, and no graph will tell the agent.
- **The landing worktree and the train** are git, not cargo.
- **The proof lines** carry whatever the command printed that the agent can count.

### Order, dependencies and risks

- Section 1 goes first; without it nothing else can be judged. Section 2 and the first card of section 9 are small and independent, and can run in the same walk.
- Sections 3, 4 and 5 are a chain on one file. They should run in a quiet walk: every other card that adds a command test will conflict with them, and #237 will rightly hold those back.
- Section 6 needs the ledger. Its last two cards (the train) need sections 3 and 4, because a train with a timing test in it fails for all its cards at once.
- Sections 7, 8 and 10 need only `ra-check`.

Risks, and what holds each:

1. **A proof by tree misses an input outside the tree.** The stamp, the per-project switch, and CI, which proves every pushed commit from nothing.
2. **The fakes drift from the real tools.** The live tier asserts the same facts from a real run, in CI on every push and by contract when the toolbox changes.
3. **A train's failure is ambiguous.** It runs only on a hermetic suite, names a card by the cause table, and halves when it cannot.
4. **Paused time hides a real wait.** A paused test cannot start a process: the unscripted worker refuses.
5. **The pool may not save the dependency build.** The card's test covers the bookkeeping only; the ledger's first-row seconds decide whether it stays.
6. **These cards change the checker that checks them.** A bad card on `land` breaks landing for every project. The section 6 cards should land one at a time, the first train with the person watching, and `reuse = false` keeps today's path one line away.

### What can be done by hand before any card runs

Three things need no card, and the first two give numbers this document lacks:

1. **Time the suite by test, once.** `cargo nextest run -p ra-console` (or the suite with one test thread and the clock) on a quiet machine. If F1 is right, a few dozen tests hold most of the 80 to 120 s and their names contain the fixtures listed there.
2. **Count the leaked targets.** List `wt-*` under the scratch root's target for this project and compare with the cards still open. Every directory for a landed card is F8's leak, and its size is the disk the first card of section 9 gives back. The deletion is the person's to make.
3. **Give every look-back card a backticked check.** #241, #242 and #243 ran the whole workspace because their acceptance named no command (F4). One filtered command each is cheaper than any design here.

### Considered and left out

- **cargo-nextest.** It gives per-test timing, retries with a flaky verdict and a process per test. It would help the agent's own suite, but the checking design has to serve gradle and python projects too, so the cause, the retry and the ledger belong in the agent. One manual run by the person is worth its few minutes, though: `cargo nextest run -p ra-console` prints each test's seconds and would split today's 80 to 120 s by test before any card runs. A measurement, not a dependency of the walk.
- **Selecting tests inside a crate by coverage.** See D5.
- **Build profile changes** (`debug = "line-tables-only"`, another linker, dropping `opt-level = 2` for dependencies). Plausible seconds on every link, unmeasured, and with a warm pool the cold build becomes rare. Decide from the ledger once section 1 has a week of rows.
- **Skipping known flaky tests.** See D3.

### How each measure is read

| Measure | Today | Read from | Moved by |
|---|---|---|---|
| Rerun rate | 69 / 85 / 97% | feed, as now | D2, D3, D4 |
| Reruns by cause | counted by hand | `rc checks`, cause column | D0, D3 |
| Reviews passed | 56/79, 10/72, 16/202 | feed | D9, D3 (zero-test guard), D4 |
| Seconds from `review ✓` to `landed` | median 214 | feed timestamps; stage `land` rows | D1 |
| Whole-suite runs per landed card | 3 to 5 (from the code) | `rc checks` | D1 |
| `ra-console` suite seconds | 80-120 alone, 150-240 loaded | `rc checks` | D2, D6 |
| Sessions spent on a non-card cause | unknown | `rc checks` | D3 |
| Tokens a turn | 40-63k | usage ledger | D7 |
| First-build seconds of a card | about 360 cold | `rc checks` | D8 |

---

# Part 2: Roadmap

The outline for `rc import` is everything from the line `## Phase AB` to the end of the file. Import that part alone: `ra_roadmap::outline::parse` reads every `## `, `### ` and `- ` line of its input as an item (`crates/ra-roadmap/src/outline.rs:117-125`), so Part 1's headings would arrive as milestones. Within the outline each card is a `- ` line at the margin with its three fields indented beneath it, one line each, which is what the parser takes.

Cards are in build order: one milestone, ten sections, 43 cards. Cards whose files are all under `crates/ra-check` are proven in seconds and do not rebuild `ra-console`; each is followed by the card that wires it in.

Three rules the acceptances follow, since the agent runs every backticked command it finds there:

- The only backticked spans in an acceptance are the commands to run.
- A filter is a substring of the test path. Where it ends in `::` it names a module the card creates; otherwise the tests the card adds carry those words in their names. The conversion cards of sections 3 and 4 are the exception: their second filter names tests that exist today (`run_all`, `red_main`, `check_leaves_the_node`, `rc_wait`, `past_the_toolboxs_limit`). A card is not done while its filter matches nothing, and the second section makes the agent enforce that.
- No acceptance names the whole `ra-console` suite. The proof set adds it, and the stages after section 6 pay for it once.

## Phase AB: checks that cost seconds and name their cause

### 1. See what a check costs

- A new crate ra-check holds the check ledger, one row per check run {c2}
  objective: No line of the feed says how long a check ran, at which stage, or why it failed (run_checks at crates/ra-console/src/agent.rs:6747 records no duration), so every claim about check cost is anecdote. Add the crate crates/ra-check (pure: no tokio, no ra-console) with ledger.rs: Row { when, card, stage, tree, base, command, stamp, result, cause, seconds, tests, reused }, append(root, row) to var/checks.tsv, read(root), and proven(root, tree, command, stamp) returning the latest passing row. Tabs and newlines in a command are escaped. The workspace's members glob takes the new directory; add its line to the workspace dependencies.
  acceptance: `cargo test -p ra-check -q ledger::` passes, with tests showing a row written is read back equal, a command holding a tab survives, a pass at the same tree, command and stamp is found, and a failure or another stamp is not.
  files: Cargo.toml crates/ra-check/Cargo.toml crates/ra-check/src/lib.rs crates/ra-check/src/ledger.rs crates/ra-check/src/ledger/tests.rs

- Every check the agent runs writes its ledger row and a check line on the feed {c3}
  objective: The preflight, the post-run check, the lint, the merge proof, the catch-up check and the landing check each run commands (agent.rs:6540, 5423, 6147, 3787, 3560, 4190) and none is timed. Time each command in run_checks, compute the tree of the sources (git read-tree, add -A and write-tree against a throwaway index, so the real index is untouched), and append one Row per command with its stage; write the same facts as one feed line starting with the word check. No behaviour of a check changes in this card.
  acceptance: `cargo test -p ra-console -q check_rows` passes, with scripted tests showing a passing and a failing check each leave a row with its stage, seconds and test count, and that the feed gains one check line per command.
  files: crates/ra-console/Cargo.toml crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/pair.rs

- rc checks prints seconds by stage, suite runs per landed card, reuse and causes {c2}
  objective: The measures the rest of this phase is judged by must be one command, not a reading of the feed. Add report.rs to ra-check: from the rows, per stage the count, median and p90 seconds; whole-suite runs per landed card; the share of reused rows; failures by cause; the five slowest commands. Add the rc checks subcommand that prints it for the project, and name it in the worker allow list.
  acceptance: `cargo test -p ra-check -q report::` passes, with tests fixing the medians, the runs per landed card and the cause counts of a hand-written set of rows; `cargo test -p rc -q rc_checks_prints` passes.
  files: crates/ra-check/src/lib.rs crates/ra-check/src/report.rs crates/ra-check/src/report/tests.rs apps/rc/Cargo.toml apps/rc/src/main.rs apps/rc/src/tests.rs rhipper.toml

### 2. A failure that is not the card's never costs a worker

- The parsers of a check's output move to ra-check with recorded outputs as their tests {c3}
  objective: failed_test_names, first_error, is_setup_failure, check_fingerprint and rerun_words (agent.rs:7014, 6648, 6659, 6669, 7038) are pure string functions inside an 8,600-line file, tested through whole runs. Move them to crates/ra-check/src/output.rs, re-export them so agent.rs compiles unchanged, and test them against recorded files under crates/ra-check/fixtures (cargo passing, cargo failing two tests, a compile error, a run of zero tests, an invalid package name, a linker lock, clippy short format, a gradle failure). Add tests_run(output), the tests that ran summed over the binaries.
  acceptance: `cargo test -p ra-check -q output::` passes, with one test per recorded file naming the failed tests, the first error, the setup verdict and the test count it must give.
  files: crates/ra-check/src/lib.rs crates/ra-check/src/output.rs crates/ra-check/src/output/tests.rs crates/ra-check/fixtures/cargo-pass.txt crates/ra-check/fixtures/cargo-fail.txt crates/ra-check/fixtures/cargo-compile-error.txt crates/ra-check/fixtures/cargo-zero-tests.txt crates/ra-check/fixtures/cargo-bad-package.txt crates/ra-check/fixtures/link-locked.txt crates/ra-check/fixtures/clippy-short.txt crates/ra-check/fixtures/gradle-fail.txt crates/ra-console/src/agent.rs

- A cause for every failed check: card, main, machine, flaky, setup or outside {c3}
  objective: Attribution today is spread over rerun_alone, main_failures, outside_failure and a string list, and has no value for a machine cause at all. Add cause.rs to ra-check: the Cause enum, the Evidence struct and classify(evidence), following the seven-row decision table of docs/fable/testing.md section D3. Pure: the caller supplies whether a wait ran out, the output, the test count, the card's files, the tests that passed alone and whether the base fails.
  acceptance: `cargo test -p ra-check -q cause::` passes, with one test per row of the decision table and one showing that a second timeout on the same tree is the card's.
  files: crates/ra-check/src/lib.rs crates/ra-check/src/cause.rs crates/ra-check/src/cause/tests.rs

- A filtered test command that runs no test is a fault of the card's text {c2}
  objective: A cargo test with a filter that matches nothing exits 0, so the named check proves nothing and only the reviewer can notice (#240 was rejected for adding no tests, at the price of a review; #245's check named a placeholder package and ran twice). In run_checks, a cargo test command with a filter whose tests_run is 0 fails with the words "the filter matched no test", and that and cargo's invalid package name error are setup causes, held for the person on first sight.
  acceptance: `cargo test -p ra-console -q matched_no_test` passes, with scripted tests showing a zero-test output of a filtered command fails the check as a setup failure and that the same output of an unfiltered command passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-check/src/output.rs crates/ra-check/src/output/tests.rs

- A wait that ran out queues the check again and leaves the card where it was {c3}
  objective: When guard_target gives up, run_task builds a failed CheckRun with setup false (agent.rs:5453-5468) and catch_up_for_review reports that the merged tree fails (agent.rs:3582); the card goes to needs revision and a worker session is spent on a locked directory, at the moments the feed shows all 8 slots busy (var/pair.log lines 1087, 1090, 1091). Classify it as a machine cause: write the row, say so on the feed, keep the card's state, and have the walk run the check again when a slot frees, twice at most before parking the card with a machine note in the hub notes.
  acceptance: `cargo test -p ra-console -q wait_ran_out` passes, with tests showing the card is not needs revision after a target wait runs out, that no provider call was made for it, and that the third wait parks it.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- A compile error is attributed by its files, then by the base {c3}
  objective: outside_failure returns nothing on any compile error (agent.rs:3647-3673), the commonest failure after a catch-up merge, so a main that does not compile is charged to every card. Read the paths after the arrows of the errors; when one is a file of the card the cause is the card; otherwise compile the base once (compile_of, cached in the base-run cache by sha) and call it main when the base fails and the card's, with the file it broke named, when the base compiles.
  acceptance: `cargo test -p ra-console -q compile_error_cause` passes, with scripted tests for the three outcomes and one showing the base is compiled once for two cards on the same sha.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-check/src/output.rs crates/ra-check/src/output/tests.rs

- A failing command with no test names is run on the base before the card is blamed {c3}
  objective: main_failures reruns failed tests by name, which rerun_words knows only for cargo and gradle (agent.rs:7038-7069); a python or script check that fails is the card's by default, on projects where checks failed 507 times (asm). When a failed command yields no test names and no compile error, run the whole command on the base worktree, cached by sha and command in the base-run cache, and park the card on red main as #235 does when it fails there too.
  acceptance: `cargo test -p ra-console -q whole_command_on_base` passes, with scripted tests showing a script failing on both trees parks the card on main, one failing only on the card's tree is the card's, and a second card on the same sha reuses the base result.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- A test that passes alone is counted in a flaky ledger every project reads {c3}
  objective: rerun_alone notes "load-sensitive, passed alone" in one card's notes (agent.rs:7074-7089) and nothing counts it, so the same test costs reruns until someone notices. Add flaky.rs to ra-check: sight(dir, project, command, test, sha, card) appends to flaky.tsv in the given directory and returns the sightings in the last seven days. The agent calls it from rerun_alone with the hub directory, writes the row's cause as flaky, and at the third sighting files one look-back note naming the test and the cards.
  acceptance: `cargo test -p ra-check -q flaky::` passes, with tests for counting within the window and across projects; `cargo test -p ra-console -q third_sighting` passes, showing the note is filed once at three and not again at four.
  files: crates/ra-check/src/lib.rs crates/ra-check/src/flaky.rs crates/ra-check/src/flaky/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- The cause leads the rerun line, the rerun prompt and the ledger row {c2}
  objective: The person reads verdicts; a rerun line that says only that the check failed forces a reading of the output. Put the classified cause in the ledger row, in the feed's rerun and waits lines and as the first line of the rerun prompt, and count failures by cause in the walk's end line.
  acceptance: `cargo test -p ra-console -q names_its_cause` passes, with tests showing the rerun line and the first line of the rerun prompt carry the cause for a card failure and a main failure.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/pair.rs

### 3. A suite that starts nothing it did not script

- The fixtures of the command tests move to a test kit both test trees use {c3}
  objective: Scripted, says, says_at, store and worker are private to crates/ra-console/src/commands/tests.rs (lines 421, 486, 546, 107, 7), so agent/tests.rs grew its own copies (scripted_checks, scripted, git_ok, git_in). Add crates/ra-console/src/testkit.rs, compiled for tests only, holding those fixtures, one git helper, and Tools, a toolbox script built from prefix rules that records what it was asked and answers cargo from the recorded outputs of ra-check. Both test trees import it; no test changes what it asserts.
  acceptance: `cargo test -p ra-console -q testkit::` passes, with tests showing Tools answers a passing and a failing cargo run from the recorded files and records the commands in order.
  files: crates/ra-console/src/lib.rs crates/ra-console/src/testkit.rs crates/ra-console/src/testkit/tests.rs crates/ra-console/src/commands/tests.rs crates/ra-console/src/agent/tests.rs

- A worker in a test refuses a real command unless the test says live {c3}
  objective: A Worker with no script runs real processes (agent.rs:250-256), so a test that forgets with_script compiles a probe project with real cargo; cargo_project appears 40 times in commands/tests.rs and the context file wrongly says a test never does this. Under cfg(test), make toolbox() without a script answer every command with an error naming the command and the fix, add Worker::live() to opt out, mark each test the change breaks as live without otherwise changing it, and add a test that counts live( in the test sources and fails above the count this card leaves.
  acceptance: `cargo test -p ra-console -q unscripted_command` passes, showing an unscripted worker's check fails naming the command; `cargo test -p ra-console -q the_live_tier_holds_at_most` passes.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/commands/tests.rs crates/ra-console/src/testkit.rs rhipper.context.md

- The single-run tests answer their check from the kit instead of compiling a probe {c3}
  objective: checked_run (commands/tests.rs:1289) and the tests around it build a real Cargo project and run cargo on it to learn how the agent treats a passing or failing check; a recorded output teaches the same. Convert every live test that calls cargo_project directly or through checked_run to a Tools script, keep its assertions, and lower the live count in the ratchet by the number converted.
  acceptance: `cargo test -p ra-console -q the_live_tier_holds_at_most` passes with the lowered count, and `cargo test -p ra-console -q check_leaves_the_node` passes, running the two tests of checked_run.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/testkit.rs

- The walk tests answer their checks from the kit {c3}
  objective: three_cards (commands/tests.rs:1311) feeds run_all, which checks and lands each card with real cargo, several builds a test, all waiting on one target lock in 2 s polls. Convert the walk and relaunch tests to Tools scripts, keeping real git only where the test is about the landing's git, and lower the live count.
  acceptance: `cargo test -p ra-console -q the_live_tier_holds_at_most` passes with the lowered count, and `cargo test -p ra-console -q run_all` passes.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/testkit.rs

- The worktree, crate-card and red-main tests answer their checks from the kit {c3}
  objective: project_with_a_card_branch, project_with_a_crate_card and red_main_project (commands/tests.rs:1771, 1845, 2067) are the remaining families that let a real check run. Convert them, keep real git, lower the live count to the tests that are about a real process, and list those by name in the ratchet's doc comment.
  acceptance: `cargo test -p ra-console -q the_live_tier_holds_at_most` passes with the lowered count, and `cargo test -p ra-console -q red_main` passes.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/testkit.rs

- The live tests are an ignored tier named live_ that the default suite skips {c2}
  objective: The tests that must run a real process (the kill of a check past its budget, a build in the worktree's target, a real cargo failure parsed) are what stretch and fail under load. Rename each with the prefix live_, mark it ignored with the reason live, add one live test per recorded cargo output asserting that a real run gives the same failed names and test count as the recorded file, and run the tier in CI with the ignored tests included.
  acceptance: `cargo test -p ra-console -q the_default_suite_holds_no_live_test` passes, a test that reads the test sources and fails when a test calling live( lacks the ignore attribute or the prefix.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/agent/tests.rs crates/ra-tools/src/tests.rs .github/workflows/build.yml rhipper.context.md

### 4. No test waits on the wall clock

- The agent's waits run on tokio's clock and their tests on paused time {c3}
  objective: guard_landing, guard_target and hold_build poll every 2 s of real time (agent.rs:2006-2107) and the workspace's tokio lacks test-util, so a_wait_on_a_target_ends_at_the_check_timeout (commands/tests.rs:4221) waits for real and asserts on elapsed seconds. Enable test-util for the tests of ra-console, use tokio's Instant, sleep and timeout in those three waits and in the walk's tick, and run their tests with start_paused, asserting what happened and never how long it took.
  acceptance: `cargo test -p ra-console -q on_paused_time` passes, with tests showing a target wait ends at its limit naming the holder and a held slot is taken when freed, neither sleeping for real.
  files: crates/ra-console/Cargo.toml crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/commands/tests.rs

- rc wait takes its poll, settle and limit as values and its tests pass milliseconds {c2}
  objective: rc_wait_on_a_live_pid_outlasts_ten_silent_seconds spawns a 13 s ping and rc_wait_on_an_empty_file waits 12 s (apps/rc/src/tests.rs:263-284) because WAIT_SETTLE and the poll are constants (agent.rs:2340-2354). Give wait a Timings argument defaulting to today's values, and have the two tests pass a settle of tens of milliseconds and the test process's own pid as the live one.
  acceptance: `cargo test -p rc -q rc_wait` passes, and `cargo test -p ra-console -q wait_timings` passes, showing the default timings equal today's constants.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs apps/rc/src/main.rs apps/rc/src/tests.rs

- The kill tests assert that the process ended, not how fast {c2}
  objective: a_check_that_outlives_its_budget_is_killed asserts elapsed under 90 s around a 15 s budget raised twice after misses under load (commands/tests.rs:4155-4181, #179, #190), and the toolbox's limit test sleeps 3 s (crates/ra-tools/src/tests.rs:214-226). Give each a child that blocks on stdin and never ends by itself, a 1 s budget, and assertions on the timeout text and on the child being gone; remove every upper bound on elapsed time.
  acceptance: `cargo test -p ra-tools -q past_the_toolboxs_limit` passes, and `cargo test -p ra-console -q no_elapsed_ceiling` passes, a test that reads the test sources of ra-console, ra-tools and rc and fails on an assertion comparing elapsed time with less-than.
  files: crates/ra-tools/src/tests.rs crates/ra-console/src/commands/tests.rs crates/ra-console/src/agent/tests.rs apps/rc/src/tests.rs

- Stamps come from one clock a test can pin, and a ratchet stops new wall-clock calls {c3}
  objective: 142 uses of sleep, Instant and SystemTime sit in 25 files, and a test of until() once failed as 6d 0h rolled to 5d 23h. Add crates/ra-console/src/clock.rs with unix_now and a pin for tests, route the SystemTime::now calls of agent.rs, pair.rs and the tests through it, and add a test holding a table of the per-file counts of thread::sleep, Instant::now and SystemTime::now in the crate's sources that fails when a count rises.
  acceptance: `cargo test -p ra-console -q wall_clock_ratchet` passes, and `cargo test -p ra-console -q pinned_clock` passes, showing a feed line written under a pinned clock carries the pinned second.
  files: crates/ra-console/src/lib.rs crates/ra-console/src/clock.rs crates/ra-console/src/clock/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/pair.rs crates/ra-console/src/commands/tests.rs

### 5. Test files a card can read whole

- The run and check tests move to their own files under commands/tests {c3}
  objective: commands/tests.rs is about 4,900 lines that every card adding a command test reads and appends to, which the overlap hold-back of #237 turns into serial runs. With the fixtures in the kit, move the tests of single runs and of checks to crates/ra-console/src/commands/tests/run.rs and checks.rs by line range with a script (not by retyping), leaving mod lines in tests.rs. Test names keep their last segment, so existing filters match.
  acceptance: `cargo test -p ra-console -q commands::tests::run::` passes and `cargo test -p ra-console -q commands::tests::checks::` passes, each running at least ten tests.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/commands/tests/run.rs crates/ra-console/src/commands/tests/checks.rs

- The walk, landing and red-main tests move to their own files {c3}
  objective: The second third of the split: the tests of run_all, of landings and worktrees and of red main move to walk.rs, land.rs and red_main.rs under commands/tests, by line range with a script.
  acceptance: `cargo test -p ra-console -q commands::tests::walk::` passes, `cargo test -p ra-console -q commands::tests::land::` passes and `cargo test -p ra-console -q commands::tests::red_main::` passes.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/commands/tests/walk.rs crates/ra-console/src/commands/tests/land.rs crates/ra-console/src/commands/tests/red_main.rs

- The review, plan and role tests move, and tests.rs keeps only its mod lines {c3}
  objective: The last third: review, plan and role tests move to review.rs, plan.rs and roles.rs, and what is left in tests.rs is the list of modules. The context file's line on where tests live names the directory and the kit.
  acceptance: `cargo test -p ra-console -q commands::tests::review::` passes and `cargo test -p ra-console -q tests_rs_holds_only_modules` passes, a test that fails when commands/tests.rs holds a fn.
  files: crates/ra-console/src/commands/tests.rs crates/ra-console/src/commands/tests/review.rs crates/ra-console/src/commands/tests/plan.rs crates/ra-console/src/commands/tests/roles.rs rhipper.context.md

- rc checks counts the conflict reruns by file {c2}
  objective: The agent had 20 merge-conflict reruns and nobody knows how many were in the shared test file, so the split's effect cannot be read. Have the conflict pass and the sent-back line name the conflicted files on the feed, and have the report count conflict reruns by file from those lines.
  acceptance: `cargo test -p ra-check -q conflicts_by_file` passes, and `cargo test -p ra-console -q conflict_names_its_files` passes.
  files: crates/ra-check/src/report.rs crates/ra-check/src/report/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

### 6. Prove a tree once

- One function gives the post-run check, the catch-up check and the landing the same commands {c3}
  objective: The post-run and catch-up checks run checks_of (agent.rs:5426, 3560) and the landing adds merged_suite (agent.rs:4190-4213), so a card can meet a command for the first time at the landing, after its review was paid for (#182, #207, #179: not landed, the merged tree fails). Add proof_set(root, node, changed): the named commands, then the suites of the changed crates, and call it at all three sites.
  acceptance: `cargo test -p ra-console -q one_proof_set` passes, with scripted tests showing the three stages ask the toolbox for the same commands in the same order for a card that changes two crates.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- The run formats its worktree before the check, and the landing only verifies the format {c2}
  objective: land runs cargo fmt on main after the squash (agent.rs:4158-4173), so the landed tree can differ from every tree that was checked or reviewed, and no proof by tree hash can hold. Format in the worktree at the end of the run, before the post-run check; in land, run the format in check mode, which builds nothing, and take today's path only when it differs.
  acceptance: `cargo test -p ra-console -q formats_before_the_check` passes, with scripted tests showing the format command precedes the check in the worktree and that a landing of a formatted tree runs no formatting write.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- A command proven on a tree is not run on that tree again {c3}
  objective: The same sources are tested by the post-run check and again by the catch-up check, by a rerun that changed nothing, and by the fix pass's second run. Before running each command of the proof set, look it up in the ledger by tree, command and toolchain stamp; on a passing row write a reused row, print "proven at <stage>" and run nothing. A project can switch it off with reuse = false under [check] in rhipper.toml.
  acceptance: `cargo test -p ra-console -q proven_tree` passes, with scripted tests showing a second check of an unchanged tree asks the toolbox for nothing, that a changed file runs it again, that a failure is never reused, and that reuse = false always runs.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs rhipper.toml

- A landing whose squash is a proven tree commits without building {c3}
  objective: The seven landings of the last walk took 141 to 326 s each from review to landed, one at a time under the landing lock (var/pair.log lines 1042-1086), rebuilding in the project's target a tree the worktree had already proven. After the squash, compute the tree; when every command of the proof set has a passing row for it, commit and skip the check, saying "landed on its proof" on the feed.
  acceptance: `cargo test -p ra-console -q lands_on_its_proof` passes, with tests on a real temporary repository and a scripted toolbox showing a card caught up to main lands with no check command asked, and that one whose main moved after its proof is checked.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests/land.rs

- An unproven landing is proven in a standing landing worktree, and main moves only to a proven commit {c4}
  objective: When main moved after the review, land commits the squash on main, builds in the project's target (cold, and locked by a running rc.exe) and resets the commit on failure (agent.rs:4174-4213), so main holds an unproven commit for minutes. Keep one worktree at var/worktrees/land with its own target; reset it to main, squash the card there, run the proof set there, and fast-forward main to that commit only on a pass. On a failure main was never touched and the reset path goes.
  acceptance: `cargo test -p ra-console -q landing_worktree` passes, with tests on a real temporary repository and a scripted toolbox showing main's head is unchanged while the check runs and after it fails, is the proven commit after it passes, and that a worktree cut for another card while the check runs stands on the old head.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests/land.rs crates/ra-providers/src/claude_code.rs

- Cards waiting to land are proven together as one train {c4}
  objective: At a walk's end several reviewed cards land one after another, each paying a build and a suite, 28 minutes for seven in the last walk. With the suite hermetic, squash every waiting card onto the landing worktree in priority order, one commit each, run the union of their proof sets once on the last commit, and fast-forward main over all of them on a pass. A card that conflicts while squashing leaves the train for the conflict pass.
  acceptance: `cargo test -p ra-console -q landing_train` passes, with tests showing three waiting cards land as three commits after one run of the check, and that a conflicting card is left out while the other two land.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests/land.rs crates/ra-console/src/commands/tests/walk.rs

- A failing train drops the card its failure points at and proves the rest {c3}
  objective: A train's failure must cost one card, not all of them. Name the card whose diff holds the failing test's file or crate (by the cause classifier's evidence), send it to needs revision with the output and the cards it was proven with, rebuild the train without it and prove again; when no single card is named, halve the train.
  acceptance: `cargo test -p ra-console -q train_drops` passes, with scripted tests showing the named card is sent back and the other two land, and that an unattributed failure of four cards proves two and two.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests/land.rs

- rc prove runs a card's proof set in its worktree and the post-run check reuses it {c3}
  objective: The worker runs the suite to verify its work and the orchestrator runs it again on the same tree; the worker also cannot tell a failure of main or a flaky test from its own, and chases it. Add rc prove: from inside a worktree it runs the card's proof set through the agent's runner, prints the condensed failure with its cause, and writes the ledger rows, which the post-run check then finds. Name it in the allow list and in the context file as the way to verify before reporting.
  acceptance: `cargo test -p rc -q rc_prove` passes, and `cargo test -p ra-console -q reuses_the_workers_proof` passes, showing a post-run check after an unchanged tree asks the toolbox for nothing.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs rhipper.toml rhipper.context.md

### 7. The lint costs no extra round

- The lint runs whether or not the tests passed, and one rerun gets both {c2}
  objective: The lint runs only after the check passed (agent.rs:5600-5620), so a card with a failing test and a warning needs one round for each. Run the lint over the card's lines in every post-run check and put the warnings after the failed tests in the same rerun prompt.
  acceptance: `cargo test -p ra-console -q lint_beside_a_failed_check` passes, with a scripted test showing the rerun prompt of a run whose test failed also holds the warning on the card's line.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- Machine-applicable lint suggestions on the card's lines are applied without a model {c3}
  objective: The last passes of #236 and #238 were whole worker sessions (3 turns and $0.34, 9 turns and $0.43) to fix one or two clippy warnings whose replacement clippy had already written. Add fixes.rs to ra-check: from clippy's JSON messages, the machine-applicable suggestions whose span lies within given line ranges, and apply(text, suggestions) editing from the last span to the first. The agent applies them in the worktree, formats and lints again; only what remains goes to the fix pass.
  acceptance: `cargo test -p ra-check -q fixes::` passes, with tests on a recorded clippy JSON output showing a suggestion inside the range is applied, one outside is not, and two in one file both land; `cargo test -p ra-console -q lint_fixed_by_the_machine` passes, showing no provider call when the second lint is clean.
  files: crates/ra-check/src/lib.rs crates/ra-check/src/fixes.rs crates/ra-check/src/fixes/tests.rs crates/ra-check/fixtures/clippy-json.txt crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

### 8. Run what the change can break

- The workspace graph names the crates a change affects {c3}
  objective: merged_suite tests only changed files under crates/ (agent.rs:3614-3639): a change to apps/rc gets no suite and a change to ra-tools does not run ra-console, which depends on it; depends_on_any reads one level of a manifest by line prefix. Add select.rs to ra-check: Graph::from_metadata over the JSON of cargo metadata, package_of(file) by the nearest manifest, transitive dependents, affected(files) where a tests-only file affects its own package alone and a file outside every package affects none, and suite_command joining the packages into one cargo test.
  acceptance: `cargo test -p ra-check -q select::` passes, with tests on a recorded metadata file of this workspace showing a change in ra-tools affects ra-console and rc, a change in a tests.rs affects only its crate, and a markdown file affects nothing.
  files: crates/ra-check/src/lib.rs crates/ra-check/src/select.rs crates/ra-check/src/select/tests.rs crates/ra-check/fixtures/metadata.json

- The proof set takes its suites from the graph, in one cargo run {c3}
  objective: Wire the graph in: proof_set reads cargo metadata once a base commit (cached by the hash of Cargo.lock and the manifests), replaces merged_suite's path rule and the crate-per-file default of ra_roadmap::check::commands with the affected packages in one command, and outside_failure uses the graph's dependents in place of depends_on_any. A card with no named command and no affected package runs no suite and says so.
  acceptance: `cargo test -p ra-console -q suites_from_the_graph` passes, with scripted tests showing a card on apps/rc/src/main.rs runs the rc suite, one on ra-tools runs its dependents in the same command, and one on a markdown file runs none.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-roadmap/src/check.rs

- A contract table runs a named check when matching files change {c3}
  objective: Some proofs no build graph knows: the live tier when the toolbox or the lockfile changes, a gradle task or a contract-runner invocation for one part of asm or runelite. Read [[check.contract]] entries (when: globs, run: a command) from rhipper.toml and have proof_set append the run of each entry a changed file matches, once. Add the agent's own entry for the live tier.
  acceptance: `cargo test -p ra-console -q contract_table` passes, with scripted tests showing a matching file adds the command once, a non-matching one adds nothing, and a command the tool would reject is reported by the roadmap check.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs rhipper.toml rhipper.context.md

### 9. Targets that start warm

- A landed card's target is removed where it is, under the scratch root too {c1}
  objective: clean_after removes root/target/wt-<id> (agent.rs:4374), but with a scratch root the target is under the scratch (shared_target, crates/ra-providers/src/claude_code.rs:1029-1046) and is never removed: the feed's later landings say worktree and branch removed and no longer target removed. Ask shared_target for the path.
  acceptance: `cargo test -p ra-console -q removes_the_scratch_target` passes, with a test giving a scratch root in a temporary directory and showing the worktree's target directory is gone after the landing's cleanup.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- The preflight of a cargo test checks its packages and its arguments and builds nothing {c2}
  objective: The dry run of a cargo test is --no-run (dry_words, agent.rs:6459-6482): a full compile of the base tree's tests in the project's target at every dispatch (agent.rs:6540-6548), to learn whether the tool rejects the command. Dry-run a cargo test with --help after its arguments, as other cargo commands already are, and look its -p names up in the workspace graph of ra-check; a name the graph lacks is the rejection, with the packages that exist named. rc check reports the same through preflight_faults, so #245's placeholder package is caught when the card is written.
  acceptance: `cargo test -p ra-console -q preflight_builds_nothing` passes, with scripted tests showing the dry run of a cargo test asks the toolbox for no --no-run command, that an unknown package holds the card naming the known ones, and that a known package with a filter goes on to its worker.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- A finished card's target is kept in a pool and the next worktree starts from it {c3}
  objective: A fresh worktree target compiles every dependency, about 6 minutes, which outlasted the foreground limit in at least eight retros. Instead of deleting a landed card's target, rename it into a pool beside the others (at most the worker's parallel count, the oldest dropped), and give a new worktree a pooled target as its own when one is free. The ledger's first cargo row of each card shows whether the dependencies were reused.
  acceptance: `cargo test -p ra-console -q target_pool` passes, with tests in a temporary scratch root showing a landed card's target joins the pool, the next worktree takes it, the pool never exceeds its size, and two worktrees never take the same one.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-providers/src/claude_code.rs

### 10. The reviewer reads the proof

- The tests a named check ran are told from the ones the card added {c3}
  objective: A named check that passes says nothing about whether the card added a test for its change; #240 was rejected at review for adding none, a fact the check could have stated for nothing. Add to ra-check's output.rs listed_tests(output), which reads the names a cargo test prints with the list flag, and new_tests(listed, diff), the listed names whose last segment is a fn on an added line of the diff. After a filtered cargo test passes, the agent runs the same command with the list flag (no rebuild) and stores the count of new tests on the ledger row, which gains that field.
  acceptance: `cargo test -p ra-check -q new_tests` passes, with tests on a recorded list output and a diff showing an added test is counted, one that only moved is not, and a name shared by two modules is matched by its last segment; `cargo test -p ra-console -q counts_the_new_tests` passes, showing the row of a passing filtered check carries the count.
  files: crates/ra-check/src/output.rs crates/ra-check/src/output/tests.rs crates/ra-check/src/ledger.rs crates/ra-check/src/ledger/tests.rs crates/ra-check/fixtures/cargo-list.txt crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- The review prompt holds the proof lines of the tree under review {c3}
  objective: The reviewer reads the handoff and the diff (agent.rs:1413-1419) and knows the checks mostly by the worker's report; the reviews of runelite #31, #51, #73 and #152 each asked again for a check's result (agent.rs:6558-6561), on projects where 10 of 72 and 16 of 202 reviews pass. Build a short block from the ledger rows of the reviewed tree (each command, pass, tests run, how many are new in the diff, seconds, and the lint's result), put it in the review prompt above the handoff, and tell the reviewer the lines are the orchestrator's on the tree under review, so a point that only asks for what they state is not raised. A named check that ran no new test is stated on its line, not failed.
  acceptance: `cargo test -p ra-console -q proof_lines` passes, with scripted tests showing the review prompt holds one line per proven command with its test count and its new tests, that a tree with no rows gets no block, and that the block names the tree it was proven on.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/review.rs
