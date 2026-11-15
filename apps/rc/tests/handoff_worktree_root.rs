//! A worker in a card's worktree is told the worktree's absolute path is the root the Read tool
//! takes, and that paths under the main checkout are refused.

use std::path::Path;

use ra_providers::claude_code::worktree_note;

#[test]
fn the_prompt_names_the_worktree_root_as_the_only_read_path() {
    let root = Path::new("C:/code/project");
    let worktree = root.join("var").join("worktrees").join("670");
    let absolute = std::path::absolute(&worktree).expect("an absolute path");
    let note = worktree_note(&worktree).expect("a worktree gets the line");
    assert!(note.contains(&absolute.display().to_string()), "{note}");
    assert!(note.contains("only root the Read tool takes"), "{note}");
    assert!(note.contains("main checkout are refused"), "{note}");
    assert!(worktree_note(root).is_none(), "the main checkout gets no line");

    // a relative worktree comes out absolute
    let relative = Path::new("var").join("worktrees").join("670");
    let note = worktree_note(&relative).expect("a relative worktree gets the line");
    let absolute = std::path::absolute(&relative).expect("an absolute path");
    assert!(note.contains(&absolute.display().to_string()), "{note}");

    // only the card's own segment counts: a checkout under some `var/worktrees` is no worktree
    let nested = root.join("var").join("worktrees").join("670").join("crates");
    assert!(worktree_note(&nested).is_none(), "a directory inside a worktree gets no line");
    let named = root.join("var").join("worktrees").join("project");
    assert!(worktree_note(&named).is_none(), "a checkout named like a project gets no line");
}
