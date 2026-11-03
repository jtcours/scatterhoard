# Store Registry

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

Where a memory set's database lives, and how a scope name resolves to one. [Set Lifecycle](set-lifecycle.md) covers the operations on a set; [Multi-Set Sync](multi-set-sync.md) covers how a store and a file reconcile. This document covers the layer underneath both: which store an operation acts on in the first place.

**Status: proposed 2026-08-06, and mostly still proposed.** It is a re-architecture rather than an addition: if adopted it supersedes the per-target reconciliation base in [Multi-Set Sync](multi-set-sync.md), closes an open question in [Open questions](../../TRACKING.md#open-questions), and changes what `create` and `status` mean in [Set Lifecycle](set-lifecycle.md). It should not be started while defects are open against the current shape.

**Two parts are decided and built**, both on 2026-08-10: [each store records its own author](#resolved-each-store-records-its-own-author), and [checkout-bound and machine-bound sets](#resolved-checkout-bound-and-machine-bound-sets). They were promoted ahead of the registry they were proposed alongside because neither needed it — both describe a defect or a want that the *current* architecture already has, since two checkouts share one `SCATTERHOARD_HOME` today whether or not anyone chose that. Every heading still reading `Proposed:` is unbuilt.

**A third part is decided and has nothing to build yet:** [never keep a reconciliation base you cannot prove](#resolved-never-keep-a-reconciliation-base-you-cannot-prove), 2026-08-11. It is a constraint on a migration rather than a mechanism, and it binds whichever route eventually closes the per-target base — so it is settled ahead of that choice rather than as part of it.

## The problem: many checkouts, one store

Today a store is keyed on the set name alone — `shared-<name>.sqlite3` in a central data directory ([store.rs](../../src/store.rs)) — so every checkout of a repository shares one database. That was a deliberate choice, and [Multi-Set Sync](multi-set-sync.md) defends it on the grounds that memories belong to the project rather than to the directory it was cloned into.

**Three separate defects all descend from that one mapping**, and each was found while trying to fix the one before it:

- **The reconciliation base had to grow a target key.** One base per record cannot describe agreement with two files at two commits, so the base became `(record, target)` where the target is the export directory. That is correct given the mapping, and it is the source of everything below.
- **A target is a path string, and paths are not identities.** `target-identity-is-an-unnormalised-path-string` in [Issues](../../TRACKING.md#issues): two spellings of one directory are two targets with two independent base sets, and import reconciles with no ancestor and says nothing. Canonicalisation narrows this and cannot close it — a renamed or moved checkout orphans every base just as thoroughly.
- **Nothing binds a set name to a repository.** *Where the trusted set name for a checkout comes from* has been open in [Open questions](../../TRACKING.md#open-questions) since the beginning. Because one store answers to a name and any directory may claim that name, a repository that keeps the header of the project it forked from merges into your set on import — with no conflict, because additions never conflict, and with attribution it wrote itself. Filed as `imported-memories-carry-no-arrival-provenance`. **Sharpened 2026-08-20:** the binding is the load-bearing fix here rather than the record of arrival that issue proposes — one store serving two repositories is also what breaks the judgment gate's skip rule in [Export Review](export-review.md), so the binding closes two things and the record of arrival is the fallback if it is refused. **Closed 2026-09-25 without a binding:** legitimacy is the checkout's provenance, and the merge described here is stopped by identity — a file declaring another `set_id` is refused on import. See [Set Lifecycle](set-lifecycle.md#no-trusted-name-for-a-checkout-and-an-identity-mismatch-is-refused).

Each has a local fix. None of the local fixes touches the cause, which is that **one store is trying to be several replicas at once.** `git` already knows how to be several replicas.

## Proposed: one store per checkout, selected by a registry

A scope resolves to exactly one store at a time, and a store belongs to exactly one checkout.

- Stores live in a central directory, **named by a UUID** assigned when the store is made.
- Each store records **its own checkout path** in `meta`, beside the scope already kept there — or records none, if it has not been attached to a checkout.
- A machine-local **registry** records, for each scope, which store is currently live.
- Re-pointing a scope at a different checkout **switches which store is live**. It does not redirect one store's exports.

A store therefore belongs to **at most one** checkout, never to two. It may belong to none, which is a legitimate and possibly long-lived state — see *an unattached set is a first-class state* below.

What this buys is that a store has at most one export destination, so the base collapses from `(record, target)` back to `(record)`. No target key, so no path spelling to normalise, no unrecognised-target case, and nothing to warn about. Two checkouts stop being one store pretending to be two replicas and become two replicas, reconciled through `git` the same way their code already is.

**The cost, stated plainly because it is the thing to weigh:** memories recorded while pointed at one checkout are not recallable while pointed at another. They travel by export, commit, pull, import — exactly as the code does. Anyone who expects a memory set to follow them between checkouts will be surprised once, and the answer is the same answer `git` gives.

### Why the stores stay central rather than in the working tree

Putting the database inside `scatterhoard_mem/` was considered first, and it is genuinely attractive: it makes the store self-locating, so a moved or renamed checkout carries its store with it and no registry is needed at all. Two things rule it out.

**An in-tree store dies with an ordinary `rm -rf`.** Deleting a stale checkout takes its unexported memories with it. That bypasses the entire save-first rule in [Set Lifecycle](set-lifecycle.md), which exists so that memories cannot be discarded without first being written somewhere the caller named — a guarantee that means nothing if a working-tree deletion sidesteps it. People delete checkouts casually and `drop` is rare, so the unguarded path would be the common one.

**An in-tree store can be committed.** That would ship unexported memories straight into the repository, bypassing the [Export Review Gate](export-review.md), which reads a `git diff` of the *file*. A committed store is an unreviewed export, and it would carry one machine's local state to everyone who pulls. A `.gitignore` inside `scatterhoard_mem/` handles the ordinary case and propagates with the repository, but *"the review gate holds as long as an ignore rule does"* is a weaker position than *"the database is not in the tree."*

A registry also turns out to do the self-locating job better rather than worse — see the next section.

### Why a UUID name, and why the store owns its path

The store's identity and its location must be separable, because the location changes and the identity must not.

- **Identity is the UUID**, fixed when the store is created and never rewritten — before any checkout is involved, which is what lets an unattached set exist at all. Nothing about a checkout — its path, its branch, its remote — can invalidate it.
- **Location is the path recorded in the store's `meta`**, which is a label to be corrected rather than a key to be matched. A moved checkout is one row to update, not an orphaned store.

The alternative was to **encode the path in the filename** (`Users_alice_Documents_projects_example.sqlite3`), which has the real advantage of being legible in `ls` and recoverable with no tooling at all. Rejected because it must be *renamed* when a checkout moves, or it states something false — and a filename that confidently asserts the wrong path is worse than an opaque one, because nothing prompts a reader to doubt it. This is the single-source-of-truth failure recorded in [STANDARDS.md](../../STANDARDS.md) as this project's largest defect source: a fact restated by hand in a second place, where both copies stay internally consistent and nothing goes red. Recovery survives without it — `sqlite3 <file> "select * from meta"` answers the same question, and `status` prints the whole mapping in the ordinary case.

### What the registry holds, and what it must not

**The registry holds one fact: which store is live for each scope.** That is the only thing no store can answer about itself.

Everything else about the population of stores — which scope each claims, which checkout each belongs to, how many memories each holds — is **derived by scanning the store directory** and reading each store's `meta`. Storing that separately would create a second copy that can disagree with the first, which is the defect class this project keeps filing.

**The path may be cached in the registry**, on one condition. The lookup made most often is *path → store*, because the LLM's working directory is what it has; a pure scan makes the common case the expensive one. Caching is defensible on exactly the terms [Set Lifecycle](set-lifecycle.md) already accepted for the dirty flag — *"a cache that spares `status` from rehashing a set, rebuildable at any time — not a second fact that can drift from the first."* The condition is that **something actually rebuilds it**, and the answer is that `status` does, on every call. A rebuild that is documented but never run is not a rebuild.

## Proposed: a registry per home

A single machine-global registry has a defect the rest of this design does not: **it is shared mutable state between server processes.** A server is registered per checkout, so two checkouts open at once means two processes, and one "which store is current for `scatterhoard`" is something they will fight over — the review session re-points, and the development session silently changes sets underneath itself.

The fix uses machinery that already exists. `SCATTERHOARD_HOME` (`data_directory` in [main.rs](../../src/main.rs)) already locates everything a server owns. **Put the registry in the home, and give each server its own home.** Two homes, two registries, nothing shared, no negotiation.

**Still proposed, and now largely superseded.** With re-pointing settled at startup rather than at runtime, the registry's one fact becomes derivable from where a store's file sits — see [checkout-bound and machine-bound sets](#resolved-checkout-bound-and-machine-bound-sets), which is what got built instead. This section is retained because two of the registry's three motivations survive it, and neither is served by placement:

- **The per-target reconciliation base.** `target-identity-is-an-unnormalised-path-string` is about two *spellings* of one directory on two export calls, and the caller supplies that string every time however many checkouts exist. A registered destination dissolves it; a directory layout does not touch it.
- **Binding a set name to a checkout.** `import` still takes a caller-supplied scope, so nothing stops a repository claiming a set name. Registration would be that binding.

What is gone is the *switchboard* — the runtime pointer two processes could fight over — and with it the question this document called its largest gap.

## Resolved: checkout-bound and machine-bound sets

**Decided and built 2026-08-10.** Giving each server its own home splits things that should not be split. `local` is the obvious one — personal and machine-wide, not about any project, so having two of it is not a configuration anyone would choose. It is **not the only one**, and treating it as the only one was the first version of this decision, shipped and superseded the same day.

- **`SCATTERHOARD_COMMON`** — the directory holding stores that belong to every checkout on this machine. **Defaults to `SCATTERHOARD_HOME`.**

Set neither and nothing changes: one directory holding everything. Set `SCATTERHOARD_HOME` per checkout and leave `SCATTERHOARD_COMMON` pointing at one place, and several agents get independent project sets over one personal memory and one copy of anything else that belongs to no single checkout. **The simple case degenerates to today's behaviour exactly**, which is the property that makes it safe to introduce: someone who never sets it cannot be affected by it.

### The line is not local-versus-shared; it is checkout-bound versus machine-bound

You separate a store when it is coupled to something that differs between checkouts. A project set is: it has a working tree and a branch, and a review checkout that changes branches constantly *should* have a store that is cheap to discard. A cross-project set is not. `coding-lessons` has no working tree, and a lesson learned while reviewing is just as true while developing tomorrow — splitting it loses that with nothing gained.

So `local` and `coding-lessons` are the same kind of thing, and the first version of this decision reached only one of them. It looked complete because **`local`'s scope *type* carries the answer**: the software can tell it is machine-bound without being told. A shared set's type carries nothing — `coding-lessons` and `scatterhoard` are both `Scope::Shared` and only intent separates them. That is the whole gap, and it is why placement has to be *said* for a shared set and not for `local`.

**Where the file sits is how it is said.** A store in the common directory is machine-bound; a store in the home belongs to that home alone. `StoreLayout::discover` in [store.rs](../../src/store.rs) scans both and resolves each scope to one file. That is one type with tests against it rather than lines of startup wiring — the local store being opened, the directories being scanned, a created set being written, each choosing a directory separately, all having to agree, and **nothing noticing if they stopped**. That is the shape of `project-stores-get-no-embedder`.

### Why this is the registry, with its one fact derived

[The registry](#proposed-a-registry-per-home) is meant to hold exactly one fact: *which store is live for each scope*. Everything else it might have held is derived by scanning, deliberately, so there is no second copy to drift.

**With re-pointing settled at startup rather than at runtime, that last fact is derivable too** — a scope resolves to whichever directory holds its file. A recorded registry earns its keep only when two stores can claim one scope in the same place and something must choose between them, which is the runtime switchboard. Removing the switchboard removes the reason to write the fact down.

So this is not a cheaper substitute for the registry. It is the registry with its single fact derived rather than stored, which is the move this document already makes for every other fact it considered keeping.

**The human partner settled the assumption it rests on, 2026-08-10:** a restart is acceptable for changing which sets a server reaches. Power users who need a layout at all will tweak it rarely, and on Claude Code a server can be reconnected from `/mcp` without starting a new session — see [SETUP.md](../../SETUP.md). On Claude Desktop it is an application restart, which is the price to weigh if that ever changes.

**That decision is about re-pointing, and it stands; a set appearing in the same directories is not
re-pointing.** Since 2026-10-16 a server rescans its directories before every tool call and opens any
set whose database has appeared there since -- one another server sharing the home has created -- by
the same scan and loading startup uses. `a-set-created-by-another-server-is-unreachable-until-restart`
in [Issues](../../TRACKING.md#issues): several servers share one home in ordinary use, so a set made
in one was otherwise unreachable from the rest for their whole sessions. Changing *which directories* a
server reads still takes a restart.

### A set becomes machine-bound by being moved, and creation cannot decide it

`create` always writes to the **home**, because nothing at creation time knows whether a set will turn out to belong to every checkout. That is a fact about how a set gets *used*, learned later — often much later, since the pre-publication life of a cross-project set can run for months.

So becoming machine-bound is a move, and for now a manual one: no tool action does it, on the scope note in [Conventions](conventions.md). **The procedure is one instruction, and it is short because of a property worth recording**: the store runs in WAL mode, and SQLite checkpoints and removes the `-wal` and `-shm` sidecars when the last connection closes cleanly. Verified 2026-08-10. So with the servers stopped there is exactly one file to move, and moving it carries every memory. It is only with a server *running* that the sidecars exist and matter.

That the servers must be stopped is not a limitation to design around — **an LLM cannot start or stop its own MCP servers**, so this step is a human's either way. What it does argue is that if moving turns out to be routine, the case for a tool action is that it could do the move *without* a restart, not that it would save typing.

### The misconfiguration this makes possible, and what answers it

Point `SCATTERHOARD_COMMON` at a directory that has no store yet and the server does exactly the right thing: it creates one. Every symptom available to the caller then says `0 memories`, nothing wrong — while a store full of memories sits unread in the old directory. Measured 2026-08-10; the memories are never at risk, but reaching them again requires knowing what happened.

Two more states the split creates, both **reported and neither acted on**:

- **A set in both directories.** The common copy wins and the home copy is named in `status` as shadowed. The direction is deliberate and is the opposite of the first proposal: every way of reaching this state — a copy instead of a move, a stale original, a `create` that made a fresh empty store for a set already common — leaves the *home* copy as the wrong one, so letting it win would put an empty store in front of real memories. Refusing to load either would strand the session, so one is chosen and the other is named.
- **`SCATTERHOARD_COMMON` naming the home.** Harmless — it is the default written out — but it is a stated intention that does nothing, and a setting that looks applied while having no effect is the same shape as a `SCATTERHOARD_AUTHOR` that is silently ignored. Reported in `status`. This is why the code keeps *unset* distinguishable from *set to the home*: the ordinary case must say nothing, because nobody asked for anything. Conflating the two also made every set shadow itself in the default configuration, which is a bug the distinction was introduced to fix.

**`status` reports the file each set is being read from**, which is what distinguishes *"you have no memories"* from *"you are looking in the wrong place."* A fact rather than a check: no heuristic decides whether the configuration looks wrong, and nothing can be wrong about it in the reassuring direction. The path comes from SQLite and is therefore fully resolved, which is the more useful of the two answers — it names where the data is rather than the route configured to reach it, and two homes reaching one store through a symlink report one file rather than two.

*A stronger check is possible — noticing a store in the home that this server is no longer reading — and is filed as `moving-the-local-store-leaves-the-old-one-unmentioned` in [Issues](../../TRACKING.md#issues) rather than built here, because it is a heuristic where the path is a fact.*

### Concurrent access is a requirement, not an assumption

Sharing one local store between servers makes multi-process access **deliberate**, so it has to be designed rather than survived. Three separate things are involved and only one of them is locking:

- **Locking.** A store file is left in WAL mode, so a writer in one server does not lock out readers in another, and a write that cannot take the lock waits rather than failing. Both are properties a *shared* store needs and a private one never exercises, so neither is observable from a single-process test — the checks live in `tests/concurrency.rs` and in the recorded mutations beside them.
- **Cache coherence, which locking does not fix.** The store holds its vector set in memory by design, so that opening a store does not put the embedder on the critical path. A cache is only safe while this connection is the only writer, and with the local store deliberately shared it is not. `PRAGMA data_version` is the detector: it changes when another connection has committed and never for this one's own writes, so a scan reloads only when it must. The discipline that keeps this from decaying is on `Store` itself — every read of cached state takes `&mut self`, so a new reader that forgets to refresh does not compile.
- **Within one process, safety is currently accidental.** The request loop is strictly serial — a property of the stdio transport, not a stated guarantee, and making the loop concurrent later would remove it silently. Sub-agents are ordinary in the clients this targets, and an isolated worktree agent may well have its own registration and therefore its own process. **This one is still open.**

The first two were filed as defects against the *current* architecture, because two checkouts already share one `SCATTERHOARD_HOME` by default — `store-is-not-configured-for-two-processes` and `vector-cache-goes-stale-across-processes` in [Issues](../../TRACKING.md#issues), both fixed 2026-08-07, ahead of the variable rather than as part of it. **This did not create them; it removed the luck that had been hiding them.** The same held for attribution, found the same way and fixed the same week — which is three defects that the *possibility* of sharing exposed and that sharing itself did not cause.

One correction worth keeping, because it is about method rather than about SQLite. The locking defect was filed from reading the store's one `PRAGMA` line and reasoning about SQLite's documented defaults, which are correct — its default `busy_timeout` really is zero. The waiting behaviour was already there anyway, because the *driver* sets five seconds of its own. Half of that entry described a database this project does not have: one built from the standard's defaults rather than from the library actually in use. See the `coding-lessons` memory set.

## Resolved: each store records its own author

**Decided and built 2026-08-10.** This section is promoted out of the proposal above; the rest of it — store placement and the registry per home — was still proposed at the time. The order is deliberate rather than incidental: the author is the part of the split that is *already* wrong, because two checkouts share one `SCATTERHOARD_HOME` today, so the local store is shared whether or not anyone chose it.

**Observed before it was fixed**, against two real server processes over one `local` store on 2026-08-10:

```
jeff         | Coffee before the standup, always.
jtcours@work | The standup moved to 10am.
```

One person, one personal store, two identities, decided by which window the memory was written from.

### The author belongs in the store, not in the environment

> **Since 2026-09-28 the environment supplies no author for a shared set at all**, and
> `SCATTERHOARD_AUTHOR` is gone. A shared set's first write is refused until someone says who they
> are in it, and only `local` takes a default, the operating system's account name
> ([Set Lifecycle](set-lifecycle.md#built-2026-09-28-create-makes-an-empty-set-and-its-first-write-settles-it)). The rule this section
> reached still holds and is why that works: the author belongs to the store, which records it at
> the first write and keeps it.

`SCATTERHOARD_AUTHOR`, read by `whoami` in [main.rs](../../src/main.rs), was per-process, and the split above breaks it. One local store opened by two servers with different `AUTHOR` values records whichever process happened to write — so the same human's personal memories carry inconsistent attribution, decided by an accident of which window they were in.

Per-scope configuration is the obvious repair and it is not quite enough, because the problem is about which *store* is being written, not which scope is named. So: **each store records its author in `meta`, defaulted from `SCATTERHOARD_AUTHOR` when the store is created.** The local store then has one stable author no matter who opens it, and each shared store carries the identity used for that project.

Two things recommend it beyond fixing the bug. It is what `git` already does — identity is per repository, for exactly this reason, because one person legitimately contributes under different identities. And it is the same pattern this document already applies to the checkout path: **a store owns the local facts about itself, and the environment supplies a default rather than the truth.** A variable read at process start is the wrong home for anything that must stay consistent across processes.

*The author is local state and never reaches a memory file's header. What travels is the per-memory `created.by` and `modified.by`, which are written from it at store time and are already part of the data model.*

#### Correcting it matters more than defaulting it well

The default chain ends at `$USER` (`whoami` in [main.rs](../../src/main.rs)), which has no reason to match the identity a contributor is known by in a repository. A smarter default is available — read `git config user.email` from the registered checkout — and it is **not** the right answer. It buys a better guess at the cost of a `git` dependency, works only for stores that have a checkout, and still guesses. **The lever is correctability, not accuracy at creation.**

So `hoard_sets` gains an action to set a store's author, and two supports that make a wrong value get noticed:

- **Report the author in the export response.** Export is the moment attribution stops being private, and therefore the last cheap moment to fix it. Same reasoning as `memorize-says-nothing-about-searchability` in [Issues](../../TRACKING.md#issues): state the fact where the caller can still act on it.
- **Report it in `status`,** which already has to scan every store.

**An update applies to future writes only.** Memories already stored keep the attribution they were written with, and the response says how many carry a different author, so the split is visible rather than discovered later.

### A store may be claimed, never taken back

*Decided while building it, 2026-08-10, and the part the original proposal did not settle.* "Defaulted from `SCATTERHOARD_AUTHOR` when the store is created" answers the store that is created and says nothing about the store that already exists — which is every store on this machine, and the only case the defect actually occurs in.

The rule is the one [Embeddings](embeddings.md) reached for the embedding model, arrived at independently here and then recognised:

- **An absent author is filled in automatically.** Whoever writes first claims the store for the identity their process is configured with. This converges, because every later process agrees on the value it finds.
- **A present author is never displaced by a process.** A differently configured server writes under the identity it finds, and says so. Only the explicit action moves it.

Automatic repair of a *mismatch* is what oscillates, and the reason is worth keeping because it is not obvious from either side: two servers would each restamp the store on every write, forever, and **each one's local experience is success** — it found a disagreement, applied the documented default, and the store now says what it expects. The alternation is only visible from outside. Gap-filling has no such shape; there is nothing to alternate with.

That the same distinction now governs two unrelated fields — which model built the vectors, and who the memories are attributed to — suggests it is a property of shared stores rather than of either feature. **A process may fill in what a shared store does not know; it may not overwrite what the store already says.**

Two consequences follow:

- **Reading does not claim.** A recall against an unclaimed store leaves it unclaimed. Otherwise the identity a store settles on is decided by whichever process happened to start first, which is the same accident this section exists to remove, moved one step earlier.
- **The disagreement is reported.** `status` says when this server is configured for an identity the set does not use, because a `SCATTERHOARD_AUTHOR` that is silently ignored is indistinguishable from one that is broken. Reporting rather than acting is what keeps the store stable. *(Withdrawn: the note fired forever on healthy sets, since identity follows a set's origin — `the-author-note-compares-against-the-wrong-thing` — and since 2026-09-28 there is no configured name to compare against. `status` now speaks only when the next write will be refused.)*

### Re-attributing existing memories: no tool action, and a procedure that works

> **Two findings from 2026-08-31 bear on this section, one narrowing it and one that may replace it.**
>
> **The procedure must be export-edit-*import*, never backup-edit-*restore*.** Measured in
> `a_backup_edit_restore_leaves_the_next_pull_with_no_ancestor`: `restore` clears every base by
> design, then records agreement only with the file it read, so the edit comes back with **no
> ancestor relative to origin** and the next ordinary pull conflicts on every record it touched. The
> import route works precisely because it edits the file whose base then advances.
>
> **And the whole procedure rests on the base**, which [the graph is the ancestor](multi-set-sync.md)
> proposes retiring. Without a base, `resolve` cannot tell *they changed it* from *we changed it*, so
> this route stops working rather than degrading. [Attribution as a
> reference](memory-data-model.md#proposed-2026-08-31-the-author-is-a-reference-and-the-name-lives-in-a-memory)
> is the proposal that would let re-attribution be a supersession instead of an edit — which needs no
> base, and would make this section obsolete rather than merely wrong.


**No bulk action is designed**, because only a human can tell the two cases apart: *"that was me under a name I no longer use"* is a label being corrected, while *"that was someone else"* is a record being falsified. Software cannot check which.

**But the capability exists, and it is not a workaround.** Export the set, edit the identities in the file, import it back. Verified end to end 2026-08-10; every edited record reconciles as `TakeFile` with no conflict.

This route is *better placed* than the tool action would be, which is why it is written down here as the answer rather than as a stopgap. The objection above was never that re-attribution is hard — it is that the judgement belongs to a human. Export-edit-import puts a human at exactly that judgement and produces **a reviewable diff** as the artifact, which is the same gate [Export Review](export-review.md) already applies to memories leaving a machine. A tool action would have to ask the question and accept whatever it was told.

**Why it reconciles cleanly rather than conflicting.** Export records the reconciliation base for that directory, so afterwards the file has moved and the store has not — which is the ordinary `TakeFile` branch of [reconcile.rs](../../src/reconcile.rs), the same one a pull takes. Nothing special happens, and no checksum is ever edited by hand: both sides recompute from the canonical serialisation.

**The property this rests on is that import writes a record exactly as the file gives it, attribution included.** That is load-bearing and easy to lose — stamping an imported record with the local identity is a plausible-looking one-line change, since `memorize` and `amend` both correctly do that, and it would foreclose this route entirely while also rewriting the authorship of everyone else's memories on every pull. It is held by `an_imported_memory_keeps_the_file_s_attribution_not_the_importing_server_s` and `an_identity_can_be_renamed_across_a_set_by_editing_an_export` in `tests/sets.rs`, and by a recorded mutation.

Four things to get right when doing it:

- **Tombstones carry attribution too**, in `forgotten.by`. A rewrite that covers only `created` and `modified` leaves one identity behind in exactly the records nobody re-reads.
- **The store's own recorded author is separate.** This rewrites the records; `hoard_sets` action `author` sets what future writes use. Doing only the first means new memories keep arriving under the old name.
- **Other export targets are disturbed but not broken.** Measured: a second directory whose bases predate the edit reports `kept_local` for every record — the re-attribution stands and the next export brings that target into line. It becomes a genuine conflict only where the other side independently changed the same records.
- **For a set others have already pulled, the diff rewrites authorship across history.** Large and alarming even when correct. `local` is the easy case, having no one to alarm, and is where the attribution defect actually bit.

The first of these is why the base warning is milder than it looks: rewriting `created.by` does change a record's checksum and so **invalidates every reconciliation base** — the same shape [Multi-Set Sync](multi-set-sync.md) records for a checksum-algorithm change — but the export that begins the procedure re-establishes the base for the directory being edited, and other targets degrade to `kept_local` rather than to conflict. Noisy for one cycle, as that document says, and not lossy.

## Resolved: never keep a reconciliation base you cannot prove

**Decided 2026-08-11.** The open question below asked whether an existing store should *become the registration for its most-recent target*, or whether the migration should *refuse and ask*. It weighed those as comparable options. They are not, and measuring the difference mostly answers the question.

### The asymmetry is arithmetic, not judgement

A base is the checksum this store and **one particular file** last agreed on. A base carried over from a different target is therefore not a weaker fact than no base at all — it is a false one, and [`resolve`](../../src/reconcile.rs) treats the two nothing alike:

- **No base.** The two present versions are compared directly. Identical is `Converged`; different is `Conflict`, which is surfaced and never resolved. **Nothing is written over.**
- **A wrong base.** Where it happens to equal what the store holds — the ordinary case, since a base is recorded at the last export and the store need not have moved since — the arithmetic reads *store unmoved, file moved* and returns `TakeFile`. **The store's version is replaced, and nothing is reported.**

One direction costs a human a look. The other loses a memory. Pinned by `a_base_carried_over_from_another_target_turns_a_conflict_into_a_silent_overwrite` in `tests/reconcile.rs`, stated as an invariant over the whole absent-base branch rather than as one more row of the table: an arm added there later that resolved in the file's favour would make this argument false, and the test is what would say so.

### The rule

**Never keep a base you cannot prove.**

| Targets in the store | What a migration does |
| --- | --- |
| None | Nothing. |
| One | Keep it, re-keyed if the key form changed and the two spellings **provably** denote one directory. The choice is forced, so there is nothing to guess. |
| Two or more | Keep at most one — the destination, where the migration establishes one — and **drop every other target's rows outright.** Where no destination is established, drop all of them. |

**Never merge across targets.** Taking the most recent base per record is the tidy-looking move and it is precisely the defect: it assembles an ancestor that no file ever agreed to, one record at a time.

This retires *adopt the most-recent target* as a general rule while keeping the case it was reaching for. Adopting one target's rows is right **only when that target is also where future imports come from** — which registration would guarantee by construction and a directory layout does not. The option was less wrong than it was a consequence of a mechanism no longer being built.

It also disposes of *refuse and ask*, on the grounds that the question cannot be asked. *Which of these two checkouts' agreements is the real ancestor* is not knowable from outside the store, so a human at that prompt has nothing to decide with. Dropping requires no one's judgement and cannot be answered wrongly.

### Canonicalising the key lands in the same place

The other route to `target-identity-is-an-unnormalised-path-string` — normalise the key rather than remove it — needs a migration too, and it is this one. Re-keying `/p/proj/` to `/p/proj` is safe exactly when the two spellings provably denote one directory, which is what canonicalisation establishes.

**What it must not do is treat a collision as a merge.** Two raw keys that canonicalise to one key carry two independent base sets, and wherever they disagree about a `memory_id` neither is the other's ancestor. Picking either supplies a wrong base for the file the other described. Drop both and let the next export re-establish agreement.

That both routes reach the same rule is why this could be settled now rather than after the choice between them: **the migration constraint does not depend on which one wins.**

*Built 2026-08-26, by the canonicalising route, in `Store::normalise_base_targets`. The rule transferred without amendment: spellings are grouped by what they resolve to, a group of one is re-keyed, and a group of two or more is dropped entire. The measurement below held — copies of all three live stores were opened with the migration in place and came out byte-for-byte unchanged, four targets already canonical and three unresolvable. **One case this section does not cover had to be decided**: a target that no longer resolves. It keeps its spelling rather than being dropped, which looks like an exception to* never keep a base you cannot prove *and is not — an unresolvable key can be matched by nothing but the path it names, so it can never supply an ancestor for a different directory. What would be lost by dropping it is a checkout on an unmounted volume, which comes back to the same path — though **that last clause assumes a path re-identifies the same volume**, and `a-target-path-can-come-to-mean-a-different-volume` in [Issues](../../TRACKING.md#issues) is the case where it does not. The decision rests on the inertness, not on the assumption.*

### What dropping costs, stated rather than waved past

Not nothing. A dropped base turns `(Some, None)` into `(None, None)`, so a record the two sides once agreed on that the store **no longer holds at all** returns as `NewFromFile` instead of being reported as an `Anomaly`. That state means a hand-edited database or a build predating tombstones — forgetting leaves a row, so the ordinary case never reaches it — but the failure is a silent resurrection where there had been a report.

A resurrection can be forgotten again; an overwritten memory cannot be recovered. The rule stands, and this is a cost it accepts rather than one it overlooked.

The second cost is one noisy cycle: every dropped base is re-established by the next export, and until then a genuinely divergent record reports a conflict. [Multi-Set Sync](multi-set-sync.md) already records that shape for a checksum-algorithm change — noisy for one cycle, and not lossy.

### Precedent, and what is actually time-sensitive

`Store::drop_cascading_bases` in [store.rs](../../src/store.rs) already migrated this table once, dropping it whole on finding the old foreign-key-cascading shape. It was right for this reason, which means the project's one existing `bases` migration already obeys the rule this section is writing down.

**This downgrades the urgency the question carried.** It was filed as the one open question where being late loses memories. True of a migration that guesses; false of one that drops. What is time-sensitive is therefore no longer the decision but the **guard** — the rule wants to be written down and tested before someone writes the tidy-looking merge. Measured 2026-08-11: this machine holds one shared store with six bases against a single target, `~/Documents/rust/projects/scatterhoard`, which is already canonical. Every migration described here is currently a no-op or a forced choice, and there is still nothing to get wrong.

## Proposed: one path per scope, and no local aliases

A scope is bound to one checkout at a time. There is no local alias mechanism — no `scatterhoard_dev` naming the same set at a second path.

The capability was considered, because a reviewing session plausibly wants two checkouts at once. It is rejected for a reason that would be very hard to recover from. **A set's name is shared identity**: it travels in the file header and in cross-set link targets, and every copy of the set agrees on it ([Set Lifecycle](set-lifecycle.md), *identity travels; location does not*). A local alias is per-machine state. A cross-set link written against `scatterhoard_dev` is meaningless on every other machine **and travels in the file looking authoritative** — machine-local state laundered into shared identity, silently, by an ordinary memory write.

Making that safe would require every link-writing path to resolve alias to canonical name without exception, which is a guarantee one forgetful code path away from failing, with a failure that is invisible locally and only wrong elsewhere. Under [Decision Placement](decision-placement.md)'s relationship to correctness by construction, the right move is to make it unrepresentable, and refusing the alias does that at no cost — because the workflow that wanted it is served better by inspection.

*If two live registrations are ever genuinely needed, the design to revisit is a typed handle that is structurally ineligible to appear as a link target — not a second name for a scope.*

## Proposed: inspection is read-only, and does not trust the header

The review workflow — examining what a contribution changed in `scatterhoard_mem/memories.json` while recording memories that will commit to your own branch — does not need the reviewed set loaded. The diff is visible to the LLM and the human alike, and review questions such as *"would this memory still be accurate once these code changes merge?"* are answered by reading, not by recall.

So there is a **read-only inspect action**: it reads a memory file at a caller-named path, reports its header and its records, and changes nothing. No store is touched, no base is recorded, no scope is bound.

**It is the one operation that should read a header without trusting it.** Everywhere else, `Expectation` exists so that a file's claimed scope can only ever *agree* with what the path already established ([File Format](file-format.md)) — the rule that closes the privilege-escalation channel in [Trigger and Loading](trigger-and-loading.md). Inspection grants no trust at all: nothing is loaded, so nothing can be escalated into. It should therefore accept a file claiming any scope and report what it claims, **labelled as a claim**. That is strictly more useful for review, since seeing a fork's header still asserting the upstream set name is exactly the thing a reviewer needs to notice.

## Proposed: status scans every store, every time

`status` opens every store in the directory, reads its `meta`, counts its memories, checks whether its recorded path still exists, and reports. It also refreshes the registry's cached paths as it goes, which is what licenses the cache.

**The scan must peek, not load.** Reading `meta` and a count has to be a distinct path from `Store::open` followed by `add_store`, because loading a non-current store would union its memories into recall and destroy the entire point of re-pointing. The failure would be silent and in the reassuring direction — recall working suspiciously well — so it is worth naming here rather than discovering it.

The states worth distinguishing, including one the design creates deliberately:

| State | Meaning |
| --- | --- |
| Current, path exists | Healthy. |
| Not current, path exists | Dormant. A checkout that can be switched back to; not a problem. |
| **Never registered** | **Unattached, and legitimate.** A shared set with no checkout at all — see below. |
| **Was registered, path gone** | **Orphaned.** The checkout is deleted and these memories may never have been exported. |
| Current, path gone | The live store for this scope cannot export at all. |
| Registry names a store that is not there | What the force-drop escape hatch produces — see [Set Lifecycle](set-lifecycle.md), *forcing a drop is deliberately not a tool*. Hand-deleting the database is blessed, so this state is expected rather than anomalous. |

**Memory count belongs in the report**, because an orphan holding two hundred memories is a different problem from one holding zero, and that number is what decides whether recovery is worth anything.

### An unattached set is a first-class state, not a degenerate one

A shared set need not belong to a repository. [Set Lifecycle](set-lifecycle.md) settles this under *a set that belongs to no repository* — the scope kinds are really *private-and-unbound* and *shareable-and-named*, and the second says nothing about checkouts.

The case that makes it concrete, and that this document originally missed: **a developer keeping project memories for a project whose maintainer has not agreed to carry a `scatterhoard_mem` directory.** The memories are about the project, belong in a named set rather than in `local`, and have nowhere to be exported to yet. That is not a broken state waiting to be repaired — it is the ordinary condition of a set before anyone is persuaded to adopt it, and it may last months. The pre-publication life of a cross-project set such as `coding-lessons` is the same shape.

So a store with no recorded path is **healthy**, reported as unattached, and never flagged. It gains a path when someone registers it, which is the moment the set acquires a home.

*An earlier draft of this document asserted that `coding-lessons` "has a repository, so it registers like anything else." That was wrong in the direction that erases this state, and it is the kind of wrong that would have shown up as an implementation refusing to create a set without a path.*

### Registration is explicit, and exporting elsewhere does not re-home a set

Since a set may have no path, something has to say when it acquires one — and it must not be `export`.

Binding the path implicitly on first export is the tempting shortcut and it is wrong, because **the safety copy required by the save-first rule goes somewhere unusual by construction** ([Set Lifecycle](set-lifecycle.md), *the path must not be the normal export path*). A set would re-home itself onto a backup destination at exactly the moment it was being protected.

So: registration is its own act, and `export` to any path other than the registered one is a **copy** — it writes the file and does not touch the base or the registration. Export to the registered path is the one that establishes agreement.

That recovers, for free, the distinction [Set Lifecycle](set-lifecycle.md) collapsed when it merged `backup` into `export`. The two operations were rightly made one, because the caller states a scope and everything else follows from it; now backup-versus-export follows from *whether the path is the set's home*, which is a fact the software already holds. Neither a second tool nor a mode argument.

### Orphans are flagged in the listing, not alerted on

An orphan — a store that *was* registered and whose path is now gone — is worth reporting, and reporting it means a marked row in `status`, with its memory count. Not a warning raised during ordinary work.

The argument for alerting more loudly was that an orphan is the passive form of the risk the save-first rule addresses: memories nobody discarded and nobody remembers. That much still holds, and it is why keeping the store alive when a checkout is deleted was the argument for central storage in the first place. But *reporting* and *interrupting* are not the same act, and the save-first analogy does not carry: that rule governs a destructive operation in progress, while an orphan is a standing condition that changes nothing about the work in front of the caller.

[Decision Placement](decision-placement.md)'s second test decides it. During ordinary memorize-and-recall work, knowing about an orphan changes nothing the caller does; it matters only to someone managing sets, who is the person running `status`. An alert anywhere else is a control with no decision behind it — and this document elsewhere treats a warning that fires on the routine case as one that gets clicked through.

**What makes an orphan noticeable is specificity, not volume.** *Orphaned, 212 memories, path gone* in a listing does the work that a repeated interruption would not.

No remedy operation is needed. An orphan can be exported anywhere, re-registered against a fresh clone, or dropped — and `drop` writes a safety copy unconditionally.

## What this changes in decisions already resolved

- **`create` still takes a name and no path.** Creating a scope and registering a checkout are different acts: a scope is created once, and registrations happen many times over its life. Folding the path into `create` would make it a mode of registration — the same conflation [Set Lifecycle](set-lifecycle.md) rejected when it refused to make `create` a mode of `import`. *(An earlier turn of this discussion had `create` gaining a path; that was wrong, and it was wrong for a reason worth keeping: it looked right only while the switchboard was assumed away.)*
- **The set-name binding question moves rather than closing.** *Where the trusted set name for a checkout comes from* is answered by registration: a human names the scope for a path, once, deliberately — the trust-on-first-use shape that document expects. What no longer needs answering is the *collision* half, because a hostile repository claiming your set name has nowhere to merge into. It reaches a store only if someone registers it or imports from it by name. *(The question closed on 2026-09-25 with no binding at all, so this is no longer a reason to adopt the registry; see [Set Lifecycle](set-lifecycle.md#no-trusted-name-for-a-checkout-and-an-identity-mismatch-is-refused).)*
- **"Identity travels; location does not" is not violated.** That rule objects to binding a path into a set's *identity*, where copies on other machines would disagree. The registry is explicitly machine-local and never enters a header. Recording a path locally was always fine; publishing it was not.
- **The per-target base becomes unnecessary**, and with it `Store::known_targets` in [store.rs](../../src/store.rs), the unrecognised-target remark on export, and the whole *dirty relative to what* discussion — which the save-first rule already stopped depending on. It also settles [Set Lifecycle](set-lifecycle.md)'s open question about what `status` reports: with one base per record, cleanliness is a plain boolean per set, and the per-target reporting question goes away with the per-target base.
- **The unconditional safety copy survives, but its argument has to be rewritten.** [Set Lifecycle](set-lifecycle.md) removed the dirty flag from the destructive path partly because bases keyed by `(record, target)` gave "is this set dirty" no single value. That leg does not survive here — one destination means one base and a well-defined answer. The decision stands on the leg that does survive, and on two this document creates:
  - A base records a past agreement, not the current contents of a file. A branch switch in the registered checkout rewrites `memories.json` with the base untouched — and with a single destination there is no second copy that might still be current, so this is *worse* under this design rather than better.
  - **An unattached set has no destination and so no base at all**, making every record unsaved by construction. A flag would need a "no base counts as dirty" special case and would then pass every time, for exactly the population most likely to be holding unexported memories.
  - **An orphaned store is where a flag is actively wrong.** It can report clean, because it genuinely agreed with its file before the checkout was deleted, and that file is gone. A flag-gated `drop` would skip the safety copy in the one case where the memories exist nowhere else.
- **`local` needs no registration**, and neither does any shared set that has not acquired a checkout. `local` is the permanent case — it has no repository by definition — while an unattached shared set is a state a set passes through, possibly for a long time. Both are stores with no registry entry, which is why registration has to be optional rather than a precondition for a set existing.

## Open questions

- [x] **~~Whether the current-store selection is machine-global or per server instance — the largest gap.~~ Settled 2026-08-10: per server instance, fixed at startup.** The question was hard only because two live processes might re-point a shared pointer underneath each other; with selection fixed when the server starts, there is no shared mutable state to fight over and the question dissolves rather than being answered. The cost it named for this branch — *"a per-checkout data directory removes the conflict but duplicates cross-project sets like `coding-lessons` across directories"* — is exactly what `SCATTERHOARD_COMMON` exists to answer, and working out that `local` and `coding-lessons` are the same kind of thing is what turned the cost into a design rather than a defect. The assumption it rests on was confirmed by the human partner: a restart is acceptable for changing which sets a server reaches.
- **Where the registry itself lives.** A table in the local store is the obvious candidate and makes the registry share the local store's fate; a separate file keeps it readable when the local store is not. The force-drop escape hatch argues for the latter, since hand-deleting a store should not require hand-editing a database to repair the registry.
- **What registration does when the path already has a `scatterhoard_mem/memories.json`.** Adopting it is the ordinary clone-and-start case; refusing is safer. Probably resolved by importing explicitly rather than by making registration do it implicitly.
- **Whether re-pointing away from a store with unexported memories should say so.** The save-first rule covers destruction, not dormancy, and re-pointing is not destructive — but it is the moment those memories stop being visible, which is the moment a human would want to know.
- [x] **~~Migration, and the fact that it is nearly free right now.~~ Settled 2026-08-11: never keep a base you cannot prove** — see [that section](#resolved-never-keep-a-reconciliation-base-you-cannot-prove). The question weighed *adopt the most-recent target* against *refuse and ask* as comparable options, and they are not: an absent base degrades to a reported conflict while a wrong one silently replaces the store's version. Dropping is therefore always available and never loses a memory, which settles every case the question left open. It applies unchanged whether the per-target key is removed by registration or normalised in place, so it did not have to wait on that choice — and the escape clause this entry rested on has since expired, the shared set now holding six bases where it once held none.
- **How the LLM learns which checkout it is in.** The server cannot see the session's working directory ([SETUP.md](../../SETUP.md)), and the model can. Publishing the registered path in `status` and in the `memorize` response lets the model compare against what it knows; whether that belongs in a convention is a separate question, and [Conventions](conventions.md) is deliberately being kept small.
