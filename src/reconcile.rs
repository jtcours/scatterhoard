// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The three-way decision an import makes about one record.
//!
//! `documents/design/multi-set-sync.md` settles the rules; this is only their arithmetic. It is a
//! pure function over three checksums so that the part most expensive to get wrong can be tested
//! without a store, a file, or a filesystem.
//!
//! # Why three values and not two timestamps
//!
//! The *base* is the checksum a record had when this store and one particular file last agreed. With
//! it, "did this side change" is answerable locally and without a clock — which matters because a
//! record may legitimately claim to have been created in 2026 and modified in 1998, and an LLM can
//! absorb that where a merge algorithm cannot.
//!
//! Without a base there is no ancestor, and no amount of comparing the two present versions can
//! recover one. That is why the base is written on export as well as on import: both establish
//! agreement, and recording only one of them would make a machine's own exported changes read as
//! changes arriving from origin.

/// What an import should do with one record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Neither side moved. Nothing to write.
    Unchanged,

    /// Only the file moved: take its version.
    TakeFile,

    /// Only the store moved: keep it, and let the next export carry it out.
    KeepLocal,

    /// Both moved and arrived at the same content.
    ///
    /// Not a conflict, and worth its own name: without this case, two people making the same
    /// correction would be reported as disagreeing, and "conflict" would stop meaning anything.
    /// Nothing is written, but the base advances, because the two sides now agree.
    Converged,

    /// Both moved, to different content, with no basis for choosing.
    ///
    /// Surfaced rather than resolved. Last-writer-wins would silently discard one side's work, and
    /// this is precisely the case `git` would have raised as a conflict if both versions had reached
    /// the same file.
    Conflict,

    /// The file has a record the store has never held.
    NewFromFile,

    /// A state no supported operation produces, reported rather than guessed at.
    ///
    /// Reached when the store and the file once agreed on a record that the store no longer holds at
    /// all. Forgetting is supposed to leave a tombstone, so the row would still be there; a missing
    /// row means a hand-edited database, or a build older than tombstones. Resurrecting it and
    /// dropping it are both wrong, and neither is recoverable by guessing.
    Anomaly,

    /// The file has a memory for an id this store has forgotten, and it stays forgotten.
    ///
    /// **A forget is one-way at import**, since 2026-09-28 (`merge-without-a-recorded-ancestor`,
    /// decided with the human partner). A memory arriving over a tombstone is its own old bytes
    /// coming back, so an older copy of the file and a revert of the commit that forgot it are the
    /// same object here and neither can be told from the other. Taking it was reported and done
    /// until then; now nothing is written and the base does not move. The deliberate undo is a new
    /// memory superseding the forgotten id -- a new event, as a git revert is a new commit -- and
    /// recall carries the old id's in-links to it.
    KeptForgotten,
}

/// Decide what to do with a record, given the file's checksum, the recorded base, and the store's.
///
/// `stored` is `None` when the store does not hold the record; `base` is `None` when this store and
/// this target have never agreed about it.
#[must_use]
pub fn resolve(file: u64, base: Option<u64>, stored: Option<u64>) -> Outcome {
    match (base, stored) {
        // Never seen here, and not in the store: straightforwardly new.
        (None, None) => Outcome::NewFromFile,

        // We agreed on it once and the store has since lost the row entirely.
        (Some(_), None) => Outcome::Anomaly,

        // Both sides hold a version and there is no ancestor to judge by. Identical content is still
        // agreement; anything else is a genuine conflict rather than a reason to pick.
        (None, Some(stored)) => {
            if stored == file {
                Outcome::Converged
            } else {
                Outcome::Conflict
            }
        }

        (Some(base), Some(stored)) => match (file == base, stored == base) {
            (true, true) => Outcome::Unchanged,
            (false, true) => Outcome::TakeFile,
            (true, false) => Outcome::KeepLocal,
            (false, false) => {
                if file == stored {
                    Outcome::Converged
                } else {
                    Outcome::Conflict
                }
            }
        },
    }
}

/// Decide what to do with an arriving **memory**, given whether the store holds its id as a
/// tombstone.
///
/// The one row of the lattice in `documents/design/multi-set-sync.md` that needs no ancestor to
/// answer, applied ahead of the three-way decision: *tombstone over memory, the tombstone*. It asks
/// what kind of record the store holds, never the clock, so this stays a pure function of what the
/// two sides hold. Everything else is [`resolve`], unchanged.
#[must_use]
pub fn resolve_memory(
    file: u64,
    base: Option<u64>,
    stored: Option<u64>,
    stored_is_tombstone: bool,
) -> Outcome {
    if stored_is_tombstone {
        return Outcome::KeptForgotten;
    }
    resolve(file, base, stored)
}

/// Decide what to do with an arriving **tombstone**, which can say what it forgot.
///
/// `documents/design/multi-set-sync.md`: a tombstone shares the id it replaces, so it is a state
/// change rather than a node and carries no chain — but `forgot`, the checksum of the record it
/// replaced, makes it answerable on its own. **Matching what the receiver holds means the forget is
/// sequential and applies; differing means the sender forgot a version the receiver never had, so
/// the two are concurrent.**
///
/// **Only consulted where there is no base**, which is deliberate rather than cautious. Where an
/// ancestor exists the three-way arithmetic already answers this correctly, and deferring to it
/// means nothing that reconciles today changes its answer. What the checksum buys is precisely the
/// case that had no answer: a file from a location this store has never agreed with, which is a
/// fresh clone, a second checkout, or a copy handed over directly — and which
/// `a-target-path-can-come-to-mean-a-different-volume` shows is also what a stale base degrades to.
///
/// Without it, a forget arriving with no base is a `Conflict` for the plainest possible reason:
/// a tombstone's checksum never equals the memory's, so *both sides changed* is true and useless.
#[must_use]
pub fn resolve_tombstone(
    file: u64,
    forgot: Option<u64>,
    base: Option<u64>,
    stored: Option<u64>,
) -> Outcome {
    let ordinary = resolve(file, base, stored);
    if base.is_some() {
        return ordinary;
    }
    match (forgot, stored) {
        // The version they removed is the version we hold: their forget is the next state after
        // ours, so it applies exactly as any other record that only the file moved.
        (Some(forgot), Some(stored)) if forgot == stored => Outcome::TakeFile,
        // Either they did not say what they forgot -- a tombstone from before the field existed, or
        // one written for an id its store did not hold -- or they removed something this store has
        // never had. Both fall back, which for the second is the `Conflict` it should be.
        _ => ordinary,
    }
}

/// What to do with a record the store holds and the file does not.
///
/// Absence in the file means nothing on its own — that is the whole reason forgetting writes a
/// tombstone rather than deleting a line. So a record the file omits is either a local addition that
/// has not been exported yet, or an anomaly if the two sides had previously agreed it existed.
#[must_use]
pub fn resolve_absent_from_file(base: Option<u64>) -> Outcome {
    match base {
        None => Outcome::KeepLocal,
        Some(_) => Outcome::Anomaly,
    }
}
