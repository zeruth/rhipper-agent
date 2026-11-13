//! The router: the model of a card when neither the card nor the command names one. The rules
//! are the `[[route]]` tables of `rhipper.toml` beside the roadmap, read in order; the first that
//! fits the card's complexity, its kind, its files, its title, its rejections and the role asked
//! for names the model and, when it says
//! so, the effort. A card's own model wins over every rule; no file, or no rule that fits, is the
//! default model.

use std::fmt;
use std::path::Path;

use anyhow::Context as _;
use ra_roadmap::model::{Effort, Node, NodeKind};
use serde::Deserialize;

/// The rules file, beside the roadmap.
pub const ROUTES_FILE: &str = "rhipper.toml";
/// The role of the worker that implements a card; a rule without a role is for it.
pub const IMPLEMENTER: &str = "implementer";

/// The role of the second model that reads a card's diff against its acceptance.
pub const REVIEWER: &str = "reviewer";

/// The role of the architect that splits a milestone or a section into cards.
pub const PLANNER: &str = "planner";

/// The complexities a rule is for: one, or a range with both ends in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Span {
    One(u8),
    Between(u8, u8),
}

impl Span {
    /// Whether `complexity` is in the span.
    #[must_use]
    pub fn holds(self, complexity: u8) -> bool {
        match self {
            Self::One(c) => c == complexity,
            Self::Between(low, high) => (low.min(high)..=low.max(high)).contains(&complexity),
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::One(c) => write!(f, "c{c}"),
            Self::Between(low, high) => write!(f, "c{low}..c{high}"),
        }
    }
}

/// One `[[route]]` table: what it is for, and the model and effort it names. A field left out
/// fits every card; every field named must hold.
///
/// ```toml
/// [[route]]
/// complexity = [1, 2]
/// files = ["apps/**", "!crates/**"]   # a card file under apps/, and none under crates/
/// words = ["gotcha", "docs"]          # one of them in the title, lower-cased
/// rejections = 1                      # once the reviewer sent the card back this often
/// model = "claude-sonnet-5-5"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Rule {
    pub complexity: Option<Span>,
    pub kind: Option<NodeKind>,
    /// Globs on the card's `files:`: one of them matches a file of the card, and none of the
    /// ones led by `!` does. `*` stays in a directory, `**` crosses them.
    #[serde(default)]
    pub files: Vec<String>,
    /// Words of which one is in the card's title, lower-cased.
    #[serde(default)]
    pub words: Vec<String>,
    /// The times the reviewer sent the card back before the rule fits it.
    pub rejections: Option<usize>,
    #[serde(default = "implementer")]
    pub role: String,
    pub model: String,
    pub effort: Option<Effort>,
}

fn implementer() -> String {
    IMPLEMENTER.to_owned()
}

impl Rule {
    /// Whether the rule is for `node` run in `role`, sent back `rejections` times so far.
    #[must_use]
    pub fn fits(&self, node: &Node, role: &str, rejections: usize) -> bool {
        self.role.eq_ignore_ascii_case(role)
            && self.complexity.is_none_or(|span| span.holds(node.complexity))
            && self.kind.is_none_or(|kind| kind == node.kind)
            && self.files_hold(node)
            && (self.words.is_empty() || self.word_in(node).is_some())
            && self.rejections.is_none_or(|least| rejections >= least)
    }

    /// Whether the card's files are what `files` asks for.
    fn files_hold(&self, node: &Node) -> bool {
        let (excluded, wanted): (Vec<&str>, Vec<&str>) = self.files.iter().map(String::as_str).partition(|glob| glob.starts_with('!'));
        let touches = |glob: &str| node.files.iter().any(|file| glob_matches(glob, file));
        (wanted.is_empty() || self.file_glob(node).is_some()) && !excluded.iter().any(|glob| touches(&glob[1..]))
    }

    /// The first wanted glob a file of the card matches.
    fn file_glob(&self, node: &Node) -> Option<&str> {
        self.files
            .iter()
            .map(String::as_str)
            .filter(|glob| !glob.starts_with('!'))
            .find(|glob| node.files.iter().any(|file| glob_matches(glob, file)))
    }

    /// The first word of the rule in the card's title.
    fn word_in(&self, node: &Node) -> Option<&str> {
        let title = node.title.to_lowercase();
        self.words.iter().map(String::as_str).find(|word| title.contains(&word.to_lowercase()))
    }

    /// The conditions of the rule that a card it fits met, the most telling first: `files
    /// apps/rc-gui/**`, `words gotcha`, `rejections 1`, then the span and the kind; `every card`
    /// for a rule that names none.
    #[must_use]
    pub fn matched(&self, node: &Node) -> String {
        let mut terms = Vec::new();
        if !self.files.is_empty() {
            terms.push(format!("files {}", self.file_glob(node).map_or_else(|| self.files.join(", "), str::to_owned)));
        }
        if let Some(word) = self.word_in(node) {
            terms.push(format!("words {word}"));
        }
        if let Some(least) = self.rejections {
            terms.push(format!("rejections {least}"));
        }
        if let Some(span) = self.complexity {
            terms.push(span.to_string());
        }
        if let Some(kind) = self.kind {
            terms.push(kind.to_string());
        }
        if terms.is_empty() { "every card".to_owned() } else { terms.join(" · ") }
    }
}

/// Whether `path` matches `glob`: `**` is any run of characters, `*` any run without a `/`, `?`
/// one character; a backslash in either reads as `/`, and a trailing `/**` takes the directory.
fn glob_matches(glob: &str, path: &str) -> bool {
    fn walk(glob: &[u8], path: &[u8]) -> bool {
        match glob {
            [] => path.is_empty(),
            // `**/` is no directory or any run of them
            [b'*', b'*', b'/', rest @ ..] => {
                walk(rest, path) || path.iter().enumerate().any(|(at, c)| *c == b'/' && walk(rest, &path[at + 1..]))
            }
            [b'*', b'*', rest @ ..] => (0..=path.len()).any(|at| walk(rest, &path[at..])),
            [b'*', rest @ ..] => {
                let end = path.iter().position(|c| *c == b'/').unwrap_or(path.len());
                (0..=end).any(|at| walk(rest, &path[at..]))
            }
            [b'?', rest @ ..] => path.first().is_some_and(|c| *c != b'/') && walk(rest, &path[1..]),
            [c, rest @ ..] => path.first() == Some(c) && walk(rest, &path[1..]),
        }
    }
    let glob = glob.trim().replace('\\', "/");
    let path = path.trim().replace('\\', "/");
    let path = path.trim_start_matches("./");
    walk(glob.trim_start_matches("./").as_bytes(), path.as_bytes()) || glob.strip_suffix("/**").is_some_and(|dir| dir == path)
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut terms = Vec::new();
        if let Some(span) = self.complexity {
            terms.push(span.to_string());
        }
        if let Some(kind) = self.kind {
            terms.push(kind.to_string());
        }
        if !self.files.is_empty() {
            terms.push(format!("files {}", self.files.join(", ")));
        }
        if !self.words.is_empty() {
            terms.push(format!("words {}", self.words.join(", ")));
        }
        if let Some(least) = self.rejections {
            terms.push(format!("rejections {least}"));
        }
        terms.push(self.role.clone());
        write!(f, "{} → {}", terms.join(" · "), self.model)?;
        if let Some(effort) = self.effort {
            write!(f, " (effort {effort})")?;
        }
        Ok(())
    }
}

/// The `[worker]` table: what a project's workers get beyond the defaults.
///
/// ```toml
/// [worker]
/// allow = ["rhtest", "Bash(ninja *)"]   # commands the worker may run here, on top of the defaults
/// binaries = ["testdata/rhtest/*.exe"]  # test binaries of the project a worker may run natively
/// check = ".\\gw.ps1 :core:test"         # the check of a card whose acceptance names none
/// lint = ""                             # the lint after a passed check; empty is none, absent is clippy
/// turns = 20                            # a run ends after this many turns and the card goes on in a fresh session
/// context = 120000                      # and after a turn whose input passes this many tokens; 120000 without it
/// check_timeout = 1800                 # the seconds a check or the lint may run; 1200 without it
/// parallel = 2                         # the cards a walk runs at once, in worktrees sharing one build; one without it
/// carry = ["local.properties"]         # untracked files a new worktree gets a copy of, directories a link to
/// read = ["third-party/jdk8u"]         # directories outside the worktree a run may read
/// borrow = false                       # no borrowing of the pool's spare Opus slots past `parallel`; true without it
/// shell = "bash"                       # the one shell a worker gets; both without it
/// web = true                           # the web tools for builders; investigators have them anyway
/// [worker.env]
/// CARGO_BUILD_JOBS = "6"               # the environment its harness runs with
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct WorkerConfig {
    /// Extra permission rules: a Claude Code rule such as `Bash(ninja *)`, or a bare command
    /// prefix such as `rhtest`, which stands for the command and its arguments in both shells.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Path globs of the project's own test binaries a worker may run, such as
    /// `testdata/rhtest/*.exe`: relative to the project, `*` inside one segment. Any other
    /// executable stays refused.
    #[serde(default)]
    pub binaries: Vec<String>,
    /// The environment the worker's harness runs with, on top of the agent's own.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// The check of a card whose acceptance names no command; the workspace tests without it.
    #[serde(default)]
    pub check: Option<String>,
    /// The lint of what a run changed, once its check passed: absent, clippy over the changed
    /// crates, as a Rust project wants; empty, no lint for this project; else this command.
    #[serde(default)]
    pub lint: Option<String>,
    /// The turns a run gets before it ends with a handoff and the walk starts the card again in
    /// a fresh session, so the context every turn carries starts small again; absent or zero, a
    /// run goes on until it answers.
    #[serde(default)]
    pub turns: Option<u32>,
    /// The input tokens a turn may carry before the run ends the same way. Absent or zero, no
    /// cap: the harness compacts its own context as it goes, and a forced restart throws away
    /// what the session holds (the person: "claude manages context on its own … let it keep
    /// going unless we have a need to /clear"). A project that wants the restart names a count.
    #[serde(default)]
    pub context: Option<u64>,
    /// The seconds a card's check, or its lint, may run before it is stopped and counts as
    /// failed; absent or zero, the agent's twenty minutes. A suite that builds and runs every
    /// reference program outgrows that (rhipper-asm #69), and the project says so here.
    #[serde(default)]
    pub check_timeout: Option<u64>,
    /// The cards a walk of this project runs at once, each in its own worktree, when
    /// `rc walk` names no `--parallel`; absent, zero or one, one after another. The runs share
    /// one build: the project's target directory and the lock on it.
    #[serde(default)]
    pub parallel: Option<u32>,
    /// Files of the project a new worktree gets a copy of: the ones git does not track that a
    /// build needs all the same, such as `local.properties` with the Android SDK's location.
    /// Without it every worktree run of runelite failed its compile at once. A directory is
    /// linked rather than copied: a JDK source tree is gigabytes, and one copy serves them all.
    #[serde(default)]
    pub carry: Vec<String>,
    /// Directories a run may read outside its worktree, relative to the project or absolute:
    /// a gitignored source tree such as runelite's `third-party/jdk8u`, which a worktree does
    /// not have and some 130 refused reads asked for (2026-11-12). They go to the harness as
    /// added directories, beside the toolchain caches the agent finds on its own.
    #[serde(default)]
    pub read: Vec<String>,
    /// Whether a walk here may borrow the pool's Opus slots no other walk needs, past its
    /// `parallel`; absent, it may. `false` holds a project to its own count exactly.
    #[serde(default)]
    pub borrow: Option<bool>,
    /// The investigators a walk runs beside its builders: read-only runs, one a ready card
    /// ahead of the builders, each leaving a research brief (`var/briefs/<id>.md`) the
    /// builder of the card opens on: where the code is, what to change, what to leave. They
    /// build nothing and edit nothing, so they cost the machine little; absent or zero, none.
    #[serde(default)]
    pub investigators: Option<u32>,
    /// The builds one session of a worker may run itself; absent, two. Zero makes the
    /// builds the orchestrator's alone: the workers write and report, and the project's check
    /// compiles each report on one of the two build slots, so six workers never build at once.
    #[serde(default)]
    pub builds: Option<u32>,
    /// The directories the project's tests make under the system temp directory, by name
    /// (`ra-asm-emit-tests` for `%TEMP%\ra-asm-emit-tests`): a landing removes their entries
    /// older than an hour, and the doctor says how much they hold. Nothing else under temp is
    /// touched. (58 GB of asm's test fixtures had piled up by 2026-11-12.)
    #[serde(default)]
    pub temp: Vec<String>,
    /// The one shell the project's workers get, `bash` or `powershell`: each shell tool's schema
    /// is read at the start of every run, and the allowed shapes serve both. Absent, both.
    #[serde(default)]
    pub shell: Option<String>,
    /// Whether the project's builders get the web tools (`WebFetch`, `WebSearch`); an
    /// investigator has them regardless. Absent, no.
    #[serde(default)]
    pub web: Option<bool>,
}

impl WorkerConfig {
    /// The cards a walk runs at once, when the table names more than one.
    #[must_use]
    pub fn parallel(&self) -> Option<u32> {
        self.parallel.filter(|n| *n > 1)
    }

    /// The investigators a walk runs beside its builders.
    #[must_use]
    pub fn investigators(&self) -> usize {
        self.investigators.unwrap_or(0) as usize
    }

    /// The builds one session of a worker may run itself.
    #[must_use]
    pub fn builds(&self) -> u32 {
        self.builds.unwrap_or(crate::agent::BUILDS_A_SESSION)
    }

    /// Whether a walk may borrow the pool's spare Opus slots past its own count.
    #[must_use]
    pub fn borrow(&self) -> bool {
        self.borrow.unwrap_or(true)
    }

    /// How long a check or the lint may run: the table's seconds, else the agent's default.
    #[must_use]
    pub fn check_timeout(&self) -> std::time::Duration {
        self.check_timeout.filter(|secs| *secs > 0).map_or(crate::agent::CHECK_TIMEOUT, std::time::Duration::from_secs)
    }

    /// The project's default check, when the table names a non-empty one.
    #[must_use]
    pub fn check(&self) -> Option<&str> {
        self.check.as_deref().map(str::trim).filter(|c| !c.is_empty())
    }

    /// Whether the project's check is the full suite: none named, which is the workspace tests,
    /// or a `cargo test` over no package. A worker's own run of it is then narrowed to the crates
    /// it changed, as the orchestrator runs the suite after the report.
    #[must_use]
    pub fn checks_the_suite(&self) -> bool {
        self.check().is_none_or(ra_gotchas::is_suite)
    }

    /// The turns after which a run ends and the card continues fresh, when the table names a
    /// positive count.
    #[must_use]
    pub fn turns(&self) -> Option<u32> {
        self.turns.filter(|turns| *turns > 0)
    }

    /// The input tokens a turn may carry before the run ends and the card continues fresh: the
    /// table's count when positive; `None`, no cap, otherwise.
    #[must_use]
    pub fn context(&self) -> Option<u64> {
        self.context.filter(|tokens| *tokens > 0)
    }

    /// The project's lint: `None` leaves the lint to the agent, `Some("")` turns it off.
    #[must_use]
    pub fn lint(&self) -> Option<&str> {
        self.lint.as_deref().map(str::trim)
    }

    /// The permission rules of `allow`: a bare prefix becomes a rule for each shell.
    #[must_use]
    pub fn rules(&self) -> Vec<String> {
        let mut rules = Vec::new();
        for entry in self.allow.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
            if entry.contains('(') {
                rules.push(entry.to_owned());
            } else {
                rules.push(format!("Bash({entry} *)"));
                rules.push(format!("Bash({entry})"));
                rules.push(format!("PowerShell({entry} *)"));
                rules.push(format!("PowerShell({entry})"));
            }
        }
        rules
    }

    /// Whether `command` starts with one of the allowed prefixes: such a command is the
    /// project's own, outside the shell allowlist the gotchas guard.
    #[must_use]
    pub fn allows(&self, command: &str) -> bool {
        let command = command.trim_start();
        self.allow
            .iter()
            .filter_map(|e| prefix_of(e))
            .any(|prefix| command.starts_with(&prefix) && command[prefix.len()..].chars().next().is_none_or(char::is_whitespace))
            || self.runs_listed_binary(command)
    }

    /// Whether the command's program is a test binary the project listed in `binaries`.
    fn runs_listed_binary(&self, command: &str) -> bool {
        let program = command.split_whitespace().next().unwrap_or("").trim_matches(['"', '\'']).replace('\\', "/");
        let program = program.strip_prefix("./").unwrap_or(&program);
        // a path that leaves the project is no listed binary, whatever its glob says
        if program.is_empty() || program.starts_with('/') || program.contains(':') || program.split('/').any(|part| part == "..") {
            return false;
        }
        self.binaries.iter().any(|glob| glob_matches(glob, program))
    }
}

/// The command prefix of an allow entry: `rhtest` of `rhtest`, `ninja` of `Bash(ninja *)`.
fn prefix_of(entry: &str) -> Option<String> {
    let entry = entry.trim();
    let inner = match entry.find('(') {
        Some(at) => entry[at + 1..].trim_end_matches(')'),
        None => entry,
    };
    let prefix = inner.trim().trim_end_matches('*').trim_end();
    (!prefix.is_empty()).then(|| prefix.to_owned())
}

/// The `[role]` table: the model of each cheap role a run ends with. A key left out takes the
/// local server when it answers a ping and no model call when it does not; an empty string turns
/// the role off, whatever the server does.
///
/// ```toml
/// [role]
/// summary = "local/qwen3-8b"     # the report condensed for the handoff
/// digest = "local/qwen3-8b"      # the run's line in the project's digest
/// classify = ""                  # whether an untagged look-back is [machine]: off
/// fix = "local/qwen3-8b"         # the fix pass of a trivially rejected card; off without a server
/// reviewer = "sonnet"            # the judge; one rung below the implementer when left out
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RoleConfig {
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub classify: Option<String>,
    /// The fix pass, the one role with tools: like the others it is off, not on a Claude model,
    /// when no local server answers and the table names nothing.
    #[serde(default)]
    pub fix: Option<String>,
    /// The judge of a card's run, read before the `reviewer` route rules; left out, it is one
    /// rung below the implementer on the ladder ([`Ladder::judge`]), so a model never reviews
    /// its own work.
    #[serde(default)]
    pub reviewer: Option<String>,
}

impl RoleConfig {
    /// The table's word on `role`: `None` left out, `Some("")` off, else the model.
    #[must_use]
    pub fn model(&self, role: ra_providers::local::Role) -> Option<&str> {
        use ra_providers::local::Role;
        match role {
            Role::Summary => &self.summary,
            Role::Digest => &self.digest,
            Role::Classify => &self.classify,
        }
        .as_deref()
        .map(str::trim)
    }
}

/// The `[plan]` table: how the week is spent by a run on a `claude-*` model, unless `--force`.
/// The ladder's marks move the small cards off the Claude route first and hold the big ones
/// last; without a ladder the default one stands, its last rung on `threshold`.
///
/// ```toml
/// [plan]
/// threshold = 0.95    # the share of the week of the default ladder's hold; 0.9 when left out
/// ladder = [
///     { at = 0.5, span = "1-2", to = "local" },
///     { at = 0.75, span = "1-3", to = "claude-haiku-4-5" },
///     { at = 0.9, span = "1-5", to = "hold", except_priority = 1 },
/// ]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PlanConfig {
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub ladder: Option<Vec<Mark>>,
}

/// Where a [`Mark`] sends a card to the local server's model, refused when none is up.
pub const TO_LOCAL: &str = "local";

/// Where a [`Mark`] holds a card: on the local server's model when one is up, else the run waits
/// for the week to reset.
pub const TO_HOLD: &str = "hold";

/// One rung of `[plan] ladder`: once the week is at `at`, a card whose complexity `span` holds
/// goes `to` the local server, a hold, or the model it names; a card of `except_priority` or
/// higher passes the rung.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Mark {
    pub at: f64,
    #[serde(deserialize_with = "span_of")]
    pub span: Span,
    pub to: String,
    #[serde(default)]
    pub except_priority: Option<u8>,
}

impl Mark {
    /// Whether the rung is for `node`: its span holds the card and the card's priority does not
    /// pass it.
    #[must_use]
    pub fn holds(&self, node: &Node) -> bool {
        let passes = self.except_priority.is_some_and(|least| node.priority.is_some_and(|p| p >= least));
        self.span.holds(node.complexity) && !passes
    }
}

/// A span as a mark writes it, `"1-2"` or `"3"`, or as a rule does, `[1, 2]` or `3`.
fn span_of<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Span, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Written {
        Span(Span),
        Text(String),
    }
    let text = match Written::deserialize(deserializer)? {
        Written::Span(span) => return Ok(span),
        Written::Text(text) => text,
    };
    let number =
        |part: &str| -> Result<u8, D::Error> { part.trim().trim_start_matches(['c', 'C']).parse::<u8>().map_err(serde::de::Error::custom) };
    match text.split_once('-') {
        Some((low, high)) => Ok(Span::Between(number(low)?, number(high)?)),
        None => Ok(Span::One(number(&text)?)),
    }
}

impl PlanConfig {
    /// The threshold, 0 to 1: the table's when it names a sane one, else the default.
    #[must_use]
    pub fn threshold(&self) -> f64 {
        self.threshold.filter(|t| (0.0..=1.0).contains(t)).unwrap_or(ra_providers::limits::DEFAULT_THRESHOLD)
    }

    /// The ladder's marks: the table's, else c1–2 to the local server at half the week, c1–3 to
    /// Haiku at three quarters and every card but a priority one held at the threshold.
    #[must_use]
    pub fn marks(&self) -> Vec<Mark> {
        if let Some(ladder) = &self.ladder {
            return ladder.clone();
        }
        let mark = |at: f64, span: Span, to: &str, except_priority: Option<u8>| Mark { at, span, to: to.to_owned(), except_priority };
        vec![
            mark(ra_providers::limits::LOCAL_MARK, Span::Between(1, 2), TO_LOCAL, None),
            mark(ra_providers::limits::CHEAP_MARK, Span::Between(1, 3), "claude-haiku-4-5", None),
            mark(self.threshold(), Span::Between(1, 5), TO_HOLD, Some(1)),
        ]
    }

    /// The rung `node` stands on with the week at `week`, 0 to 1, and its place counted from one:
    /// the highest mark the week has passed that is for the card, the earlier on a tie.
    #[must_use]
    pub fn rung(&self, node: &Node, week: f64) -> Option<(usize, Mark)> {
        self.marks()
            .into_iter()
            .enumerate()
            .filter(|(_, mark)| week >= mark.at && mark.holds(node))
            .reduce(|best, next| if next.1.at > best.1.at { next } else { best })
            .map(|(index, mark)| (index + 1, mark))
    }
}

/// The `[ladder]` table: the models a routed card climbs, the cheapest first. A small card
/// starts below the route's model and a card the reviewer sent back steps up, so a card that
/// fails on a cheap model gets a stronger one instead of the same one again, and a small card
/// does not spend the strongest. A rule's model is the ceiling of the cards it routes; the
/// card's own model never moves, nor does a route model the ladder does not list.
///
/// ```toml
/// [ladder]
/// models = ["haiku", "sonnet", "opus", "fable"]
/// start = [{ complexity = [1, 2], down = 1 }]   # c1–2 one rung below the route's model, c3+ on it
/// up_after = 1                                  # a rung up for every rejection
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Ladder {
    pub models: Vec<String>,
    pub start: Vec<Start>,
    /// The rejections a card takes for each rung up; zero never steps up.
    pub up_after: usize,
}

/// Where the cards of a complexity span start: `down` rungs below the route's model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Start {
    pub complexity: Span,
    #[serde(default)]
    pub down: usize,
}

impl Default for Ladder {
    fn default() -> Self {
        Self {
            models: ["claude-haiku-4-5", "claude-sonnet-4-5", "claude-opus-5-5", "claude-fable-5-2"].map(str::to_owned).to_vec(),
            start: vec![Start { complexity: Span::Between(1, 2), down: 1 }],
            up_after: 1,
        }
    }
}

/// A move on the ladder: the model a card runs on instead of its route's, and the words the
/// start line, the handoff and the dry run say it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub model: String,
    pub note: String,
}

impl Ladder {
    /// The rung `node` runs on when `choice` is its route and the reviewer sent it back
    /// `rejections` times: `None` when it stays on the route's model.
    #[must_use]
    pub fn step(&self, node: &Node, choice: &Choice, rejections: usize) -> Option<Step> {
        if choice.why == Why::Card {
            return None;
        }
        let id = |model: &str| crate::agent::resolve(model);
        let route = id(&choice.model);
        let at = self.models.iter().position(|model| id(model) == route)?;
        let ceiling = if matches!(choice.why, Why::Rule(_)) { at } else { self.models.len() - 1 };
        let down = self.start.iter().find(|start| start.complexity.holds(node.complexity)).map_or(0, |start| start.down);
        let base = at.saturating_sub(down);
        let rung = (base + rejections.checked_div(self.up_after).unwrap_or(0)).min(ceiling);
        let model = id(&self.models[rung]);
        let said = if rung > base {
            format!("escalated to {model} after {rejections} rejection{}", if rejections == 1 { "" } else { "s" })
        } else if rung < at {
            let rungs = if at - rung == 1 { "one rung".to_owned() } else { format!("{} rungs", at - rung) };
            format!("started {rungs} down (c{})", node.complexity)
        } else {
            return None;
        };
        Some(Step { model, note: said })
    }

    /// The default reviewer of a card of `complexity` run on `implementer`: one rung below it,
    /// never under the ladder's first sonnet for c4–5, and a rung above when the floor or the
    /// bottom of the ladder would leave the implementer judging itself. `None` when the
    /// implementer is not on the ladder or the ladder has no other rung.
    #[must_use]
    pub fn judge(&self, implementer: &str, complexity: u8) -> Option<String> {
        let id = |model: &str| crate::agent::resolve(model);
        let own = id(implementer);
        let at = self.models.iter().position(|model| id(model) == own)?;
        let floor =
            if complexity >= 4 { self.models.iter().position(|model| id(model).starts_with("claude-sonnet-")).unwrap_or(0) } else { 0 };
        let mut rung = at.saturating_sub(1).max(floor);
        if rung == at {
            rung = at + 1;
        }
        self.models.get(rung).map(|model| id(model))
    }

    /// `model` itself when it is the ladder's first sonnet or above it (or off the ladder), else
    /// that sonnet: the reviewer's floor, whatever rung the implementer started on. A haiku
    /// review of #219 returned 19 points where Sonnet accepted.
    #[must_use]
    pub fn at_least_sonnet(&self, model: String) -> String {
        let id = |model: &str| crate::agent::resolve(model);
        let Some(floor) = self.models.iter().position(|m| id(m).starts_with("claude-sonnet-")) else {
            return model;
        };
        match self.models.iter().position(|m| id(m) == model) {
            Some(at) if at < floor => id(&self.models[floor]),
            _ => model,
        }
    }
}

/// The rules of a project, in the order of the file, its worker table, its role table, its
/// plan table and its ladder.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Router {
    #[serde(default, rename = "route")]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub worker: WorkerConfig,
    #[serde(default)]
    pub role: RoleConfig,
    #[serde(default)]
    pub plan: PlanConfig,
    #[serde(default)]
    pub ladder: Ladder,
}

/// Why a choice fell where it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The card names its own model.
    Card,
    /// The rule at this index, counted from one, fits the card.
    Rule(usize),
    /// Neither the card nor a rule names one.
    Default,
}

/// The model a card runs on, the effort when a rule set one, and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub model: String,
    pub effort: Option<Effort>,
    pub why: Why,
}

impl Router {
    /// The rules of `text`, a `rhipper.toml`.
    ///
    /// # Errors
    /// When the text is not the rules file: a rule without a model, an unknown kind or effort.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        toml::from_str(text).context("the routes do not parse")
    }

    /// The rules of the `rhipper.toml` in `root`; none when there is no file.
    ///
    /// # Errors
    /// When the file is there but cannot be read or parsed.
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = root.join(ROUTES_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    /// The first rule for `node` in `role`, sent back `rejections` times so far, with its place
    /// counted from one.
    #[must_use]
    pub fn pick(&self, node: &Node, role: &str, rejections: usize) -> Option<(usize, &Rule)> {
        self.rules.iter().enumerate().find(|(_, rule)| rule.fits(node, role, rejections)).map(|(index, rule)| (index + 1, rule))
    }

    /// The model of `node` run in `role`: the card's own first, else the first rule that fits,
    /// else `default`.
    #[must_use]
    pub fn choose(&self, node: &Node, role: &str, default: &str, rejections: usize) -> Choice {
        if let Some(own) = &node.assignee {
            return Choice { model: own.model.clone(), effort: None, why: Why::Card };
        }
        match self.pick(node, role, rejections) {
            Some((position, hit)) => Choice { model: hit.model.clone(), effort: hit.effort, why: Why::Rule(position) },
            None => Choice { model: default.to_owned(), effort: None, why: Why::Default },
        }
    }

    /// [`Router::choose`], then the ladder for the implementer: the choice on the rung's model,
    /// and the step that moved it there when one did.
    #[must_use]
    pub fn route(&self, node: &Node, role: &str, default: &str, rejections: usize) -> (Choice, Option<Step>) {
        let mut choice = self.choose(node, role, default, rejections);
        let step = if role.eq_ignore_ascii_case(IMPLEMENTER) { self.ladder.step(node, &choice, rejections) } else { None };
        if let Some(step) = &step {
            choice.model.clone_from(&step.model);
        }
        (choice, step)
    }

    /// The judge of `node` run on `implementer` and sent back `rejections` times, with the
    /// words the review line says it with: the `[role] reviewer` key, else the first `reviewer`
    /// rule, else one rung below the implementer ([`Ladder::judge`]); the implementer itself
    /// only when it is off the ladder.
    #[must_use]
    pub fn reviewer(&self, node: &Node, implementer: &str, rejections: usize) -> (String, String) {
        let resolve = crate::agent::resolve;
        if let Some(model) = self.role.reviewer.as_deref().map(str::trim).filter(|model| !model.is_empty()) {
            return (resolve(model), "role".to_owned());
        }
        if let Some((place, rule)) = self.pick(node, REVIEWER, rejections) {
            return (resolve(&rule.model), format!("rule {place}"));
        }
        match self.ladder.judge(implementer, node.complexity).map(|model| self.ladder.at_least_sonnet(model)) {
            Some(model) => (model, "default, one rung down".to_owned()),
            None => (resolve(implementer), "default, the implementer: off the ladder".to_owned()),
        }
    }

    /// Why `choice` fell where it did for `node`, as the route line says it: `by rule 2: files
    /// apps/rc-gui/**`, `by the card's own model`, or the default with the reason no rule named one.
    #[must_use]
    pub fn reason(&self, node: &Node, choice: &Choice) -> String {
        match choice.why {
            Why::Card => "by the card's own model".to_owned(),
            Why::Rule(place) => self
                .rules
                .get(place - 1)
                .map_or_else(|| format!("by rule {place}"), |rule| format!("by rule {place}: {}", rule.matched(node))),
            Why::Default if self.rules.is_empty() => format!("by default: no {ROUTES_FILE} beside the roadmap"),
            Why::Default => {
                format!("by default: none of the {} rules of {ROUTES_FILE} fits c{} {}", self.rules.len(), node.complexity, node.kind)
            }
        }
    }
}

#[cfg(test)]
mod tests;
