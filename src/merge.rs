// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The field-aware three-way merge behind the `git` merge driver.
//!
//! `documents/design/multi-set-sync.md` settles the shape: `git` owns the file-against-file merge,
//! because it has a real ancestor, and this refines the one case it gets coarsely wrong. Two
//! branches editing *different fields* of one memory are one line edited twice, so plain `git`
//! reports a conflict for changes that are compatible at the field level.
//!
//! **A function over three things it can read**, like [`crate::reconcile`] is a pure function over
//! three checksums, so the part most expensive to get wrong is testable without a store, a
//! repository or a filesystem: [`merge_files`] takes three texts, and [`merge_streams`] — which it
//! wraps, and which the driver in [`crate::cli`] calls with three paths — takes three things it can
//! open. The driver is the part that knows about `git`.
//!
//! # Holding one record a side, not the set
//!
//! Every current export is in [`memory_file::file_position`] order, so the three sides are walked in
//! step like a sort-merge join and each merged record is written as soon as it is decided: the merge
//! holds a record from each side, however large the set (`streaming-merge-driver`). A side out of
//! that order — every file written before 2026-09-21 is in id order — cannot be walked that way and
//! is read whole, which [`Outcome::out_of_order`] reports.
//!
//! # What makes this a merge rather than a guess
//!
//! `git` hands a driver the merge base, so every decision here is three-way. A field that one side
//! changed takes that side's value; a field both sides changed differently is a genuine divergence
//! and falls through as a conflict — which is exactly what plain `git` would have produced, so the
//! worst case of layering this on is the behaviour it replaces.
//!
//! Collections are set-merged against the base rather than unioned, and the difference is not
//! academic: a union resurrects a keyword one side deliberately retired, and retiring a keyword is
//! the only way to take one out of circulation (`keyword-standards`).
//!
//! # `modified` is the field that forces a decision
//!
//! Every edit moves `modified`, so in precisely the case this exists for — ours bumped confidence,
//! theirs added a link — the two sides *always* disagree there. Treating that as a conflict would
//! mean nothing ever merges, so the later of the two attributions wins and the merged record claims
//! an authorship neither side wrote. That is a real cost, accepted deliberately: the alternative is
//! a driver that cannot merge anything it was built to merge. `created` is the opposite case and
//! never moves, so a divergence there is corruption rather than an edit.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};

use thiserror::Error;

use crate::memory::{Memory, MemoryId, Scope};
use crate::memory_file::{self, Expectation, Record, Records, Tombstone};

/// What became of one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merged {
    /// The two sides were reconciled into one record.
    Clean(Record),
    /// The two sides genuinely diverged. The id is carried so the driver can name it.
    Conflict(MemoryId),
}

/// A merged memory file, and what could not be merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merge {
    /// The merged file, ready to be written where the driver was told to put it.
    ///
    /// **Always a valid memory file, conflicts included.** A conflicted record keeps the receiving
    /// side's version rather than being written twice with markers between them: a memory file is
    /// one JSON object per line and cannot carry conflict markers without becoming unreadable to
    /// every tool here, including the scan a pre-commit hook runs. `git` learns the merge failed
    /// from the driver's exit status, and the human resolves against a file that still parses.
    pub text: String,
    /// The records that diverged, in file order.
    pub conflicts: Vec<MemoryId>,
    /// The first side found out of file order, which sent the merge down the whole-file path.
    ///
    /// See [`Outcome::out_of_order`].
    pub out_of_order: Option<&'static str>,
}

/// What a streaming merge reports once the merged file has been written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The records that diverged, in file order.
    pub conflicts: Vec<MemoryId>,
    /// The first side, in `git`'s order of base, ours, theirs, whose records are not in
    /// [`memory_file::file_position`] order — or `None` when all three are.
    ///
    /// **Named because it is the one case that holds whole sets.** Sides in file order are walked
    /// in step, one record from each at a time; a side out of order cannot be, so all three are read
    /// whole. Every file written before 2026-09-21 is in id order, so a person with a large set
    /// learns from this why a merge was slow, and that exporting again ends it.
    pub out_of_order: Option<&'static str>,
}

/// Something that can be opened for reading, more than once.
///
/// **More than once because a streaming merge reads each side twice**: once to check that it parses
/// and is in file order, and again to merge it. Checking before writing anything is the same
/// validate-before-applying rule reading follows (`documents/design/file-format.md`), and it is what
/// lets the choice between walking in step and reading whole be made before the first line is out.
pub type Opener<'a> = dyn Fn() -> io::Result<Box<dyn Read + 'a>> + 'a;

/// Why a merge could not be attempted at all.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MergeError {
    /// One of the three inputs is not a memory file this build can read.
    #[error("the {side} side is not a readable memory file: {detail}")]
    Unreadable {
        /// Which input: `base`, `ours` or `theirs`.
        side: &'static str,
        /// What the reader said.
        detail: String,
    },

    /// The three inputs do not describe the same set.
    ///
    /// Not a merge this can attempt: `git` is merging one path, so two different sets arriving there
    /// means the repository is holding something nobody intended, and quietly merging them would
    /// make one set's memories appear in another.
    #[error(
        "the three sides declare different sets ({ours} and {theirs}), so this is not one set's history"
    )]
    DifferentSets {
        /// The scope the receiving side declared.
        ours: String,
        /// The scope the other side declared.
        theirs: String,
    },

    /// The receiving side has no header line to carry over.
    #[error("the receiving side has no header line, so there is nothing to write as one")]
    NoHeader,

    /// The merged file could not be written.
    #[error("could not write the merged file: {0}")]
    Unwritable(#[source] io::Error),
}

/// Merge three versions of a memory file.
///
/// `base` is the merge base, `ours` the receiving side, `theirs` the side being merged in — the
/// same three `git` hands a driver as `%O`, `%A` and `%B`.
///
/// **The receiving side's header is kept verbatim.** Since 2026-09-21 nothing in a header depends on
/// the set's contents, so two sides of one set write the same header and there is nothing to choose.
/// A file an older build wrote may still carry a derived `summary`, which differs between sides and
/// cannot be recomputed without the vectors; ours is kept, and no reader looks at it.
///
/// # Errors
///
/// See [`MergeError`]. A record that cannot be merged is a conflict rather than an error: the merge
/// succeeded and produced a file, and the driver reports the conflict through its exit status.
pub fn merge_files(base: &str, ours: &str, theirs: &str) -> Result<Merge, MergeError> {
    let open_base = || -> io::Result<Box<dyn Read + '_>> { Ok(Box::new(base.as_bytes())) };
    let open_ours = || -> io::Result<Box<dyn Read + '_>> { Ok(Box::new(ours.as_bytes())) };
    let open_theirs = || -> io::Result<Box<dyn Read + '_>> { Ok(Box::new(theirs.as_bytes())) };

    let mut body: Vec<u8> = Vec::new();
    let outcome = merge_streams(&open_base, &open_ours, &open_theirs, &mut body)?;

    Ok(Merge {
        text: String::from_utf8(body).expect("a record serialises as UTF-8"),
        conflicts: outcome.conflicts,
        out_of_order: outcome.out_of_order,
    })
}

/// Merge three versions of a memory file, writing the result to `out` as it is decided.
///
/// The form the driver uses, and the one [`merge_files`] is a wrapper around. **Neither a side nor
/// the result is held whole** when all three sides are in file order, which every current export
/// is: the sides are walked in step, a record from each at a time, and each merged record is written
/// as soon as it is decided (`streaming-merge-driver`). A side out of order is read whole instead,
/// and [`Outcome::out_of_order`] names it.
///
/// Each side is read twice — see [`Opener`] — so nothing is written until all three are known to
/// parse and describe one set.
///
/// # Errors
///
/// See [`MergeError`]. Something may already have been written to `out` when a write fails, so the
/// driver writes to a file of its own and renames it into place only on success.
pub fn merge_streams<W: Write>(
    base: &Opener<'_>,
    ours: &Opener<'_>,
    theirs: &Opener<'_>,
    out: &mut W,
) -> Result<Outcome, MergeError> {
    let header = header_line(ours)?;

    let (base_scope, base_in_order) = check_side("base", base)?;
    let (our_scope, ours_in_order) = check_side("ours", ours)?;
    let (their_scope, theirs_in_order) = check_side("theirs", theirs)?;

    if our_scope != their_scope || our_scope != base_scope {
        return Err(MergeError::DifferentSets {
            ours: our_scope.as_key(),
            theirs: their_scope.as_key(),
        });
    }

    let out_of_order = [
        ("base", base_in_order),
        ("ours", ours_in_order),
        ("theirs", theirs_in_order),
    ]
    .into_iter()
    .find(|(_, in_order)| !in_order)
    .map(|(side, _)| side);

    writeln!(out, "{header}").map_err(MergeError::Unwritable)?;
    let conflicts = if out_of_order.is_some() {
        merge_whole(base, ours, theirs, out)?
    } else {
        merge_in_step(base, ours, theirs, out)?
    };

    Ok(Outcome {
        conflicts,
        out_of_order,
    })
}

/// Walk three sides in file order together, holding one record from each.
///
/// The position to write next is the lower of ours and theirs; the base is advanced up to it and
/// consulted only where it holds that same id. A record only the base holds is passed over, which
/// is what the whole-file path does too: **absence means nothing** in this format, since forgetting
/// writes a tombstone rather than removing a line.
fn merge_in_step<W: Write>(
    base: &Opener<'_>,
    ours: &Opener<'_>,
    theirs: &Opener<'_>,
    out: &mut W,
) -> Result<Vec<MemoryId>, MergeError> {
    let mut base = InStep::open("base", base)?;
    let mut ours = InStep::open("ours", ours)?;
    let mut theirs = InStep::open("theirs", theirs)?;
    let mut conflicts = Vec::new();

    loop {
        let next = match (ours.position(), theirs.position()) {
            (None, None) => break,
            (Some(position), None) | (None, Some(position)) => position,
            (Some(our_position), Some(their_position)) => our_position.min(their_position),
        };
        let our_record = ours.take_at(next)?;
        let their_record = theirs.take_at(next)?;
        base.skip_before(next)?;
        let base_record = base.take_at(next)?;

        let record = decide(
            base_record.as_ref(),
            our_record.as_ref(),
            their_record.as_ref(),
            &mut conflicts,
        );
        memory_file::write_record(&record, out).map_err(unwritable)?;
    }

    Ok(conflicts)
}

/// Read three sides whole and merge them — the path for a side out of file order.
fn merge_whole<W: Write>(
    base: &Opener<'_>,
    ours: &Opener<'_>,
    theirs: &Opener<'_>,
    out: &mut W,
) -> Result<Vec<MemoryId>, MergeError> {
    let base_records = read_side("base", base)?;
    let our_records = read_side("ours", ours)?;
    let their_records = read_side("theirs", theirs)?;
    let mut conflicts = Vec::new();

    // Every id either side knows about, in file order — which the format requires, and which a merge
    // is the likeliest thing to disturb.
    let mut ids: Vec<MemoryId> = our_records.keys().copied().collect();
    ids.extend(their_records.keys().copied());
    ids.sort_unstable_by_key(|id| memory_file::file_position(*id));
    ids.dedup();

    for id in ids {
        let record = decide(
            base_records.get(&id),
            our_records.get(&id),
            their_records.get(&id),
            &mut conflicts,
        );
        memory_file::write_record(&record, out).map_err(unwritable)?;
    }

    Ok(conflicts)
}

/// The record to write for one id, from whichever sides hold it. Shared by both paths, so walking
/// in step and reading whole cannot decide one record two ways.
fn decide(
    base: Option<&Record>,
    ours: Option<&Record>,
    theirs: Option<&Record>,
    conflicts: &mut Vec<MemoryId>,
) -> Record {
    match (ours, theirs) {
        (Some(ours), Some(theirs)) => match merge_record(base, ours, theirs) {
            Merged::Clean(record) => record,
            Merged::Conflict(id) => {
                conflicts.push(id);
                ours.clone()
            }
        },
        // **Absence means nothing**, which the format states outright: forgetting writes a
        // tombstone rather than removing a line, so a record missing from one side was never a
        // deletion. Keeping it is the reading that cannot lose a memory.
        (Some(only), None) | (None, Some(only)) => only.clone(),
        (None, None) => unreachable!("an id came from one of the two sides"),
    }
}

/// Decide one record, given what the base held.
///
/// Public because it is the whole decision and is worth testing on its own; [`merge_files`] is the
/// walk over a file that calls it.
#[must_use]
pub fn merge_record(base: Option<&Record>, ours: &Record, theirs: &Record) -> Merged {
    if ours == theirs {
        return Merged::Clean(ours.clone());
    }

    match (ours, theirs) {
        (Record::Memory(our_memory), Record::Memory(their_memory)) => {
            let base_memory = match base {
                Some(Record::Memory(memory)) => Some(memory.as_ref()),
                // A memory on both sides where the base was a tombstone: both sides resurrected the
                // id independently. There is no ancestor for the fields, so any difference between
                // them is a divergence.
                Some(Record::Tombstone(_)) | None => None,
            };
            match merge_memories(base_memory, our_memory, their_memory) {
                Some(merged) => Merged::Clean(Record::Memory(Box::new(merged))),
                None => Merged::Conflict(ours.id()),
            }
        }

        (Record::Tombstone(our_stone), Record::Tombstone(their_stone)) => {
            // Both sides forgot the same memory. The act is the same whoever did it first, so this
            // is not a divergence — but the two tombstones differ in who and when, and one of them
            // has to be written. The later one, for the reason `modified` takes the later.
            Merged::Clean(Record::Tombstone(later_tombstone(our_stone, their_stone)))
        }

        // One side forgot the memory and the other did not.
        (Record::Memory(memory), Record::Tombstone(stone))
        | (Record::Tombstone(stone), Record::Memory(memory)) => {
            let untouched = matches!(
                base,
                Some(Record::Memory(base_memory)) if base_memory.as_ref() == memory.as_ref()
            );
            if untouched {
                // Delete against unmodified: a clean delete, which is what `git` concludes at the
                // line level for the same reason.
                Merged::Clean(Record::Tombstone(stone.clone()))
            } else {
                // Delete against modify. Nothing can tell whether the edit or the forgetting was the
                // later intent, and a memory is not a thing to guess about.
                Merged::Conflict(stone.id)
            }
        }
    }
}

/// The memory both sides hold, or `None` where they genuinely diverged.
///
/// **Destructured rather than field-accessed**, so a field added to [`Memory`] later fails to
/// compile here instead of being silently dropped from every merged record — which is the quiet
/// failure this whole module exists to avoid one level up.
fn merge_memories(base: Option<&Memory>, ours: &Memory, theirs: &Memory) -> Option<Memory> {
    let Memory {
        id,
        content: our_content,
        why: our_why,
        context: our_context,
        keywords: our_keywords,
        confidence: our_confidence,
        origin: our_origin,
        created: our_created,
        modified: our_modified,
        links: our_links,
    } = ours.clone();

    // `created` never moves. Two sides disagreeing about when a memory was first recorded is
    // corruption rather than an edit, and merging past it would bury that.
    if our_created != theirs.created {
        return None;
    }

    let mut merged = Memory {
        id,
        content: pick(base.map(|b| &b.content), &our_content, &theirs.content)?,
        why: pick(base.map(|b| &b.why), &our_why, &theirs.why)?,
        context: pick(base.map(|b| &b.context), &our_context, &theirs.context)?,
        keywords: merge_collection(
            base.map_or(&[], |b| b.keywords.as_slice()),
            &our_keywords,
            &theirs.keywords,
        ),
        confidence: pick(
            base.map(|b| &b.confidence),
            &our_confidence,
            &theirs.confidence,
        )?,
        origin: pick(base.map(|b| &b.origin), &our_origin, &theirs.origin)?,
        created: our_created,
        modified: our_modified.clone(),
        links: merge_collection(
            base.map_or(&[], |b| b.links.as_slice()),
            &our_links,
            &theirs.links,
        ),
    };

    // The later of the two, through `revise`, which is the only thing that moves `modified` and
    // therefore the only place the author and the timestamp can be kept together.
    let later = if theirs.modified.at > our_modified.at {
        &theirs.modified
    } else {
        &our_modified
    };
    merged.revise(later.by, later.at);

    Some(merged)
}

/// One scalar field, three ways: `None` where both sides changed it differently.
fn pick<T: Clone + PartialEq>(base: Option<&T>, ours: &T, theirs: &T) -> Option<T> {
    if ours == theirs {
        return Some(ours.clone());
    }
    match base {
        // Exactly one side moved away from the ancestor, so that side's value is the change.
        Some(base) if base == ours => Some(theirs.clone()),
        Some(base) if base == theirs => Some(ours.clone()),
        // Both moved, differently — or there is no ancestor to measure against.
        Some(_) | None => None,
    }
}

/// One collection field, three ways.
///
/// An item is kept when neither side removed it, and added when either side added it. **Never a
/// union**: an item in the base that one side dropped was dropped deliberately, and a union would
/// put it back. Order is base order first, then our additions, then theirs, so the result is stable
/// and reads as the sum of two edits rather than as a reshuffle.
fn merge_collection<T: Clone + PartialEq>(base: &[T], ours: &[T], theirs: &[T]) -> Vec<T> {
    let mut merged: Vec<T> = base
        .iter()
        .filter(|item| ours.contains(item) && theirs.contains(item))
        .cloned()
        .collect();

    for side in [ours, theirs] {
        for item in side {
            if !base.contains(item) && !merged.contains(item) {
                merged.push(item.clone());
            }
        }
    }

    merged
}

/// The later of two tombstones for one id.
fn later_tombstone(ours: &Tombstone, theirs: &Tombstone) -> Tombstone {
    if theirs.forgotten.at > ours.forgotten.at {
        theirs.clone()
    } else {
        ours.clone()
    }
}

/// Read one side whole, keeping its records by id — the whole-file path only.
///
/// **`Expectation::AnyShared`**, and the reasoning is the same one that makes it safe for a first
/// import: the scope *kind* is not negotiable, so this can never load a file claiming `local`. A
/// local set is never in a repository — it is the one set with no copy anywhere — so a merge driver
/// never legitimately meets one, and the three sides are checked against each other by
/// [`check_side`] before this runs.
fn read_side(
    side: &'static str,
    open: &Opener<'_>,
) -> Result<BTreeMap<MemoryId, Record>, MergeError> {
    let set = memory_file::read(&Expectation::AnyShared, open_side(side, open)?)
        .map_err(|error| unreadable(side, &error))?;

    let mut records = BTreeMap::new();
    for memory in set.memories {
        records.insert(memory.id, Record::Memory(Box::new(memory)));
    }
    for tombstone in set.tombstones {
        records.insert(tombstone.id, Record::Tombstone(tombstone));
    }

    Ok(records)
}

/// Pass one over a side: check it parses, and learn its scope and whether it is in file order.
///
/// Holds no records — each is parsed and dropped — so it costs what an import's validating pass
/// does. It reads to the end even once the order is known to be broken, because a line further down
/// that does not parse must still refuse the merge before anything is written.
fn check_side(side: &'static str, open: &Opener<'_>) -> Result<(Scope, bool), MergeError> {
    let mut previous = None;
    let mut in_order = true;
    let header = memory_file::stream(&Expectation::AnyShared, open_side(side, open)?, |record| {
        let position = memory_file::file_position(record.id());
        if previous.is_some_and(|previous| previous >= position) {
            in_order = false;
        }
        previous = Some(position);
        Ok(())
    })
    .map_err(|error| unreadable(side, &error))?;
    Ok((header.scope, in_order))
}

/// The receiving side's header, verbatim: its first line that is not blank, which is what every
/// reader takes the header to be.
fn header_line(ours: &Opener<'_>) -> Result<String, MergeError> {
    for line in BufReader::new(open_side("ours", ours)?).lines() {
        let line = line.map_err(|error| unreadable("ours", &error))?;
        if !line.trim().is_empty() {
            return Ok(line);
        }
    }
    Err(MergeError::NoHeader)
}

/// One side, walked in file order a record at a time.
struct InStep<'a> {
    side: &'static str,
    records: Records<Box<dyn Read + 'a>>,
    head: Option<((u64, MemoryId), Record)>,
    last: Option<(u64, MemoryId)>,
}

impl<'a> InStep<'a> {
    fn open(side: &'static str, open: &Opener<'a>) -> Result<Self, MergeError> {
        let (_, records) = Records::open(&Expectation::AnyShared, open_side(side, open)?)
            .map_err(|error| unreadable(side, &error))?;
        let mut walk = Self {
            side,
            records,
            head: None,
            last: None,
        };
        walk.advance()?;
        Ok(walk)
    }

    /// Where this side stands, or `None` once it is exhausted.
    fn position(&self) -> Option<(u64, MemoryId)> {
        self.head.as_ref().map(|(position, _)| *position)
    }

    /// The record at `position`, if this side is standing on it, moving past it.
    fn take_at(&mut self, position: (u64, MemoryId)) -> Result<Option<Record>, MergeError> {
        if self.position() != Some(position) {
            return Ok(None);
        }
        let record = self.head.take().map(|(_, record)| record);
        self.advance()?;
        Ok(record)
    }

    /// Move past every record before `position`.
    fn skip_before(&mut self, position: (u64, MemoryId)) -> Result<(), MergeError> {
        while self.position().is_some_and(|here| here < position) {
            self.advance()?;
        }
        Ok(())
    }

    /// Read the next record, refusing one out of order.
    ///
    /// **Checked again although pass one proved the order**, because the file is read a second time
    /// and a walk that met an out-of-order record would silently pair records wrongly rather than
    /// fail. It costs one comparison a record.
    fn advance(&mut self) -> Result<(), MergeError> {
        self.head = match self.records.next() {
            None => None,
            Some(item) => {
                let (_, record) = item.map_err(|error| unreadable(self.side, &error))?;
                let position = memory_file::file_position(record.id());
                if self.last.is_some_and(|last| last >= position) {
                    return Err(MergeError::Unreadable {
                        side: self.side,
                        detail: "it changed while being merged: its records were in file order \
                                 on the first read and are not on the second"
                            .to_owned(),
                    });
                }
                self.last = Some(position);
                Some((position, record))
            }
        };
        Ok(())
    }
}

fn open_side<'a>(side: &'static str, open: &Opener<'a>) -> Result<Box<dyn Read + 'a>, MergeError> {
    open().map_err(|error| unreadable(side, &error))
}

fn unreadable(side: &'static str, error: &dyn std::fmt::Display) -> MergeError {
    MergeError::Unreadable {
        side,
        detail: error.to_string(),
    }
}

fn unwritable(error: memory_file::MemoryFileError) -> MergeError {
    match error {
        memory_file::MemoryFileError::Io(error) => MergeError::Unwritable(error),
        other => MergeError::Unwritable(io::Error::other(other.to_string())),
    }
}
