"""Measure whether graph traversal can earn its cost on a given corpus.

The acceptance check to run on any corpus proposed for traversal evaluation, and to run *before*
authoring queries, which is the expensive half. It needs no embedder: the vectors are already in the
store, as raw little-endian f32 blobs in the `vectors` table.

For each link, rank every memory by cosine against the link's source and record where the target
lands -- **as a percentile of the corpus, not as a rank**. The percentile is the scale-invariant
quantity, and reporting rank instead is the trap: at 70 memories a linked target ranking 2nd looks
like traversal is redundant, while the same percentile at 1,000 memories ranks 29th and looks
essential. See `recall-eval-harness` in TRACKING.md.

`supersedes` and `superseded_by` are excluded, because validity resolution never traverses them.

This lives in the repository rather than a scratchpad on purpose. The first version was written to
/private/tmp, which is exactly how the seed-quality corpus came within one sweep of being lost --
see corpus/seed-quality-provenance.md.
"""

import sqlite3, struct, math, json, collections, os

# The live data directory, found the way the server finds it.
BASE = os.environ.get("SCATTERHOARD_HOME") or os.path.expanduser("~/.local/share/scatterhoard")
BASE = os.path.join(BASE, "")
STORES = {"scatterhoard": BASE+"shared-scatterhoard.sqlite3",
          "coding-lessons": BASE+"shared-coding-lessons.sqlite3"}

vecs, content, edges = {}, {}, []
for name, path in STORES.items():
    con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    for mid, blob in con.execute("SELECT memory_id, vector FROM vectors"):
        vecs[mid] = list(struct.unpack(f"<{len(blob)//4}f", blob))
    for mid, c in con.execute("SELECT id, content FROM memories"):
        content[mid] = c
    for src, label, tgt in con.execute("SELECT source, label, target FROM links"):
        edges.append((name, src, label, tgt))
    con.close()

# Union centroid, matching recall_across when both sets are loaded.
dim = len(next(iter(vecs.values())))
cent = [sum(v[i] for v in vecs.values())/len(vecs) for i in range(dim)]
def centred(v): return [a-b for a, b in zip(v, cent)]
C = {k: centred(v) for k, v in vecs.items()}
def cos(a, b):
    d = sum(x*y for x, y in zip(a, b))
    na = math.sqrt(sum(x*x for x in a)); nb = math.sqrt(sum(y*y for y in b))
    return d/(na*nb) if na and nb else 0.0

SKIP = {"supersedes", "superseded_by"}
rows = []
for name, src, label, tgt in edges:
    if label in SKIP or src not in C or tgt not in C:
        continue
    scored = sorted(((cos(C[src], C[m]), m) for m in C if m != src), reverse=True)
    rank = next(i+1 for i, (_, m) in enumerate(scored) if m == tgt)
    rows.append((name, label, rank, cos(C[src], C[tgt]), src, tgt))

rows.sort(key=lambda r: -r[2])
print(f"{'set':15} {'label':12} {'rank':>5} {'cos':>7}   (rank of target when the query lands exactly on the source)")
print("-"*88)
for name, label, rank, c, src, tgt in rows:
    flag = "REDUNDANT" if rank <= 5 else ("marginal" if rank <= 10 else "USEFUL")
    print(f"{name:15} {label:12} {rank:5} {c:7.3f}   {flag}")
print()
n = len(rows)
useful = sum(1 for r in rows if r[2] > 10)
marg   = sum(1 for r in rows if 5 < r[2] <= 10)
red    = sum(1 for r in rows if r[2] <= 5)
print(f"traversable edges measured: {n}")
print(f"  target already in top 5   (traversal redundant): {red}")
print(f"  target ranked 6-10        (marginal):            {marg}")
print(f"  target ranked below 10    (traversal is the only path): {useful}")
print()
print("corpus size:", len(C), "memories")
