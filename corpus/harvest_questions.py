"""Harvest off-domain probes from prose, in the forms real recall queries actually take.

The non-match floor needs a negative class that resembles real off-domain traffic. Two attempts
failed for the same underlying reason, and the failure is not obvious:

- **Probes written to be off-domain are not representative of off-domain.** Thirty substance-shaped
  questions I authored -- sourdough, fertiliser, descaling -- scored a median 0.145 against the live
  corpus where real off-domain questions score 0.294. Being told to be distant is what made them
  distant, and every benefit figure measured against them was inflated by the difference.
- **A store on another subject is not off-domain either.** `governance-docs` holds governing
  documents, which share *rule-shaped structure* with a corpus about conventions -- and structure,
  not subject, is what these scores follow.

What works is prose somebody wrote for their own purposes. Three independently harvested classes --
a volunteer's meeting notes, public-domain non-fiction, and novel dialogue -- agree on the median to
within 0.015 and the p90 to within 0.022. The calibration point is a property of the corpus rather
than of the source, so any real prose will do.

## Form matters, and the obvious choice is wrong

**Real recall queries are mostly not questions.** Measured over 142 harvested `hoard_recall` calls:
18.3% interrogative, 40.8% declarative, 40.8% bare noun phrase -- and *not one of the 142 ends in a
question mark*. A probe set of pure interrogatives is form-mismatched with the traffic it stands in
for, so this harvests all three forms and `--mix` emits them in the measured proportions.

Noun phrases are cut from a sentence's **subject**, not its predicate: real queries look like
`reduced detail on the wire` and `protocol channel discipline`, which are subject-shaped. Cutting the
predicate instead yields fragments like `been the mark of the leisure`, which are not.

## Reading private notes: this is built to leak nothing

**Only the extracted probe leaves this script** -- never surrounding text, never a heading, never a
file path in the committable output, since a location is a way back to the note.

**Anything tripping a filter is dropped, not redacted.** Redaction invites a judgement about whether
the remainder is safe; dropping does not, and probes are cheap. Dropped: any capitalised word after
the first (names of people, places and organisations all look like this); any digit (dates, amounts,
counts, ages); any address, link or currency; anything under four or over twenty words; anything
touching pay, contracts, money, personnel or people who might be vulnerable -- none revealing once
names and numbers are gone, but all of it inferable-from by a reader who knows the organisation.

Two further filters serve *purpose* rather than privacy: anything about software, or shaped like an
identifier. `--require` matches whole files, and a file about anything may hold a paragraph about
software -- a leaked design question would be *in*-domain for the corpus this set exists to sit
outside of.

**Two outputs, one committable.** `--review` carries source filenames for a human to check against;
`--out` carries probes alone. Public-domain books are safe to commit; private notes are not.

Run:
    python3 corpus/harvest_questions.py --vault ~/books --ext .txt --mix --out corpus/probes.json
"""

import argparse
import json
import os
import random
import re

ALLOWED_CAPITALS = {"I", "I'm", "I'd", "I've", "I'll", "OK", "A", "An", "The"}

SOFTWARE_TERMS = {
    "api", "app", "backend", "branch", "browser", "bug", "cache", "class", "cli", "code",
    "commit", "compiler", "config", "cpu", "css", "database", "db", "debug", "deploy", "disk",
    "dns", "docker", "endpoint", "feed", "file", "firewall", "framework", "frontend", "function",
    "git", "gui", "hash", "html", "http", "index", "json", "kernel", "latency", "library",
    "linux", "log", "login", "memory", "merge", "migration", "network", "node", "parser",
    "password", "patch", "plugin", "port", "protocol", "query", "queue", "regex", "release",
    "repo", "repository", "runtime", "schema", "script", "server", "session", "software", "sql",
    "ssh", "stack", "string", "sync", "table", "test", "thread", "token", "ui", "url", "user",
    "username", "vector", "version", "web", "website",
}

SENSITIVE = re.compile(
    r"\b(salar|salaries|compensation|severance|wage|stipend|allowance|reimburs|pension|"
    r"exit gift|provision|contract|clause|liabilit|indemnif|lawsuit|attorney|sign(ed|ing)?|"
    r"budget|funds?|account|invest|deposit|interest|donation|financ|audit|dues|"
    r"hire|fire|resign|terminat|discipline|complaint|step down|"
    r"safely|immigration|asylum|refugee|abuse|crisis|shelter)\b",
    re.I,
)

IDENTIFIER = re.compile(r"[A-Za-z]+[._][A-Za-z]|[a-z][A-Z]")
HAS_DIGIT = re.compile(r"\d")
HAS_CONTACT = re.compile(r"@|https?://|www\.")
HAS_CURRENCY = re.compile(r"[$£€¥]")
SENTENCE_END = re.compile(r"(?<=[.!?])\s+")
WORD = re.compile(r"[A-Za-z][A-Za-z'’-]*")
CODE_FENCE = re.compile(r"^\s*(```|~~~)")

STRIP = [
    (re.compile(r"!?\[\[([^\]|]*\|)?([^\]]*)\]\]"), r"\2"),
    (re.compile(r"!?\[([^\]]*)\]\([^)]*\)"), r"\1"),
    (re.compile(r"[*_`~]+"), ""),
    (re.compile(r"^\s*#+\s*"), ""),
    (re.compile(r"^\s*[-*+]\s+(\[[ xX]\]\s*)?"), ""),
    (re.compile(r"^\s*>+\s*"), ""),
    (re.compile(r"^\s*\d+[.)]\s+"), ""),
]

LEADING_NOISE = re.compile(
    r"^(the|a|an|this|that|these|those|it|its|his|her|their|our|my|such|some|any|all|"
    r"but|and|yet|so|now|then|thus|hence|when|while|if|though|although|because|since|"
    r"for|of|in|on|at|to|as|by)\s+",
    re.I,
)

FINITE = re.compile(
    r"\b(is|are|was|were|has|have|had|do|does|did|can|could|will|would|shall|should|may|might|"
    r"must|becomes?|makes?|takes?|gives?|goes?|comes?|means?|needs?|requires?|produces?|depends?|"
    r"consists?|involves?|remains?|appears?|seems?|tends?|allows?|leads?|serves?)\b",
    re.I,
)

# Measured over 142 real `hoard_recall` calls. See the module docstring.
REAL_FORM_MIX = {"question": 0.18, "statement": 0.41, "phrase": 0.41}

# Prufrock, 1915, squarely of the period the rest of this is drawn from. Kept with its question
# mark, which nothing else here has, as the one deliberate exception.
TRIBUTE = "Do I dare to eat a peach?"


def clean(line):
    for pattern, replacement in STRIP:
        line = pattern.sub(replacement, line)
    return line.strip()


def strip_noise(text):
    while True:
        shorter = LEADING_NOISE.sub("", text, count=1)
        if shorter == text:
            return text
        text = shorter


# Quotation marks and dashes at a boundary would otherwise ride along into the probe -- and a leading
# quote is worse than untidy: it occupies position zero, so `"Henry always enjoys seeing you` passes
# the proper-noun filter, which only checks from the second word on.
EDGE_NOISE = re.compile(r"^[\"\u201c\u201d\u2018\u2019'\s\-\u2014\u2013]+|[\"\u201c\u201d\u2018\u2019\s\-\u2014\u2013]+$")
QUOTE_INSIDE = re.compile(r"[\"\u201c\u201d]")


def probes_in(line):
    """Yield (form, text) for every probe this line offers."""
    sentences = [s.strip() for s in SENTENCE_END.split(line) if s.strip()]
    for index, sentence in enumerate(sentences):
        if sentence.endswith("?"):
            for cut in (". ", "! ", "? "):
                if cut in sentence[:-1]:
                    sentence = sentence.rsplit(cut, 1)[-1]
            # Stripped, because no real query carries one.
            yield "question", EDGE_NOISE.sub("", sentence.rstrip("?"))
            continue
        if index > 0:
            continue  # statements and phrases come from the topic sentence only
        body = EDGE_NOISE.sub("", sentence.rstrip(".!"))
        # A finite verb is what separates a topic sentence from a fragment of speech.
        match = FINITE.search(body)
        if not match:
            continue
        yield "statement", body
        subject = strip_noise(LIST_MARKER.sub("", body[: match.start()].strip()))
        if len(subject.split()) >= 3:
            yield "phrase", " ".join(subject.split()[:6])


# List and enumeration markers a phrase cut can leave stranded at the front: "(ii)", "(a)", "1.".
LIST_MARKER = re.compile(r"^\(?[0-9ivxIVX]+[.)]\s*")


def rejection(text, form="question", lowercased=frozenset()):
    """Why this probe must not leave the source, or None if it may.

    `form` matters for one rule. A sentence's first word is capitalised because it is first, so the
    proper-noun check starts at the second. **A phrase has no such position**: cutting the subject out
    of a sentence strips whatever preceded it, which can leave a name exposed at index zero --
    `Hull-House, however, before it` passed this filter until the distinction was drawn. Phrases are
    therefore checked from the first word.

    That leaves one hole, and it is the one that matters for private notes: a proper noun standing
    *first* in a statement, where capitalisation is expected and so proves nothing. `lowercased` is
    what closes it -- the set of words seen in lower case anywhere in the same document. A common
    word capitalised only because it begins a sentence will appear in lower case somewhere else;
    a name will not. `Customary expenditure must be classed` survives, `Hull-House under the
    direction of` does not, and neither judgement had to be made in advance.
    """
    words = WORD.findall(text)
    if len(words) < 4:
        return "too short"
    if len(words) > 20:
        return "too long"
    if HAS_DIGIT.search(text):
        return "contains a digit"
    if HAS_CONTACT.search(text):
        return "contains an address or link"
    if HAS_CURRENCY.search(text):
        return "contains a currency amount"
    for word in words[0 if form == "phrase" else 1:]:
        if word[0].isupper() and word not in ALLOWED_CAPITALS:
            return "contains a proper noun"
    if form != "phrase" and words and words[0][0].isupper():
        if words[0] not in ALLOWED_CAPITALS and words[0].lower() not in lowercased:
            return "opens with a proper noun"
    if IDENTIFIER.search(text):
        return "contains an identifier"
    if any(word.lower() in SOFTWARE_TERMS for word in words):
        return "is about software"
    if SENSITIVE.search(text):
        return "touches a sensitive subject"
    if QUOTE_INSIDE.search(text):
        return "spans a quotation boundary"
    return None


def harvest(vaults, require, extension):
    kept = {"question": {}, "statement": {}, "phrase": {}}
    dropped, scanned = {}, 0
    for vault in vaults:
        for root, dirs, files in os.walk(vault):
            dirs[:] = [d for d in dirs if not d.startswith(".")]
            for name in sorted(files):
                if not name.endswith(extension):
                    continue
                try:
                    text = open(os.path.join(root, name), encoding="utf-8", errors="replace").read()
                except OSError:
                    continue
                if require and require.lower() not in text.lower():
                    continue
                scanned += 1
                # Every word this document ever writes in lower case. See `rejection`.
                lowercased = frozenset(
                    w for w in WORD.findall(text) if w[0].islower()
                ) | frozenset(w.lower() for w in WORD.findall(text) if w[0].islower())
                in_code = False
                for line in text.splitlines():
                    if CODE_FENCE.match(line):
                        in_code = not in_code
                        continue
                    if in_code:
                        continue
                    for form, probe in probes_in(clean(line)):
                        why = rejection(probe, form, lowercased)
                        if why:
                            dropped[why] = dropped.get(why, 0) + 1
                        else:
                            kept[form].setdefault(probe, name)
    return kept, dropped, scanned


parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
parser.add_argument("--vault", action="append", required=True, help="source root; repeatable")
parser.add_argument("--require", default="", help="only scan files containing this word")
parser.add_argument("--ext", default=".md", help=".md for a notes vault, .txt for books")
parser.add_argument("--out", required=True, help="probes only, no sources")
parser.add_argument("--review", default="", help="review copy carrying source filenames")
parser.add_argument("--topic", default="", help="what these are about, for provenance")
parser.add_argument("--mix", action="store_true", help="emit the measured real-query form mix")
parser.add_argument("--limit", type=int, default=0, help="cap the total emitted")
parser.add_argument("--tribute", action="store_true", help="include the Prufrock line")
parser.add_argument(
    "--prose-vault",
    action="append",
    default=[],
    help="source for statements and phrases only. Fiction supplies usable questions -- its dialogue "
    "lands within 0.004 of committee minutes -- but not usable topic sentences, because the first "
    "sentence of a line in a novel is a fragment of speech. Point this at expository prose.",
)
args = parser.parse_args()

kept, dropped, scanned = harvest(args.vault, args.require, args.ext)
if args.prose_vault:
    prose, prose_dropped, prose_scanned = harvest(args.prose_vault, args.require, args.ext)
    kept["statement"], kept["phrase"] = prose["statement"], prose["phrase"]
    scanned += prose_scanned
    for why, count in prose_dropped.items():
        dropped[why] = dropped.get(why, 0) + count
counts = {form: len(items) for form, items in kept.items()}

if args.mix:
    usable = [int(len(kept[f]) / REAL_FORM_MIX[f]) for f in REAL_FORM_MIX if kept[f]]
    total = min(usable) if usable else 0
    if args.limit:
        total = min(total, args.limit)
    rng = random.Random(20260909)
    selected = []
    for form, share in REAL_FORM_MIX.items():
        pool = sorted(kept[form])
        rng.shuffle(pool)
        selected += [(form, p) for p in pool[: round(total * share)]]
    selected.sort()
else:
    selected = sorted((form, p) for form in kept for p in kept[form])
    if args.limit:
        selected = selected[: args.limit]

if args.tribute:
    selected.append(("question", TRIBUTE))

with open(args.out, "w") as handle:
    json.dump(
        {
            "provenance": (
                f"Off-domain probes harvested from prose{' about ' + args.topic if args.topic else ''}. "
                "Probes only -- no surrounding text, no headings, no file paths. Anything carrying a "
                "proper noun, digit, address, link, currency amount, identifier, software term or "
                "sensitive subject was dropped rather than redacted. "
                + (
                    "Emitted in the form mix measured over 142 real hoard_recall calls: 18% "
                    "interrogative, 41% declarative, 41% noun phrase. Question marks are stripped, "
                    "because not one of those 142 carried one."
                    if args.mix
                    else "All harvested forms, unmixed."
                )
            ),
            "topic": args.topic,
            "form_mix": REAL_FORM_MIX if args.mix else None,
            "counts_available": counts,
            "questions": [p for _, p in selected],
            "by_form": {
                f: [p for form, p in selected if form == f]
                for f in ("question", "statement", "phrase")
            },
        },
        handle,
        indent=1,
    )
    handle.write("\n")

if args.review:
    with open(args.review, "w") as handle:
        json.dump(
            {
                "warning": "carries source filenames -- for human review only, never commit",
                "probes": [
                    {"form": f, "probe": p, "source": kept[f].get(p, "(tribute)")}
                    for f, p in selected
                ],
            },
            handle,
            indent=1,
        )
        handle.write("\n")

print(f"{scanned} file(s) scanned")
print("available: " + ", ".join(f"{n} {f}" for f, n in counts.items()))
print(f"{len(selected)} probe(s) emitted -> {args.out}")
print("dropped, by reason:")
for why, count in sorted(dropped.items(), key=lambda row: -row[1]):
    print(f"  {count:>5}  {why}")
