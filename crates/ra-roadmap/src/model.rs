//! The roadmap types: the hierarchy, the task lifecycle and the records the store persists.

use serde::{Deserialize, Serialize};

/// A node of the roadmap hierarchy: project, milestone, section, task or subtask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Project,
    Milestone,
    Section,
    Task,
    Subtask,
}

impl NodeKind {
    /// Every kind, from the root down.
    pub const ALL: [Self; 5] = [Self::Project, Self::Milestone, Self::Section, Self::Task, Self::Subtask];

    /// The name the store persists.
    pub fn serde_name(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Milestone => "milestone",
            Self::Section => "section",
            Self::Task => "task",
            Self::Subtask => "subtask",
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.serde_name())
    }
}

impl std::str::FromStr for NodeKind {
    type Err = anyhow::Error;

    /// The kind of its store name; the error lists the names.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|kind| kind.serde_name() == text).ok_or_else(|| anyhow::anyhow!("the kind is {}", name_list(&Self::ALL)))
    }
}

/// The lifecycle of a task, from the docs: DEFINED → READY → ASSIGNED → IN_PROGRESS →
/// IMPLEMENTED → TESTING → REVIEW → ACCEPTED, plus the side states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Defined,
    Ready,
    Assigned,
    InProgress,
    Implemented,
    Testing,
    Review,
    Accepted,
    Blocked,
    Failed,
    NeedsRevision,
    Cancelled,
}

impl TaskState {
    /// Every state: the main path in order, then the side states.
    pub const ALL: [Self; 12] = [
        Self::Defined,
        Self::Ready,
        Self::Assigned,
        Self::InProgress,
        Self::Implemented,
        Self::Testing,
        Self::Review,
        Self::Accepted,
        Self::Blocked,
        Self::Failed,
        Self::NeedsRevision,
        Self::Cancelled,
    ];

    /// The name the store persists.
    pub fn serde_name(self) -> &'static str {
        match self {
            Self::Defined => "defined",
            Self::Ready => "ready",
            Self::Assigned => "assigned",
            Self::InProgress => "in_progress",
            Self::Implemented => "implemented",
            Self::Testing => "testing",
            Self::Review => "review",
            Self::Accepted => "accepted",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::NeedsRevision => "needs_revision",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether the state is on the main lifecycle path, not a side state.
    pub fn on_main_path(self) -> bool {
        !matches!(self, Self::Blocked | Self::Failed | Self::NeedsRevision | Self::Cancelled)
    }
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.serde_name())
    }
}

impl std::str::FromStr for TaskState {
    type Err = anyhow::Error;

    /// The state of its store name; the error lists the names.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|state| state.serde_name() == text)
            .ok_or_else(|| anyhow::anyhow!("the state is {}", name_list(&Self::ALL)))
    }
}

/// The names of a list of kinds or states as prose: `a, b, c or d`.
fn name_list<T: std::fmt::Display>(items: &[T]) -> String {
    let names: Vec<String> = items.iter().map(ToString::to_string).collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}

/// Who executes a task, as a model id plus the role it plays. The router resolves it; the roadmap
/// only records the assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    /// The OpenRouter model id, such as `z-ai/glm-5.3-flash`.
    pub model: String,
    /// The worker role, such as `implementer` or `reviewer`.
    pub role: String,
}

/// One node of the roadmap. A task carries the execution fields; a project or milestone carries
/// mostly the title.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: i64,
    /// The parent node; `None` for a project.
    pub parent: Option<i64>,
    pub kind: NodeKind,
    pub title: String,
    /// What this node must achieve, one topic per sentence.
    pub objective: Option<String>,
    /// How acceptance is verified: the checks, the tests, the review requirements.
    pub acceptance: Option<String>,
    /// The workspace-relative paths the task touches, so a worker reads them first instead of
    /// exploring the repository.
    #[serde(default)]
    pub files: Vec<String>,
    /// The nodes that must be accepted first.
    pub depends_on: Vec<i64>,
    pub state: TaskState,
    pub assignee: Option<Assignment>,
    /// Complexity 1..=5, for the router and the cost model.
    pub complexity: u8,
    /// The notes of the last execution: findings, blockers, results.
    pub notes: Option<String>,
    /// What the last reassessment saw: the word [`FINAL`], which no reassessment rewrites, or a
    /// 16-hex-digit fingerprint of the inputs it judged, so an unchanged node costs no call.
    #[serde(default)]
    pub assessed: Option<String>,
}

/// The `assessed` mark of a node no reassessment rewrites.
pub const FINAL: &str = "final";

impl Node {
    /// Whether the node is marked final: reassessment leaves it alone.
    pub fn is_final(&self) -> bool {
        self.assessed.as_deref() == Some(FINAL)
    }
}
