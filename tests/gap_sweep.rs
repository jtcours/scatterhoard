// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Where the gap sweep runs: before a search by meaning, and not on a write.
//!
//! A *gap* is a memory with no vector row — written while no model was loaded, or by a server that
//! could not supply this store's model. Only recall by meaning reads vectors, so that is where the
//! sweep has work to do. A write embeds its own memory and nothing else; filling other memories'
//! gaps on every write cost a query and bought nothing, since keyword search and lookup never read
//! a vector (`where-the-gap-sweep-runs` in `TRACKING.md`).

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity, embedded_from};
use scatterhoard::memory::{
    Attribution, Confidence, Memory, MemoryId, MemoryText, Origin, Scope, SetName,
};
use scatterhoard::store::{Question, Store};

/// Counts two marker words, and logs every text it is asked to embed so a test can see which
/// memories a call embedded.
#[derive(Debug, Default)]
struct LoggingEmbedder {
    asked: Mutex<Vec<String>>,
}

impl LoggingEmbedder {
    fn times_asked_about(&self, needle: &str) -> usize {
        self.asked
            .lock()
            .expect("unpoisoned")
            .iter()
            .filter(|text| text.contains(needle))
            .count()
    }
}

impl Embedder for LoggingEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("logging-embedder")
    }

    fn dimension(&self) -> usize {
        3
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        self.asked
            .lock()
            .expect("unpoisoned")
            .extend(texts.iter().cloned());
        texts
            .iter()
            .map(|text| {
                let count = |word: &str| text.matches(word).count() as f32;
                embedded_from(vec![count("alpha"), count("beta"), 0.01])
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
            at: Timestamp::from_str("2026-09-22T09:00:00Z").expect("valid timestamp"),
        },
    )
}

/// A store holding one gap, written before any model was loaded, and the model that arrives after.
fn a_store_with_a_gap() -> (Store, Memory, Arc<LoggingEmbedder>) {
    let mut store = Store::open_in_memory_and_mint_id(scope()).expect("opens");
    let gap = a_memory("alpha written before the model");
    store.insert(&gap).expect("stores without a model");
    let embedder = Arc::new(LoggingEmbedder::default());
    store.set_embedder(embedder.clone());
    (store, gap, embedder)
}

#[test]
fn a_write_embeds_its_own_memory_and_leaves_other_gaps_alone() {
    let (mut store, gap, embedder) = a_store_with_a_gap();

    let written = a_memory("beta written after the model");
    store.insert(&written).expect("writes");

    assert!(
        store.is_findable_by_meaning(written.id).expect("reads"),
        "the write embeds what it writes"
    );
    assert_eq!(
        embedder.times_asked_about("written before the model"),
        0,
        "a write has no use for another memory's vector, so it does not make one"
    );
    assert!(!store.is_findable_by_meaning(gap.id).expect("reads"));
    assert_eq!(store.awaiting_vectors().expect("counts"), 1);
}

#[test]
fn the_next_recall_by_meaning_fills_the_gap() {
    let (mut store, gap, _) = a_store_with_a_gap();
    store
        .insert(&a_memory("beta written after the model"))
        .expect("writes");

    let recall = store.recall(&Question::new("alpha", 5)).expect("recalls");

    assert!(
        recall.hits.iter().any(|hit| hit.memory.id == gap.id),
        "the gap is embedded before the search, so the search finds it"
    );
    assert_eq!(store.awaiting_vectors().expect("counts"), 0);
}
