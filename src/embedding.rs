// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Vector storage and the similarity scan.
//!
//! See `documents/design/embeddings.md`. Three decisions from there shape this module:
//!
//! - **Vectors are local derived state.** They are never exported, so nothing here has merge
//!   semantics or a wire format to keep stable, and changing the embedding model is a local
//!   re-embed rather than an ecosystem-wide migration.
//! - **Brute force, no index.** Ten thousand memories at 384 dimensions is roughly 15 MB and a few
//!   million multiply-adds — single-digit milliseconds. `sqlite-vec` is the upgrade path if a set
//!   ever passes ~100k memories, and it fits behind this same interface.
//! - **Raw vectors are stored and the centroid subtracted at scan time**, rather than storing
//!   centred vectors. The centroid moves whenever a memory is added, so centring on write would
//!   invalidate every vector already stored. Subtracting during a scan that already touches every
//!   vector is free.

use std::collections::HashMap;

use thiserror::Error;

use crate::memory::{Memory, MemoryId};

/// The text embedded for a memory: content, then `why`, then keywords, space-separated.
///
/// Measured rather than chosen. The spike scored four candidate compositions on an identical
/// corpus and metric, and this one reached 71.7% recall@5 against 57.5% for content alone — both
/// additions contributing, and composing. See `documents/design/recall-mechanisms.md`.
///
/// `context` is deliberately absent. It is an episodic anchor — what was being worked on when a
/// memory arose — so embedding it would pull semantically unrelated memories together merely
/// because they happened around the same time. That adjacency is exactly what makes it useful to a
/// human, whose recall follows a session's through-line, and useless to a similarity search. It
/// earns its place on the output side, helping a human place a memory that recall has already
/// found.
///
/// Never empty: content is non-empty by construction, which matters because an empty string embeds
/// to a vector with no direction, and [`Vector::new`] refuses those.
#[must_use]
pub fn embedding_input(memory: &Memory) -> String {
    embedding_input_of(
        memory.content.as_data(),
        memory.why.as_ref().map(crate::memory::MemoryText::as_data),
        &memory.keywords,
    )
}

/// The same text, assembled from the parts rather than from a [`Memory`].
///
/// **One implementation, because the write door measures what the embedder will receive.** A length
/// refused at the door and a length embedded afterwards have to be the same text, and two
/// assemblies of it would be free to drift — the shape `TRACKING.md` files under *source text
/// quoted into another file*, one layer up.
pub fn embedding_input_of(
    content: &str,
    why: Option<&str>,
    keywords: &[crate::memory::Keyword],
) -> String {
    let mut parts: Vec<&str> = vec![content];
    if let Some(why) = why {
        parts.push(why);
    }
    parts.extend(keywords.iter().map(crate::memory::Keyword::as_str));
    parts.join(" ")
}

/// How long a memory may be, counted in characters of [`embedding_input`].
///
/// **2000, chosen from this project's own corpus rather than from the model.** Not one memory
/// written in Scatterhoard's first five weeks reached it — median 998, p90 1259, longest 1565 —
/// and the design was produced in that stretch. The number is in characters rather than tokens
/// because tokenising needs the model and the keyword path is the no-model path; a store that
/// refused by tokens could not answer at all there.
///
/// **It is not a proxy for the embedder's budget.** It was one while `encode` truncated at 512
/// tokens, and it stopped being one on 2026-09-19 when the encoding stopped truncating. What it
/// bounds now is what a memory *is*: a record this long is usually several claims, and the
/// relationships between them — which link labels would have carried, and which a later memory
/// could have pointed at one at a time — are buried in prose instead.
///
/// A character is a Unicode scalar, not a byte, so a set written in a script that costs more bytes
/// per character is not given a shorter memory than an English one.
pub const MAX_EMBEDDING_INPUT_CHARS: usize = 2000;

/// Why a vector could not be built or stored.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum EmbeddingError {
    /// The vector had no components.
    #[error("a vector must have at least one component")]
    Empty,

    /// Every component was zero, so the vector has no direction.
    ///
    /// Cosine similarity divides by magnitude, so such a vector would score `NaN` against
    /// everything. Excluding it here is what lets the scan sort without defending against that.
    #[error("a vector must have a direction, and every component was zero")]
    NoDirection,

    /// A component was `NaN` or an infinity.
    #[error("a vector component was not finite")]
    NotFinite,

    /// The vector's dimension does not match the set's.
    ///
    /// Means the embedding model changed, which the design handles by re-embedding the set rather
    /// than by comparing across dimensions.
    #[error("expected a {expected}-dimensional vector, got {found}")]
    WrongDimension {
        /// The set's dimension.
        expected: usize,
        /// The offending vector's dimension.
        found: usize,
    },
}

/// An embedding, as the model produced it.
///
/// Stored raw. Guaranteed non-empty, finite, and with at least one non-zero component, which is
/// what makes every similarity computed from it a real number.
#[derive(Debug, Clone, PartialEq)]
pub struct Vector(Vec<f32>);

impl Vector {
    /// Wrap a model's output.
    ///
    /// # Errors
    ///
    /// See [`EmbeddingError`]. A vector of all zeros is refused rather than stored.
    pub fn new(components: Vec<f32>) -> Result<Self, EmbeddingError> {
        if components.is_empty() {
            return Err(EmbeddingError::Empty);
        }
        if components.iter().any(|value| !value.is_finite()) {
            return Err(EmbeddingError::NotFinite);
        }
        if components.iter().all(|value| *value == 0.0) {
            return Err(EmbeddingError::NoDirection);
        }
        Ok(Self(components))
    }

    /// The components.
    #[must_use]
    pub fn components(&self) -> &[f32] {
        &self.0
    }

    /// How many dimensions this vector has.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.0.len()
    }
}

/// A memory and how well it matched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// The memory that matched.
    pub id: MemoryId,
    /// Cosine similarity, always finite and in `-1.0..=1.0`.
    pub similarity: f32,
}

/// Every vector in one store, and the scan over them.
///
/// Belongs to a single scope's store, because storage is partitioned by scope — so a scan here
/// structurally cannot reach another scope's memories.
#[derive(Debug, Clone)]
pub struct VectorSet {
    dimension: usize,
    vectors: HashMap<MemoryId, Vec<f32>>,
    /// Componentwise sum of everything stored, so the centroid is an O(d) division rather than a
    /// second pass over the whole set on every query.
    sum: Vec<f32>,
}

impl VectorSet {
    /// An empty set for vectors of the given dimension.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension,
            vectors: HashMap::new(),
            sum: vec![0.0; dimension],
        }
    }

    /// How many memories have vectors.
    #[must_use]
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// Whether any memory has a vector.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Store a memory's vector, replacing any earlier one.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingError::WrongDimension`] if the vector does not match the set.
    pub fn insert(&mut self, id: MemoryId, vector: Vector) -> Result<(), EmbeddingError> {
        if vector.dimension() != self.dimension {
            return Err(EmbeddingError::WrongDimension {
                expected: self.dimension,
                found: vector.dimension(),
            });
        }

        if let Some(previous) = self.vectors.insert(id, vector.0.clone()) {
            for (total, old) in self.sum.iter_mut().zip(&previous) {
                *total -= old;
            }
        }
        for (total, new) in self.sum.iter_mut().zip(&vector.0) {
            *total += new;
        }
        Ok(())
    }

    /// Forget a memory's vector.
    pub fn remove(&mut self, id: MemoryId) {
        if let Some(previous) = self.vectors.remove(&id) {
            for (total, old) in self.sum.iter_mut().zip(&previous) {
                *total -= old;
            }
        }
    }

    /// One memory's vector, if it has one.
    ///
    /// Exposed so that a caller ranking one memory against the rest can supply that memory's own
    /// vector as the query — which is what the evaluation harness does, using the source of a link
    /// as a stand-in for a query that lands exactly on it. Without this the caller would have to
    /// keep its own copy of every vector beside the set, which is a second fact free to drift from
    /// the first.
    #[must_use]
    pub fn get(&self, id: MemoryId) -> Option<Vector> {
        self.vectors
            .get(&id)
            .map(|components| Vector(components.clone()))
    }

    /// The componentwise sum of everything stored, with the count.
    ///
    /// Exposed so a *combined* centroid over several sets is O(d) arithmetic rather than a second
    /// pass over every vector — see [`combined_centroid`]. Without this, comparing scores across
    /// stores would mean rescanning them all on every query.
    #[must_use]
    pub fn running_sum(&self) -> (&[f32], usize) {
        (&self.sum, self.vectors.len())
    }

    /// The mean of the stored vectors, or `None` when the set is empty.
    #[must_use]
    pub fn centroid(&self) -> Option<Vector> {
        if self.vectors.is_empty() {
            return None;
        }
        let count = self.vectors.len() as f32;
        Some(Vector(self.sum.iter().map(|total| total / count).collect()))
    }

    /// The `k` memories most similar to `query`, with the corpus centroid removed from both sides.
    ///
    /// Removing the mean from both the query and the memories lifted recall@5 from 71.7% to 77.5%
    /// in the spike, and cut the worst hub from 13 of 20 queries to 6. It costs one subtraction per
    /// vector, on a scan that already visits every one.
    ///
    /// Ranked highest first. Never returns a `NaN` score: a centred vector with no direction left
    /// — which happens when a memory *is* the centroid, as in a one-memory set — scores zero
    /// rather than dividing by zero.
    #[must_use]
    pub fn nearest(&self, query: &Vector, k: usize) -> Vec<Hit> {
        let centroid = self.centroid();
        self.scan(query, k, centroid.as_ref().map(|c| c.0.as_slice()))
    }

    /// The same scan, centred on a centroid supplied from outside.
    ///
    /// Used when several stores are searched together: they must all be centred on the mean of
    /// their *union*, or their scores are on different scales and merging them is unsound.
    #[must_use]
    pub fn nearest_centred_on(&self, query: &Vector, k: usize, centroid: &Vector) -> Vec<Hit> {
        self.scan(query, k, Some(&centroid.0))
    }

    /// The same scan without centroid removal.
    ///
    /// Kept because cross-store comparison may need raw cosine — scores centred against two
    /// different corpora are not on a comparable scale, which is an open question in
    /// `TRACKING.md`.
    #[must_use]
    pub fn nearest_uncentred(&self, query: &Vector, k: usize) -> Vec<Hit> {
        self.scan(query, k, None)
    }

    /// One memory's **uncentred** cosine against a query, or `None` if this set does not hold it.
    ///
    /// The question a floor asks, and it cannot be answered from the ranking's scores: those carry
    /// the corpus centroid removed, which is the direction that marks a query as out of domain.
    /// Asked per hit rather than by re-scanning, because a floor only ever judges the handful of
    /// memories that reached the window.
    #[must_use]
    pub fn raw_similarity(&self, id: MemoryId, query: &Vector) -> Option<f32> {
        self.get(id).map(|vector| cosine(&vector.0, &query.0))
    }

    fn scan(&self, query: &Vector, k: usize, centroid: Option<&[f32]>) -> Vec<Hit> {
        let centred_query = centre(&query.0, centroid);

        let mut hits: Vec<Hit> = self
            .vectors
            .iter()
            .map(|(id, vector)| Hit {
                id: *id,
                similarity: cosine(&centre(vector, centroid), &centred_query),
            })
            .collect();

        // `similarity` is always finite, so this comparison is total.
        hits.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .expect("similarity is finite by construction")
                .then_with(|| a.id.cmp(&b.id))
        });
        hits.truncate(k);
        hits
    }
}

/// The mean over the union of several sets.
///
/// This is what makes cross-store scores comparable: centring every store on the same vector means
/// they are all scored against one corpus, which is what a caller searching project ∪ local
/// actually has. Cheap because each set keeps a running sum, so this is a few additions and one
/// division rather than a pass over every vector.
///
/// `None` when every set is empty, or when they disagree about dimension — which means the stores
/// were embedded by different models and their vectors are not comparable at all.
#[must_use]
pub fn combined_centroid(sets: &[&VectorSet]) -> Option<Vector> {
    let dimension = sets.iter().find(|set| !set.is_empty())?.dimension;
    let mut total = vec![0.0f32; dimension];
    let mut count = 0usize;

    for set in sets {
        let (sum, len) = set.running_sum();
        if len == 0 {
            continue;
        }
        if sum.len() != dimension {
            return None;
        }
        for (running, part) in total.iter_mut().zip(sum) {
            *running += part;
        }
        count += len;
    }

    if count == 0 {
        return None;
    }
    let count = count as f32;
    Some(Vector(total.into_iter().map(|v| v / count).collect()))
}

fn centre(vector: &[f32], centroid: Option<&[f32]>) -> Vec<f32> {
    match centroid {
        None => vector.to_vec(),
        Some(mean) => vector
            .iter()
            .zip(mean)
            .map(|(value, average)| value - average)
            .collect(),
    }
}

/// Cosine similarity, defined as zero when either side has no direction left after centring.
///
/// Public so that [`crate::summary`] can group vectors by the same measure recall ranks them by,
/// rather than growing a second copy of it.
#[must_use]
pub fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;

    for (a, b) in left.iter().zip(right) {
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }

    let magnitude = (left_norm * right_norm).sqrt();
    if magnitude == 0.0 {
        0.0
    } else {
        dot / magnitude
    }
}
