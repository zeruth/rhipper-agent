---
name: rhipper-pair
description: Pair with Rhipper Agent as the external architect session — run its cards, review and commit what its workers produce, iterate on the window, feed it notes, and coordinate with other sessions before changing the agent's own source. Use when working in any Rhipper project (a directory with rhipper.md), when using rc or rc-gui, or when asked to drive, review or improve the agent.
---

# Pairing with Rhipper Agent

Rhipper Agent is a roadmap-driven orchestrator: `rhipper.md` holds the cards, a worker (Claude
Code headless, `claude -p`, on the person's Max plan) implements one card per run, and the
orchestrator does the local work around it (context, lint, tests, handoffs). You are the
*external* session: you read the roadmap, start runs, review what lands, fix the small things,
commit, accept, and keep the window good. The agent in turn reads what you tell it.

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

Everything under `var/` is per project; two projects run two windows at once. One window or
console per project: a newcomer in the same directory replaces the other.

## The cycle (what this session does for every card)

1. **Run**: `rc run <id>` (foreground ~3–10 min; run it in the background and wait for the
   notification) or `rc run --all` to walk every ready card in order. No flags needed: nothing
   caps a run by default; debug mode is on.
2. **Read the result**: the run's lines (✓/✗, the check, `lint: N warning(s)`, `↯` look-back
   bullets), then `git diff --stat` and the diff of the files the handoff names.
3. **Lint**: a card comes back `needs_revision` when clippy warns on lines the run changed. The
   warnings are first in `var/context/<id>.md`. They are usually small (`# Errors` docs,
   `#[must_use]`, `format!` appends, collapsible ifs): fix them yourself, it is cheaper than a
   second run.
4. **Test**: `cargo test --workspace -q`.
5. **Build + install + relaunch**: `cargo build -q -p rc -p rc-gui; .\target\debug\rc.exe install`
   then close **this project's** window with `rc stop` run in the project directory (it asks
   the window named in `var/console.pid` to quit; never `Get-Process rc-gui | CloseMainWindow`,
   which closes every project's window on the machine, other sessions' included) and start
   `%LOCALAPPDATA%\Programs\rhipper\rc-gui.exe` with the project as working directory. The
   installed binaries never lock `target/`; installing over a running window is fine, and the
   other projects' windows keep running the old build until their own relaunch.
6. **Commit** with a message that says the card, the model, the cost line, what the worker did
   and what you fixed (the git log is the review record), with `git commit -F <scratchpad file>`.
7. **Accept**: `rc set <id> accepted`. Accept a milestone when its cards are.

During a walk (`--all`) do not edit files the worker may be on: review and commit per card
after the walk ends. Running `cargo test --workspace` while the worker's check runs can make
*your* red test the card's failure; re-check such verdicts.

## Talking to the agent

- A line typed in the window or console **without** `/` is a **note**: it lands in
  `var/retro.log` as the person's, and the next runs read it with the workers' own look-backs.
  `rc note "…"` from a terminal. Use it for what you observe ("the tooltip is slow when coming
  from another panel", "cargo test relinks rc.exe while the window runs").
- Commands start with `/`: `/help`, `/run 7`, `/show`, `/set 7 accepted`, `/remove 7 [tree]`,
  `/effort 7 high`, `/assign 7 claude-opus-5-5`, `/install [dir] [desktop]`, `/stop` (the walk
  stops after the current task), `/quit`.
- **Debug mode** (status-bar badge, `RHIPPER_DEBUG=off`, `rc run --no-debug`): the worker ends
  its report with `## Faster next time`, at most three one-line bullets, `[machine]` when the
  cause is this machine. They are parsed, kept, shown as `↯` lines, and the latest five go to
  the next runs' machine notes. Act on them: a bullet that names a project fact belongs in
  `rhipper.context.md`; a recurring machine problem belongs in the gotcha catalogue.
- **Gotchas**: a `[[gotcha]]` has `where` (command / result / edit / text / answer), a regex
  `pattern`, a `message`, and a `remedy` (hint = deny with the message, rewrite, allow). The
  pre/post tool hooks (`rc hook pre|post`) and the stream reader apply them. Add one when the
  journal's `unmatched-denied` shows the same refused command twice. Edit the seed and copy it to
  `var/gotchas.toml`.
- **MCP**: `.mcp.json` serves `show/add/details/set/depend/run/reassess/model/ping/usage` to a
  Claude Code session in this repo. Workers themselves run with `--strict-mcp-config`.

## The window (rc-gui, egui 0.37)

Blood Moon theme. Status bar: model, debug badge, next task, session tally, ledger, week pill.
Left: the roadmap as nested bubbles (project → phases parted by glowing rules → sections → task
rows; `tree|walk` toggle, state chips, search, arrows/Enter from an empty command line). Middle:
CARD / CONTEXT / LEDGER tabs. Bottom right: the console (one Markdown bubble per answer, red
bubbles for notes, run cards with jewel-tagged steps that pin to the bottom while live) and the
command line. The separators drag.

To look at it, capture **by window handle**, never the screen:
`scripts/shot-window.ps1 -Process rc-gui -Out <png>` then `scripts/crop.ps1 -In -Out -X -Y -W -H`,
and Read the png. Vocabulary lives in `apps/rc-gui/src/cards.rs` (card, bubble, badge, jewel,
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
   `var/console.pid` (a window or console is open there).
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
   another session's work. `rc install` is how every project gets the new build; the running
   windows keep going and pick it up on their next launch.
5. **The reply side**: if a session asks you about the agent repo, answer with what is live,
   what is uncommitted and whether the files they name are safe; agree a handoff or ask them
   to add a card instead.

## Reading the roadmap fast

`rc show` (states, computed `ready (defined)` / blocked), `rc show <id>`, `rc next` (the first
ready card), `rc context <id>` (what a run would send), `rc gotchas`, `rc usage`,
`rc run <id> --dry` (the whole prompt and the harness command line, no call).
