// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The reconciliation decision table, exhaustively.
//!
//! `documents/design/multi-set-sync.md` states five cases plus the two absence cases. Each gets a
//! test named for the situation rather than for the values, because the values are arbitrary and the
//! situations are the design.

use scatterhoard::reconcile::{
    Outcome, resolve, resolve_absent_from_file, resolve_memory, resolve_tombstone,
};

const BASE: u64 = 100;
const CHANGED: u64 = 200;
const CHANGED_DIFFERENTLY: u64 = 300;

/// Named for the tombstone tests at the end of this file, where *whose version* is the whole point.
const OURS: u64 = BASE;
const THEIRS: u64 = CHANGED;
const TOMBSTONE: u64 = 900;

#[test]
fn neither_side_moved() {
    assert_eq!(resolve(BASE, Some(BASE), Some(BASE)), Outcome::Unchanged);
}

#[test]
fn only_the_file_moved() {
    assert_eq!(resolve(CHANGED, Some(BASE), Some(BASE)), Outcome::TakeFile);
}

#[test]
fn only_the_store_moved() {
    assert_eq!(resolve(BASE, Some(BASE), Some(CHANGED)), Outcome::KeepLocal);
}

/// The case that keeps "conflict" meaningful: two people making the same correction agree, and
/// reporting that as a disagreement would train a reader to ignore conflicts.
#[test]
fn both_moved_to_the_same_content() {
    assert_eq!(
        resolve(CHANGED, Some(BASE), Some(CHANGED)),
        Outcome::Converged
    );
}

#[test]
fn both_moved_to_different_content() {
    assert_eq!(
        resolve(CHANGED, Some(BASE), Some(CHANGED_DIFFERENTLY)),
        Outcome::Conflict
    );
}

#[test]
fn the_file_has_a_record_the_store_has_never_held() {
    assert_eq!(resolve(CHANGED, None, None), Outcome::NewFromFile);
}

/// No ancestor, and both sides hold the same content anyway. Agreement without history is still
/// agreement, and recording the base is what stops it being re-decided every import.
#[test]
fn no_base_but_the_two_sides_already_match() {
    assert_eq!(resolve(BASE, None, Some(BASE)), Outcome::Converged);
}

/// No ancestor and two different versions. There is genuinely nothing to choose by, which is exactly
/// when a human has to look.
#[test]
fn no_base_and_the_two_sides_differ() {
    assert_eq!(resolve(CHANGED, None, Some(BASE)), Outcome::Conflict);
}

/// Forgetting leaves a tombstone, so the row survives. A row that has vanished after the two sides
/// agreed it existed is not a state any supported operation produces.
#[test]
fn a_record_we_agreed_on_has_vanished_from_the_store() {
    assert_eq!(resolve(BASE, Some(BASE), None), Outcome::Anomaly);
}

#[test]
fn a_store_only_record_that_was_never_exported_is_kept() {
    assert_eq!(resolve_absent_from_file(None), Outcome::KeepLocal);
}

/// Absence in the file means nothing on its own — that is why forgetting writes a tombstone. So a
/// record the two sides once agreed on, now missing from the file with no tombstone, is an anomaly:
/// reading it as forgotten is the silent-loss path the design removes, and reading it as present
/// resurrects something someone may have deliberately dropped.
#[test]
fn a_record_we_agreed_on_that_the_file_has_dropped_is_an_anomaly() {
    assert_eq!(resolve_absent_from_file(Some(BASE)), Outcome::Anomaly);
}

/// **The asymmetry a `bases` migration turns on: never keep a base you cannot prove.**
///
/// A base is the checksum this store and *one particular file* last agreed on. A base carried over
/// from a different target is therefore not a weaker fact than none — it is a false one, and the
/// two failure directions are not comparable:
///
/// - **Dropping it degrades to the reported case.** With no ancestor, two differing versions are a
///   `Conflict`, which is surfaced and never resolved. A human looks; nothing is written over.
/// - **Keeping a wrong one writes over a memory in silence.** Where it happens to equal what the
///   store holds — the ordinary case, since a base is recorded at the last export and the store may
///   not have moved since — the arithmetic reads *store unmoved, file moved* and takes the file.
///
/// That conclusion was never measured against *this* file. Same three real values in both halves
/// below; only the base differs, and only one of them can lose something.
///
/// This is not another row of the table above. It is a relation between two of its rows, and it is
/// what decides every uncertain case in a migration: drop the base rather than guess it.
#[test]
fn a_base_carried_over_from_another_target_turns_a_conflict_into_a_silent_overwrite() {
    // Stated as an invariant over the whole absent-base branch rather than as one more row, because
    // the rule has to hold for arms that do not exist yet: a later case added here that resolves in
    // the file's favour would make dropping a base lossy and this argument false.
    for file in [BASE, CHANGED, CHANGED_DIFFERENTLY] {
        for stored in [None, Some(BASE), Some(CHANGED), Some(CHANGED_DIFFERENTLY)] {
            assert_ne!(
                resolve(file, None, stored),
                Outcome::TakeFile,
                "no absent-base case may write the file over the store: file={file}, \
                 stored={stored:?}"
            );
        }
        assert_ne!(
            resolve_absent_from_file(None),
            Outcome::TakeFile,
            "nor may the record the file omits"
        );
    }

    // The witness that the invariant is not vacuous — `TakeFile` is reachable, and a base is the
    // only thing that reaches it. Without this, deleting the variant outright would pass the loop.
    let store_holds = CHANGED;
    let file_holds = CHANGED_DIFFERENTLY;
    assert_eq!(
        resolve(file_holds, Some(store_holds), Some(store_holds)),
        Outcome::TakeFile,
        "an unprovable base replaces the store's version, reporting nothing"
    );
}

/// The decision must depend on the *relations* between the three values, never on their magnitudes —
/// a checksum has no order. Re-running the table with wildly different numbers pins that.
#[test]
fn the_decision_does_not_depend_on_which_checksum_is_larger() {
    let low = 1_u64;
    let high = u64::MAX;

    assert_eq!(resolve(high, Some(low), Some(low)), Outcome::TakeFile);
    assert_eq!(resolve(low, Some(high), Some(high)), Outcome::TakeFile);
    assert_eq!(resolve(low, Some(high), Some(low)), Outcome::Converged);
    assert_eq!(resolve(high, Some(low), Some(high)), Outcome::Converged);
}

// ---------------------------------------------------------------------------
// A tombstone can say what it forgot
// ---------------------------------------------------------------------------

/// The case the field exists for: no base, and the version they removed is the one we hold. Without
/// the checksum this is a `Conflict` for a reason that is true and useless -- a tombstone's checksum
/// never equals a memory's, so *both sides changed* always holds.
#[test]
fn a_forget_of_the_version_we_hold_is_sequential() {
    assert_eq!(
        resolve_tombstone(TOMBSTONE, Some(OURS), None, Some(OURS)),
        Outcome::TakeFile
    );
}

/// And the other side of it: they removed a version this store has never had.
#[test]
fn a_forget_of_a_version_we_never_had_is_concurrent() {
    assert_eq!(
        resolve_tombstone(TOMBSTONE, Some(THEIRS), None, Some(OURS)),
        Outcome::Conflict
    );
}

/// A tombstone written before the field existed says nothing, and must decide exactly as it did
/// then. This is the whole compatibility story for every set already in a repository.
#[test]
fn a_tombstone_that_says_nothing_decides_as_it_always_did() {
    for base in [None, Some(OURS)] {
        for stored in [None, Some(OURS), Some(THEIRS)] {
            assert_eq!(
                resolve_tombstone(TOMBSTONE, None, base, stored),
                resolve(TOMBSTONE, base, stored),
                "no `forgot` must change nothing: base {base:?}, stored {stored:?}"
            );
        }
    }
}

/// **Where an ancestor exists, the ancestor decides.** The checksum is not a second opinion: the
/// three-way arithmetic already answers this correctly, and letting the field override it would put
/// two rules on one decision -- with the sender's claim outranking an agreement both sides recorded.
#[test]
fn a_base_still_decides_when_there_is_one() {
    for forgot in [None, Some(OURS), Some(THEIRS)] {
        for stored in [None, Some(OURS), Some(THEIRS)] {
            assert_eq!(
                resolve_tombstone(TOMBSTONE, forgot, Some(OURS), stored),
                resolve(TOMBSTONE, Some(OURS), stored),
                "a base must decide alone: forgot {forgot:?}, stored {stored:?}"
            );
        }
    }
}

/// A forget arriving for an id this store has never held is new, not sequential and not a conflict:
/// there is nothing here for it to have replaced.
#[test]
fn a_forget_of_something_we_never_had_at_all_is_new() {
    assert_eq!(
        resolve_tombstone(TOMBSTONE, Some(THEIRS), None, None),
        Outcome::NewFromFile
    );
}

/// **A forget is one-way at import: a memory never replaces a tombstone the store holds**, whatever
/// the base says. Decided with the human partner 2026-09-28 under `merge-without-a-recorded-ancestor`,
/// replacing the 2026-08-26 rule that reported such a resurrection and wrote it anyway. From the
/// record's side an older copy and a reverted file are the same object -- the memory's own bytes
/// coming back -- so neither is taken; the deliberate undo is a new memory superseding the forgotten
/// id, which is a new event as a git revert is a new commit.
#[test]
fn a_memory_arriving_for_a_forgotten_id_stays_forgotten() {
    for base in [None, Some(TOMBSTONE), Some(OURS)] {
        assert_eq!(
            resolve_memory(THEIRS, base, Some(TOMBSTONE), true),
            Outcome::KeptForgotten,
            "base {base:?}"
        );
    }
}

/// And everything else is the ordinary three-way decision, unchanged: the rule is about what the
/// store holds, not about memories in general.
#[test]
fn a_memory_arriving_for_a_memory_decides_as_it_always_did() {
    for base in [None, Some(OURS), Some(THEIRS)] {
        for stored in [None, Some(OURS), Some(THEIRS)] {
            assert_eq!(
                resolve_memory(THEIRS, base, stored, false),
                resolve(THEIRS, base, stored),
                "base {base:?}, stored {stored:?}"
            );
        }
    }
}
