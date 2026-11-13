# Rhipper Agent

A roadmap-driven, provider-independent AI development orchestrator in Rust. It breaks a project into milestones, sections and tasks, routes each task to a model, lets the model act in the project's working directory, and records what every call cost. The roadmap is `rhipper.md`; a task is a card on it; you are the worker of one card.

## Layout

- `crates/ra-roadmap` — the roadmap: nodes, states and the `rhipper.md` store (every mutation rewrites the file)
- `crates/ra-providers` — the `ModelProvider` trait; the Claude Code adapter (`claude -p`, the default route), OpenRouter and a local OpenAI-compatible route; the plan's limits
- `crates/ra-tools` — the workspace-confined tools a bare model gets: files, search, a short list of commands
- `crates/ra-usage` — the SQLite ledger of tokens and cost (`var/usage.db`)
- `crates/ra-gotchas` — the TOML catalogue of recurring machine problems and the pre/post tool hooks that act on them
- `crates/ra-context` — the project context (this file) and the per-task context files under `var/context/`
- `crates/ra-console` — the agent loop (`agent.rs`), the commands, the MCP server, the ratatui console
- `apps/rc` — the CLI: `rc show|add|set|run|context|gotchas|usage|console|mcp|hook`, every subcommand's handler in `apps/rc/src/main.rs` (`rc check`, `rc land`, `rc scratch`, `rc hold`, the hook itself), not under `crates/`
- the hub directory (`%LOCALAPPDATA%\rhipper\agent`, `hub::dir()`) is shared by every project on the machine: the window's lock and mailbox, and `notes.log`, where a `[machine]` look-back or `rc note "[machine] …"` lands for every project's runs to read (`retro.rs`)
- `apps/rc-gui` — the egui workspace window (Blood Moon theme): roadmap column, card view, console band

## Build and checks

- Every backticked command in a card's acceptance is its check (`cargo test -p x`, `python tools/x.py`, `.\gw.ps1 :task`); several run in the order written and stop at the first failure; `cargo test --workspace -q` when none is named. Run them before reporting.
- `cargo clippy --workspace` warns with the pedantic set; a change adds no warnings of its own. Casts and long functions already warn in `rc-gui`; do not add to them.
- Rust edition 2024, let chains allowed. `rustfmt.toml` sets the width; run `cargo fmt -p <crate>` on what you touched.
- A running console or window holds `rc.exe` / `rc-gui.exe`: build and test with `-p <crate>` or `--workspace --exclude rc --exclude rc-gui` when a workspace build fails to relink.
- `var/` and `rhipper.md` are untracked state; never commit them, never run `git commit` at all: the person reviews and commits.
- The machine has a scratch root (`rc scratch`, `D:\rhipper` here): cargo targets live under `<scratch>/target/<project>` (a worktree's under `wt-<id>` there), and runs get `TEMP` and `GRADLE_USER_HOME` under it. A test that asserts a target path must accept either place; read the target from `ra_providers::claude_code::project_target`, never assume `<root>/target`.
- A full `cargo test -p ra-console` takes about two minutes while other cards build, and a timing test can fail under that load: when one fails, run that test alone by name (`cargo test -p ra-console -q <name>`) before changing code for it.

## Conventions

- Match the surrounding code: its comment density, naming and idiom. Doc comments say what and why, not how; comments inside functions name the reason, not the step.
- Tests live in a `tests.rs` beside the code (`mod tests;`), named as sentences: `a_run_records_its_handoff`.
- A new field on `Node` touches every `Node { .. }` literal: `crates/ra-roadmap/src/store.rs` (two), `markdown.rs` (`node_of_line`), `markdown/tests.rs`, `tree/tests.rs`; `ra-console` tests call `markdown::parse` and read `Vec<Node>`. Give it `#[serde(default)]` and a line in `detail_lines`/`parse` of `markdown.rs`.
- The scripted provider and the `says(...)` fixture the agent tests use live in `crates/ra-console/src/commands/tests.rs`; the toolbox is scripted with `Worker::with_script`, so a test never compiles a temp project for real.
- Read the files the card names first, by the regions the outline points at; search with the Grep and Glob tools before exploring (`rg` is not on Git Bash's PATH here, the Grep tool is); do not read `rhipper.md` itself, the card's text is in your prompt.
- Bash and PowerShell both run cargo, the read-only git commands and the read-only listing commands; pipe a long output through `tail`.
- Report what changed and what was verified in a few lines; the check decides whether the card is done, not the report.
