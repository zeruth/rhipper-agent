# Rhipper Agent

A roadmap-driven, provider-independent AI development orchestrator in Rust. It breaks a project
into milestones, sections and tasks, routes each task to a model through OpenRouter or a local
inference server, and accepts work only after tests and an independent review pass.

The roadmap owns the work. Models are replaceable workers. The orchestrator owns coordination,
execution state and acceptance.

## Crates

| Crate | Responsibility |
|---|---|
| `ra-roadmap` | Projects, milestones, sections, tasks, dependencies, states; the `rhipper.md` store |
| `ra-providers` | The `ModelProvider` trait and the OpenRouter adapter |
| `ra-usage` | Token and cost accounting per model and task |
| `ra-console` | The Blood Moon terminal UI: the usage pane, the roadmap tree, the log and the prompt |
| `rc` (app) | The CLI: `show`, `add`, `depend`, `set`, `console` |

## Run

```powershell
cargo run -p rc -- add task "Write the decoder" --parent 1
cargo run -p rc -- show
cargo run -p rc -- console
```

The roadmap file is `rhipper.md`; `--roadmap` moves it. A project opens the file in the working
directory, and the console follows edits from an outside editor on its next tick.

## Status

Phase 1 (Foundation) of the plan in Rhipper-IDE's `docs/rhipper/`. The roadmap lives in a
`rhipper.md` file: one numbered line per node, the detail fields indented beneath, dependencies
by id. The console shows the usage pane, a scrollable and clickable roadmap tree, and the log;
the prompt stays anchored at the bottom. The OpenRouter adapter, the worker runtime and the
verification engine come next.

## License

AGPL-3.0-or-later
