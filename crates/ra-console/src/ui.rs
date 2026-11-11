//! The terminal UI: the layout, the key handling and the draw loop. It mirrors the structure of
//! the CrystalCore console: a status line, a log, a prompt, and a footer with the keys.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ra_roadmap::store::Store;
use ratatui::Frame;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use tokio::sync::mpsc;

use crate::agent::Worker;
use crate::commands::{self, Outcome};
use crate::logs::{Kind, Sink};
use crate::roadmap_pane;
use crate::theme;

/// Ctrl+C twice within this stops the console.
const QUIT_WINDOW: Duration = Duration::from_secs(2);
const PROMPT: &str = "❯ ";
/// Rows a mouse wheel notch scrolls.
const WHEEL_ROWS: usize = 3;
/// The model the console starts with; `model` swaps it. A bare Claude id runs on the local
/// Claude Code; a vendor-prefixed id goes through OpenRouter.
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

/// Whether stdout is a terminal; the console needs one.
pub fn is_terminal() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// What the input thread reads.
enum Input {
    Key(KeyEvent),
    /// Mouse wheel: rows up (positive) or down, at screen row `y`.
    Wheel(i32, u16),
    /// A left click at column `x`, row `y`.
    Click(u16, u16),
    /// The pointer moved to column `x`, row `y`.
    Hover(u16, u16),
}

struct App {
    input: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    /// Where Up/Down is in `history`; `None` is the line being typed.
    browsing: Option<usize>,
    /// Rows scrolled up from the newest log line; 0 follows the log.
    scroll: usize,
    /// Rows scrolled up in the roadmap pane.
    tree_scroll: usize,
    /// The ids of the nodes whose children are folded away.
    folded: Vec<i64>,
    seen: u64,
    quit_armed: Option<Instant>,
    started: Instant,
    /// Shown under the prompt until the next key (Tab's choices, for one).
    hint: Option<String>,
    /// The last-known roadmap nodes, for the tree pane.
    roadmap: Option<Vec<ra_roadmap::model::Node>>,
    /// The worker of the next session: the badge model and the provider it connected; `model`
    /// swaps the model and `key` drops the provider.
    worker: Worker,
    /// The node a click picked; the tree highlights its row.
    selected: Option<i64>,
    /// The screen position the pointer is over, for the hover surface and the buttons.
    hover: Option<(u16, u16)>,
    /// The marquee offset of the model badge, advanced while the name overflows its panel.
    marquee: usize,
    /// The tick counter; the marquee advances on every fourth tick.
    ticks: u32,
    /// Whether the trace lines show in the log; `debug` toggles them.
    show_debug: bool,
    /// The height of the log-and-prompt card, in rows; the wheel over it resizes it.
    log_card: u16,
    /// The slow call in flight, shown as the animated status above the prompt.
    job: Option<String>,
    /// The ledger's all-time totals, refreshed when a job completes; `None` without a ledger.
    all_time: Option<ra_usage::Totals>,
}

/// Runs the console until it is told to quit. `worker` carries the badge model, the provider and
/// the ledger the sessions write to.
pub async fn run(store: Store, worker: Worker, sink: Sink) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    // the wheel scrolls the panes (Shift+drag still selects text in most terminals)
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let (keys_tx, mut keys) = mpsc::unbounded_channel();
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let stop = stop.clone();
        std::thread::Builder::new().name("console-input".into()).spawn(move || read_keys(keys_tx, stop))?
    };
    let mut app = App {
        input: vec![],
        cursor: 0,
        history: vec![],
        browsing: None,
        scroll: 0,
        tree_scroll: 0,
        folded: Vec::new(),
        seen: 0,
        quit_armed: None,
        started: Instant::now(),
        hint: None,
        roadmap: None,
        all_time: worker.all_time().await,
        worker,
        selected: None,
        hover: None,
        marquee: 0,
        ticks: 0,
        show_debug: false,
        log_card: 12,
        job: None,
    };
    sink.push(Kind::Banner, format!("Rhipper Agent v{} — Tyler Bochard. Type help for commands.", env!("CARGO_PKG_VERSION")));
    // the slow calls report their completion here, so the status above the prompt can stop
    let (job_tx, mut job_done) = mpsc::unbounded_channel::<()>();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut last_draw = Instant::now() - Duration::from_secs(1);
    let mut dirty = true;
    // the screen rows of the tree pane, so the wheel and the clicks route to it
    let mut tree_area: Option<Rect> = None;
    // the log card's screen area, so the wheel knows where to resize it
    let mut log_area: Option<Rect> = None;
    // the frame height, for the resize clamp
    let mut frame_height: u16 = 24;
    // the node under every visible tree row, as the last draw laid them out
    let mut tree_rows: Vec<roadmap_pane::Hit> = Vec::new();
    let result = loop {
        let version = sink.version();
        if version != app.seen {
            if app.scroll > 0 {
                // keep what is being read where it is
                app.scroll += (version - app.seen) as usize;
            }
            app.seen = version;
            dirty = true;
        }
        if dirty || last_draw.elapsed() >= Duration::from_secs(1) {
            let mut max_scroll = app.scroll;
            let mut tree_max_scroll = 0usize;
            let mut row_ids: Vec<roadmap_pane::Hit> = Vec::new();
            if let Err(e) = terminal.draw(|f| {
                frame_height = f.area().height;
                let panes;
                (max_scroll, tree_max_scroll, panes) = draw(f, &app, &sink, &mut row_ids);
                (tree_area, log_area) = (panes.tree, panes.log);
            }) {
                break Err(e.into());
            }
            // scrolled past the oldest line: hold at it, so scrolling down answers at once
            app.scroll = app.scroll.min(max_scroll);
            app.tree_scroll = app.tree_scroll.min(tree_max_scroll);
            tree_rows = row_ids;
            last_draw = Instant::now();
            dirty = false;
            tracing::debug!("frame: log scroll {} of {}, tree scroll {}", app.scroll, max_scroll, app.tree_scroll);
        }
        tokio::select! {
            input = keys.recv() => {
                let Some(input) = input else { break Ok(()) };
                dirty = true;
                match input {
                    Input::Click(x, y) => {
                        // a click inside the tree pane: a button column runs its command, any other
                        // row picks the card it names
                        if let Some(area) = tree_area {
                            if (area.y..area.y + area.height).contains(&y) {
                                let index = (y - area.y) as usize;
                                if let Some(hit) = tree_rows.get(index) {
                                    let column = usize::from(x).saturating_sub(usize::from(area.x));
                                    if let Some(&(_, _, button)) = hit.buttons.iter().find(|(start, end, _)| column >= usize::from(*start) && column < usize::from(*end)) {
                                        let line = match button {
                                            roadmap_pane::Button::Run => format!("run {}", hit.id),
                                            roadmap_pane::Button::Reassess => format!("reassess {}", hit.id),
                                            roadmap_pane::Button::ReassessTree => format!("reassess-tree {}", hit.id),
                                        };
                                        if execute_line(line, &store, &sink, &mut app, &job_tx).await {
                                            break Ok(());
                                        }
                                        continue;
                                    }
                                    let id = hit.id;
                                    if let Some(nodes) = &app.roadmap {
                                        let has_children = nodes.iter().any(|n| n.parent == Some(id));
                                        if app.selected == Some(id) {
                                            if has_children {
                                                if app.folded.contains(&id) {
                                                    app.folded.retain(|f| *f != id);
                                                } else {
                                                    app.folded.push(id);
                                                }
                                            } else {
                                                app.selected = None;
                                            }
                                        } else {
                                            app.selected = Some(id);
                                            // the details live in the card itself; the log stays for commands
                                        }
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    Input::Hover(x, y) => {
                        let over_tree = tree_area.is_some_and(|area| (area.y..area.y + area.height).contains(&y));
                        let next = over_tree.then_some((x, y));
                        if app.hover != next {
                            app.hover = next;
                        } else {
                            dirty = false;
                        }
                        continue;
                    }
                    Input::Wheel(rows, y) => {
                        let over_tree = tree_area.is_some_and(|area| (area.y..area.y + area.height).contains(&y));
                        let over_log = log_area.is_some_and(|area| (area.y..area.y + area.height).contains(&y));
                        if over_tree {
                            // the tree skips rows from its top, so wheel-down reads downward
                            app.tree_scroll = app.tree_scroll.saturating_add_signed(-rows as isize * WHEEL_ROWS as isize);
                        } else if over_log {
                            // the wheel over the log card grows or shrinks it; the roadmap takes the rest
                            let next = i32::from(app.log_card) + rows * 2;
                            let low = 7i32;
                            let high = i32::from(frame_height).saturating_sub(12).max(low);
                            app.log_card = next.clamp(low, high) as u16;
                        } else {
                            app.scroll = app.scroll.saturating_add_signed(rows as isize * WHEEL_ROWS as isize);
                        }
                        continue;
                    }
                    Input::Key(key) => match on_key(&mut app, key) {
                        Action::None => {}
                        Action::Quit => break Ok(()),
                        Action::Run(line) => {
                            if execute_line(line, &store, &sink, &mut app, &job_tx).await {
                                break Ok(());
                            }
                        }
                    },
                }
            }
            Some(()) = job_done.recv() => {
                // the background call finished; the status above the prompt goes away, and the
                // ledger panel reads the row it wrote
                app.job = None;
                app.all_time = app.worker.all_time().await;
                dirty = true;
            }
            _ = tick.tick() => {
                app.ticks = app.ticks.wrapping_add(1);
                // the marquee walks a step every fourth tick, a pace the eye follows
                if app.ticks % 4 == 0 && app.worker.model.chars().count() + 3 > 14 {
                    app.marquee = (app.marquee + 1) % (app.worker.model.chars().count() + 3);
                    dirty = true;
                }
                // the working status flashes while a call is in flight
                if app.job.is_some() && app.ticks % 2 == 0 {
                    dirty = true;
                }
                // the roadmap pane follows the file, an outside edit included
                if let Ok(nodes) = store.all().await {
                    if app.roadmap.as_ref() != Some(&nodes) {
                        app.roadmap = Some(nodes);
                        dirty = true;
                    }
                }
            }
        }
    };
    stop.store(true, Ordering::Relaxed);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    drop(terminal);
    ratatui::restore();
    let _ = reader.join();
    result
}

/// Terminal events on a thread of their own (crossterm's reads block).
fn read_keys(tx: mpsc::UnboundedSender<Input>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        match event::poll(Duration::from_millis(100)) {
            Ok(true) => match event::read() {
                // Windows reports releases too
                Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => {
                    if tx.send(Input::Key(k)).is_err() {
                        return;
                    }
                }
                Ok(Event::Mouse(m)) => match m.kind {
                    MouseEventKind::ScrollUp => {
                        if tx.send(Input::Wheel(1, m.row)).is_err() {
                            return;
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if tx.send(Input::Wheel(-1, m.row)).is_err() {
                            return;
                        }
                    }
                    // any button press picks or runs; drag and release pass through
                    MouseEventKind::Moved => {
                        if tx.send(Input::Hover(m.column, m.row)).is_err() {
                            return;
                        }
                    }
                    MouseEventKind::Down(_) => {
                        if tx.send(Input::Click(m.column, m.row)).is_err() {
                            return;
                        }
                    }
                    _ => {}
                },
                Ok(_) => {}
                Err(_) => return,
            },
            Ok(false) => {}
            Err(_) => return,
        }
    }
}

enum Action {
    None,
    Run(String),
    Quit,
}

fn on_key(app: &mut App, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    app.hint = None;
    if !(ctrl && key.code == KeyCode::Char('c')) {
        app.quit_armed = None;
    }
    match key.code {
        KeyCode::Char('c') if ctrl => {
            if !app.input.is_empty() {
                app.input.clear();
                app.cursor = 0;
            } else if app.quit_armed.is_some_and(|t| t.elapsed() < QUIT_WINDOW) {
                return Action::Quit;
            } else {
                app.quit_armed = Some(Instant::now());
            }
        }
        KeyCode::Char('u') if ctrl => {
            app.input.drain(..app.cursor);
            app.cursor = 0;
        }
        KeyCode::Char('l') if ctrl => app.scroll = 0,
        KeyCode::Char(c) => {
            app.input.insert(app.cursor, c);
            app.cursor += 1;
            app.browsing = None;
        }
        KeyCode::Backspace if app.cursor > 0 => {
            app.cursor -= 1;
            app.input.remove(app.cursor);
        }
        KeyCode::Delete if app.cursor < app.input.len() => {
            app.input.remove(app.cursor);
        }
        KeyCode::Left => app.cursor = app.cursor.saturating_sub(1),
        KeyCode::Right => app.cursor = (app.cursor + 1).min(app.input.len()),
        KeyCode::Home if ctrl => app.scroll = usize::MAX / 2,
        KeyCode::End if ctrl => app.scroll = 0,
        KeyCode::Home => app.cursor = 0,
        KeyCode::End => app.cursor = app.input.len(),
        KeyCode::Up if shift_held(&key) => app.scroll = app.scroll.saturating_add(1),
        KeyCode::Down if shift_held(&key) => app.scroll = app.scroll.saturating_sub(1),
        KeyCode::PageUp => app.scroll = app.scroll.saturating_add(10),
        KeyCode::PageDown => app.scroll = app.scroll.saturating_sub(10),
        KeyCode::Esc => {
            if app.input.is_empty() {
                app.scroll = 0;
            }
            app.input.clear();
            app.cursor = 0;
            app.browsing = None;
        }
        KeyCode::Up if !app.history.is_empty() => {
            let i = match app.browsing {
                None => app.history.len() - 1,
                Some(i) => i.saturating_sub(1),
            };
            app.browsing = Some(i);
            app.input = app.history[i].chars().collect();
            app.cursor = app.input.len();
        }
        KeyCode::Down => {
            if let Some(i) = app.browsing {
                if i + 1 < app.history.len() {
                    app.browsing = Some(i + 1);
                    app.input = app.history[i + 1].chars().collect();
                } else {
                    app.browsing = None;
                    app.input.clear();
                }
                app.cursor = app.input.len();
            }
        }
        KeyCode::Tab => {
            let text: String = app.input.iter().collect();
            if !text.contains(char::is_whitespace) {
                let choices = commands::complete(text.trim_start_matches(['/', '.']));
                match choices.as_slice() {
                    [] => {}
                    [one] => {
                        app.input = format!("{one} ").chars().collect();
                        app.cursor = app.input.len();
                    }
                    many => app.hint = Some(many.join("  ")),
                }
            }
        }
        KeyCode::Enter => {
            let line: String = app.input.iter().collect();
            app.input.clear();
            app.cursor = 0;
            app.browsing = None;
            let line = line.trim().to_owned();
            if line.is_empty() {
                return Action::None;
            }
            if app.history.last() != Some(&line) {
                app.history.push(line.clone());
            }
            return Action::Run(line);
        }
        _ => {}
    }
    Action::None
}

fn shift_held(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::SHIFT)
}

/// Runs one command line through the dispatcher. A slow call (a run, a reassess, a ping) goes to a
/// background task: the console stays live, the status above the prompt shows the work, and the
/// completion lands in the log through the sink. Returns whether the line stopped the console.
async fn execute_line(line: String, store: &Store, sink: &Sink, app: &mut App, job_done: &mpsc::UnboundedSender<()>) -> bool {
    let name = line.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if matches!(name.as_str(), "run" | "reassess" | "reassess-tree" | "ping") {
        sink.push(Kind::Input, line.clone());
        let arg = line.split_whitespace().nth(1).unwrap_or("").to_owned();
        app.job = Some(match name.as_str() {
            "run" => format!("running #{arg}"),
            "ping" => "pinging the provider".to_owned(),
            other => format!("{other} #{arg}"),
        });
        let store = store.clone();
        let sink = sink.clone();
        let worker = app.worker.clone();
        let tx = job_done.clone();
        tokio::spawn(async move {
            match commands::run(&line, &store, &worker).await {
                Ok((out, _)) => {
                    for l in out {
                        // a multi-line answer is a change report; the bubble colors it as a diff
                        let kind = if l.contains('\n') { Kind::Diff } else { Kind::Output };
                        sink.push(kind, l);
                    }
                }
                Err(e) => sink.push(Kind::Error, format!("{e:#}")),
            }
            let _ = tx.send(());
        });
        return false;
    }
    sink.push(Kind::Input, line.clone());
    match commands::run(&line, store, &app.worker).await {
        Ok((out, outcome)) => {
            for l in out {
                sink.push(Kind::Output, l);
            }
            let quit = match outcome {
                Outcome::Continue => false,
                Outcome::Clear => {
                    sink.clear();
                    false
                }
                Outcome::Quit => true,
                Outcome::Model(model) => {
                    app.worker.model = model;
                    false
                }
                Outcome::ToggleDebug => {
                    app.show_debug = !app.show_debug;
                    sink.push(Kind::Output, format!("trace lines {}", if app.show_debug { "on" } else { "off" }));
                    false
                }
            };
            app.scroll = 0;
            quit
        }
        Err(e) => {
            sink.push(Kind::Error, format!("{e:#}"));
            app.scroll = 0;
            false
        }
    }
}

/// Draws the console; returns how far back the log can scroll (in rows) and how far back the tree
/// can, and fills `row_ids` with the node id under every visible tree row. `hover_id` names the
/// card the pointer rests on. The panes from the top: the status line, the usage pane, the roadmap
/// tree, the log, the prompt and the footer.
/// The screen areas the wheel and the clicks route against.
#[derive(Default, Clone, Copy)]
struct Panes {
    tree: Option<Rect>,
    log: Option<Rect>,
}

fn draw(f: &mut Frame, app: &App, sink: &Sink, row_ids: &mut Vec<roadmap_pane::Hit>) -> (usize, usize, Panes) {
    let [header, usage, tree, log_card, footer] = pane_layout(app.log_card).areas(f.area());
    // every cell, so even gaps the widgets never touch paint the theme
    use ratatui::widgets::Widget;
    Block::new().style(Style::new().bg(theme::BG)).render(f.area(), f.buffer_mut());
    f.render_widget(Paragraph::new(status_line(app)).style(Style::new().bg(theme::PANEL)), header);
    draw_usage(f, usage, app);
    let mut tree_max = 0usize;
    if let Some(nodes) = &app.roadmap {
        // the hover row goes in raw; the pane resolves the card on the layout it draws this frame
        let (tree_max_scroll, ids) = roadmap_pane::draw(f, nodes, &app.folded, app.selected, app.hover, app.tree_scroll, tree);
        *row_ids = ids;
        tree_max = tree_max_scroll;
    }
    // the log and the prompt live in one rounded card; the roadmap takes the rest
    let border = if app.quit_armed.is_some() { theme::WARN } else { theme::RULE };
    let card =
        Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(border)).style(Style::new().bg(theme::PANEL));
    f.render_widget(card, log_card);
    let inner =
        Rect { x: log_card.x + 1, y: log_card.y + 1, width: log_card.width.saturating_sub(2), height: log_card.height.saturating_sub(2) };
    let [log, status, prompt] = Layout::vertical([Constraint::Min(0), Constraint::Length(1), Constraint::Length(1)]).areas(inner);
    let max_scroll = draw_log(f, app, sink, log);
    if let Some(job) = &app.job {
        // the working status flashes while a slow call is in flight
        let spinner = SPINNER[usize::try_from(app.ticks % SPINNER.len() as u32).unwrap_or(0)];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {spinner} "), Style::new().fg(theme::ACCENT).bold().bg(theme::PANEL)),
                Span::styled(format!("{job}…"), Style::new().fg(theme::WARN_TEXT).bg(theme::PANEL)),
            ]))
            .style(Style::new().bg(theme::PANEL)),
            status,
        );
    }
    draw_prompt(f, app, prompt);
    f.render_widget(Paragraph::new(footer_line(app, footer.width)).style(Style::new().bg(theme::BG)), footer);
    (max_scroll, tree_max, Panes { tree: Some(tree), log: Some(log_card) })
}

/// The vertical pane layout for a log-card height, so the draw and the hit-testing agree on it.
fn pane_layout(log_card: u16) -> Layout {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(log_card),
        Constraint::Length(1),
    ])
}

/// The usage pane: four areas on one row. The badges name the model and the task of the next run;
/// the tokens carry the session's counts and cost; the ledger on the right carries the all-time
/// cost from the usage store.
fn draw_usage(f: &mut Frame, area: Rect, app: &App) {
    let [model, task, tokens, ledger] =
        Layout::horizontal([Constraint::Length(18), Constraint::Min(14), Constraint::Length(30), Constraint::Length(22)]).areas(area);
    // the model badge: the model the next worker session would run on, marqueeing when it overflows
    let inner = usage_panel(f, model);
    let model_text = app.worker.model.as_str();
    let room = usize::from(inner.width).saturating_sub(2);
    let shown = marquee(model_text, room, app.marquee);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("◉ ", Style::new().fg(theme::ACCENT)),
            Span::styled(shown, Style::new().fg(theme::TEXT).bold()),
        ])),
        inner,
    );
    // the task badge: the node the workers hold next, planned or in progress
    let task_node = app.roadmap.as_deref().and_then(current_task);
    let inner = usage_panel(f, task);
    let mut spans = vec![Span::styled("◇ ", Style::new().fg(theme::TRACE))];
    match task_node {
        Some(node) => {
            spans.push(Span::styled(format!("#{} ", node.id), Style::new().fg(theme::DIM)));
            spans.push(Span::styled(short_title(&node.title, inner.width as usize - 9), Style::new().fg(theme::TEXT).bold()));
        }
        None => spans.push(Span::styled("no task queued", Style::new().fg(theme::DIM))),
    }
    f.render_widget(Paragraph::new(Line::from(spans)), inner);
    // the tokens: the session's in, out and cost, as the worker tallied them
    let inner = usage_panel(f, tokens);
    let dim = Style::new().fg(theme::DIM);
    let session = app.worker.tally();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("⇅", Style::new().fg(theme::ACCENT)),
            Span::raw(" "),
            Span::styled("in ", dim),
            Span::styled(compact(session.prompt_tokens), Style::new().fg(theme::TEXT)),
            Span::styled(" out ", dim),
            Span::styled(compact(session.completion_tokens), Style::new().fg(theme::TEXT)),
            Span::raw(" "),
            Span::styled(dollars(session.cost_usd), Style::new().fg(theme::WARN_TEXT)),
        ])),
        inner,
    );
    // the ledger: the all-time cost from the usage store, its own panel on the right
    let inner = usage_panel(f, ledger);
    let line = match &app.all_time {
        Some(all) => Line::from(vec![
            Span::styled("◉ ", Style::new().fg(theme::ONLINE)),
            Span::styled("ledger ", dim),
            Span::styled(dollars(all.cost_usd), Style::new().fg(theme::TEXT)),
            Span::styled(format!(" · {}", all.calls), dim),
        ]),
        None => Line::from(vec![Span::styled("◌", dim), Span::raw(" "), Span::styled("no ledger", dim)]),
    };
    f.render_widget(Paragraph::new(line), inner);
}

/// A token count that fits a badge: `950`, `12.3k`, `1.2M`.
fn compact(count: u64) -> String {
    match count {
        0..=999 => count.to_string(),
        1_000..=999_999 => format!("{:.1}k", count as f64 / 1_000.0),
        _ => format!("{:.1}M", count as f64 / 1_000_000.0),
    }
}

/// A cost for a badge: cents when it is small, a dash when no call reported one.
fn dollars(cost: Option<f64>) -> String {
    match cost {
        None => "$—".to_owned(),
        Some(c) if c < 0.01 && c > 0.0 => "<$0.01".to_owned(),
        Some(c) => format!("${c:.2}"),
    }
}

/// A window of `room` characters over `text`, offset by `step`, with a gap of three spaces when it
/// wraps; a short text comes back whole.
fn marquee(text: &str, room: usize, step: usize) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    if chars.len() <= room {
        return text.to_owned();
    }
    chars.extend([' ', ' ', ' ']);
    let run = chars.len();
    (0..room).map(|i| chars[(step + i) % run]).collect()
}

/// The title clipped to `room` characters, with an ellipsis when it gives one up.
fn short_title(title: &str, room: usize) -> String {
    let count = title.chars().count();
    if count <= room {
        title.to_owned()
    } else {
        let cut: String = title.chars().take(room.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// The task the workers hold next: the first node in progress, else the first planned one.
fn current_task(nodes: &[ra_roadmap::model::Node]) -> Option<ra_roadmap::model::Node> {
    nodes
        .iter()
        .find(|n| matches!(n.state, ra_roadmap::model::TaskState::InProgress | ra_roadmap::model::TaskState::Assigned))
        .or_else(|| nodes.iter().find(|n| n.state == ra_roadmap::model::TaskState::Ready))
        .or_else(|| {
            nodes.iter().find(|n| n.state == ra_roadmap::model::TaskState::Defined && n.kind != ra_roadmap::model::NodeKind::Project)
        })
        .cloned()
}

/// Paints one rounded panel of the usage row and returns the rect for its text.
fn usage_panel(f: &mut Frame, area: Rect) -> Rect {
    use ratatui::widgets::Widget;
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::RULE))
        .style(Style::new().bg(theme::PANEL))
        .render(area, f.buffer_mut());
    Rect { x: area.x + 2, y: area.y + 1, width: area.width.saturating_sub(3), height: 1 }
}

fn status_line(app: &App) -> Line<'static> {
    let dim = Style::new().fg(theme::DIM);
    let sep = || Span::styled("  │  ", Style::new().fg(theme::RULE));
    let secs = app.started.elapsed().as_secs();
    Line::from(vec![
        Span::styled(" ✦ ", Style::new().fg(theme::ACCENT).bold()),
        Span::styled("Rhipper", Style::new().fg(theme::ACCENT).bold()),
        Span::styled(" Agent", Style::new().fg(theme::TEXT).bold()),
        Span::styled(format!(" {}", env!("CARGO_PKG_VERSION")), dim),
        sep(),
        Span::styled(format!("up {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60), dim),
    ])
}

/// Draws the log at the app's scroll; returns how far back it can scroll.
fn draw_log(f: &mut Frame, app: &App, sink: &Sink, area: Rect) -> usize {
    let (height, width) = (area.height as usize, area.width as usize);
    let want = height + app.scroll;
    let (mut visible, all) = sink.with_lines(|lines| {
        let mut taken: Vec<Line<'static>> = Vec::new();
        let mut all = true;
        for (i, line) in lines.iter().rev().enumerate() {
            // the trace lines stay out of the log until debug turns them on
            if matches!(line.kind, Kind::Log(tracing::Level::DEBUG) | Kind::Log(tracing::Level::TRACE)) && !app.show_debug {
                continue;
            }
            // the lane index counts forward from the oldest line, so a bubble keeps its color
            let lane_index = lines.len() - 1 - i;
            let mut r = rows_of(line, width, lane_index);
            r.reverse();
            taken.extend(r);
            if taken.len() >= want && i + 1 < lines.len() {
                all = false;
                break;
            }
        }
        taken.reverse();
        (taken, all)
    });
    let total = visible.len();
    let max_scroll = if all { total.saturating_sub(height) } else { usize::MAX };
    let scroll = app.scroll.min(total.saturating_sub(height));
    let end = total - scroll;
    let start = end.saturating_sub(height);
    let mut shown: Vec<Line<'static>> = visible.drain(start..end).collect();
    // newest at the bottom, like a terminal
    while shown.len() < height {
        shown.insert(0, Line::raw(""));
    }
    f.render_widget(Paragraph::new(shown), area);
    max_scroll
}

/// The spinner frames of the working status.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// One log entry as a rounded bubble: the outline in the kind's color, the text in a color that
/// matches it. An output entry takes the lane color of its place in the session, never red; an
/// error and the banner stay red. A diff entry colors its lines like a patch: `+` green, `-` red,
/// commentary in the bubble's color.
fn rows_of(line: &crate::logs::Line, width: usize, lane_index: usize) -> Vec<Line<'static>> {
    let (border, text) = match line.kind {
        Kind::Input => (theme::ACCENT, theme::TEXT),
        Kind::Output | Kind::Diff => {
            let lane = theme::lane(1 + lane_index % (theme::LANE_COUNT - 1));
            (lane, lane)
        }
        Kind::Banner => (theme::ACCENT, theme::ACCENT),
        Kind::Error => (theme::ACCENT, theme::ERROR_TEXT),
        Kind::Log(tracing::Level::ERROR) => (theme::ACCENT, theme::ERROR_TEXT),
        Kind::Log(tracing::Level::WARN) => (theme::WARN, theme::WARN_TEXT),
        Kind::Log(tracing::Level::DEBUG) => (theme::DEBUG, theme::DEBUG),
        Kind::Log(tracing::Level::TRACE) => (theme::TRACE, theme::TRACE),
        Kind::Log(_) => (theme::DIM, theme::TEXT),
    };
    let border_style = Style::new().fg(border);
    let text_style = Style::new().fg(text).bg(theme::PANEL);
    let added = Style::new().fg(theme::ONLINE).bg(theme::PANEL);
    let removed = Style::new().fg(theme::ERROR_TEXT).bg(theme::PANEL);
    let room = width.saturating_sub(4).max(4);
    // every entry may hold several lines; a diff colors each by its marker
    let sources: Vec<(String, Style)> = match line.kind {
        Kind::Diff => line
            .text
            .split('\n')
            .map(|src| {
                let style = match src.chars().next() {
                    Some('+') => added,
                    Some('-') => removed,
                    _ => text_style,
                };
                (src.to_owned(), style)
            })
            .collect(),
        _ => vec![(line.text.clone(), text_style)],
    };
    let mut rows = vec![Line::from(vec![
        Span::styled("╭", border_style),
        Span::styled("─".repeat(room + 2), border_style),
        Span::styled("╮", border_style),
    ])];
    for (source, style) in sources {
        for piece in wrap(&source, room) {
            let pad = " ".repeat(room.saturating_sub(piece.chars().count()));
            rows.push(Line::from(vec![
                Span::styled("│ ", border_style),
                Span::styled(piece, style),
                Span::styled(pad, Style::new().bg(theme::PANEL)),
                Span::styled(" │", border_style),
            ]));
        }
    }
    rows.push(Line::from(vec![
        Span::styled("╰", border_style),
        Span::styled("─".repeat(room + 2), border_style),
        Span::styled("╯", border_style),
    ]));
    rows
}

/// Wraps `text` into rows of at most `room` characters, on spaces where it can.
fn wrap(text: &str, room: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > room {
            rows.push(current.clone());
            current.clear();
        }
        if current.is_empty() && word.chars().count() > room {
            // a word longer than the room: split it hard
            for chunk in word.as_bytes().chunks(room) {
                rows.push(String::from_utf8_lossy(chunk).into_owned());
            }
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    rows.push(current);
    rows
}

/// Draws the prompt as one row inside the log card.
fn draw_prompt(f: &mut Frame, app: &App, area: Rect) {
    let room = (area.width as usize).saturating_sub(PROMPT.chars().count() + 1).max(1);
    // keep the cursor in view
    let skip = app.cursor.saturating_sub(room);
    let text: String = app.input.iter().skip(skip).take(room).collect();
    let mut spans = vec![Span::styled(PROMPT, Style::new().fg(theme::ACCENT).bold())];
    if app.input.is_empty() {
        spans.push(Span::styled("type a command (help lists them)", Style::new().fg(theme::DIM).italic()));
    } else {
        spans.push(Span::styled(text, Style::new().fg(theme::TEXT)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    let x = area.x + (PROMPT.chars().count() + app.cursor - skip) as u16;
    f.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
}

fn footer_line(app: &App, width: u16) -> Line<'static> {
    let dim = Style::new().fg(theme::DIM);
    let key = Style::new().fg(theme::ACCENT);
    if app.quit_armed.is_some_and(|t| t.elapsed() < QUIT_WINDOW) {
        return Line::from(Span::styled("  Press Ctrl+C again to stop the console", Style::new().fg(theme::WARN_TEXT)));
    }
    if let Some(hint) = &app.hint {
        return Line::from(Span::styled(format!("  {hint}"), key));
    }
    let mut spans = vec![Span::raw("  ")];
    for (i, (k, what)) in [
        ("help", "commands"),
        ("↑↓", "history"),
        ("Tab", "complete"),
        ("wheel", "tree · log size"),
        ("click", "pick node"),
        ("Ctrl+C×2", "stop"),
    ]
    .into_iter()
    .enumerate()
    {
        if i > 0 {
            spans.push(Span::styled(" · ", dim));
        }
        spans.push(Span::styled(k, key));
        spans.push(Span::styled(format!(" {what}"), dim));
    }
    let left: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let right = if app.scroll > 0 { format!("↑ {} rows back · Esc to follow  ", app.scroll) } else { String::new() };
    let pad = (width as usize).saturating_sub(left + right.chars().count());
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(right, Style::new().fg(theme::WARN_TEXT)));
    Line::from(spans)
}
