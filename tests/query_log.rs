// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the query log: what a store records about its own recalls.
//!
//! The log exists to calibrate the non-match floor from the store's own traffic rather than from a
//! constant — `documents/design/recall-mechanisms.md`, *the floor is calibrated from a local query
//! log, and reads raw similarity*. Nothing reads it yet. It is built first because calibration
//! cannot begin before there is something to calibrate from, and because a log only answers
//! questions about the period it was running.
//!
//! Two properties here are not features and will not announce themselves if they break. **No query
//! text is recorded** — the score is what calibration needs and the text is the part that would
//! make a leak possible at all. And **a question is logged only when a session asked it**: the probe
//! suite scores several hundred deliberately off-domain queries through this same path, and a
//! default-on log would take them all as traffic.

use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::mcp::Server;
use scatterhoard::memory::{
    Attribution, Author, Confidence, Link, LinkLabel, LinkTarget, Memory, MemoryId, MemoryText,
    Origin, Scope, SetName,
};
use scatterhoard::store::{
    Floor, Question, SessionId, Stages, Store, StoreLayout, recall_across, recall_across_with,
};
use serde_json::{Value, json};

/// Counts three marker words, so similarity is exactly what the test wrote rather than whatever a
/// model happened to think. The trailing constant keeps a vector from ever being all zeros, which
/// `Vector` refuses.
#[path = "support/review_all.rs"]
mod review_all;

#[derive(Debug)]
struct MarkerEmbedder;

impl Embedder for MarkerEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("marker-embedder")
    }

    fn dimension(&self) -> usize {
        4
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|text| {
                let count = |word: &str| text.matches(word).count() as f32;
                Vector::new(vec![count("alpha"), count("beta"), count("gamma"), 0.01])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

/// Answers `Unrepresentable` for any text holding no marker, which is what a real model does for a
/// query written in a script it has no tokens for.
#[derive(Debug)]
struct PickyEmbedder;

impl Embedder for PickyEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("picky-embedder")
    }

    fn dimension(&self) -> usize {
        4
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|text| {
                let count = |word: &str| text.matches(word).count() as f32;
                if count("alpha") + count("beta") + count("gamma") == 0.0 {
                    return Ok(Embedded::Unrepresentable);
                }
                Vector::new(vec![count("alpha"), count("beta"), count("gamma"), 0.01])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

fn shared() -> Scope {
    Scope::Shared(SetName::new("scatterhoard").expect("valid set name"))
}

fn a_store(scope: Scope) -> Store {
    let mut store = Store::open_in_memory_and_mint_id(scope).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    store
}

/// A fixed instant, so a test asserts on a time it chose rather than on the clock.
fn asked_at() -> Timestamp {
    Timestamp::from_str("2026-09-11T10:00:00Z").expect("valid timestamp")
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

// ---------------------------------------------------------------------------
// What gets logged, and what does not
// ---------------------------------------------------------------------------

/// One row per store the query was actually scored against, because a floor belongs to one store
/// and one model: a score from another set is measured against another corpus and cannot calibrate
/// this one.
#[test]
fn a_sessions_question_is_logged_once_per_store_searched() {
    let mut local = a_store(Scope::Local);
    let mut project = a_store(shared());
    let here = a_memory("alpha alpha");
    let there = a_memory("alpha beta");
    local.insert(&here).expect("inserts");
    project.insert(&there).expect("inserts");

    recall_across(
        &mut [&mut local, &mut project],
        &Question::new("alpha", 5).in_a_session(false, asked_at(), SessionId::generate()),
    )
    .expect("recalls");

    let one = local.query_log().expect("reads the log");
    let other = project.query_log().expect("reads the log");
    assert_eq!(one.len(), 1);
    assert_eq!(other.len(), 1);
    // Each store's own, not the merged answer: a store's log is evidence about that store, and an
    // id from another set is a memory this one could never return.
    assert_eq!(one[0].returned, vec![here.id]);
    assert_eq!(other[0].returned, vec![there.id]);
}

/// An empty store answers nothing, which says where no floor belongs. Recording it as a query that
/// found nothing would be exactly the shape of the evidence a floor is calibrated against, and it
/// would be evidence about the store's emptiness rather than about the query.
#[test]
fn a_store_with_nothing_in_it_logs_nothing() {
    let mut empty = a_store(Scope::Local);
    let mut project = a_store(shared());
    project.insert(&a_memory("alpha alpha")).expect("inserts");

    recall_across(
        &mut [&mut empty, &mut project],
        &Question::new("alpha", 5).in_a_session(false, asked_at(), SessionId::generate()),
    )
    .expect("recalls");

    assert!(empty.query_log().expect("reads the log").is_empty());
    assert_eq!(project.query_log().expect("reads the log").len(), 1);
}

/// The probe suite puts several hundred deliberately off-domain queries through this same path per
/// run. A log that defaulted to on would take them all as traffic, and the evidence it exists to
/// gather would be measuring the measurement. Those probes work on copies of the live stores, which
/// keeps the originals clean today — but a copy carries the log forward, and nothing about the log
/// makes that arrangement hold.
#[test]
fn an_instrumented_question_is_not_logged() {
    let mut store = a_store(shared());
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    store.recall(&Question::new("alpha", 5)).expect("recalls");
    recall_across_with(
        &mut [&mut store],
        &Question::new("alpha", 5),
        Stages::default(),
    )
    .expect("recalls");

    assert!(
        store.query_log().expect("reads the log").is_empty(),
        "a question nobody asked in a session was logged"
    );
}

/// What the session actually received, so that a later use of one of these memories can label the
/// query answerable. An id it never saw could never be used, and would label nothing.
#[test]
fn the_log_records_the_ids_the_session_received() {
    let mut store = a_store(shared());
    let alpha = a_memory("alpha alpha alpha");
    let beta = a_memory("beta beta beta");
    store.insert(&alpha).expect("inserts");
    store.insert(&beta).expect("inserts");

    let recall = store
        .recall(&Question::new("alpha", 5).in_a_session(false, asked_at(), SessionId::generate()))
        .expect("recalls");
    let returned: Vec<MemoryId> = recall.hits.iter().map(|hit| hit.memory.id).collect();

    let log = store.query_log().expect("reads the log");
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].returned, returned);
}

/// **The one property that would make a copy of this file a disclosure.** The score is what
/// calibration needs; the text is what it does not.
///
/// Read with `SELECT *` through a second connection rather than through [`Store::query_log`], for
/// two reasons: what matters is what is *in the file*, not what an accessor chooses to show, and a
/// column added later is covered without anyone remembering this test.
#[test]
fn no_column_of_the_log_holds_the_query_text() {
    let home = tempdir();
    let file = home.path().join("store.sqlite3");
    let mut store = Store::open_at(shared(), &file).expect("opens");
    store
        .mint_set_id()
        .expect("mints an identity, as create does");
    store.set_embedder(Arc::new(MarkerEmbedder));
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    store
        .recall(&Question::new("alpha gamma secret", 5).in_a_session(
            false,
            asked_at(),
            SessionId::generate(),
        ))
        .expect("recalls");
    assert_eq!(store.query_log().expect("reads the log").len(), 1);

    let written = every_cell(&file, "query_log") + &every_cell(&file, "query_log_hits");
    for word in ["secret", "gamma", "alpha"] {
        assert!(
            !written.contains(word),
            "the query text reached the log: {word:?} appears in {written}"
        );
    }
}

/// Every value of every column of one table, as text. Deliberately schema-blind.
fn every_cell(file: &std::path::Path, table: &str) -> String {
    let connection = rusqlite::Connection::open(file).expect("opens the store file");
    let mut statement = connection
        .prepare(&format!("SELECT * FROM {table}"))
        .expect("the table exists");
    let columns = statement.column_count();
    let mut cells = String::new();
    let mut rows = statement.query([]).expect("queries");
    while let Some(row) = rows.next().expect("reads a row") {
        for column in 0..columns {
            let value: rusqlite::types::Value = row.get(column).expect("reads a cell");
            cells.push_str(&format!("{value:?} "));
        }
    }
    cells
}

/// The label a floor cannot suppress. A scoped query that scores below the floor is a contradiction
/// sitting in the log, and it is what lowers a floor set too high — so it has to be recorded at the
/// moment of asking, where it is an argument of the call rather than a consequence of the answer.
#[test]
fn a_scoped_question_is_recorded_as_scoped() {
    let mut store = a_store(shared());
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    store
        .recall(&Question::new("alpha", 5).in_a_session(true, asked_at(), SessionId::generate()))
        .expect("recalls");
    store
        .recall(&Question::new("alpha", 5).in_a_session(false, asked_at(), SessionId::generate()))
        .expect("recalls");

    let log = store.query_log().expect("reads the log");
    assert_eq!(log.len(), 2);
    assert!(log[0].scoped, "the scoped question was not recorded as one");
    assert!(!log[1].scoped, "an unscoped question was marked scoped");
}

/// **Raw, not centred, and the difference is the whole finding.** Centring removes the corpus
/// centroid, which is exactly the direction that marked a query as out of domain — so the centred
/// score cannot answer *does anything here answer this*. Ranking keeps the centred score; the log
/// carries both, so neither has to be chosen against the other.
#[test]
fn the_logged_top_score_is_raw_where_the_ranking_is_centred() {
    let mut store = a_store(shared());
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    store.insert(&a_memory("beta beta")).expect("inserts");

    // The query embeds *parallel* to the first memory's vector — three markers against two — so its
    // raw cosine is 1, while removing a centroid built over both memories moves the two apart. It
    // has to be parallel rather than identical: an identical vector stays identical after the same
    // centroid is subtracted from both sides, and the two scores would agree by construction.
    store
        .recall(&Question::new("alpha alpha alpha", 5).in_a_session(
            false,
            asked_at(),
            SessionId::generate(),
        ))
        .expect("recalls");

    let log = store.query_log().expect("reads the log");
    assert!(
        (log[0].raw_top - 1.0).abs() < 1e-5,
        "raw top score should be the uncentred cosine, got {}",
        log[0].raw_top
    );
    assert!(
        log[0].centred_top < log[0].raw_top,
        "centred {} and raw {} are the same number, so one of them is not what it says",
        log[0].centred_top,
        log[0].raw_top
    );
}

/// A floor silences by returning nothing, which is also how it destroys the evidence that it was
/// set too high. Recording the silencing is what leaves a trace of a query whose answer was thrown
/// away.
#[test]
fn a_floor_that_silences_a_store_is_recorded_as_such() {
    let mut store = a_store(shared());
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    store.insert(&a_memory("beta beta")).expect("inserts");

    let stages = Stages {
        floor: Floor::Fixed(2.0),
        ..Stages::default()
    };
    let recall = recall_across_with(
        &mut [&mut store],
        &Question::new("alpha", 5).in_a_session(false, asked_at(), SessionId::generate()),
        stages,
    )
    .expect("recalls");
    assert!(
        recall.hits.is_empty(),
        "the fixture floor let something past"
    );

    let log = store.query_log().expect("reads the log");
    assert_eq!(log.len(), 1);
    assert!(log[0].silenced, "a silenced store was not recorded as one");
    assert!(
        log[0].raw_top > 0.0,
        "the score is recorded before the floor, so it survives the silencing"
    );
    assert!(log[0].returned.is_empty());
}

/// A floor belongs to one store and one model: every set's floor moved 0.10 to 0.16 under a
/// different model. Scores from before a model change are not evidence about the one running now,
/// and only the recorded identity can tell them apart.
#[test]
fn the_log_records_which_model_scored_the_query() {
    let mut store = a_store(shared());
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    store
        .recall(&Question::new("alpha", 5).in_a_session(false, asked_at(), SessionId::generate()))
        .expect("recalls");

    let log = store.query_log().expect("reads the log");
    assert_eq!(log[0].model, ModelIdentity::new("marker-embedder"));
}

/// Nothing was scored, so there is nothing to calibrate from. A row here would be a query recorded
/// as having found nothing, which is exactly the shape of the evidence a floor is calibrated
/// against — and this one says nothing about the corpus at all.
#[test]
fn a_query_the_model_cannot_represent_is_not_logged() {
    let mut store = Store::open_in_memory_and_mint_id(shared()).expect("opens");
    store.set_embedder(Arc::new(PickyEmbedder));
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    let recall = store
        .recall(&Question::new("\u{4f60}\u{597d}", 5).in_a_session(
            false,
            asked_at(),
            SessionId::generate(),
        ))
        .expect("recalls");
    assert!(recall.query_unrepresentable);

    assert!(
        store.query_log().expect("reads the log").is_empty(),
        "a query that was never scored was logged anyway"
    );
}

/// A store with no model is not searched at all, so it has nothing to record either.
#[test]
fn a_store_that_could_not_be_searched_is_not_logged() {
    let mut store = Store::open_in_memory_and_mint_id(shared()).expect("opens");
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    store
        .recall(&Question::new("alpha", 5).in_a_session(false, asked_at(), SessionId::generate()))
        .expect("recalls");

    assert!(store.query_log().expect("reads the log").is_empty());
}

// ---------------------------------------------------------------------------
// Through the server, and never out of it
// ---------------------------------------------------------------------------

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

fn tempdir() -> TempDir {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_nanos();
    let mut path = std::env::temp_dir();
    path.push(format!(
        "scatterhoard-query-log-{nanos}-{:?}",
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

/// A file-backed local store, so the log can be read back through a second connection — the server
/// keeps its stores to itself, and reopening the file is how a test sees what a real session left
/// behind.
fn a_server(home: &TempDir, store_at: &std::path::Path) -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_at(Scope::Local, store_at).expect("opens"));
    server
}

fn call(server: &mut Server, tool: &str, arguments: Value) -> Value {
    if tool == "hoard_sets" {
        review_all::review_before_export(server, &arguments);
    }
    let request = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": tool, "arguments": arguments}
    });
    let raw = server
        .handle(&request.to_string())
        .expect("a request with an id must be answered");
    let response: Value = serde_json::from_str(&raw).expect("the response should be JSON");
    assert_ne!(
        response["result"]["isError"],
        json!(true),
        "{}",
        response["result"]["content"][0]["text"]
    );
    response
}

fn store_one(server: &mut Server, content: &str) {
    call(
        server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": content, "why": "a fixture",
            "confidence": 0.7, "origin": "llm"
        }),
    );
}

/// The wiring the rest of this file cannot see. Every other test here builds its own question; this
/// one pins that the tool surface builds a session's question, which is the only way a real recall
/// ever reaches the log.
#[test]
fn a_recall_through_the_server_is_logged() {
    let home = tempdir();
    let file = home.path().join("local.sqlite3");
    let mut server = a_server(&home, &file);
    store_one(&mut server, "alpha is worth remembering");

    call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "local"}),
    );

    let store = Store::open_at(Scope::Local, &file).expect("reopens");
    let log = store.query_log().expect("reads the log");
    assert_eq!(log.len(), 1, "the server's own recall was not logged");
    assert!(
        log[0].scoped,
        "the caller named a scope and the log did not record it"
    );
}

/// **Never published, structurally.** Export streams memories and tombstones; the log is neither,
/// so no rule has to be remembered for it to stay behind. Pinned by exporting either side of a
/// recall and comparing the bytes, which would catch the log reaching the file by any route at all.
#[test]
fn export_does_not_carry_the_query_log() {
    let home = tempdir();
    let before = tempdir();
    let after = tempdir();
    let file = home.path().join("local.sqlite3");
    let mut server = a_server(&home, &file);
    store_one(&mut server, "alpha is worth remembering");

    call(
        &mut server,
        "hoard_sets",
        json!({"action": "export", "scope": "local", "file": before.path().join("local.json")}),
    );
    call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "local"}),
    );
    call(
        &mut server,
        "hoard_sets",
        json!({"action": "export", "scope": "local", "file": after.path().join("local.json")}),
    );

    let name = "local.json";
    let first = std::fs::read(before.path().join(name)).expect("the first export");
    let second = std::fs::read(after.path().join(name)).expect("the second export");
    assert_eq!(
        String::from_utf8_lossy(&first),
        String::from_utf8_lossy(&second),
        "a recall changed what export writes"
    );
}

// ---------------------------------------------------------------------------
// The `used` label
// ---------------------------------------------------------------------------

/// **What a session did with the answer is what says the set could answer.** The label comes from
/// ordinary work rather than from anyone labelling anything: a memory the recall returned is looked
/// up, linked to, or passed back as already held.
#[test]
fn a_returned_memory_that_is_later_used_labels_the_query() {
    let mut store = a_store(shared());
    let alpha = a_memory("alpha alpha");
    store.insert(&alpha).expect("inserts");
    let session = SessionId::generate();

    store
        .recall(&Question::new("alpha", 1).in_a_session(false, asked_at(), session))
        .expect("recalls");
    assert!(!store.query_log().expect("reads the log")[0].used);

    store.note_used(alpha.id, session).expect("marks");

    assert!(
        store.query_log().expect("reads the log")[0].used,
        "using a returned memory did not label the query that returned it"
    );
}

/// **Bounded to one session, which is what keeps the label meaning anything.** A memory returned
/// months ago by a query it did not answer would otherwise be labelled by any later use of it, and
/// a label that saturates lowers the floor toward the constant it replaced.
#[test]
fn a_use_in_another_session_labels_nothing() {
    let mut store = a_store(shared());
    let alpha = a_memory("alpha alpha");
    store.insert(&alpha).expect("inserts");

    store
        .recall(&Question::new("alpha", 1).in_a_session(false, asked_at(), SessionId::generate()))
        .expect("recalls");
    store
        .note_used(alpha.id, SessionId::generate())
        .expect("marks");

    assert!(
        !store.query_log().expect("reads the log")[0].used,
        "a use in a different session labelled this one's query"
    );
}

/// The label belongs to the queries that actually returned the memory, not to the session.
#[test]
fn a_use_labels_only_the_queries_that_returned_that_memory() {
    let mut store = a_store(shared());
    let alpha = a_memory("alpha alpha alpha");
    let beta = a_memory("beta beta beta");
    store.insert(&alpha).expect("inserts");
    store.insert(&beta).expect("inserts");
    let session = SessionId::generate();

    store
        .recall(&Question::new("alpha", 1).in_a_session(false, asked_at(), session))
        .expect("recalls");
    store
        .recall(&Question::new("beta", 1).in_a_session(false, asked_at(), session))
        .expect("recalls");
    store.note_used(alpha.id, session).expect("marks");

    let log = store.query_log().expect("reads the log");
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].returned, vec![alpha.id]);
    assert_eq!(log[1].returned, vec![beta.id]);
    assert!(log[0].used);
    assert!(!log[1].used, "a query that did not return it was labelled");
}

/// A use cannot speak for a question asked after it. The label says *this answer was used*, and an
/// answer that did not exist yet was not.
#[test]
fn a_query_asked_after_the_use_is_not_labelled_by_it() {
    let mut store = a_store(shared());
    let alpha = a_memory("alpha alpha");
    store.insert(&alpha).expect("inserts");
    let session = SessionId::generate();

    store
        .recall(&Question::new("alpha", 1).in_a_session(false, asked_at(), session))
        .expect("recalls");
    store.note_used(alpha.id, session).expect("marks");
    store
        .recall(&Question::new("alpha", 1).in_a_session(false, asked_at(), session))
        .expect("recalls");

    let log = store.query_log().expect("reads the log");
    assert!(log[0].used);
    assert!(!log[1].used, "a use labelled a query asked after it");
}

// ---------------------------------------------------------------------------
// The three signals the tool surface produces
// ---------------------------------------------------------------------------

/// Read the local store's log after driving the server, which is the only way to see what a real
/// session left behind — the server keeps its stores to itself.
fn log_of(file: &std::path::Path) -> Vec<scatterhoard::store::LoggedQuery> {
    Store::open_at(Scope::Local, file)
        .expect("reopens")
        .query_log()
        .expect("reads the log")
}

fn recall_alpha(server: &mut Server) -> String {
    let response = call(
        server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "local"}),
    );
    let payload: Value = serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("a text result"),
    )
    .expect("JSON");
    payload["memories"][0]["id"]
        .as_str()
        .expect("a returned id")
        .to_owned()
}

#[test]
fn looking_a_memory_up_labels_the_recall_that_returned_it() {
    let home = tempdir();
    let file = home.path().join("local.sqlite3");
    let mut server = a_server(&home, &file);
    store_one(&mut server, "alpha is worth remembering");
    let id = recall_alpha(&mut server);

    call(&mut server, "hoard_lookup", json!({"id": id}));

    assert!(
        log_of(&file)[0].used,
        "a lookup of a returned memory did not label the recall"
    );
}

/// A link is the strongest of the three: the session did not merely read the memory, it wrote
/// something that refers to it.
#[test]
fn linking_to_a_memory_labels_the_recall_that_returned_it() {
    let home = tempdir();
    let file = home.path().join("local.sqlite3");
    let mut server = a_server(&home, &file);
    store_one(&mut server, "alpha is worth remembering");
    let id = recall_alpha(&mut server);

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": "alpha again, with a reference", "why": "a fixture",
            "confidence": 0.7, "origin": "llm",
            "links": [{"label": "related_to", "target": id}]
        }),
    );

    assert!(
        log_of(&file)[0].used,
        "linking to a returned memory did not label the recall"
    );
}

/// Passing a memory back as already held says the session is carrying it, which is the same claim
/// the other two make and the one that arrives without a second tool call.
#[test]
fn passing_a_memory_back_as_already_known_labels_the_recall_that_returned_it() {
    let home = tempdir();
    let file = home.path().join("local.sqlite3");
    let mut server = a_server(&home, &file);
    store_one(&mut server, "alpha is worth remembering");
    let id = recall_alpha(&mut server);

    call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "local", "already_known": [id]}),
    );

    let log = log_of(&file);
    assert!(
        log[0].used,
        "passing a memory back as already known did not label the recall that returned it"
    );
    assert!(
        !log[1].used,
        "the query carrying already_known labelled itself"
    );
}

/// **Correcting a memory counts as using it**, reversed 2026-09-12 at the human partner's request.
///
/// It was excluded because the threshold of about forty labelled queries had been measured against
/// three signals and a fourth would have changed what a labelled query means without re-running that
/// measurement. That objection died with the rule: the level is moving to a percentile over a
/// bounded window, so every constant is being re-derived anyway, which makes this the cheapest
/// moment there will be to widen the signal. And it is the signal for the behaviour `recall-when`'s
/// connect-on-recall clause actually asks for — going back to an existing memory and linking it —
/// which was measured at 1 of 7 corrections since the clause shipped, and was invisible here.
#[test]
fn correcting_a_memory_counts_as_using_it() {
    let home = tempdir();
    let file = home.path().join("local.sqlite3");
    let mut server = a_server(&home, &file);
    store_one(&mut server, "alpha is worth remembering");
    let id = recall_alpha(&mut server);

    call(
        &mut server,
        "hoard_correct",
        json!({"action": "amend", "scope": "local", "id": id, "confidence": 0.9}),
    );

    assert!(
        log_of(&file)[0].used,
        "correcting a returned memory did not label the recall that found it"
    );
}

/// **A session begins when the conventions are loaded, not when the server starts.** Measured
/// 2026-09-11: the servers on this machine had been running two hours before this session's
/// transcript was created, and two other sessions had written to their transcripts in between — so
/// a process outlives a session and cannot stand in for one. `hoard_conventions` is the marker the
/// project already mandates as a session's first action.
#[test]
fn loading_the_conventions_begins_a_new_session() {
    let home = tempdir();
    let file = home.path().join("local.sqlite3");
    let mut server = a_server(&home, &file);
    store_one(&mut server, "alpha is worth remembering");
    let id = recall_alpha(&mut server);

    call(&mut server, "hoard_conventions", json!({}));
    call(&mut server, "hoard_lookup", json!({"id": id}));

    assert!(
        !log_of(&file)[0].used,
        "a use after the conventions were reloaded labelled the previous session's query"
    );
}

/// Paging is one load, not several sessions. A cursor says *continue*, and splitting a session there
/// would drop labels for every query asked before the catalog finished arriving.
#[test]
fn paging_through_the_conventions_does_not_begin_a_new_session() {
    let home = tempdir();
    let file = home.path().join("local.sqlite3");
    let mut server = a_server(&home, &file);
    store_one(&mut server, "alpha is worth remembering");
    let id = recall_alpha(&mut server);

    call(&mut server, "hoard_conventions", json!({"cursor": 1}));
    call(&mut server, "hoard_lookup", json!({"id": id}));

    assert!(
        log_of(&file)[0].used,
        "paging through the catalog was treated as the start of a new session"
    );
}

/// **A substituted hit hands the caller two ids, so the log records both.**
///
/// When a query matches a memory that has since been superseded, recall returns the *tip* and names
/// the member that matched as `replaces_matched`. The caller receives both — and `hoard_lookup` is
/// the documented way to read history, returning a superseded memory as itself rather than
/// substituting — so looking up the older member is acting on what this recall surfaced. Recording
/// only the tip made that invisible, on roughly 10% of returned hits.
#[test]
fn using_the_superseded_member_of_a_substituted_hit_labels_the_query() {
    let mut store = a_store(shared());
    let old = a_memory("alpha alpha alpha");
    let mut new = a_memory("beta beta");
    new.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(old.id),
    }];
    store.insert(&old).expect("inserts");
    store.insert(&new).expect("inserts");
    let session = SessionId::generate();

    let recall = store
        .recall(&Question::new("alpha", 1).in_a_session(false, asked_at(), session))
        .expect("recalls");
    assert_eq!(
        recall.hits[0].memory.id, new.id,
        "the fixture should substitute the replacement in"
    );
    assert_eq!(recall.hits[0].matched, Some(old.id));

    // The id the caller would take from `replaces_matched`, to read what was claimed before.
    store.note_used(old.id, session).expect("marks");

    let log = store.query_log().expect("reads the log");
    assert!(
        log[0].used,
        "using the superseded member the recall named did not label the query"
    );
    assert!(
        log[0].returned.contains(&new.id) && log[0].returned.contains(&old.id),
        "the log should hold both ids the caller received, and holds {:?}",
        log[0].returned
    );
}
