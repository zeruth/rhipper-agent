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

/// Whether the program of `command` is an executable inside the worktree that one of the `globs`
/// names; any other binary stays refused.
#[must_use]
pub fn runs_listed_binary(globs: &[String], command: &str) -> bool {
    program_inside(command).is_some_and(|program| globs.iter().any(|glob| crate::router::glob_matches(glob, &program)))
}
