# Embeddings and Vector Storage

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

How memory content becomes vectors, where those vectors live, and how they are searched. The recall behaviour built on top of this is in [Recall Mechanisms](recall-mechanisms.md).

## Resolved: embeddings are local derived state, never exported

**Vectors are computed locally and never appear in the exported memory file.** The export carries content, why, keywords, links, and metadata; an importing machine computes its own vectors from that content.

This follows from decisions already made:

- [File Format](file-format.md) exports in a consistent order specifically to keep merge diffs small. Several hundred floats per memory, on the memory's own line, would defeat that completely.
- The field-level [merge driver](multi-set-sync.md) would be reconciling vector noise that has no meaningful merge semantics — two branches that both re-embedded the same memory produce trivially different floats with no correct resolution.
- Changing the embedding model invalidates every stored vector. If vectors never leave the machine, that invalidation is *local*: one re-embed pass, affecting nobody else.

That last point is the important one. It converts the choice of embedding model from a one-way, ecosystem-wide commitment into a locally reversible decision. A project does not have to agree with its contributors about which embedder they run.

### Consequences

- **A local embedding cache keyed by content hash.** Re-import then only re-embeds memories whose content actually changed, rather than the whole set.
- **The model's identity and version are local state**, recorded alongside the cache — not in the exported file, which has no reason to know. A mismatch between the recorded model and the configured one triggers a re-embed rather than an error.
- **Import cost is a real design constraint.** Re-embedding on import is on the critical path of a normal `git pull`, which argues for a fast embedder (see below).

## Resolved: one store, one model, and no mixture

**Every vector in a store is produced by that store's recorded model, or the memory has none.** A store never holds vectors from two models at once, and changing the model resets the vector of every memory embedded with a different one — or with none.

The rule that forces this is already stated one layer up, for stores searched *together*: several stores answering one query must be embedded by the same model, because `recall_across` embeds the query once and scores that vector against all of them, and *"two different models would make those comparisons meaningless with nothing failing to say so."* That reasoning does not become weaker inside a single store. A store holding vectors from two models is that same defect at a smaller radius, and it is worse there, because the union check has at least somewhere to notice — within one store there is no seam to inspect.

The practical form of it is confusion no interface can clear up. Two memories come back ranked against each other with scores that were never comparable, and every downstream number — the centroid, the ordering, any future floor — is computed across a mixture. Nothing about the result looks wrong.

### The states a store can be in

The distinction that matters is not "has vectors" versus "has none." It is whether this process is equipped to maintain what the store already holds:

| Recorded model | Loaded model | Reads | Writes that change the embedded text |
| --- | --- | --- | --- |
| none, **and no vectors** | none | keyword, id, traversal; recall says semantic search is unavailable | **allowed** — nothing to lose, and keyword search is the search |
| none, **but vectors present** | *M′* | keyword, id, traversal | the vectors are **rebuilt** with *M′* — see below |
| none, **but vectors present** | none | keyword, id, traversal | allowed, **except** a write that would destroy a vector this process cannot rebuild |
| *M* | *M* | everything | allowed; any memory lacking a vector for *M* is embedded |
| *M* | *M′* | everything, once the sweep below has run | allowed — the mismatch **resolves itself** by re-embedding the store with *M′* |
| *M* | none | keyword, id, traversal — all still correct | allowed, **except** a write that would destroy a vector this process cannot rebuild |

The last two rows are the ones the code has never distinguished from the first, and they are not degraded configurations but broken ones: the store holds vectors this process cannot compare against. Reads stay available in every row regardless, because keyword search, direct lookup and traversal never needed a model and are not made wrong by one being absent. The rule below that *a missing model degrades loudly, never silently* is about the **first** row, where the whole store is consistently unembedded — it is not a licence to keep writing over vectors in the last.

**No recorded model is two rows, not one**, added 2026-08-09 after `embedded-store-can-report-itself-unembedded`. A store that has genuinely never embedded anything and a store whose vectors predate the identity being written down are indistinguishable by the recorded model alone, and only the first has nothing to lose — the second had 26 vectors while claiming exactly that. **The vector count is what separates them.** Every store written before the identity was recorded is in the second state, so it is a migration state rather than a corruption, and an unsafe one: a mismatch cannot be detected against a model nobody wrote down, which is how two servers came to run semantic recall against vectors from a third.

Those vectors are **rebuilt rather than adopted**. Adopting means writing the loaded model's identity over vectors it did not produce — a guess made permanent and unverifiable, and one that leaves the store looking `Current` afterwards. Rebuilding costs a re-embed, which at roughly 18,000 memories/sec is sub-second for any realistic store, and yields a provenance that is a fact. With no model in hand there is nothing to rebuild *with*, so the vectors stay and the existing protection applies: discarding them there would throw away the only copies in existence to settle a bookkeeping question.

**A wrong model and no model are the same state and want opposite treatment**, which is why they are two rows. With *M′* in hand there is something to rebuild with, so the store re-embeds itself and the mismatch is gone; refusing there would be refusing the very thing the design already specifies for a model change. With nothing in hand there is no way back, and the write has to be stopped instead.

**And what is stopped is narrower than "writes".** A new memory in a store whose model is missing destroys nothing — it arrives unembedded, says so, and is repaired by the sweep below when a model returns. The write that cannot be allowed is one that **replaces a memory that currently has a vector**, because rewriting a memory discards its vector and nothing here can produce a new one. That is the whole of `amending-without-a-model-destroys-the-embedding`, and stating the rule as *do not destroy what you cannot rebuild* is what keeps a store usable while its model is missing instead of read-only.

### Proposed 2026-08-09: gap-filling stays automatic, mismatch re-embedding becomes explicit

**Raised by evidence, not argument.** The first real two-server run (`two-servers-can-disagree-about-the-model` in [Issues](../../TRACKING.md#issues)) showed the *M*/*M′* row above producing an unbounded loop: two servers with different models each re-embedded the whole store on the other's turn, indefinitely, and a `hoard_recall` was enough to trigger it.

**The reasoning below is sound and rests on a premise that run falsified.** *"Neither the LLM nor the human holds information that would justify leaving a store half-embedded"* is true of a single decider. With two servers, **the other server's configuration is exactly that information**, and neither server holds it.

The section below also treats gap-filling and mismatch re-embedding as *"the same mechanism rather than a second one."* They diverge in precisely the case it anticipates. Its own safety argument — that concurrent sweeps *"converge on identical rows: duplicated work rather than a race"* — holds for **gaps**, where both servers compute the same missing vectors, and fails for **mismatches**, where each server's rebuild invalidates the other's. Convergence versus oscillation is the difference, and it is not visible from either process.

So the proposed split:

- **Gap-filling stays automatic.** It converges, and decision placement applies unchanged.
- **Mismatch re-embedding becomes an explicit action.** It oscillates, and the information needed to choose — which model should win — lives with the human who configured both servers.

The *M*/*M′* row changes accordingly: reads keep keyword, id and traversal, and **recall refuses the semantic path rather than rebuilding**, because the alternative measured on 2026-08-09 was a confidently-scored ranking computed across two incompatible embedding spaces, indistinguishable from a correct one.

**A refusal needs a way out, so it comes with one**, in three parts — all needed, since any two leave the LLM stuck:

1. **`status` reports the model state honestly**, naming recorded and loaded identity. Today it returns `semantic_search_available: true` on a mismatched store, and two servers holding different models reported byte-identical status.
2. **The recall refusal names the remedy**, the pattern `project-stores-get-no-embedder` established.
3. **`hoard_sets` gains a re-embed action** that rebuilds every vector with the model in hand, states which model that is, and reports how many were rebuilt.

Cost is not the objection: re-embedding runs at roughly 18,000 memories/sec, so a full rebuild of a 10,000-memory set is sub-second. What the explicit step buys is that a rebuild happens because someone chose it.

**Two servers then refuse rather than fight** — a visible standoff instead of an invisible loop, with `status` naming the conflict. That does not make disagreeing servers work, and is not meant to: it makes them stop and say why, and the resolution is configuration.

#### An explicit action moves the oscillation rather than ending it

Raised by the human partner immediately after the above, and it holds. Once the remedy is documented, the loop reappears one level up: `_b` refuses, its LLM runs the re-embed, `_a` refuses, its LLM runs the re-embed. **Each is behaving correctly and locally** — it met a documented failure and applied the documented fix — and nothing in either one's view separates *recovering from a stale model* from *taking a turn in a tug-of-war*.

That is worse than the code-level loop in one specific way: each participant's local experience is **success**. The pathology exists only in aggregate, which is the case where the actor cannot be relied on to notice it.

**Encode the signal as identity, not as elapsed time.** A timestamp asks the reader to infer contention from a duration — a weak signal, and one an LLM reasons about poorly, since the same four minutes is alarming in a tug-of-war and unremarkable otherwise. Instead the store keeps a **bounded history of the model identities it has held**, and the re-embed action tests the condition that actually matters: *is the requested model one that was already displaced?* That is a cycle, and cycle detection is exact — the same shape as the `Cycles` handling in [Recall Mechanisms](recall-mechanisms.md), which is precedent rather than a new idea. The refusal then states both models and the alternation count and says a configuration conflict needs fixing, which is actionable without any sense of time.

**It warns; it does not refuse.** Raised by the human partner, and it settles a question the paragraph above had left in the wrong place. The re-embed **always proceeds**, carrying the warning with it.

- **The harms are asymmetric.** What the check protects is *vectors* — derived state, rebuilt at roughly 18,000 memories/sec, so even a large store is a sub-second rebuild. What a wrong refusal costs is semantic recall itself. Blocking the capability to protect cheap rebuildable state is the wrong way round.
- **The lockout is concrete, not hypothetical.** Decommission the second server and try to restore recall with the model that remains: the history says that model was already displaced, so a re-entry check refuses it, while the recorded model is one nothing on the machine can supply. Semantic recall is dead and the documented remedy is the thing being refused. **A store must not be reachable into a state its own tool surface cannot repair.**
- **Refusing does not produce a working configuration**, only a stopped one. Where two servers genuinely hold different models there is no good state available either way, so the only question is which failure surfaces the conflict faster — and a warning that names it does that without cornering anyone.

**What carries the weight is that the message is directive rather than descriptive.** *"3rd alternation between two models"* is a fact the reader may or may not act on. *"Two servers are configured with different models; this will keep recurring until that is fixed — raise it with the human rather than re-running this"* says what to do, and is available in a warning exactly as much as in a refusal.

**This removes the override entirely**, along with the question nobody could answer about it: an LLM meeting a refusal that names an override is fairly likely to simply use it, and specifying an acknowledgement it would not pass reflexively was the least settled part of this design. With nothing to refuse there is nothing to override.

Still true: **the first re-embed is unremarkable.** The signal is not *the model changed* — the ordinary upgrade path — but a **return to a model already displaced**. Only re-entry is evidence, and it escalates with the count.

**`recall` refusing the semantic path is a different case and stays a refusal.** It is not a lockout: keyword search, id lookup and traversal keep working, and the remedy is available and now unblocked. Its job is to withhold a number computed across two embedding spaces, which a warning cannot do — the number would still be there, and something would still rank on it.

The timestamp keeps a place, recorded beside the history — not as the trigger, but as what a **human** uses to diagnose once a warning has told them where to look.

### Decided 2026-08-20: a store owns its model, and a mismatch is reported rather than repaired

**The store's recorded model becomes prescriptive.** Until now `meta.embedding_model` described what the last writer used, which is why it could never disagree with a server for long — the server reconciled the store to itself, and two servers reconciled it to each other forever. The record now states a requirement: *these vectors were produced by M, and only M may extend or replace them.* A server that holds a different model does not re-embed; it reports.

**A model is a capability; authority is separate from it.** The recorded identity is a content hash of the weights plus how this process encodes text with them — see [The identity covers the encoding, not only the weights](#the-identity-covers-the-encoding-not-only-the-weights) — and is not a locator, so a store knowing it needs a particular model still cannot obtain one — the bytes always come from the process. What moves to the store is the *authority* to say which model its vectors require. The server either can supply it or cannot, and that is the whole of the negotiation.

#### The identity covers the encoding, not only the weights

**Decided 2026-09-19.** The same weights fed the same memory two ways produce two different vectors, so the weights hash alone cannot say whether this process can maintain what a store already holds. The recorded identity is therefore `<weights hash>+<encoding>`, weights first so a human comparing two servers sees at once whether the model differs or only the encoding does.

What made this live is that the encoding changed. `model2vec-rs`'s `encode` defaults to `max_length = 512`, which is a default rather than a limit the model imposes — a static model mean-pools token vectors and has no context window to overrun — and what it silently removed was the *end* of the embedding input, which is a memory's `why` and then its keywords. By 2026-09-19 that reached 12.5% of the corpus, up from 4.7% ten days earlier, with the ninetieth-percentile memory past the cut. Scatterhoard now encodes the whole text.

**Changing the encoding without changing the identity would have been the earlier defect wearing a disguise.** `no-record-of-which-model-built-the-vectors` is about vectors whose provenance nothing records; here the provenance would have been recorded and wrong, with the weights hash agreeing on both sides while the vectors were built two different ways. Every store written before this reports `Mismatched` on first contact with a server that encodes whole, and the way out is the one the write door already names: `hoard_sets` action `reembed`.

**What this rests on is a claim about use, made by the human partner.** Multi-model operation is rare and deliberate: most users take the model [SETUP.md](../../SETUP.md) names, and a power user tuning a different one does so per corpus. So a mismatch is an **event to be reported**, not a steady state to be engineered around — and the design's job is to protect against a collision rather than to make disagreeing servers work.

#### Why this is smaller than the proposal above, rather than a variation on it

[The 2026-08-09 proposal](#proposed-2026-08-09-gap-filling-stays-automatic-mismatch-re-embedding-becomes-explicit) assumed a mismatch is **a state to be driven out**. From that, everything else follows: somebody must re-embed, so two somebodies take turns, so the design needs a bounded history of displaced model identities and cycle detection to damp the oscillation.

Treating a mismatch as **a configuration to be reported** removes the premise. Nobody has to act, so there is no move to take turns with, and the oscillation has no mechanism rather than a damped one. The identity history and the directive warning remain worth having as *evidence* — they tell a human where to look — but they stop being what prevents the loop.

That is the test this section is offered on: it should make the earlier machinery optional rather than wrong. The reasoning above is what makes this advantage legible, which is why it stays.

#### The rules

- **Reads never rebuild.** A recall that meets a store whose model it cannot supply leaves that store alone. `recall_across` already excludes such a store from the scan, which keeps the ranking sound — every score compared comes from one vector space.
- **The exclusion must be named.** A recall reports which scopes it left out and why, distinguishably from a recall where nothing could be searched at all. Under per-store ownership this is ordinary operation rather than a fault, so a response that merely flips a boolean is not enough; see `semantic-search-available-conflates-partial-with-none` in [Issues](../../TRACKING.md#issues).
- **Changing the model is an explicit command**, and rebuilding every vector is its side effect rather than a separate step. The information needed to choose which model should win lives with the human who configured the servers, which is where [decision placement](decision-placement.md) puts it.
- **Requesting the model the store already holds is allowed, and is a reset.** It rebuilds every vector from current content. It reads oddly — nothing appears to change — and it is nonetheless the right action in two cases: vectors of unknown provenance, which are **rebuilt rather than adopted** for the reasons already recorded above, and any doubt about vector integrity. Allowing it also removes a special case: the command's contract is *after this, every vector in this store was produced by the named model*, which is true whether or not the name changed.
- **If there are no vectors, there is no model.** Decided 2026-08-20 by the human partner, and it is the same thought as the rest of this section reaching the data: the recorded identity is a property *of the vectors*, so a store holding none is bound to nothing. The binding is released wherever vectors can be emptied and again at open, which repairs a store already left otherwise. It removes a state that was reachable and meaningless — `import` on a modelless server deleted every vector, no-opped its re-embed, and left the store reporting a mismatch against nothing while refusing to fill its own gap.
- **A write by a server that cannot supply the store's model leaves a gap, not a mixture.** The memory is stored unembedded and stays that way until a process holding the store's model fills it. Gap-filling remains automatic because it converges, but it fills gaps *with the store's own model* — never with whatever the writing process happened to have. A gap is an absence and costs coverage; a mixture is two incomparable spaces in one store and costs correctness.

#### What does not change

**One store, one model, no mixture** is unchanged — this is how it comes to be enforced rather than merely stated. **Vectors remain local derived state, never exported.** And **do not destroy what you cannot rebuild** is orthogonal to ownership and still required: a process with no model must not rewrite a memory that currently has a vector, because rewriting discards it and nothing present can produce another. That is a missing *capability*, not a model collision, and no amount of authority in the store prevents it.

### Filling the gap is automatic, not an action someone has to remember

Where a store has memories with no vector for its current model, they are embedded — no request required. This is the same mechanism as the model-mismatch re-embed in *Consequences* above rather than a second one, and it is automatic for the same reason: **there is no decision here for anyone to make.** Under [Decision Placement](decision-placement.md), a choice whose answer is always "yes, make them consistent" should not be a choice. Neither the LLM nor the human holds information that would justify leaving a store half-embedded.

This is not a rare repair. It is the ordinary onboarding path: the model arrives through a delegated setup step (below), so *store memories first, gain a model later* is what a normal installation does, and without this those memories would be permanently unfindable by meaning.

Two constraints it has to respect:

- **The embedder must not land on the critical path of opening a store.** That is why vectors are cached in memory at all. So the *gap* has to be cheap to detect even when there is none, which is what keying the cache by input hash buys: with each memory carrying the hash of its own embedded text, "how many memories have no vector for this model" is one join, and the common answer of zero costs one query and no model.
- **It has to be idempotent**, because two servers can share a store ([Store Registry](store-registry.md)) and both can notice the same gap. Keying by `(input hash, model)` makes concurrent sweeps converge on identical rows: duplicated work rather than a race.

### Decided 2026-09-22: the sweep runs before a search by meaning, not on a write

**Only a search by meaning reads vectors, so that is the one place the sweep runs** — before each store is scanned, and from `catch_up_vectors` for a caller that needs the vectors first. Until 2026-09-22 it also ran at the start of every write, as half of the `prepare` step the write path shared with recall. A write gained nothing from it: it embeds its own memory in the same call, keyword search and `hoard_lookup` never read a vector, and filling *other* memories' gaps mattered only to the next search, which fills them anyway. Nor did a write need `prepare`'s other half, the cache refresh: everything a write consults is read from the database, the cache is only inserted into, and the next search reloads it if another connection committed in between. A two-connection test writes through a deliberately stale cache to hold that (`a_write_through_a_stale_cache_leaves_every_memory_findable`). Raised by the human partner as `where-the-gap-sweep-runs` in [Open questions](../../TRACKING.md#open-questions). **Cost was not the reason**: the sweep's query measured 0.2 ms over 1,001 memories. The reason is that a write should do its own work, and the comment justifying the call had been stale since the mismatch stopped resolving itself on 2026-08-09.

**The same change made "the common answer of zero" true.** Identity records are never embedded, so they have no row, and the sweep's query found one on every call, loaded it, and discarded it. It now excludes them by the same structural test `Memory::is_identity_record` uses, a record attributed to itself, in the one query `awaiting_vectors` shares — which had filtered them separately, a second definition of a gap.

**The broader reservation is unchanged**: that the gaps a server without the store's model leaves are a special case the architecture pays for on every search. This moved half of that cost; the half on the read path is the sweep doing its job.

### Decided 2026-09-11: text the model cannot represent is a verdict, not a gap

**A memory whose embedded text holds no token the model knows is stored without a vector, and the model's verdict on it is recorded.** Mean pooling over no tokens yields a vector of zeros, which `Vector` refuses. Until this was decided, that refusal failed the whole batch the memory arrived in — and because the gap sweep sends every gap through one batch, one such memory stopped every later write to its store and every recall by meaning across every set searched with it. It arrived by a direct write, by a write during a no-model gap, and by import, which is how it reached collaborators. `one-unembeddable-memory-stops-the-store` in [Issues](../../TRACKING.md#issues).

- **The embedder answers per text.** `Embedder::embed_each` returns a vector or `Unrepresentable` for each input. Only an all-zero vector is unrepresentable, because that is the one outcome a text alone can cause; a non-finite or empty vector means a broken model and stays an error, since calling it unrepresentable would hide corruption behind a reading that sounds like a language gap. The strict `embed` survives for measurement code, which wants a vector for every input or a loud stop. The store never calls it.
- **The verdict is a row.** It is written into `vectors` as an empty blob — not `NULL`, because the column is `NOT NULL` and relaxing that means rebuilding the table in every existing store. A row is what the sweep looks for, so the sweep counts the memory as done and does not ask again: the answer can change only with a new text or a new model, and each already re-embeds through its own path. A genuine gap still has no row, and is still filled.
- **The verdict binds the store to its model**, as a vector does. It is that model's output, and a store holding nothing but verdicts must not release its model under *if there are no vectors, there is no model* — a different model arriving later would then skip every one of those memories, because their rows exist.
- **It cannot make a mixture.** The rule is that every vector comes from the store's recorded model *or the memory has none*, and a verdict puts no vector into any comparison, exactly as a gap does not.
- **A write embeds before it commits**, and the memory and its row land in one transaction. The memory used to be committed first, so a failed embedding left it stored behind an error saying it was not — and, with no id returned, beyond the reach of any tool that could remove it.
- **What the caller is told.** The write reports `findable_by_meaning: false` with a note that the model has no tokens for this text — *not* the no-model note's advice to add keywords, since a keyword the model knows would give the memory a vector built from that keyword alone, which does not represent what the memory says. A query the model cannot represent returns no hits with `query_unrepresentable` set, instead of failing the recall. A rebuild reports how many memories it left unrepresentable.

**What this does not settle.** Text with *some* representable tokens — a Chinese memory carrying one English keyword — still embeds, as a vector built from those tokens alone. That is the quiet mode recorded under `recall-is-latin-script-only`, and the check that would catch it, judging coverage of the content rather than of the whole embedded input, is not designed.

### Why not export, drop, and re-import instead

That is the documented recovery for a *project* store, whose every memory can be rebuilt from its file. It is the wrong instrument here and unavailable where it is needed most. The local store is the only copy of local memories, so rebuilding a cache by deleting it is destroying data to repair derived state. A round trip through the file also invalidates every reconciliation base ([Multi-Set Sync](multi-set-sync.md)), making each record read as divergent for a cycle — considerable collateral for rebuilding something the design is at pains to keep from ever leaving the machine.

## Storage is partitioned by scope

Each scope gets its own store — a separate database, or separate tables — rather than all memories sharing one table with a `scope` column. See [Memory Steering](memory-steering.md) for the `Scope` type itself.

Reasons, strongest first:

- **Correctness by construction.** A query against project A's store *cannot* return a local memory. With a scope column that guarantee is a `WHERE` clause someone has to remember, and a forgotten one leaks personal content into a project context — the one-way door the [Export Review](export-review.md) gate exists to guard. Exactly the class of bug [STANDARDS.md](../../STANDARDS.md) says to design out rather than test for.
- **It is the trust boundary, one layer down.** [Trigger and Loading](trigger-and-loading.md) already established that path determines trust rather than content inspection. Project memories arrive over an untrusted channel and local ones do not; separate stores make that structural rather than a column value.
- **Per-set lifecycle becomes natural.** [Multi-Set Sync](multi-set-sync.md) does everything per set — export, import, merge, optional flush. Each is an operation on a store rather than a filtered query against a shared table.
- **Hubness improves, mildly.** A centroid computed over one project's memories is better calibrated than one computed across mixed corpora, and a smaller partition has less hubness to begin with.

That last reason is deliberately listed last. It is real but not load-bearing: if hubness later yields to some other fix, the first three still carry the decision. A structural choice justified on its weakest argument gets undone when that argument evaporates.

### Resolved: stores searched together are centred on the mean of their union

Recall usually wants **project ∪ local**, not a single store — a stated preference matters while working on a project. But with per-store centroid removal, scores from two stores are measured against different baselines, so merging them by score is unsound.

**Every store in a query is centred on the mean of their union.** That makes the searched stores one corpus mathematically, which is what the caller actually has.

The property that decides it, and the one the implementation is tested against:

> A corpus split across two stores answers a query exactly as one store holding all of it would.

Partitioning exists for correctness and trust, not for ranking, so it must not change what recall returns. Only the union centroid satisfies that. **Interleaving by rank** cannot: it imposes a quota the single-store case does not have, so a query whose answer lies entirely in one set still spends slots on the other — the same failure as hubness, and it cannot express one memory being far more relevant than another. **Raw cosine across stores** cannot either: it discards the centring the single-store case applies, making the merged ordering the one measured as *worse* (71.7% against 77.5% at recall@5).

It is also the cheapest of the three. Every store already keeps a running componentwise sum for its own centroid, so the union centroid is `(sum₁ + sum₂ + …) / (n₁ + n₂ + …)` — a few additions and one division, with no second pass over any vector and nothing to cache or invalidate. The concern that motivated listing it as an open question was the cost of computing a per-union centroid at query time; that cost turned out not to exist.

Two consequences:

- **A recalled memory reports which store it came from.** Once several stores answer one query, a local preference and an imported project decision arrive in the same list, and the trust difference between them is exactly what the partition exists to preserve.
- **A store that cannot be searched makes the whole result incomplete.** If any store in the union lacks a model, recall says so rather than quietly answering from the rest — same reason a single store says so.

## Resolved: how the model gets onto the machine

The embedder needs a model file — roughly 129 MB for `potion-base-32M`, whose name counts its 32 million parameters rather than its bytes — which is far too large to commit and has to arrive somehow. The rule that decides this is not "no network":

> **No network at a moment the user did not choose.**

Fetching during installation is fine. The developer is already online installing the software; one more download is invisible to them, and *not* fetching means every user must first learn what an embedding model is in order to use a memory tool. That is real adoption friction spent on something irrelevant to the task they came to do.

What is not acceptable is the same fetch arriving later. A developer installs Scatterhoard, uses it for a week, gets on a plane with their local model stack, opens a project — and hits a network requirement they were never told about. That is the failure this rule exists to prevent, and it is much worse than the friction avoided by deferring the download, because it lands when nothing can be done about it.

Three things follow:

- **`local-only` stays a build feature, and that is what makes the guarantee structural.** With remote fetching compiled out, a runtime fetch is not something Scatterhoard *chooses* not to do — it is something it cannot do. The offline promise then does not depend on nobody adding a convenient lazy-download later.
- **Model acquisition is an explicit step, never lazy.** No fetch on first recall, on first use of a new project, or on encountering an unfamiliar model version. Anything that could fire at an arbitrary later moment is the plane scenario wearing a different hat.
- **A missing model degrades loudly, never silently.** Semantic search is one of three recall paths (see [Recall Mechanisms](recall-mechanisms.md)); keyword search, direct-ID lookup, and graph traversal all still work without a model. So Scatterhoard can run without one — but recall must *say* that semantic search is unavailable. A quiet degradation is precisely the failure the `recall-when` convention warns about: memory that isn't really being consulted, while the session believes it is.

### Setup is a bounded procedure the LLM can carry out

Since a developer using Scatterhoard is by definition working with an LLM, setup can be delegated to it: a HOWTO shipped **with Scatterhoard itself**, aimed at the model rather than the human, describing where the model file goes and how to fetch it.

Two constraints on that document:

- **It must fit in one session.** A setup procedure that runs out of context halfway leaves a half-configured machine, which is worse than one that never started. Bounded steps, no open-ended exploration, and a way to check the result.
- **The error is the entry point.** When the model is missing, the message names the fix directly rather than only reporting failure, so the common case never requires finding the HOWTO at all.

This does not conflict with the rule that nothing instruction-shaped is auto-loaded from a project ([Trigger and Loading](trigger-and-loading.md)). That rule is about content arriving over an untrusted channel — a repository someone else wrote. A HOWTO shipped with the tool the user chose to install is on the trusted side of exactly the same path-based boundary.

### Starting position

A configured path with a sensible default, and refusal-with-instructions when the model is absent. That is the only option that forecloses nothing: an installer that fetches the model can be added on top of it without changing anything here, and vendoring the model into the binary remains possible if single-file distribution ever matters more than binary size.

## Index strategy: brute force first

At the scale a project memory set plausibly reaches — thousands to low tens of thousands of memories — an approximate-nearest-neighbour index is not needed. Ten thousand memories at 384 dimensions is roughly 15 MB of `f32`, and a linear cosine scan over it is a few million multiply-adds: single-digit milliseconds, less with SIMD.

So: **store vectors as blobs and scan them linearly.** No HNSW parameters to tune, no index to rebuild on import, no index file to corrupt or to reconcile after a merge. If a memory set ever grows past roughly 100k memories, an index can be added behind the same recall interface without changing anything above it.

`sqlite-vec` (pure C, no dependencies, usable from `rusqlite` with the `bundled` feature) is the natural upgrade path if that day comes, and is worth keeping in mind when shaping the storage interface — but it is not needed to start.

## Embedder: candidate ladder

Constraints from [Components](components.md) and [Language and Runtime](language-and-runtime.md): self-contained, no network at run time, filesystem-backed, permissively licensed, Rust-native or cleanly bindable.

**Verified 2026-09-11: MIT throughout**, which is what *permissively licensed* was asking for. `potion-base-32M`, `potion-base-8M`, `potion-retrieval-32M` and `potion-multilingual-128M` are MIT, as are the models they are distilled from — `BAAI/bge-base-en-v1.5` and `BAAI/bge-m3` — and the `model2vec-rs` crate. Check a model rather than trusting this line before adding one: `https://huggingface.co/api/models/<repo>` returns it as `cardData.license`.

| Option | Dependencies | Model size | Notes |
| --- | --- | --- | --- |
| `model2vec-rs` | Pure Rust, MIT | 30–130 MB | Static embeddings. A `local-only` feature disables remote fetching entirely. The model names count parameters: `potion-base-8M` is 30 MB on disk, `potion-base-32M` 129 MB (measured 2026-09-11). |
| Candle + a BERT-family model | Pure Rust | ~90–130 MB | Higher quality, substantially slower on CPU, no ONNX dependency. |
| `fastembed` / `embed_anything` | ONNX Runtime (native library) | ~90–130 MB | Good quality; the native dependency complicates single-binary installation. |

**Starting position: `model2vec-rs`**, pending the empirical check tracked in [EARLY-VALIDATION.md](../../EARLY-VALIDATION.md).

Published comparisons put its `potion-base-32M` model at roughly 89–92% of MiniLM's average MTEB score, at a fraction of the size and with throughput measured in tens of thousands of embeddings per second rather than hundreds.

Two reasons that trade is more favourable here than the raw benchmark gap suggests:

- **Semantic search only produces seeds.** [Recall Mechanisms](recall-mechanisms.md) uses semantic similarity to find starting points, then traverses the memory graph from there — and keyword search and direct-ID lookup are independent paths to the same memories. The embedder has to surface plausible entry points, not to be the entire retrieval system.
- **Speed is on the import path.** Because vectors are never exported, every import re-embeds whatever changed. Sub-second for a full set versus roughly a minute makes the difference between an invisible step and one a user notices on every pull.

The `local-only` build feature matters more than it first appears: it makes the offline constraint structural rather than a matter of configuration discipline, in the same spirit as the path-based trust rule in [Trigger and Loading](trigger-and-loading.md).

## Measured: model2vec matches a transformer on this workload

A spike over a 45-memory labelled corpus of real Scatterhoard design memories with 20 queries, embedding content+why+keywords, comparing `potion-base-32M` against `all-MiniLM-L6-v2` on identical corpus, input, and metric:

| model | r@5 raw | r@10 raw | r@5 centred | r@10 centred |
| --- | --- | --- | --- | --- |
| `potion-base-32M` | 71.7% | 83.3% | **77.5%** | 87.5% |
| `all-MiniLM-L6-v2` | 74.2% | 84.2% | 74.2% | **88.3%** |

**The published MTEB gap does not appear on this task.** With centroid removal, the static model is ahead at r@5 and within a point at r@10. The ceiling here is the task and the corpus, not the model — so there is no meaningful quality gain available by paying the size, speed, and dependency cost of a transformer.

Throughput on this hardware was roughly 18,000 memories/sec, so re-embedding a 10,000-memory set on import is comfortably sub-second. Both concerns that motivated the ladder above are resolved in favour of `model2vec-rs`.

### Subtract the corpus centroid before comparing

Removing the mean of the memory vectors from both memory and query vectors lifted `potion-base-32M` from 71.7% to 77.5% at r@5 and 83.3% to 87.5% at r@10. It costs one vector subtraction and fits the design already chosen: the centroid is local derived state like the vectors themselves, and the brute-force scan absorbs the subtraction at negligible cost. Store raw vectors and subtract at scan time rather than storing centred ones, so adding a memory doesn't invalidate everything already stored.

### Hubness is real but not a static-embedding artifact

One memory dense in common domain vocabulary appeared in the top 5 for 13 of 20 queries under `potion-base-32M`. The initial reading was that this is a static-embedding failure from lack of contextualisation — but MiniLM shows the same effect, with its worst hub at 8 of 20. Static embeddings are worse, not uniquely afflicted. Hubness is a general property of high-dimensional spaces: points near the centre of the data become nearest neighbours to disproportionately many others.

**Centroid removal largely fixes it**, which is why it is adopted above. Removing the mean from both sides drops the worst hub from 13 of 20 to 6 of 20 while also improving recall.

CSLS local scaling — penalising each memory by its mean similarity to its own *k* nearest neighbours — was tested as a further correction:

| variant | r@1 | r@3 | r@5 | r@10 | worst hub |
| --- | --- | --- | --- | --- | --- |
| raw cosine | 39.2% | 65.8% | 71.7% | 83.3% | 13 |
| centroid removed | 39.2% | 66.7% | 77.5% | 87.5% | 6 |
| CSLS k=10 alone | 39.2% | 55.8% | 71.7% | 85.8% | 10 |
| centroid + CSLS k=10 | 36.7% | 59.2% | 78.3% | 87.5% | 5 |

CSLS on its own is *worse* than centroid removal, and combining them buys little: +0.8 points at r@5 and one hub, paid for with a 7.5-point regression at r@3. **Not adopted for now.** The result is also the least trustworthy in this spike — k=10 against a 45-memory corpus means the penalty is computed over roughly a fifth of the whole set, a completely different regime from k=10 against several thousand memories. Worth re-testing at realistic scale before being ruled in or out; tracked in [EARLY-VALIDATION.md](../../EARLY-VALIDATION.md).

Hubness still matters for [Recall Mechanisms](recall-mechanisms.md) beyond the aggregate numbers: a hub seed both consumes a seed slot and sends graph traversal into the wrong neighbourhood, so its cost is higher than a recall metric alone reflects.

### Re-run 2026-09-08: raw reproduces exactly, the centred row does not

`corpus/seed-quality-provenance.md` asked for this fixture to be re-run against `potion-base-32M`
rather than trusted on arithmetic consistency. Done, by
[`probe_reproduce_the_seed_quality_figures`](../../tests/probes.rs).

**Raw cosine reproduces every published figure** — 39.2 / 65.8 / 71.7 / 83.3 and worst hub 13 — which
validates the fixture, the `content + why + keywords` composition, the per-query averaged recall
metric and the top-5 hub definition together.

**The centroid-removal row reproduces in one cell.** r@5 of 77.5%, the number quoted throughout, is
exact. The rest is not: measured for the variant that ships, with query and memories both centred,
r@1 is **34.2%** against a published 39.2%, r@3 **57.5%** against 66.7%, r@10 **83.3%** against 87.5%,
worst hub **8** against 6. Centring memories only, or the query only, does not close the gap.

Two claims above should therefore be read with care. *Centroid removal lifted r@10 from 83.3% to
87.5%* does not reproduce — centring leaves r@10 unchanged in every variant tried. And **centring is
not free**: it costs 5.0 points at r@1 and 8.3 at r@3 to buy 5.8 at r@5, where the published row
shows r@1 unchanged. The decision to centre still holds on the evidence — reading live results, it
promotes the right memory to first and surfaces set charters a raw ranking misses — but it is a
trade rather than a free improvement, and it has a second cost recorded in
[TRACKING.md](../../TRACKING.md#issues) under `an-unanswerable-query-returns-confident-hits`: centring
removes the one direction that marked an out-of-domain query as not belonging.

### The static model matters more than static-versus-contextual

Also measured 2026-09-08, re-embedding a live 867-memory corpus under each cached model and scoring
as recall does:

| query | `potion-base-32M` | `potion-retrieval-32M` | `potion-base-8M` |
| --- | --- | --- | --- |
| *what is the link traversal mechanism* | 0.458 | **0.532** | 0.441 |
| *what decides which memory set a memory belongs in* | 0.304 | **0.389** | 0.333 |
| off-domain: *premier league offside rule interpretation* | **0.420** | 0.328 | 0.442 |
| nonsense: *qwertyuiop asdfghjkl* | **0.358** | 0.229 | 0.361 |

Under the shipping model, **nonsense outscores a real in-domain question** (0.358 against 0.304) and
the off-domain query returns three `governance-docs` memories. Under `potion-retrieval-32M` the
inversion disappears: every in-domain query outscores every nonsense one, and the off-domain query
keeps one such memory in its top three. On the labelled fixture the retrieval model is roughly a
wash — better at r@3 and r@10, worse at r@5 — so this is a trade too, and six queries is a pilot.

**Trialled at scale the same day, and the pilot did not survive it.** Against 29 real queries
harvested from session transcripts and 15 with no answer in the corpus, the best possible split
leaves **4 errors of 44 under either model** — identical. The distributions shift together rather
than apart, and the clean six-query gap is gone. Labelled retrieval is a wash, and the two corpora
disagree about its direction: `potion-base-32M` leads on P@5 over the 45 real memories, 32.0% against
29.0%, while `potion-retrieval-32M` leads over the 429 synthetic ones under both blind labellers.
Only r@10 is consistent, favouring the tuned model by two to four points.

**So: no model change recommended**, and the ladder above stands. What the swap changes is *which*
queries fail rather than how many — which is still worth knowing, because it shows the failure is a
property of a particular embedding rather than of embeddings, and so is not evidence that a
contextual model would do better either.

### Decided 2026-09-11: `potion-multilingual-128M` is a supported alternative, and the default does not change

**Measured both ways first.** Under the shipping model, Russian, Hindi and Japanese score a question and an unrelated statement alike — 0.72 to 0.91, where near zero is healthy — and Chinese with no Latin token in it cannot be embedded at all. Under `potion-multilingual-128M` every language tested separates: a paraphrase outscores an unrelated sentence in 24 of 24 triplets, margins 0.24 to 0.40, and an **English** query outscores the unrelated sentence against a paraphrase written in another language, 24 of 24, margins 0.22 to 0.34. The cost is English ranking. On the 45-memory labelled fixture r@5 falls from 77.5% to 54.2% and r@10 from 83.3% to 72.5%, and against 30 authored off-domain probes the `scatterhoard` floor loses its headroom, +0.085 to −0.028.

So **the default stays `potion-base-32M`**, which ranks English better and English is what this tool is used for, and **the multilingual model is supported**, chosen per machine by whoever sets it up — where [decision placement](decision-placement.md) puts it, since only they know what they will be writing. [SETUP.md](../../SETUP.md) names both. Both are MIT, as is everything they are distilled from.

**What the choice costs operationally.** A store owns its model, so changing it is an explicit re-embed of every vector, and two servers sharing a store must agree on one. Vectors never leave the machine, so one contributor's choice binds nobody else — the property that made the embedder a locally reversible decision to begin with.

**Left open by this.** The floor's level is entirely the model's, and between a third and a half of the spread between sets moves with it, so a floor fitted under one model does not transfer to another: `floor-calibration-source` in [Open questions](../../TRACKING.md#open-questions).

### Limitations of this result

One small corpus, written and labelled by the same author, on a single domain. The absolute recall numbers should not be trusted far — some queries have three relevant memories, and the labels are judgement calls. The *comparison* is sound regardless: both models were scored against identical labels, so systematic label bias affects them equally.
