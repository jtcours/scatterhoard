// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The evaluation harness: measure whether traversal can earn its cost on a corpus.
//!
//! ```text
//! SCATTERHOARD_MODEL=/path/to/model \
//!   cargo run --bin scatterhoard-eval -- corpus/traversal-corpus.jsonl
//! ```
//!
//! # A standalone binary, and no store anywhere
//!
//! An evaluation corpus is test data shaped like memory, and it has no business in a store. This
//! was learned the expensive way: a 56-memory corpus imported as a set to get it embedded took all
//! three top results on an unrelated query, because recall without a scope searches every loaded
//! set. A corpus of several hundred would drown ordinary recall entirely, and there is still no
//! `drop` on the tool surface to undo it.
//!
//! So this reads a memory *file*, embeds it in memory, measures it, and exits. Nothing is opened,
//! created, registered or written.
//!
//! # It measures the real pipeline
//!
//! The spike this replaces reimplemented the embedding input and the cosine, either of which could
//! drift from the product without anything failing. This uses the real
//! [`embedding_input`](scatterhoard::embedding::embedding_input), a real
//! [`Embedder`](scatterhoard::embedder::Embedder), and the product's own centred scan, so what is
//! reported is what recall would actually do.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use scatterhoard::embedder::{Embedder, StaticEmbedder};
use scatterhoard::embedding::{VectorSet, embedding_input};
use scatterhoard::eval::{self, Corpus};
use scatterhoard::memory::MemoryId;
use scatterhoard::memory_file::{self, Expectation};

/// The cut a caller is judged against, and the one every recorded figure uses.
const K: usize = 5;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or_else(|| {
        "usage: scatterhoard-eval <corpus.jsonl> [labels.json] [tune|sweep|grid] \
         [--seeds N] [--hops N] [--decay F] [--frontier N] [--rule NAME] [--k F]\n\nMeasures where \
         each link's target ranks when a query lands exactly on its source. Set SCATTERHOARD_MODEL \
         to the model directory."
            .to_owned()
    })?;

    let embedder = load_embedder()?;
    let file = std::fs::File::open(&path).map_err(|error| format!("{path}: {error}"))?;

    // `AnyShared` rather than a named scope: this reads a file at a path the caller gave, loads
    // nothing, and binds nothing, so there is no trust to escalate into — the same reasoning that
    // makes inspection the one operation permitted to read a header without trusting it.
    let set = memory_file::read(&Expectation::AnyShared, file)
        .map_err(|error| format!("{path}: {error}"))?;

    eprintln!(
        "corpus: {} memories from {path}\nmodel: {} ({} dimensions)",
        set.memories.len(),
        embedder.identity(),
        embedder.dimension()
    );

    let inputs: Vec<String> = set.memories.iter().map(embedding_input).collect();
    let vectors = embedder.embed(&inputs).map_err(|error| error.to_string())?;

    let mut vector_set = VectorSet::new(embedder.dimension());
    for (memory, vector) in set.memories.iter().zip(vectors) {
        vector_set
            .insert(memory.id, vector)
            .map_err(|error| error.to_string())?;
    }

    let corpus = Corpus::new(set.memories, vector_set);

    // With a labels file, the question changes from "how far out are linked targets" to "did the
    // labelling collapse into content similarity". Both read the same corpus.
    match (std::env::args().nth(2), std::env::args().nth(3)) {
        (Some(labels), Some(mode)) if mode == "tune" => {
            tune(&corpus, &embedder, &labels, walk_from_arguments()?)?;
        }
        (Some(labels), Some(mode)) if mode == "grid" => grid(&corpus, &embedder, &labels)?,
        (Some(labels), Some(mode)) if mode == "sweep" => sweep(&corpus, &embedder, &labels)?,
        (Some(labels), _) => diagnose_labels(&corpus, &embedder, &labels)?,
        (None, _) => report(&corpus),
    }
    Ok(())
}

/// Where labelled memories rank by content alone.
///
/// The check a relevance labelling has to pass before anything is built on it. Labels are meant to
/// be an independent answer key, but a labeller reading text may simply be re-finding what the
/// ranker already returns. If every label sits in the top few, the labels carry no information the
/// mechanism does not have, and traversal's distinctive contribution is invisible by construction.
fn diagnose_labels(corpus: &Corpus, embedder: &StaticEmbedder, path: &str) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct Labelled {
        q: String,
        relevant: Vec<String>,
        /// What the labeller judged to be at the edge of relevance. Absent in the first round's
        /// format, which is why it defaults rather than being required.
        #[serde(default)]
        borderline: Vec<String>,
    }
    #[derive(serde::Deserialize)]
    struct Labels {
        queries: Vec<Labelled>,
    }

    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let labels: Labels = serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))?;

    // Relevance shades off rather than stopping, so the answer key has two readings and the honest
    // move is to run both. If a conclusion holds under each, the boundary the labeller had to draw
    // did not decide it.
    let permissive = std::env::args()
        .nth(3)
        .is_some_and(|mode| mode == "permissive");
    println!(
        "\nreading the answer key as: {}",
        if permissive {
            "PERMISSIVE (relevant + borderline)"
        } else {
            "STRICT (relevant only)"
        }
    );

    let texts: Vec<String> = labels.queries.iter().map(|row| row.q.clone()).collect();
    let vectors = embedder.embed(&texts).map_err(|error| error.to_string())?;

    let mut all: Vec<usize> = Vec::new();
    let mut rows: Vec<(String, usize, usize)> = Vec::new();
    for (row, vector) in labels.queries.iter().zip(&vectors) {
        let ids: Vec<MemoryId> = row
            .relevant
            .iter()
            .chain(if permissive {
                row.borderline.iter()
            } else {
                [].iter()
            })
            .filter_map(|id| id.parse::<MemoryId>().ok())
            .collect();
        let ranks = eval::ranks_from_query(corpus, vector, &ids);
        let outside = ranks.iter().filter(|rank| **rank > K).count();
        rows.push((row.q.clone(), ranks.len(), outside));
        all.extend(ranks);
    }

    all.sort_unstable();
    println!("\nlabels: {} over {} queries", all.len(), rows.len());
    if all.is_empty() {
        return Ok(());
    }

    println!("\nrank of a labelled memory by content alone:");
    for (name, q) in [
        ("10th", 0.10),
        ("25th", 0.25),
        ("median", 0.50),
        ("75th", 0.75),
        ("90th", 0.90),
    ] {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation
        )]
        let index = (((all.len() - 1) as f64) * q).round() as usize;
        println!("  {name:>7}  {:>5}", all[index]);
    }

    let inside = all.iter().filter(|rank| **rank <= K).count();
    #[allow(clippy::cast_precision_loss)]
    let share = inside as f64 / all.len() as f64;
    println!(
        "\n{:.0}% of labels already rank inside the top {K} by content alone ({inside} of {}).",
        100.0 * share,
        all.len()
    );
    if share > 0.5 {
        println!(
            "MORE THAN HALF: the labelling largely re-finds what the ranker already returns, so \
             this answer key carries little the mechanism does not have."
        );
    } else {
        println!(
            "The labels reach well past what content similarity returns, so they can distinguish \
             a mechanism that reaches further from one that does not."
        );
    }

    rows.sort_by_key(|row| std::cmp::Reverse(row.2));
    println!("\nqueries whose labels sit furthest outside the cut:");
    for (name, total, outside) in rows.iter().take(8) {
        println!(
            "  {outside:>3} of {total:<3} outside  {}",
            &name[..name.len().min(58)]
        );
    }
    Ok(())
}

/// Compare score-combination rules against a labelled answer key.
///
/// **The question this whole corpus exists for.** A candidate reached by the walk has two scores —
/// the decayed share inherited from its seed, and its own similarity to the query — and how they
/// combine decides which memories survive the cut. `documents/design/recall-mechanisms.md` records
/// the choice as unevidenced, and every weight as held flat until a corpus could license one.
///
/// Read the baseline first. `OwnOnly` is what recall does today, so a rule that does not beat it is
/// a rule that has bought nothing with the walk's cost.
/// One named flag's value, if the caller gave it.
///
/// A hand-rolled scan rather than a dependency: this binary takes six optional flags and is not on
/// anyone's critical path. `clap` is already in the tree for the server, and reaching for it here
/// would buy argument grouping nobody needs.
fn flag(name: &str) -> Option<String> {
    let arguments: Vec<String> = std::env::args().collect();
    arguments
        .iter()
        .position(|argument| argument == name)
        .and_then(|at| arguments.get(at + 1).cloned())
}

/// A flag parsed into whatever it should be, with the failure naming the flag.
fn flag_as<T: std::str::FromStr>(name: &str, fallback: T) -> Result<T, String> {
    match flag(name) {
        None => Ok(fallback),
        Some(value) => value
            .parse()
            .map_err(|_| format!("{name} expects a number, got {value:?}")),
    }
}

/// The walk to measure at, taken from the caller.
///
/// Defaults to the configuration the earlier measurements used, so a run with no flags reproduces
/// what is already recorded rather than quietly measuring something else.
fn walk_from_arguments() -> Result<eval::Walk, String> {
    Ok(eval::Walk {
        seeds: flag_as("--seeds", 5)?,
        hops: flag_as("--hops", 2)?,
        decay: flag_as("--decay", 0.7)?,
        frontier: flag_as("--frontier", 20)?,
    })
}

/// The rules to compare, in a stable order. `--rule` narrows it to one.
fn rules_from_arguments(
    walk: eval::Walk,
) -> Result<Vec<(String, eval::Walk, eval::Combine)>, String> {
    let k: f64 = flag_as("--k", 60.0)?;
    let all: Vec<(String, eval::Walk, eval::Combine)> = vec![
        (
            "baseline (content only)".to_owned(),
            eval::Walk::none(),
            eval::Combine::OwnOnly,
        ),
        ("max / floor".to_owned(), walk, eval::Combine::Max),
        ("sum".to_owned(), walk, eval::Combine::Sum),
        ("blend w=0.25".to_owned(), walk, eval::Combine::Blend(0.25)),
        ("blend w=0.50".to_owned(), walk, eval::Combine::Blend(0.50)),
        ("blend w=0.75".to_owned(), walk, eval::Combine::Blend(0.75)),
        (
            format!("rank fusion k={k:.0}"),
            walk,
            eval::Combine::RankFusion { k },
        ),
    ];

    let Some(wanted) = flag("--rule") else {
        return Ok(all);
    };
    let chosen: Vec<_> = all
        .into_iter()
        .filter(|(name, _, _)| name.starts_with(&wanted) || name == "baseline (content only)")
        .collect();
    if chosen.len() < 2 {
        return Err(format!(
            "--rule {wanted:?} matched nothing; try max, sum, blend or rank"
        ));
    }
    Ok(chosen)
}

/// Every rule at every decay, and then how much each rule *moved*.
///
/// **This answers a different question from `tune`, and it is the one that matters for choosing a
/// rule.** `tune` reports which rule wins at one walk; a rule that wins at the walk it was tuned on
/// and collapses either side of it is a worse choice than one that is merely good everywhere, since
/// the corpus a rule is tuned on is never the corpus it runs on. So this reports each rule's spread
/// across decays beside its best cell — the flat rule is the safe one.
///
/// The score distribution is reported first, because it says which sensitivity this corpus can
/// exercise at all: a rule that compares an inherited level against the gaps between own-scores
/// behaves differently on a packed ranking than on a spread one.
fn grid(corpus: &Corpus, embedder: &StaticEmbedder, path: &str) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct Labelled {
        q: String,
        relevant: Vec<String>,
    }
    #[derive(serde::Deserialize)]
    struct Labels {
        queries: Vec<Labelled>,
    }

    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let labels: Labels = serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))?;
    let texts: Vec<String> = labels.queries.iter().map(|row| row.q.clone()).collect();
    let vectors = embedder.embed(&texts).map_err(|error| error.to_string())?;

    let answers: Vec<(usize, Vec<MemoryId>)> = labels
        .queries
        .iter()
        .enumerate()
        .map(|(index, row)| {
            (
                index,
                row.relevant
                    .iter()
                    .filter_map(|id| id.parse::<MemoryId>().ok())
                    .collect(),
            )
        })
        .filter(|(_, ids): &(usize, Vec<MemoryId>)| !ids.is_empty())
        .collect();

    // What the own-score distribution looks like, averaged over the queries. `mean` near zero says
    // standardising the scores could not change any ranking; `top gap` against `deviation` says how
    // packed the head of a typical ranking is, which is what a level-versus-gap rule is sensitive to.
    let mut means = Vec::new();
    let mut deviations = Vec::new();
    let mut gaps = Vec::new();
    for (query_index, _) in &answers {
        let scores: Vec<f64> = eval::rank_scored(
            corpus,
            &vectors[*query_index],
            &eval::Walk::none(),
            eval::Combine::OwnOnly,
        )
        .into_iter()
        .map(|(_, score)| score)
        .collect();
        if let Some(spread) = eval::Spread::over(&scores) {
            means.push(spread.mean);
            deviations.push(spread.deviation);
            gaps.push(spread.top_gap);
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let average = |values: &[f64]| values.iter().sum::<f64>() / values.len().max(1) as f64;
    println!(
        "\nown-score distribution over {} queries: mean {:+.4}, deviation {:.4}, top gap {:.4}",
        means.len(),
        average(&means),
        average(&deviations),
        average(&gaps)
    );

    let decays = [0.3f64, 0.5, 0.7, 0.9];
    let seeds: usize = flag_as("--seeds", 5)?;
    let hops: usize = flag_as("--hops", 2)?;
    let frontier: usize = flag_as("--frontier", 20)?;
    println!(
        "\n{} queries; {seeds} seeds, {hops} hops, frontier {frontier}; r@10 by rule and decay",
        answers.len()
    );

    print!("{:<22}", "rule");
    for decay in decays {
        print!("{:>9}", format!("d={decay}"));
    }
    println!("{:>9}{:>9}", "best", "spread");

    let template = eval::Walk {
        seeds,
        hops,
        decay: decays[0],
        frontier,
    };
    for (name, _, rule) in rules_from_arguments(template)? {
        let mut cells = Vec::new();
        for decay in decays {
            let walk = eval::Walk { decay, ..template };
            let walk = if matches!(rule, eval::Combine::OwnOnly) {
                eval::Walk::none()
            } else {
                walk
            };
            let mut total = 0.0;
            for (query_index, relevant) in &answers {
                let ranked = eval::rank(corpus, &vectors[*query_index], &walk, rule);
                total += eval::recall_at(&ranked, relevant, 10);
            }
            #[allow(clippy::cast_precision_loss)]
            cells.push(100.0 * total / answers.len() as f64);
        }
        let best = cells.iter().copied().fold(f64::MIN, f64::max);
        let worst = cells.iter().copied().fold(f64::MAX, f64::min);
        print!("{name:<22}");
        for cell in &cells {
            print!("{cell:>8.1}%");
        }
        println!("{:>8.1}%{:>8.1}", best, best - worst);
    }

    println!(
        "\n`spread` is the range each rule covers across those decays. A rule that wins one cell and \
         \ncollapses beside it is a worse choice than one that is merely good everywhere, because the \
         \ncorpus a rule is tuned on is never the corpus it runs on."
    );
    Ok(())
}

fn tune(
    corpus: &Corpus,
    embedder: &StaticEmbedder,
    path: &str,
    walk: eval::Walk,
) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct Labelled {
        q: String,
        relevant: Vec<String>,
    }
    #[derive(serde::Deserialize)]
    struct Labels {
        queries: Vec<Labelled>,
    }

    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let labels: Labels = serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))?;

    let texts: Vec<String> = labels.queries.iter().map(|row| row.q.clone()).collect();
    let vectors = embedder.embed(&texts).map_err(|error| error.to_string())?;

    let answers: Vec<(usize, Vec<MemoryId>)> = labels
        .queries
        .iter()
        .enumerate()
        .map(|(index, row)| {
            (
                index,
                row.relevant
                    .iter()
                    .filter_map(|id| id.parse::<MemoryId>().ok())
                    .collect(),
            )
        })
        .filter(|(_, ids): &(usize, Vec<MemoryId>)| !ids.is_empty())
        .collect();

    println!(
        "\n{} queries with a non-empty answer key; walk: {} seeds, {} hops, decay {}, frontier {}",
        answers.len(),
        walk.seeds,
        walk.hops,
        walk.decay,
        walk.frontier
    );

    let rules = rules_from_arguments(walk)?;

    println!("\n{:<26} {:>9} {:>9} {:>9}", "rule", "r@5", "r@10", "r@20");
    let mut baseline = [0.0f64; 3];
    for (index, (name, walk, rule)) in rules.iter().enumerate() {
        let mut totals = [0.0f64; 3];
        for (query_index, relevant) in &answers {
            let ranked = eval::rank(corpus, &vectors[*query_index], walk, *rule);
            for (slot, k) in [5usize, 10, 20].iter().enumerate() {
                totals[slot] += eval::recall_at(&ranked, relevant, *k);
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let n = answers.len() as f64;
        let mean = [totals[0] / n, totals[1] / n, totals[2] / n];
        if index == 0 {
            baseline = mean;
        }
        let delta = |slot: usize| {
            if index == 0 {
                String::new()
            } else {
                format!(" ({:+.1})", 100.0 * (mean[slot] - baseline[slot]))
            }
        };
        println!(
            "{name:<26} {:>8.1}%{} {:>8.1}%{} {:>8.1}%{}",
            100.0 * mean[0],
            delta(0),
            100.0 * mean[1],
            delta(1),
            100.0 * mean[2],
            delta(2)
        );
    }

    // A mean over 24 queries can be carried by two of them, so the paired count says how broadly
    // the winner actually wins. This is the honest statistic at this sample size.
    let best = eval::Combine::Sum;
    let mut better = 0;
    let mut same = 0;
    let mut worse = 0;
    for (query_index, relevant) in &answers {
        let with = eval::recall_at(
            &eval::rank(corpus, &vectors[*query_index], &walk, best),
            relevant,
            10,
        );
        let without = eval::recall_at(
            &eval::rank(
                corpus,
                &vectors[*query_index],
                &eval::Walk::none(),
                eval::Combine::OwnOnly,
            ),
            relevant,
            10,
        );
        if with > without + 1e-9 {
            better += 1;
        } else if without > with + 1e-9 {
            worse += 1;
        } else {
            same += 1;
        }
    }
    println!(
        "\nsum versus baseline at r@10, per query: {better} better, {same} unchanged, {worse} worse"
    );
    Ok(())
}

/// Read a labelled answer key into query vectors and their relevant sets.
fn answer_key(
    embedder: &StaticEmbedder,
    path: &str,
) -> Result<Vec<(scatterhoard::embedding::Vector, Vec<MemoryId>)>, String> {
    #[derive(serde::Deserialize)]
    struct Labelled {
        q: String,
        relevant: Vec<String>,
    }
    #[derive(serde::Deserialize)]
    struct Labels {
        queries: Vec<Labelled>,
    }
    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let labels: Labels = serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))?;
    let texts: Vec<String> = labels.queries.iter().map(|row| row.q.clone()).collect();
    let vectors = embedder.embed(&texts).map_err(|error| error.to_string())?;

    Ok(labels
        .queries
        .iter()
        .zip(vectors)
        .map(|(row, vector)| {
            let ids: Vec<MemoryId> = row
                .relevant
                .iter()
                .filter_map(|id| id.parse::<MemoryId>().ok())
                .collect();
            (vector, ids)
        })
        .filter(|(_, ids)| !ids.is_empty())
        .collect())
}

/// Measure the walk's four parameters, and then how to decide when to stop returning memories.
///
/// The design holds every weight flat and *recorded as untuned* until a corpus can license one, on
/// the grounds that a flat weight announces itself as provisional where "a weight guessed to three
/// decimals reads as reasoned about and gets built upon". A corpus exists now, so these can be
/// measured instead — which is the same rule honoured rather than set aside.
fn sweep(corpus: &Corpus, embedder: &StaticEmbedder, path: &str) -> Result<(), String> {
    let key = answer_key(embedder, path)?;
    #[allow(clippy::cast_precision_loss)]
    let n = key.len() as f64;
    let rule = eval::Combine::Sum;

    let score = |walk: &eval::Walk| -> (f64, f64) {
        let mut r10 = 0.0;
        let mut r5 = 0.0;
        for (query, relevant) in &key {
            let ranked = eval::rank(corpus, query, walk, rule);
            r5 += eval::recall_at(&ranked, relevant, 5);
            r10 += eval::recall_at(&ranked, relevant, 10);
        }
        (r5 / n, r10 / n)
    };

    println!(
        "\n{} queries. Sum is the rule; the walk is what varies.",
        key.len()
    );
    let baseline = score(&eval::Walk::none());
    println!(
        "baseline, no walk:  r@5 {:.1}%  r@10 {:.1}%",
        100.0 * baseline.0,
        100.0 * baseline.1
    );

    let mut best = (
        eval::Walk {
            seeds: 5,
            hops: 2,
            decay: 0.7,
            frontier: 20,
        },
        0.0,
        0.0,
    );
    let mut rows: Vec<(f64, eval::Walk, f64)> = Vec::new();
    for seeds in [3usize, 5, 8, 12] {
        for hops in [1usize, 2, 3, 4, 5] {
            for decay in [0.3f64, 0.5, 0.7, 0.9, 0.95, 0.99] {
                for frontier in [5usize, 10, 20, 40] {
                    let walk = eval::Walk {
                        seeds,
                        hops,
                        decay,
                        frontier,
                    };
                    let (r5, r10) = score(&walk);
                    rows.push((r10, walk, r5));
                    if r10 > best.2 {
                        best = (walk, r5, r10);
                    }
                }
            }
        }
    }
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));

    println!("\nbest ten configurations by r@10:");
    println!(
        "  {:>5} {:>5} {:>6} {:>8}   {:>7} {:>7}",
        "seeds", "hops", "decay", "frontier", "r@5", "r@10"
    );
    for (r10, walk, r5) in rows.iter().take(10) {
        println!(
            "  {:>5} {:>5} {:>6.1} {:>8}   {:>6.1}% {:>6.1}%",
            walk.seeds,
            walk.hops,
            walk.decay,
            walk.frontier,
            100.0 * r5,
            100.0 * r10
        );
    }

    // How much of the spread is real. If the best and the median are close, the parameters barely
    // matter and a flat default costs nothing -- which is worth knowing before pinning four numbers.
    let median = rows[rows.len() / 2].0;
    println!(
        "\nspread across {} configurations: best r@10 {:.1}%, median {:.1}%, worst {:.1}%",
        rows.len(),
        100.0 * rows[0].0,
        100.0 * median,
        100.0 * rows[rows.len() - 1].0
    );

    // ---------------------------------------------------------------------
    // When to stop
    // ---------------------------------------------------------------------
    let walk = best.0;
    println!(
        "\n\nwhen to stop, using seeds {} hops {} decay {} frontier {}:",
        walk.seeds, walk.hops, walk.decay, walk.frontier
    );
    println!(
        "\n  {:>6} {:>9} {:>9} {:>9}",
        "k", "recall", "precision", "F1"
    );
    for k in [1usize, 3, 5, 8, 10, 15, 20, 30] {
        let mut recall = 0.0;
        let mut precision = 0.0;
        for (query, relevant) in &key {
            let ranked = eval::rank(corpus, query, &walk, rule);
            recall += eval::recall_at(&ranked, relevant, k);
            precision += eval::precision_at(&ranked, relevant, k);
        }
        let (recall, precision) = (recall / n, precision / n);
        let f1 = if recall + precision > 0.0 {
            2.0 * recall * precision / (recall + precision)
        } else {
            0.0
        };
        println!(
            "  {k:>6} {:>8.1}% {:>8.1}% {:>8.1}%",
            100.0 * recall,
            100.0 * precision,
            100.0 * f1
        );
    }

    println!("\n  cutting at the largest gap instead of a fixed k:");
    for window in [10usize, 20, 30] {
        let mut recall = 0.0;
        let mut precision = 0.0;
        let mut returned = 0.0;
        for (query, relevant) in &key {
            let scored = eval::rank_scored(corpus, query, &walk, rule);
            let cut = eval::cut_at_largest_gap(&scored, window);
            let ranked: Vec<MemoryId> = scored.iter().map(|(id, _)| *id).collect();
            recall += eval::recall_at(&ranked, relevant, cut);
            precision += eval::precision_at(&ranked, relevant, cut);
            #[allow(clippy::cast_precision_loss)]
            {
                returned += cut as f64;
            }
        }
        let (recall, precision, returned) = (recall / n, precision / n, returned / n);
        // How much it adapts is the point of it. A rule that always returns the same count is a
        // fixed `k` wearing a disguise.
        let mut cuts: Vec<usize> = key
            .iter()
            .map(|(query, _)| {
                eval::cut_at_largest_gap(&eval::rank_scored(corpus, query, &walk, rule), window)
            })
            .collect();
        cuts.sort_unstable();
        let f1 = if recall + precision > 0.0 {
            2.0 * recall * precision / (recall + precision)
        } else {
            0.0
        };
        println!(
            "  window {window:>3}: recall {:>5.1}%  precision {:>5.1}%  F1 {:>5.1}%  mean {:.1} returned",
            100.0 * recall,
            100.0 * precision,
            100.0 * f1,
            returned
        );
        println!(
            "              cut sizes: min {} median {} max {}",
            cuts[0],
            cuts[cuts.len() / 2],
            cuts[cuts.len() - 1]
        );
    }
    Ok(())
}

fn load_embedder() -> Result<StaticEmbedder, String> {
    let path = match std::env::var_os("SCATTERHOARD_MODEL") {
        Some(path) => PathBuf::from(path),
        None => {
            let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_owned())?;
            PathBuf::from(home).join(".local/share/scatterhoard/model")
        }
    };
    StaticEmbedder::load(&path).map_err(|error| error.to_string())
}

fn report(corpus: &Corpus) {
    let reaches = eval::link_reach(corpus);
    let summary = eval::summarise(corpus);

    println!("\ncorpus size: {} memories", summary.corpus_size);
    println!(
        "traversable edges: {} measured, {} unmeasurable",
        summary.measured, summary.unmeasurable
    );

    if summary.measured == 0 {
        println!("\nnothing to measure.");
        return;
    }

    // Percentiles, not ranks. A rank means nothing without the corpus size beside it.
    let mut percentiles: Vec<f64> = reaches.iter().map(|reach| reach.percentile).collect();
    percentiles.sort_by(f64::total_cmp);
    println!("\npercentile of the linked target, from its source:");
    for (label, value) in [
        ("10th", quantile(&percentiles, 0.10)),
        ("25th", quantile(&percentiles, 0.25)),
        ("median", summary.median_percentile),
        ("75th", quantile(&percentiles, 0.75)),
        ("90th", quantile(&percentiles, 0.90)),
    ] {
        println!("  {label:>7}  {:>6.2}%", 100.0 * value);
    }

    // Per label, which is the question the pilot could only hint at: a decision and its motivation
    // share vocabulary, while a principle and a case of it are worded completely differently.
    let mut by_label: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for reach in &reaches {
        by_label
            .entry(format!("{:?}", reach.label))
            .or_default()
            .push(reach.percentile);
    }
    let mut rows: Vec<(String, usize, f64)> = by_label
        .into_iter()
        .map(|(label, mut values)| {
            values.sort_by(f64::total_cmp);
            let median = quantile(&values, 0.50);
            (label, values.len(), median)
        })
        .collect();
    rows.sort_by(|a, b| a.2.total_cmp(&b.2));

    println!("\nby label, median percentile (small n is a signal, not a result):");
    println!("  {:<16} {:>4} {:>9}", "label", "n", "median");
    for (label, n, median) in rows {
        println!("  {label:<16} {n:>4} {:>8.2}%", 100.0 * median);
    }

    // The number that actually answers the question. The median can sit inside the cut while a
    // large minority sits well outside it, and that minority is the population traversal exists to
    // reach — so it is reported first, and the median-derived crossover below is context for it.
    println!("\nshare of targets a top-k cut would miss — the edges traversal is for:");
    for k in [1usize, 5, 10, 20] {
        let share = eval::share_outside_cut(&reaches, k);
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation
        )]
        let count = (share * reaches.len() as f64).round() as usize;
        println!(
            "  k={k:<3} {:>6.1}%  ({count} of {})",
            100.0 * share,
            reaches.len()
        );
    }

    // The finding the whole size argument rests on, stated rather than left as arithmetic.
    match summary.crossover_at(K) {
        Some(crossover) => {
            println!("\nat k={K}, the median target leaves the cut near {crossover} memories.",);
            if summary.discriminates_at(K) {
                println!(
                    "this corpus ({} memories) is above that, so traversal is distinguishable \
                     from a larger k.",
                    summary.corpus_size
                );
            } else {
                println!(
                    "this corpus ({} memories) is BELOW that: traversal and a larger k return the \
                     same memories, so no number of queries can tell one combination rule from \
                     another. The corpus needs to be larger before queries are worth authoring.",
                    summary.corpus_size
                );
            }
        }
        None => println!("\nno median to derive a crossover from."),
    }

    // The rare far-out edge is the pure case for traversal: two memories sharing almost nothing
    // textually, connected because an author knew they belonged together.
    let mut furthest: Vec<&eval::Reach> = reaches.iter().collect();
    furthest.sort_by(|a, b| b.percentile.total_cmp(&a.percentile));
    println!("\nfurthest targets — the edges content similarity cannot reach:");
    for reach in furthest.iter().take(5) {
        println!(
            "  {:>6.2}%  rank {:<5} cos {:>6.3}  {:?}  {} -> {}",
            100.0 * reach.percentile,
            reach.rank,
            reach.similarity,
            reach.label,
            short(&reach.source.to_string()),
            short(&reach.target.to_string()),
        );
    }
}

/// The value at a quantile of an already-sorted slice, by nearest rank.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len().is_multiple_of(2) && (q - 0.5).abs() < f64::EPSILON {
        let mid = sorted.len() / 2;
        return (sorted[mid - 1] + sorted[mid]) / 2.0;
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation
    )]
    let index = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn short(id: &str) -> String {
    id.chars().take(8).collect()
}
