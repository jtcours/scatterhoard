# Multi-Set Sync

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

## Multiple simultaneous sets

The Scatterhoard system should be able to manage tracking multiple sets of memories at once.

It should not be necessary to flush a set of memories when switching to a different project (though that should probably be an option the code supports). Exporting simply creates a copy of the in-database memories; it does not automatically clear them. In fact, the database might contain memories from several projects that the user touched some time ago, though they might be out of date.

## The reconciliation problem

Reading a set of memories when there are already existing memories for a project creates the question of how to handle a couple of classes of project memories:

- Those that used to exist in the project's memories but that have been deliberately forgotten or superseded, and
- Entirely new memories created locally, such as ones created as part of work on the current branch.

## Resolved: git-driven merge, layered with a field-aware merge driver

The two options originally considered here were "flush and rebuild" (export, pull, merge the exported file via `git`, re-import) versus "merge on import" (keep forgotten IDs around and reconcile directly against the live database on read).

The deciding insight: these aren't really "simple vs. complex" — they differ in whether reconciliation leans on `git`'s own three-way merge (which has a real common ancestor to merge against) or reinvents that logic against live database state (which doesn't have an ancestor unless it reconstructs one, at which point it's just re-implementing what `git` already does, against rows instead of file lines). Given [File Format](file-format.md) already committed to consistent export ordering specifically to keep diffs small for merging, the intent was always to let `git` do this work. So: **merging two versions of the file is `git`'s job**, done against the real ancestor its history provides.

That settles the file-against-file half of reconciliation and only that half. The original phrasing continued *"then re-import the merged result"*, which quietly assumed the store held nothing the file did not — and by the time anyone pulls, it usually does. See *Reconciling a store against a file* below, which fills that hole and is the reason two of the conclusions in this section have changed.

### The two hard cases

- **Deliberately forgotten/superseded memories.** At the file layer this holds exactly as described: `git`'s three-way merge, having the real ancestor, treats delete-vs-unmodified as a clean delete and delete-vs-modified as a genuine conflict, correctly and for free. **The conclusion originally drawn from it — that no separate tombstone mechanism is needed — is overturned below.** A deleted line is unambiguous only to a reader holding the ancestor, and the store does not hold it. Forgetting now writes a tombstone; see [File Format](file-format.md).
- **Divergent edits to the same memory.** Same story — `git`'s ancestor-aware conflict detection already flags this correctly without Scatterhoard having to reconstruct three-way-merge semantics itself.

### The weakness: line-granularity vs. field-granularity

If the exported format is one JSON object per line, and two branches edit *different fields* of the *same* memory (e.g., one adds a Link, the other bumps Confidence), `git` still sees two edits to the same line and raises a text conflict — even though the changes are logically compatible at the field level. Plain line-based merging will over-report conflicts.

### The enhancement: a custom merge driver, as a graceful add-on, not a hard requirement

A custom `git` merge driver, registered for the memory-file type, can intercept exactly the cases where `git` would otherwise flag a line conflict, and attempt a three-way *field-level* JSON merge of that one record (base/ours/theirs) instead — falling back to a real conflict (for the LLM or human to resolve) only when the same field genuinely diverged on both sides.

This has to be a **graceful enhancement layered on top of plain `git` merging, not a dependency the whole scheme requires** — because registering a merge driver needs local, per-clone `git` configuration (an entry in that clone's `git config`, since `git` deliberately won't let a repo-tracked file specify arbitrary commands to run, for the same reason it won't auto-run hooks from a clone). That registration step can fail to happen or fail to fire silently, the same failure mode hooks are prone to. The saving property: if the driver isn't registered, or doesn't fire, `git` just falls back to its standard three-way merge — correct, just coarser-grained (more line-level conflicts than strictly necessary) — not silent data loss or corruption. That graceful-degradation property is exactly why this is safe to build despite the registration risk: worst case, it behaves like plain Option A above.

Scatterhoard's own installer/tooling should register the driver automatically on setup, rather than documenting a manual step for humans to follow — but given the mixed real-world reliability of `git` hook-like mechanisms, this needs hands-on testing before being relied on. Tracked in [EARLY-VALIDATION.md](../../EARLY-VALIDATION.md).

**Measured 2026-09-21: the degradation is graceful only when the driver is *unregistered*.** Registered but missing — a binary moved or never installed on that machine — is a different failure: `git` runs the command, gets exit status 127, reports a conflict, and leaves `%A` as the receiving side had it. No markers, none of the other side's memories, and a file that reads as resolved, so committing it silently drops the other branch's additions. So the registration [SETUP.md](../../SETUP.md#7-optional) documents is not the bare command but one that falls back to `git merge-file` on 127, which writes markers where it cannot merge; measured in a scratch repository with the binary absent and present. Registration is documented as a manual, optional step rather than performed by Scatterhoard, a decision taken with the human partner the same day: the driver resolves every case it reaches, but a host's merge button never runs it, so the plain-`git` behaviour of the file had to improve regardless — which is what [ordering by a hash of the id](file-format.md#resolved-the-concrete-layout) does.

#### Built 2026-09-16: the driver, and the policy it had to settle

**`scatterhoard merge-file BASE OURS THEIRS`**, positional in `git`'s own `%O %A %B` order, writing the result over `OURS` and reporting conflicts through its exit status. The merge itself is [`scatterhoard::merge`](../../src/merge.rs), a function over three things it can read — three texts in tests, three paths in the driver — the same shape as [`reconcile`](../../src/reconcile.rs) being a pure function over three checksums, and for the same reason: the part most expensive to get wrong is testable without a repository. It reads the three sides in step rather than whole, since all three are in file order ([File Format](file-format.md#resolved-reading-does-not-materialise-the-whole-set)), and writes the result beside `OURS` before renaming it into place, because `OURS` is still being read while the result is written.

**Three things the design did not settle, and what they turned out to be.**

**The header conflicts on every concurrent export, and cannot be recomputed.** Its `summary` is derived from the set's vectors, which a driver holding three files does not have. So the receiving side's header line is carried over verbatim. Safe because a summary is a claim a reader recomputes and compares — never a fact it displays — and the next export rewrites it from the store. This was not foreseen here and is the first thing a merge of two exported branches meets. **Superseded 2026-09-21: the header no longer carries a summary**, so nothing in it changes with the set's contents and two sides of one set write the same header ([export-review](export-review.md#4-what-the-set-amounts-to--derived-shown-and-never-judged) says why the stored copy went). The driver still keeps ours, which now matters only for a file an older build wrote. The other collision a plain `git` merge meets — memories added on both sides landing together at the end of the file — is unaffected, and is not something the driver can fix on a hosted merge, which never runs it.

**`modified` forces a decision.** Every edit moves it, so in exactly the case the driver exists for — ours bumped `confidence`, theirs added a link — the two sides *always* disagree there. Treating that as a conflict would mean nothing ever merges, so **the later attribution wins and the merged record claims an authorship neither side wrote**. That cost is accepted deliberately; the alternative is a driver that cannot merge what it was built for. `created` is the opposite and never moves, so a divergence there is corruption and conflicts.

**Collections are set-merged against the base, never unioned.** An item either side added is kept; an item either side *removed* stays removed. A union would resurrect a keyword one side deliberately retired, and retiring a keyword is the only way to take one out of circulation.

The rest follows: a scalar one side changed takes that side's value, one both changed differently conflicts, delete-against-modify conflicts, delete-against-unmodified applies, and a record only one side holds is kept — *absence means nothing*, since forgetting writes a tombstone rather than removing a line. **A conflicted record keeps the receiving side's version and the file still parses**; conflict markers would make it unreadable to every tool here, the pre-commit scan included, so `git` learns of the failure from the exit status alone. Merged records are written through `memory_file::write_record`, so a merged line is byte-identical to an exported one — anything else would make every merged record read as changed at the next import, since the reconciliation checksum is taken over exactly those bytes.

**[`.gitattributes`](../../.gitattributes) ships with the repository and the driver registration does not**, which is the asymmetry the registration risk lives in. Proving that half remains [EARLY-VALIDATION.md](../../EARLY-VALIDATION.md)'s, unchanged by this.

### Dangling links after a merge

If a merge forgets a memory that something else still links to, that's just a dangling link — [Cross-Set References](cross-set-references.md) already establishes a "soft failure, let the LLM decide" stance for dangling references across sets. The same philosophy applies here, within a set; no new mechanism is needed.

## Resolved: reconciling a store against a file

There are **two** reconciliations, not one, and the section above settles only the first:

- **File against file** — this checkout's exported memories against the ones arriving from `origin`. `git` owns this, with a real ancestor, optionally refined by the merge driver above. Unchanged.
- **Store against file** — the live database against whatever the file says after a pull. Nothing owned this. The store is a replica that `git` has never seen, and it is the one that has been changing all session.

### The hazard this closes

The old sequence was export, pull, merge, re-import. It is correct if followed in that order, and the order is load-bearing in a way nothing can enforce.

Let `B` be the file at the last export, `S` the store now, `F` the file after a pull. Export-then-pull works: the working tree holds `S`, so `git` merges `origin` against `S` with a real ancestor. **Pull-then-import is silent data loss**: the working tree still holds `B`, `git` fast-forwards it to `F`, and an import that replaces writes `F` over `S`. Every memory recorded that session is gone, with no error raised and nothing left to diff against.

`git pull` happens in a terminal Scatterhoard never sees. So "export first" is a rule the tool cannot check at the moment it is broken, and the penalty for breaking it falls on exactly the work that just happened. That is not a usability wrinkle to document; it is a data-loss defect waiting for its first real user, and the fix has to be a design in which the order does not matter.

### The mechanism: tombstones plus a per-record base checksum

Two changes make import order-independent.

**Forgetting becomes explicit.** With tombstones ([File Format](file-format.md)), absence of a record in the file no longer means deletion — deletion is a line that says so. This is what lets import keep a memory the file has never heard of instead of having to guess whether it was forgotten upstream.

**Each record carries the checksum it had when the store and the file last agreed.** Call it the *base*. It is local, per-replica state: it lives in the store beside the vectors and the export-review ledger, never in the file, because it records an agreement between two particular replicas and means nothing to a third.

### Resolved: the base is per record *and per target*

One store serves every checkout of a repository, because the database is keyed on the set name alone — and that is right, since memories belong to the project rather than to the directory it was cloned into.

But a developer may hold the same repository checked out twice, on different branches, for different purposes; the human partner runs exactly that, one checkout for development and one for review. Then there are two files, at two commits, and **one base per record cannot describe both.** Importing from the second checkout would read the first checkout's agreement as changes arriving from origin, and every record would show up modified.

So the base is keyed by `(record, target)`, where the target is the export destination. Each checkout reconciles independently against the shared store, and the store accumulates from both. The cost is one checksum per record per target — a ten-thousand-memory set with two checkouts is a few hundred kilobytes, so this is bookkeeping rather than scale.

The alternative considered was **one nominated target, with exports anywhere else being untracked copies.** Rejected: it saves state that is demonstrably cheap, makes the second checkout second-class, and if anyone ever imported from it the store would mis-merge silently — a footgun placed in a configuration people actually run.

**A fourth, from 2026-08-30, goes further: there may be no base to key.** [The graph is the ancestor](#proposed-2026-08-30-the-graph-is-the-ancestor-and-the-per-target-base-may-not-be-needed-at-all) argues that immutability plus the three ancestry-carrying operations make the per-record merge a lattice join, which needs no ancestor at all. If that holds, this whole section dissolves rather than being amended — including the target key it exists to justify.

**A third alternative was not considered at the time and is now on the table: give each checkout its own store.** Then a store has one destination, the target key disappears, and two checkouts are two replicas reconciled by `git` rather than one store emulating two. Drafted as a proposal in [Store Registry](store-registry.md), which would supersede this section rather than amend it. Recorded here because the reasoning above is sound *given one store per set* — it never examined that premise.

*Who may write memories from a reviewing checkout, and when, is a separate question and is open* — see [Open questions](../../TRACKING.md#open-questions). The base decision makes that case mechanically reconcilable; it does not settle the authority.

### Resolved: a 64-bit non-cryptographic checksum

The base is a **change detector, not a security boundary**, and that is what sizes it.

Trace what a collision buys an attacker with write access to the repository. The base decides only whether a record changed since the last agreement, so a crafted record colliding with the stored base makes reconciliation conclude *origin did not change this* — and therefore **not import their content**. The collision suppresses their own change. Every other branch needs control of the store's side, which supplying a file does not give. Nor can it launder anything past review: the gate in [Export Review](export-review.md) reads a `git diff`, and the review ledger keys on memory id and `modified`, not on this value.

A cryptographic hash is therefore not merely unnecessary but **misleading** — it would state a guarantee this value does not provide and invite a later reader to use it as an integrity or provenance check. Performance is not the deciding factor at these sizes either: a fifteen-megabyte set is tens of milliseconds of SHA-256 against one or two of a fast non-cryptographic hash, once per import or export.

**Width matters more than algorithm.** This compares one record against its own stored base, so it is not a birthday problem but a direct collision: roughly 2⁻³² at 32 bits, about one in four billion per changed record, and a miss means a memory silently stays stale — a correctness bug rather than an annoyance. Sixty-four bits takes it to about 5 × 10⁻²⁰.

Two consequences for implementation:

- **The algorithm must be pinned deliberately, and identified in the stored state.** Rust's `DefaultHasher` is documented as unstable across releases, so persisting its output would let a compiler upgrade invalidate every base on every machine. An externally specified algorithm at a fixed version is *safer than one written here*, because a value defined in our own code is one well-meaning refactor away from changing silently.
- **Getting it wrong is recoverable**, which is why the simple choice is the right one. The base is machine-local derived state and never reaches the memory file, so a change of algorithm means treating every record as divergent and re-establishing agreement on the next export — noisy for one cycle, not lossy. An algorithm tag stored beside the bases turns that from garbage comparisons into a detected mismatch.

The value is called a **checksum** throughout, deliberately and consistently: "hash" invites the cryptographic reading this section exists to rule out.

With the base, the checksum of the incoming record, and the checksum of the current store record, reconciliation is a five-way decision per id:

| File vs base | Store vs base | Resolution |
|---|---|---|
| same | same | nothing to do |
| differs | same | take the file's version |
| same | differs | keep local; it exports later |
| differs | differs, **and file == store** | converged independently — accept, clear the divergence |
| differs | differs, and file ≠ store | **genuine conflict, surfaced** |

The fourth row matters more than it looks: without it, two people making the same correction produces a conflict, and "conflict" stops meaning anything.

### Why checksums rather than timestamps

An earlier form of this used `modified` timestamps to decide which side was newer. Timestamps answer *which happened later*, which needs clocks that agree across machines and monotonicity nothing can enforce. Checksums answer *is this the same record*, which is local and verifiable.

Timestamps stay on the memory, where they serve LLM reasoning — a reader can absorb a record claiming to be created in 2026 and modified in 1998 and draw the obvious conclusion. A merge algorithm acting on the same pair would do something indefensible. **The clock is a good input to judgment and a bad input to arithmetic.**

### Observed 2026-08-26: this is a three-way merge over states, and the graph already holds histories

*The human partner's framing, and it narrows the problem more than it first appears.*

[`reconcile::resolve`](../../src/reconcile.rs) takes theirs, the ancestor and ours, and its outcomes are git's: `NewFromFile` is an addition, `TakeFile` and `KeepLocal` are fast-forwards, `Converged` is the same change made twice, `Conflict` is surfaced. Per-record identity makes it simpler than git's job — no rename detection, no line heuristics, no ordering questions.

**What it cannot ask is git's other question.** Given an older backup of this same set, git says *already up to date*, because commits carry parents and it can ask whether theirs is reachable from ours. A base is a single checksum — one agreement, not a lineage — so this merge is three-way over **states rather than histories**, and an ancestor is indistinguishable from a change.

**But the graph already carries ancestry for most of what changes.** A changed claim is superseded rather than amended ([Corrections](corrections.md)), so the old memory persists with an edge to the new one, and that chain travels in the file. A new memory is a root with a time-ordered id. **The exceptions are exactly two**: `amend`, which rewrites apparatus in place, and `forget`, which replaces a memory with a tombstone. Neither leaves anything saying which version came first.

So the question is not *should records carry ancestry* — most already do. It is **what to do about the two operations that do not**, which is a far smaller question and may not need a lineage at all: both produce a change that is detectable as *backwards* by other means, which is what the proposal below rests on.

#### The three cases want three different answers

*Worked through 2026-08-26, at the human partner's question of whether `git`'s model could be built on the graph as it stands. It splits three ways, not two, and treating the last two as one problem was hiding that they need opposite things.*

**Supersession needs nothing: it is already a DAG, forks included.** A changed claim mints a **new id** with an edge to the old one, so two machines revising one memory do not produce two versions of a record — they produce a genuine branch, and `resolve_supersession` returns **both ends** rather than choosing: `Superseded { tips: [ours, theirs] }`, surfaced on the wire as a list. That is `git`'s fork detection, built, and it was untested until `two_independent_supersessions_resolve_to_two_tips`.

**Forgetting needs one step, and only one, because it is terminal.** A tombstone shares the id it replaces, so it is a state change rather than a node and carries no ancestry — but nothing follows a tombstone, so no chain is needed. **If a tombstone recorded the checksum of the record it forgot**, a received tombstone becomes answerable: does what I hold match what you forgot? Matching means sequential, apply it; differing means you forgot a version I never had, so we are concurrent and it is a conflict. Today that distinction exists only where a base happens to survive for that target, which is exactly what goes missing in every defect filed against this list. One field, and it replaces the inference in *the clock as a check on the arithmetic's direction* below with proof.

**Amendment cannot be closed by ancestry, and one step is provably not enough.** Amendments *chain*: two amendments from your version leave a parent I have never seen, so a single parent pointer cannot separate descendant from sibling. Closing it properly needs the whole chain or a version vector per writer. Keeping the chain is refused by [Corrections](corrections.md) on grounds that still hold — apparatus has no history worth preserving, because nobody ever believed the wrong value.

**But the pressure on amendment is lower than it looks, and it is the granularity question wearing an ancestry question's clothes.** Most concurrent amendments touch *different fields* — one adds a keyword, another bumps confidence — and are logically compatible. The open section below on merge granularity is what would resolve them; what survives is same-field divergence, where **ancestry would not spare anyone a human anyway**. So the useful work for amendment is bringing the field-aware merge this document already plans for the `git` layer down to the store boundary, not building a version vector.

*A fourth answer is possible and would remove the case rather than solve it: **abolish amendment and supersede throughout**. See [whether one mechanism should replace two](../../TRACKING.md#open-questions).*

#### Resolved 2026-08-29: amendment's case closes by becoming unreachable

*The fourth answer was taken in a modified form, and the consequence for this document is larger
than the section it settles.* [Corrections](corrections.md) now licenses correction **only for a
record that never entered service**, where service means exported to any target, backups included.

**That makes concurrent amendment structurally impossible.** A record that has never been exported
exists on exactly one machine, so there is no second copy to diverge from; once exported, correction
is refused. Every change to a record another replica could hold is therefore a supersession or a
forget — and both carry ancestry once a tombstone records the checksum of what it forgot. Every
operation is then covered:

| Operation | Ancestry | How |
|---|---|---|
| new memory | yes | a root with a time-ordered id |
| supersede | yes | a DAG, forks included; `resolve_supersession` returns both tips |
| forget | yes | tombstone carrying the forgotten record's supersession edges **and** the checksum of the record it replaced (both built 2026-08-30) |
| correct | **not needed** | unreachable for any record a second replica could hold |

**All three ancestry cases are built as of 2026-08-30**, which closes the table above. The third
was the tombstone's, and what it needed turned out to be one field: `forgot`, the checksum of the
record it replaced. **It is consulted only where there is no base**, and that restraint is the
design rather than caution — where an ancestor exists the three-way arithmetic already answers
correctly, so deferring to it means nothing that reconciles today changes its answer. What the field
buys is exactly the case that had no answer, and it is worth stating why that case is the important
one: a file from a location this store has never agreed with is a fresh clone, a second checkout, or
a copy handed over directly — and it is also what a *stale* base degrades to, which is the whole
subject of `a-target-path-can-come-to-mean-a-different-volume`.

Without it, a forget arriving with no base resolved as `Conflict` for a reason that is true and
useless: **a tombstone's checksum never equals the memory's**, so *both sides changed* holds for
every forget that has ever crossed a boundary, and the arithmetic could not tell the ordinary case
from the dangerous one. With it, matching what the receiver holds means sequential and applies;
differing means the sender removed a version this store never had, and that is a real conflict.

*One case is deliberately left as it was: two machines forgetting the same memory independently, with
no base, still resolves as a conflict, because their tombstones differ in attribution and neither
`forgot` says anything about the other. It is the right shape for a later decision about whether two
forgets can converge, and it is not this one.*

**Two mechanisms designed for this problem are dropped, and are recorded here so they are not
rebuilt.**

- **A per-record checksum history.** Proposed 2026-08-29 by the human partner and worked out in
  detail: each record carries a list of checksums over its amendable parts, a correction appends the
  current checksum before amending, and reconciliation classifies by **membership** — my current
  checksum in your list means you are ahead, neither in the other means genuine divergence. It
  works, and it dissolves the refusal above, because the objection to keeping the chain was that
  *apparatus has no history worth preserving* and a list of checksums preserves no value at all,
  only ancestry. It also reaches what a version vector would reach **without introducing replica
  identity**, which this design has otherwise avoided needing. Two findings from the working-out are
  worth keeping if it is ever revived: a list suffices only if a merge **absorbs the discarded tip**
  as well as the new state, or a third replica holding the losing side sees a resolved divergence as
  a fresh conflict; and `modified` must be outside the checksum, or two machines making the
  identical correction can never converge. It is dropped because the service rule removes the
  operation it exists for.
- **Field-level and element-level merge granularity at the store boundary.** With correction
  unreachable after export, two copies of one id can differ only by one side having forgotten it, so
  the compatible-edits case the granularity ladder was built for cannot arise there. *The two halves
  of the system disagree about merge granularity*, below, largely dissolves rather than needing an
  answer — what remains is the `git`-layer merge driver, which is about file lines and is unaffected.

**The evidence that made the shape decidable** was a measurement of the two live sets rather than an
argument: 16 supersessions, 10 tombstones, 2 corrections, with both corrections 18 and 144 seconds
after creation and supersessions running out to 12.4 days. The operation that spans the
reconciliation window is the one that already carries ancestry; the operation lacking ancestry never
leaves the first two minutes, which close long before any export. Kept as
`01M18CDT5ZEQ0VMT1V97QCDZPW` in the `scatterhoard` set, with the caveat that no multi-party merge
has ever happened and the figure should be re-measured after one.

**Following that observation to `forget` found two defects in one step**, `an-older-file-silently-resurrects-a-forgotten-memory` and `an-id-can-hold-both-a-memory-and-a-tombstone` in [Issues](../../TRACKING.md#issues). The second is the sharper one and is not about ancestry at all: the record-at-a-time write path never clears a tombstone, so a resurrected memory sits beside the tombstone it replaced and the next export writes a file this project's own reader refuses.

**Built 2026-08-30, and the test suite is where the consequence became concrete.** With the gate in
place, *nothing in the tool surface can change an exported record in place*, which leaves exactly two
ways a store's copy of one can differ from the file it agreed with:

- a **`forget`**, which writes a tombstone rather than a new version, and
- an **import against a different target** — a second checkout, a colleague's branch, a backup —
  which writes the other side's version into this store while the first target's base stays where it
  was.

Three tests in [tests/sets.rs](../../tests/sets.rs) had built their divergence by amending after an
export, and could not: `a_memory_changed_on_both_sides_is_a_conflict`,
`a_memory_changed_on_one_side_only_is_not_a_conflict` and
`an_import_that_reverts_an_earlier_change_says_so` now reach the same reconciliation branches through
the second checkout. That is a strictly better fixture than the one it replaced — the amendment route
was the artificial one, since a divergence a user actually meets arrives through a repository — but
it is worth stating plainly that **the second target is now load-bearing for testing reconciliation
at all**, and a future change that made import re-baseline every target at once would silently make
this branch untestable rather than failing.

## Proposed 2026-08-30: the graph is the ancestor, and the per-target base may not be needed at all

*Worked out with the human partner, whose questions are the load-bearing steps: **why is the base still the ancestor**, **express the absorb rewrite as a supersession**, and the observation that a memory whose content differs must be a different memory. Recorded as a proposal and not a resolution — it contradicts [the base is per record and per target](#resolved-the-base-is-per-record-and-per-target) rather than extending it, and it has a falsification step that has not been run.*

### The argument

`correction-gated-on-service` made an exported memory **immutable**: nothing can change its content once a second copy could exist. The externally visible changes are then exactly three, and each carries its own ancestry — **creation** mints an id and has no ancestor, **supersession** mints a new id and names the one it replaces, **forgetting** replaces the record under one id with a tombstone.

So the state of any one id is three-valued and the transitions run one way:

```
absent ──create──▶ memory ──forget──▶ tombstone
```

A one-way, three-valued state space makes the per-record merge a **lattice join**, and a join needs no ancestor because it is order-independent and idempotent:

| ours | theirs | join |
|---|---|---|
| absent | memory | the memory |
| memory | memory | identical by immutability — converged |
| memory | tombstone | the tombstone; the transition is one-way |
| tombstone | memory | the tombstone, ours |
| tombstone | tombstone | forgotten either way |

`KeepLocal` and `TakeFile` both collapse into *the tombstone wins*, and `Conflict` on a memory record becomes **unreachable through sanctioned operations**, because one id never holds two contents. Every row is computable from the records themselves.

*The row **tombstone | memory** has been built on its own since 2026-09-28 — see [a forget is one-way at import](#resolved-2026-09-28-a-forget-is-one-way-at-import-and-is-undone-by-superseding-it), which also settles how a forget is undone.*

### What the base was buying, and what is left of it

The base is the ancestor of a three-way merge: it is what distinguishes *only one side moved* from *both moved*. Under immutability there is almost nothing left for it to distinguish — "we moved" for an existing id can only mean *we forgot it*, and the lattice answers that without an ancestor.

**And the direction of the failure matters.** Where a base is absent the answer is loud and safe; where a base is *wrong* the answer is silent and lossy. `a-target-path-can-come-to-mean-a-different-volume` in [Issues](../../TRACKING.md#issues) is caused **by** the base — a recorded agreement offered as the ancestor for a different replica — so removing the ancestor removes that defect rather than mitigating it.

What genuinely goes with it is one diagnostic: `Anomaly` for a record the two sides once agreed existed and the store no longer holds at all. That detects a hand-edited database, not a merge.

### Memory ids are unique within a set, and the set is identified by a UUID

*The human partner's scoping, and it formalises what the code already does.* `Store::incoming` refuses rows whose `target_set` is set, and says why: without it *"a row whose target names another set matches when its id happens to equal one held here… and this is the lookup `resolve_supersession` walks, so the collision would change a memory's standing."* Resolution is already set-scoped; this makes it the rule rather than a defence bolted onto one query.

The set's own identity then has to be more than a name two people can both choose — see [the set UUID](../../TRACKING.md#open-questions). With it, an unrelated set arriving under a shared name is refused at the header before any merge runs, which is the *make merges intentional* justification recorded there.

**The header verifies; it never gates.** A file can claim any UUID, so trust still comes from the path before contents are read ([Trigger and Loading](trigger-and-loading.md)). The bar is that the UUID be **stable**, not that it be tamper-proof.

### Absorption becomes a supersession, which removes the only receiver-side rewrite

> **Built 2026-08-31** as `hoard_sets` action `absorb`, which is the one part of this proposal that
> is no longer a proposal. `memory_file::absorb`, `import`'s `absorbing` argument and
> `Store::import_absorbing` are gone; `restore-silently-reverts-an-absorption` is closed and
> `a-hand-merge-into-a-new-set-name-leaves-supersessions-inert` is closed again by the new
> mechanism. What the operation settled beyond what is written below is recorded at the end of this
> section.

`memory_file::absorb` rewrote a cross-set link into a same-set link on the way into the store. It is the **only place in the system where a receiver edits authored content**, and it is what breaks the lattice: it produces one id holding two contents, with no supersession between them, from a legitimate operation.

It is also lossy — `LinkTarget::CrossSet { set, id, keywords }` becomes `SameSet(id)`, dropping the set name and the `keywords` that exist so a dangling reference can be re-found — and the stripped form is what gets exported, so whoever absorbs first destroys that for everyone downstream.

**Expressed as a supersession it stops being an exception.** The absorbing store authors a new memory with the same claim and same-set links, superseding the original. One id, one content; the ancestry is explicit and travels in the file; the original keeps its cross-set link, keywords intact, as history. And it is the rule this project already adopted rather than a new one: since `correction-gated-on-service`, **every** post-export change supersedes, whatever is wrong with it. Absorb was quietly doing an eleventh thing instead.

It is idempotent by construction: absorbing A and later A' supersedes nothing the second time, because the memories that had cross-set links into A already carry same-set links on their current tips.

**Two people absorbing the same set independently produce a fork**, and *that is accepted*. Both mint a superseding id for the same change, so the chain ends with two tips of identical content. The volume is bounded by the number of cross-set links **between the two sets**, not by the size of either — seven of the 60 links in the live `scatterhoard` set, all to `coding-lessons` — re-derived 2026-08-31 from the committed export, where it had read *six of 59* — and the alternatives cost more than the noise does: recording that absorption has happened reintroduces set-level state, and deriving the superseding id from its content would break the id contract the file's ordering guarantee leans on. Finding such pairs afterwards is [the claim digest](../../TRACKING.md#open-questions), filed separately.

#### What building it settled

Three things the proposal did not decide, each forced by an implementation choice that had to be made one way or the other:

- **An arriving record identical to one already held is not a collision.** The id check refuses *two different memories* under one id, which is the thing set-scoped ids make possible; identical bytes are one memory arriving again. Without the distinction, absorbing the same file twice — or a later export of the same set, which carries every record already taken — would be refused on every id it had already delivered, and the idempotence claimed above would hold for the supersessions and not for the operation. The response counts them as `already_held`.
- **Only memories that are *current* are superseded.** A superseded memory keeps its cross-set link because that is what it said when it was in service, and that is also what makes the second absorption of one file do nothing.
- **The order of the two halves matters, and it is: collect, then write, then supersede.** Candidates are collected before the arrivals are written, so *this set's own memories* means what it says and an arriving record cannot be superseded on the way in.
- **A link is made internal only if its target will be a memory here: arrived in the file, or already held** — settled 2026-09-28, closing `absorb-internalises-links-to-memories-that-did-not-arrive`. A whole set brings every target, but `absorb` reads a file carrying part of one, and internalising a link whose target did not arrive exchanged a working cross-set link for a same-set one to an id this set lacks. The rest keep their cross-set form and are named in the response under `not_internalised`; a memory with nothing to internalise is not superseded, and a tombstone does not count as held. *Already held* is what a move done in steps needs — a target taken in by an earlier absorb is the memory the link means — and it is safe because ids are generated to be universally unique: one id in two sets is one memory copied, and an arriving copy that differs from the one held is refused. The first build asked of the file alone, on the mistaken worry that a held id might be a different memory; the human partner's scenario of absorbing C and later B showed what that cost.

And one thing it cost, recorded under [`a-hand-merge-into-a-new-set-name-leaves-supersessions-inert`](../../TRACKING.md#issues): folding two sets into a file declaring a **third** name was expressible when `absorbing` took a list, and is now *absorb `b` into `a`* plus a header edit. Absorbing both into a freshly created third set works only in one order, and not at all for sets that link to each other — so the two-set form is the supported one.

### Cross-set links carry the foreign set's UUID, with the name kept as a hint

If a set is identified by a UUID, a link that *refers* to a set should use the identity and not the label — otherwise renaming a set breaks every inbound reference that has travelled, and two sets sharing a name make a link ambiguous.

The name stays, demoted to exactly the status `target_keywords` already has: **carried so a lost reference can be re-found, never sufficient to resolve one.** That matters more than it sounds. A file is read and merged by people, and the export gate is a human judgement — `"set":"coding-lessons"` tells a reviewer what a link means where a bare UUID tells them nothing; and if you do not hold the target set, the UUID alone does not tell you which set to go looking for.

### Two things to settle before any of this is built

**~~Falsify the immutability claim first.~~ Run 2026-08-30, and the answer is the useful one.** The premise is **false as the code stands**, and `absorb` is the only route: two holders of one set importing identical bytes, one passing `absorbing` and one not, end up with different content under one id, and reconciling them is a `Conflict` on a memory record. Every other write path was driven and leaves the author's bytes unchanged. So absorption is not *a* leak but *the* leak, and the supersession above closes the only one there is — which converts this section from a proposal resting on an untested premise into one resting on a measured one.

Two things came out of the attempt that the proposal did not anticipate. `restore-silently-reverts-an-absorption` in [Issues](../../TRACKING.md#issues) is the same root cause through a second door, and it is worse: restore takes no `absorbing` argument, so it reverts every rewritten link to a spelling that no longer resolves, and nothing on the surface can put it back. And the legacy route — an amendment predating the gate, met by a restore from an older file — is **unreachable through the surface**, because producing two files that differ for one id would need an amendment between two exports, which the gate refuses. The lattice therefore holds going forward and cannot be assumed for records written before 2026-08-30.

**Decide the record-shape batch, because the window is closing.** Adding `set_id` to a cross-set link changes the **record** shape, so it changes the checksum of every memory holding one. Applying that to existing records means rewriting them in place — the very act this proposal eliminates. It is safe *now* only because no second store holds these sets, so there is no receiver to diverge from. **This is the last moment any record-shape change can be made by rewriting rather than by superseding**, which argues for deciding the shape changes we are confident about as one batch: six records here, and an unbounded number of supersessions later.

### What it would retire

- The per-target base as a **correctness** mechanism, and with it `set_bases`, `merge_bases`, the target key, `normalise_target`, and the two-checkouts reasoning that [the section above](#resolved-the-base-is-per-record-and-per-target) settles.
- `a-target-path-can-come-to-mean-a-different-volume`, which the base causes.
- `target-identity-is-an-unnormalised-path-string`, which exists to give that key one spelling.
- ***Is this the same copy?*** entirely — the question replica identity was proposed to answer. What survives is *is this the same set*, which a UUID answers and which has nothing to do with copies.

### Open: the two halves of the system disagree about merge granularity

> ⚠️ **Largely dissolved 2026-08-29** by the service rule in [Corrections](corrections.md): with
> correction unreachable after export, there are no concurrent field-level changes to one record for
> the store boundary to merge. What is written below remains an accurate description of
> `checksum::of_record`'s atomicity; what has gone is the workload that made it matter.

*Noticed 2026-08-26 while working the framing above through, and true today.*

`checksum::of_record` covers the whole canonical record, so at the store boundary a record is **atomic**: one side adding a link and the other bumping confidence is a `Conflict`, though the changes are logically compatible. *The weakness: line-granularity vs. field-granularity* above identifies exactly this problem at the **file** layer and answers it with a field-aware merge driver — which helps two machines exchanging through `git` and does nothing for a store reconciling against a file.

So the same logical situation resolves differently depending on which path it took: cleanly through a merge driver, or as a conflict at the store boundary. Whether the store boundary should gain the same field-awareness, and whether the driver's fallback argument (*worst case, coarser-grained, never lossy*) transfers to a place with no `git` beneath it to fall back to, is undecided.

### Resolved 2026-08-26: the clock as a check on the arithmetic's direction

*Raised by the human partner, from the thumb-drive case in `a-target-path-can-come-to-mean-a-different-volume`. **This is not a retreat from the section above** — it is that section's own rule applied to a use it did not anticipate. The clock decides nothing here; it produces a sentence for a reader.*

**A `TakeFile` whose incoming version is older than the one it replaces is never what a pull means.** Every real edit bumps `modified`, so in an honest pull the file's version is the newer one, always. The reverse direction says the file is not a successor to what this store holds — it is a *predecessor*, and taking it is a regression to superseded content.

Today that arrives as `took_from_file: N`, which is exactly what an ordinary pull looks like. The proposal is to name those records and say which way the clock runs, changing nothing about what is written.

**What it catches is the realistic half of a defect nothing else sees.** A backup of *this same set*, taken earlier, read from a path this store has agreed with before: the set identity matches, the target key matches, the checksums are all legitimately different, and every record that moved on between the two backups is taken backwards. No replica identity is needed to notice it, and no new state, and no change to the file format.

**What it does not catch, stated rather than discovered later.** Equal timestamps say nothing. A clock set forward defeats it, in the direction that matters — which is a reason not to *decide* by it and no reason not to report. And it fires on an upstream restore, where a colleague's genuinely current content carries an old `modified`: a false positive, and arguably a word worth having there too, since that is the one other way to receive content that lost its own history.

**Built the same day**, as `taken_backwards` on the import response, together with the tombstone invariant it had to ship with — see `an-older-file-silently-resurrects-a-forgotten-memory` and `an-id-can-hold-both-a-memory-and-a-tombstone` in [Issues](../../TRACKING.md#issues). Two things settled in the building. The comparison lives in `import_set` rather than in `resolve`, so reconciliation stays a pure function of three checksums and the clock never enters the arithmetic. And `Record::as_of` reads `modified` on a memory and `forgotten` on a tombstone, which makes a resurrection compare like any other edit and lets **one comparison cover both operations that lack ancestry** rather than needing a rule for each.

### Resolved 2026-09-28: a forget is one-way at import, and is undone by superseding it

*Decided with the human partner while working out what would close `a-target-path-can-come-to-mean-a-different-volume`; recorded under `merge-without-a-recorded-ancestor` in [Open Questions](../../TRACKING.md#open-questions).*

**A memory arriving for an id this store holds as a tombstone is never written**, whatever the base says; the import reports it under `kept_forgotten`, and the base does not move. Until now such a record was taken and reported under `taken_backwards`, on the reasoning that a deliberate rollback — a `git revert` of the commit carrying the tombstone — looks identical to an older copy and only the caller can tell them apart. That is still true, and is exactly why neither is taken: from the record's side both are the memory's own old bytes coming back.

**Git tells them apart because a revert is a new commit.** An older copy is an ancestor, so merging it is a no-op; a revert is a descendant whose content happens to undo something. Per record, the equivalent is a new memory with a `supersedes` link to the forgotten id — a new event rather than an old state returning — and it already works: the tool accepts it, and recall's walk carries links aimed at the forgotten id to the replacement (`a_memory_superseding_a_forgotten_id_receives_its_in_links` in `tests/cross_store.rs`). The human partner first proposed a tool to tag a tombstone so that an import would overwrite it, and chose supersession as the cleaner route: no new tool, no new record state.

**What a forgotten replacement leaves behind, and what an undo restores** (2026-09-30). A memory whose replacement was forgotten stays superseded: forgetting a replacement does not reinstate what it replaced, which is what the tombstone's ancestry is kept for. It is reported with `replacement_forgotten` rather than `superseded_beyond_this_set`, because the remedies are opposite. One means load the set that holds the replacement; the other means write the claim anew, superseding the forgotten id, which `incoming` names. Once that undo exists, the chain resolves *through* the tombstone to it, and a link landing on any earlier member is walked there too (`a_link_before_a_forgotten_member_of_a_chain_reaches_what_undid_the_forget` in `tests/cross_store.rs`). `a-forgotten-replacement-reads-as-superseded-beyond-this-set` in [Issues](../../TRACKING.md#issues).

This is the lattice's *tombstone over memory, the tombstone* row, built on its own ahead of the rest of [the graph is the ancestor](#proposed-2026-08-30-the-graph-is-the-ancestor-and-the-per-target-base-may-not-be-needed-at-all). It is decided by what kind of record the store holds, never by the clock, in `reconcile::resolve_memory`. A memory-over-memory difference still goes through the three-way decision, which records written before 2026-08-30 still need.

### Why last-writer-wins stays refused, even where the clocks agree

*The section above rejects timestamps for arithmetic on grounds of fidelity: clocks disagree, monotonicity cannot be enforced. The human partner supplied a case on 2026-08-26 that needs none of that, and it is the stronger argument.*

Two users, one common ancestor, two records:

| | X | Y |
|---|---|---|
| **A edits at** | T1 | T2 |
| **B edits at** | T2 | T1 |

Resolve per record by *take the newer*, with perfect clocks and honest peers, and X comes from B while Y comes from A — **a pairing that existed on neither machine**. It is [Set Lifecycle](set-lifecycle.md)'s *"a set that never existed at any moment in time"*, reached with every individual decision locally defensible and only the combination indefensible.

**So the objection to last-writer-wins is about coherence, not only fidelity.** A per-record rule has no way to see that records constrain each other, and the damage grows with how related they are: where Y supersedes X, an alternating winner does not merely pick arbitrarily, it assembles a supersession that neither author wrote.

Reporting the conflict has none of this shape, because it hands both versions to someone who can see both records at once.

### Open: a conflict report says nothing about conflicts being related to one another

*Noticed 2026-08-26 while working the case above through.*

A conflicted record is never written, so the supersession-cycle check that runs over an import's writes never sees it. The report names conflicted ids independently, and a caller resolving them one at a time can hand-build exactly the pairing the section above says software must not: take B's X and A's Y, and the state that no machine held is assembled by a human instead of by a merge rule.

**That is categorically different and not obviously a defect.** The whole reason a conflict is reported rather than resolved is that a person can see both versions and judge; a person who does that with two linked memories in front of them is doing the thing the design wants. What is missing is narrower: **the linkage is information the report already holds and does not pass on.** Whether that is worth surfacing — and whether it generalises past supersession to any two conflicted records that reference each other — is undecided.

### The base is written in both directions

The base is updated on **import and on export**, because both establish agreement. Updating it only on import would make the next import read every locally exported change as an incoming change from `origin`, and the whole file would arrive as a conflict.

### The dirty flag is a cache, not a second fact

"Has this record changed locally" is already answered by comparing the current record against its base, so the per-record dirty flag is an **optimisation** — it keeps `status` from rehashing the set to answer whether there are unexported changes. The base is the truth and the flag is derived from it, rebuildable by rehashing. Stated in that order deliberately: two independent facts that can disagree is the failure this project keeps filing, and a cache with a known source is not that.

There is **no per-*set* dirty flag**, and the base being per-target is why. This sentence used to say the per-set flag was the disjunction of the per-record ones, needed by [Set Lifecycle](set-lifecycle.md)'s save-first rule. Both halves are now wrong: a record is dirty relative to a *target*, so a disjunction has to say which targets it ranges over, and the save-first rule stopped asking — it writes the safety copy unconditionally rather than consulting a flag it cannot trust. See *the save is unconditional* in that document for the argument. `status` reports what the per-target bases say; nothing collapses them.

### A record absent with no tombstone is an anomaly

Present in the base, missing from the file, no tombstone. Under tombstone discipline this cannot arise from any supported operation, so it means a hand-edit, a bad merge, or a file written by a build older than tombstones.

It is **surfaced, not resolved**. Reading it as "forgotten" is the silent-loss path this whole section removes; reading it as "keep" quietly resurrects things someone may have deliberately removed. Neither guess is available to the software, and a human or the LLM can settle it in a sentence.

This also constrains any future pruning of tombstones: a pruned tombstone is indistinguishable from this anomaly. That is a good argument for not pruning until there is a concrete reason, which — given that forgetting is expected to be rare next to superseding — there may never be.

### The exchange is whole-set in both directions

`export` writes the entire set and `import` reconciles the entire store against the entire file. There is no partial mode, and the base machinery assumes there is none: `Store::set_bases` **replaces** a target's whole base set rather than merging into it, on the stated grounds that an export writes the whole file.

So two parties — two developers, or two servers with separate homes — exchange memories by each exporting everything and importing everything. Nothing needs selecting, and the sequence does not matter, which is the property *what this costs* below already buys.

**Trimming records out of an exported file is not a subset export.** It is the anomaly above, arriving through an edit that feels routine — because editing an export *is* a procedure the design blesses elsewhere: [re-attribution](store-registry.md#re-attributing-existing-memories-no-tool-action-and-a-procedure-that-works) is export, edit, import. The difference is the one that matters here and is easy to miss: re-attribution changes records in place and removes none. A removed record the target had previously agreed on is reported, named, and written for not at all. Loud rather than lossy, and pinned by `a_record_trimmed_out_of_an_exported_file_re_imports_as_an_anomaly` in `tests/sets.rs`.

Genuine partial exchange is deferred rather than rejected ([Open questions](../../TRACKING.md#open-questions)). Worth recording that it would need less than it looks: **a recipient already handles a partial file correctly**, because holding no bases for that target means records they lack arrive as `NewFromFile` and records the file omits resolve to `KeepLocal`. What is missing is a way to say which records travel, and a base rule for a target that agrees with only part of a set.

### What this costs

`git`'s ancestor-aware conflict detection covered the store-against-file case for free in the old design, at the price of an ordering requirement that could not be enforced. We now detect those conflicts ourselves, from the base. That is real code with real edge cases, and it is the principal thing this change buys with effort rather than saving.

What it buys back: **the sequence stops mattering.** The file is never expected to hold store state, so pulling before exporting costs nothing, and the operation that used to destroy a session's work is now the one that merges it.
