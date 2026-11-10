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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// The nodes that must be accepted first.
    pub depends_on: Vec<i64>,
    pub state: TaskState,
    pub assignee: Option<Assignment>,
    /// Complexity 1..=5, for the router and the cost model.
    pub complexity: u8,
    /// The notes of the last execution: findings, blockers, results.
    pub notes: Option<String>,
}
