// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Measuring whether graph traversal can earn its cost on a given corpus.
//!
//! This is the acceptance check to run on any corpus proposed for traversal evaluation, and to run
//! **before** authoring queries, which is the expensive half. See `recall-eval-harness` in
//! TRACKING.md.
//!
//! # An evaluation corpus is not a memory set
//!
//! It is test data shaped like memory, and it has no business in a store. Importing one as a set to
//! get it embedded was tried and is the reason this module exists: an unscoped recall searches every
//! loaded set, so a 56-memory test corpus took all three top results on an unrelated query. A corpus
//! of several hundred would drown ordinary recall entirely.
//!
//! So nothing here opens a store, creates a set, or writes a vector anywhere. A corpus is read from
//! a memory file, embedded in memory, measured, and dropped.
//!
//! # It measures the real pipeline
//!
//! The embedder spike this replaces reimplemented [`embedding_input`](crate::embedding::embedding_input)
//! and cosine similarity, which is a reimplementation free to drift from what the product runs. This
//! takes vectors from a real [`Embedder`](crate::embedder::Embedder) over the real embedding input,
//! and scores them with the centroid removed, as [`recall`](crate::store) does. A harness measuring
//! a pipeline the product does not run answers a question nobody asked.
//!
//! # The measurement is a percentile, never a rank
//!
//! For each link, every memory is ranked by similarity to the link's *source* and the target's
//! position is recorded — as a fraction of the corpus. That is the scale-invariant quantity, and
//! reporting a rank instead is the trap: at 70 memories a linked target ranking 2nd looks like
//! traversal is redundant, while the same percentile at 1,000 memories ranks 29th and looks
//! essential.

use std::collections::HashMap;

use crate::embedding::{Vector, VectorSet};
use crate::memory::{LinkLabel, Memory, MemoryId};
use crate::traversal::{self, Adjacency};

/// The walk's bounds and its one weight.
///
/// Re-exported rather than redefined: the harness exists to measure the walk the product runs, so
/// varying a parameter here has to vary the same field recall reads. See [`crate::traversal`].
pub use crate::traversal::Walk;

/// A corpus under measurement: memories, and a vector for each.
///
/// Holds them together because every operation here needs both, and a corpus whose vectors have
/// drifted out of step with its memories would produce a plausible number computed against the
/// wrong population.
#[derive(Debug)]
pub struct Corpus {
    memories: Vec<Memory>,
    vectors: VectorSet,
}

impl Corpus {
    /// Pair memories with their vectors.
    ///
    /// A memory with no vector is legitimate — it simply cannot be reached or ranked — so this does
    /// not demand that the two line up. What it must never do is *invent* a pairing, which is why
    /// the lookup below is by id rather than by position.
    #[must_use]
    pub fn new(memories: Vec<Memory>, vectors: VectorSet) -> Self {
        Self { memories, vectors }
    }

    /// The memories in this corpus.
    #[must_use]
    pub fn memories(&self) -> &[Memory] {
        &self.memories
    }

    /// How many memories carry a vector.
    ///
    /// This, rather than the memory count, is the denominator for a percentile: a memory with no
    /// vector never appears in a ranking, so including it would report a target as further out than
    /// anything could actually place it.
    #[must_use]
    pub fn size(&self) -> usize {
        self.vectors.len()
    }
}

/// Where one link's target landed when the query fell exactly on its source.
///
/// The source's own vector stands in for a query here, which is deliberately the **best case** for
/// traversal from that seed: nothing a real query could do would place the source better. A target
/// that is still far out under this measurement is one content similarity genuinely cannot reach.
#[derive(Debug, Clone)]
pub struct Reach {
    /// The memory holding the link.
    pub source: MemoryId,
    /// The memory pointed at.
    pub target: MemoryId,
    /// What the source claims about the target.
    pub label: LinkLabel,
    /// The target's 1-based position when every other memory is ranked against the source.
    pub rank: usize,
    /// [`rank`](Self::rank) as a fraction of the corpus.
    pub percentile: f64,
    /// The centred cosine between source and target.
    pub similarity: f32,
}

/// Rank a link's target against every other memory, from the source.
///
/// Returns one row per traversable edge whose target is in this corpus. Edges that cannot be
/// measured are omitted here and counted by [`unmeasurable_edges`], so the two never disagree about
/// which population a percentile was taken over.
#[must_use]
pub fn link_reach(corpus: &Corpus) -> Vec<Reach> {
    let mut reaches = Vec::new();

    for memory in &corpus.memories {
        // The source's own vector stands in for a query landing exactly on it. Absent means the
        // memory was never embedded, so nothing can be ranked from here.
        let Some(query) = corpus.vectors.get(memory.id) else {
            continue;
        };

        let mut ranking: Option<Vec<crate::embedding::Hit>> = None;

        for link in &memory.links {
            if !traversal::is_traversable(&link.label) {
                continue;
            }
            // A cross-set target names a set this corpus is not, so it is unreachable here even
            // when an id of that value happens to be present. Resolving it by id alone would
            // measure a relationship that does not exist.
            if link.target.set().is_some() {
                continue;
            }
            let target_id = link.target.id();
            if corpus.vectors.get(target_id).is_none() {
                continue;
            }

            // The product's own scan, centred on the corpus mean, rather than a cosine
            // reimplemented here. That is the whole advantage over the spike this replaces: a
            // harness scoring by its own arithmetic measures a pipeline the product does not run,
            // and is free to drift from it silently.
            // Computed once per source and reused across its links: a memory with several links
            // would otherwise rescan the whole corpus for each of them.
            let hits = ranking.get_or_insert_with(|| {
                corpus
                    .vectors
                    .nearest(&query, corpus.size())
                    .into_iter()
                    // The source is excluded because it is standing in for the query, and a query
                    // cannot retrieve itself.
                    .filter(|hit| hit.id != memory.id)
                    .collect::<Vec<_>>()
            });

            let Some(position) = hits.iter().position(|hit| hit.id == target_id) else {
                continue;
            };
            let rank = position + 1;
            let similarity = hits[position].similarity;

            reaches.push(Reach {
                source: memory.id,
                target: target_id,
                label: link.label.clone(),
                rank,
                #[allow(clippy::cast_precision_loss)]
                percentile: rank as f64 / corpus.size() as f64,
                similarity,
            });
        }
    }
    reaches
}

/// How many traversable edges could not be measured.
///
/// Reported rather than swallowed. A corpus quietly measuring half its edges would produce a
/// percentile distribution over a population nobody chose, and the two ways an edge goes unmeasured
/// — a target absent from the corpus, and a target in another set — are both ordinary rather than
/// exceptional.
#[must_use]
pub fn unmeasurable_edges(corpus: &Corpus) -> usize {
    corpus
        .memories
        .iter()
        .flat_map(|memory| &memory.links)
        .filter(|link| traversal::is_traversable(&link.label))
        .filter(|link| {
            link.target.set().is_some() || corpus.vectors.get(link.target.id()).is_none()
        })
        .count()
}

/// How a reached memory's inherited and own scores combine.
///
/// This is the open question in `documents/design/recall-mechanisms.md`. Both terms are needed: the
/// inherited score is what makes traversal worth doing, and the own score is what makes uniform
/// expansion safe, since a walk that wanders out of the sense a query intended is harmless only
/// because the cut re-scores everything.
///
/// **The design names four shapes and they are three functions.** *A maximum* and *the inherited
/// score acting as a floor* are the same arithmetic: a floor says the score is at least the
/// inherited value, which is `max(own, inherited)`. They are listed separately because they are
/// reached by different intuitions, not because they compute different things, and a harness that
/// implemented both would report a tie it had built in.
#[derive(Debug, Clone, Copy)]
pub enum Combine {
    /// Content similarity alone. The baseline: no traversal, and what recall does today.
    OwnOnly,
    /// The larger of the two. Also *the inherited score as a floor* — see the type documentation.
    Max,
    /// Both terms added.
    Sum,
    /// A weighted blend, `w` on the inherited term.
    Blend(f64),
    /// Combine the two **rankings** by position rather than the two scores by value.
    ///
    /// The others all compare an inherited *level* against the *gaps* between own-scores, so how
    /// tightly a corpus packs its top few decides how far the walk can reach — which makes them
    /// sensitive to a property of the corpus rather than of the query. Fusing positions depends on
    /// order alone, so any monotone rescaling of the scores leaves it untouched.
    ///
    /// `k` damps the head of each ranking: without it the top position dominates every fusion. It is
    /// a constant, which this design dislikes, and the reason it is tolerable here is that it can be
    /// **measured** on the same corpus as everything else rather than borrowed.
    RankFusion {
        /// The damping constant.
        k: f64,
    },
}

impl Combine {
    /// The pointwise rules, applied to one memory's two scores.
    ///
    /// `RankFusion` is absent deliberately: it is not a function of one memory's two numbers, which
    /// is the whole point of it. [`rank_scored`] dispatches it to [`fuse_by_rank`] instead.
    fn apply(self, own: f64, inherited: f64) -> f64 {
        match self {
            Self::OwnOnly | Self::RankFusion { .. } => own,
            Self::Max => own.max(inherited),
            // Its own arithmetic, deliberately: `traversal::combine` is now the shipped *blend*,
            // and a harness whose "sum" row silently became the blend could not measure the choice
            // between them. `the_shipped_rule_is_the_blend_the_harness_measures` keeps the two in
            // step where it matters.
            Self::Sum => own + inherited,
            Self::Blend(weight) => weight * inherited + (1.0 - weight) * own,
        }
    }
}

/// Undirected adjacency over the traversable edges within this corpus.
///
/// A cross-set target names a set this corpus is not, so it is left out even when an id of that
/// value happens to be present: resolving it by id alone would walk a relationship that does not
/// exist here.
fn adjacency(corpus: &Corpus) -> Adjacency {
    let mut adjacency = Adjacency::new();
    for memory in &corpus.memories {
        for link in &memory.links {
            if !traversal::is_traversable(&link.label) || link.target.set().is_some() {
                continue;
            }
            let target = link.target.id();
            if corpus.vectors.get(target).is_none() {
                continue;
            }
            adjacency.add_edge(memory.id, target);
        }
    }
    adjacency
}

/// Everything the walk reaches from a query's seeds, with the score each inherited.
///
/// Seeds are the top content hits, which is what recall does too. The walk itself lives in
/// [`crate::traversal`] and is shared with recall, so what this measures is what ships.
#[must_use]
pub fn reachable(corpus: &Corpus, query: &Vector, walk: &Walk) -> HashMap<MemoryId, f64> {
    let seeds: Vec<(MemoryId, f64)> = corpus
        .vectors
        .nearest(query, walk.seeds)
        .into_iter()
        .map(|hit| (hit.id, f64::from(hit.similarity)))
        .collect();
    traversal::reachable(&adjacency(corpus), &seeds, walk)
}

/// Rank the corpus for a query, walking the graph and combining the two scores.
#[must_use]
pub fn rank(corpus: &Corpus, query: &Vector, walk: &Walk, combine: Combine) -> Vec<MemoryId> {
    rank_scored(corpus, query, walk, combine)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// Combine a content ranking with what the walk reached, by **position** rather than by value.
///
/// Each memory scores `1/(k + own position)`, plus `1/(k + inherited position)` when the walk
/// reached it. Positions are one-based and taken from the order the two lists are in, so the result
/// depends on nothing but that order.
///
/// **What this buys is independence from the shape of the score distribution.** Whether `sum` lets a
/// reached memory pass a direct hit turns on an inherited level against the gap between two
/// own-scores, so a corpus whose best answers are tightly packed behaves differently from one whose
/// are well separated — and neither property has anything to do with the query. Positions have no
/// scale to be packed.
///
/// `own` must already be ordered best first. `k` damps the head of each list, and is the one
/// constant this rule costs.
#[must_use]
pub fn fuse_by_rank(
    own: &[(MemoryId, f64)],
    inherited: &HashMap<MemoryId, f64>,
    k: f64,
) -> Vec<(MemoryId, f64)> {
    let mut reached: Vec<(MemoryId, f64)> = inherited
        .iter()
        .filter(|(_, score)| score.is_finite())
        .map(|(id, score)| (*id, *score))
        .collect();
    reached.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let reached_at: HashMap<MemoryId, usize> = reached
        .iter()
        .enumerate()
        .map(|(position, (id, _))| (*id, position + 1))
        .collect();

    // A memory the walk never reached sits at the **end** of the inherited ranking rather than
    // absent from it. Leaving it out entirely was the first version and it was wrong in a way worth
    // recording: with no second term at all, *any* reached memory outranks *every* unreached one
    // whatever its content rank, so the head of the result became whatever the walk happened to
    // touch. Rank fusion assumes two rankings of the same population; a subset has to be completed,
    // not omitted.
    let unreached = own.len() + 1;

    let mut fused: Vec<(MemoryId, f64)> = own
        .iter()
        .enumerate()
        .map(|(position, (id, _))| {
            let walked = reached_at.get(id).copied().unwrap_or(unreached);
            #[allow(clippy::cast_precision_loss)]
            let score = 1.0 / (k + (position + 1) as f64) + 1.0 / (k + walked as f64);
            (*id, score)
        })
        .collect();
    fused.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    fused
}

/// The shape of one query's own-score distribution.
///
/// Reported because the rules differ in what they are sensitive to, and this says which sensitivity
/// a given corpus will exercise. `top_gap` is the one to read first: it is the distance from the
/// best match to the second, and it is what an inherited score has to clear for the walk to reorder
/// the head of the ranking.
#[derive(Debug, Clone, Copy)]
pub struct Spread {
    /// The mean of the scores.
    ///
    /// Near zero says that standardising the scores could not change any ranking, because then the
    /// own-scores are a pure scaling of their own z-scores and every rule here is scale-invariant.
    /// Far from zero says the opposite, and that a rule mixing terms at different hop counts is
    /// carrying an offset that came from where the centroid sits.
    pub mean: f64,
    /// The population standard deviation.
    pub deviation: f64,
    /// The drop from the best score to the second best.
    pub top_gap: f64,
}

impl Spread {
    /// Measure a ranking, or `None` when there is nothing to measure.
    ///
    /// Fewer than two scores has no gap and no meaningful deviation, and returning zeros would enter
    /// a fabricated point into the summary this exists to inform.
    #[must_use]
    pub fn over(scores: &[f64]) -> Option<Self> {
        if scores.len() < 2 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        let count = scores.len() as f64;
        let mean = scores.iter().sum::<f64>() / count;
        let deviation = (scores.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / count).sqrt();

        let mut ordered: Vec<f64> = scores.to_vec();
        ordered.sort_by(|a, b| b.total_cmp(a));
        Some(Self {
            mean,
            deviation,
            top_gap: ordered[0] - ordered[1],
        })
    }
}

/// Rank the corpus for a query, keeping the scores.
///
/// [`rank`] discards them, which is right for a caller that only wants an ordering. A cut-off rule
/// needs the shape of the decline, which is in the numbers rather than in the order.
#[must_use]
pub fn rank_scored(
    corpus: &Corpus,
    query: &Vector,
    walk: &Walk,
    combine: Combine,
) -> Vec<(MemoryId, f64)> {
    let inherited = reachable(corpus, query, walk);
    let ranked: Vec<(MemoryId, f64)> = corpus
        .vectors
        .nearest(query, corpus.size())
        .into_iter()
        .map(|hit| (hit.id, f64::from(hit.similarity)))
        .collect();

    // Fusion needs the whole ranking rather than one memory's pair of numbers, which is the property
    // that makes it insensitive to how the scores are distributed.
    if let Combine::RankFusion { k } = combine {
        return fuse_by_rank(&ranked, &inherited, k);
    }

    let mut scored: Vec<(MemoryId, f64)> = ranked
        .into_iter()
        .map(|(id, own)| {
            let carried = inherited.get(&id).copied().unwrap_or(0.0);
            let carried = if carried.is_finite() { carried } else { 0.0 };
            (id, combine.apply(own, carried))
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored
}

/// How many results to return: cut at the largest drop between consecutive scores.
///
/// The rule itself is [`traversal::cut_at_largest_gap`], which is what recall applies. This takes
/// the scored ranking the harness works in and hands it the scores.
#[must_use]
pub fn cut_at_largest_gap(scored: &[(MemoryId, f64)], window: usize) -> usize {
    let scores: Vec<f64> = scored.iter().map(|(_, score)| *score).collect();
    traversal::cut_at_largest_gap(&scores, window)
}

/// The share of what was returned that was actually wanted.
///
/// Recall alone cannot choose a cut-off, because returning the whole corpus scores a perfect 100%.
/// Precision is what makes the cost of an extra result visible, and here that cost is context spent
/// on a memory nobody asked for.
#[must_use]
pub fn precision_at(ranked: &[MemoryId], relevant: &[MemoryId], k: usize) -> f64 {
    let top = &ranked[..k.min(ranked.len())];
    if top.is_empty() {
        return 0.0;
    }
    let found = top.iter().filter(|id| relevant.contains(id)).count();
    #[allow(clippy::cast_precision_loss)]
    {
        found as f64 / top.len() as f64
    }
}

/// The share of a query's relevant memories appearing in the top `k`.
///
/// A query with no relevant memories scores zero rather than one. It is an undefined question rather
/// than a perfectly answered one, and callers are expected to exclude it from a mean rather than
/// letting it flatter or penalise every mechanism equally.
#[must_use]
pub fn recall_at(ranked: &[MemoryId], relevant: &[MemoryId], k: usize) -> f64 {
    if relevant.is_empty() {
        return 0.0;
    }
    let top: Vec<MemoryId> = ranked.iter().take(k).copied().collect();
    let found = relevant.iter().filter(|id| top.contains(id)).count();
    #[allow(clippy::cast_precision_loss)]
    {
        found as f64 / relevant.len() as f64
    }
}

/// Where each of `ids` ranks when the whole corpus is ranked against `query`.
///
/// Different from [`link_reach`] in what it excludes, and the difference is the point. A link's
/// source stands in for a query, so it is dropped from its own ranking; a real query is not a member
/// of the corpus, so nothing is dropped and every memory competes.
///
/// **This is the diagnostic that says whether a labelling collapsed into the mechanism being
/// tested.** Relevance labels are meant to be an independent answer key, but a labeller reading text
/// may simply be re-finding what content similarity already returns. If every labelled memory ranks
/// inside the top few, the labels carry no information the ranker does not already have, and
/// traversal's distinctive contribution — the memory relevant because of its relation rather than
/// its wording — is invisible by construction.
///
/// Ids absent from the corpus are skipped rather than reported as last: a fabricated rank would
/// quietly enter the distribution this exists to read.
#[must_use]
pub fn ranks_from_query(corpus: &Corpus, query: &Vector, ids: &[MemoryId]) -> Vec<usize> {
    let ranking: Vec<MemoryId> = corpus
        .vectors
        .nearest(query, corpus.size())
        .into_iter()
        .map(|hit| hit.id)
        .collect();

    ids.iter()
        .filter_map(|id| ranking.iter().position(|other| other == id))
        .map(|position| position + 1)
        .collect()
}

/// The share of measured targets a top-`k` cut would miss.
///
/// **This, rather than the median, is what says whether traversal earns its cost.** A distribution
/// can sit inside the cut at its median and still put a large minority well outside it, and that
/// minority is exactly the population traversal exists to reach — the memory relevant because of
/// its relation rather than its wording. Reporting only the median would average that minority away.
///
/// Returns zero for an empty slice rather than dividing by nothing.
#[must_use]
pub fn share_outside_cut(reaches: &[Reach], k: usize) -> f64 {
    if reaches.is_empty() {
        return 0.0;
    }
    let outside = reaches.iter().filter(|reach| reach.rank > k).count();
    #[allow(clippy::cast_precision_loss)]
    {
        outside as f64 / reaches.len() as f64
    }
}

/// What a run of the measurement found.
#[derive(Debug, Clone)]
pub struct Summary {
    /// Traversable edges whose target was ranked.
    pub measured: usize,
    /// Traversable edges that could not be ranked.
    pub unmeasurable: usize,
    /// The median of the measured percentiles, or zero when nothing was measured.
    pub median_percentile: f64,
    /// Memories carrying a vector.
    pub corpus_size: usize,
}

impl Summary {
    /// The corpus size at which the median linked target stops falling inside a top-`k` cut.
    ///
    /// Derived rather than restated, because it is the number the entire size argument rests on: at
    /// a 2.9% median and `k` of 5 it lands near 170 memories, which is what sent this project's own
    /// corpus from 56 memories to 429.
    ///
    /// Returns `None` when the median is zero, which means nothing was measured — there is no
    /// crossover to compute, and returning some large number would read as a finding.
    #[must_use]
    pub fn crossover_at(&self, k: usize) -> Option<usize> {
        if self.median_percentile <= 0.0 {
            return None;
        }
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation
        )]
        Some((k as f64 / self.median_percentile).ceil() as usize)
    }

    /// Whether this corpus is large enough for traversal to be distinguishable from a larger `k`.
    ///
    /// Below the crossover the two return the same memories, so **no number of queries can tell one
    /// combination rule from another** — which is why this is reported rather than left as
    /// arithmetic for a reader to do after the fact.
    #[must_use]
    pub fn discriminates_at(&self, k: usize) -> bool {
        self.crossover_at(k)
            .is_some_and(|crossover| self.corpus_size >= crossover)
    }
}

/// Measure a corpus and summarise it.
///
/// The median is taken over the **measured** edges alone. An unmeasurable edge has no percentile to
/// contribute, and counting it as anything would invent a number; its existence travels in
/// [`Summary::unmeasurable`] instead.
#[must_use]
pub fn summarise(corpus: &Corpus) -> Summary {
    let reaches = link_reach(corpus);
    let mut percentiles: Vec<f64> = reaches.iter().map(|reach| reach.percentile).collect();
    percentiles.sort_by(f64::total_cmp);

    Summary {
        measured: reaches.len(),
        unmeasurable: unmeasurable_edges(corpus),
        median_percentile: median(&percentiles),
        corpus_size: corpus.size(),
    }
}

/// The median of an already-sorted slice, or zero when it is empty.
fn median(sorted: &[f64]) -> f64 {
    match sorted.len() {
        0 => 0.0,
        // An even count averages the two middle values rather than picking one, so the reported
        // median does not depend on which side a tie-break happens to fall.
        n if n % 2 == 0 => (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0,
        n => sorted[n / 2],
    }
}
