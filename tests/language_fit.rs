// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for scoring how well the loaded model represents a declared language.
//!
//! `recall-is-latin-script-only` in `TRACKING.md`. A set written in a language its model cannot
//! represent still stores every memory, but each vector is close to arbitrary, so recall by meaning
//! returns noise and nothing says why. The score is how many of ten paraphrase triplets the model
//! ranks first: each anchor's paraphrase against every other sentence written in that language. A
//! rank rather than a similarity, because the scale of similarity moves with the model and a rank
//! does not.

use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::language_fit::{self, BUILT_IN, Band, TRIPLETS_PER_LANGUAGE};

/// Places a text by its first word, `t0` to `t9` for a topic and `u0` to `u9` for an unrelated
/// sentence, each on an axis of its own -- so similarity is exactly what the test wrote. A text
/// containing `unknown` has no representation, as a script outside the vocabulary has none.
#[derive(Debug)]
struct AxisEmbedder;

impl Embedder for AxisEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("axis-embedder")
    }

    fn dimension(&self) -> usize {
        20
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|text| {
                if text.contains("unknown") {
                    return Ok(Embedded::Unrepresentable);
                }
                let word = text.split_whitespace().next().expect("a placing word");
                let (kind, index) = word.split_at(1);
                let index: usize = index.parse().expect("a topic number");
                let axis = if kind == "t" { index } else { 10 + index };
                let mut components = vec![0.0; 20];
                components[axis] = 1.0;
                Vector::new(components)
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

/// Every text on the same point: a model that cannot tell anything apart.
#[derive(Debug)]
struct FlatEmbedder;

impl Embedder for FlatEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("flat-embedder")
    }

    fn dimension(&self) -> usize {
        2
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|_| {
                Vector::new(vec![1.0, 1.0])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

/// Ten triplets the axis embedder separates perfectly: anchor and paraphrase share a topic axis,
/// and the unrelated sentence sits on an axis nothing else uses.
fn separable() -> Vec<[String; 3]> {
    (0..TRIPLETS_PER_LANGUAGE)
        .map(|n| {
            [
                format!("t{n} anchor"),
                format!("t{n} paraphrase"),
                format!("u{n} other"),
            ]
        })
        .collect()
}

fn score(embedder: &dyn Embedder, triplets: &[[String; 3]]) -> u8 {
    let borrowed: Vec<[&str; 3]> = triplets
        .iter()
        .map(|[a, p, u]| [a.as_str(), p.as_str(), u.as_str()])
        .collect();
    language_fit::score(embedder, &borrowed).expect("the model embeds")
}

#[test]
fn a_model_that_separates_every_paraphrase_scores_ten() {
    assert_eq!(score(&AxisEmbedder, &separable()), 10);
}

#[test]
fn a_model_that_scores_everything_alike_scores_zero() {
    // A tie is not a win: a model that cannot tell the paraphrase from its rivals has not ranked it
    // first, and counting ties would pass exactly the collapsed scripts the check exists to catch.
    assert_eq!(score(&FlatEmbedder, &separable()), 0);
}

#[test]
fn a_rival_that_outscores_the_paraphrase_costs_that_triplet() {
    let mut triplets = separable();
    // The paraphrase now sits on an unrelated axis, while the anchor still sits on its topic.
    triplets[3][1] = "u7 paraphrase".to_string();
    assert_eq!(score(&AxisEmbedder, &triplets), 9);
}

#[test]
fn a_rival_from_another_triplet_counts_as_a_rival() {
    let mut triplets = separable();
    // The fifth anchor's nearest neighbour is now the second triplet's unrelated sentence, on the
    // same axis as its paraphrase -- a tie, which loses.
    triplets[1][2] = "t4 other".to_string();
    assert_eq!(score(&AxisEmbedder, &triplets), 9);
}

#[test]
fn an_anchor_the_model_cannot_represent_loses_its_triplet() {
    let mut triplets = separable();
    triplets[0][0] = "t0 unknown".to_string();
    assert_eq!(score(&AxisEmbedder, &triplets), 9);
}

#[test]
fn a_paraphrase_the_model_cannot_represent_loses_its_triplet() {
    let mut triplets = separable();
    triplets[0][1] = "t0 unknown".to_string();
    assert_eq!(score(&AxisEmbedder, &triplets), 9);
}

#[test]
fn an_unrepresentable_rival_is_no_rival() {
    // It has no position, so it cannot outrank anything; the triplets it is not part of are
    // unaffected.
    let mut triplets = separable();
    triplets[2][2] = "u2 unknown".to_string();
    assert_eq!(score(&AxisEmbedder, &triplets), 10);
}

#[test]
fn the_bands_split_at_seven_and_four() {
    for (score, band) in [
        (10, Band::Searchable),
        (7, Band::Searchable),
        (6, Band::Unreliable),
        (4, Band::Unreliable),
        (3, Band::Unsearchable),
        (0, Band::Unsearchable),
    ] {
        assert_eq!(Band::of(score), band, "score {score}");
    }
}

#[test]
fn a_searchable_language_needs_no_note() {
    assert_eq!(Band::Searchable.note("Japanese"), None);
}

#[test]
fn an_unreliable_language_is_told_a_better_model_exists() {
    let note = Band::Unreliable.note("Japanese").expect("a note");
    assert!(note.contains("Japanese"), "{note}");
    assert!(note.contains("unreliable"), "{note}");
    // The trade a dedicated model can cost is the human's to weigh, so the note names it.
    assert!(note.contains("other languages"), "{note}");
}

#[test]
fn an_unsearchable_language_is_told_its_memories_are_safe() {
    let note = Band::Unsearchable.note("Japanese").expect("a note");
    assert!(note.contains("Japanese"), "{note}");
    assert!(note.contains("intact"), "{note}");
    assert!(note.contains("keyword"), "{note}");
}

#[test]
fn every_built_in_language_has_ten_triplets_of_distinct_sentences() {
    let mut tags = std::collections::HashSet::new();
    for language in &BUILT_IN {
        assert!(tags.insert(language.tag), "{} listed twice", language.tag);
        let sentences: std::collections::HashSet<&str> =
            language.triplets.iter().flatten().copied().collect();
        assert_eq!(
            sentences.len(),
            3 * TRIPLETS_PER_LANGUAGE,
            "{} repeats a sentence",
            language.tag
        );
        assert!(sentences.iter().all(|s| !s.trim().is_empty()));
    }
}

#[test]
fn a_built_in_language_is_found_by_its_tag() {
    assert_eq!(
        language_fit::built_in("ja").map(|l| l.name),
        Some("Japanese")
    );
}

#[test]
fn a_regional_tag_finds_its_language() {
    assert_eq!(language_fit::built_in("en-gb").map(|l| l.tag), Some("en"));
}

#[test]
fn simplified_chinese_finds_the_chinese_sentences() {
    assert_eq!(language_fit::built_in("zh-hans").map(|l| l.tag), Some("zh"));
}

#[test]
fn a_script_the_sentences_are_not_written_in_finds_nothing() {
    // The built-in Chinese is written in simplified characters. Scoring a traditional-script set
    // with them would measure the wrong script, so it is left for sentences in its own.
    assert!(language_fit::built_in("zh-hant").is_none());
}

#[test]
fn a_language_with_no_built_in_sentences_finds_nothing() {
    assert!(language_fit::built_in("sm").is_none());
}
