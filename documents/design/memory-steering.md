# Memory Steering

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

Scatterhoard needs to provide guidance to the LLM to route memories to the proper memory set or project.

- We don't want personal memories to leak into an open source project's public repository, and we don't want memories that are important to a project to get filed in a personal memory set where they're inaccessible to the project itself.

## Resolved: explicit scope at store time

The software itself doesn't have enough intelligence to steer a memory to the right set. So: **the LLM must explicitly direct the store tool as to which memory set/scope to use** for every memory (e.g., a required `scope` parameter on `hoard_memorize` — see [Tool Surface](tool-surface.md)). This is not inferred implicitly from context. The conventions provide the guidance the LLM uses to choose correctly.

Scope is single-valued per memory. If the same memory is genuinely relevant to more than one project, that's handled as an explicit, separate cross-reference/duplication operation (see [Cross-Set References](cross-set-references.md)) rather than a memory carrying multiple scope tags — this is expected to be rare.

### Representation

```rust
enum Scope {
    Shared(SetName),
    Local,
}
```

Three properties of the design are carried by the type rather than by discipline (see the correctness-by-construction section of [STANDARDS.md](../../STANDARDS.md)):

- **`Shared` carries the set name.** A shareable memory that doesn't say which set it belongs to cannot be constructed. Several sets are loaded at once (see [Multi-Set Sync](multi-set-sync.md)), so the name is never redundant. The variant was called `Project` until 2026-08-04; it was renamed because a shared set needs a name and an export destination but *not* a repository, and a cross-project set of coding lessons made the old name read as a claim the type never made (see [Set Lifecycle](set-lifecycle.md)).
- **No `Default` implementation, ever.** That is what enforces "explicit, never inferred" above. Deriving `Default` would silently reintroduce exactly the inference this decision rules out.
- **Single-valued by construction** — one enum field, not a collection. The rule above becomes untypeable rather than merely discouraged.

Export eligibility follows from the type instead of living in a separate check: only `Project` has an export target, so a method along the lines of `fn exports_to(&self) -> Option<&SetName>` puts "local memories never leave the machine" in the type system rather than in a rule someone has to remember at each call site.

## Resolved 2026-08-21: `local` is a reserved set name

A shared set could be called `local` until this date, and two tests said so in passing. It cannot now: [`SetName::new`](../../src/memory/scope.rs) refuses the word, case-folded.

The reason is that the scope grammar had already spent it. `local` is the scalar spelling of `Scope::Local`, so a shared set of that name is a *second* set answering to the same word — and the two are reachable through different fields of one call, which is how the confusion actually showed up: `scope: "local"` meant the local set while a link's `target_set: "local"` meant the shared one, so a caller linking to their own local memories got a link that silently pointed elsewhere (`target-set-local-names-a-different-set` in [Issues](../../TRACKING.md#issues)).

**Refused at the constructor rather than per field**, because the ambiguity has four doors — `hoard_sets create`, a memory file's header, the `shared:local` scalar form, and a link's target set — and a check placed at any one of them leaves the others. This is the [correctness-by-construction](../../STANDARDS.md) preference applied to a name: unrepresentable beats detected.

**One word only.** `shared` needs no reservation: the scalar form is `shared:<name>` and the first colon is always the separator, so `shared:shared` reads exactly one way.

*What was given up is small and worth naming: a repository genuinely called `local` cannot have a memory set of the same name and must pick another. The two tests that had asserted the old permission now pin what they were really protecting — that no shared set's store file can collide with the local store's — which does not depend on that word.*

**A local set is still written to a file**, for backup and restore — see [File Format](file-format.md). That is a different operation from export and does not go through `exports_to`, which continues to answer `None` for `Local`. Serialising a set and being allowed to publish it are separate questions, and only the second one is what this method decides.

### What a set name may be

The natural set name is the project's own, so the permitted shape is chosen to accept names projects actually have: ASCII letters and digits, plus `-`, `_` and `.` **inside** the name only, up to 100 characters. Requiring the first and last character to be alphanumeric is a single rule that keeps `.`, `..`, and leading-dot names unrepresentable — path traversal is excluded by construction rather than filtered for, which matters because a set name keys a store on disk and arrives over the same untrusted channel as the memories.

An earlier, more defensive rule excluded `.` entirely and rejected most real dotted project names (`next.js`, `docs.example.com`, `asp.net-core`). The set name **defaults to** the project's name rather than being defined as it — the header declares it explicitly, so a project whose name still does not fit can declare a different one. That keeps every host's naming rules from becoming Scatterhoard's problem.

Names are neither trimmed nor case-folded, since a set name keys a store and is compared for equality when resolving cross-set references, so two spellings must not silently become one.

Scope also partitions storage: each scope gets its own store rather than sharing one table with a scope column, so a query against one project's memories structurally cannot return another scope's. See [Embeddings and Vector Storage](embeddings.md#storage-is-partitioned-by-scope).

### Why `Local` is one variant, not two

The Vision distinguishes personal memories from general memories unrelated to a specific project, and an earlier draft of this design gave each its own variant. That was dropped: nothing downstream reads the difference. Both stay local, neither is exported, and the [Export Review](export-review.md) gate inspects project-scoped memories rather than local ones — so the split would have cost a judgment call on every store and bought nothing. It can be reintroduced if something ever needs to tell them apart. See [Decision Placement](decision-placement.md).

## Resolved: export-time gate as a backstop

Because a personal memory leaking into a public project repo is effectively a one-way door (hard to undo once committed and pushed), store-time scope selection alone isn't enough. A second check happens at export time, before memories are committed into a project's repository — see [Export Review Gate](export-review.md) for the full mechanism (mechanical prohibited-data scan + a judgment-based gate scoped to the local user's own edits, backed by a ledger so already-cleared memories aren't re-litigated on every export).
