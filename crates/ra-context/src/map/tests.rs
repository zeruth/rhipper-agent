use std::collections::HashMap;

use super::*;

fn files_of(texts: &[(&str, &str)]) -> (impl Fn(&str) -> Option<String> + use<>, Vec<String>) {
    let read: HashMap<String, String> = texts.iter().map(|(f, t)| ((*f).to_owned(), (*t).to_owned())).collect();
    let files = texts.iter().map(|(f, _)| (*f).to_owned()).collect();
    (move |file: &str| read.get(file).cloned(), files)
}

const ABC: &str = "fn first() {}\nfn second() {}\nfn third() {}\n";

#[test]
fn map_ranks_a_named_item_above_an_early_one() {
    let (read, files) = files_of(&[("src/a.rs", ABC)]);
    let ask = Ask { card: "the first fn and `third`", ..Ask::default() };
    let all = map_of(&read, &files, &ask, &ChangedLines::new(), 10_000);
    let third = all.pointers.iter().find(|p| p.item == "fn third()").expect("third is pointed at");
    assert_eq!(third.score, 100);
    assert!(all.pointers.iter().any(|p| p.item == "fn first()" && p.score < 100));

    let map = map_of(&read, &files, &ask, &ChangedLines::new(), tokens(&third.line()));
    let shown: Vec<&str> = map.pointers.iter().map(|p| p.item.as_str()).collect();
    assert_eq!(shown, ["fn third()"]);
    assert_eq!((map.pointers[0].start, map.pointers[0].end), (3, 3));
}

#[test]
fn map_ranks_a_changed_item_and_keeps_it() {
    let (read, files) = files_of(&[("src/a.rs", ABC)]);
    let changed: ChangedLines = HashMap::from([("src/a.rs".to_owned(), vec![(2, 2)])]);
    let map = map_of(&read, &files, &Ask { card: "nothing in here", ..Ask::default() }, &changed, 10_000);
    assert_eq!(map.pointers.len(), 1);
    assert_eq!(map.pointers[0].item, "fn second()");
    assert_eq!(map.pointers[0].score, 80);
    assert!(map.pointers[0].why.contains("changed"));
}

#[test]
fn map_ranks_a_common_word_under_a_rare_one() {
    let text = "fn parse_head() {}\nfn parse_line() {}\nfn parse_tail() {}\nfn ledger_total() {}\n";
    let (read, files) = files_of(&[("src/a.rs", text)]);
    let map = map_of(&read, &files, &Ask { card: "parse the ledger", ..Ask::default() }, &ChangedLines::new(), 10_000);
    let score = |item: &str| map.pointers.iter().find(|p| p.item == item).map_or(0, |p| p.score);
    assert!(score("fn parse_head()") > 0);
    assert!(score("fn parse_head()") < score("fn ledger_total()"), "{map:?}");
    assert_eq!(score("fn ledger_total()"), 40);
}

#[test]
fn map_ranks_within_the_room_across_two_files() {
    let a: String = (0..10).map(|i| format!("fn a_step_{i}() {{}}\n")).collect();
    let b: String = (0..10).map(|i| format!("fn b_step_{i}() {{}}\n")).collect();
    let card: String = (0..10).map(|i| format!("`a_step_{i}` `b_step_{i}` ")).collect();
    let (read, files) = files_of(&[("src/a.rs", &a), ("src/b.rs", &b)]);
    let room = 40;
    let map = map_of(&read, &files, &Ask { card: &card, ..Ask::default() }, &ChangedLines::new(), room);
    assert!(!map.pointers.is_empty());
    assert!(map.pointers.len() < 20);
    assert!(map.pointers.iter().map(|p| tokens(&p.line())).sum::<usize>() <= room);
    assert!(map.pointers.windows(2).all(|w| (w[0].file.as_str(), w[0].start) < (w[1].file.as_str(), w[1].start)));
    assert_eq!(map.rest.len(), 2, "{:?}", map.rest);
    assert!(map.rest[0].starts_with("src/a.rs (10 lines): "));
    assert!(map.rest[1].starts_with("src/b.rs (10 lines): "));
}

#[test]
fn map_finds_outside_the_card_files_a_name_they_do_not_define() {
    let texts: HashMap<&str, &str> = HashMap::from([
        ("crates/x/src/lib.rs", "fn alpha() {}\n"),
        ("apps/rc/src/main.rs", "fn main() {}\n\nfn handle_note(line: &str) {}\n"),
    ]);
    let read = |file: &str| texts.get(file).map(|t| (*t).to_owned());
    let files = ["crates/x/src/lib.rs".to_owned()];
    let sources = ["apps/rc/src/main.rs".to_owned(), "crates/x/src/lib.rs".to_owned(), "gone.rs".to_owned()];
    let card = "fix `alpha`, wire `handle_note()` to `Ledger::brand_new`; check with `cargo test -p x` in `apps/rc/`";
    let map = map_of(read, &files, &Ask { card, sources: &sources, ..Ask::default() }, &ChangedLines::new(), 10_000);
    let outside: Vec<&Pointer> = map.pointers.iter().filter(|p| p.file == "apps/rc/src/main.rs").collect();
    assert_eq!(outside.len(), 1, "{map:?}");
    assert_eq!((outside[0].item.as_str(), outside[0].start, outside[0].end), ("fn handle_note(line: &str)", 3, 3));
    assert_eq!(outside[0].score, 100);
    assert_eq!(map.pointers.iter().filter(|p| p.item == "fn alpha()").count(), 1, "{map:?}");
    assert_eq!(map.unresolved, ["Ledger::brand_new"]);
    let rendered = map.render(&read, 0);
    assert!(
        rendered.contains("apps/rc/src/main.rs 3-3  fn handle_note(line: &str)  · named by the card, outside its files\n"),
        "{rendered}"
    );
    assert!(rendered.ends_with("nothing defines: Ledger::brand_new; treat it as new\n"), "{rendered}");
}

fn function_of(name: &str, lines: usize) -> String {
    let body: String = (0..lines - 2).map(|i| format!("    let value_{i} = {i};\n")).collect();
    format!("fn {name}() {{\n{body}}}\n")
}

#[test]
fn render_quotes_what_fits_the_room_highest_score_first() {
    let twenty = function_of("twenty", 20);
    let (read, files) = files_of(&[("src/a.rs", &twenty)]);
    let ask = Ask { card: "change `twenty`", ..Ask::default() };
    let map = map_of(&read, &files, &ask, &ChangedLines::new(), 10_000);
    let rendered = map.render(&read, 6_000);
    assert!(rendered.contains("src/a.rs 1-20  fn twenty()  · named by the card\n"), "{rendered}");
    assert!(rendered.contains("   1  fn twenty() {\n   2      let value_0 = 0;\n"), "{rendered}");
    assert!(rendered.contains("  20  }\n"), "{rendered}");

    let long = function_of("long", 400);
    let (read, files) = files_of(&[("src/b.rs", &long)]);
    let ask = Ask { card: "change `long`", ..Ask::default() };
    let map = map_of(&read, &files, &ask, &ChangedLines::new(), 10_000);
    let rendered = map.render(&read, 100);
    assert!(rendered.contains("src/b.rs 1-400  fn long()"), "{rendered}");
    assert!(!rendered.contains("let value_0"), "{rendered}");

    let two = format!("{}{}", function_of("high", 14), function_of("low", 10));
    let (read, files) = files_of(&[("src/c.rs", &two)]);
    let changed: ChangedLines = HashMap::from([("src/c.rs".to_owned(), vec![(20, 20)])]);
    let ask = Ask { card: "change `high`", ..Ask::default() };
    let map = map_of(&read, &files, &ask, &changed, 10_000);
    let [high, low] = [&map.pointers[0], &map.pointers[1]];
    assert!(high.item.contains("high") && low.item.contains("low") && high.score > low.score, "{map:?}");
    // The higher-scored body is the dearer: the smallest room that holds it holds the cheaper
    // one alone too, but not both, so only the order decides which is quoted.
    let high_line = "   2      let value_0 = 0;\n";
    let low_line = "  16      let value_0 = 0;\n";
    let room = (0..6_000).find(|&q| map.render(&read, q).contains(high_line)).expect("some room quotes high");
    let low_alone = Map { pointers: vec![low.clone()], ..Map::default() };
    assert!(low_alone.render(&read, room).contains(low_line), "the room holds the cheaper body alone");
    let rendered = map.render(&read, room);
    assert!(rendered.contains(high_line), "{rendered}");
    assert!(!rendered.contains(low_line), "{rendered}");
    assert!(rendered.contains("src/c.rs 15-24  fn low()"), "{rendered}");
}

#[test]
fn map_ranks_nothing_in_a_file_and_prints_its_one_line() {
    let (read, files) =
        files_of(&[("src/a.rs", "fn alpha() {}\nfn beta() {}\n"), ("src/b.rs", "fn zeta() {}\nfn eta() {}\nfn theta() {}\n")]);
    let map = map_of(&read, &files, &Ask { card: "fix `alpha`", ..Ask::default() }, &ChangedLines::new(), 10_000);
    let rendered = map.render(&read, 0);
    assert!(rendered.contains("src/a.rs 1-1  fn alpha()  · named by the card\n"), "{rendered}");
    assert!(rendered.contains("src/a.rs (2 lines): 1 more items not shown\n"), "{rendered}");
    assert!(rendered.contains("src/b.rs (3 lines): 3 items, none the card points at\n"), "{rendered}");
    assert_eq!(rendered.lines().filter(|l| l.starts_with("src/b.rs")).count(), 1);
}
