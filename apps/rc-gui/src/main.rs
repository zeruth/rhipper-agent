//! The Rhipper Agent workspace: an IntelliJ-style window over the roadmap and the workers, in
//! the Blood Moon theme. The roadmap tool window down the left, the selected card in the middle,
//! the console across the bottom above the command line; a splash while it loads. It drives the
//! same store, worker, commands and log as the terminal console, and takes its place: one of
//! them runs at a time.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod cards;
mod console;
mod fonts;
mod input;
mod markdown;
mod theme;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;
use egui::{Align, Color32, CornerRadius, FontId, Frame, Key, Layout, Margin, Modifiers, RichText, Stroke, TextStyle};
use ra_console::agent::Worker;
use ra_console::commands::{self, Outcome};
use ra_console::logs::{Kind, Sink};
use ra_gotchas::{Catalogue, Journal};
use ra_roadmap::model::{Node, NodeKind, TaskState};
use ra_roadmap::store::Store;
use ra_roadmap::tree;
use ra_usage::{Totals, UsageStore};
use tokio::sync::mpsc;

/// How long the splash stays at least.
const SPLASH: Duration = Duration::from_millis(1800);
/// The catalogue of the gotchas and the journal of the ones that fired, as `rc gotchas` reads them.
const GOTCHAS_FILE: &str = "var/gotchas.toml";
const GOTCHAS_LOG: &str = "var/gotchas.log";
/// How many of the refused commands the ledger tab lists, the same as `rc gotchas`.
const UNMATCHED_SHOWN: usize = 20;

/// The Claude models a card can be assigned from the card view, strongest first.
const MODELS: [(&str, &str); 3] =
    [("claude-fable-5-1", "Fable 5.1"), ("claude-opus-5-5", "Opus 5.5"), ("claude-haiku-4-5-20251001", "Haiku 4.5")];

/// The three tabs of the card area.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Card,
    Context,
    Ledger,
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
    /// Each gotcha that fired: the id, how often, and its message from the catalogue, `None`
    /// for an id the catalogue does not hold.
    gotchas: Vec<(String, usize, Option<String>)>,
    /// The refused commands no gotcha names, oldest first.
    unmatched: Vec<String>,
    /// How many entries the catalogue holds; what the empty view says.
    catalogue: usize,
    read_at: Option<Instant>,
}

fn main() -> eframe::Result<()> {
    let icon = image::load_from_memory(include_bytes!("../assets/icon.png")).ok().map(|img| {
        let rgba = img.to_rgba8();
        egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1500.0, 940.0])
        .with_min_inner_size([980.0, 620.0])
        .with_maximized(true)
        .with_title(format!("Rhipper Agent {}", ra_console::VERSION));
    if let Some(icon) = icon {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Rhipper Agent", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

struct App {
    rt: tokio::runtime::Runtime,
    store: Store,
    worker: Worker,
    sink: Sink,
    started: Instant,
    /// The roadmap as last read, and the mtime it came from.
    roadmap: Vec<Node>,
    roadmap_seen: Option<SystemTime>,
    /// The width the roadmap's rows need, measured once per read of the file.
    roadmap_width: f32,
    selected: Option<i64>,
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
    context: ContextView,
    tab: Tab,
    /// The plan's windows as the harness last reported them, and when they were last read.
    limits: ra_providers::limits::Limits,
    limits_read: Instant,
    /// The week's utilization the pill bar last aimed at, and when it last moved, for the glow.
    week_target: f32,
    week_moved: Option<Instant>,
    show_debug: bool,
    /// What went wrong at start, shown on the splash instead of the workspace.
    fault: Option<String>,
    /// The choice a directory without a roadmap gets, until it has one.
    setup: Option<Setup>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        fonts::install(&cc.egui_ctx);
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
            style.visuals.popup_shadow =
                egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(160) };
        });

        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build().expect("a tokio runtime");
        let sink = Sink::new();
        let _ = std::fs::create_dir_all("var");
        let mut fault = None;
        if let Ok(log) = std::fs::OpenOptions::new().create(true).append(true).open("var/console.log") {
            ra_console::trace::init(log, Some(sink.clone()), None);
        }
        // one instance at a time: a console or an older window gives way to this one
        let replaced = ra_console::instance::stop_running();
        if let Err(e) = ra_console::instance::claim() {
            tracing::warn!("cannot claim the instance: {e:#}");
        }
        // the directory the window starts in is the project; without a roadmap the window
        // offers to make one rather than writing a bare file on the quiet
        let root = Path::new(".");
        let mut setup = None;
        let temp_store =
            || rt.block_on(Store::open(&std::env::temp_dir().join("rhipper-agent-fault.md"), "Rhipper Agent")).expect("a temp store");
        let store = if ra_console::setup::exists(root) {
            match rt.block_on(Store::open(Path::new(ra_console::setup::ROADMAP_FILE), "Rhipper Agent")) {
                Ok(store) => store,
                Err(e) => {
                    fault = Some(format!("{e:#}"));
                    // a store that cannot open still needs a value; the splash shows the fault
                    temp_store()
                }
            }
        } else {
            setup = Some(Setup::new(root));
            temp_store()
        };
        let mut worker = Worker::new(ra_console::ui::DEFAULT_MODEL);
        match rt.block_on(UsageStore::open(Path::new("var/usage.db"))) {
            Ok(ledger) => worker = worker.with_ledger(ledger),
            Err(e) => tracing::warn!("no ledger: {e:#}"),
        }
        sink.push(Kind::Banner, format!("Rhipper Agent {} — the workspace. Type help for commands.", ra_console::VERSION));
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
            roadmap_seen: None,
            roadmap_width: 360.0,
            selected: None,
            followed: None,
            hidden_states: HashSet::new(),
            folded: HashSet::new(),
            search: String::new(),
            visible_rows: Vec::new(),
            scroll_to: None,
            input: String::new(),
            history: Vec::new(),
            browsing: None,
            activity_offset: std::fs::metadata(ra_console::ui::ACTIVITY_FILE).map_or(0, |m| m.len()),
            job: None,
            job_tx,
            job_done,
            ledger: Ledger::default(),
            context: ContextView::default(),
            tab: Tab::Card,
            limits: ra_providers::limits::Limits::load(Path::new(ra_providers::claude_code::LIMITS_FILE)),
            limits_read: Instant::now(),
            week_target: 0.0,
            week_moved: None,
            show_debug: false,
            fault,
            setup,
        };
        app.refresh_ledger();
        app
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
        worker.model.clone_from(&setup.model);
        let (sink, tx) = (self.sink.clone(), self.job_tx.clone());
        self.rt.spawn(async move {
            match ra_console::setup::init(Path::new("."), &worker, mode).await {
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
        if ra_console::setup::exists(Path::new("."))
            && let Ok(store) = self.rt.block_on(Store::open(Path::new(ra_console::setup::ROADMAP_FILE), "Rhipper Agent"))
        {
            self.store = store;
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
            cards::card(ui, theme::ACCENT, true, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("No roadmap in this directory").font(FontId::proportional(26.0)).color(theme::ACCENT).strong());
                ui.label(RichText::new(&dir).monospace().color(theme::TEXT));
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "The directory a window starts in is its project; this one has no {} yet. Choose how it begins; the file is yours to edit after.",
                        ra_console::setup::ROADMAP_FILE
                    ))
                    .color(theme::DIM),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("model").small().strong().color(theme::DIM));
                    if let Some(setup) = &mut self.setup {
                        ui.add(egui::TextEdit::singleline(&mut setup.model).desired_width(240.0).font(TextStyle::Monospace));
                    }
                    ui.label(RichText::new("the strongest once per project; a claude-* id runs on Claude Code").color(theme::DIM).small());
                });
                ui.add_space(10.0);
                if let Some(what) = &running {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().color(theme::ACCENT));
                        ui.label(RichText::new(format!("{what} …")).color(theme::WARN_TEXT));
                    });
                    return;
                }
                cards::section(ui, "DRAFT", theme::ACCENT);
                ui.label(RichText::new("the model reads the README, the layout, the manifest and the documents, and answers with milestones and cards").color(theme::DIM).small());
                if action(ui, "▶  draft a roadmap from this directory", theme::ACCENT) {
                    picked = Some(ra_console::setup::Mode::Draft);
                }
                ui.add_space(8.0);
                if !candidates.is_empty() {
                    cards::section(ui, "IMPORT", theme::WARN_TEXT);
                    ui.label(RichText::new("a roadmap-shaped file found here, converted by the model with every item kept").color(theme::DIM).small());
                    for file in &candidates {
                        if action(ui, &format!("⇥  import {}", file.display().to_string().replace('\\', "/")), theme::WARN_TEXT) {
                            picked = Some(ra_console::setup::Mode::Import(file.clone()));
                        }
                    }
                    ui.add_space(8.0);
                }
                cards::section(ui, "EMPTY", theme::DIM);
                ui.label(RichText::new("the bare project and nothing else; no model call").color(theme::DIM).small());
                if action(ui, "○  start empty", theme::DIM) {
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
        let (all_time, by_model, by_node) =
            self.rt.block_on(async { (self.worker.all_time().await, self.worker.by_model().await, self.worker.by_node().await) });
        let catalogue = match Catalogue::load(Path::new(GOTCHAS_FILE)) {
            Ok(catalogue) => Some(catalogue),
            Err(e) => {
                tracing::warn!("cannot read the gotchas catalogue: {e:#}");
                None
            }
        };
        let journal = Journal::at(Path::new(GOTCHAS_LOG));
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
            gotchas,
            unmatched: journal.unmatched(),
            catalogue: catalogue.map_or(0, |c| c.entries.len()),
            read_at: Some(Instant::now()),
        };
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

    /// The housekeeping of a frame: the activity of other processes, the roadmap file, the jobs,
    /// and a request to quit from another instance.
    fn tick(&mut self, ctx: &egui::Context) {
        if ra_console::instance::quit_requested() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ra_console::ui::tail_activity(&mut self.activity_offset, &self.sink);
        let modified = self.store.modified();
        if modified != self.roadmap_seen {
            self.roadmap_seen = modified;
            match self.rt.block_on(self.store.all()) {
                Ok(nodes) => {
                    self.roadmap_width = measure_roadmap(ctx, &nodes);
                    self.roadmap = nodes;
                }
                Err(e) => tracing::warn!("the roadmap file does not parse: {e:#}"),
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
            }
            self.followed = working;
        }
        while self.job_done.try_recv().is_ok() {
            self.job = None;
            self.refresh_ledger();
            self.context.id = None;
            if self.setup.as_ref().is_some_and(|s| s.running.is_some()) {
                self.finish_setup();
            }
        }
        // the plan's windows, every five seconds: a run of any instance may have moved them
        if self.limits_read.elapsed() >= Duration::from_secs(5) {
            self.limits = ra_providers::limits::Limits::load(Path::new(ra_providers::claude_code::LIMITS_FILE));
            self.limits_read = Instant::now();
            // the week's pill aims at the new value and glows for a moment when it moved
            #[allow(clippy::cast_possible_truncation)]
            let target = self.limits.windows.get("seven_day").map_or(0.0, |w| w.utilization as f32);
            if (target - self.week_target).abs() > 0.0005 {
                self.week_target = target;
                self.week_moved = Some(Instant::now());
            }
        }
    }

    /// Picks a card; the model field of the card view follows.
    fn select(&mut self, id: Option<i64>) {
        self.selected = id;
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
            let line = format!("run {}", node.id);
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
        if matches!(name.as_str(), "run" | "reassess" | "reassess-tree" | "ping") {
            let arg = line.split_whitespace().nth(1).unwrap_or("").to_owned();
            self.job = Some(match name.as_str() {
                "run" => format!("running #{arg}"),
                "ping" => "pinging the model".to_owned(),
                other => format!("{other} #{arg}"),
            });
            let (store, sink, worker, tx) = (self.store.clone(), self.sink.clone(), self.worker.clone(), self.job_tx.clone());
            self.rt.spawn(async move {
                match commands::run(&line, &store, &worker).await {
                    Ok((out, _)) => {
                        for l in out {
                            let kind = if l.contains('\n') { Kind::Diff } else { Kind::Output };
                            sink.push(kind, l);
                        }
                    }
                    Err(e) => sink.push(Kind::Error, format!("{e:#}")),
                }
                let _ = tx.send(());
            });
            return;
        }
        match self.rt.block_on(commands::run(&line, &self.store, &self.worker)) {
            Ok((out, outcome)) => {
                for l in out {
                    self.sink.push(Kind::Output, l);
                }
                match outcome {
                    Outcome::Continue => {}
                    Outcome::Clear => self.sink.clear(),
                    Outcome::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    Outcome::Model(model) => self.worker.model = model,
                    Outcome::Budgets(on) => self.worker.budgets = on,
                    Outcome::ToggleDebug => {
                        self.show_debug = !self.show_debug;
                        self.sink.push(Kind::Output, format!("trace lines {}", if self.show_debug { "on" } else { "off" }));
                    }
                }
            }
            Err(e) => self.sink.push(Kind::Error, format!("{e:#}")),
        }
    }

    fn splash(&self, root: &mut egui::Ui) {
        egui::CentralPanel::default().frame(Frame::new().fill(theme::BG)).show(root, |ui| {
            let avail = ui.available_rect_before_wrap();
            // the picture keeps 16:10 and fills what it can, centered
            let scale = (avail.width() / 640.0).min(avail.height() / 400.0);
            let size = egui::vec2(640.0 * scale, 400.0 * scale);
            let rect = egui::Rect::from_center_size(avail.center(), size);
            egui::Image::new(egui::include_image!("../assets/splash.svg")).paint_at(ui, rect);
            let painter = ui.painter();
            let unit = scale;
            painter.text(
                rect.left_bottom() + egui::vec2(40.0 * unit, -34.0 * unit),
                egui::Align2::LEFT_BOTTOM,
                "Rhipper",
                FontId::proportional(44.0 * unit),
                Color32::from_rgb(255, 215, 208),
            );
            let mut x = rect.left() + 43.0 * unit;
            for ch in "AGENT".chars() {
                painter.text(
                    egui::pos2(x, rect.bottom() - 14.0 * unit),
                    egui::Align2::LEFT_BOTTOM,
                    ch,
                    FontId::proportional(12.5 * unit),
                    Color32::from_rgb(229, 115, 115),
                );
                x += 11.0 * unit;
            }
            let right = rect.right_bottom() + egui::vec2(-32.0 * unit, -28.0 * unit);
            painter.text(
                right,
                egui::Align2::RIGHT_BOTTOM,
                format!("Rhipper Agent build {}", ra_console::VERSION),
                FontId::proportional(12.0 * unit),
                Color32::from_rgb(255, 171, 145),
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
                Color32::from_rgb(229, 115, 115),
            );
            let progress = (self.started.elapsed().as_secs_f32() / SPLASH.as_secs_f32()).min(1.0);
            let bar = egui::Rect::from_min_size(
                rect.left_bottom() - egui::vec2(0.0, 3.0 * unit),
                egui::vec2(rect.width() * progress, 3.0 * unit),
            );
            painter.rect_filled(bar, 0.0, theme::ACCENT);
        });
        root.ctx().request_repaint_after(Duration::from_millis(16));
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(" ✦ Rhipper").color(theme::ACCENT).strong().size(15.0));
            ui.label(RichText::new("Agent").strong().size(15.0));
            ui.label(RichText::new(ra_console::VERSION).color(theme::DIM).small());
            ui.add_space(10.0);
            cards::badge_marked(ui, "◉", &self.worker.model, theme::ACCENT)
                .on_hover_text("the model of the next run; `model <id>` swaps it");
            match current_task(&self.roadmap) {
                Some(node) => {
                    cards::badge_marked(ui, "◇", &format!("#{} {}", node.id, node.title), theme::TRACE)
                        .on_hover_text("the task the workers hold next");
                }
                None => {
                    cards::badge_marked(ui, "◇", "no task queued", theme::DIM);
                }
            }
            if let Some(job) = &self.job {
                let t = ui.input(|i| i.time);
                let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][((t * 10.0) as usize) % 10];
                cards::badge_marked(ui, spinner, job, theme::WARN_TEXT);
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
                theme::WARN_TEXT,
            )
            .on_hover_text("this session's tokens and cost");
            match &self.ledger.all_time {
                Some(all) => {
                    cards::badge_marked(ui, "◉", &format!("ledger {} · {} calls", dollars(all.cost_usd), all.calls), theme::ONLINE)
                        .on_hover_text("all time, from var/usage.db");
                }
                None => {
                    cards::badge_marked(ui, "◌", "no ledger", theme::DIM);
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
                    theme::ERROR_TEXT
                } else if target >= 0.5 {
                    theme::WARN_TEXT
                } else {
                    theme::ONLINE
                };
                cards::pill_bar(ui, &format!("week {:.0}%", shown * 100.0), shown, color, pulse).on_hover_text(format!(
                    "the plan's week: {:.0}% used, resets in {}\nfrom the harness's last run",
                    week.utilization * 100.0,
                    ra_providers::limits::until(week.resets_at)
                ));
            }
            let secs = self.started.elapsed().as_secs();
            ui.label(RichText::new(format!("up {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)).color(theme::DIM).small());
        });
    }

    fn roadmap_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("ROADMAP").small().strong().color(theme::DIM));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let count = if self.search.trim().is_empty() {
                    format!("{} cards", self.roadmap.len())
                } else {
                    format!("{} of {} cards", self.visible_rows.len(), self.roadmap.len())
                };
                ui.label(RichText::new(count).small().color(theme::DIM));
            });
        });
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
            ui.label(RichText::new("⌕").color(theme::DIM).size(16.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.search.is_empty()
                    && ui.small_button(RichText::new("×").color(theme::DIM)).on_hover_text("clear the search (Escape)").clicked()
                {
                    self.search.clear();
                }
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text(RichText::new("search by title or id").color(theme::DIM).italics())
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
        };
        let mut out = Walked::default();
        let scroll = egui::ScrollArea::vertical().id_salt("roadmap").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for root in nodes.iter().filter(|n| n.parent.is_none()) {
                draw_bubble(ui, &nodes, root, &walk, &mut out);
            }
        });
        // while a tooltip is open the rest of the column steps back: a shade over everything
        // but the row it belongs to; the tooltip itself is on its own layer above
        if let Some(row) = out.tooltip_row {
            let area = scroll.inner_rect;
            let shade = cards::alpha(theme::BG, 150);
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
    }

    /// The middle of the workspace: the tab row, then the selected card or the ledger.
    fn center(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if cards::tab(ui, "CARD", theme::ACCENT, self.tab == Tab::Card) {
                self.tab = Tab::Card;
            }
            if cards::tab(ui, "CONTEXT", theme::DEBUG, self.tab == Tab::Context) {
                self.tab = Tab::Context;
            }
            if cards::tab(ui, "LEDGER", theme::ONLINE, self.tab == Tab::Ledger) {
                self.tab = Tab::Ledger;
            }
            if self.tab == Tab::Context {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button(RichText::new("↻").color(theme::DEBUG))
                        .on_hover_text("build the context again from the card and its files")
                        .clicked()
                    {
                        self.context.id = None;
                    }
                    if let Some(at) = self.context.read_at.filter(|_| self.context.id.is_some()) {
                        let secs = at.elapsed().as_secs();
                        let when = if secs < 5 { "just now".to_owned() } else { format!("{secs}s ago") };
                        ui.label(RichText::new(format!("built {when} · refreshed when a run ends")).color(theme::DIM).small());
                    }
                });
            }
            if self.tab == Tab::Ledger {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button(RichText::new("↻").color(theme::ONLINE))
                        .on_hover_text("read the ledger and the journal again")
                        .clicked()
                    {
                        self.refresh_ledger();
                    }
                    if let Some(at) = self.ledger.read_at {
                        let secs = at.elapsed().as_secs();
                        let when = if secs < 5 { "just now".to_owned() } else { format!("{secs}s ago") };
                        ui.label(RichText::new(format!("read {when} · refreshed when a run ends")).color(theme::DIM).small());
                    }
                });
            }
        });
        ui.add_space(6.0);
        match self.tab {
            Tab::Card => self.card_view(ui),
            Tab::Context => self.context_view(ui),
            Tab::Ledger => self.ledger_view(ui),
        }
    }

    /// The context tab: what a run of the selected card sends, the brief, the outlines of its
    /// files and the handoffs of its earlier runs, rendered as the notes are; the project
    /// context folded under a header, since it is the same for every card.
    fn context_view(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.38);
                ui.label(RichText::new("◆").color(cards::alpha(theme::DEBUG, 120)).size(40.0));
                ui.label(RichText::new("select a card").color(theme::DIM).size(16.0));
                ui.label(RichText::new("this tab shows what a run of it would send").color(cards::alpha(theme::DIM, 160)).small());
            });
            return;
        };
        // built on demand: the outlines read the card's files, so not every frame and not for
        // a card nobody looks at
        if self.context.id != Some(id) {
            self.refresh_context();
        }
        let context = &self.context;
        egui::ScrollArea::vertical().id_salt("context").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            cards::card(ui, theme::DIM, false, |ui| {
                ui.set_width(ui.available_width());
                egui::CollapsingHeader::new(RichText::new("PROJECT CONTEXT").small().strong().color(theme::DIM))
                    .id_salt("project-context")
                    .default_open(false)
                    .show(ui, |ui| {
                        if context.project.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "no {} yet; the first run drafts it, or `rc context --init` does",
                                    ra_console::agent::PROJECT_FILE
                                ))
                                .color(theme::DIM),
                            );
                        } else {
                            ui.label(
                                RichText::new(format!("sent ahead of every task, from {}", ra_console::agent::PROJECT_FILE))
                                    .color(theme::DIM)
                                    .small(),
                            );
                            markdown::render(ui, &context.project, theme::DIM);
                        }
                    });
            });
            ui.add_space(6.0);
            cards::card(ui, theme::DEBUG, false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "TASK CONTEXT", theme::DEBUG);
                ui.label(
                    RichText::new(format!("the brief, the outlines and the handoffs a run of #{id} sends · what `rc context {id}` prints"))
                        .color(theme::DIM)
                        .small(),
                );
                match &context.fault {
                    Some(fault) => {
                        ui.label(RichText::new(fault).color(theme::ERROR_TEXT));
                    }
                    None => markdown::render_prompt(ui, &context.text, theme::DEBUG),
                }
            });
            ui.add_space(6.0);
        });
    }

    /// The ledger tab: the usage by model with the total, the gotchas by how often they fired,
    /// and the refused commands no gotcha names, the same numbers as `rc usage` and `rc gotchas`.
    fn ledger_view(&self, ui: &mut egui::Ui) {
        let ledger = &self.ledger;
        egui::ScrollArea::vertical().id_salt("ledger").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            cards::card(ui, theme::ACCENT, false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "USAGE BY CARD", theme::ACCENT);
                ui.label(
                    RichText::new("all time, summed up the roadmap: a milestone is its cards, the root is the total · the calls outside any card last")
                        .color(theme::DIM)
                        .small(),
                );
                let rows = usage_by_card(&self.roadmap, &ledger.by_node);
                if rows.is_empty() {
                    ui.label(RichText::new("the ledger is empty: no completion has run yet").color(theme::DIM));
                } else {
                    let depths = tree::depths(&self.roadmap);
                    egui::Grid::new("usage-by-card").num_columns(6).spacing([22.0, 5.0]).striped(true).show(ui, |ui| {
                        ui.label(RichText::new("card").small().strong().color(theme::DIM));
                        for head in ["calls", "in", "out", "cached", "cost"] {
                            cell_right(ui, RichText::new(head).small().strong().color(theme::DIM));
                        }
                        ui.end_row();
                        for (node, t) in &rows {
                            let (label, color) = match node {
                                Some(node) => {
                                    let depth = depths.get(&node.id).copied().unwrap_or(0);
                                    let color = if node.kind == NodeKind::Project { theme::ACCENT } else { theme::lane(depth) };
                                    (format!("{}#{} {}", "  ".repeat(depth), node.id, node.title), color)
                                }
                                None => ("outside the roadmap (pings, chats)".to_owned(), theme::DIM),
                            };
                            usage_row(ui, &label, t, color);
                        }
                    });
                }
            });
            ui.add_space(6.0);
            cards::card(ui, theme::ONLINE, false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "USAGE BY MODEL", theme::ONLINE);
                ui.label(
                    RichText::new("all time, from var/usage.db, the costliest first · what `rc usage` prints").color(theme::DIM).small(),
                );
                if ledger.by_model.is_empty() {
                    ui.label(RichText::new("the ledger is empty: no completion has run yet").color(theme::DIM));
                } else {
                    egui::Grid::new("usage").num_columns(6).spacing([22.0, 5.0]).striped(true).show(ui, |ui| {
                        ui.label(RichText::new("model").small().strong().color(theme::DIM));
                        for head in ["calls", "in", "out", "cached", "cost"] {
                            cell_right(ui, RichText::new(head).small().strong().color(theme::DIM));
                        }
                        ui.end_row();
                        for (model, t) in &ledger.by_model {
                            usage_row(ui, model, t, theme::TEXT);
                        }
                        if let Some(all) = &ledger.all_time {
                            usage_row(ui, "total", all, theme::ONLINE);
                        }
                    });
                }
            });
            ui.add_space(6.0);
            cards::card(ui, theme::WARN_TEXT, false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "GOTCHAS", theme::WARN_TEXT);
                ui.label(RichText::new("how often each fired, from var/gotchas.log · what `rc gotchas` prints").color(theme::DIM).small());
                if ledger.gotchas.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "no gotcha has fired yet; the catalogue holds {} entries in {GOTCHAS_FILE}",
                            ledger.catalogue
                        ))
                        .color(theme::DIM),
                    );
                } else {
                    egui::Grid::new("gotchas").num_columns(3).spacing([22.0, 5.0]).striped(true).show(ui, |ui| {
                        ui.label(RichText::new("gotcha").small().strong().color(theme::DIM));
                        cell_right(ui, RichText::new("hits").small().strong().color(theme::DIM));
                        ui.label(RichText::new("message").small().strong().color(theme::DIM));
                        ui.end_row();
                        for (id, count, message) in &ledger.gotchas {
                            let known = message.is_some();
                            ui.label(RichText::new(id).monospace().color(if known { theme::TEXT } else { theme::ERROR_TEXT }));
                            cell_right(ui, RichText::new(count.to_string()).color(theme::WARN_TEXT).strong());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(message.as_deref().unwrap_or("a refused command no gotcha names; see below"))
                                        .color(if known { theme::TEXT } else { theme::DIM }),
                                )
                                .wrap(),
                            );
                            ui.end_row();
                        }
                    });
                }
            });
            ui.add_space(6.0);
            if !ledger.unmatched.is_empty() {
                cards::card(ui, theme::ERROR_TEXT, false, |ui| {
                    ui.set_width(ui.available_width());
                    cards::section(ui, "REFUSED COMMANDS NO GOTCHA NAMES", theme::ERROR_TEXT);
                    let shown = ledger.unmatched.len().min(UNMATCHED_SHOWN);
                    ui.label(
                        RichText::new(format!(
                            "newest last · {shown} of {} · each is a gotcha the catalogue could grow by",
                            ledger.unmatched.len()
                        ))
                        .color(theme::DIM)
                        .small(),
                    );
                    for command in ledger.unmatched.iter().rev().take(UNMATCHED_SHOWN).rev() {
                        // one line each, the whole command on hover
                        ui.add(egui::Label::new(RichText::new(command).monospace().color(theme::TEXT)).truncate())
                            .on_hover_text(command.as_str());
                    }
                });
                ui.add_space(6.0);
            }
        });
    }

    /// The selected card, large, in the middle of the workspace.
    fn card_view(&mut self, ui: &mut egui::Ui) {
        let Some(node) = self.selected.and_then(|id| self.roadmap.iter().find(|n| n.id == id).cloned()) else {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.38);
                ui.label(RichText::new("◆").color(cards::alpha(theme::ACCENT, 120)).size(40.0));
                ui.label(RichText::new("select a card").color(theme::DIM).size(16.0));
                ui.label(
                    RichText::new("hover a row for its summary · click for the whole card").color(cards::alpha(theme::DIM, 160)).small(),
                );
            });
            return;
        };
        let depth = tree::depth(&self.roadmap, node.id);
        let lane = theme::lane(depth);
        let mut command: Option<String> = None;
        let nodes = self.roadmap.clone();
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
                ui.label(RichText::new(crumbs.join("  ›  ")).color(theme::DIM).small());
            }
            cards::card(ui, lane, true, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(cards::kind_mark(node.kind)).color(lane).size(26.0));
                    ui.label(RichText::new(&node.title).color(lane).size(26.0).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(format!("#{}", node.id)).color(theme::DIM).size(18.0));
                    });
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    cards::badge(ui, &node.state.to_string(), theme::state_color(node.state));
                    cards::badge(ui, &node.kind.to_string(), theme::DIM);
                    cards::priority(ui, node.complexity);
                    if self.worker.budgets {
                        cards::badge_marked(ui, "$", &cards::budget_text(node.complexity), theme::WARN_TEXT)
                            .on_hover_text("the budget of a run, from the complexity; `budgets off` lifts it");
                    } else {
                        cards::badge_marked(ui, "$", "no budget", theme::DIM)
                            .on_hover_text("budgets are off; `budgets on` caps a run by its card's complexity");
                    }
                    let model =
                        node.assignee.as_ref().map_or_else(|| format!("{} (badge)", short(&self.worker.model)), |a| a.model.clone());
                    cards::badge_marked(ui, "◉", &model, theme::TRACE).on_hover_text("the model a run of this card uses");
                    // a click steps the effort up, high wrapping round to low
                    let effort = cards::badge_marked(ui, "⚡", &node.effort.to_string(), theme::WARN_TEXT)
                        .interact(egui::Sense::click())
                        .on_hover_text(format!("the effort a run asks of the model; click for {}", node.effort.next()));
                    if effort.clicked() {
                        command = Some(format!("effort {} {}", node.id, node.effort.next()));
                    }
                    if node.is_final() {
                        cards::badge(ui, "final", theme::WARN).on_hover_text("never reassessed");
                    }
                    if node.session.is_some() {
                        cards::badge_marked(ui, "↻", "resumes its session", theme::ONLINE);
                    }
                });
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if node.kind != NodeKind::Project && action(ui, "▶  run", theme::ACCENT) {
                        command = Some(format!("run {}", node.id));
                    }
                    if action(ui, "↻  reassess", theme::WARN_TEXT) {
                        command = Some(format!("reassess {}", node.id));
                    }
                    if action(ui, "↻↻  reassess tree", theme::WARN_TEXT) {
                        command = Some(format!("reassess-tree {}", node.id));
                    }
                    if action(ui, if node.is_final() { "⊘  unpin" } else { "⊙  pin final" }, theme::WARN) {
                        command = Some(format!("final {}{}", node.id, if node.is_final() { " off" } else { "" }));
                    }
                });
            });
            ui.add_space(10.0);
            // the fields, each its own glowing card
            text_card(ui, "OBJECTIVE", node.objective.as_deref(), theme::WARN_TEXT);
            text_card(ui, "ACCEPTANCE", node.acceptance.as_deref(), theme::ONLINE);
            if !node.files.is_empty() {
                cards::card(ui, theme::DEBUG, false, |ui| {
                    ui.set_width(ui.available_width());
                    cards::section(ui, "FILES", theme::DEBUG);
                    for file in &node.files {
                        ui.label(RichText::new(file).monospace().color(theme::TEXT));
                    }
                });
                ui.add_space(6.0);
            }
            if !node.depends_on.is_empty() {
                cards::card(ui, theme::TRACE, false, |ui| {
                    ui.set_width(ui.available_width());
                    cards::section(ui, "WAITS ON", theme::TRACE);
                    ui.horizontal_wrapped(|ui| {
                        for dep in &node.depends_on {
                            match nodes.iter().find(|n| n.id == *dep) {
                                Some(d) => {
                                    cards::badge(ui, &format!("#{} {}", d.id, d.title), theme::state_color(d.state))
                                        .on_hover_text(d.state.to_string());
                                }
                                None => {
                                    cards::badge(ui, &format!("#{dep} (missing)"), theme::ERROR_TEXT);
                                }
                            }
                        }
                    });
                });
                ui.add_space(6.0);
            }
            if let Some(notes) = node.notes.as_deref() {
                cards::card(ui, theme::DIM, false, |ui| {
                    ui.set_width(ui.available_width());
                    cards::section(ui, "NOTES", theme::DIM);
                    // the whole of the notes: the worker's report is the point of the card
                    markdown::render(ui, notes, theme::DIM);
                });
                ui.add_space(6.0);
            }
            cards::card(ui, theme::TRACE, false, |ui| {
                ui.set_width(ui.available_width());
                cards::section(ui, "MODEL", theme::TRACE);
                ui.horizontal(|ui| {
                    // the Claude models, strongest first, and the badge's; a pick assigns at once.
                    // Most cards leave this to the agent's routing
                    let current = node.assignee.as_ref().map_or("", |a| a.model.as_str());
                    let shown = MODELS.iter().find(|(id, _)| *id == current).map_or_else(
                        || if current.is_empty() { "the badge model".to_owned() } else { current.to_owned() },
                        |(_, name)| (*name).to_owned(),
                    );
                    egui::ComboBox::from_id_salt(("model", node.id))
                        .selected_text(RichText::new(shown).color(theme::TRACE))
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(current.is_empty(), "the badge model").clicked() && !current.is_empty() {
                                command = Some(format!("assign {} none", node.id));
                            }
                            for (id, name) in MODELS {
                                if ui.selectable_label(current == id, name).on_hover_text(id).clicked() && current != id {
                                    command = Some(format!("assign {} {id}", node.id));
                                }
                            }
                        });
                    ui.label(RichText::new("strongest first · a claude-* id runs on Claude Code").color(theme::DIM).small());
                });
                if let Some(session) = &node.session {
                    ui.label(RichText::new(format!("session {session}")).color(theme::DIM).small().monospace());
                }
            });
        });
        if let Some(line) = command {
            self.submit(ui.ctx(), line);
        }
    }

    fn input_bar(&mut self, ui: &mut egui::Ui) {
        let completions = input::completions(&self.input);
        let mut submitted: Option<String> = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("❯").color(theme::ACCENT).strong().size(16.0).monospace());
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
                    RichText::new("type a command (help lists them) · arrows walk the roadmap, Enter runs the selected card")
                        .color(theme::DIM)
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
            ui.label(RichText::new(completions.join("  ")).color(theme::DIM).small());
        }
        if let Some(line) = submitted {
            self.submit(ui.ctx(), line);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.tick(&ctx);
        if self.started.elapsed() < SPLASH || self.fault.is_some() {
            self.splash(root);
            return;
        }
        egui::Panel::top("status")
            .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(8, 6)))
            .show(root, |ui| self.status_bar(ui));
        // the roadmap takes the whole height at the left, the width its rows need up to two
        // fifths of the window; the console and the command line sit right of it
        if self.setup.is_none() {
            let width = self.roadmap_width.min(root.max_rect().width() * 0.4);
            egui::Panel::left("roadmap")
                .resizable(false)
                .exact_size(width)
                .frame(Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(8, 8)).stroke(Stroke::new(1.0, theme::RULE)))
                .show(root, |ui| self.roadmap_panel(ui));
        }
        // the command line at the bottom, the console right above it
        egui::Panel::bottom("input")
            .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(12, 8)).stroke(Stroke::new(1.0, theme::RULE)))
            .show(root, |ui| self.input_bar(ui));
        egui::Panel::bottom("console")
            .resizable(true)
            .default_size(300.0)
            .frame(Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(12, 6)))
            .show(root, |ui| {
                ui.label(RichText::new("CONSOLE").small().strong().color(theme::DIM));
                egui::ScrollArea::vertical().id_salt("console").auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                    console::draw(ui, &self.sink, self.show_debug);
                });
            });
        if self.setup.is_some() {
            // no roadmap yet: the choice takes the place of the roadmap and the card
            egui::CentralPanel::default()
                .frame(Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(18, 12)))
                .show(root, |ui| self.setup_view(ui));
            ctx.request_repaint_after(Duration::from_millis(if self.job.is_some() { 60 } else { 250 }));
            return;
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(18, 12)))
            .show(root, |ui| self.center(ui));
        let live = self.sink.live_run().is_some() || self.job.is_some();
        ctx.request_repaint_after(Duration::from_millis(if live { 40 } else { 250 }));
    }
}

impl Drop for App {
    fn drop(&mut self) {
        ra_console::instance::release();
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
        let title = ctx.fonts_mut(|f| f.layout_no_wrap(node.title.clone(), FontId::proportional(size), theme::TEXT)).size().x;
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
    depths: std::collections::HashMap<i64, usize>,
}

/// What the walk brings back: the rows shown top to bottom (what the arrow keys walk), and
/// what the pointer did.
#[derive(Default)]
struct Walked {
    shown: Vec<i64>,
    clicked: Option<i64>,
    toggled: Option<i64>,
    command: Option<String>,
    /// The row whose tooltip is open, so the rest of the column can step back.
    tooltip_row: Option<egui::Rect>,
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
    let frame = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, if selected { cards::alpha(lane, 200) } else { Color32::TRANSPARENT }))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(6, 3));
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let arrow = if !has_children {
                "  "
            } else if walk.is_folded(node.id) {
                "▸"
            } else {
                "▾"
            };
            if ui.add(egui::Label::new(RichText::new(arrow).color(theme::DIM)).sense(egui::Sense::click())).clicked() && has_children {
                out.toggled = Some(node.id);
            }
            ui.label(RichText::new(cards::kind_mark(node.kind)).color(lane));
            ui.label(RichText::new(format!("#{}", node.id)).color(theme::DIM).small());
            let title = RichText::new(&node.title).color(if selected { theme::TEXT } else { theme::mix(theme::TEXT, lane, 0.45) });
            ui.label(match node.kind {
                NodeKind::Project => title.strong().size(16.0),
                NodeKind::Milestone => title.strong().size(15.0),
                NodeKind::Section => title.strong(),
                _ => title,
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                cards::badge(ui, &node.state.to_string(), theme::state_color(node.state));
                if node.is_final() {
                    cards::badge(ui, "final", theme::WARN);
                }
                if let Some(a) = &node.assignee {
                    cards::badge_marked(ui, "◉", short(&a.model), theme::TRACE).on_hover_text(&a.model);
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
                        let color = if done == cards_under.len() { theme::ONLINE } else { theme::DIM };
                        ui.label(RichText::new(format!("{done}/{}", cards_under.len())).small().color(color))
                            .on_hover_text("cards accepted of the cards under it");
                    }
                }
                if selected && node.kind != NodeKind::Project {
                    if ui.small_button(RichText::new("▶").color(theme::ACCENT)).on_hover_text("run this card").clicked() {
                        out.command = Some(format!("run {}", node.id));
                    }
                    if ui.small_button(RichText::new("↻").color(theme::WARN_TEXT)).on_hover_text("reassess this card").clicked() {
                        out.command = Some(format!("reassess {}", node.id));
                    }
                }
            });
        });
    });
    // the row's own id, so the next frame reads this frame's hover for the fill
    let response = ui.interact(inner.response.rect, row_id, egui::Sense::click());
    if egui::Tooltip::was_tooltip_open_last_frame(ui.ctx(), response.id) {
        out.tooltip_row = Some(response.rect);
    }
    let response = response.on_hover_ui(|ui| cards::tooltip(ui, node, lane));
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

/// A field of the card as its own glowing card, titled in `color`.
fn text_card(ui: &mut egui::Ui, name: &str, value: Option<&str>, color: Color32) {
    cards::card(ui, color, false, |ui| {
        ui.set_width(ui.available_width());
        cards::section(ui, name, color);
        match value {
            Some(text) => ui.label(RichText::new(text).color(theme::TEXT).size(15.0)),
            None => ui.label(RichText::new("—").color(theme::DIM)),
        };
    });
    ui.add_space(6.0);
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

fn usage_row(ui: &mut egui::Ui, model: &str, t: &Totals, color: Color32) {
    ui.label(RichText::new(model).monospace().color(color));
    for n in [t.calls, t.prompt_tokens, t.completion_tokens, t.cached_tokens] {
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

/// The task the workers hold next: the first in progress, else the first ready, else the first
/// defined task.
fn current_task(nodes: &[Node]) -> Option<&Node> {
    nodes
        .iter()
        .find(|n| matches!(n.state, TaskState::InProgress | TaskState::Assigned))
        .or_else(|| nodes.iter().find(|n| n.state == TaskState::Ready))
        .or_else(|| nodes.iter().find(|n| n.state == TaskState::Defined && n.kind != NodeKind::Project))
}

fn short(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
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
