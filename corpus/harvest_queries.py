"""Recover real `hoard_recall` queries from this project's own session transcripts.

The traversal corpus has memories and links but no queries, and `recall-eval-harness` in TRACKING.md
requires that whoever authors queries be **blind to the link structure** -- otherwise the evaluation
measures the author rather than the mechanism.

Harvesting solves that better than authoring does. These queries were issued during real work,
against the live memory sets, by sessions that had no idea a corpus would ever be built from the
design documents. They are blind by *construction* rather than by procedure, which is the same move
the design makes everywhere else: make the wrong thing unrepresentable rather than forbidden.

## The cutoff is the whole safeguard

`corpus/build_traversal_corpus.py` was written on 2026-08-15. A query issued before that date cannot
have been influenced by the corpus, because the corpus did not exist. A query issued on or after it
was issued by a session that may have had the corpus, its links, or its measurements in context --
including the sessions that built it.

So CUTOFF is not a tidiness filter and must not be relaxed to gain a few more queries. Every query
it excludes is one whose independence cannot be established.

## What this deliberately does not take

**Not the results.** Transcripts do capture what recall returned, but those results are against the
*live* sets, whose memories are a different population with different ids. They cannot serve as
relevance labels for this corpus, and treating them as if they could would silently score one corpus
against another's answers.

**Not the transcripts.** Only the query strings, their timestamps, and a session prefix for
provenance. A session transcript is a working record containing whatever was being worked on; the
queries are the part this project has a reason to keep, and taking only them is what keeps this file
safe to commit.

Run: python3 corpus/harvest_queries.py
"""

import argparse
import glob
import json
import os

# Sources: the archive the human partner keeps, and the live directory it is copied from. Both are
# read because the archive is a snapshot and goes stale -- it was two sessions behind when this was
# written -- while the live directory holds sessions the cutoff below then excludes anyway.
# The project directory name is part of the path, so **moving the checkout renames the transcript
# directory** and a path naming one location silently stops seeing the other. That happened: this
# listed only `Documents/...` while the checkout had moved to `git/...`, so the twenty most recent
# sessions were invisible and the harvest looked like it had worked. Globbed on the project name
# instead, which survives the next move.
SOURCES = [
    os.path.expanduser("~/Downloads/scatterhoard_chats/raw/*.jsonl"),
    os.path.expanduser("~/.claude/projects/*scatterhoard*/*.jsonl"),
]

# The traversal corpus was built 2026-08-15. See the module docstring: this is the safeguard, not a
# filter, and relaxing it to gain queries defeats the reason for harvesting at all.
CUTOFF = "2026-08-15"

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "harvested-queries.json")

# A cutoff is independence from *one particular thing*, and the default is independence from the
# traversal corpus. A different question wants a different date, which is why `--cutoff` and `--out`
# exist -- and why they are arguments rather than an edit to the constant above, so that relaxing
# the default stays as hard as the docstring says it should be.
#
# The live instance: the non-match floor was designed 2026-09-09, and the sessions that studied the
# problem ran from 2026-09-07. A query issued before that date was issued by a session with no idea
# a floor would exist, which is the independence a floor's evaluation needs -- and it is three weeks
# later than the traversal corpus's date, because the traversal corpus is not what it must be blind
# to. State which question a harvest is for, or the date means nothing.


def tool_calls(path):
    """Yield (timestamp, tool name, input) for every tool call in a transcript."""
    with open(path) as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            content = (entry.get("message") or {}).get("content")
            if not isinstance(content, list):
                continue
            for block in content:
                if isinstance(block, dict) and block.get("type") == "tool_use":
                    yield entry.get("timestamp"), block.get("name", ""), block.get("input") or {}


parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
parser.add_argument(
    "--cutoff",
    default=CUTOFF,
    help="ISO date; calls on or after it are excluded. Defaults to the traversal corpus's date.",
)
parser.add_argument("--out", default=OUT, help="where to write the harvest")
parser.add_argument(
    "--why",
    default="the traversal corpus was built then",
    help="what this cutoff establishes independence from, recorded in the file's provenance",
)
args = parser.parse_args()
CUTOFF, OUT = args.cutoff, args.out

paths = {}
for pattern in SOURCES:
    for path in glob.glob(pattern):
        paths.setdefault(os.path.basename(path), path)

semantic, keyword, excluded = {}, {}, []
for name, path in sorted(paths.items()):
    session = name[:8]
    for timestamp, tool, arguments in tool_calls(path):
        if "hoard_recall" not in tool:
            continue
        stamp = timestamp or ""
        query, keywords = arguments.get("query"), arguments.get("keywords")

        if stamp[:10] >= CUTOFF:
            excluded.append((stamp[:10], session, str(query or keywords)))
            continue

        if query:
            # Keep the earliest sighting: a query re-issued later was still first asked then.
            if query not in semantic or stamp < semantic[query]["first_seen"]:
                semantic[query] = {"query": query, "first_seen": stamp, "session": session}
        elif keywords:
            key = tuple(keywords)
            if key not in keyword or stamp < keyword[key]["first_seen"]:
                keyword[key] = {
                    "keywords": list(keywords),
                    "first_seen": stamp,
                    "session": session,
                }

queries = sorted(semantic.values(), key=lambda row: row["first_seen"])
keywords = sorted(keyword.values(), key=lambda row: row["first_seen"])

with open(OUT, "w") as handle:
    json.dump(
        {
            "provenance": (
                "Real hoard_recall calls issued during development, recovered from session "
                f"transcripts. Only calls made before {CUTOFF} are included -- {args.why} -- so "
                "none of these could have been influenced by it. Relevance labels are NOT here."
            ),
            "cutoff": CUTOFF,
            "sessions_scanned": len(paths),
            "queries": queries,
            "keyword_searches": keywords,
        },
        handle,
        indent=1,
    )
    handle.write("\n")

print(f"{len(paths)} transcripts scanned")
print(f"{len(queries)} distinct semantic queries, {len(keywords)} distinct keyword searches -> {OUT}")
print(f"{len(excluded)} call(s) excluded as on or after {CUTOFF}:")
for day, session, what in sorted(excluded):
    print(f"  [{day}] {session}  {what}")
