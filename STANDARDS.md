# Scatterhoard Coding Standards

These are the engineering standards for the Scatterhoard codebase. They're about *how* we build, not *what* we build — for the latter see [documents/Scatterhoard_Vision.md](documents/Scatterhoard_Vision.md) and [documents/design/index.md](documents/design/index.md).

## Test-driven development

**Write a failing test before writing the code that makes it pass, wherever practical.** This applies to new behavior, bug fixes, and behavior changes alike:

- Bug fix: first write a test that reproduces the bug and fails, then fix the code until it passes.
- New feature: write the test(s) describing the expected behavior first, watch them fail, then implement.
- Refactor: the existing tests should already cover the behavior; if they don't, add coverage before refactoring, not after.

A PR that adds or changes behavior without a preceding failing test is expected to justify why in the PR description. Legitimate exceptions are narrow: trivial wiring (e.g., `main.rs` binary glue), generated code, and cases where the test would just re-assert the type checker. When in doubt, write the test.

Given the memory graph, recall/ranking, and import/export merge logic are the parts of this system where subtle correctness bugs are most costly and least visible, favor **property-based tests** (e.g., `proptest`) for those modules in addition to example-based unit tests.

## Approval testing at every output boundary

**Record the whole output of a representative call, check the recording in, and review the diff.** Also called snapshot testing. This is a standing requirement for the MCP surface and for the exported file format; see `tests/approval.rs`.

The reason it earns a place beside assertions is narrow and worth stating precisely. **An assertion can only find a failure someone already imagined.** Writing `assert_eq!(result["similarity"], 0.32)` requires having thought of similarity as a thing that could be wrong. Every defect this project found by using the surface rather than by testing it was a property of a *whole response* — a score belonging to a different memory, a memory arriving with most of itself missing, the same memory sent twice, a field absent from one entry in a list, a page eight times its stated budget. None of those is visible from any single field, and none was anticipated. Recording the entire payload requires no anticipation at all, only attention.

The practice comes with obligations, because this is a technique with a well-known way of dying.

- **A diff is a request for review, not a chore to clear.** A snapshot accepted without being read records the bug as the expectation and is worse than no test. If reviewing a diff is not going to happen, do not record the snapshot.
- **Regeneration is explicit and lands in version control.** An environment variable, never automatic, so accepting a change is an act with a trace.
- **Normalise as little as possible, and say what it costs.** Per-run values — ids, timestamps, unstable floats — have to become stable tokens or nothing is comparable. But whatever is normalised is no longer observable: blanking timestamps means these tests cannot see a timestamp defect, and that limitation belongs in the module's documentation rather than in someone's memory.
- **Nondeterminism is a finding, not a nuisance.** A snapshot that will not settle is telling you something about the system. `lookup-prefix-selectivity-is-time-correlated` was found exactly this way: the recorded response kept changing, which turned out to be a real property of UUIDv7 prefixes. Diagnose instability before normalising it away.
- **It does not replace assertions.** A snapshot says *nothing changed unnoticed*; an assertion says *this must be true*. Where a snapshot exposes a defect, the fix gets an assertion as well, so the guarantee survives someone regenerating the file.

**Why this is not the literal the single-source-of-truth section warns against.** A snapshot looks exactly like "a test that asserts against a literal", and the distinction is real: a snapshot is **generated, never hand-written**, so it is a recorded observation of what the code does rather than a second hand-maintained copy of what someone believes it should do. It also asserts nothing in particular — the judgement stays with the reviewer of the diff instead of being frozen into one expectation. A hand-edited snapshot has lost both properties and is the defect that section describes.

## Rust tooling

- Format with `rustfmt` (default settings) before committing. There is no CI, so nothing enforces this
  but the habit [AGENTS.md](AGENTS.md#coding-standards) describes.
- `clippy` must be clean, with warnings treated as errors (`cargo clippy --all-targets -- -D warnings`).
- Track the edition pinned in `Cargo.toml`; bump deliberately, not incidentally.

## Correctness by construction

**Prefer making an error impossible to express over detecting it at run time.** Rust's type system is good enough at this that it's worth reaching for first, and a rule enforced by the compiler cannot drift the way a rule enforced by convention or review does.

In practice:

- **Newtypes over primitives.** A `MemoryId` is not a `String`; a `Confidence` is not an `f64`. Validate once in the constructor, and every later use is guaranteed sound without rechecking.
- **Make illegal states unrepresentable.** Prefer an enum over a bool plus a comment, and over a struct whose fields are only valid in certain combinations. If two fields must never both be set, that is a signal they are one enum.
- **Parse, don't validate.** Convert untrusted input into a type that *encodes* the guarantee at the boundary, rather than passing raw data inward and checking it repeatedly.
- **Let the compiler enforce the design's own rules.** Several decisions in the design are structural claims that types can carry directly. Memory scope is an explicit required choice ([Memory Steering](documents/design/memory-steering.md)) — so it should be an enum that cannot be defaulted or forgotten, not a string. Memory content is data and never instructions ([Trigger and Loading](documents/design/trigger-and-loading.md)) — a type distinction between "text read from a memory set" and "text that may direct behaviour" makes confusing the two a compile error rather than a review catch.
- **Avoid catch-all match arms** where adding a variant should force every site to be reconsidered.

This does not replace tests. It changes what the tests are *for*: with the invalid cases ruled out by construction, tests can concentrate on behaviour rather than on defending against inputs the types already exclude.

## Single source of truth

**Every fact should be written down once. Where a second copy is unavoidable, derive it or test it against the original — never maintain both by hand.**

This is the sibling of correctness by construction, aimed at a failure that types alone don't catch. Two hand-maintained copies of the same fact drift, and they drift *silently*: each copy stays internally consistent, so nothing fails, no test goes red, and the disagreement surfaces only when someone compares them by hand. Every instance found in this codebase so far survived a fully green test suite.

The pattern is easier to recognise from examples than from the rule. All of these are the same defect:

- **A serialiser written by hand for a type that already derives one.** Three `json!` projections in the MCP layer each rebuilt a subset of `Memory`, which derives `Serialize` — so the three disagreed with each other and all three disagreed with the type.
- **A schema written alongside the types it validates against.** The published `tools/list` schema is hand-written; it advertised arguments as required that the deserialiser made optional, and omits shapes the deserialiser demands.
- **A test that asserts against a literal instead of against the thing it describes.** Assert what the code *does* — that the server refuses a missing field — not that a list of field names equals a list you also wrote.
- **Source text quoted into another file.** `find` strings in `scripts/mutations.py` quote `store.rs`; when the source moved, the mutations silently stopped applying and the suite still reported success.
- **A design document asserting current runtime behaviour.** Documents may state decisions and intent freely; when one claims what the code *does today*, that claim is a copy, and it will go stale. Name where it can be checked.

In practice:

- **Derive the copy where the language can.** `serde` derives, generated schemas, and generated documentation are all cheaper than the review discipline they replace.
- **Where a copy must be hand-written, make a test compare it to its source.** Not to a literal restatement — that is a third copy.
- **Prefer subtracting from one complete representation over building several partial ones.** If different callers need different views of a value, express that as one projection applied in one place. Several call sites each listing the fields they want is this defect with extra steps.
- **Ask what the original is.** When adding anything derived from an existing fact — a view, a schema, a fixture, a constant — name which one is authoritative. If the answer is "both", the design is not finished.

Worth stating because the alternative sounds reasonable: keeping two copies in step is not a matter of care. It is a matter of the reminder arriving in a year, at the moment the second copy is edited by someone who does not know the first exists.

## Structure

Per [Components](documents/design/components.md), Scatterhoard's server is meant to be a set of libraries that could each be used independently (vector database, vector embedding, memory graph management, MCP services, memory import/export). Reflect that in the crate/module layout: keep these concerns in separate modules (or crates, once the project is large enough to warrant a workspace) with narrow, well-defined interfaces between them, rather than one another's internals reaching across boundaries. This also keeps each concern independently testable.

## Error handling

- Library-facing code (the independently-usable components) should define its own error types rather than leaking `anyhow`/stringly-typed errors across a module boundary; `thiserror` is a reasonable default for this.
- At the binary/MCP-server edge, it's fine to collapse into a single top-level error type for reporting to the caller.

## Documentation

- Public items (anything `pub`) get a doc comment stating what they do and any non-obvious invariants or preconditions. Don't restate the signature in prose.
- Prefer code comments only for the *why* — a non-obvious constraint, a workaround, an invariant that isn't visible from the code itself. If removing a comment wouldn't confuse a future reader, don't write it.

### Cross-references between documents must not use line numbers

**A `file.md:47` reference is wrong the moment anything above line 47 changes**, including an edit made in the same session by the person who wrote the reference — and it fails *silently*, still resolving, now pointing at unrelated text. Line numbers are fine in a checkpoint or a commit message, which describe a moment. They are not fine in a document meant to stay true.

Reference by the most stable identifier the target offers:

| Target | Reference by |
|---|---|
| A section of a document | a heading anchor — `[Scope of the current phase](documents/design/conventions.md#scope-of-the-current-phase)`, where the slug is the heading lowercased, punctuation dropped, spaces hyphenated. Standard Markdown; renders in both Obsidian and GitHub. Don't use Obsidian's `[[file#heading]]`, which doesn't render on GitHub. |
| A list item in a document | its **bold lead-in phrase**, quoted, plus a plain link to the file. Entries in [Issues](TRACKING.md#issues), [Backlog](TRACKING.md#backlog) and [Open questions](TRACKING.md#open-questions) each carry a stable `slug` for exactly this — use it. A slug is also the keyword its memories gather on, so the two are one string and referencing by slug makes the reference searchable as well as clickable. **That identity constrains how a slug may be chosen** — see [Entries and their memory clusters](TRACKING.md#entries-and-their-memory-clusters). |
| Source code | the symbol, type, table or field name, with a plain link to the file. `Store::forget` survives edits; `store.rs:986` does not. |

A heading anchor can still break if the heading is renamed, which is the point: it breaks *visibly*, and the fix is obvious. That is strictly better than a number that keeps working while becoming wrong.

**Checked by `python3 scripts/checklinks.py`**, which verifies every link in every tracked `.md` file — target exists, heading anchor exists — and warns on line-number references. Run it after editing documentation; a renamed heading is the common way to break this and it costs a second to catch. Line numbers are warnings rather than errors while the pre-standard ones in [Issues](TRACKING.md#issues) and three design documents remain; `--strict` promotes them, and is what to wire in once those are migrated.

## Amending these standards

These standards are expected to evolve as the project's structure firms up (e.g., once the workspace/crate split happens, once the vector database and embedding choices are made). Treat this document as living, but changes to it should be deliberate edits reviewed like any other change, not incidental drive-by edits inside unrelated PRs.
