// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the walk itself, apart from any store.
//!
//! The walk is shared: recall runs it over a store's links, and the evaluation harness runs it over
//! a corpus held in memory. These tests exercise it directly, on adjacency built by hand, so a
//! property of the walk is asserted where it lives rather than through whichever caller happened to
//! be convenient.

use scatterhoard::memory::{LinkLabel, MemoryId};
use scatterhoard::traversal::{self, Adjacency, Walk};

fn ids(n: usize) -> Vec<MemoryId> {
    (0..n).map(|_| MemoryId::generate()).collect()
}

/// A path `0 - 1 - 2 - 3`, so distance from the first is the index.
fn path(ids: &[MemoryId]) -> Adjacency {
    let mut adjacency = Adjacency::new();
    for pair in ids.windows(2) {
        adjacency.add_edge(pair[0], pair[1]);
    }
    adjacency
}

// ---------------------------------------------------------------------------
// Uniform, symmetric expansion
// ---------------------------------------------------------------------------

/// Expansion is symmetric: an edge is crossed the same way from either end. An outbound-only walk
/// would strand the memory that *cites* the seed, which is half of what an authored link means and
/// the half that grows with the corpus.
#[test]
fn an_edge_is_crossed_from_either_end() {
    let ids = ids(2);
    let mut adjacency = Adjacency::new();
    adjacency.add_edge(ids[0], ids[1]);

    let walk = Walk {
        seeds: 1,
        hops: 1,
        decay: 0.5,
        frontier: 10,
    };

    let forward = traversal::reachable(&adjacency, &[(ids[0], 1.0)], &walk);
    let backward = traversal::reachable(&adjacency, &[(ids[1], 1.0)], &walk);

    assert_eq!(forward.get(&ids[1]).copied(), Some(0.5));
    assert_eq!(backward.get(&ids[0]).copied(), Some(0.5));
}

/// The inherited score fades with distance and with nothing else. Every other term the design names
/// -- per-label weights, native confidence, in-degree -- is held flat, so distance is the only thing
/// that may move this number.
#[test]
fn the_inherited_score_decays_with_distance() {
    let ids = ids(4);
    let walk = Walk {
        seeds: 1,
        hops: 3,
        decay: 0.5,
        frontier: 10,
    };

    let reached = traversal::reachable(&path(&ids), &[(ids[0], 1.0)], &walk);

    assert_eq!(reached.get(&ids[1]).copied(), Some(0.5));
    assert_eq!(reached.get(&ids[2]).copied(), Some(0.25));
    assert_eq!(reached.get(&ids[3]).copied(), Some(0.125));
}

/// The hop count is a bound on distance, not on effort: nothing beyond it is reached at all.
#[test]
fn the_walk_stops_after_the_last_hop() {
    let ids = ids(4);
    let walk = Walk {
        seeds: 1,
        hops: 2,
        decay: 0.9,
        frontier: 10,
    };

    let reached = traversal::reachable(&path(&ids), &[(ids[0], 1.0)], &walk);

    assert!(reached.contains_key(&ids[2]), "two hops out is reached");
    assert!(
        !reached.contains_key(&ids[3]),
        "three hops out is beyond the bound"
    );
}

/// The bound is on **expansion**, never on inclusion. Reaching a heavily-cited memory is cheap and
/// often the most useful thing returned; expanding *through* it is what branches. A cap on inclusion
/// would discard good results to solve a problem they did not cause.
#[test]
fn the_frontier_bounds_expansion_not_admission() {
    let ids = ids(12);
    let mut adjacency = Adjacency::new();
    // One hub, cited by everything after it.
    for id in &ids[2..] {
        adjacency.add_edge(ids[1], *id);
    }
    adjacency.add_edge(ids[0], ids[1]);

    let walk = Walk {
        seeds: 1,
        hops: 2,
        decay: 0.9,
        frontier: 1,
    };

    let reached = traversal::reachable(&adjacency, &[(ids[0], 1.0)], &walk);

    assert!(
        reached.len() >= 11,
        "expanding through one hub admits everything it reaches, got {}",
        reached.len()
    );
}

/// A memory reached by several paths keeps the **best** inherited score rather than a sum of them.
/// Summing would make a dense neighbourhood accumulate weight merely by being dense, which is
/// in-degree entering the ranking by the back door -- and the sign of an in-degree term is
/// label-dependent and deliberately unresolved.
#[test]
fn several_paths_to_one_memory_keep_the_best_rather_than_adding() {
    let ids = ids(4);
    let mut adjacency = Adjacency::new();
    // Two one-hop routes from the seed to the same memory, plus a longer one.
    adjacency.add_edge(ids[0], ids[3]);
    adjacency.add_edge(ids[0], ids[1]);
    adjacency.add_edge(ids[1], ids[3]);
    adjacency.add_edge(ids[1], ids[2]);
    adjacency.add_edge(ids[2], ids[3]);

    let walk = Walk {
        seeds: 1,
        hops: 3,
        decay: 0.5,
        frontier: 10,
    };

    let reached = traversal::reachable(&adjacency, &[(ids[0], 1.0)], &walk);

    assert_eq!(
        reached.get(&ids[3]).copied(),
        Some(0.5),
        "the shortest path sets the score, and the others do not add to it"
    );
}

/// Supersession is a **validity** relation rather than a relevance one, resolved before ranking
/// rather than as part of it. The walk must not cross it, or a memory would be reached through the
/// very edge that says it is out of date.
#[test]
fn supersession_is_not_a_traversable_edge() {
    assert!(!traversal::is_traversable(&LinkLabel::Supersedes));
    assert!(!traversal::is_traversable(&LinkLabel::SupersededBy));
    assert!(traversal::is_traversable(&LinkLabel::Supports));
    assert!(traversal::is_traversable(&LinkLabel::Contradicts));
}

/// **Nothing walks an edge it does not understand.** A label from a newer build is carried and
/// shown, but reaching a memory through it would be guessing at what the edge claims -- the half of
/// the original argument for a closed vocabulary that survived carrying unknown labels opaquely.
#[test]
fn an_unrecognised_label_is_not_walked() {
    let label = LinkLabel::from_name("is_sketch");
    assert!(
        !label.is_recognised(),
        "the fixture must be a label this build does not know"
    );
    assert!(!traversal::is_traversable(&label));
}

/// **A walk origin never inherits.** Expansion is symmetric and the walk takes several hops, so a
/// path out of a seed and back along the same edge returns to where it started — and since a
/// reached memory's score is its own similarity *plus* what it inherited, a seed would be summing a
/// decayed copy of its own score. That is not a relation to anything: it ranks a memory with any
/// traversable edge above one with none, independently of what the edge points at, which is
/// in-degree entering the ranking by a route this design closes elsewhere.
#[test]
fn a_walk_origin_does_not_inherit_along_a_path_that_returns_to_it() {
    let ids = ids(3);
    let walk = Walk {
        seeds: 1,
        hops: 3,
        decay: 0.9,
        frontier: 10,
    };

    let reached = traversal::reachable(&path(&ids), &[(ids[0], 1.0)], &walk);

    assert!(
        !reached.contains_key(&ids[0]),
        "the seed must not inherit a decayed copy of its own score, got {:?}",
        reached.get(&ids[0])
    );
    assert!(reached.contains_key(&ids[1]) && reached.contains_key(&ids[2]));
}

/// The same rule between two seeds. Both already have their own similarity to the query — the walk
/// exists to reach what content similarity did not — so a link between two direct hits must not
/// promote either of them for being linked.
#[test]
fn two_seeds_linked_to_each_other_do_not_promote_each_other() {
    let ids = ids(2);
    let mut adjacency = Adjacency::new();
    adjacency.add_edge(ids[0], ids[1]);

    let walk = Walk {
        seeds: 2,
        hops: 2,
        decay: 0.9,
        frontier: 10,
    };

    let reached = traversal::reachable(&adjacency, &[(ids[0], 1.0), (ids[1], 0.9)], &walk);

    assert!(
        reached.is_empty(),
        "neither seed may inherit from the other, got {reached:?}"
    );
}

/// Each memory takes **one** frontier slot however many paths improve it. Without that, a memory
/// improved twice occupies two slots, so the bound on expansion spends its budget on a duplicate and
/// a different neighbourhood goes unexplored.
///
/// The seeds are deliberately in *ascending* score order, because that is the case that produces a
/// second improvement at all — and the walk must not depend on the order its seeds arrive in.
#[test]
fn a_memory_takes_one_frontier_slot_however_many_paths_improve_it() {
    let ids = ids(6);
    let (weak, strong, crowded, other, beyond_crowded, beyond_other) =
        (ids[0], ids[1], ids[2], ids[3], ids[4], ids[5]);

    let mut adjacency = Adjacency::new();
    // Both seeds reach `crowded`; the weaker one gets there first and is improved on.
    adjacency.add_edge(weak, crowded);
    adjacency.add_edge(strong, crowded);
    adjacency.add_edge(weak, other);
    adjacency.add_edge(crowded, beyond_crowded);
    adjacency.add_edge(other, beyond_other);

    let walk = Walk {
        seeds: 2,
        hops: 2,
        decay: 0.9,
        // One slot each for the two memories the first hop reaches, and none to spare.
        frontier: 2,
    };

    let reached = traversal::reachable(&adjacency, &[(weak, 0.4), (strong, 1.0)], &walk);

    assert!(
        reached.contains_key(&beyond_other),
        "a duplicate of the improved memory took the slot this neighbourhood needed"
    );
}

// ---------------------------------------------------------------------------
// Where to stop
// ---------------------------------------------------------------------------

/// The same rule, in the case that actually exercises it: a memory reached first by a poor path and
/// then by a better one. Only here can "keep the best" and "add them up" be told apart — while every
/// path found gets worse, the two agree, which is how a mutation that summed them survived a test
/// built on a fixture where the best path happened to come first.
#[test]
fn a_better_path_found_later_replaces_the_score_rather_than_adding_to_it() {
    let ids = ids(4);
    let mut adjacency = Adjacency::new();
    // Two hops from the strong seed, one hop from the weak one.
    adjacency.add_edge(ids[0], ids[1]);
    adjacency.add_edge(ids[1], ids[3]);
    adjacency.add_edge(ids[2], ids[3]);

    let walk = Walk {
        seeds: 2,
        hops: 2,
        decay: 0.5,
        frontier: 10,
    };

    // The weak seed reaches it first, at 0.1; the strong seed reaches it a hop later, at 0.25.
    let reached = traversal::reachable(&adjacency, &[(ids[0], 1.0), (ids[2], 0.2)], &walk);

    assert_eq!(
        reached.get(&ids[3]).copied(),
        Some(0.25),
        "the better path replaces the worse one; the two are not added"
    );
}

/// The cut falls at the largest drop between consecutive scores.
#[test]
fn the_cut_falls_at_the_largest_drop() {
    let scores = [0.90, 0.88, 0.85, 0.20, 0.18, 0.17];
    assert_eq!(traversal::cut_at_largest_gap(&scores, 6), 3);
}

/// Translating and scaling every score leaves the cut where it was. Without that the rule would
/// need a constant, and a constant is what the cold-start problem makes indefensible: a store with
/// few memories has scores whose absolute values mean nothing at all.
#[test]
fn the_cut_is_unmoved_by_translation_and_scale() {
    let base = [0.90, 0.88, 0.85, 0.20, 0.18, 0.17];
    let shifted: Vec<f64> = base.iter().map(|score| 3.0 * score - 4.0).collect();

    assert!(
        shifted[0] < 0.0,
        "the top hit must be negative to test this"
    );
    assert_eq!(
        traversal::cut_at_largest_gap(&base, 6),
        traversal::cut_at_largest_gap(&shifted, 6)
    );
}

/// The window bounds how far down a cliff is looked for, and is the ceiling on what comes back.
///
/// A drop below the window is not reached for, however large it is: the caller's budget is a budget,
/// not a hint. Within the window the rule still cuts at the best drop it can see, which is why this
/// asserts a ceiling rather than that the whole window comes back.
#[test]
fn the_window_is_a_ceiling_on_the_cut() {
    let scores = [0.90, 0.89, 0.88, 0.87, 0.10, 0.09];

    assert_eq!(
        traversal::cut_at_largest_gap(&scores, 6),
        4,
        "given the whole ranking, the cut falls at the cliff"
    );
    assert!(
        traversal::cut_at_largest_gap(&scores, 3) <= 3,
        "a window of three cannot return a fourth memory to reach that cliff"
    );
}

/// A ranking that falls away evenly has no cliff to find, so the rule must not invent one at
/// position 1: the largest gap in a ranking is very often the one just below the best match, and
/// cutting there would make recall a coin flip on whether the query had exactly one answer.
///
/// The steps are exact in binary deliberately. Written as `1.0 - 0.1 * i` they are not, and the
/// rounding leaves one gap fractionally largest — which let a mutation that cut directly below the
/// top hit survive, because the assertion was passing on a floating-point artifact rather than on
/// the rule.
#[test]
fn an_even_decline_is_not_cut_at_the_top() {
    let scores = [1.0, 0.75, 0.5, 0.25, 0.0];

    assert_eq!(
        traversal::cut_at_largest_gap(&scores, 5),
        2,
        "with every drop the same size, the cut goes to the first the rule is allowed to take"
    );
}

/// Fewer than three scores have no interior gap to find, so everything in the window is returned.
#[test]
fn a_ranking_too_short_to_have_a_cliff_returns_all_of_it() {
    assert_eq!(traversal::cut_at_largest_gap(&[0.9, 0.1], 5), 2);
    assert_eq!(traversal::cut_at_largest_gap(&[0.9], 5), 1);
    assert_eq!(traversal::cut_at_largest_gap(&[], 5), 0);
}

/// The rule the design settled on 2026-08-18: a reached memory's score is a **weighted blend** of
/// its own similarity and the decayed share it inherited, at a quarter on the inherited term.
///
/// Chosen on stability rather than on a peak. It matches what `sum` reaches on both answer keys
/// while moving about a point across the decays tested, against sum's fourteen to twenty-six, and it
/// is the only rule measured that stays above the content-only baseline at every decay on both keys.
#[test]
fn the_inherited_share_is_a_quarter_of_a_reached_memory_s_score() {
    let blended = traversal::combine(0.4, 0.8);
    assert!(
        (blended - (0.75 * 0.4 + 0.25 * 0.8)).abs() < 1e-12,
        "got {blended}"
    );
}

/// **A blend does not penalise a memory for going unreached**, which is worth pinning because the
/// design once rejected blends for exactly that reason. Multiply through and `0.75·own + 0.25·inh`
/// orders identically to `own + (1/3)·inh`: a common positive factor on every score, so an unreached
/// memory keeps its place against every other unreached memory, and against reached ones it loses
/// only what the walk genuinely added elsewhere.
#[test]
fn the_blend_orders_unreached_memories_exactly_as_their_own_scores_do() {
    let better = traversal::combine(0.6, 0.0);
    let worse = traversal::combine(0.4, 0.0);
    assert!(better > worse);

    // The ordering is the same as a sum with the inherited term scaled by a third, which is what the
    // blend is, rank for rank.
    let scaled = |own: f64, inherited: f64| own + inherited / 3.0;
    let pairs = [(0.9, 0.0), (0.4, 0.8), (0.5, 0.2), (-0.3, 0.9)];
    for (a, b) in pairs.iter().zip(pairs.iter().skip(1)) {
        let blended = traversal::combine(a.0, a.1) > traversal::combine(b.0, b.1);
        assert_eq!(
            blended,
            scaled(a.0, a.1) > scaled(b.0, b.1),
            "the blend and the scaled sum must order {a:?} against {b:?} alike"
        );
    }
}
