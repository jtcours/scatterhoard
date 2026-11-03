// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the field-aware merge, the half of `git-merge-driver` that merges.
//!
//! `documents/design/multi-set-sync.md` settles the shape: `git` owns the file-against-file merge
//! with a real ancestor, and this refines the one case plain `git` gets coarsely wrong — two
//! branches editing *different fields* of the same record, which is one line to `git` and therefore
//! a conflict. A genuine same-field divergence still falls through to a conflict, which is the whole
//! reason this is safe to layer on: the worst case is what plain `git` would have done anyway.
//!
//! **The base is what makes this a merge rather than a guess.** `git` hands a driver `%O`, so a
//! collection field gets a real three-way set merge: an item one side added is kept, and an item one
//! side *removed* stays removed. A union would resurrect a retired keyword, which in this project is
//! a deliberate act — retiring a keyword is the only way to take one out of circulation.

use std::str::FromStr;

use jiff::Timestamp;
use scatterhoard::memory::{
    Attribution, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId, MemoryText,
    Origin, Scope, SetName,
};
use scatterhoard::memory_file::{self, Record, Tombstone};
use scatterhoard::merge::{self, Merged};

fn at(rfc3339: &str) -> Timestamp {
    Timestamp::from_str(rfc3339).expect("valid RFC 3339 timestamp")
}

/// One id per name, so two records by the same `who` share an author record.
fn author_record(who: &str) -> MemoryId {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static IDS: RefCell<HashMap<String, MemoryId>> = RefCell::new(HashMap::new());
    }
    IDS.with(|ids| {
        *ids.borrow_mut()
            .entry(who.to_owned())
            .or_insert_with(MemoryId::generate)
    })
}

fn attribution(who: &str, when: &str) -> Attribution {
    Attribution {
        by: author_record(who),
        at: at(when),
    }
}

fn a_memory(content: &str) -> Memory {
    Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        attribution("jtcours", "2026-08-01T09:00:00Z"),
    )
}

fn keyword(text: &str) -> Keyword {
    Keyword::new(text).expect("valid keyword")
}

fn links_to(target: MemoryId) -> Link {
    Link {
        label: LinkLabel::RelatedTo,
        target: LinkTarget::SameSet(target),
    }
}

fn a_scope() -> Scope {
    Scope::Shared(SetName::new("scatterhoard").expect("valid set name"))
}

/// A header carrying a distinguishing summary, as an older build wrote one, so a test can tell one
/// side's header from the other's. Current builds write no summary, so their headers cannot differ.
fn header_with(cluster_size: usize) -> String {
    format!(
        "{{\"format_version\":\"0.4.0\",\"scope\":{{\"shared\":\"scatterhoard\"}},\
         \"summary\":{{\"derivation\":\"clusters/2\",\"model\":\"fnv1a64:0000000000000000\",\
         \"clusters\":[{{\"size\":{cluster_size},\"keywords\":[\"one\",\"two\"]}}]}}}}"
    )
}

/// A memory file: one header line, then one line per record in file order, as an export writes it.
fn file_of(header: &str, records: &[Record]) -> String {
    let mut ordered: Vec<&Record> = records.iter().collect();
    ordered.sort_by_key(|record| memory_file::file_position(record.id()));
    lines_of(header, &ordered)
}

/// A memory file in **id order**, as every build before 2026-09-21 wrote one — the shape that sends
/// a merge down the whole-file path, because a side out of file order cannot be walked in step.
fn file_in_id_order(header: &str, records: &[Record]) -> String {
    let mut ordered: Vec<&Record> = records.iter().collect();
    ordered.sort_by_key(|record| record.id());
    lines_of(header, &ordered)
}

fn lines_of(header: &str, ordered: &[&Record]) -> String {
    let mut text = String::from(header);
    text.push('\n');
    for record in ordered {
        let mut line = Vec::new();
        memory_file::write_record(record, &mut line).expect("writing to a Vec cannot fail");
        text.push_str(std::str::from_utf8(&line).expect("valid UTF-8"));
    }
    text
}

fn memory_record(memory: &Memory) -> Record {
    Record::Memory(Box::new(memory.clone()))
}

/// The merged file, parsed back into the memory it carries for `id`.
fn merged_memory(text: &str, id: MemoryId) -> Memory {
    let set = memory_file::read(
        &memory_file::Expectation::Exactly(a_scope()),
        text.as_bytes(),
    )
    .expect("the merged file is a valid memory file");
    set.memories
        .into_iter()
        .find(|memory| memory.id == id)
        .expect("the merged file carries the record")
}

// ---------------------------------------------------------------------------
// The case the driver exists for
// ---------------------------------------------------------------------------

/// **Two branches, two different fields of one record.** To `git` this is one line edited twice, so
/// it conflicts; at the field level the two changes are compatible and both survive.
#[test]
fn edits_to_different_fields_of_one_record_both_survive() {
    let base = a_memory("the claim");
    let target = MemoryId::generate();

    let mut ours = base.clone();
    ours.confidence = Confidence::new(0.95).expect("in range");
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));

    let mut theirs = base.clone();
    theirs.links = vec![links_to(target)];
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&ours)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    let result = merged_memory(&merged.text, base.id);
    assert_eq!(
        result.confidence,
        Confidence::new(0.95).expect("in range"),
        "our confidence was lost"
    );
    assert_eq!(result.links, vec![links_to(target)], "their link was lost");
}

/// **The forced decision.** Any edit moves `modified`, so the two sides always collide there in
/// exactly the case above — treating that as a conflict would mean nothing ever merges. The later
/// attribution wins, and the merged record therefore claims something neither side wrote.
#[test]
fn a_merged_record_takes_the_later_modified() {
    let base = a_memory("the claim");

    let mut ours = base.clone();
    ours.confidence = Confidence::new(0.95).expect("in range");
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));

    let mut theirs = base.clone();
    theirs.context = Some(MemoryText::new("where it came up").expect("valid text"));
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&ours)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    let result = merged_memory(&merged.text, base.id);
    assert_eq!(
        result.modified,
        attribution("someone", "2026-09-02T10:00:00Z"),
        "the later modification should be the one the merged record claims"
    );
    assert_eq!(
        result.created, base.created,
        "created never moves, whatever the merge did"
    );
}

/// The same field, changed differently on both sides, is the case that must **not** be merged.
#[test]
fn the_same_field_changed_on_both_sides_is_a_conflict() {
    let base = a_memory("the original claim");

    let mut ours = base.clone();
    ours.content = MemoryText::new("our correction").expect("valid text");
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));

    let mut theirs = base.clone();
    theirs.content = MemoryText::new("their correction").expect("valid text");
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&ours)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    assert_eq!(
        merged.conflicts,
        vec![base.id],
        "a genuine divergence must fall through to a conflict"
    );
}

/// Identical edits on both sides are not a divergence: there is nothing to choose between.
#[test]
fn the_same_change_made_on_both_sides_merges_cleanly() {
    let base = a_memory("the original claim");

    let mut ours = base.clone();
    ours.content = MemoryText::new("the same correction").expect("valid text");
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));
    let theirs = ours.clone();

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&ours)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    assert_eq!(
        merged_memory(&merged.text, base.id).content.as_data(),
        "the same correction"
    );
}

/// **`created` is the field that never moves**, so two sides disagreeing about when a memory was
/// first recorded is corruption rather than an edit. Merging past it would bury that — and it is the
/// exact opposite case to `modified`, which always disagrees and is always resolved.
#[test]
fn a_record_whose_created_disagrees_is_a_conflict() {
    let base = a_memory("the claim");

    let mut ours = base.clone();
    ours.confidence = Confidence::new(0.95).expect("in range");
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));

    let mut theirs = base.clone();
    theirs.created = attribution("someone", "2026-07-01T09:00:00Z");
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&ours)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    assert_eq!(
        merged.conflicts,
        vec![base.id],
        "a disagreement about `created` must not be merged past"
    );
}

// ---------------------------------------------------------------------------
// Collections merge three ways, never by union
// ---------------------------------------------------------------------------

/// A keyword added on one side and one added on the other both survive.
#[test]
fn keywords_added_on_each_side_both_survive() {
    let mut base = a_memory("the claim");
    base.keywords = vec![keyword("shared")];

    let mut ours = base.clone();
    ours.keywords = vec![keyword("shared"), keyword("ours")];
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));

    let mut theirs = base.clone();
    theirs.keywords = vec![keyword("shared"), keyword("theirs")];
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&ours)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    let result = merged_memory(&merged.text, base.id);
    assert_eq!(
        result.keywords,
        vec![keyword("shared"), keyword("ours"), keyword("theirs")],
        "both additions should survive, in base-then-added order"
    );
}

/// **The case a union would get wrong.** One side retired a keyword; the other left it alone. The
/// base says it was there, so the removal is a change and the keyword stays gone.
#[test]
fn a_keyword_retired_on_one_side_stays_retired() {
    let mut base = a_memory("the claim");
    base.keywords = vec![keyword("kept"), keyword("retired")];

    let mut ours = base.clone();
    ours.keywords = vec![keyword("kept")];
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));

    let mut theirs = base.clone();
    theirs.confidence = Confidence::new(0.9).expect("in range");
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&ours)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    let result = merged_memory(&merged.text, base.id);
    assert_eq!(
        result.keywords,
        vec![keyword("kept")],
        "a union would have resurrected the retired keyword"
    );
}

/// Links merge the same way, and a link is the likelier of the two to be added concurrently.
#[test]
fn links_added_on_each_side_both_survive() {
    let base = a_memory("the claim");
    let ours_target = MemoryId::generate();
    let theirs_target = MemoryId::generate();

    let mut ours = base.clone();
    ours.links = vec![links_to(ours_target)];
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));

    let mut theirs = base.clone();
    theirs.links = vec![links_to(theirs_target)];
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&ours)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    let result = merged_memory(&merged.text, base.id);
    assert_eq!(
        result.links.len(),
        2,
        "both links should survive: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Records only one side touched, and records only one side has
// ---------------------------------------------------------------------------

#[test]
fn a_record_only_one_side_changed_takes_that_sides_version() {
    let base = a_memory("the claim");

    let mut theirs = base.clone();
    theirs.confidence = Confidence::new(0.5).expect("in range");
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&base)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    assert_eq!(
        merged_memory(&merged.text, base.id).confidence,
        Confidence::new(0.5).expect("in range")
    );
}

/// A memory each side added independently. Both are new, neither is in the base, and they are
/// different records — so both belong in the result.
#[test]
fn memories_added_on_each_side_both_appear() {
    let shared = a_memory("already here");
    let ours_new = a_memory("written on our branch");
    let theirs_new = a_memory("written on their branch");

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&shared)]),
        &file_of(
            &header_with(2),
            &[memory_record(&shared), memory_record(&ours_new)],
        ),
        &file_of(
            &header_with(3),
            &[memory_record(&shared), memory_record(&theirs_new)],
        ),
    )
    .expect("a valid merge");

    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    for id in [shared.id, ours_new.id, theirs_new.id] {
        let _ = merged_memory(&merged.text, id);
    }
}

// ---------------------------------------------------------------------------
// Forgetting
// ---------------------------------------------------------------------------

/// **Delete against modify is a genuine conflict**, which `multi-set-sync.md` states at the file
/// layer and which holds here for the same reason: nothing can decide whether the edit or the
/// forgetting was the later intent, and a memory is not something to guess about.
#[test]
fn forgetting_on_one_side_and_editing_on_the_other_conflicts() {
    let base = a_memory("the claim");

    let mut ours = base.clone();
    ours.content = MemoryText::new("our correction").expect("valid text");
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));

    let tombstone = Tombstone::new(
        base.id,
        attribution("someone", "2026-09-02T10:00:00Z"),
        Vec::new(),
        None,
    );

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&ours)]),
        &file_of(&header_with(3), &[Record::Tombstone(tombstone)]),
    )
    .expect("a valid merge");

    assert_eq!(merged.conflicts, vec![base.id]);
}

/// Forgetting on one side and leaving it alone on the other is not a conflict: one side changed.
#[test]
fn forgetting_on_one_side_alone_applies() {
    let base = a_memory("the claim");
    let tombstone = Tombstone::new(
        base.id,
        attribution("someone", "2026-09-02T10:00:00Z"),
        Vec::new(),
        None,
    );

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&base)]),
        &file_of(&header_with(3), &[Record::Tombstone(tombstone.clone())]),
    )
    .expect("a valid merge");

    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    let set = memory_file::read(
        &memory_file::Expectation::Exactly(a_scope()),
        merged.text.as_bytes(),
    )
    .expect("a valid memory file");
    assert!(set.memories.is_empty(), "the memory should be gone");
    assert_eq!(set.tombstones, vec![tombstone]);
}

// ---------------------------------------------------------------------------
// The header
// ---------------------------------------------------------------------------

/// **A header that differs between sides keeps ours verbatim.** Only a file an older build wrote can
/// differ, through the derived summary it carried, which a driver holding three files could not
/// recompute; no reader looks at that summary any more, so which side's survives does not matter.
#[test]
fn the_merged_file_keeps_our_header_verbatim() {
    let base = a_memory("the claim");
    let mut theirs = base.clone();
    theirs.confidence = Confidence::new(0.5).expect("in range");
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let ours_header = header_with(2);
    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&ours_header, &[memory_record(&base)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    assert_eq!(
        merged.text.lines().next().expect("a header line"),
        ours_header,
        "the receiving side's header should survive unchanged"
    );
}

/// Three ids whose file order is the reverse of their id order, so a test of either ordering cannot
/// pass by coincidence -- three random ids would share both orders one time in six.
const REVERSED_BY_POSITION: [&str; 3] = [
    "01M3378RFH2K8GV764KCRGE00K",
    "01M3378RFH4SXWF6E996X5208E",
    "01M3378RFHXHMFYFF1TK31CZT5",
];

fn with_id(mut memory: Memory, id: &str) -> Memory {
    memory.id = MemoryId::parse(id).expect("a valid id");
    memory
}

/// Records come back in file order, which the format requires: a merge that reordered lines would
/// make the next export look like a rewrite.
#[test]
fn the_merged_file_is_in_file_order() {
    let [one, two, three] = REVERSED_BY_POSITION;
    let first = with_id(a_memory("one"), one);
    let second = with_id(a_memory("two"), two);
    let third = with_id(a_memory("three"), three);

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&first)]),
        &file_of(
            &header_with(2),
            &[memory_record(&first), memory_record(&third)],
        ),
        &file_of(
            &header_with(3),
            &[memory_record(&first), memory_record(&second)],
        ),
    )
    .expect("a valid merge");

    let ids: Vec<MemoryId> = merged
        .text
        .lines()
        .skip(1)
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).expect("a record line");
            MemoryId::parse(value["id"].as_str().expect("an id")).expect("a valid id")
        })
        .collect();
    assert_eq!(
        ids,
        vec![third.id, second.id, first.id],
        "records must be written in file order"
    );
}

// ---------------------------------------------------------------------------
// Walking the sides in step, and the whole-file path
// ---------------------------------------------------------------------------
//
// `streaming-merge-driver`: sides in file order are read one record at a time, in step, so the merge
// never holds a whole set. A side out of file order cannot be walked that way and is read whole.

/// **Every record finds its own base when edits are spread through the file.** Walking in step means
/// the base is advanced past records neither side is on; a walk that failed to advance it, or
/// advanced it too far, would pair a record with no ancestor, and two compatible edits to it would
/// then read as a divergence.
#[test]
fn edits_spread_across_the_file_each_find_their_base() {
    let [one, two, three] = REVERSED_BY_POSITION;
    let first = with_id(a_memory("one"), one);
    let second = with_id(a_memory("two"), two);
    let third = with_id(a_memory("three"), three);

    let mut our_first = first.clone();
    our_first.keywords = vec![keyword("ours")];
    our_first.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));
    let mut our_second = second.clone();
    our_second.confidence = Confidence::new(0.95).expect("in range");
    our_second.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));

    let mut their_second = second.clone();
    their_second.keywords = vec![keyword("theirs")];
    their_second.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));
    let mut their_third = third.clone();
    their_third.confidence = Confidence::new(0.5).expect("in range");
    their_third.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(
            &header_with(1),
            &[
                memory_record(&first),
                memory_record(&second),
                memory_record(&third),
            ],
        ),
        &file_of(
            &header_with(2),
            &[
                memory_record(&our_first),
                memory_record(&our_second),
                memory_record(&third),
            ],
        ),
        &file_of(
            &header_with(3),
            &[
                memory_record(&first),
                memory_record(&their_second),
                memory_record(&their_third),
            ],
        ),
    )
    .expect("a valid merge");

    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    let second_merged = merged_memory(&merged.text, second.id);
    assert_eq!(
        second_merged.confidence,
        Confidence::new(0.95).expect("in range")
    );
    assert_eq!(second_merged.keywords, vec![keyword("theirs")]);
    assert_eq!(
        merged_memory(&merged.text, first.id).keywords,
        vec![keyword("ours")]
    );
    assert_eq!(
        merged_memory(&merged.text, third.id).confidence,
        Confidence::new(0.5).expect("in range")
    );
}

/// **A record only the base holds is passed over, and the walk still finds the next one's base.**
/// The base is the one side that can be standing on a record neither ours nor theirs has — a line
/// both sides dropped — and a walk that did not advance it past that record would leave every later
/// record without an ancestor, so two compatible edits after it would read as a divergence.
#[test]
fn a_record_only_the_base_holds_is_passed_over() {
    let [_, two, three] = REVERSED_BY_POSITION;
    let dropped = with_id(a_memory("dropped"), three);
    let kept = with_id(a_memory("kept"), two);

    let mut our_kept = kept.clone();
    our_kept.confidence = Confidence::new(0.95).expect("in range");
    our_kept.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));
    let mut their_kept = kept.clone();
    their_kept.keywords = vec![keyword("theirs")];
    their_kept.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(
            &header_with(1),
            &[memory_record(&dropped), memory_record(&kept)],
        ),
        &file_of(&header_with(2), &[memory_record(&our_kept)]),
        &file_of(&header_with(3), &[memory_record(&their_kept)]),
    )
    .expect("a valid merge");

    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    assert_eq!(merged.out_of_order, None);
    assert_eq!(merged.text.lines().count(), 2, "{}", merged.text);
}

/// Sides in file order are walked in step, and the result says so by naming no side.
#[test]
fn sides_in_file_order_are_walked_in_step() {
    let [one, two, _] = REVERSED_BY_POSITION;
    let first = with_id(a_memory("one"), one);
    let second = with_id(a_memory("two"), two);
    let both = [memory_record(&first), memory_record(&second)];

    let merged = merge::merge_files(
        &file_of(&header_with(1), &both),
        &file_of(&header_with(2), &both),
        &file_of(&header_with(3), &both),
    )
    .expect("a valid merge");

    assert_eq!(merged.out_of_order, None);
}

/// **A side out of file order still merges, to the same bytes, and is named.** Every file written
/// before 2026-09-21 is in id order, so for a while a merge base will routinely be one. It cannot be
/// walked in step, so it is read whole; naming it is what tells a person with a large set why a merge
/// was slow, and that re-exporting fixes it.
#[test]
fn a_side_out_of_file_order_merges_the_same_and_is_named() {
    let [one, two, three] = REVERSED_BY_POSITION;
    let first = with_id(a_memory("one"), one);
    let second = with_id(a_memory("two"), two);
    let third = with_id(a_memory("three"), three);
    let mut our_second = second.clone();
    our_second.confidence = Confidence::new(0.95).expect("in range");
    our_second.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));
    let mut their_second = second.clone();
    their_second.keywords = vec![keyword("theirs")];
    their_second.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let base = [
        memory_record(&first),
        memory_record(&second),
        memory_record(&third),
    ];
    let ours = [
        memory_record(&first),
        memory_record(&our_second),
        memory_record(&third),
    ];
    let theirs = [
        memory_record(&first),
        memory_record(&their_second),
        memory_record(&third),
    ];

    let in_step = merge::merge_files(
        &file_of(&header_with(1), &base),
        &file_of(&header_with(2), &ours),
        &file_of(&header_with(3), &theirs),
    )
    .expect("a valid merge");
    let whole = merge::merge_files(
        &file_in_id_order(&header_with(1), &base),
        &file_of(&header_with(2), &ours),
        &file_of(&header_with(3), &theirs),
    )
    .expect("a valid merge");

    assert_eq!(whole.out_of_order, Some("base"));
    assert_eq!(
        whole.text, in_step.text,
        "the whole-file path must write what walking in step writes"
    );
    assert!(whole.conflicts.is_empty(), "{:?}", whole.conflicts);
}

/// The first side out of order is the one named, checked in `git`'s order: base, ours, theirs.
#[test]
fn the_side_named_out_of_order_is_the_first_found() {
    let [one, two, _] = REVERSED_BY_POSITION;
    let both = [
        memory_record(&with_id(a_memory("one"), one)),
        memory_record(&with_id(a_memory("two"), two)),
    ];

    let merged = merge::merge_files(
        &file_of(&header_with(1), &both),
        &file_of(&header_with(2), &both),
        &file_in_id_order(&header_with(3), &both),
    )
    .expect("a valid merge");

    assert_eq!(merged.out_of_order, Some("theirs"));
}

/// **The streaming entry point reads each side from something it can open twice**, once to check
/// the side and once to merge it, and writes to a sink rather than returning the file — so the
/// driver can hand it paths and a file and hold neither side nor result.
#[test]
fn the_streaming_entry_point_writes_what_the_text_entry_point_returns() {
    let base = a_memory("the claim");
    let mut theirs = base.clone();
    theirs.confidence = Confidence::new(0.5).expect("in range");
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let base_text = file_of(&header_with(1), &[memory_record(&base)]);
    let our_text = file_of(&header_with(2), &[memory_record(&base)]);
    let their_text = file_of(&header_with(3), &[memory_record(&theirs)]);

    let open_base =
        || -> std::io::Result<Box<dyn std::io::Read + '_>> { Ok(Box::new(base_text.as_bytes())) };
    let open_ours =
        || -> std::io::Result<Box<dyn std::io::Read + '_>> { Ok(Box::new(our_text.as_bytes())) };
    let open_theirs =
        || -> std::io::Result<Box<dyn std::io::Read + '_>> { Ok(Box::new(their_text.as_bytes())) };
    let mut written = Vec::new();
    let outcome = merge::merge_streams(&open_base, &open_ours, &open_theirs, &mut written)
        .expect("a valid merge");

    let text = merge::merge_files(&base_text, &our_text, &their_text).expect("a valid merge");
    assert_eq!(String::from_utf8(written).expect("UTF-8"), text.text);
    assert_eq!(outcome.conflicts, text.conflicts);
}

/// **A side whose order changes between its two reads is refused, not mispaired.** The order is
/// proved on the first read and relied on in the second; a walk that met a record out of order would
/// pass records by without pairing them, and write a merge that silently lost edits.
#[test]
fn a_side_that_changes_between_reads_is_refused() {
    let [one, two, _] = REVERSED_BY_POSITION;
    let both = [
        memory_record(&with_id(a_memory("one"), one)),
        memory_record(&with_id(a_memory("two"), two)),
    ];
    let in_order = file_of(&header_with(1), &both);
    let reordered = file_in_id_order(&header_with(1), &both);
    let ours_text = file_of(&header_with(2), &both);

    let reads = std::cell::Cell::new(0);
    let open_base = || -> std::io::Result<Box<dyn std::io::Read + '_>> {
        reads.set(reads.get() + 1);
        let text = if reads.get() == 1 {
            &in_order
        } else {
            &reordered
        };
        Ok(Box::new(text.as_bytes()))
    };
    let open_other =
        || -> std::io::Result<Box<dyn std::io::Read + '_>> { Ok(Box::new(ours_text.as_bytes())) };

    let outcome = merge::merge_streams(&open_base, &open_other, &open_other, &mut Vec::new());

    assert!(
        matches!(
            outcome,
            Err(merge::MergeError::Unreadable { side: "base", .. })
        ),
        "{outcome:?}"
    );
}

/// **The header is the first line that is not blank**, which is what every reader takes it to be. A
/// merge that copied a leading blank line as the header would write a file with no header at all.
#[test]
fn a_leading_blank_line_is_not_taken_as_the_header() {
    let base = a_memory("the claim");
    let ours_header = header_with(2);
    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &format!("\n{}", file_of(&ours_header, &[memory_record(&base)])),
        &file_of(&header_with(3), &[memory_record(&base)]),
    )
    .expect("a valid merge");

    assert_eq!(merged.text.lines().next(), Some(ours_header.as_str()));
}

// ---------------------------------------------------------------------------
// What the merged bytes have to be
// ---------------------------------------------------------------------------

/// **A merged record must be byte-identical to what an export would write**, because the checksum
/// reconciliation keys on is taken over exactly those bytes. A driver that spelled a record its own
/// way would make every merged record read as changed at the next import.
#[test]
fn a_merged_record_is_spelled_the_way_an_export_spells_it() {
    let base = a_memory("the claim");
    let mut theirs = base.clone();
    theirs.confidence = Confidence::new(0.5).expect("in range");
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&base)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    let expected = file_of(&header_with(2), &[memory_record(&theirs)]);
    assert_eq!(
        merged.text, expected,
        "the merged file should be exactly what an export of this state writes"
    );
}

// ---------------------------------------------------------------------------
// Outcome reporting
// ---------------------------------------------------------------------------

/// A conflicted record keeps **our** version in the file, so the result is always a valid memory
/// file: `git` marks the merge failed by the driver's exit status, and the human resolves from a
/// file that parses rather than from one carrying conflict markers a memory file cannot hold.
#[test]
fn a_conflicted_record_keeps_our_version_and_the_file_still_parses() {
    let base = a_memory("the original claim");

    let mut ours = base.clone();
    ours.content = MemoryText::new("our correction").expect("valid text");
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));

    let mut theirs = base.clone();
    theirs.content = MemoryText::new("their correction").expect("valid text");
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let merged = merge::merge_files(
        &file_of(&header_with(1), &[memory_record(&base)]),
        &file_of(&header_with(2), &[memory_record(&ours)]),
        &file_of(&header_with(3), &[memory_record(&theirs)]),
    )
    .expect("a valid merge");

    assert_eq!(merged.conflicts, vec![base.id]);
    assert_eq!(
        merged_memory(&merged.text, base.id).content.as_data(),
        "our correction",
        "a conflicted record keeps our side"
    );
}

/// The record-level decision, reachable on its own: the pure function the file walk is built from.
#[test]
fn one_record_merges_without_a_file_around_it() {
    let base = a_memory("the claim");
    let mut ours = base.clone();
    ours.confidence = Confidence::new(0.95).expect("in range");
    ours.revise(author_record("jtcours"), at("2026-09-01T10:00:00Z"));
    let mut theirs = base.clone();
    theirs.keywords = vec![keyword("added")];
    theirs.revise(author_record("someone"), at("2026-09-02T10:00:00Z"));

    let outcome = merge::merge_record(
        Some(&memory_record(&base)),
        &memory_record(&ours),
        &memory_record(&theirs),
    );

    let Merged::Clean(record) = outcome else {
        panic!("compatible field edits should merge: {outcome:?}");
    };
    let Record::Memory(memory) = record else {
        panic!("a memory merged with a memory is a memory");
    };
    assert_eq!(memory.confidence, Confidence::new(0.95).expect("in range"));
    assert_eq!(memory.keywords, vec![keyword("added")]);
}
