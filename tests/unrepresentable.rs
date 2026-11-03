// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! What a store does with text its model cannot represent.
//!
//! `one-unembeddable-memory-stops-the-store` in `TRACKING.md`. A memory whose embedded text holds no
//! token the model knows pools to a vector of zeros, which `Vector` refuses. Before this file, that
//! one memory made its store refuse every later write and every recall by meaning, took unscoped
//! recall down across every other set with it, and was put there by a write that reported failure
//! after committing it. It arrived three ways — a direct write, a write during a no-model gap, and
//! import — and each is pinned below.
//!
//! **What is being tested is the store, not a model**, so the fake here does to a marker word what
//! `potion-base-32M` does to Chinese: every piece is `[UNK]`, is dropped before pooling, and the
//! pooled vector is all zeros.

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity, embedded_from};
use scatterhoard::mcp::Server;
use scatterhoard::memory::{
    Attribution, Author, Confidence, Keyword, Memory, MemoryId, MemoryText, Origin, Scope, SetName,
};
use scatterhoard::memory_file::MemorySet;
use scatterhoard::store::{ModelState, Question, Store, recall_across};
use serde_json::{Value, json};

/// Text containing `zzz` is a script this model has no tokens for; text containing `fault` makes it
/// misbehave; everything else counts two marker words.
///
/// **Logs every text it is asked to embed**, so a test can see whether the store asks about a
/// memory again after the model has already said it cannot represent it.
#[derive(Debug, Default)]
struct ScriptEmbedder {
    asked: Mutex<Vec<String>>,
}

impl ScriptEmbedder {
    fn times_asked_about(&self, needle: &str) -> usize {
        self.asked
            .lock()
            .expect("unpoisoned")
            .iter()
            .filter(|text| text.contains(needle))
            .count()
    }

    fn forget_what_was_asked(&self) {
        self.asked.lock().expect("unpoisoned").clear();
    }
}

impl Embedder for ScriptEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("script-embedder")
    }

    fn dimension(&self) -> usize {
        3
    }

    /// Classified by [`embedded_from`], the function the real model's output goes through, so these
    /// tests exercise the same line between *nothing to represent* and *a broken model*.
    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        self.asked
            .lock()
            .expect("unpoisoned")
            .extend(texts.iter().cloned());
        texts
            .iter()
            .map(|text| {
                embedded_from(if text.contains("zzz") {
                    vec![0.0, 0.0, 0.0]
                } else if text.contains("fault") {
                    vec![f32::NAN, 0.0, 0.01]
                } else {
                    let count = |word: &str| text.matches(word).count() as f32;
                    vec![count("alpha"), count("beta"), 0.01]
                })
            })
            .collect()
    }
}

/// A different model, whose vocabulary does cover `zzz` — the multilingual model a user might
/// install once they find their language is not represented.
#[derive(Debug)]
struct WiderEmbedder;

impl Embedder for WiderEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("wider-embedder")
    }

    fn dimension(&self) -> usize {
        3
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|text| {
                let count = |word: &str| text.matches(word).count() as f32;
                embedded_from(vec![count("alpha"), count("zzz"), 0.01])
            })
            .collect()
    }
}

fn scope() -> Scope {
    Scope::Shared(SetName::new("scatterhoard").expect("valid set name"))
}

fn a_store() -> Store {
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    store.set_embedder(Arc::new(ScriptEmbedder::default()));
    store
}

/// Minimal scratch directory, as in `tests/concurrency.rs`, so one test does not add a dependency.
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

fn a_memory(content: &str) -> Memory {
    Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: Timestamp::from_str("2026-09-11T09:00:00Z").expect("valid timestamp"),
        },
    )
}

// ---------------------------------------------------------------------------
// The three routes in
// ---------------------------------------------------------------------------

/// A direct write. The memory is kept — keyword search and lookup still reach it — and it is
/// honestly not findable by meaning.
#[test]
fn a_memory_the_model_cannot_represent_is_stored_without_a_vector() {
    let mut store = a_store();
    let memory = a_memory("zzz zzz");

    store
        .insert(&memory)
        .expect("a memory the model cannot represent is still stored");

    assert!(store.get(memory.id).expect("reads").is_some());
    assert!(!store.is_findable_by_meaning(memory.id).expect("reads"));
}

#[test]
fn one_unrepresentable_memory_does_not_stop_later_writes() {
    let mut store = a_store();
    store.insert(&a_memory("zzz")).expect("stores");

    let later = a_memory("alpha");
    store
        .insert(&later)
        .expect("an ordinary write after it must still succeed");

    assert!(store.is_findable_by_meaning(later.id).expect("reads"));
}

/// A write during a no-model gap stores fine and used to poison the store the moment a model
/// arrived, because the sweep sent every gap through one batch and one refusal failed it.
///
/// **The genuine gap beside it must still be filled** — that is the sweep's whole job, and a fix that
/// stopped the sweep from failing by stopping it from embedding would pass the first assertion.
#[test]
fn a_gap_holding_an_unrepresentable_memory_is_still_filled_once_a_model_arrives() {
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    store
        .insert(&a_memory("zzz"))
        .expect("stores without a model");
    let genuine_gap = a_memory("alpha alpha");
    store.insert(&genuine_gap).expect("stores without a model");

    store.set_embedder(Arc::new(ScriptEmbedder::default()));

    let recall = store
        .recall(&Question::new("alpha", 5))
        .expect("recall must survive a memory the model cannot represent");
    assert!(
        recall
            .hits
            .iter()
            .any(|hit| hit.memory.id == genuine_gap.id),
        "the memory the model *can* represent must be embedded by the sweep and found"
    );
    store
        .insert(&a_memory("beta"))
        .expect("and writes must still work");
}

/// Import is the route that reaches collaborators: one such memory in a shared set used to stop the
/// store on every machine that imported it, after the import had already replaced the set.
#[test]
fn a_set_carrying_an_unrepresentable_memory_imports_whole() {
    let mut store = a_store();
    let unrepresentable = a_memory("zzz");
    let ordinary = a_memory("alpha");

    store
        .import(&MemorySet {
            scope: scope(),
            memories: vec![unrepresentable.clone(), ordinary.clone()],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("a set carrying a memory the model cannot represent still imports");

    assert_eq!(store.count().expect("counts"), 2);
    assert!(store.is_findable_by_meaning(ordinary.id).expect("reads"));
    assert!(
        !store
            .is_findable_by_meaning(unrepresentable.id)
            .expect("reads")
    );
    store
        .insert(&a_memory("beta"))
        .expect("and the store still takes writes");
}

// ---------------------------------------------------------------------------
// How far the damage reached
// ---------------------------------------------------------------------------

/// A poisoned store took down unscoped recall for every set searched with it, including sets that
/// held nothing wrong.
#[test]
fn an_unrepresentable_memory_in_one_set_does_not_stop_recall_across_the_others() {
    let mut poisoned = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    poisoned.insert(&a_memory("zzz")).expect("stores");
    poisoned.set_embedder(Arc::new(ScriptEmbedder::default()));

    let mut healthy = Store::open_in_memory_and_mint_id(Scope::Shared(
        SetName::new("other").expect("valid set name"),
    ))
    .expect("opens");
    healthy.set_embedder(Arc::new(ScriptEmbedder::default()));
    let ordinary = a_memory("alpha");
    healthy.insert(&ordinary).expect("stores");

    let recall = recall_across(
        &mut [&mut poisoned, &mut healthy],
        &Question::new("alpha", 5),
    )
    .expect("one set's unrepresentable memory must not fail recall across all of them");

    assert!(recall.hits.iter().any(|hit| hit.memory.id == ordinary.id));
}

/// A query in a script the model has no tokens for used to fail the whole recall.
#[test]
fn a_query_the_model_cannot_represent_is_answered_rather_than_failed() {
    let mut store = a_store();
    store.insert(&a_memory("alpha")).expect("stores");

    let recall = store
        .recall(&Question::new("zzz", 5))
        .expect("a query the model cannot represent is answered, not failed");

    assert!(recall.hits.is_empty());
    assert!(
        !recall.searched,
        "nothing was searched by meaning, and recall must not claim otherwise"
    );
    assert!(
        recall.query_unrepresentable,
        "and it must say the query is why, since no set is to blame"
    );
}

// ---------------------------------------------------------------------------
// What the fix must not break
// ---------------------------------------------------------------------------

/// **A non-finite vector is a broken model, not a property of the text**, so it stays an error —
/// treating it as merely unrepresentable would hide corruption. And the write it failed must not be
/// left behind, which is the other half of the defect: the row used to be committed before the
/// embedding was attempted.
#[test]
fn a_write_whose_embedding_faults_fails_and_leaves_nothing_behind() {
    let mut store = a_store();
    let memory = a_memory("fault");

    assert!(
        store.insert(&memory).is_err(),
        "a model producing a non-finite vector is broken, and that must stay an error"
    );
    assert!(
        store.get(memory.id).expect("reads").is_none(),
        "a write that reported failure must not have been committed"
    );
    store
        .insert(&a_memory("alpha"))
        .expect("and nothing was left behind to stop the next write");
}

/// Once the model has said it cannot represent a memory, asking again cannot give a different
/// answer: only a change of text or of model could, and each re-embeds through its own path. So the
/// sweep must not keep asking — for a store written mostly in a script its model lacks, that would
/// be re-embedding the whole store on every call.
#[test]
fn the_sweep_does_not_ask_again_about_a_memory_the_model_could_not_represent() {
    let embedder = Arc::new(ScriptEmbedder::default());
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    store.set_embedder(embedder.clone());
    store.insert(&a_memory("zzz")).expect("stores");
    embedder.forget_what_was_asked();

    store.insert(&a_memory("alpha")).expect("stores");
    store.recall(&Question::new("alpha", 5)).expect("recalls");
    store.recall(&Question::new("beta", 5)).expect("recalls");

    assert_eq!(
        embedder.times_asked_about("zzz"),
        0,
        "the sweep must treat the model's verdict as an answer, not as a gap"
    );
}

/// The verdict belongs to the model that gave it, so a model that *can* represent the memory has to
/// get its own chance — through the rebuild a model change already goes through.
#[test]
fn a_model_that_can_represent_the_memory_embeds_it_on_rebuild() {
    let mut store = a_store();
    let memory = a_memory("zzz");
    store.insert(&memory).expect("stores");

    store.set_embedder(Arc::new(WiderEmbedder));
    store.rebuild_vectors().expect("rebuilds");

    assert!(store.is_findable_by_meaning(memory.id).expect("reads"));
}

/// Changing the text is the other way the answer can change, and a rewrite re-embeds through `write`.
#[test]
fn rewriting_an_unrepresentable_memory_in_a_represented_script_embeds_it() {
    let mut store = a_store();
    let mut memory = a_memory("zzz");
    store.insert(&memory).expect("stores");

    memory.content = MemoryText::new("alpha").expect("valid content");
    store.insert(&memory).expect("rewrites");

    assert!(store.is_findable_by_meaning(memory.id).expect("reads"));
}

/// The in-memory set has to follow the verdict: a memory rewritten into a script the model cannot
/// represent must not go on being found by the vector its old text had.
#[test]
fn rewriting_a_memory_into_an_unrepresentable_script_stops_its_old_vector_matching() {
    let mut store = a_store();
    store.insert(&a_memory("beta")).expect("stores");
    let mut memory = a_memory("alpha alpha");
    store.insert(&memory).expect("stores");

    memory.content = MemoryText::new("zzz").expect("valid content");
    store.insert(&memory).expect("rewrites");

    let recall = store.recall(&Question::new("alpha", 5)).expect("recalls");
    assert!(
        recall.hits.iter().all(|hit| hit.memory.id != memory.id),
        "the old vector must not survive the rewrite"
    );
}

/// **A verdict has to survive the store being reopened**, which is when the vector table is read
/// back from disk. An empty blob decodes to no components, which `Vector` refuses, so a loader that
/// did not know about verdicts would fail to open any store holding one.
#[test]
fn a_store_holding_a_verdict_can_be_reopened() {
    let directory = tempdir();
    let path = directory.path().join("store.sqlite3");
    let ordinary = a_memory("alpha");
    let unrepresentable = a_memory("zzz");
    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(ScriptEmbedder::default()));
        store.insert(&ordinary).expect("stores");
        store.insert(&unrepresentable).expect("stores");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(ScriptEmbedder::default()));
    let recall = store
        .recall(&Question::new("alpha", 5))
        .expect("a store holding a verdict must load");

    assert!(recall.hits.iter().any(|hit| hit.memory.id == ordinary.id));
    assert!(
        !store
            .is_findable_by_meaning(unrepresentable.id)
            .expect("reads")
    );
}

/// A rebuild that leaves memories without vectors has to say how many, or seven memories rebuilding
/// to five vectors reads as two lost rather than two in a script the model does not cover.
#[test]
fn a_rebuild_reports_the_memories_its_model_cannot_represent() {
    let mut store = a_store();
    store.insert(&a_memory("alpha")).expect("stores");
    store.insert(&a_memory("zzz")).expect("stores");

    let rebuilt = store.rebuild_vectors().expect("rebuilds");

    assert_eq!(rebuilt.vectors, 1);
    assert_eq!(rebuilt.unrepresentable, 1);
}

/// **The verdict ties the store to the model that gave it**, exactly as a vector does. Otherwise a
/// store holding only unrepresentable memories would count as holding nothing, release its model
/// under *no vectors, no model* — and a different model arriving later would skip every one of them,
/// because their rows exist.
#[test]
fn a_store_holding_only_unrepresentable_memories_stays_tied_to_its_model() {
    let mut store = a_store();
    store.insert(&a_memory("zzz")).expect("stores");

    assert!(
        matches!(
            store.model_state().expect("reads"),
            ModelState::Current { .. }
        ),
        "the model's verdict binds the store to that model"
    );
}

/// The same, through the path that actually asks: a rebuild empties the table and then releases the
/// model if nothing is left, so a store of nothing but verdicts is exactly the case that decides
/// whether a verdict counts.
#[test]
fn rebuilding_a_store_of_only_unrepresentable_memories_keeps_it_tied_to_its_model() {
    let mut store = a_store();
    store.insert(&a_memory("zzz")).expect("stores");

    store.rebuild_vectors().expect("rebuilds");

    assert!(
        matches!(
            store.model_state().expect("reads"),
            ModelState::Current { .. }
        ),
        "a rebuild that produced only verdicts must not release the model that gave them"
    );
}

// ---------------------------------------------------------------------------
// What the caller is told
// ---------------------------------------------------------------------------

/// Give a shared store its charter, which its first memory has to be since
/// `set-settled-at-first-write`. The counting marker words are absent, so it sits nowhere near a query.
fn with_a_charter(mut store: Store) -> Store {
    // Named first, as a set's first write would: since `set-settled-at-first-write` a shared set
    // takes no write until someone has said who they are in it, and the charter is theirs.
    let author = store
        .author_record_or_mint(
            &Author::new("jtcours").expect("valid author"),
            jiff::Timestamp::now(),
        )
        .expect("records who writes here")
        .expect("the set holds no other identity record");
    let mut charter = a_memory("This set holds the fixtures these tests write.");
    charter.created.by = author;
    charter.modified.by = author;
    charter.keywords = vec![Keyword::new("hoard-scope-topics").expect("valid keyword")];
    store.insert(&charter).expect("takes its charter");
    store
}

fn a_server() -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(with_a_charter(a_store()));
    server
}

fn call(server: &mut Server, tool: &str, arguments: Value) -> Value {
    let raw = server
        .handle(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": tool, "arguments": arguments}
            })
            .to_string(),
        )
        .expect("a request with an id must be answered");
    serde_json::from_str(&raw).expect("the response should be JSON")
}

fn is_error(response: &Value) -> bool {
    response["result"]["isError"].as_bool() == Some(true)
}

fn payload(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a text result, got {response}"));
    serde_json::from_str(text).unwrap_or_else(|_| panic!("expected a JSON payload, got {text}"))
}

/// **The note must not suggest adding keywords.** Keywords are part of the embedded text, so a
/// keyword in a language the model knows would give the memory a vector built from that keyword
/// alone — not one representative of what it says. Nor is installing a model the remedy the no-model
/// note names: there is a model, and it does not cover this language.
#[test]
fn memorizing_an_unrepresentable_memory_says_the_model_cannot_represent_it() {
    let mut server = a_server();

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "zzz zzz",
            "why": "zzz",
            "confidence": 0.9,
            "origin": "llm"
        }),
    );

    assert!(
        !is_error(&response),
        "the memory must be stored: {response}"
    );
    let result = payload(&response);
    assert_eq!(result["findable_by_meaning"], json!(false));
    let note = result["note"].as_str().expect("a note says why");
    assert!(
        !note.contains("Add keywords"),
        "adding keywords would manufacture an unrepresentative vector: {note}"
    );
    assert!(
        !note.contains("installed"),
        "a model is installed; it does not cover this text: {note}"
    );
}

#[test]
fn recalling_with_an_unrepresentable_query_is_answered_rather_than_failed() {
    let mut server = a_server();

    let response = call(
        &mut server,
        "hoard_recall",
        json!({"query": "zzz", "detail": "summary"}),
    );

    assert!(!is_error(&response), "recall must answer: {response}");
    let note = payload(&response)["note"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        note.contains("keywords"),
        "the caller must be told keyword search still works: {response}"
    );
}

/// The no-model note keeps its own remedy. There the advice to add keywords is safe — nothing is
/// embedded until a model arrives, and then the content embeds too — and installing a model is
/// exactly what is missing.
#[test]
fn memorizing_without_a_model_still_says_a_model_is_needed() {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(with_a_charter(
        Store::open_in_memory_and_mint_id(scope()).expect("opens"),
    ));

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha",
            "why": "alpha",
            "confidence": 0.9,
            "origin": "llm"
        }),
    );

    assert!(
        !is_error(&response),
        "the memory must be stored: {response}"
    );
    let note = payload(&response)["note"]
        .as_str()
        .expect("a note says why")
        .to_owned();
    assert!(
        note.contains("installed"),
        "without a model, installing one is the remedy: {note}"
    );
}
