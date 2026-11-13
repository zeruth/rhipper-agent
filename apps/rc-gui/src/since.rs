//! What changed since the person last looked at a project (observability, section 5): the runs
//! of its feed that ended after the seen mark in `var/seen`, counted into landed, failed on
//! their first run, conflicts and planned cards, and the band that draws them under the
//! project's header on the dashboard. Opening the project's tab or pressing `seen` writes the
//! mark; the cards that wait for the person count whatever the mark, and clear only when they move.

use std::collections::HashMap;
use std::path::Path;

use eframe::egui;
use egui::{Align, Layout, RichText};

use crate::{cards, theme};

#[cfg(test)]
mod tests;

/// The project's seen mark: one unix time, the last time the person looked.
pub(crate) const SEEN_FILE: &str = "var/seen";

/// What changed in one project since the seen mark.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Since {
    /// The mark the counts are taken after; 0 when the person never looked.
    pub at: u64,
    /// Runs that landed, and what they cost.
    pub landed: u32,
    pub landed_usd: f64,
    /// First runs of a card that ended without landing, and what they cost.
    pub failed_first: u32,
    pub failed_usd: f64,
    /// Runs that hit a merge conflict.
    pub conflicts: u32,
    /// Cards that wait for the person, whatever the mark.
    pub yours: u32,
    /// Cards a plan or a look-back pass added.
    pub planned: u32,
    /// The cards behind each count, for the hovers.
    pub landed_cards: Vec<i64>,
    pub failed_cards: Vec<i64>,
    pub conflict_cards: Vec<i64>,
}

impl Since {
    /// Whether nothing happened since the mark and nothing waits.
    pub(crate) fn quiet(&self) -> bool {
        self.landed == 0 && self.failed_first == 0 && self.conflicts == 0 && self.yours == 0 && self.planned == 0
    }
}

/// The seen mark of the project at `root`; 0 when there is none.
pub(crate) fn seen(root: &Path) -> u64 {
    std::fs::read_to_string(root.join(SEEN_FILE)).ok().and_then(|text| text.trim().parse().ok()).unwrap_or(0)
}

/// Writes `at` as the project's seen mark.
pub(crate) fn mark(root: &Path, at: u64) -> std::io::Result<()> {
    let path = root.join(SEEN_FILE);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, at.to_string())
}

/// Marks the project at `root` seen now; a mark that cannot be written leaves the band as it was.
pub(crate) fn mark_now(root: &Path) {
    let _ = mark(root, now());
}

/// The unix time now.
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The project's counts since its mark, from its feed; `yours` cards wait for the person.
pub(crate) fn of(root: &Path, yours: u32) -> Since {
    let feed = std::fs::read_to_string(root.join("var").join("pair.log")).unwrap_or_default();
    count(&feed, seen(root), yours)
}

/// One run of a card as the count follows it.
#[derive(Default)]
struct Run {
    card: i64,
    nth: u32,
    /// The time of its last word: an end, a review, a landing or a word against it.
    ended: Option<u64>,
    usd: f64,
    landed: bool,
    failed: bool,
    conflict: bool,
}

/// The counts of the stamped `feed` (`time<TAB>text`) after the mark `seen`: every run whose
/// last word came after it, each numbered over the whole feed so a first run is told from a
/// rerun; `yours` is carried as it is.
pub(crate) fn count(feed: &str, seen: u64, yours: u32) -> Since {
    let mut runs: Vec<Run> = Vec::new();
    let mut latest: HashMap<i64, usize> = HashMap::new();
    let mut since = Since { at: seen, yours, ..Since::default() };
    for line in feed.lines() {
        let Some((at, body)) = line.split_once('\t') else { continue };
        let Ok(at) = at.trim().parse::<u64>() else { continue };
        if at > seen {
            since.planned += planned_of(body);
        }
        if let Some(rest) = body.strip_prefix("start #") {
            let Some(card) = card_id(rest) else { continue };
            let nth = latest.get(&card).map_or(1, |&i| runs[i].nth + 1);
            latest.insert(card, runs.len());
            runs.push(Run { card, nth, ..Run::default() });
            continue;
        }
        let conflict_pass = body.starts_with('#') && body.contains(" conflict pass by ");
        let sent_back = body.starts_with('#') && (body.contains(" not reviewed: ") || body.contains(" not landed: "));
        let said = body.starts_with("end ") || body.starts_with("review ") || body.starts_with("landed #") || sent_back || conflict_pass;
        if !said {
            continue;
        }
        let Some(card) = body.find('#').and_then(|i| card_id(&body[i + 1..])) else { continue };
        let Some(&i) = latest.get(&card) else { continue };
        let run = &mut runs[i];
        run.ended = Some(at);
        if body.starts_with("end ") {
            run.usd += crate::metrics::usd_of(body).unwrap_or(0.0);
            run.failed |= body.starts_with("end ✗");
        } else if let Some(rest) = body.strip_prefix("review ") {
            run.usd += crate::metrics::usd_of(body).unwrap_or(0.0);
            run.failed |= !rest.starts_with('✓');
        } else if body.starts_with("landed #") {
            run.landed = true;
        } else if conflict_pass {
            run.conflict = true;
        } else {
            run.failed = true;
            run.conflict |= body.contains(" not landed: ") && !body.contains("merged tree fails");
        }
    }
    for run in runs.iter().filter(|r| r.ended.is_some_and(|at| at > seen)) {
        if run.landed {
            since.landed += 1;
            since.landed_usd += run.usd;
            since.landed_cards.push(run.card);
        } else if run.failed && run.nth == 1 {
            since.failed_first += 1;
            since.failed_usd += run.usd;
            since.failed_cards.push(run.card);
        }
        if run.conflict {
            since.conflicts += 1;
            since.conflict_cards.push(run.card);
        }
    }
    since
}

/// The cards a plan or a look-back pass added by its `added N planned card(s)` line.
fn planned_of(body: &str) -> u32 {
    let Some(at) = body.find("added ") else { return 0 };
    let rest = &body[at + "added ".len()..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    if rest[end..].starts_with(" planned card(s)") { rest[..end].parse().unwrap_or(0) } else { 0 }
}

/// The card id `text` starts with.
fn card_id(text: &str) -> Option<i64> {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    text[..end].parse().ok()
}

/// How long ago `at` was from `now`, in a word: `12m`, `3h 05m`, `2d`.
fn ago(now: u64, at: u64) -> String {
    let minutes = now.saturating_sub(at) / 60;
    match minutes {
        m if m < 60 => format!("{m}m"),
        m if m < 24 * 60 => format!("{}h {:02}m", m / 60, m % 60),
        m => format!("{}d", m / (24 * 60)),
    }
}

/// The ids as `#1 #2 #3`.
fn ids(cards: &[i64]) -> String {
    cards.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(" ")
}

/// The band under a project's header: the counts since the mark, each a badge with its cards on
/// hover, `yours` lit with the waiting lines; returns true when the person pressed `seen`.
pub(crate) fn band(ui: &mut egui::Ui, since: &Since, waiting: &[String]) -> bool {
    let mut pressed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let when = if since.at == 0 { "since ever".to_owned() } else { format!("since {} ago", ago(now(), since.at)) };
        ui.label(RichText::new(when).small().color(theme::dim()));
        if since.quiet() {
            ui.label(RichText::new("nothing new").small().italics().color(theme::dim()));
        }
        if since.landed > 0 {
            cards::badge_marked(ui, "▲", &format!("{} landed ${:.2}", since.landed, since.landed_usd), theme::online())
                .on_hover_text(format!("landed: {}", ids(&since.landed_cards)));
        }
        if since.failed_first > 0 {
            cards::badge_marked(ui, "✗", &format!("{} failed first run ${:.2}", since.failed_first, since.failed_usd), theme::error_text())
                .on_hover_text(format!("first runs that did not land: {}", ids(&since.failed_cards)));
        }
        if since.conflicts > 0 {
            cards::badge_marked(ui, "⟲", &format!("{} conflict", since.conflicts), theme::warn_text())
                .on_hover_text(format!("runs that met a conflict: {}", ids(&since.conflict_cards)));
        }
        if since.yours > 0 {
            cards::badge_marked(ui, "⏸", &format!("{} yours", since.yours), theme::warn())
                .on_hover_text(format!("{}\n\nclears when the card moves, not on seen", waiting.join("\n")));
        }
        if since.planned > 0 {
            cards::badge(ui, &format!("planned {}", since.planned), theme::dim()).on_hover_text("cards a plan or a look-back pass added");
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let button = egui::Button::new(RichText::new("seen").small().color(theme::text()));
            pressed = ui.add(button).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("count from now").clicked();
        });
    });
    pressed
}
