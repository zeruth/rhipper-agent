//! The roadmap pane: rounded cards nested inside each other. The project card holds the whole
//! tree; a milestone card holds its sections; a section card holds its tasks. A parent's border
//! columns run through its children, so an expanded branch reads as boxes embedded in their
//! ancestor's box, with a blank line of the card between parent and children. The selected card
//! opens its objective, acceptance and notes as self-contained bubbles inside its outline, with
//! the lifecycle buttons beneath. The outlines take the lane color of the depth, bright on the
//! hovered chain, dim elsewhere. The wheel and the keys scroll it, the mouse clicks it.

use std::collections::HashMap;

use ra_roadmap::model::{Node, NodeKind, TaskState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
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

/// The roadmap as the rows read it: the children of every node, and the state each card stands in
/// once its waits-on are read.
struct Shape {
    children: HashMap<i64, Vec<Node>>,
    states: HashMap<i64, TaskState>,
}

/// The rows of one card and, when it is open, of its whole subtree. `width` is the outer width of
/// this card's box; a child's box takes two columns less, one for each side of its parent. The
/// outline takes the lane color of the depth: bright on the hovered chain and the selected card,
/// dim everywhere else.
// the recursion carries the pane's whole state down; a struct for it would be these arguments
// under another name
#[allow(clippy::too_many_arguments)]
fn rows_of(
    node: &Node,
    shape: &Shape,
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
    let folded_here = shape.children.contains_key(&id) && folded.contains(&id);
    let computed = shape.states.get(&id).copied().unwrap_or(node.state);
    let mut rows =
        vec![Row { id, border: true, buttons: Vec::new(), line: titled_edge(node, computed, is_selected, width, outline_style) }];

    // the objective and the acceptance of every open card, each in its own rounded bubble in the
    // accent, sealed off from the parent's outline. The notes and the buttons stay with the picked
    // card alone
    let room = width.saturating_sub(4);
    if !folded_here {
        let accent = Style::new().fg(theme::ACCENT).bg(theme::PANEL);
        for (label, value) in [("objective", node.objective.as_deref()), ("acceptance", node.acceptance.as_deref())] {
            let Some(value) = value else { continue };
            for line in field_bubble(label, value, accent, theme::ACCENT, room) {
                rows.push(Row { id, border: false, buttons: Vec::new(), line: boxed_row(line.spans, width, outline_style) });
            }
        }
        if is_selected {
            if let Some(notes) = &node.notes {
                let dim = Style::new().fg(theme::DIM).bg(theme::PANEL);
                for line in field_bubble("notes", notes, dim, theme::DIM, room) {
                    rows.push(Row { id, border: false, buttons: Vec::new(), line: boxed_row(line.spans, width, outline_style) });
                }
            }
        }
        if is_selected {
            if !node.depends_on.is_empty() {
                let ids: Vec<String> = node.depends_on.iter().map(|id| id.to_string()).collect();
                rows.push(Row {
                    id,
                    border: false,
                    buttons: Vec::new(),
                    line: boxed_row(
                        vec![
                            Span::styled("waits-on: ", Style::new().fg(theme::DIM).bg(theme::PANEL)),
                            Span::styled(ids.join(" "), Style::new().fg(theme::TEXT).bg(theme::PANEL)),
                        ],
                        width,
                        outline_style,
                    ),
                });
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
    }

    // the children, embedded: each child's box is two columns narrower, this card's outline runs
    // down both sides of it, and a blank line of the card separates the parent from its children
    // and each child from the next
    if !folded_here && let Some(kids) = shape.children.get(&id) {
        for kid in kids {
            rows.push(Row { id, border: false, buttons: Vec::new(), line: boxed_row(Vec::new(), width, outline_style) });
            for row in rows_of(kid, shape, folded, selected, chain, hover_button, depth + 1, width.saturating_sub(2)) {
                rows.push(Row { id: row.id, border: row.border, buttons: row.buttons, line: frame_row(row.line, width, outline_style) });
            }
        }
    }

    rows.push(Row { id, border: true, buttons: Vec::new(), line: edge_row("╰", "─", "╯", width, outline_style) });
    rows
}

/// A border row: the corners and the run between them, `width` cells wide.
fn edge_row(left: &str, fill: &str, right: &str, width: usize, style: Style) -> Line<'static> {
    let run = width.saturating_sub(2);
    Line::from(vec![Span::styled(left.to_owned(), style), Span::styled(fill.repeat(run), style), Span::styled(right.to_owned(), style)])
}

/// The text clipped to `room` characters, with an ellipsis when it gives one up.
fn short_title(title: &str, room: usize) -> String {
    let count = title.chars().count();
    if count <= room {
        title.to_owned()
    } else {
        let cut: String = title.chars().take(room.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// The top edge of a card, with the role, the id, the title and the badges set into the outline
/// itself, the way the objective labels sit in their rules. A project, a milestone or a section
/// reads as `Roadmap:`, a task or a subtask as `Task:`. The title clips when it would not fit, and
/// the filler run closes the edge to `width`. The state badge is the computed one, with the stored
/// one in brackets when they differ: `[ready (defined)]`.
fn titled_edge(node: &Node, computed: TaskState, is_selected: bool, width: usize, outline_style: Style) -> Line<'static> {
    let title_style = if is_selected {
        Style::new().fg(theme::WARN_TEXT).bold().bg(theme::PANEL)
    } else {
        Style::new().fg(theme::TEXT).bold().bg(theme::PANEL)
    };
    let badge = |text: String, color: ratatui::style::Color| Span::styled(text, Style::new().fg(color).bg(theme::PANEL));
    let role = match node.kind {
        NodeKind::Task | NodeKind::Subtask => "Task: ",
        _ => "Roadmap: ",
    };
    let assignee = node.assignee.as_ref().map_or(0, |a| format!(" [{}:{}]", a.role, a.model).chars().count());
    let state = if computed == node.state {
        format!(" [{}]", node.state.serde_name())
    } else {
        format!(" [{} ({})]", computed.serde_name(), node.state.serde_name())
    };
    let overhead = 2usize
        + if node.kind == NodeKind::Project { 2 } else { 0 }
        + format!("#{} ", node.id).chars().count()
        + role.chars().count()
        + format!(" {{c{}}}", node.complexity).chars().count()
        + state.chars().count()
        + assignee
        + 2; // the space and the closing corner
    let title = short_title(&node.title, width.saturating_sub(overhead).max(4));
    let mut spans = vec![Span::styled("╭ ", outline_style)];
    if node.kind == NodeKind::Project {
        spans.push(badge("◆ ".to_owned(), theme::ACCENT));
    }
    spans.push(badge(format!("#{} ", node.id), theme::DIM));
    spans.push(badge(role.to_owned(), theme::TRACE));
    spans.push(Span::styled(title, title_style));
    spans.push(badge(format!(" {{c{}}}", node.complexity), theme::DIM));
    spans.push(badge(state, state_color(computed)));
    if let Some(assignee) = &node.assignee {
        spans.push(badge(format!(" [{}:{}]", assignee.role, assignee.model), theme::TRACE));
    }
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let filler = width.saturating_sub(used + 1);
    spans.push(Span::styled("─".repeat(filler), outline_style));
    spans.push(Span::styled("╮", outline_style));
    Line::from(spans)
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

/// One rounded bubble inside a card: the label set into the top edge, the wrapped text sealed
/// inside, the outline and the label in their own colors, `room` cells wide. The bubble touches
/// nothing else; the caller frames it with the card's outline.
fn field_bubble(label: &str, value: &str, frame: Style, label_color: ratatui::style::Color, room: usize) -> Vec<Line<'static>> {
    let label_style = Style::new().fg(label_color).bold().bg(theme::PANEL);
    let text = Style::new().fg(theme::TEXT).bg(theme::PANEL);
    let inner = room.saturating_sub(4).max(4);
    let head = format!(" {label} ");
    let head_width = head.chars().count();
    let mut rows = Vec::new();
    rows.push(Line::from(vec![
        Span::styled("╭", frame),
        Span::styled(head, label_style),
        Span::styled("─".repeat(inner.saturating_sub(head_width)), frame),
        Span::styled("╮", frame),
    ]));
    for piece in wrap(value, inner) {
        let pad = " ".repeat(inner.saturating_sub(piece.chars().count()));
        rows.push(Line::from(vec![
            Span::styled("│ ", frame),
            Span::styled(piece, text),
            Span::styled(pad, Style::new().bg(theme::PANEL)),
            Span::styled(" │", frame),
        ]));
    }
    rows.push(Line::from(vec![Span::styled("╰", frame), Span::styled("─".repeat(inner), frame), Span::styled("╯", frame)]));
    rows
}

/// The lifecycle buttons on one row: bracketed labels in cyan, brightening when the pointer rests
/// on one. Returns the spans and each button's columns within the row.
fn buttons_row(buttons: &[(Button, &str)], hover_button: Option<Button>) -> (Vec<Span<'static>>, Vec<(u16, u16, Button)>) {
    let cool = Style::new().fg(theme::DEBUG).bg(theme::PANEL);
    let hot = Style::new().fg(Color::Rgb(140, 240, 248)).bold().bg(theme::PANEL);
    let mut spans = Vec::new();
    let mut ranges = Vec::new();
    let mut offset = 0u16;
    for (i, (button, label)) in buttons.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", cool));
            offset += 2;
        }
        let start = offset;
        spans.push(Span::styled("[", cool));
        let style = if hover_button == Some(*button) { hot } else { cool };
        spans.push(Span::styled((*label).to_owned(), style));
        spans.push(Span::styled("]", cool));
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
    let shape = Shape { children, states: ra_roadmap::tree::effective_states(nodes) };
    let width = area.width as usize;
    let empty = std::collections::HashSet::new();
    let mut rows: Vec<Row> = Vec::new();
    for root in &roots {
        rows.extend(rows_of(root, &shape, folded, selected, &empty, None, 0, width));
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
            rows.extend(rows_of(root, &shape, folded, selected, &chain, hover_button, 0, width));
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
