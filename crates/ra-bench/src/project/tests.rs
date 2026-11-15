use std::path::Path;
use std::process::Command;

use super::*;

/// Runs `cargo <args>` in `dir`, offline, and returns its stdout and stderr when it succeeds.
fn cargo(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("cargo").args(args).current_dir(dir).env("CARGO_NET_OFFLINE", "true").output().expect("cargo runs");
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "cargo {args:?} failed:\n{text}");
    text
}

#[test]
fn the_project_is_in_the_bundle() {
    let bundle = Bundle::embedded();
    for path in ["project/Cargo.toml", "project/crates/tally/src/amount.rs", "project/rhipper.toml"] {
        assert!(bundle.get(path).is_some(), "{path} is embedded");
    }
}

/// The specification's text and its twelve sections, each as its heading line and body.
fn specification() -> (String, Vec<String>) {
    let bytes = Bundle::embedded().get("project/docs/format.md").expect("the specification is embedded").to_vec();
    let text = String::from_utf8(bytes).expect("the specification is UTF-8");
    let sections = text.split("\n## ").skip(1).map(str::to_owned).collect();
    (text, sections)
}

#[test]
fn specification_is_between_sixteen_and_twenty_four_kilobytes() {
    let (text, _) = specification();
    assert!((16 * 1024..=24 * 1024).contains(&text.len()), "{} bytes", text.len());
}

#[test]
fn specification_has_twelve_numbered_sections_in_order() {
    let (_, sections) = specification();
    let titles = [
        "Purpose",
        "Lines",
        "Categories",
        "Amounts",
        "Whitespace",
        "Comments and blank lines",
        "Errors",
        "The report",
        "Totals",
        "Exit codes",
        "Worked examples",
        "History of the format",
    ];
    assert_eq!(sections.len(), 12);
    for (index, (section, title)) in sections.iter().zip(titles).enumerate() {
        let heading = format!("{}. {title}\n", index + 1);
        assert!(section.starts_with(&heading), "section {} is `{title}`", index + 1);
    }
}

#[test]
fn specification_gives_section_six_word_for_word() {
    let (_, sections) = specification();
    let body = "A line whose first character that is not whitespace is `#` is a comment. A line that is empty or only whitespace is blank. A reader skips both. A `#` anywhere else is not a comment: `food 1#2` is an entry with a malformed amount. A skipped line still has a number: the line number in an error counts every line of the file, skipped or not.";
    assert!(sections[5].contains(body));
}

#[test]
fn specification_names_the_five_error_sentences_in_section_seven() {
    let (_, sections) = specification();
    for sentence in
        ["`empty line`", "`no amount after the category`", "`not an amount: `", "`more than two words`", "`not a category name: `"]
    {
        assert!(sections[6].contains(sentence), "section 7 has {sentence}");
    }
}

#[test]
#[ignore = "runs cargo on a copy of the project"]
fn project_is_clean() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    write(&Bundle::embedded(), dir.path()).expect("the project is written");
    let root = dir.path();

    cargo(root, &["test", "--workspace", "--offline", "-q"]);
    let clippy = cargo(root, &["clippy", "--workspace", "--all-targets", "--offline", "--", "-D", "warnings"]);
    assert!(!clippy.contains("warning"), "clippy prints none:\n{clippy}");
    cargo(root, &["fmt", "--all", "--check"]);

    // The latent fault the cards repair: minus 150 cents prints with two minus signs.
    std::fs::write(root.join("fault.ledger"), "food -2.50\n").expect("a ledger");
    let report = cargo(root, &["run", "-q", "--offline", "-p", "tally-cli", "--", "fault.ledger"]);
    assert!(report.contains("total -1.-50"), "the fault survives:\n{report}");
}
