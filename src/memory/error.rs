// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The single error type the memory model's constructors return.

use thiserror::Error;

/// Why a value could not be turned into one of the memory model's types.
///
/// One error type covers the whole module rather than one per newtype: every variant is raised at
/// the same boundary (untrusted text becoming a typed value) and reported the same way, so
/// splitting them would give callers more to match on without giving them more to do.
///
/// `field` names the thing being constructed, so a message stays useful when the error surfaces
/// far from the constructor — for example while importing a memory file.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum ValidationError {
    /// The value was empty, or contained nothing but whitespace.
    #[error("{field} must not be empty")]
    Empty {
        /// The value being constructed.
        field: &'static str,
    },

    /// The value exceeded the type's length limit.
    #[error("{field} is {actual} characters, over the limit of {max}")]
    TooLong {
        /// The value being constructed.
        field: &'static str,
        /// The type's limit.
        max: usize,
        /// The rejected value's length.
        actual: usize,
    },

    /// The value contained a character the type does not permit.
    #[error("{field} must not contain {character:?}")]
    IllegalCharacter {
        /// The value being constructed.
        field: &'static str,
        /// The first offending character found.
        character: char,
    },

    /// The value is a word the scope grammar has already given a meaning.
    #[error(
        "{field} may not be {value:?}: that word already names the local scope, and a set called \
         {value:?} could not be told apart from it -- `local` and `shared:local` read the same to \
         anyone meeting them, and a link naming it would mean the shared set rather than the local \
         one. Pick another name."
    )]
    ReservedName {
        /// The value being constructed.
        field: &'static str,
        /// The reserved word that was offered.
        value: String,
    },

    /// The value began or ended with a character that is only allowed internally.
    #[error("{field} must not start or end with {character:?}")]
    IllegalEdgeCharacter {
        /// The value being constructed.
        field: &'static str,
        /// The offending character.
        character: char,
    },

    /// A keyword was more than one word.
    #[error("a keyword must be a single word, not {value:?}")]
    NotASingleWord {
        /// The rejected value.
        value: String,
    },

    /// A confidence fell outside 0.0..=1.0.
    #[error("confidence must be between 0.0 and 1.0, not {value}")]
    ConfidenceOutOfRange {
        /// The rejected value.
        value: f64,
    },

    /// A confidence was NaN or an infinity.
    ///
    /// Separate from [`ValidationError::ConfidenceOutOfRange`] because it is a different mistake:
    /// out-of-range is a miscalibrated number, non-finite is usually a computation that went wrong
    /// upstream.
    #[error("confidence must be a finite number")]
    ConfidenceNotFinite,

    /// A string was not a well-formed memory identifier.
    #[error("`{value}` is not a well-formed memory id")]
    MalformedMemoryId {
        /// The rejected value.
        value: String,
    },

    /// A string was not a well-formed set identifier.
    ///
    /// Separate from [`MalformedMemoryId`](ValidationError::MalformedMemoryId) because the two
    /// accept different spellings — a memory id still reads the legacy hex form and a set id never
    /// had one — so one message would have to describe a rule that holds for only half its callers.
    #[error(
        "`{value}` is not a well-formed set id: 26 Crockford base32 characters, as `hoard_sets` \
         reports for a set it holds"
    )]
    MalformedSetId {
        /// The rejected value.
        value: String,
    },
}
