//! The evidence line for a gotcha that fired names up to three distinct commands it refused,
//! each with its count, and the project it fired in.

use ra_console::plan::evidence::gotchas;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

#[test]
fn the_line_names_the_refused_commands_with_counts_and_project() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("var")).unwrap();
    let at = now();
    let mut log = String::new();
    for (command, times) in [("sed -i a b", 3), ("awk x", 2), ("perl y", 1), ("python -", 1)] {
        for _ in 0..times {
            log.push_str(&format!("{at}\t7\tsed-and-python\tBash\t{command}\n"));
        }
    }
    std::fs::write(dir.path().join("var/gotchas.log"), log).unwrap();
    let lines = gotchas(dir.path());
    let text = &lines.first().expect("a line").text;
    let project = ra_console::hub::project_name(dir.path());
    assert!(text.contains("`sed -i a b` ×3, `awk x` ×2, `perl y` ×1"), "{text}");
    assert!(!text.contains("python -"), "{text}");
    assert!(text.ends_with(&format!("in {project}")), "{text}");
}
