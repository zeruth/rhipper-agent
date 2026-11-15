//! A landing's squash commit is named from the card's own run, never from a lint pass's report.

use ra_console::agent::card_landing_message;
use ra_context::Handoff;

fn handoff(report: &str) -> Handoff {
    Handoff {
        when: 0,
        model: "m".to_owned(),
        outcome: "implemented".to_owned(),
        check: String::new(),
        verdict: None,
        changed: Vec::new(),
        report: report.to_owned(),
        warnings: Vec::new(),
        read: Vec::new(),
        failures: Vec::new(),
    }
}

#[test]
fn a_lint_pass_does_not_name_the_landing() {
    let run = handoff("Done.\n\n## Commit\nRename filed to the card's name\n\nWhy.\n");
    let lint = handoff("lint pass: fresh.\nFixed it.\n\n## Commit\nCombine match arms with identical bodies\n");
    let message = card_landing_message("Card title", "", &[run, lint]);
    assert_eq!(message, "Rename filed to the card's name\n\nWhy.");
}

#[test]
fn a_reruns_rewritten_commit_section_names_the_landing() {
    let first = handoff("Done.\n\n## Commit\nAdd must_use to a function\n");
    let rerun = handoff("Answered the review.\n\n## Commit\nName a landing for the card's own run\n");
    let lint = handoff("lint pass: fresh.\nFixed it.\n\n## Commit\nTidy a lint\n");
    let message = card_landing_message("Card title", "", &[first, rerun, lint]);
    assert_eq!(message, "Name a landing for the card's own run");
}

#[test]
fn notes_a_fix_pass_wrote_do_not_beat_the_runs_report() {
    let run = handoff("Done.\n\n## Commit\nCap the slots\n");
    let message = card_landing_message("Card title", "## Commit\nTidy a lint\n", &[run]);
    assert_eq!(message, "Cap the slots");
}

#[test]
fn a_card_without_a_commit_section_lands_under_its_title() {
    let message = card_landing_message("Card title", "", &[handoff("Did the work.\n\nMore.")]);
    assert_eq!(message, "Card title\n\nDid the work.");
}
