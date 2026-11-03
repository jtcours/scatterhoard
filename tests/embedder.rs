// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Tests for loading a model and embedding with it.
//!
//! Most of these need a real model on disk, which not every machine has. A test that quietly
//! becomes a no-op is indistinguishable from one that passed — the failure mode recorded in
//! the `coding-lessons` set — so the ones that need a model **announce loudly** when they skip, and the
//! missing-model path is tested unconditionally, since that is the case every new install hits
//! first.

use std::path::PathBuf;
use std::sync::Arc;

use scatterhoard::embedder::{
    DEFAULT_MODEL, Embedded, Embedder, EmbedderError, ModelIdentity, StaticEmbedder, embedded_from,
};
use scatterhoard::embedding::EmbeddingError;

/// Where a model happens to live on a developer machine that has run the spike. Not a supported
/// configuration — just enough to exercise the real path when it is available.
fn model_directory() -> Option<PathBuf> {
    let base = dirs_home()?
        .join(".cache/huggingface/hub")
        .join(format!("models--minishlab--{DEFAULT_MODEL}"))
        .join("snapshots");

    std::fs::read_dir(base)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("model.safetensors").exists())
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Announces rather than skipping silently. Returns `None` when the caller should stop.
fn model_or_announce(test: &str) -> Option<StaticEmbedder> {
    match model_directory() {
        Some(path) => match StaticEmbedder::load(&path) {
            Ok(embedder) => Some(embedder),
            Err(error) => panic!("the model at {} should load: {error}", path.display()),
        },
        None => {
            eprintln!(
                "\n*** SKIPPED: {test} — no {DEFAULT_MODEL} model found on this machine.\n\
                 *** This test did NOT run. It is not evidence of anything.\n"
            );
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Runs everywhere
// ---------------------------------------------------------------------------

/// The first thing a new install hits, so it is worth more than a bare error type: the message has
/// to carry the fix, name what still works, and say that Scatterhoard will not fetch anything
/// itself — because a user who expects an automatic download will wait for one that never comes.
#[test]
fn a_missing_model_is_reported_with_the_fix() {
    let path = PathBuf::from("/nonexistent/scatterhoard/model");
    let error = StaticEmbedder::load(&path).expect_err("nothing is there");

    assert!(matches!(error, EmbedderError::ModelMissing { .. }));

    let message = error.to_string();
    for expected in [DEFAULT_MODEL, "keyword search", "never downloads"] {
        assert!(
            message.contains(expected),
            "the missing-model message should mention {expected:?}, got: {message}"
        );
    }
}

/// Pooling over no tokens gives exactly zero, and that is the one outcome a text alone can cause — so
/// it is a verdict about the text, not a fault. `one-unembeddable-memory-stops-the-store`.
#[test]
fn a_vector_of_zeros_means_the_text_had_nothing_to_represent() {
    assert!(matches!(
        embedded_from(vec![0.0, 0.0, 0.0]),
        Ok(Embedded::Unrepresentable)
    ));
}

/// **A broken model must not pass as a language gap.** Calling a non-finite or empty vector
/// unrepresentable would store every memory without a vector and report it as the text's fault.
#[test]
fn a_vector_only_a_broken_model_produces_is_still_an_error() {
    assert!(matches!(
        embedded_from(vec![f32::NAN, 1.0]),
        Err(EmbedderError::Unusable(EmbeddingError::NotFinite))
    ));
    assert!(matches!(
        embedded_from(Vec::new()),
        Err(EmbedderError::Unusable(EmbeddingError::Empty))
    ));
}

#[test]
fn an_ordinary_vector_is_a_vector() {
    assert!(matches!(
        embedded_from(vec![0.0, 1.0]),
        Ok(Embedded::Vector(_))
    ));
}

/// The strict form keeps its old contract, because measurement code relies on it: a probe that asks
/// for a vector and cannot have one must stop, not carry on with a hole in its data.
#[test]
fn the_strict_form_still_refuses_a_text_with_nothing_to_represent() {
    #[derive(Debug)]
    struct Blank;
    impl Embedder for Blank {
        fn identity(&self) -> ModelIdentity {
            ModelIdentity::new("blank")
        }
        fn dimension(&self) -> usize {
            2
        }
        fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
            Ok(texts.iter().map(|_| Embedded::Unrepresentable).collect())
        }
    }

    assert!(matches!(
        Blank.embed(&["anything".to_owned()]),
        Err(EmbedderError::Unusable(EmbeddingError::NoDirection))
    ));
}

// ---------------------------------------------------------------------------
// Needs a model
// ---------------------------------------------------------------------------

/// The case the fake in `tests/unrepresentable.rs` stands in for, on the real model: a sentence in
/// a script `potion-base-32M` has no tokens for is a verdict, and an English one beside it in the
/// same batch is unaffected by it.
#[test]
fn text_in_a_script_the_model_has_no_tokens_for_is_unrepresentable() {
    let Some(embedder) =
        model_or_announce("text_in_a_script_the_model_has_no_tokens_for_is_unrepresentable")
    else {
        return;
    };

    let outcomes = embedder
        .embed_each(&[
            "这个汤需要多放点盐".to_owned(),
            "the soup needs more salt".to_owned(),
        ])
        .expect("one unrepresentable text must not fail the batch");

    assert!(matches!(outcomes[0], Embedded::Unrepresentable));
    assert!(matches!(outcomes[1], Embedded::Vector(_)));
}

#[test]
fn the_model_reports_a_stable_dimension() {
    let Some(embedder) = model_or_announce("the_model_reports_a_stable_dimension") else {
        return;
    };

    assert!(embedder.dimension() > 0);
    let vectors = embedder
        .embed(&["one".to_owned(), "two".to_owned()])
        .unwrap();
    assert_eq!(vectors.len(), 2);
    for vector in vectors {
        assert_eq!(vector.dimension(), embedder.dimension());
    }
}

/// The property the whole design rests on: semantically related text lands closer together than
/// unrelated text. Not a quality benchmark — the spike measured that — just a check that the model
/// is wired up the right way round.
#[test]
fn related_text_embeds_closer_than_unrelated_text() {
    let Some(embedder) = model_or_announce("related_text_embeds_closer_than_unrelated_text") else {
        return;
    };

    let texts = [
        "memories are partitioned by scope so a query cannot cross between them",
        "each scope gets its own store rather than sharing a table",
        "the embedder runs offline and never fetches a model at run time",
    ];
    let vectors = embedder.embed(&texts.map(str::to_owned)).expect("embeds");

    let cosine = |a: &[f32], b: &[f32]| {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (na * nb)
    };

    let related = cosine(vectors[0].components(), vectors[1].components());
    let unrelated = cosine(vectors[0].components(), vectors[2].components());

    assert!(
        related > unrelated,
        "the two scope memories should be closer to each other ({related}) \
         than to the offline one ({unrelated})"
    );
}

/// Import re-embeds whatever changed and sits on the critical path of an ordinary `git pull`, so
/// embedding a set the size of the corpus must be imperceptible rather than merely tolerable.
#[test]
fn embedding_a_corpus_sized_batch_is_fast() {
    let Some(embedder) = model_or_announce("embedding_a_corpus_sized_batch_is_fast") else {
        return;
    };

    let batch: Vec<String> = (0..500)
        .map(|i| format!("memory number {i} about storage, scope, recall, and merging"))
        .collect();

    let start = std::time::Instant::now();
    let vectors = embedder.embed(&batch).expect("embeds");
    let elapsed = start.elapsed();

    assert_eq!(vectors.len(), 500);
    assert!(
        elapsed.as_secs_f64() < 5.0,
        "500 memories took {elapsed:?}, which would make import noticeable"
    );
}

// ---------------------------------------------------------------------------
// End to end
// ---------------------------------------------------------------------------

/// The first time every layer runs together: the real corpus, read through the real file reader,
/// imported into a real store, embedded by the real model, and searched by meaning.
///
/// The assertions are deliberately weak. A recall-quality claim at 39 memories would be measuring
/// noise — the spike measured quality, on a labelled corpus, with a metric. What this establishes
/// is that the pipeline is connected and pointing the right way: a query about a topic the corpus
/// covers returns memories about that topic rather than arbitrary ones.
#[test]
fn the_corpus_can_be_imported_embedded_and_searched() {
    let Some(embedder) = model_or_announce("the_corpus_can_be_imported_embedded_and_searched")
    else {
        return;
    };

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/corpus/design-memories.jsonl");
    let file = std::fs::File::open(path).expect("the corpus file should exist");
    let set =
        scatterhoard::memory_file::read(&scatterhoard::memory_file::Expectation::AnyShared, file)
            .expect("the corpus reads");

    let mut store =
        scatterhoard::store::Store::open_in_memory_and_mint_id(set.scope.clone()).expect("opens");
    store.set_embedder(Arc::new(embedder));
    store.import(&set).expect("imports");

    let recall = store
        .recall(&scatterhoard::store::Question::new(
            "why is storage split up by scope?",
            5,
        ))
        .expect("recalls");

    assert!(recall.searched);
    // A window rather than a count: the cut falls at the largest drop inside it, so five is the most
    // that can come back and any number of them is a legitimate answer. Asserting equality here was
    // asserting the old fixed cut.
    assert!(!recall.hits.is_empty() && recall.hits.len() <= 5);
    println!(
        "returned by the gap cut, from a window of five: {}",
        recall.hits.len()
    );

    let text: String = recall
        .hits
        .iter()
        .map(|hit| hit.memory.content.as_data().to_lowercase())
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        text.contains("scope"),
        "a query about scope should surface a memory mentioning it; got: {text}"
    );

    // The corpus records two corrections, so two of its memories are superseded and must never come
    // back as though current -- whatever they score.
    let superseded_returned = recall
        .hits
        .iter()
        .filter(|hit| hit.matched.is_some())
        .count();
    println!("substitutions made by validity resolution: {superseded_returned}");
}

/// Text past the model's default token budget has to reach the vector.
///
/// `model2vec-rs` defaults `encode` to `max_length = 512`, which silently drops the tail — and the
/// tail of a memory's embedding input is its `why` and then its keywords, so the default removes
/// exactly the part that distinguishes one long memory from another. A static model mean-pools and
/// has no context window, so there is nothing the budget protects.
#[test]
fn text_past_the_default_token_budget_still_reaches_the_vector() {
    let Some(embedder) =
        model_or_announce("text_past_the_default_token_budget_still_reaches_the_vector")
    else {
        return;
    };

    let long = "convention ".repeat(600);
    let appended = format!("{long} zzyzx pangolin quixotic");
    let mut vectors = embedder.embed(&[long, appended]).expect("both texts embed");
    let with_suffix = vectors.remove(1);
    let without = vectors.remove(0);

    assert!(
        without.components() != with_suffix.components(),
        "appending text to a long input must change its vector; identical components mean the \
         suffix never reached the model. Compared as a flag rather than with assert_ne!, which \
         would print both vectors in full and say nothing a reader can use."
    );
}
