// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The two recall paths that do not need an embedding model.
//!
//! `documents/design/recall-mechanisms.md` names three ways into the memory graph: semantic
//! similarity, keyword search, and direct reference. These are the other two, and they are what
//! makes the promise that a missing model *degrades* rather than disables true.

use std::str::FromStr;

use jiff::Timestamp;
use scatterhoard::memory::{
    Attribution, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId, MemoryText,
    Origin, Scope, SetName,
};
use scatterhoard::memory_file::MemorySet;
use scatterhoard::store::{Resolution, Store};

fn scope() -> Scope {
    Scope::Shared(SetName::new("scatterhoard").expect("valid set name"))
}

/// Note the absence of an embedder: neither path needs one.
fn a_store() -> Store {
    Store::open_in_memory_and_mint_id(scope()).expect("opens")
}

fn a_memory(content: &str, keywords: &[&str]) -> Memory {
    let mut memory = Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: Timestamp::from_str("2026-08-02T09:00:00Z").expect("valid timestamp"),
        },
    );
    memory.keywords = keywords
        .iter()
        .map(|word| Keyword::new(*word).expect("valid keyword"))
        .collect();
    memory
}

fn keywords(words: &[&str]) -> Vec<Keyword> {
    words
        .iter()
        .map(|word| Keyword::new(*word).expect("valid keyword"))
        .collect()
}

// ---------------------------------------------------------------------------
// Keyword search
// ---------------------------------------------------------------------------

/// **More matches than one batch, in two tiers, across two stores.** Keyword search reads each
/// store's matches a bounded batch at a time since 2026-09-29, continuing each batch from the last
/// `(matches, id)` it read; every other keyword test has a handful of matches, so none of them ever
/// reaches a second batch. Two hundred matches -- a hundred carrying both keywords and a hundred
/// carrying one, split between two stores -- cross several batches and put a tier boundary inside
/// one, which is where a continuation that compared the wrong column would skip or repeat rows.
#[test]
fn keyword_search_ranks_and_pages_correctly_across_several_batches() {
    let mut first = a_store();
    let mut second = Store::open_in_memory_and_mint_id(Scope::Shared(
        SetName::new("elsewhere").expect("valid set name"),
    ))
    .expect("opens");

    let mut both: Vec<MemoryId> = Vec::new();
    let mut one: Vec<MemoryId> = Vec::new();
    for n in 0..200 {
        let words: &[&str] = if n % 2 == 0 {
            &["paging", "deep"]
        } else {
            &["paging"]
        };
        let memory = a_memory(&format!("memory {n}"), words);
        if n % 2 == 0 {
            both.push(memory.id);
        } else {
            one.push(memory.id);
        }
        let store = if n % 3 == 0 { &mut first } else { &mut second };
        store.insert(&memory).expect("inserts");
    }
    both.sort();
    one.sort();
    let expected: Vec<MemoryId> = both.iter().chain(&one).copied().collect();
    let query = keywords(&["paging", "deep"]);

    let all: Vec<MemoryId> =
        scatterhoard::store::search_keywords_across(&[&first, &second], &query, 500)
            .expect("searches")
            .iter()
            .map(|hit| hit.memory.id)
            .collect();
    assert_eq!(
        all, expected,
        "every match exactly once, both-keyword tier first, each tier oldest first"
    );

    // Paging past a held prefix that ends mid-batch and mid-tier returns exactly the next ones.
    let held = &expected[..130];
    let page: Vec<MemoryId> =
        scatterhoard::store::search_keywords_across_holding(&[&first, &second], &query, 50, held)
            .expect("searches")
            .iter()
            .map(|hit| hit.memory.id)
            .collect();
    assert_eq!(page, expected[130..180].to_vec());
}

#[test]
fn keyword_search_works_without_an_embedding_model() {
    let mut store = a_store();
    let wanted = a_memory("about storage", &["storage", "scope"]);
    store.insert(&wanted).expect("inserts");
    store
        .insert(&a_memory("about something else", &["merging"]))
        .expect("inserts");

    let hits = store
        .search_keywords(&keywords(&["storage"]), 5)
        .expect("searches");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.id, wanted.id);
}

/// Ranked by how many of the requested keywords matched, and the count is reported rather than
/// dressed up as a similarity. A match count and a cosine score are different scales; merging them
/// into one number is the mistake that made cross-store scores incomparable.
#[test]
fn keyword_hits_are_ranked_by_how_many_matched_and_say_so() {
    let mut store = a_store();
    let both = a_memory("matches both", &["storage", "scope"]);
    let one = a_memory("matches one", &["storage"]);
    store.insert(&one).expect("inserts");
    store.insert(&both).expect("inserts");

    let hits = store
        .search_keywords(&keywords(&["storage", "scope"]), 5)
        .expect("searches");

    assert_eq!(hits[0].memory.id, both.id);
    assert_eq!(hits[0].matched_keywords, 2);
    assert_eq!(hits[1].matched_keywords, 1);
}

/// Keywords are normalised at construction, so the search finds a memory however the caller
/// capitalised the word.
#[test]
fn keyword_search_ignores_capitalisation() {
    let mut store = a_store();
    let memory = a_memory("about storage", &["storage"]);
    store.insert(&memory).expect("inserts");

    let hits = store
        .search_keywords(&keywords(&["STORAGE"]), 5)
        .expect("searches");
    assert_eq!(hits[0].memory.id, memory.id);
}

/// Keyword search is *search*, so it resolves validity like semantic recall does: a superseded
/// memory is replaced by whatever corrected it rather than returned as though current.
#[test]
fn keyword_search_does_not_return_a_superseded_memory() {
    let mut store = a_store();
    let old = a_memory("the earlier conclusion", &["hubness"]);
    let mut new = a_memory("the correction", &["hubness"]);
    new.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(old.id),
    }];
    store.insert(&old).expect("inserts");
    store.insert(&new).expect("inserts");

    let hits = store
        .search_keywords(&keywords(&["hubness"]), 5)
        .expect("searches");

    assert!(hits.iter().all(|hit| hit.memory.id != old.id));
    assert!(hits.iter().any(|hit| hit.memory.id == new.id));
}

/// The case the test above cannot see, and the one the change is for.
///
/// **Renamed from `keyword_search_substitutes_a_superseded_memory` on 2026-09-02.** Both of its
/// memories carry the term, so it passed before the change by substitution and passes after it by
/// the replacement matching on its own account — a test that goes green for a different reason than
/// its name claims. Its assertions are still worth keeping; what it never covered is the case where
/// the replacement **dropped** the term, which is the whole of
/// `a-keyword-cannot-be-retired-once-a-memory-has-been-exported` in `TRACKING.md`.
#[test]
fn a_keyword_the_replacement_dropped_matches_nothing() {
    let mut store = a_store();
    let old = a_memory("the earlier conclusion", &["hubness"]);
    let mut new = a_memory("the correction", &["centrality"]);
    new.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(old.id),
    }];
    store.insert(&old).expect("inserts");
    store.insert(&new).expect("inserts");

    let hits = store
        .search_keywords(&keywords(&["hubness"]), 5)
        .expect("searches");

    assert!(
        hits.is_empty(),
        "the replacement does not carry `hubness` and the superseded memory is not reachable \
         through it, so nothing matches: {:?}",
        hits.iter().map(|hit| hit.memory.id).collect::<Vec<_>>()
    );
}

#[test]
fn keyword_search_with_no_keywords_finds_nothing() {
    let mut store = a_store();
    store
        .insert(&a_memory("anything", &["storage"]))
        .expect("inserts");

    assert!(store.search_keywords(&[], 5).expect("searches").is_empty());
}

// ---------------------------------------------------------------------------
// Direct reference
// ---------------------------------------------------------------------------

#[test]
fn a_memory_can_be_looked_up_by_its_full_id() {
    let mut store = a_store();
    let memory = a_memory("findable", &[]);
    store.insert(&memory).expect("inserts");

    let found = store
        .lookup(memory.id)
        .expect("looks up")
        .expect("a whole id finds its memory");
    assert_eq!(found.memory.id, memory.id);
}

/// **Reversed by `crockford-base32-ids`.** `recall-mechanisms.md` used to ask that a truncated id
/// resolve, because models truncate UUIDs the way people truncate commit hashes. Accommodating that
/// is what made the habit succeed *silently*, so recovery is now a fragment search the caller has to
/// choose — and it takes a piece from anywhere, because the front of an id is its timestamp and is
/// the one place worth not cutting.
#[test]
fn a_fragment_from_the_end_of_an_id_finds_the_memory() {
    let mut store = a_store();
    let memory = a_memory("findable", &[]);
    store.insert(&memory).expect("inserts");

    let whole = memory.id.to_string();
    let tail = &whole[whole.len() - 6..];
    let found = store.find_by_fragment(tail).expect("searches");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].memory.id, memory.id);
}

/// The mapping that made Crockford the right alphabet rather than merely a denser one.
#[test]
fn a_fragment_transcribed_with_confusable_characters_still_resolves() {
    let mut store = a_store();
    let memory = a_memory("findable", &[]);
    store.insert(&memory).expect("inserts");

    let whole = memory.id.to_string();
    let tail = &whole[whole.len() - 8..];
    let mangled = tail.replace('0', "O").replace('1', "I");

    let found = store.find_by_fragment(&mangled).expect("searches");
    assert_eq!(
        found.len(),
        1,
        "`O` for zero and `I` for one must still resolve: {mangled}"
    );
}

#[test]
fn an_id_fragment_matching_nothing_finds_nothing() {
    let store = a_store();
    assert!(
        store
            .find_by_fragment("ZZZZZZZZ")
            .expect("searches")
            .is_empty()
    );
}

/// Junk is not a fragment. Keeping `%` and `_` out of the pattern matters as much as the refusal.
#[test]
fn a_fragment_that_is_not_id_characters_matches_nothing() {
    let mut store = a_store();
    store.insert(&a_memory("findable", &[])).expect("inserts");

    for junk in ["%", "_", "", "  ", "not an id"] {
        assert!(
            store.find_by_fragment(junk).expect("searches").is_empty(),
            "`{junk}` should not match anything"
        );
    }
}

/// **The exemption.** Asking for a memory by id means wanting *that* memory, so lookup returns it
/// even when it has been superseded — flagged, not substituted. Without this the prior version is
/// unreachable, which would make superseding equivalent to forgetting and defeat the decision to
/// keep it.
#[test]
fn looking_up_a_superseded_memory_returns_it_flagged_not_replaced() {
    let mut store = a_store();
    let old = a_memory("the earlier conclusion", &[]);
    let mut new = a_memory("the correction", &[]);
    new.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(old.id),
    }];
    store.insert(&old).expect("inserts");
    store.insert(&new).expect("inserts");

    let found = store
        .lookup(old.id)
        .expect("looks up")
        .expect("a whole id finds its memory");

    assert_eq!(
        found.memory.id, old.id,
        "lookup must return the memory asked for, not its replacement"
    );
    assert_eq!(
        found.validity,
        Resolution::Superseded {
            tips: vec![new.id],
            unresolved: false,
            forgotten: false,
        },
        "but it has to say the memory has been replaced, and by what"
    );
}

#[test]
fn a_current_memory_is_looked_up_without_a_supersession_flag() {
    let mut store = a_store();
    let memory = a_memory("still true", &[]);
    store.insert(&memory).expect("inserts");

    let found = store
        .lookup(memory.id)
        .expect("looks up")
        .expect("a whole id finds its memory");
    assert_eq!(found.validity, Resolution::Current);
}

/// A prefix short enough to be ambiguous returns every match rather than guessing — the caller has
/// the context to choose, and picking one silently would be the wrong memory half the time.
#[test]
fn an_ambiguous_prefix_returns_every_match() {
    let mut store = a_store();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let memory = a_memory("one of several", &[]);
        ids.push(memory.id.to_string());
        store.insert(&memory).expect("inserts");
    }

    // Ids are UUIDv7, so memories created in the same millisecond share a long prefix. The prefix
    // is taken from a real id rather than written as a literal: those leading characters are a
    // timestamp, so a literal silently stops matching once the clock passes it. `"019"` did exactly
    // that -- see `prefix-test-literal-expired-with-the-clock` in TRACKING.md.
    let head = &ids[0][..3];
    let found = store.find_by_fragment(head).expect("searches");
    assert_eq!(
        found.len(),
        3,
        "a fragment from the head matches everything written nearby, which is exactly why the \
         surface steers callers to the tail: {head}"
    );
}

/// **Lookup does not collapse a chain, and this is what makes walking one by hand possible.**
///
/// The two directions come from different fields, which is worth pinning because it is not obvious
/// from either name:
///
/// - `superseded_by` reports the **end** of the chain, skipping every memory in between. It answers
///   *what should I read instead*, which is the question a caller holding a stale id usually has.
/// - `incoming` carries the `supersedes` edge written by the **immediate** successor, so stepping
///   one link at a time is possible from it.
///
/// The step-by-step direction is the one that matters when the corpus is damaged: a chain with a
/// broken or forked middle cannot be understood from its endpoints, and a caller that can only jump
/// to the tip cannot see where it went wrong.
#[test]
fn a_chain_can_be_walked_one_link_at_a_time_as_well_as_jumped_to_its_end() {
    let mut store = a_store();
    let oldest = a_memory("as first written", &[]);
    let mut middle = a_memory("corrected once", &[]);
    middle.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(oldest.id),
    }];
    let mut newest = a_memory("corrected again", &[]);
    newest.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(middle.id),
    }];
    for memory in [&oldest, &middle, &newest] {
        store.insert(memory).expect("inserts");
    }

    let found = store
        .lookup(oldest.id)
        .expect("looks up")
        .expect("a whole id finds its memory");
    assert_eq!(found.memory.id, oldest.id, "the memory asked for");
    assert_eq!(
        found.validity,
        Resolution::Superseded {
            tips: vec![newest.id],
            unresolved: false,
            forgotten: false,
        },
        "and the end of the chain, not the next link"
    );

    // One link at a time, forwards, from the reverse index.
    let next = |id: MemoryId| -> Option<MemoryId> {
        store
            .lookup(id)
            .expect("looks up")
            .as_ref()
            .and_then(|found| {
                found
                    .incoming
                    .iter()
                    .find(|edge| edge.label == LinkLabel::Supersedes)
                    .map(|edge| edge.source)
            })
    };
    assert_eq!(next(oldest.id), Some(middle.id), "one step forward");
    assert_eq!(next(middle.id), Some(newest.id), "and the next");
    assert_eq!(next(newest.id), None, "the tip is superseded by nothing");

    // And backwards, from the memory's own authored links.
    let previous = store
        .lookup(newest.id)
        .expect("looks up")
        .as_ref()
        .and_then(|found| {
            found
                .memory
                .links
                .iter()
                .find(|link| link.label == LinkLabel::Supersedes)
                .map(|link| link.target.id())
        });
    assert_eq!(previous, Some(middle.id), "one step back");
}

/// **A supersession cycle is a data error, and every tool has to say so.**
///
/// The design handles cycles in two places and is explicit that the second is not redundant: write
/// refuses to create one, and read reports one, because a hand-edit during a merge conflict
/// introduces a cycle no write path ever saw. Resolution detected it all along; both callers threw
/// the answer away, so a cycle read as *current* — byte-identical to a memory nothing supersedes.
#[test]
fn a_memory_in_a_supersession_cycle_is_looked_up_as_neither_current_nor_replaced() {
    let mut store = a_store();
    let mut first = a_memory("one", &[]);
    let mut second = a_memory("two", &[]);
    first.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(second.id),
    }];
    second.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(first.id),
    }];
    // Imported rather than inserted: writing this is refused, which is the other half of the rule.
    store
        .import(&MemorySet {
            scope: scope(),
            memories: vec![first.clone(), second],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("a corrupt file still imports");

    let found = store
        .lookup(first.id)
        .expect("looks up")
        .expect("a whole id finds its memory");

    assert_eq!(found.memory.id, first.id, "the memory asked for");
    assert_eq!(
        found.validity,
        Resolution::Cycle,
        "and the fact that its history is broken, which is the whole reason to look it up"
    );
}

/// A memory nothing supersedes says exactly that, rather than saying nothing.
#[test]
fn a_current_memory_reports_itself_current() {
    let mut store = a_store();
    let memory = a_memory("still true", &[]);
    store.insert(&memory).expect("inserts");

    let found = store
        .lookup(memory.id)
        .expect("looks up")
        .expect("a whole id finds its memory");
    assert_eq!(found.validity, Resolution::Current);
}
