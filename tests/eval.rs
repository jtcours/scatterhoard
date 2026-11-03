// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the evaluation harness.
//!
//! None of this needs an embedding model. The harness takes vectors it is handed, which is what
//! makes the measurement assertable: whether the *model* produces useful vectors is a separate
//! question, and the binary that loads one is deliberately a thin shell over what is tested here.
//!
//! The property under test throughout is the one the measurement exists to report — a **percentile**
//! rather than a rank. `documents/design/recall-mechanisms.md` and the `recall-eval-harness` entry
//! in TRACKING.md both turn on that distinction: at 70 memories a linked target ranking 2nd looks
//! like traversal is redundant, and the same percentile at 1,000 memories ranks 29th and looks
//! essential.

use scatterhoard::embedding::{Vector, VectorSet};
use scatterhoard::eval::{self, Corpus};
use std::str::FromStr;

use std::collections::HashMap;

use jiff::Timestamp;
use scatterhoard::memory::{
    Attribution, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId, MemoryText,
    Origin, SetName,
};

fn attribution() -> Attribution {
    Attribution {
        by: MemoryId::generate(),
        at: Timestamp::from_str("2026-08-15T09:00:00Z").expect("a valid timestamp"),
    }
}

/// A memory with the links it authors. Content is unique per id so nothing collides by accident.
fn memory_with(id: MemoryId, links: Vec<Link>) -> Memory {
    Memory {
        id,
        content: MemoryText::new(format!("content of {id}")).expect("non-empty"),
        why: None,
        context: None,
        keywords: vec![],
        confidence: Confidence::new(0.9).expect("in range"),
        origin: Origin::Joint,
        created: attribution(),
        modified: attribution(),
        links,
    }
}

fn same_set(label: LinkLabel, target: MemoryId) -> Link {
    Link {
        label,
        target: LinkTarget::SameSet(target),
    }
}

fn vector(values: &[f32]) -> Vector {
    Vector::new(values.to_vec()).expect("a non-zero vector")
}

/// Build a corpus whose vectors are laid out so that similarity to the first memory is a known,
/// strictly decreasing function of position. Memory 0 is the source; memory `i` sits at angle
/// proportional to `i`, so ranking against memory 0 recovers the insertion order exactly.
fn ladder(n: usize) -> (Vec<MemoryId>, VectorSet) {
    let ids: Vec<MemoryId> = (0..n).map(|_| MemoryId::generate()).collect();
    let mut set = VectorSet::new(2);
    for (i, id) in ids.iter().enumerate() {
        // cos/sin of a small increasing angle: strictly decreasing similarity to index 0.
        #[allow(clippy::cast_precision_loss)]
        let theta = (i as f32) * 0.01;
        set.insert(*id, vector(&[theta.cos(), theta.sin()]))
            .expect("inserts");
    }
    (ids, set)
}

// ---------------------------------------------------------------------------
// What counts as a traversable edge
// ---------------------------------------------------------------------------

/// Validity resolution never traverses supersession, so those edges are not part of the question
/// this harness asks. Counting them would mix a *validity* relation into a measurement about
/// *relevance*, and they are the most content-similar edges in any corpus — a superseding memory is
/// supposed to resemble its predecessor — so including them would flatter the result specifically.
#[test]
fn supersession_edges_are_not_traversable() {
    let (ids, vectors) = ladder(4);
    let memories = vec![
        memory_with(
            ids[0],
            vec![
                same_set(LinkLabel::Supersedes, ids[1]),
                same_set(LinkLabel::SupersededBy, ids[2]),
                same_set(LinkLabel::Supports, ids[3]),
            ],
        ),
        memory_with(ids[1], vec![]),
        memory_with(ids[2], vec![]),
        memory_with(ids[3], vec![]),
    ];

    let corpus = Corpus::new(memories, vectors);
    let reaches = eval::link_reach(&corpus);

    assert_eq!(reaches.len(), 1, "only the SUPPORTS edge is traversable");
    assert_eq!(reaches[0].label, LinkLabel::Supports);
    assert_eq!(reaches[0].target, ids[3]);
}

/// A link whose target is not in the corpus cannot be measured — there is nothing to rank. It is
/// skipped and *counted*, rather than dropped silently, because a corpus quietly measuring half its
/// edges would report a percentile distribution over a population nobody chose.
#[test]
fn a_dangling_target_is_skipped_and_counted() {
    let (ids, vectors) = ladder(3);
    let absent = MemoryId::generate();
    let memories = vec![
        memory_with(
            ids[0],
            vec![
                same_set(LinkLabel::Supports, ids[1]),
                same_set(LinkLabel::Supports, absent),
            ],
        ),
        memory_with(ids[1], vec![]),
        memory_with(ids[2], vec![]),
    ];

    let corpus = Corpus::new(memories, vectors);
    assert_eq!(eval::link_reach(&corpus).len(), 1);
    assert_eq!(eval::unmeasurable_edges(&corpus), 1);
}

/// A cross-set link names a set this corpus is not, so its target is unreachable here for the same
/// reason a dangling one is. The harness must not silently resolve it by id and pretend the set name
/// was not there.
#[test]
fn a_cross_set_target_is_not_resolved_by_id_alone() {
    let (ids, vectors) = ladder(3);
    let memories = vec![
        memory_with(
            ids[0],
            vec![Link {
                label: LinkLabel::Supports,
                target: LinkTarget::CrossSet {
                    set: SetName::new("elsewhere").expect("a valid set name"),
                    // Deliberately an id this corpus *does* hold.
                    id: ids[1],
                    keywords: vec![Keyword::new("k").expect("a valid keyword")],
                    set_id: None,
                },
            }],
        ),
        memory_with(ids[1], vec![]),
        memory_with(ids[2], vec![]),
    ];

    let corpus = Corpus::new(memories, vectors);
    assert!(eval::link_reach(&corpus).is_empty());
    assert_eq!(eval::unmeasurable_edges(&corpus), 1);
}

// ---------------------------------------------------------------------------
// The measurement itself
// ---------------------------------------------------------------------------

/// The rank is the target's position when every *other* memory is ranked by similarity to the
/// source. With the ladder layout, memory `i` is the `i`th most similar to memory 0, so a link from
/// 0 to `i` must report rank `i`.
#[test]
fn rank_is_the_targets_position_from_the_source() {
    let (ids, vectors) = ladder(20);
    let memories: Vec<Memory> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            if i == 0 {
                memory_with(*id, vec![same_set(LinkLabel::Supports, ids[7])])
            } else {
                memory_with(*id, vec![])
            }
        })
        .collect();

    let reaches = eval::link_reach(&Corpus::new(memories, vectors));
    assert_eq!(reaches.len(), 1);
    assert_eq!(reaches[0].rank, 7);
}

/// **The property the whole harness exists for.** The same relationship measured in two corpora of
/// different sizes must report the same percentile and a different rank. Reporting rank alone is
/// the trap this guards: it makes traversal look redundant at small N and essential at large N for
/// no reason but N.
#[test]
fn percentile_is_scale_invariant_where_rank_is_not() {
    // A target one tenth of the way down the ranking, in two corpora an order of magnitude apart.
    let small = {
        let (ids, vectors) = ladder(50);
        let memories: Vec<Memory> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                if i == 0 {
                    memory_with(*id, vec![same_set(LinkLabel::Supports, ids[5])])
                } else {
                    memory_with(*id, vec![])
                }
            })
            .collect();
        eval::link_reach(&Corpus::new(memories, vectors))
    };
    let large = {
        let (ids, vectors) = ladder(500);
        let memories: Vec<Memory> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                if i == 0 {
                    memory_with(*id, vec![same_set(LinkLabel::Supports, ids[50])])
                } else {
                    memory_with(*id, vec![])
                }
            })
            .collect();
        eval::link_reach(&Corpus::new(memories, vectors))
    };

    assert_eq!(small[0].rank, 5);
    assert_eq!(large[0].rank, 50);
    // 5/50 and 50/500 are the same tenth.
    assert!((small[0].percentile - large[0].percentile).abs() < 1e-6);
    assert!((small[0].percentile - 0.10).abs() < 1e-6);
}

/// The percentile is the rank as a fraction of the whole corpus, matching the figure the design
/// records. Dividing by anything else — the ranked population, which excludes the source — would
/// make the published 2.9% and the 170-memory crossover it implies incomparable with later runs.
#[test]
fn percentile_divides_by_the_corpus_size() {
    let (ids, vectors) = ladder(100);
    let memories: Vec<Memory> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            if i == 0 {
                memory_with(*id, vec![same_set(LinkLabel::Supports, ids[3])])
            } else {
                memory_with(*id, vec![])
            }
        })
        .collect();

    let reaches = eval::link_reach(&Corpus::new(memories, vectors));
    assert_eq!(reaches[0].rank, 3);
    // 3/100, not 3/99.
    assert!((reaches[0].percentile - 0.03).abs() < 1e-9);
}

/// Scoring happens with the corpus centroid removed, as recall does. This is not cosmetic: centroid
/// removal is what lifted recall@5 from 71.7% to 77.5% in the spike and dropped the worst hub from
/// 13 of 20 queries to 6, so a harness scoring raw cosine would be measuring a pipeline the product
/// does not run.
#[test]
fn scoring_removes_the_corpus_centroid() {
    // Three vectors sharing a large common component, chosen so that the two scorings disagree in
    // *sign* rather than merely in magnitude: centred, the source and target sit on opposite sides
    // of the mean at −0.55; uncentred, the shared component dominates and they read as +0.97.
    //
    // The similarity is what is asserted, not the rank. An earlier version of this test asserted
    // the rank and a similarity above 0.9, and mutation testing caught it as vacuous — the target
    // ranked first and scored ~1.0 under *both* scorings, so the test passed with centroid removal
    // deleted. Sign is what separates them here.
    let ids: Vec<MemoryId> = (0..3).map(|_| MemoryId::generate()).collect();
    let mut vectors = VectorSet::new(2);
    vectors
        .insert(ids[0], vector(&[10.0, 2.0]))
        .expect("inserts");
    vectors
        .insert(ids[1], vector(&[11.0, -1.0]))
        .expect("inserts");
    vectors
        .insert(ids[2], vector(&[9.0, -0.5]))
        .expect("inserts");

    let memories = vec![
        memory_with(ids[0], vec![same_set(LinkLabel::Supports, ids[2])]),
        memory_with(ids[1], vec![]),
        memory_with(ids[2], vec![]),
    ];

    let reaches = eval::link_reach(&Corpus::new(memories, vectors));
    assert!(
        reaches[0].similarity < 0.0,
        "centred similarity is -0.55; scoring uncentred would report +0.97, and did — was {}",
        reaches[0].similarity
    );
    assert!(
        (reaches[0].similarity - -0.5547).abs() < 1e-3,
        "expected the centred cosine, was {}",
        reaches[0].similarity
    );
}

// ---------------------------------------------------------------------------
// Summarising a run
// ---------------------------------------------------------------------------

/// The median is what the design reports, and it is taken over the measured edges rather than over
/// all of them — an unmeasurable edge has no percentile to contribute, and counting it as anything
/// would invent a number. The unmeasurable count still travels in the summary, so a run that could
/// only measure half its edges says so.
#[test]
fn the_summary_median_is_over_measured_edges() {
    let (ids, vectors) = ladder(20);
    let absent = MemoryId::generate();
    let memories: Vec<Memory> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| match i {
            0 => memory_with(
                *id,
                vec![
                    same_set(LinkLabel::Supports, ids[2]),
                    same_set(LinkLabel::Supports, ids[4]),
                    same_set(LinkLabel::Supports, ids[6]),
                    // Unmeasurable: it must not be counted as a percentile of any value.
                    same_set(LinkLabel::Supports, absent),
                ],
            ),
            _ => memory_with(*id, vec![]),
        })
        .collect();

    let corpus = Corpus::new(memories, vectors);
    let summary = eval::summarise(&corpus);

    assert_eq!(summary.measured, 3);
    assert_eq!(summary.unmeasurable, 1);
    assert_eq!(summary.corpus_size, 20);
    // Ranks 2, 4 and 6 over 20 memories: the median is 4/20.
    assert!(
        (summary.median_percentile - 0.20).abs() < 1e-9,
        "expected 0.20, got {}",
        summary.median_percentile
    );
}

/// The crossover is the corpus size at which the median linked target stops falling inside a top-k
/// cut. It is derived from the measured median rather than restated, because it is the number the
/// whole size argument rests on: at 2.9% and k=5 it lands near 170 memories.
#[test]
fn the_crossover_follows_from_the_median_percentile() {
    let summary = eval::Summary {
        measured: 1,
        unmeasurable: 0,
        median_percentile: 0.029,
        corpus_size: 70,
    };
    let crossover = summary.crossover_at(5).expect("a positive median");
    assert!(
        (169..=173).contains(&crossover),
        "expected the recorded ~170, got {crossover}"
    );
}

/// A corpus whose median target already falls inside the cut cannot distinguish traversal from a
/// larger `k`, and the harness has to say so rather than leaving a reader to do the arithmetic. This
/// is the finding that sent the corpus from 56 memories to 429.
#[test]
fn a_corpus_below_its_crossover_reports_that_it_cannot_discriminate() {
    let below = eval::Summary {
        measured: 23,
        unmeasurable: 0,
        median_percentile: 0.029,
        corpus_size: 70,
    };
    let above = eval::Summary {
        measured: 451,
        unmeasurable: 0,
        median_percentile: 0.029,
        corpus_size: 429,
    };

    assert!(!below.discriminates_at(5));
    assert!(above.discriminates_at(5));
}

/// The share of targets a top-`k` cut would miss is the directly useful number, and it is not
/// recoverable from the median: a distribution can sit inside the cut at its median and still put a
/// large minority well outside it. That minority is the population traversal exists to reach, so it
/// is reported rather than inferred.
#[test]
fn the_share_outside_the_cut_is_not_the_median() {
    let (ids, vectors) = ladder(100);
    // Four links: three well inside a top-5 cut, one far outside it. The median rank is 3, so a
    // reader working from the median alone would conclude the cut loses nothing.
    let memories: Vec<Memory> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| match i {
            0 => memory_with(
                *id,
                vec![
                    same_set(LinkLabel::Supports, ids[1]),
                    same_set(LinkLabel::Supports, ids[2]),
                    same_set(LinkLabel::Supports, ids[4]),
                    same_set(LinkLabel::Supports, ids[80]),
                ],
            ),
            _ => memory_with(*id, vec![]),
        })
        .collect();

    let corpus = Corpus::new(memories, vectors);
    let reaches = eval::link_reach(&corpus);

    assert!((eval::share_outside_cut(&reaches, 5) - 0.25).abs() < 1e-9);
    // Every one of them is outside a cut of one, and none is outside a cut of the whole corpus.
    assert!((eval::share_outside_cut(&reaches, 1) - 0.75).abs() < 1e-9);
    assert!(eval::share_outside_cut(&reaches, 100).abs() < 1e-9);
}

/// Ranking labelled memories against a real query is a different measurement from ranking a link's
/// target against its source, and the difference is what gets excluded. A link's source stands in
/// for the query, so it is dropped from its own ranking; a real query is not a member of the corpus,
/// so nothing is dropped and every memory competes.
///
/// This is the diagnostic that says whether a labelling collapsed into the mechanism being tested.
/// If every labelled memory ranks inside the top few by content alone, the labeller was
/// pattern-matching on vocabulary and traversal's distinctive contribution is invisible by
/// construction.
#[test]
fn ranks_from_a_query_exclude_nothing() {
    let (ids, vectors) = ladder(20);
    let memories: Vec<Memory> = ids.iter().map(|id| memory_with(*id, vec![])).collect();
    let corpus = Corpus::new(memories, vectors);

    // A query sitting exactly where memory 0 sits. Memory 0 is then rank 1, so memory 3 is rank 4 —
    // where `link_reach` would call it rank 3, having removed the source.
    let query = vector(&[1.0, 0.0]);
    let ranks = eval::ranks_from_query(&corpus, &query, &[ids[3], ids[7]]);

    assert_eq!(ranks, vec![4, 8]);
}

/// An id that is not in the corpus has no rank. It is skipped rather than reported as last, because
/// a fabricated rank would quietly enter the distribution this diagnostic exists to read.
///
/// Uses a corpus large enough for the ordering to be stable. A five-memory ladder does not order as
/// its layout suggests, because centroid removal distorts a corpus that small — the same effect
/// recorded in `TRACKING.md`, where a one-memory store scores itself 0.0.
#[test]
fn an_unknown_id_has_no_rank() {
    let (ids, vectors) = ladder(20);
    let memories: Vec<Memory> = ids.iter().map(|id| memory_with(*id, vec![])).collect();
    let corpus = Corpus::new(memories, vectors);

    let ranks = eval::ranks_from_query(
        &corpus,
        &vector(&[1.0, 0.0]),
        &[ids[2], MemoryId::generate(), ids[4]],
    );
    assert_eq!(ranks, vec![3, 5]);
}

/// Ranking against a query removes the centroid, as recall does.
///
/// The ladder used elsewhere cannot show this: its vectors sit at increasing angles, and centring
/// preserves that ordering, so a test built on it passes with centroid removal deleted. Mutation
/// testing caught exactly that. This uses a **hub** instead — a memory sitting on the centroid, so
/// its centred vector has no direction left and it scores zero rather than near one.
///
/// Uncentred, the hub is the best match for a query near it. Centred, it falls behind a memory that
/// actually points the query's way. That inversion is the thing being asserted, and it is the same
/// hubness effect centroid removal was adopted to fix: one memory dense in common vocabulary
/// appearing in the top 5 for 13 of 20 queries.
#[test]
fn ranking_against_a_query_removes_the_centroid() {
    let ids: Vec<MemoryId> = (0..3).map(|_| MemoryId::generate()).collect();
    let mut vectors = VectorSet::new(2);
    vectors
        .insert(ids[0], vector(&[1.0, 1.0]))
        .expect("inserts");
    vectors
        .insert(ids[1], vector(&[1.0, -1.0]))
        .expect("inserts");
    // Exactly the mean of the three, so centring leaves it with no direction.
    vectors
        .insert(ids[2], vector(&[1.0, 0.0]))
        .expect("inserts");

    let memories: Vec<Memory> = ids.iter().map(|id| memory_with(*id, vec![])).collect();
    let corpus = Corpus::new(memories, vectors);

    // A query sitting close to the hub. Uncentred it scores 0.995 against the hub and 0.774 against
    // the first memory, so the hub would rank 1 and the first memory 2.
    let ranks = eval::ranks_from_query(&corpus, &vector(&[1.0, 0.1]), &[ids[0], ids[2]]);

    assert_eq!(
        ranks,
        vec![1, 2],
        "centred, the hub falls behind; uncentred it would lead, giving [2, 1]"
    );
}

// ---------------------------------------------------------------------------
// The walk, and how a reached memory's two scores combine
// ---------------------------------------------------------------------------

/// A path of memories, each linked to the next, laid out so that content similarity to the first is
/// strictly decreasing. Traversal from memory 0 should reach the far end; content alone should not.
fn chain(n: usize) -> (Vec<MemoryId>, Vec<Memory>, VectorSet) {
    let (ids, vectors) = ladder(n);
    let memories: Vec<Memory> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let links = if i + 1 < n {
                vec![same_set(LinkLabel::Supports, ids[i + 1])]
            } else {
                vec![]
            };
            memory_with(*id, links)
        })
        .collect();
    (ids, memories, vectors)
}

/// With no traversal, the ranking is exactly the content ranking. This is the baseline every
/// combination rule is measured against, so if it drifted from a plain scan the comparison would be
/// against something the product does not do.
#[test]
fn own_only_is_the_plain_content_ranking() {
    let (ids, memories, vectors) = chain(20);
    let corpus = Corpus::new(memories, vectors);
    let query = vector(&[1.0, 0.0]);

    let ranked = eval::rank(&corpus, &query, &eval::Walk::none(), eval::Combine::OwnOnly);
    let expected: Vec<MemoryId> = eval::ranks_from_query(&corpus, &query, &ids)
        .iter()
        .zip(&ids)
        .map(|(_, id)| *id)
        .collect();

    assert_eq!(ranked[..5], expected[..5]);
}

/// Expansion is uniform and symmetric: every label crossed the same way, in both directions. A walk
/// that only followed edges forwards would miss the memory that cites the seed, which is half of
/// what an authored link means.
#[test]
fn the_walk_crosses_edges_in_both_directions() {
    let (ids, vectors) = ladder(30);
    // Only memory 29 links, and it links *back* to memory 1. Seeding on memory 1 must still reach
    // 29, which content similarity puts far away.
    let memories: Vec<Memory> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            if i == 29 {
                memory_with(*id, vec![same_set(LinkLabel::Supports, ids[1])])
            } else {
                memory_with(*id, vec![])
            }
        })
        .collect();
    let corpus = Corpus::new(memories, vectors);

    let walk = eval::Walk {
        seeds: 3,
        hops: 1,
        decay: 0.8,
        frontier: 10,
    };
    let reached = eval::reachable(&corpus, &vector(&[1.0, 0.0]), &walk);

    assert!(
        reached.contains_key(&ids[29]),
        "an inbound edge must be walked; only outbound would strand the citing memory"
    );
}

/// A memory further from a seed inherits less. Without decay the walk would hand its full seed score
/// to everything it touched, and a bound on expansion would be the only thing keeping a large
/// component from flooding the result.
#[test]
fn inherited_score_decays_with_distance() {
    let (ids, memories, vectors) = chain(20);
    let corpus = Corpus::new(memories, vectors);

    let walk = eval::Walk {
        seeds: 1,
        hops: 3,
        decay: 0.5,
        frontier: 10,
    };
    let reached = eval::reachable(&corpus, &vector(&[1.0, 0.0]), &walk);

    let one = reached[&ids[1]];
    let two = reached[&ids[2]];
    let three = reached[&ids[3]];
    assert!(one > two && two > three, "got {one}, {two}, {three}");
    assert!(
        (two / one - 0.5).abs() < 1e-6,
        "each hop should halve the inherited score"
    );
}

/// The bound is on **expansion**, never on inclusion. Reaching a heavily-cited memory is cheap and
/// often the most useful thing returned; expanding *through* it is what branches. A cap on inclusion
/// would discard good results to solve a problem they did not cause.
#[test]
fn the_bound_limits_expansion_not_inclusion() {
    // A hub linked to many memories. With a frontier of 1 the walk may expand through the hub only,
    // but everything the hub reaches is still admitted.
    let (ids, vectors) = ladder(30);
    let mut memories: Vec<Memory> = ids.iter().map(|id| memory_with(*id, vec![])).collect();
    memories[1] = memory_with(
        ids[1],
        (10..30)
            .map(|i| same_set(LinkLabel::Supports, ids[i]))
            .collect(),
    );
    let corpus = Corpus::new(memories, vectors);

    let walk = eval::Walk {
        seeds: 2,
        hops: 1,
        decay: 0.9,
        frontier: 1,
    };
    let reached = eval::reachable(&corpus, &vector(&[1.0, 0.0]), &walk);

    assert!(
        reached.len() >= 20,
        "expanding through one hub admits everything it reaches, got {}",
        reached.len()
    );
}

/// The rules have to actually differ, or comparing them is theatre. Given one memory with a high
/// inherited score and low own score, and another the reverse, the orderings must come apart.
#[test]
fn the_combination_rules_are_not_the_same_function() {
    let (_ids, memories, vectors) = chain(20);
    let corpus = Corpus::new(memories, vectors);
    let query = vector(&[1.0, 0.0]);
    let walk = eval::Walk {
        seeds: 1,
        hops: 4,
        decay: 0.9,
        frontier: 5,
    };

    let orderings: Vec<Vec<MemoryId>> = [
        eval::Combine::OwnOnly,
        eval::Combine::Max,
        eval::Combine::Sum,
        eval::Combine::Blend(0.8),
    ]
    .into_iter()
    .map(|rule| eval::rank(&corpus, &query, &walk, rule))
    .collect();

    assert_ne!(
        orderings[0], orderings[2],
        "sum must differ from the baseline"
    );
    assert_ne!(orderings[1], orderings[2], "max must differ from sum");
}

/// Recall at `k` is the share of a query's relevant memories appearing in the top `k`.
#[test]
fn recall_at_k_is_the_share_of_relevant_found() {
    let ids: Vec<MemoryId> = (0..10).map(|_| MemoryId::generate()).collect();
    let relevant = vec![ids[0], ids[5], ids[9]];

    assert!((eval::recall_at(&ids, &relevant, 1) - 1.0 / 3.0).abs() < 1e-9);
    assert!((eval::recall_at(&ids, &relevant, 6) - 2.0 / 3.0).abs() < 1e-9);
    assert!((eval::recall_at(&ids, &relevant, 10) - 1.0).abs() < 1e-9);
    // No relevant memories is not zero recall; it is an undefined question, and averaging a zero
    // into a mean would silently penalise every mechanism for a query nobody could answer.
    assert_eq!(eval::recall_at(&ids, &[], 5), 0.0);
}

// ---------------------------------------------------------------------------
// Knowing when to stop returning memories
// ---------------------------------------------------------------------------

/// Precision at `k` is the share of what was returned that was actually wanted. Recall alone cannot
/// choose a cut-off, because returning the whole corpus scores a perfect 100%.
#[test]
fn precision_at_k_is_the_share_of_returned_that_was_wanted() {
    let ids: Vec<MemoryId> = (0..10).map(|_| MemoryId::generate()).collect();
    let relevant = vec![ids[0], ids[1], ids[7]];

    assert!((eval::precision_at(&ids, &relevant, 2) - 1.0).abs() < 1e-9);
    assert!((eval::precision_at(&ids, &relevant, 4) - 0.5).abs() < 1e-9);
    assert!((eval::precision_at(&ids, &relevant, 10) - 0.3).abs() < 1e-9);
}

/// The cut falls at the largest drop between consecutive scores.
///
/// A **gap** is the one shape that survives what this scoring does to a threshold. Scores are
/// centred cosines, so they are freely negative and a ratio to the top hit is meaningless when the
/// top hit scores below zero. A gap is a *difference*, so it is unmoved by translation, and gaps are
/// compared against each other rather than against a constant, so it is unmoved by scale. That is
/// what lets a cut-off exist at all given `no-non-match-floor` — no absolute number is defensible,
/// and this needs none.
#[test]
fn the_cut_falls_at_the_largest_drop() {
    let ids: Vec<MemoryId> = (0..6).map(|_| MemoryId::generate()).collect();
    // Three strong hits, then a cliff.
    let scored: Vec<(MemoryId, f64)> = ids
        .iter()
        .zip([0.90, 0.88, 0.85, 0.20, 0.18, 0.17])
        .map(|(id, score)| (*id, score))
        .collect();

    assert_eq!(eval::cut_at_largest_gap(&scored, 6), 3);
}

/// Translating and scaling every score leaves the cut where it was. Without that the rule would
/// need a constant, and a constant is what the cold-start problem makes indefensible: a store with
/// few memories has scores whose absolute values mean nothing.
#[test]
fn the_cut_is_unmoved_by_translation_and_scale() {
    let ids: Vec<MemoryId> = (0..6).map(|_| MemoryId::generate()).collect();
    let base = [0.90, 0.88, 0.85, 0.20, 0.18, 0.17];

    let plain: Vec<(MemoryId, f64)> = ids.iter().zip(base).map(|(i, s)| (*i, s)).collect();
    // Shifted well below zero, which a ratio-to-top rule could not survive.
    let shifted: Vec<(MemoryId, f64)> = ids
        .iter()
        .zip(base)
        .map(|(i, s)| (*i, 3.0 * s - 4.0))
        .collect();

    assert_eq!(
        eval::cut_at_largest_gap(&plain, 6),
        eval::cut_at_largest_gap(&shifted, 6)
    );
    assert!(
        shifted[0].1 < 0.0,
        "the shifted top hit must be negative to test the point"
    );
}

/// A ranking that falls away evenly has no cliff to find, so the rule must not invent one at
/// position 1. It returns the window it was given, and the caller's bound is what stops it.
#[test]
fn an_even_decline_is_not_cut_at_the_top() {
    let ids: Vec<MemoryId> = (0..6).map(|_| MemoryId::generate()).collect();
    // Exact in binary: written as `1.0 - 0.1 * i` the steps are not, and one gap comes out
    // fractionally largest, which is enough for this to pass without the rule holding.
    let scored: Vec<(MemoryId, f64)> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, 1.0 - 0.25 * i as f64))
        .collect();

    let cut = eval::cut_at_largest_gap(&scored, 6);
    assert!(
        cut > 1,
        "an even decline should not collapse to a single result, got {cut}"
    );
}

/// The ranking used for tuning removes the centroid too.
///
/// `rank_scored` is what every measured figure goes through, and its centring was unpinned until a
/// mutation moved onto it: the recorded mutation matched a line that `ranks_from_query` and
/// `rank_scored` now share, so it was silently testing the wrong function. Same hub construction as
/// the other centring test — a memory sitting on the centroid leads uncentred and falls behind once
/// the mean is removed.
#[test]
fn the_tuning_ranking_removes_the_centroid() {
    let ids: Vec<MemoryId> = (0..3).map(|_| MemoryId::generate()).collect();
    let mut vectors = VectorSet::new(2);
    vectors
        .insert(ids[0], vector(&[1.0, 1.0]))
        .expect("inserts");
    vectors
        .insert(ids[1], vector(&[1.0, -1.0]))
        .expect("inserts");
    vectors
        .insert(ids[2], vector(&[1.0, 0.0]))
        .expect("inserts");

    let memories: Vec<Memory> = ids.iter().map(|id| memory_with(*id, vec![])).collect();
    let corpus = Corpus::new(memories, vectors);

    let ranked = eval::rank(
        &corpus,
        &vector(&[1.0, 0.1]),
        &eval::Walk::none(),
        eval::Combine::OwnOnly,
    );
    assert_eq!(
        ranked[0], ids[0],
        "centred, the memory pointing the query's way leads; uncentred the hub would"
    );
}

// ---------------------------------------------------------------------------
// Combining by position rather than by score
// ---------------------------------------------------------------------------

/// **The property the whole rule exists for.** Fusing two rankings by position depends on the
/// *order* of the scores and on nothing else, so any monotone rescaling of them leaves the answer
/// untouched. That is what makes it robust to a corpus whose scores are packed differently from the
/// one it was tuned on — where `sum` compares an inherited *level* against a *gap* between
/// own-scores, and packing decides which wins.
#[test]
fn rank_fusion_is_unmoved_by_any_monotone_rescaling_of_the_scores() {
    let ids: Vec<MemoryId> = (0..4).map(|_| MemoryId::generate()).collect();

    let spread_out: Vec<(MemoryId, f64)> = vec![
        (ids[0], 0.90),
        (ids[1], 0.60),
        (ids[2], 0.30),
        (ids[3], -0.50),
    ];
    // The same order, packed at the top and stretched at the bottom, which is exactly the
    // difference between one corpus and another.
    let packed: Vec<(MemoryId, f64)> = vec![
        (ids[0], 0.9001),
        (ids[1], 0.9000),
        (ids[2], 0.8999),
        (ids[3], -9.0),
    ];

    let inherited: HashMap<MemoryId, f64> = HashMap::from([(ids[3], 0.4)]);

    let one = eval::fuse_by_rank(&spread_out, &inherited, 60.0);
    let other = eval::fuse_by_rank(&packed, &inherited, 60.0);

    let order =
        |fused: &[(MemoryId, f64)]| -> Vec<MemoryId> { fused.iter().map(|(id, _)| *id).collect() };
    assert_eq!(
        order(&one),
        order(&other),
        "packing the scores must not move a ranking built from positions"
    );
}

/// The inherited ranking has to reach the answer, or fusion is an expensive way to return the
/// content ranking.
#[test]
fn rank_fusion_lifts_what_the_walk_reached() {
    let ids: Vec<MemoryId> = (0..4).map(|_| MemoryId::generate()).collect();
    let own: Vec<(MemoryId, f64)> = vec![
        (ids[0], 0.90),
        (ids[1], 0.60),
        (ids[2], 0.30),
        (ids[3], -0.50),
    ];

    let untouched = eval::fuse_by_rank(&own, &HashMap::new(), 60.0);
    assert_eq!(
        untouched.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        ids,
        "with nothing reached, fusion returns the content ranking"
    );

    let reached = eval::fuse_by_rank(&own, &HashMap::from([(ids[3], 0.4)]), 60.0);
    assert!(
        reached.iter().position(|(id, _)| *id == ids[3])
            < untouched.iter().position(|(id, _)| *id == ids[3]),
        "the memory the walk reached has to climb"
    );
}

/// Fusion keeps **both** signals, which is what separates it from a maximum: a memory that ranks
/// well on content *and* was reached beats one that was only reached, where a maximum collapses the
/// two the moment the inherited term is larger.
///
/// Note what this test does **not** claim. A memory reached first can pass the content leader — that
/// is how fusing two lists works, and it is why the rule has to be measured rather than assumed
/// safe. What is bounded is the *size* of the gain: being reached is worth at most one list's top
/// position, never a multiple of the score range, which is exactly the unbounded quantity `sum`
/// adds. In positional terms `k` **is** that exchange rate — being reached is worth roughly `k`
/// places of content rank — which is a far more legible knob than a decay, and one this corpus can
/// measure.
#[test]
fn rank_fusion_keeps_both_signals() {
    let ids: Vec<MemoryId> = (0..3).map(|_| MemoryId::generate()).collect();
    let own: Vec<(MemoryId, f64)> = vec![(ids[0], 0.99), (ids[1], 0.50), (ids[2], -0.90)];

    // The second memory is reached best; the third is reached too, but further out.
    let fused = eval::fuse_by_rank(&own, &HashMap::from([(ids[1], 0.9), (ids[2], 0.4)]), 60.0);

    assert_eq!(
        fused[0].0, ids[1],
        "good on both lists beats good on either alone"
    );
    assert!(
        fused.iter().position(|(id, _)| *id == ids[1])
            < fused.iter().position(|(id, _)| *id == ids[2]),
        "and between two reached memories, the better content match leads"
    );
}

// ---------------------------------------------------------------------------
// What the score distribution looks like
// ---------------------------------------------------------------------------

/// Whether a corpus is *packed* is what decides how a level-versus-gap rule like `sum` behaves on
/// it, and the mean says whether standardising the scores could ever change a ranking. Both are
/// properties of the query's own-score distribution, so they are reported rather than assumed.
#[test]
fn the_spread_reports_the_mean_the_deviation_and_the_gap_below_the_leader() {
    // Deliberately not in order: a ranking arrives sorted, but a fixture that is already sorted
    // cannot tell a rule that sorts from one that does not — which is how the first version of this
    // test let a mutation removing the sort survive.
    let spread = eval::Spread::over(&[0.0, 1.0, -0.5, 0.5]).expect("four scores have a spread");

    assert!((spread.mean - 0.25).abs() < 1e-9);
    assert!((spread.top_gap - 0.5).abs() < 1e-9, "1.0 down to 0.5");
    assert!(
        spread.deviation > 0.5 && spread.deviation < 0.6,
        "{spread:?}"
    );

    // A packed top and the same count: the deviation barely moves, the top gap collapses. That
    // difference is the one that matters, which is why both are reported.
    let packed = eval::Spread::over(&[0.0, 0.999, -0.5, 1.0]).expect("a spread");
    assert!(packed.top_gap < 0.01);
}

/// An empty ranking has no distribution. Reporting zeros would enter a fabricated point into the
/// summary this exists to inform.
#[test]
fn an_empty_ranking_has_no_spread() {
    assert!(eval::Spread::over(&[]).is_none());
}

/// The harness must be able to measure exactly what ships, or the next decision is taken on a rule
/// the product does not run.
///
/// This is the single-source-of-truth check for a fact that genuinely lives in two places: the
/// product's [`traversal::combine`] and the harness's `Blend`, which exist separately so the harness
/// can also measure the rules that were *not* chosen.
#[test]
fn the_shipped_rule_is_the_blend_the_harness_measures() {
    let ids: Vec<MemoryId> = (0..2).map(|_| MemoryId::generate()).collect();
    let mut vectors = VectorSet::new(2);
    vectors
        .insert(ids[0], vector(&[1.0, 0.2]))
        .expect("inserts");
    vectors
        .insert(ids[1], vector(&[0.2, 1.0]))
        .expect("inserts");
    let corpus = Corpus::new(
        ids.iter().map(|id| memory_with(*id, vec![])).collect(),
        vectors,
    );

    let shipped_weight = scatterhoard::traversal::INHERITED_WEIGHT;
    let query = vector(&[1.0, 0.3]);
    let walk = eval::Walk::measured();

    let harness = eval::rank_scored(&corpus, &query, &walk, eval::Combine::Blend(shipped_weight));
    for (id, score) in harness {
        let own =
            f64::from(corpus_similarity(&corpus, &query, id).expect("every memory is scored"));
        let inherited = 0.0;
        assert!(
            (score - scatterhoard::traversal::combine(own, inherited)).abs() < 1e-12,
            "the harness's blend and the shipped rule disagree for {id}"
        );
    }
}

/// One memory's own similarity under the same scan the ranking uses.
fn corpus_similarity(corpus: &Corpus, query: &Vector, id: MemoryId) -> Option<f32> {
    eval::rank_scored(corpus, query, &eval::Walk::none(), eval::Combine::OwnOnly)
        .into_iter()
        .find(|(other, _)| *other == id)
        .map(|(_, score)| score as f32)
}
