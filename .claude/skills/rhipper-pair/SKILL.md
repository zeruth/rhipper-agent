---
name: rhipper-pair
description: Pair with Rhipper Agent as the external architect session — run its cards, review and commit what its workers produce, iterate on the window, feed it notes, and coordinate with other sessions before changing the agent's own source. Use when working in any Rhipper project (a directory with rhipper.md), when using rc or rc-gui, or when asked to drive, review or improve the agent.
---

# Pairing with Rhipper Agent

Rhipper Agent is a roadmap-driven orchestrator: `rhipper.md` holds the cards, a worker (Claude
Code headless, `claude -p`, on the person's Max plan) implements one card per run, and the
orchestrator does the local work around it (context, checks, lint, the reviewer, handoffs). You
are the *external* session, usually over **every project on the machine at once**: you read
the roadmaps, start the walks, read the verdicts, accept, commit, re-queue, steer with gambits
and priorities, fix the small things in the agent, and keep the window good. The agent in turn
reads what you tell it. The README is the full picture; this is the playbook.

Standing rules from the person (2026-11-11):
- The person reads verdicts, not diffs. Report in a few lines what landed and what came back.
- Efficiency and local execution everywhere: tokens down, cheap roles local, nothing in the
  loop that a model call is not needed for. "A gotcha is anything standing in the way."
- **Leave the agent's own roadmap alone** once it is clear, with one exception: a serious
  token or performance saving found in a run or a review goes on it as a card at once and runs.
- One job per project (`rc walk`, never `--parallel`).
- No physical UI tests to debug the window (no screenshots, no mouse driving): read the code,
  instrument, fix. A screenshot is for showing the person, not for finding a bug.
- Never `git add -A` in a project while one of its runs is live: commit the session's own
  files by name, and a card's files after its run ends (a sweep put #169's work under a
  dashboard commit and the reviewer saw an empty diff).

## Where things are

| Thing | Place |
|---|---|
| The roadmap | `rhipper.md` (gitignored here; the project's own in other projects) |
| Project context every run reads | `rhipper.context.md` — edit it when the project moves |
| Per-task context + handoffs | `var/context/<id>.md` (brief, file outlines, one handoff per run, warnings) |
| Progress of the live run | `var/activity.log` (steps as `Read …`, `$ cmd`, `… words`, `⚠ gotcha: …`; `◆start`/`◆end` mark runs) |
| Gotchas (machine problems, no tokens) | catalogue `var/gotchas.toml` (seed in `crates/ra-gotchas/gotchas.toml`), journal `var/gotchas.log` |
| Look-backs and notes | `var/retro.log` — the workers' "Faster next time" bullets and the person's/your notes |
| Ledger, plan limits | `var/usage.db`, `var/limits.json` |
| Installed binaries | `%LOCALAPPDATA%\Programs\rhipper\{rc,rc-gui}.exe` (on PATH); desktop shortcut `Rhipper Agent.lnk` |

Everything under `var/` is per project. **One window on the machine hosts every project as a
tab** (Chrome-style strip at the top: name, a dot that burns while a run goes, ×, `+` for a
path; Ctrl+Tab / Ctrl+1..9 switch). The window holds `%LOCALAPPDATA%\rhipper\agent\gui.lock`;
its list of open projects is `open.txt` beside it, and a start with no arguments brings them
back; `open\*.txt` beside them is the mailbox `rc open` drops a path into.

**Open a project from a session**: `rc open <dir>` (or `rc open .`). It hands the directory to
the running window, which opens it as a tab (or brings the tab to the front), or starts the
installed `rc-gui` from `%LOCALAPPDATA%\Programs\rhipper` when none is running — on last
session's tabs plus this one. Never start `rc-gui.exe` yourself for this and never look for a
second window: there is one. The **dashboard** tab at the left of the strip shows every
project's running card (or next ready cards) and its latest run, on one page.

Within a project, one instance at a time: a console or a window's tab claims
`var/console.pid`; `rc stop` in the project directory asks only that one to leave (a window
closes the tab, not itself).

## The link: arm it first

The agent writes `var/pair.log` in the project — one line per event: `start #id title · model
· check: …`, `end ✓|✗|↻ #id … · turns · tokens · cost · the cap or the check`, `review ✓|✗ #id
… · N point(s)`, `walk ran N card(s) · why it stopped`, `note …`, `window up|down`. It cannot
push into a session, so the session subscribes. **First thing, arm ONE Monitor over every
project's feed** (the person: "stop setting per task monitors, set per project monitors long
term") and re-arm it only when it expires:

```
Monitor(command: "tail -n 0 -F C:/…/rhipper-agent/var/pair.log C:/…/rhipper-asm/var/pair.log C:/…/runelite/var/pair.log",
        description: "the three projects' pair feeds", timeout_ms: 1800000)
```

`tail -F` over several files prefixes each burst with `==> <file> <==`, which names the
project. Never one Monitor per card or per wait, never a `run_in_background` poll. `rc pair
--since <ts>` prints what landed while the Monitor was down; `rc pair` the last twenty. Do not
tail `var/activity.log` for this; that is the step-by-step stream the window draws.

## The cycle (what this session does for every card)

1. **Walk**: **`rc walk`** in the project walks every ready card in order (by priority, then
   the gambits, then the file). `rc walk` does not run the walk in your shell: the window that
   has the project starts it as a process of its own (detached, hours long, survives a window
   restart and your session's tool limits), and the tab shows it. `rc walk --stop` ends it
   after its current card; a second `rc walk` while one runs says so and does nothing. Never
   `Start-Process rc run --all` yourself. **A walk keeps the rc it started with**: after an
   install that matters, `rc walk --stop`, wait for the card's end line, `rc walk` again.
   `rc run <id>` runs one card in the foreground (background it and wait for the event).
2. **Read the end line and the review line**: `end ✓ … · review ✓` is done; `review ✗ … N
   point(s)` came back; `end ↻ … continues` hit a cap and the walk continues it by itself;
   `end ✗ … check failed` has the output at the head of the notes. `rc show <id>` prints the
   verdict and the points. Judge the reviewer, not the diff: a substantive rejection on a
   placeholder handoff ("the suite is still running") is right; a rejection for files it
   could not see (the reviewer diffs the card's files, the handoff's and the untracked among
   them) is wrong, and a `rc review <id>` after the fix judges again for cents.
3. **Accept or re-queue**: `rc set <id> accepted`, or `rc set <id> defined` to send it round
   again. A re-queued card is still a rerun: the review block at the head of its notes rides
   in the prompt and the run resumes the session it came back from. A card whose acceptance
   names something the person forbade (an APK build, a device) gets its acceptance rewritten
   with `rc edit <id> --acceptance "…"`, not a run that cannot pass.
4. **Lint**: a lint-only failure takes the cheap fix pass by itself. What is left is small
   (`# Errors` docs, `#[must_use]`, collapsible ifs): fix it yourself rather than pay a run.
5. **Test** the agent after its own cards: `cargo test --workspace -q`.
6. **Build + install (+ restart)**: `cargo build -q -p rc -p rc-gui; .\target\debug\rc.exe install`.
   A change to `rc` alone is live at once: runs are child processes of the installed `rc`. A
   change to the **window's own code** (`apps/rc-gui`, or `ra-console` code the window links)
   shows after `rc restart`: the one window quits whole and comes back on the new build with
   the same tabs, the same one in front. It is every session's window, so tell the others first
   (`ListAgents` + `SendMessage`, "restarting the window in a minute for <change>; your runs
   continue, your tab comes back") and wait a moment for an objection. Never `Get-Process
   rc-gui | CloseMainWindow`, `Stop-Process` or `rc stop` in every project to do this: `rc stop`
   is for leaving one project (its tab comes back next start all the same; the × on the tab
   is what forgets it). **When**: `rc install` any time a card lands (the next run is on it);
   `rc restart` only on a **milestone commit** — a phase's cards done and committed together —
   or when the person asks. Not per card: the window is every session's and a restart
   interrupts whoever is looking at it. The run's check line says which the card is:
   `window: restart on the milestone commit` (it touched `apps/rc-gui` or `crates/ra-console`),
   `rc: install any time, no restart` (it touched `apps/rc` or another crate), or nothing (no
   Rust code). Follow that line.
7. **Commit per card** with a message that says the card, what the worker did and what you
   fixed (the git log is the review record), `git commit -F <scratchpad file>`, **adding the
   card's files by name** (the handoff's `changed:` line and the card's `files:`), never
   `-A` while a run is live. Files another card still holds dirty stay out; say so in the
   message when a shared hunk rides along. Accept a milestone when its cards are.

During a walk do not edit files the live run names (`var/context/<id>.md` lists them).
Running `cargo test --workspace` while the worker's check runs can make *your* red test the
card's failure; re-check such verdicts.

## Steering: gambits, priorities, plans

- `rc gambits` lists the standing direction; `rc gambit add "…" --weight 1..5 --focus words`
  adds one; `rc gambit park|activate|done|weight|focus` change it. Focus words match a card's
  title, milestone and files as substrings: a word like `c` or `64` matches everything through
  file paths, so pick words that name the work (`emit`, `oracle`, `port`, `x64`). `rc next
  --why` shows the queue with the gambits that scored each card.
- `rc priority <id> 1..5` puts a card ahead of the gambits; `--clear` leaves it to them.
- A card: `rc add task "…" --parent <milestone> --objective … --acceptance … --files … --complexity N`.
  Backticked commands in the acceptance are the check. Cite evidence (an end line, a retro
  line, a cost) in the objective when the card is a saving.
- `rc plan` plans the next cards from the runs' own evidence under the last open milestone;
  `rc plan <id>` splits a milestone with the architect model and asks before adding.
- Economics (the person, 2026-11-11): no turn cap; every run resumes its session, reruns
  included; `[worker] context = 160000` ends a run on a handoff once a turn passes it, after
  the first twelve turns; a run whose last session hit it reading in goes uncapped. A turn's
  cost scales with the context it carries (148k a turn cost four times 41k). Read the `avg …
  tokens a turn, largest …` of the end lines; a serious waste is a card for the agent.

## Talking to the agent

- A line typed in the window or console **without** `/` is a **note**: it lands in
  `var/retro.log` as the person's, and the next runs read it with the workers' own look-backs.
  `rc note "…"` from a terminal. Use it for what you observe ("the tooltip is slow when coming
  from another panel", "cargo test relinks rc.exe while the window runs").
- Commands start with `/`: `/help`, `/run 7`, `/show`, `/set 7 accepted`, `/remove 7 [tree]`,
  `/effort 7 high`, `/assign 7 claude-opus-5-5`, `/install [dir] [desktop]`, `/stop` (the walk
  stops after the current task), `/quit`.
- **Debug mode** (on by default; the `⚙ debug` badge toggles it everywhere, `RHIPPER_DEBUG=off`,
  `rc run --no-debug`): the worker ends its report with `## Faster next time`, at most three
  one-line bullets naming what cost the run turns or time and the one change that would have
  avoided it, `[machine]` when the cause is this machine. They are parsed out of the report,
  kept in `var/retro.log`, shown as `↯` lines, and the latest five go to the next runs'
  machine notes; `rc plan` reads them as evidence. Act on them: a bullet that names a project
  fact belongs in `rhipper.context.md`; a recurring machine problem belongs in the gotcha
  catalogue; a stale one (a complaint about a check since fixed) can mislead a worker for a
  run, so correct it with a note.
- **Gotchas**: a `[[gotcha]]` has `where` (command / result / edit / text / answer), a regex
  `pattern`, a `message`, and a `remedy` (hint = deny with the message, rewrite, allow). The
  pre/post tool hooks (`rc hook pre|post`) and the stream reader apply them. Every refused
  command counts (the `refused` seed). Add one when `rc gotchas` shows the same refused command
  twice: a project-only one in the project's `var/gotchas.toml` (runelite's `no-apk-build`), a
  general one in the seed `crates/ra-gotchas/gotchas.toml`, which merges into every project's
  catalogue on the next run start.
- **MCP**: `.mcp.json` serves `show/add/details/set/depend/run/reassess/model/ping/usage` to a
  Claude Code session in this repo. Workers themselves run with `--strict-mcp-config`.

## The window (rc-gui, egui 0.37)

Blood Moon theme. Status bar: model, debug badge, next task, session tally, ledger, week pill.
Left: the roadmap as nested bubbles (project → phases parted by glowing rules → sections → task
rows; `tree|walk` toggle, state chips, search, arrows/Enter from an empty command line). Middle:
CARD / CONTEXT / LEDGER tabs. Bottom right: the console (one Markdown bubble per answer, red
bubbles for notes, run cards with jewel-tagged steps that pin to the bottom while live) and the
command line. The separators drag.

To show it to the person, capture **by window handle**, never the screen:
`powershell -NoProfile -ExecutionPolicy Bypass -File scripts\shot-window.ps1 -Process rc-gui -Out <png>`
(DPI-aware PrintWindow, the frame without its shadow), and Read the png. Not for debugging:
a layout bug is found in the code (egui 0.37: a Frame's outer size is content + inner margin
+ 2×stroke + outer margin; a resizable Panel in a Ui stores its content's width, so content
sized "available − margins" without the stroke grows 2 px a frame; measure with
`ui.min_rect()` and feed last frame's overshoot back, as the dashboard does). Vocabulary lives in `apps/rc-gui/src/cards.rs` (card, bubble, badge, jewel,
fade, divider, pill_bar, tooltip); the roadmap column is `draw_bubble`/`header_row` in
`apps/rc-gui/src/main.rs`; the console is `apps/rc-gui/src/console.rs`; Markdown in
`apps/rc-gui/src/markdown.rs`. egui gotchas met here: a single-line `TextEdit` gives up focus on
Enter before `has_focus()` is read (read the focus before `ui.add`); `with_layout` inside a Grid
cell takes the panel's whole height (allocate one text line); `Id::new` is deprecated for
`Id::unique`; `read_response(id)` only works for a widget interacted under that id.

## Machine notes (this machine)

- The person's global Claude Code hooks deny Bash and wrap PowerShell for interactive sessions;
  the worker skips them with `--setting-sources project,local`. For *you*, the PowerShell tool
  works and Bash output may be silent.
- Never write files with heredocs or `Set-Content`; use the Write/Edit tools. Commit messages via
  `git commit -F <file>`. `Remove-Item` on repo files is blocked; use `git clean -f -- <path>`.
- PowerShell 5.1 strips quotes from a JSON argument to an exe: pass `--settings` as a file.
- The ledger's `turns` column and other schema changes migrate on open; `var/` carries over.

## Working on the agent itself from another project

The agent's source is `C:\Users\tyler\Documents\code\rhipper-agent` (its own `rhipper.md` is the
agent's roadmap). A session using the agent in some other repo will meet things worth fixing in
it. Other sessions may be on it at the same time, one of them often with a walk running. Before
you change anything there:

1. **Look**: in the agent repo, `git status --short` (uncommitted work is someone's), the last
   `◆start`/`◆end` in `var/activity.log` (a start without an end is a live worker run),
   `var/console.pid` (the project is a tab of the window, or a console is open there).
2. **Ask**: `ListAgents`, then `SendMessage` to any session whose working directory is the agent
   repo: what you want to change, which files. Wait for the answer. If no session is listed and
   nothing is live, you are alone; say so in a note (`rc note "<session> taking <files> for
   <change>"` run in the agent repo) so the next one sees it in `var/retro.log`.
3. **Prefer the roadmap**: a change that is not urgent goes on the agent's roadmap as a card
   (`rc add task "…" --parent <phase>` in the agent repo, with objective, acceptance, files) and
   the agent implements it on its next walk. A note (`rc note`) is right for an observation
   without a fix.
4. **If you edit**: stay on files no live run names (`var/context/<id>.md` of the running card
   lists them), keep the cycle (test, build, `rc install`, commit with the message that says
   why), and tell the session you spoke to when you are done. Never `git stash`, reset or amend
   another session's work. `rc install` is how every project gets the new build: the next
   `rc run` is on it; the window picks it up when it is restarted (see step 5 of the cycle).
5. **The reply side**: if a session asks you about the agent repo, answer with what is live,
   what is uncommitted and whether the files they name are safe; agree a handoff or ask them
   to add a card instead.

## A project's own commands and environment

The worker's shell allowlist is the build, the read-only git and the read-only listing commands.
A project that needs more puts it in its `rhipper.toml` (beside `rhipper.md`, the same file as
the `[[route]]` rules):

```toml
[worker]
allow = ["rhtest", "Bash(ninja *)"]   # a bare name allows `name …` in both shells; a Claude Code rule is taken as is
[worker.env]
CARGO_BUILD_JOBS = "6"               # the harness runs with these on top of the agent's own
```

The extra commands are exempt from the `shell-outside-allowlist` gotcha (the other gotchas still
apply). The next `rc run` reads it; no restart.

**The check and the lint of a run** (since 0.1.491): every backticked command in a card's
acceptance is a check, run in the project directory in the order written, stopping at the
first failure; a card with none uses the project's default; a project with none uses
`cargo test --workspace -q`. The lint is clippy over the changed crates unless the project
names its own command or turns it off:

```toml
[worker]
check = ".\\gw.ps1 :android-awt:compileDebugJavaWithJavac"   # the default check of a card that names none
lint = ""                                                      # no lint here; a command runs it instead of clippy
check_timeout = 1800                                           # a suite that needs half an hour
context = 160000                                               # the context cap; absent, none; no `turns` anywhere
```

A check command written `.\gw.ps1` kept its backslash since a4835e1; before that it reached
PowerShell as `-File .gw.ps1` and sent three runelite cards back with their work complete
(`exit code -196608` in the notes was that).

`rc run <id> --dry` prints the resolved `check: … · lint: …`; the same line follows the
`◆start` marker in `var/activity.log` and ends the `start #id` event of `var/pair.log`.

## Reading the roadmap fast

`rc show` (states, computed `ready (defined)` / blocked), `rc show <id>`, `rc next` (the first
ready card), `rc context <id>` (what a run would send), `rc gotchas`, `rc usage`,
`rc run <id> --dry` (the whole prompt and the harness command line, no call).
