#!/usr/bin/env python3
"""Generate a tracking file's index rows from the exported memory set.

[TRACKING.md](../TRACKING.md) is an index over memory clusters rather than a file
holding the detail itself. Each entry is a *head* memory carrying one of the
file's section keywords; the findings accumulated under it are member memories
carrying the entry's slug, which the head names.

Reading the exported file rather than the store keeps this a plain script with
no Rust change and no context cost -- the export is already committed beside
the code. The cost is that a head stored since the last export is not yet in
the index; export first, then regenerate.

    python3 scripts/genindex.py TRACKING.md                          # every region in the file
    python3 scripts/genindex.py TRACKING.md --check                  # are they current?
    python3 scripts/genindex.py TRACKING.md hoard-proj-issue-entry   # just the one region
    python3 scripts/genindex.py --self-check                         # ordering and attribution

**The keyword argument is optional, and omitting it is the ordinary case.** One
file holds a region per record type, and naming one of them regenerates that
region while leaving the others silently stale -- which is the failure the
unification was supposed to remove, not introduce. With no keyword the script
discovers every region the file declares and does them all.

The rows go **into the file**, between marker comments, because the alternative
-- printing them for a person to paste -- is the commonest cause of defects in
this project's own issues, `a-fact-restated-by-hand-drifts`, re-created on every
regeneration. `--check` is the counterpart `STANDARDS.md` asks for where a copy
exists at all: it rewrites nothing and exits non-zero when the file disagrees
with the memories, so a stale index is caught the way a stale mutation or a
broken link is, by a script that answers in a second. **Every region is checked
before it exits**, rather than stopping at the first stale one, so a run reports
the whole file's disagreement instead of one region's.

A head's keywords carry the whole row. Beside its section keyword
(`hoard-proj-issue-entry`, `-backlog-entry`, `-question-entry`) a head carries
**exactly one**
status keyword sharing that keyword's family prefix -- `hoard-proj-issue-open`,
`-issue-fixed`, `-backlog-built`, `-question-resolved`, `-issue-withdrawn` -- plus
its slug and any facet keywords the section uses. A facet is a second axis that is
not the standing -- `hoard-proj-severity-*` on issues and `hoard-proj-readiness-*`
on backlog entries, where
Ready, Blocked and Deferred all describe unbuilt work and so cannot be status.
Facets render after the slug in FACETS order. The status word is derived
rather than tabulated: the box is unchecked for `open` and checked for anything
else. A state outside ORDINARY_DONE is written into the row as well, so a third
standing like `withdrawn` stays visible instead of flattening into `[x]` -- while
`fixed`, `built` and `resolved` say nothing the box has not already said and are
left out.  Forgetting to add a new family's done word there prints a redundant
word, which is loud and harmless; the reverse would hide a standing.

**The rows are ordered by each entry's earliest head, not its current one.**
A head is superseded whenever an entry's standing changes, and a supersession
mints a later id -- so ordering by the head's own id made a one-character change
of standing rewrite the row's *position*, and the file's order, which the
migration preserved, decayed into order-of-last-edit. Walking `supersedes` links
back to an entry's first version gives a key supersession cannot move, and keeps
the order the one thing it is good for: entries filed in the same stretch sit
together, so a defect found alongside another is read alongside it. Two heads
can share a first version, where an entry was split in two; they are ordered by
their own ids, because the fallback, the order the memory file lists them in,
stopped being id order on 2026-09-21.

Two alternatives were weighed and rejected, and the reasons are here because the
question of whether this order means anything had never been asked anywhere.
Sorting by **slug** is immune to supersession too, but it scatters an entry away
from the work it was found with, discards the filing order for good, and -- the
measurement that settled it -- moved 200 of the 218 rows where this moved 18.
Putting **status** first, so open rows float up, reads well against 16 open
issues in 96: it is rejected because it would put a row's position back under the
control of the thing that changes most often, making the close of an entry move
its row rather than toggle its box, and splitting each stretch's entries apart as
they close.

Note the contrast with an entry's *members*, where id order is load-bearing: a
single-keyword recall returns them id-ascending, and that order is the entry's
narrative. Across *heads* it was incidental, and inherited rather than chosen.

**The causes region is the one that joins two families.** A cause head
(`hoard-proj-cause-entry`, standing `-active` or `-retired`) is ranked by how
many issue heads it explains, with those issues on an indented line beneath it.
An issue is attributed by a link in either direction -- `instance_of` from the
issue, `causes` from the cause -- and from any version of either chain, so an
attribution survives the supersession that marks an issue fixed; see
`attributions`. Ranking by count is deliberate here and rejected everywhere
else: in the entry regions a row's position must not move when its standing
does, while in this one the rank is the information.

**The section a row lands in is its head's keyword, not the heading it sits
under.** That is what lets three record types share one file: a misfiled entry
appears under the wrong index, where it is visible, rather than under the right
index in the wrong part of a file, where it is not.

Five guards, because a generator that edits a file can destroy one:

- **The markers must exist and be well formed.** Both carry the keyword, so a
  file with three regions still holds exactly one `BEGIN` and one `END` bearing
  any given one. Missing or crossed, the script refuses rather than guessing
  where the region is -- and an `END` that names a different region cannot be
  mistaken for this one's, which is what an untagged `END` would have allowed
  the moment a second region appeared. Everything outside the markers -- the
  headers and the severity and readiness tables -- is hand-written and never
  touched.
- **A regeneration that would remove rows stops** unless `--allow-removals` is
  given. The innocent cause of a shrinking index is an entry genuinely retired,
  which is rare; the likely cause is an export older than the store, since this
  reads the exported file. Additions and edits apply silently.
- **A head whose standing is ambiguous stops the run.** No status keyword, or
  two, and the script refuses by id rather than picking one -- an entry silently
  rendered open when it is fixed is worse than no row, because it reads as
  tracked work that does not exist.
- **A head with no slug stops the run**, for the same reason: the slug is what
  gathers the entry's findings, so a row without one points at nothing. No
  keyword beginning `hoard-` is a slug, whatever order the head lists it in, so a
  watch mark on a head cannot title its row.
- **Git is the backstop**, and `export-before-commit` already requires reading
  the export diff before committing.

`--self-check` asks whether the ordering itself still holds, in no time at all
and without reading the memory file. It covers the two chain shapes the real file
does not currently contain -- an ancestor pruned from the file, and a supersession
cycle -- because their symptom is a row that moves for no visible reason, which
is indistinguishable from the defect this ordering was written to fix. It also
checks slug selection with the reserved mark listed first, since that is the
order a passing case would otherwise depend on.
"""

import datetime
import json
import pathlib
import re
import sys

MEMORIES = pathlib.Path(__file__).resolve().parent.parent / "scatterhoard_mem" / "scatterhoard.json"

# Memory ids are ULIDs in Crockford base32: the first ten characters are milliseconds since 1970.
CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"

# The projection the tool applies on `detail: "summary"`, reimplemented here for one reason: this
# script reads a file, not the server. Kept deliberately short so the divergence is visible; if the
# rows ever disagree with what a summary recall shows, this is the copy that is wrong.
CAP = 120
CUT = "…"


def summary(text):
    line = " ".join(text.split())
    if len(line) <= CAP:
        return line
    room = CAP - 1
    for at in range(room - 1, -1, -1):
        if line[at] in ".!?" and line[at + 1] == " ":
            return line[: at + 1] + CUT
    cut = room if line[room] == " " else (line.rfind(" ", 0, room) + 1 or room + 1) - 1
    return line[:cut] + CUT


def load():
    """[`parse`] applied to the exported memory file."""
    return parse(MEMORIES.read_text().splitlines())


def parse(lines):
    """The current records, and every record by id -- the second for walking supersession chains.

    A superseded record is still in the file, and is not a row; it is read only for the
    `supersedes` link it carries, which is one step of the chain back to an entry's first version.

    **A tombstone is a step too, and never a row.** `forget` deletes a record's links with it and
    keeps the supersessions in the tombstone's `ancestry`, so a forgotten chain middle still joins
    its successor to its predecessor. Skipping tombstones, as this did until 2026-10-19, read that
    as the chain ending, and the entry's first version came back as an open row.
    """
    records, superseded = {}, set()
    for raw in lines:
        raw = raw.strip()
        if not raw or raw.startswith('{"set"'):
            continue
        record = json.loads(raw)
        if "content" not in record:
            if "ancestry" not in record:
                continue
            record = {"id": record["id"], "links": record["ancestry"]}
        records[record["id"]] = record
        for link in record.get("links", []):
            if link["label"] == "supersedes" and isinstance(link["target"], str):
                superseded.add(link["target"])
    return [r for i, r in records.items() if i not in superseded and "content" in r], records


def earliest_ancestor(rid, records):
    """The id of an entry's first version: the oldest id reachable by `supersedes` links.

    Ids are ULIDs, so the oldest is the smallest, and taking the minimum over everything the walk
    reaches needs no special case for the two shapes that would otherwise want one -- a head that
    supersedes two entries, and a cycle, which is a broken history `hoard_lookup` reports and no
    index can repair. A link target absent from the file still counts: the id is what the position
    is made of, and reading it from the link means pruning a superseded record cannot move a row.
    """
    seen, frontier, earliest = set(), [rid], rid
    while frontier:
        at = frontier.pop()
        if at in seen:
            continue
        seen.add(at)
        earliest = min(earliest, at)
        record = records.get(at)
        if record is None:
            continue
        frontier.extend(link["target"] for link in record.get("links", [])
                        if link["label"] == "supersedes" and isinstance(link["target"], str))
    return earliest


# The done words that say nothing the checked box has not: any other state is spelled out beside it.
ORDINARY_DONE = {"fixed", "built", "resolved"}

# Second axes, rendered in this order. Not the standing: an entry can be `hoard-proj-readiness-deferred`
# and open, or `hoard-proj-severity-corrupts` and fixed, so folding either into the checkbox would say
# the entry does not.
FACETS = ("hoard-proj-severity-", "hoard-proj-readiness-")

# Every control mark begins with this, facets included, so no slug can.
RESERVED = "hoard-"

# The two families the causes region joins. A cause's standing is `-active` or `-retired`; a retired
# cause keeps its row, checked, for the reason a fixed issue does -- it is evidence about where
# defects came from.
ISSUE = "hoard-proj-issue-entry"
CAUSE = "hoard-proj-cause-entry"

BEGIN = "<!-- BEGIN GENERATED INDEX: {} -->"
END = "<!-- END GENERATED INDEX: {} -->"
DECLARED = re.compile(r"<!-- BEGIN GENERATED INDEX: (\S+) -->")


def declared_in(text):
    """Every region the file declares, in the order they appear."""
    return DECLARED.findall(text)


def region(text, keyword):
    """The span between the markers, refusing anything ambiguous."""
    begin, end = BEGIN.format(keyword), END.format(keyword)
    if text.count(begin) != 1 or text.count(end) != 1:
        sys.exit(
            f"expected exactly one {begin!r} and one {end!r}; found "
            f"{text.count(begin)} and {text.count(end)}. Refusing to guess where the region is."
        )
    start, stop = text.index(begin) + len(begin), text.index(end)
    if stop < start:
        sys.exit("the end marker comes before the begin marker; refusing to write.")
    return start, stop


def slugs_in(block):
    return {line.split("`")[1] for line in block.splitlines() if line.startswith("- [") and "`" in line}


def slug_candidates(keywords):
    """Every keyword on a head that could be its slug, in the order the head carries them.

    The whole `hoard-` prefix is excluded rather than a list of marks, because `keyword-standards`
    reserves it for control marks -- section, status, facet, watch mark -- and so no slug may begin
    with it. A list would leave the next mark to be picked as a slug; the prefix needs no change.
    Before this, only the section and status keywords and the facets were excluded, so a head
    carrying `hoard-investigation` ahead of its slug was titled by the watch mark.

    One slug did begin `hoard-`, filed before the write path refused unknown reserved terms, and
    was renamed with an `about-` prefix rather than excepted here.
    """
    return [k for k in keywords if not k.startswith(RESERVED)]


def row_order(head, records):
    """A head's place in its index: its entry's first version, then its own id to break a tie.

    The tie is real -- an entry split in two leaves two heads with one first version -- and without
    the second key the sort fell back to the order the file lists them, which since 2026-09-21 is a
    hash of each id rather than the id. Its own id reproduces the order the file used to give.
    """
    return (earliest_ancestor(head["id"], records), head["id"])


def standing(head, keyword):
    """A head's state word and slug, refusing a head that has not exactly one of each to give.

    Shared by every region, so an entry and a cause are held to one rule about what makes a row.
    """
    keywords = head.get("keywords", [])
    family = keyword.rsplit("-", 1)[0]
    status = [k for k in keywords if k.startswith(family + "-") and k != keyword]
    if len(status) != 1:
        sys.exit(
            f"{head['id']}: expected exactly one `{family}-` status keyword beside "
            f"`{keyword}`; found {status or 'none'}. Refusing to guess this entry's standing."
        )
    candidates = slug_candidates(keywords)
    if not candidates:
        sys.exit(f"{head['id']}: no slug keyword beside `{keyword}` and `{status[0]}`.")
    # Positional among what survives, and deliberately so: 29 of 222 heads carried subject terms
    # beside their slug when this was written, listed after it, so refusing a second candidate
    # would refuse established practice. Telling the two apart is
    # `a-slug-that-is-also-a-subject-keyword-over-gathers`, not this.
    return status[0][len(family) + 1:], candidates[0]


def head_of(current, records):
    """Every record id in a current head's supersession chain, mapped to that head's id."""
    heads = {}
    for record in current:
        seen, frontier = set(), [record["id"]]
        while frontier:
            rid = frontier.pop()
            if rid in seen:
                continue
            seen.add(rid)
            heads[rid] = record["id"]
            earlier = records.get(rid, {}).get("links", [])
            frontier += [l["target"] for l in earlier
                         if l["label"] == "supersedes" and isinstance(l["target"], str)]
    return heads


def attributions(current, records):
    """Which current issue heads each current cause head explains.

    **A link in either direction attributes**: `instance_of` from an issue to a cause, which is
    what a session filing an issue writes, or `causes` from a cause to an issue, which is how a
    cause takes on issues filed before it existed without superseding every one of them. Both at
    once count once.

    **Only the current version of either chain speaks**, since 2026-09-24: an out-link stays with
    the record that wrote it, the rule the walk in `src/store.rs` follows too, so a supersession
    that does not restate an attribution has withdrawn it. Until then any version counted, because
    a supersession dropped links silently -- `hoard_memorize` now reports what one leaves behind
    (`link_delta`), and the 78 attributions standing that day were all restated before the rule
    changed (`out-links-stay-with-the-record-that-wrote-them`). The *target* still resolves through
    `head_of`, so a link to a cause's or issue's earlier version reaches its current one.

    Only heads are issues. A member of an issue's cluster linking to a cause is a finding about
    the issue, and the issue is attributed by its head or by the cause.
    """
    heads = head_of(current, records)
    causes = {r["id"] for r in current if CAUSE in r.get("keywords", [])}
    issues = {r["id"] for r in current if ISSUE in r.get("keywords", [])}
    found = {cause: set() for cause in causes}
    for record in current:
        mine = heads.get(record["id"])
        if mine is None:
            continue
        for link in record.get("links", []):
            target = heads.get(link["target"]) if isinstance(link["target"], str) else None
            if link["label"] == "instance_of" and mine in issues and target in causes:
                found[target].add(mine)
            elif link["label"] == "causes" and mine in causes and target in issues:
                found[mine].add(target)
    return found


def rows_for_causes():
    """The causes region: each cause, ranked by how many issues it explains, with those issues.

    **Ranked by count**, unlike the entry regions, because rank is what the region is for -- a
    cause explaining nine issues is the one worth a structural fix. Ties fall back to filing order.
    The issues follow on an indented line, which a list item's renderer folds into the item and
    the removal guard, reading only lines that open a row, ignores.
    """
    current, records = load()
    found = attributions(current, records)
    issues = {r["id"]: r for r in current if ISSUE in r.get("keywords", [])}
    heads = [m for m in current if CAUSE in m.get("keywords", [])]
    heads.sort(key=lambda m: (-len(found[m["id"]]), row_order(m, records)))
    rows = []
    for head in heads:
        state, slug = standing(head, CAUSE)
        explained = sorted(found[head["id"]], key=lambda i: row_order(issues[i], records))
        still_open = [i for i in explained if standing(issues[i], ISSUE)[0] == "open"]
        row = f"- [{' ' if state == 'active' else 'x'}] `{slug}`"
        if state != "active":
            row += f" · {state}"
        count = len(explained)
        row += f" · {count} issue{'' if count == 1 else 's'}, {len(still_open)} open"
        row += f" — {summary(head['content'])}"
        rows.append(row)
        if explained:
            rows.append("  " + " · ".join(
                f"`{standing(issues[i], ISSUE)[1]}`" + (" (open)" if i in still_open else "")
                for i in explained))
    return rows


def rows_for(keyword):
    current, records = load()
    heads = [m for m in current if keyword in m.get("keywords", [])]
    heads.sort(key=lambda m: row_order(m, records))
    family = keyword.rsplit("-", 1)[0]
    rows = []
    for head in heads:
        keywords = head.get("keywords", [])
        state, slug = standing(head, keyword)
        # Strip the facet prefix that matched, not a hyphen position: the label is what follows
        # the family, and a `k.split("-", 1)` assumed the family was one segment long.
        facets = [k[len(p):] for p in FACETS for k in keywords if k.startswith(p)]
        blocked = [l["target"] for l in head.get("links", []) if l["label"] == "depends_on"]
        row = f"- [{' ' if state == 'open' else 'x'}] `{slug}`"
        if state != "open" and state not in ORDINARY_DONE:
            row += f" · {state}"
        for facet in facets:
            row += f" · {facet}"
        row += f" — {summary(head['content'])}"
        if blocked:
            row += f" · blocked by {len(blocked)}"
        rows.append(row)
    return rows


def self_check():
    """Prove a row keeps its place across a supersession, which is what the ordering is for.

    The sort key is `earliest_ancestor` and nothing else, so its cases are the whole of the
    ordering's behaviour -- including the two the real file does not currently contain, a pruned
    ancestor and a supersession cycle, which is exactly why they are asserted here rather than
    left to be discovered by a regeneration that moves a row for no visible reason.
    """

    def head(rid, *supersedes):
        return {"id": rid, "content": "x",
                "links": [{"label": "supersedes", "target": t} for t in supersedes]}

    def among(*records):
        return {r["id"]: r for r in records}

    cases = [
        ("an unsuperseded head is its own ancestor",
         among(head("01C")), "01C", "01C"),
        ("a chain walks back to the entry's first version",
         among(head("01C", "01B"), head("01B", "01A"), head("01A")), "01C", "01A"),
        ("an ancestor pruned from the file still fixes the position",
         among(head("01B", "01A")), "01B", "01A"),
        ("a supersession cycle cannot hang the walk",
         among(head("01Y", "01X"), head("01X", "01Y")), "01Y", "01X"),
        ("a head superseding two entries takes the earlier",
         among(head("01M", "01F", "01G"), head("01F"), head("01G")), "01M", "01F"),
        ("a link that is not a supersession is not a chain",
         among({**head("01C"), "links": [{"label": "related_to", "target": "01A"}]},
               head("01A")), "01C", "01C"),
    ]

    failures = []
    for name, records, rid, expected in cases:
        try:
            got = earliest_ancestor(rid, records)
        except RecursionError:
            failures.append(f"{name}: the walk did not terminate")
            continue
        if got != expected:
            failures.append(f"{name}: expected {expected}, got {got}")

    # A reserved mark listed *before* the slug is the case that matters: keyword order is storage
    # order, so a case that passes only with the slug listed first proves nothing.
    status = "hoard-proj-issue-open"
    slug_cases = [
        ("a lone slug is the slug",
         ["hoard-proj-issue-entry", "the-slug", status], ["the-slug"]),
        ("a watch mark listed before the slug is not the slug",
         ["hoard-proj-issue-entry", "hoard-investigation", "the-slug", status], ["the-slug"]),
        ("a reserved mark nobody has invented yet is not the slug",
         ["hoard-proj-issue-entry", "hoard-proj-later-mark", "the-slug", status], ["the-slug"]),
        ("a facet is not the slug",
         ["hoard-proj-issue-entry", "hoard-proj-severity-misleads", "the-slug", status],
         ["the-slug"]),
        ("a subject term after the slug is reported after it",
         ["hoard-proj-issue-entry", "the-slug", status, "subject-term"],
         ["the-slug", "subject-term"]),
    ]
    for name, keywords, expected in slug_cases:
        got = slug_candidates(keywords)
        if got != expected:
            failures.append(f"{name}: expected {expected}, got {got}")

    # Two heads can share an earliest ancestor -- an entry split in two -- and a stable sort then
    # keeps them in the order the file lists them. That order is a hash of each id since 2026-09-21,
    # so a tie has to be broken by something in the heads themselves, or the rows swap whenever the
    # file's order does.
    split = among(head("01A"), head("01C", "01A"), head("01B", "01A"))
    shuffled = [split["01C"], split["01B"]]
    ordered = [h["id"] for h in sorted(shuffled, key=lambda h: row_order(h, split))]
    if ordered != ["01B", "01C"]:
        failures.append(f"heads sharing an ancestor are ordered by their own id: got {ordered}")

    # Attribution for the causes region. A link in either direction counts, from the current version
    # of either chain, and counts once -- see `attributions`.
    def rec(rid, keywords, *links, supersedes=()):
        return {"id": rid, "content": rid, "keywords": keywords,
                "links": [{"label": l, "target": t} for l, t in links]
                         + [{"label": "supersedes", "target": t} for t in supersedes]}

    cause_kw = [CAUSE, "the-cause", "hoard-proj-cause-active"]
    issue_kw = [ISSUE, "an-issue", "hoard-proj-issue-open"]
    attribution_cases = [
        ("an issue naming the cause is attributed",
         [rec("01K", cause_kw), rec("01I", issue_kw, ("instance_of", "01K"))],
         {"01K": {"01I"}}),
        ("a cause naming the issue is attributed",
         [rec("01K", cause_kw, ("causes", "01I")), rec("01I", issue_kw)],
         {"01K": {"01I"}}),
        ("both directions count once",
         [rec("01K", cause_kw, ("causes", "01I")), rec("01I", issue_kw, ("instance_of", "01K"))],
         {"01K": {"01I"}}),
        ("an issue superseded without restating its link has dropped it",
         [rec("01K", cause_kw), rec("01I", issue_kw, ("instance_of", "01K")),
          rec("01J", [ISSUE, "an-issue", "hoard-proj-issue-fixed"], supersedes=("01I",))],
         {"01K": set()}),
        ("an issue superseded restating its link keeps its attribution",
         [rec("01K", cause_kw), rec("01I", issue_kw, ("instance_of", "01K")),
          rec("01J", [ISSUE, "an-issue", "hoard-proj-issue-fixed"], ("instance_of", "01K"),
              supersedes=("01I",))],
         {"01K": {"01J"}}),
        ("a cause superseded without restating its link has dropped it",
         [rec("01K", cause_kw, ("causes", "01I")), rec("01L", cause_kw, supersedes=("01K",)),
          rec("01I", issue_kw)],
         {"01L": set()}),
        ("a link to a cause's earlier version reaches its current one",
         [rec("01K", cause_kw), rec("01L", cause_kw, supersedes=("01K",)),
          rec("01I", issue_kw, ("instance_of", "01K"))],
         {"01L": {"01I"}}),
        ("a member of an issue's cluster is not an issue",
         [rec("01K", cause_kw), rec("01M", ["an-issue"], ("instance_of", "01K"))],
         {"01K": set()}),
        ("a link of another label is not an attribution",
         [rec("01K", cause_kw), rec("01I", issue_kw, ("related_to", "01K"))],
         {"01K": set()}),
    ]
    for name, recs, expected in attribution_cases:
        records = {r["id"]: r for r in recs}
        superseded = {l["target"] for r in recs for l in r["links"] if l["label"] == "supersedes"}
        current = [r for r in recs if r["id"] not in superseded]
        got = {c: set(i) for c, i in attributions(current, records).items()}
        if got != expected:
            failures.append(f"{name}: expected {expected}, got {got}")

    # Slug collisions: a member well older than the entry flags it; one from the same sitting does
    # not; and an entry migrated before the check's start date is never asked.
    def dated(rid, stamp, keywords, *supersedes):
        return {"id": rid, "content": rid, "keywords": keywords, "created": {"at": stamp},
                "links": [{"label": "supersedes", "target": t} for t in supersedes]}

    collision_cases = [
        ("a slug in use well before its entry collides",
         [dated("01H", "2026-10-10T00:00:00Z", [ISSUE, "taken-term", "hoard-proj-issue-open"]),
          dated("01A", "2026-10-01T00:00:00Z", ["taken-term"])], [("taken-term", 1)]),
        ("a finding stored the same day as its entry does not",
         [dated("01H", "2026-10-10T12:00:00Z", [ISSUE, "fresh-term", "hoard-proj-issue-open"]),
          dated("01A", "2026-10-10T09:00:00Z", ["fresh-term"])], []),
        ("an entry migrated before the check began is not asked",
         [dated("01H", "2026-09-07T00:00:00Z", [ISSUE, "old-term", "hoard-proj-issue-open"]),
          dated("01A", "2026-08-01T00:00:00Z", ["old-term"])], []),
        ("the entry's first version is what is dated, not its latest",
         [dated("01G", "2026-10-02T00:00:00Z", [ISSUE, "kept-term", "hoard-proj-issue-open"]),
          dated("01H", "2026-10-20T00:00:00Z", [ISSUE, "kept-term", "hoard-proj-issue-fixed"], "01G"),
          dated("01A", "2026-10-01T12:00:00Z", ["kept-term"])], []),
    ]
    for name, recs, expected in collision_cases:
        records = {r["id"]: r for r in recs}
        superseded = {l["target"] for r in recs for l in r["links"] if l["label"] == "supersedes"}
        current = [r for r in recs if r["id"] not in superseded]
        got = slug_collisions(current, records)
        if got != expected:
            failures.append(f"{name}: expected {expected}, got {got}")

    # A forgotten record. `forget` deletes a record and its links and leaves a tombstone whose
    # `ancestry` holds the supersessions it had, so a chain survives the forget of its middle. Read
    # as absence, the chain stops at the tombstone and the entry's first version comes back as a
    # current head -- which is what the first bulk forget of chain middles did, on 2026-10-19.
    def id_at(stamp):
        millis = int(datetime.datetime.fromisoformat(stamp).replace(
            tzinfo=datetime.timezone.utc).timestamp() * 1000)
        digits = ""
        for _ in range(10):
            millis, digit = divmod(millis, 32)
            digits = CROCKFORD[digit] + digits
        return digits + "0" * 16

    first, middle, latest = (id_at("2026-10-01T00:00:00"), id_at("2026-10-02T00:00:00"),
                             id_at("2026-10-20T00:00:00"))
    tombstone_lines = [
        json.dumps({"content": "open", "id": first, "keywords": [], "links": []}),
        json.dumps({"ancestry": [{"label": "supersedes", "target": first}], "forgot": "0" * 16,
                    "forgotten": {"at": "2026-10-19T00:00:00Z", "by": "x"}, "id": middle}),
        json.dumps({"content": "built", "id": latest, "keywords": [],
                    "links": [{"label": "supersedes", "target": middle}]}),
    ]
    forgotten_head, _ = parse(tombstone_lines[:2])
    if forgotten_head != []:
        failures.append(f"a forgotten head is not a row: got {[r['id'] for r in forgotten_head]}")
    current, records = parse(tombstone_lines)
    forget_cases = 4
    if [r["id"] for r in current] != [latest]:
        failures.append(f"a forgotten middle keeps its chain: current heads were "
                        f"{[r['id'] for r in current]}, expected only the latest")
    if earliest_ancestor(latest, records) != first:
        failures.append("a forgotten middle keeps its chain: the walk stopped at the tombstone")
    # And an entry whose first version is gone is dated from that version's id, which a ULID
    # carries, rather than looked up in a record that is not there.
    pruned = [dated(latest, "2026-10-20T00:00:00Z", [ISSUE, "kept-term", "hoard-proj-issue-fixed"],
                    middle),
              dated(id_at("2026-10-01T12:00:00"), "2026-10-01T12:00:00Z", ["kept-term"])]
    try:
        got = slug_collisions([pruned[0], pruned[1]], {r["id"]: r for r in pruned})
        if got != []:
            failures.append(f"an entry whose first version is gone is dated by its id: got {got}")
    except KeyError as missing:
        failures.append(f"an entry whose first version is gone is dated by its id: "
                        f"looked up {missing}")

    total = (len(cases) + len(slug_cases) + len(attribution_cases) + len(collision_cases) + 1
             + forget_cases)
    for failure in failures:
        print(f"self-check: {failure}", file=sys.stderr)
    if failures:
        print(f"self-check: {len(failures)} of {total} cases failed.", file=sys.stderr)
        return 1
    print(f"self-check: a row's position survives a supersession, and no reserved mark is taken "
          f"for its slug ({total} cases)")
    return 0


ENTRIES = ("hoard-proj-issue-entry", "hoard-proj-backlog-entry", "hoard-proj-question-entry",
           CAUSE)

# Entries whose first record is older than this were migrated into memory from hand-written files
# between 2026-09-06 and 2026-09-08. They existed as text before their records did, so a finding
# older than the record may be the entry's own, and the test below cannot tell. Anything filed
# since was born with its record.
COLLISION_CHECK_SINCE = datetime.datetime(2026, 9, 9)

# A finding stored minutes or hours before its entry is filed -- the usual order within a sitting --
# is the entry's own. A slug in use well before is a subject term the entry was coined over.
COLLISION_MARGIN = datetime.timedelta(days=1)


def instant(stamp):
    """A record's RFC 3339 timestamp, to the second."""
    return datetime.datetime.fromisoformat(stamp.rstrip("Z").partition(".")[0])


def born_at(rid, records):
    """When a record was created: its `created` time, or the time its id carries when the record
    itself is gone -- pruned from the file, or forgotten and left as a tombstone, both of which
    `earliest_ancestor` deliberately walks to."""
    record = records.get(rid)
    if record is not None and "created" in record:
        return instant(record["created"]["at"])
    millis = 0
    for character in rid[:10]:
        millis = millis * 32 + CROCKFORD.index(character)
    return datetime.datetime.fromtimestamp(millis / 1000, datetime.timezone.utc).replace(tzinfo=None)


def slug_collisions(current, records):
    """Entries whose slug was already in use as a keyword well before the entry existed.

    `a-slug-that-is-also-a-subject-keyword-over-gathers`: an entry's slug is the keyword its
    findings gather on, so a slug that was already a subject term gathers the whole subject --
    `graph-traversal` held 26 memories, most of them about traversal rather than the entry. The test
    is a member carrying the slug that is older than the entry's first record by more than
    `COLLISION_MARGIN`, which no finding of the entry can be. Returns `(slug, count)` pairs.
    """
    found = []
    for head in current:
        keywords = head.get("keywords", [])
        if not any(k in ENTRIES for k in keywords):
            continue
        candidates = slug_candidates(keywords)
        if not candidates:
            continue
        slug = candidates[0].lower()
        born = born_at(earliest_ancestor(head["id"], records), records)
        if born < COLLISION_CHECK_SINCE:
            continue
        older = [r for r in current
                 if slug in (k.lower() for k in r.get("keywords", []))
                 and instant(r["created"]["at"]) < born - COLLISION_MARGIN]
        if older:
            found.append((slug, len(older)))
    return found


def one_region(path, keyword, flags):
    """Bring one region up to date, or report it stale. True if it is out of date under `--check`."""
    text = path.read_text()
    start, stop = region(text, keyword)
    current = text[start:stop]
    rows = rows_for_causes() if keyword == CAUSE else rows_for(keyword)
    generated = "\n" + "\n".join(rows) + "\n"

    if current == generated:
        print(f"{path} [{keyword}]: index is current ({len(slugs_in(generated))} entries)")
        return False

    lost = slugs_in(current) - slugs_in(generated)
    if lost and "--allow-removals" not in flags:
        sys.exit(
            f"{path} [{keyword}]: regenerating would remove {len(lost)} row(s): "
            f"{', '.join(sorted(lost))}.\n"
            "The likely cause is an export older than the store -- export first, then retry.\n"
            "If the entries really are gone, pass --allow-removals."
        )
    if "--check" in flags:
        print(f"{path} [{keyword}]: index is out of date.")
        return True

    path.write_text(text[:start] + generated + text[stop:])
    print(f"{path} [{keyword}]: wrote {len(slugs_in(generated))} entries")
    return False


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    flags = {a for a in sys.argv[1:] if a.startswith("--")}
    # `--self-check` asks about this script rather than about a file, so it takes no path and is
    # answered before the argument check that would demand one.
    if flags == {"--self-check"} and not args:
        return self_check()
    if not 1 <= len(args) <= 2 or flags - {"--check", "--allow-removals"}:
        sys.exit(__doc__)
    path = pathlib.Path(args[0])

    keywords = args[1:] or declared_in(path.read_text())
    if not keywords:
        sys.exit(f"{path}: no `BEGIN GENERATED INDEX` marker found; nothing to generate.")

    # Every region is visited before exiting: stopping at the first stale one would report a
    # third of the file's disagreement and read as the whole of it.
    stale = [k for k in keywords if one_region(path, k, flags)]
    collisions = slug_collisions(*load())
    for slug, older in collisions:
        print(f"{path}: the slug `{slug}` was already a keyword on {older} memor"
              f"{'y' if older == 1 else 'ies'} older than its entry, so recalling the entry gathers "
              "that subject too. Give the entry a slug not yet in use, and link it to the subject "
              "instead (a-slug-that-is-also-a-subject-keyword-over-gathers).")
    if collisions:
        sys.exit(f"{path}: {len(collisions)} entry slug(s) collide with keywords already in use.")
    if stale:
        sys.exit(
            f"{path}: {len(stale)} region(s) out of date. "
            f"Run: python3 {sys.argv[0]} {args[0]}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
