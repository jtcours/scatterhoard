// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Scatterhoard — a project-scale auxiliary memory tool for LLMs.
//!
//! The crate is organised as a set of components that can be used independently; see
//! `documents/design/components.md`.

pub mod checksum;
pub mod cli;
pub mod conventions;
pub mod embedder;
pub mod embedding;
pub mod eval;
pub mod language_fit;
pub mod mcp;
pub mod memory;
pub mod memory_file;
pub mod merge;
pub mod output_schema;
pub mod preferences;
pub mod prohibited;
pub mod reconcile;
pub mod review;
pub mod store;
pub mod summary;
pub mod traversal;
