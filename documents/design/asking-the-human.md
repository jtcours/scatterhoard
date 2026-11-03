# Asking the Human

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

How the server gets an answer that only the human has, without the model supplying it. The first such
answer is **the name a person writes under in a set**, and it is the one this document is built
around; the mechanism is general, and the [tool surface](tool-surface.md) is where any later use would
be recorded.

**Status: built, 2026-10-15.** Proposed 2026-10-11, and its fallback built that day; the rest was
built on 2026-10-15, as [Built 2026-10-15](#built-2026-10-15-what-the-build-changed) records, which also
says where the build departed from this proposal. Whether a real client puts the question to the human
is in [EARLY-VALIDATION.md](../../EARLY-VALIDATION.md), since no client has yet. Tracked as
`asking-the-human` in [Backlog](../../TRACKING.md#backlog).

## The problem: a name only the human has, chosen by the model

**A shared set's first write is refused until someone is named for it**, decided 2026-09-24
([Set Lifecycle](set-lifecycle.md)): the name is how a person is known where the set is published, which
varies by set, so no machine-wide default can be right. The refusal points to `hoard_sets` action `mint`,
whose `author` parameter takes that name as free text **from the model**.

**That is a decision placed where the information is not.** [Decision Placement](decision-placement.md)
asks whether whoever decides has what they need to decide correctly. The model does not know the name; the
human does. The refusal and the parameter description both say to ask, and the design has relied on that
being read and obeyed.

**It is not obeyed reliably, and wording does not fix it.** Probed on `gemma4:12b` on 2026-10-10, ten runs
per text, on a task whose repository sits under someone else's organisation (`github.com/acme-corp/widget`):

| Refusal text | Asked the human | Minted a name it made up | Minted `acme-corp` |
|---|---|---|---|
| Today's | 0 | 10 | 0 |
| Plus a hint naming the remote's owner | 0 | 0 | 10 |
| Best of six variants: stop and ask first, the owner offered only inside the question | 6 | 0 | 3 |

The failure is restraint, not discovery: the model never struggles to *find* a name, only to hold back
from using one. Removing the instructions for `mint` from the refusal did not help, because **the tool
schema itself carries them** — `hoard_sets` lists `mint` with a free-text `author` in every turn's context.
A hint naming a concrete candidate made things worse in a way that matters more than the count: it turned
an obviously wrong author (`coding-agent`) into a plausible wrong one (`acme-corp`), which a reader of the
set will not catch. The probes are recorded in the `scatterhoard` set under `author-default-from-remote-url`.

**So the fix is structural.** [Decision Placement](decision-placement.md) says that once a decision's
proper home is settled, the wrong placement should be made unrepresentable rather than discouraged. Here
that means **the name never passes through the model**: there is then no string for it to invent, and no
plausible stranger's handle for it to substitute.

## Proposed: the server asks, and the model never supplies the name

MCP's **elicitation** lets a server ask the human a question through the client: the client shows a
form, and the answer goes back to the server without the model producing it. Claude Code shows form
elicitation as a dialog ([Claude Code: MCP](https://code.claude.com/docs/en/mcp), *Respond to MCP
elicitation requests*). A name or username is explicitly permitted in form mode; the specification's
prohibition covers secrets and credentials.

### What the tools ask, and when

**The name is asked for at the moment it is first needed**, which is the moment the design already uses:
a set is settled at its first write.

- **A shared set's first write** (`hoard_memorize`, or anything else that writes) asks the human for the
  name, then mints the identity record and completes the write in the same call. The refusal disappears
  for a client that can ask.
- **`hoard_sets` action `mint` loses its `author` parameter.** It asks instead. It stays as an action for
  a session that wants to settle identity before writing.
- **`rename` loses its `author` parameter** for the same reason: the new name is as much the human's as
  the first one.
- **`identify` asks the human to choose among the set's `hoard-author` records**, rather than taking a
  record id from the model. It is the same mistake one step removed — a model choosing which existing
  person the human is — and the form can offer the records as a single-select list, with a further choice
  for *none of these, I am new here*, which leads to the name field.
- **`local` is unchanged.** It is never published, takes the operating system's account name, and has
  nothing to ask.

Removing `author` from the schema is what closes the route the probe found. A description that says *ask
the human* beside a parameter that accepts any string is the arrangement that failed.

### The form

One required string field, titled for the human rather than the model, with a message that says which set
it is for and why the name matters: *the name you are known by where `shared:<set>` is published — an
account name on one host, a full name somewhere else. It is recorded on every memory you write to this set.*

**The hint the first draft gave the model goes to the human instead**, as the field's description: *for a
set published from a repository, this is often the account or organisation the repository is hosted
under.* The human can judge whether that is them; the model could not.

### What pre-fills it

**Only a value derived from the set's own recorded location**, and nothing else. The store's `bases`
table holds every path the set has been exported to or imported from. Where one of those lies in a
repository whose origin URL parses, the owner component is offered as the field's `default`. Where there
is none — the usual case for a new set at its first write — the field is blank.

**Not the server process's working directory.** A stdio server's working directory is wherever the client
launched it, which may be a different repository from the one the set is published from. A pre-filled
value is one a hurried human accepts, so a wrong default is worse than none: it reintroduces, through
inattention, exactly the substitution the form exists to prevent.

### Decline and cancel

**Nothing is written, and the result says so.** A decline is reported as the human's choice: the session
should not ask again in the same breath, and should not try to write under any name. A cancel is reported
as unanswered, and the next write will ask again. Neither falls back to a default.

### What the model sees

**Only the final result, unless the server says more.** *Corrected 2026-10-16: this section first said
the model sees that a question was put to the human, and a live test showed it does not.* A client that
shows the form retries the call itself, so the model makes one call and gets one result; the question,
and the human's answer, pass between client and server without it. A session that wrote a set's charter
this way found the set attributed to a name it had never seen, and could not account for it. **So a write
that settles who writes to a set from the human's form says what they answered**, in an `author_note`
on its result: the name they gave, the record they chose, or that they are new, and that the form was
one the model did not see. A refusal after an unanswered form already says what happened.

The model never sees a field it could fill. A client that lets a model construct the retried request
would defeat this; that is a defect in the client, which the [protocol](#the-2026-07-28-protocol-layer)
assigns to the client, and is out of reach of anything a server can do.

## How the question reaches the human, revision by revision

The server negotiates the protocol revision per connection. Elicitation does not exist on every revision,
and where it does, it does not travel the same way.

| Revision | Elicitation | How it travels | Proposed here |
|---|---|---|---|
| 2024-11-05 | none | — | [the fallback](#decided-2026-10-11-where-nothing-can-ask-the-model-names-and-the-human-is-told) |
| 2025-06-18 | form mode | the server sends `elicitation/create` as its own request in the middle of a tool call, and waits for the client's response on the same stream | [the fallback](#decided-2026-10-11-where-nothing-can-ask-the-model-names-and-the-human-is-told) |
| 2025-11-25 | form and URL mode | as 2025-06-18 | not spoken today; nothing here changes that |
| 2026-07-28 | form and URL mode | the tool call returns an `InputRequiredResult`; the client asks the human and retries the call with the answer attached | **the route** |

**On 2026-07-28 it fits the server as it is.** The server answers requests and never sends one: its stdio
loop in [main.rs](../../src/main.rs) reads a line, answers it, and reads the next. The multi round-trip
pattern keeps that shape — the first answer is an `InputRequiredResult` rather than a result, and the
retry is an ordinary `tools/call` carrying `inputResponses`. The retry repeats the original arguments, so
the server needs no state between the two calls and no `requestState`; the specification requires one of
`inputRequests` or `requestState`, and the form is the former.

**The answer is read from `inputResponses` in the request's parameters, and never from `arguments`.** The
model writes `arguments`; the client writes `inputResponses`. A model that put a name into its arguments
under the same key must not be mistaken for a human answering.

**A client that does not declare the `elicitation` capability on the request gets the fallback**, on any
revision. The specification forbids sending an input request the client has not declared, and the
capability travels on every modern request, so this is decided per call.

### Why not the mid-call request on 2025-06-18

**It would make the server wait on its own input stream in the middle of answering**, which the loop has
never done. Between sending `elicitation/create` and receiving its response, other messages can arrive —
a cancellation, a ping, in principle another call — and each would need handling inside a handler that is
itself waiting. That is a different server architecture, and the revision it serves has already been
superseded: 2026-07-28 removes server-initiated requests outright. Claude Code's newer runtime asks stdio
servers for 2026-07-28 and uses it where they support it, so the clients most worth reaching are reached by
the route that fits.

**What this leaves on the older revisions** is the fallback, for every client that stays there. That
population is not known; Claude Desktop's chat was not found to support elicitation on 2026-10-10.

## Decided 2026-10-11: where nothing can ask, the model names and the human is told

**On 2024-11-05, on 2025-06-18, and on any connection whose client does not declare elicitation, the
server cannot ask.** There, the model still supplies the name, and **the safeguard moves from before the
write to just after it**: the human is told what was recorded, in words that invite a correction, and
`rename` makes the correction.

- **`mint` keeps its `author` parameter on those connections**, and only there; on a connection that can
  ask, the parameter is absent. The tool list already varies by revision, which is how output schemas
  reach 2025-06-18 and not 2024-11-05.
- **The first-write refusal is rewritten in the pattern that did best in the probe**: stop first, say the
  name is one only the human can give, offer any candidate only as something to put in the question, and
  say how to `mint` only after that.
- **The `mint` result ends with a sentence for the model to relay verbatim**, built from what was recorded:
  *End your reply to the human with this sentence: I recorded "‹name›" as the name you write under in
  ‹set› -- if that is not how you are known where this set is published, tell me the right name and I will
  change it.* `rename`'s result does the same with the new name.

**Why this rather than having the human run a command**, which keeps the name out of the model on every
connection: the human partner judged that friction too high for what it buys. The agents most users will
run have followed the existing instruction to ask without trouble, so the residue falls mostly on weak
models; and a wrong name is not stuck, since `rename` supersedes the one identity record rather than
rewriting what points at it. What the fallback has to guarantee is therefore not that the name is right,
but that **a wrong one is seen while the human can still fix it cheaply**.

**Probed on `gemma4:12b` on 2026-10-11**, from a run that had already minted `acme-corp` against the
human's interest, carried on until the model replied in text:

| `mint` result | Final reply named the recorded author | …and said it could be changed |
|---|---|---|
| Today's note | 7 of 10 (9 of 10 in a second run) | 0 |
| An instruction to tell the human, in prose | 10 of 10 | 4 |
| A sentence to end the reply with, verbatim | **20 of 20** | **20** |

**What remains is a window, not a hidden error**: a wrong name is recorded and is visible in the reply
that recorded it, until the human answers. Memories written in that window refer to the identity record,
so a `rename` corrects them too.

**Rejected: the human names themselves, outside the conversation** (`scatterhoard mint <set> <name>`),
for the friction above. **Rejected: a model-supplied name marked unconfirmed until the human confirms
it** — it keeps the record honest about where the name came from, but adds a state every reader of every
identity record must understand.

## The 2026-07-28 protocol layer

A connection on 2026-07-28 follows that revision's rules throughout, not only for elicitation. **Almost
all of it sits in the request envelope, not in the tools**: names, schemas and results carry over.

### Era selection: both, side by side

The specification calls an implementation that speaks both handshake-based and per-request revisions
**dual-era**, and allows a server to be one. A dual-era server chooses by how each client opens:

- **An `initialize` request selects the legacy revisions**, served exactly as today, for the life of the
  process.
- **A request carrying `io.modelcontextprotocol/protocolVersion` in its `_meta` is served statelessly**
  under 2026-07-28.

A client that speaks both probes a stdio server with `server/discover` and falls back to `initialize` on
any error that is not a recognised modern one. The server today answers an unknown method with `-32601`,
which such a client reads as *legacy* — so a server that adds nothing keeps working with every client it
works with now.

### What every modern request and result needs

- **`server/discover`**, which a modern server must implement: its supported versions, its capabilities
  (`tools`), and its `serverInfo`, with caching hints.
- **The version and the client's capabilities read from each request's `_meta`.** A request missing either
  required field is refused with `-32602`. A version the server does not speak is refused with
  `UnsupportedProtocolVersionError` (`-32022`), listing the versions it does.
- **`resultType`** on every result: `"complete"`, or `"input_required"` for the form.
- **`ttlMs` and `cacheScope`** on `tools/list` and `server/discover`. The tool list is the same for every
  caller and changes only when the binary does, so `"public"` with a long freshness is accurate. It must be
  returned in a fixed order.
- **`serverInfo` in each result's `_meta`**, which the revision recommends.
- **No `ping`.** The revision removes it; the legacy side keeps answering it.

The error codes the server emits today, `-32700` and `-32601`, are standard JSON-RPC codes outside the
range the revision reserves, so none needs renumbering.

### What the server does not implement, and need not

Resources, prompts, subscriptions and change notifications, logging, sampling, roots, tasks, and
everything specific to HTTP or to authorization. The tool list does not change while a server runs, so
there is nothing for a subscription to report.

### What the server holds between requests, which the revision forbids relying on

- **The negotiated version** is held on the server (`Server::protocol`, set by `initialize` and read by
  `tools/list` and every result that decides whether to publish output schemas). Under 2026-07-28 it comes
  from each request instead, and a request on the modern side must never read what an earlier `initialize`
  set.
- **The query log's `session`** treats one server process as one session, which bounds the `used` label
  that calibrates the non-match floor ([Recall Mechanisms](recall-mechanisms.md)). The revision states
  plainly that a stdio process is not a conversation. That was already known — a stdio server process does
  not correspond to a conversation — and changes nothing here, but the revision removes any remaining
  reason to hope otherwise.

## Testing

**The form is tested by building modern requests in the test suite**, as the existing suites build legacy
ones: a first write with the capability declared returns an `InputRequiredResult` and writes nothing; the
retry with an accepted answer mints and writes; a decline and a cancel write nothing; an answer placed in
`arguments` instead of `inputResponses` is not taken as one; a request without the capability gets the
fallback. Each guard gets a mutation in [scripts/mutations.py](../../scripts/mutations.py).

**The era selection is tested from both sides**: `initialize` followed by today's traffic is unchanged
byte for byte against the approval snapshot, and a modern request without `initialize` is served.

**The small-model probe is no longer the test for the modern route**, since no wording is involved. It
remains the test for the fallback's two texts, the refusal and the sentence to relay; the task is
`first-write-to-new-set` in [corpus/local-model-tasks.json](../../corpus/local-model-tasks.json). The relay
probe carries a run on past the `mint` call to the model's final reply, which the harness's one-step
`retry` mode cannot do, so it belongs in [scripts/local_model.py](../../scripts/local_model.py) as a mode
of its own.

**Before any of it, one fact is checked rather than assumed**: what Claude Code actually declares to this
server, on which revision, read from the requests it sends.

## Order of work

1. **Record what clients send.** The initialize parameters, and on a modern request its `_meta`, logged to
   stderr, so the client capability claims above are observed rather than read from documentation.
2. **The dual-era envelope**, with no tool changes: `server/discover`, per-request version and
   capabilities, `resultType`, caching hints. Legacy traffic unchanged.
3. **Asking**: the form on first write, and the changed `mint`, `rename` and `identify`, on the modern
   route.
4. **The fallback**: the rewritten refusal, and the sentence to relay on `mint` and `rename`.
   Independent of steps 2 and 3, so it can ship first — it improves every connection on the day it lands.
5. **The pre-fill from `bases`.** Last, because it is a convenience on top of a mechanism that is correct
   without it.

## Built 2026-10-15: what the build changed

**Built as proposed**, with two departures, both found while building:

- **On 2026-07-28, `author` and `record` are optional rather than absent.** The tool list must be the
  same for every caller -- the revision says it may not vary by connection, and clients cache it -- while
  whether a client shows forms is declared on each request. So the argument stays in the schema,
  optional, with a description saying it is used only where a refusal says the connection cannot ask;
  and **where the human can be asked, a name or record the model supplied is set aside**, never used
  and never used as the form's default. What closes the route the probe found is that the server does
  not read the argument there, not that the schema lacks it. On the handshake revisions the arguments
  are required, as before.
- **Nothing pre-fills the form.** The proposal pre-filled the name from the set's recorded export
  locations. Building it showed that a set records no locations until it holds memories -- an export of
  an empty set writes no bases -- so a new set never has one at its first write; and the sets that do
  have locations before anyone here has written to them are sets taken on from someone else, where the
  repository's owner is the *publisher*. The default would have offered exactly the stranger's handle
  the form exists to keep out. The hint stays, as the field's description, where the human reads it.

**And two things the proposal did not foresee.** `create`'s note and `status`'s author note had
told the model to ask the human and call `mint`; where the first write will ask the human itself, both
now say so and say not to ask, since a model that asked as well would be asking twice. And a `mint` or
`rename` that the store would refuse anyway -- this machine already writes to the set, or has no
identity there to rename -- is refused *before* the form, so nobody fills one in for nothing.

**And one the first live test found, 2026-10-16: a client that declares forms and cannot show them.**
Claude Code running in print mode -- which is how the desktop app's Code tab runs it -- declares
`elicitation: {form, url}` on every request, and answers every form with a decline the human never
sees; its log says *Elicitation request received in print mode*. The server cannot tell that decline
from a human's, since the answer is identical and nothing on the request says print mode. Strict, a
new shared set could never be written from that host. **So a form that comes back unanswered --
declined or dismissed, since which word a client uses for "nobody answered" is not the point -- falls
back to the model**: the refusal stops saying the human declined, and tells the session to ask in the
conversation and pass the answer as `mint`'s `author`, `rename`'s `author` or `identify`'s `record`.
When that call's own form also goes unanswered, the argument is used, and `mint` and `rename` relay
the name back -- the guarantee every handshake connection has, decided by the human partner for the
fallback on 2026-10-11 and extended to this case on 2026-10-16. **A form the human did answer always
wins** over anything the model passed. The cost is that a human who genuinely declines can still be
named by the model, visibly, through the relay.

**Where it lives:** `Server::handle_modern` and `Exchange` in [mcp.rs](../../src/mcp.rs) for the
revision; `name_from_human`, `identity_from_human` and the two question builders for the forms;
`Store::current_identity_records` for the records offered. **What a client declares is logged to
stderr** on `initialize` and on `server/discover`, which is step 1 of the order of work, built as a log
line rather than a separate change. The tests are in [tests/asking.rs](../../tests/asking.rs).

## Still open

- **Whether `create` asks as well**, so that a set is named for its creator at birth rather than at its
  first write. The design settled on first write for charter and identity together; asking at `create`
  would split them.
- **Whether any other answer belongs to the human in the same way.** The set's charter is the nearest
  candidate, and it is deliberately not proposed: a charter is drafted with the model's help and reviewed,
  where a name is simply known.
