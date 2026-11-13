use super::*;

fn none(_: &str) -> bool {
    false
}

fn here() -> std::path::PathBuf {
    std::env::current_dir().unwrap()
}

fn bash(command: &str) -> Shape {
    judge(command, Shell::Bash, &here(), &none)
}

fn powershell(command: &str) -> Shape {
    judge(command, Shell::PowerShell, &here(), &none)
}

fn denied_with(shape: &Shape, part: &str, form: &str) -> bool {
    matches!(shape, Shape::Denied(reason) if reason.contains(part) && reason.contains(form))
}

#[test]
fn a_pipe_with_a_redirect_passes_when_its_parts_do_and_a_write_is_sent_to_write() {
    assert_eq!(bash("cargo test -p ra-gotchas -q 2>&1 | tail -n 40"), Shape::Allowed);
    assert_eq!(bash("cargo build 2>/dev/null | grep error | head -5"), Shape::Allowed);
    assert_eq!(powershell("cargo test -p x 2>&1 | Select-Object -Last 40"), Shape::Allowed);
    assert_eq!(powershell("cargo clippy 2>$null"), Shape::Allowed);
    let shape = bash("cargo test 2>&1 | sed -n 1,20p");
    assert!(denied_with(&shape, "sed -n 1,20p", "Read"), "{shape:?}");
    let shape = bash("cargo test > out.txt");
    assert!(denied_with(&shape, "cargo test", "Write"), "{shape:?}");
    // a `>` inside quotes is text, not a redirect
    assert_eq!(bash("echo 'a > b' | wc -l"), Shape::Allowed);
}

#[test]
fn a_semicolon_chain_passes_when_its_parts_do() {
    assert_eq!(bash("git status; git diff --stat"), Shape::Allowed);
    assert_eq!(powershell("git log -3; Get-ChildItem crates"), Shape::Allowed);
    let shape = bash("git status; cd crates");
    assert!(denied_with(&shape, "cd crates", "relative"), "{shape:?}");
    let shape = powershell("Get-Content a.rs; Set-Content a.rs x");
    assert!(denied_with(&shape, "Set-Content", "Edit"), "{shape:?}");
}

#[test]
fn an_and_chain_passes_when_its_parts_do() {
    assert_eq!(bash("cargo fmt -p ra-gotchas && cargo test -p ra-gotchas -q"), Shape::Allowed);
    assert_eq!(bash("git diff || git status"), Shape::Allowed);
    let shape = bash("cargo test && git commit -m done");
    assert!(denied_with(&shape, "git commit", "person"), "{shape:?}");
    let shape = bash("git -C ../other status && ls");
    assert!(denied_with(&shape, "git -C", "-C <dir>"), "{shape:?}");
}

#[test]
fn a_worktree_read_and_an_index_only_rm_pass_and_the_rest_of_git_keeps_its_verdict() {
    assert_eq!(bash("git merge-base main HEAD"), Shape::Allowed);
    assert_eq!(bash("git -C var/worktrees/214 status"), Shape::Allowed);
    assert_eq!(powershell("git -C var\\worktrees\\214 log -3"), Shape::Allowed);
    assert_eq!(bash("git rm --cached foo"), Shape::Allowed);
    assert!(denied_with(&bash("git -C var/worktrees/214 push"), "git -C", "-C <dir>"));
    assert!(denied_with(&bash("git -C var/worktrees/../.. status"), "git -C", "-C <dir>"));
    assert!(denied_with(&bash("git rm foo"), "git rm", "person"));
    assert!(denied_with(&bash("git push --force"), "git push", "person"));
}

#[test]
fn an_env_prefix_is_the_command_it_heads() {
    assert_eq!(bash("RUST_BACKTRACE=1 cargo test -p ra-gotchas"), Shape::Allowed);
    assert_eq!(bash("A=1 B=two cargo build && tail -n 5 log.txt"), Shape::Allowed);
    let shape = bash("LC_ALL=C sed -i s/a/b/ x.rs");
    assert!(denied_with(&shape, "sed -i", "Edit"), "{shape:?}");
    // a variable read from the environment is not a value the hook has seen
    assert_eq!(bash("cat $HOME/notes"), Shape::Unknown);
}

#[test]
fn a_powershell_call_is_the_program_it_calls() {
    let cargo = here().join("tools").join("cargo.exe");
    assert_eq!(powershell(&format!("& \"{}\" test -p ra-gotchas", cargo.display())), Shape::Allowed);
    assert_eq!(powershell("& cargo build 2>&1 | Select-Object -Last 20"), Shape::Allowed);
    let shape = powershell("& taskkill.exe /PID 42");
    assert!(denied_with(&shape, "taskkill", "pid"), "{shape:?}");
    // a project's own script passes by its own rule
    let own = |line: &str| line.starts_with(".\\gw.ps1");
    assert_eq!(judge("& .\\gw.ps1 :test", Shell::PowerShell, &here(), &own), Shape::Allowed);
    assert_eq!(powershell("& .\\gw.ps1 :test"), Shape::Unknown);
}

#[test]
fn a_powershell_variable_passes_when_it_holds_a_value_the_command_set() {
    assert_eq!(powershell("$f = 'crates/ra-gotchas/src/lib.rs'; Get-Content $f -TotalCount 20"), Shape::Allowed);
    assert_eq!(powershell("$env:RUST_LOG='debug'; cargo test -p ra-gotchas"), Shape::Allowed);
    let shape = powershell("$f = 'x.rs'; Remove-Item $f");
    assert!(denied_with(&shape, "Remove-Item", "person"), "{shape:?}");
    // a variable the command did not set, or one run as a command, is not judged
    assert_eq!(powershell("Get-Content $profile"), Shape::Unknown);
    assert_eq!(powershell("& $tool test"), Shape::Unknown);
}

#[test]
fn a_path_inside_the_workspace_passes_and_one_outside_is_the_harnesss_to_judge() {
    let inside = here().join("crates");
    assert_eq!(bash(&format!("ls {}", inside.display())), Shape::Allowed);
    assert_eq!(powershell(&format!("Get-ChildItem \"{}\"", inside.display())), Shape::Allowed);
    assert_eq!(bash("cat ../secret.txt"), Shape::Unknown);
    assert_eq!(bash("cat /etc/passwd"), Shape::Unknown);
    assert_eq!(bash("cat ~/.ssh/id_rsa"), Shape::Unknown);
}

#[test]
fn a_shape_the_parser_does_not_read_is_left_to_the_allowlist() {
    for command in ["echo $(whoami)", "ls `pwd`", "(cd x && ls)", "cat <<EOF", "cargo build &", "echo \"open"] {
        assert_eq!(bash(command), Shape::Unknown, "{command}");
    }
    // a program the table does not know is not refused by the hook
    assert_eq!(bash("cargo test && xargs echo"), Shape::Unknown);
}

#[test]
fn the_rules_and_the_prompt_block_come_from_the_one_table() {
    let rules = rules();
    assert_eq!(rules[0], "Bash(cargo *)");
    assert!(rules.contains(&"PowerShell(Get-ChildItem*)".to_owned()), "{rules:?}");
    let block = block();
    assert!(block.starts_with("Allowed shell shapes"), "{block}");
    assert!(block.contains("Bash runs: cargo, rustfmt, git status"), "{block}");
    assert!(block.contains("sed, awk, perl: change a file with Edit"), "{block}");
    assert!(block.contains("`a && b`"), "{block}");
}
