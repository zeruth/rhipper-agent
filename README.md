# Rhipper Agent

A roadmap-driven development orchestrator in Rust. A project's work is a roadmap of cards;
workers (Claude Code headless on the person's plan, or any model through OpenRouter or a local
server) implement the cards, several at once, each in a git worktree of its own; the
orchestrator does everything around the runs without a model call: the context, the checks,
the lint, the review, the landing, the handoff, the ledger, the gotchas. One window hosts every
project as a tab and a dashboard over all of them, and an external agent session pairs with
the whole thing over a one-line-per-event feed.

![The dashboard: three projects walking at once, the worker slots, each project's allocation and its live runs](docs/window.png)

The roadmap owns the work. Models are replaceable workers. The orchestrator owns coordination,
execution state, acceptance and the merge. The agent runs on its own roadmap too, and its
workers prune and regenerate that roadmap from the evidence of its own runs.

## The loop

```
rhipper.md ──rc walk──▶ run cards in worktrees ──▶ check ──▶ lint ──▶ review ──▶ land on the branch
    ▲                     │ handoff, ledger, gotchas, look-backs          │ sent back: rerun on its session
    └── rc plan / regeneration cards (from the runs' own evidence) ◀──────┘
```

1. **A card** has an objective, acceptance criteria (every backticked command in it is a
   check), the files the change lands in, a complexity, and what it waits on.
2. **A run** sends the worker the card, the project context, an outline of each file it names
   (regions, not whole files), the last handoffs, and the machine notes. The worker acts in the
   card's worktree with its own tools; the shell allowlist and the gotcha hooks hold it.
3. **The check** runs after the worker returns: the acceptance's commands in order, else the
   project's default, else the workspace tests. Then **the lint** over the files the run
   changed. A lint-only failure takes a cheap fix pass, not a second full run.
4. **The reviewer** reads the acceptance, the handoff and the diff of the card's files, and
   answers `accepted` or `needs revision` with points. A card sent back goes straight back into
   the queue and its rerun resumes the same session with the verdict as its prompt.
5. **The landing**: an accepted card is committed on `card/<id>`, merged into the project's
   branch, the check runs again on the merged tree, and the worktree, the branch and the card's
   temp files are cleared. The git log is the review record; the person reads verdicts.

Everything the orchestrator does between the model calls is local and free: the context is
built from the files, the handoff from the diff, the gotchas from regular expressions, the
ledger from the stream's own usage events.

## The pieces

### The roadmap

`rhipper.md` beside the code: project → milestones (phases) → sections → cards (tasks and
subtasks). A card's states: `defined` → `in_progress` → `implemented` → `review` → `accepted`,
with `needs_revision`, `failed`, `blocked`, `cancelled` at the side. A card is *ready* when it
is defined (or sent back) and everything it waits on is **accepted**, landed on the branch, not
merely implemented. `rc show`, `rc show <id>`, `rc next --why`, `rc check` (the lint of the
roadmap itself), `rc add`, `rc edit`, `rc depend`, `rc move`, `rc remove`, `rc import` (a
Markdown outline), `rc init` (a strong model drafts the roadmap from what the directory
shows). The console and the window follow outside edits of the file.

Workers may reshape a roadmap themselves: inside a card's worktree `rc` opens the project's
`rhipper.md`, and a project can allow its workers the roadmap commands (`rc show`, `add`,
`edit`, `remove`, `move`, `depend`, `priority`, `set <id> defined|blocked|cancelled`; never
`accepted`). A regeneration card prunes the done and duplicated cards and writes the next
phases to the depth of the best cards already there.

### Absolute cards

A card marked absolute (`rc absolute <id>`, or any card of complexity 5) works until it is
done: no turn cap, no context cap, its session resumed whatever its weight, and a failed check
or a rejection sends it straight back to work inside the same run, the verdict as its next
prompt, instead of back to the queue. Only a full stop ends it: the plan's limit, the provider
gone, the person, or three passes in a row that change no file at all, which is a card going
round stuck; the walk then blocks it with its last answer on the feed. The window wears an
`absolute` badge on such cards. A monumental card costs less as one whole run than as the
restarts a cap would cut it into.

### Gambits: the standing direction

A gambit is a sentence of direction with a weight and focus words. The walk orders the ready
cards by the person's priority first, then by the summed weight of the active gambits whose
focus matches the card's title, milestone or files, then by the file's order. Park a gambit
and the cards it pulled sink; add one and the queue reorders itself.

```
rc gambit add "The goal is the PE port, 32 to 64 and 64 to 64 …" --weight 5 --focus emit output port x86 x64 …
rc gambit park 6
rc priority 47 5
rc next --why          # each ready card with the gambits that scored it
```

A gambit with `plan-next: true` asks the walk to plan an empty milestone when nothing is ready.

### The walk

`rc walk` runs the ready cards as a process of its own that the window starts and the tab
shows: detached, hours long, surviving window restarts. With `[worker] parallel = N` it keeps
N cards running, each in its own worktree (`var/worktrees/<id>` on `card/<id>`); when one ends
the next ready card takes its runner at once. Before a run the project's branch is merged into
the card's, so a card starts from the current tree; a conflict there is the worker's first job.
The walk re-reads its runners and its routes before every card, so the dashboard's controls
apply while it runs. `rc walk --stop` ends it after the cards it is in.

Cards that come back from the reviewer or a failed check are rerun straight away; a card sent
back three times in a row in one walk is blocked with its worker's reason on the feed (usually
a missing toolchain or fixture only a person can provide). A walk lands every accepted card it
finds when it starts, so a restart never strands accepted work. A walk that stops says why on
the feed: no card ready, the plan's week past the project's `[plan] threshold`, or a failed run.

### The machine's pool and the priority order

Every project on the machine shares one pool: twelve Opus writers and the worker slots below.
`rc order runelite rhipper-asm` sets the priority order: the first project with ready work gets
the whole pool, the next takes what it leaves. Fable projects run outside the pool. The
dashboard moves a project up or down the order with its `priority` buttons.

### Worker slots: the heavy work, on a pooled CPU

Builds and the other heavy work run on the machine's worker slots, four by default and set from
the dashboard (`BUILDERS − n +`). A project's workers write code; `[worker] builds = 0` leaves
the compiling to the orchestrator's check, which takes a slot. A worker's own gradle or cargo
build, a `.py`, `.ps1` or node script, a JVM, a recursive search (`rg`, `grep -r`) and a WSL
toolchain command are rewritten by the hook to `rc hold -- <command>`, which waits for a slot
when all are taken; only gradle, cargo and the contract runner count against a session's build
budget. The slots are shared fairly: a project that wants a build and runs none goes before
the projects already running one.

Every process holding a slot runs in a Windows job object with a hard CPU cap: the slots share
90% of the processor, split evenly among the ones held and read again every two seconds as
holders come and go, so one build gets 90%, two 45% each, three 30% each. The dashboard's worker
row shows who holds each slot, for how long and at what share.

### Workers and routes

The model id decides the route. A bare `claude-*` id runs on the local Claude Code
(`claude -p`, stream-json, the machine's own login), so a subscription pays. A `vendor/model`
id goes through OpenRouter; `local/<name>` to an OpenAI-compatible server at
`RHIPPER_LOCAL_URL`. A bare model on those routes gets the `ra-tools` toolbox and the tool
loop. `[[route]]` rules in `rhipper.toml` pick a model by complexity and kind; a card can name
its own (`--model`); `rc route <id>` says which and why. The dashboard's `fable / opus` toggle
sets a project's first route. The cheap roles a run ends with (the handoff summary, the digest
line, the look-back classifier) go to the `[role]` table's models, else to the local server
when it answers a ping, else to no call at all.

Every run of a card resumes the harness session of the last one, reruns included: the cache
holds the context the worker read the code in. A rerun after a rejection resumes only a lean
session (its largest turn under 80k tokens); a heavier one starts fresh on the handoffs. A
project may set `[worker] context = 160000`: once a turn's input passes it, the run ends on a
handoff and the card continues in a fresh session. A turn's cost scales with the context it
carries, so this is the lever, not turn counts. Absolute cards are exempt from both.

### Checks, lint, the reviewer and the landing

The check is what the card's acceptance names in backticks, in order, stopping at the first
failure; the project's `[worker] check` when it names none; `cargo test --workspace -q` when
neither does. `[worker] check_timeout` gives a long suite its half hour. The lint is clippy
over the changed crates, or the project's `[worker] lint`, or none (`lint = ""`). The
reviewer's diff covers the card's files, the files the handoff names and the untracked files
among them. `rc review <id>` runs the reviewer by hand.

An accepted card lands: committed with a message that tells its story, merged `--no-ff`, the
check run again on the merged tree (a failure undoes the merge), then cleared. A merge that
conflicts sends the card back to its worker, whose rerun merges the branch in and resolves the
files the brief names. One landing at a time per project, across processes. `rc land <id>`
lands by hand; `rc doctor` checks for accepted cards whose merge never happened and for temp
directories left behind.

### Context and handoffs

`rhipper.context.md` is the project context every run carries: layout, build and checks,
conventions, the toolchains, the standing direction. `rc context --init` drafts it; it is kept
by hand. `var/context/<id>.md` is a card's context: the brief, the outlines, one handoff per
run (the outcome, the check, the verdict, the files changed, what was done and what is left).
`rc context <id>` prints what a run would send; `rc run <id> --dry` the whole prompt and the
harness command line.

### Gotchas: a gotcha is anything standing in the way

A `[[gotcha]]` in `var/gotchas.toml` (seeded from `crates/ra-gotchas/gotchas.toml`, the seed
merged in on every run start) is a regular expression over a command, a result, an edit or a
text, with a remedy: a hint that denies with a message, a rewrite of the command, or a count.
Claude Code's tool hooks call `rc hook pre|post`, which refuses or rewrites before the command
runs and adds the hint a result calls for. A command the project's own allowlist names passes
the allowlist hint; the rewrites (the worker slots) still apply. `rc gotchas` shows the counts
and the refused commands no gotcha names yet. No model call.

### Notes, look-backs and debug mode

A line typed in the window without `/`, or `rc note "…"`, is a note the next runs read: what
slowed one, a quirk of the machine, a direction. Notes live in `var/retro.log` with the
workers' own look-backs, and the latest ride in the next runs' prompts, with the other
projects' digests, one line each.

Debug mode (on by default, the `⚙ debug` badge; `RHIPPER_DEBUG=off` or `rc run --no-debug` for
one run) asks every run to close its report with `## Faster next time`: at most three one-line
bullets naming what cost the run turns or time and the change that would have avoided it,
`[machine]` when the cause is the machine. The bullets land in `var/retro.log`, show as `↯`
lines in the window and ride in the next runs' machine notes; a recurring one becomes a
gotcha, a cost pattern a card.

### Evidence planning

`rc plan` plans the next cards from the runs' own records: the ledger's costliest cards, the
cards run more than once, the look-backs, the gotcha journal and the other projects' digests.
Each card cites its evidence and names a saving and a check. `rc plan <id>` asks the architect
model to split a milestone into cards and adds them on confirmation.

### The ledger and the plan's room

`var/usage.db` records every call: model, role, card, turns, tokens, cached tokens, cost.
`rc usage` prints the plan's windows first, then the ledger by model; `rc status` the
project's line. The provider keeps the plan's `rate_limit_event` in `var/limits.json`; the
status bar's week pill shows it. A walk stops when the week passes the project's
`[plan] threshold` (0.9 by default; `1.0` spends the whole week).

### The window

`rc-gui`, egui, the Blood Moon theme. One window on the machine, Chrome-style tabs:

- **The dashboard**: the machine's gauges in one row (CPU, memory, disk, tokens per task,
  today, all time); the worker row under them (the `BUILDERS − n +` count and each slot's
  holder, time and CPU share); then one bubble per project with its allocation controls
  (`fable / opus`, `runners − n +`, `priority − #n +`), its queue (with `absolute` badges) and
  its live runs, up to twelve, in a grid of two columns, each card held to its half. Idle
  projects fold to their header line.
- **The report**: what landed, by time or by project.
- **A tab per project**: the roadmap as nested bubbles, the card / context / ledger views, and
  the console, with one bubble per answer and run cards whose steps pin to the bottom while
  live.

Tabs are remembered across sessions. `rc open <dir>` hands a project to the running window or
starts it; `rc restart` brings the window back on a new build with the same tabs, the walks
running on undisturbed; `rc stop` closes one project's tab.

### The pair feed: an external agent on every project

`var/pair.log` is one line per event: `start #id …`, `end ✓|✗|↻ #id … · cost`, `review ✓|✗ #id …`,
`landed #id …`, `#id not landed: why`, `#id blocked: …`, `walk ran N card(s) · why it
stopped`, `note …`, `window up|down`. `rc pair --follow --since <ts>` follows it. A pairing
session (Claude Code in a terminal, say) watches the feeds of every project, acts on what lands
or stalls, adds cards and gambits, and leaves notes. The repository's `rhipper-pair` skill is
its playbook.

## A project's configuration

`rhipper.toml` beside `rhipper.md`:

```toml
[worker]
parallel = 4              # cards running at once, each in its own worktree (the dashboard's runners)
builds = 0                # the workers write code; the check compiles, on a worker slot
borrow = false            # whether the walk may take the pool's spare Opus slots
carry = ["local.properties"]   # untracked files a new worktree gets a copy of
allow = ["clang", "upx", "Bash(rc show*)"]   # commands beyond the agent's own allowlist
check = ".\\gw.ps1 :android-awt:compileDebugJavaWithJavac"   # the default check of a card that names none
lint = ""                 # no lint; a command runs it instead of clippy; absent, clippy over the changed crates
check_timeout = 1800      # seconds a check may run; 1200 without it
context = 160000          # a run restarts fresh once a turn carries this many tokens; absent, no cap
temp = ["ra-asm-emit-tests"]   # %TEMP% entries the landing sweeps once older than an hour
[worker.env]
CARGO_BUILD_JOBS = "6"

[role]                    # the cheap roles: summary, digest, classify
summary = "local/qwen"    # absent: the local server when it answers, else no call

[[route]]                 # the implementer (the dashboard's fable / opus toggle sets the first)
model = "claude-opus-5-5"

[plan]
threshold = 1.0           # the share of the plan's week a walk may spend; 0.9 by default
```

## Day to day

```powershell
rc init                      # or rc import plan.md: a roadmap from what is there
rc open .                    # the project as a tab of the one window
rc order runelite rhipper-asm   # which project the pool serves first
rc gambit add "…" --weight 5 --focus …
rc absolute 140              # this card works until it is done
rc walk                      # the walk runs the ready cards; rc walk --stop ends it after the current ones
rc pair --follow             # or a pairing session's watcher on var/pair.log
rc show 47                   # the verdict and the notes; rc set 47 defined to re-queue a blocked card
rc land 47                   # land an accepted card by hand
rc note "the oracle suite needs half an hour here"
rc install; rc restart       # a new build of the agent itself; the window comes back on its tabs
```

Several projects at once: open each as a tab, walk each, set each one's implementer, runners
and place in the order from the dashboard, and watch one feed over all of them.

## Why it works

- **The roadmap owns the work, not the conversation.** A card is the unit of context, of
  cost, of review and of memory. Nothing depends on a chat transcript surviving.
- **Everything around the model is local and free.** Context, outlines, checks, lint, diffs,
  handoffs, landings, gotchas, the ledger: no tokens. The model is spent on the change itself.
- **Parallel without chaos.** Worktrees keep the cards apart, the merge-before-run keeps them
  current, the landing proves the merged tree, and the worker slots keep the machine usable.
- **The system learns from its own records.** Look-backs, gotchas, costs and reruns feed the
  next prompts and the next plan; regeneration cards rewrite the roadmap from that evidence.
- **Verdicts, not diffs.** The reviewer reads the diff; the person reads one line per card.
- **Direction without micromanagement.** Gambits, priorities, the machine's order and the
  dashboard's allocation steer dozens of cards across projects.
- **Economics by construction.** Sessions resume so the cache holds the context; the context
  cap bounds what a turn costs; monumental cards run whole; cheap roles run locally.

## Crates

| Crate | Responsibility |
|---|---|
| `ra-roadmap` | Projects, milestones, sections, cards, dependencies, states, gambits, priorities, absolute cards; the `rhipper.md` store; the tree walks and the ranking |
| `ra-providers` | The `ModelProvider` trait; the Claude Code adapter (stream-json, the context watch, the gotcha hooks), the OpenRouter adapter and the local route; the plan's limits |
| `ra-tools` | The tools a bare model gets to act in a workspace: files, search, an allowlist of commands; the check runner; the worker slots' CPU share |
| `ra-usage` | The usage ledger: tokens and cost per call, model, role and card |
| `ra-gotchas` | The catalogue of recurring machine problems, the hooks that act on them, the journal |
| `ra-context` | The project context every run carries, a card's context and handoffs, the review block |
| `ra-console` | The worker that runs a card, the checks and the lint, the reviewer and the fix pass, the landing, the planner, the walks, the hub (pool, worker slots, priority order, window lock, digests), the project allocation, the pair feed, the commands, the MCP server, the terminal console |
| `rc` (app) | The CLI, every command above |
| `rc-gui` (app) | The window: the strip, the dashboard, the report, the roadmap column, the card view, the console |

## Install

A release on GitHub carries `rhipper-agent-<version>-setup.exe`, a per-user installer that
puts `rc` and `rc-gui` in `%LOCALAPPDATA%\Programs\rhipper` on the PATH (where `rc install`
puts them) with a Start Menu entry, and a zip of the same binaries with the README and the
pairing skill. The version is `0.2.<n>`, `n` the commit count of the tree the build is of.

Every push builds and tests on Windows (`.github/workflows/build.yml`: format, build, tests);
a `v*` tag, or the release workflow run by hand, makes the release
(`.github/workflows/release.yml`, `installer/rhipper.iss`).

## Build

```powershell
cargo build -p rc -p rc-gui
.\target\debug\rc.exe install     # builds and copies rc and rc-gui to %LOCALAPPDATA%\Programs\rhipper, on the PATH
rc restart                        # the window on the new build, same tabs; the walks keep running
cargo fmt --all; cargo test --workspace -q
```

`rc doctor` checks what a run needs: `claude` on the path and logged in, the hooks, the
settings a worker loads, git, the local server, the OpenRouter key, the roadmap, the ledger,
the landings and the temp directories. The MCP server (`rc mcp`, `.mcp.json`) serves the
roadmap and the workers to a Claude Code session; the workers themselves run with
`--strict-mcp-config`.
