use super::*;

/// A file of its own under the temp dir, named for the test, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str, text: &str) -> Self {
        let path = std::env::temp_dir().join(format!("rc-gui-code-{}-{name}", std::process::id()));
        std::fs::write(&path, text).expect("a scratch file");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn a_pane_reads_the_file_and_widens_its_tabs() {
    let file = Scratch::new("read.rs", "fn main() {\n\tlet x = 1;\n}\n");
    let pane = CodePane::open(&file.0);
    assert_eq!(pane.lines, ["fn main() {", "    let x = 1;", "}"]);
    assert!(pane.fault.is_none());
    assert_eq!(pane.path(), file.0.as_path());
}

#[test]
fn a_pane_follows_an_edit_on_disk() {
    let file = Scratch::new("follow.toml", "a = 1\n");
    let mut pane = CodePane::open(&file.0);
    assert_eq!(pane.lines, ["a = 1"]);
    std::fs::write(&file.0, "a = 1\nb = 2\n").expect("the edit");
    pane.refresh();
    assert_eq!(pane.lines, ["a = 1", "b = 2"]);
    // the same file again is not read twice: the stamp has not moved
    let read_at = pane.read_at;
    pane.refresh();
    assert_eq!(pane.read_at, read_at);
}

#[test]
fn a_missing_file_is_a_fault_not_stale_text() {
    let file = Scratch::new("gone.txt", "here\n");
    let mut pane = CodePane::open(&file.0);
    assert_eq!(pane.lines, ["here"]);
    std::fs::remove_file(&file.0).expect("removed");
    pane.refresh();
    assert!(pane.lines.is_empty());
    assert!(pane.fault.as_deref().is_some_and(|f| f.starts_with("cannot read")));
}

#[test]
fn a_row_carries_its_number_and_the_colored_pieces() {
    let file = Scratch::new("row.rs", "let s = \"hi\"; // note\n");
    let pane = CodePane::open(&file.0);
    let job = pane.row(0, 3, &FontId::monospace(13.0));
    assert_eq!(job.text, "  1  let s = \"hi\"; // note");
    assert_eq!(job.sections[0].format.color, cards::alpha(theme::dim(), 170));
    assert!(job.sections.iter().any(|s| s.format.color == theme::warn_text()));
    assert_eq!(job.sections.last().map(|s| s.format.color), Some(theme::dim()));
}

#[test]
fn a_pane_opened_at_a_line_jumps_there_and_lights_it() {
    let file = Scratch::new("at.rs", "fn a() {}\nfn b() {}\nfn c() {}\n");
    let pane = CodePane::open_at(&file.0, 2);
    assert_eq!(pane.jump, Some(2));
    assert!(pane.lit_fade(1) > 0.9);
    assert_eq!(pane.lit_fade(0), 0.0);
    let font = FontId::monospace(13.0);
    assert_ne!(pane.row(1, 1, &font).sections[0].format.background, Color32::TRANSPARENT);
    assert_eq!(pane.row(0, 1, &font).sections[0].format.background, Color32::TRANSPARENT);
    assert_eq!(row_offset(1, 15.0), 0.0);
    assert_eq!(row_offset(11, 15.0), 150.0);
}
