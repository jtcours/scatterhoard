# Set Lifecycle

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

How a memory set comes into existence, gets written to a file, gets read back, and goes away. [File Format](file-format.md) settles what the file *is*; [Multi-Set Sync](multi-set-sync.md) settles how two divergent copies reconcile. This document covers the operations themselves and the tool surface that exposes them.

**Status: drafted 2026-08-03; revised 2026-08-05** to re-derive `import` and `restore` on the reconciliation model in [Multi-Set Sync](multi-set-sync.md); **revised 2026-08-06** to make the safety copy unconditional and take the dirty flag off the destructive path; **revised 2026-09-25** to allow [several sets in one directory](#several-sets-in-one-directory) and to [refuse an identity mismatch](#no-trusted-name-for-a-checkout-and-an-identity-mismatch-is-refused), both unbuilt.

**Implemented as of 2026-08-25: all of it** — `status`, `create`, `export`, `import` (the last two in both path forms), `restore` and `drop`, with the save-first rule below. That rule is what pulled export's explicit file form forward: a safety copy must by construction write somewhere that is *not* the set's normal export path.

## The hole this filled

*Historical, and worth keeping: this is the argument that produced `create`, and it is the reason `create` is a distinct operation rather than a mode of import.*

There was **no supported way to bring a memory set into existence.**

The local store is created on first run, so it never needed one. Shared stores were assumed to arrive by import, and refusing an unknown set name was right, because inventing a store on a typo would silently split a set. But import only materialises a database from a file that already exists, which means it could only create a set that someone else had already created. **The first set, the first time, has nothing to import from.**

The gap was filled by hand: create `shared-<name>.sqlite3` in the data directory and restart the server. That worked only because `Store::open` runs `CREATE TABLE IF NOT EXISTS`, and it was not a procedure anyone should follow. The file-side equivalent was worse — hand-writing a header line with a `format_version` the writer is supposed to own is precisely the [single source of truth](../../STANDARDS.md) failure this project keeps finding, and it puts a version number a human guessed at into the one field that decides whether a future build refuses the file.

So: **`create` is a distinct operation, not a special case of import.**

**What this turned out to settle, which was not obvious at the time:** `create` is also how a set someone *else* exported is adopted. Naming it binds the name on this machine, the file's header can then only agree, and a wrong name is a refusal rather than a silent re-target — so clone-and-import needs no separate first-import mechanism. `Expectation::AnyShared` would save a human typing a name; it does not enable anything. That was mis-stated once as a missing capability and corrected by writing the test; see `adopting-a-set-is-undocumented` in [Issues](../../TRACKING.md#issues).

## Resolved: create makes the database, and export makes the file

Given "either the file or the database must come into existence," the database is the right one to create directly.

- The database is what the server can use. A file with no database is inert until something imports it, so creating a file first means creating a set that does not yet exist as far as any tool is concerned.
- Creating the database first makes the file a *derived* artifact, produced by the writer that owns the format version. Nothing hand-writes a header.
- It matches the working sequence: create the set, put memories in it, export when there is something worth committing.

An empty set is a legitimate state. `create` produces an empty database with its scope recorded in `meta`, exactly as a fresh local store already is.

### What create decides that nothing else can

For a shared set, `create` is **where the set name is first bound on this machine**. The naming decision has to happen somewhere, and it cannot come from the file (a header is untrusted content, per [File Format](file-format.md)) or from the path (a project's directory name and its set name need not coincide). It comes from whoever runs `create`, which is the only participant that knows what the set is *for*.

**A name differing only in case from one this machine holds is refused, on every platform** (2026-09-30). `notes` and `Notes` are distinct names but one store file on macOS and Windows, whose filesystems are case-insensitive by default. Linux would hold them as two sets until the data directory moved, then they would collide. The refusal names the set holding the name, in its own spelling; `evaluate` applies the same rule to a review's label. The comparison is `SetName::shares_a_file_with` in [scope.rs](../../src/memory/scope.rs), exact only while set names are ASCII. The memory-file naming rule compares ignoring case for the same reason. `set-names-differing-only-in-case-share-a-store-file` in [Issues](../../TRACKING.md#issues).

**It does not bind the name to a checkout, and this section used to say it did** — wrongly, as the code plainly shows: `create` takes a name and nothing else (`create_set` in [mcp.rs](../../src/mcp.rs)), so no directory is in scope at the moment it runs. The claim was doubly misleading, because it presented the open question in [Open questions](../../TRACKING.md#open-questions) — *"where the trusted set name for a checkout comes from"* — as already half-answered by an operation that cannot see a checkout.

What actually associates a set with a directory is the **first export or import naming that directory**, which records bases against it. That is a real trust-on-first-use record, arrived at by accident rather than by design; see *the target list is an unintended trust record* below. *(Since 2026-09-25 no trust-on-first-use binding is wanted: [no trusted name for a checkout](#no-trusted-name-for-a-checkout-and-an-identity-mismatch-is-refused).)*

### Built 2026-09-28: `create` makes an empty set, and its first write settles it

**Decided 2026-09-24 by the human partner, as `set-settled-at-first-write` in [Backlog](../../TRACKING.md#backlog).** The set operations are three primitives: `create` makes an empty set, `import` syncs a set with a file, and `drop` removes one after a safety copy. `restore` stays the one composite, for repair under stress ([below](#resolved-restore-is-one-operation-not-a-sequence)), and `absorb` and `evaluate` stay as the two operations that do not decompose. What `create` used to settle — the set's identity and who writes to it — it no longer does, because it cannot tell a new set from one about to be taken on by `import`, and anything it settled early was either displaced by the file or forked it. Each is settled at the moment it becomes load-bearing, where the caller holds the remedy:

- **Identity.** A set takes the identity of the first file it imports that declares one. Otherwise it gets one at its first write, which mints it for every set, shared or `local`, and at its first export, which writes an identity into the file and keeps it only once nothing has refused the export. A refused export that had already minted would turn the import its refusal advises into a mismatch. Importing an empty file settles nothing, because nothing was written. A held identity is never displaced, records or not: the displacement rule `adopt_set_id` once had for an empty set existed only to undo `create` minting too early, and went with it. **This reverses [the 2026-09-14 refusal](#decided-2026-09-14-a-shared-set-with-no-identity-takes-no-records).**
- **Author.** A shared set's first write is refused until someone says who they are in it: `hoard_sets` action `mint` for a new contributor, `identify` for one whose record the set already holds. The name is how you are known where that set is published, which varies by set, so a machine-wide name is wrong for anyone who publishes in more than one place — which is how a corpus came to be attributed to a machine account. **`local` is the exception**: it takes the operating system's account name (`USERNAME` on Windows, `USER` then `LOGNAME` elsewhere), because it is never published and refusing its first write would put a step in front of every new user. Where the system supplies no name, `local` is refused like any other set. `SCATTERHOARD_AUTHOR` is gone.
- **Charter.** A shared set's first memory is refused unless it is the charter, a memory carrying `hoard-scope-topics`. Only the first: a set taken on from a file that carries no charter, or one whose charter was forgotten, keeps taking writes, because there the remedy belongs to the set's author, and `status` reports the missing charter instead.

So a new set is `create`, then `mint`, then the charter; `create`'s response says so. A set taken on is `create`, then `import`, then `identify` or `mint`. Built in three commits on 2026-09-28. **Two first writes at once agree on one identity**, since the same day (`concurrent-first-writes-can-mint-two-identities` in [Issues](../../TRACKING.md#issues)): minting and adopting both go through a single insert-if-absent, and a mint returns the identity the set holds afterwards rather than the one it generated, so the later of two first writes takes the earlier's. Minting had read, found none, and written with an upsert, so both writes landed and the later won.

## Resolved: export and backup are one operation

The prior distinction was that `backup`/`restore` operate on the local set and `import`/`export` on shared ones. That is not what the design actually records, and the real distinction is more interesting.

[File Format](file-format.md) already settles that **local sets are written to files too** — "a memory set that cannot be backed up is one bad disk away from being gone" — and that the header carries a full `Scope` precisely so a backed-up local set is *recognisably local on sight*. So both scopes are written by the same writer, in the same format, today. There is no format difference to justify two operations.

The one genuine difference is that **backup does not run the prohibited-data scan**, since nothing is being shared. But that is not a choice anyone makes: it follows from the scope. `Scope::exports_to` already returns `None` for `Local` and `Some(name)` for a project, and the [Export Review Gate](export-review.md) applies to memories destined for a repository. Local is never destined for one.

Apply [Decision Placement](decision-placement.md): *does the answer change anything the caller decides?* No — the caller states the scope, and whether the gate runs is derivable from it. A separate `backup` tool would be a second name for one operation, distinguished by a fact the software already holds.

**So: one write operation, `export`, taking a scope. The review gate runs for shared scopes and not for local ones, because that is what the scope means.** Two tool names collapse into one with no capability lost.

A caller *may* still say "back up my local memories" in words; the tool it reaches for is `export` with `local`, and the response should say the gate was skipped and why, so the difference stays visible where it matters — in the result, not in the surface.

### Built 2026-09-15: export from the command line, so a backup can be scheduled

**`scatterhoard export SCOPE --directory DIR`, or `--file PATH`.** Export was reachable only from inside a session, which meant the operation this section calls a backup could not run on a schedule — the one context where a backup is worth having. The subcommand goes through `Server::export_to`, which is the same function the tool calls under another name, so a rule added to export cannot be missed by the command line: there is no second implementation to miss it.

**The destination is a flag rather than a second positional, and `Destination` is public for it.** The two forms are not interchangeable — a directory has the set's file found in its `scatterhoard_mem`, a file is written exactly as given — and which one is meant is the one thing only the caller knows. Inferring it from the path's shape would guess at that; publishing the type lets the caller state it. The alternative considered was for the binary to build a `tools/call` request and read `isError` back off the response, which adds no public API and makes the binary depend on the shape of the MCP envelope rather than on the operation.

**Every refusal exits non-zero and says what was wrong on stderr**, because a scheduled job is watched by its exit status and read by nobody: a destination that no longer exists, a set this machine does not hold, a missing or duplicated destination. The message goes to stderr so a cron mail carries it; the report, as JSON, goes to stdout.

**It builds the same server a session gets**, model included. Export uses the model to describe the set it is writing, so a subcommand that skipped loading it would write files whose description was silently absent — and it records reconciliation bases and discharges the description exactly as a session's export does, against a store a running server may hold open at the same time. That is safe for the reason `Store::prepare_connection` gives: WAL, so a reader is not blocked by a writer.

`export-subcommand` in [Backlog](../../TRACKING.md#backlog); contract in `tests/cli.rs`, which spawns the built binary.

### Built 2026-09-02: restoring from a database backup, which is not `restore`

**Two different operations share the word and must not be confused.** `hoard_sets` action `restore`
replaces a set's contents from an *exported memory file* — a lossier artifact, carrying no bases, no
`meta` and therefore no set identity or author record. Restoring from a `sqlite3 ".backup"` copy is a
file-level act with no tool behind it, and it is the only one that returns a store to exactly the
state it was in. The procedure for making one is [above](#resolved-export-and-backup-are-one-operation);
this is the other direction, which nothing recorded until a restore point was actually needed.

1. **Stop every server holding the store.** Not politeness: the sidecars are the whole difficulty.
2. **Check for `-wal` and `-shm` beside the live file.** After a clean shutdown there are none —
   SQLite checkpoints and removes them when the last connection closes, which
   [Store Registry](store-registry.md) records as verified. If they are present, a server is still
   running or one did not exit cleanly, and step 1 is not finished.
3. **Remove the live `.sqlite3` and any surviving sidecars**, then copy the backup into place.
4. **Verify against something other than the copy**: open it and compare counts to what the backup's
   own record says it held.

**Step 3 is the one that bites, and it fails in the worst direction.** SQLite applies a leftover
`-wal` to whatever main file sits beside it, so copying a backup over a live file without clearing
the sidecars replays an old journal onto the restored database. That does not raise an error — it
produces records from nowhere, which reads as data corruption of unknown origin rather than as a
botched restore.

**Verification has to reach past the copy itself.** A truncated or stale backup opens cleanly, passes
`PRAGMA integrity_check`, and reports counts that look plausible because nothing contradicts them —
which is precisely how `2026-09-01-pre-attribution-migration-TRUNCATED-DO-NOT-RESTORE` came to exist,
holding 51 of 81 memories and looking healthy. The check that catches it is comparison with a source
outside the artifact.

## Resolved: import and restore stay separate

Here the same test gives the opposite answer, and it is worth being explicit about why, because the surface pressure runs the other way.

[File Format](file-format.md) records that restore is not import, and that conflating them would quietly merge stale backup content into a live store and produce a set that never existed at any moment in time. That conclusion stands. The reasoning originally given for it has been replaced in both documents, and the replacement is below. *(This paragraph used to quote that document's text verbatim, which made it a second copy to keep in step — and it duly went stale the moment the original was revised.)*

**"Merge" is the word that caused the trouble**, because it names two different operations at two different layers.

`git` merges two **files**, with a common ancestor to work from. That is real three-way merge, and [Multi-Set Sync](multi-set-sync.md) settles that Scatterhoard leans on it rather than reinventing it at the file layer. **That much is unchanged.**

What changed is the assumption underneath: that by the time a file is imported, the store holds nothing the file does not, so the store side needs no reconciliation at all. It routinely does. The per-record base checksum in [Multi-Set Sync](multi-set-sync.md) supplies the ancestor for that boundary — and it is worth being precise that this is **not** the proposal that was considered and rejected. That one reconstructed an ancestor from live database state, which is re-deriving what `git` already does. This one *records* the ancestor at the moment agreement actually happens, which is cheap, exact, and available at a boundary `git` cannot see. The rejection stands; it just never covered this.

`import` **reconciles** a file against a store that may have diverged from it. [Multi-Set Sync](multi-set-sync.md) settles how: tombstones make forgetting explicit, and a per-record base checksum supplies the ancestor `git` cannot provide across the store boundary, so each record resolves to take-the-file, keep-local, converged, or a surfaced conflict. A memory the store holds that the file has never carried is kept.

`restore` **replaces**: the file becomes the store, and local divergence is discarded on purpose. It serves the case where the store is corrupt or being rebuilt — where reconciling *against* it would mean merging against content nobody trusts.

**The earlier version of this section had the mechanism wrong and the conclusion right**, which is worth recording because the wrong mechanism was the more attractive one. It described `import` as an upsert that left store-only memories alone, and located the whole distinction from `restore` in *what happens to store-only memories*. That cannot work while absence means deletion: the documented way to forget a memory was to delete its line, so an upsert would have left the normal export-pull-merge-import cycle **incapable of ever forgetting anything** — delete the line, re-import, and the memory comes back. The implementation had it right, and for the stated reason ([store.rs](../../src/store.rs): *"a memory absent from the file has been forgotten and must not survive in the store"*); the draft was overturning working code without noticing what the code was protecting.

Tombstones rescue the upsert instinct by removing its precondition. Once deletion is a line rather than an absence, keeping unknown store memories is safe. The draft was reaching for the right behaviour and missing the mechanism that would license it.

What actually separates the two operations is **whether local divergence is preserved or discarded** — and that is a difference no scope determines. A shared set can need either: reconcile after a pull, replace after a corrupted database. A local set can need either: restore a backup, or reconcile a set exported from another machine. Scope carries no information about which is wanted, so the earlier local-vs-project framing would have made the choice *unavailable* in exactly the cases that need it.

The caller does have the information: it knows whether it is recovering or reconciling. So the decision is properly placed at the caller — the only question is whether it arrives as two tool names or one tool with a mode.

The separation now rests on firmer ground than it did. `restore` discards memories, which makes it destructive, and this document's own rule is that **an operation that can destroy memories gets its own name**. The distinction outlives the argument that was originally given for it.

### Why replace is a name, not a flag

**Replace is destructive and merge is not**, and a flag makes the destructive reading one token away from the safe one. A mode argument is chosen once, cheaply, in the middle of a longer argument list, by a caller whose attention is on the path. A tool name has to be selected deliberately and is legible in a transcript afterward.

This generalises, and it is the principle this document proposes for the whole lifecycle surface: **an operation that can destroy memories gets its own name; it is never a mode on a constructive operation.** That covers `restore` and `drop` and nothing else here.

Naming is a speed bump, not a guarantee, so it does not stand alone — see the state rules below.

## Resolved: a destructive operation saves first, rather than refusing

`drop` removes the database and leaves any exported file alone, which is what makes it recoverable: the set can be restored from its file. That guarantee only holds if the file is current. `restore` destroys memories the same way, for the same reason.

An earlier draft made this a **refusal** — `drop` declines a set modified since its last export. That is wrong for `restore` and unnecessarily rigid for `drop`, so the rule is instead:

**A destructive operation takes a path, and writes the set there before destroying it.** Not a refusal — a save. `drop` and `restore` share one rule, and the guarantee sharpens from *"cannot discard unexported memories"* to *"cannot discard memories without first writing them somewhere the caller named."*

The save is **unconditional**; it is not gated on the set having unsaved changes. That was not the original formulation, and the argument for removing the gate is below.

Three reasons this is better than refusing:

- **Refusal is worst exactly when the operation matters most.** `restore` is reached for when something is broken and a human is under pressure. An operation that says *no, go and do something else first* spends the moment when attention is scarcest.
- **The safety copy is more useful than a clean flag.** The exported format is one JSON object per line ordered by id ([File Format](file-format.md)), chosen for reviewable diffs. So a pre-restore copy can be diffed against the restored set and individual memories cherry-picked back. Refusing produces no such artifact.
- **It composes.** One rule covers both operations instead of `drop` having a gate and `restore` having none.

### The path must not be the normal export path

> **Replaced 2026-08-31 by a simpler and stronger rule: the path must not exist.** The question below
> — *is this a path this set has been exported to* — needed the per-target base's target list to
> answer, and could only ever see paths this store had written itself; a colleague's backup, an
> unrelated file, or a copy made by hand were all invisible to it. The harm is destroying a file,
> not destroying a file of a particular provenance, so *does something already live here* is what
> the rule below was approximating, needs no stored state, and refuses more. It also refuses
> re-running a rescue onto an earlier one's path, which is correct: that copy is the thing you would
> work back from. One helper, called by `drop`, `restore` and `absorb`, pinned per call site because
> the likeliest failure of a shared guard is one caller losing it.


This is a correctness requirement, not a convenience. `restore` reads from a file, and if the safety copy were written to the set's usual export path it could **overwrite the very file being restored from** — destroying the good copy in the act of protecting the bad one. So the destination is stated by the caller and is a separate location.

### When the safety copy cannot be written

Two failures, and they are not the same:

- **The store cannot be read** — the corruption case. The memories are already beyond saving; refusing to proceed protects nothing and blocks the repair. **Proceed, and report unmissably that no safety copy was made and what was lost.**
- **The destination cannot be written** — full disk, bad path, no permission. The store is fine and the memories are recoverable, so this is a real failure of the save. **Refuse**, and let the caller fix the destination.

The distinction is worth stating because the natural implementation — *if the export failed, stop* — gets the first case exactly backwards.

### Resolved: the save is unconditional, and no flag gates it

*Decided 2026-08-06. The earlier text said the flag decides whether a safety copy is required — a clean set already exists in a file, so there is nothing to save. That premise was written when there was one base per record. It did not survive the base becoming per-target.*

The fact this guard needs is: **is there any memory here whose current content exists in no file?** The base cannot answer it. [Multi-Set Sync](multi-set-sync.md) is explicit that the base "records an agreement between two particular replicas" — it is merge ancestry, recorded at the moment of agreement, and it is a claim about a past moment rather than about what is on disk now.

Two collapses were considered, and both over-read it in opposite directions:

- **Dirty if a record differs from its base for *any* target.** This reads "not yet agreed with every replica" as "not saved." A memory exported to the development checkout an hour ago is in a file right now; the review checkout being behind does not make it unsaved. The consequence is that a set with two checkouts is dirty whenever either is behind, so the gate would pass in nearly every real configuration — a guard that does not discriminate.
- **Dirty only if a record differs from its base for *every* target.** Closer to the real question, but it makes a claim it cannot support. Export targets are `git` working trees. A branch switch, a checkout, a `git clean` changes `memories.json` on disk with the store's base untouched, so "the base equals the current checksum, therefore that content is in that file" goes stale routinely — in precisely the two-checkout configuration the per-target base exists to serve.

Neither is a tuning problem. **The guard wants a fact the base was never built to carry, and no collapse rule makes it carry it honestly.** So the gate is removed rather than tuned, and the copy is always written. The question *dirty relative to what* is not answered for this purpose; it becomes unrepresentable.

What that buys, beyond correctness: the destructive rules no longer read any state out of the database they are about to destroy. That matters because of a general shape worth remembering past this document — *a guard stored inside the artifact it protects is unavailable exactly when it is needed.* The previous formulation met it with a fallback (an unreadable flag counts as dirty); removing the read is strictly better than getting the fallback right.

The flag itself does not disappear. It stops gating destruction and stays where it answers a question someone is actually asking — see *the state this needs* below.

> **If [Store Registry](store-registry.md) is adopted, rewrite this section rather than leaving it.** Its first bullet above rests on bases being keyed by `(record, target)`, and that proposal gives a store a single destination — so "is this set dirty" regains a single well-defined value and this argument's first leg stops being true. **The conclusion survives**; it stands on the second bullet plus two cases that proposal introduces, and they are set out there under *what this changes in decisions already resolved*. Flagged because the text would otherwise stay internally consistent while resting on a premise that had quietly changed, which is this project's most-filed defect class.

### Built 2026-08-25: what the rule turned into

Three things had to be decided that the sections above do not settle, and they are recorded here rather than in the backlog because each is a rule rather than a task.

**The set is named, not scoped.** `drop` takes a set name exactly as `create` does, so `local` is unrepresentable rather than refused — the same construction, and for a sharper reason at this end. `local` is the destination [route-where](conventions.md)'s fallback clause points at, guaranteed to exist, and the one set that never reaches a repository: hence the only set with no copy anywhere by construction. A `name` of `local` reaches the *shared* set of that name, which is a different set and droppable like any other.

**The safety copy's argument is `save_to`, and it is not `file`.** These are two decisions, not one spelled twice: `file` says where a set *lives* — written there by `export`, read from there by `import` — while `save_to` says where a copy goes before the set stops living anywhere. `restore` settles it beyond argument, since it reads a `file` **and** writes a safety copy in the same call. Worth recording because the published schema found this before the reasoning did: sharing the name handed `drop` the description written for a destination the caller means to keep.

**The copy cannot land on the set's own export path**, which *the path must not be the normal export path* above states as a correctness requirement without saying what enforces it. Two things do, and they cover different cases. A checkout is covered for free: no explicit file may name anything inside `scatterhoard_mem`. A set exported to an explicit file is reachable only through the recorded targets, so `known_targets` answers it — and this is the first thing to read that table for a purpose other than reconciliation, which *the target list is an unintended trust record* below anticipated in general terms.

### Forcing a drop is deliberately not a tool

Requiring the path unconditionally means `drop` cannot run when nothing is writable — a full disk, most plausibly one filled by the store itself. There is a way through, and it is **outside the tool on purpose**: delete `shared-<name>.sqlite3` by hand and restart the server.

That is not the hand-editing this document rejected. Hand-*creating* a store was refused because it makes a human invent a `format_version` the writer owns; deleting invents nothing, which is the same distinction that makes a fork's rename an acceptable hand-edit.

**Putting the escape outside the surface is the point, not a gap in it.** An unconditional-destruction option inside `hoard_sets` is one argument away from any caller, including a model working quickly; a manual file deletion and a server restart cannot happen without a human doing it deliberately. This is the guarantee the *replace is a name, not a flag* argument was reaching for and could not enforce by naming alone.

## Resolved: restore is one operation, not a sequence

`restore` is equivalent to truncate-then-import, or to `drop` + `create` + `import`. It is still exposed as **a single operation**, and the decomposition is not offered as the way to do it.

The reason is the intermediate states. A hand-composed sequence that fails partway leaves **no set at all** — dropped, not yet re-created, or created and not yet populated — which is strictly worse than either endpoint and arrives without warning. Composing three destructive steps by hand is a thing to be doing when nothing has gone wrong, and by construction nobody is doing it then.

The same reasoning as the safety copy: this is the operation reached for when a human is stressed and the LLM may have **no memories to rely on**, because the store holding them is the thing that broke. Everything about the operation should assume the caller has less context than usual, not more.

### Built 2026-08-25: what restore turned out to need

**A scope, not a name — the opposite of `drop`, and deliberately.** `drop` takes a name so `local` cannot be given to it, because it destroys the only copy a local set will ever have. Restore replaces a set *from a copy the caller is holding*, and for `local` that file is the only recovery route there is, since a local set never reaches a repository. The two operations share a rule and diverge on this because the rule is about *what happens to the memories*, not about how dangerous the word sounds.

**Both guards this document asks for are now real, and only the caller could have tripped either.** The copy may not be written over the file being restored from — *the path must not be the normal export path*, above, and this is the case it was written for — and it may not be a path the set is exported to. Both paths come from the caller on every call, by the same reasoning that keeps the export directory out of stored state, so nothing but an explicit comparison could catch them.

**Clearing `bases` is part of replacing.** A base says *this record's content agreed with that target at this checksum*; replacing the contents makes every one of them unprovable, and a base for a record the restored set no longer holds resolves as an anomaly — the state that is supposed to mean something happened outside the tools. Keeping them would have restore manufacture that report. So every base goes, and the agreement with the file just restored from is recorded in its place: [Store Registry](store-registry.md), *never keep a reconciliation base you cannot prove*, applied to an operation that did not exist when that rule was written.

**One thing this section got right for a reason that has since changed.** *Restore is one operation, not a sequence* argues from intermediate states, and that still holds. What it did not anticipate is that the same argument settles the file-reading shape: the whole file is materialised before the store is touched, so a malformed file costs nothing — and a separate validating pass, which `import` needs because it writes as it reads, turned out to prove nothing here. The guarantee is the same one [File Format](file-format.md) asks for; it is met by the transaction rather than by a second read.

### The state this needs, and who else needs it

Dirty-tracking was specified as a **boolean flag in the `meta` table** — any write sets it, export clears it, `drop` refuses while it is set. The last clause is twice superseded: `drop` no longer refuses, and it no longer consults it at all. What survives is the argument for the *shape* of the state, which is still worth keeping because both rejected alternatives remain tempting.

**Not a timestamp**, for the reason recorded in the `coding-lessons` memory set under *a technique carries its old context's assumptions with it*: an mtime acquires meaning from whatever else reads it, and this has to survive clock skew, a database copied between machines, and a file restored from backup. A flag lives inside the database and travels with it.

**Not a counter either**, which an earlier draft of this document argued for on exactly the reasoning above — wrongly. Everything in that paragraph is an argument against a *timestamp*, and a flag satisfies all of it. A counter answers a strictly larger question, *how far apart are two things*, which the drop rule never asks. Recorded because the mistake is a tidy example of the [decision-placement](decision-placement.md) test failing in the direction people rarely check: the extra state was not a wrong answer, it was an answer to a question nobody had asked.

**A counter was the candidate for gating `restore`** by comparing a file's version against the store's. That gate is not being built — the save-first rule replaces it — so the counter has nothing left to justify it. It is also the wrong instrument for the case that matters: a version comparison reads the store to decide whether the store may be replaced, which is unavailable precisely when the store is corrupt.

*That last sentence generalised further than it was written to. Applied to the dirty flag itself it removes the flag from the destructive path entirely, which is what happened above — the counter argument was right about a class and was only ever applied to one member of it.*

This is the same state a **status** operation needs, which is what makes them one feature rather than two: *which sets are loaded, and which have unexported changes* are the two halves of one question. It also closes `no-tool-lists-loaded-scopes` in [Issues](../../TRACKING.md#issues), which is currently a `Blocks` defect with no other fix — a caller told to "check the set name" has nothing to check against.

### The reconciliation base, and what it does to the flag

[Multi-Set Sync](multi-set-sync.md) adds a second piece of state: **each record keeps the checksum it had when the store and the file last agreed**, and both `import` and `export` update it. It is per-record rather than per-set, it lives in the store rather than in the file, and it is the ancestor that turns `import` into a three-way decision rather than a guess.

Its relationship to the dirty flag belongs here, where the flag is specified: **the flag is derived from the base, not independent of it.** A record is dirty, *relative to a given target*, when its current checksum differs from its base for that target. The flag is therefore a cache that spares `status` from rehashing a set, rebuildable at any time — not a second fact that can drift from the first. Stated in that order deliberately, because two independent facts that can disagree is the defect class this project keeps filing.

**What the flag no longer does is collapse to a single boolean for the set.** Bases are keyed by `(record, target)`, so a set can be settled against the development checkout and diverged against the review one, and "is this set dirty?" has no answer until someone says *dirty against what*. That question used to have to be answered before `drop` could be written; it no longer does, because `drop` does not ask it. Its only remaining consumer is `status` — and there it is not an ambiguity to resolve away but the substance of the answer, since *which* target is behind is exactly what a human with two checkouts wants to know.

This does not disturb the reasoning above about a guard living inside the artifact it protects. The base has the same weakness, and the destructive path now avoids it by not reading the base at all. Where `status` reads it and cannot, an unreadable base is reported as unreadable rather than as agreement.

### Open: what `status` reports, now that it is the only consumer

*This section previously held* **dirty relative to what**, *recorded as blocking `drop`. It no longer blocks anything — the save-first rule does not consult the flag — and what survives of it is a reporting question.*

`status` answers *which sets are loaded, and which have unexported changes*, and the second half is now the only place the per-target structure has to be turned into something a reader can act on. Undecided: whether it returns per-target divergence counts, plus a count of records with no base anywhere, or something coarser.

What *is* settled is that the per-target structure is real information rather than an ambiguity to be resolved away. A set reported as one boolean tells a human with two checkouts nothing they can act on, and discards exactly what the per-target base was built to preserve.

### The target list is an unintended trust record

*Observed 2026-08-06, working out whether `status` could enumerate targets. Nothing here is decided; it is recorded because the two questions turn out to be one table read in two directions, and they have been carried as separate open questions.*

*The binding half closed on 2026-09-25 without a binding — [no trusted name for a checkout](#no-trusted-name-for-a-checkout-and-an-identity-mismatch-is-refused) — so what follows about the set-name binding's key is history. The target half, and the unrecognised-target remark, stand.*

`SELECT DISTINCT target FROM bases` (`Store::known_targets` in [store.rs](../../src/store.rs)) is a list of **every directory this set has ever reached agreement with**. That is materially what [Open questions](../../TRACKING.md#open-questions) describes wanting under *where the trusted set name for a checkout comes from* — "the first import binds this checkout to a set name and records it in local state." The record exists. It was built to supply merge ancestry and acquired the second meaning for free.

Three things follow, and none of them is a decision yet.

**It is read in only one direction.** The table answers *given this set, which directories?* Trust needs the reverse — *given this directory, which set may claim it?* — and answering that means scanning every loaded store's targets. Nothing does, so nothing enforced one set per repository — a rule [since reversed](#several-sets-in-one-directory): two sets can both record the same directory as a target and no operation notices.

**It was checked in only one place, and is now checked in two.** Export remarked when the caller named an unrecognised directory; import did not, so it reconciled with no ancestor and said nothing — filed as `target-identity-is-an-unnormalised-path-string` in [Issues](../../TRACKING.md#issues), and since fixed: `export_set` and `import_set` in [mcp.rs](../../src/mcp.rs) both remark on a target they have never reconciled with. Today the scope is simply asserted by the caller on every call, which is the decision trust-on-first-use exists to stop being taken afresh each time.

**The two bindings want different keys, which is why the remote-versus-path question has been hard.** [Open questions](../../TRACKING.md#open-questions) asks "whether the git remote is a better key than the working-tree path," as one question. It is two:

- A **target** must distinguish two checkouts of one repository — that is the entire reason the base is per-target ([Multi-Set Sync](multi-set-sync.md)). Keying it on the remote would merge them back together and reintroduce the defect the per-target base was built to fix. The key has to be per-working-tree.
- A **set-name binding** wants the opposite. Keying it on the remote makes a second checkout inherit the first's binding, so a file placed in a fresh checkout claiming a different set name is refused rather than silently accepted as a new binding. Keying it per-working-tree makes every new checkout a fresh trust decision.

So the answer is plausibly *remote for the binding, path for the target*, on one store — not one key serving both.

**And no key removes the need for the warning.** Canonicalising a path collapses spellings; it does not survive the checkout being renamed or moved, which orphans every base just as thoroughly and reads as a target never seen before. Since no key eliminates the unrecognised-target case, **the load-bearing fix is that an unrecognised target is always said out loud**, and normalisation is hygiene that reduces how often it fires.

### What hangs off this list, as of 2026-08-26

*This section observed that one table is read in two directions and that the two questions had been carried separately. Three more have since attached to it, and they are recorded here rather than in their own documents because the thing they share is this list — not any of the operations that use it.*

- **`target-identity-is-an-unnormalised-path-string`** — settled and built. One spelling per directory, and the unrecognised-target remark on both sides of the round trip. Its value was mostly the remark: the live targets on the machine that had the defect turned out to be canonical already.
- **`a-target-path-can-come-to-mean-a-different-volume`** in [Issues](../../TRACKING.md#issues) — open, and `Corrupts`. A path resolves to one key, and two removable volumes at one mount point are two replicas the key cannot separate; the stale base is then offered as the ancestor for a file it never agreed with. The silent case is `KeepLocal`, which reports as routine local work.
- **[When a target stops being interesting](../../TRACKING.md#open-questions)** — open. Pruning, and it is downstream of the one above: *this agreement can no longer be proved* is the fact that makes a stale target safe to forget.
- **[How to inspect a memory set without adopting it](../../TRACKING.md#open-questions)** — open. The other end of the same binding: import is the only way to read a set, so reading one binds it.
- **[Whether a set carries a UUID](../../TRACKING.md#open-questions)** — open, and proposed as a candidate answer to several of the above at once. It would make the name stop being identity. It does **not** reach the volume defect, because a UUID identifies the set and a base must distinguish two checkouts of one repository.

**What changed on 2026-08-26, after this list was written.** Two of these looked as though they converged on one undecided thing — *what identifies a replica* — and they converge on it much less than that. [Multi-Set Sync](multi-set-sync.md) now carries a proposal to *detect* a reconciliation running backwards, which covers the realistic half of the volume defect with no replica identity at all. **Noticing the anomaly where it would do damage is worth more than an identity scheme that has to be minted, migrated and kept honest** — and it leaves replica identity as something to want rather than something to wait for.

**The pattern worth naming.** This list was built to supply merge ancestry and acquired a second meaning for free — *every directory this set has ever reached agreement with*. Each of the four above is a question about that second meaning, and none of them is a question about ancestry. **A record that acquires a second meaning acquires a second set of requirements nobody wrote down**, which is why they arrived one at a time and looked unrelated.

*Both were built 2026-08-26 and the ordering above held up: `Store::normalise_target` resolves a target fully, and **import** now carries the remark that only export had. The half that mattered was the remark — the store's live targets turned out to be canonical already, so normalising changed nothing on this machine, while the silent no-ancestor import was reachable at any time. A path that cannot be resolved keeps its spelling: a base is only ever consulted by exact key, so an unresolvable key can be matched by nothing but the path it names, and a checkout on an unmounted volume keeps its agreement rather than having it dropped for being unprovable.*

*The second half of that reason was stated too strongly and is corrected here rather than left reading as settled. "It comes back to the same path" assumes a path re-identifies the same volume, and removable media is exactly where that fails — `a-target-path-can-come-to-mean-a-different-volume`. **The decision stands and its first reason carries it**: an unresolvable key can supply an ancestor for nothing but the path it names, so keeping it is inert. What does not follow is that keeping it is *safe because the volume will be the same one*.*

## Resolved: identity travels; location does not

**`export` takes the path. `create` takes the name.**

The immediate reasons are practical: an absolute path is wrong on the next workstation, since nothing guarantees two developers check a repository out to the same place, and a repository-relative path buys uniformity at the cost of ever exporting anywhere else — including the safety copy, which by construction must go somewhere unusual.

The reason underneath is what makes it a rule rather than a preference. **A set's name is shared identity: it travels between machines in the file header, and every copy of the set agrees on it.** A path is per-checkout state that no other machine can see or use. Binding them at `create` would write machine-local state into the set's identity, and the copies would immediately disagree — the same class of mistake as recording scope per memory when the file already declares it ([File Format](file-format.md)).

So the split follows the lifetime of the information: `create` fixes what every copy must agree on, `export` states what only this machine knows.

### The set name is not the file name

Two different names are easy to conflate, and only one of them is identity.

- The **database** is `shared-<name>.sqlite3` ([store.rs](../../src/store.rs)). The set name appears here, but this is a local implementation detail — nobody else sees this file, and it never leaves the machine.
- The **exported file** was `scatterhoard_mem/memories.json`, a fixed well-known name that did *not* encode the set name, and was not to start to. *(Reversed 2026-09-25 and built 2026-09-26: the path settles a file's kind and the header which set it is, so a file is named after its set and found by its identity — [several sets in one directory](#several-sets-in-one-directory). The paragraph below is the argument as it stood.)* [Trigger and Loading](trigger-and-loading.md) rests on **the path determining how contents are trusted** — *"a file found here is a project set no matter what its header claims"* — and that check is only possible because the path is known in advance. A filename derived from the set name would mean the path could no longer be stated without first knowing what the file says about itself, which is the privilege-escalation channel that rule exists to close.

**So the set name lives in the header, and nowhere else that anyone shares.** Renaming a set is a one-line change to the header — and because both branches of a merge carry an identical header, a rename produces a real conflict rather than a silent divergence, which is the correct outcome for a change both sides must agree on. That is the same reasoning [File Format](file-format.md) already applies to a version bump; renaming inherits it for free.

### What a fork does

A fork copies the repository, header included, so the fork's memories continue to claim the original set name. Whether that is right depends on something no software can see: whether the fork is a contributor that will merge back, or a divergence that will not.

- **Contributor:** keeping the name is correct. The memories are the same set, and merging them back is exactly the export-pull-merge-import cycle in [Multi-Set Sync](multi-set-sync.md).
- **Divergence:** keeping the name is wrong, and the fork should rename. Two histories claiming one identity will eventually be imported onto one machine, and the merge will produce a set that never existed.

This is the same trust-on-first-use question already open under *where the trusted set name for a checkout comes from*, now with a concrete instance: **the collision is what needs a human, not the naming.** A set name never seen before can be bound silently, because nothing existing can be contaminated. A name already bound to a *different* checkout is the case to refuse and surface — and a fork is the ordinary way that happens, rather than an exotic one. *(Resolved 2026-09-25 by identity rather than by a name binding: [an identity mismatch is refused](#no-trusted-name-for-a-checkout-and-an-identity-mismatch-is-refused), and a hard fork gets its own identity by the procedure below.)*

### Renaming a fork is a hand-edit, and that is acceptable here

A hard fork — LibreOffice from OpenOffice — renames by **editing the `scope` field in the file's header line** and committing that. No tool is proposed for it.

This is a deliberate exception to the argument that made `create` a real operation, where hand-writing a header was rejected because it puts a human-guessed `format_version` into the field that decides whether a future build refuses the file. The cases differ in what is being written: a rename **edits one field of an existing, valid header**, leaving the version the writer produced untouched. Nothing is being invented.

**What makes it safe rather than merely small** is that the mistake is caught. Reading a memory file requires the caller to state what it believes the file to be, and the reader can only agree or refuse ([File Format](file-format.md)). So a header edited to `libreoffice` and then imported into a store still bound to `openoffice` is a mismatch and a refusal — not a silent merge of two projects' memories. The rule written for privilege escalation turns out to make hand-editing recoverable, which is a good sign it was drawn in the right place.

Two consequences worth stating, because neither is obvious from the edit:

- **The forker's own machine needs `drop` then `import`.** The local database records the old name in `meta`, and editing the file does not reach it. That is the drop-plus-import path working as intended, and the save-first rule means the old set is written out before it goes.
- **Renaming also deletes `set_id`, and the file committed is a re-export, not the edited copy.** Decided 2026-09-22 under `how-a-renamed-fork-gets-its-own-identity` in [Open questions](../../TRACKING.md#open-questions). This section predates `set_id`, and an edit of `scope` alone leaves the renamed file claiming its origin's identity: an import into the forker's freshly created set *adopts* it, so every copy of the fork answers to the origin, and a machine holding both refuses the fork. Measured, not read from code. The procedure, on the renaming machine only:

  1. Copy the file **outside the repository**, and in the copy's header edit `scope` to the new name and delete `set_id`.
  2. `drop` the old set if this machine holds it, then `create` the new name and `import` the copy. A file declaring no identity adopts nothing, so the import — the set's first write — mints the fork's own identity, and says so. *(Until 2026-09-28 `create` minted it; since then a set is [settled at its first write](#built-2026-09-28-create-makes-an-empty-set-and-its-first-write-settles-it).)*
  3. `export` into the checkout and commit **that**. Its header carries the fork's own identity, which every collaborator's `create` then `import` adopts.

  **The order is the whole safeguard.** Committing the hand-edited copy instead gives every machine that imports it an identity of its own, minted by its own first import — and once that set holds records it keeps it, so a later import of the proper re-export does not repair it. That failure was measured the same day, and since then it is at least not silent: an import of a file holding records but declaring no identity says so, and an import or export where the file's identity and the set's differ names both (`an-import-silently-keeps-a-different-set-identity` in [Issues](../../TRACKING.md#issues)). **A `fork` operation that does all three steps was considered and deferred**: it would make the wrong file impossible to commit rather than loud, and is worth building when a real fork shows the procedure is not enough.
- **Renaming dangles every cross-set reference pointing at the old name.** A cross-set reference is an id plus a set name ([Cross-Set References](cross-set-references.md)), so links from other sets into this one stop resolving. For a hard fork that is arguably *correct* — it really is a different set now — and the design already treats dangling cross-set references as the LLM's call rather than a hard failure. Worth knowing before renaming a set that is not a fork.

### A set that belongs to no repository

The scope kinds are not really *local* and *project*. They are **private-and-unbound** and **shareable-and-named**, and only the first is what `Local` means. `Project` means a set with a name that can be exported — it does not require a repository, and nothing in the lifecycle above assumes one.

So a cross-project set such as `coding-lessons` needs no new mechanism. It is `Shared("coding-lessons")`: created by `create`, exported wherever the caller says, loaded alongside everything else. Recall already unions every loaded store and every result names the set it came from, which is the Vision's *"local memories — personal and general — unrelated to a specific project"* landing on the shareable side of the line rather than the private one.

**How another project gets those memories: the developer imports the set, not the project.** A set loaded on a machine participates in recall for every session on that machine, so working on project A with `coding-lessons` imported means the lessons are recalled alongside A's own memories, distinguished by scope. **Nothing is copied into A's repository**, and that is the point — the lessons live in one place and are read by many projects, rather than being vendored into each and drifting.

Copying memories *between* sets is a different operation, deliberately not designed here. It would need an answer for what happens when the source set later changes, which is the problem vendoring always has.

Publishing the set later changes nothing structural: it gets a repository, its memories land in that repository's `scatterhoard_mem`, and consumers clone and import. (This read *one set per repository still holds* until 2026-09-25, when [several sets in one directory](#several-sets-in-one-directory) replaced that rule; publishing still changes nothing structural.)

### Resolved 2026-08-23: `local` is one set, not a class of them

**There are exactly two scope kinds, and only one of them is a family.** `Shared(SetName)` is open-ended — any number of sets, all symmetric, each named by a `SetName` that means the same thing on every machine holding it. `Local` is a single set, guaranteed to exist, never published to a repository. It is not the first member of a class of machine-bound sets, and no second one is planned.

This closes the question [Open questions](../../TRACKING.md#open-questions) had been circling as *what to call the class* — by answering that there is no class to name. The candidate spellings (`local:personal`, `local:private`, `private:local`) were all attempts to name a family whose only member was the one we already had, chosen against three scenarios none of which turned out to want machine-binding at all.

**The argument that settles it.** Three questions had been recurring and each was really the same question: whether `local` is unique or a class; what to call the class if it is one; and why a choice binding at set-creation time has to be made when the person knows least about what the set will hold. They were joined by a fourth — what happens when a memory in one set supersedes a memory in another — and that fourth one turned out to be load-bearing for all of them.

*Local was a class-shaped thing only because it had a power no other set had.* Under the withdrawn trust gradient, `Local` was not just unpublished but **authoritative**: its edges could change the standing of a project memory, and no project memory could touch it. A scope with special powers invites a scope *system* — you start asking which other scopes should have which powers, and the answer needs a vocabulary. [Cross-Set References](cross-set-references.md#resolved-2026-08-23-no-edge-changes-the-standing-of-a-memory-in-another-set) withdrew that power on independent grounds. With it gone, `Local` has exactly one property left: `exports_to` answers `None`.

**One property does not need a class word.** It needs a boolean, and there already is one.

**What this costs, stated plainly.** The three scenarios that reopened the question — a personal set across two of your own laptops, a household set shared with a spouse, a volunteer board's institutional memory — are all served as ordinary `Shared` sets. Each gets a name, each can be exported and imported, none needs a repository. That was already true before this resolution: *the scope kinds are not really local and project*, per the section above, and `Project` never required a repository. What the resolution adds is that nothing further is coming to serve them better, so they should be created as shared sets today rather than waited on.

**What they do not get is the review gate's absence.** A shared set with no repository still answers `Some(name)` to `exports_to`, so it is treated as publishable — because it *is* publishable, at any time, by acquiring a repository. That is the honest answer and it is mildly over-strict: the prohibited-data warning fires for a household set that will never leave a Drive folder. Over-strict in the safe direction, and it preserves *publishing later changes nothing structural*, which is the property doing the real work here. See the sub-question in [Open questions](../../TRACKING.md#open-questions) on whether the boundary is three-valued rather than two.

**The Golden Rule constraint is met by the one set that remains.** [The Golden Rule](golden-rule.md) names *it travels* as one of three mechanics that make memories about people hazardous, and the mitigation is that routing always has a destination which does not travel. `Local` is that destination, guaranteed by construction — `StoreLayout::discover` inserts it unconditionally and the store is created on demand — so `route-where`'s fallback clause can never point at nothing. **That guarantee is the thing to protect in any future change here**, and it is now protected by a simpler arrangement than before: there is no rename to survive and no class for a fresh install to get wrong.

**The binding-choice complaint dissolves rather than being answered.** The choice at set creation is now only *will this ever be published* — and because publishing changes nothing structural, getting it wrong is not a one-way door. A set created shared and never published has lost nothing. The one-way door that remains is the one that should: putting a memory into a set that publishes.

### Resolved 2026-08-23: the fixed export path is a property of the trigger, not of the format

`export` takes a directory and finds the set's file in its `scatterhoard_mem` (until 2026-09-26, appended `scatterhoard_mem/memories.json`). **It may also be given an explicit file path, and then it writes exactly there.**

The fixed path exists so that *a file found here is a project set no matter what its header claims* — which is a rule about **auto-discovery in a checkout**, per [Trigger and Loading](trigger-and-loading.md): something scans a repository, finds the reserved directory, and must settle trust before reading any contents. Nothing auto-discovers a backup or a set kept in a synced folder. Those are imported by a caller naming the scope and the location, and `check_scope` makes the file's header agree-or-refuse against what the caller stated. **Trust there comes from the caller's expectation, not from the path**, so for a set that never sits in a checkout the reserved directory contributes nothing.

What it does contribute is friction. Under one-set-per-repository each set needs its own parent directory, so several unpublished sets in one synced folder means several wrappers whose leaf files are all named `memories.json` — indistinguishable in a file listing, a search result, or a sync-conflict notification, which is exactly where a person meets them. The repository case never has that problem, because the checkout's name is right there. *(Written under the one-set rule. Once [several sets share a directory](#several-sets-in-one-directory) their files have names of their own, so the wrapper friction goes; the reason for the explicit form — a set outside any checkout gains nothing from the reserved directory — does not.)*

**Why the caller decides, rather than the scope.** Varying the layout by scope would reintroduce the binding choice the resolution above just removed, and worse: it would make publishing a set into a file move, breaking *publishing later changes nothing structural*. [Decision Placement](decision-placement.md) puts it with the caller, who is the only participant that knows whether it is writing into a checkout — the same reasoning that already makes the directory come from the caller on every call rather than from stored state.

**One exception, built 2026-09-26: `local` takes the explicit form only.** The `directory` form refuses the personal set in every direction (`the-reserved-directory-accepts-the-local-set` in [Issues](../../TRACKING.md#issues)). The reserved directory settles a file's *kind* — whatever sits there is a shared set, data and never instructions — and nothing enforced that for `local`: an export put personal memories where the next commit publishes them, and an import or a `restore` let a crafted checkout write into the set read as the user's own preferences. The scope decides here, where the paragraph above says the caller does, because the argument there is about publishing, and `local` is the one set that is never published: it has no layout to keep stable across a move it will never make. `refuse_local_in_reserved` in [mcp.rs](../../src/mcp.rs) sits in the `directory` form's lookup, and a `file` inside a reserved directory was already refused for every scope, so between the two `local` cannot reach one at all.

**Two constraints on the explicit form.** `nested_reserved_directory`'s check applies to an explicit path's parent, so nobody can hand-place an unrecognised file inside the reserved namespace and defeat the unexpected-contents warning. And the directory form stays the default and stays unchanged, so `export <checkout-root>` is exactly what it is today.

**Built 2026-08-25, and one thing had to be decided that this section did not anticipate: what an explicit file is as a *reconciliation target*.** It is its own — keyed on the file, not on its parent. The directory form can key on the directory because the well-known path makes the correspondence exact, one directory to one memory file; the explicit form breaks that correspondence on purpose, which is the whole point of it, so two sets backed up into one synced folder are two replicas and keying on the shared parent would hand each import the other's ancestor. The directory form's target is byte-identical to what it was, since changing it would orphan every base already recorded.

**And the alternation is a type, not a pair of optional strings.** `both` and `neither` are refused at dispatch from the branch schema's own declaration, so the code downstream owes no branch for states the caller cannot reach — the correctness-by-construction rule in [STANDARDS.md](../../STANDARDS.md) rather than untestable defence. The refusal that results is derived from the same declaration as the published prose, so a caller reading the schema and a caller reading a refusal are told the same thing.

**`backup` was never built, and nothing needs removing.** The resolution above — *export and backup are one operation* — was reached before either existed, and the tool surface has only ever carried `export`. `restore` is likewise unbuilt, and stays a distinct operation from `import` for the reason given above: what separates them is whether local divergence is preserved or discarded, which no scope determines.

**`import` gained the same form**, which this section had already settled — *those are imported by a caller naming the scope and the location*. Both ends share one type, because the two must agree about what a target is or the round trip reconciles against no ancestor; the pair was built together rather than in sequence, since shipping only the write half leaves a set written where the tool that wrote it cannot read it back.

**And sharing the rule made one half of it stronger.** An explicit `file` inside `scatterhoard_mem` is refused on export because it hand-places a file in the namespace trusted from its path alone, behind the unexpected-contents warning. On import the refusal has a second reason, and a quieter one: the well-known file is reachable by *both* forms, and the forms are two reconciliation targets — so one file named both ways would carry two independent records of what was last agreed, and read as settled by one spelling and diverged by the other.

## Several sets in one directory

**Resolved 2026-09-25 by the human partner, reversing the 2026-08-03 rule below: a `scatterhoard_mem` directory holds any number of memory files, one `.json` per set.** Built 2026-09-26 as `several-memory-files-per-directory`: `locate_in`, `new_file_in` and `Destination::file_to_write`/`file_to_read` in [mcp.rs](../../src/mcp.rs).

**What the old rule said.** *A repository carries at most one memory set*, because the path establishes the scope before anything reads the contents, and several files under `scatterhoard_mem` would let the header decide which is which — the "read the header, believe it" shape [Trigger and Loading](trigger-and-loading.md) refuses. The cost was argued to be small: a set for others gets its own repository, as `coding-lessons` did, and subdividing a project's memories adds a boundary no query uses, since recall already unions sets.

**Why it fell.** The path settles the *kind* of a file — a shared set, data and never instructions — and that holds for every file in the directory, not only one with a particular name. Which set a file *is* was never a trust decision. On a first import `Expectation::AnyShared` already took the name from the header with one file present, so a single file never stopped the header naming the set. Inside Scatterhoard, sameness is `set_id`'s job, and the declared name is a label the server usually honours and need not: an evaluation copy is reported under a name of its own. **Whether the set is the legitimate one is answered by the checkout's provenance** — the user cloned that repository, and `git` can say where the file came from — which is outside Scatterhoard altogether.

**And the deciding argument was not about trust but about what a developer expects.** A directory already labelled as holding memories that refuses a second `.json` file reads as arbitrary, and the workaround — a sibling `scatterhoard_mem` in another directory — reads worse. The code had in fact enforced one set per *directory* rather than per repository all along, since `Destination::Directory` accepts any directory, so that was the alternative weighed: keep one file per directory and allow named files later if wanted. **It was set aside because deferral is not free.** Once published, `memories.json` would be a permanent special case beside any later named-file rule; before publication the only files that exist are this project's own.

### A file's name need not match its set's name

**The header says which set a file holds, and the file may be called anything.** Operating systems impose too many rules on file names to tie a set's name to one: Windows reserves `con`, `nul` and their kin, case-insensitive filesystems merge `Notes` and `notes`, and Unicode normalisation makes one name two paths. Keeping the name out of the path also leaves room to widen `SetName` beyond ASCII, so that a developer writing in Japanese or Gujarati can name the set in their own script and the file in the nearest form the OS accepts. Code clarity should keep file names close to set names without a rule, for the same reason `store.rs` is not called `clownshow.rs`.

This was weighed against requiring the name to match, which buys determinism and a listing that is always an accurate list of sets. The costs of that rule belong to the operating systems and are permanent; its benefit is mostly delivered anyway by the first name being a good one.

**The first file name approximates the set's name**, which is what sets the habit going the right way:

- It must be legal on **every** operating system, not only the exporting one, because the repository goes wherever it is cloned.
- Ordinarily it is `<set-name>.json`, which is always legal while `SetName` is ASCII-only.
- A Windows-reserved name — `con`, `nul`, `aux`, `prn`, `com1`–`com9`, `lpt1`–`lpt9` — takes a suffix: `con-set.json`.
- A name another set's file already uses in that directory, **compared case-insensitively**, takes a numeric suffix: `notes-2.json`.
- If set names are ever widened beyond ASCII, the rule grows to normalise to NFC and replace whatever some OS forbids. Not to be built before then.

### Finding a set's file: by identity, read from the headers

**In `directory` form, export and import read the header of each `scatterhoard_mem/*.json` and match on `set_id`.** One match is the file; two are refused; none means export writes a new file under the approximated name and import says the set is not there. A set with no identity yet — which can only be one being imported for the first time — matches on its declared name instead, which is the header naming the set and is fine for the reason above.

**Bases stay keyed on the directory.** A store holds one set, so the directory plus the store's identity already picks out one file whatever it is called.

The alternative was to record the file name per target. It adds state, and it breaks the first time someone renames the file: export would recreate the old name beside the renamed one, leaving two files for one set. A file name that need not match invites exactly that tidying, so a hand rename has to cost nothing, and matching on identity is what makes it free.

**What else the build carries:** the unexpected-contents warning narrows to files that are not `.json`; `.gitattributes` and [SETUP.md](../../SETUP.md) move to `**/scatterhoard_mem/*.json`, which also reaches a set in a subdirectory; this repository's and `coding-lessons`' files are renamed; and every refusal that names `memories.json` is reworded.

### Built 2026-09-26: what the lookup had to decide that the design did not

- **The name is the fallback whenever identity finds nothing**, not only for a set with no identity. A file declaring the set's name under another identity is then *found*, so what follows can act on the mismatch — reported today, refused once `identity-mismatch-refusal` ships — rather than export writing a second file beside it. A file carrying the set's identity still wins over one merely sharing its name.
- **A `.json` whose header cannot be read is refused, naming it**, rather than skipped: it may be the set's own file, damaged, and skipping it would have export write a second file beside it. A name beginning with a dot is not read at all, since macOS writes `._name` beside files on volumes without extended attributes.
- **`absorb` and `evaluate` take `directory` with `set`, the name of the set to read, as well as `file`.** Agreed with the human partner. They read a set this machine does not hold under that name, so there is no identity to search for: in `directory` form the request names the set and the headers are searched for it, as a first `import` does. `set` is required there even when one file is present, so a request that works today cannot start failing when a second set is exported beside it; with `file` it is optional and checked against the header. **The distinction between the forms is the header search**, and both of these actions need both: a checkout's set is known by name, and a downloaded file by its path. Two alternatives were set aside. Taking the only file and refusing several left no way through, since the `file` its refusal pointed to is refused inside `scatterhoard_mem`. Accepting `file` alone would have had the model open every header itself to find a set named in its request, and would have needed a per-action exception to the rule that each file is reachable by one form.
- **A Windows device name is checked on the part before the first dot**, since Windows reserves `nul.notes` as well as `nul`: that set's first file is `nul-set.notes.json`.
- **The unexpected-contents warning is not built**, so narrowing it to non-`.json` files is carried forward to whenever it is.

## No trusted name for a checkout, and an identity mismatch is refused

**Resolved 2026-09-25 by the human partner, closing *where the trusted set name for a checkout comes from*.** Built 2026-09-26 as `identity-mismatch-refusal`: `identity_mismatch` and `Replaced::identity_refusal` in [mcp.rs](../../src/mcp.rs).

**No name needs trusting, so there is no binding.** The question proposed trust-on-first-use: the first import binds a checkout to a set name, and later imports are checked against it. That assumed Scatterhoard had to establish that a checkout's set is the legitimate one. It does not. **Legitimacy is the checkout's provenance**: the user cloned that repository, `git` can say where the file came from, and the caller restates the choice on every call by naming the `scope`, which `check_scope` makes the header agree with. Sameness is `set_id`'s job. A legitimate rename is the same `set_id` under a new name. The narrowings the question had gathered — remote versus path, [the target list as an accidental binding](#the-target-list-is-an-unintended-trust-record), registration in [Store Registry](store-registry.md) — were all answers to *where a binding lives*, and fall with it. A binding would matter only to auto-discovery, loading a checkout's sets without being told which, and that is not built; if it ever is, this can be reopened then.

**What the binding was really guarding is a collision, and identity answers it.** A file whose `set_id` differs from the set's is **refused**: on import, and on an export that would write over it, in both path forms. A mismatch arises in only three ways, and the refusal names the way out of each:

- **Two legitimate sets that share a name** — two people's `notes`, each with its own provenance and identity. Nothing is wrong except the choice of name, so the route is to **open one under an alias** (`open-a-set-under-an-alias` in [Backlog](../../TRACKING.md#backlog), additive and so not on the release gate). The alias is the set's `scope` on this machine; the declared name stays its own, and **the alias never reaches a memory or a file** — export writes the declared name, and a link naming the alias is translated at the door. An evaluation copy is the working precedent: loaded under `eval-<set>-<label>`, carrying its file's identity.
- **An intended merge** — `absorb`, which exists for exactly this.
- **A diverged copy of one set** — the fork trap, where a machine minted its own identity from a file that declared none. `drop` with its safety copy, `create`, and import, so the empty set adopts the file's identity; then put back by hand anything the safety copy holds that the file does not. `restore` cannot do this, because it keeps the store's identity.

**Why refuse rather than report.** Reporting was chosen on 2026-09-22 because what identity should gate was still open. Its failure is worst in the third case: the diverged set keeps its own identity, and its next export writes that identity over the shared file's `set_id` — so every other collaborator then mismatches, and only the machine at fault was ever told. Refusing puts the cost on the machine that made the mistake. What made refusing acceptable for the first case, which is not a mistake at all, was the alias.

**A file holding records but declaring no identity keeps its note**, since there is nothing to compare against.

### Built 2026-09-26: what the refusal had to decide that the design did not

- **Import asks after adoption, not before.** A set with no identity takes the file's, which is `create` then `import` working as designed, so there is no mismatch to find. *(This read "a set holding no records" until 2026-09-28, when `create` stopped minting and an identity once held stopped being displaceable.)* Still differing after adoption means the set kept its own because it has a history, and adoption wrote nothing in that case, so the refusal leaves the store as it was.
- **Export asks identity before anything else.** *(Until 2026-09-28 it defended only a set with a history, because an empty set's identity was what an import would replace. Now every held identity is defended, and a set with none meeting a file that declares one is told to import it — the step it skipped.)* Where both apply, identity is refused first: the dropped-records refusal advises importing from the destination, and with the identities apart that import is refused too.
- **The collision route says the alias is not built**, rather than naming a call that does not exist. Until `open-a-set-under-an-alias` ships, two sets sharing a name cannot both be loaded on one machine, and the refusal tells the reader to keep the file out of the set's imports and exports until then.
- **Two machines' `local` sets are two sets**, each with an identity, so crossing between them is refused — consistent with [`local` never leaving its machine](#resolved-2026-08-23-local-is-one-set-not-a-class-of-them), and with the open question of whether `local` is one set across a person's machines still to be settled before any sync is built. Tests that had used two machines' `local` sets as a fixture for unrelated behaviour now use one shared set held on two machines.

## Moving memories between sets: no tool action, and a procedure that works

**Written up 2026-08-12 after doing it to 13 memories**, on the same grounds as [re-attributing existing memories](store-registry.md#re-attributing-existing-memories-no-tool-action-and-a-procedure-that-works): the judgement is a human's, the capability already exists, and the procedure produces a reviewable diff as its artifact. Expected to be common — misrouting is easy, `route-where` did not exist until this week, and a set that acquires a cross-project sibling later will want a redistribution.

**Do not do it memory by memory with `memorize` and `forget`.** A re-stored memory gets a new id, so every link pointing at the old one dangles, and the supersession chains break silently. The procedure below preserves ids, attribution, timestamps and links, because it moves the records themselves.

### The procedure

1. **Back up both stores.** `sqlite3 <store> ".backup <destination>"` is safe against a live database, where copying the file is not — the servers hold it open, and the `-wal` sidecar carries committed data. This is the only true restore point: an export is lossier, since it carries no bases, tombstones or `meta`.
2. **Export the source set** to get its records in readable form.
3. **Check the links before choosing what moves.** This is the step with the trap in it — see below.
4. **Append the chosen records, verbatim, to the destination set's own exported file.** Not a new file: the destination's existing export, so the file stays *complete* relative to that store. Records carry no `scope` field — scope lives only in the header — so a record moves between sets with no per-record edit at all.
5. **Import from the destination's usual directory.** The records already there reconcile as `Unchanged`; the moved ones arrive as `NewFromFile`.
6. **Forget the originals from the source set**, which leaves tombstones so the removal propagates.
7. **Verify**: counts on both sides, supersession chains still resolving, and no dangling same-set links.
8. **Export and read the diff.** The review gate's input falls out of the procedure rather than needing to be produced.

### Why the file must stay complete

A *partial* file is the tempting shortcut and it is wrong in one specific case. Import reconciles the whole store against the whole file, so a record the store holds and the file omits is judged by whether a base exists for that target: no base means `KeepLocal`, but a base means **`Anomaly`**. Importing a file containing only the new records, into the directory the destination set is normally exported to — which is exactly where its bases are — reports every existing memory as an anomaly.

Appending to the destination's own export sidesteps this without needing the rule: the file contains everything the store has plus the arrivals, so nothing is omitted and nothing can be anomalous.

### The trap: a `supersedes` link pointing at something you were going to discard

Selecting records by topic will pull a superseder without its predecessor. Both times this arose in the first real transfer, the memory that looked like a discardable near-duplicate was in fact a **superseded predecessor with an explicit link**, and dropping it would have left a `supersedes` edge pointing at nothing — destroying precisely what [Corrections](corrections.md) says supersession is for, that a later reader can see the question was live and which way it was answered first.

**So a superseded record moves with its superseder.** The destination set already demonstrated the right shape: its own first two records are a supersession chain, both present.

Check before selecting, mechanically, rather than by reading the memories:

```sql
select source, label, target from links where label = 'supersedes';
```

and confirm every target is in the moving set. The same check catches ordinary links that would dangle; those are tolerable, since a dangling link is a state the design expects, while a broken supersession chain is a loss.

## Built 2026-08-31: a set has an identity, and the cutover that gives the existing ones theirs

`create` mints a `SetId` — UUIDv7 spelled Crockford base32, like every id here — and it travels in the file header. *(Since 2026-09-28 `create` mints nothing: a set is [settled at its first write](#built-2026-09-28-create-makes-an-empty-set-and-its-first-write-settles-it).)* `status` reports it. An import **adopts** one from a file when the store has none, and never displaces one it already holds; a file carrying none is checked by name exactly as before. Nothing refuses on a mismatch yet, deliberately: what identity *gates* is [still open](../../TRACKING.md#open-questions), and a refusal invented at this stage would answer that by accident.

### The cutover has an order, and one step in it is unforgiving

The three sets on this machine were created before any of this existed, so none has an identity, and `create` was then the only thing that minted. They get one the way the migration rules say: **from a file**.

1. **Deploy the build and restart the servers.** Nothing before this point can read or write a `set_id`.
2. **Write a minted id into each set's exported file header**, by hand, one per set. Any 26-character Crockford value that nothing else uses; generate them rather than inventing them by keyboard.
3. **Import each set from its own file.** The store has no identity, so it adopts the file's — the response says `adopted_set_id`, which is how you confirm it rather than assume it.
4. **Export, and read the diff.** The header should now carry what you wrote, and nothing else should have moved: the record lines are untouched by any of this.

**Step 2 must not straddle a restart, and this is the sharp edge.** A server built before this change does not know the field, so **exporting through one silently strips it** — no error, and a diff nobody is looking at. The window between hand-editing a file and running the import is the only time that can happen, so close it deliberately instead of remembering it.

**`local` was cut over the same way on 2026-08-31** — exported to a fresh dated backup, header edited, imported back — and the code changed so no future `local` has to be. **It claims an identity on its first write**, which is `author_or_claim`'s rule and its reason: opening or reading a store must never write to it. A shared set still never minted on a write — *until 2026-09-28, when every set began minting at its first write; the fork this asymmetry guarded against is refused at the next exchange since `identity-mismatch-refusal`* — and the asymmetry was the auto-mint hazard applied rather than relaxed: that hazard is *two machines holding one set each minting a rival*, and `local` never leaves the machine, so there is one store by construction and nothing to fork against. **That last clause is an assumption with a known expiry** — peer-to-peer sync between one person's two laptops would break it, and by then both machines will have minted already. See [whether `local` is one set across a person's machines](../../TRACKING.md#open-questions), which must be settled before any such sync is built.

Without that, `local` could never have acquired an identity at all — `create` is the only other route and `local` is guaranteed to exist rather than created.

### Decided 2026-09-14: a shared set with no identity takes no records

> **Reversed 2026-09-28.** A set now mints its identity at its first write, and this refusal is gone
> ([settled at its first write, above](#built-2026-09-28-create-makes-an-empty-set-and-its-first-write-settles-it)). What it protected still
> holds: a set holding records has an identity. The argument below against minting on a write lost
> its force when `create` stopped minting too: `create` forked just as blindly, and since
> `identity-mismatch-refusal` a fork is refused at the next exchange rather than merged. Kept as the
> record of why the invariant exists.

**The store refuses to give records to a shared set that has no identity**, at the write every authored path and import share, at restore, and at import before any record is reconciled. Forgetting is not refused: it adds nothing, and it is how sensitive data leaves a set. A tombstone arriving from a file is applied by the same unrefused path, which is why import checks the set before it reconciles anything: without that, a file whose records begin with tombstones would leave them in a set with no identity before its first memory was refused, and a file of tombstones alone would import with no error. Decided by the human partner.

**Why it became an invariant rather than a habit of the callers.** Whether a stored link is the set's own is decided by comparing identities ([Cross-Set References](cross-set-references.md), *which of the two a stored link is, is decided by identity*), which is sound only if every set holding records has one. Every tool path already ensured it — `create` mints before any write, `local` claims one on its first, import adopts before reconciling, and adoption displaces an identity only on a set with no records — but the store accepted a caller that did not.

**Refused, not minted.** Minting on the write was the other candidate, and it is the hazard the cutover above was designed around: two holders of one set each minting a rival identity. The refusal names the ways through instead, because no tool mints a set identity on its own: `import` from an exported file that carries the identity, which adopts it; or, where no such file exists, `drop` with a `save_to` path, `create` under the same name, and `import` from the saved copy. It deliberately does not point at `hoard_sets` action `mint`, which makes an author record.

**Tests say which kind of store they open.** `Store::open_in_memory` stays exactly like `open`, with no identity, and `Store::open_in_memory_and_mint_id` behaves like a set `create` just made. The minting is in the name at the human partner's review, because a familiar constructor that quietly wrote an identity would not read as doing so. Writing at open does not cross the rule that opening a store must never write to it: that rule exists because several processes open one database file, and nothing else can open an in-memory store.

### The links come after, and are a different kind of change

Filling `set_id` into the existing cross-set links is **not** part of the above. Those seven records ([Cross-Set References](cross-set-references.md)) would each change checksum, which is a rewrite of authored records rather than an addition to a header — safe only while no second store holds these sets, which is the window [absorption's reasoning](multi-set-sync.md) describes. Newly authored links get their identity automatically; the existing ones can be left carrying names indefinitely, because a link with only a name resolves exactly as well as it always did. So this step is optional, and doing it is a decision about tidiness with a real cost, not a completion of the migration.

## Built 2026-08-31: `absorb` takes one whole set into another

The section above moves *memories* between sets and is deliberately a procedure rather than a tool: the judgement is a human's and the capability already exists. Moving a **whole set** into another is a different operation, and it does need one — because the thing that has to happen to the links cannot be done by appending records to a file.

`hoard_sets` action `absorb` takes `scope` (the set doing the absorbing), a `directory` or `file`, and `save_to`. Its full reasoning is in [Multi-Set Sync](multi-set-sync.md#absorption-becomes-a-supersession-which-removes-the-only-receiver-side-rewrite); what belongs here is the lifecycle shape.

**It reads a file that declares another set, and that inversion is the point.** Every other reader here requires the file to declare the set named in `scope`. This one requires it not to, which turns *this file must be mine* into *this file is someone else's and I mean to take it in* — intentionality expressed as an operation rather than inferred from a header.

**One operation, not a composed sequence, and the precedent is `restore`.** The decomposition — `create`, `import`, `absorb`, `drop`, `export` — has a dangerous failure mode, and it is not the one that looks dangerous: stopping *after* the absorb leaves the memories in two sets under one set of ids, with nothing saying you are half done. It also uses `create` as a staging area for a set you intend to discard, which the tool cannot tell from the real thing. Where the absorbed set happens to be loaded on this machine, the response says so for the same reason.

**The save-first rule applies although this is not a destructive operation.** `documents/design/set-lifecycle.md` states it for destructive ones, but its purpose is reversibility rather than deletion, and this is a bulk change that is tedious to unpick. So the safety copy is written first and unconditionally — with one difference from `drop` and `restore`, which is that an unreadable store is **not** proceeded past here. Those exist *for* the corruption case, so refusing would withhold the repair from the situation that needs it; this one is additive, and a set too damaged to read is not one anything should be merged into.

**Id collisions refuse before anything is written at all**, safety copy included: both sides are in hand, so comparing is cheap, and collapsing two distinct memories onto one id is the one thing set-scoped ids make possible and nothing else prevents. A record *identical* to one already held is not that case — it is one memory arriving again, which is what lets the same file be absorbed twice.

**Into `local`, it takes a `local` file, and nothing else does** (2026-09-30). Setting `SCATTERHOARD_COMMON` without moving `local.sqlite3` leaves two personal stores on one machine, each with its own identity. `import` refuses the second identity and names `absorb`, which until then refused every `local` file on the ground that `local` never leaves its machine, so a split personal store had no way back (`a-split-local-store-cannot-be-merged-back` in [Issues](../../TRACKING.md#issues)). For `local` the same set is told by identity, since every `local` file declares the same scope, and there are no links to make internal, since nothing links into `local` from another set. A shared set still never takes a `local` file, so personal memories never reach a published set by this route. The procedure was run end to end before the change and after it.

## Resolved 2026-08-06: one lifecycle tool, not five

The operations above are `create`, `export`, `import`, `restore`, `drop`, `status`. Adding six tools to a four-tool surface contradicts [Tool Surface](tool-surface.md)'s standing rule that every tool is a permanent charge against the model's attention, competing with whatever other servers the user has loaded.

The proposal is **one `hoard_sets` tool taking an action**, giving a five-tool surface rather than a ten-tool one.

The argument is not merely arithmetic. The four existing tools are used *constantly* — conventions once a session, memorize and recall throughout. Lifecycle operations are used at **setup and at commit time**, by a human or at a human's request, a handful of times in a project's life. Keeping the constantly-used surface small is worth more than keeping the rarely-used surface flat, and a model reaching for lifecycle work is already in a different mode.

This sits more comfortably than it first appeared with *"replace is a name, not a flag"* above. Both destructive actions are guarded by the same save-first rule, which does not depend on the operation's name to slow anyone down, and the action name still does its legibility work in the transcript. Naming was never load-bearing; it was standing in for a structural guard that now exists.

**Resolved 2026-08-06, by building it and using it.** One tool with an action, and the reasoning is recorded in [Tool Surface](tool-surface.md) rather than restated here. The short version: the frequency gap between constantly-used and occasionally-used tools is large and real, ten tools would have been the alternative, and the action enum turned out to carry weight nobody anticipated — a `oneOf` branch per action publishes only that action's arguments, so a call combining arguments from two actions is not one the schema invites, where a flat tool with everything optional is the shape that produced `schema-underspecifies-scope-and-links`. *(Said "cannot be constructed" until 2026-08-11; the branches inform rather than constrain, and a flat view is now published beneath them so a consumer that ignores `oneOf` can still call the tool at all — see [Tool Surface](tool-surface.md#corrected-2026-08-11-the-branches-inform-they-do-not-constrain-and-they-cannot-stand-alone).)*

The counter-argument — that a wrong action value and a wrong tool name are equally available mistakes — held up and turned out not to matter. Both are refusals, and the action refusal is the more useful, because it can enumerate what would have worked.

## Open questions

- **Where the export path comes from.** A shared set in a repository exports to `scatterhoard_mem/memories.json` in the repository, but nothing in the server knows where the repository is. Does `export` take a path, is it bound at `create` alongside the set name, or is it derived from the same trust-on-first-use record? Entangled with the set-name binding in [Open questions](../../TRACKING.md#open-questions). *(Answered in practice: `export` takes the path on every call, in two forms. The set-name binding was dropped on 2026-09-25.)*
- **Whether `Project` is still the right name for the scope kind**, given it now means "a named, exportable set" rather than "belongs to a repository" — see *Identity travels; location does not* above. A rename is cheap now and gets expensive once sets exist in the wild carrying it in their headers.
- **What `create` does when the database already exists.** Refusing is the safe default and matches `drop`'s posture, but a set whose database was dropped and whose file survives should be re-creatable — which may make that case `restore`'s job rather than `create`'s. *(Answered 2026-09-28: `drop` removes the database, so `create` proceeds, and `create` then `import` is how a dropped set comes back, recovering the identity in its own file. What `create` refuses is a database on disk that this server has not loaded.)*
- **Where `forget` lives.** Tombstones make forgetting a concrete operation ([File Format](file-format.md)), and it has no tool. It is deliberately **not** in the action list above: it acts on a single memory rather than on a set, so it belongs beside amendment in [Open questions](../../TRACKING.md#open-questions) under *how a mis-recorded memory is corrected*. Recorded here so the omission reads as a decision. What it must *do* is already settled — delete the content, the keywords and the vector from the store, and leave a tombstone carrying id and attribution only, because the operation exists partly to remove sensitive data and a flagged row that still holds the text removes nothing.
- **Whether `drop` should also be able to remove the file.** Deliberately excluded here: the file is usually in a git repository, where deletion is git's job and a memory tool reaching into a working tree to delete a tracked file is a surprise. Recorded so the omission is a decision rather than an oversight.
