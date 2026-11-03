# Tool Surface

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

Keep the tool surface fairly simple, clear, and ergonomic for the LLM. We don't know how many other MCP servers the user will load, and too many tools can be confusing. Name the tools clearly, using a consistent prefix (e.g., `hoard_memorize`, `hoard_recall`), and favor active voice and a consistent parameter order.

## Resolved: the tools

| tool | purpose |
| --- | --- |
| `hoard_conventions` | Load the operating conventions at session start, paged against a budget. |
| `hoard_memorize` | Store a memory. |
| `hoard_recall` | Search memory, by meaning **or** by exact keyword — one per call. |
| `hoard_lookup` | Fetch memories by id, or by the start of one. |
| `hoard_correct` | Change a memory that already exists: `amend` its apparatus, or `forget` it. |
| `hoard_sets` | Manage sets rather than memories: `status`, `create`, `export`, `import`. |

The Vision's *store* and *recall*, the convention loading [Trigger and Loading](trigger-and-loading.md) requires before anything else happens, and the two tools that manage what has already been stored. Nothing else until it earns a place — every tool added is a standing charge against the model's attention, competing with whatever other servers the user has loaded.

Listed in the order a session uses them rather than alphabetically: conventions at the start, the store/recall pair, then lookup for when an id is already in hand, then the two that are reached for occasionally rather than throughout.

*(This heading read **"three tools"** while the table listed four, and kept reading that way as the surface grew to six. A count in a heading is a hand-maintained copy of the length of the table beneath it, which is the duplication [STANDARDS.md](../../STANDARDS.md) describes, in the smallest possible form. The heading no longer carries a number.)*

### Resolved: occasional operations group behind an action; constant ones stay flat

`hoard_correct` and `hoard_sets` each take an `action` and dispatch on it, where the first four tools are flat. That asymmetry is deliberate and was proposed in [Set Lifecycle](set-lifecycle.md) as an open question — *is one tool with several actions genuinely easier for a model to use correctly than several small tools?* It is now settled, by having built and used both shapes.

What decided it:

- **The frequency difference is real and large.** Conventions load once a session; memorize and recall run throughout. Lifecycle and correction operations are reached for at setup, at commit time, and when something needs fixing — a handful of times in a project's life. Keeping the constantly-used surface flat is worth more than keeping the rarely-used surface flat.
- **Ten tools would have been the alternative.** Six lifecycle operations plus two corrections, against two tools with an action each. The attention cost this document opens with is paid on every tool, in every session, whether or not it is used.
- **The action enum turned out to carry its own weight**, which was not anticipated. A `oneOf` branch per action lets each action publish *only* its own arguments, so `{"action": "status", "name": …}` is not something the schema invites — where a flat tool with every argument optional is exactly the shape that produced `schema-underspecifies-scope-and-links`. An unknown action can also name the available ones, which a wrong *tool* name cannot.

### Corrected 2026-08-11: the branches inform, they do not constrain, and they cannot stand alone

This bullet read *"cannot be constructed and believed"* until a client proved otherwise, and the overstatement mattered in two directions.

**It was never enforcement.** JSON Schema permits properties a branch does not list unless a branch says `additionalProperties: false`, which these do not. A caller could always have sent `{"action": "status", "name": …}`. What the branches buy is that nothing *suggests* it — real, and worth having, but communicative rather than structural.

**And a consumer may not read them at all.** Published as `oneOf` with no top-level `properties`, `hoard_sets` and `hoard_correct` reached one client as tools taking no arguments — every argument stripped, every call refused, the whole management surface unreachable — and another as a merge of the branches with `action` pinned to the first, able to express only `status`. Same publication, both entitled to read it that way. `oneof-tool-schemas-present-no-parameters`.

So a flat top-level view is published: every argument, `action` as an `enum` over all of them, and the per-action combinations restated in prose. It is **derived from the branches** rather than written twice, so an action added to one appears in the other. Restating the combinations in words costs nothing that was ever enforced.

The general form is worth keeping, because it is not about JSON Schema: **a publication read by software you do not control has to degrade into something usable, not into something empty.** The precise statement can be the *best* answer available; it cannot be the only one.

### Corrected again the same day: a top-level `oneOf` is refused outright, so the branches are not published at all

The section above kept the branches and added the flat view *beneath* them, on the reasoning that a consumer honouring `oneOf` should still get the precise statement. **That was still too generous to the alternation, and by a margin that matters.**

The Anthropic API rejects the keyword outright:

```
400 tools.N.custom.input_schema: input_schema does not support oneOf, allOf, or anyOf
    at the top level
```

Three things make this decisive rather than another compatibility annoyance.

- **Publishing `properties` alongside does not help.** A reported case published `anyOf` *and* `properties` and was refused just the same. The keyword's presence at the top level is the violation.
- **The blast radius is other people's servers.** A client that forwards MCP schemas to the API unmodified re-sends the whole tool list on every request, so one offending schema fails *every* tool from *every* registered server, for the rest of the session. Two public reports describe exactly that, one from a calendar connector and one from `browser_click`.
- **It had been surviving on a courtesy.** Scatterhoard's tools worked only because something between the server and the API happened to normalise them away. That is precisely the dependency this section was written to warn about, left in place by the fix that named it.

So the branches are now internal: they remain the single source for the flat view, for the `action` enum and for the actions a refusal names, and they never reach the wire. Held by `no_tool_publishes_top_level_alternation` and a recorded mutation.

**Nested alternation is untouched by *this* decision.** `scope` and `links.target` keep their `oneOf` here, because the API's restriction is explicitly *at the top level* and it accepts them nested.

*Superseded within the day by [publish the centre of MCP, accept the edges](#resolved-publish-the-centre-of-mcp-accept-the-edges), which removes them anyway — not because the API refuses them but because a client discards them, and the client in question is the one a global installation uses. The paragraph that stood here said this was "not something the wire format should be redesigned around", on the belief that only a duplicate declaration was affected. Testing Claude Desktop directly settled that it was not.*

*Worth recording that the correction above was written to be more careful than the thing it corrected, and still under-stated the constraint: it said consumers "vary in whether they honour" the branches, where the truth is that the format's largest consumer forbids them. Being more defensive is not the same as checking.*

The counter-argument that a wrong action value and a wrong tool name are equally available mistakes held up, but turned out not to matter: both are refusals, and the action refusal is the more helpful of the two because it can enumerate what would have worked.

## Resolved: publish the centre of MCP, accept the edges

**Decided 2026-08-11, after two clients disagreed about the same schema.** The corrections above were each a repair to one construct in one place. This is the rule they were instances of, arrived at by surveying the whole surface rather than the parts that happened to break.

### What was measured

Two clients, the same binary, the same session. Every difference is in how each treats the parts of JSON Schema that express *alternatives*:

| Construct | Client A (project declaration) | Client B (Claude Desktop declaration) |
| --- | --- | --- |
| `type: string` / `object` / `array`, `properties`, `required`, `items` | preserved | preserved |
| `enum` | preserved, all fourteen link labels | preserved |
| `description` | preserved | preserved on `action`, dropped elsewhere |
| **top-level `oneOf`** | merged into flat `properties`, with a generated prose note | dropped |
| **nested `oneOf`** (`scope`, `links.target`) | preserved | **reduced to `{}`** |
| **union `type: ["string","null"]`** (`context`) | preserved | **reduced to `{}`** |
| `additionalProperties: false` | preserved | rewritten to `{}` |
| `type: integer` | preserved | rewritten to `number` |

And underneath both, a hard limit that is not a client's choice at all: **the Anthropic API rejects `oneOf`, `allOf` and `anyOf` at the top level of a tool's `input_schema`** and fails the entire tool list — every tool from every registered server — for one violation.

The consequence measured on the reduced side is not cosmetic. A property published as `{}` carries no type, so an object argument is serialised as text and arrives as a string: `scope: {"shared": "scatterhoard"}` comes back as `unknown variant '{"shared":"scatterhoard"}'`. **On that client no shared set can be named at all.**

### The rule

**Publish only what survives; accept every spelling a client might produce.**

- **Publishing** stays inside the subset both clients preserved intact: a single scalar `type`, `properties`, `required`, `items`, `enum`, `description`. No alternation anywhere — not at the top level, where it is refused outright, and not nested, where it is silently emptied.
- **Accepting** is deliberately wider than publishing. Every form a client might send is parsed, including the ones no longer advertised.

### Why the usual default inverts here

A server is normally right to be strict, because a strict server is what stops clients drifting into private dialects — the discipline is the point, and tolerating a malformed request today is what makes it permanent tomorrow.

**That argument assumes influence, and this server has none.** Scatterhoard cannot cause a client to change; refusing an object-valued `scope` would not teach anyone anything, it would only mean the memories are unreachable from that client. Where the strictness cannot land on anyone who could act on it, all it does is convert someone else's transformation into your failure. So the asymmetry is not laxity: **be strict where strictness is a signal, and liberal where it is only a refusal.**

The cost is real and worth stating: two accepted spellings for one concept can drift, and the older one has no published description to keep it honest. That is paid for with a round-trip test over both spellings rather than with a comment.

### What it changes

Two values are alternations today, and both become scalars — **as published**. The wire format is untouched, because these are different boundaries:

- **`scope`** publishes as a string: `"local"` or `"shared:<name>"`. This is **not a new encoding.** `scope_key` and `scope_from_key` in [store.rs](../../src/store.rs) have used exactly that spelling for the store's `meta` row since the beginning — `select * from meta` on any store returns `shared:scatterhoard` — and it round-trips in both directions already. What changes is that an encoding the project already relies on internally is promoted to the tool boundary. It is unambiguous by construction: `SetName` permits letters, digits, `-`, `_` and `.`, so a set name can never contain the delimiter.
- **`links.target`** cannot become a string without losing something, so it does not become one. The object form carries `set`, `id` and `keywords`, and those keywords exist *"so a reference that goes dangling has something to be re-found by"* — a `"set:id"` spelling would drop exactly the field that makes a dangling cross-set link recoverable. Instead the alternation moves out into sibling scalars on the link itself: `target` is always an id, with optional `target_set` and `target_keywords` beside it. **Objects with named properties survive both clients**; only the alternation inside them does not. Nothing is lost and nothing needs a second spelling.
- **`context`** publishes as `string` and keeps accepting `null`, which is how it is cleared.

*The `target_set` shape does make an invalid combination constructible — a `target_set` with no `target`. That is the objection `schema-underspecifies-scope-and-links` raised, and it is accepted here for a reason that did not apply then: the alternative is not a more precise schema but an emptier one, since the client that would have enforced the distinction is the client that discards it. A combination the schema no longer forbids is refused by the parser and named in the response.*

### What this retires as a side effect

`const` (18 uses) and `title` (8) exist only inside `oneOf` branches, and `additionalProperties: false` (10) only on the object branch of `scope`. Removing the alternations removes all three without a separate decision. `type: integer` survives one client and is rewritten to `number` by the other, which costs nothing and is left alone — a note rather than a change, so that the next person to see it does not read it as an oversight.

### The general form, which is not about JSON Schema

**A publication read by software you do not control has to degrade into something usable, not into something empty.** Stated in the correction above, and this is the same rule applied before the fact rather than after: the question is not whether a construct is *legal*, it is whether it is *reliably carried*. Legality is decidable from a specification; carriage is only decidable by measurement, and the measurement has to be repeated when the surface changes. `no_tool_publishes_top_level_alternation` and the survey it grew out of are that check made permanent.

### Resolved: correction is its own tool, not a set operation

`hoard_correct` is separate from `hoard_sets` because it acts on **one memory**, where every `hoard_sets` action acts on a whole set. Grouping them by "things you do occasionally" would have put two unrelated scopes of effect behind one name.

Note that the third correction operation, **supersession, is not here at all** — it is a link written at store time through `hoard_memorize`, because superseding *creates* a memory where amending and forgetting change or remove one. [Corrections](corrections.md) settles which is proper when.

### Why lookup is a tool and keyword search is a parameter

Keyword search is *search* — it produces ranked results and resolves validity exactly as semantic recall does, so it belongs on the same tool, as a second way of asking the same question.

Direct reference is *retrieval*, and it behaves differently in the one way that matters: it is **exempt from supersession substitution** ([Recall Mechanisms](recall-mechanisms.md)). Making it a parameter on `hoard_recall` would give one tool two contradictory behaviours depending on which argument was supplied — a superseded memory swapped for its replacement or returned as asked, decided by a field. That is a worse cost than one extra tool.

### One search per call: a query or keywords, never both

`hoard_recall` takes **either** a `query` or `keywords`, and refuses a call carrying both. Results come back in a single `memories` list.

A match count and a cosine score are different scales, and blending them into one ordering would be the same mistake that made cross-store scores incomparable ([Embeddings and Vector Storage](embeddings.md)). The original answer to that was to run both searches and return two lists — semantic hits in `memories`, keyword hits in `by_keyword`, the caller sees both and decides.

**That answer was wrong, and using it is what showed why.** The combined call ran the two paths independently and stapled the results together. It integrated nothing, so it produced nothing a caller could not get from two calls — while creating three problems at once: a memory found both ways came back twice in full (measured at ~1,100 duplicated bytes carrying ~30 bytes of new information), the response held two scales with no stated relationship, and any attempt to order the combined result had to either invent a blended ranking or exile keyword-only hits to the bottom. Refusing the combination makes all three **unrepresentable** rather than fixed, which is the correctness-by-construction posture the rest of the design takes.

The cost is one extra call when a caller genuinely wants both, and the loss of one real signal: a memory found by *both* paths is stronger evidence of relevance than one found by either, and splitting hides it. That is recorded as the reason to revisit, in [Open questions](../../TRACKING.md#open-questions) under *hybrid search*.

**Keyword search is not redundant with semantic search, and this was measured rather than assumed.** Keywords do reach the embedding — [Embeddings and Vector Storage](embeddings.md) builds the embedded text from content, then `why`, then keywords — but five keyword tokens averaged into a paragraph-length vector barely move it, and a static embedder represents a rare identifier poorly. Queried by meaning for a term carried *only* as a keyword, the memory holding it ranked **second, beneath an unrelated memory**. Exact match does work the embedding cannot, which is why both paths exist and why the answer was to separate them rather than to drop one.

## Built 2026-10-15: two protocol eras, and a name only the human gives

**The server speaks revision 2026-07-28 beside the handshake revisions**, choosing by how each request
arrives: a request naming that revision in its `_meta` is served statelessly under it, and an
`initialize` selects 2025-06-18 or 2024-11-05 as before. On 2026-07-28 a shared set's first write, and
`hoard_sets` `mint`, `rename` and `identify`, ask the human through an elicitation form where the
client declares one, and a name or record the model supplied is set aside. Everywhere else they take
it from the model, and `mint` and `rename` end with a sentence for the session to relay. The design,
the probes behind it and where the build departed from the proposal are in [Asking the
Human](asking-the-human.md).

## Resolved 2026-10-03: what each tool returns is published, and checked against every test response

**Every tool that returns JSON publishes an `outputSchema`, and returns `structuredContent` beside its text**, so a client can learn the response shape from the contract instead of by provoking each case. `response-shape-is-unpublished` in [Issues](../../TRACKING.md#issues). The schemas are built in [the output-schema module](../../src/output_schema.rs); `hoard_conventions` publishes none, since its pages are text.

- **The tests are strict and the contract is lenient.** Every tool response in the approval, MCP and sets suites is checked against its tool's schema, and a field the schema does not declare fails the test, so a new field cannot arrive undocumented. The published schemas allow undeclared fields, so no client breaks on one added later. The checker is [a small one in the tests](../../tests/support/output_schema_check.rs) that understands exactly the six keywords the schemas use and refuses any other.
- **They were written from evidence.** Every response the suite produces, 2,383 of them, was recorded and summarised into a field tree per tool and per `hoard_sets` action, and cross-checked against every field the handlers set. The survey found a live defect on its first pass: recall spelled `referenced_from[].scope` as an object where lookup spelled it as the published scalar, the drift `WireScope` fixed elsewhere.
- **`required` names only what is present on every response**, checked against the code, because a wrongly required field is the one mistake that breaks a strict client.
- **Descriptions say only what a name and type cannot**: when a field appears and what an absence or null means. Restating every field nearly tripled `tools/list`, which a host may put in a model's context.
- **Not on 2024-11-05.** The server negotiates the protocol version, answering the one asked for when it speaks it and the newest otherwise, and a client on 2024-11-05 gets neither schemas nor structured content — which is also how a client avoids their cost. A request on 2026-07-28, which [Asking the Human](asking-the-human.md) added on 2026-10-15, gets both.
- **An output schema's top level must say `"type": "object"`.** Claude's client refused the whole tool list on 2026-10-03, taking every tool offline, because two schemas opened with `anyOf` alone; a test pins it.
- **For the model reading a response**, which a host may never show the schema, the recall description says the one thing a reader acts on: a replacement carries `matched_similarity` and `replaces_matched` in place of `similarity`. Not tested on a local model before it shipped.

## Resolved 2026-10-01: a convention page is a slice of one document, ended by an instruction

**`max_chars` is a true bound, and a page says in words what to do next.** `max-chars-is-not-a-bound` in [Issues](../../TRACKING.md#issues): the pager served whole conventions, so a budget smaller than one convention returned that convention anyway, up to eight times over budget, while the parameter was described as the page's budget.

- **The unit is a character, not a convention.** The catalog is serialised once as compact JSON, `{"conventions":[…]}`, and each page's first line is a slice of it cut wherever the budget falls. Joined in order the slices are that document, so no unit is too large to split and nothing forces an overrun. The cursor is a character offset into it. The design was the human partner's: an unclosed JSON list carried across pages.
- **The budget counts the whole page**, the page's own lines included, in characters. A budget too small to hold those lines and one character of the catalog is refused, naming the least that works.
- **With no budget the whole catalog arrives in one page**, which is the case for nearly every session.
- **What the server says beside the catalog** — the rebuilt-binary `note`, the owed set descriptions — travels as one JSON line between the slice and the last line, and is spent from the same budget.
- **The last line is an instruction, and its wording is evidence.** With more to come it reads *"The conventions are not loaded yet. Call hoard_conventions again with {"cursor": N, "max_chars": M} for the rest."*; on the last page, *"All N conventions are loaded."* with N counted Tested against `gemma4:12b` before it was chosen: a page that only *showed* more was coming — a cursor, unclosed JSON, "1 of 11", a terse status line — was taken as the end every time with thinking off, and this sentence after the catalog was followed every time, both arguments included. The same sentence before the catalog failed, as did a variant that had worked under a different tool name. **Re-test before rewording it**, with `python3 scripts/local_model.py page`; the doc comment on `more_follows` in [the conventions module](../../src/conventions/mod.rs) says the same, and the measurements are in the `scatterhoard` set under `gemma-reading-test`.

**Cost to a client that parsed the old response:** a page is now text rather than one JSON value. Generic MCP hosts pass text through; a program joins the first lines of its pages and parses the result.

## Resolved 2026-09-12: a server reports that its own binary has been rebuilt, on `hoard_conventions`

**An MCP client serves whatever binary it launched with**, so rebuilding the source changes nothing a running session sees. That is ordinary, and would not need a mechanism except for where it lands: the conventions are the steering input, so a stale catalog does not degrade anything visibly. It answers every call successfully and simply governs. Three occurrences are recorded under `detect-a-stale-running-server`, and on the third the served text contradicted that day's headline decision without anyone noticing.

**The tell everyone reaches for first cannot fire.** *Fewer conventions than the source defines* moves only when one is added or removed; on 2026-09-07 the count was right and the text was a day old, which is the common case, because most changes are edits.

**The server compares its own binary against how it looked at launch**, and reports on `hoard_conventions`. Three things decide that placement. It is the call a session makes first, so the notice arrives before anything has been reasoned from the old text. It is the surface whose staleness is most dangerous, so the notice sits on the thing it is about. And it is the one call every session is told to make — `hoard_sets status` would be the tidier home for operational state and is read by almost nobody, which is the property that matters here.

**The notice sits beside the catalog, never in place of it.** A session that cannot read the conventions is worse off than one reading conventions a build behind, so the page is served either way and the note says what to do: the tools cannot be fixed from inside the session, and the remedy is restarting the client.

**Compared, not hashed.** Modification time and size, two fields of one `stat`, asked at every session's first call. Size is there because time alone is a poor equality test on a file a build tool replaces — filesystems differ in resolution and a rebuild landing inside one tick would read as unchanged. The approximation errs one way: a rebuild that produced an identical binary still reports, so a session is occasionally told to restart when nothing changed. A real change going unreported is the failure this exists for; being told to restart unnecessarily is not.

**A server that cannot see its own binary reports nothing**, rather than guessing or failing. `std::env::current_exe` can be unavailable, and the tools work unchanged without this.

## Resolved: transport is not part of the tool surface

The server is a **pure function from a JSON-RPC request to a response**, with stdio handled separately in the binary. Two things follow, and both matter more than the small amount of glue it costs:

- The tools are testable without an MCP client or an async runtime, so the parts with decisions in them are exercised directly.
- Adopting an SDK, or a different transport, becomes a change to the glue rather than a rewrite of the decisions.

## Resolved: arguments are validated by being parsed

Tool arguments deserialise straight into the memory model's types, whose constructors already validate. An out-of-range confidence or a malformed set name is refused because the *type* refuses it, not because the tool layer checks — parse-don't-validate reaching the outermost boundary.

The important case is scope. `Scope` has no `Default` ([Memory Steering](memory-steering.md)), so a call omitting it **cannot be deserialised at all**. The "explicit, never inferred" rule therefore reaches the tool boundary with no check anyone could forget — confirmed by mutation: making the parameter optional does not fail a test, it fails to compile, because there is no function that guesses a scope.

## Resolved: a failing tool answers, it does not break

A tool that cannot do what was asked returns a result marked `isError` rather than a JSON-RPC protocol error. The model should see the failure as an answer it can act on — a wrong set name, a missing model — rather than as a broken call it can only retry.

Two consequences of the same principle:

- **Naming a set this machine does not hold is an error, not an invitation to create one.** Inventing a store on a typo would silently split a set in two. **But the refusal has to name an action the caller can take**, which it did not: it said *"check the set name"* with no way to list names, and *"have to be imported"* with no import tool — two dead ends in one sentence, tracked as `no-tool-lists-loaded-scopes` and fixed by pointing it at `hoard_sets`. A refusal that is correct and unactionable is only half a refusal.
- **A missing embedding model does not fail the call.** Recall returns what it can and says, in words the model will read, that the store could not be searched by meaning and that memory should be treated as unconsulted rather than empty. See [Embeddings and Vector Storage](embeddings.md) on why that has to be loud.

## Resolved: scope is required to store, optional to recall

`hoard_memorize` requires a scope; `hoard_recall` defaults to every loaded set. The asymmetry is the point.

Storing into the wrong set is a **one-way door** — a personal memory committed to a shared repository cannot be taken back — so the choice is required and never guessed. Searching the wrong set is a read: it costs nothing and returns less. And **project ∪ local is the normal case**, since a stated preference matters while working on a project, so the default is what a caller almost always wants.

This became available once cross-store scores were made comparable by centring every searched store on the mean of their union (see [Embeddings and Vector Storage](embeddings.md)). Before that, an implicit union would have had a ranking nobody could defend. Every result reports which set it came from, so a local preference and an imported project decision never arrive indistinguishable.
