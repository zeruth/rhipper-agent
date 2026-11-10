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
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use tokio::sync::mpsc;

use crate::commands::{self, Outcome};
use crate::logs::{Kind, Sink};
use crate::theme;

/// Ctrl+C twice within this stops the console.
const QUIT_WINDOW: Duration = Duration::from_secs(2);
const PROMPT: &str = "❯ ";
/// Rows a mouse wheel notch scrolls.
const WHEEL_ROWS: usize = 3;

/// Whether stdout is a terminal; the console needs one.
pub fn is_terminal() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// What the input thread reads.
enum Input {
    Key(KeyEvent),
    /// Mouse wheel: rows up (positive) or down.
    Wheel(i32),
}

struct App {
    input: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    /// Where Up/Down is in `history`; `None` is the line being typed.
    browsing: Option<usize>,
    /// Rows scrolled up from the newest line; 0 follows the log.
    scroll: usize,
    seen: u64,
    quit_armed: Option<Instant>,
    started: Instant,
    /// Shown under the prompt until the next key (Tab's choices, for one).
    hint: Option<String>,
}

/// Runs the console until it is told to quit.
pub async fn run(store: Store, sink: Sink) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    // the wheel scrolls the log (Shift+drag still selects text in most terminals)
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
        seen: 0,
        quit_armed: None,
        started: Instant::now(),
        hint: None,
    };
    sink.push(Kind::Output, "Rhipper Agent console ready. Type help for commands.");
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut last_draw = Instant::now() - Duration::from_secs(1);
    let mut dirty = true;
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
            if let Err(e) = terminal.draw(|f| max_scroll = draw(f, &app, &sink)) {
                break Err(e.into());
            }
            // scrolled past the oldest line: hold at it, so scrolling down answers at once
            app.scroll = app.scroll.min(max_scroll);
            last_draw = Instant::now();
            dirty = false;
        }
        tokio::select! {
            input = keys.recv() => {
                let Some(input) = input else { break Ok(()) };
                dirty = true;
                let key = match input {
                    Input::Wheel(rows) => {
                        app.scroll = app.scroll.saturating_add_signed(rows as isize * WHEEL_ROWS as isize);
                        continue;
                    }
                    Input::Key(key) => key,
                };
                match on_key(&mut app, key) {
                    Action::None => {}
                    Action::Quit => break Ok(()),
                    Action::Run(line) => {
                        sink.push(Kind::Input, line.clone());
                        match commands::run(&line, &store).await {
                            Ok((out, outcome)) => {
                                for l in out {
                                    sink.push(Kind::Output, l);
                                }
                                match outcome {
                                    Outcome::Continue => {}
                                    Outcome::Clear => sink.clear(),
                                    Outcome::Quit => break Ok(()),
                                }
                            }
                            Err(e) => sink.push(Kind::Error, format!("{e:#}")),
                        }
                        app.scroll = 0;
                    }
                }
            }
            _ = tick.tick() => {}
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
                Ok(Event::Mouse(m)) => {
                    let rows = match m.kind {
                        MouseEventKind::ScrollUp => 1,
                        MouseEventKind::ScrollDown => -1,
                        _ => continue,
                    };
                    if tx.send(Input::Wheel(rows)).is_err() {
                        return;
                    }
                }
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

/// Draws the console; returns how far back the log can scroll (in rows).
fn draw(f: &mut Frame, app: &App, sink: &Sink) -> usize {
    let [header, rule, body, input, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1), Constraint::Length(3), Constraint::Length(1)])
            .areas(f.area());
    f.render_widget(Block::new().style(Style::new().bg(theme::BG)), f.area());
    // every cell, so even gaps the widgets never touch paint the theme
    use ratatui::widgets::Widget;
    Block::new().style(Style::new().bg(theme::BG)).render(f.area(), f.buffer_mut());
    f.render_widget(Paragraph::new(status_line(app)).style(Style::new().bg(theme::PANEL)), header);
    f.render_widget(Paragraph::new("━".repeat(rule.width as usize)).fg(theme::RULE), rule);
    let max_scroll = draw_log(f, app, sink, body);
    draw_input(f, app, input);
    f.render_widget(Paragraph::new(footer_line(app, footer.width)).style(Style::new().bg(theme::BG)), footer);
    max_scroll
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
    for (i, (k, what)) in
        [("help", "commands"), ("↑↓", "history"), ("Tab", "complete"), ("wheel/PgUp/PgDn", "scroll"), ("Ctrl+C×2", "stop")]
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
