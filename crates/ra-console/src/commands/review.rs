//! The weight of a needs-revision verdict: whether a cheap pass can fix it or the card is one
//! for a person's attention. Rule-based, on the words of the reviewer's points; the tag goes on
//! the card's handoff so the walk and the pair feed can tell the two apart.

/// How much a needs-revision verdict asks of the next run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    /// Every point is a lint warning, a format diff or a missing doc comment: a cheap pass fixes
    /// it and the card is re-judged.
    Trivial,
    /// A point touches behaviour, tests or what the card is for; or says something the rule
    /// cannot place, which stays substantive rather than be fixed blind.
    Substantive,
}

impl Weight {
    /// The tag as it reads on the handoff and the notes.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Trivial => "trivial",
            Self::Substantive => "substantive",
        }
    }
}

/// The words of a point that make it trivial: the lint, the formatter and the doc comment.
const TRIVIAL: [&str; 13] = [
    "clippy",
    "lint",
    "warning",
    "rustfmt",
    "cargo fmt",
    "formatting",
    "formatted",
    "unformatted",
    "doc comment",
    "doc-comment",
    "missing docs",
    "missing_docs",
    "docstring",
];

/// The words of a point that keep it substantive whatever else it names: a lint warning on a
/// test that is wrong is still a wrong test.
const SUBSTANTIVE: [&str; 15] = [
    "test",
    "behaviour",
    "behavior",
    "logic",
    "panic",
    "bug",
    "wrong",
    "incorrect",
    "fail",
    "crash",
    "regress",
    "return",
    "acceptance",
    "objective",
    "not implemented",
];

/// The weight of a verdict's points: trivial only when every point is, and no point is empty.
#[must_use]
pub fn classify(points: &[String]) -> Weight {
    if !points.is_empty() && points.iter().all(|point| weight_of(point) == Weight::Trivial) { Weight::Trivial } else { Weight::Substantive }
}

fn weight_of(point: &str) -> Weight {
    let text = point.to_ascii_lowercase();
    let names = |words: &[&str]| words.iter().any(|word| text.contains(word));
    if names(&SUBSTANTIVE) || !names(&TRIVIAL) { Weight::Substantive } else { Weight::Trivial }
}
