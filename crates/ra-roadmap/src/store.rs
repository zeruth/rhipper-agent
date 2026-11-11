//! The store over a `rhipper.md` file. Every mutation rewrites the file, so a git diff shows each
//! change and an editor outside the console stays authoritative: the store reads the file again
//! when it changed on disk.
//!
//! The ids come from the file, not from a counter: the highest id plus one, so a hand-added node
//! keeps its number and a deleted node's id is not reused for a while.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::markdown::{self, render};
use crate::model::{Assignment, Node, NodeKind, TaskState};
use crate::tree;

/// The store over one `rhipper.md`.
#[derive(Clone, Debug)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// Opens the store at `path`, creating the file with one project when it is new.
    pub async fn open(path: &Path, title: &str) -> anyhow::Result<Self> {
        if !path.exists() {
            let root = Node {
                id: 1,
                parent: None,
                kind: NodeKind::Project,
                title: title.to_owned(),
                objective: None,
                acceptance: None,
                files: Vec::new(),
                session: None,
                depends_on: Vec::new(),
                state: TaskState::Defined,
                assignee: None,
                complexity: 3,
                notes: None,
                assessed: None,
            };
            std::fs::write(path, render(std::slice::from_ref(&root))).with_context(|| format!("cannot write {}", path.display()))?;
        }
        Ok(Self { path: path.to_owned() })
    }

    /// The file the store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// When the file last changed, for a reader that polls it; `None` when it cannot be read.
    pub fn modified(&self) -> Option<std::time::SystemTime> {
        std::fs::metadata(&self.path).ok()?.modified().ok()
    }

    /// Reads every node, in tree order.
    pub async fn all(&self) -> anyhow::Result<Vec<Node>> {
        let text = std::fs::read_to_string(&self.path).with_context(|| format!("cannot read {}", self.path.display()))?;
        markdown::parse(&text)
    }

    /// Reads one node.
    pub async fn read(&self, id: i64) -> anyhow::Result<Node> {
        self.all().await?.into_iter().find(|n| n.id == id).with_context(|| format!("no node {id}"))
    }

    /// Inserts a node under `parent` (`None` for a project) and returns its id.
    pub async fn insert(
        &self,
        parent: Option<i64>,
        kind: NodeKind,
        title: &str,
        objective: Option<&str>,
        acceptance: Option<&str>,
        complexity: u8,
    ) -> anyhow::Result<i64> {
        let mut nodes = self.all().await?;
        let parent = parent.filter(|_| kind != NodeKind::Project);
        // the new node goes after the last descendant of its parent, so the file keeps its tree
        // order and the node reads back under the right parent; a project goes last
        let position = match parent {
            Some(parent) => {
                anyhow::ensure!(nodes.iter().any(|n| n.id == parent), "no node {parent}");
                let last = tree::subtree(&nodes, parent).last().map(|n| n.id);
                nodes.iter().position(|n| Some(n.id) == last).map_or(nodes.len(), |i| i + 1)
            }
            None => {
                anyhow::ensure!(kind == NodeKind::Project, "a node below a project needs a parent");
                nodes.len()
            }
        };
        let id = nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        let node = Node {
            id,
            parent,
            kind,
            title: title.to_owned(),
            objective: objective.map(str::to_owned),
            acceptance: acceptance.map(str::to_owned),
            files: Vec::new(),
            session: None,
            depends_on: Vec::new(),
            state: TaskState::Defined,
            assignee: None,
            complexity,
            notes: None,
            assessed: None,
        };
        nodes.insert(position, node);
        self.write(nodes).await?;
        Ok(id)
    }

    /// Adds a dependency: `id` waits for `waits_for`.
    pub async fn depend_on(&self, id: i64, waits_for: i64) -> anyhow::Result<()> {
        let mut nodes = self.all().await?;
        let node = node_mut(&mut nodes, id)?;
        if !node.depends_on.contains(&waits_for) {
            node.depends_on.push(waits_for);
            self.write(nodes).await?;
        }
        Ok(())
    }

    /// Sets the state of a node.
    pub async fn set_state(&self, id: i64, state: TaskState) -> anyhow::Result<()> {
        let mut nodes = self.all().await?;
        node_mut(&mut nodes, id)?.state = state;
        self.write(nodes).await
    }

    /// Assigns a node to a worker, or clears the assignment with `None`.
    pub async fn assign(&self, id: i64, assignment: Option<&Assignment>) -> anyhow::Result<()> {
        let mut nodes = self.all().await?;
        node_mut(&mut nodes, id)?.assignee = assignment.cloned();
        self.write(nodes).await
    }

    /// Records the notes of the last execution on a node.
    pub async fn set_notes(&self, id: i64, notes: &str) -> anyhow::Result<()> {
        let mut nodes = self.all().await?;
        node_mut(&mut nodes, id)?.notes = Some(notes.to_owned());
        self.write(nodes).await
    }

    /// Replaces the files a node names, the workspace-relative paths its worker reads first; an
    /// empty list clears them. A path holds no whitespace, since the file separates paths by spaces.
    pub async fn set_files(&self, id: i64, files: &[String]) -> anyhow::Result<()> {
        anyhow::ensure!(files.iter().all(|f| !f.is_empty() && !f.contains(char::is_whitespace)), "a path holds no whitespace");
        let mut nodes = self.all().await?;
        node_mut(&mut nodes, id)?.files = files.to_vec();
        self.write(nodes).await
    }

    /// Keeps the harness session of a node's last worker run, so the next run resumes it; `None`
    /// forgets it. A session id is one word.
    pub async fn set_session(&self, id: i64, session: Option<&str>) -> anyhow::Result<()> {
        let session = session.map(str::trim).filter(|s| !s.is_empty());
        anyhow::ensure!(session.is_none_or(|s| !s.contains(char::is_whitespace)), "a session id holds no whitespace");
        let mut nodes = self.all().await?;
        node_mut(&mut nodes, id)?.session = session.map(str::to_owned);
        self.write(nodes).await
    }

    /// Replaces the objective and the acceptance of a node, as a reassessment wrote them. A field
    /// of `None` keeps its text.
    pub async fn set_details(&self, id: i64, objective: Option<&str>, acceptance: Option<&str>) -> anyhow::Result<()> {
        let mut nodes = self.all().await?;
        let node = node_mut(&mut nodes, id)?;
        if let Some(objective) = objective {
            node.objective = Some(objective.to_owned());
        }
        if let Some(acceptance) = acceptance {
            node.acceptance = Some(acceptance.to_owned());
        }
        self.write(nodes).await
    }

    /// Records what the last reassessment of a node saw: a fingerprint, or the word `final`;
    /// `None` clears the mark, so the next reassessment calls the model again.
    pub async fn set_assessed(&self, id: i64, assessed: Option<&str>) -> anyhow::Result<()> {
        let mut nodes = self.all().await?;
        node_mut(&mut nodes, id)?.assessed = assessed.map(str::to_owned);
        self.write(nodes).await
    }

    /// Writes the nodes back as the `rhipper.md` text. The file is small, so a blocking write stays
    /// fine.
    async fn write(&self, nodes: Vec<Node>) -> anyhow::Result<()> {
        let text = render(&nodes);
        std::fs::write(&self.path, text).with_context(|| format!("cannot write {}", self.path.display()))
    }
}

/// The node with `id`, mutable.
fn node_mut<'a>(nodes: &'a mut [Node], id: i64) -> anyhow::Result<&'a mut Node> {
    nodes.iter_mut().find(|n| n.id == id).with_context(|| format!("no node {id}"))
}

#[cfg(test)]
mod tests;
