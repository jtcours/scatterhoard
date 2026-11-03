# Conventions

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

Conventions act like memories but are hard-coded. The LLM should load them into any session that uses Scatterhoard if it has not already done so (see [Trigger and Loading](trigger-and-loading.md)).

Conventions consume context, and some systems like Claude Code will dump to a file if the response is too large, so we should keep them compact and maybe arrange a cursoring mechanism in the tool that fetches them. We should also be careful about interfering with locally defined prompts and rules.

## Reinforcement, and the comments beside it

Each convention carries two kinds of supporting text, and they are **not** the same thing:

- **Reinforcement** — served alongside the convention, every time it's used. It reinforces the convention for the LLM in the moment and helps clarify when the convention applies. This lives in the data.
- **Update-time context** — the background someone needs when *changing* the convention, drawn on when editing it and never when using it. This lives in source comments, which are never served.

**The test that sorts them: does this text reinforce the convention at the moment of use?** If yes, it's reinforcement and belongs in the data. If it only helps someone deciding whether to revise the convention, it's a comment.

### Served text must not assert facts about the reader's store

**A second test, added 2026-08-13 after two conventions had already failed it.** `keyword-standards` said *"this store holds `budget` beside `context-budget`"* and `recall-before-storing` said *"this project's own store grew four such pairs"* — both true of the store they were drafted against, and **false for every other user of Scatterhoard**, who reads "this store" as a claim about their own.

The second one existed because the first was followed as precedent, which is how a one-off becomes a pattern. Caught on reading the ten-convention catalog end to end, which is a different act from drafting one entry.

**The fix is not to strip the evidence** — concreteness is what makes both rules land. It is to *illustrate rather than assert*: `budget` beside `context-budget` still appears, now as an example of what a split looks like rather than as a report on the reader's data. **The measurement itself belongs in the source comment**, which is never served and where a claim about this project's store is simply true.

The general form: **reinforcement is read by strangers.** Anything phrased as *this store*, *this project*, or a count of memories is a claim they can check and find false, and a convention caught being wrong about the reader loses authority on everything else it says.

### Why it isn't called "why"

A memory also has a [why](memory-data-model.md), and the two are similar — both help clarify when the thing they're attached to applies — but they are governed differently, and sharing a name would blur that:

- A convention's **reinforcement** is held to the reinforcement standard above. Conventions are hard-coded, reviewed, and served into every session, so their supporting text can be held to a fixed bar.
- A memory's **why** is not held to that standard. The LLM drafts it in the moment, and the memory's context is only used on recall, so there's room for more flexibility in what goes into it. It still does the work of letting a later reader reason about when the memory's factual content applies.

The field is therefore named `reinforcement`, which states the standard it has to meet.

The Golden Rule reaches runtime through this field (see below), which is why it must be served data rather than a comment — a comment would mean that decision silently never takes effect.

## Served: routing, capture, and export

Three conventions drafted and served 2026-08-12, taking the catalog from three to six. The scope note below said to keep the set small *until the basic system works*; that precondition is what these were waiting on, and the expansion is deliberate rather than drift. It uses most of what was then the budget — 7,465 characters of 8,000 — which made the next addition a decision about the budget as much as about the convention. That decision has since been taken: see [the budget is 16,000 characters](#resolved-the-budget-is-16000-characters).

### `route-where` — which set a memory belongs to

`store-when` already says to state the scope explicitly, and its reinforcement says why it must be explicit. **Neither says how to choose**, which is the gap the misfiling on this machine came through: of 26 memories in `local` on 2026-08-12, roughly eighteen were Scatterhoard project knowledge, six were general engineering lessons, one was a genuine working preference, and one was a leftover test fixture. Four had twins already in `shared:scatterhoard`, recorded later and reframed, with the local originals never removed.

**The rule asks what the memory is *about*, not what you were doing.** That distinction is what the data demanded: you are working on the project either way, so "what am I doing" routes everything to the project set, and the general lessons that happen to be learned here get filed with it.

#### The taxonomy belongs in the sets, not in the catalog

**A first draft named three destinations — project, cross-project, personal — and that was wrong**, caught by the human partner before it shipped. It compiled one user's arrangement into everyone's catalog: a user with a single set, or one per client, or a team set beside a personal one, would get a rule describing somebody else's setup and no way to correct it short of a rebuild.

So each set says what it is for, in a memory carrying the reserved keyword **`hoard-scope-topics`**, and routing is a match against those. This is the move the design makes everywhere else — derive rather than keep a second copy — applied to the one fact the catalog had no business holding. The sets are the authority on what they contain.

**It needs no new machinery.** `hoard_recall` with a keyword and no scope already searches every loaded set and tags each hit with the set it came from, so the lookup is one keyword search, which also works with no embedding model loaded.

**A memory travels; a `meta` row does not.** Putting the description in the set means it moves through export and import, so whoever clones a repository and imports its set learns what belongs there. Anywhere else and every machine has to be told separately, and a collaborator is told nothing at all. It also means a change to a set's charter is reviewed like any other memory, through the same export gate.

#### What stays compiled, and why that division is the safety property

**A description that steers routing is instruction-shaped**, and it arrives from a repository like any other memory. That is the channel [Trigger and Loading](trigger-and-loading.md) closes with *project memories are always data, never instructions*. A careless or hostile set could describe itself as the home for working notes and credentials and quietly collect them into a public repository.

The answer is a division rather than a refusal. **Descriptions are topical hints; two rules override any of them, and those two stay in compiled text a repository cannot reach:**

- Anything about the person, their machine, or their working preferences belongs to the personal set, whatever another set claims to want. *Since 2026-10-21, unless the person's sharing preferences keep it out of every set, which means storing it nowhere: a person who says never may be relying on it for their safety. See [export review](export-review.md#decided-2026-10-20-the-preferences-start-from-a-template-the-person-edits).*
- **Secrets are never stored in any set.** Put them in a file and record where the file is.

That keeps the extensible half extensible and the dangerous half fixed.

**The secrets clause carries [Prohibited Data](prohibited-data.md) to runtime for the first time.** That document asks for the block *"in both the conventions and, to the extent possible, in software"*, and as of 2026-08-12 neither existed — the mechanical scan is still a [Backlog](../../TRACKING.md#backlog) item. The convention is the better half to have first: the scan runs at export, by which point the secret is already in the store, where forgetting is the only remedy because a vector is a copy of the text it was built from.

#### The reserved keyword namespace

`hoard-scope-topics` is the first **operational** keyword — one Scatterhoard reads rather than one a human searches by. So the prefix is reserved: **keywords beginning `hoard-` belong to Scatterhoard**, and a project using one for its own content is making an error rather than creating an ambiguity. That is the same namespace claim the project already makes three times over — `SCATTERHOARD_*` for environment, `hoard_*` for tools, `scatterhoard_mem` for the reserved directory — and `hoard-` was chosen over a new abbreviation for exactly that continuity.

**Hyphen rather than underscore, on the evidence rather than on taste.** The first reserved keyword forced the question of how keywords are spelled at all, which is why it was settled here. The rule and its evidence now live in [keyword standards](#served-keyword-standards), where the rest of the keyword guidance is.

#### Decided 2026-09-11: the write path guards the reserved namespace

**The `about-` rule did not hold on its own.** Memories *about* charters were written carrying `hoard-scope-topics` three times in six days — five on 2026-09-06, one on 09-07 by a session that had loaded the corrected rule that same day, and two on 09-08 — and each round was delivered to every session as though it were a charter until something happened to look. Served text reaches a session before it writes; nothing stops the write. So two checks move into software, both at the authoring entry points — `memorize`, amendment and supersession — and **neither at import**, where sets written by other versions of Scatterhoard, and merges that legitimately disagree, must still load. The precedent for that split is the supersession-cycle refusal, which applies to an authored edge and not to a record reconciled from a file.

- **One live charter per set.** What [the memory data model](memory-data-model.md) described as a singleton becomes an invariant: a write that adds `hoard-scope-topics` while the set already has a live charter, and does not supersede that charter, is refused. The refusal names both ways out — supersede the charter to replace it, or use `about-hoard-scope-topics` for a memory about charters. A set's first charter, and a charter change made by supersession, are unaffected. It is the check that would have refused every instance on record. **Built 2026-09-14** as `one-live-charter-per-set` in [Backlog](../../TRACKING.md#backlog): `Store::insert` refuses, beside the cycle check, so `memorize` and amendment both reach it, while import writes through `accept_from_file` and never does. **`absorb` carries charters forward without it**, through `Store::carry_forward`: absorbing merges the other set's records by the import path, which can leave two live charters, and then supersedes this set's memories that linked into the absorbed set — the charter among them if it did. Refusing that replacement would stop an absorb with the records already merged. A set left holding two charters shows both in `hoard_sets status`, and the next charter written has to supersede both.
- **Unknown `hoard-` keywords are refused.** A keyword beginning `hoard-` must be a control mark Scatterhoard knows, or begin `hoard-proj-`, the carve-out a project's own tooling uses. This catches typos and invented marks — not the misuse above, since `hoard-scope-topics` is a legitimate mark. **Built 2026-09-14** as `reserved-keyword-block-list` in [Backlog](../../TRACKING.md#backlog): `CONTROL_MARKS` and `CONTROL_MARK_FAMILIES` in `src/store.rs` are the list, `Store::insert` refuses by it beside the charter check, and `tests/conventions.rs` fails if the served catalog names a mark the list does not know.
  - **Families are prefixes, not lists.** `hoard-lang-` joins `hoard-proj-`, and anything after either prefix is accepted: the embedding model is the user's choice, so the languages a set can be written in are too, and no list in one version could name them.
  - **Decided 2026-09-14 by the human partner: a term the set already holds counts as known**, on any memory, superseded ones included. Copies of a set run different versions of Scatterhoard, and keyword structure has to be able to change between them as `hoard-watch` gave way to the three watch-list marks. A check that knew only its own list would paint that into a corner twice over: an older version could not write a mark a newer one introduced, even after importing it, and a newer version could not amend a memory still carrying a mark it retired — twelve live memories in this project's set carry such terms. With the set vouching for its own terms, amending or superseding restates rather than introduces, and is never refused for a term it did not add; a retired mark stays valid wherever it already exists and simply stops being served. **The cost**: a typo that arrived before the check, or by import, stays writable in that set — the leniency import already needs — and a retired mark can still be put on new memories in a set that holds it, where the served `about-` rule is what stands against it.

**A dedicated tool for setting a charter was the other candidate, and was not taken**: it adds a tool for what supersession already does. The watch-list marks — `hoard-signpost`, `hoard-collect`, `hoard-investigation` — have no structural test that separates a list from a memory about lists, so they stay with the served `about-` rule. `hoard-author` does have a shape, since an identity record attributes itself, and was not part of this decision.

#### Fallback, bootstrap, and the split case

**Nothing fits: use the personal set, and say so.** It is the destination that cannot be published and can be corrected later, which is the right way for an unroutable memory to fail. Saying so is what gives the human the chance to correct the routing or create the missing set.

**A new set has nothing to match against**, so the convention also says to give a set its `hoard-scope-topics` memory when creating it. Placement follows the information: whoever creates a set knows what it is for, and nobody later does.

**One insight can need two memories.** A general lesson with a project-specific instance has no single right destination, and the convention says to store both rather than choose. This project's own store makes the case concrete: its largest single source of defects is a general principle whose evidence is entirely local to Scatterhoard. Two memories in `local` had already been split that way by instinct — their general halves sat beside project-framed twins in `shared:scatterhoard`, which is what the rule now prescribes.

### `capture-before-ending` — the moment, not just the content

`store-when` settles what is worth recording and cannot reach the session that never asks the question. This is that half, and it supersedes the narrower `articulate-after-operation` framing, which named only the end of a multi-turn operation.

**The trigger list is the substance.** The moments are those where the working context is about to be discarded: a piece of work finishing, a commit, an export, or the human signalling they are wrapping up or clearing context. What they share is that knowledge is cheapest to record while the context that produced it still exists, and unrecoverable afterwards — the boundary is where that cost flips.

**Recorded because it was observed rather than reasoned out:** the human partner's habit of saying *"I'm about to clear context, so if there's anything you'd like to capture, please do"* provoked additional captures reliably enough to become a habit. That is a tested trigger, and it is the one an LLM cannot detect for itself, which is why the convention names the human's signal alongside the moments the model can see.

**Writing the list before storing is the mechanism, not a formatting preference.** The model's own emitted text stays in context and acts as a commitment; an unstated intention to record something later does not survive a few turns. Saying it aloud also puts the human in a position to correct what was taken to be worth keeping, which is the cheapest review available.

**Amended 2026-08-20: that review runs after the storing, not before it.** As first drafted, the sentence above named a benefit without naming its timing, and a session read *"lets the human correct"* as licence to wait for approval before storing anything — an approval gate nobody wrote, in a convention whose own reinforcement explains two sentences earlier why postponing past the boundary is the failure it exists to catch. The correction is stated in the served text rather than left to be inferred, because a model should not have to re-derive the rule from an adjacent sentence to know it binds — the same failure [the Golden Rule](golden-rule.md) diagnoses in its own machinery, where clauses compiled from conclusions each misjudged a case the principle plainly covers.

**How it was found is the part worth keeping.** The human partner patched the behaviour with a personal memory, and then observed that doing so made their instance behave unlike one running the conventions alone. That is the hazard in general: a personal memory that overrides convention-governed behaviour leaves the convention broken for everyone else *and* costs the person tuning it the baseline they tune against. It bites hardest here because tuning a convention takes living with it across several sessions, so the confound corrupts a long observation rather than a single one — and because a memory only influences a session that happens to recall it, the deviation is intermittent, which is harder to reason about than a consistent one.

**"Nothing worth keeping" is stated as a valid answer on purpose.** A convention that demands a memory will get one. The catalog's standing worry is a session that records everything as much as one that records nothing, and the permissive phrasing is what stops the prompt manufacturing.

**The triggers above were reworded 2026-09-14**, around handing control back to the human after work that used tools, with the line required even when its answer is *nothing* — see [Served 2026-09-14](#served-2026-09-14-capture-before-ending-fires-before-handing-control-back). The reasoning in this section still describes why the entry exists; that one describes why its first trigger moved.

### `export-before-commit` — the export half, and only that

This is the convention previously described here as *"expose a fresh set of memories"* before committing, **split in two after that phrasing turned out to carry three things**: exporting the file, reviewing the resulting diff, and an unstated assumption that the memories worth exporting had been captured at all. The third is the one that hid: a convention that reads as though it produces memories can only ship ones that already exist, and reports success on an unchanged file when nothing was captured.

Capture now belongs to `capture-before-ending`, which fires at this boundary among others, so the two compose without either reaching for the other's job. Placement follows the information: **at commit time the knowledge has faded and you are reconstructing; at the end of the work it is fresh** — while the diff and the commit only exist here.

*The ambiguity had already propagated. [Export Review](export-review.md) depends on this convention for the moment at which a `git diff` of the memory file is available, which is the export reading — so the phrase was load-bearing in one sense while reading naturally in the other.*

**Routing failure and export failure turn out to be the same failure.** A local set is never published, so a project memory misfiled as personal is not merely untidy: this convention can never see it, and it stays on one machine permanently. That is what the eighteen misplaced memories in `local` are.

## Served: keyword standards

Drafted and served 2026-08-12, immediately after [the budget was raised](#resolved-the-budget-is-16000-characters) to make room for it, as `keyword-standards` in `src/conventions/catalog.rs`. It takes the catalog to 8,899 characters over seven conventions. It unblocks `investigation-pipeline` in [Backlog](../../TRACKING.md#backlog), which was waiting on it.

### The measurement it was drafted against

Both live sets on this machine, 2026-08-12: **109 distinct keywords over 34 memories, 4.7 per memory (range 3–6), and 71% of the distinct keywords used exactly once.**

**The singletons are the finding, because they divide sharply rather than uniformly.** One group earns it — `Keyword::folded`, `oneOf`, `ON-DELETE-CASCADE`, `keywords-silently-lowercased`, `serde_json` — each naming one identifiable thing precisely, which is the case exact-match search exists for and the case [semantic search is measured to be worst at](embeddings.md). The other group does not: `design`, `client`, `schema`, `debugging`, `rust`, `verification` — broad words coined once, gathering nothing and too vague for anyone to search on purpose.

**So the rule is not "avoid rare keywords", which would have discarded the half that works.** It is that a keyword does one of two jobs — *gather* this memory with others on its subject, or *name* one thing exactly — and a broad word appearing nowhere else does neither. That division is what the convention text encodes.

**The second measured failure is vocabulary drift.** This store holds `budget` and `context-budget` as separate singletons, and `mcp` (three uses) beside `mcp-clients` and `client`. A keyword only gathers when it matches the spelling already in use, so a second name for one idea splits a subject into two searches that each return half of it. Hence *recall the keyword before coining it* — a mechanism that costs one call and works with no embedding model loaded. *The `budget`/`context-budget` split was produced during this same session, an hour before the convention was drafted and with nothing in the catalog to prevent it, which is as fair a test of the need as the evidence gets.*

### Spelling, on the usage rather than on taste

`Keyword::new` permits either separator, rejecting only empty values and internal whitespace, so the code has no preference. The stores do: **50 hyphenated keywords against 3 underscored, and all three underscored ones — `hoard_memorize`, `scope_key`, `serde_json` — are identifiers quoted verbatim from code**, where a hyphen would misname the thing referred to. Eight more carry capitals for the same reason (`SQLite`, `Keyword::new`, `oneOf`), and none of the 109 contains a space.

**So the rule is not "always hyphen" but "hyphens for terms we coin, identifiers spelled as the code spells them"**, which is what the project had been doing without writing it down. Keeping an identifier's capitals costs nothing at lookup because `folded` is a separate column from the spelling as written — see `keywords-silently-lowercased` in [Issues](../../TRACKING.md#issues) for why those are two columns. *Supporting but not deciding: keywords reach the embedding — `embedding_input` is content, then `why`, then keywords — so their spelling is tokenizer input, and hyphenated compounds are ordinary in prose where underscores are not. Unmeasured, and it points the same way as the usage data.*

### What was left out

**No keyword count.** Practice already runs 3 to 6 with no rule saying so, so a number would describe sessions rather than change them — and by [decision placement](decision-placement.md), a choice whose answer changes nothing should not exist. How many keywords a memory needs is information that lives with the memory being written.

**No rule against keying a memory by a person's name.** It was drafted and rejected by the human partner, and the reasoning is recorded because the clause is tempting. The argument for it was that keywords add *aggregation* to the three hazards [the Golden Rule](golden-rule.md) names — six scattered observations keyed by a colleague's name become a dossier retrievable in one call.

Two things defeat it:

- **The legitimate uses are ordinary.** `jbrown` is the right keyword on *"the third pull request from jbrown carrying this error"*, on *"jbrown is the subject matter expert on XYZ"*, and on *"jbrown maintains XYZ"*. A ban would have blocked the index while leaving the record untouched.
- **The dossier is prevented by routing, not by spelling.** A topical set holds memories about its topic, so a name recalled there returns what someone did on that project rather than who they are. The two cases where that fails are the personal set, which is the catch-all but never travels — so the Golden Rule's *it travels* mechanic does not apply — and a set that declares itself for dossiers on people, which a keyword rule would not stop anyway.

**The content is the target, and `store-when` is already aiming there**: *record the specific observation rather than a characterisation of them*. This is [decision placement](decision-placement.md) applied to a safeguard — the keyword layer knows how a term is spelled and not what it aggregates, so a rule placed there is a rule placed where the information is not. The residue was an open question about a set whose declared charter is people, which is a `route-where` question rather than a keyword one; it is **resolved below** as `route-where`'s third override.

*The `store-when` phrasing quoted above was itself replaced the next day — see [the person clauses](#resolved-the-person-clauses-recompiled-from-the-principle). The rejection recorded here stands unchanged; it was the clause's **placement** that was right, and its wording that was not.*

## Resolved: the person clauses, recompiled from the principle

**Three open questions were settled together on 2026-08-13, because they were one question.** They were: whether the Golden Rule should reach runtime more overtly, whether `store-when`'s person clause was calibrated on the right axis, and whether the design should say anything about a set whose declared charter is people. The human partner's read at the end of 2026-08-12 was that these shared an unsurfaced root, and that turned out to be right — **the clauses carrying the principle had each been compiled from its *conclusions*, and the derivation had never been written down.** It is written down now, as four demands, in [the Golden Rule](golden-rule.md#what-the-principle-is-for-four-demands-it-makes). This section records what changed in the catalog; the reasoning lives there.

**Two conventions were revised, taking the catalog from 8,899 to 10,195 characters of 16,000.**

### `store-when`'s person clause, replaced

The old clause — *record the specific observation rather than a characterisation of them* — carried the verdict and lost the axis, so it read *"jbrown is the subject matter expert on XYZ"* as suspect when the hazard is an unfavourable claim on thin evidence. The new clause states the demands directly: record what a later session's work needs, record what you observed rather than what you concluded, name where it can be checked, and — before generalising — account for what you do not know, recording the observation rather than the generalisation where the unknowns are significant, and reading those unknowns in favour of the person's motives.

**Two decisions inside that are worth keeping deliberate.**

*It restates rules the convention already makes, rather than referring to them.* The answerability and earn-its-place demands are the same two the convention opens with — the person clause is those demands applied without the exemption a judgement about someone quietly gets. That was the elegant finding in the derivation and it is exactly what **must not** be left implicit: the human partner's constraint was that a weaker model, without an extended reasoning session, will not make that leap. The design document may derive; the served text must state.

*The scope is any reasoning being, an LLM included.* Not an extension of the principle but its stated scope — it already reads *accounting for the fact that humans and LLMs may want different things*, and [Origin](memory-data-model.md) is already an application of it to a model.

**The reinforcement now carries the boundary into served text for the first time**: *this is charity about motive only; it says nothing about whether the tests pass.* That sentence is the runtime answer to the standing objection that a kindness directive softens technical criticism, and it is load-bearing rather than decorative.

### `route-where` gains a third override

**No shared set takes people as its subject, whatever its description claims** — a memory about a person goes with the work it concerns, or to the personal set.

This closes the hole the rejected keyword clause could never have closed. **Aggregation harm is a property of the set, not of any record in it**: a dossier can be assembled entirely from individually answerable, work-serving memories, so no rule examining one memory at a time can see it, and routing is the only layer that can. That is why it joins the compiled overrides rather than the topical descriptions — this is the one description whose *faithful* execution produces the artefact the principle exists to prevent, so it has to sit in text a repository cannot reach.

**Scoped to shared sets on purpose.** The personal set is the acknowledged catch-all and can accumulate cross-topic memories about one person, but it never travels, so the durability and reach mechanics are weaker on one person's own machine — and widening the override to every set would forbid the fallback the convention's own next sentence depends on.

**One consequence is accepted rather than overlooked: this forbids a *team* set whose charter is personnel notes.** The open question weighed that against the transparency argument — a charter is visible by design and reviewed like any other memory, and a tool refusing to hold personnel notes is making a policy choice its users did not ask for. It was settled the other way, and this is the paragraph to revisit if that trade ever looks wrong.

### What this did *not* change

**The Golden Rule stays out of the served catalog as a standalone convention**, and the question is answered more firmly than before rather than left open. The mechanism — reinforcement text on the memory-writing conventions — was never what failed. What failed was compiling from conclusions with no derivation to compile from. The reasoning, including why the **residue is an argument for this arrangement rather than a gap in it**, is in [How this reaches runtime](golden-rule.md#how-this-reaches-runtime). The optional-convention shape remains open for when that mechanism exists.

## Served: recall before storing

Drafted and served 2026-08-13 as `recall-before-storing` in `src/conventions/catalog.rs`, taking the catalog to 11,619 characters over eight conventions. It closes a gap neither neighbouring convention reached: `recall-when` fires before *acting*, and `store-when` assumes the question is being asked for the first time.

**Second in catalog order, and the position is the argument.** The front of the catalog is the sequence of questions asked while writing one memory — is this worth recording (`store-when`), is it already recorded (here), which set (`route-where`), how is it indexed (`keyword-standards`). This question only arises once the first is answered yes, and it must be settled before the two that follow, because **a memory that turns out to be a correction is routed and keyed by the record it corrects** rather than on its own.

### Recall-and-read, not a tool that warns

Decided 2026-08-13 by the human partner, and the deciding argument is **half-overlaps**. As a corpus grows the common case stops being duplicate-or-not: a new memory partly overlaps an existing one, and the right response is then to **split or correct the memory already there** rather than to decide whether to store.

That reframes what the software can contribute. **Proximity is all a vector knows**, and a duplicate, a reframed twin, a deliberate supersession and a partial overlap are indistinguishable from a distance — a supersession is *supposed* to resemble its predecessor. So a tool that warns asserts a relationship it has no information about, which is [decision placement](decision-placement.md)'s *a control that looks usable but isn't gets used, and used wrongly*. Software surfaces the neighbours; the session reads them.

**This is also what unblocks the software half rather than deferring it.** A warning needs a threshold, and a threshold inherits `no-non-match-floor` and the [cold-start scoring question](../../TRACKING.md#open-questions) — measured separation here is thin enough that any constant would be arbitrary. Surfacing the nearest few asserts nothing and needs no floor. Declining to make the claim removes the dependency. The store-time surfacing remains unbuilt and is tracked in [Backlog](../../TRACKING.md#backlog).

### It points at `correct-when` rather than restating it

Most of the outcomes — supersede, amend, forget — already belong to [the correcting convention](#served-the-correcting-convention). **This is not in tension with *served text must state rather than derive***: that rule is about not making a reader reconstruct an omitted derivation, and `correct-when` is served in the same catalog, in front of the reader. **Restate what is absent; point at what is present.** The pull while drafting was to inline it, which is why the distinction is written down.

### The evidence, and why the remedy is a reading step

This project's own store grew **four pairs of twins** — memories in `local` with counterparts in `shared:scatterhoard`, recorded later and reframed, the local originals never removed (see [routing, capture, and export](#served-routing-capture-and-export)). **Reframing is exactly what let them survive every mechanical check**, which is the argument against a comparison-based remedy and for a reading one.

**Its "split it" clause depends on [`split-when-partly-stale`](#served-splitting-a-partly-stale-memory)**, which was drafted the same day for that reason: without it, this convention instructs a session to split with no rule for how, and the natural reading — amend or replace the original — is the one outcome that strands cross-set links. That convention also defers splitting to detection time, on the grounds that at store time nobody can predict which half of a memory will rot first, and a store-time recall returning a partial overlap **is** detection, satisfying its own test.

## Served: splitting a partly-stale memory

Drafted and served 2026-08-13 as `split-when-partly-stale`, immediately after `correct-when`, taking the catalog to 12,959 characters over nine conventions. **Served under a different id than the backlog's `splitting-partly-stale`**, so it matches the `<verb>-when` shape the rest of the catalog uses and reads as `correct-when`'s sibling at the point of use — the same rename precedent as `articulate-after-operation` → `capture-before-ending`.

**It was drafted next because [`recall-before-storing`](#served-recall-before-storing) created a dependency on it.** That convention's text tells a session to *split it* when a new memory only partly overlaps an existing one, and until this one existed, nothing in the catalog said how. The natural reading of that instruction — amend or replace the original — is precisely the outcome that strands links. A convention that instructs an operation the catalog does not define is the same failure that kept `correct-when` withheld while `amend` and `forget` did not exist, arriving from the other direction.

### Adjacency to `correct-when` is deliberate

`correct-when` says a changed claim is superseded by storing *"the corrected memory"* — **singular**, which is wrong when only part of the memory changed. This convention is a refinement of that branch rather than a separate operation, and serving them adjacently is what makes them read as one rule. Separated, a session applies the general form and bundles a half-stale correction into a single record.

### Leaving the original in place is the load-bearing half

Links elsewhere point at the original's id, and **a link held in another set cannot be rewritten from here** — the memory holding it may live in a repository this machine has no business committing to. Keeping the original means those references still resolve and traversal follows the supersession forward to the parts. Deleting or amending it strands them.

This is the clause most likely to be "simplified" away on tidiness grounds, which is why the source comment says not to.

### Why detection time rather than a rule against bundling

At store time nobody can predict which half of a memory will rot first, so *keep observations and explanations separate* asks the writer for a judgement they cannot make; once one half has gone stale the answer is already known. That is [decision placement](decision-placement.md) — and it is also why the store-time recall introduced by `recall-before-storing` reconciles with it rather than contradicting it: that recall is a *detection* moment, not a writing-time guess.

**The cost of bundling is measured, not assumed.** In the 2026-08-01 link-label corpus test, `SUPERSEDES` versus `CONTRADICTS` was unambiguous only where an observation and its explanation had been stored as two memories; bundled, a later correction partly contradicted and partly superseded, and neither label fit.

### The shared-set sentence is a constraint, not a flourish

A correction to a project-scoped memory is an **export-visible edit** that goes through the review gate and a human maintainer, and the convention must not read as licensing a local fix-up. The trust direction itself is fine — local knowledge correcting an imported memory is the permitted direction, not the forbidden one.

## Served: confidence standards

Drafted and served 2026-08-13 as `confidence-standards`, fifth in catalog order, immediately after `keyword-standards`. **Renamed from the backlog's `confidence-guidance`** to pair with its neighbour: these are the two field-level conventions that [the `store-when` editing comment](#reinforcement-and-the-comments-beside-it) predicted would arrive separately rather than by growing that entry, and they do the same job of saying what a field is for.

### The measurement, refreshed

**Across both live sets on 2026-08-13: 44 memories carrying five distinct values** — 0.8 three times, 0.85 twelve, 0.9 eighteen, 0.95 eight, 1.0 three — **mean 0.888, and nothing below 0.8 ever written** in roughly two weeks of use.

The 2026-08-12 measurement was 37 memories and the *same five values*, so seven further memories introduced none. That is the finding strengthening rather than merely persisting: the field is not slowly acquiring range, and a scale whose entire observed span is its top fifth costs a judgement at every store and returns nothing to the reader.

### Anchored on evidence, not on feeling

**This is the whole of the calibration.** How you came by a claim is a fact about the claim; how sure you feel is a fact about you, and two sessions will not agree on the second. The entry's built-when asked for ends concrete enough that two sessions land within a band of each other on the same memory, and the ladder — watched it happen, measured or decided, inferred from evidence that fits without compelling, a reading you would check first — is what answers it.

**The numbers are anchors, not thresholds.** Nothing branches on them; the served text says *near* deliberately. The consumer today is the reading session deciding how far to lean without re-checking, and [graph traversal](../../TRACKING.md#open-questions) will consume the field mechanically once it exists.

### Two clauses that must not be dropped

**Recalibration is an amendment.** [Corrections](corrections.md) settles that changing confidence on an identical claim is apparatus rather than a new belief. Without the clause, a session that becomes less sure reaches for supersession — the general rule — and fills a chain with records nobody ever believed differently.

**A low number is not permission.** This convention opens a laundering route by its own existence: once the low end is usable, *record it at 0.4* becomes an available answer to a memory `store-when` says to leave out — in particular the unfavourable generalisation that [demand D](golden-rule.md#what-the-principle-is-for-four-demands-it-makes) removes. The last sentence of the served text closes it.

*[The Golden Rule](golden-rule.md) carried exactly that error in its own machinery list until this convention was drafted: its confidence bullet still said an uncharitable generalisation should be recorded as a low-confidence claim, which demand D had contradicted hours earlier without anyone noticing. Same compiled-from-conclusions drift, inside the document that names it.*

## Resolved 2026-08-24: a charter is a description, not a catalogue — and `store-when` loses its enumeration

**Two edits, one rule, landed together because they are the same rule at two layers: a list of kinds must never stand in place of the criterion it illustrates.**

### What the measurement showed, and how it corrected the question

[Open questions](../../TRACKING.md#open-questions) asked whether `store-when`'s enumeration should name *method* alongside knowledge, raised by a measurement procedure that went unrecorded in 2026-08-15. Measured 2026-08-24 across the two live shared sets — 105 memories — **the enumeration was not what suppressed it**. `coding-lessons` is dominated by method under the same `store-when`, so the convention plainly permits it.

What differs is each set's `hoard-scope-topics` charter, which [`route-where`](#route-where--which-set-a-memory-belongs-to) sends every candidate to be matched against. And the difference is not in the charters' *criteria*: both carry a belongs-here test, and neither test mentions kinds at all. It is in the **descriptive gloss** each opens with — one names a subject with a method-shaped half, the other names four kinds of conclusion — and the corpora follow the glosses rather than the criteria.

**So a charter does two jobs and only one of them is written deliberately.** The boundary test decides *which set*; the description silently decides *which kinds of thing get written down at all*. That is what the expanded clause is aimed at, and it is why guidance aimed at the belongs-here test would have changed nothing: authors already write good ones.

### Why the guidance cannot name the kinds to include

**The first `route-where` draft named three destinations — project, cross-project, personal — and was rejected for compiling one user's arrangement into everyone's catalog.** Guidance saying *remember to include methods* is the same mistake one layer down: a taxonomy of kinds rather than of destinations, and the next author would omit a different category. So the rule is shaped as **do not let a list stand as the description**, never as a list of what to include.

That also settles the `store-when` question in the direction the open question suspected. The enumeration is **cut rather than extended**: the opening clause already covers anything a later session would spend real effort working out again — a method and a path to a conclusion included — and five examples inside the rule narrow it by implication. Adding items chases one category at a time, and the list can only ever hold the categories somebody thought of.

### The cut exposed a hole, and `learn or decide` closes it

**The enumeration's first item was "a decision and what it rules out" — the one place the served text said a decision counts.** Cutting it left `learn` as the sole gate, and a decision the session *made* is not something it learned. So the opening clause now reads **"when you learn or decide something"**, and this is the other half of the cut rather than a separate change.

**The evidence is close to a controlled experiment.** The catalog budget was raised twice. The raise that was *learned* — read out of a handoff note while restoring context — was stored as a memory the same evening. The raise the session *made* mid-drafting was not, and the corpus held the withdrawn figure for eleven days, until the human partner contradicted a number quoted from this document on 2026-08-24. Same convention, same session length, opposite outcomes, and the fact fell on opposite sides of that verb.

**Deliberately not fixed by naming where the decision went instead.** It went into a session handoff note, which is this project's practice and not Scatterhoard's — a stock install has none. Strip that away and the failure survives in a commit message or an issue: every session has somewhere its narrative goes, and whatever that is absorbs a decision, because a decision reads as *what happened*. Naming one of them would compile one user's process into everyone's catalog, which is [the mistake this convention set already records being caught at](#the-taxonomy-belongs-in-the-sets-not-in-the-catalog). The reinforcement names the pull without naming the place.

### `correct-when` gains the moment it was missing

Its opening was *"When a memory turns out to need changing"* — agentless, naming no moment, so it never fired on the case that matters: **you change the thing a memory describes, the change succeeds, and nothing about it mentions the memory.** It now opens *"When you change something a memory describes, or a memory otherwise turns out to need changing"*.

`recall-before-storing` cannot cover this and it is worth saying why: that check runs before a *store*, and an invalidation stores nothing. **An addition passes through the machinery; a change to the world goes around it.** That asymmetry is the reinforcement's new closing sentence, and it is why three correct copies of the new budget in this repository sat beside one confidently wrong memory.

### Extended `route-where` rather than drafted as its own convention

Decided on the budget. A standalone charter convention costs roughly 1,430 characters against 1,235 for the extension, and `investigation-pipeline` is still to draft and was expected to run above the 1,461-character average — it is the convention [the raise to 18,000](#resolved-the-budget-is-18000-characters) was made for, and charter guidance should not be what displaces it. The catalog stands at **16,746 of 18,000** after all three edits in this change.

**That leaves 1,254, and `investigation-pipeline` was expected to run above the 1,461-character average — so it no longer fits, and this is the decision the budget exists to force rather than a detail to absorb.** Three honest answers: trim these three conventions against each other, which is precisely what [the first raise](#resolved-the-budget-is-16000-characters) was made to stop; draft `investigation-pipeline` smaller than its estimate and check the estimate was not just pessimism; or raise the budget a third time, which has to clear the standard the second raise set — *well under 4% of the smallest context window in range* — and has to answer the standing objection recorded there, that **a budget raised whenever it binds is not a budget**. Both raises so far were argued from what the number spends, and a third argued from what will not fit would be the first of a different kind.

**If trimming is the answer, `route-where` at 3,355 characters is the largest convention and the charter material is its natural seam.**

**The cost is real and should be watched:** `route-where` is now the largest convention in the catalog at 3,355 characters, which is precisely the condition that made `store-when`'s size a question. If it needs splitting later, the charter material is the natural seam.

### Revised 2026-08-25 by applying it, which is what found it wrong

**The list rule overshot, and the passing case was in hand the whole time.** As first drafted it read *"describe the subject rather than listing the kinds of record it takes"* — which condemns `coding-lessons`, the charter that **works**: it leads with a subject gloss and then lists, and it collects the kinds its list names. The measured failure was never a list; it was a list that **leads** and stands in place of the subject. The clause now permits an illustrating list and forbids a leading one.

Worth recording how the overshoot happened, because it is cheap to repeat: **the rule was drafted from the failing case alone.** Both charters were measured, both were quoted in the section above, and the rule was still written as though only one of them existed.

**A charter sentence moved into the convention, and the test is who would need it.** The `shared:scatterhoard` charter carried *"a lesson that would hold anywhere belongs elsewhere, even when Scatterhoard is where it was learned, though the Scatterhoard evidence for such a lesson does belong here."* Raised by the human partner: is it doing significant work, and would a set someone else creates have anything like it?

It is doing real work — the corpus shows the split being made in both directions, general form to `coding-lessons` and project evidence here. But **it is not a fact about this set.** It is a routing rule every set would need, and nobody writing their first charter would think to include it, so every set created without it routes general lessons into a project set — which is [the misfiling `route-where` was created to fix](#route-where--which-set-a-memory-belongs-to). So it now lives in the convention, absorbed into the existing two-memories clause.

**This is the taxonomy argument run in the other direction.** What varies by set belongs in the charter; what is identical for every set belongs in the catalog. Both directions are worth stating, because the charter is the tempting home for anything that reads as routing — and a general rule kept there is not a copy that drifts but a copy every other set silently lacks, which nothing reports.

### The charter this was drafted from, rewritten — and the other left alone

`shared:scatterhoard`'s charter now leads with its subject: *"how it works, how it came to be as it is, and how the work on it is done."* That last clause is what admits a measurement procedure without naming *method* as a category, since naming categories is what built the whitelist in the first place.

**`coding-lessons` is deliberately unchanged, as the control.** It is the reason the finding was measurable, and rewriting both would leave nothing to compare against. The two sets' intake over the next several sessions is the evidence for whether the rewrite did anything.

### What is not settled here

Whether `hoard_sets create` should refuse to make a shared set without a charter — filed in [Open questions](../../TRACKING.md#open-questions). The convention can only ask; the tool surface is where it could be guaranteed.

## Retired 2026-09-04: the recall-gloss convention, which is two rules and neither needs the catalog

This section stood for weeks as *To draft: recall-gloss convention*, holding two things: that the LLM should lead with a gloss built from Context when surfacing a recalled memory, and that there should be guidance on what is worth capturing *into* Context in the first place. **Both have left the catalog, in opposite directions, and the split is the finding.**

**The capture half went to the field's own description**, where [decision placement](decision-placement.md) puts a decision made with the schema in front of you. The evidence is that the description is what actually drives the behaviour: the previous wording produced **98% adoption of an optional field** and then returned its own vocabulary — dates, occasions, and the checkability pointers it implicitly licensed by saying *not searched*. A rewrite of that string reaches every client, is pinned by [tests/snapshots/tools_list.json](../../tests/snapshots/tools_list.json), and costs nothing against the 18,000-character budget. The full measurement and the new text are in [the Context field resolution](memory-data-model.md#resolved-context-field).

**The gloss half is read-side, and has nowhere to live yet.** It is a rule about what to do with a memory recall has already returned, so it belongs with the read-side documentation of the field — which does not exist, because no tool publishes an `outputSchema` (`response-shape-is-unpublished` in [Issues](../../TRACKING.md#issues), where `context` is named as one of the fields that appears only when non-empty). Fixing that issue creates the home.

**Always or on demand: the middle option is the one to refuse.** Glossing only when the human is struggling was raised 2026-09-04 by the human partner as possibly sufficient. It survives the trigger test, because *them asking* is externally observable — while *anticipating* that they might not place it is self-assessed, and [a standing instruction firing on a boundary the model must notice for itself does not fire](conventions.md). So on-demand and always are both workable and "gloss when it seems needed" is not, since it degrades silently to never.

**What tipped it away from a convention is that the good behaviour turned out to be nearly free.** The one recorded observation of this working — two references to a stored memory minutes apart, one placed immediately and one not — shows the successful one was not a gloss appended to a reference but the reference *made well*, leading with a fragment of the memory's own situation. The failure said "your standing request" and named nothing. That is a property of how a memory is referred to, statable in a sentence beside the field, rather than a standing behaviour needing served text.

**The behaviour is not gone in the meantime, and that is easy to miss.** `recall-when`'s final served clause already says it — *"When you surface a recalled memory, lead with enough of its context to let the human place it before you rely on it"* — and the source comment above it in [catalog.rs](../../src/conventions/catalog.rs) names it as the recall-gloss behaviour. So retiring the standalone convention removes a planned entry, not a live rule. **That clause is phrased as always**, which is the choice left open above; it is kept as it stands for now, because it is one sentence and it is the only channel that reaches a session at recall time until the read-side documentation exists. Revisit it then, not before — softening it first would leave the behaviour with nowhere to live at all.

**If the optional-conventions mechanism is built, a more continuous version belongs there as an option** — noted 2026-09-04 by the human partner. That is the placement the [add-ons question](../../TRACKING.md#open-questions) already reaches for this case: always-glossing is output styling some users will feel strongly about and should be able to decline.

**What it frees.** Three Ready conventions were competing for the remaining budget; this removes one of them without dropping anything it was going to do, leaving the room to `hoard-watch` and `investigation-pipeline`.

## Served 2026-09-04: `hoard-watch`, which absorbed the investigation pipeline

Drafted with the human partner across one session, from the four-question agenda recorded on `investigation-pipeline` in [Backlog](../../TRACKING.md#backlog). **The headline is that two Ready conventions became one, which is what made it affordable.**

**A seed and a trap are the same object.** The agenda framed them as differing in trigger — a trap fires because you touched an artifact, a seed because a subject came up — and that distinction does not survive contact with the shape: `hoard-watch` is session-start push, so neither fires at those moments. What differs is what the memory asks of you once you have it, which is furniture rather than machinery. So one keyword marks *push me*, and a second names the kind.

**Three kinds, and the boundary is where the verb changes.** `hoard-signpost` (recall before reasoning about its subject), `hoard-collect` (add findings that fit), `hoard-investigation` (add evidence for or against a hypothesis). The human partner's own worked example is what settled it: a bare collection of tool-call failures, then *"how can I deal better with failing tool calls"*, then *"I hypothesize improved warnings would reduce the frequency."* The middle one — the puzzle — takes the same action as the first with **narrower fit criteria**, and fit criteria are content. Only the hypothesis changes what you do. So the puzzle stage is real and is carried by the memory's own text rather than by vocabulary, which keeps a stage out of the catalog that would otherwise have to be defined there.

**Advancement is spawning, not superseding, and this corrects [the re-entry question](../../TRACKING.md#open-questions).** That question reasoned that transitions are supersessions with the state as the chain tip. It cannot be: one observation set can drive several questions, and supersession affords one successor. Representing several would mean deliberately forking, which means "two people advanced this independently" — and would walk into `a-healed-fork-reports-its-ancestor-as-a-cycle` in [Issues](../../TRACKING.md#issues) the moment anyone merged. So a question is a **new head that draws on the observations**, the observation set carries on collecting, and supersession keeps two jobs: correcting a head, and retiring one.

**Observations are members, not heads.** Pushing every observation would break the budget the design rests on, since observations are the class that grows without bound. Membership is an ordinary precise keyword the head names — the issue-slug pattern already in use, where `malformed-tool-call-damage-is-stored-silently` gathers four memories across two sets — so many-to-many falls out of keywords being multi-valued, and no new link labels are needed. `results_from` carries question-from-observations, `resolves` carries investigation-answers-question.

### The two rules that keep the push list bounded

**Marking is the exception, not the default**, and this is the budget story push had been missing. Most accumulation stays passive and is reached by ordinary recall, costing the session-start list nothing. `hoard-watch` is for what you would never think to search for. The human partner's two examples land on opposite sides of exactly that line: a recurring crash is describable when it recurs, so it needs no mark, while *what shape was that question* is unsayable and does. Keeping things off the list is a better bound than an eviction rule for removing them later.

**Marking obliges you to say what would end it**, and the obligation attaches to the keyword rather than to collections generally — the thing that spends the scarce resource is the thing that must say when it stops. It takes a different shape per kind, and in each case it is what that kind is worst at judging later: the subject a signpost depends on, what would count as enough for a collect, the decision that turns on an investigation's answer. The three named shapes also do the work an abstract *must be checkable* standard would do, more concretely.

**Retirement is dropping `hoard-watch` on the superseding memory. It is cheap and reversible** — the memory stays, recall still finds it, and a later case re-opens it by superseding with the keyword back on — which is why the convention says to retire on the evidence you have rather than waiting for certainty.

### Where the ending is judged from, which is the part that needed working out

Three conditions retire a head: the subject is gone, the cause is resolved, or the collection has saturated. The first two are external events, which is why a crash collection feels easy to close. **Saturation is the hard one, and measuring it inside Scatterhoard is a trap the human partner identified**: a rate needs a denominator, and recording redundant instances means either a memory per instance — which `recall-before-storing` exists to prevent — or a supersession chain bumping a counter, which is the chain-of-records-nobody-believed-differently objection that already killed amendment-for-confidence.

**So Scatterhoard does not hold the denominator.** A collect's members are distinct findings; instance counts belong where events already live. This is the same boundary that put defects in [Issues](../../TRACKING.md#issues) because the natural home for a known defect is an issue tracker.

**And the rate is mostly unnecessary, because the conventions already create the moment saturation is visible.** A session meets an instance, recalls before storing, and finds the collection already covers it — that session has experienced one unit of redundancy first-hand, and nobody reading a stored counter is better placed to judge. Hence the clause added to `recall-before-storing`: *if what you found is an active collection, having nothing to add is evidence toward the ending it declares.* It defers to the head's declared ending rather than restating the taxonomy, which matters because redundancy tells you a **pattern hunt** is finished and says nothing about whether a **question** is answered — the tenth confirmation may be what the question needed. A question-bearing head declares a decision, so a repeat does not touch it.

### What the ossification rule cost, and what it did not

Agenda item 4 asked what stops a seed hardening, since a seed accumulates fitting cases by construction. The rule it invoked — `01M1JX9FD7FMH96QF48F5VHMG7`, *record the best case against the thing you are recording* — did not earn a standalone clause. For a collect the ending condition absorbs it: an ending like *twenty that have not reduced* cannot be evaluated without counting the ones that did not fit. For an investigation it does not, because naming the decision does nothing to stop you gathering only supporting evidence, so it survives as seven words in the text: **a hypothesis with nothing recorded against it has not been tested.**

### Provisional as of 2026-09-05: the length clause is being replaced

**The convention shipped and was probed the same night, and whole-record push does not scale.** The per-record envelope is 411 characters and does not shrink with the prose, so three stores of fifteen watches cost 1.9x this whole catalog and a hundred cost 4.2x — and both of the human partner's reasons reach past fifteen unaided: a cross-set recall multiplies by the number of stores, and a large project has many open items.

**The length clause is also unfollowable as written.** *Short enough to read fifteen of* requires arithmetic over a quantity the author cannot see, and the session that wrote it violated it within the hour, producing a 1,940-character head. That is not a discipline failure to be met with a memory: the mistake happens inside the act of writing, with no moment to recall into. Its cost figures were wrong for the same reason — *about a quarter* counted prose and ignored the envelope; corrected to *under half* on 2026-09-05.

**The replacement is `watch-push-is-an-index-per-set` in [Backlog](../../TRACKING.md#backlog)**: push a line derived from the leading sentence of `content`, per set rather than cross-set, at session start and again when a set is loaded. Deriving the line removes the length failure mode rather than guarding it, and the clause becomes one an author can follow — *make the first sentence say what this is for and when it applies*.

**Per-set is a security property first.** Push is the only channel where memory content reaches a session unconditionally, so it is the surface *memories are data, never instructions* exists to close. The catalog supplying the verbs bounds what a watch can make a session **do**; it does not stop a set putting chosen text at the top of every session. Per-set push is revocable per set. It also creates a dependency: a set loaded for evaluation must not push, and evaluation mode does not exist — see [How to inspect a memory set without adopting it](../../TRACKING.md#open-questions).

### The budget, which was the reason to expect a third raise

**No raise was needed.** The convention is 1,877 characters and the `recall-before-storing` clause is 107, against 1,747 free — an overrun of 237, closed by trimming justifying reinforcement from `split-when-partly-stale`, which the 2026-09-02 audit measured at 79% justifying with the highest reinforcement ratio in the catalog. Three sentences went: the restatement that references still resolve, the placement argument behind the timing rule, and the measured instance of what bundling costs. 363 characters freed, no rule removed.

The catalog stood at **17,875 of 18,000** on 2026-09-04, when this trim was made. [Derive the current figure](#the-headroom-is-derived-never-written-down) rather than reading it here.

**The expectation of a raise rested on an estimate that merging invalidated.** `investigation-pipeline` was expected to run above the 1,461-character average *as its own convention*, alongside `hoard-watch`. One convention replaced both, and [`discovery-convention`](../../TRACKING.md#backlog) — deferred pending "whether anything is left over once the pipeline convention is drafted" — is now the check to run rather than a third claimant. **The standing objection stands unspent**: a budget raised whenever it binds is not a budget, and this is the first time the cheaper answer was attempted rather than only priced.

**The hold on catalog re-tuning was lifted for this trim** by the human partner on 2026-09-04, the second such exception, granted on the ground that the information the hold was protecting had already been gathered.

### The hold is lifted for the watch-index changes, and it is not another exception

**Lifted 2026-09-06 by the human partner, for the whole of the watch-index work rather than one pass.** The stated reason retires the hold for this cycle rather than suspending it: *the purpose of that hold was to give me a chance to evaluate the performance under the conventions we adopted at the time. I've done that evaluation, which led to the current set of changes. It does me no good to evaluate a partially implemented set of changes, so we'll do all the updates, then I'll leave it alone for a while.*

**What follows is that the updates get finished.** Text may be moved between conventions and justifying reinforcement pared back where the arithmetic needs it — the swap is not a trim of one entry to fund another, which is what [the first raise](#resolved-the-budget-is-16000-characters) was made to stop, but a rewrite of one mechanism spread across several entries. **A quiet period begins when the changes land**, which is what the hold is *for*: a measurement is worth having against text that will be kept, and a half-implemented change is text nobody will keep.

**One thing this does not say, corrected 2026-09-06 after it was written down wrongly.** An earlier version of this section read *"anything discovered mid-cycle that would edit the catalog again belongs in BACKLOG for after the quiet period, not in the same pass"* — attributed to the human partner and invented by the session. It inverts the instruction: *do all the updates* means a change found while making them is part of the work, not a deferral. The quiet period is what comes **after** the cycle, and nothing was said about narrowing what the cycle contains.

**What was trimmed will be named**, entry by entry with the character count, in the same place this section records the 363-character trim above. A trim that nobody can find later is indistinguishable from a rule quietly dropped.

## Served 2026-09-06: `hoard-watch` rewritten around the index, and what funded it

The mechanism is [`watch-push-is-an-index-per-set`](../../TRACKING.md#backlog), and it followed the code rather than preceding it — the projection shipped first, and this is the convention catching up. Three changes to the served text, one correction to the plan, and three trims to pay for them.

**What is pushed is the opening line, so the length rule is replaced rather than tightened.** *Keep a marked memory short enough to read fifteen of* demanded arithmetic over a serialized envelope the author never sees, and the session that wrote it broke it within the hour. It becomes **make its opening 120 characters say what it is for and when it applies** — a bound the author can see, and one both write paths now show them: `hoard_memorize` and `hoard_correct`'s `amend` echo the derived line. The reinforcement's cost arithmetic went with the rule it justified: it priced pushing *records*, and under an index a long memory costs what a short one does.

**The trigger did not move, because the place it was to move to does not do what the entry said.** [`watch-push-is-an-index-per-set`](../../TRACKING.md#backlog) had the watch recall joining `route-where`'s `hoard-scope-topics` recall, *"per set at startup"*. Read as served, that recall happens **when routing a memory**, in one call across every set — neither at startup nor per set. The only session-start instruction in the catalog is `recall-when`'s, which is where the clause already was, and it stays there. `route-where`'s text is untouched.

**What the clause says, and why each part of it.** *At the start of a session, and whenever a writable set is loaded, recall `hoard-signpost`, `hoard-collect` and `hoard-investigation` as summaries, one keyword and one set per call.*

- **One keyword per call, not three.** A summary hit is an id and a line, so a mixed list cannot say *which* action each entry wants — the three kinds ask for three different things. The call is what carries that, so asking one keyword at a time is what makes the returned list actionable. It costs three calls per set instead of one, and the human partner's judgement is that this is cheaper than labelling every entry inline. **This is the same defect as the marker fix below, met a second time**: the projection drops the field a rule was going to read.
- **One set per call**, for the reasons the reinforcement gives — a per-set allowance, and revocability per set — and for one the human partner added: keeping each response small enough to stay inline rather than spilling to a file.
- **Whenever a writable set is loaded**, which closes the mid-session case. *Writable* is the condition rather than *adopted*, because it will become mechanical: read-only evaluation stores are not designed yet, and when they are, they will not pull a watch list unless asked — possibly on being switched to read/write. Today every store is writable, so the clause reads as *whenever a set is loaded*, and the carve-out arrives with the thing it carves out. The case that motivates it: about to work on a dependency's sources, load that project's set, and its signposts are exactly the pitfalls you want before touching the code.

### The marker fix was chosen twice, and the first choice failed against this same mechanism

`about-hoard-watch-is-both-marker-and-subject` in [Issues](../../TRACKING.md#issues): the term **marked** a watch and **named** the mechanism, so design discussion carried it as an ordinary subject keyword — three of the four hits in the first live recall.

The cheaper fix was chosen first: keep the marker, add a served sentence saying a result carrying `hoard-watch` alone is not a watch, and let the existing ranking order the rest, since a watch carries two of the four terms and discussion carries one. **Both halves were drafted and then found unusable against the mechanism they were written for.** The watch recall asks for summaries, and a summary hit is an id and a line — no keywords, no `matched_keywords`. The sentence asked a reader to evaluate a rule against data the response does not carry, and the ranking it leaned on orders a list whose scores are invisible.

**So the filter moved to the query.** The kind keyword *is* the mark: `hoard-signpost`, `hoard-collect` or `hoard-investigation` alone means pushed, and `hoard-watch` marks nothing. A memory about the mechanism no longer matches the recall at all, which is the case that actually occurred. The residue is smaller and stated: a memory *about* signposts legitimately carries `hoard-signpost` and will appear, at the cost of one line.

**Measured end to end before being believed**, which is the one step that found the original defect too. On the live sets the day it landed: the four-term recall returns eight memories and one is a watch — the discussion population had gone from three to seven in two days, all of it written while building the mechanism — and the three-term recall returns exactly the one watch. No migration was needed, since the only live watch already carried its kind keyword.

**The anti-fix still holds** — renaming the marker only moves the collision. What closes it is that a term nobody marks with is free to be written about, which is what `hoard-watch` now is.

### `about-` puts a memory about a mechanism outside the reserved space

**Added 2026-09-06 by the human partner**, generalising the marker fix rather than repeating it. `hoard-` is reserved because those keywords have **control mechanics**: whatever carries one is put into a list that something acts on. A memory *about* the mechanism is a subject, not an entry, so it takes `about-` in front — `about-hoard-watch`, never `hoard-watch`.

**The charter case is worse than the watch case and is live.** Measured the same day, a recall of `hoard-scope-topics` returns **nine memories, of which four are charters**; the other five are Scatterhoard design memories discussing the charter mechanism, which `route-where` would have a session match a candidate memory against as though they described a set. `hoard-author` is clean at four of four.

**The rule prevents new instances, and the fifteen already stored were repaired the same day** — each superseded with a replacement carrying `about-hoard-scope-topics` or `about-hoard-watch`, its claim otherwise unchanged. Verified by enumeration: the three reserved terms returned twenty-two memories with nine impostors before the pass, and afterwards return eight with none. It is done by superseding each with a replacement carrying the `about-` form. Amendment is refused after export, but supersession reaches, because `a-keyword-cannot-be-retired-once-a-memory-has-been-exported` was fixed: a keyword hit follows the tip's keywords, so a term the replacement drops stops matching. Tracked as `reserved-keywords-carry-subject-memories` in [Issues](../../TRACKING.md#issues).

**This section first said the repair was impossible**, on the authority of a memory describing the behaviour *before* that fix. It is corrected here rather than quietly, because the failure is the one this project keeps meeting: a structural claim in a memory reads as current long after the code has moved, and the contradicting evidence — the store response's own keyword-delta note — was in the same session's transcript.

### The charter recall moved to set access, and it cost one character

**Decided 2026-09-06 by the human partner**, from the leak measured above: `route-where`'s charter recall fired on **every store**, returning full records each time. A set's charter changes very rarely and is announced as a deliberate act when it does, so re-reading it per routing decision buys nothing.

**It goes where a set comes into reach, not at write time.** Knowing what a set holds informs whether to recall from it at all, which is a judgement made long before anything is stored — so the charter joins the watch clause in `recall-when`: *at the start of a session, and whenever a writable set is loaded, recall that set's `hoard-scope-topics` charter and its watch lists.* One clause, per set, at the two moments a set becomes available. That is what [`watch-push-is-an-index-per-set`](../../TRACKING.md#backlog) wanted when it said the trigger should join the charter recall — the arrangement it wrongly believed already existed now does, reached from the other direction.

**The move cost one character**: `route-where` gave up 77 and `recall-when` took 78. A leak worth closing needed no trim to fund it, because the instruction relocated rather than being added.

### Slugs are formed by a rule now, and it lives where slugs are written

`about-hoard-watch-is-both-marker-and-subject` was `hoard-watch-is-both-marker-and-subject` until 2026-09-06 — an ISSUES slug sitting inside the reserved keyword space. **No rule produced it and none forbade it**: the tracking files require a slug and require it to be stable, `STANDARDS.md` says to cross-reference by it, and `keyword-standards` sanctions an issue as a keyword and says coined terms are lowercase-hyphenated. Nothing said how a slug is *formed*, so a defect in `hoard-watch` produced a slug beginning `hoard-`.

**The rule went into the headers of [Issues](../../TRACKING.md#issues) and [Backlog](../../TRACKING.md#backlog)** rather than into the catalog, on the human partner's placement argument: those headers are read when the files are edited, which is the only moment a slug is coined, and the catalog is read by every session whether or not it touches those files. It carries one clause the earlier discussion had not: **the slug and the memory keyword must be the same string**, because the slug is what gathers an entry's memories and two spellings split the gathering with nothing to report the split.

**Half of that rule was removed again on 2026-09-08, when the three tracking files became one.** The reserved-space clause and its `about-` companion were three copies of what `keyword-standards` already serves to every session — including sessions with no copy of this repository — so unification collapsed them to none rather than to one, and the surviving text recalls the convention instead of paraphrasing it. **The clause that stayed is the one the catalog does not carry:** the slug and the memory keyword must be the same string, which is a fact about how this project's tooling gathers an entry's findings and not about keywords in general. That split is the placement argument above applied a second time, and it lands differently because the two clauses have different single sources.

**Exempting slugs was considered and rejected.** Matching is exact, so a longer identifier containing a mechanism's name can never appear in a control recall and is inert — but *inert* is not the same as *unconfusing*, and the reserved space is worth keeping legible to a reader as well as to the matcher.

### What was trimmed, and by how much

The hold on catalog re-tuning was lifted for this whole cycle (above). Three trims, each in a class this document already sanctions, none removing a rule:

| Entry | Cut | Chars | Why it was not reinforcement |
|---|---|---|---|
| `route-where` | *"Everything else about a set can be read off its contents, which is why the catalog holds no taxonomy of sets"* | 118 | Update-time context, and already written in the source comment above the entry — the argument for having no taxonomy is for someone revising the convention, not someone routing a memory. |
| `confidence-standards` | *"A scale used across only its top fifth costs a judgement at every store and gives nothing back"* | 96 | Restates the entry's own text — *use the low end — a field only ever written in the top fifth of its range tells a later reader nothing*. A reinforcement repeating its text spends the budget twice on one idea. |
| `keyword-standards` | *"measured here at a relevant memory ranking below an unrelated one"* | 67 | The remedy [this document prescribes](#served-text-must-not-assert-facts-about-the-readers-store) for a measurement in served text: the claim stays, the figure moves to the source comment where it is true. |

| `route-where` | *"The descriptions live in the sets because only a set knows what it holds, and they travel with it through export, so whoever imports a set learns what belongs in it"* | 166 | Update-time context, and — like the trim above it — already written verbatim in the source comment: *"Keeping the descriptions in the sets also makes them travel."* |
| `store-when` | the orientation preamble of *"Orientation is much of what makes a codebase fast to work in, and rediscovering it every session is a real cost — but memories restating what the code plainly shows crowd out the ones carrying something"* | 118 net | The operative half is kept as its own sentence. What went is the argument for why the convention exists, which is for someone deciding whether to keep it. |

**The catalog stood at 17,981 of 18,000 on 2026-09-06**, immediately after these trims — [derive the current figure](#the-headroom-is-derived-never-written-down). **That is the budget genuinely binding, and it is reported rather than quietly relieved.** Five entries were trimmed in one day for a total of roughly 565 characters; the reserve of justifying reinforcement is large but it is not infinite, and *a budget trimmed to fit whenever it binds is not a budget* in the same way a raised one is not. The next change to the catalog should expect to make the case rather than find room.

## Served 2026-09-07: `keyword-standards` carves out `hoard-proj-`, and what funded it

**Decided by the human partner**, extending [the `about-` rule](#about--puts-a-memory-about-a-mechanism-outside-the-reserved-space) rather than repeating it a third time. `keyword-standards` now reads *"Keywords beginning `hoard-` are reserved for control marks -- Scatterhoard's own, and a project's own tooling under `hoard-proj-`"*. The earlier rule said what to do with a memory *about* a control mark; this says which spellings **are** control marks when the thing acting on them is a project's tooling rather than the server.

**What forced it.** Scatterhoard's own tracking-file index is read by [genindex.py](../../scripts/genindex.py), and its marks were spelled `issue-entry`, `issue-open`, `severity-*` — indistinguishable from ordinary subject keywords. On 2026-09-07 a memory *about how deeply to record an entry* was filed carrying `issue-entry`, which enrolled it in the generated index as a head with no standing and stopped `--check` by id. That is the third instance of the marker-and-subject failure, after `about-hoard-watch-is-both-marker-and-subject` and `reserved-keywords-carry-subject-memories` — and the first where the author demonstrably knew the rule, because a correctly-shaped sibling memory was stored eight seconds earlier.

**Which identifies the half that was missing.** `about-` runs *after* recognition. A mark spelled like a topic is not recognised as a mark at the moment of writing, so no rule about marks is consulted, however well known that rule is. The prefix is what makes the rule reachable; the `about-` form is what you do once it is. Two instances had already been repaired without anyone noticing that the repair only covered marks whose spelling announced them.

**Why under `hoard-` rather than a project-owned prefix.** A project prefix such as `track-` needs no served text at all, and was recommended by the LLM at a moment when the catalog had 18 characters free. The human partner rejected that reasoning as letting the budget drive a design decision — *the visible cases can be compensated for; the invisible ones cannot* — and on the merits the carve-out is the better mechanism: inheritance through the served catalog is the only channel that reaches a project which does not yet know it has this problem, where copying the pattern from this source, or from a lessons set, requires someone already looking. The `proj` segment answers the objection that would otherwise apply to subletting, since it says the acting party is not the server.

**Scatterhoard's own marks moved the same day** — `hoard-proj-issue-entry`, the backlog and question families beside it, and the `hoard-proj-severity-*` and `hoard-proj-readiness-*` facets. 94 memories and 276 keyword instances, done as one export/rewrite/`restore` cycle rather than 94 supersessions: a vocabulary rename changes no claim, so supersession would leave 94 records identical to their replacements for recall to step over, with none of the history it exists to preserve. The committed pre-rename export is what makes that honest, and it is why the rewrite is recorded here rather than only in the memories.

### What funded it

The carve-out sentence cost 65 characters, paid for by 367 characters of **cross-entry duplication** — the class the 2026-08-25 restructuring pass identified, deliberately left alone, and recorded as "a separate decision about content". This is that decision.

| Entry | Cut | Chars | Why it was not reinforcement |
|---|---|---|---|
| `store-when` | *"Scope has to be explicit because the software cannot infer it, and a personal memory committed to a shared repository cannot be taken back."* | 142 | The thinnest of three copies. `route-where` opens on the same claim carrying the asymmetry that makes it actionable, and `export-before-commit` carries it again as the reason the diff is a gate. `store-when`'s text still instructs the reader to state scope; only the duplicated reason went. |
| `hoard-watch` | *"Recalling one set at a time keeps that cost divisible: each set has its own allowance, and a noisy one can be left out."* and *"One keyword per call is what says which action a list wants, since a summary carries none to read it from."* | 231 | Both restate `recall-when`'s *one keyword and one set per call*. The instruction lives there; these are its reasons, and they moved to the source comment where a reviser tempted to relax the rule will find them. |

**The other pair that note named was kept, and the reason is worth stating.** `store-when` and `correct-when` both carry *"read by people who were not there"*, and the comment beside `correct-when` says not to simplify it away: that clause is where the Golden Rule *reverses* an answer rather than shading it, sending an unfair characterisation to `forget` where the general rule would supersede. Cutting there removes the exception rather than a restatement of it. A duplication audit that goes by string matching would have taken it.

## Served 2026-09-14: `capture-before-ending` fires before handing control back

**The clause** replaces the trigger list and the instruction to write it out: *"Before handing control back to the human after work that used tools, before a commit or an export, or when the human signals they are wrapping up or clearing context, first write "worth remembering from this: ..." with what you worked out rather than looked up, or "worth remembering from this: nothing". Then store what survives it, in the same turn."* The rest of the entry, and all of its reinforcement, is unchanged. Proposed by the human partner and settled with them the same day.

**Why the trigger moved.** The old first trigger, *a multi-turn piece of work is finished*, was the best-placed moment and the least observable: nothing announces that work has finished. Material worked out on the way to a question handed back to the human therefore had no trigger, and went unrecorded on 2026-09-07 and again on 2026-09-14 — the second time through a whole investigation of set identity whose findings changed the session's recommendations and lived only in the conversation until the human partner asked. `meta-observation-convention` in [Open questions](../../TRACKING.md#open-questions) had already named the moment before handing back as the candidate: it is an act the session performs, and it comes when a summary is being written, which is when the synthesis exists. **This answers that question's trigger half.** Its other half, stepping back to ask whether a result makes sense in the larger context, is not an enumeration and is not addressed here.

**"After work that used tools"**, not *after any operation across multiple turns*, which was the proposal's first wording. Whether a tool was used is observable; how many turns an operation spanned is a judgement, and the investigation that prompted this ran inside one of the human's turns across dozens of tool calls, so a reading of *turn* as the human's turn would have excluded the case the change exists for.

**"What you worked out rather than looked up"** is the bound the open question said any version needed. A hand-back is mostly status, which `store-when` already excludes, and a line that always fills with status is skimmed and then ignored.

**The line is required even when empty.** *"Worth remembering from this: nothing"* is the explicit form of the permission this entry has always given, and it doubles as evidence that the step ran: on 2026-09-07 this convention fired on one commit in ten while `export-before-commit`, which has a tool call to show for itself, fired on all ten. A judgement step paired with a mechanical one needs its own evidence of having run.

**"Or", and "before" on every moment that has one.** The first draft joined the hand-back to the rest with *"and"*, which reads as one compound condition — hand back *and* be about to commit — and would have shrunk the trigger to the case that already fired. A tighter draft said *"at a commit"*, which can be read as after it, too late for the memories to be in the file the commit carries.

### What funded it

Nothing: the new text is 351 characters against the 385 it replaced, so the change returns 34 to the headroom.

## Served 2026-09-21: `recall-when` weighs a recall against what it could save

**The clause** replaces the opening of `recall-when`'s text: *"Search memory before acting whenever a recall costs less than what it could save: at the start of substantive work on a project, before a decision an earlier decision may govern, when the human refers to something as settled, and before redoing work, searching for something, or starting anything slow."* The reinforcement's sentence on when to notice gains its second case: *"familiar-seeming work an earlier decision constrains, or a step whose next move looks obvious."*

**What it answers.** The `recall-would-have-helped` investigation — a `hoard-investigation` memory in the `scatterhoard` set, `01M2B99XD0…`, not a tracking entry — had four cases by 2026-09-21. In three, no recall was made at all: a PDF worked around by hand-writing a parser and then asking the human partner to install a package; a mutation pass waited out for 53 minutes; and a set's memory file searched for on disk. The answer was in `local` for two of them and in `coding-lessons` for the other. The investigation's hypothesis held for all three: the four moments the convention named were all decision-shaped, and a problem presenting as mechanical, with an obvious next move, never registered as one of them.

**The framing is the human partner's, and it replaced a narrower draft.** The first draft named the machine — *before searching or working around the machine, whose facts are in the personal set* — which fitted two cases and only brushed the third. Their objection was that it was too specific: people recall before most actions because a recall attempt is cheaper than walking the house for the car keys, and the same comparison, stated, covers cases no list anticipates. For a session it is lopsided: a recall is one call and, as a summary, a few hundred characters, against a disk search, a long run, or an install request that leaves the session. The named moments stay, as examples of the comparison rather than as its boundary. **It passes the catalog's second drafting test**, orienting rather than persuading, because *does a recall cost less than this?* is askable at the moment of acting, before the answer is known.

**Where the personal set comes in, and why it is not in the clause.** That facts about the machine live in `local` is already served, by `route-where`; and an unscoped recall, the default, searches every set, so once a recall is made it finds them. The fourth case is different in kind — a recall made, but scoped to the project's set while the lesson lived in `coding-lessons` — and this clause does not address it.

**Cost: 69 characters**, leaving the catalog 24 short of its budget. Two phrases were trimmed to make room without changing what they say: *"that an earlier decision may already govern"* to *"an earlier decision may govern"*, and *"already settled"* to *"settled"*.

## Served 2026-09-14: `recall-when` writes and searches in a declared language

**The clause**, added after the charter recall: *"Write that set's memories, and phrase queries against it, in a language its charter declares with a `hoard-lang-` keyword, translating when you can; keep keywords and identifiers as spelled."* The reinforcement gained one calibrating sentence: *"The loaded model may not represent other languages, so text in them can be stored and never found."* Together 290 characters, taken from headroom rather than funded by relocation.

**Why a convention and not the tool descriptions**, decided with the human partner. A session chooses a language while composing a memory or a query, before it reaches for `hoard_memorize` or `hoard_recall`, and a description is read after that choice — the timing [the `hoard_conventions` note in AGENTS.md](../../AGENTS.md#load-the-conventions-first) warns about for anything governing a decision made before a tool is used. It also sits where the declaration is already in view: the charter recall one sentence earlier returns the `hoard-lang-` keywords.

**Why `local` is covered.** The declaration was decided as a standard a maintainer reviews, which reaches only shared sets. The human partner's correction when the build began is that its larger job is findability: the shipping model separates a paraphrase from an unrelated sentence at a third of English's margin in Spanish, German and French and at none in Russian, Hindi and Chinese, and barely at all across languages ([Memory Data Model](memory-data-model.md#resolved-2026-09-11-a-set-declares-its-languages-on-its-charter)). That is as true of `local` as of any set.

**Two phrases are load-bearing.** *"Translating when you can"* is conditional because not every model driving a session translates well, and a poor translation stored as the memory loses more than the language did. *"Keep keywords and identifiers as spelled"* because both are matched exactly, and translating them would cut the one path that works in every language.

## Served 2026-09-13: `export-before-commit` reads the description, and what funded it

**The clause** makes the export's derived description part of the gate this convention already makes of the diff: *"then read that file's diff, and the description the export returns against the set's charter, before committing."* The description is the mechanism half of [demand C](golden-rule.md#what-the-principle-is-for-four-demands-it-makes), built under `derived-set-summary` and described in [Export Review](export-review.md#4-what-the-set-amounts-to--derived-shown-and-never-judged). A response field nothing tells a session to read is skimmed past in a long export report, and the diff gate is where the whole set is already in view.

**The reinforcement gained one sentence, and its closing clause is the part that was worked over.** *"The description is the one view of what the set amounts to as a whole, which no record in the diff shows; it describes the set and implies nothing about the motives of the memories' authors."* The [Golden Rule](golden-rule.md#what-the-principle-is-for-four-demands-it-makes) requires that any text compiled from C keep *this set amounts to X* visibly apart from *this user is doing X*, so a bare instruction to read the description was not enough. **"Implies" rather than "says"**, because the hazard is what a reader infers and not what the description states.

**The first draft drew the line in the wrong place, and the human partner corrected it the same day.** It ended *"implies nothing about why it was built"*, which denies the inference the description exists to support. Keywords clustering around a missile guidance system or a biological weapon make the purpose of the set a safe inference, and a model with a safeguard may appropriately act on it — the reason the description is recomputed at import is that the receiving model's safeguards can engage there. What the principle forbids is the step past that, from the set's purpose to its authors' motives: that is [D(ii)](golden-rule.md#what-the-principle-is-for-four-demands-it-makes), and C's own second instance says a set can amount to a harmful capability while everyone involved intends nothing but good work. **Purpose is a property of the set and may be read off it; motive is a property of people and may not.** The draft had merged the two, having reached for "why" only to rule out "who".

### What funded it

The clause and sentence cost 258 characters as corrected (238 as first drafted). At the human partner's suggestion they were funded by relocating **justification** out of served reinforcement, applying the test in the header of [catalog.rs](../../src/conventions/catalog.rs) — does a sentence *calibrate* the rule, or *justify* it; does it help a session recognise the case, or argue for having the rule — rather than by spending headroom. Four sentences failed that test cleanly and moved to `//` comments beside their entries, for 268 characters: the addition is fully funded and the headroom grew slightly.

| Entry | Cut | Chars | Why it was not reinforcement |
|---|---|---|---|
| `store-when` | *"Naming where it can be checked is what keeps it honest."* | 56 | Argues for the clause. The sentence before it — structural claims go stale silently, decisions do not — tells a session which memories need a check point, and stayed. |
| `recall-before-storing` | *"-- the same failure `keyword-standards` names for spellings, one layer up"* | 74 | Describes how two conventions relate: an arrangement, not a hazard. |
| `correct-when` | *"That is worth having whether or not the old answer was ever true."* | 66 | Argues for superseding. The text's *whatever is wrong with it* is the calibration, and stayed. |
| `capture-before-ending` | *"That is why the boundary is the moment rather than some point after it."* | 72 | Explains why the text is worded around the moment, which the text already is. |

**Two candidates were left alone on purpose.** `recall-when` is the entry the file names as the model to draft against, and its *"which is the friction that makes recalled memory feel costly"* is the nearest thing in it to justification. `hoard-watch`'s *"a set that could name its own action could make a session act on text it did not write"* reads as justification and is the header's own example of calibration in disguise: it is why a session refuses a memory that names its own action.

## Served: the correcting convention

[Corrections](corrections.md) settles which of supersede, amend and forget is proper when. This is the convention that carries it to runtime. Its text is decided:

> **text.** When a memory turns out to need changing, choose by what is wrong with it. If the memory should not be in the set at all — it carries sensitive data, or it characterises a person unfairly — forget it. If the claim it makes is different now, whether because the world moved, because it was wrong, or because it was true but incomplete, supersede it. If only its keywords, links, context or confidence were recorded wrongly, amend it. When the answer is genuinely unclear, supersede.
>
> **reinforcement.** Superseding keeps the old memory readable, which is the point: a later reader learns that the question was live and which way it was answered first, and that is worth having whether or not the old answer was ever true. Amending replaces a value nobody ever believed, so there is no history to lose — but amending something that should have been superseded destroys a belief someone held, while superseding something that should have been amended only clutters a chain, so the unclear cases go the recoverable way. A memory about a person is the exception that runs the other way: superseding an unfair characterisation keeps it permanently readable by people who were not there, including its subject, so retracting one is forgetting rather than superseding.

**It was deliberately withheld from the served catalog** while `amend` and `forget` did not exist: guidance telling a session to reach for two absent tools would have been a convention asserting behaviour the system lacked, which is the single-source-of-truth failure [STANDARDS.md](../../STANDARDS.md) describes, sitting in the one place read into every session.

**Both tools landed on 2026-08-05 and it is served now**, as `correct-when` in `src/conventions/catalog.rs`. Writing the text before the tools was worth doing: the reasoning was fresh, the implementation had something to be built against, and the wording did not have to be re-derived when the withholding condition expired.

### Resolved 2026-08-30: the two contradicting conventions were redrafted

[Corrections](corrections.md) moved the boundary from *which fields* to *was this ever believed* — correction is for a record that **never entered service**, and reaches the whole record; everything else supersedes. **Two catalog entries had been written against the old boundary**, and for one day between the decision and the guard they said the wrong thing, which is the failure this document already names: a convention asserting behaviour the system does not have, sitting in the place read into every session. Both were redrafted on 2026-08-30, in the commit that made the guard real.

- **`correct-when`'s third clause** enumerated *keywords, links, context, confidence, origin* as amendment's territory — a list that was no longer the rule and no longer complete, since correction now reaches `content` and `why`, while its real precondition went unmentioned. A session following it after an export would have been told to amend where the tool refuses. **It now reads**: everything but a memory that should not exist at all turns on whether the memory has been exported, *which the server knows and you cannot* — so attempt the amendment and supersede if it is refused. The enumeration is gone rather than corrected, which also retires the standing chore of keeping a served list of fields in step with the code.
- **`confidence-standards`' recalibration clause** — *"amend the number rather than superseding: that is recalibration, not a new belief"* — was wrong for any record that entered service. Its reinforcement was weaker than it looked, too: the objection it rested on, that superseding a confidence change would fill a chain with records nobody believed differently, **was measured and did not occur**. Every confidence change across 136 live memories rode a supersession that rewrote the claim substantially, because the evidence that moves a number usually also changes how the point would be stated. **It now points at `correct-when`** rather than naming an instrument, so there is one place the rule lives and the reinforcement carries what the pointer leaves out: recalibration alone would be an amendment, but a memory another copy holds needs the chain.

**The redraft was costed against the budget, and it cost.** The catalog stood at 16,988 of 18,000 characters — 1,012 free, re-derived 2026-08-29 — with three Ready conventions competing for the remainder. It stood at **17,097, leaving 903** after that redraft, and at **15,887, leaving 2,113** after the cold-reread pass later the same day (both re-derived; the paging measure, which excludes the ten ids, is 15,721). *Re-derive the figure before quoting it* — the enforced test counts the ids and the paging measure does not.

**The prediction that the service rule would be cheaper to state was wrong, and by a useful margin.** `correct-when`'s served text did shrink — 649 to 633 — because a three-way choice became a two-way one and the tie-break clause disappeared with the unclear case it broke. The growth was all in `reinforcement`: the field list needed no justification a session could not see, while *why the boundary is the record's history* is an argument about a merge that has not happened yet, and an instruction whose reason is invisible at the point of use has to carry it. **The general form is worth keeping**: replacing a rule a caller can check with a rule only the server can check makes the text shorter and the reinforcement longer, because the caller is now being asked to trust rather than to verify.

*A first draft of `confidence-standards` restated the whole rule inline and grew the entry by 274 characters; pointing at `correct-when` instead brought that to 42. Both conventions are served in the same catalog, so the boundary this document already draws applies — restate what the reader does not have, point at what they do.*

### Resolved 2026-08-30: reinforcement ratchets, and relocation is the lever

*Raised by the human partner, from the observation that `reinforcement` grows even in the commits where `text` shrinks.*

**Measured across every commit that touched the catalog: reinforcement grew in 15 of 17 revisions and shrank in exactly one** — the 2026-08-25 restructure, which cut 151 characters as a side effect and whose own note says length is not a lever worth reaching for again. The ratio of reinforcement to text drifted from **1.27** at the first commit to **1.42**; the sharpest single divergence is 2026-08-24, text +394 against reinforcement +1,192.

**Five reasons, and none of them is carelessness.** Text is shaped by being an instruction and so resists growth, while an argument that grows reads as better supported. The editing rule filters for *a sentence that only helps a reviser*, which catches the wrong thing — the harder question, **would a session act differently without this**, is asked nowhere. The judge is the author minutes after finding the argument compelling. Nothing triggers a re-read, because conventions are edited locally wherever a decision touches them. And the one that makes it a ratchet rather than a drift: **additions carry a recorded reason and removals carry none**, so cutting means overruling a written argument with a judgement backed by nothing.

**The lever is relocation, not deletion**, and the pass that followed recovered **1,382 characters** with over half of it moved rather than cut. `route-where` was carrying 707 characters of charter-*writing* guidance — good text, served every session, for an operation that happens when someone creates a set. It now lives on `hoard_sets create`, which is [decision placement](decision-placement.md) applied to served text: put it where its moment is. That sidesteps the ratchet's real problem, because a good sentence cannot be argued away but it can be put somewhere better.

**What was cut outright was small and each piece failed the same test.** `store-when`'s justification of the `why` field, which the tool requires and a session cannot omit; a connective whose work the following sentence already did; and the naming of `toNFKC_Casefold`, which is mechanism for a reviser where the actionable half — *an identifier loses nothing by keeping its capitals* — survives.

**The honest limit** is that all of this is judgement, by the party that also writes the text. Making removal answerable needs a measurement, which is filed as [whether convention text can be evaluated objectively](../../TRACKING.md#open-questions), and `cold-reread-of-the-catalog` in [Backlog](../../TRACKING.md#backlog) is the recurring reading pass in the meantime.

**One caution about the sequencing**, from this document's own rule: convention text must not describe a tool surface that does not exist yet. The service rule is decided and unbuilt, so `correct-when` must not be rewritten to describe it until `hoard_correct` actually refuses on service — otherwise the catalog spends a session telling callers about a guard that is not there, which is the withholding condition that kept this convention out of the catalog in the first place, arriving from the opposite direction.

**That precondition expired on 2026-08-30**, when `correction-gated-on-service` shipped, and the redraft went into the same commit as the guard. The sequencing rule was honoured rather than waived: the text describing the refusal and the refusal itself became true at the same moment, which is the strictest reading of *do not serve a convention for a tool surface that does not exist*. What now reaches a session is: forget what should not exist at all; otherwise **attempt the amendment and supersede if it is refused**, because the server holds the fact and the caller cannot see it.

### A hole neither of them covers, and it is not a contradiction

**Nothing in the catalog ever triggers a recalibration.** `confidence-standards` says what to do *when you become more or less sure*; no convention ever asks whether you have. Both conventions that put a session in front of an existing memory — `recall-when` and `recall-before-storing` — ask about the thing being *written*, never whether the thing being *read* is still calibrated, and `correct-when`'s *"or a memory otherwise turns out to need changing"* is passive: it handles the case once noticed and does no noticing. The asymmetry is sharpest at session end, where `capture-before-ending` sweeps for **new** knowledge and nothing sweeps for **changed** knowledge.

That is why the 2 corrections in 136 memories is **a floor on need rather than a measure of it**, and it should be quoted that way. Filling the hole is not proposed here: it would be a new convention, and *Scope of the current phase* below keeps the set small until the basic system works.

### How the Golden Rule reached this one

Worth noting as an instance rather than a claim. [The Golden Rule](golden-rule.md) says it reaches runtime through the reasoning attached to memory-writing conventions, and here it did more than that: it **reversed an answer**. The rest of the correction rule says a claim that changed is superseded, which would have made a retracted characterisation of a colleague permanently readable — the durability hazard that document names, produced by faithfully applying the general rule. The exception is in the convention text because the principle was in context while the text was being drafted, which is the mechanism working as intended.

## Scope of the current phase

**Keep the convention set small until the basic system works.** The catalog is deliberately minimal — enough to exercise the mechanism, not an attempt at coverage. Expanding it is easy and tempting, and doing it before there is a working system to test conventions against would mean writing a lot of guidance nobody has watched operate.

Two things are expected later rather than now, and are deferred on purpose: formalising the [standing principles](index.md) into conventions of their own, and a convention about *how* to store memories in ways that let new principles be discovered from practice.

**This section's precondition expired on 2026-08-12** and the section is kept as the reasoning it was, not as a live constraint. *Until the basic system works* stopped binding the day three conventions shipped against it, and what binds now is the [character budget](#resolved-the-budget-is-18000-characters) — so a deferred entry needs its own reason rather than this one. Recorded here on 2026-09-07; until then the only note of the expiry lived in a TRACKING.md section preamble, invisible from this document and from AGENTS.md, which sends a reader here before expanding the set.

## Proposed: what belongs built-in rather than add-on

**A convention belongs in the built-in catalog if a user who does not know it exists would be worse off.** Proposed 2026-08-09 and endorsed, but **held as a test to try against candidates rather than a settled rule** — the division between built-in and add-on is expected to move as the catalog grows, and a criterion adopted before it has judged anything real is a criterion nobody has watched operate.

What recommends it is that it discriminates without needing a taxonomy of subject matter. The three decided additions in [Backlog](../../TRACKING.md#backlog) all pass: a session that never articulates what it learned does not know what it lost, which is precisely the case where the user cannot supply the missing convention because nothing tells them it is missing. A specialised convention someone would go looking for fails it — and failing it is not a demerit, it is what *available option* means.

Note the asymmetry it encodes, which is the reason it works: **the cost of a missing built-in is invisible to the person paying it, and the cost of an unnecessary one is not.** An absent convention shows up as knowledge quietly never recorded; a surplus one shows up as context spent, which the budget above already makes someone confront. So the test puts the burden of proof where the failure is silent.

Apply it to candidates as they arrive and record what it gets wrong.

## Resolved: the context budget covers built-ins only

**The character budget applies to the built-in catalog and not to add-ons.** Decided 2026-08-09. Built-in conventions are served to everyone, so their cost is a tax nobody opted into and the project has to hold the line on their behalf. Add-ons are a matter of user choice, and a user who enables one has both the information about their own context pressure and the stake in it — so the tradeoff is theirs to make, not the project's to pre-empt. That is [decision placement](decision-placement.md) applied to context cost.

A convenient consequence: `catalog_fits_context_budget` measures the *compiled* catalog, which is exactly the built-in set. If built-ins stay compiled in and add-ons load at runtime, the existing test goes on measuring precisely the right thing with no change — the boundary the policy draws is already the boundary the code draws.

It does not follow that add-on cost is invisible. A user choosing well needs to know what a convention costs *before* enabling it, which is a reporting problem rather than an enforcement one, and is open.

**The built-in set is expected to grow, and by how much is now the binding constraint.** Three additions are decided — see [Backlog](../../TRACKING.md#backlog): a rule that makes storing durable knowledge an explicit step after a multi-turn operation, minimal keyword standards, and an investigation pipeline built on those. Measured 2026-08-09, the three current conventions occupy roughly 3,800 of the 8,000 characters, averaging about 1,270 each. Three more at that average lands near 7,600 — it fits, with little room, and the investigation pipeline is likely above average since it carries three linked concepts. **Expect the budget to bind during drafting rather than after**, and treat raising it as the deliberate decision the constant's documentation already demands rather than the obvious way out. *It bound as predicted, three conventions later; the section below is that decision.*

**Keeping the set small is a decision about the *built-in* catalog, and it creates its own pressure.** A minimal catalog limits the context tax and is right for shipping; it also means the good conventions will be discovered by use rather than written up front, and that there will be standing pressure to add more. How that pressure is absorbed — bundled-optional conventions, and add-on conventions installable without a rebuild — is *how conventions get extended without recompiling* in [Open questions](../../TRACKING.md#open-questions). Note what that question turns on: a file format for a convention has to carry `text` and `reinforcement` as *separate served fields*, for the reason the section above gives.

**Both are tracked as work in [Backlog](../../TRACKING.md#backlog)**, along with the pre-commit convention named above, as `Deferred` — the deferral applies to the two general ones rather than to all of them. The recall-gloss convention was also tracked here until it was retired on 2026-09-04; what remains of it is `context-gloss-on-recall`, and it is `Blocked`. Track a convention's *status* there and its *reasoning* here; this section stating that the set is kept small is the decision, and the backlog is where someone looks to find out what has not been drafted yet. A convention whose text is written but not yet served is a normal state and is a backlog entry, not an omission — see the `correct-when` history above.

## Resolved: the budget is 16,000 characters

**Raised from 8,000 to 16,000 on 2026-08-12 by the human partner**, with the catalog at 7,465 of 8,000 and `keyword-standards` — about 1,244 characters at the current average — unable to fit. `MAX_CATALOG_CHARS` in [src/conventions/mod.rs](../../src/conventions/mod.rs) is the enforced copy; the test that reads it is unchanged.

**The decision was made on what the budget is actually spending, rather than on what the next convention needs.** 8,000 characters is around 1,950 tokens and 16,000 is around 3,900, against context windows running from 128K on the smaller open-weight local models up to about 1M. Both numbers are small at either end of that range. A budget set where the old one was would have been paid for by shrinking conventions against each other — spending drafting effort and clarity to save something the context window does not notice.

*The token figures are the pessimistic end of the proxy, which is the right end to decide on.* They divide at roughly 4.1 characters per token, which is the measured rate for bare identifiers; catalog prose runs nearer 5.5, so 16,000 characters of convention text as currently written is closer to 2,900 tokens. The arithmetic that justified the raise assumed the catalog drifts toward the vocabulary tables the constant's documentation warns about.

**What did not change is the reason there is a budget at all.** Growth still has to be a decision someone makes rather than drift nobody notices, and the enforcement is still a test that fails. Nor does it change [the built-ins-only scope](#resolved-the-context-budget-covers-built-ins-only): 16,000 is the ceiling on what every session is charged without opting in.

**16,000 is a number to work towards, not a target to fill.** The instruction attached to it was to work towards it and see how we do — which is an invitation to find out what the catalog costs in practice, not authorisation to spend the headroom. The standing worry about a session that records everything has a catalog form, and doubling the room is exactly when it applies.

## Resolved: the budget is 18,000 characters

**Raised from 16,000 on 2026-08-13 by the human partner**, the second raise, and decided on the same ratio the first one was — what the budget *spends*, not what the next convention needs. The [16,000 decision](#resolved-the-budget-is-16000-characters) above is kept as the reasoning it was; this section records what changed.

**The position at the raise.** The catalog stood at 14,610 characters over ten conventions — **about 3,560 tokens at the identifier rate, 2,660 at the prose rate** — leaving 1,390, with `investigation-pipeline` still to draft and expected to run *above* the 1,461-character average because it carries three linked concepts. So the next convention did not fit, and the only way to make it fit under 16,000 was to shrink the others against each other, which is exactly what the first raise was made to stop.

**The bar the human partner set was proportional rather than absolute: stay well under 4% of the smallest context window in range.** 18,000 characters is about 4,390 tokens, which is **3.4% of 128K** — and 128K is the floor of the range, from smaller open-weight local models; against a 1M window it is under half a percent. The arithmetic is done at the identifier rate for the reason the constant's documentation gives: that is the direction the catalog drifts.

**Two things this does not change.**

*The instruction is still "work towards it, not fill it"* — and it applies harder at a second raise than a first. **A budget raised whenever it binds is not a budget.** What has kept these two honest is that both were argued from the ratio rather than from what would not fit; a raise argued the other way round is the one to refuse.

*The scope is still built-ins only.* 18,000 is the ceiling on what every session is charged without opting in — see [the built-ins-only policy](#resolved-the-context-budget-covers-built-ins-only), which is unaffected.

### The headroom is derived, never written down

**The budget is a constant and may be quoted; the headroom is derived state and may not.** `MAX_CATALOG_CHARS` is 18,000, enforced by `catalog_fits_context_budget`, and it changes only by a deliberate decision recorded in this document. What is left over changes with every catalog edit, and every prose copy of it has eventually been wrong.

```text
cargo test --test conventions report_catalog_size -- --ignored --nocapture
```

**This is not a hypothetical.** On 2026-09-07 a session derived the headroom as **18 characters** while three passages stated it in the present tense: 125 free, 19 free, and 2,113 free in [`third-catalog-budget-raise`](../../TRACKING.md#open-questions). Each had been re-derived correctly on the day it was written; each then went stale in silence while still reading as current. The 2,113 figure did real damage — it says the budget is not binding, which was the opposite of the truth, and it was quoted into a design recommendation before the derivation caught it.

**The same holds for any counted claim about the catalog, not just the headroom.** TRACKING.md's Conventions preamble once said the catalog served three conventions long after it served nine — the count is derivable from `conventions::all()`, and a prose copy of it decays exactly as the character figure does.

**So the rule is about tense, not accuracy.** A figure tied to a date and written in the past — *"the catalog stood at 17,981 on 2026-09-06"* — records what a decision was taken against and stays true forever. The same figure in the present tense is a claim that decays with the next edit. Narrate with dated past-tense figures; when you need the current number, derive it.

**`MAX_CATALOG_CHARS` is not a page size, though until 2026-10-01 it was one.** It used to double as the default `max_chars` for a `hoard_conventions` page, which is what kept the catalog in a single page. Since then a call with no budget receives the whole catalog whatever its size — necessarily, because the served document carries JSON punctuation and ids the budget does not count, and already ran past 18,000 characters that day — so the constant bounds what the catalog may grow to and says nothing about how it is served. How it is served is in [Tool Surface](tool-surface.md#resolved-2026-10-01-a-convention-page-is-a-slice-of-one-document-ended-by-an-instruction).

## The Golden Rule shapes convention drafting

[The Golden Rule](golden-rule.md) is a standing input to *every* convention drafted here, not a convention of its own. Load it before drafting or revising convention text.

It reaches runtime through the "why" attached to the memory-writing conventions (see above) rather than as a standalone directive — the reasoning for that, and the boundary it has to respect (charitable about people, exact about code), are in that document.

**Draft from the four demands, not from an existing clause.** That document now states what the principle is *for* as four demands a memory about a person has to meet. Compiling a new clause from a previous compression is what produced the two miscalibrations resolved above, and it is an easy mistake to repeat, because the existing clauses read as though they were the principle.
