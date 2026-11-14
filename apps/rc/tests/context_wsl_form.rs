//! The run context names the one-program WSL form when the card uses WSL, and says nothing of it
//! when the card does not.

use ra_console::hub::manifest::{Manifest, Tool};

fn manifest() -> Manifest {
    Manifest {
        day: "2026-11-14".to_owned(),
        tools: vec![Tool { name: "wsl".to_owned(), path: Some("C:\\Windows\\wsl.exe".into()), ..Tool::default() }],
        sdks: Vec::new(),
        wsl: vec![("Ubuntu".to_owned(), vec!["git".to_owned()])],
    }
}

#[test]
fn a_wsl_card_gets_the_one_program_form() {
    let block = manifest().block_for("build it with wsl");
    assert!(block.contains("`wsl -d <distro> -- <tool> <args>`"), "{block}");
    assert!(block.contains("one program per command"), "{block}");
    assert!(block.contains("bash -lc"), "{block}");
    assert!(block.contains("refused"), "{block}");
}

#[test]
fn a_card_without_wsl_gets_no_wsl_form() {
    let mut manifest = manifest();
    manifest.tools.push(Tool { name: "cargo".to_owned(), path: Some("C:\\cargo.exe".into()), ..Tool::default() });
    let block = manifest.block_for("run cargo");
    assert!(!block.contains("wsl -d"), "{block}");
}
