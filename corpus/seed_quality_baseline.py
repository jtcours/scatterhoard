"""Transformer baseline for the seed-quality spike.

Answers the one question the model2vec numbers can't: is ~87% r@10 the model's
ceiling or the corpus's? Same corpus, same embedding input, same metric.
"""

import json
import sys

import numpy as np
from sentence_transformers import SentenceTransformer

MODEL = sys.argv[1] if len(sys.argv) > 1 else "sentence-transformers/all-MiniLM-L6-v2"

corpus = json.load(open("corpus.json"))
memories, queries = corpus["memories"], corpus["queries"]

# Same input as the Rust spike: content + why + keywords.
inputs = [f"{m['content']} {m['why']} {' '.join(m['keywords'])}" for m in memories]
ids = [m["id"] for m in memories]

model = SentenceTransformer(MODEL)
mem = model.encode(inputs, normalize_embeddings=True)
qry = model.encode([q["q"] for q in queries], normalize_embeddings=True)


def evaluate(mem, qry, label):
    ks = [1, 3, 5, 10]
    totals = [0.0] * len(ks)
    hubs = {}
    for qi, query in enumerate(queries):
        sims = mem @ qry[qi]
        order = np.argsort(-sims)
        for ki, k in enumerate(ks):
            top = {ids[i] for i in order[:k]}
            found = sum(1 for r in query["relevant"] if r in top)
            totals[ki] += found / len(query["relevant"])
        for i in order[:5]:
            hubs[ids[i]] = hubs.get(ids[i], 0) + 1
    n = len(queries)
    print(
        f"{label:<28} "
        + " ".join(f"{100 * t / n:>7.1f}%" for t in totals)
    )
    return hubs


print(f"model: {MODEL}")
print(f"corpus: {len(memories)} memories, {len(queries)} queries\n")
print(f"{'variant':<28} {'r@1':>8} {'r@3':>8} {'r@5':>8} {'r@10':>8}")

hubs_raw = evaluate(mem, qry, "raw cosine")

centre = mem.mean(axis=0)
mem_d = mem - centre
qry_d = qry - centre
mem_d /= np.linalg.norm(mem_d, axis=1, keepdims=True)
qry_d /= np.linalg.norm(qry_d, axis=1, keepdims=True)
evaluate(mem_d, qry_d, "centroid removed (both)")

top = sorted(hubs_raw.items(), key=lambda kv: -kv[1])[:4]
print(f"\nhubness, top-5 appearances across {len(queries)} queries:")
print(f"  raw: {top}")
