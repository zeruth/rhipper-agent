//! The prompt's shell block and the hook read one allowlist: a tool the block lists, or a form it
//! suggests in place of a refusal, never runs into a refusal of the hook.

use ra_gotchas::shape::{self, Shell};
use ra_gotchas::{Catalogue, Conditions, Verdict};

fn refused(catalogue: &Catalogue, command: &str) -> bool {
    matches!(catalogue.command(command, Conditions::default()), Verdict::Deny { .. })
}

#[test]
fn every_tool_the_prompt_lists_passes_the_hook() {
    let catalogue = Catalogue::seed();
    let block = shape::block();
    for (line, shell) in [("- Bash runs: ", Shell::Bash), ("- PowerShell runs: ", Shell::PowerShell)] {
        let list = block.lines().find_map(|l| l.strip_prefix(line)).unwrap_or_else(|| panic!("no `{line}` line in {block}"));
        for tool in list.split(", ") {
            let command = format!("{tool} x");
            assert!(!refused(&catalogue, &command), "{} lists `{tool}` and the hook refuses it", shell.tool());
        }
    }
}

/// The runtime forms a worker reaches for to run a script it wrote.
const RUNTIME_FORMS: &[&str] =
    &["python file.py", "python -c \"print(1)\"", "python3 file.py", "python3 -c \"print(1)\"", "py file.py", "py -c \"print(1)\""];

/// The programs the block offers a worker: its tool lists and the first word of every backticked
/// command a suggestion names (the text after a refusal's `: `).
fn offered(block: &str) -> Vec<String> {
    let mut programs = Vec::new();
    for line in block.lines() {
        if let Some(list) = line.strip_prefix("- Bash runs: ").or_else(|| line.strip_prefix("- PowerShell runs: ")) {
            programs.extend(list.split(", ").filter_map(|t| t.split_whitespace().next()).map(str::to_lowercase));
            continue;
        }
        let Some((_, form)) = line.split_once(": ") else { continue };
        for command in form.split('`').skip(1).step_by(2) {
            programs.extend(command.split_whitespace().next().map(str::to_lowercase));
        }
    }
    programs
}

#[test]
fn the_hook_refuses_python_file_and_python_c() {
    let catalogue = Catalogue::seed();
    for command in ["python file.py", "python -c \"print(1)\""] {
        assert!(refused(&catalogue, command), "the hook lets `{command}` run; the prompt may offer it again");
    }
}

#[test]
fn the_prompt_names_no_runtime_the_hook_refuses() {
    let catalogue = Catalogue::seed();
    let block = shape::block();
    let offered = offered(&block);
    for command in RUNTIME_FORMS {
        if !refused(&catalogue, command) {
            continue;
        }
        let program = command.split_whitespace().next().unwrap_or_default();
        assert!(!offered.iter().any(|p| p == program), "the prompt offers `{program}` and the hook refuses `{command}`:\n{block}");
    }
}
