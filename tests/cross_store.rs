// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Recall across more than one store.
//!
//! Storage is partitioned by scope, but a session usually wants **project ∪ local** — a stated
//! preference matters while working on a project. Per-store centroid removal puts each store's
//! scores on its own scale, so merging them by raw score is unsound. These tests pin the property
//! that decides how to fix that.

use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::memory::{
    Attribution, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId, MemoryText,
    Origin, Scope, SetName,
};
use scatterhoard::memory_file::MemorySet;
use scatterhoard::store::{
    Centring, Question, Resolution, Stages, Store, recall_across, recall_across_with,
    referenced_from_other_sets, resolve_within, search_keywords_across,
};

#[derive(Debug)]
struct MarkerEmbedder;

impl Embedder for MarkerEmbedder {
    fn identity(&self) -> ModelIdentity {
        // One identity for every test double, because these tests are about behaviour that assumes
        // a consistent model. The one place two models have to disagree is tests/model_identity.rs.
        ModelIdentity::new("marker-embedder")
    }

    fn dimension(&self) -> usize {
        3
    }
    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|text| {
                let count = |w: &str| text.matches(w).count() as f32;
                Vector::new(vec![count("alpha"), count("beta"), count("gamma") + 0.01])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

/// How many pages a test will ask for before deciding the paging is broken. A bound rather than a
/// `loop`, because the mutation that stops `already_known` suppressing turns an unbounded version
/// into an infinite one — which reports nothing, where a failure reports the defect by name.
const PAGE_LIMIT: usize = 20;

fn a_memory(content: &str) -> Memory {
    Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: Timestamp::from_str("2026-08-02T09:00:00Z").expect("valid timestamp"),
        },
    )
}

fn store_of(scope: Scope, memories: &[&Memory]) -> Store {
    let mut store = Store::open_in_memory_and_mint_id(scope).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    for memory in memories {
        store.insert(memory).expect("inserts");
    }
    store
}

/// One model, several stores. The embedder is shared rather than cloned because a session's stores
/// must all be embedded by the *same* model: `recall_across` embeds the query once, with whichever
/// embedder it finds first, and then scores that vector against every store's vectors. Two models
/// would make those comparisons meaningless without anything failing.
///
/// The concrete defect this guards is `project-stores-get-no-embedder`: only the local store was
/// given a model, and `embed_all` skips silently without one, so a memory stored in a project set
/// got no vector at all and could never be found by meaning.
#[test]
fn stores_sharing_one_model_are_all_searchable_by_meaning() {
    let local = a_memory("alpha alpha alpha");
    let project_memory = a_memory("alpha alpha beta");

    let model: Arc<dyn Embedder> = Arc::new(MarkerEmbedder);
    let mut local_store = Store::open_in_memory(Scope::Local).expect("opens");
    local_store.set_embedder(Arc::clone(&model));
    local_store.insert(&local).expect("inserts");

    let mut project_store =
        Store::open_in_memory_and_mint_id(project("scatterhoard")).expect("opens");
    project_store.set_embedder(Arc::clone(&model));
    project_store.insert(&project_memory).expect("inserts");

    let recall = recall_across(
        &mut [&mut local_store, &mut project_store],
        &Question::new("alpha", 10),
    )
    .expect("recalls");

    assert!(
        recall.complete,
        "every store has the model, so the union is complete"
    );
    assert!(recall.searched, "and the semantic path ran");
    let found: Vec<_> = recall.hits.iter().map(|hit| hit.memory.id).collect();
    assert!(
        found.contains(&project_memory.id),
        "a memory in the project store was stored without a vector and is unreachable by meaning: \
         found {found:?}"
    );
    assert!(found.contains(&local.id));
}

fn project(name: &str) -> Scope {
    Scope::Shared(SetName::new(name).expect("valid set name"))
}

/// Deliberately spread so centring can reorder them: three crowd one direction and one sits apart.
fn a_corpus() -> Vec<Memory> {
    [
        "alpha alpha alpha",
        "alpha alpha beta",
        "alpha beta beta",
        "gamma gamma gamma",
    ]
    .iter()
    .map(|content| a_memory(content))
    .collect()
}

/// **The property that decides the design.** A corpus split across two stores must answer a query
/// exactly as one store holding all of it would. Anything else means the partitioning — which
/// exists for correctness and trust, not for ranking — has silently changed what recall returns.
///
/// Only a centroid computed over the union can satisfy this. Interleaving by rank cannot (it
/// imposes a quota the single-store case does not have), and comparing raw cosine across stores
/// cannot either (it discards the centring that the single-store case applies).
#[test]
fn splitting_a_corpus_across_stores_does_not_change_the_answer() {
    let corpus = a_corpus();
    let refs: Vec<&Memory> = corpus.iter().collect();

    let mut whole = store_of(project("whole"), &refs);
    let mut left = store_of(project("left"), &refs[..2]);
    let mut right = store_of(Scope::Local, &refs[2..]);

    let query = "alpha alpha gamma";
    let single = whole.recall(&Question::new(query, 4)).expect("recalls");
    let union =
        recall_across(&mut [&mut left, &mut right], &Question::new(query, 4)).expect("recalls");

    let ids = |hits: &[scatterhoard::store::Recalled]| -> Vec<String> {
        hits.iter().map(|hit| hit.memory.id.to_string()).collect()
    };

    assert_eq!(
        ids(&union.hits),
        ids(&single.hits),
        "a split corpus ranked differently from the same memories in one store"
    );
    for (a, b) in union.hits.iter().zip(&single.hits) {
        assert!(
            (a.similarity - b.similarity).abs() < 1e-5,
            "scores diverged: {} vs {}",
            a.similarity,
            b.similarity
        );
    }
}

/// The problem being solved is real, not hypothetical: scoring each store against *its own*
/// centroid and merging by that score gives a different answer. This is the failure the union
/// centroid exists to prevent, asserted so that a future change cannot quietly reintroduce it.
#[test]
fn per_store_centroids_would_give_a_different_answer() {
    let corpus = a_corpus();
    let refs: Vec<&Memory> = corpus.iter().collect();

    let mut left = store_of(project("left"), &refs[..2]);
    let mut right = store_of(Scope::Local, &refs[2..]);
    let query = "alpha alpha gamma";

    // What merging per-store scores would have produced.
    let mut naive: Vec<(f32, String)> = left
        .recall(&Question::new(query, 4))
        .expect("recalls")
        .hits
        .iter()
        .chain(
            right
                .recall(&Question::new(query, 4))
                .expect("recalls")
                .hits
                .iter(),
        )
        .map(|hit| (hit.similarity, hit.memory.id.to_string()))
        .collect();
    naive.sort_by(|a, b| b.0.partial_cmp(&a.0).expect("finite"));

    let union =
        recall_across(&mut [&mut left, &mut right], &Question::new(query, 4)).expect("recalls");
    let merged: Vec<String> = union.hits.iter().map(|h| h.memory.id.to_string()).collect();
    let naive_order: Vec<String> = naive.into_iter().map(|(_, id)| id).collect();

    assert_ne!(
        merged, naive_order,
        "this fixture exists to show per-store centroids ranking differently"
    );
}

/// Recall across stores has to say which store each memory came from, or a local preference and a
/// project decision arrive indistinguishable — and the trust difference between them is exactly
/// what the scope partition exists to preserve.
#[test]
fn each_recalled_memory_reports_the_scope_it_came_from() {
    let corpus = a_corpus();
    let refs: Vec<&Memory> = corpus.iter().collect();
    let mut left = store_of(project("left"), &refs[..2]);
    let mut right = store_of(Scope::Local, &refs[2..]);

    let union = recall_across(
        &mut [&mut left, &mut right],
        &Question::new("alpha gamma", 4),
    )
    .expect("recalls");

    let scopes: Vec<&Scope> = union.hits.iter().map(|hit| &hit.scope).collect();
    assert!(scopes.contains(&&project("left")));
    assert!(scopes.contains(&&Scope::Local));
}

#[test]
fn recall_across_no_stores_finds_nothing_and_says_why() {
    let recall = recall_across(&mut [], &Question::new("alpha", 5)).expect("recalls");
    assert!(recall.hits.is_empty());
    assert!(
        !recall.searched,
        "with no stores at all there was nothing to search by meaning"
    );
}

/// One store without a model does not silently reduce the result to the stores that have one: the
/// caller is told the search was incomplete, for the same reason a single store says so.
#[test]
fn a_store_without_a_model_makes_the_union_search_incomplete() {
    let corpus = a_corpus();
    let refs: Vec<&Memory> = corpus.iter().collect();
    let mut with_model = store_of(project("left"), &refs[..2]);

    let mut without = Store::open_in_memory(Scope::Local).expect("opens");
    without.insert(refs[2]).expect("inserts");

    let recall = recall_across(
        &mut [&mut with_model, &mut without],
        &Question::new("alpha", 5),
    )
    .expect("recalls");

    assert!(
        !recall.hits.is_empty(),
        "the store with a model still answers"
    );
    // `complete` rather than `searched`: the searchable store *was* searched, and saying otherwise
    // beside a real ranking is `semantic-search-available-conflates-partial-with-none`. The split is
    // pinned by `a_partial_union_reports_that_it_searched_and_that_it_was_incomplete`.
    assert!(
        !recall.complete,
        "a store that could not be searched has to be reported"
    );
}

// ---------------------------------------------------------------------------
// What the walk does at a set boundary
// ---------------------------------------------------------------------------

/// A cross-set link is an edge **inside the searched population** when both sets are loaded, which
/// unscoped recall is: it crosses and carries on, same decay, no penalty for the crossing. A
/// cross-set edge decaying faster would be a per-kind weight, and every weight here is held flat.
///
/// Asserted on the inherited score rather than on a position, because a memory the walk lifts need
/// not clear the adaptive cut — and the score is the thing this test is actually about.
#[test]
fn an_unscoped_recall_walks_across_a_set_boundary() {
    // Nine memories, because the walk takes five seeds and a seed never inherits: in a corpus at or
    // below that size every memory is a walk origin and traversal does nothing at all. `distant` is
    // the worst content match of the nine, so nothing but the edge can reach it.
    let contents = [
        "alpha alpha alpha",
        "alpha alpha gamma",
        "alpha gamma gamma",
        "alpha gamma",
        "gamma gamma",
        "gamma",
        "beta beta beta",
        "beta gamma",
    ];
    let matched = a_memory(contents[0]);
    let fillers: Vec<Memory> = contents[1..].iter().map(|text| a_memory(text)).collect();
    let distant = a_memory("gamma gamma gamma");

    let mut linking = distant.clone();
    linking.links = vec![Link {
        label: LinkLabel::Supports,
        target: LinkTarget::CrossSet {
            set: SetName::new("scatterhoard").expect("valid set name"),
            id: matched.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let question = Question::new("alpha alpha", 20);
    let inherited_by_distant = |local_side: &Memory| {
        let shared_memories: Vec<&Memory> =
            std::iter::once(&matched).chain(&fillers[..3]).collect();
        let local_memories: Vec<&Memory> =
            std::iter::once(local_side).chain(&fillers[3..]).collect();
        let mut shared = store_of(project("scatterhoard"), &shared_memories);
        let mut local = store_of(Scope::Local, &local_memories);

        let mut held: Vec<MemoryId> = Vec::new();
        loop {
            let asked = Question {
                already_known: &held,
                ..question.clone()
            };
            let recall = recall_across(&mut [&mut local, &mut shared], &asked).expect("recalls");
            if recall.hits.is_empty() {
                return None;
            }
            for hit in &recall.hits {
                if hit.memory.id == distant.id {
                    return Some(hit.inherited);
                }
                held.push(hit.memory.id);
            }
        }
    };

    assert!(
        inherited_by_distant(&linking).is_some_and(|inherited| inherited > 0.0),
        "the link is an edge inside the searched population, so the walk crosses it"
    );
    assert_eq!(
        inherited_by_distant(&distant),
        Some(0.0),
        "and without the edge nothing reaches it, or this proves nothing"
    );
}

/// **A scoped recall does not walk out of its scope, and the reason is arithmetic.** The centroid
/// was computed over the searched set alone, so a memory from an unsearched set has no
/// own-similarity on that scale — summing that with an inherited score would add two numbers
/// measured against different corpora, which is what the union centroid exists to prevent.
///
/// What the caller gets instead is the link, which carries the target's set and id.
#[test]
fn a_scoped_recall_does_not_walk_out_of_its_scope() {
    let matched = a_memory("alpha alpha alpha");
    let mut linking = a_memory("beta beta beta");
    linking.links = vec![Link {
        label: LinkLabel::Supports,
        target: LinkTarget::CrossSet {
            set: SetName::new("scatterhoard").expect("valid set name"),
            id: matched.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let mut local = store_of(Scope::Local, &[&linking, &a_memory("gamma gamma")]);
    let recall = local
        .recall(&Question::new("alpha alpha", 20))
        .expect("recalls");

    assert!(
        recall.hits.iter().all(|hit| hit.memory.id != matched.id),
        "a memory from an unsearched set has no score on this scale and must not be returned"
    );
    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == linking.id)
        .expect("the linking memory is in scope and should be returned");
    assert_eq!(
        hit.memory.links[0].target.id(),
        matched.id,
        "the caller still sees the link, and can widen the scope or look the target up"
    );
}

/// The same bridge written as a **bare id**, which is the spelling the locating step cannot drop.
///
/// A target naming another set is located to whichever store holds that set, and a set nobody loaded
/// locates to nothing, so the edge is never built -- that is what the test below exercises now. A
/// same-set target locates to *this* store whether or not the id is in it, so the population check
/// is the only thing standing between two memories that cite one absent id and a walk that carries
/// score from one to the other through a memory nobody holds.
///
/// Written 2026-08-26, after partitioning identity by set made the recorded mutation for that check
/// stop being caught: the cross-set fixture had been covering it, and no longer reaches it.
#[test]
fn a_walk_does_not_bridge_two_memories_through_a_bare_id_nobody_holds() {
    let absent = a_memory("delta delta delta").id;
    let citing = |content: &str| {
        let mut memory = a_memory(content);
        memory.links = vec![Link {
            label: LinkLabel::Supports,
            target: LinkTarget::SameSet(absent),
        }];
        memory
    };

    // Both ends of the bridge cite an id this set does not contain.
    let matched = citing("alpha alpha alpha");
    let distant = citing("gamma gamma gamma");
    let fillers: Vec<Memory> = [
        "alpha alpha gamma",
        "alpha gamma gamma",
        "alpha gamma",
        "gamma gamma",
        "gamma",
        "beta beta beta",
        "beta gamma",
    ]
    .iter()
    .map(|content| a_memory(content))
    .collect();

    let mut held: Vec<&Memory> = vec![&matched, &distant];
    held.extend(fillers.iter());
    let mut local = store_of(Scope::Local, &held);

    // Paged, for the reason the sibling test pages: the gap cut can drop the far end of the bridge
    // before anything can be asserted about it, and the assertion is about a memory that is by
    // construction a poor match for the query. Passing the last page back walks the whole set.
    let mut seen: Vec<MemoryId> = Vec::new();
    for _ in 0..PAGE_LIMIT {
        let question = Question::new("alpha alpha", 20).already_holding(&seen);
        let recall = local.recall(&question).expect("recalls");
        if recall.hits.is_empty() {
            break;
        }
        for hit in &recall.hits {
            assert_eq!(
                hit.inherited, 0.0,
                "score crossed between two memories through an id the store does not hold"
            );
            seen.push(hit.memory.id);
        }
    }
    assert_eq!(
        seen.len(),
        held.len(),
        "every memory in scope should have been seen"
    );
}

/// The population is what stops a scoped walk, and it stops it in the case that is easy to miss: an
/// out-of-scope memory being used as a **bridge**. Two in-scope memories can both cite the same
/// memory in an unsearched set, which is a two-hop path between them through something this search
/// cannot see or score. Score may not flow along it.
///
/// Nine memories, because the walk takes five seeds and a seed never inherits: below that size every
/// memory is a walk origin and this would assert nothing. `distant` is the worst content match, so
/// the bridge is the only thing that could reach it.
#[test]
fn a_scoped_walk_does_not_route_through_an_unsearched_set() {
    let absent = a_memory("delta delta delta");
    let elsewhere = SetName::new("scatterhoard").expect("valid set name");
    let citing = |content: &str| {
        let mut memory = a_memory(content);
        memory.links = vec![Link {
            label: LinkLabel::Supports,
            target: LinkTarget::CrossSet {
                set: elsewhere.clone(),
                id: absent.id,
                keywords: Vec::new(),
                set_id: None,
            },
        }];
        memory
    };

    // Both ends of the bridge cite the memory this search will never load.
    let matched = citing("alpha alpha alpha");
    let distant = citing("gamma gamma gamma");
    let fillers: Vec<Memory> = [
        "alpha alpha gamma",
        "alpha gamma gamma",
        "alpha gamma",
        "gamma gamma",
        "gamma",
        "beta beta beta",
        "beta gamma",
    ]
    .iter()
    .map(|content| a_memory(content))
    .collect();

    let mut held: Vec<&Memory> = vec![&matched, &distant];
    held.extend(fillers.iter());
    let mut local = store_of(Scope::Local, &held);

    let mut seen: Vec<MemoryId> = Vec::new();
    for _ in 0..PAGE_LIMIT {
        let question = Question::new("alpha alpha", 20).already_holding(&seen);
        let recall = local.recall(&question).expect("recalls");
        if recall.hits.is_empty() {
            break;
        }
        for hit in &recall.hits {
            assert_eq!(
                hit.inherited, 0.0,
                "the only path between these memories runs through a set this search never \
                 looked at, so nothing may inherit"
            );
            seen.push(hit.memory.id);
        }
    }
    assert_eq!(
        seen.len(),
        held.len(),
        "every memory in scope should have been seen"
    );
}

// ---------------------------------------------------------------------------
// Keyword search across stores is one ranked list
// ---------------------------------------------------------------------------

/// `keyword-recall-multiplies-the-limit-by-the-number-of-sets`.
///
/// The semantic path merges every store into one ranked list and applies the limit to that. The
/// keyword path looped, so each store contributed up to `limit` and nothing truncated the union --
/// `limit: 1` against two stores returned two memories, in store iteration order.
fn tagged(content: &str, keywords: &[&str]) -> Memory {
    let mut memory = a_memory(content);
    memory.keywords = keywords
        .iter()
        .map(|word| Keyword::new(*word).expect("valid keyword"))
        .collect();
    memory
}

#[test]
fn keyword_search_across_stores_honours_the_limit_once() {
    let here = tagged("a local note", &["routing"]);
    let there = tagged("a project note", &["routing"]);
    let local = store_of(Scope::Local, &[&here]);
    let elsewhere = store_of(project("other"), &[&there]);

    let hits = search_keywords_across(
        &[&local, &elsewhere],
        &[Keyword::new("routing").expect("valid keyword")],
        1,
    )
    .expect("searches");

    assert_eq!(
        hits.len(),
        1,
        "the limit is a bound on the answer, not on each store's share of it: {:?}",
        hits.iter().map(|hit| &hit.scope).collect::<Vec<_>>()
    );
}

/// The second consequence, and the one easier to miss: concatenation is not ranking. `hoard_recall`
/// tells the caller results come back best first, which was true of the semantic path and false
/// here -- a two-keyword match in the second store sorted below a one-keyword match in the first.
#[test]
fn keyword_search_ranks_the_union_rather_than_concatenating_it() {
    let weak = tagged("one match, first store", &["routing"]);
    let strong = tagged("two matches, second store", &["routing", "scope"]);
    let local = store_of(Scope::Local, &[&weak]);
    let elsewhere = store_of(project("other"), &[&strong]);

    let hits = search_keywords_across(
        &[&local, &elsewhere],
        &[
            Keyword::new("routing").expect("valid keyword"),
            Keyword::new("scope").expect("valid keyword"),
        ],
        20,
    )
    .expect("searches");

    assert_eq!(hits.len(), 2, "both memories carry a requested keyword");
    assert_eq!(
        hits[0].matched_keywords,
        2,
        "the better match should lead regardless of which store it came from: {:?}",
        hits.iter()
            .map(|hit| hit.matched_keywords)
            .collect::<Vec<_>>()
    );
}

/// Equal match counts are the common case, so the tie-break has to be a rule rather than whichever
/// store happened to be iterated first -- otherwise the same query answers differently depending on
/// a `BTreeMap` ordering the caller cannot see.
#[test]
fn an_equal_match_count_breaks_the_tie_the_same_way_every_time() {
    let here = tagged("a local note", &["routing"]);
    let there = tagged("a project note", &["routing"]);
    let local = store_of(Scope::Local, &[&here]);
    let elsewhere = store_of(project("other"), &[&there]);
    let word = [Keyword::new("routing").expect("valid keyword")];

    let one = search_keywords_across(&[&local, &elsewhere], &word, 20).expect("searches");
    let other = search_keywords_across(&[&elsewhere, &local], &word, 20).expect("searches");

    let ids = |hits: &[scatterhoard::store::KeywordHit]| {
        hits.iter().map(|hit| hit.memory.id).collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&one),
        ids(&other),
        "the order of the stores handed in must not decide the order of the answer"
    );
}

// ---------------------------------------------------------------------------
// A skipped set is named, not merely counted
// ---------------------------------------------------------------------------

/// `semantic-search-available-conflates-partial-with-none`.
///
/// `recall_across` computes *completeness* and the surface reads it as *availability*, so a union
/// where one set is unsearchable returns semantically-scored hits while the response says nothing
/// was searched by meaning. The two questions are different and need different fields: **did the
/// semantic path run at all**, and **did it cover everything asked for**.
#[test]
fn a_partial_union_reports_that_it_searched_and_that_it_was_incomplete() {
    let corpus = a_corpus();
    let refs: Vec<&Memory> = corpus.iter().collect();
    let mut with_model = store_of(project("left"), &refs[..2]);
    let mut without = Store::open_in_memory(Scope::Local).expect("opens");
    without.insert(refs[2]).expect("inserts");

    let recall = recall_across(
        &mut [&mut with_model, &mut without],
        &Question::new("alpha", 5),
    )
    .expect("recalls");

    assert!(
        !recall.hits.is_empty(),
        "the store with a model still answers"
    );
    assert!(
        recall.searched,
        "the semantic path did run -- saying otherwise while returning scored hits is the defect"
    );
    assert!(
        !recall.complete,
        "and it did not cover the set that could not be searched"
    );
}

/// The naming half: which set was left out, and why, without a second call to find out.
#[test]
fn a_skipped_set_is_named_with_its_reason() {
    let corpus = a_corpus();
    let refs: Vec<&Memory> = corpus.iter().collect();
    let mut with_model = store_of(project("left"), &refs[..2]);
    let mut without = Store::open_in_memory(Scope::Local).expect("opens");
    without.insert(refs[2]).expect("inserts");

    let recall = recall_across(
        &mut [&mut with_model, &mut without],
        &Question::new("alpha", 5),
    )
    .expect("recalls");

    assert_eq!(
        recall.skipped.len(),
        1,
        "one set was left out: {:?}",
        recall.skipped
    );
    assert_eq!(recall.skipped[0].scope, Scope::Local);
    assert!(
        !recall.skipped[0].reason.is_empty(),
        "a scope with no reason is a number without its referent"
    );
}

/// Where nothing could be searched, `searched` is false and there is no ranking to misread.
#[test]
fn a_union_with_no_model_anywhere_reports_that_it_did_not_search() {
    let corpus = a_corpus();
    let refs: Vec<&Memory> = corpus.iter().collect();
    let mut first = Store::open_in_memory_and_mint_id(project("left")).expect("opens");
    first.insert(refs[0]).expect("inserts");
    let mut second = Store::open_in_memory(Scope::Local).expect("opens");
    second.insert(refs[1]).expect("inserts");

    let recall =
        recall_across(&mut [&mut first, &mut second], &Question::new("alpha", 5)).expect("recalls");

    assert!(recall.hits.is_empty());
    assert!(!recall.searched, "nothing could be searched by meaning");
    assert_eq!(recall.skipped.len(), 2, "and both sets say so");
}

// ---------------------------------------------------------------------------
// One set asserting authority over another set's memory.
//
// Raised 2026-08-21 by the human partner: projects A and B are rivals, A records something
// unflattering about B, and B hand-edits its own memory file to supersede it. Nothing stops B
// writing that edge -- a memory file is a text file its owner controls -- so the question is what
// the *reader* does with it when both sets are loaded.
// ---------------------------------------------------------------------------

/// The reproduction for `a-set-can-suppress-another-sets-memory`, unparked 2026-08-23 when the
/// design settled and the fix landed. `documents/design/cross-set-references.md` resolves that no
/// edge changes the standing of a memory in another set, in either direction, so the rival's edge
/// is ignored at resolution -- which is the half that protects a reader, since the rival's file was
/// never written by our client.
#[test]
fn a_rival_set_can_supersede_a_memory_it_does_not_hold() {
    let mut theirs = Store::open_in_memory_and_mint_id(project("rival")).expect("opens");
    theirs.set_embedder(Arc::new(MarkerEmbedder));
    let mut ours = Store::open_in_memory_and_mint_id(project("ours")).expect("opens");
    ours.set_embedder(Arc::new(MarkerEmbedder));

    // Ours says something they dislike.
    let unflattering = a_memory("alpha alpha is implemented more cleanly here");
    ours.insert(&unflattering).expect("inserts");

    // Theirs, hand-edited, claims to replace it. A cross-set SUPERSEDES pointing into our set.
    let mut rebuttal = a_memory("alpha alpha is implemented more cleanly by us actually");
    rebuttal.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: unflattering.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];
    theirs.insert(&rebuttal).expect("inserts");

    let recall =
        recall_across(&mut [&mut ours, &mut theirs], &Question::new("alpha", 10)).expect("recalls");
    let returned: Vec<_> = recall.hits.iter().map(|hit| hit.memory.id).collect();

    assert!(
        returned.contains(&unflattering.id),
        "a memory in our set was suppressed by an edge written in someone else's: returned \
         {returned:?}, and ours was {}",
        unflattering.id
    );
}

/// A memory that has yielded to a replacement in a set this machine does not hold is **returned and
/// flagged**, not withheld.
///
/// Recall used to drop it: `Ranked::Unreachable => continue`, on the reasoning that returning it
/// would pass off something known to be stale. That is *resolving* a case the software cannot
/// resolve, and it is the one place the design's own `report rather than resolve` principle was not
/// being applied. Withholding also fails in the ordinary case rather than an exotic one -- the
/// replacement's set is usually merely unloaded, not gone -- and it fails silently, leaving a
/// session with no way to know a memory exists at all.
///
/// Reporting it gives the session the move that actually helps: the memory's own `SUPERSEDED_BY`
/// link names the set and the id, so it can load that set and see the replacement for itself. The
/// flag says *do not treat this as current*; the link says *here is where the rest of the story is*.
#[test]
fn a_memory_superseded_into_an_unloaded_set_is_returned_and_flagged() {
    let mut ours = Store::open_in_memory_and_mint_id(project("ours")).expect("opens");
    ours.set_embedder(Arc::new(MarkerEmbedder));

    let mut yielded = a_memory("alpha alpha this was corrected in the other project");
    yielded.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::CrossSet {
            set: SetName::new("elsewhere").expect("valid set name"),
            id: a_memory("the replacement, in a set we do not hold").id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];
    ours.insert(&yielded).expect("inserts");

    let recall = recall_across(&mut [&mut ours], &Question::new("alpha", 10)).expect("recalls");
    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == yielded.id)
        .unwrap_or_else(|| {
            panic!(
                "a memory that yielded to an unloaded set was withheld entirely, so nothing tells \
                 the session it exists: returned {:?}",
                recall.hits.iter().map(|h| h.memory.id).collect::<Vec<_>>()
            )
        });

    assert!(
        hit.superseded_beyond_this_set,
        "returned without the flag, so it reads as current when it is known not to be"
    );
    assert!(
        !hit.in_cycle,
        "this is not a cycle, and conflating the two would send a session looking for a data error"
    );
}

/// An id that exists in **two** loaded sets must not let one set's edge speak about the other's
/// memory — even when both rows live in the same store.
///
/// This is the id-collision arm of `intentional-edits-to-a-memory-file-are-largely-unprobed`, and it
/// is the case the `edges` guard does not reach. Here a link row in our own store carries a cross-set
/// `SUPERSEDES` whose target id happens to equal one we hold, so nothing crosses a store boundary
/// and `Store::incoming` is the only thing standing between the two memories.
///
/// **Driven through the keyword path on purpose.** Semantic recall resolves validity through
/// `Neighbourhood::over`, which reads `Store::edges` — and that guard already refuses to put a
/// cross-set target into the replaced list, so it masks this one. Keyword search resolves each hit
/// through `resolve_supersession` directly, which is what actually consults `incoming`. Two guards
/// on two paths, and this is the test that tells them apart.
///
/// The stake: without the `target_set IS NULL` condition, anyone who learns a published id can
/// suppress the memory holding it on any machine that also imports their set, by colliding with it.
#[test]
fn an_edge_naming_another_set_does_not_speak_about_a_colliding_local_id() {
    let target = tagged("the memory being aimed at", &["collision"]);

    // Ours, but hand-written: it claims to supersede an id in `elsewhere` that collides with one we
    // hold. Same store, so only the set name distinguishes the two memories.
    let mut collider = tagged("the colliding claim", &["collision"]);
    collider.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::CrossSet {
            set: SetName::new("elsewhere").expect("valid set name"),
            id: target.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let ours = store_of(project("ours"), &[&target, &collider]);
    let hits = search_keywords_across(
        &[&ours],
        &[Keyword::new("collision").expect("valid keyword")],
        10,
    )
    .expect("searches");

    let hit = hits
        .iter()
        .find(|hit| hit.memory.id == target.id)
        .unwrap_or_else(|| {
            panic!(
                "a memory was suppressed by an edge that names a different set and merely collides \
                 with its id: returned {:?}",
                hits.iter().map(|h| h.memory.id).collect::<Vec<_>>()
            )
        });

    assert!(
        hit.matched.is_none(),
        "returned, but as a substitution -- the colliding edge still changed its standing"
    );
}

// ---------------------------------------------------------------------------
// Reporting the ignored edge.
//
// Ignoring a cross-set standing-changing edge protects the reader; it does not tell them a dispute
// exists. `documents/design/cross-set-references.md` resolves to report **the edge, not the
// memory** -- because a rival's memory always returned beside the one it attacks has bought a
// permanent slot in every recall that matches it.
// ---------------------------------------------------------------------------

/// A rival set's claim is **named on the hit** while changing nothing about it.
#[test]
fn a_cross_set_claim_is_reported_on_the_memory_it_targets() {
    let ours = a_memory("alpha alpha ours, and disputed");
    let mut rival = a_memory("gamma gamma the rival's rebuttal");
    rival.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: ours.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their_store = store_of(project("rival"), &[&rival]);

    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 10),
    )
    .expect("recalls");

    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == ours.id)
        .expect("our memory must still be returned -- that is the suppression fix");

    assert_eq!(
        hit.disputed_by.len(),
        1,
        "the dispute is invisible, so a session cannot judge it: {:?}",
        hit.disputed_by
    );
    let claim = &hit.disputed_by[0];
    assert_eq!(claim.set, SetName::new("rival").expect("valid set name"));
    assert_eq!(claim.source, rival.id);
    assert_eq!(claim.label, LinkLabel::Supersedes);
    assert!(
        hit.matched.is_none() && !hit.superseded_beyond_this_set,
        "reporting the edge must not also change the standing it was refused the power to change"
    );
}

/// **The edge, not the memory.** The rival's memory does not match the query, and reporting its
/// claim must not smuggle it into the results — otherwise attacking a memory buys a permanent slot
/// in every recall that memory matches, which is weaker than suppression and still a free channel.
#[test]
fn reporting_a_claim_does_not_rank_the_memory_making_it() {
    let ours = a_memory("alpha alpha ours, and disputed");
    let mut rival = a_memory("gamma gamma nothing to do with the query");
    rival.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: ours.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their_store = store_of(project("rival"), &[&rival]);

    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 1),
    )
    .expect("recalls");

    let returned: Vec<_> = recall.hits.iter().map(|hit| hit.memory.id).collect();
    assert!(returned.contains(&ours.id));
    assert!(
        !returned.contains(&rival.id),
        "the disputing memory was ranked as a result of disputing: returned {returned:?}"
    );
    // The claim is still reported -- naming it is what the caller needs, and fetching it is what
    // they can choose to do next.
    assert_eq!(recall.hits[0].disputed_by.len(), 1);
}

/// All three standing-changing labels are reported, and `SUPERSEDED_BY` is not one of them: an edge
/// naming *our* memory as its own replacement concedes standing rather than claiming it.
#[test]
fn only_standing_changing_labels_count_as_a_dispute() {
    let ours = a_memory("alpha alpha ours");
    let claim = |label: LinkLabel, content: &str| {
        let mut memory = a_memory(content);
        memory.links = vec![Link {
            label,
            target: LinkTarget::CrossSet {
                set: SetName::new("ours").expect("valid set name"),
                id: ours.id,
                keywords: Vec::new(),
                set_id: None,
            },
        }];
        memory
    };

    let disputes = [
        claim(LinkLabel::Supersedes, "gamma one"),
        claim(LinkLabel::ExceptionTo, "gamma two"),
        claim(LinkLabel::Contradicts, "gamma three"),
    ];
    // Neither of these is a dispute: the first concedes, the second merely associates.
    let quiet = [
        claim(LinkLabel::SupersededBy, "gamma four"),
        claim(LinkLabel::RelatedTo, "gamma five"),
    ];

    let all: Vec<&Memory> = disputes.iter().chain(quiet.iter()).collect();
    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their_store = store_of(project("rival"), &all);

    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 10),
    )
    .expect("recalls");
    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == ours.id)
        .expect("returned");

    let mut labels: Vec<LinkLabel> = hit
        .disputed_by
        .iter()
        .map(|claim| claim.label.clone())
        .collect();
    labels.sort_by_key(|label| format!("{label:?}"));
    assert_eq!(
        labels,
        vec![
            LinkLabel::Contradicts,
            LinkLabel::ExceptionTo,
            LinkLabel::Supersedes
        ],
        "got {:?}",
        hit.disputed_by
    );
}

/// An undisputed memory reports nothing. Worth pinning: a field that is always present teaches a
/// reader to skim past it, and the whole value here is that a dispute is unusual.
#[test]
fn an_undisputed_memory_reports_no_claims() {
    let ours = a_memory("alpha alpha nobody disputes this");
    let theirs = a_memory("alpha alpha an unrelated memory elsewhere");
    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their_store = store_of(project("rival"), &[&theirs]);

    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 10),
    )
    .expect("recalls");

    for hit in &recall.hits {
        assert!(
            hit.disputed_by.is_empty(),
            "{} reported a dispute nobody made: {:?}",
            hit.memory.id,
            hit.disputed_by
        );
    }
}

/// A link inside our own store that spells our own set name is **not** a dispute: nothing crossed a
/// set boundary, so there is no stranger to report.
///
/// This pins a judgement rather than an obvious rule. Such a row is a same-set link written the long
/// way, and resolution currently ignores it — because the guards key on `target_set IS NULL` rather
/// than on comparing set names, which is filed as `a-same-set-link-spelled-with-its-set-name-is-inert`
/// in `TRACKING.md`. Reporting it *here* would be the wrong repair: it would tell a caller their own
/// set is attacking them, which is both alarming and untrue.
#[test]
fn a_link_naming_our_own_set_is_not_reported_as_a_dispute() {
    let ours = a_memory("alpha alpha ours");
    let mut long_way = a_memory("alpha alpha ours too, linked the long way");
    long_way.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: ours.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let mut our_store = store_of(project("ours"), &[&ours, &long_way]);
    let recall =
        recall_across(&mut [&mut our_store], &Question::new("alpha", 10)).expect("recalls");

    for hit in &recall.hits {
        assert!(
            hit.disputed_by.is_empty(),
            "{} was reported as disputed by its own set: {:?}",
            hit.memory.id,
            hit.disputed_by
        );
    }
}

// ---------------------------------------------------------------------------
// One spelling, canonicalised on the way in.
//
// `a-same-set-link-spelled-with-its-set-name-is-inert`: a link whose `target_set` names its own set
// is a same-set link written the long way, and the resolution guards -- which key on `target_set IS
// NULL` -- ignored it. Fixed by normalising at the write rather than by teaching every lookup to
// compare names, so the ambiguity never reaches the table.
// ---------------------------------------------------------------------------

/// The long spelling **works**: a supersession written with the set's own name replaces its target,
/// exactly as the compact spelling does.
#[test]
fn a_supersedes_naming_its_own_set_still_replaces_its_target() {
    let old = a_memory("alpha alpha the original claim");
    let mut new = a_memory("alpha alpha the corrected claim");
    new.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: old.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let mut store = store_of(project("ours"), &[&old, &new]);
    let recall = store.recall(&Question::new("alpha", 10)).expect("recalls");
    let returned: Vec<_> = recall.hits.iter().map(|hit| hit.memory.id).collect();

    assert!(
        !returned.contains(&old.id),
        "the superseded memory was returned as current: the long spelling is inert"
    );
    assert!(returned.contains(&new.id), "got {returned:?}");
}

/// And it is normalised **on disk**, not merely tolerated at each lookup. Reading the memory back
/// gives the compact form, so the store holds one spelling and every lookup over it agrees by
/// construction rather than by each one remembering to compare names.
#[test]
fn a_link_naming_its_own_set_is_stored_in_the_compact_form() {
    let target = a_memory("alpha the target");
    let mut source = a_memory("alpha the source");
    source.links = vec![Link {
        label: LinkLabel::RelatedTo,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: target.id,
            keywords: vec![Keyword::new("ignored").expect("valid keyword")],
            set_id: None,
        },
    }];

    let store = store_of(project("ours"), &[&target, &source]);
    let found = store.lookup(source.id).expect("looks up").expect("found");

    assert_eq!(
        found.memory.links[0].target,
        LinkTarget::SameSet(target.id),
        "stored as written rather than canonicalised, so two spellings of one fact reach the table"
    );
}

/// A genuinely foreign target keeps its set name. The normalisation is about one specific
/// redundancy, and widening it would silently turn every cross-set link into a same-set one.
#[test]
fn a_genuinely_cross_set_link_keeps_its_set_name() {
    let mut source = a_memory("alpha the source");
    let elsewhere = MemoryId::generate();
    source.links = vec![Link {
        label: LinkLabel::RelatedTo,
        target: LinkTarget::CrossSet {
            set: SetName::new("elsewhere").expect("valid set name"),
            id: elsewhere,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let store = store_of(project("ours"), &[&source]);
    let found = store.lookup(source.id).expect("looks up").expect("found");

    assert_eq!(
        found.memory.links[0].target.set().map(SetName::as_str),
        Some("elsewhere"),
        "a foreign link lost its set name, which would make it resolve against the wrong set"
    );
}

/// The canonicalisation must hold on the **import** path, not only on `insert`.
///
/// This is the path that matters for a hand-merge: someone folds another project's memories into
/// this set's file, and every link those memories carried into *this* set is spelled the long way,
/// because from where they were written it really was another set. Import is also the only way such
/// a record can arrive at all, since the write door refuses to author one.
#[test]
fn import_canonicalises_a_link_naming_the_importing_set() {
    let old = a_memory("alpha the original claim");
    let mut new = a_memory("alpha the correction, written from elsewhere");
    new.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: old.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let mut store = Store::open_in_memory_and_mint_id(project("ours")).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    store
        .import(&MemorySet {
            scope: project("ours"),
            memories: vec![old.clone(), new.clone()],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("imports");

    assert_eq!(
        store.resolve_supersession(old.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![new.id],
            unresolved: false,
            forgotten: false,
        },
        "an imported correction spelled the long way is inert, which is the hand-merge case"
    );
}

/// The suppression fix must key on **who holds the memory**, not on how the link is spelled.
///
/// The first repair filtered on `target_set`, which an attacker simply omits: a bare id is the
/// easier thing to hand-write, and it bypassed the guard completely. The structural fault is that
/// `Neighbourhood::over` asked *each* store to resolve every id its own edges called replaced --
/// including ids that store does not hold -- and merged every answer into one validity map shared
/// by the whole recall. A store may only speak about its own memories.
#[test]
fn a_rival_set_cannot_supersede_a_memory_it_does_not_hold_with_a_bare_id() {
    let ours = a_memory("alpha alpha ours, and targeted");
    let mut rival = a_memory("alpha alpha the rival's rebuttal");
    // No `target_set` at all -- a same-set link whose id happens to live in someone else's set.
    rival.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(ours.id),
    }];

    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their_store = store_of(project("rival"), &[&rival]);

    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 10),
    )
    .expect("recalls");
    let returned: Vec<_> = recall.hits.iter().map(|hit| hit.memory.id).collect();

    assert!(
        returned.contains(&ours.id),
        "a memory was suppressed by an edge in a set that does not hold it, written with a bare \
         id: returned {returned:?}, ours was {}",
        ours.id
    );
}

// ---------------------------------------------------------------------------
// The hand-edited-file sweep.
//
// `intentional-edits-to-a-memory-file-are-largely-unprobed` filed the neighbours of the confirmed
// suppression defect and asked for each to be driven from a hand-built file: a cross-set
// `SUPERSEDED_BY`, an injected cycle, `EXCEPTION_TO` and `CONTRADICTS`, and an id shared by two
// loaded sets. Everything below is one of those arms. The passing tests say an arm is harmless; the
// parked ones are reproductions, each naming the defect it pins.
//
// A memory file is a text file its owner controls, so none of these can be prevented at the write
// surface. What is being probed throughout is what the *reader* does with the file.
// ---------------------------------------------------------------------------

/// Two loaded sets holding one id must not collapse into one memory.
///
/// **Parked, not passing.** `two-loaded-sets-sharing-one-id-collapse-to-one-memory` in `TRACKING.md`.
/// `recall_across` keys its candidates by `MemoryId`, so one id is one candidate however many sets
/// hold it: the memory returned is whichever store was scanned last, carrying that store's content
/// and scope, and the other set's memory is not returned at all. Nothing in the result says a second
/// memory existed.
///
/// This is the ranking arm the issue named as entirely unexplored, and it is a suppression route the
/// `target_set` and `holds_record` guards do not reach — those stop one store's *edge* speaking about
/// another store's memory, and this needs no edge at all.
#[test]
fn two_sets_holding_one_id_both_reach_the_reader() {
    let ours = a_memory("alpha alpha ours, under a shared id");
    let mut theirs = a_memory("alpha alpha theirs, under the same id");
    theirs.id = ours.id;

    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their_store = store_of(project("rival"), &[&theirs]);

    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 10),
    )
    .expect("recalls");

    let scopes: Vec<&Scope> = recall.hits.iter().map(|hit| &hit.scope).collect();
    assert_eq!(
        recall.hits.len(),
        2,
        "one of two memories sharing an id was dropped without a word; what came back was from \
         {scopes:?}"
    );
}

/// A colliding id must not let another set's `SUPERSEDED_BY` speak about our memory.
///
/// **Parked, not passing.** The `SUPERSEDED_BY` arm of the sweep, and it turns out to depend on the
/// collision rather than on the label: an authored `SUPERSEDED_BY` names its own source, so it can
/// only concede its holder's standing — until two sets share an id, at which point the holder and
/// the victim are one identity to `Neighbourhood`'s validity map.
///
/// What comes back today is the rival's record, flagged `superseded_beyond_this_set`, and ours not
/// at all. So this fails first on ours being absent, which is the test above's defect; the flag
/// assertion is the second half, live once a fix returns both memories. Kept as its own test because
/// a fix that returned ours while still resolving its standing from another set's file would pass
/// the test above and be wrong here.
#[test]
fn a_colliding_id_does_not_carry_a_foreign_sets_concession() {
    let ours = a_memory("alpha alpha ours, current and uncontested");
    let mut theirs = a_memory("alpha alpha theirs, conceding to something nobody here holds");
    theirs.id = ours.id;
    theirs.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::CrossSet {
            set: SetName::new("elsewhere").expect("valid set name"),
            id: a_memory("a replacement in a set we do not hold").id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their_store = store_of(project("rival"), &[&theirs]);

    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 10),
    )
    .expect("recalls");

    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.scope == project("ours"))
        .expect("our memory must be returned at all");
    assert!(
        !hit.superseded_beyond_this_set,
        "our memory was flagged obsolete by a concession written in another set's file, which \
         merely shares its id"
    );
}

/// A cycle in another set's file must not become a cycle in ours.
///
/// **Parked, not passing.** The cycle-injection arm. `check_no_cycle` guards the write and nothing
/// guards a file, so a hand-built file can hold two memories superseding each other — that much is
/// already known and is the rival's own business. What the collision adds is a victim: with one of
/// the pair wearing our id, `Neighbourhood::over` resolves the rival's loop and files the answer
/// under an id we also hold.
///
/// **The injected cycle turns out to be the milder half.** Today ours is not returned at all — the
/// collision drops it, as above — and the rival's two loop members come back flagged `in_cycle`. The
/// assertion here is what must hold once ours is returned again: a memory whose own history is one
/// uncontested record must not be reported as having a broken one because a stranger's loop shares
/// its id.
///
/// Reached only through a collision. Without one, a foreign file cannot inject a cycle: `edges`
/// drops a cross-set `SUPERSEDES` from `replaced`, `Store::incoming` refuses cross-set rows, and
/// `Neighbourhood::over` asks a store to resolve only ids it holds. Those three are what a fix here
/// must not have to duplicate.
#[test]
fn a_colliding_id_does_not_put_our_memory_in_a_foreign_cycle() {
    let ours = a_memory("alpha alpha ours, whose history is a single record");

    // The rival's file: two memories replacing each other, one of them wearing our id. Built by
    // import, which is the path a file takes and the one that runs no cycle check.
    let mut theirs = a_memory("alpha alpha theirs, one half of a loop");
    theirs.id = ours.id;
    let mut partner = a_memory("alpha alpha theirs, the other half of the loop");
    theirs.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(partner.id),
    }];
    partner.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(theirs.id),
    }];

    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their_store = Store::open_in_memory_and_mint_id(project("rival")).expect("opens");
    their_store.set_embedder(Arc::new(MarkerEmbedder));
    their_store
        .import(&MemorySet {
            scope: project("rival"),
            memories: vec![theirs, partner],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("imports");

    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 10),
    )
    .expect("recalls");

    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.scope == project("ours"))
        .expect("our memory must be returned at all");
    assert!(
        !hit.in_cycle,
        "our memory was reported as having a broken history because another set's loop shares its \
         id, which sends a session looking for a data error it does not have"
    );
}

/// **The keyword path is unharmed by a collision**, and that is worth pinning rather than assuming.
///
/// It resolves and returns each store's hits separately — `search_keywords_across` concatenates
/// per-store results rather than keying them by id — so two memories sharing an id come back as two
/// memories, each with its own set, content and standing. The semantic path's collapse is therefore
/// a property of `Neighbourhood`'s shared validity map and of ranking by identity, not of the store
/// layer, which is what says where a fix belongs.
#[test]
fn a_colliding_id_leaves_the_keyword_path_alone() {
    let ours = tagged("ours, under a shared id", &["collision"]);
    let mut theirs = tagged("theirs, under the same id", &["collision"]);
    theirs.id = ours.id;

    let our_store = store_of(project("ours"), &[&ours]);
    let their_store = store_of(project("rival"), &[&theirs]);

    let hits = search_keywords_across(
        &[&our_store, &their_store],
        &[Keyword::new("collision").expect("valid keyword")],
        10,
    )
    .expect("searches");

    let scopes: Vec<&Scope> = hits.iter().map(|hit| &hit.scope).collect();
    assert_eq!(
        hits.len(),
        2,
        "a memory was lost to an id it shares with another set: returned {scopes:?}"
    );
    assert!(
        hits.iter().all(|hit| hit.matched.is_none()),
        "neither memory is a substitution for the other, and saying so is the point"
    );
}

/// **A cross-set `EXCEPTION_TO` or `CONTRADICTS` changes nothing about its target's standing**, which
/// is the arm of the sweep that turns out to be already safe.
///
/// The design names all three of `SUPERSEDES`, `EXCEPTION_TO` and `CONTRADICTS` as standing-changing
/// *when traversal acts on them*; resolution acts on supersession alone, so the other two have no
/// standing effect anywhere today, across a set boundary or within one. That makes them harmless in
/// the sense this sweep asked about, and the reason is worth recording: they are safe because
/// nothing implements them, not because a guard refuses them.
#[test]
fn a_cross_set_dispute_leaves_its_targets_standing_alone() {
    let ours = a_memory("alpha alpha ours, and disputed twice over");
    let claim = |label: LinkLabel, content: &str| {
        let mut memory = a_memory(content);
        memory.links = vec![Link {
            label,
            target: LinkTarget::CrossSet {
                set: SetName::new("ours").expect("valid set name"),
                id: ours.id,
                keywords: Vec::new(),
                set_id: None,
            },
        }];
        memory
    };
    let excepted = claim(LinkLabel::ExceptionTo, "gamma gamma the exception");
    let contradicted = claim(LinkLabel::Contradicts, "gamma gamma the contradiction");

    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their_store = store_of(project("rival"), &[&excepted, &contradicted]);

    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 10),
    )
    .expect("recalls");
    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == ours.id)
        .expect("returned");

    assert!(
        hit.matched.is_none() && !hit.in_cycle && !hit.superseded_beyond_this_set,
        "a disputing label from another set changed the standing it is only allowed to report: \
         matched {:?}, in_cycle {}, beyond {}",
        hit.matched,
        hit.in_cycle,
        hit.superseded_beyond_this_set
    );
    assert_eq!(hit.disputed_by.len(), 2, "and both claims are reported");
}

/// A disputing edge from another set must not carry the disputing memory into the results.
///
/// **Parked, not passing.** `a-cross-set-dispute-carries-score-to-the-memory-making-it` in
/// `TRACKING.md`. `SUPERSEDES` is not traversable, so the design's rule — *what is reported is the
/// edge, not the other memory* — holds for it. `EXCEPTION_TO` and `CONTRADICTS` are traversable, so
/// writing one at a memory in someone else's set puts the writer one hop from it and the walk pays
/// out accordingly. The claimant buys the slot the design declined to give it.
///
/// The control is the whole test: `bystander` has the same content and no edge, so the difference
/// between the two inherited scores is the edge and nothing else. The crowd exists only to spend the
/// five seed slots, since a seed never inherits — without it the disputer is its own seed and the
/// effect is invisible.
#[test]
fn a_cross_set_dispute_does_not_carry_score_to_the_memory_making_it() {
    let ours = a_memory("alpha alpha alpha alpha alpha alpha the one excepted against");

    let mut disputer = a_memory("gamma gamma gamma gamma gamma the disputing memory");
    disputer.links = vec![Link {
        label: LinkLabel::ExceptionTo,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: ours.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];
    let bystander = a_memory("gamma gamma gamma gamma gamma the control, no edge");

    let crowd: Vec<Memory> = (0..4)
        .map(|n| a_memory(&format!("alpha alpha alpha gamma gamma gamma crowd {n}")))
        .collect();

    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their: Vec<&Memory> = vec![&disputer, &bystander];
    their.extend(crowd.iter());
    let mut their_store = store_of(project("rival"), &their);

    let known: Vec<MemoryId> = crowd.iter().map(|memory| memory.id).collect();
    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 20).already_holding(&known),
    )
    .expect("recalls");

    let carried = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == disputer.id)
        .map_or(0.0, |hit| hit.inherited);
    assert!(
        carried.abs() < f32::EPSILON,
        "a memory in another set was lifted by the edge it wrote at ours, inheriting {carried}"
    );
}

/// The keyword path must answer the same way as recall about a concession to a loaded set.
///
/// `superseded-beyond-this-set-is-reported-when-the-set-is-loaded`. Keyword search resolves validity
/// exactly as semantic recall does -- a superseded memory is replaced by whatever corrected it --
/// so a concession the one path honours and the other reports as out of reach would be two answers
/// to one question, which is the shape of `lookup-and-recall-disagree-on-shape`.
#[test]
fn the_keyword_path_resolves_a_concession_to_a_loaded_set() {
    let ours = tagged("ours, the named replacement", &["conceded"]);
    let mut theirs = tagged("theirs, conceding to ours by name", &["conceded"]);
    theirs.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: ours.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let our_store = store_of(project("ours"), &[&ours]);
    let their_store = store_of(project("rival"), &[&theirs]);

    let hits = search_keywords_across(
        &[&our_store, &their_store],
        &[Keyword::new("conceded").expect("valid keyword")],
        10,
    )
    .expect("searches");

    assert!(
        !hits.iter().any(|hit| hit.superseded_beyond_this_set),
        "the keyword path reported a replacement as out of reach while its set was searched in the \
         same call"
    );
    assert!(
        !hits.iter().any(|hit| hit.memory.id == theirs.id),
        "the conceding memory was returned as itself, where recall substitutes its replacement"
    );
    assert_eq!(
        hits.iter().filter(|hit| hit.memory.id == ours.id).count(),
        1,
        "the replacement should appear once, not once per store that reached it"
    );
}

/// **A replacement the caller holds stays out, even when only a substitution reaches it.** Held ids
/// are skipped by each store before loading, which covers every hit returned as itself; a hit
/// substituted by another set's replacement is only known to be held once the substitution is made,
/// so the merge asks again. Here the replacement does not carry the keyword, so the substitution is
/// the only way it arrives.
#[test]
fn a_substituted_keyword_hit_the_caller_holds_is_left_out() {
    let ours = tagged("ours, the named replacement", &["unrelated"]);
    let mut theirs = tagged("theirs, conceding to ours by name", &["conceded"]);
    theirs.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: ours.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];
    let our_store = store_of(project("ours"), &[&ours]);
    let their_store = store_of(project("rival"), &[&theirs]);
    let query = [Keyword::new("conceded").expect("valid keyword")];

    let reached =
        search_keywords_across(&[&our_store, &their_store], &query, 10).expect("searches");
    assert_eq!(
        reached.iter().map(|hit| hit.memory.id).collect::<Vec<_>>(),
        vec![ours.id],
        "the control: the replacement arrives by substitution"
    );

    let held = scatterhoard::store::search_keywords_across_holding(
        &[&our_store, &their_store],
        &query,
        10,
        &[ours.id],
    )
    .expect("searches");
    assert!(
        held.is_empty(),
        "a replacement the caller holds came back by substitution: {:?}",
        held.iter().map(|hit| hit.memory.id).collect::<Vec<_>>()
    );
}

/// A `SUPERSEDED_BY` naming an id no set holds must leave its memory findable.
///
/// **Parked, not passing.** `a-superseded-by-naming-an-absent-id-hides-its-memory` in `TRACKING.md`.
/// Found by this sweep rather than named by it: the same-set spelling is what
/// [Cross-Set References](../documents/design/cross-set-references.md) calls the commonest malformed
/// reference — a cross-set link written without its set name — and it is the one spelling that makes
/// the memory disappear. Written with the set name, the memory comes back flagged
/// `superseded_beyond_this_set`; written without it, `resolve_supersession` reports a tip that
/// resolved, no store can fetch that tip, and both search paths drop the hit without a word.
///
/// No collision and no second set: a file its own author edited is enough.
#[test]
fn a_memory_yielding_to_an_id_nobody_holds_is_still_findable() {
    let stranger = a_memory("the replacement, which no set holds").id;
    let mut yielded = tagged("alpha alpha this yielded to a bare id", &["dangling"]);
    yielded.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::SameSet(stranger),
    }];
    let neighbour = tagged("alpha alpha an ordinary memory beside it", &["dangling"]);

    let mut store = Store::open_in_memory_and_mint_id(project("ours")).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    store
        .import(&MemorySet {
            scope: project("ours"),
            memories: vec![yielded.clone(), neighbour],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("imports");

    let recall = recall_across(&mut [&mut store], &Question::new("alpha", 10)).expect("recalls");
    let found: Vec<MemoryId> = recall.hits.iter().map(|hit| hit.memory.id).collect();
    assert!(
        found.contains(&yielded.id),
        "a memory vanished from recall because its own file names a replacement nobody holds: \
         returned {found:?}, and it was {}",
        yielded.id
    );

    let hits = search_keywords_across(
        &[&store],
        &[Keyword::new("dangling").expect("valid keyword")],
        10,
    )
    .expect("searches");
    let found: Vec<MemoryId> = hits.iter().map(|hit| hit.memory.id).collect();
    assert!(
        found.contains(&yielded.id),
        "and it is gone from the keyword path too, which is the path that needs no model: returned \
         {found:?}"
    );
}

/// A memory conceding to a replacement in a set that **is** loaded resolves like any other
/// concession: the replacement is returned in its place.
///
/// `superseded-beyond-this-set-is-reported-when-the-set-is-loaded` in `TRACKING.md`. The flag's
/// documented meaning is *a replacement in a set not held here*, and the remedy it exists to enable
/// -- load that set and fetch the target -- is the one thing a reader who already holds the set
/// cannot do.
///
/// **Substitution rather than an unset flag**, which is what the entry asks for and what the design
/// supports: an authored `SUPERSEDED_BY` is the memory's own claim about itself, so honouring it
/// changes the conceding memory's own standing and reaches into nobody else's. Leaving it returned
/// and merely unflagged would say one of the two is stale without saying which replaced which.
#[test]
fn a_concession_to_a_loaded_set_resolves_to_the_replacement() {
    // The stale memory is the nearer match on purpose: this is the case substitution exists for,
    // where the query matched the wording of the claim that was corrected and the reader should be
    // handed the correction. It also fixes the scan order, and with it whether `matched` is
    // populated at all -- a tip that outranks the memory it replaced is entered as itself first and
    // carries no `matched`, which is ordinary and is not what this test is about.
    //
    // **The counts read backwards and are right.** `MarkerEmbedder` scores against the centroid of
    // the searched sets, and with two memories that is their midpoint, so the query is nearer the
    // memory whose alpha count is *unlike* it. The same shape as the fixture behind
    // `a_query_matching_a_superseded_memory_returns_its_replacement` in `tests/recall.rs`.
    let ours = a_memory("alpha alpha alpha ours, the named replacement");
    let mut theirs = a_memory("alpha theirs, conceding to ours by name");
    theirs.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::CrossSet {
            set: SetName::new("ours").expect("valid set name"),
            id: ours.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let mut our_store = store_of(project("ours"), &[&ours]);
    let mut their_store = store_of(project("rival"), &[&theirs]);

    let recall = recall_across(
        &mut [&mut our_store, &mut their_store],
        &Question::new("alpha", 10),
    )
    .expect("recalls");

    assert!(
        !recall.hits.iter().any(|hit| hit.memory.id == theirs.id),
        "the conceding memory was returned as itself, though the memory it yielded to was searched \
         in the same call"
    );

    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == ours.id)
        .expect("the replacement is returned");
    assert_eq!(
        hit.matched,
        Some(theirs.id),
        "the substitution happened without saying which memory it stood in for"
    );
    assert!(
        !hit.superseded_beyond_this_set,
        "reported as replaced from beyond this set, while that set was searched in the same call"
    );
}

// ---------------------------------------------------------------------------
// A cross-set link resolves by set identity, not by the set's name
// ---------------------------------------------------------------------------

/// **A link naming a set carries that set's identity, and the identity is what should find it.**
///
/// A set's local name can change and two sets can share one — which is why a link records the
/// target's `set_id` when the authoring machine knows it. Until now nothing read that field:
/// resolution located the holder by comparing names, so a link whose name had gone stale reached
/// nothing while its identity sat unused in the same row.
#[test]
fn a_cross_set_link_finds_its_target_by_identity_when_the_name_has_gone_stale() {
    let replacement = a_memory("alpha alpha the correction, in the set that was renamed");
    let mut holder = store_of(project("known-by-another-name-now"), &[&replacement]);
    let identity = holder.mint_set_id().expect("mints an identity");

    let mut yielded = a_memory("alpha alpha this was corrected elsewhere");
    yielded.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::CrossSet {
            // The name the set had when the link was written, and no longer has.
            set: SetName::new("the-old-name").expect("valid set name"),
            id: replacement.id,
            keywords: Vec::new(),
            set_id: Some(identity),
        },
    }];
    let mut ours = store_of(project("ours"), &[&yielded]);

    let recall =
        recall_across(&mut [&mut ours, &mut holder], &Question::new("alpha", 10)).expect("recalls");
    let returned: Vec<MemoryId> = recall.hits.iter().map(|hit| hit.memory.id).collect();

    // Resolving succeeded, so the superseded memory is substituted by its replacement rather than
    // returned flagged. That substitution *is* the observable: it can only happen if the link's
    // identity found the holder, because its name matches no loaded set.
    assert!(
        returned.contains(&replacement.id),
        "the replacement should stand in for the memory that conceded to it: {returned:?}"
    );
    assert!(
        !returned.contains(&yielded.id),
        "and the superseded memory should not also come back: {returned:?}"
    );
}

/// **And an identity is not overridden by a name that happens to match.**
///
/// This is the ambiguity `set_id` exists to remove. A link carrying one set's identity must not be
/// resolved against a different set that merely shares the name it was written with — that is the
/// wrong-set case, which is worse than not resolving at all, because it attaches a concession to a
/// memory nobody conceded to.
#[test]
fn a_cross_set_link_does_not_resolve_against_a_set_that_only_shares_the_name() {
    let impostor = a_memory("alpha alpha a memory in a different set that shares the name");
    let mut namesake = store_of(project("shared-name"), &[&impostor]);
    namesake.mint_set_id().expect("mints its own identity");

    let elsewhere = Store::open_in_memory(project("somewhere-else")).expect("opens");
    let other_identity = elsewhere.mint_set_id().expect("mints an identity");

    let mut yielded = a_memory("alpha alpha this yielded to a set we do not hold");
    yielded.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::CrossSet {
            set: SetName::new("shared-name").expect("valid set name"),
            id: impostor.id,
            keywords: Vec::new(),
            // The identity of a set that is *not* the loaded namesake.
            set_id: Some(other_identity),
        },
    }];
    let mut ours = store_of(project("ours"), &[&yielded]);

    let recall = recall_across(&mut [&mut ours, &mut namesake], &Question::new("alpha", 10))
        .expect("recalls");
    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == yielded.id)
        .expect("our memory must be returned");

    assert!(
        hit.superseded_beyond_this_set,
        "the named set is loaded but is not the set this link means, so the replacement is genuinely \
         out of reach and must be reported as such rather than taken from the namesake"
    );
}

/// **No link reaches the personal set, by identity any more than by name.**
/// `a-link-can-reach-the-local-set-by-its-identity`: `local` has held an identity since 2026-08-31,
/// and identity resolution compared a link's `set_id` against every loaded store, so a shared
/// memory's `superseded_by` carrying this machine's `local` identity conceded to a personal memory,
/// and recall substituted it. No authoring path writes that identity; a crafted or hand-edited file
/// is the way in, and the link below is what such a file carries.
#[test]
fn a_concession_carrying_the_local_sets_identity_does_not_resolve_into_it() {
    let personal = a_memory("alpha alpha a personal memory the shared one must not reach");
    let mut local = store_of(Scope::Local, &[&personal]);
    let local_identity = local
        .set_id()
        .expect("reads")
        .expect("the local set has an identity");

    let mut yielded = a_memory("alpha alpha a shared memory from a crafted file");
    yielded.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::CrossSet {
            // Any name: `local` is not a set name, so a crafted file has to spell something else.
            set: SetName::new("anything").expect("valid set name"),
            id: personal.id,
            keywords: Vec::new(),
            set_id: Some(local_identity),
        },
    }];
    let mut ours = store_of(project("ours"), &[&yielded]);

    let recall =
        recall_across(&mut [&mut ours, &mut local], &Question::new("alpha", 10)).expect("recalls");
    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == yielded.id)
        .expect("the shared memory must come back as itself, not substituted by a personal one");
    assert_eq!(hit.matched, None, "it was substituted: {hit:?}");
    assert!(
        hit.superseded_beyond_this_set,
        "the concession names no set a link can reach, so it is out of reach and must say so"
    );
}

/// The walk is the second path the same resolution fed: a shared memory's edge carrying the local
/// set's identity made a personal memory its neighbour. The control is the same edge carrying a
/// shared set's identity, which the walk must cross, so the test cannot pass by the edge being
/// inert for some other reason.
#[test]
fn a_walk_does_not_reach_the_local_set_by_its_identity() {
    // The same population as `an_unscoped_recall_walks_across_a_set_boundary`: nine memories, so the
    // walk has seeds to spare and `distant` is reachable by nothing but the edge.
    let contents = [
        "alpha alpha alpha",
        "alpha alpha gamma",
        "alpha gamma gamma",
        "alpha gamma",
        "gamma gamma",
        "gamma",
        "beta beta beta",
        "beta gamma",
    ];
    let matched = a_memory(contents[0]);
    let fillers: Vec<Memory> = contents[1..].iter().map(|text| a_memory(text)).collect();
    let distant = a_memory("gamma gamma gamma");

    let inherited_by_distant = |target_scope: Scope| {
        let target_memories: Vec<&Memory> =
            std::iter::once(&matched).chain(&fillers[..3]).collect();
        let mut target = store_of(target_scope, &target_memories);
        let identity = target.set_id().expect("reads").expect("has an identity");

        let mut linking = distant.clone();
        linking.links = vec![Link {
            label: LinkLabel::Supports,
            target: LinkTarget::CrossSet {
                set: SetName::new("anything").expect("valid set name"),
                id: matched.id,
                keywords: Vec::new(),
                set_id: Some(identity),
            },
        }];
        let shared_memories: Vec<&Memory> =
            std::iter::once(&linking).chain(&fillers[3..]).collect();
        let mut shared = store_of(project("scatterhoard"), &shared_memories);

        let mut held: Vec<MemoryId> = Vec::new();
        for _ in 0..PAGE_LIMIT {
            let asked = Question {
                already_known: &held,
                ..Question::new("alpha alpha", 20)
            };
            let recall = recall_across(&mut [&mut target, &mut shared], &asked).expect("recalls");
            if recall.hits.is_empty() {
                return None;
            }
            for hit in &recall.hits {
                if hit.memory.id == distant.id {
                    return Some(hit.inherited);
                }
                held.push(hit.memory.id);
            }
        }
        panic!("paging did not end");
    };

    assert!(
        inherited_by_distant(project("target")).is_some_and(|inherited| inherited > 0.0),
        "the control: an edge carrying a shared set's identity is walked"
    );
    assert_eq!(
        inherited_by_distant(Scope::Local),
        Some(0.0),
        "the same edge carrying the local set's identity reached a personal memory"
    );
}

/// **Undoing a forget by superseding the tombstone carries its inbound links forward.** Asked
/// 2026-09-28 under `merge-without-a-recorded-ancestor`: if a forget is one-way at import, the
/// deliberate undo is a new memory superseding the forgotten id -- git's revert, a new event rather
/// than a return to an old state -- and that is only as good as the old id's in-links following it.
/// Three cases on one population: the link reaches a memory it names directly (the control), a
/// forgotten id with nothing superseding it (reaches nothing), and a forgotten id that a replacement
/// supersedes (the question).
#[test]
fn a_memory_superseding_a_forgotten_id_receives_its_in_links() {
    let contents = [
        "alpha alpha gamma",
        "alpha gamma gamma",
        "alpha gamma",
        "gamma gamma",
        "gamma",
        "beta beta beta",
        "beta gamma",
    ];
    let fillers: Vec<Memory> = contents.iter().map(|text| a_memory(text)).collect();
    let forgotten = a_memory("alpha alpha alpha, forgotten");
    // The same memory in every case; only whether it supersedes the forgotten id differs.
    let replacement = a_memory("alpha alpha alpha");

    // `undo`: what supersedes the forgotten id, if anything. `direct`: link straight to the
    // replacement instead, as the control.
    let inherited_by_distant = |undo: bool, direct: bool| {
        let mut distant = a_memory("gamma gamma gamma");
        distant.links = vec![Link {
            label: LinkLabel::Supports,
            target: LinkTarget::SameSet(if direct { replacement.id } else { forgotten.id }),
        }];
        let mut store = store_of(project("scatterhoard"), &[&forgotten, &distant]);
        for filler in &fillers {
            store.insert(filler).expect("inserts");
        }
        if !direct {
            store
                .forget(forgotten.id, MemoryId::generate(), Timestamp::now())
                .expect("forgets");
        }
        let mut tip = replacement.clone();
        if undo {
            tip.links = vec![Link {
                label: LinkLabel::Supersedes,
                target: LinkTarget::SameSet(forgotten.id),
            }];
        }
        store.insert(&tip).expect("inserts the replacement");

        let mut held: Vec<MemoryId> = Vec::new();
        for _ in 0..PAGE_LIMIT {
            let asked = Question {
                already_known: &held,
                ..Question::new("alpha alpha", 20)
            };
            let recall = recall_across(&mut [&mut store], &asked).expect("recalls");
            if recall.hits.is_empty() {
                return None;
            }
            for hit in &recall.hits {
                if hit.memory.id == distant.id {
                    return Some(hit.inherited);
                }
                held.push(hit.memory.id);
            }
        }
        panic!("paging did not end");
    };
    assert!(
        inherited_by_distant(false, true).is_some_and(|inherited| inherited > 0.0),
        "the control: a link naming the replacement directly is walked"
    );
    assert_eq!(
        inherited_by_distant(false, false),
        Some(0.0),
        "a link to a forgotten id that nothing supersedes reaches nothing"
    );
    assert!(
        inherited_by_distant(true, false).is_some_and(|inherited| inherited > 0.0),
        "a link to a forgotten id should reach the memory that supersedes it"
    );
}

/// **A chain ending on an id nothing holds stays out of reach across stores.** `standing_within`
/// re-resolves a memory's concessions to other sets, and used to derive `unresolved` from those
/// alone -- so a chain stopping on a same-set id nobody holds lost the reason it stopped, and the
/// memory looked up as current. Found 2026-09-30 while fixing
/// `a-forgotten-replacement-reads-as-superseded-beyond-this-set`.
#[test]
fn a_chain_ending_on_an_absent_id_stays_unresolved_across_stores() {
    let absent = MemoryId::generate();
    let ours = a_memory("alpha ours, a replacement in a loaded set");

    // Alone: no concession, only an id nobody holds.
    let mut alone = a_memory("alpha yielding to an id nothing holds");
    alone.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::SameSet(absent),
    }];
    // Beside a concession that does resolve: the absent branch is still out of reach.
    let mut split = a_memory("alpha split, one half in a loaded set and one nowhere");
    split.links = vec![
        Link {
            label: LinkLabel::SupersededBy,
            target: LinkTarget::CrossSet {
                set: SetName::new("ours").expect("valid set name"),
                id: ours.id,
                keywords: Vec::new(),
                set_id: None,
            },
        },
        Link {
            label: LinkLabel::SupersededBy,
            target: LinkTarget::SameSet(absent),
        },
    ];

    let our_store = store_of(project("ours"), &[&ours]);
    let their_store = store_of(project("rival"), &[&alone, &split]);
    let stores = [&our_store, &their_store];

    assert_eq!(
        resolve_within(&stores, 1, alone.id).expect("resolves"),
        Resolution::Superseded {
            tips: Vec::new(),
            unresolved: true,
            forgotten: false,
        },
        "a chain stopping on an absent id must not read as current"
    );
    assert_eq!(
        resolve_within(&stores, 1, split.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![ours.id],
            unresolved: true,
            forgotten: false,
        },
        "the concession resolves, and the absent branch is still reported"
    );
}

/// **A link landing earlier in a chain is walked past a forgotten member to the memory that undid
/// the forget.** A is superseded by B, B is forgotten, and C supersedes B's id -- the undo. A link
/// naming A must reach C: the tombstone keeps B's ancestry so the chain survives losing it, and the
/// walk crosses an id it cannot produce rather than stopping there. Asked by the human partner
/// 2026-09-30, while `a-forgotten-replacement-reads-as-superseded-beyond-this-set` was being fixed.
#[test]
fn a_link_before_a_forgotten_member_of_a_chain_reaches_what_undid_the_forget() {
    let contents = [
        "alpha alpha gamma",
        "alpha gamma gamma",
        "alpha gamma",
        "gamma gamma",
        "gamma",
        "beta beta beta",
        "beta gamma",
    ];
    let fillers: Vec<Memory> = contents.iter().map(|text| a_memory(text)).collect();
    let oldest = a_memory("delta, the first version");
    let mut forgotten = a_memory("delta, the version that was forgotten");
    forgotten.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(oldest.id),
    }];
    let mut undo = a_memory("alpha alpha alpha");
    undo.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(forgotten.id),
    }];
    let mut distant = a_memory("gamma gamma gamma");
    distant.links = vec![Link {
        label: LinkLabel::Supports,
        target: LinkTarget::SameSet(oldest.id),
    }];

    let mut store = store_of(project("scatterhoard"), &[&oldest, &forgotten, &distant]);
    for filler in &fillers {
        store.insert(filler).expect("inserts");
    }
    store
        .forget(forgotten.id, MemoryId::generate(), Timestamp::now())
        .expect("forgets");
    store.insert(&undo).expect("inserts the undo");

    assert!(
        matches!(
            store.resolve_supersession(oldest.id).expect("resolves"),
            Resolution::Superseded { ref tips, .. } if tips == &vec![undo.id]
        ),
        "the oldest memory resolves past the tombstone to the undo"
    );

    let mut held: Vec<MemoryId> = Vec::new();
    for _ in 0..PAGE_LIMIT {
        let asked = Question {
            already_known: &held,
            ..Question::new("alpha alpha", 20)
        };
        let recall = recall_across(&mut [&mut store], &asked).expect("recalls");
        assert!(
            !recall.hits.is_empty(),
            "paging ended without reaching the memory that links to the oldest version"
        );
        if let Some(hit) = recall.hits.iter().find(|hit| hit.memory.id == distant.id) {
            assert!(
                hit.inherited > 0.0,
                "a link naming the oldest version should be walked to the undo, which the query \
                 matches: inherited {}",
                hit.inherited
            );
            return;
        }
        held.extend(recall.hits.iter().map(|hit| hit.memory.id));
    }
    panic!("paging did not end");
}

/// **A reference from another set is matched by identity too**, on the rule `holder_of` follows:
/// a row naming this set by a name it no longer uses still points here, and a row naming a different
/// set that happens to share the name does not.
#[test]
fn a_reference_from_another_set_follows_identity_rather_than_the_name() {
    let target = a_memory("alpha the memory being pointed at");
    let held = store_of(project("known-by-another-name-now"), &[&target]);
    let identity = held.mint_set_id().expect("mints an identity");

    let mut pointing = a_memory("alpha a note elsewhere that refers to it");
    pointing.links = vec![Link {
        label: LinkLabel::RelatedTo,
        target: LinkTarget::CrossSet {
            set: SetName::new("the-old-name").expect("valid set name"),
            id: target.id,
            keywords: Vec::new(),
            set_id: Some(identity),
        },
    }];
    let elsewhere = store_of(project("elsewhere"), &[&pointing]);

    let stores: Vec<&Store> = vec![&held, &elsewhere];
    let found = referenced_from_other_sets(&stores, held.scope(), target.id).expect("reads");
    assert_eq!(
        found.iter().map(|r| r.source).collect::<Vec<_>>(),
        vec![pointing.id],
        "a stale name must not hide a reference whose identity names this set"
    );

    // And the mirror: a row carrying a *different* set's identity is not ours, whatever it is named.
    let mut impostor = a_memory("alpha a note naming this set but meaning another");
    impostor.links = vec![Link {
        label: LinkLabel::RelatedTo,
        target: LinkTarget::CrossSet {
            set: SetName::new("known-by-another-name-now").expect("valid set name"),
            id: target.id,
            keywords: Vec::new(),
            set_id: Some(scatterhoard::memory::SetId::generate()),
        },
    }];
    let confused = store_of(project("confused"), &[&impostor]);
    let stores: Vec<&Store> = vec![&held, &elsewhere, &confused];
    let found = referenced_from_other_sets(&stores, held.scope(), target.id).expect("reads");
    assert!(
        !found.iter().any(|r| r.source == impostor.id),
        "a row stating another set's identity does not point here, whatever name it uses"
    );
}

// ---------------------------------------------------------------------------
// A link naming its own set by a name the set no longer uses
// ---------------------------------------------------------------------------
//
// Written 2026-09-14 to confirm a suspected gap rather than to pin a fix. `write_links` stores a link
// in the compact same-set form only when the name it carries is the set's *current* name, while
// `edges_into_another_set` and `holder_of` match a set by its identity. So a link inside a set that
// names that set by an old name, and carries its identity, is stored as though it pointed at another
// set -- the shape a pre-rename copy's link takes once the set has been renamed and the copy imported.
// Found by re-anchoring `disputes: ask the memory's own store for claims against it`, which had been
// landing on `referenced_from_other_sets`' scope guard; with that guard removed the whole suite passed.

/// A link to `id` spelled as a cross-set link: naming `the-old-name`, which the holding set no longer
/// uses, and carrying the holding set's own `identity`.
fn a_link_to_its_own_set_by_an_old_name(
    label: LinkLabel,
    id: MemoryId,
    identity: scatterhoard::memory::SetId,
) -> Link {
    Link {
        label,
        target: LinkTarget::CrossSet {
            set: SetName::new("the-old-name").expect("valid set name"),
            id,
            keywords: Vec::new(),
            set_id: Some(identity),
        },
    }
}

/// **The guard in `referenced_from_other_sets`, reached.** A memory relating to another memory in its
/// own set is not a reference *from another set*, however the link spells the set.
#[test]
fn a_link_naming_its_own_set_by_an_old_name_is_not_a_reference_from_another_set() {
    let target = a_memory("alpha the memory being pointed at");
    let mut held = store_of(project("known-by-another-name-now"), &[&target]);
    let identity = held.mint_set_id().expect("mints an identity");

    let mut pointing = a_memory("alpha a note in the same set that refers to it");
    pointing.links = vec![a_link_to_its_own_set_by_an_old_name(
        LinkLabel::RelatedTo,
        target.id,
        identity,
    )];
    held.insert(&pointing).expect("inserts");

    let stores: Vec<&Store> = vec![&held];
    let found = referenced_from_other_sets(&stores, held.scope(), target.id).expect("reads");
    assert!(
        found.is_empty(),
        "a link within one set was reported as a reference from another set: {:?}",
        found.iter().map(|r| r.source).collect::<Vec<_>>()
    );
}

/// **A supersession written `supersedes`** -- the form `hoard_memorize` writes -- naming its own set by
/// an old name. The memory it replaces should resolve as replaced, by it.
#[test]
fn a_supersedes_link_naming_its_own_set_by_an_old_name_still_supersedes() {
    let yielded = a_memory("alpha alpha the memory that was corrected");
    let mut store = store_of(project("known-by-another-name-now"), &[&yielded]);
    let identity = store.mint_set_id().expect("mints an identity");

    let mut replacement = a_memory("alpha alpha the correction");
    replacement.links = vec![a_link_to_its_own_set_by_an_old_name(
        LinkLabel::Supersedes,
        yielded.id,
        identity,
    )];
    store.insert(&replacement).expect("inserts");

    assert_eq!(
        store.resolve_supersession(yielded.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![replacement.id],
            unresolved: false,
            forgotten: false,
        },
        "a replacement in the same set, naming it by a name it no longer uses, did not replace it"
    );
}

/// **The same concession written `superseded_by`**, from the replaced memory's side.
#[test]
fn a_superseded_by_link_naming_its_own_set_by_an_old_name_still_resolves() {
    let replacement = a_memory("alpha alpha the correction");
    let mut store = store_of(project("known-by-another-name-now"), &[&replacement]);
    let identity = store.mint_set_id().expect("mints an identity");

    let mut yielded = a_memory("alpha alpha the memory that was corrected");
    yielded.links = vec![a_link_to_its_own_set_by_an_old_name(
        LinkLabel::SupersededBy,
        replacement.id,
        identity,
    )];
    store.insert(&yielded).expect("inserts");

    assert_eq!(
        store.resolve_supersession(yielded.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![replacement.id],
            unresolved: false,
            forgotten: false,
        },
        "a concession to a memory this very set holds was reported as out of reach"
    );
}

/// **Restore reaches the same step by its own path.** `replace_with` writes a file's memories without
/// going through `insert`, so a supersession arriving that way has to be canonicalised by identity
/// too.
#[test]
fn a_restored_supersedes_link_naming_its_own_set_by_an_old_name_still_supersedes() {
    let mut store = store_of(project("known-by-another-name-now"), &[]);
    let identity = store.mint_set_id().expect("mints an identity");

    let yielded = a_memory("alpha alpha the memory that was corrected");
    let mut replacement = a_memory("alpha alpha the correction");
    replacement.links = vec![a_link_to_its_own_set_by_an_old_name(
        LinkLabel::Supersedes,
        yielded.id,
        identity,
    )];
    let scope = store.scope().clone();
    store
        .replace_with(
            &scope,
            &[
                scatterhoard::memory_file::Record::Memory(Box::new(yielded.clone())),
                scatterhoard::memory_file::Record::Memory(Box::new(replacement.clone())),
            ],
        )
        .expect("restores");

    assert_eq!(
        store.resolve_supersession(yielded.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![replacement.id],
            unresolved: false,
            forgotten: false,
        },
        "a restored replacement naming its own set by an old name did not replace it"
    );
}

/// The oldest and newest ends of a chain whose middle arrives only as a tombstone, and that tombstone:
/// its ancestry says the forgotten middle superseded the oldest, naming the set by an old name.
///
/// **The tombstone's edge is the only thing joining the oldest memory to the chain.** The oldest
/// carries no `superseded_by` of its own, deliberately: with one, the chain resolved through that
/// link and these tests passed with the tombstone's edge stored as cross-set -- which is how the
/// mutations removing the identity from both tombstone paths first survived them.
fn a_chain_around_a_tombstone_naming_its_own_set_by_an_old_name(
    identity: scatterhoard::memory::SetId,
) -> (Memory, scatterhoard::memory_file::Tombstone, Memory) {
    let middle = MemoryId::generate();
    let oldest = a_memory("alpha the first conclusion");
    let tombstone = scatterhoard::memory_file::Tombstone::new(
        middle,
        Attribution {
            by: MemoryId::generate(),
            at: Timestamp::from_str("2026-09-14T09:00:00Z").expect("valid timestamp"),
        },
        vec![a_link_to_its_own_set_by_an_old_name(
            LinkLabel::Supersedes,
            oldest.id,
            identity,
        )],
        None,
    );
    let mut newest = a_memory("alpha the conclusion that stands");
    newest.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(middle),
    }];
    (oldest, tombstone, newest)
}

/// **A tombstone arriving from a file keeps its ancestry as the file wrote it**, so a forgotten
/// middle's edge naming this set by an old name reaches `entomb` in the long spelling -- where a
/// memory forgotten on this machine never can, its ancestry being read back from rows already
/// canonicalised.
#[test]
fn an_arriving_tombstones_ancestry_naming_its_own_set_by_an_old_name_still_links_the_chain() {
    let mut store = store_of(project("known-by-another-name-now"), &[]);
    let identity = store.mint_set_id().expect("mints an identity");
    let (oldest, tombstone, newest) =
        a_chain_around_a_tombstone_naming_its_own_set_by_an_old_name(identity);

    store.insert(&oldest).expect("inserts");
    store.insert(&newest).expect("inserts");
    store.accept_tombstone(&tombstone).expect("accepts");

    assert_eq!(
        store.resolve_supersession(oldest.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![newest.id],
            unresolved: false,
            forgotten: false,
        },
        "the forgotten middle's edge, naming its own set by an old name, no longer linked the chain"
    );
}

/// And the same chain arriving through a restore, whose tombstones take a path of their own.
#[test]
fn a_restored_tombstones_ancestry_naming_its_own_set_by_an_old_name_still_links_the_chain() {
    let mut store = store_of(project("known-by-another-name-now"), &[]);
    let identity = store.mint_set_id().expect("mints an identity");
    let (oldest, tombstone, newest) =
        a_chain_around_a_tombstone_naming_its_own_set_by_an_old_name(identity);

    let scope = store.scope().clone();
    store
        .replace_with(
            &scope,
            &[
                scatterhoard::memory_file::Record::Memory(Box::new(oldest.clone())),
                scatterhoard::memory_file::Record::Tombstone(tombstone),
                scatterhoard::memory_file::Record::Memory(Box::new(newest.clone())),
            ],
        )
        .expect("restores");

    assert_eq!(
        store.resolve_supersession(oldest.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![newest.id],
            unresolved: false,
            forgotten: false,
        },
        "a restored tombstone's edge, naming its own set by an old name, no longer linked the chain"
    );
}

/// **The other half of deciding by identity.** A link naming this set's *current* name but carrying a
/// different set's identity means that other set -- the rule a set that only shares a name already
/// follows at resolution -- so it is not a same-set supersession, and the memory it names here stays
/// current. Canonicalising by name made it one.
#[test]
fn a_link_naming_this_sets_name_with_another_sets_identity_does_not_supersede_here() {
    let yielded = a_memory("alpha alpha a memory that nothing in this set replaced");
    let mut store = store_of(project("shared-name"), &[&yielded]);
    store.mint_set_id().expect("mints its own identity");

    let mut claimant = a_memory("alpha alpha a memory whose link means a different set");
    claimant.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::CrossSet {
            set: SetName::new("shared-name").expect("valid set name"),
            id: yielded.id,
            keywords: Vec::new(),
            set_id: Some(scatterhoard::memory::SetId::generate()),
        },
    }];
    store.insert(&claimant).expect("inserts");

    assert_eq!(
        store.resolve_supersession(yielded.id).expect("resolves"),
        Resolution::Current,
        "a link whose identity names another set superseded a memory in this one"
    );
}

/// Raw cosine satisfies the split-invariance property too — for a stronger reason than the union
/// centroid does.
///
/// [`splitting_a_corpus_across_stores_does_not_change_the_answer`] pins the property against the
/// shipping [`Centring::Combined`], which earns it by computing one centroid over everything being
/// searched. [`Centring::None`] earns it by having no population statistic at all, so partitioning
/// cannot reach the score. Asserted because the ablation in `tests/probes.rs` found raw cosine
/// separating answerable from unanswerable queries better than centring does, which makes *may we
/// use it* a live question — and this is the property that would have ruled it out.
#[test]
fn raw_cosine_also_survives_splitting_a_corpus_across_stores() {
    let corpus = a_corpus();
    let refs: Vec<&Memory> = corpus.iter().collect();

    for centring in [Centring::None, Centring::Combined] {
        let mut whole = store_of(project("whole"), &refs);
        let mut left = store_of(project("left"), &refs[..2]);
        let mut right = store_of(Scope::Local, &refs[2..]);
        let stages = Stages {
            centring,
            ..Stages::default()
        };
        let query = "alpha alpha gamma";

        let single = recall_across_with(&mut [&mut whole], &Question::new(query, 4), stages)
            .expect("recalls");
        let union = recall_across_with(
            &mut [&mut left, &mut right],
            &Question::new(query, 4),
            stages,
        )
        .expect("recalls");

        let ids = |hits: &[scatterhoard::store::Recalled]| -> Vec<String> {
            hits.iter().map(|hit| hit.memory.id.to_string()).collect()
        };
        assert_eq!(
            ids(&union.hits),
            ids(&single.hits),
            "{centring:?}: a split corpus ranked differently from one store holding all of it"
        );
        for (a, b) in union.hits.iter().zip(&single.hits) {
            assert!(
                (a.similarity - b.similarity).abs() < 1e-5,
                "{centring:?}: scores diverged, {} vs {}",
                a.similarity,
                b.similarity
            );
        }
    }
}

/// The union centroid's own cost: **what a set scores depends on which other sets are loaded.**
///
/// The centroid is computed over everything being searched, so adding an unrelated set moves it and
/// rescores memories that set has nothing to do with. A session holding four sets therefore gets
/// different scores for a project's memories than a session holding only that project — the same
/// memories, the same query, the same model.
///
/// This is the mirror image of [`splitting_a_corpus_across_stores_does_not_change_the_answer`]:
/// that one pins invariance to *how the same memories are partitioned*, this one shows there is no
/// invariance to *which other memories are present*. Raw cosine has both, which is the case for
/// taking it seriously. Asserted rather than merely measured so the difference cannot quietly go
/// away.
#[test]
fn the_union_centroid_makes_a_set_score_differently_when_another_set_is_loaded() {
    let corpus = a_corpus();
    let refs: Vec<&Memory> = corpus.iter().collect();
    let unrelated: Vec<Memory> = ["gamma gamma beta", "beta beta beta"]
        .iter()
        .map(|c| a_memory(c))
        .collect();
    let unrelated_refs: Vec<&Memory> = unrelated.iter().collect();
    let query = "alpha alpha gamma";

    let scores_for = |centring: Centring, with_neighbour: bool| -> Vec<f32> {
        let mut mine = store_of(project("mine"), &refs);
        let mut theirs = store_of(Scope::Local, &unrelated_refs);
        let stages = Stages {
            centring,
            ..Stages::default()
        };
        let recall = if with_neighbour {
            recall_across_with(
                &mut [&mut mine, &mut theirs],
                &Question::new(query, 4),
                stages,
            )
        } else {
            recall_across_with(&mut [&mut mine], &Question::new(query, 4), stages)
        }
        .expect("recalls");
        recall
            .hits
            .iter()
            .filter(|hit| hit.scope == project("mine"))
            .map(|hit| hit.similarity)
            .collect()
    };

    let combined_alone = scores_for(Centring::Combined, false);
    let combined_beside = scores_for(Centring::Combined, true);
    assert!(
        combined_alone
            .iter()
            .zip(&combined_beside)
            .any(|(a, b)| (a - b).abs() > 1e-5),
        "the union centroid was expected to rescore a set when another set is loaded, and did not"
    );

    let raw_alone = scores_for(Centring::None, false);
    let raw_beside = scores_for(Centring::None, true);
    for (a, b) in raw_alone.iter().zip(&raw_beside) {
        assert!(
            (a - b).abs() < 1e-6,
            "raw cosine rescored a set when another set was loaded: {a} vs {b}"
        );
    }
}
