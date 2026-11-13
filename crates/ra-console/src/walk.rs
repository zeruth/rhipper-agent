//! The start of a walk: which build it runs with, and the line the feed says about it.

use std::path::Path;

use crate::channel::{Build, Standing};

/// The build a walk of `project` runs with and, while a candidate stands, the feed's line
/// saying so; said once a walk, at its start.
#[must_use]
pub fn begin(standing: &Standing, project: &Path, home: Option<&Path>) -> (Build, Option<String>) {
    let build = standing.build_for(project, home);
    let line = standing.candidate.as_ref().map(|candidate| match build {
        Build::Candidate => format!("walking with the candidate build {}", candidate.display()),
        Build::Stable => format!("walking with the stable build {}; a candidate stands in the agent's project", standing.stable.display()),
    });
    (build, line)
}
