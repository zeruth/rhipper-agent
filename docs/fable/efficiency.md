# Token, turn and context efficiency of a worker run

Review of 2026-11-13. Read from `var/pair.log` (1,096 lines, 269 run ends, 126 reviews), the
stream files of the last walk (`var/worktrees/<id>/var/runs/last.jsonl`), the system prompt a
worker was really given (`var/worktrees/245/var/runs/system-19396.md`), the retro logs, and the
code at `overseer/history` (787be7f). Line numbers are of that commit.

The short version:

1. A run's cost is **turns × context**, and context only grows. Every token a worker reads on
   turn 5 is read again on every later turn. The lever is therefore what happens before the
   first edit, and how many turns are spent on things that are not the work.
2. Two things eat most of the turns that are not the work: **finding where the change goes**
   (0.8–1.5M input tokens before the first edit on a c3 card in `agent.rs`) and **waiting for
   a build** (2–5 turns per build, each at the full context; a card's first build is cold,
   6.5–15 minutes, and starts after its last edit). Eight of the ten look-back lines the
   last walk's workers wrote are about a build they waited on or a card that named something
   that does not exist.
3. The fixed floor under every turn is **17–21k tokens** before the worker has done anything;
   9.5k of it is tool schemas, of which the streams show four tools never called. A rerun in
   a large file starts at **45–50k**, because it is handed 800 lines the last run read.
4. A resumed session is priced as "93% cached" in the feed and costs ten times a fresh one.
5. Review, summary and the look-back pass are about 3% of the tokens. Moving them to a cheaper
   model is not the lever. Moving the **reading** to a context that is thrown away is.

Phase AA (#234–#238) and the look-back cards #240–#245 are taken as landed; nothing below
repeats them.

Scope of the evidence: the feed, streams and retro logs are the agent project's. The feeds of
`rhipper-asm` and `runelite` were not readable from this session, so their numbers here are
the ones in the brief (50–110k a turn, 85–97% rerun). Every change below is in a shared crate
(`ra-context`, `ra-providers`, `ra-console`, `rc`), so all three projects get it; the measures
of D8 are what will show whether the larger files of the other two gain more, as I expect.

---

# Part 1: findings and design

## 1. What a run carries today

Measured on the first turn of fresh runs in the last walk (pid 19396):

| Part | Tokens | Built at | Paid |
|---|---|---|---|
| Tool schemas and the harness's own scaffolding (11 tools named, 10 loaded) | 9,465–9,558 | `WORKER_TOOLS`, `crates/ra-providers/src/claude_code.rs:32` | cache read, every turn of every run |
| System prompt file (92 lines in the #245 run) | ≈ 4.5k | `harness_messages`, `crates/ra-console/src/agent.rs:5911`; `system_of`, `claude_code.rs:258`; written by `write_system_prompt`, `claude_code.rs:985` | cache write on turn 1, cache read after |
| Task prompt, first run | 3–8k | `TaskContext::prompt`, `crates/ra-context/src/lib.rs:1018` | same |
| Task prompt, rerun with no read set | 7–19k | `task_text`, `agent.rs:5960`: brief, outline, review, then `rerun_text`, `agent.rs:8273` | same |
| Task prompt, rerun after a run that read much | ≈ 30–35k | the same, led by the last run's regions: up to 800 lines (`REGIONS_ROOM`, `lib.rs:837`) | same |
| Transcript | +1–1.5k a turn while editing, +5–7k per 400-line read (`LONG_READ`, `agent.rs:40`) | the harness | cache read on every later turn |

Turn 1 in the streams: 9.5k cache read plus a cache write of 7,752 (#240, first run), 10,400
(#215), 12,088 (#245, rerun), 24,005 (#213, conflict rerun). So the floor of a first run is
**17–21k**; a conflict rerun started at 33.5k (#236 now replaces it with a conflict pass);
and a rerun of a c3 card in `agent.rs` starts at **45–50k** (F9).

Two counts share the word "turn" on the end line. `turns 65` is the harness's count, which
steps once per tool call (the #215 stream: `num_turns` 32, 31 tool calls). `avg 88k tokens a
turn` is over the model calls the stream showed (`average_turn`, `agent.rs:7325`): 3.88M ÷ 88k
is 44 calls, not 65. The line also says `0 tool call(s)` on every Claude Code run. Every
number below is quoted as the feed printed it.

The system prompt holds, in order: the `SYSTEM` paragraph (`agent.rs:8499`); the project
context cut to the card's crates (`cut_to`, `lib.rs:64`) with the standing direction; four
"about this machine" bullets; five lines learned by other projects; two digests of other
projects (`digest_block`, `agent.rs:5972`); the retro prompt; the commit prompt
(`COMMIT_PROMPT`, `agent.rs:3330`); the allowed shell shapes (`ra_gotchas::shape::block()`,
about 17 lines). In the #245 run the five learned lines were runelite's `third-party/jdk8u`
(twice), asm's `%TEMP%` and asm's rhtest executable: none applies to a card of the agent.

The task prompt holds: regions of the last run's read set (reruns only), the brief with one
line per ancestor, an outline per file of the card (`outline_files`, `lib.rs:495`, the first
60 items, `OUTLINE_ROOM`, `lib.rs:160`), the reviewer's points, and the last three handoffs
(`HANDOFFS_SHOWN`, `lib.rs:163`, 40 lines each, `REPORT_LINES`, `lib.rs:169`).

What bounds a run: `[worker] context = 160000` in `rhipper.toml:8` (a fresh restart on a
handoff past it, with a grace of 12 turns fresh and 2 resumed, `claude_code.rs:707–754`), no
turn cap, `--effort medium`.

## 2. Findings

### F1. The outline does not point at the work, so the first twenty turns are reading

`outline_marked` (`crates/ra-context/src/lib.rs:337–357`) prints the **first 60 items in file
order** and "… and N more":

```rust
let hidden = items.len().saturating_sub(OUTLINE_ROOM);
for (start, end, item) in items.iter().take(OUTLINE_ROOM) { … }
if hidden > 0 { let _ = writeln!(out, "         … and {hidden} more"); }
```

For `crates/ra-console/src/agent.rs` (about 8,600 lines, over 320 items) that is the constants,
the `Worker` builder methods, `doctor` and `probe`: lines 28–646. `var/context/213.md` shows
it: a card about review and handoffs got an outline that ends at line 646 with "… and 261
more". The outline cost about 2k tokens every turn and named nothing the card touches.

Three more defects in the same place:

- The outline is read from the **project root**, not the tree the worker acts in
  (`agent.rs:5867`, `ra_context::outline_files(&root, …)`). On a rerun the worktree's line
  numbers have moved.
- A row carries the start line only. A worker that wants one function must guess the length,
  and reads 400 lines.
- Only the card's `files:` are outlined. Retro of #219: "`rc check` handler lives in
  `apps/rc/src/main.rs`, but the outline only lists `crates/` … would have saved about eight
  searches".

The ranking the outline lacks already exists one layer down: `hint_of` (`agent.rs:885–924`)
picks the items whose names appear in the brief, but only after a long read has been refused
or made. The knowledge is spent on the repair, not on the prompt.

What it costs, from the feed (first runs of c3 cards in `agent.rs`, Opus):

| Card | Turns | Input | Read-in (to the first edit) | Avg a turn | Largest | Result |
|---|---|---|---|---|---|---|
| #235 | 65 | 3.88M | 1,491k | 88k | 130k | ✗ |
| #236 | 49 | 4.03M | 1,509k | 98k | 135k | ✗ |
| #238 | 47 | 1.86M | 843k | 58k | 77k | ✗ |
| #211 | 69 | 3.5M | 1,245k | — | — | — |

Read-in is 36–45% of the input, and it sets the level every later turn is paid at. Against
that, c2 cards in small files on Sonnet: #237 26 turns, read-in 163k, avg 36k, ✓.

It is not only the large cards. The tool calls of the seven worktree streams that hold a
whole run, counted to the first Edit or Write:

| Stream | Harness turns | Tool calls | Before the first edit | What they were |
|---|---|---|---|---|
| #213 (rerun) | 8 | 7 | 2 | 2 Grep |
| #215 | 32 | 31 | 3 | 1 Grep, 2 Read |
| #216 | 21 | 20 | 7 | 2 Grep, 4 Read, 1 Bash |
| #240 | 15 | 14 | 8 | 4 Grep, 1 Glob, 3 Read |
| #241 | 12 | 11 | 9 | 4 Grep, 4 Read, 1 Bash |
| #242 | 40 | 39 | 20 | 11 Grep, 9 Read |
| #243 | 30 | 29 | 19 | 8 Grep, 7 Read, 1 Glob, 3 Bash |
| all | | 151 | 68 (45%) | |

Of all 151 calls, 41 were Grep, 41 Read, 48 Edit or Write, 19 Bash, 2 Glob: **more than half
of what a worker does is look for the place**. In #241, a c2 card that changed two places,
nine of eleven calls were the search. The stream of the project root's own run
(`var/runs/last.jsonl`) shows 30 reads and greps before its first edit.

What the searches were, in #242: greps for words of the card (`merge-base|git rm`, `gate`,
`shape::judge`, `Command::Hook`), then six reads of `apps/rc/src/main.rs` in pieces of 20 to
90 lines at offsets the greps gave. The first two of the six were of the **project root's**
copy (offsets 300 and 915); the next two read the same places again from the worktree
(offsets 315 and 925). The worker has both trees in reach; what sent it to the root's I did
not trace, and it is one stream of eight. Every one of those lookups is a name-to-range
question a parser answers in milliseconds.

The prompt already tells the worker what to do: "Files: read the regions the outline points
at, not whole files" (`lib.rs:1029`). The outline points at nothing, so the instruction
cannot be followed.

The workers say the same in their own look-backs, ten times over in the archived retro logs
(#49, #50, #66, #67, #75, #76, #77, #85, #153, #160): "a card line naming `<function>` in
`<file>` would have saved the reads".

### F2. A build costs two to five turns, each at the full context

In the #245 rerun stream the worker ran `cargo test -p ra-console -q relocate_points 2>&1 |
tail -n 25` with `"timeout":600000`. The harness answered "Command did not complete within its
120s timeout and was moved to the background". Then: `rc wait` (printed its usage), `sleep 1;
echo waiting`, a `tail` of the task file, `cargo test` again in the background. **Five of the
run's seven tool calls were the build and waiting for it.**

The same shape in #213 (`rc hold -- cargo test … | tail -25`, then a Read of the task output,
then `until [ -s … ]; do sleep 10; done` at 58k a turn) and #215 (two tests, a check, `rc
wait`, a Read of the task output). "Moved to the background" is in three of the eight
worktree streams as they stand (#242, #243, #245); in #242 and #243 every call after the
last edit is a shell call, four and five of them, around one build.

The retro logs carry 33 lines about it (17 and 16 in the two archives), and four of the ten
lines in today's `var/retro.log`:

- #215: "`rc wait` gave up twice at 540 s while the build ran".
- #213: "`rc hold -- cargo test` printed nothing for over 12 minutes, most likely waiting for
  a build slot, so the session ended with no test result".
- #242: "A cold `cargo build -p rc` took about 6.5 minutes in the worktree and went to the
  background."
- #243: "`cargo build -p rc` took 15 minutes on a cold worktree target, and the 120 s
  foreground limit moved it to the background".

**The first build of every card is cold.** A card builds in its own target,
`<root>/target/wt-<id>` (`shared_target`, `crates/ra-providers/src/claude_code.rs:1021–1046`),
so that two worktrees never compile against each other's copy (#176, #181). The directory is
empty when the card first runs: the worker's first build compiles every dependency of the
workspace again, 6.5 minutes alone (#242) and 15 with a second card building beside it
(#243). Nothing starts that build before the worker asks for it, which is after its last
edit, when the context is at its largest; and no timeout the worker can pass covers 15
minutes. For a c2 card whose change is twenty lines, the cold build is the run.

Why the 600 s the context file tells workers to pass does not hold: the pre hook answers a
rewrite with

```rust
"updatedInput": {"command": command}          // apps/rc/src/main.rs:998
```

Only `command` goes back. The worker's `timeout` (and `run_in_background`) are not in the
object, and the harness falls to its 120 s default. `crates/ra-gotchas/src` has no mention of
`timeout`, `run_in_background` or `updatedInput`: nothing else puts it back. This is the
reading that fits the stream (600000 asked, 120 s applied); the first card of the section
proves it with a test on the hook's answer before anything else is built on it.

A longer timeout alone does not close it. `rc hold` waits up to 15 minutes for a slot
(`HOLD_WAIT`, `agent.rs:2072`), says so once on stderr (`agent.rs:2295`), and only then
builds; a shell call lives ten minutes at most. A build that queues cannot answer inside the
call that started it, and the worker has no verb to come back to it: `rc wait` takes a pid or
a file the worker must find (#242: `cat` of the harness's task file, `rc wait bst410gi9`,
`tail` of the task file, for one `cargo test`), and gives up at 9 minutes (`WAIT_LIMIT`,
`agent.rs:2340`). That is #213's twelve silent minutes and #215's two waits that gave up.

Then the orchestrator compiles the same tree again for the card's check, on one of the eight
slots the waiting workers are queued for.

### F3. The floor is 17–21k tokens and a quarter of it is never used

- **Tools, 9.5k.** `WORKER_TOOLS` is `Bash,PowerShell,Read,Edit,Write,Glob,Grep,TodoWrite,
  WebFetch,WebSearch,NotebookEdit` for every card; the harness's init event lists ten of them
  (it has no `TodoWrite`). In the 151 calls of the seven worktree streams counted under F1
  there is no web call, no notebook call and no PowerShell call; the project root's own
  stream has one PowerShell call, its last. Two shells means two copies of the longest tool
  description, and `rhipper.toml` writes every allow rule twice (`rhipper.toml:16–45`).
- **Slash commands.** The same init event lists `rhipper-pair`, `deep-research`, `design`,
  `design-sync`, `dataviz` and more as slash commands of the worker's session. Whether their
  descriptions reach the context I could not tell from the stream; the turn-1 field (D8)
  with and without them will.
- **System prompt, 4.5k.** Other projects' learned lines and digests ride in every run whether
  or not they name anything this project has. (The shell rule stated three times is #241's.)
- **Brief.** Each ancestor adds a line: "Under #204 Phase Y: …" is a 100-token paragraph, and
  "Under #1 Rhipper Agent: no objective" says nothing (`var/context/213.md`).
- **Order.** The per-card cut of the context sits in the middle of the system prompt, so
  nothing after it can be shared between two runs of the same walk; the whole 7–12k is a
  cache write per run.

On a 13-turn c2 run at 21k a turn, the floor **is** the run.

### F4. A resumed session costs ten times a fresh one and the feed calls it cached

Short runs that resumed a session (before #210):

| Card | Turns | Avg a turn | Feed | Cost |
|---|---|---|---|---|
| #216 | 1 | 83k | 93% cached | $1.52 |
| #208 | 1 | 93k | 13% cached | $1.87 |
| #185 | 3 | 93k | | $3.80 |
| #205 | 3 | 86k | | $1.55 |
| #192 | 4 | 111k | | $1.66 |

Short runs that started fresh:

| Card | Turns | Avg a turn | Cost |
|---|---|---|---|
| #213 | 3 | 19k | $0.11 |
| #216 | 3 | 18k | $0.10 |
| #211 | 4 | 19k | $0.12 |
| #196 | 3 | 21k | $0.12 |

A session resumed after the provider's cache has expired pays a cache **write** on the whole
transcript. `TurnTally` keeps `cached` and `written` per turn (`claude_code.rs:556–701`) but
`tokens_line` (`agent.rs:7267–7277`) prints only the cached share, so the feed cannot show it.

#210 made reruns start fresh (`rerun_resume`, `agent.rs:8250`). Still resuming: the second
ask for a report (`agent.rs:5245–5280`; "report asked twice" on #196, #209, #215, #220, #233),
absolute cards, and any session inside `SESSION_LIFE` (`agent.rs:8078`, 24 hours, where the
cache lives minutes).

### F5. Nothing stops a long read-in but the 160k cap

The cap is above where every bad run lives (largest turns 130–135k on #235 and #236), and a
cap that fires throws the session away. Between "fine" and "capped" there is no signal to the
worker and none on the feed until the end line. The read set is handed on only when a run is
capped or fails its check (`agent.rs:5407`, `agent.rs:5704`).

Where the cap did fire while a run was still reading, the answer was to take the cap away: a
card whose last session hit it within 20 turns runs next with none (`capped_reading_in`,
`agent.rs:8213`; runelite #15 reached 161k at turn 7 reading two JDK classes). That keeps the
card moving and leaves the cause: the worker was not told which part of the files it needed.

### F6. One 8,600-line file is most of the agent's cards

`agent.rs` and `commands/tests.rs` are the files of most agent cards. A read of either is 400
lines at a time. After #237 (cards with overlapping files do not run at once) the size of the
file is also the limit on how many cards run in parallel. The same holds for any project with
a file like it; the agent has no tool to split one without a model writing every moved line
out again.

### F7. Cards name things that do not exist

Four of the ten lines in today's `var/retro.log`:

- #241: "The card named `rc handoff --dry-run` and `docs/context.md`, neither of which
  exists".
- #242: "The card named `crates/ (shell allowlist/gate)` and `.claude/settings.json`, which
  does not exist here, and the card said `rc gate`, which did not exist either."
- #243: "The card named only `crates/ (gate + gotchas)` and neither `rc gate` nor `rc
  gotcha-match` existed: naming `apps/rc/src/main.rs` on the card would have saved the
  search." (#242 and #243 are the two streams with 20 and 19 calls before the first edit.)
- #245: "The acceptance check named `<review-crate>` literally and failed before any test
  ran. Naming `ra-console` on the card would have avoided the rerun." A whole run and a
  rerun, for a placeholder a parser sees.

Archive, #201: "started on a card whose work was already committed … three reads to find that
out". Every one of these is a check a machine can make before the worker starts: a path that
is not in the tree, a name nothing defines and the card does not call new, a check command
that names no package.

### F8. The cheap roles are already cheap

Reviews are one call each, 7.5k–33k in: $0.024–0.127 on Sonnet today. The look-back pass on
`claude-fable-5-1` was 8,963 in, 3,610 out, $0.36 for eight cards. `summary`, `digest`,
`classify` and `fix` already go to the local server when it answers (`rhipper.toml:71–78`,
`crates/ra-providers/src/route.rs:30`). What is left to move is not a role that exists: it is
work the worker does on Opus at 90k a turn that a cheaper context could do: locating code,
and writing the report a run forgot.

The agent has the right organ for the first and does not use it. `investigate`
(`agent.rs:4928`) is a read-only run that leaves `var/briefs/<id>.md`, which the builder's
prompt opens on (`agent.rs:5947`). As built it cannot pay:

- it is off unless `[worker] investigators` is set (`router.rs:297`, default 0); the agent's
  `rhipper.toml` does not set it and the feed has no `brief #` line;
- it runs on **the implementer's own model** (`worker.for_card`, `agent.rs:4932`), so the
  reading costs what it would have cost the worker;
- it takes up to 40 turns (`INVESTIGATE_TURNS`, `agent.rs:4906`) for every ready card, small
  or large, whether or not the card already names its place;
- it is given the brief and the project context and **no outline at all** (`agent.rs:4937–
  4941`), so it starts colder than the worker does;
- its answer is a brief of up to 150 lines, carried on every turn of the builder.

### F9. A rerun in a large file starts at 45–50k, because its regions are counted in lines

A rerun's prompt opens with the regions the last run read (`regions`, `lib.rs:902`): 200
lines a region, 800 lines in all, in the order the last run first read them. The bound is in
lines and the order is the order of reading, not of use. The reruns of the three c3 cards:

| Card | Turns | Avg a turn | Largest | Cost |
|---|---|---|---|---|
| #236 | 3 | 47k | 47k | $0.34 |
| #235 | 6 | 49k | 50k | $0.41 |
| #238 | 9 | 48k | 51k | $0.43 |

A three-turn run whose average and largest are both 47k started at about 45k. Reruns with no
read set start at 18–21k (#213, #216, #211, #196 above, $0.10–0.12). The difference, 25–30k,
is 800 numbered lines of `agent.rs`, the first 800 the failed run happened to read, on every
turn of the rerun. The rerun block beside them already says what failed and what the
reviewer asked; the regions are not chosen by either.

The idea is right (these reruns passed in 3–9 turns) and the price is three to four times
what it needs to be. With a 69% rerun rate, this is the commonest prompt the agent writes.

## 3. The model

For a run of `T` turns with floor `F` and a transcript that grows by `g` a turn:

    input ≈ T·F + g·T²/2

A read of `r` tokens at turn `t` costs `r·(T − t)`: a 6k read at turn 5 of a 50-turn run is
270k tokens. A wait turn late in a run costs the whole context for nothing. So, in order:

1. **Fewer turns** (`T`): no waiting, no searching, no refused calls.
2. **Less read early** (`g` in the first turns): ranges, not 400-line pages; the right ranges.
3. **A smaller floor** (`F`): matters most on short runs, which are most runs.

## 4. The run to build

A c3 card in a large file, today and designed:

| | Today (#235, #236) | Designed |
|---|---|---|
| Turn 1 | 17–21k; a page of `agent.rs` | ≈ 14k; the named items are quoted in the prompt, the pointed ranges are read in one message |
| To the first edit | 15–20 turns, 0.8–1.5M tokens | 2–3 turns, under 100k |
| Edits | at 80–130k a turn | at 25–35k a turn |
| A build | 2–5 turns | 1 turn: one blocking call that prints the failures |
| Report | sometimes asked twice on the resumed session | written once; when missing, a cheap role writes it from the diff |
| Whole run | 47–65 turns, 2.7–4.0M in | ≤ 25 turns, ≤ 40k avg, ≤ 1.0M in |

A c2 card on Sonnet: today 12–26 turns at 21–46k; designed ≤ 10 turns at ≤ 20k.

### D1. The map: an outline that points (F1)

Replace "the first 60 items of each file" with a ranked set of pointers inside one budget for
the whole prompt.

```rust
// crates/ra-context/src/lib.rs
pub struct Pointer {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub item: String,      // the signature line, clipped
    pub why: Why,
    pub score: u32,
}
pub enum Why {
    Named,                 // the card's text names the item
    Changed,               // the last run changed it
    Failing,               // the last check's failure names it
    Test,                  // a test whose name holds the acceptance's filter
    Word(String),          // a word of the card in the item's name
    Caller(String),        // it calls a named item
    Scout,                 // a scout run pointed at it (D6)
}
pub struct Map {
    pub pointers: Vec<Pointer>,
    pub files: Vec<(String, usize, usize)>,   // path, lines, items not shown
    pub unresolved: Vec<String>,              // names of the card nothing defines
}
pub struct Ask<'a> {
    pub title: &'a str,
    pub objective: &'a str,
    pub acceptance: &'a str,
    pub points: &'a str,                       // a rerun: the review's points, the failures, the warnings
}

/// `read` gives a file's text by its workspace-relative path: the acting tree in a run, a
/// commit's tree in `rc map --eval`.
pub fn map_of(read: &dyn Fn(&str) -> Option<String>, files: &[String], ask: &Ask, changed: &ChangedLines, room: usize) -> Map;
/// The rows of the map and, under them, the text of the best pointers that fit `quote` tokens.
pub fn render(map: &Map, read: &dyn Fn(&str) -> Option<String>, quote: usize) -> String;
```

Scoring, all of it deterministic:

| Signal | Score |
|---|---|
| An identifier in the card (backticked, `snake_case` or `CamelCase`) equals `item_name` | 100 |
| The item overlaps a range the last run changed, or a failure names it | 80 |
| A test whose name contains the filter of the acceptance command | 70 |
| A word of the title or objective is a word of the item's name, weighted by how rare the word is among the file's items (so `run` and `card` score near nothing) | up to 40 |
| The item's body calls a `Named` item | 30 |

The rows that fit `room` (1,500 tokens for the whole prompt, not 60 rows a file) print in
file order as `start-end  signature  (why)`, then one line per file: "`agent.rs` 8,612 lines,
318 more items: Grep for a name before reading".

**Quoting.** The best pointers are quoted under the rows, numbered as `regions` numbers them
now, inside one budget of **6k tokens**, counted as characters ÷ 4 and not as lines. On a
first run those are the `Named` items: for them, turn 1 can be the edit. On a rerun they are
the items the failures, the warnings and the review's points name, then the items the last
run changed, and only then the rest of what it read, as rows with ranges. This replaces
`regions` and its 800 lines (F9): the same idea, chosen by what the rerun is asked to do, at
a fifth of the size.

Names that resolve nowhere in the card's files are searched once over the project's tracked
sources (the #219 case), and the hit becomes a pointer with its file.

The map is built from the tree the worker acts in (`worker.cwd`, not `root`, at
`agent.rs:5867`).

**The test recipe.** Most acceptances are `cargo test -p <crate> -q <filter>`. The prompt
gains three lines: whether a test matching the filter exists; the `tests.rs` it belongs in
and the name of the last test there; and the nearest existing test by the same scoring,
quoted whole when it is under 40 lines. The worker copies a shape instead of paging through
`commands/tests.rs` to learn the fixtures.

**The first move.** The task prompt ends with one line built from the map: "Start by reading
these ranges in one message: `agent.rs:5855-5902`, `agent.rs:8273-8317`. Write the test
first. Run the check once, with `rc try`."

**What it removes:** `OUTLINE_ROOM` as a per-file count; the 2k-token outline that names
nothing; `REGIONS_ROOM` and `REGION_LINES`; most of `read_hint`'s work, since the hint
arrives before the read.

**How the map is judged without a model.** At a run's end the agent has the pointers it gave
and the ranges the run changed. The share of changed items that lay inside a pointer is the
**hit rate**, printed on the end line (`map 4/5`). `rc map --eval` replays it over landed
cards: the map built from the tree at the card's base against the ranges its landing commit
changed. A landing commit carries no card number (`landing_message`, `agent.rs:3350`), so
the landing records the base and the landing commit in the ledger; the eval reads the pair
from there and the files through `git show <base>:<path>`. The scoring is tuned on that
number, on all three projects, before a worker pays for a bad guess.

### D2. A build answers in the call that started it (F2)

1. The pre hook returns the **whole** `tool_input` it was given with `command` replaced, and
   for a command it wraps in `rc hold` sets `timeout` to 600000 and removes
   `run_in_background`.
2. The harness is started with `BASH_DEFAULT_TIMEOUT_MS` and `BASH_MAX_TIMEOUT_MS` set, so a
   call that passes no timeout still waits (`claude_code.rs`, where the child's environment is
   built).
3. `rc hold` prints the failures and the last 25 lines of what it ran and writes the whole
   output to `var/runs/build-<n>.log`, naming the file. A compile error with 300 lines of
   notes becomes 30 lines in the transcript, paid on every later turn. Workers stop piping
   to `tail`, which hides the first error and everything `rc hold` says while it waits.
4. `rc try`: the card's own acceptance commands, in the worktree, under one hold, as one
   call. It prints a line per command and the failures, and records a stamp: the worktree's
   `HEAD`, a hash of `git status --porcelain` and of `git diff HEAD`, and the result. The
   worker stops guessing which test to run and how to pipe it.
5. `rc try` is a job, not a child of the call. It starts the check detached, with its output
   in `var/runs/try-<id>.log`, and waits for it up to nine minutes. Done: the result. Not
   done: one line (`queued 6 min behind 3 builds` or `building 4 min`) and exit 124. **The same command again joins the same job** while the stamp holds; it never
   starts a second build. One verb, at most one call per nine minutes of real build, no pid,
   no file to find.
6. The orchestrator's check of the card in its worktree reads the stamp. Same tree, passed:
   the card's own commands are not run a second time; the merged-tree suite (#231) runs as
   now. One build fewer per passing run on the slots the workers queue for.
7. **The build starts when the run does.** When a card's worktree is ready and before its
   worker is started, the orchestrator starts the compile half of the card's check as the
   same job `rc try` joins: for a `cargo test -p <crate> …` command, `cargo test -p <crate>
   --no-run`, in `target/wt-<id>`, under a slot. The worker reads and edits while the
   dependencies compile; its first `rc try` finds them built and compiles the crate it
   changed. A check that is not cargo's has no compile half and nothing starts. On a rerun
   the target is warm and the job ends in seconds.
8. **A new target starts from the project's dependencies.** Before item 7's job, a card's
   empty `wt-<id>` is seeded from the project's own target with the artifacts of packages
   that are not members of the workspace (their `deps`, `.fingerprint` and `build` entries,
   times kept). Cargo's key for a registry package does not hold the workspace's path, so I
   expect them to be taken as fresh; the members are left out, which keeps what #176 and
   #181 fixed. This is the one item here I could not prove from the code: its card carries a
   test of the copy, and the feed's warm line (`warm 0:48` against `warm 6:30`) says whether
   cargo took it.

```rust
// crates/ra-context/src/lib.rs, kept in var/context/<id>.try.json
pub struct Stamp {
    pub head: String,            // git rev-parse HEAD of the worktree
    pub tree: String,            // hash of `git status --porcelain` and `git diff HEAD`
    pub commands: Vec<String>,
    pub state: TryState,         // Queued { since } | Running { since, pid } | Passed | Failed { failures }
    pub log: PathBuf,
}
```

**What it removes:** the `rc wait` / `sleep` / Read-the-task-file turns; the "run cargo with
a 600000 timeout" line of the context once the hook does it; `rc wait` for builds; the
`| tail -n 25` habit; one compile per passing run; the 6–15 minutes a card's first build
spends at the end of its session.

### D3. A smaller floor (F3)

- **Tools per card.** `tools_for(card, project)`: one shell (a `[worker] shell` key, default
  the one the project's check commands are written for); `WebFetch` and `WebSearch` only for
  investigations and when `[worker] web = true`; `NotebookEdit` only when the card's files
  hold an `.ipynb`; `TodoWrite` off. My estimate is 3–5k of the 9.5k; the turn-1 field (D8)
  measures it on the first run after.
- **Notes by relevance.** A line learned on another project reaches this one only when it
  names something this one has: a path that exists here, or a command of this project's
  checks. Two lines that say the same thing are one line.
- **Brief.** An ancestor with no objective prints nothing; an ancestor's objective is cut to
  its first sentence.
- **Order.** What is the same for every run of a walk goes first (`SYSTEM`, the commit and
  retro prompts, the shell shapes, the other projects' digests), then the project's context.
  What is chosen by the card (its machine notes, its manifest lines, its last warnings) opens
  the task prompt instead of sitting between them (`harness_messages`, `agent.rs:5920–5943`).
  Two runs of a walk then share a cached prefix, and the cache write on turn 1 falls to the
  part that is the card's.

### D4. Resume only when it is cheaper (F4)

```rust
// crates/ra-console/src/agent.rs
struct Held { session: String, last_turn_at: SystemTime, last_turn_tokens: u64 }
const CACHE_LIFE: Duration = Duration::from_secs(5 * 60);
fn worth_resuming(held: &Held, now: SystemTime, fresh_floor: u64) -> bool {
    now.duration_since(held.last_turn_at).is_ok_and(|age| age < CACHE_LIFE) || held.last_turn_tokens <= 2 * fresh_floor
}
```

Every path that passes `--resume` asks it. `CACHE_LIFE` is set from what the feed shows once
it prints cache-written tokens (D8): a resumed first turn that writes its whole transcript
was past the life.

A missing report is not asked for on the resumed session. A new `report` role (local when the
server answers, else Haiku) writes it from what the orchestrator already holds: the diff
stat, the run's last assistant texts, the check's output. About 3k tokens in place of 83–93k.

### D5. A read-in that is seen while it happens (F5)

- At the tenth tool call without an edit, the post hook adds one line of context: the map's
  first move again and "ten calls without an edit". No kill.
- The end line always says the turn of the first edit.
- A run stopped for any reason hands on its read set, not only a capped or failed one, so the
  next run's map has it.

### D6. Reading in a context that is thrown away (F1, F8)

The investigator becomes a **scout**: the same read-only run, changed in five ways.

| | Investigator today | Scout |
|---|---|---|
| Runs for | every ready card, when `investigators` > 0 | a card whose map is weak: c3+, more than 1,500 lines behind "N more items", no `Named` or `Changed` pointer |
| Model | the implementer's | a `scout` role: a rung under the implementer on the ladder; the local model once the numbers allow |
| Starts on | the brief | the brief and the map, with what the map could not resolve said first |
| Turns | 40 | 12 |
| Answer | a brief under 150 lines | at most eight pointer lines and 20 lines of "what is there, what to reuse" |

    crates/ra-console/src/agent.rs:5855-5902  prepare: where the outline is built
    crates/ra-context/src/lib.rs:337-357      outline_marked: the rows to rank

Its lines become `Why::Scout` pointers, so the same quoting and the same hit rate apply to
them. The scout's 80k of reading is paid once on a cheaper model and discarded; the worker
starts at 14k plus what was pointed at. It goes to the local model only when the live hit
rate of scouted cards says a local scout points as well; until then Haiku or Sonnet.

Built after D1 on purpose: the deterministic map may leave few cards weak, and the scout is
judged by the map's own measure.

### D7. Files small enough to read once (F6)

`rc carve <file> <new file> <item>…` moves named items (with their doc comments) out of a
Rust file into a new module and adds the `mod` and `use` lines, using the parser the outline
already has (`items`, `lib.rs:363`). No model writes the moved lines. The doctor names
the files worth it: over 2,000 lines and in the `files:` of three or more open cards. Then
`agent.rs` is carved along the seams its items already show: review, merge and conflict,
build slots, prompt assembly.

### D8. The measures (first, so every later card is judged)

One word, one count: `turns` on the end line becomes the model calls the stream showed, which
is what every average is already taken over, with the harness's count beside it only when it
differs (`turns 44 (65 tool steps)`). The dead `0 tool call(s)` goes.

`TurnTally` gains four numbers; the end line and the ledger carry them:

| Field | Meaning | Target |
|---|---|---|
| `turn 1` | input tokens of the first turn | ≤ 14k first run, ≤ 18k rerun |
| `first edit` | the turn of the first Edit or Write | ≤ 4 on c1–c2, ≤ 8 on c3 |
| `waits` | turns whose only calls wait on a build (`rc wait`, `sleep`, a read or `tail` of a harness task file) | 0 |
| `written` | cache-written tokens of the run | ≈ turn 1's write plus the growth; a spike is a cold resume |
| `map` | changed items inside a pointer / changed items | ≥ 0.8 |

End line, after:

    ✗ #235 … · turns 65 · in 3,882k (97% cached, 212k written) out 21k · turn 1 19k · first edit 22 · read-in 1,491k, 3 files whole · waits 6 · map 1/5

`rc usage --runs` prints, per project and per complexity, the medians of turns, turn 1, first
edit, waits and average turn for the last 50 runs: one row to read after a walk.

## 5. Ranking

| # | Change | Moves | Expected | Evidence |
|---|---|---|---|---|
| 1 | D1 the map (ranked, ranged, quoted, from the acting tree; test recipe; first move) | turns, tokens a turn, pass rate on large files | c3 first runs from 47–65 turns at 58–98k to ≤ 25 at ≤ 40k: a third of the input or less. On every card, the 45% of calls before the first edit toward 20% | F1: read-in 0.8–1.5M; 68 of 151 calls; ten retro lines |
| 2 | D2 builds: one call, started with the run, on a seeded target | turns, wall-clock, runs that end with no result | 3–5 turns fewer per build; no "no test result" endings; the first build's 6–15 minutes out of the session's tail | F2: 5 of 7 calls in #245; 3 of 8 streams; 33 retro lines and 4 of today's 10 |
| 3 | D1 quoting on reruns (the regions by what the rerun is asked, in tokens) | tokens a turn and $ on the commonest run there is | reruns in large files from 47–49k a turn to about 25k | F9: $0.34–0.43 against $0.10–0.12 |
| 4 | F7 the card resolves before it runs | rerun rate, turns to the first edit | a search loop of 9–20 calls, or a whole run and its rerun, per card that names what is not there | 4 of today's 10 retro lines (#241, #242, #243, #245); #201 |
| 5 | D4 resume policy and the report role | $ per landed card | each avoided cold resume: $1–3.80 → $0.10–0.25 | F4 |
| 6 | D3 the floor | tokens a turn on every run of every project | 4–7k of 17–21k: 20–30% on short runs | F3 |
| 7 | D6 the scout | tokens a turn on the cards the map leaves weak | reading moved to a cheaper model and dropped | F1, F8 |
| 8 | D7 carve | parallel cards after #237, read size | agent cards stop queueing on one file | F6 |
| 9 | D5 read-in seen | long tails | fewer runs past 40 turns | F5 |

D8 (the measures) is built first: it is how every row above is checked.

The rerun rate moves with rows 1, 2 and 4: a run that finds its place, gets its build answer
and was given a card that names real things is a run that passes its check the first time.
Eight of the ten look-back lines the last walk's workers wrote are rows 2 and 4.

## 6. How each change is measured

- **Before/after on the same feed.** D8 lands first and one walk runs on it unchanged: that
  is the baseline, per project, per complexity.
- **D1:** `rc map --eval` hit rate over landed cards before any worker sees the map; then
  `first edit` and read-in medians on c3 cards; then turns and average turn. For reruns:
  `turn 1` of a rerun in `agent.rs` (45–50k today) and whether the rerun still passes in as
  few turns (3–9 today). If the turns go up by more than the tokens come down, the quote
  budget is too small: raise it and read the row again.
- **D2:** `waits` median to 0; the count of "moved to the background" in stream files to 0;
  retro lines naming `rc wait`, timeout or background per walk (33 in two archives) to 0;
  the `warm` time on a card's first run, seeded against unseeded (6:30 and 15:00 today), and
  the wall-clock from a run's last edit to its end line.
- **F7:** the count of runs whose look-back says the card named what was not there (four in
  the last walk) and of reruns whose check never ran a test (#245).
- **D3:** `turn 1`, read off the first run after each card.
- **D4:** `written` on resumed runs; no end line of ≤ 4 turns above $0.50.
- **D6:** hit rate of scouted cards against unscouted ones of the same complexity; the
  scout's own tokens under `scout` in `rc usage`.
- **D7:** the number of open cards whose `files:` overlap another's; largest file read.
- **All:** `$ per landed card` and rerun rate from the walk's summary, as now.

---

# Part 2: roadmap

## Phase AB: a run reaches its check in the fewest turns, on the smallest context

### The feed says where a run's turns and tokens went

- The turn tally counts the first turn, the turn of the first edit, wait turns and cache-written tokens {c2}
  objective: The feed prints a cached share and a read-in total, which hid three costs: a cold resume (#216, 1 turn, "93% cached", $1.52), build waits (5 of 7 calls in the #245 rerun) and how late the first edit came (68 of 151 tool calls in seven streams were before it). `TurnTally` already keeps `written` per turn and sees every tool call; it gains `first` (input of turn 1), `first_edit` (the model call of the first Edit or Write), `calls` (tool calls, which the end line prints as 0 today), `waits` (model calls whose only tool calls are `rc wait`, `sleep`, or a read or `tail` of a harness task output file) and the run's cache-written total, and the provider's response carries them.
  acceptance: `cargo test -p ra-providers -q tally_counts` passes five tests over recorded stream lines: the first turn's size, the first-edit turn, the tool calls counted, a wait turn counted and a build turn not counted, cache-written tokens summed.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs crates/ra-providers/src/lib.rs
- The end line of a run prints turn 1, first edit, waits and written tokens {c2}
  objective: The person reads the feed, not the stream files. `tokens_line` (`crates/ra-console/src/agent.rs:7267`) prints `in N (X% cached) out N · read-in N, N files whole`; it gains `N written` beside the cached share, and `turn 1 Nk · first edit N · waits N`, with `waits` left out at zero. The line's `turns` is one count: the model calls the average is taken over (`average_turn`, `agent.rs:7325`), with the harness's own count in brackets when it differs (#235 printed `turns 65` and an average that is the input over 44), and `N tool call(s)` is the tally's count, not 0.
  acceptance: `cargo test -p ra-console -q tokens_line_says` passes: a session with a cold resume prints its written tokens, one with two wait turns prints `waits 2`, one with none prints no `waits`, and a session of 44 model calls and 65 harness turns prints `turns 44 (65 tool steps)`.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- The ledger keeps the four numbers and `rc usage --runs` prints their medians by project and complexity {c3}
  objective: A change to the prompt is judged over a walk, not a run. `Entry` (`crates/ra-usage/src/lib.rs:20`) has `turns` and `read_in`; it gains `first_turn`, `first_edit`, `wait_turns` and `written_tokens`, old rows read as zero, and `rc usage --runs` prints one row per complexity for the last 50 worker runs: runs, median turns, median turn 1, median first edit, median waits, median average turn.
  acceptance: `cargo test -p ra-usage -q run_medians` passes: a ledger with rows of two complexities gives the medians of each, and a ledger written before the fields reads without error.
  files: crates/ra-usage/src/lib.rs crates/ra-usage/src/tests.rs apps/rc/src/main.rs

### A build answers in the call that started it

- The pre hook hands back the whole tool input, with the timeout a held build needs {c2}
  objective: A worker passed `"timeout":600000` and the harness said "did not complete within its 120s timeout and was moved to the background" (#245 rerun stream); the next four calls were waiting. The hook's rewrite answers `"updatedInput": {"command": command}` (`apps/rc/src/main.rs:998`), which drops every other field of the call. The answer becomes the input it was given with `command` replaced; a command wrapped in `rc hold` gets `timeout` 600000 and loses `run_in_background`.
  acceptance: `cargo test -p rc -q rewrite_keeps` passes: a rewritten call keeps its `description` and `timeout`, a held build with no timeout gets 600000, a held build asked for in the background comes back in the foreground.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs
- A worker's harness starts with the long shell timeout as its default {c1}
  objective: A build the hook does not rewrite (a project's own command on the allow list) still falls to the harness's 120 s default when the worker passes no timeout. The provider sets `BASH_DEFAULT_TIMEOUT_MS` and `BASH_MAX_TIMEOUT_MS` to 600000 in the environment of every worker it starts (`run_env`, applied at `crates/ra-providers/src/claude_code.rs:366`), under whatever `[worker] env` adds.
  acceptance: `cargo test -p ra-providers -q shell_timeout` passes: the environment of a worker's command holds both variables, and a `[worker] env` value for either wins.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs
- `rc hold` prints the failures and the tail of what it ran, and keeps the whole output in a file {c3}
  objective: Build output stays in the transcript and is paid on every later turn; workers already pipe to `tail -n 25`, which loses the first error and what `rc hold` says while it queues. `hold` (`crates/ra-console/src/agent.rs:2279`) runs its command with the terminal's own streams; it captures them instead, writes them whole to `var/runs/build-<n>.log` in the worktree, and prints the failures the check's own parser finds (`crates/ra-context/src/lib.rs:648`, 40 failures of 12 lines) and the last 25 lines, then `whole output: <path>`. A passing command prints its last line only.
  acceptance: `cargo test -p ra-console -q hold_prints` passes: a failing run of 300 lines prints the failure, the tail and the path in under 60 lines, the file holds all 300, and a passing run prints one line.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-context/src/lib.rs
- `rc try` runs the card's own check in one blocking call {c3}
  objective: Workers choose a test filter, a pipe and a timeout by hand, and run two or three builds a session (#215: test, check, test). `rc try`, run in a card's worktree, runs that card's acceptance commands under one hold, prints `✓`/`✗` and the command per line with the failures of the first that failed, and exits with its result. The task prompt's last line names it.
  acceptance: `cargo test -p ra-console -q try_runs` passes: with a scripted check, a passing card prints one `✓` line per command and exits 0, a failing one prints the failure and exits 1.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs apps/rc/src/main.rs
- `rc try` run again joins the check that is still running {c3}
  objective: `rc hold` waits up to 15 minutes for a slot (`HOLD_WAIT`, `crates/ra-console/src/agent.rs:2072`) and a shell call lives ten, so a queued build cannot answer in its call: #213 ended "with no test result" after twelve minutes, #215's `rc wait` "gave up twice at 540 s", #242 spent three calls finding the harness's task file. `rc try` starts the check as a detached job writing `var/runs/try-<id>.log` and a stamp (`head`, `tree`, `commands`, `state`), waits nine minutes, and on a job not done prints where it stands (`queued 6 min behind 3 builds` or `building 4 min`) and exits 124; run again on the same stamp it joins that job and starts no second build; on a changed tree it stops the old job and starts anew.
  acceptance: `cargo test -p ra-console -q try_joins` passes: a second call while the scripted job runs starts no second job and returns the first's result, a call after an edit starts a new one, and a job past the wait prints the standing line and exits 124.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-context/src/lib.rs apps/rc/src/main.rs
- The orchestrator's check takes a passed `rc try` of the same tree as its answer {c3}
  objective: A run that checked itself is compiled a second time by the orchestrator on one of the eight slots the other workers queue for. The stamp of `rc try` holds the worktree's `HEAD`, a hash of `git status --porcelain` and `git diff HEAD`, the commands and their result. The run's check reads it: same stamp and passed, the commands are taken as passed and the check line says `checked by the worker's own run`; anything else runs as now. The merged-tree suite (#231) is untouched.
  acceptance: `cargo test -p ra-console -q stamp_of_a_try` passes: an unchanged tree skips the commands, a tree with one more edit runs them, a failed stamp runs them.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-context/src/lib.rs
- A run's start begins compiling the card's check while the worker reads {c3}
  objective: A card builds in its own empty target, `target/wt-<id>` (`shared_target`, `crates/ra-providers/src/claude_code.rs:1029`), and nothing builds there until the worker asks after its last edit: "A cold `cargo build -p rc` took about 6.5 minutes in the worktree and went to the background" (#242), "15 minutes on a cold worktree target" (#243). Before a card's worker is started the orchestrator starts the compile half of the card's check as the job `rc try` joins: each cargo test command of the acceptance with `--no-run` in place of its filter, under a build slot, in the card's target. The worker's first `rc try` waits on that job and then runs the check; a check with no cargo command starts nothing. The feed prints `warm 6:30` when the job ends.
  acceptance: `cargo test -p ra-console -q warm_at_start` passes: a card whose check is a cargo test has the compile-only form of it started before the scripted worker's first turn, a try made while it runs starts no second build, and a card whose check is a script starts no job.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-context/src/lib.rs
- A card's new target starts from the project's built dependencies {c3}
  objective: The warm build above still compiles every dependency once per card: 6.5 to 15 minutes of a slot for each new card, on a machine where the project's own target holds the same artifacts. `seed_target(project, card, members)` copies into an empty `wt-<id>` the `deps`, `.fingerprint` and `build` entries of the packages that are not workspace members, keeping their times, and leaves the members out (two worktrees must never share a member's artifacts, #176 and #181). It runs before the warm build, only into an empty target, and a failed copy leaves the target empty and says so. Whether cargo takes the copies as fresh is read off the next walk's `warm` times; if it does not, the card is reverted and the feed has said why.
  acceptance: `cargo test -p ra-providers -q seed_target` passes: from a fixture target with a dependency's and a member's files, the dependency's three kinds of entry are in the new target with their modified times, the member's are not, and a target that already has files is left as it is.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs crates/ra-console/src/agent.rs

### The outline points at the card's work

- Outline rows carry `start-end`, and the outline is read from the tree the worker acts in {c2}
  objective: A row gives a start line only, so one function costs a 400-line read; and `prepare` builds the outline from the project root (`crates/ra-console/src/agent.rs:5867`) where the regions two lines on are read from the worktree, so a rerun gets line numbers that have moved. Rows print `start-end`; an item's range opens on its doc comment and attributes, where `items` (`crates/ra-context/src/lib.rs:363`) today gives them to the item before; and `prepare` passes the worker's directory.
  acceptance: `cargo test -p ra-context -q outline_rows` passes: a row of a three-line function with a two-line doc comment at line 8 prints `8-12`; and `cargo test -p ra-console -q outline_of_the_worktree` passes: a worktree whose file differs from the root's is outlined as the worktree has it.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- `map_of` ranks a file's items against the card's words {c3}
  objective: `outline_marked` (`crates/ra-context/src/lib.rs:337`) shows the first 60 items in file order: for `agent.rs` lines 28–646 of 8,600, none of them a card's subject (`var/context/213.md`), and c3 cards there read 0.8–1.5M tokens before their first edit. `map_of(read, files, ask, changed, room)`, with `read` a function from a path to the file's text and `ask` the card's title, objective, acceptance and a rerun's points, returns `Pointer { file, start, end, item, why, score }`: 100 for an item the card names, 80 for one the last run changed or the points name, 70 for a test holding the acceptance's filter, up to 40 for a word of the card in the item's name weighted by its rarity in the file, 30 for a caller of a named item; the best that fit `room` tokens across all files, printed in file order with the reason, then one line per file for what is not shown.
  acceptance: `cargo test -p ra-context -q map_ranks` passes five tests: a named item outranks an early one, a changed item is kept, a common word scores under a rare one, the budget holds across two files, and a file with nothing pointed prints its one line.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs
- A worker's prompt carries the map in place of the per-file outline {c2}
  objective: The ranked map replaces `outline_files` in `prepare` and in the rerun prompt, with the card's title, objective and acceptance as the ask, the last handoff's changed ranges and failures, and a room of 1,500 tokens; `OUTLINE_ROOM` goes. `var/context/<id>.md` shows the map as the worker got it.
  acceptance: `cargo test -p ra-console -q prompt_carries_the_map` passes: a card naming a function at the end of a 100-item file has that function's row in its prompt and no row of the file's first items.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-context/src/lib.rs
- A name the card uses that its files do not define is found once across the project {c3}
  objective: #219's look-back: the handler was in `apps/rc/src/main.rs`, the card listed `crates/` only, "about eight searches". For each identifier of the card that no item of its files matches, `map_of` searches the project's tracked source files for an item of that name and adds it as a pointer with its file; a name found nowhere is returned in `Map::unresolved`.
  acceptance: `cargo test -p ra-context -q map_finds_outside` passes: an item named by the card and defined in a file outside `files` is pointed at with its path, and a name defined nowhere is listed unresolved.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs
- The items a card names are quoted in its first prompt {c2}
  objective: A rerun's prompt quotes the regions the last run read (`regions`, `crates/ra-context/src/lib.rs:902`); a first run gets nothing and spends its first turns fetching what the card already names (#241: nine of eleven calls before its two edits). `render(map, read, quote)` prints under the map's rows the text of the `Named` pointers, numbered as regions are, the highest score first, inside `quote` tokens counted as characters over four; a first run's `quote` is 6,000. An item that does not fit keeps its row.
  acceptance: `cargo test -p ra-context -q render_quotes` passes: a card naming a 20-line function has its numbered body under the rows, one naming a function longer than the budget has the row and no body, and two named items over the budget together quote the higher-scored one.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- A rerun is handed the items its points name, not the first 800 lines the last run read {c3}
  objective: Reruns of c3 cards in `agent.rs` ran at 47–49k tokens a turn from their first turn (#236: 3 turns, average and largest both 47k, $0.34; #235: 6 turns, $0.41; #238: 9 turns, $0.43) where reruns with no read set run at 18–21k ($0.10–0.12): `regions` quotes 800 lines in the order the last run first read them. On a rerun the map's ask carries the failures, the lint's warnings and the review's points; the quote holds the items they name, then the items the last run changed, inside the same 6,000 tokens; the rest of the read set prints as rows with ranges. `regions`, `REGION_LINES` and `REGIONS_ROOM` go.
  acceptance: `cargo test -p ra-console -q rerun_quotes` passes: a rerun whose failure names one of five functions the last run read quotes that function and lists the other four as rows; and its prompt is under a third of the length the 800-line regions gave the same fixture.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs
- The prompt says where the card's test goes and quotes the nearest one {c3}
  objective: An acceptance is `cargo test -p <crate> -q <filter>`; the worker pages through `tests.rs` (`commands/tests.rs` is the largest file a card reads after `agent.rs`) to learn the fixtures. From the acceptance the prompt gains: whether a test matching the filter exists and where; the `tests.rs` beside the card's first file and the name and line of its last test; the highest-scored existing test for the card's words, quoted when under 40 lines.
  acceptance: `cargo test -p ra-context -q test_recipe` passes: a filter no test matches gives "no test matches yet", the tests file and its last test; a filter that matches gives the test's range; the quoted test is the one sharing the card's words.
  files: crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs crates/ra-console/src/agent.rs
- The task prompt ends on the first move {c1}
  objective: Turn 1 of a fresh run is usually one read of one page. The prompt's last line is built from the map: the ranges to read in one message (the three best pointers not quoted), "write the test first", and `rc try` as the one check.
  acceptance: `cargo test -p ra-console -q prompt_ends_on` passes: the last line names the best pointers as `path:start-end` and the try command, and a card with an empty map ends on the try command alone.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- The end line says how much of the change the map pointed at {c2}
  objective: The map's scoring needs a number that does not take a model to read. At a run's end the changed ranges are set against the pointers the prompt carried: `map 4/5` is the changed items that overlapped a pointer over the changed items; the ledger keeps both counts.
  acceptance: `cargo test -p ra-console -q map_hit` passes: a run that changed two pointed items and one unpointed prints `map 2/3`, and a run with no map prints nothing.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-usage/src/lib.rs
- `rc map --eval` replays landed cards and prints the hit rate {c3}
  objective: Scoring weights should be tuned on history, on every project, before workers pay for a bad guess. A landing commit carries no card number (`landing_message`, `crates/ra-console/src/agent.rs:3350`), so a landing writes the card's base commit and landing commit to the ledger. For each card with the pair, the map is built from the card's text and the files as `git show <base>:<path>` gives them, and set against the ranges `git diff -U0 <base> <landing>` changed; the command prints a row per card and the mean, and `rc map <id>` prints one card's map as its prompt would carry it. Cards landed before the pair was kept are passed over.
  acceptance: `cargo test -p ra-console -q map_eval` passes: in a fixture repository with two landed cards, one whose changed function the card names and one not, the command prints `1/1`, `0/1` and a mean of 0.5.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-usage/src/lib.rs apps/rc/src/main.rs
- A read of the project root's copy of a file from a worktree is answered from the worktree {c2}
  objective: In the #242 stream the worker read `apps/rc/src/main.rs` twice from the project root (offsets 300 and 915) and then the same two places again from its worktree (315 and 925): four reads for two, the first two of a tree that is not the one it edits and whose lines differ once the card has changed the file. It is one stream of eight, so the card is small and comes last in its section. For a Read or a Grep whose path lies under the project root and outside the worktree the session runs in, where the worktree has the same relative path, the pre hook hands back the call's input with the worktree's path; paths under `var/` are left alone.
  acceptance: `cargo test -p rc -q read_of_the_root` passes: a Read of the root's copy of a source file from a session in a worktree comes back with the worktree's path and its offset and limit kept, a Read of the root's context file under var is untouched, and a Read from a session in the root is untouched.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs

### A card resolves before a worker starts on it

- A run's start names what its card points at that is not there {c2}
  objective: Four of the ten look-backs of the last walk: "The card named `rc handoff --dry-run` and `docs/context.md`, neither of which exists" (#241); `.claude/settings.json` and `rc gate` (#242); "neither `rc gate` nor `rc gotcha-match` existed" (#243, 19 calls before its first edit). Before the worker starts, each path in the card's `files:` and each backticked path in its text is looked for in the tree, each backticked `rc <verb>` among the commands `rc` has, and `Map::unresolved` gives the identifiers nothing defines. The brief gains one line (`not in the tree: docs/context.md; no such command: rc gate; nothing defines: handoff_dry`), ending "treat these as new, to be made by this card, unless the card says otherwise", and the feed prints the same, so the worker does not search for what is not there and the person sees a stale card.
  acceptance: `cargo test -p ra-console -q brief_names_the_missing` passes: a card with one missing path and one existing prints the missing one only, a card naming an rc verb that does not exist prints it, and a path the card calls new is not listed.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs apps/rc/src/main.rs
- A check that cannot run is said before a worker is spent on it {c2}
  objective: #245's look-back: "The acceptance check named `<review-crate>` literally and failed before any test ran. Naming `ra-console` on the card would have avoided the rerun." A whole run and a rerun for what a parser sees. When a walk picks a card, each command `named_checks` (`crates/ra-console/src/agent.rs:6400`) takes from its acceptance is read: a `<…>` placeholder left in it, or a `-p` that names no package of the workspace, and the card is passed over with one feed line (`#245 not started: its check names -p <review-crate>, no package of this workspace`) as a card with an unmet dependency is; a dry run and `rc show` print the same line.
  acceptance: `cargo test -p ra-console -q check_that_cannot_run` passes: a card whose check holds a placeholder is not started and the line names it, one whose package does not exist likewise, and a card whose check names a real package starts.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- A card whose check already passes on its base is offered for review without a run {c3}
  objective: #201's look-back: "started on a card whose work was already committed … three reads to find that out". When a card's files changed on main since the card was written and it has no handoff, the card's acceptance commands run on the base tree before a worker is started; all passing with a test matching each filter, the card goes to the reviewer with the line `already passes on main` and no worker runs.
  acceptance: `cargo test -p ra-console -q already_passes` passes: a scripted check that passes sends the card to review with no worker call, a failing one starts the worker, and a card whose files did not change runs no check first.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

### A smaller floor under every turn

- A worker gets the tools its card can use {c2}
  objective: The first turn of every fresh run reads 9.5k tokens of tool schemas before the prompt (cache read 9,465–9,558 in four streams); `WORKER_TOOLS` (`crates/ra-providers/src/claude_code.rs:32`) names eleven tools for every card, the harness loads ten, and the 151 calls of seven streams use six: Grep, Read, Edit, Write, Bash, Glob. `tools_for` gives one shell (`[worker] shell`, else both as now), `WebFetch` and `WebSearch` only to an investigation or under `[worker] web = true`, `NotebookEdit` only when a card file ends `.ipynb`, and no `TodoWrite`, which the harness no longer has.
  acceptance: `cargo test -p ra-providers -q tools_for` passes: a plain card's list has no web, notebook or todo tool; an investigation has the web tools; a project with `shell = "bash"` has no PowerShell.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs crates/ra-console/src/router.rs
- A line learned on another project is shown only where it names something this project has {c2}
  objective: The #245 run's system prompt carried runelite's `third-party/jdk8u` line twice and two lines of asm's, none of use to an agent card (`var/worktrees/245/var/runs/system-19396.md`). `machine_notes` (`crates/ra-console/src/agent.rs:7226`) keeps a line from another project when a path it names exists here or a command it names is in this project's checks or allow list, and drops a line whose words another kept line already holds.
  acceptance: `cargo test -p ra-console -q machine_notes_keep` passes: a line naming a path absent here is dropped, one naming the build command of this project's checks is kept, and two lines differing by one word print once.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/retro.rs
- The brief skips an ancestor with no objective and cuts the others to a sentence {c1}
  objective: `var/context/213.md`: "Under #1 Rhipper Agent: no objective" and a 100-token paragraph for the phase, in every prompt of every card under it. An ancestor with no objective prints nothing; one with an objective prints its first sentence.
  acceptance: `cargo test -p ra-console -q brief_of_ancestors` passes: an ancestor without an objective adds no line, and one with three sentences adds the first.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- The system prompt puts what a walk's runs share first {c2}
  objective: `harness_messages` (`crates/ra-console/src/agent.rs:5911`) puts the context cut to the card second, then notes chosen by the card (`machine_notes` takes its id and acceptance, the manifest its brief, the warnings are its last run's), then the retro and commit prompts, which are the same for every run; so each run writes its whole 7–12k to the cache (turn-1 cache writes of 7,752 to 12,088 tokens). The order becomes: `SYSTEM`, the commit and retro prompts, the shell shapes, the other projects' digests, then the project's context and direction; the notes, the manifest lines and the warnings of the card open the task prompt. Two runs of one walk then share the prefix.
  acceptance: `cargo test -p ra-console -q system_prompt_shares` passes: the system prompts of two cards of different crates in one project are equal up to the project's context heading.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-providers/src/claude_code.rs

### A session is resumed only when that is the cheaper start

- Every resume asks the age and size of the session first {c2}
  objective: One-turn runs on a resumed session cost $1.52 (#216) and $1.87 (#208) against $0.10–0.12 for three fresh turns (#213, #216): past the provider's cache life a resume writes the whole transcript again. `worth_resuming(held, now, fresh_floor)` is true inside `CACHE_LIFE` (five minutes) of the session's last turn or when its last turn was under twice a fresh start; the report ask, an absolute card's next run and a continued run all go through it, and a run that does not resume starts on the rerun block as #210's reruns do.
  acceptance: `cargo test -p ra-console -q worth_resuming` passes: a 90k session two minutes old resumes, the same session an hour old does not, a 20k session an hour old does.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- A missing report is written by the `report` role from the diff and the run's last words {c3}
  objective: "report asked twice" (#196, #209, #215, #220, #233) resumes the session for up to three turns at its full context (`crates/ra-console/src/agent.rs:5245`). A `report` role in the role table (`crates/ra-providers/src/route.rs:30`; the local model when the server answers, else Haiku) is given the diff stat, the changed items, the run's last three assistant texts and the check's output, and writes the report in the handoff's shape; the session is asked again only when the role is off.
  acceptance: `cargo test -p ra-console -q report_role` passes: a run that ends without a report gets one from the scripted role with no second worker call, and with `report = ""` the worker is asked as before.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-providers/src/route.rs crates/ra-console/src/router.rs

### A long read-in is seen while it happens

- A run ten turns in with no edit is shown its first move again {c2}
  objective: Between a good run and the 160k cap there is no signal: #242 made 20 calls and #243 made 19 before their first edit. The hook is one process per call (`hook`, `apps/rc/src/main.rs:1009`), so it keeps a count per session in `var/runs/calls-<session_id>.json` (calls, whether an edit was seen, whether the line was said). On the tenth tool call of a session with no Edit or Write, the post hook adds one line of context: `ten calls without an edit` and the map's first-move line, read from the card's context file. It says it once.
  acceptance: `cargo test -p rc -q nudge_at_ten` passes: the hook's answer on the tenth tool call of a session with no edit carries the line, on the eleventh it does not, and a session that edited at its third call never gets it.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs crates/ra-console/src/agent.rs
- Every run that stops hands on what it read {c1}
  objective: The read set reaches the handoff only from a capped run or a failed check (`crates/ra-console/src/agent.rs:5407`, `:5704`); a run stopped by a limit, a kill or a blocked card leaves the next one to read it all again. Every handoff of a run that did not pass carries `read_set(&session.reads)`.
  acceptance: `cargo test -p ra-console -q handoff_carries_reads` passes: a run that ends blocked has its read ranges in the handoff, and the next prompt quotes them.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs

### The reading is done in a context that is thrown away

- The investigator runs on the `scout` role's model, opens on the map and has twelve turns {c2}
  objective: `investigate` (`crates/ra-console/src/agent.rs:4928`) reads on the implementer's own model for up to 40 turns (`INVESTIGATE_TURNS`) and is given the brief with no outline, so the reading costs what the worker's would and starts colder. Its model becomes the `scout` key of `[role]`, else one rung under the card's implementer on the ladder; its prompt carries the card's map and the names the map left unresolved; its turns are 12.
  acceptance: `cargo test -p ra-console -q scout_runs_on` passes: with no `scout` key a card routed to opus is investigated on sonnet, with `scout = "haiku"` on haiku, and the investigator's prompt holds the map's rows.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/router.rs crates/ra-providers/src/route.rs
- The scout answers in pointer lines, and they join the card's map {c3}
  objective: A brief of up to 150 lines rides on every turn of the builder (`crates/ra-console/src/agent.rs:5947`). The scout is asked for at most eight lines of `path:start-end  why` and 20 lines of what is there and what to reuse; the pointer lines parse into `Why::Scout` pointers at a score under a named item's, so the builder's prompt quotes them inside the one quote budget and the hit rate counts them; lines that do not parse stay in the brief.
  acceptance: `cargo test -p ra-console -q scout_leaves` passes: a scripted scout's two pointer lines are rows of the builder's map marked as the scout's, the first is quoted, and an answer with no pointer line leaves the map as it was and the brief as written.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs
- A walk scouts the cards whose map is weak, and only those {c2}
  objective: With `investigators` set a walk investigates every ready card that has no brief (`crates/ra-console/src/agent.rs:4775`), and with it unset none; the agent's own project has never run one. A card is scouted when it is c3 or above, its map has no named or changed pointer, and more than 1,500 lines of its files sit behind "more items"; `investigators` stays the count of seats, and `[worker] scout = "all"` or `"none"` overrides the rule.
  acceptance: `cargo test -p ra-console -q walk_scouts` passes: of a c3 card with a weak map, a c3 card that names its function and a c1 card, the walk's scripted provider sees one investigation, of the first.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs crates/ra-console/src/router.rs
- `rc usage --runs` sets scouted runs beside unscouted ones {c2}
  objective: The scout stays only if it pays: its row shows, per complexity, runs with and without a scout, their median first edit, turns and average turn, the map hit rate of each, and the scout's own tokens, so moving the role to the local model is a decision read off one table.
  acceptance: `cargo test -p ra-usage -q scouted_rows` passes: a ledger with scouted and unscouted runs of one complexity prints two rows with their medians and the scout's tokens.
  files: crates/ra-usage/src/lib.rs crates/ra-usage/src/tests.rs apps/rc/src/main.rs

### Files small enough to read once

- `rc carve` moves named items of a Rust file into a new module without a model writing them {c3}
  objective: `agent.rs` is 8,600 lines and in the `files:` of most agent cards; after #237 one file is also one queue. Splitting it by hand means a worker writes every moved line out again. `rc carve <file> <new file> <item>…` takes the items' ranges from the outline's parser (`items`, `crates/ra-context/src/lib.rs:363`, with the doc comment and attributes above an item counted as its own), writes them to the new file, removes them from the old, and adds `mod` and `use super::*` lines; `--dry` prints the ranges.
  acceptance: `cargo test -p ra-context -q carve_moves` passes: two named functions and their doc comments leave the old text and make the new, in order, and a name that is not an item is an error that changes nothing.
  files: crates/ra-context/src/carve.rs crates/ra-context/src/lib.rs crates/ra-context/src/tests.rs apps/rc/src/main.rs
- The doctor names the files that are long and in many cards {c2}
  objective: Every project has its `agent.rs`. `rc doctor` prints, for each source file over 2,000 lines named by three or more open cards, its length and the cards, and `carve candidates: <the largest groups of items sharing a name prefix or a section comment>`.
  acceptance: `cargo test -p ra-console -q doctor_names_long_files` passes: a 2,500-line file in three cards is listed with them, a long file in one card is not.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/tests.rs
- The review and fix passes of the agent live in `agent/review.rs` {c2}
  objective: First carve of `agent.rs` along a seam its items show: `review`, `judge`, `lint_fix`, `fix_model`, the fix instructions and their helpers (about lines 1150–1860). Moved with `rc carve`, visibility widened to `pub(super)` where the build asks, no behaviour changed.
  acceptance: `cargo test -p ra-console -q` passes with the same count of tests as before the move.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/review.rs
- The build slots and waits of the agent live in `agent/builds.rs` {c2}
  objective: Second carve: `hold`, `wait`, `build_allowance` and their constants (about lines 2110–2400), the code the build cards of this phase touch, out of the file every other card touches.
  acceptance: `cargo test -p ra-console -q` passes with the same count of tests as before the move.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/builds.rs
- The prompt assembly of the agent lives in `agent/prompt.rs` {c2}
  objective: Third carve: `prepare`, `harness_messages`, `task_text`, `digest_block`, `machine_notes`, `rerun_text` and the prompt constants: everything that decides what a run carries in one file of a size a worker reads once.
  acceptance: `cargo test -p ra-console -q` passes with the same count of tests as before the move.
  files: crates/ra-console/src/agent.rs crates/ra-console/src/agent/prompt.rs
