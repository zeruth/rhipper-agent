//! The roadmap pane: slim horizontal cards, one per node. A card leads with a state-colored rail,
//! then the id, the title and the badge row. A parent card folds its children; the selected card
//! expands its detail fields beneath itself. The wheel and the keys scroll it, the mouse hovers
//! and clicks it.

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

/// The visible cards: one per unfolded node, and the detail rows of the selected one.
pub fn card_rows(nodes: &[Node], folded: &[i64], selected: Option<i64>) -> Vec<CardRow> {
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
        out.push(CardRow { id: node.id, line: card_line(node, level, has_children && folded.contains(&node.id), is_selected) });
        if is_selected {
            for detail in detail_lines(node, level) {
                out.push(CardRow { id: node.id, line: detail });
            }
        }
    }
    out
}

/// The card of one node: the rail, the fold marker, the id, the title and the badges.
fn card_line(node: &Node, level: usize, folded: bool, selected: bool) -> Line<'static> {
    let rail = Style::new().fg(state_color(node.state));
    let marker = if node.kind == NodeKind::Project {
        "◆"
    } else if folded {
        "▸"
    } else {
        "▾"
    };
    let title_style = if selected { Style::new().fg(theme::WARN_TEXT).bold() } else { Style::new().fg(theme::TEXT).bold() };
    let mut spans = vec![
        Span::styled("  ".repeat(level), Style::new()),
        Span::styled("▌", rail),
        Span::styled(format!("{marker} {}", if marker == "◆" { "" } else { marker }), {
            let color = if folded { theme::DIM } else { theme::ACCENT };
            Style::new().fg(color)
        }),
        Span::styled(format!("#{} ", node.id), Style::new().fg(theme::DIM)),
        Span::styled(node.title.clone(), title_style),
        Span::styled(format!(" {{c{}}}", node.complexity), Style::new().fg(theme::DIM)),
        Span::styled(format!(" [{}]", node.state.serde_name()), Style::new().fg(state_color(node.state))),
    ];
    if let Some(assignee) = &node.assignee {
        spans.push(Span::styled(format!(" [{}:{}]", assignee.role, assignee.model), Style::new().fg(theme::TRACE)));
    }
    Line::from(spans)
}

/// The detail rows under a selected card, dim and indented under it.
fn detail_lines(node: &Node, level: usize) -> Vec<Line<'static>> {
    let indent = " ".repeat(level * 2 + 5);
    let dim = Style::new().fg(theme::DIM);
    let mut lines = Vec::new();
    for (label, value) in
        [("objective", node.objective.as_deref()), ("acceptance", node.acceptance.as_deref()), ("notes", node.notes.as_deref())]
    {
        if let Some(value) = value {
            for (i, piece) in value.split('\n').enumerate() {
                let head = if i == 0 { format!("{label}: ") } else { "· ".to_owned() };
                lines.push(Line::from(vec![
                    Span::styled(indent.clone(), Style::new()),
                    Span::styled(head, dim),
                    Span::styled(piece.to_owned(), Style::new().fg(theme::TEXT)),
                ]));
            }
        }
    }
    if !node.depends_on.is_empty() {
        let ids: Vec<String> = node.depends_on.iter().map(|id| id.to_string()).collect();
        lines.push(Line::from(vec![
            Span::styled(format!("{indent}waits-on: "), dim),
            Span::styled(ids.join(" "), Style::new().fg(theme::TEXT)),
        ]));
    }
    lines
}

/// Draws the pane at `scroll`; the card under `hover` carries the raised surface. Returns how far
/// back it can scroll.
pub fn draw(f: &mut Frame, nodes: &[Node], folded: &[i64], selected: Option<i64>, hover: Option<u16>, scroll: usize, area: Rect) -> usize {
    let rows = card_rows(nodes, folded, selected);
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
