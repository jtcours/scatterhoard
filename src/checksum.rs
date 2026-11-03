// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The reconciliation checksum: whether a record is the same record.
//!
//! `documents/design/multi-set-sync.md` settles what this is for. A record's checksum is compared
//! against the *base* — the value it had when the store and a file last agreed — to decide whether
//! that side changed. It answers **is this the same record**, not *which version is newer*, which is
//! why it needs no clock.
//!
//! # Deliberately not cryptographic
//!
//! This is a change detector, not a security boundary, and the design records why in full. In short:
//! a collision crafted by someone with write access to a repository makes reconciliation conclude
//! that origin did *not* change the record, so their content is not imported — the collision
//! suppresses their own change. A cryptographic hash here would state a guarantee this value does not
//! provide and invite a later reader to use it as an integrity or provenance check.
//!
//! Sixty-four bits because this is a direct collision rather than a birthday problem: one record
//! against its own base. At 32 bits that is roughly one in four billion per changed record, whose
//! failure is a memory silently staying stale; at 64 it is about 5 × 10⁻²⁰.
//!
//! # Why FNV-1a rather than a crate or an ad-hoc function
//!
//! The design requires the algorithm to be **pinned deliberately**, because a persisted checksum
//! whose definition can drift invalidates every base on every machine. `std`'s `DefaultHasher` is
//! documented as unstable across Rust releases and is therefore disqualified outright.
//!
//! FNV-1a is externally specified and tiny. The property that matters is not that the code lives
//! here but that **its behaviour is pinned by published test vectors** ([`ALGORITHM`] and the tests
//! beside this module), so a well-meaning refactor that changes the output fails a test rather than
//! silently re-baselining every store.

use crate::memory::Memory;
use crate::memory_file::Record;

/// Names the algorithm and its input convention, stored beside the bases it produced.
///
/// Recorded so a build meeting checksums it cannot reproduce **detects** that rather than comparing
/// garbage. Changing the algorithm means changing this string, which is the signal to treat every
/// record as divergent and re-establish agreement on the next export — noisy for one cycle, and not
/// lossy, because the base never reaches the memory file.
pub const ALGORITHM: &str = "fnv1a64/json-sorted-v1";

const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over bytes.
#[must_use]
pub fn of_bytes(bytes: &[u8]) -> u64 {
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// The checksum of a memory, over its canonical form.
///
/// The canonical form is the memory's own `serde` serialisation — which is **the same function that
/// writes a line into the memory file**, so the checksum covers exactly the bytes an export would
/// produce, and a field added to [`Memory`] reaches both without a second edit.
///
/// Crucially this hashes the *re-serialisation of a parsed memory*, never a line as it was read.
/// `documents/design/file-format.md` requires that: hashing raw text would let a merge tool
/// normalising whitespace, or a writer emitting keys in a new order, register as a content change on
/// every record in the file and turn the next reconciliation into one enormous false conflict.
///
/// Key order is **sorted**, and getting there takes deliberate work rather than luck. Serialising a
/// struct directly emits its fields in *declaration* order, so the first version of this function
/// made the checksum depend on where each field sat in a source file — reordering `Memory`'s fields
/// would have invalidated every recorded base on every machine, silently. The test that claimed to
/// pin the order parsed into a `serde_json::Value` first, whose map is sorted by construction, so it
/// asserted a property of `Value` and would have passed whatever the serialiser did.
///
/// Sorting is now done by [`Record::canonical`], through which this function routes so there is one
/// definition of a record's bytes rather than two.
#[must_use]
pub fn of_memory(memory: &Memory) -> u64 {
    of_record(&Record::Memory(Box::new(memory.clone())))
}

/// The checksum of a record, memory or tombstone alike.
///
/// A tombstone is a record with a canonical form like any other, so it has a checksum and the
/// three-way reconciliation in [`crate::reconcile`] applies to it without knowing which shape it
/// holds. That is what makes *forgetting* propagate through the same machinery as any other change,
/// rather than needing a path of its own.
#[must_use]
pub fn of_record(record: &Record) -> u64 {
    of_bytes(record.canonical().as_bytes())
}
