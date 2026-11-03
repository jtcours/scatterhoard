"""Recover the `hoard_recall` queries a session treated as answerable *for one set*.

`harvest_queries.py` recovers every real query. This asks a narrower question, the one a floor needs:
which of them did the session behave as though the set could answer? Two signals, both visible in a
transcript and both produced by ordinary work rather than by anyone labelling anything:

- **scope** -- the recall named this set, so the session meant to search it;
- **used** -- a memory the recall returned from this set was mentioned in a later call in the same
  session: looked up by id, linked to, superseded, or passed back as `already_known`.

`scope` is the weaker claim -- naming a set says where the session looked, not that the set answered
-- and it is deliberately kept, because it is the one signal a floor cannot suppress. Once a floor
silences a query nothing comes back, so nothing can be used afterwards and `used` disappears; the
scoped ones remain, and a scoped query scoring below the floor is the contradiction that lowers it.

Run it per set, writing `corpus/labelled-queries-<set>.json`:

    python3 corpus/label_queries.py scatterhoard
    python3 corpus/label_queries.py coding-lessons

Reads this machine's transcripts under `~/.claude/projects/*scatterhoard*/` and the live store, for
ids. Both are local, so the output is the portable part and is committed beside the other corpora.
"""

import glob
import json
import os
import re
import sqlite3
import sys

SET_NAME = sys.argv[1] if len(sys.argv) > 1 else "scatterhoard"
HERE = os.path.dirname(os.path.abspath(__file__))
MEMORY_ID = re.compile(r"\b01[0-9A-HJKMNP-TV-Z]{24}\b")


def ids_in_the_set(name):
    """Every memory id the set holds, read from the live store read-only."""
    stem = name if name == "local" else f"shared-{name}"
    path = os.path.expanduser(f"~/.local/share/scatterhoard/{stem}.sqlite3")
    connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    try:
        return {row[0] for row in connection.execute("SELECT id FROM memories")}
    finally:
        connection.close()


def blocks_of(path):
    """Tool calls in file order, and tool results keyed by the call they answer."""
    calls, results = [], {}
    for line in open(path, encoding="utf-8"):
        try:
            row = json.loads(line)
        except ValueError:
            continue
        message = row.get("message") or {}
        content = message.get("content")
        if not isinstance(content, list):
            continue
        for block in content:
            if block.get("type") == "tool_use":
                calls.append((len(calls), block, row.get("timestamp"), row.get("sessionId")))
            elif block.get("type") == "tool_result":
                body = block.get("content")
                if isinstance(body, list) and body and isinstance(body[0], dict):
                    results[block.get("tool_use_id")] = body[0].get("text") or ""
                elif isinstance(body, str):
                    results[block.get("tool_use_id")] = body
    return calls, results


def main():
    ours = ids_in_the_set(SET_NAME)
    earliest = {}

    for path in sorted(glob.glob(os.path.expanduser("~/.claude/projects/*scatterhoard*/*.jsonl"))):
        calls, results = blocks_of(path)
        for index, block, timestamp, session in calls:
            if not (block.get("name") or "").endswith("hoard_recall"):
                continue
            arguments = block.get("input") or {}
            query = arguments.get("query")
            if not query:
                continue  # a keyword search, which no floor applies to

            label = None
            scope = arguments.get("scope")
            if scope is not None and SET_NAME in json.dumps(scope):
                label = "scoped"
            if label is None:
                returned = set(MEMORY_ID.findall(results.get(block.get("id"), ""))) & ours
                if returned:
                    for later, other, _, _ in calls:
                        if later <= index:
                            continue
                        if returned & set(MEMORY_ID.findall(json.dumps(other.get("input") or {}))):
                            label = "used"
                            break
            if label is None:
                continue

            row = {
                "query": query,
                "first_seen": timestamp,
                "session": (session or "")[:8],
                "label": label,
            }
            if query not in earliest or (timestamp or "") < (earliest[query]["first_seen"] or ""):
                earliest[query] = row

    queries = sorted(earliest.values(), key=lambda row: row["first_seen"] or "")
    output = {
        "provenance": "labelled from ~/.claude/projects/*scatterhoard*/*.jsonl by corpus/label_queries.py: "
                      "the recall named this set, or a memory it returned from this set was used later "
                      "in the same session",
        "store": "local" if SET_NAME == "local" else f"shared:{SET_NAME}",
        "queries": queries,
    }
    path = os.path.join(HERE, f"labelled-queries-{SET_NAME}.json")
    with open(path, "w", encoding="utf-8") as out:
        json.dump(output, out, indent=1, ensure_ascii=False)

    counts = {}
    for row in queries:
        counts[row["label"]] = counts.get(row["label"], 0) + 1
    span = f"{queries[0]['first_seen'][:10]} to {queries[-1]['first_seen'][:10]}" if queries else "-"
    print(f"{SET_NAME}: {len(queries)} labelled {counts}, {span} -> {path}")


if __name__ == "__main__":
    main()
