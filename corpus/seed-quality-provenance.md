# Seed-quality corpus — provenance and what it can answer

Recovered 2026-08-15 from the session transcript of the embedder spike, having never been committed
at the time. This is the fixture behind the recall@5 figures cited in
[Recall Mechanisms](../documents/design/recall-mechanisms.md) and
[Embeddings and Vector Storage](../documents/design/embeddings.md).

## What is here

| file | what it is |
| --- | --- |
| [seed-quality-corpus.json](seed-quality-corpus.json) | **45 memories, 20 queries.** Each memory has `id`, `content`, `why`, `keywords`; each query has `q` and `relevant` — a list of memory ids. |
| [seed_quality_baseline.py](seed_quality_baseline.py) | The transformer baseline, run against `all-MiniLM-L6-v2` via `sentence-transformers`. Reports recall@{1,3,5,10} and worst-hub counts. |
| [seed_quality_spike_main.rs.txt](seed_quality_spike_main.rs.txt) | The Rust spike that produced the `potion-base-32M` numbers, kept as text so it is not a build target. |

The embedding input is `content + why + keywords`, which is the decision the ablation settled.

## Why it is trustworthy as *the* fixture

It was not re-run to confirm this — the arithmetic is what identifies it. The relevance lists hold
one, two or three ids, so per-query recall lands on units, halves and thirds. Every published figure
is exactly reachable as a sum over 20 queries:

| figure | as n/20 |
| --- | --- |
| 57.5% (content only) | 11.5 |
| 65.8% (content+keywords) | 13.1666… |
| 66.7% (content+why) | 13.3333… |
| 71.7% (content+why+keywords) | 14.3333… |
| 77.5% (centroid removed) | 15.5 |

No relevance id dangles. **Re-run 2026-09-08 and the arithmetic held for raw cosine**: 71.7% at r@5,
and every other cell of the raw row, reproduced exactly. The centroid-removal row did not — see
[Embeddings and Vector Storage](../documents/design/embeddings.md#re-run-2026-09-08-raw-reproduces-exactly-the-centred-row-does-not).
The probe is [`probe_reproduce_the_seed_quality_figures`](../tests/probes.rs).

## What it can and cannot answer

**It can** settle embedder and embedding-input questions, which is what it was built for, and it can
be re-run against any future embedder to check that a swap does not regress recall.

**It cannot, as it stands, settle the traversal score combination.** Two gaps, in order of severity:

- **No links.** The memories carry no edges, so nothing can be reached by traversal at all.
- **No link-dependent queries.** This is the harder gap and the easier one to overlook. Every
  candidate combination of an inherited score with a query-similarity score — maximum, sum, weighted
  blend, inherited-as-floor — ranks *identically* on a query whose best answer content similarity
  already finds. Discriminating between them needs queries whose relevant memories are reachable by
  an edge but semantically distant from the query wording, and those have to be authored
  deliberately.

So this is the harness's foundation rather than the harness. See `recall-eval-harness` in
[Backlog](../TRACKING.md#backlog).

## The lesson worth keeping

The apparatus behind a measurement is part of the measurement. This corpus sat in a `/private/tmp`
scratchpad, was cited in two design documents as settled evidence, and was one temp-directory sweep
from being unrecoverable — the directory is indeed gone. It survived only because the transcript
recorded the `Write` call that created it.
