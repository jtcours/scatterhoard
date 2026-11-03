# Scatterhoard Design — Index

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

This is the entry point for the Scatterhoard design documents. The design used to live in a single file (`Scatterhoard_Design.md`); it is now split by subject so each topic can be found, linked, and edited independently. `Scatterhoard_Design.md` now just points here.

## Standing principles

- [The Golden Rule](golden-rule.md) — a standing input to convention drafting and memory creation; load it before drafting or revising [Conventions](conventions.md)
- [Decision Placement](decision-placement.md) — put each decision where both the information and the stake live; apply it before adding any parameter, field, or required choice

## Subjects

- [Language and Runtime](language-and-runtime.md) — implementation language, offline/local-first constraints
- [Trigger and Loading](trigger-and-loading.md) — how an LLM session discovers and activates Scatterhoard
- [Components](components.md) — the MCP server's internal library breakdown
- [Memory Data Model](memory-data-model.md) — what a memory is and the fields it carries
- [Memory Steering](memory-steering.md) — routing memories to the correct set (personal vs. project vs. cross-project)
- [Corrections](corrections.md) — which of supersede, amend and forget is proper when a memory needs changing
- [Export Review Gate](export-review.md) — the export-time backstop check before memories reach a project's repo
- [Prohibited Data](prohibited-data.md) — sensitive data the system must keep out of memories
- [Cross-Set References](cross-set-references.md) — links between memories that live in different sets
- [Recall Mechanisms](recall-mechanisms.md) — semantic similarity, keyword search, direct reference
- [Embeddings and Vector Storage](embeddings.md) — embedder choice, why vectors are local derived state, index strategy
- [File Format](file-format.md) — the on-disk/export format for memory sets
- [Set Lifecycle](set-lifecycle.md) — how a set is created, exported, imported, restored and dropped *(draft)*
- [Load Modes](load-modes.md) — read-write, read-only and evaluation: how a machine holds a set, and what each mode refuses *(proposal)*
- [Multi-Set Sync](multi-set-sync.md) — importing, merging, and reconciling memory sets across machines and branches
- [Store Registry](store-registry.md) — where a set's database lives and how a scope resolves to one *(proposal; would re-architect the two above)*
- [Conventions](conventions.md) — the hard-coded operating conventions loaded into an LLM session
- [Tool Surface](tool-surface.md) — the MCP tools Scatterhoard exposes
- [Asking the Human](asking-the-human.md) — how the server gets an answer only the human has, such as the name they write under, without the model supplying it *(proposal)*

## Open Questions

Unresolved design questions pulled from across the subject files live in [Open questions](../../TRACKING.md#open-questions), so they don't get lost when scattered across documents.
