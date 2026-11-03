// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for a set's own model-fit material: the hub, its sentence sets, and the models it
//! recommends.
//!
//! `documents/design/memory-data-model.md`, *the model-fit report*. A set can carry its own
//! paraphrase test sentences, so every copy scores its languages alike and a maintainer can review
//! them, and so that a language the server has no sentences for can be scored at all. One hub per
//! set, marked `hoard-fit-hub`, links out to memories marked `hoard-fit-sentences`, each holding one
//! `[anchor, paraphrase, unrelated]` array and tagged with its language. **Zero or exactly ten per
//! language**, because the number of rivals is part of the measure: a language scored from more or
//! fewer sets would be on a different scale from every other.
//!
//! These are the checks made when a hub or a sentence set is written, which is when a mistake is
//! cheapest to fix. Like every authoring check they are not applied to a memory arriving in a file.

use std::str::FromStr;

use jiff::Timestamp;
use scatterhoard::memory::{
    Attribution, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId, MemoryText,
    Origin, Scope, SetName,
};
use scatterhoard::store::{Store, StoreError};

fn a_store() -> Store {
    Store::open_in_memory_and_mint_id(Scope::Shared(
        SetName::new("scatterhoard").expect("valid set name"),
    ))
    .expect("opens")
}

fn a_memory(content: &str, keywords: &[&str], links: Vec<Link>) -> Memory {
    let mut memory = Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.9).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: Timestamp::from_str("2026-10-04T09:00:00Z").expect("valid timestamp"),
        },
    );
    memory.keywords = keywords
        .iter()
        .map(|keyword| Keyword::new(*keyword).expect("valid keyword"))
        .collect();
    memory.links = links;
    memory
}

fn refers_to(target: MemoryId) -> Link {
    Link {
        label: LinkLabel::RefersTo,
        target: LinkTarget::SameSet(target),
    }
}

fn supersedes(target: MemoryId) -> Link {
    Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(target),
    }
}

/// A sentence set for `tag`, distinct by `n`, with the prose line a reviewer reads before the array.
fn a_sentence_set(tag: &str, n: usize) -> Memory {
    let lang = format!("hoard-lang-{tag}");
    a_memory(
        &format!(
            "Paraphrase test set {n} for {tag}. \
             [\"anchor {n}\", \"paraphrase {n}\", \"unrelated {n}\"]"
        ),
        &["hoard-fit-sentences", &lang],
        Vec::new(),
    )
}

/// Store `count` sentence sets for each tag, and return their ids.
fn stored_sets(store: &mut Store, groups: &[(&str, usize)]) -> Vec<MemoryId> {
    let mut ids = Vec::new();
    for (tag, count) in groups {
        for n in 0..*count {
            let set = a_sentence_set(tag, n);
            store
                .insert(&set)
                .expect("a well-formed sentence set is accepted");
            ids.push(set.id);
        }
    }
    ids
}

fn a_hub(sets: &[MemoryId]) -> Memory {
    a_memory(
        "This set's model-fit hub: its own paraphrase test sentences.",
        &["hoard-fit-hub"],
        sets.iter().copied().map(refers_to).collect(),
    )
}

// ---------------------------------------------------------------------------
// A sentence set
// ---------------------------------------------------------------------------

#[test]
fn a_sentence_set_with_one_language_and_one_triplet_is_accepted() {
    let mut store = a_store();
    store.insert(&a_sentence_set("sm", 0)).expect("accepted");
}

/// A sentence set is scored for its language and no other, so it has to say which one.
#[test]
fn a_sentence_set_with_no_language_is_refused() {
    let mut store = a_store();
    let set = a_memory(
        "[\"anchor\", \"paraphrase\", \"unrelated\"]",
        &["hoard-fit-sentences"],
        Vec::new(),
    );
    let error = store.insert(&set).expect_err("refused");
    assert!(
        matches!(error, StoreError::MalformedFitSentences { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("hoard-lang-"), "{error}");
}

#[test]
fn a_sentence_set_with_two_languages_is_refused() {
    let mut store = a_store();
    let set = a_memory(
        "[\"anchor\", \"paraphrase\", \"unrelated\"]",
        &["hoard-fit-sentences", "hoard-lang-sm", "hoard-lang-mi"],
        Vec::new(),
    );
    assert!(matches!(
        store.insert(&set),
        Err(StoreError::MalformedFitSentences { .. })
    ));
}

#[test]
fn a_sentence_set_with_no_array_is_refused() {
    let mut store = a_store();
    let set = a_memory(
        "anchor, paraphrase, unrelated",
        &["hoard-fit-sentences", "hoard-lang-sm"],
        Vec::new(),
    );
    let error = store.insert(&set).expect_err("refused");
    // The refusal shows the shape, because a caller told only that the text is wrong has to guess.
    assert!(error.to_string().contains("[\""), "{error}");
}

#[test]
fn a_sentence_set_of_two_sentences_is_refused() {
    let mut store = a_store();
    let set = a_memory(
        "[\"anchor\", \"paraphrase\"]",
        &["hoard-fit-sentences", "hoard-lang-sm"],
        Vec::new(),
    );
    assert!(store.insert(&set).is_err());
}

#[test]
fn a_sentence_set_with_an_empty_sentence_is_refused() {
    let mut store = a_store();
    let set = a_memory(
        "[\"anchor\", \"  \", \"unrelated\"]",
        &["hoard-fit-sentences", "hoard-lang-sm"],
        Vec::new(),
    );
    assert!(store.insert(&set).is_err());
}

/// Non-ASCII escaped as `\uXXXX` is valid JSON, and reasonable for an agent to write.
#[test]
fn a_sentence_set_written_with_unicode_escapes_is_accepted() {
    let mut store = a_store();
    let set = a_memory(
        "Hindi set. [\"\\u0915\\u093e\", \"\\u0916\", \"\\u0917\"]",
        &["hoard-fit-sentences", "hoard-lang-hi"],
        Vec::new(),
    );
    store.insert(&set).expect("accepted");
}

// ---------------------------------------------------------------------------
// The hub
// ---------------------------------------------------------------------------

/// A hub with no sentence sets is how a set carries recommendations alone, for a language the
/// server has sentences for -- a set about art wanting a model better at French terms.
#[test]
fn a_hub_with_no_sentence_sets_is_accepted() {
    let mut store = a_store();
    store.insert(&a_hub(&[])).expect("accepted");
}

#[test]
fn a_hub_with_ten_sets_for_a_language_is_accepted() {
    let mut store = a_store();
    let sets = stored_sets(&mut store, &[("sm", 10)]);
    store.insert(&a_hub(&sets)).expect("accepted");
}

#[test]
fn a_hub_with_nine_sets_for_a_language_is_refused() {
    let mut store = a_store();
    let sets = stored_sets(&mut store, &[("sm", 9)]);
    let error = store.insert(&a_hub(&sets)).expect_err("refused");
    assert!(
        matches!(error, StoreError::IncompleteFitHub { .. }),
        "{error}"
    );
    let message = error.to_string();
    assert!(message.contains("sm") && message.contains('9'), "{message}");
}

#[test]
fn a_hub_with_ten_sets_for_each_of_two_languages_is_accepted() {
    let mut store = a_store();
    let sets = stored_sets(&mut store, &[("sm", 10), ("mi", 10)]);
    store.insert(&a_hub(&sets)).expect("accepted");
}

/// **Counted per language, not as a total.** Twenty sets is a multiple of ten, but seven and
/// thirteen score neither language on the scale every other language is scored on.
#[test]
fn a_hub_whose_total_is_a_multiple_of_ten_but_whose_languages_are_not_is_refused() {
    let mut store = a_store();
    let sets = stored_sets(&mut store, &[("sm", 7), ("mi", 13)]);
    assert!(matches!(
        store.insert(&a_hub(&sets)),
        Err(StoreError::IncompleteFitHub { .. })
    ));
}

/// **Region is ignored**: American and British English are one language to a model.
#[test]
fn sets_differing_only_in_region_count_as_one_language() {
    let mut store = a_store();
    let sets = stored_sets(&mut store, &[("en-us", 5), ("en-gb", 5)]);
    store.insert(&a_hub(&sets)).expect("accepted");
}

/// **Script is not**: hieroglyphs and their transliteration are different text to a model.
#[test]
fn sets_differing_in_script_count_as_two_languages() {
    let mut store = a_store();
    let sets = stored_sets(&mut store, &[("egy-latn", 5), ("egy-egyp", 5)]);
    assert!(matches!(
        store.insert(&a_hub(&sets)),
        Err(StoreError::IncompleteFitHub { .. })
    ));
}

/// The hub may link to other things -- the charter, a note on how the sentences were chosen -- and
/// only memories carrying `hoard-fit-sentences` are its sentence sets.
#[test]
fn a_hub_link_to_a_memory_that_is_not_a_sentence_set_is_not_counted() {
    let mut store = a_store();
    // Tagged with the language, as a note about the Samoan sentences would be, so only the mark can
    // be what keeps it from counting as an eleventh set.
    let note = a_memory(
        "how these sentences were chosen",
        &["hoard-lang-sm"],
        Vec::new(),
    );
    store.insert(&note).expect("accepted");
    let mut sets = stored_sets(&mut store, &[("sm", 10)]);
    sets.push(note.id);
    store.insert(&a_hub(&sets)).expect("accepted");
}

#[test]
fn a_second_live_hub_is_refused() {
    let mut store = a_store();
    store.insert(&a_hub(&[])).expect("accepted");
    assert!(matches!(
        store.insert(&a_hub(&[])),
        Err(StoreError::SecondLiveFitHub { .. })
    ));
}

#[test]
fn a_hub_that_supersedes_the_live_one_is_accepted() {
    let mut store = a_store();
    let first = a_hub(&[]);
    store.insert(&first).expect("accepted");
    let sets = stored_sets(&mut store, &[("sm", 10)]);
    let mut second = a_hub(&sets);
    second.links.push(supersedes(first.id));
    store.insert(&second).expect("accepted");
}

/// **A file is never refused for it**: a merge can bring in an incomplete hub, and a refusal there
/// would stop the import with the store partly written. Status reports it instead.
#[test]
fn an_incomplete_hub_arriving_in_a_file_is_accepted() {
    let mut store = a_store();
    let sets = stored_sets(&mut store, &[("sm", 3)]);
    store
        .accept_from_file(&a_hub(&sets))
        .expect("a file is reconciled, not judged");
}

/// **A set is fixed by superseding it alone**, and the hub's link follows to the replacement -- so
/// a hub written before the fix, or one listing the old id, still counts ten.
#[test]
fn a_superseded_sentence_set_counts_as_its_replacement() {
    let mut store = a_store();
    let sets = stored_sets(&mut store, &[("sm", 10)]);
    let mut replacement = a_sentence_set("sm", 99);
    replacement.links.push(supersedes(sets[4]));
    store.insert(&replacement).expect("accepted");

    let hub = a_hub(&sets);
    let listed = store.fit_sentence_sets(&hub).expect("reads");
    assert!(
        listed.iter().any(|(_, set)| set.id == replacement.id),
        "the replacement is what the hub now lists"
    );
    assert!(listed.iter().all(|(_, set)| set.id != sets[4]));
    store.insert(&hub).expect("still ten");
}

// ---------------------------------------------------------------------------
// A recommendation
// ---------------------------------------------------------------------------

fn a_recommendation(links: Vec<Link>) -> Memory {
    a_memory(
        "potion-multilingual-128M serves this set's Samoan better than the default.",
        &["hoard-fit-model", "hoard-lang-sm"],
        links,
    )
}

/// **The link to the hub is how the server finds a recommendation**, so one without it would carry
/// a reserved mark and do nothing -- refused when written, where it is cheapest to fix.
#[test]
fn a_recommendation_linking_to_no_hub_is_refused() {
    let mut store = a_store();
    let error = store
        .insert(&a_recommendation(Vec::new()))
        .expect_err("refused");
    assert!(
        matches!(error, StoreError::UnlinkedFitModel { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("hoard-fit-hub"), "{error}");
}

/// **With no hub, the refusal shows the hub to write**: told only to write one first, a small model
/// linked to the charter instead and repeated the refused call four times (2026-10-04).
#[test]
fn the_refusal_with_no_hub_shows_the_hub_to_write_first() {
    let mut store = a_store();
    let message = store
        .insert(&a_recommendation(Vec::new()))
        .expect_err("refused")
        .to_string();
    assert!(message.contains("has no hub"), "{message}");
    assert!(
        message.contains("\"keywords\": [\"hoard-fit-hub\"]"),
        "the refusal should show the hub as a call would write it: {message}"
    );
}

/// With a hub, the refusal names it, since linking to it is the whole of the fix.
#[test]
fn the_refusal_with_a_hub_names_it() {
    let mut store = a_store();
    let hub = a_hub(&[]);
    store.insert(&hub).expect("accepted");
    let message = store
        .insert(&a_recommendation(Vec::new()))
        .expect_err("refused")
        .to_string();
    assert!(message.contains(&hub.id.to_string()), "{message}");
}

#[test]
fn a_recommendation_linking_to_the_hub_is_accepted() {
    let mut store = a_store();
    let hub = a_hub(&[]);
    store.insert(&hub).expect("accepted");
    store
        .insert(&a_recommendation(vec![refers_to(hub.id)]))
        .expect("accepted");
}

#[test]
fn a_recommendation_linking_to_something_other_than_a_hub_is_refused() {
    let mut store = a_store();
    let other = a_memory("not a hub", &[], Vec::new());
    store.insert(&other).expect("accepted");
    assert!(matches!(
        store.insert(&a_recommendation(vec![refers_to(other.id)])),
        Err(StoreError::UnlinkedFitModel { .. })
    ));
}

/// A recommendation written against a hub since superseded reaches the hub that replaced it, as a
/// sentence set does -- so replacing the hub never strands what links to it.
#[test]
fn a_recommendation_linking_to_a_superseded_hub_reaches_its_replacement() {
    let mut store = a_store();
    let first = a_hub(&[]);
    store.insert(&first).expect("accepted");
    let recommendation = a_recommendation(vec![refers_to(first.id)]);
    store.insert(&recommendation).expect("accepted");
    let mut second = a_hub(&[]);
    second.links.push(supersedes(first.id));
    store.insert(&second).expect("accepted");

    let material = store.fit_material().expect("reads");
    assert_eq!(
        material
            .recommendations
            .iter()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        [recommendation.id]
    );
    store
        .insert(&a_recommendation(vec![refers_to(first.id)]))
        .expect("still reaches the live hub");
}

/// A file is never refused for an unlinked recommendation, but one is found only through the hub,
/// so it is not listed -- it recommends nothing until something links it.
#[test]
fn an_unlinked_recommendation_arriving_in_a_file_is_not_listed() {
    let mut store = a_store();
    store.insert(&a_hub(&[])).expect("accepted");
    store
        .accept_from_file(&a_recommendation(Vec::new()))
        .expect("a file is reconciled, not judged");
    assert!(
        store
            .fit_material()
            .expect("reads")
            .recommendations
            .is_empty()
    );
}

/// **A link naming its own set is a same-set link**, as the write stores it -- so a recommendation
/// spelled that way reaches the hub, and was refused until the checks asked the write's question.
/// A small model wrote exactly this on 2026-10-04 and was refused five times running.
#[test]
fn a_recommendation_linking_to_the_hub_by_naming_its_own_set_is_accepted() {
    let mut store = a_store();
    let hub = a_hub(&[]);
    store.insert(&hub).expect("accepted");
    let own_set = Link {
        label: LinkLabel::RefersTo,
        target: LinkTarget::CrossSet {
            set: SetName::new("scatterhoard").expect("valid set name"),
            id: hub.id,
            keywords: Vec::new(),
            set_id: None,
        },
    };
    store
        .insert(&a_recommendation(vec![own_set]))
        .expect("the link names this set, so it is a same-set link");
    assert_eq!(
        store.fit_material().expect("reads").recommendations.len(),
        1
    );
}

/// The same for a hub listing its sentence sets that way.
#[test]
fn a_hub_listing_sets_by_naming_its_own_set_counts_them() {
    let mut store = a_store();
    let sets = stored_sets(&mut store, &[("sm", 9)]);
    let mut hub = a_hub(&[]);
    hub.links = sets
        .iter()
        .map(|id| Link {
            label: LinkLabel::RefersTo,
            target: LinkTarget::CrossSet {
                set: SetName::new("scatterhoard").expect("valid set name"),
                id: *id,
                keywords: Vec::new(),
                set_id: None,
            },
        })
        .collect();
    assert!(
        matches!(store.insert(&hub), Err(StoreError::IncompleteFitHub { .. })),
        "nine sets named by their own set are still nine sets"
    );
}
