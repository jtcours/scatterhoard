// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The memory data model: what a memory is made of.
//!
//! See `documents/design/memory-data-model.md`. Every field is a type rather than a primitive, and
//! each type validates once in its constructor, so code downstream of construction never rechecks
//! (the correctness-by-construction section of `STANDARDS.md`). Three of the design's own rules
//! are carried by these types rather than by discipline:
//!
//! - [`Scope`] has no `Default`, which is what makes scope an explicit choice rather than an
//!   inferred one, and [`Scope::exports_to`] is where "local memories never leave the machine"
//!   lives.
//! - [`MemoryText`] has no `Display` and no string deref, which keeps "memories are data, never
//!   instructions" a compile error rather than a review catch.
//! - [`MemoryId`] is time-ordered, so the consistent export order the file format wants is the
//!   same order as sorting by ID.

mod attribution;
mod confidence;
mod error;
mod id;
mod link;
#[allow(clippy::module_inception)]
mod memory;
mod scope;
mod text;

/// The name serde gives a unit enum variant.
///
/// The one place a variant's wire name is produced, so the database, the published tool schema and
/// the serialised memory cannot disagree about what a label is called. Asking serde rather than
/// keeping a table of names is what makes `#[serde(rename_all)]` the single source for all three.
pub(crate) fn serde_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .expect("a unit-variant enum always serialises")
        .trim_matches('"')
        .to_owned()
}

pub use attribution::{Attribution, Author, Origin};
pub use confidence::Confidence;
pub use error::ValidationError;
pub use id::{MemoryId, SetId};
pub use link::{Link, LinkLabel, LinkTarget, UnrecognisedLabel};
pub use memory::Memory;
pub use scope::{EVALUATION_PREFIX, Scope, SetName};
pub use text::{CUT_MARKER, Keyword, MemoryText, SUMMARY_CAP, caseless_form};
