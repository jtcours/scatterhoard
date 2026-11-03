# Decision Placement

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

**Put each decision where both the information and the stake live.**

Scatterhoard is a collaboration between software, an LLM, and a human, each of which knows things the others don't. Most of the design questions that have been genuinely hard turned out to be questions about *which of the three should be deciding* — and several dissolved once that was answered.

## The check

Before adding a parameter, a field, a configuration knob, or a required choice, ask two things:

1. **Does whoever must decide have the information to decide correctly?**
2. **Does the answer change anything?**

If the first is no, the decision is misplaced — move it to whoever holds the information. If the second is no, the decision should not exist at all.

## Why a badly placed decision is worse than none

A control that *looks* usable but isn't gets used, and used wrongly. It also reads as a deliberate affordance, so a later reader assumes it was reasoned about and builds on it. An absent control at least prompts someone to ask why.

The same holds for a choice with no consequence: it produces inconsistent data, because different sessions will decide it differently and nothing ever corrects them. It also spends context making an arbitrary choice look principled.

## Worked examples

**Scope is an explicit parameter, not inferred.** ([Memory Steering](memory-steering.md)) The software cannot judge whether a memory belongs to the project or stays local — that needs an understanding of what the memory means. The LLM has that. So the decision moves to the LLM, and the type system refuses to let it be skipped.

**No "don't seed from this memory" control on recall.** ([Recall Mechanisms](recall-mechanisms.md)) Hubness is a property of the whole vector distribution; an LLM sees only what came back and cannot tell which memories are hubs. Handing it that control would be handing it a lever with no information behind it. The correction belongs in the ranking instead.

**No Personal/General split in scope.** ([Memory Steering](memory-steering.md)) The LLM could plausibly draw this line — but nothing downstream reads it. Both stay local, neither exports, and the export gate inspects project-scoped memories rather than local ones. A judgment call on every store, paying for nothing. Deleted rather than relocated.

**Dangling cross-set references are the LLM's call, not a hard failure.** ([Cross-Set References](cross-set-references.md)) Whether a missing reference is a problem depends on what the memory was for, which is exactly the context software lacks.

**Unexpected files in `scatterhoard_mem` are surfaced to a human, at a moment they can act.** ([Trigger and Loading](trigger-and-loading.md)) Neither the software nor the LLM can judge whether a stray file is innocent. The human can — but only the human who is about to commit, which is why the warning lands at the export gate rather than at session start.

## Relationship to correctness by construction

Once a decision's proper home is settled, the type system should make the wrong placement unrepresentable rather than merely discouraged — see the correctness-by-construction section of [STANDARDS.md](../../STANDARDS.md). `Scope` deriving `Default` would quietly undo the first example above, so it must not.
