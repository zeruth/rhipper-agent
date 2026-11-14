//! A `saves:` claim is scored in the unit it names: a refused command, a rerun or an `rc wait`
//! call is counted in the run; only a claim in turns or tokens is compared to the median.

use ra_console::retro::{Event, count_of, event_of};

#[test]
fn a_claim_names_the_event_it_counts_or_none() {
    assert_eq!(event_of("one refused command fewer per run"), Some(Event::Refusal));
    assert_eq!(event_of("no rerun after a review"), Some(Event::Rerun));
    assert_eq!(event_of("one rc wait call instead of three"), Some(Event::Wait));
    assert_eq!(event_of("two turns fewer"), None);
    assert_eq!(event_of("5k tokens a turn less"), None);
}

#[test]
fn refusals_and_waits_are_counted_from_the_journal_of_the_card() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("var")).unwrap();
    let log = "1\t7\tshell-outside-allowlist\tCommand\tsed -n 1p x\n\
               2\t7\trefused-sed\tCommand\tsed -i x\n\
               3\t7\trefused-rc\tCommand\trc wait 12 --timeout 9\n\
               4\t8\tshell-outside-allowlist\tCommand\tawk x\n\
               5\t7\tmojibake\tResult\tx\n";
    std::fs::write(dir.path().join("var/gotchas.log"), log).unwrap();
    assert_eq!(count_of(Event::Refusal, dir.path(), 7, 1), 2);
    assert_eq!(count_of(Event::Wait, dir.path(), 7, 1), 1);
    assert_eq!(count_of(Event::Refusal, dir.path(), 9, 1), 0);
}

#[test]
fn a_rerun_is_a_run_after_the_first() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(count_of(Event::Rerun, dir.path(), 7, 0), 0);
    assert_eq!(count_of(Event::Rerun, dir.path(), 7, 1), 0);
    assert_eq!(count_of(Event::Rerun, dir.path(), 7, 3), 2);
}
