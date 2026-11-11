use super::*;
use crate::logs::{RUN_END, RUN_START};

#[test]
fn the_feed_keeps_its_events_in_order_and_one_line_each() {
    let dir = tempfile::tempdir().unwrap();
    assert!(recent(dir.path(), 5).is_empty());
    event(dir.path(), "start #7 A card · claude-fable-5-1");
    event(dir.path(), "end #7 ✓ implemented\nwith a second line");
    event(dir.path(), "note the tooltip is slow");
    assert_eq!(recent(dir.path(), 2), ["end #7 ✓ implemented with a second line", "note the tooltip is slow"]);
    assert_eq!(recent(dir.path(), 10).len(), 3);
    // the digest names the last end, not the note after it
    assert_eq!(last_end(dir.path()).as_deref(), Some("#7 ✓ implemented with a second line"));
    assert!(last_end(&dir.path().join("none")).is_none());
}

#[test]
fn the_live_start_is_the_open_one_with_its_pid_and_none_after_an_end() {
    let dir = tempfile::tempdir().unwrap();
    assert!(live_start(dir.path()).is_none());
    event(dir.path(), "start #7 A card · claude-fable-5-1 · check: none · pid 4242");
    assert_eq!(live_start(dir.path()), Some(("#7 A card · claude-fable-5-1 · check: none · pid 4242".to_owned(), Some(4242))));
    event(dir.path(), "note still going");
    assert!(live_start(dir.path()).is_some());
    event(dir.path(), "end ✓ #7 A card implemented");
    assert!(live_start(dir.path()).is_none());
    // a start from before the marker carried a pid is open, with none to judge it by
    event(dir.path(), "start #8 B card · claude-fable-5-1");
    assert_eq!(live_start(dir.path()), Some(("#8 B card · claude-fable-5-1".to_owned(), None)));
    event(dir.path(), "run orphaned #8");
    assert!(live_start(dir.path()).is_none());
}

fn write(dir: &Path, text: &str) {
    std::fs::create_dir_all(dir.join("var")).unwrap();
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(PAIR_FILE)).unwrap();
    write!(file, "{text}").unwrap();
}

#[test]
fn since_replays_only_the_lines_after_the_time() {
    let dir = tempfile::tempdir().unwrap();
    assert!(since(dir.path(), 0).is_empty());
    write(dir.path(), "100\tstart #1 a\n200\tend #1 ✓\n300\tnote late\n");
    assert_eq!(since(dir.path(), 200), ["300\tnote late"]);
    assert_eq!(since(dir.path(), 99).len(), 3);
    assert!(since(dir.path(), 300).is_empty());
}

#[test]
fn a_follower_catches_up_then_sees_new_lines_and_survives_a_rotation() {
    let dir = tempfile::tempdir().unwrap();
    let mut follower = Follower::new(dir.path(), 100);
    assert!(follower.poll().is_empty());
    write(dir.path(), "100\told\n150\tcaught up\n");
    assert_eq!(follower.poll(), ["150\tcaught up"]);
    assert!(follower.poll().is_empty());
    write(dir.path(), "200\tnew, in two ");
    assert!(follower.poll().is_empty());
    write(dir.path(), "writes\n");
    assert_eq!(follower.poll(), ["200\tnew, in two writes"]);
    std::fs::rename(dir.path().join(PAIR_FILE), dir.path().join("var/pair.1.log")).unwrap();
    assert!(follower.poll().is_empty());
    write(dir.path(), "300\tafter the rotation\n");
    assert_eq!(follower.poll(), ["300\tafter the rotation"]);
}

#[test]
fn the_window_says_when_it_comes_up_and_goes_down() {
    let dir = tempfile::tempdir().unwrap();
    window_up_with(dir.path(), "0.9.1+abc", 3, |_| true);
    window_down(dir.path(), "restart");
    let pid = std::process::id();
    assert_eq!(recent(dir.path(), 10), [format!("window up 0.9.1+abc pid {pid} tabs 3"), "window down restart".to_owned()]);
}

#[test]
fn a_run_whose_process_is_gone_is_orphaned_once_and_a_finished_one_never() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("var")).unwrap();
    let activity = dir.path().join(crate::ui::ACTIVITY_FILE);
    std::fs::write(
        &activity,
        format!(
            "{RUN_START}#2 Finished · opus · pid 111\nread a.rs\n{RUN_END}✓ $0.01\n\
             {RUN_START}#3 Killed with the window · opus · pid 222\nedit b.rs\n\
             {RUN_START}#4 Still going · opus · pid 333\n\
             {RUN_START}#5 Of an older build, no pid · opus\n"
        ),
    )
    .unwrap();
    // 111 finished, 222 is gone, 333 lives, #5 cannot be judged
    window_up_with(dir.path(), "b", 1, |pid| pid == 333);
    let lines = recent(dir.path(), 10);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[1], "run orphaned #3");
    // the orphan got its end in the activity log, so the next start does not report it again
    let text = std::fs::read_to_string(&activity).unwrap();
    assert!(text.ends_with(&format!("{RUN_END}✗ orphaned #3: process 222 is gone\n")), "{text}");
    window_up_with(dir.path(), "b", 1, |pid| pid == 333);
    assert_eq!(recent(dir.path(), 10).len(), 3);
}

#[test]
fn an_end_closes_the_newest_open_run() {
    let text = format!("{RUN_START}#1 a · m · pid 10\n{RUN_START}#2 b · m · pid 20\n{RUN_END}✓\n");
    assert_eq!(orphans(&text, |_| false), [(1, 10)]);
    assert!(orphans(&text, |_| true).is_empty());
    assert!(orphans("", |_| false).is_empty());
}
