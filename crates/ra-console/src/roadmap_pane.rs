//! The roadmap pane: rounded cards nested inside each other. The project card holds the whole
//! tree; a milestone card holds its sections; a section card holds its tasks. A parent's border
//! columns run through its children, so an expanded branch reads as boxes embedded in their
//! ancestor's box. The selected card opens its detail fields inside its own outline; the hovered
//! card raises its border. The wheel and the keys scroll it, the mouse clicks it.

use std::collections::HashMap;

use ra_roadmap::model::{Node, NodeKind, TaskState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme;

/// One generated row of the pane: the card it belongs to, whether it is a border row, and its
/// content. Border rows carry the card's outline; a child's rows sit between its parent's.
struct Row {
    id: i64,
    border: bool,
    line: Line<'static>,
}

/// The style of an outline row: the accent for the selected card, the warm highlight for the
/// hovered one, the rule color otherwise, all on the panel surface.
fn outline(selected: bool, hovered: bool) -> Style {
    let fg = if selected {
        theme::ACCENT
    } else if hovered {
        theme::WARN_TEXT
    } else {
        theme::RULE
    };
    Style::new().fg(fg).bg(theme::PANEL)
}

/// The rows of one card and, when it is open, of its whole subtree. `width` is the outer width of
/// this card's box; a child's box takes two columns less, one for each side of its parent.
fn rows_of(
    node: &Node,
    children: &HashMap<i64, Vec<Node>>,
    folded: &[i64],
    selected: Option<i64>,
    hover_id: Option<i64>,
    width: usize,
) -> Vec<Row> {
    let id = node.id;
    let is_selected = selected == Some(id);
    let is_hovered = hover_id == Some(id);
    let outline_style = outline(is_selected, is_hovered);
    let folded_here = children.contains_key(&id) && folded.contains(&id);
    let mut rows = vec![Row { id, border: true, line: edge_row("╭", "─", "╮", width, outline_style) }];

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
        rows.push(Row { id, border: false, line: boxed_row(line, width, outline_style) });
    }
    if is_selected {
        for line in detail_rows(node, room) {
            rows.push(Row { id, border: false, line: boxed_row(line, width, outline_style) });
        }
    }

    // the children, embedded: each child's box is two columns narrower, and this card's outline
    // runs down both sides of it
    if !folded_here {
        if let Some(kids) = children.get(&id) {
            for kid in kids {
                for row in rows_of(kid, children, folded, selected, hover_id, width.saturating_sub(2)) {
                    rows.push(Row { id: row.id, border: row.border, line: frame_row(row.line, width, outline_style) });
                }
            }
        }
    }

    rows.push(Row { id, border: true, line: edge_row("╰", "─", "╯", width, outline_style) });
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

/// The detail rows of the selected card: the fields, wrapped, one per line of the source text.
fn detail_rows(node: &Node, room: usize) -> Vec<Vec<Span<'static>>> {
    let dim = || Style::new().fg(theme::DIM).bg(theme::PANEL);
    let text = || Style::new().fg(theme::TEXT).bg(theme::PANEL);
    let mut rows = Vec::new();
    for (label, value) in
        [("objective", node.objective.as_deref()), ("acceptance", node.acceptance.as_deref()), ("notes", node.notes.as_deref())]
    {
        let Some(value) = value else { continue };
        for (i, piece) in value.split('\n').enumerate() {
            let head = if i == 0 { format!("{label}: ") } else { "· ".to_owned() };
            for (j, part) in wrap(piece, room.saturating_sub(head.chars().count())).into_iter().enumerate() {
                let head = if j == 0 { head.clone() } else { "  ".to_owned() };
                rows.push(vec![Span::styled(head, dim()), Span::styled(part, text())]);
            }
        }
    }
    if !node.depends_on.is_empty() {
        let ids: Vec<String> = node.depends_on.iter().map(|id| id.to_string()).collect();
        rows.push(vec![Span::styled("waits-on: ", dim()), Span::styled(ids.join(" "), text())]);
    }
    rows
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

/// Draws the pane at `scroll`; returns how far back the rows can scroll, and the card id under
/// every visible screen row, for the click hit test. `hover_id` comes from the previous frame's
/// hit test, so the highlight follows the pointer one frame behind, which the eye cannot see.
pub fn draw(
    f: &mut Frame,
    nodes: &[Node],
    folded: &[i64],
    selected: Option<i64>,
    hover_id: Option<i64>,
    scroll: usize,
    area: Rect,
) -> (usize, Vec<i64>) {
    let mut children: HashMap<i64, Vec<Node>> = HashMap::new();
    let mut roots: Vec<Node> = Vec::new();
    for node in nodes {
        match node.parent {
            None => roots.push(node.clone()),
            Some(parent) => children.entry(parent).or_default().push(node.clone()),
        }
    }
    let width = area.width as usize;
    let mut rows: Vec<Row> = Vec::new();
    for root in &roots {
        rows.extend(rows_of(root, &children, folded, selected, hover_id, width));
    }

    let height = area.height as usize;
    let max_scroll = rows.len().saturating_sub(height);
    let scroll = scroll.min(max_scroll);
    let mut row_ids: Vec<i64> = Vec::with_capacity(height);
    for (i, row) in rows.iter().skip(scroll).take(height).enumerate() {
        let y = area.y + i as u16;
        f.render_widget(Paragraph::new(row.line.clone()), Rect { x: area.x, y, width: area.width, height: 1 });
        row_ids.push(row.id);
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
