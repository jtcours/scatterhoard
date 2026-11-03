// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for two server processes sharing one store file.
//!
//! This is not a speculative configuration. A server is registered per checkout, `.mcp.json` lives
//! in the repository, and `SCATTERHOARD_HOME` defaults to a single path under `$HOME` — so a
//! developer with two checkouts open already has two processes on one `local` store. The proposed
//! `SCATTERHOARD_LOCAL` split in `documents/design/store-registry.md` makes that sharing deliberate
//! rather than incidental.
//!
//! Two separate failures live here and neither fixes the other. Locking is about whether a write
//! *waits*; cache coherence is about whether the reader can *see* what the other process wrote. A
//! store could be perfectly serialised and still serve vectors from before the other process
//! existed.
//!
//! Everything here uses two connections in one process rather than two operating-system processes.
//! SQLite draws no distinction — its locking and its `data_version` counter are per *connection* —
//! and what would be bought by forking is not worth a test that cannot run under a debugger.

use std::str::FromStr;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use jiff::Timestamp;
use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::mcp::Server;
use scatterhoard::memory::{
    Attribution, Author, Confidence, Memory, MemoryId, MemoryText, Origin, Scope, SetName,
};
use scatterhoard::store::{Question, Store};
use serde_json::{Value, json};

/// Counts three marker words, so similarity is exactly what the test wrote rather than whatever a
/// model happened to think. The trailing constant keeps a vector from ever being all zeros, which
/// `Vector` refuses.
#[derive(Debug)]
struct MarkerEmbedder;

impl Embedder for MarkerEmbedder {
    fn identity(&self) -> ModelIdentity {
        // One identity for every test double, because these tests are about behaviour that assumes
        // a consistent model. The one place two models have to disagree is tests/model_identity.rs.
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

fn scope() -> Scope {
    Scope::Shared(SetName::new("scatterhoard").expect("valid set name"))
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

/// Two stores on one file, each with the same model — the configuration two checkouts produce.
fn two_stores_on_one_file(directory: &TempDir) -> (Store, Store) {
    let path = directory.path().join("shared.sqlite3");
    let mut first = Store::open_at(scope(), &path).expect("opens");
    first
        .mint_set_id()
        .expect("mints an identity, as create does");
    let mut second = Store::open_at(scope(), &path).expect("opens");
    first.set_embedder(Arc::new(MarkerEmbedder));
    second.set_embedder(Arc::new(MarkerEmbedder));
    (first, second)
}

// ---------------------------------------------------------------------------
// A set's first identity, claimed from two connections at once
//
// `concurrent-first-writes-can-mint-two-identities`. Minting read the identity, found none, and
// wrote one with an upsert, so two first writes that both read *none* each wrote, and the later one
// won -- leaving the earlier writer's records stamped with an identity the set no longer held. The
// critical section is split into its steps and interleaved by hand across two connections, the
// classic way to test a race deterministically: both read, then both claim. What is proved is that
// the claim is atomic -- one SQL statement -- rather than a race run with threads and luck.
// ---------------------------------------------------------------------------

/// Two stores on one file that has no identity yet: the configuration in which a first write can
/// happen twice at once.
fn two_unclaimed_stores(directory: &TempDir) -> (Store, Store) {
    let path = directory.path().join("unclaimed.sqlite3");
    let first = Store::open_at(scope(), &path).expect("opens");
    let second = Store::open_at(scope(), &path).expect("opens");
    (first, second)
}

/// **Both read none, both claim; the first claim stands and the second is told it lost.**
#[test]
fn two_claims_after_two_empty_reads_agree_on_the_first() {
    let directory = tempdir();
    let (first, second) = two_unclaimed_stores(&directory);
    assert_eq!(first.set_id().expect("reads"), None);
    assert_eq!(second.set_id().expect("reads"), None);

    let mine = scatterhoard::memory::SetId::generate();
    let theirs = scatterhoard::memory::SetId::generate();
    assert_eq!(first.claim_set_id(mine).expect("claims"), mine);
    assert_eq!(
        second.claim_set_id(theirs).expect("claims"),
        mine,
        "the later claim must return the identity already held, not overwrite it"
    );
    assert_eq!(first.set_id().expect("reads"), Some(mine));
    assert_eq!(second.set_id().expect("reads"), Some(mine));
}

/// **An import adopting a file's identity does not displace one another connection minted**, and
/// says it did not take the offer -- which is what makes import's mismatch check refuse the file
/// rather than write records under an identity the set does not hold. Adoption has no read of its
/// own to interleave: it is the same single statement as the claim, so there is no window to race.
#[test]
fn an_adoption_after_another_connections_mint_does_not_displace_it() {
    let directory = tempdir();
    let (importing, writing) = two_unclaimed_stores(&directory);
    assert_eq!(importing.set_id().expect("reads"), None);

    let minted = writing.mint_set_id().expect("mints on a first write");
    let offered = scatterhoard::memory::SetId::generate();
    assert!(
        !importing.adopt_set_id(offered).expect("adopts"),
        "the set already holds an identity, so the offer was not taken"
    );
    assert_eq!(importing.set_id().expect("reads"), Some(minted));
}

// ---------------------------------------------------------------------------
// Locking
// ---------------------------------------------------------------------------

/// A store file must be left in WAL mode, so a reader is not blocked by a writer holding the
/// database.
///
/// Checked through a connection this code did not configure, because the question is what is true
/// of the *file*: `journal_mode` is persistent, so a second process opening the same store inherits
/// it without having to agree to it first. Asking the store's own connection would only prove that
/// whatever it set, it set.
#[test]
fn a_store_file_is_left_in_wal_mode() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");
    let _store = Store::open_at(scope(), &path).expect("opens");

    let plain = rusqlite::Connection::open(&path).expect("opens the same file");
    let mode: String = plain
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("reads the journal mode");

    assert_eq!(
        mode.to_lowercase(),
        "wal",
        "a shared store file must be in WAL mode, not the default rollback journal"
    );
}

/// A write that cannot take the lock must wait for it, not fail.
///
/// SQLite's own default `busy_timeout` is zero: a connection that cannot get its lock returns
/// `SQLITE_BUSY` immediately rather than waiting, which with two servers on one store turns a
/// momentary overlap into a hard error.
///
/// **This test passed before the store set a timeout at all**, and that is worth stating rather
/// than quietly enjoying. `rusqlite::Connection::open` sets five seconds of its own, so the waiting
/// was already happening — by way of a dependency's default, not by anything this project had
/// decided. The store now asks for it explicitly and this test guards the requirement rather than
/// proving a fix. `scripts/mutations.py` records the same finding from the other side: removing the
/// explicit timeout is a mutation that survives, with the reason written down.
///
/// The lock is held for a fixed short interval and the timeout is far longer, so this test does not
/// race: it fails only if the store refuses to wait at all, or if the machine stalls for seconds.
#[test]
fn a_write_waits_for_a_lock_another_connection_holds() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");
    let mut store = Store::open_at(scope(), &path).expect("opens");
    store
        .mint_set_id()
        .expect("mints an identity, as create does");

    let (locked, lock_taken) = mpsc::channel();
    let holder_path = path.clone();
    let holder = std::thread::spawn(move || {
        let connection = rusqlite::Connection::open(&holder_path).expect("opens the same file");
        // IMMEDIATE takes the write lock at BEGIN rather than at the first write, so the lock is
        // certainly held by the time this thread reports it.
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .expect("takes the write lock");
        locked.send(()).expect("the test is still listening");
        std::thread::sleep(Duration::from_millis(250));
        connection.execute_batch("COMMIT").expect("releases it");
    });

    lock_taken.recv().expect("the lock was taken");
    store
        .insert(&a_memory("alpha alpha"))
        .expect("a write must wait for the lock rather than fail with SQLITE_BUSY");

    holder.join().expect("the lock holder finished");
}

// ---------------------------------------------------------------------------
// Cache coherence
// ---------------------------------------------------------------------------

/// What one process stores, another process must be able to find by meaning.
///
/// The vector set is held in memory so that opening a store does not put the embedder on the
/// critical path. Nothing invalidated it when a *different connection* wrote, so a memory stored by
/// one server was invisible to the other's semantic recall until it restarted — while keyword
/// search, which reads SQL, found it. Two search paths disagreeing about what the store contains,
/// with nothing in either response saying so.
#[test]
fn a_memory_stored_by_another_connection_is_found_by_meaning() {
    let directory = tempdir();
    let (mut writer, mut reader) = two_stores_on_one_file(&directory);

    writer.insert(&a_memory("alpha alpha")).expect("inserts");

    let recall = reader.recall(&Question::new("alpha", 5)).expect("recalls");
    assert_eq!(
        recall.hits.len(),
        1,
        "a memory written by another connection must be reachable by meaning without a restart"
    );
    assert_eq!(recall.hits[0].memory.content.as_data(), "alpha alpha");
}

/// **A write through a stale cache loses nothing.** A write no longer brings the in-memory vectors
/// up to date first, since 2026-09-22 (`where-the-gap-sweep-runs`): everything it consults is read
/// from the database, and the next recall reloads if another connection committed meanwhile. This
/// is that case — the reader's cache is built, the other connection writes, then the reader writes
/// through its now-stale cache — and both memories must still be found by meaning.
#[test]
fn a_write_through_a_stale_cache_leaves_every_memory_findable() {
    let directory = tempdir();
    let (mut other, mut this) = two_stores_on_one_file(&directory);
    this.insert(&a_memory("gamma to build the cache"))
        .expect("inserts");
    this.recall(&Question::new("gamma", 5)).expect("recalls");

    other
        .insert(&a_memory("alpha from the other connection"))
        .expect("inserts");
    this.insert(&a_memory("alpha from this connection"))
        .expect("inserts through a stale cache");

    let recall = this.recall(&Question::new("alpha", 5)).expect("recalls");
    let found: Vec<&str> = recall
        .hits
        .iter()
        .map(|hit| hit.memory.content.as_data())
        .collect();
    assert!(
        found.contains(&"alpha from the other connection")
            && found.contains(&"alpha from this connection"),
        "both writes must be found by meaning: {found:?}"
    );
}

/// The same, in the other direction: what one process forgets must stop being recalled by another.
///
/// This is the half that matters for removal. A forgotten memory's vector is a copy of it — several
/// hundred floats reconstructed from the content — so a reader still holding it in memory has kept
/// the thing the deletion was for.
#[test]
fn a_memory_forgotten_by_another_connection_stops_being_recalled() {
    let directory = tempdir();
    let (mut writer, mut reader) = two_stores_on_one_file(&directory);

    let doomed = a_memory("alpha alpha");
    writer.insert(&doomed).expect("inserts");
    assert_eq!(
        reader
            .recall(&Question::new("alpha", 5))
            .expect("recalls")
            .hits
            .len(),
        1,
        "the reader must see it before it is forgotten, or this proves nothing"
    );

    writer
        .forget(
            doomed.id,
            MemoryId::generate(),
            Timestamp::from_str("2026-08-07T09:00:00Z").expect("valid timestamp"),
        )
        .expect("forgets");

    assert!(
        reader
            .recall(&Question::new("alpha", 5))
            .expect("recalls")
            .hits
            .is_empty(),
        "a memory forgotten by another connection must not still be scored against queries"
    );
}

/// The same thing again through the whole surface, because that is the configuration the defect is
/// actually about: two MCP servers, each registered by its own checkout, on one `local` store.
///
/// Worth its own test rather than being taken as implied by the one above. `hoard_recall` does not
/// call [`Store::recall`] — it goes through `recall_across`, which is a second reader of the same
/// cache — so a fix applied to one and not the other would leave every real call stale while the
/// direct test stayed green.
#[test]
fn one_servers_memory_is_recallable_from_another_server_on_the_same_store() {
    let directory = tempdir();
    let path = directory.path().join("local.sqlite3");

    let mut writer = a_server_on(&path);
    let mut reader = a_server_on(&path);

    let stored = call(
        &mut writer,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha alpha",
            "confidence": 0.8,
            "origin": "llm",
        }),
    );
    let id = payload(&stored)["id"]
        .as_str()
        .expect("the store reports an id")
        .to_owned();

    let found = payload(&call(
        &mut reader,
        "hoard_recall",
        json!({"query": "alpha", "limit": 5}),
    ));
    let ids: Vec<&str> = found["memories"]
        .as_array()
        .expect("a list of memories")
        .iter()
        .filter_map(|memory| memory["id"].as_str())
        .collect();

    assert!(
        ids.contains(&id.as_str()),
        "the other server's memory must be recallable without a restart, got {found}"
    );
}

/// Reading is not supposed to *cost* a reload when nothing has changed.
///
/// The check is `PRAGMA data_version`, which changes only when another connection has committed —
/// so a store that is alone with its file, or that did the last write itself, reloads nothing. This
/// test cannot see the reload directly; what it can see is that repeated recall keeps working and
/// keeps agreeing with itself, which is the property a mistaken invalidation would break.
#[test]
fn repeated_recall_without_another_writer_stays_stable() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");
    let mut store = Store::open_at(scope(), &path).expect("opens");
    store
        .mint_set_id()
        .expect("mints an identity, as create does");
    store.set_embedder(Arc::new(MarkerEmbedder));
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    let first = store.recall(&Question::new("alpha", 5)).expect("recalls");
    let second = store.recall(&Question::new("alpha", 5)).expect("recalls");

    assert_eq!(first.hits.len(), 1);
    assert_eq!(second.hits.len(), 1);
    assert_eq!(first.hits[0].memory.id, second.hits[0].memory.id);
}

// ---------------------------------------------------------------------------
// Attribution belongs to the store, not to the process
// ---------------------------------------------------------------------------

/// Two servers, two author identities, one `local` store: the same human's personal memories must
/// not carry attribution decided by which window they were written from.
///
/// Reproduced against two real server processes on 2026-08-10, before this test existed — one store
/// holding `jeff` on one memory and `jtcours@work` on the next. `local` is personal and
/// machine-wide, so this is not an exotic configuration; it is what two checkouts already are.
/// `documents/design/store-registry.md`, *the author belongs in the store, not in the environment*.
#[test]
fn a_second_server_attributes_to_the_store_rather_than_to_its_own_environment() {
    let directory = tempdir();
    let path = directory.path().join("local.sqlite3");

    let mut first = a_server_as(&path, "jeff");
    let mut second = a_server_as(&path, "jtcours@work");

    memorize(&mut first, "coffee before the standup");
    memorize(&mut second, "the standup moved to ten");

    assert_eq!(
        authors_in(&path),
        vec!["jeff", "jeff"],
        "the second server must write under the identity the store already holds, not its own"
    );
}

/// The other half, and the reason this is adoption rather than enforcement: a store nobody has
/// stamped takes the identity of whoever writes to it first.
///
/// Filling an *absent* author converges — every process agrees on the value it finds. Overwriting a
/// *present* one would not: each server would restamp on every write and the store would change
/// hands forever, which is the oscillation `documents/design/embeddings.md` records for the
/// embedding model. Same shape, same resolution.
#[test]
fn a_store_that_has_never_been_told_takes_the_author_of_its_first_writer() {
    let directory = tempdir();
    let path = directory.path().join("local.sqlite3");

    let mut second = a_server_as(&path, "jtcours@work");
    memorize(&mut second, "the standup moved to ten");

    assert_eq!(
        authors_in(&path),
        vec!["jtcours@work"],
        "an unclaimed store should adopt the author of the process that first writes to it"
    );

    let mut first = a_server_as(&path, "jeff");
    memorize(&mut first, "coffee before the standup");

    assert_eq!(
        authors_in(&path),
        vec!["jtcours@work", "jtcours@work"],
        "and must not change hands when a differently configured server arrives"
    );
}

/// Reading is not claiming. A server that only recalls must leave an unclaimed store unclaimed, or
/// the identity a store settles on becomes a race between two processes' startup.
#[test]
fn recall_does_not_claim_a_store_for_the_reader() {
    let directory = tempdir();
    let path = directory.path().join("local.sqlite3");

    let mut reader = a_server_as(&path, "jtcours@work");
    call(&mut reader, "hoard_recall", json!({"query": "alpha"}));

    let store = Store::open_at(Scope::Local, &path).expect("opens");
    assert_eq!(
        store.author().expect("reads the author"),
        None,
        "a read must not stamp the store with the reader's identity"
    );
}

// ---------------------------------------------------------------------------

/// A server whose local store is the file at `path` — one of the two a pair of checkouts produces.
fn a_server_on(path: &std::path::Path) -> Server {
    a_server_as(path, "jtcours")
}

/// The same, under a named identity: the per-process `SCATTERHOARD_AUTHOR` two checkouts can differ
/// on.
fn a_server_as(path: &std::path::Path, author: &str) -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new(author).expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.add_store(Store::open_at(Scope::Local, path).expect("opens"));
    server
}

fn memorize(server: &mut Server, content: &str) {
    let response = call(
        server,
        "hoard_memorize",
        json!({"scope": "local", "content": content, "confidence": 0.8, "origin": "llm"}),
    );
    assert!(
        response["result"]["isError"] != json!(true),
        "memorize should succeed, got {response}"
    );
}

/// Who the memories on disk are attributed to, read through a third connection so the assertion
/// does not depend on either server's view of its own write.
/// The names ordinary memories are attributed to, resolved through their author records.
///
/// **Resolved rather than read**, because `by` holds a reference now: the field is an id, and the
/// question these tests ask is which *name* the writes landed under. **Author records are skipped**
/// -- they are ordinary memories carrying a reserved keyword, so a set's own identity record would
/// otherwise count as something the caller wrote.
fn authors_in(path: &std::path::Path) -> Vec<String> {
    let store = Store::open_at(Scope::Local, path).expect("opens");
    let mut ids = Vec::new();
    store
        .stream_memories(|memory| {
            if !memory
                .keywords
                .iter()
                .any(|keyword| keyword.as_str() == scatterhoard::store::AUTHOR_KEYWORD)
            {
                ids.push(memory.created.by);
            }
            Ok(())
        })
        .expect("streams");
    let mut authors: Vec<String> = ids
        .into_iter()
        .map(|id| {
            store.author_name(id).expect("resolves").map_or_else(
                || "<unresolved>".to_owned(),
                |name| name.as_str().to_owned(),
            )
        })
        .collect();
    authors.sort();
    authors
}

fn call(server: &mut Server, tool: &str, arguments: Value) -> Value {
    let request = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": tool, "arguments": arguments}
    });
    let raw = server
        .handle(&request.to_string())
        .expect("a request with an id must be answered");
    serde_json::from_str(&raw).expect("the response should be JSON")
}

fn payload(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a text result, got {response}"));
    serde_json::from_str(text).expect("the tool payload should be JSON")
}

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
