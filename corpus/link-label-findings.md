# Link-label corpus test — findings

Run 2026-08-01. Corpus: [design-memories.jsonl](design-memories.jsonl) — 39 memories and 29 edges
drawn from real Scatterhoard material (the embedder spike findings, the design decisions of
2026-07-30 through 2026-08-01, and the standards). The memories were written **before** the link
vocabulary existed, so the labels are fitted to the material rather than the material to the labels.

Two passes. The first tested the twelve labels from the larger memory system plus `RESULTS_FROM`
(replacing `CAUSED_BY`), `SUPERSEDED_BY`, and `SUPPORTS`. The second, after review, renamed
`REQUIRES` to `DEPENDS_ON` and added `MOTIVATED_BY` and `VIOLATES`. Both passes are reproducible via
`python3 corpus/build_corpus.py`; the second-pass relabellings are listed explicitly in the
`REVISIONS` table there.

## 1. The outbound rule holds without exception

**29 of 29 edges point outbound from the newer memory.** Not one wanted to be written from the older
side, including every edge using the "free direction" labels, where the new memory's side was
supposed to be unconstrained. It wasn't. The freedom is in *which label* the new memory needs, never
in which side it occupies.

This is what justifies the reverse labels rather than undermining them. Without `RESULTS_FROM` /
`MOTIVATED_BY`, nine edges would each have required opening an already-written memory to add an
outbound edge — which is exactly what a multi-set design often makes impossible.

`CAUSES` still earns its place: both its uses are a new memory recording that it is the *cause* of
something already written (`xcode-licence CAUSES red-phase-unobserved`). Both directions of the
causal pair are used, and both are used outbound.

## 2. Final label usage

| label | uses | |
| --- | ---: | --- |
| `MOTIVATED_BY` | 8 | dominant — 28% of all edges |
| `SUPPORTS` | 4 | |
| `RELATED_TO` | 3 | escape hatch, behaving as intended |
| `CAUSES` | 2 | |
| `SUPERSEDES` | 2 | |
| `VIOLATES` | 2 | |
| `RESOLVES` | 2 | |
| `REFERS_TO` | 2 | |
| `RESULTS_FROM` | 1 | |
| `CONTRADICTS` | 1 | |
| `DEPENDS_ON` | 1 | |
| `INSTANCE_OF` | 1 | |
| `SUPERSEDED_BY` | 0 | untestable here — see §5 |
| `EXCEPTION_TO` | 0 | emptied by `VIOLATES` — see §4 |
| `PRECEDES` / `FOLLOWS` | 0 | **not fairly tested** — see §6 |

## 3. `REQUIRES` was a trap, and the split shows how big a one

Three of the four first-pass `REQUIRES` edges were not dependencies at all — they were "was
motivated by". After renaming to `DEPENDS_ON` and splitting out `MOTIVATED_BY`, exactly **one**
genuine dependency remains (`scope-partitioned-storage DEPENDS_ON scope-explicit` — you cannot
partition by scope unless scope exists).

The larger result is in the other column. `RESULTS_FROM` fell from 7 uses to 1 once `MOTIVATED_BY`
existed to take the load:

> **The dominant relation in a design corpus is reasoning, not causation** — and it had been
> travelling under a label named for causation.

`MOTIVATED_BY` at 8 uses is three times anything else in the set. Physical causation (`CAUSES`,
`RESULTS_FROM`) accounts for 3 edges between them.

One case stayed genuinely hard: `edge-descriptions-dropped → why-is-a-field`. The decision to drop
edge descriptions is *justified by* the `why` field existing, and is also *invalid without* it —
motivation and dependency at once. It is recorded as `MOTIVATED_BY`. This is the discrimination to
watch in convention text.

## 4. `VIOLATES` earns its place, and its absence was actively harmful

Two uses in a 39-memory corpus, which is enough on frequency alone in a coding context. But the
sharper argument is what happened to `EXCEPTION_TO`: adding `VIOLATES` **emptied it**. The single
first-pass `EXCEPTION_TO` edge — `red-phase-unobserved → tdd-required` — was wrong.
[STANDARDS.md](../STANDARDS.md) defines the sanctioned exceptions narrowly (trivial wiring, generated
code, tests that would only re-assert the type checker). A red phase missed because a toolchain was
broken is not one of them; it is a violation that was mitigated after the fact.

So without `VIOLATES`, the failure mode is not a missing edge. It is **defects silently reclassified
as approved carve-outs** — the memory system recording that a rule was bent by permission when it
was actually broken. That is worse than having no edge at all.

`EXCEPTION_TO` and `VIOLATES` are the same shape — a deviation from a rule — separated by whether
the deviation is sanctioned. That is a clean, checkable distinction. `EXCEPTION_TO` showing zero uses
here reflects corpus composition (no sanctioned carve-outs happened to be recorded as memories), not
evidence against the label.

## 5. `SUPERSEDED_BY` was not exercised, and could not have been

Zero uses, as expected. It exists so the lower-authority side can record its own obsolescence when a
project memory supersedes a local one. A single-project corpus contains no trust boundary and so
cannot produce the case. **Not evidence against it — evidence that this corpus cannot test it.**
Evaluating it needs a two-scope corpus.

## 6. `PRECEDES` / `FOLLOWS` were not fairly tested — first-pass reasoning retracted

The first pass concluded these should be cut, on two grounds. **The first ground was wrong.**

It argued the information is already in the store, since every memory carries `created` and ids are
UUIDv7. That holds only when memory creation order tracks world-event order — and it did here
**only because the corpus was generated in event order**. That is an artifact of how the corpus was
built, not a property of the domain.

In real use, information reaches an LLM out of order: reconstructing a bug's history, reading a
postmortem, being told background after the fact. Then `created` order says nothing about world
order, and these labels carry information nothing else does. The corpus already contains one such
inversion, glossed over at the time — `xcode-licence` was recorded *after* `red-phase-unobserved`
but describes a condition that existed *before* it. Causation was available there, so `PRECEDES` was
never needed; had the causal link been unknown, nothing else could have expressed the order.

The second ground survives but is weaker: everywhere world-time ordering mattered here, a causal
relation also held, and `CAUSES` / `RESULTS_FROM` says strictly more. That is a real dominance
effect, but it is exactly what fails in the case these labels are *for* — a sequence of events whose
causal structure is precisely what is unknown.

**Verdict changed from "cut" to "not tested."** A fair test needs a corpus built from a
reconstruction context — a debugging session or incident write-up, where memories are created in one
sitting about events spread across past time. No such material exists in this repository yet; it
should be captured rather than synthesised, since synthesising it would reproduce the same bias in a
new form.

Related design question this surfaces: there is no `occurred` field distinct from `created`. If
world-time matters, these edges are currently the only way to express it. Whether that belongs in a
field or in edges is a live question — a field is cheaper when absolute times are known, edges are
better when only relative order is, and an LLM reconstructing a history usually has the latter.

## 7. Other hesitations worth recording

- **`MOTIVATED_BY` and `SUPPORTS` overlap.** `direction-fixed-by-creation-order` carries both to the
  same target: it is derived from that observation *and* is evidence for it. Both edges are honest,
  but the two will be confused.
- **`SUPERSEDES` versus `CONTRADICTS` resolved cleanly only because two memories had been split.**
  `hubness-general SUPERSEDES hub-cause-static` was unambiguous because the hub *observation* and the
  hub *explanation* were separate memories. Bundled, the correction would have partly contradicted
  and partly superseded, and neither label would have fit. Working rule: `SUPERSEDES` when the new
  memory replaces the old claim, `CONTRADICTS` when both stand and conflict.
- **`RELATED_TO` absorbed residue without attracting better-labelled edges.** Three uses, each a
  genuine "these belong together and nothing more specific applies" — e.g.
  `csls-rejected RELATED_TO centroid-does-help`, where the real relation is *was tested against and
  lost to*, which nothing in the set expresses.

## Recommendation

**Sixteen labels**, of which four are unproven:

| | |
| --- | --- |
| **Proven in this corpus** | `MOTIVATED_BY`, `SUPPORTS`, `RELATED_TO`, `CAUSES`, `SUPERSEDES`, `VIOLATES`, `RESOLVES`, `REFERS_TO`, `RESULTS_FROM`, `CONTRADICTS`, `DEPENDS_ON`, `INSTANCE_OF` |
| **Kept on structural grounds** | `SUPERSEDED_BY` — needs a two-scope corpus |
| **Kept, untested** | `EXCEPTION_TO` — sound distinction against `VIOLATES`, no instance here |
| **Deferred** | `PRECEDES` / `FOLLOWS` — needs a reconstruction-context corpus |

Changes from the vocabulary as received: `CAUSED_BY` → `RESULTS_FROM` (rename), `REQUIRES` →
`DEPENDS_ON` (rename, to break the motivation confusion), and four additions — `MOTIVATED_BY`,
`SUPPORTS`, `VIOLATES`, `SUPERSEDED_BY`.

## Reproducing

`python3 corpus/build_corpus.py` regenerates the JSONL and reprints every number above. The memories
and edges are the data at the top of that script; edit there, not in the JSONL.

The `_key` and `_link_keys` fields in the JSONL are **not part of the memory format** — they carry
human-readable identifiers so the file stays reviewable and edges can be read without resolving
UUIDs. An importer should ignore keys beginning with `_`.

The file is written in the real format and **is read by the real reader** — `the_labelled_corpus_reads`
in `tests/memory_file.rs` parses it through `memory_file::read` on every test run, so it cannot
drift out of the format silently. `format_version` is no longer a placeholder; it is the version
the tool writes.

One value is still provisional: `author` is uniform throughout rather than reflecting real
per-memory attribution, which matters if the corpus is ever used to exercise the export review.
