//! Which build of the agent a project walks with. A new build stands as a candidate until it has
//! earned its install; while it stands, the agent's own project walks with it and every other
//! project keeps the stable build, so a build that fails costs the agent a card and not another
//! project a day.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A build of the agent a walk can run with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    Stable,
    Candidate,
}

/// The builds on the machine: the stable one, and the candidate while one stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    pub stable: PathBuf,
    pub candidate: Option<PathBuf>,
}

/// The file of the hub directory that remembers the standing builds.
const FILE: &str = "builds.txt";

impl Standing {
    /// The standing builds the hub directory `dir` remembers, when it remembers a stable one.
    #[must_use]
    pub fn load(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join(FILE)).ok()?;
        let value = |key: &str| {
            text.lines()
                .find_map(|l| l.trim().strip_prefix(key).and_then(|v| v.strip_prefix('=')))
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Some(Self { stable: value("stable")?, candidate: value("candidate") })
    }

    /// Remembers the standing builds in the hub directory `dir`.
    ///
    /// # Errors
    /// When the directory or the file cannot be written.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut text = format!("stable={}\n", self.stable.display());
        if let Some(candidate) = &self.candidate {
            let _ = writeln!(text, "candidate={}", candidate.display());
        }
        std::fs::write(dir.join(FILE), text)
    }

    /// The build `project` walks with: the candidate when one stands and `project` is the
    /// agent's own (`home`, from `plan::home::agent_home`), the stable build otherwise.
    #[must_use]
    pub fn build_for(&self, project: &Path, home: Option<&Path>) -> Build {
        if self.candidate.is_some() && home == Some(project) { Build::Candidate } else { Build::Stable }
    }

    /// The binary of a build.
    #[must_use]
    pub fn binary(&self, build: Build) -> &Path {
        match (build, &self.candidate) {
            (Build::Candidate, Some(candidate)) => candidate,
            _ => &self.stable,
        }
    }

    /// Each project's build at one stage boundary: the same standing, so the agent's project
    /// and the others part ways here and nowhere else.
    #[must_use]
    pub fn at_boundary(&self, projects: &[PathBuf], home: Option<&Path>) -> Vec<(PathBuf, Build)> {
        projects.iter().map(|p| (p.clone(), self.build_for(p, home))).collect()
    }
}

#[cfg(test)]
mod tests;
