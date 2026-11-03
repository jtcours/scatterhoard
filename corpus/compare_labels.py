"""Compare two independent labellings, leading with the policies rather than the memories.

The human partner reviews through a chat interface, and the first version of this file was built
around that: every memory printed in full, because an id means nothing to them and a truncation
cannot be expanded. That constraint still holds and is still honoured below.

**What changed is what the review is for.** Adjudicating individual memories at the edge of relevance
is inherently noisy work -- it asks a reviewer to place a sharp boundary on a smooth gradation, and
produces a lot of judgement for very little signal. Reviewing the *policies* two labellers applied is
where a human is far more effective: a policy choice is legible, arguable, and decides many labels at
once.

So this leads with each labeller's stated policy and its per-query reading, and reaches for the
memories themselves only where the two disagree enough that the text is needed to settle it.

## The borderline band, and what it buys

Each labeller separates what it would defend (`relevant`) from what it judged to be at the edge
(`borderline`). That is not a finer-grained answer key; it is a **sensitivity test**. The eval can be
run against the strict set and again against strict-plus-borderline, and if the ranking of
score-combination rules is the same under both, the boundary noise provably does not affect the
conclusion. If it differs, that is a finding, and it arrives as one fact rather than as a hundred
adjudications.

Run: python3 corpus/compare_labels.py
"""

import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
WORKSHEET = os.path.join(HERE, "labelling-worksheet.json")
LABELS = (os.path.join(HERE, "labels-a2.json"), os.path.join(HERE, "labels-b2.json"))
OUT = os.path.join(HERE, "label-review.md")

# Below this, the two labellers disagree enough that the memories are worth printing.
NEEDS_TEXT = 0.60


def load(path):
    with open(path) as handle:
        return json.load(handle)


def by_query(labelling):
    return {entry["q"]: entry for entry in labelling["queries"]}


def jaccard(left, right):
    union = left | right
    return len(left & right) / len(union) if union else None


worksheet = load(WORKSHEET)
memories = {memory["id"]: memory for memory in worksheet["memories"]}
a, b = load(LABELS[0]), load(LABELS[1])
qa, qb = by_query(a), by_query(b)

out = ["# Label review\n"]
out.append(
    "\nTwo labellers worked independently, on different models, from a view of the corpus with the "
    "link structure withheld.\n"
)
out.append(
    "\n**Read the policies first.** They decide many labels each, and they are the part worth "
    "arguing with. The per-memory text below is only printed where the two disagree enough that "
    "reading it is the only way to settle the question.\n"
)

out.append("\n---\n\n## The policies each labeller applied\n")
for name, labelling in (("A", a), ("B", b)):
    out.append(f"\n### Labeller {name}\n")
    out.append(f"\n{labelling.get('policies', '*none stated*')}\n")

# ---------------------------------------------------------------------------
# Per-query summary, which is where a policy difference becomes visible
# ---------------------------------------------------------------------------
rows = []
for entry in worksheet["queries"]:
    query = entry["q"]
    ra = set(qa.get(query, {}).get("relevant", []))
    rb = set(qb.get(query, {}).get("relevant", []))
    ba = set(qa.get(query, {}).get("borderline", []))
    bb = set(qb.get(query, {}).get("borderline", []))
    rows.append(
        {
            "q": query,
            "ra": ra,
            "rb": rb,
            "ba": ba,
            "bb": bb,
            "strict": jaccard(ra, rb),
            "permissive": jaccard(ra | ba, rb | bb),
            "reading_a": qa.get(query, {}).get("reading", ""),
            "reading_b": qb.get(query, {}).get("reading", ""),
        }
    )

out.append("\n---\n\n## Where the two agree, query by query\n")
out.append(
    "\n`strict` compares what each would defend. `permissive` adds each one's borderline band. A "
    "large gap between them means the disagreement is about *where the edge is*, not about what "
    "the query means.\n"
)
out.append("\n| strict | permissive | A rel/bord | B rel/bord | query |\n")
out.append("|---:|---:|---:|---:|---|\n")
for row in sorted(rows, key=lambda r: (r["strict"] is not None, r["strict"])):
    strict = "--" if row["strict"] is None else f"{row['strict']:.0%}"
    perm = "--" if row["permissive"] is None else f"{row['permissive']:.0%}"
    out.append(
        f"| {strict} | {perm} | {len(row['ra'])}/{len(row['ba'])} | "
        f"{len(row['rb'])}/{len(row['bb'])} | {row['q'][:58]} |\n"
    )

# ---------------------------------------------------------------------------
# The readings, which is where a policy difference is stated rather than inferred
# ---------------------------------------------------------------------------
out.append("\n---\n\n## What each labeller took the query to mean\n")
out.append(
    "\nWhere these differ, the labels differ for a reason that can be settled by choosing a "
    "reading -- which is one decision, not many.\n"
)
for row in sorted(rows, key=lambda r: (r["strict"] is not None, r["strict"])):
    out.append(f"\n### {row['q']}\n")
    strict = "--" if row["strict"] is None else f"{row['strict']:.0%}"
    out.append(f"\n*strict overlap {strict}*\n")
    out.append(f"\n- **A read it as:** {row['reading_a'] or '*no reading given*'}\n")
    out.append(f"- **B read it as:** {row['reading_b'] or '*no reading given*'}\n")

# ---------------------------------------------------------------------------
# The memories, only where the readings do not settle it
# ---------------------------------------------------------------------------
disputed = [
    row
    for row in rows
    if row["strict"] is not None and row["strict"] < NEEDS_TEXT and (row["ra"] ^ row["rb"])
]
out.append("\n---\n\n## Memories worth reading\n")
out.append(
    f"\nOnly the {len(disputed)} queries where the two agree on less than {NEEDS_TEXT:.0%} of what "
    "they would defend. Everything else is settled by the policies above.\n"
)
for row in disputed:
    out.append(f"\n### {row['q']}\n")
    for title, ids in (
        ("Only A would defend", sorted(row["ra"] - row["rb"])),
        ("Only B would defend", sorted(row["rb"] - row["ra"])),
    ):
        if not ids:
            continue
        out.append(f"\n#### {title} ({len(ids)})\n")
        for memory_id in ids:
            memory = memories.get(memory_id)
            if memory is None:
                out.append(f"\n- unknown id `{memory_id}`\n")
                continue
            out.append(f"\n**`{memory_id[:8]}`** — {memory['content']}\n")
            if memory.get("why"):
                out.append(f"\n*Why:* {memory['why']}\n")

with open(OUT, "w") as handle:
    handle.write("".join(out))

strict_all = [row["strict"] for row in rows if row["strict"] is not None]
perm_all = [row["permissive"] for row in rows if row["permissive"] is not None]
mean = lambda values: sum(values) / len(values) if values else 0.0
print(f"{len(rows)} queries")
print(f"strict agreement     mean {mean(strict_all):.0%}")
print(f"permissive agreement mean {mean(perm_all):.0%}")
print(f"relevant: A {sum(len(r['ra']) for r in rows)}, B {sum(len(r['rb']) for r in rows)}")
print(f"borderline: A {sum(len(r['ba']) for r in rows)}, B {sum(len(r['bb']) for r in rows)}")
print(f"{len(disputed)} queries need their memories read")
print(f"-> {OUT}")
