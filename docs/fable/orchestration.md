# Orchestration and reliability: the walk that never wastes a run

Reviewed 2026-11-13 against branch `overseer/history` (787be7f) and the agent project's feed
(`var/pair.log`, 1096 lines). Line numbers are `crates/ra-console/src/agent.rs` unless another
file is named. Feed times are the feed's own unix seconds.

The short version: the worker is not the only source of waste in the walk. Four orchestration
habits are the rest. The walk moves the project's branch *before* it has proven the merged tree;
it gives every card a build target that starts empty; it keeps what it knows about its runs in
memory and in a text log it parses back; and one card's failure can end or stall every other
card's run. Each shows in the feed or in the workers' look-backs as reruns, as minutes of
serialized waiting, as six to fifteen minutes of compiling dependencies per card, or as a walk
that ends with "no card is ready" when that is not the reason. Phase AA (#234–#238) and the look-back cards (#240–#247) attack the reruns a worker
causes; nothing here repeats them. This document is about the reruns and the waiting the walk
causes itself.

How to read the two parts: Part 1 uses no `##` or `###` headings and no `- ` list items, so by
the rules of the outline parser (`crates/ra-roadmap/src/outline.rs:103–163`) it holds no item
and the whole file can be given to `rc import`; I did not run the import. Part 2, from the line
`## The walk that never wastes a run` to the end of the file, is the outline.

Two kinds of evidence are kept apart below. "Observed" means the feed shows it happening.
"Latent" means the code allows it and I found no line in the feed or in `var/walk.log` (which is
empty) showing that it has happened yet.


Part 1: Findings and design
===========================

The ranking
-----------

Ranked by expected effect on the measures (runs per landed card, review pass rate, wall-clock,
the person's attention). "Measured by" names a field of the walk report designed in section D7,
so each change can be judged by the walk after it lands.

| # | Change | What it removes, with the evidence | Measured by |
|---|--------|------------------------------------|-------------|
| 1 | Landing proves in its own tree, then moves the branch; proofs are cached by tree; accepted cards land in batches | 2043 s of serialized landing wait over 8 landings in the last walk (avg 255 s); an unproven commit at HEAD for minutes that other cards branch from and merge in; the same check run up to three times per pass | `landing_wait_secs`, `checks_run` against `checks_cached`, `landing_refused` |
| 2 | Build targets are leased warm from a pool, one card at a time each, and the cleanup looks where the build was | Observed, in the workers' own look-backs: "a fresh worktree target compiles every dependency (about 6 minutes)" (#214), 6.5 minutes (#242), 15 minutes (#243); at least six more cards lost a turn or a result when that build passed the 120 s foreground limit. Read from the code: `clean_after` looks for `<root>/target/wt-<id>` while the build uses `<scratch>/target/<project>/wt-<id>` | `first_build_secs`; scratch disk use |
| 3 | A run ledger on disk, children that die with their walk, and a restart that resumes each run at its stage | #201 started twice across a window restart (220 s and a second session); runelite's 23 restart reruns; a card whose check passed but was never reviewed is landed by the next walk; a dead walk's `claude` processes keep writing in worktrees a new walk hands out again | `interrupted` runs and the turns they cost; `resumed_at` by stage |
| 4 | Conflicts settled without a model: recorded resolutions replayed, both-added hunks merged mechanically | Observed: 20 merge conflicts sent back to a worker (the brief); #211 and #216 conflicted three times each | `merges` by tier |
| 5 | Send-back budgets by cause, kept in the ledger | Observed: #223 took 8 runs to land; the ~20 "blocked: sent back 3 times in a row" lines came only from the idle rule, because a parallel walk caps nothing else | `runs / landed`, `max_runs_one_card` |
| 6 | A planner that is a pure function: fills every free seat, orders by what a card unblocks, and gives every waiting card a reason; the pool counts another project's demand no higher than its runners | Observed: eight cards on one 8600-line file dispatched in 44 s, three of them conflicted. By #237's own note: a free seat stays empty behind a held-back card. Read from the code: a project ahead in the order reserves seats it can never use | `idle_slot_secs` by reason |
| 7 | An honest end line and a wake condition | Observed: "walk ran 18 card(s) · no card is ready" right after five cards were blocked. Latent: a card that waits on main is only looked at again when somebody starts a walk | the end line itself; `left`, `wake` |
| 8 | One card's failure cannot end or stall the walk: dispatch inside the run's future, typed stops, an atomic and locked roadmap file | Observed: "walk ran 0 card(s) · stopped" on one card's spawn error. Latent: a `?` on one card's `git worktree add` ends the walk and drops up to 12 runs in flight; a half-written `rhipper.md` read by another process | walks in `walks.log` with no `ended` |
| 9 | One walk: `run_all` goes, `run_parallel` gains what only `run_all` has | Observed: a running parallel walk never moves to a newer install; #215's conflict went back to its worker 6 s after the conflict pass (#236) landed | lines deleted; `install` changing inside one walk |

Rows 1 and 2 are the wall-clock: between them they account for ten to twenty minutes of every
card's first pass that no worker is thinking in. Row 3 is what makes every later change
crash-safe for free. Rows 1 to 7 are ranked on what the feed and the workers' look-backs show;
row 8 is mostly insurance and is cheap. The roadmap in Part 2 is in build order, which is not
this order: the ledger comes first there because the landing queue, the budgets and the planner
all write to it.


What the feed shows
-------------------

The last full walk of the agent project (walk pid 19396) is the cleanest sample, because Phase
AA was its cargo.

| Card | Runs to land | end ✓ → landed |
|------|--------------|----------------|
| #237 | 1 | 171 s |
| #214 | 2 | 438 s |
| #212 | 2 | 203 s |
| #207 | 2 | 257 s |
| #238 | 2 | 141 s |
| #236 | 2 | 326 s |
| #208 | 3 | 214 s |
| #235 | 3 | 293 s |
| #215 | 3 | (see below) |

1. Eight cards were dispatched in 44 s (1794551061 to 1794551105), every one on `ra-console`.
   First runs took 14 to 34 minutes (#236: 49 turns, 4.03M tokens in; #235: 65 turns, 3.88M).
2. The eight landings waited 2043 s between them from "end ✓" to "landed", one at a time, and the
   project's branch moved eight times in about thirty minutes. Each move made every card still
   running stale.
3. Four cards were refused a review because main had moved under them: "not reviewed: main merged
   into card/X with conflicts … sent back to its worker" for #221, #208 (…552783), #235 (…553380)
   and #215 (…553990). The last came six seconds after #236, the card that adds the conflict
   pass, landed at …553984: the walk was still the old program.
4. Six times a check waited: "the check waits for a worker slot, 8 of the machine's 8 are
   running".
5. One card of nine landed on its first run.

Across the whole feed:

1. About ninety `window down restart` / `window up` pairs; the feed's own count is 21 to 31
   restarts per 24 hours.
2. #201 started at …533632 under pid 27344, the window restarted at …533852, and the card started
   again at …533960 under pid 33276: 220 s of work thrown away. It was then accepted, not landed
   ("conflicts in files git did not name"), rerun for one turn at $0.53, accepted again, and
   landed at …534823.
3. Fifteen "not landed: … conflicts in files git did not name" lines (#211 three times, #216
   three times, #213, #196 twice each, others once). #211's worker wrote: "I made no changes, and
   this is the third time #211 has been sent after it landed." #236 answers the refusal itself;
   what it does not answer is why a landing is the place a card finds out.
4. About twenty "blocked: sent back 3 times in a row" lines. #223 took eight runs before it
   landed.
5. Checks that failed on the merged tree at landing, after the review had accepted: #182, #207,
   #179, #221.
6. Walk ends: "walk ran 0 card(s) · stopped: the run of #81 failed: the provider is unreachable:
   cannot start claude: The filename or extension is too long. (os error 206)" (line 57); "walk
   ran 18 card(s) · no card is ready" directly after five cards were blocked (line 859); the same
   closing words after 24, 17, 13 and 7 cards.
7. `var/walk.log`, which a detached walk's stderr is appended to (2999–3004), is empty. So no
   walk has died on an error since that file was last cleared, and the walk-ending paths in F5
   other than the two stops above are latent.

The workers' look-backs (`var/retro.log` and `var/archive/*/retro.log`) add what the feed
cannot see, the time inside a run:

1. A new card's first build is cold. #214: "a fresh worktree target compiles every dependency
   (about 6 minutes)". #242: "A cold `cargo build -p rc` took about 6.5 minutes in the worktree".
   #243: "`cargo build -p rc` took 15 minutes on a cold worktree target". Against that, the full
   `ra-console` suite on a warm target is "about 150 s" (#224) and was 78 s on the shared tree
   (#124).
2. The cold build passes the worker's 120 s foreground limit and goes to the background, which
   costs a turn or the result: #223 ("its background run was lost with the session"), #228,
   #231, #232, #237, #214.
3. Why the targets are per card at all: on 2026-11-12 the worktrees shared one target and
   sibling cards overwrote each other's builds (#176: "cargo's build hash ignores where the
   worktree sits"; LNK1104 on #178, #181, #182; a sibling's test binary judged #179's check).
4. A build that waits for a slot says nothing. #213: "`rc hold -- cargo test` printed nothing
   for over 12 minutes, most likely waiting for a build slot, so the session ended with no test
   result". The feed has 30 "wait:" lines in all.
5. A merge git calls clean still breaks the build: #210 twice and #213 once met a struct literal
   that main had added without the card's new field.

Of the agent project's 147 runs the brief counts 20 as merge conflicts sent back to a worker.
The feed adds the restart reruns and the landing refusals on top. A fair reading is that between
a fifth and a quarter of the project's runs were caused by the walk, not by the card or its
worker.


Findings
--------

#### F1. The landing moves the branch first and proves it afterwards, in the person's checkout, under a lock every other card waits on

`land` (4039–4225) commits the worktree (4057–4064), runs `git merge --squash` into the project's
own checkout (4111), stashes the person's files around it (4129), runs `cargo fmt --all` in the
root on a build slot (4158–4173), commits (4174), and only then runs the check on the merged tree
(4190–4213). A failure is undone with `git reset --merge HEAD~1` (4203). `run_parallel` calls it
through `land_or_say` while holding the walk's one in-process git mutex (4761–4764); a card that
only wants to checkpoint (`checkpoint_and_prove`, 4766) waits behind it.

Five things follow.

1. Serialized minutes. The check is a full build and test run (2–4 minutes for
   `cargo test -p ra-console`). Eight landings, 2043 s.
2. An unproven HEAD. While the check runs, HEAD of the project's branch is a commit nobody has
   proven. `worktree_add` branches new cards from HEAD (3138–3160) and `catch_up` merges it into
   running ones (3462–3492), and neither takes the lock. When the check then fails and the commit
   is reset away, the rejected work stays inside the branches that took it. Their next merge of
   main conflicts with code main never had, or lands the rejected change under another card's
   name.
3. A crash between the commit and the check leaves the commit (latent). The next walk lands the
   card again (4629–4636), the squash stages nothing, `land` says the branch "is in HEAD already"
   (4152–4153), skips the check because nothing is staged (4192), and sets the card accepted
   (4217) on a tree that was never checked.
4. The person's checkout is the workbench. The stash, the fmt and the reset all run in the tree
   the person edits. `cargo fmt --all` (4160) reformats every Rust file in that checkout, the
   person's uncommitted ones included, though only the card's are staged (4162–4170). The
   comment at 4047–4049 already records one hand merge clobbered by a landing.
5. The landing is where the whole suite first runs. The review's catch-up proves the card's own
   commands on the merged tree (3560–3586); the landing adds `cargo test -p <crate> -q` for every
   crate the squash changed (`merged_suite`, 3614–3639). A card that breaks another test of its
   crate finds out after the reviewer has been paid for: #182, #207, #179 and #221 failed there.
   #244 moves the full suite into the worker's check; the catch-up needs the same set so that
   the landing's proof is the review's proof.

#### F2. The same tree is proven again and again

One clean pass of a card runs its check in `run_task` (5450), again on the merged tree in
`catch_up_for_review` (3515–3608) when main moved, and again in `land` (4190–4213) plus
`merged_suite` (3614). The comment at 4186–4189 calls the last one "the last guard, for a project
that moved on since the review", but it runs whether the project moved or not. When main did not
move between two of those the trees are identical byte for byte and the second run can only
repeat the first. Every run holds one of the machine's
build slots (`hub::BUILD_SLOTS = 4`, at most 8, `hub.rs:897–900`), which is where the "8 of 8
are running" waits come from. A rerun that changed no file runs the whole check again too.

A run that is sent back pays once more. Its future checkpoints the worktree and then compiles
the checkpoint on a build slot (`checkpoint_and_prove`, 4008–4016; `prove_tree`, 3787–3840). But
the run's own check ran on that same content seconds earlier: if it failed on a test, the tree
compiled; if it failed to compile, the notes already say so and the proof block says it a second
time. The compile after a checkpoint only has something to find after a merge.

#### F3. What the walk knows about its runs is not on disk

1. A run's liveness is rebuilt by parsing start and end lines out of `var/activity.log`
   (`pair::orphaned`, `pair.rs:234–249`; `open_runs`, 267–284), and the pid on those lines is
   `std::process::id()` (1053, 1062): the walk's, not the `claude` process's.
2. The provider spawns `claude` (`claude_code.rs:351–373`) with no `kill_on_drop`, no job object
   and no record of the child's pid. A walk that dies leaves its children running with nobody
   reading them. The next walk sees a dead walk pid, re-queues the card (`requeue_orphans`,
   2579–2612) and starts a second writer in the same worktree.
3. The session id reaches the roadmap only when the run returns (5236–5240). A run killed at turn
   40 has no session to resume, so its successor reads everything in again.
4. `parked` is a `HashSet` in the walk's memory (4651). Only the idle count survives a restart
   (`idle_rounds`, `ra-context/src/lib.rs:1060`).
5. `Implemented` means two things. `run_task` sets it when the check passes (5652), before any
   reviewer has looked; `review_after_run` leaves it there when the verdict accepts (1452). A
   walk that dies between the two (the review's catch-up alone is a multi-minute check) leaves a
   card "implemented" with no verdict. The next walk lands every implemented card first
   (4629–4636), and `land` asks only for the state (4041–4045). I found no test of a verdict on
   that path. This is the one place where the person's rule, "a card is review-ready only with a
   verdict attached" (1086), can be broken by timing alone.
6. The window re-queues too: `pair::window_up` (`pair.rs:191–219`) spawns an `adopt` per live run
   (2619–2634), and each adoption ends in `requeue_orphans_with`, a write to the roadmap from the
   window's process while a walk in another process writes the same file.

#### F4. Every writer of the roadmap overwrites the whole file, unlocked

`Store::write` is `std::fs::write(&self.path, text)` (`ra-roadmap/src/store.rs:654–657`) and
every mutation is load, change, write (`set_state_by`, 327–341). The walk, the window, the `rc`
command line, the MCP server and the look-back pass are separate processes doing this to one
file. Two of them inside the same few milliseconds lose one update without a trace; a reader that
arrives mid-write reads a truncated file. `run_parallel` reads the roadmap every three seconds
with `store.load().await?` (4670), so a truncated read ends the walk.

#### F5. One card can end or stall the walk

1. Dispatch runs in the walk's own loop: `worktree_add(&root, id).await?` (4720) and
   `catch_up_worktree` (4724). While they run, no run in flight is polled. The `?` returns out of
   `run_parallel`, which drops every future in flight, and so do `store.load().await?` (4670) and
   `store.read(id).await?` (4830). `run_parallel` writes no feed line on those paths; the error
   reaches only `var/walk.log` through `rc`'s stderr. Latent.
2. A full stop is decided by substring: an error whose text contains "unreachable", "limit",
   "week", "logged in" or "authentication" stops the walk (4877–4879), after `map_error`
   (8569–8584) has turned the provider's typed error into a sentence. Line 57 of the feed is one
   card's spawn failure stopping a walk at zero cards, because a failed spawn reads as "the
   provider is unreachable". Any error text that quotes "limit" or "week" does the same.
3. A parallel walk caps only idle rounds (`idle_rounds() >= RERUNS_A_WALK`, 4842–4869). A card
   whose check fails a different way each time, or that the reviewer rejects each time, is
   `NeedsRevision`, which `ready_cards` offers again at once (3106–3121). The per-cause caps
   (`CHECK_RETRIES = 2`, `REVIEW_RETRIES = 1`, 2794–2820) live in `run_all`, which no walk calls.

#### F6. Two walks in one file

`spawn_walk` always passes `--parallel` (2986–3007), so `run_all` (2651–2850) is reached only by
`rc run --all` (`apps/rc/src/main.rs:710`) and about 25 tests. It still holds behaviour
`run_parallel` never got: the move to a newer install (`handed_walk_lock` and `relaunch`,
2831–2848), `plan::read_back` (2688), `plan_next` (2712–2728), `plan::split::due` (2750),
`retro::landed` (2780) and the per-cause retry caps. Fixes land in one walk and not the other;
the tests exercise the one that does not run.

#### F7. The scheduler prefers, it does not plan

`spread` (4573–4589) with `shared_file` (4559–4567) holds a card back when a running card claims
one of its files, and the held-back card starts only when no free card is ready (4697–4708).
`claimed_files` (4592–4603) shells out to `git diff --name-only` once per running card on every
three-second tick. Three gaps:

1. A free seat can stay empty while a card that overlaps nothing waits further down (#237's own
   note says so).
2. Readiness is all or nothing on `depends_on` being `Accepted` (3107). Among ready cards the
   order is the roadmap's, whatever a card unblocks.
3. Overlap is by file name. Most orchestration cards name `agent.rs`, 8600 lines. A strict hold
   would have run the last walk's eight cards one after another; no hold let three of eight
   conflict. Neither is right, and the walk has no record of which merges actually conflicted to
   tune by.

#### F8. The end line says one thing whatever happened

The walk ends with `"ran {ran} card(s) · no card is ready"` (4797–4810) whether the cards left
are blocked for the person, waiting on a red main, waiting on a dependency that was sent back,
or parked by this walk. A card that waits on main (`wait_on_main`, 5634) is looked at again only
when a walk starts, and nothing starts one when main moves.

#### F9. Locks and markers

1. `walk_running` asks whether a walk runs by taking the walk lock and dropping it (2949–2951).
   `other_walks` (5099) calls it for every other project from every walk, and `spawn_walk` calls
   it just before it starts the child (2987). A walk that starts in the instant of somebody's
   question fails `take_walk_lock` and bails with "a walk is already running" (4620–4622); the
   window believes it started one. Latent. `hub::build_slots` probes build slots the same way
   (`slot_note`, `hub.rs:1093–1099`) and every waiting builder calls it on every 500 ms poll
   (`hub.rs:1054`), so waiters make each other miss free slots; that costs polls, not builds.
2. `var/walk.stop` is a bare file. A stop left behind stops the next walk after one card; the
   doc comment of `stop_walk` (3063) says so.
3. `var/lookback.running` holds the pid of the pass (`lookback.rs:57`) but `running` only asks
   whether the file exists (`lookback.rs:38–40`). A pass killed mid-plan leaves the marker, `due`
   is false from then on, and look-back never runs again without a word.
4. After `HOLD_WAIT` (15 minutes, 2072) a build runs with no slot at all (`hold`, 2291–2294, and
   `hold_build`, 2079–2107): the limit lifts exactly when the machine is fullest.
5. Build slots are shared fairly between projects (`try_build_slot`, `hub.rs:1045–1071`: a
   project that already builds leaves a free slot to one that does not) but within a project they
   are first come, first served on a 500 ms poll. A landing's proof, which frees a seat and
   unblocks dependents, queues level with a worker's exploratory build.

#### F10. Build targets

Every card builds into a target of its own, `wt-<id>`, from nothing (`shared_target`,
`claude_code.rs:1029–1046`). That cured the clobbering of 2026-11-12 and it is why a card's first
build compiles every dependency: six to fifteen minutes by the workers' own count, on a build
slot, while the session that asked for it sits past its foreground limit. Eight cards dispatched
in 44 s means eight cold builds queued on four to eight slots, each at a share of the CPU
(`BUILD_CPU / running`, 2396–2403). It is a likely part of the spread between the 14-minute and
the 34-minute first runs of the last walk; the walk does not time builds, so the share is not
known, and the roadmap adds the timer.

The clobbering itself points at the cure. Cargo names a workspace crate's build by its path
inside the workspace, not by where the worktree sits (#176 saw exactly that), so a target that
one card used is warm for the next card: the dependencies are built, and only the workspace
crates, whose files a fresh checkout stamps as new, compile again. What went wrong on 2026-11-12
was two cards in one target at the same time, not one after another.

And the cleanup misses. `clean_after` removes `root.join("target").join("wt-{id}")` (4374). The
build runs in `project_target(&root).join("wt-{card}")` (`claude_code.rs:1042`), which is
`<scratch>/target/<project>/wt-<id>` whenever a scratch root is set (`claude_code.rs:1072–1079`),
as it is on this machine (the look-backs of #220 and #223 name `D:/rhipper/target/<project>`).
Then `target.is_dir()` is false and the directory of every landed card stays on the scratch
drive unless something else sweeps it. `hub::build_target` (`hub.rs:991`) spells the path the
first way as well. I could not list the scratch drive from this session, so the size of what is
left there is unmeasured.

#### F11. The pool reserves seats nobody can sit in

With a priority order, a walk's seats are the pool minus the demand of every walking project
ahead of it, and that demand is the seats the other project holds plus its count of ready cards
(5052–5060), read from a digest written at the end of its last run. Nothing caps it at the other
project's own runners, though that project's walk caps itself there (5061–5064). A project ahead
with 4 runners and 30 ready cards counts as 34 against a pool of 12, and the projects behind it
get nothing while eight seats are free. The walk that is starved this way sleeps and says
nothing (4797–4802). Read from the code; the agent's own feed cannot show it, being first in
its order or without one.


Design
------

The design keeps what works: one task polls every run, so two runs never interleave a write of
the roadmap; a card works in its own worktree on its own branch; landing is a squash; the
conflict pass and the red-main parking of Phase AA stay as they are. It changes where state
lives, where the proof happens, and who decides what starts.

New code goes into a new module, `crates/ra-console/src/walk.rs` with `walk/ledger.rs`,
`walk/proof.rs`, `walk/landing.rs`, `walk/schedule.rs`, `walk/merge.rs` and `walk/report.rs`.
`agent.rs` only loses lines. That is itself part of the design: while every orchestration card
edits `agent.rs`, file-aware scheduling of the agent's own roadmap degenerates to one card at a
time.

#### D1. The run ledger: one small file per card in flight

`var/runs/<id>.json`, written by temp file and rename, removed by `clean_after`.

```rust
// crates/ra-console/src/walk/ledger.rs
pub struct Run {
    pub card: i64,
    pub stage: Stage,
    pub walk: u32,                // pid of the walk that owns the run
    pub child: Option<u32>,       // pid of the claude process while one runs
    pub session: Option<String>,  // its session id, from the stream's first event
    pub base: String,             // sha of the project's branch the card's branch last merged
    pub tip: String,              // sha of the card's branch at its last checkpoint
    pub proven: Option<String>,   // tree sha the card's check last passed on
    pub touched: Vec<String>,     // files the branch changes, refreshed at each checkpoint
    pub sent_back: SentBack,      // counts since the card last landed or a person reset it
    pub parked: Option<Parked>,
    pub at: u64,
}
pub enum Stage { Dispatching, Working, Checking, Checked, Reviewing, Queued, Landing, Landed }
pub struct SentBack { pub check: u8, pub review: u8, pub conflict: u8, pub landing: u8, pub idle: u8, pub interrupted: u8 }
pub enum Parked { MainRed { sha: String }, Person { why: String }, After { card: i64 } }
```

The provider cannot take a new field on `CompletionRequest` cheaply (every call site spells the
struct out), and it does not need one: `claude_code::project_of` (`claude_code.rs:1084`) already
derives the project and the card from the workspace. When the child is spawned, and again when
the stream's first event names the session, the provider writes `var/runs/<id>.child` (pid and
session, two lines). The ledger reads that file beside its own.

Children die with their walk. On Windows the walk creates one job object with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and the provider assigns every `claude` child to it
(`ra-tools/src/cpu.rs` already has a job for the CPU share; this is a second, nested one). The
operating system then kills the child and its `cargo` grandchildren when the walk's handle
closes, however the walk died. Elsewhere `kill_on_drop(true)` and a process group do the same.
With that, "the walk is dead and its worker still writes" cannot happen, and a worktree has one
writer by construction.

What a new walk does with the records it finds, in place of `requeue_orphans` and the loop at
4629–4636:

| Stage found | What the walk does | Worker turns spent |
|-------------|--------------------|--------------------|
| `Dispatching` | Back on the queue | 0 |
| `Working` | Checkpoint the worktree so the work is kept, count `interrupted`, resume `session` with three lines: the run was interrupted, the edits are in the tree, go on. No session: the rerun block with the changed files | the rest of the run, not all of it |
| `Checking` | Run the check (free when the tree is in the proof cache) | 0 |
| `Checked` | Review. Never land | 0 |
| `Reviewing` | Review again | 0 |
| `Queued`, `Landing` | Back into the landing queue | 0 |
| `Landed` | Finish the cleanup | 0 |

The window stops writing the roadmap for recovery. `pair::window_up` reads `var/runs/*.json`; a
run is live while its `walk` pid is alive. When the walk is dead and records remain, the window
starts a walk (`spawn_walk`), and that walk recovers them.

Send-back budgets live in `sent_back`: check 3, review 2, conflict 2, landing 2, idle 2,
interrupted unlimited. A card over a budget is `Parked::Person` with the cause in words, on the
first walk and on every later one, because the count is on disk. `settle_idle`, the idle rule at
4842–4869 and `run_all`'s `CHECK_RETRIES` and `REVIEW_RETRIES` become this one table.

Removes: `pair::orphaned`, `pair::open_runs`, `pair::adoptable` and the marker parsing behind
them; `agent::adopt`, `requeue_orphans` and `requeue_orphans_with`; the `parked` set; the
"implemented means landable" loop at 4629–4636.

#### D2. The proof cache: a tree that passed is not checked again

```rust
// crates/ra-console/src/walk/proof.rs
pub struct Proof { pub tree: String, pub command: String, pub toolchain: String, pub at: u64, pub secs: u32 }
pub fn key(tree: &str, command: &str, toolchain: &str) -> String;       // sha256 of the three
pub fn passed(root: &Path, tree: &str, command: &str) -> Option<Proof>;  // var/proofs/<key>.json
pub fn record(root: &Path, proof: &Proof);
```

`tree` is `git rev-parse HEAD^{tree}` after a checkpoint, so it names the content and not the
history: a card's branch that has merged main at M and a squash of that branch onto M have the
same tree. Only passes are recorded; a failure always runs again, so a flaky test can never be
cached red. `Cargo.lock` is in the tree and the toolchain is in the key, which covers what a
check reads outside the sources. Entries older than a week are swept at walk start.

Asked in four places: the run's check (a rerun that changed nothing costs no build), the
review's catch-up (main did not move: nothing to run), `prove_tree`, and the landing.

Two rules make the hits line up. First, a check that got as far as running tests records the
compile command (`compile_of`, used at 3794) as passed for that tree, whatever the tests did, so
the compile after a checkpoint of the same content is a lookup. Second, the review's catch-up
proves the landing's command set, `merged_suite` included, not only the card's own filter: a
failure then costs no review, and a pass is the landing's proof when main has not moved.

The red-main probe gets it for nothing: after a landing the project's tip is a proven tree, so
"is main green for this command" is a file lookup before it is a build.

#### D3. Landing: prove in the landing tree, then move the branch

A landing tree, `var/worktrees/land`, an agent-owned worktree on a detached HEAD, built the way
`base_worktree` (3168) builds the tree for the red-main probe; the two can be one directory with
one warm target, `<project target>/land`.

```rust
// crates/ra-console/src/walk/landing.rs
pub struct Batch { pub base: String, pub cards: Vec<i64>, pub tip: String, pub tree: String }
pub enum Refused { Conflict { card: i64, files: Vec<String> }, Check { card: i64, output: String }, Dirty { files: Vec<String> } }
pub async fn stage(root: &Path, cards: &[i64]) -> (Batch, Vec<Refused>);   // git only
pub async fn advance(root: &Path, batch: &Batch) -> Result<(), Refused>;    // the one shared write
```

The flow for the cards the reviewer has accepted since the last landing, in the order accepted:

1. `stage`: read the branch tip as `base`; reset the landing tree to it; for each card
   `git merge --squash card/<id>` and commit with the card's message. A card that conflicts
   leaves the batch (`Refused::Conflict`), goes to the conflict pass in its own worktree
   (`resolve_landing`, 3966) and queues again; the others go on. `cargo fmt --all` runs here,
   where it disturbs nobody.
2. Proof: `tree` of the result; `proof::passed` for each check command; run only what is missing,
   in the landing tree, on one build slot. With one card in the batch and main unmoved since its
   review, the tree is the card's reviewed tree and nothing runs at all.
3. A failed proof with more than one card: stage and prove each card alone, in order. With one
   card: `Refused::Check`, sent back exactly as a `LandCheck` is today.
4. `advance`, under `guard_landing` (2006) for this step only: when the branch tip is still
   `base`, `git merge --ff-only <tip>` in the project's checkout (or `git fetch . <tip>:<branch>`
   when the person has another branch checked out). Git itself refuses when a file the person
   has open would be overwritten (`Refused::Dirty`); the walk says which files and tries again on
   a later tick. The person's files are never stashed. When the tip moved (the person
   committed), stage again on the new base.
5. For each card: `Accepted`, ledger `Landed`, `clean_after`, the look-back count,
   `retro::landed`.

The project's branch only ever points at a commit whose tree has passed. So a new worktree
always branches from a proven commit, a catch-up always merges one, and a crash at any step
leaves the branch where it was with the ledger saying `Landing`.

`cargo fmt --all` also moves into `checkpoint_and_prove` (4008), in the card's worktree, so the
tree the reviewer reads is the tree that lands and the cache hits.

In the walk, the landing is one more future in `flight`, started when the queue is not empty and
no landing is running. Runs no longer take a lock to land; they push their card on the queue and
give their seat back. Nothing a card does in its own worktree needs a project-wide lock (git
locks each ref itself), so `git_lock_owned` goes.

What the last walk would have looked like, as an estimate: eight cards accepted over about
thirty minutes with a three to four minute proof means batches of two or three, so three or four
moves of main instead of eight and three or four proofs instead of eight. The stale-main
send-backs (four in that walk) fall with the number of moves.

Removes: the squash into the root (4111), the stash of the person's files (4129), fmt in the
root (4158–4173), the check after the commit and `git reset --merge HEAD~1` (4190–4213),
`git_lock_owned` (4645) and its three uses, the second check in `land`.

#### D4. A walk no card can kill

1. `run_card(id, seat)` is the whole life of a run as one future: `Dispatching` (worktree,
   catch-up), `Working`, `Checking`, `Reviewing`, then onto the landing queue. A failure in
   `Dispatching` parks that card with the git error in words; the loop never awaits git itself.
2. A full stop is a type. `ProviderError` (`ra-providers/src/lib.rs:199–222`) has `Auth` and
   `Unavailable` already; it gains `UsageOut` for the plan's limit, which today travels as text.
   The provider classes a failed spawn by its kind: `claude` not found is `Unavailable`, any
   other spawn error is `Other`, the card's own. `map_error` wraps the three stopping kinds in a
   `Stop` error the walk finds with `downcast_ref`, and the walk stops on that alone. The
   substring list at 4877–4879 goes.
3. The roadmap reads at 4670 and 4830 keep the last good copy: a failed read is said once on the
   feed and tried again on the next tick.
4. `Store::write` writes `rhipper.md.tmp-<pid>` and renames it over the file (retrying a few
   times, since a Windows reader holding the file open can refuse the rename). Every mutation
   runs under an operating-system lock on `rhipper.md.lock`, taken in a new `Store::mutate`, so
   load, change and write are one step across processes.
5. Every way out of `run_parallel` writes an end line to the feed, the error ones included: the
   feed is what the person and the window read, and `var/walk.log` is not.

#### D5. The planner

```rust
// crates/ra-console/src/walk/schedule.rs
pub struct Card { pub id: i64, pub deps: Vec<i64>, pub files: Vec<String>, pub unblocks: u32, pub warm: bool }
pub struct Running { pub id: i64, pub files: Vec<String> }
pub enum Wait {
    Dependency(i64), Overlap { with: i64, file: String }, Seats, BuildSlots,
    MainRed(String), Person(String), Budget(&'static str), LandingQueue,
}
pub struct Plan { pub start: Vec<i64>, pub waits: Vec<(i64, Wait)> }
pub fn plan(cards: &[Card], running: &[Running], landed: &[i64], free: usize, width: impl Fn(&str) -> usize) -> Plan;
```

A pure function: no git, no clock, no store, so its tests are tables.

1. Order: cards sent back whose session is still warm first (the rerun is cheapest while the
   provider's cache holds), then by `unblocks` (how many cards not yet landed depend on this one,
   directly or through others), then the roadmap's order.
2. Fill: walk the whole order and start every card that fits, up to `free`. A card that does not
   fit is skipped with its reason, never stood behind.
3. Overlap: a file may be claimed by at most `width(file)` cards in flight. `width` is 1 for a
   file under 1500 lines and 1 more for each further 3000 (so 3 for `agent.rs` today). This is a
   rule of thumb; D6's merge log is what corrects it.
4. `Running.files` comes from the ledger's `touched`, refreshed at checkpoints. `claimed_files`
   and its `git diff` per card per tick go.
5. Every card not started has exactly one `Wait`. That list is the idle report (D7).

Stacking a dependent on a dependency that is queued to land, so a chain does not wait for each
proof, is deliberately last in the roadmap and gated on the report: build it only if
`idle_slot_secs` shows `Dependency` waits on `Queued` cards as a real share. With D3 the wait it
would save is seconds to a few minutes per link.

#### D6. Conflicts, cheapest first

Phase AA gave the walk a conflict pass of 15 turns. Three tiers go in front of it.

1. Fewer conflicts to begin with: D3 moves main three or four times where it moved eight.
2. Recorded resolutions. `rerere.enabled` and `rerere.autoupdate` are set on the project's
   repository at walk start. The record lives in the common git directory, so every worktree
   shares it: a conflict the pass resolved at a card's catch-up resolves by itself when the same
   hunks meet again at the next catch-up or in the landing tree. #211 and #216 met the same
   conflict three times.
3. Both-added hunks. Tests live in `tests.rs` beside the code and each card appends its own, so
   two cards conflict at the end of the same file with an empty base. `walk::merge::settle_added`
   reads the conflicted file in diff3 form and, for every hunk whose base side is empty, keeps
   ours then theirs; any other hunk is left for the pass. `prove_tree` (3787) then compiles the
   result as it does for any merge.
4. The conflict pass, as it is.
5. Back to the worker: only when the pass gives up.

Every merge writes one line to `var/merges.log`: card, file, and the tier that settled it
(`clean`, `recorded`, `added`, `pass`, `worker`).

One kind of conflict git never reports: main adds a use of something the card changed. #210
met it twice and #213 once ("The merge brought in a `Handoff` literal without the card's new
`failures` field, which cost a build"), and #235 found "two halves of the merge did the same job
with different signatures". No merge tier can settle these; what bounds their cost is where
they are found. Today that is the worker's next build, or the check after the branch has
moved. Here it is `prove_tree` straight after the catch-up, whose compile error goes to the
conflict pass as its brief, and the landing tree's proof before the branch moves. Their number
falls with the number of moves of main, as the textual conflicts' does.

#### D7. The walk report and the wake condition

```rust
// crates/ra-console/src/walk/report.rs
pub struct Report {
    pub started: u64, pub ended: u64, pub install: String,
    pub dispatched: u32, pub runs: u32, pub landed: u32, pub max_runs_one_card: u32,
    pub sent_back: SentBack,                       // summed over the walk
    pub resumed_at: BTreeMap<String, u32>,         // stage -> runs resumed there
    pub landing_wait_secs: Vec<u32>, pub landing_refused: u32,
    pub checks_run: u32, pub checks_cached: u32,
    pub first_build_secs: Vec<u32>,                // one per card dispatched: its first build
    pub slot_wait_secs: BTreeMap<String, u64>,     // rank -> seconds builds waited for a slot
    pub merges: BTreeMap<String, u32>,             // tier -> count
    pub idle_slot_secs: BTreeMap<String, u64>,     // wait reason -> free-seat seconds
    pub left: Vec<(i64, String)>,                  // every card not landed, with its reason
    pub wake: Vec<Wake>,
}
pub enum Wake { MainMoves { from: String }, Never }
```

Each tick, every free seat adds the tick's length to the reason of the first waiting card in the
plan. At the end the walk appends the report as one line to `var/walks.log`, writes
`var/walk.idle.json`, and says on the feed, for example:

```
walk ended · 8 landed of 9 in 17 runs (1.9 a card) · landing wait 12 s median · checks 11 run, 9 cached
left: #215 for a person (sent back 3 times by its check) · #240 waits on #239 · #221 waits: main fails hub::tests::x at 787be7f
free seats: 14 min behind dependencies, 6 min on crates/ra-console/src/agent.rs, 2 min on build slots
```

The window already polls for a walk request (`apps/rc-gui/src/main.rs:2132`). On the same tick
it reads `var/walk.idle.json` and starts a walk when a `MainMoves` wake is met, that is, when the
branch tip is no longer `from`. A card parked on a red main is picked up when the person's fix
lands, with nobody asking.

Removes: the fixed closing words at 4797–4810.

#### D8. One walk

`run_parallel` takes over, in this order: `retro::landed` and the look-back count (in D3's step
5), the move to a newer install, then `plan::read_back`, `plan_next` and `plan::split::due`.

The move to a newer install: when `hub::moved_to` (`hub.rs:810`) names one, the walk starts no
card that has not run before, lets the runs in flight reach a stage boundary, and relaunches
(`relaunch`, 3048). The ledger carries every run across, so the new program resumes them by D1's
table. That is the difference from `run_all`'s version, which could only hand over between
cards.

`rc run --all` then calls `run_parallel` at width 1 and `run_all`, `walk_runner` (2864–2880) and
`handed_walk_lock` (3026) are deleted with the tests moved onto the one walk.

#### D9. Locks, markers, slots, targets

1. A question never takes a lock. `walk_running` and `hub::build_slots` read the holder's
   `.who` file (`try_guard_as` writes it, `hub.rs:870`; `held_by` reads it, 881) and ask
   `pid_alive`. `take_walk_lock` also retries for half a second before it gives up.
2. `var/walk.stop` carries the pid of the walk it is for; a walk deletes a stop that is not its
   own.
3. `lookback::running` is true only while the pid in the marker is alive.
4. A build that has waited `HOLD_WAIT` goes on waiting and says so each five minutes; it never
   runs slotless. With D2 there are fewer builds to wait behind.
5. Build slots have three ranks: a landing's proof, then a worker's own build, then the
   orchestrator's checks. The landing first because it frees dependents and is short. The
   worker before the check because a worker that waits holds a seat and a session whose prompt
   cache goes cold while it waits (the provider keeps it five minutes by default), while a check
   that waits holds only time. The wish a waiter leaves today is one file per project holding a
   pid (`hub.rs:1068–1069`); it becomes one file per waiter,
   `build-want-<project>-<rank>-<pid>`, and a taker stands back while a live waiter of a better
   rank exists. The fairness between projects stays as it is.
6. `rc hold` says where it stands: on entry and each minute after, one line with how many builds
   are ahead and whose (`hub::build_slots` already knows the holders). #213's twelve silent
   minutes become a line the worker can act on.
7. `Pool::take` counts a project ahead in the order as `min(held + ready, runners)`, with
   `allocation::runners` (`allocation.rs:42`), and a walk that waits for a seat says so once on
   the feed, naming who holds them.

#### D10. Leased build targets

A project keeps a pool of targets, `<project target>/wt-pool-<n>`. A card takes one when its
worktree is made and keeps it until it lands; the next card to take it finds every dependency
built.

```rust
// crates/ra-providers/src/claude_code.rs, beside shared_target
pub fn lease_target(root: &Path, card: i64) -> PathBuf;          // the card's lease, taking a free target or making one
pub fn leased_target(root: &Path, card: i64) -> Option<PathBuf>;  // read only
pub fn release_target(root: &Path, card: i64);
```

The lease is a file, `wt-pool-<n>/.lease`, holding the card's id, made with `create_new` so two
walks cannot both take one target. A target is free when it has no `.lease`, or when the card
its lease names has no worktree any more (a lease a crash left behind frees itself). It needs no
ledger, so it can be built before D1. `worktree_add` (3138) calls `lease_target`;
`shared_target` answers `leased_target` and falls back to `wt-<card>` for a worktree made before
this; `clean_after` calls `release_target` and deletes nothing; `hub::build_target` asks
`shared_target` too, so a slot's note names the directory the build is in. Every other reader
already goes through `shared_target` (the worker's environment at `claude_code.rs:202`, the
tools' shell at `ra-tools/src/commands.rs:92`, the checks at 3566, 3800, 5431 and 6886), so
nothing else changes. The pool grows only when every target is leased, and a walk that starts
deletes the free targets beyond the project's runners, so the pool holds about as many targets
as cards run at once, where today there is one directory per card ever run.

One card per target at a time is the rule that 2026-11-12 broke, and the lease file is what
keeps it. The other rule is the order: the lease is taken before `git worktree add`. Cargo
calls a workspace crate fresh when no source file is newer than its last build, so a checkout
written before the previous card's last build would be judged by that card's binaries. Leased
first and checked out second, every source file is newer than anything in the target, and cargo
rebuilds each workspace crate once and no dependency.

Expected: a first build of one and a half to three minutes (the workspace crates and the test
binary) where the workers report six to fifteen. That also brings most first builds back toward
the foreground limit that six cards tripped over.


What goes away
--------------

| Code | Where | Replaced by |
|------|-------|-------------|
| `run_all`, `walk_runner`, `handed_walk_lock` | 2651–2850, 2864–2880, 3026 | `run_parallel` at width 1 (D8) |
| `requeue_orphans`, `requeue_orphans_with`, `adopt` | 2579–2634 | recovery by stage (D1) |
| `pair::orphaned`, `open_runs`, `adoptable`, the start/end marker parsing | `pair.rs:224–284` | `ledger::all` |
| The implemented-cards loop at walk start | 4629–4636 | `Checked` → review, `Queued` → queue |
| `git_lock_owned` | 4645, 4761, 4766 | nothing; `guard_landing` around `advance` |
| Squash, stash, fmt and check in the root; `git reset --merge HEAD~1` | 4111–4213 | `landing::stage`, proof, `landing::advance` |
| The second and third run of a check on an unchanged tree | 3515–3608, 4190–4213 | `proof::passed` |
| `parked`, the idle-only cap, `CHECK_RETRIES`, `REVIEW_RETRIES` | 4651, 4842–4869, 2794–2820 | `SentBack` budgets |
| The substring stop list | 4877–4879 | typed stops |
| `claimed_files` and its `git diff` per tick; `spread`'s hold-back branch | 4592–4603, 4697–4708 | `schedule::plan` over `touched` |
| The closing words "no card is ready" | 4797–4810 | the report's `left` |
| Lock probes that take the lock | 2949–2951, `hub.rs:1078` | `.who` and `pid_alive` |
| The slotless build after 15 minutes | 2291–2294, 2079–2107 | ranked waiting |
| A target per card, `wt-<id>`, and its deletion at landing | `claude_code.rs:1042`, 4370–4380 | the lease pool (D10) |
| The path `<root>/target/wt-<id>` spelled by hand | 4374, `hub.rs:991` | `shared_target` |


How each change is measured
---------------------------

Every number below is a field of `var/walks.log` (D7), so the comparison is one walk against the
next on the same project, and the three projects get the same report.

| Change | Before (from the feed) | After, what to expect | Field |
|--------|------------------------|-----------------------|-------|
| D3 landing | 255 s average from accepted to landed, serialized; 8 moves of main per 8 cards | under 30 s median when main has not moved; one proof per batch | `landing_wait_secs`, `landed` against moves |
| D2 proof cache | up to 3 runs of a check per pass | 1 per pass plus 1 per batch | `checks_run`, `checks_cached` |
| D1 ledger | #201: 220 s and a session lost to a restart; runelite 23 restart reruns | no rerun from a restart at `Checked` or later; `Working` resumes its session | `resumed_at`, `sent_back.interrupted` |
| Budgets | 8 runs for one card | no card over 4 runs without a person | `max_runs_one_card` |
| D4 | walks that end with no line | 0 | walks in `walks.log` without `ended` |
| D5 planner | not known: the walk does not record free seats | the first walk gives the baseline | `idle_slot_secs` |
| D6 conflicts | 20 sent back to a worker | most settled at `recorded` or `added` | `merges` |
| D7 report | "no card is ready" | every card left has a reason; a parked card restarts when main moves | `left`, `wake` |
| D8 | a landed fix reaches the walk at the next walk | at the next stage boundary | `install` changing inside one walk |
| D9 slots | a build that waits says one line, then nothing (#213: 12 minutes); a landing's proof queues behind worker builds | a line a minute; a landing's proof takes the next free slot | `slot_wait_secs` by rank |
| D9 pool | not known for the two projects behind the agent in the order | no free seat while a card of any project waits for one | `idle_slot_secs` under `Seats` |
| D10 targets | first build of a card 6 to 15 minutes (look-backs of #214, #242, #243) | 1.5 to 3 minutes; fewer builds past the foreground limit | `first_build_secs` |

The headline number is `runs / landed`. On the agent project it is about 1.9 in the last walk and
3.2 over the feed (147 runs, 69% reruns). The orchestration share of that, by the reading above,
is between a fifth and a quarter of all runs; these changes aim to take that share to near zero,
which on their own would put the agent project near 1.5 runs a card before Phase AA's effect on
the worker's own reruns is counted.


Part 2: Roadmap
===============

Everything from the next line to the end of the file is the outline. New code is in new files
under `crates/ra-console/src/walk/`; each card's tests sit in the `tests.rs` beside the file it
changes. The order is the build order: the build targets first, because they need nothing else
and every later card's worker builds faster for them; then state, the landing, the walk's own
survival, the planner, conflicts, and the removal of the second walk.

## The walk that never wastes a run

### Build targets that start warm

- The cleanup and the hub ask the provider where a worktree builds {c1}
  objective: `clean_after` removes `<root>/target/wt-<id>` (agent.rs:4374) while a worktree builds in `<scratch>/target/<project>/wt-<id>` (claude_code.rs:1042, 1072-1079), so with a scratch root every landed card's target stays on the scratch drive; `hub::build_target` spells the path the same wrong way (hub.rs:991), so a slot's note names a directory no build is in. Ask `claude_code::shared_target` in both.
  acceptance: `cargo test -p ra-console -q target_of_the_worktree` passes and runs at least `the_cleanup_removes_the_target_under_the_scratch_root` and `the_hub_names_the_target_the_build_uses`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/hub.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/hub/tests.rs

- A worktree's target can be a lease from the project's pool {c3}
  objective: Each card builds every dependency again in a target of its own: about 6 minutes for #214, 6.5 for #242, 15 for #243 by their look-backs, and the build passes the worker's 120 s foreground limit (#223, #228, #231, #232, #237). Add `lease_target`, `leased_target` and `release_target` beside `shared_target` (claude_code.rs:1029) as in docs/fable/orchestration.md D10: targets `wt-pool-<n>` under `project_target`, a `.lease` file made with `create_new` holding the card, a lease whose card has no worktree counted as free; `shared_target` answers the lease when there is one and `wt-<card>` when not. No caller yet.
  acceptance: `cargo test -p ra-providers -q lease` passes and runs at least `a_released_target_is_the_next_cards_lease`, `two_cards_asking_at_once_get_two_targets`, `a_lease_whose_worktree_is_gone_is_free` and `a_worktree_without_a_lease_builds_where_it_did`
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs

- The walk leases a target before it checks a card out and returns it when the card lands {c2}
  objective: With the lease functions in place, `worktree_add` (agent.rs:3138-3160) takes the card's lease before `git worktree add`, so every source file is stamped later than anything in the target and cargo rebuilds the workspace crates and nothing else; `clean_after` (agent.rs:4357-4393) releases the lease and no longer deletes a target. At walk start remove the `wt-<id>` targets whose card has no worktree and the free pool targets beyond `allocation::runners`.
  acceptance: `cargo test -p ra-console -q leased_target` passes and runs at least `a_new_worktree_holds_a_lease_older_than_its_files`, `a_landed_card_returns_its_target_whole` and `a_walk_sweeps_the_targets_of_cards_that_are_gone`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

### State that survives a crash

- A card whose check passed but was never reviewed is reviewed, not landed {c2}
  objective: `run_task` sets `Implemented` when the check passes (agent.rs:5652) and the review leaves it there on an accept (agent.rs:1452), so a walk killed between the two leaves a card the next walk lands unreviewed (agent.rs:4629-4636, 4041-4045). The walk-start loop and `land` must ask for an accepting verdict on the card's last handoff and send a card without one to `review_after_run`.
  acceptance: `cargo test -p ra-console -q never_reviewed` passes and runs at least `an_implemented_card_that_was_never_reviewed_is_reviewed_before_it_lands` and `land_refuses_a_card_with_no_accepting_verdict`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

- Roadmap writes are a temp file and a rename under a cross-process lock {c3}
  objective: `Store::write` is `std::fs::write` (store.rs:654-657) and every mutation is load, change, write with no lock (store.rs:327-341) while the walk, the window, `rc`, the MCP server and look-back all write the file; a concurrent write loses an update and a mid-write read is truncated, which ends a walk at agent.rs:4670. Add `Store::mutate` holding an OS lock on `rhipper.md.lock` around load, change and write, and make `write` rename a temp file over the roadmap with a short retry.
  acceptance: `cargo test -p ra-roadmap -q store` passes and runs at least `two_stores_setting_different_cards_at_once_lose_neither` and `a_reader_never_sees_a_half_written_roadmap`
  files: crates/ra-roadmap/src/store.rs crates/ra-roadmap/src/store/tests.rs

- The run ledger: one record per card in flight in var/runs {c3}
  objective: Run state is rebuilt from text markers in `var/activity.log` (pair.rs:234-284) and from memory (`parked`, agent.rs:4651). Add `walk::ledger` with `Run`, `Stage`, `SentBack` and `Parked` as in docs/fable/orchestration.md D1, `put` (temp file and rename), `get`, `all` and `remove`, reading `var/runs/<id>.child` for the child pid and session when it exists. No caller yet.
  acceptance: `cargo test -p ra-console -q walk::ledger` passes and runs at least `a_record_put_is_the_record_read_back`, `a_put_cut_short_leaves_the_last_whole_record` and `the_child_file_fills_the_pid_and_the_session`
  files: crates/ra-console/src/walk.rs crates/ra-console/src/walk/ledger.rs crates/ra-console/src/walk/ledger/tests.rs crates/ra-console/src/lib.rs

- The provider records the claude child's pid and session as the run starts {c3}
  objective: The pid on the start marker is the walk's (agent.rs:1053, 1062) and the session id reaches the roadmap only when the run returns (agent.rs:5236-5240), so a killed run has no child to find and no session to resume. In `claude_code::run` write `var/runs/<card>.child` (pid, then the session id when the stream's first event names it) for a workspace `project_of` resolves to a card (claude_code.rs:1084), and remove it when the child exits.
  acceptance: `cargo test -p ra-providers -q child_file` passes and runs at least `a_run_in_a_card_worktree_writes_its_pid_and_session_beside_the_ledger` and `a_run_outside_a_worktree_writes_no_child_file`
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs

- Worker processes die with the walk that started them {c3}
  objective: `claude` is spawned with no `kill_on_drop` and no job object (claude_code.rs:351-373), so a dead walk leaves workers writing in worktrees the next walk hands to a second writer (agent.rs:2579-2612). Add `ra_tools::cpu::own_children()` creating one job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` on Windows and assign each spawned child to it; set `kill_on_drop(true)` everywhere.
  acceptance: `cargo test -p ra-tools -q dies_with_its_owner` passes and runs `a_child_in_the_owning_job_is_gone_when_the_job_closes`
  files: crates/ra-tools/src/cpu.rs crates/ra-tools/src/tests.rs crates/ra-providers/src/claude_code.rs

- A run writes its stage to the ledger at every boundary {c3}
  objective: With the ledger in place, `run_picked` and `run_task` record `Working`, `Checking`, `Checked`, `Reviewing` and `Queued` as they pass them (agent.rs:1023-1146, 5127, 5450, 5652, 1090), `land` records `Landing` and `Landed`, and `clean_after` removes the record, so a restart can tell a run that needs a worker from one that needs only a review or a landing.
  acceptance: `cargo test -p ra-console -q stage_is_on_disk` passes and runs at least `a_scripted_run_leaves_checked_then_reviewing_then_queued` and `a_landed_card_leaves_no_record`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/ledger.rs crates/ra-console/src/commands/tests.rs

- Recovery is a plan read off the ledger {c2}
  objective: `requeue_orphans` sends every orphan back to `Defined` for a full rerun (agent.rs:2593) whatever it had finished; #201 lost 220 s and a session to one restart. Add the pure `ledger::recover(records, alive) -> Vec<(i64, Resume)>` with `Resume::{Queue, Session(String), Rerun, Check, Review, Land, Cleanup}` following the stage table in docs/fable/orchestration.md D1.
  acceptance: `cargo test -p ra-console -q walk::ledger::tests::recover` passes and runs at least `a_checked_run_resumes_at_review`, `a_working_run_with_a_session_resumes_the_session` and `a_run_of_a_live_walk_is_left_alone`
  files: crates/ra-console/src/walk/ledger.rs crates/ra-console/src/walk/ledger/tests.rs

- A new walk resumes each run at the stage it died in {c4}
  objective: Replace `requeue_orphans` and the implemented-cards loop at the head of `run_parallel` (agent.rs:4626-4636) with `ledger::recover`: checkpoint an interrupted worktree, resume the recorded session with a three-line prompt, send `Checked` and `Reviewing` to `review_after_run` and `Queued` and `Landing` to the landing, counting `interrupted` on the record. Runelite's 23 restart reruns and the agent's 21 to 31 window restarts a day are the cost today.
  acceptance: `cargo test -p ra-console -q resumes_at_its_stage` passes and runs at least `a_walk_killed_after_the_check_spends_no_worker_turn_on_restart` and `a_walk_killed_mid_run_resumes_the_session_it_recorded`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/ledger.rs crates/ra-console/src/commands/tests.rs

- The window reads live runs from the ledger and no longer re-queues {c3}
  objective: `pair::window_up` spawns an `adopt` per run that ends in a roadmap write from the window's process (pair.rs:191-219, agent.rs:2619-2634) beside a walk writing the same file. Make `window_up` list `ledger::all` with a live `walk` pid, start a walk when records remain and none runs, and delete `adopt`, `requeue_orphans`, `requeue_orphans_with`, `pair::orphaned`, `pair::open_runs` and `pair::adoptable`.
  acceptance: `cargo test -p ra-console -q pair` passes and runs at least `a_window_coming_up_shows_the_ledgers_live_runs` and `a_window_coming_up_writes_nothing_to_the_roadmap`; `cargo build -p rc-gui -q` prints nothing
  files: crates/ra-console/src/pair.rs crates/ra-console/src/pair/tests.rs crates/ra-console/src/agent.rs apps/rc-gui/src/main.rs

- Send-back budgets by cause, counted in the ledger {c3}
  objective: A parallel walk caps only idle rounds (agent.rs:4842-4869); a card that fails its check differently each time or is rejected each time is ready again at once (agent.rs:3106-3121), and #223 took 8 runs. Count each send-back in `SentBack` by cause, park a card over its budget (check 3, review 2, conflict 2, landing 2, idle 2) as `Parked::Person` with the cause in words, and drop the in-memory `parked` set.
  acceptance: `cargo test -p ra-console -q budget` passes and runs at least `a_card_rejected_twice_by_its_reviewer_waits_for_a_person`, `a_fourth_failed_check_is_not_run` and `the_count_survives_a_new_walk`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/ledger.rs crates/ra-console/src/commands/tests.rs

### Landing that proves before it moves

- The proof cache: a tree that passed a command is not checked again {c3}
  objective: One pass runs the same check in `run_task` (agent.rs:5450), in `catch_up_for_review` (3515-3608) and in `land` (4190-4213), each on a build slot, and six checks waited on "8 of 8" slots in the last walk. Add `walk::proof` with `key`, `passed` and `record` over `var/proofs/<key>.json`, keyed by tree sha, command and toolchain, passes only, swept after a week. No caller yet.
  acceptance: `cargo test -p ra-console -q walk::proof` passes and runs at least `a_recorded_pass_is_found_by_tree_and_command`, `a_failure_is_never_recorded` and `another_toolchain_is_another_key`
  files: crates/ra-console/src/walk/proof.rs crates/ra-console/src/walk/proof/tests.rs crates/ra-console/src/walk.rs

- The run's check and the review's catch-up ask the proof cache first {c3}
  objective: A rerun that changed no file, and a review catch-up when main has not moved, run the full check on a tree that already passed. Checkpoint before the check, read `HEAD^{tree}`, skip the commands `proof::passed` knows and record the ones that pass, in `run_task` and `catch_up_for_review`; say `check passed (proven at <time>)` on the check line.
  acceptance: `cargo test -p ra-console -q proven_tree` passes and runs at least `a_second_check_of_an_unchanged_tree_runs_no_command` and `a_tree_main_changed_is_checked_again`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/proof.rs crates/ra-console/src/commands/tests.rs

- The review's catch-up proves the commands the landing will ask for {c2}
  objective: The catch-up runs the card's own commands on the merged tree (agent.rs:3560-3586) and the landing adds `cargo test -p <crate> -q` for each changed crate (`merged_suite`, agent.rs:3614-3639), so the whole suite first runs after the reviewer was paid for: #182, #207, #179 and #221 failed there. Run `merged_suite`'s set in the catch-up and record each pass, and let a check that reached its tests record the compile command (`compile_of`, agent.rs:3794) as passed for that tree, so `prove_tree` on the same content is a lookup.
  acceptance: `cargo test -p ra-console -q catch_up_proves` passes and runs at least `the_catch_up_runs_the_changed_crates_suite_before_the_review` and `a_tree_whose_tests_ran_is_not_compiled_again_to_prove_it`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

- The checkpoint formats the tree, so the tree reviewed is the tree landed {c2}
  objective: `cargo fmt --all` runs in the project's root during the landing (agent.rs:4158-4173), which makes the landed tree differ from the reviewed one and defeats the proof cache. Run it in the card's worktree inside `checkpoint_and_prove` (agent.rs:4008) before the commit.
  acceptance: `cargo test -p ra-console -q checkpoint_formats` passes and runs `a_checkpoint_commits_the_formatted_tree`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- Staging a landing: squash the accepted cards into the landing tree {c3}
  objective: `land` squashes into the person's checkout (agent.rs:4111) and stashes their files (4129). Add `walk::landing::stage(root, cards)` building `var/worktrees/land` the way `base_worktree` does (agent.rs:3168), resetting it to the branch tip, squashing and committing each card in order, dropping a card that conflicts as `Refused::Conflict`, and answering a `Batch` with base, tip and tree. Git only; the project's checkout is not touched.
  acceptance: `cargo test -p ra-console -q walk::landing::tests::stage` passes and runs at least `two_cards_stage_as_two_commits_on_the_tip`, `a_conflicting_card_leaves_the_batch_and_the_other_stays` and `staging_leaves_the_projects_checkout_as_it_was`
  files: crates/ra-console/src/walk/landing.rs crates/ra-console/src/walk/landing/tests.rs crates/ra-console/src/walk.rs

- Advancing a landing: the branch moves by fast-forward only {c3}
  objective: A failed landing check is undone with `git reset --merge HEAD~1` (agent.rs:4203) after HEAD has been an unproven commit for minutes that new worktrees branch from (3138-3160). Add `landing::advance(root, batch)` under `guard_landing`: fast-forward the project's branch to the batch tip when the tip is still the batch base, answer `Refused::Dirty` naming the files when git will not overwrite the person's edits, and report a moved base so the caller stages again.
  acceptance: `cargo test -p ra-console -q walk::landing::tests::advance` passes and runs at least `the_branch_fast_forwards_to_the_staged_tip`, `a_dirty_file_the_landing_would_overwrite_refuses_it_and_is_kept` and `a_base_that_moved_is_reported_not_overwritten`
  files: crates/ra-console/src/walk/landing.rs crates/ra-console/src/walk/landing/tests.rs

- land stages, proves, then advances; the undo path is deleted {c4}
  objective: With `stage`, `proof` and `advance` in place, make `land` call them in that order, running in the landing tree only the commands `proof::passed` does not know, and delete the squash into the root, the stash, the fmt in the root, the check after the commit and the reset (agent.rs:4111-4213). #182, #207, #179 and #221 each put an unproven commit on main before their check failed.
  acceptance: `cargo test -p ra-console -q land` passes and runs at least `a_failed_proof_never_moves_the_branch`, `a_card_reviewed_on_the_current_tip_lands_without_a_check` and `a_crash_after_staging_leaves_the_branch_where_it_was`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/landing.rs crates/ra-console/src/commands/tests.rs

- The walk lands through a queue, several cards under one proof {c4}
  objective: Each run lands under the walk's one git mutex (agent.rs:4761-4764): 2043 s of waiting over 8 landings in the last walk and 8 moves of main in thirty minutes. Runs push their card on a queue and free their seat; one landing future in `flight` stages everything queued, proves once, falls back to one card at a time when the batch fails, and advances. Delete `git_lock_owned`.
  acceptance: `cargo test -p ra-console -q landing_queue` passes and runs at least `two_cards_accepted_together_land_under_one_check`, `a_batch_that_fails_lands_its_good_card_alone` and `a_checkpoint_does_not_wait_for_a_landing`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/landing.rs crates/ra-console/src/commands/tests.rs

### A walk no card can end

- Dispatch runs inside the run's future and a git failure parks one card {c3}
  objective: `worktree_add(&root, id).await?` and `catch_up_worktree` run in the walk's loop (agent.rs:4720-4724): no run is polled meanwhile, and the `?` ends the walk and drops every run in flight. Move both into the run's future as the `Dispatching` stage; a failure parks that card with the error in words and the walk goes on.
  acceptance: `cargo test -p ra-console -q dispatch` passes and runs at least `a_card_whose_worktree_cannot_be_made_is_parked_and_the_others_run` and `the_walk_ends_with_a_line_when_a_dispatch_fails`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

- Only a typed provider stop ends a walk {c2}
  objective: A walk stops when a card's error text contains "unreachable", "limit", "week", "logged in" or "authentication" (agent.rs:4877-4879), read after `map_error` has turned the typed `ProviderError` into words (agent.rs:8569-8584); feed line 57 is a walk stopped at zero cards by one card's spawn error. Add `ProviderError::UsageOut` beside `Auth` and `Unavailable` (lib.rs:199-222) and raise it where the provider reads a usage-limit result; make a spawn failure `Unavailable` only when the program is not found and `Other` when it is one run's own; have `map_error` wrap the three stopping kinds in a `Stop` error; stop the walk on `downcast_ref::<Stop>()` alone and delete the substring list.
  acceptance: `cargo test -p ra-console -q typed_stop` passes and runs at least `a_card_error_that_says_limit_does_not_stop_the_walk`, `one_runs_spawn_failure_parks_that_card_only` and `usage_running_out_stops_the_walk_after_the_runs_in_flight`
  files: crates/ra-console/src/agent.rs crates/ra-providers/src/lib.rs crates/ra-providers/src/claude_code.rs crates/ra-console/src/commands/tests.rs

- A roadmap read that fails is tried again, and every way out of the walk says so on the feed {c2}
  objective: `store.load().await?` on each tick (agent.rs:4670) ends the walk on one bad read, and the error returns write no feed line; `var/walk.log` is empty after every walk that died. Keep the last good roadmap for a tick, say the failed read once, and write an end line with the error on every return from `run_parallel`.
  acceptance: `cargo test -p ra-console -q walk_says_why` passes and runs at least `one_unreadable_roadmap_does_not_end_the_walk` and `a_walk_that_fails_leaves_its_reason_on_the_feed`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

- A question about a lock never takes the lock {c2}
  objective: `walk_running` takes and drops the walk lock (agent.rs:2949-2951) and is called for every other project by every walk (agent.rs:5099), so a walk starting in that instant bails at agent.rs:4621; `hub::build_slots` probes slots the same way (hub.rs:1078). Answer both from the holder's `.who` file and `pid_alive` (hub.rs:870-881), and let `take_walk_lock` retry for half a second.
  acceptance: `cargo test -p ra-console -q asks_without_taking` passes and runs at least `asking_whether_a_walk_runs_leaves_the_lock_free` and `counting_build_slots_takes_none`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/hub.rs crates/ra-console/src/hub/tests.rs crates/ra-console/src/agent/tests.rs

- Stale markers stop nothing: the stop file names its walk and the look-back marker its live pass {c2}
  objective: A leftover `var/walk.stop` stops the next walk after one card (doc of `stop_walk`, agent.rs:3063), and `lookback::running` is true while the marker file exists (lookback.rs:38-40) though the pass that wrote its pid there (lookback.rs:57) is dead, which holds every later pass. Write the target walk's pid into the stop file and ignore a stop for another pid; make `running` ask `pid_alive`.
  acceptance: `cargo test -p ra-console -q stale_marker` passes and runs at least `a_stop_left_for_a_dead_walk_does_not_stop_the_next` and `a_look_back_marker_of_a_dead_pass_holds_nothing`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/lookback.rs crates/ra-console/src/agent/tests.rs

### A planner with a reason for every wait

- The planner: which cards start now and why each other one waits {c3}
  objective: `spread` only prefers (agent.rs:4573-4589, 4697-4708): a free seat can stay empty behind a held-back card, and ready cards run in roadmap order whatever they unblock. Add the pure `walk::schedule::plan` of docs/fable/orchestration.md D5: order by warm session, then cards unblocked, then roadmap order; start every card that fits; give each other card one `Wait`.
  acceptance: `cargo test -p ra-console -q walk::schedule` passes and runs at least `a_free_seat_goes_to_the_next_card_that_overlaps_nothing`, `the_card_that_unblocks_most_starts_first` and `every_card_not_started_has_one_reason`
  files: crates/ra-console/src/walk/schedule.rs crates/ra-console/src/walk/schedule/tests.rs crates/ra-console/src/walk.rs

- The walk dispatches from the planner and reads claimed files from the ledger {c3}
  objective: `claimed_files` runs `git diff --name-only` for every running card on every 3 s tick (agent.rs:4592-4603). Record `touched` on the ledger at each checkpoint, feed `schedule::plan` from the roadmap and the ledger, start what it names, and delete `claimed_files` and the hold-back branch of the loop.
  acceptance: `cargo test -p ra-console -q planned_dispatch` passes and runs at least `the_walk_starts_the_cards_the_plan_names` and `a_tick_with_nothing_new_runs_no_git`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/schedule.rs crates/ra-console/src/commands/tests.rs

- A wide file takes more than one card: overlap by width, and a log of how merges ended {c2}
  objective: Overlap is by file name, and most orchestration cards name the 8600-line agent.rs; in the last walk 5 of 8 cards on it merged clean and 3 conflicted. Let a file be claimed by one card under 1500 lines and one more per further 3000, and append card, file and outcome (`clean`, `recorded`, `added`, `pass`, `worker`) to `var/merges.log` at every catch-up and landing so the rule can be corrected.
  acceptance: `cargo test -p ra-console -q width` passes and runs at least `a_short_file_holds_the_second_card_back`, `a_wide_file_takes_three` and `a_merge_writes_its_outcome_to_the_log`
  files: crates/ra-console/src/walk/schedule.rs crates/ra-console/src/walk/schedule/tests.rs crates/ra-console/src/agent.rs

- The walk report: what ran, what waited, and why each card is left {c3}
  objective: Every walk ends "ran N card(s) · no card is ready" (agent.rs:4797-4810), as line 859 of the feed did straight after five cards were blocked, and no walk records free seats. Add `walk::report::Report` of docs/fable/orchestration.md D7, add each free seat's tick to its wait reason, append the report to `var/walks.log`, write `var/walk.idle.json`, and end the walk with the counts and one reason per card left.
  acceptance: `cargo test -p ra-console -q walk::report` passes and runs at least `the_end_line_names_each_card_left_and_its_reason`, `free_seat_seconds_add_up_by_reason` and `a_walk_appends_one_line_to_the_walks_log`
  files: crates/ra-console/src/walk/report.rs crates/ra-console/src/walk/report/tests.rs crates/ra-console/src/walk.rs crates/ra-console/src/agent.rs

- The window starts a walk when what the last one waited for has happened {c3}
  objective: A card parked on a red main (agent.rs:5634) is looked at again only when someone starts a walk. Have the report carry `Wake::MainMoves { from }` when such cards are left, and have the window's tick beside `walk_requested` (apps/rc-gui/src/main.rs:2132) start a walk when the branch tip is no longer `from`.
  acceptance: `cargo test -p ra-console -q wake` passes and runs at least `a_moved_main_wakes_a_walk_that_waited_on_it` and `an_unmoved_main_wakes_nothing`; `cargo build -p rc-gui -q` prints nothing
  files: crates/ra-console/src/walk/report.rs crates/ra-console/src/walk/report/tests.rs apps/rc-gui/src/main.rs

- Build slots have ranks and no build runs without one {c3}
  objective: After 15 minutes a build runs slotless (agent.rs:2291-2294, 2079-2107), and a landing's proof queues level with every other build on a 500 ms poll (hub.rs:1045-1071). Give `try_build_slot` a rank: a landing's proof first, since it frees dependents; a worker's own build second, since a waiting worker holds a seat and a session whose prompt cache goes cold; the orchestrator's checks last. A waiter leaves `build-want-<project>-<rank>-<pid>`, a taker stands back for a live waiter of a better rank, and no wait ends in a slotless build.
  acceptance: `cargo test -p ra-console -q slot_rank` passes and runs at least `a_landing_takes_the_freed_slot_before_a_waiting_worker`, `a_worker_takes_it_before_a_waiting_check`, `a_dead_waiters_wish_is_ignored` and `a_long_wait_never_runs_without_a_slot`
  files: crates/ra-console/src/hub.rs crates/ra-console/src/hub/tests.rs crates/ra-console/src/agent.rs

- rc hold says its place in the queue on entry and each minute {c2}
  objective: `hold` prints one line when it starts to wait and nothing after (agent.rs:2295-2302); #213's worker saw `rc hold -- cargo test` print "nothing for over 12 minutes" and ended its session with no test result. Add a pure `wait_line(holders, waited)` naming how many builds run, whose they are (card and project, from `hub::build_slots`) and how long this one has waited, and print it on entry and each minute, so the worker waits knowingly or does other work.
  acceptance: `cargo test -p ra-console -q wait_line` passes and runs at least `the_wait_line_names_the_builds_ahead_and_the_time_waited` and `a_hold_that_gets_a_slot_at_once_prints_nothing`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/hub.rs crates/ra-console/src/agent/tests.rs

- The pool counts a project ahead of this one no higher than its runners, and a walk without a seat says so {c2}
  objective: `Pool::take` subtracts `held + ready` for each walking project ahead in the order (agent.rs:5052-5060) with no cap at that project's `[worker] parallel`, so a project with 4 runners and 30 ready cards leaves the ones behind it no seat while 8 are free, and the starved walk sleeps without a word (agent.rs:4797-4802). Cap each project's demand at `allocation::runners` of its directory (allocation.rs:42), write one feed line when a walk first waits for a seat naming the projects that hold them, and correct the doc comment that still says six slots.
  acceptance: `cargo test -p ra-console -q pool_demand` passes and runs at least `a_project_ahead_reserves_no_more_than_its_runners` and `a_walk_with_no_seat_says_who_holds_them`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- The walk report times each card's first build {c2}
  objective: The cost of a card's first build is known only from the workers' look-backs (6 to 15 minutes before leased targets). Time each card's first build-slot hold and how long it waited for the slot, and add `first_build_secs` and `slot_wait_secs` to the walk report, so the lease pool and the slot ranks are judged by the next walk.
  acceptance: `cargo test -p ra-console -q first_build` passes and runs `the_report_carries_the_first_build_and_slot_wait_of_each_card`
  files: crates/ra-console/src/walk/report.rs crates/ra-console/src/walk/report/tests.rs crates/ra-console/src/agent.rs

### Conflicts that cost no turns

- Git replays a resolution it has seen: rerere on for every project {c1}
  objective: #211 and #216 met the same conflict three times each, every time as a fresh one. Set `rerere.enabled` and `rerere.autoupdate` on the project's repository at walk start; the record is in the common git directory, so a resolution made in one worktree applies in the others and in the landing tree.
  acceptance: `cargo test -p ra-console -q rerere` passes and runs `a_conflict_resolved_once_merges_clean_the_second_time`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- Hunks both sides only added are merged without a model {c3}
  objective: Tests live in `tests.rs` beside the code and each card appends its own, so two cards conflict at the end of one file with an empty base, and the 15-turn conflict pass or a worker rerun pays for it (20 conflicts sent back to a worker on the agent project). Add `walk::merge::settle_added`, which rewrites a diff3-conflicted file keeping ours then theirs for every hunk whose base is empty and leaves any other hunk, and call it in `catch_up` before `conflict_pass` (agent.rs:3462-3492, 3901); `prove_tree` judges the result.
  acceptance: `cargo test -p ra-console -q walk::merge` passes and runs at least `two_tests_appended_to_one_file_are_both_kept`, `a_hunk_both_sides_changed_is_left_conflicted` and `a_file_with_both_kinds_keeps_only_the_real_conflict`
  files: crates/ra-console/src/walk/merge.rs crates/ra-console/src/walk/merge/tests.rs crates/ra-console/src/walk.rs crates/ra-console/src/agent.rs

### One walk

- The parallel walk counts landings for look-back and the retro, reads back, plans next and splits {c3}
  objective: `plan::read_back`, `plan_next`, `plan::split::due` and `retro::landed` are called only from `run_all` (agent.rs:2688, 2712-2728, 2750, 2780), which no walk runs since `spawn_walk` always passes `--parallel` (agent.rs:2986-3007). Call each from `run_parallel` at the matching point: read-back at start, the retro at each landing, split and plan-next when the planner starts nothing and nothing is in flight.
  acceptance: `cargo test -p ra-console -q parallel_walk_plans` passes and runs at least `a_parallel_walk_with_nothing_ready_plans_the_next_cards` and `a_parallel_walk_splits_a_card_that_is_due`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs crates/ra-console/src/plan/tests.rs

- A running walk moves to a newer install at the next stage boundary {c3}
  objective: Only `run_all` hands its lock to a newer install (agent.rs:2831-2848); #215's conflict went back to its worker at 1794553990, six seconds after the conflict pass landed. When `hub::moved_to` (hub.rs:810) names a newer install, start no card that has not run, wait for the runs in flight to reach a stage boundary, and `relaunch`; the ledger resumes them.
  acceptance: `cargo test -p ra-console -q newer_install` passes and runs at least `a_parallel_walk_relaunches_once_its_runs_reach_a_boundary` and `the_relaunched_walk_resumes_the_runs_from_the_ledger`
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs

- rc run --all is the parallel walk at width one {c3}
  objective: Two walks share one file and the tests exercise the one that does not run. Route `rc run --all` (apps/rc/src/main.rs:710) to `run_parallel` with width 1 and move the `run_all` tests in commands/tests.rs to a `walk_one_at_a_time` helper that calls it.
  acceptance: `cargo test -p ra-console -q commands::tests` passes with no test calling `run_all`; `cargo build -p rc -q` prints nothing
  files: apps/rc/src/main.rs crates/ra-console/src/commands/tests.rs crates/ra-console/src/plan/tests.rs

- run_all, walk_runner and handed_walk_lock are deleted {c2}
  objective: With every caller on `run_parallel`, the one-at-a-time walk (agent.rs:2651-2850, 2864-2880, 3026) and its `CHECK_RETRIES` and `REVIEW_RETRIES` are dead code of about 250 lines in the file every orchestration card edits.
  acceptance: `cargo build --workspace -q` prints nothing and `cargo test -p ra-console -q commands::tests` passes
  files: crates/ra-console/src/agent.rs crates/ra-console/src/commands.rs

- The landing, catch-up and merge functions move out of agent.rs into the walk module {c3}
  objective: agent.rs is 8600 lines and every card on the walk names it, which is what makes file-aware scheduling serialize the agent's own roadmap. Move `catch_up`, `catch_up_for_review`, `merge_in`, `settle_refusal`, `conflict_pass`, `prove_tree`, `checkpoint_worktree`, `land`, `land_or_say`, `say_landing` and `clean_after` to `walk/landing.rs` and `walk/merge.rs` unchanged, re-exported from `agent` so no caller changes.
  acceptance: `cargo build --workspace -q` prints nothing and `cargo test -p ra-console -q land` passes
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk/landing.rs crates/ra-console/src/walk/merge.rs

- run_parallel, the pool and the walk's lock files move out of agent.rs {c3}
  objective: The second half of the move: `run_parallel`, `Pool`, `Seat`, `higher_walking`, `other_walks`, `take_walk_lock`, `walk_running`, `request_walk`, `spawn_walk`, `stop_walk` and `relaunch` go to `walk.rs` and a new `walk/pool.rs` unchanged, re-exported from `agent`.
  acceptance: `cargo build --workspace -q` prints nothing and `cargo test -p ra-console -q walk` passes
  files: crates/ra-console/src/agent.rs crates/ra-console/src/walk.rs crates/ra-console/src/walk/pool.rs

### Only if the report asks for it

- A dependent starts on a dependency that is queued to land {c4}
  objective: A card is ready only when every dependency is `Accepted` (agent.rs:3107), so a chain waits for each link's proof. Build only if `idle_slot_secs` in `var/walks.log` shows waits on dependencies at stage `Queued` above a tenth of free-seat time. Let `schedule::plan` start such a card with its worktree branched from the dependency's branch, and send it through a catch-up when the dependency lands or is refused.
  acceptance: `cargo test -p ra-console -q stacked` passes and runs at least `a_dependent_starts_on_its_queued_dependency`, `it_merges_clean_once_the_dependency_lands` and `a_refused_dependency_parks_its_dependent`
  files: crates/ra-console/src/walk/schedule.rs crates/ra-console/src/walk/schedule/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/commands/tests.rs
