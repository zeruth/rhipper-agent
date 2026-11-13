//! The report: every card a run ended on in the chosen span, across the open projects, one row
//! a card with its verdict, the reviewer's first line or the cause, how many runs it took, what
//! they cost, how much of it went on runs that did not land, and the time they ran. Grouped as
//! landed, sent back and yours, so it says where the money went and not only what came of it:
//! on four cards of 2026-11-12, $8.61 of $9.93 went on runs that did not land. Read from each
//! project's pair feed, `var/pair.log`, whose lines are `<secs>\t<event>`.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use eframe::egui;
use egui::{Align, Color32, Frame, Layout, Margin, RichText};
use ra_roadmap::model::TaskState;

use crate::metrics::number_after;
use crate::{App, cards, dollars, theme};

#[cfg(test)]
mod tests;

/// How far back the report reaches: the runs that ended inside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Span {
    #[default]
    Day,
    Week,
    Month,
}

impl Span {
    pub(crate) const ALL: [Self; 3] = [Self::Day, Self::Week, Self::Month];

    fn name(self) -> &'static str {
        match self {
            Self::Day => "24 h",
            Self::Week => "7 days",
            Self::Month => "30 days",
        }
    }

    pub(crate) fn secs(self) -> u64 {
        match self {
            Self::Day => 86_400,
            Self::Week => 7 * 86_400,
            Self::Month => 30 * 86_400,
        }
    }
}

/// Where a card's last word left it, in the order the report draws the groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Group {
    /// Merged into the project.
    Landed,
    /// Back with its worker: a failed run, a rejecting review, a check or a merge that failed.
    SentBack,
    /// Waiting on the person: a review to accept, a parked card, a reviewer that did not answer.
    Yours,
}

impl Group {
    fn name(self) -> &'static str {
        match self {
            Self::Landed => "landed",
            Self::SentBack => "sent back",
            Self::Yours => "yours",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::Landed => theme::state_color(TaskState::Accepted),
            Self::SentBack => theme::state_color(TaskState::NeedsRevision),
            Self::Yours => theme::warn(),
        }
    }
}

/// One row of the report: a card with a run that ended in the span.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    /// The index of the project's tab.
    pub tab: usize,
    pub project: String,
    pub id: i64,
    pub title: String,
    pub group: Group,
    /// The verdict in a word or two: `landed`, `implemented`, `accepted`, `needs revision`,
    /// `failed`, `not landed`, `waits`, `parked`, `no verdict`.
    pub verdict: String,
    /// The reviewer's first line, or the cause the card was sent back or waits for.
    pub sentence: String,
    /// The runs that ended in the span.
    pub runs: u32,
    /// What those runs and their reviews cost.
    pub cost: f64,
    /// The part of `cost` spent on runs that did not land: every run but the one that landed.
    pub not_landed: f64,
    /// The seconds those runs ran, start to end, summed.
    pub wall: u64,
    /// When the card's last word was said.
    pub at: u64,
}

/// One run of a card as the feed has it.
#[derive(Default)]
struct Run {
    start: Option<u64>,
    end: u64,
    cost: f64,
    landed: bool,
}

/// What the feed said of one card.
#[derive(Default)]
struct Card {
    title: String,
    runs: Vec<Run>,
    open: Option<u64>,
    word: Option<(Group, String, String, u64)>,
}

/// The rows of one project's feed: the cards with a run that ended at or after `since`.
pub(crate) fn rows_of(feed: &str, since: u64, tab: usize, project: &str) -> Vec<Row> {
    let mut cards: HashMap<i64, Card> = HashMap::new();
    for line in feed.lines() {
        let Some((at, body)) = line.split_once('\t') else { continue };
        let Ok(at) = at.trim().parse::<u64>() else { continue };
        read_event(&mut cards, at, body.trim_end());
    }
    let mut rows: Vec<Row> = cards
        .into_iter()
        .filter_map(|(id, card)| {
            let runs: Vec<&Run> = card.runs.iter().filter(|r| r.end >= since).collect();
            if runs.is_empty() {
                return None;
            }
            let last_end = runs.iter().map(|r| r.end).max().unwrap_or(0);
            let (group, verdict, sentence, at) = card.word.unwrap_or((Group::Yours, "ended".to_owned(), String::new(), last_end));
            Some(Row {
                tab,
                project: project.to_owned(),
                id,
                title: card.title,
                group,
                verdict,
                sentence,
                runs: u32::try_from(runs.len()).unwrap_or(u32::MAX),
                cost: runs.iter().map(|r| r.cost).sum(),
                not_landed: runs.iter().filter(|r| !r.landed).map(|r| r.cost).sum(),
                wall: runs.iter().map(|r| r.start.map_or(0, |s| r.end.saturating_sub(s))).sum(),
                at: at.max(last_end),
            })
        })
        .collect();
    order(&mut rows, false);
    rows
}

/// Folds one event of the feed into the cards it speaks of; a line about no card is skipped.
fn read_event(cards: &mut HashMap<i64, Card>, at: u64, body: &str) {
    if let Some(rest) = body.strip_prefix("start #") {
        let Some((id, rest)) = card_of(rest) else { return };
        let card = cards.entry(id).or_default();
        card.title = rest.split(" · ").next().unwrap_or_default().trim().to_owned();
        card.open = Some(at);
    } else if let Some(rest) = body.strip_prefix("end ") {
        let ok = rest.starts_with('✓');
        let Some((id, rest)) = rest.find('#').and_then(|i| card_of(&rest[i + 1..])) else { return };
        let card = cards.entry(id).or_default();
        let start = card.open.take();
        card.runs.push(Run { start, end: at, cost: number_after(body, "$").unwrap_or(0.0), landed: false });
        let said = after_title(rest, &card.title);
        let word = if ok {
            (Group::Yours, "implemented", sentence_of(said))
        } else if let Some(why) = said.strip_prefix("failed:") {
            (Group::SentBack, "failed", why.trim().to_owned())
        } else {
            (Group::SentBack, "needs revision", sentence_of(said))
        };
        card.word = Some((word.0, word.1.to_owned(), word.2, at));
    } else if let Some(rest) = body.strip_prefix("review ") {
        let ok = rest.starts_with('✓');
        let Some((id, rest)) = rest.find('#').and_then(|i| card_of(&rest[i + 1..])) else { return };
        let card = cards.entry(id).or_default();
        if let Some(run) = card.runs.last_mut() {
            run.cost += number_after(body, "$").unwrap_or(0.0);
        }
        let said = after_title(rest, &card.title);
        let word = if let Some(why) = said.strip_prefix("no verdict:") {
            (Group::Yours, "no verdict", why.trim().to_owned())
        } else if ok {
            (Group::Yours, "accepted", sentence_of(said))
        } else {
            (Group::SentBack, "needs revision", sentence_of(said))
        };
        card.word = Some((word.0, word.1.to_owned(), word.2, at));
    } else if let Some(rest) = body.strip_prefix("landed #") {
        let Some((id, rest)) = card_of(rest) else { return };
        let card = cards.entry(id).or_default();
        if let Some(run) = card.runs.last_mut() {
            run.landed = true;
        }
        let said = after_title(rest, &card.title);
        card.word = Some((Group::Landed, "landed".to_owned(), sentence_of(said), at));
    } else if let Some(rest) = body.strip_prefix("rerun #").or_else(|| body.strip_prefix("waits #")) {
        let Some((id, rest)) = card_of(rest) else { return };
        let why = rest.split(" · ").nth(1).unwrap_or_default().trim().to_owned();
        let word = if body.starts_with("rerun") { (Group::SentBack, "needs revision") } else { (Group::Yours, "waits") };
        cards.entry(id).or_default().word = Some((word.0, word.1.to_owned(), why, at));
    } else if let Some(rest) = body.strip_prefix("parked #") {
        let Some((id, rest)) = card_of(rest) else { return };
        let why = rest.trim_start_matches(':').trim().to_owned();
        cards.entry(id).or_default().word = Some((Group::Yours, "parked".to_owned(), why, at));
    } else if let Some(rest) = body.strip_prefix('#') {
        let Some((id, rest)) = card_of(rest) else { return };
        let Some(why) = rest.strip_prefix("not landed") else { return };
        let why = why.trim_start_matches(':').trim().to_owned();
        // a merge the walk could not send back is landed by hand
        let group = if why.contains("by hand") { Group::Yours } else { Group::SentBack };
        cards.entry(id).or_default().word = Some((group, "not landed".to_owned(), why, at));
    }
}

/// The card id at the head of `text` (after its `#`) and the text after it.
fn card_of(text: &str) -> Option<(i64, &str)> {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let id = text[..end].parse().ok()?;
    Some((id, text[end..].trim_start_matches(':').trim_start()))
}

/// `text` without the card's title in front, when it is there.
fn after_title<'a>(text: &'a str, title: &str) -> &'a str {
    if title.is_empty() { text } else { text.strip_prefix(title).unwrap_or(text).trim_start() }
}

/// The words of an event without its numbers: the turns, the tokens, the read-in and the cost
/// are the row's own columns.
fn sentence_of(text: &str) -> String {
    let numbers = ["turns ", "in ", "read-in ", "$", "squashed"];
    text.split(" · ").filter(|part| !numbers.iter().any(|n| part.trim().starts_with(n))).collect::<Vec<_>>().join(" · ").trim().to_owned()
}

/// Orders the rows: by group, newest first within it; `by_project` lays each group's rows out
/// in the tabs' order, newest first within a project.
pub(crate) fn order(rows: &mut [Row], by_project: bool) {
    rows.sort_by_key(|r| (r.group, if by_project { r.tab } else { 0 }, std::cmp::Reverse(r.at), r.tab, r.id));
}

/// Seconds as the row shows a run's time: `40s`, `12m`, `1h 05m`.
fn wall_text(secs: u64) -> String {
    if secs < 60 { format!("{secs}s") } else { ra_roadmap::history::age(secs) }
}

impl App {
    /// The report: every card a run ended on in the span, as bubbles in two columns under a
    /// divider per group. A click on a bubble opens the project's tab with the card selected.
    pub(crate) fn report_view(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.rebuild_report(now);
        let mut by_project = self.tabs.first().is_some_and(|t| t.report_by_project);
        let mut span = self.report_span;
        let mut open: Option<(usize, i64)> = None;
        let rows = &self.report_rows;
        egui::CentralPanel::default().frame(Frame::new().fill(theme::bg()).inner_margin(Margin::symmetric(18, 12))).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("REPORT").small().strong().color(theme::dim()));
                let cost: f64 = rows.iter().map(|r| r.cost).sum();
                let lost: f64 = rows.iter().map(|r| r.not_landed).sum();
                let text = format!(
                    "{} cards across {} open · {} of {} on runs that did not land",
                    rows.len(),
                    self.tabs.len(),
                    dollars(Some(lost)),
                    dollars(Some(cost))
                );
                ui.label(RichText::new(text).small().color(theme::dim()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if cards::tab(ui, "by time", theme::accent(), !by_project) {
                        by_project = false;
                    }
                    if cards::tab(ui, "by project", theme::accent(), by_project) {
                        by_project = true;
                    }
                    ui.add_space(10.0);
                    for choice in Span::ALL.into_iter().rev() {
                        if cards::tab(ui, choice.name(), theme::accent(), span == choice) {
                            span = choice;
                        }
                    }
                });
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().id_salt("report").auto_shrink([false, false]).show(ui, |ui| {
                // the page's width, handed down: the columns are sized from it, so no bubble
                // takes the room a scroll area's content would otherwise be allowed
                let page = ui.available_width();
                ui.set_width(page);
                let column = ((page - 10.0) / 2.0).max(200.0);
                if rows.is_empty() {
                    ui.label(RichText::new(format!("no run ended in the last {}", span.name())).color(theme::dim()));
                }
                for (g, group) in rows.chunk_by(|a, b| a.group == b.group).enumerate() {
                    let color = group[0].group.color();
                    if g > 0 {
                        ui.add_space(4.0);
                    }
                    ui.horizontal(|ui| {
                        cards::badge(ui, group[0].group.name(), color);
                        let lost: f64 = group.iter().map(|r| r.not_landed).sum();
                        let text = format!("{} cards · {} not landed", group.len(), dollars(Some(lost)));
                        ui.label(RichText::new(text).small().color(theme::dim()));
                    });
                    cards::divider(ui, color);
                    for pair in group.chunks(2) {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            for item in pair {
                                ui.allocate_ui_with_layout(egui::vec2(column, 0.0), Layout::top_down(Align::Min), |ui| {
                                    ui.set_width(column);
                                    if row_bubble(ui, item, now) {
                                        open = Some((item.tab, item.id));
                                    }
                                });
                            }
                        });
                    }
                }
            });
        });
        self.report_span = span;
        if self.tabs.first().is_some_and(|t| t.report_by_project != by_project) {
            // every project's layout keeps the choice, so whichever opens first reads it back
            for tab in &mut self.tabs {
                tab.report_by_project = by_project;
                tab.save_layout();
            }
            self.report_from = None;
        }
        if let Some((tab, id)) = open
            && tab < self.tabs.len()
        {
            self.active = tab;
            self.report = false;
            self.tabs[tab].select(Some(id));
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Rebuilds the report's rows when a tab's roadmap or ledger moved since the last build, a
    /// tab came or went, or the span changed; a frame that finds them where they were draws the
    /// rows it has.
    fn rebuild_report(&mut self, now: u64) {
        let stamps: Vec<_> = self.tabs.iter().map(|t| (t.roadmap_seen, t.ledger.read_at)).collect();
        let from = (stamps, self.report_span);
        if self.report_from.as_ref() == Some(&from) {
            return;
        }
        let since = now.saturating_sub(self.report_span.secs());
        self.report_from = Some(from);
        let mut rows = Vec::new();
        for (tab, workspace) in self.tabs.iter().enumerate() {
            let project = workspace
                .project_title
                .clone()
                .unwrap_or_else(|| workspace.root.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()));
            let feed = std::fs::read_to_string(workspace.root.join("var").join("pair.log")).unwrap_or_default();
            for mut row in rows_of(&feed, since, tab, &project) {
                // the roadmap's title over the feed's: a card renamed since its run reads as it is now
                if let Some(node) = workspace.roadmap.iter().find(|n| n.id == row.id) {
                    row.title.clone_from(&node.title);
                }
                rows.push(row);
            }
        }
        order(&mut rows, self.tabs.first().is_some_and(|t| t.report_by_project));
        self.report_rows = rows;
    }
}

/// The bubble of one row: the project as a small badge, the id and the title; the verdict, the
/// runs, the cost and the part of it that did not land, the time the runs took and how long ago
/// the last word was said; the sentence under them. Returns true when clicked.
fn row_bubble(ui: &mut egui::Ui, item: &Row, now: u64) -> bool {
    let lane = theme::lane(1 + item.tab % 9);
    let shown = cards::bubble(ui, lane, false, |ui| {
        let inner = ui.available_width();
        ui.set_width(inner);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            cards::badge(ui, &item.project, lane);
            ui.label(RichText::new(format!("#{}", item.id)).small().strong().color(cards::alpha(theme::text(), 180)));
            let title = egui::Label::new(RichText::new(&item.title).color(theme::text())).truncate();
            ui.add(title).on_hover_text(format!("#{} {}", item.id, item.title));
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            cards::badge(ui, &item.verdict, item.group.color());
            let runs = if item.runs == 1 { "1 run".to_owned() } else { format!("{} runs", item.runs) };
            ui.label(RichText::new(runs).small().color(theme::dim()));
            ui.label(RichText::new(dollars(Some(item.cost))).small().color(theme::dim()));
            if item.not_landed > 0.0 {
                ui.label(RichText::new(format!("{} not landed", dollars(Some(item.not_landed)))).small().color(theme::warn()))
                    .on_hover_text("what the runs that did not land cost, their reviews with them");
            }
            ui.label(RichText::new(wall_text(item.wall)).small().color(theme::dim())).on_hover_text("the time the runs took, start to end");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let when = format!("{} ago", ra_roadmap::history::age(now.saturating_sub(item.at)));
                ui.label(RichText::new(when).small().color(theme::dim()));
            });
        });
        if !item.sentence.is_empty() {
            let sentence = egui::Label::new(RichText::new(&item.sentence).small().color(theme::dim())).truncate();
            ui.add(sentence).on_hover_text(&item.sentence);
        }
    });
    ui.interact(shown.response.rect, ui.make_persistent_id(("report", item.tab, item.id)), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}
