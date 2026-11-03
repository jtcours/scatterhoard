"""Build a labelled link corpus from real Scatterhoard design memories.

Purpose: test the proposed link-label vocabulary against material that was written before the
vocabulary existed, so the labels are being fitted to the memories rather than the reverse.

Emits corpus/design-memories.jsonl in the format decided for Scatterhoard's memory file: a header
line carrying the format version, then one memory per line, ordered by id (which for UUIDv7 is
creation order).
"""

import json
import os
import random
from datetime import datetime, timezone

random.seed(20260801)  # reproducible ids

PROPOSED_LABELS = [
    "RELATED_TO", "REFERS_TO", "CAUSES", "RESULTS_FROM", "MOTIVATED_BY", "SUPERSEDES",
    "SUPERSEDED_BY", "CONTRADICTS", "EXCEPTION_TO", "VIOLATES", "RESOLVES", "PRECEDES",
    "FOLLOWS", "DEPENDS_ON", "INSTANCE_OF", "SUPPORTS",
]

# Second pass, after review: REQUIRES was standing in for "was motivated by" and is renamed
# DEPENDS_ON to break that; MOTIVATED_BY splits reasoning from physical causation; VIOLATES splits
# an unsanctioned deviation from a sanctioned EXCEPTION_TO.
REVISIONS = [
    # (from, to, old label, new label, note)
    ("export-order", "git-merge", "RESULTS_FROM", "MOTIVATED_BY", "reason, not dependency"),
    ("uuidv7", "export-order", "RESULTS_FROM", "MOTIVATED_BY", "reason, not dependency"),
    ("read-time-canonicalisation", "scope-partitioned-storage", "RESULTS_FROM", "MOTIVATED_BY", "reason"),
    ("trust-directional-edges", "scope-partitioned-storage", "RESULTS_FROM", "MOTIVATED_BY", "reason"),
    ("superseded-by-needed", "trust-directional-edges", "RESULTS_FROM", "MOTIVATED_BY", "reason"),
    # This memory keeps its separate SUPPORTS edge to the same target: it is both derived from the
    # observation and evidence for it. That is the RESULTS_FROM/SUPPORTS overlap, surviving the split.
    ("direction-fixed-by-creation-order", "outbound-rule", "RESULTS_FROM", "MOTIVATED_BY", "derived from"),
    ("embeddings-local", "export-order", "REQUIRES", "MOTIVATED_BY", "reason, not dependency"),
    ("edge-descriptions-dropped", "why-is-a-field", "REQUIRES", "MOTIVATED_BY", "HARD: reason or dependency?"),
    ("scope-partitioned-storage", "scope-explicit", "REQUIRES", "DEPENDS_ON", "genuine prerequisite"),
    ("link-missing-set-name", "cross-set-ref-format", "CONTRADICTS", "VIOLATES", "code failing a decision"),
    ("red-phase-unobserved", "tdd-required", "EXCEPTION_TO", "VIOLATES", "unsanctioned, not a carve-out"),
]

# Labels whose direction is claimed to be fixed by creation order: the memory being written is
# always the `from` side. The corpus is the test of that claim.
ONE_DIRECTIONAL = {
    "REFERS_TO", "SUPERSEDES", "RESOLVES", "EXCEPTION_TO", "VIOLATES", "DEPENDS_ON",
    "INSTANCE_OF", "SUPPORTS", "MOTIVATED_BY",
}
SYMMETRIC = {"RELATED_TO", "CONTRADICTS"}
FREE = {"CAUSES", "RESULTS_FROM", "PRECEDES", "FOLLOWS"}
# SUPERSEDED_BY is the trust-safe reverse; it is one-directional but points backwards in time.
BACKWARD = {"SUPERSEDED_BY"}


def uuid7(ts_ms: int) -> str:
    """A UUIDv7 for a given millisecond, so ids sort into creation order."""
    rand_a = random.getrandbits(12)
    rand_b = random.getrandbits(62)
    n = (ts_ms & 0xFFFFFFFFFFFF) << 80
    n |= 0x7 << 76
    n |= rand_a << 64
    n |= 0b10 << 62
    n |= rand_b
    h = f"{n:032x}"
    return f"{h[0:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:32]}"


def ts(day: str, hhmm: str) -> tuple[int, str]:
    dt = datetime.fromisoformat(f"{day}T{hhmm}:00+00:00").astimezone(timezone.utc)
    return int(dt.timestamp() * 1000), dt.strftime("%Y-%m-%dT%H:%M:%SZ")


# key, day, time, origin, confidence, keywords, content, why, context, [(label, target_key), ...]
M = [
    ("scope-explicit", "2026-07-30", "21:10", "joint", 0.95, ["scope", "steering", "store"],
     "Memory scope is an explicit parameter on every store operation, never inferred from context.",
     "The software has no way to tell which set a memory belongs to, and a personal memory committed to a public repository cannot be taken back.",
     "Decided while working through memory steering, early in the design phase.", []),

    ("git-merge", "2026-07-30", "21:30", "joint", 0.9, ["sync", "git", "merge"],
     "Memory sets reconcile by exporting, pulling, and letting git three-way-merge the exported file, then re-importing.",
     "Git already has a real common ancestor to merge against; reimplementing that against live database state would just re-derive what git does, against rows instead of lines.",
     None, []),

    ("export-order", "2026-07-30", "21:45", "joint", 0.9, ["file-format", "merge", "ordering"],
     "The exported memory file is written in a consistent order, keyed on memory id or creation time, to keep merge diffs small.",
     "Diff size is what determines whether a human can review a memory merge, and unstable ordering makes every export look like a rewrite.",
     None, [("RESULTS_FROM", "git-merge")]),

    ("embed-input", "2026-07-30", "22:20", "llm", 0.7, ["recall", "embedding", "keywords"],
     "The embedding input for semantic recall is content plus why plus keywords, not content alone.",
     "The why text often names related topics and paths not taken, which is what semantic recall should surface; keywords are already curated for topical relevance and cost nothing extra to include.",
     None, []),

    ("import-cost", "2026-07-31", "07:10", "llm", 0.85, ["import", "embedding", "performance"],
     "Because vectors are never exported, every import re-embeds whatever changed, putting embedder speed on the critical path of an ordinary git pull.",
     "A step that takes a minute on every pull is one users will disable; a sub-second one is invisible.",
     None, []),

    ("store-when-lowered", "2026-07-31", "07:40", "human", 0.9, ["conventions", "store-when"],
     "The store-when threshold is rediscovery effort - roughly two or more turns to work out again - and explicitly includes how parts of the system fit together.",
     "The earlier bar, cannot be re-derived from the repository, was nearly unsatisfiable and excluded exactly the orientation knowledge that makes a codebase fast to work in.",
     "Raised by the human partner after reviewing the first convention draft.", []),

    ("tdd-required", "2026-07-31", "08:00", "human", 0.95, ["standards", "tdd", "testing"],
     "Write a failing test before the code that makes it pass, wherever practical.",
     "A test written after the code tends to assert what the code does rather than what it should do, and never demonstrates that it can fail.",
     None, []),

    ("embed-input-measured", "2026-07-31", "14:20", "llm", 0.9, ["spike", "recall", "embedding", "measurement"],
     "Measured on a 45-memory labelled corpus: content+why+keywords reached 71.7% recall@5 against 57.5% for content alone, with both additions contributing and composing.",
     "The embedding-input decision had been made on reasoning alone; an ablation was cheap and could have contradicted it.",
     "From the embedder spike run on 2026-07-31.",
     [("SUPPORTS", "embed-input")]),

    ("model-size-beats-tuning", "2026-07-31", "14:30", "llm", 0.7, ["spike", "embedder", "measurement"],
     "potion-base-32M outperformed potion-retrieval-32M on this corpus, so the retrieval-tuned variant bought nothing over the general model of the same size.",
     "The retrieval-tuned variant looked like the obvious choice for a retrieval task, and was not.",
     "From the embedder spike.", []),

    ("hub-observed", "2026-07-31", "14:45", "llm", 0.95, ["spike", "hubness", "recall"],
     "One memory dense in common domain vocabulary appeared in the top 5 for 13 of 20 queries, including queries it had nothing to do with.",
     "It consumes a seed slot and sends graph traversal into the wrong neighbourhood, so its cost is higher than a recall metric alone shows.",
     "Observed while reviewing per-query results in the embedder spike.", []),

    ("hub-cause-static", "2026-07-31", "14:50", "llm", 0.6, ["spike", "hubness", "embedder"],
     "The hub effect is caused by static embeddings having no contextualisation, so a memory dense in common vocabulary lands near the corpus centroid.",
     "It explained the observation and matched the known weakness of static embedding models.",
     "First explanation offered for the hub, during the spike.", []),

    ("centroid-no-help", "2026-07-31", "15:05", "llm", 0.5, ["spike", "hubness", "centroid"],
     "Removing the corpus centroid improves the recall metric but does not dislodge the specific hub memory.",
     "The hub counts appeared unchanged after centring, so centroid removal looked like a metric improvement that left the underlying problem intact.",
     "Reported mid-spike, before the variant being read was checked.", []),

    ("centroid-does-help", "2026-07-31", "15:30", "llm", 0.9, ["spike", "hubness", "centroid", "correction"],
     "Removing the centroid from both memory and query vectors drops the worst hub from 13 of 20 queries to 6 of 20, while also lifting recall@5 from 71.7% to 77.5%.",
     "The earlier reading came from a variant that centred only the memory vectors; centring both sides is what actually moves the hub counts.",
     "A correction made mid-session after re-reading the spike output.",
     [("SUPERSEDES", "centroid-no-help")]),

    ("hubness-general", "2026-07-31", "15:40", "llm", 0.85, ["spike", "hubness", "embedder"],
     "Hubness is a general property of high-dimensional spaces rather than a static-embedding artifact: all-MiniLM-L6-v2 shows the same effect with its worst hub at 8 of 20 queries.",
     "Points near the centre of any high-dimensional distribution become nearest neighbours to disproportionately many others; static embeddings are worse at it, not uniquely subject to it.",
     None,
     [("SUPERSEDES", "hub-cause-static")]),

    ("ceiling-open", "2026-07-31", "15:50", "llm", 0.8, ["spike", "open-question", "embedder"],
     "Whether 87.5% recall@10 is the model's ceiling or the corpus's is unresolved, pending a transformer baseline on the identical corpus and metric.",
     "Without a stronger model scored the same way there is no way to tell a model limitation from a task limitation.",
     "Left open at the end of the first spike pass.", []),

    ("model2vec-matches", "2026-07-31", "17:10", "llm", 0.9, ["spike", "embedder", "measurement"],
     "potion-base-32M reached 77.5% recall@5 against all-MiniLM-L6-v2 at 74.2% on an identical corpus and metric, so the published MTEB gap does not appear on this workload.",
     "The ceiling is the task and the corpus, not the model, so paying a transformer's size and dependency cost buys no measurable quality here.",
     None,
     [("RESOLVES", "ceiling-open")]),

    ("throughput", "2026-07-31", "17:20", "llm", 0.9, ["spike", "performance", "import"],
     "Embedding throughput is roughly 18,000 memories per second, so a cold re-embed of a 10,000-memory set is comfortably sub-second.",
     "This was the concern that made import cost a design constraint in the first place.",
     None,
     [("RESOLVES", "import-cost")]),

    ("csls-rejected", "2026-07-31", "17:40", "llm", 0.5, ["spike", "csls", "hubness"],
     "CSLS local scaling is not adopted: alone it is worse than centroid removal, and stacked it buys 0.8 points at recall@5 for a 7.5-point regression at recall@3.",
     "The measurement is the least trustworthy in the spike - k=10 against a 45-memory corpus computes the penalty over roughly a fifth of the whole set, nothing like k=10 over thousands.",
     None,
     [("RELATED_TO", "centroid-does-help")]),

    ("embeddings-local", "2026-07-31", "20:15", "joint", 0.95, ["embedding", "export", "file-format"],
     "Vector embeddings are local derived state and never appear in the exported memory file; an importing machine computes its own.",
     "Several hundred floats per memory would destroy the small-diff property the export order exists to protect, and two branches re-embedding the same memory produce different floats with no correct merge resolution.",
     None,
     [("REQUIRES", "export-order"), ("CAUSES", "import-cost")]),

    ("scope-partitioned-storage", "2026-07-31", "21:20", "joint", 0.9, ["storage", "scope", "correctness"],
     "Each scope gets its own store rather than all memories sharing a table with a scope column.",
     "A query against one project's store then cannot return another scope's memories at all, instead of that guarantee being a WHERE clause someone has to remember.",
     "Decided at the end of the 2026-07-31 session.",
     [("REQUIRES", "scope-explicit")]),

    ("cross-store-scores", "2026-07-31", "21:40", "llm", 0.85, ["recall", "scoring", "open-question"],
     "Recall usually wants project union local, but per-store centroid removal puts the two stores' scores on incomparable scales, so merging their rankings by raw score is unsound.",
     "Centroid removal is defined relative to a corpus, and two partitions have two corpora.",
     None,
     [("RESULTS_FROM", "scope-partitioned-storage"), ("REFERS_TO", "centroid-does-help")]),

    ("red-phase-unobserved", "2026-08-01", "06:30", "llm", 0.9, ["testing", "tdd", "caveat"],
     "The first convention tests were never observed failing, because the Xcode licence had not been accepted and linking failed; two mutation tests were substituted after the fact.",
     "Recording the gap honestly matters more than the tests being green - a test never seen to fail has not been shown to test anything.",
     "From the first session that wrote Rust in this repo.",
     [("EXCEPTION_TO", "tdd-required")]),

    ("xcode-licence", "2026-08-01", "06:35", "human", 0.9, ["environment", "toolchain"],
     "An unaccepted Xcode licence makes every Rust link step fail on this machine, with an error that does not name the licence as the cause.",
     "It presents as a broken toolchain rather than a one-command fix, which is why it cost a whole red phase before being diagnosed.",
     None,
     [("CAUSES", "red-phase-unobserved")]),

    ("uuidv7", "2026-08-01", "09:15", "llm", 0.9, ["identity", "file-format", "ordering"],
     "Memory ids are UUIDv7, so sorting by id and sorting by creation time are the same order.",
     "The export wants one consistent order and names id or creation time as the key; a time-ordered id makes those one sort key rather than two that can disagree, and appends new memories near the end of the file where git merges most cleanly.",
     "Chosen while building the memory data model's value types.",
     [("RESULTS_FROM", "export-order")]),

    ("confidence-guard-redundant", "2026-08-01", "10:05", "llm", 0.95, ["testing", "mutation", "types"],
     "Confidence's explicit is_finite check is redundant for rejection - the range check already excludes NaN and infinities, because nothing compares inside a range against NaN - so it only decides which error is returned.",
     "Found by mutation: removing the guard broke no test, because the test asserted only that construction failed. The guard was kept and the test strengthened to assert the variant.",
     "Found while mutation-testing the memory model on 2026-08-01.",
     [("REFERS_TO", "tdd-required")]),

    ("local-untypeable", "2026-08-01", "10:20", "llm", 0.95, ["types", "scope", "correctness"],
     "Scope::Local cannot be made to report an export target: the mutation is a borrow-checker error, because Local has no SetName to borrow.",
     "Deriving export eligibility from the scope type means local-memories-never-leave is carried by the compiler rather than by a rule at each call site.",
     "Confirmed by attempting the mutation during the memory-model slice.",
     [("SUPPORTS", "scope-explicit"), ("INSTANCE_OF", "correctness-by-construction")]),

    ("correctness-by-construction", "2026-07-31", "20:40", "human", 0.95, ["standards", "types"],
     "Prefer making an error impossible to express over detecting it at run time; a rule enforced by the compiler cannot drift the way one enforced by review does.",
     "Requested by the human partner after seeing how many of the design's own rules were structural claims a type could carry directly.",
     None, []),

    ("link-missing-set-name", "2026-08-01", "11:30", "llm", 0.9, ["defect", "links", "cross-set"],
     "The Link type as first written carries only a memory id, but a cross-set reference was already decided to be memory id plus the name of the referenced set.",
     "The design decision predated the code by weeks and was simply missed when the type was written; it needs fixing regardless of which link labels are chosen.",
     "Noticed while reviewing the link vocabulary on 2026-08-01.",
     [("CONTRADICTS", "cross-set-ref-format")]),

    ("cross-set-ref-format", "2026-07-30", "22:40", "joint", 0.9, ["cross-set", "links", "identity"],
     "A cross-set reference stores the exact memory id plus the name of the referenced set; keywords from the target set may be carried as supplementary context but are not sufficient to resolve a reference.",
     "An id alone cannot be resolved once several sets are loaded, and a dangling reference needs something a human or LLM can search on.",
     None, []),

    ("edge-descriptions-dropped", "2026-08-01", "12:10", "joint", 0.85, ["links", "design", "economy"],
     "Link edges carry no description field; if a relation needs explaining, the explanation belongs in the why of the memory it explains.",
     "An edge description is only load-bearing when the reader cannot see both endpoints, and recall traverses to the connected memory itself; the per-memory why already carries relational reasoning, and a second home for it would be a drift surface outside the conventions and the export review.",
     "Settled while working through the link vocabulary on 2026-08-01.",
     [("REQUIRES", "why-is-a-field")]),

    ("why-is-a-field", "2026-07-30", "22:00", "joint", 0.9, ["data-model", "why"],
     "Why is a separate field on every memory rather than prose folded into the content, and the conventions push hard for every memory to carry one.",
     "Keeping it separate lets recall embed it deliberately and lets a reader judge whether a memory still applies without parsing it back out of the content.",
     None, []),

    ("read-time-canonicalisation", "2026-08-01", "12:30", "joint", 0.8, ["links", "sets", "index"],
     "Inverse link pairs are never canonicalised on write, only at read time via a reverse index built over the loaded stores.",
     "A memory in a set you do not own cannot be rewritten, so asserting a relation from the side you can edit is often the only option available; normalising on write assumes ownership of both endpoints, which a multi-set system does not have.",
     None,
     [("RESULTS_FROM", "scope-partitioned-storage"), ("RELATED_TO", "cross-set-ref-format")]),

    ("trust-directional-edges", "2026-08-01", "12:45", "joint", 0.75, ["links", "trust", "scope"],
     "An edge asserted from a less-trusted store must not suppress a memory in a more-trusted one: a local memory may supersede a project memory, but not the reverse.",
     "Project memories arrive over an untrusted channel and local ones do not, and relying on memory ids being unguessable is the kind of incidental protection this design has consistently refused.",
     None,
     [("RESULTS_FROM", "scope-partitioned-storage")]),

    ("superseded-by-needed", "2026-08-01", "13:00", "llm", 0.7, ["links", "vocabulary", "trust"],
     "SUPERSEDED_BY is needed even though the creating memory is normally the superseder, so the lower-authority side can record its own obsolescence.",
     "If a project memory supersedes a local one, writing SUPERSEDES on the project memory is both impossible and the trust inversion just forbidden; writing SUPERSEDED_BY locally is the only trust-safe option.",
     None,
     [("RESULTS_FROM", "trust-directional-edges")]),

    ("outbound-rule", "2026-08-01", "13:20", "human", 0.85, ["links", "vocabulary", "conventions"],
     "The LLM naturally writes link labels outbound from the memory it is creating, and passive-voice label names cause mislabels.",
     "Observed over a few months of daily use of a larger memory system, which is why that system's edge set has reverse labels for causation and time and for nothing else.",
     "Reported by the human partner while reviewing Scatterhoard's link vocabulary.", []),

    ("direction-fixed-by-creation-order", "2026-08-01", "13:35", "llm", 0.8, ["links", "vocabulary", "principle"],
     "A link label needs a reverse only when the new memory's side is genuinely free, which is true just for causation and temporal order - the two relations about the world's chronology rather than the memories' own.",
     "A memory cannot cite, replace, or resolve something that did not exist when it was written, so for those relations creation order fixes the direction and the reverse label names a case that cannot arise.",
     None,
     [("RESULTS_FROM", "outbound-rule"), ("SUPPORTS", "outbound-rule")]),

    ("identifiers-cost-more-tokens", "2026-08-01", "13:50", "llm", 0.7, ["conventions", "budget", "tokens"],
     "Identifiers cost roughly a third more tokens per character than prose (4.14 against 5.52 chars per token on a WordPiece tokenizer), so a character budget systematically under-prices a label vocabulary.",
     "The catalog budget is a character count used as a token proxy, and it gets more permissive in token terms the more the catalog shifts from prose toward tables of identifiers.",
     "Measured while sizing the link vocabulary; the tokenizer is not Claude's, so the ratio transfers and the absolute numbers do not.",
     [("CONTRADICTS", "catalog-char-budget")]),

    ("catalog-char-budget", "2026-07-31", "09:00", "llm", 0.6, ["conventions", "budget"],
     "The convention catalog is capped at 8,000 characters as a deliberately crude proxy for tokens, at roughly four characters each.",
     "Conventions are loaded into every session, so growth past the budget should be a decision someone makes rather than drift nobody notices.",
     None, []),

    ("three-column-table", "2026-08-01", "14:00", "llm", 0.75, ["links", "vocabulary", "economy"],
     "Serving the link vocabulary as a three-column table with a reverse column costs 9.0 tokens per label against 14.3 for a two-column table, covering 18 labels in fewer tokens than 12.",
     "The reverse column is one identifier per row and needs no second reading, so orthogonality in the vocabulary converts directly into context economy.",
     None,
     [("SUPPORTS", "outbound-rule"), ("RELATED_TO", "identifiers-cost-more-tokens")]),
]

by_key = {m[0]: m for m in M}
order = {m[0]: (m[1], m[2]) for m in M}

# Apply the second-pass revisions in place.
_revised = {(s, t, old): new for s, t, old, new, _ in REVISIONS}
_hit = set()
for m in M:
    links = m[9]
    for j, (lbl, tgt) in enumerate(links):
        if (m[0], tgt, lbl) in _revised:
            _hit.add((m[0], tgt, lbl))
            links[j] = (_revised[(m[0], tgt, lbl)], tgt)
missing = set(_revised) - _hit
assert not missing, f"revisions that matched no edge: {missing}"

# Each memory's id is computed once and reused. uuid7() draws fresh randomness per call, so
# generating an id here and again for a link target produced two different UUIDs -- the links
# pointed at memories that did not exist. Caught by the store, which actually resolves them.
ids = {key: uuid7(ts(day, hhmm)[0]) for key, day, hhmm, *_ in M}

records = []
for key, day, hhmm, origin, conf, kw, content, why, context, links in M:
    ms, iso = ts(day, hhmm)
    # Field order mirrors the Rust `Memory` declaration, so a regenerated corpus diffs cleanly
    # against a file the tool itself wrote. Scope is deliberately absent: the file *is* the set,
    # and it is declared once in the header.
    stamp = {"by": "jtcours", "at": iso}
    rec = {
        "id": ids[key],
        "content": content,
        "why": why,
        **({"context": context} if context else {}),
        "keywords": kw,
        "confidence": conf,
        "origin": origin,
        "created": stamp,
        "modified": dict(stamp),
        # Labels go out in the wire form the Rust `LinkLabel` serialises to (snake_case), and a
        # same-set target is a bare id, matching `LinkTarget::SameSet`. Keeping the file in the real
        # format is the point: it is meant to be importable once the store exists.
        "links": [
            {"label": lbl.lower(), "target": ids[tgt]} for lbl, tgt in links
        ],
        "_key": key,
        "_link_keys": [[lbl, tgt] for lbl, tgt in links],
    }
    records.append((ms, rec))

# ids are UUIDv7, so sorting by id is sorting by creation time -- the property `uuidv7` records.
records.sort(key=lambda r: r[1]["id"])

out_dir = os.path.dirname(os.path.abspath(__file__))
os.makedirs(out_dir, exist_ok=True)
path = os.path.join(out_dir, "design-memories.jsonl")
with open(path, "w") as f:
    header = {"format_version": "0.1.0", "scope": {"shared": "scatterhoard"}}
    f.write(json.dumps(header) + "\n")
    for _, rec in records:
        f.write(json.dumps(rec) + "\n")

# ---- analysis -------------------------------------------------------------

edges = []
for _, rec in records:
    for lbl, tgt in rec["_link_keys"]:
        edges.append((rec["_key"], lbl, tgt))

print(f"{len(records)} memories, {len(edges)} edges -> {path}\n")

counts = {}
for _, lbl, _ in edges:
    counts[lbl] = counts.get(lbl, 0) + 1

print("label usage:")
for lbl in PROPOSED_LABELS:
    n = counts.get(lbl, 0)
    flag = "   <-- UNUSED" if n == 0 else ""
    print(f"  {lbl:16s} {n:2d}{flag}")

print("\noutbound-rule check (does `from` post-date `to` for the one-directional labels?):")
violations = 0
for src, lbl, tgt in edges:
    if lbl not in ONE_DIRECTIONAL:
        continue
    if order[src] < order[tgt]:
        print(f"  VIOLATION  {src} --{lbl}--> {tgt}  (from is OLDER than to)")
        violations += 1
print(f"  {violations} violation(s) out of "
      f"{sum(1 for _, l, _ in edges if l in ONE_DIRECTIONAL)} one-directional edges")

print("\nfree-direction labels, which side is the newer memory on?:")
for src, lbl, tgt in edges:
    if lbl not in FREE:
        continue
    rel = "newer" if order[src] > order[tgt] else "OLDER"
    print(f"  {src:32s} --{lbl:12s}--> {tgt:32s}  from is {rel}")
