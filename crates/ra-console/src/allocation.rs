//! A project's share of the machine as the window's dashboard changes it: its runners (the
//! cards a walk runs at once, `[worker] parallel`), its implementer (Fable or Opus, the first
//! `[[route]]`'s model) and its place in the machine's priority order. The table is edited line
//! by line, so its comments stay; a walk reads the runners and the model again before each card.
//! The person's ask of 2026-11-12: "change projects allocation in the gui".

use std::path::Path;

use anyhow::Context;

/// The two implementers the dashboard toggles between.
pub const FABLE: &str = "claude-fable-5-1";
pub const OPUS: &str = "claude-opus-5-5";

/// The most runners the dashboard's `+` sets.
pub const MAX_RUNNERS: u32 = 12;

/// The project's runners: `[worker] parallel`, one when it names none.
#[must_use]
pub fn runners(root: &Path) -> u32 {
    crate::router::Router::load(root).ok().and_then(|r| r.worker.parallel).unwrap_or(1).max(1)
}

/// The implementer the project's first route names, when it names one.
#[must_use]
pub fn model(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(crate::router::ROUTES_FILE)).ok()?;
    let mut in_route = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_route = trimmed == "[[route]]";
            continue;
        }
        if in_route && let Some(value) = key_value(trimmed, "model") {
            return Some(value.trim_matches('"').to_owned());
        }
    }
    None
}

/// Sets the project's runners, 1 to [`MAX_RUNNERS`].
///
/// # Errors
/// When the table cannot be read or written.
pub fn set_runners(root: &Path, runners: u32) -> anyhow::Result<()> {
    let runners = runners.clamp(1, MAX_RUNNERS);
    edit(root, |text| set_key(text, "[worker]", "parallel", &runners.to_string()))
}

/// Sets the implementer of the project's first route; a table without one gets one.
///
/// # Errors
/// When the table cannot be read or written.
pub fn set_model(root: &Path, model: &str) -> anyhow::Result<()> {
    edit(root, |text| set_key(text, "[[route]]", "model", &format!("\"{model}\"")))
}

/// Moves the project one place up (`up`) or down the machine's priority order; a project not in
/// the order joins it at the end first.
///
/// # Errors
/// When the hub's file cannot be written.
pub fn move_priority(root: &Path, up: bool) -> anyhow::Result<()> {
    let name = crate::hub::project_name(&crate::hub::project_root_of(root));
    let mut order = crate::hub::priority();
    let at = match order.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
        Some(at) => at,
        None => {
            order.push(name);
            order.len() - 1
        }
    };
    let to = if up { at.saturating_sub(1) } else { (at + 1).min(order.len() - 1) };
    order.swap(at, to);
    crate::hub::set_priority(&order)
}

/// The project's place in the priority order, 1 first; `None` when it is not in it.
#[must_use]
pub fn priority_place(root: &Path) -> Option<usize> {
    let name = crate::hub::project_name(&crate::hub::project_root_of(root));
    crate::hub::priority().iter().position(|n| n.eq_ignore_ascii_case(&name)).map(|at| at + 1)
}

/// The cards the project landed today and their cost, as its digest last said it, beside the
/// allocation so the person spends the week where a landed card is cheapest; a digest written on
/// another day says nothing of today.
#[must_use]
pub fn landed(root: &Path) -> (u32, Option<f64>) {
    let today = crate::hub::day_of(crate::hub::unix_now());
    crate::hub::own_digest(&crate::hub::project_root_of(root))
        .filter(|digest| crate::hub::day_of(digest.when) == today)
        .map_or((0, None), |digest| digest.landed_today)
}

/// Reads the project's table, changes it with `change` and writes it back.
fn edit(root: &Path, change: impl FnOnce(&str) -> String) -> anyhow::Result<()> {
    let path = root.join(crate::router::ROUTES_FILE);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let changed = change(&text);
    std::fs::write(&path, changed).with_context(|| format!("cannot write {}", path.display()))
}

/// The value of `key = value` on a line, when the line sets `key`.
fn key_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (name, value) = line.split_once('=')?;
    (name.trim() == key).then(|| value.split('#').next().unwrap_or(value).trim())
}

/// The table text with `key = value` in the first `header` section: the line replaced where it
/// is, else added under the header, else the section added at the end. For an array of tables
/// (`[[route]]`) the first entry is the one changed.
fn set_key(text: &str, header: &str, key: &str, value: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let start = lines.iter().position(|l| l.trim() == header);
    let Some(start) = start else {
        let mut out = text.trim_end().to_owned();
        if !out.is_empty() {
            out.push_str(newline);
            out.push_str(newline);
        }
        out.push_str(&format!("{header}{newline}{key} = {value}{newline}"));
        return out;
    };
    let end = lines.iter().enumerate().skip(start + 1).find(|(_, l)| l.trim_start().starts_with('[')).map_or(lines.len(), |(i, _)| i);
    let line = format!("{key} = {value}");
    match (start + 1..end).find(|&i| key_value(lines[i].trim(), key).is_some()) {
        Some(i) => {
            let indent: String = lines[i].chars().take_while(|c| c.is_whitespace()).collect();
            lines[i] = format!("{indent}{line}");
        }
        None => lines.insert(start + 1, line),
    }
    let mut out = lines.join(newline);
    if text.ends_with('\n') {
        out.push_str(newline);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_replaced_in_place_and_the_comments_stay() {
        let text = "# top\n[worker]\n# six at once\nparallel = 6 # the pool\nbuilds = 0\n\n[[route]]\nmodel = \"claude-opus-5-5\"\n";
        let out = set_key(text, "[worker]", "parallel", "4");
        assert_eq!(out, "# top\n[worker]\n# six at once\nparallel = 4\nbuilds = 0\n\n[[route]]\nmodel = \"claude-opus-5-5\"\n");
        let out = set_key(&out, "[[route]]", "model", "\"claude-fable-5-1\"");
        assert!(out.contains("[[route]]\nmodel = \"claude-fable-5-1\"\n"), "{out}");
    }

    #[test]
    fn a_missing_key_or_section_is_added() {
        assert_eq!(set_key("[worker]\nbuilds = 0\n", "[worker]", "parallel", "2"), "[worker]\nparallel = 2\nbuilds = 0\n");
        assert_eq!(set_key("[worker]\nbuilds = 0\n", "[[route]]", "model", "\"x\""), "[worker]\nbuilds = 0\n\n[[route]]\nmodel = \"x\"\n");
        assert_eq!(set_key("", "[worker]", "parallel", "3"), "[worker]\nparallel = 3\n");
    }

    #[test]
    fn the_model_is_read_from_the_first_route() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(crate::router::ROUTES_FILE), "[worker]\nmodel = \"no\"\n[[route]]\nmodel = \"claude-fable-5-1\"\n")
            .unwrap();
        assert_eq!(model(dir.path()).as_deref(), Some(FABLE));
        set_model(dir.path(), OPUS).unwrap();
        assert_eq!(model(dir.path()).as_deref(), Some(OPUS));
        set_runners(dir.path(), 40).unwrap();
        assert_eq!(runners(dir.path()), MAX_RUNNERS);
    }
}
