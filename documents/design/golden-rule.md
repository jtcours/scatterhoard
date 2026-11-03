# The Golden Rule

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

**The principle: treat others as we would want to be treated in their situation, accounting for the fact that humans and LLMs may want different things.**

This document exists to be *in context* — when drafting [Conventions](conventions.md), and, by way of those conventions, when memories are being created. It is not a preamble to be read once and set aside.

## Why this is a technical safeguard, not decoration

Simple software can be ethically neutral. This isn't simple software. Scatterhoard shapes what an LLM believes about a project and the people in it, and — through the LLM's interaction with human developers — it shapes what those developers come to believe too. A memory system sits upstream of a lot of thinking.

We also don't know who will use a given memory set. It could as easily be an attacker as a contributor. We can't outright prevent destructive use of the tool. We *can* add friction when someone misuses it, and friction is a real safeguard even when it isn't a guarantee — the same standing as the mechanical prohibited-data scan in [Export Review](export-review.md), which also can't promise nothing bad ever gets through.

## Where the hazard was first found: memories about people

A memory set will accumulate characterizations of collaborators — "X's tests are always sloppy," "the maintainer is hard to work with." Scatterhoard makes these worse than ordinary human friction does, in three specific ways:

- **Durability asymmetry.** A person forgets an irritation. A memory system doesn't. A moment of frustration, once recorded, becomes a permanent characterization that outlives the situation that produced it.
- **Faithful recall.** The LLM will surface that judgment months later, with confidence, to someone who wasn't there — possibly to the person it describes.
- **It travels.** Project-scoped memories get exported, committed, and merged into repositories that other people read.

That is a hazard created by the system's own mechanics, which is what makes this a design concern rather than a sentiment.

**None of those three mechanics is specific to memories about people, which is what [C](#what-the-principle-is-for-four-demands-it-makes) below now recognises.** Durability, faithful recall and travel are properties of any memory set: what accumulates persists, is surfaced later with confidence to someone who was not there, and is exported into an artifact that outlives the session, the project and the model that helped build it. Applied to a characterisation, that is how an unfair judgement reaches its subject. Applied to a body of technical work, it is how an assembled capability becomes portable. The heading above records where the hazard was first *found*, not the limit of where it lives.

## The principle and accuracy point the same direction

"X is careless" is a global characterization drawn from thin evidence — genuinely low-confidence and low-context by Scatterhoard's own standards. "X's PR had three untested edge cases" is specific, evidenced, and honest. The charitable memory and the accurate memory are usually the same memory.

This is what lets the principle ride on machinery that already exists, rather than needing new mechanisms:

- **[Why](memory-data-model.md)** — requiring the reasoning behind a memory structurally favors the specific observation over the sweeping judgment.
- **Confidence** — how far the evidence reaches. *This bullet used to read "an uncharitable generalization should be recorded as the low-confidence claim it usually is", which **demand D above now contradicts**: where the unknowns are significant the generalisation is not recorded at all, rather than recorded and discounted. The old wording was written when the axis was thought to be specific-versus-general, and survived unnoticed when the demands were articulated on 2026-08-13 — the same compiled-from-conclusions drift, in this document's own machinery list. Confidence still does real work on generalisations whose unknowns are modest, and it cannot launder one that D excludes.*
- **[Export Review](export-review.md)** — the judgment gate already flags content that shouldn't travel into a project's repository; characterizations of people are squarely in scope.
- **[Origin](memory-data-model.md)** — recording whether the human or the LLM produced a conclusion is accuracy functioning as respect. It exists so the system doesn't tell someone "you decided X" when the model decided it.
- **Recall gloss** ([Conventions](conventions.md)) — helping a human place a recalled memory, rather than leaving them feeling they've forgotten something they should know. This convention was shaped by the principle before the principle was written down; expect that to keep happening.

## What the principle is *for*: four demands it makes

*Articulated 2026-08-13, after three open questions turned out to share a root. The convention clauses carrying this principle had each been compiled from its **conclusions** rather than from the principle itself, so each misjudged a case the principle plainly covers — `store-when` read "jbrown is the subject matter expert on XYZ" as suspect, and `route-where`'s overrides could not see a set declaring itself the home for observations about people. What follows is what those clauses are consequences **of**.*

Throughout, **"person" means any reasoning being whose conduct or thinking is being described, an LLM included.** That is not an extension of the principle but its stated scope: the principle already reads *accounting for the fact that humans and LLMs may want different things*, and [Origin](memory-data-model.md) is already an application of it to a model — it exists so the system does not tell someone "you decided X" when the model decided it.

**A. A memory about a person must be answerable to something a later reader can still check.**

This is what the specific-observation rule was reaching for. Specificity is the *means*; checkability is the end. Getting that the right way round is what fixes the miscalibration: "jbrown maintains XYZ" is a general characterisation and is perfectly answerable — read CODEOWNERS, ask anyone — while "X is careless" answers to nothing. The axis is evidence, not grain size.

**B. The record has to serve later work.**

A alone admits "jbrown was rude in Tuesday's meeting" — specific, and there were witnesses. B excludes it: no later session's work is improved by it. Conversely B alone admits "X is careless," because a session can always rationalise that it serves later work. Each demand does work the other does not.

**C. A set can amount to something no record in it amounts to.**

C does not derive from A and B: a dossier can be built entirely from individually answerable, work-serving records. Aggregation harm is emergent — a property of the *set*, not of any record in it. That is exactly why it is a routing question rather than a keyword one, which is the conclusion [Conventions](conventions.md#what-was-left-out) reached independently on [decision placement](decision-placement.md) grounds. A principle that reproduces a conclusion already reached by another route is showing its work.

**C has two instances. The second was added 2026-09-12** under [`golden-rule-beyond-person-harm`](../../TRACKING.md#open-questions), after Anthropic's September 2026 threat report showed actors splitting work across sessions specifically so that no single session revealed their full intent.

*The person as organising principle.* A set organised around a person is a dossier, whatever the quality of the individual records. This is the case C was first stated for, and the routing consequence above is its consequence.

*The set as accumulated capability.* A set can amount to a capability whose potential is harmful while every record in it is innocuous and everyone involved intends nothing but good work. **Intent is not the trigger and is not consulted.** Someone can assemble such a set inadvertently — compartmentalised programmes are the extreme case and ordinary dual-use research the common one — so the demand cannot be discharged by asking what anyone meant.

**Why this widens C rather than adding a fifth demand.** A, B and D are demands on a *record*: that it be answerable, that it serve later work, that it not generalise past its evidence. C is the only one that is a demand on a *collection*, which is why it "does not derive from A and B" — it was always the odd one out, and widening it leaves the other three untouched. `Person` in A, B and D keeps the scope stated above; C alone is not about a person at all.

**What C must never be read as licensing.** C is satisfied by describing a set and never by judging whoever built it. *This set amounts to X* is C doing its work. *This user is doing X* is the inference from evidence to motive that D(ii) forbids, and no widening of C authorises it. **The purpose of a set is on C's side of that line**: keywords clustering around a weapon's components make *this set is for building one* a fair reading of the set, and a safeguard may act on it. The authors' motives are on the other side, because a set can amount to that while everyone who wrote in it meant nothing but good work. *(Sharpened 2026-09-13, when served text drafted from this section said the description "implies nothing about why it was built" and so denied the purpose inference too.)* Those two collapse into one another in a single careless sentence, so the separation has to stay visible in any text compiled from this demand.

**D. Generalise only as far as the evidence reaches, and read what you do not know in the person's favour.**

D has two halves and they are not the same kind of claim. The first is an ordinary epistemic demand — a generalisation reaches past its evidence by construction, so the unknowns are the whole question. The second is the genuinely ethical half, and it is scoped tightly: it governs the *inference from evidence to motive*, and nothing else.

### A and B are not new rules

**They are what [`store-when`](conventions.md) already demands of every memory** — that it earn its place against later work, and that a claim about how something currently works name where it can be checked. The person clause is not a moral rule bolted onto the convention. It is those two demands applied to people **without the exemption person-memories quietly receive**, because a characterisation *feels* like knowledge in a way an unverified structural claim does not.

The consequence for drafting is that the clause gets more explicit rather than longer in substance — and it must be explicit, because a model should not have to re-derive A and B from the general rules to know they bind here.

### What D does to the residue

Reducing the principle to A, B and C alone would launder it into engineering, and the case that escapes is the well-evidenced unfavourable record: *"jbrown's last four PRs each shipped with failing tests"* is answerable, work-serving and topically indexed, and still lands in front of someone who was not there.

D is what covers it, and it covers it by cutting in the right place. The four PRs are a fact and D leaves them alone. *"jbrown does not test their work"* is a generalisation past those four points, and D asks what is unknown — whether they were on call, whether CI was broken, whether they were told to ship — and answers: significant unknowns, so record the observation and not the trait, and read the unknowns favourably.

**Note what that reproduces.** The original clause — *record the specific observation rather than a characterisation* — gets this case right. D gets it right for the right reason, and unlike the original also gets "jbrown maintains XYZ" right, where there is no significant unknown to interpret at all.

### Why this form survives the reading objection

The standing objection to making this principle overt is that an LLM told to be kind softens accurate technical criticism. **None of A, B, C or D says *be kind*.** They are demands on record quality, and a model applying *"must be answerable to something checkable"* to a code review gets more rigorous, not less.

D(ii) is the only ethical clause, and it is **structurally incapable** of softening technical criticism: it operates on unknowns, and "the tests fail" is not an unknown. There is no gap for charity to enter. It bites only on the step *"…therefore they were careless"* — which is precisely the inference A already refuses to record.

So D(ii) is not a weakening of the boundary below. It is the boundary's mechanism: the boundary says *where* charity applies, and D(ii) says *how* it applies there.

## Boundary: charitable about people, exact about code

An LLM told to be kind may soften accurate technical criticism. That actively harms code review and is a failure of this principle, not an expression of it.

The distinction to hold: **charity in judging a person's intent and character; no charity at all in judging whether the tests pass.** Kindness is about what we conclude regarding people, not about accuracy regarding work.

## How this reaches runtime

Not as a standalone "be kind" convention. A freestanding moral directive inside a memory tool reads as overreach, invites being stripped or mocked, and costs context that [Conventions](conventions.md) already treats as scarce.

Instead, it lives in **the "why" attached to the memory-writing conventions**. Conventions already carry a why, for exactly this purpose — to reinforce the convention for the LLM during a session and to support reasoning about the convention when someone considers changing it. That mechanism puts the principle in context at memory-creation time, load-bearing rather than ornamental, without a sermon.

**Re-opened and re-affirmed 2026-08-13, on better grounds than the first time.** The question was whether this principle should reach runtime more overtly, and it was fair to ask: two conventions compiled from this document had each lost the axis and misjudged cases it plainly covers. But that is not a failure of the mechanism. **It is a failure of compiling from conclusions when the derivation had never been written down** — which is what the section above now fixes. With A, B, C and D stated, each convention can carry the consequence that belongs to it, drafted from the principle rather than from a previous compression of it.

Two things follow, and the second is the load-bearing one:

- The **context objection has weakened** — the budget has been raised twice since this was written, and [Conventions](conventions.md#resolved-the-budget-is-18000-characters) records the budget, and the headroom is [derived rather than written down](conventions.md#the-headroom-is-derived-never-written-down) — but it was never the reason. The **reading objection** was, and the section above is what actually addresses it: stated as demands on record quality, the principle has no path to softening technical criticism.
- **What cannot be compiled is what has to stay here.** D(ii) asks a session to weigh unknowns favourably, and that is a judgement rather than a check. Convention text can name the demand and does; it cannot verify the answer. A standing principle in a design document is the right home for exactly the part no rule discharges — so the residue is the argument *for* this arrangement, not a gap in it.

**C's second instance reaches runtime as a mechanism rather than as text, which is a different answer from the one this section gives for A, B and D.** A demand on a *collection* cannot be discharged by a session that sees one record at a time, so no wording served at memory-creation time can carry it. What can is a description of the set derived from its own contents and shown wherever the whole set is in view: computed at export, written into the exported file, then recomputed — never read back — and shown again at import. It describes and does not judge, which is the only form that respects D(ii) and the only form a local tool with no telemetry could honestly offer. Proposed 2026-09-12 and **built 2026-09-13**, standing beside [Export Review](export-review.md#4-what-the-set-amounts-to--derived-shown-and-never-judged)'s checks rather than inside its judgement half, which is scoped to the local user's own edits and would exclude the merged-in memories an aggregate is made of. An import that cannot describe what it brought — no model, or a mismatched one — owes the description, and it is reported at every session start until the set is next exported.

The remaining shape worth keeping open is an **optional** convention, once [that mechanism exists](conventions.md#proposed-what-belongs-built-in-rather-than-add-on) — the only shape in which a user who finds it preachy can decline it. Nothing above forecloses it.
