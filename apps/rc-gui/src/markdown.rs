//! The notes of a roadmap card as Markdown: headings, paragraphs with inline code, bold, italic
//! and links, lists, quotes, rules, and fenced code with a light syntax coloring. The parser is
//! a pure function over the text, so it is tested without a window; the renderer paints the
//! blocks it returns with the theme's colors.
//!
//! The worker's notes are a report and then the checks it ran: a `check failed:` line, the
//! command, and the raw output of the command. The parser knows that shape, lights the check
//! line in the outcome's color, names the failed tests on it, and folds the raw output under
//! it as a code block; the rest of the notes stay open.

use std::path::Path;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align, Color32, CornerRadius, CursorIcon, FontId, Frame, Label, Layout, Margin, RichText, Sense, Shape, Stroke, TextStyle,
    TextWrapMode, Ui,
};

use crate::{cards, theme};

/// A run of inline text with one formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Inline {
    /// Plain words.
    Text(String),
    /// A span between backticks.
    Code(String),
    /// A span between double stars.
    Bold(String),
    /// A span between single stars or underscores.
    Italic(String),
    /// A bare URL.
    Link(String),
}

/// One item of a list: its marker, its depth (0 or 1) and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    /// `None` for a bullet, the number as written for a numbered item.
    number: Option<String>,
    depth: u8,
    spans: Vec<Inline>,
}

/// A block of the document, in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    /// A heading of level 1 to 3.
    Heading { level: u8, spans: Vec<Inline> },
    /// Lines of prose joined into one paragraph.
    Paragraph(Vec<Inline>),
    /// A run of list items, bullets and numbers mixed as written.
    List(Vec<Item>),
    /// A run of `>` lines.
    Quote(Vec<Inline>),
    /// A horizontal rule.
    Rule,
    /// A fenced code block.
    Code { lang: Option<String>, lines: Vec<String> },
    /// A `check failed:` or `check passed:` line, with what follows the colon; a failed check
    /// keeps the raw output that follows it and the names of the tests that failed in it.
    Check { passed: bool, text: String, output: Vec<String>, failed: Vec<String> },
}

/// What a line is before it joins a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blank,
    Heading,
    Fence,
    Rule,
    Quote,
    ListItem,
    Check,
    Text,
}

/// Tells the kind of a line from its first characters.
fn kind(line: &str) -> Kind {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Kind::Blank
    } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        Kind::Fence
    } else if heading(trimmed).is_some() {
        Kind::Heading
    } else if is_rule(trimmed) {
        Kind::Rule
    } else if trimmed.starts_with('>') {
        Kind::Quote
    } else if list_item(line).is_some() {
        Kind::ListItem
    } else if check(trimmed).is_some() {
        Kind::Check
    } else {
        Kind::Text
    }
}

/// The level and the text of a heading line, if it is one.
fn heading(trimmed: &str) -> Option<(u8, &str)> {
    let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    let level = u8::try_from(hashes.min(3)).unwrap_or(3);
    Some((level, rest.trim()))
}

/// Whether a line is a horizontal rule: three or more of `-`, `*` or `_`, spaces allowed.
fn is_rule(trimmed: &str) -> bool {
    let marks: Vec<char> = trimmed.chars().filter(|c| *c != ' ').collect();
    marks.len() >= 3 && ['-', '*', '_'].iter().any(|m| marks.iter().all(|c| c == m))
}

/// The depth, the number and the text of a list item line, if it is one.
fn list_item(line: &str) -> Option<(u8, Option<String>, &str)> {
    let indent = line.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>();
    let rest = line.trim_start();
    let depth = u8::from(indent >= 2);
    if let Some(text) = ["- ", "* ", "+ "].iter().find_map(|m| rest.strip_prefix(m)) {
        return Some((depth, None, text.trim()));
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && digits <= 3 {
        let after = &rest[digits..];
        if let Some(text) = after.strip_prefix(". ").or_else(|| after.strip_prefix(") ")) {
            return Some((depth, Some(rest[..digits].to_owned()), text.trim()));
        }
    }
    None
}

/// The outcome and the text of a check line, if it is one.
fn check(trimmed: &str) -> Option<(bool, &str)> {
    let lower = trimmed.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("check failed:") {
        Some((false, trimmed[trimmed.len() - rest.len()..].trim()))
    } else if let Some(rest) = lower.strip_prefix("check passed:") {
        Some((true, trimmed[trimmed.len() - rest.len()..].trim()))
    } else {
        None
    }
}

/// The ways compilers and test runners open their lines.
const OPENERS: [&str; 18] = [
    "error",
    "warning",
    "note:",
    "help:",
    "-->",
    "test ",
    "thread ",
    "failures",
    "panicked",
    "FAILED",
    "running ",
    "Compiling",
    "Checking",
    "Finished",
    "Running",
    "Doc-tests",
    "---- ",
    "$ ",
];

/// Whether a line after a blank looks like command output rather than prose: it is indented,
/// or it opens the way compilers and test runners open their lines.
fn looks_raw(line: &str) -> bool {
    line.starts_with(' ') || line.starts_with('\t') || OPENERS.iter().any(|o| line.starts_with(o)) || line.contains("::")
}

/// The index of the next line that is not blank, from `from`.
fn next_nonblank(lines: &[&str], from: usize) -> Option<usize> {
    (from..lines.len()).find(|i| !lines[*i].trim().is_empty())
}

/// Parses the text into blocks.
fn parse(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        match kind(line) {
            Kind::Blank => i += 1,
            Kind::Heading => {
                let (level, text) = heading(trimmed).unwrap_or((1, trimmed));
                blocks.push(Block::Heading { level, spans: inlines(text) });
                i += 1;
            }
            Kind::Rule => {
                blocks.push(Block::Rule);
                i += 1;
            }
            Kind::Fence => {
                let lang = trimmed.trim_start_matches(['`', '~']).trim();
                let lang = (!lang.is_empty()).then(|| lang.to_owned());
                let mut body = Vec::new();
                i += 1;
                while i < lines.len() && kind(lines[i]) != Kind::Fence {
                    body.push(lines[i].to_owned());
                    i += 1;
                }
                // the closing fence, when there is one
                i += 1;
                blocks.push(Block::Code { lang, lines: body });
            }
            Kind::Quote => {
                let mut text = String::new();
                while i < lines.len() && kind(lines[i]) == Kind::Quote {
                    let inner = lines[i].trim().trim_start_matches('>').trim();
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(inner);
                    i += 1;
                }
                blocks.push(Block::Quote(inlines(&text)));
            }
            Kind::ListItem => blocks.push(Block::List(list(&lines, &mut i))),
            Kind::Check => {
                let (passed, text) = check(trimmed).unwrap_or((false, trimmed));
                i += 1;
                let output = if passed { Vec::new() } else { raw_output(&lines, &mut i) };
                let failed = failed_tests(&output);
                blocks.push(Block::Check { passed, text: text.to_owned(), output, failed });
            }
            Kind::Text => {
                let mut text = String::new();
                while i < lines.len() && kind(lines[i]) == Kind::Text {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(lines[i].trim());
                    i += 1;
                }
                blocks.push(Block::Paragraph(inlines(&text)));
            }
        }
    }
    blocks
}

/// Collects the list that starts at `lines[*i]` and moves `i` past it: the items, the indented
/// prose that continues an item, and the blanks between items while an item follows.
fn list(lines: &[&str], i: &mut usize) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    while *i < lines.len() {
        match kind(lines[*i]) {
            Kind::ListItem => {
                let (depth, number, text) = list_item(lines[*i]).unwrap_or((0, None, ""));
                items.push(Item { number, depth, spans: inlines(text) });
                *i += 1;
            }
            Kind::Text if lines[*i].starts_with(' ') || lines[*i].starts_with('\t') => {
                if let Some(last) = items.last_mut() {
                    last.spans.push(Inline::Text(" ".to_owned()));
                    last.spans.extend(inlines(lines[*i].trim()));
                }
                *i += 1;
            }
            Kind::Blank => match next_nonblank(lines, *i) {
                Some(next) if kind(lines[next]) == Kind::ListItem => *i = next,
                _ => break,
            },
            _ => break,
        }
    }
    items
}

/// Collects the raw output after a failed check, from `lines[*i]`, and moves `i` past it: the
/// lines straight after the check, and past a blank only while they still look like output.
fn raw_output(lines: &[&str], i: &mut usize) -> Vec<String> {
    let mut raw: Vec<String> = Vec::new();
    while *i < lines.len() {
        match kind(lines[*i]) {
            Kind::Text => {
                raw.push(lines[*i].to_owned());
                *i += 1;
            }
            Kind::Blank => match next_nonblank(lines, *i) {
                Some(next) if kind(lines[next]) == Kind::Text && looks_raw(lines[next]) => {
                    raw.extend((*i..next).map(|_| String::new()));
                    *i = next;
                }
                _ => break,
            },
            _ => break,
        }
    }
    raw
}

/// The names of the tests that failed in a check's output: the `test path::name ... FAILED`
/// lines the runner prints, the last segment of each path.
fn failed_tests(output: &[String]) -> Vec<String> {
    output
        .iter()
        .filter_map(|line| {
            let path = line.trim().strip_prefix("test ")?.strip_suffix("... FAILED")?.trim();
            Some(path.rsplit("::").next().unwrap_or(path).to_owned())
        })
        .collect()
}

/// Splits a line of prose into its inline runs.
fn inlines(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Inline> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    let flush = |plain: &mut String, spans: &mut Vec<Inline>| {
        if !plain.is_empty() {
            spans.push(Inline::Text(std::mem::take(plain)));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let at_word_start = i == 0 || chars[i - 1].is_whitespace() || "([{\"'".contains(chars[i - 1]);
        if c == '`' {
            if let Some(end) = (i + 1..chars.len()).find(|j| chars[*j] == '`') {
                flush(&mut plain, &mut spans);
                spans.push(Inline::Code(chars[i + 1..end].iter().collect()));
                i = end + 1;
                continue;
            }
        } else if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            if let Some(end) = find_pair(&chars, i + 2, "**")
                && end > i + 2
            {
                flush(&mut plain, &mut spans);
                spans.push(Inline::Bold(chars[i + 2..end].iter().collect()));
                i = end + 2;
                continue;
            }
        } else if (c == '*' || c == '_') && at_word_start && i + 1 < chars.len() && !chars[i + 1].is_whitespace() {
            let closer = c.to_string();
            if let Some(end) = find_pair(&chars, i + 1, &closer) {
                let closes_word = end + 1 == chars.len() || !chars[end + 1].is_alphanumeric();
                if end > i + 1 && !chars[end - 1].is_whitespace() && (c == '*' || closes_word) {
                    flush(&mut plain, &mut spans);
                    spans.push(Inline::Italic(chars[i + 1..end].iter().collect()));
                    i = end + 1;
                    continue;
                }
            }
        } else if at_word_start && (starts_with(&chars, i, "http://") || starts_with(&chars, i, "https://")) {
            let mut end = (i..chars.len()).find(|j| chars[*j].is_whitespace()).unwrap_or(chars.len());
            // the punctuation that closes a sentence is not part of the link
            while end > i && ".,;:!?)]}>'\"".contains(chars[end - 1]) {
                end -= 1;
            }
            flush(&mut plain, &mut spans);
            spans.push(Inline::Link(chars[i..end].iter().collect()));
            i = end;
            continue;
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut plain, &mut spans);
    spans
}

/// Whether `chars` carries `needle` at `at`.
fn starts_with(chars: &[char], at: usize, needle: &str) -> bool {
    let needle: Vec<char> = needle.chars().collect();
    chars.len() >= at + needle.len() && chars[at..at + needle.len()] == needle[..]
}

/// The index where `needle` next appears, from `from`.
fn find_pair(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    (from..chars.len()).find(|j| starts_with(chars, *j, needle))
}

// ---------------------------------------------------------------------------------------------
// rendering

/// The fonts and the color a run of prose takes.
#[derive(Clone)]
struct Prose {
    font: FontId,
    mono: FontId,
    color: Color32,
    italics: bool,
}

impl Prose {
    /// The body text, as the style sets it.
    fn body(ui: &Ui) -> Self {
        let font = TextStyle::Body.resolve(ui.style());
        let mut mono = TextStyle::Monospace.resolve(ui.style());
        mono.size = font.size - 1.0;
        Self { font, mono, color: theme::text(), italics: false }
    }

    /// The format of one inline run.
    fn format(&self, span: &Inline) -> TextFormat {
        let mut format = TextFormat { font_id: self.font.clone(), color: self.color, italics: self.italics, ..Default::default() };
        match span {
            Inline::Text(_) | Inline::Link(_) => {}
            Inline::Code(_) => {
                format.font_id = self.mono.clone();
                format.color = theme::mix(self.color, theme::warn_text(), 0.35);
                format.background = theme::panel_raised();
                format.expand_bg = 2.0;
            }
            Inline::Bold(_) => {
                format.color = theme::mix(self.color, theme::light(), 0.5);
                format.coords.push(b"wght", 700.0);
            }
            Inline::Italic(_) => format.italics = true,
        }
        format
    }
}

/// The text of a run.
fn text_of(span: &Inline) -> &str {
    match span {
        Inline::Text(t) | Inline::Code(t) | Inline::Bold(t) | Inline::Italic(t) | Inline::Link(t) => t,
    }
}

/// One layout job over `spans`, links as plain text.
fn job(spans: &[Inline], prose: &Prose) -> LayoutJob {
    let mut job = LayoutJob::default();
    for span in spans {
        job.append(text_of(span), 0.0, prose.format(span));
    }
    job
}

/// Draws `spans`: one wrapped label when there is no link, else the runs around each link as
/// labels and the link as a hyperlink, flowing together.
fn spans_ui(ui: &mut Ui, spans: &[Inline], prose: &Prose) {
    if !spans.iter().any(|s| matches!(s, Inline::Link(_))) {
        ui.add(Label::new(job(spans, prose)).wrap_mode(TextWrapMode::Wrap));
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut run: Vec<Inline> = Vec::new();
        for span in spans {
            if let Inline::Link(url) = span {
                let trimmed = trim_run(std::mem::take(&mut run));
                if !trimmed.is_empty() {
                    ui.add(Label::new(job(&trimmed, prose)).wrap_mode(TextWrapMode::Wrap));
                }
                ui.hyperlink_to(RichText::new(url).font(prose.font.clone()).color(theme::accent()), url);
            } else {
                run.push(span.clone());
            }
        }
        let trimmed = trim_run(run);
        if !trimmed.is_empty() {
            ui.add(Label::new(job(&trimmed, prose)).wrap_mode(TextWrapMode::Wrap));
        }
    });
}

/// Trims the spaces off both ends of a run, which the item spacing provides instead.
fn trim_run(mut run: Vec<Inline>) -> Vec<Inline> {
    if let Some(Inline::Text(t)) = run.first_mut() {
        *t = t.trim_start().to_owned();
    }
    if let Some(Inline::Text(t)) = run.last_mut() {
        *t = t.trim_end().to_owned();
    }
    run.retain(|s| !text_of(s).is_empty());
    run
}

/// Renders `text` as Markdown into `ui`, headings in `accent`.
pub(crate) fn render(ui: &mut Ui, text: &str, accent: Color32) {
    let blocks = parse(text);
    let prose = Prose::body(ui);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        for (index, block) in blocks.iter().enumerate() {
            block_ui(ui, block, index, accent, &prose);
        }
    });
}

/// Renders the text a run sends: the lines of the brief each their own paragraph, the indented
/// rows of the file lists as code, the outlines as links, and the Markdown of the handoffs as
/// it is. The prompt is written for a model, one fact a line; joined into paragraphs it would
/// not read. Returns the file and the line of the outline item that was clicked, for the code
/// pane to open.
pub(crate) fn render_prompt(ui: &mut Ui, text: &str, accent: Color32) -> Option<(String, usize)> {
    let prose = Prose::body(ui);
    let mut clicked = None;
    for (index, segment) in segments(text).iter().enumerate() {
        match segment {
            Segment::Prose(prose_text) => render(ui, &prompt_markdown(prose_text), accent),
            Segment::Outline(outline) => {
                if let Some(line) = outline_ui(ui, outline, index, &prose) {
                    clicked = Some((outline.file.clone(), line));
                }
            }
        }
    }
    clicked
}

/// A stretch of the prompt: prose, or the outline of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Prose(String),
    Outline(Outline),
}

/// The outline of one file as the prompt carries it: the header `path (N lines) — doc`, a row
/// per item with its line number, a `*` ahead of an item a run of the card changed, and the
/// count of the items the outline left out.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Outline {
    file: String,
    /// What follows the path on the header line: the line count and the doc line.
    head: String,
    items: Vec<OutlineItem>,
    /// The `… and N more` trailer, when the file has more items than the outline shows.
    more: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutlineItem {
    line: usize,
    changed: bool,
    text: String,
}

/// Cuts the outlines out of the prompt, each with the rows that follow its header; what is
/// between them stays prose.
fn segments(text: &str) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut prose = String::new();
    let mut outline: Option<Outline> = None;
    for line in text.lines() {
        if let Some(current) = &mut outline {
            if let Some((number, changed, item)) = outline_row(line) {
                current.items.push(OutlineItem { line: number, changed, text: item.to_owned() });
                continue;
            }
            if current.more.is_none()
                && let Some(more) = outline_more(line)
            {
                current.more = Some(more.to_owned());
                continue;
            }
            out.push(Segment::Outline(outline.take().expect("an outline is open")));
        }
        if let Some((file, head)) = outline_header(line) {
            if !prose.is_empty() {
                out.push(Segment::Prose(std::mem::take(&mut prose)));
            }
            outline = Some(Outline { file: file.to_owned(), head: head.to_owned(), items: Vec::new(), more: None });
            continue;
        }
        prose.push_str(line);
        prose.push('\n');
    }
    if let Some(outline) = outline {
        out.push(Segment::Outline(outline));
    }
    if !prose.is_empty() {
        out.push(Segment::Prose(prose));
    }
    out
}

/// The path and the rest of an outline's header line, `src/lib.rs (120 lines) — The crate`.
fn outline_header(line: &str) -> Option<(&str, &str)> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let (file, rest) = line.split_once(" (")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0
        && !file.is_empty()
        && !file.contains(' ')
        && (rest[digits..].starts_with(" lines)") || rest[digits..].starts_with(" line)")))
    .then(|| (file, &line[file.len() + 1..]))
}

/// The line number, the change mark and the text of an outline row, `*    42  fn run()`.
fn outline_row(line: &str) -> Option<(usize, bool, &str)> {
    let changed = match line.as_bytes().first()? {
        b' ' => false,
        b'*' => true,
        _ => return None,
    };
    let rest = line[1..].trim_start_matches(' ');
    if rest.len() == line.len() - 1 {
        return None;
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let number = rest[..digits].parse().ok()?;
    let text = rest[digits..].strip_prefix("  ")?;
    Some((number, changed, text.trim_end()))
}

/// The trailer that counts the items an outline left out.
fn outline_more(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    (line.starts_with(' ') && trimmed.starts_with("… and ") && trimmed.ends_with(" more")).then_some(trimmed)
}

/// Draws one outline as the code blocks are framed, every row a link: the number in the
/// gutter, the item colored as code, lit as the pointer passes. Returns the line of the row
/// that was clicked, line 1 for the header.
fn outline_ui(ui: &mut Ui, outline: &Outline, index: usize, prose: &Prose) -> Option<usize> {
    let mut clicked = None;
    let language = Language::of(Path::new(&outline.file).extension().and_then(|e| e.to_str()));
    let width = ui.available_width();
    Frame::new()
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, theme::rule()))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(width - 20.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let name =
                    Label::new(RichText::new(&outline.file).monospace().strong().color(theme::text()).underline()).sense(Sense::click());
                if ui.add(name).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("open the file beside the card").clicked() {
                    clicked = Some(1);
                }
                ui.label(RichText::new(&outline.head).small().color(theme::dim()));
            });
            if outline.items.is_empty() && outline.more.is_none() {
                return;
            }
            ui.add_space(2.0);
            egui::ScrollArea::horizontal().id_salt(("outline", index)).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for item in &outline.items {
                    // the fill goes under the row, so a hovered row lights behind its text
                    let fill = ui.painter().add(Shape::Noop);
                    let mut job = LayoutJob::default();
                    let mark = if item.changed { "*" } else { " " };
                    job.append(
                        &format!("{mark}{:>5}  ", item.line),
                        0.0,
                        TextFormat::simple(
                            prose.mono.clone(),
                            if item.changed { theme::warn_text() } else { cards::alpha(theme::dim(), 170) },
                        ),
                    );
                    for (text, color) in language.tokens(&item.text) {
                        job.append(text, 0.0, TextFormat::simple(prose.mono.clone(), color));
                    }
                    let response = ui
                        .add(Label::new(job).wrap_mode(TextWrapMode::Extend).sense(Sense::click()))
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(format!("open {} at line {}", outline.file, item.line));
                    if response.hovered() {
                        let rect = response.rect.expand2(egui::vec2(4.0, 0.0));
                        ui.painter().set(fill, Shape::rect_filled(rect, 3.0, theme::debug().gamma_multiply(0.18)));
                    }
                    if response.clicked() {
                        clicked = Some(item.line);
                    }
                }
                if let Some(more) = &outline.more {
                    ui.label(RichText::new(format!("       {more}")).font(prose.mono.clone()).color(theme::dim()));
                }
            });
        });
    clicked
}

/// The prompt as Markdown: a blank after every plain line, a fence around every run of
/// indented rows, and a fenced block left as it came.
fn prompt_markdown(text: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    let mut in_rows = false;
    for line in text.lines() {
        let fence = kind(line) == Kind::Fence;
        if in_fence {
            out.push_str(line);
            out.push('\n');
            in_fence = !fence;
            continue;
        }
        let row = line.starts_with("  ") && !line.trim().is_empty();
        if row != in_rows {
            out.push_str("```\n");
            in_rows = row;
        }
        if row {
            out.push_str(line.trim_end());
            out.push('\n');
        } else if fence {
            in_fence = true;
            out.push_str(line);
            out.push('\n');
        } else {
            out.push_str(line.trim());
            out.push_str("\n\n");
        }
    }
    if in_rows {
        out.push_str("```\n");
    }
    out
}

/// Draws one block.
fn block_ui(ui: &mut Ui, block: &Block, index: usize, accent: Color32, prose: &Prose) {
    match block {
        Block::Heading { level, spans } => {
            let size = match level {
                1 => 20.0,
                2 => 17.0,
                _ => 15.0,
            };
            let mut heading = prose.clone();
            heading.font = FontId::proportional(size);
            heading.mono = FontId::monospace(size - 1.0);
            heading.color = accent;
            let mut job = job(spans, &heading);
            for section in &mut job.sections {
                section.format.coords.push(b"wght", 700.0);
            }
            if *level == 1 {
                ui.add_space(2.0);
            }
            ui.add(Label::new(job).wrap_mode(TextWrapMode::Wrap));
        }
        Block::Paragraph(spans) => spans_ui(ui, spans, prose),
        Block::List(items) => {
            ui.spacing_mut().item_spacing.y = 3.0;
            for item in items {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add_space(4.0 + 16.0 * f32::from(item.depth));
                    let marker =
                        item.number.as_ref().map_or_else(|| if item.depth == 0 { "•" } else { "◦" }.to_owned(), |n| format!("{n}."));
                    ui.label(RichText::new(marker).color(accent).font(prose.font.clone()));
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width());
                        spans_ui(ui, &item.spans, prose);
                    });
                });
            }
            ui.spacing_mut().item_spacing.y = 6.0;
        }
        Block::Quote(spans) => {
            let mut quote = prose.clone();
            quote.color = theme::mix(theme::text(), theme::dim(), 0.5);
            quote.italics = true;
            let response = ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width());
                    spans_ui(ui, spans, &quote);
                });
            });
            let rect = response.response.rect;
            let x = rect.left() + 3.0;
            ui.painter().line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], Stroke::new(2.0, theme::rule()));
        }
        Block::Rule => {
            ui.separator();
        }
        Block::Code { lang, lines } => code_ui(ui, lang.as_deref(), lines, index, prose),
        Block::Check { passed, text, output, failed } => {
            let color = if *passed { theme::online() } else { theme::error_text() };
            let mut job = LayoutJob::default();
            let mut head = TextFormat::simple(prose.font.clone(), color);
            head.coords.push(b"wght", 700.0);
            job.append(if *passed { "check passed:" } else { "check failed:" }, 0.0, head);
            job.append(text, 6.0, TextFormat::simple(prose.mono.clone(), theme::text()));
            // the failed tests on the header line, so the output can stay folded
            if !failed.is_empty() {
                job.append("·", 6.0, TextFormat::simple(prose.font.clone(), theme::dim()));
                job.append(&failed.join(", "), 6.0, TextFormat::simple(prose.mono.clone(), color));
            }
            if output.is_empty() {
                ui.add(Label::new(job).wrap_mode(TextWrapMode::Wrap));
            } else {
                egui::CollapsingHeader::new(job).id_salt(("notes-check", index)).default_open(false).show(ui, |ui| {
                    code_ui(ui, None, output, index, prose);
                });
            }
        }
    }
}

/// Draws the diff of one file folded under its name: a line in the color of what it does,
/// added green, removed red, a hunk header in the debug tint, the file headers dim, the rest
/// as they are. Returns whether the name was clicked, to open the file beside the card; a
/// file without lines says so instead of folding nothing.
pub(crate) fn render_diff(ui: &mut Ui, file: &str, lines: &[String], index: usize) -> bool {
    let prose = Prose::body(ui);
    let (added, removed) = lines.iter().fold((0usize, 0usize), |(a, r), line| match diff_tint(line) {
        DiffLine::Added => (a + 1, r),
        DiffLine::Removed => (a, r + 1),
        _ => (a, r),
    });
    let mut clicked = false;
    let header = egui::CollapsingHeader::new(RichText::new(file).strong().color(theme::text()))
        .id_salt(("diff-file", index))
        .default_open(true)
        .show(ui, |ui| {
            if lines.is_empty() {
                ui.label(
                    RichText::new("no change git can show: committed since, or a file git does not know yet").color(theme::dim()).small(),
                );
                return;
            }
            let mut job = LayoutJob::default();
            for (n, line) in lines.iter().enumerate() {
                if n > 0 {
                    job.append("\n", 0.0, TextFormat::simple(prose.mono.clone(), theme::text()));
                }
                let color = match diff_tint(line) {
                    DiffLine::Added => theme::online(),
                    DiffLine::Removed => theme::error_text(),
                    DiffLine::Hunk => theme::debug(),
                    DiffLine::Header => theme::dim(),
                    DiffLine::Plain => theme::text(),
                };
                job.append(line, 0.0, TextFormat::simple(prose.mono.clone(), color));
            }
            Frame::new()
                .fill(theme::panel())
                .stroke(Stroke::new(1.0, theme::rule()))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 20.0);
                    egui::ScrollArea::horizontal().id_salt(("diff-code", index)).auto_shrink([false, true]).show(ui, |ui| {
                        ui.add(Label::new(job).wrap_mode(TextWrapMode::Extend));
                    });
                });
        });
    // the counts sit on the header's row, at the right; a double click on the header opens
    // the file, so a single click keeps folding
    if header.header_response.double_clicked() {
        clicked = true;
    }
    let counts = header.header_response.rect;
    let text = format!("+{added} −{removed}");
    let galley = ui.painter().layout_no_wrap(text, prose.mono.clone(), theme::dim());
    let pos = egui::pos2(ui.max_rect().right() - galley.size().x - 8.0, counts.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, theme::dim());
    header.header_response.on_hover_text("double-click to open the file beside the card");
    clicked
}

/// What a line of a unified diff does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DiffLine {
    Added,
    Removed,
    Hunk,
    Header,
    Plain,
}

/// The kind of a line of `git diff`: the file headers before the first hunk count as headers,
/// not as removed and added lines.
fn diff_tint(line: &str) -> DiffLine {
    const HEADERS: [&str; 8] =
        ["+++ ", "--- ", "diff --git", "index ", "new file mode", "deleted file mode", "similarity index", "rename "];
    if HEADERS.iter().any(|h| line.starts_with(h)) {
        DiffLine::Header
    } else if line.starts_with("@@") {
        DiffLine::Hunk
    } else if line.starts_with('+') {
        DiffLine::Added
    } else if line.starts_with('-') {
        DiffLine::Removed
    } else {
        DiffLine::Plain
    }
}

/// Draws a code block: a rounded frame, the language tag at the top right, the lines colored
/// and left as they are, scrolling sideways when they are wider than the card.
fn code_ui(ui: &mut Ui, lang: Option<&str>, lines: &[String], index: usize, prose: &Prose) {
    let width = ui.available_width();
    Frame::new()
        .fill(theme::panel())
        .stroke(Stroke::new(1.0, theme::rule()))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(width - 20.0);
            if let Some(lang) = lang {
                ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                    ui.label(RichText::new(lang).small().color(theme::dim()));
                });
            }
            let mut job = LayoutJob::default();
            let language = Language::of(lang);
            for (n, line) in lines.iter().enumerate() {
                if n > 0 {
                    job.append("\n", 0.0, TextFormat::simple(prose.mono.clone(), theme::text()));
                }
                // a line of a tool's output that says how things went is lit whole: an error
                // or a failed test in red, a warning in amber, a pass in green, a location dim
                if let Some(color) = outcome_tint(line) {
                    job.append(line, 0.0, TextFormat::simple(prose.mono.clone(), color));
                    continue;
                }
                for (text, color) in language.tokens(line) {
                    job.append(text, 0.0, TextFormat::simple(prose.mono.clone(), color));
                }
            }
            egui::ScrollArea::horizontal().id_salt(("notes-code", index)).auto_shrink([false, true]).show(ui, |ui| {
                ui.add(Label::new(job).wrap_mode(TextWrapMode::Extend));
            });
        });
}

/// The color a line of a tool's output gets whole, by what it says: `None` for a line that is
/// only code or text.
fn outcome_tint(line: &str) -> Option<Color32> {
    let trimmed = line.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.starts_with("-->") || trimmed.starts_with("= note") || trimmed.starts_with("= help") {
        return Some(theme::dim());
    }
    if lower.starts_with("error") || lower.contains("panicked at") || lower.contains(" failed") || lower.starts_with("failures:") {
        return Some(theme::error_text());
    }
    if lower.starts_with("warning") || trimmed.starts_with('⚠') {
        return Some(theme::warn_text());
    }
    if lower.starts_with("test result: ok") || lower.ends_with(" ok") || trimmed.starts_with('✓') || lower.starts_with("check passed") {
        return Some(theme::online());
    }
    if trimmed.starts_with('✗') || lower.starts_with("check failed") {
        return Some(theme::error_text());
    }
    None
}

// ---------------------------------------------------------------------------------------------
// syntax coloring

/// The little a code block knows about its language: the comment opener, the keywords, and
/// whether a key before `=` or `:` is a keyword too. The code pane colors a whole file with it.
#[derive(Clone, Copy)]
pub(crate) struct Language {
    comment: Option<&'static str>,
    keywords: &'static [&'static str],
    keyed: Option<char>,
    single_quotes: bool,
    char_literals: bool,
}

const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl",
    "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
    "true", "type", "unsafe", "use", "where", "while",
];
const PYTHON: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for",
    "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
    "yield", "True", "False", "None",
];
const SHELL: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "in", "function", "return", "export", "local",
    "param", "foreach", "switch", "try", "catch", "finally", "throw", "begin", "process", "end",
];
const SCRIPT: &[&str] = &[
    "const",
    "let",
    "var",
    "function",
    "return",
    "if",
    "else",
    "for",
    "while",
    "class",
    "new",
    "this",
    "import",
    "export",
    "from",
    "async",
    "await",
    "try",
    "catch",
    "throw",
    "true",
    "false",
    "null",
    "undefined",
    "interface",
    "type",
];
const CFAMILY: &[&str] = &[
    "if", "else", "for", "while", "return", "struct", "class", "public", "private", "static", "void", "int", "new", "true", "false",
    "null", "fun", "val", "var",
];
const VALUES: &[&str] = &["true", "false", "null"];

impl Language {
    /// The language of a tag or a file extension, the plain one when it is unknown or missing.
    pub(crate) fn of(tag: Option<&str>) -> Self {
        let plain = Self { comment: None, keywords: &[], keyed: None, single_quotes: false, char_literals: false };
        let quoted = |comment, keywords, keyed| Self { comment: Some(comment), keywords, keyed, single_quotes: true, char_literals: false };
        match tag.map(str::to_ascii_lowercase).as_deref() {
            Some("rs" | "rust") => Self { comment: Some("//"), keywords: RUST, keyed: None, single_quotes: false, char_literals: true },
            Some("py" | "python") => quoted("#", PYTHON, None),
            Some("ps1" | "powershell" | "pwsh" | "sh" | "bash" | "shell" | "zsh") => quoted("#", SHELL, None),
            Some("toml") => quoted("#", VALUES, Some('=')),
            Some("yaml" | "yml") => quoted("#", VALUES, Some(':')),
            Some("js" | "ts" | "javascript" | "typescript") => quoted("//", SCRIPT, None),
            Some("c" | "cpp" | "java" | "kt" | "kotlin") => quoted("//", CFAMILY, None),
            _ => plain,
        }
    }

    /// Cuts one line into colored pieces, in order, covering the whole line.
    pub(crate) fn tokens<'a>(&self, line: &'a str) -> Vec<(&'a str, Color32)> {
        let mut out: Vec<(&'a str, Color32)> = Vec::new();
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut plain_from = 0;
        // a toml table header is a key by itself
        if self.keyed == Some('=') && line.trim_start().starts_with('[') {
            return vec![(line, theme::trace())];
        }
        while i < bytes.len() {
            let c = bytes[i];
            let token: Option<(usize, Color32)> = if self.comment.is_some_and(|opener| line[i..].starts_with(opener)) {
                Some((bytes.len(), theme::dim()))
            } else if c == b'"' || (c == b'\'' && self.single_quotes) {
                Some((string_end(bytes, i), theme::warn_text()))
            } else if c == b'\'' && self.char_literals {
                // a char literal closes within a few bytes; a lifetime does not close at all
                let end = string_end(bytes, i);
                (end - i <= 4 && bytes.get(end - 1) == Some(&b'\'')).then_some((end, theme::warn_text()))
            } else if c.is_ascii_digit() && (i == 0 || !is_word(bytes[i - 1])) {
                let mut end = i;
                while end < bytes.len() && (is_word(bytes[end]) || bytes[end] == b'.') {
                    end += 1;
                }
                Some((end, theme::warn_text()))
            } else if c.is_ascii_alphabetic() || c == b'_' {
                let mut end = i;
                while end < bytes.len() && is_word(bytes[end]) {
                    end += 1;
                }
                let word = &line[i..end];
                let is_key = self.keyed.is_some_and(|sep| line[..i].trim().is_empty() && line[end..].trim_start().starts_with(sep));
                let color = if is_key || self.keywords.contains(&word) {
                    theme::trace()
                } else if c.is_ascii_uppercase() {
                    theme::debug()
                } else {
                    theme::text()
                };
                Some((end, color))
            } else {
                None
            };
            match token {
                Some((end, color)) => {
                    if plain_from < i {
                        out.push((&line[plain_from..i], theme::text()));
                    }
                    out.push((&line[i..end], color));
                    i = end;
                    plain_from = end;
                }
                None => i += 1,
            }
        }
        if plain_from < bytes.len() {
            out.push((&line[plain_from..], theme::text()));
        }
        out
    }
}

/// Whether a byte continues an identifier.
fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The index past the closing quote of the string that opens at `from`, or the end of the line.
fn string_end(bytes: &[u8], from: usize) -> usize {
    let quote = bytes[from];
    let mut i = from + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_owned())
    }

    #[test]
    fn a_diff_line_is_told_by_its_first_characters() {
        assert_eq!(diff_tint("diff --git a/x.rs b/x.rs"), DiffLine::Header);
        assert_eq!(diff_tint("--- a/x.rs"), DiffLine::Header);
        assert_eq!(diff_tint("+++ b/x.rs"), DiffLine::Header);
        assert_eq!(diff_tint("@@ -1,2 +1,3 @@ fn main"), DiffLine::Hunk);
        assert_eq!(diff_tint("+let a = 1;"), DiffLine::Added);
        assert_eq!(diff_tint("-let a = 0;"), DiffLine::Removed);
        assert_eq!(diff_tint(" let b = 2;"), DiffLine::Plain);
    }

    #[test]
    fn headings_take_their_level_and_text() {
        let blocks = parse("# Title\n## Second\n### Third\n#### Deep\n#not a heading");
        assert_eq!(blocks[0], Block::Heading { level: 1, spans: vec![text("Title")] });
        assert_eq!(blocks[1], Block::Heading { level: 2, spans: vec![text("Second")] });
        assert_eq!(blocks[2], Block::Heading { level: 3, spans: vec![text("Third")] });
        assert_eq!(blocks[3], Block::Heading { level: 3, spans: vec![text("Deep")] });
        assert_eq!(blocks[4], Block::Paragraph(vec![text("#not a heading")]));
    }

    #[test]
    fn a_list_keeps_its_items_and_their_depth() {
        let blocks = parse("- one\n- two\n  - nested\n1. first\n2) second\n\nafter");
        let Block::List(items) = &blocks[0] else { panic!("not a list: {blocks:?}") };
        assert_eq!(items.len(), 5);
        assert_eq!(items[0], Item { number: None, depth: 0, spans: vec![text("one")] });
        assert_eq!(items[2], Item { number: None, depth: 1, spans: vec![text("nested")] });
        assert_eq!(items[3], Item { number: Some("1".to_owned()), depth: 0, spans: vec![text("first")] });
        assert_eq!(items[4].number.as_deref(), Some("2"));
        assert_eq!(blocks[1], Block::Paragraph(vec![text("after")]));
    }

    #[test]
    fn a_prompt_keeps_its_lines_apart_and_its_rows_as_code() {
        let prompt = "Task #3: Title\nObjective: do it\nFiles of the card:\n  src/a.rs\n  src/b.rs\n\nEarlier runs:\n## Handoff 1 · m · implemented\ncheck: ok\n```\nraw\n```\n";
        let blocks = parse(&prompt_markdown(prompt));
        assert_eq!(blocks[0], Block::Paragraph(vec![text("Task #3: Title")]));
        assert_eq!(blocks[1], Block::Paragraph(vec![text("Objective: do it")]));
        assert_eq!(blocks[2], Block::Paragraph(vec![text("Files of the card:")]));
        assert_eq!(blocks[3], Block::Code { lang: None, lines: vec!["  src/a.rs".to_owned(), "  src/b.rs".to_owned()] });
        assert_eq!(blocks[4], Block::Paragraph(vec![text("Earlier runs:")]));
        assert!(matches!(&blocks[5], Block::Heading { level: 2, .. }));
        assert_eq!(blocks[6], Block::Paragraph(vec![text("check: ok")]));
        assert_eq!(blocks[7], Block::Code { lang: None, lines: vec!["raw".to_owned()] });
    }

    #[test]
    fn the_outlines_of_a_prompt_are_cut_out_with_their_rows() {
        let prompt = "Files: read the regions the outline points at.\nsrc/lib.rs (120 lines) — The crate\n      9  mod cards\n*    41  pub fn run(root: &Path) -> Self\n         … and 3 more\nsrc/new.rs (not in the workspace yet)\nUnder #1 Root: no objective\n";
        let segments = segments(prompt);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0], Segment::Prose("Files: read the regions the outline points at.\n".to_owned()));
        let Segment::Outline(outline) = &segments[1] else { panic!("not an outline: {segments:?}") };
        assert_eq!(outline.file, "src/lib.rs");
        assert_eq!(outline.head, "(120 lines) — The crate");
        assert_eq!(outline.items[0], OutlineItem { line: 9, changed: false, text: "mod cards".to_owned() });
        assert_eq!(outline.items[1], OutlineItem { line: 41, changed: true, text: "pub fn run(root: &Path) -> Self".to_owned() });
        assert_eq!(outline.more.as_deref(), Some("… and 3 more"));
        // a missing file and the lines after are prose, not an outline
        assert_eq!(segments[2], Segment::Prose("src/new.rs (not in the workspace yet)\nUnder #1 Root: no objective\n".to_owned()));
    }

    #[test]
    fn an_outline_row_needs_the_gutter_and_a_line_number() {
        assert_eq!(outline_row("     12  fn x()"), Some((12, false, "fn x()")));
        assert_eq!(outline_row("*     3  struct S"), Some((3, true, "struct S")));
        assert_eq!(outline_row("  src/a.rs"), None);
        assert_eq!(outline_row("12  fn x()"), None);
        assert_eq!(outline_header("a (b) c"), None);
        assert_eq!(outline_header("Task #3 (the third)"), None);
        assert_eq!(outline_header("README.md (1 line)"), Some(("README.md", "(1 line)")));
    }

    #[test]
    fn a_fenced_block_keeps_its_language_and_indentation() {
        let blocks = parse("before\n```rust\nfn main() {\n    let x = 1;\n}\n```\nafter");
        assert_eq!(blocks[0], Block::Paragraph(vec![text("before")]));
        assert_eq!(
            blocks[1],
            Block::Code {
                lang: Some("rust".to_owned()),
                lines: vec!["fn main() {".to_owned(), "    let x = 1;".to_owned(), "}".to_owned()],
            }
        );
        assert_eq!(blocks[2], Block::Paragraph(vec![text("after")]));
    }

    #[test]
    fn an_unclosed_fence_runs_to_the_end_without_a_language() {
        let blocks = parse("```\nraw\nmore");
        assert_eq!(blocks, vec![Block::Code { lang: None, lines: vec!["raw".to_owned(), "more".to_owned()] }]);
    }

    #[test]
    fn inline_runs_split_on_code_bold_italic_and_links() {
        let spans = inlines("run `cargo test` now, **really** *soon*, see https://example.com/x. Done");
        assert_eq!(
            spans,
            vec![
                text("run "),
                Inline::Code("cargo test".to_owned()),
                text(" now, "),
                Inline::Bold("really".to_owned()),
                text(" "),
                Inline::Italic("soon".to_owned()),
                text(", see "),
                Inline::Link("https://example.com/x".to_owned()),
                text(". Done"),
            ]
        );
    }

    #[test]
    fn underscores_inside_words_stay_plain() {
        assert_eq!(inlines("the snake_case_name and a * b"), vec![text("the snake_case_name and a * b")]);
        assert_eq!(inlines("an unclosed `tick"), vec![text("an unclosed `tick")]);
    }

    #[test]
    fn a_failed_check_keeps_its_raw_output_under_its_line() {
        let notes = "Fixed the parser.\n\ncheck failed: cargo test -p x\nerror[E0308]: mismatched types\n  --> src/lib.rs:4:5\n\ntest result: FAILED. 1 failed\n\nThe fix is in the second commit.\ncheck passed: cargo fmt --check";
        let blocks = parse(notes);
        assert_eq!(blocks[0], Block::Paragraph(vec![text("Fixed the parser.")]));
        assert_eq!(
            blocks[1],
            Block::Check {
                passed: false,
                text: "cargo test -p x".to_owned(),
                output: vec![
                    "error[E0308]: mismatched types".to_owned(),
                    "  --> src/lib.rs:4:5".to_owned(),
                    String::new(),
                    "test result: FAILED. 1 failed".to_owned(),
                ],
                failed: Vec::new(),
            }
        );
        assert_eq!(blocks[2], Block::Paragraph(vec![text("The fix is in the second commit.")]));
        assert_eq!(blocks[3], Block::Check { passed: true, text: "cargo fmt --check".to_owned(), output: Vec::new(), failed: Vec::new() });
        assert_eq!(blocks.len(), 4);
    }

    #[test]
    fn a_failed_check_names_the_tests_that_failed() {
        let notes = "check failed: cargo test --workspace -q\ntest store::tests::a_run_records_its_handoff ... FAILED\ntest agent::tests::a_check_names_its_failures ... FAILED\nthread 'x' panicked at src/a.rs:1:1\n\ntest result: FAILED. 2 failed\n\nThe report.";
        let blocks = parse(notes);
        let Block::Check { passed, failed, output, .. } = &blocks[0] else { panic!("not a check: {blocks:?}") };
        assert!(!passed);
        assert_eq!(failed, &["a_run_records_its_handoff".to_owned(), "a_check_names_its_failures".to_owned()]);
        assert_eq!(output.len(), 5);
        assert_eq!(blocks[1], Block::Paragraph(vec![text("The report.")]));
    }

    #[test]
    fn quotes_and_rules_are_their_own_blocks() {
        let blocks = parse("> a thought\n> goes on\n---\ntext");
        assert_eq!(blocks[0], Block::Quote(vec![text("a thought goes on")]));
        assert_eq!(blocks[1], Block::Rule);
        assert_eq!(blocks[2], Block::Paragraph(vec![text("text")]));
    }

    #[test]
    fn code_tokens_cover_the_line_and_color_the_parts() {
        let rust = Language::of(Some("rs"));
        let tokens = rust.tokens("let Name = \"hi\"; // 42");
        let rebuilt: String = tokens.iter().map(|(t, _)| *t).collect();
        assert_eq!(rebuilt, "let Name = \"hi\"; // 42");
        assert_eq!(tokens[0], ("let", theme::trace()));
        assert_eq!(tokens[2], ("Name", theme::debug()));
        assert!(tokens.contains(&("\"hi\"", theme::warn_text())));
        assert_eq!(tokens.last().copied(), Some(("// 42", theme::dim())));

        let toml = Language::of(Some("toml"));
        assert_eq!(toml.tokens("[package]"), vec![("[package]", theme::trace())]);
        assert_eq!(toml.tokens("name = \"x\"")[0], ("name", theme::trace()));

        let plain = Language::of(None);
        assert_eq!(
            plain.tokens("error: 3 tests FAILED"),
            vec![
                ("error", theme::text()),
                (": ", theme::text()),
                ("3", theme::warn_text()),
                (" ", theme::text()),
                ("tests", theme::text()),
                (" ", theme::text()),
                ("FAILED", theme::debug()),
            ]
        );
    }
}
