use ra_console::agent::BUILD_RULES;

#[test]
fn the_rules_say_rc_wait_takes_only_rc_ids() {
    assert!(BUILD_RULES.contains("`rc wait` takes only rc's own ids"), "{BUILD_RULES}");
}

#[test]
fn the_rules_say_a_shell_tasks_output_file_is_empty_until_the_notification() {
    assert!(BUILD_RULES.contains("empty or partial until the notification arrives"), "{BUILD_RULES}");
    assert!(BUILD_RULES.contains("always empty under `rc hold --shell bash`"), "{BUILD_RULES}");
}

#[test]
fn the_rules_name_the_notification_as_the_only_wait() {
    assert!(BUILD_RULES.contains("make no call between starting the run and its notification"), "{BUILD_RULES}");
}
