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
    let ask = Ask { card: "the first fn and `third`", points: &[] };
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
    let map = map_of(&read, &files, &Ask { card: "nothing in here", points: &[] }, &changed, 10_000);
    assert_eq!(map.pointers.len(), 1);
    assert_eq!(map.pointers[0].item, "fn second()");
    assert_eq!(map.pointers[0].score, 80);
    assert!(map.pointers[0].why.contains("changed"));
}

#[test]
fn map_ranks_a_common_word_under_a_rare_one() {
    let text = "fn parse_head() {}\nfn parse_line() {}\nfn parse_tail() {}\nfn ledger_total() {}\n";
    let (read, files) = files_of(&[("src/a.rs", text)]);
    let map = map_of(&read, &files, &Ask { card: "parse the ledger", points: &[] }, &ChangedLines::new(), 10_000);
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
    let map = map_of(&read, &files, &Ask { card: &card, points: &[] }, &ChangedLines::new(), room);
    assert!(!map.pointers.is_empty());
    assert!(map.pointers.len() < 20);
    assert!(map.pointers.iter().map(|p| tokens(&p.line())).sum::<usize>() <= room);
    assert!(map.pointers.windows(2).all(|w| (w[0].file.as_str(), w[0].start) < (w[1].file.as_str(), w[1].start)));
    assert_eq!(map.rest.len(), 2, "{:?}", map.rest);
    assert!(map.rest[0].starts_with("src/a.rs (10 lines): "));
    assert!(map.rest[1].starts_with("src/b.rs (10 lines): "));
}

#[test]
fn map_ranks_nothing_in_a_file_and_prints_its_one_line() {
    let (read, files) =
        files_of(&[("src/a.rs", "fn alpha() {}\nfn beta() {}\n"), ("src/b.rs", "fn zeta() {}\nfn eta() {}\nfn theta() {}\n")]);
    let map = map_of(&read, &files, &Ask { card: "fix `alpha`", points: &[] }, &ChangedLines::new(), 10_000);
    let rendered = map.render();
    assert!(rendered.contains("src/a.rs 1-1  fn alpha()  · named by the card\n"), "{rendered}");
    assert!(rendered.contains("src/a.rs (2 lines): 1 more items not shown\n"), "{rendered}");
    assert!(rendered.contains("src/b.rs (3 lines): 3 items, none the card points at\n"), "{rendered}");
    assert_eq!(rendered.lines().filter(|l| l.starts_with("src/b.rs")).count(), 1);
}
