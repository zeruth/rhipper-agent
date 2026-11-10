//! The roadmap pane: the tree of `rhipper.md` as a scrollable, collapsible view. A mouse click on
//! a node line folds or unfolds its subtree; the wheel and the keys scroll it.

use ra_roadmap::model::{Node, NodeKind, TaskState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme;

/// One row of the pane: the node it names and whether its children are folded away.
pub struct Row {
    pub node: Node,
    pub depth: usize,
    pub folded: bool,
}

/// The visible rows of the tree, given the fold set: the id of every node whose children are
/// hidden.
pub fn rows(nodes: &[Node], folded: &[i64]) -> Vec<Row> {
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
        out.push(Row { node: node.clone(), depth: level, folded: has_children && folded.contains(&node.id) });
    }
    out
}

/// Draws the pane at `scroll`, with the row of `selected` highlighted; returns how far back it can
/// scroll.
pub fn draw(f: &mut Frame, nodes: &[Node], folded: &[i64], selected: Option<i64>, scroll: usize, area: Rect) -> usize {
    let rows = rows(nodes, folded);
    let height = area.height as usize;
    let total = rows.len();
    let max_scroll = total.saturating_sub(height);
    let scroll = scroll.min(max_scroll);
    let shown: Vec<Line> = rows.into_iter().skip(scroll).take(height).map(|row| row_line(&row, selected == Some(row.node.id))).collect();
    f.render_widget(Paragraph::new(shown), area);
    max_scroll
}

/// The row of one node: the fold marker, the id, the kind, the title and the inline fields.
fn row_line(row: &Row, selected: bool) -> Line<'static> {
    let marker = if row.node.kind == NodeKind::Project {
        "◆ ".to_owned()
    } else if row.folded {
        "▸ ".to_owned()
    } else {
        "▾ ".to_owned()
    };
    let marker_color = if row.folded { theme::DIM } else { theme::ACCENT };
    let state_color = state_color(row.node.state);
    let title_style = if selected { Style::new().fg(theme::WARN_TEXT).bold() } else { Style::new().fg(theme::TEXT).bold() };
    let mut spans = vec![
        Span::styled("  ".repeat(row.depth), Style::new()),
        Span::styled(marker, Style::new().fg(marker_color)),
        Span::styled(format!("#{} ", row.node.id), Style::new().fg(theme::DIM)),
        Span::styled(row.node.title.clone(), title_style),
        Span::styled(format!(" {{c{}}}", row.node.complexity), Style::new().fg(theme::DIM)),
        Span::styled(format!(" [{}]", row.node.state.serde_name()), Style::new().fg(state_color)),
    ];
    if let Some(assignee) = &row.node.assignee {
        spans.push(Span::styled(format!(" [{}:{}]", assignee.role, assignee.model), Style::new().fg(theme::TRACE)));
    }
    Line::from(spans)
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

/// The row index of the node at a screen row, for the mouse click. The pane shows the rows from
/// `scroll` on; a click on `y` names the row `scroll + y`.
pub fn row_at(y: usize, scroll: usize) -> usize {
    scroll + y
}
