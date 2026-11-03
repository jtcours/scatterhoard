// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for vector storage and the similarity scan.
//!
//! None of this needs an embedding model: the vectors are written by hand, which is what makes the
//! properties assertable at all. Whether the *model* produces useful vectors was settled by the
//! spike (see `documents/design/embeddings.md`); what is tested here is that the arithmetic built
//! on top of it does what the design says — centroid removal happening at scan time rather than at
//! storage time, and a scan that cannot return a NaN.

use scatterhoard::embedding::{Vector, VectorSet};
use scatterhoard::memory::MemoryId;

fn vector(values: &[f32]) -> Vector {
    Vector::new(values.to_vec()).expect("a non-zero vector")
}

fn set_of(vectors: &[&[f32]]) -> (VectorSet, Vec<MemoryId>) {
    let mut set = VectorSet::new(vectors[0].len());
    let ids: Vec<MemoryId> = vectors.iter().map(|_| MemoryId::generate()).collect();
    for (id, values) in ids.iter().zip(vectors) {
        set.insert(*id, vector(values)).expect("inserts");
    }
    (set, ids)
}

// ---------------------------------------------------------------------------
// Vectors
// ---------------------------------------------------------------------------

/// A zero vector has no direction, so its cosine similarity to anything is a division by zero. It
/// is excluded at construction rather than special-cased at every comparison — which is what lets
/// the scan below sort without defending against NaN.
#[test]
fn a_vector_with_no_direction_is_rejected() {
    assert!(Vector::new(vec![0.0, 0.0, 0.0]).is_err());
    assert!(Vector::new(vec![]).is_err());
    assert!(Vector::new(vec![0.0, 1.0, 0.0]).is_ok());
}

#[test]
fn a_vector_rejects_non_finite_components() {
    assert!(Vector::new(vec![1.0, f32::NAN]).is_err());
    assert!(Vector::new(vec![1.0, f32::INFINITY]).is_err());
}

// ---------------------------------------------------------------------------
// The set
// ---------------------------------------------------------------------------

/// Every vector in a store comes from one model, so a differing dimension means the model changed
/// — which the design handles by re-embedding, not by comparing across dimensions.
#[test]
fn a_vector_of_the_wrong_dimension_is_refused() {
    let mut set = VectorSet::new(3);
    assert!(
        set.insert(MemoryId::generate(), vector(&[1.0, 0.0, 0.0]))
            .is_ok()
    );
    assert!(
        set.insert(MemoryId::generate(), vector(&[1.0, 0.0]))
            .is_err()
    );
}

/// The centroid is asserted, not just the count. The set keeps a running componentwise sum so the
/// centroid costs O(d) rather than a second pass, and that sum is the thing a replace has to fix up
/// — a count and a ranking both look right while the sum still carries the old vector.
#[test]
fn inserting_the_same_memory_twice_replaces_its_vector() {
    let id = MemoryId::generate();
    let mut set = VectorSet::new(2);
    set.insert(id, vector(&[1.0, 0.0])).expect("inserts");
    set.insert(id, vector(&[0.0, 1.0])).expect("replaces");

    assert_eq!(set.len(), 1);
    assert_eq!(
        set.centroid().expect("a centroid").components(),
        &[0.0, 1.0],
        "the replaced vector is still contributing to the running sum"
    );
    assert_eq!(set.nearest(&vector(&[0.0, 1.0]), 1)[0].id, id);
}

#[test]
fn removing_a_memory_removes_its_vector() {
    let (mut set, ids) = set_of(&[&[1.0, 0.0], &[0.0, 4.0]]);
    set.remove(ids[0]);

    assert_eq!(set.len(), 1);
    assert_eq!(
        set.centroid().expect("a centroid").components(),
        &[0.0, 4.0],
        "the removed vector is still contributing to the running sum"
    );
    assert!(
        set.nearest(&vector(&[1.0, 0.0]), 5)
            .iter()
            .all(|h| h.id != ids[0])
    );
}

/// The centroid is the mean of the stored vectors, and it moves as memories are added. This is why
/// the design stores raw vectors and subtracts at scan time: centring on write would mean every
/// stored vector went stale the moment another memory arrived.
#[test]
fn the_centroid_is_the_mean_and_moves_as_memories_arrive() {
    let mut set = VectorSet::new(2);
    set.insert(MemoryId::generate(), vector(&[2.0, 0.0]))
        .expect("inserts");
    assert_eq!(
        set.centroid().expect("a centroid").components(),
        &[2.0, 0.0]
    );

    set.insert(MemoryId::generate(), vector(&[0.0, 4.0]))
        .expect("inserts");
    assert_eq!(
        set.centroid().expect("a centroid").components(),
        &[1.0, 2.0]
    );
}

#[test]
fn an_empty_set_has_no_centroid_and_no_hits() {
    let set = VectorSet::new(3);
    assert!(set.centroid().is_none());
    assert!(set.nearest(&vector(&[1.0, 0.0, 0.0]), 5).is_empty());
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

#[test]
fn the_scan_ranks_by_similarity_and_returns_at_most_k() {
    // Three directions, deliberately spread so that centring cannot reorder them.
    let (set, ids) = set_of(&[&[1.0, 0.0], &[0.9, 0.44], &[-1.0, 0.0]]);

    let hits = set.nearest(&vector(&[1.0, 0.0]), 2);
    assert_eq!(hits.len(), 2, "k caps the number of hits");
    assert_eq!(hits[0].id, ids[0]);
    assert_eq!(hits[1].id, ids[1]);
    assert!(
        hits[0].similarity >= hits[1].similarity,
        "hits must come back in descending similarity"
    );
}

#[test]
fn asking_for_more_hits_than_exist_returns_what_there_is() {
    let (set, _) = set_of(&[&[1.0, 0.0], &[0.0, 1.0]]);
    assert_eq!(set.nearest(&vector(&[1.0, 0.0]), 50).len(), 2);
}

/// Cosine compares direction, so scaling a query must not change its raw score.
///
/// Only the *uncentred* scan has this property, and the distinction is worth pinning. Subtracting a
/// fixed centroid from a scaled query changes the resulting direction, so centred scores are not
/// scale-invariant. That costs nothing in practice — queries come from the embedder with its own
/// magnitudes, never rescaled by hand — but it means magnitude cannot be assumed irrelevant once
/// centring is in play.
#[test]
fn raw_similarity_ignores_magnitude() {
    let (set, ids) = set_of(&[&[1.0, 0.0], &[0.0, 1.0]]);

    let near = set.nearest_uncentred(&vector(&[1.0, 0.0]), 1);
    let far = set.nearest_uncentred(&vector(&[1000.0, 0.0]), 1);

    assert_eq!(near[0].id, ids[0]);
    assert_eq!(far[0].id, ids[0]);
    assert!((near[0].similarity - far[0].similarity).abs() < 1e-6);
}

/// The measured effect from the spike, in miniature. Three memories crowd one direction and one
/// sits apart; against a query nearer the crowd, the raw scan puts a member of the crowd first,
/// while removing the corpus mean lets the distinctive memory win. That is what centroid removal
/// buys — it stops a memory dense in common vocabulary from matching everything.
#[test]
fn centring_the_query_and_the_memories_changes_the_ranking() {
    let (set, ids) = set_of(&[
        &[1.0, 0.02],
        &[1.0, 0.0],
        &[1.0, -0.02],
        &[0.6, 0.8], // the distinctive one
    ]);
    let query = vector(&[1.0, 0.45]);

    let raw = set.nearest_uncentred(&query, 4);
    let centred = set.nearest(&query, 4);

    assert_eq!(
        raw[0].id, ids[0],
        "raw cosine should be won by a member of the crowd"
    );
    assert_eq!(
        centred[0].id, ids[3],
        "centring should surface the memory that is not in the crowd"
    );
}

/// Similarity feeds ranking, so it has to be totally ordered. Zero vectors are excluded at
/// construction and non-finite components are refused, which together make a NaN score
/// unreachable — this asserts the consequence rather than the mechanism.
#[test]
fn no_score_is_ever_nan() {
    let (set, _) = set_of(&[&[1.0, 0.0], &[0.0, 1.0], &[-1.0, -1.0]]);

    for hit in set.nearest(&vector(&[1.0, 1.0]), 10) {
        assert!(hit.similarity.is_finite(), "similarity was not finite");
    }
}

/// Centring subtracts the mean from both sides, and a set holding one memory has that memory *as*
/// its mean — so the centred vector is zero and no direction survives to compare. It must degrade
/// to a usable score rather than producing a NaN.
#[test]
fn a_single_memory_set_still_scans() {
    let (set, ids) = set_of(&[&[1.0, 2.0, 3.0]]);
    let hits = set.nearest(&vector(&[1.0, 2.0, 3.0]), 5);

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, ids[0]);
    assert!(hits[0].similarity.is_finite());
}

/// A stored vector can be read back by id.
///
/// Needed because a caller ranking one memory against the rest — the evaluation harness does
/// exactly this — has to supply that memory's own vector as the query. Without an accessor the only
/// way to get it is to keep a second copy alongside the set, which is the two-facts-that-can-drift
/// failure this project keeps filing.
#[test]
fn a_stored_vector_can_be_read_back_by_id() {
    let (set, ids) = set_of(&[&[1.0, 0.0], &[0.0, 1.0]]);

    assert_eq!(set.get(ids[0]), Some(vector(&[1.0, 0.0])));
    assert_eq!(set.get(ids[1]), Some(vector(&[0.0, 1.0])));
    assert_eq!(set.get(MemoryId::generate()), None);
}

/// Reading back what was removed returns nothing, rather than a stale vector the running sum no
/// longer accounts for.
#[test]
fn a_removed_vector_cannot_be_read_back() {
    let (mut set, ids) = set_of(&[&[1.0, 0.0], &[0.0, 1.0]]);
    set.remove(ids[0]);

    assert_eq!(set.get(ids[0]), None);
    assert_eq!(set.get(ids[1]), Some(vector(&[0.0, 1.0])));
}
