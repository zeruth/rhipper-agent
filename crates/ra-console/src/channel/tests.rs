//! The tests of the channels: who walks with the candidate, and what the feed says.

use std::path::PathBuf;

use super::*;
use crate::walk;

fn standing(candidate: bool) -> Standing {
    Standing { stable: PathBuf::from("stable/rc"), candidate: candidate.then(|| PathBuf::from("cand/rc")) }
}

#[test]
fn canary_is_the_agent_and_every_other_project_stays_stable() {
    let agent = PathBuf::from("agent");
    let other = PathBuf::from("runelite");
    let standing = standing(true);
    let moved = standing.at_boundary(&[agent.clone(), other.clone()], Some(&agent));
    assert_eq!(moved, vec![(agent.clone(), Build::Candidate), (other.clone(), Build::Stable)]);
    assert_eq!(standing.binary(Build::Candidate), Path::new("cand/rc"));
    assert_eq!(standing.binary(Build::Stable), Path::new("stable/rc"));

    let (build, line) = walk::begin(&standing, &other, Some(&agent));
    assert_eq!(build, Build::Stable);
    assert!(line.unwrap().contains("stable"));
    assert!(walk::begin(&standing, &agent, Some(&agent)).1.unwrap().contains("candidate"));
}

#[test]
fn without_a_candidate_everyone_is_stable_and_the_feed_is_quiet() {
    let agent = PathBuf::from("agent");
    let standing = standing(false);
    assert_eq!(standing.build_for(&agent, Some(&agent)), Build::Stable);
    assert_eq!(walk::begin(&standing, &agent, Some(&agent)).1, None);
}

#[test]
fn the_standing_builds_survive_a_save() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(Standing::load(dir.path()), None);
    let standing = standing(true);
    standing.save(dir.path()).unwrap();
    assert_eq!(Standing::load(dir.path()), Some(standing));
}
