use ra_console::retro;

#[test]
fn a_lookback_naming_no_cost_is_dropped_before_it_is_logged() {
    let report =
        "## Faster next time\n- the code was tidy\n- [machine] the window holds rc.exe\n- a files line would have saved a search\n";
    let notes = retro::kept(retro::parse(report));
    let texts: Vec<&str> = notes.iter().map(|n| n.text.as_str()).collect();
    assert_eq!(texts, ["the window holds rc.exe", "a files line would have saved a search"]);
    assert!(retro::kept(retro::parse("## Faster next time\n- nothing much\n")).is_empty());
}
