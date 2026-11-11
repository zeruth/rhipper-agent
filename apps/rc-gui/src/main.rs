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
use egui::{Align, Color32, CornerRadius, FontId, Frame, Key, Layout, Margin, RichText, Stroke, TextStyle};
use ra_console::agent::Worker;
use ra_console::commands::{self, Outcome};
use ra_console::logs::{Kind, Sink};
use ra_roadmap::model::{Node, NodeKind, TaskState};
use ra_roadmap::store::Store;
use ra_roadmap::tree;
use ra_usage::{Totals, UsageStore};
use tokio::sync::mpsc;

/// How long the splash stays at least.
const SPLASH: Duration = Duration::from_millis(1800);

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
    input: String,
    history: Vec<String>,
    browsing: Option<usize>,
    /// How far into the activity file the log has read.
    activity_offset: u64,
    /// The slow call in flight, named for the status row.
    job: Option<String>,
    job_tx: mpsc::UnboundedSender<()>,
    job_done: mpsc::UnboundedReceiver<()>,
    all_time: Option<Totals>,
    /// The plan's windows as the harness last reported them, and when they were last read.
    limits: ra_providers::limits::Limits,
    limits_read: Instant,
    show_debug: bool,
    /// The model typed into the card view, to assign to the selected card.
    assign_model: String,
    /// What went wrong at start, shown on the splash instead of the workspace.
    fault: Option<String>,
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
        let store = match rt.block_on(Store::open(Path::new("rhipper.md"), "Rhipper Agent")) {
            Ok(store) => store,
            Err(e) => {
                fault = Some(format!("{e:#}"));
                // a store that cannot open still needs a value; the splash shows the fault
                rt.block_on(Store::open(&std::env::temp_dir().join("rhipper-agent-fault.md"), "Rhipper Agent")).expect("a temp store")
            }
        };
        let mut worker = Worker::new(ra_console::ui::DEFAULT_MODEL);
        match rt.block_on(UsageStore::open(Path::new("var/usage.db"))) {
            Ok(ledger) => worker = worker.with_ledger(ledger),
            Err(e) => tracing::warn!("no ledger: {e:#}"),
        }
        let all_time = rt.block_on(worker.all_time());
        sink.push(Kind::Banner, format!("Rhipper Agent {} — the workspace. Type help for commands.", ra_console::VERSION));
        if let Some(what) = replaced {
            sink.push(Kind::Log(tracing::Level::INFO), what);
        }
        let (job_tx, job_done) = mpsc::unbounded_channel();
        Self {
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
            input: String::new(),
            history: Vec::new(),
            browsing: None,
            activity_offset: std::fs::metadata(ra_console::ui::ACTIVITY_FILE).map_or(0, |m| m.len()),
            job: None,
            job_tx,
            job_done,
            all_time,
            limits: ra_providers::limits::Limits::load(Path::new(ra_providers::claude_code::LIMITS_FILE)),
            limits_read: Instant::now(),
            show_debug: false,
            assign_model: String::new(),
            fault,
        }
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
            self.followed = working;
        }
        while self.job_done.try_recv().is_ok() {
            self.job = None;
            self.all_time = self.rt.block_on(self.worker.all_time());
        }
        // the plan's windows, every five seconds: a run of any instance may have moved them
        if self.limits_read.elapsed() >= Duration::from_secs(5) {
            self.limits = ra_providers::limits::Limits::load(Path::new(ra_providers::claude_code::LIMITS_FILE));
            self.limits_read = Instant::now();
        }
    }

    /// Picks a card; the model field of the card view follows.
    fn select(&mut self, id: Option<i64>) {
        self.selected = id;
        let node = id.and_then(|id| self.roadmap.iter().find(|n| n.id == id));
        self.assign_model = node.and_then(|n| n.assignee.as_ref()).map(|a| a.model.clone()).unwrap_or_default();
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
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let secs = self.started.elapsed().as_secs();
                ui.label(RichText::new(format!("up {:02}:{:02}:{:02} ", secs / 3600, secs / 60 % 60, secs % 60)).color(theme::DIM).small());
                // the plan's windows, colored by how full they are
                if self.limits.fresh() {
                    for (name, window) in self.limits.windows.iter().rev() {
                        let label = ra_providers::limits::label(name);
                        let color = if window.utilization >= 0.8 {
                            theme::ERROR_TEXT
                        } else if window.utilization >= 0.5 {
                            theme::WARN_TEXT
                        } else {
                            theme::ONLINE
                        };
                        cards::badge_marked(ui, "◔", &format!("{label} {:.0}%", window.utilization * 100.0), color).on_hover_text(format!(
                            "the plan's {label} window: {:.0}% used, resets in {}\nfrom the harness's last run",
                            window.utilization * 100.0,
                            ra_providers::limits::until(window.resets_at)
                        ));
                    }
                }
                match &self.all_time {
                    Some(all) => {
                        cards::badge_marked(ui, "◉", &format!("ledger {} · {} calls", dollars(all.cost_usd), all.calls), theme::ONLINE)
                            .on_hover_text("all time, from var/usage.db");
                    }
                    None => {
                        cards::badge_marked(ui, "◌", "no ledger", theme::DIM);
                    }
                }
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
            });
        });
    }

    fn roadmap_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("ROADMAP").small().strong().color(theme::DIM));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("{} cards", self.roadmap.len())).small().color(theme::DIM));
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
        ui.add_space(4.0);
        let depths = tree::depths(&self.roadmap);
        let nodes = self.roadmap.clone();
        let mut hidden_under: Vec<i64> = Vec::new();
        let mut clicked: Option<i64> = None;
        let mut toggled: Option<i64> = None;
        let mut command: Option<String> = None;
        egui::ScrollArea::vertical().id_salt("roadmap").auto_shrink([false, false]).show(ui, |ui| {
            for node in &nodes {
                // a node under a folded ancestor stays out
                if node.parent.is_some_and(|p| hidden_under.contains(&p)) {
                    hidden_under.push(node.id);
                    continue;
                }
                if self.folded.contains(&node.id) {
                    hidden_under.push(node.id);
                }
                // a task in a hidden state stays out; the structure above it stays
                if matches!(node.kind, NodeKind::Task | NodeKind::Subtask) && self.hidden_states.contains(&node.state) {
                    continue;
                }
                let depth = depths.get(&node.id).copied().unwrap_or(0);
                let lane = theme::lane(depth);
                let has_children = nodes.iter().any(|n| n.parent == Some(node.id));
                let selected = self.selected == Some(node.id);
                let row_id = ui.make_persistent_id(("row", node.id));
                let hovered = ui.ctx().read_response(row_id).is_some_and(|r| r.hovered());
                let fill = if selected {
                    cards::alpha(lane, 40)
                } else if hovered {
                    cards::alpha(lane, 18)
                } else {
                    Color32::TRANSPARENT
                };
                let frame = Frame::new()
                    .fill(fill)
                    .stroke(Stroke::new(1.0, if selected { cards::alpha(lane, 200) } else { Color32::TRANSPARENT }))
                    .corner_radius(CornerRadius::same(7))
                    .inner_margin(Margin::symmetric(6, 4));
                let response = frame
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.add_space(depth as f32 * 16.0);
                            let arrow = if !has_children {
                                "  "
                            } else if self.folded.contains(&node.id) {
                                "▸"
                            } else {
                                "▾"
                            };
                            if ui.add(egui::Label::new(RichText::new(arrow).color(theme::DIM)).sense(egui::Sense::click())).clicked()
                                && has_children
                            {
                                toggled = Some(node.id);
                            }
                            ui.label(RichText::new(cards::kind_mark(node.kind)).color(lane));
                            ui.label(RichText::new(format!("#{}", node.id)).color(theme::DIM).small());
                            let title =
                                RichText::new(&node.title).color(if selected { theme::TEXT } else { theme::mix(theme::TEXT, lane, 0.45) });
                            ui.label(if matches!(node.kind, NodeKind::Project | NodeKind::Milestone) {
                                title.strong().size(15.0)
                            } else {
                                title
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                cards::badge(ui, &node.state.to_string(), theme::state_color(node.state));
                                if node.is_final() {
                                    cards::badge(ui, "final", theme::WARN);
                                }
                                if let Some(a) = &node.assignee {
                                    cards::badge_marked(ui, "◉", short(&a.model), theme::TRACE).on_hover_text(&a.model);
                                }
                                if node.kind == NodeKind::Task || node.kind == NodeKind::Subtask {
                                    cards::priority(ui, node.complexity);
                                }
                                if selected && node.kind != NodeKind::Project {
                                    if ui.small_button(RichText::new("▶").color(theme::ACCENT)).on_hover_text("run this card").clicked() {
                                        command = Some(format!("run {}", node.id));
                                    }
                                    if ui
                                        .small_button(RichText::new("↻").color(theme::WARN_TEXT))
                                        .on_hover_text("reassess this card")
                                        .clicked()
                                    {
                                        command = Some(format!("reassess {}", node.id));
                                    }
                                }
                            });
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                ui.ctx().memory_mut(|m| m.data.insert_temp(row_id, ()));
                let response = response.on_hover_ui(|ui| cards::tooltip(ui, node, lane));
                if response.clicked() {
                    clicked = Some(node.id);
                }
                if response.double_clicked() && has_children {
                    toggled = Some(node.id);
                }
            }
        });
        if let Some(id) = toggled {
            if !self.folded.remove(&id) {
                self.folded.insert(id);
            }
        }
        if let Some(id) = clicked {
            let next = if self.selected == Some(id) && toggled.is_none() { None } else { Some(id) };
            self.select(next);
        }
        if let Some(line) = command {
            self.submit(ui.ctx(), line);
        }
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
            ui.set_max_width(980.0);
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
                    ui.add(
                        egui::TextEdit::singleline(&mut self.assign_model)
                            .hint_text("the badge model, unless set here")
                            .desired_width(260.0)
                            .font(TextStyle::Monospace),
                    );
                    if action(ui, "assign", theme::TRACE) {
                        let model =
                            if self.assign_model.trim().is_empty() { "none".to_owned() } else { self.assign_model.trim().to_owned() };
                        command = Some(format!("assign {} {model}", node.id));
                    }
                    ui.label(RichText::new("a claude-* id runs on Claude Code · vendor/model on OpenRouter").color(theme::DIM).small());
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
            let edit = egui::TextEdit::singleline(&mut self.input)
                .hint_text(RichText::new("type a command (help lists them)").color(theme::DIM).italics())
                .desired_width(f32::INFINITY)
                .frame(Frame::new())
                .lock_focus(true)
                .layouter(&mut layouter);
            let response = ui.add(edit);
            if response.has_focus() {
                let (enter, up, down, tab) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, Key::Enter),
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                        i.consume_key(egui::Modifiers::NONE, Key::Tab),
                    )
                });
                if enter {
                    submitted = Some(std::mem::take(&mut self.input));
                }
                if up && !self.history.is_empty() {
                    let i = match self.browsing {
                        None => self.history.len() - 1,
                        Some(i) => i.saturating_sub(1),
                    };
                    self.browsing = Some(i);
                    self.input = self.history[i].clone();
                }
                if down {
                    if let Some(i) = self.browsing {
                        if i + 1 < self.history.len() {
                            self.browsing = Some(i + 1);
                            self.input = self.history[i + 1].clone();
                        } else {
                            self.browsing = None;
                            self.input.clear();
                        }
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
        // the command line at the very bottom, the console right above it, across the window
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
        // the roadmap takes the width its rows need, up to two fifths of the window
        let width = self.roadmap_width.min(root.max_rect().width() * 0.4);
        egui::Panel::left("roadmap")
            .resizable(false)
            .exact_size(width)
            .frame(Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(8, 8)))
            .show(root, |ui| self.roadmap_panel(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(18, 12)))
            .show(root, |ui| self.card_view(ui));
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
