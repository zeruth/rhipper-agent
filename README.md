# Rhipper Agent

A roadmap-driven, provider-independent AI development orchestrator in Rust. It breaks a project
into milestones, sections and tasks, routes each task to a model through OpenRouter or a local
inference server, and accepts work only after tests and an independent review pass.

The roadmap owns the work. Models are replaceable workers. The orchestrator owns coordination,
execution state and acceptance.

## Crates

| Crate | Responsibility |
|---|---|
| `ra-roadmap` | Projects, milestones, sections, tasks, dependencies, states; SQLite persistence |
| `ra-providers` | The `ModelProvider` trait and the OpenRouter adapter |
| `ra-usage` | Token and cost accounting per model and task |
| `ra-console` | The Blood Moon terminal UI |
| `rc` (app) | The CLI: `show`, `add`, `depend`, `set`, `console` |

## Run

```powershell
cargo run -p rc -- add project "My project"
cargo run -p rc -- add milestone "Phase 1" --parent 1
cargo run -p rc -- show
```

The roadmap database is `var/roadmap.db`; `--db` moves it.

## Status

Phase 1 of the plan in Rhipper-IDE's `docs/rhipper/`: the roadmap model, the store and the
OpenRouter adapter compile and the CLI exercises them. The console, the worker runtime, the
communication bus and the verification engine come next.

## License

AGPL-3.0-or-later
