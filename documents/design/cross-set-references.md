# Cross-Set References

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

Memory cross-references across sets should be possible but have the potential to cause confusion. For example, a personal memory that refers directly to a project memory could become a dangling reference if the project does not merge the memory into its repository or someone later causes the LLM to forget that memory.

It probably doesn't make sense to put a memory into a project's memory set that cross-references a personal memory. However, it might make sense to cross-reference a memory in a different project's set, at least if it's already merged into the main branch.

These considerations suggest a somewhat relaxed attitude towards failed cross-references, probably letting the LLM determine what to do about them rather than failing hard in software.

## Resolved: reference format

A cross-set reference stores the exact memory ID plus the name of the referenced set. Keywords from the target set may be included as supplementary context (to help the LLM re-find or re-establish the link if the ID goes dangling), but the ID + set name is the canonical reference — keywords alone are not sufficient to resolve one.

Dangling cross-set references are not a hard software failure; the LLM decides how to handle them (drop, flag, or attempt keyword-based re-resolution) consistent with the relaxed stance above.

## Resolved 2026-08-20: an unresolvable target is reported, never refused

*The LLM decides how to handle a failed reference* was unimplementable while nothing told the LLM a reference had failed. Storing a link ran no check of any kind, so a target that resolved to nothing was indistinguishable from one that resolved, at every later moment as well as at the moment of writing. This resolution supplies the missing half of the relaxed stance rather than qualifying it.

**Two conditions were being conflated, and only one of them is what the relaxed stance is about.** A *dangling* reference was meaningful and became unreachable — the target was never merged, or was forgotten, or lives in a set this machine does not hold. A *malformed* one never pointed anywhere: the commonest case is a cross-set reference written without its set name, which is then recorded as a same-set reference to an id that set does not contain. The first is an ordinary condition of a distributed set and the caller's to judge. The second is a mistake, and the information needed to catch it is in hand at the moment of the write.

**Both are nonetheless reported rather than refused**, and the reason is the claim-versus-apparatus split that [Corrections](corrections.md) already turns on. Links are apparatus. Refusing `hoard_memorize` over an unresolvable target would make an apparatus error cost the claim: a sound memory, correctly reasoned, not stored at all because of a field `amend` exists to repair cheaply. That is the wrong direction under *the unclear cases go the recoverable way*, and it is unnecessary — a report at store time reaches the caller in the same turn a refusal would.

It would also be wrong on its merits in a case that looks like a mistake and is not. A memory set is a replica of a git-synced file, so a store can lag the repository it came from; an id merged upstream but not yet imported is absent locally and is a perfectly good target. **That applies to a same-set target exactly as it does to a cross-set one**, which is why the check does not treat the two differently. Being in the set you are writing into confers no completeness.

### What is reported

The write reports each link it could not resolve, distinguishing four states — because a check that did not run must not be silent, which would make it indistinguishable from one that passed:

| Target | State |
|---|---|
| Same-set, id held as a memory or a tombstone | resolved |
| Same-set, id absent from this set | unresolved: no such id in this set |
| Cross-set, named set loaded, id absent from it | unresolved: no such id in that set |
| Cross-set, named set not loaded here | unchecked: the set is not held on this machine |

A tombstone counts as resolved. A supersession chain whose middle was forgotten still holds a valid reference, and checking only live memories would report a correct link as broken.

**Refusal remains correct for one case, and it is different in kind.** A `SUPERSEDES` edge closing a loop is refused, because a cycle is not an unresolvable target but a broken state that resolution genuinely cannot resolve, and the caller holds both ends of it in one call.

### Where the check lives, and where it must not

At the tool surface — `hoard_memorize` and `hoard_correct`'s `amend`, which replaces a memory's whole link array and therefore needs the identical check rather than a lighter one.

**Deliberately not in `Store::insert`.** That function is shared with the import path, where a per-record judgement costs the whole file, and this is not hypothetical: `two-import-paths-disagree-about-supersession-cycles` in [Issues](../../TRACKING.md#issues) records `check_no_cycle` reaching import through exactly that route and aborting a partly-written import. A write-time check added to `insert` inherits both callers whether or not that was intended.

*Per-label severity — a dangling `SUPERSEDES` mattering more than a dangling `RELATED_TO`, per the section below — is not settled here and does not need to be: it becomes a question about what one reported field says, rather than about whether the caller is told at all.*

### The link target carries the distinction

A link's target is one of two things, not a memory ID with optional extras:

```rust
enum LinkTarget {
    SameSet(MemoryId),
    CrossSet { set: SetName, id: MemoryId, keywords: Vec<Keyword>, set_id: Option<SetId> },
}
```

Within-set links are the common case and stay compact — the set name would be redundant on every one of them, and this file's format exists to keep merge diffs small. The supplementary keywords hang off the variant that can actually use them: a same-set reference has nothing to re-resolve against, so keywords there would be a field that is meaningless half the time. Same reasoning as `Scope::Shared(SetName)` — the variant carries what only that variant needs.

#### Resolved 2026-09-14: which of the two a stored link is, is decided by identity

**A link is stored as same-set when its `set_id` equals the holding set's identity, and as cross-set when the two identities differ — whatever names either carries. Only where one side has no identity does the name decide.** The write canonicalises a same-set link spelled the long way into the compact form, as it has since `a-same-set-link-spelled-with-its-set-name-is-inert`, but that fix compared names because links carried no identity when it was made. Set identities came later, the lookups (`holder_of`, `edges_into_another_set`) moved to identity, and the write did not — so a link inside a set naming that set by an old name was stored as cross-set and ignored by supersession, which only walks same-set rows: `a-link-naming-its-own-set-by-an-old-name-is-inert` in [Issues](../../TRACKING.md#issues). Decided by the human partner.

**It changes one behaviour on purpose.** A link naming this set's *current* name but carrying another set's identity used to canonicalise to same-set, by name; it now stays cross-set, the same answer resolution already gives a set that only shares a name.

**Collapsing both forms into `[set_id, memory_id]` was considered and declined.** It would remove canonicalisation, and with it the whole class of defect where the write and the lookups disagree. It lost on what moving memories between sets would then require: absorbing a set, or adopting another set's memories, would have to know the *source* set's identity to tell which links had been internal to it, where a relative same-set link needs no such knowledge and survives a set taking a new identity. It would also make `local` addressable through its per-machine identity. Linking into a set never held was *not* an objection: an identity belongs to the set, so knowing it is enough.

**No migration, and the reason is measured rather than assumed.** On 2026-09-14 every store on the one machine running Scatterhoard was queried read-only: none held a cross-set row carrying its own identity or naming its own current name. And every set holding records has an identity by construction — `create` minted before any write, `local` claimed one on its first write, import adopted before reconciling, and adoption displaced an identity only on a set with no records — so no link is ever stored before its set's identity is settled. *(Since 2026-09-28 the construction differs and the conclusion holds: every set mints at its first write, import adopts or mints before reconciling, and a held identity is never displaced — [Set Lifecycle](set-lifecycle.md#built-2026-09-28-create-makes-an-empty-set-and-its-first-write-settles-it).)*

#### Fixed 2026-09-15: the set a target lives in is part of a stored link's identity

**The `links` row key is `(source, label, target, target_set)`, and a unique index over `(source, label, target)` where `target_set IS NULL` keeps the same-set rows collapsed.** The key was the first three columns alone, where `target` is a memory id and nothing else. Ids repeat across sets by construction — a memory copied into another set keeps its id, which is what `MemoryFileError::DuplicateId` describes arising from a merge — so a memory linking with one label to that id in two different sets wrote two rows that collided, and the `INSERT OR REPLACE` in `write_links` kept only the second. `insert` returned `Ok` and the link was simply absent afterwards: `a-memory-linking-one-id-in-two-sets-keeps-one-link` in [Issues](../../TRACKING.md#issues), filed from the schema and confirmed by `two_links_to_one_id_in_different_sets_both_survive`.

**Widening the key alone would have traded one silent fault for another, which is why the index is half the fix rather than an optimisation.** SQLite holds NULLs in a key distinct from one another, and a same-set link is exactly a row whose `target_set` is NULL — so under the wider key two identical same-set links both insert, where the old key collapsed them. Measured directly before the change rather than reasoned about. The partial index covers precisely the rows the key stopped covering.

**Storing `''` for a same-set link instead of NULL was considered and declined.** It would need no partial index, but *null means same-set* is the idiom the resolution guards and `links_of` are written in — the same idiom the canonicalisation above exists to protect — and rewriting it everywhere buys nothing this does not.

**`compact_own_set_links` narrows with `UPDATE OR REPLACE`.** Once a set can hold both spellings of one same-set link, narrowing the long one onto the compact one violates the new index, and a plain `UPDATE` would fail the *open* rather than the write. The two rows say the same thing, so replacing is what the operation means. Before the rekey the case could not arise, because the narrower key had already merged them on the way in.

**The migration rebuilds the table, detected by the key's width** — three primary-key columns against four, read from `pragma_table_info`. Every row under the old key is unique under the new one, which is wider, so nothing is dropped or merged. `target_set_id` deliberately stays out of the key: two links to one id in two sets that happen to share a name is a case nothing produces, and including it would split rows that differ only by whether the author knew the target set's identity.

#### Built 2026-08-31: the target set's identity, where it is known

`set_id` names the target *set* by identity rather than by label, so a link survives that set being renamed and stops being ambiguous between two sets that share a name.

**It is optional permanently, not during a migration**, and the reason is structural. A link may name a set this machine has never held — the tool documents a dangling target as allowed and expected — so at authoring time there is frequently no identity to write, and there never will be. A link carrying only a name therefore resolves exactly as well as it did before identities existed. The stronger form the [set UUID](../../TRACKING.md#open-questions) reasoning describes, where the name is demoted to the status `target_keywords` already has, holds only for links whose target set the author holds; making it hold everywhere would mean refusing to author a link into a set this machine does not have, which trades away a documented affordance.

**The server writes it and the caller cannot.** A model writing a link knows an id and a set name; a set's identity is a fact the process holds and no caller could supply without being told it first, so putting it on the tool schema would ask for information the asker does not have — [Decision Placement](decision-placement.md). It is filled in at authoring only. Filling it in for a record *arriving from a file* would be a receiver-side rewrite, which is the act [absorption stopped being](multi-set-sync.md#absorption-becomes-a-supersession-which-removes-the-only-receiver-side-rewrite); pinned by `importing_does_not_add_an_identity_the_author_did_not_write`.

**Absent serialises to nothing at all**, not to `null`, and that is what keeps this off the checksum of every record that already holds a cross-set link — seven in the live `scatterhoard` set. Their bytes are unchanged, so none of them reconciles as modified and none needs rewriting: a compatible addition rather than a rewrite of everybody's records. `a_cross_set_link_without_an_identity_adds_nothing_to_the_line` is the guard.

### Dangling is not equally bad for every label

The relaxed stance above was set when links were undifferentiated. With a label vocabulary it needs to be per-label rather than blanket. A dangling `RELATED_TO` costs one association. A dangling `SUPERSEDES` means a memory cannot be told to be obsolete — and the failure is silent and asymmetric, because from the holder's point of view nothing is missing.

## Resolved 2026-08-23: no edge changes the standing of a memory in another set

**Every standing-changing edge whose target lives in a different set is reported, never acted on.** `SUPERSEDES`, `EXCEPTION_TO` and `CONTRADICTS` change their target's standing when traversal acts on them; across a set boundary, traversal does not act. Recall returns the target live, and says that a memory in some other set claims to supersede, except to, or contradict it.

*Built 2026-08-23, both halves.*

**Being walked is being acted on, and that took a third half — 2026-08-25.** Resolution ignored these edges as intended, and the *walk* did not: `EXCEPTION_TO` and `CONTRADICTS` are traversable, so a memory in another set that wrote one at ours sat one hop from it and inherited score accordingly, measured at 0.192 against a control of 0. Nothing about the target's standing changed, so the rule above read as satisfied — but the section below spends a paragraph refusing the claimant a permanent slot in every recall that matches its target, and a walkable edge is that slot bought with a different label. Such an edge is now excluded from the walk as well as from resolution, per `a-cross-set-dispute-carries-score-to-the-memory-making-it` in [Issues](../../TRACKING.md#issues). **The same edge written within one set is ordinary and keeps working**: what is excluded is the crossing, not the label.

Both halves are needed and only one of them protects anybody. **Refusing the write** stops an honest client authoring such an edge. **Ignoring it at resolution** is what actually protects the reader, because the file that carries a hostile edge was never written by our client in the first place. A refusal on its own would be security theatre.

### What a claim carries, and what it deliberately does not

A hit reports `disputed_by`: for each standing-changing edge in another loaded set that names it, the **claimant's set, its id, and the label it wrote**. Nothing else. The claiming memory is neither fetched nor ranked, so its content never reaches the reader unasked; an id and a set name are enough to go and read it on purpose with `hoard_lookup`, which is the only moment it is wanted.

`disputed_by` is a **separate field from `incoming`**, and that separation is the point. `incoming` is what a memory's own set says about it. Folding a stranger's claim into the same list would make the two indistinguishable, which is precisely the distinction a session needs in order to weigh one. It is omitted entirely when empty, because a dispute is unusual and a field present on every hit is a field readers learn to skip.

Reported on **lookup as well as recall**. Lookup is how a session reads a disputed memory on purpose, so telling it about a claim only when it stumbled on the memory by search would be the stranger of the two.

**Three things are outside a claim by construction.** A memory in the local set cannot be disputed, because `LinkTarget::CrossSet` carries a `SetName` and `Scope::Local` is not a named set, so no `target_set` addresses it — and, since 2026-09-28, no `set_id` does either: resolution matches an identity only among the sets a name could reach, which closed `a-link-can-reach-the-local-set-by-its-identity`. From `local`'s first identity on 2026-08-31 until then, a crafted file carrying that identity could make a shared memory concede to a personal one, or reach one in the walk. A claim from a local set is not reported either, because naming `local` as the claimant would name a different set on every machine. And a row inside a memory's own store that spells its own set name is not a claim at all — nothing crossed a boundary — which is a separate defect about spelling, filed as `a-same-set-link-spelled-with-its-set-name-is-inert`.

**What is reported is the edge, not the other memory.** [Recall Mechanisms](recall-mechanisms.md) settled this tradeoff for the neighbouring case: `CONTRADICTS` needed no override because the problem was *"visibility rather than promotion"*, and forcing a result slot was rejected because *"visibility costs one identifier per dispute where a forced result slot would not."* One asymmetry makes it sharper here. A memory always returned beside the one it attacks has bought a permanent slot in every recall that matches it — weaker than suppression and still a free channel into a stranger's context. Naming the edge gives a session everything it needs to judge, and lets it fetch the other memory only when the dispute bears on the work.

### What this overturns, and why the trust gradient did not survive

**This section previously resolved that an edge could not suppress a memory it did not outrank**, and set a gradient: a local memory *may* supersede a project memory, a project memory *may not* suppress a local one, on the grounds that [Embeddings and Vector Storage](embeddings.md) establishes project memories as arriving over an untrusted channel and local ones as not. That gradient is withdrawn. The reasoning behind it was sound and is preserved below, because what changed is not that it was wrong but that it answered a smaller question than the one that turned out to matter.

**It had no answer for project-versus-project**, which is the case that actually bites. `a-set-can-suppress-another-sets-memory` in [Issues](../../TRACKING.md#issues) is a `Corrupts` defect resting exactly there: two shared sets, neither outranking the other, and a hand-edited file in one removing a memory from the other for anyone holding both. Ids are published in every exported `memories.json`, so targeting is free. Extending the gradient to cover it would have meant ranking shared sets against each other, which is a judgement no software can make.

**The human partner's argument is what settles it**, 2026-08-21: *an LLM is capable of holding and evaluating two contradictory memories in context; software is not — so the right answer is to surface it to the LLM.* This is the fifth independent arrival at *report rather than resolve* in this design; see the same-named principle behind a reported supersession cycle, a reported model mismatch, a reported unresolvable link target, and a warned-rather-than-refused export scan. The boundary that keeps it from becoming *never refuse*: refusal is right where the caller holds the remedy and the mistake is theirs to fix, which is why the write still refuses a credential — and why it refuses this edge too, while declining to rely on that refusal.

**The gradient's remaining cost is one convenience, and it has never been used.** Local knowledge overriding stale imported knowledge now surfaces instead of suppressing: the local memory is stored and linked exactly as before, and recall returns the project memory with the claim attached rather than in place of it. Measured 2026-08-23 across the three live sets, there are ten `SUPERSEDES` edges and **every one is same-set**; the only cross-set edges in existence are three, all descriptive (`MOTIVATED_BY`, `RELATED_TO`). The capability being given up has no live instance.

**What the change buys is a single rule where there were three.** Under the gradient, resolution had to compare the scopes of both endpoints and answer differently in three cases, one of which was unspecified. Now it asks one question — *is the target in another set?* — and no scope comparison enters the resolution path at all. That is strictly less machinery than implementing the gradient correctly would have been, which is the unusual case of a security fix that also shrinks the design.

**One consequence for `SUPERSEDED_BY`, and it survives.** The label was introduced *because of* the gradient: it is how the lower-authority side records its own obsolescence without anyone reaching across the boundary. With no gradient there is no lower side, so that argument goes. Its second and independent argument is untouched and now carries it alone — supersession learned by *derivation*, from a `SUPERSEDES` edge on the other memory, requires that other memory's set to be loaded; without it the fact is not merely unreportable but unknown. An authored `SUPERSEDED_BY` is the memory's own data and travels with it, so it is the only form that survives the superseding memory being unavailable. **Note what it does not become:** an authored `SUPERSEDED_BY` naming a target in another set is still the memory's own claim about itself, made by its own author, so it is honoured. The rule above governs an edge reaching *outward* to change someone else's standing, not a memory conceding its own.

### Resolved 2026-08-25: a concession resolves when the set it names is loaded

**Honoured means resolved, not merely not-flagged.** A concession naming a set that is among those being searched resolves like any other: the replacement is returned in the conceding memory's place, and `matched` names the memory that yielded. Reporting it `superseded_beyond_this_set` instead told the reader to load a set they were already holding, and named no replacement — so the one remedy the flag exists to enable was the one thing already done. See `superseded-beyond-this-set-is-reported-when-the-set-is-loaded`.

The flag is now reserved for a replacement that genuinely cannot be produced: the named set is not loaded, or it is loaded and does not hold the id, or what it holds is itself out of reach. Those read alike to a reader, which is why they report alike.

**One question is asked at every surface that reports a standing**, since a concession honoured by semantic recall and disowned by keyword search or lookup would be two answers to one question — the shape [Recall Mechanisms](recall-mechanisms.md) records as `lookup-and-recall-disagree-on-shape`. Lookup stays exempt from *substitution*, which is a different rule and unchanged: asking by id means wanting that memory, so it comes back naming its replacement rather than replaced by it.

**A consequence worth naming rather than discovering.** A supersession chain contributes one candidate carrying the best evidence any of its members matched, so when a concession crosses a set boundary the conceding memory's similarity is what the replacement is ranked with. A set can therefore lift a memory in *another* set for queries of its own choosing, by conceding to it. This is deliberately not blocked. The claimant gains nothing — it removes itself from the results and none of its content reaches the reader — so the free-channel argument that governs `EXCEPTION_TO` and `CONTRADICTS` does not apply: there is no slot for the claimant to occupy. Blocking it would also break the ordinary case, which is that a memory's relevance should pass to whatever corrected it.

### The reasoning that was withdrawn, kept readable

*Preserved because a later reader is better served knowing the question was live and which way it was answered first. The rule below no longer holds.*

> Some labels only describe (`CAUSES`, `REFERS_TO`, `INSTANCE_OF`). Three change the standing of their target: `SUPERSEDES`, `EXCEPTION_TO`, and to a lesser degree `CONTRADICTS`. Once traversal acts on those — suppressing a superseded memory, flagging a contradicted one — the edge is an assertion of authority over another memory.
>
> In a single store that is unremarkable, because there is one trust level. Here there is not: [Embeddings and Vector Storage](embeddings.md) establishes that project memories arrive over an untrusted channel and local ones do not. So:
>
> - **A local memory may supersede a project memory.** Local knowledge overriding stale imported knowledge, with the effect staying local. `EXCEPTION_TO` is the gentler form of the same thing — it qualifies rather than suppresses.
> - **A project memory may not suppress a local one.** That inverts the trust gradient: an imported memory reaching into state it did not come from.
>
> This is not much of an attack today, since exploiting it would need a local memory's ID and those are not public. That is exactly why it is worth making structural. Relying on identifiers being unguessable is the kind of incidental protection this design has refused before — path-based trust in [Trigger and Loading](trigger-and-loading.md) exists because content inspection was the wrong place to enforce it.

**One line of it survives the withdrawal and is worth keeping in view**: relying on identifiers being unguessable is not protection. The new rule does not rely on it — it keys on the set boundary, which is visible at resolution time, rather than on a target's obscurity.

This remains a sharper form of the provenance-weighting question in [Open questions](../../TRACKING.md#open-questions), which asks about `Origin`/`Author` on *memories*. Edges turn out to need it more than memories do — and the answer edges got is that provenance decides whether an edge is *acted on*, which is a coarser and more robust use of it than weighting a score.
