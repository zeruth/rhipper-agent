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

/// The effort a run of a card asks of its model: how hard the harness thinks before it acts.
/// Medium is the efficient setting for the routine card; a hard one asks for more, a chore for
/// less. The Claude Code provider passes it as `--effort`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    #[default]
    Medium,
    High,
}

impl Effort {
    /// Every level, from the least.
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    /// The name the store persists, and the harness takes.
    #[must_use]
    pub fn serde_name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// The next level up, wrapping round to the least: a click on the badge cycles them.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Low,
        }
    }
}

impl std::fmt::Display for Effort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.serde_name())
    }
}

impl std::str::FromStr for Effort {
    type Err = anyhow::Error;

    /// The level of its name; the error lists the names.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|level| level.serde_name() == text.trim().to_ascii_lowercase())
            .ok_or_else(|| anyhow::anyhow!("the effort is {}", name_list(&Self::ALL)))
    }
}

/// What a run of a card is expected to spend: the tokens read and written over the run, and the
/// turns it takes. The runner derives one from the complexity; a card's `estimate:` line
/// overrides it, so a run says over or under and the ledger shows the subtrees that ran far over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Estimate {
    pub tokens: u64,
    pub turns: u64,
}

impl Estimate {
    /// The tokens the estimate allows a turn: its tokens over its turns. The context a turn
    /// carries is the cost of a run, so this is the figure a run is judged against.
    #[must_use]
    pub fn a_turn(self) -> u64 {
        self.tokens / self.turns.max(1)
    }

    /// Whether a run that averaged `tokens_a_turn` ran over the estimate: a run of many lean
    /// turns is under, a run of few huge turns is over.
    #[must_use]
    pub fn is_over(self, tokens_a_turn: u64) -> bool {
        tokens_a_turn > self.a_turn()
    }
}

impl std::fmt::Display for Estimate {
    /// The form the store persists, `40k tokens, 12 turns`; a count under a thousand is written
    /// whole.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.tokens >= 1000 && self.tokens.is_multiple_of(1000) {
            write!(f, "{}k tokens, {} turns", self.tokens / 1000, self.turns)
        } else {
            write!(f, "{} tokens, {} turns", self.tokens, self.turns)
        }
    }
}

impl std::str::FromStr for Estimate {
    type Err = anyhow::Error;

    /// The estimate of its line: two counts, the tokens then the turns, with `k` allowed on the
    /// tokens; the words between them are free, so `40k tokens, 12 turns` and `40000 12` both read.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut counts = text.split(|c: char| c.is_whitespace() || c == ',').filter(|w| w.starts_with(|c: char| c.is_ascii_digit()));
        let (Some(tokens), Some(turns)) = (counts.next(), counts.next()) else {
            anyhow::bail!("the estimate is the tokens then the turns, as `40k tokens, 12 turns`");
        };
        let tokens = match tokens.strip_suffix(['k', 'K']) {
            Some(thousands) => thousands.parse::<u64>().map(|n| n * 1000),
            None => tokens.parse::<u64>(),
        }
        .map_err(|_| anyhow::anyhow!("the tokens of an estimate are a count, as 40000 or 40k"))?;
        let turns = turns.parse::<u64>().map_err(|_| anyhow::anyhow!("the turns of an estimate are a count"))?;
        Ok(Self { tokens, turns })
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
    /// The harness session of the last worker run, so the next run resumes it and keeps its
    /// context instead of reading the repository again.
    #[serde(default)]
    pub session: Option<String>,
    /// When the run that made the session ended, in unix seconds: a session older than a day is
    /// not resumed.
    #[serde(default)]
    pub session_at: Option<u64>,
    /// The fingerprint of the card the session worked on, its objective, acceptance and files: a
    /// card changed since then starts fresh.
    #[serde(default)]
    pub session_of: Option<String>,
    /// The nodes that must be accepted first.
    pub depends_on: Vec<i64>,
    pub state: TaskState,
    pub assignee: Option<Assignment>,
    /// Complexity 1..=5, for the router and the cost model.
    pub complexity: u8,
    /// The effort a run asks of the model; medium when the card names none.
    #[serde(default)]
    pub effort: Effort,
    /// The card's own estimate of what a run spends; `None` takes the complexity's.
    #[serde(default)]
    pub estimate: Option<Estimate>,
    /// The notes of the last execution: findings, blockers, results.
    pub notes: Option<String>,
    /// What the last reassessment saw: the word [`FINAL`], which no reassessment rewrites, or a
    /// 16-hex-digit fingerprint of the inputs it judged, so an unchanged node costs no call. The
    /// words after it, each `-` and a 16-hex-digit hash of a title, are the cards the person
    /// declined when a reassessment proposed them, so they do not return.
    #[serde(default)]
    pub assessed: Option<String>,
    /// The person's priority on the card, 1..=5, apart from its complexity: the walk takes a
    /// higher one first. `None` leaves the order to the file and the gambits.
    #[serde(default)]
    pub priority: Option<u8>,
    /// The evidence line a planned card came from, cited as `source:line`, so the person sees why
    /// the planner wrote it; a card without evidence is not planned.
    #[serde(default)]
    pub evidence: Option<String>,
    /// What the planned card claims to save: turns, tokens, a refused command, a person's
    /// intervention.
    #[serde(default)]
    pub saving: Option<String>,
    /// An absolute card works until it is done: no turn or context cap restarts it, a review
    /// that sends it back reruns it at once on its session, and no count of reruns parks or
    /// blocks it; only a full stop (the plan's limit, the provider gone, the person) ends its
    /// run. A card of complexity 5 is absolute whether or not it says so ([`Node::is_absolute`]).
    /// The person's rule of 2026-11-12: the prune cards "should work until done, no limits
    /// unless there is an actual full stop; monumental tasks get the absolute badge as well".
    #[serde(default)]
    pub absolute: bool,
}

impl Node {
    /// Whether the card runs as an absolute card: marked so, or monumental (complexity 5), the
    /// work a cap would cut into restarts that cost more than one whole run.
    #[must_use]
    pub fn is_absolute(&self) -> bool {
        self.absolute || self.complexity >= 5
    }
}

/// The range a priority and a gambit's weight share.
pub const WEIGHT_RANGE: std::ops::RangeInclusive<u8> = 1..=5;

/// The state of a gambit: active ones steer, a parked one waits, a done one stays in the file as
/// a record of the direction taken.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GambitState {
    #[default]
    Active,
    Parked,
    Done,
}

impl GambitState {
    /// Every state, in the order of a gambit's life.
    pub const ALL: [Self; 3] = [Self::Active, Self::Parked, Self::Done];

    /// The name the store persists.
    #[must_use]
    pub fn serde_name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Parked => "parked",
            Self::Done => "done",
        }
    }
}

impl std::fmt::Display for GambitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.serde_name())
    }
}

impl std::str::FromStr for GambitState {
    type Err = anyhow::Error;

    /// The state of its name; the error lists the names.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|state| state.serde_name() == text.trim().to_ascii_lowercase())
            .ok_or_else(|| anyhow::anyhow!("unknown gambit state {}; the state is {}", text.trim(), name_list(&Self::ALL)))
    }
}

/// A gambit: one line of the person's standing direction, with the weight it carries in the
/// walk and the words a card matches on. The file writes it as `- g1 ‹gambit› text {w3} [active]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gambit {
    /// The number after the `g`; the gambits count apart from the nodes.
    pub id: u32,
    pub text: String,
    /// The weight 1..=5: how far the gambit moves a card that matches its focus.
    pub weight: u8,
    pub state: GambitState,
    /// The words a card matches on, in its title, its milestone's title or its files; empty
    /// matches every card.
    #[serde(default)]
    pub focus: Vec<String>,
    /// Whether the planner may fill the roadmap when no card is ready: it plans the next
    /// milestone and adds its cards as defined. The file writes it as a `plan-next: true` line.
    #[serde(default)]
    pub plan_next: bool,
}

/// What a `rhipper.md` holds: the gambits first, then the tree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Roadmap {
    #[serde(default)]
    pub gambits: Vec<Gambit>,
    pub nodes: Vec<Node>,
}

/// The `assessed` mark of a node no reassessment rewrites.
pub const FINAL: &str = "final";

impl Node {
    /// Whether the node is marked final: reassessment leaves it alone.
    pub fn is_final(&self) -> bool {
        self.assessed.as_deref().is_some_and(|a| a.split_whitespace().next() == Some(FINAL))
    }
}
