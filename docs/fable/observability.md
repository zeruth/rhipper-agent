# Observability: the window, the feed and what the person can see

Review of 2026-11-13. Scope: `apps/rc-gui` (dashboard, run cards, queue, gauges, metrics row),
the pair feed and the activity log, `rc status`, the look-back pass and its badge.

How to read this file. Part 1 is findings and design. Part 2 is the roadmap as an `rc import`
outline. The file is built so the whole of it can be handed to `rc import --dry`: Part 1 uses
no `##`/`###` headings, no `-` bullets and no field lines, which are the only things
`ra_roadmap::outline::parse` reads as items (crates/ra-roadmap/src/outline.rs:103-163), so only
Part 2 parses. I had no shell in this session: every number below is a Grep count over the
agent's own `var/pair.log` (1,096 lines, 46.6 hours), `var/activity.log` (14,337 lines) and
`var/retro.log`; no acceptance command below has been run; the import has not been dry-run.
The feeds of `runelite` and `rhipper-asm` were outside what I was allowed to read, so for those
two projects I rely on the measured table in the brief.

Not re-proposed here, because it is landed or in hand: Phase AA #235 to #238 (red main parks,
conflict pass, overlapping cards held, rerun carries work points), Phase X #201 to #203 (waiting
cards and walk stop in `rc status` and the window, orphan re-queue, cost per landed card in the
digest), #104/#112/#113 (machine gauges, report tab, token gauges), Look-back #244 and #247
(one filtered check, timeout that scales with slot load), Phase Z #211 and #216. Everything
below builds past those.


# Part 1. Findings and design

#### 0. The short version

The person oversees three projects and wants verdicts. Today the window gives them a wrong
verdict, no cause, no phase and no reason for waiting, and the numbers that Phase AA is judged
by cannot be read off the row that is supposed to show them.

| Rank | Finding | Evidence | Effect on the measures |
|---|---|---|---|
| 1 | The window marks failed runs as passed | activity log has 321 `◆end ✓` and 30 `◆end ✗`; the feed has 100 `end ✗` for the same runs | attention: about 70 failures in 46 h shown as green |
| 2 | A failure carries no cause | `end ✗ … needs revision` names no check, no test, no reviewer; the brief's cause table was made by hand | reruns: the cause split is the steering wheel for Phase AA and after |
| 3 | The metrics row miscounts | first-match `" in "` parse loses input on 62 lines; pass rate double-counts; sessions merge all projects and are glued by restarts | Phase AA's acceptance (rerun under 30%, pass over 70% on the next agent walk) cannot be read |
| 4 | A live run shows a clock and a step count | no phase; #241 spent 1,200 of 1,246 s in a timed-out check; landing takes 135 to 438 s with nothing on screen | wall-clock and attention |
| 5 | A waiting card has a reason only in three states | `hub::waiting` covers blocked, needs revision, not landed; a ready card that does not run, and an idle project, say nothing | attention; idle runners |
| 6 | Nothing says what changed since the last look | report lists landed cards only; failed toasts never expire, and never show on the dashboard | attention |
| 7 | Money is shown per card, not per attempt | four recent cards: $8.61 of $9.93 (87%) went to runs that did not land | cost per landed card |
| 8 | The look-back badge counts to ten and says nothing of yield | last pass planned 8 cards for $0.36; the first three run all failed, one in 50 s on an unrunnable check | reruns made by the planner itself |
| 9 | The window computes on the frame | 25 frames a second while live; per frame, per project: a clone of the roadmap, walk order, depths; per live card a clone of the whole run | CPU on a machine that is running 12 workers and 8 builds; jank |
| 10 | 18% of the feed is the window talking about itself | 201 of 1,096 lines are `window up`/`window down` | noise for the paired session and for every reader of the feed |

One design answers ranks 1 to 8: **a typed event log beside the prose feed, written by one
function, folded by one reader into a `Board` that `rc status`, the window, the metrics row and
the look-back all read.** Rank 9 is answered by a snapshot built off the frame. Rank 10 falls
out of the first.


#### 1. The window's verdict is wrong (rank 1)

What the code does. A run that returns `Ok` always writes `◆end ✓` to the activity log, whatever
the outcome text says (crates/ra-console/src/agent.rs:1118 and :1122); only an `Err` writes
`◆end ✗`. The feed gets the outcome's first line, which may start `✗ … needs revision`
(agent.rs:1116-1128). The window reads the activity log, so:

+ `var/activity.log:14327-14328`: `@#241 check failed: cargo test --workspace -q · timed out after 1200s`
  and on the next line `@#241 ◆end ✓ $0.1247 · 90% cached`. The feed line for the same run,
  `var/pair.log:1096`, is `end ✗ #241 … needs revision`.
+ Same for #240: activity.log:14316 `◆end ✓ $0.1868`, pair.log:1092 `end ✗`.
+ Totals: 351 end markers in the activity log, 30 of them ✗. The feed has 100 `end ✗`. So
  about 70 runs were drawn with a green outcome and raised a green toast
  (apps/rc-gui/src/main.rs:2216-2227 reads `outcome.starts_with('✗')`). A caveat on the 70:
  the activity log has no timestamps, so the two files cannot be cut to the same span. It
  holds 351 end markers against the feed's 302 starts, so it covers at least the feed's span
  and 70 is a floor on that reading; the two runs above are the line-for-line proof.

And the opposite error on the feed. `end ✓ … implemented` is not a pass either: it is followed
by `#N not reviewed: main merged into card/N with conflicts … sent back` at pair.log:1004-1005,
1048-1049, 1062-1063 and 1082-1083. After a `review ✗` the end line is still `end ✓ …
implemented` (pair.log:519-524, #185). The mark on an end line means "the worker stopped
without an error", which is not what anyone reading it assumes.

Around it:

+ The verdict strip shows only the first line of the last handoff's verdict
  (main.rs:2096-2104).
+ A failed toast never expires: `retain(|t| t.failed || t.shown.elapsed() < TOAST_FOR)`
  (main.rs:2232-2262). Toasts are drawn only from `Workspace::draw` (main.rs:4185), so the
  dashboard and the report, where the person sits when overseeing three projects, show none,
  and a tab opened an hour later shows the stale stack.
+ Card notes, which the person and the next worker both read as "the verdict", hold raw check
  output including git's `LF will be replaced by CRLF` warnings (#237's notes in `rhipper.md`).

Design. One outcome type for a run, written once, read everywhere (section 2 has the record):

```rust
pub enum Outcome {
    Open,                       // still running, or implemented and in the landing queue
    Landed,                     // the only pass
    SentBack(Cause),            // will be run again by the walk
    Parked(Cause),              // stays put until main or a prerequisite changes
    Yours(Cause),               // waits for the person
    Crashed,                    // the worker or the window died under it
}
pub enum Cause {
    Check { command: String, timed_out: bool },
    Lint,
    Review { by: String },
    Conflict,                   // main merged into the card with conflicts
    MergedCheck { command: String },   // green on the card, red on the merged tree
    MainRed,
    Turns,                      // the worker ran out of turns
    Refused,
    NoVerdict,
    Restart,                    // orphaned by a window or walk restart
}
```

The quick fix that does not wait for the event log is card 1 of the roadmap: `◆end` takes its
mark from the outcome's first line and carries the cause in words
(`◆end ✗ check timed out: cargo test --workspace -q · $0.12`).

How it is measured. For runs started after the card lands, the count of `◆end ✗` in
`var/activity.log` equals the count of `end ✗` in `var/pair.log` (today 30 against 100). After
the event log lands: every `End` event has a cause unless its outcome is `Landed` or `Open`; a
test holds that.


#### 2. The feed is prose that four readers parse by substring (ranks 2, 3, 10)

What is there. `pair::event(root, text)` appends `time<TAB>free text`
(crates/ra-console/src/pair.rs:16-23). 42 call sites outside tests (36 of them in
`agent.rs`) each format their own sentence. The readers:

+ `apps/rc-gui/src/metrics.rs:65-126` splits lines on `" in "`, `" out "` and `$`.
  `number_after(&body, " in ")` (:97, fn at :179-183) takes the **first** `" in "`. Titles
  contain it: #207 "One-runner walks run in a worktree too", #211 "… prerequisites are in its
  base …". 62 start/end lines in the agent's feed have `" in "` in the title part. On those
  the input reads as 0; the #207 run alone loses 1,097,499 input tokens from ACCEPTED / M
  TOKENS and READ-IN / INPUT.
+ PASS RATE: `checks_failed` counts any `end ✗ … needs revision` (:109) and the denominator is
  reviews plus checks failed (:171). A run whose review rejected it and whose end line says
  needs revision is counted twice as a failure.
+ Sessions are cut at a one-hour gap between any two lines of **any** open project (:65-95,
  `SESSION_GAP = 3600`). `window up/down` lines count as activity, and there are 201 of them
  ("restarts in 24 h: 30"), so sessions rarely close. All projects are summed, so asm's 97%
  rerun rate drowns the agent's 69%. There is no per-project and no per-walk number, and there
  cannot be one from this feed: a walk writes a line when it stops (agent.rs:2695, :4806) and
  none when it starts (only its digest, agent.rs:2683). Phase AA's
  acceptance is "RERUN RATE under 30% and PASS RATE over 70% on the next agent walk". The row
  cannot show a walk, or the agent alone.
+ `hub::waiting_of` and `feed_reason` (crates/ra-console/src/hub.rs:489-542) search the last
  2,000 lines for a sentence about the card. `pair::last_stop`, `live_start` (pair.rs:69-102)
  do the same for the walk.

What is noise in it (agent feed, 1,096 lines):

| Lines | Share | What |
|---|---|---|
| 201 | 18% | `window up …` / `window down …` |
| 302 | 28% | `start #id <title of 80 to 150 chars> · model · checks · pid` |
| about 250 | 23% | `end …`, each repeating the title and the dead field `0 tool call(s)` |
| 126 | 11% | `review ✓/✗ <title again>` |
| 47 | 4% | `landed #id <title again>` |
| 30 | 3% | `wait: the check waits for a worker slot, N of M are running`, with **no card id** (agent.rs:2097-2103) |
| about 116 | 11% | `#N not landed…`, `#N not reviewed…`, `walk ran N card(s)…`, look-back, added |

A rerun appears as a plain `start` with no ordinal and no reason. The check line
`check passed: cargo test -p ra-console -q merge_proof` appears three times for one run of #215
(activity.log:14314, 14322, 14329).

Design: `crates/ra-console/src/events.rs`, one record, one writer, one reader.

```rust
/// One line of var/events.jsonl.
#[derive(Serialize, Deserialize)]
pub struct Event {
    pub at: u64,                 // unix seconds
    pub card: Option<i64>,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Kind {
    WalkStart { runners: u32, ready: u32 },
    WalkStop  { why: String, ran: u32, landed: u32 },
    Start     { title: String, model: String, pid: u32, checks: Vec<String> },
    Phase     { phase: Phase, detail: String },
    Review    { pass: bool, by: String, line: String },
    End       { outcome: Outcome, turns: u64, input: u64, cached: u64, output: u64, usd: f64, line: String },
    Landed    { commit: String },
    SentBack  { cause: Cause, line: String },        // after the end: conflict, merged check
    Parked    { cause: Cause, line: String },
    Lookback  { evidence: u32, planned: Vec<i64>, usd: f64 },
    Window    { up: bool, build: String, pid: u32 },
    Note      { text: String },                      // everything not yet typed
}

pub enum Phase { Work, SlotWait, Check, Lint, Review, Merge, Land }

/// Appends the event to var/events.jsonl and, unless it is a Window event,
/// its prose line to var/pair.log, so the paired session reads what it reads today.
pub fn emit(root: &Path, kind: Kind, card: Option<i64>);

/// The sentence the pair feed shows for an event. One place owns the wording.
pub fn prose(event: &Event) -> Option<String>;
```

Reader, same shape as the `pair::Follower` that already exists and that the window does not use
(pair.rs:120-161):

```rust
pub struct Tail { offset: u64, pub board: Board }
impl Tail { pub fn poll(&mut self, root: &Path) -> bool }   // true when the board changed

#[derive(Default, Clone)]
pub struct Board {
    pub runs: BTreeMap<i64, Vec<RunRecord>>,   // every attempt of every card, in order
    pub live: BTreeMap<i64, LiveRun>,          // started, not ended
    pub landing: BTreeMap<i64, u64>,           // implemented and accepted, not yet landed: since when
    pub walks: Vec<WalkRecord>,
    pub passes: Vec<Pass>,                     // look-back passes
    pub window_starts: Vec<u64>,
}
pub struct RunRecord {
    pub card: i64, pub nth: u32,               // 1 is the first run; the fold counts starts
    pub walk: usize,                           // index into walks
    pub started: u64, pub ended: u64, pub model: String,
    pub spans: Vec<(Phase, u64)>,              // seconds in each phase, in order
    pub outcome: Outcome,                      // End, then Landed / SentBack / Parked refine it
    pub turns: u64, pub input: u64, pub cached: u64, pub output: u64, pub usd: f64,
    pub line: String,                          // the one sentence a person reads
}
pub struct LiveRun {
    pub card: i64, pub nth: u32, pub started: u64, pub pid: u32, pub model: String,
    pub phase: Phase, pub phase_since: u64, pub detail: String,
    pub before: Option<Cause>,                 // why the previous run of this card did not land
}
pub struct WalkRecord {
    pub started: u64, pub stopped: Option<u64>, pub why: String,
    pub runs: u32, pub reruns: u32, pub first_run_landed: u32, pub cards: u32, pub landed: u32,
    pub reviews: u32, pub reviews_passed: u32,
    pub usd: f64, pub usd_not_landed: f64, pub secs: u64,
    pub causes: BTreeMap<String, u32>,         // cause name to count: the brief's table, made by the machine
}
pub struct Pass { pub at: u64, pub evidence: u32, pub planned: Vec<i64>, pub usd: f64 }
```

The flow.

1. `agent.rs` calls `events::emit` at the run's start, at each phase change, at review, at the
   end, at landing and at each send-back; `lookback.rs:66` and the walk's start and stop do the
   same. The `pair::event` sites that are not one of these stay as they are (they become
   `Note` when someone needs them typed).
2. `emit` writes the JSON line, then calls `pair::event(root, &prose(&event))`. The prose
   drops the dead `0 tool call(s)`, cuts the title to 60 chars after the first mention in a
   run, and adds the ordinal and cause: `start #235 run 2 after conflict · fable · pid 4120`,
   `end ✗ #241 check timed out after 1200s: cargo test --workspace -q · 12 turns · $0.12`.
3. `Window` events go to `events.jsonl` only. `pair::window_up` counts restarts from
   `Board::window_starts` instead of from feed lines.
4. Each reader holds one `Tail`. `rc status` builds one per call (one pass over the file).
   The window holds one per project on its refresher thread (section 8).

What it removes: the substring parsers in `metrics.rs` (:65-126, :179-183),
`hub::feed_reason` and the three whole-file reads in `hub::status_with` (hub.rs:598-625),
`pair::last_stop`/`stop_of`/`live_start` as feed scans, 201 lines in every 1,096 of the feed.

Size. About 10 events a run at about 200 bytes: the agent's 302 starts in 46 h would be about
600 KB. The fold is a single pass; the tail is incremental. Rotation can wait until a file
passes 20 MB.

How it is measured. (a) The metrics row and the brief's hand count agree on one walk.
(b) The count of `window ` lines in `var/pair.log` stays flat across a day. (c) A test feeds
the fold three runs shaped like #235's (failed, sent back on a conflict, landed) and reads
back `nth` 1, 2, 3 with their causes.


#### 3. What a run is doing right now (rank 4)

What the person sees today. A run card titles itself `"{clock} · {n} steps"` while live
(apps/rc-gui/src/console.rs:323-402) and lists the last tool steps (:407-521). The clock is
the window's `Instant` at attach (crates/ra-console/src/logs.rs:58-76), so each of the 30
restarts a day resets it. There is no phase, no turn count, no tokens, no time since the last
step, no rerun number and no word about why the previous run failed.

What that hides:

+ #241 ran 1,246 s, of which 1,200 s was the default check `cargo test --workspace -q` timing
  out. The card showed a clock.
+ `var/retro.log` line 3 (#213): "`rc hold -- cargo test` printed nothing for over 12 minutes,
  most likely waiting for a build slot, so the session ended with no test result". `rc hold`
  says it waits once, on stderr, and is then silent until `HOLD_WAIT`, 15 minutes, runs out
  (crates/ra-console/src/agent.rs:2072, :2279-2304, the `said` flag at :2295). The worker
  could not see how long or behind what, and neither could the person: this wait is inside
  the worker's own process and writes nothing to the feed at all. The 30 `wait:` lines the
  feed does have come from the agent's own checks (`hold_build`) and name no card. A run that
  ends with no test result is a rerun, so this is a rerun cause, not only a blind spot. Line
  1 of the same log (#215) is the same blindness met from a session: "`rc wait` gave up twice
  at 540 s while the build ran".
+ From the review-accepted end to `landed`: #233 136 s, #221 135 s, #214 438 s, #237 171 s,
  #212 203 s, #207 257 s, #208 214 s, #238 141 s, #236 326 s, #235 293 s. Median 209 s, 39
  minutes over ten cards, during which the run card is closed and the queue row says nothing.
+ #235's first run was 65 turns and 28 minutes before it failed. Nothing on the card says
  "turn 65 of a c3 card".

Design. The `Phase` events of section 2 are emitted where the phase already changes:

| Phase | Emitted at | Detail carried |
|---|---|---|
| `Work` | run start (agent.rs:1046-1062) | model |
| `SlotWait` | the agent's `hold_build` (agent.rs:2079-2107, six call sites), now with the card id; and the worker's `rc hold` (agent.rs:2279-2304), which knows its card from its worktree path | `8 of 8 busy · behind #215 cargo test 3:10` from `hub::build_slots()` (hub.rs:1078), or `target held by #215` (:2059) |
| `Check` | each acceptance command start | the command |
| `Lint` | clippy on the changed lines | |
| `Review` | where `review by <model>` is logged | the reviewing model |
| `Merge` | catch-up and squash-merge | `7 commits from main` |
| `Land` | after the end, when the card enters the landing queue | position in the queue |

Live turns and tokens. `TurnTally` in crates/ra-providers/src/claude_code.rs:556-575 already
books each turn's usage as the stream arrives, and the read loop already publishes each tool
step with `tracing::info!(target: "ra::progress", …)` (claude_code.rs:408-413). One more line
on the same target when a new turn id is booked, `◇ 12 turns · 61k`, needs no new channel; it
arrives in the activity log tagged like any step, and `Sink::progress` (logs.rs:133-174) folds
a line starting `◇` into two new fields on `Run` (`turns`, `context`) instead of pushing a
step. `rc hold` gets the same treatment for the worker's benefit: it repeats its wait line
every 60 s with what it is behind, so a worker can decide to wait instead of ending the
session.

The run card, in Blood Moon. No new palette entries: `Work` is `theme::accent()`, `SlotWait`
is `theme::dim()`, `Check` and `Lint` are `theme::warn()`, `Review` is `theme::light()`,
`Merge` and `Land` are `theme::online()`, a failed span is `theme::error_text()`. The strip is
the existing `pill_bar` with one segment per span, widths by seconds; the breathing border
stays and takes the colour of the current phase.

A word on the mock-ups in this document: they show shape. A figure in one is real only where
the text around it cites the feed; card names, causes, clock times and commits in them are
made up, and anything about runelite is made up because I could not read that project.

```
┌ #215 Every merge is proven clean before a worker gets the tree ──────────────── run 2 ┐
│ after ✗ check: merge_proof                                    fable · 12 turns · 61k  │
│ ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░▒▒▒▒▒▒▒                                                           │
│ work 6:10        slot 0:40  check 2:05 …                      last step 4 s ago       │
│ ◆ $ cargo test -p ra-console -q merge_proof                                           │
│ ◆ Edit crates/ra-console/src/agent.rs                                                 │
└───────────────────────────────────────────────────────────────────────────────────────┘
```

Rules that turn the strip into a verdict the person can act on without reading steps:

+ `last step` turns `warn` after 120 s in `Work`, and the card gets a `quiet 4 min` badge.
+ `turns` turns `warn` past the card's budget (10 turns per complexity point is the starting
  rule; `ran_far_over` at main.rs:5190 already has a cost version of this).
+ `SlotWait` over 60 s shows what it is behind.
+ A card in `Land` keeps a one-line row in the queue: `#236 landing · 3:10 · 2nd in the queue`.

How it is measured. Wall-clock by phase becomes a number: `WalkRecord` sums `spans`, and the
WALL / LANDED metric (section 6) shows the split on hover. The #247 card (timeout scales with
slot load) gets its before and after from `SlotWait` and `Check` seconds. For attention: the
person can answer "what is #215 doing" from the dashboard without opening the tab.


#### 4. Why a card is waiting (rank 5)

What is there. `hub::Waiting { id, title, state, reason }` (hub.rs:433-439) with a short form
(`reviewer ×n`, `check ×n`, `landing`, `idle`, `refused`, `main red`, :451-472), computed by
`waiting_of` for three states: blocked, needs revision, implemented and not landed
(:489-524). The dashboard shows `N waits` with the lines on hover (main.rs:4358-4361).

What is missing:

+ A **ready** card that is not running. It may wait on an unaccepted prerequisite, be held
  behind a card that names the same file (#237 writes `#N waits behind #M: both name <file>`
  to the feed, agent.rs:4703, and nothing reads it), have no free runner, sit in a project
  lower in the machine order, be stopped by the plan's week limit, or there may be no walk at
  all.
+ An **idle project**. With no live run the project card is its header line and nothing else
  (`if live.is_none() { return; }`, main.rs:4377-4379). The last walk's stop reason is in the
  feed and in `rc status`, not on the card. With three projects, "why is this one doing
  nothing" is the first question and the dashboard has no answer.
+ The header says `"{projects} projects · {live} running"` where `live` counts projects with a
  run, not runs (main.rs:1093-1095).

Design. One function in the hub answers for every open card and for the project, from the
roadmap and the `Board`:

```rust
pub enum Why {
    Running { phase: Phase, since: u64 },
    Landing { since: u64, place: u32 },
    Next,                                   // a runner takes it when one frees
    NoRunner { busy: u32, of: u32 },
    WaitsOn(Vec<i64>),                      // prerequisites not accepted
    Behind { card: i64, path: String },     // overlapping card is running
    Again { nth: u32, cause: Cause },       // sent back, the walk will rerun it
    Parked(Cause),
    Yours(String),                          // the command that frees it, e.g. `rc set 245 defined`
    WalkStopped { at: u64, why: String },
    NoWalk,
    WeekLimit,
}
pub fn why(nodes: &[Node], board: &Board, slots: &Slots, id: i64) -> Why;

pub struct ProjectWhy {
    pub walk: Option<(u64, u32)>,           // since, cards run so far
    pub idle: Option<Why>,                  // WalkStopped / NoWalk / WeekLimit when nothing runs
    pub yours: Vec<(i64, String)>,          // cards that wait for the person, with the command
    pub next: Vec<i64>,                     // what the walk would take up, in order
}
pub fn project_why(nodes: &[Node], board: &Board, slots: &Slots) -> ProjectWhy;
```

`Why::short()` and `Why::line()` replace `Waiting::short()` and `Waiting::line()`; `Waiting`
stays as the list of cards whose `Why` is `Again`, `Parked` or `Yours`.

Where it shows:

+ Every queue row ends in its reason, dim, one jewel wide: `next`, `behind #215`, `waits on
  #211`, `run 3 after review`, `yours`.
+ An idle project card is three lines instead of none:

```
┌ RUNELITE  ~/code/runelite ──────────────────────────────────────── idle 2 h 10 min ┐
│ walk stopped 12:44 · no card is ready                                              │
│ yours  #88 Tile overlay batches draws · reviewer ×3 · `rc set 88 defined`          │
│ next   nothing until #88: 3 cards wait on it                                       │
└────────────────────────────────────────────────────────────────────────────────────┘
```

+ `rc status` prints the same lines; `rc why <id>` prints one card's `Why` and its runs (the
  turns and costs below are #235's from the feed, as is the 4:53 before landing; the causes,
  the other durations, the check name and the commit are placeholders, since the feed does
  not record them, which is the point):

```
#235 A red main parks the card instead of rerunning it [accepted]
  run 1  fable  65 turns  28:02  $2.54  ✗ check: cargo test -p ra-console -q main_red
  run 2  fable   6 turns   3:10  $0.41  ⟲ conflict with main
  run 3  fable  12 turns   5:40  $0.30  ✓ landed 6c0410c after 4:53 in the landing queue
  $3.25, of which $2.95 did not land
```

How it is measured. A test builds a roadmap in which each variant applies and reads the
variant back. In use: the count of open cards whose `Why` is `Next` while a runner is free is
the scheduler's idle-runner bug count; it should be 0, and the dashboard would show it.


#### 5. What changed since the person last looked (rank 6)

What is there. The report tab lists implemented and accepted cards, newest first, with model,
cost, tokens per turn and age (main.rs:1371-1562). No failures, no rerun count, no wall-clock,
no notion of "new". Toasts are the only push, and section 1 covers how they fail.

Design.

+ A **seen mark** per project: `var/seen`, one unix time, written when the person presses
  `seen` on the band or opens the project's tab. Kept out of `rhipper.md` and out of the feed.
+ A **band** at the top of each project card on the dashboard, built from `Board` runs with
  `ended > seen`:

```
│ since 14:02   ▲ 4 landed $1.32   ✗ 3 failed first run $8.20   ⟲ 1 conflict   ⏸ 2 yours   look-back planned 8   [seen] │
```

  Each count is a `badge_marked`; a hover lists the cards with their one sentence
  (`RunRecord::line`). The `yours` badge is `theme::warn()` and does not clear on `seen`; it
  clears when the card moves.
+ **Toasts move to the window**: one stack in `App`, fed by every tab's ended runs, drawn on
  the dashboard, the report and the tabs. A failed toast lives 30 s and then is in the band;
  nothing is immortal.
+ The **report becomes the verdict ledger**: one row per finished card since a chosen time
  (seen, today, this walk), failures included, grouped by outcome:

```
 ✓ #236 Run a conflict pass on merge conflicts   2 runs · 52 turns · 31 min · $3.02 ($2.68 not landed) · first run ✗ check
 ✗ #241 Dry-run handoff prints the context       1 run · 12 turns · 21 min · $0.12 · check timed out after 1200s  → yours
```

  The sentence after the numbers is the reviewer's first line for a landed card and the cause
  for one that did not land. Clicking a row opens the card view, which gains the run history
  of section 4 and the whole verdict instead of its first line.

How it is measured. Attention: the number of project tabs opened per hour of oversight should
fall (the band's card emits a `Note` event each time a tab is opened from the dashboard, so
the count per day is in the event log). Correctness: a test gives the band a board and a seen
time and reads the counts.


#### 6. Where the money and the time went (ranks 3, 7)

Per attempt, the feed already holds the story and no view shows it:

| Card | Run 1 | Later runs | Landed run | Not landed |
|---|---|---|---|---|
| #235 | 65 turns, $2.54, ✗ with no cause on the feed, 28 min | 6 turns $0.41, sent back on a conflict | 12 turns $0.30 | $2.95 of $3.25 |
| #236 | 49 turns, $2.68, ✗ | | 3 turns $0.34 | $2.68 of $3.02 |
| #207 | 14 turns, $1.70, ✗ (109k a turn) | | 7 turns $0.25 | $1.70 of $1.95 |
| #238 | 47 turns, $1.28, ✗ | | 9 turns $0.43 | $1.28 of $1.71 |
| sum | | | $1.32 | $8.61 of $9.93, 87% |

The landed runs are cheap (3 to 12 turns, $0.25 to $0.43) because the rerun starts from the
work. The first run is where the money goes, and a first run that passes is the one number
that moves cost per landed card. The row should lead with it.

The metrics row, redesigned. Six tiles stay; the unit becomes a **walk** (explicit
`WalkStart`/`WalkStop` events, no gap heuristic), the scope a **project** (chips ALL · AGENT ·
ASM · RUNELITE, the existing `chip` widget), the sparkline the last 10 walks.

| Tile | Formula over a walk | Hover |
|---|---|---|
| FIRST-RUN PASS | cards whose run 1 landed / cards whose run 1 ended | the cards that did not |
| RERUN RATE | runs with `nth > 1` / runs | `causes`: check 49 · conflict 20 · review 14 … |
| REVIEW PASS | `reviews_passed / reviews`, each review counted once | by reviewing model |
| COST / LANDED | `usd / landed` | `usd_not_landed / usd` as "not landed 87%" |
| TOKENS / TURN | `(input + cached) / turns`, median and p90 | cache read share, read-in share (today's two tiles) |
| WALL / LANDED | seconds of every run of the walk, start to landed or sent back, summed / landed | seconds by phase: work, slot wait, check, review, land |

ACCEPTED / M TOKENS, CACHE READ and READ-IN / INPUT do not go away; they become hovers of the
tile they explain. The dashboard header counts runs: `3 projects · 5 running · 2 landing`.

`rc status` gets the last walk's line per project from the same `WalkRecord` (shape; 7 runs,
4 landed and $0.77 are the brief's last agent walk, the clock times and the cause split are
made up):
`walk 09:12 to 10:40 · 7 runs · 4 landed · 3 reruns (2 check, 1 conflict) · $3.08 · $0.77 a landed card`.

How it is measured. This section is the measuring instrument for the others and for Phase AA.
Its own check: on one agent walk, the tile values equal a hand count of the feed.


#### 7. The look-back pass and its badge (rank 8)

What is there. `lookback::EVERY = 10`; the badge reads `look-back {since}/{every}` or
`look-back running` with a generic hover (main.rs:4337-4346). The pass writes one feed line
(crates/ra-console/src/lookback.rs:54-70). Nothing records which cards a pass planned or how
they fared.

What the feed says about the last pass (pair.log:1071-1072): 8 cards from 39 evidence lines
for $0.36. Of the first three to run: #240 ✗, #241 ✗ (it named `rc handoff --dry-run` and
`docs/context.md`, neither of which exists, retro.log line 2), #245 ✗ in 50 s on the check
`cargo test -p <review-crate> relocate_points` (pair.log:1074, 1078). `rc check`'s placeholder
rule only catches a field that is a bracket from end to end
(crates/ra-roadmap/src/tree.rs:261-266), so a hole inside a command passes.

The planner is the cheapest place to stop a rerun and the person has no view of whether it is
adding to the rerun rate or taking from it.

Design.

+ `Kind::Lookback { evidence, planned, usd }` records the pass. Yield is a join, not a new
  store: for a `Pass`, `planned` ids against `Board::runs`.

```rust
pub struct Yield { pub planned: u32, pub run: u32, pub first_run_landed: u32, pub landed: u32, pub usd: f64 }
pub fn yield_of(pass: &Pass, board: &Board) -> Yield;
```

+ The badge becomes `look-back 7/10 · last 0 of 3`, `theme::warn()` when fewer than half of
  the last pass's run cards landed on their first run. Hover: the pass's cards, one line
  each with outcome and cause. The report gets a LOOK-BACK group.
+ `rc check` (and so the planner's own gate) refuses a backticked command with a `<word>` hole.
  This is outside the window; it is here because the feed found it and it is one small card.

How it is measured. First-run pass of look-back cards against first-run pass of hand-written
cards, per pass, on the badge.


#### 8. What is slow in the window (rank 9)

The window repaints every 40 ms while any run is live (main.rs:1364-1365 and :4188-4189),
and with three projects walking, a run is always live. On each of those 25 frames a second:

| Where | What runs on the frame |
|---|---|
| main.rs:991-995 | every tab's `tick`, also on the dashboard and the report |
| main.rs:2127-2210 | `tick`: file stats; on a roadmap change `rt.block_on(store.load())` parses all of `rhipper.md` (about 1,400 lines, notes of several KB each, rewritten on every state change) on the UI thread, then `store.history(None)` |
| main.rs:2216-2227 | `raise_toasts`: `sink.run(announced + 1)` clones a whole `Run`, steps and all |
| main.rs:4226-4279, :4409 | per project: `self.roadmap.clone()`, `walk_order`, `tree::depths`, `self.last_changes.clone()`, `waiting_on` |
| main.rs:3595 | card view: `self.roadmap.clone()` |
| main.rs:2848-2849 | roadmap panel of every drawn tab: `tree::depths(&self.roadmap)` and `self.roadmap.clone()` |
| console.rs:84-86, :123 | per project: every run line's text cloned; per live card `sink.run(id)` clones the `Run` with all its steps (`Sink::run`, logs.rs:297-299) |
| console.rs:323-402 | title fitting by binary search over text measurement |
| logs.rs | `live_run()`/`live_runs()` lock and scan the run map several times per tab; `live_run_of_pid` scans all lines per tagged step (:276-294) |

Once a second or slower, still on the UI thread: `hub::waiting` per project, which reads the
whole of `pair.log` through `pair::recent(root, 2000)` (main.rs:4241-4251, pair.rs:60-64);
three SQLite queries per tab in `TokenGauges::tick` (main.rs:546-580); five ledger queries and
the gotchas catalogue in `refresh_ledger` (main.rs:2025-2071); `git diff HEAD`
(main.rs:2108-2123); every project's whole feed for the metrics row every 30 s
(main.rs:1200-1204).

And one leak: `Sink` caps lines at 10,000 (logs.rs:351-354) but never prunes the `runs` map.

I did not profile; the claim is structural: work that depends on three inputs (roadmap mtime,
event offset, sink version) is redone 25 times a second, and blocking I/O sits on the thread
that paints.

Design.

```rust
/// Everything a project's dashboard card and queue draw. Built off the frame.
pub struct Snapshot {
    pub version: u64,
    pub nodes: Arc<Vec<Node>>,
    pub order: Vec<i64>,                        // walk order
    pub depths: HashMap<i64, usize>,
    pub why: HashMap<i64, Why>,
    pub project: ProjectWhy,
    pub board: Arc<Board>,
    pub since: Since,                           // the band's counts
    pub ledger: Ledger,                         // today's and all-time totals, by card
    pub changes: Arc<Vec<Change>>,
}
/// Pure: the refresher does the reads and hands the parts in. `since` and `ledger` join the
/// struct with their own cards.
pub fn build(nodes: Arc<Vec<Node>>, gambits: &[Gambit], board: Arc<Board>, slots: &Slots,
             changes: Arc<Vec<Change>>, prev: Option<&Snapshot>) -> Snapshot;
```

+ One **refresher thread** for the window (not one per tab) loops over open projects: stats
  the roadmap, polls each `events::Tail`, checks `usage.db`'s mtime; when one moved, builds a
  `Snapshot`, stores it in that tab's `Arc<Mutex<Arc<Snapshot>>>` and calls
  `ctx.request_repaint()`. It reuses `prev`'s parts whose input did not move.
+ A frame does `let snap = tab.snap();` (an `Arc` clone) and draws. `tick` keeps only what
  must be on the frame (`quit_requested`, `tail_activity_at`, which is already incremental).
+ `Sink::with_run(id, |run| …)` lends the run under the lock. `Sink::version()` exists
  already (logs.rs:319-321, :356, :363-365: an `AtomicU64` bumped on every push) and is what
  the repaint rule below keys on. Ended runs beyond the newest 200 are dropped and a run keeps
  its newest 2,000 steps with a count of the rest.
+ Repaint: on a snapshot or sink change at once; otherwise every 100 ms while a run is live
  and the window has focus (enough for the breathing border and the clock), every 1 s without
  focus. The cadence is one pure function so it can be tested.
+ The debug badge that exists on the dashboard status row gains `frame p95 N ms · M fps`.

What it removes: every `rt.block_on` reachable from `App::ui`; the per-frame clones listed
above; the once-a-second whole-feed reads.

How it is measured. The debug badge before and after, on the same three projects with runs
live: frames a second from 25 to 10, p95 frame time down. A test asserts that drawing the
dashboard twice with no input change does not call the builder (a counter on the refresher).

A note on `main.rs`. It is 5,297 lines. Every window card that names it makes a worker read
around it at 40 to 63k tokens a turn, and #237 now holds cards that name the same file, so
window cards run one at a time. The cards below put new code in new modules
(`snapshot.rs`, `since.rs`, `report.rs`, `phase.rs`) and touch `main.rs` only at the call
site, which is also what lets two of them run at once later.


#### 9. What moves where

| Today | Becomes | Removed |
|---|---|---|
| 42 `pair::event` sentences, 4 substring readers | typed sites call `events::emit`; readers fold a `Board` | `metrics::sessions`, `number_after`, `hub::feed_reason`, feed scans in `pair.rs` |
| `◆end ✓` for any `Ok` | mark from the outcome, cause in words | 70 false greens per 46 h |
| `Waiting` for 3 states | `Why` for every open card and the project | idle project with no explanation |
| `window up/down` in `pair.log` | `Window` events only | 18% of the feed |
| metrics by merged session | by walk, by project | glued sessions, double-counted pass rate |
| report of landed cards | verdict ledger with failures, attempts, waste, wall time | first-line-only verdict |
| toasts per tab, failed ones immortal | one stack in `App`, 30 s, then the band | stale stacks |
| `look-back n/10` | `n/10 · last k of m` with yield | |
| per-frame compute, 25 fps | snapshot off the frame, 10 fps | UI-thread `block_on`, per-frame clones, unbounded runs map |

Open questions I could not close from what I could read:

1. Thirty window restarts in 24 hours. The agent landed 47 cards in 46.6 h, about 24 a day,
   so the restarts track landings on the self-hosted project (the window rebuilding onto the
   newer build). That is an inference from the two rates, not something I traced in code. If
   it holds, the event-based clock (section 3) makes restarts harmless to read, and the
   restart itself is a question for whoever owns #181.
2. Whether the review always sits inside a run's start and end. It does for #185
   (pair.log:519-524: `review ✗`, then `end ✓ … implemented`); I did not trace every path in
   `agent.rs`. The fold in section 2 does not depend on the order: `Review`, `End`, `Landed`
   and `SentBack` each refine the last run of the card whenever they arrive.
3. The asm and runelite feeds may hold line shapes the agent's does not. The event log makes
   that moot going forward; nothing below parses old prose.
4. History. The `Board` starts empty on the day the event cards land; the metrics row shows
   `n of 10 walks` until it fills. I chose not to spend a card on parsing 1,096 lines of old
   prose into events: the baseline is the table in the brief and the one below.


#### 10. The scoreboard: what each change should move, and where to read it

Most of this work does not lower the rerun rate by itself. It makes every rerun attributable,
which is what the next round of fixes needs, and it gives the person the verdict without the
tab. Two cards do cut reruns directly: the `rc hold` line (no more sessions that end with no
test result) and the hole check in `rc check` (no more look-back cards whose check cannot
run).

| Measure | Baseline: agent feed, 46.6 h to 2026-11-13 | After | Read from |
|---|---|---|---|
| failed runs the window marks ✓ | 70 of 100 | 0 | `◆end ✗` count in the activity log equals `end ✗` count in the feed |
| failed ends that name a cause | 0 of 100 | all | `rc why <id>`; a test on the fold |
| feed lines about the window | 201 of 1,096 (18%) | 0 | count of `window ` lines in `var/pair.log` over a day |
| lines the metrics row mis-parses | 62 | none: nothing parses prose | `metrics.rs` holds no `number_after` |
| Phase AA's two numbers for one agent walk | not readable | RERUN RATE and REVIEW PASS tiles, scope AGENT, last walk | the row; `rc status` walk line |
| rerun causes | counted by hand (49 / 20 / 14) | RERUN RATE hover, per walk and project | `WalkRecord::causes` |
| seconds between end and landed with nothing shown | 2,314 s over ten cards | shown as `Land` rows | queue row; WALL / LANDED hover |
| longest silent slot wait | 12 min and more (limit 15) | a line every 60 s, a `SlotWait` span on the card | `rc hold` stderr; phase strip |
| cost of runs that did not land | by hand: 87% on four cards | COST / LANDED hover; report rows | `WalkRecord::usd_not_landed` |
| first-run pass of look-back cards | by hand: 0 of 3 | on the badge | `yield_of` |
| a ready card with no reason shown | every one | none | `Why` on every queue row; `rc status` |
| frames a second while a run is live | 25, each cloning the roadmap per project | 10, no clone, no `block_on` under `App::ui` | debug badge `frame p95 · fps` |

The one measure I would watch to judge the whole milestone: how often the person opens a
project tab to find out what happened. The dashboard should answer it. The since-band card
writes a `Note` event on each open, and the count per hour of live walk should fall.


# Part 2. Roadmap

Twenty-four cards, in the order they build. Cards 1 to 3 need nothing before them. The window
cards name new modules so they overlap as little as `main.rs` allows. At the last walk's
$0.77 a landed card this is under $20 if most land on the first run, and three to four times
that if they rerun like the four cards in section 6; the acceptance commands are filtered to
one crate each so no check is the 1,200 s workspace run.

## Observability: the window reports verdicts, causes and waits
  objective: The window, the feed and `rc status` say for every run what happened and why, for every open card why it waits, and for every walk what it cost per project, from one typed event log instead of prose parsed by substring; evidence and design in docs/fable/observability.md Part 1.

### The mark on a run is true

- The activity log ends a run with the mark and cause of its outcome {c1}
  objective: A run that returns Ok always writes `◆end ✓` (crates/ra-console/src/agent.rs:1118 and :1122) even when its outcome line starts with ✗, so the window drew about 70 failed runs as passed in 46 hours (activity log 321 ✓ and 30 ✗ against 100 `end ✗` on the feed; activity.log:14327-14328 against pair.log:1096 for #241). Take the mark from the first char of the outcome's first line and put the cause words after it, as `◆end ✗ check timed out: <command> · $0.12`. Put the choice in a small pure function so it has a test.
  acceptance: `cargo test -p ra-console -q end_marker` prints `test result: ok` with at least 3 passed, among them tests that a needs-revision outcome gives a marker starting `◆end ✗`, that an implemented outcome gives `◆end ✓`, and that the cost stays on the line.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- Toasts belong to the window, show on every view and expire {c2}
  objective: Toasts are drawn only from `Workspace::draw` (apps/rc-gui/src/main.rs:4185), so the dashboard and the report show none, and `retain(|t| t.failed || …)` (main.rs:2232-2262) keeps a failed toast for ever, so a tab opened later shows a stale stack. Move the toast list from `Workspace` to `App`, have each tab hand its newly ended runs to it (keep `raise_toasts` but push to the shared list with the project name), draw the stack from `App::ui` over whichever view is up, and expire a failed toast after 30 s and a passed one after the existing `TOAST_FOR`. Put the expiry rule in a pure function `toast_alive(failed, age)`.
  acceptance: `cargo test -p rc-gui -q toast_` prints `test result: ok` with at least 3 passed, among them that a failed toast is alive at 29 s and gone at 31 s, and that a passed toast is gone after `TOAST_FOR`.
  files: apps/rc-gui/src/main.rs apps/rc-gui/src/tests.rs

- rc check refuses an acceptance command with a hole in it {c1}
  objective: The last look-back pass planned #245 with the check `cargo test -p <review-crate> relocate_points`; it ran and failed in 50 s (pair.log:1074, 1078). `is_placeholder` (crates/ra-roadmap/src/tree.rs:261-266) only catches a field that is one bracket from end to end. In `definition_of_done` (tree.rs:245-257) add a fault `#id: the acceptance command has a placeholder <word>` when a backticked span holds `<` letters, digits or dashes `>` with no space inside; a shell redirect such as `< file` or `2>&1` must not trip it.
  acceptance: `cargo test -p ra-roadmap -q command_with_a_hole` prints `test result: ok` with at least 2 passed, one where a cargo test command whose crate is an angle-bracketed word is a fault and one where a command with an input redirect from a file and one with a stderr redirect are clean.
  files: crates/ra-roadmap/src/tree.rs crates/ra-roadmap/src/tree/tests.rs

### One typed event log

- events.rs holds the Event record, its JSON line and its prose line {c2}
  objective: Four readers parse the prose of var/pair.log by substring and get it wrong (apps/rc-gui/src/metrics.rs:97 takes the first " in " and loses the input on 62 lines whose title holds those words). Add a new module `events` in ra-console with `Event { at, card, kind }`, the `Kind`, `Phase`, `Outcome` and `Cause` enums as in docs/fable/observability.md section 2 (serde, `tag = "kind"`, snake_case), `emit(root, card, kind)` that appends one JSON line to `var/events.jsonl` and, for every kind but `Window`, the line `prose(&event)` through `pair::event`, and `read(root) -> Vec<Event>` that skips lines it cannot parse. No call site changes in this card. Add `pub mod events;` to the crate root.
  acceptance: `cargo test -p ra-console -q events::` prints `test result: ok` with at least 5 passed, among them a round trip of every `Kind` through its JSON line, that `emit` of a `Start` writes one line to each of events.jsonl and pair.log in a temp dir, that a `Window` event writes none to pair.log, and that a torn last line is skipped by `read`.
  files: crates/ra-console/src/events.rs crates/ra-console/src/events/tests.rs crates/ra-console/src/lib.rs

- A run's start, review, end, landing and send-back are written as events with the cause {c3}
  objective: `end ✗ … needs revision` names no cause, `end ✓ … implemented` is followed by a conflict send-back four times in the last hundred lines (pair.log:1004-1005, 1048-1049, 1062-1063, 1082-1083), and a rerun is a plain `start`. Replace the `pair::event` calls at the run start (crates/ra-console/src/agent.rs:1046-1062), the review lines (:1455, :1464), the end (:1116-1128), the landing lines (:1587, :3838, :3959) and `pair::sent_back`/`parked`/`held` (crates/ra-console/src/pair.rs:29-56) with `events::emit` of `Start`, `Review`, `End`, `Landed`, `SentBack` and `Parked`, each with the `Cause` the code already knows at that point (failed check and its command, timeout, lint, review, conflict, merged check, main red, turns, refused, no verdict). The prose line drops `0 tool call(s)` and keeps the words existing feed readers match (`start #`, `end `, `review `, `landed #`, `rerun #`, `waits #`, `parked #`) so nothing else changes yet.
  acceptance: `cargo test -p ra-console -q run_events` prints `test result: ok` with at least 4 passed, among them that a failed check ends with `Outcome::SentBack(Cause::Check { .. })` carrying the command, that a timeout sets `timed_out`, that a conflict after an implemented end writes a `SentBack` with `Cause::Conflict`, and that the prose of an `End` still starts with `end ✓` or `end ✗` and holds no `tool call`.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/pair.rs crates/ra-console/src/events.rs crates/ra-console/src/agent/tests.rs

- Phase changes are written as events, the slot wait with its card {c3}
  objective: A live run shows a clock and a step count. #241 spent 1,200 of 1,246 s in a timed-out check, landing takes 135 to 438 s (median 209 s over ten cards) with nothing shown, and the 30 `wait:` lines of the feed name no card (crates/ra-console/src/agent.rs:2097-2103). Emit `Kind::Phase` with the card id at: run start (`Work`), `hold_build` entry (`SlotWait`, detail = slots busy and of, or the holder of the cargo target from :2059) and exit (back to the phase before), each acceptance command start (`Check`, detail = the command), lint (`Lint`), the review call (`Review`, detail = the model), catch-up and squash-merge (`Merge`), and entry to the landing queue after the end (`Land`). `hold_build` takes `card: Option<i64>` as a new argument at its six call sites (agent.rs:3571, :3804, :4159, :4194, :5439, :6546; the ones that check main pass `None`). A phase event writes no prose line unless it is a `SlotWait` longer than 60 s, which writes `#id waits for a build slot · N of M busy`.
  acceptance: `cargo test -p ra-console -q phase_events` prints `test result: ok` with at least 3 passed, among them that a held build emits `SlotWait` with the card id and then the earlier phase again, that a check emits `Check` with its command as detail, and that a short slot wait adds no line to pair.log.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/events.rs crates/ra-console/src/agent/tests.rs

- rc hold says every minute what it waits behind and writes its slot wait as an event {c2}
  objective: #213's worker ran `rc hold -- cargo test`, saw nothing for over 12 minutes and ended its session with no test result (var/retro.log line 3), which is a rerun. `agent::hold` (crates/ra-console/src/agent.rs:2279-2304) prints its wait once (`said`, :2295-2302) and is silent until `HOLD_WAIT`. Print the line again every 60 s with the elapsed time and what holds the slots, from `hub::build_slots()` (crates/ra-console/src/hub.rs:1078), as `rc hold: 3:00 waiting · 8 of 8 busy · oldest #215 for 4:10 · runs unguarded at 15:00` (`HOLD_WAIT` is 15 minutes, agent.rs:2072); and when the directory is a card worktree (`var/worktrees/<id>` under a project root) emit `Kind::Phase { SlotWait }` for that card on entry and `Work` on getting the slot. Put the line in a pure `hold_line(elapsed, busy, of, oldest, limit)`.
  acceptance: `cargo test -p ra-console -q hold_line` prints `test result: ok` with at least 3 passed, among them that the line names the elapsed time, the busy count and the oldest holder, and that a worktree path under var/worktrees gives its card id while a project root gives none.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

- Walks, look-back passes and window starts are events, and the window lines leave pair.log {c2}
  objective: 201 of the feed's 1,096 lines (18%) are `window up`/`window down`, and they glue the metrics row's sessions together. The look-back pass leaves one sentence and no record of the cards it planned (crates/ra-console/src/lookback.rs:54-70). The feed has no line for a walk's start at all (the walk writes only its digest, crates/ra-console/src/agent.rs:2683), which is why the row guesses sessions from gaps. Emit `WalkStart` beside that digest write and `WalkStop { why, ran, landed }` where the walk writes `walk …` today (agent.rs:2695, :4806), `Lookback { evidence, planned, usd }` with the ids the pass added, and `Window { up, build, pid }` from `pair::window_up`/`window_down` (crates/ra-console/src/pair.rs:191-253), which stop writing to pair.log and count `restarts in 24 h` from the `Window` events of events.jsonl. Keep the `adopted #…` information as a `Note` event.
  acceptance: `cargo test -p ra-console -q window_events` prints `test result: ok` with at least 3 passed, among them that `window_up` adds no line to pair.log, that the restart count over 24 h is read from events.jsonl, and that a look-back pass emits the ids it planned.
  files: crates/ra-console/src/pair.rs crates/ra-console/src/lookback.rs crates/ra-console/src/agent.rs crates/ra-console/src/pair/tests.rs

- events::Board folds the log into runs, live runs, walks and passes, and Tail follows it by offset {c3}
  objective: Every reader needs the same answers: which attempt this is, what the last one died of, how long each phase took, what a walk cost. Add `events/board.rs` with `Board`, `RunRecord`, `LiveRun`, `WalkRecord`, `Pass` and `Yield` as in docs/fable/observability.md sections 2 and 7, `Board::apply(&mut self, &Event)` (a `Start` counts `nth` and copies the previous run's cause to `LiveRun::before`; a `Phase` closes the open span; an `End` moves the run from `live` to `runs` and, when implemented, into `landing`; `Landed`/`SentBack`/`Parked` refine the last run's outcome; walk totals and `causes` are kept as events arrive), `yield_of(pass, board)`, and `Tail { offset, board }` with `poll(root) -> bool` that reads only the bytes added since the last poll and leaves a torn last line for the next one (the shape of `pair::Follower`, crates/ra-console/src/pair.rs:120-161).
  acceptance: `cargo test -p ra-console -q board_` prints `test result: ok` with at least 6 passed, among them that three runs of one card read back as nth 1, 2, 3 with causes check, conflict and none and outcome landed on the third, that phase spans sum to the run's seconds, that a walk's `usd_not_landed` is the cost of its runs that did not land, that `yield_of` counts first-run landings of planned cards, and that `poll` after an append applies only the new events.
  files: crates/ra-console/src/events.rs crates/ra-console/src/events/board.rs crates/ra-console/src/events/tests.rs

### Why a card waits

- hub::why gives every open card and every project one reason {c3}
  objective: `hub::waiting_of` (crates/ra-console/src/hub.rs:489-524) explains three states by searching feed sentences; a ready card that does not run and a project with no run have no reason anywhere, and the `#N waits behind #M` line of #237 is read by nothing. Add `hub/why.rs` with `Why`, `ProjectWhy`, a small `Slots { runners: u32, busy: u32, walking: bool }` the caller fills from `allocation::runners` and the board's live runs, `why(nodes, board, slots, id)` and `project_why(nodes, board, slots)` as in docs/fable/observability.md section 4, with `short()` and `line()`. Order of the rules for one card: live run, landing queue, last run's outcome (`Again`, `Parked`, `Yours`), unaccepted waits-on, an overlapping live card (same test the walk uses for #237), no walk or walk stopped (from the board's last `WalkStop`), no free runner, else `Next`. `waiting_of` keeps its signature and is rebuilt on `why` so the callers stand.
  acceptance: `cargo test -p ra-console -q why_` prints `test result: ok` with at least 8 passed, one for each of `Running`, `Landing`, `Again`, `Yours`, `WaitsOn`, `Behind`, `WalkStopped` and `Next`, each built from a small roadmap and a board made of events.
  files: crates/ra-console/src/hub.rs crates/ra-console/src/hub/why.rs crates/ra-console/src/hub/tests.rs

- rc status prints each live run's phase and each waiting card's reason, and rc why prints one card's runs {c2}
  objective: `rc status` prints a header, one line, the walk's stop and the three-state waits (apps/rc/src/main.rs:657-679); `hub::status_with` reads the whole feed three times for it (crates/ra-console/src/hub.rs:598-625). Build one `Board` per call from events.jsonl and print: per live run `#id run N · phase mm:ss · detail`, per card in the landing queue its age, the `ProjectWhy` lines (walk or why idle, yours with the command, next), and the last walk as `walk HH:MM to HH:MM · R runs · L landed · K reruns (causes) · $X · $Y a landed card`. Add `rc why <id>`: the card's `Why::line()`, then one line per run with model, turns, duration, cost and outcome, then the cost that did not land. Keep the pure formatting in apps/rc/src/status.rs.
  acceptance: `cargo test -p rc -q why_lines` prints `test result: ok` with at least 3 passed, among them that a card with runs check-failed, conflict, landed prints three run lines and `of which $2.95 did not land` for costs 2.54, 0.41 and 0.30, and that a live run's line names its phase.
  files: apps/rc/src/main.rs apps/rc/src/status.rs apps/rc/src/tests.rs crates/ra-console/src/hub.rs

### The window reads and does not compute

- Sink lends a run instead of cloning it and forgets old runs {c2}
  objective: `Sink::run` clones a `Run` with all its steps (crates/ra-console/src/logs.rs:297-299); the window calls it per live card per frame (apps/rc-gui/src/console.rs:123) and once more per frame in `raise_toasts` (apps/rc-gui/src/main.rs:2216-2227), 25 times a second, and the `runs` map is never pruned while lines are capped at 10,000 (logs.rs:351-354). Add `Sink::with_run(id, |&Run| -> R) -> Option<R>`, drop ended runs beyond the newest 200, and keep a run's newest 2,000 steps with a count of the dropped ones in a new field `earlier`. Move the callers in console.rs and main.rs to `with_run`; `draw_latest_run` stops cloning each run line's text (console.rs:84-86). The tests go in the test module at the foot of logs.rs.
  acceptance: `cargo test -p ra-console -q sink_` prints `test result: ok` with at least 3 passed, among them that the 201st ended run evicts the oldest and a live run is never evicted, that a run with 2,500 steps holds 2,000 and reports 500 earlier, and that `with_run` on an unknown id is `None`.
  files: crates/ra-console/src/logs.rs apps/rc-gui/src/console.rs apps/rc-gui/src/main.rs

- A Snapshot holds what a project's dashboard card draws, and a builder makes it {c3}
  objective: Per frame and per project the dashboard clones the roadmap and recomputes walk order, depths and waiting (apps/rc-gui/src/main.rs:4226-4279, :4409) and once a second reads the whole feed through `hub::waiting` (:4241-4251). Add `apps/rc-gui/src/snapshot.rs` with `Snapshot { version, nodes: Arc<Vec<Node>>, order, depths, why, project, board: Arc<Board>, changes }` and `build(nodes, gambits, board, slots, changes, prev) -> Snapshot` that reuses the parts of `prev` whose input is the same `Arc`; the order is `ra_console::agent::walk_order(nodes, gambits, tight)` (crates/ra-console/src/agent.rs:3077) called once per build. No call site changes in this card beyond `mod snapshot;`.
  acceptance: `cargo test -p rc-gui -q snapshot::` prints `test result: ok` with at least 3 passed, among them that the order equals `walk_order` on the same nodes, that every open card has a `why` entry, and that a build with the same nodes `Arc` as `prev` returns the same `order` allocation.
  files: apps/rc-gui/src/snapshot.rs apps/rc-gui/src/snapshot/tests.rs apps/rc-gui/src/main.rs

- A refresher thread loads the roadmap and builds snapshots, and frames read an Arc {c3}
  objective: `Workspace::tick` runs every frame for every tab, also on the dashboard (apps/rc-gui/src/main.rs:991-995), and on a roadmap change parses all of rhipper.md with `rt.block_on(store.load())` on the UI thread (:2127-2210). Start one thread per window that, for each open project every 250 ms, stats the roadmap, polls the project's `events::Tail` and, when either moved, loads the nodes and history, builds a `Snapshot`, stores it behind the tab's `Arc<Mutex<Arc<Snapshot>>>` and calls `ctx.request_repaint()`. `tick` keeps `quit_requested`, `walk_requested`, `tail_activity_at` and the toasts; `dashboard_card`, the queue and `card_view` (:3595) read `self.snap()` in place of `self.roadmap.clone()`, `walk_order`, `tree::depths`, `last_changes.clone()`, `waiting_on` and `hub::waiting`. The refresher's loop body is a function `refresh_once(&mut State) -> bool` with a build counter so a test can drive it.
  acceptance: `cargo test -p rc-gui -q refresher_` prints `test result: ok` with at least 3 passed, among them that `refresh_once` with no file change builds nothing, that touching the roadmap file builds one snapshot with a higher version, and that an appended event line does the same.
  files: apps/rc-gui/src/snapshot.rs apps/rc-gui/src/snapshot/tests.rs apps/rc-gui/src/main.rs

- The ledger, the token gauges and the diff refresh on the refresher thread {c3}
  objective: Still on the UI thread after the card before: three SQLite queries per tab in `TokenGauges::tick` (apps/rc-gui/src/main.rs:546-580), five ledger queries, the gotchas catalogue and `retro::recent` in `refresh_ledger` (:2025-2071), and `git diff HEAD` in `refresh_diff` (:2108-2123). Add `ledger: Arc<Ledger>` and `diff: Arc<String>` to `Snapshot`, fill them on the refresher when `var/usage.db`'s mtime or the board moved (the diff when a run of that project ended), and have the three functions read the snapshot. After this card no `block_on` is reachable from `App::ui`.
  acceptance: `cargo test -p rc-gui -q ledger_snapshot` prints `test result: ok` with at least 2 passed, among them that the ledger part is rebuilt when the usage file's mtime moves and reused when it does not.
  files: apps/rc-gui/src/snapshot.rs apps/rc-gui/src/snapshot/tests.rs apps/rc-gui/src/main.rs

- The window repaints on change, at 10 fps while a run is live and at 1 fps without focus {c1}
  objective: `request_repaint_after(40 ms)` while any run is live (apps/rc-gui/src/main.rs:1364-1365, :4188-4189) is 25 frames a second for as long as any of three projects walks. Add a pure `cadence(live, focused) -> Option<Duration>` (100 ms live and focused, 1 s live and unfocused, `None` idle), use it at both sites, repaint at once when a tab's sink version or snapshot version moved since the last frame, and add `frame p95 N ms · M fps` to the debug badge of `dashboard_status` (:1069-1172) from a ring of the last 120 frame times.
  acceptance: `cargo test -p rc-gui -q cadence_` prints `test result: ok` with at least 3 passed, one for each of the three cases.
  files: apps/rc-gui/src/main.rs apps/rc-gui/src/tests.rs

### The views

- A worker's turns and context so far reach its run card {c2}
  objective: #235's first run went 65 turns and 28 minutes before it failed and nothing on screen counted. `TurnTally` (crates/ra-providers/src/claude_code.rs:556-575) books each turn's usage as the stream arrives, and the read loop publishes tool steps with `tracing::info!(target: "ra::progress", …)` (claude_code.rs:408-413). Give `TurnTally` a pure `line(&self) -> String` returning `◇ N turns · Xk` (X the last turn's input in thousands) and publish it on the same target each time `note` books a new turn id, not on each content block of the same turn. Have `Sink::progress` (crates/ra-console/src/logs.rs:133-174) fold a line starting `◇` into new `Run` fields `turns: u64` and `context: u64` in place of pushing a step. The Sink test goes in the test module at the foot of logs.rs.
  acceptance: `cargo test -p ra-console -q turn_line` prints `test result: ok` with at least 2 passed, among them that the line for 12 turns and 61k sets turns 12 and context 61000 on the live run and adds no step; and `cargo test -p ra-providers -q turn_progress` prints `test result: ok` with at least 1 passed, that a stream of three turns, one of them in two events, gives the line for 3 turns.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs crates/ra-console/src/logs.rs

- A run card shows its phase strip, run number, the last cause and a clock that survives a restart {c3}
  objective: The run card's status is `clock · n steps` (apps/rc-gui/src/console.rs:323-402) and the clock is the window's `Instant` at attach, reset by each of about 30 restarts a day. Add `apps/rc-gui/src/phase.rs` with `strip(live: &LiveRun, now) -> Vec<(Phase, f32)>` (span shares), `phase_color(Phase)` from the theme (accent work, dim slot wait, warn check and lint, light review, online merge and land, error_text failed; no new colours), and `status_line(live, run_turns, run_context, last_step_age, now)`. `run_title` draws `run N`, `after ✗ <cause>` when `before` is set, the strip with the existing `pill_bar`, the clock from `LiveRun::started`, turns and context, and `last step N s ago` in warn past 120 s of `Work`. The breathing border takes the phase colour.
  acceptance: `cargo test -p rc-gui -q phase::` prints `test result: ok` with at least 4 passed, among them that strip shares sum to 1, that a run started 600 s ago reads `10:00` whatever the window's uptime, that `before = Check` renders `after ✗ check`, and that a 130 s quiet `Work` phase is flagged.
  files: apps/rc-gui/src/phase.rs apps/rc-gui/src/phase/tests.rs apps/rc-gui/src/console.rs apps/rc-gui/src/main.rs

- Queue rows carry their reason and an idle project says why it is idle {c3}
  objective: A project with no live run is one header line (`if live.is_none() { return; }`, apps/rc-gui/src/main.rs:4377-4379) and a ready card shows nothing about why it is not running; the header counts projects with a run where it says running (:1093-1095). From the snapshot: end each queue row with `Why::short()` in dim (`yours` in warn) with `Why::line()` on hover; give a card in the landing queue a row `#id landing · mm:ss`; draw an idle project as three lines (walk stopped when and why, or no walk; yours with the command; next); and make the header `N projects · R running · L landing`. Put the row and idle-line text in pure functions `queue_reason(&Why)` and `idle_lines(&ProjectWhy, now)`.
  acceptance: `cargo test -p rc-gui -q idle_lines` prints `test result: ok` with at least 3 passed, among them that a stopped walk gives a first line starting `walk stopped`, that a card waiting for its owner prints its `rc set` command, and that `queue_reason` of `Behind` names the other card.
  files: apps/rc-gui/src/main.rs apps/rc-gui/src/tests.rs

- The metrics row counts by walk and by project from the Board {c3}
  objective: The row merges all projects into gap-cut sessions (apps/rc-gui/src/metrics.rs:65-126), loses input on 62 lines (:97, :179-183) and double-counts failed reviews (:109, :171), so Phase AA's acceptance, rerun under 30% and pass over 70% on the next agent walk, cannot be read from it. Rebuild `metrics.rs` on `WalkRecord`: tiles FIRST-RUN PASS, RERUN RATE, REVIEW PASS, COST / LANDED, TOKENS / TURN, WALL / LANDED with the formulas and hovers of docs/fable/observability.md section 6 (rerun causes on hover; cache read and read-in share as the hover of TOKENS / TURN), the last 10 walks as the sparkline, and a scope chip ALL or one project. Delete `sessions`, `number_after` and `SESSION_GAP`. The row reads boards from the snapshots, no file.
  acceptance: `cargo test -p rc-gui -q metrics` prints `test result: ok` with at least 5 passed, among them that a walk of 7 runs over 4 cards with 3 reruns reads 43% rerun rate, that a review counted once gives 2 of 3 as 67%, that the scope of one project ignores another project's walks, and that cost per landed for 3.08 over 4 is 0.77. The tests replace the ones in the test module at the foot of metrics.rs.
  files: apps/rc-gui/src/metrics.rs apps/rc-gui/src/main.rs

- A since-you-looked band on each project card, with a seen mark {c3}
  objective: Nothing in the window says what changed since the last look; the report lists landed cards only and toasts are the one push. Add `apps/rc-gui/src/since.rs` with `Since { landed, landed_usd, failed_first, failed_usd, conflicts, yours, planned }`, `since(board, why, seen) -> Since` counting runs with `ended > seen`, `read_seen(root)`/`write_seen(root, at)` on `var/seen`, and `band(ui, &Since)` drawing the counts as `badge_marked` in the theme (landed online, failed error_text, yours warn) with the cards' sentences on hover and a `seen` button. Draw it under each project's header on the dashboard, idle projects too; opening the project's tab writes the mark and emits `Kind::Note { text: "opened" }`, so the number of times the dashboard was not enough can be counted per day. `yours` does not clear on seen.
  acceptance: `cargo test -p rc-gui -q since::` prints `test result: ok` with at least 4 passed, among them that runs ended before the mark are not counted, that a first-run failure adds its cost to `failed_usd`, that `yours` counts whatever the mark, and that the mark round-trips through a temp dir.
  files: apps/rc-gui/src/since.rs apps/rc-gui/src/since/tests.rs apps/rc-gui/src/main.rs

- The report lists every finished card with verdict, cause, runs, cost not landed and wall time {c3}
  objective: `rebuild_landed` (apps/rc-gui/src/main.rs:1458-1493) keeps implemented and accepted cards only, so the report cannot answer where the money went: on #235, #236, #207 and #238, $8.61 of $9.93 was spent on runs that did not land. Move `report_view`, `Landed` and `landed_bubble` (:1371-1562) to a new `apps/rc-gui/src/report.rs`; build rows from the boards: one `Row { project, card, title, outcome, runs, turns, secs, usd, usd_not_landed, first_cause, sentence }` per card with a run ended in the chosen span (since seen, today, this walk; the existing `choice` widget), grouped as landed, sent back, yours, with a LOOK-BACK group for cards a pass planned. The sentence is the reviewer's first line for a landed card and the cause for the others. A row opens the card.
  acceptance: `cargo test -p rc-gui -q report::` prints `test result: ok` with at least 4 passed, among them that a card with three runs gives one row with `runs` 3 and `usd_not_landed` 2.95 for costs 2.54, 0.41, 0.30, that a card with only a failed run is in the sent-back group with its cause as the sentence, and that the span filter drops older runs.
  files: apps/rc-gui/src/report.rs apps/rc-gui/src/report/tests.rs apps/rc-gui/src/main.rs

- The look-back badge shows the yield of the last pass {c2}
  objective: The badge reads `look-back n/10` (apps/rc-gui/src/main.rs:4337-4346) while the last pass's first three cards all failed their first run (#240, #241, #245; pair.log:1071-1078). From the board's last `Pass` and `yield_of`: badge text `look-back n/10 · last K of M` (K first-run landings of M planned cards run so far), warn colour when K is under half of M and M is at least 2, hover listing each planned card with its outcome and cause and the pass's evidence count and cost. Pure text in `lookback_badge(since, every, Option<&Yield>) -> (String, bool)`.
  acceptance: `cargo test -p rc-gui -q lookback_badge` prints `test result: ok` with at least 3 passed, among them `look-back 7/10 · last 0 of 3` flagged, `look-back 2/10 · last 3 of 4` not flagged, and no pass yet giving the plain `look-back 2/10`.
  files: apps/rc-gui/src/main.rs apps/rc-gui/src/tests.rs

- The card view shows the card's runs and the whole verdict {c2}
  objective: `refresh_verdict` shows the first line of the last handoff's verdict (apps/rc-gui/src/main.rs:2096-2104) and the card view has no history of attempts. In `card_view` (:3571) add a RUNS card under the hero: one line per `RunRecord` (run number, model, turns, duration, cost, outcome and cause, coloured by `phase_color` of the outcome) and a foot line with the cost that did not land; show the verdict in full in the existing bubble, folded past eight lines, with git's `LF will be replaced by CRLF` warning lines filtered out by a pure `verdict_lines(&str) -> Vec<&str>`.
  acceptance: `cargo test -p rc-gui -q verdict_lines` prints `test result: ok` with at least 2 passed, among them that CRLF warning lines are dropped and the other lines keep their order.
  files: apps/rc-gui/src/main.rs apps/rc-gui/src/tests.rs
