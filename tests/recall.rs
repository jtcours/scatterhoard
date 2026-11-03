// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for recall.
//!
//! Almost all of this runs against a deterministic fake embedder rather than a real model. That is
//! deliberate: what needs testing here is the *behaviour built on top of* similarity — supersession
//! resolving before ranking, loud degradation when no model is present — and a real model would
//! make those tests depend on its judgement instead of on ours. Whether the model produces useful
//! vectors was settled by the spike.

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
use scatterhoard::store::{Floor, Question, Stages, Store, recall_across, recall_across_with};

/// Counts three marker words, so similarity is exactly what the test wrote rather than whatever a
/// model happened to think. The trailing constant keeps a vector from ever being all zeros, which
/// `Vector` refuses.
#[derive(Debug)]
struct MarkerEmbedder;

impl Embedder for MarkerEmbedder {
    fn identity(&self) -> ModelIdentity {
        // One identity for every test double, because these tests are about behaviour that assumes
        // a consistent model. The one place two models have to disagree is tests/model_identity.rs.
        ModelIdentity::new("marker-embedder")
    }

    fn dimension(&self) -> usize {
        4
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|text| {
                let count = |word: &str| text.matches(word).count() as f32;
                Vector::new(vec![count("alpha"), count("beta"), count("gamma"), 0.01])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

fn scope() -> Scope {
    Scope::Shared(SetName::new("scatterhoard").expect("valid set name"))
}

fn a_store() -> Store {
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    store
}

fn a_memory(content: &str) -> Memory {
    Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: Timestamp::from_str("2026-08-01T09:00:00Z").expect("valid timestamp"),
        },
    )
}

// ---------------------------------------------------------------------------
// Degradation
// ---------------------------------------------------------------------------

/// Without a model, semantic search is gone but keyword search, direct lookup and traversal are
/// not. Recall must *say so*: a session that believes memory is being consulted when it isn't is
/// the precise failure the `recall-when` convention warns about, and it is worse than no memory.
#[test]
fn recall_without_a_model_says_semantic_search_is_unavailable() {
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    let recall = store.recall(&Question::new("alpha", 5)).expect("recalls");

    assert!(
        !recall.searched,
        "recall must report that it could not search by meaning"
    );
    assert!(recall.hits.is_empty());
}

#[test]
fn recall_with_a_model_reports_semantic_search_as_available() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    assert!(
        store
            .recall(&Question::new("alpha", 5))
            .expect("recalls")
            .searched
    );
}

// ---------------------------------------------------------------------------
// Finding things
// ---------------------------------------------------------------------------

#[test]
fn recall_ranks_the_closest_memory_first() {
    let mut store = a_store();
    let alpha = a_memory("alpha alpha alpha");
    let gamma = a_memory("gamma gamma gamma");
    store.insert(&alpha).expect("inserts");
    store.insert(&gamma).expect("inserts");

    let recall = store
        .recall(&Question::new("alpha alpha", 5))
        .expect("recalls");
    assert_eq!(recall.hits[0].memory.id, alpha.id);
}

#[test]
fn recall_returns_at_most_k_memories() {
    let mut store = a_store();
    for _ in 0..5 {
        store.insert(&a_memory("alpha beta")).expect("inserts");
    }

    assert_eq!(
        store
            .recall(&Question::new("alpha", 2))
            .expect("recalls")
            .hits
            .len(),
        2
    );
}

/// Import embeds the whole set, so a store is searchable straight after an import without any
/// separate indexing step.
#[test]
fn an_imported_set_is_searchable() {
    let mut store = a_store();
    let memory = a_memory("alpha alpha");
    store
        .import(&scatterhoard::memory_file::MemorySet {
            scope: scope(),
            memories: vec![memory.clone(), a_memory("gamma gamma")],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("imports");

    let recall = store.recall(&Question::new("alpha", 5)).expect("recalls");
    assert_eq!(recall.hits.len(), 2);
    assert_eq!(recall.hits[0].memory.id, memory.id);
}

/// Vectors are derived state, but re-deriving them on every process start would put the embedder on
/// the critical path of simply opening a store. They are persisted, so reopening does not re-embed
/// — and a store reopened *without* a model can still search, because the vectors are already there
/// even though a new query could not be embedded.
#[test]
fn vectors_survive_reopening_the_store() {
    let directory = tempdir();
    let memory = a_memory("alpha alpha");
    {
        let mut store = Store::open(scope(), directory.path()).expect("creates");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(MarkerEmbedder));
        store.insert(&memory).expect("inserts");
    }

    let mut store = Store::open(scope(), directory.path()).expect("reopens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    let recall = store.recall(&Question::new("alpha", 5)).expect("recalls");

    assert_eq!(recall.hits.len(), 1, "vectors were not persisted");
    assert_eq!(recall.hits[0].memory.id, memory.id);
}

// ---------------------------------------------------------------------------
// Validity resolves before ranking
// ---------------------------------------------------------------------------

/// The decision from `recall-mechanisms.md`: supersession is a validity relation, not a relevance
/// one, so it resolves *before* ranking. A query matching a superseded memory returns the memory
/// that replaced it — plus the id of what was actually matched, without which the caller holds a
/// memory it did not ask for and cannot tell why.
#[test]
fn a_query_matching_a_superseded_memory_returns_its_replacement() {
    let mut store = a_store();
    let old = a_memory("alpha alpha alpha");
    let mut new = a_memory("alpha alpha alpha beta");
    new.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(old.id),
    }];
    store.insert(&old).expect("inserts");
    store.insert(&new).expect("inserts");

    let recall = store
        .recall(&Question::new("alpha alpha alpha", 5))
        .expect("recalls");

    assert!(
        recall.hits.iter().all(|hit| hit.memory.id != old.id),
        "a superseded memory must not be returned as though current"
    );
    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == new.id)
        .expect("the replacement should be present");
    assert_eq!(
        hit.matched,
        Some(old.id),
        "the id actually matched has to be reported, or the caller cannot tell why it got this"
    );
}

/// Resolving to the *tip*, not to an intermediate that was itself corrected.
#[test]
fn a_query_matching_the_start_of_a_chain_returns_the_end_of_it() {
    let mut store = a_store();
    let oldest = a_memory("alpha alpha alpha");
    let mut middle = a_memory("alpha alpha");
    middle.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(oldest.id),
    }];
    let mut newest = a_memory("alpha beta");
    newest.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(middle.id),
    }];
    for memory in [&oldest, &middle, &newest] {
        store.insert(memory).expect("inserts");
    }

    let recall = store
        .recall(&Question::new("alpha alpha alpha", 5))
        .expect("recalls");
    let ids: Vec<_> = recall.hits.iter().map(|hit| hit.memory.id).collect();

    assert!(ids.contains(&newest.id), "the tip should be returned");
    assert!(!ids.contains(&oldest.id) && !ids.contains(&middle.id));
}

/// Resolution can collapse two hits into one when a memory and its replacement both rank. The
/// replacement must appear once, not twice.
#[test]
fn a_collapsed_pair_is_returned_once() {
    let mut store = a_store();
    let old = a_memory("alpha alpha");
    let mut new = a_memory("alpha alpha");
    new.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(old.id),
    }];
    store.insert(&old).expect("inserts");
    store.insert(&new).expect("inserts");

    let recall = store.recall(&Question::new("alpha", 5)).expect("recalls");
    let appearances = recall.hits.iter().filter(|h| h.memory.id == new.id).count();

    assert_eq!(appearances, 1, "the replacement was returned twice");
}

/// A memory nothing supersedes reports no `matched` id — the field means "you asked for something
/// else", so populating it always would make it meaningless.
#[test]
fn a_current_memory_reports_no_substitution() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    let recall = store.recall(&Question::new("alpha", 5)).expect("recalls");
    assert_eq!(recall.hits[0].matched, None);
}

/// Findability is a property of the memory, not a property of the store it sits in.
///
/// There is exactly one window in which the two visibly differ, and this test stands in it: a store
/// that has just been handed a model has not embedded anything yet, because the sweep that fills
/// the gap runs on first *use* rather than at open — deliberately, since embedding at open is what
/// the in-memory vector cache exists to avoid. Inside that window the store is capable and the
/// memory is still unfindable, which is the whole difference between asking the `vectors` table and
/// asking whether an embedder exists.
///
/// Answering from the capability would be the server-wide-flag mistake behind
/// `project-stores-get-no-embedder`, one layer down.
#[test]
fn findability_asks_the_memory_rather_than_the_store() {
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    let orphan = a_memory("alpha alpha");
    store.insert(&orphan).expect("inserts");
    assert!(
        !store.is_findable_by_meaning(orphan.id).expect("checks"),
        "nothing could have embedded it"
    );

    store.set_embedder(Arc::new(MarkerEmbedder));
    assert!(
        !store.is_findable_by_meaning(orphan.id).expect("checks"),
        "the store gained the capability; this memory has not yet gained a vector"
    );

    // And using the store closes the gap, which is what the design asks for and what
    // `a-populated-store-cannot-gain-embeddings` was filed about.
    store.recall(&Question::new("alpha", 5)).expect("recalls");
    assert!(
        store.is_findable_by_meaning(orphan.id).expect("checks"),
        "first use must repair a memory stored before the model arrived"
    );
}

/// Forgetting must take the vector out of the in-memory set, not merely out of the database.
///
/// `forget` deletes the memory row and reloads, and the reload only ever *added* rows — so the
/// forgotten memory's vector stayed behind. Nothing leaked its content, because resolving a hit
/// reads SQL and finds nothing there; what leaked was scan width. Recall scans a fixed multiple of
/// the requested limit and drops the hits that resolve to nothing, so enough departed vectors ahead
/// of a surviving memory push it off the end of the scan and recall answers with nothing at all.
///
/// Written with the survivor pointing away from the query, so it is genuinely the *last* of the
/// three: a test where it ranked first would pass however much rubbish the set held.
#[test]
fn forgetting_does_not_cost_the_survivors_their_place_in_the_scan() {
    let mut store = a_store();
    let nearest = a_memory("alpha alpha");
    let middle = a_memory("alpha beta");
    let survivor = a_memory("beta beta");
    store.insert(&nearest).expect("inserts");
    store.insert(&middle).expect("inserts");
    store.insert(&survivor).expect("inserts");

    let by = MemoryId::generate();
    let at = Timestamp::from_str("2026-08-07T09:00:00Z").expect("valid timestamp");
    store.forget(nearest.id, by, at).expect("forgets");
    store.forget(middle.id, by, at).expect("forgets");

    let recall = store.recall(&Question::new("alpha", 1)).expect("recalls");
    assert_eq!(
        recall.hits.len(),
        1,
        "the only memory left must still be recallable"
    );
    assert_eq!(recall.hits[0].memory.id, survivor.id);
}

/// A forgotten memory's vector must leave the scan, and the assertion has to be about the **scores**
/// rather than about what comes back.
///
/// The test above used to catch a reload that kept the old vectors, because recall scanned only
/// twice the requested count: two stale vectors filled that window and the survivor never appeared.
/// The walk changed that — the whole population is scored now, so a stale vector is skipped when it
/// resolves to a memory that is no longer there, and nothing about the returned list looks wrong.
///
/// What a stale vector still does is sit in the **centroid**, which every score is measured against,
/// and take a seed slot in a walk that can never reach anything from it. So this compares a store
/// that forgot against one that never held them: the two must agree exactly.
#[test]
fn a_forgotten_memory_leaves_the_centroid_every_score_is_measured_against() {
    let survivors = [a_memory("alpha alpha"), a_memory("beta beta")];
    let forgotten = [a_memory("gamma gamma gamma"), a_memory("gamma alpha")];

    let mut haunted = a_store();
    for memory in survivors.iter().chain(forgotten.iter()) {
        haunted.insert(memory).expect("inserts");
    }
    let by = MemoryId::generate();
    let at = Timestamp::from_str("2026-08-07T09:00:00Z").expect("valid timestamp");
    for memory in &forgotten {
        haunted.forget(memory.id, by, at).expect("forgets");
    }

    let mut clean = a_store();
    for memory in &survivors {
        clean.insert(memory).expect("inserts");
    }

    let question = Question::new("alpha", 20);
    let scores = |store: &mut Store| -> Vec<(MemoryId, f32)> {
        store
            .recall(&question)
            .expect("recalls")
            .hits
            .iter()
            .map(|hit| (hit.memory.id, hit.similarity))
            .collect()
    };

    assert_eq!(
        scores(&mut haunted),
        scores(&mut clean),
        "a forgotten memory must not go on shifting the corpus mean"
    );
}

// ---------------------------------------------------------------------------

fn tempdir() -> TempDir {
    let mut path = std::env::temp_dir();
    path.push(format!("scatterhoard-recall-{}", MemoryId::generate()));
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------

/// Links do not enter the embedding input, so a store with a link and one without hold identical
/// vectors. That makes an unlinked twin of a fixture an exact control: any difference in the result
/// is the walk and nothing else.
fn linking(from: &mut Memory, label: LinkLabel, to: &Memory) {
    from.links.push(Link {
        label,
        target: LinkTarget::SameSet(to.id),
    });
}

/// Nine memories, which is deliberately **more than the five seeds the walk takes**: in a corpus
/// smaller than the seed budget every memory is a walk origin, so nothing can inherit and traversal
/// correctly does nothing at all. A fixture that small cannot show the mechanism working, and one
/// built on it was reading the old self-inheriting walk instead.
///
/// Ranked against `alpha alpha` by content alone, they run 0, 1, 3, 7, 2, 6, 8, 5, 4 — so [`FAINT`]
/// is the worst match in the set, which is what makes it a fair test of reaching what wording
/// cannot.
fn nine_memories() -> Vec<Memory> {
    [
        "alpha alpha alpha",
        "alpha alpha gamma",
        "alpha gamma gamma",
        "alpha gamma",
        "gamma gamma gamma",
        "gamma gamma",
        "gamma",
        "beta beta beta",
        "beta gamma",
    ]
    .iter()
    .map(|content| a_memory(content))
    .collect()
}

/// The memory the query matches best, and so the walk's strongest seed.
const MATCHED: usize = 0;
/// The memory the query matches worst — last of the nine on content alone.
const FAINT: usize = 4;
/// Second-worst, and two hops out in the chain fixture.
const FAINTER_STILL: usize = 5;

fn store_holding(memories: &[Memory]) -> Store {
    let mut store = a_store();
    for memory in memories {
        store.insert(memory).expect("inserts");
    }
    store
}

fn recalled_ids(store: &mut Store, question: &Question<'_>) -> Vec<MemoryId> {
    store
        .recall(question)
        .expect("recalls")
        .hits
        .iter()
        .map(|hit| hit.memory.id)
        .collect()
}

/// How many pages either helper will ask for before deciding the paging is broken.
///
/// **A bound rather than a `loop`, and it was earned.** Written unbounded, these helpers spin
/// forever the moment `already_known` stops suppressing — which is exactly what one of the recorded
/// mutations does, so the mutation runner wedged on it for four hours instead of reporting a caught
/// mutation in three seconds. A test that hangs on a defect reports nothing; a test that fails on it
/// reports the defect by name. No fixture here holds more than a dozen memories, so any run past
/// this is broken rather than large.
const PAGE_LIMIT: usize = 20;

/// The whole ranking, read a tranche at a time by handing back what the last call returned.
///
/// The cut is adaptive, so most of a ranking sits below it — and comparing two rankings is what
/// separates the walk's effect from the cut's. This is also the paging mechanism exercising itself:
/// if `already_known` did not work, this would not terminate with everything in it.
fn everything_in_order(store: &mut Store, query: &str) -> Vec<MemoryId> {
    let mut held: Vec<MemoryId> = Vec::new();
    for _ in 0..PAGE_LIMIT {
        let question = Question::new(query, 20).already_holding(&held);
        let hits = recalled_ids(store, &question);
        if hits.is_empty() {
            return held;
        }
        held.extend(hits);
    }
    panic!(
        "paging did not terminate: {} memories held and still more coming, which means \
            already_known stopped suppressing",
        held.len()
    );
}

/// The whole ranking with the walk's contribution kept, for assertions about *why* a memory is
/// placed where it is rather than only about where.
fn everything_scored(store: &mut Store, query: &str) -> Vec<(MemoryId, f32)> {
    let mut held: Vec<MemoryId> = Vec::new();
    let mut scored: Vec<(MemoryId, f32)> = Vec::new();
    for _ in 0..PAGE_LIMIT {
        let question = Question::new(query, 20).already_holding(&held);
        let hits = store.recall(&question).expect("recalls").hits;
        if hits.is_empty() {
            return scored;
        }
        for hit in hits {
            held.push(hit.memory.id);
            scored.push((hit.memory.id, hit.inherited));
        }
    }
    panic!(
        "paging did not terminate: {} memories held and still more coming, which means \
            already_known stopped suppressing",
        held.len()
    );
}

/// What one memory inherited, read from the whole ranking rather than from the cut.
///
/// The shipped weight admits a quarter of the inherited score, so on a fixture whose own-scores are
/// widely spaced a reached memory moves past its neighbours rather than to the top. Asserting on the
/// score itself says *reached or not* without depending on how far a particular fixture lets it
/// travel.
fn recalled_inherited(store: &mut Store, query: &str, id: MemoryId) -> Option<f32> {
    everything_scored(store, query)
        .into_iter()
        .find(|(other, _)| *other == id)
        .map(|(_, inherited)| inherited)
}

fn position_of(ids: &[MemoryId], id: MemoryId) -> Option<usize> {
    ids.iter().position(|other| *other == id)
}

/// **The inherited score has to survive into the ranking, or the walk is redundant.** A memory is
/// often relevant because of its *relation* rather than its wording — the measurement supporting a
/// decision, the constraint that decision rests on — and those are precisely the memories whose
/// content does not match the query.
///
/// Asserted against a control holding the same memories and no link, because the vectors are
/// identical between the two: the only difference is the edge.
#[test]
fn a_linked_memory_climbs_past_ones_the_query_matched_better() {
    let plain = nine_memories();
    let mut linked = plain.clone();
    linking(&mut linked[MATCHED], LinkLabel::Supports, &plain[FAINT]);

    let with = everything_in_order(&mut store_holding(&linked), "alpha alpha");
    let without = everything_in_order(&mut store_holding(&plain), "alpha alpha");

    assert_eq!(
        position_of(&without, plain[FAINT].id),
        Some(8),
        "on wording alone it is last of the nine"
    );
    // An overtaking rather than an absolute position: the shipped weight admits a quarter of the
    // inherited score, so on a fixture whose own-scores are widely spaced the lift moves a memory
    // past its neighbours rather than to the top. That is the behaviour, not a weak test — what must
    // not happen is no movement at all.
    assert!(
        position_of(&with, plain[FAINT].id) < position_of(&without, plain[FAINT].id),
        "the link has to lift it"
    );
    assert!(
        position_of(&with, plain[FAINT].id) < position_of(&with, plain[FAINTER_STILL].id),
        "specifically past the memory it trailed on wording alone"
    );
}

/// The walk reaches beyond a seed's immediate neighbours, and what it carries there is worth less
/// than what it carried one hop back.
#[test]
fn the_walk_reaches_a_memory_two_hops_out() {
    let plain = nine_memories();
    let mut linked = plain.clone();
    linking(&mut linked[MATCHED], LinkLabel::Supports, &plain[FAINT]);
    linking(
        &mut linked[FAINT],
        LinkLabel::Supports,
        &plain[FAINTER_STILL],
    );

    let with = everything_in_order(&mut store_holding(&linked), "alpha alpha");

    let two_hops = plain[FAINTER_STILL].id;
    assert!(
        recalled_inherited(&mut store_holding(&linked), "alpha alpha", two_hops)
            .is_some_and(|inherited| inherited != 0.0),
        "two hops from the seed is still reached, carrying a twice-decayed share"
    );
    assert!(
        position_of(&with, plain[FAINT].id) < position_of(&with, two_hops),
        "one hop out must outrank two hops out"
    );
}

/// Supersession is a validity relation rather than a relevance one, and this pins both halves of
/// what that means for the walk.
///
/// **The edge is not crossed** — reaching a memory *through* the edge that says something is out of
/// date would have the walk contradicting the resolution that already ran. **And the tip stands
/// where its predecessor stood**: a link authored against the superseded memory, which is the
/// ordinary case since the link was usually written before the correction was, belongs to the claim
/// that replaced it. Without that, the dead memory is a conduit for inherited score while the live
/// one is short of routes.
#[test]
fn the_walk_stands_on_the_tip_rather_than_the_memory_it_replaced() {
    let plain = nine_memories();
    let mut linked = plain.clone();
    linking(&mut linked[MATCHED], LinkLabel::Supersedes, &plain[FAINT]);
    linking(
        &mut linked[FAINTER_STILL],
        LinkLabel::Supports,
        &plain[FAINT],
    );

    let with = everything_in_order(&mut store_holding(&linked), "alpha alpha");

    assert!(
        !with.contains(&plain[FAINT].id),
        "the superseded memory is replaced by its tip, not returned"
    );
    assert!(
        recalled_inherited(
            &mut store_holding(&linked),
            "alpha alpha",
            plain[FAINTER_STILL].id
        )
        .is_some_and(|inherited| inherited != 0.0),
        "the memory citing the superseded one is reached from the tip that inherited that edge"
    );
}

/// **An out-link stays with the record that wrote it**: a superseded memory's own links are not
/// walked from the memory that replaced it.
///
/// Step 3 of `out-links-stay-with-the-record-that-wrote-them`. The in-link half above is untouched:
/// a link *to* the superseded memory was written about the claim, and follows it to the tip. A link
/// *from* it is that record's claim, and a replacement that did not restate it has dropped it --
/// which is the only way to drop a link once the memory holding it has been exported. The control
/// is the same graph with the link restated on the tip, so the test cannot pass because the target
/// was out of reach for some other reason.
#[test]
fn a_superseded_memorys_own_links_are_not_walked_from_its_tip() {
    let plain = nine_memories();

    let mut dropped = plain.clone();
    linking(&mut dropped[MATCHED], LinkLabel::Supersedes, &plain[FAINT]);
    linking(
        &mut dropped[FAINT],
        LinkLabel::Supports,
        &plain[FAINTER_STILL],
    );

    let mut restated = dropped.clone();
    linking(
        &mut restated[MATCHED],
        LinkLabel::Supports,
        &plain[FAINTER_STILL],
    );

    assert!(
        recalled_inherited(
            &mut store_holding(&restated),
            "alpha alpha",
            plain[FAINTER_STILL].id
        )
        .is_some_and(|inherited| inherited != 0.0),
        "the control: a link the tip restates is walked"
    );
    assert!(
        !recalled_inherited(
            &mut store_holding(&dropped),
            "alpha alpha",
            plain[FAINTER_STILL].id
        )
        .is_some_and(|inherited| inherited != 0.0),
        "a link only the superseded memory holds is not walked from its tip"
    );
}

// ---------------------------------------------------------------------------
// Where recall stops
// ---------------------------------------------------------------------------

/// `result-count-follows-the-limit-not-the-query`. Recall returns as many as the limit asks for,
/// best first, and no longer stops at the largest drop between scores: measured on real queries
/// against nonsense ones, that drop marked no boundary, and choosing among noise made the count
/// move with the limit. Nine memories, so a limit of six returns six and one of twenty returns all.
#[test]
fn recall_returns_as_many_as_the_limit_asks_for() {
    let memories = nine_memories();
    let mut store = store_holding(&memories);

    let six = recalled_ids(&mut store, &Question::new("alpha alpha", 6));
    assert_eq!(six.len(), 6);
    assert_eq!(six[0], memories[MATCHED].id);

    let all = recalled_ids(&mut store, &Question::new("alpha alpha", 20));
    assert_eq!(
        all.len(),
        memories.len(),
        "a limit past the set returns the set"
    );
}

/// **The answer does not change with the budget it was asked under**: a smaller limit returns the
/// start of what a larger one returns, so asking for more only ever adds to the end.
#[test]
fn a_smaller_limit_returns_the_start_of_a_larger_one() {
    let mut store = store_holding(&nine_memories());
    let all = recalled_ids(&mut store, &Question::new("alpha alpha", 9));

    for limit in 1..9 {
        assert_eq!(
            recalled_ids(&mut store, &Question::new("alpha alpha", limit)),
            all[..limit],
            "a limit of {limit}"
        );
    }
}

/// The limit is a ceiling.
#[test]
fn the_limit_caps_what_comes_back() {
    let mut store = store_holding(&nine_memories());

    assert_eq!(
        store
            .recall(&Question::new("alpha", 1))
            .expect("recalls")
            .hits
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// What the caller already holds
// ---------------------------------------------------------------------------

/// The parameter removes a memory from the **results**. Returning it again is pure cost: it spends
/// context telling the LLM something it already knows.
#[test]
fn an_already_known_memory_is_not_returned() {
    let memories = nine_memories();
    let mut store = store_holding(&memories);

    let held = [memories[MATCHED].id];
    let hits = recalled_ids(
        &mut store,
        &Question::new("alpha alpha", 20).already_holding(&held),
    );

    assert!(
        !hits.contains(&memories[MATCHED].id),
        "a memory the caller says it holds must not be spent again"
    );
    assert!(!hits.is_empty(), "and the rest of the ranking still comes");
}

/// **It suppresses output, not participation.** A memory the LLM already holds and is now querying
/// around is the best available seed — a known-relevant, already-validated anchor into the right
/// neighbourhood — and its links stay valid connective tissue. Dropping it from the walk would
/// disconnect whatever lies beyond it.
#[test]
fn an_already_known_memory_still_seeds_the_walk() {
    let plain = nine_memories();
    let mut linked = plain.clone();
    linking(&mut linked[MATCHED], LinkLabel::Supports, &plain[FAINT]);

    let held = [plain[MATCHED].id];
    let question = Question::new("alpha alpha", 20).already_holding(&held);

    let mut store = store_holding(&linked);
    assert!(
        !recalled_ids(&mut store, &question).contains(&plain[MATCHED].id),
        "the memory the caller holds is not returned"
    );

    let inherited = everything_scored(&mut store, "alpha alpha")
        .into_iter()
        .find(|(id, _)| *id == plain[FAINT].id)
        .map(|(_, inherited)| inherited);

    assert!(
        inherited.is_some_and(|score| score > 0.0),
        "the suppressed memory must still carry score to what it points at, got {inherited:?}"
    );
}

/// **Stateless paging.** Passing back what the last call returned is how a caller asks for the next
/// tranche: no cursor, no session state, nothing the server has to invalidate. This is what makes a
/// small default count safe — *give me more* has to mean something other than the same memories
/// again.
#[test]
fn asking_again_with_what_was_returned_gives_the_next_tranche() {
    let memories = nine_memories();
    let mut store = store_holding(&memories);

    // Four of nine, so there is a second tranche to ask for.
    let first = recalled_ids(&mut store, &Question::new("alpha alpha", 4));
    let second = recalled_ids(
        &mut store,
        &Question::new("alpha alpha", 4).already_holding(&first),
    );

    assert_eq!((first.len(), second.len()), (4, 4));
    assert!(
        second.iter().all(|id| !first.contains(id)),
        "the second call must not spend context on what the first already returned"
    );
    assert_eq!(
        everything_in_order(&mut store, "alpha alpha").len(),
        memories.len(),
        "paging to exhaustion must reach every memory exactly once"
    );
}

// ---------------------------------------------------------------------------
// Validity, before the walk rather than after it
// ---------------------------------------------------------------------------

/// **A supersession chain spends one seed, not several.** Its members resemble each other by
/// construction — a superseding memory is supposed to — so they rank adjacently, and seeds chosen
/// from unresolved ids hand most of a five-seed budget to memories that collapse to one.
///
/// Six links of one chain rank above everything else here, so seeding from raw ids would spend the
/// whole budget on them and the anchor below would never seed at all. What the anchor points at is
/// the only thing that can show the difference: it is the worst content match in the set, so nothing
/// but the walk can reach it.
#[test]
fn a_supersession_chain_does_not_spend_more_than_one_seed() {
    // The chain's six links are worded so the embedder sees them as identical — it counts marker
    // words — which is the point: a superseding memory resembles its predecessor, and here they
    // occupy the whole top of the ranking between them.
    let mut memories: Vec<Memory> = [
        "alpha alpha alpha as first written",
        "alpha alpha alpha corrected once",
        "alpha alpha alpha corrected twice",
        "alpha alpha alpha corrected again",
        "alpha alpha alpha nearly right",
        "alpha alpha alpha as it stands",
        "alpha gamma",
        "beta beta beta",
        "beta gamma",
        "gamma",
        "gamma gamma gamma",
    ]
    .iter()
    .map(|content| a_memory(content))
    .collect();

    for step in 1..6 {
        let previous = memories[step - 1].clone();
        linking(&mut memories[step], LinkLabel::Supersedes, &previous);
    }
    let anchor = 6;
    let distant = memories[10].clone();
    linking(&mut memories[anchor], LinkLabel::Supports, &distant);

    let tip = memories[5].id;
    let mut store = store_holding(&memories);
    let scored = everything_scored(&mut store, "alpha alpha");

    let superseded: Vec<MemoryId> = memories[..5].iter().map(|memory| memory.id).collect();
    assert!(
        scored.iter().all(|(id, _)| !superseded.contains(id)),
        "no link of the chain but its tip may be returned"
    );
    assert!(
        scored.iter().any(|(id, _)| *id == tip),
        "and the tip itself must be"
    );

    let inherited = scored
        .iter()
        .find(|(id, _)| *id == distant.id)
        .map(|(_, inherited)| *inherited);
    // Reached, not *positively* reached: scores are centred cosines and the anchor sits below this
    // corpus's mean, so what it carries is negative. Unreached is reported as exactly zero, which is
    // the distinction that matters here — and a seed that carries a negative share to its
    // neighbours is a property of summing centred scores, recorded in `recall-mechanisms.md`.
    assert!(
        inherited.is_some_and(|score| score != 0.0),
        "the anchor had to seed the walk, which needs the chain to have spent one slot, got \
         {inherited:?}"
    );
}

/// **Keyword search does not traverse.** An exact match returns a set with no scores, so there is
/// no seed score for the walk to inherit and decay — the mechanism has nothing to operate on. And
/// the reason the keyword path exists is to surface only memories tagged a particular way, which
/// traversal would dilute.
#[test]
fn keyword_search_does_not_traverse() {
    let mut tagged = a_memory("alpha alpha alpha");
    tagged.keywords = vec![Keyword::new("routing").expect("valid keyword")];
    let mut neighbour = a_memory("beta beta beta");
    linking(&mut neighbour, LinkLabel::Supports, &tagged);

    let store = store_holding(&[tagged.clone(), neighbour]);
    let hits = store
        .search_keywords(&[Keyword::new("routing").expect("valid keyword")], 20)
        .expect("searches");

    let ids: Vec<MemoryId> = hits.iter().map(|hit| hit.memory.id).collect();
    assert_eq!(ids, vec![tagged.id], "only the memory carrying the keyword");
}

// ---------------------------------------------------------------------------
// Validity, settled before the ranking
// ---------------------------------------------------------------------------

/// **A supersession cycle is returned, and named.** Dropping it would hide a memory because its
/// apparatus is broken; treating it as current — which is what recall did until 2026-08-19 — hides
/// the one data error validity resolution is able to detect, in the situation where the caller is
/// least able to work it out.
#[test]
fn a_memory_in_a_supersession_cycle_comes_back_saying_so() {
    let mut store = a_store();
    let memories = nine_memories();
    let mut first = memories[MATCHED].clone();
    let mut second = a_memory("alpha alpha alpha as corrected");
    linking(&mut first, LinkLabel::Supersedes, &second);
    linking(&mut second, LinkLabel::Supersedes, &first);

    let mut all = vec![first.clone(), second.clone()];
    all.extend(memories.iter().skip(1).cloned());
    // Imported rather than inserted: writing a cycle is refused, which is the other half of the
    // rule, so only a hand-edited or merged file can deliver one.
    store
        .import(&MemorySet {
            scope: scope(),
            memories: all,
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("a corrupt file still imports");

    let hits = store
        .recall(&Question::new("alpha alpha", 20))
        .expect("recalls")
        .hits;

    let looped: Vec<&scatterhoard::store::Recalled> = hits
        .iter()
        .filter(|hit| hit.memory.id == first.id || hit.memory.id == second.id)
        .collect();
    assert!(
        !looped.is_empty(),
        "a memory whose history is broken is still a memory, and must come back"
    );
    assert!(
        looped.iter().all(|hit| hit.in_cycle),
        "and every one of them has to say its history cannot be resolved"
    );
    assert!(
        looped.iter().all(|hit| hit.matched.is_none()),
        "a cycle is not a substitution: nothing here can say what replaced what"
    );
}

/// The walk carries score to **tips**, so the ranking has to look it up by the identity it is
/// ranking, not by whichever id happened to match.
///
/// The case needs three things at once, which is why it went untested until 2026-08-19: a memory
/// that was superseded, whose *predecessor* is the better content match, and whose tip the walk
/// reaches from somewhere else. Get those together and the two lookups disagree — one finds the
/// inherited score, the other finds nothing.
#[test]
fn a_substituted_candidate_still_carries_what_the_walk_brought_its_tip() {
    let plain = nine_memories();
    let mut linked = plain.clone();

    let mut tip = a_memory("gamma gamma gamma as corrected");
    linking(&mut tip, LinkLabel::Supersedes, &plain[FAINTER_STILL]);
    // The seed points at the tip, so the walk reaches it while its predecessor is what a query
    // about `alpha` scores best of the pair.
    linking(&mut linked[MATCHED], LinkLabel::Supports, &tip);
    linked.push(tip.clone());

    let inherited = recalled_inherited(&mut store_holding(&linked), "alpha alpha", tip.id);

    assert!(
        inherited.is_some_and(|score| score != 0.0),
        "the tip is what the walk reached, so that is the score this candidate carries, got \
         {inherited:?}"
    );
}

/// A memory that says it was replaced by something this machine does not hold is **returned and
/// flagged**, not withheld.
///
/// **This reverses what this test asserted until 2026-08-23**, deliberately and on the human
/// partner's call, so the reversal is recorded here rather than the old test being quietly deleted.
/// The old reasoning was: it is known to be stale and the replacement cannot be read, so handing it
/// back as though it stood would be worse than returning nothing. What that missed is that
/// *withholding* is also an answer, and a worse-behaved one — it fails silently, in the ordinary
/// case rather than an exotic one, since a replacement's set is usually merely unloaded rather than
/// gone. A session was left unable to discover that the memory existed at all, so it could neither
/// read it nor go and find the replacement.
///
/// Returning it flagged gives the session the move that helps: the memory's own `SUPERSEDED_BY`
/// link names the set and the id, so it can load that set and read the replacement for itself. This
/// is the design's `report rather than resolve` principle applied to the last case in recall that
/// was still deciding on the caller's behalf.
///
/// `SUPERSEDED_BY` still exists for exactly this — a memory recording its own obsolescence when it
/// cannot write to the set that replaced it — and that is unchanged.
#[test]
fn a_memory_replaced_from_a_set_this_machine_lacks_is_returned_and_flagged() {
    let plain = nine_memories();
    let mut linked = plain.clone();
    linked[MATCHED].links.push(Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::CrossSet {
            set: SetName::new("elsewhere").expect("valid set name"),
            id: MemoryId::generate(),
            keywords: Vec::new(),
            set_id: None,
        },
    });

    let mut store = store_holding(&linked);
    let recall = store
        .recall(&Question::new("alpha alpha", 20))
        .expect("recalls");

    let hit = recall
        .hits
        .iter()
        .find(|hit| hit.memory.id == plain[MATCHED].id)
        .expect("the best content match was withheld, so nothing tells a session it exists");

    assert!(
        hit.superseded_beyond_this_set,
        "returned without the flag, so a memory known to be stale reads as current"
    );
    assert!(
        !hit.in_cycle,
        "not a cycle: conflating the two sends a session looking for a data error that is not there"
    );
    assert!(
        hit.matched.is_none(),
        "nothing was substituted for it -- there is no reachable tip to substitute"
    );
    assert!(
        recall.hits.len() > 1,
        "and the rest of the ranking still answers"
    );
}

/// The instrumented entry point, at its defaults, must be the shipping one.
///
/// `Stages` exists so the scorer's decisions — centring, the walk, the inherited blend, the gap cut
/// — can be turned off one at a time and their effect on a real ranking measured, which is what
/// `probe_the_effect_of_each_scoring_decision` does. That is only worth anything if the default
/// composition is *exactly* what ships, and this is the guard on that: a divergence here would mean
/// every ablation is measured against a baseline the product does not run.
///
/// Compares the whole result rather than the top hit, because the decisions being varied change the
/// number of results as well as their order.
#[test]
fn the_instrumented_scorer_at_its_defaults_matches_the_shipping_one() {
    let mut store = a_store();
    for content in [
        "alpha alpha alpha beta",
        "alpha beta gamma",
        "beta beta gamma",
        "gamma gamma gamma",
        "alpha gamma gamma beta",
        "beta gamma alpha alpha",
    ] {
        store.insert(&a_memory(content)).expect("inserts");
    }

    for query in ["alpha", "beta gamma", "alpha beta gamma", "gamma"] {
        for window in [3usize, 5, 10] {
            let question = Question::new(query, window);
            let shipping = recall_across(&mut [&mut store], &question).expect("recalls");

            let question = Question::new(query, window);
            let instrumented = recall_across_with(&mut [&mut store], &question, Stages::default())
                .expect("recalls");

            assert_eq!(
                shipping.hits.len(),
                instrumented.hits.len(),
                "result count diverged for {query:?} at window {window}"
            );
            for (a, b) in shipping.hits.iter().zip(&instrumented.hits) {
                assert_eq!(a.memory.id, b.memory.id, "order diverged for {query:?}");
                assert!(
                    (a.similarity - b.similarity).abs() < f32::EPSILON,
                    "similarity diverged for {query:?}"
                );
                assert!(
                    (a.inherited - b.inherited).abs() < f32::EPSILON,
                    "inherited score diverged for {query:?}"
                );
            }
        }
    }
}

/// **The floor reads raw content similarity, not the centred score the ranking uses.**
///
/// Centring removes the corpus centroid, which is exactly the direction that marked a query as out
/// of domain — so the centred score cannot answer *does anything here answer this*, and calibrated
/// on real stores it silenced 2.7% of off-domain probes where raw silenced 62.7%
/// (`documents/design/recall-mechanisms.md`). The fixture puts a floor between the two numbers: the
/// query embeds parallel to the first memory, so its raw cosine is 1 while its centred score is
/// about 0.95.
#[test]
fn the_floor_is_read_against_raw_similarity() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    store.insert(&a_memory("beta beta")).expect("inserts");

    let recall = recall_across_with(
        &mut [&mut store],
        &Question::new("alpha alpha alpha", 10),
        Stages {
            floor: Floor::Fixed(0.97),
            ..Stages::default()
        },
    )
    .expect("recalls");

    assert_eq!(
        recall.hits.len(),
        1,
        "a floor of 0.97 sits above the centred score and below the raw one, so exactly the \
         memory the query is parallel to should survive it"
    );
}

/// A floor removes what falls below it, and `None` removes nothing.
///
/// The floor is the scorer's only absolute question, so the two ends of it are what matter: a floor
/// above everything must return no hits at all — which is `nothing here answers this`, the outcome
/// `an-unanswerable-query-returns-confident-hits` exists for — and the shipping default must be
/// indistinguishable from having no floor.
#[test]
fn a_floor_removes_hits_below_it_and_none_removes_nothing() {
    let mut store = a_store();
    for content in ["alpha alpha alpha", "alpha beta gamma", "gamma gamma gamma"] {
        store.insert(&a_memory(content)).expect("inserts");
    }

    let unfloored = recall_across_with(
        &mut [&mut store],
        &Question::new("alpha", 10),
        Stages {
            floor: Floor::None,
            ..Stages::default()
        },
    )
    .expect("recalls");
    assert!(
        !unfloored.hits.is_empty(),
        "the query should match something"
    );

    let above_everything = recall_across_with(
        &mut [&mut store],
        &Question::new("alpha", 10),
        Stages {
            floor: Floor::Fixed(2.0),
            ..Stages::default()
        },
    )
    .expect("recalls");
    assert!(
        above_everything.hits.is_empty(),
        "a floor above every possible cosine returned {} hits",
        above_everything.hits.len()
    );

    let below_everything = recall_across_with(
        &mut [&mut store],
        &Question::new("alpha", 10),
        Stages {
            floor: Floor::Fixed(-2.0),
            ..Stages::default()
        },
    )
    .expect("recalls");
    assert_eq!(
        below_everything.hits.len(),
        unfloored.hits.len(),
        "a floor below every possible cosine changed the result set"
    );
}

/// Scanning only the tips changes which text a chain is found by.
///
/// A supersession chain is scored on the best evidence any member matched, so a query matching text
/// that was *replaced* still returns the replacement — deliberately, and reported as
/// `FoundBy::SemanticSubstituted`. `Stages::include_superseded` turns that off, so the chain is
/// found only by what it currently says.
///
/// This exists to measure what the superseded bodies contribute to a ranking, not because either
/// mode is wrong: the default is what ships and the flag is instrumentation.
#[test]
fn scanning_only_the_tips_scores_a_chain_on_its_current_text() {
    let mut store = a_store();
    let replaced = a_memory("alpha alpha alpha");
    store.insert(&replaced).expect("inserts");
    let mut tip = a_memory("gamma gamma gamma");
    tip.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(replaced.id),
    }];
    store.insert(&tip).expect("inserts");

    // Default: the chain is found by the text it used to carry, and says so.
    let shipping = recall_across_with(
        &mut [&mut store],
        &Question::new("alpha", 5),
        Stages::default(),
    )
    .expect("recalls");
    let hit = shipping
        .hits
        .iter()
        .find(|h| h.memory.id == tip.id)
        .expect("the tip is returned for the replaced text");
    assert_eq!(
        hit.matched,
        Some(replaced.id),
        "the score should be the ancestor's"
    );
    let borrowed = hit.similarity;

    // Tips only: the same query reaches the tip on its own text, or not at all.
    let tips_only = recall_across_with(
        &mut [&mut store],
        &Question::new("alpha", 5),
        Stages {
            include_superseded: false,
            ..Stages::default()
        },
    )
    .expect("recalls");
    if let Some(hit) = tips_only.hits.iter().find(|h| h.memory.id == tip.id) {
        assert_eq!(
            hit.matched, None,
            "nothing was substituted, so nothing should be named"
        );
        assert!(
            hit.similarity < borrowed,
            "the tip's own text should score below what it borrowed: {} vs {borrowed}",
            hit.similarity
        );
    }
}
