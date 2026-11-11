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
- `apps/rc` — the CLI: `rc show|add|set|run|context|gotchas|usage|console|mcp|hook`
- `apps/rc-gui` — the egui workspace window (Blood Moon theme): roadmap column, card view, console band

## Build and checks

- `cargo test --workspace -q` is the check of a card unless its acceptance names another cargo command in backticks; run it before reporting.
- `cargo clippy --workspace` warns with the pedantic set; a change adds no warnings of its own. Casts and long functions already warn in `rc-gui`; do not add to them.
- Rust edition 2024, let chains allowed. `rustfmt.toml` sets the width; run `cargo fmt -p <crate>` on what you touched.
- A running console or window holds `rc.exe` / `rc-gui.exe`: build and test with `-p <crate>` or `--workspace --exclude rc --exclude rc-gui` when a workspace build fails to relink.
- `var/` and `rhipper.md` are untracked state; never commit them, never run `git commit` at all: the person reviews and commits.

## Conventions

- Match the surrounding code: its comment density, naming and idiom. Doc comments say what and why, not how; comments inside functions name the reason, not the step.
- Tests live in a `tests.rs` beside the code (`mod tests;`), named as sentences: `a_run_records_its_handoff`.
- Read the files the card names first, by the regions the outline points at; search with Grep and Glob before exploring; do not read `rhipper.md` itself, the card's text is in your prompt.
- Bash and PowerShell both run cargo, the read-only git commands and the read-only listing commands; pipe a long output through `tail`.
- Report what changed and what was verified in a few lines; the check decides whether the card is done, not the report.
