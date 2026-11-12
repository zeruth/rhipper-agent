//! The store over a `rhipper.md` file. Every mutation rewrites the file, so a git diff shows each
//! change and an editor outside the console stays authoritative: the store reads the file again
//! when it changed on disk.
//!
//! The ids come from the file, not from a counter: the highest id plus one, so a hand-added node
//! keeps its number and a deleted node's id is not reused for a while.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::history::{self, Change, Source};
use crate::markdown::{self, render};
use crate::model::{Assignment, Effort, Estimate, Gambit, GambitState, Node, NodeKind, Roadmap, TaskState, WEIGHT_RANGE};
use crate::tree;

/// The store over one `rhipper.md`.
#[derive(Clone, Debug)]
pub struct Store {
    path: PathBuf,
    /// Who the history names for a state change made through this store; the agent loop names
    /// itself per change instead.
    by: Source,
}

/// Everything a new card carries besides its place and its title, so one insert writes a card
/// that is complete: the texts, the files its worker reads first, the cards it waits on and the
/// model that runs it.
#[derive(Clone, Debug, Default)]
pub struct Card<'a> {
    pub objective: Option<&'a str>,
    pub acceptance: Option<&'a str>,
    /// Complexity 1..=5; the default of the struct is 3.
    pub complexity: u8,
    pub files: &'a [String],
    pub waits_on: &'a [i64],
    /// The model of the card's implementer; `None` leaves the card to the default route.
    pub model: Option<&'a str>,
    /// The evidence line a planned card cites, `source:line`.
    pub evidence: Option<&'a str>,
    /// What a planned card claims to save.
    pub saving: Option<&'a str>,
}

impl Card<'_> {
    /// A card of complexity 3 with nothing else set.
    #[must_use]
    pub fn new() -> Self {
        Self { complexity: 3, ..Self::default() }
    }
}

/// What one edit rewrites on a card: a field of `None` keeps what the card has, and nothing
/// the edit does not name moves (the notes, the state, the session unless cleared).
#[derive(Clone, Debug, Default)]
pub struct Edit<'a> {
    pub title: Option<&'a str>,
    pub objective: Option<&'a str>,
    pub acceptance: Option<&'a str>,
    pub complexity: Option<u8>,
    pub files: Option<&'a [String]>,
    /// Forgets the harness session, so the next run starts fresh instead of resuming.
    pub clear_session: bool,
}

impl Edit<'_> {
    /// Whether the edit names anything at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.objective.is_none()
            && self.acceptance.is_none()
            && self.complexity.is_none()
            && self.files.is_none()
            && !self.clear_session
    }
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
                session_at: None,
                session_of: None,
                depends_on: Vec::new(),
                state: TaskState::Defined,
                assignee: None,
                complexity: 3,
                effort: Effort::default(),
                estimate: None,
                notes: None,
                assessed: None,
                priority: None,
                absolute: false,
                evidence: None,
                saving: None,
            };
            std::fs::write(path, render(std::slice::from_ref(&root))).with_context(|| format!("cannot write {}", path.display()))?;
        }
        let store = Self { path: path.to_owned(), by: Source::default() };
        // a cancelled card does not exist: a file written before that rule loses them on open
        store.remove_cancelled().await?;
        Ok(store)
    }

    /// The same store with the source its state changes are recorded under.
    #[must_use]
    pub fn with_source(self, by: Source) -> Self {
        Self { by, ..self }
    }

    /// The file the store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The history file beside the roadmap, `var/history.log`.
    pub fn history_path(&self) -> PathBuf {
        history::path_beside(&self.path)
    }

    /// The state changes recorded so far, oldest first; of one card with `id`.
    pub fn history(&self, id: Option<i64>) -> anyhow::Result<Vec<Change>> {
        history::read(&self.history_path(), id)
    }

    /// When the file last changed, for a reader that polls it; `None` when it cannot be read.
    pub fn modified(&self) -> Option<std::time::SystemTime> {
        std::fs::metadata(&self.path).ok()?.modified().ok()
    }

    /// Reads the whole file: the gambits and the nodes in tree order.
    pub async fn load(&self) -> anyhow::Result<Roadmap> {
        let text = std::fs::read_to_string(&self.path).with_context(|| format!("cannot read {}", self.path.display()))?;
        markdown::parse_file(&text)
    }

    /// Reads every node, in tree order.
    pub async fn all(&self) -> anyhow::Result<Vec<Node>> {
        Ok(self.load().await?.nodes)
    }

    /// Reads the gambits, in file order.
    pub async fn gambits(&self) -> anyhow::Result<Vec<Gambit>> {
        Ok(self.load().await?.gambits)
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
        self.insert_card(parent, kind, title, &Card { objective, acceptance, complexity, ..Card::new() }).await
    }

    /// Inserts a node with everything a [`Card`] carries, in one write. The cards it waits on
    /// must be on the roadmap, the paths hold no whitespace and the complexity is 1 to 5.
    ///
    /// # Errors
    /// When the parent or a waited-on node is not there, a field is malformed, or the file cannot
    /// be written.
    pub async fn insert_card(&self, parent: Option<i64>, kind: NodeKind, title: &str, card: &Card<'_>) -> anyhow::Result<i64> {
        anyhow::ensure!((1..=5).contains(&card.complexity), "complexity is 1 to 5");
        anyhow::ensure!(card.files.iter().all(|f| !f.is_empty() && !f.contains(char::is_whitespace)), "a path holds no whitespace");
        anyhow::ensure!(card.model.is_none_or(|m| !m.is_empty() && !m.contains(char::is_whitespace)), "a model id is one word");
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        for waited in card.waits_on {
            anyhow::ensure!(nodes.iter().any(|n| n.id == *waited), "no node {waited}");
        }
        let parent = parent.filter(|_| kind != NodeKind::Project);
        // the new node goes after the last descendant of its parent, so the file keeps its tree
        // order and the node reads back under the right parent; a project goes last
        let position = match parent {
            Some(parent) => {
                anyhow::ensure!(nodes.iter().any(|n| n.id == parent), "no node {parent}");
                let last = tree::subtree(nodes, parent).last().map(|n| n.id);
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
            objective: card.objective.map(str::to_owned),
            acceptance: card.acceptance.map(str::to_owned),
            files: card.files.to_vec(),
            session: None,
            session_at: None,
            session_of: None,
            depends_on: dedup(card.waits_on),
            state: TaskState::Defined,
            assignee: card.model.map(|model| Assignment { model: model.to_owned(), role: "implementer".to_owned() }),
            complexity: card.complexity,
            effort: Effort::default(),
            estimate: None,
            notes: None,
            assessed: None,
            priority: None,
            absolute: false,
            evidence: card.evidence.map(str::to_owned),
            saving: card.saving.map(str::to_owned),
        };
        nodes.insert(position, node);
        self.write(roadmap).await?;
        Ok(id)
    }

    /// Renames a node; the same as [`Store::set_title`], kept for the commands that say `rename`.
    pub async fn rename(&self, id: i64, title: &str) -> anyhow::Result<()> {
        self.set_title(id, title).await
    }

    /// Sets the title of a node, trimmed; a title is one line, since the node line holds it.
    ///
    /// # Errors
    /// When the title is empty or holds a line break, the node is not there or the file cannot
    /// be written.
    pub async fn set_title(&self, id: i64, title: &str) -> anyhow::Result<()> {
        let title = title.trim();
        anyhow::ensure!(!title.is_empty(), "a title is not empty");
        anyhow::ensure!(!title.contains(['\n', '\r']), "a title is one line");
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        node_mut(nodes, id)?.title = title.to_owned();
        self.write(roadmap).await
    }

    /// Sets the complexity of a node, 1 to 5.
    ///
    /// # Errors
    /// When the complexity is out of range, the node is not there or the file cannot be written.
    pub async fn set_complexity(&self, id: i64, complexity: u8) -> anyhow::Result<()> {
        ensure_weight(Some(complexity), "complexity")?;
        let mut roadmap = self.load().await?;
        node_mut(&mut roadmap.nodes, id)?.complexity = complexity;
        self.write(roadmap).await
    }

    /// Adds the items of an outline under `parent`: a milestone under the parent, a section
    /// under the last milestone, a task under the last section or milestone, a subtask under the
    /// last task. When the parent is the project and the outline names a title, the project
    /// takes it. Returns the ids added, in order.
    pub async fn import(&self, parent: i64, outline: &crate::outline::Outline) -> anyhow::Result<Vec<i64>> {
        use crate::outline::Item;
        let root = self.read(parent).await?;
        if let Some(title) = &outline.title
            && root.kind == NodeKind::Project
        {
            self.rename(parent, title).await?;
        }
        let (mut milestone, mut section, mut task) = (None, None, None);
        let mut ids = Vec::new();
        for Item { kind, title, complexity, done, objective, acceptance, files } in &outline.items {
            let under = match kind {
                NodeKind::Project | NodeKind::Milestone => parent,
                NodeKind::Section => milestone.unwrap_or(parent),
                NodeKind::Task => section.or(milestone).unwrap_or(parent),
                NodeKind::Subtask => task.or(section).or(milestone).unwrap_or(parent),
            };
            let kind = if *kind == NodeKind::Project { NodeKind::Milestone } else { *kind };
            let id = self.insert(Some(under), kind, title, objective.as_deref(), acceptance.as_deref(), *complexity).await?;
            if !files.is_empty() {
                self.set_files(id, files).await?;
            }
            if *done {
                self.set_state(id, TaskState::Accepted).await?;
            }
            match kind {
                NodeKind::Milestone => (milestone, section, task) = (Some(id), None, None),
                NodeKind::Section => (section, task) = (Some(id), None),
                NodeKind::Task => task = Some(id),
                _ => {}
            }
            ids.push(id);
        }
        Ok(ids)
    }

    /// Adds a dependency: `id` waits for `waits_for`.
    pub async fn depend_on(&self, id: i64, waits_for: i64) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        let node = node_mut(nodes, id)?;
        if !node.depends_on.contains(&waits_for) {
            node.depends_on.push(waits_for);
            self.write(roadmap).await?;
        }
        Ok(())
    }

    /// Sets the state of a node, recorded in the history under the store's source. A card set
    /// to cancelled is removed with its subtree: a cancelled card does not stay on the roadmap,
    /// where it would hold its phase open.
    pub async fn set_state(&self, id: i64, state: TaskState) -> anyhow::Result<()> {
        self.set_state_by(id, state, self.by).await
    }

    /// [`set_state`](Self::set_state) recorded under `by`, for the agent loop, whose changes
    /// are the run's and not the surface's that started it. A state set again is no change
    /// and leaves no line.
    pub async fn set_state_by(&self, id: i64, state: TaskState, by: Source) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        let from = node_mut(&mut roadmap.nodes, id)?.state;
        if state == TaskState::Cancelled {
            self.remove(id, true).await?;
        } else {
            node_mut(&mut roadmap.nodes, id)?.state = state;
            self.write(roadmap).await?;
        }
        if from != state {
            let change = Change { at: unix_now(), id, from, to: state, by };
            history::append(&self.history_path(), &change)?;
        }
        Ok(())
    }

    /// Removes a node, and with `tree` its subtree; a node with children is refused without
    /// `tree`, and the project is never removed. Every `waits-on` reference to a removed id goes
    /// with it. Returns the ids removed, the node first.
    pub async fn remove(&self, id: i64, tree: bool) -> anyhow::Result<Vec<i64>> {
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        let node = nodes.iter().find(|n| n.id == id).with_context(|| format!("no node {id}"))?;
        anyhow::ensure!(node.kind != NodeKind::Project, "#{id} is the project; it stays");
        let children: Vec<String> = nodes.iter().filter(|n| n.parent == Some(id)).map(|n| format!("#{}", n.id)).collect();
        anyhow::ensure!(
            tree || children.is_empty(),
            "#{id} has {} card(s) under it ({}); remove them first, or the whole tree",
            children.len(),
            children.join(" ")
        );
        let removed: Vec<i64> = tree::subtree(nodes, id).iter().map(|n| n.id).collect();
        nodes.retain(|n| !removed.contains(&n.id));
        for n in nodes.iter_mut() {
            n.depends_on.retain(|d| !removed.contains(d));
        }
        self.write(roadmap).await?;
        Ok(removed)
    }

    /// Moves a node with its subtree under `under`, after the sibling `after` or last among the
    /// children when none is named; the file keeps its tree order. The project does not move, and
    /// a node is never put under itself or one of its descendants.
    ///
    /// # Errors
    /// When a node is not there, the node is the project, `under` sits in the moved subtree,
    /// `after` is not a child of `under`, or the file cannot be written.
    pub async fn move_node(&self, id: i64, under: i64, after: Option<i64>) -> anyhow::Result<()> {
        self.relocate(id, under, after.map_or(Slot::Last, Slot::After)).await
    }

    /// Moves a node with its subtree under `under` as its first child: where a drop above the
    /// first row of a bubble lands, which no `after` can name.
    ///
    /// # Errors
    /// As [`Store::move_node`].
    pub async fn move_first(&self, id: i64, under: i64) -> anyhow::Result<()> {
        self.relocate(id, under, Slot::First).await
    }

    async fn relocate(&self, id: i64, under: i64, slot: Slot) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        let node = nodes.iter().find(|n| n.id == id).with_context(|| format!("no node {id}"))?;
        anyhow::ensure!(node.kind != NodeKind::Project, "#{id} is the project; it stays");
        anyhow::ensure!(nodes.iter().any(|n| n.id == under), "no node {under}");
        anyhow::ensure!(!tree::is_within(nodes, under, id), "#{under} is #{id} or under it; a card does not move under itself");
        if let Slot::After(after) = slot {
            let sibling = nodes.iter().find(|n| n.id == after).with_context(|| format!("no node {after}"))?;
            anyhow::ensure!(sibling.parent == Some(under), "#{after} is not under #{under}");
            anyhow::ensure!(after != id, "#{id} cannot follow itself");
        }
        // the subtree comes out whole, then goes back right behind the new parent's line when it
        // goes first, else after the last descendant of the sibling it follows, or of the new
        // parent when it goes last
        let moved: Vec<i64> = tree::subtree(nodes, id).iter().map(|n| n.id).collect();
        let mut lifted: Vec<Node> = Vec::with_capacity(moved.len());
        let mut rest = Vec::with_capacity(nodes.len());
        for n in nodes.drain(..) {
            if moved.contains(&n.id) { lifted.push(n) } else { rest.push(n) }
        }
        *nodes = rest;
        lifted[0].parent = Some(under);
        let behind = match slot {
            Slot::First => Some(under),
            Slot::Last => tree::subtree(nodes, under).last().map(|n| n.id),
            Slot::After(after) => tree::subtree(nodes, after).last().map(|n| n.id),
        };
        let position = nodes.iter().position(|n| Some(n.id) == behind).map_or(nodes.len(), |i| i + 1);
        nodes.splice(position..position, lifted);
        self.write(roadmap).await
    }

    /// Removes every cancelled card the file holds, with its subtree; how many went. An older
    /// file may still carry them.
    pub async fn remove_cancelled(&self) -> anyhow::Result<usize> {
        let cancelled: Vec<i64> = self.all().await?.iter().filter(|n| n.state == TaskState::Cancelled).map(|n| n.id).collect();
        let mut gone = 0;
        for id in cancelled {
            // a card under a cancelled card went with it already
            if let Ok(removed) = self.remove(id, true).await {
                gone += removed.len();
            }
        }
        Ok(gone)
    }

    /// Assigns a node to a worker, or clears the assignment with `None`.
    pub async fn assign(&self, id: i64, assignment: Option<&Assignment>) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        node_mut(nodes, id)?.assignee = assignment.cloned();
        self.write(roadmap).await
    }

    /// Sets the effort a run of a node asks of its model.
    ///
    /// # Errors
    /// When the node is not there or the file cannot be written.
    pub async fn set_effort(&self, id: i64, effort: Effort) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        node_mut(nodes, id)?.effort = effort;
        self.write(roadmap).await
    }

    /// Sets a node's own estimate of what a run spends; `None` goes back to the complexity's.
    ///
    /// # Errors
    /// When the node is not there or the file cannot be written.
    pub async fn set_estimate(&self, id: i64, estimate: Option<Estimate>) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        node_mut(nodes, id)?.estimate = estimate;
        self.write(roadmap).await
    }

    /// Records the notes of the last execution on a node.
    ///
    /// # Errors
    /// When the node is not there or the file cannot be written.
    pub async fn set_notes(&self, id: i64, notes: &str) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        node_mut(nodes, id)?.notes = Some(notes.to_owned());
        self.write(roadmap).await
    }

    /// Replaces the files a node names, the workspace-relative paths its worker reads first; an
    /// empty list clears them. A path holds no whitespace, since the file separates paths by spaces.
    pub async fn set_files(&self, id: i64, files: &[String]) -> anyhow::Result<()> {
        anyhow::ensure!(files.iter().all(|f| !f.is_empty() && !f.contains(char::is_whitespace)), "a path holds no whitespace");
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        node_mut(nodes, id)?.files = files.to_vec();
        self.write(roadmap).await
    }

    /// Keeps the harness session of a node's last worker run, with the fingerprint of the card it
    /// worked on and the time, so the next run resumes it while the card is the same and the
    /// session young; `None` forgets it. A session id and a fingerprint are one word each.
    ///
    /// # Errors
    /// When an id or a fingerprint holds whitespace, the node is not there, or the file cannot
    /// be written.
    pub async fn set_session(&self, id: i64, session: Option<(&str, &str)>, now: u64) -> anyhow::Result<()> {
        let session = session.map(|(s, of)| (s.trim(), of.trim())).filter(|(s, _)| !s.is_empty());
        anyhow::ensure!(
            session.is_none_or(|(s, of)| !s.contains(char::is_whitespace) && !of.is_empty() && !of.contains(char::is_whitespace)),
            "a session id and its fingerprint hold no whitespace"
        );
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        let node = node_mut(nodes, id)?;
        node.session = session.map(|(s, _)| s.to_owned());
        node.session_of = session.map(|(_, of)| of.to_owned());
        node.session_at = session.map(|_| now);
        self.write(roadmap).await
    }

    /// Replaces the objective and the acceptance of a node, as a reassessment wrote them. A field
    /// of `None` keeps its text.
    pub async fn set_details(&self, id: i64, objective: Option<&str>, acceptance: Option<&str>) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        let node = node_mut(nodes, id)?;
        if let Some(objective) = objective {
            node.objective = Some(objective.to_owned());
        }
        if let Some(acceptance) = acceptance {
            node.acceptance = Some(acceptance.to_owned());
        }
        self.write(roadmap).await
    }

    /// Rewrites the fields an edit names and nothing else, in one write, under the same rules
    /// as the setters of each field.
    ///
    /// # Errors
    /// When the title is empty or holds a line break, the complexity is out of range, a path
    /// holds whitespace, the node is not there or the file cannot be written.
    pub async fn edit(&self, id: i64, edit: &Edit<'_>) -> anyhow::Result<()> {
        let title = edit.title.map(str::trim);
        anyhow::ensure!(title.is_none_or(|t| !t.is_empty()), "a title is not empty");
        anyhow::ensure!(title.is_none_or(|t| !t.contains(['\n', '\r'])), "a title is one line");
        ensure_weight(edit.complexity, "complexity")?;
        anyhow::ensure!(
            edit.files.is_none_or(|files| files.iter().all(|f| !f.is_empty() && !f.contains(char::is_whitespace))),
            "a path holds no whitespace"
        );
        let mut roadmap = self.load().await?;
        let node = node_mut(&mut roadmap.nodes, id)?;
        if let Some(title) = title {
            title.clone_into(&mut node.title);
        }
        if let Some(objective) = edit.objective {
            node.objective = Some(objective.to_owned());
        }
        if let Some(acceptance) = edit.acceptance {
            node.acceptance = Some(acceptance.to_owned());
        }
        if let Some(complexity) = edit.complexity {
            node.complexity = complexity;
        }
        if let Some(files) = edit.files {
            node.files = files.to_vec();
        }
        if edit.clear_session {
            node.session = None;
            node.session_of = None;
            node.session_at = None;
        }
        self.write(roadmap).await
    }

    /// Records what the last reassessment of a node saw: a fingerprint, or the word `final`;
    /// `None` clears the mark, so the next reassessment calls the model again.
    pub async fn set_assessed(&self, id: i64, assessed: Option<&str>) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        let nodes = &mut roadmap.nodes;
        node_mut(nodes, id)?.assessed = assessed.map(str::to_owned);
        self.write(roadmap).await
    }

    /// Sets the person's priority on a node, 1 to 5; `None` clears it.
    ///
    /// # Errors
    /// When the priority is out of range, the node is not there or the file cannot be written.
    pub async fn set_priority(&self, id: i64, priority: Option<u8>) -> anyhow::Result<()> {
        ensure_weight(priority, "priority")?;
        let mut roadmap = self.load().await?;
        node_mut(&mut roadmap.nodes, id)?.priority = priority;
        self.write(roadmap).await
    }

    /// Marks a card absolute or not ([`crate::model::Node::absolute`]).
    ///
    /// # Errors
    /// When the node is not there or the file cannot be written.
    pub async fn set_absolute(&self, id: i64, absolute: bool) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        node_mut(&mut roadmap.nodes, id)?.absolute = absolute;
        self.write(roadmap).await
    }

    /// Adds a gambit, active, with its weight and the words of its focus; returns its id, the
    /// highest plus one.
    ///
    /// # Errors
    /// When the text is empty, the weight is out of range, a focus word holds whitespace or the
    /// file cannot be written.
    pub async fn add_gambit(&self, text: &str, weight: u8, focus: &[String]) -> anyhow::Result<u32> {
        anyhow::ensure!(!text.trim().is_empty(), "a gambit is not empty");
        ensure_weight(Some(weight), "weight")?;
        anyhow::ensure!(focus.iter().all(|w| !w.is_empty() && !w.contains(char::is_whitespace)), "a focus word holds no whitespace");
        let mut roadmap = self.load().await?;
        let id = roadmap.gambits.iter().map(|g| g.id).max().unwrap_or(0) + 1;
        roadmap.gambits.push(Gambit {
            id,
            text: text.trim().to_owned(),
            weight,
            state: GambitState::Active,
            focus: focus.to_vec(),
            plan_next: false,
        });
        self.write(roadmap).await?;
        Ok(id)
    }

    /// Sets the state of a gambit. A done gambit stays in the file.
    pub async fn set_gambit_state(&self, id: u32, state: GambitState) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        gambit_mut(&mut roadmap.gambits, id)?.state = state;
        self.write(roadmap).await
    }

    /// Sets the weight of a gambit, 1 to 5.
    pub async fn set_gambit_weight(&self, id: u32, weight: u8) -> anyhow::Result<()> {
        ensure_weight(Some(weight), "weight")?;
        let mut roadmap = self.load().await?;
        gambit_mut(&mut roadmap.gambits, id)?.weight = weight;
        self.write(roadmap).await
    }

    /// Replaces the focus words of a gambit; none makes it match every card.
    ///
    /// # Errors
    /// When a word holds whitespace, the gambit is unknown or the file cannot be written.
    pub async fn set_gambit_focus(&self, id: u32, focus: &[String]) -> anyhow::Result<()> {
        anyhow::ensure!(focus.iter().all(|w| !w.is_empty() && !w.contains(char::is_whitespace)), "a focus word holds no whitespace");
        let mut roadmap = self.load().await?;
        gambit_mut(&mut roadmap.gambits, id)?.focus = focus.to_vec();
        self.write(roadmap).await
    }

    /// Sets whether a gambit lets the planner fill the roadmap when no card is ready.
    ///
    /// # Errors
    /// When the gambit is unknown or the file cannot be written.
    pub async fn set_gambit_plan_next(&self, id: u32, plan_next: bool) -> anyhow::Result<()> {
        let mut roadmap = self.load().await?;
        gambit_mut(&mut roadmap.gambits, id)?.plan_next = plan_next;
        self.write(roadmap).await
    }

    /// Writes the file back as the `rhipper.md` text. The file is small, so a blocking write stays
    /// fine.
    async fn write(&self, roadmap: Roadmap) -> anyhow::Result<()> {
        let text = markdown::render_file(&roadmap);
        std::fs::write(&self.path, text).with_context(|| format!("cannot write {}", self.path.display()))
    }
}

/// Seconds since the epoch, for a history line.
/// Where a moved subtree goes among the new parent's children.
#[derive(Clone, Copy)]
enum Slot {
    First,
    Last,
    After(i64),
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The ids in their order, each once: a dependency named twice is one dependency.
fn dedup(ids: &[i64]) -> Vec<i64> {
    let mut out: Vec<i64> = Vec::with_capacity(ids.len());
    for id in ids {
        if !out.contains(id) {
            out.push(*id);
        }
    }
    out
}

/// The node with `id`, mutable.
fn node_mut<'a>(nodes: &'a mut [Node], id: i64) -> anyhow::Result<&'a mut Node> {
    nodes.iter_mut().find(|n| n.id == id).with_context(|| format!("no node {id}"))
}

/// The gambit with `id`, mutable.
fn gambit_mut<'a>(gambits: &'a mut [Gambit], id: u32) -> anyhow::Result<&'a mut Gambit> {
    gambits.iter_mut().find(|g| g.id == id).with_context(|| format!("no gambit g{id}"))
}

/// A priority or a weight sits in [`WEIGHT_RANGE`]; `what` names it in the error.
fn ensure_weight(value: Option<u8>, what: &str) -> anyhow::Result<()> {
    anyhow::ensure!(value.is_none_or(|v| WEIGHT_RANGE.contains(&v)), "the {what} is {} to {}", WEIGHT_RANGE.start(), WEIGHT_RANGE.end());
    Ok(())
}

#[cfg(test)]
mod tests;
