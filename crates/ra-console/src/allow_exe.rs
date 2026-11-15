//! The test binaries a project lets its worker run: `[worker] binaries = ["testdata/rhtest/*.exe"]`.

/// The program of `command` as a path relative to the worktree, when it names one: quotes and
/// a leading `./` dropped, `\` read as `/`. `None` for a path that leaves the worktree (absolute,
/// a drive, or a `..` part), whatever a glob would say of it.
#[must_use]
pub fn program_inside(command: &str) -> Option<String> {
    let first = command.split_whitespace().next().unwrap_or("").trim_matches(['"', '\'']).replace('\\', "/");
    let program = first.strip_prefix("./").unwrap_or(&first);
    let outside = program.is_empty() || program.starts_with('/') || program.contains(':') || program.split('/').any(|part| part == "..");
    (!outside).then(|| program.to_owned())
}

/// What a refused script inside the worktree needs to run: the allowlist file, the nearest glob
/// of `binaries` (the one whose directory it shares most), and the exact line that would allow it.
/// `None` when the command names no path inside the worktree or a glob already allows it.
#[must_use]
pub fn allowlist_entry(globs: &[String], command: &str) -> Option<String> {
    let program = program_inside(command).filter(|p| p.contains('/'))?;
    if globs.iter().any(|glob| crate::router::glob_matches(glob, &program)) {
        return None;
    }
    let (dir, file) = program.rsplit_once('/')?;
    let entry = match file.rsplit_once('.') {
        Some((_, ext)) => format!("{dir}/*.{ext}"),
        None => program.clone(),
    };
    let shared = |glob: &str| glob.replace('\\', "/").split('/').zip(program.split('/')).take_while(|(a, b)| a == b).count();
    let nearest = globs.iter().max_by_key(|g| shared(g)).filter(|g| shared(g) > 0);
    let near = nearest.map_or_else(String::new, |g| format!(" The nearest allowed entry is `{g}`."));
    let mut all: Vec<String> = globs.to_vec();
    all.push(entry);
    let list = all.iter().map(|g| format!("\"{g}\"")).collect::<Vec<_>>().join(", ");
    Some(format!(
        "`{program}` is not on the allowlist of {}.{near} To allow it, set in its `[worker]` table: `binaries = [{list}]`",
        crate::router::ROUTES_FILE
    ))
}

/// The reason of a hook's deny of `command` with the allowlist entry a refused script needs, when
/// it needs one; that entry is kept as a note of the card whose worktree `cwd` is, once, so the
/// run's note carries the line for the person after the refusal has scrolled past.
#[must_use]
pub fn deny_reason(cwd: &std::path::Path, globs: &[String], command: &str, reason: String) -> String {
    let Some(entry) = allowlist_entry(globs, command) else { return reason };
    if let Err(e) = note_entry(cwd, &entry) {
        tracing::warn!("the allowlist entry was not noted: {e:#}");
    }
    format!("{reason} {entry}")
}

/// Keeps `entry` in the project's `var/retro.log` under the card of the worktree `cwd` (no card
/// outside one), unless the card has it already: a script refused ten times is one line to add.
///
/// # Errors
/// When the file cannot be written.
pub fn note_entry(cwd: &std::path::Path, entry: &str) -> anyhow::Result<()> {
    let root = crate::hub::project_root_of(cwd);
    let card = (root != std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf()))
        .then(|| cwd.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse::<i64>().ok()))
        .flatten();
    let node = card.map_or_else(|| "-".to_owned(), |c| c.to_string());
    let kept = std::fs::read_to_string(root.join(crate::retro::RETRO_FILE)).unwrap_or_default();
    if kept.lines().any(|line| line.split('\t').nth(1) == Some(node.as_str()) && line.ends_with(entry)) {
        return Ok(());
    }
    let retro_note = crate::retro::Note { when: 0, node: card, machine: false, pair: false, text: entry.to_owned(), against: None };
    crate::retro::record(&root, card, &[retro_note])
}

/// Whether the program of `command` is an executable inside the worktree that one of the `globs`
/// names; any other binary stays refused.
#[must_use]
pub fn runs_listed_binary(globs: &[String], command: &str) -> bool {
    program_inside(command).is_some_and(|program| globs.iter().any(|glob| crate::router::glob_matches(glob, &program)))
}
