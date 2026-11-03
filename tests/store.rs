// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the per-scope memory store.
//!
//! The store is where several decisions that were only ever prose have to become behaviour: that a
//! stored edge is discoverable from both of its endpoints, that supersession resolves to the tip of
//! its chain, and that a supersession cycle is refused when written but survivable when found.

use std::str::FromStr;

use jiff::Timestamp;
use scatterhoard::memory::{
    Attribution, Author, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId,
    MemoryText, Origin, Scope, SetId, SetName,
};
use scatterhoard::memory_file::{MemorySet, Tombstone};
use scatterhoard::store::{
    CONTROL_MARK_FAMILIES, CONTROL_MARKS, Resolution, Store, StoreError, StoreLayout,
};

fn scope(name: &str) -> Scope {
    Scope::Shared(SetName::new(name).expect("valid set name"))
}

fn a_store() -> Store {
    Store::open_in_memory_and_mint_id(scope("scatterhoard")).expect("opens")
}

fn a_memory(content: &str) -> Memory {
    Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: Timestamp::from_str("2026-08-01T09:00:00Z").expect("valid timestamp"),
        },
    )
}

fn supersedes(target: MemoryId) -> Link {
    Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(target),
    }
}

// ---------------------------------------------------------------------------
// Memories
// ---------------------------------------------------------------------------

/// **Export writes what `stream_records` yields, so the file's order is decided here.** The ids are
/// chosen so their file order is the reverse of their id order, which is the order they are stored
/// in; a stream in id order fails, where three random ids would pass by chance one time in six.
#[test]
fn records_stream_in_file_order_not_id_order() {
    let ids = [
        "01M3378RFH2K8GV764KCRGE00K",
        "01M3378RFH4SXWF6E996X5208E",
        "01M3378RFHXHMFYFF1TK31CZT5",
    ]
    .map(|id| MemoryId::parse(id).expect("a valid id"));
    let mut store = a_store();
    for (n, id) in ids.iter().enumerate() {
        let mut memory = a_memory(&format!("memory {n}"));
        memory.id = *id;
        store.insert(&memory).expect("inserts");
    }

    let mut streamed = Vec::new();
    store
        .stream_records(|record| {
            if ids.contains(&record.id()) {
                streamed.push(record.id());
            }
            Ok(())
        })
        .expect("streams");
    assert_eq!(streamed, vec![ids[2], ids[1], ids[0]]);
}

/// The three fixture ids above, in id order. Their file order is the reverse, and the first one's
/// hash has its top bit set while the other two do not — so a position stored as a signed integer
/// without correction would sort it first rather than last.
const REVERSED_BY_POSITION: [&str; 3] = [
    "01M3378RFH2K8GV764KCRGE00K",
    "01M3378RFH4SXWF6E996X5208E",
    "01M3378RFHXHMFYFF1TK31CZT5",
];

fn reversed_ids() -> [MemoryId; 3] {
    REVERSED_BY_POSITION.map(|id| MemoryId::parse(id).expect("a valid id"))
}

fn streamed_among(store: &Store, ids: &[MemoryId]) -> Vec<(MemoryId, bool)> {
    let mut streamed = Vec::new();
    store
        .stream_records(|record| {
            if ids.contains(&record.id()) {
                let is_tombstone =
                    matches!(record, scatterhoard::memory_file::Record::Tombstone(_));
                streamed.push((record.id(), is_tombstone));
            }
            Ok(())
        })
        .expect("streams");
    streamed
}

/// **Writing a memory for a forgotten id clears its tombstone**, so one id is never both.
/// `an-id-can-hold-both-a-memory-and-a-tombstone`. Its test went through import until 2026-09-28,
/// when a forget became one-way there (`merge-without-a-recorded-ancestor`) and import stopped
/// writing such a memory at all -- but the invariant belongs to the store's writer, which any caller
/// can still hand one, so it is pinned here, where no import policy can route around it.
#[test]
fn a_memory_written_for_a_forgotten_id_clears_the_tombstone() {
    let mut store = a_store();
    let memory = a_memory("alpha, forgotten and then written again");
    store.insert(&memory).expect("inserts");
    store
        .forget(memory.id, MemoryId::generate(), Timestamp::now())
        .expect("forgets");

    store
        .accept_from_file(&memory)
        .expect("a file's record is written as given");

    assert_eq!(
        streamed_among(&store, &[memory.id]),
        vec![(memory.id, false)],
        "the id must stream once, as a memory, or the set exports a file its reader refuses"
    );
}

/// **Memories and tombstones interleave in file order**, because they are two tables and the file
/// is one sequence. A stream that emitted one table and then the other would order by *shape*, and
/// every diff that forgets a memory would move its line.
#[test]
fn memories_and_tombstones_stream_interleaved_in_file_order() {
    let ids = reversed_ids();
    let mut store = a_store();
    for (n, id) in ids.iter().enumerate() {
        let mut memory = a_memory(&format!("memory {n}"));
        memory.id = *id;
        store.insert(&memory).expect("inserts");
    }
    store
        .forget(
            ids[1],
            MemoryId::generate(),
            Timestamp::from_str("2026-09-22T09:00:00Z").expect("valid timestamp"),
        )
        .expect("forgets");

    assert_eq!(
        streamed_among(&store, &ids),
        vec![(ids[2], false), (ids[1], true), (ids[0], false)]
    );
}

/// The previous schema's `memories` and `tombstones`, with no stored position, holding the fixture
/// ids — the store a build before `file-position-column` left behind.
fn a_store_without_positions(path: &std::path::Path) {
    let old = rusqlite::Connection::open(path).expect("creates");
    old.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE memories (
             id TEXT PRIMARY KEY, content TEXT NOT NULL, why TEXT, context TEXT,
             confidence REAL NOT NULL, origin TEXT NOT NULL,
             created_by TEXT NOT NULL, created_at TEXT NOT NULL,
             modified_by TEXT NOT NULL, modified_at TEXT NOT NULL);
         CREATE TABLE tombstones (
             id TEXT PRIMARY KEY, forgotten_by TEXT NOT NULL, forgotten_at TEXT NOT NULL,
             forgot TEXT);",
    )
    .expect("builds the previous schema");
    old.execute(
        "INSERT INTO meta (key, value) VALUES ('scope', 'shared:scatterhoard')",
        [],
    )
    .expect("records the scope");
    let author = MemoryId::generate().to_string();
    let ids = reversed_ids();
    for id in [ids[0], ids[2]] {
        old.execute(
            "INSERT INTO memories (id, content, confidence, origin, created_by, created_at,
                                   modified_by, modified_at)
             VALUES (?1, 'a memory from an older build', 0.8, 'llm', ?2,
                     '2026-08-01T09:00:00Z', ?2, '2026-08-01T09:00:00Z')",
            rusqlite::params![id.to_string(), author],
        )
        .expect("writes a memory");
    }
    old.execute(
        "INSERT INTO tombstones (id, forgotten_by, forgotten_at)
         VALUES (?1, ?2, '2026-08-02T09:00:00Z')",
        rusqlite::params![ids[1].to_string(), author],
    )
    .expect("writes a tombstone");
}

/// **A store written before positions were stored gets one for every row on opening**, memories
/// and tombstones alike, and then streams in file order. Without the backfill every existing row
/// would have no position, and the export would refuse the store rather than write it.
#[test]
fn a_store_written_before_stored_positions_streams_in_file_order() {
    let directory = tempdir();
    let path = directory.path().join("old.sqlite3");
    a_store_without_positions(&path);

    let store = Store::open_at(scope("scatterhoard"), &path).expect("opens an older store");
    let ids = reversed_ids();

    assert_eq!(
        streamed_among(&store, &ids),
        vec![(ids[2], false), (ids[1], true), (ids[0], false)]
    );
}

/// **A stored position that disagrees with its id refuses the export rather than misordering it.**
/// The position is derived state, and derived state can drift — a row written by a build with a
/// different rule, or edited by hand. Checking each record against the one before costs one
/// comparison and turns a silently reordered file into a refusal naming the record.
#[test]
fn a_stored_position_that_disagrees_with_its_id_refuses_the_stream() {
    let directory = tempdir();
    let path = directory.path().join("drifted.sqlite3");
    a_store_without_positions(&path);
    drop(Store::open_at(scope("scatterhoard"), &path).expect("opens and backfills"));

    let ids = reversed_ids();
    rusqlite::Connection::open(&path)
        .expect("reopens")
        .execute(
            "UPDATE memories SET file_position = ?1 WHERE id = ?2",
            rusqlite::params![i64::MAX, ids[0].to_string()],
        )
        // The last record in file order, moved further back: still in order, and still wrong, so
        // only the comparison with the id's own position can catch it.
        .expect("moves the last row further back");

    let store = Store::open_at(scope("scatterhoard"), &path).expect("opens");
    let outcome = store.stream_records(|_| Ok(()));

    assert!(
        matches!(
            outcome,
            Err(scatterhoard::store::StoreError::OutOfFileOrder { .. })
        ),
        "{outcome:?}"
    );
}

/// **One id held as both a memory and a tombstone refuses the stream**, instead of writing that id
/// twice into a file no reader accepts. `write_memory` and `forget` each clear the other's row now,
/// but a store written before that rule (`an-id-can-hold-both-a-memory-and-a-tombstone`) can still
/// hold the pair, and both rows carry the same, correct position — so only the check against the
/// record before catches it.
#[test]
fn an_id_held_as_both_a_memory_and_a_tombstone_refuses_the_stream() {
    let directory = tempdir();
    let path = directory.path().join("both.sqlite3");
    a_store_without_positions(&path);
    let ids = reversed_ids();
    rusqlite::Connection::open(&path)
        .expect("reopens")
        .execute(
            "INSERT INTO tombstones (id, forgotten_by, forgotten_at)
             VALUES (?1, ?2, '2026-08-02T09:00:00Z')",
            rusqlite::params![ids[0].to_string(), MemoryId::generate().to_string()],
        )
        .expect("writes a tombstone beside the memory");

    let store = Store::open_at(scope("scatterhoard"), &path).expect("opens");
    let outcome = store.stream_records(|_| Ok(()));

    assert!(
        matches!(
            outcome,
            Err(scatterhoard::store::StoreError::OutOfFileOrder { id }) if id == ids[0]
        ),
        "{outcome:?}"
    );
}

#[test]
fn a_memory_round_trips_through_the_store() {
    let mut store = a_store();
    let memory = a_memory("storage is partitioned by scope");
    store.insert(&memory).expect("inserts");

    assert_eq!(store.get(memory.id).expect("reads"), Some(memory));
}

#[test]
fn a_fully_populated_memory_round_trips() {
    let mut store = a_store();
    let mut memory = a_memory("the memory itself");
    memory.why = Some(MemoryText::new("the reasoning").unwrap());
    memory.context = Some(MemoryText::new("where it came up").unwrap());
    memory.keywords = vec![
        Keyword::new("storage").unwrap(),
        Keyword::new("scope").unwrap(),
    ];
    memory.links = vec![
        supersedes(MemoryId::generate()),
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
    memory.revise(
        MemoryId::generate(),
        Timestamp::from_str("2026-08-02T11:00:00Z").unwrap(),
    );

    store.insert(&memory).expect("inserts");
    assert_eq!(store.get(memory.id).expect("reads"), Some(memory));
}

#[test]
fn an_unknown_id_reads_as_absent() {
    let store = a_store();
    assert_eq!(store.get(MemoryId::generate()).expect("reads"), None);
}

/// Re-import re-inserts memories that already exist, so insert replaces rather than duplicating —
/// including the memory's links, which must not accumulate across writes.
#[test]
fn inserting_a_revised_memory_replaces_the_stored_one() {
    let mut store = a_store();
    let mut memory = a_memory("first version");
    memory.links = vec![supersedes(MemoryId::generate())];
    store.insert(&memory).expect("inserts");

    memory.content = MemoryText::new("second version").unwrap();
    memory.links.clear();
    store.insert(&memory).expect("re-inserts");

    let stored = store.get(memory.id).expect("reads").expect("still present");
    assert_eq!(stored.content.as_data(), "second version");
    assert!(stored.links.is_empty(), "links accumulated across writes");
}

/// Two cross-set links from one memory, same label, same target id, different sets.
///
/// Ids repeat across sets by construction — a memory copied between sets keeps its id, which is
/// what `MemoryFileError::DuplicateId` describes arising from a merge — while the `links` table is
/// keyed `(source, label, target)` with `target` the id alone. `a-memory-linking-one-id-in-two-sets-keeps-one-link`
/// in `TRACKING.md`: the second row is expected to replace the first, losing a link with nothing
/// reporting it.
#[test]
fn two_links_to_one_id_in_different_sets_both_survive() {
    let mut store = a_store();
    let shared = MemoryId::generate();
    let mut memory = a_memory("one id, two sets");
    memory.links = vec![
        Link {
            label: LinkLabel::RefersTo,
            target: LinkTarget::CrossSet {
                set: SetName::new("left").unwrap(),
                id: shared,
                keywords: Vec::new(),
                set_id: None,
            },
        },
        Link {
            label: LinkLabel::RefersTo,
            target: LinkTarget::CrossSet {
                set: SetName::new("right").unwrap(),
                id: shared,
                keywords: Vec::new(),
                set_id: None,
            },
        },
    ];

    store.insert(&memory).expect("inserts");

    let stored = store.get(memory.id).expect("reads").expect("still present");
    let sets: Vec<&str> = stored
        .links
        .iter()
        .filter_map(|link| match &link.target {
            LinkTarget::CrossSet { set, .. } => Some(set.as_str()),
            LinkTarget::SameSet(_) => None,
        })
        .collect();
    assert_eq!(sets, vec!["left", "right"], "a cross-set link was lost");
}

/// The other half of that key change: two *identical* same-set links must still collapse to one
/// row. SQLite holds NULLs in a key distinct from each other, and a same-set link is a row whose
/// `target_set` is NULL, so the widened key alone would keep both — a silent duplicate in place of
/// the silent loss. The partial unique index `links_same_set` is what this pins.
#[test]
fn two_identical_same_set_links_collapse_to_one() {
    let mut store = a_store();
    let target = MemoryId::generate();
    let mut memory = a_memory("the same edge, written twice");
    memory.links = vec![supersedes(target), supersedes(target)];

    store.insert(&memory).expect("inserts");

    let stored = store.get(memory.id).expect("reads").expect("still present");
    assert_eq!(
        stored.links,
        vec![supersedes(target)],
        "a duplicate same-set link survived the write"
    );
}

/// A store written before the rekey opens, keeps its rows, and comes out with the wider key.
///
/// Built directly here, because a migration can only be tested against a database this code did not
/// write. The old key cannot itself hold two links differing only by set — that is the defect — so
/// what this asserts is that an existing row survives and the shape afterwards is the new one.
#[test]
fn a_store_written_before_the_link_rekey_widens_its_key() {
    let directory = tempdir();
    let path = directory.path().join("old-links.sqlite3");
    let memory = a_memory("a memory whose link predates the rekey");
    let target = MemoryId::generate();

    {
        let old = rusqlite::Connection::open(&path).expect("creates");
        old.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE memories (
                 id TEXT PRIMARY KEY, content TEXT NOT NULL, why TEXT, context TEXT,
                 confidence REAL NOT NULL, origin TEXT NOT NULL,
                 created_by TEXT NOT NULL, created_at TEXT NOT NULL,
                 modified_by TEXT NOT NULL, modified_at TEXT NOT NULL);
             CREATE TABLE links (
                 source          TEXT NOT NULL,
                 label           TEXT NOT NULL,
                 target          TEXT NOT NULL,
                 target_set      TEXT,
                 target_keywords TEXT,
                 target_set_id   TEXT,
                 PRIMARY KEY (source, label, target));",
        )
        .expect("builds the previous schema");
        old.execute(
            "INSERT INTO meta (key, value) VALUES ('scope', 'shared:scatterhoard')",
            [],
        )
        .expect("records the scope");
        old.execute(
            "INSERT INTO memories (id, content, confidence, origin, created_by, created_at,
                                   modified_by, modified_at)
             VALUES (?1, 'a memory whose link predates the rekey', 0.8, 'llm', ?2,
                     '2026-08-01T09:00:00Z', ?2, '2026-08-01T09:00:00Z')",
            // An author is a reference to a `hoard-author` memory, so this column holds an id.
            rusqlite::params![memory.id.to_string(), MemoryId::generate().to_string()],
        )
        .expect("writes a memory");
        old.execute(
            "INSERT INTO links (source, label, target, target_set)
             VALUES (?1, 'refers_to', ?2, 'other-project')",
            rusqlite::params![memory.id.to_string(), target.to_string()],
        )
        .expect("writes a link");
    }

    let store = Store::open_at(scope("scatterhoard"), &path).expect("opens an older store");

    let stored = store.get(memory.id).expect("reads").expect("still present");
    assert_eq!(
        stored.links,
        vec![Link {
            label: LinkLabel::RefersTo,
            target: LinkTarget::CrossSet {
                set: SetName::new("other-project").unwrap(),
                id: target,
                keywords: Vec::new(),
                set_id: None,
            },
        }],
        "the migration lost an authored link"
    );

    // Read the shape through a plain connection: SQLite already answers this, and a test-only
    // accessor on `Store` would widen the API to repeat it.
    let key_columns: i64 = rusqlite::Connection::open(&path)
        .expect("opens the same file")
        .prepare("SELECT COUNT(*) FROM pragma_table_info('links') WHERE pk > 0")
        .expect("prepares")
        .query_row([], |row| row.get(0))
        .expect("counts");
    assert_eq!(key_columns, 4, "the links key was not widened");
}

/// A store holding **both** spellings of one same-set link still opens.
///
/// `compact_own_set_links` narrows the long spelling to the compact one at every open. Once
/// `target_set` is in the key the two can coexist, and narrowing one onto the other violates
/// `links_same_set` — so a plain `UPDATE` fails the open rather than the write, taking the whole
/// store down. Only a store written elsewhere can hold both, which is why this is built by hand.
#[test]
fn a_store_holding_both_spellings_of_one_link_still_opens() {
    let directory = tempdir();
    let path = directory.path().join("both-spellings.sqlite3");
    let memory = a_memory("linked to one target, spelled two ways");
    let target = MemoryId::generate();

    {
        let old = rusqlite::Connection::open(&path).expect("creates");
        old.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE memories (
                 id TEXT PRIMARY KEY, content TEXT NOT NULL, why TEXT, context TEXT,
                 confidence REAL NOT NULL, origin TEXT NOT NULL,
                 created_by TEXT NOT NULL, created_at TEXT NOT NULL,
                 modified_by TEXT NOT NULL, modified_at TEXT NOT NULL);
             CREATE TABLE links (
                 source          TEXT NOT NULL,
                 label           TEXT NOT NULL,
                 target          TEXT NOT NULL,
                 target_set      TEXT,
                 target_keywords TEXT,
                 target_set_id   TEXT,
                 PRIMARY KEY (source, label, target, target_set));",
        )
        .expect("builds a store already carrying the wider key");
        old.execute(
            "INSERT INTO meta (key, value) VALUES ('scope', 'shared:scatterhoard')",
            [],
        )
        .expect("records the scope");
        old.execute(
            "INSERT INTO memories (id, content, confidence, origin, created_by, created_at,
                                   modified_by, modified_at)
             VALUES (?1, 'linked to one target, spelled two ways', 0.8, 'llm', ?2,
                     '2026-08-01T09:00:00Z', ?2, '2026-08-01T09:00:00Z')",
            // An author is a reference to a `hoard-author` memory, so this column holds an id.
            rusqlite::params![memory.id.to_string(), MemoryId::generate().to_string()],
        )
        .expect("writes a memory");
        old.execute(
            "INSERT INTO links (source, label, target, target_set)
             VALUES (?1, 'supersedes', ?2, NULL), (?1, 'supersedes', ?2, 'scatterhoard')",
            rusqlite::params![memory.id.to_string(), target.to_string()],
        )
        .expect("writes both spellings");
    }

    let store = Store::open_at(scope("scatterhoard"), &path).expect("opens without failing");

    let stored = store.get(memory.id).expect("reads").expect("still present");
    assert_eq!(
        stored.links,
        vec![supersedes(target)],
        "the two spellings did not converge on one same-set link"
    );
}

// ---------------------------------------------------------------------------
// Edges are discoverable from both ends
// ---------------------------------------------------------------------------

/// The guarantee from `recall-mechanisms.md`: a stored edge is discoverable from both endpoints.
/// The edge is written only on the source, so this is the half that has to be derived — and it has
/// to be true immediately after the write, not after some later reload.
#[test]
fn an_edge_is_discoverable_from_its_target_as_soon_as_it_is_written() {
    let mut store = a_store();
    let older = a_memory("the earlier conclusion");
    let mut newer = a_memory("the correction");
    newer.links = vec![supersedes(older.id)];

    store.insert(&older).expect("inserts");
    store.insert(&newer).expect("inserts");

    let incoming = store.incoming(older.id).expect("reads");
    assert_eq!(incoming, vec![(newer.id, LinkLabel::Supersedes)]);
    assert!(
        store.incoming(newer.id).expect("reads").is_empty(),
        "nothing points at the newer memory"
    );
}

#[test]
fn removing_an_edge_removes_it_from_the_reverse_direction_too() {
    let mut store = a_store();
    let target = a_memory("the target");
    let mut source = a_memory("the source");
    source.links = vec![supersedes(target.id)];

    store.insert(&target).expect("inserts");
    store.insert(&source).expect("inserts");
    assert_eq!(store.incoming(target.id).expect("reads").len(), 1);

    source.links.clear();
    store.insert(&source).expect("re-inserts");

    assert!(
        store.incoming(target.id).expect("reads").is_empty(),
        "a withdrawn edge must not survive in the reverse direction"
    );
}

// ---------------------------------------------------------------------------
// Supersession
// ---------------------------------------------------------------------------

#[test]
fn an_unsuperseded_memory_resolves_as_current() {
    let mut store = a_store();
    let memory = a_memory("still true");
    store.insert(&memory).expect("inserts");

    assert_eq!(
        store.resolve_supersession(memory.id).expect("resolves"),
        Resolution::Current
    );
}

/// **Two independent supersessions of one memory resolve to two tips**, which is the graph
/// detecting a fork.
///
/// This is what makes content revision answer `git`'s question without any new machinery: a changed
/// claim mints a *new id* with an edge to the old one, so two machines revising the same memory
/// produce a genuine branch in the DAG rather than two versions of one record — and the walk
/// returns both ends rather than choosing. Nothing tested it until 2026-08-26, when the question
/// *would modelling `git`'s three-way merge work with the graph we have* turned on the answer.
#[test]
fn two_independent_supersessions_resolve_to_two_tips() {
    let mut store = a_store();
    let original = a_memory("the conclusion both sides revised");
    let mut ours = a_memory("our revision");
    ours.links = vec![supersedes(original.id)];
    let mut theirs = a_memory("their revision");
    theirs.links = vec![supersedes(original.id)];

    for memory in [&original, &ours, &theirs] {
        store.insert(memory).expect("inserts");
    }

    let mut expected = vec![ours.id, theirs.id];
    expected.sort();
    assert_eq!(
        store.resolve_supersession(original.id).expect("resolves"),
        Resolution::Superseded {
            tips: expected,
            unresolved: false,
            forgotten: false,
        },
        "a fork is reported as both ends, not resolved to one"
    );
}

/// A supersedes B supersedes C: recalling C must yield A, not B — B was itself corrected, and
/// surfacing it would be surfacing a conclusion already known to be wrong.
#[test]
fn a_chain_resolves_to_its_tip() {
    let mut store = a_store();
    let oldest = a_memory("first conclusion");
    let mut middle = a_memory("second conclusion");
    middle.links = vec![supersedes(oldest.id)];
    let mut newest = a_memory("third conclusion");
    newest.links = vec![supersedes(middle.id)];

    for memory in [&oldest, &middle, &newest] {
        store.insert(memory).expect("inserts");
    }

    assert_eq!(
        store.resolve_supersession(oldest.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![newest.id],
            unresolved: false,
            forgotten: false,
        }
    );
    assert_eq!(
        store.resolve_supersession(newest.id).expect("resolves"),
        Resolution::Current
    );
}

/// The convention for splitting a partly-stale memory produces several memories each superseding
/// the original, so resolution is one-to-many rather than a replacement.
#[test]
fn a_split_memory_resolves_to_every_tip() {
    let mut store = a_store();
    let original = a_memory("an observation bundled with its explanation");
    let mut observation = a_memory("the observation alone");
    observation.links = vec![supersedes(original.id)];
    let mut explanation = a_memory("the corrected explanation");
    explanation.links = vec![supersedes(original.id)];

    for memory in [&original, &observation, &explanation] {
        store.insert(memory).expect("inserts");
    }

    let Resolution::Superseded { mut tips, .. } =
        store.resolve_supersession(original.id).expect("resolves")
    else {
        panic!("a split memory is superseded");
    };
    tips.sort();
    let mut expected = vec![observation.id, explanation.id];
    expected.sort();

    assert_eq!(tips, expected);
}

/// An authored `SUPERSEDED_BY` pointing into a set this store does not hold: the memory is known
/// to be superseded, but the replacement cannot be produced. That has to be reported rather than
/// silently treated as current.
#[test]
fn a_supersession_leaving_this_store_is_reported_as_unresolved() {
    let mut store = a_store();
    let mut memory = a_memory("replaced by something elsewhere");
    memory.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::CrossSet {
            set: SetName::new("other-project").unwrap(),
            id: MemoryId::generate(),
            keywords: Vec::new(),
            set_id: None,
        },
    }];
    store.insert(&memory).expect("inserts");

    assert_eq!(
        store.resolve_supersession(memory.id).expect("resolves"),
        Resolution::Superseded {
            tips: Vec::new(),
            unresolved: true,
            forgotten: false,
        }
    );
}

/// The same concession written **without** its set name — the malformation
/// `documents/design/cross-set-references.md` calls the commonest one. The reader's situation is
/// identical to the case above: the memory is known to be superseded and the replacement cannot be
/// produced. Reporting a tip no store holds hides the memory from every search path instead, which
/// is `a-superseded-by-naming-an-absent-id-hides-its-memory` in `TRACKING.md`.
#[test]
fn a_concession_to_an_id_nobody_holds_is_reported_as_unresolved() {
    let mut store = a_store();
    let mut memory = a_memory("yielding to an id that was never stored");
    memory.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::SameSet(MemoryId::generate()),
    }];

    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories: vec![memory.clone()],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("a hand-edited file still imports");

    assert_eq!(
        store.resolve_supersession(memory.id).expect("resolves"),
        Resolution::Superseded {
            tips: Vec::new(),
            unresolved: true,
            forgotten: false,
        }
    );
}

/// A concession whose replacement was later forgotten. The record is still here, so the chain is
/// walkable, but the tip cannot be handed to a reader — and a tip that cannot be fetched drops the
/// hit exactly as an absent one does. Reporting it is what keeps the memory findable.
///
/// **Reported as forgotten, not as unresolved**, since 2026-09-30: the two send a reader opposite
/// ways, and only this one is answerable from the store in hand.
/// `a-forgotten-replacement-reads-as-superseded-beyond-this-set`.
#[test]
fn a_chain_ending_on_a_forgotten_memory_is_reported_as_forgotten() {
    let mut store = a_store();
    let replacement = a_memory("the correction, later forgotten");
    let mut memory = a_memory("yielding to something since removed");
    memory.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::SameSet(replacement.id),
    }];

    store.insert(&replacement).expect("inserts");
    store.insert(&memory).expect("inserts");
    store
        .forget(
            replacement.id,
            MemoryId::generate(),
            Timestamp::from_str("2026-08-25T09:00:00Z").expect("valid timestamp"),
        )
        .expect("forgets");

    assert_eq!(
        store.resolve_supersession(memory.id).expect("resolves"),
        Resolution::Superseded {
            tips: Vec::new(),
            unresolved: false,
            forgotten: true,
        }
    );
}

/// A chain is followed *through* an id this store holds no record of, which is why the check for a
/// fetchable replacement sits at the end of the chain rather than at each step. A set that lags the
/// repository it came from is exactly this: the intermediate memory has not arrived, the newest one
/// has, and reporting the reader as stranded when the replacement is right here would be wrong.
#[test]
fn a_chain_passing_through_an_id_nobody_holds_still_resolves_to_its_tip() {
    let mut store = a_store();
    let absent = MemoryId::generate();
    let mut oldest = a_memory("the first conclusion");
    oldest.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::SameSet(absent),
    }];
    let mut newest = a_memory("the conclusion that stands");
    newest.links = vec![supersedes(absent)];

    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories: vec![oldest.clone(), newest.clone()],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("a lagging file still imports");

    assert_eq!(
        store.resolve_supersession(oldest.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![newest.id],
            unresolved: false,
            forgotten: false,
        }
    );
}

/// And the chain must still be walked *through* a tombstone, which is the case
/// `documents/design/cross-set-references.md` settles the other way: a supersession chain whose
/// middle was forgotten still holds a good reference, so it resolves to the live tip beyond it.
#[test]
fn a_chain_whose_middle_was_forgotten_still_resolves_to_its_tip() {
    let mut store = a_store();
    let middle = a_memory("the intermediate conclusion, later forgotten");
    let mut oldest = a_memory("the first conclusion");
    oldest.links = vec![Link {
        label: LinkLabel::SupersededBy,
        target: LinkTarget::SameSet(middle.id),
    }];
    let mut newest = a_memory("the conclusion that stands");
    newest.links = vec![supersedes(middle.id)];

    for memory in [&middle, &oldest, &newest] {
        store.insert(memory).expect("inserts");
    }
    store
        .forget(
            middle.id,
            MemoryId::generate(),
            Timestamp::from_str("2026-08-25T09:00:00Z").expect("valid timestamp"),
        )
        .expect("forgets");

    assert_eq!(
        store.resolve_supersession(oldest.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![newest.id],
            unresolved: false,
            forgotten: false,
        }
    );
}

// ---------------------------------------------------------------------------
// Cycles
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// One live charter per set
// ---------------------------------------------------------------------------

/// A memory marked as a set's charter, optionally superseding others.
fn a_charter(content: &str, supersedes_these: &[MemoryId]) -> Memory {
    let mut memory = a_memory(content);
    memory.keywords = vec![Keyword::new("hoard-scope-topics").unwrap()];
    memory.links = supersedes_these.iter().map(|id| supersedes(*id)).collect();
    memory
}

/// **What the guard exists to refuse.** Memories *about* charters were written carrying the charter
/// mark three times in six days, each time in a set that already had one, and each was delivered to
/// every session as though it described the set. `one-live-charter-per-set`.
#[test]
fn a_second_live_charter_is_refused() {
    let mut store = a_store();
    store
        .insert(&a_charter("what this set is for", &[]))
        .expect("a set's first charter is accepted");

    let refused = store
        .insert(&a_charter("a memory about charters, mismarked", &[]))
        .expect_err("a second live charter must be refused");
    let message = refused.to_string();
    assert!(
        message.contains("supersede") && message.contains("about-hoard-scope-topics"),
        "the refusal must name both ways out: {message}"
    );
}

#[test]
fn a_charter_that_supersedes_the_live_one_is_accepted() {
    let mut store = a_store();
    let first = a_charter("what this set was for", &[]);
    store.insert(&first).expect("inserts");

    store
        .insert(&a_charter("what this set is for now", &[first.id]))
        .expect("replacing the charter by superseding it is how a charter changes");
}

/// An amendment rewrites a memory under its own id, so the live charter rewritten in place is not a
/// second charter.
#[test]
fn rewriting_the_live_charter_in_place_is_accepted() {
    let mut store = a_store();
    let mut charter = a_charter("what this set is for", &[]);
    store.insert(&charter).expect("inserts");

    charter.content = MemoryText::new("what this set is for, better worded").unwrap();
    store
        .insert(&charter)
        .expect("the charter is not a second charter beside itself");
}

/// The amendment that turns an ordinary memory into a charter is a second charter all the same.
#[test]
fn marking_an_existing_memory_as_a_charter_is_refused_while_another_is_live() {
    let mut store = a_store();
    store
        .insert(&a_charter("what this set is for", &[]))
        .expect("inserts");
    let mut memory = a_memory("a design note about how charters are matched");
    store.insert(&memory).expect("inserts");

    memory.keywords = vec![Keyword::new("hoard-scope-topics").unwrap()];
    assert!(
        store.insert(&memory).is_err(),
        "adding the charter mark by amendment makes a second live charter"
    );
}

/// The form the refusal points at: a memory *about* charters carries the `about-` keyword and is not
/// a charter.
#[test]
fn a_memory_about_charters_is_accepted_beside_the_charter() {
    let mut store = a_store();
    store
        .insert(&a_charter("what this set is for", &[]))
        .expect("inserts");
    let mut about = a_memory("how charters are matched during routing");
    about.keywords = vec![Keyword::new("about-hoard-scope-topics").unwrap()];

    store
        .insert(&about)
        .expect("a memory about charters is not a charter");
}

/// **Never at import.** A file from another version, or a merge that legitimately disagrees, can carry
/// two live charters, and refusing one would abort the file partway -- the same reason the cycle
/// refusal stays off the reconciling path.
#[test]
fn two_live_charters_arriving_by_import_are_accepted() {
    let mut store = a_store();
    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories: vec![a_charter("ours", &[]), a_charter("theirs", &[])],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("a file carrying two live charters still imports");
}

/// Once two are live, a replacement has to supersede both: superseding one leaves the other standing
/// beside it.
#[test]
fn a_charter_superseding_only_one_of_two_live_charters_is_refused() {
    let mut store = a_store();
    let ours = a_charter("ours", &[]);
    let theirs = a_charter("theirs", &[]);
    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories: vec![ours.clone(), theirs.clone()],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("imports");

    assert!(
        store.insert(&a_charter("one", &[ours.id])).is_err(),
        "the charter left unsuperseded is still live"
    );
    store
        .insert(&a_charter("both", &[ours.id, theirs.id]))
        .expect("superseding both leaves one live charter");
}

/// **Carrying a charter forward is not authoring one.** Absorbing a set can leave two live charters,
/// and then supersedes this set's memories that linked into the absorbed set -- the charter among
/// them if it did. That replacement stands beside the absorbed charter, and refusing it would stop the
/// absorb with the records already merged.
#[test]
fn a_charter_carried_forward_beside_another_live_charter_is_accepted() {
    let mut store = a_store();
    let ours = a_charter("ours", &[]);
    let theirs = a_charter("theirs", &[]);
    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories: vec![ours.clone(), theirs],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("imports");

    let carried = a_charter("ours, with its links made local", &[ours.id]);
    assert!(
        store.insert(&carried).is_err(),
        "authored, this would be a second charter"
    );
    store
        .carry_forward(&carried)
        .expect("carried forward by an operation, it is not");
}

// ---------------------------------------------------------------------------
// Reserved keywords
// ---------------------------------------------------------------------------

/// A memory carrying the given keywords.
fn a_memory_keyed(content: &str, keywords: &[&str]) -> Memory {
    let mut memory = a_memory(content);
    memory.keywords = keywords
        .iter()
        .map(|keyword| Keyword::new(*keyword).expect("valid keyword"))
        .collect();
    memory
}

/// A store holding memories brought in the way another version's file would bring them, past every
/// authoring check.
fn imported(memories: Vec<Memory>) -> Store {
    let mut store = a_store();
    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories,
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("imports");
    store
}

/// **What the check exists to refuse**: a `hoard-` term nobody has used in the set before and this
/// version does not know, which is a typo or an invented mark. `reserved-keyword-block-list`.
#[test]
fn a_mistyped_control_mark_is_refused() {
    let mut store = a_store();
    assert!(
        store
            .insert(&a_memory_keyed("a watch, mistyped", &["hoard-signposts"]))
            .is_err(),
        "a hoard- term neither known nor held by the set must be refused"
    );
}

#[test]
fn an_invented_control_mark_is_refused() {
    let mut store = a_store();
    assert!(
        store
            .insert(&a_memory_keyed("a mark of my own", &["hoard-important"]))
            .is_err(),
        "a hoard- term neither known nor held by the set must be refused"
    );
}

/// The refusal is read by a session deciding what to write instead, so it names every mark it would
/// have accepted and both ways out: the `about-` form for a memory about a mark, and a newer
/// Scatterhoard for a mark this version predates.
#[test]
fn the_refusal_names_the_known_marks_and_the_ways_out() {
    let mut store = a_store();
    let message = store
        .insert(&a_memory_keyed("a mark of my own", &["hoard-important"]))
        .expect_err("refused")
        .to_string();
    for mark in CONTROL_MARKS.iter().chain(CONTROL_MARK_FAMILIES) {
        assert!(
            message.contains(mark),
            "the refusal does not name `{mark}`: {message}"
        );
    }
    assert!(
        message.contains("about-hoard-important"),
        "the refusal should spell the about- form of the term it refused: {message}"
    );
    assert!(
        message.contains("newer"),
        "the refusal should say the mark may come from a newer Scatterhoard: {message}"
    );
}

/// **Known, which is not the same as accepted bare**: a mark can carry rules of its own -- a sentence
/// set needs a language and a triplet -- so what this pins is that none is refused as unknown.
#[test]
fn every_control_mark_this_version_knows_is_accepted() {
    for mark in CONTROL_MARKS {
        let mut store = a_store();
        if let Err(error @ StoreError::UnknownControlMark { .. }) =
            store.insert(&a_memory_keyed("marked", &[*mark]))
        {
            panic!("`{mark}` is a known mark: {error}");
        }
    }
}

/// A set's model-fit material: the hub, its sentence sets and the models it recommends. Named here
/// as well as through [`CONTROL_MARKS`], because what this pins is that these three exist, which a
/// loop over the list would pass without. `recall-is-latin-script-only`.
#[test]
fn the_model_fit_marks_are_accepted() {
    for mark in ["hoard-fit-hub", "hoard-fit-sentences", "hoard-fit-model"] {
        let mut store = a_store();
        if let Err(error @ StoreError::UnknownControlMark { .. }) =
            store.insert(&a_memory_keyed("model fit", &[mark]))
        {
            panic!("`{mark}` is a known mark: {error}");
        }
    }
}

/// **Three marks, not a family**: a mistyped `hoard-fit-` term is refused like any other, which a
/// prefix would have let through.
#[test]
fn a_hoard_fit_term_that_is_not_one_of_the_three_is_refused() {
    let mut store = a_store();
    assert!(
        store
            .insert(&a_memory_keyed("model fit", &["hoard-fit-sentence"]))
            .is_err()
    );
}

/// **A family is a prefix, not a list.** The embedding model is the user's choice, so the languages a
/// set can be written in are too, and no list in this version could name them. A project's own marks
/// are its own business by the same argument.
#[test]
fn any_member_of_a_mark_family_is_accepted() {
    for keyword in [
        "hoard-lang-tlh",
        "hoard-lang-x-private-use",
        "hoard-proj-backlog-entry",
    ] {
        let mut store = a_store();
        store
            .insert(&a_memory_keyed("in a family", &[keyword]))
            .unwrap_or_else(|error| panic!("`{keyword}` belongs to a family: {error}"));
    }
}

/// A bare prefix names no language and no project mark -- `Store::declared_languages` already reads
/// `hoard-lang-` alone as declaring nothing.
#[test]
fn a_family_prefix_on_its_own_is_refused() {
    for prefix in CONTROL_MARK_FAMILIES {
        let mut store = a_store();
        assert!(
            store
                .insert(&a_memory_keyed("a bare prefix", &[*prefix]))
                .is_err(),
            "`{prefix}` alone is not a member of its family"
        );
    }
}

/// Keywords match caselessly, so a mark spelled in capitals is the mark.
#[test]
fn a_control_mark_is_recognised_whatever_its_case() {
    let mut store = a_store();
    store
        .insert(&a_memory_keyed(
            "marked",
            &["HOARD-Signpost", "Hoard-Lang-EN"],
        ))
        .expect("a known mark in another case is still known");
}

/// The form the refusal points at is outside the reserved space, whatever mark it is about.
#[test]
fn a_memory_about_a_mark_is_accepted() {
    let mut store = a_store();
    store
        .insert(&a_memory_keyed(
            "how marks are chosen",
            &["about-hoard-watch", "about-hoard-important"],
        ))
        .expect("an about- keyword is not a control mark");
}

/// **Never at import.** A file from another version can carry a mark this one has never heard of, or
/// has retired, and refusing it would abort the file partway.
#[test]
fn an_unknown_mark_arriving_by_import_is_accepted() {
    imported(vec![a_memory_keyed(
        "from a newer version",
        &["hoard-scope-keywords"],
    )]);
}

/// **The set vouches for its own terms.** A mark a newer version introduced reaches an older one by
/// import, and the older one has to be able to write it again: otherwise keyword structure could only
/// change once every copy had upgraded. Decided 2026-09-14 by the human partner.
#[test]
fn a_mark_the_set_already_holds_is_accepted_on_a_new_memory() {
    let mut store = imported(vec![a_memory_keyed(
        "from a newer version",
        &["hoard-scope-keywords"],
    )]);
    store
        .insert(&a_memory_keyed("written here", &["hoard-scope-keywords"]))
        .expect("a term the set already holds is known to it");
}

/// **A retired mark does not strand the memories carrying it.** `hoard-watch` was replaced by the
/// three watch-list marks, and live memories still carry it: amending one restates the term rather
/// than introducing it.
#[test]
fn amending_a_memory_that_carries_a_retired_mark_is_accepted() {
    let mut memory = a_memory_keyed("a design note", &["hoard-watch"]);
    let mut store = imported(vec![memory.clone()]);

    memory.content = MemoryText::new("a design note, corrected").unwrap();
    store
        .insert(&memory)
        .expect("restating a term the memory already carries is not introducing it");
}

#[test]
fn superseding_a_memory_while_restating_a_retired_mark_is_accepted() {
    let old = a_memory_keyed("a design note", &["hoard-watch"]);
    let mut store = imported(vec![old.clone()]);

    let mut replacement = a_memory_keyed("the design note, superseded", &["hoard-watch"]);
    replacement.links = vec![supersedes(old.id)];
    store
        .insert(&replacement)
        .expect("a replacement restating its original's terms is not introducing them");
}

/// Superseded memories count: a retired term is still part of the set's history, and the memories
/// carrying it are still read.
#[test]
fn a_mark_held_only_by_a_superseded_memory_is_still_known() {
    let old = a_memory_keyed("old", &["hoard-watch"]);
    let mut replacement = a_memory_keyed("new", &["watch-convention"]);
    replacement.links = vec![supersedes(old.id)];
    let mut store = imported(vec![old, replacement]);

    store
        .insert(&a_memory_keyed("later", &["hoard-watch"]))
        .expect("a term in the set's history is held by the set");
}

/// An amendment is an authored write like any other: restating is exempt, introducing is not.
#[test]
fn adding_an_unknown_mark_by_amendment_is_refused() {
    let mut store = a_store();
    let mut memory = a_memory_keyed("a note", &["design"]);
    store.insert(&memory).expect("inserts");

    memory
        .keywords
        .push(Keyword::new("hoard-important").unwrap());
    assert!(
        store.insert(&memory).is_err(),
        "an amendment introducing an unknown hoard- term must be refused"
    );
}

#[test]
fn a_supersession_cycle_is_refused_at_write() {
    let mut store = a_store();
    let first = a_memory("one");
    let mut second = a_memory("two");
    second.links = vec![supersedes(first.id)];

    store.insert(&first).expect("inserts");
    store.insert(&second).expect("inserts");

    let mut closes_the_loop = first.clone();
    closes_the_loop.links = vec![supersedes(second.id)];

    assert!(
        store.insert(&closes_the_loop).is_err(),
        "a write that closes a supersession cycle must be refused"
    );
}

/// **A fork that is later healed resolves to the one memory that healed it.** Two people advance the
/// same record, which the design expects, and a third memory supersedes both. The shared ancestor is
/// reached twice on the way to the tip, and reaching a memory twice is a diamond, not a loop.
///
/// The ancestor is the case that matters most: it is the id every older link still points at, so a
/// false cycle here hands back withdrawn text and diagnoses a merge that went wrong when nothing did.
/// `a-healed-fork-reports-its-ancestor-as-a-cycle`.
#[test]
fn a_healed_fork_resolves_to_the_memory_that_healed_it() {
    let mut store = a_store();
    let original = a_memory("the conclusion both sides revised");
    let mut ours = a_memory("our revision");
    ours.links = vec![supersedes(original.id)];
    let mut theirs = a_memory("their revision");
    theirs.links = vec![supersedes(original.id)];
    let mut healed = a_memory("the revision reconciling both");
    healed.links = vec![supersedes(ours.id), supersedes(theirs.id)];

    for memory in [&original, &ours, &theirs, &healed] {
        store
            .insert(memory)
            .expect("healing a fork is not a cycle, so the write is accepted");
    }

    let healed_tip = Resolution::Superseded {
        tips: vec![healed.id],
        unresolved: false,
        forgotten: false,
    };
    assert_eq!(
        store.resolve_supersession(original.id).expect("resolves"),
        healed_tip,
        "the shared ancestor is reached twice, which is a diamond and not a loop"
    );
    assert_eq!(
        store.resolve_supersession(ours.id).expect("resolves"),
        healed_tip
    );
}

/// **Healing a fork must not cost loop detection.** The obvious repair -- stop treating a revisit as
/// a cycle -- would also stop reporting a genuine loop, and a loop past a diamond is exactly where
/// that repair and the right one disagree. `a-supersession-cycle-reads-as-current` is what losing it
/// looked like.
#[test]
fn a_loop_past_a_healed_fork_is_still_a_cycle() {
    let mut store = a_store();
    let original = a_memory("the conclusion both sides revised");
    let mut ours = a_memory("our revision");
    ours.links = vec![supersedes(original.id)];
    let mut theirs = a_memory("their revision");
    theirs.links = vec![supersedes(original.id)];
    let mut healed = a_memory("the revision reconciling both");
    let mut looped = a_memory("a hand-edited merge that loops");
    healed.links = vec![
        supersedes(ours.id),
        supersedes(theirs.id),
        supersedes(looped.id),
    ];
    looped.links = vec![supersedes(healed.id)];

    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories: vec![original.clone(), ours, theirs, healed, looped],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("a corrupt file still imports");

    assert_eq!(
        store.resolve_supersession(original.id).expect("resolves"),
        Resolution::Cycle
    );
}

/// Refusing at write only covers writes this store made. A hand-edit during a merge conflict can
/// put a cycle in a file, so importing must survive it and resolution must report it rather than
/// looping — recall has to defend itself against a store it did not fully author.
#[test]
fn a_cycle_arriving_by_import_is_reported_rather_than_looping() {
    let mut store = a_store();
    let mut first = a_memory("one");
    let mut second = a_memory("two");
    first.links = vec![supersedes(second.id)];
    second.links = vec![supersedes(first.id)];

    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories: vec![first.clone(), second],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("a corrupt file still imports");

    assert_eq!(
        store.resolve_supersession(first.id).expect("resolves"),
        Resolution::Cycle
    );
}

// ---------------------------------------------------------------------------
// Scope
// ---------------------------------------------------------------------------

/// The same guard as the memory file's header, one layer down: a store knows its own scope, and
/// opening it as something else is a refusal rather than a silent re-target.
#[test]
fn opening_a_store_under_a_different_scope_is_refused() {
    let directory = tempdir();
    Store::open(scope("scatterhoard"), directory.path()).expect("creates");

    assert!(
        Store::open(scope("other-project"), directory.path()).is_ok(),
        "a different set is a different store, not a conflict"
    );
    assert!(
        Store::open_at(
            Scope::Local,
            &Store::path_for(&scope("scatterhoard"), directory.path())
        )
        .is_err(),
        "opening a project's store as the local one must be refused"
    );
}

// ---------------------------------------------------------------------------
// Where each scope's store file lives
// ---------------------------------------------------------------------------

/// The default, and the property that makes the split safe to adopt: name only a home and nothing
/// about today's layout changes.
#[test]
fn without_a_common_directory_every_store_lives_in_the_home() {
    let layout = StoreLayout::new(std::path::PathBuf::from("/homes/one"), None);

    assert_eq!(
        layout.path_for(&Scope::Local),
        layout.home().join("local.sqlite3")
    );
    assert_eq!(
        layout.path_for(&scope("scatterhoard")),
        layout.home().join("shared-scatterhoard.sqlite3")
    );
}

/// `local` is always machine-bound, so it follows the common directory without anyone deciding —
/// its scope *type* carries the answer. A shared set's does not, which is what discovery below is
/// for: `path_for` says where a *new* shared set is created, and that is always the home.
#[test]
fn a_common_directory_takes_the_local_store_and_leaves_shared_sets_alone() {
    let layout = StoreLayout::new(
        std::path::PathBuf::from("/homes/one"),
        Some(std::path::PathBuf::from("/personal")),
    );

    assert_eq!(
        layout.path_for(&Scope::Local),
        std::path::Path::new("/personal/local.sqlite3"),
        "the local store follows the common directory"
    );
    assert_eq!(
        layout.path_for(&scope("scatterhoard")),
        std::path::Path::new("/homes/one/shared-scatterhoard.sqlite3"),
        "a shared set created here belongs to the home"
    );
}

/// Two homes, one personal store — the configuration the whole variable exists for. Stated as a test
/// because it is the claim, not an illustration of one.
#[test]
fn two_homes_sharing_a_common_directory_resolve_to_one_local_store() {
    let personal = std::path::PathBuf::from("/personal");
    let first = StoreLayout::new(
        std::path::PathBuf::from("/checkouts/review"),
        Some(personal.clone()),
    );
    let second = StoreLayout::new(
        std::path::PathBuf::from("/checkouts/development"),
        Some(personal),
    );

    assert_eq!(
        first.path_for(&Scope::Local),
        second.path_for(&Scope::Local),
        "two homes sharing a local directory must reach the same file"
    );
    assert_ne!(
        first.path_for(&scope("scatterhoard")),
        second.path_for(&scope("scatterhoard")),
        "and must still keep their shared sets apart"
    );
}

/// No shared set reaches the local store's file, wherever the local store has been moved to.
///
/// *These two tests read `scope("local")` until 2026-08-21, when `local` became a reserved set name
/// (`target-set-local-names-a-different-set`). What they were protecting is path derivation being
/// injective across scopes, which is worth keeping on its own terms and does not depend on that one
/// word — so they now use the near-misses that are still legal. The word itself is pinned as
/// unconstructible in `tests/memory_model.rs`, which is a stronger guarantee than these were.*
#[test]
fn no_shared_set_collides_with_the_local_store_across_directories() {
    let layout = StoreLayout::new(
        std::path::PathBuf::from("/homes/one"),
        Some(std::path::PathBuf::from("/personal")),
    );

    for name in ["locale", "local-notes", "scatterhoard"] {
        assert_ne!(
            layout.path_for(&scope(name)),
            layout.path_for(&Scope::Local),
            "the shared set {name} reached the local store's file"
        );
    }
}

// ---------------------------------------------------------------------------
// Discovering which stores exist, and where
// ---------------------------------------------------------------------------

fn write_store(directory: &std::path::Path, scope: &Scope) -> std::path::PathBuf {
    let path = Store::path_for(scope, directory);
    Store::open_at(scope.clone(), &path).expect("creates a store");
    path
}

fn found(discovery: &scatterhoard::store::Discovery, wanted: &Scope) -> Option<std::path::PathBuf> {
    discovery
        .stores
        .iter()
        .find(|(scope, _)| scope == wanted)
        .map(|(_, path)| path.clone())
}

/// A shared set in the common directory is machine-bound: every checkout on this machine reaches the
/// same store, which is the whole point — `coding-lessons` is not about any one checkout.
#[test]
fn a_shared_set_in_the_common_directory_is_found() {
    let home = tempdir();
    let common = tempdir();
    write_store(common.path(), &scope("coding-lessons"));

    let layout = StoreLayout::new(home.path().to_path_buf(), Some(common.path().to_path_buf()));
    let discovery = layout.discover();

    assert_eq!(
        found(&discovery, &scope("coding-lessons")),
        Some(Store::path_for(&scope("coding-lessons"), common.path()))
    );
    assert!(discovery.shadowed.is_empty());
}

/// And a shared set in the home stays checkout-bound. Both directories are scanned, and where a set
/// sits is what decides which kind it is.
#[test]
fn a_shared_set_in_the_home_is_still_found() {
    let home = tempdir();
    let common = tempdir();
    write_store(home.path(), &scope("scatterhoard"));
    write_store(common.path(), &scope("coding-lessons"));

    let layout = StoreLayout::new(home.path().to_path_buf(), Some(common.path().to_path_buf()));
    let discovery = layout.discover();

    assert_eq!(
        found(&discovery, &scope("scatterhoard")),
        Some(Store::path_for(&scope("scatterhoard"), home.path()))
    );
    assert_eq!(
        discovery.stores.len(),
        3,
        "and `local`, {:?}",
        discovery.stores
    );
}

/// **The common copy wins.** The likely way to reach this state is a move that left the original
/// behind, or a `create` that made a fresh empty store in the home for a set already living in the
/// common directory — and in both, the home copy is the stale one. Letting it win would put an empty
/// store in front of real memories, which is the failure
/// `moving-the-local-store-leaves-the-old-one-unmentioned` already describes once.
#[test]
fn a_set_in_both_directories_resolves_to_the_common_copy() {
    let home = tempdir();
    let common = tempdir();
    write_store(home.path(), &scope("coding-lessons"));
    write_store(common.path(), &scope("coding-lessons"));

    let layout = StoreLayout::new(home.path().to_path_buf(), Some(common.path().to_path_buf()));
    let discovery = layout.discover();

    assert_eq!(
        found(&discovery, &scope("coding-lessons")),
        Some(Store::path_for(&scope("coding-lessons"), common.path())),
        "the machine-bound copy is the one that should be live"
    );
    assert_eq!(
        discovery.shadowed,
        vec![(
            scope("coding-lessons"),
            Store::path_for(&scope("coding-lessons"), home.path())
        )],
        "and the losing copy must be reported rather than silently ignored"
    );
}

/// The default: one directory, so nothing can shadow anything and no set is discovered twice.
#[test]
fn with_no_common_directory_nothing_is_shadowed() {
    let home = tempdir();
    write_store(home.path(), &scope("scatterhoard"));

    let discovery = StoreLayout::new(home.path().to_path_buf(), None).discover();

    assert_eq!(discovery.stores.len(), 2, "the set and `local`");
    assert!(discovery.shadowed.is_empty());
}

/// **Declaring the common directory to be the home is reported.** It is not harmful — it is the
/// default spelled out — but someone who sets it has stated an intention it does not carry out, and
/// a configuration that looks applied while doing nothing is the same shape as a
/// `SCATTERHOARD_AUTHOR` that is silently ignored.
#[test]
fn naming_the_home_as_the_common_directory_is_reported_as_doing_nothing() {
    let home = tempdir();

    let declared = StoreLayout::new(home.path().to_path_buf(), Some(home.path().to_path_buf()));
    assert!(declared.common_is_the_home());

    let defaulted = StoreLayout::new(home.path().to_path_buf(), None);
    assert!(
        !defaulted.common_is_the_home(),
        "the ordinary case must not be reported: nobody asked for anything"
    );
}

/// The same thing spelled differently. A path that merely *reaches* the home is the home, and
/// comparing the strings would miss it — which is the defect
/// `target-identity-is-an-unnormalised-path-string` in another costume.
#[test]
fn a_common_directory_that_only_looks_different_from_the_home_is_still_the_home() {
    let home = tempdir();
    let roundabout = home.path().join("..").join(
        home.path()
            .file_name()
            .expect("the scratch directory is named"),
    );

    let layout = StoreLayout::new(home.path().to_path_buf(), Some(roundabout));

    assert!(
        layout.common_is_the_home(),
        "two spellings of one directory are one directory"
    );
    assert!(
        layout.discover().shadowed.is_empty(),
        "and must not make every set shadow itself"
    );
}

/// A shared set's store must never become the local store, whatever it is called.
#[test]
fn no_shared_set_collides_with_the_local_store() {
    let directory = tempdir();
    let local = Store::path_for(&Scope::Local, directory.path());

    for name in ["locale", "local-notes", "scatterhoard"] {
        assert_ne!(
            Store::path_for(&scope(name), directory.path()),
            local,
            "the shared set {name} collided with the local store"
        );
    }
}

#[test]
fn a_store_remembers_its_scope_across_reopening() {
    let directory = tempdir();
    let memory = a_memory("persisted");
    {
        let mut store = Store::open(scope("scatterhoard"), directory.path()).expect("creates");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.insert(&memory).expect("inserts");
    }

    let store = Store::open(scope("scatterhoard"), directory.path()).expect("reopens");
    assert_eq!(store.scope(), &scope("scatterhoard"));
    assert!(store.get(memory.id).expect("reads").is_some());
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// The corpus is a real 39-memory set with 29 edges; importing it is the first end-to-end exercise
/// of the store against material it did not generate.
#[test]
fn the_labelled_corpus_imports_and_its_edges_are_traversable() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/corpus/design-memories.jsonl");
    let file = std::fs::File::open(path).expect("the corpus file should exist");
    let set =
        scatterhoard::memory_file::read(&scatterhoard::memory_file::Expectation::AnyShared, file)
            .expect("the corpus reads");

    let mut store = a_store();
    store.import(&set).expect("imports");

    let edges: usize = set.memories.iter().map(|m| m.links.len()).sum();
    let reversed: usize = set
        .memories
        .iter()
        .map(|m| store.incoming(m.id).expect("reads").len())
        .sum();
    assert_eq!(
        reversed, edges,
        "every stored edge should be discoverable from its target"
    );

    let superseded = set
        .memories
        .iter()
        .filter(|m| {
            matches!(
                store.resolve_supersession(m.id).expect("resolves"),
                Resolution::Superseded { .. }
            )
        })
        .count();
    assert_eq!(
        superseded, 2,
        "the corpus records two corrections, so two memories are superseded"
    );
}

/// Import replaces the set rather than merging into it: a memory dropped from the file is gone
/// after import, not left behind as a memory nothing references.
#[test]
fn import_replaces_the_set_rather_than_accumulating() {
    let mut store = a_store();
    let dropped = a_memory("present in the first import only");
    let kept = a_memory("present in both");

    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories: vec![dropped.clone(), kept.clone()],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("imports");

    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories: vec![kept.clone()],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("re-imports");

    assert!(store.get(kept.id).expect("reads").is_some());
    assert_eq!(
        store.get(dropped.id).expect("reads"),
        None,
        "a memory absent from the imported file should be gone"
    );
}

#[test]
fn importing_a_set_from_a_different_scope_is_refused() {
    let mut store = a_store();

    assert!(
        store
            .import(&MemorySet {
                scope: scope("other-project"),
                memories: vec![a_memory("from elsewhere")],
                tombstones: Vec::new(),
                set_id: None,
            })
            .is_err(),
        "a store must not absorb another set's memories"
    );
}

// ---------------------------------------------------------------------------

/// Minimal scratch directory, so the tests do not depend on a dev-dependency for one use.
fn tempdir() -> TempDir {
    let mut path = std::env::temp_dir();
    path.push(format!("scatterhoard-test-{}", MemoryId::generate()));
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Opening a store as the wrong scope must say which scope it *actually* holds — and the recorded
/// scope is encoded and decoded by two functions that each spelled the prefix by hand. Renaming the
/// `Project` variant to `Shared` on 2026-08-04 changed the writer and not the reader, and the whole
/// suite stayed green.
///
/// The blast radius is smaller than it first looks, because `bind_scope` compares stored string
/// against stored string: a store's scope comes from the caller, never from the database, which is
/// the same "guard, never a source" rule the memory file header follows. So the decoder is reached
/// only on the error path — and a shared store opened under the wrong scope would have reported
/// itself as `Local`, naming the wrong set in the one message whose whole job is to say which set
/// was found.
///
/// A first attempt at this test asserted that a reopened store still reported its own scope, which
/// passes whatever the decoder does, because the caller supplied that scope. Recorded because it is
/// the trap recorded in the `coding-lessons` set: the check has to reach the path that can be wrong.
#[test]
fn opening_a_store_as_the_wrong_scope_names_the_scope_it_holds() {
    let directory = tempdir();
    // One path, opened as two different scopes -- `Store::open` derives the filename from the
    // scope, so reaching the mismatch at all requires naming the file directly.
    let path = directory.path().join("set.sqlite3");
    let held = scope("coding-lessons");

    Store::open_at(held.clone(), &path).expect("creates");
    let Err(error) = Store::open_at(Scope::Local, &path) else {
        panic!("opening a store as the wrong scope must be refused");
    };

    let message = error.to_string();
    assert!(
        message.contains("coding-lessons"),
        "the refusal has to name the scope actually held, or it sends the reader after the wrong \
         set: {message}"
    );
}

/// The fix for `keywords-silently-lowercased`, at the layer that has to hold it: the keywords table
/// is both the record and the index, so preserving the caller's spelling and matching without
/// regard to case are the same table's problem.
///
/// Before this, `Keyword::new` lowercased at construction and `keywords_of` re-ran that constructor
/// on the way out, so the original spelling was destroyed twice over and no interface could show it.
#[test]
fn a_keyword_is_stored_as_written_and_found_whatever_the_case() {
    let mut store = Store::open_in_memory_and_mint_id(scope("scatterhoard")).expect("opens");
    let mut memory = a_memory("a memory about views");
    memory.keywords = vec![
        Keyword::new("MemoryView").expect("valid keyword"),
        Keyword::new("STANDARDS").expect("valid keyword"),
    ];
    store.insert(&memory).expect("inserts");

    for spelling in ["MemoryView", "memoryview", "MEMORYVIEW"] {
        let hits = store
            .search_keywords(&[Keyword::new(spelling).expect("valid keyword")], 5)
            .expect("searches");
        assert_eq!(hits.len(), 1, "`{spelling}` found nothing");
        assert_eq!(
            hits[0].memory.keywords[0].as_str(),
            "MemoryView",
            "searching for `{spelling}` returned the memory with its keyword rewritten"
        );
        assert_eq!(hits[0].memory.keywords[1].as_str(), "STANDARDS");
    }
}

/// A store written before keywords kept their case must keep working when this build opens it.
///
/// `CREATE TABLE IF NOT EXISTS` cannot add a column to an existing table, so without a migration an
/// older store opens with no `folded` column and **every keyword search silently matches nothing** —
/// the store looks healthy and answers wrongly. This builds the previous schema directly, because a
/// migration can only be tested against a database this code did not write.
///
/// The backfill copies `keyword` into `folded`, which is exactly right for such a store: every
/// keyword in it went through the old lowercasing constructor, so its stored spelling already is
/// its folded form.
#[test]
fn a_store_written_before_folded_keywords_still_searches() {
    let directory = tempdir();
    let path = directory.path().join("old.sqlite3");
    let memory = a_memory("a memory from an older build");

    {
        let old = rusqlite::Connection::open(&path).expect("creates");
        old.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE memories (
                 id TEXT PRIMARY KEY, content TEXT NOT NULL, why TEXT, context TEXT,
                 confidence REAL NOT NULL, origin TEXT NOT NULL,
                 created_by TEXT NOT NULL, created_at TEXT NOT NULL,
                 modified_by TEXT NOT NULL, modified_at TEXT NOT NULL);
             CREATE TABLE keywords (
                 memory_id TEXT NOT NULL, keyword TEXT NOT NULL, position INTEGER NOT NULL,
                 PRIMARY KEY (memory_id, position));",
        )
        .expect("builds the previous schema");
        old.execute(
            "INSERT INTO meta (key, value) VALUES ('scope', 'shared:scatterhoard')",
            [],
        )
        .expect("records the scope");
        // Attribution carries a record id rather than a name, which is orthogonal to the folded
        // keywords this test is about. A store old enough to predate *that* change is a separate
        // problem, and one no reader can solve on its own: whether two names are one person is not
        // answerable from the database. See `documents/design/memory-data-model.md`.
        let author = MemoryId::generate().to_string();
        old.execute(
            "INSERT INTO memories (id, content, confidence, origin, created_by, created_at,
                                   modified_by, modified_at)
             VALUES (?1, 'a memory from an older build', 0.8, 'llm', ?2,
                     '2026-08-01T09:00:00Z', ?2, '2026-08-01T09:00:00Z')",
            rusqlite::params![memory.id.to_string(), author],
        )
        .expect("writes a memory");
        // Already folded, because the old constructor folded everything it was given.
        old.execute(
            "INSERT INTO keywords (memory_id, keyword, position) VALUES (?1, 'memoryview', 0)",
            rusqlite::params![memory.id.to_string()],
        )
        .expect("writes a keyword");
    }

    let store = Store::open_at(scope("scatterhoard"), &path).expect("opens an older store");
    let hits = store
        .search_keywords(&[Keyword::new("MemoryView").expect("valid keyword")], 5)
        .expect("searches");

    assert_eq!(
        hits.len(),
        1,
        "an older store's keywords became unfindable, silently"
    );
    assert_eq!(hits[0].memory.id, memory.id);
}

/// The migration must give **every** existing row a lookup key, including a keyword this build's
/// constructor will not parse.
///
/// This is what separates the backfill from `refold_keywords`, and without it nothing does. The
/// refold runs two statements later in the same open and recomputes `folded` for every row whose
/// stored key differs from what `Keyword::folded` produces — which covers the whole table and made
/// the backfill unobservable, so `store: skip the backfill when adding the folded column` was
/// recorded as caught while the suite passed without it
/// (`the-folded-column-backfill-is-untested-and-the-corpus-calls-it-caught` in `TRACKING.md`).
///
/// The one row it cannot cover is one whose spelling `Keyword::new` rejects: it `continue`s past
/// it, and then records its marker on the way out, so a row left at the column's `DEFAULT ''` stays
/// that way for the life of the store. A keyword carrying whitespace is exactly what a store
/// written before the constructor tightened can hold.
///
/// **Asserted against the table rather than through a search**, because a query keyword must itself
/// be a valid `Keyword`, so no search can be constructed that would match either the right value or
/// the wrong one. What is being pinned is that the migration leaves the row carrying its spelling
/// rather than nothing — the state a later relaxation of the constructor, or any second reader of
/// that column, would find.
#[test]
fn the_migration_gives_a_keyword_this_build_cannot_parse_a_lookup_key() {
    let directory = tempdir();
    let path = directory.path().join("unparseable.sqlite3");
    let memory = a_memory("a memory from a build with looser keywords");

    {
        let old = rusqlite::Connection::open(&path).expect("creates");
        old.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE memories (
                 id TEXT PRIMARY KEY, content TEXT NOT NULL, why TEXT, context TEXT,
                 confidence REAL NOT NULL, origin TEXT NOT NULL,
                 created_by TEXT NOT NULL, created_at TEXT NOT NULL,
                 modified_by TEXT NOT NULL, modified_at TEXT NOT NULL);
             CREATE TABLE keywords (
                 memory_id TEXT NOT NULL, keyword TEXT NOT NULL, position INTEGER NOT NULL,
                 PRIMARY KEY (memory_id, position));",
        )
        .expect("builds the previous schema");
        old.execute(
            "INSERT INTO meta (key, value) VALUES ('scope', 'shared:scatterhoard')",
            [],
        )
        .expect("records the scope");
        old.execute(
            "INSERT INTO memories (id, content, confidence, origin, created_by, created_at,
                                   modified_by, modified_at)
             VALUES (?1, 'a memory from a build with looser keywords', 0.8, 'llm', 'jtcours',
                     '2026-08-01T09:00:00Z', 'jtcours', '2026-08-01T09:00:00Z')",
            rusqlite::params![memory.id.to_string()],
        )
        .expect("writes a memory");
        // Whitespace, which `Keyword::new` refuses — so the refold cannot derive a key for it and
        // the backfill is the only statement that ever writes one.
        old.execute(
            "INSERT INTO keywords (memory_id, keyword, position) VALUES (?1, 'memory view', 0)",
            rusqlite::params![memory.id.to_string()],
        )
        .expect("writes a keyword this build would refuse");
    }

    drop(Store::open_at(scope("scatterhoard"), &path).expect("opens an older store"));

    let reopened = rusqlite::Connection::open(&path).expect("reopens");
    let folded: String = reopened
        .query_row(
            "SELECT folded FROM keywords WHERE keyword = 'memory view'",
            [],
            |row| row.get(0),
        )
        .expect("the row survived the migration");

    assert_eq!(
        folded, "memory view",
        "the migration left a keyword it could not parse with no lookup key at all, and the refold \
         records its marker on the way out, so nothing will ever fill it"
    );
}

/// A store written before `crockford-base32-ids` holds every id in the hex spelling, and this build
/// stringifies an id as Crockford — so without a migration `WHERE id = ?` matches nothing and **every
/// memory in the store becomes unreachable while the file looks healthy**. That is the same silent
/// shape as `a_store_written_before_folded_keywords_still_searches`, one column over.
///
/// Built by writing a store with this build and then rewriting its ids to the previous spelling,
/// because what is being tested is the spelling rather than the schema, and the schema migrations
/// are covered separately.
#[test]
fn a_store_written_with_hex_ids_is_reachable_after_the_encoding_change() {
    let directory = tempdir();
    let path = directory.path().join("hex.sqlite3");

    let first = a_memory("the memory an older build wrote");
    let second = {
        let mut m = a_memory("a memory pointing at it");
        m.links = vec![Link {
            label: LinkLabel::RelatedTo,
            target: LinkTarget::SameSet(first.id),
        }];
        m
    };

    {
        let mut store = Store::open_at(scope("scatterhoard"), &path).expect("creates");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.insert(&first).expect("inserts");
        store.insert(&second).expect("inserts");
    }

    // Rewrite every id column into the previous spelling.
    let hex_of = |id: &MemoryId| {
        let h = format!("{:032x}", id.to_u128());
        format!(
            "{}-{}-{}-{}-{}",
            &h[0..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..32]
        )
    };
    {
        let old = rusqlite::Connection::open(&path).expect("opens");
        // The same reason the migration itself does this: `memories.id` parents three cascading
        // children, and the schema declares no `ON UPDATE`.
        old.execute_batch("PRAGMA foreign_keys = OFF")
            .expect("relaxes enforcement for the rewrite");
        for id in [first.id, second.id] {
            let (new, previous) = (id.to_string(), hex_of(&id));
            for (table, column) in [
                ("memories", "id"),
                ("keywords", "memory_id"),
                ("links", "source"),
                ("links", "target"),
                ("vectors", "memory_id"),
            ] {
                old.execute(
                    &format!("UPDATE {table} SET {column} = ?1 WHERE {column} = ?2"),
                    rusqlite::params![previous, new],
                )
                .expect("rewrites an id");
            }
        }
    }

    // The setup has to be proved, or a change to `hex_of` could leave this test passing while
    // exercising nothing — which is the failure mode this repository keeps finding.
    {
        let old = rusqlite::Connection::open(&path).expect("opens");
        let hex_rows: i64 = old
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE length(id) = 36",
                [],
                |row| row.get(0),
            )
            .expect("counts");
        assert_eq!(
            hex_rows, 2,
            "the fixture must actually hold the old spelling"
        );
    }

    let store = Store::open_at(scope("scatterhoard"), &path).expect("opens an older store");

    let found = store
        .get(first.id)
        .expect("reads")
        .expect("a memory written by an older build must still be reachable by its id");
    assert_eq!(found.content.as_data(), "the memory an older build wrote");

    // And the reference survives, which is the half a per-row fix would miss.
    let incoming = store.incoming(first.id).expect("reads incoming edges");
    assert_eq!(
        incoming.len(),
        1,
        "a link written in the previous spelling must still point at its target"
    );
    assert_eq!(incoming[0].0, second.id);
}

/// A store written before the `bases` migration opens, and stops cascading.
///
/// The first version of that table declared `memory_id ... REFERENCES memories(id) ON DELETE
/// CASCADE`, copied from the sibling tables where cascading is correct. It is not correct here:
/// `write_memory` upserts with `INSERT OR REPLACE`, which deletes the old row, so **every amendment
/// destroyed the base it should have preserved** — turning a one-sided amendment into a conflict on
/// the next import. Built directly here, because a migration can only be tested against a database
/// this code did not write.
#[test]
fn a_store_written_before_the_bases_migration_stops_cascading() {
    let directory = tempdir();
    let path = directory.path().join("cascading.sqlite3");
    let memory = a_memory("a memory from a build with the wrong foreign key");

    {
        let old = rusqlite::Connection::open(&path).expect("creates");
        old.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE memories (
                 id TEXT PRIMARY KEY, content TEXT NOT NULL, why TEXT, context TEXT,
                 confidence REAL NOT NULL, origin TEXT NOT NULL,
                 created_by TEXT NOT NULL, created_at TEXT NOT NULL,
                 modified_by TEXT NOT NULL, modified_at TEXT NOT NULL);
             CREATE TABLE bases (
                 target    TEXT NOT NULL,
                 memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                 checksum  TEXT NOT NULL,
                 PRIMARY KEY (target, memory_id));",
        )
        .expect("builds the previous schema");
        old.execute(
            "INSERT INTO meta (key, value) VALUES ('scope', 'shared:scatterhoard')",
            [],
        )
        .expect("records the scope");
        old.execute(
            "INSERT INTO memories (id, content, confidence, origin, created_by, created_at,
                                   modified_by, modified_at)
             VALUES (?1, 'a memory from a build with the wrong foreign key', 0.8, 'llm', 'jtcours',
                     '2026-08-01T09:00:00Z', 'jtcours', '2026-08-01T09:00:00Z')",
            rusqlite::params![memory.id.to_string()],
        )
        .expect("writes a memory");
        old.execute(
            "INSERT INTO bases (target, memory_id, checksum) VALUES ('/somewhere', ?1, 'abc')",
            rusqlite::params![memory.id.to_string()],
        )
        .expect("writes a base");
    }

    let mut store = Store::open_at(scope("scatterhoard"), &path).expect("opens an older store");
    store
        .mint_set_id()
        .expect("mints an identity, as create does");

    // Read through a plain connection rather than through the store: the schema is what is being
    // asserted, and a test-only accessor on `Store` would widen the API to say something SQLite
    // already answers.
    let cascades: i64 = rusqlite::Connection::open(&path)
        .expect("opens the same file")
        .prepare("SELECT COUNT(*) FROM pragma_foreign_key_list('bases')")
        .expect("prepares")
        .query_row([], |row| row.get(0))
        .expect("counts");
    assert_eq!(cascades, 0, "the cascading foreign key survived the open");

    // **The rows are dropped, and that is the repair rather than a cost of it.** A base is local
    // derived state that never reaches a memory file, so losing one makes the next import read that
    // record as having no ancestor -- noisy for one cycle, and not lossy. Preserving rows through a
    // table rebuild would be more code defending a value that is cheaper to re-establish, and it is
    // the direction `tests/reconcile.rs` calls out: never keep a base you cannot prove.
    assert!(
        store.bases_for("/somewhere").expect("reads").is_empty(),
        "a base carried through the migration is one nothing proved"
    );

    // And the store still works: the migration must not have taken the memory with it.
    store.insert(&memory).expect("writes after the migration");
    let after = store.bases_for("/somewhere").expect("reads");
    assert!(after.is_empty());
}

/// **The migration must not fire on a store that never had the fault.**
///
/// `drop_cascading_bases` keys on whether `bases` declares any foreign key at all. If that detection
/// were wrong in the permissive direction the table would be dropped on *every* open, and the damage
/// would be invisible: no error, no missing memory, just every record reading as having no ancestor
/// at the next import, for ever. That is a worse failure than the one being migrated away from,
/// because it never stops happening.
#[test]
fn opening_a_current_store_twice_keeps_its_bases() {
    let directory = tempdir();
    let path = directory.path().join("current.sqlite3");
    let memory = a_memory("a memory in a store this build wrote");

    {
        let mut store = Store::open_at(scope("scatterhoard"), &path).expect("creates");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.insert(&memory).expect("writes");
        store
            .set_bases("/somewhere", &[(memory.id, 0xabc_def)])
            .expect("records an agreement");
    }

    let store = Store::open_at(scope("scatterhoard"), &path).expect("reopens");
    let bases = store.bases_for("/somewhere").expect("reads");

    assert_eq!(
        bases.get(&memory.id).copied(),
        Some(0xabc_def),
        "reopening a healthy store dropped its bases, so every import will see every record as new"
    );
}

/// A store imported **before** links were canonicalised holds the long spelling, and those rows are
/// inert: every lookup that decides standing keys on `target_set IS NULL`, so a supersession
/// written with its own set's name silently does nothing.
///
/// The write-side fix cannot reach such a store on its own — a set is not re-imported often — so
/// the compaction runs at open. `a-same-set-link-spelled-with-its-set-name-is-inert`.
#[test]
fn a_store_holding_the_long_spelling_is_compacted_at_open() {
    let directory = tempdir();
    let path = directory.path().join("legacy.sqlite3");
    let scope = Scope::Shared(SetName::new("ours").expect("valid set name"));

    let old = a_memory("the original claim");
    let mut new = a_memory("the corrected claim");
    new.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(old.id),
    }];

    {
        let mut store = Store::open_at(scope.clone(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.insert(&old).expect("inserts");
        store.insert(&new).expect("inserts");
    }

    // Rewrite the compact row into the long form, which is what an import before the fix produced.
    {
        let connection = rusqlite::Connection::open(&path).expect("opens the database directly");
        let changed = connection
            .execute(
                "UPDATE links SET target_set = 'ours' WHERE source = ?1",
                rusqlite::params![new.id.to_string()],
            )
            .expect("rewrites the row");
        assert_eq!(changed, 1, "the fixture did not plant the long spelling");
    }

    let store = Store::open_at(scope, &path).expect("reopens");
    assert_eq!(
        store.resolve_supersession(old.id).expect("resolves"),
        Resolution::Superseded {
            tips: vec![new.id],
            unresolved: false,
            forgotten: false,
        },
        "the long-spelled edge is still inert, so a real correction does nothing"
    );
}

/// A store written when the fold was `to_lowercase` holds keys the query path can no longer match,
/// so its keyword search silently returns less than it should. Opening it repairs them.
///
/// `folding-is-lowercase-not-casefold`. Driven by planting the old key with `rusqlite` directly,
/// because the current code cannot produce it — the same reason the compaction migration is tested
/// this way.
#[test]
fn a_store_folded_under_the_old_rule_is_refolded_at_open() {
    let directory = tempdir();
    let path = directory.path().join("old-fold.sqlite3");
    let scope = Scope::Shared(SetName::new("ours").expect("valid set name"));

    let mut memory = a_memory("a memory about a German street");
    memory.keywords = vec![Keyword::new("Straße").expect("valid keyword")];

    {
        let mut store = Store::open_at(scope.clone(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.insert(&memory).expect("inserts");
    }

    // Plant the lowercase-era key and clear the marker, which is exactly what an older build left.
    {
        let connection = rusqlite::Connection::open(&path).expect("opens the database directly");
        connection
            .execute(
                "UPDATE keywords SET folded = 'straße' WHERE keyword = 'Straße'",
                [],
            )
            .expect("plants the old key");
        connection
            .execute("DELETE FROM meta WHERE key = 'keyword_fold'", [])
            .expect("clears the marker");
    }

    let store = Store::open_at(scope, &path).expect("reopens");
    let hits = store
        .search_keywords(&[Keyword::new("STRASSE").expect("valid keyword")], 10)
        .expect("searches");

    assert_eq!(
        hits.len(),
        1,
        "the old lookup key survived, so this keyword is unreachable by the rule the query uses"
    );
    assert_eq!(hits[0].memory.id, memory.id);
}

/// **A whole-set import carries what the set forgot.**
///
/// `memory-file-read-silently-discards-tombstones`, the last corner of it. `MemorySet` could not
/// express a tombstone at all until 2026-08-26, so this path was structurally incapable of moving
/// one — and a set imported through it came out having forgotten what it had forgotten, ready to
/// resurrect every one of them on the next export.
#[test]
fn importing_a_whole_set_keeps_what_it_forgot() {
    let mut store = Store::open_in_memory_and_mint_id(scope("scatterhoard")).expect("opens");

    let live = a_memory("alpha is still here");
    let gone = MemoryId::generate();

    store
        .import(&MemorySet {
            scope: scope("scatterhoard"),
            memories: vec![live.clone()],
            tombstones: vec![Tombstone::new(
                gone,
                Attribution {
                    by: MemoryId::generate(),
                    at: Timestamp::from_str("2026-08-02T09:00:00Z").expect("valid timestamp"),
                },
                Vec::new(),
                None,
            )],
            set_id: None,
        })
        .expect("imports");

    assert_eq!(store.count().expect("counts"), 1);
    assert_eq!(store.forgotten_count().expect("counts"), 1);
    assert_eq!(
        store
            .tombstones()
            .expect("reads")
            .iter()
            .map(|tombstone| tombstone.id)
            .collect::<Vec<_>>(),
        vec![gone]
    );

    // The reason a tombstone is kept rather than dropped: it is what makes a link into the
    // forgotten memory a good reference instead of a dangling one.
    assert!(
        store.holds_record(gone).expect("asks"),
        "a tombstone is a record the store holds"
    );
}

// ---------------------------------------------------------------------------
// One spelling per directory, for rows written before there was a rule
//
// `target-identity-is-an-unnormalised-path-string`. `Store::normalise_target` fixes what arrives
// from now on; these are about what a store already carried. The rule is settled in
// `documents/design/store-registry.md` under *never keep a base you cannot prove*.
// ---------------------------------------------------------------------------

/// A spelling that resolves to another is re-keyed, and the agreement survives.
#[test]
fn opening_a_store_re_keys_a_target_onto_its_resolved_spelling() {
    let home = tempdir();
    let checkout = home.path().join("checkout");
    std::fs::create_dir_all(&checkout).expect("creates");

    let memory = a_memory("alpha was agreed once");
    let spelled_with_a_slash = format!("{}/", checkout.to_string_lossy());

    {
        let mut store = Store::open(scope("scatterhoard"), home.path()).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.insert(&memory).expect("inserts");
        store
            .set_bases(&spelled_with_a_slash, &[(memory.id, 0x1234)])
            .expect("records");
    }

    let store = Store::open(scope("scatterhoard"), home.path()).expect("reopens");
    let targets = store.known_targets().expect("reads");
    assert_eq!(
        targets.len(),
        1,
        "one directory is one target however it was spelled: {targets:?}"
    );
    assert!(
        !targets[0].ends_with('/'),
        "the stored spelling should be the resolved one: {targets:?}"
    );
    assert_eq!(
        store
            .bases_for(&checkout.to_string_lossy())
            .expect("reads")
            .get(&memory.id),
        Some(&0x1234),
        "re-keying must carry the agreement with it, not drop it"
    );
}

/// **Two spellings of one directory are a collision, not a merge.**
///
/// The trap the design names by itself: they carry two independent base sets, and wherever they
/// disagree about a `memory_id` neither is the other's ancestor. Picking either supplies a wrong
/// base for the file the other described, which is a silent overwrite — the one outcome
/// reconciliation exists to prevent. Both go, and the next export re-establishes agreement.
#[test]
fn two_spellings_that_resolve_to_one_are_dropped_rather_than_merged() {
    let home = tempdir();
    let checkout = home.path().join("checkout");
    std::fs::create_dir_all(&checkout).expect("creates");

    let memory = a_memory("alpha was agreed twice, differently");
    let plain = checkout.to_string_lossy().to_string();
    let with_a_slash = format!("{plain}/");

    {
        let mut store = Store::open(scope("scatterhoard"), home.path()).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.insert(&memory).expect("inserts");
        // Two agreements about one memory, disagreeing — which is exactly what makes neither of
        // them the other's ancestor.
        store
            .set_bases(&plain, &[(memory.id, 0xAAAA)])
            .expect("records");
        // Written straight to the table, because `set_bases` now normalises and would otherwise
        // collapse these two before they could collide. The point is a store that predates that.
        let connection =
            rusqlite::Connection::open(home.path().join("shared-scatterhoard.sqlite3"))
                .expect("opens the database directly");
        connection
            .execute(
                "INSERT INTO bases (target, memory_id, checksum) VALUES (?1, ?2, ?3)",
                rusqlite::params![with_a_slash, memory.id.to_string(), "000000000000bbbb"],
            )
            .expect("writes the second spelling");
    }

    let store = Store::open(scope("scatterhoard"), home.path()).expect("reopens");
    assert!(
        store.known_targets().expect("reads").is_empty(),
        "neither spelling's agreement can be proved, so neither may be kept"
    );
    assert!(
        store.bases_for(&plain).expect("reads").is_empty(),
        "and picking one would supply a wrong ancestor for the file the other described"
    );
}

/// A target whose directory has gone keeps its spelling, and its agreement.
///
/// Dropping it would be the tidy-looking move and it discards ancestry that is still good: an
/// unmounted volume, or a checkout deleted and later restored, comes back to the same path. Keeping
/// it is safe because a base is only ever consulted by exact key, so an unresolvable key can be
/// matched by nothing but the path it names.
#[test]
fn a_target_that_no_longer_exists_is_left_alone() {
    let home = tempdir();
    let memory = a_memory("alpha was agreed with a volume that is not mounted");
    let gone = "/nowhere-that-exists/checkout";

    {
        let mut store = Store::open(scope("scatterhoard"), home.path()).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.insert(&memory).expect("inserts");
        store
            .set_bases(gone, &[(memory.id, 0x5678)])
            .expect("records");
    }

    let store = Store::open(scope("scatterhoard"), home.path()).expect("reopens");
    assert_eq!(
        store.known_targets().expect("reads"),
        vec![gone.to_owned()],
        "an unresolvable target must stay nameable"
    );
    assert_eq!(
        store.bases_for(gone).expect("reads").get(&memory.id),
        Some(&0x5678),
        "and keep the agreement it recorded"
    );
}

/// **Minting is idempotent, and the guard is load-bearing.** Every write asks for a mint, since
/// `set-settled-at-first-write`, so this is what keeps the second write from renaming the set: the rule
/// *a set's identity never changes* has to hold at the layer that owns it.
#[test]
fn minting_a_set_identity_twice_keeps_the_first() {
    // Opened with no identity, so the first mint is the one that makes it.
    let store = Store::open_in_memory(scope("scatterhoard")).expect("opens");
    let first = store.mint_set_id().expect("mints");
    let second = store.mint_set_id().expect("mints again");

    assert_eq!(first, second, "a set's identity must never change");
    assert_eq!(store.set_id().expect("reads"), Some(first));
}

/// A store with no identity takes the one offered; one that has an identity keeps it. Filling an
/// absence converges, because every reader agrees on what it finds; displacing oscillates.
///
/// **Held means held, records or not, since `set-settled-at-first-write`.** An empty set used to be
/// allowed to give up the identity it held, because `create` minted one seconds before `import` could
/// offer the file's. `create` no longer mints, so the only empty set holding an identity is one that
/// was given it on purpose, and there is nothing left for the exception to repair.
#[test]
fn a_store_adopts_an_identity_only_into_an_absence() {
    let mut store = Store::open_in_memory(scope("scatterhoard")).expect("opens");
    assert_eq!(
        store.set_id().expect("reads"),
        None,
        "a store starts with no identity: opening one must not write to it"
    );

    let offered = SetId::generate();
    assert!(store.adopt_set_id(offered).expect("adopts"), "took it");
    assert_eq!(store.set_id().expect("reads"), Some(offered));

    let rival = SetId::generate();
    assert!(
        !store.adopt_set_id(rival).expect("declines"),
        "an identity once held is not displaced, even by a set that has written nothing under it"
    );
    assert_eq!(store.set_id().expect("reads"), Some(offered));

    store
        .insert(&a_memory("the first record, written under `offered`"))
        .expect("inserts");
    assert!(
        !store.adopt_set_id(SetId::generate()).expect("declines"),
        "and still not once it has"
    );
    assert_eq!(store.set_id().expect("reads"), Some(offered));
}

/// **Every set claims an identity on its first write, `local` included**, and never changes it.
///
/// **On the first write rather than at open**, following `author_record_or_mint` and for its stated
/// reason: opening a store, or reading one, must never write to it.
#[test]
fn a_local_store_claims_an_identity_on_its_first_write() {
    let mut store = Store::open_in_memory(Scope::Local).expect("opens");
    assert_eq!(
        store.set_id().expect("reads"),
        None,
        "opening a store must not write to it"
    );

    store
        .insert(&a_memory("alpha, the first thing this set ever held"))
        .expect("writes");
    let minted = store
        .set_id()
        .expect("reads")
        .expect("a local set claims an identity when it is first written to");

    store
        .insert(&a_memory("beta, written afterwards"))
        .expect("writes");
    assert_eq!(
        store.set_id().expect("reads"),
        Some(minted),
        "and never changes it afterwards"
    );
}

/// **A shared set claims its identity on its first write, as `local` does**, since
/// `set-settled-at-first-write` (decided 2026-09-24). It used to be refused, on the ground that two
/// holders of one set would each mint a rival -- but `create` minted just as early and forked just as
/// well, and since `identity-mismatch-refusal` a fork is refused at the next import rather than
/// merged silently. What stays true is the invariant the refusal protected: a set holding records has
/// an identity, which is what deciding whether a stored link is same-set relies on.
#[test]
fn a_shared_store_claims_an_identity_on_its_first_write() {
    let mut store = Store::open_in_memory(scope("scatterhoard")).expect("opens");

    store
        .insert(&a_memory("alpha in a set that travels"))
        .expect("a shared set with no identity takes its first record");
    let minted = store
        .set_id()
        .expect("reads")
        .expect("and claims an identity to hold it under");

    store
        .insert(&a_memory("beta, written afterwards"))
        .expect("writes");
    assert_eq!(
        store.set_id().expect("reads"),
        Some(minted),
        "and never changes it afterwards"
    );
}

/// Restore writes a whole file's records by its own path, so it claims one the same way -- before
/// any record, since every record is canonicalised against the identity of the set holding it.
#[test]
fn restoring_into_a_shared_store_with_no_identity_claims_one() {
    let mut store = Store::open_in_memory(scope("scatterhoard")).expect("opens");

    store
        .replace_with(
            &scope("scatterhoard"),
            &[scatterhoard::memory_file::Record::Memory(Box::new(
                a_memory("alpha, restored into a set with no identity"),
            ))],
        )
        .expect("a restore into a set with no identity proceeds");
    assert!(store.set_id().expect("reads").is_some(), "and claims one");
    assert_eq!(store.count().expect("counts"), 1);
}

// ---------------------------------------------------------------------------
// Author records
// ---------------------------------------------------------------------------

/// A real identity record, built the way the store builds one.
///
/// **Through `Memory::self_attributed`, not by adding the keyword to an ordinary memory.** The
/// fixture used to do the latter, and that is why the suite could not see
/// `about-hoard-author-as-a-subject-keyword-silently-disables-embedding`: it imitated the marker
/// rather than the shape, so it passed under a keyword test and would have passed under any other
/// test of the same keyword. What makes a record an identity is that it attributes itself.
fn an_author_record(name: &str) -> Memory {
    let mut memory = Memory::self_attributed(
        MemoryText::new(name).expect("valid content"),
        Confidence::new(1.0).expect("in range"),
        Origin::Human,
        jiff::Timestamp::from_str("2026-08-02T09:00:00Z").expect("valid timestamp"),
    );
    memory.keywords = vec![Keyword::new("hoard-author").expect("valid keyword")];
    memory
}

#[test]
fn an_author_record_resolves_to_the_name_it_carries() {
    let mut store = a_store();
    let record = an_author_record("jtcours");
    let id = record.id;
    store.insert(&record).expect("inserts");

    assert_eq!(
        store.author_name(id).expect("resolves"),
        Some(Author::new("jtcours").expect("valid author")),
    );
}

#[test]
fn a_superseded_author_record_resolves_to_the_name_at_the_tip() {
    let mut store = a_store();
    let old = an_author_record("alice");
    let old_id = old.id;
    store.insert(&old).expect("inserts");

    let mut new = an_author_record("jtcours");
    new.links = vec![supersedes(old_id)];
    store.insert(&new).expect("inserts");

    // The whole point of holding `by` as a reference: memories still pointing at the old id read
    // as the new name without any of them being rewritten.
    assert_eq!(
        store.author_name(old_id).expect("resolves"),
        Some(Author::new("jtcours").expect("valid author")),
    );
}

/// **The name a set publishes this store's author under is the current one**: the tip of the record
/// the store resolves as its author, so a rename merged in from another copy moves it, and a record
/// split in two names nobody. `the-review-flags-the-persons-own-published-name`.
#[test]
fn the_published_name_is_the_current_name_of_this_stores_author() {
    let at = Timestamp::from_str("2026-10-29T09:00:00Z").expect("valid timestamp");
    let mut store = a_store();
    assert_eq!(
        store.published_name().expect("reads"),
        None,
        "no author yet"
    );

    let minted = store
        .author_record_or_mint(&Author::new("alice").expect("valid author"), at)
        .expect("mints")
        .expect("an id");
    assert_eq!(
        store.published_name().expect("reads").as_deref(),
        Some("alice")
    );

    let mut renamed = an_author_record("jtcours");
    renamed.links = vec![supersedes(minted)];
    store.insert(&renamed).expect("inserts");
    assert_eq!(
        store.published_name().expect("reads").as_deref(),
        Some("jtcours"),
        "a rename from another copy is the current name"
    );

    let mut split = an_author_record("asmith");
    split.links = vec![supersedes(minted)];
    store.insert(&split).expect("inserts");
    assert_eq!(
        store.published_name().expect("reads"),
        None,
        "two current names are no answer"
    );
}

#[test]
fn a_memory_that_is_not_an_author_record_resolves_to_nothing() {
    let mut store = a_store();
    let ordinary = a_memory("jtcours");
    let id = ordinary.id;
    store.insert(&ordinary).expect("inserts");

    // Content alone must not make something an identity: the reserved keyword is what marks it.
    assert_eq!(store.author_name(id).expect("resolves"), None);
}

#[test]
fn an_author_record_split_in_two_resolves_to_neither() {
    let mut store = a_store();
    let original = an_author_record("alice");
    let original_id = original.id;
    store.insert(&original).expect("inserts");

    for name in ["jtcours", "asmith"] {
        let mut replacement = an_author_record(name);
        replacement.links = vec![supersedes(original_id)];
        store.insert(&replacement).expect("inserts");
    }

    // Two tips means one identity became two, which is the duplicate-chain hazard the design says
    // nothing downstream can detect. Picking either one would be inventing the answer.
    assert_eq!(store.author_name(original_id).expect("resolves"), None);
}

#[test]
fn renaming_an_author_re_attributes_a_memory_without_rewriting_it() {
    let mut store = a_store();

    let identity = an_author_record("alice");
    let identity_id = identity.id;
    store.insert(&identity).expect("inserts");

    let mut memory = a_memory("attribution is the one part that legitimately changes");
    memory.created.by = identity_id;
    memory.modified.by = identity_id;
    store.insert(&memory).expect("inserts");

    let before = scatterhoard::checksum::of_memory(&memory);
    assert_eq!(
        store.author_name(memory.created.by).expect("resolves"),
        Some(Author::new("alice").expect("valid author")),
    );

    let mut renamed = an_author_record("jtcours");
    renamed.links = vec![supersedes(identity_id)];
    store.insert(&renamed).expect("inserts");

    // This is the whole thesis of holding `by` as a reference: the memory now reads as written by
    // `jtcours`, and its own record did not move -- so re-attributing 155 memories is one
    // supersession rather than 155 rewrites, and no checksum changes.
    assert_eq!(
        store.author_name(memory.created.by).expect("resolves"),
        Some(Author::new("jtcours").expect("valid author")),
    );
    let after = store.get(memory.id).expect("reads").expect("still there");
    assert_eq!(scatterhoard::checksum::of_memory(&after), before);
}

// ---------------------------------------------------------------------------
// The durable author map, which lives in `local`
// ---------------------------------------------------------------------------

fn a_local_store() -> Store {
    Store::open_in_memory(Scope::Local).expect("opens")
}

#[test]
fn the_author_map_answers_by_set_identity() {
    let local = a_local_store();
    let set = SetId::generate();
    let record = MemoryId::generate();

    assert_eq!(local.author_map_get(set).expect("reads"), None);

    local
        .author_map_put(set, record, "shared:scatterhoard")
        .expect("writes");

    let found = local.author_map_get(set).expect("reads").expect("a row");
    assert_eq!(found.record, record);
    assert_eq!(found.scope, "shared:scatterhoard");
}

#[test]
fn the_author_map_refreshes_the_scope_label_but_not_the_key() {
    let local = a_local_store();
    let set = SetId::generate();
    let record = MemoryId::generate();
    local
        .author_map_put(set, record, "shared:old-name")
        .expect("writes");

    // A set's local name can change; its identity cannot. The label follows the name so a person
    // can recognise the row, and the key stays the identity so nothing matches on the name.
    local
        .author_map_put(set, record, "shared:new-name")
        .expect("rewrites");

    let found = local.author_map_get(set).expect("reads").expect("a row");
    assert_eq!(found.scope, "shared:new-name");
    assert_eq!(local.author_map_rows().expect("lists").len(), 1);
}

#[test]
fn a_row_survives_for_a_set_that_is_no_longer_here() {
    let local = a_local_store();
    let gone = SetId::generate();
    local
        .author_map_put(gone, MemoryId::generate(), "shared:dropped")
        .expect("writes");

    // The whole point of the durable copy: dropping a set destroys its own database, and re-adding
    // it must not meet a question already answered. So the row outlives the set, and a person
    // prunes it by the scope label without the set being present.
    assert!(local.author_map_get(gone).expect("reads").is_some());
}

#[test]
fn a_deleted_row_is_safe_because_the_fallback_asks() {
    let local = a_local_store();
    let set = SetId::generate();
    local
        .author_map_put(set, MemoryId::generate(), "shared:pruned")
        .expect("writes");

    local.author_map_forget(set).expect("forgets");

    assert_eq!(local.author_map_get(set).expect("reads"), None);
}

#[test]
fn a_shared_store_holds_no_author_map() {
    let store = a_store();
    let error = store
        .author_map_get(SetId::generate())
        .expect_err("a shared store has no map to read");

    // The map answers *which identity record is me in set X*, which is a fact about this machine
    // rather than about any set. Held in one store so there is never a question which copy answers.
    assert!(
        error.to_string().contains("local store"),
        "the refusal should say where the map lives, and got: {error}"
    );
}

/// **The store's own answer wins over minting, tested where that guard lives.**
///
/// A server consults the durable map in `local` before it ever reaches this, so a mutation removing
/// this check is invisible from the tool surface — the outer layer supplies the same answer and the
/// suite stays green while the inner guard stops being one. That masking is the reason this test is
/// at store level rather than through a server: two layers enforcing one property need a test each,
/// or the corpus reports coverage that only the outer one has.
#[test]
fn resolving_an_author_twice_returns_the_same_record_rather_than_minting_again() {
    let mut store = a_store();
    let name = Author::new("jtcours").expect("valid author");
    let at = Timestamp::from_str("2026-09-02T09:00:00Z").expect("valid timestamp");

    let first = store
        .author_record_or_mint(&name, at)
        .expect("resolves")
        .expect("mints where nothing can collide");
    let second = store
        .author_record_or_mint(&name, at)
        .expect("resolves")
        .expect("the same identity, from the store's own record of it");

    assert_eq!(
        first, second,
        "a second write must not mint a second identity for one person"
    );
    assert_eq!(
        store.identity_count().expect("counts"),
        1,
        "and the set must hold exactly one identity record"
    );
}

/// **What makes a record an identity is that it attributes itself, not that it carries the
/// keyword.** `identity-counts-test-the-keyword-not-the-record`: a memory that took `hoard-author`
/// as a subject keyword was counted as an identity, so `status` reported one memory fewer than the
/// set held and `export` named an ordinary memory among its `identities`. The rest of the store
/// asks [`Memory::is_identity_record`]; these are the sites that asked the keyword instead.
#[test]
fn a_memory_about_identity_records_counts_as_a_memory() {
    let mut store = a_store();
    store.insert(&an_author_record("jtcours")).expect("inserts");
    store
        .insert(&a_memory_keyed(
            "Identity records are never embedded.",
            &["hoard-author"],
        ))
        .expect("inserts");

    assert_eq!(store.count().expect("counts"), 1, "the memory is a memory");
    assert_eq!(
        store.identity_count().expect("counts"),
        1,
        "and only the self-attributed record is an identity"
    );
}

/// The same mistake at the gate on minting: a set holding nothing but a memory *about* identity
/// records read as already holding one, and would not mint the first.
#[test]
fn a_memory_about_identity_records_does_not_stop_the_first_mint() {
    let mut store = a_store();
    store
        .insert(&a_memory_keyed(
            "Identity records are never embedded.",
            &["hoard-author"],
        ))
        .expect("inserts");
    let at = Timestamp::from_str("2026-09-30T09:00:00Z").expect("valid timestamp");

    assert!(
        !store.holds_any_author_record().expect("reads"),
        "a memory carrying the keyword is not an identity record"
    );
    assert!(
        store
            .author_record_or_mint(&Author::new("jtcours").expect("valid author"), at)
            .expect("resolves")
            .is_some(),
        "so the set has no identity yet, and gets one"
    );
}

/// And at the refusal to mint a name already held: an ordinary memory whose text is a name is not
/// a record holding that name.
#[test]
fn a_memory_whose_text_is_a_name_does_not_hold_that_name() {
    let mut store = a_store();
    store
        .insert(&a_memory_keyed("jtcours", &["hoard-author"]))
        .expect("inserts");
    let at = Timestamp::from_str("2026-09-30T09:00:00Z").expect("valid timestamp");

    store
        .mint_author(&Author::new("jtcours").expect("valid author"), at)
        .expect("no identity record holds the name, so minting it is not a fork");
}

/// **A set that has only forgotten things has still been used**, so it cannot be made read-only
/// either: `drop` skips the safety copy for a read-only set on the ground that this machine never
/// wrote to it, and a tombstone is a write. *Holds a record* has to mean any record, not a live one.
#[test]
fn a_set_holding_only_tombstones_cannot_be_made_read_only() {
    let mut store = Store::open_in_memory(scope("scatterhoard")).expect("opens");
    let memory = a_memory("a memory that will not survive");
    store.insert(&memory).expect("inserts");
    store
        .forget(
            memory.id,
            MemoryId::generate(),
            Timestamp::from_str("2026-09-02T09:00:00Z").expect("valid timestamp"),
        )
        .expect("forgets");
    assert_eq!(store.count().expect("counts"), 0, "no live memories remain");

    assert!(
        store.choose_read_only().is_err(),
        "the tombstone is a record this machine wrote"
    );
}

/// **A mode is chosen at birth.** Making a set read-only once it holds records would let a set this
/// machine wrote to be dropped without a safety copy -- `documents/design/load-modes.md`.
#[test]
fn a_set_holding_records_cannot_be_made_read_only() {
    let mut store = Store::open_in_memory_and_mint_id(scope("used")).expect("opens");
    store
        .insert(&a_memory("alpha already here"))
        .expect("stores");

    assert!(store.choose_read_only().is_err());
    assert_eq!(
        store.load_mode().expect("reads"),
        scatterhoard::store::LoadMode::ReadWrite
    );
}

/// The store refuses a session's write itself, so no tool can forget to ask.
#[test]
fn a_read_only_store_refuses_a_session_write_and_takes_one_from_a_file() {
    let mut store = Store::open_in_memory_and_mint_id(scope("reference")).expect("opens");
    store
        .choose_read_only()
        .expect("an empty set can be made read-only");

    assert!(store.insert(&a_memory("alpha")).is_err());
    let arriving = a_memory("alpha from the publisher");
    store
        .accept_from_file(&arriving)
        .expect("a file is how a read-only set gets its records");
    assert!(store.get(arriving.id).expect("reads").is_some());
}

/// Every session entry point refuses, not only `insert`: a replacement carried forward and a forget
/// are writes too.
#[test]
fn a_read_only_store_refuses_forgetting_and_carrying_forward() {
    let mut store = Store::open_in_memory_and_mint_id(scope("reference")).expect("opens");
    store
        .choose_read_only()
        .expect("an empty set can be made read-only");
    let arriving = a_memory("alpha from the publisher");
    store
        .accept_from_file(&arriving)
        .expect("takes the file's record");

    assert!(store.carry_forward(&a_memory("alpha carried")).is_err());
    assert!(
        store
            .forget(arriving.id, arriving.id, Timestamp::now())
            .is_err()
    );
    assert!(store.get(arriving.id).expect("reads").is_some());
}

/// **The evaluation mode's policy**: no session writes, no watch marks, no safety copy owed. It
/// takes its records from a file like any read-only set -- `documents/design/load-modes.md`.
#[test]
fn an_evaluation_store_takes_records_from_a_file_and_refuses_a_session() {
    use scatterhoard::store::LoadMode;

    assert!(!LoadMode::Evaluation.accepts_session_writes());
    assert!(!LoadMode::Evaluation.owes_a_safety_copy());
    assert!(
        LoadMode::Evaluation.watch_marks().is_empty(),
        "a set nobody has adopted asks nothing of a session"
    );

    let named = SetName::evaluation(&SetName::new("source").expect("valid"), "issue-3")
        .expect("builds an evaluation name");
    let mut store = Store::open_in_memory_and_mint_id(Scope::Shared(named)).expect("opens");
    store
        .choose_evaluation()
        .expect("an empty store can be an evaluation load");
    assert_eq!(store.load_mode().expect("reads"), LoadMode::Evaluation);

    assert!(store.insert(&a_memory("alpha")).is_err());
    let arriving = a_memory("alpha as the contributor wrote it");
    store
        .accept_from_file(&arriving)
        .expect("takes the file's record");
    assert!(store.get(arriving.id).expect("reads").is_some());
}

/// **A stored link may not name a set loaded for review**, whether a session wrote it or it arrived
/// in a file: such a copy lives in one session's memory, so the link would dangle by the next start
/// while still reading as a reference to a set that exists.
#[test]
fn a_link_naming_an_evaluation_set_is_refused_from_either_door() {
    let mut store = a_store();
    let target = a_memory("alpha the target");
    let mut pointing = a_memory("alpha the memory that points");
    pointing.links = vec![Link {
        label: LinkLabel::RelatedTo,
        target: LinkTarget::CrossSet {
            set: SetName::evaluation(&SetName::new("scatterhoard").expect("valid"), "issue-3")
                .expect("builds"),
            id: target.id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];

    let authored = store
        .insert(&pointing)
        .expect_err("a session may not author one");
    assert!(
        authored.to_string().contains("eval-scatterhoard-issue-3"),
        "the refusal should name the set the link claimed: {authored}"
    );
    assert!(
        store.accept_from_file(&pointing).is_err(),
        "nor may one arrive in a file"
    );
}
