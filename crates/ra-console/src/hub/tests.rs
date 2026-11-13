use super::*;

fn digest_of(name: &str, pid: u32, when: u64) -> Digest {
    Digest {
        name: name.to_owned(),
        path: format!("C:/code/{name}"),
        pid,
        when,
        live: Some("#12 The digest · pid 4242 · started 10:04".to_owned()),
        last: Some("✓ #11 Shared notes implemented".to_owned()),
        ready: 3,
        changed: vec!["crates/ra-console/src/hub.rs".to_owned(), "crates/ra-console/src/agent.rs".to_owned()],
        leftovers: vec![],
        landed_today: (0, None),
        waiting: 0,
    }
}

#[test]
fn a_walk_moves_to_a_newer_install_or_a_rebuild_of_its_own_and_never_to_an_older_one() {
    // newer by its number, whatever the walk saw at its start
    assert_eq!(moved_to("0.2.10", Some("0.2.11 5"), Some("0.2.11 5")), Some("0.2.11".to_owned()));
    // the same number, installed again since the start: a rebuild
    assert_eq!(moved_to("0.2.10+", Some("0.2.10+ 5"), Some("0.2.10+ 9")), Some("0.2.10+".to_owned()));
    // unchanged since the start, older, or never installed
    assert_eq!(moved_to("0.2.10+", Some("0.2.10+ 5"), Some("0.2.10+ 5")), None);
    assert_eq!(moved_to("0.2.10", None, Some("0.2.9 9")), None);
    assert_eq!(moved_to("0.2.10", None, None), None);
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(installed(dir.path()), None);
    std::fs::write(dir.path().join(INSTALLED_FILE), "0.2.11 5\n").unwrap();
    assert_eq!(installed(dir.path()).as_deref(), Some("0.2.11 5"));
}

#[test]
fn a_digest_written_from_one_project_is_read_from_another_as_one_line_and_never_its_own() {
    let hub = tempfile::tempdir().unwrap();
    let alpha = tempfile::tempdir().unwrap();
    let beta = tempfile::tempdir().unwrap();
    let dir = hub.path().join("digest");
    let now = 1_700_000_000;
    assert!(digest_lines_with(&dir, beta.path(), |_| true, now).is_empty());
    let mut digest = digest_of(&project_name(alpha.path()), 4242, now);
    // a path that is on disk, short texts and one file, so the whole of it fits the line
    digest.path = alpha.path().display().to_string();
    digest.live = Some("#12 The digest".to_owned());
    digest.last = Some("✓ #11 Shared notes".to_owned());
    digest.changed = vec!["hub.rs".to_owned()];
    write_digest_in(&dir, &digest).unwrap();
    // the other project reads it whole, in one line; the writer reads nothing back
    let lines = digest_lines_with(&dir, beta.path(), |_| true, now);
    assert_eq!(lines.len(), 1);
    let line = &lines[0];
    assert!(line.starts_with(&format!("{} ({})", digest.name, digest.path)), "{line}");
    assert!(line.contains(" · live: #12 The digest · "), "{line}");
    assert!(line.contains(" · last: ✓ #11 Shared notes · "), "{line}");
    assert!(line.contains("3 ready"), "{line}");
    assert!(line.ends_with("3 ready · changed: hub.rs"), "{line}");
    assert!(!line.contains('\n'));
    assert!(digest_lines_with(&dir, alpha.path(), |_| true, now).is_empty());
    // the writer reads its own back whole, so a walk's start keeps the last run's end and files
    assert_eq!(own_digest_in(&dir, alpha.path()).unwrap(), digest);
    assert!(own_digest_in(&dir, beta.path()).is_none());
}

#[test]
fn a_digest_line_is_clipped_at_the_width() {
    let mut digest = digest_of("gamma", 1, 1);
    digest.changed = (0..40).map(|i| format!("crates/ra-console/src/file_{i}.rs")).collect();
    let line = digest_line(&digest);
    assert_eq!(line.chars().count(), DIGEST_WIDTH, "{line}");
    assert!(line.ends_with('…'));
    digest.changed.clear();
    digest.live = None;
    assert_eq!(digest_line(&digest), "gamma (C:/code/gamma) · last: ✓ #11 Shared notes implemented · 3 ready");
}

#[test]
fn a_digest_keeps_the_leftovers_of_the_cards_under_revision_and_a_clean_tree_adds_none() {
    let hub = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let dir = hub.path().join("digest");
    let mut digest = digest_of(&project_name(project.path()), 1, 1);
    digest.path = project.path().display().to_string();
    digest.live = None;
    digest.changed.clear();
    write_digest_in(&dir, &digest).unwrap();
    let text = std::fs::read_to_string(dir.join(format!("{}.txt", digest.name))).unwrap();
    assert!(!text.contains("leftovers"), "{text}");
    digest.leftovers = vec!["src/a.rs (#7)".to_owned(), "src/b.rs (#7 #9)".to_owned()];
    write_digest_in(&dir, &digest).unwrap();
    assert_eq!(own_digest_in(&dir, project.path()).unwrap(), digest);
    assert!(digest_line(&digest).ends_with("3 ready · leftovers: src/a.rs (#7), src/b.rs (#7 #9)"), "{}", digest_line(&digest));
}

#[test]
fn a_stale_digest_is_dropped_but_a_fresh_or_live_one_is_kept() {
    let hub = tempfile::tempdir().unwrap();
    let me = tempfile::tempdir().unwrap();
    let dir = hub.path().join("digest");
    let now = 1_700_000_000;
    let day = 24 * 60 * 60;
    let on_disk = |name: &str, pid: u32, when: u64| {
        let mut digest = digest_of(name, pid, when);
        digest.path = me.path().display().to_string();
        digest
    };
    write_digest_in(&dir, &on_disk("old-dead", 1, now - day - 1)).unwrap();
    write_digest_in(&dir, &on_disk("old-alive", 2, now - 3 * day)).unwrap();
    write_digest_in(&dir, &on_disk("fresh-dead", 3, now - day + 60)).unwrap();
    // a project whose directory is gone, a test's temp dir say, is dropped however fresh or live
    write_digest_in(&dir, &digest_of("gone", 2, now)).unwrap();
    let alive = |pid: u32| pid == 2;
    let lines = digest_lines_with(&dir, me.path(), alive, now);
    let names: Vec<&str> = lines.iter().map(|l| l.split_once(' ').unwrap().0).collect();
    assert_eq!(names, ["fresh-dead", "old-alive"]);
    // the stale and the gone are gone from the directory too
    assert!(!dir.join("old-dead.txt").exists());
    assert!(!dir.join("gone.txt").exists());
    assert!(dir.join("old-alive.txt").exists());
}

#[test]
fn the_tests_hub_is_a_temp_directory_of_this_process_and_rhipper_hub_names_another() {
    let hub = dir();
    assert!(hub.starts_with(std::env::temp_dir()), "{}", hub.display());
    assert!(hub.ends_with(format!("rhipper-test-hub-{}", std::process::id())), "{}", hub.display());
    // what the tests write lands under it: digests, locks, the shared notes
    assert!(digest_dir().starts_with(&hub));
    assert!(locks_dir().starts_with(&hub));
    assert!(crate::retro::shared_file().starts_with(&hub));
    let named = tempfile::tempdir().unwrap();
    assert_eq!(dir_from(Some(named.path().as_os_str())), named.path());
    // an empty variable is no hub; the machine's is used, and it is never the tests' one
    let machine = dir_from(Some(std::ffi::OsStr::new("")));
    assert_eq!(machine, dir_from(None));
    assert!(machine.ends_with("agent"), "{}", machine.display());
    assert_ne!(machine, hub);
}

#[test]
fn a_status_shows_the_live_run_an_orphan_by_its_dead_pid_and_idle_without_a_feed() {
    let root = tempfile::tempdir().unwrap();
    let today = "2026-11-11";
    let idle = status_with(root.path(), |_| true, &[], today);
    assert_eq!(idle.run, Run::Idle);
    assert_eq!(idle.name, project_name(root.path()));
    assert!(idle.last.is_none());
    assert!(idle.ready.is_none(), "no roadmap, no count");
    assert_eq!(idle.calls_today, 0);
    assert!(idle.line().contains(" idle "), "{}", idle.line());
    crate::pair::event(root.path(), "start #12 The status · claude-fable-5-1 · check: none · pid 4242");
    let live = status_with(root.path(), |pid| pid == 4242, &[], today);
    assert_eq!(live.run, Run::Live("#12 The status · claude-fable-5-1 · check: none · pid 4242".to_owned()));
    let dead = status_with(root.path(), |_| false, &[], today);
    assert_eq!(dead.run, Run::Orphaned("#12 The status · claude-fable-5-1 · check: none · pid 4242".to_owned()));
    assert!(dead.line().contains("orphaned: #12 The status"), "{}", dead.line());
    crate::pair::event(root.path(), "end ✓ #12 The status implemented");
    let ended = status_with(root.path(), |_| false, &[], today);
    assert_eq!(ended.run, Run::Idle);
    assert_eq!(ended.last.as_deref(), Some("✓ #12 The status implemented"));
    assert!(ended.line().ends_with("✓ #12 The status implemented"), "{}", ended.line());
}

#[test]
fn a_status_counts_the_spend_of_today_only() {
    let spend = |at: &str, cost: Option<f64>| ra_usage::Spend { at: at.to_owned(), tokens: 10, cost_usd: cost };
    let spends = [
        spend("2026-11-10 23:59:59", Some(0.5)),
        spend("2026-11-11 00:00:01", Some(0.25)),
        spend("2026-11-11 10:04:00", None),
        spend("2026-11-11 11:00:00", Some(0.25)),
    ];
    assert_eq!(spend_on(&spends, "2026-11-11"), (3, Some(0.5)));
    assert_eq!(spend_on(&spends, "2026-11-10"), (1, Some(0.5)));
    assert_eq!(spend_on(&spends, "2026-11-12"), (0, None));
    assert_eq!(spend_on(&spends[2..3], "2026-11-11"), (1, None));
    let root = tempfile::tempdir().unwrap();
    let status = status_with(root.path(), |_| true, &spends, "2026-11-11");
    assert_eq!((status.calls_today, status.spend_today), (3, Some(0.5)));
    assert!(status.line().contains("3 calls $0.50"), "{}", status.line());
    // the day is the ledger's: UTC
    assert_eq!(day_of(1_700_000_000), "2023-11-14");
    assert_eq!(day_of(0), "1970-01-01");
    assert_eq!(day_of(951_782_400), "2000-02-29");
}

#[test]
fn two_roots_in_one_workspace_guard_the_same_target_and_the_second_waits() {
    let hub = tempfile::tempdir().unwrap();
    let locks = hub.path().join("locks");
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("Cargo.toml"), "[workspace]\nmembers = [\"alpha\", \"beta\"]\n").unwrap();
    let alpha = workspace.path().join("alpha");
    let beta = workspace.path().join("beta");
    for root in [&alpha, &beta] {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    }
    let apart = tempfile::tempdir().unwrap();
    // the two roots build in the workspace's target; the third in its own
    let shared = target_lock_name(&cargo_target_with(&alpha, None));
    assert_eq!(shared, target_lock_name(&cargo_target_with(&beta, None)));
    assert_eq!(cargo_target_with(&alpha, None), std::fs::canonicalize(workspace.path()).unwrap().join("target"));
    let own = target_lock_name(&cargo_target_with(apart.path(), None));
    assert_ne!(shared, own);
    // CARGO_TARGET_DIR wins over the workspace
    assert_eq!(cargo_target_with(&alpha, Some(std::ffi::OsStr::new("build"))), alpha.join("build"));
    let poll = std::time::Duration::from_millis(1);
    // the first check takes the lock without a wait
    let mut waits = 0;
    let first = guard_in(&locks, &shared, poll, |_| {
        waits += 1;
        false
    })
    .unwrap();
    assert!(first.is_some());
    assert_eq!(waits, 0);
    // the second, on the same target, waits on it: three tries, then it gives up here
    let second = guard_in(&locks, &shared, poll, |n| n < 3).unwrap();
    assert!(second.is_none());
    // a different target does not wait
    let mut waits = 0;
    let other = guard_in(&locks, &own, poll, |_| {
        waits += 1;
        false
    })
    .unwrap();
    assert!(other.is_some());
    assert_eq!(waits, 0);
    // the first check's end lets the second through
    drop(first);
    assert!(guard_in(&locks, &shared, poll, |_| false).unwrap().is_some());
}
