# Load Modes

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

How a set is held on this machine: as the machine's own, writable record; as a document someone else
publishes, readable and never written; or as a copy under review that nobody has adopted. [Set
Lifecycle](set-lifecycle.md) covers the operations that create, move and remove a set; this document
covers the property those operations now have to respect.

**Status: proposal, 2026-09-16; revised the same day** to default `create` to read-write, allow several
evaluation loads at once under user-chosen labels, mark evaluation hits from the load mode, and load
each set identity only once.
**Built so far (2026-09-17):** the read-write and read-only modes, stored in `meta`; `create`'s
`mode`; `status` reporting it; every refusal in [what read-only refuses](#what-read-only-refuses-and-where);
[dropping without a safety copy](#dropping-a-read-only-set); [one copy of each
set](#one-copy-of-each-set), in `Server::settle_identity`; and [the watch marks each set
pulls](#which-watch-marks-a-set-pulls); and, on 2026-09-18, **evaluation**, as `hoard_sets` action
`evaluate`. **Not yet built:** promotion (deferred, `promotion-to-read-write`). It gathers decisions that until now existed
only as memories under `evaluation-mode`, `read-only-distribution` and `load-mode`, and it answers
parts of both questions in [Open questions](../../TRACKING.md#open-questions). **Read-only is built
first; evaluation follows as a special case of it.**

## Why this exists

The Vision's review loop has a maintainer reading a contributor's memories before merging them. Today
the only way to read a set is to import it, and **import is adoption**: checking the branch back out
does not undo it, and every later import reports those memories as anomalies. That is the
`evaluation-mode` question, raised 2026-08-26.

The second need arrived while narrowing the first: a set published **to be read**, the memory
equivalent of a PDF. The reader gets the memories but may not add to them, and the reading machine
contributes nothing back. That is `read-only-distribution`.

The third was found by fixing a defect. Two loaded sets could answer to one set identity, and the fix
(`two-sets-can-share-an-identity`) had to refuse something. With nothing to say which of two holders
is the machine's own, the server withholds **both**, which can lock someone out of their own set at a
restart. The human partner accepted that only on condition it is replaced here, and the
[rule below](#one-copy-of-each-set) replaces it wherever a mode can tell the two apart.

## Three modes, and what each one is

| | **Read-write** | **Read-only** | **Evaluation** |
|---|---|---|---|
| For | the machine's own sets — everything today | a set published as a document | a copy under review |
| Session writes | yes | refused | refused |
| Refresh from a file (`import`, `restore`) | yes | yes | no — it is loaded once |
| Name | its own | its own, stable, citable | `eval-<set>-<label>`, temporary |
| Unscoped recall | searched | searched | **not searched**; only when its scope is named |
| Returned memories marked | no | no | **every one, from the load mode** |
| Watch marks a session recalls | charter, signpost, collect, investigation | charter and signpost | none |
| Drop needs a safety copy | yes, always | no, and `save_to` is honoured if given | no |
| Survives a restart | yes | yes | yes — loaded again from its file |
| Copies usually loaded | one | one | several, for comparison |
| Counts as the loaded copy of its identity | yes | yes | no — [never a rival holder](#cross-set-links-are-surfaced-never-resolved) |

**The mode is a property of how this machine holds the set — never of the set, and never of its
file.** A file's header is contents, and `Expectation` exists precisely so that contents never decide
how they are trusted ([File Format](file-format.md)). A publisher therefore cannot ship a set that
loads read-only; the reader chooses. The same rule is why a repository cannot ship a set claiming
`local` scope.

**One value carrying a policy, not a flag.** The two non-writable modes differ in only a few rows
above. Built as a `read_only` boolean, each difference becomes another flag, and three coherent modes
become eight states of which five are nonsense. A mode enum whose policy is asked of it — *may a
session write*, *which marks are recalled*, *is a safety copy owed*, *is this searched unscoped* —
makes those five unrepresentable, which is [STANDARDS.md](../../STANDARDS.md)'s stated preference over
detecting them at run time.

## Where the mode is stored

**In the set's own database, in `meta`, under `load_mode`.** An absent row means read-write, which is
what every set created before this design is.

This is safe because `meta` never travels. An export writes records and a header, not `meta` rows.
`restore` rebuilds through `Store::replace_with`, which clears the record tables and leaves `meta`
alone. `drop` deletes the database, so the mode goes with the set. And because the row lives in the
database rather than in a list beside it, it moves with the set between the home and common
directories ([Store Registry](store-registry.md)) and leaves nothing stale behind when the set goes.
A separate machine-wide settings file was the alternative, and was rejected for exactly those two
costs.

**An evaluation load writes nothing that a restart could find.** It is not a discovered set, so it
does not need a stored mode; see [Evaluation](#evaluation).

## Choosing the mode: once, at `create`

**`hoard_sets` action `create` takes an optional `mode`: `read-write`, the default, or `read-only`.**
No action changes a mode afterwards except [promotion](#promotion-one-way-and-deferred).

Two checks from [Decision Placement](decision-placement.md):

1. **Does whoever decides have the information?** Whether a set is the machine's own to write or
   someone else's document to read is almost always evident from why it is being created: *take on
   the company policy set* says read-only without anyone asking. The LLM has that context, the
   software does not, and the publisher must not decide it. So the tool description says **when to
   choose read-only** — taking on reference material someone else publishes — and does **not** tell
   the LLM to ask. Most users assume memories are read-write, and asking a new user whether their new
   set should be read-only would confuse them. The author name is asked for because only the human
   knows it; a set's mode usually is not like that.
2. **Does the answer change anything?** Yes, and a wrong mode is corrected only by dropping and
   recreating the set. But the default errs in the safe direction. A set created read-write that
   should have been read-only is merely writable when it did not need to be, and
   [drop, create read-only, import](#no-demotion) puts it right with nothing lost. A required
   parameter would make nearly every call carry the same choice, which is the cost this principle
   counts against it.

**A distributed set is taken on as `create` with `read-only`, then `import`.** Import is allowed on a
read-only set — see the table — so the order costs nothing, and the set is never writable for even a
moment.

### No demotion

Nothing turns a read-write set read-only. Every case that seemed to want it has another route:

- **Taking on a distributed set** is `create` read-only, then `import`, as above.
- **Two copies of one set on one machine** are resolved by dropping the extra copy, which writes its
  safety copy; no copy changes mode ([One copy of each set](#one-copy-of-each-set)).

**What leaving it out buys is the safety-copy exception, made structural.** `drop` writes a safety
copy unconditionally, and that rule was made exceptionless on 2026-08-06 by *removing* a guard: the
guard needed a fact — *does this set hold anything no file has?* — that the database could not
reliably report, and a guard stored inside the artifact it protects is unavailable when it is needed
([Set Lifecycle](set-lifecycle.md)). A set that has been read-only since it was created answers that
question by construction: this machine never wrote to it. With demotion, a set could be written and
then demoted, and the exception would lose that writing silently — or need an *ever writable* flag in
the very database being destroyed, which is the removed guard back again. **The honest wording is
*the save is unconditional for a set this machine could have written to*, not *skip the copy when
nothing changed*.**

### Promotion: one-way, and deferred

**Deferred rather than undecided**, 2026-09-17: the backlog entry is `promotion-to-read-write`, and
the workaround is to drop the set and create it again read-write, which costs only the time to embed
its memories again. When it is built it is a
`hoard_sets` action that removes the `load_mode` row, and it is one-way for two reasons:

- **The store:** demotion does not exist, for the reason above.
- **The session:** promoting a set makes it writable, and the `recall-when` convention already says a
  session recalls a writable set's collection and investigation marks when the set is loaded. Those
  memories then sit in the session's context, and that cannot be undone either. The human partner
  noted this, and it is why the convention needs no new wording for promotion: the existing trigger
  fires.

A promoted set is then an ordinary read-write set. Its safety copy is owed from that moment on,
because it is now a set this machine could have written to.

## What read-only refuses, and where

**The rule: a read-only set refuses what would change its records at a session's request, and
allows what a file supplies or what is this machine's derived state.**

| Refused | Allowed |
|---|---|
| `hoard_memorize` into it | `hoard_recall`, `hoard_lookup` |
| `hoard_correct` (amend, forget) | `import`, `restore` — refreshing from the publisher's file |
| `absorb` — brings another set's records in | `export` — writes nothing into the set's records |
| `mint`, `rename`, `identify` — these exist only to attribute writes | `reembed`, `calibrate`, `uncalibrate` — derived, machine-local state |
| | `drop` — see below |

Links from other sets **into** a read-only set are unaffected: they live in the other set. Nothing
written elsewhere changes a read-only memory's standing either, since no edge changes the standing of
a memory in another set ([Cross-Set References](cross-set-references.md)).

**Enforced at the store, not at each tool.** The safety-copy exception rests on this, so it is a
condition rather than a preference: checked once where session writes enter the store, the guarantee
is structural; checked by each caller, it is an assumption that the next caller breaks. The store
already separates the two kinds of write — a session's through `Store::insert`, a file's through
`Store::accept_from_file`, sharing one private `write` so they cannot drift apart — so the refusal
sits on the session side of that split: `Store::insert`, `Store::carry_forward` and `Store::forget`
each refuse through `refuse_session_writes_if_read_only`, and the file-side entry points do not.
Each has a store-level test and a recorded mutation.

**The server also refuses early, in two places, for the wording rather than the guarantee.**
`Server::author_for` — asked only for a write — refuses first, because a set imported with its
publishers' identity records would otherwise answer a write with *which of these is you*, a question
that means nothing for a set this machine never writes to. And `absorb` and the three author actions
are refused before they start: `absorb` writes its safety copy before it asks who is writing, and
`identify` writes only machine state, which the store does not guard.

**A refusal says what the set is and what to do.** For example: *"`shared:example` is read-only on
this machine, chosen when it was created. Its memories come from the file it was imported from, so
changes belong there."* Once promotion exists, the refusal names it.

### Dropping a read-only set

**`save_to` is optional, and no copy is written unless it is given.** Not having a copy to write and
then clean up is one of the incentives for holding a set read-only, so the default is to write
nothing; the set's memories came from its publisher's file, which is what it would be restored from.
**Dropping a read-write set without `save_to` is still refused**, so the unconditional save survives
exactly where it was argued for: a set this machine could have written to.

**Given anyway, `save_to` is honoured rather than refused.** The published prose says *`drop` takes
name and save_to*, and a set's mode is not visible at the call site, so a caller will often pass it
out of habit. Honouring it writes the file they asked for; refusing would turn the documented habit
into an error and a retry. Ignoring it silently is the option both other answers beat, since a path
that does nothing is the control [Decision Placement](decision-placement.md) warns against.

**The case that argues the other way, and why it is accepted.** Someone who deletes the publisher's
file makes their read-only copy the only one, and dropping it then loses the memories. That is rare,
and recoverable in advance: `export` reproduces the publisher's file from a loaded set byte for byte,
where the publisher wrote it with the same build and model (`a_read_only_set_re_exports_the_file_it_was_given`
in `tests/sets.rs`). So the response to a drop that saved nothing says plainly that nothing was saved
and what that means if the publisher's file is gone. It does not advise `export` there: by then the
database has been destroyed, and advice a reader can no longer act on is worse than none.

**How a reader learns a set is read-only before hitting a refusal:** `hoard_sets status` reports
`mode` on every set.

### Which watch marks a set pulls

**`status` names them per set, and the convention says to recall what it lists.** A read-write set
pulls all three — `hoard-signpost`, `hoard-collect` and `hoard-investigation` — and a read-only set
pulls `hoard-signpost` alone: it is the mark that asks for a recall before reasoning about its
subject, where the other two ask for findings and evidence a read-only set cannot take. The mapping is
`LoadMode::watch_marks`, beside the mode itself.

**Why not spell the modes out in the catalog.** The clause used to say *whenever a **writable** set is
loaded*, a two-valued distinction written in September as a forward reference to evaluation, which
pulls nothing. Three modes do not fit it, and enumerating them in served text would cost characters
now and need rewording for every mode added later. Pointing at `status` costs neither: a session
already reads `status` to learn what is loaded, and the rewrite **freed 51 characters** of catalog by
taking the three mark names out of a clause that did not define them. `hoard-watch` still names them,
where each mark's action is stated.

**The risk this takes on** is that served text now leans on a field: a build that stopped reporting
`watch_marks` would leave sessions recalling nothing, silently. `every_watch_mark_a_mode_names_is_a_control_mark`
in `tests/conventions.rs` and a recorded mutation hold the field and the marks together.

## One copy of each set

**A set identity is loaded once on a machine, whatever its mode.** Decided 2026-09-16 by the human
partner. Evaluation copies are the exception, and are covered below.

Nothing needs more. A publisher holds their set read-write and a reader holds it read-only, so the two
modes of one set do not meet on one machine. The reference sets read-only exists for — documentation
for a piece of software, company policies, regulations with their case law — need one copy each. And
comparing versions of a set is what [evaluation](#evaluation) is for, under names that say so.

**What it buys is an unambiguous identity.** Every link that carries a set identity, and every
cross-set link naming a read-only set, resolves to exactly one loaded set, by construction, with no
rule about which holder wins. `import` refuses to give a set an identity any loaded set already holds,
whatever the modes of the two.

**At startup, the machine's own copy wins where the modes say which it is** -- and the same rule
settles a set picked up later, when another server sharing the home creates one, since the rescan
before each call loads through the same path. Two stores on disk can still share an identity, left by
an older build or by files handled by hand:

| Stores on disk sharing an identity | Outcome |
|---|---|
| one read-write, the rest read-only | the read-write set loads; the read-only copies are withheld |
| two or more read-write | all withheld, as today — nothing says which is the original |
| read-only only | all withheld, so the rule stays uniform; little rides on it for a reference document |

A withheld store is listed in `hoard_sets status` and reachable by `drop`, and once only one copy is
left it loads. So the lock-out the human partner objected to is confined to two read-write copies, a
state the tools can no longer produce.

**What it gives up:** a machine cannot hold a set read-write and also keep a read-only snapshot of an
earlier version loaded. An evaluation copy does that, labelled as such.

**An evaluation load is never a holder of its source's identity.** A recall that names no set does not
search it, one that names it searches nothing else, and it resolves no outbound link at all — see
[cross-set links](#cross-set-links-are-surfaced-never-resolved). So it is excluded from the rule
above: loading a copy for review must never withhold the set it is a copy of.

## Evaluation

Evaluation is read-only with the additions below. **It is built after read-only, on top of it.**

- **Loaded from a file, once**, by `hoard_sets` action `evaluate`, which takes the file and a label.
  There is no `create` step, because nothing is being taken on: the records go through the same file
  entry point as an import, one at a time, since `Store::import` refuses a file whose scope is not the
  store's and here it deliberately is not. The copy is [a database under the home](#on-disk-under-the-home). The copy adopts the file's identity, so a link naming that
  set can be reported against it, and `Server::settle_identity` and `Server::other_holder_of` both
  leave evaluation stores out — so opening a review neither withholds the reviewer's own set nor stops
  it importing its own file.
- **Several at once.** A review may compare two or more versions of one set side by side, so any
  number of evaluation loads can be open. This is where evaluation differs most from read-only, whose
  uses — documentation for a piece of software, company policies, regulations with their case law —
  need one copy.
- **Searched only when named.** This keeps the maintainer's ordinary recall uncontaminated, including
  its scoring. Scores are computed against all the sets a recall searches, so a loaded copy would
  shift everyone else's scores even if its hits were filtered out afterwards. Comparing two
  evaluation sets therefore means one recall per set, which needs no new tool surface; what each set
  says can be compared, but scores cannot be compared across calls.

### Naming: `eval-<set>-<label>`

**The user chooses the label**, since it has to say which copy this is — a branch, an issue, a
version — and that is theirs to know; the LLM passes it on. The software adds the prefix and the
declared set name, turns the label into a valid set name (a branch such as `feature/x` cannot be
one as it stands), reports the name it produced, and refuses a name already loaded.

**No real set may have a name starting `eval-`.** Otherwise the name could not be trusted as a sign
of what a set is. **The ban sits at three boundaries** — `create`, a file's header, and a link written
into a store — and `SetName::evaluation` is the only constructor that builds such a name.

**Not in `SetName::new`, which was tried first and is worth recording.** `new` is also how a set
*already loaded* is named: `drop`'s argument, a scope in a tool call, the store reading its own scope
back out of `meta`. Refusing there refused a review copy the moment anything addressed it — `drop`
could not name one. The three boundaries are where a *real* set could come to carry the prefix, which
is what the rule is actually about.

The alternative was a third `Scope` kind, or carrying the status outside the name — something like
`scatterhoard (evaluation: issue-3)`. Either makes counterfeiting impossible without reserving a
prefix. A third kind would also reopen the resolution that there are exactly two
([Set Lifecycle](set-lifecycle.md#resolved-2026-08-23-local-is-one-set-not-a-class-of-them)) and
touch every place the code handles a scope, for a guarantee the constructor already gives.

### Marking what is returned

**Every memory returned from an evaluation load carries a field such as `"evaluation": true`, taken
from the store's load mode — never parsed from the name.** The set is loaded because the LLM is
exploring it, so its memories are proposals under review and must not be taken as the project's
settled record. The name is not the source of truth: parsing it would mean every return path has to
remember to, and one that forgot would fail silently in the reassuring direction. The name and the
mode cannot disagree anyway, since the evaluation constructor produces both.

- **The recall tool's description gains one clause** saying what the field means, as it already does
  for `inherited`.
- **A response holding at least one such memory carries a note** explaining it, in the way
  `in_links_note` and `language_note` already work. An ordinary recall pays nothing for it, and the
  explanation arrives exactly when it matters. The convention catalog was ruled out, for its budget
  and because the note already reaches the LLM at the right moment.
- **The field goes wherever a memory is returned**: direct hits, memories reached through links in a
  recall that named the evaluation set, and `hoard_lookup` results. An unscoped recall cannot reach
  an evaluation memory even through a link, because an evaluation store is not among the stores it
  searches. **That is true of recall only**: see [cross-set links](#still-open) below.

### Cross-set links are surfaced, never resolved

**An evaluation set resolves no outbound cross-set link.** Each one is reported instead, carrying the
target's memory id, the set name the link recorded, the target's **set identity**, and **every loaded
scope answering to that identity, with its mode**. Plural, because one identity can be answered by the
reviewer's own read-write set and by several evaluation copies at once — `coding-lessons`,
`eval-coding-lessons-1234`, `eval-coding-lessons-1011`. The mode is there so the list reads as *your
copy, and two under review* rather than as three equivalent names. **Nothing resolves inward either:**
an ordinary recall or lookup resolves nothing into an evaluation set.

**Why this costs a review nothing.** A link's *standing* is still honest: a memory whose replacement is
out of reach already reports `superseded_beyond_this_set`, so a contributor's superseded memory never
reads as current. And `hoard_lookup` by id already returns a hit per loaded set, each labelled by
scope, so following a link by hand shows the branch's copy beside the reviewer's own — which is the
comparison a review is for. The cost is a round trip per link followed, and the compensation is that
nothing is followed silently.

**What it rules out.** Resolving into the reviewer's own set would show their version of a memory as
the target of the contributor's link — one set's concession attached to another set's memory, which is
the hazard `holder_of` exists to avoid. Marking such a hit would make it honest but needs a
distinction the code does not have, between the sets a recall searches and the sets it may resolve
against. Not resolving needs neither, and it removes the need for a *cohort* in resolution: grouping
evaluation loads by their shared label has only one possible job left, letting one recall rank across
two evaluation sets, which is separable and not yet needed.

**One thing the build must not miss:** evaluation copies are excluded from `Server::settle_identity`,
or a loaded evaluation copy would count as a rival holder of its source set's identity and withhold
the reviewer's own set.

### No calibration signal

*Built with the rest: a store in evaluation mode writes no query-log row at all, in
`recall_across_with` (`src/store.rs`).*

**An evaluation set's recalls do not feed floor calibration.** A scoped recall is what calibration
treats as a labelled signal, and naming the scope is the only way to search an evaluation set, so
every query against one would otherwise count. Two reasons not to let them: a reviewer's queries are
exploratory rather than evidence the set answered well, and the measurement that made the scope label
worth using treated it as a *noisy-but-frequent* signal, which an evaluation load makes universal
instead. An evaluation store is in memory and does not outlive the session, so a floor it calibrated
would die with it in any case.

**What this gives up was checked rather than assumed:** on 2026-09-17 no set on this machine held a
floor — `scatterhoard`, `coding-lessons` and `local` each report none, with queries logged throughout.
Calibration would help a review and is not needed to perform one.

### On disk, under the home

**A review copy is a database file in `reviews/` under `SCATTERHOARD_HOME`**, not a store in memory.
Decided 2026-09-18 by the human partner, reversing the in-memory design: Scatterhoard is meant to run
beside a model on the user's own machine, so the server has to be sparing with memory — and **no
portable way exists to ask how much there is**, which makes an in-memory limit a number nobody can
pick. An 8 GB laptop and a 128 GB workstation want different answers; a file wants none.

**Under the home rather than the system temp directory**, because the home is a directory the user
chose and which already holds their stores, so it is known to be sized for them. `/tmp` is commonly a
small root partition, and supporting a `SCATTERHOARD_TEMP` to work around that would be a setting
added to answer a question the home already answers.

**Loaded again at every start**, from its own directory, which is what makes `drop` the single way a
copy goes. The alternative — never loading them — left a file after every restart that only a person
would remove, and made a reviewer re-embed someone else's whole set to carry on. A forgotten review
is then a set listed in `status`, which is visible and removable, rather than a stray file nobody
sees. A user who wants reviews closed promptly can say so in their own prompting; the tool does not
decide it for them.

**Two servers share a home**, so the file is the shared thing: a label another server already holds a
review under is refused here too, naming the label and pointing at `drop`. That is the same refusal a
label loaded in this server gets, for the same reason.

**No size limit, and that is a decision rather than an omission.** An exported file carries no
vectors, so `evaluate` embeds every memory before returning — which is why a cap was proposed. Timed
on 2026-09-18 over this project's own set (948 memories) it does not need one: **0.63 s** in release
under `potion-base-32M`, **0.80 s** under `potion-multilingual-128M`, so a ten-thousand-memory set is
about seven seconds and the *looks like a hang* failure needs a corpus around a hundred thousand. The
models differ by under 30% and swap places between builds, where the build itself costs about nine
times more — debug is 6.5 and 5.5 ms per memory — so the model was not the variable a limit would have
had to account for. A record count would be wrong for the reason the memory cap was: the right number
depends on the machine and the build, neither of which the software can see. **So the response carries
`took_ms` instead**, which puts the cost in front of whoever ran it and accumulates evidence about the
one axis nobody here can test — hardware, which would mean publishing the branch to pull it onto an
older machine. The probe is `probe_what_a_review_costs` in `tests/probes.rs`.

## Resolved 2026-09-30: recipes for users are a published set

*Decided by the human partner, settling `where-user-facing-recipes-live` in [Open questions](../../TRACKING.md#open-questions).*

**Procedures for Scatterhoard's users live in a published set, `scatterhoard-cookbook`, taken on read-only**: the case this mode exists for, a set published to be read. A user has no copy of this repository; what reaches them is the served catalog and the sets they import, so a recipe kept in a document here would not reach the sessions that need it. Recall finds a recipe at the moment it is wanted, which is why each one **opens with the reader's goal or symptom** — *moving your personal memory to another directory*, *your personal memories seem to have disappeared* — since that is how a session phrases the query. A short document points to the set, for the person deciding whether to take it on.

**A recipe is published only after its procedure has been run end to end**, which is the question's own caution: a recipe written before its mechanism has been used is a second copy of untested design. The first is moving the data directories, from `moving-the-local-store-leaves-the-old-one-unmentioned`; the tracking system and the lessons-set adoption mechanism are the candidates after it. **Its file lives in this repository's `scatterhoard_mem`, beside `scatterhoard.json`**, decided by the human partner when the set was created: recipes accumulate with the rest of the project and are committed with the work they describe, and two memory files in one directory is a live test of `several-memory-files-per-directory`.

## Still open

- **Whether one recall can rank across two evaluation sets**, for comparing versions side by side.
  Separate recalls work today and their scores are not comparable across calls.
- **Whether a returned memory also carries its set's identity**, so two copies of one set are
  visibly copies. With evaluation names carrying the source set's name this matters less than it
  did.
- **How a read-only set's signposts get recalled.** The watch lists are not pushed by the server; the
  `recall-when` convention tells a session to recall them, and today only for writable sets. Read-only
  should get its signposts — the marks that ask for a recall before reasoning — and not the collection
  and investigation marks, which ask for writes. That needs either convention text or a `status` field
  naming which marks to recall for each set, and the convention catalog is within a hundred characters
  of its [budget](conventions.md#resolved-the-budget-is-18000-characters). A read-only set's charter
  matters for the same reason: it declares the language a query should be phrased in.
- **The fork question** `how-a-renamed-fork-gets-its-own-identity` is adjacent rather than part of
  this: it concerns a set that should stop sharing an identity, not a second copy of one.
