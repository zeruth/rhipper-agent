//! The tests of the key handling: the environment path, and the mask of a short key.

use super::*;

#[test]
fn the_environment_overrides_the_store() {
    // SAFETY: the test serializes on the process env; no parallel test reads this key
    unsafe { std::env::set_var("OPENROUTER_API_KEY", "sk-or-from-env") };
    assert_eq!(api_key().unwrap(), "sk-or-from-env");
    unsafe { std::env::remove_var("OPENROUTER_API_KEY") };
}

#[test]
fn a_mask_holds_the_ends() {
    assert_eq!(masked_of("sk-or-v1-abcdefghij"), "sk-… hij".replace(' ', ""));
    assert_eq!(masked_of("short"), "<short key set>");
}

fn masked_of(key: &str) -> String {
    if key.len() > 8 { format!("{}…{}", &key[..3], &key[key.len() - 3..]) } else { "<short key set>".into() }
}

#[test]
fn an_empty_key_is_a_refusal() {
    // SAFETY: the test serializes on the process env; no parallel test reads this key
    unsafe { std::env::set_var("OPENROUTER_API_KEY", "  ") };
    let error = api_key().unwrap_err();
    assert!(format!("{error:#}").contains("no OpenRouter key"), "{error:#}");
    unsafe { std::env::remove_var("OPENROUTER_API_KEY") };
}
