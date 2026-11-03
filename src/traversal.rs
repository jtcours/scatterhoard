// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The walk outward from a query's seeds, and where to stop returning what it found.
//!
//! See `documents/design/recall-mechanisms.md`. The division this module rests on is that **the
//! walk generates candidates and the embedding judges them**: expansion is uniform and symmetric,
//! crossing every label the same way in both directions and decaying with distance alone, because
//! the graph does not know relevance — it knows what some author thought worth connecting.
//! Relevance is judged afterwards, by scoring each candidate against the query.
//!
//! That tolerance for a rough walk is **bought by the re-scoring, and is not a property of the
//! walk**. A `RELATED_TO` edge from "ice" into frozen water is picked up by the expansion and
//! dropped by the ranker; were ranking ever to run on graph distance alone, this same crude
//! expansion would become a defect.
//!
//! # It lives here so that recall and the harness cannot drift apart
//!
//! [`crate::store`] runs this over a store's links; [`crate::eval`] runs it over a corpus held in
//! memory, to measure it. A harness measuring a walk the product does not run answers a question
//! nobody asked, so the walk, the combination rule and the cut-off are written once and called
//! twice, rather than implemented on each side.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::memory::{LinkLabel, MemoryId};

/// How far the walk goes, and how fast an inherited score fades.
///
/// Every field here is a **bound** except `decay`, and the difference matters: a bound set too
/// tight is conservative rather than incorrect and can be defended with no corpus at all, while a
/// weight is a tuning problem. `documents/design/recall-mechanisms.md` holds every weight flat
/// until a corpus can speak to it.
#[derive(Debug, Clone, Copy)]
pub struct Walk {
    /// How many content hits seed the walk.
    pub seeds: usize,
    /// How far from a seed the walk reaches.
    pub hops: usize,
    /// What fraction of a score survives one hop.
    pub decay: f64,
    /// How many memories may be **expanded through** at each hop.
    ///
    /// Never a cap on what is admitted. Reaching a heavily-cited memory is cheap and often the most
    /// useful thing returned; expanding *through* it is what branches.
    pub frontier: usize,
}

impl Walk {
    /// The parameters recall runs with.
    ///
    /// **Measured 2026-08-18 against the rule that ships**, over 24 queries harvested from this
    /// project's own transcripts and labelled twice, blind, on different models. Both answer keys
    /// put the best cell here: three hops and five seeds each beat their neighbours on both, and a
    /// decay of 0.7 leads at that shape.
    ///
    /// An earlier version of these constants was tuned against a walk that let a seed inherit along
    /// a path back to itself, and against an unweighted sum. Both are gone, so the numbers were
    /// re-measured rather than carried over — the old sweep was partly measuring an artifact.
    ///
    /// **The decay barely matters now**, which is the point of the weight it accompanies: across
    /// 0.3 to 0.9 this configuration moves about one and a half points. That is a deliberate trade
    /// of a sensitive constant for an insensitive one.
    ///
    /// `frontier` did not bind at 429 memories — 5, 20 and 40 give identical results — so it is
    /// carried as the safety property it was defined to be, for a corpus larger than anything
    /// measured, rather than as a knob anyone tuned.
    #[must_use]
    pub const fn measured() -> Self {
        Self {
            seeds: 5,
            hops: 3,
            decay: 0.7,
            frontier: 20,
        }
    }

    /// No traversal at all — the content-only baseline every rule is measured against.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            seeds: 0,
            hops: 0,
            decay: 0.0,
            frontier: 0,
        }
    }
}

/// Whether an edge is one the walk crosses.
///
/// Supersession is excluded. It is a **validity** relation rather than a relevance one — resolved
/// before ranking rather than as part of it — so reaching a memory through the edge that says it is
/// out of date would be the walk contradicting the resolution.
///
/// **An unrecognised label is excluded too**: nothing walks an edge it does not understand, which
/// is the half of the 2026-08-02 argument that survived carrying unknown labels opaquely
/// (`documents/design/memory-data-model.md`). Written as an exhaustive match rather than as a
/// negation, because `!matches!(Supersedes | SupersededBy)` is exactly the shape that makes a new
/// variant walkable by omission — which is what it did to `Unrecognised` the day it was added.
#[must_use]
pub fn is_traversable(label: &LinkLabel) -> bool {
    match label {
        LinkLabel::RelatedTo
        | LinkLabel::RefersTo
        | LinkLabel::Causes
        | LinkLabel::ResultsFrom
        | LinkLabel::MotivatedBy
        | LinkLabel::Supports
        | LinkLabel::Contradicts
        | LinkLabel::Resolves
        | LinkLabel::DependsOn
        | LinkLabel::ExceptionTo
        | LinkLabel::Violates
        | LinkLabel::InstanceOf => true,
        LinkLabel::Supersedes | LinkLabel::SupersededBy | LinkLabel::Unrecognised(_) => false,
    }
}

/// Which memories are one edge apart, with direction discarded.
///
/// Undirected because expansion is symmetric: an edge is walked the same way from either end. An
/// outbound-only walk would strand the memory that *cites* the seed, which is half of what an
/// authored link means and the half that grows with the corpus.
///
/// **What may be added is the caller's decision, and it is where the scope rule lives.** A store
/// adds an edge only when both endpoints are in the population being searched, which is what makes
/// an unscoped recall cross a set boundary and a scoped one stop at it — the same rule producing
/// both behaviours rather than two rules that could disagree.
/// **Generic in what identifies a memory, and that is the point.** A bare [`MemoryId`] is identity
/// only where one store is in play; across several, two loaded sets may hold the same id and are
/// then two memories, not one -- `two-loaded-sets-sharing-one-id-collapse-to-one-memory`. Recall
/// therefore walks on a key that carries the set as well, and the default keeps every single-store
/// caller reading as it did.
#[derive(Debug, Clone)]
pub struct Adjacency<K = MemoryId> {
    neighbours: HashMap<K, Vec<K>>,
}

impl<K> Default for Adjacency<K> {
    fn default() -> Self {
        Self {
            neighbours: HashMap::new(),
        }
    }
}

impl<K: Copy + Eq + Hash + Ord> Adjacency<K> {
    /// An empty adjacency.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that two memories are one edge apart.
    ///
    /// Both directions, always. A self-edge is dropped: it cannot carry a score anywhere, and
    /// admitting it would let a memory inherit a decayed copy of its own score.
    pub fn add_edge(&mut self, one: K, other: K) {
        if one == other {
            return;
        }
        self.neighbours.entry(one).or_default().push(other);
        self.neighbours.entry(other).or_default().push(one);
    }

    /// What is one edge from this memory.
    #[must_use]
    pub fn neighbours(&self, id: K) -> &[K] {
        self.neighbours.get(&id).map_or(&[], Vec::as_slice)
    }

    /// How many memories have at least one edge.
    #[must_use]
    pub fn len(&self) -> usize {
        self.neighbours.len()
    }

    /// Whether nothing is connected to anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.neighbours.is_empty()
    }
}

/// Everything the walk reaches from the given seeds, with the score each inherited.
///
/// Seeds are supplied rather than selected here: recall takes them from the content ranking, and the
/// harness from whatever it is measuring. Each is an id and the score it carries. **They are
/// expected to be distinct memories** — recall resolves them to their tips first, so a supersession
/// chain does not spend several of a small seed budget on one memory.
///
/// # A walk origin never inherits
///
/// The seeds are excluded from the result, and this is a correctness rule rather than an
/// optimisation. Expansion is symmetric and the walk takes several hops, so a path out of a seed
/// and back along the same edge returns to where it started; since [`combine`] adds the inherited
/// share to a memory's *own* similarity, a seed would be summing a decayed copy of its own score.
/// That promotes a memory for having any edge at all, independently of what the edge points at —
/// which is in-degree entering the ranking by a route the design closes deliberately elsewhere.
///
/// It applies between seeds as well: two direct hits linked to each other may not promote each
/// other, because both already carry their own similarity and the walk exists to reach what content
/// similarity did **not**.
///
/// # Reached twice, scored once
///
/// A memory reached by more than one path keeps the **best** inherited score rather than a sum, so a
/// densely connected neighbourhood does not accumulate weight merely by being dense. An improvement
/// re-enters the frontier, so a shortcut found after the long way round propagates onward rather
/// than only correcting the one memory. And each memory occupies **one** frontier slot per hop
/// carrying its best score, so the bound on expansion is never spent on a duplicate — which is what
/// makes the walk independent of the order its seeds arrive in.
#[must_use]
pub fn reachable<K: Copy + Eq + Hash + Ord>(
    adjacency: &Adjacency<K>,
    seeds: &[(K, f64)],
    walk: &Walk,
) -> HashMap<K, f64> {
    let mut inherited: HashMap<K, f64> = HashMap::new();
    if walk.seeds == 0 || walk.hops == 0 {
        return inherited;
    }

    let mut frontier: Vec<(K, f64)> = seeds.iter().take(walk.seeds).copied().collect();
    let origins: HashSet<K> = frontier.iter().map(|(id, _)| *id).collect();

    for _ in 0..walk.hops {
        let mut next: HashMap<K, f64> = HashMap::new();
        for (id, score) in &frontier {
            let carried = score * walk.decay;
            for neighbour in adjacency.neighbours(*id) {
                if origins.contains(neighbour) {
                    continue;
                }
                let entry = inherited.entry(*neighbour).or_insert(f64::NEG_INFINITY);
                if carried > *entry {
                    *entry = carried;
                    let slot = next.entry(*neighbour).or_insert(f64::NEG_INFINITY);
                    *slot = slot.max(carried);
                }
            }
        }
        // The bound applies here, to what is expanded *through*, and never to `inherited`, which is
        // what gets admitted to the results.
        let mut ordered: Vec<(K, f64)> = next.into_iter().collect();
        ordered.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ordered.truncate(walk.frontier);
        if ordered.is_empty() {
            break;
        }
        frontier = ordered;
    }
    inherited
}

/// What share of a reached memory's score comes from the walk rather than from its own wording.
///
/// **Chosen on stability, not on a peak.** Across decays from 0.3 to 0.9 on two independent answer
/// keys, this weight matches what an unweighted sum reaches while moving about one point, against
/// sum's fourteen to twenty-six — and it is the only rule measured that stays above the content-only
/// baseline at *every* decay on *both* keys. A rule that wins the cell it was tuned in and collapses
/// beside it is the worse choice, because the corpus a rule is tuned on is never the corpus it runs
/// on. Decided 2026-08-18 by the human partner on that evidence.
pub const INHERITED_WEIGHT: f64 = 0.25;

/// How a reached memory's inherited and own scores combine: a **weighted blend**, a quarter of it
/// from the walk.
///
/// # This is a scaled sum, and saying so corrects the reasoning that rejected it
///
/// `0.75·own + 0.25·inherited` orders identically to `own + (1/3)·inherited` — multiply through by
/// four thirds. So a blend does **not** "take weight away from the own term", which is what this
/// design originally held against blends: a common positive factor changes no ordering. What the
/// weight actually sets is *how much inherited score is admitted at all*, and the blend that
/// measured worse than no traversal was admitting three times what a sum does, not penalising
/// direct hits.
///
/// It follows that this weight and the walk's `decay` do overlapping work — both scale the inherited
/// term — which is why a smaller weight is less sensitive to the decay. That overlap is recorded
/// rather than resolved; see `documents/design/recall-mechanisms.md`.
///
/// Unlike a maximum, the blend keeps both signals: a reached memory that also matches well is
/// separated from one that barely matches.
#[must_use]
pub fn combine(own: f64, inherited: f64) -> f64 {
    combine_with(own, inherited, INHERITED_WEIGHT)
}

/// The same blend at a stated weight, so the weight can be varied without editing the constant.
///
/// Exists for instrumentation: `Stages` turns the scorer's decisions off one at a time to measure
/// what each contributes, and the inherited blend is a weight rather than a switch. Passing
/// [`INHERITED_WEIGHT`] reproduces [`combine`] exactly, which is what
/// `the_instrumented_scorer_at_its_defaults_matches_the_shipping_one` pins.
#[must_use]
pub fn combine_with(own: f64, inherited: f64, weight: f64) -> f64 {
    (1.0 - weight) * own + weight * inherited
}

/// How many results to return: cut at the largest drop between consecutive scores.
///
/// **A gap is the only shape of cut-off this scoring admits.** Scores are centred cosines, so they
/// are freely negative and a ratio to the top hit means nothing when the top hit is below zero. An
/// absolute threshold is ruled out by `no-non-match-floor` and by the cold-start problem — a store
/// with few memories has scores whose absolute values carry no information, and a one-memory store
/// scores its only memory 0.0. A gap is a *difference*, so translation moves it not at all, and gaps
/// are compared against each other rather than against a constant, so scale moves it not at all
/// either. It needs no number anyone would have to defend.
///
/// `window` bounds how far down the ranking a cliff is looked for, and is the caller's budget rather
/// than a property of the data.
///
/// `scores` must already be ordered best first, which is what makes a difference between
/// consecutive entries a *drop* rather than an arbitrary pair.
#[must_use]
pub fn cut_at_largest_gap(scores: &[f64], window: usize) -> usize {
    let limit = window.min(scores.len());
    if limit < 3 {
        return limit;
    }
    let mut best = (limit, f64::NEG_INFINITY);
    // Start at 2 so a single runaway top hit does not collapse every answer to one memory: the
    // largest gap in a ranking is very often the one just below the best match, and cutting there
    // would make recall a coin flip on whether the query happened to have exactly one answer.
    for index in 2..limit {
        let gap = scores[index - 1] - scores[index];
        if gap > best.1 {
            best = (index, gap);
        }
    }
    best.0
}
