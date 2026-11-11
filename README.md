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
| `ra-providers` | The `ModelProvider` trait, the Claude Code adapter and the OpenRouter adapter |
| `ra-tools` | The tools a bare model gets to act in a workspace: files, search, and an allowlist of commands |
| `ra-usage` | The usage ledger: tokens and cost per completion, model and task |
| `ra-console` | The Blood Moon terminal UI, the commands, and the worker that runs a task |
| `rc` (app) | The CLI: `show`, `add`, `depend`, `set`, `run`, `ping`, `usage`, `console`, `stop`, `mcp` |

## Run

```powershell
.\rhipper-agent.ps1
```

The script builds and launches the console. Without it:

```powershell
cargo run -p rc -- add task "Write the decoder" --parent 1
cargo run -p rc -- show
cargo run -p rc -- ping --model claude-opus-5-5
cargo run -p rc -- console
```

The roadmap file is `rhipper.md`; `--roadmap` moves it. The console follows edits from an outside
editor whenever the file's mtime moves. The ledger is `var/usage.db`; `rc usage` prints it by model.

## Workers

The model decides the route. A bare Claude id such as `claude-opus-5-5` runs on the local Claude
Code (`claude -p`), on the machine's own login, so a subscription pays for the work; the harness
brings its own tools and acts in the directory of the roadmap file. A vendor-prefixed id such as
`z-ai/glm-5.3-flash` goes through OpenRouter with the key from `OPENROUTER_API_KEY` or the
console's `key` command. A `local/<name>` id goes to a local OpenAI-compatible server such as
llama.cpp at `RHIPPER_LOCAL_URL` (default `http://127.0.0.1:8080/v1`), with no key; the id goes
out as typed, which a single-model server ignores. A bare model on either route gets the `ra-tools`
toolbox and runs through the tool loop.

A node can name its own model (`assign <id> <model>`), the files its change lands in (`files <id>
<path>...`, which the worker reads first), and a budget comes from its complexity (c1 $0.25 up to
c5 $5.00) or `rc run --budget`. A run past its budget fails the node and leaves a note with the
spend. `reassess` skips a node marked `final <id>` or one whose inputs have not changed since its
last assessment, with no model call.

In the console: `run <id>` executes a task, `reassess <id>` and `reassess-tree <id>` have the model
revise the objective and acceptance of what is still open, `model <id>` swaps the badge, and
`ping` proves the route. The `▶` and `↻` buttons on a selected card do the same. Every run draws
its own bubble in the log, with the harness's steps as they happen and the cost when it ends;
runs started from `rc run` or over MCP show there too, through `var/activity.log`.

## MCP

`rc mcp` serves the same tools over stdio for a client such as Claude Code: `show`, `add`,
`details`, `set`, `depend`, `run`, `reassess`, `model`, `ping` and `usage`. `.mcp.json` registers it
for this repository.

## Launching

`rhipper-agent.ps1` reopens itself in a Windows Terminal tab (the console paints in 24-bit color),
asks a running console to quit through `var/console.quit`, builds, and starts `rc console
--replace`. The build names itself `0.1.<n>` from `var/build-number`.

## Status

Phase 1 (Foundation) of the plan in Rhipper-IDE's `docs/rhipper/`: the roadmap, two providers, a
worker with tools, budgets, the ledger, and the console. The agent implements its own roadmap
cards; the reviews so far are in the git log. Not yet: a local acceptance check before a card
counts as done, session resume, a local model route, review workers, worker-to-worker messages,
parallel workers over git worktrees.

## License

AGPL-3.0-or-later
