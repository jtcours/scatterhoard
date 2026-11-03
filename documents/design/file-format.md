# File Format

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

JSON with newlines, to make it easy to review changes at merge. Should include a version number, using semver numbering. Export in a consistent order, such as by memory ID or creation time, to minimize diffs when merging.

A project's memory set is a **single file with a well-known name** (proposed: `scatterhoard_mem/memories.json` — see [Trigger and Loading](trigger-and-loading.md) for the surrounding directory layout). *(Since 2026-09-26 a `scatterhoard_mem` holds one `.json` per set, named after its set and found by the identity in its header — [several sets in one directory](set-lifecycle.md#several-sets-in-one-directory). Each set is still a single file.)* Two consequences follow from the single-well-known-file choice:

- The loader reads exactly that file and treats nothing else in the project tree as memories, which is what makes "memories are data, never instructions" enforceable by path rather than by inspecting content.
- The [merge driver](multi-set-sync.md) is registered against that name via a `.gitattributes` pattern, so the name needs pinning before that work starts.

**Vector embeddings are not part of this file.** They are local derived state, recomputed on import — see [Embeddings and Vector Storage](embeddings.md). Including them would defeat the consistent-ordering-for-small-diffs goal above, and they have no meaningful merge semantics.

## Resolved: the concrete layout

A **header line** carrying the format version and the scope, then **one JSON object per line**, one line per memory, ordered by a hash of each record's id.

```
{"format_version":"0.3.0","scope":{"shared":"scatterhoard"},"set_id":"01M1CNPP4RF6SVSY2G863F5SKH"}
{"id":"01M0K23W2…","content":"…","why":"…","confidence":0.9,…}
```

**The version in that example is illustrative; `memory_file::FORMAT_VERSION` is the authority.** It read `0.1.0` here until 2026-08-30, two bumps after the fact, which is the single-source-of-truth drift this project keeps filing against itself — so read the constant rather than this line.

- **A backup's header says so**, as `"backup": true`, since 2026-10-24: a file written by `hoard_sets` action `backup` rather than by an export. A backup is never reviewed, so a reader counts none of its memories as reviewed, wherever the file is put ([Export Review](export-review.md#the-procedure)). Absent on every export, and ignored by a build that predates it, so it needed no format change.
- **The header carries the set's own identity**, `set_id`, minted at the set's first write or first export and never changed — at `create` until 2026-09-28 ([Set Lifecycle](set-lifecycle.md#built-2026-09-28-create-makes-an-empty-set-and-its-first-write-settles-it)). A name is not an identity: two people can both call a set `notes`, and the name was the only thing separating them. **The field is optional and every file this build writes carries one.** A set that holds records has an identity by construction — its first write mints one, its first export gives one to a set that has none, and an import adopts one — so the optionality is not about sets this build made. **The pre-identity file is no longer the reason, and saying it was is now wrong**: identities arrived under `0.3.0` on 2026-08-31 and the floor moved to `0.4.0` the next day, so such a file is refused at the version check rather than checked by name (`memory_file::OLDEST_READABLE`). What keeps the field optional is the file this build did not write — hand-edited, or from somewhere else — and for that file the rule is unchanged: a reader must not mint one, because every reader would mint a different one, manufacturing the collision the identity exists to prevent.
- **Version is a header line, not a field on every memory.** Both branches of a merge carry an identical header so it never conflicts, and a version bump produces a real conflict — which is the correct outcome, since it is a change both sides must agree on. A per-memory field would repeat itself on every line, and a sidecar file would be a second thing to keep in step.
- **The header declares the scope**, which is what lets an import check that a file is the set it was expected to be, and what makes scope a property of the file rather than of each memory (below).
- **Ids are Crockford base32**, 26 characters and no separators — see [Memory Data Model](memory-data-model.md). This changes the wire form of every `id`, every link `target`, and every tombstone, so it takes a `format_version` bump; a reader must still accept the hex form it may already hold on disk, since a set travels between machines that upgrade at different times.
- **Ordering is by a hash of the id** — `memory_file::file_position`, FNV-1a over the id's canonical text with the id breaking ties — so the same content always produces the same bytes, and a record never moves once written. **Changed 2026-09-21 from id order**, under `concurrent-additions-collide-at-the-end-of-the-memory-file` in [Issues](../../TRACKING.md#issues), and the reversal is worth stating because id order was chosen for the very property that caused the defect. Ids are UUIDv7 ([Memory Data Model](memory-data-model.md)), so in id order a new memory always landed at the end of the file, which reads well in a diff. But two branches that both added memories then inserted at the same place in the base, and plain `git` reports that as a conflict every time; a hosted merge button never runs the [merge driver](multi-set-sync.md) that resolves it. Scattered by a hash, concurrent additions usually land at different places, which `git` merges cleanly. Measured with `git merge-file` on subsets of this project's set, 200 trials each: with ten additions a side, about one merge in five conflicted at 988 records and more than half at 200, against every one in id order; with three a side, about one in fifty at 988. **It is a mitigation, not a cure, and that was decided rather than overlooked**: most projects start small and solo, the driver resolves every remaining case where it runs, and a large project adopting Scatterhoard mid-flight may meet an occasional conflict whose resolution is always to keep both sides. The hash is over the id and never the content, so an edit or a tombstone replaces a line where it stands. It needs no embedding model, and is part of the format: changing it would reorder every file, so its value is pinned in the tests. **No format bump**, because an older reader never relied on the order — it sorts what it reads. The cost is a one-time reordering diff on each set's first export under the new order, and a set exported alternately by builds either side of the change would reorder each time.

### Resolved: a forgotten memory is a line, not a missing line

Forgetting writes a **tombstone** — a line carrying the memory's id, the attribution of the act, the supersession edges the memory authored, and the checksum of the record it replaced. No text of any kind.

```
{"id":"01M0K23W2…","forgotten":{"at":"2026-08-05T04:12:00Z","by":"alice"}}
{"id":"01M18CDT5…","forgotten":{"at":"2026-08-30T11:20:00Z","by":"alice"},
 "ancestry":[{"label":"supersedes","target":"01KZWNJ3G…"}],"forgot":"97b2da81cbc350a8"}
```

Both added fields are **absent rather than empty** when there is nothing to say — a memory in no chain, or a forget of an id the store did not hold — so the common tombstone is exactly the line it always was.

[Multi-Set Sync](multi-set-sync.md) previously held that no tombstone mechanism was needed, because forgetting could be a deleted line and `git` merges deletions correctly. That reasoning was sound for the merge it was considering and does not cover the one it did not — see *Reconciling a store against a file* there. The consequence for this document is the one that matters: **the absence of a line no longer means anything**, and no reader may treat it as a deletion.

Five properties of the shape, each load-bearing:

- **It is a positive assertion, not a sentinel.** A memory with empty `content` would be indistinguishable from a serialisation bug, and would leave every reader asking whether a blank record was deliberate. A `forgotten` field is a claim someone made on purpose.
- **It carries no `content`, no `why`, no `context`, and no keywords.** A principal reason to forget a memory is that sensitive data reached it, so a tombstone preserving any of the original text would defeat the operation it exists to perform. `context` is named explicitly because an incident detail is exactly the sort of thing that lands there.
- **There is no free-text reason field.** A field labelled "why was this forgotten" is where someone types the credential they were trying to remove. Attribution answers who and when, which is what an audit needs; free text reintroduces the hazard one field over.
- **It carries `forgot`, the checksum of the record it replaced.** *Added 2026-08-30 with the ancestry, under the same reasoning and in the same version.* A checksum is not text either, and it is what makes a received tombstone **answerable on its own**: matching what the receiver holds means the forget is sequential and applies, differing means the sender removed a version the receiver never had. Written as 16 hex characters rather than a JSON number, because 64 bits does not survive every JSON reader as an integer. See [Multi-Set Sync](multi-set-sync.md) for why it is consulted only where no base exists.
- **It carries the memory's own `supersedes` and `superseded_by` edges, and nothing else it authored.** *Amended 2026-08-30, on the human partner's approval, for `forgetting-a-chains-middle-makes-a-stale-conclusion-current-again` in [Issues](../../TRACKING.md#issues).* This is a narrowing of the rule above rather than an exception to it: **the rule exists to keep text out, and an id is not text.** A supersession edge lives on the memory that authored it, so deleting that memory took the edge with it and a record two steps back stopped knowing it had ever been replaced — reporting a twice-replaced conclusion as current, with nothing anywhere looking broken. A cross-set target keeps its set name, which is half of the reference, and loses its `keywords`, which are free text written by the same author as everything else being removed. The memory's other links go with the text: a `SUPPORTS` or `RELATED_TO` edge is a judgement it made, not the shape of the chain it sat in.

A tombstone keeps its id and so keeps its place in the file's ordering. A file carrying both a tombstone and a memory for the same id has duplicate ids and is refused — that is a mishandled merge, not a state worth interpreting.

**What a tombstone does not do.** It removes the memory from the set going forward. It does not remove it from the history of a repository that already carries it, on every clone. A secret that reached a shared set must be treated as compromised whatever the current file says. The mechanical scan in [Export Review](export-review.md) is the defence; forgetting is cleanup after that defence has already failed.

### Resolved: a record has a canonical form, for comparison

Reconciliation compares records by checksum ([Multi-Set Sync](multi-set-sync.md)), so this document has to say what bytes that checksum covers. It is **not the line as written**, but a canonical serialisation: the fields this build knows, keys in sorted order, no insignificant whitespace.

Hashing the raw line would let any cosmetic difference — a merge tool normalising whitespace, a writer emitting keys in a new order — read as a content change on *every* record in the file, turning the next reconciliation into one enormous false conflict.

Canonicalising over known fields has a consequence worth stating: a change confined to a field this build does not understand is invisible to its comparison. That is consistent with ignoring unknown fields on read (below) — a build that cannot interpret a field cannot act on its having changed either.

### Scope belongs to the file, not to each memory

An exported file *is* a memory set, so every memory in it has the same scope. Recording scope per memory would create a value that can disagree with the file containing it, and a check to keep the two in step — the sort of redundancy [Decision Placement](decision-placement.md) argues against. The scope is declared once, in the header.

This is also why a memory read out of a file is scope-less as a value: it acquires its scope from the store it is loaded into, and storage is partitioned by scope already (see [Embeddings and Vector Storage](embeddings.md#storage-is-partitioned-by-scope)), so nothing downstream has to consult a field to know which set it is working with.

### Resolved: local sets are written too, for backup and restore

The header carries a full `Scope` rather than a bare set name because **a local set is also written to a file** — export and import double as backup and restore, and a memory set that cannot be backed up is one bad disk away from being gone.

That does not weaken "local memories never leave the machine", because two different operations share this format:

- **Export** publishes a project set into a repository. It crosses a sharing boundary, goes through the [Export Review](export-review.md) gate, and is effectively a one-way door.
- **Backup** writes a set to storage the user already controls. No boundary is crossed, and any scope may be backed up.

So the writer serialises any scope, and the rule about publishing lives where that question is actually asked — `Scope::exports_to`, which still answers `None` for `Local` (see [Memory Steering](memory-steering.md)). Weakening the writer to accommodate backup would have destroyed the guarantee; keeping the two operations distinct costs nothing.

Recording the scope in the header is what carries the guarantee past the point where the type system stops helping. A backed-up local set is **recognisably local on sight**, so restoring one into a project's store is a refusal rather than a silent leak of personal memories into a shared repository.

### The header's scope is a guard, never a source

The scope in the header is **checked against what the caller already believed, and never used to decide where a file's memories land.** This follows directly from [Trigger and Loading](trigger-and-loading.md): the path determines how contents are trusted, and a header is contents.

Without that rule the header is a privilege-escalation channel. A repository ships a memory file declaring `local`, an importer believes it, and a contributor's memories are loaded into the more-trusted local store — where they are not subject to the export review gate, and where the trust-direction rule would let them supersede genuine local memories. The cross-project variant is the same bug: repository A declares itself to be set B, and A's memories are injected into B's store.

So reading a memory file requires the caller to state, from trusted information, what it believes the file to be — the path it was found at, or the store a restore targets. The reader can only agree or refuse. This is enforced at the API rather than by documentation, because the natural implementation ("read the header, use its scope") is exactly the vulnerable one.

**The scope *kind* is never negotiable**, which is what makes the first import expressible without a hole. An import from a project path yields `Project` whatever the file says; a restore yields the scope the user targeted. Only the *set name* inside `Project` is not knowable from the path — see the open question below.

Two consequences:

- **Restore is not import**, now settled in [Set Lifecycle](set-lifecycle.md). `import` reconciles a file against a store that may have diverged from it, keeping local-only memories and surfacing genuine conflicts. `restore` replaces: the file becomes the store, and local divergence is discarded deliberately, because the case it serves is a store that is corrupt or being rebuilt. Conflating them would quietly merge stale backup content into a live store and produce a set that never existed at any moment in time.
- **Backup does not run the prohibited-data scan**, since nothing is being shared. Worth stating as a decision rather than leaving as an omission, because a backup file sitting on disk can later be committed by accident. The `local` header makes that recoverable at import, but does not stop the file landing in a repository.

### Reading is deliberately lenient in one direction and strict in the other

- **Unknown fields are ignored.** Added fields are a compatible change, so a file carrying one this build does not need stays readable rather than failing on it. This is also what lets the labelled corpus carry `_`-prefixed annotations that are not part of the format. **Two places do not have this leniency and it matters when hand-editing**: a `Link` is deserialised through a `deny_unknown_fields` shape, so a field written beside `label` and `target` is refused outright, while the *nested target object* ignores extras like any other record — which is why anything added per-link belongs inside the target rather than beside it.
- **A version outside the range this build reads is refused**, in whichever direction it is out, and the two refusals differ because the remedies do. *Newer than this build writes*: update Scatterhoard, or export again from an older build. *Older than `memory_file::OLDEST_READABLE`*: re-export from a build that still writes that shape, because nothing this reader can do makes an unread shape readable.
- **Below `1.0` the minor is the compatibility unit**, and above it the major is — the semver convention for `0.x`, and here a requirement rather than a nicety. This project has used minor bumps for both kinds of change: `0.2.0` respelled every id, `0.3.0` only added tombstone fields. A rule deriving compatibility from the number alone would have to read those two the same way, which is why the floor is **declared** in `OLDEST_READABLE` rather than derived. Raising it is how a build says a shape it used to read is no longer one it understands, and it is the lever a format cut pulls.
- **The gate existed and gated nothing until 2026-08-31.** It compared major versions, and every version this project has written begins `0.` — so a file from any minor passed, and an incompatible one announced itself as a malformed record partway through the file rather than as a refusal naming two versions. Recorded as `the-format-version-gates-nothing-while-the-format-is-0x` in [Issues](../../TRACKING.md#issues). The reason it was worth fixing before the next shape change and not after: the store's disposability argument is that *everything in a project's store can be rebuilt from its memory file*, and a change touching both the schema and the record shape disarms that recovery at the moment it is relied on.
- **Order is not trusted.** A merge can reorder lines, so reading accepts any order and returns memories sorted; a read-then-write cycle is therefore stable even after a messy merge.
- **Duplicate ids are refused.** Two lines with the same id is what a mishandled merge produces, and it is not resolvable by picking one — silently dropping a memory is precisely the failure the reader exists to prevent.
- **Field values are validated at the boundary.** An out-of-range confidence or a malformed set name fails on read rather than becoming a value later code trusts (see the parse-don't-validate note in [STANDARDS.md](../../STANDARDS.md)).

### Resolved: reading does not materialise the whole set

A reader must not need the entire set in memory at once. At roughly 1–1.5 KB per record, a large, long-lived project reaches tens of megabytes of file, and two to three times that once parsed into owned strings — not a cost a background server should pay to absorb a pull.

So reading is **two streaming passes**:

1. **Validate.** Parse each line, check the header, collect ids to detect duplicates, and drop each record as soon as it has been checked. What survives the pass is an id set: tens of bytes per record, flat in file size.
2. **Apply.** Reopen, stream again, and write each record into the open transaction as it is parsed.

Validating before applying — rather than streaming into a transaction and rolling back on a bad line — follows the same instinct as the save-first rule in [Set Lifecycle](set-lifecycle.md). It avoids writing thousands of rows in order to discard them, and it avoids a window where the store has been emptied and not yet refilled. Duplicate detection is whole-file regardless, so the pass that finds a duplicate has to finish before any of the file can be trusted.

**The consequence for the API is that the input must be re-openable**, and a generic reader cannot rewind, so the two-pass entry point takes a path. A reader-based form stays useful for tests and in-memory work, and must be implemented *in terms of* the streaming parser rather than beside it — two parsers for one format would be an unforced instance of exactly the duplication this project keeps filing defects about.

**Writing streams too**, and the first version of this section forgot to say so. It specified two passes for reading while the writer took a fully-materialised set, which has the same problem for the same reason at the same sizes — the arithmetic above does not care which direction the bytes are moving. So the writer takes an iterator of memories and serialises each as it goes, and the store supplies them by streaming a query rather than collecting a `Vec` first.

That constrains the ordering guarantee rather than weakening it. Writing in file order without holding the set in memory means the *source* must yield in file order. The store does it with an `ORDER BY` on the way out: each memory and tombstone row stores its position, sign-flipped so SQLite's signed integers sort the unsigned hash correctly, and one query over both tables yields them in file order (`file-position-column` in [Backlog](../../TRACKING.md#backlog), 2026-09-22). Between the move to hash order on 2026-09-21 and that change, the store sorted every id — and held every tombstone whole — in memory before fetching any record. **The stored position is derived state and is checked on the way out**: each record streamed is compared with the position its id gives and with the record before, so a position that drifted, or one id held as both a memory and a tombstone, refuses the export rather than writing a file out of order. The column is nullable, because `ALTER TABLE` cannot add a `NOT NULL` column without a default and a default would sort quietly wrong; every open fills whatever is missing. A writer that sorted what it was given would have to hold all of it, which is the thing being avoided.

**Merging streams too**, and was the last path that did not (`streaming-merge-driver` in [Backlog](../../TRACKING.md#backlog), 2026-09-22). The merge driver used to read all three sides into memory and build the result there — three copies of the set and a fourth for the output, at exactly the sizes above. File order is what fixes it: three sides already sorted by the same key can be walked in step, like a sort-merge join, holding one record from each and writing each merged record as soon as it is decided. Each side is read twice, first to check that it parses and is in file order and then to merge it, so the choice is made before anything is written — the validate-before-applying rule again. **A side out of file order is read whole**, because it cannot be walked in step, and the driver says which side and that exporting again ends it. Every file written before 2026-09-21 is in id order, so for a while that path will be routine for merge bases.

The version is `0.x` while the format is still moving; saying so is more useful than a `1.0.0` that would have to be broken.
