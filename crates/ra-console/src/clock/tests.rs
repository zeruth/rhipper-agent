use std::collections::BTreeMap;
use std::path::Path;

use super::*;

#[test]
fn pinned_clock_stamps_a_feed_line_with_the_pinned_second() {
    let dir = tempfile::tempdir().unwrap();
    {
        let _pin = pin(1_700_000_000);
        assert_eq!(unix_now(), 1_700_000_000);
        crate::pair::event(dir.path(), "run #1 started");
    }
    let feed = std::fs::read_to_string(dir.path().join(crate::pair::PAIR_FILE)).unwrap();
    assert_eq!(feed, "1700000000\trun #1 started\n");
    assert!(unix_now() > 1_700_000_000, "dropping the pin gives the wall clock back");
}

/// Each file's count of wall-clock calls — sleeps and reads of `Instant` and `SystemTime` —
/// under `src`, paths relative to it. A count may fall (lower it here); a rise fails the
/// ratchet: a new stamp goes through [`unix_now`] or [`now`], which a test can pin.
const WALL_CLOCK: &[(&str, usize)] = &[
    ("agent.rs", 20),
    ("commands.rs", 1),
    ("commands/tests.rs", 6),
    ("hub.rs", 6),
    ("hub/manifest.rs", 2),
    ("install.rs", 1),
    ("instance.rs", 3),
    ("logs.rs", 3),
    ("lookback.rs", 1),
    ("pair.rs", 1),
    ("retro.rs", 2),
    ("ui.rs", 4),
    ("ui/tests.rs", 2),
];

/// The calls the ratchet counts, split so this file does not count itself.
const NEEDLES: [[&str; 2]; 3] = [["SystemTime::", "now("], ["Instant::", "now("], ["sleep", "("]];

fn wall_clock_counts(dir: &Path, src: &Path, counts: &mut BTreeMap<String, usize>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            wall_clock_counts(&path, src, counts);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            let count: usize = NEEDLES.iter().map(|[a, b]| text.matches(&format!("{a}{b}")).count()).sum();
            if count > 0 {
                let name = path.strip_prefix(src).unwrap().to_string_lossy().replace('\\', "/");
                counts.insert(name, count);
            }
        }
    }
}

#[test]
fn wall_clock_ratchet_holds_each_file_to_its_count() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut counts = BTreeMap::new();
    wall_clock_counts(&src, &src, &mut counts);
    let table: BTreeMap<&str, usize> = WALL_CLOCK.iter().copied().collect();
    let risen: Vec<String> = counts
        .iter()
        .filter_map(|(file, &n)| {
            let allowed = table.get(file.as_str()).copied().unwrap_or(0);
            (n > allowed).then(|| format!("{file}: {n} wall-clock calls, {allowed} allowed"))
        })
        .collect();
    assert!(risen.is_empty(), "read the time through crate::clock instead:\n{}", risen.join("\n"));
}
