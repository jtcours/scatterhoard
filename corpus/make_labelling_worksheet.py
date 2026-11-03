"""Emit a view of the traversal corpus that cannot show its link structure.

`recall-eval-harness` in TRACKING.md requires that whoever labels relevance be **blind to the links**,
or the evaluation measures the labeller rather than the mechanism. The queries were solved by
harvesting real ones (`corpus/harvest_queries.py`); the labels still have to be authored, and this is
what makes authoring them blind a property of the artifact rather than of anyone's discipline.

The reasoning is the one `documents/design/recall-mechanisms.md` applies to the reverse index: *"must
not emit it" as a filter is a step someone forgets*, so the exported type should not have the field
at all. A labeller working from this file is not resisting a temptation; there is nothing here to
resist.

## Three ways the structure leaks, not one

**The `links` field.** The obvious one, and the only one a "strip the links" script would catch.

**The `context` field**, which in this corpus reads *Extracted from documents/design/X.md*. Roughly
three quarters of the edges are within-document, so naming the source document tells a labeller most
of what the link structure would have. Dropped.

**The record order.** Ids are UUIDv7 over synthetic timestamps assigned per document, so id order
groups by document -- 429 records form 91 contiguous same-document runs. A labeller scrolling the
file in that order sees the documents reassembled even with `context` gone. Shuffled.

## An allowlist, because a denylist fails open

Records are built from the four permitted fields rather than by deleting the forbidden ones. A
denylist is correct only against the fields that exist when it is written: add a field to `Memory`
that carries structure and a stripping script keeps emitting it, silently, while still passing every
test it has. The allowlist below fails the other way -- a new field is absent until someone adds it
here deliberately.

## The shuffle is seeded, and that is safe here

A fixed seed buys reproducibility rather than stability, and this project has already been bitten by
that: ids drawn from one seeded stream shifted whenever a record was inserted. **That hazard is about
identities, and an ordering is not one.** Reordering changes no id, and every record carries its own
id, so a reader can always recover which memory is which. What the seed buys is that two people
labelling from this file are looking at the same file.

Run: python3 corpus/make_labelling_worksheet.py
"""

import json
import os
import random

HERE = os.path.dirname(os.path.abspath(__file__))
CORPUS = os.path.join(HERE, "traversal-corpus.jsonl")
QUERIES = os.path.join(HERE, "harvested-queries.json")
OUT = os.path.join(HERE, "labelling-worksheet.json")

# The only fields a relevance labeller needs, and the shape `corpus/seed-quality-corpus.json`
# already uses -- so one labelled-corpus format serves both and the harness can read either.
PERMITTED = ("id", "content", "why", "keywords")

SHUFFLE_SEED = 20260815

# Queries dropped from the scored set, with the reason, because a silent exclusion is a thumb on
# the scale that leaves no trace. Each of these produced **zero overlap** between two independent
# labellers -- not disagreement at the margin, but no memory in common at all -- and in each case
# the cause is the same: the corpus has no crisp counterpart for what was asked, so there is no
# determinate relevant set to agree on.
#
# A query with no determinate answer contributes noise to every mechanism equally, so keeping it
# would add variance without adding discrimination. Five entries, four distinct questions: the
# "reduced detail on the wire" pair is one question asked twice.
EXCLUDED = {
    "Scatterhoard design decisions and project state":
        "An orientation request issued at session start rather than a retrieval request. Nearly "
        "every memory in the corpus is a design decision, so no determinate relevant set exists.",
    "scatterhoard design decisions and constraints":
        "The same orientation request, differently worded.",
    "protocol channel discipline":
        "A phrase the asker coined. Nothing in the corpus is named this, and the two labellers "
        "resolved it to different subjects.",
    "reduced detail on the wire":
        "Ambiguous between the schema-degradation cluster and the open question of how much of a "
        "memory a response returns. The two labellers took different readings and shared no "
        "memory at all.",
    "how should reduced detail on the wire be implemented":
        "The same question asked at greater length, and ambiguous the same way.",
}


def permitted_only(record):
    """Build a record from the permitted fields. Never deletes; only ever selects."""
    return {field: record[field] for field in PERMITTED if field in record}


memories = []
for line in open(CORPUS):
    line = line.strip()
    if not line:
        continue
    record = json.loads(line)
    if "format_version" in record:
        continue
    # A tombstone carries no content to label against.
    if "content" not in record:
        continue
    memories.append(permitted_only(record))

random.Random(SHUFFLE_SEED).shuffle(memories)

harvested = json.load(open(QUERIES))
kept = [row for row in harvested["queries"] if row["query"] not in EXCLUDED]
dropped = [row["query"] for row in harvested["queries"] if row["query"] in EXCLUDED]
assert len(dropped) == len(EXCLUDED), (
    f"an excluded query no longer appears in the harvest: "
    f"{sorted(set(EXCLUDED) - set(dropped))}"
)
queries = [{"q": row["query"], "relevant": []} for row in kept]
# Keyword-search relevance is a **lookup, not a judgement**, so nothing is asked of a labeller.
# `hoard_recall` with keywords does exact matching against a memory's curated keyword list, so the
# relevant set is exactly the memories carrying one of the terms -- computable, and not a matter of
# opinion. Two labellers disagreed on 50 labels here purely because one answered semantically, which
# is a different question from the one the mechanism asks.
#
# Recorded rather than dropped, because it bears on the *hybrid search* open question. It bears on
# nothing about traversal: exact match returns a set with no ranking, so no score-combination rule
# can be distinguished by it.
by_keyword = {}
for memory in memories:
    for word in memory["keywords"]:
        by_keyword.setdefault(word.lower(), []).append(memory["id"])

keyword_searches = []
for row in harvested["keyword_searches"]:
    hits = sorted({
        memory_id
        for term in row["keywords"]
        for memory_id in by_keyword.get(term.lower(), [])
    })
    keyword_searches.append(
        {"keywords": row["keywords"], "relevant": hits, "resolved": "exact match, computed"}
    )

worksheet = {
    "note": (
        "A blind view of corpus/traversal-corpus.jsonl for relevance labelling. Link structure is "
        "absent by construction: records are built from an allowlist of four fields, the source "
        "document is not named, and the order is shuffled because id order groups by document. "
        "Fill each query's `relevant` list with the ids of the memories that query should have "
        "found. Judge relevance from the query and the memory alone. Do not consult "
        "traversal-corpus.jsonl, which carries the links this file exists to withhold. "
        "`keyword_searches` are already resolved by exact match and need no judgement."
    ),
    "corpus": "traversal-corpus.jsonl",
    "shuffle_seed": SHUFFLE_SEED,
    "excluded": [{"q": q, "reason": EXCLUDED[q]} for q in dropped],
    "memories": memories,
    "queries": queries,
    "keyword_searches": keyword_searches,
}

# Mechanical checks, because the value of this file is a claim about what is *not* in it, and that
# is exactly the kind of claim that rots without something asserting it.
#
# The first version of these searched the serialised worksheet for the words "links", "target" and
# "context", and fired immediately -- on prose. This is a corpus *about* a memory system with links,
# so those words are its subject matter: 23 memories discuss links, 37 discuss targets. A check that
# cannot tell the subject from the structure is not a weaker check, it is a different one, and the
# structural questions are the ones worth asking.
for memory in memories:
    assert set(memory) <= set(PERMITTED), f"unexpected field in {memory['id']}"

# The real leak to guard: a link target *is* an id, so structure escapes through any field that
# names another memory. An id may appear as its own `id` and nowhere else.
ids = {memory["id"] for memory in memories}
for memory in memories:
    body = json.dumps({key: value for key, value in memory.items() if key != "id"})
    for other in ids:
        assert other not in body, f"{memory['id']} names {other}"

# `.md` is absent from every memory's prose, so unlike the words above it is a clean signal that the
# source document has come back -- through `context`, or through anything else naming a file.
assert ".md" not in json.dumps(worksheet), "the worksheet names a source document"

# Order must not reconstruct the documents. Nothing here can check that directly once `context` is
# gone, which is the point -- so it is asserted against the corpus, where the grouping is visible.
assert worksheet["memories"][0]["id"] != sorted(ids)[0], "the worksheet is still in id order"

with open(OUT, "w") as handle:
    json.dump(worksheet, handle, indent=1)
    handle.write("\n")

print(f"{len(memories)} memories, {len(queries)} queries, {len(keyword_searches)} keyword searches")
print(f"{len(dropped)} queries excluded with a recorded reason")
print(f"fields emitted: {', '.join(PERMITTED)}")
print(f"-> {OUT}")
