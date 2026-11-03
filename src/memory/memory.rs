// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! A memory: the record the whole system exists to store, recall, and merge.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{Attribution, Confidence, Keyword, Link, MemoryId, MemoryText, Origin};

/// A single memory.
///
/// See `documents/design/memory-data-model.md`. Field order here is the field order in the
/// exported file, and changing it changes every line of every export — so it is fixed
/// deliberately, most-identifying first.
///
/// **Scope is not a field.** An exported file *is* a memory set, so every memory in it shares one
/// scope; carrying it per-memory would create a value that can disagree with the file containing
/// it, and a check to keep the two in step. The set is declared once, in the file header — see
/// [`crate::memory_file`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    /// Globally unique, and time-ordered, so sorting by it is sorting by creation time.
    pub id: MemoryId,

    /// The memory itself.
    pub content: MemoryText,

    /// The reasoning that led to the content, if it was recorded.
    ///
    /// Optional in the type although the conventions push hard for it, and deliberately so: a type
    /// that demanded reasoning would make the LLM invent some when it has none, which produces a
    /// worse memory than an honest omission. The requirement belongs in convention, where it can be
    /// a strong expectation rather than a hard gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<MemoryText>,

    /// An episodic cue: what was being worked on when this arose, named as a person would say it.
    ///
    /// Deliberately kept out of the embedding input: it is about a specific past occasion, which is
    /// exactly the kind of thing that would pull semantically unrelated memories together. It earns
    /// its place on the output side, letting a human place a memory once recall has found it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<MemoryText>,

    /// Curated topical keywords, part of the embedding input and of keyword search.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<Keyword>,

    /// How much the author trusts this memory.
    pub confidence: Confidence,

    /// Whose reasoning produced the content, as distinct from whose identity is attached to it.
    pub origin: Origin,

    /// Who first recorded this memory, and when.
    pub created: Attribution,

    /// Who last changed it, and when. Equal to [`Memory::created`] until the first revision.
    pub modified: Attribution,

    /// Labelled edges to other memories, stored on this memory and travelling with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
}

impl Memory {
    /// Record a new memory, minting its identifier.
    ///
    /// The optional fields start empty and are filled in by the caller; everything required to make
    /// the memory well-formed is a parameter here, so a partially-initialised memory never exists.
    /// `modified` starts equal to `created`, which is what "never revised" means.
    #[must_use]
    pub fn new(
        content: MemoryText,
        confidence: Confidence,
        origin: Origin,
        created: Attribution,
    ) -> Self {
        Self {
            id: MemoryId::generate(),
            content,
            why: None,
            context: None,
            keywords: Vec::new(),
            confidence,
            origin,
            modified: created.clone(),
            created,
            links: Vec::new(),
        }
    }

    /// A memory that attributes itself, for the one record that has nothing earlier to point at.
    ///
    /// **The first author record in a set is its own author.** `by` holds a [`MemoryId`], so the
    /// record naming an identity has to point somewhere, and there is no prior identity in a set
    /// that has none. Self-reference is well defined here because `by` is a field rather than an
    /// edge in `links`, so nothing traverses it and no cycle can form in the graph.
    ///
    /// It has its own constructor rather than being assembled by overwriting `by` after
    /// construction, because that spelling leaves a dangling reference the moment somebody forgets
    /// the second half, and a dangling `by` is the one failure this design adds.
    #[must_use]
    pub fn self_attributed(
        content: MemoryText,
        confidence: Confidence,
        origin: Origin,
        at: Timestamp,
    ) -> Self {
        let id = MemoryId::generate();
        let mut memory = Self::new(content, confidence, origin, Attribution { by: id, at });
        memory.id = id;
        memory
    }

    /// Record that this memory was changed, by whom and when.
    ///
    /// The only way to move `modified`, so the timestamp cannot advance without the author moving
    /// with it.
    pub fn revise(&mut self, by: MemoryId, at: Timestamp) {
        self.modified = Attribution { by, at };
    }

    /// Whether this memory has been changed since it was first recorded.
    #[must_use]
    pub fn is_revised(&self) -> bool {
        self.modified != self.created
    }

    /// Whether this is a set's identity record rather than an ordinary memory.
    ///
    /// **Asked of the record's shape, never of a keyword.** Every identity record is built by
    /// [`Memory::self_attributed`] — both the first one and every rename — so `created.by` is its
    /// own id, and an ordinary memory cannot be: `hoard_memorize` mints a fresh id and attributes
    /// it to the *author record's* id, which is a different record by construction.
    ///
    /// This replaced a test for the `hoard-author` keyword, which any caller can write.
    /// `about-hoard-author-as-a-subject-keyword-silently-disables-embedding` in `TRACKING.md`: a
    /// memory that merely **discussed** identity records was filtered out of embedding and left
    /// semantic search with no vector, while the response blamed a missing model. The keyword still
    /// marks these records for *search* — recalling it is how a caller finds the identities to
    /// choose between — but nothing branches on it, so using it as a subject costs nothing but a
    /// stray hit.
    #[must_use]
    pub fn is_identity_record(&self) -> bool {
        self.created.by == self.id
    }
}
