//! A timed-out check's note names what each hung test waited on.

use ra_console::{hub, pair, timeout_note, timeout_note_with_waits, wait_line};

#[test]
fn timeout_note_names_waits() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("proj");
    let own = project.join("var").join("worktrees").join("7");
    let other = project.join("var").join("worktrees").join("8");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    // the check of card 7 waited on its target; card 8's check said a wait of its own after it
    pair::event(&project, "end #1 ok");
    pair::event(&project, &wait_line("wait: the check waits on the cargo target D:/t, held by other #9", &hub::builder_name(&own)));
    pair::event(
        &project,
        &wait_line("wait: the check waits for a worker slot, 2 of the machine's 2 are running", &hub::builder_name(&other)),
    );
    let output = "test a_one has been running for over 60 seconds\ntest b_two has been running for over 60 seconds\ntimed out after 120s";
    let note = timeout_note(&own, output);
    assert!(note.contains("a_one wait: the check waits on the cargo target D:/t, held by other #9"), "{note}");
    assert!(note.contains("b_two wait: the check waits on the cargo target D:/t, held by other #9"), "{note}");
    assert!(!note.contains("worker slot"), "card 8's wait is not card 7's: {note}");
    // a check that said no wait of its own, and output that did not time out, come back as they were
    let feed = pair::recent(&project, 50);
    assert_eq!(timeout_note_with_waits(output, &feed, "proj #5"), output);
    assert_eq!(timeout_note(&own, "exit code 1"), "exit code 1");
}
