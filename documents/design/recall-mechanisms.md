# Recall Mechanisms

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

## Semantic Similarity

The primary recall mechanism will be semantic similarity: generate a set of topic keys, then perform a nearest vector search against the memory content's embedding.

The search establishes a set of seed memories, probably using cosine similarity to select them.

Once the memory traversal system has used semantic search to locate a set of seed memories, it can traverse the graph from there, decreasing the memory's effective confidence level based on its native confidence and its distance from the seed.

The embedder, the storage of vectors, and the search strategy are covered in [Embeddings and Vector Storage](embeddings.md).

### Resolved: embedding inputs

Both the **why** text and the **keywords** are included in the embedding input used for semantic search, by default. Why: the "why" often surfaces related topics or records of paths not taken, which is exactly what semantic recall should be able to find; keywords are already curated for topical relevance, so including them is close to free given they're computed from the same content.

**Measured, not just reasoned.** An ablation over a 45-memory labelled corpus of real Scatterhoard design memories with 20 queries (`potion-base-32M`, raw cosine) put recall@5 at 57.5% for content alone, 65.8% for content+keywords, 66.7% for content+why, and 71.7% for content+why+keywords. Both additions help and they compose, so the decision holds on evidence rather than intuition.

## Already-known memories

A recall call can name memories the LLM already has in context: *"I already hold 1a2b3c4d, don't spend output on it."*

What this buys:

- **Session efficiency.** A memory already in context is pure cost if returned again — it spends context budget telling the LLM something it already knows.
- **Iterative refinement.** It lets a second call mean "something other than what you just gave me", which is otherwise impossible against a deterministic ranking.

### It suppresses output, not participation

The parameter removes a memory from the **results**. It does *not* remove it from seed selection or from graph traversal:

- **Still a seed.** A memory the LLM already holds and is now querying around is the *best available* seed — a known-relevant, already-validated anchor into the right neighbourhood. Traversing from it is exactly how "tell me what surrounds what I know" gets answered. Dropping it as a seed would discard the strongest entry point in the set.
- **Still traversable.** Its links remain valid connective tissue. Excluding it from the walk would disconnect whatever lies beyond it.

So this is a recall *improvement*, not merely a cost saving: the caller contributes free high-quality seeds by declaring what it already has.

### Why there is no separate "don't seed from this" control

Hubness (see [Embeddings and Vector Storage](embeddings.md)) might look like a reason to let the caller suppress bad seeds. It isn't, because hubness is a corpus-level statistical property of the whole vector distribution — an LLM cannot tell which memories are hubs, it only sees what came back. A control that requires information the caller does not have will be pulled wrongly or not at all. Hubness belongs in the ranking, not in the tool surface.

A memory that is genuinely *wrong* is likewise not a case for per-call filtering: it should be corrected, superseded, or forgotten, which is a different mechanism.

### Other design notes

- **A per-call parameter, not session state.** The server does not track what a session has seen; the caller passes what it holds. That keeps the tool surface simple (see [Tool Surface](tool-surface.md)) and avoids the server holding state it cannot reliably invalidate.
- Consistent with the design's general stance of trusting the LLM's judgement in the moment rather than legislating it in software.

## Resolved: inverse link pairs are canonicalised at read time, never on write

`CAUSES` and `RESULTS_FROM` describe the same relation from opposite ends, so both may be stored for one relation. Traversal must treat them as one edge or it double-counts, and under field-level merge two records of the same relation can drift apart independently.

The obvious fix — normalise to one direction when the link is written — **is not available here**. A memory in a set this machine does not own cannot be rewritten, so asserting the relation from whichever side is editable is often the only option. Normalising on write assumes ownership of both endpoints, which a multi-set system does not have.

So the reverse direction has to be **derived and held locally**. What matters is the guarantee, not the mechanism:

> Every stored edge is discoverable from both of its endpoints whenever both endpoints' sets are loaded, without either memory being rewritten in its exported form.

Whether that is a reverse adjacency index keyed by memory ID, or a back-link materialised onto the target's record in the local database, is an implementation decision. The rules below apply either way.

### When the reverse becomes available

The write side needs no case analysis: **a memory cannot be stored into a set that is not loaded**, so the forward edge is always durably written to a set this machine holds. Only the target varies.

- **Target in the same set** — the common case by a wide margin — or in another loaded set: the reverse is derived immediately, as part of the write.
- **Target in a known but unloaded set:** derived when that set is next loaded.
- **Target in a set this machine does not have:** not derivable. This is the bound on the guarantee above, and it is the same bound as a dangling cross-set reference (see [Cross-Set References](cross-set-references.md)).

**"At load time" is not sufficient on its own**, which is easy to get wrong. The MCP server is long-running and backed by a database; a write happens mid-session and there may be no load event for days. An LLM that records a supersession, then starts a fresh session with cleared context, must still find the superseded memory marked — nothing in the session lifecycle triggers a rebuild. Derivation is maintained on **every write**, not on an event.

### The reverse preserves the original label and direction

A derived reverse is presented as an **incoming edge carrying the label as written** — recall shows *"A supersedes this memory"* — rather than as a synthesised inverse link on the target (*"this memory `SUPERSEDED_BY` A"*).

Two reasons, the second decisive:

- Synthesising inverses would require names for directions the vocabulary deliberately does not have. [Memory Data Model](memory-data-model.md) settles fourteen labels precisely because the LLM only ever writes outbound; `MOTIVATES`, `SUPPORTED_BY`, `HAS_INSTANCE` and the rest were cut and should not return through the back door.
- It would **destroy a distinction the vocabulary exists to carry.** An authored `SUPERSEDED_BY` means *this memory's own author says it has been replaced*. A derived one would mean only *someone else asserted they replaced you*. Conflating them loses exactly the information that made the label worth having — and since 2026-08-23 that distinction decides whether the edge is acted on at all: [Cross-Set References](cross-set-references.md#resolved-2026-08-23-no-edge-changes-the-standing-of-a-memory-in-another-set) resolves that a cross-set edge cannot change its target's standing, so a memory conceding its own is honoured where someone else's claim over it is not. *(This bullet used to say the authored form meant "the lower-authority side acknowledges being replaced", which was the trust gradient that resolution withdrew. The distinction it names survived the gradient; only its justification changed.)*

So the authoring vocabulary and the reading vocabulary stay the same size, and derived edges stay distinguishable from authored ones.

### Maintenance rules

- **Incremental on write.** Safe, because the change is known exactly.
- **Fully recomputed on import, never incrementally patched.** A pull can *remove* a forward edge — someone else rewrote or forgot the superseding memory — and patching would leave a reverse pointing at an edge that no longer exists.
- **Never authoritative.** The derived direction must be reconstructible from the forward edges alone, so a corrupt or stale derivation is always recoverable by rebuilding.
- **Subject to the trust rule.** A project memory asserting `SUPERSEDES` against a local memory does not produce a reverse, because that edge does not get to suppress anything (see [Cross-Set References](cross-set-references.md)).

### Export must be structurally unable to carry it

If the reverse is materialised onto the memory's record, the export path must not emit it — and "must not emit it" as a filter is a step someone forgets, the same failure mode that argued for partitioned storage over a `WHERE scope =` clause. The exported type should simply not have the field, so omitting it is not an action anyone can skip.

### This deliberately crosses the scope partition

[Embeddings and Vector Storage](embeddings.md) partitions storage so that a query against one project's store cannot return another scope's memories. Reverse derivation crosses that on purpose: a local memory superseding a project memory means recalling the project memory should surface the local one.

That is safe because the partition is protecting **export**, and edges live on the source memory — so exporting a project set never carries an edge that lives on a local memory. The partition holds where it was needed; the derivation spans where recall needs it.

## Resolved: what recall does with a `SUPERSEDES` edge

Per-label traversal is otherwise an open question, but this one label could not wait for it, and for a reason that also explains why deciding it early prejudges nothing:

**Supersession is a validity relation, not a relevance relation.** Every other label affects how likely a memory is to be worth surfacing. This one affects whether it is still true — which has to be resolved *before* ranking rather than as part of it. It is a different question that happens to be carried by an edge.

It also could not wait because the whole purpose of recording a supersession is that a later session, holding none of the context that produced it, is told the memory is obsolete. An edge derived correctly but never surfaced is a correction that reaches nobody.

### Resolve to the tip; report the hit; spend nothing on the superseded memory

**Recall resolves supersession to the end of the chain and returns that memory in full**, together with the ID of the memory the query actually matched — and nothing else about it:

> Query matched C, which `B` supersedes, which `A` supersedes → return **A**, plus *"matched 019f…C, superseded"*.

Returning the intermediate `B` would surface a conclusion that was itself corrected. Returning nothing about `C` would leave the caller holding `A` for a query that matched `C`, with no idea why.

**The ID is the whole of the report — no gloss, no excerpt, no date.** In practice supersession most often corrects a memory whose recorded *conclusion* turned out to be wrong, rather than one that merely aged. Its substance is therefore actively misleading, not just stale, and surfacing any of it risks a corrected conclusion being read as live context. The ID is a handle for fetching if the history genuinely matters — *"why did we change this?"* is answered by the old memory — and costs one identifier.

**Chains need no further reporting, because they document themselves.** `SUPERSEDES` lives on the superseding memory, so `A`'s own link list already contains `SUPERSEDES B`; a caller can walk back one hop at a time through ordinary traversal. Listing intermediates would spend tokens on memories that were each wrong enough to be replaced.

### This is suppression of output, not of participation

Exactly as with [already-known memories](#already-known-memories):

- **Its in-links follow it; its out-links stay with it, since 2026-09-24.** A link *to* a superseded memory was written about the claim, and the walk follows it onto the tip. A link *from* one is that record's own claim, and a replacement that did not restate it has dropped it — the only way to drop a link once the memory holding it has been exported — so the walk does not carry it onto the tip. This reverses the earlier rule that a superseded memory's links remain connective tissue: that rule kept what lay beyond the old version reachable, and did it by making every replacement assert its predecessor's links whether its author meant them or not. `hoard_memorize` reports what a supersession leaves behind (`link_delta`), so the drop is seen when it is made. A superseded memory with no tip this search can reach stands as itself and keeps its links. `out-links-stay-with-the-record-that-wrote-them` in [Backlog](../../TRACKING.md#backlog); the rule is in `Neighbourhood::over` in `src/store.rs`.
- **Direct ID lookup is exempt from substitution.** Asking for a memory by ID means wanting *that* memory — it is returned, marked as superseded. Without this exemption the prior version becomes unreachable, which would make superseding equivalent to forgetting and defeat the decision to keep it. Forgetting already exists and is just deleting the line (see [Multi-Set Sync](multi-set-sync.md)); an author who superseded instead chose to keep the memory, and recall should not overrule that.
- **Keyword search is exempt too, since 2026-09-02**, and for the same reason as ID lookup rather than a second one. The rule above is justified on a superseded memory's *substance* being actively misleading. That is true of content and says nothing about an **index term**: a keyword is an author's addressing decision, so a replacement that drops one is that author saying *this is no longer about that*, and substituting under the old term overrides the statement instead of correcting it. The general form is **explicit addressing is honoured as written; inferred matching resolves to the tip** — which puts lookup and keyword search on one side and semantic search on the other under one principle, rather than as two special cases. Nothing is lost by it: if the replacement carries the term its own row matches directly, so the superseded row was always redundant. What is gained is the only way an author can retire a keyword at all, since amendment is refused after export and forgetting destroys the record — `a-keyword-cannot-be-retired-once-a-memory-has-been-exported` in [Issues](../../TRACKING.md#issues). **A concession across sets still substitutes**, because that is a memory's own statement about its standing rather than an index term being reached through, and `search_keywords_across` continues to honour it.
- **Supersession is one-to-many.** The convention for splitting a partly-stale memory produces several memories each superseding the original, so resolution returns every tip rather than "the replacement".

### Two ways supersession is learned, with different availability

This asymmetry decides what can be reported, and is the reason the date is not:

| how it is known | requires | if the other set is unloaded |
| --- | --- | --- |
| derived from `A SUPERSEDES B` | A's set loaded | **not known at all** — B is returned unmarked |
| authored `SUPERSEDED_BY` on B | nothing; it is B's own data | **B is returned, flagged** *superseded beyond this set* |

So the derived case always has both memories in hand, and the authored-unresolvable case has an ID and nothing more.

**Returned and flagged rather than withheld, since 2026-08-23.** Recall used to drop B outright, on the reasoning that handing back something known to be stale is worse than handing back nothing. That is the software deciding a case it cannot decide, and it fails in the ordinary direction: the replacement's set is usually merely unloaded rather than gone, so B was being withheld from the session most able to go and get it. The flag says *do not read this as current*; B's own `SUPERSEDED_BY` link carries the set name and the id, so **loading that set and fetching the target is the move the flag exists to make possible** — which is why the flag does not duplicate the set name it would then have to keep in step with. Pinned by `a_memory_replaced_from_a_set_this_machine_lacks_is_returned_and_flagged`, which reverses what that test asserted under its previous name. A date could only ever be reported opportunistically, and a field that is sometimes present teaches the reader to expect it and misleads when it is absent.

This is a second, independent argument for `SUPERSEDED_BY`, which was adopted on trust-direction grounds in [Cross-Set References](cross-set-references.md): it is also the only form that survives the superseding memory being unavailable.

**Resolution goes as far as the loaded sets allow.** If the tip's set is not loaded, the chain resolves to the furthest memory that is, and that one is returned.

**A chain that ends on an id nothing holds is the same case, and is reported the same way, since 2026-08-25.** A `SUPERSEDED_BY` naming an id in this set that no memory answers to used to resolve as though the chain had ended somewhere real: recall substituted the tip, could not fetch it, and dropped the memory from every search path without a word — `a-superseded-by-naming-an-absent-id-hides-its-memory` in [Issues](../../TRACKING.md#issues). The reader's situation is identical to the cross-set case above, so the flag is identical: *superseded, and the replacement is not available here*. It says less here, because the link carries an id and no set name, so there is nothing for the reader to go and load — what it buys is that the memory is returned at all.

**Where that question is asked is the load-bearing part.** It is asked once, at the point the chain stops, and it asks whether a **live memory** can be handed back. It is deliberately not asked at each step, because an id this store holds no record of can still be superseded in turn by one it does — a set lagging the repository it came from is exactly that shape, and refusing to walk would strand a reader whose replacement is in the result list. The same reasoning keeps a tombstone walkable: a chain whose middle was forgotten still holds a valid reference, per [Cross-Set References](cross-set-references.md), while a chain that *ends* on one has no replacement to hand back and is flagged.

### Resolved 2026-08-26: identity in the recall path is the set and the id, not the id

**An id names a memory inside one set and nowhere else.** Ids are unique within a file and the reader enforces it; nothing makes them unique *across* files, and nothing can, since two sets are authored by different people and every exported `memories.json` publishes its ids. Two loaded sets may therefore each hold one, in which case they are two memories with two contents and two scopes.

Recall used to key its population, its validity map, its walk and its candidate table on the id alone, which made them one identity **before any edge was consulted** — so one memory was dropped with nothing said, the survivor was whichever store happened to be scanned last, and a rival file's concession or cycle attached to our memory through the shared slot. That is `two-loaded-sets-sharing-one-id-collapse-to-one-memory` in [Issues](../../TRACKING.md#issues).

**This changed the ranking path and not the model**, which is why it was the right one of the three candidates considered. Everywhere else already treated identity as per-set: a duplicate id inside one file is refused when it is read, `hoard_lookup` collects a hit from every store, keyword search returns both, and a link is either same-set — resolved inside the store holding it — or carries the name of the set it points into. Semantic recall was the single place assuming otherwise.

Two consequences follow and both are structural rather than incidental:

- **An edge has to say where its far end is.** A walkable edge carries the set its target names, because an id cannot locate the far end once two sets may share one. Resolving it by searching every store for that id would attach the edge to a memory it never pointed at — the same mistake one layer down.
- **A supersession tip has to be located too.** A single store's [`Resolution`](../../src/store.rs) names its tips by id, which is right there, since a same-set tip is in the store that resolved it. Across stores a tip need not be: a concession to a loaded set resolves into that set. So the recall path carries the store with each tip, and the flat form is derived from the located one rather than the two being maintained separately.

**Collapsing true duplicates was considered and declined.** Keying on the id and a content checksum would let two sets holding a byte-identical memory come back once. The objection is the defect itself: a collapsed record must claim a scope, and when it stands for memories in two sets there is no honest one — choosing is store order deciding what the reader sees. Making it honest means a hit that names several scopes, with `matched`, `incoming` and `disputed_by` each saying which set they came from, which is a wire-shape change bought for a case with no live instance. Returning both, each with its own scope, is at least true. Whether identical results should be de-duplicated is a separate question and stays open.

### Cycles

`A` supersedes `B` and `B` supersedes `A` is a data error, and is handled in both places:

- **Rejected at write.** When both memories are in loaded sets the store can see the cycle forming, and refusing it there is cheaper than defending every use of validity resolution against non-termination.
- **Reported at read.** Not redundant with the above: write-time rejection only covers writes this store made. A hand-edit during a merge conflict introduces a cycle no write path ever saw, so recall has to defend itself against a store it did not fully author — the same posture it takes toward dangling references.

Resolution therefore carries cycle detection and a depth bound regardless, and reports the error rather than silently picking a winner.

## Resolved: the walk is uniform expansion, and ranking does the discriminating

Most of the difficulty in the per-label question came from asking one mechanism to do two jobs. It divides cleanly, and the two halves have different instruments.

**The walk generates candidates; the embedding judges them.** Expansion from the seeds is uniform and symmetric — every label crossed the same way, in both directions, decaying with distance and with nothing else. It makes no attempt to judge relevance, because the graph does not know relevance: it knows what some author thought worth connecting. Relevance is judged afterwards, by scoring each candidate against the query. That needs no information the store does not already hold, since every memory has a vector and so does the query.

### The walk is written once and called twice

**Built 2026-08-18** in `src/traversal.rs`, and shared: recall runs it over a store's links, and the [evaluation harness](../../TRACKING.md#backlog) runs it over a corpus held in memory. The harness exists to measure what ships, so the walk, the combination rule and the cut-off are one implementation rather than one per side — a harness measuring a walk the product does not run answers a question nobody asked.

### Uniform expansion is licensed by the re-scoring, not by itself

This is a dependency rather than a happy accident, and it constrains any later change. A walk that wanders out of the sense a query intended is harmless *because the cut re-scores everything*: think of "ice" alongside "computers", and a `RELATED_TO` edge into frozen water is picked up by the expansion and dropped by the ranker. Were ranking ever to run on graph distance alone, that same crude expansion would become a defect.

**The tolerance for a rough walk is bought by the ranker. It is not a property of the walk.**

### The inherited score must survive into the ranking, or the walk is redundant

If a candidate's final score were only its own similarity to the query, traversal would return nothing that a larger `k` would not return sooner and more cheaply — and it would penalise the very case it exists for. A memory is often relevant *because of its relation* rather than its wording: the measurement supporting a decision, the constraint that decision rests on. Those are precisely the memories whose content does not match the query. This is measured rather than supposed — a memory carrying a rare identifier ranked *below an unrelated memory* on content similarity ([Embeddings and Vector Storage](embeddings.md)).

So a reached memory carries a decayed share of its seed's score, as specified above, and the query-similarity term shapes the result on top of it. How the two combine is the open question below.

### Bounds may be set now; weights are held flat

This distinction is what makes the walk buildable ahead of the evidence:

- A **bound** — the expansion limit — is a safety property. Set too tight it is conservative rather than incorrect, and it can be defended with no corpus at all.
- A **weight** — the decay rate, any in-degree term, any per-label variation — is a tuning problem, and every weight here is currently untunable.

Every weight therefore starts **flat, and recorded as untuned rather than chosen**. A flat weight that is wrong costs ranking quality and announces itself as provisional; a weight guessed to three decimals reads as reasoned about and gets built upon.

### The bound is on expansion, not on inclusion

Reaching a heavily-cited memory is cheap, and it is often the most useful thing returned. Expanding *through* it is what branches. The limit therefore applies to using a memory as a frontier node, never to admitting it as a result — a cap on inclusion would discard good results to solve a problem they did not cause.

### In-degree: no caller control, and no ranking term yet

The caller gets no control over any of this, for the reason given about hub seeds in [Already-known memories](#why-there-is-no-separate-dont-seed-from-this-control): an LLM sees what came back and cannot see the corpus. **That argument settles placement only.** Whether the mechanism — which does see the whole graph, and knows the in-degree of every node it stands on — should read in-degree as a ranking signal is a separate question, and it is not answered here, because **the sign is label-dependent**. High inbound `MOTIVATED_BY` means load-bearing: many decisions rested on it. High inbound `INSTANCE_OF` means generic: it accumulated children by being unspecific. The same number pulls opposite ways, so it is a weight, and it is held flat with the rest.

This is *graph* hubness, and it is not the *vector* hubness measured in [Embeddings and Vector Storage](embeddings.md) and largely fixed there by centroid removal. They are different properties of possibly different memories, and neither ruling transfers to the other.

### Why starting simple is principled here rather than merely cheap

**Recall is advisory in a way supersession is not.** A mediocre walk returns a slightly worse candidate set, and the next query — with different wording, which both `recall-when` and `recall-before-storing` push for — recovers what it missed. A mistaken supersession serves a claim that is false. The two warrant different standards of proof, and traversal sits on the cheaper side of that line.

Published tuning for graph-augmented retrieval is not a shortcut past this. Nearly all of it concerns graphs derived from text automatically, where the edges are statistical; here every edge was authored deliberately by the writer of one of its endpoints. Constants would not transfer, and borrowing them would reintroduce exactly the unmeasured numbers this design has avoided elsewhere.

## Resolved: a reached memory's score is a blend, a quarter of it inherited

A candidate reached by the walk has two scores: the decayed share it inherits from its seed, and its own similarity to the query. Both are needed — the first is what makes traversal worth doing, the second is what makes uniform expansion safe.

**Decided 2026-08-18 by the human partner: `0.75 × own + 0.25 × inherited`, chosen on stability.** It matches what an unweighted sum reaches on both answer keys while moving about a point across the decays tested, against sum's fourteen to twenty-six, and it is the only rule measured that stays above the content-only baseline at *every* decay on *both* keys.

*This replaces a decision of 2026-08-15 that the scores are **summed**, taken on a measurement that has since been re-run: the walk it described let a seed inherit along a path back to itself, and correcting that changed which rule the corpus prefers. The history is kept below because the reasoning, not only the answer, turned out to be wrong in an instructive way.*

### The blend is a scaled sum, and saying so corrects why blends were rejected

`0.75·own + 0.25·inherited` orders identically to `own + (1/3)·inherited` — multiply through by four thirds, which is a common positive factor and moves no ranking. So the objection recorded below, that *"a weighted blend takes weight away from `own` for every memory, including the direct hits"*, **does not describe a real effect**: no memory is penalised, because scaling every score alike changes nothing.

What the weight actually sets is **how much inherited score is admitted at all**. The blend at `w = 0.75` measured worse than no traversal because it admits *three times* what a sum does, not because it takes anything from direct hits. Sum is the special case `w = 0.5`, admitting exactly one unit.

**One consequence is worth carrying: this weight and the walk's `decay` do overlapping work.** Both scale the inherited term, which is why a smaller weight is markedly less sensitive to the decay — the measured spread falls from fourteen points to one. That overlap is recorded rather than resolved; the design has two knobs where the arithmetic has closer to one and a half, and the honest reading of the stability result is that **the choice made was mostly about admitting less inherited score**, expressed as a weight rather than as a decay.

*What follows is the 2026-08-15 reasoning, kept as history.* Sum was held to have a property the alternatives lack: It only ever *adds*, so a memory that matched the query directly is never penalised for not having been reached — where a weighted blend takes weight away from `own` for every memory, including the direct hits, which is why the blend weighted toward the inherited term is the only rule measured to do worse than no traversal at all. And unlike a maximum, sum keeps both signals: a reached memory that also matches well is separated from one that barely matches, where a maximum collapses them the moment the inherited term is larger.

*The variation across `k` is read as an irregularity of this corpus rather than a property of the rule.*

This one question replaces most of what "per-label traversal behaviour" was asking, and it is tractable in a way that question was not: two terms rather than fourteen labels, and a shape that can be measured against a labelled corpus once one exists.

The seed's own decay rate is the accompanying weight, held flat per the rule above.

### Measured 2026-08-15: three functions, not four, and `sum` wins on both answer keys

*Evidence, not a decision — the choice is the human partner's, and this records what the corpus can now say about it.*

**Two of the four candidate shapes are the same function.** *A maximum* and *the inherited score acting as a floor* are the same arithmetic: a floor says the score is at least the inherited value, which is `max(own, inherited)`. And a blend at `w = 0.5` is a sum scaled by one half, which is the same ordering. So the four shapes are **max**, **sum**, and a one-parameter blend of which sum is the midpoint. A harness implementing all four reports a tie it built in.

Measured over the 24 harvested queries of [corpus/harvested-queries.json](../../corpus/harvested-queries.json), against two independent labellings, with a uniform symmetric walk of 5 seeds, 2 hops, decay 0.7 and a frontier of 20:

| rule | r@5 | r@10 | r@20 |
|---|---:|---:|---:|
| baseline, content only | 38.3 / 46.8% | 51.5 / 60.8% | 62.5 / 72.3% |
| max — also *floor* | +1.1 / +2.1 | +4.6 / +3.9 | +4.5 / +3.1 |
| **sum — also blend at 0.5** | **+2.2 / +3.7** | **+6.3 / +5.0** | **+6.6 / +2.9** |
| blend, `w` = 0.25 | +1.6 / +2.1 | +3.1 / +3.2 | +2.9 / +3.2 |
| blend, `w` = 0.75 | +0.1 / +0.2 | **−2.3 / −2.9** | +5.0 / +1.4 |

*(each cell is labeller A / labeller B)*

**Every rule beats content alone except a blend weighted heavily toward the inherited term, which is the only one that goes negative.** That is [uniform expansion is licensed by the re-scoring](#uniform-expansion-is-licensed-by-the-re-scoring-not-by-itself) appearing as a measurement: weight the walk too heavily and its wandering stops being harmless.

**Sum wins under both answer keys**, which is the result that matters most. The two labellings embody different policies — one sweeps a named topic, the other scopes to the named mechanism — and were run on different models. That they select the same rule says the policy difference has no consequence for this decision.

**Read the paired count before the means.** At r@10, sum beats the baseline on 10 of 24 queries under one labelling and 7 of 24 under the other, is unchanged on most of the rest, and loses on exactly one either way. A mean over 24 queries can be carried by two of them; this one is not, but neither is it broad.

**Three caveats travel with this.** The walk parameters are one configuration rather than a tuned optimum. The corpus is denser than live writing and its links follow document boundaries, so absolute lift is inflated — [the density caveat](../../TRACKING.md#backlog) applies. And 24 queries is a small sample, which is why the paired count is reported rather than a significance claim.

### Resolved 2026-08-16: the cut falls at the largest gap, not at a fixed count

**Superseded 2026-10-02 by [the section below](#resolved-2026-10-02-the-gap-cut-is-removed-and-limit-is-a-count-defaulting-to-eight)**: the gap was choosing among noise. Kept as the record of what was decided and why.

**Decided by the human partner.** Recall returns results down to the largest drop between consecutive scores, within a bounded window, rather than a fixed number of them.

**A gap is the only shape of cut-off this scoring admits**, which is what makes it defensible rather than merely convenient. Scores are centred cosines and freely negative, so a ratio to the top hit means nothing when the top hit is below zero. An absolute threshold is ruled out twice over — by `no-non-match-floor`, and by [the cold-start problem](../../TRACKING.md#open-questions), where a store with few memories has scores whose absolute values carry no information at all. A gap is a *difference*, so translation does not move it; and gaps are compared against each other rather than against a constant, so scale does not move it either. **It requires no number anyone would have to defend.**

**The clause about an absolute threshold no longer holds, corrected 2026-09-11.** `no-non-match-floor`'s ruling rested on a five-memory store, and the measurement's own recorded caveat said it could not support that conclusion; the cold-start objection is answered by calibrating per store from that store's own queries and setting no floor at all until there are enough of them. The gap cut stands on its own merits and is unaffected — it is simply no longer the only admissible shape of cut-off. See [the proposal below](#proposed-2026-09-11-the-floor-is-calibrated-from-a-local-query-log-and-reads-raw-similarity).

Measured over 24 queries against two independent answer keys, at the tuned walk: cutting at the largest gap reaches an F1 of 46.8% and 50.9%, against 51.0% and 49.8% for a fixed cut of ten — comparable, while returning **five to seven memories instead of ten**, which in this system is context spent. And it adapts, which is the point: cut sizes range from 2 to 28 with a median of 4 to 6, where the labelled answer keys themselves run from 1 memory to 36.

*Worth recording that the fixed default it replaces was also measured, and was wrong: `DEFAULT_LIMIT` is 5, and F1 for a fixed cut peaks at eight to ten on both keys.*

### Resolved 2026-10-02: the gap cut is removed, and `limit` is a count defaulting to eight

**Decided with the human partner.** Recall returns `limit` memories, best first, or fewer where the set's calibrated floor removes them; the default is **8**. A smaller limit returns the start of what a larger one would, so the answer no longer changes with the budget it was asked under — `result-count-follows-the-limit-not-the-query` in [Issues](../../TRACKING.md#issues).

**Why the cut came out.** The section above defended the gap as the only admissible *shape* of cut-off, and never established that the data has a gap to find. The 2026-09-08 probes found it did not: queries the corpus could not answer had breaks as sharp as the best real ones. Re-measured on 2026-10-02 at 1,330 and 181 memories, over every recorded real query against 100 off-domain probes, by `probe_whether_real_queries_break_and_whether_the_cut_follows_the_window` in [the probes](../../tests/probes.rs): on `scatterhoard` 29% of real rankings broke sharply against 17% of nonsense ones, and on `coding-lessons` 16% against 18%. A break that appears in a sixth of nonsense rankings cannot mark where an answer ends, so the cut was picking whichever small wiggle was largest inside the window — and widening the window let a different one win, on about one real query in six.

**Why eight.** In practice the cut returned three or four, so removing it with the old window of 20 as the default would have quintupled a typical response. The only measurement of result-count quality, above, put a fixed count's best F1 at eight to ten; eight is the predictable end of that range. `already_known` is what keeps a small default affordable, exactly as it was for the cut.

**The cut survives as instrumentation**, `Stages::gap_cut`, off in what ships, so a measurement can still ask what it would have done.

### Resolved 2026-10-02: `limit` has no ceiling, by decision, and a floor of one

**Decided with the human partner.** `limit` is honoured as given, however large; `recall-limit-is-unbounded` in [Issues](../../TRACKING.md#issues) is closed as not a defect rather than fixed.

- **The caller holds the budget the limit protects, and the server cannot see it.** Whether the client has 128K tokens or millions, and whether it wants three hundred summaries for a survey, are facts on the caller's side, so by [decision placement](decision-placement.md) the choice is the caller's. A model with a very large window may reasonably take most of a set into context and work from that. A ceiling in the server would override an explicit request the way the gap cut overrode the count.
- **Only an explicit request is large.** The default is eight, summary detail is a small fraction of a full record per memory, and `already_known` pages without repeats. Measured that day on the real `scatterhoard` set, `limit: 2000` on an on-topic query returned 309 memories and 667,196 characters in full — but only because it was asked for.
- **What the issue was filed against has gone.** In August a large limit returned the whole store with non-matches formatted as matches, since there was no floor, and the scan grew with the limit; every memory is scored on every recall now regardless, so the limit changes only what is sent.
- **A floor of one, refused by name.** `limit: 0` returned an empty list that reads exactly like *nothing matched*, so a caller's slip passed for an answer; it is refused, as are a negative number and a fraction, in one message naming the argument. The schema declares `minimum: 1` and says the limit has no ceiling.

**What would reopen it** is evidence of callers passing limits they did not mean — the human partner's candidate is a small local model adding a zero — rather than the possibility of it. A maximum with a cursor is the shape to reach for then.

### Proposed 2026-09-11: the floor is calibrated from a local query log, and reads raw similarity

Simulated over real queries in the order they were asked, measuring on the ones that came after: [`probe_simulate_a_floor_calibrated_from_labelled_queries`](../../tests/probes.rs), on `scatterhoard` (85 labelled queries, 796 memories) and `coding-lessons` (63, 129), against the 601 off-domain probes.

**The floor must read raw content similarity, not the centred score it reads today.** `Stages::floor` compares `hit.similarity`, which carries the corpus centroid removed — and centring is exactly what removes the direction that marked a query as out of domain, recorded under `an-unanswerable-query-returns-confident-hits`. Calibrated on everything each store had, raw silences **62.7%** of the probes on `scatterhoard` and **66.7%** on `coding-lessons`, against **2.7%** and **41.1%** centred. Worse, centred the admissible floor *sinks* as queries accumulate — 0.303 to 0.205 on `scatterhoard` — so its benefit decays toward nothing. Ranking stays centred, which is what centring was measured to improve; the scan carries both numbers rather than choosing between them.

**Calibration comes from the store's own log.** Every recall records, *before* the floor is applied, the raw top score for each store searched, the `scope` argument if one was given, the ids that would have been returned, and whether the floor silenced it. **No query text**: the score is what calibration needs, and text is the part a published file must never carry. The log lives in the store and export never reads it, so "never published" is structural rather than a rule to remember.

**What labels a query answerable** is what a session did with the result: it named this set as its `scope`, or it used a memory the recall returned — looked it up, linked to it, superseded it. Measured across 37 transcripts and 222 semantic recalls: 73% carry at least one signal, giving 87 labelled queries for `scatterhoard`, 63 for `coding-lessons`, 12 for `local` and 1 for `governance-docs`.

**The rule, and the count it needs.** The floor sits a margin below the lowest labelled score, and is set only once there are roughly **forty** labelled queries. Ten is not enough and the failure is not subtle: on `coding-lessons` a floor set from the first ten sits at 0.469 against a true minimum of 0.307, and silences **10 of the next 53** real queries. At forty, both stores lose nothing while silencing 41–67%. A margin of 0.05 below the minimum loses nothing on either store from thirty queries onward. Below the threshold there is no floor, which is the over-recalling end of the [bias toward over-recalling](../../TRACKING.md#issues) and therefore the safe one.

**A floor hides the evidence that would correct it**, which is the property that shapes the rest. Once it silences a query nothing comes back, so no memory can be used afterwards and that query can never earn the `used` label. Two things answer it:

- **The scope label survives**, because it is an argument of the call rather than a consequence of the answer. A scoped query scoring below the floor is a contradiction sitting in the log: on `coding-lessons`, 7 of the 10 queries a bad floor silenced were scoped, and the first of them is enough. So **the floor may never sit above the lowest scope-labelled score in the log**, lowering itself when contradicted; raising it takes an explicit recalibration.
- **Recall says when the floor silenced everything**, and the caller may ask again without it. That covers the residue the scope label cannot — the case silenced at thirty queries was labelled only by use — and it keeps the mechanism honest at the moment it costs something.

**A floor belongs to one store and one model.** The level is entirely the model's: every set's floor moved 0.10 to 0.16 under `potion-multilingual-128M`, and between a third and a half of the spread between sets moved with it. A model change clears the floor exactly as it clears vectors, and the store recalibrates from the log once enough queries have been scored under the new model.

**Open, and left open deliberately:** whether forty is the right count and 0.05 the right margin — both are read off two stores of one project — what bounds the log, and whether a query no session ever labelled is worth keeping in it at all. Tracked as `floor-calibration-source` in [Open questions](../../TRACKING.md#open-questions).

### Built 2026-09-17: calibration reports itself in `status`

**`hoard_sets status` carries each set's floor and how many labelled queries have arrived since its
last calibration**, plus a note offering `calibrate` once that count passes `LABELS_PER_CALIBRATION`.

Both halves exist because neither could be asked for without changing the answer: `calibrate` sets a
floor and `uncalibrate` removes one. Establishing that no set on this machine had a floor took three
direct database reads, and the readiness half is why the report is more than tidiness — on 2026-09-17
`scatterhoard` held 26 labelled queries and no floor, six past the threshold, with nothing having
said so. Calibration stays **explicit**, for the reason the section above gives: raising a floor is the
direction that loses answers. Offering it is what the absent report was costing.

The count is `Store::labels_since_calibration`, which reads the same watermark `calibrate` reads, so
the offer and the operation cannot disagree about what evidence exists.

### Built 2026-09-11: the query log, and nothing that reads it yet

The first piece of the proposal above. **Every semantic recall a session asks writes one row into each store it was scored against**, and nothing reads those rows — calibration cannot begin before there is something to calibrate from, and a log only ever answers questions about the period it was running. [`tests/query_log.rs`](../../tests/query_log.rs) is the contract; the tables are `query_log` and `query_log_hits`, created with the rest of the schema in [`store.rs`](../../src/store.rs).

A row holds when it was asked, the model that scored it, whether the caller named a `scope`, the **raw** top similarity, the **centred** top similarity, whether a floor silenced this store, and the ids the session received. **No query text**, which is the property that makes a copy of the file harmless, and export never reads the tables — it streams memories and tombstones, so the absence is structural rather than a rule anyone has to remember.

Five things were settled in the building, none of them stated by the proposal.

- **Only a session's question is logged, and the default is silence.** The probe suite puts several hundred deliberately off-domain queries through the shipping recall path in a single pass; recorded as traffic they would calibrate the floor against questions nobody asked. Those probes run against *copies* of the live stores, so nothing reaches the originals today — but that is a property of how the probes are written, not of the log. Only a caller that knows a session is at the other end can say a query is traffic, so `Question::in_a_session` says it and the MCP recall handler is the one caller. The cost runs the other way: a future shipping path that forgets logs nothing, and an under-collected log reports nothing about itself.
- **That decision is read from one binding, not tested twice.** It was written as two gates — one suppressing the extra scan, one suppressing the write — and a mutation flipping either alone changed nothing, so neither could be proved by any test. Two gates for one decision is two places for it to be decided; the surviving mutation is what exposed it, which is the case for recording a mutation against a guard rather than trusting it.
- **The raw score comes from a second pass over the same vectors**, taken only when a session asked. The scan the ranking uses is centred, and centring is exactly what removes the direction that marks a query as out of domain, so the two numbers cannot be derived from each other.
- **A store that scored nothing logs nothing.** An empty store is not a query that found nothing, and a zero written into the distribution the floor is read off would be evidence about the store rather than about the query.
- **The ids are what the session received; the scores are taken before the floor.** A memory the caller never saw could never be used, so logging it would label nothing — while a score taken after the floor would be the floor's own output, which is the shape of evidence that cannot correct it.

**The `used` label, built the same day.** A query is labelled answerable when the session that asked it went on to use one of the memories it returned, which arrives from ordinary work rather than from anyone labelling anything. Three signals, and they are exactly the ones `corpus/label_queries.py` counted when the forty-query threshold was measured: the caller **looks a memory up** by id or reaches it by fragment, **links to it** in something it writes, or **passes it back as `already_known`**. Each is a point where the caller names an id it already has, and each is one call site in the tool surface with a mutation against it, because losing one loses a signal silently — the log keeps filling and only the labelled count is quietly smaller.

**Amending and forgetting are deliberately not uses.** Correcting a memory you found is plainly using it, and counting it would change what a labelled query means without re-running the measurement that produced *about forty*. The threshold and the signals have to move together or neither means anything.

**The label is bounded to one session, and a session begins when the conventions are loaded.** Without the bound, any later use of a memory would label every query that ever returned it, the labelled set saturates, and the floor read off it sinks back toward the constant it replaced. The bound is also what the simulation used — *mentioned in a later call in the same session* — and matching it is the same requirement that keeps amending out of the signal list: the threshold of about forty was measured under these semantics and does not transfer to others.

**The server process was tried as the session and is measurably wrong.** On 2026-09-11 the servers on this machine had been running two hours before the current session's transcript was created, with two other sessions writing to their own transcripts in between — so a process spans several sessions, and the desktop client does not spawn one per conversation as the first version assumed. `hoard_conventions` is the marker instead: the catalog's own description says to call it once at the start of a session, and this project requires it before any visible text. The server rolls its identity when the first page is asked for, and not on a continuation, since paging is one load.

**That marker answers to a convention rather than to the protocol**, which is its one weakness and worth stating plainly. A session that never loads the conventions inherits the previous one's identity — the looser process-shaped bound, so a degradation rather than a break. A session that reloads them midway splits in two, losing labels rather than inventing them. Both leave the floor lower or later than it should be, which is the over-recalling end. A row is written unlabelled and can only be labelled afterwards, so a use cannot reach back for a query asked after it.

**Still unbuilt in this entry**, tracked as [`calibrated-non-match-floor`](../../TRACKING.md#backlog): the calibration action itself; the floor's storage per store and model; and the floor reading the raw score rather than the centred one. **Still open**: what bounds the log, and whether a query no session ever labelled is worth keeping in it.

### Revised 2026-09-12: a percentile over a bounded window, and only calibration writes the level

**The rule shipped that morning was a ratchet, and the human partner found it by asking how often recalibration would have to run.** `calibrate` took the *minimum* of the labelled scores over the whole log. A minimum over a growing set is monotonically non-increasing, so every recalibration returned the same level or a lower one, for ever; the only thing that raised a floor was a model change, which clears it. The level was therefore unable to follow a corpus whose scale is known to move — the same probes put the floor at 0.089 on an eight-memory set and 0.286 on a 429-memory one.

**And the decay was fast rather than gradual.** Recall wrote the floor down to any scope-labelled query that scored beneath it, on the spot. Scoping a recall to a set and finding it knows nothing is not an edge case, it is how you find out — so one such question pinned the level at that score permanently. The design had read a scoped query scoring low as *evidence the floor is too high*, which holds only if scoping implies the session expected an answer; often it implies only that the session knew which set to ask. The same weak signal was doing two jobs, as a label feeding calibration and as a contradiction writing the floor.

**What replaces it.** The level is the **5th percentile** of a **bounded window** of labelled queries, less the same 0.05 margin, recalculated when **20 new labels** have arrived since the last calibration — counted since, so no level is computed from evidence a previous one already spent. Recall never writes the floor: a contradicting scoped query is an ordinary label, and the next calibration reads it like any other. `hoard_sets uncalibrate` removes a floor outright, which is the remedy nothing else provides — the self-correction only answers to queries that *name* the set, so an unscoped search silenced by a bad level leaves no trace that would fix it.

**The window is bounded per kind, at 100 rows each.** Labelled rows are the calibration corpus, and bounding them is what lets a level rise, since old low scores have to be able to age out. An unlabelled row from a finished session can never become labelled — `note_used` is bounded to one session — so it is evidence about the labelling rate and nothing else, and is bounded on its own terms rather than competing for room with the rows calibration reads.

**Swept over both stores' real query history, walked forward, against the 601 off-domain probes** ([`probe_sweep_a_windowed_percentile_floor`](../../tests/probes.rs)). Moving from the minimum to the 5th percentile:

| | minimum | 5th percentile |
|---|---|---|
| `scatterhoard` | 0% of labelled queries lost, 39.8% of probes silenced | **1.5% lost, 74.4% silenced** |
| `coding-lessons` | 0% lost, 48.4% silenced | **7.0% lost, 83.2% silenced** |

Past 0.05 the cost climbs faster than the benefit on both. Recalibrating every 20 new labels loses 6.2% of the labelled queries that follow where every 40 loses 15.6%. The probe reproduces the earlier measurement where the two overlap — at the minimum with no margin it reports 62.7% and 66.2%, against the 62.7% and 66.7% recorded above.

**What the sweep could not settle, and what it gave up.** Both stores hold fewer labelled queries than the window, so 100 and unbounded are the same configuration there; and the simulated corpus is frozen at today's contents while the real one grew underneath those queries, so a rule that tracks growth is indistinguishable from one that does not. The window size is therefore *chosen* — a few weeks of traffic at the measured 19–20 labelled queries a week — and not measured. And the old rule lost nothing **by construction**, since the floor sat below every labelled query; a percentile deliberately silences a few. That is the tolerance that makes a noisy, frequent signal like `scope` usable at all, and it is knowingly given up rather than overlooked. The losses are recoverable: a silenced scoped query is still logged, still labelled, and feeds the next calibration, and `ignore_floor` answers in the moment.

**The `used` signal widened at the same time**, since every constant was being re-derived and no other moment is as cheap. `hoard_correct` now counts — both actions, wired at the dispatcher — which is the only instrument that can see the behaviour `recall-when`'s connect-on-recall clause asks for: measured across this project's transcripts, adding a link by amending an existing memory happened 15 times ever, against 322 links carried by new memories. And a **substituted hit records both ids**, the tip and the `replaces_matched` member, because the caller receives both and `hoard_lookup` is the documented way to read what was claimed before; roughly 10% of returned hits are substitutions, and a lookup of the older member had been counting for nothing.

### Built 2026-09-12: the floor itself

The rest of the proposal. **A store calibrates its own level from its own log, recall reads it against raw similarity, and a caller who is told the floor silenced everything may ask again without it.** [`tests/floor.rs`](../../tests/floor.rs) is the contract.

**Which floor is in force is now a choice with three answers, not a number that might be absent.** `Stages::floor` was `Option<f64>`, where `None` meant both *no floor* and *no floor set yet*; it is a `Floor` enum — `Calibrated` (what ships: each store's own level, or none where it has not calibrated one), `None` (the baseline every measurement of what a floor contributes is taken against), and `Fixed` (one level across the search, instrumentation only). Collapsing the first two would make the content-only baseline silently become *whatever the live stores happen to have calibrated*, which is the error the `Stages` type exists to prevent.

**One level per store, because a floor belongs to one store.** A multi-set recall asks each searched store for its own rather than applying a single number across the union — a level read off one corpus says nothing about another, and applying one across would silence a set that had never calibrated anything.

**Raw similarity is asked per hit, and of the memory whose score is being carried.** The log needed only the per-store top and took it from a second uncentred scan; filtering needs a number for each candidate, so `VectorSet::raw_similarity` answers one at a time — a floor only ever judges the handful of memories that reached the window. Where a supersession chain folded an older member's evidence onto its tip, the raw score is asked of the member that matched, so the two numbers describe the same text. A memory whose vector has gone missing is kept: failing open lands on the over-recalling end.

**The level lives in the store's `meta` and is cleared where the vectors are.** A floor's level is entirely the model's — every set's floor moved 0.10 to 0.16 under `potion-multilingual-128M` — so `record_model` clears it on an actual model change, and not on a rebuild under the same model, which displaces nothing.

*Superseded the same day by [the revision above](#revised-2026-09-12-a-percentile-over-a-bounded-window-and-only-calibration-writes-the-level): the level is a percentile of a bounded window, twenty new labels trigger it, and only calibration writes it — recall never lowers it. Kept as the first version's account.* **Calibration is explicit, and only ever raises.** `hoard_sets calibrate` sets the level `FLOOR_MARGIN` (0.05) below the lowest score any *labelled* query reached, and only once there are `LABELLED_QUERIES_FOR_A_FLOOR` (40) of them; below that it sets nothing and says how many it has. Both constants are measured rather than chosen, and both fail quietly if changed — a floor at the minimum silences the query that proved the set could answer, and one set from ten sits too high and silences real questions. Lowering needs no action at all: a scope-labelled query scoring below the floor contradicts it, and recall drops the floor to that score on the spot, which is why raising it is the act with a trace.

**And the caller is told, with the way round it.** Silence and *nothing matched* are the same empty result, so `Recall::silenced_by_floor` reports which it was and `hoard_recall`'s note names `ignore_floor: true`. The waiver is on the `Question` rather than in `Stages`, because it is a caller's choice and not one of the scorer's own settled decisions: a floor is a guess calibrated from past traffic, and the session asking is better placed than the store to know whether this particular question deserved an answer anyway.

**Added 2026-09-24: an answer that comes only from sets without a floor is marked.** When some searched set has a floor and every hit comes from sets that have none, `Recall::answered_only_without_a_floor` names those sets and `hoard_recall` says so in its note, with `scope` as the way to ask one set alone. It closes the case `an-unanswerable-query-returns-confident-hits` was filed on, which turned out not to be a calibration problem: on an eight-memory set the offside query scores at the level the set gives any question not about it, and calibrating the busy sets makes an unanswerable question fall through to whichever set has no floor. **Marked rather than withheld**, by the human partner's decision, because such a hit may still be a real answer — a bylaw question asked without a scope is answered by the set that has no floor. A waiver clears every floor, so it is never marked. `note-when-only-uncalibrated-sets-answer` in [Backlog](../../TRACKING.md#backlog).

**Added 2026-09-24: a set too little used to gather labels is calibrated from its own memories.** Twelve days after the floor shipped, the two sets in daily use had passed twenty labels and the two consulted rarely had one and four, and at that rate never would. So when `calibrate` finds too few new labels and the set has no floor read from labels, it asks each current claim's index line as a question against the rest of the set, leaving the memory itself out, and sets the floor at the same 5th percentile less 0.05. `probe_a_floor_without_labelled_queries` measured it against the two sets that have real floors: 0.372 against 0.360 and 0.332 against 0.333, silencing 1 of 78 real labelled queries. The alternative the discussion favoured first, a floor above every unscoped query a set was scored against and never answered, would have silenced a third of them, because a question about a neighbouring subject scores like an answerable one. **Labels outrank it both ways**: a floor from labels is never replaced by one from memories, and one from memories gives way as soon as twenty labels arrive — which they can under a floor, since a scoped query is labelled even when silenced. **It needs twenty current claims**, the same count labels need; at eight the percentile is the weakest memory, and on `governance-docs` it placed no useful floor. The source is stored beside the level and reported by `calibrate` and `status`, because an inferred floor is weaker evidence than an observed one, and `status` offers the backup to a set that could take it. `calibration-from-self-queries` in [Backlog](../../TRACKING.md#backlog).

**What is still open** is unchanged and now testable against real traffic: whether forty and 0.05 generalise past two stores of one project, what bounds the log, and whether a query no session ever labelled is worth keeping in it. One more, deferred at the proposal and still deferred: whether a walk-reached memory whose origin cleared the floor should survive a floor its own content similarity fails — today it does not, because the floor reads content similarity alone.

### Resolved 2026-08-18: a walk origin never inherits, and the measurement moved with it

The build found two consequences of summing **centred** scores that the decision did not state. One is a note; the other was a defect this build introduced, and the human partner had it fixed the same day rather than filed.

**A memory could inherit a decayed copy of its own score.** Expansion is symmetric and the walk takes several hops, so a path out of a seed and back along the same edge returned to where it started: at three hops with decay 0.9, a seed adjacent to anything got back 0.81 of its own score and summed it in. That is not a relation to anything — it promotes a memory for having any edge at all, independently of what the edge points at, which is [in-degree entering the ranking](#in-degree-no-caller-control-and-no-ranking-term-yet) by a route this design closes elsewhere.

**The rule now is that a walk origin inherits nothing**: not along a path back to itself, and not from another seed, since both already carry their own similarity and the walk exists to reach what content similarity did **not**. A memory reached by several paths still keeps the best of them, an improvement still re-enters the frontier so a shortcut found after the long way round propagates onward, and each memory takes one frontier slot per hop — so the bound on expansion is never spent on a duplicate.

**An inherited share can still be negative, and then the sum subtracts.** Scores are centred cosines, so a seed that matched poorly carries a negative score outward and a memory reached from it scores *below* its own similarity. The design's reason for preferring sum — *"sum only ever adds, so a memory that matched the query directly is never penalised"* — is exactly true of an **unreached** memory, which keeps its own score untouched. It is not true of a reached one.

#### What the correction did to the evidence

**Re-measured the same day, on the same corpus, the same 24 harvested queries and both blind answer keys: `sum` now loses where it used to win.** At the harness's tuning walk, against the content-only baseline:

| rule | r@5 | r@10 | r@20 |
|---|---:|---:|---:|
| max — also *floor* | +1.1 / +2.1 | +3.9 / +3.9 | +4.5 / +3.1 |
| **sum** | **−6.9 / −10.4** | **−1.0 / −5.2** | +6.6 / +2.9 |
| blend, `w` = 0.25 | +3.7 / +2.2 | +3.1 / +3.2 | +2.9 / +3.2 |

*(each cell is labeller A / labeller B; sum previously stood at +2.2 / +6.3 / +6.6 on the first key)*

**The reading is that self-inheritance was amplifying content similarity rather than reaching anything.** Every direct hit with a traversable edge got 0.81 of its own score back; links correlate with relevance in this corpus; so the artifact sharpened the top of the ranking and flattered precision at small `k`. Remove it and sum's inherited term has to compete with direct hits, which it displaces — paying only at r@20, where every rule still beats the baseline. **That the walk earns its cost is not in question; how its score should meet the query's is.**

**The parameters move with the rule, and the two cannot be read apart.** Swept again over the same 480 configurations, the top of the range is now at **decay 0.5** on both answer keys — 57.7% r@10 on the first and 67.2% on the second, against baselines of 51.5% and 60.8% — where before it was 0.9. Hops are flat from two upward and seeds flat from three to five, as they were.

**That is the caveat the table above needs**, and it is what stops this from being a verdict. The rules were compared at **one** walk, decay 0.7, which is now nobody's optimum: `sum` at its own best decay reaches 57.7% and 67.2% at r@10, which is *better* than `max` and the light blend managed at 0.7 (55.4% and 64.7%). What the table honestly shows is that **sum is the rule most sensitive to the decay**, which follows from what it does — it is the only one whose inherited term can displace a direct hit, so how fast that term fades is exactly what decides whether it helps. Comparing the rules each at its own best walk needs the harness to take those parameters from its caller, which it does not yet.

What holds under every reading: **at r@10 and r@20, with a decay it can live with, the walk earns its cost on both keys.** What is unsettled is the pairing of rule and decay. The shipped constants are left where they are pending that decision, because tuning them alone would settle half a pair. Recorded as [an open question](../../TRACKING.md#open-questions).

### Measured 2026-08-18: which rule is *flat*, which is what stability means here

The re-measurement above compared rules at one walk. The harness now crosses rule with decay
([`eval-walk-from-the-caller`](../../TRACKING.md#backlog)), because a rule that wins the cell it was tuned in
and collapses beside it is a worse choice than one that is merely good everywhere — **the corpus a
rule is tuned on is never the corpus it runs on.** Recall@10, five seeds, two hops, frontier 20:

| rule | best (A / B) | spread across decays 0.3–0.9 (A / B) |
|---|---:|---:|
| baseline, content only | 51.5 / 60.8% | — |
| max — also *floor* | **55.3 / 64.8%** | 3.9 / 6.3 |
| sum — also blend at 0.5 | 54.9 / 64.1% | **14.3 / 25.6** |
| **blend, `w` = 0.25** | 54.9 / 64.1% | **1.4 / 1.0** |
| blend, `w` = 0.75 | 54.1 / 60.2% | 25.8 / 35.4 |
| rank fusion, `k` = 10 | 49.2 / 53.8% | 4.1 / 4.8 |

**The light blend matches sum's ceiling on both keys with a tenth to a twentieth of its movement**,
and it is the only rule in the table that stays above the content-only baseline at *every* decay
tested on *both* answer keys. Sum dips below the baseline at 0.9; so does max, on the second key.
What the blend really trades is one sensitive constant for one insensitive one: with a spread near a
single point, the decay stops mattering much, which is a better place for the project's remaining
untuned weight to sit.

**Rank fusion is stable and not good.** Combining the two *rankings* by position is immune to the
scale and the packing of the scores by construction, and it measured flat — and well below the
baseline, because equal-weight fusion gives the walk as much say as content similarity, which is
roughly the `w = 0.5` setting the blend already shows to be too much. Making it competitive would
mean weighting the two lists, at which point it is a rank-space blend and the score-space one is
cheaper and better understood. *Recorded so the option is closed with a reason rather than left
looking promising.*

### What the corpus's own scores look like, and why it matters

Measured over the same 24 queries: **mean −0.0022, deviation 0.1063, gap below the leader 0.0560.**

Two things follow. The mean is near zero, so the own-scores are very nearly a pure scaling of their
own z-scores — which means **standardising them could not change any ranking**, and a "normalised
sum" is a rule this corpus cannot distinguish from plain sum. And the gap below the leader is about
half a standard deviation, so the head of a typical ranking here is **packed**: an inherited term has
little distance to clear before it starts reordering the top, which is exactly the sensitivity `sum`
has and `max` bounds.

### What the walk does at a set boundary

**Decided 2026-08-16.** Unscoped recall — the default — already searches every loaded set, and every store in a query is centred on the mean of their union. A cross-set link is therefore an edge inside the searched population, and the walk **crosses it and continues normally**: same decay, same hop count, no penalty for the crossing. A cross-set edge decaying faster would be a per-kind weight, and every weight here is held flat.

**A scoped recall does not walk out of its scope, and the reason is arithmetic rather than contractual.** If the query searched one set, the centroid was computed over that set alone. A memory from an unsearched set has no own-similarity on that scale — centring it against a baseline it is not part of produces a number, and the number means nothing. Summing that with an inherited score adds two quantities measured against different corpora, which is exactly the incomparability [the union centroid](embeddings.md#resolved-stores-searched-together-are-centred-on-the-mean-of-their-union) exists to prevent. The rule that forbids raw cosine across stores forbids this too, one layer down.

**What the caller gets instead is the link.** A returned memory carries its links, and a cross-set link carries the target's set, id and keywords — which exist *"so a reference that goes dangling has something to be re-found by"*. The model can look the target up or widen the scope. That is the same *surface it and let the LLM decide* posture the design takes toward a dangling reference.

### Keyword search does not traverse

**Decided 2026-08-16**, settling the *"maybe also to disable graph traversal during the search"* left open under [Pure Keyword Search](#pure-keyword-search).

Exact match returns a **set with no scores**, so there is no seed score to inherit and decay — the mechanism has nothing to operate on. And the reason the keyword path exists is to surface only memories tagged in a specific way, which traversal would dilute.

**Nothing is lost, because the links still ship.** A keyword hit returns the same shape as any other memory, links included, so a caller who wants to follow one can. The design's standing preference for surfacing over deciding applies unchanged.

### Keyword search across sets is one ranked list

**Decided 2026-08-19**, from a defect rather than from the design: `hoard_recall`'s keyword path
asked each loaded store for `limit` hits and concatenated them, so the response held up to `limit ×
loaded sets` and arrived in store order. See `keyword-recall-multiplies-the-limit-by-the-number-of-sets`
in [Issues](../../TRACKING.md#issues).

**The rule is the one the semantic path already follows**: merge every searched set, rank the union,
then cut once. What differs is the scale it ranks on. Semantic recall cannot compare raw scores
across stores at all — that is what [the union centroid](embeddings.md#resolved-stores-searched-together-are-centred-on-the-mean-of-their-union)
exists to fix. A **match count needs no such correction**: it counts the caller's own keywords, so it
means the same thing in every set, and merging is sound without any centring. The one place this
document's incomparability rule does not bite.

**Equal counts break the tie on the memory id, ascending.** Ties are the common case rather than the
exception — most keyword queries are one keyword, so most hits score 1 — and the alternative is not a
neutral order but whichever set the server happened to iterate first, which no caller can see and
which changes with the sets loaded. Ids are UUIDv7, so ascending is oldest first. That is arbitrary
in the sense that no property of recall recommends it, and deliberate in the sense that the *choice*
had to be made explicitly rather than inherited from a map.

**Since 2026-09-29 it is the id of the memory that *matched***, which after a cross-set substitution
is the conceding memory's rather than the replacement's. The count already stayed with the memory
that matched; keying the tie on it too is what let retrieval become a merge that stops, below. The
order differs from before only where a substituted hit ties with others.

**Retrieved as a merge that stops, not a union that is cut** — also 2026-09-29, extending
`keyword-recall-ignores-already-known` at the human partner's request. Each store is read by a cursor
in rank order, a bounded batch of ids at a time (`KEYWORD_BATCH`), continuing each batch from the last
`(matches, id)` it read; superseded memories and the caller's `already_known` are skipped before a
record is loaded. The merge takes the best head across stores one hit at a time — substituting,
de-duplicating and leaving out held ids as it goes — and stops at `limit`; claims are computed only for
what it returns. What it replaced read every matching id in every store, loaded up to `limit` records
from each, and computed claims for all of them before cutting. The per-store cut the old shape argued
for is no longer needed: nothing is read that the merge will not take, save one batch of ids.

### Native confidence is not in the decay, and the reason is about the evidence

The opening of this document describes the walk as decreasing a memory's effective confidence *"based on its native confidence and its distance from the seed."* **Only distance is implemented, and confidence is deliberately absent rather than forgotten.**

It is left out because **nothing available can tune it**. The evaluation corpus was authored with confidence values spanning 0.8 to 0.95, and the live sets measured for [confidence standards](conventions.md#served-confidence-standards) held five distinct values between 0.8 and 1.0 across 44 memories. A term whose entire observed range is the top fifth of its scale cannot be distinguished from a constant by any corpus this project currently has, so including it would be guessing a second weight immediately after taking the trouble to measure the first.

**That is a statement about the evidence, not about the field.** Confidence may well belong in the decay; what is missing is data with enough range to say. Revisit with operating experience, once stored confidence values vary more than the test corpus and the early live sets do.

**A precondition attaches to that revisit, found 2026-08-18.** Recall resolves validity *after* it ranks, so a returned tip carries the score computed for the memory it replaced. A confidence term added while that holds would rank a correction on the **superseded** memory's confidence — and supersession is typically a guess replaced by a measurement, so the error would run one way: corrections ranked on the low confidence of the claims they corrected. **Resolve-before-rank first, confidence second** — the same shape as escape-before-refusal. Filed as `validity-is-resolved-after-the-ranking` in [Issues](../../TRACKING.md#issues).

### Resolved 2026-08-18: the collapsed hit does not arise, but not for the reason recorded

*A loose end recorded here since 2026-08-14: validity resolution can collapse two hits into one when a memory and its superseder both rank, freeing a result slot, and whether that slot is backfilled was called part of the same ranking decision.*

The reading offered here on 2026-08-16 rested on two claims, and **the build found one of them false.** It was recorded as a reading to be confirmed against the implementation, with the note that if the code turned out to rank before resolving it would come straight back. It does rank first, so this is that check reporting back.

- **"Supersession is resolved before ranking" is not what the code does.** Every memory with a vector is scored and sorted, and *then* the top candidates are resolved in score order, a tip inheriting the score of whichever superseded memory matched. Resolution runs before the **cut**, not before the ranking.
- **"A gap cut has no quota a collapse could leave short" is true, and it is sufficient on its own.** Two entries that collapse leave a shorter list, and the cut is taken over the list that survives collapsing. There is no slot to backfill because nothing was reserved.

So the conclusion stands and its stated reason did not. What makes it hold is the second half alone: **a cut with no quota cannot be left short.** The first half was doing no work, which is worth recording, because it would have kept looking like the reason.

**The false half is not harmless, and the human partner found what it costs the same day.** Because resolution runs late, the walk's seeds are chosen from unresolved ids — so a supersession chain, whose members resemble each other by construction, can spend several of the five seed slots on memories that collapse to one, and a superseded memory acts as a conduit for inherited score while its tip cannot reach the same neighbours. That is `validity-is-resolved-after-the-ranking` in [Issues](../../TRACKING.md#issues), and it is what makes the ordering worth fixing rather than merely worth documenting.

One consequence of resolving after ranking is worth stating rather than leaving to be rediscovered. A tip is returned at the score of the superseded memory that matched — the tip's own similarity is never computed on that query — and where both rank, the better-scoring of the two is the one kept. That is the same substitution [the resolve-to-the-tip rule](#resolve-to-the-tip-report-the-hit-spend-nothing-on-the-superseded-memory) already describes, and the response names it: a substituted hit reports `matched_similarity` rather than `similarity`, so the number is never presented as describing the content beside it.

## Resolved: `CONTRADICTS` needs no override, and inbound edges must ship by default

The one candidate exception to uniform expansion was `CONTRADICTS`: should a contradiction force both memories into the results, on the grounds that a contradiction the model does not see is worse than no link? **No — and the question was aimed at the wrong layer.**

### The problem was visibility, not promotion

`CONTRADICTS` carries what neither correction instrument can express. [Corrections](corrections.md) routes a claim that *replaces* another to supersession, so what is left for `CONTRADICTS` is the case where both memories stand and conflict — nobody concedes, and nothing resolves.

Under the outbound rule the edge is written on the newer memory. So the **older** memory — the one likelier to have gone stale, and the one recall may well surface on its own merits — carries no trace of the dispute at all. Its only record of being contradicted is an inbound edge.

Make that edge visible and the label needs nothing from traversal. The contradiction arrives labelled, in both directions, and the caller decides whether it bears on the work in hand. Promotion into the results would be a *relevance* instrument aimed at a *validity* problem, and it would decide for the caller something the caller is better placed to judge: **a disputed memory far from the centre of a search is a dispute worth ignoring, and no ranking rule can tell that case from a serious one.** Whether a contradiction matters depends on what the caller is doing, which is the test in [Decision Placement](decision-placement.md).

It also scales the right way. Contradiction is rare in the present corpus but must not be assumed rare in general: supersession requires somebody to concede, and across sets — the workflow the [Vision](../Scatterhoard_Vision.md) is built around — two sets may disagree with neither side wrong and neither able to rewrite the other, leaving `CONTRADICTS` as the only instrument. Visibility costs one identifier per dispute and scales with that. A forced result slot each would not.

### Inbound edges ship by default, and an option may not invert that

This resolution depends entirely on the caller seeing inbound edges, which today it does not — `incoming-edges-invisible-on-the-wire` in [Issues](../../TRACKING.md#issues). Fixing that is a **precondition** of this decision rather than a related improvement.

**The default is settled by the same argument that opened the question.** An edge that ships only when asked for is invisible to a caller that does not know to ask, which rebuilds the trap exactly: a contradiction nobody sees. A caller may be offered a way to turn inbound edges *off* — a caller that suppresses them has decided it does not want them and knows what it gave up. Default-off is a different thing, and is not available.

This is a constraint on the single detail-level projection rather than a second field beside it, per that issue's fix condition.

### Label and direction stay distinct, even with expansion uniform

Uniform expansion means direction does not change how an edge is *walked*. It still changes what an edge *means* when reported, because [the reverse preserves the original label and direction](#the-reverse-preserves-the-original-label-and-direction): an edge is visible from both endpoints under the name it was written with, and no inverse label is synthesised. Reciprocity is carried by direction, not by a second label.

- **`CONTRADICTS` is symmetric.** Inbound and outbound assert the same thing, which is why one rule covers both.
- **`INSTANCE_OF` is not.** Outbound is instance → pattern: one hop, and it generalises. Inbound is pattern → instances: it enumerates, and it is the fan-out case below. One label, two operations sharing nothing but a name.
- **`SUPERSEDES` is the worked precedent.** Validity resolution already reads the two directions as answering different questions — an inbound `SUPERSEDES` means something replaced this memory, while an outbound `SUPERSEDED_BY` is the memory's own acknowledgement, and only the latter survives the replacement's set being unavailable.

### Fan-out is a property of the inbound direction

`INSTANCE_OF` walked **inbound** from a general pattern reaches every instance across every loaded set. The asymmetry behind it is durable rather than incidental:

> A memory's **outbound** degree is fixed when it is written, and bounded by how much its author chose to link. Its **inbound** degree is written by everyone who cites it afterwards, and grows with the corpus.

Outbound traversal is self-limiting; inbound traversal is not. This is what the expansion bound above exists to contain, and why that bound is an inbound concern.

### What the corpus can currently support

Measured 2026-08-14 over the 36 live memories of the `scatterhoard` set. Unlike the [link-label corpus](../../corpus/link-label-findings.md), these were written through the tool in the course of other work rather than in a pass whose purpose was labelling: **16 edges carrying four distinct labels** — `SUPERSEDES` 7, `RELATED_TO` 7, `RESULTS_FROM` 1, `RESOLVES` 1. `MOTIVATED_BY`, `SUPPORTS`, `CONTRADICTS` and `INSTANCE_OF` have no instances at all. Counting both directions, 11 of the 36 memories are isolated and the highest incident degree is 3.

**Measured again 2026-08-15 against a purpose-built corpus, and the per-label question is now answerable.** The evaluation corpus in [corpus/traversal-corpus.jsonl](../../corpus/traversal-corpus.jsonl) extracts all twenty-one of these design documents — 429 memories, 433 traversable edges over eight labels — and the harness ([src/eval.rs](../../src/eval.rs)) ranks each link's target from its source against the real embedder. **Nearly half the edges, 214 of 433, have a target a top-5 cut would miss**, which is the population traversal exists to reach, counted rather than inferred.

The per-label medians differ by more than an order of magnitude, and sensibly:

| label | n | median percentile |
|---|---:|---:|
| `MOTIVATED_BY` | 6 | 0.23% |
| `EXCEPTION_TO` | 34 | 0.70% |
| `RESULTS_FROM` | 124 | 0.93% |
| `DEPENDS_ON` | 71 | 1.17% |
| `SUPPORTS` / `RELATED_TO` | 127 / 33 | 1.63% |
| `INSTANCE_OF` | 35 | **7.46%** |

**`INSTANCE_OF` targets sit roughly thirty times further out than `MOTIVATED_BY` ones.** A decision and its motivation are about the same thing and share vocabulary; a principle and a case of it are worded completely differently. That is the same ordering a 56-memory pilot found at a factor of seven, so it is one finding measured twice at very different scales rather than two.

**This is evidence for the open question below rather than an answer to it.** It says a uniform decay applied across labels is averaging over a real difference; it does not say what the per-label weights should be, and [the rule that weights start flat](#bounds-may-be-set-now-weights-are-held-flat) is unchanged until queries exist. **The corpus also carries two biases that pull opposite ways** — it is denser than live writing (1.05 edges per memory against 0.44) and its links are *textually closer* (a 1.17% median against 2.9% across the live sets) — so it flatters traversal on how many edges there are and understates it on how hard they are to reach.

**Counting outbound links alone puts isolation at 21 of 36, and that is the wrong number.** It is an easy one to produce, because export carries each edge once, on the memory that authored it — so a count taken from the exported file sees only half of every relation. Any measurement of graph shape must count both directions. The tool surface makes this same error structurally, which is the defect above.

## Resolved: the reduced detail level is one derived line, and the caller asks for it

*Built 2026-09-06.* `hoard_recall` takes `detail`, which is `full` — the whole record, and the
default — or `summary`, which returns the memory's id and one line derived from its `content`,
capped at 120 characters. The projection is [`MemoryView::to_json_at`](../../src/mcp.rs) and the cut
is [`MemoryText::summary_as_data`](../../src/memory/text.rs); `recall_in_summary_detail.json` in
`tests/snapshots` records what a caller receives.

**It is a read-side projection, not a watch feature**, although
[`watch-push-is-an-index-per-set`](../../TRACKING.md#backlog) is what it was built for. The measurements
behind the cap are in that entry and are not repeated here. A summary over a *semantic* recall is a
survey — a broad question at a large `limit`, thirty lines for the cost of six records, then
`hoard_lookup` for the few worth reading — and that use is the argument for the level being general
rather than reserved to the watch recall.

**The choice belongs to the caller**, by [Decision Placement](decision-placement.md): only the
caller knows whether it is surveying or reading, and the answer changes the response by a factor of
nearly five. **The default stays `full`**, which is the constraint
[the inbound-edge resolution](#inbound-edges-ship-by-default-and-an-option-may-not-invert-that)
already set on this projection: a reduced level may be offered and may not become the default,
because a caller that has not chosen cannot know what it gave up.

**The line is derived, not authored, and that is what removes a failure mode rather than guarding
it.** A rule asking an author to keep a memory short enough to read many of requires arithmetic over
a serialized envelope the author never sees. Under a derived line an over-long memory costs the same
index line as a short one, no refusal is needed, and the author-facing rule becomes one a person can
follow without counting: *make the opening say what this is for and when it applies.*

**Two cutting rules, and the second was decided during the build.** A sentence end inside the cap is
where the line ends, and a terminator counts only when whitespace follows it, so `0.5` and
`store.rs` are not sentence ends. Where several sentences end inside the cap it takes the **last**
rather than the first: taking the first spends most of the budget whenever a memory opens with a
short sentence, and truncates the line to four characters on an `e.g.` — silently, since a short
line looks like a short memory. With no sentence end at all the line ends at the last whole word,
and a single word longer than the whole line is cut where the room runs out, because an empty line
names nothing.

**A cut line ends in an ellipsis, and a whole one does not.** Decided 2026-09-06 by the human
partner, after the level shipped without a marker. The marker answers the question a reader
deciding whether to fetch is actually asking — *is there more memory beyond this line* — so it is on
every cut rather than only the ragged ones: a sentence cut reads as a finished thought whether or
not three more claims follow it, and marking only the mid-word cuts would leave an unmarked line
meaning either *complete* or *cut at a sentence*, which moves the ambiguity rather than closing it.
It is one character rather than three dots, and it comes **out of** the 120 rather than being added
past them: a cap the marker can push over stops binding on exactly the lines that are already at it.

**A summary hit carries the id and the line and nothing else**, the scope included. The envelope the
cap was measured against is 49 characters, so a scope, a score or a match count would each spend
about a quarter of the saving — and under the per-set recall this projection was built for, the
caller already knows the set it asked. What is missing is one `hoard_lookup` away, which is the
trade an index makes. **The failure mode is incompleteness, not inversion**: measured over all 192
live memories of both shared sets, none produced a line asserting the opposite of its memory.

**The store response echoes the same line, always.** `hoard_memorize` returns `summary` beside the
id. A character cap is invisible at drafting time to a model as much as to a person, so showing the
derived line at the moment of writing is what keeps the cap from repeating the mistake it exists to
fix. It echoes rather than refuses — a weak opening is craft, not damage — and it is unconditional
rather than keyed to a keyword, which would couple the response to `hoard-watch` and confine the
reflection to the one write where the author is already careful.

**`hoard_correct`'s `amend` echoes it too, when the amendment moved `content`.** Added 2026-09-06 by
the human partner, from the field's actual first use: *making sure there is a coherent description
in the first 120 characters, which might require a few tries.* That is a loop rather than a
reflection — store, read the line, rewrite the opening, read it again — and `hoard_memorize` covers
only the first turn of it. Amend is the right instrument for the rest rather than a second thought
about it: the loop runs before export, which is exactly the window amend is open in, and after
export the same echo arrives through `hoard_memorize` because supersession is a store. **Scoped to
`content` rather than unconditional**, unlike the store echo, on the rule the in-link remark beside
it already follows — the line is derived from `content` alone, so echoing after a confidence change
would report something that did not happen and teach a caller to skip the field on the writes where
it did.

**One surface deliberately unchanged.** `hoard_lookup` takes no detail level: it fetches one memory
by id, which is the case where the caller has already decided it wants the record.

## Pure Keyword Search

The conventions may establish specific ways of tagging certain information, such as for gathering information on, and investigating, a bug across multiple merges, or tagging work associated with a specific branch. It would be useful to be able to generate seed memories purely through keyword search, and maybe also to disable graph traversal during the search in order to surface only memories tagged in that specific way.

## Direct Reference

Memories have globally unique IDs. It should be possible to search by them.

*Superseded 2026-08-20 — the observation stands, the remedy was backwards.* The original text read:

> Some LLM models have a habit of truncating a UUID when recording a reference to it, and this approach is common in conversations about `git` commits. If the memories use UUIDs as identifiers, the lookup tools for direct reference should also support prefix-based searches.

**The observation was right and is the root of the whole problem; accommodating it was the mistake.** Supporting prefixes made the habit *work* — silently and wrongly, since a UUIDv7 prefix constrains time rather than identity — which is `lookup-prefix-selectivity-is-time-correlated` in [Issues](../../TRACKING.md#issues). The design met a bad instinct by building it a path.

**Decided 2026-08-20 by the human partner, in two parts.** The identifier changes so the instinct is not provoked — Crockford base32, no separators, no hex alphabet, per [Memory Data Model](memory-data-model.md). And **prefix lookup goes away**: `hoard_lookup` takes a whole id, and a partial one is refused rather than resolved. Recovery is a **separate, explicitly selected substring search**, not a fallback the tool reaches for on its own.

**Substring rather than prefix, because the head is the one place worth not cutting.** Measured over the live sets, a six-character window is unique at every offset except the first two — so a fragment from anywhere but the front identifies a memory, and the front is exactly what a prefix takes.

**What this buys is not the end of truncation but the end of silent truncation.** Nothing stops a caller shortening an id to fit a column. What changes is that a shortened id handed back as an id now *fails* instead of resolving to a plausible wrong memory — a loud error in place of an answer indistinguishable from a correct one. That is [STANDARDS.md](../../STANDARDS.md)'s preference for making an error impossible to express over detecting it at run time, applied one level up: the error is still expressible, but it can no longer succeed.

When the recovery search returns more than one memory, its message should say that fragments from the **end** of an id are far more selective — the head-anchored fragment is the one users will reach for first.
