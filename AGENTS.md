# Scatterhoard — Working Notes for Coding Agents

This repo *is* Scatterhoard. **The tool works and is attached to this session** — see [SETUP.md](SETUP.md) — so a session working here uses Scatterhoard on Scatterhoard, and the memory sets are the project's own record. `checkpoints/` still carries session continuity, which memories deliberately do not: a checkpoint is a working note about where the work stands, and a memory is a curated claim meant to outlive it.

## Load the conventions first

**When the Scatterhoard tools are available in this conversation, call `hoard_conventions` before producing any visible text.** It is the first action of the session, ahead of reading the checkpoint or anything else. Take no arguments, and the whole catalog arrives in one page; if you do pass `max_chars`, keep calling as each page's last line says until it says every convention is loaded. **Load them again whenever the conversation has been summarized or compacted since you last did**: a summary standing in for earlier turns means the catalog is no longer in front of you.

**Then** read the most recent checkpoint, per the section below. The two are ordered rather than competing: the conventions say how to use the memory at all, the checkpoint says where the work stands.

The conventions govern which memories are worth storing, which set each belongs in, and when to search before acting instead of assuming. **Loading them after the first store or recall is late in a way that does not recover**: a memory filed to the wrong set cannot be taken back out of a shared repository, and a session that assumed where it should have recalled has already acted on the assumption.

**This is deliberately not left to the tool's own description**, which says the same thing. A description is read when a model reaches for the tool — which is after it has decided to use it, and the decisions the conventions govern are made before that. This is the project's own argument about refusals turned on its own process: where a mistake is not recoverable, arrange things so it cannot be made quietly, rather than labelling it and relying on care.

## Session checkpoints

- `checkpoints/` holds dated session-checkpoint files (`YYYY-MM-DD-HHMM.md`). **The stamp is UTC** — `date -u +%Y-%m-%d-%H%M`, which is what [scripts/precompact-checkpoint.sh](scripts/precompact-checkpoint.sh) writes. The filenames are the only ordering these files have, so one local stamp among UTC ones does not merely look odd: on 2026-09-07 a mid-session checkpoint named in local time sorted *before* the checkpoint that superseded it, which is how a session picks up the wrong one. It's gitignored: checkpoints are working notes, not curated project memories, and shouldn't flow into any branch — same personal-vs-project distinction the design draws in [documents/design/memory-steering.md](documents/design/memory-steering.md), just applied to this project's own process.
- **At the start of a session** working on this project, check `checkpoints/` for the most recent file and read it before doing any work — second only to loading the conventions above — so work can resume where it left off.
- **At the end of a session, whenever the user indicates they're wrapping up, or whenever they ask for one mid-session for safe-keeping**, write a new checkpoint summarizing: what was decided (with links to the design docs that changed), what's still open, and the concrete next step to pick up with. **A checkpoint written mid-session says the work so far is safe, not that the session should end** — the instruction used to name only the end, and on 2026-09-16 that inverted: after two safe-keeping checkpoints a session began offering stopping points nobody had asked for. Flag a clean boundary if it is useful; whether to continue is the user's call.

- **Re-derive every count you put in a checkpoint, and say how.** A number written into a handoff is a claim nothing re-checks, so it drifts and is then believed — and a wrong count reads as *lost coverage*, sending the next session to look for a defect in the code when the defect is in the prose. This has happened: the 2026-08-23 checkpoint reported 191 mutations when the file held 187, and disproving it cost a verification cycle. Run the command rather than carrying the figure forward:

  ```bash
  cargo test 2>&1 | grep -E "^test result" | awk -F'[ ;]+' '{p+=$4; f+=$6; i+=$8} END {print p" passed, "f" failed, "i" ignored"}'
  ```

  **`[ ;]+`, not `[ ;]`** — and the difference is not cosmetic. A single-character class makes each `; ` two separators with an empty field between, so `$6` and `$8` land on the words `failed` and `ignored` rather than on their counts, and the command prints **`0 failed, 0 ignored` whatever the run did**. It read correctly from the day it was added (`4c57c07`, 2026-08-23) because both counts were genuinely zero; the first run with a parked test, the next day, is what exposed it. A counting instrument that cannot report the number it exists to report is worse than no instrument, because its answer is the reassuring one.

  ```bash
  python3 scripts/mutate.py --list | awk '{print $2}' | grep -E '^(caught|survives|compiler)$' | sort | uniq -c
  ```

  **`compiler` is in that alternation because leaving it out undercounts silently.** It is the
  third `expect` value, added 2026-08-26 for a mutation the type system stops rather than a test —
  and a `grep` that predates it drops those entries from the total while still printing a
  confident-looking number. That is the same shape as the `[ ;]+` bug above: a counting instrument
  that cannot report one of the things it counts.

  **A count is not coverage, and this is where the two part company.** A full pass now reports how
  many mutations the *compiler* stopped rather than a test. Anything above zero that is not
  recorded `compiler` means the corpus is claiming coverage the suite does not have — seven of 243
  were doing exactly that when the report was added, all of them replacement text that had rotted
  while its `find` text still matched, which is a class `--stale` structurally cannot see.

  The runner's own `--list` is authoritative over a grep of [scripts/mutations.py](scripts/mutations.py), because a grep counts entries the runner might not load. If a count ever looks like it *fell*, check it against git before assuming anything was lost — `git show <ref>:scripts/mutations.py` at the commit the old checkpoint describes — and compare the **names** rather than the totals, since a matching total can still hide a swap.

## Design documents

Start at [documents/design/index.md](documents/design/index.md). All design docs carry a Vision Constraint notice — load [documents/Scatterhoard_Vision.md](documents/Scatterhoard_Vision.md) before proposing any change to the design. The vision document itself may not be modified without the human partner's explicit approval.

Two standing principles apply across the design rather than sitting in one subject file:

- **Before drafting or revising convention text**, load [documents/design/golden-rule.md](documents/design/golden-rule.md). It's a standing input to every convention, not a one-time read.
- **Before adding any parameter, field, or required choice**, apply [documents/design/decision-placement.md](documents/design/decision-placement.md) — put each decision where both the information and the stake live.

The convention set is kept small deliberately, but **not for the reason the scope note gives** — its *until the basic system works* precondition expired on 2026-08-12, when three conventions shipped against it. What binds now is the [character budget](documents/design/conventions.md#resolved-the-budget-is-18000-characters), to be worked towards rather than filled, and the catalog is close enough to it that the next convention should expect to make its case rather than find room. Derive the current figure; never quote one from a document.

## Coding standards

[STANDARDS.md](STANDARDS.md) — notably, test-driven development is required wherever practical.

**Run `cargo fmt --check` at the *start* of a session, not only before committing.** `cargo fmt`
rewrites every file it disagrees with, so on a tree that was left unformatted it adds the whole
accumulated backlog to whatever you happened to be working on: 433 hunks across twelve untouched
files on 2026-09-11, which all but landed inside a commit about the query log, where a reviewer would
have had to find sixty real lines among 3,800. Run first, the check-only form names those files in a
second and you can put them back; run afterwards, you are reverting by hand from memory, and the diff
is unreviewable until you do. `the-committed-tree-is-not-rustfmt-clean` in
[Issues](TRACKING.md#issues) is the instance, and a reformat is a commit of its own — never a passenger.

**Nothing executes that rule.** There is no CI and no remote here, so this paragraph *is* the
enforcement — and a written rule is precisely what failed the first time, since the standard requiring
the formatter before each commit had been in [STANDARDS.md](STANDARDS.md) throughout. If the tree
drifts unformatted again, the answer is a pre-commit check wired per machine, as this project already
wires its hooks, rather than a firmer sentence here.

**Mutation testing is how this project proves a test covers a guard.** Add a mutation to
[scripts/mutations.py](scripts/mutations.py) whenever a new guard is worth proving, and give any
mutation that survives a written reason. **Run `python3 scripts/mutate.py --stale` after any rename or
refactor**, and before trusting a mutation count, and `--self-check` after any change to `mutate.py`.
How the runner behaves, and what to suspect when a mutation survives or a pass is slow or interrupted,
is in [CONTRIBUTING.md](CONTRIBUTING.md#mutation-testing): read it before the first pass of a session.

**Before shipping text a model reads at the moment it acts** — a tool or parameter description, a refusal, a line a response ends with — consider testing it on a small local model with [scripts/local_model.py](scripts/local_model.py) ([CONTRIBUTING.md](CONTRIBUTING.md#testing-served-text-on-a-small-local-model)). Reading it yourself is not the test: on 2026-10-01 a paging line that looked plainly right was ignored by `gemma4:12b` every time, and a variant that worked under one tool name failed under another.

**Cross-references between documents must not use line numbers** — see [the rule in STANDARDS.md](STANDARDS.md#cross-references-between-documents-must-not-use-line-numbers). Run `python3 scripts/checklinks.py` after editing documentation to catch a link whose target moved.

## Closing a change

**Store everything before the review, and review once.** When a change's code is finished, close it
in this order:

1. Get the tests, mutations, `cargo fmt --check` and clippy green.
2. Store every memory the change owes: the `capture-before-ending` list, the decisions made along the
   way, and the [TRACKING.md](TRACKING.md) heads it changes, such as a backlog entry superseded as built.
   The code is built once the checks pass; the commit only records that.
3. Review the set once, over all of it.
4. Export it, and read the diff and the description.
5. Run `genindex.py` and `checklinks.py`. `genindex.py` reads the exported file, so it comes after the
   export.
6. Commit.

**Anything learned after the commit waits for the next one.** It stays in the store, and the next
session start's `review_due` lists it. So don't write a memory that can only be true after the commit:
name a commit by its date and subject, never by a hash.

**This order exists because the reverse never ends**, as on 2026-10-26: storing "the gate is built"
after the export needed a second review and export, and the memory recording that would have needed a
third.

## Running it

[SETUP.md](SETUP.md) — how to attach the MCP server to a client, with a smoke test to run before touching any config. Start there rather than reconstructing the invocation.

## Lessons

This project draws on the `coding-lessons` memory set, if `hoard_sets` status lists it. Before each change that adds or alters behaviour — a new function, file, data format, check, refusal or test, but not a rename, a reformat or moved code — write one sentence saying what you are about to write, and recall it once: `hoard_recall` with that sentence as `query`, no `scope`, `ignore_floor: true`, `detail: "summary"` and `limit: 5`. Where a returned line is a lesson or a trigger for what you are about to do, look it up with `hoard_lookup` and write accordingly. When you find a mistake that a lesson covers, or that one should, recall the keyword `coding-lessons-adoption-guide` and do what it says. If the set is not loaded, work without it and say so once, so the user can decide whether to add it.

This project is also measuring whether the habit pays: record what it catches in the `scatterhoard` set under `adopting-coding-lessons`, as the `coding-lessons` entry in [EARLY-VALIDATION.md](EARLY-VALIDATION.md) describes.

## Tracking work

[TRACKING.md](TRACKING.md) holds everything the project has decided to do, has not yet decided, or
has got wrong: open questions, backlog and issues, in one file with a generated index each, and a
fourth ranking the common causes behind the issues. **Read it
before filing anything, and file as you go** rather than carrying an observation in the session and
hoping it reaches the checkpoint. Which of the three an entry belongs to, how to form its slug, how
the memory cluster behind a row is shaped, and how to regenerate the indexes all live there.

The fourth state — decided and buildable but **not yet proven** — is still its own hand-written file,
[EARLY-VALIDATION.md](EARLY-VALIDATION.md), because its entries have never been migrated into memory
clusters.

**The prompting for all of this used to sit here**, in three sections and a table; it moved into
[TRACKING.md](TRACKING.md) on 2026-09-08, so that the rules for filing an entry live in the file the
entry goes into. The cost is discovery, and this pointer is the whole of the mitigation: a session
reads `AGENTS.md` at the start and `TRACKING.md` only once it knows to, so if filing ever stops
happening, suspect this paragraph before suspecting the rules it points at.
