// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the exported memory file.
//!
//! The format exists to be merged by `git` and reviewed by a human (see
//! `documents/design/file-format.md` and `documents/design/multi-set-sync.md`), so most of these
//! test properties that serve *those* goals rather than round-tripping alone: one memory per line,
//! a stable order, and absent fields taking up no room.

use std::str::FromStr;

use jiff::Timestamp;
use scatterhoard::memory::{
    Attribution, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId, MemoryText,
    Origin, Scope, SetId, SetName,
};
use scatterhoard::memory_file::{
    self, Expectation, FORMAT_VERSION, MemoryFileError, MemorySet, OLDEST_READABLE, Tombstone,
};

fn at(rfc3339: &str) -> Timestamp {
    Timestamp::from_str(rfc3339).expect("valid RFC 3339 timestamp")
}

/// One id per name, so two records by the same `who` still share an author record.
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

fn a_scope() -> Scope {
    Scope::Shared(SetName::new("scatterhoard").expect("valid set name"))
}

fn a_set(memories: Vec<Memory>) -> MemorySet {
    MemorySet {
        scope: a_scope(),
        memories,
        tombstones: Vec::new(),
        set_id: None,
    }
}

/// Most tests here are about the format rather than about trust, so they assert the scope they
/// wrote. The scope check itself is exercised separately, below.
fn read_project(text: &str) -> Result<MemorySet, MemoryFileError> {
    memory_file::read(&Expectation::Exactly(a_scope()), text.as_bytes())
}

fn write_to_string(set: &MemorySet) -> String {
    let mut out = Vec::new();
    memory_file::write(set, &mut out).expect("writes");
    String::from_utf8(out).expect("utf-8")
}

// ---------------------------------------------------------------------------
// Shape of the file
// ---------------------------------------------------------------------------

fn header_of(text: &str) -> serde_json::Value {
    serde_json::from_str(text.lines().next().expect("a header line")).expect("header is JSON")
}

#[test]
fn the_header_declares_the_format_version_and_the_scope() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    let header = header_of(&text);

    assert_eq!(header["format_version"], FORMAT_VERSION);
    assert_eq!(header["scope"]["shared"], "scatterhoard");
}

/// The local set is written too — export and backup share this serialiser, and backup/restore is
/// the reason a local set is ever written at all. Whether a set may be *published* is a separate
/// question that `Scope::exports_to` answers, and this function does not ask it.
fn a_local_file() -> String {
    write_to_string(&MemorySet {
        scope: Scope::Local,
        memories: vec![a_memory("a personal preference")],
        tombstones: Vec::new(),
        set_id: None,
    })
}

#[test]
fn a_local_set_round_trips() {
    let original = MemorySet {
        scope: Scope::Local,
        memories: vec![a_memory("a personal preference")],
        tombstones: Vec::new(),
        set_id: None,
    };

    let parsed = memory_file::read(
        &Expectation::Exactly(Scope::Local),
        write_to_string(&original).as_bytes(),
    )
    .expect("reads");
    assert_eq!(parsed, original);
}

#[test]
fn a_local_file_announces_itself_as_local() {
    let local = a_local_file();
    let project = write_to_string(&a_set(vec![a_memory("a project decision")]));

    assert_eq!(header_of(&local)["scope"], "local");
    assert!(
        header_of(&local) != header_of(&project),
        "a local file must not be mistakable for a project file"
    );
}

// ---------------------------------------------------------------------------
// The scope in the header is a guard, never a source
// ---------------------------------------------------------------------------

/// The escalation this whole mechanism exists to stop: a repository ships a memory file whose
/// header claims `local`, and an importer that believed the header would load a contributor's
/// memories into the more-trusted local store. Path decides scope; the header only gets to agree.
#[test]
fn a_project_import_refuses_a_file_claiming_local_scope() {
    let local = a_local_file();

    assert!(
        read_project(&local).is_err(),
        "a file claiming local scope must not import as a project set"
    );
    assert!(
        memory_file::read(&Expectation::AnyShared, local.as_bytes()).is_err(),
        "AnyShared must never match a local file, even with no set name bound yet"
    );
}

/// The cross-project variant of the same bug: repository A ships a file declaring itself to be
/// set B, and an importer that trusted it would inject A's memories into B's store.
#[test]
fn a_file_declaring_a_different_set_is_refused() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    let other = Scope::Shared(SetName::new("other-project").expect("valid set name"));

    assert!(
        memory_file::read(&Expectation::Exactly(other), text.as_bytes()).is_err(),
        "a file declaring one set must not be read as another"
    );
}

/// A restore targeting the local store must not accept a project's file either — the guard has to
/// work in both directions, or backup/restore becomes a laundering route for project memories.
#[test]
fn a_local_restore_refuses_a_project_file() {
    let project = write_to_string(&a_set(vec![a_memory("a project decision")]));

    assert!(
        memory_file::read(&Expectation::Exactly(Scope::Local), project.as_bytes()).is_err(),
        "a project file must not restore into the local store"
    );
}

/// First import, before any set name is bound to this checkout: the name is not yet knowable from
/// trusted state, so it is taken from the file — which is safe, because the *kind* still is not.
#[test]
fn a_first_import_accepts_a_project_file_and_reports_its_declared_set() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));

    let parsed =
        memory_file::read(&Expectation::AnyShared, text.as_bytes()).expect("first import reads");
    assert_eq!(parsed.scope, a_scope());
}

/// One memory per line is what confines a change to one memory to a single line in a merge, and
/// what the field-level merge driver is built around.
#[test]
fn each_memory_occupies_exactly_one_line() {
    let memories: Vec<_> = (0..5)
        .map(|i| a_memory(&format!("memory number {i}\nwith an embedded newline")))
        .collect();
    let text = write_to_string(&a_set(memories));

    assert_eq!(
        text.lines().count(),
        6,
        "expected a header plus five memories, one per line"
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

/// The ids of a written file's records, in the order the file holds them.
fn ids_in_file_order(text: &str) -> Vec<MemoryId> {
    text.lines()
        .skip(1)
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).expect("a record");
            MemoryId::parse(value["id"].as_str().expect("an id")).expect("a valid id")
        })
        .collect()
}

fn sorted_by_file_position(mut ids: Vec<MemoryId>) -> Vec<MemoryId> {
    ids.sort_by_key(|id| memory_file::file_position(*id));
    ids
}

/// **The position is part of the format**, so it is pinned by value: a change to how it is computed
/// would reorder every file in one diff. The value is FNV-1a over the id's canonical text, computed
/// independently of this crate.
#[test]
fn a_records_file_position_is_pinned() {
    let id = MemoryId::parse("01M3378RFHFC8B4PQ75FGJ2HNA").expect("a valid id");
    assert_eq!(memory_file::file_position(id), (0x5151_fe03_ee3e_00f0, id));
}

/// The export order decision: records are written by a position derived from the id alone, so the
/// same content always produces the same bytes and a record never moves once written. **Not by id**:
/// ids are UUIDv7, so id order put every new memory at the end of the file, where two branches that
/// both added memories always collided (`concurrent-additions-collide-at-the-end-of-the-memory-file`).
#[test]
fn memories_are_written_in_file_order_whatever_order_they_arrive_in() {
    let mut memories: Vec<_> = (0..10).map(|i| a_memory(&format!("memory {i}"))).collect();
    memories.reverse();

    let text = write_to_string(&a_set(memories));
    let ids = ids_in_file_order(&text);
    assert_eq!(
        ids,
        sorted_by_file_position(ids.clone()),
        "memories were not written in file order"
    );
}

/// Absent optional fields must take up no room in the file: every byte written is a byte a
/// reviewer reads and a byte the merge driver reconciles.
#[test]
fn absent_optional_fields_are_omitted_entirely() {
    let text = write_to_string(&a_set(vec![a_memory("a memory with no optional fields")]));
    let line = text.lines().nth(1).expect("a memory line");

    for absent in ["why", "context", "keywords", "links"] {
        assert!(
            !line.contains(absent),
            "`{absent}` should be omitted when empty, got {line}"
        );
    }
}

/// Scope is a property of the file, not of a memory: the file *is* the set. A memory carrying its
/// own scope could disagree with the file containing it.
#[test]
fn a_memory_does_not_carry_its_own_scope() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    let line = text.lines().nth(1).expect("a memory line");

    assert!(
        !line.contains("scope"),
        "scope belongs to the file header, not to each memory: {line}"
    );
}

// ---------------------------------------------------------------------------
// Round trips
// ---------------------------------------------------------------------------

#[test]
fn a_minimal_set_round_trips() {
    let original = a_set(vec![a_memory("the only memory")]);
    let parsed = read_project(&write_to_string(&original)).expect("reads");
    assert_eq!(parsed, original);
}

#[test]
fn a_fully_populated_memory_round_trips() {
    let mut memory = a_memory("storage is partitioned by scope");
    memory.why = Some(MemoryText::new("a forgotten WHERE clause leaks personal content").unwrap());
    memory.context = Some(MemoryText::new("decided at the end of the session").unwrap());
    memory.keywords = vec![
        Keyword::new("storage").unwrap(),
        Keyword::new("scope").unwrap(),
    ];
    memory.links = vec![
        Link {
            label: LinkLabel::Supersedes,
            target: LinkTarget::SameSet(MemoryId::generate()),
        },
        Link {
            label: LinkLabel::RefersTo,
            target: LinkTarget::CrossSet {
                set: SetName::new("other-project").unwrap(),
                id: MemoryId::generate(),
                keywords: vec![Keyword::new("hubness").unwrap()],
                set_id: None,
            },
        },
    ];
    memory.revise(author_record("someone-else"), at("2026-08-02T11:00:00Z"));

    let original = a_set(vec![memory]);
    let parsed = read_project(&write_to_string(&original)).expect("reads");
    assert_eq!(parsed, original);
}

/// A merge can reorder lines, so reading must not depend on the file arriving sorted — but what
/// comes back is sorted regardless, so a read-then-write cycle is stable.
#[test]
fn reading_tolerates_unsorted_input_and_returns_sorted_memories() {
    let memories: Vec<_> = (0..6).map(|i| a_memory(&format!("memory {i}"))).collect();
    let text = write_to_string(&a_set(memories));

    let mut lines: Vec<&str> = text.lines().collect();
    let header = lines.remove(0);
    lines.reverse();
    let shuffled = format!("{header}\n{}\n", lines.join("\n"));

    let parsed = read_project(&shuffled).expect("reads unsorted input");
    let ids: Vec<_> = parsed.memories.iter().map(|m| m.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();

    assert_eq!(ids, sorted, "read should return memories in id order");
    assert_eq!(write_to_string(&parsed), text, "read-write is not stable");
}

// ---------------------------------------------------------------------------
// Rejecting bad files
// ---------------------------------------------------------------------------

#[test]
fn a_file_with_no_header_is_rejected() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    let body = text.lines().skip(1).collect::<Vec<_>>().join("\n");

    assert!(
        read_project(&body).is_err(),
        "a file whose first line is a memory must not be read as headerless"
    );
    assert!(read_project("").is_err(), "empty file");
}

/// A major version this build does not know may have moved fields it depends on, so it is refused
/// rather than partially understood.
#[test]
fn a_file_from_an_unknown_major_version_is_rejected() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    let bumped = text.replacen(FORMAT_VERSION, "99.0.0", 1);

    assert!(
        read_project(&bumped).is_err(),
        "a file from a future major version must be refused"
    );
}

/// **A file from a newer minor is refused while the format is `0.x`, and this is the gate's whole
/// point.** Below `1.0` semver promises nothing about minor bumps, and this project has used them
/// for both kinds of change: `0.2.0` respelled every id, `0.3.0` only added tombstone fields. So a
/// higher minor may carry a shape this build cannot parse, and the alternative to refusing is what
/// `the-format-version-gates-nothing-while-the-format-is-0x` records — a malformed-record error
/// partway through a file, or serde's *"data did not match any variant"* on a link.
#[test]
fn a_file_from_a_newer_minor_is_refused_while_the_format_is_zero_x() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    let ahead = text.replacen(FORMAT_VERSION, "0.99.0", 1);

    let error = read_project(&ahead).expect_err("a newer minor must be refused");
    let complaint = error.to_string();
    assert!(
        complaint.contains("0.99.0") && complaint.contains(FORMAT_VERSION),
        "the refusal must name both versions so a reader knows which side to move: {complaint}"
    );
}

/// **And an older minor this build still understands is accepted**, which is the half that keeps
/// the gate from being a wall. `0.1.0` spelled ids in hex and `MemoryId::parse` still reads that
/// form, so a file that old is genuinely readable and refusing it would strand the sets that exist.
#[test]
fn a_file_from_an_older_readable_minor_is_accepted() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    let older = text.replacen(FORMAT_VERSION, OLDEST_READABLE, 1);

    let parsed = read_project(&older).expect("a version this build still reads");
    assert_eq!(parsed.memories.len(), 1);
}

/// **Below the floor is refused, and differently.** *Too new* and *too old* have opposite remedies —
/// update this build, or re-export from one that still writes that shape — so they must not share a
/// message. This is the lever the format cut uses: raising `OLDEST_READABLE` is how a build declares
/// that a shape it used to read is no longer one it understands.
#[test]
fn a_file_older_than_this_build_reads_is_refused_and_says_so() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    // `0.0.x` is below every floor this build could declare, so this test does not have to move
    // when `OLDEST_READABLE` does.
    let ancient = text.replacen(FORMAT_VERSION, "0.0.1", 1);

    let error = read_project(&ancient).expect_err("below the floor must be refused");
    let complaint = error.to_string();
    assert!(
        complaint.contains("0.0.1") && complaint.contains(OLDEST_READABLE),
        "the refusal must name the oldest version this build reads: {complaint}"
    );
    assert_ne!(
        complaint,
        read_project(&text.replacen(FORMAT_VERSION, "0.99.0", 1))
            .expect_err("newer is refused too")
            .to_string(),
        "too old and too new have opposite remedies and must not read the same"
    );
}

/// The constants have to be consistent with each other, and nothing else checks it: a floor above
/// what this build writes would refuse every file it had just written.
#[test]
fn the_oldest_readable_version_is_not_newer_than_what_this_build_writes() {
    let parse = |version: &str| {
        let mut parts = version.split('.');
        let major: u64 = parts.next().expect("major").parse().expect("numeric major");
        let minor: u64 = parts.next().expect("minor").parse().expect("numeric minor");
        (major, minor)
    };
    assert!(
        parse(OLDEST_READABLE) <= parse(FORMAT_VERSION),
        "OLDEST_READABLE {OLDEST_READABLE} is newer than FORMAT_VERSION {FORMAT_VERSION}"
    );
}

/// A file this build wrote is one it reads. Trivial, and it is the assertion that would have caught
/// a gate whose bounds were the wrong way round.
#[test]
fn this_builds_own_output_passes_its_own_gate() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    assert!(
        read_project(&text).is_ok(),
        "a build must read its own file"
    );
}

/// **The gate's contract, as a range rather than a list of versions.**
///
/// This used to loop over `0.1.0`, `0.2.0` and `0.3.0` asserting each still read, and that tested
/// the gate and nothing else: it swapped the version string on a file *this build wrote*, so the
/// records, the id spelling and the header were all current. A version means the shape of the
/// records under it, and this test never saw an old one. Its sibling
/// `a_file_in_the_shape_0_1_0_was_actually_written_in_is_refused` carries a genuine old file and is
/// where that claim now lives.
///
/// What is true and worth pinning is the range and the two refusals, which have opposite remedies:
/// too old is fixed by re-exporting from a build that still writes that shape, and too new by
/// updating this build. A gate that refused with one message would send half its callers the wrong
/// way.
#[test]
fn the_version_gate_accepts_its_declared_range_and_refuses_outside_it() {
    let current = write_to_string(&a_set(vec![a_memory("a memory")]));

    for version in [OLDEST_READABLE, FORMAT_VERSION] {
        let declared = current.replacen(FORMAT_VERSION, version, 1);
        assert!(
            read_project(&declared).is_ok(),
            "{version} is inside the range this build declares"
        );
    }

    // One step below the floor. `0.3.0` is the version immediately before the attribution cut, and
    // the one that most needs refusing: it is the last shape that held a *name* where `by` now
    // holds an id, so letting it through would surface as a malformed record rather than a refusal.
    let below = current.replacen(FORMAT_VERSION, "0.3.0", 1);
    let error = read_project(&below).expect_err("below the floor");
    assert!(
        error.to_string().contains("re-exported") && error.to_string().contains(OLDEST_READABLE),
        "the too-old refusal must name the remedy and the floor, and got: {error}"
    );

    let above = current.replacen(FORMAT_VERSION, "0.5.0", 1);
    let error = read_project(&above).expect_err("above what this build writes");
    assert!(
        error.to_string().contains(FORMAT_VERSION),
        "the too-new refusal must name what this build writes, and got: {error}"
    );
}

/// A version string that is not a version at all is refused, and refused as **too new**.
///
/// The reader cannot tell a corrupted header from one written by something that versions
/// differently, and of the two readings only *newer* is safe: it stops, where treating it as
/// readable would hand a header nobody understands to the record parser and surface the problem as
/// a malformed line. A third "unknown version" message was considered and is not worth it — it
/// would say nothing the caller could act on that this one does not.
#[test]
fn a_version_that_does_not_parse_is_refused_rather_than_attempted() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    for nonsense in ["banana", "", "0.x.0", "one.two"] {
        let declared = text.replacen(FORMAT_VERSION, nonsense, 1);
        assert!(
            read_project(&declared).is_err(),
            "a version of {nonsense:?} must not be read as this build's own"
        );
    }
}

/// Within a major version, added fields are a compatible change — and the labelled corpus carries
/// `_`-prefixed annotations that an importer is meant to ignore.
#[test]
fn unknown_fields_within_the_same_major_version_are_ignored() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    let mut lines = text.lines();
    let header = lines.next().expect("header");
    let memory = lines.next().expect("memory");

    // Built through serde rather than string surgery: the header now nests an object for the
    // scope, so appending a field by replacing a brace would land it in the wrong place.
    let annotate = |line: &str, key: &str, value: serde_json::Value| {
        let mut object: serde_json::Value = serde_json::from_str(line).expect("line is JSON");
        object[key] = value;
        object.to_string()
    };

    let annotated = format!(
        "{}\n{}\n",
        annotate(header, "future_field", serde_json::json!(true)),
        annotate(memory, "_key", serde_json::json!("some-label")),
    );

    let parsed = read_project(&annotated).expect("unknown fields should be ignored");
    assert_eq!(parsed.memories.len(), 1);
}

/// Corrupts line 2 by position rather than by content.
///
/// The first version matched on `{"id"` to find a record line, which coupled it to the order fields
/// happened to be emitted in — and it silently stopped corrupting anything when the canonical form
/// became sorted, so it passed while asserting nothing. A line is identified by *being* line 2.
#[test]
fn a_malformed_memory_line_is_rejected_and_names_its_line() {
    let text = write_to_string(&a_set(vec![a_memory("one"), a_memory("two")]));
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    lines[1] = "{not json".to_owned();
    let broken = lines.join("\n") + "\n";

    let error = read_project(&broken).expect_err("malformed line must be rejected");
    assert!(
        error.to_string().contains('2'),
        "the error should name the offending line, got: {error}"
    );
}

/// Two lines with the same id is what a bad merge produces, and it is not resolvable by picking
/// one — the reader refuses rather than silently dropping a memory.
#[test]
fn duplicate_ids_are_rejected() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    let memory_line = text.lines().nth(1).expect("a memory line");
    let duplicated = format!("{text}{memory_line}\n");

    assert!(
        read_project(&duplicated).is_err(),
        "a repeated id must be rejected"
    );
}

// ---------------------------------------------------------------------------
// The labelled corpus
// ---------------------------------------------------------------------------

/// `corpus/design-memories.jsonl` was written to drive the first real import, and is the only file
/// in the repository this code did not produce — so it is the one honest check that the format is
/// writable by something other than `write` itself. It also exercises the `_`-prefixed annotation
/// fields an importer is meant to ignore.
#[test]
fn the_labelled_corpus_reads() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/corpus/design-memories.jsonl");
    let file = std::fs::File::open(path).expect("the corpus file should exist");

    // Read as a first import would: the set name is not bound to anything here, and `AnyShared`
    // is what says so without letting the file choose its own scope kind.
    let set = memory_file::read(&Expectation::AnyShared, file)
        .expect("the corpus should read as a memory file");

    assert_eq!(
        set.scope,
        Scope::Shared(SetName::new("scatterhoard").expect("valid set name"))
    );
    // 40 since the corpus gained the identity record its `by` fields now refer to.
    assert_eq!(set.memories.len(), 40);

    let edges: usize = set.memories.iter().map(|m| m.links.len()).sum();
    assert_eq!(edges, 29, "the corpus should carry 29 labelled edges");

    assert!(
        set.memories.iter().all(|m| m.why.is_some()),
        "every corpus memory was written with its reasoning"
    );
}

/// Parse, don't validate: an invalid confidence in a file from elsewhere fails at the boundary
/// rather than becoming a value later code trusts.
#[test]
fn a_memory_with_an_invalid_field_is_rejected_at_the_boundary() {
    let text = write_to_string(&a_set(vec![a_memory("a memory")]));
    let broken = text.replace("0.8", "1.8");

    assert!(
        read_project(&broken).is_err(),
        "an out-of-range confidence must not survive reading"
    );
}

// ---------------------------------------------------------------------------
// One id, one record.
//
// `a-merged-file-with-a-duplicate-id-silently-drops-a-memory`: a file carrying an id twice imported
// successfully and lost a record, because `INSERT OR REPLACE` picks the later one. A hand-merge of
// two sets produces this whenever a memory was ever copied between them.
// ---------------------------------------------------------------------------

/// Two memories sharing an id is a **refusal**, not a silent replacement.
///
/// Refused at the read rather than at the store, because `stream` is the one chokepoint every file
/// passes through — the reconciling import uses it directly and never calls `read` — and because
/// the condition is a property of the file, which is what this layer is for.
///
/// **The one place refusal is plainly right.** The design's standing preference is to report and
/// carry on, and its stated boundary is that refusal is correct where the caller holds the remedy.
/// Here they do: it is their file, the duplicate is visible in it, and only they can say which
/// record should survive. Carrying on means choosing a winner silently, which is the defect.
#[test]
fn two_memories_sharing_an_id_are_refused() {
    let first = a_memory("the memory from one set");
    let mut second = a_memory("a different memory that shares the id");
    second.id = first.id;

    let text = write_to_string(&a_set(vec![first.clone(), second]));
    let error = read_project(&text).expect_err("a duplicate id must not be accepted");

    let message = error.to_string();
    assert!(
        message.contains(&first.id.to_string()),
        "the refusal must name the id, since the caller has to find it in their file: {message}"
    );
}

/// A memory and a tombstone for the same id is the same malformation wearing a different hat: the
/// record both exists and was forgotten. Covered by the same rule rather than a second one.
#[test]
fn a_memory_and_its_own_tombstone_are_refused() {
    let memory = a_memory("a memory that is also claimed forgotten");
    let text = format!(
        "{}{}\n",
        write_to_string(&a_set(vec![memory.clone()])),
        serde_json::json!({
            "id": memory.id.to_string(),
            "forgotten": {"by": author_record("jtcours").to_string(), "at": "2026-08-02T09:00:00Z"}
        })
    );

    let error = read_project(&text).expect_err("a memory and its tombstone must not both stand");
    assert!(error.to_string().contains(&memory.id.to_string()));
}

/// And an ordinary file is untouched. Worth pinning: a check keyed on identity is one off-by-one
/// from refusing every well-formed set.
#[test]
fn distinct_ids_are_not_refused() {
    let set = a_set(vec![a_memory("one"), a_memory("two"), a_memory("three")]);
    let text = write_to_string(&set);
    assert_eq!(
        read_project(&text)
            .expect("a well-formed set must still read")
            .memories
            .len(),
        3
    );
}

// ---------------------------------------------------------------------------
// A whole-file read holds the whole file
//
// `memory-file-read-silently-discards-tombstones`. `read` and `write` are a matched pair over one
// type, and both dropped tombstones -- `read` filtered them out to keep a `Vec<Memory>` compiling
// when the format grew a second record shape, and `write` only ever iterated `memories`. So a
// `MemorySet` claimed to be a file's contents while holding a subset, in both directions, with no
// count, flag or error to say so.
// ---------------------------------------------------------------------------

fn a_tombstone(id: MemoryId) -> Tombstone {
    Tombstone::new(
        id,
        attribution("jtcours", "2026-08-02T09:00:00Z"),
        Vec::new(),
        None,
    )
}

/// A tombstone in the file reaches the value the reader hands back.
#[test]
fn reading_a_file_keeps_its_tombstones() {
    let kept = a_memory("a memory that stayed");
    let gone = a_memory("a memory that did not");
    let text = write_to_string(&MemorySet {
        scope: a_scope(),
        memories: vec![kept.clone()],
        tombstones: vec![a_tombstone(gone.id)],
        set_id: None,
    });

    let parsed = read_project(&text).expect("reads");
    assert_eq!(parsed.memories.len(), 1);
    assert_eq!(
        parsed.tombstones.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![gone.id],
        "a tombstone is a record of the file, not something a reader may drop"
    );
    assert_eq!(parsed.tombstones[0].forgotten.by, author_record("jtcours"));
}

/// And a tombstone survives the whole round trip, which is the pair working rather than either half.
#[test]
fn tombstones_survive_a_write_and_read_round_trip() {
    let original = MemorySet {
        scope: a_scope(),
        memories: vec![a_memory("still here")],
        tombstones: vec![a_tombstone(a_memory("forgotten").id)],
        set_id: None,
    };

    let once = write_to_string(&original);
    let parsed = read_project(&once).expect("reads");
    let twice = write_to_string(&parsed);

    assert_eq!(
        once, twice,
        "writing what was read must reproduce the file exactly"
    );
}

/// **Records interleave by file position, tombstones included.**
///
/// `documents/design/file-format.md` orders a file by a position derived from the id, so a record
/// never moves and duplicate ids are adjacent. Writing the memories and then the tombstones would
/// order by *shape* instead, which sorts nothing a reader cares about and makes every diff that
/// forgets a memory move a line to the end of the file.
#[test]
fn a_written_file_orders_every_record_by_file_position_whatever_its_shape() {
    let [one, two, three] = REVERSED_BY_POSITION;
    let first = with_id(a_memory("first"), one);
    let second = with_id(a_memory("second"), two);
    let third = with_id(a_memory("third"), three);

    let text = write_to_string(&MemorySet {
        scope: a_scope(),
        memories: vec![third.clone(), first.clone()],
        tombstones: vec![a_tombstone(second.id)],
        set_id: None,
    });

    assert_eq!(
        ids_in_file_order(&text),
        vec![third.id, second.id, first.id],
        "the tombstone belongs at its own position, not after the memories"
    );
}

/// **A cross-set link with no identity writes the bytes it always did**, which is the property the
/// whole migration rests on: every record already holding one keeps its checksum, so none of them
/// reconciles as modified and none needs rewriting. Adding a field that serialised as `null` would
/// have moved every one of them.
#[test]
fn a_cross_set_link_without_an_identity_adds_nothing_to_the_line() {
    let mut memory = a_memory("alpha pointing into another set");
    memory.links = vec![Link {
        label: LinkLabel::RelatedTo,
        target: LinkTarget::CrossSet {
            set: SetName::new("elsewhere").expect("valid set name"),
            id: MemoryId::generate(),
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let text = write_to_string(&a_set(vec![memory]));
    assert!(
        !text.contains("set_id"),
        "an absent identity must not appear in the file at all: {text}"
    );
}

/// And when it is there, it is there — written and read back, so the identity survives the round
/// trip that carries it to another machine.
#[test]
fn a_cross_set_links_identity_survives_a_write_and_read() {
    let set_id = SetId::generate();
    let mut memory = a_memory("alpha pointing into a set we can name exactly");
    memory.links = vec![Link {
        label: LinkLabel::RelatedTo,
        target: LinkTarget::CrossSet {
            set: SetName::new("elsewhere").expect("valid set name"),
            id: MemoryId::generate(),
            keywords: Vec::new(),
            set_id: Some(set_id),
        },
    }];

    let text = write_to_string(&a_set(vec![memory]));
    let read = read_project(&text).expect("reads back");
    let LinkTarget::CrossSet { set_id: found, .. } = &read.memories[0].links[0].target else {
        panic!("the link should still be cross-set");
    };
    assert_eq!(*found, Some(set_id));
}

/// **A real `0.1.0` file, in the shape one was actually written in.**
///
/// `every_format_version_this_project_has_written_is_still_readable` above swaps the version string
/// on a file this build wrote, which tests the gate and nothing else: the records, the id spelling
/// and the header's scope are all current. A genuine `0.1.0` file differs in two ways that the
/// version number does not describe — the scope is a **bare string** rather than an object, and ids
/// are **36-character hyphenated hex** rather than Crockford base32.
///
/// Written after finding that this machine's only backup of the `local` set is such a file, from
/// 2026-08-12, and realising nothing established it could still be read.
///
/// **It no longer reads, and that is the decision rather than a regression.** `by` became a
/// reference to a `hoard-author` memory, so a file holding a name there cannot be read by this
/// build at all — and no import can repair it, because deciding whether two names are one person is
/// not answerable from the file. So a pre-attribution file is refused, its sets are converted by
/// hand while the window is open, and `OLDEST_READABLE` moves past this shape.
///
/// **What this test still pins, and what it does not yet.** It pins that such a file is refused
/// rather than half-read. It does *not* yet pin the refusal being a clear statement about the
/// format version — today it surfaces as a malformed id, which is a parse error where a refusal
/// belongs. Tightening that is the version-floor step, and this assertion is what will show it
/// landed.
#[test]
fn a_file_in_the_shape_0_1_0_was_actually_written_in_is_refused() {
    let text = concat!(
        r#"{"format_version":"0.1.0","scope":"local"}"#,
        "\n",
        r#"{"confidence":0.9,"content":"a memory from before the id spelling changed","#,
        r#""context":"written when the header carried a bare string","#,
        r#""created":{"at":"2026-08-12T17:11:00Z","by":"alice"},"#,
        r#""id":"019fc5fe-6bd8-7352-8406-c0b07244ad71","keywords":["legacy"],"#,
        r#""modified":{"at":"2026-08-12T17:11:00Z","by":"alice"},"origin":"human","#,
        r#""why":"so the oldest readable format is a tested claim"}"#,
        "\n",
    );

    let error = memory_file::read(&Expectation::Exactly(Scope::Local), text.as_bytes())
        .expect_err("a file predating the attribution change cannot be read");

    // **Refused at the version, not at the first record it cannot parse.** The remedy differs: a
    // malformed id says *this file is damaged*, and sends someone looking for the damage. The
    // version refusal says *this build no longer reads that shape*, names both versions, and points
    // at the only thing that helps — re-exporting from a build that still writes it.
    let message = error.to_string();
    assert!(
        message.contains("0.1.0") && message.contains(memory_file::OLDEST_READABLE),
        "the refusal should name the file's version and this build's floor, and got: {error}"
    );
    assert!(
        !message.contains("alice"),
        "reaching a record at all means the gate let the file through: {error}"
    );
}

/// **A file may not declare a set loaded for review.** A file is how a set travels; a review copy is
/// held in one session's memory and adopted by nobody, so a file claiming that name is either a
/// mistake or an attempt to pass a proposal off as a project's record. One of the three boundaries
/// keeping the `eval-` prefix meaningful -- `documents/design/load-modes.md`.
#[test]
fn a_file_declaring_an_evaluation_set_is_refused() {
    let named = SetName::evaluation(&SetName::new("scatterhoard").expect("valid"), "issue-3")
        .expect("builds");
    let header = format!("{{\"format_version\":\"0.4.0\",\"scope\":{{\"shared\":\"{named}\"}}}}\n");

    let refused = memory_file::read(&Expectation::AnyShared, header.as_bytes())
        .expect_err("a file may not name a review copy");

    assert!(
        refused.to_string().contains("eval-scatterhoard-issue-3"),
        "the refusal should name what the file declared: {refused}"
    );
}
