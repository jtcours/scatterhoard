# Memory Data Model

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

## What a memory is

A memory is a chunk of information small enough that the vector database can properly express its embeddings.

**Resolved 2026-09-19: the size is bounded, at 2000 characters of what the embedding is built from — content, `why` and keywords together — and a longer memory is refused at both write doors.** The reason is not the embedder's budget. That was the reason while `encode` truncated at 512 tokens, and it stopped being one the same day, when [the encoding stopped truncating](embeddings.md#the-identity-covers-the-encoding-not-only-the-weights). What the limit bounds is what a memory *is*: a record that long is usually several claims, and the relationships between them — which link labels exist to carry, and which a later memory could otherwise point at one at a time — are buried in prose instead. So the refusal says to split rather than to shorten.

The number came from this project's own corpus: not one memory written in Scatterhoard's first five weeks reached it, with a median of 998 characters and a longest of 1565, and that is the stretch in which this design was produced. Characters rather than tokens because tokenising needs the model, and the keyword path is the path that works without one — a store that refused by tokens could not answer at the moment it had to. A character is a Unicode scalar rather than a byte, so a set written in a script that costs more bytes per character is not given a shorter memory than an English one.

**What is expected to follow, and is not a defect:** sizes fall toward the cap in a corpus written before it existed. The human partner's judgement, 2026-09-19, is that this project's own set will feel it for a while and no other user ever will, since nobody else has a corpus written under no limit at all.

A memory consists of multiple parts:

- **Unique ID.** *(Form resolved — see below.)* Every memory has a globally unique identifier to allow cross-references. Scatterhoard serves distributed development efforts, so it needs globally unique identifiers.
- **Confidence.** The LLM's confidence in the memory, on the range 0.0 to 1.0. The conventions will include guidance on setting confidence levels.
- **Timestamps** (created, modified), expressed in UTC or Unix timestamps (whichever is easier for an LLM to reason about).
- **Author.** The (project-relative) user IDs of the memory's creator and last modifier.
- **Origin.** *(Resolved — see below.)* Whether the memory's content originated from the human or the LLM.
- **Keywords.** Each memory will have one or more keywords. The conventions will contain guidance on how to choose them consistently. A keyword is **stored as written and matched without regard to case** — the value is a record and the fold is an index key, and they are separate because conflating them destroyed the caller's spelling irreversibly (`keywords-silently-lowercased` in [Issues](../../TRACKING.md#issues)). Case is also information the embedder uses, since keywords are part of what gets vectorised ([Embeddings and Vector Storage](embeddings.md)): a rare identifier does not tokenise the same way folded.
- **Content.** The actual content of the memory itself. A semantic search during recall would use similarity to the content's vector embedding.
- **Why.** The background reasoning that led to the memory's content. Including the reasoning lets the LLM reason about whether the memory is valid in a given context, or whether the current context represents an exception to the rule. The conventions should strongly favor including reasoning with every memory. Distinct from a convention's `reinforcement` field, which is held to a fixed standard the memory's `why` deliberately isn't — see [Conventions](conventions.md#why-it-isnt-called-why).
- **Context.** *(Resolved — see below.)* Optional. Episodic cues (when/where/why this came up) kept out of the embedding, used to help the LLM place a recalled memory for the human.
- **Links.** *(Resolved — see below.)* Memories are nodes in a graph. The graph includes labeled links to other memories, where the labels might be `RELATED_TO`, `SUPERSEDES`, `CONTRADICTS`, `EXPLAINS`, `SUPPORTS`, and so on.

## Resolved: timestamps and authors travel in pairs

`created` and `modified` are each a **(author, timestamp) pair** rather than four independent fields. They are only meaningful together — a revision that moves the timestamp but leaves the previous author attached, or the reverse, is a false record — and pairing them makes that combination unwritable rather than merely discouraged. Moving `modified` is the only operation that sets it, so the time cannot advance without the author moving with it.

Timestamps are UTC instants serialised as RFC 3339. The data model leaves the encoding to whichever an LLM reads more easily; a readable string in the exported file costs nothing at the diff that a Unix integer would save.

## Proposed 2026-08-31: the author is a reference, and the name lives in a memory

*Worked out with the human partner, from a concrete case: every memory in these sets is attributed
to a macOS account name, and the identity they publish under is `jtcours`. The account
name means nothing to anyone reading the repository, and keeping a public identity separate from a
personal one — without a publicly readable trail joining them — is a reason to want this that is not
tidiness.*

**Attribution is the one part of a settled record that legitimately changes afterwards.** Everything
else about a memory is a claim its author made at a moment; the *name they publish under* is a fact
about them that can change later — a rename, leaving an organisation, deciding to pseudonymise. The
record shape puts that mutable fact inside 155 immutable records, so changing it means rewriting all
of them.

**The proposal**: `created.by` and `modified.by` hold a [`MemoryId`](#resolved-memory-ids-are-uuidv7)
referring to a memory that carries the name, keyworded `hoard-author`. Re-attribution then supersedes
one small record instead of rewriting every record that names it.

### Why a memory rather than a new record shape

`hoard-scope-topics` is the precedent and it already works: set-level information stored as an
ordinary memory under a reserved keyword. [`keyword-standards`](conventions.md) already reserves the
`hoard-` prefix for Scatterhoard itself, so the namespace exists. An author record then merges
line-wise like every other line, supersedes through machinery that is already tested, travels in the
file, and passes the export-review gate like anything else — where a third record shape beside
memory and tombstone would need a format change, a reader change, and a merge story of its own.

**The name lives in the record's `content`, not in a second keyword** *(decided 2026-08-31 while
building)*. Keyword lookup folds case — `folded` is the index key, and that is deliberate — so a
keyword-borne name would make `JDoe` and `jdoe` one identity, which is disqualifying for the one
field whose job is telling two people apart. `content` also gives supersession something to carry:
a renamed identity is a new record with new content, which is an ordinary correction rather than a
special case. `AUTHOR_KEYWORD` in `src/store.rs` is the reserved marker, and `Store::author_name`
resolves an id to the name at the tip of its chain — returning nothing where the chain has several
tips, since that is one identity split in two and picking either would invent the answer.

### It reuses the graph's machinery without joining the graph

**`by` is an id-valued field, not an edge in `links`, and the difference is load-bearing.** Traversal
expands uniformly across labels by design, so an author record reached as an ordinary link would
acquire an in-degree equal to *every memory in the set* — a hub one hop from everything, dragged into
every recall as a related memory, distorting the walk it sits in. Holding the id in a field gets id
resolution, supersession chains and the dangling-reference report; joining `links` would additionally
get the walk, which is the one part that must not happen.

The same distinction the model already draws for [`context`](#resolved-context-field): carried on the
record and deliberately kept out of what gets traversed.

### One chain per contributor, not one record per set

Unlike `hoard-scope-topics`, the keyword gathers a **class** rather than a singleton: a set with four
contributors holds four author records and four independent supersession chains. Two consequences
follow.

**Finding your own record is exact, and searching for it is not.** Recalling `hoard-author` returns
every contributor's current tip. The unambiguous route is to follow the `by` of a memory you wrote,
which names your record by id and cannot pick up someone else's.

**The hazard is minting a second record instead of superseding your first**, which leaves one person
with two chains, two names, and memories split between them. Nothing detects it: two author records
with different ids are exactly what four contributors look like. The procedure has to say *find and
supersede*, never *store a new one*.

### The sequence, and why superseding alone is not enough

Superseding an author record leaves the old record readable, carrying the old name — one record
rather than 155, but still there. Where the motive is unlinkability rather than correctness, the
sequence is **supersede, then forget the superseded record**: a tombstone carries no content, so the
name is gone from the current file, and memories pointing at the old id still resolve forward through
the supersession. Note the tombstone's own `forgotten.by` names *whoever forgot it* — by then, the
new identity — and not the original author, so it opens no trail of its own.

Both operations exist and compose; what makes this worth writing down is that *supersede it* is the
answer everyone reaches for first, and it is half of the answer.

### Absorption composes with it, without rewriting anything

Absorbing another set brings its author records in [as authored](multi-set-sync.md), so one person
who contributed to both sets ends up with two records and two ids in the merged set. Unifying them is
one supersession, and **no memory's `by` has to change**, because resolution follows the chain. That
is the same property that made [absorption itself a supersession](multi-set-sync.md): the merged-away
reference keeps working because the graph carries the redirection, not because anyone edited the
records that point at it.

### What it costs, and the window it has to land in

**Every record's checksum moves**, because `by` changes shape. That is the rewriting-rather-than-
superseding act, safe only while no second store holds these sets — true as of 2026-08-31, since
neither repository has a remote, and false from the first push. So this belongs *with* the
record-shape batch rather than after it.

**The cross-set links need backfilling in the same pass** *(found 2026-08-31)*. `target_set_id`
landed with set identity but nothing filled it in for links authored before it, so all nine
cross-set links across the two shared sets hold `NULL` — and `NULL` is also the permanent, legitimate
answer for a link naming a set nobody here holds, so the two are indistinguishable in the data.
`identify_link_targets` is authoring-only by design and must stay so, since filling an identity in
on arrival is the receiver-side rewrite `absorb-as-supersession` removed. That leaves the one-off
backfill to the same manual conversion as the attribution remap, which is the only place that can
tell *authored before the field existed* from *genuinely unknown*.

**The dangling-reference report has to cover attribution**, or the one failure mode this design adds
— an author reference pointing at nothing — is silent. That is code, but it is the existing report
learning a new kind of target rather than a new tool surface.

### Identity is a function of the set's origin

*The human partner, 2026-08-31.* One person holds several identities at once, and which is correct is
decided by where the set is published rather than by who they are: an account name on this Mac, `jtcours`
on GitHub, another at work, others again on GitLab or Codeberg. **Divergence between sets is
therefore the correct steady state, not drift to be reconciled.** Three consequences follow, and they
decide most of what is below.

**Per-set is the data model, not a granularity choice.** An author record lives inside a set, so an
author id has no meaning outside the set holding it — a machine-wide identity is not merely
unenforced, it is inexpressible. Every mechanism here is per-set for that reason.

**A cross-set rename would be wrong, not merely risky.** It would overwrite the work identity with the
GitHub one. So renaming is per-set, and the *partial rename* — changing two sets of three and leaving
the third carrying the old name — is not a failure state at all, because there is no end state in
which all sets agree.

**It restates the defect narrowly.** The problem was never that a machine account name is a bad name;
it is the right name for `local`, whose origin *is* this machine. The problem is that a machine-local
identity leaked into two sets whose origins are elsewhere. Re-derived from `hoard_sets status` on
2026-08-31: the 155 records wanting re-attribution are exactly `shared:scatterhoard` (80) and
`shared:coding-lessons` (75), and `local`'s 6 were never wrong.

**What an author record holds is context-dependent too** *(the human partner)*: whatever identifies
the person where the record will eventually be published — a GitHub user id, or a full name and title
for academic work. Not only the value but the *form* varies with the origin, which is what makes
every automatic source below unreliable rather than merely imprecise.

### Built 2026-08-31: what the type change settled

`by` is a `MemoryId` in the record, the store and the file. Four things were decided while building,
each because the suite refused the alternative.

**Identity records are never embedded.** They are not knowledge, and every set holds one, so leaving
them embedded puts a record matching nothing into the ranking of every semantic recall. Excluded at
the write rather than filtered at each read, so a read path added later cannot reintroduce them; they
stay findable by the `hoard-author` keyword, which is how this document says to find them. It also
made `no-non-match-floor` worse before it was fixed: that filed defect used to be theoretical and now
fires on every set.

**`memories` excludes them; `records` counts them; `identities` names them.** A set reporting one more
memory than anybody wrote is reporting infrastructure as content. But `records = memories + forgotten`
is the export gate's own arithmetic, so dropping identity records from one side without naming them
anywhere would have broken the one check that exists to catch off-by-ones. `export` now reports all
four and they reconcile.

**Resolution happens after every refusal.** Resolving may mint, which is a write — so a refused
`hoard_memorize` was leaving an identity record behind, breaking the promise that a refused call
stores nothing. Nothing below the mint reads attribution, so there is no cost to waiting.

**A rebuild reports vectors written, not memories offered.** `rebuild_vectors` counted its input,
which now includes records `embed_all` filters out. Reporting an intention rather than an outcome is
the shape that hid `project-stores-get-no-embedder`.

**Making `by` an id took away the field's self-description, so the tools have to supply it.**
`"by": "jtcours"` needed no gloss; `"by": "01M1G5HK..."` is inert without one, and an id nobody knows
is fetchable costs exactly the property it was chosen for — that resolving it is one cheap lookup
where resolving a name would be an ambiguous search. `hoard_recall` now says what the field holds and
`hoard_lookup` lists it among the places an id comes from. Stating it twice is the symptom rather
than the fix: there is nowhere else to say it while
[the response shape is unpublished](../../TRACKING.md#issues).

**And the demonstration is a test.** `an_identity_can_be_renamed_across_a_set_by_editing_an_export`
edits one record in an exported file and re-imports: `took_from_file: 1`, `unchanged: 2`, and both
memories resolve to the new name. Before this change the same test edited every record's `by`. On
these sets that is 1 instead of 155.

**A database written before this change cannot be read either**, and the same reasoning applies as to
a file: whether two names are one person is not answerable from the store, so it is refused rather
than migrated, and the live sets are converted by hand while the window is open.

### Done 2026-09-01: the three sets are converted

Run by hand against the three live stores while the window was open, which is the only place the
judgement it needs exists — *which of these names are one person* is not answerable from the data.

| set | identity record | repointed |
|---|---|---|
| `shared:scatterhoard` | `jtcours` | 80 memories, 5 tombstones |
| `shared:coding-lessons` | `jtcours` | 77 memories, 5 tombstones |
| `local` | the macOS account name | 6 memories, 31 tombstones |

**One record per set and no supersession**, because a half-finished rename converted while nothing
else holds these sets is a rewrite rather than a correction: the account name now appears **nowhere** in
either shared set — not in a record, not in a superseded record, not in a tombstone. Done after the
first push the same outcome would have cost a mint, a supersession and a forget, and still left a
link saying *an identity changed here*. `local` keeps the account name, which was always the right name
there.

**The nine cross-set links were backfilled in the same pass**, seven in `scatterhoard` and two in
`coding-lessons`, and no `target_set_id` is null in either export. That had to be here rather than in
`import`, which cannot tell *authored before the field existed* from *genuinely unknown*.

Verified through the tools rather than through SQL: `status` reports 80/77/6 with identity records
excluded from `memories`, both exports reconcile as `memories + forgotten + identities = records`,
recall by the `hoard-author` keyword returns all three records self-attributed, and a semantic recall
returns ordinary memories without surfacing any of them — they carry no vector, so they cannot.

### Built 2026-09-02: the durable map, and what it cost `create`

The three layers are real now. The set's own `meta` answers every ordinary write in one read of a
store already open; a table in `local` — `set_id`, author record, scope label — is consulted only on
a miss; the question is last. `AuthorRow` and `author_map_get`/`_put`/`_forget`/`_rows` in
`src/store.rs`, orchestrated by `Server::author_for`, because the durable copy lives in a *different*
store from the one being written to and only the server holds both.

**The map exists in the local store alone, and `require_local` enforces it.** A shared store asked
for the map is refused rather than answering emptily: several copies of a machine-wide fact, with
nothing to say which one answers, is worse than none.

**A row is checked against the set, never trusted.** It can name a record this copy does not hold —
restore a set from a file older than its identity record and that is the ordinary case — and adopting
it would point every write at a record that is not there, which is the one failure `by`-as-a-reference
introduces. A row that does not resolve reads as no row at all, so the set mints or asks.

**`create` stopped minting, and the suite caught the reason.** It resolved an author at creation,
which meant adopting a set you already contribute to — `create` then `import` — minted a second
identity record *before* the first arrived. Two chains for one person, which this document says
nothing downstream can detect, produced by the tool itself. `create` then recorded the name via
`record_author_name` and minted nothing; the record is resolved at the first write, when the set's
records are present. That is what this document already prescribed, and the code had not followed.
*(Since 2026-09-28 `create` records no name either, and `record_author_name` is gone: a shared set's
first write asks who you are in it — [Set Lifecycle](set-lifecycle.md#built-2026-09-28-create-makes-an-empty-set-and-its-first-write-settles-it).)*

**The case it was named for works as of 2026-09-02**, once a set could take back the identity in its
own file. `adopt_set_id` had refused to displace any identity a set held; it now refuses only one the
set has *written under*. A set holding no record has attributed nothing to what it minted, so
replacing it contradicts nothing, and after adopting it has records and never adopts again — which
converges for the same reason filling an absence does. Until that changed, `create` then `import`
left a re-added set under a fresh identity and silently dropped its file's, so nothing keyed on
identity could survive a drop. The map serves the other misses too: a second checkout sharing one
`local`, and any set whose cache is gone while its identity is not.

### Built 2026-09-02: the format floor moves to the cut

`FORMAT_VERSION` is `0.4.0` and `OLDEST_READABLE` moves with it, so this build reads exactly what it
writes and refuses everything before the attribution change. That is what a cut with no migration
means, and the two constants being equal is a real state rather than something to tidy — they
separate again at the next merely-additive change.

**"Merely additive" means a change an older reader cannot parse, and a field it ignores is not one.** Decided 2026-09-13, when the header gained `summary` under `derived-set-summary` and the version stayed at `0.4.0`. Below `1.0` the minor is the compatibility unit, so bumping to `0.5.0` would have made every `0.4.0` build refuse these files as newer — and wrongly, because `Header` does not deny unknown fields, so a `0.4.0` reader already reads them correctly by skipping the field. Nor does it lose anything by skipping it: a summary is a claim a reader recomputes and checks, never a fact it relies on. **Removing the field on 2026-09-21 took no bump for the same reason**: a `0.4.0` reader defaults the absent field, and a current one skips it in a file an older build wrote ([export-review](export-review.md#4-what-the-set-amounts-to--derived-shown-and-never-judged)). A version bump is for a shape an older reader would misread or fail on — a changed field, a new record kind, a field that reader would need to understand. An optional field it can safely ignore takes no bump, and the two constants separate only at a change of that first kind.

**The bump belonged with the record-shape change and was missed.** `by` changed shape while the
constant stayed at `0.3.0`, so for one commit two shapes shared one number, which is precisely what
`FORMAT_VERSION`'s own documentation says to bump for. The two exports written in that window were
corrected to `0.4.0` rather than left declaring a version that describes two shapes, and both were
read back with the new build to check that the correction was all that was needed.

**Refusing rather than half-reading is the point.** Before this, a pre-cut file reached the reader
and failed at the first record with an error saying the account name *is not a well-formed memory id* — a damage report,
which sends someone looking for damage that is not there. The refusal names both versions and the
only thing that helps: re-export from a build that still writes that shape.

The test that claimed every version this project has written is still readable is gone, because the
claim is now false and it never tested it anyway — it swapped the version string on a file *this
build wrote*, so nothing old was ever parsed. What replaced it pins the gate's actual contract: the
declared range reads, and each side of it is refused with the message carrying the right remedy,
since *update this build* and *re-export from an older one* are opposite instructions.

### Built 2026-09-02: a cross-set link resolves by identity, not by name

The last of the record-shape batch, and it was **written and never read**. `target_set_id` landed
with set identity, travelled in the file, and was backfilled across nine links — while both places
that locate a cross-set target found the holder by comparing `SetName`. The field existed for two
failures it was not preventing: a set renamed locally, whose links then reached nothing; and two sets
sharing a name, whose links reached the wrong one.

`holder_of` now answers both, at cross-set supersession resolution and at edge walking, on one rule:
**an identity, once stated, is not overridden by a name — and does not fall back to one.** If the
identity names no loaded set, the target set is not here, and a loaded set that merely shares the
name is a *different* set. Taking it would be the ambiguity `set_id` exists to remove, and it is
worse than not resolving: it attaches one set's concession to another set's memory. A link carrying
no identity — written before the field, or by a machine not holding the target — still resolves by
name exactly as before.

**The wrong-set case is not theoretical, and the test that found it showed why.** With the namesake
set loaded, the conceding memory did not come back from recall *at all*: it was substituted by a
memory from a set it had never named. A reader would see a replacement with no way to tell it came
from somewhere else.

### Whether `local` gets author records: yes

**Decided 2026-08-31.** `local` has one author by construction and never publishes, so uniformity is
the visible argument — but the load-bearing one is the format. Exempting `local` makes `by` a
[`MemoryId`](#resolved-memory-ids-are-uuidv7) in shared sets and a bare string in `local`: a union in
the record shape, permanently, to save one record. The record-shape batch exists to spend the
pre-remote window removing exactly that kind of variability, so introducing a new one in the same
change is self-defeating.

It also removes a special case from the machine-side table below, where `local`'s own row becomes one
row among the others.

The honest cost: inside `local` the record proves nothing. And it lands in the same bucket as the
filed two-laptop concern — `claim_set_id_if_local` already mints per machine — so it adds no new class
of problem there, and the remedy is the same single supersession at merge time.

### Which record is you is machine state, not set state

*Raised by the human partner: if someone drops a store and re-adds it, how does the server know which
record is them?*

**The split that answers it**: the author records are **set** data — they travel in the file and reach
every holder, which is the point. *Which of them is the local user* is a fact about **this machine**,
like the directory a set is exported to. Storing it in the set would be the actual mistake, since it
would mean nothing to anybody else holding that file.

**But it cannot live in the set's own store alone, which is where this proposal first put it.** There
is one database per set — `Store::path_for` gives `shared-<name>.sqlite3` or `local.sqlite3` — and
`drop` unlinks that file. A mapping held only in the set's `meta` therefore dies with the set, and
re-adding it meets a question already answered. *Losing it when the database is destroyed is correct*
was right about the whole store and wrong about one set of it.

**So two levels** *(the human partner, 2026-08-31)*:

| layer | consulted | holds |
|---|---|---|
| the set's own `meta` | every write | the resolved author record id |
| a table in `local` | on a miss — first contact, or re-add after a drop | `set_id` → record id, and the scope as a label |
| the question | both miss, and it is ambiguous | — |

The set's `meta` is the cache, so `by` still costs one `meta` read per write in the store already
open, as originally costed — the durable table is not on the write path. `local` is the right home for
that table: it is machine-bound by scope type, every checkout reaches it, and `drop` accepts only
`Scope::Shared`, so the durable copy is out of reach of the one surface that would destroy it.

**Keyed on `set_id`, never on the set's name.** A local name is precisely what `set_id` exists to stop
being authoritative — `a-hand-merge-into-a-new-set-name-leaves-supersessions-inert` is that failure —
and keying on identity is also what lets a second copy of one set resolve to the same record without
asking again.

**The scope rides along as a label, never as a key** *(the human partner)*. It lets someone prune rows
for projects they have finished without the set being present, since human memory is enough to
recognise a name. Refreshed on every contact, it can only be stale for a set that is absent — exactly
the case where a misread costs nothing, because deleting a row is safe: the table below either mints
where nothing can collide, or asks.

**No last-used date** *(considered and rejected, the human partner)*. It is redundant where it is
checkable and unverifiable where it is not: for a loaded set, when an identity was last used is
already in the set, since every memory carries `by` and `modified.at`; for an absent set the date
would be all you had, and it supports no action.

**A table rather than `meta` rows**, since it is three columns over many rows where `meta` is
key/value. It sits beside `meta` in the local store, and nothing about it exports.

**Updated rather than invalidated, and there are three triggers.** `rename` and `identify` both name a
set, so both hold its store open and know the new tip: they write through, where clearing would only
force re-derivation of an answer already in hand. The third is records arriving by `import`, `restore`
or `absorb` — your record may have been superseded in the incoming file, by you from another machine,
leaving the cached id a valid reference that is no longer the tip. That one cannot be write-through,
because no identity act happened locally; it re-resolves to the tip on arrival.

**Rebuilding the cache is where the danger is, and it is [the duplicate-chain
hazard](#one-chain-per-contributor-not-one-record-per-set) arriving automatically rather than by
mistake.** The obvious rebuild — match the configured author name against the `hoard-author` tips and
adopt what matches — fails silently for exactly the person this whole proposal is for: someone who
has changed the name they publish under in configuration but not yet superseded their record matches
nothing, and a rebuild that mints on no-match hands them a second chain. Two chains for one person
are indistinguishable from two contributors, so nothing downstream can report it.

**So first contact may neither mint silently into a set that has author records, nor guess which one
is you:**

| the set holds | what first contact does |
|---|---|
| no `hoard-author` records | `local`: mint one under the account name — nothing to collide with. A shared set: **refuse until someone is named**, with `mint` or `identify` *(since 2026-09-28)* |
| any author records at all | **ask which record is you** |

**The auto-adopt row is gone** *(2026-08-31)*. It read *one tip matching the configured name → adopt
it, cache it, and say so*, and [identity being a function of
origin](#identity-is-a-function-of-the-sets-origin) removes its premise: there is no machine-wide *who
I am* for a name match to be evidence about. Worse, the configured name cannot say what it is —
`whoami()` collapsed `SCATTERHOARD_AUTHOR`, `$USER` and `"unknown"` into one string in `main.rs`, so
nothing downstream could distinguish a name someone chose from their operating system login *(until
2026-09-28, when `whoami` gave way to `account_name`, which supplies `local`'s default and nothing
else)*. That
collapse is the mechanism that put a machine account name on 155 records, and a match against it is a
coincidence rather than evidence.

**The rule covering both rows: never write an identity into a set from a name the operating system
supplied.** Dropping the adopt row alone would leave the defect armed in the mint row, which fires for
any file exported before author records existed.

**Defaults are offered, never written**, in precedence order:

1. `SCATTERHOARD_AUTHOR`, because someone took the trouble to set it and that is the best evidence
   available.
2. `$USER` — **`local` only**, and written silently there rather than offered, since `local`'s origin
   is this machine and the machine's name for you is the correct one. A new user who only ever uses
   `local` is asked nothing.
3. Otherwise a blank, because a blank gets filled in where a plausible wrong pre-fill gets accepted.

*As built 2026-09-28, simpler than this list:* `SCATTERHOARD_AUTHOR` is gone, and nothing is offered
for a shared set — its first write is refused until someone runs `mint` or `identify`, and the name
is theirs to type. The second row stands as written, reading `USERNAME` on Windows, which does not
normally set `USER`, and `USER` then `LOGNAME` elsewhere. Where none is set, `local` is refused like
any set rather than recorded as `unknown` — [Set Lifecycle](set-lifecycle.md#built-2026-09-28-create-makes-an-empty-set-and-its-first-write-settles-it).

**Git is not a source, and the reasoning is worth keeping** *(2026-08-31)*. Two forms were considered
and both fail. `git config user.name` is the wrong kind of string — a display name where a handle is
wanted — and with repo-local config unset, as it usually is, resolves to the *global* value: one
string per machine, which is the very defect being removed, wearing more credible provenance. Reading
the owner component out of a remote URL is better shaped, being a handle in the origin's namespace
that varies per repository, but the owner is not the author for an org repo, a fork, or anything you
contribute to without owning, so it can offer someone else's handle as your identity. It is also
unavailable when most needed: this design has to work *before* publication, because the record-shape
window closes at the first push, and neither repository has a remote today. Above all, since [what an
author record holds is context-dependent](#identity-is-a-function-of-the-sets-origin), no automatic
source can be trusted to produce even the right *kind* of string. The remote-URL form is filed in
`TRACKING.md`, reachable once the sets have remotes.

**`create` should require an author** *(the human partner, 2026-08-31; superseded 2026-09-24, when the
same person moved the question to the first write, where the set's records are present and the caller
holds the remedy — [Set Lifecycle](set-lifecycle.md#built-2026-09-28-create-makes-an-empty-set-and-its-first-write-settles-it))*. It removes the silent default
that produced this whole problem — a machine account name, stamped on 155 records because nobody was
ever asked. The precedent is `hoard_memorize` requiring `scope`, on the identical ground that what
cannot be taken back afterwards should be asked for at the front.

**But the name is chosen at `create` and the record is resolved at first contact, and collapsing those
two moments would fork exactly the person this is for.** Adoption is `create` then `import`. A
contributor adopting a set they already contribute to *from another machine* would, if `create` minted
immediately, mint a second record before `import` had brought their existing one in — two chains for
one person, which nothing downstream can detect. So `create` recorded **who you are**, and the table
above runs later, when the set's records are actually present. *(Since 2026-09-28 `create` records
nothing, and the name is given at the first write instead.)*

### Built 2026-09-02: the three operations, and what `rename` broke

`mint`, `identify` and `rename` replace `author`, which is gone rather than kept as a synonym: one
string could not say which of three acts was meant, and that was the whole reason for the split.

**Two tests asserting the opposite of the new behaviour were deleted, not repaired.** They said a
name change applies to *future writes only*, that stored memories keep the identity they were
written under, and that the response should count the split so it becomes visible. All of that was
true while `by` held a name — a stored record could not change without being rewritten. Holding a
reference removes the split rather than reporting it: nothing is rewritten and everything
re-attributes, because the memories point at a record and the record is what moved. There is no
*attributed otherwise* count now, because there is no otherwise.

**`rename` exposed a gap in `hoard_lookup` that had to be closed with it.** Forgetting the superseded
record means every memory written before a rename carries a `by` naming a tombstone — and lookup
answered *nothing here*, which is the same answer it gives for an id that never existed. The path
this project's own tool description sends a reader down would have dead-ended on exactly the ids it
was written for. Lookup now distinguishes the two: a forgotten id comes back under `forgotten`,
carrying no content — a tombstone has none — and naming where the chain goes. *What was forgotten*
and *what was never here* are different answers, and only one of them has a next step.

**Adopting a set is `mint`, not `identify`.** The tests that take on someone else's set were spelled
`author` and are now `mint`: importing a set brings its author's identity record along, and a
machine that has never written to it is a new contributor rather than an existing one under a
different name.

### `author` becomes three operations

**Decided 2026-08-31.** The existing `hoard_sets` action `author` takes a name and sets it, which now
conflates three acts differing in what they touch and in how badly they fail:

| operation | does | refuses when | visible to |
|---|---|---|---|
| `mint` | new record, new row | a row exists, or a record already matches the name | the set — exports |
| `identify` | points the row at an existing record | there is no such record | this machine alone |
| `rename` | supersedes my record with a new name, then forgets the old one | I have no record here | the set — exports |

**A single entry point inferring which was meant would be a [decision
placement](decision-placement.md) violation** *(the human partner)*. The caller can inspect one
precondition — recall `hoard-author` and see whether a matching record exists — and cannot inspect the
other, whether a row exists and what it points at. The system would then choose among three acts using
information the caller cannot see, while the stake — forking yourself into two chains that nothing
downstream can detect — is entirely the caller's. Distinct operations that refuse loudly on an unmet
precondition tell the caller that one of its assumptions about the system's state does not hold, which
is the thing inference cannot do. The precedents are `create` refusing over an existing database and
`hoard_memorize` demanding `scope`.

**`rename` performs supersede *and* forget** *(the human partner)*. Superseding alone is [half the
answer](#the-sequence-and-why-superseding-alone-is-not-enough) and is what everyone reaches for first,
so an operation named `rename` that stopped there would be a half-measure wearing a complete-sounding
name. The halves stay reachable through the existing correct and forget tools for anyone who wants one
alone. Two orderings matter: point the row and the set's cache at the new tip *before* forgetting the
old record, or both briefly name a tombstone; and the supersession link that survives points at an
empty tombstone, so the structure of *an identity changed here* stays visible while the name does not
— the bargain [the sequence](#the-sequence-and-why-superseding-alone-is-not-enough) already struck.

### What it does not buy

It makes attribution no harder to forge. A contributor can author an author record naming anyone,
exactly as they can write any string into `by` today; in an unsigned format, whoever can alter one
field can alter the rest in the same edit. This is a change in *where the name lives*, not in what it
proves.

### Open

- **Whether the first author record attributes itself.** Self-reference is well defined — `by` is not
  in the graph, so nothing walks it — but it is worth stating rather than discovering. The same
  question applies to the record `rename` creates, where the tombstone precedent argues for
  self-attribution: `forgotten.by` names whoever forgot it, which by then is the new identity.
- **What becomes of the existing `meta.author` string.** All three sets hold one today, and it is the
  input to `author_or_claim`. Whether it is migrated into a minted record, kept as the offered
  default, or dropped is undecided.
- **Where the question gets asked** — narrowed rather than settled. The table above puts it at first
  contact, which makes `hoard_memorize` newly refusable for a reason having nothing to do with the
  memory being stored: a bad place to meet a question. The two-level cache makes it rare — once per
  set per machine, and *not* again after a drop — but rare is not the same as well-placed. `import`
  remains the better candidate on [decision placement](decision-placement.md) grounds: it is already
  deliberate, it is where the author records arrive, and answering there means the answer is ready
  before anyone tries to write.

  **The objection to asking at import dissolves** *(the human partner, 2026-08-31)*: a holder who
  only ever reads is not a case to be detected, it is a **read-only evaluation load** — the shape
  [how to inspect a memory set without adopting it](../../TRACKING.md#open-questions) was narrowed to on
  2026-08-29. Such a load mints nothing and is asked nothing, because it is declared rather than
  inferred. That couples the two questions: whichever way the evaluation load is built, it is what
  keeps merely *reading* a set from writing your name into its contributor list.

## Resolved 2026-09-11: a set declares its languages on its charter

**The languages a set is written in are declared as keywords on its `hoard-scope-topics` charter** — `hoard-lang-en`, `hoard-lang-zh-hans` and so on, several of them where a set is genuinely mixed.

**Review is what decided it, not machine-readability.** Many sets will live in `git` for open-source projects, which will want to standardise the language of their memories exactly as they standardise the language of their code comments. So the declaration has to travel with the set and be visible in the JSON a maintainer reads. Going multilingual is a cost imposed on everyone who imports the set: each of them needs the larger model ([Embeddings](embeddings.md)), and each loses discrimination for it — 23 points of recall@5 in English, measured 2026-09-11. A change with that reach belongs in front of a reviewer, which is what rules a `meta` row out rather than merely making it inconvenient: it stays on one machine and tells a collaborator nothing.

A keyword is also compact, supersedes like any other claim — the charter is one per set, replaced by superseding it — and reads familiarly to anyone who has met a language tag on the web. Prose in the charter was rejected because no server can act on it, and a second reserved memory because it adds another per-set singleton beside the charter.

### What consumes it — decided and built 2026-09-14

**The session is the first consumer, not only the reviewer.** The human partner's point, when the build began: a memory written in a language the loaded model does not represent can be stored and never found by meaning, and a query in one language barely reaches a memory in another. Under the shipping model a paraphrase beat an unrelated sentence by 0.40 in English, about 0.13 in Spanish, German and French, about zero in Russian, Hindi and Chinese, and only 0.02 to 0.08 across languages (2026-09-11). So the declaration tells a session which language to write in and to search in, and that is true of `local` exactly as of a shared set — `local` is never reviewed, but it is recalled from.

- **`recall-when` carries a clause**: write a set's memories, and phrase queries against it, in a language its charter declares, translating when you can, and keep keywords and identifiers as spelled. It sits after the charter recall that already puts the keywords in front of the session. Served as a convention rather than in tool descriptions because the language is chosen while composing a memory or a query, before a tool is reached for — [Conventions](conventions.md).
- **`hoard_sets status` reports each set's declared `languages`**, read from every live charter, and adds a `language_note` to every set that declares none — `local` included, since it is never made through `create` and nothing else would prompt it. A set with no charter at all is told the charter is what is missing. It reports and never refuses. The code is `Store::declared_languages` and `Server::set_status`; the contract is `tests/set_languages.rs`.
- **`create` asks for the declaration** in the same sentence that asks for the charter.

### The model-fit report — decided and built 2026-10-04

**The report exists to prevent confusion, not data loss.** A set written in a language its model cannot represent still stores every memory, but each vector is close to arbitrary, so recall by meaning returns noise. The memories are intact, keyword recall works, and recall by meaning returns once a model that covers the language is loaded. What goes wrong is the agent's reading of what it gets back, so the report says what a poor fit means, not just that one exists.

- **A score of 0 to 10 per declared language**: how many of ten paraphrase triplets the model ranks first, each anchor's paraphrase against every other sentence in that language — 29 rivals. A rank rather than a similarity, because similarity's scale moves with the model (unrelated Russian sentences average 0.87 under `potion-base-32M` and 0.31 under a retrieval model). **The rival count is part of the measure**: the same Japanese triplets win 10 of 10 against one unrelated sentence each and 5 of 10 against all 29. A tie loses.
- **Three bands, each with its own message.** 7–10 is reported as a score alone. 4–6 says recall is unreliable and a model built for the language would likely do better, possibly at the cost of other languages in the same set — a trade that is the user's to weigh. 0–3 says the model cannot search the language by meaning, that the memories are intact and keyword recall works, and that a covering model restores it. The middle band says *switch models* rather than *partly works* because a 4 can come from a model that has not separated the language at all: Japanese scored 4 under the English model with its paraphrases behind their rivals on average.
- **Measured 2026-10-04**, ten triplets per language: under `potion-base-32M` English scored 10 and every other language 0–4; under `potion-multilingual-128M` every language scored 5–10. Ten tests leave about ±1 of noise, so a language near a boundary can flip bands; that changes only how strongly a model change is urged, never what is said about the memories' safety.
- **Where it is reported**: `hoard_sets status` gives `language_scores` keyed by declared tag, and puts the band messages in `language_note`. A recall by meaning carries a `language_note` naming each set searched whose languages fall below the top band; keyword recall carries none, since no model takes part in it. Nothing is scored for a set that cannot be searched by meaning at all — no model, or one its vectors were not made by — because a score would blame the language for what `embedding_model` already reports.
- **Worked out once per language per server process**, since the answer depends only on the model and the language; loading another model clears it. The declaration is read from the charter on every call.
- **The sentences**: built in for English, Spanish, German, French, Russian, Chinese (simplified), Hindi and Japanese, in `language_fit::BUILT_IN`. Matched on the language subtag, so `en-gb` is scored as `en`, but a script subtag must agree: `zh-hant` is not scored with simplified sentences.
- **A set's own sentences and recommended models — built 2026-10-04** — decided 2026-10-04 with the human partner and kept in the set rather than on one machine or in a README, so every copy scores alike, a maintainer can review them, and a recommendation cannot become separated from the set it is for.
  - **One model-fit hub per set**, separate from the charter: short prose carrying a reserved mark. It links out to sentence-set memories, one `[anchor, paraphrase, unrelated]` JSON array each, tagged with its `hoard-lang-` language. **Zero or exactly ten per language**, counted per language rather than as a total, because the rival count is part of the measure; anything else is refused when the hub is stored. One set per memory keeps each far below the 2,000-character limit, which has no exceptions — a single memory holding all ten would have been the first — and lets one bad set be fixed by superseding it alone.
  - **A set's own ten take precedence over the built-ins**, which lets a set measure its own vocabulary — English full of Italian cooking terms or French terms of art — that everyday sentences cannot test. For a language with neither, status asks the agent to supply ten: a set declared as Samoan is almost certainly being written by an agent that can write Samoan.
  - **Recommendations link in to the hub**, so adding one never supersedes it; tagged with the languages they were measured on, or untagged where they concern the whole set. One is refused when written unless it links to the live hub, since that link is the only way it is found, and a link to a superseded hub reaches its replacement. **Status lists them as `recommended_models` always** — whatever the scores, and with no model loaded, which is when knowing which to install matters most — each as its id, summary line and tags; the server never parses a model name out of the prose, so the opening must name it. **The band note for a language below 7 names the ones tagged for it or untagged**, turning the warning into a next step. Recall does not repeat them.
  - **Matched by language and script, ignoring region**, as the built-ins are: `en-us` and `en-gb` are one language, `egy-latn` and `egy-egyp` are two. A missing script matches any, and a declared tag that would match two script groups is left unscored and reported. Sentence sets for a tag no declaration matches are reported too.
  - **The marks** are `hoard-fit-hub`, `hoard-fit-sentences` and `hoard-fit-model`: three fixed marks in `CONTROL_MARKS`, not a family, so a mistyped one is still refused. A sentence set is refused at the write door unless it carries exactly one language tag and its text holds a readable triplet; a hub is refused for a second live one, or for a count other than zero or ten in any language. A file is never refused for either — status reports what it finds instead.
  - **What status says beyond the score**: a language with no sentences asks for ten, saying how to write them; a language whose hub lists other than ten readable sets is left unscored and said; a declaration matching two scripts is named rather than guessed; sentence sets for an undeclared language, and sets no hub lists, are named. These are setup work, so they appear in status and not on recall, which carries only the band messages.
  - **Scored by the sentences themselves**, cached per process like the built-ins but keyed by their text, so superseding one set rescores the language. **A band message from them names the set's use of the language** — "English as this set writes it" — because the score describes the set's vocabulary rather than the language: on 2026-10-04 the same model read everyday English at 10 and an English set's Italian kitchen terms at 1, and a note saying it could not search English would have sent the user after a model that does not help. Measured that day, `potion-multilingual-128M` read those terms at 2, so for borrowed terms of art the dependable fallback is keywords.
  - **How a session learns any of this** is served text alone, since a user has no copy of this repository. When a language scores below 7, status adds one how-to beside the bands: record a model that serves the set better as a `hoard-fit-model` memory linked to the hub — named by id where there is one, or shown as a call to make first where there is none — and measure an unusual vocabulary with ten sentence sets of the set's own. A refusal for an unlinked recommendation says the same, depending on whether the set has a hub. Tested on `gemma4:12b` on 2026-10-04: without the how-to it wrote an ordinary memory every time; with it, it recorded a correctly linked recommendation. Told only to "write the hub first", it linked to the charter and repeated the refused call, and showing the hub as a call fixed that. The cookbook carries the recipe for a user who arrives with the goal rather than a low score.
  - **A link naming its own set counts as a same-set link** in these checks, asked by `LinkTarget::is_foreign_to` exactly as the write asks it. The checks first read only the bare spelling and refused a recommendation the write would have stored correctly.
  - The code is `Store::fit_material`, `Store::fit_sentence_sets`, `Store::reaches_a_hub` and `Server::language_fit`; the contracts are `tests/model_fit.rs` for the write-time checks and `tests/set_languages.rs` for what status and recall report.

The code is `src/language_fit.rs` and `Server::language_fit`; the contracts are `tests/language_fit.rs` and `tests/set_languages.rs`; `probe_whether_each_language_ranks_a_paraphrase_first` in `tests/probes.rs` reproduces the measurements and asserts it agrees with the shipped score.

### What consumes it — still proposed, not yet decided

- **The note at the write door**, which today can only say the model has no tokens for this text, and could say instead that the set declares a language this model serves poorly.
- **The language of any calibration probe** generated for a set, which `floor-calibration-source` will need.
- **Detection cross-checking the declaration** — a script histogram over the set's memories, weighing the bulk of the text rather than the presence of any Latin in it, since identifiers and code are Latin inside memories in every language.

**Rejected: coverage derived from the model's own vocabulary, by script.** Measured 2026-09-19, Russian and Greek tokenise fully against `potion-base-32M`'s vocabulary and still collapse, so a vocabulary test would report both as covered. A mismatch still reports and never refuses, following *a store owns its model, and a mismatch is reported rather than repaired*.

Tracked in [Backlog](../../TRACKING.md#backlog) as two entries: the declaration half above is `charter-declares-set-languages`, built; this checking half is `model-covers-declared-languages`, deferred by the human partner on 2026-09-14, taken up on 2026-10-03 as the fix for `recall-is-latin-script-only`, and built 2026-10-04.

## Resolved: scope is not a field on a memory

An exported file *is* a memory set, so every memory in it shares one scope, and a memory read out of a file is scope-less as a value — it acquires its scope from the store it is loaded into. See [File Format](file-format.md) for the reasoning and for where the set is declared instead.

## Resolved: `why` is optional in the type, required by convention

The conventions push hard for every memory to carry its reasoning, and the type still allows its absence. That is deliberate rather than an oversight: a type that *demanded* reasoning would make the LLM invent some whenever it has none to give, which produces a worse memory than an honest omission — and a fabricated `why` is worse than no `why`, because the whole point of the field is to let a later reader judge whether the memory still applies.

This is the boundary between what belongs in the compiler and what belongs in convention. Scope cannot be forgotten, because the software genuinely cannot proceed without it. Reasoning can be absent, because the honest answer is sometimes that there isn't any.

## Resolved: memory IDs are UUIDv7

**Identifiers are UUIDv7** — globally unique without coordination, which is what distributed development requires, and *time-ordered*, which the rest of the design turns out to want.

The ordering is the part worth recording. [File Format](file-format.md) asks for a consistent export order specifically to keep merge diffs small, and names memory ID *or* creation time as the key. With a time-ordered identifier those are the same order, so there is one sort key rather than two that can disagree — and a newly stored memory appends near the end of the file instead of landing in the middle of it, which is the case `git` merges most cleanly.

Random identifiers (UUIDv4) would have satisfied uniqueness equally well and lost this for nothing.

**The ordering is held by Scatterhoard, not left to the `uuid` crate.** `Uuid::now_v7` can hand back an id that sorts before one it already issued: it reads the clock before taking its lock, and a reading from an earlier second that arrives late moves its stored time almost a second ahead, after which the next millisecond falls back to the true time. Found 2026-10-01 behind a test that had failed once in a full-suite run, and still present in the crate's latest release. So `MemoryId` and `SetId` are minted through one process-wide record of the last id issued, and a candidate that does not sort after it is replaced by the next UUIDv7 after it (`issue` in `src/memory/id.rs`). Ids from two processes still interleave within a millisecond, which nothing here depends on. `generated-ids-can-sort-out-of-creation-order` in [Issues](../../TRACKING.md#issues).

## Resolved: ids are written in Crockford base32

**Decided 2026-08-20 by the human partner.** The bits stay UUIDv7; only the spelling changes.
`019fc5fe-6bd8-7352-8406-c0b07244ad71` becomes `01KZ2ZWTYRED9881P0P1S49BBH` — 26 characters, no
separators. That is ULID's text encoding, so this is an existing named form rather than a bespoke one.

**The reason is the truncation instinct, not the length.** [Recall Mechanisms](recall-mechanisms.md)
recorded the originating observation years before it became a defect: models truncate a UUID when
recording a reference, *"and this approach is common in conversations about `git` commits."* What
transfers is **a habit whose validity depends on a property these ids lack** — truncating a hash is
sound because hash output is uniformly distributed, so every prefix is equally selective; truncating a
UUIDv7 is unsound because the entropy sits at the end.

**Two features of the hex form invite the cut, and the separators matter at least as much as the
alphabet.** The 8-4-4-4-12 grouping offers a pre-segmented unit, so taking the first group feels like
taking a natural boundary rather than choosing an arbitrary length — you are not deciding how much to
keep, you are keeping "the first part". Every truncation recorded in this project landed on exactly
eight characters, which is both the first group and git's abbreviation length; the two causes are
confounded and the base32 form removes both, having no separator to cut at and no hex alphabet to
read as a hash.

**Measured before deciding**, across the 84 memories in the three live sets. Collisions out of 84:

| fragment | from the head | from the tail |
|---|---|---|
| 4 characters | 82 | 0 |
| 6 characters | 61 | 0 |
| 8 characters | 0 | 0 |

A six-character window slid along the id is unique at **every offset except 0 and 1**: nineteen of the
twenty-one possible windows identify a memory outright, and a prefix takes one of the two that do not.
Read those zeroes with their referent — a tail fragment carries five bits per character, so four
characters stay safe to roughly a thousand memories, six to about 32,000, eight to about a million.

**Crockford rather than plain base32**, because it defines mappings for confusable characters — `I` and
`L` to `1`, `O` to `0` — so a fragment transcribed slightly wrong still resolves. That is exactly the
failure a recovery mechanism exists to absorb.

**No type sigil.** An `m_` prefix was considered and rejected. It addresses the *semantic* truncation
instinct that the ULID form already removes, while doing nothing about *display* truncation — fitting a
column — which no format prevents and which the opt-in recovery below makes harmless anyway. It would
also make the head of the id the visually salient part, which is the region carrying no identity, and
would cost compatibility with any ULID reader.

**Ordering survives, which is load-bearing.** Crockford base32 is order-preserving over the same bit
sequence, verified against the real ids. [File Format](file-format.md) sorts by id to keep merge diffs
small, and keyword recall breaks ties on ascending id ([Recall Mechanisms](recall-mechanisms.md)); an
encoding that reordered would take both.

## Resolved: the canonical link labels

**Where the set is defined: in software.** The deciding argument is what happens to an unrecognised label. Defined in software, it fails at import, where a person can see it. *(The failing half was reversed on 2026-09-25 — an unknown label is now carried opaquely; see [Unknown labels](#resolved-2026-09-25-unknown-labels-are-carried-opaquely) below. The half about traversal stands: nothing walks an edge it does not understand.)* Left to the conventions, it becomes a label that parses fine and that nothing knows how to traverse — a memory silently disconnected from the graph it was meant to join. Traversal is code, so the set traversal understands is the real set either way; writing it down anywhere else just creates a second list to drift.

**Which labels.** Fourteen, adapted from a vocabulary proven over several months of daily use in a larger memory system, then tested against a corpus of real Scatterhoard memories written before the vocabulary existed ([corpus/link-label-findings.md](../../corpus/link-label-findings.md)). Keeping the names close to that system's is deliberate: it makes importing from it largely mechanical.

Every row reads **"this memory … that one"**, because every link is written outbound from the memory being stored — see below.

| label | this memory … |
| --- | --- |
| `RELATED_TO` | is associated with it; use when nothing else fits |
| `REFERS_TO` | mentions or cites it |
| `CAUSES` | is the cause of it |
| `RESULTS_FROM` | is an effect of it |
| `MOTIVATED_BY` | was decided because of it |
| `SUPPORTS` | is evidence for it |
| `CONTRADICTS` | conflicts with it |
| `SUPERSEDES` | replaces it — see [Corrections](corrections.md) for when this is the right instrument rather than amending or forgetting |
| `SUPERSEDED_BY` | is replaced by it |
| `RESOLVES` | answers it, an open question |
| `DEPENDS_ON` | needs it to hold |
| `EXCEPTION_TO` | is a sanctioned carve-out from that rule |
| `VIOLATES` | breaks that rule, without sanction |
| `INSTANCE_OF` | is a specific case of that pattern |

`PRECEDES` and `FOLLOWS` are **deferred, not rejected** — see [Early Validation](../../EARLY-VALIDATION.md).

### Links are always written outbound from the new memory

Across the whole test corpus, **29 of 29 edges pointed outbound from the newer memory** — including every edge using labels where the new memory's side was expected to be free. This matches what the larger system's author observed over months of use: the model reaches for a relation *from* the memory it is creating, and passive-voice label names cause mislabels.

Three things follow, and they are why the set looks the way it does:

- **A label needs a reverse only where the new memory's side is genuinely free.** That is true just for causation, which is why `CAUSES`/`RESULTS_FROM` are a pair. For the rest, creation order fixes the direction: a memory cannot cite, replace, or resolve something that did not exist when it was written, so the reverse label would name a case that cannot arise.
- **`SUPERSEDED_BY` is the exception, and it exists for a scope reason.** If a project memory supersedes a local one, writing `SUPERSEDES` on the project memory is impossible (that set is not this machine's to edit) *and* is the trust inversion forbidden in [Cross-Set References](cross-set-references.md). The lower-authority side records its own obsolescence instead.
- **The table costs less because of it.** Since direction never has to be explained, each row is a bare predicate. Measured at 12.5 tokens per label against 14.3 for a `from`/`to` phrasing — the saving roughly pays for the labels added beyond the original twelve.

### What the corpus test changed

- **`REQUIRES` → `DEPENDS_ON`.** Three of four `REQUIRES` edges turned out to be "was motivated by" rather than a dependency. The name was the trap, so it was renamed and the real relation given its own label.
- **`MOTIVATED_BY` added, and it dominates.** Eight uses, three times any other label. The commonest relation in a design corpus is *reasoning*, not causation — and it had been travelling under a label named for causation. Physical causation accounts for three edges in total.
- **`SUPPORTS` added.** Four uses, all the same shape: a measurement corroborating a decision made on reasoning alone. `CONTRADICTS` already covered evidence going against a claim; the omission of the *for* case was an asymmetry with no defence.
- **`VIOLATES` added, and its absence had been doing harm.** Adding it emptied `EXCEPTION_TO`: the one edge so labelled was a violation, not a sanctioned carve-out. Without `VIOLATES` the failure is not a missing edge but defects silently recorded as approved exceptions. The two labels are the same shape separated by whether the deviation is sanctioned.
- **`CAUSED_BY` → `RESULTS_FROM`.** The only passive-voice name in the original set, and the sole label with a near-identical active sibling to be confused with. `MOTIVATED_BY` and `SUPERSEDED_BY` keep their passive form because neither has such a sibling.
- **`EXPLAINS` dropped.** Redundant against the per-memory `why` field, which this design has and the larger system does not.

### Constraints on any later change

- **The serialised names are format-visible.** They appear in every exported memory carrying a link, so changing the set once real memory files exist is a format migration rather than an edit.
- **The labels are a convention surface.** The LLM chooses among them at store time, and that guidance lands in the deliberately-small convention set (see [Conventions](conventions.md)). Notes toward it are recorded in `src/conventions/catalog.rs` under "Discovered, not yet drafted".
- **Per-label traversal behaviour is still open** — see [Open questions](../../TRACKING.md#open-questions). The vocabulary was chosen on how memories are *written*; how recall *walks* each label is a separate decision.

### Resolved 2026-09-25: unknown labels are carried opaquely

*Closes `unrecognised-link-labels-on-import`, with the human partner. Built 2026-09-28 as `unknown-labels-carried-opaquely` in [Backlog](../../TRACKING.md#backlog): `LinkLabel::Unrecognised` in [link.rs](../../src/memory/link.rs), `unknown_label_refusal` in [mcp.rs](../../src/mcp.rs), pinned by [tests/unknown_labels.rs](../../tests/unknown_labels.rs).*

**Why it could not stay closed.** Failing at import makes every addition to the vocabulary after publication break every older reader, exactly as a removal breaks newer ones, and an older build's strictness is fixed on the day it ships. So the tolerance has to be in the first published build.

**What an older build does with a label it does not know:**

- **Carries it.** Stored as text — the `label` column already is — and exported exactly as read, so checksums still agree and a round trip changes nothing.
- **Shows it, as written,** in recall and lookup.
- **Never walks it.** The original argument above survives here: an edge nobody understands is not guessed at.
- **Lets it be restated, never authored.** The tool surface restates links rather than taking add/remove deltas: a supersession carries exactly the `links` it is given, and `link_delta` reports what it left behind. So a build that could not name an unknown label could never keep one across a supersession. A write may therefore carry an unknown label only when it restates the same label and target from the memory being superseded — copying, which needs no knowledge of the label. Authoring a *new* link with one is still refused, so strictness still catches typos.

**Settled while building it, 2026-09-28, with the human partner:**

- **Amend restates on the same terms**, from the memory being amended: `hoard_correct` replaces the whole link array, which is the same trap.
- **"The same" is byte-for-byte**, label and target id — the pair `link_delta` compares. A respelled or retargeted unknown label is a new claim, and so is one copied from any memory other than the one being superseded or amended.
- **The schema names the vocabulary in the label's description instead of publishing an `enum`.** With an `enum`, a validating client refuses the restatement before the server sees it, and a model reading the schema never writes it, so the rule above would guard nothing. An `anyOf` of the enum or any string was ruled out by [tool-surface.md](tool-surface.md): a nested alternation is reduced to `{}` on one client. The cost is that a typo is caught by the server's refusal, which names the vocabulary, rather than by the schema. **The tool descriptions of `hoard_memorize` and `hoard_correct` name it too**, because Claude Desktop was measured dropping property descriptions while keeping the `enum` — so without it, a session there would see no vocabulary at all. Both copies are generated from `LinkLabel::ALL`.
- **Traversal excludes it by an exhaustive match.** `is_traversable` had been written as *not a supersession*, which would have made every unknown label walkable, and the compiler says nothing about a negation.

**Adding a label follows ordinary semantic versioning, and its author decides which kind of change it is.** The test: a label is a compatible addition if an older build that does only the four things above still leaves a set the newer build accepts; otherwise it is breaking, and takes the bump that makes older builds refuse the file whole. The session first proposed that any label carrying behaviour must be breaking; the human partner replaced that with this, because only whoever adds a label knows whether older builds carrying it opaquely is safe — a label that only refines ranking or display on newer builds is. `compatibility_of` in [memory_file.rs](../../src/memory_file.rs) already reads a minor bump as compatible from `1.0`; below `1.0` the minor is the breaking unit, so a compatible addition there is a patch.

## Resolved: Origin field for human/LLM attribution

Author alone doesn't let the LLM distinguish its own reasoning from the human's — Git tracks the committing user, but local prompting heavily influences an LLM's output, so "Author" can end up crediting a human for an idea that was actually the model's, and vice versa. That mismatch can confuse the human ("you determined X" when the AI determined it).

Resolution: add an **Origin** field alongside Author, with values `human`, `llm`, or `joint`. Author still records whose identity is attached to the memory (and is what Git/the review process cares about); Origin separately records whose reasoning produced the content. This is cheap to add now and avoids baking the ambiguity into the format permanently.

## Resolved: Context field

Motivating problem (observed in a larger, more mature memory system): the LLM would cite a recalled memory, and without further cues the human would only have a vague recollection of what specific situation it referred to — burning time working out what, specifically, was meant before the conversation could move forward.

### Why Context is a different axis from Content and Why

- **Content** is the substance of the memory — the fact, decision, or preference itself.
- **Why** is the background reasoning *for the content* — it lets the LLM (or the human) judge whether the memory still applies, or whether the current situation is an exception. It's about the content's validity.
- **Context** is neither of those — it's the episodic anchor: *when, where, and in response to what* the memory arose. It's not needed to judge whether the memory is true or still valid; it's needed to let a human *place* the memory once it's been surfaced, the way a person says "oh, right, that was during the outage postmortem" and everything clicks into place.

That's also why Context has to stay out of the embedding even though Why doesn't (see [Recall Mechanisms](recall-mechanisms.md)): Why is part of the memory's meaning, so it should help semantic recall find the memory. Context is about a specific past occasion, which is exactly the kind of thing that would drag semantically unrelated memories together just because they happened around the same time or task — it would add noise to recall rather than help it. It belongs strictly on the output side (glossing a memory for the human once recall has already found it), not the input side (helping recall find it).

### What it should contain

**What was being worked on when the memory arose, named the way a person would say it rather than the way the repository does.** "Making the author changeable. Documenting that `bg` is a memory ID." — not "`make-author-changeable` step 4". The goal is a foothold for the human's own memory, not a complete record.

**Work going on at the same time belongs there even when it has nothing to do with the memory.** This is the part no one derives, because the instinct is that a field attached to a memory should be about that memory. Human recall is associative, and a session runs as one thread — it meanders, but there is a through-line — so naming any part of that thread can pull the rest of it back. Adjacency in time is a handle where adjacency in subject is not.

**Leave out anything the record already carries**: the date (`created.at`), the id, the author (`created.by`). Leave out where something can be checked, which belongs in Content or Why, where it is embedded and read on recall. A date that is genuinely part of the claim — a decision taken weeks before it was written down — goes in Content.

### Resolution

- **Optional.** Not every memory will have a good anchor available, and forcing one would just produce noise.
- **Free text, not structured.** A fixed schema (e.g., separate task/session/trigger sub-fields) was considered and rejected as unnecessary complexity — a short freeform note carries most of the value, and structure can be revisited later if a real need for querying by context type emerges.
- **The guidance lives on the field, not in the convention catalog.** What to write here is decided with the tool schema in front of you, which is where [decision placement](decision-placement.md) puts it; the catalog is read at session start, before any of it applies. See the revision below for the evidence that the field's own description is what actually drives the behaviour.
- **Consumption**: this is the field to draw on when surfacing a recalled memory to a human — leading with what was going on at the time rather than restating Content cold. That behaviour is what fixes the motivating problem, and it belongs with the field's read-side documentation, which does not exist yet: no tool publishes an `outputSchema`, tracked as `response-shape-is-unpublished` in [Issues](../../TRACKING.md#issues).

### Revised 2026-09-04, from the corpus the first draft produced

**The original list was mostly identifiers, and identifiers are the thing that does not work.** It read: *the task, branch, or PR being worked on; the conversation or session it came out of; the specific question, bug, or decision point that prompted it; who else was involved.* Half of that is a pointer to an object rather than a name for an activity, and the shipped parameter description led with *when* — so the corpus filled with dates and file paths.

**Measured across both exported shared sets on 2026-09-04**, counting live memories: **86 of 88 `scatterhoard` and 89 of 91 `coding-lessons` memories carry a context — 98% of each**, at a median of 240 characters. A field designed optional is in practice near-universal, and it returned the description's own vocabulary: dates in 89% and 65% of contexts, occasion phrasing in 82% and 73%, and **checkability pointers in 72% and 62%** — `store-when`'s *name where that can be checked* being satisfied in the one field that is excluded from the embedding and never read on recall.

**Of 135 dated contexts, 103 sit 0 or 1 day from `created.at`** — the one-day cases are UTC rollover on evening sessions, not a different event — so three quarters of those dates restate a field the record already holds.

**Twenty-one of 89 `coding-lessons` contexts cite `LESSONS.md`, deleted 2026-08-15**, and no memory cites that file in Content or Why. The rot is perfectly confined to the field nothing re-reads. It was first read as staleness, and that diagnosis was wrong in a way worth recording: *"LESSONS.md in the Scatterhoard project."* names a location rather than an activity, so it could never have jogged anything — deletion made a pre-existing uselessness visible rather than creating it, and the real cost is that the slot where the recoverable occasion would have gone was spent.

**The change is forward-looking. Existing memories are not repaired**, decided 2026-09-04 by the human partner: repairing an exported memory's context is a supersession under [Corrections](corrections.md), and 21 chains cluttered with a version nobody believed differently is a poor trade for anchors that are already gone.
