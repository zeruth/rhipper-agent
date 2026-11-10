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

use crate::commands::{self, Outcome};
use crate::logs::{Kind, Sink};
use crate::roadmap_pane;
use crate::theme;

/// Ctrl+C twice within this stops the console.
const QUIT_WINDOW: Duration = Duration::from_secs(2);
const PROMPT: &str = "❯ ";
/// Rows a mouse wheel notch scrolls.
const WHEEL_ROWS: usize = 3;
/// The model the console starts with; `model` swaps it.
pub const DEFAULT_MODEL: &str = "z-ai/glm-5.3-flash";

/// Whether stdout is a terminal; the console needs one.
pub fn is_terminal() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// What the input thread reads.
enum Input {
    Key(KeyEvent),
    /// Mouse wheel: rows up (positive) or down, at screen row `y`.
    Wheel(i32, u16),
    /// A left click on screen row `y`.
    Click(u16),
    /// The pointer moved to screen row `y`.
    Hover(u16),
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
    /// The model the next worker session would run on; `model` swaps it.
    model: String,
    /// The node a click picked; the tree highlights its row.
    selected: Option<i64>,
    /// The screen row the pointer is over, for the hover surface.
    hover: Option<u16>,
    /// The marquee offset of the model badge, advanced while the name overflows its panel.
    marquee: usize,
    /// The tick counter; the marquee advances on every fourth tick.
    ticks: u32,
    /// Whether the trace lines show in the log; `debug` toggles them.
    show_debug: bool,
}

/// Runs the console until it is told to quit.
pub async fn run(store: Store, sink: Sink) -> anyhow::Result<()> {
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
        model: DEFAULT_MODEL.to_owned(),
        selected: None,
        hover: None,
        marquee: 0,
        ticks: 0,
        show_debug: false,
    };
    sink.push(Kind::Output, "Rhipper Agent console ready. Type help for commands.");
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut last_draw = Instant::now() - Duration::from_secs(1);
    let mut dirty = true;
    // the screen rows of the tree pane, so the wheel and the clicks route to it
    let mut tree_area: Option<Rect> = None;
    // the node id under every visible tree row, as the last draw laid them out
    let mut tree_rows: Vec<i64> = Vec::new();
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
            let mut row_ids: Vec<i64> = Vec::new();
            // the card the pointer rests on, named by the previous frame's hit test
            let hover_id = match (app.hover, tree_area) {
                (Some(y), Some(a)) if (a.y..a.y + a.height).contains(&y) => tree_rows.get((y - a.y) as usize).copied(),
                _ => None,
            };
            if let Err(e) = terminal.draw(|f| {
                (max_scroll, tree_max_scroll) = draw(f, &app, &sink, hover_id, &mut row_ids);
                tree_area = tree_area_of(f);
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
                    Input::Click(y) => {
                        // a click inside the tree pane picks the card the row ids name: its details
                        // expand inside its outline, a second click folds a parent
                        if let Some(area) = tree_area {
                            if (area.y..area.y + area.height).contains(&y) {
                                let index = (y - area.y) as usize;
                                if let Some(&id) = tree_rows.get(index) {
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
                                            if let Ok(node) = store.read(id).await {
                                                sink.push(Kind::Output, node_brief(&node));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    Input::Hover(y) => {
                        let over_tree = tree_area.is_some_and(|area| (area.y..area.y + area.height).contains(&y));
                        let next = over_tree.then_some(y);
                        if app.hover != next {
                            app.hover = next;
                        } else {
                            dirty = false;
                        }
                        continue;
                    }
                    Input::Wheel(rows, y) => {
                        let over_tree = tree_area.is_some_and(|area| (area.y..area.y + area.height).contains(&y));
                        if over_tree {
                            app.tree_scroll = app.tree_scroll.saturating_add_signed(rows as isize * WHEEL_ROWS as isize);
                        } else {
                            app.scroll = app.scroll.saturating_add_signed(rows as isize * WHEEL_ROWS as isize);
                        }
                        continue;
                    }
                    Input::Key(key) => match on_key(&mut app, key) {
                        Action::None => {}
                        Action::Quit => break Ok(()),
                        Action::Run(line) => {
                            sink.push(Kind::Input, line.clone());
                            match commands::run(&line, &store, &app.model).await {
                                Ok((out, outcome)) => {
                                    for l in out {
                                        sink.push(Kind::Output, l);
                                    }
                                    match outcome {
                                        Outcome::Continue => {}
                                        Outcome::Clear => sink.clear(),
                                        Outcome::Quit => break Ok(()),
                                        Outcome::Model(model) => app.model = model,
                                        Outcome::ToggleDebug => {
                                            app.show_debug = !app.show_debug;
                                            sink.push(Kind::Output, format!("trace lines {}", if app.show_debug { "on" } else { "off" }));
                                        }
                                    }
                                }
                                Err(e) => sink.push(Kind::Error, format!("{e:#}")),
                            }
                            app.scroll = 0;
                        }
                    },
                }
            }
            _ = tick.tick() => {
                app.ticks = app.ticks.wrapping_add(1);
                // the marquee walks a step every fourth tick, a pace the eye follows
                if app.ticks % 4 == 0 && app.model.chars().count() + 3 > 14 {
                    app.marquee = (app.marquee + 1) % (app.model.chars().count() + 3);
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
                    // any button press folds the node under it; drag and release pass through
                    MouseEventKind::Moved => {
                        if tx.send(Input::Hover(m.row)).is_err() {
                            return;
                        }
                    }
                    MouseEventKind::Down(_) => {
                        if tx.send(Input::Click(m.row)).is_err() {
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

/// Draws the console; returns how far back the log can scroll (in rows) and how far back the tree
/// can, and fills `row_ids` with the node id under every visible tree row. `hover_id` names the
/// card the pointer rests on. The panes from the top: the status line, the usage pane, the roadmap
/// tree, the log, the prompt and the footer.
fn draw(f: &mut Frame, app: &App, sink: &Sink, hover_id: Option<i64>, row_ids: &mut Vec<i64>) -> (usize, usize) {
    let [header, usage, tree, body, input, footer] = pane_layout().areas(f.area());
    // every cell, so even gaps the widgets never touch paint the theme
    use ratatui::widgets::Widget;
    Block::new().style(Style::new().bg(theme::BG)).render(f.area(), f.buffer_mut());
    f.render_widget(Paragraph::new(status_line(app)).style(Style::new().bg(theme::PANEL)), header);
    draw_usage(f, usage, app);
    let mut tree_max = 0usize;
    if let Some(nodes) = &app.roadmap {
        let (tree_max_scroll, ids) = roadmap_pane::draw(f, nodes, &app.folded, app.selected, hover_id, app.tree_scroll, tree);
        *row_ids = ids;
        tree_max = tree_max_scroll;
    }
    let max_scroll = draw_log(f, app, sink, body);
    draw_input(f, app, input);
    f.render_widget(Paragraph::new(footer_line(app, footer.width)).style(Style::new().bg(theme::BG)), footer);
    (max_scroll, tree_max)
}

/// The vertical pane layout, once, so the draw and the hit-testing agree on it.
fn pane_layout() -> Layout {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Percentage(40),
        Constraint::Min(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
}

/// The tree pane's screen area, as the layout just computed it. The draw of the frame already ran,
/// so a re-split of the frame area gives the same rectangles.
fn tree_area_of(f: &Frame) -> Option<Rect> {
    pane_layout().split(f.area()).get(2).copied()
}

/// The usage pane: four areas on one row. The badges name the model and the task of the next run;
/// the tokens carry the counts; the ledger, the OpenRouter connection state, stands on the right
/// until the provider integration fills it.
fn draw_usage(f: &mut Frame, area: Rect, app: &App) {
    let [model, task, tokens, ledger] =
        Layout::horizontal([Constraint::Length(18), Constraint::Min(14), Constraint::Length(20), Constraint::Length(17)]).areas(area);
    // the model badge: the model the next worker session would run on, marqueeing when it overflows
    let inner = usage_panel(f, model);
    let model_text = app.model.as_str();
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
    // the tokens: in, out and the session cost; the counts land with the OpenRouter run
    let inner = usage_panel(f, tokens);
    let dim = Style::new().fg(theme::DIM);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("⇅", Style::new().fg(theme::ACCENT)),
            Span::raw(" "),
            Span::styled("in ", dim),
            Span::styled("—", Style::new().fg(theme::TEXT)),
            Span::styled(" out ", dim),
            Span::styled("—", Style::new().fg(theme::TEXT)),
            Span::raw(" "),
            Span::styled("$", dim),
            Span::styled("—", Style::new().fg(theme::WARN_TEXT)),
        ])),
        inner,
    );
    // the ledger: the OpenRouter connection, its own panel on the right
    let inner = usage_panel(f, ledger);
    f.render_widget(Paragraph::new(Line::from(vec![Span::styled("◌", dim), Span::raw(" "), Span::styled("not linked", dim)])), inner);
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
            let mut r = rows_of(line, width);
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

/// One log line as screen rows `width` wide: a styled prefix, then the text, wrapped under it.
fn rows_of(line: &crate::logs::Line, width: usize) -> Vec<Line<'static>> {
    let dim = Style::new().fg(theme::DIM);
    let (prefix, text_style): (Vec<Span<'static>>, Style) = match line.kind {
        Kind::Input => {
            let (name, args) = line.text.split_once(' ').unwrap_or((&line.text, ""));
            let mut spans = vec![Span::styled(PROMPT.to_string(), Style::new().fg(theme::ACCENT).bold())];
            if !args.is_empty() {
                return vec![Line::from(vec![
                    Span::styled(PROMPT.to_string(), Style::new().fg(theme::ACCENT).bold()),
                    Span::styled(name.to_owned(), Style::new().fg(theme::ACCENT).bold()),
                    Span::styled(format!(" {args}"), Style::new().fg(theme::TEXT)),
                ])];
            }
            spans.push(Span::styled(name.to_owned(), Style::new().fg(theme::ACCENT).bold()));
            return vec![Line::from(spans)];
        }
        Kind::Output => (vec![Span::styled("  ⎿ ".to_string(), dim)], Style::new().fg(theme::OUTPUT)),
        Kind::Error => (vec![Span::styled("  ⎿ ".to_string(), dim)], Style::new().fg(theme::ERROR_TEXT)),
        Kind::Log(_) => (vec![Span::styled("· ".to_string(), dim)], Style::new().fg(theme::TEXT)),
    };
    let indent: usize = prefix.iter().map(|s| s.content.chars().count()).sum();
    let room = width.saturating_sub(indent).max(10);
    let mut out = Vec::new();
    for (i, piece) in wrap(&line.text, room).into_iter().enumerate() {
        let mut spans = if i == 0 { prefix.clone() } else { vec![Span::raw(" ".repeat(indent))] };
        spans.push(Span::styled(piece, text_style));
        out.push(Line::from(spans));
    }
    if out.is_empty() {
        out.push(Line::from(prefix));
    }
    out
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

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let border = if app.quit_armed.is_some() { theme::WARN } else { theme::ACCENT_DEEP };
    let block =
        Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(border)).style(Style::new().bg(theme::PANEL));
    let inner = block.inner(area);
    let room = (inner.width as usize).saturating_sub(PROMPT.chars().count() + 1).max(1);
    // keep the cursor in view
    let skip = app.cursor.saturating_sub(room);
    let text: String = app.input.iter().skip(skip).take(room).collect();
    let mut spans = vec![Span::styled(PROMPT, Style::new().fg(theme::ACCENT).bold())];
    if app.input.is_empty() {
        spans.push(Span::styled("type a command (help lists them)", Style::new().fg(theme::DIM).italic()));
    } else {
        spans.push(Span::styled(text, Style::new().fg(theme::TEXT)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
    let x = inner.x + (PROMPT.chars().count() + app.cursor - skip) as u16;
    f.set_cursor_position((x.min(inner.right().saturating_sub(1)), inner.y));
}

/// One node as a log line: the fields a click on its tree row shows.
fn node_brief(node: &ra_roadmap::model::Node) -> String {
    let mut text = format!("#{} {} {} {{c{}}}", node.id, node.kind.serde_name(), node.title, node.complexity);
    if let Some(objective) = &node.objective {
        text.push_str(&format!("\n  objective: {objective}"));
    }
    if let Some(acceptance) = &node.acceptance {
        text.push_str(&format!("\n  acceptance: {acceptance}"));
    }
    if !node.depends_on.is_empty() {
        text.push_str(&format!("\n  waits-on: {:?}", node.depends_on));
    }
    if let Some(notes) = &node.notes {
        text.push_str(&format!("\n  notes: {notes}"));
    }
    text
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
        ("wheel", "scroll"),
        ("click", "pick node"),
        ("PgUp/PgDn", "page log"),
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
