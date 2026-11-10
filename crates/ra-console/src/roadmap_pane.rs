//! The roadmap pane: slim horizontal cards, one per node. A card leads with a state-colored rail,
//! then the id, the title and the badge row. A card longer than the pane wraps onto continuation
//! rows that keep the rail column, so the cards read uniform. A parent card folds its children;
//! the selected card expands its detail fields beneath itself. The wheel and the keys scroll it,
//! the mouse hovers and clicks it.

use ra_roadmap::model::{Node, NodeKind, TaskState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::theme;

/// One visible row of the pane: the node it names and the line to draw.
pub struct CardRow {
    pub id: i64,
    pub line: Line<'static>,
}

/// The visible cards: one per unfolded node, and the detail rows of the selected one. `width` is
/// the pane width in cells.
pub fn card_rows(nodes: &[Node], folded: &[i64], selected: Option<i64>, width: usize) -> Vec<CardRow> {
    let by_id: std::collections::HashMap<i64, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    let depth = |node: &Node| -> usize {
        let mut level = 0;
        let mut parent = node.parent;
        while let Some(id) = parent {
            parent = by_id.get(&id).and_then(|n| n.parent);
            level += 1;
        }
        level
    };
    let mut out = Vec::new();
    for node in nodes {
        let level = depth(node);
        // hidden under a folded ancestor
        let mut parent = node.parent;
        let mut hidden = false;
        while let Some(id) = parent {
            if folded.contains(&id) {
                hidden = true;
                break;
            }
            parent = by_id.get(&id).and_then(|n| n.parent);
        }
        if hidden {
            continue;
        }
        let has_children = nodes.iter().any(|n| n.parent == Some(node.id));
        let is_selected = selected == Some(node.id);
        for line in card_lines(node, level, has_children && folded.contains(&node.id), is_selected, width) {
            out.push(CardRow { id: node.id, line });
        }
        if is_selected {
            for line in detail_lines(node, level, width) {
                out.push(CardRow { id: node.id, line });
            }
        }
    }
    out
}

/// The card of one node, wrapped to `width`: the rail, the fold marker, the id, the title and the
/// badges. A continuation row carries a dim rail at the same column, so the block reads uniform.
fn card_lines(node: &Node, level: usize, folded: bool, selected: bool, width: usize) -> Vec<Line<'static>> {
    let marker = if node.kind == NodeKind::Project {
        "◆"
    } else if folded {
        "▸"
    } else {
        "▾"
    };
    let title_style = if selected { Style::new().fg(theme::WARN_TEXT).bold() } else { Style::new().fg(theme::TEXT).bold() };
    let rail = Span::styled("▌", Style::new().fg(state_color(node.state)));
    let marker_span = Span::styled(
        format!("{marker} {}", if marker == "◆" { "" } else { marker }),
        Style::new().fg(if folded { theme::DIM } else { theme::ACCENT }),
    );
    let id_span = Span::styled(format!("#{} ", node.id), Style::new().fg(theme::DIM));
    let mut body = vec![
        Span::styled(node.title.clone(), title_style),
        Span::styled(format!(" {{c{}}}", node.complexity), Style::new().fg(theme::DIM)),
        Span::styled(format!(" [{}]", node.state.serde_name()), Style::new().fg(state_color(node.state))),
    ];
    if let Some(assignee) = &node.assignee {
        body.push(Span::styled(format!(" [{}:{}]", assignee.role, assignee.model), Style::new().fg(theme::TRACE)));
    }
    // the prefix of a continuation row: the indent, then a dim rail where the card's rail sits
    let lead = {
        let mut lead = vec![Span::raw("  ".repeat(level))];
        lead.push(Span::styled("▌", Style::new().fg(theme::RULE)));
        lead.push(Span::raw("  "));
        lead
    };
    let room = width.saturating_sub(2 * level + 4);
    let rows = wrap_spans(body, room);
    let mut out = Vec::new();
    for (i, row) in rows.into_iter().enumerate() {
        let mut spans =
            if i == 0 { vec![Span::raw("  ".repeat(level)), rail.clone(), marker_span.clone(), id_span.clone()] } else { lead.clone() };
        spans.extend(row);
        out.push(Line::from(spans));
    }
    if out.is_empty() {
        out.push(Line::from(vec![Span::raw("  ".repeat(level)), rail, marker_span, id_span]));
    }
    out
}

/// The detail rows under a selected card, dim and indented under it, wrapped to `width`.
fn detail_lines(node: &Node, level: usize, width: usize) -> Vec<Line<'static>> {
    let lead = " ".repeat(level * 2 + 5);
    let dim = Style::new().fg(theme::DIM);
    let text = Style::new().fg(theme::TEXT);
    let mut lines = Vec::new();
    for (label, value) in
        [("objective", node.objective.as_deref()), ("acceptance", node.acceptance.as_deref()), ("notes", node.notes.as_deref())]
    {
        let Some(value) = value else { continue };
        for (i, piece) in value.split('\n').enumerate() {
            let head = if i == 0 { format!("{label}: ") } else { "· ".to_owned() };
            let room = width.saturating_sub(lead.len() + 2);
            let rows = wrap(piece, room.saturating_sub(head.chars().count()));
            for (j, part) in rows.into_iter().enumerate() {
                let head = if j == 0 { head.clone() } else { "  ".to_owned() };
                lines.push(Line::from(vec![Span::styled(lead.clone(), Style::new()), Span::styled(head, dim), Span::styled(part, text)]));
            }
        }
    }
    if !node.depends_on.is_empty() {
        let ids: Vec<String> = node.depends_on.iter().map(|id| id.to_string()).collect();
        lines.push(Line::from(vec![
            Span::styled(lead.clone(), Style::new()),
            Span::styled("waits-on: ", dim),
            Span::styled(ids.join(" "), text),
        ]));
    }
    lines
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

/// Packs styled spans into rows of `room` cells, splitting a span at the boundary. The rows carry
/// no prefix; the caller adds one.
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

/// Draws the pane at `scroll`; the card under `hover` carries the raised surface. Returns how far
/// back it can scroll.
pub fn draw(f: &mut Frame, nodes: &[Node], folded: &[i64], selected: Option<i64>, hover: Option<u16>, scroll: usize, area: Rect) -> usize {
    let rows = card_rows(nodes, folded, selected, area.width as usize);
    let height = area.height as usize;
    let total = rows.len();
    let max_scroll = total.saturating_sub(height);
    let scroll = scroll.min(max_scroll);
    let hover_row = hover.map(|y| usize::from(y).saturating_sub(usize::from(area.y)) + scroll);
    use ratatui::widgets::Widget;
    for (i, card) in rows.iter().skip(scroll).take(height).enumerate() {
        let surface = if hover_row == Some(i) { theme::PANEL } else { theme::BG };
        let row = Rect { x: area.x, y: area.y + i as u16, width: area.width, height: 1 };
        Block::new().style(Style::new().bg(surface)).render(row, f.buffer_mut());
        f.render_widget(Paragraph::new(card.line.clone()), row);
    }
    max_scroll
}

/// The visible index of a click at screen row `y`.
pub fn row_at(y: u16, area_y: u16, scroll: usize) -> usize {
    usize::from(y) - usize::from(area_y) + scroll
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
