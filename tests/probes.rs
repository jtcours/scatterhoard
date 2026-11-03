//! Measurements against this machine's live memory sets, run by hand.
//!
//! Every test here is `#[ignore]`d and prints rather than asserts, in the shape of
//! `report_catalog_size`: what they answer is *what is the corpus doing today*, which is a number a
//! person reads and not a property the suite can hold. They are in the repository rather than a
//! scratchpad for the reason `corpus/seed-quality-provenance.md` gives -- a measurement written to
//! /private/tmp is one sweep away from being lost, and these are meant to be re-run as the corpus
//! grows.
//!
//! ```text
//! SCATTERHOARD_MODEL="$(find ~/.cache/huggingface/hub/models--minishlab--potion-base-32M/snapshots \
//!   -name model.safetensors -exec dirname {} \;)" \
//!   cargo test --test probes -- --ignored --nocapture
//! ```
//!
//! **They copy the stores before opening them.** `recall_across` calls `prepare`, which may write,
//! and the live stores are held open by an attached server; a probe must not be able to disturb the
//! thing it measures.
//!
//! **Several probes measure a set of an association's governing documents**, which each machine names
//! for itself. Set `SCATTERHOARD_PROBE_GOVERNANCE_SET` to that set's name; `governance-docs`, the name
//! the recorded findings use, is a placeholder.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use scatterhoard::embedder::{Embedded, Embedder, StaticEmbedder};
use scatterhoard::embedding::{self, Vector, VectorSet};
use scatterhoard::language_fit::{self, BUILT_IN, Sentences};
use scatterhoard::mcp::Server;
use scatterhoard::memory::Author;
use scatterhoard::memory::MemoryId;
use scatterhoard::memory::Scope;
use scatterhoard::store::{
    Centring, Floor, Question, Stages, Store, StoreLayout, recall_across, recall_across_with,
};
use scatterhoard::{eval, traversal};

/// The live data directory, found the way the server finds it: `SCATTERHOARD_HOME`, otherwise
/// `.local/share/scatterhoard` under `HOME`.
fn live() -> PathBuf {
    std::env::var_os("SCATTERHOARD_HOME").map_or_else(
        || {
            PathBuf::from(std::env::var_os("HOME").expect("HOME or SCATTERHOARD_HOME is set"))
                .join(".local/share/scatterhoard")
        },
        PathBuf::from,
    )
}

/// The windows each query is asked at, smallest first.
///
/// The cut is computed inside the window, so asking the same query at several is how the probe sees
/// whether the answer is a property of the corpus or of the budget it was given.
const WINDOWS: [usize; 4] = [5, 10, 20, 40];

/// The set of an association's governing documents that several probes measure, by the name this
/// machine gives it.
fn governance_set() -> String {
    std::env::var("SCATTERHOARD_PROBE_GOVERNANCE_SET")
        .unwrap_or_else(|_| "governance-docs".to_string())
}

/// [`governance_set`] as a scope key.
fn governance_scope() -> String {
    format!("shared:{}", governance_set())
}

/// A copy of the live stores, so nothing here can write to a store a server is holding.
fn copied_stores(into: &Path) -> Vec<(Scope, PathBuf)> {
    let mut copied = Vec::new();
    let governance = governance_scope();
    for key in [
        "shared:scatterhoard",
        "shared:coding-lessons",
        "local",
        governance.as_str(),
    ] {
        let file = match key {
            "local" => "local.sqlite3".to_string(),
            other => format!("shared-{}.sqlite3", other.strip_prefix("shared:").unwrap()),
        };
        let to = into.join(&file);
        // -wal and -shm too: without the write-ahead log the copy is the database as of the last
        // checkpoint, which is a real corpus but not today's.
        for suffix in ["", "-wal", "-shm"] {
            let from = live().join(format!("{file}{suffix}"));
            if from.exists() {
                std::fs::copy(&from, into.join(format!("{file}{suffix}"))).expect("copy a store");
            }
        }
        copied.push((Scope::from_key(key).expect("a valid scope key"), to));
    }
    copied
}

fn model() -> Option<Arc<StaticEmbedder>> {
    let path = std::env::var("SCATTERHOARD_MODEL").ok()?;
    StaticEmbedder::load(Path::new(&path)).ok().map(Arc::new)
}

/// Queries in three classes, because the class is what the probe is testing.
///
/// The 2026-09-02 replay behind `no-non-match-floor` used eleven *real* recalls, every one of them
/// on-topic -- so it could not have seen what the cut does to a query the corpus cannot answer,
/// which is the case the defect is named for.
const QUERIES: [(&str, &str); 9] = [
    (
        "specific",
        "why does the generated index end marker carry its keyword",
    ),
    (
        "specific",
        "what decides which memory set a memory belongs in",
    ),
    (
        "specific",
        "how is a mutation counted as caught when the tests hang",
    ),
    ("broad", "design decisions and constraints"),
    ("broad", "how the work has gone wrong"),
    ("broad", "measurement"),
    (
        "off-corpus",
        "sourdough starter hydration ratio for a rye loaf",
    ),
    ("off-corpus", "how to replace a bicycle inner tube"),
    ("off-corpus", "premier league offside rule interpretation"),
];

#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_where_the_gap_cut_falls() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let stores = copied_stores(scratch.path());

    let mut opened: Vec<Store> = stores
        .iter()
        .map(|(scope, path)| {
            let mut store = Store::open_at(scope.clone(), path).expect("open a copied store");
            store.set_embedder(embedder.clone());
            store
        })
        .collect();
    let total: usize = opened.iter().map(|s| s.count().unwrap_or(0)).sum();

    println!("\nCORPUS {total} memories across {} sets\n", opened.len());
    println!(
        "{:<11} {:<48} {:>19}  {:>5} {:>5}  top scores (own+inh=combined)",
        "class", "query", "returned at window", "neg", "mono"
    );

    for (class, query) in QUERIES {
        let mut counts = Vec::new();
        let mut last = None;
        for window in WINDOWS {
            let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
            let recall = recall_across(&mut refs, &Question::new(query, window)).expect("recall");
            counts.push(format!("{:>2}", recall.hits.len()));
            last = Some(recall);
        }
        let widest = last.expect("four windows were asked");

        let negatives = widest.hits.iter().filter(|h| h.similarity <= 0.0).count();
        let monotonic = widest
            .hits
            .windows(2)
            .all(|pair| pair[0].similarity >= pair[1].similarity);
        let scores: String = widest
            .hits
            .iter()
            .take(4)
            .map(|h| {
                let combined = traversal::combine(f64::from(h.similarity), f64::from(h.inherited));
                format!("{:.3}+{:.3}={:.3}", h.similarity, h.inherited, combined)
            })
            .collect::<Vec<_>>()
            .join("  ");

        println!(
            "{class:<11} {:<48} {:>19}  {negatives:>5} {:>5}  {scores}",
            &query[..query.len().min(48)],
            counts.join(" "),
            if monotonic { "yes" } else { "NO" },
        );
    }
    println!(
        "\nwindows asked: {WINDOWS:?}; `neg` counts returned memories scoring <= 0 at window 40;"
    );
    println!(
        "`mono` is whether the returned order is monotonic in the reported `similarity` alone."
    );
}

#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_the_cut_against_the_window() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let mut opened: Vec<Store> = copied_stores(scratch.path())
        .iter()
        .map(|(scope, path)| {
            let mut store = Store::open_at(scope.clone(), path).expect("open a copied store");
            store.set_embedder(embedder.clone());
            store
        })
        .collect();

    println!(
        "\nHow many results each window returns. The claim under test is that the cut is a\n\
              property of the corpus rather than of the budget: if so, a row is flat.\n"
    );
    print!("{:<11} {:<44}", "class", "query");
    for window in 3..=24 {
        print!("{window:>3}");
    }
    println!();
    for (class, query) in QUERIES {
        print!("{class:<11} {:<44}", &query[..query.len().min(44)]);
        for window in 3..=24 {
            let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
            let recall = recall_across(&mut refs, &Question::new(query, window)).expect("recall");
            print!("{:>3}", recall.hits.len());
        }
        println!();
    }
    println!(
        "\nThe live default window is 20 (`DEFAULT_WINDOW` in src/mcp.rs), so read that column."
    );
}

#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_what_an_unanswerable_query_returns() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let mut opened: Vec<Store> = copied_stores(scratch.path())
        .iter()
        .map(|(scope, path)| {
            let mut store = Store::open_at(scope.clone(), path).expect("open a copied store");
            store.set_embedder(embedder.clone());
            store
        })
        .collect();

    println!(
        "\nWhat comes back at the default window for a query the corpus cannot answer,\n\
              beside an on-topic query for comparison. Scores are own similarity.\n"
    );
    for (class, query) in QUERIES {
        if class != "off-corpus" && !query.starts_with("what decides") {
            continue;
        }
        let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
        let recall = recall_across(&mut refs, &Question::new(query, 20)).expect("recall");
        println!("[{class}] {query:?} -> {} results", recall.hits.len());
        for hit in recall.hits.iter().take(6) {
            let line: String = hit
                .memory
                .content
                .as_data()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            println!("   {:.3}  {}", hit.similarity, &line[..line.len().min(96)]);
        }
        println!();
    }
}

#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_any_per_query_statistic_separates_a_non_match() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    // Read the vectors straight out of the copied databases, the way `corpus/*.py` does, because
    // the whole score distribution is what is under test and recall only ever returns its cut.
    let mut vectors = VectorSet::new(0);
    let mut dimension = 0;
    for (_, path) in &copies {
        let db = rusqlite::Connection::open(path).expect("open a copied store");
        let mut statement = db
            .prepare("SELECT memory_id, vector FROM vectors")
            .expect("query");
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .expect("read vectors");
        for row in rows {
            let (id, blob) = row.expect("a vector row");
            let components: Vec<f32> = blob
                .chunks_exact(4)
                .map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]]))
                .collect();
            if dimension == 0 {
                dimension = components.len();
                vectors = VectorSet::new(dimension);
            }
            if components.len() != dimension {
                continue;
            }
            let Ok(id) = MemoryId::parse(&id) else {
                continue;
            };
            let Ok(vector) = Vector::new(components) else {
                continue;
            };
            let _ = vectors.insert(id, vector);
        }
    }
    let centroid = embedding::combined_centroid(&[&vectors]).expect("a centroid");

    println!(
        "\nThe whole score distribution per query, over {} vectors. If an absolute reference\n\
              could exist, an off-corpus query's top score would sit low in its own distribution.\n",
        vectors.len()
    );
    println!(
        "{:<11} {:<44} {:>7} {:>7} {:>7} {:>7} {:>8}",
        "class", "query", "top1", "median", "p99", "max-med", "top1 z"
    );
    for (class, query) in QUERIES {
        let vector = embedder
            .embed(&[query.to_string()])
            .expect("embed the query")
            .remove(0);
        let mut hits = vectors.nearest_centred_on(&vector, vectors.len(), &centroid);
        hits.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
        let scores: Vec<f64> = hits.iter().map(|h| f64::from(h.similarity)).collect();
        let top1 = scores[0];
        let median = scores[scores.len() / 2];
        let p99 = scores[scores.len() / 100];
        let mean = scores.iter().sum::<f64>() / scores.len() as f64;
        let sd =
            (scores.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / scores.len() as f64).sqrt();
        println!(
            "{class:<11} {:<44} {top1:>7.3} {median:>7.3} {p99:>7.3} {:>7.3} {:>8.2}",
            &query[..query.len().min(44)],
            top1 - median,
            (top1 - mean) / sd,
        );
    }
    println!(
        "\n`top1 z` is how many standard deviations the best hit sits above that query's own mean."
    );
}

#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_the_shape_of_the_ranking_around_a_cliff() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let mut opened: Vec<Store> = copied_stores(scratch.path())
        .iter()
        .map(|(scope, path)| {
            let mut store = Store::open_at(scope.clone(), path).expect("open a copied store");
            store.set_embedder(embedder.clone());
            store
        })
        .collect();

    println!(
        "\nThe ranking either side of a cut, with the gap between each score and the next.\n\
              The cut is placed at the largest of these gaps, so what matters is whether one of\n\
              them stands out or whether they are all the same size.\n"
    );

    for (query, window) in [
        ("what decides which memory set a memory belongs in", 17),
        ("what decides which memory set a memory belongs in", 18),
    ] {
        let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
        let recall = recall_across(&mut refs, &Question::new(query, window)).expect("recall");
        let scores: Vec<f64> = recall
            .hits
            .iter()
            .map(|h| traversal::combine(f64::from(h.similarity), f64::from(h.inherited)))
            .collect();

        // Page once past the cut, the way a caller asking for more would: `already_known` removes
        // these from the results and changes nothing else, so what comes back is the next tranche
        // of the same ranking.
        let known: Vec<_> = recall.hits.iter().map(|h| h.memory.id).collect();
        let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
        let more = recall_across(
            &mut refs,
            &Question::new(query, window).already_holding(&known),
        )
        .expect("recall the next tranche");
        let beyond: Vec<f64> = more
            .hits
            .iter()
            .take(3)
            .map(|h| traversal::combine(f64::from(h.similarity), f64::from(h.inherited)))
            .collect();

        println!("window {window} -> {} results returned", scores.len());
        let all: Vec<f64> = scores.iter().chain(beyond.iter()).copied().collect();
        for (rank, score) in all.iter().enumerate() {
            let gap = all.get(rank + 1).map(|next| score - next);
            let edge = if rank + 1 == scores.len() {
                "  <-- cut here"
            } else {
                ""
            };
            let past = if rank >= scores.len() {
                "  (past the cut)"
            } else {
                ""
            };
            match gap {
                Some(gap) => println!(
                    "  {:>2}. {score:.4}   gap to next {gap:.4}{edge}{past}",
                    rank + 1
                ),
                None => println!("  {:>2}. {score:.4}{past}", rank + 1),
            }
        }
        println!();
    }
}

/// Queries aimed at each set, because a set can only be fairly asked what it is about.
const PER_SET: [(&str, &[&str]); 2] = [
    (
        "shared:scatterhoard",
        &[
            "what decides which memory set a memory belongs in",
            "why does the generated index end marker carry its keyword",
            "how are convention texts kept within their budget",
        ],
    ),
    (
        "shared:coding-lessons",
        &[
            "a check only finds what someone thought to look for",
            "why a slow build might not be the tests themselves",
            "what to do when a measurement turns out to be confounded",
        ],
    ),
];

/// The empirical null: rankings that are known to contain no answer.
///
/// A synthetic null has to assume a shape for the score distribution, and similarity scores are not
/// uniform. These are the real machinery over the real corpus with the one thing removed that the
/// on-topic queries have, so whatever gap structure they show is what "no structure" looks like
/// here.
const NULL_QUERIES: [&str; 3] = [
    "sourdough starter hydration ratio for a rye loaf",
    "how to replace a bicycle inner tube",
    "premier league offside rule interpretation",
];

/// Gaps between consecutive scores, from rank 2 down: the ones `cut_at_largest_gap` may choose.
///
/// Returns `(largest, median, rank of the largest)`. The ratio between the first two is the shape:
/// near 1 means a smooth roll-off where the largest gap is noise, and large means a real break.
fn gap_profile(scores: &[f64]) -> (f64, f64, usize) {
    let mut gaps: Vec<(f64, usize)> = scores
        .windows(2)
        .enumerate()
        .skip(1)
        .map(|(index, pair)| (pair[0] - pair[1], index + 1))
        .collect();
    if gaps.is_empty() {
        return (0.0, 0.0, 0);
    }
    let largest = gaps
        .iter()
        .copied()
        .fold((f64::MIN, 0), |a, b| if b.0 > a.0 { b } else { a });
    gaps.sort_by(|a, b| a.0.total_cmp(&b.0));
    (largest.0, gaps[gaps.len() / 2].0, largest.1)
}

#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_linking_puts_structure_into_the_roll_off() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    println!(
        "\nDoes the graph walk put a break into a ranking that similarity alone rolls off\n\
              smoothly? `largest/median` is over the gaps the cut may choose, so ~1 is a smooth\n\
              roll-off where the largest gap is noise, and a large ratio is a real break.\n"
    );
    println!(
        "{:<27} {:<46} {:>8} {:>8} {:>7} {:>5} {:>7}",
        "set", "query", "largest", "median", "ratio", "at", "inherit"
    );

    let arms: Vec<(&str, Vec<&str>)> = PER_SET
        .iter()
        .map(|(key, queries)| (*key, queries.to_vec()))
        .chain(std::iter::once((
            "shared:scatterhoard NULL",
            NULL_QUERIES.to_vec(),
        )))
        .collect();
    for (label, queries) in arms {
        let key = label.trim_end_matches(" NULL");
        let scope = Scope::from_key(key).expect("a valid scope key");
        let (_, path) = copies
            .iter()
            .find(|(s, _)| *s == scope)
            .expect("a copied store");
        let mut store = Store::open_at(scope.clone(), path).expect("open a copied store");
        store.set_embedder(embedder.clone());

        for query in queries {
            // Page with `already_known` to walk 20 deep: recall truncates at its own cut, and the
            // shape of the ranking below that cut is exactly what is being asked about. **Without
            // the floor**, which calibration added after this probe was written: a calibrated set
            // drops everything below it, and the probe would measure a ranking cut short.
            let mut scores: Vec<f64> = Vec::new();
            let mut inherited = 0usize;
            let mut known: Vec<_> = Vec::new();
            while scores.len() < 20 {
                let mut refs = vec![&mut store];
                let question = Question::new(query, 20)
                    .already_holding(&known)
                    .without_the_floor();
                let recall = recall_across(&mut refs, &question).expect("recall");
                if recall.hits.is_empty() {
                    break;
                }
                for hit in &recall.hits {
                    scores.push(traversal::combine(
                        f64::from(hit.similarity),
                        f64::from(hit.inherited),
                    ));
                    if hit.inherited > 0.0 {
                        inherited += 1;
                    }
                    known.push(hit.memory.id);
                }
            }
            scores.truncate(20);
            let (largest, median, at) = gap_profile(&scores);
            println!(
                "{label:<27} {:<46} {largest:>8.4} {median:>8.4} {:>7.2} {at:>5} {:>7}",
                &query[..query.len().min(46)],
                if median > 0.0 {
                    largest / median
                } else {
                    f64::INFINITY
                },
                format!("{inherited}/{}", scores.len()),
            );
        }
    }
    println!(
        "\n`at` is the rank the largest gap sits at; `inherit` is how many of the 20 the walk reached."
    );
}

/// Is the instrument working at all?
///
/// Asked because every finding above reads *the scores carry little information*, and the
/// alternative explanation for that is a broken embedder or a broken similarity. Four checks, each
/// of which a defect in either would fail: the embedder is deterministic; a memory's stored vector
/// still matches what its text embeds to today; a memory's own text retrieves it at rank 1; and a
/// query that was never embedded into the memory still retrieves it well above chance.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_the_embedder_is_working() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let (scope, path) = copies
        .iter()
        .find(|(s, _)| *s == Scope::from_key("shared:scatterhoard").unwrap())
        .expect("the scatterhoard store");
    let store = Store::open_at(scope.clone(), path).expect("open a copied store");

    println!("\n1. DETERMINISM");
    let once = embedder
        .embed(&["the same text twice".to_string()])
        .expect("embed");
    let twice = embedder
        .embed(&["the same text twice".to_string()])
        .expect("embed");
    println!(
        "   identical vectors for identical text: {}",
        once[0].components() == twice[0].components()
    );
    println!(
        "   dimension {}, first components {:?}",
        once[0].dimension(),
        &once[0].components()[..4]
    );

    // Every memory and its stored vector, read from the copy the way `corpus/*.py` does.
    let db = rusqlite::Connection::open(path).expect("open");
    let ids: Vec<String> = db
        .prepare("SELECT memory_id FROM vectors ORDER BY memory_id")
        .expect("query")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("read")
        .filter_map(Result::ok)
        .collect();
    let stored: std::collections::HashMap<String, Vec<f32>> = db
        .prepare("SELECT memory_id, vector FROM vectors")
        .expect("query")
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .expect("read")
        .filter_map(Result::ok)
        .map(|(id, blob)| {
            let components = blob
                .chunks_exact(4)
                .map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]]))
                .collect();
            (id, components)
        })
        .collect();

    // The whole set, uncentred, so a self-match scores 1.0 and nothing is relative to a corpus.
    let dimension = once[0].dimension();
    let mut all = VectorSet::new(dimension);
    for (id, components) in &stored {
        if let (Ok(id), Ok(vector)) = (MemoryId::parse(id), Vector::new(components.clone())) {
            let _ = all.insert(id, vector);
        }
    }

    let sample: Vec<&String> = ids.iter().step_by(ids.len() / 40).take(40).collect();

    println!("\n2. STORED VECTORS vs RE-EMBEDDING THE SAME TEXT TODAY");
    let mut drift: Vec<f64> = Vec::new();
    for id in &sample {
        let Ok(parsed) = MemoryId::parse(id) else {
            continue;
        };
        let Ok(Some(memory)) = store.get(parsed) else {
            continue;
        };
        let fresh = embedder
            .embed(&[embedding::embedding_input(&memory)])
            .expect("embed")
            .remove(0);
        let mut one = VectorSet::new(dimension);
        let _ = one.insert(
            parsed,
            Vector::new(stored[*id].clone()).expect("a stored vector"),
        );
        let hit = one.nearest_uncentred(&fresh, 1);
        drift.push(f64::from(hit[0].similarity));
    }
    drift.sort_by(f64::total_cmp);
    println!(
        "   {} sampled; cosine(stored, re-embedded) min {:.6} median {:.6} max {:.6}",
        drift.len(),
        drift[0],
        drift[drift.len() / 2],
        drift[drift.len() - 1]
    );

    println!("\n3. SELF-RETRIEVAL -- a memory's own embedded text as the query");
    let mut ranks: Vec<usize> = Vec::new();
    for id in &sample {
        let Ok(parsed) = MemoryId::parse(id) else {
            continue;
        };
        let Ok(Some(memory)) = store.get(parsed) else {
            continue;
        };
        let query = embedder
            .embed(&[embedding::embedding_input(&memory)])
            .expect("embed")
            .remove(0);
        let hits = all.nearest_uncentred(&query, 5);
        ranks.push(
            hits.iter()
                .position(|h| h.id == parsed)
                .map_or(999, |at| at + 1),
        );
    }
    let at_one = ranks.iter().filter(|r| **r == 1).count();
    println!(
        "   rank 1 for {at_one} of {}; ranks {:?}",
        ranks.len(),
        &ranks[..ranks.len().min(12)]
    );

    println!("\n4. HELD-OUT RETRIEVAL -- `context`, which is deliberately never embedded");
    let mut held: Vec<usize> = Vec::new();
    for id in &ids {
        let Ok(parsed) = MemoryId::parse(id) else {
            continue;
        };
        let Ok(Some(memory)) = store.get(parsed) else {
            continue;
        };
        let Some(context) = memory.context.as_ref() else {
            continue;
        };
        let query = embedder
            .embed(&[context.as_data().to_string()])
            .expect("embed")
            .remove(0);
        let hits = all.nearest_uncentred(&query, all.len());
        held.push(
            hits.iter()
                .position(|h| h.id == parsed)
                .map_or(9999, |at| at + 1),
        );
        if held.len() >= 60 {
            break;
        }
    }
    held.sort_unstable();
    let median = held[held.len() / 2];
    println!(
        "   {} memories; rank of the memory its own `context` came from:",
        held.len()
    );
    println!(
        "   median {median}, best {}, worst {}, top-10 for {} of them, chance median would be {}",
        held[0],
        held[held.len() - 1],
        held.iter().filter(|r| **r <= 10).count(),
        all.len() / 2
    );
}

/// Load a copied store as an eval [`Corpus`]: every memory, and the vector it already carries.
fn corpus_from(path: &Path, store: &Store, dimension: usize) -> eval::Corpus {
    let db = rusqlite::Connection::open(path).expect("open a copied store");
    let mut memories = Vec::new();
    let mut vectors = VectorSet::new(dimension);
    let rows: Vec<(String, Vec<u8>)> = db
        .prepare("SELECT memory_id, vector FROM vectors")
        .expect("query")
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .expect("read")
        .filter_map(Result::ok)
        .collect();
    for (id, blob) in rows {
        let Ok(parsed) = MemoryId::parse(&id) else {
            continue;
        };
        let Ok(Some(memory)) = store.get(parsed) else {
            continue;
        };
        let components: Vec<f32> = blob
            .chunks_exact(4)
            .map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]]))
            .collect();
        let Ok(vector) = Vector::new(components) else {
            continue;
        };
        let _ = vectors.insert(parsed, vector);
        memories.push(memory);
    }
    eval::Corpus::new(memories, vectors)
}

/// Where a linked memory sits relative to its source, re-measured on today's corpus.
///
/// This was last taken on 2026-08-15 over 23 traversable edges, when the median target sat at the
/// 2.9th percentile and the corpus was below the crossover that makes traversal distinguishable
/// from simply raising the limit. Both the edge count and the corpus have grown several-fold since.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_where_a_link_target_sits_now() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let dimension = embedder.embed(&["dimension".to_string()]).expect("embed")[0].dimension();

    println!(
        "\nWhere a link's target ranks when the query falls exactly on its source -- the best\n\
              case for traversal from that seed. Compare with 2026-08-15: 23 edges, 2.9% median.\n"
    );
    println!(
        "{:<22} {:>7} {:>7} {:>13} {:>11} {:>16}",
        "set", "size", "edges", "median pct", "crossover", "discriminates@5"
    );
    for key in ["shared:scatterhoard", "shared:coding-lessons"] {
        let scope = Scope::from_key(key).expect("a valid scope key");
        let (_, path) = copied_stores(scratch.path())
            .into_iter()
            .find(|(s, _)| *s == scope)
            .expect("a copied store");
        let store = Store::open_at(scope, &path).expect("open");
        let corpus = corpus_from(&path, &store, dimension);
        let summary = eval::summarise(&corpus);
        println!(
            "{key:<22} {:>7} {:>7} {:>12.2}% {:>11} {:>16}",
            summary.corpus_size,
            summary.measured,
            summary.median_percentile * 100.0,
            summary
                .crossover_at(5)
                .map_or("n/a".to_string(), |c| c.to_string()),
            summary.discriminates_at(5),
        );
    }
}

/// Do the walk's finds form a population the ranking can be cut between?
///
/// The hypothesis is that a `related_to` or `contradicts` target need not resemble the query at
/// all, so the walk's contributions should sit well below the content hits and leave a break the
/// content ranking on its own does not have.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_walked_memories_separate_from_content_hits() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    println!(
        "\nThe top 20 for each query, marked `W` where the walk reached the memory. If walked\n\
              memories were a separate population, the Ws would gather at one end.\n"
    );
    for (key, queries) in PER_SET {
        let scope = Scope::from_key(key).expect("a valid scope key");
        let (_, path) = copies
            .iter()
            .find(|(s, _)| *s == scope)
            .expect("a copied store");
        let mut store = Store::open_at(scope.clone(), path).expect("open");
        store.set_embedder(embedder.clone());

        for query in queries {
            let mut marks = String::new();
            let mut own_walked: Vec<f64> = Vec::new();
            let mut own_content: Vec<f64> = Vec::new();
            let mut known = Vec::new();
            while marks.len() < 20 {
                let mut refs = vec![&mut store];
                let recall =
                    recall_across(&mut refs, &Question::new(query, 20).already_holding(&known))
                        .expect("recall");
                if recall.hits.is_empty() {
                    break;
                }
                for hit in &recall.hits {
                    if hit.inherited > 0.0 {
                        marks.push('W');
                        own_walked.push(f64::from(hit.similarity));
                    } else {
                        marks.push('.');
                        own_content.push(f64::from(hit.similarity));
                    }
                    known.push(hit.memory.id);
                }
            }
            marks.truncate(20);
            let mean = |v: &[f64]| {
                if v.is_empty() {
                    f64::NAN
                } else {
                    v.iter().sum::<f64>() / v.len() as f64
                }
            };
            println!(
                "  {marks}  own-sim walked {:>6.3}  content {:>6.3}   {}",
                mean(&own_walked),
                mean(&own_content),
                &query[..query.len().min(44)]
            );
        }
    }
    println!(
        "\nRank 1 is leftmost. `own-sim` is mean similarity to the query, ignoring what was inherited."
    );
}

/// Why does a football query retrieve corporate bylaws?
///
/// Two candidate mechanisms, and they predict different things. **Vocabulary**: a static
/// bag-of-tokens model matches *rule*, *interpretation*, *league* against governance text, in which
/// case single words should carry the score. **Centring**: every vector has the corpus centroid
/// subtracted, and that centroid is dominated by the 700-odd `scatterhoard` memories -- so a
/// memory from an unrelated set and a query from an unrelated domain both point *away* from the
/// main corpus, and centring could be turning "not about this corpus" into a shared direction. If
/// that is it, the uncentred ranking will not show these hits.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_why_an_off_domain_query_matches() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let dimension = embedder.embed(&["d".to_string()]).expect("embed")[0].dimension();

    // The union, exactly as an unscoped recall sees it.
    let mut vectors = VectorSet::new(dimension);
    let mut origin: std::collections::HashMap<MemoryId, String> = std::collections::HashMap::new();
    let mut text: std::collections::HashMap<MemoryId, String> = std::collections::HashMap::new();
    for (scope, path) in &copies {
        let store = Store::open_at(scope.clone(), path).expect("open");
        let db = rusqlite::Connection::open(path).expect("open");
        let rows: Vec<(String, Vec<u8>)> = db
            .prepare("SELECT memory_id, vector FROM vectors")
            .expect("query")
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .expect("read")
            .filter_map(Result::ok)
            .collect();
        for (id, blob) in rows {
            let Ok(parsed) = MemoryId::parse(&id) else {
                continue;
            };
            let components: Vec<f32> = blob
                .chunks_exact(4)
                .map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]]))
                .collect();
            let Ok(vector) = Vector::new(components) else {
                continue;
            };
            let _ = vectors.insert(parsed, vector);
            origin.insert(parsed, scope.as_key());
            if let Ok(Some(memory)) = store.get(parsed) {
                text.insert(
                    parsed,
                    memory
                        .content
                        .as_data()
                        .split_whitespace()
                        .take(9)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
        }
    }
    let centroid = embedding::combined_centroid(&[&vectors]).expect("a centroid");

    let query = "premier league offside rule interpretation";
    let vector = embedder
        .embed(&[query.to_string()])
        .expect("embed")
        .remove(0);

    println!("\nQUERY {query:?} over {} vectors\n", vectors.len());
    for (label, hits) in [
        (
            "CENTRED (what recall uses)",
            vectors.nearest_centred_on(&vector, 6, &centroid),
        ),
        (
            "UNCENTRED (raw cosine)",
            vectors.nearest_uncentred(&vector, 6),
        ),
    ] {
        println!("  {label}");
        for hit in &hits {
            println!(
                "    {:.3}  [{:<22}] {}",
                hit.similarity,
                origin.get(&hit.id).map_or("?", String::as_str),
                text.get(&hit.id).map_or("?", String::as_str)
            );
        }
        println!();
    }

    println!("  WORD BY WORD -- each word alone, centred, and which set its best hit is in");
    for word in query.split_whitespace() {
        let one = embedder
            .embed(&[word.to_string()])
            .expect("embed")
            .remove(0);
        let hits = vectors.nearest_centred_on(&one, 1, &centroid);
        println!(
            "    {word:<15} {:.3}  [{}]",
            hits[0].similarity,
            origin.get(&hits[0].id).map_or("?", String::as_str)
        );
    }

    println!("\n  HUBS -- mean centred similarity across all nine probe queries, top 6");
    let mut totals: Vec<(MemoryId, f64)> = Vec::new();
    let queries: Vec<Vector> = QUERIES
        .iter()
        .map(|(_, q)| {
            embedder
                .embed(&[(*q).to_string()])
                .expect("embed")
                .remove(0)
        })
        .collect();
    for query in &queries {
        for hit in vectors.nearest_centred_on(query, vectors.len(), &centroid) {
            match totals.iter_mut().find(|(id, _)| *id == hit.id) {
                Some(entry) => entry.1 += f64::from(hit.similarity),
                None => totals.push((hit.id, f64::from(hit.similarity))),
            }
        }
    }
    totals.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (id, total) in totals.iter().take(6) {
        println!(
            "    {:.3}  [{:<22}] {}",
            total / queries.len() as f64,
            origin.get(id).map_or("?", String::as_str),
            text.get(id).map_or("?", String::as_str)
        );
    }
    let governance = governance_scope();
    let governance_hubs = totals
        .iter()
        .take(20)
        .filter(|(id, _)| origin.get(id).is_some_and(|s| *s == governance))
        .count();
    println!(
        "\n  {governance_hubs} of the top 20 hubs are {governance} memories, out of 9 in a corpus of {}.",
        vectors.len()
    );
}

/// The two controls the off-domain investigation needs before its numbers mean anything.
///
/// **What does nonsense score?** If a string with no meaning also finds a best match around 0.25,
/// then 0.25 is the expected maximum cosine over a corpus this size and not evidence of a
/// vocabulary collision -- and the whole "these words happen to match governance text" reading is
/// wrong. **And are the responsible memories hubs?** Measured properly this time, by k-occurrence:
/// every memory is used as a query in turn and each memory counts how often it lands in another's
/// top 10. Averaging over a handful of hand-written queries measures the queries, not the corpus.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_controls_for_the_off_domain_match() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let dimension = embedder.embed(&["d".to_string()]).expect("embed")[0].dimension();

    let mut vectors = VectorSet::new(dimension);
    let mut origin: std::collections::HashMap<MemoryId, String> = std::collections::HashMap::new();
    for (scope, path) in &copies {
        let db = rusqlite::Connection::open(path).expect("open");
        let rows: Vec<(String, Vec<u8>)> = db
            .prepare("SELECT memory_id, vector FROM vectors")
            .expect("query")
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .expect("read")
            .filter_map(Result::ok)
            .collect();
        for (id, blob) in rows {
            let Ok(parsed) = MemoryId::parse(&id) else {
                continue;
            };
            let components: Vec<f32> = blob
                .chunks_exact(4)
                .map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]]))
                .collect();
            let Ok(vector) = Vector::new(components) else {
                continue;
            };
            let _ = vectors.insert(parsed, vector);
            origin.insert(parsed, scope.as_key());
        }
    }
    let centroid = embedding::combined_centroid(&[&vectors]).expect("a centroid");

    println!("\n  CONTROL -- what a string with no meaning scores, top hit, centred");
    for probe in [
        "zzqxlp",
        "qwertyuiop asdfghjkl",
        "xyzzy plugh frotz",
        "the of and to in a",
        "premier league offside rule interpretation",
        "rule",
    ] {
        let vector = embedder
            .embed(&[probe.to_string()])
            .expect("embed")
            .remove(0);
        let hits = vectors.nearest_centred_on(&vector, 1, &centroid);
        println!(
            "    {:<44} {:.3}  [{}]",
            format!("{probe:?}"),
            hits[0].similarity,
            origin.get(&hits[0].id).map_or("?", String::as_str)
        );
    }

    println!("\n  HUBNESS by k-occurrence -- every memory used as a query, k = 10");
    let ids: Vec<MemoryId> = origin.keys().copied().collect();
    let mut occurrences: std::collections::HashMap<MemoryId, usize> =
        std::collections::HashMap::new();
    for id in &ids {
        let Some(vector) = vectors.get(*id) else {
            continue;
        };
        for hit in vectors.nearest_centred_on(&vector, 11, &centroid) {
            if hit.id != *id {
                *occurrences.entry(hit.id).or_default() += 1;
            }
        }
    }
    let mut ranked: Vec<(MemoryId, usize)> = occurrences.into_iter().collect();
    ranked.sort_by_key(|(_, occurrences)| std::cmp::Reverse(*occurrences));
    println!(
        "    top 5 by k-occurrence: {:?}",
        ranked
            .iter()
            .take(5)
            .map(|(id, n)| (origin.get(id).map_or("?", String::as_str), n))
            .collect::<Vec<_>>()
    );
    let governance = governance_scope();
    let park: Vec<usize> = ranked
        .iter()
        .filter(|(id, _)| origin.get(id).is_some_and(|s| *s == governance))
        .map(|(_, n)| *n)
        .collect();
    let all_mean = ranked.iter().map(|(_, n)| *n as f64).sum::<f64>() / ranked.len() as f64;
    let park_mean = if park.is_empty() {
        0.0
    } else {
        park.iter().sum::<usize>() as f64 / park.len() as f64
    };
    println!(
        "    mean k-occurrence: whole corpus {all_mean:.2}, {governance} memories {park_mean:.2} over {} of them",
        park.len()
    );
    println!(
        "    best {governance} rank in the hub list: {:?}",
        ranked
            .iter()
            .position(|(id, _)| origin.get(id).is_some_and(|s| *s == governance))
    );
}

/// Is the *coherence* of a result set a signal where its scores are not?
///
/// Suggested by the controls: gibberish scores as high as a real question, so the top score says
/// nothing. But a real query should pull back memories that resemble **each other** -- they are
/// answers to one question -- where a meaningless query pulls back whatever happened to be nearest
/// in ten unrelated directions. That is not a statistic computed from the query's similarities, so
/// it is outside the family already ruled out.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_result_set_coherence_separates_a_non_match() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let dimension = embedder.embed(&["d".to_string()]).expect("embed")[0].dimension();

    let mut vectors = VectorSet::new(dimension);
    for (_, path) in &copies {
        let db = rusqlite::Connection::open(path).expect("open");
        let rows: Vec<(String, Vec<u8>)> = db
            .prepare("SELECT memory_id, vector FROM vectors")
            .expect("query")
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .expect("read")
            .filter_map(Result::ok)
            .collect();
        for (id, blob) in rows {
            let Ok(parsed) = MemoryId::parse(&id) else {
                continue;
            };
            let components: Vec<f32> = blob
                .chunks_exact(4)
                .map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]]))
                .collect();
            let Ok(vector) = Vector::new(components) else {
                continue;
            };
            let _ = vectors.insert(parsed, vector);
        }
    }
    let centroid = embedding::combined_centroid(&[&vectors]).expect("a centroid");

    println!("\n{:<46} {:>7} {:>10}", "query", "top1", "coherence");
    let cases: Vec<(&str, &str)> = QUERIES
        .iter()
        .copied()
        .chain([
            ("gibberish", "qwertyuiop asdfghjkl"),
            ("gibberish", "zzqxlp"),
            ("gibberish", "xyzzy plugh frotz"),
        ])
        .collect();
    for (class, query) in cases {
        let vector = embedder
            .embed(&[query.to_string()])
            .expect("embed")
            .remove(0);
        let hits = vectors.nearest_centred_on(&vector, 10, &centroid);

        // Mean pairwise similarity among the ten results themselves, centred the same way.
        let mut pairs = Vec::new();
        for (at, hit) in hits.iter().enumerate() {
            let Some(one) = vectors.get(hit.id) else {
                continue;
            };
            let mut just_these = VectorSet::new(dimension);
            for other in hits.iter().skip(at + 1) {
                if let Some(vector) = vectors.get(other.id) {
                    let _ = just_these.insert(other.id, vector);
                }
            }
            if just_these.is_empty() {
                continue;
            }
            let scored = just_these.nearest_centred_on(&one, just_these.len(), &centroid);
            pairs.extend(scored.into_iter().map(|hit| f64::from(hit.similarity)));
        }
        let coherence = pairs.iter().sum::<f64>() / pairs.len() as f64;
        println!(
            "{:<46} {:>7.3} {:>10.3}   {class}",
            &query[..query.len().min(46)],
            hits[0].similarity,
            coherence
        );
    }
    println!("\n`coherence` is the mean pairwise similarity among the ten results themselves.");
}

fn norm(v: &[f32]) -> f64 {
    v.iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt()
}

fn cosine_of(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    dot / (norm(a) * norm(b))
}

/// Is the model itself sane, and is cosine hiding the signal that would separate gibberish?
///
/// Two questions in one probe because they are the two places the fault could be. **The model**:
/// classic word pairs should separate, and if they do not, the fault is in loading or running it.
/// **Our use of it**: cosine discards magnitude, and magnitude is exactly *how far this text sits
/// from the average memory*. A query that lands near the corpus centroid has almost no direction
/// left after centring, yet cosine divides that tiny vector by its own tiny norm and reports a
/// confident number. If gibberish sits nearer the centroid than a real question does, the floor
/// nobody could find in the similarities is sitting in the norms.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_the_model_and_what_cosine_discards() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };

    println!("\n1. IS THE MODEL SANE? raw cosine between single words, no corpus involved");
    let pairs: [(&str, &str, &str); 8] = [
        ("near", "king", "queen"),
        ("far ", "king", "bicycle"),
        ("near", "dog", "puppy"),
        ("far ", "dog", "algebra"),
        ("near", "car", "automobile"),
        ("far ", "car", "lettuce"),
        ("near", "memory", "recall"),
        ("far ", "memory", "sourdough"),
    ];
    for (expect, a, b) in pairs {
        let vectors = embedder
            .embed(&[a.to_string(), b.to_string()])
            .expect("embed");
        println!(
            "   {expect} cos({a:<10}, {:<12}) = {:.3}",
            b,
            cosine_of(vectors[0].components(), vectors[1].components())
        );
    }

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let dimension = embedder.embed(&["d".to_string()]).expect("embed")[0].dimension();
    let mut vectors = VectorSet::new(dimension);
    for (_, path) in &copies {
        let db = rusqlite::Connection::open(path).expect("open");
        let rows: Vec<(String, Vec<u8>)> = db
            .prepare("SELECT memory_id, vector FROM vectors")
            .expect("query")
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .expect("read")
            .filter_map(Result::ok)
            .collect();
        for (id, blob) in rows {
            let Ok(parsed) = MemoryId::parse(&id) else {
                continue;
            };
            let components: Vec<f32> = blob
                .chunks_exact(4)
                .map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]]))
                .collect();
            let Ok(vector) = Vector::new(components) else {
                continue;
            };
            let _ = vectors.insert(parsed, vector);
        }
    }
    let centroid = embedding::combined_centroid(&[&vectors]).expect("a centroid");

    println!("\n2. WHAT COSINE DISCARDS -- how far each query sits from the corpus centroid");
    println!(
        "   {:<44} {:>7} {:>10} {:>10}",
        "query", "top1", "||q-c||", "cos(q,c)"
    );
    let cases: Vec<(&str, &str)> = QUERIES
        .iter()
        .copied()
        .chain([
            ("gibberish", "qwertyuiop asdfghjkl"),
            ("gibberish", "zzqxlp"),
            ("gibberish", "xyzzy plugh frotz"),
        ])
        .collect();
    for (class, query) in cases {
        let vector = embedder
            .embed(&[query.to_string()])
            .expect("embed")
            .remove(0);
        let centred: Vec<f32> = vector
            .components()
            .iter()
            .zip(centroid.components())
            .map(|(q, c)| q - c)
            .collect();
        let top = vectors.nearest_centred_on(&vector, 1, &centroid)[0].similarity;
        println!(
            "   {:<44} {top:>7.3} {:>10.4} {:>10.3}   {class}",
            &query[..query.len().min(44)],
            norm(&centred) / norm(vector.components()),
            cosine_of(vector.components(), centroid.components()),
        );
    }
    println!(
        "\n   ||q-c|| is normalised by ||q||, so it reads as *what fraction of this query is not the corpus average*."
    );
}

// ---------------------------------------------------------------------------------------------
// A scoring bench: the same corpus under several scoring rules, at several scales.
//
// **The answer key is `context`.** Every memory carries one, it is deliberately never embedded, and
// it was written by the same author about that memory -- so it is a held-out query with a known
// right answer, and there are hundreds of them. That is what makes a scale sweep possible at all:
// a hand-labelled key exists for one 45-memory corpus from 2026-07-31 and for nothing since.
//
// Its limits, stated because they bound every number below. A context is an episodic anchor rather
// than a description, so it is a *hard* query -- these figures are floors, not estimates. And
// memories stored in one batch often share a context word for word, so the memory a context came
// from is not always the only right answer; scoring only that one memory makes this a known-item
// test and understates every variant equally.
//
// Run these with `--release`: the CSLS penalty is a full pairwise pass.
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scoring {
    Raw,
    Centred,
    CentredCsls,
}

impl Scoring {
    fn label(self) -> &'static str {
        match self {
            Self::Raw => "raw cosine",
            Self::Centred => "centred (ships)",
            Self::CentredCsls => "centred + CSLS",
        }
    }
}

/// A corpus held as plain vectors, so a scoring rule can be swapped without touching the store.
struct Bench {
    ids: Vec<MemoryId>,
    raw: Vec<Vec<f32>>,
    centroid: Vec<f32>,
    /// Each memory's mean centred similarity to its `k` nearest neighbours -- the CSLS penalty.
    penalty: Vec<f64>,
}

const CSLS_K: usize = 10;

impl Bench {
    fn build(entries: Vec<(MemoryId, Vec<f32>)>) -> Self {
        let dimension = entries.first().map_or(0, |(_, v)| v.len());
        let mut centroid = vec![0.0f32; dimension];
        for (_, vector) in &entries {
            for (slot, value) in centroid.iter_mut().zip(vector) {
                *slot += value;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let count = entries.len() as f32;
        for slot in &mut centroid {
            *slot /= count;
        }

        let (ids, raw): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
        let centred: Vec<Vec<f32>> = raw
            .iter()
            .map(|vector| vector.iter().zip(&centroid).map(|(v, c)| v - c).collect())
            .collect();

        // r_k(m): the mean similarity of each memory to its k nearest *other* memories.
        let penalty = centred
            .iter()
            .map(|one| {
                let mut neighbours: Vec<f64> = centred
                    .iter()
                    .filter(|other| !std::ptr::eq(one.as_slice(), other.as_slice()))
                    .map(|other| cosine_of(one, other))
                    .collect();
                neighbours.sort_by(|a, b| b.total_cmp(a));
                let take = CSLS_K.min(neighbours.len());
                if take == 0 {
                    0.0
                } else {
                    #[allow(clippy::cast_precision_loss)]
                    let n = take as f64;
                    neighbours[..take].iter().sum::<f64>() / n
                }
            })
            .collect();

        Self {
            ids,
            raw,
            centroid,
            penalty,
        }
    }

    fn size(&self) -> usize {
        self.ids.len()
    }

    /// Every memory scored against one query, best first.
    fn rank(&self, query: &[f32], scoring: Scoring) -> Vec<(MemoryId, f64)> {
        let centred_query: Vec<f32> = query
            .iter()
            .zip(&self.centroid)
            .map(|(q, c)| q - c)
            .collect();
        let mut scored: Vec<(MemoryId, f64)> = self
            .ids
            .iter()
            .enumerate()
            .map(|(at, id)| {
                let score = match scoring {
                    Scoring::Raw => cosine_of(query, &self.raw[at]),
                    Scoring::Centred | Scoring::CentredCsls => {
                        let centred: Vec<f32> = self.raw[at]
                            .iter()
                            .zip(&self.centroid)
                            .map(|(v, c)| v - c)
                            .collect();
                        let similarity = cosine_of(&centred_query, &centred);
                        if scoring == Scoring::CentredCsls {
                            // r_k(query) is constant across memories, so it cannot change this
                            // ordering -- it is omitted here and matters only to a threshold.
                            2.0f64.mul_add(similarity, -self.penalty[at])
                        } else {
                            similarity
                        }
                    }
                };
                (*id, score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored
    }

    /// Worst and mean k-occurrence: how often each memory lands in another's top ten.
    fn hubness(&self, scoring: Scoring) -> (usize, f64) {
        let mut occurrences: HashMap<MemoryId, usize> = HashMap::new();
        for (at, id) in self.ids.iter().enumerate() {
            for (other, _) in self.rank(&self.raw[at], scoring).into_iter().take(11) {
                if other != *id {
                    *occurrences.entry(other).or_default() += 1;
                }
            }
        }
        let worst = occurrences.values().copied().max().unwrap_or(0);
        #[allow(clippy::cast_precision_loss)]
        let mean = occurrences.values().sum::<usize>() as f64 / self.size().max(1) as f64;
        (worst, mean)
    }
}

/// A scope's memories: the vectors to score, and the contexts that serve as the answer key.
type Loaded = (Vec<(MemoryId, Vec<f32>)>, Vec<(MemoryId, String)>);

/// Every memory in a scope, with its vector and its context.
fn entries_and_contexts(scope: &Scope, path: &Path) -> Loaded {
    let store = Store::open_at(scope.clone(), path).expect("open a copied store");
    let db = rusqlite::Connection::open(path).expect("open");
    let rows: Vec<(String, Vec<u8>)> = db
        .prepare("SELECT memory_id, vector FROM vectors")
        .expect("query")
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .expect("read")
        .filter_map(Result::ok)
        .collect();
    let mut entries = Vec::new();
    let mut contexts = Vec::new();
    for (id, blob) in rows {
        let Ok(parsed) = MemoryId::parse(&id) else {
            continue;
        };
        let components: Vec<f32> = blob
            .chunks_exact(4)
            .map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]]))
            .collect();
        entries.push((parsed, components));
        if let Ok(Some(memory)) = store.get(parsed)
            && let Some(context) = memory.context.as_ref()
        {
            contexts.push((parsed, context.as_data().to_string()));
        }
    }
    (entries, contexts)
}

/// A named corpus with its vectors and its answer key.
type Corpus = (String, Vec<(MemoryId, Vec<f32>)>, Vec<(MemoryId, String)>);

/// A named corpus with just the query strings that have a unique answer in it.
type Labelled = (String, Vec<(MemoryId, Vec<f32>)>, Vec<String>);

/// The five scales this machine can offer, smallest first.
fn scales() -> [String; 4] {
    [
        governance_scope(),
        "local".to_string(),
        "shared:coding-lessons".to_string(),
        "shared:scatterhoard".to_string(),
    ]
}

/// CSLS and centring re-measured at four corpus sizes plus the union.
///
/// The CSLS rejection of 2026-07-31 was taken on 45 memories and flagged in `EARLY-VALIDATION.md`
/// as untrustworthy for exactly that reason: `k=10` over 45 memories computes the penalty across a
/// fifth of the corpus. This runs it from 9 memories to 855, which is what the entry asked for.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_scoring_rules_across_corpus_scales() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    let mut corpora: Vec<Corpus> = Vec::new();
    let mut union_entries = Vec::new();
    let mut union_contexts = Vec::new();
    for key in scales() {
        let scope = Scope::from_key(&key).expect("a scope key");
        let (_, path) = copies
            .iter()
            .find(|(s, _)| *s == scope)
            .expect("a copied store");
        let (entries, contexts) = entries_and_contexts(&scope, path);
        union_entries.extend(entries.iter().cloned());
        union_contexts.extend(contexts.iter().cloned());
        corpora.push((key.to_string(), entries, contexts));
    }
    corpora.push((
        "UNION (all four)".to_string(),
        union_entries,
        union_contexts,
    ));

    println!(
        "\nKnown-item retrieval: each memory's `context` as the query, its own rank as the score.\n\
              Hubness is k-occurrence at k=10 -- how often the worst offender lands in another's top ten.\n"
    );
    println!(
        "{:<24} {:>5} {:>7} {:<16} {:>6} {:>6} {:>6} {:>8} {:>6}",
        "corpus", "size", "queries", "scoring", "r@1", "r@5", "r@10", "median", "hub"
    );

    for (name, entries, contexts) in &corpora {
        let bench = Bench::build(entries.clone());
        // **Only memories whose context is unique in this corpus.** Batch-migrated memories share a
        // context word for word -- one string is on 224 of them -- and asking which single memory a
        // context came from when 224 share it is unanswerable by construction, not badly retrieved.
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (_, context) in contexts {
            *seen.entry(context.as_str()).or_default() += 1;
        }
        let unique: Vec<&(MemoryId, String)> = contexts
            .iter()
            .filter(|(_, context)| seen[context.as_str()] == 1)
            .collect();
        // Spread the sample rather than taking a prefix: ids are chronological, so a prefix is the
        // oldest memories, and on the union it is whichever set was loaded first.
        let stride = (unique.len() / 150).max(1);
        let sample: Vec<&(MemoryId, String)> =
            unique.into_iter().step_by(stride).take(150).collect();
        for scoring in [Scoring::Raw, Scoring::Centred, Scoring::CentredCsls] {
            let mut ranks: Vec<usize> = Vec::new();
            for (id, context) in &sample {
                let query = embedder
                    .embed(std::slice::from_ref(context))
                    .expect("embed")
                    .remove(0);
                let ranked = bench.rank(query.components(), scoring);
                ranks.push(
                    ranked
                        .iter()
                        .position(|(other, _)| other == id)
                        .map_or(9999, |at| at + 1),
                );
            }
            ranks.sort_unstable();
            let share = |k: usize| {
                #[allow(clippy::cast_precision_loss)]
                let out =
                    ranks.iter().filter(|r| **r <= k).count() as f64 / ranks.len().max(1) as f64;
                out * 100.0
            };
            let (worst, mean) = bench.hubness(scoring);
            let _ = mean;
            println!(
                "{:<24} {:>5} {:>7} {:<16} {:>5.1}% {:>5.1}% {:>5.1}% {:>8} {:>6}",
                if scoring == Scoring::Raw {
                    name.as_str()
                } else {
                    ""
                },
                if scoring == Scoring::Raw {
                    bench.size().to_string()
                } else {
                    String::new()
                },
                if scoring == Scoring::Raw {
                    sample.len().to_string()
                } else {
                    String::new()
                },
                scoring.label(),
                share(1),
                share(5),
                share(10),
                ranks[ranks.len() / 2],
                worst
            );
        }
        println!();
    }
}

/// The same scoring rules over one set subsampled to a ladder of sizes.
///
/// The cross-set sweep confounds scale with content: `coding-lessons` at 121 is both a size and a
/// particular corpus, so *centring hurts at 121* and *centring hurts on coding-lessons* cannot be
/// told apart there. Subsampling one set holds the content fixed and varies only the size, which is
/// the question a user growing their own set is actually asking.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_scoring_rules_as_one_set_grows() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let scope = Scope::from_key("shared:scatterhoard").expect("a scope key");
    let (_, path) = copies
        .iter()
        .find(|(s, _)| *s == scope)
        .expect("a copied store");
    let (entries, contexts) = entries_and_contexts(&scope, path);

    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (_, context) in &contexts {
        *seen.entry(context.as_str()).or_default() += 1;
    }
    let unique: Vec<&(MemoryId, String)> = contexts
        .iter()
        .filter(|(_, context)| seen[context.as_str()] == 1)
        .collect();

    println!(
        "\nOne set, subsampled. Content held fixed, size varied -- so any trend here is scale.\n\
              Each subsample is spread across the corpus, never a prefix, and every query's answer\n\
              is inside its own subsample.\n"
    );
    println!(
        "{:<8} {:>7} {:<16} {:>6} {:>6} {:>6} {:>8} {:>6}",
        "size", "queries", "scoring", "r@1", "r@5", "r@10", "median", "hub"
    );

    for size in [30usize, 60, 121, 250, 480, entries.len()] {
        let stride = (entries.len() / size).max(1);
        let subset: Vec<(MemoryId, Vec<f32>)> =
            entries.iter().step_by(stride).take(size).cloned().collect();
        let present: std::collections::HashSet<MemoryId> =
            subset.iter().map(|(id, _)| *id).collect();
        let bench = Bench::build(subset);

        let askable: Vec<&&(MemoryId, String)> = unique
            .iter()
            .filter(|(id, _)| present.contains(id))
            .collect();
        if askable.is_empty() {
            println!(
                "{:<8} {:>7} -- no unique context inside this subsample",
                bench.size(),
                0
            );
            continue;
        }
        for scoring in [Scoring::Raw, Scoring::Centred, Scoring::CentredCsls] {
            let mut ranks: Vec<usize> = Vec::new();
            for (id, context) in askable.iter().copied() {
                let query = embedder
                    .embed(std::slice::from_ref(context))
                    .expect("embed")
                    .remove(0);
                let ranked = bench.rank(query.components(), scoring);
                ranks.push(
                    ranked
                        .iter()
                        .position(|(other, _)| other == id)
                        .map_or(9999, |at| at + 1),
                );
            }
            ranks.sort_unstable();
            let share = |k: usize| {
                #[allow(clippy::cast_precision_loss)]
                let out =
                    ranks.iter().filter(|r| **r <= k).count() as f64 / ranks.len().max(1) as f64;
                out * 100.0
            };
            let (worst, _) = bench.hubness(scoring);
            println!(
                "{:<8} {:>7} {:<16} {:>5.1}% {:>5.1}% {:>5.1}% {:>8} {:>6}",
                if scoring == Scoring::Raw {
                    bench.size().to_string()
                } else {
                    String::new()
                },
                if scoring == Scoring::Raw {
                    ranks.len().to_string()
                } else {
                    String::new()
                },
                scoring.label(),
                share(1),
                share(5),
                share(10),
                ranks[ranks.len() / 2],
                worst
            );
        }
        println!();
    }
}

/// Hand-written queries from domains this machine's sets do not cover.
const OFF_DOMAIN: [&str; 12] = [
    "how long to proof sourdough at room temperature",
    "replacing the brake pads on a road bicycle",
    "symptoms of vitamin D deficiency in adults",
    "best time of year to visit the Scottish highlands",
    "how to prune an apple tree in winter",
    "the offside rule in association football",
    "tuning a guitar to drop D",
    "why bread dough needs a second rise",
    "how tides are caused by the moon",
    "choosing between diesel and petrol for towing",
    "what to feed a rescue greyhound",
    "how to read a knitting pattern",
];

/// Does `cos(query, centroid)` separate answerable from unanswerable, and does it hold as a set grows?
///
/// The pilot on twelve queries put a boundary near 0.19. This asks it properly: every unique
/// context in a corpus as the answerable class, and three sources of unanswerable — other sets'
/// contexts, which control for author and register; hand-written off-domain questions; and
/// gibberish. Run at each scale, because a threshold that moves with corpus size is not a threshold.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_the_corpus_alignment_boundary_across_scales() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    let mut per_scope: Vec<Labelled> = Vec::new();
    for key in scales() {
        let scope = Scope::from_key(&key).expect("a scope key");
        let (_, path) = copies
            .iter()
            .find(|(s, _)| *s == scope)
            .expect("a copied store");
        let (entries, contexts) = entries_and_contexts(&scope, path);
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (_, context) in &contexts {
            *seen.entry(context.as_str()).or_default() += 1;
        }
        let unique: Vec<String> = contexts
            .iter()
            .filter(|(_, context)| seen[context.as_str()] == 1)
            .map(|(_, context)| context.clone())
            .collect();
        per_scope.push((key.to_string(), entries, unique));
    }

    println!(
        "\ncos(query, centroid) as a separator. `in` is that corpus's own unique contexts;\n\
              `out` is other sets' contexts, twelve hand-written off-domain questions, and gibberish.\n\
              `best` is the threshold with fewest errors, and `errors` how many it still makes.\n"
    );
    println!(
        "{:<24} {:>5} {:>5} {:>5} {:>8} {:>8} {:>7} {:>18}",
        "corpus", "size", "in", "out", "med in", "med out", "best", "errors at best"
    );

    for (at, (name, entries, inside)) in per_scope.iter().enumerate() {
        let bench = Bench::build(entries.clone());
        let stride = (inside.len() / 120).max(1);
        let answerable: Vec<&String> = inside.iter().step_by(stride).take(120).collect();

        let mut unanswerable: Vec<String> = OFF_DOMAIN.iter().map(|q| (*q).to_string()).collect();
        unanswerable.extend([
            "qwertyuiop asdfghjkl".to_string(),
            "zzqxlp".to_string(),
            "xyzzy plugh frotz".to_string(),
        ]);
        for (index, (_, _, contexts)) in per_scope.iter().enumerate() {
            if index != at {
                let stride = (contexts.len() / 20).max(1);
                unanswerable.extend(contexts.iter().step_by(stride).take(20).cloned());
            }
        }

        let alignment = |query: &str| -> f64 {
            let vector = embedder
                .embed(&[query.to_string()])
                .expect("embed")
                .remove(0);
            cosine_of(vector.components(), &bench.centroid)
        };
        let mut scores_in: Vec<f64> = answerable.iter().map(|q| alignment(q)).collect();
        let mut scores_out: Vec<f64> = unanswerable.iter().map(|q| alignment(q)).collect();
        scores_in.sort_by(f64::total_cmp);
        scores_out.sort_by(f64::total_cmp);

        // The threshold with the fewest misclassifications, searched over every observed value.
        let mut candidates: Vec<f64> = scores_in.iter().chain(&scores_out).copied().collect();
        candidates.sort_by(f64::total_cmp);
        let (best, missed, wrong) = candidates
            .iter()
            .map(|cut| {
                let missed = scores_in.iter().filter(|s| *s < cut).count();
                let wrong = scores_out.iter().filter(|s| *s >= cut).count();
                (*cut, missed, wrong)
            })
            .min_by_key(|(_, missed, wrong)| missed + wrong)
            .expect("at least one candidate");

        println!(
            "{name:<24} {:>5} {:>5} {:>5} {:>8.3} {:>8.3} {:>7.3} {:>18}",
            bench.size(),
            scores_in.len(),
            scores_out.len(),
            scores_in[scores_in.len() / 2],
            scores_out[scores_out.len() / 2],
            best,
            format!("{missed} missed, {wrong} wrong")
        );
    }
}

/// Reproduce the published spike figures on the real 45-memory fixture.
///
/// `corpus/seed-quality-provenance.md` says this was never re-run: the numbers in
/// `documents/design/embeddings.md` were identified by arithmetic consistency rather than by
/// repetition, and the note asks for exactly this. It is also the only validation available for the
/// `Bench` above -- if this reproduces figures a different implementation produced two months ago,
/// the bench's other numbers can be trusted; if it does not, they cannot.
///
/// Targets, from the published table:
///   raw cosine       r@1 39.2  r@3 65.8  r@5 71.7  r@10 83.3   worst hub 13
///   centroid removed r@1 39.2  r@3 66.7  r@5 77.5  r@10 87.5   worst hub  6
///   CSLS alone       r@1 39.2  r@3 55.8  r@5 71.7  r@10 85.8   worst hub 10
///   centroid + CSLS  r@1 36.7  r@3 59.2  r@5 78.3  r@10 87.5   worst hub  5
///
/// Recall is per-query and averaged: relevance lists hold one to three ids, so each query
/// contributes a unit, a half or a third, and 71.7% is 14.3333 of 20.
#[test]
#[ignore = "needs SCATTERHOARD_MODEL; reads corpus/seed-quality-corpus.json"]
fn probe_reproduce_the_seed_quality_figures() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let raw = std::fs::read_to_string("corpus/seed-quality-corpus.json").expect("the fixture");
    let fixture: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");

    // `embedding_input`'s composition, rebuilt here because the fixture holds fields rather than
    // `Memory` values: content, then `why`, then keywords, space-separated.
    let mut ids = Vec::new();
    let mut texts = Vec::new();
    for memory in fixture["memories"].as_array().expect("memories") {
        ids.push(memory["id"].as_str().expect("an id").to_string());
        let keywords: Vec<&str> = memory["keywords"]
            .as_array()
            .map(|list| list.iter().filter_map(serde_json::Value::as_str).collect())
            .unwrap_or_default();
        texts.push(format!(
            "{} {} {}",
            memory["content"].as_str().unwrap_or_default(),
            memory["why"].as_str().unwrap_or_default(),
            keywords.join(" ")
        ));
    }
    let vectors: Vec<Vec<f32>> = embedder
        .embed(&texts)
        .expect("embed the fixture")
        .into_iter()
        .map(|vector| vector.components().to_vec())
        .collect();

    // The bench is keyed by MemoryId, and these are UUIDs from before the id change -- so index by
    // position instead, which is all the fixture needs.
    let centroid: Vec<f32> = {
        let mut sum = vec![0.0f32; vectors[0].len()];
        for vector in &vectors {
            for (slot, value) in sum.iter_mut().zip(vector) {
                *slot += value;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let count = vectors.len() as f32;
        sum.into_iter().map(|total| total / count).collect()
    };
    let centred: Vec<Vec<f32>> = vectors
        .iter()
        .map(|vector| vector.iter().zip(&centroid).map(|(v, c)| v - c).collect())
        .collect();
    let penalty: Vec<f64> = centred
        .iter()
        .enumerate()
        .map(|(at, one)| {
            let mut near: Vec<f64> = centred
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != at)
                .map(|(_, other)| cosine_of(one, other))
                .collect();
            near.sort_by(|a, b| b.total_cmp(a));
            near[..CSLS_K.min(near.len())].iter().sum::<f64>() / CSLS_K.min(near.len()) as f64
        })
        .collect();

    println!(
        "\n{:<22} {:>7} {:>7} {:>7} {:>7} {:>10}",
        "variant", "r@1", "r@3", "r@5", "r@10", "worst hub"
    );
    // `both` is what ships -- `VectorSet::scan` centres the query and every memory. The other two
    // are the ways a centring can be implemented by accident, and are here to identify which
    // variant the published figures actually describe.
    for (label, centre_memories, centre_query, use_csls) in [
        ("raw cosine", false, false, false),
        ("centred: both (ships)", true, true, false),
        ("centred: memories only", true, false, false),
        ("centred: query only", false, true, false),
        ("CSLS alone", false, false, true),
        ("centred both + CSLS", true, true, true),
    ] {
        let use_centre = centre_memories;
        let mut totals = [0.0f64; 4];
        let mut appearances = vec![0usize; ids.len()];
        let queries = fixture["queries"].as_array().expect("queries");
        for query in queries {
            let text = query["q"].as_str().expect("a query");
            let relevant: Vec<&str> = query["relevant"]
                .as_array()
                .expect("relevant")
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect();
            let embedded = embedder
                .embed(&[text.to_string()])
                .expect("embed")
                .remove(0);
            let query_vector: Vec<f32> = if centre_query {
                embedded
                    .components()
                    .iter()
                    .zip(&centroid)
                    .map(|(q, c)| q - c)
                    .collect()
            } else {
                embedded.components().to_vec()
            };
            let mut ranked: Vec<(usize, f64)> = (0..ids.len())
                .map(|at| {
                    let side = if use_centre {
                        &centred[at]
                    } else {
                        &vectors[at]
                    };
                    let similarity = cosine_of(&query_vector, side);
                    let score = if use_csls {
                        2.0f64.mul_add(similarity, -penalty[at])
                    } else {
                        similarity
                    };
                    (at, score)
                })
                .collect();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1));

            for (slot, k) in [1usize, 3, 5, 10].into_iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let found = ranked[..k.min(ranked.len())]
                    .iter()
                    .filter(|(at, _)| relevant.contains(&ids[*at].as_str()))
                    .count() as f64;
                #[allow(clippy::cast_precision_loss)]
                let out = found / relevant.len() as f64;
                totals[slot] += out;
            }
            for (at, _) in ranked.iter().take(5) {
                appearances[*at] += 1;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let n = queries.len() as f64;
        println!(
            "{label:<23} {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>10}",
            totals[0] / n * 100.0,
            totals[1] / n * 100.0,
            totals[2] / n * 100.0,
            totals[3] / n * 100.0,
            appearances.iter().max().copied().unwrap_or(0)
        );
    }
    println!(
        "\npublished           raw 39.2 65.8 71.7 83.3 hub 13 | centred 39.2 66.7 77.5 87.5 hub 6"
    );
    println!(
        "                   CSLS 39.2 55.8 71.7 85.8 hub 10 | both    36.7 59.2 78.3 87.5 hub 5"
    );
}

/// The three scoring rules side by side on the live union, with the memories they return.
///
/// The metrics disagree about centring and there is no labelled corpus at this size, so the last
/// available judgement is reading the results. Queries are the ones this session has used
/// throughout, so the output can be compared against what was measured earlier under the rule that
/// ships.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_prototype_scoring_on_the_live_corpus() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    let mut entries = Vec::new();
    let mut origin: HashMap<MemoryId, String> = HashMap::new();
    let mut text: HashMap<MemoryId, String> = HashMap::new();
    for (scope, path) in &copies {
        let store = Store::open_at(scope.clone(), path).expect("open");
        let (loaded, _) = entries_and_contexts(scope, path);
        for (id, vector) in loaded {
            if let Ok(Some(memory)) = store.get(id) {
                text.insert(
                    id,
                    memory
                        .content
                        .as_data()
                        .split_whitespace()
                        .take(13)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            origin.insert(id, scope.as_key().replace("shared:", ""));
            entries.push((id, vector));
        }
    }
    let bench = Bench::build(entries);
    println!(
        "\nLive union, {} memories. Top five per rule, to be read rather than scored.\n",
        bench.size()
    );

    for query in [
        "what decides which memory set a memory belongs in",
        "how is a mutation counted as caught when the tests hang",
        "premier league offside rule interpretation",
        "qwertyuiop asdfghjkl",
    ] {
        println!("QUERY {query:?}");
        let vector = embedder
            .embed(&[query.to_string()])
            .expect("embed")
            .remove(0);
        for scoring in [Scoring::Raw, Scoring::Centred, Scoring::CentredCsls] {
            let ranked = bench.rank(vector.components(), scoring);
            let scores: Vec<f64> = ranked.iter().take(20).map(|(_, s)| *s).collect();
            let (largest, median, _) = gap_profile(&scores);
            println!(
                "  {:<16} top1 {:.3}  gap ratio {:.2}",
                scoring.label(),
                scores[0],
                if median > 0.0 { largest / median } else { 0.0 }
            );
            for (id, score) in ranked.iter().take(5) {
                println!("      {score:>7.3} [{:<16}] {}", origin[id], text[id]);
            }
        }
        println!();
    }
}

/// Does a query compared against each set's *charter* say which set it belongs to?
///
/// Suggested by the prototype run: on a routing query the centred ranking surfaced three sets'
/// `hoard-scope-topics` charters at the top, so charters are semantically reachable. A charter is a
/// deliberate statement of what a set is for, which is a different object from a centroid -- the
/// average of a set's text, which failed at scale because it measures register. `route-where`
/// already has a session match a memory against charters when routing; this asks whether a *query*
/// can be matched the same way.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_set_charters_can_place_a_query() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    let mut charters: Vec<(String, Vec<f32>)> = Vec::new();
    for (scope, path) in &copies {
        let db = rusqlite::Connection::open(path).expect("open");
        let found: Option<String> = db
            .prepare("SELECT memory_id FROM keywords WHERE keyword = 'hoard-scope-topics' LIMIT 1")
            .expect("query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("read")
            .filter_map(Result::ok)
            .next();
        let Some(id) = found else { continue };
        let blob: Option<Vec<u8>> = db
            .prepare("SELECT vector FROM vectors WHERE memory_id = ?1")
            .expect("query")
            .query_map([&id], |row| row.get::<_, Vec<u8>>(0))
            .expect("read")
            .filter_map(Result::ok)
            .next();
        let Some(blob) = blob else { continue };
        charters.push((
            scope.as_key().replace("shared:", ""),
            blob.chunks_exact(4)
                .map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]]))
                .collect(),
        ));
    }

    println!(
        "\nRaw cosine between a query and each set's charter. The claim under test is that this\n\
              places a query -- names which set it belongs to, or none.\n"
    );
    print!("{:<46}", "query");
    for (name, _) in &charters {
        print!(" {:>16}", name);
    }
    println!("   best");
    for query in [
        "what decides which memory set a memory belongs in",
        "how is a mutation counted as caught when the tests hang",
        "why a slow build might not be the tests themselves",
        "who may serve on the board committee",
        "premier league offside rule interpretation",
        "how to replace a bicycle inner tube",
        "qwertyuiop asdfghjkl",
    ] {
        let vector = embedder
            .embed(&[query.to_string()])
            .expect("embed")
            .remove(0);
        print!("{:<46}", &query[..query.len().min(46)]);
        let mut best = ("none", f64::MIN);
        for (name, charter) in &charters {
            let score = cosine_of(vector.components(), charter);
            print!(" {score:>16.3}");
            if score > best.1 {
                best = (name.as_str(), score);
            }
        }
        println!("   {}", best.0);
    }
}

/// Narrow technical questions against the charters -- the case the charter proposal has not met.
///
/// Every query the charter idea was tested on was broad or conceptual. The ordinary query a session
/// issues is narrow and carries identifiers: *when does the code return `Cycle`*. A charter is an
/// abstract statement of what a set is for, so a precise question may sit far from it while being
/// squarely answerable -- which would make charter alignment reject exactly the queries that matter
/// most. This asks whether that happens, and pairs it with whether the corpus can answer them.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_charters_against_narrow_technical_queries() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    let mut charters: Vec<(String, Vec<f32>)> = Vec::new();
    let mut entries = Vec::new();
    let mut origin: HashMap<MemoryId, String> = HashMap::new();
    let mut text: HashMap<MemoryId, String> = HashMap::new();
    for (scope, path) in &copies {
        let store = Store::open_at(scope.clone(), path).expect("open");
        let db = rusqlite::Connection::open(path).expect("open");
        if let Some(id) = db
            .prepare("SELECT memory_id FROM keywords WHERE keyword = 'hoard-scope-topics' LIMIT 1")
            .expect("query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("read")
            .filter_map(Result::ok)
            .next()
            && let Some(blob) = db
                .prepare("SELECT vector FROM vectors WHERE memory_id = ?1")
                .expect("query")
                .query_map([&id], |row| row.get::<_, Vec<u8>>(0))
                .expect("read")
                .filter_map(Result::ok)
                .next()
        {
            charters.push((
                scope.as_key().replace("shared:", ""),
                blob.chunks_exact(4)
                    .map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]]))
                    .collect(),
            ));
        }
        let (loaded, _) = entries_and_contexts(scope, path);
        for (id, vector) in loaded {
            if let Ok(Some(memory)) = store.get(id) {
                text.insert(
                    id,
                    memory
                        .content
                        .as_data()
                        .split_whitespace()
                        .take(11)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            origin.insert(id, scope.as_key().replace("shared:", ""));
            entries.push((id, vector));
        }
    }
    let bench = Bench::build(entries);

    println!(
        "\nNarrow questions a session actually asks, against charters and against the corpus.\n\
              `best charter` is the highest of the four; `top1` and the two hits are the centred\n\
              ranking that ships, so the last column says whether the corpus can in fact answer.\n"
    );
    for (class, query) in [
        ("narrow", "what is the link traversal mechanism"),
        ("narrow", "when does the code return Cycle"),
        (
            "narrow",
            "what does amend refuse to do after a memory has been exported",
        ),
        (
            "narrow",
            "which keyword prefix is reserved for control marks",
        ),
        ("broad", "what decides which memory set a memory belongs in"),
        ("off", "how to replace a bicycle inner tube"),
    ] {
        let vector = embedder
            .embed(&[query.to_string()])
            .expect("embed")
            .remove(0);
        let best = charters
            .iter()
            .map(|(name, charter)| (name.clone(), cosine_of(vector.components(), charter)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .expect("a charter");
        let ranked = bench.rank(vector.components(), Scoring::Centred);
        println!("[{class:<6}] {query:?}");
        println!(
            "    best charter {:.3} ({})   top1 {:.3}",
            best.1, best.0, ranked[0].1
        );
        for (id, score) in ranked.iter().take(2) {
            println!("      {score:>6.3} [{:<16}] {}", origin[id], text[id]);
        }
    }
}

/// Re-embed the live corpus with whatever model `SCATTERHOARD_MODEL` names, and run the failures.
///
/// The stored vectors were built by `potion-base-32M`, so comparing models means re-embedding
/// rather than reading the `vectors` table. Point the variable at another model and run this again:
/// `potion-retrieval-32M` and `potion-base-8M` are both cached on this machine and both load
/// through `model2vec-rs` unchanged.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_the_failures_under_whichever_model_is_configured() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    let mut ids = Vec::new();
    let mut texts = Vec::new();
    let mut origin: HashMap<MemoryId, String> = HashMap::new();
    for (scope, path) in &copies {
        let store = Store::open_at(scope.clone(), path).expect("open");
        let (loaded, _) = entries_and_contexts(scope, path);
        for (id, _) in loaded {
            if let Ok(Some(memory)) = store.get(id) {
                ids.push(id);
                texts.push(embedding::embedding_input(&memory));
                origin.insert(id, scope.as_key().replace("shared:", ""));
            }
        }
    }
    let vectors: Vec<Vec<f32>> = embedder
        .embed(&texts)
        .expect("re-embed the corpus")
        .into_iter()
        .map(|vector| vector.components().to_vec())
        .collect();
    let bench = Bench::build(ids.iter().copied().zip(vectors).collect());

    println!(
        "\nModel: {}",
        std::env::var("SCATTERHOARD_MODEL")
            .unwrap_or_default()
            .rsplit('/')
            .nth(2)
            .unwrap_or("?")
    );
    println!(
        "Re-embedded {} memories. Centred scoring, as ships.\n",
        bench.size()
    );
    println!(
        "{:<48} {:>7} {:<28}",
        "query", "top1", "sets in the top three"
    );
    for query in [
        "what is the link traversal mechanism",
        "when does the code return Cycle",
        "what decides which memory set a memory belongs in",
        "premier league offside rule interpretation",
        "how to replace a bicycle inner tube",
        "qwertyuiop asdfghjkl",
    ] {
        let vector = embedder
            .embed(&[query.to_string()])
            .expect("embed")
            .remove(0);
        let ranked = bench.rank(vector.components(), Scoring::Centred);
        let sets: Vec<&str> = ranked
            .iter()
            .take(3)
            .map(|(id, _)| origin[id].as_str())
            .collect();
        println!(
            "{:<48} {:>7.3} {:<28}",
            &query[..query.len().min(48)],
            ranked[0].1,
            sets.join(", ")
        );
    }
}

/// Load a model straight from this machine's Hugging Face cache, by repository name.
fn cached_model(name: &str) -> Option<Arc<StaticEmbedder>> {
    let home = std::env::var("HOME").ok()?;
    let root = PathBuf::from(home)
        .join(".cache/huggingface/hub")
        .join(format!("models--minishlab--{name}"))
        .join("snapshots");
    let snapshot = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .next()?;
    StaticEmbedder::load(&snapshot.path()).ok().map(Arc::new)
}

/// Rank a set of texts against a query, centred as recall does, and return ids best first.
fn ranked_ids(bench: &Bench, embedder: &StaticEmbedder, query: &str) -> Vec<MemoryId> {
    let vector = embedder
        .embed(&[query.to_string()])
        .expect("embed")
        .remove(0);
    bench
        .rank(vector.components(), Scoring::Centred)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// Precision at k, recall at k, and the reciprocal rank of the first relevant hit.
fn score_ranking(ranked: &[MemoryId], relevant: &[MemoryId], k: usize) -> (f64, f64, f64) {
    let top = &ranked[..k.min(ranked.len())];
    #[allow(clippy::cast_precision_loss)]
    let hits = top.iter().filter(|id| relevant.contains(id)).count() as f64;
    #[allow(clippy::cast_precision_loss)]
    let precision = if top.is_empty() {
        0.0
    } else {
        hits / top.len() as f64
    };
    #[allow(clippy::cast_precision_loss)]
    let recall = if relevant.is_empty() {
        0.0
    } else {
        hits / relevant.len() as f64
    };
    #[allow(clippy::cast_precision_loss)]
    let reciprocal = ranked
        .iter()
        .position(|id| relevant.contains(id))
        .map_or(0.0, |at| 1.0 / (at + 1) as f64);
    (precision, recall, reciprocal)
}

/// The trial `potion-retrieval-32M` has to pass before it is believed.
///
/// Designed against the lesson the day taught: **the hard cases go first, not last.** Every earlier
/// candidate survived whichever queries were lying around and died to one chosen deliberately, so
/// the queries that killed the previous four are the first thing reported here.
///
/// Then both labelled corpora, because neither is sufficient alone: `seed-quality-corpus.json` is
/// 45 *real* memories, and `traversal-corpus.jsonl` is 429 *synthetic* ones harvested from markdown
/// with 24 real queries labelled blind by two labellers. Agreement across both is the evidence;
/// disagreement is a reason to trust neither. Relevance lists here run to 40 of 429, so recall@k is
/// bounded by construction and precision@k and reciprocal rank carry the comparison.
#[test]
#[ignore = "reads the labelled corpora and needs both models cached"]
fn probe_trial_the_retrieval_tuned_model() {
    let Some(base) = cached_model("potion-base-32M") else {
        println!("potion-base-32M not cached -- nothing measured. This is a skip, not a pass.");
        return;
    };
    let Some(tuned) = cached_model("potion-retrieval-32M") else {
        println!(
            "potion-retrieval-32M not cached -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let models: [(&str, &StaticEmbedder); 2] =
        [("base-32M (ships)", &base), ("retrieval-32M", &tuned)];

    // ---- 1. The hard cases, first. ----
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let mut live_ids = Vec::new();
    let mut live_texts = Vec::new();
    let mut origin: HashMap<MemoryId, String> = HashMap::new();
    for (scope, path) in &copies {
        let store = Store::open_at(scope.clone(), path).expect("open");
        let (loaded, _) = entries_and_contexts(scope, path);
        for (id, _) in loaded {
            if let Ok(Some(memory)) = store.get(id) {
                live_ids.push(id);
                live_texts.push(embedding::embedding_input(&memory));
                origin.insert(id, scope.as_key().replace("shared:", ""));
            }
        }
    }

    println!("\n=== 1. THE CASES THAT KILLED THE PREVIOUS FOUR CANDIDATES ===\n");
    println!("{:<50} {:>18} {:>18}", "query", models[0].0, models[1].0);
    let mut benches = Vec::new();
    for (_, embedder) in models {
        let vectors: Vec<Vec<f32>> = embedder
            .embed(&live_texts)
            .expect("re-embed")
            .into_iter()
            .map(|v| v.components().to_vec())
            .collect();
        benches.push(Bench::build(
            live_ids.iter().copied().zip(vectors).collect(),
        ));
    }
    for (label, query) in [
        ("in-domain, narrow", "what is the link traversal mechanism"),
        ("in-domain, narrow", "when does the code return Cycle"),
        ("in-domain, one word", "measurement"),
        ("OFF-DOMAIN", "premier league offside rule interpretation"),
        ("OFF-DOMAIN", "how to replace a bicycle inner tube"),
        ("NONSENSE", "qwertyuiop asdfghjkl"),
    ] {
        print!(
            "{:<50}",
            format!("[{label}] {query}")
                .chars()
                .take(50)
                .collect::<String>()
        );
        for (at, (_, embedder)) in models.into_iter().enumerate() {
            let vector = embedder
                .embed(&[query.to_string()])
                .expect("embed")
                .remove(0);
            let ranked = benches[at].rank(vector.components(), Scoring::Centred);
            let park = ranked
                .iter()
                .take(3)
                .filter(|(id, _)| origin[id] == governance_set())
                .count();
            print!(
                " {:>12.3}{:>6}",
                ranked[0].1,
                if park > 0 {
                    format!("({park}pk)")
                } else {
                    String::new()
                }
            );
        }
        println!();
    }

    // ---- 1b. The separation at scale: 24 real queries against 15 that have no answer. ----
    println!("\n=== 1b. TOP-1 SEPARATION, 24 REAL HARVESTED QUERIES vs 15 UNANSWERABLE ===\n");
    let harvested: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string("corpus/harvested-queries.json").expect("harvested"),
    )
    .expect("JSON");
    let in_domain: Vec<String> = harvested["queries"]
        .as_array()
        .expect("queries")
        .iter()
        .filter_map(|q| q["query"].as_str())
        .map(ToString::to_string)
        .collect();
    let mut out_domain: Vec<String> = OFF_DOMAIN.iter().map(|q| (*q).to_string()).collect();
    out_domain.extend([
        "qwertyuiop asdfghjkl".to_string(),
        "zzqxlp".to_string(),
        "xyzzy plugh frotz".to_string(),
    ]);

    println!(
        "{:<18} {:>9} {:>9} {:>9} {:>9} {:>26}",
        "model", "in med", "in min", "out med", "out max", "overlap"
    );
    for (at, (name, embedder)) in models.into_iter().enumerate() {
        let top1 = |query: &str| -> f64 {
            let vector = embedder
                .embed(&[query.to_string()])
                .expect("embed")
                .remove(0);
            benches[at].rank(vector.components(), Scoring::Centred)[0].1
        };
        let mut inside: Vec<f64> = in_domain.iter().map(|q| top1(q)).collect();
        let mut outside: Vec<f64> = out_domain.iter().map(|q| top1(q)).collect();
        inside.sort_by(f64::total_cmp);
        outside.sort_by(f64::total_cmp);
        // How many of each class sit on the wrong side of the best available split.
        let mut candidates: Vec<f64> = inside.iter().chain(&outside).copied().collect();
        candidates.sort_by(f64::total_cmp);
        let (missed, wrong) = candidates
            .iter()
            .map(|cut| {
                (
                    inside.iter().filter(|s| *s < cut).count(),
                    outside.iter().filter(|s| *s >= cut).count(),
                )
            })
            .min_by_key(|(missed, wrong)| missed + wrong)
            .expect("a candidate");
        println!(
            "{name:<18} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>26}",
            inside[inside.len() / 2],
            inside[0],
            outside[outside.len() / 2],
            outside[outside.len() - 1],
            format!(
                "{missed} of {} in, {wrong} of {} out",
                inside.len(),
                outside.len()
            )
        );
    }

    // ---- 2. Both labelled corpora. ----
    println!("\n=== 2. LABELLED RETRIEVAL, BOTH CORPORA ===\n");
    println!(
        "{:<34} {:<18} {:>7} {:>7} {:>7} {:>7}",
        "corpus / key", "model", "P@5", "P@10", "MRR", "r@10"
    );
    for (corpus_label, path, label_files) in [
        (
            "seed-quality (45 real)",
            "corpus/seed-quality-corpus.json",
            vec!["self"],
        ),
        (
            "traversal (429 synthetic)",
            "corpus/traversal-corpus.jsonl",
            vec!["labels-a2.json", "labels-b2.json"],
        ),
    ] {
        // Build the corpus texts, however this file stores them.
        let mut ids: Vec<String> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        let mut queries: Vec<(String, Vec<String>)> = Vec::new();
        if path.ends_with(".json") {
            let value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(path).expect("fixture"))
                    .expect("JSON");
            for memory in value["memories"].as_array().expect("memories") {
                ids.push(memory["id"].as_str().expect("id").to_string());
                let keywords: Vec<&str> = memory["keywords"]
                    .as_array()
                    .map(|l| l.iter().filter_map(serde_json::Value::as_str).collect())
                    .unwrap_or_default();
                texts.push(format!(
                    "{} {} {}",
                    memory["content"].as_str().unwrap_or_default(),
                    memory["why"].as_str().unwrap_or_default(),
                    keywords.join(" ")
                ));
            }
            for query in value["queries"].as_array().expect("queries") {
                queries.push((
                    query["q"].as_str().expect("q").to_string(),
                    query["relevant"]
                        .as_array()
                        .expect("relevant")
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(ToString::to_string)
                        .collect(),
                ));
            }
        } else {
            for line in std::fs::read_to_string(path).expect("corpus").lines() {
                let record: serde_json::Value = serde_json::from_str(line).expect("JSON");
                let Some(content) = record["content"].as_str() else {
                    continue;
                };
                ids.push(record["id"].as_str().expect("id").to_string());
                let keywords: Vec<&str> = record["keywords"]
                    .as_array()
                    .map(|l| l.iter().filter_map(serde_json::Value::as_str).collect())
                    .unwrap_or_default();
                texts.push(format!(
                    "{content} {} {}",
                    record["why"].as_str().unwrap_or_default(),
                    keywords.join(" ")
                ));
            }
        }

        for label_file in label_files {
            if label_file != "self" {
                queries.clear();
                let value: serde_json::Value = serde_json::from_str(
                    &std::fs::read_to_string(format!("corpus/{label_file}")).expect("labels"),
                )
                .expect("JSON");
                for query in value["queries"].as_array().expect("queries") {
                    queries.push((
                        query["q"].as_str().expect("q").to_string(),
                        query["relevant"]
                            .as_array()
                            .expect("relevant")
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(ToString::to_string)
                            .collect(),
                    ));
                }
            }
            for (name, embedder) in models {
                // Index by position: these corpora predate the current id format.
                let vectors: Vec<Vec<f32>> = embedder
                    .embed(&texts)
                    .expect("embed")
                    .into_iter()
                    .map(|v| v.components().to_vec())
                    .collect();
                let synthetic: Vec<MemoryId> =
                    (0..ids.len()).map(|_| MemoryId::generate()).collect();
                let bench = Bench::build(synthetic.iter().copied().zip(vectors).collect());
                let at_of: HashMap<MemoryId, usize> = synthetic.iter().copied().zip(0..).collect();

                let (mut p5, mut p10, mut mrr, mut r10) = (0.0, 0.0, 0.0, 0.0);
                for (query, relevant) in &queries {
                    let ranked = ranked_ids(&bench, embedder, query);
                    let wanted: Vec<MemoryId> = ranked
                        .iter()
                        .copied()
                        .filter(|id| relevant.contains(&ids[at_of[id]]))
                        .collect();
                    let (a, _, c) = score_ranking(&ranked, &wanted, 5);
                    let (b, d, _) = score_ranking(&ranked, &wanted, 10);
                    p5 += a;
                    p10 += b;
                    mrr += c;
                    r10 += d;
                }
                #[allow(clippy::cast_precision_loss)]
                let n = queries.len() as f64;
                println!(
                    "{:<34} {name:<18} {:>6.1}% {:>6.1}% {:>7.3} {:>6.1}%",
                    if name.starts_with("base") {
                        format!("{corpus_label} / {label_file}")
                    } else {
                        String::new()
                    },
                    p5 / n * 100.0,
                    p10 / n * 100.0,
                    mrr / n,
                    r10 / n * 100.0
                );
            }
        }
    }
}

/// Minimal scratch directory, matching `concurrency.rs` rather than adding a dev-dependency.
fn tempdir() -> TempDir {
    let mut path = std::env::temp_dir();
    path.push(format!("scatterhoard-probe-{}", MemoryId::generate()));
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

struct TempDir(PathBuf);

impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The four candidate compositions of a memory's embedding input.
///
/// `Live` is what [`scatterhoard::embedding::embedding_input`] builds today. The other three are
/// the rest of the 2026-07-31 ablation, re-run at the size the corpus has actually reached.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Composition {
    ContentOnly,
    ContentKeywords,
    ContentWhy,
    Live,
}

impl Composition {
    fn label(self) -> &'static str {
        match self {
            Self::ContentOnly => "content",
            Self::ContentKeywords => "content+keywords",
            Self::ContentWhy => "content+why",
            Self::Live => "content+why+keywords (live)",
        }
    }

    /// Build the input text this composition would embed for a memory.
    fn input(self, content: &str, why: Option<&str>, keywords: &[String]) -> String {
        let mut parts: Vec<&str> = vec![content];
        if matches!(self, Self::ContentWhy | Self::Live)
            && let Some(why) = why
        {
            parts.push(why);
        }
        if matches!(self, Self::ContentKeywords | Self::Live) {
            parts.extend(keywords.iter().map(String::as_str));
        }
        parts.join(" ")
    }
}

const COMPOSITIONS: [Composition; 4] = [
    Composition::ContentOnly,
    Composition::ContentKeywords,
    Composition::ContentWhy,
    Composition::Live,
];

/// Forty queries in three classes, no answer key.
///
/// The class label is all this probe needs: the question is whether an off-corpus query scores like
/// an in-domain one, which is a property of the score distributions and not of which memory is
/// right. Forty because the recorded bar on
/// `an-unanswerable-query-returns-confident-hits` is forty, after three pilots of six, seven and
/// twelve each manufactured separation a proper trial removed.
///
/// `narrow` queries carry an identifier or name one mechanism; they are the class charter matching
/// died on, and the class a session actually asks.
const FORTY: [(&str, &str); 40] = [
    ("narrow", "what is the link traversal mechanism"),
    ("narrow", "when does the code return Cycle"),
    (
        "narrow",
        "which keyword prefix is reserved for control marks",
    ),
    ("narrow", "what does amend refuse after export"),
    (
        "narrow",
        "why does the generated index end marker carry its keyword",
    ),
    (
        "narrow",
        "how is a mutation counted as caught when the tests hang",
    ),
    (
        "narrow",
        "what happens when the embedding model identity does not match the store",
    ),
    ("narrow", "why are identity records never embedded"),
    (
        "narrow",
        "what does hoard_sets restore write before it overwrites",
    ),
    ("narrow", "how is a vector with no direction handled"),
    (
        "narrow",
        "what is the character budget for the convention catalog",
    ),
    (
        "narrow",
        "why is the centroid subtracted at scan time rather than on write",
    ),
    ("narrow", "what does the precompact hook deliberately skip"),
    ("narrow", "how does mutate.py detect a stale mutation"),
    ("narrow", "what makes a checkpoint filename sort wrongly"),
    ("broad", "design decisions and constraints"),
    ("broad", "how the work has gone wrong"),
    ("broad", "measurement"),
    ("broad", "what decides which memory set a memory belongs in"),
    (
        "broad",
        "when should a memory be superseded rather than amended",
    ),
    ("broad", "how should confidence be set"),
    ("broad", "what makes a good keyword"),
    ("broad", "why does the project keep session checkpoints"),
    ("broad", "how are shared sets published"),
    ("broad", "what is worth recording as a memory"),
    (
        "off-corpus",
        "sourdough starter hydration ratio for a rye loaf",
    ),
    ("off-corpus", "how to replace a bicycle inner tube"),
    ("off-corpus", "premier league offside rule interpretation"),
    (
        "off-corpus",
        "best fertiliser for tomato seedlings in clay soil",
    ),
    ("off-corpus", "how long to roast a leg of lamb per kilogram"),
    ("off-corpus", "symptoms of feline hyperthyroidism"),
    ("off-corpus", "tuning a mandolin to open G"),
    ("off-corpus", "visa requirements for travelling to Vietnam"),
    ("off-corpus", "what year did the Bretton Woods system end"),
    ("off-corpus", "how do noise cancelling headphones work"),
    ("off-corpus", "knitting a raglan sleeve top down"),
    ("off-corpus", "why do cicadas emerge on prime number cycles"),
    ("off-corpus", "recommended tyre pressure for a hatchback"),
    ("off-corpus", "how to descale an espresso machine"),
    ("off-corpus", "rules for castling out of check"),
];

/// Everything a composition needs to rebuild one memory's input.
struct Source {
    id: MemoryId,
    scope: String,
    content: String,
    why: Option<String>,
    keywords: Vec<String>,
    opening: String,
}

/// Read every embeddable memory out of the copied stores, keeping the raw fields.
fn sources(copies: &[(Scope, PathBuf)]) -> Vec<Source> {
    let mut out = Vec::new();
    for (scope, path) in copies {
        let store = Store::open_at(scope.clone(), path).expect("open a copied store");
        let db = rusqlite::Connection::open(path).expect("open");
        let ids: Vec<String> = db
            .prepare("SELECT memory_id FROM vectors")
            .expect("query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("read")
            .filter_map(Result::ok)
            .collect();
        for id in ids {
            let Ok(parsed) = MemoryId::parse(&id) else {
                continue;
            };
            let Ok(Some(memory)) = store.get(parsed) else {
                continue;
            };
            out.push(Source {
                id: parsed,
                scope: scope.as_key().replace("shared:", ""),
                content: memory.content.as_data().to_string(),
                why: memory.why.as_ref().map(|w| w.as_data().to_string()),
                keywords: memory
                    .keywords
                    .iter()
                    .map(|k| k.as_str().to_string())
                    .collect(),
                opening: memory
                    .content
                    .as_data()
                    .split_whitespace()
                    .take(11)
                    .collect::<Vec<_>>()
                    .join(" "),
            });
        }
    }
    // Ids ascend with time, so sorting here is what lets the drift report below be chronological.
    out.sort_by_key(|s| s.id.to_string());
    out
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let n = values.len() as f64;
    values.iter().sum::<f64>() / n
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Is `why` carrying its weight in the embedding input, or is it carrying a shared register?
///
/// The composition was fixed by an ablation over 45 memories and 20 queries on the 2026-07-31 spike
/// corpus -- 57.5 / 65.8 / 66.7 / 71.7 at recall@5 -- and has not been varied since, while the
/// corpus grew to roughly 870 and `why` fields grew into multi-paragraph essays about the act of
/// recording. Every mechanism ruled out under
/// `an-unanswerable-query-returns-confident-hits` is a function of the embedding; this varies the
/// embedding's *input*, which is upstream of all of them.
///
/// Three questions, in order of how cheaply they could kill the idea:
///
/// 1. **How much of the embedded text is `why`?** If it is a small share, nothing follows.
/// 2. **Does that share predict hubness?** A hub is a memory that answers everything. If a memory's
///    hub rate tracks how much `why` it carries rather than what it is about, the shared register is
///    in the vectors.
/// 3. **Does dropping `why` separate the classes?** Off-corpus against narrow in-domain, over forty
///    queries -- the comparison every ruled-out mechanism failed.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_why_belongs_in_the_embedding_input() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let sources = sources(&copies);
    println!(
        "\n{} embeddable memories across {} sets.\n",
        sources.len(),
        copies.len()
    );

    // ---- 1. How much of the embedded text is `why`? ----------------------------------------
    println!("== 1. Share of the embedded input that is `why` ==\n");
    let shares: Vec<f64> = sources
        .iter()
        .map(|s| {
            #[allow(clippy::cast_precision_loss)]
            let content = s.content.split_whitespace().count() as f64;
            #[allow(clippy::cast_precision_loss)]
            let why = s
                .why
                .as_ref()
                .map_or(0.0, |w| w.split_whitespace().count() as f64);
            #[allow(clippy::cast_precision_loss)]
            let keywords = s.keywords.len() as f64;
            let total = content + why + keywords;
            if total > 0.0 { why / total } else { 0.0 }
        })
        .collect();
    let with_why = sources.iter().filter(|s| s.why.is_some()).count();
    let mut sorted = shares.clone();
    println!(
        "  memories carrying a `why`: {with_why} of {}",
        sources.len()
    );
    println!(
        "  share of input that is `why`:  mean {:.1}%   median {:.1}%",
        mean(&shares) * 100.0,
        median(&mut sorted) * 100.0
    );
    let over_half = shares.iter().filter(|s| **s > 0.5).count();
    println!("  memories where `why` is more than half the embedded text: {over_half}");

    // Drift: ids ascend with time, so deciles of the sorted-by-id list are chronological.
    println!("\n  chronological drift, by decile of the corpus (oldest first):");
    let decile = sources.len() / 10;
    if decile > 0 {
        for d in 0..10 {
            let slice = &shares[d * decile..((d + 1) * decile).min(shares.len())];
            #[allow(clippy::cast_precision_loss)]
            let words = mean(
                &sources[d * decile..((d + 1) * decile).min(sources.len())]
                    .iter()
                    .map(|s| {
                        s.why
                            .as_ref()
                            .map_or(0.0, |w| w.split_whitespace().count() as f64)
                    })
                    .collect::<Vec<_>>(),
            );
            println!(
                "    decile {:>2}   `why` is {:>5.1}% of input   mean `why` length {:>5.0} words",
                d + 1,
                mean(slice) * 100.0,
                words
            );
        }
    }

    // ---- 2 and 3. Re-embed under each composition. -------------------------------------------
    let mut hub_by_composition: Vec<(Composition, Vec<usize>)> = Vec::new();

    for composition in COMPOSITIONS {
        let inputs: Vec<String> = sources
            .iter()
            .map(|s| composition.input(&s.content, s.why.as_deref(), &s.keywords))
            .collect();
        let vectors = embedder.embed(&inputs).expect("embed the corpus");
        let entries: Vec<(MemoryId, Vec<f32>)> = sources
            .iter()
            .zip(&vectors)
            .map(|(s, v)| (s.id, v.components().to_vec()))
            .collect();
        let bench = Bench::build(entries);

        let mut hits: HashMap<MemoryId, usize> = HashMap::new();
        let mut by_class: HashMap<&str, Vec<f64>> = HashMap::new();
        for (class, query) in FORTY {
            let vector = embedder
                .embed(&[query.to_string()])
                .expect("embed a query")
                .remove(0);
            let ranked = bench.rank(vector.components(), Scoring::Raw);
            by_class.entry(class).or_default().push(ranked[0].1);
            for (id, _) in ranked.iter().take(10) {
                *hits.entry(*id).or_default() += 1;
            }
        }

        println!("\n== {} ==", composition.label());
        let mut narrow = by_class.get("narrow").cloned().unwrap_or_default();
        let mut broad = by_class.get("broad").cloned().unwrap_or_default();
        let mut off = by_class.get("off-corpus").cloned().unwrap_or_default();
        let narrow_min = narrow.iter().copied().fold(f64::INFINITY, f64::min);
        let off_max = off.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        println!(
            "  top-1 score by class:   narrow {:.3}   broad {:.3}   off-corpus {:.3}  (medians)",
            median(&mut narrow),
            median(&mut broad),
            median(&mut off)
        );
        println!(
            "  worst narrow {narrow_min:.3}   best off-corpus {off_max:.3}   margin {:+.3}",
            narrow_min - off_max
        );
        let inversions = FORTY
            .iter()
            .filter(|(class, _)| *class == "narrow")
            .zip(by_class.get("narrow").cloned().unwrap_or_default())
            .filter(|(_, score)| *score < off_max)
            .count();
        println!(
            "  narrow in-domain queries scoring below the best off-corpus query: {inversions} of 15"
        );

        if composition == Composition::Live {
            println!("\n  every query, by top-1 score -- the overlap region is what matters:");
            let mut all: Vec<(&str, &str, f64)> = Vec::new();
            for (class, query) in FORTY {
                let vector = embedder
                    .embed(&[query.to_string()])
                    .expect("embed")
                    .remove(0);
                let ranked = bench.rank(vector.components(), Scoring::Raw);
                all.push((class, query, ranked[0].1));
            }
            all.sort_by(|a, b| b.2.total_cmp(&a.2));
            for (class, query, score) in &all {
                let flag = match *class {
                    "off-corpus" if *score > 0.33 => "  <-- off-corpus scoring high",
                    "narrow" if *score < 0.40 => "  <-- in-domain scoring low",
                    _ => "",
                };
                println!("      {score:.3}  [{class:<10}] {query}{flag}");
            }
            println!();
        }

        let counts: Vec<usize> = sources
            .iter()
            .map(|s| hits.get(&s.id).copied().unwrap_or(0))
            .collect();
        let worst = counts.iter().copied().max().unwrap_or(0);
        println!("  worst hub appears in {worst} of 40 top-10s");
        hub_by_composition.push((composition, counts));

        if composition == Composition::Live || composition == Composition::ContentOnly {
            let mut ranked_hubs: Vec<(usize, &Source)> = sources
                .iter()
                .map(|s| (hits.get(&s.id).copied().unwrap_or(0), s))
                .collect();
            ranked_hubs.sort_by_key(|(count, _)| std::cmp::Reverse(*count));
            println!("  top hubs:");
            for (count, source) in ranked_hubs.iter().take(4) {
                println!(
                    "      {count:>2}/40 [{:<16}] {}",
                    source.scope, source.opening
                );
            }
        }
    }

    // ---- 2, concluded: does `why`-share predict hubbiness? ------------------------------------
    println!("\n== 2. Does a memory's `why` share predict how often it is retrieved? ==\n");
    let mut order: Vec<usize> = (0..sources.len()).collect();
    order.sort_by(|a, b| shares[*a].total_cmp(&shares[*b]));
    for (composition, counts) in &hub_by_composition {
        let quartile = order.len() / 4;
        if quartile == 0 {
            continue;
        }
        let means: Vec<f64> = (0..4)
            .map(|q| {
                #[allow(clippy::cast_precision_loss)]
                let bucket: Vec<f64> = order[q * quartile..((q + 1) * quartile).min(order.len())]
                    .iter()
                    .map(|i| counts[*i] as f64)
                    .collect();
                mean(&bucket)
            })
            .collect();
        println!(
            "  {:<28} mean top-10 appearances by `why`-share quartile: {:.2}  {:.2}  {:.2}  {:.2}",
            composition.label(),
            means[0],
            means[1],
            means[2],
            means[3]
        );
    }
    println!(
        "\n  (quartiles run least `why` to most. A rising series is the shared register showing up.)"
    );
}

/// Off-corpus queries controlled for *form*, in two subclasses of twelve.
///
/// Suggested by the forty-query run: the only two off-corpus queries that scored into the in-domain
/// range were *premier league offside rule interpretation* and *rules for castling out of check* --
/// both questions about how a formal rule system adjudicates a case, which is what almost every
/// memory in the `scatterhoard` set is about. Everything that scored low was about a substance or a
/// physical object.
///
/// If that is the pattern, the two subclasses separate from each other by more than either
/// separates from the corpus, and *off-corpus* stops being one class.
const RULE_SHAPED: [&str; 12] = [
    "premier league offside rule interpretation",
    "rules for castling out of check",
    "when may a substitution be made in a test match",
    "what happens if a player touches a piece and does not move it",
    "how is a tie broken in a parliamentary vote",
    "when does a contract become void rather than voidable",
    "what disqualifies a swimmer in a medley relay",
    "which exceptions apply to the hearsay rule",
    "when is a passing move permitted in go",
    "how are penalties applied for a false start",
    "what makes an amendment out of order under the standing orders",
    "when does a lease automatically renew rather than lapse",
];

const SUBSTANCE_SHAPED: [&str; 12] = [
    "sourdough starter hydration ratio for a rye loaf",
    "best fertiliser for tomato seedlings in clay soil",
    "how long to roast a leg of lamb per kilogram",
    "symptoms of feline hyperthyroidism",
    "recommended tyre pressure for a hatchback",
    "knitting a raglan sleeve top down",
    "how to descale an espresso machine",
    "what causes copper pipes to develop green stains",
    "how deep to plant garlic cloves in autumn",
    "why does cast iron rust after washing",
    "which wood is best for a chopping board",
    "how to remove candle wax from a wool carpet",
];

/// In-domain queries that the corpus can actually answer.
///
/// Answerability, not topic. The forty-query run mislabelled *how is a vector with no direction
/// handled* as in-domain: it is in-domain by subject and unanswerable by this corpus, because the
/// fact lives in a doc comment in `src/embedding.rs` and was never stored as a memory. That is the
/// axis the entry `an-unanswerable-query-returns-confident-hits` is actually about, and topic is a
/// different one.
const ANSWERABLE: [&str; 12] = [
    "what does amend refuse after export",
    "which keyword prefix is reserved for control marks",
    "what decides which memory set a memory belongs in",
    "how is a mutation counted as caught when the tests hang",
    "why does the generated index end marker carry its keyword",
    "what does the precompact hook deliberately skip",
    "when should a memory be superseded rather than amended",
    "why are identity records never embedded",
    "what happens when the embedding model identity does not match the store",
    "what is the character budget for the convention catalog",
    "what makes a checkpoint filename sort wrongly",
    "how should confidence be set",
];

/// Is *off-corpus* one class, or does the form of the question decide the score?
///
/// The standing conclusion under `an-unanswerable-query-returns-confident-hits` is that seven
/// separation mechanisms failed because the confusion is in the embedding. This asks a prior
/// question: whether the confusion is in the *query set*. If rule-shaped off-corpus queries score
/// like in-domain ones and substance-shaped ones do not, then every pilot's verdict depended on how
/// many of each it happened to contain -- which is a mechanism for verdicts that swing between
/// setups while the system underneath does not change.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_the_form_of_a_question_decides_its_score() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let sources = sources(&copies);
    let inputs: Vec<String> = sources
        .iter()
        .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
        .collect();
    let vectors = embedder.embed(&inputs).expect("embed the corpus");
    let bench = Bench::build(
        sources
            .iter()
            .zip(&vectors)
            .map(|(s, v)| (s.id, v.components().to_vec()))
            .collect(),
    );
    println!(
        "\nLive union, {} memories, live composition, raw cosine.\n",
        bench.size()
    );

    let mut summary: Vec<(&str, Vec<f64>)> = Vec::new();
    for (label, set) in [
        ("answerable in-domain", ANSWERABLE.as_slice()),
        ("off-corpus, rule-shaped", RULE_SHAPED.as_slice()),
        ("off-corpus, substance-shaped", SUBSTANCE_SHAPED.as_slice()),
    ] {
        println!("== {label} ==");
        let mut scores = Vec::new();
        for query in set {
            let vector = embedder
                .embed(&[(*query).to_string()])
                .expect("embed")
                .remove(0);
            let ranked = bench.rank(vector.components(), Scoring::Raw);
            scores.push(ranked[0].1);
            println!("    {:.3}  {query}", ranked[0].1);
        }
        let mut sorted = scores.clone();
        println!(
            "  median {:.3}   min {:.3}   max {:.3}\n",
            median(&mut sorted),
            scores.iter().copied().fold(f64::INFINITY, f64::min),
            scores.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        );
        summary.push((label, scores));
    }

    println!("== The comparison that matters ==\n");
    let med = |v: &Vec<f64>| {
        let mut c = v.clone();
        median(&mut c)
    };
    let (answerable, rules, substance) = (&summary[0].1, &summary[1].1, &summary[2].1);
    println!(
        "  answerable in-domain          median {:.3}",
        med(answerable)
    );
    println!("  off-corpus, rule-shaped       median {:.3}", med(rules));
    println!(
        "  off-corpus, substance-shaped  median {:.3}",
        med(substance)
    );
    println!();
    println!(
        "  in-domain  vs  rule-shaped      gap {:+.3}",
        med(answerable) - med(rules)
    );
    println!(
        "  rule-shaped vs substance-shaped gap {:+.3}",
        med(rules) - med(substance)
    );
    println!();
    let overlap = rules
        .iter()
        .filter(|r| **r >= answerable.iter().copied().fold(f64::INFINITY, f64::min))
        .count();
    let clean = substance
        .iter()
        .filter(|s| **s >= answerable.iter().copied().fold(f64::INFINITY, f64::min))
        .count();
    println!("  rule-shaped queries reaching the worst answerable query:      {overlap} of 12");
    println!("  substance-shaped queries reaching the worst answerable query: {clean} of 12");
    println!();
    println!("  If those two counts differ sharply, `off-corpus` is not one class, and a pilot's");
    println!("  verdict was decided by which kind of off-corpus query it happened to contain.");
}

/// Six more answerable queries, each one checked against a memory read this session.
///
/// Kept separate from [`ANSWERABLE`] only so the provenance of the check stays visible: every one
/// of these has a memory that answers it, named in the comment, rather than being presumed
/// answerable because it scored well -- which would make the positive class circular.
const ANSWERABLE_MORE: [&str; 6] = [
    "why was charter matching withdrawn", // 01M214MJZKE38S0VXZBHKDEGZX
    "what shape is the link graph between memories", // 01M01S5FW4EQGAAXNVN99B2P72
    "what happens when the model file changes but the path does not", // 01M1WZ1SJQFB8SWPAJ8D2TVDMZ
    "why does semantic search not replace keyword search", // 01KZ55MF7GE88A7WVRDB5MQ2TY
    "what did lowercasing keywords do to the embedded text", // 01M1WYMXN5E1095FTKWFAEG7E7
    "which memories are filtered out before embedding", // 01M1WY6CETF80V2TMHQ337X23D
];

/// The separation statistics ruled out on 2026-09-08, as one table.
///
/// Each maps a query's ranking to *how answerable this query looks*, higher meaning more
/// answerable. They are re-tested together because the finding under test is not about any one of
/// them -- it is about whether the **verdict comparing them** is stable once the query set's
/// composition is controlled.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Statistic {
    Top1,
    TopRatio,
    LargestGap,
    GapToMedian,
    ZScore,
    Coherence,
    CentroidCos,
    CentredTop1,
}

const STATISTICS: [Statistic; 8] = [
    Statistic::Top1,
    Statistic::TopRatio,
    Statistic::LargestGap,
    Statistic::GapToMedian,
    Statistic::ZScore,
    Statistic::Coherence,
    Statistic::CentroidCos,
    Statistic::CentredTop1,
];

impl Statistic {
    fn label(self) -> &'static str {
        match self {
            Self::Top1 => "absolute floor (top1)",
            Self::TopRatio => "ratio to second hit",
            Self::LargestGap => "largest gap",
            Self::GapToMedian => "gap to median gap",
            Self::ZScore => "per-query z-score",
            Self::Coherence => "result-set coherence",
            Self::CentroidCos => "cos(query, centroid)",
            Self::CentredTop1 => "centred top1",
        }
    }
}

/// Every statistic for one query, computed once so the resampling below is free.
fn statistics_for(bench: &Bench, index: &HashMap<MemoryId, usize>, query: &[f32]) -> Vec<f64> {
    let ranked = bench.rank(query, Scoring::Raw);
    let centred = bench.rank(query, Scoring::Centred);
    let window: Vec<f64> = ranked.iter().take(20).map(|(_, s)| *s).collect();
    let (largest, median_gap, _) = gap_profile(&window);
    let mean_window = mean(&window);
    #[allow(clippy::cast_precision_loss)]
    let variance = window
        .iter()
        .map(|s| (s - mean_window).powi(2))
        .sum::<f64>()
        / window.len() as f64;
    let deviation = variance.sqrt();

    // Coherence: the mean pairwise similarity among the top five retrieved memories.
    let top: Vec<&Vec<f32>> = ranked
        .iter()
        .take(5)
        .filter_map(|(id, _)| index.get(id))
        .map(|i| &bench.raw[*i])
        .collect();
    let mut pairs = Vec::new();
    for a in 0..top.len() {
        for b in (a + 1)..top.len() {
            pairs.push(cosine_of(top[a], top[b]));
        }
    }

    STATISTICS
        .iter()
        .map(|statistic| match statistic {
            Statistic::Top1 => window[0],
            Statistic::TopRatio => {
                if window[1].abs() > f64::EPSILON {
                    window[0] / window[1]
                } else {
                    0.0
                }
            }
            Statistic::LargestGap => largest,
            Statistic::GapToMedian => {
                if median_gap > f64::EPSILON {
                    largest / median_gap
                } else {
                    0.0
                }
            }
            Statistic::ZScore => {
                if deviation > f64::EPSILON {
                    (window[0] - mean_window) / deviation
                } else {
                    0.0
                }
            }
            Statistic::Coherence => mean(&pairs),
            Statistic::CentroidCos => cosine_of(query, &bench.centroid),
            Statistic::CentredTop1 => centred[0].1,
        })
        .collect()
}

/// Area under the ROC curve: the chance a random answerable query outscores a random unanswerable
/// one. 0.5 is no separation, 1.0 is perfect, and it needs no threshold -- which is what makes it
/// the right measure of *would a pilot have adopted this*.
fn auc(positive: &[f64], negative: &[f64]) -> f64 {
    if positive.is_empty() || negative.is_empty() {
        return 0.5;
    }
    let mut total = 0.0;
    for p in positive {
        for n in negative {
            total += if p > n {
                1.0
            } else if (p - n).abs() < f64::EPSILON {
                0.5
            } else {
                0.0
            };
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let pairs = (positive.len() * negative.len()) as f64;
    total / pairs
}

/// A deterministic generator, so a surprising run can be reproduced exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// `count` distinct indices below `len`.
    fn sample(&mut self, len: usize, count: usize) -> Vec<usize> {
        let mut pool: Vec<usize> = (0..len).collect();
        for i in (1..pool.len()).rev() {
            #[allow(clippy::cast_possible_truncation)]
            let j = (self.next() % (i as u64 + 1)) as usize;
            pool.swap(i, j);
        }
        pool.truncate(count.min(len));
        pool
    }
}

fn percentile(values: &mut [f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let index = ((values.len() - 1) as f64 * p).round() as usize;
    values[index]
}

/// Does controlling the off-corpus class stop the verdicts from flipping?
///
/// The prediction, made by the human partner before the cause was known: once whatever drives the
/// sensitivity is found, the numbers will still vary but the *recommendations* will stop reversing.
/// This tests it directly rather than assuming it.
///
/// Eight ruled-out statistics are scored against pilot-sized query sets drawn repeatedly from four
/// differently-composed unanswerable pools. What is reported is not any statistic's score but the
/// **stability of the comparison between them**: the spread of each one's AUC across draws, and how
/// often the draw changes which statistic wins. A pilot's verdict is exactly *which one won*, so a
/// winner that changes from draw to draw is the swing, measured.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_controlling_question_form_stabilises_the_verdict() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let sources = sources(&copies);
    let inputs: Vec<String> = sources
        .iter()
        .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
        .collect();
    let vectors = embedder.embed(&inputs).expect("embed the corpus");
    let bench = Bench::build(
        sources
            .iter()
            .zip(&vectors)
            .map(|(s, v)| (s.id, v.components().to_vec()))
            .collect(),
    );
    let index: HashMap<MemoryId, usize> = bench
        .ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();

    let answerable: Vec<&str> = ANSWERABLE
        .iter()
        .chain(ANSWERABLE_MORE.iter())
        .copied()
        .collect();
    let table = |queries: &[&str]| -> Vec<Vec<f64>> {
        queries
            .iter()
            .map(|q| {
                let v = embedder
                    .embed(&[(*q).to_string()])
                    .expect("embed")
                    .remove(0);
                statistics_for(&bench, &index, v.components())
            })
            .collect()
    };
    let positive = table(&answerable);
    let rules = table(&RULE_SHAPED);
    let substance = table(&SUBSTANCE_SHAPED);

    println!(
        "\n{} memories. {} answerable queries, {} rule-shaped, {} substance-shaped.",
        bench.size(),
        positive.len(),
        rules.len(),
        substance.len()
    );
    println!("Pilot-sized draws: 6 answerable against 6 unanswerable, 400 draws per condition.\n");

    // The whole-pool verdict for each condition: what a fully-powered trial would conclude.
    println!("== Verdict at full power, per condition (AUC; 0.5 separates nothing) ==\n");
    println!(
        "  {:<24} {:>10} {:>12} {:>12}",
        "statistic", "vs mixed", "vs substance", "vs rules"
    );
    let mixed: Vec<Vec<f64>> = rules.iter().chain(substance.iter()).cloned().collect();
    for (s, statistic) in STATISTICS.iter().enumerate() {
        let column = |negative: &Vec<Vec<f64>>| {
            let p: Vec<f64> = positive.iter().map(|row| row[s]).collect();
            let n: Vec<f64> = negative.iter().map(|row| row[s]).collect();
            auc(&p, &n)
        };
        println!(
            "  {:<24} {:>10.3} {:>12.3} {:>12.3}",
            statistic.label(),
            column(&mixed),
            column(&substance),
            column(&rules)
        );
    }

    // The resampling study.
    for (label, negative_pool, balanced) in [
        (
            "UNCONTROLLED (rule and substance mixed, as the pilots drew)",
            &mixed,
            false,
        ),
        ("CONTROLLED: substance-shaped only", &substance, false),
        ("CONTROLLED: rule-shaped only", &rules, false),
        (
            "CONTROLLED: balanced 3 rule + 3 substance every draw",
            &mixed,
            true,
        ),
    ] {
        println!("\n== {label} ==\n");
        let mut rng = Rng(0x5EED_1234_ABCD_0001);
        let mut aucs: Vec<Vec<f64>> = vec![Vec::new(); STATISTICS.len()];
        let mut wins: Vec<usize> = vec![0; STATISTICS.len()];

        for _ in 0..400 {
            let pick_positive = rng.sample(positive.len(), 6);
            let pick_negative: Vec<Vec<f64>> = if balanced {
                let r = rng.sample(rules.len(), 3);
                let s = rng.sample(substance.len(), 3);
                r.iter()
                    .map(|i| rules[*i].clone())
                    .chain(s.iter().map(|i| substance[*i].clone()))
                    .collect()
            } else {
                rng.sample(negative_pool.len(), 6)
                    .iter()
                    .map(|i| negative_pool[*i].clone())
                    .collect()
            };

            let mut best = (0usize, f64::NEG_INFINITY);
            for (s, _) in STATISTICS.iter().enumerate() {
                let p: Vec<f64> = pick_positive.iter().map(|i| positive[*i][s]).collect();
                let n: Vec<f64> = pick_negative.iter().map(|row| row[s]).collect();
                let value = auc(&p, &n);
                aucs[s].push(value);
                if value > best.1 {
                    best = (s, value);
                }
            }
            wins[best.0] += 1;
        }

        println!(
            "  {:<24} {:>7} {:>7} {:>7} {:>9} {:>8}",
            "statistic", "median", "p10", "p90", "spread", "wins"
        );
        for (s, statistic) in STATISTICS.iter().enumerate() {
            let mut values = aucs[s].clone();
            let p10 = percentile(&mut values.clone(), 0.10);
            let p90 = percentile(&mut values.clone(), 0.90);
            let share = f64::from(u32::try_from(wins[s]).unwrap_or(0)) / 4.0;
            println!(
                "  {:<24} {:>7.3} {:>7.3} {:>7.3} {:>9.3} {:>7.1}%",
                statistic.label(),
                percentile(&mut values, 0.50),
                p10,
                p90,
                p90 - p10,
                share
            );
        }
        let distinct = wins.iter().filter(|w| **w > 0).count();
        let top = wins.iter().copied().max().unwrap_or(0);
        #[allow(clippy::cast_precision_loss)]
        let dominance = f64::from(u32::try_from(top).unwrap_or(0)) / 4.0;
        println!(
            "\n  {distinct} of 8 statistics won at least one draw; the most frequent winner took {dominance:.1}% of them."
        );
    }

    println!(
        "\n  A pilot's recommendation is whichever statistic won its draw. Wide `wins` spread"
    );
    println!(
        "  across many statistics is the swing; one statistic taking most draws is a stable verdict."
    );

    // The absolute floor scored far higher here than the record says it can. If a single threshold
    // exists, that is a design proposal rather than another measurement, so it is worth pinning
    // down exactly what one would cost.
    println!("\n== Is there a usable threshold on top1? ==\n");
    let index_top1 = 0;
    let pos: Vec<f64> = positive.iter().map(|r| r[index_top1]).collect();
    let neg_rules: Vec<f64> = rules.iter().map(|r| r[index_top1]).collect();
    let neg_substance: Vec<f64> = substance.iter().map(|r| r[index_top1]).collect();
    let neg_all: Vec<f64> = neg_rules
        .iter()
        .chain(neg_substance.iter())
        .copied()
        .collect();

    let mut candidates: Vec<f64> = pos.iter().chain(neg_all.iter()).copied().collect();
    candidates.sort_by(f64::total_cmp);
    let mut best = (0.0, usize::MAX, 0usize, 0usize);
    for candidate in &candidates {
        let missed = pos.iter().filter(|p| *p < candidate).count();
        let admitted = neg_all.iter().filter(|n| *n >= candidate).count();
        if missed + admitted < best.1 {
            best = (*candidate, missed + admitted, missed, admitted);
        }
    }
    println!(
        "  best single threshold {:.3}: {} errors of {} queries",
        best.0,
        best.1,
        pos.len() + neg_all.len()
    );
    println!(
        "     answerable queries rejected: {} of {}",
        best.2,
        pos.len()
    );
    println!(
        "     unanswerable queries admitted: {} of {}",
        best.3,
        neg_all.len()
    );
    let admitted_rules = neg_rules.iter().filter(|n| **n >= best.0).count();
    let admitted_substance = neg_substance.iter().filter(|n| **n >= best.0).count();
    println!(
        "       of which rule-shaped: {admitted_rules} of {}",
        neg_rules.len()
    );
    println!(
        "       of which substance-shaped: {admitted_substance} of {}",
        neg_substance.len()
    );
    println!();
    println!(
        "  worst answerable {:.3}   best rule-shaped {:.3}   best substance-shaped {:.3}",
        pos.iter().copied().fold(f64::INFINITY, f64::min),
        neg_rules.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        neg_substance
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
    );
    println!();
    println!("  Ignoring rule-shaped questions entirely -- treating them as a case the corpus");
    println!("  genuinely resembles rather than a failure -- the threshold problem is:");
    let mut best_sub = (0.0, usize::MAX);
    for candidate in &candidates {
        let missed = pos.iter().filter(|p| *p < candidate).count();
        let admitted = neg_substance.iter().filter(|n| *n >= candidate).count();
        if missed + admitted < best_sub.1 {
            best_sub = (*candidate, missed + admitted);
        }
    }
    println!(
        "     best threshold {:.3}: {} errors of {}",
        best_sub.0,
        best_sub.1,
        pos.len() + neg_substance.len()
    );
}

/// Thirty deliberately-unanswerable questions, all about substances and physical objects.
///
/// The candidate calibration probe set. Substance-shaped throughout, because
/// `probe_whether_the_form_of_a_question_decides_its_score` found rule-shaped questions land inside
/// the answerable range and so cannot calibrate anything. None touches software, governance or
/// knowledge work.
///
/// **The unsolved problem is that a shipped probe set cannot be off-domain for every store.** These
/// are safely distant from Scatterhoard's own sets; they would be squarely *on* domain for a
/// gardening or cookery set, and a probe that the corpus can answer pushes the floor up and
/// silently suppresses real results. The spread reported below is what says whether a robust
/// statistic survives that.
const CALIBRATION_PROBES: [&str; 30] = [
    "sourdough starter hydration ratio for a rye loaf",
    "best fertiliser for tomato seedlings in clay soil",
    "how long to roast a leg of lamb per kilogram",
    "symptoms of feline hyperthyroidism",
    "recommended tyre pressure for a hatchback",
    "knitting a raglan sleeve top down",
    "how to descale an espresso machine",
    "what causes copper pipes to develop green stains",
    "how deep to plant garlic cloves in autumn",
    "why does cast iron rust after washing",
    "which wood is best for a chopping board",
    "how to remove candle wax from a wool carpet",
    "what temperature kills bread yeast",
    "how to prune a mature apple tree in winter",
    "why do onions make your eyes water",
    "how to season a carbon steel wok",
    "what causes limescale in a kettle",
    "how long does emulsion paint take to cure",
    "why do bananas ripen faster in a paper bag",
    "how to sharpen a chisel on a waterstone",
    "what makes concrete crack as it sets",
    "how to treat aphids on roses without pesticide",
    "why does milk boil over so suddenly",
    "what causes a fridge to frost up at the back",
    "how to darn a hole in a wool sock",
    "why does honey crystallise in the jar",
    "what soil pH do blueberries need",
    "how to remove rust from a bicycle chain",
    "why does bread go stale faster in the fridge",
    "what causes candle tunnelling",
];

/// The highest raw cosine any memory in a corpus gives a query.
fn top1_raw(corpus: &[Vec<f32>], query: &[f32]) -> f64 {
    corpus
        .iter()
        .map(|v| cosine_of(v, query))
        .fold(f64::NEG_INFINITY, f64::max)
}

/// Read a corpus out of a memory-file JSONL, skipping the header line.
fn jsonl_corpus(path: &str) -> Vec<String> {
    let text = std::fs::read_to_string(path).expect("read a corpus file");
    text.lines()
        .skip(1)
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|value| {
            let content = value.get("content")?.as_str()?.to_string();
            let why = value
                .get("why")
                .and_then(|w| w.as_str())
                .map(ToString::to_string);
            let keywords: Vec<String> = value
                .get("keywords")
                .and_then(|k| k.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|k| k.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default();
            Some(Composition::Live.input(&content, why.as_deref(), &keywords))
        })
        .collect()
}

/// Read the seed-quality corpus, which is a single JSON object rather than JSONL.
fn seed_corpus(path: &str) -> Vec<String> {
    let text = std::fs::read_to_string(path).expect("read the seed corpus");
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse the seed corpus");
    value["memories"]
        .as_array()
        .map(|memories| {
            memories
                .iter()
                .filter_map(|m| {
                    let content = m.get("content")?.as_str()?.to_string();
                    let why = m
                        .get("why")
                        .and_then(|w| w.as_str())
                        .map(ToString::to_string);
                    let keywords: Vec<String> = m
                        .get("keywords")
                        .and_then(|k| k.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|k| k.as_str().map(ToString::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(Composition::Live.input(&content, why.as_deref(), &keywords))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// How much does a calibrated non-match floor vary between corpora, and with corpus size?
///
/// Two questions the human partner raised when the per-store calibration route was proposed: how
/// much the calibration value actually moves between stores, and therefore how often it would have
/// to be recomputed. Measured over every live set, their union, and two corpora that are not live
/// sets at all -- the 429-memory traversal corpus built from Scatterhoard's own design documents,
/// which is the same subject in a different register, and the 45-memory seed-quality corpus.
///
/// The floor is reported under two rules, because they behave very differently when a probe turns
/// out to be answerable: **max** over the probe set, and **p90**. The size ladder at the end is what
/// answers the cadence question -- top-1 is a maximum over the corpus, so it should drift upward as
/// a set grows whether or not anything else changes.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_how_much_a_calibration_floor_varies_across_corpora() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    // Every live set separately, then their union, then the two file corpora.
    let mut corpora: Vec<(String, Vec<Vec<f32>>)> = Vec::new();
    let mut union: Vec<Vec<f32>> = Vec::new();
    let mut scatterhoard_only: Vec<Vec<f32>> = Vec::new();
    for (scope, path) in &copies {
        let one = sources(std::slice::from_ref(&(scope.clone(), path.clone())));
        if one.is_empty() {
            continue;
        }
        let inputs: Vec<String> = one
            .iter()
            .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
            .collect();
        let vectors: Vec<Vec<f32>> = embedder
            .embed(&inputs)
            .expect("embed")
            .iter()
            .map(|v| v.components().to_vec())
            .collect();
        union.extend(vectors.iter().cloned());
        if scope.as_key() == "shared:scatterhoard" {
            scatterhoard_only = vectors.clone();
        }
        corpora.push((scope.as_key().replace("shared:", ""), vectors));
    }
    corpora.sort_by_key(|(_, v)| v.len());
    corpora.push(("UNION of live sets".to_string(), union));

    for (name, path) in [
        (
            "traversal corpus (design docs)",
            "corpus/traversal-corpus.jsonl",
        ),
        ("seed-quality corpus", "corpus/seed-quality-corpus.json"),
    ] {
        let inputs = if path.ends_with(".jsonl") {
            jsonl_corpus(path)
        } else {
            seed_corpus(path)
        };
        if inputs.is_empty() {
            println!("  (no corpus at {path})");
            continue;
        }
        let vectors: Vec<Vec<f32>> = embedder
            .embed(&inputs)
            .expect("embed")
            .iter()
            .map(|v| v.components().to_vec())
            .collect();
        corpora.push((name.to_string(), vectors));
    }

    let probe_vectors: Vec<Vec<f32>> = CALIBRATION_PROBES
        .iter()
        .map(|q| {
            embedder
                .embed(&[(*q).to_string()])
                .expect("embed")
                .remove(0)
                .components()
                .to_vec()
        })
        .collect();

    println!(
        "\n== Calibration value per corpus, from {} substance-shaped probes ==\n",
        probe_vectors.len()
    );
    println!(
        "  {:<32} {:>6} {:>8} {:>8} {:>8} {:>9}",
        "corpus", "n", "median", "p90", "max", "max-p90"
    );
    let mut floors: Vec<(String, usize, f64, f64)> = Vec::new();
    for (name, vectors) in &corpora {
        let mut scores: Vec<f64> = probe_vectors.iter().map(|q| top1_raw(vectors, q)).collect();
        let p50 = percentile(&mut scores.clone(), 0.50);
        let p90 = percentile(&mut scores.clone(), 0.90);
        let max = percentile(&mut scores, 1.00);
        println!(
            "  {:<32} {:>6} {:>8.3} {:>8.3} {:>8.3} {:>9.3}",
            name,
            vectors.len(),
            p50,
            p90,
            max,
            max - p90
        );
        floors.push((name.clone(), vectors.len(), p90, max));
    }

    let spread = |pick: fn(&(String, usize, f64, f64)) -> f64| {
        let values: Vec<f64> = floors.iter().map(pick).collect();
        let low = values.iter().copied().fold(f64::INFINITY, f64::min);
        let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (low, high, high - low)
    };
    let (p90_low, p90_high, p90_spread) = spread(|f| f.2);
    let (max_low, max_high, max_spread) = spread(|f| f.3);
    println!("\n  floor by p90 rule: {p90_low:.3} to {p90_high:.3}, spread {p90_spread:.3}");
    println!("  floor by max rule: {max_low:.3} to {max_high:.3}, spread {max_spread:.3}");

    // Headroom, on the one corpus with verified answerable queries.
    let answerable: Vec<&str> = ANSWERABLE
        .iter()
        .chain(ANSWERABLE_MORE.iter())
        .copied()
        .collect();
    let real: Vec<f64> = answerable
        .iter()
        .map(|q| {
            let v = embedder
                .embed(&[(*q).to_string()])
                .expect("embed")
                .remove(0);
            top1_raw(&scatterhoard_only, v.components())
        })
        .collect();
    let worst_real = real.iter().copied().fold(f64::INFINITY, f64::min);
    println!(
        "\n== Headroom, `scatterhoard` set only ({} verified answerable queries) ==\n",
        real.len()
    );
    println!("  worst answerable query scores {worst_real:.3}");
    let this = floors
        .iter()
        .find(|f| f.0 == "scatterhoard")
        .expect("the scatterhoard row");
    println!(
        "  its own probe floor: p90 {:.3} (headroom {:+.3}), max {:.3} (headroom {:+.3})",
        this.2,
        worst_real - this.2,
        this.3,
        worst_real - this.3
    );
    println!(
        "  a floor taken from the widest corpus instead: {max_high:.3} (headroom {:+.3})",
        worst_real - max_high
    );

    // Which probes score highest -- the max rule sits above the worst real query, so at least one
    // of these thirty is colliding with the corpus and needs to be seen rather than summarised.
    println!("\n== The five highest-scoring probes against `scatterhoard` ==\n");
    let mut ranked: Vec<(f64, &str)> = CALIBRATION_PROBES
        .iter()
        .zip(&probe_vectors)
        .map(|(q, v)| (top1_raw(&scatterhoard_only, v), *q))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (score, query) in ranked.iter().take(5) {
        let flag = if *score >= worst_real {
            "  <-- above the worst real query"
        } else {
            ""
        };
        println!("      {score:.3}  {query}{flag}");
    }

    // `stale` is a term of art in this corpus -- stale mutations, a memory gone stale,
    // `split-when-partly-stale`. If one loaded word is what lifts that probe, minimal pairs will
    // show it, and the design consequence is that probes need screening against the corpus's own
    // vocabulary rather than merely being off-topic.
    println!("\n== Minimal pairs: is it one loaded word? ==\n");
    for (a, b) in [
        (
            "why does bread go stale faster in the fridge",
            "why does bread dry out faster in the fridge",
        ),
        (
            "what makes concrete crack as it sets",
            "what makes concrete split as it hardens",
        ),
        (
            "why does milk boil over so suddenly",
            "why does milk froth over so suddenly",
        ),
    ] {
        let va = embedder.embed(&[a.to_string()]).expect("embed").remove(0);
        let vb = embedder.embed(&[b.to_string()]).expect("embed").remove(0);
        let (sa, sb) = (
            top1_raw(&scatterhoard_only, va.components()),
            top1_raw(&scatterhoard_only, vb.components()),
        );
        println!("      {sa:.3}  {a}");
        println!("      {sb:.3}  {b}");
        println!("        difference {:+.3}\n", sb - sa);
    }

    // What one answerable probe does to each rule.
    println!("\n== What one contaminated probe costs ==\n");
    let mut scores: Vec<f64> = probe_vectors
        .iter()
        .map(|q| top1_raw(&scatterhoard_only, q))
        .collect();
    let clean_p90 = percentile(&mut scores.clone(), 0.90);
    let clean_max = percentile(&mut scores.clone(), 1.00);
    scores.push(worst_real); // as if one probe were answerable after all
    let dirty_p90 = percentile(&mut scores.clone(), 0.90);
    let dirty_max = percentile(&mut scores, 1.00);
    println!(
        "  p90 rule: {clean_p90:.3} -> {dirty_p90:.3}  ({:+.3})",
        dirty_p90 - clean_p90
    );
    println!(
        "  max rule: {clean_max:.3} -> {dirty_max:.3}  ({:+.3})",
        dirty_max - clean_max
    );

    // The cadence question: does the floor drift with corpus size alone?
    println!("\n== Size ladder on `scatterhoard`: does the floor move as a set grows? ==\n");
    println!("  {:>6} {:>10} {:>10} {:>10}", "n", "p90", "max", "vs full");
    let mut rng = Rng(0x0CA1_B4A7_0000_0001);
    let full_p90 = this.2;
    for size in [30usize, 60, 120, 240, 480, scatterhoard_only.len()] {
        if size > scatterhoard_only.len() {
            continue;
        }
        let mut p90s = Vec::new();
        let mut maxes = Vec::new();
        for _ in 0..12 {
            let pick = rng.sample(scatterhoard_only.len(), size);
            let subset: Vec<Vec<f32>> =
                pick.iter().map(|i| scatterhoard_only[*i].clone()).collect();
            let mut s: Vec<f64> = probe_vectors.iter().map(|q| top1_raw(&subset, q)).collect();
            p90s.push(percentile(&mut s.clone(), 0.90));
            maxes.push(percentile(&mut s, 1.00));
        }
        let p90 = mean(&p90s);
        println!(
            "  {:>6} {:>10.3} {:>10.3} {:>10.3}",
            size,
            p90,
            mean(&maxes),
            p90 - full_p90
        );
    }
    println!("\n  Top-1 is a maximum over the corpus, so any upward drift here is size alone --");
    println!(
        "  the same probes against more memories. That is what sets the recalibration trigger."
    );
}

/// Six classes of meaningless query, because "gibberish" is not one thing.
///
/// The human partner's reason for asking: a meaningless query is the only probe class that is
/// **domain-free by construction**, so it cannot be on-domain for somebody's gardening store the way
/// [`CALIBRATION_PROBES`] would be. Against that, the record has nonsense scoring 0.358 where a real
/// in-domain question scored 0.304.
///
/// Split by class because the suspected mechanism is the tokenizer rather than meaning. A static
/// model embeds by pooling token vectors; a string that survives tokenization as two junk subwords
/// yields a vector built from almost nothing, which cosine then normalises to full scale and which
/// can point anywhere. `RealWordsNoMeaning` is the control that separates *no meaning* from *no
/// tokens*: those are ordinary words the model knows, assembled into nothing.
const NONSENSE: [(&str, &str); 30] = [
    ("keyboard roll", "qwertyuiop asdfghjkl"),
    ("keyboard roll", "asdfasdf jkljkljkl"),
    ("keyboard roll", "zxcvbnm qwerty uiop"),
    ("keyboard roll", "hjkl hjkl asdf gfds"),
    ("keyboard roll", "poiuytrewq lkjhgfdsa"),
    ("random letters", "xkqvzr mplfdw bgtjhn"),
    ("random letters", "wzjxqk vbnrmt pfdslg"),
    ("random letters", "qgzxwv nfhbkm tprlds"),
    ("random letters", "jvxzqw kbnmfg rtplsd"),
    ("random letters", "mkgzxv qwbnpf rtdjls"),
    ("pseudoword", "flimbex quorvat pendaline"),
    ("pseudoword", "trandolic weffering blosk"),
    ("pseudoword", "gorbulent frastic wendible"),
    ("pseudoword", "plerimant zoggish varnute"),
    ("pseudoword", "krendal fibbous mantorine"),
    (
        "real words, no meaning",
        "purple seventeen radiator whisper",
    ),
    ("real words, no meaning", "cabbage velocity pillow marble"),
    ("real words, no meaning", "trumpet gravel onion lantern"),
    ("real words, no meaning", "sandal quarterly bishop kettle"),
    ("real words, no meaning", "curtain badger orbit vinegar"),
    ("repeated characters", "aaaaaa bbbbbb cccccc"),
    ("repeated characters", "xxxxxxxx yyyyyyyy"),
    ("repeated characters", "ababab cdcdcd efefef"),
    ("repeated characters", "oooooo iiiiii uuuuuu"),
    ("repeated characters", "zzzzzzzzzz wwwwwwwwww"),
    ("digits and symbols", "3847 92736 118 4402"),
    ("digits and symbols", "$$$ ### @@@ %%% &&&"),
    ("digits and symbols", "1.2.3 4/5/6 7-8-9"),
    ("digits and symbols", "0000 1111 2222 3333"),
    ("digits and symbols", ">>> <<< ||| ~~~ ^^^"),
];

/// Are meaningless queries usable as calibration probes, or do they have a hole in them?
///
/// Three things decide it. **Level**: do they score below real queries, or above? **Stability**: a
/// probe class whose scores scatter cannot calibrate anything however low its median. **Mechanism**:
/// if the cause is tokenization rather than meaning, the class that uses real words assembled into
/// nothing will behave quite differently from keyboard mashing, and vector norm and distance to the
/// corpus centroid will say why.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_meaningless_queries_can_calibrate_a_floor() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    let mut by_set: Vec<(String, Vec<Vec<f32>>)> = Vec::new();
    let mut scatterhoard_only: Vec<Vec<f32>> = Vec::new();
    for (scope, path) in &copies {
        let one = sources(std::slice::from_ref(&(scope.clone(), path.clone())));
        if one.is_empty() {
            continue;
        }
        let inputs: Vec<String> = one
            .iter()
            .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
            .collect();
        let vectors: Vec<Vec<f32>> = embedder
            .embed(&inputs)
            .expect("embed")
            .iter()
            .map(|v| v.components().to_vec())
            .collect();
        if scope.as_key() == "shared:scatterhoard" {
            scatterhoard_only = vectors.clone();
        }
        by_set.push((scope.as_key().replace("shared:", ""), vectors));
    }
    by_set.sort_by_key(|(_, v)| v.len());

    // The corpus centroid, to ask whether a nonsense vector simply lands in the middle of everything.
    let dimension = scatterhoard_only[0].len();
    let mut centroid = vec![0.0f32; dimension];
    for vector in &scatterhoard_only {
        for (slot, value) in centroid.iter_mut().zip(vector) {
            *slot += value;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let count = scatterhoard_only.len() as f32;
    for slot in &mut centroid {
        *slot /= count;
    }

    let answerable: Vec<&str> = ANSWERABLE
        .iter()
        .chain(ANSWERABLE_MORE.iter())
        .copied()
        .collect();
    let real: Vec<(f64, f64, f64)> = answerable
        .iter()
        .map(|q| {
            let v = embedder
                .embed(&[(*q).to_string()])
                .expect("embed")
                .remove(0);
            (
                top1_raw(&scatterhoard_only, v.components()),
                norm(v.components()),
                cosine_of(v.components(), &centroid),
            )
        })
        .collect();
    let worst_real = real.iter().map(|r| r.0).fold(f64::INFINITY, f64::min);
    let mean_real_norm = mean(&real.iter().map(|r| r.1).collect::<Vec<_>>());
    let mean_real_centroid = mean(&real.iter().map(|r| r.2).collect::<Vec<_>>());

    println!(
        "\n== Meaningless queries against the `scatterhoard` set ({} memories) ==\n",
        scatterhoard_only.len()
    );
    println!(
        "  For reference: {} verified answerable queries score {:.3} at worst, mean norm {:.2}, mean cos-to-centroid {:.3}",
        real.len(),
        worst_real,
        mean_real_norm,
        mean_real_centroid
    );
    println!("  and the 30 substance-shaped probes have p90 0.275.\n");
    println!(
        "  {:<24} {:>8} {:>8} {:>8} {:>8} {:>7} {:>10}",
        "class", "median", "max", "spread", "> real?", "norm", "cos-centre"
    );

    let mut classes: Vec<&str> = Vec::new();
    for (class, _) in NONSENSE {
        if !classes.contains(&class) {
            classes.push(class);
        }
    }
    let mut all_scores: Vec<f64> = Vec::new();
    for class in &classes {
        let members: Vec<&str> = NONSENSE
            .iter()
            .filter(|(c, _)| c == class)
            .map(|(_, q)| *q)
            .collect();
        let mut scores = Vec::new();
        let mut norms = Vec::new();
        let mut centroids = Vec::new();
        for query in &members {
            let v = embedder
                .embed(&[(*query).to_string()])
                .expect("embed")
                .remove(0);
            scores.push(top1_raw(&scatterhoard_only, v.components()));
            norms.push(norm(v.components()));
            centroids.push(cosine_of(v.components(), &centroid));
        }
        all_scores.extend(scores.iter().copied());
        let low = scores.iter().copied().fold(f64::INFINITY, f64::min);
        let high = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let above = scores.iter().filter(|s| **s >= worst_real).count();
        println!(
            "  {:<24} {:>8.3} {:>8.3} {:>8.3} {:>6}/{} {:>7.2} {:>10.3}",
            class,
            percentile(&mut scores.clone(), 0.50),
            high,
            high - low,
            above,
            members.len(),
            mean(&norms),
            mean(&centroids)
        );
    }

    let worst_class_max = {
        let mut v = all_scores.clone();
        percentile(&mut v, 1.00)
    };
    println!(
        "\n  Highest nonsense score of all 30: {worst_class_max:.3}   worst real query: {worst_real:.3}"
    );
    let inversions = all_scores.iter().filter(|s| **s >= worst_real).count();
    println!("  Nonsense queries scoring at or above the worst real query: {inversions} of 30");

    // Worst offenders, named -- a class median hides the case that would break a floor.
    let mut ranked: Vec<(f64, &str, &str)> = NONSENSE
        .iter()
        .map(|(class, query)| {
            let v = embedder
                .embed(&[(*query).to_string()])
                .expect("embed")
                .remove(0);
            (top1_raw(&scatterhoard_only, v.components()), *class, *query)
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("\n  The five highest:");
    for (score, class, query) in ranked.iter().take(5) {
        let flag = if *score >= worst_real {
            "  <-- above the worst real query"
        } else {
            ""
        };
        println!("      {score:.3}  [{class:<22}] {query}{flag}");
    }

    // Does the class behave the same way on every store, or is it corpus-specific?
    println!(
        "\n== Same probes, every set: is the level a property of the probes or the corpus? ==\n"
    );
    println!(
        "  {:<24} {:>6} {:>10} {:>10} {:>12}",
        "corpus", "n", "median", "p90", "max"
    );
    for (name, vectors) in &by_set {
        let mut scores: Vec<f64> = NONSENSE
            .iter()
            .map(|(_, q)| {
                let v = embedder
                    .embed(&[(*q).to_string()])
                    .expect("embed")
                    .remove(0);
                top1_raw(vectors, v.components())
            })
            .collect();
        println!(
            "  {:<24} {:>6} {:>10.3} {:>10.3} {:>12.3}",
            name,
            vectors.len(),
            percentile(&mut scores.clone(), 0.50),
            percentile(&mut scores.clone(), 0.90),
            percentile(&mut scores, 1.00)
        );
    }
}

/// Meaningless, but shaped like a question.
///
/// The gap this exists to close: word salad made of ordinary words scores 0.105 against
/// `scatterhoard` while a real off-domain *question* scores around 0.275, and the useful threshold
/// is the second number. If the difference is the interrogative form rather than the meaning, then
/// a grammatical question assembled from unrelated common words should land near the second while
/// staying domain-free -- which is what a shipped probe set needs and what
/// [`CALIBRATION_PROBES`] cannot be.
const QUESTION_SHAPED_NONSENSE: [&str; 12] = [
    "why does the cabbage velocity pillow the marble",
    "how long should a trumpet gravel the onion lantern",
    "what causes a sandal to quarterly the bishop kettle",
    "when does a curtain badger orbit the vinegar",
    "how do you purple a seventeen radiator whisper",
    "what makes the gravel onion lantern trumpet",
    "why is a pillow marble slower than a cabbage velocity",
    "how many bishop kettles fit inside a quarterly sandal",
    "what happens when a badger curtains an orbit",
    "which whisper radiator suits a seventeen purple",
    "when should a velocity cabbage be marbled",
    "how is a lantern onion gravelled in trumpet",
];

/// Grammatical, meaningful English, about nothing that exists.
///
/// The other half of the control. These are not word salad -- each is a sentence a person could
/// parse -- but no corpus can answer them. If form is what drives the score, these should land in
/// the same band as a real off-domain question.
const ABSURD_BUT_GRAMMATICAL: [&str; 12] = [
    "how many badgers fit inside a Tuesday",
    "what colour is the smell of Thursday afternoon",
    "how do you apologise to a staircase",
    "why do clouds refuse to sign contracts",
    "when is the best time to teach a river to whistle",
    "how long does a shadow need to cool down",
    "what do mountains eat during the winter",
    "why are Wednesdays heavier than Fridays",
    "how do you fold a conversation into thirds",
    "what temperature does a promise melt at",
    "when should you water a piano",
    "how far can a memory be thrown",
];

/// Does question form, rather than meaning, put a probe in the useful band?
///
/// Runs the two constructed classes against every live set alongside the two already measured, so
/// the comparison is like for like. What would make a shipped probe set viable is a class that is
/// domain-free by construction *and* lands near the substance-shaped level rather than far below it.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_question_form_puts_nonsense_in_the_useful_band() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let mut by_set: Vec<(String, Vec<Vec<f32>>)> = Vec::new();
    let mut scatterhoard_only: Vec<Vec<f32>> = Vec::new();
    for (scope, path) in &copies {
        let one = sources(std::slice::from_ref(&(scope.clone(), path.clone())));
        if one.is_empty() {
            continue;
        }
        let inputs: Vec<String> = one
            .iter()
            .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
            .collect();
        let vectors: Vec<Vec<f32>> = embedder
            .embed(&inputs)
            .expect("embed")
            .iter()
            .map(|v| v.components().to_vec())
            .collect();
        if scope.as_key() == "shared:scatterhoard" {
            scatterhoard_only = vectors.clone();
        }
        by_set.push((scope.as_key().replace("shared:", ""), vectors));
    }
    by_set.sort_by_key(|(_, v)| v.len());

    let word_salad: Vec<&str> = NONSENSE
        .iter()
        .filter(|(c, _)| *c == "real words, no meaning")
        .map(|(_, q)| *q)
        .collect();
    let answerable: Vec<&str> = ANSWERABLE
        .iter()
        .chain(ANSWERABLE_MORE.iter())
        .copied()
        .collect();

    let families: Vec<(&str, Vec<&str>)> = vec![
        ("word salad (real words)", word_salad),
        (
            "question-shaped nonsense",
            QUESTION_SHAPED_NONSENSE.to_vec(),
        ),
        ("absurd but grammatical", ABSURD_BUT_GRAMMATICAL.to_vec()),
        ("substance-shaped (real)", CALIBRATION_PROBES.to_vec()),
        ("ANSWERABLE (reference)", answerable),
    ];

    println!(
        "\n== Against the `scatterhoard` set ({} memories) ==\n",
        scatterhoard_only.len()
    );
    println!(
        "  {:<28} {:>4} {:>9} {:>8} {:>8} {:>8}",
        "family", "n", "median", "p90", "max", "min"
    );
    for (label, queries) in &families {
        let mut scores: Vec<f64> = queries
            .iter()
            .map(|q| {
                let v = embedder
                    .embed(&[(*q).to_string()])
                    .expect("embed")
                    .remove(0);
                top1_raw(&scatterhoard_only, v.components())
            })
            .collect();
        println!(
            "  {:<28} {:>4} {:>9.3} {:>8.3} {:>8.3} {:>8.3}",
            label,
            queries.len(),
            percentile(&mut scores.clone(), 0.50),
            percentile(&mut scores.clone(), 0.90),
            percentile(&mut scores.clone(), 1.00),
            percentile(&mut scores, 0.00)
        );
    }

    println!("\n== The same families on every set: is a class's level portable? ==\n");
    for (label, queries) in &families {
        print!("  {label:<28}");
        for (name, vectors) in &by_set {
            let mut scores: Vec<f64> = queries
                .iter()
                .map(|q| {
                    let v = embedder
                        .embed(&[(*q).to_string()])
                        .expect("embed")
                        .remove(0);
                    top1_raw(vectors, v.components())
                })
                .collect();
            print!(
                "  {}: {:.3}",
                &name[..name.len().min(9)],
                percentile(&mut scores, 0.90)
            );
        }
        println!();
    }
    println!("\n  (p90 per set. A class usable as a shipped probe set has to sit just under the");
    println!("   answerable band on every corpus, not only on the one it was written against.)");
}

/// Does plain lexical overlap with the corpus predict the score, whatever the query means?
///
/// The unifying candidate for everything measured on 2026-09-08. Each explanation offered during
/// the day covered part of the evidence and failed elsewhere: question form was refuted by
/// question-shaped nonsense scoring 0.192; rule-versus-substance holds here but is corpus-specific
/// and cannot say why `>>> <<< ||| ~~~ ^^^` scores 0.478. Lexical overlap covers all of them, and it
/// is what a static bag-of-tokens model actually computes.
///
/// If overlap predicts the score across families that differ wildly in meaning, form and
/// answerability, then the score is a function of vocabulary rather than of meaning -- and no
/// statistic computed from that score can recover answerability, which is exactly what the eight
/// ruled-out mechanisms found the hard way.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_lexical_overlap_predicts_the_score() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let sources = sources(&copies);
    let inputs: Vec<String> = sources
        .iter()
        .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
        .collect();
    let vectors: Vec<Vec<f32>> = embedder
        .embed(&inputs)
        .expect("embed")
        .iter()
        .map(|v| v.components().to_vec())
        .collect();

    // Document frequency for every word in the corpus.
    let mut document_frequency: HashMap<String, usize> = HashMap::new();
    for text in &inputs {
        let mut seen: Vec<String> = text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        seen.sort();
        seen.dedup();
        for word in seen {
            *document_frequency.entry(word).or_default() += 1;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let total = inputs.len() as f64;

    // A query's overlap: the mean, over its words, of how much of the corpus contains that word.
    let overlap = |query: &str| -> f64 {
        let words: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        if words.is_empty() {
            return 0.0;
        }
        let shares: Vec<f64> = words
            .iter()
            .map(|w| {
                #[allow(clippy::cast_precision_loss)]
                let count = document_frequency.get(w).copied().unwrap_or(0) as f64;
                count / total
            })
            .collect();
        mean(&shares)
    };

    let answerable: Vec<&str> = ANSWERABLE
        .iter()
        .chain(ANSWERABLE_MORE.iter())
        .copied()
        .collect();
    let word_salad: Vec<&str> = NONSENSE
        .iter()
        .filter(|(c, _)| *c == "real words, no meaning")
        .map(|(_, q)| *q)
        .collect();
    let symbols: Vec<&str> = NONSENSE
        .iter()
        .filter(|(c, _)| *c == "digits and symbols")
        .map(|(_, q)| *q)
        .collect();
    let families: Vec<(&str, Vec<&str>)> = vec![
        ("answerable", answerable),
        ("rule-shaped off-corpus", RULE_SHAPED.to_vec()),
        ("substance-shaped off-corpus", CALIBRATION_PROBES.to_vec()),
        ("absurd but grammatical", ABSURD_BUT_GRAMMATICAL.to_vec()),
        (
            "question-shaped nonsense",
            QUESTION_SHAPED_NONSENSE.to_vec(),
        ),
        ("word salad", word_salad),
        ("digits and symbols", symbols),
    ];

    println!(
        "\n== Lexical overlap against top-1 score, {} memories ==\n",
        vectors.len()
    );
    println!(
        "  {:<30} {:>6} {:>12} {:>10}",
        "family", "n", "mean overlap", "mean top1"
    );
    let mut points: Vec<(f64, f64, &str, &str)> = Vec::new();
    for (label, queries) in &families {
        let mut overlaps = Vec::new();
        let mut scores = Vec::new();
        for query in queries {
            let v = embedder
                .embed(&[(*query).to_string()])
                .expect("embed")
                .remove(0);
            let score = top1_raw(&vectors, v.components());
            let value = overlap(query);
            overlaps.push(value);
            scores.push(score);
            points.push((value, score, label, query));
        }
        println!(
            "  {:<30} {:>6} {:>12.4} {:>10.3}",
            label,
            queries.len(),
            mean(&overlaps),
            mean(&scores)
        );
    }

    // Pearson across every query, families pooled -- the families differ in meaning, form and
    // answerability, so a correlation that survives pooling is about vocabulary and nothing else.
    let xs: Vec<f64> = points.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = points.iter().map(|p| p.1).collect();
    let (mx, my) = (mean(&xs), mean(&ys));
    let covariance: f64 = xs.iter().zip(&ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let vx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum::<f64>().sqrt();
    let vy: f64 = ys.iter().map(|y| (y - my).powi(2)).sum::<f64>().sqrt();
    let r = if vx > 0.0 && vy > 0.0 {
        covariance / (vx * vy)
    } else {
        0.0
    };
    println!(
        "\n  Pearson r over all {} queries, families pooled: {r:.3}  (r^2 = {:.3})",
        points.len(),
        r * r
    );

    // Spearman, since the relationship need not be linear.
    let rank_of = |values: &[f64]| -> Vec<f64> {
        let mut order: Vec<usize> = (0..values.len()).collect();
        order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
        let mut ranks = vec![0.0; values.len()];
        for (rank, index) in order.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            {
                ranks[*index] = rank as f64;
            }
        }
        ranks
    };
    let (rx, ry) = (rank_of(&xs), rank_of(&ys));
    let (mrx, mry) = (mean(&rx), mean(&ry));
    let rcov: f64 = rx.iter().zip(&ry).map(|(x, y)| (x - mrx) * (y - mry)).sum();
    let rvx: f64 = rx.iter().map(|x| (x - mrx).powi(2)).sum::<f64>().sqrt();
    let rvy: f64 = ry.iter().map(|y| (y - mry).powi(2)).sum::<f64>().sqrt();
    println!(
        "  Spearman rho: {:.3}",
        if rvx > 0.0 && rvy > 0.0 {
            rcov / (rvx * rvy)
        } else {
            0.0
        }
    );

    println!("\n  Highest-overlap unanswerable queries -- the false matches, explained or not:\n");
    let mut unanswerable: Vec<&(f64, f64, &str, &str)> =
        points.iter().filter(|p| p.2 != "answerable").collect();
    unanswerable.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (value, score, label, query) in unanswerable.iter().take(6) {
        println!("      top1 {score:.3}  overlap {value:.4}  [{label:<27}] {query}");
    }
    println!(
        "\n  Lowest-overlap answerable queries -- what the corpus answers but barely matches:\n"
    );
    let mut real: Vec<&(f64, f64, &str, &str)> =
        points.iter().filter(|p| p.2 == "answerable").collect();
    real.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (value, score, _, query) in real.iter().take(3) {
        println!("      top1 {score:.3}  overlap {value:.4}  {query}");
    }
}

/// Bert-style pre-tokenization: lowercase, split on whitespace, and split every punctuation
/// character out as its own token. Matches the `BertPreTokenizer` and `BertNormalizer` named in the
/// model's `tokenizer.json`.
fn pretokenize(text: &str) -> Vec<String> {
    use unicode_normalization::UnicodeNormalization;
    // `BertNormalizer` lowercases *and* strips accents, which it does by decomposing to NFD and
    // dropping the combining marks. Omitting the second half was a real bug in this helper: it made
    // `naive` and `naïve` tokenize differently from the way the model actually sees them, and
    // `probe_recompute_the_forward_pass_independently` is what caught it.
    let folded: String = text
        .to_lowercase()
        .nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for c in folded.chars() {
        if c.is_whitespace() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        } else if c.is_alphanumeric() {
            current.push(c);
        } else {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            out.push(c.to_string());
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Greedy longest-match WordPiece, with the `##` continuing prefix the vocabulary declares.
fn wordpiece(vocab: &std::collections::HashSet<String>, pretoken: &str) -> Vec<String> {
    let chars: Vec<char> = pretoken.chars().collect();
    let mut tokens = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = chars.len();
        let mut found = None;
        while start < end {
            let piece: String = chars[start..end].iter().collect();
            let candidate = if start > 0 {
                format!("##{piece}")
            } else {
                piece
            };
            if vocab.contains(&candidate) {
                found = Some(candidate);
                break;
            }
            end -= 1;
        }
        match found {
            Some(token) => {
                tokens.push(token);
                start = end;
            }
            None => return vec!["[UNK]".to_string()],
        }
    }
    tokens
}

fn tokenize(vocab: &std::collections::HashSet<String>, text: &str) -> Vec<String> {
    pretokenize(text)
        .iter()
        .flat_map(|p| wordpiece(vocab, p))
        .collect()
}

/// Pearson r, and the rank correlation beside it.
fn correlations(xs: &[f64], ys: &[f64]) -> (f64, f64) {
    let (mx, my) = (mean(xs), mean(ys));
    let cov: f64 = xs.iter().zip(ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let vx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum::<f64>().sqrt();
    let vy: f64 = ys.iter().map(|y| (y - my).powi(2)).sum::<f64>().sqrt();
    let pearson = if vx > 0.0 && vy > 0.0 {
        cov / (vx * vy)
    } else {
        0.0
    };

    let rank_of = |values: &[f64]| -> Vec<f64> {
        let mut order: Vec<usize> = (0..values.len()).collect();
        order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
        let mut ranks = vec![0.0; values.len()];
        for (rank, index) in order.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            {
                ranks[*index] = rank as f64;
            }
        }
        ranks
    };
    let (rx, ry) = (rank_of(xs), rank_of(ys));
    let (mrx, mry) = (mean(&rx), mean(&ry));
    let rcov: f64 = rx.iter().zip(&ry).map(|(x, y)| (x - mrx) * (y - mry)).sum();
    let rvx: f64 = rx.iter().map(|x| (x - mrx).powi(2)).sum::<f64>().sqrt();
    let rvy: f64 = ry.iter().map(|y| (y - mry).powi(2)).sum::<f64>().sqrt();
    let spearman = if rvx > 0.0 && rvy > 0.0 {
        rcov / (rvx * rvy)
    } else {
        0.0
    };
    (pearson, spearman)
}

/// The subword test: is the score driven by the corpus-commonest *token*, not by average overlap?
///
/// Word-level overlap was refuted at r-squared 0.100
/// (`probe_whether_lexical_overlap_predicts_the_score`), with two counter-examples that both point
/// the same way: `>>> <<< ||| ~~~ ^^^` has no words at all yet scores 0.478, and *bread go stale*
/// owes 0.110 of its score to one word out of eight. Both say the driver is a **single constituent**
/// rather than the mean — and a mean is exactly what the earlier measure computed, so it would
/// dilute a strong single match across a long query.
///
/// This runs the same correlation at the granularity the model actually uses, WordPiece over a
/// Bert pre-tokenizer, and compares the **max** statistic against the **mean** on identical tokens.
/// If max predicts and mean does not, the refuted theory was refuted by its operationalisation.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_subword_overlap_predicts_the_score() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let Ok(model_dir) = std::env::var("SCATTERHOARD_MODEL") else {
        return;
    };
    let tokenizer_text = std::fs::read_to_string(Path::new(&model_dir).join("tokenizer.json"))
        .expect("read tokenizer.json");
    let tokenizer: serde_json::Value =
        serde_json::from_str(&tokenizer_text).expect("parse tokenizer.json");
    let vocab: std::collections::HashSet<String> = tokenizer["model"]["vocab"]
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    println!("\nVocabulary: {} WordPiece tokens.", vocab.len());

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let sources = sources(&copies);
    let inputs: Vec<String> = sources
        .iter()
        .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
        .collect();
    let vectors: Vec<Vec<f32>> = embedder
        .embed(&inputs)
        .expect("embed")
        .iter()
        .map(|v| v.components().to_vec())
        .collect();

    // Subword document frequency across the corpus.
    let mut document_frequency: HashMap<String, usize> = HashMap::new();
    for text in &inputs {
        let mut seen = tokenize(&vocab, text);
        seen.sort();
        seen.dedup();
        for token in seen {
            *document_frequency.entry(token).or_default() += 1;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let total = inputs.len() as f64;
    let share = |token: &str| -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let count = document_frequency.get(token).copied().unwrap_or(0) as f64;
        count / total
    };

    let answerable: Vec<&str> = ANSWERABLE
        .iter()
        .chain(ANSWERABLE_MORE.iter())
        .copied()
        .collect();
    let word_salad: Vec<&str> = NONSENSE
        .iter()
        .filter(|(c, _)| *c == "real words, no meaning")
        .map(|(_, q)| *q)
        .collect();
    let symbols: Vec<&str> = NONSENSE
        .iter()
        .filter(|(c, _)| *c == "digits and symbols")
        .map(|(_, q)| *q)
        .collect();
    let families: Vec<(&str, Vec<&str>)> = vec![
        ("answerable", answerable),
        ("rule-shaped off-corpus", RULE_SHAPED.to_vec()),
        ("substance-shaped off-corpus", CALIBRATION_PROBES.to_vec()),
        ("absurd but grammatical", ABSURD_BUT_GRAMMATICAL.to_vec()),
        (
            "question-shaped nonsense",
            QUESTION_SHAPED_NONSENSE.to_vec(),
        ),
        ("word salad", word_salad),
        ("digits and symbols", symbols),
    ];

    println!("\n== Subword overlap by family ==\n");
    println!(
        "  {:<30} {:>4} {:>9} {:>9} {:>9}",
        "family", "n", "mean tok", "max tok", "top1"
    );
    let mut means = Vec::new();
    let mut maxes = Vec::new();
    let mut scores = Vec::new();
    let mut labelled: Vec<(f64, f64, &str, &str)> = Vec::new();
    for (label, queries) in &families {
        let (mut fm, mut fx, mut fs) = (Vec::new(), Vec::new(), Vec::new());
        for query in queries {
            let tokens = tokenize(&vocab, query);
            let shares: Vec<f64> = tokens.iter().map(|t| share(t)).collect();
            let mean_share = mean(&shares);
            let max_share = shares.iter().copied().fold(0.0, f64::max);
            let v = embedder
                .embed(&[(*query).to_string()])
                .expect("embed")
                .remove(0);
            let score = top1_raw(&vectors, v.components());
            means.push(mean_share);
            maxes.push(max_share);
            scores.push(score);
            fm.push(mean_share);
            fx.push(max_share);
            fs.push(score);
            labelled.push((max_share, score, label, query));
        }
        println!(
            "  {:<30} {:>4} {:>9.4} {:>9.4} {:>9.3}",
            label,
            queries.len(),
            mean(&fm),
            mean(&fx),
            mean(&fs)
        );
    }

    let (mean_r, mean_rho) = correlations(&means, &scores);
    let (max_r, max_rho) = correlations(&maxes, &scores);
    println!("\n== The comparison ==\n");
    println!(
        "  mean subword share vs top1:  r {mean_r:.3}  (r^2 {:.3})   rho {mean_rho:.3}",
        mean_r * mean_r
    );
    println!(
        "  max  subword share vs top1:  r {max_r:.3}  (r^2 {:.3})   rho {max_rho:.3}",
        max_r * max_r
    );
    println!("  word-level mean, measured earlier: r 0.317 (r^2 0.100)");

    // The two counter-examples that motivated this, decomposed.
    println!("\n== The two counter-examples, token by token ==\n");
    for query in [
        "why does bread go stale faster in the fridge",
        ">>> <<< ||| ~~~ ^^^",
    ] {
        let tokens = tokenize(&vocab, query);
        let v = embedder
            .embed(&[query.to_string()])
            .expect("embed")
            .remove(0);
        println!(
            "  {query:?}  -> top1 {:.3}",
            top1_raw(&vectors, v.components())
        );
        let mut ranked: Vec<(f64, &String)> = tokens.iter().map(|t| (share(t), t)).collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        let shown: Vec<String> = ranked
            .iter()
            .take(6)
            .map(|(s, t)| format!("{t}={:.0}%", s * 100.0))
            .collect();
        println!("      commonest tokens: {}\n", shown.join("  "));
    }
}

/// How far off are the calibration numbers, measured the way Scatterhoard actually scores?
///
/// **The probes above score with raw cosine and no centroid.** `Store::recall` does neither: it
/// builds a `combined_centroid` over every set being searched and calls `nearest_centred_on`, then
/// truncates with `traversal::cut_at_largest_gap`. So every floor reported by
/// `probe_how_much_a_calibration_floor_varies_across_corpora` describes a configuration that does
/// not ship, and the design proposal that would rest on those numbers has to be rebuilt on these.
///
/// Three scorings, same probes, same corpora: **raw** (what was measured), **centred on the set's
/// own centroid** (a single-set recall), and **centred on the combined centroid** (a multi-set
/// recall, which is the ordinary case for a session holding four sets).
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_the_calibration_numbers_under_the_shipping_scorer() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    let mut sets: Vec<(String, Vec<Vec<f32>>)> = Vec::new();
    for (scope, path) in &copies {
        let one = sources(std::slice::from_ref(&(scope.clone(), path.clone())));
        if one.is_empty() {
            continue;
        }
        let inputs: Vec<String> = one
            .iter()
            .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
            .collect();
        let vectors: Vec<Vec<f32>> = embedder
            .embed(&inputs)
            .expect("embed")
            .iter()
            .map(|v| v.components().to_vec())
            .collect();
        sets.push((scope.as_key().replace("shared:", ""), vectors));
    }
    sets.sort_by_key(|(_, v)| v.len());

    let centroid_of = |vectors: &[Vec<f32>]| -> Vec<f32> {
        let dimension = vectors[0].len();
        let mut c = vec![0.0f32; dimension];
        for v in vectors {
            for (slot, value) in c.iter_mut().zip(v) {
                *slot += value;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let n = vectors.len() as f32;
        for slot in &mut c {
            *slot /= n;
        }
        c
    };
    let all: Vec<Vec<f32>> = sets.iter().flat_map(|(_, v)| v.iter().cloned()).collect();
    let combined = centroid_of(&all);

    let top1_centred = |corpus: &[Vec<f32>], query: &[f32], centroid: &[f32]| -> f64 {
        let cq: Vec<f32> = query.iter().zip(centroid).map(|(q, c)| q - c).collect();
        corpus
            .iter()
            .map(|v| {
                let cv: Vec<f32> = v.iter().zip(centroid).map(|(x, c)| x - c).collect();
                cosine_of(&cv, &cq)
            })
            .fold(f64::NEG_INFINITY, f64::max)
    };

    let probes: Vec<Vec<f32>> = CALIBRATION_PROBES
        .iter()
        .map(|q| {
            embedder
                .embed(&[(*q).to_string()])
                .expect("embed")
                .remove(0)
                .components()
                .to_vec()
        })
        .collect();
    let answerable: Vec<&str> = ANSWERABLE
        .iter()
        .chain(ANSWERABLE_MORE.iter())
        .copied()
        .collect();
    let real: Vec<Vec<f32>> = answerable
        .iter()
        .map(|q| {
            embedder
                .embed(&[(*q).to_string()])
                .expect("embed")
                .remove(0)
                .components()
                .to_vec()
        })
        .collect();

    println!("\n== Probe floor (p90) under each scoring ==\n");
    println!(
        "  {:<24} {:>6} {:>10} {:>14} {:>16}",
        "corpus", "n", "raw", "centred (own)", "centred (combined)"
    );
    for (name, vectors) in &sets {
        let own = centroid_of(vectors);
        let mut r: Vec<f64> = probes.iter().map(|q| top1_raw(vectors, q)).collect();
        let mut o: Vec<f64> = probes
            .iter()
            .map(|q| top1_centred(vectors, q, &own))
            .collect();
        let mut c: Vec<f64> = probes
            .iter()
            .map(|q| top1_centred(vectors, q, &combined))
            .collect();
        println!(
            "  {:<24} {:>6} {:>10.3} {:>14.3} {:>16.3}",
            name,
            vectors.len(),
            percentile(&mut r, 0.90),
            percentile(&mut o, 0.90),
            percentile(&mut c, 0.90)
        );
    }

    println!("\n== Headroom on `scatterhoard`: worst answerable minus probe floor ==\n");
    let (_, sh) = sets
        .iter()
        .find(|(n, _)| n == "scatterhoard")
        .expect("scatterhoard");
    let own = centroid_of(sh);
    for (label, centroid) in [
        ("raw", None),
        ("centred (own)", Some(&own)),
        ("centred (combined)", Some(&combined)),
    ] {
        let score = |v: &Vec<f32>| match centroid {
            None => top1_raw(sh, v),
            Some(c) => top1_centred(sh, v, c),
        };
        let mut probe_scores: Vec<f64> = probes.iter().map(score).collect();
        let real_scores: Vec<f64> = real.iter().map(score).collect();
        let floor = percentile(&mut probe_scores.clone(), 0.90);
        let worst = real_scores.iter().copied().fold(f64::INFINITY, f64::min);
        let max_probe = percentile(&mut probe_scores, 1.00);
        println!(
            "  {:<22} floor(p90) {floor:.3}   worst answerable {worst:.3}   headroom {:+.3}   worst probe {max_probe:.3}",
            label,
            worst - floor
        );
    }

    println!("\n== Separation on `scatterhoard`: AUC, 18 answerable vs 30 probes ==\n");
    for (label, centroid) in [
        ("raw", None),
        ("centred (own)", Some(&own)),
        ("centred (combined)", Some(&combined)),
    ] {
        let score = |v: &Vec<f32>| match centroid {
            None => top1_raw(sh, v),
            Some(c) => top1_centred(sh, v, c),
        };
        let p: Vec<f64> = real.iter().map(score).collect();
        let n: Vec<f64> = probes.iter().map(score).collect();
        println!("  {:<22} AUC {:.3}", label, auc(&p, &n));
    }
}

/// Is the corpus being silently truncated before it is embedded?
///
/// `StaticEmbedder::embed` calls `StaticModel::encode`, whose documented default is
/// `max_length = 512` tokens. model2vec-rs also pre-truncates at the character level to
/// `max_tokens * median_token_length` before tokenizing, then truncates the token ids again. A
/// memory input is content, then `why`, then keywords — so whatever is cut is cut off the **end**,
/// which is where `why` and the keywords live.
///
/// This matters beyond the obvious. If long memories are already losing their `why` to truncation,
/// then `probe_whether_why_belongs_in_the_embedding_input` was comparing compositions that are
/// partly identical after truncation, and its null result would be an artifact rather than a finding.
///
/// Proved by construction rather than by reading the crate: embed a text, then embed the same text
/// with distinctive material appended, and see whether the vector moves at all.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_long_memories_are_truncated_before_embedding() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let Ok(model_dir) = std::env::var("SCATTERHOARD_MODEL") else {
        return;
    };
    let tokenizer_text = std::fs::read_to_string(Path::new(&model_dir).join("tokenizer.json"))
        .expect("read tokenizer.json");
    let tokenizer: serde_json::Value =
        serde_json::from_str(&tokenizer_text).expect("parse tokenizer.json");
    let vocab: std::collections::HashSet<String> = tokenizer["model"]["vocab"]
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();

    // 1. Does appending text to a long input change its vector at all?
    println!("\n== Does appended text reach the embedding? ==\n");
    let filler = "convention ".repeat(600);
    for (label, base) in [
        (
            "short base (~10 tokens)",
            "the memory set records a decision".to_string(),
        ),
        ("long base (~600 tokens)", filler.clone()),
    ] {
        let appended = format!("{base} zzyzx pangolin quixotic");
        let a = embedder
            .embed(std::slice::from_ref(&base))
            .expect("embed")
            .remove(0);
        let b = embedder.embed(&[appended]).expect("embed").remove(0);
        let similarity = cosine_of(a.components(), b.components());
        let identical = (similarity - 1.0).abs() < 1e-9;
        println!(
            "  {label:<26} cos(base, base+suffix) = {similarity:.9}{}",
            if identical {
                "   <-- suffix never reached the model"
            } else {
                ""
            }
        );
    }

    // 2. How much of the live corpus is over the limit?
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let sources = sources(&copies);
    println!("\n== Corpus token lengths against the 512-token default ==\n");
    let mut lengths: Vec<usize> = Vec::new();
    let mut why_lost = 0usize;
    let mut over = 0usize;
    for source in &sources {
        let whole =
            Composition::Live.input(&source.content, source.why.as_deref(), &source.keywords);
        let count = tokenize(&vocab, &whole).len();
        lengths.push(count);
        if count > 512 {
            over += 1;
            // Does the cut land before `why` even begins?
            let content_tokens = tokenize(&vocab, &source.content).len();
            if content_tokens >= 512 {
                why_lost += 1;
            }
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let as_f64: Vec<f64> = lengths.iter().map(|n| *n as f64).collect();
    let mut sorted = as_f64.clone();
    println!(
        "  {} memories: median {:.0} tokens, p90 {:.0}, max {:.0}",
        lengths.len(),
        percentile(&mut sorted.clone(), 0.50),
        percentile(&mut sorted.clone(), 0.90),
        percentile(&mut sorted, 1.00)
    );
    #[allow(clippy::cast_precision_loss)]
    let share = 100.0 * over as f64 / lengths.len() as f64;
    println!(
        "  over 512 tokens: {over} of {} ({share:.1}%)",
        lengths.len()
    );
    println!("  of those, content alone already fills 512 so `why` is entirely lost: {why_lost}");

    // 3. Does truncation actually flatten the composition ablation?
    println!("\n== The compositions, on long memories only ==\n");
    let long: Vec<&Source> = sources
        .iter()
        .filter(|s| {
            tokenize(
                &vocab,
                &Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords),
            )
            .len()
                > 512
        })
        .collect();
    if long.is_empty() {
        println!("  none over the limit -- nothing to compare.");
        return;
    }
    let mut identical = 0usize;
    let mut moved = Vec::new();
    for source in &long {
        let with =
            Composition::Live.input(&source.content, source.why.as_deref(), &source.keywords);
        let without = Composition::ContentOnly.input(
            &source.content,
            source.why.as_deref(),
            &source.keywords,
        );
        let a = embedder.embed(&[with]).expect("embed").remove(0);
        let b = embedder.embed(&[without]).expect("embed").remove(0);
        let similarity = cosine_of(a.components(), b.components());
        if (similarity - 1.0).abs() < 1e-9 {
            identical += 1;
        }
        moved.push(similarity);
    }
    println!("  {} memories over the limit.", long.len());
    println!(
        "  content+why+keywords identical to content alone: {identical} of {}",
        long.len()
    );
    println!(
        "  mean cosine between the two compositions: {:.6}",
        mean(&moved)
    );
    println!(
        "\n  If those are identical, dropping `why` changed nothing for these memories because"
    );
    println!("  truncation had already removed it -- and the composition ablation's null result");
    println!("  was measuring truncation rather than composition.");
}

/// Independently recompute the model's forward pass and compare it against `StaticEmbedder`.
///
/// **Every embedder check on record is an internal-consistency check** — determinism, stored vectors
/// re-embedding at cosine 1.000000, self-retrieval, word pairs separating. Each verifies that the
/// Rust implementation agrees with itself, and none would notice if `model2vec-rs` tokenised,
/// pooled or normalised differently from the reference. That is the same shape of blind spot as the
/// raw-versus-centred error: internally consistent, externally unchecked.
///
/// This recomputes the whole pass from the raw files — safetensors weights read by byte offset,
/// WordPiece run over a Bert pre-tokenizer, `[UNK]` dropped, mean pooled, L2 normalised — and
/// compares vectors component by component. It is an independent implementation rather than *the*
/// reference: agreement rules out a large class of bugs but not a misconception shared with the
/// crate, which is what a comparison against Python `model2vec` would settle.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_recompute_the_forward_pass_independently() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let Ok(model_dir) = std::env::var("SCATTERHOARD_MODEL") else {
        return;
    };
    let dir = Path::new(&model_dir);

    // token -> id, and the id `[UNK]` occupies, which the crate discards rather than embeds.
    let tokenizer: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("tokenizer.json")).expect("read"))
            .expect("parse");
    let vocab_map = tokenizer["model"]["vocab"]
        .as_object()
        .expect("a vocab object");
    let ids: HashMap<String, usize> = vocab_map
        .iter()
        .filter_map(|(k, v)| {
            v.as_u64()
                .map(|n| (k.clone(), usize::try_from(n).unwrap_or(0)))
        })
        .collect();
    let vocab: std::collections::HashSet<String> = ids.keys().cloned().collect();
    let unk = ids.get("[UNK]").copied();

    // safetensors: 8-byte little-endian header length, then that many bytes of JSON, then data.
    let bytes = std::fs::read(dir.join("model.safetensors")).expect("read weights");
    let header_len = usize::try_from(u64::from_le_bytes(bytes[0..8].try_into().expect("8 bytes")))
        .expect("header length fits");
    let header: serde_json::Value =
        serde_json::from_slice(&bytes[8..8 + header_len]).expect("parse the safetensors header");
    let entry = &header["embeddings"];
    let shape: Vec<usize> = entry["shape"]
        .as_array()
        .expect("shape")
        .iter()
        .map(|n| usize::try_from(n.as_u64().unwrap_or(0)).unwrap_or(0))
        .collect();
    let (rows, cols) = (shape[0], shape[1]);
    let start = 8
        + header_len
        + usize::try_from(entry["data_offsets"][0].as_u64().unwrap_or(0)).unwrap_or(0);
    println!(
        "\nWeights: {rows} x {cols}, dtype {}, data at byte {start}.",
        entry["dtype"].as_str().unwrap_or("?")
    );
    let row_at = |index: usize| -> Vec<f32> {
        let base = start + index * cols * 4;
        (0..cols)
            .map(|c| {
                let o = base + c * 4;
                f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
            })
            .collect()
    };

    // The forward pass, rebuilt: tokenize, drop [UNK], mean pool, L2 normalise.
    let recompute = |text: &str| -> Vec<f32> {
        let mut token_ids: Vec<usize> = tokenize(&vocab, text)
            .iter()
            .filter_map(|t| ids.get(t).copied())
            .collect();
        if let Some(unk) = unk {
            token_ids.retain(|id| *id != unk);
        }
        token_ids.truncate(512);
        let mut sum = vec![0.0f32; cols];
        for id in &token_ids {
            if *id >= rows {
                continue;
            }
            for (slot, value) in sum.iter_mut().zip(row_at(*id)) {
                *slot += value;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let denom = token_ids.len().max(1) as f32;
        for slot in &mut sum {
            *slot /= denom;
        }
        let norm = sum.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
        for slot in &mut sum {
            *slot /= norm;
        }
        sum
    };

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let sources = sources(&copies);
    let real_memory = Composition::Live.input(
        &sources[0].content,
        sources[0].why.as_deref(),
        &sources[0].keywords,
    );

    let cases: Vec<(&str, String)> = vec![
        (
            "plain english",
            "what decides which memory set a memory belongs in".to_string(),
        ),
        ("single word", "stale".to_string()),
        (
            "mixed case",
            "Why Does The Generated Index End Marker Carry Its Keyword".to_string(),
        ),
        ("punctuation only", ">>> <<< ||| ~~~ ^^^".to_string()),
        ("digits", "3847 92736 118 4402".to_string()),
        (
            "identifier-ish",
            "hoard_recall Store::open_at hoard-proj-issue-entry".to_string(),
        ),
        ("nonsense letters", "xkqvzr mplfdw bgtjhn".to_string()),
        ("unicode", "naïve café — résumé façade".to_string()),
        ("a real memory input", real_memory),
        ("over the token limit", "convention ".repeat(600)),
    ];

    println!("\n== Independent recomputation vs StaticEmbedder ==\n");
    println!(
        "  {:<24} {:>14} {:>14} {:>10}",
        "case", "cosine", "max |diff|", "verdict"
    );
    let mut worst = 0.0f64;
    let mut lowest = 1.0f64;
    for (label, text) in &cases {
        let mine = recompute(text);
        let theirs = embedder
            .embed(std::slice::from_ref(text))
            .expect("embed")
            .remove(0);
        let similarity = cosine_of(&mine, theirs.components());
        let max_difference = mine
            .iter()
            .zip(theirs.components())
            .map(|(a, b)| f64::from((a - b).abs()))
            .fold(0.0, f64::max);
        worst = worst.max(max_difference);
        lowest = lowest.min(similarity);
        let verdict = if similarity > 0.999_999 {
            "match"
        } else {
            "DIVERGES"
        };
        println!("  {label:<24} {similarity:>14.9} {max_difference:>14.3e} {verdict:>10}");
    }
    println!("\n  worst component difference across all cases: {worst:.3e}");
    println!("  lowest cosine across all cases: {lowest:.9}");

    // The whole corpus, not just hand-picked cases.
    println!("\n== The same check across every memory in the corpus ==\n");
    let inputs: Vec<String> = sources
        .iter()
        .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
        .collect();
    let theirs = embedder.embed(&inputs).expect("embed the corpus");
    let mut similarities = Vec::new();
    let mut diverged = 0usize;
    for (text, vector) in inputs.iter().zip(&theirs) {
        let similarity = cosine_of(&recompute(text), vector.components());
        if similarity <= 0.999_999 {
            diverged += 1;
        }
        similarities.push(similarity);
    }
    let mut sorted = similarities.clone();
    println!(
        "  {} memories: min cosine {:.9}, median {:.9}",
        similarities.len(),
        percentile(&mut sorted.clone(), 0.00),
        percentile(&mut sorted, 0.50)
    );
    println!("  memories where the two implementations disagree past 1e-6: {diverged}");
}

/// What does each of the scorer's decisions actually contribute?
///
/// **Runs the shipping path, not a replica.** `recall_across_with` is `recall_across` with the
/// decisions selectable, and `Stages::default()` is pinned equal to the shipping composition by
/// `the_instrumented_scorer_at_its_defaults_matches_the_shipping_one`. This exists because a day of
/// measurements was taken with raw cosine against a scorer that centres, and nothing said so.
///
/// The measure is the one the human partner named as the point of the feature: **how much a query
/// the corpus cannot answer costs**. A recall that returns six confident hits for a question about
/// sourdough spends the session's cycles and context on nothing, so `returned` for the unanswerable
/// classes is the number to read, and `returned` for the answerable class is what it must not break.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_the_effect_of_each_scoring_decision() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let mut opened: Vec<Store> = copies
        .iter()
        .map(|(scope, path)| {
            let mut store = Store::open_at(scope.clone(), path).expect("open a copied store");
            store.set_embedder(embedder.clone());
            store
        })
        .collect();
    let total: usize = opened.iter().map(|s| s.count().unwrap_or(0)).sum();

    let answerable: Vec<&str> = ANSWERABLE
        .iter()
        .chain(ANSWERABLE_MORE.iter())
        .copied()
        .collect();
    let families: Vec<(&str, Vec<&str>)> = vec![
        ("answerable", answerable),
        ("off-corpus, rule-shaped", RULE_SHAPED.to_vec()),
        ("off-corpus, substance", CALIBRATION_PROBES.to_vec()),
        (
            "word salad",
            NONSENSE
                .iter()
                .filter(|(c, _)| *c == "real words, no meaning")
                .map(|(_, q)| *q)
                .collect(),
        ),
    ];

    let variants: Vec<(&str, Stages)> = vec![
        ("DEFAULT (ships)", Stages::default()),
        (
            "no centring (raw cosine)",
            Stages {
                centring: Centring::None,
                ..Stages::default()
            },
        ),
        (
            "centre per store",
            Stages {
                centring: Centring::PerStore,
                ..Stages::default()
            },
        ),
        (
            "no walk",
            Stages {
                walk: traversal::Walk::none(),
                ..Stages::default()
            },
        ),
        (
            "inherited weight 0.0",
            Stages {
                inherited_weight: 0.0,
                ..Stages::default()
            },
        ),
        (
            "inherited weight 0.5",
            Stages {
                inherited_weight: 0.5,
                ..Stages::default()
            },
        ),
        (
            "no gap cut",
            Stages {
                gap_cut: false,
                ..Stages::default()
            },
        ),
        (
            "raw + no walk + no cut",
            Stages {
                centring: Centring::None,
                walk: traversal::Walk::none(),
                inherited_weight: 0.0,
                gap_cut: false,
                include_superseded: true,
                floor: Floor::None,
            },
        ),
    ];

    println!(
        "\n{total} memories across {} sets, window 10.\n",
        opened.len()
    );
    println!("Mean results returned per query, by family. Lower is better for everything");
    println!("except `answerable`, where results are the point.\n");
    print!("  {:<26}", "variant");
    for (label, _) in &families {
        print!(" {:>23}", label);
    }
    println!("   {:>10}", "AUC top1");

    for (label, stages) in &variants {
        print!("  {label:<26}");
        let mut positives: Vec<f64> = Vec::new();
        let mut negatives: Vec<f64> = Vec::new();
        for (family, queries) in &families {
            let mut returned = Vec::new();
            let mut tops = Vec::new();
            for query in queries {
                let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
                let recall = recall_across_with(&mut refs, &Question::new(query, 10), *stages)
                    .expect("recall");
                #[allow(clippy::cast_precision_loss)]
                returned.push(recall.hits.len() as f64);
                let top = recall.hits.first().map_or(0.0, |h| f64::from(h.similarity));
                tops.push(top);
            }
            if *family == "answerable" {
                positives.extend(tops.iter().copied());
            } else {
                negatives.extend(tops.iter().copied());
            }
            print!(" {:>23.2}", mean(&returned));
        }
        println!("   {:>10.3}", auc(&positives, &negatives));
    }

    // The gap cut is the stage that decides how many results come back, so it deserves its own view.
    println!("\n== What the gap cut removes, by family (window 10) ==\n");
    println!(
        "  {:<26} {:>12} {:>12} {:>10}",
        "family", "with cut", "without", "removed"
    );
    for (family, queries) in &families {
        let mut with = Vec::new();
        let mut without = Vec::new();
        for query in queries {
            for (target, stages) in [
                (&mut with, Stages::default()),
                (
                    &mut without,
                    Stages {
                        gap_cut: false,
                        ..Stages::default()
                    },
                ),
            ] {
                let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
                let recall = recall_across_with(&mut refs, &Question::new(query, 10), stages)
                    .expect("recall");
                #[allow(clippy::cast_precision_loss)]
                target.push(recall.hits.len() as f64);
            }
        }
        println!(
            "  {:<26} {:>12.2} {:>12.2} {:>10.2}",
            family,
            mean(&with),
            mean(&without),
            mean(&without) - mean(&with)
        );
    }
}

/// How much does loading another set move a set's own scores?
///
/// `tests/cross_store.rs` establishes the direction: the union centroid rescores a set when another
/// set is loaded, and raw cosine does not. That is a property test on four memories. This asks what
/// it is worth on the live corpus — whether a session holding four sets really gets different
/// answers from `scatterhoard` than a session holding only `scatterhoard`.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_how_much_loading_another_set_moves_a_sets_own_scores() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let open = |which: &[usize]| -> Vec<Store> {
        which
            .iter()
            .map(|i| {
                let (scope, path) = &copies[*i];
                let mut store = Store::open_at(scope.clone(), path).expect("open");
                store.set_embedder(embedder.clone());
                store
            })
            .collect()
    };
    let scatterhoard_index = copies
        .iter()
        .position(|(scope, _)| scope.as_key() == "shared:scatterhoard")
        .expect("the scatterhoard set");
    let everything: Vec<usize> = (0..copies.len()).collect();

    let answerable: Vec<&str> = ANSWERABLE
        .iter()
        .chain(ANSWERABLE_MORE.iter())
        .copied()
        .collect();

    println!("\n== `scatterhoard` recalled alone, versus beside the other three sets ==\n");
    println!(
        "  {:<22} {:>14} {:>16} {:>18}",
        "centring", "mean |Δscore|", "max |Δscore|", "top hit changed"
    );
    for centring in [Centring::Combined, Centring::None] {
        let stages = Stages {
            centring,
            ..Stages::default()
        };
        let mut deltas: Vec<f64> = Vec::new();
        let mut changed = 0usize;
        for query in &answerable {
            let mut alone = open(&[scatterhoard_index]);
            let mut refs: Vec<&mut Store> = alone.iter_mut().collect();
            let solo =
                recall_across_with(&mut refs, &Question::new(query, 10), stages).expect("recall");

            let mut all = open(&everything);
            let mut refs: Vec<&mut Store> = all.iter_mut().collect();
            let union =
                recall_across_with(&mut refs, &Question::new(query, 10), stages).expect("recall");

            // Compare only the memories that came from `scatterhoard`, matched by id.
            let solo_scores: HashMap<MemoryId, f64> = solo
                .hits
                .iter()
                .map(|h| (h.memory.id, f64::from(h.similarity)))
                .collect();
            for hit in union
                .hits
                .iter()
                .filter(|h| h.scope.as_key() == "shared:scatterhoard")
            {
                if let Some(before) = solo_scores.get(&hit.memory.id) {
                    deltas.push((f64::from(hit.similarity) - before).abs());
                }
            }
            let solo_top = solo.hits.first().map(|h| h.memory.id);
            let union_top = union
                .hits
                .iter()
                .find(|h| h.scope.as_key() == "shared:scatterhoard")
                .map(|h| h.memory.id);
            if solo_top != union_top {
                changed += 1;
            }
        }
        let max = deltas.iter().copied().fold(0.0, f64::max);
        println!(
            "  {:<22} {:>14.5} {:>16.5} {:>15}/{}",
            format!("{centring:?}"),
            mean(&deltas),
            max,
            changed,
            answerable.len()
        );
    }
    println!(
        "\n  A non-zero delta means the same memory, the same query and the same model gave a"
    );
    println!("  different score because an unrelated set happened to be loaded.");
}

/// Raw plus a floor, against centred: the four-way the ablation pointed at.
///
/// Three counts favour raw cosine — it separates better (AUC 0.959 to 0.901), it keeps
/// split-invariance, and it eliminates the union centroid's subset-sensitivity — and one does not:
/// its answerable-to-substance *result count* ratio is 0.94 against centring's 1.46, which is the
/// metric closest to what the feature is for. The ratio is exactly what a floor would fix, so the
/// question is whether raw plus a floor beats centred without one on every axis.
///
/// **The floor is calibrated on a held-out half of the probes.** Fifteen set the floor, the other
/// fifteen are scored against it along with every other family. Calibrating and evaluating on the
/// same probes would guarantee the substance family looked separated, which is the shape of error
/// that has already cost this investigation three pilots.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_raw_plus_a_floor_against_centred() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let mut opened: Vec<Store> = copies
        .iter()
        .map(|(scope, path)| {
            let mut store = Store::open_at(scope.clone(), path).expect("open");
            store.set_embedder(embedder.clone());
            store
        })
        .collect();
    let total: usize = opened.iter().map(|s| s.count().unwrap_or(0)).sum();

    // Held-out split: odd-indexed probes calibrate, even-indexed probes are evaluated.
    let calibrate: Vec<&str> = CALIBRATION_PROBES
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, q)| *q)
        .collect();
    let held_out: Vec<&str> = CALIBRATION_PROBES
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 0)
        .map(|(_, q)| *q)
        .collect();

    let answerable: Vec<&str> = ANSWERABLE
        .iter()
        .chain(ANSWERABLE_MORE.iter())
        .copied()
        .collect();
    let salad: Vec<&str> = NONSENSE
        .iter()
        .filter(|(c, _)| *c == "real words, no meaning")
        .map(|(_, q)| *q)
        .collect();
    let families: Vec<(&str, Vec<&str>)> = vec![
        ("answerable", answerable.clone()),
        ("substance (held out)", held_out),
        ("rule-shaped", RULE_SHAPED.to_vec()),
        ("word salad", salad),
    ];

    // Calibrate each centring separately: the two put scores on different scales, so one number
    // cannot serve both.
    let mut floors: Vec<(Centring, f64)> = Vec::new();
    for centring in [Centring::None, Centring::Combined] {
        let stages = Stages {
            centring,
            ..Stages::default()
        };
        let mut tops: Vec<f64> = Vec::new();
        for query in &calibrate {
            let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
            let recall =
                recall_across_with(&mut refs, &Question::new(query, 10), stages).expect("recall");
            tops.push(recall.hits.first().map_or(0.0, |h| f64::from(h.similarity)));
        }
        floors.push((centring, percentile(&mut tops, 0.90)));
    }
    println!(
        "\n{total} memories, window 10. Floors calibrated on {} held-back probes at p90:",
        calibrate.len()
    );
    for (centring, floor) in &floors {
        println!("    {centring:?}: {floor:.3}");
    }

    let variants: Vec<(String, Stages)> = vec![
        ("centred, no floor (SHIPS)".to_string(), Stages::default()),
        (
            format!("centred + floor {:.3}", floors[1].1),
            Stages {
                floor: Floor::Fixed(floors[1].1),
                ..Stages::default()
            },
        ),
        (
            "raw, no floor".to_string(),
            Stages {
                centring: Centring::None,
                ..Stages::default()
            },
        ),
        (
            format!("raw + floor {:.3}", floors[0].1),
            Stages {
                centring: Centring::None,
                floor: Floor::Fixed(floors[0].1),
                ..Stages::default()
            },
        ),
    ];

    println!("\n== Mean results returned per query ==\n");
    print!("  {:<28}", "variant");
    for (label, _) in &families {
        print!(" {:>21}", label);
    }
    println!();
    for (label, stages) in &variants {
        print!("  {label:<28}");
        for (_, queries) in &families {
            let mut returned = Vec::new();
            for query in queries {
                let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
                let recall = recall_across_with(&mut refs, &Question::new(query, 10), *stages)
                    .expect("recall");
                #[allow(clippy::cast_precision_loss)]
                returned.push(recall.hits.len() as f64);
            }
            print!(" {:>21.2}", mean(&returned));
        }
        println!();
    }

    println!("\n== The two outcomes that matter ==\n");
    println!(
        "  {:<28} {:>26} {:>24}",
        "variant", "unanswerable silenced", "answerable broken"
    );
    for (label, stages) in &variants {
        // Silenced: an unanswerable query that returns nothing at all.
        let mut unanswerable_total = 0usize;
        let mut silenced = 0usize;
        for (family, queries) in &families {
            if *family == "answerable" {
                continue;
            }
            for query in queries {
                let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
                let recall = recall_across_with(&mut refs, &Question::new(query, 10), *stages)
                    .expect("recall");
                unanswerable_total += 1;
                if recall.hits.is_empty() {
                    silenced += 1;
                }
            }
        }
        // Broken: an answerable query that now returns nothing.
        let mut broken = 0usize;
        for query in &answerable {
            let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
            let recall =
                recall_across_with(&mut refs, &Question::new(query, 10), *stages).expect("recall");
            if recall.hits.is_empty() {
                broken += 1;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let share = 100.0 * silenced as f64 / unanswerable_total as f64;
        println!(
            "  {label:<28} {:>18}/{unanswerable_total} ({share:>4.0}%) {:>18}/{}",
            silenced,
            broken,
            answerable.len()
        );
    }
    println!("\n  `silenced` is the feature working: the recall said nothing here answers this.");
    println!("  `broken` is its cost: a question the corpus can answer, answered with silence.");

    // A drop in result count is only harmless if it is the tail going. Centring loses 45% of its
    // answerable results to the floor and raw loses none, so what those results *were* decides
    // whether that is a cost or a saving.
    println!("\n== What the floor takes from an answerable query ==\n");
    println!(
        "  {:<28} {:>16} {:>18} {:>20}",
        "variant", "top hit kept", "2nd hit kept", "mean rank lost from"
    );
    for (centring, floor) in [
        (Centring::Combined, floors[1].1),
        (Centring::None, floors[0].1),
    ] {
        let without = Stages {
            centring,
            ..Stages::default()
        };
        let with = Stages {
            centring,
            floor: Floor::Fixed(floor),
            ..Stages::default()
        };
        let (mut top_kept, mut second_kept) = (0usize, 0usize);
        let mut first_lost: Vec<f64> = Vec::new();
        for query in &answerable {
            let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
            let before =
                recall_across_with(&mut refs, &Question::new(query, 10), without).expect("recall");
            let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
            let after =
                recall_across_with(&mut refs, &Question::new(query, 10), with).expect("recall");
            let kept: HashSet<MemoryId> = after.hits.iter().map(|h| h.memory.id).collect();
            if before
                .hits
                .first()
                .is_some_and(|h| kept.contains(&h.memory.id))
            {
                top_kept += 1;
            }
            if before
                .hits
                .get(1)
                .is_some_and(|h| kept.contains(&h.memory.id))
            {
                second_kept += 1;
            }
            // The best rank the floor removed: 1 would mean it took the top answer.
            if let Some(position) = before
                .hits
                .iter()
                .position(|h| !kept.contains(&h.memory.id))
            {
                #[allow(clippy::cast_precision_loss)]
                first_lost.push((position + 1) as f64);
            }
        }
        println!(
            "  {:<28} {:>13}/{} {:>15}/{} {:>20.1}",
            format!("{centring:?} + floor {floor:.3}"),
            top_kept,
            answerable.len(),
            second_kept,
            answerable.len(),
            mean(&first_lost)
        );
    }
    println!(
        "\n  `mean rank lost from` is the best position the floor removed, averaged. A number"
    );
    println!(
        "  near the result count means only the tail went; near 1 means it took real answers."
    );
}

/// The floor against 142 queries real sessions actually asked.
///
/// **The weak point of `probe_raw_plus_a_floor_against_centred` is that its eighteen answerable
/// queries were written by the same person who set the floor.** A floor that removes nothing from
/// those is barely evidence. These 142 were issued through `hoard_recall` during ordinary work,
/// recovered from 37 session transcripts by `corpus/harvest_queries.py`, and every one predates
/// 2026-09-07 — before any session began studying the non-match problem, so none could have been
/// shaped by the floor or by what it would be tested against. They are blind by construction.
///
/// **No relevance labels, and none are needed for the question being asked.** What matters is
/// whether the floor *silences* a query somebody really put to the corpus. That is a regression
/// test, not a ranking measurement. It does need judgement afterwards, because a real session can
/// certainly have asked something the corpus could not answer — silence would then be correct — so
/// the silenced queries are printed rather than only counted.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_the_floor_against_queries_real_sessions_asked() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let harvested = std::fs::read_to_string("corpus/harvested-queries-pre-floor.json")
        .expect("run corpus/harvest_queries.py --cutoff 2026-09-07 first");
    let parsed: serde_json::Value = serde_json::from_str(&harvested).expect("parse the harvest");
    let real: Vec<String> = parsed["queries"]
        .as_array()
        .expect("queries")
        .iter()
        .filter_map(|row| row["query"].as_str().map(ToString::to_string))
        .collect();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let mut opened: Vec<Store> = copies
        .iter()
        .map(|(scope, path)| {
            let mut store = Store::open_at(scope.clone(), path).expect("open");
            store.set_embedder(embedder.clone());
            store
        })
        .collect();

    // Calibrate exactly as before: p90 over half the substance probes, per centring.
    let calibrate: Vec<&str> = CALIBRATION_PROBES
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, q)| *q)
        .collect();
    let mut floors = Vec::new();
    for centring in [Centring::None, Centring::Combined] {
        let stages = Stages {
            centring,
            ..Stages::default()
        };
        let mut tops: Vec<f64> = Vec::new();
        for query in &calibrate {
            let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
            let recall =
                recall_across_with(&mut refs, &Question::new(query, 10), stages).expect("recall");
            tops.push(recall.hits.first().map_or(0.0, |h| f64::from(h.similarity)));
        }
        floors.push(percentile(&mut tops, 0.90));
    }
    let (raw_floor, centred_floor) = (floors[0], floors[1]);

    println!(
        "\n{} real queries from {} transcripts, all before 2026-09-07.",
        real.len(),
        parsed["sessions_scanned"]
    );
    println!("Floors: raw {raw_floor:.3}, centred {centred_floor:.3}.\n");
    println!(
        "  {:<30} {:>12} {:>16} {:>18}",
        "variant", "silenced", "mean results", "lost their top hit"
    );

    let variants: Vec<(String, Stages)> = vec![
        ("centred, no floor (SHIPS)".to_string(), Stages::default()),
        (
            format!("centred + floor {centred_floor:.3}"),
            Stages {
                floor: Floor::Fixed(centred_floor),
                ..Stages::default()
            },
        ),
        (
            "raw, no floor".to_string(),
            Stages {
                centring: Centring::None,
                ..Stages::default()
            },
        ),
        (
            format!("raw + floor {raw_floor:.3}"),
            Stages {
                centring: Centring::None,
                floor: Floor::Fixed(raw_floor),
                ..Stages::default()
            },
        ),
    ];

    let mut silenced_by_raw: Vec<(f64, &String)> = Vec::new();
    for (label, stages) in &variants {
        let baseline = Stages {
            centring: stages.centring,
            ..Stages::default()
        };
        let mut silenced = 0usize;
        let mut lost_top = 0usize;
        let mut returned = Vec::new();
        for query in &real {
            let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
            let recall =
                recall_across_with(&mut refs, &Question::new(query, 10), *stages).expect("recall");
            #[allow(clippy::cast_precision_loss)]
            returned.push(recall.hits.len() as f64);
            if recall.hits.is_empty() {
                silenced += 1;
                if stages.floor != Floor::None && stages.centring == Centring::None {
                    let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
                    let before = recall_across_with(&mut refs, &Question::new(query, 10), baseline)
                        .expect("recall");
                    silenced_by_raw.push((
                        before.hits.first().map_or(0.0, |h| f64::from(h.similarity)),
                        query,
                    ));
                }
                continue;
            }
            if stages.floor != Floor::None {
                let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
                let before = recall_across_with(&mut refs, &Question::new(query, 10), baseline)
                    .expect("recall");
                let kept: HashSet<MemoryId> = recall.hits.iter().map(|h| h.memory.id).collect();
                if before
                    .hits
                    .first()
                    .is_some_and(|h| !kept.contains(&h.memory.id))
                {
                    lost_top += 1;
                }
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let share = 100.0 * silenced as f64 / real.len() as f64;
        println!(
            "  {label:<30} {:>7}/{} ({share:>4.1}%) {:>16.2} {:>18}",
            silenced,
            real.len(),
            mean(&returned),
            lost_top
        );
    }

    println!("\n== Every real query the raw floor silenced, with the score it would have had ==\n");
    silenced_by_raw.sort_by(|a, b| b.0.total_cmp(&a.0));
    if silenced_by_raw.is_empty() {
        println!("  none.");
    }
    for (score, query) in &silenced_by_raw {
        println!("      would have scored {score:.3}  {query}");
    }
    println!("\n  Judge these by hand: a real session can have asked something the corpus cannot");
    println!("  answer, and silence is the right answer there. A near-floor score is the worry.");
}

/// Unanswerable by provenance: real queries against a store they are not about.
///
/// The 53%-silenced figure rests on thirty substance probes I wrote, which is the same weakness the
/// harvest fixed on the answerable side. There is no harvest of real *off-domain* queries — no
/// session ever issued one against `governance-docs`, and the `coding-lessons`-scoped queries
/// overlap `scatterhoard`'s subject far too much to count as unanswerable.
///
/// **So invert it.** A query a session really asked while working on Scatterhoard is, put to the
/// `governance-docs` store, unanswerable by construction: that store holds eight memories about an
/// association's board and nothing else. No authoring, no judgement call, and the queries are the same
/// 142 already established as blind to the floor.
///
/// **This test is invalid, and is kept for the reason it failed.** `governance-docs` is not an
/// off-domain store for these queries. It holds the governing documents of an association
/// — a priority clause ranking Articles over By-Laws, what percentage a rule requires, who may serve
/// on a committee — and Scatterhoard's own memories are about conventions, what a rule requires, and
/// amendment versus supersession. The two share **rule-shaped structure**, which
/// `probe_whether_the_form_of_a_question_decides_its_score` established is what drives the score
/// where topic does not. So a survivor here is plausibly a genuine match, and the silencing rate
/// measures nothing about the floor.
///
/// *Unanswerable by provenance* was the mistaken step: provenance establishes a different subject,
/// and subject is precisely what these scores do not follow. There is no genuinely off-domain corpus
/// on this machine — `coding-lessons` overlaps by subject, this one by structure — so the floor's
/// benefit cannot presently be measured here in either direction.
///
/// A secondary limit, now moot: eight memories is the cold-start regime, where absolute scores carry
/// little information anyway.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_real_queries_against_a_store_they_are_not_about() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let harvested = std::fs::read_to_string("corpus/harvested-queries-pre-floor.json")
        .expect("run corpus/harvest_queries.py --cutoff 2026-09-07 first");
    let parsed: serde_json::Value = serde_json::from_str(&harvested).expect("parse");
    let real: Vec<String> = parsed["queries"]
        .as_array()
        .expect("queries")
        .iter()
        .filter_map(|row| row["query"].as_str().map(ToString::to_string))
        .collect();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());

    // Each set on its own, so "the store they are not about" is a real store rather than a union.
    let governance = governance_scope();
    for target in [
        governance.as_str(),
        "shared:coding-lessons",
        "shared:scatterhoard",
    ] {
        let (scope, path) = copies
            .iter()
            .find(|(s, _)| s.as_key() == target)
            .expect("a copied store for the target set");
        let mut store = Store::open_at(scope.clone(), path).expect("open");
        store.set_embedder(embedder.clone());
        let held = store.count().unwrap_or(0);

        // Calibrate on this store alone, raw, p90 of half the substance probes.
        let calibrate: Vec<&str> = CALIBRATION_PROBES
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 2 == 1)
            .map(|(_, q)| *q)
            .collect();
        let raw = Stages {
            centring: Centring::None,
            ..Stages::default()
        };
        let mut tops: Vec<f64> = Vec::new();
        for query in &calibrate {
            let recall = recall_across_with(&mut [&mut store], &Question::new(query, 10), raw)
                .expect("recall");
            tops.push(recall.hits.first().map_or(0.0, |h| f64::from(h.similarity)));
        }
        let floor = percentile(&mut tops, 0.90);
        let floored = Stages {
            centring: Centring::None,
            floor: Floor::Fixed(floor),
            ..raw
        };

        let mut without = Vec::new();
        let mut with = Vec::new();
        let mut silenced = 0usize;
        let mut survivors: Vec<(f64, &String)> = Vec::new();
        for query in &real {
            let before = recall_across_with(&mut [&mut store], &Question::new(query, 10), raw)
                .expect("recall");
            let after = recall_across_with(&mut [&mut store], &Question::new(query, 10), floored)
                .expect("recall");
            #[allow(clippy::cast_precision_loss)]
            without.push(before.hits.len() as f64);
            #[allow(clippy::cast_precision_loss)]
            with.push(after.hits.len() as f64);
            if after.hits.is_empty() {
                silenced += 1;
            } else {
                survivors.push((f64::from(after.hits[0].similarity), query));
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let share = 100.0 * silenced as f64 / real.len() as f64;
        let relation = if target == "shared:scatterhoard" {
            "ANSWERABLE (the store these queries are about)"
        } else if target == "shared:coding-lessons" {
            "partly overlapping"
        } else {
            "UNANSWERABLE by provenance"
        };
        println!("\n== {target} — {held} memories — {relation} ==");
        println!(
            "  floor {floor:.3}   results without {:.2}   with {:.2}   silenced {silenced}/{} ({share:.1}%)",
            mean(&without),
            mean(&with),
            real.len()
        );
        if target == governance && !survivors.is_empty() {
            survivors.sort_by(|a, b| b.0.total_cmp(&a.0));
            println!("  survived the floor against a board-governance store:");
            for (score, query) in survivors.iter().take(6) {
                println!("      {score:.3}  {query}");
            }
        }
    }
    println!(
        "\n  Read the three together. Silencing everything on a store the queries have nothing"
    );
    println!(
        "  to do with is only meaningful beside not silencing them on the store they are about."
    );
}

/// The floor against a negative class nobody authored for it.
///
/// Every earlier measurement of the floor's *benefit* used thirty substance-shaped probes I wrote,
/// and `probe_real_queries_against_a_store_they_are_not_about` failed to check them because its own
/// negative class turned out to share rule-shaped structure with the corpus. This is the first
/// genuinely independent negative class: real questions from the human partner's notes on an
/// unrelated volunteer role, harvested by `corpus/harvest_questions.py`, which drops anything
/// carrying a proper noun, digit, address, identifier, software term or sensitive subject.
///
/// **The question that matters is not how many get silenced but where the floor lands.** If a floor
/// calibrated on my authored probes sits well below one calibrated on real off-domain questions,
/// then those probes were unrepresentatively distant and every benefit figure taken against them was
/// inflated — which is what the inversion attempt suspected and could not establish.
///
/// The data is not in the repository: it comes from private notes and only aggregate counts leave
/// this probe. Point `SCATTERHOARD_OFFDOMAIN` at the harvest to run it.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL and SCATTERHOARD_OFFDOMAIN"]
fn probe_the_floor_against_an_independent_negative_class() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let Ok(offdomain) = std::env::var("SCATTERHOARD_OFFDOMAIN") else {
        println!(
            "SCATTERHOARD_OFFDOMAIN unset -- run corpus/harvest_questions.py and point it here."
        );
        return;
    };
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&offdomain).expect("read the harvest"))
            .expect("parse the harvest");
    let offdomain_questions: Vec<String> = parsed["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .filter_map(|q| q.as_str().map(ToString::to_string))
        .collect();

    let harvested = std::fs::read_to_string("corpus/harvested-queries-pre-floor.json")
        .expect("run corpus/harvest_queries.py --cutoff 2026-09-07 first");
    let real: Vec<String> =
        serde_json::from_str::<serde_json::Value>(&harvested).expect("parse")["queries"]
            .as_array()
            .expect("queries")
            .iter()
            .filter_map(|row| row["query"].as_str().map(ToString::to_string))
            .collect();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let (scope, path) = copies
        .iter()
        .find(|(s, _)| s.as_key() == "shared:scatterhoard")
        .expect("the scatterhoard set");
    let mut store = Store::open_at(scope.clone(), path).expect("open");
    store.set_embedder(embedder.clone());
    let held = store.count().unwrap_or(0);

    let raw = Stages {
        centring: Centring::None,
        ..Stages::default()
    };
    let top1 = |store: &mut Store, query: &str| -> f64 {
        recall_across_with(&mut [store], &Question::new(query, 10), raw)
            .expect("recall")
            .hits
            .first()
            .map_or(0.0, |h| f64::from(h.similarity))
    };

    // Two calibrations, each on half its own class, so neither is scored against what set it.
    let authored: Vec<&str> = CALIBRATION_PROBES
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, q)| *q)
        .collect();
    let offdomain_calibrate: Vec<&String> = offdomain_questions
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, q)| q)
        .collect();
    let offdomain_held: Vec<&String> = offdomain_questions
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 0)
        .map(|(_, q)| q)
        .collect();

    let mut authored_scores: Vec<f64> = authored.iter().map(|q| top1(&mut store, q)).collect();
    let mut offdomain_scores: Vec<f64> = offdomain_calibrate
        .iter()
        .map(|q| top1(&mut store, q))
        .collect();
    let authored_floor = percentile(&mut authored_scores.clone(), 0.90);
    let offdomain_floor = percentile(&mut offdomain_scores.clone(), 0.90);

    println!("\n{held} memories, raw cosine, window 10.");
    println!(
        "{} real in-domain queries; {} off-domain questions from an unrelated domain.\n",
        real.len(),
        offdomain_questions.len()
    );
    println!("== Where the two calibrations land ==\n");
    println!(
        "  authored substance probes   median {:.3}   p90 {:.3}",
        percentile(&mut authored_scores, 0.50),
        authored_floor
    );
    println!(
        "  real off-domain questions   median {:.3}   p90 {:.3}",
        percentile(&mut offdomain_scores, 0.50),
        offdomain_floor
    );
    println!(
        "  difference in floor: {:+.3}",
        offdomain_floor - authored_floor
    );

    println!("\n== What each floor does ==\n");
    println!(
        "  {:<34} {:>22} {:>22}",
        "floor", "off-domain silenced", "real queries silenced"
    );
    for (label, floor) in [
        (
            format!("from authored probes  {authored_floor:.3}"),
            authored_floor,
        ),
        (
            format!("from real off-domain  {offdomain_floor:.3}"),
            offdomain_floor,
        ),
    ] {
        let stages = Stages {
            centring: Centring::None,
            floor: Floor::Fixed(floor),
            ..Stages::default()
        };
        let silence = |store: &mut Store, q: &str| {
            recall_across_with(&mut [store], &Question::new(q, 10), stages)
                .expect("recall")
                .hits
                .is_empty()
        };
        let off = offdomain_held
            .iter()
            .filter(|q| silence(&mut store, q))
            .count();
        let on = real.iter().filter(|q| silence(&mut store, q)).count();
        #[allow(clippy::cast_precision_loss)]
        let off_share = 100.0 * off as f64 / offdomain_held.len() as f64;
        #[allow(clippy::cast_precision_loss)]
        let on_share = 100.0 * on as f64 / real.len() as f64;
        println!(
            "  {label:<34} {:>13}/{} ({off_share:>4.1}%) {:>13}/{} ({on_share:>4.1}%)",
            off,
            offdomain_held.len(),
            on,
            real.len()
        );
    }

    let positives: Vec<f64> = real.iter().map(|q| top1(&mut store, q)).collect();
    let negatives: Vec<f64> = offdomain_held.iter().map(|q| top1(&mut store, q)).collect();
    println!(
        "\n  AUC, real in-domain queries against real off-domain questions: {:.3}",
        auc(&positives, &negatives)
    );
    println!(
        "\n  A floor from authored probes sitting below one from real off-domain questions is"
    );
    println!("  the probes having been unrepresentatively distant -- and every benefit figure");
    println!("  measured against them inflated by the difference.");
}

/// What a floor calibrated from labelled queries would have done, walked forward in time.
///
/// `floor-calibration-source` proposes that the store log what recall scored, and calibrate from the
/// queries a session labelled answerable -- by naming a scope, or by later using a memory the recall
/// returned. This replays that: the labelled queries in the order they were asked, a floor set from
/// the first *k* of them, and the cost measured on the ones that came after, which that floor never
/// saw. Benefit is measured against the off-domain probe set.
///
/// `SCATTERHOARD_SET` names the set and `SCATTERHOARD_LABELLED` its labelled queries, which
/// `corpus/label_queries.py` writes to `corpus/labelled-queries-<set>.json` -- the two have to agree,
/// since a query labelled for one set says nothing about another:
///
/// ```text
/// SCATTERHOARD_SET=shared:coding-lessons \
///   SCATTERHOARD_LABELLED=corpus/labelled-queries-coding-lessons.json \
///   SCATTERHOARD_OFFDOMAIN=corpus/offdomain-probes.json \
///   cargo test --test probes probe_simulate_a_floor_calibrated_from_labelled_queries -- --ignored --nocapture
/// ```
///
/// Scored with `gap_cut` off, taking the **best** content similarity in the window, because that is
/// what the floor stage acts on -- `retain(|hit| hit.similarity >= floor)` leaves a query answered if
/// any one hit clears the floor.
///
/// The corpus is held fixed at what the set holds today while the real one grew underneath these
/// queries, so the floors here are, if anything, higher than the ones that would have been set at the
/// time; `probe_how_much_a_calibration_floor_varies_across_corpora` measures that drift separately.
#[test]
#[ignore = "reads this machine's live sets; needs SCATTERHOARD_MODEL, SCATTERHOARD_LABELLED and SCATTERHOARD_OFFDOMAIN"]
fn probe_simulate_a_floor_calibrated_from_labelled_queries() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let (Ok(labelled_path), Ok(offdomain_path)) = (
        std::env::var("SCATTERHOARD_LABELLED"),
        std::env::var("SCATTERHOARD_OFFDOMAIN"),
    ) else {
        println!(
            "SCATTERHOARD_LABELLED and SCATTERHOARD_OFFDOMAIN must both be set -- nothing measured."
        );
        return;
    };

    let labelled_file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&labelled_path).expect("read"))
            .expect("parse");
    let mut labelled: Vec<(String, String, String)> = labelled_file["queries"]
        .as_array()
        .expect("queries")
        .iter()
        .filter_map(|row| {
            Some((
                row["first_seen"].as_str()?.to_owned(),
                row["query"].as_str()?.to_owned(),
                row["label"].as_str().unwrap_or("used").to_owned(),
            ))
        })
        .collect();
    labelled.sort_by(|a, b| a.0.cmp(&b.0));
    let probes: Vec<String> = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(&offdomain_path).expect("read"),
    )
    .expect("parse")["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .filter_map(|q| q.as_str().map(ToString::to_string))
        .collect();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    // Which set to simulate. The labelled queries have to be the ones labelled *for this set*, so
    // the two arguments travel together.
    let wanted =
        std::env::var("SCATTERHOARD_SET").unwrap_or_else(|_| "shared:scatterhoard".to_owned());
    let (scope, path) = copies
        .iter()
        .find(|(s, _)| s.as_key() == wanted)
        .unwrap_or_else(|| panic!("no set {wanted} on this machine"));
    let mut store = Store::open_at(scope.clone(), path).expect("open");
    store.set_embedder(embedder.clone());
    // A copied store carries the model its vectors came from. Under any other model recall would
    // exclude it and every query would score alike, which is a silent zero rather than a result.
    if !matches!(
        store.model_state().expect("model state"),
        scatterhoard::store::ModelState::Current { .. }
    ) {
        let rebuilt = store.rebuild_vectors().expect("rebuild");
        println!(
            "  (rebuilt {} vectors under the loaded model; {} unrepresentable)",
            rebuilt.vectors, rebuilt.unrepresentable
        );
    }

    #[allow(clippy::cast_precision_loss)]
    fn share(count: usize, total: usize) -> f64 {
        100.0 * count as f64 / total as f64
    }

    println!(
        "\n  {} labelled queries against {} memories, {} off-domain probes.",
        labelled.len(),
        store.count().expect("count"),
        probes.len()
    );

    // Both compositions, because the floor acts on `hit.similarity` -- which recall reports with the
    // corpus centroid removed, and centring is what `an-unanswerable-query-returns-confident-hits`
    // records as removing the one direction that marked a query as out of domain. If the separation
    // lives in the raw scores, the floor is being asked of the wrong quantity.
    for (label, stages) in [
        (
            "shipping, centred",
            Stages {
                gap_cut: false,
                ..Stages::default()
            },
        ),
        (
            "raw cosine",
            Stages {
                gap_cut: false,
                centring: Centring::None,
                ..Stages::default()
            },
        ),
    ] {
        let best = |store: &mut Store, query: &str| -> f64 {
            recall_across_with(&mut [store], &Question::new(query, 10), stages)
                .expect("recall")
                .hits
                .iter()
                .map(|hit| f64::from(hit.similarity))
                .fold(f64::NEG_INFINITY, f64::max)
        };
        let labelled_scores: Vec<f64> = labelled
            .iter()
            .map(|(_, q, _)| best(&mut store, q))
            .collect();
        let probe_scores: Vec<f64> = probes.iter().map(|q| best(&mut store, q)).collect();

        let mut sorted = labelled_scores.clone();
        println!("\n== {label} ==");
        println!(
            "  labelled scores: min {:.3}, p05 {:.3}, median {:.3}   |   probes: median {:.3}, p90 {:.3}",
            percentile(&mut sorted.clone(), 0.0),
            percentile(&mut sorted.clone(), 0.05),
            percentile(&mut sorted, 0.50),
            percentile(&mut probe_scores.clone(), 0.50),
            percentile(&mut probe_scores.clone(), 0.90),
        );
        println!(
            "  {:<8} {:>7} {:>8} {:>18} {:>16} {:>20}",
            "from k", "margin", "floor", "later silenced", "of those scoped", "off-domain silenced"
        );
        for k in [10usize, 20, 30, 40, 60] {
            if k + 5 > labelled_scores.len() {
                continue;
            }
            let base = labelled_scores[..k]
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min);
            for margin in [0.0, 0.05] {
                let floor = base - margin;
                let later = &labelled_scores[k..];
                let lost = later.iter().filter(|score| **score < floor).count();
                // How many silenced queries named a scope: the label a floor cannot suppress, and so
                // the evidence that lowers a wrong floor at the next calibration.
                let correcting = later
                    .iter()
                    .zip(&labelled[k..])
                    .filter(|(score, row)| **score < floor && row.2 == "scoped")
                    .count();
                let cut = probe_scores.iter().filter(|score| **score < floor).count();
                println!(
                    "  {:<8} {:>7.2} {:>8.3} {:>10} / {:<5} {:>16} {:>19.1}%",
                    k,
                    margin,
                    floor,
                    lost,
                    later.len(),
                    correcting,
                    share(cut, probe_scores.len())
                );
            }
        }
        let all = labelled_scores
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let cut_now = probe_scores.iter().filter(|score| **score < all).count();
        println!(
            "  calibrated on all {}: floor {:.3}, silencing {:.1}% of probes (in sample)",
            labelled_scores.len(),
            all,
            share(cut_now, probe_scores.len())
        );
    }

    println!(
        "\n  With no floor, nothing is silenced and nothing is lost: that is the row every other"
    );
    println!("  row is paying against.");
}

/// What a floor does to the queries it does *not* silence.
///
/// The walk-forward simulation asked only whether a query survived. This asks what survival costs:
/// the floor removes hits below it **before** the gap cut, so a query can come back having lost its
/// lower-ranked memories, and the cut then falls somewhere else. The top-ranked hit is not safe
/// either — ranking is content *plus* inherited score, so a memory the walk carried to the top can
/// sit below a floor that is asked of content alone.
///
/// Same inputs as `probe_simulate_a_floor_calibrated_from_labelled_queries`, and the floor is the one
/// that probe calibrates on the whole labelled set: the lowest raw top score.
#[test]
#[ignore = "reads this machine's live sets; needs SCATTERHOARD_MODEL, SCATTERHOARD_LABELLED and SCATTERHOARD_SET"]
fn probe_what_a_floor_costs_the_queries_it_does_not_silence() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let Ok(labelled_path) = std::env::var("SCATTERHOARD_LABELLED") else {
        println!("SCATTERHOARD_LABELLED unset -- point it at corpus/labelled-queries-<set>.json.");
        return;
    };
    let labelled: Vec<String> = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(&labelled_path).expect("read"),
    )
    .expect("parse")["queries"]
        .as_array()
        .expect("queries")
        .iter()
        .filter_map(|row| row["query"].as_str().map(ToString::to_string))
        .collect();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let wanted =
        std::env::var("SCATTERHOARD_SET").unwrap_or_else(|_| "shared:scatterhoard".to_owned());
    let (scope, path) = copies
        .iter()
        .find(|(s, _)| s.as_key() == wanted)
        .unwrap_or_else(|| panic!("no set {wanted} on this machine"));
    let mut store = Store::open_at(scope.clone(), path).expect("open");
    store.set_embedder(embedder.clone());

    const WINDOW: usize = 10;
    let raw = Stages {
        gap_cut: false,
        centring: Centring::None,
        ..Stages::default()
    };
    let shipping = Stages {
        gap_cut: false,
        ..Stages::default()
    };

    // Raw content similarity per memory, which is what the floor is proposed to read, and the
    // shipping ranking, which is what the caller actually receives.
    /// One returned memory: its id, the score it was ranked on, and its raw content similarity.
    type Ranked = (String, f64, f64);
    let mut floor_inputs: Vec<f64> = Vec::new();
    let mut per_query: Vec<Vec<Ranked>> = Vec::new();
    for query in &labelled {
        let raw_hits = recall_across_with(&mut [&mut store], &Question::new(query, WINDOW), raw)
            .expect("recall")
            .hits;
        let raw_of: std::collections::HashMap<String, f64> = raw_hits
            .iter()
            .map(|hit| (hit.memory.id.to_string(), f64::from(hit.similarity)))
            .collect();
        let best_raw = raw_of.values().copied().fold(f64::NEG_INFINITY, f64::max);
        floor_inputs.push(best_raw);

        let ranked: Vec<Ranked> =
            recall_across_with(&mut [&mut store], &Question::new(query, WINDOW), shipping)
                .expect("recall")
                .hits
                .iter()
                .map(|hit| {
                    let id = hit.memory.id.to_string();
                    let combined = traversal::combine_with(
                        f64::from(hit.similarity),
                        f64::from(hit.inherited),
                        shipping.inherited_weight,
                    );
                    let raw_similarity = raw_of.get(&id).copied().unwrap_or(f64::NEG_INFINITY);
                    (id, combined, raw_similarity)
                })
                .collect();
        per_query.push(ranked);
    }

    let floor = floor_inputs.iter().copied().fold(f64::INFINITY, f64::min);

    let (
        mut changed,
        mut dropped_total,
        mut top_dropped,
        mut emptied,
        mut baseline_total,
        mut floored_total,
    ) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for ranked in &per_query {
        let baseline_scores: Vec<f64> = ranked.iter().map(|(_, combined, _)| *combined).collect();
        let baseline = traversal::cut_at_largest_gap(&baseline_scores, WINDOW);
        let kept: Vec<&Ranked> = ranked.iter().filter(|(_, _, raw)| *raw >= floor).collect();
        let kept_scores: Vec<f64> = kept.iter().map(|(_, combined, _)| *combined).collect();
        let floored = if kept_scores.is_empty() {
            0
        } else {
            traversal::cut_at_largest_gap(&kept_scores, WINDOW)
        };

        baseline_total += baseline;
        floored_total += floored;
        if floored == 0 {
            emptied += 1;
        }
        let baseline_ids: Vec<&String> =
            ranked.iter().take(baseline).map(|(id, _, _)| id).collect();
        let floored_ids: Vec<&String> = kept.iter().take(floored).map(|(id, _, _)| id).collect();
        if baseline_ids != floored_ids {
            changed += 1;
            dropped_total += baseline_ids
                .iter()
                .filter(|id| !floored_ids.contains(id))
                .count();
        }
        if ranked.first().is_some_and(|(_, _, raw)| *raw < floor) {
            top_dropped += 1;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn per(total: usize, n: usize) -> f64 {
        total as f64 / n as f64
    }
    let n = per_query.len();
    println!(
        "\n  {wanted}: {n} labelled queries, floor {floor:.3} (lowest raw top score), window {WINDOW}"
    );
    println!(
        "  memories returned per query: {:.2} without the floor, {:.2} with it",
        per(baseline_total, n),
        per(floored_total, n)
    );
    println!("  queries whose returned set changed at all: {changed} of {n}");
    println!(
        "  memories dropped from those, in total: {dropped_total} (mean {:.2} per changed query)",
        if changed == 0 {
            0.0
        } else {
            per(dropped_total, changed)
        }
    );
    println!("  queries whose top-ranked hit was below the floor: {top_dropped}");
    println!("  queries left with nothing: {emptied}");
    println!(
        "\n  A dropped memory is not necessarily a wrong drop -- these queries have no relevance"
    );
    println!("  labels, so this measures how much the floor moves, not how much it costs.");
}

/// The floor's whole operating curve, so a cost ratio can pick the point.
///
/// The human partner settled the trade 2026-09-09: **bias toward over-recalling.** A missed memory
/// is disorienting in a way a surplus result is not -- *but you knew this last session* -- so a
/// false silence costs far more than a wasted hit. p90-of-the-negatives was never a chosen point,
/// only wherever a probe set happened to fall, and under that ruling it is plainly too aggressive.
///
/// Reports what each candidate floor silences and what it costs, so the point is picked rather than
/// inherited.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL and SCATTERHOARD_OFFDOMAIN"]
fn probe_the_floors_operating_curve() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let Ok(offdomain) = std::env::var("SCATTERHOARD_OFFDOMAIN") else {
        println!("SCATTERHOARD_OFFDOMAIN unset -- point it at corpus/offdomain-probes.json.");
        return;
    };
    let probes: Vec<String> = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(&offdomain).expect("read"),
    )
    .expect("parse")["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .filter_map(|q| q.as_str().map(ToString::to_string))
        .collect();
    let real: Vec<String> = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string("corpus/harvested-queries-pre-floor.json").expect("read"),
    )
    .expect("parse")["queries"]
        .as_array()
        .expect("queries")
        .iter()
        .filter_map(|row| row["query"].as_str().map(ToString::to_string))
        .collect();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let (scope, path) = copies
        .iter()
        .find(|(s, _)| s.as_key() == "shared:scatterhoard")
        .expect("the scatterhoard set");
    let mut store = Store::open_at(scope.clone(), path).expect("open");
    store.set_embedder(embedder.clone());

    let raw = Stages {
        centring: Centring::None,
        ..Stages::default()
    };
    let top1 = |store: &mut Store, q: &str| -> f64 {
        recall_across_with(&mut [store], &Question::new(q, 10), raw)
            .expect("recall")
            .hits
            .first()
            .map_or(0.0, |h| f64::from(h.similarity))
    };
    let real_scores: Vec<f64> = real.iter().map(|q| top1(&mut store, q)).collect();
    let probe_scores: Vec<f64> = probes.iter().map(|q| top1(&mut store, q)).collect();

    println!(
        "\n{} real queries, {} off-domain probes, raw cosine, window 10.\n",
        real.len(),
        probes.len()
    );
    println!(
        "  {:<28} {:>10} {:>20} {:>20}",
        "floor chosen at", "value", "off-domain silenced", "real queries lost"
    );
    for (label, floor) in [
        (
            "p00 of real queries",
            percentile(&mut real_scores.clone(), 0.00),
        ),
        (
            "p01 of real queries",
            percentile(&mut real_scores.clone(), 0.01),
        ),
        (
            "p05 of real queries",
            percentile(&mut real_scores.clone(), 0.05),
        ),
        (
            "p10 of real queries",
            percentile(&mut real_scores.clone(), 0.10),
        ),
        (
            "p50 of off-domain probes",
            percentile(&mut probe_scores.clone(), 0.50),
        ),
        (
            "p75 of off-domain probes",
            percentile(&mut probe_scores.clone(), 0.75),
        ),
        (
            "p90 of off-domain probes",
            percentile(&mut probe_scores.clone(), 0.90),
        ),
    ] {
        let silenced = probe_scores.iter().filter(|s| **s < floor).count();
        let lost = real_scores.iter().filter(|s| **s < floor).count();
        #[allow(clippy::cast_precision_loss)]
        let sp = 100.0 * silenced as f64 / probe_scores.len() as f64;
        #[allow(clippy::cast_precision_loss)]
        let lp = 100.0 * lost as f64 / real_scores.len() as f64;
        println!(
            "  {label:<28} {floor:>10.3} {:>12}/{} ({sp:>4.1}%) {:>12}/{} ({lp:>4.1}%)",
            silenced,
            probe_scores.len(),
            lost,
            real_scores.len()
        );
    }
    println!("\n  Under `bias toward over-recalling`, the rightmost column is the one that binds:");
    println!(
        "  a floor is only admissible if it loses no real query, and then silences what it can."
    );
}

/// Where a linked memory sits in the ranking of its source, re-measured.
///
/// `01M045BW5HEA98EH1Y62BF6HJB` put the median linked target at the 2.9th percentile of similarity
/// to its source, over 23 edges on 2026-08-15. The graph now carries 178. The percentile is what
/// decides whether a link earns its cost -- it is scale-invariant where rank is not -- so it is
/// worth re-deriving before any convention text quotes it.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_where_a_linked_memory_ranks_against_its_source() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let sources = sources(&copies);
    let inputs: Vec<String> = sources
        .iter()
        .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
        .collect();
    let vectors: Vec<Vec<f32>> = embedder
        .embed(&inputs)
        .expect("embed")
        .iter()
        .map(|v| v.components().to_vec())
        .collect();
    let index: HashMap<MemoryId, usize> =
        sources.iter().enumerate().map(|(i, s)| (s.id, i)).collect();

    // Edges the walk actually crosses: supersession is a validity relation and is never traversed.
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for (scope, path) in &copies {
        let store = Store::open_at(scope.clone(), path).expect("open");
        for source in sources
            .iter()
            .filter(|s| s.scope == scope.as_key().replace("shared:", ""))
        {
            let Ok(Some(memory)) = store.get(source.id) else {
                continue;
            };
            for link in &memory.links {
                if matches!(link.label, scatterhoard::memory::LinkLabel::Supersedes) {
                    continue;
                }
                // Cross-set targets resolve too: the union is embedded here as one population,
                // which is also how `recall_across` scores it.
                let target = match &link.target {
                    scatterhoard::memory::LinkTarget::SameSet(id) => Some(id),
                    scatterhoard::memory::LinkTarget::CrossSet { id, .. } => Some(id),
                };
                if let Some(target) = target
                    && let (Some(a), Some(b)) = (index.get(&source.id), index.get(target))
                {
                    edges.push((*a, *b));
                }
            }
        }
    }

    let mut percentiles: Vec<f64> = Vec::new();
    let mut ranks: Vec<f64> = Vec::new();
    for (from, to) in &edges {
        let mut scored: Vec<(usize, f64)> = vectors
            .iter()
            .enumerate()
            .filter(|(i, _)| i != from)
            .map(|(i, v)| (i, cosine_of(&vectors[*from], v)))
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        if let Some(rank) = scored.iter().position(|(i, _)| i == to) {
            #[allow(clippy::cast_precision_loss)]
            {
                ranks.push((rank + 1) as f64);
                percentiles.push(100.0 * (rank + 1) as f64 / scored.len() as f64);
            }
        }
    }
    println!(
        "\n{} traversable edges over {} memories.\n",
        percentiles.len(),
        sources.len()
    );
    println!(
        "  median percentile of a linked target: {:.2}%",
        percentile(&mut percentiles.clone(), 0.50)
    );
    println!(
        "  median rank:                          {:.0}",
        percentile(&mut ranks.clone(), 0.50)
    );
    println!(
        "  p25 / p75 of rank:                    {:.0} / {:.0}",
        percentile(&mut ranks.clone(), 0.25),
        percentile(&mut ranks.clone(), 0.75)
    );
    let inside = ranks.iter().filter(|r| **r <= 10.0).count();
    #[allow(clippy::cast_precision_loss)]
    let share = 100.0 * inside as f64 / ranks.len() as f64;
    println!(
        "  linked targets already inside a top-10 window: {inside}/{} ({share:.0}%)",
        ranks.len()
    );
    println!("\n  The ones outside are what a link buys: near enough to be worth reaching,");
    println!("  not near enough for content similarity to return them.");
}

/// When a hit's score was borrowed from a memory it replaced, how much did that lift it?
///
/// Superseded memories keep their vectors and are scanned, and `recall_across` folds a chain into
/// one candidate **carrying the best evidence any member matched** — deliberately, so that a query
/// matching an old claim's wording returns the correction. `Recalled.matched` names the ancestor
/// when that happened.
///
/// The question the human partner raised is whether that is also a source of noise, and the recorded
/// history says why it might be: the third consequence on
/// `validity-is-resolved-after-the-ranking` notes that the skew from ranking a tip on its
/// predecessor's value is **systematic rather than random, since supersession is typically a guess
/// replaced by a measurement**. That was said of confidence. It applies to similarity the same way —
/// the predecessor's text is the earlier and often wronger formulation, so a tip lifted by it is
/// lifted by wording the set has already retracted.
///
/// Measured by re-scoring each such hit on its *own* text and comparing.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_how_much_a_hit_is_lifted_by_the_memory_it_replaced() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let real: Vec<String> = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string("corpus/harvested-queries-pre-floor.json").expect("read"),
    )
    .expect("parse")["queries"]
        .as_array()
        .expect("queries")
        .iter()
        .filter_map(|row| row["query"].as_str().map(ToString::to_string))
        .collect();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let mut opened: Vec<Store> = copies
        .iter()
        .map(|(scope, path)| {
            let mut store = Store::open_at(scope.clone(), path).expect("open");
            store.set_embedder(embedder.clone());
            store
        })
        .collect();

    // **Both sides must be scored the way the pipeline scores**, which is centred on a centroid
    // built over every set being searched. Re-embedding the tip and taking a raw cosine compares two
    // different scales and produces a "lift" that is mostly the difference between them -- the same
    // raw-versus-centred error that invalidated a day of calibration figures earlier today.
    let union: Vec<Vec<f32>> = {
        let sources = sources(&copies);
        let inputs: Vec<String> = sources
            .iter()
            .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
            .collect();
        embedder
            .embed(&inputs)
            .expect("embed")
            .iter()
            .map(|v| v.components().to_vec())
            .collect()
    };
    let stored: HashMap<MemoryId, Vec<f32>> = {
        let sources = sources(&copies);
        sources
            .iter()
            .zip(&union)
            .map(|(s, v)| (s.id, v.clone()))
            .collect()
    };
    let centroid: Vec<f32> = {
        let dimension = union[0].len();
        let mut c = vec![0.0f32; dimension];
        for v in &union {
            for (slot, value) in c.iter_mut().zip(v) {
                *slot += value;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let n = union.len() as f32;
        for slot in &mut c {
            *slot /= n;
        }
        c
    };
    let centred = |a: &[f32], b: &[f32]| -> f64 {
        let ca: Vec<f32> = a.iter().zip(&centroid).map(|(x, c)| x - c).collect();
        let cb: Vec<f32> = b.iter().zip(&centroid).map(|(x, c)| x - c).collect();
        cosine_of(&ca, &cb)
    };

    let mut hits = 0usize;
    let mut borrowed = 0usize;
    let mut lifts: Vec<f64> = Vec::new();
    let mut worst: Vec<(f64, f64, String, String)> = Vec::new();
    let mut top_slot = 0usize;

    for query in &real {
        let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
        let recall = recall_across(&mut refs, &Question::new(query, 10)).expect("recall");
        let vector = embedder
            .embed(std::slice::from_ref(query))
            .expect("embed")
            .remove(0);
        for (position, hit) in recall.hits.iter().enumerate() {
            hits += 1;
            let Some(ancestor) = hit.matched else {
                continue;
            };
            borrowed += 1;
            if position == 0 {
                top_slot += 1;
            }
            // What this memory scores on its own stored vector, centred exactly as the scan does.
            let (Some(tip_vector), Some(ancestor_vector)) =
                (stored.get(&hit.memory.id), stored.get(&ancestor))
            else {
                continue;
            };
            let own_score = centred(tip_vector, vector.components());
            let ancestor_score = centred(ancestor_vector, vector.components());
            let lift = ancestor_score - own_score;
            lifts.push(lift);
            worst.push((
                lift,
                ancestor_score,
                query.clone(),
                format!("{} <- {}", hit.memory.id, ancestor),
            ));
        }
    }

    println!("\n{} queries, {hits} hits returned.\n", real.len());
    #[allow(clippy::cast_precision_loss)]
    let share = 100.0 * borrowed as f64 / hits as f64;
    println!("  hits scored on a memory they replaced: {borrowed} of {hits} ({share:.1}%)");
    println!("  of those, in the top slot: {top_slot}");
    if lifts.is_empty() {
        println!("\n  Nothing borrowed: the chains present never out-scored their own tips.");
        return;
    }
    println!(
        "  lift over the tip's own text: median {:+.3}, p90 {:+.3}, max {:+.3}",
        percentile(&mut lifts.clone(), 0.50),
        percentile(&mut lifts.clone(), 0.90),
        percentile(&mut lifts.clone(), 1.00)
    );
    let harmful = lifts.iter().filter(|l| **l > 0.02).count();
    println!("  hits lifted by more than 0.02: {harmful}");
    // Signal or noise turns on whether the tip is still about what the ancestor was about. A
    // correction keeps the subject and changes the answer, and surfacing it is the feature working.
    // A supersession that moved the subject leaves a memory returned for wording it does not carry
    // and no longer concerns, which is the case worth separating out.
    println!("\n  Did the subject survive the supersession?");
    let mut kinship: Vec<f64> = Vec::new();
    for (_, _, _, pair) in &worst {
        let mut halves = pair.split(" <- ");
        let (Some(tip), Some(ancestor)) = (halves.next(), halves.next()) else {
            continue;
        };
        let (Ok(tip), Ok(ancestor)) = (MemoryId::parse(tip), MemoryId::parse(ancestor)) else {
            continue;
        };
        if let (Some(a), Some(b)) = (stored.get(&tip), stored.get(&ancestor)) {
            kinship.push(centred(a, b));
        }
    }
    println!(
        "      tip-to-ancestor similarity: median {:.3}, p10 {:.3}, min {:.3}",
        percentile(&mut kinship.clone(), 0.50),
        percentile(&mut kinship.clone(), 0.10),
        percentile(&mut kinship.clone(), 0.00)
    );
    let drifted = kinship.iter().filter(|k| **k < 0.5).count();
    println!(
        "      pairs below 0.5, where the subject arguably moved: {drifted} of {}",
        kinship.len()
    );

    // **Is the extra surface a function of how far the rewrite moved the vector?** If a supersession
    // barely changes the text -- adding a link, fixing a keyword -- the ancestor sits almost on top
    // of the tip and can lift it nowhere. A rewrite that moves the vector covers a region the tip
    // does not, and that region is where a query can land on wording the memory no longer carries.
    // Geometrically the lift is bounded by the distance, so the question is the magnitude and
    // whether the far pairs are the doubtful ones.
    println!("\n  Lift against how far the supersession moved the vector:");
    let mut paired: Vec<(f64, f64)> = Vec::new();
    for (lift, _, _, pair) in &worst {
        let mut halves = pair.split(" <- ");
        let (Some(tip), Some(ancestor)) = (halves.next(), halves.next()) else {
            continue;
        };
        let (Ok(tip), Ok(ancestor)) = (MemoryId::parse(tip), MemoryId::parse(ancestor)) else {
            continue;
        };
        if let (Some(a), Some(b)) = (stored.get(&tip), stored.get(&ancestor)) {
            paired.push((1.0 - centred(a, b), *lift));
        }
    }
    let xs: Vec<f64> = paired.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = paired.iter().map(|p| p.1).collect();
    let (r, rho) = correlations(&xs, &ys);
    println!(
        "      distance moved vs lift: r {r:.3} (r^2 {:.3}), rho {rho:.3}",
        r * r
    );
    for (label, low, high) in [
        ("barely moved  (distance < 0.30)", 0.0, 0.30),
        ("moved somewhat (0.30 - 0.45)", 0.30, 0.45),
        ("rewritten     (distance > 0.45)", 0.45, 2.0),
    ] {
        let bucket: Vec<f64> = paired
            .iter()
            .filter(|(d, _)| *d >= low && *d < high)
            .map(|(_, l)| *l)
            .collect();
        if bucket.is_empty() {
            println!("      {label:<34} none");
        } else {
            println!(
                "      {label:<34} {:>2} pairs, median lift {:+.3}, max {:+.3}",
                bucket.len(),
                percentile(&mut bucket.clone(), 0.50),
                percentile(&mut bucket.clone(), 1.00)
            );
        }
    }

    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("\n  The largest lifts:");
    for (lift, score, query, pair) in worst.iter().take(5) {
        println!(
            "      {lift:+.3} (returned at {score:.3})  {}",
            &query[..query.len().min(58)]
        );
        println!("          {pair}");
    }
}

/// What do the superseded bodies contribute to the floor, and to its variation?
///
/// The human partner's question: chains whose rewrite moved the vector a long way lift their tips by
/// a median 0.191, so do they inflate the floor and account for some of the variation seen between
/// calibrations? The clean test is to calibrate twice — once against the whole scan, once against
/// **the tips only** — and compare.
///
/// A floor calibrated on inflated probe scores is a floor set too high, which under *bias toward
/// over-recalling* is the expensive direction: it costs real queries.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL and SCATTERHOARD_OFFDOMAIN"]
fn probe_what_the_superseded_bodies_contribute_to_the_floor() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let Ok(offdomain) = std::env::var("SCATTERHOARD_OFFDOMAIN") else {
        println!("SCATTERHOARD_OFFDOMAIN unset -- point it at corpus/offdomain-probes.json.");
        return;
    };
    let probes: Vec<String> = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(&offdomain).expect("read"),
    )
    .expect("parse")["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .filter_map(|q| q.as_str().map(ToString::to_string))
        .collect();
    let real: Vec<String> = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string("corpus/harvested-queries-pre-floor.json").expect("read"),
    )
    .expect("parse")["queries"]
        .as_array()
        .expect("queries")
        .iter()
        .filter_map(|row| row["query"].as_str().map(ToString::to_string))
        .collect();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let mut opened: Vec<Store> = copies
        .iter()
        .map(|(scope, path)| {
            let mut store = Store::open_at(scope.clone(), path).expect("open");
            store.set_embedder(embedder.clone());
            store
        })
        .collect();

    println!(
        "\n{} off-domain probes, {} real queries, raw cosine, window 10.\n",
        probes.len(),
        real.len()
    );
    println!(
        "  {:<26} {:>10} {:>12} {:>12} {:>14}",
        "scan", "probe p90", "probe median", "real min", "real median"
    );

    let mut summary: Vec<(bool, f64, Vec<f64>, Vec<f64>)> = Vec::new();
    for include in [true, false] {
        let stages = Stages {
            centring: Centring::None,
            include_superseded: include,
            ..Stages::default()
        };
        let top1 = |opened: &mut Vec<Store>, q: &str| -> f64 {
            let mut refs: Vec<&mut Store> = opened.iter_mut().collect();
            recall_across_with(&mut refs, &Question::new(q, 10), stages)
                .expect("recall")
                .hits
                .first()
                .map_or(0.0, |h| f64::from(h.similarity))
        };
        let probe_scores: Vec<f64> = probes.iter().map(|q| top1(&mut opened, q)).collect();
        let real_scores: Vec<f64> = real.iter().map(|q| top1(&mut opened, q)).collect();
        let floor = percentile(&mut probe_scores.clone(), 0.90);
        println!(
            "  {:<26} {floor:>10.3} {:>12.3} {:>12.3} {:>14.3}",
            if include {
                "whole chain (ships)"
            } else {
                "tips only"
            },
            percentile(&mut probe_scores.clone(), 0.50),
            percentile(&mut real_scores.clone(), 0.00),
            percentile(&mut real_scores.clone(), 0.50)
        );
        summary.push((include, floor, probe_scores, real_scores));
    }

    let (_, whole_floor, whole_probes, whole_real) = &summary[0];
    let (_, tips_floor, tips_probes, tips_real) = &summary[1];
    println!(
        "\n  floor moves {:+.4} when the superseded bodies leave the scan",
        tips_floor - whole_floor
    );

    // How many probes and real queries moved at all, and by how much.
    let moved = |a: &[f64], b: &[f64]| -> (usize, f64) {
        let deltas: Vec<f64> = a.iter().zip(b).map(|(x, y)| (y - x).abs()).collect();
        (
            deltas.iter().filter(|d| **d > 1e-6).count(),
            deltas.iter().copied().fold(0.0, f64::max),
        )
    };
    let (probes_moved, probes_max) = moved(whole_probes, tips_probes);
    let (real_moved, real_max) = moved(whole_real, tips_real);
    println!(
        "  off-domain probes whose top score changed: {probes_moved} of {}, largest {probes_max:.3}",
        whole_probes.len()
    );
    println!(
        "  real queries whose top score changed:      {real_moved} of {}, largest {real_max:.3}",
        whole_real.len()
    );

    println!(
        "\n  Under the admissible-floor rule -- lose no real query, then silence what you can:"
    );
    for (label, floor, probe_scores, real_scores) in [
        ("whole chain (ships)", whole_floor, whole_probes, whole_real),
        ("tips only", tips_floor, tips_probes, tips_real),
    ] {
        let admissible = real_scores.iter().copied().fold(f64::INFINITY, f64::min);
        let silenced = probe_scores.iter().filter(|s| **s < admissible).count();
        #[allow(clippy::cast_precision_loss)]
        let share = 100.0 * silenced as f64 / probe_scores.len() as f64;
        println!(
            "      {label:<22} admissible floor {admissible:.3}, silences {silenced}/{} ({share:.1}%)  [p90 would be {floor:.3}]",
            probe_scores.len()
        );
    }
}

/// Does semantic recall work at all outside Latin script?
///
/// The human partner asked whether an English calibration set could serve a Chinese or Hindi user.
/// That question has a prior: `potion-base-32M` is distilled from `baai/bge-base-en-v1.5`, whose
/// WordPiece vocabulary is **92.8% Latin** — 58,548 of 63,091 tokens. CJK gets 492 characters where
/// common usage needs three to five thousand; Devanagari gets 71, roughly the bare alphabet. And
/// `model2vec-rs` **discards `[UNK]` before pooling**, so whatever the tokenizer cannot name
/// contributes nothing rather than contributing noise.
///
/// Three things decide it, in order. **Does a vector exist at all** — `Vector::new` refuses a
/// direction-less vector, so text that tokenises entirely to `[UNK]` cannot be embedded. **How much
/// of the text survives** tokenisation. And **does anything separate**: if a language's text
/// collapses onto the handful of tokens the vocabulary does hold, then every query in it matches
/// every memory in it, and no floor calibrated on anything can help.
#[test]
#[ignore = "needs SCATTERHOARD_MODEL"]
fn probe_whether_recall_works_outside_latin_script() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let Ok(model_dir) = std::env::var("SCATTERHOARD_MODEL") else {
        return;
    };
    let tokenizer: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(&model_dir).join("tokenizer.json")).expect("read"),
    )
    .expect("parse");
    let vocab: std::collections::HashSet<String> = tokenizer["model"]["vocab"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    // One question and one unrelated statement per language. The pair is what tests separation:
    // if a script collapses onto a few tokens, the two will score alike against each other.
    let samples: [(&str, &str, &str); 8] = [
        (
            "English",
            "should we rent our own car",
            "the soup needs more salt",
        ),
        (
            "Spanish",
            "deberiamos alquilar nuestro propio coche",
            "la sopa necesita mas sal",
        ),
        (
            "German",
            "sollten wir unser eigenes Auto mieten",
            "die Suppe braucht mehr Salz",
        ),
        (
            "French",
            "devrions-nous louer notre propre voiture",
            "la soupe a besoin de plus de sel",
        ),
        (
            "Russian",
            "должны ли мы арендовать собственную машину",
            "супу нужно больше соли",
        ),
        ("Chinese", "我们应该租一辆自己的车吗", "这个汤需要多放点盐"),
        (
            "Hindi",
            "क्या हमें अपनी कार किराए पर लेनी चाहिए",
            "सूप में और नमक चाहिए",
        ),
        (
            "Japanese",
            "自分の車を借りるべきですか",
            "スープにもっと塩が必要です",
        ),
    ];

    println!(
        "\n  {:<10} {:>12} {:>10} {:>12} {:>16}",
        "language", "chars", "tokens", "unknown", "pair similarity"
    );
    for (language, question, unrelated) in samples {
        let tokens = tokenize(&vocab, question);
        let unknown = tokens.iter().filter(|t| *t == "[UNK]").count();
        let a = embedder.embed(&[question.to_string()]);
        let b = embedder.embed(&[unrelated.to_string()]);
        let similarity = match (&a, &b) {
            (Ok(a), Ok(b)) if !a.is_empty() && !b.is_empty() => {
                format!("{:>16.3}", cosine_of(a[0].components(), b[0].components()))
            }
            _ => format!("{:>16}", "NO VECTOR"),
        };
        println!(
            "  {language:<10} {:>12} {:>10} {:>10}/{:<2}{similarity}",
            question.chars().count(),
            tokens.len(),
            unknown,
            tokens.len()
        );
    }

    // What the *real* embedder does with each, since the token counts above come from this file's
    // own WordPiece helper, which treats CJK as one word where `BertPreTokenizer` splits it per
    // character. The error text is the fact worth having.
    println!("\n  What the shipping embedder returns:");
    for (language, question, _) in samples {
        match embedder.embed(&[question.to_string()]) {
            Ok(v) if !v.is_empty() => {
                let norm_of = norm(v[0].components());
                println!(
                    "      {language:<10} vector of {} dimensions, norm {norm_of:.3}",
                    v[0].components().len()
                );
            }
            Ok(_) => println!("      {language:<10} embedded to nothing"),
            Err(error) => println!("      {language:<10} REFUSED: {error}"),
        }
    }
    println!("\n  And the unrelated member of each pair, which is where a refusal appeared:");
    for (language, _, unrelated) in samples {
        match embedder.embed(&[unrelated.to_string()]) {
            Ok(v) if !v.is_empty() => println!("      {language:<10} ok"),
            Ok(_) => println!("      {language:<10} embedded to nothing"),
            Err(error) => println!("      {language:<10} REFUSED: {error}"),
        }
    }

    println!("\n  `pair similarity` is one question against an unrelated statement in the *same*");
    println!("  language. Near zero is healthy -- they are about different things. Near one means");
    println!(
        "  the script has collapsed onto whatever few tokens the vocabulary holds, and nothing"
    );
    println!("  in that language can be told apart by meaning.");
}

/// Does meaning survive in each language, or only the absence of collapse?
///
/// `probe_whether_recall_works_outside_latin_script` scores a question against an unrelated
/// statement, and a score near zero shows the script has *not collapsed* -- but a model that scored
/// everything near zero would pass it too. This is the other half: a paraphrase has to score clearly
/// above an unrelated sentence, in the same language, before the model can be said to represent
/// meaning there at all.
///
/// Each triplet in `language_fit::BUILT_IN` is an anchor, a paraphrase worded to share as little with
/// it as the language allows, and an unrelated sentence. Where a language could not avoid a shared
/// word, a margin here is an upper bound on meaning rather than a clean measure of it.
///
/// The cross-language columns score the **English** anchor against each language's paraphrase and
/// unrelated sentence: what an English query would find in a memory a collaborator wrote in that
/// language. Raw cosine, no centring -- these are pairs, not a corpus.
#[test]
#[ignore = "needs SCATTERHOARD_MODEL"]
fn probe_whether_paraphrases_outscore_unrelated_text_in_each_language() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };

    // `embed_each`, because under a model that lacks a script a sentence can come back with no
    // vector at all, and that is a result to report rather than a reason to stop.
    let one = |text: &str| -> Option<Vec<f32>> {
        match embedder
            .embed_each(&[text.to_string()])
            .expect("the model embeds")
            .remove(0)
        {
            Embedded::Vector(vector) => Some(vector.components().to_vec()),
            Embedded::Unrepresentable => None,
        }
    };
    let english = BUILT_IN[0].triplets;

    println!(
        "\n  {:<10} {:>9} {:>11} {:>8} {:>6}   {:>14} {:>11}",
        "language", "related", "unrelated", "margin", "wins", "cross margin", "cross wins"
    );
    for Sentences {
        name: language,
        triplets,
        ..
    } in &BUILT_IN
    {
        let (mut related, mut unrelated, mut cross) = (Vec::new(), Vec::new(), Vec::new());
        let (mut wins, mut cross_wins, mut missing) = (0usize, 0usize, 0usize);
        for (index, [anchor, paraphrase, other]) in triplets.iter().enumerate() {
            let (Some(a), Some(p), Some(u)) = (one(anchor), one(paraphrase), one(other)) else {
                missing += 1;
                continue;
            };
            let (r, n) = (cosine_of(&a, &p), cosine_of(&a, &u));
            wins += usize::from(r > n);
            related.push(r);
            unrelated.push(n);
            if let Some(e) = one(english[index][0]) {
                let (cr, cn) = (cosine_of(&e, &p), cosine_of(&e, &u));
                cross_wins += usize::from(cr > cn);
                cross.push(cr - cn);
            }
        }
        let scored = triplets.len() - missing;
        let margins: Vec<f64> = related.iter().zip(&unrelated).map(|(r, n)| r - n).collect();
        let note = if missing > 0 {
            format!("   {missing} with no vector")
        } else {
            String::new()
        };
        println!(
            "  {language:<10} {:>9.3} {:>11.3} {:>8.3} {:>4}/{scored}   {:>14.3} {:>9}/{}{note}",
            mean(&related),
            mean(&unrelated),
            mean(&margins),
            wins,
            mean(&cross),
            cross_wins,
            cross.len()
        );
    }

    println!(
        "\n  `related` scores an anchor against its paraphrase and `unrelated` against an unrelated"
    );
    println!(
        "  sentence, both in the same language; `margin` is the difference and `wins` how many"
    );
    println!("  triplets put the paraphrase first. A margin near zero means the language has not");
    println!(
        "  collapsed -- that is the other probe's question -- but cannot tell a paraphrase from a"
    );
    println!(
        "  stranger either. The cross columns use the English anchor: what an English query finds."
    );
}

/// Does each anchor's paraphrase rank first among everything else written in its language?
///
/// The proposed check for `recall-is-latin-script-only`. A threshold on raw similarity was rejected
/// because its scale moves with the model -- mean pairwise cosine among unrelated Russian sentences
/// is 0.87 under `potion-base-32M` and 0.31 under a retrieval model -- and a rank cancels the scale:
/// what matters is only whether the paraphrase beats its rivals, whatever the numbers are.
///
/// The rivals are every other sentence in that language from `language_fit::BUILT_IN`: the other
/// anchors and paraphrases as well as the unrelated sentences, 29 in all. That is harder than the
/// seven rivals three triplets gave on 2026-10-03, so a score here is not comparable with that one.
///
/// The grid prints the paraphrase's rank for each triplet, `1` meaning first, so a triplet that fails
/// in every language reads as a fault in the triplet rather than in the model. `margin` is the mean
/// of the paraphrase's score less its best rival's, negative where it lost.
#[test]
#[ignore = "needs SCATTERHOARD_MODEL"]
fn probe_whether_each_language_ranks_a_paraphrase_first() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    // `embed_each`, because under a model that lacks a script a sentence can come back with no
    // vector at all, and that is a result to report rather than a reason to stop.
    let one = |text: &str| -> Option<Vec<f32>> {
        match embedder
            .embed_each(&[text.to_string()])
            .expect("the model embeds")
            .remove(0)
        {
            Embedded::Vector(vector) => Some(vector.components().to_vec()),
            Embedded::Unrepresentable => None,
        }
    };

    println!(
        "\n  {:<10} {:>6} {:>8}   rank of the paraphrase, triplet by triplet",
        "language", "first", "margin"
    );
    for Sentences {
        name: language,
        triplets,
        ..
    } in &BUILT_IN
    {
        // Every sentence in the language, tagged with its triplet and its role in it.
        let sentences: Vec<(usize, usize, Option<Vec<f32>>)> = triplets
            .iter()
            .enumerate()
            .flat_map(|(index, triplet)| {
                triplet
                    .iter()
                    .enumerate()
                    .map(move |(role, text)| (index, role, *text))
            })
            .map(|(index, role, text)| (index, role, one(text)))
            .collect();

        let (mut first, mut scored, mut margins, mut grid) =
            (0usize, 0usize, Vec::new(), Vec::new());
        for (index, _) in triplets.iter().enumerate() {
            let vector = |role: usize| {
                sentences
                    .iter()
                    .find(|(i, r, _)| *i == index && *r == role)
                    .and_then(|(_, _, v)| v.as_ref())
            };
            let (Some(anchor), Some(paraphrase)) = (vector(0), vector(1)) else {
                grid.push("  -".to_string());
                continue;
            };
            let target = cosine_of(anchor, paraphrase);
            let rivals: Vec<f64> = sentences
                .iter()
                .filter(|(i, r, _)| !(*i == index && *r <= 1))
                .filter_map(|(_, _, v)| v.as_ref().map(|v| cosine_of(anchor, v)))
                .collect();
            let rank = 1 + rivals.iter().filter(|score| **score >= target).count();
            let best = rivals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            scored += 1;
            first += usize::from(rank == 1);
            margins.push(target - best);
            grid.push(format!("{rank:>3}"));
        }
        // The grid is this probe's own; the count is what the server reports, and the two must
        // agree or the probe is measuring a different rule from the one that ships.
        assert_eq!(
            usize::from(language_fit::score(&*embedder, triplets).expect("the model embeds")),
            first,
            "{language}: the probe and `language_fit::score` disagree"
        );
        let missing = sentences.iter().filter(|(_, _, v)| v.is_none()).count();
        let note = if missing > 0 {
            format!("   {missing} sentences with no vector")
        } else {
            String::new()
        };
        println!(
            "  {language:<10} {first:>3}/{scored:<2} {:>8.3}   {}{note}",
            mean(&margins),
            grid.join("")
        );
    }

    println!(
        "\n  A tie counts against the paraphrase: a model that scores everything alike has not"
    );
    println!("  told the paraphrase apart, and `>=` is what keeps it from passing on a tie.");
}

/// What happens when a memory's text cannot be embedded at all?
///
/// `probe_whether_recall_works_outside_latin_script` found a Chinese sentence the model refuses:
/// every character falls outside the 492 CJK tokens the vocabulary holds, all become `[UNK]`,
/// `model2vec-rs` discards those before pooling, and `Vector::new` refuses the direction-less result.
/// The question that matters for a user is whether the *memory* survives that.
#[test]
#[ignore = "needs SCATTERHOARD_MODEL"]
fn probe_storing_a_memory_the_model_cannot_embed() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let mut store = Store::open_at(
        Scope::from_key("local").expect("scope"),
        &scratch.path().join("trial.sqlite3"),
    )
    .expect("open a fresh store");
    store.set_embedder(embedder.clone());

    for (label, content) in [
        ("English", "the soup needs more salt"),
        ("Chinese", "这个汤需要多放点盐"),
    ] {
        let memory = scatterhoard::memory::Memory::new(
            scatterhoard::memory::MemoryText::new(content).expect("valid content"),
            scatterhoard::memory::Confidence::new(0.8).expect("in range"),
            scatterhoard::memory::Origin::Llm,
            scatterhoard::memory::Attribution {
                by: scatterhoard::memory::MemoryId::generate(),
                at: jiff::Timestamp::now(),
            },
        );
        let id = memory.id;
        match store.insert(&memory) {
            Ok(()) => {
                let found = store.get(id).ok().flatten().is_some();
                let searchable = store
                    .recall(&Question::new(content, 5))
                    .map(|r| r.hits.iter().any(|h| h.memory.id == id))
                    .unwrap_or(false);
                println!(
                    "  {label:<8} stored: yes   readable: {found}   findable by its own text: {searchable}"
                );
            }
            Err(error) => println!("  {label:<8} STORE FAILED: {error}"),
        }
    }
    println!(
        "\n  A memory that stores but cannot be found by its own text is the worse outcome of"
    );
    println!("  the two, because nothing reports it: the write succeeded and the recall is simply");
    println!("  empty.");
}

/// Does how badly a query fragments predict what it scores?
///
/// The human partner's connection, made from the nonsense classes: keyboard mashing and symbol runs
/// scored high because they **tokenise poorly**. Across seven class medians, tokens-per-word against
/// median score gives r 0.827 — better than anything else tried on this entry, where word overlap
/// reached r² 0.10 and the single-strongest-constituent statistic 0.01.
///
/// Class medians hide everything, so this runs it per query over the real corpus. It is the same
/// mechanism as the script failure one scale down: Chinese collapses because every character becomes
/// `[UNK]`, and `>>> <<<` scores 0.478 because every character becomes its own token.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_fragmentation_predicts_the_score() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let Ok(model_dir) = std::env::var("SCATTERHOARD_MODEL") else {
        return;
    };
    let tokenizer: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(&model_dir).join("tokenizer.json")).expect("read"),
    )
    .expect("parse");
    let vocab: std::collections::HashSet<String> = tokenizer["model"]["vocab"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let sources = sources(&copies);
    let inputs: Vec<String> = sources
        .iter()
        .map(|s| Composition::Live.input(&s.content, s.why.as_deref(), &s.keywords))
        .collect();
    let vectors: Vec<Vec<f32>> = embedder
        .embed(&inputs)
        .expect("embed")
        .iter()
        .map(|v| v.components().to_vec())
        .collect();

    let real: Vec<String> = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string("corpus/harvested-queries-pre-floor.json").expect("read"),
    )
    .expect("parse")["queries"]
        .as_array()
        .expect("queries")
        .iter()
        .filter_map(|row| row["query"].as_str().map(ToString::to_string))
        .collect();
    let nonsense: Vec<String> = NONSENSE.iter().map(|(_, q)| (*q).to_string()).collect();
    let substance: Vec<String> = CALIBRATION_PROBES
        .iter()
        .map(|q| (*q).to_string())
        .collect();

    let fragmentation = |text: &str| -> f64 {
        let words = pretokenize(text).len().max(1);
        #[allow(clippy::cast_precision_loss)]
        let ratio = tokenize(&vocab, text).len() as f64 / words as f64;
        ratio
    };

    println!(
        "\n  {:<30} {:>5} {:>12} {:>12} {:>10}",
        "query set", "n", "mean tok/word", "mean top1", "r"
    );
    let mut pooled: Vec<(f64, f64)> = Vec::new();
    for (label, set) in [
        ("real in-domain queries", &real),
        ("substance-shaped probes", &substance),
        ("nonsense, all classes", &nonsense),
    ] {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for query in set {
            let v = embedder
                .embed(std::slice::from_ref(query))
                .expect("embed")
                .remove(0);
            let top = top1_raw(&vectors, v.components());
            xs.push(fragmentation(query));
            ys.push(top);
            pooled.push((fragmentation(query), top));
        }
        let (r, _) = correlations(&xs, &ys);
        println!(
            "  {label:<30} {:>5} {:>12.2} {:>12.3} {:>10.3}",
            set.len(),
            mean(&xs),
            mean(&ys),
            r
        );
    }
    let xs: Vec<f64> = pooled.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = pooled.iter().map(|p| p.1).collect();
    let (r, rho) = correlations(&xs, &ys);
    println!(
        "\n  pooled over all {} queries: r {r:.3} (r^2 {:.3}), rho {rho:.3}",
        pooled.len(),
        r * r
    );
    println!("\n  For comparison, on the same corpus: word-level overlap r^2 0.100, subword mean");
    println!("  0.113, subword max 0.012. Fragmentation is the first query-side property measured");
    println!("  here that predicts anything.");
}

/// What a **windowed percentile** floor would have done, walked forward — the rule that replaces
/// `min − margin`.
///
/// `min` over an ever-growing set is monotonically non-increasing, so recalibration could only ever
/// lower a floor and one mislabelled query set the level for good. The replacement keeps a bounded
/// window of the most recent labelled queries, takes a low percentile of it, and recalibrates every
/// `trigger` new labels. This replays that over the real queries in the order they were asked, and
/// asks four things of each configuration: how often it recalibrates, whether the level moves in
/// both directions, what it costs the labelled queries that came after, and what it silences of the
/// off-domain probe set.
///
/// Same inputs and the same environment variables as
/// `probe_simulate_a_floor_calibrated_from_labelled_queries`. **Raw scores only** — that the floor
/// must read raw rather than centred was settled by that probe, and re-measuring it here would only
/// invite the two to disagree.
///
/// **What this cannot answer.** The corpus is held at what the set holds today while the real one
/// grew underneath these queries, so a rule that tracks a growing store is indistinguishable here
/// from one that does not. Windowing was adopted *because* the level should rise as a store grows;
/// this measures only that it can move both ways and what it costs, not that it tracks growth.
/// `probe_how_much_a_calibration_floor_varies_across_corpora` is the one that speaks to drift.
#[test]
#[ignore = "reads this machine's live sets; needs SCATTERHOARD_MODEL, SCATTERHOARD_LABELLED and SCATTERHOARD_OFFDOMAIN"]
fn probe_sweep_a_windowed_percentile_floor() {
    let Some(embedder) = model() else {
        println!("SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip.");
        return;
    };
    let (Ok(labelled_path), Ok(offdomain_path)) = (
        std::env::var("SCATTERHOARD_LABELLED"),
        std::env::var("SCATTERHOARD_OFFDOMAIN"),
    ) else {
        println!("SCATTERHOARD_LABELLED and SCATTERHOARD_OFFDOMAIN must both be set.");
        return;
    };

    let labelled_file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&labelled_path).expect("read"))
            .expect("parse");
    let mut labelled: Vec<(String, String, String)> = labelled_file["queries"]
        .as_array()
        .expect("queries")
        .iter()
        .filter_map(|row| {
            Some((
                row["first_seen"].as_str()?.to_owned(),
                row["query"].as_str()?.to_owned(),
                row["label"].as_str().unwrap_or("used").to_owned(),
            ))
        })
        .collect();
    labelled.sort_by(|a, b| a.0.cmp(&b.0));
    let probes: Vec<String> = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(&offdomain_path).expect("read"),
    )
    .expect("parse")["questions"]
        .as_array()
        .expect("questions")
        .iter()
        .filter_map(|q| q.as_str().map(ToString::to_string))
        .collect();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let wanted =
        std::env::var("SCATTERHOARD_SET").unwrap_or_else(|_| "shared:scatterhoard".to_owned());
    let (scope, path) = copies
        .iter()
        .find(|(s, _)| s.as_key() == wanted)
        .unwrap_or_else(|| panic!("no set {wanted} on this machine"));
    let mut store = Store::open_at(scope.clone(), path).expect("open");
    store.set_embedder(embedder.clone());
    if !matches!(
        store.model_state().expect("model state"),
        scatterhoard::store::ModelState::Current { .. }
    ) {
        let rebuilt = store.rebuild_vectors().expect("rebuild");
        println!(
            "  (rebuilt {} vectors under the loaded model)",
            rebuilt.vectors
        );
    }

    let stages = Stages {
        gap_cut: false,
        centring: Centring::None,
        floor: Floor::None,
        ..Stages::default()
    };
    let best = |store: &mut Store, query: &str| -> f64 {
        recall_across_with(&mut [store], &Question::new(query, 10), stages)
            .expect("recall")
            .hits
            .iter()
            .map(|hit| f64::from(hit.similarity))
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let labelled_scores: Vec<f64> = labelled
        .iter()
        .map(|(_, q, _)| best(&mut store, q))
        .collect();
    let probe_scores: Vec<f64> = probes.iter().map(|q| best(&mut store, q)).collect();

    #[allow(clippy::cast_precision_loss)]
    fn share(count: usize, total: usize) -> f64 {
        100.0 * count as f64 / total as f64
    }

    println!(
        "\n  {} labelled queries against {} memories, {} off-domain probes. Raw scores.",
        labelled.len(),
        store.count().expect("count"),
        probes.len()
    );
    println!(
        "  {:<7} {:>4} {:>8} {:>7} {:>6} {:>13} {:>17} {:>15} {:>12}",
        "window",
        "pct",
        "trigger",
        "margin",
        "calib",
        "floor range",
        "final floor",
        "later lost",
        "probes cut"
    );

    for window in [40usize, 100, usize::MAX] {
        for pct in [0.0f64, 0.05, 0.10, 0.20] {
            for trigger in [20usize, 40] {
                for margin in [0.0f64, 0.05] {
                    let mut held: Vec<f64> = Vec::new();
                    let mut floor: Option<f64> = None;
                    let mut since = 0usize;
                    let mut calibrations = 0usize;
                    let mut levels: Vec<f64> = Vec::new();
                    let (mut lost, mut judged, mut lost_scoped) = (0usize, 0usize, 0usize);

                    for (index, score) in labelled_scores.iter().enumerate() {
                        // Judged against the floor in force *before* this query joined the window.
                        if let Some(level) = floor {
                            judged += 1;
                            if *score < level {
                                lost += 1;
                                if labelled[index].2 == "scoped" {
                                    lost_scoped += 1;
                                }
                            }
                        }
                        held.push(*score);
                        if held.len() > window {
                            held.remove(0);
                        }
                        since += 1;
                        if since >= trigger {
                            let level = percentile(&mut held.clone(), pct) - margin;
                            floor = Some(level);
                            levels.push(level);
                            calibrations += 1;
                            since = 0;
                        }
                    }

                    let Some(final_level) = floor else { continue };
                    let low = levels.iter().copied().fold(f64::INFINITY, f64::min);
                    let high = levels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let cut = probe_scores.iter().filter(|s| **s < final_level).count();
                    let name = if window == usize::MAX {
                        "all".to_owned()
                    } else {
                        window.to_string()
                    };
                    println!(
                        "  {:<7} {:>4.2} {:>8} {:>7.2} {:>6} {:>6.3}-{:<6.3} {:>17.3} {:>9} / {:<3} {:>11.1}%",
                        name,
                        pct,
                        trigger,
                        margin,
                        calibrations,
                        low,
                        high,
                        final_level,
                        lost,
                        judged,
                        share(cut, probe_scores.len()),
                    );
                    let _ = lost_scoped;
                }
            }
        }
    }
    println!("\n  `later lost` counts labelled queries the floor in force silenced, over the");
    println!("  labelled queries judged by any floor at all. With no floor both columns are zero.");
}

/// What each live set amounts to, as `derived-set-summary` would report it at export.
///
/// This is the probe the design rests on: the labelling scheme was chosen on 2026-09-12 by reading
/// this output for `scatterhoard` and `coding-lessons` and judging it legible, against two rival
/// schemes that were not. Re-run it as the corpus grows — the question it answers is whether a
/// reader could still tell what these sets are about, and no assertion can hold that.
///
/// Needs no embedding model: it reads the vectors the stores already hold.
#[test]
#[ignore = "measurement against this machine's live sets; run by hand"]
fn probe_what_each_set_amounts_to() {
    use scatterhoard::summary::{Subject, clusters_for, summarise};

    let scratch = tempdir();
    for (scope, path) in copied_stores(scratch.path()) {
        let db = rusqlite::Connection::open(&path).expect("open a copied store");

        let mut keywords: HashMap<String, Vec<scatterhoard::memory::Keyword>> = HashMap::new();
        let mut statement = db
            .prepare("SELECT memory_id, keyword FROM keywords")
            .expect("query keywords");
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("read keywords");
        for row in rows.flatten() {
            if let Ok(keyword) = scatterhoard::memory::Keyword::new(row.1) {
                keywords.entry(row.0).or_default().push(keyword);
            }
        }

        let mut subjects: Vec<Subject> = Vec::new();
        let mut statement = db
            .prepare("SELECT memory_id, vector FROM vectors")
            .expect("query vectors");
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .expect("read vectors");
        for row in rows.flatten() {
            let components: Vec<f32> = row
                .1
                .chunks_exact(4)
                .map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]]))
                .collect();
            let Ok(vector) = Vector::new(components) else {
                continue;
            };
            subjects.push(Subject::new(
                &MemoryId::parse(&row.0).expect("a stored id parses"),
                vector,
                keywords.get(&row.0).cloned().unwrap_or_default(),
            ));
        }

        let total = subjects.len();
        println!(
            "\n### {} — {total} memories, k={}\n",
            scope.as_key(),
            clusters_for(total)
        );
        for label in summarise(&subjects, 6) {
            let names: Vec<&str> = label
                .keywords
                .iter()
                .map(scatterhoard::memory::Keyword::as_str)
                .collect();
            #[allow(clippy::cast_precision_loss)]
            let share = 100.0 * label.size as f64 / total.max(1) as f64;
            println!(
                "{:4} ({:4.1}%)  {}",
                label.size,
                share,
                if names.is_empty() {
                    "(nothing distinctive)".to_owned()
                } else {
                    names.join(", ")
                }
            );
        }
    }
}

/// How far a set's derived description survives a one-memory change, for the seeding that ships and
/// for the candidates `a-set-description-reshuffles-on-a-small-change` is weighing.
///
/// For each variant the whole set is partitioned once, then again with one memory removed at eight
/// spread positions, and each smaller partition is compared with the whole one over the memories
/// both hold. **Adjusted Rand** says how much of the grouping survived, with 1.0 identical and 0.0 no
/// better than chance; **labels kept** is, per cluster, the most of its six label keywords any cluster
/// of the smaller partition shares, averaged; **cohesion** is the mean cosine of a memory to its own
/// centre, so a more stable variant cannot win by grouping worse.
///
/// The k-means here is a copy, so the first line per set checks it reproduces the cluster sizes
/// `summary::cluster` returns. A mismatch means the comparison is about a different algorithm.
///
/// Run in release: `cargo test --release --test probes probe_how_stable -- --ignored --nocapture`.
#[test]
#[ignore = "measurement against this machine's live sets; run by hand"]
fn probe_how_stable_a_set_description_is() {
    use scatterhoard::memory::Keyword;
    use scatterhoard::summary::{Cluster, Subject, cluster, clusters_for, label_clusters};

    #[derive(Clone, Copy)]
    enum Seeding {
        /// `k-means++` drawn by position from a constant `splitmix64` stream: `clusters/1`.
        Position,
        /// `k-means++` drawn by a key hashed from each memory's own vector.
        VectorKey,
        /// `k-means++` drawn by a key hashed from each memory's id, which no floating-point
        /// difference between machines can move: `clusters/2`, with ten restarts.
        IdKey,
    }

    struct Variant {
        name: &'static str,
        seeding: Seeding,
        restarts: u64,
        refinements: usize,
    }

    fn splitmix(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[allow(clippy::cast_precision_loss)]
    fn unit(bits: u64) -> f64 {
        (bits >> 11) as f64 / (1u64 << 53) as f64
    }

    fn fnv(vector: &[f32], step: u64, restart: u64) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |byte: u8| {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        };
        for value in vector {
            value.to_le_bytes().into_iter().for_each(&mut feed);
        }
        step.to_le_bytes().into_iter().for_each(&mut feed);
        restart.to_le_bytes().into_iter().for_each(&mut feed);
        hash
    }

    fn fnv_id(id: &str, step: u64, restart: u64) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let bytes = id
            .bytes()
            .chain(step.to_le_bytes())
            .chain(restart.to_le_bytes());
        for byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash
    }

    fn nearest(vector: &[f32], centres: &[Vec<f32>]) -> usize {
        let mut best = (f32::NEG_INFINITY, 0usize);
        for (index, centre) in centres.iter().enumerate() {
            let similarity = embedding::cosine(vector, centre);
            if similarity > best.0 {
                best = (similarity, index);
            }
        }
        best.1
    }

    fn seeds(
        vectors: &[Vec<f32>],
        ids: &[String],
        parts: usize,
        seeding: Seeding,
        restart: u64,
    ) -> Vec<usize> {
        let mut mean = vec![0.0f32; vectors[0].len()];
        for vector in vectors {
            for (slot, value) in mean.iter_mut().zip(vector) {
                *slot += *value;
            }
        }
        let mut chosen = vec![nearest(&mean, vectors)];
        let mut state = 0x9E37_79B9_7F4A_7C15u64.wrapping_add(restart);
        while chosen.len() < parts {
            let weights: Vec<f64> = vectors
                .iter()
                .map(|vector| {
                    let closest = chosen
                        .iter()
                        .map(|seed| embedding::cosine(vector, &vectors[*seed]))
                        .fold(f32::NEG_INFINITY, f32::max);
                    let distance = f64::from(1.0 - closest).max(0.0);
                    distance * distance
                })
                .collect();
            let pick = match seeding {
                Seeding::Position => {
                    let total: f64 = weights.iter().sum();
                    if total <= 0.0 {
                        break;
                    }
                    let mut target = unit(splitmix(&mut state)) * total;
                    let mut found = None;
                    for (index, weight) in weights.iter().enumerate() {
                        target -= weight;
                        if target <= 0.0 && *weight > 0.0 {
                            found = Some(index);
                            break;
                        }
                    }
                    found.or_else(|| weights.iter().position(|weight| *weight > 0.0))
                }
                // Weighted sampling by key: the memory maximising `ln(u) / w` is drawn with
                // probability proportional to `w`, and `u` belongs to the memory rather than to its
                // position, so inserting another memory cannot move anyone else's draw.
                Seeding::VectorKey | Seeding::IdKey => weights
                    .iter()
                    .enumerate()
                    .filter(|(_, weight)| **weight > 0.0)
                    .map(|(index, weight)| {
                        let step = chosen.len() as u64;
                        let bits = match seeding {
                            Seeding::IdKey => fnv_id(&ids[index], step, restart),
                            _ => fnv(&vectors[index], step, restart),
                        };
                        let draw = unit(bits).max(f64::MIN_POSITIVE);
                        (draw.ln() / weight, index)
                    })
                    .max_by(|left, right| left.0.total_cmp(&right.0).then(right.1.cmp(&left.1)))
                    .map(|(_, index)| index),
            };
            match pick {
                Some(index) => chosen.push(index),
                None => break,
            }
        }
        chosen
    }

    /// One run: the assignment, its cohesion, and how many refinement passes it took.
    fn kmeans(
        vectors: &[Vec<f32>],
        ids: &[String],
        parts: usize,
        variant: &Variant,
        restart: u64,
    ) -> (Vec<usize>, f64, usize) {
        let mut centres: Vec<Vec<f32>> = seeds(vectors, ids, parts, variant.seeding, restart)
            .into_iter()
            .map(|index| vectors[index].clone())
            .collect();
        let mut assignment = vec![0usize; vectors.len()];
        let mut passes = 0;
        for _ in 0..variant.refinements {
            passes += 1;
            let mut moved = false;
            for (index, vector) in vectors.iter().enumerate() {
                let to = nearest(vector, &centres);
                if assignment[index] != to {
                    assignment[index] = to;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
            let mut sums = vec![vec![0.0f32; vectors[0].len()]; centres.len()];
            let mut counts = vec![0usize; centres.len()];
            for (index, vector) in vectors.iter().enumerate() {
                counts[assignment[index]] += 1;
                for (slot, value) in sums[assignment[index]].iter_mut().zip(vector) {
                    *slot += *value;
                }
            }
            for (part, centre) in centres.iter_mut().enumerate() {
                if counts[part] > 0 {
                    #[allow(clippy::cast_precision_loss)]
                    let divisor = counts[part] as f32;
                    for (slot, total) in centre.iter_mut().zip(&sums[part]) {
                        *slot = total / divisor;
                    }
                }
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let cohesion = vectors
            .iter()
            .enumerate()
            .map(|(index, vector)| {
                f64::from(embedding::cosine(vector, &centres[assignment[index]]))
            })
            .sum::<f64>()
            / vectors.len() as f64;
        (assignment, cohesion, passes)
    }

    fn best_of(
        vectors: &[Vec<f32>],
        ids: &[String],
        parts: usize,
        variant: &Variant,
    ) -> (Vec<usize>, f64, usize) {
        (0..variant.restarts)
            .map(|restart| kmeans(vectors, ids, parts, variant, restart))
            .fold(
                None,
                |best: Option<(Vec<usize>, f64, usize)>, run| match best {
                    Some(held) if held.1 >= run.1 => Some(held),
                    _ => Some(run),
                },
            )
            .expect("at least one restart")
    }

    #[allow(clippy::cast_precision_loss)]
    fn adjusted_rand(left: &[usize], right: &[usize]) -> f64 {
        let pairs = |n: f64| n * (n - 1.0) / 2.0;
        let mut table: HashMap<(usize, usize), f64> = HashMap::new();
        let mut rows: HashMap<usize, f64> = HashMap::new();
        let mut columns: HashMap<usize, f64> = HashMap::new();
        for (a, b) in left.iter().zip(right) {
            *table.entry((*a, *b)).or_default() += 1.0;
            *rows.entry(*a).or_default() += 1.0;
            *columns.entry(*b).or_default() += 1.0;
        }
        let index: f64 = table.values().map(|n| pairs(*n)).sum();
        let row_sum: f64 = rows.values().map(|n| pairs(*n)).sum();
        let column_sum: f64 = columns.values().map(|n| pairs(*n)).sum();
        let expected = row_sum * column_sum / pairs(left.len() as f64);
        let maximum = (row_sum + column_sum) / 2.0;
        (index - expected) / (maximum - expected)
    }

    fn labels(assignment: &[usize], keywords: &[Vec<Keyword>], parts: usize) -> Vec<Vec<String>> {
        let mut members: Vec<Vec<Vec<Keyword>>> = vec![Vec::new(); parts];
        for (index, part) in assignment.iter().enumerate() {
            members[*part].push(keywords[index].clone());
        }
        let clusters: Vec<Cluster> = members
            .into_iter()
            .filter(|group| !group.is_empty())
            .map(Cluster::new)
            .collect();
        label_clusters(&clusters, 6)
            .into_iter()
            .map(|label| {
                label
                    .keywords
                    .iter()
                    .map(|k| k.as_str().to_owned())
                    .collect()
            })
            .collect()
    }

    #[allow(clippy::cast_precision_loss)]
    fn labels_kept(whole: &[Vec<String>], smaller: &[Vec<String>]) -> f64 {
        whole
            .iter()
            .map(|names| {
                smaller
                    .iter()
                    .map(|other| names.iter().filter(|name| other.contains(name)).count())
                    .max()
                    .unwrap_or(0) as f64
            })
            .sum::<f64>()
            / whole.len().max(1) as f64
    }

    let variants = [
        Variant {
            name: "clusters/1 (25 passes)",
            seeding: Seeding::Position,
            restarts: 1,
            refinements: 25,
        },
        Variant {
            name: "clusters/1, run to settle",
            seeding: Seeding::Position,
            restarts: 1,
            refinements: 300,
        },
        Variant {
            name: "by index, best of 10",
            seeding: Seeding::Position,
            restarts: 10,
            refinements: 300,
        },
        Variant {
            name: "by key",
            seeding: Seeding::VectorKey,
            restarts: 1,
            refinements: 300,
        },
        Variant {
            name: "by key, best of 10",
            seeding: Seeding::VectorKey,
            restarts: 10,
            refinements: 300,
        },
        Variant {
            name: "by id",
            seeding: Seeding::IdKey,
            restarts: 1,
            refinements: 300,
        },
        Variant {
            name: "by id, best of 10",
            seeding: Seeding::IdKey,
            restarts: 10,
            refinements: 300,
        },
    ];
    const TRIALS: usize = 8;

    let scratch = tempdir();
    for (scope, path) in copied_stores(scratch.path()) {
        let key = scope.as_key();
        if key != "shared:scatterhoard" && key != "shared:coding-lessons" {
            continue;
        }
        let db = rusqlite::Connection::open(&path).expect("open a copied store");

        let mut keyed: HashMap<String, Vec<Keyword>> = HashMap::new();
        let mut statement = db
            .prepare("SELECT memory_id, keyword FROM keywords ORDER BY memory_id, keyword")
            .expect("query keywords");
        for row in statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("read keywords")
            .flatten()
        {
            if let Ok(keyword) = Keyword::new(row.1) {
                keyed.entry(row.0).or_default().push(keyword);
            }
        }

        let mut vectors: Vec<Vec<f32>> = Vec::new();
        let mut ids: Vec<String> = Vec::new();
        let mut keywords: Vec<Vec<Keyword>> = Vec::new();
        let mut subjects: Vec<Subject> = Vec::new();
        let mut statement = db
            .prepare("SELECT memory_id, vector FROM vectors ORDER BY memory_id")
            .expect("query vectors");
        for row in statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .expect("read vectors")
            .flatten()
        {
            let components: Vec<f32> = row
                .1
                .chunks_exact(4)
                .map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]]))
                .collect();
            let Ok(vector) = Vector::new(components) else {
                continue;
            };
            let words = keyed.get(&row.0).cloned().unwrap_or_default();
            ids.push(row.0.clone());
            vectors.push(vector.components().to_vec());
            keywords.push(words.clone());
            subjects.push(Subject::new(
                &MemoryId::parse(&row.0).expect("a stored id parses"),
                vector,
                words,
            ));
        }

        let n = vectors.len();
        let parts = clusters_for(n);
        println!("\n### {key} — {n} memories, k={parts}\n");

        let mut shipped: Vec<usize> = cluster(&subjects, parts)
            .iter()
            .map(Cluster::size)
            .collect();
        shipped.sort_unstable_by(|a, b| b.cmp(a));
        // Production has shipped `by id, best of 10` since 2026-09-14 (`clusters/2`); the copy of
        // that variant is what has to reproduce it. The other rows are kept as the comparison that
        // chose it.
        let shipping = variants
            .iter()
            .find(|variant| variant.name == "by id, best of 10")
            .expect("the shipped variant is listed");
        let (replica, _, _) = best_of(&vectors, &ids, parts, shipping);
        let mut sizes = vec![0usize; parts];
        replica.iter().for_each(|part| sizes[*part] += 1);
        sizes.retain(|size| *size > 0);
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        println!(
            "fidelity: shipped {shipped:?}, copy {sizes:?} -- {}\n",
            if shipped == sizes {
                "match"
            } else {
                "MISMATCH, ignore the table"
            }
        );

        println!(
            "{:28} {:>9} {:>7} {:>9} {:>9} {:>8}",
            "variant", "cohesion", "passes", "ARI mean", "ARI min", "labels"
        );
        for variant in &variants {
            let (whole, cohesion, passes) = best_of(&vectors, &ids, parts, variant);
            let whole_labels = labels(&whole, &keywords, parts);
            let mut rands = Vec::new();
            let mut kept = Vec::new();
            for trial in 0..TRIALS {
                let removed = (n * trial + n / (2 * TRIALS)) / TRIALS;
                let fewer: Vec<Vec<f32>> = vectors
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != removed)
                    .map(|(_, vector)| vector.clone())
                    .collect();
                let fewer_keywords: Vec<Vec<Keyword>> = keywords
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != removed)
                    .map(|(_, words)| words.clone())
                    .collect();
                let fewer_ids: Vec<String> = ids
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != removed)
                    .map(|(_, id)| id.clone())
                    .collect();
                let (smaller, _, _) = best_of(&fewer, &fewer_ids, parts, variant);
                let common: Vec<usize> = whole
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != removed)
                    .map(|(_, part)| *part)
                    .collect();
                rands.push(adjusted_rand(&common, &smaller));
                kept.push(labels_kept(
                    &whole_labels,
                    &labels(&smaller, &fewer_keywords, parts),
                ));
            }
            #[allow(clippy::cast_precision_loss)]
            let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len() as f64;
            println!(
                "{:28} {:>9.4} {:>7} {:>9.3} {:>9.3} {:>6.2}/6",
                variant.name,
                cohesion,
                passes,
                mean(&rands),
                rands.iter().copied().fold(f64::INFINITY, f64::min),
                mean(&kept)
            );
        }
    }
}

/// **How long a review costs, and how much of that is the model.** `hoard_sets evaluate` embeds every
/// memory in the file before it returns, so a large file is a tool call that looks like a hang. The
/// limit in `evaluation-load-size-limit` has to be a measurement, and the two things it varies with
/// are the machine and the model. The machine needs hardware nobody here has; the model is two
/// directories away, so this probe reports one of the two.
///
/// Run it once per model:
///
/// ```text
/// SCATTERHOARD_MODEL=<path to potion-base-32M> \
///   cargo test --test probes probe_what_a_review_costs -- --ignored --nocapture
/// SCATTERHOARD_MODEL=<path to potion-multilingual-128M> \
///   cargo test --test probes probe_what_a_review_costs -- --ignored --nocapture
/// ```
#[test]
#[ignore = "times a real evaluation load and needs SCATTERHOARD_MODEL"]
fn probe_what_a_review_costs() {
    let Some(embedder) = model() else {
        println!("REVIEW COST no model: set SCATTERHOARD_MODEL");
        return;
    };
    let home = tempdir();
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.set_embedder(embedder.clone());

    let started = std::time::Instant::now();
    let response = server
        .handle(
            &serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "hoard_sets", "arguments": {
                    "action": "evaluate", "label": "cost-probe", "set": "scatterhoard", "directory": repository,
                }},
            })
            .to_string(),
        )
        .expect("a request with an id is answered");
    let elapsed = started.elapsed();

    let payload: serde_json::Value = serde_json::from_str(&response).expect("JSON");
    let text = payload["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    let result: serde_json::Value = serde_json::from_str(text).expect("a tool payload");
    let Some(memories) = result["memories"].as_u64() else {
        println!("REVIEW COST refused: {text}");
        return;
    };

    let per = elapsed.as_secs_f64() / memories as f64;
    println!(
        "REVIEW COST model={} memories={memories} total={:.2}s per_memory={:.1}ms \
         projected_10k={:.0}s",
        embedder.identity().as_str(),
        elapsed.as_secs_f64(),
        per * 1000.0,
        per * 10_000.0,
    );
}

/// Can a character cap stand in for a token cap, and what does a language the model does not
/// cover look like from either side?
///
/// `memory-size-drift` asks for a hard limit that refuses an over-length memory when it is stored,
/// and the unit is open. A token count is what the embedding actually spends, but tokenising needs
/// the model, and Scatterhoard runs without one — the keyword path is the no-model path, and a
/// store that refused by tokens could not answer there at all. So the question is how well a
/// character count proxies the token count it stands for, and the answer is the width of the band
/// where the two disagree.
///
/// The tokeniser is this file's own WordPiece reimplementation over the model's vocabulary, the
/// same one `probe_whether_long_memories_are_truncated_before_embedding` counts with, so these
/// figures are comparable with those and not with another tokeniser's. A word the vocabulary does
/// not hold becomes one `[UNK]`, which `model2vec-rs` then *removes* before pooling — so tokens
/// that survive, rather than tokens produced, is what the vector is built from.
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_a_character_cap_can_stand_in_for_a_token_cap() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let Ok(model_dir) = std::env::var("SCATTERHOARD_MODEL") else {
        return;
    };
    let tokenizer_text = std::fs::read_to_string(Path::new(&model_dir).join("tokenizer.json"))
        .expect("read tokenizer.json");
    let tokenizer: serde_json::Value =
        serde_json::from_str(&tokenizer_text).expect("parse tokenizer.json");
    let vocab: std::collections::HashSet<String> = tokenizer["model"]["vocab"]
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();

    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let sources = sources(&copies);

    let mut measured: Vec<(usize, usize)> = Vec::new(); // (chars, tokens)
    for source in &sources {
        let whole =
            Composition::Live.input(&source.content, source.why.as_deref(), &source.keywords);
        measured.push((whole.chars().count(), tokenize(&vocab, &whole).len()));
    }

    println!("\n== Characters per token, over what the embedding actually receives ==\n");
    #[allow(clippy::cast_precision_loss)]
    let mut ratios: Vec<f64> = measured
        .iter()
        .filter(|(_, t)| *t > 0)
        .map(|(c, t)| *c as f64 / *t as f64)
        .collect();
    println!(
        "  {} memories: median {:.2} chars/token, p10 {:.2}, p90 {:.2}, max {:.2}",
        measured.len(),
        percentile(&mut ratios.clone(), 0.50),
        percentile(&mut ratios.clone(), 0.10),
        percentile(&mut ratios.clone(), 0.90),
        percentile(&mut ratios, 1.00)
    );

    println!("\n== Where a character cap and a 512-token cap disagree ==\n");
    let over: Vec<usize> = measured
        .iter()
        .filter(|(_, t)| *t > 512)
        .map(|(c, _)| *c)
        .collect();
    let under: Vec<usize> = measured
        .iter()
        .filter(|(_, t)| *t <= 512)
        .map(|(c, _)| *c)
        .collect();
    match (over.iter().min(), under.iter().max()) {
        (Some(&lowest_over), Some(&highest_under)) => {
            println!("  shortest memory that is over 512 tokens: {lowest_over} chars");
            println!("  longest memory that is under it:         {highest_under} chars");
            if lowest_over > highest_under {
                println!("  the two do not overlap: a character cap between them is exact.");
            } else {
                let band = measured
                    .iter()
                    .filter(|(c, _)| *c >= lowest_over && *c <= highest_under)
                    .count();
                #[allow(clippy::cast_precision_loss)]
                let share = 100.0 * band as f64 / measured.len() as f64;
                println!(
                    "  they overlap over {} chars, holding {band} memories ({share:.1}%), where \
                     no character cap can be exact.",
                    highest_under - lowest_over
                );
            }
        }
        _ => println!("  one side is empty -- nothing to separate."),
    }

    println!("\n  cap (chars)   refuses   of those, under 512 tokens   lets through over 512");
    for cap in [2000usize, 2500, 3000, 3584, 4000] {
        let refused = measured.iter().filter(|(c, _)| *c > cap).count();
        let needless = measured
            .iter()
            .filter(|(c, t)| *c > cap && *t <= 512)
            .count();
        let missed = measured
            .iter()
            .filter(|(c, t)| *c <= cap && *t > 512)
            .count();
        println!("  {cap:<13} {refused:<9} {needless:<28} {missed}");
    }

    println!("\n== A language the model does not cover ==\n");
    println!("  sample                     chars  tokens  surviving  embeds?");
    let samples: [(&str, &str); 4] = [
        (
            "english",
            "the memory set records a decision about how recall is scoped",
        ),
        (
            "russian",
            "набор памяти записывает решение о том как ограничивается поиск",
        ),
        (
            "japanese",
            "記憶セットは想起の範囲をどう決めるかという決定を記録する",
        ),
        (
            "greek",
            "το σύνολο μνήμης καταγράφει μια απόφαση για το πώς οριοθετείται η ανάκληση",
        ),
    ];
    for (label, text) in samples {
        let tokens = tokenize(&vocab, text);
        let surviving = tokens.iter().filter(|t| *t != "[UNK]").count();
        // The norm, not merely whether it embedded: text the vocabulary cannot reach becomes
        // `[UNK]`, which `model2vec-rs` removes before pooling, so the interesting outcome is a
        // vector pooled from nothing that is still returned as a vector.
        let embeds = match embedder.embed(&[text.to_string()]) {
            Ok(vectors) => vectors.first().map_or_else(
                || "no vector".to_string(),
                |vector| {
                    let norm: f32 = vector
                        .components()
                        .iter()
                        .map(|component| component * component)
                        .sum::<f32>()
                        .sqrt();
                    let zeros = vector
                        .components()
                        .iter()
                        .filter(|component| **component == 0.0)
                        .count();
                    format!(
                        "norm {norm:.4}, {zeros}/{} components zero",
                        vector.components().len()
                    )
                },
            ),
            Err(error) => format!("refused: {error}"),
        };
        println!(
            "  {label:<24} {:6} {:7} {surviving:10}  {embeds}",
            text.chars().count(),
            tokens.len()
        );
    }

    // A unit vector is not evidence that the text reached the model: pooling normalises whatever
    // it is given. The question is whether two *different* texts in one language separate, which
    // is what recall needs and what an out-of-vocabulary text cannot deliver however well formed
    // its vector is.
    // A unit vector is not evidence that the text reached the model: pooling normalises whatever
    // it is given. What recall needs is that unrelated texts point in different directions, so the
    // measure is the mean pairwise cosine among four unrelated sentences per language. Near zero is
    // separation; near one means every text in that language lands in one place and recall within
    // it cannot rank.
    println!("\n  mean pairwise cosine among four unrelated texts, by language\n");
    let corpora: [(&str, [&str; 4]); 4] = [
        (
            "english",
            [
                "the memory set records a decision about how recall is scoped",
                "a badger crossed the frozen river before dawn and vanished",
                "bread dough needs a long cold rise to develop its flavour",
                "the harbour lights failed during the storm last November",
            ],
        ),
        (
            "russian",
            [
                "набор памяти записывает решение о том как ограничивается поиск",
                "барсук перешёл замёрзшую реку перед рассветом и исчез",
                "хлебному тесту нужен долгий холодный подъём для вкуса",
                "огни гавани погасли во время шторма в прошлом ноябре",
            ],
        ),
        (
            "japanese",
            [
                "記憶セットは想起の範囲をどう決めるかという決定を記録する",
                "アナグマは夜明け前に凍った川を渡って消えた",
                "パン生地は風味を出すために長く冷たい発酵が必要だ",
                "港の灯りは去年の十一月の嵐の間に消えた",
            ],
        ),
        (
            "greek",
            [
                "το σύνολο μνήμης καταγράφει μια απόφαση για το πώς οριοθετείται η ανάκληση",
                "ένας ασβός πέρασε το παγωμένο ποτάμι πριν την αυγή και εξαφανίστηκε",
                "η ζύμη του ψωμιού χρειάζεται αργή ψυχρή ωρίμανση για τη γεύση της",
                "τα φώτα του λιμανιού έσβησαν στη διάρκεια της καταιγίδας τον περασμένο Νοέμβριο",
            ],
        ),
    ];
    for (label, texts) in corpora {
        let vectors: Vec<_> = texts
            .iter()
            .map(|text| {
                embedder
                    .embed(&[(*text).to_string()])
                    .expect("embed")
                    .remove(0)
            })
            .collect();
        let mut similarities = Vec::new();
        for (index, first) in vectors.iter().enumerate() {
            for second in vectors.iter().skip(index + 1) {
                similarities.push(cosine_of(first.components(), second.components()));
            }
        }
        let mean_similarity = mean(&similarities);
        let worst = similarities.iter().copied().fold(f64::MIN, f64::max);
        println!(
            "  {label:<12} mean {mean_similarity:>7.3}   highest pair {worst:>6.3}{}",
            if mean_similarity > 0.5 {
                "   <-- unrelated texts do not separate"
            } else {
                ""
            }
        );
    }
}

/// Can a set's floor be placed without labelled queries? A backup for sets too little used to
/// accumulate the twenty labels a calibration needs, tested against the two sets that have them.
///
/// `explicit-calibration-pass`, 2026-09-24: `governance-docs` and `local` were still uncalibrated
/// twelve days after the floor shipped, and calibrating the busy sets made an unanswerable query fall
/// through to them. Two sources of evidence need no session and no labels:
///
/// - **B, negatives from real traffic.** Every unscoped query a set was scored against, contributed
///   nothing to and was not used afterwards. Real questions, none authored -- which is what the
///   2026-09-09 objection to negative-side probes was about.
/// - **C, positives from the set itself.** Each memory's index line asked as a query against the rest
///   of its set, leaving the memory out, so the best score is what the set gives a question it can
///   answer from somewhere else.
///
/// The check is the two calibrated sets: run B and C as though their labels did not exist, and see
/// where the proposed floors land beside the one real evidence produced, and what each would have
/// silenced among the real labelled queries.
///
/// ```text
/// SCATTERHOARD_MODEL=... cargo test --test probes probe_a_floor_without_labelled_queries -- --ignored --nocapture
/// ```
#[test]
#[ignore = "reads this machine's live sets and needs SCATTERHOARD_MODEL"]
fn probe_a_floor_without_labelled_queries() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };

    fn percentile(values: &[f64], p: f64) -> f64 {
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        if sorted.is_empty() {
            return f64::NAN;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let index = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        sorted[index]
    }
    fn below(values: &[f64], floor: f64) -> usize {
        values.iter().filter(|v| **v < floor).count()
    }

    const MARGIN: f64 = 0.05;
    let scratch = tempdir();
    for (scope, path) in copied_stores(scratch.path()) {
        let store = Store::open_at(scope.clone(), &path).expect("open");
        let log = store.query_log().expect("read the log");
        let labelled: Vec<f64> = log
            .iter()
            .filter(|q| q.scoped || q.used)
            .map(|q| f64::from(q.raw_top))
            .collect();
        let negatives: Vec<f64> = log
            .iter()
            .filter(|q| !q.scoped && !q.used && q.returned.is_empty())
            .map(|q| f64::from(q.raw_top))
            .collect();

        // Current claims only. A superseded memory's index line all but repeats its replacement's,
        // which would score every chain as a near-perfect answer; and an identity record is a name,
        // not a claim, so nothing is meant to answer to it.
        let memories: Vec<_> = store
            .memory_ids()
            .expect("ids")
            .into_iter()
            .filter(|id| {
                matches!(
                    store.resolve_supersession(*id).expect("resolves"),
                    scatterhoard::store::Resolution::Current
                )
            })
            .filter_map(|id| store.get(id).expect("get"))
            .filter(|m| {
                !m.keywords
                    .iter()
                    .any(|k| k.as_str() == scatterhoard::store::AUTHOR_KEYWORD)
            })
            .collect();
        let inputs: Vec<String> = memories.iter().map(embedding::embedding_input).collect();
        let lines: Vec<String> = memories
            .iter()
            .map(|m| m.content.summary_as_data())
            .collect();
        let bodies = embedder.embed_each(&inputs).expect("embed memories");
        let questions = embedder.embed_each(&lines).expect("embed index lines");
        let mut positives: Vec<f64> = Vec::new();
        for (i, question) in questions.iter().enumerate() {
            let Embedded::Vector(question) = question else {
                continue;
            };
            let best = bodies
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .filter_map(|(_, body)| match body {
                    Embedded::Vector(body) => {
                        Some(cosine_of(question.components(), body.components()))
                    }
                    Embedded::Unrepresentable => None,
                })
                .fold(f64::NEG_INFINITY, f64::max);
            if best.is_finite() {
                positives.push(best);
            }
        }

        let real = store.floor().expect("floor");
        let from_labels = percentile(&labelled, 5.0) - MARGIN;
        let from_negatives = negatives.iter().copied().fold(f64::NEG_INFINITY, f64::max) + 0.01;
        let from_positives = percentile(&positives, 5.0) - MARGIN;

        println!(
            "\n  {} -- {} memories, {} logged queries: {} labelled, {} negatives",
            scope.as_key(),
            memories.len(),
            log.len(),
            labelled.len(),
            negatives.len()
        );
        println!(
            "    labelled  p5 {:.3}  median {:.3}  |  negatives max {:.3}  p90 {:.3}  |  \
             self-queries p5 {:.3}  median {:.3}",
            percentile(&labelled, 5.0),
            percentile(&labelled, 50.0),
            negatives.iter().copied().fold(f64::NAN, f64::max),
            percentile(&negatives, 90.0),
            percentile(&positives, 5.0),
            percentile(&positives, 50.0),
        );
        println!(
            "    floor: stored {}  |  from labels {:.3}",
            real.map_or("none".to_owned(), |f| format!("{f:.3}")),
            from_labels
        );
        for (name, floor) in [
            ("B  above every negative", from_negatives),
            ("C  under the self-queries", from_positives),
            ("B+C lower of the two", from_negatives.min(from_positives)),
        ] {
            println!(
                "    {name:<26} {floor:.3}  silences {}/{} labelled, {}/{} negatives",
                below(&labelled, floor),
                labelled.len(),
                below(&negatives, floor),
                negatives.len(),
            );
        }
    }
}

/// The ranking 50 deep for one query, without the floor, as recall scores it.
fn ranking_without_the_floor(store: &mut Store, query: &str, depth: usize) -> Vec<f64> {
    let mut scores: Vec<f64> = Vec::new();
    let mut known: Vec<_> = Vec::new();
    while scores.len() < depth {
        let mut refs = vec![&mut *store];
        let question = Question::new(query, depth)
            .already_holding(&known)
            .without_the_floor();
        let recall = recall_across(&mut refs, &question).expect("recall");
        if recall.hits.is_empty() {
            break;
        }
        for hit in &recall.hits {
            scores.push(traversal::combine(
                f64::from(hit.similarity),
                f64::from(hit.inherited),
            ));
            known.push(hit.memory.id);
        }
    }
    scores.truncate(depth);
    scores
}

/// **Does a ranking break anywhere, on real questions, now the sets are larger?** Re-asked
/// 2026-10-02 before removing `cut_at_largest_gap`, which the 2026-09-08 probes above found to be
/// choosing among noise on nine hand-picked queries. This asks it of every recall query a session
/// is recorded as having asked of each set (`corpus/labelled-queries-*.json`) against 100 of the
/// off-domain probes as a null, and reports two things: the largest gap over the median gap in the
/// top 20, and how many results the cut returns at windows of 10, 20 and 50, which is how often the
/// answer moves with the budget (`result-count-follows-the-limit-not-the-query`).
#[test]
#[ignore = "reads this machine's live memory sets and needs SCATTERHOARD_MODEL"]
fn probe_whether_real_queries_break_and_whether_the_cut_follows_the_window() {
    let Some(embedder) = model() else {
        println!(
            "SCATTERHOARD_MODEL unset or unloadable -- nothing measured. This is a skip, not a pass."
        );
        return;
    };
    let scratch = tempdir();
    let copies = copied_stores(scratch.path());
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(root.join(name)).expect("readable"))
            .expect("valid JSON")
    };
    let nulls: Vec<String> = read("corpus/offdomain-probes.json")["questions"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .take(100)
        .map(str::to_owned)
        .collect();

    for (key, file) in [
        (
            "shared:scatterhoard",
            "corpus/labelled-queries-scatterhoard.json",
        ),
        (
            "shared:coding-lessons",
            "corpus/labelled-queries-coding-lessons.json",
        ),
    ] {
        let real: Vec<String> = read(file)["queries"]
            .as_array()
            .expect("a list")
            .iter()
            .filter_map(|q| q["query"].as_str())
            .map(str::to_owned)
            .collect();
        let scope = Scope::from_key(key).expect("a valid scope key");
        let (_, path) = copies
            .iter()
            .find(|(s, _)| *s == scope)
            .expect("a copied store");
        let mut store = Store::open_at(scope.clone(), path).expect("open a copied store");
        store.set_embedder(embedder.clone());

        for (arm, queries) in [("real", &real), ("null", &nulls)] {
            let mut ratios: Vec<f64> = Vec::new();
            let mut moved = 0usize;
            let mut counts: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            for query in queries {
                let scores = ranking_without_the_floor(&mut store, query, 50);
                if scores.len() < 20 {
                    continue;
                }
                let (largest, median, _) = gap_profile(&scores[..20]);
                if median > 0.0 {
                    ratios.push(largest / median);
                }
                let at: Vec<usize> = [10, 20, 50]
                    .iter()
                    .map(|&w| traversal::cut_at_largest_gap(&scores, w))
                    .collect();
                for (slot, n) in counts.iter_mut().zip(&at) {
                    slot.push(*n as f64);
                }
                if at.windows(2).any(|pair| pair[0] != pair[1]) {
                    moved += 1;
                }
            }
            let over = ratios.iter().filter(|r| **r > 10.09).count();
            println!(
                "{key:<22} {arm:<5} n={:>3}  ratio median {:>5.2} p90 {:>5.2} max {:>6.2}  over noise p95 {:>3}  \
                 count moved {:>3}/{:<3}  median count at 10/20/50: {}/{}/{}",
                ratios.len(),
                percentile(&mut ratios, 0.5),
                percentile(&mut ratios, 0.9),
                percentile(&mut ratios, 1.0),
                over,
                moved,
                counts[0].len(),
                percentile(&mut counts[0], 0.5),
                percentile(&mut counts[1], 0.5),
                percentile(&mut counts[2], 0.5),
            );
        }
    }
    println!(
        "\nNoise reference (2026-09-08): 20 unstructured scores give a median ratio of 4.69 and a \
         95th percentile of 10.09."
    );
}
