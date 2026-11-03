// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Who a memory is attributed to, and whose reasoning produced it.

use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{MemoryId, ValidationError};

/// Who did something to a memory, and when.
///
/// The two travel together because they are only meaningful together: a revision that moves the
/// timestamp without changing the author, or the reverse, is a false record. Pairing them makes
/// that combination unwritable rather than merely discouraged.
///
/// Timestamps are UTC instants, serialised as RFC 3339. `documents/design/memory-data-model.md`
/// leaves the encoding to whichever an LLM reads most easily, and a readable string in the exported
/// file costs nothing at the diff that a Unix integer would save.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Attribution {
    /// The identity attached to the act, held as a reference to a `hoard-author` memory rather
    /// than as the name itself.
    ///
    /// **Attribution is the one part of a settled record that legitimately changes afterwards** —
    /// the name someone publishes under can change, where every other field is a claim they made at
    /// a moment. Holding the name here would put that mutable fact inside immutable records, so
    /// re-attribution would mean rewriting every record naming it. As a reference, it is one
    /// supersession of one small record, and no memory pointing at it moves.
    ///
    /// **A field, not an edge in `links`, and the difference is load-bearing.** Traversal expands
    /// uniformly across labels, so an author record reached as an ordinary link would acquire an
    /// in-degree equal to every memory in the set — a hub one hop from everything, dragged into
    /// every recall. See `documents/design/memory-data-model.md`.
    pub by: MemoryId,
    /// When it happened.
    pub at: Timestamp,
}

/// A project-relative user identifier.
///
/// Author records whose *identity* is attached to a memory — what Git and the export review care
/// about. Whose reasoning produced the content is a separate axis; see [`Origin`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Author(String);

impl Author {
    /// Construct an author identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::Empty`] if the value is empty or only whitespace, and
    /// [`ValidationError::IllegalCharacter`] if it contains a control character — a memory is one
    /// line of the exported file (`documents/design/multi-set-sync.md`), so an embedded newline
    /// would break the format's line granularity and the merge driver built on it.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        const FIELD: &str = "an author";

        let value = value.into();
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Err(ValidationError::Empty { field: FIELD });
        }

        if let Some(character) = trimmed.chars().find(|c| c.is_control()) {
            return Err(ValidationError::IllegalCharacter {
                field: FIELD,
                character,
            });
        }

        Ok(Self(trimmed.to_owned()))
    }

    /// The author identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Author {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Author {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Author> for String {
    fn from(author: Author) -> Self {
        author.0
    }
}

/// Whose reasoning produced a memory's content.
///
/// Distinct from [`Author`] on purpose (see `documents/design/memory-data-model.md`): local
/// prompting heavily influences what an LLM writes, so the committing identity can credit a human
/// for the model's idea and vice versa. Recording the two separately keeps "you determined X" from
/// being said to someone who did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The human's reasoning.
    Human,
    /// The model's reasoning.
    Llm,
    /// Arrived at together, with neither side's contribution separable.
    Joint,
}

impl Origin {
    /// Every origin, in a stable order.
    ///
    /// The single source for "what values does this accept", so the published tool schema can be
    /// generated from it rather than restating it. The fixed length is deliberate, in the same way
    /// [`crate::memory::LinkLabel::ALL`]'s is: adding a variant should fail to compile until every
    /// place that enumerates them has been reconsidered.
    pub const ALL: [Self; 3] = [Self::Human, Self::Llm, Self::Joint];
}
