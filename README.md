# Rhipper Agent

A roadmap-driven, provider-independent AI development orchestrator in Rust. It breaks a project
into milestones, sections and tasks, routes each task to a model, lets the model act in the
project's working directory, and records what every call cost.

The roadmap owns the work. Models are replaceable workers. The orchestrator owns coordination,
execution state and acceptance.

## Crates

| Crate | Responsibility |
|---|---|
| `ra-roadmap` | Projects, milestones, sections, tasks, dependencies, states; the `rhipper.md` store; the tree walks |
| `ra-providers` | The `ModelProvider` trait; the Claude Code adapter, the OpenRouter adapter and the local route; the plan's limits |
| `ra-tools` | The tools a bare model gets to act in a workspace: files, search, and an allowlist of commands |
| `ra-usage` | The usage ledger: tokens and cost per completion, model and task |
| `ra-gotchas` | The catalogue of recurring machine problems a run hits, the hooks that act on them, the journal |
| `ra-context` | The project context every run carries, and a task's context kept until its card is done |
| `ra-console` | The worker that runs a task, the commands, the MCP server, and the Blood Moon terminal UI |
| `rc` (app) | The CLI: `show`, `add`, `depend`, `set`, `run`, `ping`, `usage`, `context`, `gotchas`, `console`, `stop`, `mcp`, `hook` |
| `rc-gui` (app) | The workspace window: the roadmap column, the card view, the ledger and gotchas tab, the console band |

## Run

```powershell
.\rhipper-agent.ps1   # the terminal console
.\rhipper-gui.ps1     # the window
```

Each script builds and launches; a window and a console are one instance, so starting one asks
the other to quit (`var/console.quit`, no tokens). Without the scripts:

```powershell
cargo run -p rc -- add task "Write the decoder" --parent 1
cargo run -p rc -- show
cargo run -p rc -- run 7 --no-budget
cargo run -p rc -- console
```

The roadmap file is `rhipper.md`; `--roadmap` moves it. The console and the window follow edits
from an outside editor whenever the file's mtime moves. The ledger is `var/usage.db`; `rc usage`
prints the plan's windows first, then the ledger by model.

## Workers

The model decides the route. A bare Claude id such as `claude-fable-5-1` (the default) runs on
the local Claude Code (`claude -p`), on the machine's own login, so a subscription pays for the
work; the harness brings its own tools and acts in the directory of the roadmap file, with Bash
and PowerShell allowed for cargo, the read-only git commands and the read-only listing commands.
The worker loads the project's and the local Claude Code settings, not the user's: hooks written
for an interactive session can leave a headless worker without a shell. A vendor-prefixed id
such as `z-ai/glm-5.3-flash` goes through OpenRouter with the key from `OPENROUTER_API_KEY` or
the console's `key` command. A `local/<name>` id goes to a local OpenAI-compatible server such
as llama.cpp at `RHIPPER_LOCAL_URL` (default `http://127.0.0.1:8080/v1`), with no key. A bare
model on either route gets the `ra-tools` toolbox and runs through the tool loop. The cheap roles
a run ends with (the report condensed for the handoff, the run's digest line, whether an untagged
look-back is `[machine]`) go to the model the `[role]` table of `rhipper.toml` names, else to the
local server when it answers a ping, else to no model at all; `rc usage` shows them by model.

A run ends with a local check, no model call: the first backticked `cargo …` command in the
card's acceptance, else `cargo test --workspace -q`. Passed, the card is `implemented`; failed,
it is `needs_revision` with the output in its notes. The harness session of a run is kept on the
card, so the next run of it resumes where the last one stopped.

A card can name its own model (`assign <id> <model>`) and the files its change lands in
(`files <id> <path>...`). Budgets are on by default, from the complexity (c1 $0.25 up to c5
$5.00) or `rc run --budget`; a run past its budget fails the card and leaves a note with the
spend. On a subscription there is nothing to cap: `rc run --no-budget`, `budgets off` in the
console, or `RHIPPER_BUDGETS=off` turns them off. `reassess` skips a card marked `final <id>`
or one whose inputs have not changed since its last assessment.

In the console: `run <id>` executes a task, `reassess <id>` and `reassess-tree <id>` have the
model revise the objective and acceptance of what is still open, `model <id>` swaps the badge,
and `ping` proves the route. Every run draws its own bubble in the log, with the harness's
steps as they happen and the cost when it ends; runs started from `rc run` or over MCP show
there too, through `var/activity.log`.

## Context

A run does not send the worker to find out where the change lives. `rhipper.context.md` beside
the roadmap is the project context: the layout, the build and the checks, the conventions.
`rc context --init` drafts it from the workspace (the README, the crates and their descriptions,
the manifest's lints and edition); it is edited by hand after, and every run carries it.
`var/context/<id>.md` is a task's context: the brief of the card, an outline of each file it
names (the items with their line numbers, so the worker reads regions rather than files), and
a handoff from each run (the outcome, the check, the files the run changed, the first lines of
the report). The prompt carries the outlines and the last three handoffs; the files of done
cards go when a run ends. `rc context <id>` prints what a run would send.

## Gotchas

A gotcha is a recurring, machine-dependent problem a run hits: a command the allowlist refuses,
a binary a running console holds, a code page that mangles text. `var/gotchas.toml` (seeded
from `crates/ra-gotchas/gotchas.toml`) names each with a regular expression and a remedy: a
hint, a rewrite of the command, or a count. Claude Code's PreToolUse and PostToolUse hooks call
`rc hook pre|post`, which refuses or rewrites a command before it runs and adds the hint a
result calls for; the provider's stream reader counts what the run hit anyway. `var/gotchas.log`
is the journal; `rc gotchas` and the window's LEDGER tab show the counts and the refused
commands no gotcha names yet, so the catalogue grows from them. The machine notes of the next
run name the gotchas of the last runs. Nothing here costs a model call.

## Limits

Every Claude Code run reports the plan's windows (`rate_limit_event`); the provider keeps the
latest in `var/limits.json`. The console's status line, the window's status bar (the week as a
pill filled to its percent) and `rc usage` show them, so a person sees the week's room without
a model call.

## MCP

`rc mcp` serves the same tools over stdio for a client such as Claude Code: `show`, `add`,
`details`, `set`, `depend`, `run`, `reassess`, `model`, `ping` and `usage`. `.mcp.json` registers it
for this repository.

## Launching

`rhipper-agent.ps1` reopens itself in a Windows Terminal tab (the console paints in 24-bit color),
asks a running console to quit through `var/console.quit`, builds, and starts `rc console
--replace`. `rhipper-gui.ps1` does the same for the window, which starts maximized and follows
the card a run puts `in_progress`. The build names itself `0.1.<n>` from `var/build-number`.

## Status

Phases A to D of the agent's own roadmap: the roadmap, three routes, a worker with tools,
budgets that switch off, the ledger, the local check, session resume, the gotchas, the context
layer, the console and the window. The agent implements its own roadmap cards; the reviews are
in the git log. Not yet: review workers, worker-to-worker messages, parallel workers over git
worktrees.

## License

AGPL-3.0-or-later
