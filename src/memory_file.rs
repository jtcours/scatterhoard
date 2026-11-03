// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Reading and writing a project's exported memory file.
//!
//! The format is a header line carrying the format version and the scope, then **one JSON
//! object per line**, one line per memory, ordered by [`file_position`]. See
//! `documents/design/file-format.md`.
//!
//! Every property of the format serves merging rather than parsing convenience
//! (`documents/design/multi-set-sync.md`): one memory per line is what keeps a change to one memory
//! from touching any other memory's line in a merge, and a stable order is what stops an export
//! looking like a rewrite. Vector embeddings are deliberately absent — they are local derived
//! state, and several hundred floats per line would defeat the small-diff goal entirely.
//!
//! # This module serialises; it does not decide what may be published
//!
//! Two different operations share this format:
//!
//! - **Export** publishes a project set into a repository. It crosses a sharing boundary, is
//!   subject to the export review gate, and is effectively a one-way door.
//! - **Backup** writes a set to storage the user already controls, so that it can be restored. No
//!   boundary is crossed, and *any* scope may be backed up — including [`Scope::Local`], which is
//!   the only reason a local set is ever written at all.
//!
//! So [`write`] takes any scope, and the rule that local memories are never published lives where
//! that question is actually asked: `Scope::exports_to`, which still answers `None` for `Local`.
//! Weakening this function to accommodate backup would have destroyed that guarantee; keeping the
//! two operations distinct costs nothing.
//!
//! Because a written file outlives the process that wrote it, the scope is recorded in the header —
//! so a local backup is recognisably local once the type system is no longer there to help, and
//! restoring one into a project's store is a refusal rather than a silent leak.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};

use serde::{Deserialize, Serialize};

use thiserror::Error;

use crate::memory::{Attribution, Link, LinkLabel, LinkTarget, Memory, MemoryId, Scope, SetId};

/// The format version this build writes, in semver.
///
/// Still `0.x`: the format is not yet stable, and saying so is more useful than a `1.0.0` that
/// would have to be broken. The major version is what compatibility is judged on — see
/// [`read`].
///
/// **`0.3.0` since a tombstone carries `ancestry` and `forgot`** — the supersession edges the
/// forgotten memory authored (`forgetting-a-chains-middle-makes-a-stale-conclusion-current-again`)
/// and the checksum of the record it replaced (`tombstone-carries-the-checksum-it-forgot`). Unlike
/// the bump below these are compatible in both directions: the fields are *added*, nothing existing
/// is respelled, and unknown fields are ignored on read — so an old build reads a new file and
/// simply loses them, which is the behaviour it had anyway. The minor is bumped to say something is
/// there, not to gate anything.
///
/// **Both landed under one bump, on the same day, and that is deliberate.** They were built hours
/// apart and `0.3.0` had never been written to a file when the second one landed — the exported set
/// in this repository was still `0.2.0`, because the running server predated the change. A second
/// bump would have minted a version that describes no file that ever existed. Where that is not
/// true, bump again: the cost of a spare minor version is nothing and the cost of two shapes
/// sharing one number is a reader that cannot tell which it has.
///
/// **`0.2.0` was `crockford-base32-ids`**, which changed how every `id` and link `target` is
/// spelled. The minor rather than the major, deliberately, and the consequence is worth stating
/// plainly rather than leaving to be discovered: a *new* build reads an old file, because
/// [`MemoryId::parse`] still accepts the hex spelling — but an *old* build meeting a new file fails
/// at the first id with a malformed-id error rather than at the version check, since minor versions
/// are not gated. That asymmetry is accepted while the format is `0.x` and one machine holds every
/// set; it is exactly the coordination cost that makes doing this early cheaper than doing it late.
/// **`0.4.0` is the attribution cut** — `created.by`, `modified.by` and `forgotten.by` hold the id
/// of a `hoard-author` memory rather than a name. That is the first change this project has made
/// that an older build cannot read *and* a newer build cannot read backwards: a name where an id
/// belongs does not parse, and no reader can repair it, because whether two names are one person is
/// not answerable from the file.
///
/// **It should have landed with the change and did not.** The record shape moved while this
/// constant stayed at `0.3.0`, so for one commit two shapes shared one number — the exact thing the
/// paragraph above says to bump for. The two exports written in that window are re-exported at
/// `0.4.0` rather than left claiming a version that describes two shapes.
pub const FORMAT_VERSION: &str = "0.4.0";

/// The oldest format version this build still understands.
///
/// **The gate needs two numbers because it answers two questions with opposite remedies.** A file
/// from a *newer* version may carry a shape this build cannot parse, and the fix is to update this
/// build. A file from an *older* one is readable only while this build keeps the code that reads
/// it, and the fix is to re-export from a build that still writes that shape. One constant cannot
/// say both, and until 2026-08-31 the check compared major versions alone — which, while every
/// version this project has ever written begins `0.`, refused nothing at all
/// (`the-format-version-gates-nothing-while-the-format-is-0x` in `TRACKING.md`).
///
/// **Below `1.0` the minor is the compatibility unit**, because semver promises nothing there and
/// this project has used minor bumps for both kinds of change: `0.2.0` respelled every id, and
/// `0.3.0` only added fields. A rule that read one of those as compatible would have to read the
/// other the same way, so the version number cannot carry the distinction on its own — which is
/// exactly why the floor is declared rather than derived.
///
/// **Raising this constant is how a build declares that a shape it used to read is no longer one it
/// understands**, and it is the lever a format cut pulls: without it, an incompatible change is
/// announced by a malformed record partway through a file rather than by a refusal naming two
/// versions. It sat at `0.1.0` while that was a claim about code — hex ids stayed readable because
/// [`MemoryId::parse`] still accepts them.
///
/// **`0.4.0` is where it moves, and the floor equals the version this build writes.** Everything
/// earlier holds a *name* in `created.by`, and this build has nowhere to put one: `by` is a
/// [`MemoryId`](crate::memory::MemoryId). That is not a spelling this reader could learn, the way it
/// learned hex — it would have to decide which of the names in the file are one person, which the
/// file does not say and no reader can infer. So the whole of the past is refused rather than half
/// read, and the sets that mattered were converted by hand while nothing else held them
/// (`documents/design/memory-data-model.md`).
///
/// **The floor and the written version being equal is a real state, not a mistake to tidy.** It
/// says this build reads exactly what it writes, which is what a cut with no migration means. They
/// separate again at the next change an older reader cannot parse but this one still can. **An
/// optional field an older reader skips is not such a change** and takes no bump: below `1.0` a
/// bump makes every older build refuse the file as newer, which for a skippable field refuses a
/// file that reader handles correctly. A `summary` in [`Header`] was the first case, added
/// 2026-09-13 (`documents/design/memory-data-model.md`) — and **removing one is not such a change
/// either**, which it proved when it went on 2026-09-21: an older reader defaults the absent field,
/// and this one ignores it in a file an older build wrote.
pub const OLDEST_READABLE: &str = "0.4.0";

/// A memory set as it exists in a file: the scope it belongs to, and its memories.
///
/// Carries a [`Scope`] rather than a bare set name because a local set is written too, for backup
/// and restore — see the module documentation on why that does not weaken the rule against
/// publishing local memories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySet {
    /// The scope these memories belong to. Declared once here rather than on every memory.
    pub scope: Scope,
    /// The memories. [`read`] returns these sorted by id; [`write`] sorts before writing.
    pub memories: Vec<Memory>,
    /// What the set has forgotten. [`read`] returns these sorted by id; [`write`] merges them back
    /// among the memories, since a file is ordered by id and not by record shape.
    ///
    /// **Added 2026-08-26, closing `memory-file-read-silently-discards-tombstones`.** This type
    /// dates from the first commit that defined the format at all, when a record could only be a
    /// memory. When forgetting became a line rather than an absence, [`validate`] was updated to
    /// count both shapes and [`read`] was given the minimum edit that kept a `Vec<Memory>`
    /// compiling — an `if let` that dropped every tombstone on the floor. [`write`], its other
    /// half, never had anywhere to get one from. So the type claimed to be a file's contents while
    /// holding a subset, in both directions, with no count, flag or error to say so.
    ///
    /// **A tombstone is a graph node, which is why the omission mattered.** [`Store::forget`] keeps
    /// the memory's inbound edges and leaves only the tombstone under that id, and
    /// `Store::holds_record` counts one — because `documents/design/cross-set-references.md`
    /// requires that a link into a supersession chain whose middle was forgotten reads as a *good*
    /// reference. Drop the tombstone and every one of those edges reads as dangling, and forgetting
    /// stops propagating: an agreed removal comes back as an anomaly on the next reconciliation.
    ///
    /// [`Store::forget`]: crate::store::Store::forget
    pub tombstones: Vec<Tombstone>,

    /// The set's own identity, absent for a file written before set identity existed.
    ///
    /// Here for the reason `memory-file-read-silently-discards-tombstones` established: this type
    /// claims to be a file's contents, and a field it silently drops is one a read-then-write cycle
    /// silently erases.
    pub set_id: Option<SetId>,
}

/// A memory file's first line: what wrote it, which set it is, and which set it *is*.
///
/// **Nothing in it changes with the set's contents**, and that is load-bearing for merging. Until
/// 2026-09-21 it also carried a description derived from the memories, so every export rewrote it
/// and two branches that both exported collided on it — on a hosted merge too, where no merge driver
/// runs. A file an older build wrote may still carry that `summary`; it is ignored on read, because
/// what a set amounts to is derived from the memories that arrive and never taken from the file
/// (`how-a-merged-file-gets-a-current-summary` in `TRACKING.md`).
///
/// **`set_id` is optional, and every file this build writes carries one.** A set holding records has
/// an identity by construction — its first write mints one, an import adopts one, and an export
/// gives one to a set that has none — so what the option is for is the file this build did not write: hand-edited, or from elsewhere.
///
/// **It is no longer for the pre-identity file.** Identity landed under `0.3.0` on 2026-08-31 and
/// [`OLDEST_READABLE`] moved to `0.4.0` the next day, so a file older than identities is refused at
/// the version check and never reaches the question. Minting one on read stays rejected whatever the
/// file: every reader of one such file would mint a different identity for the same set,
/// manufacturing the collision the identity exists to prevent.
#[derive(Debug, Serialize, Deserialize)]
pub struct Header {
    /// The format version the writing build declared.
    pub format_version: String,
    /// The scope the file declares.
    pub scope: Scope,
    /// The set's own identity, if the writing store had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_id: Option<SetId>,
    /// Whether the file is a backup, written by `hoard_sets` action `backup` rather than by an
    /// export. **A backup is never reviewed**, so its memories never count as reviewed if the file is
    /// later put where a set is published (`documents/design/export-review.md`). Absent on every
    /// export, and ignored by a build that predates it, so it needs no format change.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub backup: bool,
}

/// A forgotten memory: its id, who dropped it when, and the ancestry it was carrying.
///
/// Carries **no `content`, no `why`, no `context`, no keywords, and no free-text reason** — see
/// `documents/design/file-format.md`. A principal motive for forgetting is that sensitive data
/// reached a memory, so a tombstone preserving any of the original text would defeat the operation,
/// and a field labelled "why was this forgotten" is where someone types the credential they were
/// removing.
///
/// **`ancestry` is the exception the rule always implied**, added 2026-08-30 for
/// `forgetting-a-chains-middle-makes-a-stale-conclusion-current-again`. A supersession edge lives
/// on the memory that authored it, so forgetting that memory used to take the edge with it and a
/// record two steps back stopped knowing it had ever been replaced — reporting a twice-replaced
/// conclusion as current, with no artifact to notice. These are the forgotten record's own
/// `supersedes` and `superseded_by` edges and nothing else: **ids, not text**, which is the
/// property the rest of the rule turns on. A cross-set target keeps its set name, because that is
/// half of the reference, and loses its `keywords`, which are free text written by the same author
/// as everything else being removed. [`Tombstone::new`] is the only way to build one, so the
/// stripping cannot be forgotten at a call site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    /// The memory that is gone.
    pub id: MemoryId,
    /// Who forgot it, and when.
    pub forgotten: Attribution,
    /// The supersession edges the forgotten memory authored, stripped of free text.
    ///
    /// Absent rather than empty when there are none, so a tombstone for a memory outside any chain
    /// serialises exactly as it did before this field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ancestry: Vec<Link>,
    /// The checksum of the record this tombstone replaced, where the forgetting store held one.
    ///
    /// **What makes a received tombstone answerable without a base**
    /// (`tombstone-carries-the-checksum-it-forgot`). A tombstone shares the id it replaces, so it is
    /// a state change rather than a node and has no chain to carry ancestry — but this one field
    /// says *which version was removed*. Matching what the receiver holds means the forget is
    /// **sequential** and applies; differing means the sender forgot a version the receiver never
    /// had, so the two are **concurrent** and it is a conflict. Before this, that distinction
    /// survived only where a base happened to exist for the target, which is exactly what goes
    /// missing in the reconciliation defects.
    ///
    /// `None` where there was nothing to checksum — forgetting an id the store does not hold, which
    /// is not an error, and every tombstone written before this field existed.
    ///
    /// Written as hex rather than a JSON number, matching how a checksum is spelled everywhere else
    /// in this project: 64 bits does not survive every JSON reader as an integer, and the `bases`
    /// table already stores the same value as hex text for a related reason.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "checksum_hex"
    )]
    pub forgot: Option<u64>,
}

/// A 64-bit checksum on the wire, as 16 hex characters.
mod checksum_hex {
    use serde::{Deserialize, Deserializer, Serializer};

    #[expect(
        clippy::ref_option,
        reason = "the signature serde requires for a `with` module"
    )]
    pub fn serialize<S: Serializer>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(checksum) => serializer.serialize_str(&format!("{checksum:016x}")),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        let Some(text) = Option::<String>::deserialize(deserializer)? else {
            return Ok(None);
        };
        u64::from_str_radix(&text, 16)
            .map(Some)
            .map_err(serde::de::Error::custom)
    }
}

impl Tombstone {
    /// Build a tombstone, keeping only the ancestry edges and only their structure.
    ///
    /// Every construction goes through here — forgetting, reading a file, reading the store — so
    /// the two rules that make the field safe are applied once rather than remembered three times:
    /// a label that is not a supersession edge is dropped, and a cross-set target's `keywords` are
    /// dropped. A hand-written file is the reason the read path runs it too.
    #[must_use]
    pub fn new(
        id: MemoryId,
        forgotten: Attribution,
        links: Vec<Link>,
        forgot: Option<u64>,
    ) -> Self {
        let ancestry = links
            .into_iter()
            .filter(|link| matches!(link.label, LinkLabel::Supersedes | LinkLabel::SupersededBy))
            .map(|link| Link {
                label: link.label,
                target: match link.target {
                    LinkTarget::SameSet(id) => LinkTarget::SameSet(id),
                    LinkTarget::CrossSet {
                        set, id, set_id, ..
                    } => LinkTarget::CrossSet {
                        set,
                        id,
                        keywords: Vec::new(),
                        // Kept where the keywords are dropped, and the difference is what each is
                        // for: keywords help a *human* re-find a lost reference and are the kind of
                        // text a tombstone must not carry, while an identity is apparatus that
                        // resolves the edge. A tombstone's ancestry has to keep resolving.
                        set_id,
                    },
                },
            })
            .collect();
        Self {
            id,
            forgotten,
            ancestry,
            forgot,
        }
    }
}

/// A line of a memory file: a memory, or a record that one was forgotten.
///
/// Forgetting writes a line rather than removing one, so **the absence of a record means nothing**.
/// That is what lets reconciliation keep a memory the file has never carried instead of guessing
/// whether it was deleted upstream (`documents/design/multi-set-sync.md`).
///
/// Both shapes are records with a canonical serialisation, so both have a checksum, and the
/// three-way reconciliation applies to them without knowing which it is holding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// A memory.
    Memory(Box<Memory>),
    /// A memory that was forgotten.
    Tombstone(Tombstone),
}

impl Record {
    /// When this record last changed, whichever shape it is.
    ///
    /// A memory's `modified`, a tombstone's `forgotten` — the moment the record took the form it is
    /// in. **One accessor over both shapes**, so a caller comparing two records never has to know
    /// which it is holding, and a resurrection compares as naturally as an ordinary edit: the
    /// memory coming back is older than the tombstone that replaced it.
    ///
    /// **For judgment, never for arithmetic.** `documents/design/multi-set-sync.md` settles that
    /// reconciliation decides by checksum and not by clock — *"the clock is a good input to judgment
    /// and a bad input to arithmetic"* — and this exists to produce a sentence for a reader rather
    /// than to choose a winner. A crossed pair of edits, each side later on one record and earlier
    /// on the other, is the case that shows why: taking the newer per record assembles a state
    /// neither side ever held.
    #[must_use]
    pub fn as_of(&self) -> jiff::Timestamp {
        match self {
            Self::Memory(memory) => memory.modified.at,
            Self::Tombstone(tombstone) => tombstone.forgotten.at,
        }
    }

    /// The id this record is about, whichever shape it is.
    #[must_use]
    pub fn id(&self) -> MemoryId {
        match self {
            Self::Memory(memory) => memory.id,
            Self::Tombstone(tombstone) => tombstone.id,
        }
    }

    /// The canonical serialisation — the exact text this record occupies as a line.
    ///
    /// One function for both shapes, so a checksum describes the bytes an export writes whichever
    /// kind of record it is.
    ///
    /// **Keys are sorted**, which `documents/design/file-format.md` requires and which the first
    /// implementation did not do: serialising a struct directly emits fields in *declaration* order,
    /// so reordering the fields of [`Memory`] would silently change every checksum and invalidate
    /// every recorded base on every machine. Going through a `Value` — whose map is ordered — makes
    /// the canonical form depend on the field *names* rather than on their position in a source file.
    #[must_use]
    pub fn canonical(&self) -> String {
        let value = match self {
            Self::Memory(memory) => serde_json::to_value(memory).expect(SERIALISES),
            Self::Tombstone(tombstone) => serde_json::to_value(tombstone).expect(SERIALISES),
        };
        serde_json::to_string(&value).expect(SERIALISES)
    }
}

/// What a caller believes a file to be, established from trusted information — the path it was
/// found at, or the store a restore is targeting — never from the file itself.
///
/// `documents/design/trigger-and-loading.md` establishes that **the path determines how contents
/// are trusted**. A file's header is contents. So the scope it declares is checked against this
/// expectation and can only ever agree with it; it never decides where a file's memories land.
/// Without that, a repository could ship a memory file claiming `local` scope and have its
/// memories loaded into the more-trusted local store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expectation {
    /// The file must declare exactly this scope. Used for a repeat import, where the set name is
    /// already bound to this checkout, and for a restore, where the user chose the target.
    Exactly(Scope),

    /// A project set whose name is not yet bound to this checkout — the first import of a project.
    ///
    /// Accepts the set name the file declares, which is safe because the scope *kind* is still not
    /// negotiable: this never matches [`Scope::Local`]. The name only becomes dangerous if it
    /// collides with a set already bound to a different checkout, which is a question for whatever
    /// holds those bindings, not for this function.
    AnyShared,
}

/// Why a memory file could not be read.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MemoryFileError {
    /// The file declares a set name reserved for a copy loaded for review.
    #[error(
        "the file declares the set {declared:?}, and a name beginning \"eval-\" names a copy loaded \
         for review rather than a set anyone has adopted. A review copy is never written to a file: \
         it lives in memory for one session. If this is a real set, it needs a name of its own."
    )]
    EvaluationScope {
        /// The name the file declared.
        declared: String,
    },

    /// The underlying reader or writer failed.
    #[error("reading or writing the memory file failed")]
    Io(#[from] io::Error),

    /// The file was empty, or its first line was not a well-formed header.
    #[error("the memory file does not begin with a valid header line")]
    MissingHeader,

    /// The file was written by a newer build than this one.
    ///
    /// **Refused rather than attempted**, because below `1.0` a higher version may have respelled
    /// something this build parses by shape — and the failure that produces is a malformed record
    /// partway through the file, or, for a cross-set link, serde's *"data did not match any
    /// variant"*. Naming both versions is what makes the remedy obvious.
    #[error(
        "the memory file is format version {found}, which is newer than this build writes \
         ({writes}). Something in it may be spelled a way this build cannot parse. Update \
         Scatterhoard, or export the set again from a build that writes {writes}."
    )]
    VersionTooNew {
        /// The version declared in the file.
        found: String,
        /// The version this build writes.
        writes: String,
    },

    /// The file is older than any shape this build still reads.
    ///
    /// The opposite remedy to [`VersionTooNew`](MemoryFileError::VersionTooNew), which is why it is
    /// a separate variant rather than one message covering both: nothing the reader can do makes an
    /// unread shape readable, so the answer lives with whoever still holds a build that writes it.
    #[error(
        "the memory file is format version {found}, and this build no longer reads anything older \
         than {oldest}. The set has to be re-exported by a build that still understands that \
         shape; this one would have to guess at fields that have since been respelled."
    )]
    VersionTooOld {
        /// The version declared in the file.
        found: String,
        /// The oldest version this build understands.
        oldest: String,
    },

    /// A line did not parse as a memory.
    #[error("line {line} of the memory file is not a valid memory: {source}")]
    MalformedMemory {
        /// 1-based line number, counting the header as line 1.
        line: usize,
        /// What the JSON parser objected to.
        source: serde_json::Error,
    },

    /// The file declared a scope other than the one the caller expected.
    ///
    /// Refusing is the point: this is what a repository claiming to be the local set, or claiming
    /// to be a different project, runs into.
    #[error("the memory file declares scope {declared:?}, which is not what was expected here")]
    ScopeMismatch {
        /// The scope the file claimed.
        declared: Scope,
    },

    /// The same memory id appeared on more than one line.
    ///
    /// Not resolvable by picking one: this is what a mishandled merge produces, and silently
    /// dropping a memory is the failure the reader exists to prevent.
    /// Two records in one file claim the same id.
    ///
    /// **Names the id and both lines, not just the repeat.** A line number alone is enough to find
    /// the second occurrence and useless for finding the first, and the first is what the caller
    /// has to compare against to decide which record survives. That matters most in the case this
    /// arises from: a hand-merge of two sets, where the file is long, the duplicate is not adjacent
    /// to anything related, and `a-merged-file-with-a-duplicate-id-is-refused-at-the-read` records
    /// how a memory copied between two sets produces one.
    #[error(
        "lines {first} and {line} both claim memory id {id}. A file may carry an id once: two \
         records with one identity cannot both be stored, and choosing between them is yours to do \
         -- keep the record you want, or give the other a new id. This commonly comes from merging \
         two sets that each held a copy of the same memory."
    )]
    DuplicateId {
        /// 1-based line number of the repeat.
        line: usize,
        /// 1-based line number where that id was first seen.
        first: usize,
        /// The id claimed twice.
        id: MemoryId,
    },
}

/// Where a record sits in a memory file: FNV-1a over its id's canonical text, the id breaking ties.
///
/// **Not id order, deliberately.** Ids are UUIDv7, so a new memory's id sorts after every existing
/// one, and a file in id order puts every addition at the end. Two branches that both added memories
/// then insert at the same place in the base, which plain `git` reports as a conflict every time —
/// and a hosted merge button never runs the merge driver that would resolve it. Scattering records
/// by a hash of the id puts concurrent additions at different places, where `git` merges them
/// cleanly; measured on 2026-09-21, ten additions a side into a 988-record set conflicted in about
/// one merge in five, against every one in id order
/// (`concurrent-additions-collide-at-the-end-of-the-memory-file` in `TRACKING.md`).
///
/// **The id, never the content**, so a record never moves once written: an edit or a tombstone
/// replaces its line where it stands, and the same content always produces the same bytes. A given
/// id always lands in one place, so a duplicate still sits beside its twin.
///
/// **Part of the format, and pinned by value in the tests**: changing it would reorder every file
/// in one diff. It reuses [`crate::checksum::of_bytes`] for its pinned definition, not for anything
/// to do with reconciliation, and needs no embedding model. FNV-1a mixes its *final* byte weakly into
/// the high bits that decide the order, so ids differing only in their last character sit near each
/// other; real ids differ throughout a sixteen-character random tail, which is what was measured.
#[must_use]
pub fn file_position(id: MemoryId) -> (u64, MemoryId) {
    (crate::checksum::of_bytes(id.to_string().as_bytes()), id)
}

/// Write a memory set.
///
/// Records are written in [`file_position`] order regardless of the order they appear in `set`, so
/// the same content always produces the same bytes.
///
/// # Errors
///
/// Returns [`MemoryFileError::Io`] if the writer fails.
pub fn write<W: Write>(set: &MemorySet, writer: &mut W) -> Result<(), MemoryFileError> {
    write_header(&set.scope, set.set_id, writer)?;

    // Sorts because this entry point has been handed the whole set already, so ordering it costs
    // nothing extra. The streaming path cannot do this — see [`write_memory`].
    //
    // **Both shapes sort together.** A file is ordered by a position derived from the id, which is
    // what keeps a record in one place and what puts a duplicate id next to its twin. Writing the memories and then the
    // tombstones would order by *shape* instead — nothing a reader cares about, and every diff that
    // forgets a memory would move a line to the end of the file.
    let mut records: Vec<Record> = set
        .memories
        .iter()
        .map(|memory| Record::Memory(Box::new(memory.clone())))
        .chain(
            set.tombstones
                .iter()
                .map(|tombstone| Record::Tombstone(tombstone.clone())),
        )
        .collect();
    records.sort_by_key(|record| file_position(record.id()));

    for record in &records {
        write_record(record, writer)?;
    }

    Ok(())
}

/// Write the header line for a set of this scope.
///
/// Exists so a caller that never holds the whole set can still produce a valid file: `write_header`
/// once, then [`write_memory`] per memory as they arrive. `documents/design/file-format.md` requires
/// that a large set can be written without being materialised, for the same reason reading it takes
/// two streaming passes.
///
/// # Errors
///
/// Returns [`MemoryFileError::Io`] if the writer fails.
pub fn write_header<W: Write>(
    scope: &Scope,
    set_id: Option<SetId>,
    writer: &mut W,
) -> Result<(), MemoryFileError> {
    write_header_marked(scope, set_id, false, writer)
}

/// [`write_header`], marking the file as a backup where `backup` is set.
///
/// # Errors
///
/// As for [`write_header`].
pub fn write_header_marked<W: Write>(
    scope: &Scope,
    set_id: Option<SetId>,
    backup: bool,
    writer: &mut W,
) -> Result<(), MemoryFileError> {
    let header = Header {
        format_version: FORMAT_VERSION.to_owned(),
        scope: scope.clone(),
        set_id,
        backup,
    };
    writeln!(
        writer,
        "{}",
        serde_json::to_string(&header).expect(SERIALISES)
    )?;
    Ok(())
}

/// Write one memory as one line.
///
/// **The caller owns the ordering.** A streaming writer cannot sort what it has not yet seen, so
/// [`file_position`] order — which the format requires, and which is what keeps an export from
/// looking like a rewrite — has to come from the source. A store supplies it from a position each
/// row stores, so the database orders the records and nothing holds the set.
///
/// This is the single place a memory becomes a line, and it is the same serialisation
/// [`crate::checksum::of_memory`] covers, so a checksum describes exactly the bytes written here.
///
/// # Errors
///
/// Returns [`MemoryFileError::Io`] if the writer fails.
pub fn write_memory<W: Write>(memory: &Memory, writer: &mut W) -> Result<(), MemoryFileError> {
    write_record(&Record::Memory(Box::new(memory.clone())), writer)
}

/// Write one record — a memory or a tombstone — as one line.
///
/// The single place a record becomes a line, and the same serialisation
/// [`crate::checksum::of_record`] covers, so a checksum describes exactly the bytes written here.
///
/// # Errors
///
/// Returns [`MemoryFileError::Io`] if the writer fails.
pub fn write_record<W: Write>(record: &Record, writer: &mut W) -> Result<(), MemoryFileError> {
    writeln!(writer, "{}", record.canonical())?;
    Ok(())
}

/// Read a file's header alone, checking nothing it declares.
///
/// **For telling files apart, never for trusting one.** The `directory` form reads every header in
/// a `scatterhoard_mem` to find which file holds a set, and it has to read files that the set being
/// looked for would refuse -- another set's, or one from a newer build -- to know they are not it.
/// Whatever it picks is then read in full through [`Records::open`], which is where the version and
/// the scope are checked against what the caller expected.
///
/// # Errors
///
/// [`MemoryFileError::MissingHeader`] or [`MemoryFileError::Io`].
pub fn read_header<R: Read>(reader: R) -> Result<Header, MemoryFileError> {
    first_header(BufReader::new(reader).lines())
}

/// The first non-blank line, parsed as a header.
fn first_header(
    lines: impl Iterator<Item = io::Result<String>>,
) -> Result<Header, MemoryFileError> {
    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        return serde_json::from_str(&line).map_err(|_| MemoryFileError::MissingHeader);
    }
    Err(MemoryFileError::MissingHeader)
}

/// Read a memory set, checking it is what the caller expected.
///
/// `expected` must come from trusted information — the path the file was found at, or the store a
/// restore targets. See [`Expectation`] for why the file's own header cannot serve.
///
/// Blank lines are skipped, and the memories come back sorted by id however they were ordered in
/// the file — a merge can reorder lines, and depending on the file's order would make reading
/// fragile in exactly the situation the format exists to survive.
///
/// Unknown fields are ignored rather than rejected. Within a major version, added fields are a
/// compatible change, so a file written by a newer minor version stays readable; a file from an
/// unknown *major* version is refused outright, since it may have moved something this build
/// depends on.
///
/// # Errors
///
/// See [`MemoryFileError`]. Every variant except `Io` names the line at fault where it can.
pub fn read<R: Read>(expected: &Expectation, reader: R) -> Result<MemorySet, MemoryFileError> {
    let mut memories = Vec::new();
    let mut tombstones = Vec::new();
    let header = stream(expected, reader, |record| {
        match record {
            Record::Memory(memory) => memories.push((**memory).clone()),
            Record::Tombstone(tombstone) => tombstones.push(tombstone.clone()),
        }
        Ok(())
    })?;

    memories.sort_by_key(|memory| memory.id);
    tombstones.sort_by_key(|tombstone| tombstone.id);

    Ok(MemorySet {
        scope: header.scope,
        memories,
        tombstones,
        set_id: header.set_id,
    })
}

/// What a validating pass learned, holding no memories.
///
/// The id set is what makes this worth returning rather than discarding: a reconciling caller needs
/// to know which ids the file carries in order to find the records the *store* holds and the file
/// does not. Sixteen bytes each, so it stays flat in file size where the memories would not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validated {
    /// The scope the file declared, having agreed with what the caller expected.
    pub scope: Scope,
    /// The set's own identity, if the file declares one.
    pub set_id: Option<SetId>,
    /// Every id in the file, in the order encountered.
    pub ids: Vec<MemoryId>,
}

/// Pass one: check the file is well-formed, and collect its ids.
///
/// `documents/design/file-format.md` requires reading in two streaming passes, and requires
/// validating before applying. Each memory is parsed — so a malformed line is found — and then
/// **dropped**, so peak memory is one record plus the id set rather than the whole set.
///
/// Validating before applying is not merely tidier. A single pass that wrote into a transaction and
/// rolled back on a bad line would journal thousands of rows only to discard them, and would open a
/// window where the store had been emptied and not yet refilled. Duplicate detection is whole-file
/// regardless, so the pass that finds one has to finish before any of the file can be trusted.
///
/// # Errors
///
/// See [`MemoryFileError`].
pub fn validate<R: Read>(expected: &Expectation, reader: R) -> Result<Validated, MemoryFileError> {
    let mut ids = Vec::new();
    let header = stream(expected, reader, |record| {
        ids.push(record.id());
        Ok(())
    })?;
    Ok(Validated {
        scope: header.scope,
        set_id: header.set_id,
        ids,
    })
}

/// Parse a memory file, handing each memory to `visit` and dropping it.
///
/// The single parser for this format. [`read`] and [`validate`] are both expressed in terms of it,
/// rather than beside it — two parsers for one format would be an unforced instance of the
/// duplication this project keeps filing defects against, and the one that matters most, since a
/// reader that disagrees with itself corrupts on the way in. It is itself a walk over [`Records`],
/// which is the parser; this adds the whole-file duplicate check a pull reader cannot make without
/// holding every id it has seen.
///
/// Returns the header, whose scope has already been checked against `expected`. The header rather
/// than the scope alone because a caller adopting a set's identity needs both, and returning one and
/// re-reading the file for the other is how two readers over one format start.
///
/// # Errors
///
/// See [`MemoryFileError`], plus whatever `visit` returns.
pub fn stream<R: Read, F>(
    expected: &Expectation,
    reader: R,
    mut visit: F,
) -> Result<Header, MemoryFileError>
where
    F: FnMut(&Record) -> Result<(), MemoryFileError>,
{
    let (header, records) = Records::open(expected, reader)?;

    // Id to the line it was first seen on, rather than a bare set: the refusal has to name the
    // first occurrence, which is the one the caller needs in order to compare and choose.
    let mut seen: HashMap<MemoryId, usize> = HashMap::new();

    for item in records {
        let (number, record) = item?;
        if let Some(first) = seen.insert(record.id(), number) {
            return Err(MemoryFileError::DuplicateId {
                line: number,
                first,
                id: record.id(),
            });
        }
        visit(&record)?;
    }

    Ok(header)
}

/// A memory file read one record at a time, when the caller asks for the next.
///
/// **The parser, in the form a caller walking several files in step needs.** [`stream`] pushes
/// records at a callback, which suits one file; a merge reading three sides together has to pull
/// from each as its position requires, and cannot be handed records by three callbacks at once.
///
/// It does **not** refuse a duplicate id, because doing so means holding every id seen. A caller
/// that needs the whole-file check runs [`stream`] or [`validate`] first — reading validates before
/// applying anyway — and a caller that has already proved the file is in strictly increasing
/// [`file_position`] order has proved there are no duplicates, since a duplicate sits beside its twin.
pub struct Records<R: Read> {
    lines: std::iter::Enumerate<io::Lines<BufReader<R>>>,
}

impl<R: Read> Records<R> {
    /// Read and check the header, and return it with a reader positioned at the first record.
    ///
    /// # Errors
    ///
    /// [`MemoryFileError::MissingHeader`], an unsupported version, a scope that disagrees with
    /// `expected`, or [`MemoryFileError::Io`].
    pub fn open(expected: &Expectation, reader: R) -> Result<(Header, Self), MemoryFileError> {
        let mut lines = BufReader::new(reader).lines().enumerate();
        let header = first_header(lines.by_ref().map(|(_, line)| line))?;

        check_version(&header.format_version)?;
        check_scope(expected, &header.scope)?;

        Ok((header, Self { lines }))
    }
}

impl<R: Read> Iterator for Records<R> {
    /// The record, with the one-based line it was read from.
    type Item = Result<(usize, Record), MemoryFileError>;

    fn next(&mut self) -> Option<Self::Item> {
        for (index, line) in self.lines.by_ref() {
            let line = match line {
                Ok(line) => line,
                Err(error) => return Some(Err(error.into())),
            };
            if line.trim().is_empty() {
                continue;
            }
            let number = index + 1;
            return Some(parse_record(&line, number).map(|record| (number, record)));
        }
        None
    }
}

/// Parse one line into a record.
///
/// A `forgotten` key is what distinguishes a tombstone, checked explicitly rather than by an untagged
/// enum: untagged reports "data did not match any variant", which tells a reader nothing about which
/// shape was wrong or why, on a format whose whole job is to survive a messy merge.
fn parse_record(line: &str, number: usize) -> Result<Record, MemoryFileError> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|source| MemoryFileError::MalformedMemory {
            line: number,
            source,
        })?;

    if value.get("forgotten").is_some() {
        let tombstone: Tombstone =
            serde_json::from_value(value).map_err(|source| MemoryFileError::MalformedMemory {
                line: number,
                source,
            })?;
        // Rebuilt through the constructor rather than taken as parsed: this is the boundary a
        // hand-written or hand-merged file arrives at, and `ancestry` is the one field on a
        // tombstone that could carry someone's keywords back into the set.
        return Ok(Record::Tombstone(Tombstone::new(
            tombstone.id,
            tombstone.forgotten,
            tombstone.ancestry,
            tombstone.forgot,
        )));
    }

    let memory: Memory =
        serde_json::from_value(value).map_err(|source| MemoryFileError::MalformedMemory {
            line: number,
            source,
        })?;
    Ok(Record::Memory(Box::new(memory)))
}

/// The declared scope may only agree with what the caller already believed.
///
/// **And may never name an evaluation load**, whatever the caller expected: a file is how a set
/// travels, and a set loaded for review is one this machine holds for a session and nobody has
/// adopted. This is one of the three boundaries that keep the `eval-` prefix meaning what it says;
/// the others are `create` and a link written into a store.
fn check_scope(expected: &Expectation, declared: &Scope) -> Result<(), MemoryFileError> {
    if let Scope::Shared(set) = declared
        && set.is_evaluation()
    {
        return Err(MemoryFileError::EvaluationScope {
            declared: set.as_str().to_owned(),
        });
    }

    let agrees = match expected {
        Expectation::Exactly(scope) => scope == declared,
        // Deliberately not a wildcard: `Local` must be refused here, and adding a scope variant
        // should force this decision to be made again rather than defaulting to "accepted".
        Expectation::AnyShared => match declared {
            Scope::Shared(_) => true,
            Scope::Local => false,
        },
    };

    if agrees {
        Ok(())
    } else {
        Err(MemoryFileError::ScopeMismatch {
            declared: declared.clone(),
        })
    }
}

/// The major version is the compatibility boundary; minor and patch are ignored on read.
/// Refuse a file this build cannot be trusted to read, naming which direction it is out by.
///
/// **A version that will not parse is treated as newer, not as an error of its own.** The reader
/// cannot tell a corrupted header from one written by something that versions differently, and of
/// the two readings only *newer* is safe: it refuses, where a third "unknown" branch would need a
/// message saying nothing actionable.
fn check_version(found: &str) -> Result<(), MemoryFileError> {
    let writes =
        compatibility_of(FORMAT_VERSION).expect("FORMAT_VERSION is a valid semver constant");
    let oldest =
        compatibility_of(OLDEST_READABLE).expect("OLDEST_READABLE is a valid semver constant");

    match compatibility_of(found) {
        Some(unit) if unit > writes => Err(MemoryFileError::VersionTooNew {
            found: found.to_owned(),
            writes: FORMAT_VERSION.to_owned(),
        }),
        Some(unit) if unit < oldest => Err(MemoryFileError::VersionTooOld {
            found: found.to_owned(),
            oldest: OLDEST_READABLE.to_owned(),
        }),
        Some(_) => Ok(()),
        None => Err(MemoryFileError::VersionTooNew {
            found: found.to_owned(),
            writes: FORMAT_VERSION.to_owned(),
        }),
    }
}

/// The comparable unit of a version: what has to match for two builds to agree on the record shape.
///
/// **Below `1.0` that is `(0, minor)`; at or above it, `(major, 0)`.** The split is the semver
/// convention for `0.x` and it is what the format's own history requires — see [`OLDEST_READABLE`].
/// Once the format reaches `1.0`, a minor bump is a promise of compatibility again and stops
/// counting here, which is what makes this rule survive the transition rather than needing removal
/// at it.
///
/// The patch component is deliberately ignored at both ends: a patch that changed the record shape
/// would be a mis-numbered minor, and honouring it here would let one be published quietly.
fn compatibility_of(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next().unwrap_or("0").parse().ok()?;
    Some(if major == 0 { (0, minor) } else { (major, 0) })
}

/// Serialising a `Memory` cannot fail: every field is a plain owned value with a derived
/// implementation, and the only documented failure modes for `serde_json` are non-string map keys
/// and a `Serialize` impl that returns an error itself. Neither exists here.
const SERIALISES: &str = "a Memory always serialises";
