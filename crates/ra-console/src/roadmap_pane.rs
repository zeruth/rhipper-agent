//! The roadmap pane: slim rounded cards, one per node, stacked and indented under their parent.
//! Each card carries the rounded outline the prompt has, in the state's color; the hovered card
//! raises its border, and the selected card expands its detail fields inside its own outline. A
//! parent card folds its children. The wheel and the keys scroll it, the mouse hovers and clicks
//! it.

use ra_roadmap::model::{Node, NodeKind, TaskState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::theme;

/// One visible card: the node it renders, its tree depth and whether its children are folded away.
struct Card {
    id: i64,
    node: Node,
    level: usize,
    folded: bool,
}

/// The visible cards, in tree order, with the children of a folded parent left out.
fn cards(nodes: &[Node], folded: &[i64]) -> Vec<Card> {
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
        out.push(Card { id: node.id, node: node.clone(), level: depth(node), folded: has_children && folded.contains(&node.id) });
    }
    out
}

/// Draws the pane at `scroll`; the card under `hover` and the selected card carry a brighter
/// outline. Returns how far back the cards can scroll, and the node id under every visible screen
/// row, for the click hit test.
pub fn draw(
    f: &mut Frame,
    nodes: &[Node],
    folded: &[i64],
    selected: Option<i64>,
    hover: Option<u16>,
    scroll: usize,
    area: Rect,
) -> (usize, Vec<i64>) {
    let all = cards(nodes, folded);
    let width = area.width as usize;
    let mut visible = 0usize;
    let mut used = 0usize;
    for card in all.iter().rev() {
        let h = card_height(card, selected == Some(card.id), width);
        if used + h > area.height as usize && used > 0 {
            break;
        }
        used += h;
        visible += 1;
    }
    let max_scroll = all.len().saturating_sub(visible);
    let scroll = scroll.min(max_scroll);

    let mut row_ids: Vec<i64> = Vec::with_capacity(area.height as usize);
    let mut y = u16::from(area.y);
    for card in all.iter().skip(scroll) {
        let is_selected = selected == Some(card.id);
        let height = card_height(card, is_selected, width);
        if y + height as u16 > area.y + area.height {
            break;
        }
        let level = card.level;
        let indent = 2 * level;
        let rect = Rect { x: area.x + indent as u16, y, width: area.width.saturating_sub(indent as u16), height: height as u16 };
        let hovered = hover.is_some_and(|hy| (rect.y..rect.y + rect.height).contains(&hy));
        let border = if is_selected {
            theme::ACCENT
        } else if hovered {
            theme::WARN_TEXT
        } else {
            theme::RULE
        };
        let block =
            Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(border)).style(Style::new().bg(theme::PANEL));
        f.render_widget(block, rect);
        let inner = Rect { x: rect.x + 1, y: rect.y + 1, width: rect.width.saturating_sub(2), height: rect.height.saturating_sub(2) };
        let lines = content_lines(card, is_selected, inner.width as usize);
        for (i, line) in lines.iter().enumerate() {
            f.render_widget(Paragraph::new(line.clone()), Rect { y: inner.y + i as u16, ..inner });
        }
        for _ in 0..height {
            row_ids.push(card.id);
        }
        y += height as u16;
    }
    (max_scroll, row_ids)
}

/// The screen height of one card: two border rows plus its content rows.
fn card_height(card: &Card, is_selected: bool, width: usize) -> usize {
    2 + content_lines(card, is_selected, width.saturating_sub(2)).len()
}

/// The content rows of a card: the title row, its wrapped continuations, and, when selected, the
/// detail fields.
fn content_lines(card: &Card, is_selected: bool, width: usize) -> Vec<Line<'static>> {
    let node = &card.node;
    let marker = if node.kind == NodeKind::Project {
        "◆"
    } else if card.folded {
        "▸"
    } else {
        "▾"
    };
    let title_style = if is_selected { Style::new().fg(theme::WARN_TEXT).bold() } else { Style::new().fg(theme::TEXT).bold() };
    let mut body = vec![
        Span::styled(marker.to_owned(), Style::new().fg(if card.folded { theme::DIM } else { theme::ACCENT })),
        Span::styled(format!(" #{} ", node.id), Style::new().fg(theme::DIM)),
        Span::styled(node.title.clone(), title_style),
        Span::styled(format!(" {{c{}}}", node.complexity), Style::new().fg(theme::DIM)),
        Span::styled(format!(" [{}]", node.state.serde_name()), Style::new().fg(state_color(node.state))),
    ];
    if let Some(assignee) = &node.assignee {
        body.push(Span::styled(format!(" [{}:{}]", assignee.role, assignee.model), Style::new().fg(theme::TRACE)));
    }
    let mut lines: Vec<Line<'static>> = wrap_spans(body, width).into_iter().map(Line::from).collect();
    if is_selected {
        lines.extend(detail_lines(node, width));
    }
    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines
}

/// The detail rows under a selected card's title, dim, wrapped to `width`.
fn detail_lines(node: &Node, width: usize) -> Vec<Line<'static>> {
    let dim = Style::new().fg(theme::DIM);
    let text = Style::new().fg(theme::TEXT);
    let mut lines = Vec::new();
    for (label, value) in
        [("objective", node.objective.as_deref()), ("acceptance", node.acceptance.as_deref()), ("notes", node.notes.as_deref())]
    {
        let Some(value) = value else { continue };
        for (i, piece) in value.split('\n').enumerate() {
            let head = if i == 0 { format!("{label}: ") } else { "· ".to_owned() };
            let head_width = head.chars().count();
            for (j, part) in wrap(piece, width.saturating_sub(head_width)).into_iter().enumerate() {
                let head = if j == 0 { head.clone() } else { "  ".to_owned() };
                lines.push(Line::from(vec![Span::styled(head, dim), Span::styled(part, text)]));
            }
        }
    }
    if !node.depends_on.is_empty() {
        let ids: Vec<String> = node.depends_on.iter().map(|id| id.to_string()).collect();
        lines.push(Line::from(vec![Span::styled("waits-on: ", dim), Span::styled(ids.join(" "), text)]));
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

/// The color of a state: the bright material colors carry the meaning.
pub fn state_color(state: TaskState) -> ratatui::style::Color {
    match state {
        TaskState::Accepted => theme::ONLINE,
        TaskState::InProgress | TaskState::Assigned => theme::WARN_TEXT,
        TaskState::Failed | TaskState::Blocked | TaskState::NeedsRevision => theme::ERROR_TEXT,
        TaskState::Review | TaskState::Testing | TaskState::Implemented => theme::TRACE,
        TaskState::Cancelled => theme::DIM,
        TaskState::Defined | TaskState::Ready => theme::TEXT,
    }
}
