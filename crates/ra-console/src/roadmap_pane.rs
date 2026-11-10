//! The roadmap pane: rounded cards nested inside each other. The project card holds the whole
//! tree; a milestone card holds its sections; a section card holds its tasks. A parent's border
//! columns run through its children, so an expanded branch reads as boxes embedded in their
//! ancestor's box. The selected card opens its objective and acceptance as small framed panels and
//! its lifecycle buttons inside its own outline. The outlines take the lane color of the depth,
//! bright on the hovered chain, dim elsewhere. The wheel and the keys scroll it, the mouse clicks
//! it.

use std::collections::HashMap;

use ra_roadmap::model::{Node, NodeKind, TaskState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme;

/// One generated row of the pane: the card it belongs to, whether it is a border row, its content,
/// and the buttons its columns run.
struct Row {
    id: i64,
    border: bool,
    line: Line<'static>,
    /// The clickable columns of the row, pane-relative: start, end, button.
    buttons: Vec<(u16, u16, Button)>,
}

/// The lifecycle buttons of the expanded card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Execute the task on the badge model.
    Run,
    /// The model revises this card's objective and acceptance.
    Reassess,
    /// The model revises this card and its whole subtree.
    ReassessTree,
}

/// One row of the hit test: the card under it and the buttons its columns run.
#[derive(Debug, Clone)]
pub struct Hit {
    pub id: i64,
    /// The clickable columns of the row, pane-relative: start, end, button.
    pub buttons: Vec<(u16, u16, Button)>,
}

/// The rows of one card and, when it is open, of its whole subtree. `width` is the outer width of
/// this card's box; a child's box takes two columns less, one for each side of its parent. The
/// outline takes the lane color of the depth: bright on the hovered chain and the selected card,
/// dim everywhere else.
fn rows_of(
    node: &Node,
    children: &HashMap<i64, Vec<Node>>,
    folded: &[i64],
    selected: Option<i64>,
    chain: &std::collections::HashSet<i64>,
    hover_button: Option<Button>,
    depth: usize,
    width: usize,
) -> Vec<Row> {
    let id = node.id;
    let is_selected = selected == Some(id);
    let in_chain = chain.contains(&id);
    let lane = if in_chain || is_selected { theme::lane(depth) } else { theme::dim_lane(depth) };
    let outline_style = Style::new().fg(lane).bg(theme::PANEL);
    let folded_here = children.contains_key(&id) && folded.contains(&id);
    let mut rows = vec![Row { id, border: true, buttons: Vec::new(), line: edge_row("╭", "─", "╮", width, outline_style) }];

    // the title rows: the marker, the id, the title and the badges, wrapped inside the box
    let room = width.saturating_sub(4);
    let title_style = if is_selected {
        Style::new().fg(theme::WARN_TEXT).bold().bg(theme::PANEL)
    } else {
        Style::new().fg(theme::TEXT).bold().bg(theme::PANEL)
    };
    let mut body = vec![
        Span::styled(
            marker_of(node.kind, folded_here),
            Style::new().fg(if folded_here { theme::DIM } else { theme::ACCENT }).bg(theme::PANEL),
        ),
        Span::styled(format!(" #{} ", id), Style::new().fg(theme::DIM).bg(theme::PANEL)),
        Span::styled(node.title.clone(), title_style),
        Span::styled(format!(" {{c{}}}", node.complexity), Style::new().fg(theme::DIM).bg(theme::PANEL)),
        Span::styled(format!(" [{}]", node.state.serde_name()), Style::new().fg(state_color(node.state)).bg(theme::PANEL)),
    ];
    if let Some(assignee) = &node.assignee {
        body.push(Span::styled(format!(" [{}:{}]", assignee.role, assignee.model), Style::new().fg(theme::TRACE).bg(theme::PANEL)));
    }
    for line in wrap_spans(body, room) {
        rows.push(Row { id, border: false, buttons: Vec::new(), line: boxed_row(line, width, outline_style) });
    }

    // the selected card opens its fields: the objective and the acceptance framed in the accent,
    // the notes plain, and the lifecycle buttons on one row, dim until the pointer is over one
    if is_selected {
        for (label, value) in [("objective", node.objective.as_deref()), ("acceptance", node.acceptance.as_deref())] {
            if let Some(value) = value {
                for line in framed_block(label, value, room) {
                    rows.push(Row { id, border: false, buttons: Vec::new(), line });
                }
            }
        }
        for line in plain_detail(node, room) {
            rows.push(Row { id, border: false, buttons: Vec::new(), line });
        }
        let mut defs: Vec<(Button, String)> = Vec::new();
        if node.kind == NodeKind::Task || node.kind == NodeKind::Subtask {
            defs.push((Button::Run, "▶ run".to_owned()));
        }
        if node.kind != NodeKind::Project {
            defs.push((Button::Reassess, "↻ re-assess".to_owned()));
            defs.push((Button::ReassessTree, "↻↻ re-assess tree".to_owned()));
        }
        if !defs.is_empty() {
            let labels: Vec<(Button, &str)> = defs.iter().map(|(b, l)| (*b, l.as_str())).collect();
            let (content, mut ranges) = buttons_row(&labels, hover_button);
            // the columns sit after the ancestors' frames and this card's own border and margin
            for range in ranges.iter_mut() {
                *range = (range.0 + depth as u16 + 2, range.1 + depth as u16 + 2, range.2);
            }
            rows.push(Row { id, border: false, buttons: ranges, line: boxed_row(content, width, outline_style) });
        }
    }

    // the children, embedded: each child's box is two columns narrower, and this card's outline
    // runs down both sides of it
    if !folded_here {
        if let Some(kids) = children.get(&id) {
            for kid in kids {
                for row in rows_of(kid, children, folded, selected, chain, hover_button, depth + 1, width.saturating_sub(2)) {
                    rows.push(Row {
                        id: row.id,
                        border: row.border,
                        buttons: row.buttons,
                        line: frame_row(row.line, width, outline_style),
                    });
                }
            }
        }
    }

    rows.push(Row { id, border: true, buttons: Vec::new(), line: edge_row("╰", "─", "╯", width, outline_style) });
    rows
}

/// The fold marker of a card: a diamond for the project, an arrow for a parent, a bar for a leaf.
fn marker_of(kind: NodeKind, folded: bool) -> String {
    let marker = if kind == NodeKind::Project {
        "◆"
    } else if folded {
        "▸"
    } else {
        "▾"
    };
    format!("{marker} ")
}

/// A border row: the corners and the run between them, `width` cells wide.
fn edge_row(left: &str, fill: &str, right: &str, width: usize, style: Style) -> Line<'static> {
    let run = width.saturating_sub(2);
    Line::from(vec![Span::styled(left.to_owned(), style), Span::styled(fill.repeat(run), style), Span::styled(right.to_owned(), style)])
}

/// A body row inside a box: the left outline, the content padded to fit, the right outline.
fn boxed_row(content: Vec<Span<'static>>, width: usize, outline_style: Style) -> Line<'static> {
    let used: usize = content.iter().map(|s| s.content.chars().count()).sum();
    let room = width.saturating_sub(4);
    let pad = " ".repeat(room.saturating_sub(used));
    let mut spans = vec![Span::styled("│ ", outline_style)];
    spans.extend(content);
    spans.push(Span::styled(pad, Style::new().bg(theme::PANEL)));
    spans.push(Span::styled(" │", outline_style));
    Line::from(spans)
}

/// A child row framed by its parent's outline: the parent's border columns run past it.
fn frame_row(line: Line<'static>, width: usize, outline_style: Style) -> Line<'static> {
    let inner = width.saturating_sub(2);
    let mut spans = vec![Span::styled("│", outline_style)];
    spans.extend(line.spans);
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    spans.push(Span::styled(" ".repeat(inner.saturating_sub(used)), Style::new().bg(theme::PANEL)));
    spans.push(Span::styled("│", outline_style));
    Line::from(spans)
}

/// One framed field of the selected card: a top edge naming the field, the wrapped text, a bottom
/// edge, all in the accent. The rows span `room` cells, the content width of the card.
fn framed_block(label: &str, value: &str, room: usize) -> Vec<Line<'static>> {
    let frame = Style::new().fg(theme::ACCENT).bg(theme::PANEL);
    let label_style = Style::new().fg(theme::WARN_TEXT).bold().bg(theme::PANEL);
    let text_style = Style::new().fg(theme::TEXT).bg(theme::PANEL);
    let inner = room.saturating_sub(4);
    let mut rows = Vec::new();
    let label_cell = format!(" {label} ");
    let run = inner.saturating_sub(label_cell.chars().count());
    rows.push(Line::from(vec![
        Span::styled("╭", frame),
        Span::styled(label_cell, label_style),
        Span::styled("─".repeat(run), frame),
        Span::styled("╮", frame),
    ]));
    for piece in wrap(value, inner) {
        let pad = " ".repeat(inner.saturating_sub(piece.chars().count()));
        rows.push(Line::from(vec![
            Span::styled("│ ", frame),
            Span::styled(piece, text_style),
            Span::styled(pad, Style::new().bg(theme::PANEL)),
            Span::styled(" │", frame),
        ]));
    }
    rows.push(Line::from(vec![Span::styled("╰", frame), Span::styled("─".repeat(inner), frame), Span::styled("╯", frame)]));
    rows
}

/// The plain detail rows of the selected card: the notes and the waits-on, dim.
fn plain_detail(node: &Node, room: usize) -> Vec<Line<'static>> {
    let dim = Style::new().fg(theme::DIM).bg(theme::PANEL);
    let text = Style::new().fg(theme::TEXT).bg(theme::PANEL);
    let mut rows = Vec::new();
    if let Some(notes) = &node.notes {
        for (i, piece) in notes.split('\n').enumerate() {
            let head = if i == 0 { "notes: ".to_owned() } else { "  ".to_owned() };
            for (j, part) in wrap(piece, room.saturating_sub(head.chars().count())).into_iter().enumerate() {
                let head = if j == 0 { head.clone() } else { "  ".to_owned() };
                rows.push(Line::from(vec![Span::styled(head, dim), Span::styled(part, text)]));
            }
        }
    }
    if !node.depends_on.is_empty() {
        let ids: Vec<String> = node.depends_on.iter().map(|id| id.to_string()).collect();
        rows.push(Line::from(vec![Span::styled("waits-on: ", dim), Span::styled(ids.join(" "), text)]));
    }
    rows
}

/// The lifecycle buttons on one row: bracketed labels, dim until the pointer rests on one. Returns
/// the spans and each button's columns within the row.
fn buttons_row(buttons: &[(Button, &str)], hover_button: Option<Button>) -> (Vec<Span<'static>>, Vec<(u16, u16, Button)>) {
    let dim = Style::new().fg(theme::DIM).bg(theme::PANEL);
    let hot = Style::new().fg(theme::ACCENT).bold().bg(theme::PANEL);
    let mut spans = Vec::new();
    let mut ranges = Vec::new();
    let mut offset = 0u16;
    for (i, (button, label)) in buttons.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", dim));
            offset += 2;
        }
        let start = offset;
        spans.push(Span::styled("[", dim));
        let style = if hover_button == Some(*button) { hot } else { dim };
        spans.push(Span::styled((*label).to_owned(), style));
        spans.push(Span::styled("]", dim));
        offset += 1 + label.chars().count() as u16 + 1;
        ranges.push((start, offset, *button));
    }
    (spans, ranges)
}

/// Wraps plain text into rows of at most `room` characters, on spaces where it can.
fn wrap(text: &str, room: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        let take = word.chars().count();
        if !current.is_empty() && current.chars().count() + 1 + take > room {
            rows.push(std::mem::take(&mut current));
        }
        if current.is_empty() && take > room {
            for chunk in word.chars().collect::<Vec<_>>().chunks(room) {
                rows.push(chunk.iter().collect());
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

/// Packs styled spans into rows of `room` cells, splitting a span at the boundary.
fn wrap_spans(spans: Vec<Span<'static>>, room: usize) -> Vec<Vec<Span<'static>>> {
    if room == 0 {
        return vec![spans];
    }
    let mut rows: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    let mut used = 0usize;
    for span in spans {
        let style = span.style;
        for word in span.content.split_inclusive(' ') {
            let take = word.chars().count();
            if take > room {
                // a word longer than the room: each chunk takes a row of its own
                for piece in word.chars().collect::<Vec<_>>().chunks(room) {
                    rows.push(vec![Span::styled(piece.iter().collect::<String>(), style)]);
                }
                used = 0;
                continue;
            }
            if used + take > room && used > 0 {
                rows.push(Vec::new());
                used = 0;
            }
            rows.last_mut().unwrap().push(Span::styled(word.to_owned(), style));
            used += take;
        }
    }
    rows.retain(|row| !row.is_empty());
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

/// Draws the pane at `scroll`; returns how far back the rows can scroll and the hit test under
/// every visible screen row. `hover` is the pointer's position; the hovered card and its ancestors
/// carry their lane colors bright and the hovered button runs hot, all resolved on the layout of
/// this very frame, so an open or a close cannot leave the highlight on the wrong card.
pub fn draw(
    f: &mut Frame,
    nodes: &[Node],
    folded: &[i64],
    selected: Option<i64>,
    hover: Option<(u16, u16)>,
    scroll: usize,
    area: Rect,
) -> (usize, Vec<Hit>) {
    let mut children: HashMap<i64, Vec<Node>> = HashMap::new();
    let mut roots: Vec<Node> = Vec::new();
    for node in nodes {
        match node.parent {
            None => roots.push(node.clone()),
            Some(parent) => children.entry(parent).or_default().push(node.clone()),
        }
    }
    let width = area.width as usize;
    let empty = std::collections::HashSet::new();
    let mut rows: Vec<Row> = Vec::new();
    for root in &roots {
        rows.extend(rows_of(root, &children, folded, selected, &empty, None, 0, width));
    }
    // the card and the button under the pointer, on this frame's layout, then the rows again with
    // the chain bright and the button hot
    let index = hover.and_then(|(_, y)| usize::from(y).checked_sub(usize::from(area.y))).map(|i| i + scroll);
    let hover_id = index.and_then(|i| rows.get(i)).map(|row| row.id);
    let hover_button = index.and_then(|i| rows.get(i)).and_then(|row| {
        hover.and_then(|(x, _)| {
            let x = usize::from(x).saturating_sub(usize::from(area.x));
            row.buttons.iter().find(|(start, end, _)| x >= usize::from(*start) && x < usize::from(*end)).map(|b| b.2)
        })
    });
    if let Some(hover_id) = hover_id {
        let mut chain: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut next = Some(hover_id);
        while let Some(id) = next {
            chain.insert(id);
            next = nodes.iter().find(|n| n.id == id).and_then(|n| n.parent);
        }
        rows.clear();
        for root in &roots {
            rows.extend(rows_of(root, &children, folded, selected, &chain, hover_button, 0, width));
        }
    }

    let height = area.height as usize;
    let max_scroll = rows.len().saturating_sub(height);
    let scroll = scroll.min(max_scroll);
    tracing::debug!(
        "roadmap pane: {} rows, {} cards, scroll {} of {}, pane {}x{}",
        rows.len(),
        roots.len(),
        scroll,
        max_scroll,
        area.width,
        area.height
    );
    let mut row_ids: Vec<Hit> = Vec::with_capacity(height);
    for (i, row) in rows.iter().skip(scroll).take(height).enumerate() {
        let y = area.y + i as u16;
        f.render_widget(Paragraph::new(row.line.clone()), Rect { x: area.x, y, width: area.width, height: 1 });
        row_ids.push(Hit { id: row.id, buttons: row.buttons.clone() });
    }
    (max_scroll, row_ids)
}

/// The color of a state: the bright material colors carry the meaning.
fn state_color(state: TaskState) -> ratatui::style::Color {
    match state {
        TaskState::Accepted => theme::ONLINE,
        TaskState::InProgress | TaskState::Assigned => theme::WARN_TEXT,
        TaskState::Failed | TaskState::Blocked | TaskState::NeedsRevision => theme::ERROR_TEXT,
        TaskState::Review | TaskState::Testing | TaskState::Implemented => theme::TRACE,
        TaskState::Cancelled => theme::DIM,
        TaskState::Defined | TaskState::Ready => theme::TEXT,
    }
}
