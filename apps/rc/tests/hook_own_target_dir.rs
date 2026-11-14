//! The hook lets a worker list and run what is under its own card's target dir, and no other
//! card's.

use std::path::Path;

use ra_gotchas::shape::{Shape, Shell, judge_in};

const OWN: &str = "D:/rhipper/target/rhipper-agent/wt-684";

fn judged(command: &str) -> Shape {
    judge_in(command, Shell::Bash, Path::new("C:/work/wt"), &|_| false, Some(Path::new(OWN)))
}

#[test]
fn the_own_target_dir_can_be_listed_and_run() {
    for command in [format!("ls {OWN}/debug"), format!("ls -la {OWN}/debug/deps"), format!("{OWN}/debug/rc.exe --version")] {
        assert_eq!(judged(&command), Shape::Allowed, "{command}");
    }
}

#[test]
fn another_cards_target_dir_stays_refused() {
    for command in [
        "ls D:/rhipper/target/rhipper-agent/wt-685/debug".to_owned(),
        "D:/rhipper/target/rhipper-agent/wt-685/debug/rc.exe --version".to_owned(),
        format!("ls {OWN}/../wt-685/debug"),
    ] {
        assert_ne!(judged(&command), Shape::Allowed, "{command}");
    }
}
