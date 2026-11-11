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
| `rc` (app) | The CLI: `show`, `add`, `depend`, `set`, `ping`, `usage`, `console` |

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

The badge model decides the route. A bare Claude id such as `claude-opus-5-5` runs on the local
Claude Code (`claude -p`), on the machine's own login, so a subscription pays for the work; the
harness brings its own tools and acts in the directory of the roadmap file. A vendor-prefixed id
such as `z-ai/glm-5.3-flash` goes through OpenRouter with the key from `OPENROUTER_API_KEY` or the
console's `key` command; a bare model gets the `ra-tools` toolbox and runs through the tool loop.

In the console: `run <id>` executes a task, `reassess <id>` and `reassess-tree <id>` have the model
revise the objective and acceptance of what is still open, `model <id>` swaps the badge, and
`ping` proves the route. The `▶` and `↻` buttons on a selected card do the same.

## Status

Phase 1 (Foundation) of the plan in Rhipper-IDE's `docs/rhipper/`: the roadmap, two providers, a
worker with tools, and the ledger. Not yet: independent testing and review workers, acceptance
gates, worker-to-worker messages, parallel workers over git worktrees.

## License

AGPL-3.0-or-later
