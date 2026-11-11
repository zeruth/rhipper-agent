//! The Rhipper Agent workspace: an IntelliJ-style window over the roadmap and the workers, in
//! the Blood Moon theme. The roadmap tool window down the left, the selected card in the middle,
//! the console across the bottom above the command line; a splash while it loads. It drives the
//! same store, worker, commands and log as the terminal console, and takes its place: one of
//! them runs at a time.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod cards;
mod code;
mod console;
mod fonts;
mod input;
mod markdown;
#[cfg(test)]
mod tests;
mod theme;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;
use egui::{Align, Color32, CornerRadius, FontId, Frame, Key, Layout, Margin, Modifiers, Rect, RichText, Stroke, TextStyle};
use ra_console::agent::{Worker, estimate_of, proposals_of};
use ra_console::commands::{self, Outcome};
use ra_console::logs::{Kind, Sink};
use ra_gotchas::{Catalogue, Journal};
use ra_roadmap::history::{Change, Source};
use ra_roadmap::model::{Gambit, GambitState, Node, NodeKind, TaskState};
use ra_roadmap::store::Store;
use ra_roadmap::tree;
use ra_usage::{Totals, UsageStore};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// How long the splash stays at least.
const SPLASH: Duration = Duration::from_millis(1800);
/// The catalogue of the gotchas and the journal of the ones that fired, as `rc gotchas` reads them.
const GOTCHAS_FILE: &str = "var/gotchas.toml";
const GOTCHAS_LOG: &str = "var/gotchas.log";
/// How many of the refused commands the ledger tab lists, the same as `rc gotchas`.
const UNMATCHED_SHOWN: usize = 20;
/// Where the window keeps its layout between runs.
const LAYOUT_FILE: &str = "var/window.toml";

/// The Claude models a card can be assigned from the card view, strongest first.
const MODELS: [(&str, &str); 3] =
    [("claude-fable-5-1", "Fable 5.1"), ("claude-opus-5-5", "Opus 5.5"), ("claude-haiku-4-5-20251001", "Haiku 4.5")];

/// The four tabs of the card area.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Tab {
    Card,
    Context,
    Diff,
    Ledger,
}

/// What the window remembers of itself in `var/window.toml`: the console band's height, the
/// chips turned off, the folded nodes, the tab, the selected card, the theme and the zoom. A missing or
/// broken file reads as the defaults; a field missing from an older file does too.
#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
struct WindowLayout {
    /// The console band's height in points; zero until the band first draws, when the window
    /// gives it two fifths of the height.
    console: f32,
    hidden: Vec<TaskState>,
    folded: Vec<i64>,
    tab: Option<Tab>,
    selected: Option<i64>,
    /// The theme's name; empty or unknown reads as the default.
    theme: String,
    /// The zoom over the monitor's own scale, `ZOOM_MIN..=ZOOM_MAX`; zero reads as one.
    zoom: f32,
    /// Whether the report groups its bubbles under a divider per project, or interleaves them
    /// by time.
    report_by_project: bool,
}

/// The zoom Ctrl+= and Ctrl+- move between, a step at a time; Ctrl+0 puts it back to one.
const ZOOM_MIN: f32 = 0.8;
const ZOOM_MAX: f32 = 1.6;
const ZOOM_STEP: f32 = 0.1;

impl WindowLayout {
    fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join(LAYOUT_FILE)).ok().and_then(|text| toml::from_str(&text).ok()).unwrap_or_default()
    }

    fn save(&self, root: &Path) {
        if let Err(e) =
            toml::to_string(self).map_err(anyhow::Error::from).and_then(|text| Ok(std::fs::write(root.join(LAYOUT_FILE), text)?))
        {
            tracing::warn!("the layout does not save: {e:#}");
        }
    }
}

/// A directory without a roadmap: where the window started, the roadmap-shaped files it found,
/// the model that would draft or convert, and what is running once a choice is made.
struct Setup {
    dir: String,
    candidates: Vec<PathBuf>,
    model: String,
    running: Option<String>,
}

impl Setup {
    fn new(root: &Path) -> Self {
        let dir =
            std::fs::canonicalize(root).map_or_else(|_| root.display().to_string(), |p| p.display().to_string().replace("\\\\?\\", ""));
        Self { dir, candidates: ra_console::setup::candidates(root), model: ra_console::setup::SETUP_MODEL.to_owned(), running: None }
    }
}

/// The fields of the card view as the person edits them, apart from the card until a save
/// writes them through the store. The files are one path per line here and one word each in
/// the file.
#[derive(Clone, PartialEq, Eq)]
struct Draft {
    id: i64,
    title: String,
    objective: String,
    acceptance: String,
    files: String,
    complexity: u8,
}

impl Draft {
    fn of(node: &Node) -> Self {
        Self {
            id: node.id,
            title: node.title.clone(),
            objective: node.objective.clone().unwrap_or_default(),
            acceptance: node.acceptance.clone().unwrap_or_default(),
            files: node.files.join("\n"),
            complexity: node.complexity,
        }
    }

    /// The paths the files field names, however they are separated.
    fn files(&self) -> Vec<String> {
        self.files.split_whitespace().map(str::to_owned).collect()
    }

    /// Whether the draft says anything the card does not; the mark the view shows, and what
    /// asks before another card is picked.
    fn dirty(&self, node: &Node) -> bool {
        self.title.trim() != node.title
            || self.objective.trim() != node.objective.as_deref().unwrap_or("").trim()
            || self.acceptance.trim() != node.acceptance.as_deref().unwrap_or("").trim()
            || self.files() != node.files
            || self.complexity != node.complexity
    }
}

/// A pick of the roadmap column held back by an unsaved edit: another card, or none.
#[derive(Clone, Copy)]
enum Pick {
    Card(i64),
    Nothing,
}

/// The form a `+` on a milestone or section row opens: the fields of a card to add under it,
/// held apart from the roadmap until Enter writes them through the store as `rc add` would.
/// The files are one path per line, the waits-on ids with or without their `#`.
struct NewCard {
    parent: i64,
    title: String,
    objective: String,
    acceptance: String,
    files: String,
    waits_on: String,
    complexity: u8,
    /// Why the last add was refused, shown under the fields until the next.
    fault: Option<String>,
    /// Whether the title has taken the caret; it does on the form's first frame.
    focused: bool,
}

impl NewCard {
    fn under(parent: i64) -> Self {
        Self {
            parent,
            title: String::new(),
            objective: String::new(),
            acceptance: String::new(),
            files: String::new(),
            waits_on: String::new(),
            complexity: 3,
            fault: None,
            focused: false,
        }
    }

    fn files(&self) -> Vec<String> {
        self.files.split_whitespace().map(|p| p.replace('\\', "/")).collect()
    }

    /// The ids the waits-on field names, or the word that is not one.
    fn waits_on(&self) -> Result<Vec<i64>, String> {
        self.waits_on
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|w| !w.is_empty())
            .map(|w| w.trim_start_matches('#').parse::<i64>().map_err(|_| format!("`{w}` is not a card id")))
            .collect()
    }
}

/// What the form's frame asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FormAct {
    Stay,
    Add,
    Cancel,
}

/// What the context tab shows: the text a run of the card would send, as `rc context <id>`
/// prints it, and the project context that goes ahead of it. Built when the tab shows a card
/// it was not built for, and again when a run ends: the handoffs and the outlines move then.
#[derive(Default)]
struct ContextView {
    /// The card the text was built for.
    id: Option<i64>,
    text: String,
    project: String,
    /// Why the text could not be built, shown in its place.
    fault: Option<String>,
    read_at: Option<Instant>,
}

/// What the card view says of the reviewer: the verdict's first line on the selected card's
/// last handoff, `None` for a card never judged.
#[derive(Default)]
struct VerdictView {
    /// The card the verdict was read for.
    id: Option<i64>,
    text: Option<String>,
}

/// What the diff tab shows: `git diff HEAD` of the files the selected card's last handoff
/// names, one entry per file. Built when the tab shows a card it was not built for, and again
/// when a run ends: the handoff and the working tree move then.
#[derive(Default)]
struct DiffView {
    /// The card the diff was built for.
    id: Option<i64>,
    /// Each file the handoff names with the lines of its diff; empty lines when git has no
    /// change for it, which is a file committed since or one git does not know.
    files: Vec<(String, Vec<String>)>,
    /// Why git could not be asked, shown in its place.
    fault: Option<String>,
    read_at: Option<Instant>,
}

/// What the ledger tab shows, read the way `rc usage` and `rc gotchas` read it: the usage by
/// model and all time from `var/usage.db`, the gotchas by id with their counts, and the refused
/// commands no gotcha names from `var/gotchas.log`. Read at start and again when a run ends.
#[derive(Default)]
struct Ledger {
    by_model: Vec<(String, Totals)>,
    /// The totals by roadmap node, `None` for the calls that served none; the tab sums them
    /// up the tree, so the root reads as the total.
    by_node: Vec<(Option<i64>, Totals)>,
    all_time: Option<Totals>,
    /// The calls of the selected card, the latest first, with the turns and the tokens of each;
    /// read again when the selection moves.
    calls: Vec<ra_usage::Call>,
    /// Each gotcha that fired: the id, how often, and its message from the catalogue, `None`
    /// for an id the catalogue does not hold.
    gotchas: Vec<(String, usize, Option<String>)>,
    /// The refused commands no gotcha names, oldest first.
    unmatched: Vec<String>,
    /// What the runs said would have made them faster, newest first.
    retro: Vec<ra_console::retro::Note>,
    /// How many entries the catalogue holds; what the empty view says.
    catalogue: usize,
    read_at: Option<Instant>,
}

/// How long the toast of a finished run stays; a failed run's stays until it is clicked.
const TOAST_FOR: Duration = Duration::from_secs(8);

/// The announcement of a run that ended, raised in the lower right of the window: the card,
/// the outcome with its cost and its gotcha count, from the log whichever process ran it.
struct Toast {
    /// The card the run was of, when its title named one; a click picks it.
    card: Option<i64>,
    /// The run's title as the log opened it, `#15 Files · opus-5-5`.
    title: String,
    outcome: String,
    failed: bool,
    shown: Instant,
}

fn main() -> eframe::Result<()> {
    // the projects: the ones open last session come back, the arguments join them and the
    // last argument is in front; with neither, this directory. One window for all of them: a
    // second start hands its arguments to the first and leaves
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(|a| plain_path(Path::new(&a))).collect();
    let Some(lock) = ra_console::hub::try_lock() else {
        for project in &args {
            if let Err(e) = ra_console::hub::request_open(project) {
                eprintln!("cannot hand {} to the running window: {e:#}", project.display());
            }
        }
        return Ok(());
    };
    let mut projects: Vec<PathBuf> = ra_console::hub::recent().iter().map(|p| plain_path(p)).filter(|p| p.is_dir()).collect();
    let mut front: Option<PathBuf> = None;
    for arg in args {
        if !projects.contains(&arg) {
            projects.push(arg.clone());
        }
        front = Some(arg);
    }
    if projects.is_empty() {
        projects.push(plain_path(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))));
    }
    let icon = image::load_from_memory(include_bytes!("../assets/icon.png")).ok().map(|img| {
        let rgba = img.to_rgba8();
        egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1500.0, 940.0])
        .with_min_inner_size([980.0, 620.0])
        .with_maximized(true)
        .with_title(window_title(None, &projects[0]));
    if let Some(icon) = icon {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Rhipper Agent", options, Box::new(move |cc| Ok(Box::new(App::new(cc, lock, &projects, front.as_deref())))))
}

/// A path as the window shows and compares it: canonical, without the `\\?\` Windows puts on.
fn plain_path(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    PathBuf::from(canonical.display().to_string().trim_start_matches(r"\\?\"))
}

/// The window: the projects as tabs, one workspace each, and the hub's lock that makes it the
/// one window. The tab strip sits at the top; the active workspace draws under it; the others
/// keep ticking, so a run in a project not in front still shows when it is.
struct App {
    #[allow(dead_code)]
    lock: ra_console::hub::Lock,
    tabs: Vec<Workspace>,
    active: usize,
    /// When the hub's mailbox was last read.
    mailbox_read: Instant,
    /// The path typed into the tab strip's `+`, while it is open.
    adding: Option<String>,
    /// The title the window last set, so it is set only when it changes.
    title: String,
    /// Whether the dashboard is in front: every project at once, instead of the active tab.
    dashboard: bool,
    /// When the window started, for the dashboard's uptime.
    started: Instant,
    /// Whether the report is in front: the cards that landed across every project.
    report: bool,
    /// The report's bubbles in the order they draw, rebuilt when a roadmap or a ledger moves.
    landed: Vec<Landed>,
    /// What the report was built from: each tab's roadmap and ledger stamps, so a frame rebuilds
    /// it only when one of them moved.
    landed_from: Vec<(Option<SystemTime>, Option<Instant>)>,
    /// Why the window goes when it does, for the projects' pair feeds: `close` unless
    /// `rc restart` asked.
    down: &'static str,
    /// The machine's cpu, memory and disk, for the dashboard's gauges.
    gauges: Gauges,
    /// The tokens the runs took, from every open project's ledger, for the row under them.
    tokens: TokenGauges,
}

/// One reading of the machine: the cpu's share in use (0 to 1) and the bytes of memory and of
/// the projects' disk.
#[derive(Clone, Copy, Default)]
struct Reading {
    cpu: f32,
    memory_used: u64,
    memory_total: u64,
    disk_free: u64,
    disk_total: u64,
}

/// The last two minutes of one gauge's readings, 0 to 1, the newest last; a reading past the
/// span pushes the oldest out.
#[derive(Default)]
struct Ring(VecDeque<f32>);

impl Ring {
    fn push(&mut self, value: f32) {
        if self.0.len() == cards::GAUGE_SPAN {
            self.0.pop_front();
        }
        self.0.push_back(value);
    }

    fn samples(&self) -> Vec<f32> {
        self.0.iter().copied().collect()
    }
}

/// The machine's gauges: the readings a background thread takes once a second, kept two
/// minutes deep for the dashboard's graphs. The frame only takes what the thread left.
struct Gauges {
    readings: mpsc::UnboundedReceiver<Reading>,
    latest: Reading,
    cpu: Ring,
    memory: Ring,
    disk: Ring,
}

impl Gauges {
    /// Starts the sampling thread; it refreshes the cpu, the memory and the one disk `root`
    /// lives on, once a second, and stops when the window drops the receiver.
    fn start(root: &Path) -> Self {
        use sysinfo::{DiskRefreshKind, Disks, MemoryRefreshKind, System};
        let (tx, readings) = mpsc::unbounded_channel();
        let root = root.to_path_buf();
        let spawned = std::thread::Builder::new().name("gauges".into()).spawn(move || {
            let mut system = match System::new() {
                Ok(system) => system,
                Err(e) => {
                    tracing::warn!("the machine does not read: {e}");
                    return;
                }
            };
            // the disk the projects live on: the deepest mount point the root is under; the
            // first disk when none holds it; none at all leaves the gauge at zero
            let mut disks = Disks::new_with_refreshed_list().ok();
            let disk = disks.as_ref().and_then(|disks| {
                disks
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| root.starts_with(d.mount_point()))
                    .max_by_key(|(_, d)| d.mount_point().as_os_str().len())
                    .map(|(i, _)| i)
                    .or_else(|| (!disks.is_empty()).then_some(0))
            });
            let memory = MemoryRefreshKind::nothing().with_ram();
            let storage = DiskRefreshKind::nothing().with_storage();
            // the cpu's share is the change since the last refresh, so the first one only primes
            system.refresh_cpu_usage();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                system.refresh_cpu_usage();
                system.refresh_memory_specifics(memory);
                let (disk_free, disk_total) = disks.as_mut().zip(disk).map_or((0, 0), |(disks, i)| {
                    let d = &mut disks[i];
                    d.refresh_specifics(storage);
                    (d.available_space(), d.total_space())
                });
                let reading = Reading {
                    cpu: system.global_cpu_usage() / 100.0,
                    memory_used: system.used_memory(),
                    memory_total: system.total_memory(),
                    disk_free,
                    disk_total,
                };
                if tx.send(reading).is_err() {
                    break;
                }
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("the gauges thread does not start: {e:#}");
        }
        Self { readings, latest: Reading::default(), cpu: Ring::default(), memory: Ring::default(), disk: Ring::default() }
    }

    /// Takes the readings the thread left since the last frame: one a second, none most frames.
    fn tick(&mut self) {
        while let Ok(reading) = self.readings.try_recv() {
            self.latest = reading;
            self.cpu.push(reading.cpu);
            self.memory.push(ratio(reading.memory_used, reading.memory_total));
            self.disk.push(ratio(reading.disk_total.saturating_sub(reading.disk_free), reading.disk_total));
        }
    }

    /// The row of the three gauges across `page`: equal widths down to a minimum.
    fn row(&self, ui: &mut egui::Ui, page: f32) {
        let r = self.latest;
        let width = ((page - 20.0) / 3.0).max(180.0);
        let cpu = format!("{:.0} %", r.cpu * 100.0);
        let memory = format!("{} / {} GB", gib(r.memory_used), gib(r.memory_total));
        let disk = format!("{} of {} GB free", gib(r.disk_free), gib(r.disk_total));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            cards::gauge(ui, "CPU", &cpu, &self.cpu.samples(), theme::lane(1), width);
            cards::gauge(ui, "MEMORY", &memory, &self.memory.samples(), theme::lane(2), width);
            cards::gauge(ui, "DISK", &disk, &self.disk.samples(), theme::lane(3), width);
        });
    }
}

/// How many runs the token graphs span: the last 60 across the open projects.
const RUNS_SHOWN: usize = 60;

/// One token gauge as the row draws it: the badge's text and the graph's samples, 0 to 1,
/// the oldest first.
#[derive(Default, Debug, PartialEq)]
struct TokenGauge {
    value: String,
    samples: Vec<f32>,
}

/// The token gauges of the dashboard, from the ledgers of the open projects: the tokens a
/// card run takes this session, the session's total, and every run the ledgers hold. The
/// ledgers are read on the tick when a run ended somewhere, never per frame; the row draws
/// what the read left.
#[derive(Default)]
struct TokenGauges {
    /// What the ledgers were last read for: the tabs open and the runs their logs saw end;
    /// `None` before the first read.
    read_for: Option<(usize, u64)>,
    task: TokenGauge,
    session: TokenGauge,
    all: TokenGauge,
}

impl TokenGauges {
    /// Reads the ledgers again when a run ended, or a tab came or went, since the last read.
    fn tick(&mut self, tabs: &[Workspace]) {
        let key = (tabs.len(), tabs.iter().map(|t| t.announced).sum::<u64>());
        if self.read_for == Some(key) {
            return;
        }
        let (mut session, mut all) = (Vec::new(), Vec::new());
        let (mut session_totals, mut all_totals) = (Totals::default(), Totals::default());
        let shown = u32::try_from(RUNS_SHOWN).unwrap_or(u32::MAX);
        for tab in tabs {
            let Some(usage) = &tab.usage else { continue };
            let read = tab.rt.block_on(async {
                anyhow::Ok((
                    usage.recent(tab.session_from, shown).await?,
                    usage.totals_after(tab.session_from).await?,
                    usage.recent(0, shown).await?,
                    usage.totals().await?,
                ))
            });
            match read {
                Ok((runs, totals, every, every_totals)) => {
                    session.extend(runs);
                    session_totals.merge(&totals);
                    all.extend(every);
                    all_totals.merge(&every_totals);
                }
                Err(e) => tracing::warn!("the ledger of {} does not read: {e:#}", tab.root.display()),
            }
        }
        *self = token_gauges(session, &session_totals, all, &all_totals);
        self.read_for = Some(key);
    }

    /// The row of the three token gauges across `page`, the same widths as the machine's.
    fn row(&self, ui: &mut egui::Ui, page: f32) {
        let width = ((page - 20.0) / 3.0).max(180.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            cards::gauge(ui, "TOKENS / TASK", &self.task.value, &self.task.samples, theme::lane(4), width);
            cards::gauge(ui, "SESSION", &self.session.value, &self.session.samples, theme::lane(5), width);
            cards::gauge(ui, "ALL TIME", &self.all.value, &self.all.samples, theme::lane(6), width);
        });
    }
}

/// The token gauges from what the ledgers answered: `session` the rows since the window
/// started and `all` the newest rows of each project, both merged by time and cut to the last
/// [`RUNS_SHOWN`]; the totals cover every row of their span, not only the ones shown.
fn token_gauges(
    mut session: Vec<ra_usage::Spend>,
    session_totals: &Totals,
    mut all: Vec<ra_usage::Spend>,
    all_totals: &Totals,
) -> TokenGauges {
    for runs in [&mut session, &mut all] {
        runs.sort_by(|a, b| a.at.cmp(&b.at));
        let extra = runs.len().saturating_sub(RUNS_SHOWN);
        runs.drain(..extra);
    }
    let average = |totals: &Totals| if totals.calls == 0 { 0 } else { totals.tokens() / totals.calls };
    let per_run = |runs: &[ra_usage::Spend]| {
        let top = runs.iter().map(|s| s.tokens).max().unwrap_or(0);
        runs.iter().map(|s| ratio(s.tokens, top)).collect::<Vec<_>>()
    };
    let task = TokenGauge {
        value: match session_totals.calls {
            0 => "no runs yet".to_owned(),
            1 => format!("{} · 1 run", tokens_text(average(session_totals))),
            n => format!("{} avg · {n} runs", tokens_text(average(session_totals))),
        },
        samples: per_run(&session),
    };
    // the running total climbs to the session's total at the right; the runs before the ones
    // shown are the floor it starts from
    let total = session_totals.tokens();
    let mut running = total - session.iter().map(|s| s.tokens).sum::<u64>().min(total);
    let climb = session
        .iter()
        .map(|s| {
            running += s.tokens;
            ratio(running, total)
        })
        .collect();
    let session = TokenGauge { value: format!("{} · {}", tokens_text(total), cents(session_totals.cost_usd)), samples: climb };
    let all =
        TokenGauge { value: format!("{} avg · {}", tokens_text(average(all_totals)), cents(all_totals.cost_usd)), samples: per_run(&all) };
    TokenGauges { read_for: None, task, session, all }
}

/// A token count as the gauges read it: `842`, `1.2k`, `128k`, `1.2M`.
#[allow(clippy::cast_precision_loss)]
fn tokens_text(count: u64) -> String {
    let (value, unit) = match count {
        0..=999 => return count.to_string(),
        1_000..=999_999 => (count as f64 / 1_000.0, "k"),
        _ => (count as f64 / 1_000_000.0, "M"),
    };
    // one decimal while it fits in three characters, none past ten, so 9 960 reads 10k
    if value < 9.95 { format!("{value:.1}{unit}") } else { format!("{value:.0}{unit}") }
}

/// A cost as the gauges read it, cents without the dollar sign: `.34`, `12.40`; `—` when no
/// run reported one.
fn cents(cost: Option<f64>) -> String {
    match cost {
        None => "—".to_owned(),
        Some(c) => {
            let text = format!("{c:.2}");
            match text.strip_prefix('0') {
                Some(rest) => rest.to_owned(),
                None => text,
            }
        }
    }
}

/// `part` of `whole` as 0 to 1; nothing of nothing is 0.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn ratio(part: u64, whole: u64) -> f32 {
    if whole == 0 { 0.0 } else { (part as f64 / whole as f64) as f32 }
}

/// Bytes as gibibytes with one decimal, the unit Task Manager calls GB.
#[allow(clippy::cast_precision_loss)]
fn gib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

impl Drop for App {
    fn drop(&mut self) {
        // the tabs still open hear the window go; a tab closed earlier heard it then
        for tab in &self.tabs {
            ra_console::pair::window_down(&tab.root, self.down);
        }
    }
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, lock: ra_console::hub::Lock, projects: &[PathBuf], front: Option<&Path>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        fonts::install(&cc.egui_ctx);
        // the theme of the first project's last session comes back before the first frame
        let layout = WindowLayout::load(&projects[0]);
        theme::set(theme::by_name(&layout.theme).unwrap_or(theme::BLOOD_MOON));
        cc.egui_ctx.all_styles_mut(|style| {
            style.visuals = theme::visuals();
            style.text_styles.insert(TextStyle::Body, FontId::proportional(14.5));
            style.text_styles.insert(TextStyle::Button, FontId::proportional(13.5));
            style.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
            style.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
            style.text_styles.insert(TextStyle::Heading, FontId::proportional(22.0));
            style.spacing.item_spacing = egui::vec2(8.0, 5.0);
            style.spacing.button_padding = egui::vec2(10.0, 4.0);
            style.spacing.tooltip_width = 500.0;
            // a tooltip comes at once, moving or not, and stays a moment between neighbouring
            // rows; the default waits half a second after the pointer stops
            style.interaction.tooltip_delay = 0.08;
            style.interaction.tooltip_grace_time = 0.35;
            style.interaction.show_tooltips_only_when_still = false;
            style.visuals.popup_shadow =
                egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(160) };
        });
        // the window's log, under the hub: the workspaces' progress comes from their own files
        if let Ok(log) = std::fs::OpenOptions::new().create(true).append(true).open(ra_console::hub::dir().join("gui.log")) {
            ra_console::trace::init(log, None, None);
        }
        let mut app = Self {
            lock,
            tabs: Vec::new(),
            active: 0,
            mailbox_read: Instant::now(),
            adding: None,
            title: String::new(),
            dashboard: false,
            started: Instant::now(),
            report: false,
            landed: Vec::new(),
            landed_from: Vec::new(),
            down: "close",
            gauges: Gauges::start(&projects[0]),
            tokens: TokenGauges::default(),
        };
        for project in projects {
            app.open(&cc.egui_ctx, project);
        }
        // every project's feed hears the window is up, once the tab count is known
        for tab in &app.tabs {
            ra_console::pair::window_up(&tab.root, ra_console::VERSION, app.tabs.len());
        }
        // the project asked for is in front; without one, many projects open on all of them
        match front.and_then(|f| app.tabs.iter().position(|t| t.root == f)) {
            Some(at) => app.active = at,
            None => app.dashboard = app.tabs.len() > 1,
        }
        app
    }

    /// Opens a project as a tab, or brings its tab to the front when it is open already. Says
    /// whether the tab is new.
    fn open(&mut self, ctx: &egui::Context, project: &Path) -> bool {
        let project = plain_path(project);
        if let Some(at) = self.tabs.iter().position(|t| t.root == project) {
            self.active = at;
            return false;
        }
        self.tabs.push(Workspace::new(ctx, project));
        self.active = self.tabs.len() - 1;
        ra_console::hub::remember(&self.tabs.iter().map(|t| t.root.clone()).collect::<Vec<_>>());
        true
    }

    /// Closes a tab: its workspace releases its project as it drops. A tab the person closes
    /// (`forget`) stays closed next session; one that leaves because its project was asked to
    /// quit, or because the window restarts, comes back.
    fn close(&mut self, at: usize, forget: bool) {
        if at < self.tabs.len() {
            ra_console::pair::window_down(&self.tabs[at].root, if forget { "tab closed" } else { "quit" });
            self.tabs.remove(at);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            if forget {
                ra_console::hub::remember(&self.tabs.iter().map(|t| t.root.clone()).collect::<Vec<_>>());
            }
        }
    }

    /// The tab strip: a tab per project in the Chrome manner, the active one joined to the
    /// bar below it, a dot that burns while the project has a run going, a close on each, and
    /// a `+` that takes a path. Returns what the pointer asked.
    fn strip(&mut self, ui: &mut egui::Ui) {
        let mut switch: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut open: Option<PathBuf> = None;
        let mut active_rect: Option<(Rect, Color32)> = None;
        let mut home = false;
        let mut report = false;
        let strip = ui.horizontal(|ui| {
            // the strip is buttons, not text: the pointer is a hand over it, never a caret
            ui.style_mut().interaction.selectable_labels = false;
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(4.0);
            // the window's own tabs first: the dashboard, every project at a glance, and the
            // report, what landed across them
            let dashboard_tab = house_tab(ui, "⊞", "dashboard", self.dashboard, "every project: the card each is on and its latest run");
            if dashboard_tab.clicked {
                home = true;
            }
            if let Some(rect) = dashboard_tab.active {
                active_rect = Some(rect);
            }
            let report_tab = house_tab(ui, "≣", "report", self.report, "every project: the cards that landed, newest first");
            if report_tab.clicked {
                report = true;
            }
            if let Some(rect) = report_tab.active {
                active_rect = Some(rect);
            }
            ui.add_space(6.0);
            for (i, tab) in self.tabs.iter().enumerate() {
                let active = i == self.active && !self.dashboard && !self.report;
                let live = tab.sink.live_run().is_some() || tab.job.is_some();
                let lane = theme::lane(1 + i % 9);
                let name = tab
                    .project_title
                    .clone()
                    .unwrap_or_else(|| tab.root.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()));
                let fill = if active { theme::panel() } else { theme::mix(theme::bg(), theme::panel(), 0.5) };
                let frame = Frame::new()
                    .fill(fill)
                    .stroke(Stroke::new(1.0, if active { cards::alpha(lane, 200) } else { cards::alpha(theme::rule(), 180) }))
                    .corner_radius(CornerRadius { nw: 9, ne: 9, sw: 0, se: 0 })
                    .inner_margin(Margin { left: 12, right: 8, top: 6, bottom: 6 });
                let painter = ui.painter().clone();
                let slot = painter.add(egui::Shape::Noop);
                let shown = frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        // the dot: the lane at rest, the warm color breathing while a run goes
                        let dot = if live {
                            let t = ui.input(|i| i.time);
                            #[allow(clippy::cast_possible_truncation)]
                            let pulse = ((t * 2.4).sin() * 0.5 + 0.5) as f32;
                            ui.ctx().request_repaint_after(Duration::from_millis(40));
                            theme::mix(lane, theme::warn_text(), pulse)
                        } else if active {
                            lane
                        } else {
                            cards::alpha(lane, 120)
                        };
                        let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot_rect.center(), 4.0, dot);
                        if live {
                            ui.painter().circle_stroke(dot_rect.center(), 6.0, Stroke::new(1.0, cards::alpha(dot, 120)));
                        }
                        let text = RichText::new(&name).color(if active { theme::text() } else { theme::dim() });
                        ui.label(if active { text.strong() } else { text });
                        let x = ui.add(egui::Label::new(RichText::new("×").color(theme::dim()).small()).sense(egui::Sense::click()));
                        if x.on_hover_text("close this project's tab").clicked() {
                            close = Some(i);
                        }
                    });
                });
                let rect = shown.response.rect;
                if active {
                    // the sheen of the active tab, and its light along the top edge
                    painter.set(
                        slot,
                        cards::fade_shape(Rect::from_min_size(rect.min, egui::vec2(rect.width(), rect.height() * 0.5)), lane, 36, false),
                    );
                    painter.rect_filled(
                        Rect::from_min_size(rect.left_top(), egui::vec2(rect.width(), 2.0)),
                        CornerRadius::same(1),
                        cards::alpha(lane, 220),
                    );
                }
                let response = ui.interact(rect, ui.make_persistent_id(("tab", &tab.root)), egui::Sense::click());
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tab.root.display().to_string());
                if response.clicked() {
                    switch = Some(i);
                }
                if response.middle_clicked() {
                    close = Some(i);
                }
            }
            // the plus: a path to open as a tab
            ui.add_space(6.0);
            match &mut self.adding {
                None => {
                    if ui
                        .add(egui::Label::new(RichText::new("+").size(18.0).color(theme::dim())).sense(egui::Sense::click()))
                        .on_hover_text("open a project")
                        .clicked()
                    {
                        self.adding = Some(String::new());
                    }
                }
                Some(path) => {
                    let edit = ui.add(
                        egui::TextEdit::singleline(path)
                            .hint_text(RichText::new("a project directory · Enter opens it, Escape closes").color(theme::dim()).italics())
                            .desired_width(360.0)
                            .font(TextStyle::Monospace),
                    );
                    edit.request_focus();
                    let (enter, escape) = ui.input_mut(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                    if enter && !path.trim().is_empty() {
                        open = Some(PathBuf::from(path.trim()));
                        self.adding = None;
                    } else if escape {
                        self.adding = None;
                    }
                }
            }
        });
        // the strip's floor: a rule the bar below sits on, open under the active tab so the
        // tab and the bar read as one surface, the Chrome way
        let floor = strip.response.rect.bottom();
        let span = ui.max_rect().x_range();
        ui.painter().hline(span, floor, Stroke::new(1.0, cards::alpha(theme::rule(), 200)));
        if let Some((rect, fill)) = active_rect {
            ui.painter().rect_filled(
                Rect::from_min_max(egui::pos2(rect.left() + 1.0, floor - 1.5), egui::pos2(rect.right() - 1.0, floor + 1.0)),
                CornerRadius::ZERO,
                fill,
            );
        }
        let ctx = ui.ctx().clone();
        if home {
            self.dashboard = true;
            self.report = false;
        }
        if report {
            self.report = true;
            self.dashboard = false;
        }
        if let Some(i) = switch {
            self.active = i;
            self.dashboard = false;
            self.report = false;
        }
        if let Some(i) = close {
            self.close(i, true);
        }
        if let Some(path) = open
            && path.is_dir()
        {
            self.open(&ctx, &path);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        // the mailbox: projects other starts asked for, twice a second; and the request to
        // quit whole (`rc restart`), which keeps the tabs for the next start
        if self.mailbox_read.elapsed() >= Duration::from_millis(500) {
            self.mailbox_read = Instant::now();
            for project in ra_console::hub::take_requests() {
                if self.open(&ctx, &project) {
                    ra_console::pair::window_up(&plain_path(&project), ra_console::VERSION, self.tabs.len());
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            if ra_console::hub::quit_requested() {
                self.down = "restart";
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        // the machine's readings land whichever view is up, so the dashboard opens with history
        self.gauges.tick();
        // the projects not in front keep up with their runs and their files; on the dashboard
        // and the report every one is in front
        let dashboard = self.dashboard || self.report;
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if i != self.active || dashboard {
                tab.tick(&ctx);
            }
        }
        // the ledgers, only when a run ended in some project since the last read
        self.tokens.tick(&self.tabs);
        // a workspace asked to quit closes its tab; the last one closes the window
        if let Some(at) = self.tabs.iter().position(|t| t.closing) {
            self.close(at, false);
            if self.tabs.is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        // Ctrl+Tab and Ctrl+Shift+Tab walk the tabs, Ctrl+1..9 pick one
        let (next, prev, pick) = ctx.input_mut(|i| {
            let next = i.consume_key(Modifiers::COMMAND, Key::Tab);
            let prev = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab);
            let keys = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
            let pick = keys.iter().position(|k| i.consume_key(Modifiers::COMMAND, *k));
            (next, prev, pick)
        });
        if !self.tabs.is_empty() {
            if next {
                self.active = (self.active + 1) % self.tabs.len();
            }
            if prev {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            }
            if let Some(n) = pick.filter(|n| *n < self.tabs.len()) {
                self.active = n;
            }
        }
        egui::Panel::top("tabs")
            .frame(Frame::new().fill(theme::bg()).inner_margin(Margin { left: 6, right: 6, top: 6, bottom: 0 }))
            .show(root, |ui| self.strip(ui));
        if self.tabs.is_empty() {
            egui::CentralPanel::default().frame(Frame::new().fill(theme::bg())).show(root, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.4);
                    ui.label(RichText::new("no project open").color(theme::dim()).size(18.0));
                    ui.label(
                        RichText::new("the + above takes a directory, or `rc open <dir>` from a terminal").color(theme::dim()).small(),
                    );
                });
            });
            return;
        }
        let active = self.active.min(self.tabs.len() - 1);
        self.active = active;
        let title = if self.dashboard {
            format!("Dashboard — Rhipper Agent {}", ra_console::VERSION)
        } else if self.report {
            format!("Report — Rhipper Agent {}", ra_console::VERSION)
        } else {
            window_title(self.tabs[active].project_title.as_deref(), &self.tabs[active].root)
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if self.dashboard {
            self.dashboard_view(root);
        } else if self.report {
            self.report_view(root);
        } else {
            self.tabs[active].draw(root);
        }
    }
}

impl App {
    /// The dashboard's status bar: the thin bar a workspace wears, over every project at once.
    /// The model, debug and the local dot are the active project's (the same for all in
    /// practice; debug toggles every tab); the session's tokens and the ledger are summed over
    /// the open projects; the week pill is the plan's, read from the active project; the uptime
    /// is the window's.
    fn dashboard_status(&mut self, ui: &mut egui::Ui) {
        let active = self.active.min(self.tabs.len().saturating_sub(1));
        ui.horizontal(|ui| {
            ui.label(RichText::new(" ✦ Rhipper").color(theme::accent()).strong().size(15.0));
            ui.label(RichText::new("Agent").strong().size(15.0));
            ui.label(RichText::new(ra_console::VERSION).color(theme::dim()).small());
            ui.add_space(10.0);
            let Some(tab) = self.tabs.get(active) else { return };
            cards::badge_marked(ui, "◉", &tab.worker.model, theme::accent())
                .on_hover_text("the model of the next run in the active project");
            let debug = tab.worker.debug;
            let (mark, color, hint) = if debug {
                ("⚙", theme::debug(), "debug mode in every project: a run ends with what would have made it faster · click to turn off")
            } else {
                ("○", theme::dim(), "debug mode off in every project · click to turn on")
            };
            let toggle = cards::badge_marked(ui, mark, "debug", color).interact(egui::Sense::click()).on_hover_text(hint).clicked();
            let url = ra_providers::openrouter::local_url();
            let (mark, color, hint) = match tab.local.up() {
                Some(true) => ("●", theme::online(), format!("the local server at {url} answers; probed once a minute")),
                Some(false) => ("●", theme::error_text(), format!("no local server at {url}; set RHIPPER_LOCAL_URL to move it")),
                None => ("○", theme::dim(), format!("probing the local server at {url}")),
            };
            cards::badge_marked(ui, mark, "local", color).on_hover_text(hint);
            let live = self.tabs.iter().filter(|t| t.sink.live_run().is_some()).count();
            let projects = self.tabs.len();
            cards::badge_marked(ui, "◇", &format!("{projects} projects · {live} running"), theme::trace());
            // the counts over every open project
            ui.add_space(18.0);
            let (mut prompt, mut completion, mut cost) = (0u64, 0u64, 0f64);
            let (mut all_cost, mut all_calls, mut any_ledger) = (0f64, 0u64, false);
            for t in &self.tabs {
                let session = t.worker.tally();
                prompt += session.prompt_tokens;
                completion += session.completion_tokens;
                cost += session.cost_usd.unwrap_or(0.0);
                if let Some(all) = &t.ledger.all_time {
                    all_cost += all.cost_usd.unwrap_or(0.0);
                    all_calls += all.calls;
                    any_ledger = true;
                }
            }
            cards::badge_marked(
                ui,
                "⇅",
                &format!("in {} · out {} · {}", compact(prompt), compact(completion), dollars(Some(cost))),
                theme::warn_text(),
            )
            .on_hover_text("this session's tokens and cost, every project");
            if any_ledger {
                cards::badge_marked(ui, "◉", &format!("ledger {} · {all_calls} calls", dollars(Some(all_cost))), theme::online())
                    .on_hover_text("all time, every project's var/usage.db");
            } else {
                cards::badge_marked(ui, "◌", "no ledger", theme::dim());
            }
            // the plan's week, from the active project's limits file
            let tab = &self.tabs[active];
            if tab.limits.fresh()
                && let Some(week) = tab.limits.windows.get("seven_day")
            {
                let target = tab.week_target;
                let shown = ui.ctx().animate_value_with_time(egui::Id::unique("dashboard-week-pill"), target, 0.9);
                let fade = tab.week_moved.map_or(0.0, |t| (1.0 - t.elapsed().as_secs_f32() / 1.6).clamp(0.0, 1.0));
                if fade > 0.0 {
                    ui.ctx().request_repaint();
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let pulse = (40.0 + 190.0 * fade) as u8;
                let color = if target >= 0.8 {
                    theme::error_text()
                } else if target >= 0.5 {
                    theme::warn_text()
                } else {
                    theme::online()
                };
                let (used, resets_at, threshold) = (week.utilization, week.resets_at, tab.threshold);
                cards::pill_bar(ui, &format!("week {:.0}%/{:.0}%", shown * 100.0, threshold * 100.0), shown, color, pulse).on_hover_ui(
                    |ui| {
                        ui.label(format!(
                            "the plan's week: {:.0}% used, resets in {}",
                            used * 100.0,
                            ra_providers::limits::until(resets_at)
                        ));
                        ui.label(format!("claude-* runs steer at {:.0}%: local when up, else refused without --force", threshold * 100.0));
                        cards::sparkline(ui, &tab.week.days, color);
                    },
                );
            }
            let secs = self.started.elapsed().as_secs();
            ui.label(RichText::new(format!("up {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)).color(theme::dim()).small());
            if toggle {
                for t in &mut self.tabs {
                    t.worker.debug = !debug;
                }
            }
        });
    }

    /// The dashboard: one card per project, top to bottom, as many as the window has room for
    /// and a scroll for the rest. Each card is the project's name and state, the card a run is
    /// on (else the next ready ones) in the roadmap's own rows, and its latest run.
    fn dashboard_view(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        let mut open: Option<usize> = None;
        egui::Panel::top("dashboard-status")
            .frame(Frame::new().fill(theme::panel()).inner_margin(Margin::symmetric(8, 6)))
            .show(root, |ui| self.dashboard_status(ui));
        egui::CentralPanel::default().frame(Frame::new().fill(theme::bg()).inner_margin(Margin::symmetric(18, 12))).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("DASHBOARD").small().strong().color(theme::dim()));
                let live = self.tabs.iter().filter(|t| t.sink.live_run().is_some()).count();
                let text = match live {
                    0 => format!("{} open · nothing running", self.tabs.len()),
                    1 => format!("{} open · 1 running", self.tabs.len()),
                    n => format!("{} open · {n} running", self.tabs.len()),
                };
                ui.label(RichText::new(text).small().color(theme::dim()));
            });
            ui.add_space(6.0);
            // the page's width is the window's, not what the panel reports: the panel's ui
            // answers with far more than the window shows, and everything laid out to it
            // ran off the right edge. The margins and the scroll bar come off the content rect.
            let page = (ctx.content_rect().width() - 36.0 - 14.0).min(ui.available_width()).max(400.0);
            self.gauges.row(ui, page);
            ui.add_space(10.0);
            self.tokens.row(ui, page);
            ui.add_space(6.0);
            // the height left under the gauges, shared out among the project cards: a card
            // whose queue and run would leave it empty grows into its share, so a tall window
            // shows more of the queue and more of the run, not a blank band under the cards.
            // The gauges above keep their size. Measured from the window, as the width is
            let count = self.tabs.len().max(1) as f32;
            let left = ctx.content_rect().bottom() - ui.next_widget_position().y - 12.0 - 4.0;
            let budget = ((left - 10.0 * (count - 1.0)) / count).max(0.0);
            egui::ScrollArea::vertical().id_salt("dashboard").auto_shrink([false, false]).show(ui, |ui| {
                // the page's width, handed down: a card holds itself to it, so nothing in it
                // takes the room a scroll area's content would otherwise be allowed
                ui.set_width(page);
                ui.set_max_width(page);
                for i in 0..self.tabs.len() {
                    let lane = theme::lane(1 + i % 9);
                    if self.tabs[i].dashboard_card(ui, lane, page, budget) {
                        open = Some(i);
                    }
                    ui.add_space(10.0);
                }
            });
        });
        if let Some(i) = open {
            self.active = i;
            self.dashboard = false;
        }
        let live = self.tabs.iter().any(|t| t.sink.live_run().is_some() || t.job.is_some());
        ctx.request_repaint_after(Duration::from_millis(if live { 40 } else { 250 }));
    }

    /// The report: the cards that landed in every open project, as bubbles in two columns,
    /// newest first; grouped under a divider per project when the toggle says so. A click on
    /// a bubble opens the project's tab with the card selected.
    fn report_view(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        self.rebuild_landed();
        let mut by_project = self.tabs.first().is_some_and(|t| t.report_by_project);
        let mut open: Option<(usize, i64)> = None;
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        egui::CentralPanel::default().frame(Frame::new().fill(theme::bg()).inner_margin(Margin::symmetric(18, 12))).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("REPORT").small().strong().color(theme::dim()));
                let text = format!("{} landed across {} open", self.landed.len(), self.tabs.len());
                ui.label(RichText::new(text).small().color(theme::dim()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if cards::tab(ui, "by time", theme::accent(), !by_project) {
                        by_project = false;
                    }
                    if cards::tab(ui, "by project", theme::accent(), by_project) {
                        by_project = true;
                    }
                });
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().id_salt("report").auto_shrink([false, false]).show(ui, |ui| {
                // the page's width, handed down: the columns are sized from it, so no bubble
                // takes the room a scroll area's content would otherwise be allowed
                let page = ui.available_width();
                ui.set_width(page);
                let column = ((page - 10.0) / 2.0).max(200.0);
                if self.landed.is_empty() {
                    ui.label(RichText::new("nothing has landed yet: a card implemented or accepted shows here").color(theme::dim()));
                }
                let mut rows: Vec<&[Landed]> = Vec::new();
                if by_project {
                    for group in self.landed.chunk_by(|a, b| a.tab == b.tab) {
                        rows.push(group);
                    }
                } else {
                    rows.push(&self.landed);
                }
                for (g, group) in rows.into_iter().enumerate() {
                    if by_project {
                        let lane = theme::lane(1 + group[0].tab % 9);
                        if g > 0 {
                            ui.add_space(4.0);
                        }
                        ui.horizontal(|ui| {
                            cards::badge(ui, &group[0].project, lane);
                            ui.label(RichText::new(format!("{} landed", group.len())).small().color(theme::dim()));
                        });
                        cards::divider(ui, lane);
                    }
                    for pair in group.chunks(2) {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            for item in pair {
                                ui.allocate_ui_with_layout(egui::vec2(column, 0.0), Layout::top_down(Align::Min), |ui| {
                                    ui.set_width(column);
                                    if landed_bubble(ui, item, now) {
                                        open = Some((item.tab, item.id));
                                    }
                                });
                            }
                        });
                    }
                }
            });
        });
        if self.tabs.first().is_some_and(|t| t.report_by_project != by_project) {
            // every project's layout keeps the choice, so whichever opens first reads it back
            for tab in &mut self.tabs {
                tab.report_by_project = by_project;
                tab.save_layout();
            }
            self.landed_from.clear();
        }
        if let Some((tab, id)) = open
            && tab < self.tabs.len()
        {
            self.active = tab;
            self.report = false;
            self.tabs[tab].select(Some(id));
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Rebuilds the report's list when a tab's roadmap or ledger moved since the last build,
    /// or a tab came or went; a frame that finds them where they were draws the list it has.
    fn rebuild_landed(&mut self) {
        let from: Vec<_> = self.tabs.iter().map(|t| (t.roadmap_seen, t.ledger.read_at)).collect();
        if from == self.landed_from {
            return;
        }
        self.landed_from = from;
        let mut landed = Vec::new();
        for (tab, workspace) in self.tabs.iter().enumerate() {
            let project = workspace
                .project_title
                .clone()
                .unwrap_or_else(|| workspace.root.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()));
            let cards = workspace
                .roadmap
                .iter()
                .enumerate()
                .filter(|(_, n)| n.kind == NodeKind::Task && matches!(n.state, TaskState::Implemented | TaskState::Accepted));
            for (seq, node) in cards {
                landed.push(Landed {
                    tab,
                    project: project.clone(),
                    id: node.id,
                    title: node.title.clone(),
                    state: node.state,
                    model: node.assignee.as_ref().map(|a| short(&a.model).to_owned()).unwrap_or_default(),
                    cost: workspace.costs.get(&node.id).filter(|t| t.calls > 0).and_then(|t| t.cost_usd),
                    tokens_per_turn: workspace.costs.get(&node.id).map_or(0, |t| t.tokens().checked_div(t.turns).unwrap_or(0)),
                    at: workspace.last_changes.get(&node.id).map(|c| c.at),
                    seq,
                });
            }
        }
        let by_project = self.tabs.first().is_some_and(|t| t.report_by_project);
        order_landed(&mut landed, by_project);
        self.landed = landed;
    }
}

/// One bubble of the report: a card that landed, with what the bubble shows of it.
#[derive(Clone, Debug, PartialEq)]
struct Landed {
    /// The index of the project's tab.
    tab: usize,
    project: String,
    id: i64,
    title: String,
    state: TaskState,
    /// The model's short name; empty when the card names none.
    model: String,
    /// The card's cost from the ledger, when it had a call.
    cost: Option<f64>,
    /// The tokens one turn took on average over the card's runs; 0 before the first turn.
    tokens_per_turn: u64,
    /// When its state last changed, from the history; `None` when the project has none.
    at: Option<u64>,
    /// The card's place in the store, the order a project without history falls back to.
    seq: usize,
}

/// Orders the report's bubbles: newest first by the time of the last state change, the cards
/// without one after them in their store's order; `by_project` keeps that order within each
/// project and lays the projects out in the tabs' order.
fn order_landed(landed: &mut [Landed], by_project: bool) {
    landed.sort_by_key(|l| (l.at.is_none(), std::cmp::Reverse(l.at), l.tab, l.seq));
    if by_project {
        landed.sort_by_key(|l| l.tab);
    }
}

/// The bubble of one landed card: the project as a small badge, the id and the title, the
/// state, the model, the cost, the tokens a turn and how long ago it landed. Returns true when
/// clicked.
fn landed_bubble(ui: &mut egui::Ui, item: &Landed, now: u64) -> bool {
    let lane = theme::lane(1 + item.tab % 9);
    let shown = cards::bubble(ui, lane, false, |ui| {
        let inner = ui.available_width();
        ui.set_width(inner);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            cards::badge(ui, &item.project, lane);
            ui.label(RichText::new(format!("#{}", item.id)).small().strong().color(cards::alpha(theme::text(), 180)));
            let title = egui::Label::new(RichText::new(&item.title).color(theme::text())).truncate();
            ui.add(title).on_hover_text(format!("#{} {}", item.id, item.title));
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            cards::badge(ui, &item.state.to_string(), theme::state_color(item.state));
            if !item.model.is_empty() {
                cards::badge(ui, &item.model, theme::accent());
            }
            if let Some(cost) = item.cost {
                ui.label(RichText::new(dollars(Some(cost))).small().color(theme::dim()));
            }
            if item.tokens_per_turn > 0 {
                let text = format!("{} tok/turn", compact(item.tokens_per_turn));
                ui.label(RichText::new(text).small().color(theme::dim()))
                    .on_hover_text("the tokens a turn read and wrote, on average over the card's runs");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let when = item.at.map_or_else(|| "—".to_owned(), |at| format!("{} ago", ra_roadmap::history::age(now.saturating_sub(at))));
                ui.label(RichText::new(when).small().color(theme::dim()));
            });
        });
    });
    ui.interact(shown.response.rect, ui.make_persistent_id(("landed", item.tab, item.id)), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// What one of the window's own tabs in the strip answered: whether it was clicked, and its
/// rect and fill when it is the one in front, for the floor to open under.
struct HouseTab {
    clicked: bool,
    active: Option<(Rect, Color32)>,
}

/// One of the window's own tabs in the strip, the dashboard's or the report's: a mark and a
/// name in the accent, drawn the way a project's tab is.
fn house_tab(ui: &mut egui::Ui, mark: &str, name: &str, on: bool, hover: &str) -> HouseTab {
    let lane = theme::accent();
    let fill = if on { theme::panel() } else { theme::mix(theme::bg(), theme::panel(), 0.5) };
    let frame = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, if on { cards::alpha(lane, 200) } else { cards::alpha(theme::rule(), 180) }))
        .corner_radius(CornerRadius { nw: 9, ne: 9, sw: 0, se: 0 })
        .inner_margin(Margin { left: 12, right: 12, top: 6, bottom: 6 });
    let shown = frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(mark).size(15.0).color(if on { lane } else { theme::dim() }));
            let text = RichText::new(name).color(if on { theme::text() } else { theme::dim() });
            ui.label(if on { text.strong() } else { text });
        });
    });
    let rect = shown.response.rect;
    if on {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), egui::vec2(rect.width(), 2.0)),
            CornerRadius::same(1),
            cards::alpha(lane, 220),
        );
    }
    let response =
        ui.interact(rect, ui.make_persistent_id(("tab-house", name)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    HouseTab { clicked: response.on_hover_text(hover).clicked(), active: on.then_some((rect, fill)) }
}

/// One project in the window: its store, its log, its panels. The window holds one per tab.
struct Workspace {
    rt: tokio::runtime::Runtime,
    store: Store,
    worker: Worker,
    sink: Sink,
    started: Instant,
    /// The roadmap as last read, its gambits, and the mtime it came from.
    roadmap: Vec<Node>,
    gambits: Vec<Gambit>,
    /// The one-line field under the gambit bubbles while `+` holds it open; Enter adds.
    new_gambit: Option<String>,
    /// Whether the done gambits are unfolded from under their dim header.
    done_gambits_open: bool,
    roadmap_seen: Option<SystemTime>,
    /// The last state change of every card, read with the roadmap; the tooltip shows it.
    last_changes: HashMap<i64, Change>,
    /// The width the roadmap's rows need, measured once per read of the file.
    roadmap_width: f32,
    /// Whether the cost column shows at the right of the rows; the chip above the tree toggles it.
    show_cost: bool,
    /// The cost of every node's subtree, summed from the ledger by card; the root is the total.
    costs: HashMap<i64, Totals>,
    /// The width the cost column needs, measured once per reload of the ledger or the roadmap;
    /// `None` until the column next draws.
    cost_width: Option<f32>,
    selected: Option<i64>,
    /// The edits of the card view, built when it shows a card it was not built for.
    draft: Option<Draft>,
    /// A pick made while the draft was unsaved, held until the confirm answers it.
    pending: Option<Pick>,
    /// The card being added from the form, which takes the card area while it is open.
    new_card: Option<NewCard>,
    /// The card last seen in progress; a new one takes the view, so a run started over MCP or
    /// from the command line shows here as it starts.
    followed: Option<i64>,
    /// The states whose tasks the roadmap column leaves out; the chips above it toggle them.
    hidden_states: HashSet<TaskState>,
    folded: HashSet<i64>,
    /// What the search box above the tree holds; a row stays while its title or id contains it.
    search: String,
    /// The rows the roadmap column drew last frame, top to bottom: what the arrow keys walk.
    visible_rows: Vec<i64>,
    /// A row the keyboard just picked, to scroll into view as it draws.
    scroll_to: Option<i64>,
    input: String,
    history: Vec<String>,
    browsing: Option<usize>,
    /// How far into the activity file the log has read.
    activity_offset: u64,
    /// The slow call in flight, named for the status row.
    job: Option<String>,
    job_tx: mpsc::UnboundedSender<()>,
    job_done: mpsc::UnboundedReceiver<()>,
    /// The numbers of the ledger tab and the status bar's ledger badge.
    ledger: Ledger,
    /// The project's ledger, the same the worker writes, for the dashboard's token gauges;
    /// `None` when it did not open.
    usage: Option<UsageStore>,
    /// The ledger's newest row when the workspace opened: the rows after it are this session's.
    session_from: i64,
    context: ContextView,
    diff: DiffView,
    /// The reviewer's verdict on the selected card's last run, read from its context when the
    /// card is selected and again when a run ends, not every frame.
    verdict: VerdictView,
    tab: Tab,
    /// The file open beside the card, once a name under FILES is clicked; `None` keeps the
    /// card at full width.
    code: Option<code::CodePane>,
    /// The plan's windows as the harness last reported them, and when they were last read.
    limits: ra_providers::limits::Limits,
    limits_read: Instant,
    /// The share of the week past which a claude-* run is steered, from the project's
    /// `rhipper.toml`; the week's pill shows it.
    threshold: f64,
    /// Whether the local server is up, probed once a minute on the runtime with no model call;
    /// the status bar shows it as a dot. `None` until the first probe went out.
    local: ra_providers::openrouter::LocalProbe,
    local_probed: Option<Instant>,
    /// The week's utilization day by day, from the limits log, for the pill's sparkline.
    week: ra_providers::limits::Week,
    /// The week's utilization the pill bar last aimed at, and when it last moved, for the glow.
    week_target: f32,
    week_moved: Option<Instant>,
    show_debug: bool,
    /// What went wrong at start, shown on the splash instead of the workspace.
    fault: Option<String>,
    /// The first lines of the last three commits, newest first, so the splash tells a relaunch
    /// from the last one; empty outside a git checkout.
    commits: Vec<String>,
    /// The choice a directory without a roadmap gets, until it has one.
    setup: Option<Setup>,
    /// The column as the tree, or as the order a walk would run the cards in.
    view: RoadmapView,
    /// The row whose tooltip was open last, and when: the shade over the column stays a moment
    /// after the pointer leaves it, so a pass over the rules does not flash it.
    shade: Option<(Rect, Instant)>,
    /// The row being dragged across the column, until the pointer lets go.
    drag: Option<i64>,
    /// The row that shakes because its drop was refused, and when the shake began.
    shake: Option<(i64, Instant)>,
    /// The runs that ended and have not been clicked or timed out, oldest first.
    toasts: Vec<Toast>,
    /// The newest run of the log a toast announced; the runs after it are still going.
    announced: u64,
    /// The console band's height as last drawn, or as the layout file had it before that.
    console_height: f32,
    /// The zoom over the monitor's own scale, as the keys set it or the layout file had it.
    zoom: f32,
    /// The layout as last written, so a frame that changed nothing writes nothing.
    layout_saved: WindowLayout,
    /// The report's toggle, as the layout file had it; the window keeps every tab's the same.
    report_by_project: bool,
    /// The command palette while Ctrl+K holds it open.
    palette: Option<input::Palette>,
    /// The project's directory: every file of the workspace is under it.
    root: PathBuf,
    /// The project's title from its roadmap, for the tab and the window's title.
    project_title: Option<String>,
    /// Set when the workspace was asked to quit (another instance claims the project, or
    /// `/quit`): the window closes the tab.
    closing: bool,
}

/// The two ways the roadmap column reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RoadmapView {
    Tree,
    Walk,
}

impl Workspace {
    /// A workspace on the project at `root`. The window's style and fonts are the window's,
    /// installed once; a workspace brings its own store, ledger, log and layout.
    fn new(ctx: &egui::Context, root: PathBuf) -> Self {
        let layout = WindowLayout::load(&root);
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build().expect("a tokio runtime");
        let sink = Sink::new();
        let _ = std::fs::create_dir_all(root.join("var"));
        let mut fault = None;
        // one instance per project at a time: a console or an older window on it gives way
        let replaced = ra_console::instance::stop_running(&root);
        if let Err(e) = ra_console::instance::claim(&root) {
            tracing::warn!("cannot claim the instance: {e:#}");
        }
        // the directory is the project; without a roadmap the workspace offers to make one
        // rather than writing a bare file on the quiet
        let mut setup = None;
        let temp_store =
            || rt.block_on(Store::open(&std::env::temp_dir().join("rhipper-agent-fault.md"), "Rhipper Agent")).expect("a temp store");
        let store = if ra_console::setup::exists(&root) {
            match rt.block_on(Store::open(&root.join(ra_console::setup::ROADMAP_FILE), "Rhipper Agent")) {
                Ok(store) => store.with_source(Source::Window),
                Err(e) => {
                    fault = Some(format!("{e:#}"));
                    // a store that cannot open still needs a value; the splash shows the fault
                    temp_store()
                }
            }
        } else {
            setup = Some(Setup::new(&root));
            temp_store()
        };
        let mut worker = Worker::new(ra_console::ui::DEFAULT_MODEL);
        let mut usage = None;
        match rt.block_on(UsageStore::open(&root.join("var/usage.db"))) {
            Ok(ledger) => {
                worker = worker.with_ledger(ledger.clone());
                usage = Some(ledger);
            }
            Err(e) => tracing::warn!("no ledger: {e:#}"),
        }
        let session_from = usage.as_ref().map_or(0, |u| rt.block_on(u.last_id()).unwrap_or(0));
        sink.push(
            Kind::Banner,
            format!(
                "Rhipper Agent {} — the workspace. /help lists the commands; a line without / is a note for the next runs.",
                ra_console::VERSION
            ),
        );
        if let Some(what) = replaced {
            sink.push(Kind::Log(tracing::Level::INFO), what);
        }
        let (job_tx, job_done) = mpsc::unbounded_channel();
        let mut app = Self {
            rt,
            store,
            worker,
            sink,
            started: Instant::now(),
            roadmap: Vec::new(),
            gambits: Vec::new(),
            new_gambit: None,
            done_gambits_open: false,
            roadmap_seen: None,
            last_changes: HashMap::new(),
            roadmap_width: 360.0,
            show_cost: false,
            costs: HashMap::new(),
            cost_width: None,
            selected: None,
            draft: None,
            pending: None,
            new_card: None,
            followed: None,
            hidden_states: layout.hidden.iter().copied().collect(),
            folded: layout.folded.iter().copied().collect(),
            search: String::new(),
            visible_rows: Vec::new(),
            scroll_to: None,
            input: String::new(),
            history: Vec::new(),
            browsing: None,
            // a run going when the tab opens is re-attached from its start, so it shows live
            activity_offset: ra_console::ui::attach_offset(&root.join(ra_console::ui::ACTIVITY_FILE)),
            job: None,
            job_tx,
            job_done,
            ledger: Ledger::default(),
            usage,
            session_from,
            context: ContextView::default(),
            diff: DiffView::default(),
            verdict: VerdictView::default(),
            tab: layout.tab.unwrap_or(Tab::Card),
            code: None,
            limits: ra_providers::limits::Limits::load(&root.join(ra_providers::claude_code::LIMITS_FILE)),
            limits_read: Instant::now(),
            threshold: ra_console::agent::plan_threshold(&root),
            local: ra_providers::openrouter::LocalProbe::default(),
            local_probed: None,
            week: ra_providers::limits::Week::load(&root.join(ra_providers::claude_code::LIMITS_LOG)),
            week_target: 0.0,
            week_moved: None,
            show_debug: false,
            fault,
            commits: last_commits(&root),
            setup,
            view: RoadmapView::Tree,
            shade: None,
            drag: None,
            shake: None,
            toasts: Vec::new(),
            announced: 0,
            console_height: layout.console,
            zoom: if layout.zoom > 0.0 { layout.zoom.clamp(ZOOM_MIN, ZOOM_MAX) } else { 1.0 },
            report_by_project: layout.report_by_project,
            layout_saved: layout.clone(),
            palette: None,
            root,
            project_title: None,
            closing: false,
        };
        // the zoom of the last session comes back before the first frame; egui's own Ctrl+=/-
        // keys are off, the window's run a narrower range and remember it
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        ctx.set_zoom_factor(app.zoom);
        // the card of the last session comes back selected; a card since removed shows as none
        app.select(layout.selected);
        app.refresh_ledger();
        app
    }

    /// The layout as the window holds it now, in the shape the file keeps.
    fn layout(&self) -> WindowLayout {
        let mut hidden: Vec<TaskState> = self.hidden_states.iter().copied().collect();
        hidden.sort_by_key(|s| TaskState::ALL.iter().position(|a| a == s));
        let mut folded: Vec<i64> = self.folded.iter().copied().collect();
        folded.sort_unstable();
        WindowLayout {
            console: self.console_height,
            hidden,
            folded,
            tab: Some(self.tab),
            selected: self.selected,
            theme: theme::current().name.to_owned(),
            zoom: self.zoom,
            report_by_project: self.report_by_project,
        }
    }

    /// Sets the window's zoom to `zoom` within its range; the layout remembers it.
    fn set_zoom(&mut self, ctx: &egui::Context, zoom: f32) {
        self.zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        ctx.set_zoom_factor(self.zoom);
    }

    /// Writes the layout when it moved since the last write.
    fn save_layout(&mut self) {
        let layout = self.layout();
        if layout != self.layout_saved {
            layout.save(&self.root);
            self.layout_saved = layout;
        }
    }

    /// Starts making the roadmap the way `mode` says, on the runtime; the console band shows
    /// the outcome and the window turns into the workspace when the file is there.
    fn start_setup(&mut self, mode: ra_console::setup::Mode) {
        let Some(setup) = &mut self.setup else { return };
        let what = match &mode {
            ra_console::setup::Mode::Empty => "writing the bare roadmap".to_owned(),
            ra_console::setup::Mode::Draft => format!("drafting the roadmap with {}", short(&setup.model)),
            ra_console::setup::Mode::Import(file) => format!("importing {} with {}", file.display(), short(&setup.model)),
        };
        setup.running = Some(what.clone());
        self.job = Some(what.clone());
        self.sink.push(Kind::Input, what);
        let mut worker = self.worker.clone();
        worker.model = ra_console::agent::resolve(&setup.model);
        let (sink, tx, root) = (self.sink.clone(), self.job_tx.clone(), self.root.clone());
        self.rt.spawn(async move {
            match ra_console::setup::init(&root, &worker, mode).await {
                Ok(lines) => {
                    for l in lines {
                        sink.push(Kind::Output, l);
                    }
                }
                Err(e) => sink.push(Kind::Error, format!("{e:#}")),
            }
            let _ = tx.send(());
        });
    }

    /// The setup finished: with a roadmap there now, the window opens it and becomes the
    /// workspace; without one, the console band says why and the choice stays.
    fn finish_setup(&mut self) {
        let Some(setup) = &mut self.setup else { return };
        setup.running = None;
        if ra_console::setup::exists(&self.root)
            && let Ok(store) = self.rt.block_on(Store::open(&self.root.join(ra_console::setup::ROADMAP_FILE), "Rhipper Agent"))
        {
            self.store = store.with_source(Source::Window);
            self.setup = None;
            self.roadmap_seen = None;
        }
    }

    /// The choice a directory without a roadmap gets: a draft by a strong model, an import of
    /// a roadmap-shaped file found here, or the bare project.
    fn setup_view(&mut self, ui: &mut egui::Ui) {
        let Some(setup) = &self.setup else { return };
        let (dir, candidates, running) = (setup.dir.clone(), setup.candidates.clone(), setup.running.clone());
        let mut picked: Option<ra_console::setup::Mode> = None;
        egui::ScrollArea::vertical().id_salt("setup").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(900.0);
            cards::card(ui, theme::accent(), true, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("No roadmap in this directory").font(FontId::proportional(26.0)).color(theme::accent()).strong());
                ui.label(RichText::new(&dir).monospace().color(theme::text()));
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "The directory a window starts in is its project; this one has no {} yet. Choose how it begins; the file is yours to edit after.",
                        ra_console::setup::ROADMAP_FILE
                    ))
                    .color(theme::dim()),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("model").small().strong().color(theme::dim()));
                    if let Some(setup) = &mut self.setup {
                        ui.add(egui::TextEdit::singleline(&mut setup.model).desired_width(240.0).font(TextStyle::Monospace));
                    }
                    ui.label(RichText::new("the strongest once per project; a claude-* id runs on Claude Code").color(theme::dim()).small());
                });
                ui.add_space(10.0);
                if let Some(what) = &running {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().color(theme::accent()));
                        ui.label(RichText::new(format!("{what} …")).color(theme::warn_text()));
                    });
                    return;
                }
                cards::section(ui, "DRAFT", theme::accent());
                ui.label(RichText::new("the model reads the README, the layout, the manifest and the documents, and answers with milestones and cards").color(theme::dim()).small());
                if action(ui, "▶  draft a roadmap from this directory", theme::accent()) {
                    picked = Some(ra_console::setup::Mode::Draft);
                }
                ui.add_space(8.0);
                if !candidates.is_empty() {
                    cards::section(ui, "IMPORT", theme::warn_text());
                    ui.label(RichText::new("a roadmap-shaped file found here, converted by the model with every item kept").color(theme::dim()).small());
                    for file in &candidates {
                        if action(ui, &format!("⇥  import {}", file.display().to_string().replace('\\', "/")), theme::warn_text()) {
                            picked = Some(ra_console::setup::Mode::Import(file.clone()));
                        }
                    }
                    ui.add_space(8.0);
                }
                cards::section(ui, "EMPTY", theme::dim());
                ui.label(RichText::new("the bare project and nothing else; no model call").color(theme::dim()).small());
                if action(ui, "○  start empty", theme::dim()) {
                    picked = Some(ra_console::setup::Mode::Empty);
                }
            });
        });
        if let Some(mode) = picked {
            self.start_setup(mode);
        }
    }

    /// Reads the ledger and the gotchas again: the same queries as `rc usage` and the same
    /// journal as `rc gotchas`, so the tab shows their numbers.
    fn refresh_ledger(&mut self) {
        let (all_time, by_model, by_node, calls) = self.rt.block_on(async {
            (
                self.worker.all_time().await,
                self.worker.by_model().await,
                self.worker.by_node().await,
                match self.selected {
                    Some(id) => self.worker.calls_of(Some(id)).await,
                    None => None,
                },
            )
        });
        let catalogue = match Catalogue::load(&self.root.join(GOTCHAS_FILE)) {
            Ok(catalogue) => Some(catalogue),
            Err(e) => {
                tracing::warn!("cannot read the gotchas catalogue: {e:#}");
                None
            }
        };
        let journal_path = self.root.join(GOTCHAS_LOG);
        let journal = Journal::at(&journal_path);
        let gotchas = journal
            .counts()
            .into_iter()
            .map(|(id, count)| {
                let message = catalogue.as_ref().and_then(|c| c.get(&id)).map(|g| g.message.clone());
                (id, count, message)
            })
            .collect();
        self.ledger = Ledger {
            by_model: by_model.unwrap_or_default(),
            by_node: by_node.unwrap_or_default(),
            all_time,
            calls: calls.unwrap_or_default(),
            gotchas,
            unmatched: journal.unmatched(),
            retro: ra_console::retro::recent(&self.root, 20),
            catalogue: catalogue.map_or(0, |c| c.entries.len()),
            read_at: Some(Instant::now()),
        };
        self.refresh_costs();
    }

    /// Sums the ledger by card up the tree for the cost column, and has its width measured
    /// again; called when the ledger or the roadmap is read.
    fn refresh_costs(&mut self) {
        self.costs = subtree_costs(&self.roadmap, &self.ledger.by_node);
        self.cost_width = None;
    }

    /// Builds the context tab's text for the selected card: what `rc context <id>` prints, and
    /// the project context the run sends ahead of it. A card that is not there leaves it empty.
    fn refresh_context(&mut self) {
        let id = self.selected;
        let text = id.map(|id| self.rt.block_on(ra_console::agent::context_preview(&self.store, id)));
        let (text, fault) = match text {
            Some(Ok(text)) => (text, None),
            Some(Err(e)) => (String::new(), Some(format!("{e:#}"))),
            None => (String::new(), None),
        };
        self.context =
            ContextView { id, text, project: ra_console::agent::project_context(&self.store), fault, read_at: Some(Instant::now()) };
    }

    /// Reads the reviewer's verdict on the selected card's last run from its context; a card
    /// never run or never judged, or none selected, leaves it empty.
    fn refresh_verdict(&mut self) {
        let id = self.selected;
        let root = self.store.path().parent().filter(|p| !p.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let text = id
            .and_then(|id| ra_context::TaskContext::open(&root, id).handoffs.pop())
            .and_then(|h| h.verdict)
            .and_then(|v| v.lines().next().map(|line| line.trim().to_owned()));
        self.verdict = VerdictView { id, text };
    }

    /// Builds the diff tab for the selected card: `git diff HEAD` over the files its last
    /// handoff names, split by file. A card without a handoff, or none selected, leaves it empty.
    fn refresh_diff(&mut self) {
        let id = self.selected;
        let root = self.store.path().parent().filter(|p| !p.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let changed = id.and_then(|id| ra_context::TaskContext::open(&root, id).handoffs.pop()).map(|h| h.changed).unwrap_or_default();
        let (files, fault) = if changed.is_empty() {
            (Vec::new(), None)
        } else {
            let out = std::process::Command::new("git").args(["diff", "HEAD", "--"]).args(&changed).current_dir(&root).output();
            match out {
                Ok(out) if out.status.success() => (split_diff(&String::from_utf8_lossy(&out.stdout), &changed), None),
                Ok(out) => (Vec::new(), Some(String::from_utf8_lossy(&out.stderr).trim().to_owned())),
                Err(e) => (Vec::new(), Some(format!("git does not run: {e}"))),
            }
        };
        self.diff = DiffView { id, files, fault, read_at: Some(Instant::now()) };
    }

    /// The housekeeping of a frame: the activity of other processes, the roadmap file, the jobs,
    /// and a request to quit from another instance.
    fn tick(&mut self, ctx: &egui::Context) {
        if ra_console::instance::quit_requested(&self.root) {
            self.closing = true;
        }
        // `rc walk` from a session: the window starts the walk, so the tab shows it
        if let Some(parallel) = ra_console::agent::walk_requested(&self.root) {
            self.start_walk(parallel);
        }
        ra_console::ui::tail_activity_at(&self.root.join(ra_console::ui::ACTIVITY_FILE), &mut self.activity_offset, &self.sink);
        self.raise_toasts();
        let modified = self.store.modified();
        if modified != self.roadmap_seen {
            self.roadmap_seen = modified;
            match self.rt.block_on(self.store.load()) {
                Ok(roadmap) => {
                    let nodes = roadmap.nodes;
                    self.roadmap_width = measure_roadmap(ctx, &nodes);
                    // the project's name, for its tab and the window's title
                    self.project_title = nodes.iter().find(|n| n.kind == NodeKind::Project).map(|n| n.title.clone());
                    self.roadmap = nodes;
                    self.gambits = roadmap.gambits;
                    self.refresh_costs();
                }
                Err(e) => tracing::warn!("the roadmap file does not parse: {e:#}"),
            }
            // a state change rewrites the file, so the history moved with it; the latest line
            // of a card wins
            match self.store.history(None) {
                Ok(changes) => self.last_changes = changes.into_iter().map(|c| (c.id, c)).collect(),
                Err(e) => tracing::warn!("the history does not read: {e:#}"),
            }
            // a card that just went to work takes the view, wherever the run was started
            let working = self.roadmap.iter().find(|n| n.state == TaskState::InProgress).map(|n| n.id);
            if working.is_some() && working != self.followed {
                self.select(working);
            }
            // a run of any instance just started or ended: the ledger and the journal may have
            // moved, and the context of the card that ran has a new handoff
            if working != self.followed {
                self.refresh_ledger();
                self.context.id = None;
                self.diff.id = None;
                self.verdict.id = None;
            }
            self.followed = working;
        }
        while self.job_done.try_recv().is_ok() {
            self.job = None;
            self.refresh_ledger();
            self.context.id = None;
            self.diff.id = None;
            self.verdict.id = None;
            if self.setup.as_ref().is_some_and(|s| s.running.is_some()) {
                self.finish_setup();
            }
        }
        // the open file follows the disk, so a worker's edit shows as it lands
        if let Some(pane) = &mut self.code {
            pane.follow();
        }
        // the local server, once a minute, with a model list and no model call
        if self.local_probed.is_none_or(|t| t.elapsed() >= ra_providers::openrouter::PROBE_EVERY) {
            self.local_probed = Some(Instant::now());
            self.rt.spawn(self.local.clone().probe_local());
        }
        // the plan's windows, every five seconds: a run of any instance may have moved them
        if self.limits_read.elapsed() >= Duration::from_secs(5) {
            self.limits = ra_providers::limits::Limits::load(Path::new(ra_providers::claude_code::LIMITS_FILE));
            self.limits_read = Instant::now();
            self.week = ra_providers::limits::Week::load(Path::new(ra_providers::claude_code::LIMITS_LOG));
            // the week's pill aims at the new value and glows for a moment when it moved
            #[allow(clippy::cast_possible_truncation)]
            let target = self.limits.windows.get("seven_day").map_or(0.0, |w| w.utilization as f32);
            if (target - self.week_target).abs() > 0.0005 {
                self.week_target = target;
                self.week_moved = Some(Instant::now());
            }
        }
    }

    /// Raises a toast for every run of the log that ended since the last frame, in the order
    /// the log opened them. The log numbers its runs in sequence, so the scan walks on from the
    /// last one announced and stops at the first still going, whichever process runs it: a run
    /// of `rc run` elsewhere reaches the log through the activity file.
    fn raise_toasts(&mut self) {
        while let Some(run) = self.sink.run(self.announced + 1) {
            let Some(outcome) = run.outcome else { break };
            self.announced += 1;
            let id = self.announced;
            let title =
                self.sink.with_lines(|lines| lines.iter().find(|l| l.kind == Kind::Run && l.run == Some(id)).map(|l| l.text.clone()));
            let title = title.unwrap_or_else(|| "a run".to_owned());
            let card = title.strip_prefix('#').and_then(|t| t.split_whitespace().next()).and_then(|n| n.parse().ok());
            self.toasts.push(Toast { card, title, failed: outcome.starts_with('✗'), outcome, shown: Instant::now() });
        }
    }

    /// The toasts, stacked up from the lower right over whatever is there, the newest lowest. A
    /// click on one picks its card and takes it down; the clock takes a finished run's down
    /// after [`TOAST_FOR`], a failed run's waits for the click.
    fn toasts_view(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|t| t.failed || t.shown.elapsed() < TOAST_FOR);
        if self.toasts.is_empty() {
            return;
        }
        let mut picked = None;
        egui::Area::new(egui::Id::unique("toasts"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
            .show(ctx, |ui| {
                ui.with_layout(Layout::bottom_up(Align::Max), |ui| {
                    for (i, toast) in self.toasts.iter().enumerate().rev() {
                        // the card's own name and lane when the roadmap still has it
                        let node = toast.card.and_then(|id| self.roadmap.iter().find(|n| n.id == id));
                        let name = node.map_or_else(|| toast.title.clone(), |n| format!("#{} {}", n.id, n.title));
                        let lane =
                            if toast.failed { theme::accent() } else { theme::lane(node.map_or(1, |n| tree::depth(&self.roadmap, n.id))) };
                        if cards::toast(ui, &name, &toast.outcome, lane, toast.failed).clicked() {
                            picked = Some(i);
                        }
                        ui.add_space(8.0);
                    }
                });
            });
        if let Some(i) = picked {
            let toast = self.toasts.remove(i);
            if toast.card.is_some() {
                self.select(toast.card);
            }
        }
    }

    /// Picks a card; the model field of the card view and the ledger's calls follow. An unsaved
    /// edit holds the pick and asks first.
    fn select(&mut self, id: Option<i64>) {
        if self.selected == id {
            return;
        }
        if self.draft_dirty() {
            self.pending = Some(id.map_or(Pick::Nothing, Pick::Card));
            return;
        }
        self.selected = id;
        self.ledger.calls = match id {
            Some(id) => self.rt.block_on(self.worker.calls_of(Some(id))).unwrap_or_default(),
            None => Vec::new(),
        };
    }

    /// Whether the card view holds an edit its card does not have yet.
    fn draft_dirty(&self) -> bool {
        self.draft.as_ref().is_some_and(|d| self.roadmap.iter().find(|n| n.id == d.id).is_some_and(|n| d.dirty(n)))
    }

    /// Writes the draft through the store, each field that moved by its own setter, and has
    /// the roadmap read again; what went wrong, if anything, reaches the console band.
    fn save_draft(&mut self) {
        let Some(draft) = self.draft.clone() else { return };
        let Some(node) = self.roadmap.iter().find(|n| n.id == draft.id).cloned() else { return };
        let store = self.store.clone();
        let result = self.rt.block_on(async {
            if draft.title.trim() != node.title {
                store.set_title(node.id, &draft.title).await?;
            }
            let objective = draft.objective.trim();
            let acceptance = draft.acceptance.trim();
            let moved = |text: &str, was: Option<&str>| text != was.unwrap_or("").trim();
            if moved(objective, node.objective.as_deref()) || moved(acceptance, node.acceptance.as_deref()) {
                store.set_details(node.id, Some(objective), Some(acceptance)).await?;
            }
            if draft.files() != node.files {
                store.set_files(node.id, &draft.files()).await?;
            }
            if draft.complexity != node.complexity {
                store.set_complexity(node.id, draft.complexity).await?;
            }
            anyhow::Ok(())
        });
        match result {
            Ok(()) => {
                self.sink.push(Kind::Output, format!("saved #{}", node.id));
                // the file changed under the store: the column reads it again now, not when
                // the mtime happens to differ
                self.roadmap_seen = None;
                self.draft = None;
                self.context.id = None;
            }
            Err(e) => self.sink.push(Kind::Error, format!("cannot save #{}: {e:#}", node.id)),
        }
    }

    /// The confirm over a pick made with an unsaved edit: save it and go, drop it and go, or
    /// stay with the card.
    fn confirm_pending(&mut self, ctx: &egui::Context) {
        let Some(next) = self.pending else { return };
        let id = self.draft.as_ref().map_or(0, |d| d.id);
        let mut answer: Option<bool> = None;
        let frame = Frame::new()
            .fill(theme::panel())
            .stroke(Stroke::new(1.0, cards::alpha(theme::warn_text(), 200)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(16, 12));
        let modal = egui::Modal::new(egui::Id::unique("unsaved-edit")).frame(frame).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(format!("#{id} has an unsaved edit")).color(theme::warn_text()).size(16.0).strong());
            ui.label(RichText::new("save it before moving on, drop it, or stay with the card").color(theme::dim()));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if action(ui, "save and go", theme::online()) {
                    answer = Some(true);
                }
                if action(ui, "drop the edit", theme::error_text()) {
                    answer = Some(false);
                }
                if action(ui, "stay", theme::dim()) {
                    self.pending = None;
                }
            });
        });
        if modal.should_close() {
            self.pending = None;
        }
        if let Some(save) = answer {
            if save {
                self.save_draft();
            }
            // a save that failed keeps its draft, and the pick with it
            if save && self.draft_dirty() {
                self.pending = None;
                return;
            }
            self.draft = None;
            self.pending = None;
            self.select(match next {
                Pick::Card(id) => Some(id),
                Pick::Nothing => None,
            });
        }
    }

    /// The keys of the roadmap column: Up and Down move the selection along the rows shown,
    /// Left and Right fold and unfold, Enter runs the selected card. `sideways` is off while a
    /// text field holds the caret, so Left and Right stay with it.
    fn roadmap_keys(&mut self, ctx: &egui::Context, sideways: bool) {
        let (up, down, left, right, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                sideways && i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                sideways && i.consume_key(Modifiers::NONE, Key::ArrowRight),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        if up {
            self.step_selection(-1);
        }
        if down {
            self.step_selection(1);
        }
        if left {
            self.fold_selected();
        }
        if right {
            self.unfold_selected();
        }
        if enter
            && let Some(node) = self.selected.and_then(|id| self.roadmap.iter().find(|n| n.id == id))
            && node.kind != NodeKind::Project
        {
            let line = format!("/run {}", node.id);
            self.submit(ctx, line);
        }
    }

    /// Moves the selection `step` rows along the rows shown; from no selection, to the first
    /// or the last.
    fn step_selection(&mut self, step: isize) {
        if self.visible_rows.is_empty() {
            return;
        }
        let last = self.visible_rows.len() - 1;
        let at = self.selected.and_then(|id| self.visible_rows.iter().position(|r| *r == id));
        let next = match at {
            None if step > 0 => 0,
            None => last,
            Some(i) => i.saturating_add_signed(step).min(last),
        };
        let id = self.visible_rows[next];
        self.select(Some(id));
        self.scroll_to = Some(id);
    }

    /// Left: folds the selected row while it has children to hide; otherwise climbs to its parent.
    fn fold_selected(&mut self) {
        let Some(id) = self.selected else { return };
        let searching = !self.search.trim().is_empty();
        let has_children = self.roadmap.iter().any(|n| n.parent == Some(id));
        if has_children && !searching && !self.folded.contains(&id) {
            self.folded.insert(id);
            return;
        }
        if let Some(parent) = self.roadmap.iter().find(|n| n.id == id).and_then(|n| n.parent) {
            self.select(Some(parent));
            self.scroll_to = Some(parent);
        }
    }

    /// Right: unfolds the selected row; already open, steps into its first child shown.
    fn unfold_selected(&mut self) {
        let Some(id) = self.selected else { return };
        if self.folded.remove(&id) {
            return;
        }
        let child = self.roadmap.iter().find(|n| n.parent == Some(id) && self.visible_rows.contains(&n.id)).map(|n| n.id);
        if let Some(child) = child {
            self.select(Some(child));
            self.scroll_to = Some(child);
        }
    }

    /// `theme <name>`: switches the palette and repaints; a bare `theme` lists them and marks
    /// the current one. The choice is saved with the layout.
    fn set_theme(&mut self, ctx: &egui::Context, name: &str) {
        if name.is_empty() {
            let names: Vec<String> = theme::ALL
                .iter()
                .map(|t| if t.name == theme::current().name { format!("{}*", t.name) } else { t.name.to_owned() })
                .collect();
            self.sink.push(Kind::Output, format!("themes: {}", names.join(", ")));
            return;
        }
        let Some(theme) = theme::by_name(name) else {
            self.sink.push(Kind::Error, format!("no theme called {name}"));
            return;
        };
        theme::set(theme);
        ctx.all_styles_mut(|style| style.visuals = theme::visuals());
        ctx.request_repaint();
        self.save_layout();
        self.sink.push(Kind::Output, format!("theme {}", theme.name));
    }

    /// Runs one command line, the way the console does: a slow call goes to the runtime and
    /// reports through the log; anything else answers now.
    fn submit(&mut self, ctx: &egui::Context, line: String) {
        let line = line.trim().to_owned();
        if line.is_empty() {
            return;
        }
        if self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        self.browsing = None;
        let name = line.split_whitespace().next().unwrap_or("").trim_start_matches(['/', '.']).to_ascii_lowercase();
        self.sink.push(Kind::Input, line.clone());
        // the theme is the window's own: no console command knows it
        if name == "theme" {
            self.set_theme(ctx, line.split_whitespace().skip(1).collect::<Vec<_>>().join(" ").as_str());
            return;
        }
        // a run goes to the installed `rc` as a child process in the project's directory: its
        // steps land in the project's own activity log, which this workspace tails, so a
        // window with many projects keeps each one's work in its own tab, and the run outlives
        // the window
        if name == "run" {
            let words: Vec<String> = line.trim_start_matches(['/', '.']).split_whitespace().skip(1).map(str::to_owned).collect();
            // the walk is a process of its own, hours long and nobody's child: not a job
            if words.iter().any(|w| w == "--all") {
                // `/run --all --parallel N` runs that many cards at once
                let parallel = words.iter().position(|w| w == "--parallel").and_then(|at| words.get(at + 1)).and_then(|n| n.parse().ok());
                self.start_walk(parallel);
                return;
            }
            let id = words.first().cloned().unwrap_or_default();
            self.job = Some(format!("running #{id}"));
            let mut args = vec!["run".to_owned(), id.clone()];
            if words.iter().any(|w| w == "dry") {
                args.push("--dry".to_owned());
            }
            // the card's own model wins; else the badge's
            let has_own =
                id.parse::<i64>().ok().and_then(|id| self.roadmap.iter().find(|n| n.id == id)).is_some_and(|n| n.assignee.is_some());
            if !has_own {
                args.extend(["--model".to_owned(), self.worker.model.clone()]);
            }
            if !self.worker.debug {
                args.push("--no-debug".to_owned());
            }
            let (sink, tx, root) = (self.sink.clone(), self.job_tx.clone(), self.root.clone());
            self.rt.spawn(async move {
                let rc = std::env::current_exe()
                    .map_or_else(|_| PathBuf::from("rc"), |p| p.with_file_name(format!("rc{}", std::env::consts::EXE_SUFFIX)));
                match tokio::process::Command::new(&rc).args(&args).current_dir(&root).output().await {
                    Ok(out) => {
                        let text = String::from_utf8_lossy(&out.stdout);
                        let lines: Vec<String> = text.lines().map(str::to_owned).filter(|l| !l.trim().is_empty()).collect();
                        push_answer(&sink, lines);
                        if !out.status.success() {
                            let err = String::from_utf8_lossy(&out.stderr);
                            sink.push(Kind::Error, err.lines().last().unwrap_or("the run failed").to_owned());
                        }
                    }
                    Err(e) => sink.push(Kind::Error, format!("cannot start {}: {e}", rc.display())),
                }
                let _ = tx.send(());
            });
            return;
        }
        // the other slow commands go to the runtime: a model call, and an install that copies
        // the binaries and reads the registry
        if matches!(name.as_str(), "reassess" | "reassess-tree" | "ping" | "install") {
            let arg = line.split_whitespace().nth(1).unwrap_or("").to_owned();
            self.job = Some(match name.as_str() {
                "run" => format!("running #{arg}"),
                "ping" => "pinging the model".to_owned(),
                "install" => "installing the build".to_owned(),
                other => format!("{other} #{arg}"),
            });
            let (store, sink, worker, tx) = (self.store.clone(), self.sink.clone(), self.worker.clone(), self.job_tx.clone());
            self.rt.spawn(async move {
                match commands::run(&line, &store, &worker).await {
                    Ok((out, _)) => push_answer(&sink, out),
                    Err(e) => sink.push(Kind::Error, format!("{e:#}")),
                }
                let _ = tx.send(());
            });
            return;
        }
        match self.rt.block_on(commands::run(&line, &self.store, &self.worker)) {
            Ok((out, outcome)) => {
                push_answer(&self.sink, out);
                match outcome {
                    Outcome::Continue => {}
                    Outcome::Clear => self.sink.clear(),
                    Outcome::Quit => self.closing = true,
                    Outcome::Model(model) => self.worker.model = model,
                    Outcome::Budgets(on) => self.worker.budgets = on,
                    Outcome::ToggleDebug => {
                        self.show_debug = !self.show_debug;
                        self.sink.push(Kind::Output, format!("trace lines {}", if self.show_debug { "on" } else { "off" }));
                    }
                    // the window's band shows the lines as they are; the fold is the terminal's,
                    // and so is the state filter: the window has its chips
                    Outcome::Fold | Outcome::Filter(_) => {}
                }
            }
            Err(e) => self.sink.push(Kind::Error, format!("{e:#}")),
        }
    }

    fn splash(&self, root: &mut egui::Ui) {
        egui::CentralPanel::default().frame(Frame::new().fill(theme::bg())).show(root, |ui| {
            let avail = ui.available_rect_before_wrap();
            // the picture keeps 16:10 and fills what it can, centered
            let scale = (avail.width() / 640.0).min(avail.height() / 400.0);
            let size = egui::vec2(640.0 * scale, 400.0 * scale);
            let rect = egui::Rect::from_center_size(avail.center(), size);
            egui::Image::new(egui::include_image!("../assets/splash.svg")).paint_at(ui, rect);
            let painter = ui.painter();
            let unit = scale;
            let [title, small, note] = theme::current().splash;
            painter.text(
                rect.left_bottom() + egui::vec2(40.0 * unit, -34.0 * unit),
                egui::Align2::LEFT_BOTTOM,
                "Rhipper",
                FontId::proportional(44.0 * unit),
                title,
            );
            let mut x = rect.left() + 43.0 * unit;
            for ch in "AGENT".chars() {
                painter.text(
                    egui::pos2(x, rect.bottom() - 14.0 * unit),
                    egui::Align2::LEFT_BOTTOM,
                    ch,
                    FontId::proportional(12.5 * unit),
                    small,
                );
                x += 11.0 * unit;
            }
            // the last commits sit beside the wordmark, newest at the bottom line, dim: a
            // relaunch after a pull is told from the one before it at a glance
            let mut y = rect.bottom() - 14.0 * unit;
            for line in self.commits.iter().take(3) {
                painter.text(
                    egui::pos2(rect.left() + 124.0 * unit, y),
                    egui::Align2::LEFT_BOTTOM,
                    line,
                    FontId::monospace(9.0 * unit),
                    theme::dim(),
                );
                y -= 11.0 * unit;
            }
            let right = rect.right_bottom() + egui::vec2(-32.0 * unit, -28.0 * unit);
            painter.text(
                right,
                egui::Align2::RIGHT_BOTTOM,
                format!("Rhipper Agent build {}", ra_console::VERSION),
                FontId::proportional(12.0 * unit),
                note,
            );
            let second = match &self.fault {
                Some(fault) => format!("cannot open the roadmap: {fault}"),
                None => format!("{} cards · {}", self.roadmap.len(), self.worker.model),
            };
            painter.text(
                right + egui::vec2(0.0, 16.0 * unit),
                egui::Align2::RIGHT_BOTTOM,
                second,
                FontId::proportional(11.0 * unit),
                small,
            );
            let progress = (self.started.elapsed().as_secs_f32() / SPLASH.as_secs_f32()).min(1.0);
            let bar = egui::Rect::from_min_size(
                rect.left_bottom() - egui::vec2(0.0, 3.0 * unit),
                egui::vec2(rect.width() * progress, 3.0 * unit),
            );
            painter.rect_filled(bar, 0.0, theme::accent());
        });
        root.ctx().request_repaint_after(Duration::from_millis(16));
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(" ✦ Rhipper").color(theme::accent()).strong().size(15.0));
            ui.label(RichText::new("Agent").strong().size(15.0));
            ui.label(RichText::new(ra_console::VERSION).color(theme::dim()).small());
            ui.add_space(10.0);
            cards::badge_marked(ui, "◉", &self.worker.model, theme::accent())
                .on_hover_text("the model of the next run; `model <id>` swaps it");
            // debug mode: a run ends with the worker's own look back, kept for the next runs
            let debug = self.worker.debug;
            let (mark, color, hint) = if debug {
                (
                    "⚙",
                    theme::debug(),
                    "debug mode: a run ends with what would have made it faster, kept for the next runs · click to turn off",
                )
            } else {
                ("○", theme::dim(), "debug mode off: runs are not asked what would have made them faster · click to turn on")
            };
            if cards::badge_marked(ui, mark, "debug", color).interact(egui::Sense::click()).on_hover_text(hint).clicked() {
                self.worker.debug = !debug;
            }
            // the local server as a dot: green up, red down, dim until the first probe answers
            let url = ra_providers::openrouter::local_url();
            let (mark, color, hint) = match self.local.up() {
                Some(true) => ("●", theme::online(), format!("the local server at {url} answers; probed once a minute")),
                Some(false) => ("●", theme::error_text(), format!("no local server at {url}; set RHIPPER_LOCAL_URL to move it")),
                None => ("○", theme::dim(), format!("probing the local server at {url}")),
            };
            cards::badge_marked(ui, mark, "local", color).on_hover_text(hint);
            match current_task(&self.roadmap, &self.gambits) {
                Some(node) => {
                    cards::badge_marked(ui, "◇", &format!("#{} {}", node.id, node.title), theme::trace())
                        .on_hover_text("the task the workers hold next");
                }
                None => {
                    cards::badge_marked(ui, "◇", "no task queued", theme::dim());
                }
            }
            if let Some(job) = &self.job {
                let t = ui.input(|i| i.time);
                let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][((t * 10.0) as usize) % 10];
                cards::badge_marked(ui, spinner, job, theme::warn_text());
            }
            // the counts, left to right after a gap: a right-aligned group pushed its first
            // badges past the edge as it grew
            ui.add_space(18.0);
            let session = self.worker.tally();
            cards::badge_marked(
                ui,
                "⇅",
                &format!(
                    "in {} · out {} · {}",
                    compact(session.prompt_tokens),
                    compact(session.completion_tokens),
                    dollars(session.cost_usd)
                ),
                theme::warn_text(),
            )
            .on_hover_text("this session's tokens and cost");
            match &self.ledger.all_time {
                Some(all) => {
                    cards::badge_marked(ui, "◉", &format!("ledger {} · {} calls", dollars(all.cost_usd), all.calls), theme::online())
                        .on_hover_text("all time, from var/usage.db");
                }
                None => {
                    cards::badge_marked(ui, "◌", "no ledger", theme::dim());
                }
            }
            // the week of the plan as a pill filled to its percent, colored by how full it is;
            // the fill eases to a new value and glows brighter for a moment when it moves
            if self.limits.fresh()
                && let Some(week) = self.limits.windows.get("seven_day")
            {
                let target = self.week_target;
                let shown = ui.ctx().animate_value_with_time(egui::Id::unique("week-pill"), target, 0.9);
                let fade = self.week_moved.map_or(0.0, |t| (1.0 - t.elapsed().as_secs_f32() / 1.6).clamp(0.0, 1.0));
                if fade > 0.0 {
                    ui.ctx().request_repaint();
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let pulse = (40.0 + 190.0 * fade) as u8;
                let color = if target >= 0.8 {
                    theme::error_text()
                } else if target >= 0.5 {
                    theme::warn_text()
                } else {
                    theme::online()
                };
                let (used, resets_at, threshold) = (week.utilization, week.resets_at, self.threshold);
                cards::pill_bar(ui, &format!("week {:.0}%/{:.0}%", shown * 100.0, threshold * 100.0), shown, color, pulse).on_hover_ui(
                    |ui| {
                        ui.label(format!(
                            "the plan's week: {:.0}% used, resets in {}",
                            used * 100.0,
                            ra_providers::limits::until(resets_at)
                        ));
                        ui.label(format!("claude-* runs steer at {:.0}%: local when up, else refused without --force", threshold * 100.0));
                        cards::sparkline(ui, &self.week.days, color);
                        let steepest = match self.week.steepest() {
                            Some((day, rise)) => format!("steepest day {day}, +{:.0}%", rise * 100.0),
                            None => "no day climbed this week".to_owned(),
                        };
                        ui.label(
                            RichText::new(format!("{} to {} · {steepest}", self.week.day_name(0), self.week.day_name(6)))
                                .color(theme::dim())
                                .small(),
                        );
                        ui.label(RichText::new("from the harness's runs").color(theme::dim()).small());
                    },
                );
            }
            let secs = self.started.elapsed().as_secs();
            ui.label(RichText::new(format!("up {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)).color(theme::dim()).small());
        });
    }

    fn roadmap_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("ROADMAP").small().strong().color(theme::dim()));
            // the tree, or the order a walk would run the cards in
            ui.spacing_mut().item_spacing.x = 4.0;
            if cards::tab(ui, "tree", theme::accent(), self.view == RoadmapView::Tree) {
                self.view = RoadmapView::Tree;
            }
            if cards::tab(ui, "walk", theme::warn_text(), self.view == RoadmapView::Walk) {
                self.view = RoadmapView::Walk;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let count = if self.search.trim().is_empty() {
                    format!("{} cards", self.roadmap.len())
                } else {
                    format!("{} of {} cards", self.visible_rows.len(), self.roadmap.len())
                };
                ui.label(RichText::new(count).small().color(theme::dim()));
                // the cost column: each row's subtree from the ledger, the root the total
                if cards::chip(ui, "cost", theme::accent(), self.show_cost) {
                    self.show_cost = !self.show_cost;
                }
            });
        });
        self.gambit_band(ui);
        // the column's width is measured once per reload, like the titles
        if self.show_cost && self.cost_width.is_none() {
            self.cost_width = Some(measure_costs(ui, &self.costs));
        }
        // the filters: one chip per state the roadmap holds; a dim chip hides its tasks
        let present: Vec<TaskState> = TaskState::ALL.into_iter().filter(|s| self.roadmap.iter().any(|n| n.state == *s)).collect();
        if present.len() > 1 {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for state in present {
                    let on = !self.hidden_states.contains(&state);
                    if cards::chip(ui, &state.to_string(), theme::state_color(state), on) {
                        if on {
                            self.hidden_states.insert(state);
                        } else {
                            self.hidden_states.remove(&state);
                        }
                    }
                }
            });
        }
        // the search box: a row stays while its title or id contains the text; its ancestors
        // stay with it, and the folds stand aside until the box is empty again
        ui.horizontal(|ui| {
            ui.label(RichText::new("⌕").color(theme::dim()).size(16.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.search.is_empty()
                    && ui.small_button(RichText::new("×").color(theme::dim())).on_hover_text("clear the search (Escape)").clicked()
                {
                    self.search.clear();
                }
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text(RichText::new("search by title or id").color(theme::dim()).italics())
                        .desired_width(ui.available_width())
                        .frame(Frame::new()),
                );
                let focused = response.has_focus() || response.lost_focus();
                if focused {
                    // Escape empties the box; Up, Down and Enter walk and run the rows under it
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        self.search.clear();
                    }
                    self.roadmap_keys(ui.ctx(), false);
                }
            });
        });
        ui.add_space(4.0);
        let needle = self.search.trim().to_lowercase();
        let searching = !needle.is_empty();
        let depths = tree::depths(&self.roadmap);
        let nodes = self.roadmap.clone();
        let matched: HashSet<i64> = if searching { search_matches(&nodes, &needle) } else { HashSet::new() };
        // the structure a hidden chip still needs: the ancestors of every card that stays shown;
        // a milestone in a hidden state with nothing shown under it goes with its cards
        let mut holds_shown: HashSet<i64> = HashSet::new();
        for node in nodes.iter().filter(|n| matches!(n.kind, NodeKind::Task | NodeKind::Subtask) && !self.hidden_states.contains(&n.state))
        {
            let mut parent = node.parent;
            while let Some(id) = parent {
                if !holds_shown.insert(id) {
                    break;
                }
                parent = nodes.iter().find(|n| n.id == id).and_then(|n| n.parent);
            }
        }
        // the column as the console draws it: the project is a bubble, each phase a bubble
        // inside it with a glowing rule between them, the tasks rows inside their phase, and
        // the subtasks of a task in a small bubble under it
        let walk = Walk {
            searching,
            matched,
            holds_shown,
            hidden: self.hidden_states.clone(),
            folded: self.folded.clone(),
            selected: self.selected,
            scroll_to: self.scroll_to,
            depths,
            last_changes: self.last_changes.clone(),
            now: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            costs: self.show_cost.then(|| self.costs.clone()),
            cost_width: self.cost_width.unwrap_or(0.0),
            dragging: self.drag,
            shaking: self.shake.map(|(id, at)| (id, at.elapsed().as_secs_f32())).filter(|(_, t)| *t < SHAKE_FOR),
        };
        if walk.dragging.is_some() || walk.shaking.is_some() {
            ui.ctx().request_repaint();
        }
        let mut out = Walked::default();
        let view = self.view;
        let scroll = egui::ScrollArea::vertical().id_salt("roadmap").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match view {
                RoadmapView::Tree => {
                    for root in nodes.iter().filter(|n| n.parent.is_none()) {
                        draw_bubble(ui, &nodes, root, &walk, &mut out);
                    }
                }
                RoadmapView::Walk => {
                    // the projected order: the cards as a walk would take them, numbered,
                    // the filters and the search applying as in the tree
                    let order = ra_console::agent::walk_order(&nodes, &self.gambits, ra_console::agent::limits_tight(&self.limits));
                    if order.is_empty() {
                        ui.label(RichText::new("no card is ready to walk").color(theme::dim()).italics());
                    }
                    for (rank, id) in order.iter().enumerate() {
                        let Some(node) = nodes.iter().find(|n| n.id == *id) else { continue };
                        if !walk.shows(node) {
                            continue;
                        }
                        out.shown.push(node.id);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new(format!("{:>2}", rank + 1)).monospace().color(theme::warn_text()).strong());
                            ui.vertical(|ui| {
                                let lane = theme::lane(walk.depths.get(&node.id).copied().unwrap_or(0));
                                header_row(ui, &nodes, node, &[], lane, &walk, &mut out);
                            });
                        });
                    }
                }
            }
        });
        // while a tooltip is open the rest of the column steps back: a shade over everything
        // but the row it belongs to; the tooltip itself is on its own layer above. The shade
        // outlasts the tooltip by a moment, so a pass over a rule between rows does not flash
        if let Some(row) = out.tooltip_row {
            self.shade = Some((row, Instant::now()));
        }
        let shade_row = self.shade.filter(|(_, at)| at.elapsed() < Duration::from_millis(400)).map(|(row, _)| row);
        if shade_row.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        }
        if let Some(row) = shade_row {
            let area = scroll.inner_rect;
            let shade = cards::alpha(theme::bg(), 150);
            let painter = ui.painter();
            if row.top() > area.top() {
                painter.rect_filled(egui::Rect::from_min_max(area.min, egui::pos2(area.right(), row.top())), CornerRadius::ZERO, shade);
            }
            if row.bottom() < area.bottom() {
                painter.rect_filled(egui::Rect::from_min_max(egui::pos2(area.left(), row.bottom()), area.max), CornerRadius::ZERO, shade);
            }
        }
        self.visible_rows = out.shown;
        self.scroll_to = None;
        if let Some(id) = out.toggled
            && !self.folded.remove(&id)
        {
            self.folded.insert(id);
        }
        if let Some(id) = out.clicked {
            let next = if self.selected == Some(id) && out.toggled.is_none() { None } else { Some(id) };
            self.select(next);
        }
        if let Some(line) = out.command {
            self.submit(ui.ctx(), line);
        }
        if let Some((id, state)) = out.set_state {
            self.set_state(id, state);
        }
        if let Some(parent) = out.add_under {
            self.new_card = Some(NewCard::under(parent));
            self.tab = Tab::Card;
        }
        // a drag holds its row until the pointer lets go; where it lets go decides the move
        if let Some(id) = out.drag_started {
            self.drag = Some(id);
        }
        if let Some(id) = self.drag {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let (released, down) = ui.input(|i| (i.pointer.any_released(), i.pointer.any_down()));
            if released {
                self.drag = None;
                if let Some((target, slot)) = out.drop {
                    self.move_card(id, target, slot);
                }
            } else if !down {
                self.drag = None;
            }
        }
    }

    /// Writes a drop through the store: onto a row makes the card its last child, above or
    /// below a row puts it beside that row under the row's parent. A refusal (its own subtree,
    /// the project) shakes the dragged row and says why in the log.
    fn move_card(&mut self, id: i64, target: i64, slot: DropSlot) {
        let Some(onto) = self.roadmap.iter().find(|n| n.id == target) else { return };
        let result = match (slot, onto.parent) {
            (DropSlot::Onto, _) | (_, None) => self.rt.block_on(self.store.move_node(id, target, None)),
            (DropSlot::After, Some(parent)) => self.rt.block_on(self.store.move_node(id, parent, Some(target))),
            (DropSlot::Before, Some(parent)) => {
                // the sibling the card follows is the one above the target, the card itself not counted
                let before = self.roadmap.iter().filter(|n| n.parent == Some(parent) && n.id != id).take_while(|n| n.id != target).last();
                match before {
                    Some(sibling) => self.rt.block_on(self.store.move_node(id, parent, Some(sibling.id))),
                    None => self.rt.block_on(self.store.move_first(id, parent)),
                }
            }
        };
        match result {
            Ok(()) => {
                self.sink.push(
                    Kind::Output,
                    format!("#{id} moved under #{}", if slot == DropSlot::Onto { target } else { onto.parent.unwrap_or(target) }),
                );
                self.roadmap_seen = None;
            }
            Err(e) => {
                self.sink.push(Kind::Error, format!("cannot move #{id}: {e:#}"));
                self.shake = Some((id, Instant::now()));
            }
        }
    }

    /// The gambits above the column: the active and parked ones as bubbles in a wrapped row,
    /// a `+` that opens the one-line field, and the done ones folded under a dim header. A pick
    /// on a bubble writes through the store and the column reloads.
    fn gambit_band(&mut self, ui: &mut egui::Ui) {
        let gambits = self.gambits.clone();
        let (done, live): (Vec<&Gambit>, Vec<&Gambit>) = gambits.iter().partition(|g| g.state == GambitState::Done);
        let mut pick: Option<(u32, cards::GambitPick)> = None;
        let mut open_field = false;
        // the heading and the add button on one line, then one gambit a row, each the width of
        // the column: side by side they ran the column past its edge
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new("GAMBITS").small().strong().color(theme::dim()));
            if self.new_gambit.is_none()
                && ui.small_button(RichText::new("+").color(theme::accent())).on_hover_text("add a gambit").clicked()
            {
                open_field = true;
            }
        });
        for gambit in &live {
            if let Some(p) = cards::gambit_bubble(ui, gambit) {
                pick = Some((gambit.id, p));
            }
        }
        if open_field {
            self.new_gambit = Some(String::new());
        }
        // the field: one line of text; Enter adds it active at weight 3, Escape drops it
        let mut add = None;
        if let Some(text) = self.new_gambit.as_mut() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("+").color(theme::accent()));
                let response = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(
                            RichText::new("a gambit: one line of direction; Enter adds it at weight 3").color(theme::dim()).italics(),
                        )
                        .desired_width(ui.available_width() - 24.0),
                );
                if open_field {
                    response.request_focus();
                }
                if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    add = Some(text.trim().to_owned());
                } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                    add = Some(String::new());
                }
            });
        }
        if let Some(text) = add {
            self.new_gambit = None;
            if !text.is_empty() {
                self.add_gambit(&text);
            }
        }
        if !done.is_empty() {
            let arrow = if self.done_gambits_open { "▾" } else { "▸" };
            let header = RichText::new(format!("{arrow} {} done", done.len())).small().color(theme::dim());
            if ui.add(egui::Label::new(header).sense(egui::Sense::click())).clicked() {
                self.done_gambits_open = !self.done_gambits_open;
            }
            if self.done_gambits_open {
                for gambit in &done {
                    if let Some(p) = cards::gambit_bubble(ui, gambit) {
                        pick = Some((gambit.id, p));
                    }
                }
            }
        }
        ui.add_space(2.0);
        if let Some((id, pick)) = pick {
            let written = match pick {
                cards::GambitPick::State(state) => {
                    self.rt.block_on(self.store.set_gambit_state(id, state)).map(|()| format!("g{id} → {state}"))
                }
                cards::GambitPick::Weight(w) => self.rt.block_on(self.store.set_gambit_weight(id, w)).map(|()| format!("g{id} weight {w}")),
            };
            match written {
                Ok(what) => {
                    self.sink.push(Kind::Output, what);
                    self.roadmap_seen = None;
                }
                Err(e) => self.sink.push(Kind::Error, format!("cannot set g{id}: {e:#}")),
            }
        }
    }

    /// Writes a gambit from the band's field, active at weight 3, and has the roadmap read again.
    fn add_gambit(&mut self, text: &str) {
        match self.rt.block_on(self.store.add_gambit(text, 3, &[])) {
            Ok(id) => {
                self.sink.push(Kind::Output, format!("g{id} {text}"));
                self.roadmap_seen = None;
            }
            Err(e) => self.sink.push(Kind::Error, format!("cannot add the gambit: {e:#}")),
        }
    }

    /// Writes a state picked from a row's menu through the store and has the roadmap read
    /// again, so the row recolors this frame rather than when the mtime happens to differ.
    fn set_state(&mut self, id: i64, state: TaskState) {
        match self.rt.block_on(self.store.set_state(id, state)) {
            Ok(()) => {
                self.sink.push(Kind::Output, format!("#{id} → {state}"));
                self.roadmap_seen = None;
            }
            Err(e) => self.sink.push(Kind::Error, format!("cannot set #{id}: {e:#}")),
        }
    }

    /// Writes the form's card under its parent, defined, as `rc add` would, and selects it;
    /// a refusal stays on the form, so the fields keep what was typed.
    fn add_card(&mut self) {
        let Some(form) = self.new_card.as_mut() else { return };
        let title = form.title.trim().to_owned();
        if title.is_empty() {
            form.fault = Some("a card needs a title".to_owned());
            return;
        }
        let waits_on = match form.waits_on() {
            Ok(ids) => ids,
            Err(fault) => {
                form.fault = Some(fault);
                return;
            }
        };
        let files = form.files();
        let objective = form.objective.trim();
        let acceptance = form.acceptance.trim();
        let card = ra_roadmap::store::Card {
            objective: Some(objective).filter(|o| !o.is_empty()),
            acceptance: Some(acceptance).filter(|a| !a.is_empty()),
            complexity: form.complexity,
            files: &files,
            waits_on: &waits_on,
            model: None,
            ..ra_roadmap::store::Card::new()
        };
        let parent = form.parent;
        match self.rt.block_on(self.store.insert_card(Some(parent), NodeKind::Task, &title, &card)) {
            Ok(id) => {
                self.sink.push(Kind::Output, format!("#{id} {title}"));
                self.roadmap_seen = None;
                self.new_card = None;
                self.draft = None;
                self.select(Some(id));
            }
            Err(e) => form.fault = Some(format!("{e:#}")),
        }
    }

    /// The new-card form in the card area: the title, the objective, the acceptance, the
    /// complexity dots, the files and the waits-on; Enter adds (Ctrl+Enter from a multiline
    /// field), Escape cancels.
    fn new_card_view(&mut self, ui: &mut egui::Ui) {
        let Some(form) = self.new_card.as_mut() else { return };
        let parent = self.roadmap.iter().find(|n| n.id == form.parent);
        let lane = theme::lane(parent.map_or(1, |p| tree::depth(&self.roadmap, p.id) + 1));
        let under = parent.map_or_else(|| format!("#{}", form.parent), |p| format!("{} #{} {}", cards::kind_mark(p.kind), p.id, p.title));
        let act = egui::ScrollArea::vertical()
            .id_salt("new-card")
            .auto_shrink([false, false])
            .show(ui, |ui| new_card_form(ui, form, lane, &under))
            .inner;
        match act {
            FormAct::Add => self.add_card(),
            FormAct::Cancel => self.new_card = None,
            FormAct::Stay => {}
        }
    }

    /// The middle of the workspace: the tab row, then the selected card or the ledger.
    fn center(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if cards::tab(ui, "CARD", theme::accent(), self.tab == Tab::Card) {
                self.tab = Tab::Card;
            }
            if cards::tab(ui, "CONTEXT", theme::debug(), self.tab == Tab::Context) {
                self.tab = Tab::Context;
            }
            if cards::tab(ui, "DIFF", theme::warn_text(), self.tab == Tab::Diff) {
                self.tab = Tab::Diff;
            }
            if cards::tab(ui, "LEDGER", theme::online(), self.tab == Tab::Ledger) {
                self.tab = Tab::Ledger;
            }
            if self.tab == Tab::Diff {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button(RichText::new("↻").color(theme::warn_text())).on_hover_text("ask git for the diff again").clicked()
                    {
                        self.diff.id = None;
                    }
                    if let Some(at) = self.diff.read_at.filter(|_| self.diff.id.is_some()) {
                        let secs = at.elapsed().as_secs();
                        let when = if secs < 5 { "just now".to_owned() } else { format!("{secs}s ago") };
                        ui.label(RichText::new(format!("read {when} · refreshed when a run ends")).color(theme::dim()).small());
                    }
                });
            }
            if self.tab == Tab::Context {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button(RichText::new("↻").color(theme::debug()))
                        .on_hover_text("build the context again from the card and its files")
                        .clicked()
                    {
                        self.context.id = None;
                    }
                    if let Some(at) = self.context.read_at.filter(|_| self.context.id.is_some()) {
                        let secs = at.elapsed().as_secs();
                        let when = if secs < 5 { "just now".to_owned() } else { format!("{secs}s ago") };
                        ui.label(RichText::new(format!("built {when} · refreshed when a run ends")).color(theme::dim()).small());
                    }
                });
            }
            if self.tab == Tab::Ledger {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button(RichText::new("↻").color(theme::online()))
                        .on_hover_text("read the ledger and the journal again")
                        .clicked()
                    {
                        self.refresh_ledger();
                    }
                    if let Some(at) = self.ledger.read_at {
                        let secs = at.elapsed().as_secs();
                        let when = if secs < 5 { "just now".to_owned() } else { format!("{secs}s ago") };
                        ui.label(RichText::new(format!("read {when} · refreshed when a run ends")).color(theme::dim()).small());
                    }
                });
            }
        });
        ui.add_space(6.0);
        match self.tab {
            Tab::Card => self.card_view(ui),
            Tab::Context => self.context_view(ui),
            Tab::Diff => self.diff_view(ui),
            Tab::Ledger => self.ledger_view(ui),
        }
    }

    /// The diff tab: what the last run of the selected card changed in the working tree, as
    /// `git diff HEAD` shows it, each file folded under its name; a file name opens it beside
    /// the card.
    fn diff_view(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.38);
                ui.label(RichText::new("±").color(cards::alpha(theme::warn_text(), 120)).size(40.0));
                ui.label(RichText::new("select a card").color(theme::dim()).size(16.0));
                ui.label(RichText::new("this tab shows what its last run changed").color(cards::alpha(theme::dim(), 160)).small());
            });
            return;
        };
        // built on demand: git runs over the files, so not every frame and not for a card
        // nobody looks at
        if self.diff.id != Some(id) {
            self.refresh_diff();
        }
        let diff = &self.diff;
        let mut open_file = None;
        egui::ScrollArea::vertical().id_salt("diff").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            cards::card(ui, theme::warn_text(), false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "DIFF", theme::warn_text());
                ui.label(
                    RichText::new(format!(
                        "`git diff HEAD` of the files the last run of #{id} changed · what the person reviews before a commit"
                    ))
                    .color(theme::dim())
                    .small(),
                );
                if let Some(fault) = &diff.fault {
                    ui.label(RichText::new(fault).color(theme::error_text()));
                } else if diff.files.is_empty() {
                    ui.label(RichText::new("no run of this card has changed a file yet").color(theme::dim()));
                }
                for (n, (file, lines)) in diff.files.iter().enumerate() {
                    ui.add_space(4.0);
                    if markdown::render_diff(ui, file, lines, n) {
                        open_file = Some(PathBuf::from(file));
                    }
                }
            });
            ui.add_space(6.0);
        });
        if let Some(path) = open_file {
            self.code = Some(code::CodePane::open(&path));
        }
    }

    /// The context tab: what a run of the selected card sends, the brief, the outlines of its
    /// files and the handoffs of its earlier runs, rendered as the notes are; the project
    /// context folded under a header, since it is the same for every card.
    fn context_view(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.38);
                ui.label(RichText::new("◆").color(cards::alpha(theme::debug(), 120)).size(40.0));
                ui.label(RichText::new("select a card").color(theme::dim()).size(16.0));
                ui.label(RichText::new("this tab shows what a run of it would send").color(cards::alpha(theme::dim(), 160)).small());
            });
            return;
        };
        // built on demand: the outlines read the card's files, so not every frame and not for
        // a card nobody looks at
        if self.context.id != Some(id) {
            self.refresh_context();
        }
        let context = &self.context;
        // an outline item clicked: the file and the line the code pane opens at
        let mut open_at: Option<(String, usize)> = None;
        egui::ScrollArea::vertical().id_salt("context").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            cards::card(ui, theme::dim(), false, |ui| {
                ui.set_width(ui.available_width());
                egui::CollapsingHeader::new(RichText::new("PROJECT CONTEXT").small().strong().color(theme::dim()))
                    .id_salt("project-context")
                    .default_open(false)
                    .show(ui, |ui| {
                        if context.project.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "no {} yet; the first run drafts it, or `rc context --init` does",
                                    ra_console::agent::PROJECT_FILE
                                ))
                                .color(theme::dim()),
                            );
                        } else {
                            ui.label(
                                RichText::new(format!("sent ahead of every task, from {}", ra_console::agent::PROJECT_FILE))
                                    .color(theme::dim())
                                    .small(),
                            );
                            markdown::render(ui, &context.project, theme::dim());
                        }
                    });
            });
            ui.add_space(6.0);
            cards::card(ui, theme::debug(), false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "TASK CONTEXT", theme::debug());
                ui.label(
                    RichText::new(format!("the brief, the outlines and the handoffs a run of #{id} sends · what `rc context {id}` prints"))
                        .color(theme::dim())
                        .small(),
                );
                match &context.fault {
                    Some(fault) => {
                        ui.label(RichText::new(fault).color(theme::error_text()));
                    }
                    None => open_at = markdown::render_prompt(ui, &context.text, theme::debug()),
                }
            });
            ui.add_space(6.0);
        });
        if let Some((file, line)) = open_at {
            let path = Path::new(&file);
            match &mut self.code {
                Some(pane) if pane.path() == path => pane.show_line(line),
                _ => self.code = Some(code::CodePane::open_at(path, line)),
            }
        }
    }

    /// The ledger tab: the usage by model with the total, the gotchas by how often they fired,
    /// and the refused commands no gotcha names, the same numbers as `rc usage` and `rc gotchas`.
    fn ledger_view(&self, ui: &mut egui::Ui) {
        let ledger = &self.ledger;
        egui::ScrollArea::vertical().id_salt("ledger").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            cards::card(ui, theme::accent(), false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "USAGE BY CARD", theme::accent());
                ui.label(
                    RichText::new("all time, summed up the roadmap: a milestone is its cards, the root is the total · the calls outside any card last")
                        .color(theme::dim())
                        .small(),
                );
                let rows = usage_by_card(&self.roadmap, &ledger.by_node);
                if rows.is_empty() {
                    ui.label(RichText::new("the ledger is empty: no completion has run yet").color(theme::dim()));
                } else {
                    let depths = tree::depths(&self.roadmap);
                    egui::Grid::new("usage-by-card").num_columns(7).spacing([22.0, 5.0]).striped(true).show(ui, |ui| {
                        ui.label(RichText::new("card").small().strong().color(theme::dim()));
                        for head in ["calls", "turns", "in", "out", "cached", "cost"] {
                            cell_right(ui, RichText::new(head).small().strong().color(theme::dim()));
                        }
                        ui.end_row();
                        for (node, t) in &rows {
                            let (label, color) = match node {
                                Some(node) => {
                                    let depth = depths.get(&node.id).copied().unwrap_or(0);
                                    // red when the subtree spent twice what its cards were sized at
                                    let color = if ran_far_over(&self.roadmap, node, t) {
                                        theme::error_text()
                                    } else if node.kind == NodeKind::Project {
                                        theme::accent()
                                    } else {
                                        theme::lane(depth)
                                    };
                                    (format!("{}#{} {}", "  ".repeat(depth), node.id, node.title), color)
                                }
                                None => ("outside the roadmap (pings, chats)".to_owned(), theme::dim()),
                            };
                            usage_row(ui, &label, t, color);
                        }
                    });
                }
            });
            ui.add_space(6.0);
            cards::card(ui, theme::accent(), false, |ui| {
                ui.set_width(ui.available_width());
                let selected = self.selected.and_then(|id| self.roadmap.iter().find(|n| n.id == id));
                let title = selected.map_or_else(|| "CALLS OF THE SELECTED CARD".to_owned(), |n| format!("CALLS OF #{} {}", n.id, n.title));
                cards::section(ui, &title, theme::accent());
                ui.label(RichText::new("each call of the card, the latest first, with the turns it took").color(theme::dim()).small());
                if selected.is_none() {
                    ui.label(RichText::new("pick a card in the roadmap column to list its calls").color(theme::dim()));
                } else if ledger.calls.is_empty() {
                    ui.label(RichText::new("no call has served this card yet").color(theme::dim()));
                } else {
                    egui::Grid::new("calls-of-card").num_columns(7).spacing([22.0, 5.0]).striped(true).show(ui, |ui| {
                        ui.label(RichText::new("when (UTC)").small().strong().color(theme::dim()));
                        ui.label(RichText::new("model").small().strong().color(theme::dim()));
                        for head in ["turns", "in", "out", "cached", "cost"] {
                            cell_right(ui, RichText::new(head).small().strong().color(theme::dim()));
                        }
                        ui.end_row();
                        for call in &ledger.calls {
                            ui.label(RichText::new(&call.at).monospace().color(theme::dim()));
                            ui.label(RichText::new(&call.model).monospace().color(theme::text()));
                            for n in [call.turns, call.prompt_tokens, call.completion_tokens, call.cached_tokens] {
                                cell_right(ui, RichText::new(n.to_string()).color(theme::text()));
                            }
                            let cost = call.cost_usd.map_or_else(|| "—".to_owned(), |c| format!("${c:.4}"));
                            cell_right(ui, RichText::new(cost).color(theme::text()));
                            ui.end_row();
                        }
                    });
                }
            });
            ui.add_space(6.0);
            cards::card(ui, theme::online(), false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "USAGE BY MODEL", theme::online());
                ui.label(
                    RichText::new("all time, from var/usage.db, the costliest first · what `rc usage` prints").color(theme::dim()).small(),
                );
                if ledger.by_model.is_empty() {
                    ui.label(RichText::new("the ledger is empty: no completion has run yet").color(theme::dim()));
                } else {
                    egui::Grid::new("usage").num_columns(7).spacing([22.0, 5.0]).striped(true).show(ui, |ui| {
                        ui.label(RichText::new("model").small().strong().color(theme::dim()));
                        for head in ["calls", "turns", "in", "out", "cached", "cost"] {
                            cell_right(ui, RichText::new(head).small().strong().color(theme::dim()));
                        }
                        ui.end_row();
                        for (model, t) in &ledger.by_model {
                            usage_row(ui, model, t, theme::text());
                        }
                        if let Some(all) = &ledger.all_time {
                            usage_row(ui, "total", all, theme::online());
                        }
                    });
                }
            });
            ui.add_space(6.0);
            cards::card(ui, theme::warn_text(), false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "GOTCHAS", theme::warn_text());
                ui.label(RichText::new("how often each fired, from var/gotchas.log · what `rc gotchas` prints").color(theme::dim()).small());
                if ledger.gotchas.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "no gotcha has fired yet; the catalogue holds {} entries in {GOTCHAS_FILE}",
                            ledger.catalogue
                        ))
                        .color(theme::dim()),
                    );
                } else {
                    egui::Grid::new("gotchas").num_columns(3).spacing([22.0, 5.0]).striped(true).show(ui, |ui| {
                        ui.label(RichText::new("gotcha").small().strong().color(theme::dim()));
                        cell_right(ui, RichText::new("hits").small().strong().color(theme::dim()));
                        ui.label(RichText::new("message").small().strong().color(theme::dim()));
                        ui.end_row();
                        for (id, count, message) in &ledger.gotchas {
                            let known = message.is_some();
                            ui.label(RichText::new(id).monospace().color(if known { theme::text() } else { theme::error_text() }));
                            cell_right(ui, RichText::new(count.to_string()).color(theme::warn_text()).strong());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(message.as_deref().unwrap_or("a refused command no gotcha names; see below"))
                                        .color(if known { theme::text() } else { theme::dim() }),
                                )
                                .wrap(),
                            );
                            ui.end_row();
                        }
                    });
                }
            });
            ui.add_space(6.0);
            if !ledger.retro.is_empty() {
                cards::card(ui, theme::debug(), false, |ui| {
                    ui.set_width(ui.available_width());
                    cards::section(ui, "FASTER NEXT TIME", theme::debug());
                    ui.label(
                        RichText::new("what the runs said themselves, newest first · the latest go to the next runs as machine notes")
                            .color(theme::dim())
                            .small(),
                    );
                    for note in &ledger.retro {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if let Some(node) = note.node {
                                ui.label(RichText::new(format!("#{node}")).color(theme::dim()).small());
                            }
                            if note.machine {
                                cards::badge(ui, "machine", theme::warn_text());
                            }
                            if note.pair {
                                cards::badge(ui, "pair", theme::accent());
                            }
                            ui.add(egui::Label::new(RichText::new(&note.text).color(theme::text())).wrap());
                        });
                    }
                });
                ui.add_space(6.0);
            }
            if !ledger.unmatched.is_empty() {
                cards::card(ui, theme::error_text(), false, |ui| {
                    ui.set_width(ui.available_width());
                    cards::section(ui, "REFUSED COMMANDS NO GOTCHA NAMES", theme::error_text());
                    let shown = ledger.unmatched.len().min(UNMATCHED_SHOWN);
                    ui.label(
                        RichText::new(format!(
                            "newest last · {shown} of {} · each is a gotcha the catalogue could grow by",
                            ledger.unmatched.len()
                        ))
                        .color(theme::dim())
                        .small(),
                    );
                    for command in ledger.unmatched.iter().rev().take(UNMATCHED_SHOWN).rev() {
                        // one line each, the whole command on hover
                        ui.add(egui::Label::new(RichText::new(command).monospace().color(theme::text())).truncate())
                            .on_hover_text(command.as_str());
                    }
                });
                ui.add_space(6.0);
            }
        });
    }

    /// The selected card, large, in the middle of the workspace.
    fn card_view(&mut self, ui: &mut egui::Ui) {
        // a card being added takes the area until it is added or dropped
        if self.new_card.is_some() {
            self.new_card_view(ui);
            return;
        }
        let Some(node) = self.selected.and_then(|id| self.roadmap.iter().find(|n| n.id == id).cloned()) else {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.38);
                ui.label(RichText::new("◆").color(cards::alpha(theme::accent(), 120)).size(40.0));
                ui.label(RichText::new("select a card").color(theme::dim()).size(16.0));
                ui.label(
                    RichText::new("hover a row for its summary · click for the whole card").color(cards::alpha(theme::dim(), 160)).small(),
                );
            });
            return;
        };
        let depth = tree::depth(&self.roadmap, node.id);
        let lane = theme::lane(depth);
        // read on selection, not every frame: the verdict is in a file
        if self.verdict.id != Some(node.id) {
            self.refresh_verdict();
        }
        let mut command: Option<String> = None;
        let nodes = self.roadmap.clone();
        // the draft lives outside the card while it draws, so the fields can hold it mutably
        let mut draft = self.draft.take().filter(|d| d.id == node.id).unwrap_or_else(|| Draft::of(&node));
        let dirty = draft.dirty(&node);
        let mut save = dirty && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S));
        let mut revert = false;
        let mut open_file: Option<PathBuf> = None;
        egui::ScrollArea::vertical().id_salt("card").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // the hero: the breadcrumb, the title in its lane, the badges
            let crumbs: Vec<String> = {
                let mut out = Vec::new();
                let mut parent = node.parent;
                while let Some(p) = parent.and_then(|id| nodes.iter().find(|n| n.id == id)) {
                    out.push(format!("{} #{} {}", cards::kind_mark(p.kind), p.id, p.title));
                    parent = p.parent;
                }
                out.reverse();
                out
            };
            if !crumbs.is_empty() {
                ui.label(RichText::new(crumbs.join("  ›  ")).color(theme::dim()).small());
            }
            cards::card(ui, lane, true, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(cards::kind_mark(node.kind)).color(lane).size(26.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(format!("#{}", node.id)).color(theme::dim()).size(18.0));
                        if dirty {
                            cards::badge(ui, "unsaved", theme::warn_text())
                                .on_hover_text("an edit the file does not have yet; Ctrl+S saves");
                        }
                        // the title edits in place, in the lane and the size the hero shows it
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.title)
                                .font(FontId::proportional(26.0))
                                .text_color(lane)
                                .desired_width(ui.available_width())
                                .frame(Frame::new())
                                .hint_text(RichText::new("a title").color(theme::dim())),
                        );
                    });
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    state_badges(ui, &nodes, &node);
                    cards::badge(ui, &node.kind.to_string(), theme::dim());
                    // a click on a dot sets the complexity; the save writes it
                    let dots = cards::priority(ui, draft.complexity).interact(egui::Sense::click());
                    if dots.clicked()
                        && let Some(pos) = dots.interact_pointer_pos()
                    {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let picked = (((pos.x - dots.rect.left()) / dots.rect.width()) * 5.0).floor() as u8 + 1;
                        draft.complexity = picked.clamp(1, 5);
                    }
                    if self.worker.budgets {
                        cards::badge_marked(ui, "$", &cards::budget_text(node.complexity), theme::warn_text())
                            .on_hover_text("the budget of a run, from the complexity; `budgets off` lifts it");
                    } else {
                        cards::badge_marked(ui, "$", "no budget", theme::dim())
                            .on_hover_text("budgets are off; `budgets on` caps a run by its card's complexity");
                    }
                    // the card's own model, else the session's: the one on the status bar
                    let (model, from) = node.assignee.as_ref().map_or_else(
                        || {
                            (
                                short(&self.worker.model).to_owned(),
                                "the session's model, from the status bar; this card names none of its own",
                            )
                        },
                        |a| (a.model.clone(), "the model this card names; a run uses it over the session's"),
                    );
                    cards::badge_marked(ui, "◉", &model, theme::trace()).on_hover_text(from);
                    // a click steps the effort up, high wrapping round to low
                    let effort = cards::badge_marked(ui, "⚡", &node.effort.to_string(), theme::warn_text())
                        .interact(egui::Sense::click())
                        .on_hover_text(format!("the effort a run asks of the model; click for {}", node.effort.next()));
                    if effort.clicked() {
                        command = Some(format!("/effort {} {}", node.id, node.effort.next()));
                    }
                    let sized_by = if node.estimate.is_some() { "the card's own line" } else { "the complexity" };
                    cards::badge_marked(ui, "≈", &estimate_of(&node).to_string(), theme::dim())
                        .on_hover_text(format!("the estimate of a run, from {sized_by}; the run line says over or under"));
                    if node.is_final() {
                        cards::badge(ui, "final", theme::warn()).on_hover_text("never reassessed");
                    }
                    if node.session.is_some() {
                        cards::badge_marked(ui, "↻", "resumes its session", theme::online());
                    }
                });
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if node.kind != NodeKind::Project && action(ui, "▶  run", theme::accent()) {
                        command = Some(format!("/run {}", node.id));
                    }
                    if action(ui, "↻  reassess", theme::warn_text()) {
                        command = Some(format!("/reassess {}", node.id));
                    }
                    if action(ui, "↻↻  reassess tree", theme::warn_text()) {
                        command = Some(format!("/reassess-tree {}", node.id));
                    }
                    if action(ui, if node.is_final() { "⊘  unpin" } else { "⊙  pin final" }, theme::warn()) {
                        command = Some(format!("/final {}{}", node.id, if node.is_final() { " off" } else { "" }));
                    }
                    if dirty {
                        ui.add_space(12.0);
                        if action(ui, "✓  save", theme::online()) {
                            save = true;
                        }
                        if action(ui, "✕  revert", theme::dim()) {
                            revert = true;
                        }
                    }
                });
            });
            ui.add_space(10.0);
            // the fields, each its own glowing card, edited in place
            edit_card(ui, "OBJECTIVE", &mut draft.objective, theme::warn_text(), "what the card is for");
            edit_card(ui, "ACCEPTANCE", &mut draft.acceptance, theme::online(), "how the check knows it is done");
            cards::card(ui, theme::debug(), false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "FILES", theme::debug());
                ui.label(RichText::new("the paths a worker reads first, one per line").color(theme::dim()).small());
                ui.add(
                    egui::TextEdit::multiline(&mut draft.files)
                        .font(TextStyle::Monospace)
                        .text_color(theme::text())
                        .desired_width(f32::INFINITY)
                        .desired_rows(2)
                        .frame(dark_field())
                        .hint_text(RichText::new("—").color(theme::dim())),
                );
                // each path as a name to click: the file opens in a pane beside the card
                let files = draft.files();
                if !files.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for file in files {
                            let open = self.code.as_ref().is_some_and(|p| p.path() == Path::new(&file));
                            let color = if open { theme::debug() } else { theme::mix(theme::text(), theme::debug(), 0.5) };
                            let label =
                                egui::Label::new(RichText::new(&file).monospace().color(color).underline()).sense(egui::Sense::click());
                            if ui.add(label).on_hover_text("open the file in a pane beside the card").clicked() {
                                open_file = Some(PathBuf::from(file));
                            }
                        }
                    });
                }
            });
            ui.add_space(6.0);
            if !node.depends_on.is_empty() {
                cards::card(ui, theme::trace(), false, |ui| {
                    ui.set_width(ui.available_width());
                    cards::section(ui, "WAITS ON", theme::trace());
                    ui.horizontal_wrapped(|ui| {
                        for dep in &node.depends_on {
                            match nodes.iter().find(|n| n.id == *dep) {
                                Some(d) => {
                                    cards::badge(ui, &format!("#{} {}", d.id, d.title), theme::state_color(d.state))
                                        .on_hover_text(d.state.to_string());
                                }
                                None => {
                                    cards::badge(ui, &format!("#{dep} (missing)"), theme::error_text());
                                }
                            }
                        }
                    });
                });
                ui.add_space(6.0);
            }
            // the cards a reassess-tree proposed under a milestone, each waiting for a word
            let proposals = if node.kind == NodeKind::Milestone { proposals_of(&self.store, node.id) } else { Vec::new() };
            if !proposals.is_empty() {
                cards::card(ui, theme::warn_text(), false, |ui| {
                    ui.set_width(ui.available_width());
                    cards::section(ui, "PROPOSED", theme::warn_text());
                    ui.label(
                        RichText::new("cards the architect found missing; accept adds one under this milestone, decline drops it for good")
                            .color(theme::dim())
                            .small(),
                    );
                    for (n, proposal) in proposals.iter().enumerate() {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if action(ui, "✓  accept", theme::online()) {
                                command = Some(format!("/accept {} {}", node.id, n + 1));
                            }
                            if action(ui, "✕  decline", theme::dim()) {
                                command = Some(format!("/decline {} {}", node.id, n + 1));
                            }
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&proposal.title).color(theme::text()).strong());
                                if !proposal.objective.is_empty() {
                                    ui.label(RichText::new(&proposal.objective).color(theme::warn_text()).small());
                                }
                                if !proposal.acceptance.is_empty() {
                                    ui.label(RichText::new(&proposal.acceptance).color(theme::online()).small());
                                }
                            });
                        });
                    }
                });
                ui.add_space(6.0);
            }
            // the reviewer's word above the handoff the notes carry: acceptance is one glance
            if let Some(verdict) = &self.verdict.text {
                let color = if verdict.starts_with("accepted") { theme::online() } else { theme::warn_text() };
                cards::card(ui, color, false, |ui| {
                    ui.set_width(ui.available_width());
                    cards::section(ui, "VERDICT", color);
                    ui.label(RichText::new(verdict).color(color));
                    ui.label(
                        RichText::new("the reviewer's verdict on the last run · `review <id>` judges it again").color(theme::dim()).small(),
                    );
                });
                ui.add_space(6.0);
            }
            if let Some(notes) = node.notes.as_deref() {
                cards::card(ui, theme::dim(), false, |ui| {
                    ui.set_width(ui.available_width());
                    cards::section(ui, "NOTES", theme::dim());
                    // the whole of the notes: the worker's report is the point of the card
                    markdown::render(ui, notes, theme::dim());
                });
                ui.add_space(6.0);
            }
            cards::card(ui, theme::trace(), false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "MODEL", theme::trace());
                ui.horizontal(|ui| {
                    // the Claude models, strongest first, and the badge's; a pick assigns at once.
                    // Most cards leave this to the agent's routing
                    let current = node.assignee.as_ref().map_or("", |a| a.model.as_str());
                    let shown = MODELS.iter().find(|(id, _)| *id == current).map_or_else(
                        || if current.is_empty() { "the badge model".to_owned() } else { current.to_owned() },
                        |(_, name)| (*name).to_owned(),
                    );
                    egui::ComboBox::from_id_salt(("model", node.id))
                        .selected_text(RichText::new(shown).color(theme::trace()))
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(current.is_empty(), "the badge model").clicked() && !current.is_empty() {
                                command = Some(format!("/assign {} none", node.id));
                            }
                            for (id, name) in MODELS {
                                if ui.selectable_label(current == id, name).on_hover_text(id).clicked() && current != id {
                                    command = Some(format!("/assign {} {id}", node.id));
                                }
                            }
                        });
                    ui.label(RichText::new("strongest first · a claude-* id runs on Claude Code").color(theme::dim()).small());
                });
                if let Some(session) = &node.session {
                    ui.label(RichText::new(format!("session {session}")).color(theme::dim()).small().monospace());
                }
            });
        });
        self.draft = if revert { None } else { Some(draft) };
        if let Some(path) = open_file {
            self.code = Some(code::CodePane::open(&path));
        }
        if save {
            self.save_draft();
        }
        if let Some(line) = command {
            self.submit(ui.ctx(), line);
        }
    }

    fn input_bar(&mut self, ui: &mut egui::Ui) {
        let completions = input::completions(&self.input);
        let mut submitted: Option<String> = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("❯").color(theme::accent()).strong().size(16.0).monospace());
            let font = FontId::monospace(14.5);
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                let mut job = input::highlight(text.as_str(), &font);
                job.wrap.max_width = wrap_width;
                ui.fonts_mut(|f| f.layout_job(job))
            };
            // a single-line edit gives its focus up on Enter before anyone can ask, so the
            // focus is read before the widget draws: that is the frame the line is submitted
            let id = egui::Id::unique("command-line");
            let had_focus = ui.memory(|m| m.has_focus(id));
            let edit = egui::TextEdit::singleline(&mut self.input)
                .id(id)
                .hint_text(
                    RichText::new("/help lists the commands · a line without / is a note for the next runs · arrows walk the roadmap, Enter runs the selected card")
                        .color(theme::dim())
                        .italics(),
                )
                .desired_width(f32::INFINITY)
                .frame(Frame::new())
                .lock_focus(true)
                .layouter(&mut layouter);
            let response = ui.add(edit);
            if had_focus || response.has_focus() {
                if self.input.is_empty() && self.browsing.is_none() {
                    // an empty line hands its keys to the roadmap: the arrows walk and fold the
                    // rows, Enter runs the selected card, Escape clears the search
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        self.search.clear();
                    }
                    self.roadmap_keys(ui.ctx(), true);
                }
                // the history: Up and Down while the line has text, Ctrl+Up and Ctrl+Down always
                let (enter, up, down, tab) = ui.input_mut(|i| {
                    (
                        i.consume_key(Modifiers::NONE, Key::Enter) || i.key_pressed(Key::Enter),
                        i.consume_key(Modifiers::NONE, Key::ArrowUp) | i.consume_key(Modifiers::COMMAND, Key::ArrowUp),
                        i.consume_key(Modifiers::NONE, Key::ArrowDown) | i.consume_key(Modifiers::COMMAND, Key::ArrowDown),
                        i.consume_key(Modifiers::NONE, Key::Tab),
                    )
                });
                if enter && !self.input.trim().is_empty() {
                    submitted = Some(std::mem::take(&mut self.input));
                    // the line keeps the keyboard after a command
                    response.request_focus();
                }
                if up && !self.history.is_empty() {
                    let i = match self.browsing {
                        None => self.history.len() - 1,
                        Some(i) => i.saturating_sub(1),
                    };
                    self.browsing = Some(i);
                    self.input = self.history[i].clone();
                }
                if down && let Some(i) = self.browsing {
                    if i + 1 < self.history.len() {
                        self.browsing = Some(i + 1);
                        self.input.clone_from(&self.history[i + 1]);
                    } else {
                        self.browsing = None;
                        self.input.clear();
                    }
                }
                if tab && completions.len() == 1 {
                    self.input = format!("{} ", completions[0]);
                }
            } else if !ui.ctx().egui_wants_keyboard_input() {
                response.request_focus();
            }
        });
        if completions.len() > 1 {
            ui.label(RichText::new(completions.join("  ")).color(theme::dim()).small());
        }
        if let Some(line) = submitted {
            self.submit(ui.ctx(), line);
        }
    }

    /// The command palette over the window: the query filters the commands and the cards, Up
    /// and Down walk the rows, Enter takes the one under the cursor. A command with what it
    /// needs after it runs at once; one still wanting its arguments goes to the command line;
    /// a card is selected.
    fn palette_view(&mut self, ctx: &egui::Context) {
        let Some(palette) = &mut self.palette else { return };
        let items = input::items(&palette.query, &self.roadmap);
        palette.cursor = palette.cursor.min(items.len().saturating_sub(1));
        // the keys go before the edit draws, so the edit never sees the arrows or the Enter
        let (up, down, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if up {
            palette.cursor = palette.cursor.saturating_sub(1);
        }
        if down && palette.cursor + 1 < items.len() {
            palette.cursor += 1;
        }
        let mut picked: Option<usize> = if enter { Some(palette.cursor) } else { None };
        let mut close = escape;
        let id = egui::Id::unique("palette-query");
        let cursor = palette.cursor;
        egui::Area::new(egui::Id::unique("palette"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::panel_raised())
                    .stroke(Stroke::new(1.0, theme::accent()))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(560.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("❯").color(theme::accent()).strong().size(16.0).monospace());
                            let edit = egui::TextEdit::singleline(&mut palette.query)
                                .id(id)
                                .hint_text(RichText::new("a command, a card's title or id · Esc closes").color(theme::dim()).italics())
                                .desired_width(f32::INFINITY)
                                .frame(Frame::new())
                                .font(TextStyle::Monospace);
                            let response = ui.add(edit);
                            // the edit takes the keyboard as the palette opens and keeps it
                            // until Enter or Escape; a click elsewhere closes the palette
                            if !ui.memory(|m| m.has_focus(id)) && !response.lost_focus() {
                                response.request_focus();
                            } else if response.lost_focus() && !enter && !escape {
                                close = true;
                            }
                        });
                        ui.separator();
                        if items.is_empty() {
                            ui.label(RichText::new("nothing matches").color(theme::dim()).italics());
                        }
                        egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                            for (i, item) in items.iter().enumerate() {
                                if palette_row(ui, item, i == cursor) {
                                    picked = Some(i);
                                }
                            }
                        });
                    });
            });
        let query = palette.query.clone();
        if let Some(i) = picked
            && let Some(item) = items.get(i)
        {
            match item {
                input::Item::Command { name, usage, .. } => {
                    // what follows the command word in the query is its arguments
                    let rest = query.trim().trim_start_matches(['/', '.']);
                    let args = rest.split_once(char::is_whitespace).map_or("", |(_, a)| a.trim());
                    if args.is_empty() && !usage.is_empty() {
                        self.input = format!("/{name} ");
                        self.browsing = None;
                    } else {
                        let line = if args.is_empty() { format!("/{name}") } else { format!("/{name} {args}") };
                        self.submit(ctx, line);
                    }
                }
                input::Item::Card { id, .. } => {
                    self.select(Some(*id));
                    self.scroll_to = Some(*id);
                }
            }
        }
        if close || picked.is_some() {
            self.palette = None;
        }
    }
}

/// One row of the palette; true when it is clicked. A command shows its name, its usage dim
/// and its key at the right; a card its number and title.
fn palette_row(ui: &mut egui::Ui, item: &input::Item, under_cursor: bool) -> bool {
    let fill = if under_cursor { theme::rule() } else { Color32::TRANSPARENT };
    let response = Frame::new()
        .fill(fill)
        .corner_radius(4)
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| match item {
                input::Item::Command { name, usage, key } => {
                    ui.label(RichText::new(format!("/{name}")).color(theme::accent()).monospace());
                    if !usage.is_empty() {
                        ui.label(RichText::new(*usage).color(theme::dim()).monospace());
                    }
                    if let Some(key) = key {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(*key).color(theme::warn_text()).small());
                        });
                    }
                }
                input::Item::Card { id, title } => {
                    ui.label(RichText::new(format!("#{id}")).color(theme::warn_text()).monospace());
                    ui.label(RichText::new(title).color(theme::text()));
                }
            });
        })
        .response;
    response.interact(egui::Sense::click()).clicked()
}

impl Workspace {
    /// One frame of the workspace: the panels of the project, in the room the window gives
    /// it under the tab strip.
    fn draw(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        self.tick(&ctx);
        if self.started.elapsed() < SPLASH || self.fault.is_some() {
            self.splash(root);
            return;
        }
        // the window's own keys, before any panel can take them: Ctrl+K the palette, Ctrl+L
        // and Ctrl+D the two commands the palette shows those keys for, Ctrl+= Ctrl+- Ctrl+0
        // the zoom
        let (palette, clear, debug, zoom_in, zoom_out, zoom_reset) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::K),
                i.consume_key(Modifiers::COMMAND, Key::L),
                i.consume_key(Modifiers::COMMAND, Key::D),
                i.consume_key(Modifiers::COMMAND, Key::Equals) | i.consume_key(Modifiers::COMMAND, Key::Plus),
                i.consume_key(Modifiers::COMMAND, Key::Minus),
                i.consume_key(Modifiers::COMMAND, Key::Num0),
            )
        });
        if zoom_in {
            self.set_zoom(&ctx, self.zoom + ZOOM_STEP);
        }
        if zoom_out {
            self.set_zoom(&ctx, self.zoom - ZOOM_STEP);
        }
        if zoom_reset {
            self.set_zoom(&ctx, 1.0);
        }
        if palette && self.setup.is_none() {
            self.palette = if self.palette.is_some() { None } else { Some(input::Palette::default()) };
        }
        if clear {
            self.submit(&ctx, "/clear".to_owned());
        }
        if debug {
            self.submit(&ctx, "/debug".to_owned());
        }
        egui::Panel::top("status")
            .frame(Frame::new().fill(theme::panel()).inner_margin(Margin::symmetric(8, 6)))
            .show(root, |ui| self.status_bar(ui));
        // the roadmap takes the whole height at the left, the width its rows need up to two
        // fifths of the window; the console and the command line sit right of it
        if self.setup.is_none() {
            // the measured width opens the column; a drag on its edge sets it after that
            let cost = if self.show_cost { self.cost_width.unwrap_or(0.0) } else { 0.0 };
            let width = (self.roadmap_width + cost).min(root.max_rect().width() * 0.4);
            egui::Panel::left("roadmap")
                .resizable(true)
                .default_size(width)
                .min_size(260.0)
                .max_size(root.max_rect().width() * 0.6)
                .frame(Frame::new().fill(theme::bg()).inner_margin(Margin::symmetric(8, 8)).stroke(Stroke::new(1.0, theme::rule())))
                .show(root, |ui| self.roadmap_panel(ui));
        }
        // the command line at the bottom, the console right above it
        egui::Panel::bottom("input")
            .frame(Frame::new().fill(theme::panel()).inner_margin(Margin::symmetric(12, 8)).stroke(Stroke::new(1.0, theme::rule())))
            .show(root, |ui| self.input_bar(ui));
        // the console takes the height of the last session, or two fifths of the window to
        // begin with; the card view scrolls in the rest, and the edge between them drags from
        // a floor to most of the window
        let height = ctx.content_rect().height();
        let opening = if self.console_height > 0.0 { self.console_height } else { height * 0.4 };
        let band = egui::Panel::bottom("console")
            .resizable(true)
            .default_size(opening.clamp(160.0, height * 0.8))
            .min_size(160.0)
            .max_size(height * 0.8)
            .frame(Frame::new().fill(theme::bg()).inner_margin(Margin::symmetric(12, 6)))
            .show(root, |ui| {
                ui.label(RichText::new("CONSOLE").small().strong().color(theme::dim()));
                egui::ScrollArea::vertical().id_salt("console").auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                    console::draw(ui, &self.sink, self.show_debug);
                });
            });
        self.console_height = band.response.rect.height();
        self.save_layout();
        if self.setup.is_some() {
            // no roadmap yet: the choice takes the place of the roadmap and the card
            egui::CentralPanel::default()
                .frame(Frame::new().fill(theme::bg()).inner_margin(Margin::symmetric(18, 12)))
                .show(root, |ui| self.setup_view(ui));
            ctx.request_repaint_after(Duration::from_millis(if self.job.is_some() { 60 } else { 250 }));
            return;
        }
        // a file open from the card takes a pane at its right, above the console; closing
        // its tab gives the card the width back
        if self.code.is_some() {
            let mut close = false;
            egui::Panel::right("code")
                .resizable(true)
                .default_size(root.max_rect().width() * 0.42)
                .frame(Frame::new().fill(theme::panel()).inner_margin(Margin::symmetric(12, 10)).stroke(Stroke::new(1.0, theme::rule())))
                .show(root, |ui| {
                    if let Some(pane) = &mut self.code {
                        close = pane.draw(ui);
                    }
                });
            if close {
                self.code = None;
            }
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::bg()).inner_margin(Margin::symmetric(18, 12)))
            .show(root, |ui| self.center(ui));
        self.toasts_view(&ctx);
        self.confirm_pending(&ctx);
        self.palette_view(&ctx);
        let live = self.sink.live_run().is_some() || self.job.is_some();
        ctx.request_repaint_after(Duration::from_millis(if live { 40 } else { 250 }));
    }
}

impl Workspace {
    /// Starts the walk in this project as a detached `rc run --all`, and says so in the log;
    /// its cards' starts and ends arrive through the activity log like any run's.
    fn start_walk(&mut self, parallel: Option<u32>) {
        match ra_console::agent::spawn_walk(&self.root, self.worker.debug, parallel) {
            Ok(pid) => self.sink.push(
                Kind::Output,
                match parallel.filter(|n| *n > 1) {
                    Some(n) => format!("walk started (pid {pid}): {n} ready cards at once, each in its own worktree; /stop ends it after the cards it is in"),
                    None => format!("walk started (pid {pid}): the ready cards run one after another; /stop ends it after the card it is in"),
                },
            ),
            Err(e) => self.sink.push(Kind::Error, format!("{e:#}")),
        }
    }

    /// The project's card on the dashboard: its name and state in the header; at the left the
    /// card a run is on, then the next ready cards, in the roadmap's own rows; at the right the
    /// latest run's card, live or finished. Returns true when the header asks for the tab.
    #[allow(clippy::too_many_lines)]
    /// `budget` is the height the card grows to: the queue adds rows past its six while they
    /// fit and then takes the rest of the share as its band, and the run card beside it is
    /// exactly as tall; the two fill the share together, equally.
    fn dashboard_card(&mut self, ui: &mut egui::Ui, lane: Color32, page: f32, budget: f32) -> bool {
        let nodes = self.roadmap.clone();
        let order = ra_console::agent::walk_order(&nodes, &self.gambits, ra_console::agent::limits_tight(&self.limits));
        let live = self.sink.live_run();
        // the card the live run is on: its id leads the run's title
        let running: Option<i64> = live.and_then(|id| {
            self.sink
                .with_lines(|lines| lines.iter().rev().find(|l| l.run == Some(id) && l.kind == Kind::Run).and_then(|l| card_id_in(&l.text)))
        });
        let name = self
            .project_title
            .clone()
            .unwrap_or_else(|| self.root.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()));
        let dir = self.root.display().to_string();
        let model = short(&self.worker.model).to_owned();
        let job = self.job.clone();
        // the rows read as in the column: the running card first, then the next ready ones
        let mut rows: Vec<i64> = running.into_iter().collect();
        rows.extend(order.iter().copied().filter(|id| Some(*id) != running).take(if running.is_some() { 5 } else { 6 }));
        // the ready cards past the six: the queue adds them one by one while the card's share
        // of the window has room for another row
        let extra: Vec<i64> = order.iter().copied().filter(|id| Some(*id) != running && !rows.contains(id)).collect();
        let walk = Walk {
            searching: false,
            matched: HashSet::new(),
            holds_shown: HashSet::new(),
            hidden: HashSet::new(),
            folded: HashSet::new(),
            selected: self.selected,
            scroll_to: None,
            depths: tree::depths(&nodes),
            last_changes: self.last_changes.clone(),
            now: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            costs: None,
            cost_width: 0.0,
            dragging: None,
            shaking: None,
        };
        let mut out = Walked::default();
        let mut open = false;
        cards::bubble(ui, lane, live.is_some(), |ui| {
            // the bubble's inner width: the page less its margins and its stroke, two of each
            let inner = (page - 18.0).max(300.0);
            ui.set_width(inner);
            ui.set_max_width(inner);
            ui.horizontal(|ui| {
                ui.set_width(inner);
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(&name).size(15.5).strong().color(theme::text()));
                ui.label(RichText::new(&dir).small().color(theme::dim()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let button = egui::Button::new(RichText::new("open ›").color(theme::text()))
                        .fill(cards::alpha(lane, 40))
                        .stroke(Stroke::new(1.0, cards::alpha(lane, 160)))
                        .corner_radius(CornerRadius::same(7));
                    if ui.add(button).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("the project's tab").clicked() {
                        open = true;
                    }
                    cards::badge(ui, &model, theme::accent());
                    let ready = format!("{} ready", order.len());
                    cards::badge(ui, &ready, if order.is_empty() { theme::dim() } else { theme::online() });
                    match (running, &job) {
                        (Some(id), _) => {
                            cards::badge(ui, &format!("running #{id}"), theme::warn_text());
                        }
                        (None, Some(job)) => {
                            cards::badge(ui, job, theme::warn_text());
                        }
                        (None, None) => {
                            cards::badge(ui, "idle", theme::dim());
                        }
                    }
                });
            });
            ui.add_space(6.0);
            let total = inner;
            let left = (total * 0.42).clamp(280.0, 620.0);
            // the right column takes what is left of the page, never more: a run card sets its
            // width to what it is given, and an open-ended column would grow past the edge
            let right = (total - left - 12.0).max(200.0);
            // the height the two columns may take: the card's share of the window less what
            // the header above them used, the bubble's own margins and stroke (6 and 3
            // above and below, 1 of stroke: 20) and the spacing under the card; zero when
            // the window has no spare room, and then the columns are what they always were
            let columns = (budget - ui.min_rect().height() - 20.0 - 10.0).max(0.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let queue = ui.allocate_ui_with_layout(egui::vec2(left, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(left);
                    if rows.is_empty() {
                        ui.label(RichText::new("no card is ready").color(theme::dim()).italics());
                    }
                    let mut shown = rows.clone();
                    let mut extra = extra.iter().copied();
                    let mut rank = 0;
                    while rank < shown.len() {
                        let id = shown[rank];
                        let Some(node) = nodes.iter().find(|n| n.id == id) else {
                            rank += 1;
                            continue;
                        };
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let mark = if Some(id) == running {
                                "▶".to_owned()
                            } else {
                                format!("{:>2}", rank + usize::from(running.is_none()))
                            };
                            ui.label(RichText::new(mark).monospace().color(theme::warn_text()).strong());
                            ui.vertical(|ui| {
                                // the row is held to the column, so a long title truncates
                                // instead of running under the badges
                                ui.set_max_width(ui.available_width());
                                let row_lane = theme::lane(walk.depths.get(&node.id).copied().unwrap_or(0));
                                header_row(ui, &nodes, node, &[], row_lane, &walk, &mut out);
                            });
                        });
                        rank += 1;
                        // past the six, one more ready card while a row of the height the
                        // rows so far averaged still fits the card's share
                        if rank == shown.len() {
                            let used = ui.min_rect().height();
                            let row = used / rank as f32;
                            if used + row <= columns
                                && let Some(next) = extra.next()
                            {
                                shown.push(next);
                            }
                        }
                    }
                    // the queue takes the whole share, rows or not: the band under its last
                    // row is the column's, so the run card beside it is as tall as the share
                    let used = ui.min_rect().height();
                    if columns > used {
                        ui.allocate_space(egui::vec2(left, columns - used));
                    }
                });
                // the run card is exactly the queue's height: the two columns grow together
                // into the card's share of the window, and never past it
                let room = queue.response.rect.height();
                ui.allocate_ui_with_layout(egui::vec2(right, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(right);
                    ui.set_max_width(right);
                    if !console::draw_latest_run(ui, &self.sink, room) {
                        ui.label(RichText::new("no run in this session yet").color(theme::dim()).italics());
                    }
                });
            });
        });
        if let Some(id) = out.clicked {
            self.select(Some(id));
            open = true;
        }
        if let Some(line) = out.command {
            self.submit(ui.ctx(), line);
        }
        if let Some((id, state)) = out.set_state {
            self.set_state(id, state);
        }
        open
    }
}

/// The card id a run's title leads with (`#43 …`), when it has one.
fn card_id_in(text: &str) -> Option<i64> {
    let at = text.find('#')?;
    let digits: String = text[at + 1..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.save_layout();
        ra_console::instance::release(&self.root);
    }
}

/// The width the roadmap's rows need: the indent, the marks, the title and the badges of the
/// widest row, every row counted whether folded or not and with room for its buttons, so the
/// width is a property of the roadmap and not of what is picked.
fn measure_roadmap(ctx: &egui::Context, nodes: &[Node]) -> f32 {
    let depths = tree::depths(nodes);
    let mut widest: f32 = 320.0;
    for node in nodes {
        let depth = depths.get(&node.id).copied().unwrap_or(0) as f32;
        let size = if matches!(node.kind, NodeKind::Project | NodeKind::Milestone) { 15.0 } else { 14.5 };
        let title = ctx.fonts_mut(|f| f.layout_no_wrap(node.title.clone(), FontId::proportional(size), theme::text())).size().x;
        // the arrow, the mark and the id in front; the state, the dots, the model, the final
        // mark and the two buttons behind
        let badges = 220.0 + if node.assignee.is_some() { 110.0 } else { 0.0 } + if node.is_final() { 50.0 } else { 0.0 };
        widest = widest.max(depth * 16.0 + 86.0 + title + badges);
    }
    widest + 24.0
}

/// What the roadmap column's walk reads this frame: the search, the chips and the structure
/// they keep, the folds, the selection, and the depth of every node.
struct Walk {
    searching: bool,
    matched: HashSet<i64>,
    holds_shown: HashSet<i64>,
    hidden: HashSet<TaskState>,
    folded: HashSet<i64>,
    selected: Option<i64>,
    scroll_to: Option<i64>,
    depths: HashMap<i64, usize>,
    /// The last state change of every card, and the time the ages count from.
    last_changes: HashMap<i64, Change>,
    now: u64,
    /// The cost of every subtree while the cost column is on, `None` while it is off.
    costs: Option<HashMap<i64, Totals>>,
    /// The width the cost column takes, the same on every row.
    cost_width: f32,
    /// The row being dragged, so the others light as the pointer passes over them.
    dragging: Option<i64>,
    /// The row shaking off a refused drop, and how long it has been shaking.
    shaking: Option<(i64, f32)>,
}

/// How long a refused drop shakes its row, in seconds.
const SHAKE_FOR: f32 = 0.45;

/// Where a drag over a row would drop: onto it (last child), or just above or below it (a
/// sibling under its parent). The band near a row's top or bottom edge is between; the rest is onto.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DropSlot {
    Before,
    Onto,
    After,
}

/// What the walk brings back: the rows shown top to bottom (what the arrow keys walk), and
/// what the pointer did.
#[derive(Default)]
struct Walked {
    shown: Vec<i64>,
    clicked: Option<i64>,
    toggled: Option<i64>,
    command: Option<String>,
    /// A state picked from a row's menu, written through the store.
    set_state: Option<(i64, TaskState)>,
    /// The row whose tooltip is open, so the rest of the column can step back.
    tooltip_row: Option<egui::Rect>,
    /// The milestone or section whose `+` was clicked: the new-card form opens under it.
    add_under: Option<i64>,
    /// The row the pointer began dragging this frame.
    drag_started: Option<i64>,
    /// The row under the dragged pointer and where on it, this frame.
    drop: Option<(i64, DropSlot)>,
}

impl Walk {
    /// Whether a node shows at all: the search keeps its matches and their ancestors; a hidden
    /// chip takes its nodes unless a shown card sits below; the project always shows.
    fn shows(&self, node: &Node) -> bool {
        if self.searching && !self.matched.contains(&node.id) {
            return false;
        }
        !(self.hidden.contains(&node.state) && node.kind != NodeKind::Project && !self.holds_shown.contains(&node.id))
    }

    /// Whether a node's children are folded away; a search opens everything.
    fn is_folded(&self, id: i64) -> bool {
        !self.searching && self.folded.contains(&id)
    }
}

/// One node of the roadmap column. A project, a milestone or a section is a bubble: its header
/// row, then its children inside, the bubbles of its phases parted by a glowing rule. A task is a
/// row; its subtasks, when it has any, sit in a small bubble under it.
fn draw_bubble(ui: &mut egui::Ui, nodes: &[Node], node: &Node, walk: &Walk, out: &mut Walked) {
    if !walk.shows(node) {
        return;
    }
    out.shown.push(node.id);
    let depth = walk.depths.get(&node.id).copied().unwrap_or(0);
    let lane = theme::lane(depth);
    let children: Vec<&Node> = nodes.iter().filter(|n| n.parent == Some(node.id)).collect();
    let folded = walk.is_folded(node.id);
    if matches!(node.kind, NodeKind::Task | NodeKind::Subtask) {
        header_row(ui, nodes, node, &children, lane, walk, out);
        if !folded && children.iter().any(|c| walk.shows(c)) {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    cards::bubble(ui, theme::lane(depth + 1), false, |ui| {
                        for child in &children {
                            draw_bubble(ui, nodes, child, walk, out);
                        }
                    });
                });
            });
        }
        return;
    }
    // a bubble is lit while it or one of its own rows is the selection
    let lit = walk.selected == Some(node.id) || children.iter().any(|c| walk.selected == Some(c.id));
    cards::bubble(ui, lane, lit, |ui| {
        header_row(ui, nodes, node, &children, lane, walk, out);
        if folded {
            return;
        }
        let mut first = true;
        for child in &children {
            if !walk.shows(child) {
                continue;
            }
            // a rule between the bubbles, not before the first and never between two rows
            if !first && !matches!(child.kind, NodeKind::Task | NodeKind::Subtask) {
                cards::divider(ui, theme::lane(depth + 1));
            }
            first = false;
            draw_bubble(ui, nodes, child, walk, out);
        }
    });
}

/// The header row of a bubble, or the row of a task: the fold arrow, the kind's mark, the id and
/// the title; at the right the badges, the progress of a bubble, and the run and reassess
/// buttons of the selected card. The row selects on click and folds on double click.
fn header_row(ui: &mut egui::Ui, nodes: &[Node], node: &Node, children: &[&Node], lane: Color32, walk: &Walk, out: &mut Walked) {
    let has_children = !children.is_empty();
    let selected = walk.selected == Some(node.id);
    let row_id = ui.make_persistent_id(("row", node.id));
    let hovered = ui.ctx().read_response(row_id).is_some_and(|r| r.hovered());
    let fill = if selected {
        cards::alpha(lane, 44)
    } else if hovered {
        cards::alpha(lane, 18)
    } else {
        Color32::TRANSPARENT
    };
    // a refused drop shakes the row: its content jitters sideways under a red edge for a moment
    let shake = walk.shaking.filter(|(id, _)| *id == node.id).map(|(_, t)| 3.0 + 3.0 * (t * 60.0).sin());
    let dragged = walk.dragging == Some(node.id);
    let stroke = if shake.is_some() {
        Stroke::new(1.0, theme::error_text())
    } else if selected {
        Stroke::new(1.0, cards::alpha(lane, 200))
    } else {
        Stroke::NONE
    };
    let frame = Frame::new().fill(fill).stroke(stroke).corner_radius(CornerRadius::same(7)).inner_margin(Margin::symmetric(6, 3));
    // the row's jewel: the state's color bright at the right, where its badge sits, fading
    // leftward under the title; the slot is filled once the row has its rect
    let painter = ui.painter().clone();
    let slot = painter.add(egui::Shape::Noop);
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        if dragged {
            ui.multiply_opacity(0.4);
        }
        ui.horizontal(|ui| {
            if let Some(dx) = shake {
                ui.add_space(dx);
            }
            let arrow = if !has_children {
                "  "
            } else if walk.is_folded(node.id) {
                "▸"
            } else {
                "▾"
            };
            if ui.add(egui::Label::new(RichText::new(arrow).color(theme::dim())).sense(egui::Sense::click())).clicked() && has_children {
                out.toggled = Some(node.id);
            }
            ui.label(RichText::new(cards::kind_mark(node.kind)).color(lane));
            ui.label(RichText::new(format!("#{}", node.id)).color(theme::dim()).small());
            let title = RichText::new(&node.title).color(if selected { theme::text() } else { theme::mix(theme::text(), lane, 0.45) });
            let title = match node.kind {
                NodeKind::Project => title.strong().size(16.0),
                NodeKind::Milestone => title.strong().size(15.0),
                NodeKind::Section => title.strong(),
                _ => title,
            };
            // a column dragged narrow clips the title to its room with an ellipsis, rather than
            // wrapping the row or running under the badges; the tooltip has the whole of it.
            // The room keeps what the badges at the right take, so a long title stops short
            // of them in a column that was not measured for it (the dashboard's)
            let reserve = 200.0
                + walk.cost_width
                + if node.assignee.is_some() { 110.0 } else { 0.0 }
                + if node.is_final() { 50.0 } else { 0.0 }
                + if selected { 70.0 } else { 0.0 };
            let room = (ui.available_width() - reserve).max(80.0);
            ui.scope(|ui| {
                ui.set_max_width(room);
                ui.add(egui::Label::new(title).truncate());
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                cost_cell(ui, nodes, node, walk);
                state_badges(ui, nodes, node);
                if node.is_final() {
                    cards::badge(ui, "final", theme::warn());
                }
                if let Some(a) = &node.assignee {
                    cards::badge_marked(ui, "◉", short(&a.model), theme::trace()).on_hover_text(&a.model);
                }
                if matches!(node.kind, NodeKind::Task | NodeKind::Subtask) {
                    cards::priority(ui, node.complexity);
                } else {
                    // the progress of a bubble: the cards accepted of the cards under it
                    let cards_under: Vec<&Node> = tree::subtree(nodes, node.id)
                        .into_iter()
                        .filter(|n| matches!(n.kind, NodeKind::Task | NodeKind::Subtask))
                        .collect();
                    if !cards_under.is_empty() {
                        let done = cards_under.iter().filter(|n| n.state == TaskState::Accepted).count();
                        let color = if done == cards_under.len() { theme::online() } else { theme::dim() };
                        ui.label(RichText::new(format!("{done}/{}", cards_under.len())).small().color(color))
                            .on_hover_text("cards accepted of the cards under it");
                    }
                }
                if selected && node.kind != NodeKind::Project {
                    if ui.small_button(RichText::new("▶").color(theme::accent())).on_hover_text("run this card").clicked() {
                        out.command = Some(format!("/run {}", node.id));
                    }
                    if ui.small_button(RichText::new("↻").color(theme::warn_text())).on_hover_text("reassess this card").clicked() {
                        out.command = Some(format!("/reassess {}", node.id));
                    }
                }
                add_button(ui, node, row_id, out);
            });
        });
    });
    let row = inner.response.rect;
    let glow_color = theme::state_color(node.state);
    painter.set(
        slot,
        cards::fade_shape(Rect::from_min_max(egui::pos2(row.left() + row.width() * 0.35, row.top()), row.max), glow_color, 46, true),
    );
    // the row's own id, so the next frame reads this frame's hover for the fill
    let response = ui.interact(inner.response.rect, row_id, egui::Sense::click_and_drag());
    if response.drag_started() && node.kind != NodeKind::Project {
        out.drag_started = Some(node.id);
    }
    // while another row is dragged, this one lights where the pointer would drop it: a filled
    // frame to take it as a child, a line along its top or bottom edge to take it as a sibling;
    // the project has no siblings, so it only takes children
    if let Some(dragging) = walk.dragging
        && dragging != node.id
        && let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
        && row.contains(pointer)
    {
        let band = (row.height() * 0.25).min(8.0);
        let slot = if node.kind == NodeKind::Project {
            DropSlot::Onto
        } else if pointer.y < row.top() + band {
            DropSlot::Before
        } else if pointer.y > row.bottom() - band {
            DropSlot::After
        } else {
            DropSlot::Onto
        };
        let accent = theme::accent();
        match slot {
            DropSlot::Onto => {
                painter.rect(row, CornerRadius::same(7), cards::alpha(accent, 40), Stroke::new(1.5, accent), egui::StrokeKind::Inside);
            }
            DropSlot::Before => {
                painter.hline(row.x_range(), row.top(), Stroke::new(2.0, accent));
            }
            DropSlot::After => {
                painter.hline(row.x_range(), row.bottom(), Stroke::new(2.0, accent));
            }
        }
        out.drop = Some((node.id, slot));
    }
    if egui::Tooltip::was_tooltip_open_last_frame(ui.ctx(), response.id) {
        out.tooltip_row = Some(response.rect);
    }
    let response = response.on_hover_ui(|ui| cards::tooltip(ui, node, lane, walk.last_changes.get(&node.id), walk.now));
    // a right click opens the row's menu: the twelve states, then what the card view offers
    response.context_menu(|ui| {
        if let Some(pick) = cards::row_menu(ui, node) {
            apply_pick(ui.ctx(), node, pick, out);
        }
    });
    if walk.scroll_to == Some(node.id) {
        response.scroll_to_me(None);
    }
    if response.clicked() {
        out.clicked = Some(node.id);
    }
    if response.double_clicked() && has_children {
        out.toggled = Some(node.id);
    }
}

/// The cost column's cell at the right end of a row, while the column is on: the subtree's cost
/// from the ledger, right-aligned in the measured width; dim when nothing was spent, red when
/// the subtree ran far over its estimate.
fn cost_cell(ui: &mut egui::Ui, nodes: &[Node], node: &Node, walk: &Walk) {
    let Some(costs) = &walk.costs else { return };
    let spent = costs.get(&node.id).copied().unwrap_or_default();
    let (text, color) = if spent.calls == 0 {
        ("—".to_owned(), theme::dim())
    } else if ran_far_over(nodes, node, &spent) {
        (dollars(spent.cost_usd), theme::error_text())
    } else {
        (dollars(spent.cost_usd), theme::text())
    };
    let size = egui::vec2(walk.cost_width, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
        ui.label(RichText::new(text).small().color(color)).on_hover_text("the cost of this card and the cards under it");
    });
}

/// The width the cost column needs: the widest cost of any row as the cell prints it, the dash
/// of an empty one at least.
fn measure_costs(ui: &egui::Ui, costs: &HashMap<i64, Totals>) -> f32 {
    let font = TextStyle::Small.resolve(ui.style());
    let width = |text: String| ui.fonts_mut(|f| f.layout_no_wrap(text, font.clone(), theme::text())).size().x;
    let widest = costs.values().filter(|t| t.calls > 0).map(|t| width(dollars(t.cost_usd))).fold(width("—".to_owned()), f32::max);
    widest + 8.0
}

/// The `+` at the right of a hovered milestone or section row: a click opens the new-card form
/// under it. The row's last response says whether the pointer is on it, its buttons included.
fn add_button(ui: &mut egui::Ui, node: &Node, row_id: egui::Id, out: &mut Walked) {
    let pointed = ui.ctx().read_response(row_id).is_some_and(|r| r.contains_pointer());
    if pointed
        && matches!(node.kind, NodeKind::Milestone | NodeKind::Section)
        && ui.small_button(RichText::new("+").color(theme::online()).strong()).on_hover_text("add a card under this one").clicked()
    {
        out.add_under = Some(node.id);
    }
}

/// The fields of the new-card form, each as the card view draws its own, under the parent it
/// goes to; what the keys and the buttons asked for comes back.
fn new_card_form(ui: &mut egui::Ui, form: &mut NewCard, lane: Color32, under: &str) -> FormAct {
    ui.set_width(ui.available_width());
    ui.label(RichText::new(format!("{under}  ›  a new card")).color(theme::dim()).small());
    let mut act = FormAct::Stay;
    // whether the caret sits in a multiline field, where Enter is a newline and Ctrl+Enter adds
    let mut in_multiline = false;
    cards::card(ui, lane, true, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(cards::kind_mark(NodeKind::Task)).color(lane).size(26.0));
            let title = ui.add(
                egui::TextEdit::singleline(&mut form.title)
                    .font(FontId::proportional(26.0))
                    .text_color(lane)
                    .desired_width(ui.available_width())
                    .frame(Frame::new())
                    .hint_text(RichText::new("a title").color(theme::dim())),
            );
            if !form.focused {
                title.request_focus();
                form.focused = true;
            }
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            cards::badge(ui, "defined", theme::state_color(TaskState::Defined));
            cards::badge(ui, "task", theme::dim());
            // a click on a dot sets the complexity, as on the card view
            let dots = cards::priority(ui, form.complexity).interact(egui::Sense::click());
            if dots.clicked()
                && let Some(pos) = dots.interact_pointer_pos()
            {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let picked = (((pos.x - dots.rect.left()) / dots.rect.width()) * 5.0).floor() as u8 + 1;
                form.complexity = picked.clamp(1, 5);
            }
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            if action(ui, "✓  add", theme::online()) {
                act = FormAct::Add;
            }
            if action(ui, "✕  cancel", theme::dim()) {
                act = FormAct::Cancel;
            }
            ui.label(RichText::new("Enter adds · Escape cancels").color(theme::dim()).small());
        });
        if let Some(fault) = &form.fault {
            ui.label(RichText::new(fault).color(theme::error_text()).small());
        }
    });
    ui.add_space(10.0);
    in_multiline |= edit_card(ui, "OBJECTIVE", &mut form.objective, theme::warn_text(), "what the card is for");
    in_multiline |= edit_card(ui, "ACCEPTANCE", &mut form.acceptance, theme::online(), "how the check knows it is done");
    in_multiline |= edit_card(ui, "FILES", &mut form.files, theme::debug(), "the paths a worker reads first, one per line");
    cards::card(ui, theme::trace(), false, |ui| {
        ui.set_width(ui.available_width());
        cards::section(ui, "WAITS ON", theme::trace());
        ui.add(
            egui::TextEdit::singleline(&mut form.waits_on)
                .font(TextStyle::Monospace)
                .text_color(theme::text())
                .desired_width(f32::INFINITY)
                .hint_text(RichText::new("the ids of the cards this one waits on, #12 #15").color(theme::dim()).italics()),
        );
    });
    let (enter, escape) = ui.input_mut(|i| {
        let enter = i.consume_key(Modifiers::COMMAND, Key::Enter) || (!in_multiline && i.consume_key(Modifiers::NONE, Key::Enter));
        (enter, i.consume_key(Modifiers::NONE, Key::Escape))
    });
    if enter {
        act = FormAct::Add;
    }
    if escape {
        act = FormAct::Cancel;
    }
    act
}

/// What a row's menu pick comes to: a state for the store, a command for the console, or the
/// id on the clipboard.
fn apply_pick(ctx: &egui::Context, node: &Node, pick: cards::RowPick, out: &mut Walked) {
    match pick {
        cards::RowPick::State(state) => out.set_state = Some((node.id, state)),
        cards::RowPick::Run => out.command = Some(format!("/run {}", node.id)),
        cards::RowPick::Reassess => out.command = Some(format!("/reassess {}", node.id)),
        cards::RowPick::PinFinal => {
            out.command = Some(format!("/final {}{}", node.id, if node.is_final() { " off" } else { "" }));
        }
        cards::RowPick::CopyId => ctx.copy_text(format!("#{}", node.id)),
    }
}

/// The window's title: the project's name (the roadmap's root once it is read, the directory
/// before that), the directory when the two differ, and the build — so a window per project
/// is told from the next in the task bar.
fn window_title(project: Option<&str>, root: &Path) -> String {
    let dir = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match project {
        // a project named after the agent itself (this repo) reads once, not twice
        Some(name) if !name.is_empty() && name != dir && name != "Rhipper Agent" => {
            format!("{name} ({dir}) — Rhipper Agent {}", ra_console::VERSION)
        }
        _ => format!("{dir} — Rhipper Agent {}", ra_console::VERSION),
    }
}

/// A command's answer into the log: the plain lines as one bubble, so they read as one
/// Markdown block, and each diff (a line with lines inside it) as a bubble of its own.
fn push_answer(sink: &Sink, out: Vec<String>) {
    let (diffs, plain): (Vec<String>, Vec<String>) = out.into_iter().partition(|l| l.contains('\n'));
    if !plain.is_empty() {
        sink.push(Kind::Output, plain.join("\n"));
    }
    for diff in diffs {
        sink.push(Kind::Diff, diff);
    }
}

/// The rows a search keeps: every node whose title or id contains `needle` (lowercase, trimmed),
/// and every ancestor of one, so a hit keeps its place in the tree. A `#` in front of a number
/// is allowed, the way ids are written.
fn search_matches(nodes: &[Node], needle: &str) -> HashSet<i64> {
    let id_part = needle.trim_start_matches('#');
    let mut kept = HashSet::new();
    for node in nodes {
        let hit = node.title.to_lowercase().contains(needle) || (!id_part.is_empty() && node.id.to_string().contains(id_part));
        if !hit {
            continue;
        }
        let mut current = Some(node.id);
        while let Some(id) = current {
            if !kept.insert(id) {
                break;
            }
            current = nodes.iter().find(|n| n.id == id).and_then(|n| n.parent);
        }
    }
    kept
}

/// A field of the card as its own glowing card, titled in `color`, its text edited in place;
/// `hint` shows while the field is empty. Says whether the field holds the caret.
fn edit_card(ui: &mut egui::Ui, name: &str, value: &mut String, color: Color32, hint: &str) -> bool {
    let focused = cards::card(ui, color, false, |ui| {
        ui.set_width(ui.available_width());
        cards::section(ui, name, color);
        // the text on the dark field the code lines use, apart from the card around it
        ui.add(
            egui::TextEdit::multiline(value)
                .font(FontId::proportional(15.0))
                .text_color(theme::text())
                .desired_width(f32::INFINITY)
                .desired_rows(2)
                .frame(dark_field())
                .hint_text(RichText::new(hint).color(theme::dim()).italics()),
        )
        .has_focus()
    })
    .inner;
    ui.add_space(6.0);
    focused
}

/// The field a block of text sits on inside a card: darker than the card, the code lines' own,
/// so the text reads apart from the frame around it.
fn dark_field() -> Frame {
    Frame::new()
        .fill(theme::mix(theme::bg(), Color32::BLACK, 0.25))
        .stroke(Stroke::new(1.0, theme::rule()))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 6))
}

/// A cell of a grid with its text against the right edge, the way numbers line up.
fn cell_right(ui: &mut egui::Ui, text: RichText) {
    // one text line tall: a layout given the cell's available height would take the rest of
    // the panel and stretch every row of the grid with it
    let height = ui.text_style_height(&TextStyle::Body);
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), height), Layout::right_to_left(Align::Center), |ui| {
        ui.label(text);
    });
}

/// One row of the usage grid: the model, the calls, the tokens and the cost, the numbers as
/// `rc usage` prints them.
/// The ledger by card: every node whose subtree has calls, in tree order, with the sums of its
/// subtree (so a milestone is its cards and the root is the total), then the calls that served
/// no node. Empty while the ledger is.
fn usage_by_card<'a>(nodes: &'a [Node], by_node: &[(Option<i64>, Totals)]) -> Vec<(Option<&'a Node>, Totals)> {
    let own = |id: i64| by_node.iter().find(|(n, _)| *n == Some(id)).map(|(_, t)| *t);
    let mut rows: Vec<(Option<&Node>, Totals)> = Vec::new();
    for node in nodes {
        let mut sum = Totals::default();
        for member in tree::subtree(nodes, node.id) {
            if let Some(t) = own(member.id) {
                sum.merge(&t);
            }
        }
        if sum.calls > 0 {
            rows.push((Some(node), sum));
        }
    }
    // a call booked to a node the roadmap no longer has still counts at the root
    let root_has = rows.first().is_some_and(|(n, _)| n.is_some_and(|n| n.kind == NodeKind::Project));
    let stray: Totals = by_node.iter().filter(|(id, _)| id.is_some_and(|id| !nodes.iter().any(|n| n.id == id))).fold(
        Totals::default(),
        |mut acc, (_, t)| {
            acc.merge(t);
            acc
        },
    );
    if root_has && stray.calls > 0 {
        rows[0].1.merge(&stray);
    }
    if let Some((_, t)) = by_node.iter().find(|(n, _)| n.is_none()) {
        rows.push((None, *t));
    }
    rows
}

/// The ledger by card summed up the tree for the cost column: every node's subtree. The project
/// takes the whole ledger, the calls booked to a node the roadmap lost and the calls that served
/// none included, so the root row reads as the ledger's total.
fn subtree_costs(nodes: &[Node], by_node: &[(Option<i64>, Totals)]) -> HashMap<i64, Totals> {
    let mut costs = HashMap::new();
    for node in nodes {
        let mut sum = Totals::default();
        if node.kind == NodeKind::Project {
            for (_, t) in by_node {
                sum.merge(t);
            }
        } else {
            for member in tree::subtree(nodes, node.id) {
                if let Some((_, t)) = by_node.iter().find(|(n, _)| *n == Some(member.id)) {
                    sum.merge(t);
                }
            }
        }
        costs.insert(node.id, sum);
    }
    costs
}

/// Whether the subtree of `node` spent more than twice its estimate, in tokens or in turns. The
/// estimate of a subtree is the sum of its tasks', a card's own line or its complexity's; a node
/// without a task under it is held to its own.
fn ran_far_over(nodes: &[Node], node: &Node, spent: &Totals) -> bool {
    let tasks: Vec<&Node> = tree::subtree(nodes, node.id).into_iter().filter(|n| n.kind == NodeKind::Task).collect();
    let estimates = if tasks.is_empty() { vec![estimate_of(node)] } else { tasks.into_iter().map(estimate_of).collect() };
    let tokens: u64 = estimates.iter().map(|e| e.tokens).sum();
    let turns: u64 = estimates.iter().map(|e| e.turns).sum();
    spent.prompt_tokens + spent.completion_tokens > 2 * tokens || spent.turns > 2 * turns
}

fn usage_row(ui: &mut egui::Ui, model: &str, t: &Totals, color: Color32) {
    ui.label(RichText::new(model).monospace().color(color));
    for n in [t.calls, t.turns, t.prompt_tokens, t.completion_tokens, t.cached_tokens] {
        cell_right(ui, RichText::new(n.to_string()).color(color));
    }
    let cost = t.cost_usd.map_or_else(|| "—".to_owned(), |c| format!("${c:.4}"));
    cell_right(ui, RichText::new(cost).color(color));
    ui.end_row();
}

/// An action button in `color`: tinted, with a stroke, bright on hover.
fn action(ui: &mut egui::Ui, text: &str, color: Color32) -> bool {
    let button = egui::Button::new(RichText::new(text).color(color).strong())
        .fill(cards::alpha(color, 26))
        .stroke(Stroke::new(1.0, cards::alpha(color, 150)))
        .corner_radius(CornerRadius::same(8));
    ui.add(button).clicked()
}

/// The task the workers hold next: the first in progress, else the ready card the walk scores
/// highest, the one `rc next` names, else the first defined task.
fn current_task<'a>(nodes: &'a [Node], gambits: &'a [Gambit]) -> Option<&'a Node> {
    nodes
        .iter()
        .find(|n| matches!(n.state, TaskState::InProgress | TaskState::Assigned))
        .or_else(|| tree::next_ready(nodes, gambits))
        .or_else(|| nodes.iter().find(|n| n.state == TaskState::Defined && n.kind != NodeKind::Project))
}

/// The state badge of a card: the state computed from its waits-on, and when that differs from
/// the stored one, the stored one dimmed beside it. Nothing is written to the file either way.
fn state_badges(ui: &mut egui::Ui, nodes: &[Node], node: &Node) {
    let computed = tree::effective_state(nodes, node);
    cards::badge(ui, &computed.to_string(), theme::state_color(computed));
    if computed != node.state {
        cards::badge(ui, &node.state.to_string(), theme::dim())
            .on_hover_text(format!("stored as {}; {computed} is read from its waits-on, not written", node.state));
    }
}

fn short(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

/// The short hash and first line of the last three commits of `root`, newest first; none when
/// git does not run or the directory is no checkout, and the splash shows the build alone.
fn last_commits(root: &Path) -> Vec<String> {
    std::process::Command::new("git")
        .args(["log", "-3", "--format=%h %s"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The output of `git diff` split by file, in the order of `files`: the lines under the
/// `diff --git a/<file> b/<file>` header of each, none for a file git did not report.
fn split_diff(output: &str, files: &[String]) -> Vec<(String, Vec<String>)> {
    let mut sections: HashMap<String, Vec<String>> = HashMap::new();
    let mut current: Option<String> = None;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            // the new side names a renamed file; the old side is what the handoff knew
            let name = rest.split(" b/").next().unwrap_or(rest).to_owned();
            current = Some(name);
        }
        if let Some(name) = &current {
            sections.entry(name.clone()).or_default().push(line.to_owned());
        }
    }
    files.iter().map(|f| (f.clone(), sections.remove(f).unwrap_or_default())).collect()
}

fn compact(count: u64) -> String {
    match count {
        0..=999 => count.to_string(),
        1_000..=999_999 => format!("{:.1}k", count as f64 / 1_000.0),
        _ => format!("{:.1}M", count as f64 / 1_000_000.0),
    }
}

fn dollars(cost: Option<f64>) -> String {
    match cost {
        None => "$—".to_owned(),
        Some(c) if c < 0.01 && c > 0.0 => "<$0.01".to_owned(),
        Some(c) => format!("${c:.2}"),
    }
}

#[allow(dead_code)]
fn _keep(_: PathBuf) {}
