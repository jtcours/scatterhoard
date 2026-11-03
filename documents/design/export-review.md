# Export Review Gate

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

This is the backstop half of [memory steering](memory-steering.md): even though scope is chosen explicitly at store time, an export-time gate catches mistakes before memories reach a project's public repository. A misfiled *personal* memory is a one-way door once it's committed and pushed, so it's worth a second check at the point of export rather than trusting the store-time choice alone.

It runs as part of [`export-before-commit`](conventions.md#served-routing-capture-and-export), at the point where a `git diff` of the memory file(s) for this export-and-commit cycle is available. *(This named the convention by its old one-line description, "expose a fresh set of memories" before committing — a phrase that read as though the convention also produced the memories. It does not, and this document depending on the export reading is part of what prompted the split.)*

## Two independent checks

### 1. Mechanical prohibited-data scan

A deterministic scan (e.g., regex/pattern matching) for the content categories in [Prohibited Data](prohibited-data.md): secret API keys, passwords, credit card numbers.

- Runs on **the full exported set**, every export cycle.
- **Not** scoped by authorship or by the ledger below. Whether a secret leaked in because of something the local user wrote or because of something merged in from elsewhere doesn't change the risk of committing it — this check is cheap enough to run unconditionally.

### Proposed 2026-08-06: this check belongs in `export`, not at the pre-commit gate

The two properties above mean this check **depends on nothing the pre-commit moment provides.** It reads the whole set, ignores the ledger, and ignores authorship — so unlike the judgment gate it never needs the `git diff`. Everything it requires is in hand while `export` is serialising the records. It was grouped with the other two checks by proximity rather than by requirement.

Moving it into `export` has a concrete consequence: `"prohibited_data_scanned"` stops being the hard-coded `false` it is today (`Server::export_set` in [mcp.rs](../../src/mcp.rs), pinned by a test that exists precisely because the scan does not) and becomes a fact about the export that just ran.

**Whoever builds it inherits a companion field.** Since 2026-08-11 the response also carries `"prohibited_data_note"`, because `false` was true for two unrelated reasons and said which was which for neither: a local export skips this gate *by design*, having nothing to publish, while a shared export skips it only because no scan exists. That distinction gets sharper rather than redundant once the scan lands — a shared export that genuinely went unscanned must not read like a local one that never needed scanning — so the note is not scaffolding to remove. `prohibited-data-scanned-conflates-skipped-with-unbuilt`.

**It also keeps the check in Rust.** Scatterhoard is meant to install as a self-contained binary, and putting an operational check in a script would make a scripting runtime a requirement for ordinary use — a different thing entirely from `scripts/mutate.py`, which is a development tool the users of a project never run.

**What moving it does not cover** is a hand-edited `memories.json` committed with no re-export. That is a real path rather than a hypothetical: renaming a forked set is specified as a hand-edit of the header ([Set Lifecycle](set-lifecycle.md)). The answer keeps the same constraint — expose the scan as a subcommand of the Scatterhoard binary, so a pre-commit hook is one line of shell invoking something already installed, rather than a program in another language. **Built 2026-09-14** as `scatterhoard scan FILE...`, with `-` for standard input so a hook scans the staged blob rather than the working file; the hook itself is in [SETUP.md](../../SETUP.md). It scans raw lines rather than a parsed set, because the file a hook meets may not parse, names each finding's line, category and memory id but never what matched, and exits `1` on a finding and `2` when it could not scan — so a hook can tell the two apart. It touches no store and loads no model. The bare invocation still serves MCP, pinned by a test written before any argument handling existed. `prohibited-scan-subcommand` in [Backlog](../../TRACKING.md#backlog); `export-subcommand` is filed to follow it through the same argument handling.

*Verified manually on the first export of the `scatterhoard` set, 2026-08-06, by running an equivalent scan by hand: the set was clean, and the scanner was checked against planted secrets first so that "clean" meant something.*

### Built 2026-08-20 — and it refuses at the *write*, not here

The scan is in `src/prohibited.rs`, mechanical and with no LLM in it: known vendor key prefixes with a minimum key-character run, digit runs of 13–19 that satisfy Luhn, and PEM armour lines. Nothing matches on vocabulary, which is a requirement rather than a refinement — this project's own `route-where` convention names API keys, passwords and card numbers in one sentence, so a word-matching scan would refuse Scatterhoard's own export on Scatterhoard's own rules.

**Where a match is refused turned out to be the real decision, and it is not here.** `hoard_memorize` and `amend` refuse: the caller is writing one memory that does not yet exist, it is definitionally theirs, and the remedy `route-where` already prescribes — put the value in a file and record where the file is — is one action away while the context is live.

Export only warns, for two reasons that both cut the same way:

- **A set can hold memories that arrived by import**, and `imported-memories-carry-no-arrival-provenance` means the exporter cannot tell which. Refusing would ask the caller to amend or forget somebody else's record to unblock their own work — in a set where that edit is export-visible and goes to a maintainer.
- **The filter gains rules over time.** Adding a vendor prefix is an ordinary improvement, and under a blocking export every such improvement could close a set that exported fine the day before — taking every memory written since with it, because export writes the whole set or none of it.

The general form is the same rule the link-target check settled on hours earlier: **refuse where the caller holds the remedy; report where they may not.** At a write they always do; at an export they may not.

**`prohibited_data_scanned` is now `true` and the note carries what the boolean cannot.** The failure mode of a shape-based scan is misses, not false alarms: an unfamiliar key format is not recognised, and a password has no shape to recognise at all. So the note says what was looked for, and says that clean means those shapes are absent rather than that the set holds no secret. Two tests that previously pinned the hardcoded `false` now pin that wording instead.

*Measured before shipping: zero findings across all 100 records of the two live shared sets, including the memories that name all three prohibited categories explicitly, and every memory id, commit hash and model identity in them.*

### 2. Judgment-based gate (personal content / scope correctness)

An LLM-judgment review of whether a memory's chosen scope still looks right and whether its content reads as something that shouldn't be in this set (e.g., it looks personal, despite being scoped to a project).

- Governed by conventions: a hard-coded baseline set of flagging conditions, extendable/tunable via the project's local *configuration* (`scatterhoard_mem/config/sharing-preferences.json` since 2026-10-22, first proposed as `config.json`; per [Trigger and Loading](trigger-and-loading.md)) so a project can adjust the conditions under which the LLM flags something without waiting on a change to the hard-coded conventions. Configuration rather than prompt text, deliberately: this is tuning against a bounded schema, which keeps it data rather than an instruction channel.
- *Superseded 2026-10-17 by [the review set is the diff against the committed file](#decided-2026-10-17-the-review-set-is-the-diff-against-the-committed-file); kept as history.* **Scoped to memories the local user last modified.** Gate on the memory's *last modifier* (see [Memory Data Model](memory-data-model.md)), not its creator: if the local user created or edited a memory since it was last cleared, it's their responsibility and gets reviewed; a memory merged in from main that the local user never touched has already passed review upstream (its own author's export gate, plus the maintainer's review at merge) and is skipped here to avoid re-litigating it.

#### The skip is conditional on one store serving one repository, and nothing enforces that

Recorded 2026-08-20, tracing what an imported memory is actually exposed to. The justification above is not *this memory was written by someone else*; it is **this memory came in from this project's upstream, and was reviewed there**. Those coincide only while the store this set lives in exports to the same repository it imports from.

Nothing holds that today. One store answers to a set name and any directory may claim that name, so a set can be imported from repository A and exported to repository B. A memory arriving that way is new to B, was reviewed by nobody in B's history, and is skipped by this gate under a rule that believes it was — while the export writes it into B as the local user's own commit. The gate's rationale is what waves it through, so more care at the gate cannot help.

**The fix is upstream of this document.** The two-repository configuration is what the set-name binding in [Open questions](../../TRACKING.md#open-questions) would remove; with one store bound to one repository, *last modifier is someone else* and *came from this project's upstream* are the same set again and the rule stands as written. Tracked from `imported-memories-carry-no-arrival-provenance` in [Issues](../../TRACKING.md#issues), which is the same conclusion reached from the other end.

**Do not repair it by re-scoping the gate to "not previously in this target's file."** That criterion is available today — `Store::bases_for` already records which memories a given target's file last agreed on — and it is the wrong trade: it pulls every memory merged from main into local judgment review, which is exactly the re-litigation the scoping rule exists to prevent. It is worth knowing the check is cheap if the binding is refused, and worth not reaching for otherwise.

Note this affects **only** the judgment gate. The mechanical prohibited-data scan above is deliberately unscoped by authorship and runs on the full exported set every cycle, so the leak risk that matters most is already covered regardless of where a memory came from.

### 3. Unexpected entries in `scatterhoard_mem`

The gate is also the primary place to surface unrecognized files or directories in the reserved `scatterhoard_mem` namespace — version-gated so an older build meeting a newer repository stays quiet. See [Trigger and Loading](trigger-and-loading.md#reserved-namespace-and-unexpected-contents) for the rule and the reasoning; this is where the user actually hears about it, because it's the moment they're about to add to the repository and can act.

### 4. What the set amounts to — derived, shown, and never judged

Built 2026-09-12 and 2026-09-13 under `derived-set-summary` in [Backlog](../../TRACKING.md#backlog). This is the mechanism half of [demand C](golden-rule.md#what-the-principle-is-for-four-demands-it-makes): *a set can amount to something no record in it amounts to*, and no session seeing one record at a time can notice that. What can is a description computed over the whole set and shown where the whole set is in view.

**At export**, the set's stored embeddings are clustered and each cluster is named by the keywords most distinctive to it against the rest of the set (`src/summary.rs`). The export response carries it as `describes`. The export embeds anything missing first, so the description covers every memory rather than whichever happened to have vectors.

**At import it is derived from what arrived**, and returned as `describes`. It describes the set as it now stands, the merge of what was held and what the file brought, since that is what a session will work with.

**The file carries no description, and the comparison against one is retired** — decided with the human partner 2026-09-21, under `how-a-merged-file-gets-a-current-summary` in [Open questions](../../TRACKING.md#open-questions). Until then the header carried the export's description as `summary`, and an import compared its own derivation against it with a verdict, `against_the_claim`. What the reader was shown was the derivation throughout, so nothing it sees changed. The stored copy went because it paid for nothing and cost a great deal:

- **The comparison caught only a sender who edited the claim rather than deleting it.** Deletion left the verdict saying there was nothing to compare, which is unremarkable; an edit gained the sender nothing, because the claim was never shown.
- **Merging made it fire for no reason.** A merge keeps one side's header, so the claim described one side's memories and a fresh import of the merged file reported a difference *nothing here explains* — the benign-firing signal this section's own reasoning says gets ignored, taking the rare real case with it.
- **It made the header the line two branches collide on**, since it changed with every export. A hosted merge button runs no merge driver, so that conflict reached every pull request whose base had moved.

A header written by an older build may still carry `summary`; it is ignored on read, and needs no format bump either way ([memory-data-model](memory-data-model.md)).

**An import that cannot describe what it brought owes the description.** A memory file carries no vectors, so an import onto a machine with no model — or a mismatched one, which leaves arriving memories unembedded — has nothing to cluster the import with. Skipping it would let the gate be defeated by importing first and installing the model afterwards. Instead the store records what is owed, and `hoard_conventions` pays it at the start of every session once the memories are embedded, as `imported_sets_described`, **until the set is next exported** with nothing left unembedded. That export showed the description itself at the moment the set became portable, which is what discharges it; nothing else does, so there is no acknowledgement for a session to give or skip.

**The derivation cannot be stripped by editing the file**, which is why it carries the whole mechanism. Keywords are a user-controlled field and could be chosen to mislead the labels; that is a deliberate act, and defence against a determined actor is not a success criterion here.

**It stands beside checks 1 to 3 rather than inside the judgment gate**, which was one of the choices left open when the work was filed. The judgment gate is scoped to memories the local user last modified, so that merged-in memories are not re-litigated. An aggregate is a property of the whole set, and that scoping would exclude exactly the merged-in memories that compose it.

**It describes and does not judge**: no threshold, no flag, no classifier. A local tool has no telemetry to tune one and no way to measure its false positives, and judging the person behind a set is the inference from evidence to motive that demand D(ii) forbids.

### 5. What the file being replaced did not hold

**Built 2026-09-14** as `export-reports-what-the-file-did-not-hold` in [Backlog](../../TRACKING.md#backlog). Every export compares what it is about to write against the file it replaces, reports the difference as `replaced_file`, and **refuses to write when doing so would lose records**. It stores nothing and adds no field to any record: the comparison is between two files' records by the same canonical checksum reconciliation uses, so it asks a question about files — *what changed* — rather than about authorship, which a per-session identifier was measured unable to answer.

**Why it exists when the diff review already does.** `export-before-commit` has the diff read, and that caught the incident that raised this — but the gate's reasoning is about scope errors the exporting session made itself, and it had never met a record the session did not write. One store serves every session on a machine, so another session's memories reach this checkout's export with nothing marking them. A report makes the surprise announce itself rather than depending on the review being done carefully.

**Three states, not a boolean**, because *no file* and *a file that could not be read* want opposite readings:

- `none` — nothing was at the destination. The report says every record written is new, rather than counting zero, since a zero would read as *nothing changed*.
- `unreadable` — a file was there and could not be read as this set's memory file: malformed, from a newer format, or declaring another set. **The export is refused**, naming the reader's own reason, because what writing over it would lose cannot be told. The likeliest such file is one left with unresolved merge conflict markers, and overwriting that throws away the other side of the merge.
- `compared` — counts and ids in three categories, plus the number unchanged:
  - **`new`**: records the file did not hold. The case the entry was filed for.
  - **`changed`**: records in both whose content differs. An amendment by another session is one; so is a memory forgotten since the last export, whose line has become a tombstone under the same id.
  - **`removed`**: records the file holds and the set does not. **Decided 2026-09-14 by the human partner to track alongside the other two, and the one most likely to cost work**: it is what a pull, or another machine's export to a synced file, looks like when nobody imported before exporting. An export that would remove any is refused, so on an export that went ahead this category is always empty — reported anyway, as the explicit statement that nothing was dropped.

**Refused when writing would lose records, or when what it would lose cannot be told; never when nothing is there.** Decided 2026-09-14 by the human partner, after a first build the same day that only reported. Export replaces the file atomically and never checks the recorded bases against the file on disk, so until then records a pull brought in and nobody imported were overwritten with no sign but lines vanishing from a diff — and for a file that was never committed, they were gone. This departs from export's standing rule of warning rather than refusing (*Built 2026-08-20* above), and by that rule's own test: the rule refuses where the caller holds the remedy, and here they do. The refusal names the records at stake and both ways on:

- **Import from the same destination, then export again**, which keeps the records.
- **Move the file aside, or delete it, if replacing it is intended** — after a `restore`, say, which empties records from a set on purpose. Moving is named first because it leaves a copy of what the overwrite replaced.

**The way through is a filesystem act, not a parameter.** A force option was the other candidate and was left for later. A parameter puts a destructive choice on the call itself, where a caller that has just been refused can simply set it, and it discards what moving would keep. It is also hard to withdraw once copies of the code are in use, where adding one later, if the friction proves real, costs nothing.

**What this does not reach** is a removal that never passes through export: a merge resolved by dropping lines, a hand-edit, a checkout of an older file, a commit of a file nobody re-exported. Those need the comparison run against two files on its own, which is the same shape as `prohibited-scan-subcommand` in [Backlog](../../TRACKING.md#backlog).

**Ids are capped per category; counts are exact.** A format change can alter every record in a set, and a report listing eight hundred ids would bury the few that a reader needs to look at.

## Decided 2026-10-17: the review set is the diff against the committed file

**There is no ledger.** The ledger below was designed when a memory could change in place, so clearing
one meant recording *which version* was cleared. That is no longer possible: a memory that has left the
machine cannot be amended, and a change to it is a new memory that supersedes it ([Corrections](corrections.md)).
Anything worth reviewing therefore has an id the committed file does not hold.

**The committed memory file is the clearance record.** [`export-before-commit`](conventions.md#served-routing-capture-and-export)
already says to export, read the diff, then commit -- so what the file holds at `HEAD` is what has passed
review, and the routine review set is exactly the diff between `HEAD` and the new export. It is shared
through `git`, so it needs no machine-local state and no syncing between one person's machines.

**It also closes the gap [above](#the-skip-is-conditional-on-one-store-serving-one-repository-and-nothing-enforces-that)
that the last-modifier rule left open.** Memories merged from upstream arrive through `git`, so they are
already in `HEAD` and are not re-reviewed, which was that rule's intent. A set imported from one
repository and exported to another shows its memories as new in the second repository's diff, so they are
reviewed there, which that rule could not guarantee.

**The record is `git`, not the store's agreement with the file.** A store records what it and a target
last agreed on (`bases`), and that advances when an export is *written* -- before anyone reads the diff.
Keyed on it, an export that was never committed would quietly mark its memories cleared.

## Decided 2026-10-17: what the review flags

The baseline conditions, applied to each memory in the review set. **A flag is a question for the
human, not a refusal**; the procedure below says who answers it.

1. **A characterisation of a person** -- a judgement about someone rather than an observation that
   serves the work and can be checked. The line the [Golden Rule](golden-rule.md) draws.
2. **Personal details about anyone**: where they live, their employer, health, family, religion,
   sexuality, politics, ethnicity, finances, or anything comparably personal.
3. **A private individual named in a shared set**, other than as the author of their own work.
4. **Machine and account specifics**: user names, home paths, host names, internal addresses.
5. **Someone else's confidential or third-party material**: a client's affairs, an employer's internal
   detail, anything shared in confidence.
6. **Security-sensitive operational detail** -- not a credential, which [the mechanical scan](#1-mechanical-prohibited-data-scan)
   handles, but how to reach or misuse something.
7. **A memory in the wrong set**: personal content filed as the project's, or one project's content in
   another's set -- the scope check this gate exists for.

## Decided 2026-10-17: whose comfort governs

**The person a memory is about decides whether it is published**, because they hold both the
information -- what they are comfortable sharing -- and the stake ([Decision Placement](decision-placement.md)).
People differ: one contributor is proud to name their employer, another would rather it stayed private.
So a flag is routed by its subject:

- **About the person exporting it: theirs to decide**, against their own [sharing preferences](#decided-2026-10-17-personal-sharing-preferences-are-a-markdown-file-on-the-persons-machine).
- **About someone else: out, unless it serves the work and can be checked** -- *jbrown maintains the
  parser* stays, *jbrown works at Acme* goes. The exporter cannot consent for someone who is not there,
  and [`route-where`](conventions.md) already says a shared set does not take people as its subject.
- **The project's floor binds everyone.** A project may rule something out entirely through the
  baseline and its configuration; a person may be stricter about themselves than the floor, never
  looser. *(Revised 2026-10-20: the baseline is now the template a person's preferences start from,
  not a floor under them -- see [the preferences start from a template](#decided-2026-10-20-the-preferences-start-from-a-template-the-person-edits).
  A project's own configuration still binds everyone exporting to it.)*

**Unresolved means out.** Publication is one-way: a detail left out can be added later, by the person
it concerns if they wish, while one exposed cannot be withdrawn from a repository's history.

## Decided 2026-10-17: personal sharing preferences are a markdown file on the person's machine

**Not memories.** This is one of the few things a person writes for Scatterhoard that they must be able
to read back and check at a glance, and a memory set is the wrong place to audit something. A file the
person owns, read by the review, is.

**It never sits in a repository**, because it is itself sensitive: an exception allowing one's religion
in a theology set says what that religion is. It lives with the person's own configuration, outside
every checkout -- its exact location is fixed when the mechanism is built. Being instruction-shaped is
acceptable here for the reason it is not in a memory directory: it is the person's own text, on their own
machine, read by their own sessions, and it travels nowhere.

**Two dimensions, and no more.** Whether a detail is fine depends on what the set is about and on who can
read where it is published -- a person's religion may be unremarkable in a set about their religion's
theology committed to a private repository, and not generally. So the file holds categories to keep out,
in the person's own words, and exceptions naming a set, **each holding only while that set's repository
is private**:

```markdown
# What I share in memory sets

## Keep out of every set
- my religion
- where I live

## Exceptions
- In the islamic-theology set, while its repository is private: my religion.
```

~~The baseline already keeps out the commonly sensitive categories, so the file adds only what is specific
to the person.~~ *Superseded 2026-10-20: the file starts from a template and the person adds or removes
what it holds -- see [the preferences start from a template](#decided-2026-10-20-the-preferences-start-from-a-template-the-person-edits).*
Per-memory overrides and compound rules are deliberately not expressible: anything they
would cover can be cleared by hand when the review flags it.

## Decided 2026-10-18: the preferences reach the session at its start, not only the review

**The server serves the person's file alongside the conventions**, so a session applies it when it
stores a memory, not only when it exports one. Proposed by the human partner while the 2026-10-17
read-through was running. That read-through found that most of what it flagged could have been kept out
by rephrasing one sentence when the memory was written. A rule consulted only at export runs after the
moment it governs, which is the failure `coding-lessons` records for any rule reached by recall rather
than loaded in advance.

**The export review stays**, because two things are invisible at writing time:

- **Aggregation.** No single memory about an association's governing documents was a
  problem. Seventy of them together were. Only a review of the set sees what it amounts to.
- **A change of audience.** An exception holds only while its set's repository is private, so memories
  written under it need a second look when the repository goes public. That is why going public calls
  for a full review.

**The context cost is the person's own, and that is what makes it acceptable** where it would not be
for the catalog. The conventions are served to everyone, so they are held to [the character
budget](conventions.md#resolved-the-budget-is-18000-characters). This file reaches only its author's
sessions, and its author decides how long it is. It does not count against the catalog budget.

*Not yet settled:* how a session applying an exception at writing time knows whether its set's
repository is private. The candidate is to apply the exception as written and leave going public to the
full review, which turns exceptions off anyway.

## Decided 2026-10-20: the preferences start from a template the person edits

**When a person has no preferences file, Scatterhoard copies a template into place, and from then on
the file is theirs.** They add protections, remove them, scope one to a single set, and note whether a
set is published. The seven [baseline conditions](#decided-2026-10-17-what-the-review-flags) are what the
template starts with, not a floor beneath it. Proposed by the human partner, after the full review
showed how much a person had to know to publish safely. They did not want every user to repeat that
review, which is why this is on the release gate.

**Why a template rather than a baseline to add to.** Comfort varies widely with context, and only the
person knows the context. One person might keep sets on legal reasoning, on theology, and on their own
sexual practice. A single standard for that range either flags everything or protects nothing, which
is the human partner's point. A hidden floor would also mean the rules governing a person's sets were
partly in a file they could read and partly not.

**What the template cannot touch is protection for other people.** Who a detail is about still decides
([whose comfort governs](#decided-2026-10-17-whose-comfort-governs)). A person's own details are entirely
theirs to set. Details about someone else stay governed by the served conventions -- `route-where` says
no shared set takes a person as its subject, and `store-when` limits what is recorded about anyone --
because the person writing cannot consent for someone who is not there. Secrets stay with the
[mechanical scan](#1-mechanical-prohibited-data-scan), which is not a preference at all.

**So the file arrives with built-in framing.** When it is served at session start, Scatterhoard's own
text around it says it governs the person's own details and cannot override the conventions'
protections for others -- the human partner's addition. That framing is served text read at the moment
a session acts, so it is drafted against [the Golden Rule](golden-rule.md) and tested on a small local
model before it ships.

**A copied template is never updated.** Like the default memory-directory README, once the file exists
Scatterhoard does not change it, so improvements to the template reach new users only.

**The file is `sharing-preferences.md` in the common directory**: the one `SCATTERHOARD_COMMON` names,
or `SCATTERHOARD_HOME` when it is unset, which is where the personal store already lives. Decided by
the human partner when the build began. The preferences are about one person, so every server on the
machine reads one copy. Putting the file in the home would make one copy per checkout wherever
`SCATTERHOARD_HOME` is set per checkout, each copied from the template separately and free to drift.

**It is served after the last convention, as a final field of the catalog's JSON document**, carrying
the framing and the file's text, and paged with the rest. Decided by the human partner at the same
point. Paging, rather than an aside on one page, is what lets the file be any length: an aside must fit
a single page, and a refusal there would cost the session the whole catalog. At the end, the catalog
always arrives first and whole, and the framing follows the conventions it refers to.

**Inside the JSON rather than markdown after it, because the audience changes at that point.** The
markdown file is the person's to read and edit. Once it is served with the conventions, the model is the
primary reader, and the human partner's principle is that the design should allow for that. It also keeps
the tool description's promise that the pages join into one JSON document, so no served text changes to
describe a new format. The document then depends on the file and is no longer a constant. A session
keeps the text it was served on its first page, so an edit made between pages cannot shift a cursor.

**A detail kept out of every set is kept out of the personal set too.** Decided 2026-10-21 by the human
partner, after the first local-model test sent a person's employer to `local` as `route-where` asks.
People use Scatterhoard in many places, and in some a detail can get a person arrested: being gay, for
one. Someone who has written that their sexuality is never to be stored, and then mentions it to a
model, is relying on that. If it lands in the personal set it is on their disk and in every backup of
it, which is exactly what they ruled out. Never has to mean never. So the template carries an explicit
carve-out for the personal set, listing what may go there despite the keep-out list, so that a person
can read exactly what is allowed and remove it. `route-where`, which sends anything about the person
to the personal set, has to give way to that.

**The framing is held to the catalog's standard, though it has no budget of its own.** The human
partner's reasoning: the catalog's budget exists to spend tokens efficiently, because the conventions
reach every session, including one on a small local model with a short context, and the person has
no control over them. The framing reaches every session the same way, so it has to earn every
sentence too. The person's own file is the exception: they write it, so its length is theirs.

**The carve-out allows the everyday details and none of the special categories**, decided by the human
partner at the same point. Where a person lives and how to reach them, their employer, their family,
their name, and their machine and account specifics may go into the personal set by default. Health,
religion, sexuality, politics, ethnicity and finances may not go into any set. That line is roughly the
special categories of data-protection law, plus finances. **The condition the human partner set is
that a session says when it has left something out because of the preferences**, so the person can
decide in a follow-up whether and how to store it. Otherwise a long conversation can end with nothing
of it kept, and nothing to say why. The template also says the file cannot control what the AI
client keeps in its own conversation history, so it does not promise more than Scatterhoard can do.

**Served text alone did not hold a weak model to the preferences, so the server refuses the words a
person lists.** Decided 2026-10-22 by the human partner, after five local-model runs. `gemma4:12b` could
quote the rule on health after a run and still store the detail during it, both with the rule in the
framing and with it named in `store-when`. The file's *Never store these words* list is checked on every
store and amendment, in the text, reasoning, context and keywords. It is read from the file at each
store, so an edit protects at once, even in a session that never loaded the conventions. If the file
cannot be read, stores go ahead, since the session-start note already reports it. On `gemma4:12b` this
kept a listed word out in 5 of 5 runs, and the refusal's own last words made every run tell the person
what it had left out, which no served text had achieved in 20 runs. It cannot cover a category, and a
small model still stored "a recurring medical appointment" where "HIV" was refused.

**Tried and removed the same day: a line in `hoard_memorize`'s response** asking the session to check
the memory against the preferences, correct it with `hoard_correct` and tell the person. In five runs
`gemma4:12b` never called `hoard_correct` and kept the detail out no more often than without the line.
At about 175 characters per store it did not earn its keep. The human partner's reading is that the
instruction may simply be too complicated for a model that size, since it asks for three actions after
the fact where the refusal asks for one before.

**Plain words and phrases, matched without regard to case, with a trailing `*` as the only wildcard.**
The human partner's choice. Regular expressions fail silently in the hands of the people this most
protects, and automatic plurals are English-only and invisible. A trailing `*` covers plurals and most
endings in any language that inflects at the end of a word. Over-matching only refuses a store, which
names what matched so the model can reword. Under-matching is what hurts someone.

**Whole-word matching applies only where words are separated by spaces.** Chinese, Japanese, Thai and
other scripts written without spaces are matched as plain substrings, because there a word boundary
check would mean an entry never matches. The human partner asked for this in the first build, since
Chinese may be an important use case for the list.

**Three more provisions for Chinese, also asked for by the human partner on 2026-10-22:**

- **The list folds as keywords do, in `toNFKC_Casefold`.** Chinese text often writes Latin letters full
  width, and the first version lower-cased, which left ＨＩＶ and HIV two words. That was the
  `folding-is-lowercase-not-casefold` defect a second time, caught before it shipped.
- **A space beside a script written without spaces is optional**, so "HIV 阳性" and "HIV阳性" match each
  other. Between two words of a spaced script a space still separates them.
- **Simplified and traditional characters stay distinct**, as they do for keywords. Mapping one to the
  other needs a conversion table, which is a decision of its own. The template tells the person to list
  both forms, and its example does: 艾滋病 and 愛滋病.

The template stays in English. Until it is localised, a person who uses the list is assumed to read
English or to have a model that does, the human partner's call.

**The template says a set is "not public" rather than that its repository is private**, which is how
the example above puts it. A person meeting Scatterhoard for the first time may not be coding at all,
and may keep a set in a synced folder rather than a repository. The template is `TEMPLATE` in
`src/preferences.rs`. It carries only the review conditions that are the person's own to set, which are
their personal details, their name and their machine and account specifics. The others protect other
people or are not about privacy, so leaving them out is what makes the file unable to remove them.

## Decided 2026-10-31: a line the server does not read is reported, never dropped

**Every line of the preferences file that is not blank, a heading, a comment or an item is reported**
at session start, beside the file, with its line number, and the session tells the person. Until this
change such a line was skipped without a word. Two values the human partner added to *Always fine to
share* as bare lines, the way the section's instruction read, did not take effect, and nothing said so
(`a-preference-written-without-a-bullet-is-dropped` in [Issues](../../TRACKING.md#issues)).

**Reported, rather than read as an item or refused**, decided by the human partner after weighing all
three:

- **Read as an item**, a bare line would turn every sentence of prose into one, and spend the syntax for
  good: a line without a marker could never later mean a continuation, a note or a condition.
- **Refused**, the file would block storing until fixed. The server cannot fall back to the template's
  lists, because that drops the person's own never-store words, which is the failure being fixed. So a
  typo would lock out of their memory someone the template is written for: a person who may not code.
- **Reported**, the file keeps loading line by line, prose stays legal and is still read by the model,
  and a bare line keeps no meaning, so one can be given to it later. Every file that holds such a line
  has been told so at every session start until then.

**Markdown stays, rather than TOML or YAML**, by the same reasoning. YAML misreads silently, which is the
defect itself: an unquoted `no` becomes false, and a colon makes a mapping out of the template's own
exception. TOML is strict, but fails for the whole file, which brings back the lockout. Reading markdown
as precise instructions does amount to a small markup language of our own, the human partner's
observation, and it is acceptable as long as nothing in it surprises the person.

**What counts as an item, and what is reported:**

- **Items start with `- `, `* ` or `+ `**, the three bullets CommonMark treats as one.
- **A numbered line is reported**, not read: numbering says order matters, which nothing here honours.
- **The report covers every heading, known or not.** Items under an unknown heading are kept out of every
  set, so a bare line there is where a silent drop costs most.
- **An item written inside a comment is reported too**, unless it is one of the template's own examples.
  To the eye it sits in the list, the way a word added under the template's example would. That also
  reports the dash-style comments some people write, carrying ` - ` down each line, and the human
  partner accepted the cost: a note a person does not need costs less than a never-store word that
  never took effect.

**So the template keeps its explanation inside comments**, and a fresh file reports nothing. Inside a
template comment a marker appears only on a line meant to be copied, after the comment's *An example*
line, never on the explanation, so a person can tell decoration from the shape their own lines should
take. Every section that asks for items names the marker, since "one per line" alone is how the two
values came to be written bare. A file copied from the earlier template is not rewritten, so its
explanation is reported until the person moves it into comments.

## Decided 2026-10-17: a full review, and when one is due

The routine review covers the diff. **A full review covers everything the file holds**, and is due
when what the diff assumes stops being true:

- **When a repository goes public**, because every private-only exception lapses then, and memories
  cleared for a private audience are about to reach a public one. The full review applies the
  conditions with those exceptions off.
- **When the conditions change**, since memories committed earlier were cleared against the old ones.
- **Before a repository's first publication**, whatever review its history had -- the case on
  2026-10-17, when the earlier read-through had no written conditions to check against.

**A full review leaves one marker per set, not per memory**: a memory saying which commit it reviewed
through and which version of the conditions it applied. The next routine review starts from there.

## The procedure

1. **A mechanical pre-pass** pulls out candidates no reading should miss: email addresses, home paths,
   host names, phone numbers, personal names.
2. **The model reads every memory in the review set** against the conditions and the exporting person's
   preferences, and records a verdict for each.
3. **The human decides each flag**, by the routing above. A memory kept out is forgotten before the
   export is committed; one cleared stays. **One cleared by rewording is restated, then the original
   forgotten**: the restatement carries a `supersedes` link to the original, so the original's in-links
   follow it with their labels. Superseding alone would leave the old wording in the exported file, and
   forgetting alone would strand every link to it. Tried on 2026-10-18, when the first attempt left the
   link out and stranded three in-links.

**Built 2026-10-24**, as decided below: `hoard_sets` actions `review`, `clear` and `backup`, the export
gate, the session-start reminder, and the pre-pass and project file in `src/review.rs`, tested in
`tests/export_review.rs`. The rest of this section is the history of how it was decided.

**A review needs a capable model**, the human partner's conclusion the same day, after `gemma4:12b`
was run through the built review on a snapshot of a real `local`. With a batch too large for its
16,384-token window it lost the task and said it had cleared everything. Within the window, with the
response trimmed to a size cap, with an empty `clear` refused, and with a ready-made `clear` call handed
to it, it still never answered the memories, though in a short prompt with a small dedicated tool it
could. The tool had been bent and adjusted, and still gave no useful result. So the review is not made to
run on small models; it says plainly that it needs a capable one. The server's safeguards stay whatever
model runs it: nothing is cleared without answers, an empty `clear` is refused, an unfinished one says in
words that it is not finished, and a batch stays within a small window. On a small model, what protects
the person is the pre-pass and the never-store list.

*Decided 2026-10-17 with the human partner; none of it is built. Until it is, the procedure is followed
by hand. Building it is `export-review-judgement-gate` in [Backlog](../../TRACKING.md#backlog).*

**Built before the repositories go public, not as a fast follower**, decided 2026-10-22 by the human
partner. The concern is the privacy of Scatterhoard's users rather than the project's own, whose memories
had their full review by hand, and they would rather not build it under the time pressure of a public
release. **One constraint on the mechanism: Scatterhoard must not depend on `git` to run the review.** A
memory set may live in a synced folder rather than a repository. That rules out reading the committed
file with `git show`, so "the review set is the diff against the committed file" needs another way to
know what has already been cleared.

**The file on disk is the clearance record, because export never writes an unreviewed memory into it.**
Decided by the human partner the same day, over a ledger kept beside export and over using `git` where
present. Review comes before export: it compares the store with the file on disk, and the person's
decisions are recorded in the store by record checksum, so a memory changed after it was cleared is
reviewed again. Export then writes only what the file already holds or what has been cleared, and
refuses, naming them, on new memories that have not been. "In the file" therefore means "reviewed",
with or without `git`. A pull or a synced copy brings in memories already in a file, so they count as
reviewed upstream and are not reviewed again, which is the property the committed-file rule had. It also
puts the check where the never-store refusal showed even a small model complies: at the moment of acting.
The cost is that every shared-set export now waits on a review, and `export-before-commit` has to say so.
How a later review turn runs is still to be designed.

**`local` is reviewed but not gated**, decided by the human partner the same day. A backup must be
complete, and one that silently left out unreviewed memories would fail when it was needed. But some
problems only show in a review: the 2026-10-17 read-through found that no single `governance-docs`
memory was a concern while seventy together were, and a small model stores categories, such as "a
recurring medical appointment", that no word list catches. So `local` goes through the same `review`
action, against the person's own preferences rather than the publication conditions, with decisions
recorded the same way. A `local` export still writes everything, and reports the memories not yet
reviewed, any that hold a never-store word, and `describes`, the view of what the set amounts to.

**The reminder comes at session start, not at the backup.** The human partner's reasoning, by analogy
with a car that lights its maintenance warning at engine start, so that by the time you arrive you have
forgotten it: a person who backs up as they sign off for the day has forgotten a notice in the backup's
response by the next session. `hoard_conventions` therefore says when `local` holds unreviewed memories.
Unlike the car, the session can hold the notice and raise it again at a better moment.

**The person decides in conversation, not through a form per memory**, decided by the human partner the
same day over the elicitation forms Scatterhoard already uses for the author name. A person clicking
through fifty forms reaches the point of fatigue and lets things slip, where a model can present the
memories in batches, as the 2026-10-17 read-through did, and split a long review across sittings. The
cost is that the server hears the model's account of each decision rather than the person's own answer,
so the mechanical pre-pass carries the weight: whatever it catches is always put to the person. A review
can stop part-way, because each decision is recorded as it is made, and export waits only on what is left.

**The server groups the review set by subject**, with the mechanical pre-pass's hits first and on their
own, decided by the human partner the same day. The groups are the clusters `describes` already derives
(`src/summary.rs`), each named by its most distinctive keywords. Sorting fifty memories into sensible
batches is the kind of multi-step organising a small model did poorly, and grouping by subject is also
what makes aggregation visible: seventy memories about one association arrive as one batch, the view
that found the problem on 2026-10-17. A long review can take a group per sitting. A set with no
embedding model is served as one group in the order its memories were written, and says so.

**The briefing comes once per review in a session, not on every page**, decided by the human partner on
2026-10-27. The briefing is the task in full, the person's preferences and the keep-out lists for other
people and for the project. It arrives on a review's first page in a session, again when the
preferences change, and again when a full review starts. Every later page carries a one-sentence task
that keeps the instruction to answer and points back to the first page. The caution stays on every page,
since it was decided to speak at the moment of acting. Until then each page re-sent about 6,000
characters of the same text beside three to eight memories, so a full review of the `scatterhoard` set,
about 1,500 memories, would have spent most of several sessions' context re-reading it. The per-page
briefing had been kept for `gemma4:12b`, which is no longer the model a review is built for: the review
says it needs a capable one. A session begins at `hoard_conventions`, so a new conversation is briefed
again. **The limit** is a session that loses the first page from its context without a new session
starting, after compaction for instance, which then reviews from the one-sentence task alone. The
preferences also arrive with the conventions at session start, which narrows that case to the task text.
**On `gemma4:12b` it costs little**, measured the same day with `local_model.py classify --says --paged
5`: 59 memories in pages of five as one conversation, three runs per arm. Briefed on every page it got
134 of 177 answers right, with 6 misses and 37 false alarms. Briefed once, it got 131, with 7 and 39.
Most errors fell on the same memories in both arms, so they are the model's and not the briefing's. The
few that moved were on later pages, and two of them misapplied a preference, which is the drift a lost
first page would cause. The harness keeps the task in its system prompt, so this measures dropping the
preferences and not the task.

**A memory is cleared by a `clear` action that accepts only ids a review in the same session has shown**,
agreed provisionally by the human partner the same day. The other two decisions already have tools:
keeping a memory out is `hoard_correct` forget, and rewording it is a restatement with a `supersedes` link
followed by forgetting the original, whose restatement then enters the review like any new record. The
restriction is what the server can check when it cannot hear the person: not that they said yes, but
that the memory was put in front of the session. It rules out a model clearing everything outstanding,
unseen, to get an export through. **The same holds for the person's own clearances, since 2026-10-26**:
a flagged memory changed after it was shown goes back to the review rather than being cleared, because
the person decided about the text they saw. One forgotten after it was flagged no longer counts as
awaiting them. Both were found on the review's first run on this project's own set, where a reworded
memory still showed as waiting, and the amended case had let the person clear text they were never shown.

**A memory is reviewed again only when something is done to that memory**, the human partner's
standard, set the same day: a person expects to re-review a memory because they or the model changed
it, never because of a change elsewhere. The reason is trust. This is a trust mechanism, people trust
what makes sense to them, and an effect that crosses from one memory to another, however well the design
justifies it, reads as unpredictable to anyone who does not know the internals, which is nearly everyone.
Clearances use the existing full-record checksum, confirmed by the human partner, so "reviewed" has one
definition shared with the comparison against the file. The record checksum keeps that for changes outside the
record. It covers the bytes an export writes (`checksum::of_record`), and vectors are never exported, so
re-embedding a set, changing its model or calibrating its floor reopens nothing. A change to the sharing
preferences is the one outside change that bears on review, and it must not undo earlier review work
wholesale either. How it reopens only what it affects is still to be designed.

**The person cannot be the backstop for memories nobody flags**, which the human partner showed with
`TRACKING.md`'s own rows: a summary line cues the gist of something you half remember, and nothing at
all about a memory you never wrote. "We decided to change the schedule" does not cue that the next
sentence is about an HIV appointment, and the 2026-10-17 read-through covered fifteen hundred memories.
So a memory nobody flags is cleared on the model's reading and the pre-pass alone, and the design makes
that reading narrow and checkable. **`clear` takes, for each memory, who it is about (the project, the
person or someone else) and which kept-out categories it mentions**, and the server routes it: about the
project and mentioning none, it is cleared; anything else is put to the person in full. A small model
answers "does this mention health?" where it misjudged "is this a problem?". A preferences change asks
the same questions about the newly added keep-outs only. Agreed by the human partner the same day, to be
tested on a small model before it is built on. A model can still answer wrongly, so the review says
plainly that unflagged memories were cleared on its reading, and the strongest available model is
recommended for it.

**Tested on `gemma4:12b` before building, 2026-10-22 and 2026-10-23, and the questions changed as a
result** (`scripts/local_model.py classify`, on the invented memories in `corpus/review-questions.json`):

- **The review numbers each memory's sentences and asks per sentence.** Asked about a memory as a whole
  and on its own, which is what a routine review of one or two new memories looks like, the model cleared
  a detail after a harmless first sentence in every run. Numbered, it missed nothing.
- **The model reports every item it recognises, from either keep-out list, and the server applies them.**
  The model did not work out which list applies to a set: it flagged the employer in `local` whether the
  carve-out was an exception or a separate list. Giving it only its set's items cut those false alarms but
  brought back misses. So the model reads the whole file, and the server drops items from *Keep out of
  shared sets* for a `local` memory. That reading needs only what the never-store list already parses: a
  heading, then the items under it (since 2026-10-31, [every other line is reported](#decided-2026-10-31-a-line-the-server-does-not-read-is-reported-never-dropped)). A list under a heading the server does not recognise counts as kept out of
  every set, so an unfamiliar list over-flags and is never silently ignored. Exceptions stay prose for the
  model, and the file stays markdown, never JSON, which the human partner judged would be the tail wagging
  the dog.
- **The server requires an answer for every memory it shows.** Until the answer's schema demanded one per
  memory, the model answered a group's first memory and stopped.

Re-scoring the earlier runs this way suggested 48 of 48 right for lone memories, but that covered only
sixteen of them. **A live run of the whole approach, asked for by the human partner rather than trusting
the re-scoring, found more**: realistic groups missed nothing, with three false alarms where "users" was
read as someone else, while all 48 memories asked about alone gave 8 misses in 144 answers. An election
campaign was not read as politics, an employer in a shared set was missed, and a husband at a mosque was
missed twice, once by being labelled "my family", which the server then dropped for `local`. A paraphrased
quote also slipped past the exact-match filter. Reasoning and context fields, and a never-published set,
were handled correctly. The template's keep-out list was split into *Keep out of
shared sets* and *Keep out of every set, including private ones* along the way, at the human partner's
wording, replacing the exception that carved `local` out.

**Settled for building, 2026-10-23, by the human partner.** A memory reviewed alone is sent with up to four
already-cleared memories from its own subject, marked as context and not to be answered. That cut the
lone misses from 8 to 6 in 144 and removed a false alarm. The server matches quoted items leniently: the
same words once folded, or an item's opening words. The two misses left were the same memories in every
run, which reads as the limit of what the model recognises. So the residual risk on small models is
accepted, and stated plainly at the end of every review, with the strongest available model recommended
for it. The pre-pass and the never-store list sit underneath whatever model runs it.

**The kept-out categories are the person's own file, as worded there**, decided by the human partner the
same day over a list built into the server. The model reports which keep-out items a memory mentions,
quoting the file, and applies any exception for the set under review. The server routes: in a shared set
a memory is cleared only if it is about the project and mentions nothing kept out there; in `local` it is
cleared if it mentions nothing kept out there, since memories about the person are what `local` is for.
Anything about someone else is put to the person whatever the file says. One source of truth, the one
the person can read and edit, means a keep-out they add is checked with no change to Scatterhoard. A
misapplied exception flags too much rather than too little.

**Revised 2026-10-25, before the gate was first committed: a sentence about a person also says whether it
reports or judges, and the lists govern what is said about others.** The first real review, on a capable
model, showed what the three labels could not express. A paraphrase of the human partner's view was
labelled *the person* and flagged, while the sentence saying they made the decision was labelled
*project*, and every sentence about how a model behaved cleared as *project* although `store-when` counts
an LLM as a person. The human partner settled three things the same day:

- **It must be possible to report what someone did.** A sentence about the person or someone else, an LLM
  included, is answered `reports` (what they did, said, wrote or decided) or `judges` (what they are
  like, or why they acted): the line the [Golden Rule](golden-rule.md)'s demands A and D draw. The server
  cannot see that line, so the model answers and the server routes. A judgement is flagged in every set. A
  report clears unless it mentions a kept-out item, except that a sentence about the person in a shared
  set is still flagged as misfiled, as `route-where` asks. A sentence saying what the person decided or
  said about the work is about the project.
- **A person's keep-outs govern what they write about anyone else.** Someone who keeps their own sexuality
  out keeps everyone's out. Someone open about theirs is not thereby free to write about anyone else's,
  since that is the other person's to share. So for a sentence about someone else the server applies the
  template's lists together with the person's, the stricter placement winning, and the review serves the
  combined items for the model to quote. The template is the floor here, though not for the person's own
  details, which stay theirs to loosen.
- **Exceptions and always-fine values are the person's alone**, and never apply to a sentence about
  someone else.

**Tested on `gemma4:12b` the same day, and a missing answer is read as judging rather than refused**
(`scripts/local_model.py classify --says`, with invented cases added to `corpus/review-questions.json`).
Reports about other people cleared, and their health and employer were caught. But the model left `says`
out of 39 answers in 177, mostly for sentences about the person in shared sets, where it changes nothing,
so refusing them cost a round trip a small model may never get past. Joining the two questions into one
required label of five values ended the refusals and weakened *who is this about*: on the older memories
it missed 18 answers where two fields and the pre-change questions each missed 11. **So, decided by the
human partner on 2026-10-26, a sentence about a person with no `says` is routed as judging**, which errs
towards showing the person a memory. Gemma still reads a judgement of a model as `project`, and a husband
as `my family` rather than `my sexuality`; both are left to the capable model the review recommends.

**The mechanical pre-pass scans five shapes and the set's own author names**, decided by the human
partner the same day: email addresses; home paths carrying a user name (`/Users/<name>/`, `/home/<name>/`,
`C:\Users\<name>\`); private network addresses and internal host names (`.local`, `.lan`, `.internal`,
`.corp` and the like); phone numbers in shapes that read as phone numbers, with a leading `+` or grouped
digits, so that ids, counts and dates pass; and never-store words, rechecked for words added after a
memory was stored. Other personal names have no shape, and a capitalised-word heuristic is noisy where it
fires and blind in lowercase and in scripts without capitals, so they are left to the model's answer to
*who is this about*. The author records are exact, so a collaborator named in a memory's text is caught.

**Except the name the person currently publishes the set under**, decided by the human partner on
2026-10-29. That is their name as the author of their own work, which the preferences template already
excepts and every exported record's attribution already carries, so flagging it only floods the review:
on this project's own set every memory on the first page went to the person for `jtcours`. The
exception is built in rather than left to "Always fine to share", because the server holds which record
is the person in this set and the person decided the name when they chose it; an always-fine entry
would apply in every set, and would silence only the pre-pass. The name is the tip of the set's own
author record (`Store::published_name`), so a rename merged in from another copy is followed and a
record split in two exempts nothing. Past names and every other author's name are still flagged. The
first page serves it as `published_as`, and the task says that name alone mentions nothing.
`the-review-flags-the-persons-own-published-name` in [Issues](../../TRACKING.md#issues).

**An "Always fine to share" section in the preferences file silences the pre-pass for exact values**, such
as a person's work email or phone number, so they are not put to the person in every new memory. Decided
by the human partner the same day. It holds literal values, parsed and folded like the never-store list,
and applies in every set. The never-store list wins where a value is in both. It silences only the
pre-pass: the model's two questions still apply. Scatterhoard never writes the file, so when the person
clears a pre-pass hit the review suggests the line to add. Values belonging to the project rather than
the person, such as a repository's `noreply` address, belong in the project's file instead.

**The project's additions live in `scatterhoard_mem/config/sharing-preferences.json`**, decided by the
human partner the same day. It holds two keys: `keep_out`, items every memory exported to the repository
is checked against beside the person's own, and `always_fine`, exact values that belong to the project
and that the pre-pass does not flag. An unknown key is refused and named, because an ignored one would
look applied and do nothing. Nothing in it removes a condition, adds an exception or says anything
per person: the person a memory is about still decides. So the worst a careless or hostile file can do is
make a review noisier. It is a subdirectory rather than `config.json` so that no set name is taken, since
every top-level `.json` file there is read as a memory set, and so that later project settings can join
it as files. It shares its name with the person's file because it is the same idea at project level.

**A set's audience is a fact it carries, in the project's file as `"audience": "private"`, and absent
means public.** Decided by the human partner the same day, after asking how *going public* differs from
pushing to a repository that is already public. They differ: going public changes the audience of
memories already published, so all of them need another look, while a push to a public repository
publishes only new memories, which the routine review covers. But the routine review has to know the
set is public, or it goes on applying "while it is not public" exceptions that no longer hold, which a
flag on one full review could not tell it. So those exceptions apply only where the project declares
itself private, and forgetting to declare exposes nothing. When `audience` changes from private to
public, locally or through a pull, the next review is a full one with those exceptions off, and it says
why. That is a change elsewhere reopening memories, but one whose cause the person can see. A full review
can also be asked for with `full: true`, for a first publication or by choice. No convention about
pushing is needed: Scatterhoard never sees a push, and a rule nothing checks is what failed on a small
model. There is no marker memory either, since the clearances already record what was reviewed.

**The server reads a set's audience from the project's file, and no call can set it**, confirmed by the
human partner the same day. A parameter would be the one switch that loosens the review, which a model
could set by mistake or because something it read told it to. Parameters that only make a review stricter,
such as `full: true`, are the model's to pass. A `file` export outside a checkout has no project file, so
it is reviewed as public. `local` has no "while not public" exceptions, since its carve-out is
unconditional, so a backup of it is neither re-reviewed nor treated as public. **The template's carve-out
stops saying `local` "never leaves this machine"**, also decided then: a backup written to a synced
folder does leave it. It says instead that Scatterhoard never publishes `local` and that a backup goes
wherever it is written.

**A backup is its own action, never gated, written only where Scatterhoard chooses**, decided by the human
partner the same day. Without it, a set with no repository, such as `governance-docs`, would have its
first backup gated as a first publication, since no file anywhere holds its memories. `hoard_sets` action
`backup` writes the whole set into `$SCATTERHOARD_BACKUPS/<set>/<timestamp>.json`, or under
`<common>/backups/` where that is unset, and marks the file's header as a backup, so its memories never count as reviewed if it is later copied
into a repository. The caller cannot choose the path, so a backup cannot be used to get round the gate,
and the review's rule that loosening never comes from a call holds. Copying a backup off the machine is
the person's own act, and the response says where the file is so they can. `restore` reads it like any
memory file. Each backup's response says how many that set already has, and pruning is left to the
person, since a safety mechanism should not delete copies on its own.

**The person names the backup location, in the client's configuration, as `SCATTERHOARD_BACKUPS`**, decided
by the human partner the same day. The first version wrote only beside the store, which the human partner
had already found protects against deletion and nothing else, which is why they kept their own backups off
the machine. Those backups were `file` exports, which the gate would now review as public. Set beside
`SCATTERHOARD_HOME` in a server's entry, the location is chosen by whoever sets up the machine and never
by a call, so a model still cannot send a backup somewhere of its choosing. A person who points it at a
shared folder has chosen that, and the documentation says so.

**A change to the preferences reopens only what it tightens, measured against the text the last review
used**, decided by the human partner the same day. Each set keeps a copy of the person's and the
project's preferences as its last finished review read them, in the store beside the clearances, and the
next review compares line by line. Words added to the never-store list reopen the cleared memories
containing them, matched by the server. New keep-outs and removed exceptions are put to the model as the
same two questions about cleared memories, group by group, with each group's verdict shown to the person.
Loosening reopens nothing. The copy is updated only when a review finishes, so one cut short compares
against the old text next time. A reworded item counts as removed and added, which errs towards
reopening. The copy stays in the store, which is never published.

**A shared set the person never publishes is declared in their own file, under *Sets I never publish*,
and is then reviewed like `local` and refused for export.** Decided 2026-10-23 by the human partner,
after asking how `governance-docs` would be treated. As designed until then it would never have been
reviewed: it has no repository, so it is never exported, and the session-start reminder covered only
`local`. If exported anyway it would be reviewed as a public shared set, flagging nearly every memory,
since the set is about the person. The server cannot tell a set that is never published from one that
has not been exported lately, because the store deliberately does not remember destinations. So the
person declares it. A listed set is reviewed against their preferences with *Keep out of shared sets* not
counting, joins the session-start reminder, and is refused by export, though `backup` still works. The
refusal is what makes the declaration trustworthy: it loosens the review only by also guaranteeing the
set is not published, and it comes from the person's file rather than a call.

**The person is cautioned in two places**, decided by the human partner the same day. Most people will
be fine, but some, by their circumstances or a set's subject, will want to read new memories themselves
before publishing. Every review's response ends with a line the model passes on, saying that unflagged
memories were cleared on the model's reading alone and that a person for whom a miss would be costly
should read them first. It says to read the memories rather than the diff, since without `git` there may
be no diff. The template carries one sentence near its top saying the same, as ordinary text rather than
a comment, since a comment is served too.

**Upgrading to the build with the gate needs no migration.** Memories already in the file on disk count
as reviewed, so the first export after upgrading reviews only what has not been exported, as any export
would. The one residue is a file exported before the upgrade and never reviewed or committed, whose
memories are taken as reviewed.

## Superseded 2026-10-17: the export-review ledger

*Kept as history: [the review set is the diff against the committed file](#decided-2026-10-17-the-review-set-is-the-diff-against-the-committed-file)
replaces it, because a memory can no longer change in place. The reasoning below was sound for the
model it was written against.*

To avoid re-flagging a memory on every single export cycle, keep a small **export-review ledger**, separate from the memory data model itself, keyed by memory ID, recording `last_cleared_at` — the memory's `modified` timestamp at the point it was last cleared for export into a given set.

On each export-and-commit cycle:

1. Identify memories whose last modifier is the local user.
2. Of those, select the ones that are new (no ledger entry yet) or whose `modified` timestamp is newer than the ledger's `last_cleared_at`.
3. Run the judgment gate against those, using the current flagging conditions from conventions (hard-coded + local overrides).
4. On clearance, update the ledger entry to the memory's current `modified` timestamp.

The ledger is intentionally kept separate from the memory's own fields — it's process state (has this been reviewed for export), not content state, and it can live per-set/per-export-target without growing the memory data model.

It also carries a *first-encountered* flag per project, used by the unexpected-entries tripwire above so a suspicious file gets mentioned once on first meeting a project rather than waiting for that project's first export.

## Superseded 2026-10-17: ledger is local-only

The ledger is **local-only, per-machine state** — never committed alongside the memory set. Reasoning: the judgment gate is already scoped to memories the local user last modified, so no other contributor's machine ever needs to consult it — there's no cross-user redundancy for syncing to eliminate. Syncing it would only help the same human moving between two machines (e.g., laptop and desktop), and even then only saves a one-time re-review per memory; it isn't worth taking on the same merge-conflict surface that [Multi-Set Sync](multi-set-sync.md) already has to solve for the memories themselves, just for bookkeeping. The ledger is created on first memory (no upfront provisioning).

To ease that laptop/desktop case without building sync into Scatterhoard itself, the ledger's **location should be configurable** rather than hard-coded to a fixed path. That lets a user who wants continuity across machines point it at a folder that's already synced by something else they control (Dropbox, Drive, etc.) — Scatterhoard doesn't need to know or care that it's being synced; it just needs to not assume a fixed local path.
