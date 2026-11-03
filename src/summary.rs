//! ⚠️ **IMPORTANT:** ⚠️
//! This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
//! conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
//! it before making any changes to this file. After you are done modifying the source code, before
//! you commit your changes, check the Scatterhoard design documents and update them if necessary to
//! keep them current.
//!
//! What a memory set amounts to, derived from its own contents.
//!
//! This is the machinery half of [demand C](../documents/design/golden-rule.md): a set can amount to
//! something no record in it amounts to, and no session that sees one record at a time can notice
//! that. What can notice it is a description computed over the whole set and shown where the whole
//! set is in view — at export, and again at import.
//!
//! **It describes and does not judge.** There is no threshold here, no flag and no classifier. A
//! local tool has no telemetry to tune one against and no way to measure its false positives, and
//! judging would in any case be the inference from evidence to motive that demand D(ii) forbids.
//!
//! This module holds the labelling half: given a partition of a set's memories, what is each part
//! *called*. It knows nothing about how the partition was arrived at, and nothing about stores,
//! files or the MCP surface.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::embedding::{Vector, cosine};
use crate::memory::{Keyword, MemoryId};

/// Additive smoothing applied to both sides of the ratio, so a keyword absent outside its cluster
/// scores a large number rather than an infinite one.
///
/// Jeffreys' prior. It has to be applied to the proportions rather than to a raw count: a floor of
/// half a memory means one thing in a corpus of eight hundred and something quite different in a
/// corpus of four, and the constant-count form scored a genuinely distinctive keyword *negative* on
/// a three-memory cluster.
const SMOOTHING: f64 = 0.5;

/// One part of a partition: the keywords of each memory assigned to it.
///
/// Holds the keyword lists rather than indices into a set, so a cluster cannot be desynchronised
/// from the memories it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cluster {
    members: Vec<Vec<Keyword>>,
}

impl Cluster {
    /// Collect the memories assigned to one part of a partition, each as its list of keywords.
    ///
    /// A memory with no keywords is still a member: it counts toward the size, and only fails to
    /// contribute to the labelling.
    #[must_use]
    pub fn new(members: Vec<Vec<Keyword>>) -> Self {
        Self { members }
    }

    /// How many memories are in this cluster.
    #[must_use]
    pub fn size(&self) -> usize {
        self.members.len()
    }

    /// Each member's keywords, in the order the partition met them.
    #[must_use]
    pub fn members(&self) -> &[Vec<Keyword>] {
        &self.members
    }
}

/// What one cluster is called, and how much of the set it accounts for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterLabel {
    /// How many memories the cluster holds.
    pub size: usize,
    /// The keywords most distinctive to it, strongest first. Empty where nothing distinguishes the
    /// cluster from the rest of the set, which is reported rather than hidden.
    pub keywords: Vec<Keyword>,
}

/// Name each cluster by the keywords that appear in it more than they appear elsewhere.
///
/// Returns one label per cluster, **largest cluster first**, so the sizes still sum to the set and a
/// reader sees the set's bulk before its tail. Ties keep the order they were given in.
///
/// A keyword earns a place only where it is genuinely commoner inside its cluster than outside it,
/// so a term carried by the whole set — the project's own name, most often — labels nothing however
/// frequent it is. That is the difference between a summary that says what a set is about and one
/// that says what it is called.
///
/// Keywords are counted by [`Keyword::folded`], the same caseless key the store indexes under, so
/// `PID` and `pid` are one idea rather than two half-strength rivals. The spelling reported back is
/// whichever was used most, since [`Keyword`] deliberately preserves what its author wrote.
#[must_use]
pub fn label_clusters(clusters: &[Cluster], per_cluster: usize) -> Vec<ClusterLabel> {
    let total: usize = clusters.iter().map(Cluster::size).sum();
    let counts: Vec<Tally> = clusters.iter().map(Tally::of).collect();

    let mut corpus: BTreeMap<&str, usize> = BTreeMap::new();
    for tally in &counts {
        for (folded, occurrences) in &tally.by_folded {
            *corpus.entry(folded.as_str()).or_default() += occurrences.memories;
        }
    }

    let mut labelled: Vec<ClusterLabel> = clusters
        .iter()
        .zip(&counts)
        .map(|(cluster, tally)| ClusterLabel {
            size: cluster.size(),
            keywords: tally.distinctive(cluster.size(), total, &corpus, per_cluster),
        })
        .collect();

    // Stable, so clusters of equal size stay in the order the partition produced them.
    labelled.sort_by_key(|label| std::cmp::Reverse(label.size));
    labelled
}

/// How often each keyword occurs in one cluster, under its caseless key.
struct Tally {
    by_folded: BTreeMap<String, Occurrences>,
}

/// One keyword's presence in a cluster: how many of its memories carry it, and how it was spelled.
#[derive(Default)]
struct Occurrences {
    /// Counted once per memory. A memory repeating a keyword is not two pieces of evidence.
    memories: usize,
    spellings: BTreeMap<String, usize>,
}

impl Tally {
    fn of(cluster: &Cluster) -> Self {
        let mut by_folded: BTreeMap<String, Occurrences> = BTreeMap::new();
        for member in &cluster.members {
            let mut seen: BTreeMap<String, &Keyword> = BTreeMap::new();
            for keyword in member {
                seen.entry(keyword.folded()).or_insert(keyword);
            }
            for (folded, keyword) in seen {
                let entry = by_folded.entry(folded).or_default();
                entry.memories += 1;
                *entry
                    .spellings
                    .entry(keyword.as_str().to_owned())
                    .or_default() += 1;
            }
        }
        Self { by_folded }
    }

    fn distinctive(
        &self,
        size: usize,
        total: usize,
        corpus: &BTreeMap<&str, usize>,
        want: usize,
    ) -> Vec<Keyword> {
        let rest = total.saturating_sub(size);
        let mut scored: Vec<(f64, &str, &Occurrences)> = Vec::new();

        for (folded, occurrences) in &self.by_folded {
            let inside = occurrences.memories;
            let outside = corpus.get(folded.as_str()).copied().unwrap_or(inside) - inside;

            let p_inside = (inside as f64 + SMOOTHING) / (size as f64 + 2.0 * SMOOTHING);
            let p_outside = (outside as f64 + SMOOTHING) / (rest as f64 + 2.0 * SMOOTHING);

            // Weighted by how much of the cluster carries it, so a keyword on a single memory
            // cannot outrank one on half of them merely by being unique to it.
            let score = (p_inside / p_outside).ln() * (1.0 + inside as f64).ln();
            if score > 0.0 {
                scored.push((score, folded.as_str(), occurrences));
            }
        }

        scored.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.1.cmp(right.1))
        });

        scored
            .into_iter()
            .take(want)
            .filter_map(|(_, _, occurrences)| occurrences.dominant_spelling())
            .collect()
    }
}

impl Occurrences {
    /// The spelling this keyword was written with most often, ties broken alphabetically so the
    /// output does not depend on the order memories happened to arrive in.
    fn dominant_spelling(&self) -> Option<Keyword> {
        self.spellings
            .iter()
            .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)))
            .and_then(|(spelling, _)| Keyword::new(spelling.as_str()).ok())
    }
}

/// How far the cluster count grows with the size of the set.
///
/// Calibrated, not derived: `1.2 * ln(n)` reproduces the 8 clusters used on `scatterhoard` and the 6
/// used on `coding-lessons` when their output was read on 2026-09-12 and judged legible. Those two
/// readings are the only evidence this constant has, and [`clusters_for`] pins them in a test so
/// that changing it has to be deliberate.
const SPREAD: f64 = 1.2;

/// The most clusters a summary will ever report, however large the set.
///
/// A summary is read by a person or handed to a model at a gate; past a dozen lines it stops being a
/// description of the set and becomes a second corpus to read.
const MOST_CLUSTERS: usize = 12;

/// A set smaller than this is described whole rather than partitioned — there is no shape in four
/// memories that dividing them would reveal.
const TOO_SMALL_TO_PARTITION: usize = 3;

/// A cap on refinement passes in one run, not a budget: runs on the live sets settled in 14 to 17
/// passes on 2026-09-14, so the cap matters only on pathological input.
const MOST_REFINEMENTS: usize = 300;

/// How many seedings to try, keeping the partition whose memories sit closest to their centres.
///
/// Measured 2026-09-14 by `probe_how_stable_a_set_description_is`: with id-keyed seeds, keeping the
/// best of ten raised the worst agreement after removing one memory from 0.93 to 0.95 on
/// `scatterhoard` and from 0.57 to 0.98 on `coding-lessons`. **With positional seeds restarts made it
/// worse**, because every restart moved when a memory was inserted and the best of ten flipped between
/// near-equal groupings. Restarts help only when each restart's seeds are themselves stable.
const RESTARTS: u64 = 10;

/// One memory as the summary needs to see it: where it sits in the embedding space, and what it is
/// tagged with.
///
/// Deliberately carries no [`MemoryId`], only a number hashed from it. A summary reports what a set
/// is about and must not become a way to ask which memories are where, so the identifier is hashed
/// and dropped at the boundary, and nothing but seeding reads what is left.
#[derive(Debug, Clone)]
pub struct Subject {
    /// FNV-1a of the memory's id, so each memory's seeding draw belongs to it rather than to its
    /// position in the set — see [`initial_seeds`].
    key: u64,
    vector: Vector,
    keywords: Vec<Keyword>,
}

impl Subject {
    /// Take one memory's position and its keywords, keyed by its id.
    ///
    /// The id is hashed in its text form, which every machine spells the same way, so the key needs
    /// no agreement about floating-point arithmetic — which a key hashed from the vector would.
    #[must_use]
    pub fn new(id: &MemoryId, vector: Vector, keywords: Vec<Keyword>) -> Self {
        Self {
            key: fnv(FNV_OFFSET, id.to_string().bytes()),
            vector,
            keywords,
        }
    }
}

/// A digest of everything [`assignment`] reads from `subjects`, so a caller can keep a partition
/// until its input changes rather than work it out again.
///
/// The export review keeps one for each review: a page shows a handful of memories, and
/// partitioning the whole set to place them was most of every page's cost —
/// `each-review-page-recomputes-the-whole-set`. Each keyword's length goes in ahead of it, so
/// splitting one keyword into two changes the digest; each memory is already set off from the next
/// by its own key.
#[must_use]
pub fn fingerprint(subjects: &[Subject]) -> u64 {
    subjects.iter().fold(FNV_OFFSET, |state, subject| {
        let state = fnv(state, subject.key.to_le_bytes());
        let state = fnv(
            state,
            subject
                .vector
                .components()
                .iter()
                .flat_map(|component| component.to_bits().to_le_bytes()),
        );
        subject.keywords.iter().fold(state, |state, keyword| {
            let state = fnv(state, (keyword.as_str().len() as u64).to_le_bytes());
            fnv(state, keyword.as_str().bytes())
        })
    })
}

/// FNV-1a's starting state.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a, continued from `state` over `bytes`. Integer arithmetic only, so every machine draws the
/// same seeds.
fn fnv(mut state: u64, bytes: impl IntoIterator<Item = u8>) -> u64 {
    for byte in bytes {
        state ^= u64::from(byte);
        state = state.wrapping_mul(0x0100_0000_01b3);
    }
    state
}

/// A value in `[0, 1)` from a hash's high bits.
#[allow(clippy::cast_precision_loss)]
fn fraction(bits: u64) -> f64 {
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

/// How many clusters to divide a set of this size into.
///
/// Returns nothing for an empty set and a single part for one too small to partition. Otherwise it
/// grows with the logarithm of the size and stops at [`MOST_CLUSTERS`], because a summary is a thing
/// someone reads.
#[must_use]
pub fn clusters_for(memories: usize) -> usize {
    if memories == 0 {
        return 0;
    }
    if memories <= TOO_SMALL_TO_PARTITION {
        return 1;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let scaled = (SPREAD * (memories as f64).ln()).round() as usize;
    scaled.clamp(2, MOST_CLUSTERS).min(memories)
}

/// Group memories by what they are about, into at most `parts` clusters.
///
/// Empty clusters are dropped, so the result may be shorter than `parts`, and every memory appears
/// in exactly one cluster — the sizes always sum to the input.
///
/// **Deterministic on the same input**, which is load-bearing rather than tidy: the summary is
/// written into the exported file and recomputed at import so the two can be compared, and a seeded
/// or hash-ordered initialisation would make that comparison disagree with itself on identical data.
/// Ten seedings are tried and the most cohesive partition kept, earliest on a tie; each seeding is
/// drawn by keys that belong to the memories — see [`initial_seeds`] — and every tie in refinement
/// breaks toward the lower index.
///
/// **Stable under a small change as well as deterministic**, since 2026-09-14. Seeds used to be
/// drawn by position, so inserting one memory shifted every later draw and regrouped most of the set
/// — `a-set-description-reshuffles-on-a-small-change` — which turns a comparison across commits into
/// noise. Keyed draws move only when the inserted memory itself wins one.
///
/// # Preconditions
///
/// Every vector must have the same dimension, which a store guarantees because a
/// [`crate::embedding::VectorSet`] is built with one. Mixed dimensions yield a meaningless partition
/// rather than an error, since [`cosine`] compares only as far as the shorter vector.
#[must_use]
pub fn cluster(subjects: &[Subject], parts: usize) -> Vec<Cluster> {
    let assignment = assignment(subjects, parts);
    let parts = parts.clamp(1, subjects.len().max(1));
    let mut grouped: Vec<Vec<Vec<Keyword>>> = vec![Vec::new(); parts];
    for (index, subject) in subjects.iter().enumerate() {
        grouped[assignment[index]].push(subject.keywords.clone());
    }
    grouped
        .into_iter()
        .filter(|members| !members.is_empty())
        .map(Cluster::new)
        .collect()
}

/// Which part of the partition [`cluster`] settles on each subject falls in, in input order.
///
/// **Membership kept, for the export review alone**, which groups the memories it puts to a
/// session by subject (`documents/design/export-review.md`, *The procedure*). The description of a
/// set keeps only keyword lists precisely so it cannot be used to ask which memories are where;
/// the review is served to the person's own session about their own set, and grouping is its point.
#[must_use]
pub fn assignment(subjects: &[Subject], parts: usize) -> Vec<usize> {
    if subjects.is_empty() {
        return Vec::new();
    }
    let parts = parts.clamp(1, subjects.len());

    let mut best: Option<(f64, Vec<usize>)> = None;
    for restart in 0..RESTARTS {
        let (assignment, cohesion) = refine(subjects, parts, restart);
        let better = match &best {
            None => true,
            Some((held, _)) => cohesion > *held,
        };
        if better {
            best = Some((cohesion, assignment));
        }
    }
    best.expect("RESTARTS is at least one").1
}

/// What a set amounts to: its clusters, largest first, each named by what separates it.
///
/// The whole derivation in one call — the cluster count comes from [`clusters_for`], so a caller
/// chooses only how many keywords each label may carry.
#[must_use]
pub fn summarise(subjects: &[Subject], labels_per_cluster: usize) -> Vec<ClusterLabel> {
    let clusters = cluster(subjects, clusters_for(subjects.len()));
    label_clusters(&clusters, labels_per_cluster)
}

/// One seeding and its refinement: the partition it settles on, and its cohesion — the summed cosine
/// of every memory to its own centre, which is what a restart is judged by.
fn refine(subjects: &[Subject], parts: usize, restart: u64) -> (Vec<usize>, f64) {
    let mut centres: Vec<Vec<f32>> = initial_seeds(subjects, parts, restart)
        .into_iter()
        .map(|index| subjects[index].vector.components().to_vec())
        .collect();

    let mut assignment = vec![0usize; subjects.len()];
    for _ in 0..MOST_REFINEMENTS {
        let mut moved = false;
        for (index, subject) in subjects.iter().enumerate() {
            let nearest = nearest_centre(subject.vector.components(), &centres);
            if assignment[index] != nearest {
                assignment[index] = nearest;
                moved = true;
            }
        }
        if !moved {
            break;
        }
        recentre(subjects, &assignment, &mut centres);
    }

    let cohesion = subjects
        .iter()
        .zip(&assignment)
        .map(|(subject, part)| f64::from(cosine(subject.vector.components(), &centres[*part])))
        .sum();
    (assignment, cohesion)
}

/// Starting centres for one restart: the memory nearest the corpus centroid, then `k-means++` — each
/// further seed drawn with probability proportional to the square of its distance from everything
/// already chosen.
///
/// **Each memory's draw belongs to the memory, not to its position.** The draw is weighted sampling
/// by key: the memory maximising `ln(u) / w` wins with probability proportional to its weight `w`,
/// where `u` is hashed from the memory's key, the seed being chosen and the restart. Inserting a
/// memory changes nobody else's `u`, so the seeds move only if the newcomer wins a draw. Until
/// 2026-09-14 one generator was consumed by position, and an insertion shifted every draw after it:
/// `a-set-description-reshuffles-on-a-small-change`. Integer hashing keeps it identical on every
/// machine, which the export-versus-import comparison requires.
///
/// The first implementation took the *farthest* point each time rather than sampling. That needs no
/// randomness at all, and it was replaced because it is outlier-sensitive by construction: measured
/// on the live `scatterhoard` set on 2026-09-12 it seeded two clusters on single memories and let the
/// largest take 35% of the corpus, where sampling gives the even spread the labelling was judged
/// legible on.
fn initial_seeds(subjects: &[Subject], parts: usize, restart: u64) -> Vec<usize> {
    let dimension = subjects[0].vector.dimension();
    let mut mean = vec![0.0f32; dimension];
    for subject in subjects {
        for (slot, value) in mean.iter_mut().zip(subject.vector.components()) {
            *slot += *value;
        }
    }

    let mut seeds = vec![nearest_centre_index(&mean, subjects)];
    while seeds.len() < parts {
        let step = seeds.len() as u64;
        let pick = subjects
            .iter()
            .enumerate()
            .filter_map(|(index, subject)| {
                // Squared cosine distance to the closest seed. Zero for anything already chosen, so
                // a seed can never be drawn twice.
                let closest = seeds
                    .iter()
                    .map(|seed| {
                        cosine(
                            subject.vector.components(),
                            subjects[*seed].vector.components(),
                        )
                    })
                    .fold(f32::NEG_INFINITY, f32::max);
                let distance = f64::from(1.0 - closest).max(0.0);
                let weight = distance * distance;
                (weight > 0.0).then(|| {
                    let bits = fnv(
                        subject.key,
                        step.to_le_bytes().into_iter().chain(restart.to_le_bytes()),
                    );
                    (fraction(bits).max(f64::MIN_POSITIVE).ln() / weight, index)
                })
            })
            .max_by(|left, right| left.0.total_cmp(&right.0).then(right.1.cmp(&left.1)))
            .map(|(_, index)| index);
        match pick {
            Some(index) => seeds.push(index),
            // Every remaining memory sits exactly on a seed, so there is nothing left to separate.
            None => break,
        }
    }
    seeds
}

/// The index of the subject most similar to `target`, ties going to the lower index.
fn nearest_centre_index(target: &[f32], subjects: &[Subject]) -> usize {
    let mut best = (f32::NEG_INFINITY, 0usize);
    for (index, subject) in subjects.iter().enumerate() {
        let similarity = cosine(subject.vector.components(), target);
        if similarity > best.0 {
            best = (similarity, index);
        }
    }
    best.1
}

/// Which centre a vector belongs to, ties going to the lower centre so a point on a boundary does
/// not oscillate between passes.
fn nearest_centre(vector: &[f32], centres: &[Vec<f32>]) -> usize {
    let mut best = (f32::NEG_INFINITY, 0usize);
    for (index, centre) in centres.iter().enumerate() {
        let similarity = cosine(vector, centre);
        if similarity > best.0 {
            best = (similarity, index);
        }
    }
    best.1
}

/// Move each centre to the mean of its members. A centre that lost every member keeps its position,
/// so it can still attract on a later pass rather than collapsing the partition.
fn recentre(subjects: &[Subject], assignment: &[usize], centres: &mut [Vec<f32>]) {
    let dimension = centres[0].len();
    let mut sums = vec![vec![0.0f32; dimension]; centres.len()];
    let mut counts = vec![0usize; centres.len()];

    for (index, subject) in subjects.iter().enumerate() {
        let part = assignment[index];
        counts[part] += 1;
        for (slot, value) in sums[part].iter_mut().zip(subject.vector.components()) {
            *slot += *value;
        }
    }

    for (part, centre) in centres.iter_mut().enumerate() {
        if counts[part] == 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let divisor = counts[part] as f32;
        for (slot, total) in centre.iter_mut().zip(&sums[part]) {
            *slot = total / divisor;
        }
    }
}

/// The identity of this derivation, recorded in every summary it produces.
///
/// **Bump it whenever the clustering, the seeding or the labelling changes**, so a reader of two
/// descriptions can tell a different rule from a different set. It was first required for comparing
/// a file's stored description with a recomputation; that comparison was retired 2026-09-21, with
/// the stored copy. The name stays because every description shows it, and anyone setting two
/// descriptions side by side needs to know whether the rule between them changed.
///
/// `clusters/2`, 2026-09-14: seeds drawn by keys hashed from memory ids rather than by position, and
/// the most cohesive of ten seedings kept. `clusters/1` drew by position from one generator.
pub const DERIVATION: &str = "clusters/2";

/// What a set amounts to, derived from its own memories whenever it is shown.
///
/// **Never stored, and so never travels.** Until 2026-09-21 it was written into an exported file's
/// header as a claim an import compared against; the header then changed on every export, which made
/// it the line two branches collided on, and the comparison caught nothing a sender could not avoid
/// by deleting the field. It holds no memory identifiers: a summary says what a set is about and is
/// not a way to ask which memories are where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSummary {
    /// Which derivation produced this.
    pub derivation: String,
    /// The clusters, largest first.
    pub clusters: Vec<ClusterLabel>,
}

impl SetSummary {
    /// Derive a set's description from its memories.
    #[must_use]
    pub fn derive(subjects: &[Subject], labels_per_cluster: usize) -> Self {
        Self {
            derivation: DERIVATION.to_owned(),
            clusters: summarise(subjects, labels_per_cluster),
        }
    }
}
