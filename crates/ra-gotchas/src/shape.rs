//! The shapes of a shell command: a command is split into its simple commands (the parts of a
//! pipe, a `;`, `&&` or `||` chain) and passes when every part does. The harness's static
//! allowlist judges a command by its text and refused a chain, an env prefix or a PowerShell call
//! of allowed parts for their shape alone (362 refusals in the journals of 2026-11-11 and 12), so
//! the pre-tool hook answers for the shapes it can parse: allow when every part is allowed, deny
//! with the form to use instead when one is refused, and nothing when it cannot tell, where the
//! static allowlist still decides.
//!
//! One table serves three readers: the permission rules a worker's harness gets ([`rules`]), the
//! hook's judgement ([`judge`]), and the block of the worker's prompt that says the rules before
//! a refusal has to ([`block`]).

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

/// The shell of the tool a command came through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    PowerShell,
}

impl Shell {
    /// The shell of a harness tool by its name; `None` for a tool that is no shell.
    #[must_use]
    pub fn of_tool(name: &str) -> Option<Self> {
        match name {
            "Bash" => Some(Self::Bash),
            "PowerShell" => Some(Self::PowerShell),
            _ => None,
        }
    }

    /// The tool's name, as a permission rule names it.
    #[must_use]
    pub fn tool(self) -> &'static str {
        match self {
            Self::Bash => "Bash",
            Self::PowerShell => "PowerShell",
        }
    }

    fn allowed(self) -> &'static [&'static str] {
        match self {
            Self::Bash => BASH,
            Self::PowerShell => POWERSHELL,
        }
    }
}

/// The commands a worker's Bash runs: the build, the read-only git commands, the read-only
/// shell, and the text filters a pipe ends in. A rule ending in ` *` takes the word alone or
/// with arguments; one ending in `*` any text after it.
const BASH: &[&str] = &[
    "cargo *",
    "rustfmt *",
    "git status*",
    "git diff*",
    "git log*",
    "git show*",
    "ls*",
    "cat *",
    "head *",
    "tail *",
    "wc *",
    "grep *",
    "rg *",
    "find *",
    "pwd*",
    "tree*",
    "mkdir *",
    "echo *",
    "git check-ignore*",
    // the read-only git a card's history needs: where its changes start (refused to #223) and
    // what a branch holds
    "git merge-base*",
    "git rev-list*",
    "git rev-parse*",
    "git ls-files*",
    "git ls-tree*",
    "git blame*",
    "git branch --contains*",
    "git rm --cached*",
    "sort*",
    "uniq*",
    "cut *",
    "paste *",
    "tr *",
    "sha256sum *",
    // the one sanctioned wait on a long job of the worker's own
    "rc wait *",
];

/// The commands a worker's PowerShell runs, as [`BASH`] for its cmdlets.
const POWERSHELL: &[&str] = &[
    "cargo *",
    "rustfmt *",
    "git status*",
    "git diff*",
    "git log*",
    "git show*",
    "Get-ChildItem*",
    "Get-Content *",
    "Select-String *",
    "Select-Object *",
    "Test-Path *",
    "Get-Location*",
    "New-Item *",
    "Write-Output *",
    "git check-ignore*",
    "git merge-base*",
    "git rev-list*",
    "git rev-parse*",
    "git ls-files*",
    "git ls-tree*",
    "git blame*",
    "git branch --contains*",
    "git rm --cached*",
    "rc wait *",
];

/// The programs refused in either shell, each group with the form to use instead.
const REFUSED: &[(&[&str], &str)] = &[
    (
        &["sed", "awk", "perl"],
        "change a file with Edit and read one with Read (a pattern with Grep), not sed or awk; only a print-only `sed -n '1,20p' file` runs",
    ),
    (
        &["cd", "pushd", "Set-Location", "Push-Location"],
        "the shell starts in the workspace: name the paths relative to it instead of changing directory",
    ),
    (
        &["tee", "Set-Content", "Add-Content", "Out-File", "Tee-Object"],
        "write a file with Write and change one with Edit, not a shell writer",
    ),
    (&["rm", "rmdir", "del", "Remove-Item"], "a deletion is the person's: name the file in the report"),
    (&["Stop-Process", "taskkill"], "a process cannot be killed from here: name its pid in the report"),
    (
        &["bash", "sh", "cmd", "powershell", "pwsh", "Start-Process", "Invoke-Expression", "iex"],
        "run the command itself in the Bash or PowerShell tool, not under another shell",
    ),
];

/// The git subcommands that change the state, the person's to run.
const GIT_WRITES: &[&str] = &[
    "stash",
    "checkout",
    "switch",
    "restore",
    "reset",
    "clean",
    "add",
    "commit",
    "rebase",
    "merge",
    "push",
    "pull",
    "cherry-pick",
    "revert",
    "rm",
    "mv",
];

/// The git subcommands a worktree is read with from outside it.
const GIT_READS: &[&str] = &["status", "log", "diff", "show"];

const GIT_FORM: &str = "the git state is the person's: only the read-only git commands run here (status, diff, log, show)";
const GIT_C_FORM: &str = "git runs in the workspace already: drop the `-C <dir>`";
const REDIRECT_FORM: &str =
    "write a file with Write and change one with Edit, not a `>` redirect (`> /dev/null`, `2>$null` and `2>&1` pass)";
const STDIN_FORM: &str = "write the script to a file with Write and name it in the report: the hook refuses a runtime";

/// The permission rules of the table, Bash's first, as the harness's `--allowedTools` takes them.
#[must_use]
pub fn rules() -> Vec<String> {
    [Shell::Bash, Shell::PowerShell]
        .iter()
        .flat_map(|&shell| shell.allowed().iter().map(move |r| format!("{}({r})", shell.tool())))
        .collect()
}

/// What the hook makes of a command's shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// Every part is allowed: the hook answers allow.
    Allowed,
    /// A part is refused; the reason names it and the form to use instead.
    Denied(String),
    /// A part the table does not know, a path outside the workspace, or a shape the parser does
    /// not read: the hook says nothing and the static allowlist decides.
    Unknown,
}

/// One simple command of a command line.
#[derive(Debug, Default)]
struct Part {
    words: Vec<String>,
    /// Behind a PowerShell `&` call operator.
    call: bool,
    /// A `>` redirect writes a file, not a discard.
    writes: bool,
}

/// How `command` fares in `shell` run from `workspace`: the parts the table allows, or `own`
/// (the project's `[worker] allow`) does, pass; the first refused part denies the whole.
#[must_use]
pub fn judge(command: &str, shell: Shell, workspace: &Path, own: &dyn Fn(&str) -> bool) -> Shape {
    let target = std::env::var_os("CARGO_TARGET_DIR").filter(|d| !d.is_empty()).map(std::path::PathBuf::from);
    // the run's TEMP is `<scratch>/tmp/<project>/run-<id>`: its parent is the project's tmp root
    let tmp = std::env::var_os("TEMP")
        .map(std::path::PathBuf::from)
        .filter(|t| t.file_name().is_some_and(|n| n.to_string_lossy().starts_with("run")))
        .and_then(|t| t.parent().map(Path::to_path_buf));
    let roots: Vec<&Path> = target.iter().chain(tmp.iter()).map(std::path::PathBuf::as_path).collect();
    judge_roots(command, shell, workspace, own, &roots)
}

/// [`judge`] with the card's own `target` named: listing it and running a program under it pass,
/// while any other directory outside the workspace stays the harness's to judge.
#[must_use]
pub fn judge_in(command: &str, shell: Shell, workspace: &Path, own: &dyn Fn(&str) -> bool, target: Option<&Path>) -> Shape {
    let roots: Vec<&Path> = target.into_iter().collect();
    judge_roots(command, shell, workspace, own, &roots)
}

fn judge_roots(command: &str, shell: Shell, workspace: &Path, own: &dyn Fn(&str) -> bool, target: &[&Path]) -> Shape {
    let Some(parts) = split(command, shell) else { return Shape::Unknown };
    if parts.is_empty() {
        return Shape::Unknown;
    }
    let mut all = true;
    // the PowerShell variables the command sets itself, to a value judged where it is set
    let mut assigned: Vec<String> = Vec::new();
    for part in &parts {
        match judge_part(part, shell, workspace, own, target, &mut assigned) {
            Shape::Allowed => {}
            Shape::Denied(form) => return Shape::Denied(format!("`{}` is refused here: {form}", part.words.join(" "))),
            Shape::Unknown => all = false,
        }
    }
    if all { Shape::Allowed } else { Shape::Unknown }
}

fn judge_part(
    part: &Part,
    shell: Shell,
    workspace: &Path,
    own: &dyn Fn(&str) -> bool,
    target: &[&Path],
    assigned: &mut Vec<String>,
) -> Shape {
    let in_target = |w: &str| target.iter().any(|t| in_target(w, t));
    let away = |w: &str| outside(w, workspace) && !in_target(w);
    if part.writes {
        return Shape::Denied(REDIRECT_FORM.to_owned());
    }
    let raw = part.words.join(" ");
    if own(&raw) || (part.call && own(&format!("& {raw}"))) {
        return Shape::Allowed;
    }
    // an env prefix sets a variable for the command it heads; alone it only sets it
    let words: Vec<&str> = match shell {
        Shell::Bash => part.words.iter().map(String::as_str).skip_while(|w| is_assignment(w)).collect(),
        Shell::PowerShell => part.words.iter().map(String::as_str).collect(),
    };
    let Some((&first, rest)) = words.split_first() else { return Shape::Allowed };
    if shell == Shell::PowerShell && first.starts_with('$') {
        // `$name = 'value'` and `$env:NAME='value'`; a variable used as a command is not judged
        let (name, value) = first[1..].split_once('=').unwrap_or((&first[1..], ""));
        let values: Vec<&str> = match rest.split_first() {
            Some((&"=", values)) if !first.contains('=') => values.to_vec(),
            _ if first.contains('=') => std::iter::once(value).chain(rest.iter().copied()).collect(),
            _ => return Shape::Unknown,
        };
        if part.call || values.iter().any(|w| away(w) || !known(w, shell, assigned)) {
            return Shape::Unknown;
        }
        assigned.push(name.to_lowercase());
        return Shape::Allowed;
    }
    // a program by its path is the program: `& "C:\…\cargo.exe" test` is `cargo test`
    let name = program(first);
    if name == "git" && rest.first() == Some(&"mv") {
        return judge_move(&rest[1..], shell, workspace, assigned);
    }
    if name == "git" && rest.first() == Some(&"rm") && !rest.contains(&"--cached") {
        return judge_remove(&rest[1..], shell, workspace, assigned);
    }
    if let Some(form) = refusal(name, rest) {
        return Shape::Denied(form.to_owned());
    }
    // a print script is no path, and its `$` is the last line, not a variable
    if let Some(files) = sed_prints(name, rest) {
        let fine = files.iter().all(|w| !away(w) && known(w, shell, assigned));
        return if fine { Shape::Allowed } else { Shape::Unknown };
    }
    if words.iter().any(|w| away(w) || !known(w, shell, assigned)) {
        return Shape::Unknown;
    }
    if mid_check(name, rest) {
        return Shape::Allowed;
    }
    // the program is the card's own build output
    if in_target(first) && !part.call {
        return Shape::Allowed;
    }
    let line = std::iter::once(name).chain(rest.iter().copied()).collect::<Vec<_>>().join(" ");
    if own(&line) {
        return Shape::Allowed;
    }
    if is_worktree_read(&line) {
        return Shape::Allowed;
    }
    let fold = shell == Shell::PowerShell;
    if shell.allowed().iter().any(|rule| matches(rule, &line, fold)) { Shape::Allowed } else { Shape::Unknown }
}

/// `git mv` with `args` (after `mv`): allowed when it names a source and a destination and every
/// path stays inside the workspace; a path outside it is refused.
fn judge_move(args: &[&str], shell: Shell, workspace: &Path, assigned: &[String]) -> Shape {
    let paths: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-')).collect();
    if let Some(path) = args.iter().find(|w| outside(w, workspace)) {
        return Shape::Denied(format!("`git mv` runs inside the workspace only, and `{path}` leaves it: {GIT_FORM}"));
    }
    if paths.len() < 2 || args.iter().any(|w| !known(w, shell, assigned)) {
        return Shape::Unknown;
    }
    Shape::Allowed
}

/// Whether `git` run in `workspace` with `args` succeeds.
fn git_ok(workspace: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(workspace)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `git rm` with `args` (after `rm`): allowed when every path stays inside the workspace and is
/// tracked there; a forced removal also needs the paths unchanged since the last commit.
fn judge_remove(args: &[&str], shell: Shell, workspace: &Path, assigned: &[String]) -> Shape {
    let flag = |w: &str| w.starts_with('-') && w != "--";
    let known_flag = |w: &str| matches!(w, "-r" | "-f" | "-q" | "-rf" | "-fr" | "--force" | "--quiet");
    if let Some(path) = args.iter().find(|w| outside(w, workspace)) {
        return Shape::Denied(format!("`git rm` runs inside the workspace only, and `{path}` leaves it: {GIT_FORM}"));
    }
    if args.iter().filter(|w| flag(w)).any(|w| !known_flag(w)) {
        // a flag that is not a plain remove (`--ignore-unmatch`, `--cached` is handled apart)
        return Shape::Denied(GIT_FORM.to_owned());
    }
    let forced = args.iter().any(|w| matches!(*w, "-f" | "-rf" | "-fr" | "--force"));
    let paths: Vec<&str> = args.iter().copied().skip_while(|w| *w != "--").skip(1).collect();
    let paths = if paths.is_empty() { args.iter().copied().filter(|w| !w.starts_with('-')).collect() } else { paths };
    if paths.is_empty() || args.iter().any(|w| !known(w, shell, assigned)) {
        return Shape::Unknown;
    }
    for path in paths {
        if !git_ok(workspace, &["ls-files", "--error-unmatch", "--", path]) {
            return Shape::Denied(format!("`git rm` takes a tracked path, and `{path}` is not tracked here: {GIT_FORM}"));
        }
        if forced && !git_ok(workspace, &["diff", "--quiet", "HEAD", "--", path]) {
            return Shape::Denied(format!("`git rm` of `{path}` is forced over uncommitted changes: {GIT_FORM}"));
        }
    }
    Shape::Allowed
}

/// Whether `name` with `rest` is a mid-run check of a card, `rc check <id> --mid`: the run of
/// its card's own check a worker may ask for, which `rc` caps a run and puts on a build slot,
/// so it passes where the builds are refused. A bare `rc check <id>` runs uncapped and stays
/// the harness's to judge.
fn mid_check(name: &str, rest: &[&str]) -> bool {
    name.eq_ignore_ascii_case("rc") && matches!(rest, ["check", id, "--mid"] | ["check", "--mid", id] if id.parse::<i64>().is_ok())
}

/// Whether `command` is a mid-run check of a card, as [`mid_check`] judges it.
#[must_use]
pub fn is_mid_check(command: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    words.split_first().is_some_and(|(&first, rest)| mid_check(program(first), rest))
}

/// The form to use instead of a refused program, when `name` with `rest` is one.
fn refusal(name: &str, rest: &[&str]) -> Option<&'static str> {
    if sed_prints(name, rest).is_some() {
        return None;
    }
    if let Some(form) = REFUSED.iter().find(|(names, _)| names.iter().any(|n| n.eq_ignore_ascii_case(name))).map(|(_, form)| *form) {
        return Some(form);
    }
    if name == "git" {
        if rest.first() == Some(&"-C") {
            return (!worktree_read(rest)).then_some(GIT_C_FORM);
        }
        // `rm --cached` only takes a path out of the index; the file stays
        if rest.starts_with(&["rm", "--cached"]) {
            return None;
        }
        if rest.first().is_some_and(|sub| GIT_WRITES.contains(sub))
            || rest.starts_with(&["branch", "-d"])
            || rest.starts_with(&["branch", "-D"])
        {
            return Some(GIT_FORM);
        }
    }
    let python = ["python", "python3", "py"].iter().any(|p| p.eq_ignore_ascii_case(name));
    (python && (rest.is_empty() || rest.first() == Some(&"-"))).then_some(STDIN_FORM)
}

/// One print command of a `sed -n` script: an optional line number, `$` or `/pattern/` address
/// or range of them, then `p`.
static SED_PRINT: LazyLock<Regex> = LazyLock::new(|| {
    let address = r"(\d+|\$|/([^/\\]|\\.)*/)";
    Regex::new(&format!(r"^({address}(,{address})?)?p$")).expect("the sed print pattern parses")
});

/// The files of a `sed` that only prints, `sed -n '1,20p' f`: `-n` and one script of print
/// commands, so it neither writes a file (`-i`, `w`) nor runs one (`e`); `None` for any other
/// `sed`. A `$` the shell would read as a variable (`$p`) is no print script: its value is unseen.
fn sed_prints<'a, 'b>(name: &str, rest: &'a [&'b str]) -> Option<&'a [&'b str]> {
    if !name.eq_ignore_ascii_case("sed") {
        return None;
    }
    let at = rest.iter().position(|w| !w.starts_with('-'))?;
    let (flags, script, files) = (&rest[..at], rest[at], &rest[at + 1..]);
    let short = |f: &str| f.len() > 1 && !f.starts_with("--") && f[1..].chars().all(|c| matches!(c, 'n' | 'E' | 'r' | 's'));
    let quiet = |f: &str| matches!(f, "--quiet" | "--silent") || (short(f) && f.contains('n'));
    let flags_read = flags.iter().all(|f| quiet(f) || short(f) || *f == "--regexp-extended");
    let variable =
        script.match_indices('$').any(|(at, _)| script[at + 1..].starts_with(|c: char| c.is_ascii_alphabetic() || matches!(c, '_' | '{')));
    let prints = !script.is_empty() && script.split(';').all(|command| SED_PRINT.is_match(command.trim()));
    (flags_read && flags.iter().any(|f| quiet(f)) && !variable && prints && !files.iter().any(|f| f.starts_with('-'))).then_some(files)
}

/// Whether `command` starts with a `sed` that only prints, as [`sed_prints`] judges it: the
/// shell-outside-allowlist gotcha stands aside for it.
#[must_use]
pub fn is_sed_print(command: &str) -> bool {
    let Some(parts) = split(command, Shell::Bash) else { return false };
    let Some(first) = parts.first() else { return false };
    let words: Vec<&str> = first.words.iter().map(String::as_str).skip_while(|w| is_assignment(w)).collect();
    words.split_first().is_some_and(|(&name, rest)| !first.writes && sed_prints(program(name), rest).is_some())
}

/// Whether the arguments of a git command (after `git`) read a card's worktree from outside it:
/// `-C var/worktrees/<id>` and a read-only subcommand. The path is relative to the project and
/// stays under `var/worktrees/`.
fn worktree_read(rest: &[&str]) -> bool {
    let [dash_c, path, sub, ..] = rest else { return false };
    let path = path.trim_matches(['"', '\'']).replace('\\', "/");
    let path = path.strip_prefix("./").unwrap_or(&path);
    *dash_c == "-C"
        && path.strip_prefix("var/worktrees/").is_some_and(|id| !id.is_empty() && !id.split('/').any(|s| s == ".."))
        && GIT_READS.contains(sub)
}

/// Whether `command` is a `git -C var/worktrees/<id>` read, as [`worktree_read`] judges it: the
/// shell-outside-allowlist gotcha stands aside for it.
#[must_use]
pub fn is_worktree_read(command: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    words.split_first().is_some_and(|(&first, rest)| program(first) == "git" && worktree_read(rest))
}

/// Whether a permission rule takes `line`: `cargo *` the word alone or with arguments, `ls*`
/// any text after the prefix, anything else the line as it is.
fn matches(rule: &str, line: &str, fold: bool) -> bool {
    let starts =
        |prefix: &str| line.get(..prefix.len()).is_some_and(|head| if fold { head.eq_ignore_ascii_case(prefix) } else { head == prefix });
    if let Some(base) = rule.strip_suffix(" *") {
        starts(base) && line[base.len()..].chars().next().is_none_or(char::is_whitespace)
    } else if let Some(prefix) = rule.strip_suffix('*') {
        starts(prefix)
    } else {
        starts(rule) && line.len() == rule.len()
    }
}

/// The program a word names: its file name without `.exe`.
fn program(word: &str) -> &str {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    match name.len().checked_sub(4) {
        Some(at) if name.get(at..).is_some_and(|ext| ext.eq_ignore_ascii_case(".exe")) => &name[..at],
        _ => name,
    }
}

/// Whether every variable a word reads has a value the hook has seen: none in Bash, whose
/// variables come from the environment; in PowerShell `$null`, `$true`, `$false` and the ones
/// the command set before.
fn known(word: &str, shell: Shell, assigned: &[String]) -> bool {
    word.match_indices('$').all(|(at, _)| {
        let name: String = word[at + 1..].chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':')).collect();
        let name = name.to_lowercase();
        shell == Shell::PowerShell && (["null", "true", "false"].contains(&name.as_str()) || assigned.contains(&name))
    })
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Whether a word names a path outside `workspace`: an absolute one not under it (Git Bash's
/// `/c/…` read as `C:/…`), or one that climbs out with `..`. A flag's value after `=` is read
/// as its own word.
fn outside(word: &str, workspace: &Path) -> bool {
    let path = word.split_once('=').map_or(word, |(_, value)| value);
    if path.split(['/', '\\']).any(|segment| segment == "..") {
        return true;
    }
    let path = path.replace('\\', "/");
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let rooted = path.starts_with('/') || path.starts_with('~');
    if !drive && !rooted {
        return false;
    }
    if path == "/dev/null" {
        return false;
    }
    let Some(root) =
        std::path::absolute(workspace).ok().and_then(|p| p.to_str().map(|s| s.replace('\\', "/").trim_end_matches('/').to_lowercase()))
    else {
        return true;
    };
    let seen = path.to_lowercase();
    // Git Bash's `/c/Users/…` is `c:/Users/…`
    let letter = match seen.as_bytes() {
        [b'/', d, b'/', ..] | [b'/', d] if d.is_ascii_alphabetic() => Some(char::from(*d)),
        _ => None,
    };
    let seen = match letter {
        Some(d) => format!("{d}:{}", &seen[2..]),
        None => seen,
    };
    !(seen == root || seen.starts_with(&format!("{root}/")))
}

/// Whether a word names an absolute path at or under `target`, the card's own cargo target.
fn in_target(word: &str, target: &Path) -> bool {
    let path = word.split_once('=').map_or(word, |(_, value)| value).replace('\\', "/");
    let bytes = path.as_bytes();
    let absolute = path.starts_with('/') || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':');
    absolute && path != "/dev/null" && !outside(word, target)
}

/// The simple commands of `command`, or `None` for a shape it does not read: a subshell, a
/// substitution, a block, a heredoc or input redirect, a background `&`, an open quote.
fn split(command: &str, shell: Shell) -> Option<Vec<Part>> {
    let mut parts = Vec::new();
    let mut part = Part::default();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    let end_word = |part: &mut Part, word: &mut String, in_word: &mut bool| {
        if *in_word {
            part.words.push(std::mem::take(word));
            *in_word = false;
        }
    };
    let end_part = |parts: &mut Vec<Part>, part: &mut Part| {
        let done = std::mem::take(part);
        if !done.words.is_empty() || done.writes {
            parts.push(done);
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        other => word.push(other),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '`' => return None,
                        '$' if chars.peek() == Some(&'(') => return None,
                        other => word.push(other),
                    }
                }
            }
            ' ' | '\t' | '\r' => end_word(&mut part, &mut word, &mut in_word),
            '\n' | ';' => {
                end_word(&mut part, &mut word, &mut in_word);
                end_part(&mut parts, &mut part);
            }
            '|' => {
                chars.next_if_eq(&'|');
                end_word(&mut part, &mut word, &mut in_word);
                end_part(&mut parts, &mut part);
            }
            '&' if chars.peek() == Some(&'&') => {
                chars.next();
                end_word(&mut part, &mut word, &mut in_word);
                end_part(&mut parts, &mut part);
            }
            '&' if shell == Shell::PowerShell && !in_word && part.words.is_empty() && !part.call => part.call = true,
            '>' => {
                // a descriptor before the `>` (`2>`, PowerShell's `*>`) belongs to the redirect
                if in_word && (word == "*" || word.chars().all(|d| d.is_ascii_digit())) {
                    word.clear();
                    in_word = false;
                }
                end_word(&mut part, &mut word, &mut in_word);
                chars.next_if_eq(&'>');
                if chars.next_if_eq(&'&').is_some() {
                    // a descriptor copied, `2>&1`: nothing is written
                    while chars.next_if(|d| d.is_ascii_digit() || *d == '-').is_some() {}
                    continue;
                }
                while chars.next_if(|s| *s == ' ' || *s == '\t').is_some() {}
                let mut target = String::new();
                while let Some(t) = chars.next_if(|t| !t.is_whitespace() && !matches!(t, ';' | '|' | '&' | '>' | '<')) {
                    if t != '"' && t != '\'' {
                        target.push(t);
                    }
                }
                if target.is_empty() {
                    return None;
                }
                part.writes |= !["/dev/null", "$null", "nul"].iter().any(|d| d.eq_ignore_ascii_case(&target));
            }
            '&' | '<' | '(' | ')' | '{' | '}' | '`' => return None,
            '$' if chars.peek() == Some(&'(') => return None,
            '#' if !in_word => return None,
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    end_word(&mut part, &mut word, &mut in_word);
    end_part(&mut parts, &mut part);
    Some(parts)
}

/// The block of a worker's prompt that says the shell rules, from the same table the hook
/// judges by: what runs, the shapes a command may take, and what to use instead of a refusal.
#[must_use]
pub fn block() -> String {
    block_for(on_path("rg"))
}

/// Whether `program` is an executable in a directory of the shell's PATH.
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|dir| [program.to_owned(), format!("{program}.exe")].iter().any(|name| dir.join(name).is_file()))
}

/// [`block`] for a shell that does (`rg`) or does not have ripgrep on its PATH: without it the
/// list leaves `rg` out and names what searches in its place.
#[must_use]
pub fn block_for(rg: bool) -> String {
    block_with(rg, false)
}

/// [`block`] for a project whose builds are `refused` or not, on this shell's PATH.
#[must_use]
pub fn block_when(builds_refused: bool) -> String {
    block_with(on_path("rg"), builds_refused)
}

/// [`block_for`] for a project whose builds are refused: `cargo` is not listed as running,
/// one line says `cargo fmt` and `rustfmt` are the only cargo commands that do.
#[must_use]
pub fn block_with(rg: bool, builds_refused: bool) -> String {
    let names = |shell: Shell| {
        let mut seen: Vec<&str> = Vec::new();
        for rule in shell.allowed() {
            let name = rule.trim_end_matches('*').trim_end();
            if builds_refused && matches!(name, "cargo" | "rustfmt") {
                continue;
            }
            if (rg || name != "rg") && !seen.contains(&name) {
                seen.push(name);
            }
        }
        seen.join(", ")
    };
    let mut lines = vec![
        "Allowed shell shapes (the hook judges a command part by part):".to_owned(),
        format!("- Bash runs: {}", names(Shell::Bash)),
        format!("- PowerShell runs: {}", names(Shell::PowerShell)),
        "- one command per call: no `;`, `&&` or pipe between commands; a call passes when its command does, with a `2>&1`, `> /dev/null` or `2>$null`, a `VAR=value` prefix in Bash, and in PowerShell a `& \"path\\tool.exe\"` call or a `$name = 'value'` assignment".to_owned(),
        "- a command chained with `;` or `&&`, or one that tests `$?`, is refused whole: send each command in its own call".to_owned(),
        "- paths are the workspace's, relative or absolute; `..` and paths outside it are the harness's to judge".to_owned(),
        "- hash a file with `sha256sum <file>` in Bash, on a path inside the workspace".to_owned(),
        "- a `$( )` or backtick substitution, a `( )` or `{ }` block, a heredoc and a `<` redirect are not judged by the hook, and the allowlist usually refuses them".to_owned(),
        "- refused, with what to use instead:".to_owned(),
    ];
    if !builds_refused {
        lines.insert(3, "- a `cargo build`, `cargo test` or `cargo clippy` is started with `run_in_background` on its first call and awaited by its task notification: a foreground call dies at the Bash timeout while the build waits for a slot".to_owned());
    }
    if builds_refused {
        lines.insert(3, "- this project's builds are refused: `cargo fmt`/`rustfmt` are the only cargo commands that run, and the check does the building".to_owned());
        lines.insert(4, "- `rc check <id> --mid` runs your card's own check once on a build slot and prints its first failure and tail, at most twice a run: once the change is written, start it in the background and wait on its notification".to_owned());
    }
    if !rg {
        lines.insert(3, "- `rg` is not on this shell's PATH: search with the Grep tool or `grep`".to_owned());
    }
    for (names, form) in REFUSED {
        lines.push(format!("  - {}: {form}", names.join(", ")));
    }
    lines.push(format!(
        "  - git {} (but `git rm` of tracked paths, `git rm --cached` and `git mv` inside the workspace run) and git -C (but `git -C var/worktrees/<id> status|log|diff|show` runs): {GIT_FORM}",
        GIT_WRITES.join("|")
    ));
    lines.push(format!("  - a `>` redirect to a file: {REDIRECT_FORM}"));
    lines.push(format!("  - `python -` or a bare `python`: {STDIN_FORM}"));
    lines.join("\n")
}

#[cfg(test)]
mod tests;
