//! The fonts of the workspace: the system's Segoe UI for the text, Cascadia Mono or Consolas
//! for the code and the console, and Segoe UI Symbol behind both for the marks and the arrows
//! egui's own fonts do not carry. A font that is not on the machine is skipped; egui's defaults
//! stay as the last resort.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

/// Installs the fonts into `ctx`.
pub(crate) fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut load = |name: &str, file: &str| -> bool {
        let path = std::path::Path::new(r"C:\Windows\Fonts").join(file);
        match std::fs::read(&path) {
            Ok(bytes) => {
                fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let text = load("segoe", "segoeui.ttf");
    let mono = load("cascadia", "CascadiaMono.ttf") || load("cascadia", "consola.ttf");
    let symbols = load("segoe-symbol", "seguisym.ttf");
    let emoji = load("segoe-emoji", "seguiemj.ttf");
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        if text {
            family.insert(0, "segoe".to_owned());
        }
        if symbols {
            family.push("segoe-symbol".to_owned());
        }
        if emoji {
            family.push("segoe-emoji".to_owned());
        }
    }
    if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
        if mono {
            family.insert(0, "cascadia".to_owned());
        }
        if symbols {
            family.push("segoe-symbol".to_owned());
        }
        if emoji {
            family.push("segoe-emoji".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}
