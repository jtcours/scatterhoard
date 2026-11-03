// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Which model a store's vectors came from, and what happens when that is not the model in hand.
//!
//! `documents/design/embeddings.md` resolves that a store holds one model's vectors or none. The
//! distinction that matters is not *has vectors* versus *has none* — it is whether this process can
//! maintain what the store already holds, which is a comparison between two identities and needs
//! one of them written down.
//!
//! Nothing here loads a real model. Two fake embedders that disagree about their identity are
//! enough to exercise every state, and they make the *identity* the thing under test rather than
//! whatever a model file happens to hash to.

use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::memory::{
    Attribution, Confidence, Memory, MemoryId, MemoryText, Origin, Scope, SetName,
};
use scatterhoard::memory_file::MemorySet;
use scatterhoard::store::{ModelState, Question, Store};

/// Two of these with different names stand in for two different models.
#[derive(Debug)]
struct NamedEmbedder(&'static str);

impl Embedder for NamedEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new(self.0)
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

// ---------------------------------------------------------------------------
// The three states
// ---------------------------------------------------------------------------

/// A store nothing has ever embedded is not in a broken state — it is in a complete one.
///
/// This is the row of the design's table where writes stay allowed: keyword search is the search,
/// and there are no vectors for a write to invalidate.
#[test]
fn a_store_that_never_embedded_anything_records_no_model() {
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Unembedded
    );
}

/// Holding an embedder is not the same as having embedded anything, and the store records the
/// model that actually produced a vector rather than the one it was handed.
#[test]
fn a_store_records_the_model_that_embedded_its_first_memory() {
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Unembedded,
        "an embedder that has embedded nothing has not made this a model's store yet"
    );

    store.insert(&a_memory("alpha alpha")).expect("inserts");

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Current {
            model: ModelIdentity::new("potion-base-32M")
        },
        "the write that produced a vector is what binds the store to a model"
    );
}

/// Reopening with the same model is the ordinary case and must be silent.
#[test]
fn reopening_with_the_same_model_is_current() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Current {
            model: ModelIdentity::new("potion-base-32M")
        }
    );
}

/// The case with no signal today: the model file at the configured path is a different model.
///
/// Cosine similarity is perfectly computable between two vector spaces, so nothing fails and every
/// answer is confidently wrong. Detecting it requires having written down which model produced
/// what is already stored.
#[test]
fn reopening_with_a_different_model_is_detected() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("something-else")));

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Mismatched {
            recorded: ModelIdentity::new("potion-base-32M"),
            loaded: Some(ModelIdentity::new("something-else")),
        }
    );
}

/// And the case that reaches this machine today: vectors on disk, no model in the process.
///
/// Reported as the same state as a wrong model, because the consequence is the same — this process
/// cannot maintain what the store holds. Kept distinguishable in the payload so the message can be
/// specific about which of the two happened.
#[test]
fn reopening_with_no_model_at_all_is_detected_and_says_which_case_it_is() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }

    let store = Store::open_at(scope(), &path).expect("reopens");

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Mismatched {
            recorded: ModelIdentity::new("potion-base-32M"),
            loaded: None,
        }
    );
}

// ---------------------------------------------------------------------------
// Resolving a mismatch, and filling a gap
// ---------------------------------------------------------------------------

/// A changed model resets every vector — on request, rather than on the next read.
///
/// **This test asserted the opposite until 2026-08-09**, and the decision it encoded was not
/// abandoned but narrowed. The instruction it recorded — *no store ever holds vectors from two
/// models, so the way out of a mismatch is to re-embed, not to stop* — still holds, and is still
/// what this asserts: the store ends up wholly re-embedded and both memories are findable. What
/// moved is the **trigger**. Rebuilding on read let two servers with different models each undo the
/// other's work on every recall, indefinitely, which was measured rather than predicted. The
/// argument for doing it automatically was that nobody holds information justifying a half-embedded
/// store; with two servers, the other one's configuration is exactly that information, and neither
/// process has it. See `documents/design/embeddings.md`.
#[test]
fn a_changed_model_re_embeds_the_whole_store() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");
    let first = a_memory("alpha alpha");
    let second = a_memory("beta beta");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&first).expect("inserts");
        store.insert(&second).expect("inserts");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("something-else")));

    // Reading no longer triggers it -- that is the change. A recall against a mismatched store
    // reports semantic search as unavailable and leaves the vectors alone.
    store.recall(&Question::new("alpha", 5)).expect("recalls");
    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Mismatched {
            recorded: ModelIdentity::new("potion-base-32M"),
            loaded: Some(ModelIdentity::new("something-else")),
        },
        "a read must not take a turn in someone else's tug-of-war"
    );

    store.rebuild_vectors().expect("rebuilds when asked");

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Current {
            model: ModelIdentity::new("something-else")
        },
        "after the sweep the store belongs to the model in hand"
    );
    assert!(store.is_findable_by_meaning(first.id).expect("checks"));
    assert!(
        store.is_findable_by_meaning(second.id).expect("checks"),
        "every memory is re-embedded, not just the one the query touched"
    );
}

/// The other half of the same mechanism: memories written before a model arrived get one.
///
/// This is the ordinary onboarding path rather than a repair — setup is a delegated later step, so
/// *store first, gain a model afterwards* is what a normal installation does.
#[test]
fn memories_stored_before_a_model_arrived_are_embedded_when_it_does() {
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    let early = a_memory("alpha alpha");
    store.insert(&early).expect("inserts");
    assert!(
        !store.is_findable_by_meaning(early.id).expect("checks"),
        "nothing could have embedded it yet"
    );

    store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
    store.recall(&Question::new("alpha", 5)).expect("recalls");

    assert!(
        store.is_findable_by_meaning(early.id).expect("checks"),
        "the gap must be filled without anyone asking for it"
    );
    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Current {
            model: ModelIdentity::new("potion-base-32M")
        }
    );
}

/// A sweep that finds nothing to do must not disturb what is already there.
#[test]
fn a_store_with_no_gap_is_left_alone() {
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
    let memory = a_memory("alpha alpha");
    store.insert(&memory).expect("inserts");

    let before = store.recall(&Question::new("alpha", 5)).expect("recalls");
    let after = store.recall(&Question::new("alpha", 5)).expect("recalls");

    assert_eq!(before.hits.len(), 1);
    assert_eq!(after.hits.len(), 1);
    assert_eq!(after.hits[0].memory.id, memory.id);
    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Current {
            model: ModelIdentity::new("potion-base-32M")
        }
    );
}

// ---------------------------------------------------------------------------
// Refusing only what cannot be rebuilt
// ---------------------------------------------------------------------------

/// Rewriting an embedded memory with no model destroys its vector, so it is refused.
///
/// The write itself looks harmless — `insert` upserts, and upserting discards the vector on the way
/// through. With a model that is invisible because a new one is produced immediately; without one
/// there is nothing to produce it with, and the memory silently leaves semantic recall for good.
#[test]
fn rewriting_an_embedded_memory_without_a_model_is_refused() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");
    let mut memory = a_memory("alpha alpha");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&memory).expect("inserts");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    memory.context = Some(MemoryText::new("an amendment").expect("valid"));

    let refused = store.insert(&memory);

    assert!(
        refused.is_err(),
        "a write that would destroy an embedding this process cannot rebuild must be refused"
    );
    assert!(
        store.is_findable_by_meaning(memory.id).expect("checks"),
        "and the refusal must leave the vector where it was"
    );
}

/// The refusal is narrow on purpose: a *new* memory has no vector to lose.
///
/// A store whose model is missing has to stay usable — memories still store, keyword search and
/// lookup still find them, and the write reports that it is not findable by meaning. Refusing here
/// would turn a missing model into a read-only store, which is not what the degradation rule asks
/// for.
#[test]
fn storing_a_new_memory_without_a_model_is_still_allowed() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    let fresh = a_memory("beta beta");

    store
        .insert(&fresh)
        .expect("a new memory destroys no vector and must still be storable");
    assert!(
        !store.is_findable_by_meaning(fresh.id).expect("checks"),
        "it is simply unembedded, which the write surface already reports"
    );
}

/// And the same rewrite is fine once a model is back, because the vector can be rebuilt.
#[test]
fn rewriting_an_embedded_memory_is_allowed_when_a_model_can_rebuild_it() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");
    let mut memory = a_memory("alpha alpha");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&memory).expect("inserts");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
    memory.context = Some(MemoryText::new("an amendment").expect("valid"));

    store.insert(&memory).expect("rewrites");
    assert!(store.is_findable_by_meaning(memory.id).expect("checks"));
}

// ---------------------------------------------------------------------------
// Identity of a real model file
// ---------------------------------------------------------------------------

/// A model's identity has to come from its *contents*, because the failure being guarded against is
/// a different model arriving at the same configured path — where the path, the filename and the
/// directory all stay identical and only the bytes change.
#[test]
fn a_models_identity_follows_its_contents_not_its_path() {
    let directory = tempdir();
    let first = directory.path().join("model-a.bin");
    let second = directory.path().join("model-b.bin");
    std::fs::write(&first, b"the weights of one model").expect("writes");
    std::fs::write(&second, b"the weights of one model").expect("writes");

    assert_eq!(
        scatterhoard::embedder::identity_of_file(&first).expect("hashes"),
        scatterhoard::embedder::identity_of_file(&second).expect("hashes"),
        "two paths holding identical bytes are the same model"
    );

    std::fs::write(&second, b"the weights of another model").expect("rewrites");
    assert_ne!(
        scatterhoard::embedder::identity_of_file(&first).expect("hashes"),
        scatterhoard::embedder::identity_of_file(&second).expect("hashes"),
        "one path whose bytes changed is a different model"
    );
}

/// The vectors depend on *how* the text is fed to the model as well as on the model's bytes, so an
/// identity made of the bytes alone cannot tell a store encoded one way from a store encoded
/// another. Changing the encoding without changing the identity is the same failure as changing the
/// model without recording it — `no-record-of-which-model-built-the-vectors` — except that the
/// weights hash makes it look accounted for.
#[test]
fn an_identity_records_how_the_text_was_encoded_and_not_only_the_model() {
    let weights = ModelIdentity::new("fnv1a64:0123456789abcdef");
    let recorded = scatterhoard::embedder::identity_with_encoding(&weights);

    assert_ne!(
        recorded, weights,
        "a store encoded whole must not compare equal to one encoded truncated"
    );
    assert!(
        recorded.as_str().starts_with(weights.as_str()),
        "the weights must still be readable in the identity, so a human comparing two can see \
         that the model is the same and only the encoding differs: got {}",
        recorded.as_str()
    );
}

// ---------------------------------------------------------------------------

fn tempdir() -> TempDir {
    let mut path = std::env::temp_dir();
    path.push(format!("scatterhoard-model-{}", MemoryId::generate()));
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

/// A store written before model identity existed holds vectors nothing can account for.
///
/// Observed 2026-08-09 on the real stores: 26 and 6 vectors, no `embedding_model` row, because
/// `embed_all` is the only writer of that key and it is never reached when no memory lacks a
/// vector. Reporting that as [`ModelState::Unembedded`] is not a near-miss — it is the opposite
/// claim, and the variant's own documentation says *"there are no vectors for a write to
/// invalidate"* while 26 sit there. Everything downstream then reasons from it: the mismatch
/// detector cannot fire, so two servers with different models each run semantic recall against
/// vectors from a third, and the protections for a store whose model is missing do not apply.
#[test]
fn a_store_with_vectors_but_no_recorded_model_is_not_reported_unembedded() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }
    forget_which_model_embedded_this(&path);

    let store = Store::open_at(scope(), &path).expect("reopens");

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::ProvenanceUnknown { vectors: 1 },
        "a store holding vectors it cannot account for is not an unembedded store"
    );
}

/// Vectors nothing can account for are rebuilt, not adopted.
///
/// Adopting them means writing this model's identity over vectors it did not produce, which makes
/// a guess permanent and unverifiable. Rebuilding costs a re-embed — roughly 18,000 memories/sec,
/// so sub-second for any realistic store — and buys a provenance that is a fact.
///
/// **The claim is unchanged and the trigger moved**, 2026-08-20. A read used to do this; it now
/// happens only when a caller asks, because the state has no live producer and a permanent
/// read-that-writes is the wrong price for a population that can only shrink. That a read leaves
/// these vectors alone is asserted by `a_recall_does_not_rebuild_vectors_of_unknown_provenance`.
#[test]
fn vectors_of_unknown_provenance_are_rebuilt_rather_than_adopted() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }
    forget_which_model_embedded_this(&path);

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("potion-retrieval-32M")));
    // Asking is the trigger, as it is for a model change.
    store.rebuild_vectors().expect("rebuilds");

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Current {
            model: ModelIdentity::new("potion-retrieval-32M")
        },
        "the store should now be accounted for by the model that actually embedded it"
    );
    assert_eq!(
        recorded_model(&path),
        Some("potion-retrieval-32M".to_owned()),
        "the rebuilt vectors belong to the model in hand, not the one that was guessed at"
    );
}

/// With nothing to rebuild with, the vectors stay and the store says it cannot account for them.
///
/// The opposite of the case above and for the same reason as `M`/none: discarding here would throw
/// away the only vectors in existence to resolve a bookkeeping question, leaving the store worse
/// off than the ambiguity did.
#[test]
fn vectors_of_unknown_provenance_are_kept_when_there_is_no_model_to_rebuild_with() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }
    forget_which_model_embedded_this(&path);

    let store = Store::open_at(scope(), &path).expect("reopens");

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::ProvenanceUnknown { vectors: 1 },
        "nothing can rebuild these, so they are kept and the state stays honest"
    );
    assert_eq!(
        vector_count(&path),
        1,
        "the vectors are the only copy in existence and must survive being unaccounted for"
    );
}

/// Strip the `embedding_model` row, reproducing a store written before identity was recorded.
fn forget_which_model_embedded_this(path: &std::path::Path) {
    let connection = rusqlite::Connection::open(path).expect("opens the database directly");
    connection
        .execute("DELETE FROM meta WHERE key = 'embedding_model'", [])
        .expect("removes the recorded model");
}

fn vector_count(path: &std::path::Path) -> i64 {
    let connection = rusqlite::Connection::open(path).expect("opens the database directly");
    connection
        .query_row("SELECT count(*) FROM vectors", [], |row| row.get(0))
        .expect("counts the vectors")
}

fn recorded_model(path: &std::path::Path) -> Option<String> {
    let connection = rusqlite::Connection::open(path).expect("opens the database directly");
    connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'embedding_model'",
            [],
            |row| row.get(0),
        )
        .ok()
}

// ---------------------------------------------------------------------------
// A mismatch stops instead of resolving itself
// ---------------------------------------------------------------------------

/// Reading a store whose model is not the one in hand refuses rather than rebuilding it.
///
/// Replaces the automatic sweep, on evidence rather than by preference. Two servers with different
/// models each rebuilt the store on the other's turn, indefinitely, and a `hoard_recall` was enough
/// to trigger it — measured 2026-08-09. The design's own justification for automatic repair assumed
/// a single decider; the other server's configuration is exactly the information neither process
/// holds. See `documents/design/embeddings.md`.
#[test]
fn reading_a_mismatched_store_does_not_rebuild_it() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("something-else")));
    let recall = store
        .recall(&Question::new("alpha", 5))
        .expect("recall still answers");

    // Found by mutation: removing the guard in `Store::recall` left every other assertion here
    // passing, because they all describe what the store *is* rather than what it returned. The
    // store is scanned or it is not, and only the hits say which.
    assert!(
        recall.hits.is_empty(),
        "a mismatched store is not scanned at all -- a ranking across two spaces is worse than none"
    );
    assert!(
        !recall.searched,
        "the only store was mismatched, so the semantic path never ran"
    );

    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Mismatched {
            recorded: ModelIdentity::new("potion-base-32M"),
            loaded: Some(ModelIdentity::new("something-else")),
        },
        "the store still belongs to the model that embedded it"
    );
    assert!(
        !store.can_search_by_meaning().expect("reports capability"),
        "and it says search by meaning is unavailable rather than answering across two spaces"
    );
}

/// A new memory in a mismatched store arrives unembedded rather than being refused.
///
/// The same narrowing the design already applies to a missing model: a new memory destroys nothing.
/// It must not be embedded, because a vector from this process's model beside vectors from the
/// store's is the mixture *one store, one model* forbids — so it waits for an explicit rebuild.
#[test]
fn a_new_memory_in_a_mismatched_store_is_stored_but_not_embedded() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");
    let second = a_memory("beta beta");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("something-else")));
    store
        .insert(&second)
        .expect("a new memory is still allowed");

    assert_eq!(
        recorded_model(&path).as_deref(),
        Some("potion-base-32M"),
        "storing a memory must not quietly re-stamp the store with this process's model"
    );
    assert!(
        !store
            .is_findable_by_meaning(second.id)
            .expect("checks the vector"),
        "it waits for a rebuild rather than joining vectors from another model"
    );
}

/// Replacing an embedded memory in a mismatched store is refused, as it is with no model at all.
///
/// *Do not destroy what you cannot rebuild*, where rebuilding means *in this store's model*. The
/// process has an embedder, so the old test's condition — is there a model — no longer separates
/// the cases: rewriting here discards a vector that only the store's own model could replace.
#[test]
fn rewriting_an_embedded_memory_in_a_mismatched_store_is_refused() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");
    let first = a_memory("alpha alpha");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&first).expect("inserts");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("something-else")));

    let mut amended = first.clone();
    amended.content = MemoryText::new("alpha gamma").expect("valid content");

    assert!(
        store.insert(&amended).is_err(),
        "rewriting would discard a vector this process cannot replace in the store's own model"
    );
}

// ---------------------------------------------------------------------------
// The explicit way out
// ---------------------------------------------------------------------------

/// The recovery path: rebuild every vector with the model in hand, deliberately.
#[test]
fn rebuilding_makes_the_store_belong_to_the_model_in_hand() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
        store.insert(&a_memory("beta beta")).expect("inserts");
    }

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("something-else")));
    let rebuilt = store.rebuild_vectors().expect("rebuilds");

    assert_eq!(
        rebuilt.vectors, 2,
        "every memory is re-embedded, not just one"
    );
    assert_eq!(rebuilt.model, ModelIdentity::new("something-else"));
    assert_eq!(
        rebuilt.displaced,
        Some(ModelIdentity::new("potion-base-32M"))
    );
    assert_eq!(
        rebuilt.seen_before, 0,
        "the first rebuild is the ordinary upgrade path and unremarkable"
    );
    assert!(store.can_search_by_meaning().expect("reports capability"));
}

/// Returning to a model that was already displaced is what tug-of-war looks like from inside.
///
/// It **warns and proceeds** rather than refusing: the vectors it protects are derived state
/// rebuilt in under a second, while a wrong refusal costs semantic recall itself and can leave a
/// store its own tool surface cannot repair — decommission the other server and the model that
/// remains is the one being refused. See `documents/design/embeddings.md`.
#[test]
fn returning_to_a_displaced_model_is_reported_and_still_allowed() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }

    // The other server takes its turn.
    {
        let mut store = Store::open_at(scope(), &path).expect("reopens");
        store.set_embedder(Arc::new(NamedEmbedder("something-else")));
        store.rebuild_vectors().expect("rebuilds");
    }

    // And this one takes it back, which is the alternation worth naming.
    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
    let rebuilt = store.rebuild_vectors().expect("rebuilds anyway");

    assert_eq!(
        rebuilt.seen_before, 1,
        "this model has held the store before, which is the signal"
    );
    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Current {
            model: ModelIdentity::new("potion-base-32M")
        },
        "and the rebuild still happened -- a warning, not a refusal"
    );
}

// ---------------------------------------------------------------------------
// Reads never rebuild, and the model belongs to the vectors
// ---------------------------------------------------------------------------

/// **Decided 2026-08-20: a read never rebuilds.** Vectors of unknown provenance used to be deleted
/// and re-embedded by the first recall that met them, which is a read rewriting the whole vector
/// table without saying so (`recall-can-silently-rebuild-every-vector`).
///
/// The argument for doing it automatically was that it *converges* — whoever arrives first resolves
/// it, unlike a mismatch, which oscillates. What settles it against is that this state has **no
/// live producer**: the identity is recorded before the vectors that bind it, in the one place
/// vectors are written, and nothing ever deletes it. Only a store predating 2026-08-08, or a
/// hand-edited database, can be in it. Keeping an automatic read-that-writes to serve a population
/// that can only shrink is a permanent exception for a temporary condition.
#[test]
fn a_recall_does_not_rebuild_vectors_of_unknown_provenance() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }
    forget_which_model_embedded_this(&path);
    let before = vector_count(&path);
    assert_eq!(before, 1, "the fixture must actually hold a vector");

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));

    let recall = store.recall(&Question::new("alpha", 5)).expect("recalls");

    assert!(
        !recall.searched,
        "vectors that cannot be compared against anything must not be searched"
    );
    assert_eq!(
        vector_count(&path),
        before,
        "and a read must not have rewritten them"
    );
    assert_eq!(
        recorded_model(&path),
        None,
        "nor quietly adopted them by writing this model's identity over them"
    );
}

/// The way out is the explicit command, which is what the caller is told to run.
#[test]
fn an_explicit_rebuild_resolves_unknown_provenance() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }
    forget_which_model_embedded_this(&path);

    let mut store = Store::open_at(scope(), &path).expect("reopens");
    store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
    let rebuilt = store.rebuild_vectors().expect("rebuilds");

    assert_eq!(rebuilt.vectors, 1);
    assert_eq!(
        recorded_model(&path).as_deref(),
        Some("potion-base-32M"),
        "and the store is bound to a model it can now account for"
    );
    assert!(
        store
            .recall(&Question::new("alpha", 5))
            .expect("recalls")
            .searched,
        "after which it is searchable again"
    );
}

/// **Decided 2026-08-20: if there are no vectors, there is no model.**
///
/// The recorded identity is a property *of the vectors*, so with no vectors there is nothing for it
/// to be a property of. Import is where this bit: it deletes every vector and then re-embeds, and
/// with no model in hand the re-embed is a no-op — leaving a store claiming a model while holding
/// nothing, which then reported `Mismatched` with nothing to mismatch and refused to fill its own
/// gap when a model returned.
#[test]
fn importing_without_a_model_leaves_no_model_recorded() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    let carried = a_memory("alpha alpha");
    {
        let mut store = Store::open_at(scope(), &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        store.insert(&carried).expect("inserts");
    }
    assert_eq!(vector_count(&path), 1);

    // A server with no model at all imports the set.
    let mut store = Store::open_at(scope(), &path).expect("reopens");
    let set = MemorySet {
        scope: scope(),
        memories: vec![a_memory("beta beta")],
        tombstones: Vec::new(),
        set_id: None,
    };
    store.import(&set).expect("imports");

    assert_eq!(vector_count(&path), 0, "the replaced content's vectors go");
    assert_eq!(
        recorded_model(&path),
        None,
        "and the model goes with them -- no vectors, no model"
    );
    assert_eq!(
        store.model_state().expect("reads its own state"),
        ModelState::Unembedded,
        "which is the state that has nothing to lose and is freely writable"
    );
}

/// The same invariant reached by the other road: forgetting the last embedded memory.
#[test]
fn forgetting_the_last_memory_leaves_no_model_recorded() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");
    let only = a_memory("alpha alpha");

    let mut store = Store::open_at(scope(), &path).expect("opens");
    store
        .mint_set_id()
        .expect("mints an identity, as create does");
    store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
    store.insert(&only).expect("inserts");
    assert_eq!(recorded_model(&path).as_deref(), Some("potion-base-32M"));

    store
        .forget(
            only.id,
            MemoryId::generate(),
            Timestamp::from_str("2026-08-20T09:00:00Z").expect("valid timestamp"),
        )
        .expect("forgets");

    assert_eq!(vector_count(&path), 0);
    assert_eq!(
        recorded_model(&path),
        None,
        "a store holding no vectors is bound to no model"
    );
}

// ---------------------------------------------------------------------------
// The explicit command: a change, and a reset
// ---------------------------------------------------------------------------

/// **Requesting the model the store already holds is legal, and is a reset.**
///
/// Decided 2026-08-20. It reads oddly — nothing appears to change — and it is the right action for
/// vectors whose integrity is in doubt. Allowing it also removes a special case: the command's
/// contract is *after this, every vector here was produced by the named model*, which is true
/// whether or not the name changed.
///
/// What must not happen is the reset looking like a change. Nothing was displaced, and the model
/// history must not grow — otherwise repeated resets would accumulate into the alternation count
/// that exists to detect two servers fighting, and a single user resetting twice would be warned
/// about a conflict that is not there.
#[test]
fn rebuilding_with_the_model_already_recorded_is_a_reset_not_a_change() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    let mut store = Store::open_at(scope(), &path).expect("opens");
    store
        .mint_set_id()
        .expect("mints an identity, as create does");
    store.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    store.insert(&a_memory("beta beta")).expect("inserts");

    let rebuilt = store.rebuild_vectors().expect("rebuilds");

    assert_eq!(rebuilt.vectors, 2, "every vector is rebuilt, not skipped");
    assert_eq!(rebuilt.model, ModelIdentity::new("potion-base-32M"));
    assert_eq!(
        rebuilt.displaced, None,
        "nothing was displaced: the store already held this model"
    );
    assert_eq!(
        rebuilt.seen_before, 0,
        "and a reset is not an alternation, or resetting twice would look like a tug-of-war"
    );

    let again = store.rebuild_vectors().expect("rebuilds again");
    assert_eq!(
        again.seen_before, 0,
        "still not, however many times it is run"
    );
}

/// **Two servers no longer fight, because a read has no move to make.**
///
/// The measured loop was: `_a` stores, `_b` recalls and rebuilds, `_a` recalls and rebuilds,
/// indefinitely. With reads no longer rebuilding, the second server's recalls leave the store
/// exactly as they found it — the standoff is visible in what recall reports rather than in the
/// vector table.
#[test]
fn a_second_server_reading_repeatedly_never_touches_the_first_ones_vectors() {
    let directory = tempdir();
    let path = directory.path().join("shared.sqlite3");

    {
        let mut first = Store::open_at(scope(), &path).expect("opens");
        first
            .mint_set_id()
            .expect("mints an identity, as create does");
        first.set_embedder(Arc::new(NamedEmbedder("potion-base-32M")));
        first.insert(&a_memory("alpha alpha")).expect("inserts");
    }
    let settled = vector_count(&path);
    assert_eq!(settled, 1);

    let mut second = Store::open_at(scope(), &path).expect("reopens");
    second.set_embedder(Arc::new(NamedEmbedder("potion-retrieval-32M")));

    for _ in 0..3 {
        let recall = second.recall(&Question::new("alpha", 5)).expect("recalls");
        assert!(!recall.searched, "a mismatched set is not searched");
    }

    assert_eq!(
        vector_count(&path),
        settled,
        "three reads by the disagreeing server rewrote nothing"
    );
    assert_eq!(
        recorded_model(&path).as_deref(),
        Some("potion-base-32M"),
        "and the set is still accounted for by the model that embedded it"
    );
}
