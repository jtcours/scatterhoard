# Label review

Two labellers worked independently, on different models, from a view of the corpus with the link structure withheld.

**Read the policies first.** They decide many labels each, and they are the part worth arguing with. The per-memory text below is only printed where the two disagree enough that reading it is the only way to settle the question.

---

## The policies each labeller applied

### Labeller A

I judged each query by asking whether a session that issued it would want the memory back, not by vocabulary overlap: several memories use a query's exact words while answering a different question (e.g. keyword-spelling memories against a case-folding query), and several answer without sharing any word (the cold-start centroid observation against a query about small stores). Multi-topic queries -- whether joined by 'and' or by commas -- were read as unions, so a query naming dirty flags, per-target bases and the drop/restore safety copy collects everything on each leg rather than only memories spanning all three; the same for the Golden Rule query and the traversal query. Bare-noun queries ('attribution', 'reconciliation', 'store registry design') were read as topic sweeps and labelled generously across the topic's core mechanics, with adjacent-but-derived material pushed to borderline. A bare identifier ('MemoryView') was read as a request for the memory that decides that thing, not for everything the thing touches. Where a query was readable two ways I took the reading the wording most directly supports and then let borderline carry the other reading's material rather than merging them. The relevant/borderline line: relevant means the memory states, decides, or measures something the query asks for, and I would defend it in a review; borderline means the memory is one hop away -- it supplies the mechanism a relevant memory uses, applies the same rule to a neighbouring field, records a consequence of the answer, or is the same shape of problem in a different subsystem. Memories that merely share a keyword with the query and answer nothing it asked went in neither list.

### Labeller B

Relevance was judged solely from the query text and each memory's content/why/keywords, never from keyword-overlap alone: several candidates sharing a query's vocabulary were left out because they addressed a different question, and some memories with no shared wording were kept in because they answered the query's substance. For queries naming several topics (joined by commas or 'and'), the union rule was applied: a memory needed to answer only one named topic to qualify, not all of them at once. Bare single-word or short-phrase queries naming a design area that the corpus documents as a bounded cluster (e.g. 'attribution', 'reconciliation', 'store registry design', the Golden Rule / people cluster, the traversal/link-labels cluster) were read as wanting that whole cluster, since every memory in it is substantively about the named subject; queries that instead named several specific mechanisms within a larger area (e.g. the dirty flag, the per-target reconciliation base, and the drop/restore safety copy) were scoped to memories genuinely about each named mechanism rather than the entire surrounding design space. Borderline was used for memories that share the topic but answer a neighbouring question, illustrate the topic only in passing while being centrally about something else, or generalise/instantiate the query's concern one step removed (e.g. an analogous pattern applied to a different field, or a passing mention inside a memory centrally about another decision). Several queries that named a very specific phrase or mechanism (MemoryView, the withheld correcting convention, case folding of keywords, the lone-vector-centroid observation) matched exactly one memory in the corpus; those were left with a single relevant id and an empty or near-empty borderline list rather than stretched to include topically adjacent but substantively different memories. Where a query's phrasing did not exactly match any memory's language (e.g. 'malformed tool call'), the closest substantive match was used as the relevant answer and adjacent error-handling memories were placed in borderline rather than promoted to relevant on the strength of shared vocabulary alone.

---

## Where the two agree, query by query

`strict` compares what each would defend. `permissive` adds each one's borderline band. A large gap between them means the disagreement is about *where the edge is*, not about what the query means.

| strict | permissive | A rel/bord | B rel/bord | query |
|---:|---:|---:|---:|---|
| 6% | 20% | 17/8 | 1/4 | malformed tool call remark, warning rather than refusing,  |
| 17% | 50% | 6/2 | 1/3 | served convention text read by strangers, examples that as |
| 38% | 50% | 8/6 | 3/4 | why the convention catalog is deliberately kept small |
| 47% | 32% | 17/16 | 11/10 | reconciliation |
| 50% | 20% | 2/3 | 1/0 | lone vector is its own centroid so a single memory scores  |
| 50% | 14% | 2/5 | 1/0 | store registry in-tree |
| 50% | 17% | 2/4 | 1/0 | a convention that names an operation the catalog does not  |
| 60% | 33% | 4/5 | 4/3 | how are similarity scores computed across memory sets |
| 65% | 77% | 26/12 | 40/5 | dirty flag, reconciliation base per target, drop and resto |
| 71% | 70% | 12/9 | 12/6 | two servers disagreeing about the embedding model |
| 72% | 55% | 27/14 | 23/4 | graph traversal from seed memories, link labels, decay by  |
| 75% | 100% | 4/0 | 3/1 | mutation testing |
| 83% | 50% | 6/5 | 5/2 | reconciliation base migration |
| 83% | 71% | 18/5 | 15/3 | tombstones, forgotten memories, counting records in a set  |
| 86% | 67% | 7/2 | 6/0 | confidence field calibration, what a low confidence number |
| 86% | 79% | 29/4 | 25/1 | the Golden Rule, memories about people, charitable judgeme |
| 100% | 50% | 1/3 | 1/1 | what happens to similarity scores in a store with very few |
| 100% | 50% | 1/1 | 1/0 | MemoryView |
| 100% | 33% | 1/2 | 1/0 | how case folding of keywords works |
| 100% | 50% | 1/1 | 1/0 | how are keyword lookups made case-insensitive without alte |
| 100% | 83% | 12/5 | 12/4 | attribution |
| 100% | 86% | 36/6 | 36/0 | store registry design |
| 100% | 70% | 6/3 | 6/2 | how keywords should be chosen and spelled when storing a m |
| 100% | 56% | 4/5 | 4/1 | near-duplicate detection before storing, surfacing neighbo |

---

## What each labeller took the query to mean

Where these differ, the labels differ for a reason that can be settled by choosing a reading -- which is one decision, not many.

### malformed tool call remark, warning rather than refusing, surfacing damage in the response

*strict overlap 6%*

- **A read it as:** Union of three: refusing a malformed call in a way the caller can act on, preferring a directive warning to a refusal, and reporting damage in the response rather than silently. Reporting decisions in adjacent subsystems (status, forward-compatibility, tripwires) are borderline.
- **B read it as:** No memory uses the phrase 'malformed tool call'; the closest substantive match is the decision that a tool which cannot do what was asked returns an error-marked result rather than a protocol error, so the model sees an actionable answer instead of a bare refusal. Adjacent error-handling and unknown-entry-warning memories were left borderline since they share the warn-vs-refuse shape but not the same mechanism.

### served convention text read by strangers, examples that assert facts about the reader's own store

*strict overlap 17%*

- **A read it as:** Read as the drafting rule for served text: it is read by strangers, so an example asserting facts about the drafter's own store is false for its reader. Relevant are that test, the state-don't-derive rule, the reinforcement-versus-comment split, and the routing draft that compiled one user's arrangement into everyone's catalog.
- **B read it as:** Exact match: the memory describing the test caught when two conventions cited 'what this store holds', true only of the store they were drafted against and false for every other reader; other reinforcement-field memories that don't discuss the strangers-reading-examples failure were borderline.

### why the convention catalog is deliberately kept small

*strict overlap 38%*

- **A read it as:** Read as: what is the reasoning for the small catalog. Relevant are the scope-limiting decision, the context-cost framing, the character budget and its raises, and the built-in inclusion criterion. Memories that merely invoke the constraint while deciding something else are borderline.
- **B read it as:** Kept the memories that state the deliberate-minimalism policy itself and the context-cost reasoning behind it; the character-budget mechanism (which governs the size of conventions once included, a related but distinct decision) was left borderline rather than promoted.

### reconciliation

*strict overlap 47%*

- **A read it as:** Bare-noun topic sweep over store-against-file reconciliation: the two reconciliations, the base, the decision table, checksum comparison, tombstones changing what absence means, the anomaly, and the ordering hazard. Base migration, git-side file merging, and the import/restore split are borderline as neighbouring layers.
- **B read it as:** Bare topic query for the reconciliation mechanism itself (file-vs-file and store-vs-file, the five-way decision table, the checksum's role); memories that mention reconciliation only as a side effect of a different decision (create vs import, whole-set export, multi-set flush) were borderline.

### lone vector is its own centroid so a single memory scores 0.0

*strict overlap 50%*

- **A read it as:** Read as a statement of the cold-start artifact, wanting both the observation and the mechanism that produces it. The observation and the mean-subtraction decision are relevant; union-centroid and threshold memories are borderline as the same arithmetic in a different setting.
- **B read it as:** Near-verbatim restatement of one memory's content (a lone vector is its own centroid, so a single-memory store scores 0.0); no other memory states this.

### store registry in-tree

*strict overlap 50%*

- **A read it as:** Read as the specific question of putting the store inside the repository tree. Relevant are the memory that rules in-tree out and the proposal that places stores centrally instead; the other placement memories (home versus common directory, the reserved directory's contents) are borderline because they answer where among out-of-tree locations.
- **B read it as:** Specific rejected-design query (storing the registry/database inside the project's reserved directory); exactly one memory discusses this option and why it was ruled out.

### a convention that names an operation the catalog does not define, withholding convention text until tools exist

*strict overlap 50%*

- **A read it as:** Read as the withholding rule: convention text drafted before the tools it names exist, and held back until they do. The correcting-convention case and the written-but-unserved-is-normal note answer it; catalog adjacency, pointing-versus-restating, and the which-half-first argument are borderline.
- **B read it as:** Exact match: the one memory describing the correcting convention being deliberately withheld from the served catalog until the amend/forget tools existed.

### how are similarity scores computed across memory sets

*strict overlap 60%*

- **A read it as:** Read as: how is a similarity score made comparable when a query searches several stores at once. Relevant is the union-centroid machinery and the same-model precondition; the general centroid decision and single-store cosine mechanics sit borderline because they do not address the cross-set part.
- **B read it as:** Read as asking how the recall similarity score is computed, especially when several stores are searched together; kept the centroid/union-centring mechanism and the underlying per-store centring it depends on, and set aside the general linear-scan storage fact and traversal's separate score-combination question as adjacent rather than answering.

### dirty flag, reconciliation base per target, drop and restore safety copy

*strict overlap 65%*

- **A read it as:** Three named topics, labelled as a union: the dirty flag and its removal from the destructive path; the reconciliation base keyed by record and target; and the unconditional safety copy on drop and restore. Base-migration and store-registry consequences are borderline -- they follow from the per-target base rather than describing it.
- **B read it as:** Union of three named mechanisms (dirty flag, per-target reconciliation base, drop/restore safety copy); included memories substantively about any one of them, excluded the broader store-registry/set-lifecycle memories (create, fork, rename, cross-project, orphan states) that don't touch these three specifically.

### two servers disagreeing about the embedding model

*strict overlap 71%*

- **A read it as:** Read as the two-server model-mismatch problem: the observed loop, the oscillation analysis, and every rule written to answer it. Single-process embedding-state memories (mixture, provenance, blocked writes) are borderline because they describe the condition rather than the disagreement.
- **B read it as:** Kept the memories that describe the two-server model-mismatch scenario itself, the oscillation/loop it caused, and the refusal-plus-remedy design that resolved it; put single-store model-state and attribution's analogous (but not embedding-model) convergence pattern in borderline.

### graph traversal from seed memories, link labels, decay by distance, fan-out

*strict overlap 72%*

- **A read it as:** Union of four: the walk from seeds, the link-label vocabulary, decay with distance, and fan-out bounds. Labelled as one body because the traversal design and the label vocabulary are each asked for by name. Reverse-index mechanics, link-target structure and per-label open questions are borderline -- they concern edges without concerning the walk.
- **B read it as:** Union of seeds, link labels, decay-by-distance, and fan-out; kept the whole traversal-mechanics cluster and the whole link-label vocabulary cluster since both are exactly this design area, and put the adjacent already-known-memories-as-seeds efficiency angle and a keyword-tagging proposal that only mentions disabling traversal in borderline.

### mutation testing

*strict overlap 75%*

- **A read it as:** Read as: where does this project use mutation testing. Relevant are the four memories that record a mutation as the thing holding a guard, or that classify the mutation script as a development-only tool. Left borderline empty: memories about tests in general are not about mutation testing.
- **B read it as:** Kept memories that use mutation testing as evidence for a specific guard (concurrency locking, the re-attribution invariant, the no-default scope type); the export-scan memory that merely contrasts itself with 'the project's mutation-testing script' in passing was borderline rather than relevant.

### reconciliation base migration

*strict overlap 83%*

- **A read it as:** Read narrowly as the migration of stored reconciliation bases. Relevant are the never-keep-what-you-cannot-prove rule, the never-merge-across-targets rule, the canonicalisation route, the cost of dropping, the urgency assessment, and the false-base invariant that motivates all of them. Base definition and the designs that would obsolete the per-target base are borderline.
- **B read it as:** Narrower than query 7: only the memories that are specifically about migrating the reconciliation-base table (dropping/keeping/canonicalising old base rows), not the base-per-target design generally.

### tombstones, forgotten memories, counting records in a set versus live memories

*strict overlap 83%*

- **A read it as:** Union of tombstones, forgetting, and the record-count-versus-live-memory distinction. The first two legs are well covered; the third has no memory that states it directly, so status memory counts and the graph-shape measurement sit borderline rather than being stretched into relevant.
- **B read it as:** Union of tombstones, forgetting, and counting live vs total records; the tombstone/forget design cluster answers the first two fully, but no memory directly addresses counting records with vs without tombstones, so the third leg is represented only by borderline memories that mention memory counts in status reporting.

### confidence field calibration, what a low confidence number means, recalibration versus a new belief

*strict overlap 86%*

- **A read it as:** Union of confidence calibration, what a low value means, and recalibration versus supersession. Relevant are the ladder and its measurement, the anchors-not-thresholds rule, the laundering hazard at the low end, both recalibration-is-an-amendment memories, the claim/apparatus split, and the low-confidence reading of a sweeping characterisation.
- **B read it as:** Bare topic cluster (confidence field); all six memories tagged with it are directly about calibration, what a number means, or recalibration-vs-new-belief, so all are relevant with nothing left over for borderline.

### the Golden Rule, memories about people, charitable judgement versus accurate technical criticism

*strict overlap 86%*

- **A read it as:** Union of three legs: the Golden Rule itself and its four demands, memories that characterise people, and the charity-versus-accuracy boundary. All three legs live in one body of reasoning, so the relevant list is large by construction. The misuse-friction memory and the reinforcement-channel memories are borderline.
- **B read it as:** Union of the Golden Rule itself, its application to memories about people, and the charity-vs-accuracy boundary; kept the whole Golden Rule cluster since every entry is substantively about one of the three, except the misuse/friction memory which invokes the document as precedent without discussing people or the charity/accuracy boundary itself.

### what happens to similarity scores in a store with very few memories

*strict overlap 100%*

- **A read it as:** Read as: what does the scoring do when the corpus is tiny. The observed one-memory-scores-zero case answers it directly; the centroid decision, the small-corpus caveat on local scaling, and the cold-start threshold note are borderline because they bear on it without being about few memories.
- **B read it as:** Read as asking specifically about the cold-start/few-memory case; only one memory in the corpus describes what a nearly-empty store's scores look like, so the list stays short by design rather than by oversight.

### MemoryView

*strict overlap 100%*

- **A read it as:** Read as a bare type name: the memory that decided a single shared view type carries a memory's complete field set to every tool. Kept narrow -- memories that merely describe what recall returns are borderline, not answers about the type itself.
- **B read it as:** A bare name query for the shared return-type that carries a memory's complete field set through every tool; exactly one memory describes this view type, even though it never uses the name 'MemoryView' itself.

### how case folding of keywords works

*strict overlap 100%*

- **A read it as:** Read as: what does the system do about keyword case. Only the spelling convention actually states the folded-form-beside-the-written-form arrangement; the set-name rule (explicitly not case-folded) and the spelling-drift memory are borderline as the adjacent cases a reader would also want.
- **B read it as:** Only one memory describes the folded-form-as-separate-column mechanism that makes keyword lookup case-insensitive; the set-name case-folding memory was excluded since it is about a different field (set names, explicitly never folded) rather than keywords.

### how are keyword lookups made case-insensitive without altering what was stored

*strict overlap 100%*

- **A read it as:** Same subject as the previous query with the design constraint spelled out; the separate-column answer is the one memory that satisfies both halves. The set-name rule is borderline because it is the deliberate counter-case, not an answer.
- **B read it as:** Same underlying mechanism as query 4, asked from the case-insensitivity-without-altering-storage angle; same single answer.

### attribution

*strict overlap 100%*

- **A read it as:** Bare-noun topic sweep over who a memory and a store are attributed to: store author, per-memory created-by/modified-by, claiming, correction and re-attribution, plus origin as human-versus-LLM. Tombstone attribution and the export gate's last-modifier scoping are borderline -- attribution appears there in service of another decision.
- **B read it as:** Bare topic query, read as wanting the whole attribution design cluster (store author, per-memory created-by/modified-by, claiming, re-attribution); memories where attribution is only a supporting detail of a different decision (tombstone fields, an anomaly example, a concurrency-defect list) were borderline.

### store registry design

*strict overlap 100%*

- **A read it as:** Read as a sweep of the store-registry proposal: what the registry holds, identity versus location, placement and discovery, registration, status reporting, inspect, and the costs and consequences. Trust/set-name questions the registry would answer, and the per-target base it would remove, are borderline.
- **B read it as:** Broad topic query naming the whole registry design; every memory substantively about the store registry (identity, common/home split, registration, status reporting, orphans, discovery, alias rejection) was kept relevant since each is literally documenting that one design.

### how keywords should be chosen and spelled when storing a memory

*strict overlap 100%*

- **A read it as:** Read as the keyword-writing convention: how many, how spelled, whether to coin or reuse, the reserved prefix, and the rejected person-name ban. Keyword use elsewhere -- embedding input, keyword search, set-declared conventions -- is borderline because it is about what keywords do, not how to pick them.
- **B read it as:** Kept the memories that set the actual choosing/spelling guidance (count, drift/recall-before-coining, hyphen-vs-underscore spelling, namespace reservation, the measured evidence behind it, and the rejected person-name ban); a related open question about per-set keyword conventions and a tagging proposal were borderline rather than direct guidance.

### near-duplicate detection before storing, surfacing neighbours versus warning, similarity thresholds

*strict overlap 100%*

- **A read it as:** Union of pre-store duplicate checking, the surface-neighbours-rather-than-warn decision, and thresholds. Relevant are the half-overlap argument, the threshold-avoidance argument, the recall-before-storing convention and the twins evidence. Confidence anchors, cold-start separation and the general affordance rule are borderline.
- **B read it as:** Kept the memories that state the recall-before-storing convention, the half-overlap/software-surfaces-neighbours-rather-than-warns argument, the measured duplicate evidence behind it, and the decision to decline a similarity threshold; a general single-source-of-truth open question was borderline since it is about a different kind of duplication (facts, not memories).

---

## Memories worth reading

Only the 7 queries where the two agree on less than 60% of what they would defend. Everything else is settled by the policies above.

### lone vector is its own centroid so a single memory scores 0.0

#### Only A would defend (1)

**`019fe70e`** — Removing the mean of the memory vectors from both memory and query vectors lifted recall from 71.7% to 77.5% at five results and 83.3% to 87.5% at ten. It costs one vector subtraction and fits the design already chosen, since the centroid is local derived state like the vectors themselves and the brute-force scan absorbs the subtraction at negligible cost.

*Why:* Store raw vectors and subtract at scan time rather than storing centred ones, so adding a memory does not invalidate everything already stored.

### why the convention catalog is deliberately kept small

#### Only A would defend (5)

**`019fe77c`** — A convention belongs in the built-in catalog if a user who does not know it exists would be worse off. Held as a test to try against candidates rather than a settled rule, since the division is expected to move as the catalog grows and a criterion adopted before it has judged anything real is one nobody has watched operate.

*Why:* What recommends it is that it discriminates without needing a taxonomy of subject matter, and the asymmetry it encodes is the reason it works: the cost of a missing built-in is invisible to the person paying it, and the cost of an unnecessary one is not. So the test puts the burden of proof where the failure is silent.

**`019fe780`** — The character budget applies to the built-in catalog and not to add-ons. Built-in conventions are served to everyone, so their cost is a tax nobody opted into and the project has to hold the line on their behalf. Add-ons are a matter of user choice, and a user who enables one has both the information about their own context pressure and the stake in it.

*Why:* A convenient consequence: the budget test measures the compiled catalog, which is exactly the built-in set, so if built-ins stay compiled in and add-ons load at runtime the existing test goes on measuring precisely the right thing -- the boundary the policy draws is already the boundary the code draws.

**`019ff721`** — The budget was raised on what it is actually spending rather than on what the next convention needs. Eight thousand characters is around 1,950 tokens against context windows running from 128K on smaller local models up to about 1M, and both numbers are small at either end. A budget set at the old level would have been paid for by shrinking conventions against each other -- spending drafting effort and clarity to save something the context window does not notice.

*Why:* The token figures are the pessimistic end of the proxy, which is the right end to decide on: they assume the catalog drifts toward the vocabulary tables the constant's documentation warns about, where prose runs considerably cheaper.

**`019ffc4c`** — The second raise was decided on the same ratio the first was, against a proportional bar: stay well under 4% of the smallest context window in range. What has kept both honest is that each was argued from the ratio rather than from what would not fit -- a raise argued the other way round is the one to refuse, because a budget raised whenever it binds is not a budget.

*Why:* The instruction is still to work towards it rather than fill it, and it applies harder at a second raise than a first. The standing worry about a session that records everything has a catalog form, and more room is exactly when it applies.

**`019ffc51`** — The budget constant does double duty: it is also the default page size for a conventions request. That coincidence is what makes the catalog reliably serve in a single page -- the ceiling on its size and the default page size are the same number, so a catalog that passes the budget test cannot need a second page. Changing one changes the other.

*Why:* A coupling that is load-bearing and invisible from either side is worth knowing before touching the constant, since the failure would be a catalog that silently needs paging.

### reconciliation

#### Only A would defend (8)

**`019fcc3b`** — The old export-pull-merge-import sequence is correct only if followed in that order. Export-then-pull works, because the working tree holds the store's state and git merges origin against it with a real ancestor. Pull-then-import is silent data loss: the working tree still holds the last export, git fast-forwards it, and an import that replaces writes the pulled file over the store. Every memory recorded that session is gone, with no error and nothing left to diff against.

*Why:* git pull happens in a terminal Scatterhoard never sees, so export first is a rule the tool cannot check at the moment it is broken, and the penalty falls on exactly the work that just happened. That is a data-loss defect waiting for its first real user, not a usability wrinkle to document.

**`019fcc44`** — With tombstones, absence of a record in the file no longer means deletion -- deletion is a line that says so. This is what lets import keep a memory the file has never heard of instead of guessing whether it was forgotten upstream.

*Why:* The earlier conclusion that git's merge made a separate tombstone mechanism unnecessary is overturned: a deleted line is unambiguous only to a reader holding the ancestor, and the store does not hold it.

**`019fcc49`** — Each record carries the checksum it had when the store and the file last agreed, called the base. It is local per-replica state, living in the store beside the vectors and the export-review ledger, never in the file, because it records an agreement between two particular replicas and means nothing to a third.

*Why:* Keeping it out of the file is what makes it correct rather than merely private: a base written into the exchanged artifact would be read by a third replica as a fact about their agreement, which it is not.

**`019fcc4e`** — The base is keyed by record and target, where the target is the export destination. One store serves every checkout of a repository, since the database is keyed on the set name alone -- but a developer may hold the same repository checked out twice on different branches, and one base per record cannot describe both. Importing from the second checkout would read the first checkout's agreement as changes arriving from origin, and every record would show up modified.

*Why:* The cost is one checksum per record per target, a few hundred kilobytes for a ten-thousand-memory set with two checkouts, so this is bookkeeping rather than scale. The rejected alternative -- one nominated target, other exports untracked -- saves state that is demonstrably cheap and makes the second checkout mis-merge silently.

**`019fcc80`** — A record present in the base, missing from the file, with no tombstone cannot arise from any supported operation under tombstone discipline, so it means a hand-edit, a bad merge, or a file written by a build older than tombstones. It is surfaced, not resolved.

*Why:* Reading it as forgotten is the silent-loss path this design removes; reading it as keep quietly resurrects things someone may have deliberately removed. Neither guess is available to the software, and a human or the LLM can settle it in a sentence.

**`019fcc89`** — Export writes the entire set and import reconciles the entire store against the entire file. There is no partial mode, and the base machinery assumes there is none, replacing a target's whole base set rather than merging into it. Two parties exchange memories by each exporting everything and importing everything; nothing needs selecting and the sequence does not matter.

*Why:* Whole-set exchange is what makes the ordering property hold, so partiality is not a missing feature but a premise the reconciliation rests on.

**`019fcc8e`** — Trimming records out of an exported file is not a subset export; it is the missing-record anomaly arriving through an edit that feels routine, because editing an export is a procedure the design blesses elsewhere. The difference is that re-attribution changes records in place and removes none. A removed record the target had previously agreed on is reported, named, and written for not at all.

*Why:* Loud rather than lossy, and the case is pinned by a test rather than left to the argument, because the edit feels routine at exactly the moment it is not.

**`019fd14f`** — Merge is the word that caused the trouble, because it names two different operations at two different layers. Git merges two files with a common ancestor, which Scatterhoard leans on rather than reinventing. What changed is the assumption underneath -- that by the time a file is imported the store holds nothing the file does not.

*Why:* The per-record base is not the proposal that was considered and rejected. That one reconstructed an ancestor from live database state; this one records the ancestor at the moment agreement actually happens, which is cheap, exact, and available at a boundary git cannot see.

#### Only B would defend (2)

**`019fcc57`** — The base is a change detector, not a security boundary, and that is what sizes it. A crafted record colliding with the stored base makes reconciliation conclude origin did not change this, and therefore not import the attacker's content -- the collision suppresses their own change. Every other branch needs control of the store's side, which supplying a file does not give.

*Why:* A cryptographic hash is not merely unnecessary but misleading: it would state a guarantee this value does not provide and invite a later reader to use it as an integrity or provenance check. Tracing what a collision actually buys is what makes the sizing argument rather than assuming stronger is safer.

**`019fd14b`** — Import reconciles a file against a store that may have diverged from it, keeping a memory the store holds that the file has never carried. Restore replaces: the file becomes the store and local divergence is discarded on purpose, serving the case where the store is corrupt or being rebuilt and reconciling against it would mean merging against content nobody trusts.

*Why:* The same decision-placement test that collapsed export and backup gives the opposite answer here, which is worth being explicit about because the surface pressure runs the other way.

### store registry in-tree

#### Only A would defend (1)

**`019fd651`** — Proposed: a scope resolves to exactly one store at a time and a store belongs to exactly one checkout. Stores live centrally named by a UUID assigned at creation, each records its own checkout path, and a machine-local registry records which store is live for each scope. Re-pointing a scope switches which store is live rather than redirecting one store's exports.

*Why:* What this buys is that a store has at most one export destination, so the base collapses from record-and-target back to record alone -- no target key, no path spelling to normalise, no unrecognised-target case. Two checkouts stop being one store pretending to be two replicas and become two replicas.

### a convention that names an operation the catalog does not define, withholding convention text until tools exist

#### Only A would defend (1)

**`019ffc55`** — A recall-gloss convention is wanted and not yet drafted: when the LLM surfaces a recalled memory to the human it should lead with a short gloss built from the memory's context field rather than restating content cold, which is the behaviour that actually solves the friction the context field exists to address. It also needs guidance on what is worth capturing into context in the first place, kept loose rather than a fixed schema.

*Why:* A convention whose text is written but not yet served is a normal state and is a backlog entry rather than an omission, and the same holds for one identified but undrafted.

### served convention text read by strangers, examples that assert facts about the reader's own store

#### Only A would defend (5)

**`019fc6df`** — Each convention carries two kinds of supporting text and they are not the same. Reinforcement is served alongside the convention every time it is used, reinforcing it in the moment and clarifying when it applies, and it lives in the data. Update-time context is the background someone needs when changing the convention, and it lives in source comments, which are never served.

*Why:* The test that sorts them: does this text reinforce the convention at the moment of use? If yes it is reinforcement; if it only helps someone deciding whether to revise the convention, it is a comment.

**`019fc6e7`** — A memory also has a why, and the two are similar in that both clarify when the thing they are attached to applies, but they are governed differently. A convention's reinforcement is held to a fixed bar, because conventions are hard-coded, reviewed and served into every session. A memory's why is not, since the LLM drafts it in the moment. The field is named reinforcement, which states the standard it has to meet.

*Why:* The Golden Rule reaches runtime through this field, which is why it must be served data rather than a comment -- a comment would mean that decision silently never takes effect.

**`019ff56e`** — A first draft named three destinations -- project, cross-project, personal -- and that was wrong, caught before it shipped. It compiled one user's arrangement into everyone's catalog: a user with a single set, or one per client, or a team set beside a personal one, would get a rule describing somebody else's setup with no way to correct it short of a rebuild. So each set says what it is for, in a memory carrying a reserved keyword, and routing is a match against those.

*Why:* This is the move the design makes everywhere else -- derive rather than keep a second copy -- applied to the one fact the catalog had no business holding. The sets are the authority on what they contain, and it needs no new machinery, since a keyword search with no scope already searches every loaded set and tags each hit.

**`019ffb39`** — The person clause restates rules the convention already makes rather than referring to them, because it is those demands applied without the exemption a judgement about someone quietly gets. That was the elegant finding in the derivation and it is exactly what must not be left implicit: a weaker model, without an extended reasoning session, will not make that leap. The design document may derive; the served text must state.

*Why:* It is a constraint on drafting that follows from who reads the served text, and it cuts against the instinct to keep the catalog compact by pointing rather than repeating.

**`019ffb7a`** — The convention points at the correcting convention rather than restating it, which is not in tension with the rule that served text must state rather than derive: that rule is about not making a reader reconstruct an omitted derivation, and the correcting convention is served in the same catalog, in front of the reader. Restate what is absent; point at what is present.

*Why:* The pull while drafting was to inline it, which is why the distinction is written down rather than left to judgement each time.

### malformed tool call remark, warning rather than refusing, surfacing damage in the response

#### Only A would defend (16)

**`019fcc80`** — A record present in the base, missing from the file, with no tombstone cannot arise from any supported operation under tombstone discipline, so it means a hand-edit, a bad merge, or a file written by a build older than tombstones. It is surfaced, not resolved.

*Why:* Reading it as forgotten is the silent-loss path this design removes; reading it as keep quietly resurrects things someone may have deliberately removed. Neither guess is available to the software, and a human or the LLM can settle it in a sentence.

**`019fcc8e`** — Trimming records out of an exported file is not a subset export; it is the missing-record anomaly arriving through an edit that feels routine, because editing an export is a procedure the design blesses elsewhere. The difference is that re-attribution changes records in place and removes none. A removed record the target had previously agreed on is reported, named, and written for not at all.

*Why:* Loud rather than lossy, and the case is pinned by a test rather than left to the argument, because the edit feels routine at exactly the moment it is not.

**`019fd146`** — A caller may still say back up my local memories in words; the tool they reach for is export with local scope, and the response should say the gate was skipped and why, so the difference stays visible where it matters -- in the result, not in the surface.

*Why:* Collapsing two tool names into one loses nothing only if the distinction reappears in the output. Putting it there rather than in the surface is what keeps the collapse honest.

**`019fd16f`** — The two ways a safety copy can fail are not the same. If the store cannot be read -- the corruption case -- the memories are already beyond saving, refusing protects nothing and blocks the repair, so proceed and report unmissably that no copy was made. If the destination cannot be written, the store is fine and the memories are recoverable, so refuse and let the caller fix the destination.

*Why:* Worth stating because the natural implementation -- if the export failed, stop -- gets the first case exactly backwards.

**`019fd1ab`** — No key removes the need for the warning. Canonicalising a path collapses spellings but does not survive the checkout being renamed or moved, which orphans every base just as thoroughly and reads as a target never seen before. Since no key eliminates the unrecognised-target case, the load-bearing fix is that an unrecognised target is always said out loud, and normalisation is hygiene that reduces how often it fires.

*Why:* Separating the load-bearing fix from the hygiene is what stops normalisation being mistaken for the solution and the warning being dropped once paths are tidy.

**`019fe600`** — On a model mismatch, reads keep keyword, id and traversal, and recall refuses the semantic path rather than rebuilding, because the measured alternative was a confidently-scored ranking computed across two incompatible embedding spaces, indistinguishable from a correct one.

*Why:* Its job is to withhold a number computed across two embedding spaces, which a warning cannot do -- the number would still be there, and something would still rank on it. That is why this stays a refusal where the re-embed does not.

**`019fe612`** — The re-embed always proceeds, carrying the warning with it. The harms are asymmetric: what the check protects is derived state rebuilt in under a second, while a wrong refusal costs semantic recall itself. Blocking a capability to protect cheap rebuildable state is the wrong way round.

*Why:* The lockout is concrete rather than hypothetical. Decommission the second server and try to restore recall with the model that remains: the history says that model was already displaced, so a re-entry check refuses it, while the recorded model is one nothing on the machine can supply. A store must not be reachable into a state its own tool surface cannot repair.

**`019fe617`** — What carries the weight is that the message is directive rather than descriptive. A third alternation between two models is a fact the reader may or may not act on; saying that two servers are configured with different models, that this will keep recurring until that is fixed, and to raise it with the human rather than re-running says what to do -- and is available in a warning exactly as much as in a refusal.

*Why:* This removes the override entirely, along with the question nobody could answer about it: an LLM meeting a refusal that names an override is fairly likely to simply use it. With nothing to refuse there is nothing to override.

**`019fe67b`** — If any store in the union lacks a model, recall says so rather than quietly answering from the rest -- the same reason a single store says so.

*Why:* Partial results that do not announce themselves are the failure the loud-degradation rule exists to prevent, and the union case is where it would be easiest to let slip, since the answer still looks complete.

**`019fe6a9`** — A missing model degrades loudly, never silently. Semantic search is one of three recall paths, and keyword search, direct-id lookup and traversal all still work without a model, so Scatterhoard can run without one -- but recall must say that semantic search is unavailable.

*Why:* A quiet degradation is precisely the failure the recall convention warns about: memory that is not really being consulted, while the session believes it is.

**`019ff01f`** — The export response carries a note beside the scanned flag, because false was true for two unrelated reasons and said which was which for neither: a local export skips the gate by design, having nothing to publish, while a shared export skips it only because no scan exists.

*Why:* The distinction gets sharper rather than redundant once the scan lands -- a shared export that genuinely went unscanned must not read like a local one that never needed scanning -- so the note is not scaffolding to remove.

**`01a004b8`** — The branches were never enforcement. JSON Schema permits properties a branch does not list unless the branch forbids them, which these do not, so a caller could always have sent an impossible combination. What the branches buy is that nothing suggests it -- real and worth having, but communicative rather than structural.

*Why:* The claim that such a call could not be constructed and believed stood until a client proved otherwise, and the overstatement mattered in two directions.

**`01a004f4`** — The sibling shape does make an invalid combination constructible -- a target set with no target. That is the objection an earlier issue raised, and it is accepted here for a reason that did not apply then: the alternative is not a more precise schema but an emptier one, since the client that would have enforced the distinction is the client that discards it.

*Why:* A combination the schema no longer forbids is refused by the parser and named in the response, which moves the guarantee rather than dropping it.

**`01a0054f`** — Tool arguments deserialise straight into the memory model's types, whose constructors already validate. An out-of-range confidence or a malformed set name is refused because the type refuses it, not because the tool layer checks -- parse-don't-validate reaching the outermost boundary.

*Why:* The important case is scope: because the scope type has no default, a call omitting it cannot be deserialised at all, so the explicit-never-inferred rule reaches the tool boundary with no check anyone could forget. Confirmed by mutation -- making the parameter optional does not fail a test, it fails to compile, because there is no function that guesses a scope.

**`01a00558`** — Naming a set this machine does not hold is an error rather than an invitation to create one, since inventing a store on a typo would silently split a set in two. But the refusal has to name an action the caller can take, which it did not: it said to check the set name with no way to list names, and that sets have to be imported with no import tool -- two dead ends in one sentence.

*Why:* A refusal that is correct and unactionable is only half a refusal, which is the shape a correctness-focused error message reaches naturally and has to be pushed past.

**`01a0055d`** — A missing embedding model does not fail the call. Recall returns what it can and says, in words the model will read, that the store could not be searched by meaning and that memory should be treated as unconsulted rather than empty.

*Why:* Treating an unsearchable store as empty is the reassuring failure the loud-degradation rule exists to prevent, and the tool layer is where that rule has to be honoured.
