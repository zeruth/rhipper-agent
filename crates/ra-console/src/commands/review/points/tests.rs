use super::*;

fn file(lines: &[&str]) -> String {
    lines.join("\n")
}

#[test]
fn relocate_points_follows_ten_lines_inserted_above() {
    let before = file(&["fn a() {}", "let x = 1;", "fn b() {}"]);
    let point = quote("src/a.rs:2: warning: unused", |_| Some(before.clone()));
    assert_eq!(point, "src/a.rs:2: warning: unused › let x = 1;");
    let mut after: Vec<String> = (0..10).map(|i| format!("// new {i}")).collect();
    after.extend(before.lines().map(str::to_owned));
    let body = after.join("\n");
    let moved = relocate(&point, |_| Some(body.clone()));
    assert_eq!(moved, "src/a.rs:12: warning: unused › let x = 1;");
}

#[test]
fn relocate_points_marks_a_point_whose_text_is_gone() {
    let point = "src/a.rs:2: warning: unused › let x = 1;";
    let gone = relocate(point, |_| Some("fn a() {}".to_owned()));
    assert_eq!(gone, "src/a.rs:2: warning: unused › let x = 1; resolved?");
    let missing = relocate(point, |_| None);
    assert!(missing.ends_with("resolved?"));
}

#[test]
fn relocate_points_picks_the_nearest_of_equal_lines() {
    let body = file(&["x;", "a", "x;", "b", "c", "d", "x;"]);
    let moved = relocate("f.rs:4: m › x;", |_| Some(body.clone()));
    assert_eq!(moved, "f.rs:3: m › x;");
}

#[test]
fn relocate_points_leaves_unquoted_points_alone() {
    assert_eq!(relocate("the tests are thin", |_| None), "the tests are thin");
    assert_eq!(quote("no location here", |_| Some(String::new())), "no location here");
}
