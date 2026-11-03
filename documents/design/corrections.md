# Corrections

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

Three operations change a memory that already exists: **supersede**, **amend**, and **forget**. This document settles which is proper when. [Memory Data Model](memory-data-model.md) owns the `SUPERSEDES` label itself; [Set Lifecycle](set-lifecycle.md) owns where the operations live on the tool surface.

**Status: decided 2026-08-05; the boundary redrawn 2026-08-29.** All three operations exist — see *What is
implemented* below. **Read *correction is for a record that never entered service* before the table**, which
moves the amend/supersede boundary from *which fields* to *was this ever believed* and is the governing rule.

## Why this needed settling

The design has argued both sides. At times supersession has been described as expressing a shift in ground truth — the old memory was *correct when written* and the world moved. At other times it has covered a conclusion that was simply wrong. Those readings give opposite answers for the most common case there is, which is discovering that something recorded confidently was a misunderstanding.

Mistakes will be frequent. A rule that has to be re-derived each time will be applied inconsistently, and two of the three operations destroy information.

## Resolved: the claim is `content` plus `why`; everything else is apparatus

That single line does most of the work.

- **`content` and `why` together are the claim** — what the memory asserts, and the reasoning that makes it judgeable.
- **`keywords`, `links`, `context`, `confidence` and `origin` are apparatus** — how the memory is found, what it connects to, when it came up, how much weight to give it, and whose reasoning produced it.

  *`origin` was added 2026-08-21, closing `origin-cannot-be-corrected`.* It had been named in neither list, so a wrong value had no sanctioned repair at all and a caller had to guess. It belongs here by this document's own test — nobody ever believed the wrong value, so there is no history to lose, exactly as with a recalibrated confidence. **And superseding would have been the wrong instrument rather than a verbose one**: it keeps the old record readable, which for a misattribution of reasoning means the wrong attribution stays legible for ever. That is the same durability hazard that makes *forget* rather than *supersede* the instrument for a record that characterises a person unfairly, arriving in a smaller form. [The Golden Rule](golden-rule.md) leans on this field by name — it exists so the system does not tell someone "you decided X" when the model decided it — and the party filling it in is the one most likely to get it wrong in exactly that direction.

So:

| Operation | When | What happens to the old value |
|---|---|---|
| **supersede** | The claim is different now | Kept, and readable. It is a true record of what was believed. |
| **amend** | The apparatus was faulty | Replaced. Nobody ever believed the wrong value. |
| **forget** | The memory should not be in the set at all | Removed, leaving a tombstone. |

### The test, in order

> ⚠️ **Superseded 2026-08-29.** The table and the test below were correct under the field-based
> boundary and are kept because the reasoning that produced them still holds. **They are no longer
> the rule** — see *correction is for a record that never entered service*, which replaces step 3's
> *otherwise* with a question about service rather than about fields. The current test is:
>
> 1. **Should this memory exist at all?** → **forget**.
> 2. **Has it entered service** — exported to any target, backups included? If not → **correct**,
>    any field.
> 3. **Otherwise** → **supersede**.
>
> Note what moved: the old test reached `amend` as a fallback, so anything not obviously a changed
> claim landed there. The new one reaches `supersede` as the fallback, which is the recoverable
> failure this document already prefers.

1. **Should this memory exist at all?** Sensitive data that leaked in, or a record that should never have been made → **forget**.
2. **Is the claim different from what it was?** → **supersede**.
3. **Otherwise** → **amend**.

## Resolved: the criterion is what was *believed*, not what was *true*

The reading that kept collapsing was *"supersession is for a memory that was correct when written"*. It has to be abandoned, because it gives no answer for a misunderstanding — which was never correct, and which is exactly the case where history matters most.

**What supersession preserves is a record of what the project believed.** That value does not depend on the belief having been true. A later reader who finds that the project once believed X, confidently, and then stopped, learns something a corrected record alone cannot tell them: that the question was live, which way it was answered first, and therefore how much to trust their own instinct if it matches the abandoned answer.

So all three of these supersede:

- **The world moved on.** The claim was true and is no longer.
- **We were wrong.** The claim was never true and was believed anyway.
- **We were right but partial.** The claim held as far as it went. This is the case that can supersede one memory with *several*, which is why supersession resolution reports a list of chain tips rather than one.

And amendment covers what nobody ever believed: a keyword spelled wrongly, a link pointing at the id next to the intended one, a `context` naming the wrong branch, a `confidence` that was mistyped.

**The distinction is about the record, not about the truth.** A memory that always meant to say what it says, and failed to, is a recording error. A memory that said what was meant, and is now wrong or incomplete, is superseded.

## Resolved: when genuinely unclear, supersede

The two mistakes are not symmetrical.

- **Amending what should have been superseded destroys history irrecoverably.** The previous claim is gone, and nothing records that anyone held it.
- **Superseding what should have been amended clutters a chain with trivia.** That is a real cost — it degrades exactly what chains are for — but it is *visible*, and it loses nothing.

So the tie-break follows the project's usual habit of preferring the recoverable failure: **when the answer is genuinely unclear, supersede.** This should be rare, because the test above is usually obvious; a rule that fires often would be a sign the boundary is drawn in the wrong place.

## Resolved: a `confidence` change alone is an amendment

> ⚠️ **Narrowed 2026-08-29** to a record that has not entered service; once it has, a confidence
> change is superseded like anything else. The reasoning below is not wrong — a recalibration is
> still not a new belief — but the objection it rests on, that superseding would fill a chain with
> near-identical records, **was measured and did not materialise**: across 136 live memories every
> confidence change rode a supersession that also rewrote the claim substantially, and not one
> supersession copied content verbatim to move a number. The evidence and the number usually arrive
> together, because whatever moves your confidence usually also changes how you would state the
> point. **`confidence-standards` in the served catalog still says to amend**, which now contradicts
> this document — see [Conventions](conventions.md).

Worth stating because it looks like a claim changing. Becoming more or less sure of an *identical* claim is a recalibration of apparatus, not a new belief — and superseding for it would put two near-identical memories in a chain differing by one float, which is precisely the "duplicate the content to fix metadata" failure that makes chains less useful.

If the *reason* for the change in confidence is itself worth keeping — new evidence arrived — that is a **new memory about the evidence**, linked to the original. Not a supersession of it.

## Resolved: an unfair memory about a person is forgotten, not superseded

This is where [The Golden Rule](golden-rule.md) changes an answer the rest of this document would otherwise give.

A memory characterising a collaborator, which turns out to be unfair or drawn from too little evidence, looks like a claim that changed — so the table above would say supersede. **That is wrong, and superseding it is the harm.** The Golden Rule identifies durability as the specific hazard this system creates: a person forgets an irritation, a memory system does not, and supersession *keeps the characterisation readable* while merely stopping it being recalled by default. An unfair judgement of a person, preserved permanently and legible to whoever reads the set later — possibly the person described — is the outcome that document exists to prevent.

So: **retracting a characterisation of a person is `forget`.** The general principle stands — history about *claims* is worth keeping — and this is a bounded exception where preserving history is itself the injury.

Two things keep this from being a hole in the record:

- **Forgetting is attributed.** The tombstone carries who and when ([File Format](file-format.md)), so the retraction is visible even though its subject is not.
- **A specific, evidenced observation does not need this exit.** The Golden Rule's point that the charitable memory and the accurate memory are usually the same one applies here: *"X's PR had three untested edge cases"* is a fact that can be superseded when it stops being relevant. It is the sweeping characterisation that has to be removable, and that is the kind least worth preserving anyway.

## What is implemented

All three exist as of 2026-08-05, and `amend`'s boundary moved to the service test on 2026-08-30.

- **`supersede`** is a link label written at store time, via `hoard_memorize`. **Since 2026-09-02 a
  call carrying one must state the replacement's `keywords`**, and absence is refused while an
  explicit `[]` is stored as written. Neither default is safe, which is why there is no default:
  carrying the superseded memory's terms over makes the replacement findable under terms nobody
  chose for it, and letting them default to empty is how a record quietly stops being reachable —
  the failure measured in another memory system, where superseding silently dropped a record's keys.
  The two are the halves of `a-keyword-cannot-be-retired-once-a-memory-has-been-exported` in
  [Issues](../../TRACKING.md#issues). Only `Option` can tell *absent* from *deliberately empty*, so
  `keywords` on `hoard_memorize` is one; `swallowed_parameters` already drew that line for the same
  reason. The requirement is narrow on purpose — `keywords` stays optional on every other call,
  since requiring it everywhere would be the larger decision
  [Decision Placement](decision-placement.md) governs, and a caller writing a `supersedes` link is
  deciding the replacement's index whether or not they realise it. **The response reports the
  delta** — `keyword_delta`, one entry per superseded target, naming `added`, `dropped` and
  `retained`, and reporting even when nothing was dropped, since that is the confirmation a faithful
  re-typing earns. Reported and never refused: dropping a keyword is the only way to retire one, so
  the server states the fact where the caller can still act on it. Same-set targets only, because a
  cross-set target's keywords may not be held here and an empty delta would say *nothing was
  dropped* where the truth is *nothing was checked*. **Links get the same report**, since 2026-09-24:
  `link_delta` names, per superseded target, the links it carries that the replacement does not
  restate (`left_behind`) and those it does (`restated`), because an out-link is the claim of the
  record that wrote it. Compared on label and target id, so a changed label is a link left behind
  and re-typed `target_keywords` are not; the superseded memory's own `supersedes` and
  `superseded_by` are its history and are not listed. This is step 1 of
  `out-links-stay-with-the-record-that-wrote-them` in [TRACKING.md](../../TRACKING.md), which reports
  what a supersession leaves behind before the walk stops inheriting it.
- **`amend`** is `hoard_correct` action `amend`. It reaches **every field of a record that has never been exported**, `content` and `why` included, and is refused outright on one that has, with a message naming supersession. The test is `Store::has_base`: a base exists only where this store and some file have agreed, so one means a second durable copy is out there. See [the resolution below](#resolved-2026-08-29-correction-is-for-a-record-that-never-entered-service).
- **`forget`** is `hoard_correct` action `forget`. It deletes the memory row — with its keywords, its vector, and every link that is a judgement it made — and leaves a tombstone carrying the id, the attribution of the act, the memory's own supersession edges, and the checksum of what it removed — so a chain survives losing a member, and a receiver can tell a forget of the version it holds from a forget of one it never had (`forgetting-a-chains-middle-makes-a-stale-conclusion-current-again` and `tombstone-carries-the-checksum-it-forgot`, both 2026-08-30; [File Format](file-format.md#resolved-a-forgotten-memory-is-a-line-not-a-missing-line) has the rule). Tombstones travel through export and import as ordinary records, so forgetting reaches other copies through the same three-way reconciliation as any other change. **It is not gated on service**, and cannot be: the memories it exists for — a credential, an unfair characterisation of a person — are exactly the ones whose removal matters most after they have travelled.

**What the gate did to the rest of the system**, found by building it rather than by reasoning about it: a store-side change to an exported record can now only come from a `forget` or from an **import against a different target**. Three reconciliation tests created their divergence by amending after an export, which is now refused, and were re-routed through a second checkout — the same conflict, by the route a real user reaches it on. This is `concurrent correction becomes structurally impossible` observed from inside the test suite, and it is recorded in [Multi-Set Sync](multi-set-sync.md) as well, because it is a fact about reconciliation rather than about corrections.

The convention carrying this document to runtime is written in [Conventions](conventions.md) and was deliberately withheld from the served catalog while two of the three tools were missing. **That reason has now expired**, and moving it into `src/conventions/catalog.rs` is outstanding work rather than a decision still to make.

## Resolved 2026-08-29: correction is for a record that never entered service

*The human partner's answer to the question below, and it is a third option neither side of it had:
**not two mechanisms and not one, but the same two with the boundary moved** — from *which fields*
to *was this ever believed*.*

- **Correction's occasion narrows.** A record that never entered service is the only thing it may
  touch, and that is its whole licence.
- **Correction's reach widens** to the entire record, `content` and `why` included. A record nobody
  ever read holds no belief to destroy, so the field list has nothing left to protect.
- **Supersession takes the whole update load**, which is very nearly what it was already carrying.

**This is the criterion above doing all of the work instead of half of it.** *The criterion is what
was believed, not what was true* was settled on 2026-08-05, and the claim-versus-apparatus split was
a second boundary drawn on top of it. Applied directly, the belief criterion answers every case the
field list answered and the cases it could not — which is why `origin` joining the amendable set on
2026-08-21 stops reading as a line redrawn under pressure. It was the field list failing, not the
criterion.

**Measured before it was decided**, over the 136 memories of the two live shared sets on 2026-08-29:
16 supersessions, 10 tombstones, **2 amendments**. Both amendments happened 18 and 144 seconds after
the memory was created, while supersessions run from 14 seconds to 12.4 days with 6 of 16 at a day
or more. All four confidence changes in the corpus rode supersession edges — none by amendment, and
none with content held constant. The kept memory is `01M18CDT5ZEQ0VMT1V97QCDZPW` in the
`scatterhoard` set, with the method and the caveat that **the amendment figure is a floor on need
rather than a measure of it**: nothing in the catalog triggers a later recalibration, so the corpus
has never been asked whether it is still calibrated.

### The server decides it, and by a fact rather than a judgement

**The LLM cannot be asked**, and [Decision Placement](decision-placement.md) gives two reasons of
unequal weight. It *lacks the information*: whether a record was exported is per-target base state
in the store and never on the wire, and whether another session read it is not knowable to a session
that was not there. And it is *the interested party* — the caller proposing a correction is the one
who benefits from certifying that nobody believed the record yet. This document already makes that
argument about `origin`, where *the party filling it in is the one most likely to get it wrong in
exactly that direction*; it binds harder here, because a field list at least fails visibly while a
self-certified judgement fails silently.

**The property is: never exported to any target.** The per-target bases answer it exactly, with no
new state, no threshold and no write on the read path. It is also precisely the reconciliation
property — a record that has never left this machine has no second copy anywhere to diverge from.

Two candidates were rejected. **A time window** would need a number, and this project already holds
three defects whose fix is *a number the human partner has to set*; a threshold here would be a
fourth, guarding something a fact already settles. **Never recalled** is closer to *believed* in
spirit, but it needs state written on every read, and `recall-before-storing` makes reads constant
enough that it would mark almost everything in service within minutes.

**A backup counts as an export, and the reason is `import` rather than `restore`.** Restore
*replaces* — [mcp.rs](../../src/mcp.rs) is explicit that local divergence is discarded on purpose —
so a restore performs no merge and needs no ancestry. But `import_set` places no restriction on
scope, and export writes bases for a backup destination like any other target, so a backup of a
`local` set can be reconciled back in. A second durable copy exists and the machinery to merge it is
reachable, which is the whole of the test.

**The consequence for a local set with no backup was traced and accepted.** Such a memory stays
correctable indefinitely, including one written weeks ago and relied on many times: no merge can go
wrong, so the gate never closes, and a belief that was genuinely held can be rewritten leaving only
a `modified` timestamp. That timestamp is a real clue and a weak one — it says *this changed at T*
without saying what it changed from, so it tells a reader their context may be stale without telling
them whether the difference is a correction or their own error. The behaviour is kept symmetric
deliberately: local memories are the user's own data, which is already why they are exempt from the
export review gate. Making it asymmetric would need read-tracking, and there is no cheaper proxy.

### What the standard is actually tracking

*Raised by the human partner as a test of the standard rather than an objection to it, and it is the
sharper statement of why the standard is right.*

Two MCP servers on one store: one session reads a memory into context, another corrects it. The
reader now holds something stale — but it holds something stale under supersession too, and the only
difference is that supersession leaves a chain where correction leaves a `modified` timestamp.
**That difference does not matter here**, because the reader can re-retrieve and compare either way.

It does not matter because **nothing durable diverged**. One store, one truth; the stale copy lives
in a context window, is never written back as a competing version, and converges the moment anyone
looks again. A merge has none of that: two stores, two durable records, reconciled later by
machinery, on a day when neither author is present to be asked.

**History exists for the party that cannot re-read.** Ancestry is a substitute for being able to ask
again. A reader can ask. A merge algorithm cannot, so the record has to carry the answer itself.
That single rule generates both of this document's answers — refusal where a merge is possible,
and a remark where the affected party is present — rather than needing one rule for each.

### `origin` is superseded like anything else

**This reverses `origin-cannot-be-corrected` (2026-08-21)**, decided by the human partner on
2026-08-29, and it is recorded as a reversal because the earlier conclusion is cited above as
evidence that the amendable boundary was being drawn case by case.

The reasoning: `origin` exists to clear up confusion about whether the human, the model, or both
produced a piece of information — it cues a person's memory and prevents a specific
misunderstanding. The cost of a superseded wrong value is correspondingly low, and **supersession is
itself the signal** that the older record is no longer the accurate one. The superseding memory
states the correct attribution and the chain says which is current, so the misattribution does not
stand unqualified in the way an unretracted characterisation would.

*What does not change:* an unfair characterisation of a person is still `forget` rather than
supersede. That exception rests on the durability of a judgement about someone, which is a different
hazard from a mis-set provenance field, and the section above stands unaltered.

### In-links get a remark, not a refusal

Widening correction to `content` introduces a hazard apparatus correction never had: another
memory's `SUPPORTS` or `INSTANCE_OF` edge was judged against text that has now been rewritten, and
the label can be left asserting something nobody would author.

**The answer is a remark, and the standard above is what decides it.** A record eligible for
correction has never been exported, so its id has never left the machine and nothing outside can
point at it: every in-link it carries was authored locally, in this store, by this user. The
affected party is present and the record is in their own store. That is not the merge case. It is
not quite the concurrent-reader case either — the *session* that wrote the edge is gone and only the
human could re-check, with nothing prompting them — so it sits between the two, which is what a
remark is for.

Three constraints on the shape:

- **Scope it to a correction that touches `content` or `why`.** Apparatus correction cannot
  invalidate a label, because the text the edge's author judged has not moved. Fired unscoped it
  would arrive on every confidence change and teach callers to skip it.
- **A remark, in the sense [mcp.rs](../../src/mcp.rs) already uses**: the operation succeeds and the
  response carries a field naming a fact the caller could not otherwise see, together with both
  readings of it. The rule the code applies when choosing between remarking and refusing is
  *remark when the caller can still act, refuse when they cannot* — the malformed-parameter guard
  was converted from one to the other on exactly that ground. Here the caller can act in the same
  turn, holding both texts.
- **Blocking on in-links was considered and rejected.** `related_to` dominates both live sets — 24
  of 51 edges here and 37 of 48 in `coding-lessons` — and asserts least about content, so a block
  would refuse most often where it protects least, and would make whether a typo can be fixed depend
  on whether a sibling memory happened to point at the record.

**Built 2026-08-30.** A correction that touched `content` or `why` returns `in_links` — the
non-`supersedes` edges pointing at the record, each as source and label — with a note saying the
text has moved under them, that only the caller can tell whether a label survived, and that the
repair is on the *source* memory. `supersedes` is the one label left out, because it records that
this memory was replaced, which stays true however the replaced version reads. The edges come from
`Store::incoming_edges`, the same function the read surface projects, so the two cannot disagree
about the shape.

**Three mutations, and the pair is the point.** *Never fire* and *fire on everything* are equally
useless, and only having both directions under test tells them apart — so one mutation removes the
`content`/`why` scoping, one inverts it to fire on any change, and one puts `supersedes` back into
the report. Each is caught by exactly one test.

**Supersession needs the same remark for a different reason**, and it is filed as
`supersession-silently-repoints-an-in-link-nobody-reauthored` in [Issues](../../TRACKING.md#issues): the
superseded record is untouched, so no label is invalidated, but the walk resolves edges onto current
memories, so `C SUPPORTS A` is followed as `C SUPPORTS B` — a relationship neither author evaluated,
and one that leaves no artifact to inspect. **That is the in-link half only.** Since 2026-09-24 the
walk no longer carries `A`'s *own* links onto `B`: an out-link stays with the record that wrote it,
and a supersession reports the ones it leaves behind (`out-links-stay-with-the-record-that-wrote-them`).

**Built 2026-09-30.** A `hoard_memorize` call that supersedes returns `repointed_in_links`: each
memory pointing at one it replaced, as `superseded`, `source` and `label`, plus `scope` where the
source sits in another loaded set, since the walk re-points those too. A note says the links now
reach the replacement, that a claim written about the old text is being read as one about the new,
and that any repair belongs to the memory holding the link. **Both supersession labels are left
out**, not only `supersedes` as above: an in-coming `superseded_by` is a chain the walk is meant to
follow to its newest member, and reporting it would fire on every chain extension. The superseded
memories are the ones `link_deltas` selects, so the two reports agree on what a call replaced.

### The question as it was raised

*Kept because the alternative it proposed — abolish correction entirely — is what the resolution
above declines, and because its argument for immutability is what the resolution preserves.*

**Whether amendment should exist at all.** *Raised 2026-08-26 by the human partner, and recorded here because it questions this document's central division rather than any answer built on it.*

This document's work is a boundary: the claim is superseded, the apparatus is amended. **Two mechanisms for one act — updating a memory — and the boundary has already moved once**, when `origin` joined the amendable set on 2026-08-21. A line redrawn under pressure is a line being decided case by case.

The alternative is to supersede throughout and delete the distinction. What that buys reaches well beyond this document: records become **immutable**, so a record's checksum never changes, and [Multi-Set Sync](multi-set-sync.md)'s reconciliation collapses toward set union with validity resolved from the graph — which is what `git` does. It would also dissolve the one case that document cannot close, since amendment is the only operation whose changes carry no ancestry.

**What it collides with is in this document, and it is not a matter of taste.** *An unfair memory about a person is forgotten, not superseded*, above, holds that preserving the history of a characterisation is itself the injury — and `origin-cannot-be-corrected` reached the same conclusion for a misattribution of whose reasoning produced a memory. Superseding those keeps them permanently readable.

**The collision has an answer, and it makes `forget` do more rather than less.** Immutability would apply to *memories*, not to forgetting: a forgotten memory's **text** becomes unreadable — `content`, `why`, `context`, and `keywords`, which is authored prose carrying the same hazard — while the record's skeleton survives. The rule above is then satisfied in the strong form it actually wants, since the characterisation is gone rather than merely unrecalled, and the property the proposal exists for survives too: a memory's checksum never changes, because the memory ceases to exist and a tombstone takes its place.

**What must survive is the ancestry**, and that is a requirement rather than a preference. A memory's `SUPERSEDES` edge is its only link to what it replaced; today's `forget` cascades it away, which is
`forgetting-a-chains-middle-makes-a-stale-conclusion-current-again` in [Issues](../../TRACKING.md#issues) — a twice-replaced conclusion reporting itself as current. A defect today; fatal under a design where the chain is the whole history. **Keep what the graph needs; remove what a person wrote.**

Carried in [Open questions](../../TRACKING.md#open-questions), not decided here.

## Resolved 2026-08-29: correction may touch `content` — and only before service

*Closed by the rule above rather than on its own terms, which is why the deferral below never found
the case it was waiting for.*

The deferral turned on **verification**: *was this a typo, or a changed claim?* is a judgement the
software cannot check, and the failure directions are unequal. The service rule removes the question
instead of answering it. A record that never entered service has no belief to destroy, so it does
not matter which of the two it was; a record that entered service is superseded, so it does not
matter either. Neither branch needs the distinction the software could not verify.

**The concrete case that arrived on 2026-08-06 is now classified.** Transmission damage — a `why`
that swallowed its own closing tag and the parameter after it — is *neither a typo nor a changed claim*, and
that was the reason it fit no side of the old question. It is simply a record that never entered
service. The measurement found three such repairs in the live set, all done by supersession because
correction could not reach `content`, all within 61 seconds of the original; one of them says so in
its own `context`. That class is now refused at the tool boundary rather than stored damaged, so it
should not recur — but it is what the widened reach exists for.

*The deferral as it stood is kept below.*

**Whether amendment may touch `content`.**

By the principle above, a typo inside `content` is a faulty record and should therefore be amendable. `amend` nevertheless **excludes `content` and `why` initially**, and the reason is about verification rather than principle.

*Was this a typo, or a changed claim?* is a judgement the software cannot check, and the two failure directions are not equal: wrongly amending destroys a belief someone held. Until there is a real case that makes the distinction concrete, a typo in `content` is corrected by superseding — verbose, and lossless.

Revisit when the frequency is known. If content typos turn out to be common, the cost of superseding them is chain trivia at scale, and that would be an argument worth weighing against the one above.
