use super::*;

#[test]
fn card_check_timeout_replaces_the_default() {
    let acceptance = Some("`cargo test`\ntimeout: 3600");
    assert_eq!(card_check_timeout(&[None, acceptance]), Some(Duration::from_secs(3600)));
    assert_eq!(card_check_timeout(&[Some("no line here")]), None);
}

#[test]
fn card_check_timeout_over_the_ceiling_is_refused() {
    assert!(refuse_timeout("timeout: 7200").is_ok());
    assert!(refuse_timeout("nothing").is_ok());
    assert!(refuse_timeout("timeout: 7201").is_err());
    assert!(refuse_timeout("timeout: soon").is_err());
    assert_eq!(card_check_timeout(&[Some("timeout: 7201")]), None);
}
