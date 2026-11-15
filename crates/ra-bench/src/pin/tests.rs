use ra_roadmap::markdown;
use ra_roadmap::model::NodeKind;
use ra_roadmap::tree;

use super::*;

/// The embedded roadmap's text.
fn text() -> String {
    String::from_utf8(Bundle::embedded().get(ROADMAP).expect("the roadmap is embedded").to_vec()).expect("the roadmap is UTF-8")
}

/// The embedded roadmap, parsed.
fn nodes() -> Vec<Node> {
    roadmap(&Bundle::embedded()).expect("the roadmap parses")
}

/// The card `id` of the roadmap `text`, hashed without a hidden check.
fn hash_of_card(text: &str, id: i64) -> String {
    let nodes = markdown::parse(text).expect("the roadmap parses");
    card_hash(nodes.iter().find(|n| n.id == id).expect("the card is on the roadmap"), None)
}

#[test]
fn the_roadmap_loads_and_saves_back_to_the_same_bytes() {
    let text = text();
    let roadmap = markdown::parse_file(&text).expect("the roadmap parses");
    assert_eq!(markdown::render_file(&roadmap), text);
}

#[test]
fn the_roadmap_has_eight_tasks_under_one_milestone() {
    let nodes = nodes();
    let milestones: Vec<&Node> = nodes.iter().filter(|n| n.kind == NodeKind::Milestone).collect();
    assert_eq!(milestones.len(), 1);
    let tasks: Vec<&Node> = nodes.iter().filter(|n| n.kind == NodeKind::Task).collect();
    assert_eq!(tasks.len(), 8);
    assert!(tasks.iter().all(|t| t.parent == Some(milestones[0].id)));
    assert_eq!(nodes.len(), 10);
}

#[test]
fn on_the_roadmap_only_card_6_waits_and_on_4_and_5() {
    let waiting: Vec<(i64, Vec<i64>)> = nodes().into_iter().filter(|n| !n.depends_on.is_empty()).map(|n| (n.id, n.depends_on)).collect();
    assert_eq!(waiting, vec![(6, vec![4, 5])]);
}

#[test]
fn the_roadmap_lint_reports_no_fault() {
    assert_eq!(tree::lint(&nodes()), Vec::<String>::new());
}

#[test]
fn each_manifest_card_names_a_roadmap_task() {
    let bundle = Bundle::embedded();
    let manifest = manifest(&bundle).expect("the manifest reads");
    let ids: Vec<i64> = manifest.cards.iter().map(|c| c.id).collect();
    assert_eq!(ids, (3..=10).collect::<Vec<_>>());
    let nodes = nodes();
    for card in &manifest.cards {
        assert!(card_hash_in(&bundle, &nodes, card).is_some(), "card {} hashes", card.id);
    }
}

#[test]
fn a_manifest_whose_cards_are_not_the_roadmap_tasks_is_refused() {
    let nodes = nodes();
    let text = String::from_utf8(Bundle::embedded().get(MANIFEST).expect("the manifest is embedded").to_vec()).expect("UTF-8");
    let stray = text.replace("id = 10\n", "id = 11\n");
    assert!(read(&stray, &nodes).expect_err("a stray card is refused").contains("card 11"));
    let short = &text[..text.rfind("[[card]]").expect("a card")];
    assert!(read(short, &nodes).expect_err("a missing card is refused").contains("task 10"));
    let twice = text.replace("id = 10\n", "id = 9\n");
    assert!(read(&twice, &nodes).expect_err("a card named twice is refused").contains("twice"));
}

#[test]
fn a_card_hash_is_unchanged_by_reindenting_the_roadmap_file() {
    let text = text();
    let reindented: String = text
        .lines()
        .map(|line| {
            let body = line.trim_start();
            format!("{}{body}\n", " ".repeat(2 * (line.len() - body.len())))
        })
        .collect();
    assert_ne!(reindented, text);
    for id in 3..=10 {
        assert_eq!(hash_of_card(&reindented, id), hash_of_card(&text, id), "card {id}");
    }
}

#[test]
fn one_word_of_its_acceptance_changes_a_roadmap_card_hash() {
    let text = text();
    let changed = text.replace("one of them on equal sums", "one of them on equal totals");
    assert_ne!(changed, text);
    assert_ne!(hash_of_card(&changed, 3), hash_of_card(&text, 3));
    assert_eq!(hash_of_card(&changed, 4), hash_of_card(&text, 4));
}

#[test]
fn the_roadmap_file_is_not_part_of_the_base() {
    let bundle = Bundle::embedded();
    let files = bundle.paths().map(|path| (path.to_owned(), bundle.get(path).unwrap_or_default().to_vec()));
    let edited = Bundle::from_files(files.map(|(path, bytes)| {
        let bytes = if path == ROADMAP { [bytes.as_slice(), b"\n"].concat() } else { bytes };
        (path, bytes)
    }));
    assert_eq!(base_hash(&edited), base_hash(&bundle));
}
