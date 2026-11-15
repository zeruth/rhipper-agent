//! A review or lint point quotes the line it flags, so a merge from main that moves the line
//! does not leave the point naming the wrong one: the handoff finds the text again and rewrites
//! the number, or marks the point `resolved?` when the text is gone.

#[cfg(test)]
mod tests;

/// Opens the quoted text of a point: ` › <the flagged line, trimmed>`.
const QUOTE: &str = " › ";

/// What a point whose text is no longer in its file is marked with.
pub const RESOLVED: &str = " resolved?";

/// The `file:line` a point opens with, its rest, and the line number's position in the point.
fn location(point: &str) -> Option<(&str, usize, &str)> {
    let (file, rest) = point.split_once(':')?;
    if file.is_empty() || file.contains(char::is_whitespace) {
        return None;
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let line = rest[..digits].parse().ok()?;
    Some((file, line, &rest[digits..]))
}

/// The point with the text of the line it flags appended, when `read` gives the file and the
/// point names a line of it; the point as it was otherwise, or when it already quotes one.
pub fn quote(point: &str, read: impl Fn(&str) -> Option<String>) -> String {
    if point.contains(QUOTE) {
        return point.to_owned();
    }
    let Some((file, line, _)) = location(point) else { return point.to_owned() };
    let text = read(file).and_then(|body| body.lines().nth(line.wrapping_sub(1)).map(|l| l.trim().to_owned()));
    match text {
        Some(text) if !text.is_empty() => format!("{point}{QUOTE}{text}"),
        _ => point.to_owned(),
    }
}

/// The point with its line number rewritten to where its quoted text stands now: the nearest
/// line to the old number when the text occurs more than once, and `resolved?` after the quote
/// when no line has it. A point without a quote, or without a location, is left as it was.
pub fn relocate(point: &str, read: impl Fn(&str) -> Option<String>) -> String {
    let Some((head, text)) = point.rsplit_once(QUOTE) else { return point.to_owned() };
    let Some((file, line, rest)) = location(head) else { return point.to_owned() };
    let text = text.trim_end_matches(RESOLVED).trim();
    let Some(body) = read(file) else { return format!("{head}{QUOTE}{text}{RESOLVED}") };
    let found = body.lines().enumerate().filter(|(_, l)| l.trim() == text).map(|(i, _)| i + 1).min_by_key(|n| n.abs_diff(line));
    match found {
        Some(n) => format!("{file}:{n}{rest}{QUOTE}{text}"),
        None => format!("{head}{QUOTE}{text}{RESOLVED}"),
    }
}

/// Every point of `points` relocated; see [`relocate`].
pub fn relocate_all(points: &[String], read: impl Fn(&str) -> Option<String>) -> Vec<String> {
    points.iter().map(|point| relocate(point, &read)).collect()
}
