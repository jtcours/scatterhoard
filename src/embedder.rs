// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Turning memory text into vectors.
//!
//! Separated from [`crate::embedding`], which owns the vectors once they exist: the arithmetic
//! there is testable without a model, and keeping the model behind [`Embedder`] is what lets it
//! stay that way.
//!
//! # This build cannot reach the network
//!
//! `model2vec-rs` is compiled with `local-only` and without `hf-hub`, so remote fetching is not
//! something Scatterhoard declines to do — it is absent from the binary. That is deliberate, and it
//! is the whole guarantee: a developer who installs this, works for a week, and then opens a laptop
//! on a plane must not discover a download requirement they were never told about. See
//! `documents/design/embeddings.md`.
//!
//! The model therefore has to be on disk before Scatterhoard runs, and acquiring it is an explicit
//! setup step — never lazy, never triggered by a first recall or an unfamiliar project.

use std::path::{Path, PathBuf};

use model2vec_rs::model::StaticModel;
use thiserror::Error;

use crate::embedding::{EmbeddingError, Vector};

/// The model Scatterhoard expects unless told otherwise.
///
/// Measured against `all-MiniLM-L6-v2` on an identical corpus and metric, where it reached 77.5%
/// recall@5 to MiniLM's 74.2% — so the published benchmark gap does not appear on this workload,
/// and the transformer's size and dependency cost buys nothing here.
pub const DEFAULT_MODEL: &str = "potion-base-32M";

/// Why embedding was unavailable or failed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EmbedderError {
    /// No model was found where one was expected.
    ///
    /// The message names the fix rather than only reporting the failure, because in the common case
    /// that saves anyone — human or LLM — from having to go and find the setup instructions.
    #[error(
        "no embedding model at {path}. Scatterhoard needs one before it can search memories by \
         meaning; keyword search and direct lookup still work without it. Download {model} and \
         place it there, or point Scatterhoard at an existing copy. This is a one-time setup step: \
         Scatterhoard never downloads anything itself."
    )]
    ModelMissing {
        /// Where a model was looked for.
        path: PathBuf,
        /// The model that was expected.
        model: &'static str,
    },

    /// The model directory exists but could not be loaded.
    #[error("the embedding model at {path} could not be loaded: {detail}")]
    ModelUnreadable {
        /// Where the model was.
        path: PathBuf,
        /// What went wrong.
        detail: String,
    },

    /// The model produced something that is not a usable vector.
    #[error("the embedding model produced an unusable vector")]
    Unusable(#[from] EmbeddingError),
}

/// Which model produced a vector.
///
/// Opaque on purpose: nothing compares two of these for anything but equality, and giving it
/// structure would invite reading a version ordering into it that no model file promises.
///
/// Stored in a store's `meta` and compared against the loaded model on open — the mechanism
/// `documents/design/embeddings.md` requires for *one store, one model, and no mixture*. Without it
/// a swapped model leaves every stored vector being scanned against queries from a different vector
/// space, which computes perfectly and means nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelIdentity(String);

impl ModelIdentity {
    /// Name a model.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The recorded form, as it is written to and read from a store.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModelIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identify a model file by its contents.
///
/// **By contents, not by path**, because the failure this exists to catch is a different model
/// arriving where the last one was: same configured path, same filename, different bytes. A path
/// would report those as the same model, which is the case that currently goes unnoticed.
///
/// Reuses the reconciliation hash — a change detector, explicitly not a security boundary, which is
/// exactly the claim being made here. Nothing about this participates in reconciliation; it borrows
/// the function, not the meaning.
///
/// Reads the whole file, which for `potion-base-32M` is roughly 129 MB — the name counts 32 million
/// parameters, not bytes. That cost is paid once when a model is loaded, and it buys the only
/// identity that cannot be faked by moving a file.
///
/// # Errors
///
/// Returns the underlying [`std::io::Error`] if the file cannot be read.
pub fn identity_of_file(path: &Path) -> std::io::Result<ModelIdentity> {
    let bytes = std::fs::read(path)?;
    Ok(ModelIdentity(format!(
        "fnv1a64:{:016x}",
        crate::checksum::of_bytes(&bytes)
    )))
}

/// How this process feeds text to the model, as it is recorded in the identity.
///
/// `whole` says the text is encoded entire. It is in the identity because the same weights fed the
/// same memory two ways produce two different vectors, and a store cannot be maintained by a
/// process that would produce the other one.
const ENCODING: &str = "whole";

/// How many texts go to the model at once. `model2vec-rs`'s own default, kept because nothing here
/// has a reason to prefer another and a number chosen without one is a number nobody can revisit.
const BATCH: usize = 1024;

/// The identity a store records: the model's bytes, and how this process encodes text with them.
///
/// **The encoding belongs in the identity because the vectors depend on it.** Until 2026-09-19
/// the identity was the weights alone, while `StaticModel::encode` truncated every input at 512
/// tokens — so changing that would have left re-encoded vectors and truncated ones in one store,
/// comparable to each other and indistinguishable to the check that exists to catch exactly this.
/// That is `no-record-of-which-model-built-the-vectors` again, with the weights hash making it look
/// accounted for.
///
/// The weights stay at the front so the two can be read apart: a human comparing two servers sees
/// at once whether the model differs or only the encoding does.
#[must_use]
pub fn identity_with_encoding(model: &ModelIdentity) -> ModelIdentity {
    ModelIdentity(format!("{}+{ENCODING}", model.as_str()))
}

/// What a model made of one text.
///
/// **Two outcomes, and neither is an error.** A text in a script the model has no tokens for is not
/// a fault in the model or in the text: every piece is `[UNK]`, is dropped before pooling, and
/// nothing is left to average. Reporting that per text is what lets such a memory be stored without
/// a vector, instead of failing the batch it arrived in — which is how one Chinese memory used to
/// stop a whole store, `one-unembeddable-memory-stops-the-store` in `TRACKING.md`.
#[derive(Debug, Clone)]
pub enum Embedded {
    /// The text's vector.
    Vector(Vector),
    /// The model has nothing in its vocabulary to represent this text with.
    Unrepresentable,
}

/// Classify one pooled output.
///
/// **All zeros is a property of the text; anything else wrong is a property of the model.** Mean
/// pooling over no tokens yields exactly zero — `model2vec-rs` clamps both the count and the norm,
/// so it is zeros rather than `NaN` — and that is the only outcome a text alone can cause. A
/// non-finite component or a vector with no components means the model is broken, and calling that
/// unrepresentable would hide corruption behind a reading that sounds like a language gap.
///
/// Public so the classification is testable without a model, and so a test double can classify its
/// output exactly as [`StaticEmbedder`] does rather than restating the rule.
///
/// # Errors
///
/// [`EmbedderError::Unusable`] for any vector that is wrong for a reason other than being empty of
/// meaning.
pub fn embedded_from(components: Vec<f32>) -> Result<Embedded, EmbedderError> {
    match Vector::new(components) {
        Ok(vector) => Ok(Embedded::Vector(vector)),
        Err(EmbeddingError::NoDirection) => Ok(Embedded::Unrepresentable),
        Err(other) => Err(EmbedderError::from(other)),
    }
}

/// Turns text into vectors.
///
/// A trait so that the store and recall depend on the *capability* rather than on `model2vec-rs`,
/// which keeps the embedder a locally reversible choice — the whole point of vectors being local
/// derived state that never leaves the machine.
pub trait Embedder {
    /// Which model this is.
    ///
    /// Required rather than defaulted. A default would have to invent an identity, and every
    /// embedder that did not override it would claim to be the same model as every other — the
    /// silent-agreement failure that a store recording its model exists to prevent.
    fn identity(&self) -> ModelIdentity;

    /// How many components the vectors have.
    fn dimension(&self) -> usize;

    /// Embed a batch, one outcome per text, in the order given.
    ///
    /// Batched rather than one at a time because import re-embeds whatever changed, and that sits
    /// on the critical path of an ordinary `git pull`.
    ///
    /// **An outcome per text, not one for the batch.** A text the model has no tokens for is
    /// [`Embedded::Unrepresentable`] and costs only itself. Until 2026-09-11 it failed the whole
    /// batch, and the store's gap sweep sends every gap through one batch, so a single such memory
    /// stopped every later write — `one-unembeddable-memory-stops-the-store`. The error is for a
    /// model that cannot embed at all.
    ///
    /// # Errors
    ///
    /// See [`EmbedderError`].
    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError>;

    /// Embed a batch that must come back as vectors, failing if any text cannot be represented.
    ///
    /// **For measurement, never for the store.** A probe or an evaluation wants a vector for every
    /// input and should stop loudly when it cannot have one. The store must not call this: one
    /// unrepresentable memory would fail the batch it arrived in, which is the defect
    /// [`embed_each`](Self::embed_each) exists to prevent.
    ///
    /// # Errors
    ///
    /// See [`EmbedderError`]. A text with no representation is [`EmbedderError::Unusable`], as it
    /// always was.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedderError> {
        self.embed_each(texts)?
            .into_iter()
            .map(|outcome| match outcome {
                Embedded::Vector(vector) => Ok(vector),
                Embedded::Unrepresentable => {
                    Err(EmbedderError::Unusable(EmbeddingError::NoDirection))
                }
            })
            .collect()
    }
}

/// A static-embedding model loaded from disk.
pub struct StaticEmbedder {
    model: StaticModel,
    dimension: usize,
    identity: ModelIdentity,
}

// Written out rather than derived because the underlying model is not `Debug`, and printing tens of
// megabytes of weights would be unhelpful even if it were.
impl std::fmt::Debug for StaticEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticEmbedder")
            .field("dimension", &self.dimension)
            .finish_non_exhaustive()
    }
}

impl StaticEmbedder {
    /// Load a model from a directory.
    ///
    /// # Errors
    ///
    /// Returns [`EmbedderError::ModelMissing`] if nothing is there, which is the case worth
    /// distinguishing: it is a setup step not yet taken, not a fault.
    pub fn load(path: &Path) -> Result<Self, EmbedderError> {
        let weights = path.join("model.safetensors");
        if !weights.exists() {
            return Err(EmbedderError::ModelMissing {
                path: path.to_path_buf(),
                model: DEFAULT_MODEL,
            });
        }

        // Taken from the weights alone. The tokeniser and config beside them affect how text is
        // encoded, so a stricter identity would cover the directory — but a change to those without
        // a change to the weights is not a case that arises from the way models are distributed,
        // and hashing one 32 MB file is already the expensive part.
        let identity = identity_with_encoding(&identity_of_file(&weights).map_err(|error| {
            EmbedderError::ModelUnreadable {
                path: path.to_path_buf(),
                detail: error.to_string(),
            }
        })?);

        let model = StaticModel::from_pretrained(path, None, None, None).map_err(|error| {
            EmbedderError::ModelUnreadable {
                path: path.to_path_buf(),
                detail: error.to_string(),
            }
        })?;

        // Asked of the model rather than assumed, since it is what every stored vector is checked
        // against and a wrong guess would only surface as a dimension mismatch much later.
        let dimension = model
            .encode(&["probe".to_owned()])
            .first()
            .map_or(0, Vec::len);

        Ok(Self {
            model,
            dimension,
            identity,
        })
    }
}

impl Embedder for StaticEmbedder {
    fn identity(&self) -> ModelIdentity {
        self.identity.clone()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        // `encode` would be `encode_with_args(texts, Some(512), 1024)`, and that 512 is a default
        // rather than a limit the model imposes: a static model mean-pools token vectors and has no
        // context window to overrun. What the default removed was the *end* of the input — a
        // memory's `why`, then its keywords — silently, for 12.5% of the corpus by 2026-09-19.
        // `None` disables both the character pre-truncation and the token-id truncation.
        self.model
            .encode_with_args(texts, None, BATCH)
            .into_iter()
            .map(embedded_from)
            .collect()
    }
}
