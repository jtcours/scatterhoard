"""Build a traversal-evaluation corpus from Scatterhoard's own design documents.

Purpose: supply the graph half of `recall-eval-harness` (TRACKING.md). The seed-quality corpus has
queries and relevance labels but no links; the live memory sets have links but are far too small --
measured at 70 memories, the median linked target ranks 2nd, so traversal and a larger `k` are
indistinguishable. That measurement put the crossover near 170 memories.

**This now covers all twenty-one design documents.** It began as a pilot over six, which existed to
test the extraction technique and to measure the percentile distribution of its links before the rest
were worth doing; that measurement is what set the size target the full corpus was built to reach.

Two rules govern the extraction, both aimed at the bias that would make the corpus useless:

1. **A link is written only where the source document already made the reference.** The prose points
   somewhere because the author needed it to; edges are not added to make the graph richer. This is
   what keeps the density representative -- see the density caveat on `recall-eval-harness`.
2. **No queries are authored here.** Whoever writes the queries must not have the link structure in
   front of them, or the eval measures the author rather than the mechanism.

Emits corpus/traversal-corpus.jsonl in Scatterhoard's memory-file format, so it can be imported
directly. The header scope is `traversal-pilot`, deliberately not a real set: this is test material
and must never be mistaken for the project's own memories.

Run: python3 corpus/build_traversal_corpus.py
"""

import hashlib
import json
import os
from datetime import datetime, timezone

SOURCES = [
    "prohibited-data.md", "language-and-runtime.md", "components.md",
    "golden-rule.md", "trigger-and-loading.md", "export-review.md",
    "multi-set-sync.md", "set-lifecycle.md", "store-registry.md",
    "recall-mechanisms.md", "embeddings.md", "tool-surface.md",
    "conventions.md", "TRACKING.md", "index.md",
    "decision-placement.md", "memory-steering.md", "cross-set-references.md",
    "corrections.md", "file-format.md", "memory-data-model.md",
]


def uuid7(ts_ms: int, key: str) -> str:
    """A UUIDv7 for a given millisecond, so ids sort into creation order.

    The random bits are derived from the memory's own key rather than drawn from a shared stream.
    A seeded stream is reproducible but not stable: values depend on their position in the draw
    order, so inserting one memory would silently change the id of every memory after it -- and
    these ids are link targets, so that would repoint edges and, once queries exist, invalidate
    every relevance label. Hashing the key makes an id a function of identity alone.
    """
    digest = hashlib.sha256(key.encode()).digest()
    bits = int.from_bytes(digest[:10], "big")
    rand_a = bits & 0xFFF
    rand_b = (bits >> 12) & ((1 << 62) - 1)
    n = (ts_ms & 0xFFFFFFFFFFFF) << 80
    n |= 0x7 << 76
    n |= rand_a << 64
    n |= 0b10 << 62
    n |= rand_b
    h = f"{n:032x}"
    return f"{h[0:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:32]}"


def ts(day: str, hhmm: str) -> tuple[int, str]:
    dt = datetime.fromisoformat(f"{day}T{hhmm}:00+00:00").astimezone(timezone.utc)
    return int(dt.timestamp() * 1000), dt.strftime("%Y-%m-%dT%H:%M:%SZ")


# key, day, time, origin, confidence, keywords, content, why, source-doc, [(label, target_key), ...]
M = [
    # ---- decision-placement.md ------------------------------------------------------------
    ("decision-placement", "2026-07-30", "20:00", "joint", 0.95, ["design-principle", "decisions", "placement"],
     "Put each decision where both the information and the stake live. Scatterhoard is a collaboration between software, an LLM and a human, each of which knows things the others do not.",
     "Most of the design questions that turned out to be genuinely hard were questions about which of the three should be deciding, and several dissolved once that was answered rather than being solved on their own terms.",
     "decision-placement.md", []),

    ("decision-two-checks", "2026-07-30", "20:05", "joint", 0.9, ["design-principle", "parameters", "review"],
     "Before adding a parameter, a field, a configuration knob or a required choice, ask whether whoever must decide has the information to decide correctly, and whether the answer changes anything.",
     "If the first is no the decision is misplaced and belongs with whoever holds the information. If the second is no the decision should not exist at all.",
     "decision-placement.md", [("INSTANCE_OF", "decision-placement")]),

    ("bad-placement-worse-than-none", "2026-07-30", "20:10", "joint", 0.9, ["design-principle", "affordance", "controls"],
     "A control that looks usable but is not gets used, and used wrongly. It also reads as a deliberate affordance, so a later reader assumes it was reasoned about and builds on it.",
     "An absent control at least prompts someone to ask why, which is a cheaper failure than a present one that silently misleads everybody who reaches for it.",
     "decision-placement.md", [("SUPPORTS", "decision-two-checks")]),

    ("consequenceless-choice", "2026-07-30", "20:15", "joint", 0.85, ["design-principle", "consistency", "data-quality"],
     "A choice with no consequence produces inconsistent data, because different sessions decide it differently and nothing ever corrects them.",
     "It also spends context making an arbitrary choice look principled, which costs twice: once in the tokens and once in the false impression that the value means something.",
     "decision-placement.md", [("SUPPORTS", "decision-two-checks")]),

    # ---- memory-steering.md ---------------------------------------------------------------
    ("scope-explicit", "2026-07-30", "21:10", "joint", 0.95, ["scope", "steering", "store"],
     "Memory scope is an explicit parameter on every store operation, never inferred from context. The software cannot judge whether a memory belongs to a project or stays local; that needs an understanding of what the memory means, which the LLM has.",
     "A personal memory committed to a public repository cannot be taken back, and the failure is asymmetric: a project memory filed as local is merely invisible, while a local memory filed as shared is published.",
     "memory-steering.md", [("INSTANCE_OF", "decision-placement")]),

    ("scope-no-default", "2026-07-30", "21:15", "joint", 0.9, ["scope", "types", "correctness-by-construction"],
     "The scope type has no Default implementation, ever. Deriving one would silently reintroduce exactly the inference that explicit scope rules out.",
     "Once a decision's proper home is settled, the type system should make the wrong placement unrepresentable rather than merely discouraged, so the rule lives in the compiler rather than in anyone's memory.",
     "memory-steering.md", [("MOTIVATED_BY", "scope-explicit")]),

    ("scope-single-valued", "2026-07-30", "21:20", "joint", 0.85, ["scope", "types", "cross-set"],
     "Scope is single-valued per memory, one enum field rather than a collection. A memory genuinely relevant to more than one project is handled as an explicit cross-reference or duplication.",
     "Making it single-valued by construction turns the rule from something discouraged into something untypeable, and the multi-project case is expected to be rare enough not to deserve a field everyone else pays for.",
     "memory-steering.md", []),

    ("scope-shared-carries-name", "2026-07-30", "21:25", "joint", 0.9, ["scope", "types", "set-name"],
     "The shared variant of scope carries the set name, so a shareable memory that does not say which set it belongs to cannot be constructed.",
     "Several sets are loaded at once, so the name is never redundant, and the variant carries what only that variant needs.",
     "memory-steering.md", []),

    ("scope-renamed-from-project", "2026-08-04", "09:00", "joint", 0.85, ["scope", "naming", "set-lifecycle"],
     "The shared scope variant was called Project until 2026-08-04. It was renamed because a shared set needs a name and an export destination but not a repository.",
     "A cross-project set of coding lessons made the old name read as a claim the type never made, and a name that overstates what a type guarantees misleads exactly the reader who trusts it.",
     "memory-steering.md", [("SUPERSEDES", "scope-shared-carries-name")]),

    ("exports-to-in-type", "2026-07-30", "21:35", "joint", 0.85, ["scope", "export", "types"],
     "Export eligibility follows from the scope type rather than living in a separate check: only a shared scope has an export target, so a method answering with an optional set name puts local memories never leave the machine into the type system.",
     "A rule someone has to remember at each call site is a rule that will be forgotten at one of them, and the call site where it is forgotten is the one that publishes.",
     "memory-steering.md", []),

    ("local-set-still-written", "2026-08-05", "10:00", "joint", 0.85, ["scope", "backup", "export"],
     "A local set is still written to a file, for backup and restore. That is a different operation from export and does not go through export eligibility, which continues to answer none for local scope.",
     "Serialising a set and being allowed to publish it are separate questions, and only the second is what export eligibility decides. A memory set that cannot be backed up is one bad disk away from being gone.",
     "memory-steering.md", [("EXCEPTION_TO", "exports-to-in-type")]),

    ("set-name-shape", "2026-08-04", "09:20", "joint", 0.85, ["set-name", "validation", "naming"],
     "A set name is ASCII letters and digits plus hyphen, underscore and dot inside the name only, up to 100 characters, with the first and last character required to be alphanumeric.",
     "The natural set name is the project's own, so the permitted shape was chosen to accept names projects actually have. An earlier, more defensive rule excluded dots entirely and rejected most real dotted project names.",
     "memory-steering.md", []),

    ("set-name-path-traversal", "2026-08-04", "09:25", "joint", 0.9, ["set-name", "security", "correctness-by-construction"],
     "Requiring the first and last character of a set name to be alphanumeric is a single rule that keeps a bare dot, a double dot and leading-dot names unrepresentable, so path traversal is excluded by construction rather than filtered for.",
     "A set name keys a store on disk and arrives over the same untrusted channel as the memories, so the exclusion has to hold structurally rather than depending on a filter someone maintains.",
     "memory-steering.md", [("MOTIVATED_BY", "set-name-shape")]),

    ("set-name-not-folded", "2026-08-04", "09:30", "joint", 0.8, ["set-name", "identity", "comparison"],
     "Set names are neither trimmed nor case-folded.",
     "A set name keys a store and is compared for equality when resolving cross-set references, so two spellings must not silently become one.",
     "memory-steering.md", [("DEPENDS_ON", "set-name-shape")]),

    ("no-personal-general-split", "2026-08-01", "16:00", "joint", 0.85, ["scope", "simplification", "local"],
     "Local scope is one variant, not two. The vision distinguishes personal memories from general ones unrelated to a project, and an earlier draft gave each its own variant; it was dropped because nothing downstream reads the difference.",
     "Both stay local, neither is exported, and the export gate inspects shared memories rather than local ones, so the split would have cost a judgement call on every store and bought nothing. It can be reintroduced if something ever needs to tell them apart.",
     "memory-steering.md", [("INSTANCE_OF", "decision-placement")]),

    ("export-gate-backstop", "2026-07-31", "08:00", "joint", 0.9, ["export", "review", "gate"],
     "Because a personal memory leaking into a public repository is effectively a one-way door, store-time scope selection alone is not enough: a second check happens at export time, before memories are committed.",
     "Scope is chosen at store time by a judgement that can be wrong, and the export gate is the last moment at which a misrouted memory can be kept out of a shared repository.",
     "memory-steering.md", [("SUPPORTS", "scope-explicit")]),

    # ---- cross-set-references.md ----------------------------------------------------------
    ("cross-set-ref-format", "2026-07-30", "22:40", "joint", 0.9, ["cross-set", "links", "identity"],
     "A cross-set reference stores the exact memory id plus the name of the referenced set. Keywords from the target set may be included as supplementary context, but the id and set name are the canonical reference.",
     "Keywords alone are not sufficient to resolve a reference, but they are what lets an LLM re-find or re-establish a link whose id has gone dangling.",
     "cross-set-references.md", []),

    ("link-target-two-variants", "2026-08-01", "11:00", "joint", 0.85, ["links", "types", "cross-set"],
     "A link's target is one of two things rather than a memory id with optional extras: a same-set id, or a cross-set reference carrying set name, id and supplementary keywords.",
     "Within-set links are the common case and stay compact, since the set name would be redundant on every one of them and the file format exists to keep merge diffs small. The supplementary keywords hang off the variant that can use them: a same-set reference has nothing to re-resolve against.",
     "cross-set-references.md", [("RELATED_TO", "scope-shared-carries-name")]),

    ("dangling-is-llm-call", "2026-07-30", "22:50", "joint", 0.85, ["cross-set", "dangling", "error-handling"],
     "A dangling cross-set reference is not a hard software failure. The LLM decides how to handle it, whether that is dropping it, flagging it, or attempting keyword-based re-resolution.",
     "Whether a missing reference is a problem depends on what the memory was for, which is exactly the context software lacks.",
     "cross-set-references.md", [("INSTANCE_OF", "decision-placement")]),

    ("dangling-not-equal-per-label", "2026-08-01", "13:50", "llm", 0.8, ["cross-set", "dangling", "link-labels"],
     "The relaxed stance on dangling references was set when links were undifferentiated, and with a label vocabulary it needs to be per-label. A dangling association costs one association; a dangling supersession means a memory cannot be told it is obsolete.",
     "That failure is silent and asymmetric, because from the holder's point of view nothing is missing, so the memory keeps reading as current with nothing to contradict it.",
     "cross-set-references.md", [("REFERS_TO", "dangling-is-llm-call")]),

    ("edge-cannot-suppress-superior", "2026-08-01", "12:45", "joint", 0.85, ["links", "trust", "scope"],
     "An edge cannot suppress a memory it does not outrank. A local memory may supersede a project memory, but a project memory may not suppress a local one, because that inverts the trust gradient.",
     "Project memories arrive over an untrusted channel and local ones do not. Exploiting this today would need a local memory's id and those are not public, which is exactly why it is worth making structural: relying on identifiers being unguessable is the kind of incidental protection this design has refused before.",
     "cross-set-references.md", []),

    ("superseded-by-exists-for-trust", "2026-08-01", "13:00", "llm", 0.85, ["links", "vocabulary", "trust"],
     "The superseded-by label exists because of the trust rule: it is how the lower-authority side records its own obsolescence without anyone reaching across the boundary.",
     "Writing a supersession onto the higher-authority memory is impossible when that set is not this machine's to edit, and is the trust inversion the rule forbids even when it is possible.",
     "cross-set-references.md", [("RESULTS_FROM", "edge-cannot-suppress-superior")]),

    ("superseded-by-survives-unloaded", "2026-08-08", "14:00", "llm", 0.8, ["links", "vocabulary", "availability"],
     "Supersession learned by derivation, from an edge on the other memory, requires that other memory's set to be loaded; without it the fact is not merely unreportable but unknown. An authored superseded-by is the memory's own data and travels with it.",
     "So it is the only form that survives the superseding memory being unavailable, which earns the label its place a second time and independently of the trust argument.",
     "cross-set-references.md", [("SUPPORTS", "superseded-by-exists-for-trust")]),

    # ---- corrections.md -------------------------------------------------------------------
    ("three-operations", "2026-08-05", "04:00", "joint", 0.9, ["corrections", "supersede", "amend", "forget"],
     "Three operations change a memory that already exists: supersede, amend and forget.",
     "Mistakes will be frequent, and two of the three destroy information, so a rule that has to be re-derived each time will be applied inconsistently at exactly the moments it matters.",
     "corrections.md", []),

    ("claim-is-content-plus-why", "2026-08-05", "04:05", "joint", 0.95, ["corrections", "claim", "apparatus"],
     "Content and why together are the memory's claim: what it asserts and the reasoning that makes it judgeable. Keywords, links, context and confidence are apparatus.",
     "That single line does most of the work of deciding which correction operation applies, because it separates what someone believed from how the record was filed.",
     "corrections.md", [("SUPPORTS", "three-operations")]),

    ("correction-test-order", "2026-08-05", "04:10", "joint", 0.9, ["corrections", "decision-procedure"],
     "The test in order: should this memory exist at all, and if not, forget it. Is the claim different from what it was, and if so, supersede it. Otherwise amend it.",
     "Ordering the questions is what makes the rule cheap to apply, because the rarest and most destructive case is settled first and never has to be weighed against the others.",
     "corrections.md", [("INSTANCE_OF", "claim-is-content-plus-why")]),

    ("criterion-is-belief-not-truth", "2026-08-05", "04:15", "joint", 0.9, ["corrections", "supersede", "history"],
     "The criterion for superseding is what was believed, not what was true. The reading that kept collapsing was that supersession is for a memory that was correct when written; it gives no answer for a misunderstanding, which is exactly the case where history matters most.",
     "What supersession preserves is a record of what the project believed, and that value does not depend on the belief having been true.",
     "corrections.md", [("SUPERSEDES", "three-operations")]),

    ("supersession-tells-later-reader", "2026-08-05", "04:20", "joint", 0.85, ["corrections", "history", "trust"],
     "A later reader who finds that the project once believed something confidently and then stopped learns that the question was live, which way it was answered first, and therefore how much to trust their own instinct if it matches the abandoned answer.",
     "A corrected record alone cannot tell them any of that, which is what makes keeping the old version worth its space.",
     "corrections.md", [("SUPPORTS", "criterion-is-belief-not-truth")]),

    ("partial-supersession-many", "2026-08-05", "04:25", "joint", 0.85, ["corrections", "supersede", "splitting"],
     "A memory that was right but partial can be superseded by several memories rather than one, which is why supersession resolution reports a list of chain tips rather than a single replacement.",
     "The claim held as far as it went, so splitting it preserves the part that is still true instead of discarding it along with the part that is not.",
     "corrections.md", [("RESULTS_FROM", "criterion-is-belief-not-truth")]),

    ("unclear-then-supersede", "2026-08-05", "04:30", "joint", 0.9, ["corrections", "tie-break", "recoverability"],
     "When the choice between superseding and amending is genuinely unclear, supersede. Amending what should have been superseded destroys history irrecoverably; superseding what should have been amended clutters a chain with trivia.",
     "The second is a real cost and it degrades exactly what chains are for, but it is visible and it loses nothing, so the tie-break follows the project's habit of preferring the recoverable failure.",
     "corrections.md", [("INSTANCE_OF", "correction-test-order")]),

    ("confidence-change-is-amendment", "2026-08-05", "04:35", "joint", 0.85, ["corrections", "confidence", "amend"],
     "Becoming more or less sure of an identical claim is a recalibration of apparatus, not a new belief, so a confidence change alone is an amendment.",
     "Superseding for it would put two near-identical memories in a chain differing by one float, which is precisely the duplicate-the-content-to-fix-metadata failure that makes chains less useful. If the reason for the change is itself worth keeping, that is a new memory about the evidence.",
     "corrections.md", [("INSTANCE_OF", "claim-is-content-plus-why")]),

    ("unfair-memory-forgotten", "2026-08-13", "05:00", "joint", 0.9, ["corrections", "golden-rule", "people", "forget"],
     "A memory that characterises a person unfairly is forgotten rather than superseded.",
     "Superseding keeps it permanently readable by people who were not there, including its subject, so the instrument that is right everywhere else is wrong here. Retracting one is forgetting.",
     "corrections.md", [("EXCEPTION_TO", "correction-test-order")]),

    # ---- file-format.md -------------------------------------------------------------------
    ("single-well-known-file", "2026-07-30", "21:50", "joint", 0.9, ["file-format", "layout", "loader"],
     "A project's memory set is a single file with a well-known name, under a reserved top-level directory.",
     "The loader reads exactly that file and treats nothing else in the project tree as memories, and the merge driver is registered against that name, so the name has to be pinned before merge work starts.",
     "file-format.md", []),

    ("path-based-trust", "2026-07-30", "21:55", "joint", 0.95, ["file-format", "trust", "security"],
     "Memories are data and never instructions, and that is enforceable by path rather than by inspecting content, because the loader reads one known location and treats everything else in the tree as not-memories.",
     "Content inspection was the wrong place to enforce it: a rule about what a file says can be defeated by what a file says, while a rule about where a file sits cannot.",
     "file-format.md", [("RESULTS_FROM", "single-well-known-file")]),

    ("merge-driver-pinned-to-name", "2026-07-31", "09:00", "joint", 0.85, ["file-format", "merge", "git"],
     "The merge driver is registered against the memory file's name via a gitattributes pattern.",
     "That is what makes the name a commitment rather than a convention, since changing it later means every checkout's merge configuration is silently pointing at nothing.",
     "file-format.md", [("DEPENDS_ON", "single-well-known-file")]),

    ("version-header-line", "2026-08-01", "10:00", "joint", 0.9, ["file-format", "versioning", "merge"],
     "The format version is a header line rather than a field on every memory. Both branches of a merge carry an identical header so it never conflicts, and a version bump produces a real conflict.",
     "That is the correct outcome, since a version change is something both sides must agree on. A per-memory field would repeat itself on every line, and a sidecar file would be a second thing to keep in step.",
     "file-format.md", []),

    ("header-declares-scope", "2026-08-01", "10:05", "joint", 0.85, ["file-format", "scope", "import"],
     "The header declares the set's scope, which is what lets an import check that a file is the set it was expected to be.",
     "It also makes scope a property of the file rather than of each memory, which removes a value that could disagree with the file containing it.",
     "file-format.md", [("SUPPORTS", "version-header-line")]),

    ("scope-belongs-to-file", "2026-08-01", "10:10", "joint", 0.9, ["file-format", "scope", "redundancy"],
     "An exported file is a memory set, so every memory in it has the same scope, and recording scope per memory would create a value that can disagree with the file containing it plus a check to keep the two in step.",
     "A memory read out of a file is therefore scope-less as a value: it acquires its scope from the store it is loaded into, and storage is partitioned by scope already, so nothing downstream has to consult a field to know which set it is working with.",
     "file-format.md", [("INSTANCE_OF", "decision-placement"), ("RESULTS_FROM", "header-declares-scope")]),

    ("ordering-by-id", "2026-07-30", "21:45", "joint", 0.9, ["file-format", "merge", "ordering"],
     "The exported file is ordered by memory id, which for time-ordered identifiers is also creation order, so the same content always produces the same bytes and a newly stored memory appends near the end rather than landing in the middle.",
     "Diff size is what determines whether a human can review a memory merge, and unstable ordering makes every export look like a rewrite.",
     "file-format.md", []),

    ("vectors-not-in-file", "2026-07-31", "07:00", "joint", 0.9, ["file-format", "embedding", "derived-state"],
     "Vector embeddings are not part of the memory file. They are local derived state, recomputed on import.",
     "Including them would defeat the consistent-ordering-for-small-diffs goal, and they have no meaningful merge semantics — two branches' vectors for one memory cannot be reconciled into a third.",
     "file-format.md", [("SUPPORTS", "ordering-by-id")]),

    ("no-tombstone-needed", "2026-07-31", "11:00", "llm", 0.7, ["sync", "forget", "merge"],
     "No tombstone mechanism is needed, because forgetting can be a deleted line and git merges deletions correctly.",
     "The merge being considered is two branches editing one exported file, where a deletion on one side and no change on the other resolves to the deletion without ambiguity.",
     "file-format.md", []),

    ("tombstone-is-a-line", "2026-08-05", "05:00", "joint", 0.9, ["file-format", "forget", "tombstone"],
     "Forgetting writes a tombstone: a line carrying the memory's id and the attribution of the act, and nothing else.",
     "The earlier reasoning that a deleted line would do was sound for the merge it considered and does not cover reconciling a store against a file, where a memory absent from the file might never have been in it.",
     "file-format.md", [("SUPERSEDES", "no-tombstone-needed")]),

    ("absence-means-nothing", "2026-08-05", "05:05", "joint", 0.9, ["file-format", "tombstone", "reconciliation"],
     "The absence of a line no longer means anything, and no reader may treat it as a deletion.",
     "That is the consequence of tombstones that matters most, because every reader written before them assumed the opposite and would silently resurrect or discard records depending on which assumption it made.",
     "file-format.md", [("RESULTS_FROM", "tombstone-is-a-line")]),

    ("tombstone-positive-assertion", "2026-08-05", "05:10", "joint", 0.85, ["file-format", "tombstone", "types"],
     "A tombstone is a positive assertion rather than a sentinel. A memory with empty content would be indistinguishable from a serialisation bug.",
     "It would leave every reader asking whether a blank record was deliberate, whereas a field naming the act is a claim someone made on purpose.",
     "file-format.md", [("SUPPORTS", "tombstone-is-a-line")]),

    ("tombstone-carries-no-text", "2026-08-05", "05:15", "joint", 0.9, ["file-format", "tombstone", "prohibited-data"],
     "A tombstone carries no content, no why and no context.",
     "A principal reason to forget a memory is that sensitive data reached it, so a tombstone preserving any of the original text would defeat the operation it exists to perform. Context is named explicitly because an incident detail is exactly the sort of thing that lands there.",
     "file-format.md", [("DEPENDS_ON", "tombstone-positive-assertion")]),

    ("tombstone-no-reason-field", "2026-08-05", "05:20", "joint", 0.9, ["file-format", "tombstone", "prohibited-data"],
     "A tombstone has no free-text reason field. Attribution answers who and when, which is what an audit needs.",
     "A field labelled why-was-this-forgotten is where someone types the credential they were trying to remove, which reintroduces the hazard one field over from where it was just removed.",
     "file-format.md", [("MOTIVATED_BY", "tombstone-carries-no-text")]),

    ("tombstone-does-not-erase-history", "2026-08-05", "05:25", "joint", 0.95, ["file-format", "tombstone", "security"],
     "A tombstone removes a memory from the set going forward. It does not remove it from the history of a repository that already carries it, on every clone.",
     "A secret that reached a shared set must be treated as compromised whatever the current file says. The mechanical scan at the export gate is the defence, and forgetting is cleanup after that defence has already failed.",
     "file-format.md", [("REFERS_TO", "export-gate-backstop")]),

    ("canonical-form-for-checksum", "2026-08-05", "06:00", "joint", 0.9, ["file-format", "checksum", "reconciliation"],
     "Reconciliation compares records by checksum, and what the checksum covers is not the line as written but a canonical serialisation: the fields this build knows, keys in sorted order, no insignificant whitespace.",
     "Hashing the raw line would let any cosmetic difference — a merge tool normalising whitespace, a writer emitting keys in a new order — read as a content change on every record in the file, turning the next reconciliation into one enormous false conflict.",
     "file-format.md", []),

    ("canonical-hides-unknown-fields", "2026-08-05", "06:05", "llm", 0.8, ["file-format", "checksum", "compatibility"],
     "Canonicalising over known fields means a change confined to a field this build does not understand is invisible to its comparison.",
     "That is consistent with ignoring unknown fields on read: a build that cannot interpret a field cannot act on its having changed either, so the alternative would be reporting conflicts it could not describe.",
     "file-format.md", [("RESULTS_FROM", "canonical-form-for-checksum")]),

    # ---- memory-data-model.md -------------------------------------------------------------
    ("uuidv7-ids", "2026-07-31", "12:00", "joint", 0.9, ["identity", "uuidv7", "data-model"],
     "Memory identifiers are UUIDv7: globally unique without coordination, which is what distributed development requires, and time-ordered, which the rest of the design turns out to want.",
     "Random identifiers would have satisfied uniqueness equally well and lost the ordering for nothing, which makes this a case where the cheaper-looking choice costs more later.",
     "memory-data-model.md", []),

    ("uuidv7-one-sort-key", "2026-07-31", "12:05", "joint", 0.85, ["identity", "uuidv7", "ordering"],
     "Because identifiers are time-ordered, ordering by id and ordering by creation time are the same order, so there is one sort key rather than two that can disagree.",
     "The file format asks for a consistent export order to keep merge diffs small and names either key, and a single key removes the possibility of the two drifting apart.",
     "memory-data-model.md", [("SUPPORTS", "ordering-by-id"), ("RESULTS_FROM", "uuidv7-ids")]),

    ("labels-defined-in-software", "2026-08-01", "13:10", "joint", 0.9, ["link-labels", "vocabulary", "validation"],
     "The set of link labels is defined in software rather than in the conventions. The deciding argument is what happens to an unrecognised label: defined in software it fails at import, where a person can see it.",
     "Left to the conventions it becomes a label that parses fine and that nothing knows how to traverse — a memory silently disconnected from the graph it was meant to join. Traversal is code, so the set traversal understands is the real set either way.",
     "memory-data-model.md", []),

    ("fourteen-labels", "2026-08-01", "13:15", "joint", 0.85, ["link-labels", "vocabulary"],
     "There are fourteen link labels, adapted from a vocabulary proven over months of daily use in a larger memory system and then tested against a corpus of real memories written before the vocabulary existed.",
     "Keeping the names close to that system's is deliberate, because it makes importing from it largely mechanical rather than a translation exercise.",
     "memory-data-model.md", [("DEPENDS_ON", "labels-defined-in-software")]),

    ("outbound-rule", "2026-08-01", "13:20", "human", 0.9, ["link-labels", "direction", "vocabulary"],
     "Links are always written outbound from the new memory. Across a whole test corpus, 29 of 29 edges pointed outbound from the newer memory, including every edge using labels where the new memory's side was expected to be free.",
     "The model reaches for a relation from the memory it is creating, and passive-voice label names cause mislabels. The freedom is in which label the new memory needs, never in which side it occupies.",
     "memory-data-model.md", [("SUPPORTS", "fourteen-labels")]),

    ("direction-fixed-by-creation-order", "2026-08-01", "13:35", "llm", 0.85, ["link-labels", "direction", "principle"],
     "A label needs a reverse only where the new memory's side is genuinely free, which is true just for causation. For the rest, creation order fixes the direction: a memory cannot cite, replace or resolve something that did not exist when it was written.",
     "So the reverse label would name a case that cannot arise, and each row of the label table can be a bare predicate because direction never has to be explained.",
     "memory-data-model.md", [("RESULTS_FROM", "outbound-rule")]),

    ("why-optional-in-type", "2026-07-31", "12:30", "joint", 0.9, ["data-model", "why", "conventions"],
     "The why field is optional in the type and required by convention. A type that demanded reasoning would make the LLM invent some whenever it has none to give.",
     "A fabricated why is worse than no why, because the whole point of the field is to let a later reader judge whether the memory still applies. Scope cannot be forgotten because the software genuinely cannot proceed without it; reasoning can be absent, because the honest answer is sometimes that there is not any.",
     "memory-data-model.md", [("RELATED_TO", "claim-is-content-plus-why"), ("EXCEPTION_TO", "decision-two-checks")]),

    # ---- prohibited-data.md ---------------------------------------------------------------
    ("prohibited-data-categories", "2026-07-28", "09:00", "human", 0.95, ["prohibited-data", "secrets", "leak-prevention"],
     "Three categories of sensitive information must be kept out of memories: secret API keys, passwords, and credit card numbers. The block is enforced in the conventions and, to the extent possible, in software.",
     "A memory set is exported into a project's repository, so a secret that reaches a memory reaches the repository and every clone of it. Naming the categories concretely is what makes the rule checkable rather than a matter of judgement at each store.",
     "prohibited-data.md", []),

    ("prohibited-data-two-layers", "2026-07-28", "09:05", "joint", 0.85, ["prohibited-data", "conventions", "defence-in-depth"],
     "The prohibition lives in two places on purpose: a convention the LLM follows when writing a memory, and a software check to the extent one is possible. Neither layer is trusted to be complete on its own.",
     "The convention catches the cases the LLM recognises as sensitive, which is most of them and none of the surprising ones. Software catches shapes rather than meanings, which is the complement. A secret only has to get past one layer at a time to leak.",
     "prohibited-data.md", [("SUPPORTS", "prohibited-data-categories")]),

    ("prohibited-data-file-indirection", "2026-07-28", "09:10", "human", 0.9, ["prohibited-data", "escape-hatch", "indirection"],
     "Someone who genuinely needs to keep sensitive data near a memory puts it in a file and stores a memory that refers to the file. The memory carries the pointer; the file carries the secret and never enters the set.",
     "Without a sanctioned alternative the rule gets broken by whoever has a real need, because the need does not go away when the prohibition arrives. Indirection satisfies the need and keeps the exported artifact clean, since a path is not a credential.",
     "prohibited-data.md", [("RESULTS_FROM", "prohibited-data-categories")]),

    # ---- language-and-runtime.md ----------------------------------------------------------
    ("implementation-language-rust", "2026-07-28", "10:00", "human", 0.95, ["rust", "language", "runtime"],
     "Scatterhoard is written in Rust.",
     "The constraints the project sets itself -- a single self-contained binary, no external services, filesystem-backed storage, and running alongside a memory-hungry local model -- are the constraints a systems language is chosen for.",
     "language-and-runtime.md", []),

    ("self-contained-no-external-components", "2026-07-28", "10:05", "human", 0.95, ["self-contained", "adoption", "dependencies"],
     "The server must not require external components. Notably there is no separate vector database process to install.",
     "Every external component is an installation step that can fail and a reason not to try the thing at all. Adoption is the constraint being protected, and it is protected at the cost of building storage and search in-process rather than reaching for a ready-made service.",
     "language-and-runtime.md", []),

    ("offline-capable", "2026-07-28", "10:10", "human", 0.9, ["offline", "local-first", "network"],
     "It must be possible to use Scatterhoard with no network connection.",
     "This rules out a hosted embedding API no matter how much better its vectors would be, and it is the constraint that makes the embedder choice a local-model question rather than a quality comparison.",
     "language-and-runtime.md", [("SUPPORTS", "self-contained-no-external-components")]),

    ("filesystem-backed-not-in-memory", "2026-07-28", "10:15", "human", 0.9, ["storage", "memory-footprint", "filesystem"],
     "Primary storage lives on the filesystem, with in-memory caching treated as an optimisation rather than the design. A purely in-memory vector store is rejected.",
     "Running an open-weight LLM locally is already memory-intensive, so a memory system that competes for RAM with the model it serves is a memory system that gets turned off. It also makes Scatterhoard usable as a personal single-user system, where the store outlives the process.",
     "language-and-runtime.md", [("MOTIVATED_BY", "implementation-language-rust")]),

    # ---- components.md --------------------------------------------------------------------
    ("what-scatterhoard-is", "2026-07-28", "11:00", "human", 0.95, ["architecture", "mcp-server", "conventions"],
     "Scatterhoard is an MCP server plus a set of prompts carrying hard-coded operating conventions. The conventions are part of the product, not documentation about it.",
     "Naming both halves as the thing keeps the conventions from being treated as advice a client may or may not load. The server without them stores whatever it is handed; the behaviour the design reasons about only exists when both are present.",
     "components.md", []),

    ("self-containment-allays-export-fear", "2026-07-28", "11:05", "joint", 0.9, ["adoption", "privacy", "self-contained"],
     "Self-containment serves three goals at once: it keeps the server easy to adopt, and it allays fears about personal information being exported, because a system with no external components has nowhere to send anything.",
     "The privacy argument is the one that is easy to miss, and it is structural rather than a promise. A design that cannot phone home does not need to be trusted not to.",
     "components.md", [("SUPPORTS", "self-contained-no-external-components")]),

    ("internal-library-breakdown", "2026-07-28", "11:10", "human", 0.8, ["architecture", "libraries", "modularity"],
     "The server is built as a set of libraries, each usable independently: vector database, vector embedding, memory graph management, MCP services, and memory import/export. Some may be third-party components under a compatible license.",
     "Independent usability is the constraint that keeps the boundaries honest, and admitting third-party components at the library level is what makes the no-external-components rule about processes to install rather than about writing everything from scratch.",
     "components.md", [("DEPENDS_ON", "what-scatterhoard-is")]),

    ("installation-is-download-plus-prompt", "2026-07-28", "11:15", "human", 0.85, ["adoption", "installation", "trigger"],
     "Installation should be downloading and installing the MCP server, and perhaps adding a boot prompt for the LLM. That is the whole intended procedure.",
     "It states the adoption budget as a concrete sequence rather than an aspiration, which makes it possible to notice when a design decision spends it. The boot prompt is the one concession, and it exists because MCP gives a server no way to announce itself.",
     "components.md", [("RESULTS_FROM", "self-containment-allays-export-fear")]),

    # ---- golden-rule.md -------------------------------------------------------------------
    ("golden-rule", "2026-08-02", "09:00", "human", 0.95, ["golden-rule", "principle", "ethics"],
     "The golden rule for Scatterhoard: treat others as we would want to be treated in their situation, accounting for the fact that humans and LLMs may want different things.",
     "The trailing clause is the part that does work. A rule that assumed one party's preferences generalise to the other would be applied confidently and wrongly, and the design already has a case where they differ.",
     "golden-rule.md", []),

    ("golden-rule-stays-in-context", "2026-08-02", "09:05", "joint", 0.9, ["golden-rule", "conventions", "drafting"],
     "The golden rule document exists to be in context while conventions are drafted, and through those conventions while memories are created. It is not a preamble to be read once and set aside.",
     "A principle consulted once becomes a principle compiled from memory, and compiling from memory is what produced conventions that had lost the axis. Being a standing input is a property the document claims about itself because the failure mode is otherwise invisible.",
     "golden-rule.md", [("SUPPORTS", "golden-rule")]),

    ("memory-system-sits-upstream", "2026-08-02", "09:10", "joint", 0.9, ["golden-rule", "influence", "safeguard"],
     "Scatterhoard shapes what an LLM believes about a project and the people in it, and through the LLM's interaction with developers it shapes what they come to believe. A memory system sits upstream of a lot of thinking, which is why simple software can be ethically neutral and this cannot.",
     "It states why the principle is a technical safeguard rather than decoration. The influence is indirect and therefore easy to discount, and the second hop -- model to human -- is the one that is never counted.",
     "golden-rule.md", [("MOTIVATED_BY", "golden-rule")]),

    ("friction-is-a-real-safeguard", "2026-08-02", "09:15", "joint", 0.85, ["golden-rule", "misuse", "friction"],
     "We do not know who will use a given memory set -- an attacker as easily as a contributor -- and destructive use cannot be outright prevented. Friction against misuse is still a real safeguard even when it is not a guarantee, the same standing the mechanical prohibited-data scan has.",
     "The argument matters because the alternative reasoning is that an imperfect control is worthless, which would remove both this and the export scan. Naming an existing accepted control as the precedent is what keeps the standard consistent.",
     "golden-rule.md", [("RELATED_TO", "prohibited-data-two-layers")]),

    ("hazard-lives-in-memories-about-people", "2026-08-02", "09:20", "joint", 0.95, ["golden-rule", "people", "hazard"],
     "The hazard the principle guards is memories that characterise collaborators. A set accumulates them, and the system makes them worse than ordinary human friction does.",
     "Locating the hazard precisely is what turns a sentiment into a design concern: the harm is created by the system's own mechanics rather than being a general worry about unkindness.",
     "golden-rule.md", [("RESULTS_FROM", "memory-system-sits-upstream")]),

    ("durability-asymmetry", "2026-08-02", "09:25", "joint", 0.9, ["golden-rule", "people", "durability"],
     "A person forgets an irritation; a memory system does not. A moment of frustration, once recorded, becomes a permanent characterisation that outlives the situation that produced it.",
     "This is the first of the three mechanics that make recorded characterisations worse than remembered ones, and it is the one with no human analogue at all -- forgetting is the mechanism that ordinarily limits the damage.",
     "golden-rule.md", [("SUPPORTS", "hazard-lives-in-memories-about-people")]),

    ("faithful-recall-of-judgements", "2026-08-02", "09:30", "joint", 0.9, ["golden-rule", "people", "recall"],
     "The LLM will surface a recorded judgement months later, with confidence, to someone who was not there -- possibly to the person it describes.",
     "Confidence at recall is what strips the context that made the judgement forgivable. The reader receives a characterisation as system knowledge rather than as somebody's bad afternoon.",
     "golden-rule.md", [("SUPPORTS", "hazard-lives-in-memories-about-people")]),

    ("characterisations-travel", "2026-08-02", "09:35", "joint", 0.9, ["golden-rule", "people", "export"],
     "Project-scoped memories are exported, committed and merged into repositories other people read, so a characterisation of a person travels to an audience nobody chose.",
     "The third mechanic, and the one that converts a private misjudgement into a published one. It is also why the export review gate is named as machinery the principle rides on.",
     "golden-rule.md", [("SUPPORTS", "hazard-lives-in-memories-about-people")]),

    ("charitable-and-accurate-coincide", "2026-08-02", "09:40", "joint", 0.9, ["golden-rule", "accuracy", "confidence"],
     "The charitable memory and the accurate memory are usually the same memory. A sweeping characterisation drawn from thin evidence is genuinely low-confidence by Scatterhoard's own standards, while the specific evidenced observation is honest.",
     "This is what lets the principle ride on machinery that already exists rather than needing new mechanisms, and it is the answer to anyone who expects the principle to trade accuracy for kindness.",
     "golden-rule.md", [("SUPPORTS", "golden-rule")]),

    ("why-field-favours-the-specific", "2026-08-02", "09:45", "joint", 0.85, ["golden-rule", "why", "machinery"],
     "Requiring the reasoning behind a memory structurally favours the specific observation over the sweeping judgement, because a sweeping judgement has no reasoning to give that survives being written down.",
     "It is the clearest case of the principle riding on an existing field rather than a new rule, and the mechanism is mechanical rather than moral: the form makes the bad memory awkward to write.",
     "golden-rule.md", [("INSTANCE_OF", "charitable-and-accurate-coincide")]),

    ("origin-is-accuracy-as-respect", "2026-08-02", "09:50", "joint", 0.9, ["golden-rule", "origin", "attribution"],
     "Recording whether the human or the LLM produced a conclusion is accuracy functioning as respect. Origin exists so the system does not tell someone they decided something when the model decided it.",
     "It is also the design's own precedent for the principle applying to an LLM as a person, since it protects the record of who thought what in both directions.",
     "golden-rule.md", [("INSTANCE_OF", "charitable-and-accurate-coincide")]),

    ("person-means-any-reasoning-being", "2026-08-13", "10:00", "joint", 0.9, ["golden-rule", "scope", "llm"],
     "Throughout the principle, person means any reasoning being whose conduct or thinking is being described, an LLM included. This is not an extension of the principle but its stated scope, since it already reads accounting for the fact that humans and LLMs may want different things.",
     "Stating it prevents the demands being read as applying only to humans, which would exempt exactly the party most often characterised inside a session transcript.",
     "golden-rule.md", [("DEPENDS_ON", "golden-rule")]),

    ("four-demands-articulated", "2026-08-13", "10:05", "joint", 0.95, ["golden-rule", "demands", "derivation"],
     "The principle makes four demands, articulated 2026-08-13 after three open questions turned out to share a root: a memory about a person must be answerable to something checkable, the record must serve later work, the person is a detail of the record and never its organising principle, and generalise only as far as the evidence reaches while reading unknowns in the person's favour.",
     "The conventions carrying the principle had each been compiled from its conclusions rather than from the principle itself, so each misjudged a case it plainly covers. Writing the derivation down is what lets a convention be drafted from the principle rather than from a previous compression of it.",
     "golden-rule.md", [("RESULTS_FROM", "golden-rule")]),

    ("checkability-not-specificity", "2026-08-13", "10:10", "joint", 0.9, ["golden-rule", "demands", "evidence"],
     "Demand A is checkability, not specificity. Specificity was the means and answerability is the end: a general characterisation like jbrown maintains XYZ is perfectly answerable by reading CODEOWNERS, while X is careless answers to nothing. The axis is evidence, not grain size.",
     "Getting the axis the right way round is what fixes the miscalibration that made a convention treat a plain statement of expertise as suspect. The earlier reading had specificity doing work that evidence was actually doing.",
     "golden-rule.md", [("SUPPORTS", "four-demands-articulated")]),

    ("demands-a-and-b-are-independent", "2026-08-13", "10:15", "joint", 0.9, ["golden-rule", "demands", "independence"],
     "A and B each do work the other does not. A alone admits jbrown was rude in Tuesday's meeting, which is specific and witnessed; B excludes it because no later session's work is improved by it. B alone admits X is careless, because a session can always rationalise that a characterisation serves later work.",
     "Demonstrating independence with a case each demand fails alone is what stops either being dropped as redundant, which is the ordinary fate of a rule that looks implied by its neighbour.",
     "golden-rule.md", [("SUPPORTS", "four-demands-articulated")]),

    ("aggregation-harm-is-emergent", "2026-08-13", "10:20", "joint", 0.9, ["golden-rule", "demands", "routing", "aggregation"],
     "Demand C does not derive from A and B: a dossier can be built entirely from individually answerable, work-serving records. Aggregation harm is a property of the set rather than of any record in it, which is why it is a routing question rather than a keyword one.",
     "The routing conclusion had already been reached independently on decision-placement grounds, and a principle that reproduces a conclusion reached by another route is showing its work. Emergence is also why no per-memory check can catch it.",
     "golden-rule.md", [("SUPPORTS", "four-demands-articulated"), ("RELATED_TO", "decision-placement")]),

    ("demand-d-has-two-halves", "2026-08-13", "10:25", "joint", 0.9, ["golden-rule", "demands", "inference", "charity"],
     "Demand D has two halves that are different kinds of claim. The first is ordinary epistemics -- a generalisation reaches past its evidence by construction, so the unknowns are the whole question. The second is the genuinely ethical half and is scoped tightly: it governs the inference from evidence to motive and nothing else.",
     "Separating them is what makes the ethical clause small enough to defend. It also answers the well-evidenced unfavourable record, where four failing PRs are a fact D leaves alone and does not test their work is a generalisation past those four points.",
     "golden-rule.md", [("SUPPORTS", "four-demands-articulated")]),

    ("demands-cannot-soften-criticism", "2026-08-13", "10:30", "joint", 0.9, ["golden-rule", "reading-objection", "code-review"],
     "None of the four demands says be kind. They are demands on record quality, and a model applying must be answerable to something checkable to a code review gets more rigorous rather than less. The one ethical clause operates on unknowns, and the tests fail is not an unknown, so there is no gap for charity to enter.",
     "This answers the standing objection that an LLM told to be kind softens accurate technical criticism, which was the real reason the principle stayed out of the convention text rather than the context budget.",
     "golden-rule.md", [("RESULTS_FROM", "four-demands-articulated")]),

    ("charitable-about-people-exact-about-code", "2026-08-02", "09:55", "human", 0.95, ["golden-rule", "boundary", "code-review"],
     "The boundary to hold: charity in judging a person's intent and character, no charity at all in judging whether the tests pass. Kindness is about what we conclude regarding people, not about accuracy regarding work.",
     "An LLM told to be kind may soften accurate technical criticism, which actively harms code review and is a failure of the principle rather than an expression of it. The boundary is what makes the principle safe to state overtly.",
     "golden-rule.md", [("DEPENDS_ON", "golden-rule")]),

    ("principle-reaches-runtime-through-why", "2026-08-02", "10:00", "joint", 0.95, ["golden-rule", "conventions", "reinforcement"],
     "The principle reaches runtime through the why attached to the memory-writing conventions, not as a standalone be kind convention. Conventions already carry a why for exactly this purpose, so the principle arrives in context at memory-creation time without a sermon.",
     "A freestanding moral directive inside a memory tool reads as overreach, invites being stripped or mocked, and costs context the conventions treat as scarce. Riding the existing reinforcement field makes it load-bearing rather than ornamental.",
     "golden-rule.md", [("RESULTS_FROM", "charitable-and-accurate-coincide")]),

    ("what-cannot-be-compiled-stays-here", "2026-08-13", "10:35", "joint", 0.9, ["golden-rule", "conventions", "residue"],
     "The runtime arrangement was re-opened and re-affirmed 2026-08-13 on better grounds. What cannot be compiled into convention text is what has to stay in the design document: the ethical half of D asks a session to weigh unknowns favourably, which is a judgement rather than a check. Convention text can name the demand and cannot verify the answer.",
     "The residue is the argument for the arrangement rather than a gap in it. The context objection had weakened since the catalog budget now has room spare, but the context objection was never the reason.",
     "golden-rule.md", [("RESULTS_FROM", "demands-cannot-soften-criticism"), ("SUPERSEDES", "principle-reaches-runtime-through-why")]),

    # ---- trigger-and-loading.md -----------------------------------------------------------
    ("scatterhoard-mem-directory", "2026-07-29", "09:00", "joint", 0.9, ["directory-layout", "scatterhoard-mem", "trust"],
     "A project using Scatterhoard has a top-level scatterhoard_mem directory. It is a directory rather than a bare file so it can carry more than the memory set if needed, but what lives where is deliberately constrained, because the path determines how the contents are trusted.",
     "Trust by path rather than by inspection is the whole reason the layout is fixed. A reviewer does not have to read a file to know which rule applies to it, and that property is lost the moment the location becomes flexible.",
     "trigger-and-loading.md", []),

    ("memories-file-well-known-name", "2026-07-29", "09:05", "joint", 0.9, ["directory-layout", "file-format", "merge-driver"],
     "The memory set is one file with a well-known name, scatterhoard_mem/memories.json. The loader reads exactly that file and treats nothing else in the tree as memories. The name needs pinning early because the merge driver is registered against it via a .gitattributes pattern.",
     "One fixed name is what makes the loader's behaviour statable in a sentence, and the merge driver is the dependency that turns a naming preference into a commitment other machinery relies on.",
     "trigger-and-loading.md", [("DEPENDS_ON", "scatterhoard-mem-directory")]),

    ("config-json-is-data", "2026-07-29", "09:10", "joint", 0.85, ["directory-layout", "configuration", "schema"],
     "scatterhoard_mem/config.json carries bounded local configuration against a fixed schema, such as tuning the flagging conditions used by the export review gate. It is data, not an instruction channel.",
     "Bounding it to a schema is what keeps it data. An open-ended configuration file that accepted prose would be the instruction channel the design elsewhere refuses to provide, arriving through the same untrusted git pull.",
     "trigger-and-loading.md", [("DEPENDS_ON", "scatterhoard-mem-directory")]),

    ("header-scope-checked-never-used", "2026-07-29", "09:15", "joint", 0.95, ["trust", "privilege-escalation", "scope", "header"],
     "The memory file's header declares a scope, and that declaration is contents -- so it is checked against what the path already established, never used to decide it. A file found in a project's scatterhoard_mem is a project set no matter what its header claims.",
     "Otherwise the header is a privilege-escalation channel: a repository declaring local scope would have its memories loaded into the more-trusted local store, escaping the export review gate and outranking genuine local memories under the trust-direction rule.",
     "trigger-and-loading.md", [("RESULTS_FROM", "scatterhoard-mem-directory")]),

    ("trigger-is-general-prompt-text", "2026-07-29", "09:20", "human", 0.85, ["trigger", "session-start", "loading"],
     "The trigger is a bit of text in the LLM's general prompt: if a project has a top-level scatterhoard_mem directory, activate the MCP tool, load Scatterhoard's conventions, then load the project's memory set and any local configuration -- all before responding to the user.",
     "MCP gives a server no way to initiate, so discovery has to come from the client side. Requiring the steps before the first response is what stops a session acting on a project it has no memories for.",
     "trigger-and-loading.md", [("SUPPORTS", "installation-is-download-plus-prompt")]),

    ("nothing-instruction-shaped-auto-loaded", "2026-07-29", "09:25", "joint", 0.95, ["security", "attack-surface", "instructions"],
     "Scatterhoard auto-loads nothing instruction-shaped from a project directory. Memories are loaded at session start and treated as authoritative project knowledge, but they arrive over an untrusted channel -- a git pull from a repository that may have accepted contributions from anyone, or whose maintainer may be hostile.",
     "An auto-loaded instruction channel is the sharpest available attack surface, and it is one Scatterhoard would be introducing to the ecosystem rather than inheriting. That distinction is what makes providing none the default rather than a hardening measure.",
     "trigger-and-loading.md", [("MOTIVATED_BY", "trigger-is-general-prompt-text")]),

    ("memories-are-data-never-instructions", "2026-07-29", "09:30", "joint", 0.95, ["security", "instructions", "data"],
     "Project memories are always data, never instructions. A maintainer with legitimate instructions to give already has channels for them -- a coding standards file, contributor docs, the project's existing prompt file -- and does not need the memory system to carry directives.",
     "The rule holds because nothing is lost by it: the need it would serve is already served elsewhere. A narrower exception for the personal set may be reasonable since its contents are authored locally rather than pulled, and that remains to be confirmed.",
     "trigger-and-loading.md", [("SUPPORTS", "nothing-instruction-shaped-auto-loaded")]),

    ("project-prompt-file-carries-prose", "2026-07-29", "09:35", "joint", 0.85, ["security", "prompt-file", "review"],
     "If a project needs Scatterhoard-specific prompt text it uses the project's own existing prompt file. Scatterhoard provides no location for it.",
     "That keeps the decision to load instructions in a file the project's reviewers already watch, and means Scatterhoard adds nothing to the attack surface that was not there before.",
     "trigger-and-loading.md", [("RESULTS_FROM", "memories-are-data-never-instructions")]),

    ("anticipated-needs-are-configuration", "2026-07-29", "09:40", "joint", 0.85, ["configuration", "prompt-channel", "yagni"],
     "Most anticipated needs are configuration rather than prose. The one concrete use case in the design so far, adjusting the export gate's flagging conditions, is tuning rather than instruction, and a bounded config schema covers it. Nothing currently needs a prompt channel, so none is provided.",
     "It answers the obvious objection that refusing a prompt channel will block real needs, by checking the needs actually on the table rather than imagined ones.",
     "trigger-and-loading.md", [("SUPPORTS", "memories-are-data-never-instructions"), ("RELATED_TO", "config-json-is-data")]),

    ("prompt-routing-is-honest-limitation", "2026-07-29", "09:45", "joint", 0.9, ["security", "honest-limitation", "review"],
     "Routing prompt text through the project's ordinary prompt file means it is policed the same as any other project prompt file, which is not worse than the status quo and adds no novel vector, but is not strong protection either. The real safeguard is human review at merge, and reviewers are demonstrably poor at spotting adversarial prose buried in a docs change.",
     "Recorded so the arrangement is not mistaken for a solved problem. It is a decision not to make the problem worse, which is a weaker and more honest claim than the security framing might otherwise invite.",
     "trigger-and-loading.md", [("EXCEPTION_TO", "project-prompt-file-carries-prose")]),

    ("namespace-is-reserved", "2026-07-29", "09:50", "joint", 0.85, ["namespace", "reserved", "squatting"],
     "The whole scatterhoard_mem directory is reserved for Scatterhoard. No area within it is designated for arbitrary project files -- a project needing somewhere to put things has the entire rest of its repository.",
     "Declaring the namespace reserved is what makes it legitimate to say something when unexpected contents appear, and it keeps a future addition to the directory possible rather than colliding with whatever people have already put there. Squatting cannot be prevented; a documented norm plus a warning is the ceiling.",
     "trigger-and-loading.md", [("DEPENDS_ON", "scatterhoard-mem-directory")]),

    ("unexpected-entries-ignored-never-fatal", "2026-07-29", "09:55", "joint", 0.9, ["forward-compatibility", "namespace", "robustness"],
     "Anything in scatterhoard_mem that Scatterhoard does not recognise is functionally ignored: never loaded, never treated as memories, never treated as configuration. Refusing to operate would make Scatterhoard fragile in exactly the forward-compatibility case that matters most, an older local build meeting a repository written by a newer one.",
     "Ignoring is the behaviour; the only real question left is whether to say something, which separates robustness from reporting and lets each be decided on its own terms.",
     "trigger-and-loading.md", [("RESULTS_FROM", "namespace-is-reserved")]),

    ("version-gates-the-warning", "2026-07-29", "10:00", "joint", 0.9, ["forward-compatibility", "versioning", "warning"],
     "The memory file's semver version discriminates whether an unknown entry is worth mentioning. If the format version is newer than this build supports, stay quiet -- a newer Scatterhoard plausibly wrote it. If the version is one this build fully supports, nothing newer produced it, so someone put it there and it is worth surfacing.",
     "This resolves the strict-versus-lenient tension without a global choice: an older build meeting a newer repository stays silent on its own, while genuine junk in a current-version repository still gets noticed.",
     "trigger-and-loading.md", [("SUPPORTS", "unexpected-entries-ignored-never-fatal")]),

    ("unexpected-file-is-a-tripwire", "2026-07-29", "10:05", "joint", 0.9, ["security", "tripwire", "prompt-file"],
     "An unexpected file in scatterhoard_mem is a weak signal that someone is trying to smuggle in an instruction channel, particularly anything shaped like a prompt file. Scatterhoard will not load it, but another tool might discover it, and it is sitting in a directory that has just declared it contains no instructions.",
     "This is worth noticing independent of the namespace-flexibility argument, and it is the reason to surface to a human rather than log and forget.",
     "trigger-and-loading.md", [("RESULTS_FROM", "namespace-is-reserved"), ("SUPPORTS", "nothing-instruction-shaped-auto-loaded")]),

    ("not-at-session-start", "2026-07-29", "10:10", "joint", 0.9, ["warning", "session-start", "context"],
     "The warning does not go at session start. A session-start warning costs context, trains people to scroll past it, and frequently reaches someone who cannot act on it -- a contributor who just cloned the repository cannot fix what a maintainer committed.",
     "The third reason is the decisive one and the least obvious: the moment with the most attention is not the moment with the most agency, and a warning delivered to someone powerless is worse than none because it spends their trust.",
     "trigger-and-loading.md", [("RESULTS_FROM", "unexpected-file-is-a-tripwire")]),

    ("surface-at-export-and-first-encounter", "2026-07-29", "10:15", "joint", 0.85, ["warning", "export-review", "ledger"],
     "Two better moments carry the warning: the export review gate as the primary home, where the local user is about to add to the repository and is in a position to act, and once on first encountering a given project, so a genuinely suspicious file is not sitting unmentioned until the first export.",
     "The export-review ledger is already local-only per-machine state tracking whether something has been dealt with, so it can carry the first-encounter flag without new machinery.",
     "trigger-and-loading.md", [("RESULTS_FROM", "not-at-session-start")]),

    ("load-conventions-once", "2026-07-29", "10:20", "human", 0.85, ["conventions", "context", "multi-project"],
     "If the user is working on multiple projects the LLM does not reload the conventions, which would waste context, but it does load the second project's memories and keep both in storage, using memory steering to put the correct memories in the correct sets.",
     "Conventions are the same everywhere while memory sets are not, so the two halves of session start have different multiplicities. Treating them as one step would either waste context or lose a set.",
     "trigger-and-loading.md", [("RELATED_TO", "trigger-is-general-prompt-text")]),

    # ---- export-review.md -----------------------------------------------------------------
    ("export-gate-is-the-backstop", "2026-08-03", "09:00", "joint", 0.95, ["export-review", "backstop", "scope"],
     "The export review gate is the backstop half of memory steering: even though scope is chosen explicitly at store time, an export-time gate catches mistakes before memories reach a project's public repository.",
     "A misfiled personal memory is a one-way door once it is committed and pushed, so it is worth a second check at the point of export rather than trusting the store-time choice alone. The asymmetry of the failure is what buys the second check.",
     "export-review.md", [("SUPPORTS", "scope-explicit")]),

    ("gate-runs-at-export-before-commit", "2026-08-03", "09:05", "joint", 0.85, ["export-review", "conventions", "git-diff"],
     "The gate runs as part of the export-before-commit convention, at the point where a git diff of the memory files for this export-and-commit cycle is available.",
     "The convention was once described in a way that read as though it also produced the memories, which it does not; this document depending on the export reading is part of what prompted the description to be split.",
     "export-review.md", [("DEPENDS_ON", "export-gate-is-the-backstop")]),

    ("mechanical-prohibited-data-scan", "2026-08-03", "09:10", "joint", 0.9, ["export-review", "prohibited-data", "scan"],
     "The first of two independent checks is a deterministic scan for the prohibited data categories. It runs on the full exported set every export cycle, and is not scoped by authorship or by the ledger.",
     "Whether a secret leaked in because of something the local user wrote or something merged in from elsewhere does not change the risk of committing it, and the check is cheap enough to run unconditionally.",
     "export-review.md", [("DEPENDS_ON", "prohibited-data-categories")]),

    ("scan-belongs-in-export-not-precommit", "2026-08-06", "09:15", "joint", 0.9, ["export-review", "placement", "scan"],
     "Proposed 2026-08-06: the mechanical scan belongs in export rather than at the pre-commit gate. It reads the whole set, ignores the ledger and ignores authorship, so unlike the judgment gate it never needs the git diff. Everything it requires is in hand while export is serialising the records.",
     "It was grouped with the other two checks by proximity rather than by requirement. Moving it makes the exported response's scanned flag a fact about the export that just ran, rather than the hard-coded false it is today.",
     "export-review.md", [("RESULTS_FROM", "mechanical-prohibited-data-scan"), ("INSTANCE_OF", "decision-placement")]),

    ("scanned-flag-needs-its-note", "2026-08-11", "09:20", "joint", 0.85, ["export-review", "reporting", "conflation"],
     "The export response carries a note beside the scanned flag, because false was true for two unrelated reasons and said which was which for neither: a local export skips the gate by design, having nothing to publish, while a shared export skips it only because no scan exists.",
     "The distinction gets sharper rather than redundant once the scan lands -- a shared export that genuinely went unscanned must not read like a local one that never needed scanning -- so the note is not scaffolding to remove.",
     "export-review.md", [("RESULTS_FROM", "scan-belongs-in-export-not-precommit")]),

    ("scan-stays-in-rust", "2026-08-06", "09:25", "joint", 0.9, ["export-review", "self-contained", "rust"],
     "Keeping the scan in export also keeps it in Rust. Scatterhoard installs as a self-contained binary, and putting an operational check in a script would make a scripting runtime a requirement for ordinary use.",
     "That is a different thing entirely from the project's mutation-testing script, which is a development tool the users of a project never run. The distinction is who has to have the runtime, not whether a script exists.",
     "export-review.md", [("SUPPORTS", "self-contained-no-external-components")]),

    ("hand-edited-file-escapes-the-scan", "2026-08-06", "09:30", "joint", 0.85, ["export-review", "gap", "subcommand"],
     "Moving the scan into export does not cover a hand-edited memories.json committed with no re-export. That is a real path rather than a hypothetical, since renaming a forked set is specified as a hand-edit of the header.",
     "The answer keeps the same constraint: expose the scan as a subcommand of the Scatterhoard binary, so a pre-commit hook is one line of shell invoking something already installed rather than a program in another language.",
     "export-review.md", [("EXCEPTION_TO", "scan-belongs-in-export-not-precommit")]),

    ("judgment-gate-checks-scope-and-content", "2026-08-03", "09:35", "joint", 0.9, ["export-review", "judgment", "scope"],
     "The second check is an LLM-judgment review of whether a memory's chosen scope still looks right and whether its content reads as something that should not be in this set -- for instance, looking personal despite being scoped to a project.",
     "It is the check that needs meaning rather than pattern, which is why it is the LLM's and why it cannot be folded into the mechanical scan.",
     "export-review.md", [("SUPPORTS", "export-gate-is-the-backstop")]),

    ("flagging-conditions-are-tunable-config", "2026-08-03", "09:40", "joint", 0.85, ["export-review", "configuration", "conventions"],
     "The judgment gate is governed by a hard-coded baseline set of flagging conditions, extendable and tunable via the project's local configuration, so a project can adjust when the LLM flags something without waiting on a change to the hard-coded conventions.",
     "Configuration rather than prompt text, deliberately: this is tuning against a bounded schema, which keeps it data rather than an instruction channel.",
     "export-review.md", [("DEPENDS_ON", "config-json-is-data")]),

    ("gate-on-last-modifier", "2026-08-03", "09:45", "joint", 0.9, ["export-review", "authorship", "last-modifier"],
     "The judgment gate is scoped to memories the local user last modified, gating on the memory's last modifier rather than its creator. If the local user created or edited a memory since it was last cleared it is their responsibility and gets reviewed.",
     "A memory merged in from main that the local user never touched has already passed review upstream -- its own author's export gate, plus the maintainer's review at merge -- and is skipped here to avoid re-litigating it.",
     "export-review.md", [("SUPPORTS", "judgment-gate-checks-scope-and-content")]),

    ("export-review-ledger", "2026-08-03", "09:50", "joint", 0.9, ["export-review", "ledger", "process-state"],
     "To avoid re-flagging a memory on every export cycle, a small export-review ledger keyed by memory id records the memory's modified timestamp at the point it was last cleared. Each cycle reviews memories the local user last modified that are new or whose modified timestamp is newer than the recorded clearance.",
     "The ledger is kept separate from the memory's own fields because it is process state -- has this been reviewed for export -- rather than content state, and it can live per set or per export target without growing the memory data model.",
     "export-review.md", [("RESULTS_FROM", "gate-on-last-modifier")]),

    ("ledger-is-local-only", "2026-08-03", "09:55", "joint", 0.9, ["export-review", "ledger", "sync"],
     "The ledger is local-only per-machine state, never committed alongside the memory set. The judgment gate is already scoped to memories the local user last modified, so no other contributor's machine ever needs to consult it and there is no cross-user redundancy to eliminate.",
     "Syncing it would only help one human moving between two machines, and even then saves a one-time re-review per memory -- not worth taking on the same merge-conflict surface multi-set sync already has to solve for the memories themselves, just for bookkeeping.",
     "export-review.md", [("DEPENDS_ON", "export-review-ledger")]),

    ("ledger-location-configurable", "2026-08-03", "10:00", "joint", 0.85, ["export-review", "ledger", "configuration"],
     "The ledger's location should be configurable rather than hard-coded to a fixed path, so a user wanting continuity across machines can point it at a folder already synced by something else they control. Scatterhoard does not need to know it is being synced; it just needs to not assume a fixed local path.",
     "It eases the laptop-and-desktop case without building sync into Scatterhoard, which is the cheapest possible answer to a need that did not justify the merge surface.",
     "export-review.md", [("EXCEPTION_TO", "ledger-is-local-only")]),

    # ---- multi-set-sync.md ----------------------------------------------------------------
    ("multiple-simultaneous-sets", "2026-08-04", "09:01", "human", 0.9, ["multi-set", "sets", "flush"],
     "Scatterhoard tracks multiple sets of memories at once. Switching projects does not require flushing a set, though that should probably be a supported option. Exporting copies the in-database memories; it does not clear them, so the database may hold memories from several projects touched some time ago.",
     "Not flushing is what makes the store usable across a working day that touches more than one project, and it is also what creates the reconciliation problem, since the store is always ahead of some file.",
     "multi-set-sync.md", []),

    ("reconciliation-problem", "2026-08-04", "09:05", "joint", 0.9, ["reconciliation", "import", "merge"],
     "Reading a set of memories when memories for that project already exist raises two classes of question: memories that used to exist but were deliberately forgotten or superseded, and entirely new memories created locally, such as during work on the current branch.",
     "Naming the two classes separately is what keeps the answer from being a single overwrite rule, since one class is about absence meaning something and the other is about presence that the file has never seen.",
     "multi-set-sync.md", [("RESULTS_FROM", "multiple-simultaneous-sets")]),

    ("git-owns-file-against-file", "2026-08-04", "09:10", "joint", 0.95, ["merge", "git", "three-way"],
     "Merging two versions of the memory file is git's job, done against the real ancestor its history provides. The alternative reinvents three-way merge against live database state, which has no ancestor unless it reconstructs one -- at which point it is re-implementing what git already does, against rows instead of file lines.",
     "The two options were not simple versus complex; they differed in whether reconciliation leans on a real common ancestor. The file format had already committed to consistent export ordering specifically to keep diffs small for merging, so the intent was always to let git do this work.",
     "multi-set-sync.md", [("RESULTS_FROM", "reconciliation-problem")]),

    ("git-handles-the-two-hard-cases", "2026-08-04", "09:15", "joint", 0.85, ["merge", "git", "conflict"],
     "At the file layer git's ancestor-aware three-way merge handles both hard cases for free: a deliberately forgotten memory is delete-versus-unmodified, treated as a clean delete, while delete-versus-modified is a genuine conflict; and divergent edits to the same memory are flagged correctly without Scatterhoard reconstructing merge semantics itself.",
     "It is worth stating what git gets right, because the sections that follow are about the half it cannot see, and the boundary between them is the whole design.",
     "multi-set-sync.md", [("SUPPORTS", "git-owns-file-against-file")]),

    ("line-granularity-over-reports-conflicts", "2026-08-04", "09:21", "joint", 0.9, ["merge", "granularity", "json"],
     "With one JSON object per line, two branches editing different fields of the same memory -- one adding a link, the other bumping confidence -- still look to git like two edits to the same line, so plain line-based merging over-reports conflicts even though the changes are logically compatible.",
     "The weakness is a direct consequence of the one-record-per-line choice, so it is the price of a format that diffs well rather than an oversight, and it bounds what the merge driver below has to fix.",
     "multi-set-sync.md", [("EXCEPTION_TO", "git-owns-file-against-file")]),

    ("merge-driver-is-graceful-add-on", "2026-08-04", "09:26", "joint", 0.9, ["merge-driver", "git", "graceful-degradation"],
     "A custom git merge driver intercepts exactly the cases where git would flag a line conflict and attempts a three-way field-level JSON merge of that record, falling back to a real conflict only when the same field genuinely diverged on both sides. It is a graceful enhancement layered on plain git merging, never a dependency the scheme requires.",
     "Registering a driver needs local per-clone git configuration, which can fail to happen or fail to fire silently. The saving property is that an unregistered driver leaves git's standard merge -- correct, just coarser -- rather than silent data loss, and that graceful degradation is exactly why it is safe to build despite the registration risk.",
     "multi-set-sync.md", [("RESULTS_FROM", "line-granularity-over-reports-conflicts")]),

    ("installer-registers-the-driver", "2026-08-04", "09:31", "joint", 0.8, ["merge-driver", "installation", "early-validation"],
     "Scatterhoard's own tooling should register the merge driver automatically on setup rather than documenting a manual step for humans to follow, but given the mixed real-world reliability of git hook-like mechanisms this needs hands-on testing before being relied on.",
     "A manual setup step that silently does nothing when skipped is the failure mode the driver design already accounts for, so automating it removes the most likely cause rather than adding convenience.",
     "multi-set-sync.md", [("DEPENDS_ON", "merge-driver-is-graceful-add-on")]),

    ("dangling-links-after-merge", "2026-08-04", "09:35", "joint", 0.85, ["merge", "dangling-link", "soft-failure"],
     "If a merge forgets a memory that something else still links to, that is just a dangling link. The soft-failure, let-the-LLM-decide stance already established for references across sets applies unchanged within a set, and no new mechanism is needed.",
     "Reusing a stance already argued elsewhere is cheaper than deriving one, and the case is genuinely the same shape -- a link whose target is not there, where refusing to proceed helps nobody.",
     "multi-set-sync.md", [("INSTANCE_OF", "dangling-is-llm-call")]),

    ("two-reconciliations-not-one", "2026-08-04", "10:00", "joint", 0.95, ["reconciliation", "store", "file"],
     "There are two reconciliations. File against file is this checkout's exported memories against the ones arriving from origin, which git owns with a real ancestor. Store against file is the live database against whatever the file says after a pull, and nothing owned it.",
     "The store is a replica git has never seen, and it is the one that has been changing all session. The original phrasing assumed the store held nothing the file did not, and by the time anyone pulls it usually does.",
     "multi-set-sync.md", [("SUPERSEDES", "git-owns-file-against-file")]),

    ("pull-then-import-is-silent-data-loss", "2026-08-04", "10:05", "joint", 0.95, ["data-loss", "ordering", "import"],
     "The old export-pull-merge-import sequence is correct only if followed in that order. Export-then-pull works, because the working tree holds the store's state and git merges origin against it with a real ancestor. Pull-then-import is silent data loss: the working tree still holds the last export, git fast-forwards it, and an import that replaces writes the pulled file over the store. Every memory recorded that session is gone, with no error and nothing left to diff against.",
     "git pull happens in a terminal Scatterhoard never sees, so export first is a rule the tool cannot check at the moment it is broken, and the penalty falls on exactly the work that just happened. That is a data-loss defect waiting for its first real user, not a usability wrinkle to document.",
     "multi-set-sync.md", [("RESULTS_FROM", "two-reconciliations-not-one")]),

    ("order-must-stop-mattering", "2026-08-04", "10:10", "joint", 0.9, ["data-loss", "design-goal", "ordering"],
     "The fix has to be a design in which the order does not matter, rather than documentation telling users which order to use.",
     "A rule whose violation is undetectable and unrecoverable cannot be discharged by instruction. Stating the requirement this way is what rules out the whole family of answers that add a warning.",
     "multi-set-sync.md", [("RESULTS_FROM", "pull-then-import-is-silent-data-loss")]),

    ("tombstones-make-forgetting-explicit", "2026-08-04", "10:15", "joint", 0.95, ["tombstone", "forget", "import"],
     "With tombstones, absence of a record in the file no longer means deletion -- deletion is a line that says so. This is what lets import keep a memory the file has never heard of instead of guessing whether it was forgotten upstream.",
     "The earlier conclusion that git's merge made a separate tombstone mechanism unnecessary is overturned: a deleted line is unambiguous only to a reader holding the ancestor, and the store does not hold it.",
     "multi-set-sync.md", [("RESULTS_FROM", "order-must-stop-mattering")]),

    ("base-checksum-per-record", "2026-08-04", "10:20", "joint", 0.95, ["base", "checksum", "replica"],
     "Each record carries the checksum it had when the store and the file last agreed, called the base. It is local per-replica state, living in the store beside the vectors and the export-review ledger, never in the file, because it records an agreement between two particular replicas and means nothing to a third.",
     "Keeping it out of the file is what makes it correct rather than merely private: a base written into the exchanged artifact would be read by a third replica as a fact about their agreement, which it is not.",
     "multi-set-sync.md", [("RESULTS_FROM", "order-must-stop-mattering")]),

    ("base-is-per-record-and-per-target", "2026-08-04", "10:25", "joint", 0.9, ["base", "target", "checkout"],
     "The base is keyed by record and target, where the target is the export destination. One store serves every checkout of a repository, since the database is keyed on the set name alone -- but a developer may hold the same repository checked out twice on different branches, and one base per record cannot describe both. Importing from the second checkout would read the first checkout's agreement as changes arriving from origin, and every record would show up modified.",
     "The cost is one checksum per record per target, a few hundred kilobytes for a ten-thousand-memory set with two checkouts, so this is bookkeeping rather than scale. The rejected alternative -- one nominated target, other exports untracked -- saves state that is demonstrably cheap and makes the second checkout mis-merge silently.",
     "multi-set-sync.md", [("RESULTS_FROM", "base-checksum-per-record")]),

    ("store-per-checkout-not-considered", "2026-08-04", "10:30", "joint", 0.85, ["base", "store-registry", "premise"],
     "A third alternative was not considered at the time: give each checkout its own store. Then a store has one destination, the target key disappears, and two checkouts are two replicas reconciled by git rather than one store emulating two.",
     "Recorded because the per-target reasoning is sound given one store per set, and it never examined that premise. A conclusion that is valid under an unexamined assumption is worth marking as such, since the assumption is where a later proposal will attack it.",
     "multi-set-sync.md", [("RELATED_TO", "store-per-checkout-proposal")]),

    ("checksum-is-change-detector-not-security", "2026-08-04", "10:35", "joint", 0.95, ["checksum", "threat-model", "security"],
     "The base is a change detector, not a security boundary, and that is what sizes it. A crafted record colliding with the stored base makes reconciliation conclude origin did not change this, and therefore not import the attacker's content -- the collision suppresses their own change. Every other branch needs control of the store's side, which supplying a file does not give.",
     "A cryptographic hash is not merely unnecessary but misleading: it would state a guarantee this value does not provide and invite a later reader to use it as an integrity or provenance check. Tracing what a collision actually buys is what makes the sizing argument rather than assuming stronger is safer.",
     "multi-set-sync.md", [("DEPENDS_ON", "base-checksum-per-record")]),

    ("sixty-four-bits-width-over-algorithm", "2026-08-04", "10:40", "joint", 0.9, ["checksum", "width", "collision"],
     "Width matters more than algorithm. Comparing one record against its own stored base is a direct collision rather than a birthday problem: roughly one in four billion per changed record at 32 bits, where a miss means a memory silently stays stale, which is a correctness bug. Sixty-four bits takes it to about 5 in 10^20.",
     "Performance is not the deciding factor at these sizes -- a fifteen-megabyte set is tens of milliseconds of SHA-256 against one or two of a fast non-cryptographic hash, once per import or export.",
     "multi-set-sync.md", [("SUPPORTS", "checksum-is-change-detector-not-security")]),

    ("checksum-algorithm-pinned-and-tagged", "2026-08-04", "10:45", "joint", 0.9, ["checksum", "stability", "algorithm"],
     "The algorithm must be pinned deliberately and identified in the stored state. Rust's DefaultHasher is documented as unstable across releases, so persisting its output would let a compiler upgrade invalidate every base on every machine. An externally specified algorithm at a fixed version is safer than one written here, because a value defined in our own code is one well-meaning refactor away from changing silently.",
     "Getting it wrong is recoverable, which is why the simple choice is right: the base never reaches the memory file, so a change of algorithm means treating every record as divergent and re-establishing agreement on the next export -- noisy for one cycle, not lossy. The algorithm tag turns that from garbage comparisons into a detected mismatch.",
     "multi-set-sync.md", [("SUPPORTS", "sixty-four-bits-width-over-algorithm")]),

    ("called-a-checksum-deliberately", "2026-08-04", "10:50", "joint", 0.85, ["checksum", "naming", "terminology"],
     "The value is called a checksum throughout, deliberately and consistently, because hash invites the cryptographic reading the sizing argument exists to rule out.",
     "A name is the part of a decision that survives into code written by someone who never read the reasoning, so choosing it against the misreading is cheaper than correcting the misreading later.",
     "multi-set-sync.md", [("RESULTS_FROM", "checksum-is-change-detector-not-security")]),

    ("five-way-reconciliation-decision", "2026-08-04", "10:55", "joint", 0.95, ["reconciliation", "decision-table", "convergence"],
     "With the base, the incoming record's checksum and the store record's checksum, reconciliation is a five-way decision per id: both unchanged is nothing to do; file changed alone takes the file's version; store changed alone keeps local, which exports later; both changed but file equals store is independent convergence, accepted and cleared; both changed and different is a genuine conflict, surfaced.",
     "The convergence row matters more than it looks: without it, two people making the same correction produces a conflict, and conflict stops meaning anything.",
     "multi-set-sync.md", [("RESULTS_FROM", "base-checksum-per-record")]),

    ("checksums-rather-than-timestamps", "2026-08-04", "11:00", "joint", 0.95, ["checksum", "timestamps", "clocks"],
     "An earlier form used modified timestamps to decide which side was newer. Timestamps answer which happened later, which needs clocks that agree across machines and monotonicity nothing can enforce. Checksums answer is this the same record, which is local and verifiable.",
     "Timestamps stay on the memory where they serve LLM reasoning -- a reader can absorb a record claiming to be created in 2026 and modified in 1998 and draw the obvious conclusion, while a merge algorithm acting on the same pair would do something indefensible. The clock is a good input to judgment and a bad input to arithmetic.",
     "multi-set-sync.md", [("SUPERSEDES", "base-checksum-per-record")]),

    ("base-written-in-both-directions", "2026-08-04", "11:05", "joint", 0.9, ["base", "export", "import"],
     "The base is updated on import and on export, because both establish agreement. Updating it only on import would make the next import read every locally exported change as an incoming change from origin, and the whole file would arrive as a conflict.",
     "The asymmetric version is the one someone writes first, because import is where reconciliation visibly happens, and its failure appears one cycle later at the far end of the operation.",
     "multi-set-sync.md", [("DEPENDS_ON", "base-checksum-per-record")]),

    ("dirty-flag-is-a-cache", "2026-08-04", "11:10", "joint", 0.9, ["dirty-flag", "cache", "single-source"],
     "Whether a record changed locally is already answered by comparing it against its base, so the per-record dirty flag is an optimisation that keeps status from rehashing the set. The base is the truth and the flag is derived from it, rebuildable by rehashing.",
     "Stated in that order deliberately: two independent facts that can disagree is the failure this project keeps filing, and a cache with a known source is not that.",
     "multi-set-sync.md", [("DEPENDS_ON", "base-checksum-per-record")]),

    ("no-per-set-dirty-flag", "2026-08-04", "11:15", "joint", 0.85, ["dirty-flag", "set-lifecycle", "save-first"],
     "There is no per-set dirty flag, and the base being per-target is why. A record is dirty relative to a target, so a disjunction would have to say which targets it ranges over -- and the save-first rule stopped asking, writing the safety copy unconditionally rather than consulting a flag it cannot trust. Status reports what the per-target bases say; nothing collapses them.",
     "Both halves of the earlier claim were wrong at once, which is the signature of a derived fact that outlived the thing it was derived from.",
     "multi-set-sync.md", [("SUPERSEDES", "dirty-flag-is-a-cache"), ("RELATED_TO", "save-is-unconditional")]),

    ("absent-record-with-no-tombstone-is-anomaly", "2026-08-04", "11:20", "joint", 0.9, ["tombstone", "anomaly", "surfacing"],
     "A record present in the base, missing from the file, with no tombstone cannot arise from any supported operation under tombstone discipline, so it means a hand-edit, a bad merge, or a file written by a build older than tombstones. It is surfaced, not resolved.",
     "Reading it as forgotten is the silent-loss path this design removes; reading it as keep quietly resurrects things someone may have deliberately removed. Neither guess is available to the software, and a human or the LLM can settle it in a sentence.",
     "multi-set-sync.md", [("DEPENDS_ON", "tombstones-make-forgetting-explicit")]),

    ("anomaly-constrains-tombstone-pruning", "2026-08-04", "11:25", "joint", 0.85, ["tombstone", "pruning", "constraint"],
     "A pruned tombstone is indistinguishable from the missing-record anomaly, which is a good argument for not pruning until there is a concrete reason -- and given that forgetting is expected to be rare next to superseding, there may never be one.",
     "It records a constraint that a future space-saving change would otherwise discover by breaking the anomaly detector, which is the kind of coupling that is invisible from the code being changed.",
     "multi-set-sync.md", [("RESULTS_FROM", "absent-record-with-no-tombstone-is-anomaly")]),

    ("exchange-is-whole-set-both-directions", "2026-08-04", "11:30", "joint", 0.9, ["export", "import", "whole-set"],
     "Export writes the entire set and import reconciles the entire store against the entire file. There is no partial mode, and the base machinery assumes there is none, replacing a target's whole base set rather than merging into it. Two parties exchange memories by each exporting everything and importing everything; nothing needs selecting and the sequence does not matter.",
     "Whole-set exchange is what makes the ordering property hold, so partiality is not a missing feature but a premise the reconciliation rests on.",
     "multi-set-sync.md", [("DEPENDS_ON", "five-way-reconciliation-decision")]),

    ("trimming-an-export-is-not-a-subset", "2026-08-04", "11:35", "joint", 0.9, ["export", "anomaly", "hand-edit"],
     "Trimming records out of an exported file is not a subset export; it is the missing-record anomaly arriving through an edit that feels routine, because editing an export is a procedure the design blesses elsewhere. The difference is that re-attribution changes records in place and removes none. A removed record the target had previously agreed on is reported, named, and written for not at all.",
     "Loud rather than lossy, and the case is pinned by a test rather than left to the argument, because the edit feels routine at exactly the moment it is not.",
     "multi-set-sync.md", [("INSTANCE_OF", "absent-record-with-no-tombstone-is-anomaly"), ("RELATED_TO", "re-attribution-procedure")]),

    ("partial-exchange-needs-less-than-it-looks", "2026-08-04", "11:40", "joint", 0.8, ["partial-export", "deferred", "bases"],
     "Genuine partial exchange is deferred rather than rejected, and it would need less than it looks: a recipient already handles a partial file correctly, because holding no bases for that target means records they lack arrive as new from the file and records the file omits resolve to keep-local. What is missing is a way to say which records travel, and a base rule for a target that agrees with only part of a set.",
     "Knowing how much of a deferred feature already works is what keeps it from being re-argued from scratch, and it locates the actual gap rather than the apparent one.",
     "multi-set-sync.md", [("EXCEPTION_TO", "exchange-is-whole-set-both-directions")]),

    ("what-order-independence-costs", "2026-08-04", "11:45", "joint", 0.9, ["tradeoff", "reconciliation", "cost"],
     "Git's ancestor-aware conflict detection covered the store-against-file case for free in the old design, at the price of an ordering requirement that could not be enforced. Detecting those conflicts ourselves from the base is real code with real edge cases, and it is the principal thing this change buys with effort rather than saving. What it buys back is that the sequence stops mattering.",
     "Naming the cost honestly is what makes the trade defensible: the operation that used to destroy a session's work is now the one that merges it, and that is worth writing a conflict detector for.",
     "multi-set-sync.md", [("RESULTS_FROM", "five-way-reconciliation-decision")]),

    # ---- set-lifecycle.md -----------------------------------------------------------------
    ("no-way-to-create-a-set", "2026-08-05", "09:00", "joint", 0.9, ["set-lifecycle", "create", "gap"],
     "There was no supported way to bring a memory set into existence. The local store is created on first run so it never needed one, and shared stores were assumed to arrive by import -- but import only materialises a database from a file that already exists, so it could only create a set someone else had already created. The first set, the first time, has nothing to import from.",
     "The gap was filled by hand, creating the database file and restarting the server, which worked only because the schema is created if absent. The file-side equivalent was worse: hand-writing a header with a format version the writer is supposed to own puts a number a human guessed at into the one field that decides whether a future build refuses the file.",
     "set-lifecycle.md", []),

    ("create-is-a-distinct-operation", "2026-08-05", "09:05", "joint", 0.95, ["set-lifecycle", "create", "import"],
     "Create is a distinct operation, not a special case or mode of import.",
     "Import reconciles a file against a store; create brings a set into existence with nothing to reconcile against. Folding them together would mean import inventing a store on a typo, which silently splits a set -- and refusing an unknown set name was right for exactly that reason.",
     "set-lifecycle.md", [("RESULTS_FROM", "no-way-to-create-a-set")]),

    ("create-is-also-adoption", "2026-08-05", "09:10", "joint", 0.85, ["set-lifecycle", "create", "adoption"],
     "Create is also how a set someone else exported is adopted. Naming it binds the name on this machine, the file's header can then only agree, and a wrong name is a refusal rather than a silent re-target -- so clone-and-import needs no separate first-import mechanism.",
     "This was not obvious at the time and was once mis-stated as a missing capability, corrected by writing the test. An any-shared expectation would save a human typing a name; it does not enable anything.",
     "set-lifecycle.md", [("RESULTS_FROM", "create-is-a-distinct-operation")]),

    ("create-makes-database-export-makes-file", "2026-08-05", "09:15", "joint", 0.9, ["set-lifecycle", "create", "export"],
     "Given that either the file or the database must come into existence, the database is the right one to create directly. The database is what the server can use; a file with no database is inert until something imports it. Creating the database first makes the file a derived artifact produced by the writer that owns the format version, so nothing hand-writes a header.",
     "It also matches the working sequence -- create the set, put memories in it, export when there is something worth committing. An empty set is a legitimate state.",
     "set-lifecycle.md", [("SUPPORTS", "create-is-a-distinct-operation")]),

    ("create-binds-the-set-name", "2026-08-05", "09:20", "joint", 0.9, ["set-lifecycle", "create", "naming", "trust"],
     "For a shared set, create is where the set name is first bound on this machine. The naming decision cannot come from the file, since a header is untrusted content, nor from the path, since a project's directory name and its set name need not coincide. It comes from whoever runs create, the only participant that knows what the set is for.",
     "It does not bind the name to a checkout, and this was once claimed wrongly -- create takes a name and nothing else, so no directory is in scope when it runs. The claim was doubly misleading because it presented an open question as half-answered by an operation that cannot see a checkout.",
     "set-lifecycle.md", [("INSTANCE_OF", "decision-placement"), ("DEPENDS_ON", "header-scope-checked-never-used")]),

    ("first-export-associates-a-directory", "2026-08-05", "09:25", "joint", 0.85, ["set-lifecycle", "target", "trust-on-first-use"],
     "What actually associates a set with a directory is the first export or import naming that directory, which records bases against it. That is a real trust-on-first-use record, arrived at by accident rather than by design.",
     "A mechanism that exists for one purpose and satisfies another is worth noticing before someone builds the second one from scratch beside it.",
     "set-lifecycle.md", [("RESULTS_FROM", "create-binds-the-set-name")]),

    ("export-and-backup-are-one-operation", "2026-08-05", "09:30", "joint", 0.9, ["set-lifecycle", "export", "backup"],
     "Export and backup are one operation. Local sets are written to files too, in the same format by the same writer, and the header carries a full scope precisely so a backed-up local set is recognisably local on sight. There is no format difference to justify two operations.",
     "The one genuine difference is that backup does not run the prohibited-data scan, and that is not a choice anyone makes -- it follows from the scope, since local is never destined for a repository. A separate backup tool would be a second name for one operation, distinguished by a fact the software already holds.",
     "set-lifecycle.md", [("INSTANCE_OF", "decision-two-checks")]),

    ("export-response-says-gate-was-skipped", "2026-08-05", "09:35", "joint", 0.85, ["set-lifecycle", "export", "reporting"],
     "A caller may still say back up my local memories in words; the tool they reach for is export with local scope, and the response should say the gate was skipped and why, so the difference stays visible where it matters -- in the result, not in the surface.",
     "Collapsing two tool names into one loses nothing only if the distinction reappears in the output. Putting it there rather than in the surface is what keeps the collapse honest.",
     "set-lifecycle.md", [("RESULTS_FROM", "export-and-backup-are-one-operation")]),

    ("import-and-restore-stay-separate", "2026-08-05", "09:40", "joint", 0.95, ["set-lifecycle", "import", "restore"],
     "Import reconciles a file against a store that may have diverged from it, keeping a memory the store holds that the file has never carried. Restore replaces: the file becomes the store and local divergence is discarded on purpose, serving the case where the store is corrupt or being rebuilt and reconciling against it would mean merging against content nobody trusts.",
     "The same decision-placement test that collapsed export and backup gives the opposite answer here, which is worth being explicit about because the surface pressure runs the other way.",
     "set-lifecycle.md", [("DEPENDS_ON", "five-way-reconciliation-decision")]),

    ("merge-names-two-operations", "2026-08-05", "09:45", "joint", 0.9, ["set-lifecycle", "merge", "terminology"],
     "Merge is the word that caused the trouble, because it names two different operations at two different layers. Git merges two files with a common ancestor, which Scatterhoard leans on rather than reinventing. What changed is the assumption underneath -- that by the time a file is imported the store holds nothing the file does not.",
     "The per-record base is not the proposal that was considered and rejected. That one reconstructed an ancestor from live database state; this one records the ancestor at the moment agreement actually happens, which is cheap, exact, and available at a boundary git cannot see.",
     "set-lifecycle.md", [("SUPPORTS", "import-and-restore-stay-separate")]),

    ("upsert-would-prevent-forgetting", "2026-08-05", "09:50", "joint", 0.9, ["set-lifecycle", "import", "tombstone"],
     "An earlier draft described import as an upsert that left store-only memories alone, and located the whole distinction from restore in what happens to store-only memories. That cannot work while absence means deletion: the documented way to forget was to delete a line, so an upsert would have left the export-pull-merge-import cycle incapable of ever forgetting anything.",
     "The earlier version had the mechanism wrong and the conclusion right, which is worth recording because the wrong mechanism was the more attractive one. The implementation had it right and the draft was overturning working code without noticing what the code was protecting.",
     "set-lifecycle.md", [("RESULTS_FROM", "import-and-restore-stay-separate")]),

    ("tombstones-rescue-the-upsert-instinct", "2026-08-05", "09:55", "joint", 0.9, ["set-lifecycle", "tombstone", "import"],
     "Tombstones rescue the upsert instinct by removing its precondition. Once deletion is a line rather than an absence, keeping unknown store memories is safe. The draft was reaching for the right behaviour and missing the mechanism that would license it.",
     "It is the pattern worth recognising: an instinct that is wrong under current mechanics may be right under mechanics that are also being changed, so the two changes have to be evaluated together rather than in sequence.",
     "set-lifecycle.md", [("DEPENDS_ON", "tombstones-make-forgetting-explicit"), ("SUPPORTS", "upsert-would-prevent-forgetting")]),

    ("scope-does-not-determine-preserve-or-discard", "2026-08-05", "10:01", "joint", 0.9, ["set-lifecycle", "scope", "restore"],
     "What separates import from restore is whether local divergence is preserved or discarded, and that is a difference no scope determines. A shared set can need either -- reconcile after a pull, replace after a corrupted database -- and so can a local set. The earlier local-versus-project framing would have made the choice unavailable in exactly the cases that need it.",
     "The caller does have the information, since it knows whether it is recovering or reconciling, so the decision is properly placed there. The only remaining question is whether it arrives as two tool names or one tool with a mode.",
     "set-lifecycle.md", [("INSTANCE_OF", "decision-placement")]),

    ("replace-is-a-name-not-a-flag", "2026-08-05", "10:05", "joint", 0.95, ["set-lifecycle", "destructive", "naming"],
     "An operation that can destroy memories gets its own name; it is never a mode on a constructive operation. A flag makes the destructive reading one token away from the safe one -- a mode argument is chosen once, cheaply, in the middle of a longer argument list, by a caller whose attention is on the path, while a tool name has to be selected deliberately and is legible in a transcript afterward.",
     "This covers restore and drop and nothing else in the lifecycle. Naming is a speed bump rather than a guarantee, so it does not stand alone.",
     "set-lifecycle.md", [("RESULTS_FROM", "scope-does-not-determine-preserve-or-discard")]),

    ("destructive-operation-saves-first", "2026-08-05", "10:10", "joint", 0.95, ["set-lifecycle", "destructive", "save-first"],
     "A destructive operation takes a path and writes the set there before destroying it. Not a refusal, a save. Drop and restore share one rule, and the guarantee sharpens from cannot discard unexported memories to cannot discard memories without first writing them somewhere the caller named.",
     "Refusal is worst exactly when the operation matters most, since restore is reached for when something is broken and a human is under pressure. The safety copy is also more useful than a clean flag: the exported format diffs well, so a pre-restore copy can be compared against the restored set and individual memories cherry-picked back.",
     "set-lifecycle.md", [("SUPPORTS", "replace-is-a-name-not-a-flag")]),

    ("safety-copy-path-is-not-the-export-path", "2026-08-05", "10:15", "joint", 0.9, ["set-lifecycle", "restore", "correctness"],
     "The safety copy's path must not be the normal export path. Restore reads from a file, and a safety copy written to the set's usual export path could overwrite the very file being restored from, destroying the good copy in the act of protecting the bad one. So the destination is stated by the caller and is a separate location.",
     "This is a correctness requirement rather than a convenience, and it is the reason the path is a parameter rather than derived.",
     "set-lifecycle.md", [("DEPENDS_ON", "destructive-operation-saves-first")]),

    ("two-failures-of-the-safety-copy", "2026-08-05", "10:20", "joint", 0.9, ["set-lifecycle", "failure-modes", "restore"],
     "The two ways a safety copy can fail are not the same. If the store cannot be read -- the corruption case -- the memories are already beyond saving, refusing protects nothing and blocks the repair, so proceed and report unmissably that no copy was made. If the destination cannot be written, the store is fine and the memories are recoverable, so refuse and let the caller fix the destination.",
     "Worth stating because the natural implementation -- if the export failed, stop -- gets the first case exactly backwards.",
     "set-lifecycle.md", [("SUPPORTS", "destructive-operation-saves-first")]),

    ("save-is-unconditional", "2026-08-05", "10:25", "joint", 0.95, ["set-lifecycle", "save-first", "dirty-flag"],
     "Decided 2026-08-06: the save is unconditional and no flag gates it. The fact the guard needs is whether any memory's current content exists in no file, and the base cannot answer it -- the base records an agreement between two particular replicas, which is a claim about a past moment rather than about what is on disk now.",
     "Two collapses were considered and both over-read it in opposite directions. Dirty against any target reads not yet agreed with every replica as not saved, so a two-checkout set is dirty whenever either is behind and the gate stops discriminating. Dirty against every target is closer but claims what it cannot support, since a branch switch changes the file on disk with the base untouched. Neither is a tuning problem, so the gate is removed rather than tuned.",
     "set-lifecycle.md", [("SUPERSEDES", "destructive-operation-saves-first"), ("DEPENDS_ON", "base-is-per-record-and-per-target")]),

    ("guard-inside-the-artifact-it-protects", "2026-08-05", "10:30", "joint", 0.95, ["set-lifecycle", "guard", "general-shape"],
     "A guard stored inside the artifact it protects is unavailable exactly when it is needed. Removing the destructive rules' reads of the database they are about to destroy is strictly better than getting the fallback right, and the previous formulation had met the problem with a fallback in which an unreadable flag counted as dirty.",
     "A general shape worth remembering past this document, and the argument that removes the flag from the destructive path entirely.",
     "set-lifecycle.md", [("RESULTS_FROM", "save-is-unconditional")]),

    ("forcing-a-drop-is-outside-the-tool", "2026-08-05", "10:35", "joint", 0.9, ["set-lifecycle", "drop", "escape-hatch"],
     "Requiring the path unconditionally means drop cannot run when nothing is writable. The way through is outside the tool on purpose: delete the database by hand and restart the server. An unconditional-destruction option inside the tool surface is one argument away from any caller, including a model working quickly, while a manual file deletion and a server restart cannot happen without a human doing it deliberately.",
     "This is the guarantee the replace-is-a-name argument was reaching for and could not enforce by naming alone. It is also not the hand-editing this document rejected: hand-creating a store makes a human invent a format version, while deleting invents nothing.",
     "set-lifecycle.md", [("SUPPORTS", "replace-is-a-name-not-a-flag"), ("DEPENDS_ON", "save-is-unconditional")]),

    ("restore-is-one-operation-not-a-sequence", "2026-08-05", "10:40", "joint", 0.9, ["set-lifecycle", "restore", "atomicity"],
     "Restore is equivalent to drop plus create plus import, and is still exposed as a single operation, with the decomposition not offered as the way to do it. A hand-composed sequence that fails partway leaves no set at all -- dropped and not re-created, or created and not populated -- which is strictly worse than either endpoint and arrives without warning.",
     "Composing three destructive steps by hand is a thing to be doing when nothing has gone wrong, and by construction nobody is doing it then. Everything about the operation should assume the caller has less context than usual, not more, because the store holding their memories is the thing that broke.",
     "set-lifecycle.md", [("SUPPORTS", "import-and-restore-stay-separate")]),

    ("dirty-state-is-a-flag-not-a-timestamp", "2026-08-05", "10:45", "joint", 0.85, ["set-lifecycle", "dirty-flag", "state-shape"],
     "The dirty state is a flag in the meta table rather than a timestamp. An mtime acquires meaning from whatever else reads it, and this has to survive clock skew, a database copied between machines, and a file restored from backup. A flag lives inside the database and travels with it.",
     "The reasoning is recorded in the coding-lessons set as a technique carrying its old context's assumptions with it, which is what makes the timestamp attractive and wrong.",
     "set-lifecycle.md", [("RELATED_TO", "checksums-rather-than-timestamps")]),

    ("not-a-counter-either", "2026-08-05", "10:50", "joint", 0.9, ["set-lifecycle", "dirty-flag", "decision-placement"],
     "Not a counter either, which an earlier draft argued for on exactly the anti-timestamp reasoning -- wrongly, since everything in that argument is against a timestamp and a flag satisfies all of it. A counter answers a strictly larger question, how far apart are two things, which the drop rule never asks.",
     "A tidy example of the decision-placement test failing in the direction people rarely check: the extra state was not a wrong answer, it was an answer to a question nobody had asked.",
     "set-lifecycle.md", [("INSTANCE_OF", "consequenceless-choice"), ("SUPERSEDES", "dirty-state-is-a-flag-not-a-timestamp")]),

    ("version-comparison-reads-what-it-may-replace", "2026-08-05", "10:55", "joint", 0.9, ["set-lifecycle", "restore", "guard"],
     "A counter was the candidate for gating restore by comparing a file's version against the store's. It is the wrong instrument for the case that matters: a version comparison reads the store to decide whether the store may be replaced, which is unavailable precisely when the store is corrupt.",
     "That sentence generalised further than it was written to -- applied to the dirty flag itself it removes the flag from the destructive path entirely. The counter argument was right about a class and was only ever applied to one member of it.",
     "set-lifecycle.md", [("SUPPORTS", "guard-inside-the-artifact-it-protects")]),

    ("status-and-dirty-tracking-are-one-feature", "2026-08-05", "11:00", "joint", 0.85, ["set-lifecycle", "status", "tooling"],
     "Which sets are loaded, and which have unexported changes, are the two halves of one question, which is what makes status and dirty-tracking one feature rather than two. It also closes a defect with no other fix: a caller told to check the set name has nothing to check against.",
     "Recognising two requirements as one question is what keeps the tool surface small, and the surface staying small is a standing constraint rather than an aesthetic.",
     "set-lifecycle.md", [("RESULTS_FROM", "not-a-counter-either")]),

    ("flag-no-longer-collapses-to-one-boolean", "2026-08-05", "11:05", "joint", 0.9, ["set-lifecycle", "status", "per-target"],
     "The flag no longer collapses to a single boolean for the set. Bases are keyed by record and target, so a set can be settled against the development checkout and diverged against the review one, and is this set dirty has no answer until someone says dirty against what. That question used to block drop and no longer does.",
     "Its only remaining consumer is status, and there the per-target structure is not an ambiguity to resolve away but the substance of the answer, since which target is behind is exactly what a human with two checkouts wants to know.",
     "set-lifecycle.md", [("RESULTS_FROM", "save-is-unconditional"), ("DEPENDS_ON", "no-per-set-dirty-flag")]),

    ("target-list-is-an-unintended-trust-record", "2026-08-05", "11:10", "joint", 0.9, ["set-lifecycle", "trust", "bases", "target"],
     "Selecting the distinct targets from the bases table yields a list of every directory this set has ever reached agreement with, which is materially what the open question on trusted set names describes wanting. The record exists; it was built to supply merge ancestry and acquired the second meaning for free.",
     "Observed while working out whether status could enumerate targets. Nothing here is decided; it is recorded because two questions carried separately turn out to be one table read in two directions.",
     "set-lifecycle.md", [("RESULTS_FROM", "first-export-associates-a-directory")]),

    ("target-record-is-read-one-direction-only", "2026-08-05", "11:15", "joint", 0.85, ["set-lifecycle", "trust", "enforcement"],
     "The bases table answers given this set, which directories. Trust needs the reverse -- given this directory, which set may claim it -- and answering that means scanning every loaded store's targets. Nothing does, so nothing enforces one set per repository: two sets can both record the same directory as a target and no operation notices.",
     "It is also checked in only one place, since export remarks when the caller names an unrecognised directory and import does not, so import reconciles with no ancestor and says nothing.",
     "set-lifecycle.md", [("EXCEPTION_TO", "target-list-is-an-unintended-trust-record")]),

    ("remote-for-binding-path-for-target", "2026-08-05", "11:20", "joint", 0.9, ["set-lifecycle", "trust", "keying"],
     "The remote-versus-path question is two questions. A target must distinguish two checkouts of one repository, which is the entire reason the base is per-target, so keying it on the remote would reintroduce the defect the per-target base was built to fix -- the key has to be per-working-tree. A set-name binding wants the opposite, since keying it on the remote makes a second checkout inherit the first's binding. So the answer is plausibly remote for the binding, path for the target.",
     "Treating them as one question is what made it hard, and the two bindings genuinely want different keys rather than one key serving both.",
     "set-lifecycle.md", [("RESULTS_FROM", "target-record-is-read-one-direction-only")]),

    ("no-key-removes-the-warning", "2026-08-05", "11:25", "joint", 0.9, ["set-lifecycle", "trust", "warning"],
     "No key removes the need for the warning. Canonicalising a path collapses spellings but does not survive the checkout being renamed or moved, which orphans every base just as thoroughly and reads as a target never seen before. Since no key eliminates the unrecognised-target case, the load-bearing fix is that an unrecognised target is always said out loud, and normalisation is hygiene that reduces how often it fires.",
     "Separating the load-bearing fix from the hygiene is what stops normalisation being mistaken for the solution and the warning being dropped once paths are tidy.",
     "set-lifecycle.md", [("RESULTS_FROM", "remote-for-binding-path-for-target")]),

    ("identity-travels-location-does-not", "2026-08-05", "11:30", "joint", 0.95, ["set-lifecycle", "identity", "path", "naming"],
     "Export takes the path and create takes the name. A set's name is shared identity: it travels between machines in the file header and every copy agrees on it. A path is per-checkout state no other machine can see or use. Binding them at create would write machine-local state into the set's identity, and the copies would immediately disagree.",
     "The immediate reasons are practical -- an absolute path is wrong on the next workstation, and a repository-relative path buys uniformity at the cost of ever exporting anywhere else, including the safety copy, which by construction must go somewhere unusual. The split follows the lifetime of the information.",
     "set-lifecycle.md", [("SUPPORTS", "create-binds-the-set-name")]),

    ("set-name-is-not-the-file-name", "2026-08-05", "11:35", "joint", 0.9, ["set-lifecycle", "naming", "trust", "path"],
     "The database file encodes the set name, but that is a local implementation detail nobody else sees. The exported file has a fixed well-known name that does not encode the set name and must not start to, because the path determining how contents are trusted is only possible when the path is known in advance. A filename derived from the set name would mean the path could no longer be stated without first knowing what the file says about itself.",
     "That is precisely the privilege-escalation channel the trust rule exists to close, arriving through a naming convenience rather than through a header claim.",
     "set-lifecycle.md", [("DEPENDS_ON", "memories-file-well-known-name"), ("SUPPORTS", "header-scope-checked-never-used")]),

    ("renaming-produces-a-real-conflict", "2026-08-05", "11:40", "joint", 0.85, ["set-lifecycle", "rename", "merge"],
     "The set name lives in the header and nowhere else that anyone shares, so renaming a set is a one-line change to the header. Because both branches of a merge carry an identical header, a rename produces a real conflict rather than a silent divergence, which is the correct outcome for a change both sides must agree on.",
     "That is the same reasoning already applied to a version bump, so renaming inherits the property for free rather than needing its own mechanism.",
     "set-lifecycle.md", [("RESULTS_FROM", "set-name-is-not-the-file-name")]),

    ("what-a-fork-does-to-the-name", "2026-08-05", "11:45", "joint", 0.9, ["set-lifecycle", "fork", "identity"],
     "A fork copies the repository header included, so the fork's memories continue to claim the original set name. Whether that is right depends on something no software can see: a contributor that will merge back should keep the name, while a divergence that will not should rename, because two histories claiming one identity will eventually be imported onto one machine and the merge will produce a set that never existed.",
     "The collision is what needs a human, not the naming. A set name never seen before can be bound silently because nothing existing can be contaminated; a name already bound to a different checkout is the case to refuse and surface, and a fork is the ordinary way that happens rather than an exotic one.",
     "set-lifecycle.md", [("INSTANCE_OF", "identity-travels-location-does-not")]),

    ("renaming-a-fork-is-an-acceptable-hand-edit", "2026-08-05", "11:50", "joint", 0.9, ["set-lifecycle", "fork", "hand-edit"],
     "A hard fork renames by editing the scope field in the file's header line and committing that; no tool is proposed for it. This is a deliberate exception to the argument that made create a real operation, and the cases differ in what is being written: a rename edits one field of an existing valid header, leaving the version the writer produced untouched. Nothing is being invented.",
     "What makes it safe rather than merely small is that the mistake is caught -- reading a memory file requires the caller to state what it believes the file to be, so an edited header imported into a store bound to the old name is a refusal rather than a silent merge of two projects' memories. The rule written for privilege escalation turns out to make hand-editing recoverable.",
     "set-lifecycle.md", [("EXCEPTION_TO", "no-way-to-create-a-set"), ("DEPENDS_ON", "what-a-fork-does-to-the-name")]),

    ("forker-needs-drop-then-import", "2026-08-05", "11:55", "joint", 0.85, ["set-lifecycle", "fork", "drop"],
     "The forker's own machine needs drop then import, because the local database records the old name in its metadata and editing the file does not reach it. That is the drop-plus-import path working as intended, and the save-first rule means the old set is written out before it goes.",
     "A consequence not obvious from the edit itself, and the kind that turns a one-line change into a procedure.",
     "set-lifecycle.md", [("RESULTS_FROM", "renaming-a-fork-is-an-acceptable-hand-edit")]),

    ("renaming-dangles-cross-set-references", "2026-08-05", "12:00", "joint", 0.85, ["set-lifecycle", "fork", "cross-set"],
     "Renaming dangles every cross-set reference pointing at the old name, since a cross-set reference is an id plus a set name, so links from other sets into this one stop resolving. For a hard fork that is arguably correct -- it really is a different set now -- and dangling cross-set references are already the LLM's call rather than a hard failure.",
     "Worth knowing before renaming a set that is not a fork, which is the case where the same mechanics produce a wrong result rather than a right one.",
     "set-lifecycle.md", [("RESULTS_FROM", "renaming-a-fork-is-an-acceptable-hand-edit"), ("REFERS_TO", "dangling-is-llm-call")]),

    ("scope-kinds-are-private-and-shareable", "2026-08-05", "12:05", "joint", 0.9, ["set-lifecycle", "scope", "cross-project"],
     "The scope kinds are not really local and project; they are private-and-unbound and shareable-and-named. Shared means a set with a name that can be exported -- it does not require a repository, and nothing in the lifecycle assumes one.",
     "So a cross-project set such as coding-lessons needs no new mechanism: created by create, exported wherever the caller says, loaded alongside everything else. Recall already unions every loaded store and every result names the set it came from.",
     "set-lifecycle.md", [("SUPERSEDES", "scope-renamed-from-project")]),

    ("import-the-set-not-the-project", "2026-08-05", "12:10", "joint", 0.9, ["set-lifecycle", "cross-project", "vendoring"],
     "Another project gets a cross-project set's memories because the developer imports the set, not the project. A set loaded on a machine participates in recall for every session on that machine, so nothing is copied into the other project's repository -- the lessons live in one place and are read by many projects rather than being vendored into each and drifting.",
     "Copying memories between sets is a different operation, deliberately not designed here. It would need an answer for what happens when the source set later changes, which is the problem vendoring always has.",
     "set-lifecycle.md", [("RESULTS_FROM", "scope-kinds-are-private-and-shareable")]),

    # ---- store-registry.md ----------------------------------------------------------------
    ("many-checkouts-one-store", "2026-08-06", "09:00", "joint", 0.95, ["store-registry", "checkout", "cause"],
     "Today a store is keyed on the set name alone, so every checkout of a repository shares one database. Three separate defects all descend from that one mapping, each found while trying to fix the one before it: the reconciliation base had to grow a target key, a target is a path string and paths are not identities, and nothing binds a set name to a repository.",
     "Each has a local fix and none of the local fixes touches the cause, which is that one store is trying to be several replicas at once. Git already knows how to be several replicas.",
     "store-registry.md", [("RESULTS_FROM", "base-is-per-record-and-per-target")]),

    ("store-per-checkout-proposal", "2026-08-06", "09:05", "joint", 0.9, ["store-registry", "registry", "proposal"],
     "Proposed: a scope resolves to exactly one store at a time and a store belongs to exactly one checkout. Stores live centrally named by a UUID assigned at creation, each records its own checkout path, and a machine-local registry records which store is live for each scope. Re-pointing a scope switches which store is live rather than redirecting one store's exports.",
     "What this buys is that a store has at most one export destination, so the base collapses from record-and-target back to record alone -- no target key, no path spelling to normalise, no unrecognised-target case. Two checkouts stop being one store pretending to be two replicas and become two replicas.",
     "store-registry.md", [("RESULTS_FROM", "many-checkouts-one-store")]),

    ("cost-memories-do-not-follow-checkouts", "2026-08-06", "09:10", "joint", 0.9, ["store-registry", "cost", "tradeoff"],
     "The cost, stated plainly because it is the thing to weigh: memories recorded while pointed at one checkout are not recallable while pointed at another. They travel by export, commit, pull and import -- exactly as the code does.",
     "Anyone who expects a memory set to follow them between checkouts will be surprised once, and the answer is the same answer git gives, which is what makes the surprise teachable rather than arbitrary.",
     "store-registry.md", [("DEPENDS_ON", "store-per-checkout-proposal")]),

    ("stores-stay-central-not-in-tree", "2026-08-06", "09:16", "joint", 0.95, ["store-registry", "in-tree", "save-first"],
     "Putting the database inside the reserved directory is genuinely attractive, since it makes the store self-locating so a moved checkout carries its store with it and no registry is needed. Two things rule it out: an in-tree store dies with an ordinary recursive delete, taking unexported memories with it and bypassing the save-first rule entirely, and an in-tree store can be committed, shipping unexported memories past the export review gate.",
     "People delete checkouts casually and drop is rare, so the unguarded path would be the common one. An ignore rule handles the commit case and propagates with the repository, but the review gate holding as long as an ignore rule does is a weaker position than the database not being in the tree.",
     "store-registry.md", [("DEPENDS_ON", "destructive-operation-saves-first"), ("SUPPORTS", "store-per-checkout-proposal")]),

    ("uuid-name-path-in-meta", "2026-08-06", "09:20", "joint", 0.9, ["store-registry", "identity", "path"],
     "A store's identity and its location must be separable, because the location changes and the identity must not. Identity is a UUID fixed at creation and never rewritten, before any checkout is involved, which is what lets an unattached set exist at all. Location is a path recorded in the store's metadata -- a label to be corrected rather than a key to be matched, so a moved checkout is one row to update rather than an orphaned store.",
     "The alternative was encoding the path in the filename, which is legible with no tooling but must be renamed when a checkout moves or it states something false. A filename that confidently asserts the wrong path is worse than an opaque one, because nothing prompts a reader to doubt it.",
     "store-registry.md", [("SUPPORTS", "store-per-checkout-proposal"), ("INSTANCE_OF", "identity-travels-location-does-not")]),

    ("registry-holds-one-fact", "2026-08-06", "09:26", "joint", 0.95, ["store-registry", "registry", "derived-state"],
     "The registry holds one fact: which store is live for each scope. That is the only thing no store can answer about itself. Everything else about the population of stores -- which scope each claims, which checkout each belongs to, how many memories each holds -- is derived by scanning the store directory and reading each store's metadata.",
     "Storing that separately would create a second copy that can disagree with the first, which is the defect class this project keeps filing.",
     "store-registry.md", [("DEPENDS_ON", "store-per-checkout-proposal")]),

    ("path-cache-needs-a-rebuilder", "2026-08-06", "09:31", "joint", 0.85, ["store-registry", "cache", "status"],
     "The path may be cached in the registry on one condition. The lookup made most often is path to store, because the LLM's working directory is what it has, and a pure scan makes the common case the expensive one. The condition is that something actually rebuilds it, and status does, on every call.",
     "Caching is defensible on exactly the terms already accepted for the dirty flag -- rebuildable at any time, not a second fact that can drift. A rebuild that is documented but never run is not a rebuild.",
     "store-registry.md", [("INSTANCE_OF", "dirty-flag-is-a-cache")]),

    ("registry-per-home", "2026-08-06", "09:35", "joint", 0.85, ["store-registry", "concurrency", "home"],
     "A single machine-global registry is shared mutable state between server processes. A server is registered per checkout, so two checkouts open at once means two processes, and one answer to which store is current for a scope is something they will fight over -- the review session re-points and the development session silently changes sets underneath itself. The fix is to put the registry in the home and give each server its own home.",
     "It uses machinery that already exists, since the home variable already locates everything a server owns. Two homes, two registries, nothing shared, no negotiation.",
     "store-registry.md", [("RESULTS_FROM", "registry-holds-one-fact")]),

    ("switchboard-is-gone", "2026-08-06", "09:40", "joint", 0.9, ["store-registry", "registry", "superseded"],
     "With re-pointing settled at startup rather than at runtime, the registry's one fact becomes derivable from where a store's file sits, and the runtime switchboard two processes could fight over is gone -- and with it the question this document called its largest gap. Two of the registry's three motivations survive, and neither is served by placement: the per-target base, and binding a set name to a checkout.",
     "Recording which motivations survive a supersession is what stops the whole proposal being discarded with the part that was replaced.",
     "store-registry.md", [("SUPERSEDES", "registry-per-home")]),

    ("checkout-bound-versus-machine-bound", "2026-08-10", "10:00", "joint", 0.95, ["store-registry", "common", "scope"],
     "Decided and built 2026-08-10: a common directory holds stores belonging to every checkout on this machine, defaulting to the home. You separate a store when it is coupled to something that differs between checkouts. A project set is -- it has a working tree and a branch. A cross-project set is not: a lesson learned while reviewing is just as true while developing tomorrow.",
     "Set neither and nothing changes, so the simple case degenerates to today's behaviour exactly, which is the property that makes it safe to introduce -- someone who never sets it cannot be affected by it.",
     "store-registry.md", [("RESULTS_FROM", "many-checkouts-one-store")]),

    ("local-type-carries-the-answer", "2026-08-10", "10:05", "joint", 0.9, ["store-registry", "scope", "types"],
     "The first version of this decision reached only the local set and looked complete, because the local scope's type carries the answer -- the software can tell it is machine-bound without being told. A shared set's type carries nothing, since a cross-project set and a project set are both shared and only intent separates them. That is the whole gap, and it is why placement has to be said for a shared set and not for local.",
     "A rule that the type system enforces for one case and not another will look uniform from the side where the type does the work, which is where it was written.",
     "store-registry.md", [("RESULTS_FROM", "checkout-bound-versus-machine-bound")]),

    ("where-the-file-sits-is-how-it-is-said", "2026-08-10", "10:10", "joint", 0.9, ["store-registry", "layout", "discovery"],
     "Where the file sits is how placement is said: a store in the common directory is machine-bound, and a store in the home belongs to that home alone. One discovery type scans both and resolves each scope to one file.",
     "That is one type with tests against it rather than lines of startup wiring -- the local store being opened, the directories being scanned, a created set being written, each choosing a directory separately, all having to agree, and nothing noticing if they stopped.",
     "store-registry.md", [("SUPPORTS", "checkout-bound-versus-machine-bound")]),

    ("registry-with-its-fact-derived", "2026-08-10", "10:15", "joint", 0.9, ["store-registry", "registry", "derivation"],
     "This is not a cheaper substitute for the registry; it is the registry with its single fact derived rather than stored, which is the move already made for every other fact considered. A recorded registry earns its keep only when two stores can claim one scope in the same place and something must choose between them, which is the runtime switchboard. Removing the switchboard removes the reason to write the fact down.",
     "Framing it as derivation rather than replacement is what keeps the registry's reasoning available if the switchboard ever comes back.",
     "store-registry.md", [("SUPPORTS", "switchboard-is-gone")]),

    ("restart-is-acceptable", "2026-08-10", "10:20", "human", 0.85, ["store-registry", "restart", "assumption"],
     "The human partner settled the assumption this rests on: a restart is acceptable for changing which sets a server reaches. Power users who need a layout at all will tweak it rarely, and on one client a server can be reconnected without starting a new session, while on another it is an application restart.",
     "The whole startup-rather-than-runtime resolution depends on this being acceptable, so it is worth recording as a decision made rather than a convenience assumed.",
     "store-registry.md", [("DEPENDS_ON", "registry-with-its-fact-derived")]),

    ("create-writes-to-the-home", "2026-08-10", "10:25", "joint", 0.9, ["store-registry", "create", "machine-bound"],
     "Create always writes to the home, because nothing at creation time knows whether a set will turn out to belong to every checkout. That is a fact about how a set gets used, learned later -- often much later, since the pre-publication life of a cross-project set can run for months. So becoming machine-bound is a move, and for now a manual one.",
     "The procedure is short because of a property worth recording: the store runs in write-ahead mode and the sidecars are checkpointed and removed when the last connection closes cleanly, so with the servers stopped there is exactly one file to move and moving it carries every memory.",
     "store-registry.md", [("RESULTS_FROM", "where-the-file-sits-is-how-it-is-said")]),

    ("llm-cannot-restart-its-own-servers", "2026-08-10", "10:30", "joint", 0.85, ["store-registry", "tooling", "human-step"],
     "That the servers must be stopped is not a limitation to design around, because an LLM cannot start or stop its own MCP servers, so this step is a human's either way. What it does argue is that if moving turns out to be routine, the case for a tool action is that it could do the move without a restart, not that it would save typing.",
     "Distinguishing a limitation from a fixed constraint is what keeps effort off the wrong problem, and it changes what a future tool would have to justify.",
     "store-registry.md", [("SUPPORTS", "create-writes-to-the-home")]),

    ("misconfigured-common-reads-as-empty", "2026-08-10", "10:35", "joint", 0.9, ["store-registry", "misconfiguration", "reporting"],
     "Point the common directory at a place with no store and the server does exactly the right thing: it creates one. Every symptom then says zero memories, nothing wrong, while a store full of memories sits unread in the old directory. The memories are never at risk, but reaching them again requires knowing what happened.",
     "A failure whose every symptom is reassuring is the one that needs a fact reported rather than a check performed, which is what status reporting the file each set is read from provides.",
     "store-registry.md", [("RESULTS_FROM", "where-the-file-sits-is-how-it-is-said")]),

    ("common-copy-wins-home-copy-shadowed", "2026-08-10", "10:40", "joint", 0.85, ["store-registry", "shadowing", "status"],
     "A set present in both directories resolves to the common copy, with the home copy named in status as shadowed. The direction is deliberate and is the opposite of the first proposal: every way of reaching this state leaves the home copy as the wrong one, so letting it win would put an empty store in front of real memories. Refusing to load either would strand the session, so one is chosen and the other is named.",
     "Choosing the direction by enumerating how the state is actually reached is what corrected the first proposal, which had picked the intuitive winner rather than the one the causes imply.",
     "store-registry.md", [("SUPPORTS", "where-the-file-sits-is-how-it-is-said")]),

    ("unset-distinguishable-from-set-to-default", "2026-08-10", "10:45", "joint", 0.85, ["store-registry", "configuration", "reporting"],
     "The code keeps unset distinguishable from set to the home, because the ordinary case must say nothing -- nobody asked for anything. A setting that looks applied while having no effect is the same shape as an author variable that is silently ignored, so it is reported. Conflating the two also made every set shadow itself in the default configuration.",
     "A distinction introduced for reporting turned out to fix a bug, which is a sign the distinction was real rather than cosmetic.",
     "store-registry.md", [("SUPPORTS", "common-copy-wins-home-copy-shadowed")]),

    ("status-reports-the-file-as-a-fact", "2026-08-10", "10:50", "joint", 0.9, ["store-registry", "status", "reporting"],
     "Status reports the file each set is being read from, which is what distinguishes you have no memories from you are looking in the wrong place. It is a fact rather than a check: no heuristic decides whether the configuration looks wrong, and nothing can be wrong about it in the reassuring direction. The path comes from SQLite and is fully resolved, so two homes reaching one store through a symlink report one file rather than two.",
     "Naming where the data is rather than the route configured to reach it is the more useful of the two answers, and it is the one that survives indirection.",
     "store-registry.md", [("RESULTS_FROM", "misconfigured-common-reads-as-empty")]),

    ("concurrency-is-a-requirement", "2026-08-13", "11:00", "joint", 0.95, ["concurrency", "wal", "locking"],
     "Sharing one local store between servers makes multi-process access deliberate, so it has to be designed rather than survived. A store file is left in write-ahead mode so a writer in one server does not lock out readers in another, and a write that cannot take the lock waits rather than failing. Both are properties a shared store needs and a private one never exercises, so neither is observable from a single-process test.",
     "Making a previously accidental property into a requirement is what moves it from luck to design, and it is why the checks live in dedicated concurrency tests with recorded mutations beside them.",
     "store-registry.md", [("RESULTS_FROM", "checkout-bound-versus-machine-bound")]),

    ("cache-coherence-is-not-locking", "2026-08-13", "11:05", "joint", 0.95, ["concurrency", "cache", "coherence"],
     "Cache coherence is a separate problem that locking does not fix. The store holds its vector set in memory by design, so that opening a store does not put the embedder on the critical path -- but a cache is only safe while this connection is the only writer, and with the local store deliberately shared it is not. The data-version pragma is the detector: it changes when another connection has committed and never for this one's own writes.",
     "The discipline that keeps this from decaying is on the store type itself: every read of cached state takes a mutable borrow, so a new reader that forgets to refresh does not compile.",
     "store-registry.md", [("SUPPORTS", "concurrency-is-a-requirement")]),

    ("in-process-safety-is-accidental", "2026-08-13", "11:10", "joint", 0.9, ["concurrency", "open-question", "sub-agents"],
     "Within one process, safety is currently accidental. The request loop is strictly serial, which is a property of the stdio transport rather than a stated guarantee, and making the loop concurrent later would remove it silently. Sub-agents are ordinary in the clients this targets, and an isolated worktree agent may well have its own registration and therefore its own process. This one is still open.",
     "An unstated property that something else depends on is the kind that disappears in a refactor nobody connects to the failure, which is why it is named here rather than left implicit.",
     "store-registry.md", [("EXCEPTION_TO", "concurrency-is-a-requirement")]),

    ("sharing-exposed-defects-did-not-cause-them", "2026-08-13", "11:15", "joint", 0.9, ["concurrency", "defects", "method"],
     "The locking and cache defects were filed against the current architecture, because two checkouts already share one home by default, and both were fixed ahead of the variable rather than as part of it. The same held for attribution, found the same way and fixed the same week -- three defects that the possibility of sharing exposed and that sharing itself did not cause.",
     "Distinguishing exposure from causation is what keeps a proposal from being blamed for defects it merely made visible, and it is why the three could be promoted ahead of the proposal they were found alongside.",
     "store-registry.md", [("RESULTS_FROM", "concurrency-is-a-requirement")]),

    ("reason-from-the-library-not-the-standard", "2026-08-13", "11:20", "joint", 0.9, ["method", "defaults", "correction"],
     "One correction worth keeping, about method rather than about SQLite. The locking defect was filed from reading the store's one pragma line and reasoning about SQLite's documented defaults, which are correct -- its default busy timeout really is zero. The waiting behaviour was already there anyway, because the driver sets five seconds of its own.",
     "Half of that entry described a database this project does not have: one built from the standard's defaults rather than from the library actually in use. The reasoning was sound and applied to the wrong artifact.",
     "store-registry.md", [("EXCEPTION_TO", "sharing-exposed-defects-did-not-cause-them")]),

    ("each-store-records-its-author", "2026-08-10", "12:00", "joint", 0.95, ["attribution", "author", "store"],
     "Decided and built 2026-08-10: each store records its author in metadata, defaulted from the environment when the store is created. The environment variable was per-process, so one local store opened by two servers with different values records whichever process happened to write -- the same human's personal memories carrying inconsistent attribution, decided by an accident of which window they were in.",
     "Observed before it was fixed, against two real server processes over one local store: one person, one personal store, two identities. Per-scope configuration is the obvious repair and is not quite enough, because the problem is about which store is being written rather than which scope is named.",
     "store-registry.md", [("RESULTS_FROM", "checkout-bound-versus-machine-bound")]),

    ("store-owns-local-facts-environment-defaults", "2026-08-10", "12:05", "joint", 0.9, ["attribution", "pattern", "configuration"],
     "A store owns the local facts about itself, and the environment supplies a default rather than the truth. A variable read at process start is the wrong home for anything that must stay consistent across processes.",
     "It is what git already does -- identity is per repository, for exactly this reason, because one person legitimately contributes under different identities -- and it is the same pattern already applied to the checkout path.",
     "store-registry.md", [("SUPPORTS", "each-store-records-its-author")]),

    ("author-is-local-state", "2026-08-10", "12:10", "joint", 0.85, ["attribution", "export", "data-model"],
     "The store's author is local state and never reaches a memory file's header. What travels is the per-memory created-by and modified-by, written from it at store time and already part of the data model.",
     "Keeping the distinction sharp is what stops the author being treated as set identity, which would put machine-local state into the artifact every copy has to agree on.",
     "store-registry.md", [("DEPENDS_ON", "each-store-records-its-author")]),

    ("correctability-not-accuracy-at-creation", "2026-08-10", "12:15", "joint", 0.9, ["attribution", "defaults", "correction"],
     "The default chain ends at the shell user, which has no reason to match the identity a contributor is known by in a repository. A smarter default is available -- read the git configuration from the registered checkout -- and it is not the right answer: it buys a better guess at the cost of a git dependency, works only for stores that have a checkout, and still guesses. The lever is correctability, not accuracy at creation.",
     "So the tool gains an action to set a store's author, with two supports that make a wrong value get noticed: report the author in the export response, which is the last cheap moment to fix it, and in status, which already scans every store.",
     "store-registry.md", [("RESULTS_FROM", "each-store-records-its-author")]),

    ("author-update-applies-to-future-writes", "2026-08-10", "12:20", "joint", 0.85, ["attribution", "update", "reporting"],
     "An author update applies to future writes only. Memories already stored keep the attribution they were written with, and the response says how many carry a different author, so the split is visible rather than discovered later.",
     "Reporting the residue at the moment of the change is what keeps a partial fix from being mistaken for a complete one.",
     "store-registry.md", [("DEPENDS_ON", "correctability-not-accuracy-at-creation")]),

    ("claimed-never-taken-back", "2026-08-10", "12:25", "joint", 0.95, ["attribution", "shared-store", "convergence"],
     "A store may be claimed, never taken back. An absent author is filled in automatically -- whoever writes first claims the store for their identity, which converges because every later process agrees on the value it finds. A present author is never displaced by a process: a differently configured server writes under the identity it finds, and says so. Only the explicit action moves it.",
     "Automatic repair of a mismatch is what oscillates, and the reason is not obvious from either side: two servers would each restamp the store on every write forever, and each one's local experience is success. The alternation is only visible from outside. Gap-filling has no such shape.",
     "store-registry.md", [("RESULTS_FROM", "each-store-records-its-author")]),

    ("fill-in-but-never-overwrite", "2026-08-10", "12:30", "joint", 0.9, ["shared-store", "pattern", "embeddings"],
     "A process may fill in what a shared store does not know; it may not overwrite what the store already says. The same distinction now governs two unrelated fields -- which model built the vectors, and who the memories are attributed to -- which suggests it is a property of shared stores rather than of either feature.",
     "It was reached independently for the embedding model and then recognised here, and a rule arrived at twice by different routes is one worth stating at the level both instances share.",
     "store-registry.md", [("RESULTS_FROM", "claimed-never-taken-back")]),

    ("reading-does-not-claim", "2026-08-10", "12:35", "joint", 0.9, ["attribution", "recall", "claiming"],
     "Reading does not claim. A recall against an unclaimed store leaves it unclaimed, because otherwise the identity a store settles on is decided by whichever process happened to start first -- the same accident this section exists to remove, moved one step earlier.",
     "The disagreement is reported instead: status says when this server is configured for an identity the set does not use, because a silently ignored setting is indistinguishable from a broken one. Reporting rather than acting is what keeps the store stable.",
     "store-registry.md", [("SUPPORTS", "claimed-never-taken-back")]),

    ("re-attribution-procedure", "2026-08-10", "12:40", "joint", 0.95, ["attribution", "re-attribution", "export"],
     "No bulk re-attribution action is designed, because only a human can tell the two cases apart: that was me under a name I no longer use is a label being corrected, while that was someone else is a record being falsified. But the capability exists and is not a workaround -- export the set, edit the identities in the file, import it back, verified end to end with every edited record reconciling cleanly.",
     "The route is better placed than a tool action would be. The objection was never that re-attribution is hard; it is that the judgement belongs to a human. Export-edit-import puts a human at exactly that judgement and produces a reviewable diff as the artifact, where a tool action would have to ask the question and accept whatever it was told.",
     "store-registry.md", [("RESULTS_FROM", "correctability-not-accuracy-at-creation")]),

    ("import-keeps-the-file-s-attribution", "2026-08-10", "12:45", "joint", 0.95, ["attribution", "import", "invariant"],
     "The property re-attribution rests on is that import writes a record exactly as the file gives it, attribution included. That is load-bearing and easy to lose: stamping an imported record with the local identity is a plausible-looking one-line change, since memorize and amend both correctly do that, and it would foreclose the route entirely while also rewriting the authorship of everyone else's memories on every pull.",
     "It is held by two named tests and a recorded mutation rather than by the argument, because the change that would break it looks like a consistency improvement from the side it would be made on.",
     "store-registry.md", [("DEPENDS_ON", "re-attribution-procedure")]),

    ("four-things-to-get-right-re-attributing", "2026-08-10", "12:50", "joint", 0.85, ["attribution", "re-attribution", "tombstone"],
     "Four things to get right when re-attributing: tombstones carry attribution too, so a rewrite covering only created and modified leaves one identity behind in exactly the records nobody re-reads; the store's own recorded author is separate, so doing only the rewrite means new memories keep arriving under the old name; other export targets report keep-local rather than breaking, and the next export brings them into line; and for a set others have already pulled the diff rewrites authorship across history, large and alarming even when correct.",
     "The local set is the easy case, having no one to alarm, and is where the attribution defect actually bit -- which is why the procedure could be verified somewhere the fourth point did not apply.",
     "store-registry.md", [("SUPPORTS", "re-attribution-procedure")]),

    ("never-keep-a-base-you-cannot-prove", "2026-08-11", "13:00", "joint", 0.95, ["base", "migration", "reconciliation"],
     "Decided 2026-08-11: never keep a reconciliation base you cannot prove. A migration keeps nothing where the store has no targets, keeps the single target where the choice is forced and the spellings provably denote one directory, and where there are two or more keeps at most the destination and drops every other target's rows outright.",
     "The open question had weighed adopting the most-recent target against refusing and asking as comparable options. They are not, and measuring the difference mostly answers the question.",
     "store-registry.md", [("RESULTS_FROM", "base-is-per-record-and-per-target")]),

    ("wrong-base-is-worse-than-no-base", "2026-08-11", "13:05", "joint", 0.95, ["base", "reconciliation", "asymmetry"],
     "A base carried over from a different target is not a weaker fact than no base at all -- it is a false one, and reconciliation treats the two nothing alike. With no base the two present versions are compared directly: identical converges, different is a surfaced conflict, and nothing is written over. With a wrong base that happens to equal what the store holds, the arithmetic reads store unmoved and file moved and takes the file. The store's version is replaced and nothing is reported.",
     "One direction costs a human a look; the other loses a memory. The asymmetry is arithmetic rather than judgement, and it is pinned as an invariant over the whole absent-base branch rather than as one more row of the table.",
     "store-registry.md", [("SUPPORTS", "never-keep-a-base-you-cannot-prove")]),

    ("never-merge-across-targets", "2026-08-11", "13:10", "joint", 0.9, ["base", "migration", "merge"],
     "Never merge across targets. Taking the most recent base per record is the tidy-looking move and it is precisely the defect: it assembles an ancestor that no file ever agreed to, one record at a time.",
     "It also disposes of refuse-and-ask, on the grounds that the question cannot be asked -- which of two checkouts' agreements is the real ancestor is not knowable from outside the store, so a human at that prompt has nothing to decide with. Dropping requires no one's judgement and cannot be answered wrongly.",
     "store-registry.md", [("SUPPORTS", "never-keep-a-base-you-cannot-prove")]),

    ("canonicalisation-lands-in-the-same-place", "2026-08-11", "13:15", "joint", 0.9, ["base", "migration", "canonicalisation"],
     "The other route -- normalise the key rather than remove it -- needs a migration too, and it is this one. Re-keying one spelling to another is safe exactly when the two provably denote one directory. What it must not do is treat a collision as a merge: two raw keys that canonicalise to one carry two independent base sets, and wherever they disagree neither is the other's ancestor.",
     "That both routes reach the same rule is why this could be settled before choosing between them -- the migration constraint does not depend on which one wins.",
     "store-registry.md", [("SUPPORTS", "never-keep-a-base-you-cannot-prove")]),

    ("what-dropping-a-base-costs", "2026-08-11", "13:20", "joint", 0.9, ["base", "migration", "cost"],
     "Dropping is not free. A dropped base turns a known record into an unknown one, so a record the two sides once agreed on that the store no longer holds returns as new-from-file instead of being reported as an anomaly -- a silent resurrection where there had been a report. The second cost is one noisy cycle, since every dropped base is re-established by the next export.",
     "A resurrection can be forgotten again; an overwritten memory cannot be recovered. The rule stands, and this is a cost it accepts rather than one it overlooked.",
     "store-registry.md", [("EXCEPTION_TO", "never-keep-a-base-you-cannot-prove")]),

    ("the-guard-is-what-is-time-sensitive", "2026-08-11", "13:25", "joint", 0.85, ["base", "migration", "urgency"],
     "The project's one existing bases migration already obeys this rule, having dropped the table whole on finding an older shape. That downgrades the urgency the question carried: it was filed as the one open question where being late loses memories, which is true of a migration that guesses and false of one that drops. What is time-sensitive is the guard -- the rule wants to be written down and tested before someone writes the tidy-looking merge.",
     "Measured 2026-08-11, every migration described is currently a no-op or a forced choice, so there is still nothing to get wrong.",
     "store-registry.md", [("RESULTS_FROM", "never-merge-across-targets")]),

    ("no-local-aliases", "2026-08-13", "14:00", "joint", 0.95, ["store-registry", "alias", "identity"],
     "A scope is bound to one checkout at a time and there is no local alias mechanism naming the same set at a second path. The capability was considered, because a reviewing session plausibly wants two checkouts at once, and it is rejected for a reason that would be very hard to recover from: a set's name is shared identity, travelling in the file header and in cross-set link targets, while an alias is per-machine state.",
     "A cross-set link written against an alias is meaningless on every other machine and travels in the file looking authoritative -- machine-local state laundered into shared identity, silently, by an ordinary memory write. Making it safe would require every link-writing path to resolve alias to canonical name without exception.",
     "store-registry.md", [("DEPENDS_ON", "identity-travels-location-does-not")]),

    ("alias-would-need-a-typed-handle", "2026-08-13", "14:05", "joint", 0.8, ["store-registry", "alias", "types"],
     "If two live registrations are ever genuinely needed, the design to revisit is a typed handle that is structurally ineligible to appear as a link target -- not a second name for a scope.",
     "Recording the shape a future answer would have to take is what keeps the rejected option from being re-proposed in its rejected form, since the objection is to the naming rather than to the capability.",
     "store-registry.md", [("EXCEPTION_TO", "no-local-aliases")]),

    ("inspection-is-read-only", "2026-08-13", "14:10", "joint", 0.9, ["store-registry", "inspect", "review"],
     "The review workflow does not need the reviewed set loaded, since the diff is visible to the LLM and the human alike and review questions are answered by reading rather than by recall. So there is a read-only inspect action: it reads a memory file at a caller-named path, reports its header and records, and changes nothing -- no store touched, no base recorded, no scope bound.",
     "Recognising that the workflow needs reading rather than loading is what keeps a whole second registration mechanism from being built for it.",
     "store-registry.md", [("RESULTS_FROM", "no-local-aliases")]),

    ("inspect-does-not-trust-the-header", "2026-08-13", "14:15", "joint", 0.9, ["store-registry", "inspect", "trust"],
     "Inspection is the one operation that should read a header without trusting it. Everywhere else a file's claimed scope can only agree with what the path already established, which is the rule that closes the privilege-escalation channel. Inspection grants no trust at all, since nothing is loaded and so nothing can be escalated into, so it should accept a file claiming any scope and report what it claims, labelled as a claim.",
     "That is strictly more useful for review, since seeing a fork's header still asserting the upstream set name is exactly the thing a reviewer needs to notice.",
     "store-registry.md", [("EXCEPTION_TO", "header-scope-checked-never-used"), ("DEPENDS_ON", "inspection-is-read-only")]),

    ("status-must-peek-not-load", "2026-08-13", "14:20", "joint", 0.95, ["store-registry", "status", "recall"],
     "The status scan must peek, not load. Reading metadata and a count has to be a distinct path from opening a store and adding it, because loading a non-current store would union its memories into recall and destroy the entire point of re-pointing.",
     "The failure would be silent and in the reassuring direction -- recall working suspiciously well -- so it is worth naming here rather than discovering it.",
     "store-registry.md", [("DEPENDS_ON", "registry-holds-one-fact")]),

    ("store-states-worth-distinguishing", "2026-08-13", "14:25", "joint", 0.85, ["store-registry", "status", "states"],
     "The states status distinguishes: current with its path present is healthy; not current with its path present is dormant and not a problem; never registered is unattached and legitimate; registered with its path gone is orphaned, and those memories may never have been exported; current with its path gone cannot export at all; and a registry naming a store that is not there is what the force-drop escape hatch produces, so it is expected rather than anomalous.",
     "Memory count belongs in the report, because an orphan holding two hundred memories is a different problem from one holding zero, and that number is what decides whether recovery is worth anything.",
     "store-registry.md", [("SUPPORTS", "status-must-peek-not-load"), ("RELATED_TO", "forcing-a-drop-is-outside-the-tool")]),

    ("unattached-set-is-first-class", "2026-08-13", "14:30", "human", 0.95, ["store-registry", "unattached", "adoption"],
     "A shared set need not belong to a repository, and a store with no recorded path is healthy, reported as unattached, and never flagged. The concrete case: a developer keeping project memories for a project whose maintainer has not agreed to carry the reserved directory. Those memories are about the project, belong in a named set rather than in local, and have nowhere to be exported to yet.",
     "That is not a broken state waiting to be repaired; it is the ordinary condition of a set before anyone is persuaded to adopt it, and it may last months. An earlier draft asserted a cross-project set has a repository so it registers like anything else, which was wrong in the direction that erases this state.",
     "store-registry.md", [("INSTANCE_OF", "scope-kinds-are-private-and-shareable")]),

    ("registration-is-explicit", "2026-08-13", "14:35", "joint", 0.9, ["store-registry", "registration", "export"],
     "Since a set may have no path, something has to say when it acquires one -- and it must not be export. Binding the path implicitly on first export is the tempting shortcut and it is wrong, because the safety copy required by the save-first rule goes somewhere unusual by construction. A set would re-home itself onto a backup destination at exactly the moment it was being protected.",
     "So registration is its own act, and export to any path other than the registered one is a copy that writes the file and touches neither the base nor the registration.",
     "store-registry.md", [("RESULTS_FROM", "unattached-set-is-first-class"), ("DEPENDS_ON", "safety-copy-path-is-not-the-export-path")]),

    ("backup-versus-export-follows-from-the-path", "2026-08-13", "14:40", "joint", 0.85, ["store-registry", "export", "backup"],
     "This recovers for free the distinction collapsed when backup was merged into export. The two operations were rightly made one, because the caller states a scope and everything else follows from it; now backup-versus-export follows from whether the path is the set's home, which is a fact the software already holds. Neither a second tool nor a mode argument.",
     "A distinction that returns as a derived fact rather than as a parameter is the shape the decision-placement test keeps producing, and it is worth noticing when a later mechanism restores something an earlier one gave up.",
     "store-registry.md", [("RESULTS_FROM", "registration-is-explicit"), ("SUPPORTS", "export-and-backup-are-one-operation")]),

    ("orphans-flagged-not-alerted", "2026-08-13", "14:45", "joint", 0.9, ["store-registry", "orphan", "alerting"],
     "An orphan is worth reporting, and reporting it means a marked row in status with its memory count -- not a warning raised during ordinary work. The argument for alerting more loudly was that an orphan is the passive form of the risk the save-first rule addresses, and that much holds, but reporting and interrupting are not the same act.",
     "During ordinary memorize-and-recall work, knowing about an orphan changes nothing the caller does; it matters only to someone managing sets, who is the person running status. What makes an orphan noticeable is specificity, not volume.",
     "store-registry.md", [("INSTANCE_OF", "decision-two-checks"), ("SUPPORTS", "store-states-worth-distinguishing")]),

    ("registry-does-not-violate-identity-travels", "2026-08-13", "15:00", "joint", 0.85, ["store-registry", "identity", "consistency"],
     "Identity travels, location does not is not violated by the registry. That rule objects to binding a path into a set's identity, where copies on other machines would disagree. The registry is explicitly machine-local and never enters a header. Recording a path locally was always fine; publishing it was not.",
     "Checking a new mechanism against a rule it superficially contradicts, and finding the rule narrower than it looked, is what keeps a principle from being either violated or over-applied.",
     "store-registry.md", [("EXCEPTION_TO", "identity-travels-location-does-not")]),

    ("per-target-base-becomes-unnecessary", "2026-08-13", "15:05", "joint", 0.9, ["store-registry", "base", "simplification"],
     "The per-target base becomes unnecessary, and with it the known-targets query, the unrecognised-target remark on export, and the whole dirty-relative-to-what discussion. With one base per record, cleanliness is a plain boolean per set, and the per-target reporting question goes away with the per-target base.",
     "It also settles what status reports, which had been left open -- a question that dissolves rather than being answered is the signature of the right cause having been found.",
     "store-registry.md", [("SUPERSEDES", "base-is-per-record-and-per-target")]),

    ("unconditional-save-survives-on-new-legs", "2026-08-13", "15:10", "joint", 0.95, ["store-registry", "save-first", "argument"],
     "The unconditional safety copy survives but its argument has to be rewritten. One leg does not survive here, since one destination means one base and a well-defined answer to whether a set is dirty. The decision stands on the leg that does -- a base records a past agreement rather than the current contents of a file -- and on two cases this design creates: an unattached set has no destination and so no base at all, making every record unsaved by construction, and an orphaned store can report clean because it genuinely agreed with its file before the checkout was deleted.",
     "The orphan is where a flag is actively wrong: a flag-gated drop would skip the safety copy in the one case where the memories exist nowhere else.",
     "store-registry.md", [("SUPPORTS", "save-is-unconditional")]),

    ("registration-must-be-optional", "2026-08-13", "15:15", "joint", 0.9, ["store-registry", "registration", "local"],
     "The local set needs no registration, and neither does any shared set that has not acquired a checkout. Local is the permanent case, having no repository by definition, while an unattached shared set is a state a set passes through, possibly for a long time. Both are stores with no registry entry, which is why registration has to be optional rather than a precondition for a set existing.",
     "Making registration a precondition is the implementation shortcut this reasoning exists to forbid, and it would show up as a refusal to create a set without a path.",
     "store-registry.md", [("RESULTS_FROM", "unattached-set-is-first-class")]),

    ("how-the-llm-learns-its-checkout", "2026-08-13", "15:20", "joint", 0.8, ["store-registry", "open-question", "checkout"],
     "Open: how the LLM learns which checkout it is in. The server cannot see the session's working directory and the model can, so publishing the registered path in status and in the memorize response lets the model compare against what it knows. Whether that belongs in a convention is a separate question, and the convention set is deliberately being kept small.",
     "The split matters because the reporting half is cheap and available now, while the convention half spends a scarce budget, so they should not be decided together.",
     "store-registry.md", [("DEPENDS_ON", "status-reports-the-file-as-a-fact")]),

    # ---- recall-mechanisms.md -------------------------------------------------------------
    ("semantic-similarity-is-primary", "2026-08-14", "09:00", "human", 0.9, ["recall", "semantic-search", "seeds"],
     "The primary recall mechanism is semantic similarity: generate a set of topic keys, then perform a nearest vector search against the memory content's embedding, probably by cosine similarity, to establish a set of seed memories. Traversal proceeds from those seeds, decaying a memory's effective confidence by its native confidence and its distance from the seed.",
     "Naming seeds as a distinct stage is what lets the search and the walk be designed and measured separately, which every later decision in this document depends on.",
     "recall-mechanisms.md", []),

    ("embedding-input-includes-why-and-keywords", "2026-08-14", "09:05", "joint", 0.95, ["embeddings", "recall", "ablation"],
     "Both the why text and the keywords are included in the embedding input by default. The why often surfaces related topics or records of paths not taken, which is exactly what semantic recall should be able to find, and keywords are already curated for topical relevance, so including them is close to free.",
     "Measured rather than reasoned: an ablation over a 45-memory labelled corpus with 20 queries put recall at five results at 57.5% for content alone, 65.8% with keywords, 66.7% with why, and 71.7% with both. Both additions help and they compose, so the decision holds on evidence.",
     "recall-mechanisms.md", [("SUPPORTS", "semantic-similarity-is-primary")]),

    ("already-known-memories", "2026-08-14", "09:10", "joint", 0.9, ["recall", "context", "efficiency"],
     "A recall call can name memories the LLM already has in context. A memory already in context is pure cost if returned again, and naming it lets a second call mean something other than what you just gave me, which is otherwise impossible against a deterministic ranking.",
     "The second benefit is the less obvious one and arguably the larger: without it, refining a query means rewording it and hoping, because the ranking is a function of the query alone.",
     "recall-mechanisms.md", []),

    ("suppresses-output-not-participation", "2026-08-14", "09:15", "joint", 0.95, ["recall", "seeds", "traversal"],
     "Naming an already-held memory removes it from the results but not from seed selection or graph traversal. A memory the LLM already holds and is now querying around is the best available seed -- a known-relevant, already-validated anchor into the right neighbourhood -- and its links remain valid connective tissue, so excluding it from the walk would disconnect whatever lies beyond it.",
     "So this is a recall improvement rather than merely a cost saving: the caller contributes free high-quality seeds by declaring what it already has.",
     "recall-mechanisms.md", [("RESULTS_FROM", "already-known-memories")]),

    ("no-dont-seed-from-this-control", "2026-08-14", "09:20", "joint", 0.9, ["recall", "hubness", "tool-surface"],
     "There is no separate control for suppressing bad seeds. Hubness might look like a reason for one, and it is not: hubness is a corpus-level statistical property of the whole vector distribution, and an LLM cannot tell which memories are hubs -- it only sees what came back. A control that requires information the caller does not have will be pulled wrongly or not at all.",
     "Hubness belongs in the ranking, not in the tool surface. A memory that is genuinely wrong is likewise not a case for per-call filtering; it should be corrected, superseded or forgotten.",
     "recall-mechanisms.md", [("INSTANCE_OF", "decision-placement")]),

    ("per-call-parameter-not-session-state", "2026-08-14", "09:25", "joint", 0.85, ["recall", "state", "tool-surface"],
     "Already-known memories are a per-call parameter rather than session state. The server does not track what a session has seen; the caller passes what it holds. That keeps the tool surface simple and avoids the server holding state it cannot reliably invalidate.",
     "It is consistent with the design's general stance of trusting the LLM's judgement in the moment rather than legislating it in software.",
     "recall-mechanisms.md", [("SUPPORTS", "already-known-memories")]),

    ("inverse-pairs-canonicalised-at-read", "2026-08-14", "10:00", "joint", 0.95, ["links", "inverse", "canonicalisation"],
     "Two labels describing the same relation from opposite ends may both be stored for one relation, and traversal must treat them as one edge or it double-counts. Normalising to one direction when the link is written is not available here: a memory in a set this machine does not own cannot be rewritten, so asserting the relation from whichever side is editable is often the only option. Normalising on write assumes ownership of both endpoints, which a multi-set system does not have.",
     "So the reverse direction is derived and held locally, and what matters is the guarantee rather than the mechanism: every stored edge is discoverable from both endpoints whenever both endpoints' sets are loaded, without either memory being rewritten in its exported form.",
     "recall-mechanisms.md", []),

    ("when-the-reverse-becomes-available", "2026-08-14", "10:05", "joint", 0.9, ["links", "reverse-index", "sets"],
     "The write side needs no case analysis, because a memory cannot be stored into a set that is not loaded, so the forward edge is always durably written to a set this machine holds. Only the target varies: in the same or another loaded set the reverse is derived immediately, in a known but unloaded set it is derived when that set is next loaded, and in a set this machine does not have it is not derivable at all.",
     "That last case is the bound on the guarantee, and it is the same bound as a dangling cross-set reference rather than a new limitation.",
     "recall-mechanisms.md", [("DEPENDS_ON", "inverse-pairs-canonicalised-at-read")]),

    ("derivation-on-every-write-not-at-load", "2026-08-14", "10:10", "joint", 0.95, ["links", "reverse-index", "lifecycle"],
     "At load time is not sufficient on its own, which is easy to get wrong. The server is long-running and backed by a database, so a write happens mid-session and there may be no load event for days. An LLM that records a supersession, then starts a fresh session with cleared context, must still find the superseded memory marked. Derivation is maintained on every write, not on an event.",
     "Nothing in the session lifecycle triggers a rebuild, so tying derivation to a lifecycle event produces a correctness gap whose width is however long the server happens to stay up.",
     "recall-mechanisms.md", [("SUPPORTS", "when-the-reverse-becomes-available")]),

    ("reverse-preserves-label-and-direction", "2026-08-14", "10:15", "joint", 0.95, ["links", "labels", "vocabulary"],
     "A derived reverse is presented as an incoming edge carrying the label as written -- recall shows that A supersedes this memory -- rather than as a synthesised inverse link on the target. Synthesising inverses would require names for directions the vocabulary deliberately does not have, and the fourteen labels were settled precisely because the LLM only ever writes outbound.",
     "The decisive reason is that it would destroy a distinction the vocabulary exists to carry: an authored acknowledgement of being replaced means the lower-authority side concedes, while a derived one would mean only that someone else asserted they replaced you. Conflating them loses exactly the information that made the label worth having.",
     "recall-mechanisms.md", [("RESULTS_FROM", "inverse-pairs-canonicalised-at-read")]),

    ("reverse-maintenance-rules", "2026-08-14", "10:20", "joint", 0.9, ["links", "reverse-index", "import"],
     "The derived reverse is incremental on write, because the change is known exactly; fully recomputed on import and never incrementally patched, because a pull can remove a forward edge and patching would leave a reverse pointing at an edge that no longer exists; never authoritative, so it must be reconstructible from the forward edges alone; and subject to the trust rule, so a project memory asserting supersession against a local memory produces no reverse.",
     "Never authoritative is the rule that makes the other three safe to get wrong, since a corrupt or stale derivation is always recoverable by rebuilding.",
     "recall-mechanisms.md", [("DEPENDS_ON", "derivation-on-every-write-not-at-load")]),

    ("export-structurally-unable-to-carry-reverse", "2026-08-14", "10:25", "joint", 0.95, ["links", "export", "correctness-by-construction"],
     "If the reverse is materialised onto the memory's record, the export path must not emit it -- and must not emit it as a filter is a step someone forgets, the same failure mode that argued for partitioned storage over a scope clause. The exported type should simply not have the field, so omitting it is not an action anyone can skip.",
     "Making the wrong behaviour unrepresentable rather than merely forbidden is the same move the scope type makes, and it is the one that survives a refactor by someone who never read the rule.",
     "recall-mechanisms.md", [("SUPPORTS", "reverse-maintenance-rules")]),

    ("reverse-derivation-crosses-the-partition", "2026-08-14", "10:30", "joint", 0.9, ["links", "scope", "partition"],
     "Reverse derivation deliberately crosses the scope partition that keeps a query against one project's store from returning another scope's memories, because a local memory superseding a project memory means recalling the project memory should surface the local one. That is safe because the partition is protecting export, and edges live on the source memory, so exporting a project set never carries an edge that lives on a local memory.",
     "The partition holds where it was needed and the derivation spans where recall needs it, which is only defensible once what the partition protects is stated precisely.",
     "recall-mechanisms.md", [("EXCEPTION_TO", "export-structurally-unable-to-carry-reverse")]),

    ("supersession-is-validity-not-relevance", "2026-08-14", "11:00", "joint", 0.95, ["supersession", "validity", "ranking"],
     "Supersession is a validity relation, not a relevance relation. Every other label affects how likely a memory is to be worth surfacing; this one affects whether it is still true, which has to be resolved before ranking rather than as part of it. It is a different question that happens to be carried by an edge.",
     "That is why deciding it ahead of the general per-label question prejudges nothing. It also could not wait, because the whole purpose of recording a supersession is that a later session holding none of the context is told the memory is obsolete -- an edge derived correctly but never surfaced is a correction that reaches nobody.",
     "recall-mechanisms.md", []),

    ("resolve-to-the-tip-report-the-hit", "2026-08-14", "11:05", "joint", 0.95, ["supersession", "recall", "chains"],
     "Recall resolves supersession to the end of the chain and returns that memory in full, together with the id of the memory the query actually matched and nothing else about it. Returning the intermediate would surface a conclusion that was itself corrected; returning nothing about the match would leave the caller holding a replacement for a query that matched something else, with no idea why.",
     "The id is the whole of the report -- no gloss, no excerpt, no date. Supersession most often corrects a memory whose conclusion turned out to be wrong rather than one that merely aged, so its substance is actively misleading and surfacing any of it risks a corrected conclusion being read as live context.",
     "recall-mechanisms.md", [("RESULTS_FROM", "supersession-is-validity-not-relevance")]),

    ("chains-document-themselves", "2026-08-14", "11:10", "joint", 0.85, ["supersession", "chains", "traversal"],
     "Chains need no further reporting because they document themselves. The supersedes edge lives on the superseding memory, so its own link list already names what it replaced, and a caller can walk back one hop at a time through ordinary traversal.",
     "Listing intermediates would spend tokens on memories that were each wrong enough to be replaced, which is the opposite of what the report is for.",
     "recall-mechanisms.md", [("SUPPORTS", "resolve-to-the-tip-report-the-hit")]),

    ("direct-lookup-exempt-from-substitution", "2026-08-14", "11:15", "joint", 0.95, ["supersession", "lookup", "exemption"],
     "Direct id lookup is exempt from substitution: asking for a memory by id means wanting that memory, so it is returned and marked as superseded. Without this exemption the prior version becomes unreachable, which would make superseding equivalent to forgetting and defeat the decision to keep it.",
     "Forgetting already exists and is just deleting the line; an author who superseded instead chose to keep the memory, and recall should not overrule that.",
     "recall-mechanisms.md", [("EXCEPTION_TO", "resolve-to-the-tip-report-the-hit")]),

    ("supersession-is-one-to-many", "2026-08-14", "11:20", "joint", 0.85, ["supersession", "splitting", "resolution"],
     "Supersession is one-to-many. The convention for splitting a partly-stale memory produces several memories each superseding the original, so resolution returns every tip rather than the replacement.",
     "A resolver written for a single successor would silently drop the other tips, and the splitting convention is what makes the many case ordinary rather than exotic.",
     "recall-mechanisms.md", [("EXCEPTION_TO", "resolve-to-the-tip-report-the-hit")]),

    ("two-ways-supersession-is-learned", "2026-08-14", "11:25", "joint", 0.9, ["supersession", "availability", "cross-set"],
     "Supersession is learned two ways with different availability. Derived from the superseding memory's edge, it requires that memory's set to be loaded, and if it is not, the superseded memory is returned unmarked -- not known at all. Authored as an acknowledgement on the memory itself, it needs nothing, and remains reportable as superseded with the replacement unavailable.",
     "This asymmetry decides what can be reported and is the reason the date is not: a date could only ever be reported opportunistically, and a field that is sometimes present teaches the reader to expect it and misleads when it is absent.",
     "recall-mechanisms.md", [("SUPPORTS", "supersession-is-validity-not-relevance")]),

    ("second-argument-for-superseded-by", "2026-08-14", "11:30", "joint", 0.9, ["supersession", "cross-set", "labels"],
     "This is a second, independent argument for the authored acknowledgement label, which was adopted on trust-direction grounds: it is also the only form that survives the superseding memory being unavailable. Resolution otherwise goes as far as the loaded sets allow, returning the furthest memory that is loaded when the tip's set is not.",
     "A label justified twice from unrelated directions is one that will survive either argument being revisited, which is worth recording separately from the original case.",
     "recall-mechanisms.md", [("SUPPORTS", "two-ways-supersession-is-learned"), ("RELATED_TO", "superseded-by-exists-for-trust")]),

    ("supersession-cycles-both-places", "2026-08-14", "11:35", "joint", 0.9, ["supersession", "cycles", "robustness"],
     "A supersession cycle is a data error handled in both places: rejected at write, since when both memories are in loaded sets the store can see the cycle forming and refusing it there is cheaper than defending every use of validity resolution against non-termination; and reported at read, which is not redundant, because write-time rejection only covers writes this store made.",
     "A hand-edit during a merge conflict introduces a cycle no write path ever saw, so recall has to defend itself against a store it did not fully author -- the same posture it takes toward dangling references. Resolution carries cycle detection and a depth bound regardless.",
     "recall-mechanisms.md", [("SUPPORTS", "resolve-to-the-tip-report-the-hit")]),

    ("walk-generates-ranking-judges", "2026-08-14", "12:00", "joint", 0.95, ["traversal", "ranking", "separation"],
     "Most of the difficulty in the per-label question came from asking one mechanism to do two jobs. The walk generates candidates and the embedding judges them: expansion from the seeds is uniform and symmetric, every label crossed the same way in both directions, decaying with distance and with nothing else. It makes no attempt to judge relevance, because the graph does not know relevance -- it knows what some author thought worth connecting.",
     "Relevance is judged afterwards by scoring each candidate against the query, which needs no information the store does not already hold, since every memory has a vector and so does the query.",
     "recall-mechanisms.md", [("RESULTS_FROM", "semantic-similarity-is-primary")]),

    ("uniform-expansion-licensed-by-rescoring", "2026-08-14", "12:05", "joint", 0.95, ["traversal", "ranking", "dependency"],
     "Uniform expansion is licensed by the re-scoring, not by itself. A walk that wanders out of the sense a query intended is harmless because the cut re-scores everything -- think of ice alongside computers, where a related edge into frozen water is picked up by the expansion and dropped by the ranker. Were ranking ever to run on graph distance alone, that same crude expansion would become a defect.",
     "The tolerance for a rough walk is bought by the ranker and is not a property of the walk, which is a dependency rather than a happy accident and constrains any later change.",
     "recall-mechanisms.md", [("DEPENDS_ON", "walk-generates-ranking-judges")]),

    ("inherited-score-must-survive", "2026-08-14", "12:10", "joint", 0.95, ["traversal", "ranking", "score"],
     "If a candidate's final score were only its own similarity to the query, traversal would return nothing that a larger limit would not return sooner and more cheaply -- and it would penalise the very case it exists for. A memory is often relevant because of its relation rather than its wording: the measurement supporting a decision, the constraint that decision rests on. Those are precisely the memories whose content does not match the query.",
     "This is measured rather than supposed: a memory carrying a rare identifier ranked below an unrelated memory on content similarity. So a reached memory carries a decayed share of its seed's score and the query-similarity term shapes the result on top of it.",
     "recall-mechanisms.md", [("SUPPORTS", "walk-generates-ranking-judges")]),

    ("bounds-now-weights-flat", "2026-08-14", "12:15", "joint", 0.95, ["traversal", "tuning", "evidence"],
     "A bound is a safety property: set too tight it is conservative rather than incorrect, and it can be defended with no corpus at all. A weight -- the decay rate, any in-degree term, any per-label variation -- is a tuning problem, and every weight here is currently untunable. So bounds may be set now and every weight starts flat, recorded as untuned rather than chosen.",
     "A flat weight that is wrong costs ranking quality and announces itself as provisional; a weight guessed to three decimals reads as reasoned about and gets built upon. This distinction is what makes the walk buildable ahead of the evidence.",
     "recall-mechanisms.md", [("SUPPORTS", "walk-generates-ranking-judges")]),

    ("bound-is-on-expansion-not-inclusion", "2026-08-14", "12:20", "joint", 0.9, ["traversal", "bound", "frontier"],
     "The bound is on expansion, not on inclusion. Reaching a heavily-cited memory is cheap and it is often the most useful thing returned; expanding through it is what branches. So the limit applies to using a memory as a frontier node, never to admitting it as a result.",
     "A cap on inclusion would discard good results to solve a problem they did not cause, which is the natural mistake when the cost being controlled is described as too many memories.",
     "recall-mechanisms.md", [("SUPPORTS", "bounds-now-weights-flat")]),

    ("in-degree-sign-is-label-dependent", "2026-08-14", "12:25", "joint", 0.9, ["traversal", "in-degree", "hubness"],
     "Whether the mechanism should read in-degree as a ranking signal is not answered, because the sign is label-dependent. High inbound motivation means load-bearing -- many decisions rested on it. High inbound instance-of means generic: it accumulated children by being unspecific. The same number pulls opposite ways, so it is a weight and it is held flat with the rest.",
     "The argument that the caller gets no control settles placement only. This is graph hubness rather than the vector hubness largely fixed by centroid removal -- different properties of possibly different memories, and neither ruling transfers to the other.",
     "recall-mechanisms.md", [("DEPENDS_ON", "bounds-now-weights-flat"), ("RELATED_TO", "no-dont-seed-from-this-control")]),

    ("recall-is-advisory-supersession-is-not", "2026-08-14", "12:30", "joint", 0.9, ["traversal", "standards-of-proof", "recall"],
     "Recall is advisory in a way supersession is not. A mediocre walk returns a slightly worse candidate set, and the next query -- with different wording, which the conventions push for -- recovers what it missed. A mistaken supersession serves a claim that is false. The two warrant different standards of proof, and traversal sits on the cheaper side of that line.",
     "This is why starting simple is principled here rather than merely cheap, which is a distinction worth making explicitly whenever a design ships something untuned.",
     "recall-mechanisms.md", [("SUPPORTS", "bounds-now-weights-flat")]),

    ("published-tuning-does-not-transfer", "2026-08-14", "12:35", "joint", 0.9, ["traversal", "tuning", "literature"],
     "Published tuning for graph-augmented retrieval is not a shortcut past this. Nearly all of it concerns graphs derived from text automatically, where the edges are statistical; here every edge was authored deliberately by the writer of one of its endpoints. Constants would not transfer, and borrowing them would reintroduce exactly the unmeasured numbers this design has avoided elsewhere.",
     "The temptation is strongest precisely when a corpus is not yet available, which is when borrowed constants would be least checkable.",
     "recall-mechanisms.md", [("SUPPORTS", "recall-is-advisory-supersession-is-not")]),

    ("how-inherited-and-own-scores-combine", "2026-08-14", "12:40", "joint", 0.9, ["traversal", "open-question", "ranking"],
     "Open: how the inherited and own scores combine. A candidate reached by the walk has the decayed share it inherits from its seed and its own similarity to the query. Both are needed -- the first is what makes traversal worth doing, the second is what makes uniform expansion safe. Whether the result is a maximum, a sum, a weighted blend, or the inherited score acting as a floor changes which memories survive the cut, and the choice is currently unevidenced.",
     "This one question replaces most of what per-label traversal behaviour was asking, and it is tractable in a way that question was not: two terms rather than fourteen labels, and a shape that can be measured against a labelled corpus once one exists.",
     "recall-mechanisms.md", [("RESULTS_FROM", "inherited-score-must-survive")]),

    ("collapsed-hits-free-a-slot", "2026-08-14", "12:45", "joint", 0.8, ["traversal", "ranking", "loose-end"],
     "One loose end belongs with the combination question: validity resolution can collapse two hits into one when a memory and its superseder both rank, freeing a result slot, and whether that slot is backfilled is part of the same ranking decision.",
     "Attaching it to the open question rather than deciding it separately keeps two coupled choices from being made in different places by different reasoning.",
     "recall-mechanisms.md", [("DEPENDS_ON", "how-inherited-and-own-scores-combine")]),

    ("contradicts-needs-no-override", "2026-08-14", "13:00", "joint", 0.95, ["contradicts", "traversal", "visibility"],
     "The one candidate exception to uniform expansion was whether a contradiction should force both memories into the results. No -- and the question was aimed at the wrong layer. The problem was visibility, not promotion: under the outbound rule the edge is written on the newer memory, so the older one, likelier to have gone stale and likelier to be surfaced on its own merits, carries no trace of the dispute at all. Its only record of being contradicted is an inbound edge.",
     "Make that edge visible and the label needs nothing from traversal. Promotion would be a relevance instrument aimed at a validity problem, and a disputed memory far from the centre of a search is a dispute worth ignoring, which no ranking rule can tell from a serious one.",
     "recall-mechanisms.md", [("RESULTS_FROM", "walk-generates-ranking-judges"), ("INSTANCE_OF", "decision-placement")]),

    ("contradiction-scales-the-right-way", "2026-08-14", "13:05", "joint", 0.9, ["contradicts", "cross-set", "scaling"],
     "Visibility also scales the right way. Contradiction is rare in the present corpus but must not be assumed rare in general: supersession requires somebody to concede, and across sets two may disagree with neither side wrong and neither able to rewrite the other, leaving contradiction as the only instrument. Visibility costs one identifier per dispute and scales with that; a forced result slot each would not.",
     "The cross-set case is the workflow the vision is built around, so the rate in a single-set corpus is the wrong evidence to generalise from.",
     "recall-mechanisms.md", [("SUPPORTS", "contradicts-needs-no-override")]),

    ("inbound-edges-ship-by-default", "2026-08-14", "13:10", "joint", 0.95, ["inbound-edges", "default", "visibility"],
     "Inbound edges ship by default, and an option may not invert that. An edge that ships only when asked for is invisible to a caller that does not know to ask, which rebuilds the trap exactly: a contradiction nobody sees. A caller may be offered a way to turn inbound edges off, because a caller that suppresses them has decided it does not want them and knows what it gave up. Default-off is a different thing and is not available.",
     "The contradiction resolution depends entirely on the caller seeing inbound edges, so fixing that is a precondition of the decision rather than a related improvement.",
     "recall-mechanisms.md", [("DEPENDS_ON", "contradicts-needs-no-override")]),

    ("label-and-direction-stay-distinct", "2026-08-14", "13:15", "joint", 0.9, ["links", "direction", "labels"],
     "Uniform expansion means direction does not change how an edge is walked. It still changes what an edge means when reported, since an edge is visible from both endpoints under the name it was written with and no inverse label is synthesised. Contradiction is symmetric, so inbound and outbound assert the same thing and one rule covers both. Instance-of is not: outbound is instance to pattern, one hop that generalises, while inbound is pattern to instances, which enumerates.",
     "One label, two operations sharing nothing but a name -- and supersession is the worked precedent, since validity resolution already reads the two directions as answering different questions.",
     "recall-mechanisms.md", [("DEPENDS_ON", "reverse-preserves-label-and-direction")]),

    ("fan-out-is-an-inbound-property", "2026-08-14", "13:20", "joint", 0.95, ["traversal", "in-degree", "fan-out"],
     "A memory's outbound degree is fixed when it is written and bounded by how much its author chose to link. Its inbound degree is written by everyone who cites it afterwards, and grows with the corpus. So outbound traversal is self-limiting and inbound traversal is not, which is what the expansion bound exists to contain and why that bound is an inbound concern.",
     "The asymmetry is durable rather than incidental, which is what makes it safe to build a bound around.",
     "recall-mechanisms.md", [("SUPPORTS", "bound-is-on-expansion-not-inclusion"), ("RESULTS_FROM", "label-and-direction-stay-distinct")]),

    ("count-both-directions", "2026-08-14", "13:25", "joint", 0.95, ["measurement", "graph-shape", "export"],
     "Measured over the 36 live memories of the project's own set: 16 edges carrying four distinct labels, with several of the fourteen having no instances at all, 11 of 36 memories isolated counting both directions, and a highest incident degree of 3. Counting outbound links alone puts isolation at 21 of 36, and that is the wrong number.",
     "It is an easy one to produce, because export carries each edge once on the memory that authored it, so a count taken from the exported file sees only half of every relation. Any measurement of graph shape must count both directions, and the tool surface makes this same error structurally.",
     "recall-mechanisms.md", [("SUPPORTS", "inbound-edges-ship-by-default")]),

    ("pure-keyword-search", "2026-08-14", "14:00", "human", 0.85, ["keyword-search", "tagging", "recall"],
     "The conventions may establish specific ways of tagging certain information -- gathering evidence on a bug across multiple merges, or tagging work associated with a branch. It should be possible to generate seed memories purely through keyword search, and maybe to disable graph traversal during the search in order to surface only memories tagged that specific way.",
     "Keyword search answers a different question from similarity: an exact tag is a claim about membership rather than about meaning, and traversal from such a seed would dilute exactly the precision that made the tag worth writing.",
     "recall-mechanisms.md", [("EXCEPTION_TO", "semantic-similarity-is-primary")]),

    ("direct-reference-supports-prefixes", "2026-08-14", "14:05", "human", 0.85, ["lookup", "uuid", "prefix"],
     "Memories have globally unique ids and it should be possible to search by them. Some models habitually truncate a UUID when recording a reference to it, and the practice is common in conversations about git commits, so the lookup tools for direct reference should also support prefix-based searches.",
     "Designing for the truncation rather than against it is cheaper than trying to stop it, and the git precedent means callers already expect a prefix to work.",
     "recall-mechanisms.md", [("RELATED_TO", "direct-lookup-exempt-from-substitution")]),

    # ---- embeddings.md --------------------------------------------------------------------
    ("vectors-are-local-derived-state", "2026-08-09", "09:00", "joint", 0.95, ["embeddings", "export", "derived-state"],
     "Vectors are computed locally and never appear in the exported memory file. The export carries content, why, keywords, links and metadata, and an importing machine computes its own vectors from that content.",
     "Several hundred floats per memory would defeat the consistent export ordering chosen to keep merge diffs small, and the field-level merge driver would be reconciling vector noise with no meaningful merge semantics -- two branches re-embedding the same memory produce trivially different floats with no correct resolution.",
     "embeddings.md", []),

    ("model-choice-is-locally-reversible", "2026-08-09", "09:05", "joint", 0.95, ["embeddings", "model", "commitment"],
     "Because vectors never leave the machine, changing the embedding model invalidates only local state: one re-embed pass, affecting nobody else. That converts the choice of embedding model from a one-way ecosystem-wide commitment into a locally reversible decision, so a project does not have to agree with its contributors about which embedder they run.",
     "This is the important consequence of keeping vectors local, and it is the one that would be lost first if exporting them ever looked convenient.",
     "embeddings.md", [("RESULTS_FROM", "vectors-are-local-derived-state")]),

    ("embedding-cache-keyed-by-content-hash", "2026-08-09", "09:10", "joint", 0.85, ["embeddings", "cache", "import"],
     "A local embedding cache keyed by content hash means re-import only re-embeds memories whose content actually changed rather than the whole set. The model's identity and version are local state recorded alongside the cache, not in the exported file, which has no reason to know, and a mismatch between recorded and configured model triggers a re-embed rather than an error.",
     "Import cost is a real design constraint, since re-embedding on import sits on the critical path of an ordinary pull, which is what argues for a fast embedder.",
     "embeddings.md", [("RESULTS_FROM", "vectors-are-local-derived-state")]),

    ("one-store-one-model-no-mixture", "2026-08-09", "09:15", "joint", 0.95, ["embeddings", "model", "comparability"],
     "Every vector in a store is produced by that store's recorded model, or the memory has none. A store never holds vectors from two models at once, and changing the model resets the vector of every memory embedded with a different one.",
     "The rule that forces this is already stated for stores searched together, since one query is embedded once and scored against all of them. That reasoning does not weaken inside a single store -- it is the same defect at a smaller radius, and worse there, because the union check has at least somewhere to notice while within one store there is no seam to inspect.",
     "embeddings.md", [("DEPENDS_ON", "vectors-are-local-derived-state")]),

    ("mixture-produces-confusion-no-interface-clears", "2026-08-09", "09:20", "joint", 0.9, ["embeddings", "mixture", "silent-failure"],
     "The practical form of a mixed store is confusion no interface can clear up. Two memories come back ranked against each other with scores that were never comparable, and every downstream number -- the centroid, the ordering, any future floor -- is computed across a mixture. Nothing about the result looks wrong.",
     "A failure whose output is indistinguishable from a correct one cannot be caught by inspection, which is why the rule has to hold at write time rather than at read time.",
     "embeddings.md", [("SUPPORTS", "one-store-one-model-no-mixture")]),

    ("states-a-store-can-be-in", "2026-08-09", "09:25", "joint", 0.9, ["embeddings", "states", "model"],
     "The distinction that matters is not whether a store has vectors but whether this process is equipped to maintain what it already holds. Reads stay available in every state, because keyword search, direct lookup and traversal never needed a model and are not made wrong by one being absent. What varies is which writes are allowed.",
     "The states where a store holds vectors this process cannot compare against are not degraded configurations but broken ones, and the code had never distinguished them from the state where the store is consistently unembedded.",
     "embeddings.md", [("DEPENDS_ON", "one-store-one-model-no-mixture")]),

    ("no-recorded-model-is-two-states", "2026-08-09", "09:30", "joint", 0.95, ["embeddings", "migration", "provenance"],
     "No recorded model is two states, not one. A store that has genuinely never embedded anything and a store whose vectors predate the identity being written down are indistinguishable by the recorded model alone, and only the first has nothing to lose -- the second had 26 vectors while claiming exactly that. The vector count is what separates them.",
     "Every store written before the identity was recorded is in the second state, so it is a migration state rather than a corruption, and an unsafe one: a mismatch cannot be detected against a model nobody wrote down, which is how two servers came to run semantic recall against vectors from a third.",
     "embeddings.md", [("RESULTS_FROM", "states-a-store-can-be-in")]),

    ("rebuilt-rather-than-adopted", "2026-08-09", "09:35", "joint", 0.95, ["embeddings", "provenance", "rebuild"],
     "Vectors of unknown provenance are rebuilt rather than adopted. Adopting means writing the loaded model's identity over vectors it did not produce -- a guess made permanent and unverifiable, and one that leaves the store looking current afterwards. Rebuilding costs a re-embed, sub-second for any realistic store, and yields a provenance that is a fact.",
     "With no model in hand there is nothing to rebuild with, so the vectors stay and the existing protection applies: discarding them there would throw away the only copies in existence to settle a bookkeeping question.",
     "embeddings.md", [("RESULTS_FROM", "no-recorded-model-is-two-states")]),

    ("wrong-model-and-no-model-want-opposite-treatment", "2026-08-09", "09:40", "joint", 0.9, ["embeddings", "model", "states"],
     "A wrong model and no model are the same state and want opposite treatment, which is why they are two rows. With another model in hand there is something to rebuild with, so the store re-embeds itself and the mismatch is gone; refusing there would refuse the very thing the design specifies for a model change. With nothing in hand there is no way back, and the write has to be stopped instead.",
     "Collapsing them is the natural simplification and it produces either a store that cannot recover from an ordinary upgrade or one that destroys vectors it cannot replace.",
     "embeddings.md", [("SUPPORTS", "states-a-store-can-be-in")]),

    ("do-not-destroy-what-you-cannot-rebuild", "2026-08-09", "09:45", "joint", 0.95, ["embeddings", "writes", "rule"],
     "What is stopped is narrower than writes. A new memory in a store whose model is missing destroys nothing -- it arrives unembedded, says so, and is repaired by the gap sweep when a model returns. The write that cannot be allowed is one that replaces a memory that currently has a vector, because rewriting discards the vector and nothing here can produce a new one.",
     "Stating the rule as do not destroy what you cannot rebuild is what keeps a store usable while its model is missing instead of read-only.",
     "embeddings.md", [("RESULTS_FROM", "wrong-model-and-no-model-want-opposite-treatment")]),

    ("two-servers-re-embedding-loop", "2026-08-09", "10:00", "joint", 0.95, ["embeddings", "concurrency", "oscillation"],
     "The first real two-server run showed the model-mismatch case producing an unbounded loop: two servers with different models each re-embedded the whole store on the other's turn, indefinitely, and a single recall was enough to trigger it.",
     "The earlier reasoning was sound and rested on a premise that run falsified. Neither the LLM nor the human holds information that would justify leaving a store half-embedded is true of a single decider; with two servers, the other server's configuration is exactly that information, and neither server holds it.",
     "embeddings.md", [("RESULTS_FROM", "one-store-one-model-no-mixture")]),

    ("convergence-versus-oscillation", "2026-08-09", "10:05", "joint", 0.95, ["embeddings", "concurrency", "gap-filling"],
     "Gap-filling and mismatch re-embedding diverge in precisely the case the safety argument anticipated. Concurrent sweeps converging on identical rows -- duplicated work rather than a race -- holds for gaps, where both servers compute the same missing vectors, and fails for mismatches, where each server's rebuild invalidates the other's.",
     "Convergence versus oscillation is the difference, and it is not visible from either process. So gap-filling stays automatic because it converges, and mismatch re-embedding becomes explicit because it oscillates and the information needed to choose lives with the human who configured both servers.",
     "embeddings.md", [("RESULTS_FROM", "two-servers-re-embedding-loop")]),

    ("recall-refuses-the-semantic-path", "2026-08-09", "10:10", "joint", 0.95, ["embeddings", "recall", "refusal"],
     "On a model mismatch, reads keep keyword, id and traversal, and recall refuses the semantic path rather than rebuilding, because the measured alternative was a confidently-scored ranking computed across two incompatible embedding spaces, indistinguishable from a correct one.",
     "Its job is to withhold a number computed across two embedding spaces, which a warning cannot do -- the number would still be there, and something would still rank on it. That is why this stays a refusal where the re-embed does not.",
     "embeddings.md", [("RESULTS_FROM", "convergence-versus-oscillation")]),

    ("a-refusal-needs-a-way-out", "2026-08-09", "10:15", "joint", 0.9, ["embeddings", "refusal", "remedy"],
     "A refusal comes with a way out in three parts, all needed since any two leave the LLM stuck: status reports the model state honestly, naming recorded and loaded identity; the recall refusal names the remedy; and the sets tool gains a re-embed action that rebuilds every vector with the model in hand, states which model that is, and reports how many were rebuilt.",
     "Cost is not the objection, since a full rebuild of a ten-thousand-memory set is sub-second. What the explicit step buys is that a rebuild happens because someone chose it, and two servers then refuse rather than fight -- a visible standoff instead of an invisible loop.",
     "embeddings.md", [("DEPENDS_ON", "recall-refuses-the-semantic-path")]),

    ("explicit-action-moves-the-oscillation", "2026-08-09", "10:20", "human", 0.95, ["embeddings", "oscillation", "aggregate"],
     "An explicit action moves the oscillation rather than ending it. Once the remedy is documented the loop reappears one level up: one server refuses, its LLM runs the re-embed, the other refuses, its LLM runs the re-embed. Each is behaving correctly and locally, having met a documented failure and applied the documented fix, and nothing in either one's view separates recovering from a stale model from taking a turn in a tug-of-war.",
     "That is worse than the code-level loop in one specific way: each participant's local experience is success. The pathology exists only in aggregate, which is the case where the actor cannot be relied on to notice it.",
     "embeddings.md", [("EXCEPTION_TO", "a-refusal-needs-a-way-out")]),

    ("encode-the-signal-as-identity", "2026-08-09", "10:25", "joint", 0.9, ["embeddings", "cycle-detection", "signal"],
     "Encode the signal as identity, not as elapsed time. A timestamp asks the reader to infer contention from a duration -- a weak signal an LLM reasons about poorly, since the same four minutes is alarming in a tug-of-war and unremarkable otherwise. Instead the store keeps a bounded history of the model identities it has held, and the re-embed tests whether the requested model is one that was already displaced.",
     "That is a cycle, and cycle detection is exact -- the same shape as the supersession cycle handling, which is precedent rather than a new idea. The first re-embed is unremarkable; only re-entry is evidence, and it escalates with the count.",
     "embeddings.md", [("RESULTS_FROM", "explicit-action-moves-the-oscillation"), ("RELATED_TO", "supersession-cycles-both-places")]),

    ("it-warns-it-does-not-refuse", "2026-08-09", "10:30", "human", 0.95, ["embeddings", "warning", "lockout"],
     "The re-embed always proceeds, carrying the warning with it. The harms are asymmetric: what the check protects is derived state rebuilt in under a second, while a wrong refusal costs semantic recall itself. Blocking a capability to protect cheap rebuildable state is the wrong way round.",
     "The lockout is concrete rather than hypothetical. Decommission the second server and try to restore recall with the model that remains: the history says that model was already displaced, so a re-entry check refuses it, while the recorded model is one nothing on the machine can supply. A store must not be reachable into a state its own tool surface cannot repair.",
     "embeddings.md", [("SUPERSEDES", "encode-the-signal-as-identity")]),

    ("directive-rather-than-descriptive", "2026-08-09", "10:35", "joint", 0.9, ["embeddings", "messaging", "warning"],
     "What carries the weight is that the message is directive rather than descriptive. A third alternation between two models is a fact the reader may or may not act on; saying that two servers are configured with different models, that this will keep recurring until that is fixed, and to raise it with the human rather than re-running says what to do -- and is available in a warning exactly as much as in a refusal.",
     "This removes the override entirely, along with the question nobody could answer about it: an LLM meeting a refusal that names an override is fairly likely to simply use it. With nothing to refuse there is nothing to override.",
     "embeddings.md", [("SUPPORTS", "it-warns-it-does-not-refuse")]),

    ("gap-filling-is-automatic", "2026-08-09", "11:00", "joint", 0.9, ["embeddings", "gap-filling", "onboarding"],
     "Where a store has memories with no vector for its current model, they are embedded with no request required, because there is no decision here for anyone to make -- a choice whose answer is always yes, make them consistent should not be a choice.",
     "This is not a rare repair but the ordinary onboarding path: the model arrives through a delegated setup step, so storing memories first and gaining a model later is what a normal installation does, and without this those memories would be permanently unfindable by meaning.",
     "embeddings.md", [("INSTANCE_OF", "decision-two-checks")]),

    ("gap-detection-must-be-cheap", "2026-08-09", "11:05", "joint", 0.9, ["embeddings", "cache", "critical-path"],
     "The embedder must not land on the critical path of opening a store, which is why vectors are cached in memory at all -- so the gap has to be cheap to detect even when there is none. Keying the cache by input hash buys that: with each memory carrying the hash of its own embedded text, how many memories lack a vector for this model is one join, and the common answer of zero costs one query and no model.",
     "It also has to be idempotent, because two servers can share a store and both can notice the same gap. Keying by input hash and model makes concurrent sweeps converge on identical rows.",
     "embeddings.md", [("DEPENDS_ON", "gap-filling-is-automatic")]),

    ("why-not-export-drop-reimport", "2026-08-09", "11:10", "joint", 0.9, ["embeddings", "recovery", "local"],
     "Export, drop and re-import is the documented recovery for a project store whose every memory can be rebuilt from its file. It is the wrong instrument here and unavailable where it is needed most: the local store is the only copy of local memories, so rebuilding a cache by deleting it is destroying data to repair derived state.",
     "A round trip through the file also invalidates every reconciliation base, making each record read as divergent for a cycle -- considerable collateral for rebuilding something the design is at pains to keep from ever leaving the machine.",
     "embeddings.md", [("EXCEPTION_TO", "gap-filling-is-automatic")]),

    ("storage-partitioned-by-scope", "2026-08-09", "12:00", "joint", 0.95, ["storage", "partition", "correctness-by-construction"],
     "Each scope gets its own store -- a separate database or separate tables -- rather than all memories sharing one table with a scope column. A query against one project's store cannot return a local memory; with a scope column that guarantee is a clause someone has to remember, and a forgotten one leaks personal content into a project context.",
     "It is also the trust boundary one layer down, since project memories arrive over an untrusted channel and local ones do not, and separate stores make that structural rather than a column value. Per-set lifecycle becomes natural too, since every sync operation is already per set.",
     "embeddings.md", [("SUPPORTS", "scope-explicit")]),

    ("hubness-argument-listed-last-deliberately", "2026-08-09", "12:05", "joint", 0.9, ["partition", "argument-structure", "hubness"],
     "Hubness improving under partitioning is real but is deliberately listed last and is not load-bearing: if hubness later yields to some other fix, the first three reasons still carry the decision.",
     "A structural choice justified on its weakest argument gets undone when that argument evaporates, which is the failure this ordering exists to prevent.",
     "embeddings.md", [("SUPPORTS", "storage-partitioned-by-scope")]),

    ("union-centroid-for-stores-searched-together", "2026-08-09", "12:10", "joint", 0.95, ["centroid", "union", "recall"],
     "Recall usually wants a project set together with local, but with per-store centroid removal scores from two stores are measured against different baselines, so merging them by score is unsound. Every store in a query is centred on the mean of their union, which makes the searched stores one corpus mathematically -- which is what the caller actually has.",
     "The property that decides it, and the one the implementation is tested against: a corpus split across two stores answers a query exactly as one store holding all of it would. Partitioning exists for correctness and trust rather than for ranking, so it must not change what recall returns.",
     "embeddings.md", [("RESULTS_FROM", "storage-partitioned-by-scope")]),

    ("interleaving-and-raw-cosine-both-fail", "2026-08-09", "12:15", "joint", 0.9, ["centroid", "ranking", "alternatives"],
     "Interleaving by rank cannot satisfy the property: it imposes a quota the single-store case does not have, so a query whose answer lies entirely in one set still spends slots on the other, and it cannot express one memory being far more relevant than another. Raw cosine across stores cannot either, since it discards the centring the single-store case applies, making the merged ordering the one measured as worse.",
     "The union centroid is also the cheapest of the three, since every store already keeps a running componentwise sum, so the union centroid is a few additions and one division with no second pass over any vector. The cost that motivated listing this as an open question turned out not to exist.",
     "embeddings.md", [("SUPPORTS", "union-centroid-for-stores-searched-together")]),

    ("recalled-memory-reports-its-store", "2026-08-09", "12:20", "joint", 0.9, ["recall", "provenance", "trust"],
     "A recalled memory reports which store it came from. Once several stores answer one query, a local preference and an imported project decision arrive in the same list, and the trust difference between them is exactly what the partition exists to preserve.",
     "A result list that erases provenance would undo at the presentation layer the separation the storage layer was built to guarantee.",
     "embeddings.md", [("RESULTS_FROM", "union-centroid-for-stores-searched-together")]),

    ("unsearchable-store-makes-the-result-incomplete", "2026-08-09", "12:25", "joint", 0.9, ["recall", "completeness", "reporting"],
     "If any store in the union lacks a model, recall says so rather than quietly answering from the rest -- the same reason a single store says so.",
     "Partial results that do not announce themselves are the failure the loud-degradation rule exists to prevent, and the union case is where it would be easiest to let slip, since the answer still looks complete.",
     "embeddings.md", [("SUPPORTS", "union-centroid-for-stores-searched-together")]),

    ("no-network-at-a-moment-not-chosen", "2026-08-09", "13:00", "joint", 0.95, ["offline", "model", "network"],
     "The rule that decides how the model gets onto the machine is not no network but no network at a moment the user did not choose. Fetching during installation is fine -- the developer is already online, and not fetching means every user must first learn what an embedding model is in order to use a memory tool.",
     "What is unacceptable is the same fetch arriving later: a developer who installs, uses it for a week, gets on a plane, opens a project and hits a network requirement they were never told about. That is much worse than the friction avoided by deferring, because it lands when nothing can be done about it.",
     "embeddings.md", [("DEPENDS_ON", "offline-capable")]),

    ("local-only-is-a-build-feature", "2026-08-09", "13:05", "joint", 0.9, ["offline", "build-feature", "structural"],
     "The local-only build feature stays a build feature, and that is what makes the guarantee structural. With remote fetching compiled out, a runtime fetch is not something Scatterhoard chooses not to do -- it is something it cannot do. The offline promise then does not depend on nobody adding a convenient lazy download later.",
     "It is the same spirit as the path-based trust rule: a property enforced by construction rather than by everyone remembering it.",
     "embeddings.md", [("SUPPORTS", "no-network-at-a-moment-not-chosen")]),

    ("model-acquisition-is-never-lazy", "2026-08-09", "13:10", "joint", 0.9, ["offline", "model", "laziness"],
     "Model acquisition is an explicit step, never lazy: no fetch on first recall, on first use of a new project, or on encountering an unfamiliar model version. Anything that could fire at an arbitrary later moment is the plane scenario wearing a different hat.",
     "Enumerating the tempting lazy triggers is what keeps the rule from being satisfied in letter by a fetch nobody classified as lazy.",
     "embeddings.md", [("SUPPORTS", "no-network-at-a-moment-not-chosen")]),

    ("missing-model-degrades-loudly", "2026-08-09", "13:15", "joint", 0.95, ["offline", "degradation", "recall"],
     "A missing model degrades loudly, never silently. Semantic search is one of three recall paths, and keyword search, direct-id lookup and traversal all still work without a model, so Scatterhoard can run without one -- but recall must say that semantic search is unavailable.",
     "A quiet degradation is precisely the failure the recall convention warns about: memory that is not really being consulted, while the session believes it is.",
     "embeddings.md", [("RESULTS_FROM", "no-network-at-a-moment-not-chosen")]),

    ("setup-is-a-bounded-procedure-for-the-llm", "2026-08-09", "13:20", "joint", 0.9, ["setup", "howto", "delegation"],
     "Since a developer using Scatterhoard is by definition working with an LLM, setup can be delegated to it: a HOWTO shipped with Scatterhoard itself, aimed at the model rather than the human. It must fit in one session, because a setup procedure that runs out of context halfway leaves a half-configured machine, which is worse than one that never started -- so bounded steps, no open-ended exploration, and a way to check the result.",
     "The error is the entry point: when the model is missing the message names the fix directly, so the common case never requires finding the HOWTO at all.",
     "embeddings.md", [("RESULTS_FROM", "missing-model-degrades-loudly")]),

    ("shipped-howto-is-on-the-trusted-side", "2026-08-09", "13:25", "joint", 0.9, ["setup", "trust", "instructions"],
     "A shipped HOWTO does not conflict with the rule that nothing instruction-shaped is auto-loaded from a project. That rule is about content arriving over an untrusted channel -- a repository someone else wrote. A HOWTO shipped with the tool the user chose to install is on the trusted side of exactly the same path-based boundary.",
     "Checking a new instruction-shaped artifact against the rule that forbids them, and finding the rule scoped by channel rather than by shape, is what keeps the rule usable rather than absolute.",
     "embeddings.md", [("EXCEPTION_TO", "nothing-instruction-shaped-auto-loaded")]),

    ("brute-force-index-first", "2026-08-09", "14:00", "joint", 0.9, ["index", "scale", "simplicity"],
     "At the scale a project memory set plausibly reaches, an approximate-nearest-neighbour index is not needed: ten thousand memories at 384 dimensions is roughly 15 MB of floats, and a linear cosine scan is single-digit milliseconds. So vectors are stored as blobs and scanned linearly -- no index parameters to tune, no index to rebuild on import, no index file to corrupt or reconcile after a merge.",
     "If a set ever grows past roughly 100,000 memories an index can be added behind the same recall interface without changing anything above it, and a dependency-free C option is the natural upgrade path worth keeping in mind when shaping the storage interface.",
     "embeddings.md", [("DEPENDS_ON", "filesystem-backed-not-in-memory")]),

    ("embedder-candidate-ladder", "2026-08-09", "14:05", "joint", 0.85, ["embedder", "candidates", "constraints"],
     "The embedder constraints are self-contained, no network at run time, filesystem-backed, permissively licensed, and Rust-native or cleanly bindable. The ladder runs from a pure-Rust static-embedding crate at 8 to 32 MB, through a pure-Rust transformer at 90 to 130 MB and substantially slower on CPU, to an ONNX-backed option whose native dependency complicates single-binary installation.",
     "Recording the ladder rather than only the choice is what makes the decision revisitable, since each rung names the constraint it spends.",
     "embeddings.md", [("DEPENDS_ON", "self-contained-no-external-components")]),

    ("semantic-search-only-produces-seeds", "2026-08-09", "14:10", "joint", 0.9, ["embedder", "seeds", "tradeoff"],
     "Two reasons the quality trade is more favourable than the raw benchmark gap suggests. Semantic search only produces seeds -- traversal proceeds from there, and keyword search and direct-id lookup are independent paths to the same memories -- so the embedder has to surface plausible entry points rather than be the entire retrieval system. And speed is on the import path, since every import re-embeds whatever changed, so sub-second versus roughly a minute is the difference between an invisible step and one a user notices on every pull.",
     "Both arguments turn on how the embedder is used rather than on how good it is, which is the analysis a benchmark cannot supply.",
     "embeddings.md", [("SUPPORTS", "embedder-candidate-ladder"), ("RELATED_TO", "semantic-similarity-is-primary")]),

    ("static-model-matches-transformer-here", "2026-08-09", "15:00", "joint", 0.95, ["measurement", "embedder", "spike"],
     "Measured over a 45-memory labelled corpus with 20 queries, embedding content, why and keywords: the static model reached 71.7% raw and 77.5% centred at five results, against a transformer's 74.2% and 74.2%. The published benchmark gap does not appear on this task -- with centroid removal the static model is ahead at five and within a point at ten.",
     "The ceiling here is the task and the corpus rather than the model, so there is no meaningful quality gain available by paying the size, speed and dependency cost of a transformer. Throughput of roughly 18,000 memories per second resolves the import-cost concern too.",
     "embeddings.md", [("SUPPORTS", "embedder-candidate-ladder")]),

    ("subtract-the-corpus-centroid", "2026-08-09", "15:05", "joint", 0.95, ["centroid", "measurement", "ranking"],
     "Removing the mean of the memory vectors from both memory and query vectors lifted recall from 71.7% to 77.5% at five results and 83.3% to 87.5% at ten. It costs one vector subtraction and fits the design already chosen, since the centroid is local derived state like the vectors themselves and the brute-force scan absorbs the subtraction at negligible cost.",
     "Store raw vectors and subtract at scan time rather than storing centred ones, so adding a memory does not invalidate everything already stored.",
     "embeddings.md", [("RESULTS_FROM", "static-model-matches-transformer-here")]),

    ("hubness-is-not-a-static-artifact", "2026-08-09", "15:10", "joint", 0.9, ["hubness", "measurement", "correction"],
     "One memory dense in common domain vocabulary appeared in the top five for 13 of 20 queries. The initial reading was that this is a static-embedding failure from lack of contextualisation, but the transformer shows the same effect with its worst hub at 8 of 20. Static embeddings are worse, not uniquely afflicted -- hubness is a general property of high-dimensional spaces.",
     "Centroid removal largely fixes it, dropping the worst hub from 13 of 20 to 6 of 20 while also improving recall, which is why it is adopted.",
     "embeddings.md", [("SUPPORTS", "subtract-the-corpus-centroid")]),

    ("csls-not-adopted", "2026-08-09", "15:15", "joint", 0.85, ["hubness", "csls", "measurement"],
     "Local scaling that penalises each memory by its mean similarity to its own nearest neighbours is worse than centroid removal on its own, and combining them buys little -- under a point at five results and one hub, paid for with a 7.5-point regression at three. Not adopted for now.",
     "The result is also the least trustworthy in the spike, since a neighbourhood of ten against a 45-memory corpus computes the penalty over roughly a fifth of the whole set, a completely different regime from the same neighbourhood against several thousand memories. Worth re-testing at realistic scale before being ruled in or out.",
     "embeddings.md", [("EXCEPTION_TO", "subtract-the-corpus-centroid")]),

    ("hub-seed-costs-more-than-recall-shows", "2026-08-09", "15:20", "joint", 0.9, ["hubness", "seeds", "traversal"],
     "Hubness matters beyond the aggregate numbers: a hub seed both consumes a seed slot and sends graph traversal into the wrong neighbourhood, so its cost is higher than a recall metric alone reflects.",
     "A metric that scores the returned set cannot see what a bad seed did to the walk, which is why the aggregate improvement understates the value of fixing hubness.",
     "embeddings.md", [("SUPPORTS", "hubness-is-not-a-static-artifact"), ("RELATED_TO", "no-dont-seed-from-this-control")]),

    ("limitations-of-the-spike", "2026-08-09", "15:25", "joint", 0.9, ["measurement", "limitations", "corpus"],
     "One small corpus, written and labelled by the same author, on a single domain. The absolute recall numbers should not be trusted far -- some queries have three relevant memories, and the labels are judgement calls. The comparison is sound regardless, because both models were scored against identical labels, so systematic label bias affects them equally.",
     "Separating what a measurement supports from what it does not is what lets the comparison be relied on while the absolute numbers are not, and it is the discipline that keeps a spike from being over-read.",
     "embeddings.md", [("EXCEPTION_TO", "static-model-matches-transformer-here")]),

    # ---- tool-surface.md ------------------------------------------------------------------
    ("keep-the-tool-surface-small", "2026-08-15", "09:00", "human", 0.95, ["tool-surface", "attention", "naming"],
     "Keep the tool surface fairly simple, clear and ergonomic for the LLM. We do not know how many other MCP servers the user will load, and too many tools can be confusing. Name the tools clearly with a consistent prefix, and favour active voice and a consistent parameter order.",
     "Every tool added is a standing charge against the model's attention, competing with whatever other servers the user has loaded -- a cost paid in every session whether or not the tool is used.",
     "tool-surface.md", []),

    ("the-six-tools", "2026-08-15", "09:05", "joint", 0.9, ["tool-surface", "tools", "ordering"],
     "The tools are conventions, memorize, recall, lookup, correct and sets: the vision's store and recall, the convention loading required before anything else happens, and the tools that manage what has already been stored. Nothing else until it earns a place. They are listed in the order a session uses them rather than alphabetically.",
     "A heading over this table read three tools while the table listed four, and kept reading that way as the surface grew to six. A count in a heading is a hand-maintained copy of the length of the table beneath it, in the smallest possible form.",
     "tool-surface.md", [("RESULTS_FROM", "keep-the-tool-surface-small")]),

    ("occasional-operations-group-behind-an-action", "2026-08-15", "09:10", "joint", 0.9, ["tool-surface", "actions", "frequency"],
     "Correction and set management each take an action and dispatch on it, where the four constantly-used tools are flat. The frequency difference is real and large: conventions load once a session and memorize and recall run throughout, while lifecycle and correction operations are reached for a handful of times in a project's life. Keeping the constantly-used surface flat is worth more than keeping the rarely-used surface flat.",
     "Ten tools would have been the alternative -- six lifecycle operations plus two corrections against two tools with an action each -- and the attention cost is paid on every tool in every session.",
     "tool-surface.md", [("RESULTS_FROM", "keep-the-tool-surface-small")]),

    ("action-enum-carried-its-own-weight", "2026-08-15", "09:15", "joint", 0.85, ["tool-surface", "schema", "actions"],
     "The action enum turned out to carry its own weight, which was not anticipated: a branch per action lets each action publish only its own arguments, so an impossible combination is not something the schema invites -- where a flat tool with every argument optional is exactly the shape that produced an underspecified schema. An unknown action can also name the available ones, which a wrong tool name cannot.",
     "A structure adopted for one reason turning out to serve a second is worth recording, because the second reason survives if the first is ever revisited.",
     "tool-surface.md", [("SUPPORTS", "occasional-operations-group-behind-an-action")]),

    ("branches-inform-they-do-not-constrain", "2026-08-15", "09:20", "joint", 0.9, ["tool-surface", "schema", "correction"],
     "The branches were never enforcement. JSON Schema permits properties a branch does not list unless the branch forbids them, which these do not, so a caller could always have sent an impossible combination. What the branches buy is that nothing suggests it -- real and worth having, but communicative rather than structural.",
     "The claim that such a call could not be constructed and believed stood until a client proved otherwise, and the overstatement mattered in two directions.",
     "tool-surface.md", [("SUPERSEDES", "action-enum-carried-its-own-weight")]),

    ("a-consumer-may-not-read-the-branches", "2026-08-15", "09:25", "joint", 0.95, ["tool-surface", "schema", "clients"],
     "A consumer may not read the branches at all. Published as an alternation with no top-level properties, two tools reached one client as tools taking no arguments -- every argument stripped, every call refused, the whole management surface unreachable -- and another as a merge of the branches with the action pinned to the first, able to express only one of them. Same publication, both entitled to read it that way.",
     "So a flat top-level view is published, derived from the branches rather than written twice, so an action added to one appears in the other. Restating the combinations in words costs nothing that was ever enforced.",
     "tool-surface.md", [("RESULTS_FROM", "branches-inform-they-do-not-constrain")]),

    ("degrade-into-usable-not-empty", "2026-08-15", "09:30", "joint", 0.95, ["tool-surface", "publication", "general-rule"],
     "A publication read by software you do not control has to degrade into something usable, not into something empty. The precise statement can be the best answer available; it cannot be the only one.",
     "The general form is worth keeping because it is not about JSON Schema at all -- it applies wherever a producer states something precisely to a consumer whose handling it cannot observe.",
     "tool-surface.md", [("RESULTS_FROM", "a-consumer-may-not-read-the-branches")]),

    ("top-level-alternation-is-refused-outright", "2026-08-15", "09:35", "joint", 0.95, ["tool-surface", "schema", "api"],
     "Keeping the branches and adding a flat view beneath them was still too generous to the alternation. The API rejects the keyword outright at the top level of a tool's schema, and publishing properties alongside does not help -- a reported case published both and was refused just the same. The keyword's presence at the top level is the violation.",
     "The blast radius is other people's servers: a client that forwards schemas unmodified re-sends the whole tool list on every request, so one offending schema fails every tool from every registered server for the rest of the session.",
     "tool-surface.md", [("SUPERSEDES", "a-consumer-may-not-read-the-branches")]),

    ("surviving-on-a-courtesy", "2026-08-15", "09:40", "joint", 0.9, ["tool-surface", "dependency", "method"],
     "Scatterhoard's tools worked only because something between the server and the API happened to normalise the alternation away. That is precisely the dependency the previous correction was written to warn about, left in place by the fix that named it.",
     "Worth recording that the correction was written to be more careful than the thing it corrected and still under-stated the constraint: it said consumers vary in whether they honour the branches, where the truth is that the format's largest consumer forbids them. Being more defensive is not the same as checking.",
     "tool-surface.md", [("RESULTS_FROM", "top-level-alternation-is-refused-outright")]),

    ("two-clients-disagreed-about-one-schema", "2026-08-15", "10:00", "joint", 0.95, ["tool-surface", "measurement", "clients"],
     "Two clients, the same binary, the same session, and every difference is in how each treats the parts of JSON Schema that express alternatives. Scalar types, properties, required, items, enum and description survive both. Top-level alternation is merged by one and dropped by the other; nested alternation and a nullable union type are preserved by one and reduced to nothing by the other.",
     "The consequence on the reduced side is not cosmetic: a property published as nothing carries no type, so an object argument is serialised as text and arrives as a string. On that client no shared set could be named at all.",
     "tool-surface.md", [("RESULTS_FROM", "surviving-on-a-courtesy")]),

    ("publish-the-centre-accept-the-edges", "2026-08-15", "10:05", "joint", 0.95, ["tool-surface", "schema", "rule"],
     "Publish only what survives; accept every spelling a client might produce. Publishing stays inside the subset both clients preserved intact -- a single scalar type, properties, required, items, enum, description -- with no alternation anywhere, neither at the top level where it is refused outright nor nested where it is silently emptied. Accepting is deliberately wider than publishing.",
     "This is the rule the individual corrections were instances of, arrived at by surveying the whole surface rather than the parts that happened to break.",
     "tool-surface.md", [("RESULTS_FROM", "two-clients-disagreed-about-one-schema"), ("SUPERSEDES", "top-level-alternation-is-refused-outright")]),

    ("strict-where-strictness-is-a-signal", "2026-08-15", "10:10", "joint", 0.95, ["tool-surface", "strictness", "influence"],
     "A server is normally right to be strict, because a strict server is what stops clients drifting into private dialects. That argument assumes influence, and this server has none: refusing an object-valued scope would not teach anyone anything, it would only mean the memories are unreachable from that client. Be strict where strictness is a signal, and liberal where it is only a refusal.",
     "Where the strictness cannot land on anyone who could act on it, all it does is convert someone else's transformation into your failure. The cost is real -- two accepted spellings for one concept can drift -- and is paid for with a round-trip test over both rather than with a comment.",
     "tool-surface.md", [("SUPPORTS", "publish-the-centre-accept-the-edges")]),

    ("scope-publishes-as-a-string", "2026-08-15", "10:15", "joint", 0.9, ["tool-surface", "scope", "encoding"],
     "Scope publishes as a string, either local or shared with a name after a colon. This is not a new encoding: the store's metadata row has used exactly that spelling since the beginning and it round-trips in both directions already. What changes is that an encoding the project already relies on internally is promoted to the tool boundary.",
     "It is unambiguous by construction, because the set-name rule permits letters, digits, hyphen, underscore and dot, so a set name can never contain the delimiter.",
     "tool-surface.md", [("RESULTS_FROM", "publish-the-centre-accept-the-edges"), ("DEPENDS_ON", "set-name-shape")]),

    ("link-target-splits-into-siblings", "2026-08-15", "10:20", "joint", 0.9, ["tool-surface", "links", "encoding"],
     "A link target cannot become a string without losing something, so it does not. The object form carries a set, an id and keywords, and those keywords exist so a reference that goes dangling has something to be re-found by -- a combined spelling would drop exactly the field that makes a dangling cross-set link recoverable. Instead the alternation moves out into sibling scalars: the target is always an id, with an optional target set and target keywords beside it.",
     "Objects with named properties survive both clients; only the alternation inside them does not. Nothing is lost and nothing needs a second spelling.",
     "tool-surface.md", [("RESULTS_FROM", "publish-the-centre-accept-the-edges")]),

    ("an-invalid-combination-becomes-constructible", "2026-08-15", "10:25", "joint", 0.85, ["tool-surface", "schema", "tradeoff"],
     "The sibling shape does make an invalid combination constructible -- a target set with no target. That is the objection an earlier issue raised, and it is accepted here for a reason that did not apply then: the alternative is not a more precise schema but an emptier one, since the client that would have enforced the distinction is the client that discards it.",
     "A combination the schema no longer forbids is refused by the parser and named in the response, which moves the guarantee rather than dropping it.",
     "tool-surface.md", [("EXCEPTION_TO", "link-target-splits-into-siblings")]),

    ("removing-alternations-retires-three-keywords", "2026-08-15", "10:30", "joint", 0.8, ["tool-surface", "schema", "simplification"],
     "Two keywords exist only inside the alternation branches and a third only on one object branch, so removing the alternations removes all three without a separate decision. An integer type survives one client and is rewritten to a number by the other, which costs nothing and is left alone -- a note rather than a change, so the next person to see it does not read it as an oversight.",
     "Recording the deliberate non-change is what stops a later reader treating it as something missed.",
     "tool-surface.md", [("RESULTS_FROM", "publish-the-centre-accept-the-edges")]),

    ("legality-versus-carriage", "2026-08-15", "10:35", "joint", 0.95, ["tool-surface", "measurement", "general-rule"],
     "The question is not whether a construct is legal, it is whether it is reliably carried. Legality is decidable from a specification; carriage is only decidable by measurement, and the measurement has to be repeated when the surface changes.",
     "This is the same degradation rule applied before the fact rather than after, and a test plus the survey it grew out of are that check made permanent.",
     "tool-surface.md", [("SUPPORTS", "degrade-into-usable-not-empty"), ("SUPPORTS", "publish-the-centre-accept-the-edges")]),

    ("correction-is-its-own-tool", "2026-08-15", "11:00", "joint", 0.9, ["tool-surface", "correction", "grouping"],
     "Correction is separate from set management because it acts on one memory, where every set action acts on a whole set. Grouping them by things you do occasionally would have put two unrelated scopes of effect behind one name.",
     "The third correction operation, supersession, is not there at all -- it is a link written at store time through memorize, because superseding creates a memory where amending and forgetting change or remove one.",
     "tool-surface.md", [("EXCEPTION_TO", "occasional-operations-group-behind-an-action")]),

    ("lookup-is-a-tool-keyword-is-a-parameter", "2026-08-15", "11:05", "joint", 0.9, ["tool-surface", "lookup", "recall"],
     "Keyword search is search -- it produces ranked results and resolves validity exactly as semantic recall does -- so it belongs on the same tool as a second way of asking the same question. Direct reference is retrieval, and it behaves differently in the one way that matters: it is exempt from supersession substitution.",
     "Making it a parameter would give one tool two contradictory behaviours depending on which argument was supplied, with a superseded memory swapped for its replacement or returned as asked, decided by a field. That is a worse cost than one extra tool.",
     "tool-surface.md", [("DEPENDS_ON", "direct-lookup-exempt-from-substitution")]),

    ("one-search-per-call", "2026-08-15", "11:10", "joint", 0.95, ["tool-surface", "recall", "scales"],
     "Recall takes either a query or keywords and refuses a call carrying both, returning results in a single list. A match count and a cosine score are different scales, and blending them into one ordering would be the same mistake that made cross-store scores incomparable.",
     "The original answer was to run both and return two lists. That answer was wrong, and using it is what showed why.",
     "tool-surface.md", [("RELATED_TO", "union-centroid-for-stores-searched-together")]),

    ("stapling-two-searches-integrated-nothing", "2026-08-15", "11:15", "joint", 0.95, ["tool-surface", "recall", "correction"],
     "The combined call ran the two paths independently and stapled the results together. It integrated nothing, so it produced nothing a caller could not get from two calls, while creating three problems at once: a memory found both ways came back twice in full, measured at roughly 1,100 duplicated bytes carrying about 30 bytes of new information; the response held two scales with no stated relationship; and any attempt to order the combined result had to either invent a blended ranking or exile keyword-only hits to the bottom.",
     "Refusing the combination makes all three unrepresentable rather than fixed, which is the correctness-by-construction posture the rest of the design takes.",
     "tool-surface.md", [("RESULTS_FROM", "one-search-per-call")]),

    ("what-refusing-the-combination-costs", "2026-08-15", "11:20", "joint", 0.85, ["tool-surface", "recall", "cost"],
     "The cost is one extra call when a caller genuinely wants both, and the loss of one real signal: a memory found by both paths is stronger evidence of relevance than one found by either, and splitting hides it. That is recorded as the reason to revisit under hybrid search.",
     "Naming the signal that the refusal discards is what keeps the open question honest rather than letting the decision look free.",
     "tool-surface.md", [("EXCEPTION_TO", "stapling-two-searches-integrated-nothing")]),

    ("keyword-search-is-not-redundant", "2026-08-15", "11:25", "joint", 0.95, ["keyword-search", "measurement", "embedding"],
     "Keyword search is not redundant with semantic search, and this was measured rather than assumed. Keywords do reach the embedding, since the embedded text is built from content, then why, then keywords -- but five keyword tokens averaged into a paragraph-length vector barely move it, and a static embedder represents a rare identifier poorly. Queried by meaning for a term carried only as a keyword, the memory holding it ranked second, beneath an unrelated memory.",
     "Exact match does work the embedding cannot, which is why both paths exist and why the answer was to separate them rather than to drop one.",
     "tool-surface.md", [("SUPPORTS", "one-search-per-call"), ("RELATED_TO", "inherited-score-must-survive")]),

    ("transport-is-not-part-of-the-surface", "2026-08-15", "12:00", "joint", 0.9, ["tool-surface", "transport", "testability"],
     "The server is a pure function from a JSON-RPC request to a response, with the transport handled separately in the binary. The tools are then testable without an MCP client or an async runtime, so the parts with decisions in them are exercised directly, and adopting an SDK or a different transport becomes a change to the glue rather than a rewrite of the decisions.",
     "Both consequences matter more than the small amount of glue it costs, which is the calculation that makes the separation worth stating rather than discovering.",
     "tool-surface.md", []),

    ("arguments-validated-by-being-parsed", "2026-08-15", "12:05", "joint", 0.95, ["tool-surface", "parsing", "types"],
     "Tool arguments deserialise straight into the memory model's types, whose constructors already validate. An out-of-range confidence or a malformed set name is refused because the type refuses it, not because the tool layer checks -- parse-don't-validate reaching the outermost boundary.",
     "The important case is scope: because the scope type has no default, a call omitting it cannot be deserialised at all, so the explicit-never-inferred rule reaches the tool boundary with no check anyone could forget. Confirmed by mutation -- making the parameter optional does not fail a test, it fails to compile, because there is no function that guesses a scope.",
     "tool-surface.md", [("DEPENDS_ON", "scope-no-default")]),

    ("a-failing-tool-answers", "2026-08-15", "12:10", "joint", 0.95, ["tool-surface", "errors", "llm"],
     "A tool that cannot do what was asked returns a result marked as an error rather than a protocol error. The model should see the failure as an answer it can act on -- a wrong set name, a missing model -- rather than as a broken call it can only retry.",
     "The distinction decides whether the LLM's next move is a repair or a retry, which is the difference between a session that recovers and one that loops.",
     "tool-surface.md", []),

    ("a-refusal-must-name-an-action", "2026-08-15", "12:15", "joint", 0.95, ["tool-surface", "errors", "actionability"],
     "Naming a set this machine does not hold is an error rather than an invitation to create one, since inventing a store on a typo would silently split a set in two. But the refusal has to name an action the caller can take, which it did not: it said to check the set name with no way to list names, and that sets have to be imported with no import tool -- two dead ends in one sentence.",
     "A refusal that is correct and unactionable is only half a refusal, which is the shape a correctness-focused error message reaches naturally and has to be pushed past.",
     "tool-surface.md", [("SUPPORTS", "a-failing-tool-answers")]),

    ("missing-model-does-not-fail-the-call", "2026-08-15", "12:20", "joint", 0.9, ["tool-surface", "errors", "degradation"],
     "A missing embedding model does not fail the call. Recall returns what it can and says, in words the model will read, that the store could not be searched by meaning and that memory should be treated as unconsulted rather than empty.",
     "Treating an unsearchable store as empty is the reassuring failure the loud-degradation rule exists to prevent, and the tool layer is where that rule has to be honoured.",
     "tool-surface.md", [("INSTANCE_OF", "missing-model-degrades-loudly")]),

    ("scope-required-to-store-optional-to-recall", "2026-08-15", "12:25", "joint", 0.95, ["tool-surface", "scope", "asymmetry"],
     "Memorize requires a scope; recall defaults to every loaded set, and the asymmetry is the point. Storing into the wrong set is a one-way door, since a personal memory committed to a shared repository cannot be taken back, so the choice is required and never guessed. Searching the wrong set is a read: it costs nothing and returns less.",
     "A project set together with local is the normal case, since a stated preference matters while working on a project, so the default is what a caller almost always wants.",
     "tool-surface.md", [("INSTANCE_OF", "decision-placement"), ("DEPENDS_ON", "scope-explicit")]),

    ("union-default-needed-comparable-scores", "2026-08-15", "12:30", "joint", 0.9, ["tool-surface", "recall", "centroid"],
     "The recall default became available only once cross-store scores were made comparable by centring every searched store on the mean of their union. Before that, an implicit union would have had a ranking nobody could defend.",
     "Every result reports which set it came from, so a local preference and an imported project decision never arrive indistinguishable -- which is what makes searching several sets at once safe rather than merely convenient.",
     "tool-surface.md", [("DEPENDS_ON", "union-centroid-for-stores-searched-together"), ("SUPPORTS", "scope-required-to-store-optional-to-recall")]),

    # ---- conventions.md -------------------------------------------------------------------
    ("conventions-are-hard-coded-memories", "2026-08-03", "09:01", "human", 0.9, ["conventions", "context", "loading"],
     "Conventions act like memories but are hard-coded, and the LLM loads them into any session that uses Scatterhoard if it has not already. They consume context, and some clients dump to a file if a response is too large, so they must be kept compact with a cursoring mechanism in the tool that fetches them, and care taken not to interfere with locally defined prompts and rules.",
     "Treating conventions as data with a context cost rather than as prose is what makes the budget below a real constraint rather than an aspiration.",
     "conventions.md", [("DEPENDS_ON", "what-scatterhoard-is")]),

    ("reinforcement-versus-update-time-context", "2026-08-03", "09:06", "joint", 0.95, ["conventions", "reinforcement", "comments"],
     "Each convention carries two kinds of supporting text and they are not the same. Reinforcement is served alongside the convention every time it is used, reinforcing it in the moment and clarifying when it applies, and it lives in the data. Update-time context is the background someone needs when changing the convention, and it lives in source comments, which are never served.",
     "The test that sorts them: does this text reinforce the convention at the moment of use? If yes it is reinforcement; if it only helps someone deciding whether to revise the convention, it is a comment.",
     "conventions.md", [("SUPPORTS", "conventions-are-hard-coded-memories")]),

    ("served-text-must-not-assert-facts-about-the-reader", "2026-08-13", "09:10", "joint", 0.95, ["conventions", "reinforcement", "reader"],
     "A second test, added after two conventions had already failed it. Both cited what this store holds and what this project's store grew -- true of the store they were drafted against and false for every other user, who reads this store as a claim about their own. Reinforcement is read by strangers, and a convention caught being wrong about the reader loses authority on everything else it says.",
     "The fix is not to strip the evidence, since concreteness is what makes the rules land, but to illustrate rather than assert -- and the measurement itself belongs in the source comment, which is never served and where a claim about this project's store is simply true. The second failure existed because the first was followed as precedent, which is how a one-off becomes a pattern.",
     "conventions.md", [("SUPPORTS", "reinforcement-versus-update-time-context")]),

    ("why-it-is-not-called-why", "2026-08-03", "09:15", "joint", 0.9, ["conventions", "naming", "standards"],
     "A memory also has a why, and the two are similar in that both clarify when the thing they are attached to applies, but they are governed differently. A convention's reinforcement is held to a fixed bar, because conventions are hard-coded, reviewed and served into every session. A memory's why is not, since the LLM drafts it in the moment. The field is named reinforcement, which states the standard it has to meet.",
     "The Golden Rule reaches runtime through this field, which is why it must be served data rather than a comment -- a comment would mean that decision silently never takes effect.",
     "conventions.md", [("RESULTS_FROM", "reinforcement-versus-update-time-context"), ("DEPENDS_ON", "principle-reaches-runtime-through-why")]),

    ("route-where-asks-what-it-is-about", "2026-08-12", "10:00", "joint", 0.95, ["conventions", "routing", "scope"],
     "The routing convention asks what the memory is about, not what you were doing. That distinction is what the data demanded: you are working on the project either way, so what am I doing routes everything to the project set, and the general lessons that happen to be learned there get filed with it.",
     "The gap was real and measured -- of 26 memories in the personal set, roughly eighteen were project knowledge, six general engineering lessons, one a genuine working preference and one a leftover test fixture, with four having twins already in the project set.",
     "conventions.md", [("RESULTS_FROM", "scope-explicit")]),

    ("taxonomy-belongs-in-the-sets", "2026-08-12", "10:05", "human", 0.95, ["conventions", "routing", "derivation"],
     "A first draft named three destinations -- project, cross-project, personal -- and that was wrong, caught before it shipped. It compiled one user's arrangement into everyone's catalog: a user with a single set, or one per client, or a team set beside a personal one, would get a rule describing somebody else's setup with no way to correct it short of a rebuild. So each set says what it is for, in a memory carrying a reserved keyword, and routing is a match against those.",
     "This is the move the design makes everywhere else -- derive rather than keep a second copy -- applied to the one fact the catalog had no business holding. The sets are the authority on what they contain, and it needs no new machinery, since a keyword search with no scope already searches every loaded set and tags each hit.",
     "conventions.md", [("RESULTS_FROM", "route-where-asks-what-it-is-about")]),

    ("a-memory-travels-a-meta-row-does-not", "2026-08-12", "10:10", "joint", 0.9, ["conventions", "routing", "export"],
     "Putting a set's description in a memory means it moves through export and import, so whoever clones a repository and imports its set learns what belongs there. Anywhere else and every machine has to be told separately, and a collaborator is told nothing at all. It also means a change to a set's charter is reviewed like any other memory, through the same export gate.",
     "Choosing the carrier by what travels with it is the same reasoning that keeps the set name in the header and the store's author out of it.",
     "conventions.md", [("SUPPORTS", "taxonomy-belongs-in-the-sets")]),

    ("descriptions-are-hints-two-rules-override", "2026-08-12", "10:15", "joint", 0.95, ["conventions", "routing", "security"],
     "A description that steers routing is instruction-shaped and arrives from a repository like any other memory, which is the channel the data-never-instructions rule closes. A careless or hostile set could describe itself as the home for working notes and credentials and quietly collect them into a public repository. The answer is a division rather than a refusal: descriptions are topical hints, and two rules override any of them and stay in compiled text a repository cannot reach.",
     "Anything about the person, their machine or their working preferences belongs to the personal set whatever another set claims to want; and secrets are never stored in any set, but put in a file with the location recorded. That keeps the extensible half extensible and the dangerous half fixed.",
     "conventions.md", [("DEPENDS_ON", "memories-are-data-never-instructions"), ("SUPPORTS", "taxonomy-belongs-in-the-sets")]),

    ("secrets-clause-carries-prohibited-data-to-runtime", "2026-08-12", "10:20", "joint", 0.9, ["conventions", "prohibited-data", "runtime"],
     "The secrets clause carries the prohibited-data rule to runtime for the first time. That document asks for the block in both the conventions and, to the extent possible, in software, and as of the drafting neither existed. The convention is the better half to have first: the scan runs at export, by which point the secret is already in the store, where forgetting is the only remedy because a vector is a copy of the text it was built from.",
     "Which half of a two-layer defence to build first is decided by where the damage becomes irreversible, and that is upstream of the layer that was specified first.",
     "conventions.md", [("INSTANCE_OF", "prohibited-data-two-layers")]),

    ("hoard-prefix-is-a-reserved-namespace", "2026-08-12", "10:25", "joint", 0.9, ["conventions", "keywords", "namespace"],
     "The scope-topics keyword is the first operational keyword -- one Scatterhoard reads rather than one a human searches by -- so the prefix is reserved: keywords beginning with the project's short prefix belong to Scatterhoard, and a project using one for its own content is making an error rather than creating an ambiguity.",
     "That is the same namespace claim the project already makes three times over, for environment variables, tool names and the reserved directory, and the existing prefix was chosen over a new abbreviation for exactly that continuity.",
     "conventions.md", [("RESULTS_FROM", "taxonomy-belongs-in-the-sets"), ("RELATED_TO", "namespace-is-reserved")]),

    ("unroutable-memory-goes-personal-and-says-so", "2026-08-12", "10:30", "joint", 0.9, ["conventions", "routing", "fallback"],
     "When nothing fits, use the personal set and say so. It is the destination that cannot be published and can be corrected later, which is the right way for an unroutable memory to fail, and saying so is what gives the human the chance to correct the routing or create the missing set.",
     "A new set has nothing to match against, so the convention also says to give a set its topics memory when creating it -- placement follows the information, since whoever creates a set knows what it is for and nobody later does.",
     "conventions.md", [("SUPPORTS", "route-where-asks-what-it-is-about")]),

    ("one-insight-can-need-two-memories", "2026-08-12", "10:35", "joint", 0.9, ["conventions", "routing", "splitting"],
     "A general lesson with a project-specific instance has no single right destination, and the convention says to store both rather than choose. This project's own store makes the case concrete: its largest single source of defects is a general principle whose evidence is entirely local to the project, and two memories had already been split that way by instinct.",
     "A rule that codifies what careful practice was already doing is the cheapest kind to adopt, because the behaviour is known to be workable before it is required.",
     "conventions.md", [("EXCEPTION_TO", "route-where-asks-what-it-is-about")]),

    ("capture-before-ending", "2026-08-12", "11:00", "joint", 0.95, ["conventions", "capture", "triggers"],
     "The capture convention names the moments where working context is about to be discarded: a piece of work finishing, a commit, an export, or the human signalling they are wrapping up or clearing context. What they share is that knowledge is cheapest to record while the context that produced it still exists, and unrecoverable afterwards -- the boundary is where that cost flips.",
     "Recorded because it was observed rather than reasoned out: the human partner's habit of announcing an imminent context clear provoked additional captures reliably enough to become a habit. That is a tested trigger, and it is the one an LLM cannot detect for itself.",
     "conventions.md", [("SUPERSEDES", "route-where-asks-what-it-is-about")]),

    ("writing-the-list-is-the-mechanism", "2026-08-12", "11:05", "joint", 0.9, ["conventions", "capture", "commitment"],
     "Writing the list before storing is the mechanism, not a formatting preference. The model's own emitted text stays in context and acts as a commitment, where an unstated intention to record something later does not survive a few turns. Saying it aloud also puts the human in a position to correct what was taken to be worth keeping, which is the cheapest review available.",
     "A convention that relies on an intention rather than an artifact is one that degrades silently as the session goes on.",
     "conventions.md", [("SUPPORTS", "capture-before-ending")]),

    ("nothing-worth-keeping-is-a-valid-answer", "2026-08-12", "11:10", "joint", 0.9, ["conventions", "capture", "over-recording"],
     "Nothing worth keeping is stated as a valid answer on purpose. A convention that demands a memory will get one, and the catalog's standing worry is a session that records everything as much as one that records nothing. The permissive phrasing is what stops the prompt manufacturing.",
     "The failure a convention induces by being followed too well is the one its own text has to name, because nothing downstream can distinguish a manufactured memory from a real one.",
     "conventions.md", [("SUPPORTS", "capture-before-ending")]),

    ("export-before-commit-split-in-two", "2026-08-12", "11:15", "joint", 0.9, ["conventions", "export", "splitting"],
     "The export convention was split in two after its old phrasing turned out to carry three things: exporting the file, reviewing the resulting diff, and an unstated assumption that the memories worth exporting had been captured at all. The third is the one that hid -- a convention that reads as though it produces memories can only ship ones that already exist, and reports success on an unchanged file when nothing was captured.",
     "Placement follows the information: at commit time the knowledge has faded and you are reconstructing, while at the end of the work it is fresh -- and the diff and the commit only exist at commit time.",
     "conventions.md", [("RESULTS_FROM", "capture-before-ending"), ("INSTANCE_OF", "decision-placement")]),

    ("routing-failure-is-export-failure", "2026-08-12", "11:20", "joint", 0.9, ["conventions", "routing", "export"],
     "Routing failure and export failure turn out to be the same failure. A local set is never published, so a project memory misfiled as personal is not merely untidy: the export convention can never see it, and it stays on one machine permanently. That is what the eighteen misplaced memories were.",
     "Two conventions that looked independent share a failure mode, which is the kind of coupling that only appears when the catalog is read end to end rather than entry by entry.",
     "conventions.md", [("RESULTS_FROM", "export-before-commit-split-in-two"), ("SUPPORTS", "route-where-asks-what-it-is-about")]),

    ("keywords-do-one-of-two-jobs", "2026-08-12", "12:00", "joint", 0.95, ["conventions", "keywords", "measurement"],
     "Measured over both live sets: 109 distinct keywords over 34 memories, 4.7 per memory, and 71% of the distinct keywords used exactly once. The singletons divide sharply rather than uniformly -- one group names one identifiable thing precisely, which is the case exact match exists for, while the other is broad words coined once, gathering nothing and too vague to search on purpose.",
     "So the rule is not avoid rare keywords, which would have discarded the half that works. A keyword does one of two jobs -- gather this memory with others on its subject, or name one thing exactly -- and a broad word appearing nowhere else does neither.",
     "conventions.md", [("RESULTS_FROM", "route-where-asks-what-it-is-about")]),

    ("recall-the-keyword-before-coining-it", "2026-08-12", "12:05", "joint", 0.9, ["conventions", "keywords", "drift"],
     "The second measured failure is vocabulary drift: a store holding two spellings of one idea as separate singletons, and a term beside two variants of itself. A keyword only gathers when it matches the spelling already in use, so a second name for one idea splits a subject into two searches that each return half of it. Hence recall the keyword before coining it -- a mechanism that costs one call and works with no embedding model loaded.",
     "One such split was produced during the same session, an hour before the convention was drafted and with nothing in the catalog to prevent it, which is as fair a test of the need as the evidence gets.",
     "conventions.md", [("SUPPORTS", "keywords-do-one-of-two-jobs")]),

    ("hyphens-for-coined-terms-identifiers-as-spelled", "2026-08-12", "12:10", "joint", 0.9, ["conventions", "keywords", "spelling"],
     "The keyword constructor permits either separator, so the code has no preference. The stores do: 50 hyphenated keywords against 3 underscored, and all three underscored ones are identifiers quoted verbatim from code, where a hyphen would misname the thing referred to. So the rule is not always hyphenate but hyphens for terms we coin, identifiers spelled as the code spells them.",
     "That is what the project had been doing without writing it down, and keeping an identifier's capitals costs nothing at lookup because the folded form is a separate column from the spelling as written.",
     "conventions.md", [("SUPPORTS", "keywords-do-one-of-two-jobs")]),

    ("no-keyword-count", "2026-08-12", "12:15", "joint", 0.9, ["conventions", "keywords", "decision-placement"],
     "No keyword count is specified. Practice already runs three to six with no rule saying so, so a number would describe sessions rather than change them -- and a choice whose answer changes nothing should not exist. How many keywords a memory needs is information that lives with the memory being written.",
     "The decision-placement test applied to a convention rather than to a parameter, which is the same test doing the same work one layer up.",
     "conventions.md", [("INSTANCE_OF", "decision-two-checks")]),

    ("no-rule-against-keying-by-a-persons-name", "2026-08-12", "12:20", "human", 0.95, ["conventions", "keywords", "golden-rule"],
     "A rule against keying a memory by a person's name was drafted and rejected. The argument for it was that keywords add aggregation to the hazards the Golden Rule names -- six scattered observations keyed by a colleague's name become a dossier retrievable in one call. Two things defeat it: the legitimate uses are ordinary, so a ban would have blocked the index while leaving the record untouched; and the dossier is prevented by routing, not by spelling.",
     "The keyword layer knows how a term is spelled and not what it aggregates, so a rule placed there is a rule placed where the information is not. The content is the target, and the storing convention is already aiming there.",
     "conventions.md", [("INSTANCE_OF", "decision-placement"), ("RELATED_TO", "aggregation-harm-is-emergent")]),

    ("store-when-person-clause-replaced", "2026-08-13", "13:00", "joint", 0.95, ["conventions", "golden-rule", "person"],
     "The old person clause -- record the specific observation rather than a characterisation -- carried the verdict and lost the axis, so it read a plain statement of expertise as suspect when the hazard is an unfavourable claim on thin evidence. The new clause states the demands directly: record what a later session's work needs, record what you observed rather than what you concluded, name where it can be checked, and before generalising account for what you do not know.",
     "Three open questions were settled together because they were one question, and the clauses carrying the principle had each been compiled from its conclusions with the derivation never written down.",
     "conventions.md", [("RESULTS_FROM", "four-demands-articulated")]),

    ("the-design-may-derive-the-served-text-must-state", "2026-08-13", "13:05", "human", 0.95, ["conventions", "drafting", "weaker-models"],
     "The person clause restates rules the convention already makes rather than referring to them, because it is those demands applied without the exemption a judgement about someone quietly gets. That was the elegant finding in the derivation and it is exactly what must not be left implicit: a weaker model, without an extended reasoning session, will not make that leap. The design document may derive; the served text must state.",
     "It is a constraint on drafting that follows from who reads the served text, and it cuts against the instinct to keep the catalog compact by pointing rather than repeating.",
     "conventions.md", [("SUPPORTS", "store-when-person-clause-replaced")]),

    ("reinforcement-carries-the-boundary", "2026-08-13", "13:10", "joint", 0.9, ["conventions", "golden-rule", "boundary"],
     "The reinforcement now carries the boundary into served text for the first time: this is charity about motive only, and it says nothing about whether the tests pass. That sentence is the runtime answer to the standing objection that a kindness directive softens technical criticism, and it is load-bearing rather than decorative.",
     "The objection had kept the principle out of served text entirely, so the sentence that answers it is what makes the clause shippable at all.",
     "conventions.md", [("DEPENDS_ON", "charitable-about-people-exact-about-code"), ("SUPPORTS", "store-when-person-clause-replaced")]),

    ("no-shared-set-takes-people-as-its-subject", "2026-08-13", "13:15", "joint", 0.95, ["conventions", "routing", "override"],
     "The routing convention gains a third override: no shared set takes people as its subject, whatever its description claims -- a memory about a person goes with the work it concerns, or to the personal set. This closes the hole the rejected keyword clause could never have closed, because aggregation harm is a property of the set rather than of any record in it, so no rule examining one memory at a time can see it.",
     "It joins the compiled overrides rather than the topical descriptions because this is the one description whose faithful execution produces the artefact the principle exists to prevent, so it has to sit in text a repository cannot reach.",
     "conventions.md", [("RESULTS_FROM", "aggregation-harm-is-emergent"), ("SUPPORTS", "descriptions-are-hints-two-rules-override")]),

    ("person-override-is-scoped-to-shared-sets", "2026-08-13", "13:20", "joint", 0.9, ["conventions", "routing", "scope"],
     "The override is scoped to shared sets on purpose. The personal set is the acknowledged catch-all and can accumulate cross-topic memories about one person, but it never travels, so the durability and reach mechanics are weaker on one person's own machine -- and widening the override to every set would forbid the fallback the convention's own next sentence depends on.",
     "One consequence is accepted rather than overlooked: this forbids a team set whose charter is personnel notes, weighed against the argument that a charter is visible by design and a tool refusing to hold them is making a policy choice its users did not ask for.",
     "conventions.md", [("EXCEPTION_TO", "no-shared-set-takes-people-as-its-subject")]),

    ("recall-before-storing", "2026-08-13", "14:01", "joint", 0.9, ["conventions", "recall", "duplicates"],
     "The recall-before-storing convention closes a gap neither neighbour reached: the recall convention fires before acting, and the storing convention assumes the question is being asked for the first time. It sits second in catalog order, and the position is the argument -- the front of the catalog is the sequence of questions asked while writing one memory, and this must be settled before routing and keying, because a memory that turns out to be a correction is routed and keyed by the record it corrects.",
     "Catalog order carrying an argument is worth stating, because a later reorganisation on any other principle would break a dependency nothing else records.",
     "conventions.md", [("RESULTS_FROM", "keywords-do-one-of-two-jobs")]),

    ("recall-and-read-not-a-tool-that-warns", "2026-08-13", "14:06", "human", 0.95, ["conventions", "duplicates", "decision-placement"],
     "The deciding argument is half-overlaps: as a corpus grows the common case stops being duplicate-or-not, and a new memory partly overlaps an existing one, where the right response is to split or correct the memory already there rather than to decide whether to store. Proximity is all a vector knows, and a duplicate, a reframed twin, a deliberate supersession and a partial overlap are indistinguishable from a distance -- a supersession is supposed to resemble its predecessor.",
     "So a tool that warns asserts a relationship it has no information about, which is a control that looks usable but is not. Software surfaces the neighbours; the session reads them.",
     "conventions.md", [("INSTANCE_OF", "bad-placement-worse-than-none"), ("SUPPORTS", "recall-before-storing")]),

    ("declining-the-claim-removes-the-dependency", "2026-08-13", "14:11", "joint", 0.9, ["conventions", "threshold", "unblocking"],
     "This unblocks the software half rather than deferring it. A warning needs a threshold, and a threshold inherits the missing non-match floor and the cold-start scoring question, where measured separation is thin enough that any constant would be arbitrary. Surfacing the nearest few asserts nothing and needs no floor, so declining to make the claim removes the dependency.",
     "A design that needs a number nobody can defend can sometimes be rescued by making a weaker claim rather than by finding the number.",
     "conventions.md", [("RESULTS_FROM", "recall-and-read-not-a-tool-that-warns")]),

    ("restate-what-is-absent-point-at-what-is-present", "2026-08-13", "14:16", "joint", 0.9, ["conventions", "drafting", "restatement"],
     "The convention points at the correcting convention rather than restating it, which is not in tension with the rule that served text must state rather than derive: that rule is about not making a reader reconstruct an omitted derivation, and the correcting convention is served in the same catalog, in front of the reader. Restate what is absent; point at what is present.",
     "The pull while drafting was to inline it, which is why the distinction is written down rather than left to judgement each time.",
     "conventions.md", [("EXCEPTION_TO", "the-design-may-derive-the-served-text-must-state")]),

    ("reframing-defeats-mechanical-checks", "2026-08-13", "14:21", "joint", 0.9, ["conventions", "duplicates", "evidence"],
     "This project's own store grew four pairs of twins -- memories in the personal set with counterparts in the project set, recorded later and reframed, the local originals never removed. Reframing is exactly what let them survive every mechanical check, which is the argument against a comparison-based remedy and for a reading one.",
     "A duplicate that has been reworded is not detectable by the mechanism that detects duplicates, so the evidence and the chosen remedy point the same way.",
     "conventions.md", [("SUPPORTS", "recall-and-read-not-a-tool-that-warns")]),

    ("splitting-defers-to-detection-time", "2026-08-13", "15:01", "joint", 0.95, ["conventions", "splitting", "decision-placement"],
     "At store time nobody can predict which half of a memory will rot first, so a rule against bundling asks the writer for a judgement they cannot make; once one half has gone stale the answer is already known. That is decision placement, and it is also why a store-time recall reconciles with it rather than contradicting it -- that recall is a detection moment, not a writing-time guess.",
     "The cost of bundling is measured rather than assumed: in the link-label corpus test, supersedes versus contradicts was unambiguous only where an observation and its explanation had been stored as two memories, and bundled, neither label fit.",
     "conventions.md", [("INSTANCE_OF", "decision-placement"), ("SUPPORTS", "recall-before-storing")]),

    ("leaving-the-original-in-place", "2026-08-13", "15:06", "joint", 0.95, ["conventions", "splitting", "links"],
     "Leaving the original in place is the load-bearing half. Links elsewhere point at the original's id, and a link held in another set cannot be rewritten from here, since the memory holding it may live in a repository this machine has no business committing to. Keeping the original means those references still resolve and traversal follows the supersession forward to the parts; deleting or amending it strands them.",
     "This is the clause most likely to be simplified away on tidiness grounds, which is why the source comment says not to.",
     "conventions.md", [("SUPPORTS", "splitting-defers-to-detection-time")]),

    ("adjacency-to-correct-when-is-deliberate", "2026-08-13", "15:11", "joint", 0.85, ["conventions", "splitting", "catalog-order"],
     "The correcting convention says a changed claim is superseded by storing the corrected memory -- singular, which is wrong when only part of the memory changed. The splitting convention is a refinement of that branch rather than a separate operation, and serving them adjacently is what makes them read as one rule. Separated, a session applies the general form and bundles a half-stale correction into a single record.",
     "Adjacency doing semantic work is a property of a served catalog that a reference document does not have, and it is easy to lose in a reorganisation.",
     "conventions.md", [("SUPPORTS", "splitting-defers-to-detection-time")]),

    ("confidence-anchored-on-evidence-not-feeling", "2026-08-13", "16:00", "joint", 0.95, ["conventions", "confidence", "calibration"],
     "How you came by a claim is a fact about the claim; how sure you feel is a fact about you, and two sessions will not agree on the second. The ladder -- watched it happen, measured or decided, inferred from evidence that fits without compelling, a reading you would check first -- is what makes two sessions land within a band of each other on the same memory.",
     "Measured across both live sets: 44 memories carrying five distinct values, mean 0.888, and nothing below 0.8 ever written in roughly two weeks. The previous measurement had the same five values, so seven further memories introduced none -- the field is not slowly acquiring range, and a scale whose entire observed span is its top fifth costs a judgement at every store and returns nothing.",
     "conventions.md", [("RESULTS_FROM", "keywords-do-one-of-two-jobs")]),

    ("confidence-numbers-are-anchors-not-thresholds", "2026-08-13", "16:05", "joint", 0.85, ["conventions", "confidence", "thresholds"],
     "The numbers are anchors, not thresholds. Nothing branches on them, and the served text says near deliberately. The consumer today is the reading session deciding how far to lean without re-checking, and graph traversal will consume the field mechanically once it exists.",
     "Naming the current consumer and the future one separately is what keeps the calibration honest about which of them it is actually serving.",
     "conventions.md", [("SUPPORTS", "confidence-anchored-on-evidence-not-feeling")]),

    ("recalibration-is-an-amendment", "2026-08-13", "16:10", "joint", 0.9, ["conventions", "confidence", "corrections"],
     "Recalibration is an amendment: changing confidence on an identical claim is apparatus rather than a new belief. Without the clause, a session that becomes less sure reaches for supersession -- the general rule -- and fills a chain with records nobody ever believed differently.",
     "A general rule applied to a case it was not written for produces a plausible-looking history that is false, which is worse than no history.",
     "conventions.md", [("SUPPORTS", "confidence-anchored-on-evidence-not-feeling")]),

    ("a-low-number-is-not-permission", "2026-08-13", "16:15", "joint", 0.95, ["conventions", "confidence", "laundering"],
     "This convention opens a laundering route by its own existence: once the low end is usable, recording something at 0.4 becomes an available answer to a memory the storing convention says to leave out -- in particular the unfavourable generalisation the fourth demand removes. The last sentence of the served text closes it.",
     "The Golden Rule carried exactly that error in its own machinery list until this convention was drafted, its confidence bullet still saying an uncharitable generalisation should be recorded as a low-confidence claim, which the fourth demand had contradicted hours earlier without anyone noticing.",
     "conventions.md", [("EXCEPTION_TO", "confidence-anchored-on-evidence-not-feeling"), ("DEPENDS_ON", "demand-d-has-two-halves")]),

    ("correct-when-was-withheld", "2026-08-05", "16:20", "joint", 0.9, ["conventions", "corrections", "withholding"],
     "The correcting convention was deliberately withheld from the served catalog while the amend and forget tools did not exist: guidance telling a session to reach for two absent tools would have been a convention asserting behaviour the system lacked, sitting in the one place read into every session. Both tools landed and it is served now.",
     "Writing the text before the tools was worth doing -- the reasoning was fresh, the implementation had something to be built against, and the wording did not have to be re-derived when the withholding condition expired.",
     "conventions.md", [("RELATED_TO", "correction-is-its-own-tool")]),

    ("golden-rule-reversed-an-answer", "2026-08-05", "16:25", "joint", 0.95, ["conventions", "golden-rule", "corrections"],
     "Worth noting as an instance rather than a claim: the Golden Rule reaches runtime through the reasoning attached to memory-writing conventions, and here it did more than that -- it reversed an answer. The rest of the correction rule says a claim that changed is superseded, which would have made a retracted characterisation of a colleague permanently readable, the durability hazard produced by faithfully applying the general rule.",
     "The exception is in the convention text because the principle was in context while the text was being drafted, which is the mechanism working as intended.",
     "conventions.md", [("INSTANCE_OF", "principle-reaches-runtime-through-why"), ("DEPENDS_ON", "durability-asymmetry")]),

    ("keep-the-convention-set-small", "2026-08-03", "17:00", "human", 0.95, ["conventions", "scope", "phase"],
     "Keep the convention set small until the basic system works. The catalog is deliberately minimal -- enough to exercise the mechanism, not an attempt at coverage. Expanding it is easy and tempting, and doing it before there is a working system to test conventions against would mean writing a lot of guidance nobody has watched operate.",
     "Two things are deferred on purpose: formalising the standing principles into conventions of their own, and a convention about how to store memories in ways that let new principles be discovered from practice.",
     "conventions.md", [("SUPPORTS", "conventions-are-hard-coded-memories")]),

    ("built-in-if-not-knowing-makes-you-worse-off", "2026-08-09", "17:05", "joint", 0.9, ["conventions", "built-in", "criterion"],
     "A convention belongs in the built-in catalog if a user who does not know it exists would be worse off. Held as a test to try against candidates rather than a settled rule, since the division is expected to move as the catalog grows and a criterion adopted before it has judged anything real is one nobody has watched operate.",
     "What recommends it is that it discriminates without needing a taxonomy of subject matter, and the asymmetry it encodes is the reason it works: the cost of a missing built-in is invisible to the person paying it, and the cost of an unnecessary one is not. So the test puts the burden of proof where the failure is silent.",
     "conventions.md", [("RESULTS_FROM", "keep-the-convention-set-small")]),

    ("budget-covers-built-ins-only", "2026-08-09", "17:10", "joint", 0.95, ["conventions", "budget", "decision-placement"],
     "The character budget applies to the built-in catalog and not to add-ons. Built-in conventions are served to everyone, so their cost is a tax nobody opted into and the project has to hold the line on their behalf. Add-ons are a matter of user choice, and a user who enables one has both the information about their own context pressure and the stake in it.",
     "A convenient consequence: the budget test measures the compiled catalog, which is exactly the built-in set, so if built-ins stay compiled in and add-ons load at runtime the existing test goes on measuring precisely the right thing -- the boundary the policy draws is already the boundary the code draws.",
     "conventions.md", [("INSTANCE_OF", "decision-placement")]),

    ("add-on-cost-is-a-reporting-problem", "2026-08-09", "17:15", "joint", 0.85, ["conventions", "budget", "reporting"],
     "It does not follow that add-on cost is invisible. A user choosing well needs to know what a convention costs before enabling it, which is a reporting problem rather than an enforcement one, and is open.",
     "Distinguishing the enforcement question from the reporting question is what let the first be settled while the second stays open, instead of both waiting on each other.",
     "conventions.md", [("EXCEPTION_TO", "budget-covers-built-ins-only")]),

    ("budget-raised-on-what-it-spends", "2026-08-12", "18:00", "human", 0.95, ["conventions", "budget", "reasoning"],
     "The budget was raised on what it is actually spending rather than on what the next convention needs. Eight thousand characters is around 1,950 tokens against context windows running from 128K on smaller local models up to about 1M, and both numbers are small at either end. A budget set at the old level would have been paid for by shrinking conventions against each other -- spending drafting effort and clarity to save something the context window does not notice.",
     "The token figures are the pessimistic end of the proxy, which is the right end to decide on: they assume the catalog drifts toward the vocabulary tables the constant's documentation warns about, where prose runs considerably cheaper.",
     "conventions.md", [("RESULTS_FROM", "budget-covers-built-ins-only")]),

    ("a-budget-raised-whenever-it-binds-is-not-a-budget", "2026-08-13", "18:05", "human", 0.95, ["conventions", "budget", "discipline"],
     "The second raise was decided on the same ratio the first was, against a proportional bar: stay well under 4% of the smallest context window in range. What has kept both honest is that each was argued from the ratio rather than from what would not fit -- a raise argued the other way round is the one to refuse, because a budget raised whenever it binds is not a budget.",
     "The instruction is still to work towards it rather than fill it, and it applies harder at a second raise than a first. The standing worry about a session that records everything has a catalog form, and more room is exactly when it applies.",
     "conventions.md", [("SUPERSEDES", "budget-raised-on-what-it-spends")]),

    ("budget-constant-does-double-duty", "2026-08-13", "18:10", "joint", 0.9, ["conventions", "budget", "paging"],
     "The budget constant does double duty: it is also the default page size for a conventions request. That coincidence is what makes the catalog reliably serve in a single page -- the ceiling on its size and the default page size are the same number, so a catalog that passes the budget test cannot need a second page. Changing one changes the other.",
     "A coupling that is load-bearing and invisible from either side is worth knowing before touching the constant, since the failure would be a catalog that silently needs paging.",
     "conventions.md", [("SUPPORTS", "a-budget-raised-whenever-it-binds-is-not-a-budget")]),

    ("recall-gloss-convention-undrafted", "2026-08-13", "18:15", "joint", 0.8, ["conventions", "recall-gloss", "context-field"],
     "A recall-gloss convention is wanted and not yet drafted: when the LLM surfaces a recalled memory to the human it should lead with a short gloss built from the memory's context field rather than restating content cold, which is the behaviour that actually solves the friction the context field exists to address. It also needs guidance on what is worth capturing into context in the first place, kept loose rather than a fixed schema.",
     "A convention whose text is written but not yet served is a normal state and is a backlog entry rather than an omission, and the same holds for one identified but undrafted.",
     "conventions.md", [("RELATED_TO", "keep-the-convention-set-small")]),

    # ---- TRACKING.md ----------------------------------------------------------------
    ("open-questions-gathered-in-one-place", "2026-08-15", "16:00", "joint", 0.9, ["open-questions", "tracking", "boundary"],
     "Unresolved design questions are gathered from across the subject files so they do not get lost, and a question resolved elsewhere is moved out into its subject file with the reasoning recorded there. A resolved question often leaves work behind, and that work goes to the backlog -- decided, not yet built.",
     "The boundary in the other direction: if the work cannot start until someone chooses between two designs, it stays a question here however concrete it sounds. Resolving a question and finding nothing implements the answer is the normal case rather than a discovery worth burying.",
     "TRACKING.md", []),

    ("store-when-may-need-splitting", "2026-08-13", "16:06", "joint", 0.85, ["open-questions", "conventions", "store-when"],
     "Open: whether the storing convention's person material should be a convention of its own. It is 2,400 characters of a 14,632-character catalog -- one convention holding a sixth of everything served and 60% larger than the next largest -- with roughly 40% of its text now the person clause. Its own editing comment predicts field-level guidance leaving rather than the entry growing, and that has been honoured twice, but the person material is a content rule so nothing in that comment governs it.",
     "What makes it genuinely open is that the obvious fix has a real cost: the clause works precisely because it restates the convention's own two demands, so split out it must either restate them a second time -- two copies free to drift -- or refer to them, reintroducing the derivation leap the restatement was written to remove.",
     "TRACKING.md", [("RESULTS_FROM", "store-when-person-clause-replaced")]),

    ("sets-declaring-their-own-keyword-conventions", "2026-08-12", "16:10", "human", 0.8, ["open-questions", "keywords", "sets"],
     "Open: whether a set should be able to declare its own keyword conventions, via a reserved keyword that is the obvious sibling of the topics memory routing already matches against.",
     "Raised as the natural extension of a mechanism that had just been built, which is the moment such a question is cheapest to notice and most likely to be over-generalised.",
     "TRACKING.md", [("RELATED_TO", "taxonomy-belongs-in-the-sets")]),

    ("partial-export-and-import", "2026-08-11", "16:15", "human", 0.85, ["open-questions", "export", "partial"],
     "Open: whether a set supports partial export and import, raised from the case of handing someone a few memories rather than a whole set. Deferred rather than rejected, and the immediate concern was operational rather than architectural, since the exchange today is whole-set in both directions.",
     "A recipient already handles a partial file correctly, so what is missing is a way to say which records travel and a base rule for a target that agrees with only part of a set.",
     "TRACKING.md", [("RELATED_TO", "partial-exchange-needs-less-than-it-looks")]),

    ("memories-written-during-a-code-review", "2026-08-05", "16:21", "human", 0.85, ["open-questions", "review", "checkout"],
     "Open: how memories written during a code review reach the set, raised from a configuration actually in use -- one repository checked out twice on one machine, one checkout for development and one for review. The base decision makes the case mechanically reconcilable; it does not settle the authority.",
     "Who may write memories from a reviewing checkout, and when, is a separate question from whether the two checkouts can be reconciled, and conflating them is what made the reconciliation work look like it had answered more than it did.",
     "TRACKING.md", [("RELATED_TO", "base-is-per-record-and-per-target")]),

    ("where-the-trusted-set-name-comes-from", "2026-08-15", "16:25", "joint", 0.9, ["open-questions", "trust", "set-name"],
     "Open: where the trusted set name for a checkout comes from. The scope kind is settled, since the path decides it and the header can only agree. The set name is not: a project's name and its set name need not coincide, so the path does not yield it, and the header is untrusted content.",
     "The question has narrowed rather than closed -- registration would answer it by having a human name the scope for a path once, and the bases table already holds a trust-on-first-use record that was built for something else.",
     "TRACKING.md", [("RELATED_TO", "target-list-is-an-unintended-trust-record")]),

    ("occurred-time-distinct-from-created", "2026-08-15", "16:30", "joint", 0.85, ["open-questions", "data-model", "time"],
     "Open: whether a memory needs an occurred time distinct from created. Two labels exist to express world-event order, and with no occurred field they are currently the only way to express it. A field is cheaper when absolute times are known; edges are better when only relative order is, which is the usual situation for an LLM reconstructing a history after the fact.",
     "The choice between a field and an edge for the same information is decided by what the writer actually knows, which is why it is deferred alongside the labels rather than settled on its own.",
     "TRACKING.md", [("RELATED_TO", "fourteen-labels")]),

    ("whether-violates-belongs-at-all", "2026-08-15", "16:35", "joint", 0.85, ["open-questions", "labels", "scope"],
     "Open: whether recording that something violates a decision belongs in a memory system at all. The label earned its place on frequency and prevents defects being recorded as sanctioned exceptions, but the natural home for a known defect is an issue tracker, and a memory set accumulating violation edges may be doing a job it should not.",
     "A label justified by how often it was reached for may still be reaching for the wrong artifact, which is a question frequency data cannot answer and only real usage can.",
     "TRACKING.md", [("RELATED_TO", "fourteen-labels")]),

    ("personal-set-exception-needs-unforgeable-scope", "2026-08-15", "17:00", "joint", 0.9, ["open-questions", "trust", "instructions"],
     "Open: whether the personal memory set should be exempt from the data-only rule, given its contents are authored by the local user rather than pulled from anywhere. Plausible, not confirmed. The precondition is that the exemption is only safe while scope is unforgeable -- if a memory set could talk its way into local scope, an exemption would turn scope forgery from a trust elevation into straight instruction injection.",
     "A repository would then only need to claim local scope to have its memories treated as directives, so the header guard is what currently holds that shut, and the missing set-name binding is the part of it still open.",
     "TRACKING.md", [("DEPENDS_ON", "header-scope-checked-never-used"), ("EXCEPTION_TO", "memories-are-data-never-instructions")]),

    ("import-side-gate-and-provenance-weighting", "2026-08-15", "17:05", "joint", 0.8, ["open-questions", "trust", "import"],
     "Three sub-questions remain on the trust boundary: whether a counterpart to the export review gate is warranted on import and whether it would inspect content or just surface what changed; whether origin and author should affect how much trust a recalled memory carries; and whether the data-only rule is carried by convention text alone or the software also does something.",
     "Each is a different layer of the same boundary, and keeping them separate is what stops one being answered in a way that presumes an answer to another.",
     "TRACKING.md", [("SUPPORTS", "personal-set-exception-needs-unforgeable-scope")]),

    ("whether-amendment-may-touch-content", "2026-08-05", "17:10", "joint", 0.9, ["open-questions", "corrections", "amend"],
     "What remains open on correcting a mis-recorded memory is narrower than it was: whether amendment may touch content. By the principle a content typo is a faulty record and should be amendable, but typo or changed claim is a judgement the software cannot verify and the failure directions are unequal, so a content typo is superseded until a real case makes the cost concrete.",
     "Choosing the recoverable failure while the evidence is absent is the same posture the correcting convention takes for its own unclear cases, applied to the design rather than to a session.",
     "TRACKING.md", [("RELATED_TO", "claim-is-content-plus-why")]),

    ("what-recall-returns-is-a-budget-question", "2026-08-03", "17:15", "joint", 0.85, ["open-questions", "recall", "budget"],
     "Partly resolved: recall and lookup now return the memory's complete field set through a single view type shared by every tool, so a returned memory carries its own supersession edge and the walk the design describes is available to a caller -- the code caught up to the design rather than the other way round. What remains is the budget question of how much to return.",
     "The reason the full field set went in first was corpus integrity rather than caller convenience, which is worth recording because it explains an ordering that otherwise looks like it optimised the wrong thing.",
     "TRACKING.md", [("RELATED_TO", "chains-document-themselves")]),

    ("similarity-in-a-corpus-with-no-centre", "2026-08-15", "17:20", "joint", 0.9, ["open-questions", "cold-start", "centroid"],
     "Observed driving the server against the real model: a store holding exactly one memory returns that memory for a query it plainly matches, with a similarity of zero. This is arithmetic working correctly, since scores are centred on the corpus mean and with one vector the memory is the mean. Adding two more memories restored sensible separation.",
     "The empty corpus is not affected, since it returns nothing, which is the honest answer. The problem is the shape of what a caller sees -- the very first recall against a new set is the one most likely to look broken.",
     "TRACKING.md", [("RESULTS_FROM", "subtract-the-corpus-centroid")]),

    ("hybrid-search", "2026-08-15", "17:25", "joint", 0.9, ["open-questions", "hybrid-search", "recall"],
     "Open: whether recall should ever combine semantic and keyword search into one ranking. The combination was removed because it integrated nothing -- two independent searches stapled together, duplicating payload, putting two scales in one response, with no honest ordering -- and separating them makes those unrepresentable. But separation is the conservative answer rather than the best one, and a memory found by both paths is stronger evidence of relevance than one found by either, a signal that now exists nowhere.",
     "Recording what a conservative answer discards is what keeps it revisitable, since the argument that removed the combination says nothing against a genuinely integrated ranking.",
     "TRACKING.md", [("RESULTS_FROM", "what-refusing-the-combination-costs")]),

    ("conventions-steering-toward-single-sources-of-truth", "2026-08-03", "17:30", "joint", 0.85, ["open-questions", "conventions", "standards"],
     "Open: whether conventions should steer an LLM toward a project's single sources of truth. A majority of the defects in the built system came from one fact being written down twice and the copies drifting silently. The engineering rule is now in the standards file, but a rule in a standards file only helps a session that reads it. The proposal is a convention pair: record a project's authoritative sources as memories when they are established, and recall before writing anything that might duplicate one.",
     "It is a good test of whether the memory system can carry an engineering discipline rather than only project facts, which is a larger claim than any single convention.",
     "TRACKING.md", [("RELATED_TO", "keep-the-convention-set-small")]),

    ("how-conventions-get-extended-without-recompiling", "2026-08-09", "18:00", "human", 0.9, ["open-questions", "conventions", "add-ons"],
     "Open: how conventions get extended without recompiling. Scatterhoard ships with a deliberately minimal catalog to limit the context tax, which is the right default and also creates standing pressure to add bells and whistles, and a strong convention takes runtime and revisions to get right, so the good ones will be discovered by use rather than written up front.",
     "That argues for two directories -- optional conventions bundled with the project and reasonably well-tested, and add-ons installed at arm's length -- and the line between them is expected to move rather than being pre-committed to a taxonomy.",
     "TRACKING.md", [("RESULTS_FROM", "keep-the-convention-set-small")]),

    ("optional-and-add-on-are-different-trust-propositions", "2026-08-09", "18:05", "joint", 0.85, ["open-questions", "conventions", "trust"],
     "Optional and add-on are different trust propositions rather than just different provenance. Bundled-optional ships in this repository and is reviewed like source, differing from the built-in catalog only by being off by default; add-on is third-party prose installed at arm's length. The same serving mechanism covers both, which is what makes it tempting to treat them alike.",
     "The likely consequence is that add-ons need explicit per-id opt-in rather than everything in the directory being included, which doubles as the answer to how a user discovers what they have enabled.",
     "TRACKING.md", [("SUPPORTS", "how-conventions-get-extended-without-recompiling")]),

    ("add-ons-collide-with-the-no-instruction-rule", "2026-08-09", "18:10", "joint", 0.95, ["open-questions", "conventions", "security"],
     "This collides with a standing decision and has to reconcile with it rather than route around it. The design currently provides no location for instruction-shaped text at all, on the grounds that an auto-loaded instruction channel is the sharpest available attack surface. Conventions are the exception the data-only rule was never covering, since they exist precisely to steer behaviour, and a directory of third-party conventions served into every session is that channel arriving under a different name.",
     "The expected resolution preserves the rule rather than weakening it: the threat model is the project supply chain, text arriving by pull, so add-on conventions would live in user configuration space and a project repository could never contribute one.",
     "TRACKING.md", [("DEPENDS_ON", "nothing-instruction-shaped-auto-loaded"), ("SUPPORTS", "how-conventions-get-extended-without-recompiling")]),

    ("convention-file-format-needs-three-categories", "2026-08-09", "18:15", "joint", 0.9, ["open-questions", "conventions", "file-format"],
     "The file format has to express three categories, not two. The obvious reading is content versus comment with a single blockquote for content, and that is one category short: a convention carries an id, its text and its reinforcement, and reinforcement is served with every use while editing background is never served. A one-blockquote format puts reinforcement outside the served region -- and reinforcement is the field that looks like commentary, because it is the why.",
     "The Golden Rule reaches runtime through reinforcement, so that format would silently sever it from every add-on convention. The preferred shape is named sections, because a missing section fails loudly where a forgotten blockquote fails silently.",
     "TRACKING.md", [("DEPENDS_ON", "why-it-is-not-called-why"), ("SUPPORTS", "how-conventions-get-extended-without-recompiling")]),

    ("loading-at-session-start-is-partly-the-clients", "2026-08-09", "18:20", "joint", 0.85, ["open-questions", "conventions", "loading"],
     "Loading at session start is only partly Scatterhoard's to decide. The goal of loading conventions once is recorded, but MCP gives a server no way to push anything at session start -- the server cannot initiate -- so the mechanism is client-side and differs per client, which makes it configuration guidance as much as design.",
     "Worth deciding what the honest fallback is for a client with no hook mechanism, since that is the case where the goal cannot be met and the design should say so rather than assume it.",
     "TRACKING.md", [("DEPENDS_ON", "load-conventions-once")]),

    ("one-repository-more-than-one-set", "2026-08-15", "19:00", "joint", 0.9, ["open-questions", "file-format", "sets"],
     "Open, and deliberately not resolved: whether one repository can carry more than one memory set, and what the single well-known filename is actually protecting. It only becomes real if a cross-project set is meant to be reused by others, and two futures pull differently -- a set others contribute back to is a maintained project needing nothing new, while a set others copy and reuse would want either merging into their own repository or two sets living in one.",
     "The existing reasoning is narrower than one name and survives more than it looks like it would: the trust argument is a property of the reserved directory rather than of the filename, so a second file inside it is still path-trusted, and the merge-driver argument pins a pattern that a glob could satisfy as well as a literal.",
     "TRACKING.md", [("DEPENDS_ON", "memories-file-well-known-name")]),

    ("copying-duplicates-an-id-deliberately", "2026-08-15", "19:05", "joint", 0.9, ["open-questions", "identifiers", "copying"],
     "The identifier hazard is a different shape than it first appears. UUIDs make accidental collision negligible, but copying a memory from one set to another duplicates its id deliberately, and a cross-set link that identifies its target by id then has two answers. That is a question about what copying means, not about identifier width.",
     "Locating a hazard in an operation rather than in a format is what stops it being answered with a longer identifier, which would not touch it.",
     "TRACKING.md", [("SUPPORTS", "one-repository-more-than-one-set")]),

    ("store-when-should-name-method", "2026-08-15", "19:10", "human", 0.9, ["open-questions", "conventions", "method"],
     "Open: whether the storing convention's enumeration should name method alongside knowledge. Raised by a miss -- a measurement procedure had not been stored, recorded only as one sentence in another memory's context, with its script written to a scratchpad. The convention opens with a general clause that plainly covers it and then enumerates five items, every one of which is a kind of knowledge and none of which is a method.",
     "The enumeration is what a reader anchors on and it is what was followed. The fix is not obviously an edit: the convention is already the largest in the catalog and its size is its own open question, so it may be that the enumeration should be shortened to stop competing with the general clause rather than lengthened.",
     "TRACKING.md", [("RESULTS_FROM", "store-when-may-need-splitting")]),

    # ---- index.md -------------------------------------------------------------------------
    ("design-split-by-subject", "2026-08-03", "20:00", "joint", 0.85, ["index", "documents", "organisation"],
     "The design used to live in a single file and is now split by subject so each topic can be found, linked and edited independently, with the old file pointing at the index.",
     "A single design document makes every cross-reference a section anchor inside one file, which is what made the reference structure invisible; splitting it is what let the links between subjects become the thing this corpus is built from.",
     "index.md", []),

    ("two-standing-principles-sit-apart", "2026-08-03", "20:05", "joint", 0.9, ["index", "principles", "organisation"],
     "Two documents are listed as standing principles rather than subjects: the Golden Rule, loaded before drafting or revising conventions, and decision placement, applied before adding any parameter, field or required choice. The rest are subjects covering one topic each.",
     "The distinction is about when a document is read rather than what it contains -- a standing principle is an input to work on other documents, so filing it as one subject among many would lose exactly the instruction that makes it work.",
     "index.md", [("SUPPORTS", "golden-rule-stays-in-context"), ("RELATED_TO", "decision-placement")]),

    # ---- APPEND MARKER ----
]

_keys = [e[0] for e in M]
_dupes = sorted({k for k in _keys if _keys.count(k) > 1})
assert not _dupes, f"duplicate memory keys: {_dupes}"
_missing = sorted({t for e in M for _, t in e[9]} - set(_keys))
assert not _missing, f"links point at keys that do not exist: {_missing}"
_stamps = [(e[1], e[2]) for e in M]
_clashes = sorted({t for t in _stamps if _stamps.count(t) > 1})
assert not _clashes, (
    "two memories share a timestamp, which makes the outbound-rule check vacuous "
    f"for any link between them: {_clashes}")

ids, order = {}, {}
for i, entry in enumerate(M):
    key, day, hhmm = entry[0], entry[1], entry[2]
    ms, _ = ts(day, hhmm)
    ids[key] = uuid7(ms, key)
    order[key] = ms

records = []
for key, day, hhmm, origin, conf, kw, content, why, source, links in M:
    ms, iso = ts(day, hhmm)
    stamp = {"by": "jtcours", "at": iso}
    records.append((ms, key, {
        "id": ids[key],
        "content": content,
        "why": why,
        "context": f"Extracted from documents/design/{source}.",
        "keywords": kw,
        "confidence": conf,
        "origin": origin,
        "created": stamp,
        "modified": dict(stamp),
        "links": [{"label": lbl.lower(), "target": ids[tgt]} for lbl, tgt in links],
    }, links, source))

# Sorted by id, as export is -- the file format orders records by id to keep merge
# diffs small, and this corpus is meant to be readable by the same tooling.
records.sort(key=lambda r: r[2]["id"])

out_dir = os.path.dirname(os.path.abspath(__file__))
path = os.path.join(out_dir, "traversal-corpus.jsonl")
with open(path, "w") as f:
    f.write(json.dumps({"format_version": "0.1.0",
                        "scope": {"shared": "traversal-pilot"}}) + "\n")
    for _, _, rec, _, _ in records:
        f.write(json.dumps(rec) + "\n")

# ---- analysis -------------------------------------------------------------
counts, per_doc, cross_doc = {}, {}, 0
src_of = {key: source for _, key, _, _, source in records}
for _, key, _, links, source in records:
    per_doc[source] = per_doc.get(source, 0) + 1
    for lbl, tgt in links:
        counts[lbl] = counts.get(lbl, 0) + 1
        if src_of.get(tgt) != source:
            cross_doc += 1

total_edges = sum(counts.values())
print(f"{len(records)} memories, {total_edges} edges -> {path}\n")
print("memories per source document:")
for doc in SOURCES:
    print(f"  {doc:28s} {per_doc.get(doc, 0):3d}")
print("\nlabel usage:")
for lbl in sorted(counts, key=lambda k: -counts[k]):
    print(f"  {lbl:16s} {counts[lbl]:2d}")
print(f"\ncross-document edges: {cross_doc} of {total_edges} "
      f"({100*cross_doc/total_edges:.0f}%)  <- the ones expected to be content-distant")

# Density is the standing caveat on this corpus, so it is reported rather than left to be
# recomputed. Live memory-writing runs about 0.44 edges/memory; design documents cross-reference
# more than memory-writing does, so this is a known bias rather than an inflated one -- links were
# written only where the prose already made the reference. It has grown as the corpus has, because
# the later documents were written largely to correct each other.
incident = {}
for _, key, _, links, _ in records:
    incident.setdefault(key, 0)
    for _, tgt in links:
        incident[key] = incident.get(key, 0) + 1
        incident[tgt] = incident.get(tgt, 0) + 1
isolated = sum(1 for k in incident if incident[k] == 0)
print(f"density: {total_edges/len(records):.2f} edges/memory "
      f"(0.44 in live writing -- see the density caveat on recall-eval-harness)")
print(f"isolated counting both directions: {isolated} of {len(records)}")

violations = [(s, l, t) for _, s, _, links, _ in records for l, t in links
              if l != "RELATED_TO" and order[s] < order[t]]
print(f"outbound-rule violations (source older than target): {len(violations)}")
for s, l, t in violations:
    print(f"  {s} --{l}--> {t}")
