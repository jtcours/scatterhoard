// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The MCP tool surface.
//!
//! # Transport is deliberately elsewhere
//!
//! [`Server::handle`] takes a JSON-RPC request as text and returns the response as text. Nothing
//! here knows about stdio, sockets, or an async runtime — that is glue, and it lives in the binary.
//! Two things follow: the tools are testable without an MCP client, and moving to a different
//! transport or SDK later is a change to the glue rather than a rewrite of the decisions.
//!
//! # The surface is small on purpose
//!
//! `documents/design/tool-surface.md` warns that a user may have several MCP servers loaded at once
//! and that too many tools confuse the model. So: the Vision's *store* and *recall*, plus the
//! convention loading that `documents/design/trigger-and-loading.md` requires at session start.
//! Nothing else until it earns a place.
//!
//! # Validation happens by parsing
//!
//! Tool arguments deserialise straight into the memory model's types, whose constructors validate.
//! An out-of-range confidence or a malformed set name is refused because the *type* refuses it, not
//! because this module checks. Most importantly, [`Scope`] has no `Default`, so a call that omits
//! the scope cannot be deserialised at all — the design's "explicit, never inferred" rule reaches
//! the tool boundary without a check anyone could forget.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::conventions;
use crate::embedder::{Embedder, ModelIdentity};
use crate::language_fit::{self, Band};
use crate::memory::EVALUATION_PREFIX;
use crate::memory::{
    Attribution, Author, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId,
    MemoryText, Origin, Scope, SetId, SetName, serde_name,
};
use crate::memory_file;
use crate::preferences::{self, ListedPreferences, Preferences, WordList};
use crate::prohibited;
use crate::reconcile;
use crate::review::{
    Audience, Decision, Hit, ProjectPreferences, SetKind, numbered, pre_pass, route,
};
use crate::store::{
    CHARTER_KEYWORD, Calibration, DisputedBy, FloorSource, Found, IncomingEdge, KeywordHit,
    LABELS_PER_CALIBRATION, LoadMode, ModelState, Question, Recalled, ReferencedFrom, Resolution,
    ReviewSetting, SessionId, Store, StoreLayout, claims_against_hit, recall_across,
    referenced_from_other_sets, resolve_within, search_keywords_across_holding,
};
use crate::summary::SetSummary;

/// How many keywords each cluster of a set summary may be named by.
///
/// Six is what the labelling was read at on 2026-09-12 when the scheme was chosen; fewer stops
/// naming the cluster and more stops being a summary.
const SUMMARY_LABELS: usize = 6;

/// A set rendered into the file format, together with what rendering it derived.
///
/// The summary comes back rather than being recomputed by a caller that wants to report it: two
/// derivations of the same thing are two things that can disagree.
struct Serialised {
    /// The file's bytes, header included.
    body: Vec<u8>,
    /// How many *records* were written — memories plus tombstones.
    written: usize,
    /// What the set amounts to, absent where it holds no vectors.
    summary: Option<SetSummary>,
}

/// What an import that could not describe its set leaves behind, so the description can be given
/// once it can be derived.
///
/// Held in the store rather than in the process, because the moment it becomes payable -- a model
/// installed -- is a restart away from the import that owed it.
///
/// **It carries nothing but its own presence** since 2026-09-21, when the comparison with the file's
/// description was retired: the file's claim, and the two facts that said when a comparison would
/// mean nothing, were all it had held. A record an older build wrote still has them, and they are
/// not read.
#[derive(Serialize, Deserialize)]
struct OwedDescription {}

/// A summary as a response shows it.
fn describe(summary: &SetSummary) -> Value {
    json!({
        "derivation": summary.derivation,
        "clusters": summary.clusters.iter().map(|cluster| json!({
            "memories": cluster.size,
            "about": cluster.keywords.iter().map(Keyword::as_str).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// The parameters a mis-closed call can swallow, and the framing that names each one.
///
/// **The detector is a conjunction, and both halves come from here.** A mis-closed parameter absorbs
/// everything up to the next well-formed close, so the framing that should have separated two
/// parameters lands *inside* the stored text while the parameter it named arrives empty. Either
/// half alone is weak — a memory legitimately about this defect quotes the framing, and an omitted
/// optional field is ordinary — and together they are near-conclusive.
///
/// Both spellings are listed because the observed damage uses both: a close named for the *field*
/// rather than the element (`</context>`), and an opening tag that arrived as text
/// (`<parameter name="context">`).
const SWALLOWED: &[(&str, [&str; 2])] = &[
    ("why", ["</why>", "<parameter name=\"why\">"]),
    ("context", ["</context>", "<parameter name=\"context\">"]),
    ("keywords", ["</keywords>", "<parameter name=\"keywords\">"]),
    ("links", ["</links>", "<parameter name=\"links\">"]),
    (
        "confidence",
        ["</confidence>", "<parameter name=\"confidence\">"],
    ),
    ("origin", ["</origin>", "<parameter name=\"origin\">"]),
];

/// The entities a caller may write to mean the characters themselves.
///
/// **Deliberately three.** The set exists so a memory whose subject *is* tool-call markup can be
/// written at all, and that needs the two angle brackets plus the ampersand — the last because the
/// escape has to be able to describe itself, so a memory about escaping writes `&amp;lt;` and means
/// `&lt;`. Anything further would be a general HTML entity table, which is a much larger promise
/// than the problem asks for.
const ENTITIES: &[(&str, char)] = &[("&lt;", '<'), ("&gt;", '>'), ("&amp;", '&')];

/// Turn the escapes a caller wrote into the characters they name.
///
/// **This works upstream of the damage, which is why it is the precondition rather than a
/// refinement.** A caller writing `&lt;/why&gt;` emits no markup at all, so there is nothing for a
/// mis-closed parameter to swallow and nothing for the detector below to trip on. The alternative —
/// forbidding markup outright — is the shape `documents/design/prohibited-data.md` already rejects:
/// a rule with no sanctioned alternative gets broken by whoever has a real need.
///
/// One left-to-right pass, and **the output is never re-scanned**. That is what makes `&amp;lt;`
/// decode to `&lt;` rather than all the way to `<`, which is the whole point of including the
/// ampersand.
///
/// The transformation belongs here, at the boundary, rather than in the store: the stored form is
/// the decoded one, checksums are taken over that, and it is not reversible — a reader cannot tell
/// whether the author typed `<` or `&lt;`. That is standard for an escape and harmless, but it is a
/// decision about what arrives, not about what is kept.
fn decode_entities(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;

    'outer: while !rest.is_empty() {
        if rest.starts_with('&') {
            for (entity, character) in ENTITIES {
                if let Some(tail) = rest.strip_prefix(entity) {
                    decoded.push(*character);
                    rest = tail;
                    continue 'outer;
                }
            }
        }
        let mut characters = rest.chars();
        // An ampersand that begins nothing recognised is an ordinary character, not an error: this
        // decodes a small escape set, it does not police the text around it.
        let next = characters.next().expect("the string is not empty");
        decoded.push(next);
        rest = characters.as_str();
    }
    decoded
}

/// One prose field, with its escapes turned into the characters they name.
///
/// Re-validated afterwards because decoding changes the text: `MemoryText` guarantees a non-empty,
/// trimmed value, and that guarantee has to hold for what is *stored* rather than for what arrived.
fn decoded(text: MemoryText) -> Result<MemoryText, String> {
    MemoryText::new(decode_entities(text.as_data()))
        .map_err(|error| format!("the text is not valid once its escapes are decoded: {error}"))
}

/// Which parameters a call swallowed, judged by framing present and the parameter absent.
///
/// `absent` reports whether each named parameter arrived empty; `texts` is every prose field that
/// could have absorbed it. A field is named only when **both** hold, which is what lets a memory
/// quoting `</why>` be stored so long as its `why` actually arrived.
fn swallowed_parameters(texts: &[&str], absent: &dyn Fn(&str) -> bool) -> Vec<&'static str> {
    SWALLOWED
        .iter()
        .filter(|(field, framing)| {
            absent(field)
                && framing
                    .iter()
                    .any(|mark| texts.iter().any(|text| text.contains(mark)))
        })
        .map(|(field, _)| *field)
        .collect()
}

/// The MCP protocol versions this server speaks, newest first.
///
/// **2025-06-18 is the one that matters here**: it is the revision that gave a tool an
/// `outputSchema` and a result `structuredContent`, which is how the response shape is published
/// (`response-shape-is-unpublished`). The older one is kept so a client asking for it is answered
/// in kind; the extra fields are ones it is free to ignore.
const PROTOCOL_VERSIONS: [&str; 2] = ["2025-06-18", "2024-11-05"];

/// The version to answer a client's `initialize` with: the one it asked for if this server speaks
/// it, and otherwise the newest this server does -- the specification's rule, which leaves the
/// client to disconnect if it cannot speak that.
fn negotiated_version(asked: Option<&str>) -> &'static str {
    asked
        .and_then(|asked| PROTOCOL_VERSIONS.iter().find(|known| **known == asked))
        .copied()
        .unwrap_or(PROTOCOL_VERSIONS[0])
}

/// The one per-request revision this server speaks, served beside the handshake revisions above.
///
/// **Chosen by how each request arrives, not by a handshake**: a request carrying this version in
/// its `_meta` is served under it, statelessly, and an `initialize` selects one of
/// [`PROTOCOL_VERSIONS`] as before -- the specification's *dual-era* server. It is here because it
/// carries elicitation in a shape this server can serve without ever sending a request of its own:
/// the human's answer arrives on a retried call. `documents/design/asking-the-human.md`.
const MODERN_PROTOCOL: &str = "2026-07-28";

/// How long a client may treat the tool list, or the answer to `server/discover`, as fresh.
///
/// Both change only when the binary does, and a rebuilt binary is a restarted server, so an hour
/// risks nothing a restart does not already refresh.
const LIST_TTL_MS: u64 = 3_600_000;

/// The `_meta` key a modern request names its revision under.
const META_PROTOCOL: &str = "io.modelcontextprotocol/protocolVersion";

/// The `_meta` key a modern request declares the client's capabilities under.
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

/// The `_meta` key a modern request may name the client under.
const META_CLIENT: &str = "io.modelcontextprotocol/clientInfo";

/// The question asking the human for the name they publish under in a set.
const NAME_QUESTION: &str = "publishing_name";

/// The question asking the human which of a set's identity records is theirs.
const IDENTITY_QUESTION: &str = "which_identity";

/// The choice in [`IDENTITY_QUESTION`] that means *none of these, I am new here*.
const NEW_IDENTITY: &str = "new";

/// What the request being handled brings with it, beyond its method and arguments.
///
/// **Per request, and replaced at the start of every one**, because the modern revision forbids a
/// server relying on anything an earlier request established: its version and the client's
/// capabilities arrive on each request. A field rather than an argument threaded through every
/// handler, because the question is asked deep in a write -- where a set settles who writes to it --
/// and only there.
struct Exchange {
    /// The revision this request is served under: [`MODERN_PROTOCOL`], or whichever handshake
    /// revision `initialize` agreed.
    protocol: &'static str,
    /// Whether the human can be asked through a form on this request: a modern request whose client
    /// declared form elicitation. **Never on a handshake revision**, where a question would travel as
    /// a request from the server, which this server does not send.
    asks_human: bool,
    /// The human's answers, as the client attached them to a retried call -- `inputResponses`, and
    /// never anything in the model's `arguments`.
    answers: serde_json::Map<String, Value>,
    /// A question a handler needs answered before it can proceed, which turns this call's result
    /// into an `InputRequiredResult`. Set only where `asks_human`, and only before anything is
    /// written, so a call that asks has changed nothing.
    question: Option<(&'static str, Value)>,
    /// What the human answered, where this call settled who writes to a set from their form: added
    /// to the result as `author_note`. **The model never saw the question** -- a client that shows
    /// the form retries the call itself -- so without this it finds the set attributed to a name it
    /// cannot account for, which is what a session reported on 2026-10-16.
    settled: Option<String>,
}

impl Exchange {
    /// A request on a handshake revision, which never asks.
    fn handshake(protocol: &'static str) -> Self {
        Self {
            protocol,
            asks_human: false,
            answers: serde_json::Map::new(),
            question: None,
            settled: None,
        }
    }
}

/// What the human did with a question, as their client reported it on the retried call.
enum Answer {
    /// They submitted the form, with these fields.
    Given(serde_json::Map<String, Value>),
    /// They refused to answer.
    Declined,
    /// They closed the question without choosing.
    Cancelled,
    /// The question has not been put to them on this call.
    Unasked,
}

/// Whether a client's declared capabilities let a server show the human a form.
///
/// An empty `elicitation` object means form mode, for backwards compatibility; one naming only
/// `url` does not, since a URL is not somewhere a server can put a question of its own.
fn declares_form_elicitation(capabilities: &Value) -> bool {
    match capabilities.get("elicitation").and_then(Value::as_object) {
        Some(modes) => modes.is_empty() || modes.contains_key("form"),
        None => false,
    }
}

/// Why the human is being asked for a name, which is all that changes how the question reads.
#[derive(Clone, Copy)]
enum NamePurpose {
    /// The set is settling who writes to it -- a first write, or `mint`.
    First,
    /// The person is changing the name they publish under -- `rename`.
    Rename,
}

/// Which identity record the human said is theirs.
enum Chosen {
    /// One already in the set.
    Record(MemoryId),
    /// None of them: they are new here, and publish under this name.
    New(Author),
}

/// The hint a form gives the human about where their name may be found -- given to them rather than
/// to the model, which minted a stranger's handle when handed it (`asking-the-human.md`).
const NAME_HINT: &str = "For a set published from a repository, this is often the account or \
     organisation the repository is hosted under -- if that is you.";

/// Why nothing was written when the name question came back `how` -- declined or dismissed -- and how
/// to proceed: ask in the conversation and pass the answer as the action's argument.
///
/// **Not blamed on the human**, who may never have seen it: a client that cannot show forms answers
/// them unseen, and the answer reads the same either way. If their client can show the form after all,
/// the action puts it to them again, and their answer there is the one used.
fn unanswered_name(set: &str, purpose: NamePurpose, how: &str) -> String {
    let action = match purpose {
        NamePurpose::First => "mint",
        NamePurpose::Rename => "rename",
    };
    format!(
        "nothing written: the question asking for the name written under in {set} was {how} -- by \
         the human, or by a client that cannot show forms and answers them unseen. Ask the human in \
         the conversation, wait for their answer, and pass it as `author` to `hoard_sets` action \
         \"{action}\" with {set}. Where their client can show the form, it is put to them again, \
         and their answer there is the one used."
    )
}

/// [`unanswered_name`] for the question asking which identity record is the human's.
fn unanswered_identity(set: &str, how: &str) -> String {
    format!(
        "nothing written: the question asking which identity is the human's in {set} was {how} -- by \
         the human, or by a client that cannot show forms and answers them unseen. Ask the human in \
         the conversation which of the set's names is theirs -- recall the keyword `hoard-author` in \
         this set to see them -- and pass its id as `record` to `hoard_sets` action \"identify\" \
         with {set}; or, if they are new here, their name as `author` to action \"mint\". Where \
         their client can show the form, it is put to them again, and their answer there is the one \
         used."
    )
}

/// The form asking the human for the name they publish under in `set`, with `problem` leading the
/// message where an earlier answer could not be used.
fn name_question(set: &str, purpose: NamePurpose, problem: Option<&str>) -> Value {
    let ask = match purpose {
        NamePurpose::First => format!(
            "Scatterhoard needs the name you are known by where {set} is published -- an account \
             name on one host, a full name somewhere else. It is recorded as the author of what is \
             written to this set from this machine, and can be changed later."
        ),
        NamePurpose::Rename => format!(
            "Scatterhoard needs the new name you want to be known by in {set}. It replaces your \
             current name there, on everything you have written to it."
        ),
    };
    json!({
        "mode": "form",
        "message": format!("{}{ask}", problem.unwrap_or_default()),
        "requestedSchema": {
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "title": format!("Your name in {set}"),
                    "description": NAME_HINT,
                    "minLength": 1,
                },
            },
            "required": ["name"],
        },
    })
}

/// The form asking the human which of `records` is theirs in `set`, offering to add them as new
/// where `offer_new`.
fn identity_question(
    set: &str,
    records: &[(MemoryId, Author)],
    offer_new: bool,
    problem: Option<&str>,
) -> Value {
    let mut choices: Vec<Value> = records
        .iter()
        .map(|(id, name)| json!({"const": id.to_string(), "title": name.as_str()}))
        .collect();
    let mut properties = serde_json::Map::new();
    let ask = if offer_new {
        choices.push(json!({
            "const": NEW_IDENTITY,
            "title": "None of these -- I am new to this set",
        }));
        properties.insert(
            "name".to_owned(),
            json!({
                "type": "string",
                "title": "Your name, if you are new here",
                "description": NAME_HINT,
            }),
        );
        format!(
            "{set} already holds the names of people who write to it. Say which is you, so what you \
             write is attributed to you rather than to a second copy of you -- or that you are new \
             here, with the name you are known by where the set is published."
        )
    } else {
        format!(
            "{set} already holds the names of people who write to it. Say which is you, so what you \
             write is attributed to you rather than to a second copy of you."
        )
    };
    properties.insert(
        "record".to_owned(),
        json!({
            "type": "string",
            "title": format!("Which of these is you in {set}?"),
            "oneOf": choices,
        }),
    );
    json!({
        "mode": "form",
        "message": format!("{}{ask}", problem.unwrap_or_default()),
        "requestedSchema": {
            "type": "object",
            "properties": properties,
            "required": ["record"],
        },
    })
}

/// How many memories recall returns when the caller does not say.
///
/// **A count, and the whole of what decides how many come back** apart from the floor. Until
/// 2026-10-02 this was a window that a largest-gap cut looked inside, which in practice returned
/// three or four; that cut was choosing among noise -- real queries broke barely more sharply than
/// nonsense ones -- so the count moved with the window (`result-count-follows-the-limit-not-the-query`).
/// Eight because the one measurement of result-count quality put a fixed count's best F1 at eight
/// to ten, across 24 answer-keyed queries; it was chosen with the human partner as the predictable
/// end of that range. `already_known` is what keeps it affordable: asking for more never re-sends
/// what the caller holds.
const DEFAULT_LIMIT: usize = 8;

/// A memory as a client sees it: the memory itself, the set it came from, and how it was found.
///
/// The single place a memory is turned into JSON. [`Memory`] already derives `Serialize`, so
/// flattening it here means a field added to the memory reaches every tool without a second edit.
/// That inverts the previous failure mode: three hand-written projections each listed the fields
/// they happened to include, so a new field reached no client at all until every site was
/// separately remembered, and `keywords`, `links`, `origin`, `created` and `modified` reached none
/// of them. See the single-source-of-truth standard in `STANDARDS.md`.
///
/// A scope as the tool surface publishes it: `local`, or `shared:<name>`.
///
/// `a-scope-in-a-response-is-not-the-spelling-the-schema-publishes`.
/// `oneof-tool-schemas-present-no-parameters` settled that the surface publishes the scalar
/// spelling — the one [`Scope::as_key`] writes into a store's `meta` row — while the object form
/// keeps *parsing*, so nothing that already worked stopped working. That decision was applied to
/// arguments and never to responses, where `Scope`'s derived `Serialize` runs, so a caller reading
/// the published schema learned one spelling and met another in every result.
///
/// **A type rather than a call at each site**, because the sites are the problem: nine of them
/// reached the wire, and the tenth is the one somebody adds later. Serialising a bare [`Scope`]
/// into a response is now the thing that looks wrong, and the derived form stays reachable for the
/// one place that needs it.
///
/// **The file format keeps the object form and must.** Every memory file on disk carries
/// `{"shared": …}` in its header, `documents/design/file-format.md` documents it, and changing it
/// would be a format change with a version bump rather than a surface fix — pinned by
/// `the_exported_file_header_keeps_the_object_form`.
#[derive(Debug, Clone, Copy)]
struct WireScope<'a>(&'a Scope);

/// What [`Server::language_fit`] found for one set.
#[derive(Default)]
struct Fit {
    /// Each declared language's score, keyed by the tag as declared.
    scores: serde_json::Map<String, Value>,
    /// The band message for each language below the top band: what changes how recall reads.
    bands: Vec<String>,
    /// The models the set recommends for those languages: status's to say, beside the bands.
    advice: Vec<String>,
    /// What the set's own sentences need doing, which is status's to say and not recall's.
    setup: Vec<String>,
}

/// Ask for ten sentence sets for a language there are none for, saying how to write them.
///
/// **Asked of the agent** because a set declared in a language is almost certainly being written
/// by an agent that can write it, and no list the server carries could cover every language a
/// charter might declare.
fn sentences_wanted(tag: &str) -> String {
    format!(
        "This set declares `{tag}`, and there are no test sentences for it, so how well this \
         set's embedding model represents `{tag}` is unknown. To measure it, write ten paraphrase \
         tests in `{tag}`, each its own memory carrying `hoard-fit-sentences` and \
         `hoard-lang-{tag}`, holding one JSON array of three sentences: an anchor, a paraphrase \
         of it sharing as few words with it as the language allows, and a sentence on a topic \
         none of the ten anchors touch -- every anchor is scored against all the other \
         sentences. Prose may come before the array. Then link all ten from this set's \
         `hoard-fit-hub` memory with `refers_to` links, writing the hub if the set has none or \
         superseding it to add them."
    )
}

/// What a set can do about a language its model serves poorly, said once in status beside the
/// bands.
///
/// **The one place a session learns either is possible.** Nothing else served mentions recording a
/// recommendation or measuring a set's own vocabulary, so without this a user who says *record that
/// this model works for us* gets an ordinary memory status never lists, and a set full of borrowed
/// terms of art is measured on everyday sentences that miss them. Said where a score is low because
/// that is when the agent is already telling the user about models. Status only: on recall it
/// would repeat on every query without changing how the results read.
fn fit_how_to(hubs: &[MemoryId]) -> String {
    // Named where there is one, because linking to it is the instruction, and an agent told to
    // link to a memory it cannot name has to go looking for it -- or writes a second hub.
    let hub = match hubs {
        [] => "the id of this set's `hoard-fit-hub` memory -- the set has none yet, so store \
               one first, for example {\"content\": \"This set's model-fit hub.\", \
               \"keywords\": [\"hoard-fit-hub\"]}, and link to the id that call returns --"
            .to_owned(),
        [one, ..] => format!("this set's `hoard-fit-hub` memory, `{one}`,"),
    };
    format!(
        "If the user finds a model that serves this set better, record it so every copy of the \
         set passes it on: a memory carrying `hoard-fit-model` whose opening sentence names the \
         model, with a `refers_to` link to {hub} and a `hoard-lang-` keyword for each language it \
         was measured on, or none if it concerns the whole set. If the set's vocabulary is \
         unusual, such as terms of art borrowed from another language, the built-in test \
         sentences measure everyday text rather than this set: ten sentence sets of its own for a \
         language replace them, each a memory carrying `hoard-fit-sentences` and that language's \
         `hoard-lang-` keyword, holding one JSON array of an anchor, a paraphrase of it and an \
         unrelated sentence, and all ten linked from the hub."
    )
}

impl Serialize for WireScope<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.as_key())
    }
}

/// `scope` belongs here rather than on `Memory` for the reason `Memory` documents: an exported file
/// *is* a memory set, so a per-memory scope could disagree with the header declaring it. A result
/// spanning several sets still needs it per memory, which makes it a property of the view.
#[derive(Serialize)]
struct MemoryView<'a> {
    #[serde(flatten)]
    memory: &'a Memory,
    scope: WireScope<'a>,
    /// This memory came from a copy loaded for review, which nobody has adopted.
    ///
    /// **Taken from the store's load mode, never parsed from the `eval-` name.** The name is for
    /// people; a return path that read it would be one more place to forget, and an omission here
    /// fails in the reassuring direction -- a proposal reading as the project's record.
    /// `documents/design/load-modes.md`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    evaluation: bool,
    /// The edges pointing *at* this memory, shipped by default.
    ///
    /// `documents/design/recall-mechanisms.md` settles the default rather than leaving it to the
    /// caller: an edge sent only when asked for is invisible to a caller that does not know to ask,
    /// which is the defect this field exists to close. A future reduced-detail projection may offer
    /// a way to turn these *off*; it may not make off the default.
    #[serde(skip_serializing_if = "no_edges")]
    incoming: &'a [IncomingEdge],
    /// Standing-changing edges in *other* sets that name this memory, which changed nothing about
    /// it.
    ///
    /// **A separate field from `incoming` on purpose.** `incoming` is what this memory's own set
    /// says about it, and folding a stranger's claim in there would make the two indistinguishable
    /// -- which is exactly the distinction a session needs in order to judge one. Carries the
    /// claimant's set, id and label and nothing else: the claiming memory is not fetched, so
    /// attacking a memory cannot buy a slot in every recall that matches it. Fetch it with
    /// `hoard_lookup` if the dispute bears on the work.
    #[serde(skip_serializing_if = "no_claims")]
    disputed_by: &'a [DisputedBy],
    /// Memories in *other loaded sets* that point at this one, whatever the label.
    ///
    /// **A third field rather than a swelling of `incoming`, for the reason `disputed_by` is one.**
    /// `incoming` is what this memory's own set says about it; a reference from elsewhere is a
    /// different fact, and merging them would make the two indistinguishable at exactly the moment
    /// a reader needs to tell them apart.
    ///
    /// **It travels nowhere.** Derived at read time from the stores this machine holds, so it
    /// appears in no export and says nothing to any other holder. That is what lets it name a
    /// personal set as the source, where `disputed_by` cannot: a claim naming `local` would reach
    /// readers for whom `local` is a different set, and this reaches nobody but the person who holds
    /// both.
    #[serde(
        skip_serializing_if = "no_references",
        serialize_with = "references_as_wire"
    )]
    referenced_from: &'a [ReferencedFrom],
    #[serde(flatten)]
    found: FoundBy,
}

/// `referenced_from` with each scope spelled as the schema publishes it.
///
/// `ReferencedFrom` derives `Serialize`, which writes a [`Scope`] as an object; lookup built the same
/// field through [`WireScope`] and recall did not, so one field had two spellings until the response
/// shapes were surveyed for `response-shape-is-unpublished`.
fn references_as_wire<S: serde::Serializer>(
    references: &&[ReferencedFrom],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(references.len()))?;
    for reference in *references {
        seq.serialize_element(&json!({
            "scope": WireScope(&reference.scope),
            "source": reference.source,
            "label": reference.label,
        }))?;
    }
    seq.end()
}

/// Omit `referenced_from` when nothing elsewhere points here, as `incoming` and `disputed_by` are
/// omitted when quiet.
fn no_references(references: &&[ReferencedFrom]) -> bool {
    references.is_empty()
}

/// Omit `disputed_by` when nobody is disputing, which is nearly always. Same reasoning as
/// [`no_edges`], and it matters more here: a field present on every hit is a field readers learn to
/// skip, and this one is worth stopping for.
fn no_claims(claims: &&[DisputedBy]) -> bool {
    claims.is_empty()
}

/// Omit `incoming` when nothing points at the memory, exactly as `links` is omitted when it authors
/// none. An absent list and an empty one say the same thing, and the empty one costs a line.
fn no_edges(edges: &&[IncomingEdge]) -> bool {
    edges.is_empty()
}

/// Omit a flag that is not raised. Absent and false say the same thing, and most memories are
/// neither in a cycle nor superseded from beyond this machine.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(flag: &bool) -> bool {
    !*flag
}

/// Omit the inherited score when the walk did not reach this memory. Zero and absent say the same
/// thing here, and most memories in most results were found by their own wording alone.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn not_reached(inherited: &f32) -> bool {
    *inherited == 0.0
}

/// How a memory was found, with the one figure that describes that finding.
///
/// An enum rather than a struct of optional fields, so a similarity and a match count cannot appear
/// in the same object. They are measured against different things, and keeping them apart in the
/// type is the structural form of the rule that keeps them in separate lists.
#[derive(Serialize)]
#[serde(untagged)]
enum FoundBy {
    /// Matched by meaning, and this memory is the one that matched.
    Semantic {
        /// This memory's score, against the corpus centroid.
        similarity: f32,
        /// True when this memory sits in a supersession cycle — see `hoard_lookup`.
        #[serde(skip_serializing_if = "is_false")]
        supersession_cycle: bool,
        /// True when this memory's own `SUPERSEDED_BY` names a replacement in a set not loaded here.
        ///
        /// The memory is returned rather than withheld, so a session learns it exists; this says not
        /// to read it as current. **The remedy is in the memory itself** -- its `SUPERSEDED_BY` link
        /// carries the set name and the id, so loading that set and fetching the target is the move
        /// this flag exists to make possible.
        #[serde(skip_serializing_if = "is_false")]
        superseded_beyond_this_set: bool,
        /// True when this memory was superseded by a replacement **forgotten** in its own set.
        ///
        /// Not elsewhere but withdrawn, so there is no set to load. The memory is still superseded:
        /// forgetting a replacement does not reinstate what it replaced. If its claim should stand
        /// again, write it anew with a `supersedes` link to the forgotten id, which `incoming` names.
        #[serde(skip_serializing_if = "is_false")]
        replacement_forgotten: bool,
        /// What it inherited from a memory the query matched directly, decayed by distance.
        ///
        /// Absent when the walk did not reach it, which is the ordinary case. Present, it is what
        /// explains an order the `similarity` figures alone do not: this memory is here because of
        /// what it is *linked to*, which is the case traversal exists for — the measurement
        /// supporting a decision, the constraint that decision rests on. Ranking on a number the
        /// caller cannot see is how `similarity-describes-a-different-memory` misled people.
        #[serde(skip_serializing_if = "not_reached")]
        inherited: f32,
    },
    /// Matched by meaning, but what matched was a superseded memory and this is its replacement.
    ///
    /// A separate variant so the score can be *named* for what it describes. Reporting it as
    /// `similarity` alongside a different memory's content invites the caller to attribute it to
    /// the content it can see, and nothing in the payload contradicts that reading.
    SemanticSubstituted {
        /// The **superseded** memory's score. This memory was not scored; it was substituted in.
        matched_similarity: f32,
        /// What the superseded memory inherited from the walk, on the same terms as
        /// [`FoundBy::Semantic`]'s field of the same name.
        #[serde(skip_serializing_if = "not_reached")]
        inherited: f32,
        replaces_matched: MemoryId,
    },
    /// Matched by exact keyword, and this memory is the one that matched.
    Keyword {
        matched_keywords: usize,
        /// True when this memory sits in a supersession cycle — see `hoard_lookup`.
        #[serde(skip_serializing_if = "is_false")]
        supersession_cycle: bool,
        /// True when this memory's own `SUPERSEDED_BY` names a replacement in a set not loaded here.
        ///
        /// The memory is returned rather than withheld, so a session learns it exists; this says not
        /// to read it as current. **The remedy is in the memory itself** -- its `SUPERSEDED_BY` link
        /// carries the set name and the id, so loading that set and fetching the target is the move
        /// this flag exists to make possible.
        #[serde(skip_serializing_if = "is_false")]
        superseded_beyond_this_set: bool,
        /// True when this memory was superseded by a replacement **forgotten** in its own set.
        ///
        /// Not elsewhere but withdrawn, so there is no set to load. The memory is still superseded:
        /// forgetting a replacement does not reinstate what it replaced. If its claim should stand
        /// again, write it anew with a `supersedes` link to the forgotten id, which `incoming` names.
        #[serde(skip_serializing_if = "is_false")]
        replacement_forgotten: bool,
    },
    /// Matched by keyword, but what matched was superseded and this is its replacement.
    ///
    /// The substitution matters more here than on the semantic path: a replacement need not carry
    /// the keyword that was searched for, so without `replaces_matched` the caller is handed a
    /// memory that plainly does not match its own query and told nothing about why.
    KeywordSubstituted {
        /// How many of the requested keywords the **superseded** memory carried.
        matched_keywords: usize,
        replaces_matched: MemoryId,
    },
    /// Fetched by id. Retrieval rather than search, so there is no score — and a superseded memory
    /// is returned as itself rather than substituted, which is what makes this the way to read
    /// history.
    ///
    /// Three states, because [`Resolution`] has three and flattening them is what made a cycle
    /// indistinguishable from a current memory. `superseded_by` names the **end** of the chain;
    /// stepping one link at a time is done from `incoming`, which carries the immediate successor's
    /// `supersedes` edge.
    Lookup {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        superseded_by: Vec<MemoryId>,
        /// True when the chain leaves this machine — the memory is known to be replaced and the
        /// replacement is not here. Reported rather than passed off as an ending.
        #[serde(skip_serializing_if = "is_false")]
        superseded_beyond_this_set: bool,
        /// True when this memory was superseded by a replacement **forgotten** in its own set.
        ///
        /// Not elsewhere but withdrawn, so there is no set to load. The memory is still superseded:
        /// forgetting a replacement does not reinstate what it replaced. If its claim should stand
        /// again, write it anew with a `supersedes` link to the forgotten id, which `incoming` names.
        #[serde(skip_serializing_if = "is_false")]
        replacement_forgotten: bool,
        /// True when the memory sits in a supersession cycle, which is a data error rather than a
        /// history.
        #[serde(skip_serializing_if = "is_false")]
        supersession_cycle: bool,
    },
}

/// How much of a memory a caller wants back.
///
/// **One projection selected here, rather than a second view type.** `STANDARDS.md` asks for
/// subtraction from one complete representation, because several call sites each listing the fields
/// they want is the defect that produced three disagreeing serialisers in this very layer -- and
/// `documents/design/recall-mechanisms.md` adds the constraint this has to respect: a reduced level
/// may be *offered*, and may not become the default. A caller that has not chosen cannot know what
/// it gave up.
///
/// **The choice belongs to the caller** by the test in `documents/design/decision-placement.md`:
/// only the caller knows whether it is surveying or reading, and the answer changes the response
/// by a measured factor of nearly five.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Detail {
    /// The whole record, links, keywords and all.
    #[default]
    Full,
    /// The id and one derived line, for building an index over many memories at once.
    Summary,
}

impl Detail {
    /// The vocabulary the schema publishes, so the two cannot fall out of step.
    const ALL: [Self; 2] = [Self::Full, Self::Summary];
}

impl<'a> MemoryView<'a> {
    fn semantic(hit: &'a Recalled, evaluation: bool) -> Self {
        Self {
            memory: &hit.memory,
            scope: WireScope(&hit.scope),
            evaluation,
            incoming: &hit.incoming,
            referenced_from: &hit.referenced_from,
            disputed_by: &hit.disputed_by,
            found: match hit.matched {
                Some(replaces_matched) => FoundBy::SemanticSubstituted {
                    matched_similarity: hit.similarity,
                    inherited: hit.inherited,
                    replaces_matched,
                },
                None => FoundBy::Semantic {
                    similarity: hit.similarity,
                    inherited: hit.inherited,
                    supersession_cycle: hit.in_cycle,
                    superseded_beyond_this_set: hit.superseded_beyond_this_set,
                    replacement_forgotten: hit.replacement_forgotten,
                },
            },
        }
    }

    fn keyword(hit: &'a KeywordHit, evaluation: bool) -> Self {
        Self {
            memory: &hit.memory,
            scope: WireScope(&hit.scope),
            evaluation,
            incoming: &hit.incoming,
            referenced_from: &hit.referenced_from,
            disputed_by: &hit.disputed_by,
            found: match hit.matched {
                Some(replaces_matched) => FoundBy::KeywordSubstituted {
                    matched_keywords: hit.matched_keywords,
                    replaces_matched,
                },
                None => FoundBy::Keyword {
                    matched_keywords: hit.matched_keywords,
                    supersession_cycle: hit.in_cycle,
                    superseded_beyond_this_set: hit.superseded_beyond_this_set,
                    replacement_forgotten: hit.replacement_forgotten,
                },
            },
        }
    }

    fn looked_up(hit: &'a Found, evaluation: bool) -> Self {
        Self {
            memory: &hit.memory,
            scope: WireScope(&hit.scope),
            evaluation,
            incoming: &hit.incoming,
            referenced_from: &hit.referenced_from,
            disputed_by: &hit.disputed_by,
            found: match &hit.validity {
                Resolution::Current => FoundBy::Lookup {
                    superseded_by: Vec::new(),
                    superseded_beyond_this_set: false,
                    replacement_forgotten: false,
                    supersession_cycle: false,
                },
                Resolution::Superseded {
                    tips,
                    unresolved,
                    forgotten,
                } => FoundBy::Lookup {
                    superseded_by: tips.clone(),
                    superseded_beyond_this_set: *unresolved,
                    replacement_forgotten: *forgotten,
                    supersession_cycle: false,
                },
                Resolution::Cycle => FoundBy::Lookup {
                    superseded_by: Vec::new(),
                    superseded_beyond_this_set: false,
                    replacement_forgotten: false,
                    supersession_cycle: true,
                },
            },
        }
    }

    /// Serialising a view cannot fail for any `Memory` the type system admits; a failure here would
    /// mean a non-string map key or a non-finite float, neither of which these types can hold.
    fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("a memory view is always representable as JSON")
    }

    /// The same view at the detail level the caller asked for.
    ///
    /// **The summary carries the id and the line and nothing else**, and the omissions are what it
    /// is for: the measured envelope this cap was chosen against is 49 characters -- the id, the two
    /// field names, the punctuation -- so a scope, a score or a match count here would spend a
    /// quarter of the saving each. What is missing is one `hoard_lookup` away, which is the trade
    /// an index makes.
    fn to_json_at(&self, detail: Detail) -> Value {
        match detail {
            Detail::Full => self.to_json(),
            Detail::Summary => {
                let mut summary = json!({
                    "id": self.memory.id,
                    "summary": self.memory.content.summary_as_data(),
                });
                // The one field a summary carries beyond the two, and only when it is true: an index
                // that cannot tell a proposal from the record is an index of claims about the
                // project that the project has not made.
                if self.evaluation {
                    summary["evaluation"] = json!(true);
                }
                summary
            }
        }
    }
}

/// How a file looked at one moment, in the two fields a `stat` gives cheaply.
///
/// Size as well as time because time alone is a poor equality test on a file a build tool replaces:
/// filesystems differ in resolution, and a rebuild landing inside one tick would read as unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Build {
    modified: Option<std::time::SystemTime>,
    bytes: u64,
}

impl Build {
    fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            bytes: metadata.len(),
        })
    }

    /// When it was built, for a reader who wants to know how far behind they are.
    fn described(&self) -> String {
        self.modified.map_or_else(
            || "at an unknown time".to_owned(),
            |at| {
                format!(
                    "built {}",
                    jiff::Timestamp::try_from(at).map_or_else(
                        |_| "at an unreadable time".to_owned(),
                        |stamp| stamp.to_string(),
                    )
                )
            },
        )
    }
}

/// The MCP server: the stores it can reach, the model it searches them with, and who it attributes
/// new memories to.
pub struct Server {
    /// A `BTreeMap` rather than a `HashMap` so iteration order is a property of the type.
    ///
    /// Anything that reports every store — `hoard_sets` `status` today — would otherwise need to
    /// sort, and a missing sort is untestable: `HashMap` order is randomised per process, so a test
    /// for it fails a fraction of the time rather than always. `Scope` already derives `Ord`, so the
    /// guarantee costs one word here instead of a sort call and a flaky assertion.
    stores: BTreeMap<Scope, Store>,
    /// Stores held open but neither searched nor written, because a set identity is loaded once and
    /// another copy answers to theirs -- `documents/design/load-modes.md`.
    ///
    /// **Withheld rather than dropped, so the state is visible and recoverable.** `hoard_sets status`
    /// lists them and `drop` reaches them; removing a copy settles the identity again, so the survivor
    /// loads without a restart. Every copy is withheld only where nothing says which is the machine's
    /// own: several read-write copies, or none. A read-write copy beside read-only ones wins, which is
    /// what keeps a restart from locking someone out of their own set.
    withheld: BTreeMap<Scope, Store>,
    /// The operating system's name for the account this server runs as, if the startup wiring found
    /// one: `local`'s default author, and nothing else's, since `set-settled-at-first-write`. A shared
    /// set is asked who you are at its first write instead, because the name is how you are known
    /// where that set is published, which varies by set.
    account_name: Option<Author>,
    /// Held here rather than per store, because every store a session searches has to be embedded
    /// by the *same* model — [`recall_across`] embeds the query once and scores that vector against
    /// all of them. Owning it at this level is also what stops a store being reachable without one.
    embedder: Option<Arc<dyn Embedder>>,
    /// Which directories this server's stores live in.
    ///
    /// Optional because most of the surface never needs it — a server assembled from in-memory
    /// stores can serve every tool except `hoard_sets` `create`, and requiring a directory to
    /// construct one would make every test carry a path it does not use.
    layout: Option<StoreLayout>,
    /// The binary this server was launched from, as it looked at launch.
    ///
    /// **A client serves whatever binary it launched with**, so rebuilding the source changes
    /// nothing a running session sees — and the failure is silent in the way that matters most,
    /// because the conventions are the steering input: an old catalog answers every call
    /// successfully and simply governs. Three occurrences are recorded, and on the third the stale
    /// text contradicted that day's headline decision without anyone noticing.
    ///
    /// `None` when the path or its metadata could not be read, which is not an error: a server that
    /// cannot see its own binary reports nothing rather than guessing, and the tools work unchanged.
    launched_from: Option<(PathBuf, Build)>,
    /// This process's identity, which stands in for one session.
    ///
    /// It bounds the `used` label in each store's query log: a memory a session uses labels the
    /// queries *that session* asked and no others. Generated here so that a session which never
    /// loads the conventions still has a bound, and **replaced whenever they are loaded**, which is
    /// where a session actually begins -- a server outlives one, measured. See [`SessionId`].
    session: SessionId,
    /// The sharing preferences this session was served on its first page, as the field the catalog
    /// document ends with.
    ///
    /// **Kept for the session rather than read on every page**, so an edit made between pages
    /// cannot shift a cursor: the document is cut by character offset, and a file that changed
    /// length underneath it would leave the joined pages unparseable. Read again when the next
    /// session begins, which is when an edit is meant to take effect.
    sharing_preferences: Option<Value>,
    /// What each set's export review in this session has shown and flagged.
    ///
    /// **`clear` accepts only what a review here has shown**, which is what the server can check when
    /// it cannot hear the person: not that they said yes, but that the memory was put in front of the
    /// session. Kept per process rather than in the store, because it is a fact about this session.
    reviews: BTreeMap<Scope, ReviewSession>,
    /// Where `backup` writes, from `SCATTERHOARD_BACKUPS`; `None` for the common directory's
    /// `backups`. Set by whoever configures the machine and never by a call, so no call can send a
    /// backup somewhere of its choosing.
    backup_directory: Option<PathBuf>,
    /// The protocol version agreed in `initialize`, which decides whether output schemas and
    /// `structuredContent` are sent at all: they belong to the 2025-06-18 revision, so a client on
    /// the older one gets neither -- and a client can avoid their cost that way. One server serves
    /// one client over stdio, so one value is the whole of it. Starts at the newest, which is what a
    /// server driven without an `initialize` -- a test -- is talking.
    protocol: &'static str,
    /// What the request being handled brings with it: see [`Exchange`].
    exchange: Exchange,
    /// The shapes of `tools/call` already reported on stderr, so each is logged once per process.
    /// See [`call_shape`].
    call_shapes_logged: std::collections::BTreeSet<String>,
    /// How well this server's model represents each group of test sentences, worked out the first
    /// time a set needs it -- `language_fit::score`. Keyed by the built-in tag for the built-ins, so
    /// `en` and `en-gb` share one answer, and by the sentences themselves for a set's own, so a set
    /// fixed by superseding one sentence is scored afresh.
    ///
    /// **Per process, because the answer depends only on the model and the language**, and a model
    /// change restarts the server; [`Server::set_embedder`] clears it anyway, so nothing can hold a
    /// score for a model it no longer has. Which languages a set declares is read from its charter
    /// on every call, so a newly declared language is scored when it appears. Behind a lock because
    /// `status` reads through `&self`.
    language_scores: Mutex<BTreeMap<String, u8>>,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// A server with no stores, no model and no account name yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            protocol: PROTOCOL_VERSIONS[0],
            exchange: Exchange::handshake(PROTOCOL_VERSIONS[0]),
            call_shapes_logged: std::collections::BTreeSet::new(),
            stores: BTreeMap::new(),
            withheld: BTreeMap::new(),
            account_name: None,
            embedder: None,
            layout: None,
            launched_from: std::env::current_exe()
                .ok()
                .and_then(|path| Build::of(&path).map(|build| (path, build))),
            session: SessionId::generate(),
            sharing_preferences: None,
            reviews: BTreeMap::new(),
            backup_directory: None,
            language_scores: Mutex::new(BTreeMap::new()),
        }
    }

    /// Say what the operating system calls the account this server runs as.
    ///
    /// **`local`'s default author and nothing else**: `local` is never published, so the account name
    /// costs nothing there, where a shared set asks at its first write. Supplied by the startup wiring,
    /// which is what can read the environment, and by tests, which need a fixed name.
    pub fn set_account_name(&mut self, name: Author) {
        self.account_name = Some(name);
    }

    /// Watch a different file as this server's binary. Testing seam: a test cannot rebuild the test
    /// binary underneath itself, and the behaviour worth pinning is what happens when it changes.
    pub fn set_launched_from(&mut self, path: &Path) {
        self.launched_from = Build::of(path).map(|build| (path.to_path_buf(), build));
    }

    /// What the binary looks like now, when that differs from how it looked at launch.
    ///
    /// **Compared rather than hashed.** A hash would be exact; this reads two fields of one `stat`
    /// and is asked at every session's first call. The cost of being approximate runs one way: a
    /// rebuild that produced an identical binary still reports, so a session is occasionally told to
    /// restart when nothing changed. That is the harmless direction, and the opposite — a real
    /// change going unreported — is the failure this exists for.
    fn rebuilt_since_launch(&self) -> Option<Build> {
        let (path, launched) = self.launched_from.as_ref()?;
        let now = Build::of(path)?;
        (now != *launched).then_some(now)
    }

    /// Record that this session used a memory, wherever a store's log says it returned one.
    ///
    /// **Called wherever the caller names an id it already has** — a lookup, a link target, a
    /// memory passed back as already held. Those three are what
    /// `corpus/label_queries.py` counted when the forty-query threshold was measured, and adding a
    /// fourth would change what a labelled query means without re-running that measurement, so
    /// amending and forgetting are deliberately not uses.
    ///
    /// Offered to every loaded store rather than to the one holding the memory: only a store whose
    /// log records returning that id in this session updates anything, and resolving the store
    /// first would mean looking up an id the caller may have got from somewhere else entirely.
    fn note_used(&self, ids: &[MemoryId]) -> Result<(), String> {
        for id in ids {
            for store in self.stores.values() {
                store
                    .note_used(*id, self.session)
                    .map_err(|error| format!("could not record that a memory was used: {error}"))?;
            }
        }
        Ok(())
    }

    /// Say which directories this server's stores live in.
    ///
    /// `create` needs the home, and `status` needs to report what scanning them found. It is
    /// deliberately not part of [`Server::new`]: a set's *name* is shared identity and a *path* is
    /// per-machine state, and the design keeps those apart
    /// (`documents/design/set-lifecycle.md`, *identity travels; location does not*). This is the
    /// machine-local half, so it is supplied by the startup wiring that actually knows it.
    pub fn set_layout(&mut self, layout: StoreLayout) {
        self.layout = Some(layout);
    }

    /// Set where `hoard_sets` action `backup` writes, from the machine's configuration.
    pub fn set_backup_directory(&mut self, directory: PathBuf) {
        self.backup_directory = Some(directory);
    }

    /// Set the model this server searches with, and give it to every store it already holds.
    ///
    /// Order-independent on purpose: stores added later are embedded by [`Server::add_store`], and
    /// stores added earlier are caught up here. Startup wiring should not have to know which order
    /// is the safe one.
    pub fn set_embedder(&mut self, embedder: Arc<dyn Embedder>) {
        for store in self.stores.values_mut() {
            store.set_embedder(Arc::clone(&embedder));
        }
        self.embedder = Some(embedder);
        self.language_scores
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Make a store reachable, embedding it with this server's model. Its scope is taken from the
    /// store, never from a caller.
    ///
    /// Assigning the model *here* rather than at the call site is the fix for
    /// `project-stores-get-no-embedder`: startup wiring gave the model to the local store and not
    /// to the project stores, and nothing noticed, because a store with no model is a supported
    /// state that fails silently — memories stored into it get no vector and can never be found by
    /// meaning, and its presence makes the whole union report itself unsearchable. Adding a store
    /// is now what embeds it, so there is no second step to forget.
    ///
    /// **A set identity is loaded once.** A store sharing an identity with another goes through
    /// [`settle_identity`](Self::settle_identity), which loads one copy or none. A store whose
    /// identity cannot be read is loaded as before: the failure surfaces on the first call that needs
    /// the identity, which says more than a silent refusal here.
    pub fn add_store(&mut self, mut store: Store) {
        if let Some(embedder) = &self.embedder {
            store.set_embedder(Arc::clone(embedder));
        }
        let scope = store.scope().clone();
        // **A review copy loads straight away and settles nothing.** It carries its source set's
        // identity but holds it for nothing, so it neither withholds another copy nor waits on one --
        // and it must not be parked below, where only `settle_identity` could release it.
        if store.load_mode().is_ok_and(LoadMode::is_evaluation) {
            self.stores.insert(scope, store);
            return;
        }
        let Ok(Some(identity)) = store.set_id() else {
            self.stores.insert(scope, store);
            return;
        };
        // Parked first, and settled from the whole population rather than compared against what is
        // already loaded: the answer must not depend on the order stores arrive in, and a rule
        // applied to arrivals one at a time does.
        self.withheld.insert(scope, store);
        self.settle_identity(identity);
    }

    /// Decide which copy of `identity`, if any, is loaded -- `documents/design/load-modes.md`.
    ///
    /// One copy loads: a lone copy whatever its mode, or the one read-write copy among several. With
    /// no read-write copy, or more than one, nothing says which is the machine's own, so every copy is
    /// withheld -- listed by `status`, reachable by `drop`, and settled again when one goes.
    ///
    /// A copy whose mode cannot be read never wins. It refuses writes anyway, because the store's own
    /// guard fails closed on an unreadable mode, so loading it as the machine's own copy would name a
    /// winner that cannot act as one.
    fn settle_identity(&mut self, identity: SetId) {
        let mut holders: Vec<(Scope, bool)> = self
            .stores
            .iter()
            .chain(self.withheld.iter())
            // **A copy loaded for review is never a holder.** It carries its source set's identity so
            // that a link naming that set can be reported against it, and it is adopted by nobody --
            // so counting it here would withhold the reviewer's own set the moment they opened a
            // review of it. `documents/design/load-modes.md`.
            .filter(|(_, store)| !store.load_mode().is_ok_and(LoadMode::is_evaluation))
            .filter(|(_, store)| store.set_id().ok().flatten() == Some(identity))
            .map(|(scope, store)| {
                (
                    scope.clone(),
                    store
                        .load_mode()
                        .is_ok_and(LoadMode::accepts_session_writes),
                )
            })
            .collect();
        holders.sort();

        let writable: Vec<Scope> = holders
            .iter()
            .filter(|(_, writable)| *writable)
            .map(|(scope, _)| scope.clone())
            .collect();
        let winner: Option<Scope> = match (holders.as_slice(), writable.as_slice()) {
            ([(only, _)], _) => Some(only.clone()),
            (_, [one]) => Some(one.clone()),
            _ => None,
        };

        for (scope, _) in &holders {
            if Some(scope) == winner.as_ref() {
                if let Some(store) = self.withheld.remove(scope) {
                    self.stores.insert(scope.clone(), store);
                }
            } else if let Some(store) = self.stores.remove(scope) {
                self.withheld.insert(scope.clone(), store);
            }
        }
    }

    /// Which other store answers to `identity`, loaded or withheld, besides the one at `scope`.
    fn other_holder_of(&self, scope: &Scope, identity: SetId) -> Option<Scope> {
        self.stores
            .iter()
            .chain(self.withheld.iter())
            // Review copies carry the identity and hold it for nothing, so an open review of a set
            // must not stop that set importing its own file.
            .filter(|(_, store)| !store.load_mode().is_ok_and(LoadMode::is_evaluation))
            .find(|(held, store)| *held != scope && store.set_id().ok().flatten() == Some(identity))
            .map(|(held, _)| held.clone())
    }

    /// Handle one JSON-RPC request.
    ///
    /// Returns `None` for a notification — a request with no `id`. Replying to one is a protocol
    /// violation, and some clients treat it as fatal.
    #[must_use]
    pub fn handle(&mut self, request: &str) -> Option<String> {
        let parsed: Value = match serde_json::from_str(request) {
            Ok(value) => value,
            // No id to echo, because the request could not be read well enough to find one.
            Err(error) => {
                return Some(error_response(
                    Value::Null,
                    -32700,
                    &format!("could not parse the request as JSON: {error}"),
                ));
            }
        };

        let id = parsed.get("id").cloned();
        let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
        let id = id?;

        // **What a client actually sends on a call, observed once per shape.** `server/discover`
        // and `initialize` say what a client declares at connection; a call can carry something
        // else, and on 2026-10-16 Claude Code declared form elicitation on discovery while its
        // first write was served as though it could not ask. This is how that is told apart.
        if method == "tools/call" {
            let shape = call_shape(&parsed["params"]);
            if self.call_shapes_logged.insert(shape.clone()) {
                eprintln!("scatterhoard: tools/call {shape}");
            }
        }

        // **The era is read from the request**, never remembered: a request naming its revision in
        // `_meta` is modern, and anything else is served under whatever `initialize` agreed.
        if parsed["params"]["_meta"].get(META_PROTOCOL).is_some() {
            return Some(self.handle_modern(id, method, &parsed));
        }
        self.exchange = Exchange::handshake(self.protocol);

        let outcome = match method {
            "initialize" => Ok({
                self.protocol = negotiated_version(parsed["params"]["protocolVersion"].as_str());
                // On stderr, the one channel beside the protocol: what a client declares is
                // observed rather than read from its documentation.
                eprintln!(
                    "scatterhoard: initialize from {} asking {}, declaring {}",
                    parsed["params"]["clientInfo"],
                    parsed["params"]["protocolVersion"],
                    parsed["params"]["capabilities"],
                );
                json!({
                "protocolVersion": self.protocol,
                "capabilities": {"tools": {}},
                "serverInfo": server_info(),
                })
            }),
            "tools/list" => Ok(json!({"tools": tool_definitions(self.exchange.protocol)})),
            "tools/call" => self.call_tool(parsed.get("params")),
            "ping" => Ok(json!({})),
            other => {
                return Some(error_response(
                    id,
                    -32601,
                    &format!("this server has no method {other:?}"),
                ));
            }
        };

        Some(match outcome {
            Ok(result) => success_response(id, result),
            // A tool that fails reports through the result, not as a protocol error: the model
            // should see the failure as an answer it can act on rather than as a broken call.
            Err(message) => success_response(
                id,
                json!({
                    "content": [{"type": "text", "text": message}],
                    "isError": true,
                }),
            ),
        })
    }

    /// Stop using any store whose file has gone or been replaced since it was opened.
    ///
    /// `a-set-dropped-by-another-server-stays-writable-here`: when another server sharing the home
    /// drops a set, this server's connection survives the file's deletion and would go on reporting
    /// writes as stored while putting them where nobody can reach. Reproduced 2026-10-16, and worse
    /// than that: with the set made again at the same path, the stale connection damaged the new
    /// set's database so that a fresh server could not open it. So before each call, a store whose
    /// file is no longer the one it opened is taken out of service, and the rescan after this picks up
    /// whatever is at the path now.
    ///
    /// **Closing the stale connection is safe, which was checked rather than assumed.** The worry was
    /// that closing the last connection in WAL mode checkpoints and deletes the `-wal` file by path,
    /// which after a re-create names the new set's. It does not: with the new set writing on both
    /// sides of the moment the stale one is let go, every write reached its file
    /// (`letting_go_of_a_stale_set_leaves_the_new_one_intact`). SQLite's pager skips the checkpoint
    /// on close when its database file has moved, as recalled from its source -- the test is the
    /// evidence, the mechanism is not verified here.
    fn let_go_of_vanished_stores(&mut self) {
        let gone: Vec<Scope> = self
            .stores
            .iter()
            .chain(self.withheld.iter())
            .filter(|(_, store)| store.file_is_gone())
            .map(|(scope, _)| scope.clone())
            .collect();
        for scope in gone {
            let Some(store) = self
                .stores
                .remove(&scope)
                .or_else(|| self.withheld.remove(&scope))
            else {
                continue;
            };
            eprintln!(
                "scatterhoard: {scope:?} is no longer the file this server opened -- dropped, or \
                 replaced, by another process -- so it is no longer used here"
            );
            let identity = store.set_id().ok().flatten();
            drop(store);
            // A copy withheld beside the one that went may be the machine's own now.
            if let Some(identity) = identity {
                self.settle_identity(identity);
            }
        }
    }

    /// Open any set whose database has appeared in this server's directories since it last looked.
    ///
    /// `a-set-created-by-another-server-is-unreachable-until-restart`: several servers share one home
    /// in ordinary use -- the desktop app's, Claude Code's, a chat's -- and a server that learned its
    /// sets only at startup could not reach a set another had made since, nor create or drop it, for
    /// the rest of its session. **The same scan and the same loading startup uses**, so a copy that
    /// collides with a set already held is withheld exactly as it would be after a restart, and
    /// nothing here is a second rule to keep in step with the first. Before every tool call, because
    /// any of them can name the set and `status` should list it; listing a directory or two is cheap
    /// beside the call itself.
    fn pick_up_new_stores(&mut self) {
        let Some(layout) = self.layout.clone() else {
            return;
        };
        for (scope, path) in layout.discover().stores {
            if self.stores.contains_key(&scope) || self.withheld.contains_key(&scope) {
                continue;
            }
            // `local` is opened at startup or not at all: a server built without it -- a test, or
            // a store held in memory -- has chosen that, and a file appearing later is not a reason
            // to swap its personal memory out from under it.
            if scope == Scope::Local {
                continue;
            }
            match Store::open_at(scope.clone(), &path) {
                Ok(store) => self.add_store(store),
                Err(error) => eprintln!("scatterhoard: could not open {scope:?}: {error}"),
            }
        }
    }

    /// Handle one request on [`MODERN_PROTOCOL`].
    ///
    /// **Stateless, as the revision requires**: the version and the client's capabilities come from
    /// this request's `_meta`, and nothing an earlier `initialize` set is read. Every result says
    /// it is complete, or that it needs the human's input first, and names the server.
    fn handle_modern(&mut self, id: Value, method: &str, parsed: &Value) -> String {
        let params = &parsed["params"];
        let meta = &params["_meta"];
        let asked = meta[META_PROTOCOL].as_str().unwrap_or_default();
        if asked != MODERN_PROTOCOL {
            let mut supported = vec![MODERN_PROTOCOL];
            supported.extend(PROTOCOL_VERSIONS);
            return json!({"jsonrpc": "2.0", "id": id, "error": {
                "code": -32022,
                "message": "Unsupported protocol version",
                "data": {"supported": supported, "requested": meta[META_PROTOCOL]},
            }})
            .to_string();
        }
        let Some(capabilities) = meta
            .get(META_CAPABILITIES)
            .filter(|value| value.is_object())
        else {
            return error_response(
                id,
                -32602,
                &format!(
                    "a request on {MODERN_PROTOCOL} must declare the client's capabilities in \
                     `_meta` under {META_CAPABILITIES:?}"
                ),
            );
        };

        self.exchange = Exchange {
            protocol: MODERN_PROTOCOL,
            asks_human: declares_form_elicitation(capabilities),
            answers: params
                .get("inputResponses")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            question: None,
            settled: None,
        };

        let outcome = match method {
            "server/discover" => {
                eprintln!(
                    "scatterhoard: server/discover from {} on {MODERN_PROTOCOL}, declaring {capabilities}",
                    meta[META_CLIENT],
                );
                Ok(json!({
                    "supportedVersions": [MODERN_PROTOCOL],
                    "capabilities": {"tools": {}},
                    "ttlMs": LIST_TTL_MS,
                    "cacheScope": "public",
                }))
            }
            "tools/list" => Ok(json!({
                "tools": tool_definitions(MODERN_PROTOCOL),
                "ttlMs": LIST_TTL_MS,
                "cacheScope": "public",
            })),
            "tools/call" => self.call_tool(parsed.get("params")),
            other => {
                return error_response(
                    id,
                    -32601,
                    &format!("this server has no method {other:?} on {MODERN_PROTOCOL}"),
                );
            }
        };

        let mut result = match outcome {
            Ok(result) => result,
            Err(message) => json!({
                "content": [{"type": "text", "text": message}],
                "isError": true,
            }),
        };
        if result.get("resultType").is_none() {
            result["resultType"] = json!("complete");
        }
        result["_meta"] = json!({"io.modelcontextprotocol/serverInfo": server_info()});
        success_response(id, result)
    }

    fn call_tool(&mut self, params: Option<&Value>) -> Result<Value, String> {
        let params = params.ok_or("tools/call needs params")?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or("tools/call needs a tool name")?;
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        self.let_go_of_vanished_stores();
        self.pick_up_new_stores();
        let outcome = match name {
            "hoard_conventions" => self.conventions(&arguments),
            "hoard_memorize" => self.memorize(&arguments),
            "hoard_lookup" => self.lookup(&arguments),
            "hoard_recall" => self.recall(&arguments),
            "hoard_sets" => self.sets(&arguments),
            "hoard_correct" => self.correct(&arguments),
            other => Err(format!("this server has no tool {other:?}")),
        };
        // **A question outranks whatever the handler returned**: it stopped there to ask, before
        // writing anything, and its refusal text is for nobody. The client puts the question to the
        // human and retries this call with the answer.
        if let Some((key, question)) = self.exchange.question.take() {
            return Ok(json!({
                "resultType": "input_required",
                "inputRequests": {key: {"method": "elicitation/create", "params": question}},
            }));
        }
        let mut payload = outcome?;
        if let Some(note) = self.exchange.settled.take()
            && payload.is_object()
        {
            payload["author_note"] = json!(note);
        }

        // A string payload is already the text to serve -- only `hoard_conventions` returns one --
        // and serialising it would wrap it in quotes and escape every line break.
        //
        // **A JSON payload also goes out as `structuredContent`**, the form the 2025-06-18 revision
        // checks against a tool's `outputSchema` (`response-shape-is-unpublished`). The text copy
        // is the same JSON, kept because the revision asks for it and an older client reads only
        // that.
        Ok(match payload {
            Value::String(text) => json!({
                "content": [{"type": "text", "text": text}],
                "isError": false,
            }),
            structured if !publishes_output_schemas(self.exchange.protocol) => json!({
                "content": [{"type": "text", "text": structured.to_string()}],
                "isError": false,
            }),
            structured => json!({
                "content": [{"type": "text", "text": structured.to_string()}],
                "structuredContent": structured,
                "isError": false,
            }),
        })
    }

    fn conventions(&mut self, arguments: &Value) -> Result<Value, String> {
        #[derive(Deserialize, Default)]
        struct Params {
            #[serde(default)]
            cursor: usize,
            #[serde(default)]
            max_chars: Option<usize>,
        }
        let params: Params = parse(arguments)?;
        // **A session begins here.** See `SessionId` for why the process cannot say when: it
        // outlives the session, measured. The first page and not a continuation -- paging is one
        // load, and splitting a session there would drop the label for every query asked before the
        // catalog finished arriving.
        if params.cursor == 0 {
            self.session = SessionId::generate();
        }

        // Whatever the server says beside the catalog. It travels as one JSON line between the
        // catalog and the page's last line, and is spent from the same budget.
        let mut result = json!({});
        if let Some(build) = self.rebuilt_since_launch() {
            // **Beside the catalog, never instead of it.** A session that cannot read the
            // conventions is worse off than one reading conventions a build behind.
            result["note"] = json!(format!(
                "The Scatterhoard binary has been rebuilt since this server started ({}). This \
                 session is being served the older build, version {} -- these conventions, and every behaviour \
                 of these tools, are what that build does, and nothing here will change until the \
                 MCP client is restarted. Say so rather than working around it: a catalog a build \
                 behind answers every call successfully and simply governs.",
                build.described(),
                env!("CARGO_PKG_VERSION"),
            ));
        }
        // **Which build this is**, for a bug report or to notice it differs from a tester's. The
        // first page only, like the descriptions below: paging is one load, and the session is the
        // unit that needs to know. The same constant as `serverInfo` and `scatterhoard --version`,
        // so the three cannot disagree.
        if params.cursor == 0 {
            result["server_version"] = json!(env!("CARGO_PKG_VERSION"));
        }
        // The first page only, for the reason a session begins there: a continuation is the same
        // load, and repeating this on every page would make it the loudest thing in the catalog.
        let paid = if params.cursor == 0 {
            self.pay_owed_descriptions()
        } else {
            Vec::new()
        };
        if !paid.is_empty() {
            result["imported_sets_described"] = json!(paid);
            result["imported_sets_guidance"] = json!(
                "These sets were imported while their memories could not be embedded, so the \
                 import could not say what they amount to. They are described now, from the \
                 memories this machine holds. Read each description against the set's \
                 `hoard-scope-topics` charter: the two answer the same question from different \
                 sides. This describes and does not judge, and it is repeated at every session start until the \
                 set is next exported."
            );
        }
        // **Read at the first page and kept for the session**; a continuation with nothing kept
        // -- a server restarted partway through paging -- reads the file afresh rather than
        // serving a document without it. A file that cannot be read is said beside the catalog on
        // the first page and costs the session its preferences, never the conventions.
        if params.cursor == 0 || self.sharing_preferences.is_none() {
            match self.load_sharing_preferences() {
                Some(Ok(preferences)) => self.sharing_preferences = Some(preferences.served()),
                Some(Err(problem)) => {
                    self.sharing_preferences = None;
                    if params.cursor == 0 {
                        result["sharing_preferences"] = json!(format!(
                            "The person's sharing preferences could not be served, so this session \
                             has none: {problem}. Tell them, since the file is how they say which of \
                             their own details may be stored."
                        ));
                    }
                }
                None => self.sharing_preferences = None,
            }
        }
        if params.cursor == 0 {
            let due = self.reviews_due()?;
            if !due.is_empty() {
                result["review_due"] = json!(due);
                result["review_due_note"] = json!(REVIEW_DUE_NOTE);
            }
        }
        let aside = result
            .as_object()
            .is_some_and(|aside| !aside.is_empty())
            .then(|| result.to_string());
        let page = match &self.sharing_preferences {
            Some(preferences) => conventions::page_of(
                &conventions::document_with(preferences),
                params.cursor,
                params.max_chars,
                aside.as_deref(),
            ),
            None => conventions::page(params.cursor, params.max_chars, aside.as_deref()),
        }
        .map_err(|refusal| refusal.to_string())?;
        // **Text, not a JSON value.** A page is a slice of one document plus lines addressed to the
        // reader, so it is served as written; `call_tool` passes a string payload through as is.
        Ok(Value::String(page.text))
    }

    /// The person's sharing preferences, from the common directory, or `None` with nowhere to keep
    /// them. `preferences::Preferences::load` writes the template first when the file is missing.
    /// Why `fields` cannot be written, if they hold a word the person's file never stores.
    fn never_store_refusal(&self, fields: &[&str], nothing: &str) -> Option<String> {
        let layout = self.layout.as_ref()?;
        preferences::never_store_refusal(layout.common(), fields, nothing)
    }

    fn load_sharing_preferences(&self) -> Option<Result<Preferences, String>> {
        let layout = self.layout.as_ref()?;
        Some(Preferences::load(layout.common()))
    }

    /// Describe every loaded set an import left owing a description, once it can be described.
    ///
    /// A set whose memories still lack vectors is passed over in silence: there is nothing yet to
    /// say, and saying so at every session start on a machine that simply has no model would be
    /// noise a reader learns to skip. A set that cannot be read at all is reported, because that
    /// silence would not be temporary.
    fn pay_owed_descriptions(&mut self) -> Vec<Value> {
        let mut paid = Vec::new();
        for (scope, store) in &mut self.stores {
            let mut entry = match Self::owed_description_of(store) {
                Ok(None) => continue,
                Ok(Some(entry)) => entry,
                Err(error) => json!({ "error": error }),
            };
            entry["scope"] = json!(WireScope(scope));
            paid.push(entry);
        }
        paid
    }

    /// One set's owed description, if it is owed and can now be given.
    fn owed_description_of(store: &mut Store) -> Result<Option<Value>, String> {
        let Some(record) = store
            .owed_description()
            .map_err(|error| format!("could not read the owed description: {error}"))?
        else {
            return Ok(None);
        };
        let _: OwedDescription = serde_json::from_str(&record)
            .map_err(|error| format!("the owed description is unreadable: {error}"))?;

        store
            .catch_up_vectors()
            .map_err(|error| format!("could not embed the set to describe it: {error}"))?;
        if store
            .awaiting_vectors()
            .map_err(|error| format!("could not tell whether the set is embedded: {error}"))?
            > 0
        {
            return Ok(None);
        }

        let derived = Self::derive_summary(store)?;
        Ok(Some(json!({
            "describes": derived.as_ref().map_or(Value::Null, describe),
        })))
    }

    fn memorize(&mut self, arguments: &Value) -> Result<Value, String> {
        /// Every field is a validated type, so this struct *is* the validation.
        #[derive(Deserialize)]
        struct Params {
            scope: Scope,
            content: MemoryText,
            #[serde(default)]
            why: Option<MemoryText>,
            #[serde(default)]
            context: Option<MemoryText>,
            /// `Option` rather than a defaulted `Vec`, so **absent and explicitly empty are
            /// distinguishable**. That distinction is the whole of the guard below: a caller
            /// writing a `supersedes` link must state what the replacement is indexed under, and
            /// `[]` is a legal answer while saying nothing is not. `hoard_correct` has taken the
            /// same shape for the same reason since it gained the field.
            #[serde(default)]
            keywords: Option<Vec<Keyword>>,
            confidence: Confidence,
            origin: Origin,
            #[serde(default)]
            links: Vec<Link>,
        }

        let params: Params = parse(arguments)?;

        // Decoded before anything else looks at the text, so the escape is what a caller writes
        // *instead of* markup rather than a second thing to get right afterwards.
        let content = decoded(params.content)?;
        let why = params.why.map(decoded).transpose()?;
        let context = params.context.map(decoded).transpose()?;

        let now = jiff::Timestamp::now();
        let mut links = params.links;
        self.identify_link_targets(&mut links)?;
        // Writing something that refers to a memory is the strongest of the three use signals: the
        // session did not merely read it. Done here rather than after the write, so a link the
        // refusals below reject still counts -- the session had the memory in hand either way, and
        // that is what the label claims.
        let targets: Vec<MemoryId> = links.iter().map(|link| link.target.id()).collect();
        self.note_used(&targets)?;

        // **Every refusal below runs before the author is resolved, and that ordering is
        // load-bearing.** Resolving may *mint* an identity record for a set that has none, which is
        // a write -- so resolving first would leave a refused call having stored something, which
        // is the exact promise `a_swallowed_parameter_is_refused_and_leaves_nothing_behind` exists
        // to keep. Nothing below reads the attribution, so there is no cost to waiting.

        // `malformed-tool-call-damage-is-stored-silently` in TRACKING.md, thirteen occurrences.
        //
        // **Refused before the write, not remarked on after it.** A mis-closed parameter absorbs
        // everything up to the next well-formed close, so the framing lands inside the stored text
        // and the parameters after it never arrive. The remark this replaces put the fact where the
        // caller still had it, which was an improvement on silence and still left the memory stored
        // damaged -- and the repair is itself a site of the defect, because writing a correction is
        // exactly the moment of emitting many parameters in sequence. Four of the recorded
        // occurrences were repairs for the one before. Refusing turns a corruption plus a repair
        // into one failed call.
        //
        // Safe to refuse only because the escape above exists: a memory whose subject is markup can
        // now be written without emitting any, so this no longer makes a legitimate memory
        // unstorable. That ordering was a precondition, not a refinement.
        let swallowed = swallowed_parameters(
            &[
                content.as_data(),
                why.as_ref().map_or("", MemoryText::as_data),
                context.as_ref().map_or("", MemoryText::as_data),
            ],
            &|field| match field {
                "why" => why.is_none(),
                "context" => context.is_none(),
                // `is_none`, not `is_empty`: a swallowed parameter never arrives, while `[]` is a
                // caller who sent one. The two were indistinguishable until `keywords` became an
                // `Option`, and conflating them made this fire on a legitimate empty list.
                "keywords" => params.keywords.is_none(),
                "links" => links.is_empty(),
                // Both are required, so serde has already refused a call that lost them. They are
                // listed for the framing they contribute rather than as fields that can go absent.
                _ => false,
            },
        );
        if !swallowed.is_empty() {
            return Err(format!(
                "This call looks damaged, so nothing was stored. The text carries tool-call \
                 framing naming {}, and {} arrived empty -- which together mean a parameter was \
                 closed with the wrong tag and everything after it was absorbed into the text. \
                 Nothing is lost: send it again with each parameter closed properly. If you meant \
                 to write about the markup itself, escape it -- &lt; &gt; and &amp; are decoded \
                 into the characters they name, so a memory about this defect can say so without \
                 emitting any.",
                swallowed.join(" and "),
                if swallowed.len() == 1 { "it" } else { "they" }
            ));
        }

        // Before anything is written, and after decoding, so the escape cannot be used to smuggle a
        // credential past the scan as entity references.
        if let Some(refusal) = prohibited_data_refusal(&[
            ("content", content.as_data()),
            ("why", why.as_ref().map_or("", MemoryText::as_data)),
            ("context", context.as_ref().map_or("", MemoryText::as_data)),
        ]) {
            return Err(refusal);
        }

        // **Beside the secrets scan, and for the same reason it refuses rather than reports**: the
        // caller holds the remedy, which is to reword. The person's never-store list, read from the
        // file now, so the check holds in a session that never loaded the conventions.
        let mut written = vec![
            content.as_data(),
            why.as_ref().map_or("", MemoryText::as_data),
            context.as_ref().map_or("", MemoryText::as_data),
        ];
        written.extend(params.keywords.iter().flatten().map(Keyword::as_str));
        if let Some(refusal) = self.never_store_refusal(&written, "Nothing was stored") {
            return Err(refusal);
        }

        let holder = self
            .stores
            .get(&params.scope)
            .map(Store::set_id)
            .transpose()
            .map_err(|error| format!("could not read the set's identity: {error}"))?
            .flatten();
        if let Some(refusal) = cross_set_authority_refusal(&links, &params.scope, holder) {
            return Err(refusal);
        }

        if let Some(refusal) = unstated_keywords_refusal(&links, params.keywords.as_deref()) {
            return Err(refusal);
        }

        if links.iter().any(|link| !link.label.is_recognised()) {
            let held = match self.stores.get(&params.scope) {
                Some(store) => superseded_links(store, &links, &params.scope, holder)?,
                None => Vec::new(),
            };
            if let Some(refusal) = unknown_label_refusal(&links, &held) {
                return Err(refusal);
            }
        }

        // After decoding, so an escaped entity is measured at the length it will be stored and
        // embedded at rather than at the length it was written as.
        if let Some(refusal) = over_length_refusal(
            content.as_data(),
            why.as_ref().map(MemoryText::as_data),
            params.keywords.as_deref().unwrap_or_default(),
        ) {
            return Err(refusal);
        }

        // **A shared set's first memory has to be its charter**, since `set-settled-at-first-write`
        // (decided 2026-09-24). Here rather than at `create`, which cannot tell a new set from one
        // about to be taken on by `import`; and before the author is resolved, because resolving
        // mints a record, and a refused write must leave the set as empty as it found it. After the
        // read-only refusal, so a set that takes no writes is not told to write a charter. Only the
        // first write: a set already holding memories without a charter came from someone else, and
        // `status` reports that rather than refusing.
        self.refuse_if_read_only(&params.scope)?;
        if let Some(refusal) =
            self.charter_first_refusal(&params.scope, params.keywords.as_deref())?
        {
            return Err(refusal);
        }

        // Nothing can refuse from here on, so minting is safe.
        let author = self.author_for(&params.scope)?;
        let mut memory = Memory::new(
            content,
            params.confidence,
            params.origin,
            Attribution {
                by: author,
                at: now,
            },
        );
        memory.why = why;
        memory.context = context;
        memory.keywords = params.keywords.unwrap_or_default();
        memory.links = links;

        let id = memory.id;
        // Before the write, because resolving a link needs `&self` across every store while the
        // write needs `&mut` on one. Nothing here can change the outcome of the write -- this
        // reports, it does not gate.
        let unresolved = self.unresolved_links(&params.scope, &memory.links)?;
        // Before the write, and read from the store rather than from the call, because the caller
        // states the *new* set and the question is what changed against the old one.
        let deltas = self
            .stores
            .get(&params.scope)
            .map(|store| keyword_deltas(store, &memory.links, &memory.keywords))
            .unwrap_or_default();
        let link_changes = self
            .stores
            .get(&params.scope)
            .map(|store| link_deltas(store, &memory.links, &params.scope, holder))
            .unwrap_or_default();
        // Before the write, so the replacement's own `supersedes` edge is not among them -- and the
        // superseded memories are the ones `link_deltas` chose, so the two reports cannot disagree
        // about which memories this call replaced.
        let repointed = {
            let stores: Vec<&Store> = self.stores.values().collect();
            repointed_in_links(&stores, &params.scope, &link_changes).map_err(|error| {
                format!("could not read what points at the superseded memory: {error}")
            })?
        };
        let store = self.store_for_mut(&params.scope)?;
        store
            .insert(&memory)
            .map_err(|error| format!("could not store the memory: {error}"))?;

        // Asked of the store, about this memory, after the write. Reporting what `insert` intended
        // -- or a server-wide capability flag -- is the shape that hid
        // `project-stores-get-no-embedder`: it was right about the case someone had in mind and
        // silent about every other way a vector fails to arrive.
        let findable = store
            .is_findable_by_meaning(id)
            .map_err(|error| format!("could not check whether the memory is findable: {error}"))?;

        let mut result = json!({
            "id": id.to_string(),
            "findable_by_meaning": findable,
            // The line this memory would contribute to an index, echoed at the moment of writing.
            //
            // **Not a convenience.** A character cap is invisible while composing a call, to a
            // model as much as to a person, so a rule stated as a length is a rule its own author
            // cannot follow -- which is what the length rule it replaces did. Showing the derived
            // line is the version of that rule nobody has to count for.
            //
            // **Echoed, never refused.** A weak opening is craft, not damage: the memory is fine
            // and only its first line is thin, and blocking a legitimate write over presentation is
            // not what refusal is for.
            //
            // **Always, rather than when the memory is marked.** Keying it to a keyword would
            // couple this response to `hoard-watch`, whose own open defect is likely to change that
            // keyword, and would confine the reflection to the one write where the author is
            // already paying attention. The cost is about 130 characters on a call that happens a
            // handful of times a session. The risk is that an always-present field gets skimmed --
            // real, but the predictor measured on this surface is *invariance* rather than
            // presence, and this line differs with every call.
            "summary": memory.content.summary_as_data(),
        });

        if !findable {
            let unrepresentable = store.is_unrepresentable(id).map_err(|error| {
                format!("could not check whether the memory is findable: {error}")
            })?;
            result["note"] = json!(if unrepresentable {
                // **Not the advice below.** Keywords are part of the embedded text, so one in a
                // language the model knows would give this memory a vector built from that keyword
                // alone -- findable, and matched on something other than what it says. And a model
                // is installed: the remedy is one that covers this language, which is the user's
                // to choose. `one-unembeddable-memory-stops-the-store`.
                "This memory was stored, but this set's embedding model has no tokens for any of \
                 its text, so hoard_recall cannot find it by meaning. It is still reachable by the \
                 keywords it has and by hoard_lookup. Tell the user this set's model does not cover \
                 the language this memory is written in."
            } else {
                // Names what the caller can still do *now*, while it is cheap. Saying only that a
                // model is missing would be advice its reader cannot act on -- the caller cannot
                // install one.
                "This memory was stored but has no embedding, so hoard_recall will never find it \
                 by meaning. It is still reachable by exact keyword match and by hoard_lookup. Add \
                 keywords to it now if it needs to be findable, and tell the user Scatterhoard \
                 needs its model installed for this set."
            });
        }
        report_keyword_deltas(&mut result, &deltas);
        report_link_deltas(&mut result, &link_changes);
        report_repointed_in_links(&mut result, repointed);
        report_unresolved(&mut result, &unresolved);
        Ok(result)
    }

    /// Takes `&mut self` because recall reads each store's vector cache, and a store shared with
    /// another server process has to bring that cache up to date first — see [`Store`].
    fn recall(&mut self, arguments: &Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Params {
            /// Omitted means every loaded set.
            ///
            /// Optional here although it is required on `hoard_memorize`, and the asymmetry is the
            /// point: storing into the wrong set is a one-way door, while searching the wrong set
            /// is a read that costs nothing and returns less. Project ∪ local is also the normal
            /// case — a stated preference matters while working on a project — so the default is
            /// what a caller almost always wants.
            #[serde(default)]
            scope: Option<Scope>,
            #[serde(default)]
            query: Option<String>,
            /// Mutually exclusive with `query`, not merely searched separately from it: a match
            /// count and a cosine score are different scales, and one call answering both has no
            /// honest way to order the result.
            #[serde(default)]
            keywords: Vec<Keyword>,
            /// Taken as any JSON value and checked here, for the reason `already_known` is: so a
            /// zero, a negative number or a fraction is refused by name, saying what the argument
            /// takes, rather than as whatever serde would say about a `usize`.
            #[serde(default)]
            limit: Option<Value>,
            /// Ids of memories the caller already has in context.
            ///
            /// Taken as strings and parsed here so a mistyped id is refused by name, rather than as
            /// whatever serde would say about a sequence element.
            #[serde(default)]
            already_known: Vec<String>,
            /// How much of each memory to return. Defaults to the whole record.
            #[serde(default)]
            detail: Detail,
            /// Ask again without this set's non-match floor, after being told it silenced
            /// everything.
            #[serde(default)]
            ignore_floor: bool,
        }

        let params: Params = parse(arguments)?;
        let mut already_known = Vec::with_capacity(params.already_known.len());
        for id in &params.already_known {
            already_known.push(MemoryId::parse(id).map_err(|_| {
                format!(
                    "already_known must hold whole memory ids as hoard_recall returned them, and \
                     {id:?} is not one. Pass the ids from an earlier result, not a prefix."
                )
            })?);
        }
        // Memories the caller says it already holds, which is the use signal that arrives without a
        // second tool call. Before the scan, so this query's own row -- written during it -- is not
        // labelled by the ids that came with it: what is being recorded is that an *earlier* answer
        // was used.
        self.note_used(&already_known)?;

        let under_review = self.evaluation_scopes();
        let loaded_sets = self.loaded_sets();
        // Copied out before the stores are borrowed mutably, which is the whole of the reason: a
        // session identity is a plain value and reading it through `&self` afterwards would be a
        // second borrow of the thing already lent to the scan.
        let session = self.session;
        let mut stores: Vec<&mut Store> = match &params.scope {
            Some(scope) => vec![self.store_for_mut(scope)?],
            // **A copy loaded for review is searched only when its scope is named.** Excluding it
            // here rather than filtering its hits afterwards is the whole point: scores are computed
            // against every store a recall searched, so a copy nobody adopted would shift the
            // ranking of everything else even if none of its own memories were returned.
            // `documents/design/load-modes.md`.
            None => self
                .stores
                .values_mut()
                .filter(|store| !store.load_mode().is_ok_and(LoadMode::is_evaluation))
                .collect(),
        };

        // A plain count on both paths: how many to return, best first. **Honoured as given above
        // one, with no ceiling, by decision** (`recall-limit-is-unbounded`): the caller holds its
        // context budget and the server cannot see it, so a ceiling here would second-guess an
        // explicit request. Zero is refused because an empty list reads as *nothing matched*.
        let limit = match &params.limit {
            None => DEFAULT_LIMIT,
            Some(value) => value
                .as_u64()
                .filter(|&n| n >= 1)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| {
                    format!(
                        "`limit` must be a whole number of 1 or more, not {value}: it is how many \
                         memories to return, best first. Omit it for {DEFAULT_LIMIT}."
                    )
                })?,
        };

        // One search per call. Answering both in one response is what produced
        // `combined-recall-duplicates-content`: the two paths were run independently and their
        // results stapled together, so a memory found both ways came back twice in full, and the
        // response carried a cosine score and a match count with no honest way to order them
        // against each other. Nothing was integrated, so the combined form bought a caller nothing
        // two calls would not — while costing the whole shape. Refusing it makes the duplication,
        // the two loose scales, and the ordering question unrepresentable rather than fixed.
        // Hybrid search is the better end state and is deliberately deferred: see
        // `TRACKING.md`.
        let in_scope: Vec<Scope> = stores.iter().map(|store| store.scope().clone()).collect();
        let (memories, coverage) = match (&params.query, params.keywords.is_empty()) {
            (Some(_), false) => {
                return Err(
                    "hoard_recall takes a query or keywords, not both: a similarity and a \
                            match count are different scales and cannot be ranked against each \
                            other. Search by meaning, or match keywords exactly, and call twice if \
                            you want both."
                        .to_owned(),
                );
            }
            (None, true) => {
                // **The caller's own call, with the missing part added**
                // (`the-missing-query-refusal-gives-no-example`). Naming what was missing was not
                // enough: `gemma4:12b`, asked to read a whole set, answered the old wording with a
                // call still missing both, 10 times out of 10, and this with a query, 10 of 10.
                // The placeholder cannot be sent as it stands without reading as one, where a
                // filled-in example could be.
                let mut example = arguments.as_object().cloned().unwrap_or_default();
                example.remove("keywords");
                example.insert("query".to_owned(), json!("<what you are looking for>"));
                return Err(format!(
                    "hoard_recall needs something to search for, and this call has neither a \
                     `query` nor `keywords`. Recall always searches: it has no mode that lists a \
                     set without one. Call it again with a `query` in your own words, searched by \
                     meaning, or `keywords` to match exactly -- for example {}.",
                    Value::Object(example)
                ));
            }
            (Some(query), true) => {
                // **The one call that says a session is asking**, which is what puts a query into
                // each searched store's log. Everything else that reaches `recall_across` is
                // instrumentation, and a measurement recorded as traffic is evidence about nothing.
                let mut question = Question::new(query, limit)
                    .already_holding(&already_known)
                    .in_a_session(params.scope.is_some(), jiff::Timestamp::now(), session);
                if params.ignore_floor {
                    question = question.without_the_floor();
                }
                let recall = recall_across(&mut stores, &question)
                    .map_err(|error| format!("recall failed: {error}"))?;
                let views = recall
                    .hits
                    .iter()
                    .map(|hit| {
                        let reviewing = under_review.contains(&hit.scope);
                        let mut view =
                            MemoryView::semantic(hit, reviewing).to_json_at(params.detail);
                        surface_cross_set_targets(&mut view, reviewing, &hit.memory, &loaded_sets);
                        view
                    })
                    .collect();
                (views, Coverage::from_recall(&recall))
            }
            (None, false) => {
                // One merged, ranked, cut list -- the shape the semantic path above already
                // has. Looping here instead gave each store its own `limit` and concatenated
                // the results, which is `keyword-recall-multiplies-the-limit-by-the-number-of-sets`.
                let searched: Vec<&Store> = stores.iter().map(|store| &**store).collect();
                // With the held ids, as the semantic path above has them:
                // `keyword-recall-ignores-already-known`.
                let views: Vec<Value> = search_keywords_across_holding(
                    &searched,
                    &params.keywords,
                    limit,
                    &already_known,
                )
                .map_err(|error| format!("keyword search failed: {error}"))?
                .iter()
                .map(|hit| {
                    let reviewing = under_review.contains(&hit.scope);
                    let mut view = MemoryView::keyword(hit, reviewing).to_json_at(params.detail);
                    surface_cross_set_targets(&mut view, reviewing, &hit.memory, &loaded_sets);
                    view
                })
                .collect();
                // Asked of the stores rather than assumed. This path does not run a semantic
                // search, which is exactly why it cannot infer the answer — and it is the path a
                // caller with no model is on, so a wrong answer here is maximally misleading.
                let mut coverage = Coverage::keyword_path();
                for store in &stores {
                    if !store
                        .can_search_by_meaning()
                        .map_err(|error| format!("could not read the model state: {error}"))?
                    {
                        coverage.skipped.push(json!({
                                "scope": WireScope(store.scope()),
                                "reason": store
                                    .why_unsearchable()
                                    .map_err(|error| format!("could not read the model state: {error}"))?,
                            }));
                    }
                }
                coverage.searched = false;
                coverage.complete = coverage.skipped.is_empty();
                (views, coverage)
            }
        };

        let mut result = json!({
            "memories": memories.clone(),
            // **Unchanged in name and meaning: can every set in scope be searched by meaning.**
            // That is a capability statement, which is what the keyword path needs -- it runs no
            // semantic search and cannot report whether one happened, and reporting a guess there
            // is what `keyword-only recall lies about semantic_search_available` was.
            //
            // No second field beside it, because *every set is searchable* and *the search covered
            // every set* are the same predicate: a set is skipped exactly when it cannot be
            // searched. A `complete` field would be a second copy of this one.
            "semantic_search_available": coverage.complete,
        });
        note_any_under_review(&mut result, &memories);
        if !coverage.skipped.is_empty() {
            result["skipped_sets"] = json!(coverage.skipped);
        }
        if !coverage.complete {
            // The per-set reason lives in `skipped_sets`; this says what the caller should
            // conclude, which differs sharply between the two cases and is what the single old
            // sentence got wrong -- it announced that nothing was searched while returning a
            // ranking. `semantic-search-available-conflates-partial-with-none`.
            result["note"] = json!(if coverage.searched {
                "Some sets in scope could not be searched by meaning, and `skipped_sets` names \
                 which and why. The memories returned were searched by meaning and their scores \
                 are sound -- an unsearchable set is left out of the scan rather than compared \
                 across vector spaces -- so treat this as an incomplete answer rather than an \
                 unreliable one, and say which sets were missing if it matters to the conclusion."
            } else {
                "No set in scope could be searched by meaning, and `skipped_sets` names why for \
                 each. Keyword search and hoard_lookup still work and are unaffected. Treat \
                 semantic recall as unconsulted rather than as empty."
            });
        }
        if coverage.silenced_by_floor {
            // **Named with the way round it**, because a caller who cannot act on the report is
            // merely being told that its question failed. The floor is a level calibrated from this
            // set's own past traffic, and the session asking is better placed than the store to
            // know whether this particular question deserved an answer anyway.
            result["note"] = json!(
                "Nothing scored above this set's non-match floor, which is a level calibrated \
                 from what the set has been able to answer before -- so this reads as *nothing \
                 here answers that* rather than as an empty set. Ask again with \
                 `ignore_floor: true` to see what was there regardless."
            );
        }
        if !coverage.answered_only_without_a_floor.is_empty() {
            // **Marked, not withheld.** Every set with a floor declined this question, so what came
            // back is from sets that have nothing separating a non-match from an answer -- often
            // their everyday background, sometimes a real answer. Only the caller can tell, and it
            // has the way to ask one set on its own. Joins a note already there rather than
            // replacing it, since which sets were skipped is still true.
            // `note-when-only-uncalibrated-sets-answer`.
            result["answered_only_without_a_floor"] = json!(
                coverage
                    .answered_only_without_a_floor
                    .iter()
                    .map(WireScope)
                    .collect::<Vec<_>>()
            );
            let marked = "Every set with a non-match floor declined this question, so these \
                          memories come only from sets without one, where nothing marks a \
                          non-match -- they may be that set's usual background rather than an \
                          answer. Judge them on their content, and ask again with `scope` naming \
                          one set to search it alone.";
            result["note"] = match result.get("note").and_then(Value::as_str) {
                Some(earlier) => json!(format!("{earlier} {marked}")),
                None => json!(marked),
            };
        }
        if params.query.is_some() {
            // **Only by meaning**: keyword recall matches spellings, which no model takes part in.
            // Every set searched is named, not only those that returned something, because a set
            // its model cannot read can also come back empty -- and empty from a set the agent
            // trusts is the same confusion as noise. `recall-is-latin-script-only`.
            let mut notes = Vec::new();
            for scope in &in_scope {
                let Some(store) = self.stores.get(scope) else {
                    continue;
                };
                let tags = store
                    .declared_languages()
                    .map_err(|error| {
                        format!("could not read what this set's charter declares: {error}")
                    })?
                    .unwrap_or_default();
                // Only the bands: what the set's own sentences need is setup work, said in status,
                // and would repeat on every recall without changing how its results read.
                let said = self.language_fit(store, &tags)?.bands;
                if !said.is_empty() {
                    notes.push(format!("In {}: {}", scope.as_key(), said.join(" ")));
                }
            }
            if !notes.is_empty() {
                result["language_note"] = json!(notes.join(" "));
            }
        }
        if coverage.query_unrepresentable {
            // Replaces the note above rather than joining it: when the question itself cannot be
            // represented, which sets could have been searched is beside the point.
            result["note"] = json!(
                "Nothing was searched by meaning: the embedding model has no tokens for any of \
                 this query's text, so there is nothing to compare it against. Search by keywords \
                 instead, which match exactly in any script; hoard_lookup is unaffected."
            );
        }
        Ok(result)
    }

    /// Fetch by whole id, or — said explicitly — by a fragment of one.
    ///
    /// `crockford-base32-ids`. `id` no longer accepts a partial value: a shortened id is refused
    /// rather than resolved, so truncation fails loudly instead of returning a plausible wrong
    /// memory. Recovery is `fragment`, which the caller has to choose, because **the caller is the
    /// only party that knows whether it holds a whole id or a piece of one.**
    fn lookup(&self, arguments: &Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Params {
            #[serde(default)]
            id: Option<String>,
            #[serde(default)]
            fragment: Option<String>,
        }
        let params: Params = parse(arguments)?;

        let hits = match (&params.id, &params.fragment) {
            (Some(_), Some(_)) => {
                return Err(
                    "hoard_lookup takes `id` or `fragment`, not both. `id` is a whole \
                            id and returns at most one memory; `fragment` is a search and may \
                            return several."
                        .to_owned(),
                );
            }
            (None, None) => {
                return Err(
                    "hoard_lookup needs an `id` -- a whole one, as a link or a recall \
                            result gave it to you. If all you have is part of an id, pass it as \
                            `fragment` instead."
                        .to_owned(),
                );
            }
            (Some(id), None) => {
                // Refused rather than resolved. A truncated id used to match by prefix and return
                // whatever shared the timestamp, which is `lookup-prefix-selectivity-is-time-correlated`.
                let id = MemoryId::parse(id).map_err(|_| {
                    format!(
                        "{id:?} is not a whole memory id. An id is 26 characters. If this \
                         is part of one, pass it as `fragment` -- and prefer a piece from the \
                         *end*, which identifies a memory far more sharply than one from the start."
                    )
                })?;
                let mut hits = Vec::new();
                for store in self.stores.values() {
                    if let Some(hit) = store
                        .lookup(id)
                        .map_err(|error| format!("lookup failed: {error}"))?
                    {
                        hits.push(hit);
                    }
                }
                hits
            }
            (None, Some(fragment)) => {
                let mut hits = Vec::new();
                for store in self.stores.values() {
                    hits.extend(
                        store
                            .find_by_fragment(fragment)
                            .map_err(|error| format!("lookup failed: {error}"))?,
                    );
                }
                hits
            }
        };

        // Looking a memory up is a use of it, whichever branch above found it: an id comes from a
        // link or from a recall result, and a fragment search that lands on a memory is the same
        // act with a worse handle.
        let reached: Vec<MemoryId> = hits.iter().map(|hit| hit.memory.id).collect();
        self.note_used(&reached)?;

        // Filled here rather than in `Store::lookup`, which sees one store and therefore cannot see
        // a claim written in another. Done once over the collected hits so a fragment matching
        // several memories costs one pass, not one per branch above.
        let under_review = self.evaluation_scopes();
        let loaded_sets = self.loaded_sets();
        let stores: Vec<&Store> = self.stores.values().collect();
        let mut hits = hits;
        for hit in &mut hits {
            hit.disputed_by =
                claims_against_hit(&stores, &hit.scope, hit.memory.id).map_err(|error| {
                    format!("could not check for claims against this memory: {error}")
                })?;
            // Filled here for the same reason as the claims: the row lives in the *source's* store,
            // and `Store::lookup` sees one store, so this is the only layer that can find it.
            hit.referenced_from = referenced_from_other_sets(&stores, &hit.scope, hit.memory.id)
                .map_err(|error| {
                    format!("could not check what else points at this memory: {error}")
                })?;
            // Re-resolved for the same reason the claims are filled here: `Store::lookup` sees one
            // store, so a concession naming another set is *not available here* from where it
            // stands, whether or not this server holds that set. Lookup is exempt from substitution
            // -- asking by id means wanting that memory -- so what changes is only what the caller
            // is told: the replacement is named rather than declared out of reach
            // (`superseded-beyond-this-set-is-reported-when-the-set-is-loaded`). This is the surface
            // where that wording does the most work, since the flag is all lookup offers.
            if let Some(home) = stores.iter().position(|store| store.scope() == &hit.scope) {
                hit.validity = resolve_within(&stores, home, hit.memory.id).map_err(|error| {
                    format!("could not resolve this memory's standing: {error}")
                })?;
            }
        }

        let ambiguous = params.fragment.is_some() && hits.len() > 1;
        let memories: Vec<Value> = hits
            .iter()
            .map(|hit| {
                let reviewing = under_review.contains(&hit.scope);
                let mut view = MemoryView::looked_up(hit, reviewing).to_json();
                surface_cross_set_targets(&mut view, reviewing, &hit.memory, &loaded_sets);
                view
            })
            .collect();
        let mut result = json!({"memories": memories});
        note_any_under_review(&mut result, &memories);

        // **An id that names something this set forgot is a different answer from an id it never
        // held**, and until now both came back as an empty list. That gap became load-bearing with
        // `rename`: it forgets the record it superseded, so every memory written before a rename
        // carries a `by` naming a tombstone — and the path this tool's own description sends a
        // reader down would dead-end on exactly the ids it was meant to resolve. What comes back
        // carries no content, because a tombstone has none; it carries where the chain goes.
        if let (Some(_), true) = (&params.id, hits.is_empty()) {
            let asked = MemoryId::parse(params.id.as_deref().unwrap_or_default())
                .map_err(|error| format!("could not read the id: {error}"))?;
            let mut forgotten = Vec::new();
            for (index, store) in stores.iter().enumerate() {
                if let Some(tombstone) = store
                    .tombstone_for(asked)
                    .map_err(|error| format!("lookup failed: {error}"))?
                {
                    let leads_to = match resolve_within(&stores, index, asked)
                        .map_err(|error| format!("could not resolve its standing: {error}"))?
                    {
                        Resolution::Superseded { tips, .. } => {
                            tips.iter().map(ToString::to_string).collect()
                        }
                        _ => Vec::new(),
                    };
                    // **`incoming` as well as `superseded_by`, because they answer different
                    // questions and this tool's own description sends a reader to the first.**
                    // `superseded_by` names the *end* of the chain; walking it one link at a time
                    // means following the `supersedes` edge in `incoming`. Without it, a chain with
                    // a forgotten record in the middle can be jumped over but not walked, so the
                    // record that replaced this one is unreachable going forward -- reachable only
                    // by starting from the far end and reading backwards.
                    let incoming: Vec<Value> = store
                        .incoming_edges(asked)
                        .map_err(|error| format!("could not read what cites it: {error}"))?
                        .iter()
                        .map(|edge| {
                            json!({
                                "label": serde_name(&edge.label),
                                "source": edge.source.to_string(),
                            })
                        })
                        .collect();
                    forgotten.push(json!({
                        "id": asked.to_string(),
                        "scope": WireScope(store.scope()),
                        "forgotten": {
                            "by": tombstone.forgotten.by.to_string(),
                            "at": tombstone.forgotten.at.to_string(),
                        },
                        "incoming": incoming,
                        "superseded_by": leads_to,
                    }));
                }
            }
            if !forgotten.is_empty() {
                result["forgotten"] = json!(forgotten);
                result["note"] = json!(
                    "This id names a memory that was forgotten, not one that was never here. A \
                     tombstone carries no content -- that is what forgetting means -- but it still \
                     says where the chain goes, so a `superseded_by` here is the record that \
                     replaced it and the one to read instead."
                );
            }
        }
        if ambiguous {
            // Named where the caller can act on it. The head of an id is its timestamp, so a
            // fragment taken from the front is the one that matches everything written nearby.
            result["note"] = json!(
                "This fragment matched more than one memory. A fragment taken from the *end* \
                 of an id is far more selective than one from the start: the leading characters \
                 are a timestamp, so memories written close together share them."
            );
        }
        Ok(result)
    }

    /// Dispatch a `hoard_correct` call.
    ///
    /// One tool with an action for the same reason `hoard_sets` is: `forget` is coming, and a tool
    /// each would put the surface over the budget `documents/design/tool-surface.md` argues for.
    fn correct(&mut self, arguments: &Value) -> Result<Value, String> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("");
        if let Some(refusal) = argument_refusal(&correct_branches(), action, arguments) {
            return Err(refusal);
        }
        // **Correcting a memory is using it**, and this is the one place both actions pass through.
        // Amending is the signal for what `recall-when`'s connect-on-recall clause asks for --
        // going back to an existing memory and linking it -- which no other call site can see.
        // Forgetting counts too: the recall surfaced the thing the session then acted on, which is
        // what the label claims and all it claims. A malformed id is left to the branch handler to
        // refuse; nothing is labelled by one, since no log row can carry it.
        let mut used: Vec<MemoryId> = arguments
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| MemoryId::parse(id).ok())
            .into_iter()
            .collect();
        if let Some(links) = arguments.get("links").and_then(Value::as_array) {
            used.extend(
                links
                    .iter()
                    .filter_map(|link| link.get("target"))
                    .filter_map(|target| {
                        target
                            .as_str()
                            .or_else(|| target.get("id").and_then(Value::as_str))
                    })
                    .filter_map(|id| MemoryId::parse(id).ok()),
            );
        }
        self.note_used(&used)?;
        match action {
            "amend" => self.amend(arguments),
            "forget" => self.forget(arguments),
            other => Err(unknown_correction(other)),
        }
    }

    /// Repair a record that never entered service, leaving anything that did to supersession.
    ///
    /// `documents/design/corrections.md`, resolved 2026-08-29: the boundary is **whether the record
    /// was ever believed**, not which of its fields is being changed. A record that has never been
    /// exported to any target exists on one machine and nobody has read it, so there is no belief to
    /// destroy and no second copy for a merge to reconcile — correction may therefore rewrite the
    /// whole of it, `content` and `why` included. Once it has been exported, every change has to
    /// carry ancestry, and the instrument that carries it is supersession.
    ///
    /// **The server decides service, and by a fact.** The per-target bases answer it exactly, with
    /// no new state and no threshold. Asking the caller would be the wrong placement twice over: it
    /// cannot see the bases, and it is the party that benefits from answering *no*.
    ///
    /// This replaced a field list — apparatus amendable, claim refused — which had been redrawn once
    /// already when `origin` joined it (`origin-cannot-be-corrected`, 2026-08-21). The belief
    /// criterion answers that case and the ones the list could not.
    fn amend(&mut self, arguments: &Value) -> Result<Value, String> {
        /// Absent means "leave alone"; present means "replace". `Option<Option<_>>` on the fields
        /// that may be absent from a memory — `why` and `context` — is what distinguishes omitting
        /// the field from clearing it with `null`; without it there would be no way to remove a
        /// value recorded in error, only to overwrite it.
        #[derive(Deserialize)]
        struct Params {
            scope: Scope,
            id: MemoryId,
            /// No `Option<Option<_>>`: a memory always has content, so there is nothing to clear.
            #[serde(default)]
            content: Option<MemoryText>,
            #[serde(default, deserialize_with = "present_even_if_null")]
            why: Option<Option<MemoryText>>,
            #[serde(default)]
            keywords: Option<Vec<Keyword>>,
            #[serde(default)]
            links: Option<Vec<Link>>,
            #[serde(default, deserialize_with = "present_even_if_null")]
            context: Option<Option<MemoryText>>,
            #[serde(default)]
            confidence: Option<Confidence>,
            #[serde(default)]
            origin: Option<Origin>,
        }

        let mut params: Params = parse(arguments)?;

        // The escape reaches every prose field a caller can write, `amend`'s included. Offering it
        // on `hoard_memorize` alone would be an escape a caller has to remember the scope of, which
        // is the same trap as a rule with no sanctioned alternative.
        if let Some(content) = params.content {
            params.content = Some(decoded(content)?);
        }
        if let Some(Some(why)) = params.why {
            params.why = Some(Some(decoded(why)?));
        }
        if let Some(Some(context)) = params.context {
            params.context = Some(Some(decoded(context)?));
        }

        // The same conjunction `hoard_memorize` applies: framing naming a parameter, and that
        // parameter arriving empty. Every prose field is scanned rather than `context` alone,
        // because a mis-closed parameter swallows whatever follows it and correction now carries
        // three of them -- which is also the shape of the damage this widening exists to repair.
        let swallowed = swallowed_parameters(
            &[
                params.content.as_ref().map_or("", MemoryText::as_data),
                params
                    .why
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map_or("", MemoryText::as_data),
                params
                    .context
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map_or("", MemoryText::as_data),
            ],
            &|field| match field {
                "why" => params.why.is_none(),
                "context" => params.context.is_none(),
                "keywords" => params.keywords.is_none(),
                "links" => params.links.is_none(),
                "confidence" => params.confidence.is_none(),
                _ => false,
            },
        );
        if !swallowed.is_empty() {
            return Err(format!(
                "This call looks damaged, so nothing was amended. The text carries tool-call \
                 framing naming {}, which arrived empty -- together that means a parameter was \
                 closed with the wrong tag and everything after it was absorbed. Send it again \
                 with each parameter closed properly, or escape the markup as &lt; &gt; and &amp; \
                 if you meant to write about it.",
                swallowed.join(" and ")
            ));
        }

        if params.content.is_none()
            && params.why.is_none()
            && params.keywords.is_none()
            && params.links.is_none()
            && params.context.is_none()
            && params.confidence.is_none()
            && params.origin.is_none()
        {
            return Err(
                "this amendment would change nothing. Give at least one of `content`, `why`, \
                        `origin`, `keywords`, `links`, `context` or `confidence`."
                    .to_owned(),
            );
        }

        // The identical check `hoard_memorize` runs, and it must be identical rather than lighter:
        // amend replaces the whole link array, so anything the write door refuses is reachable
        // through this one otherwise.
        let holder = self
            .stores
            .get(&params.scope)
            .map(Store::set_id)
            .transpose()
            .map_err(|error| format!("could not read the set's identity: {error}"))?
            .flatten();
        if let Some(links) = &params.links
            && let Some(refusal) = cross_set_authority_refusal(links, &params.scope, holder)
        {
            return Err(refusal);
        }

        // Taken before the mutable borrow, because the attribution below needs the store's author
        // while the store is held.
        let author = self.author_for(&params.scope)?;
        // The same door as `hoard_memorize`, and now over the same three fields: prose a caller
        // writes is scanned on the same terms wherever it arrives. Correction reaching `content`
        // would otherwise be a way in for a credential that the write door refuses.
        if let Some(refusal) = prohibited_data_refusal(&[
            (
                "content",
                params.content.as_ref().map_or("", MemoryText::as_data),
            ),
            (
                "why",
                params
                    .why
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map_or("", MemoryText::as_data),
            ),
            (
                "context",
                params
                    .context
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map_or("", MemoryText::as_data),
            ),
        ]) {
            return Err(refusal);
        }
        // The never-store list on the same terms as `hoard_memorize`, or an amendment would be the
        // way to put back a word the write door refused.
        let mut written = vec![
            params.content.as_ref().map_or("", MemoryText::as_data),
            params
                .why
                .as_ref()
                .and_then(Option::as_ref)
                .map_or("", MemoryText::as_data),
            params
                .context
                .as_ref()
                .and_then(Option::as_ref)
                .map_or("", MemoryText::as_data),
        ];
        written.extend(params.keywords.iter().flatten().map(Keyword::as_str));
        if let Some(refusal) = self.never_store_refusal(&written, "Nothing was changed") {
            return Err(refusal);
        }

        // Same reason, and the same check `hoard_memorize` runs: amend replaces the whole link
        // array rather than adding to it, so an unresolvable target arrives here exactly as it
        // would there -- including when the amendment is itself the repair for one.
        let unresolved = match &params.links {
            Some(links) => self.unresolved_links(&params.scope, links)?,
            None => Vec::new(),
        };
        let store = self.store_for_mut(&params.scope)?;
        let Some(mut memory) = store
            .get(params.id)
            .map_err(|error| format!("could not read the memory: {error}"))?
        else {
            return Err(format!(
                "no memory {} in this set. `hoard_correct` takes a whole id, which is 26 \
                 characters. If what you have is part of one, `hoard_lookup` takes a `fragment` -- \
                 and prefer a piece from the *end*, which identifies a memory far more sharply \
                 than one from the start.",
                params.id
            ));
        };

        // **The gate.** Asked of this record rather than of the set, because a memory written after
        // an export has never left the machine even though its neighbours have. Placed after the
        // memory is fetched so a mistyped id is answered as a mistyped id, and before anything is
        // written so a refusal leaves nothing behind.
        //
        // **It is worded to route, not to deny, and that was a repair.** Until 2026-09-09 it read
        // "can no longer be corrected ... **Supersede** it instead ... correction is for a record
        // that never entered service" -- three denials in one message, and it did measurable
        // damage. A session came to regard supersession as an expensive fallback and filed a design
        // question against a cost the project nowhere claims: no convention calls it costly,
        // `correct-when` calls keeping the old memory readable "the point",
        // `LinkLabel::Supersedes` means only "replaces it, which is kept as history", chains run to
        // a mean depth of 1.30, and the same session had performed eight supersessions that day
        // without hesitating. The belief came from the tone of this string.
        //
        // The mechanism did not change and must not: the gate is a fact about the record, and both
        // operations are correct for their own case. What changed is that the message now says
        // which case this is. `the_export_gate_names_the_instrument_rather_than_refusing` in
        // `tests/correct.rs` guards it, because prose about tone does not survive an edit.
        if store.has_base(params.id).map_err(|error| {
            format!("could not read whether this memory has been exported: {error}")
        })? {
            return Err(format!(
                "memory {} has entered service -- a second durable copy of it exists -- so the \
                 instrument here is supersession. Store the corrected memory with a `supersedes` \
                 link to this one: that records what changed, keeps this version readable as what \
                 was believed, and gives a later merge the ancestry it needs to reason about. \
                 Amendment is the instrument for a record nobody else holds yet; both are ordinary, \
                 and which one applies is a fact about the record rather than a judgement.",
                params.id
            ));
        }

        // After the gate, so a memory in service is routed to supersession -- which restates on
        // the same terms -- rather than told about a label. Only this memory's own pairs may be
        // restated: amend replaces its whole link array, which is the trap supersession has too.
        if let Some(links) = &params.links
            && let Some(refusal) = unknown_label_refusal(links, &memory.links)
        {
            return Err(refusal);
        }

        let mut changed = Vec::new();
        if let Some(content) = params.content {
            memory.content = content;
            changed.push("content");
        }
        if let Some(why) = params.why {
            memory.why = why;
            changed.push("why");
        }
        if let Some(keywords) = params.keywords {
            memory.keywords = keywords;
            changed.push("keywords");
        }
        if let Some(links) = params.links {
            memory.links = links;
            changed.push("links");
        }
        if let Some(context) = params.context {
            memory.context = context;
            changed.push("context");
        }
        if let Some(confidence) = params.confidence {
            memory.confidence = confidence;
            changed.push("confidence");
        }
        if let Some(origin) = params.origin {
            memory.origin = origin;
            changed.push("origin");
        }

        // **Measured on the memory as it would stand, not on what arrived.** An amendment replaces
        // some fields and leaves the rest, so neither the old text nor the new one is the length
        // that matters -- and a door that skipped this would not be a lenient door, it would be the
        // way around the other one. Before the write, so a refusal leaves the memory as it was.
        if let Some(refusal) = over_length_refusal(
            memory.content.as_data(),
            memory.why.as_ref().map(MemoryText::as_data),
            &memory.keywords,
        ) {
            return Err(refusal);
        }

        // The record of the act, since the previous value is not kept. Also what the export review
        // gate reads: an amendment makes the local user the memory's last modifier.
        memory.modified = Attribution {
            by: author,
            at: jiff::Timestamp::now(),
        };

        // Through `insert`, which replaces the row, replaces its links, re-runs the supersession
        // cycle check, and re-embeds. Re-embedding matters: keywords are part of the vectorised text,
        // so a corrected keyword that did not re-embed would leave the memory findable by the
        // spelling it no longer has.
        store
            .insert(&memory)
            .map_err(|error| format!("could not amend the memory: {error}"))?;

        let mut result = json!({
            "id": params.id.to_string(),
            "amended": changed,
            // Said here for the reason `memorize-says-nothing-about-searchability` was filed: this
            // call rewrote a memory's vector, so whether the result is still findable by meaning is a
            // fact about this write and worth stating rather than leaving to be discovered.
            "semantic_search_available": store
                .can_search_by_meaning()
                .map_err(|error| format!("could not read the model state: {error}"))?,
        });
        // The line this memory now shows as in an index, on the one write that can still change it.
        //
        // **`hoard_memorize` echoes the first draft and this echoes every one after**, which is the
        // shape the field is actually used in: the opening is what an index shows, a 120-character
        // cap is invisible while composing, and getting an opening to say what the memory is for
        // takes a try or two. Amend is the right instrument for that loop rather than a second
        // thought about it -- the loop runs before export, which is exactly the window amend is
        // open in, and after export supersession carries the same echo through `hoard_memorize`.
        //
        // **Only when the content moved**, on the same rule as the in-link remark below: the line
        // is derived from `content` alone, so echoing it after a confidence change would report
        // something that did not happen and teach a caller to skip the field on the writes where it
        // did.
        if changed.contains(&"content") {
            result["summary"] = json!(memory.content.summary_as_data());
        }
        // **Only when the claim moved.** Apparatus correction cannot invalidate a label: the text
        // the edge's author judged has not moved. Unscoped, this would fire on every confidence
        // change and teach callers to skip it, which costs the cases it exists for.
        if changed
            .iter()
            .any(|field| matches!(*field, "content" | "why"))
        {
            let judged: Vec<IncomingEdge> = store
                .incoming_edges(params.id)
                .map_err(|error| format!("amended, but could not read what points at it: {error}"))?
                .into_iter()
                // `supersedes` is the one label that says nothing about the text: it records that
                // this memory was replaced, which stays true however the replaced version reads.
                .filter(|edge| edge.label != LinkLabel::Supersedes)
                .collect();
            if !judged.is_empty() {
                result["in_links"] = serde_json::to_value(&judged).unwrap_or(Value::Null);
                // A remark, never a refusal -- the rule this codebase already applies is *remark
                // when the caller can still act*, and here they can, in the same turn, holding both
                // texts. A refusal would also fire most often on `related_to`, which dominates live
                // writing and asserts least about content.
                result["in_links_note"] = json!(
                    "Amended. These memories point at this one, and each was written against the \
                     version you just replaced -- so a `supports`, `instance_of` or `contradicts` \
                     may no longer hold. Only you can tell: you have both texts in front of you \
                     and their authors are not here. Nothing was refused and nothing needs doing \
                     unless a label has stopped being true; where one has, amend the *source* \
                     memory's links, or supersede that memory if its own claim went with it. \
                     `supersedes` edges are left out, because they say this memory was replaced, \
                     which stays true however it reads."
                );
            }
        }

        report_unresolved(&mut result, &unresolved);
        Ok(result)
    }

    /// Remove a memory from the set, leaving a tombstone.
    ///
    /// `documents/design/corrections.md`. Forgetting is not a correction of a claim but a removal of
    /// a record, and it is what to reach for in two cases: sensitive data reached a memory, or a
    /// memory characterises a person unfairly — the second because superseding it would keep the
    /// characterisation permanently readable, which is the durability hazard
    /// `documents/design/golden-rule.md` names.
    fn forget(&mut self, arguments: &Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Params {
            scope: Scope,
            id: MemoryId,
        }

        let params: Params = parse(arguments)?;
        let author = self.author_for(&params.scope)?;
        let now = jiff::Timestamp::now();

        let store = self.store_for_mut(&params.scope)?;
        let existed = store
            .get(params.id)
            .map_err(|error| format!("could not read the memory: {error}"))?
            .is_some();

        store
            .forget(params.id, author, now)
            .map_err(|error| format!("could not forget the memory: {error}"))?;

        let mut result = json!({
            "forgotten": params.id.to_string(),
            // Stated on every call rather than only when it matters, because the caller who most
            // needs to read it is the one who just removed a secret and may believe the job is done.
            "note": "The memory is gone from this set going forward, and a tombstone records that it \
                     was forgotten so the removal reaches other copies on the next export. This does \
                     not remove it from the history of any repository that already carries it: a \
                     secret that reached a shared set must be treated as compromised and rotated.",
        });
        if !existed {
            // Not an error: the outcome asked for is already the case. Worth saying, because a caller
            // removing sensitive data needs to know whether it found anything to remove.
            result["already_absent"] = json!(true);
        }
        Ok(result)
    }

    /// Dispatch a `hoard_sets` call.
    ///
    /// The match below is the authority on which actions exist; the published schema is checked
    /// against it by `tests/sets.rs`, so an action advertised and not implemented is a test failure
    /// rather than a runtime surprise.
    fn sets(&mut self, arguments: &Value) -> Result<Value, String> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("");
        if let Some(refusal) = argument_refusal(
            &sets_branches_for(self.exchange.protocol),
            action,
            arguments,
        ) {
            return Err(refusal);
        }
        match action {
            "status" => self.set_status(),
            "create" => {
                let request: CreateSet = parse(arguments)?;
                self.create_set(
                    &request.name,
                    request.mode.map_or(LoadMode::ReadWrite, LoadMode::from),
                )
            }
            "export" => {
                let request: ExportSet = parse(arguments)?;
                self.export_set(&request.scope, &request.destination)
            }
            "evaluate" => {
                let request: EvaluateSet = parse(arguments)?;
                self.evaluate_set(&request.label, &request.source, request.set.as_ref())
            }
            "import" => {
                let request: ImportSet = parse(arguments)?;
                self.import_set(&request.scope, &request.destination)
            }
            "restore" => {
                let request: RestoreSet = parse(arguments)?;
                self.restore_set(&request.scope, &request.source, &request.save_to)
            }
            "absorb" => {
                let request: AbsorbSet = parse(arguments)?;
                self.refuse_if_read_only(&request.scope)?;
                self.absorb_set(
                    &request.scope,
                    &request.source,
                    request.set.as_ref(),
                    &request.save_to,
                )
            }
            "drop" => {
                let request: DropSet = parse(arguments)?;
                self.drop_set(&request.name, request.save_to.as_deref())
            }
            "review" => {
                let request: ReviewRequest = parse(arguments)?;
                let destination = request.destination()?;
                self.review_set(
                    &request.scope,
                    destination,
                    request.full,
                    request.recheck.as_deref(),
                )
            }
            "backup" => {
                let request: BackupRequest = parse(arguments)?;
                self.backup_set(&request.scope)
            }
            "clear" => {
                let request: ClearRequest = parse(arguments)?;
                self.clear_reviewed(&request.scope, &request.answers, &request.person_cleared)
            }
            "reembed" => {
                let request: ReembedSet = parse(arguments)?;
                self.reembed_set(&request.scope)
            }
            "calibrate" => {
                let request: CalibrateSet = parse(arguments)?;
                self.calibrate_set(&request.scope)
            }
            "uncalibrate" => {
                let request: CalibrateSet = parse(arguments)?;
                self.uncalibrate_set(&request.scope)
            }
            // **Where the human can be asked, these three ask, and a name or record the model
            // supplied is set aside**: the tool list cannot say whether a client shows forms, since
            // that is declared per request, so the argument stays in the schema and is simply not
            // the source here. Each refuses what it would refuse anyway *before* asking, so nobody
            // fills in a form for nothing. `documents/design/asking-the-human.md`.
            "mint" => {
                let request: MintAuthor = parse(arguments)?;
                self.refuse_if_read_only(&request.scope)?;
                if self.asks_about(&request.scope) {
                    if let Some(held) = self
                        .store_for_mut(&request.scope)?
                        .author_record()
                        .map_err(|error| error.to_string())?
                    {
                        return Err(crate::store::StoreError::AlreadyIdentified {
                            record: held.to_string(),
                        }
                        .to_string());
                    }
                    // **An unanswered form falls back to the model's name**, decided 2026-10-16: a
                    // client that cannot show forms answers them unseen, and the server cannot tell
                    // that from the human refusing. The name is relayed back, as on a handshake
                    // connection. A form the human did answer always wins.
                    if let Some(author) = request
                        .author
                        .as_ref()
                        .filter(|_| self.went_unanswered(NAME_QUESTION))
                    {
                        return self.mint_author(&request.scope, author, Source::Model);
                    }
                    let author = self.name_from_human(&request.scope, NamePurpose::First)?;
                    self.mint_author(&request.scope, &author, Source::Human)
                } else {
                    let author = request
                        .author
                        .ok_or_else(|| model_must_name("mint", "author"))?;
                    self.mint_author(&request.scope, &author, Source::Model)
                }
            }
            "identify" => {
                let request: IdentifyAuthor = parse(arguments)?;
                self.refuse_if_read_only(&request.scope)?;
                let unanswered = self.went_unanswered(IDENTITY_QUESTION);
                let record = if let Some(record) = request
                    .record
                    .filter(|_| self.asks_about(&request.scope) && unanswered)
                {
                    record
                } else if self.asks_about(&request.scope) {
                    match self.identity_from_human(&request.scope, false)? {
                        Chosen::Record(record) => record,
                        Chosen::New(_) => {
                            return Err("a newcomer is minted, not identified".to_owned());
                        }
                    }
                } else {
                    request
                        .record
                        .ok_or_else(|| model_must_name("identify", "record"))?
                };
                self.identify_author(&request.scope, record)
            }
            "rename" => {
                let request: RenameAuthor = parse(arguments)?;
                self.refuse_if_read_only(&request.scope)?;
                if self.asks_about(&request.scope) {
                    if self
                        .store_for_mut(&request.scope)?
                        .author_record()
                        .map_err(|error| error.to_string())?
                        .is_none()
                    {
                        return Err(crate::store::StoreError::NotIdentifiedHere.to_string());
                    }
                    if let Some(author) = request
                        .author
                        .as_ref()
                        .filter(|_| self.went_unanswered(NAME_QUESTION))
                    {
                        return self.rename_author(&request.scope, author, Source::Model);
                    }
                    let author = self.name_from_human(&request.scope, NamePurpose::Rename)?;
                    self.rename_author(&request.scope, &author, Source::Human)
                } else {
                    let author = request
                        .author
                        .ok_or_else(|| model_must_name("rename", "author"))?;
                    self.rename_author(&request.scope, &author, Source::Model)
                }
            }
            other => Err(unknown_action(other)),
        }
    }

    /// Set a set's non-match floor from its own query log.
    ///
    /// **Explicit rather than automatic, in both directions.** Raising a floor is the direction
    /// that loses answers, so it should be an act with a trace. Lowering was automatic until
    /// 2026-09-12 — recall wrote the floor down to a scoped query that scored under it — and that
    /// made it a ratchet one unanswerable question destroyed for good. A contradicting query is now
    /// evidence this reads next time, and the note says so: it had gone on describing the removed
    /// rule (`the-calibrate-note-promises-a-floor-that-lowers-itself`).
    fn calibrate_set(&mut self, scope: &Scope) -> Result<Value, String> {
        let store = self.store_for_mut(scope)?;
        let previous = store
            .floor()
            .map_err(|error| format!("could not read the set's floor: {error}"))?;
        let calibration = store
            .calibrate()
            .map_err(|error| format!("could not calibrate: {error}"))?;

        Ok(match calibration {
            Calibration::Set { floor, labelled } => json!({
                "scope": WireScope(scope),
                "floor": floor,
                "previous_floor": previous,
                "labelled_queries": labelled,
                "note": "Recall now returns nothing from this set for a query no memory reaches \
                         this closely, which is how *nothing here answers that* is said. The level \
                         sits a margin below nearly every question the set has been seen to \
                         answer, and belongs to this set and this embedding model -- changing the \
                         model clears it. Nothing else moves it: a query naming this set that \
                         scores under it is still silenced, and counts as evidence at the next \
                         calibration, which can lower the level or raise it. `ignore_floor` asks a \
                         silenced question again, and `uncalibrate` removes the level.",
            }),
            Calibration::FromItsOwnMemories { floor, memories } => json!({
                "scope": WireScope(scope),
                "floor": floor,
                "previous_floor": previous,
                "floor_source": FloorSource::ItsOwnMemories.as_str(),
                "memories_asked": memories,
                // **Says what the evidence is, because it is weaker than labels.** A reader of this
                // response or of `status` should be able to tell a level observed from real answers
                // from one inferred from the set's contents. `calibration-from-self-queries`.
                "note": "Too few labelled queries have arrived to read a floor from, so it was \
                         placed from this set's own memories instead: each current memory's first \
                         line asked as a question against the rest, and the floor put a margin \
                         below nearly all of what they scored. That is weaker evidence than queries \
                         the set was seen to answer, and it gives way to them -- once enough \
                         labelled queries arrive, calibrating again reads those instead. Recall now \
                         returns nothing from this set for a query no memory reaches this closely. \
                         `ignore_floor` asks a silenced question again, and `uncalibrate` removes \
                         the level.",
            }),
            Calibration::NotEnoughEvidence { labelled } => json!({
                "scope": WireScope(scope),
                "floor": previous,
                "labelled_queries": labelled,
                "note": "No floor was set, and any existing one is untouched. A floor is read off \
                         the queries this set has been seen to answer -- ones that named it as \
                         their scope, or returned a memory the session then used -- and there are \
                         not yet enough to place one. Set from too few, a floor sits too high and \
                         silences real questions, which is the one failure it cannot report: a \
                         silenced query returns nothing, so nothing can be used, so nothing \
                         contradicts it.",
            }),
        })
    }

    /// Remove a set's floor, so recall returns what it ranks again.
    fn uncalibrate_set(&mut self, scope: &Scope) -> Result<Value, String> {
        let store = self.store_for_mut(scope)?;
        let had = store
            .uncalibrate()
            .map_err(|error| format!("could not clear the floor: {error}"))?;
        Ok(json!({
            "scope": WireScope(scope),
            "cleared": had,
            "note": match had {
                Some(_) => "The floor is gone and recall returns whatever it ranks, however weakly \
                            it matched. Nothing else changed: the queries this set has answered are \
                            still recorded, so calibrating again will set a level from them.",
                None => "This set had no floor, so nothing changed.",
            },
        }))
    }

    /// How well this server's model fits each language `store` declares: a score for every
    /// language there are sentences for, the band message for each one below the top band, and
    /// what the set's own sentences need doing. `language_fit` says what the score is and why the
    /// bands fall where they do.
    ///
    /// **A set's own ten sets for a language take precedence over the built-ins**, so a set can
    /// measure its own vocabulary. They are matched by language and script, ignoring region; a
    /// declaration matching two scripts is left unscored and named rather than guessed at.
    ///
    /// **Nothing where the set cannot be searched by meaning at all**: with no model, or one the
    /// set's vectors were not made by, a score would blame the language for what
    /// `semantic_search_available` and `embedding_model` already report. A language with no
    /// sentences is left unscored rather than reported as zero, which would be a claim about the
    /// model made with no evidence.
    fn language_fit(&self, store: &Store, tags: &[String]) -> Result<Fit, String> {
        let mut fit = Fit::default();
        let Some(embedder) = &self.embedder else {
            return Ok(fit);
        };
        if !store
            .can_search_by_meaning()
            .map_err(|error| format!("could not read the model state: {error}"))?
        {
            return Ok(fit);
        }
        let material = store
            .fit_material()
            .map_err(|error| format!("could not read this set's model-fit hub: {error}"))?;
        let mut matched = HashSet::new();
        let mut known = self
            .language_scores
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut score_of =
            |key: String, name: &str, triplets: &[[&str; 3]]| -> Result<u8, String> {
                if let Some(score) = known.get(&key) {
                    return Ok(*score);
                }
                let score = language_fit::score(embedder.as_ref(), triplets).map_err(|error| {
                    format!("could not score {name} against the model: {error}")
                })?;
                known.insert(key, score);
                Ok(score)
            };
        for tag in tags {
            let (language, script) = language_fit::language_and_script(tag);
            let own: Vec<&String> = material
                .groups
                .keys()
                .filter(|key| {
                    let (theirs, their_script) = language_fit::language_and_script(key);
                    theirs == language
                        && language_fit::scripts_agree(script.as_deref(), their_script.as_deref())
                })
                .collect();
            matched.extend(own.iter().map(|key| (*key).clone()));
            let built_in = language_fit::built_in(tag);
            let name =
                built_in.map_or_else(|| format!("`{tag}`"), |sentences| sentences.name.to_owned());
            // **What the band message is about**: the language, from the built-ins, or the set's
            // use of it, from its own sentences. The same model read everyday English at 10 and an
            // English set's Italian kitchen terms at 1 (2026-10-04), so a note saying it cannot
            // search English would send the user after a model that does not help.
            let mut about = name.clone();
            let score = match own.as_slice() {
                [] => match built_in {
                    Some(sentences) => score_of(
                        format!("built-in:{}", sentences.tag),
                        &name,
                        &sentences.triplets,
                    )?,
                    None => {
                        fit.setup.push(sentences_wanted(tag));
                        continue;
                    }
                },
                [key] => {
                    let sets = &material.groups[*key];
                    let triplets: Vec<[String; 3]> = sets
                        .iter()
                        .filter_map(|set| language_fit::triplet_in(set.content.as_data()).ok())
                        .collect();
                    if triplets.len() != language_fit::TRIPLETS_PER_LANGUAGE {
                        fit.setup.push(format!(
                            "This set's hub lists {} readable sentence sets for `{key}`, and a \
                             language is scored from exactly {}, so {name} is not scored. Store \
                             sets until there are {} -- each a memory carrying \
                             `hoard-fit-sentences` and `hoard-lang-{key}` -- and link them from \
                             the `hoard-fit-hub` memory with `refers_to` links.",
                            triplets.len(),
                            language_fit::TRIPLETS_PER_LANGUAGE,
                            language_fit::TRIPLETS_PER_LANGUAGE,
                        ));
                        continue;
                    }
                    let borrowed: Vec<[&str; 3]> = triplets
                        .iter()
                        .map(|[a, p, u]| [a.as_str(), p.as_str(), u.as_str()])
                        .collect();
                    let key = format!("own:{}", triplets.concat().join("\u{1f}"));
                    about = format!("{name} as this set writes it");
                    score_of(key, &name, &borrowed)?
                }
                several => {
                    let named: Vec<String> = several.iter().map(|key| format!("`{key}`")).collect();
                    fit.setup.push(format!(
                        "This set declares `{tag}`, and its hub lists sentence sets for {}, which \
                         are different scripts of it -- different text to a model -- so `{tag}` is \
                         not scored rather than scored from a guess. Declare each script on the \
                         charter instead, such as `hoard-lang-{}`.",
                        named.join(" and "),
                        several[0]
                    ));
                    continue;
                }
            };
            fit.scores.insert(tag.clone(), json!(score));
            let band = Band::of(score);
            fit.bands.extend(band.note(&about));
            if band != Band::Searchable {
                // Tagged for this language, matched as sentences are, or untagged and so about the
                // whole set.
                let named: Vec<String> = material
                    .recommendations
                    .iter()
                    .filter(|recommendation| {
                        recommendation.tags.is_empty()
                            || recommendation.tags.iter().any(|theirs| {
                                let (their_language, their_script) =
                                    language_fit::language_and_script(theirs);
                                their_language == language
                                    && language_fit::scripts_agree(
                                        script.as_deref(),
                                        their_script.as_deref(),
                                    )
                            })
                    })
                    .map(|recommendation| {
                        format!("{} (`{}`)", recommendation.summary, recommendation.id)
                    })
                    .collect();
                if !named.is_empty() {
                    fit.advice.push(format!(
                        "For {name}, this set recommends: {}",
                        named.join(" ")
                    ));
                }
            }
        }
        if !fit.bands.is_empty() {
            fit.advice.push(fit_how_to(&material.hubs));
        }
        for key in material.groups.keys().filter(|key| !matched.contains(*key)) {
            fit.setup.push(format!(
                "This set's hub lists sentence sets for `{key}`, which its charter does not \
                 declare, so they score nothing. Declare `hoard-lang-{key}` on the charter if the \
                 set is written in it."
            ));
        }
        if !material.orphans.is_empty() {
            let ids: Vec<String> = material.orphans.iter().map(ToString::to_string).collect();
            fit.setup.push(format!(
                "Sentence sets no `hoard-fit-hub` memory lists, so they score nothing: {}. List \
                 them from the hub, or supersede them if they are no longer wanted.",
                ids.join(", ")
            ));
        }
        Ok(fit)
    }

    /// Every loaded set and how much it holds.
    ///
    /// This is the answer `no-tool-lists-loaded-scopes` needed: a caller told a scope is not loaded
    /// now has somewhere to look.
    ///
    /// **No dirty flag yet, deliberately.** `documents/design/set-lifecycle.md` specifies one, and it
    /// depends on the per-record reconciliation base in `documents/design/multi-set-sync.md`, which
    /// is not built. Reporting `dirty: false` from state nothing maintains would be a field that is
    /// always wrong in the reassuring direction — the exact shape of defect this project keeps
    /// filing. It arrives with the base.
    ///
    /// **`semantic_search_available` is reported per set, and asked of the store.** The first version
    /// of this reported it once for the whole server from `self.embedder`, and an approval snapshot
    /// caught it disagreeing with `hoard_recall` on the very same server — recall asks the stores,
    /// which is authoritative, and a server can hold a model that a store was never given. A
    /// server-wide flag is also the shape that lied in `project-stores-get-no-embedder`: it cannot
    /// express the one situation worth reporting, which is *this* set being unsearchable while
    /// others are fine.
    fn set_status(&self) -> Result<Value, String> {
        let mut sets = Vec::with_capacity(self.stores.len());
        for store in self.stores.values() {
            let memories = store
                .count()
                .map_err(|error| format!("could not count {:?}: {error}", store.scope()))?;
            let state = store
                .model_state()
                .map_err(|error| format!("could not read the model state: {error}"))?;
            let author = store.author_display().map_err(|error| {
                format!("could not read who this set attributes writes to: {error}")
            })?;

            // Reported beside `memories` on every set, present even at zero. An absent field
            // would leave a reader working out whether it means none or means not-reported, which
            // is `memories-count-means-two-different-things` one layer down.
            let forgotten = store
                .forgotten_count()
                .map_err(|error| format!("could not count {:?}: {error}", store.scope()))?;
            let labelled = store
                .labels_since_calibration()
                .map_err(|error| format!("could not read the set's query log: {error}"))?;
            let mode = store
                .load_mode()
                .map_err(|error| format!("could not read the set's load mode: {error}"))?;
            let mut entry = json!({
                "scope": WireScope(store.scope()),
                "memories": memories,
                "forgotten": forgotten,
                "semantic_search_available": store
                    .can_search_by_meaning()
                    .map_err(|error| format!("could not read the model state: {error}"))?,
                "embedding_model": describe_model_state(&state),
                "author": author.as_ref().map(Author::as_str),
                // Which file this set is actually being read from. A fact rather than a check, and
                // the only thing that distinguishes an empty set from a server pointed at the wrong
                // directory -- see `SCATTERHOARD_LOCAL` in
                // `documents/design/store-registry.md`. Every other symptom of that misconfiguration
                // is indistinguishable from a set that is legitimately empty.
                "file": store.file().map(|path| path.to_string_lossy()),
                // **Both halves of the calibration state, because neither can be asked for without
                // changing it**: `calibrate` sets a floor and `uncalibrate` removes one. The floor
                // decides whether recall answers nothing rather than its best guess, which is the one
                // outcome a caller cannot tell from a set that holds nothing on the subject.
                "non_match_floor": store
                    .floor()
                    .map_err(|error| format!("could not read the set's floor: {error}"))?,
                "labelled_since_calibration": labelled,
                // What the floor was read from, since a floor placed from the set's own memories is
                // weaker evidence than one read off labelled queries.
                // `calibration-from-self-queries`.
                "floor_source": store
                    .floor_source()
                    .map_err(|error| format!("could not read the floor's source: {error}"))?
                    .map(FloorSource::as_str),
                // Reported on every set so a session can tell which sets are writable before it
                // tries, which is what `recall-when`'s *whenever a writable set is loaded* reads.
                "mode": mode.as_str(),
                // **Which marks to recall for this set, rather than the session working it out.**
                // The convention says to recall what this lists; the mapping from mode to marks is
                // `LoadMode::watch_marks`, so a mode added later needs no convention text.
                "watch_marks": mode.watch_marks(),
                // Absent for a set created before identities existed, and that absence is
                // meaningful rather than missing data: it says *not yet minted*, which is the state
                // an import from a file carrying one resolves. Reported because the hard cut needs
                // a human to be able to read an identity and compare two.
                "set_id": store
                    .set_id()
                    .map_err(|error| format!("could not read the set identity: {error}"))?
                    .map(|id| id.to_string()),
            });
            // Every set, `local` included: `local` is never reviewed, but it is recalled from, and
            // it is never made through `create`, so nothing else would prompt its declaration. A
            // note rather than a refusal, and it clears itself once the charter declares a language.
            // `charter-declares-set-languages`.
            // **Whatever the scores, and with no model loaded**: a set's authors pass a
            // recommendation on so a new machine can be set up right, and with no model is when
            // knowing which to install matters most.
            let recommended = store
                .fit_material()
                .map_err(|error| format!("could not read this set's model-fit hub: {error}"))?
                .recommendations;
            if !recommended.is_empty() {
                entry["recommended_models"] = recommended
                    .iter()
                    .map(|recommendation| {
                        json!({
                            "id": recommendation.id.to_string(),
                            "summary": recommendation.summary,
                            "languages": recommendation.tags,
                        })
                    })
                    .collect();
            }
            let declared = store.declared_languages().map_err(|error| {
                format!("could not read what this set's charter declares: {error}")
            })?;
            entry["languages"] = json!(declared.clone().unwrap_or_default());
            match declared {
                None => {
                    entry["language_note"] = json!(
                        "This set has no `hoard-scope-topics` charter, so it declares neither what \
                         it is for nor what language it is written in. Write one, with a \
                         `hoard-lang-` keyword for each language its memories are written in, such \
                         as `hoard-lang-en`."
                    );
                }
                Some(tags) if tags.is_empty() => {
                    entry["language_note"] = json!(
                        "This set's charter declares no language. Supersede it with a `hoard-lang-` \
                         keyword added for each language its memories are written in, such as \
                         `hoard-lang-en`. Sessions write memories and phrase queries in a declared \
                         language, because the loaded model may not represent others: a memory in \
                         one it lacks can be stored and never found by meaning."
                    );
                }
                Some(tags) => {
                    let fit = self.language_fit(store, &tags)?;
                    if !fit.scores.is_empty() {
                        entry["language_scores"] = Value::Object(fit.scores);
                    }
                    let notes: Vec<String> = fit
                        .bands
                        .into_iter()
                        .chain(fit.advice)
                        .chain(fit.setup)
                        .collect();
                    if !notes.is_empty() {
                        entry["language_note"] = json!(notes.join(" "));
                    }
                }
            }

            // **Offered, never taken.** Raising a floor is the direction that loses answers, so it
            // stays an act with a trace -- and a session that is never told the evidence has arrived
            // will not perform it. `scatterhoard` sat 6 labels past the threshold, uncalibrated,
            // with nothing saying so.
            let floor = store
                .floor()
                .map_err(|error| format!("could not read the set's floor: {error}"))?;
            if floor.is_none()
                && labelled < LABELS_PER_CALIBRATION
                && memories >= LABELS_PER_CALIBRATION
            {
                // **The backup is offered too**, or a set too little used to gather labels would
                // never be told it can have a floor at all. Worded with *if*, because `memories`
                // counts records and the backup counts current claims, which can be fewer.
                entry["calibration_note"] = json!(format!(
                    "This set has no floor and {labelled} labelled queries, fewer than the \
                     {LABELS_PER_CALIBRATION} a floor is read from. If it holds at least \
                     {LABELS_PER_CALIBRATION} current memories, `hoard_sets` action \"calibrate\" \
                     would place one from the set's own memories instead, which gives way to \
                     labelled queries once enough arrive. Without one, recall returns this set's \
                     best guess for questions it cannot answer, and on a question no set with a \
                     floor answers, that guess is the whole reply."
                ));
            }
            if labelled >= LABELS_PER_CALIBRATION {
                entry["calibration_note"] = json!(format!(
                    "This set has {labelled} labelled queries since its last calibration, past the \
                     {LABELS_PER_CALIBRATION} a calibration reads. `hoard_sets` action \"calibrate\" \
                     would set a floor from them, below which recall answers nothing for this set \
                     rather than returning its best guess. It is offered rather than done, because \
                     raising a floor is the direction that loses answers."
                ));
            }

            let scope = store.scope().clone();
            let must_ask = self.must_ask_who_you_are(&scope)?;
            let claimed = store
                .author_record()
                .map_err(|error| error.to_string())?
                .is_some()
                || store
                    .holds_any_author_record()
                    .map_err(|error| error.to_string())?;
            let default = self.default_author(&scope)?;
            let asks = self.asks_about(&scope);
            if let Some(note) = describe_author(claimed, default.as_ref(), memories, must_ask, asks)
            {
                entry["author_note"] = json!(note);
            }
            sets.push(entry);
        }

        let mut result = json!({"sets": sets});

        if !self.withheld.is_empty() {
            result["withheld"] = self
                .withheld
                .values()
                .map(|store| {
                    json!({
                        "scope": WireScope(store.scope()),
                        "file": store.file().map(|path| path.to_string_lossy()),
                        "set_id": store.set_id().ok().flatten().map(|id| id.to_string()),
                        "memories": store.count().ok(),
                        "note": "Not loaded: another set here answers to the same set identity, and \
                                 nothing says which is the original. Neither is searched or written \
                                 until one is dropped -- `drop` writes a safety copy first -- and \
                                 then the other loads.",
                    })
                })
                .collect();
        }

        // Facts about the *layout* rather than about any one set, and the only place they can be
        // said: nothing about a loaded store reveals that a second copy of it lost, or that a
        // directory was named to no effect. Both are reported and neither is acted on --
        // `documents/design/store-registry.md`.
        if let Some(layout) = &self.layout {
            let discovery = layout.discover();
            if !discovery.shadowed.is_empty() {
                result["shadowed"] = discovery
                    .shadowed
                    .iter()
                    .map(|(scope, path)| {
                        json!({
                            "scope": WireScope(scope),
                            "file": path.to_string_lossy(),
                            "note": "A store for this set exists in both directories. The one in the \
                                     directory shared between checkouts is the one in use; this copy \
                                     is not loaded and nothing reads it. That is usually a move that \
                                     left the original behind. Nothing is lost -- decide which you \
                                     meant, and delete or merge the other.",
                        })
                    })
                    .collect();
            }
            if layout.common_is_the_home() {
                result["configuration_note"] = json!(format!(
                    "SCATTERHOARD_COMMON names the same directory as SCATTERHOARD_HOME ({}), so it \
                     is doing nothing. That is the default with the variable unset, and it is \
                     harmless -- but if the intent was to share sets between checkouts, it is not \
                     happening, and every set here belongs to this home alone.",
                    layout.home().display()
                ));
            }
        }

        Ok(result)
    }

    /// `mint` — an identity for someone new to this set.
    fn mint_author(
        &mut self,
        scope: &Scope,
        author: &Author,
        source: Source,
    ) -> Result<Value, String> {
        let record = {
            let store = self
                .stores
                .get_mut(scope)
                .ok_or_else(|| missing_scope(&self.withheld, scope))?;
            store
                .mint_author(author, jiff::Timestamp::now())
                .map_err(|error| error.to_string())?
        };
        // **Read after minting, not before.** Minting is a new set's first write, since
        // `set-settled-at-first-write` the usual one, and that write is what gives the set its
        // identity -- so an identity read beforehand is absent, and the durable map row that lets a
        // re-added copy of the set recognise you would go unwritten.
        let set_id = self
            .stores
            .get(scope)
            .ok_or_else(|| missing_scope(&self.withheld, scope))?
            .set_id()
            .map_err(|error| format!("could not read the set's identity: {error}"))?;
        if let Some(set_id) = set_id {
            self.remember_author(scope, set_id, record)?;
        }

        Ok(json!({
            "scope": WireScope(scope),
            "minted": record.to_string(),
            "author": author.as_str(),
            "note": with_relay(
                scope,
                author,
                source,
                "A new identity record, which travels in this set's exported file like any other \
                 memory. Everything you write here from now on refers to it, and changing the name \
                 later supersedes this one record rather than rewriting what points at it.",
            ),
        }))
    }

    /// `identify` — point at a record already in the set. Touches no set data.
    fn identify_author(&mut self, scope: &Scope, record: MemoryId) -> Result<Value, String> {
        let (name, set_id) = {
            let store = self
                .stores
                .get(scope)
                .ok_or_else(|| missing_scope(&self.withheld, scope))?;
            let name = store
                .identify_author(record)
                .map_err(|error| error.to_string())?;
            let set_id = store
                .set_id()
                .map_err(|error| format!("could not read the set's identity: {error}"))?;
            (name, set_id)
        };
        if let Some(set_id) = set_id {
            self.remember_author(scope, set_id, record)?;
        }

        Ok(json!({
            "scope": WireScope(scope),
            "identified": record.to_string(),
            "author": name.as_str(),
            "note": "Nothing in the set changed: which record is you is a fact about this machine, \
                     so this is remembered here and travels nowhere. Memories already stored keep \
                     the identity they were written under.",
        }))
    }

    /// `rename` — supersede this machine's identity record here, and forget the name it carried.
    fn rename_author(
        &mut self,
        scope: &Scope,
        author: &Author,
        source: Source,
    ) -> Result<Value, String> {
        let set_id = {
            let store = self
                .stores
                .get(scope)
                .ok_or_else(|| missing_scope(&self.withheld, scope))?;
            store
                .set_id()
                .map_err(|error| format!("could not read the set's identity: {error}"))?
        };
        let renamed = {
            let store = self
                .stores
                .get_mut(scope)
                .ok_or_else(|| missing_scope(&self.withheld, scope))?;
            store
                .rename_author(author, jiff::Timestamp::now())
                .map_err(|error| error.to_string())?
        };
        if let Some(set_id) = set_id {
            self.remember_author(scope, set_id, renamed.replacement)?;
        }

        Ok(json!({
            "scope": WireScope(scope),
            "author": author.as_str(),
            "record": renamed.replacement.to_string(),
            "superseded": renamed.previous.to_string(),
            "was": renamed.was.as_ref().map(Author::as_str),
            "note": with_relay(
                scope,
                author,
                source,
                "Every memory here now reads as written by the new name, and none of them moved -- \
                 they point at the superseded record, which leads to this one. The old record was \
                 forgotten as well as superseded, so the name it carried is gone from the set rather \
                 than merely retired; a tombstone remains, saying that an identity changed here \
                 without saying from what. Export to carry this to everyone else holding the set.",
            ),
        }))
    }

    /// Rebuild a set's vectors with the model this server holds.
    ///
    /// The way out of a mismatch, and since 2026-08-09 the only one — reads no longer rebuild, so
    /// without this a store whose model is not the one in hand would have no route back to semantic
    /// recall at all. That is why it lands with the refusal rather than after it.
    ///
    /// **It never refuses on account of the history.** Returning to a model that already held the
    /// store is reported and allowed: the vectors at stake are derived state rebuilt in under a
    /// second, while a refusal costs semantic recall itself and can strand a store whose other
    /// server has since been decommissioned. `documents/design/embeddings.md`.
    fn reembed_set(&mut self, scope: &Scope) -> Result<Value, String> {
        let store = self.store_for_mut(scope)?;
        let rebuilt = store
            .rebuild_vectors()
            .map_err(|error| format!("could not rebuild the vectors: {error}"))?;

        let mut result = json!({
            "scope": WireScope(scope),
            "vectors_rebuilt": rebuilt.vectors,
            // Memories this model has no tokens for, left without a vector. Named so a rebuild that
            // leaves memories unfindable by meaning says so, rather than reading as memories lost.
            "unrepresentable": rebuilt.unrepresentable,
            "model": rebuilt.model.as_str(),
            "displaced": rebuilt.displaced.as_ref().map(ModelIdentity::as_str),
        });

        if rebuilt.seen_before > 0 {
            // Directive rather than descriptive. A count is a fact the reader may or may not act
            // on; saying what to do is what makes this usable by a session that has no way to know
            // another server exists. The alternation is stated as identity rather than as elapsed
            // time on purpose -- see `documents/design/embeddings.md`.
            result["warning"] = json!(format!(
                "This model has held this set before and was replaced. Rebuilding it now is at \
                 least the {} time the set has changed hands, which is what two servers configured \
                 with different embedding models look like from inside -- each one meets a \
                 documented failure and applies the documented fix. The rebuild was done, and it \
                 will keep recurring until the configurations agree. Raise it with the human rather \
                 than running this again.",
                ordinal(rebuilt.seen_before + 1)
            ));
        }
        Ok(result)
    }

    /// Bring a new shared set into existence.
    ///
    /// Takes a **name**, not a scope, so `local` is unrepresentable: the local set always exists and
    /// creating it is not a thing anyone can mean.
    /// The scopes this server holds as copies loaded for review.
    ///
    /// **Asked of each store's mode**, which is where the fact lives -- the `eval-` name is for
    /// people. Collected once per response rather than per hit, since a recall can return many
    /// memories from one store.
    fn evaluation_scopes(&self) -> HashSet<Scope> {
        self.stores
            .iter()
            .filter(|(_, store)| store.load_mode().is_ok_and(LoadMode::is_evaluation))
            .map(|(scope, _)| scope.clone())
            .collect()
    }

    /// What this server holds, flattened: enough to say where a link lands, and nothing borrowed.
    ///
    /// Taken before a recall borrows the stores mutably, which is why it is a snapshot rather than a
    /// lookup: the answer is needed while the search still holds them.
    fn loaded_sets(&self) -> Vec<LoadedSet> {
        self.stores
            .iter()
            .map(|(scope, store)| LoadedSet {
                scope: scope.clone(),
                identity: store.set_id().ok().flatten(),
                mode: store.load_mode().map_or("unreadable", LoadMode::as_str),
            })
            .collect()
    }

    /// Refuse, before it starts, an operation that would write to a read-only set.
    ///
    /// The store refuses each write on its own; this is for operations that would otherwise fail
    /// partway -- `absorb` merges records before it carries anything forward -- and for the author
    /// actions, which exist only to attribute writes a read-only set never takes. A scope that is not
    /// loaded passes, so the operation reports that in its own words.
    fn refuse_if_read_only(&self, scope: &Scope) -> Result<(), String> {
        match self.stores.get(scope) {
            Some(store) => store
                .refuse_session_writes_if_read_only()
                .map_err(|error| error.to_string()),
            None => Ok(()),
        }
    }

    /// Why this write cannot be a shared set's first, if it cannot: it is not the charter.
    fn charter_first_refusal(
        &self,
        scope: &Scope,
        keywords: Option<&[Keyword]>,
    ) -> Result<Option<String>, String> {
        let Scope::Shared(_) = scope else {
            return Ok(None);
        };
        let is_charter = keywords
            .unwrap_or_default()
            .iter()
            .any(|keyword| keyword.folded() == CHARTER_KEYWORD);
        if is_charter {
            return Ok(None);
        }
        let Some(store) = self.stores.get(scope) else {
            return Ok(None);
        };
        let empty = store
            .holds_nothing_but_identities()
            .map_err(|error| format!("could not tell whether the set holds anything: {error}"))?;
        Ok(empty.then(|| {
            format!(
                "{} holds nothing yet, and a shared set's first memory has to be its charter: a \
                 memory with the keyword `{CHARTER_KEYWORD}` saying what the set is for, which \
                 routing matches every later memory against, and a `hoard-lang-` keyword for each \
                 language its memories will be written in, such as `hoard-lang-en`. Store that \
                 first, then this. If the set is someone else's, taken on from their file, import \
                 the file instead: it brings their charter.",
                json!(WireScope(scope))
            )
        }))
    }

    /// A set this server holds whose name shares a store file with `set`, if any.
    ///
    /// Asked of every store held, withheld ones included, since a withheld set still owns its file.
    /// `set` itself counts, so the callers ask only after refusing the exact name in words of their
    /// own -- what is left for this to find is a name differing only in case.
    fn case_variant_of(&self, set: &SetName) -> Option<&SetName> {
        self.stores
            .keys()
            .chain(self.withheld.keys())
            .find_map(|scope| match scope {
                Scope::Shared(held) if held.shares_a_file_with(set) => Some(held),
                Scope::Shared(_) | Scope::Local => None,
            })
    }

    fn create_set(&mut self, name: &str, mode: LoadMode) -> Result<Value, String> {
        let set = SetName::new(name)
            .map_err(|error| format!("{name:?} cannot be a set name: {error}"))?;
        // **One of the three boundaries where the evaluation prefix is refused**, the others being a
        // file's header and a link written into a store. A real set carrying it would make the name
        // a convention rather than a fact, and the name is what marks a memory as a proposal.
        if set.is_evaluation() {
            return Err(format!(
                "{name:?} begins with {EVALUATION_PREFIX:?}, which names a set loaded for review \
                 rather than one anyone has adopted. A set of your own needs a name of its own; \
                 `hoard_sets` action \"evaluate\" is what makes a review copy, and names it itself."
            ));
        }
        let scope = Scope::Shared(set);

        if self.stores.contains_key(&scope) {
            return Err(format!(
                "the set {name:?} is already loaded; `hoard_sets` with action \"status\" lists what \
                 this machine holds"
            ));
        }
        if let Scope::Shared(set) = &scope
            && let Some(held) = self.case_variant_of(set)
        {
            return Err(format!(
                "this machine already holds the set {:?}, and {name:?} differs from it only in \
                 case. Set names that differ only in case would share one database file on macOS \
                 and Windows, so they cannot be two sets. Use {:?}, or choose a name that differs \
                 by more than case.",
                held.as_str(),
                held.as_str()
            ));
        }

        let layout = self.layout.as_ref().ok_or(
            "this server has no data directory, so it cannot write a new set's database. That is \
             startup wiring, not something a caller can supply.",
        )?;
        let directory = layout.home().to_path_buf();

        // Refusing an existing database is the safe default recorded in set-lifecycle.md: `create`
        // must not silently adopt a store it did not make, because the set on disk may hold memories
        // whose absence from this session is the actual problem. A set that was dropped is not this
        // case: `drop` removes the database, so `create` proceeds, and `create` then `import` brings
        // the set back under the identity its own file carries.
        //
        // A new set is always made in the *home*, never in the common directory: nothing at creation
        // time knows whether it will turn out to belong to every checkout on this machine, which is a
        // fact about how it gets used and is learned later. Moving the file is how it becomes
        // machine-bound -- `documents/design/store-registry.md`.
        let path = Store::path_for(&scope, &directory);
        if path.exists() {
            return Err(format!(
                "a database for the set {name:?} already exists at {} but is not loaded: it could \
                 not be opened, which the server's log says more about, or it is withheld as a second \
                 copy of a set already held, which `status` lists. Creating over it could hide \
                 memories it holds; restore the set from its exported file instead.",
                path.display()
            ));
        }

        let store = Store::open(scope.clone(), &directory)
            .map_err(|error| format!("could not create the set {name:?}: {error}"))?;
        // Through `add_store`, so the new store gets this server's model rather than silently
        // storing memories that can never be found by meaning.
        self.add_store(store);

        // **Nor is an author named here, since `set-settled-at-first-write` step 3.** Creation used
        // to record the server's configured name, which is how a corpus came to be attributed to a
        // machine account. A shared set's first write now asks who you are in it, because that is how
        // you are known where this set is published -- and `create` cannot tell a new set from one
        // about to be taken on by `import`, which brings its contributors' records with it.

        // **No identity is minted here, since `set-settled-at-first-write`.** `create` cannot tell a
        // new set from one about to be taken on by `import`, so an identity minted now would be
        // displaced by the file's or would fork it. The set takes its identity from its first import
        // or its first write, whichever comes first.

        // Last, once the set is otherwise complete: a read-only set takes nothing from a session, and
        // the store only lets an empty set be made read-only, which this one still is.
        if mode == LoadMode::ReadOnly {
            self.stores
                .get(&scope)
                .expect("just added")
                .choose_read_only()
                .map_err(|error| format!("could not make the set read-only: {error}"))?;
        }

        // Said here because `route-where` promises it is, and because this is the moment the creator
        // is deciding what to do next. A read-only set is never written from here, so the writable
        // set's two steps would both be refused: it is told only where its contents come from.
        let note = match mode {
            LoadMode::ReadOnly => format!(
                "This set is read-only, so nothing is written to it from here. Take it on by \
                 importing its publisher's file: `hoard_sets` action \"import\" with scope \
                 \"{}\" and the directory or file they publish it in. That brings the set's \
                 contents, and importing again refreshes them.",
                scope.as_key()
            ),
            // **Where the human will be asked, the note does not tell the model to ask them**: the
            // first write puts the question to them through their client, and a model that asked
            // as well would be asking twice. `asking-the-human`.
            LoadMode::ReadWrite | LoadMode::Evaluation if self.asks_about(&scope) => format!(
                "The first write here asks the human, through their client, for the name they \
                 publish under in this set -- do not ask them for it yourself. That first memory has \
                 to be its charter: a memory with the keyword `{CHARTER_KEYWORD}` saying what the set \
                 is for, and a `hoard-lang-` keyword for each language its memories will be written \
                 in. If you are taking on a set someone else publishes, import their file instead, \
                 which brings their charter and their records; the first write then asks the human \
                 which record is theirs."
            ),
            LoadMode::ReadWrite | LoadMode::Evaluation => format!(
                "Before anything is written here, this set needs the name its writer is known by \
                 where it is published, which only the human can give you: ask them for it, and with \
                 their answer use `hoard_sets` action \"mint\". {PUBLISHING_NAME_HINT} Then its \
                 first memory has to be its charter: a memory with the keyword `{CHARTER_KEYWORD}` \
                 saying what the set is for, and a `hoard-lang-` keyword for each language its \
                 memories will be written in. If you are taking on a set someone else publishes, \
                 import their file instead, which brings their charter and their records, then \
                 `identify` or `mint`."
            ),
        };

        Ok(json!({
            "created": WireScope(&scope),
            "memories": 0,
            "mode": mode.as_str(),
            "note": note,
        }))
    }

    /// Replace a set's contents with a file's, having written the set out first.
    ///
    /// `set-restore`. **Not `import`, and `documents/design/set-lifecycle.md` is emphatic about
    /// why.** Import *reconciles* a file against a store that may have diverged from it, keeping
    /// local work and reporting genuine conflicts. Restore *replaces*: the file becomes the store
    /// and local divergence is discarded on purpose, because it serves the case where the store is
    /// corrupt or being rebuilt — and reconciling *against* such a store means merging against
    /// content nobody trusts. Conflating them would produce a set that never existed at any moment
    /// in time.
    ///
    /// **One operation, not a sequence.** It is equivalent to `drop` + `create` + `import`, and that
    /// decomposition is deliberately not offered: a hand-composed sequence that fails partway leaves
    /// *no set at all* — dropped and not re-created, or created and not populated — which is
    /// strictly worse than either endpoint and arrives without warning. Composing three destructive
    /// steps by hand is a thing to be doing when nothing has gone wrong, and by construction nobody
    /// is doing it then.
    ///
    /// **Everything about it assumes the caller has less context than usual, not more**, because
    /// this is what a person reaches for under pressure with the store that held their memories as
    /// the thing that broke.
    ///
    /// The order is: validate the file, refuse a destination that would destroy the source, write
    /// the safety copy, then replace. Nothing is destroyed until every one of those has succeeded.
    fn restore_set(
        &mut self,
        scope: &Scope,
        source: &Destination,
        save_to: &std::path::Path,
    ) -> Result<Value, String> {
        if !self.stores.contains_key(scope) {
            return Err(missing_scope(&self.withheld, scope));
        }
        if let Some(refusal) = source.refusal() {
            return Err(refusal);
        }

        let copy_to = Destination::File(save_to.to_path_buf());
        if let Some(refusal) = copy_to.refusal() {
            return Err(refusal);
        }
        // The store's identity if it can still say, and the set's name if not: a restore is what
        // a person reaches for when the store is the thing that broke, so a store that cannot read
        // its own identity must not stop it finding the file.
        let identity = self
            .stores
            .get(scope)
            .and_then(|store| store.set_id().ok().flatten());
        let path = source.file_to_read(scope, identity)?;

        // **The correctness requirement, and the case it was written for.** Both paths come from the
        // caller, so nothing but this stops the safety copy landing on the file being restored from
        // -- destroying the good copy in the act of protecting the bad one. Kept ahead of the
        // occupancy check below, which would also refuse it: this one names *which* file it is, and
        // that is the difference between a diagnosis and a fact.
        //
        // Compared as the caller spelled them, which is the same basis every other path comparison
        // here uses; two spellings of one path are `target-identity-is-an-unnormalised-path-string`
        // and are not a new defect of this one.
        if save_to == path {
            return Err(format!(
                "the safety copy would be written over {}, which is the file being restored from -- \
                 destroying the good copy in the act of protecting the bad one. `save_to` names \
                 somewhere else entirely: it is the *old* contents being put aside, not the new \
                 ones.",
                path.display()
            ));
        }
        if let Some(refusal) = occupied_safety_copy(save_to) {
            return Err(refusal);
        }

        let expectation = memory_file::Expectation::Exactly(scope.clone());

        // **One pass, and the two-pass rule is met by the shape rather than by a second read.**
        // `documents/design/file-format.md` requires that a whole file be checked before any of it
        // is applied — which matters more here than anywhere, since a file malformed on its last
        // line must not cost the caller the set they were rescuing. `import` needs two passes
        // because it writes as it reads. This does not: every record is materialised first and the
        // store is not touched until the whole file has parsed, so a failure anywhere leaves it
        // exactly as it was.
        //
        // **Held in memory deliberately.** The replacement is one transaction, and a half-applied
        // one is the state this operation exists to get out of. The reconciling import already
        // holds a whole file's decisions at once, so this is that path's profile rather than a new
        // one.
        //
        // A validating pass was written here first and removed: a mutation deleting it left the
        // suite green, and tracing why showed it genuinely subsumed — duplicate-id detection lives
        // inside `stream` rather than in `validate`, so the second read was re-proving what the
        // first already had. **If this is ever changed to stream into the store instead, the
        // validating pass has to come back**, because the guarantee above would then rest on
        // nothing.
        let mut records: Vec<memory_file::Record> = Vec::new();
        {
            let file = std::fs::File::open(&path)
                .map_err(|error| format!("could not open {}: {error}", path.display()))?;
            memory_file::stream(&expectation, file, |record| {
                records.push(record.clone());
                Ok(())
            })
            .map_err(|error| format!("{} is not usable: {error}", path.display()))?;
        }

        let store = self.stores.get(scope).expect("just checked");

        // **The same two failure modes as `drop`, answered the same way** -- and this is the
        // operation that makes the asymmetry obvious rather than clever. An unreadable store is the
        // case restore is *for*, so refusing to proceed would withhold the repair from exactly the
        // situation it exists to repair.
        let mut copy: Option<std::path::PathBuf> = None;
        let mut unreadable: Option<String> = None;
        match Self::serialise_set(store, scope, |_, _| {}) {
            Ok(rendered) => {
                copy = Some(Self::write_memory_file(&copy_to, save_to, &rendered.body)?);
            }
            Err(error) => unreadable = Some(error),
        }

        let target = source.target();
        let store = self.stores.get_mut(scope).expect("checked above");
        let replaced = store.replace_with(scope, &records).map_err(|error| {
            format!(
                "nothing was replaced: {error}. If the store is too damaged to be written to, drop \
                 the set and create it again, then import the file{}.",
                copy.as_ref().map_or_else(String::new, |path| format!(
                    " -- the previous contents are at {}",
                    path.display()
                ))
            )
        })?;

        // The agreement this restore just established, and *only* it: `replace_with` cleared every
        // other target's bases, because a base for a record the restored set no longer holds
        // resolves as an anomaly. Recording this one is what stops the next import from this file
        // re-deciding every record.
        let bases: Vec<(MemoryId, u64)> = records
            .iter()
            .map(|record| (record.id(), crate::checksum::of_record(record)))
            .collect();
        store.set_bases(&target, &bases).map_err(|error| {
            format!(
                "restored from {}, but could not record the agreement: {error}. The next import \
                 will treat every record as changed.",
                path.display()
            )
        })?;

        let mut result = json!({
            "restored": WireScope(scope),
            "read": path.to_string_lossy(),
            "path_form": source.form(),
            "memories": replaced.memories,
            "forgotten": replaced.forgotten,
            "safety_copy": copy.as_ref().map(|path| path.to_string_lossy()),
        });
        if let Some(error) = unreadable {
            result["no_safety_copy"] = json!(format!(
                "The set was restored and NO SAFETY COPY EXISTS: the old contents could not be \
                 read, so there was nothing to write. {error}. Anything the store held that this \
                 file does not carry is gone. Proceeding rather than refusing is deliberate -- a \
                 store too damaged to read is the case restore exists for, and refusing would have \
                 withheld the repair from the situation that needed it."
            ));
        } else {
            result["discarded_local_divergence"] = json!(format!(
                "The store's previous contents were replaced by the file, not merged with it. \
                 Anything it held that {} does not carry is gone from the set and is in the safety \
                 copy, which is an ordinary memory file: diff it against the restored set and store \
                 back anything that should have survived.",
                path.display()
            ));
        }
        Ok(result)
    }

    /// Take a file declaring **another** set into this one, as an authored supersession.
    ///
    /// `documents/design/multi-set-sync.md`, *absorption becomes a supersession*. It replaces
    /// `memory_file::absorb`, which rewrote a cross-set link into a same-set link on the way into
    /// the store and was **the only place in the system where a receiver edited authored content**.
    /// That rewrite put one id under two contents from a legitimate operation — measured, not
    /// argued: the test that drove two holders of one file to different content and reconciled
    /// them into a `Conflict` on a memory record is now
    /// `absorbing_reaches_the_other_holder_as_a_record_rather_than_a_conflict` in `tests/sets.rs`,
    /// asserting the closed form of what it used to assert as a defect.
    ///
    /// **Only one direction needs a supersession, and the asymmetry is the whole design.** This
    /// set's own memories with links into the absorbed set have been in service *here*, so changing
    /// them needs ancestry: each is left where it is and a new memory is authored making the same
    /// claim with the link internal, `SUPERSEDES` pointing back. The arriving records are written
    /// **as authored** — their links naming this set canonicalise at the door exactly as every
    /// imported record's already do (`a-same-set-link-spelled-with-its-set-name-is-inert`), which
    /// is unconditional and deterministic and therefore cannot diverge the way a per-call argument
    /// did.
    ///
    /// **The file, not a loaded set.** Reading a file whose header declares another set inverts the
    /// usual scope check from *this file must declare S* into *this file declares another set and I
    /// mean to take it into S* — the intentionality [the set UUID](TRACKING.md)
    /// asks for, expressed as an operation rather than inferred from a header.
    ///
    /// **One operation, not a composed sequence**, and the precedent is [`restore_set`]. The
    /// decomposition — `create`, `import`, `absorb`, `drop`, `export` — has a dangerous failure
    /// mode: stopping after the absorb leaves the memories duplicated across two sets under one set
    /// of ids, with nothing saying you are half done. It also uses `create` as a staging area for a
    /// set you intend to discard, which the tool cannot tell from the real thing.
    ///
    /// **Idempotent by construction.** A second pass finds nothing to supersede, because the tips
    /// already carry same-set links; nothing records that an absorption happened, and nothing needs
    /// to.
    ///
    /// The order is: read the file, refuse a file declaring this very set, refuse an id both sides
    /// hold, write the safety copy, then write.
    fn absorb_set(
        &mut self,
        scope: &Scope,
        source: &Destination,
        set: Option<&SetName>,
        save_to: &std::path::Path,
    ) -> Result<Value, String> {
        if !self.stores.contains_key(scope) {
            return Err(missing_scope(&self.withheld, scope));
        }
        if let Some(refusal) = source.refusal() {
            return Err(refusal);
        }

        let copy_to = Destination::File(save_to.to_path_buf());
        if let Some(refusal) = copy_to.refusal() {
            return Err(refusal);
        }
        let path = source.file_of_named(set)?;
        // The same correctness requirement `restore` has, for the same reason: both paths come from
        // the caller, and nothing else stops the safety copy landing on the file being read.
        if save_to == path {
            return Err(format!(
                "the safety copy would be written over {}, which is the file being absorbed -- \
                 destroying the set you are taking in. `save_to` names somewhere else entirely: it \
                 is *this* set's current contents being put aside.",
                path.display()
            ));
        }
        if let Some(refusal) = occupied_safety_copy(save_to) {
            return Err(refusal);
        }

        // **One pass, holding the records**, and the two-pass rule is met by the shape rather than
        // by a second read — the same argument as `restore_set`: nothing is written until the whole
        // file has parsed, and duplicate-id detection lives inside `stream`. If this is ever changed
        // to write as it reads, a validating pass has to come back.
        let mut records: Vec<memory_file::Record> = Vec::new();
        let arriving = {
            let file = std::fs::File::open(&path)
                .map_err(|error| format!("could not open {}: {error}", path.display()))?;
            // `AnyShared`, which is the inversion: the file is *expected* to declare a set other
            // than this one, and the kind is still not negotiable — a local file never matches.
            // Narrowed to exactly the named set when the caller said which.
            //
            // **Except into `local`, which takes a `local` file and nothing else**, since 2026-09-30.
            // Setting `SCATTERHOARD_COMMON` without moving `local.sqlite3` leaves two personal stores
            // on one machine, each with its own identity, and this is the one way to merge them back
            // -- `import` refuses the second identity and names this action
            // (`a-split-local-store-cannot-be-merged-back`). A shared set still never takes a
            // `local` file: that would carry personal memories into a set that is published.
            let expectation = if *scope == Scope::Local {
                memory_file::Expectation::Exactly(Scope::Local)
            } else {
                named_or_any(set)
            };
            memory_file::stream(&expectation, file, |record| {
                records.push(record.clone());
                Ok(())
            })
            .map_err(|error| format!("{} is not usable: {error}", path.display()))?
        };

        let declared = arriving.scope.clone();
        // The name links into the absorbed set carry. A `local` file has none, and needs none:
        // nothing can link into `local` from another set, so there is no link to make internal.
        let absorbed = match &declared {
            Scope::Shared(name) => Some(name.clone()),
            Scope::Local => None,
        };
        let names_absorbed = |link: &Link| {
            absorbed
                .as_ref()
                .is_some_and(|name| link.target.set() == Some(name))
        };

        // **For `local` the same set is told by identity**, because every `local` file declares the
        // same scope. Only both identities present and equal is the same set: a store not yet
        // written to has none, and has nothing for the file to collide with.
        let same_set = match &declared {
            Scope::Local => {
                let held = self
                    .stores
                    .get(scope)
                    .expect("checked above")
                    .set_id()
                    .map_err(|error| format!("could not read this set's identity: {error}"))?;
                matches!((arriving.set_id, held), (Some(file), Some(held)) if file == held)
            }
            Scope::Shared(_) => declared == *scope,
        };
        if same_set {
            return Err(format!(
                "{} declares {}, which is the set it would be absorbed into. Absorbing takes \
                 *another* set's file: to reconcile a file into the set it declares use action \
                 \"import\", and to replace that set's contents with it use action \"restore\".",
                path.display(),
                declared.as_key()
            ));
        }

        let store = self.stores.get(scope).expect("just checked");

        // One pass over the store, answering both questions it has to answer before anything is
        // written: which ids are already taken, and which of this set's memories link into the set
        // arriving.
        //
        // **Reading the store here is also the guard on an unreadable one**, and the asymmetry with
        // `drop` and `restore` is deliberate rather than an oversight. Those exist *for* the
        // corruption case, so refusing would withhold the repair from the situation that needs it.
        // This one is additive: a set too damaged to read is not a set anything should be merged
        // into, and failing here leaves it exactly as it was.
        let mut held: std::collections::HashMap<MemoryId, u64> = std::collections::HashMap::new();
        let mut held_memories: std::collections::HashSet<MemoryId> =
            std::collections::HashSet::new();
        let mut linked: Vec<Memory> = Vec::new();
        store
            .stream_records(|record| {
                held.insert(record.id(), crate::checksum::of_record(record));
                if matches!(record, memory_file::Record::Memory(_)) {
                    held_memories.insert(record.id());
                }
                if let memory_file::Record::Memory(memory) = record
                    && memory.links.iter().any(&names_absorbed)
                {
                    linked.push((**memory).clone());
                }
                Ok(())
            })
            .map_err(|error| format!("could not read the set: {error}"))?;

        // **Refused before any write, safety copy included.** Both sides are in hand, so comparing
        // is cheap — and collapsing two distinct memories onto one id is the one thing set-scoped
        // ids make possible and nothing else prevents. `documents/design/multi-set-sync.md`,
        // *memory ids are unique within a set*.
        //
        // **A record identical to one already here is not a collision, and that distinction is what
        // makes absorption idempotent.** The danger named above is *two different memories* under
        // one id; identical bytes are one memory, arriving again. Without this, absorbing the same
        // file twice — or a later export of the same set, which carries every record already taken
        // — would be refused on every id it had already delivered, and the idempotence the design
        // claims would hold only for the supersessions and not for the operation.
        let mut already_held = 0usize;
        let mut collisions: Vec<String> = Vec::new();
        for record in &records {
            match held.get(&record.id()) {
                None => {}
                Some(checksum) if *checksum == crate::checksum::of_record(record) => {
                    already_held += 1;
                }
                Some(_) => collisions.push(record.id().to_string()),
            }
        }
        if !collisions.is_empty() {
            return Err(format!(
                "nothing was absorbed: {} and {} hold different records under {}. Two different \
                 memories under one id cannot both be stored, and choosing between them is yours to \
                 do -- keep the one you want and give the other a new id in {}, then absorb again. \
                 A record identical to one already here is not this case and passes without \
                 comment.",
                scope.as_key(),
                declared.as_key(),
                and_list(&collisions),
                path.display()
            ));
        }

        // **A link is made internal only if its target will be a memory here**, since 2026-09-28
        // (`absorb-internalises-links-to-memories-that-did-not-arrive`). A whole set brings every
        // target, but a file carrying part of one is read without complaint, and rewriting a working
        // cross-set link to a same-set id this set lacks exchanges it for a dangling one.
        //
        // **Arrived in this file, or already held**, and the second half is what a move done in
        // steps needs: a target taken in by an earlier absorb is the memory the link means. Ids are
        // generated to be universally unique, so one id in two sets is one memory copied, never a
        // coincidence -- and an arriving copy that differs from the one held is refused above. First
        // built as *arrived in this file* alone, on the mistaken worry that a held id might be a
        // different memory; the human partner's multi-step scenario showed what that cost.
        // Tombstones count for neither: nothing is made internal to a memory this set forgot.
        let will_hold: std::collections::HashSet<MemoryId> = records
            .iter()
            .filter(|record| matches!(record, memory_file::Record::Memory(_)))
            .map(memory_file::Record::id)
            .chain(held_memories.iter().copied())
            .collect();
        let internalises =
            |link: &Link| names_absorbed(link) && will_hold.contains(&link.target.id());

        // Only memories that are still current. A superseded one keeps its cross-set link because
        // that is what it said when it was in service, and superseding it again would author a
        // second replacement for a claim already replaced -- which is also what makes a second
        // absorption of the same file a no-op.
        //
        // And only those with a link this file lets it internalise: a replacement identical to the
        // original apart from its id would be a supersession that changes nothing.
        let mut to_supersede: Vec<Memory> = Vec::new();
        let mut not_internalised: Vec<Value> = Vec::new();
        for memory in linked {
            match store
                .resolve_supersession(memory.id)
                .map_err(|error| format!("could not read this set's supersessions: {error}"))?
            {
                Resolution::Current => {
                    for link in &memory.links {
                        if names_absorbed(link) && !internalises(link) {
                            not_internalised.push(json!({
                                "memory": memory.id.to_string(),
                                "target": link.target.id().to_string(),
                            }));
                        }
                    }
                    if memory.links.iter().any(internalises) {
                        to_supersede.push(memory);
                    }
                }
                Resolution::Superseded { .. } | Resolution::Cycle => {}
            }
        }

        let rendered = Self::serialise_set(store, scope, |_, _| {})?;
        let copy = Self::write_memory_file(&copy_to, save_to, &rendered.body)?;

        let author = self.author_for(scope)?;
        let now = jiff::Timestamp::now();
        let store = self.stores.get_mut(scope).expect("checked above");

        let mut arrived = 0usize;
        let mut forgotten = 0usize;
        for record in &records {
            // Already here, byte for byte. Writing it again would land the same bytes, so this is
            // for the report rather than for correctness: a caller absorbing a second time should
            // be told the file mostly said what the set already knew.
            if held.contains_key(&record.id()) {
                continue;
            }
            match record {
                // `accept_from_file`, not `insert`: the cycle refusal on `insert` is right for a
                // caller authoring one edge and wrong for a whole file, where it would land on
                // whichever record a merge's cycle happened to close on and abort with the set
                // already partly written. `two-import-paths-disagree-about-supersession-cycles`.
                memory_file::Record::Memory(memory) => {
                    let is_identity = memory.is_identity_record();
                    store.accept_from_file(memory).map_err(|error| {
                        format!("could not store memory {}: {error}", record.id())
                    })?;
                    // Stored either way -- it has to survive the round trip -- but not counted as a
                    // memory the caller took on. `count` and `embed_all` draw the same line.
                    if !is_identity {
                        arrived += 1;
                    }
                }
                memory_file::Record::Tombstone(tombstone) => {
                    store.accept_tombstone(tombstone).map_err(|error| {
                        format!("could not forget memory {}: {error}", record.id())
                    })?;
                    forgotten += 1;
                }
            }
        }

        let mut superseded: Vec<Value> = Vec::new();
        for original in to_supersede {
            let mut replacement = original.clone();
            replacement.id = MemoryId::generate();
            replacement.created = Attribution {
                by: author,
                at: now,
            };
            replacement.modified = replacement.created.clone();
            for link in &mut replacement.links {
                if internalises(link) {
                    link.target = crate::memory::LinkTarget::SameSet(link.target.id());
                }
            }
            replacement.links.push(Link {
                label: LinkLabel::Supersedes,
                target: crate::memory::LinkTarget::SameSet(original.id),
            });
            // `carry_forward`, not `insert`: this replacement is a record the set already held,
            // carried forward, and the absorbed set may have brought a charter of its own. Refusing a
            // second charter here would stop the absorb with the records already merged.
            store.carry_forward(&replacement).map_err(|error| {
                format!(
                    "the records arrived, but memory {} could not be superseded: {error}. The set \
                     as it was is at {}.",
                    original.id,
                    copy.display()
                )
            })?;
            superseded.push(json!({
                "memory": original.id.to_string(),
                "superseded_by": replacement.id.to_string(),
            }));
        }

        let mut result = json!({
            "absorbed": WireScope(&declared),
            "into": WireScope(scope),
            "read": path.to_string_lossy(),
            "path_form": source.form(),
            "memories": arrived,
            "forgotten": forgotten,
            "already_held": already_held,
            "superseded": superseded,
            "safety_copy": copy.to_string_lossy(),
            "note": if absorbed.is_some() {
                "The arriving memories were written as their author wrote them. Links from this \
                 set into the absorbed one were not rewritten: each such memory is still here, \
                 saying what it said, and a new memory supersedes it with the link made \
                 internal. That ancestry travels in the exported file, so nobody has to be told \
                 about the merge again."
            } else {
                "The arriving memories were written as their author wrote them, and the identity \
                 record of the store they came from arrived with them, so they are still \
                 attributed to it. Nothing can link into `local` from another set, so nothing \
                 was superseded. Once you are satisfied, the store they came from can be deleted: \
                 its memories are here, and the file read is a copy of it."
            },
        });
        // **Said out loud, because a partial file is otherwise indistinguishable from a whole one.**
        // These links still name the absorbed set, which is usually about to be dropped, so they
        // will dangle unless the rest of that set arrives.
        if !not_internalised.is_empty() {
            result["not_internalised"] = json!(not_internalised);
            result["not_internalised_note"] = json!(
                "These links name the absorbed set, but neither the file nor this set holds their \
                 targets, so they were left as cross-set links rather than made internal to an id \
                 nothing here answers to. If the file was meant to carry the whole set, it did not: \
                 absorb the rest before dropping the absorbed set, or these links will point at \
                 nothing."
            );
        }

        // **Say that you are half done, because nothing else will.** With the absorbed set still
        // loaded, its memories are now in two sets under one set of ids -- the failure mode that
        // argued against composing this operation out of five, arrived by a different route.
        if declared != *scope && self.stores.contains_key(&declared) {
            result["still_loaded"] = json!(format!(
                "{} is still loaded on this machine, so its memories are now in two sets under one \
                 set of ids. Absorbing does not remove it: use action \"drop\" when you are \
                 satisfied with what arrived.",
                declared.as_key()
            ));
        }
        Ok(result)
    }

    /// Remove a set: write it somewhere the caller named, then destroy the database.
    ///
    /// `set-drop`. A set could be created and imported and never removed, so an accidental import
    /// was permanent — demonstrated 2026-08-15, when a test corpus imported as a set took all three
    /// top results on an unrelated query and the only way back was deleting the file by hand and
    /// restarting the server.
    ///
    /// **The save is unconditional and consults no dirty flag**
    /// (`documents/design/set-lifecycle.md`, *the save is unconditional, and no flag gates it*). The
    /// fact such a guard would need — is there any memory here whose content exists in no file? —
    /// is one the reconciliation base cannot answer, because the base records a past agreement
    /// between two replicas rather than what is on disk now. So the gate was removed rather than
    /// tuned, and nothing here reads state out of the database it is about to destroy in order to
    /// decide whether to destroy it.
    ///
    /// **The two ways the copy can fail are opposites, and the natural implementation — *if the
    /// save failed, stop* — gets the first of them exactly backwards:**
    ///
    /// - **The store cannot be read.** The corruption case. Those memories are already beyond
    ///   saving, so refusing protects nothing and blocks the repair. Proceed, and report unmissably
    ///   that no copy was made.
    /// - **The destination cannot be written.** Full disk, bad path, no permission. The store is
    ///   fine and the memories are recoverable, so this is a real failure of the save. Refuse, and
    ///   let the caller fix the destination.
    ///
    /// **There is no force flag, and the escape hatch is outside this surface on purpose.** Where
    /// nothing is writable at all, the way through is to delete `shared-<name>.sqlite3` by hand and
    /// restart the server. A flag is one argument away from any caller including a model working
    /// quickly; a manual deletion plus a restart cannot happen without a human doing it
    /// deliberately.
    fn drop_set(&mut self, name: &str, save_to: Option<&std::path::Path>) -> Result<Value, String> {
        let set = SetName::new(name)
            .map_err(|error| format!("{name:?} cannot be a set name: {error}"))?;
        let scope = Scope::Shared(set);

        // A withheld store is reachable here and nowhere else: dropping one of two sets that share
        // an identity is how the other gets loaded.
        let Some(store) = self
            .stores
            .get(&scope)
            .or_else(|| self.withheld.get(&scope))
        else {
            return Err(unknown_scope(&scope));
        };
        let identity = store.set_id().ok().flatten();
        let mode = store
            .load_mode()
            .map_err(|error| format!("could not read the set's load mode: {error}"))?;

        // **The unconditional save, kept exactly where it was argued for.** A set this machine could
        // have written to holds what no file may, so it is written out first and there is no flag to
        // skip that. A read-only set holds what its publisher's file brought and nothing else, so the
        // copy is skipped -- and `save_to` stays honoured where a caller gives it anyway, out of habit
        // or because they want the file. `documents/design/load-modes.md`.
        let destination = match (save_to, mode.owes_a_safety_copy()) {
            (Some(path), _) => Some((Destination::File(path.to_path_buf()), path)),
            (None, false) => None,
            (None, true) => {
                return Err(format!(
                    "dropping the set {name:?} destroys its database, and this machine could have \
                     written to it, so it is saved first: give `save_to` a path to write it to. \
                     (A read-only set needs none, because its memories came from its publisher's \
                     file.)"
                ));
            }
        };
        if let Some((destination, path)) = &destination {
            if let Some(refusal) = destination.refusal() {
                return Err(refusal);
            }
            // Ahead of reading the store, and that ordering is the documented asymmetry rather than
            // convenience: *the destination cannot be written* is the failure drop refuses on, where
            // *the store cannot be read* is the one it proceeds past. This is the first kind.
            if let Some(refusal) = occupied_safety_copy(path) {
                return Err(refusal);
            }
        }

        // The database this is about to remove, asked of the connection rather than rebuilt from
        // the layout: a store may be opened from the home or the common directory, and recomputing
        // the path would guess which. `None` is an in-memory store, which has no file to unlink.
        let database = store.file().map(std::path::Path::to_path_buf);

        // **Everything the store is read for happens here, before anything is destroyed**, so that
        // an unreadable store takes one branch rather than failing partway through a drop. Skipped
        // entirely when no copy is being written: the read is only for the copy, and a set that
        // cannot be read is not a reason to refuse a drop that was never going to save it.
        let mut copy: Option<std::path::PathBuf> = None;
        let mut unreadable: Option<String> = None;
        let mut counts = json!({});
        if let Some((destination, path)) = &destination {
            match Self::serialise_set(store, &scope, |_, _| {}) {
                Ok(rendered) => {
                    counts = json!({"memories": rendered.written});
                    copy = Some(Self::write_memory_file(destination, path, &rendered.body)?);
                }
                // Reported, not refused, and reported in a field a caller cannot read past. See the
                // second bullet above: this is the branch that must not become a `?`.
                Err(error) => unreadable = Some(error),
            }
        }

        // Removing the store from the map drops its `Connection`, which is what closes the database
        // before the file is unlinked. Doing it in the other order leaves an open handle to a
        // deleted file -- harmless on this platform and not on every one.
        self.stores.remove(&scope);
        self.withheld.remove(&scope);
        // Settled again whether or not the dropped copy was a withheld one: removing a copy can leave
        // a lone read-only copy, or the one read-write copy among several.
        if let Some(identity) = identity {
            self.settle_identity(identity);
        }
        if let Some(database) = &database {
            std::fs::remove_file(database).map_err(|error| {
                format!(
                    "the set was written to {} and unloaded, but its database at {} could not be \
                     removed: {error}. It will load again on the next start. Remove the file by \
                     hand, or restore the set from the copy.",
                    copy.as_ref()
                        .map_or_else(|| "no file".to_owned(), |path| path.display().to_string()),
                    database.display()
                )
            })?;
        }

        let mut result = json!({
            "dropped": WireScope(&scope),
            "safety_copy": copy.as_ref().map(|path| path.to_string_lossy()),
            "database_removed": database.as_ref().map(|path| path.to_string_lossy()),
        });
        if let Some(memories) = counts.get("memories") {
            result["memories"] = memories.clone();
        }
        if destination.is_none() {
            result["safety_copy_skipped"] = json!(
                "Nothing was saved: no `save_to` was given, and this set was read-only here, so \
                 this machine never wrote to it and it held nothing its publisher's file does not. \
                 If you no longer have that file, these memories are gone from this machine."
            );
        }
        if let Some(error) = unreadable {
            result["no_safety_copy"] = json!(format!(
                "The set was dropped and NO SAFETY COPY EXISTS: it could not be read, so there was \
                 nothing to write. {error}. Whatever it held is gone unless the set has an exported \
                 file elsewhere -- dropping does not touch one, so look for it before doing anything \
                 else. Proceeding rather than refusing is deliberate: memories that cannot be read \
                 cannot be saved, and refusing would only have blocked the repair."
            ));
        }
        Ok(result)
    }

    /// Write one set's memory file, for a caller that is not a session.
    ///
    /// **One path in, deliberately.** The command line could have built a `tools/call` request and
    /// read `isError` back off the response envelope, which would have added no public API and made
    /// the binary depend on the envelope's shape rather than on the operation. Delegating instead
    /// means a rule added to [`Self::export_set`] cannot be missed by the other caller, because
    /// there is no other caller: this *is* that function, named for outside use.
    ///
    /// # Errors
    ///
    /// Returns the refusal, worded for a person, when the scope is not one this server holds or the
    /// destination cannot be written to.
    pub fn export_to(&mut self, scope: &Scope, destination: &Destination) -> Result<Value, String> {
        self.export_set(scope, destination)
    }

    /// Write a set to a file under a caller-named directory, and record the agreement.
    ///
    /// **In a checkout, the directory settles the kind and the header settles which set.**
    /// `documents/design/trigger-and-loading.md` rests on the reserved directory being known before
    /// anything reads the contents — *"a file found here is a project set no matter what its header
    /// claims"* — which makes whatever is there data, never instructions. Which set a file holds was
    /// never a trust decision, so the set's file is found by the identity in its header and may be
    /// called anything: `documents/design/set-lifecycle.md`, *several sets in one directory*.
    ///
    /// **The directory comes from the caller on every call, never from stored state.** One store
    /// serves every checkout of a repository, and a developer may hold two — development and review —
    /// so a remembered directory would send one session's export into the other's working tree,
    /// silently, since both paths are valid. Nothing on this side can tell which checkout is asking:
    /// the process serves one session but its working directory is whatever the client gave it. The
    /// caller is the only participant that knows. What *is* remembered is used to warn, not to decide.
    /// The person's preferences file as text, or empty where there is none to read.
    fn person_preferences_text(&self) -> String {
        self.layout
            .as_ref()
            .and_then(|layout| Preferences::load(layout.common()).ok())
            .map(|preferences| preferences.text().to_owned())
            .unwrap_or_default()
    }

    /// How the review treats `scope`, given the person's lists.
    fn review_kind(scope: &Scope, lists: &ListedPreferences) -> SetKind {
        match scope {
            Scope::Local => SetKind::Local,
            Scope::Shared(name) if lists.never_publish.iter().any(|n| n == name.as_str()) => {
                SetKind::NeverPublished
            }
            Scope::Shared(_) => SetKind::Shared,
        }
    }

    /// The records of the file a shared set is published to, by checksum. Empty where there is no
    /// file yet, which is the first export, when nothing has been reviewed.
    fn published_records(
        &self,
        scope: &Scope,
        destination: &Destination,
    ) -> Result<std::collections::HashMap<MemoryId, u64>, String> {
        let identity = self
            .stores
            .get(scope)
            .map(Store::set_id)
            .transpose()
            .map_err(|error| error.to_string())?
            .flatten();
        let path = match destination.file_to_read(scope, identity) {
            Ok(path) => path,
            Err(_) => return Ok(std::collections::HashMap::new()),
        };
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(std::collections::HashMap::new());
            }
            Err(error) => return Err(format!("could not read {}: {error}", path.display())),
        };
        let mut held = std::collections::HashMap::new();
        let header = memory_file::stream(
            &memory_file::Expectation::Exactly(scope.clone()),
            file,
            |record| {
                held.insert(record.id(), crate::checksum::of_record(record));
                Ok(())
            },
        )
        .map_err(|error| format!("{} is not usable: {error}", path.display()))?;
        // A backup was never reviewed, so nothing in it counts as reviewed, wherever it is put.
        if header.backup {
            held.clear();
        }
        Ok(held)
    }

    /// The sets reviewed but never gated -- `local`, and those the person never publishes -- that hold
    /// memories nobody has reviewed, with how many each.
    ///
    /// **Said at session start, not at the backup**, decided 2026-10-22 by the human partner: like a
    /// car that lights its maintenance warning at engine start, a notice in a backup made as the
    /// person signs off is forgotten by the next session. A published shared set is left out: export
    /// refuses it, which is its reminder, at the moment it matters.
    fn reviews_due(&self) -> Result<Vec<Value>, String> {
        let lists = ListedPreferences::parse(&self.person_preferences_text());
        let mut due = Vec::new();
        for scope in self.stores.keys() {
            if Self::review_kind(scope, &lists) == SetKind::Shared {
                continue;
            }
            let waiting = self
                .unreviewed(scope, &std::collections::HashMap::new())?
                .len();
            if waiting > 0 {
                due.push(json!({"set": scope.as_key(), "memories": waiting}));
            }
        }
        Ok(due)
    }

    /// Every memory in `scope` the export review has not cleared, in id order.
    ///
    /// **"In the file" counts as reviewed**, because export never writes an unreviewed memory into
    /// it -- except while a full review is under way, when only clearances count.
    fn unreviewed(
        &self,
        scope: &Scope,
        published: &std::collections::HashMap<MemoryId, u64>,
    ) -> Result<Vec<Memory>, String> {
        let store = self
            .stores
            .get(scope)
            .ok_or_else(|| format!("no set {} is loaded", scope.as_key()))?;
        let cleared = store.clearances().map_err(|error| error.to_string())?;
        let reopened = store.reopened().map_err(|error| error.to_string())?;
        let full = store
            .review_setting(ReviewSetting::FullReview)
            .map_err(|error| error.to_string())?
            .is_some();
        let mut found = Vec::new();
        store
            .stream_records(|record| {
                // An identity record is the name the set publishes under, put there on purpose,
                // and nothing a review could keep out without breaking every attribution.
                if let memory_file::Record::Memory(memory) = record
                    && memory.created.by != memory.id
                {
                    let sum = crate::checksum::of_record(record);
                    let in_file = !full && published.get(&memory.id) == Some(&sum);
                    let reviewed = cleared.get(&memory.id) == Some(&sum) || in_file;
                    if !reviewed || reopened.contains(&memory.id) {
                        found.push((**memory).clone());
                    }
                }
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        Ok(found)
    }

    /// The pre-pass over everything an export would publish of `memory`.
    fn review_hits(
        memory: &Memory,
        lists: &ListedPreferences,
        always_fine: &WordList,
        authors: &[String],
    ) -> Vec<Hit> {
        let mut text = memory.content.as_data().to_owned();
        for field in [&memory.why, &memory.context].into_iter().flatten() {
            text.push('\n');
            text.push_str(field.as_data());
        }
        for keyword in &memory.keywords {
            text.push('\n');
            text.push_str(keyword.as_str());
        }
        pre_pass(&text, &lists.never_store, always_fine, authors)
    }

    /// `hoard_sets` action `review`: put the memories export would refuse in front of the session.
    ///
    /// `documents/design/export-review.md`, *The procedure*. Grouped by subject, each memory numbered
    /// by sentence across its text, reasoning and context, with the mechanical pre-pass's hits first.
    /// A small review set is shown beside already-reviewed memories from each memory's subject.
    fn review_set(
        &mut self,
        scope: &Scope,
        destination: Option<Destination>,
        full: bool,
        recheck: Option<&[String]>,
    ) -> Result<Value, String> {
        self.refuse_if_read_only(scope)?;
        let person = self.person_preferences_text();
        let lists = ListedPreferences::parse(&person);
        let kind = Self::review_kind(scope, &lists);
        let (published, project) = match kind {
            SetKind::Shared => {
                let destination = destination.as_ref().ok_or(
                    "reviewing a shared set needs `directory` or `file`: the file it is published \
                     to, which is what already reviewed is measured against. A set the person never \
                     publishes is listed in their preferences file instead.",
                )?;
                let project = match destination {
                    Destination::Directory(directory) => ProjectPreferences::load(directory)?,
                    Destination::File(_) => None,
                }
                .unwrap_or_default();
                (self.published_records(scope, destination)?, Some(project))
            }
            _ => (std::collections::HashMap::new(), None),
        };
        let audience = project.as_ref().map(|p| p.audience);

        {
            let store = self.store_for_mut(scope)?;
            // **A change of audience to public reopens the whole set**: "while it is not public"
            // exceptions no longer hold for anything already published.
            let before = store
                .review_setting(ReviewSetting::Audience)
                .map_err(|error| error.to_string())?;
            let became_public =
                before.as_deref() == Some("private") && audience == Some(Audience::Public);
            if full || became_public {
                store
                    .forget_clearances()
                    .map_err(|error| error.to_string())?;
                store
                    .set_review_setting(ReviewSetting::FullReview, Some("1"))
                    .map_err(|error| error.to_string())?;
            }
            if let Some(audience) = audience {
                let declared = match audience {
                    Audience::Private => "private",
                    Audience::Public => "public",
                };
                store
                    .set_review_setting(ReviewSetting::Audience, Some(declared))
                    .map_err(|error| error.to_string())?;
            }
        }
        let preferences_changed = self.absorb_preference_change(
            scope,
            kind,
            &person,
            &lists,
            project.as_ref(),
            &published,
            recheck,
        )?;

        self.prune_flagged(scope)?;
        let session = self.reviews.entry(scope.clone()).or_default();
        session.destination = destination.clone();
        let flagged = session.flagged.clone();

        let mut pending: Vec<Memory> = self
            .unreviewed(scope, &published)?
            .into_iter()
            .filter(|memory| !flagged.contains(&memory.id))
            .collect();
        let remaining = pending.len();

        let store = self.stores.get(scope).ok_or("no such set")?;
        // The person's own published name is not flagged: `the-review-flags-the-persons-own-published-name`.
        let authors = store
            .others_author_names()
            .map_err(|error| error.to_string())?;
        let published_as = store.published_name().map_err(|error| error.to_string())?;
        let mut fine: Vec<String> = lists
            .always_fine
            .written()
            .iter()
            .map(|w| (*w).to_owned())
            .collect();
        if let Some(project) = &project {
            fine.extend(project.always_fine.iter().cloned());
        }
        let always_fine = WordList::from_items(&fine);

        // Pre-pass hits first, then the rest, each part in id order. Taken out of the session and
        // put back with the page, since `unreviewed` below borrows the server whole.
        let mut memo = std::mem::take(&mut self.reviews.entry(scope.clone()).or_default().hits);
        let against = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (&person, &fine, &authors).hash(&mut hasher);
            hasher.finish()
        };
        if memo.against != against {
            memo = ReviewHitMemo {
                against,
                ..ReviewHitMemo::default()
            };
        }
        pending.sort_by_cached_key(|memory| {
            let sum = crate::checksum::of_memory(memory);
            let hit = match memo.found.get(&memory.id) {
                Some((held, hit)) if *held == sum => *hit,
                _ => {
                    let hit = !Self::review_hits(memory, &lists, &always_fine, &authors).is_empty();
                    memo.found.insert(memory.id, (sum, hit));
                    hit
                }
            };
            (!hit, memory.id)
        });
        let mut spent = 0;
        let mut fits = 0;
        for memory in &pending {
            let size = memory.content.as_data().chars().count()
                + memory
                    .why
                    .as_ref()
                    .map_or(0, |w| w.as_data().chars().count())
                + memory
                    .context
                    .as_ref()
                    .map_or(0, |c| c.as_data().chars().count());
            if fits > 0 && (fits == REVIEW_BATCH || spent + size > REVIEW_BATCH_CHARS) {
                break;
            }
            spent += size;
            fits += 1;
        }
        pending.truncate(fits);

        let subjects = store
            .summary_subjects_by_id()
            .map_err(|error| error.to_string())?;
        let (ids, plain): (Vec<MemoryId>, Vec<crate::summary::Subject>) =
            subjects.into_iter().unzip();
        let fingerprint = crate::summary::fingerprint(&plain);
        let grouping = match self.reviews.get_mut(scope).and_then(|s| s.grouping.take()) {
            Some(held) if held.fingerprint == fingerprint => held,
            _ => {
                let groups_of =
                    crate::summary::assignment(&plain, crate::summary::clusters_for(plain.len()));
                let labels = Self::group_labels(store, &ids, &groups_of)?;
                ReviewGrouping {
                    fingerprint,
                    group_of: ids.iter().copied().zip(groups_of).collect(),
                    ids,
                    labels,
                }
            }
        };
        let ReviewGrouping {
            ids,
            group_of,
            labels,
            ..
        } = &grouping;

        let show = |memory: &Memory| {
            let parts = numbered(
                memory.content.as_data(),
                memory.why.as_ref().map(MemoryText::as_data),
                memory.context.as_ref().map(MemoryText::as_data),
            );
            (
                parts.len(),
                json!({
                    "id": memory.id.to_string(),
                    "sentences": parts,
                    "keywords": memory.keywords.iter().map(Keyword::as_str).collect::<Vec<_>>(),
                }),
            )
        };

        let mut pre_pass_hits = Vec::new();
        let mut grouped: BTreeMap<Option<usize>, Vec<Value>> = BTreeMap::new();
        let mut shown = Vec::new();
        for memory in &pending {
            let hits = Self::review_hits(memory, &lists, &always_fine, &authors);
            if !hits.is_empty() {
                pre_pass_hits.push(json!({"id": memory.id.to_string(), "hits": hits}));
            }
            let (count, shown_as) = show(memory);
            shown.push((memory.id, crate::checksum::of_memory(memory), count));
            grouped
                .entry(group_of.get(&memory.id).copied())
                .or_default()
                .push(shown_as);
        }

        // Context for a small review: already-reviewed memories from each memory's own subject.
        let mut context: BTreeMap<Option<usize>, Vec<Value>> = BTreeMap::new();
        if !pending.is_empty() && remaining <= REVIEW_ALONE {
            let waiting: HashSet<MemoryId> = self
                .unreviewed(scope, &published)?
                .iter()
                .map(|memory| memory.id)
                .collect();
            for group in grouped.keys().copied().collect::<Vec<_>>() {
                let neighbours: Vec<Value> = ids
                    .iter()
                    .filter(|id| group_of.get(id).copied() == group && !waiting.contains(id))
                    .filter_map(|id| store.get(*id).ok().flatten())
                    .take(REVIEW_CONTEXT)
                    .map(|memory| show(&memory).1)
                    .collect();
                if !neighbours.is_empty() {
                    context.insert(group, neighbours);
                }
            }
        }

        let groups: Vec<Value> = grouped
            .into_iter()
            .map(|(group, memories)| {
                let mut entry = json!({
                    "about": group
                        .and_then(|g| labels.get(&g).cloned())
                        .unwrap_or_else(|| vec!["not embedded: in the order written".to_owned()]),
                    "memories": memories,
                });
                if let Some(neighbours) = context.remove(&group) {
                    entry["context"] = json!(neighbours);
                }
                entry
            })
            .collect();

        let current = self.session;
        let session = self.reviews.entry(scope.clone()).or_default();
        for (id, sum, count) in shown {
            session.shown.insert(id, (sum, count));
        }
        session.grouping = Some(grouping);
        session.hits = memo;
        // **Briefed again on a new session, a change to the preferences, or a full review's start**:
        // each is a reader who does not yet hold the text, or holds a version of it that is stale.
        let brief = full || preferences_changed.is_some() || session.briefed_in != Some(current);
        session.briefed_in = Some(current);

        let mut result = json!({
            "set": scope.as_key(),
            "kind": match kind {
                SetKind::Local => "personal",
                SetKind::NeverPublished => "never published",
                SetKind::Shared => "shared",
            },
            "audience": match (kind, audience) {
                (SetKind::Shared, Some(Audience::Private)) => "private",
                (SetKind::Shared, _) => "public",
                _ => "personal",
            },
            "to_review": remaining,
            "shown": pending.len(),
            "task": if brief { REVIEW_TASK } else { REVIEW_TASK_AGAIN },
            "groups": groups,
            "caution": REVIEW_CAUTION,
        });
        if brief {
            result["preferences"] = json!(person);
            result["others_keep_out"] = json!(lists.for_others().items());
            if let Some(project) = project.filter(|p| !p.keep_out.is_empty()) {
                result["project_keep_out"] = json!(project.keep_out);
            }
            if let Some(name) = published_as {
                result["published_as"] = json!(name);
            }
        }
        if !pre_pass_hits.is_empty() {
            result["pre_pass"] = json!(pre_pass_hits);
        }
        if !flagged.is_empty() {
            result["awaiting_the_person"] =
                json!(flagged.iter().map(ToString::to_string).collect::<Vec<_>>());
        }
        if let Some(changed) = preferences_changed {
            result["preferences_changed"] = changed;
        }
        Ok(result)
    }

    /// Turn a change to the preferences since the last review into memories to review again.
    ///
    /// `documents/design/export-review.md`, *a change to the preferences reopens only what it
    /// tightens*. Words added to the never-store list reopen the reviewed memories that contain them,
    /// matched by the server. Items added to a keep-out list, and exceptions removed, need judgement:
    /// the first review after the change shows every subject group of reviewed memories and asks
    /// which the change could apply to, and the next, given `recheck`, reopens those groups. Loosening
    /// reopens nothing. Once absorbed, the current text becomes what the next change is measured from.
    #[allow(clippy::too_many_arguments)]
    fn absorb_preference_change(
        &mut self,
        scope: &Scope,
        kind: SetKind,
        person: &str,
        lists: &ListedPreferences,
        project: Option<&ProjectPreferences>,
        published: &std::collections::HashMap<MemoryId, u64>,
        recheck: Option<&[String]>,
    ) -> Result<Option<Value>, String> {
        let project_keep_out = project.map(|p| p.keep_out.clone()).unwrap_or_default();
        let current = format!(
            "{person}\n{}",
            serde_json::to_string(&project_keep_out).unwrap_or_default()
        );
        let store = self.stores.get(scope).ok_or("no such set")?;
        let Some(before) = store.review_snapshot().map_err(|error| error.to_string())? else {
            self.store_for_mut(scope)?
                .set_review_snapshot(&current)
                .map_err(|error| error.to_string())?;
            return Ok(None);
        };
        if before == current {
            return Ok(None);
        }
        let (old_person, old_project) = before.rsplit_once('\n').unwrap_or((before.as_str(), "[]"));
        let old = ListedPreferences::parse(old_person);
        let old_project: Vec<String> = serde_json::from_str(old_project).unwrap_or_default();

        let added = |now: &[String], then: &[String]| -> Vec<String> {
            now.iter()
                .filter(|item| !then.contains(item))
                .cloned()
                .collect()
        };
        let old_words: Vec<String> = old
            .never_store
            .written()
            .iter()
            .map(|w| (*w).to_owned())
            .collect();
        let now_words: Vec<String> = lists
            .never_store
            .written()
            .iter()
            .map(|w| (*w).to_owned())
            .collect();
        let new_words = added(&now_words, &old_words);
        let mut tightened = added(&lists.keep_out_every, &old.keep_out_every);
        if kind == SetKind::Shared {
            tightened.extend(added(&lists.keep_out_shared, &old.keep_out_shared));
            tightened.extend(added(&project_keep_out, &old_project));
        }
        let removed_exceptions = added(&old.exceptions, &lists.exceptions);

        let waiting: HashSet<MemoryId> = self
            .unreviewed(scope, published)?
            .iter()
            .map(|m| m.id)
            .collect();
        let store = self.stores.get(scope).ok_or("no such set")?;
        let mut reviewed: Vec<Memory> = Vec::new();
        store
            .stream_records(|record| {
                if let memory_file::Record::Memory(memory) = record
                    && memory.created.by != memory.id
                    && !waiting.contains(&memory.id)
                {
                    reviewed.push((**memory).clone());
                }
                Ok(())
            })
            .map_err(|error| error.to_string())?;

        let words = WordList::from_items(&new_words);
        let mut reopen: Vec<MemoryId> = reviewed
            .iter()
            .filter(|memory| {
                let mut text = memory.content.as_data().to_owned();
                for field in [&memory.why, &memory.context].into_iter().flatten() {
                    text.push('\n');
                    text.push_str(field.as_data());
                }
                words.find(&text).is_some()
            })
            .map(|memory| memory.id)
            .collect();

        let needs_judgement = !tightened.is_empty() || !removed_exceptions.is_empty();
        let mut ask = None;
        if needs_judgement {
            let subjects = store
                .summary_subjects_by_id()
                .map_err(|error| error.to_string())?;
            let (ids, plain): (Vec<MemoryId>, Vec<crate::summary::Subject>) =
                subjects.into_iter().unzip();
            let assignment =
                crate::summary::assignment(&plain, crate::summary::clusters_for(plain.len()));
            let labels = Self::group_labels(store, &ids, &assignment)?;
            let reviewed_ids: HashSet<MemoryId> = reviewed.iter().map(|m| m.id).collect();
            let mut groups: BTreeMap<usize, Vec<MemoryId>> = BTreeMap::new();
            for (id, group) in ids.iter().zip(&assignment) {
                if reviewed_ids.contains(id) {
                    groups.entry(*group).or_default().push(*id);
                }
            }
            match recheck {
                Some(chosen) => {
                    for (group, members) in &groups {
                        if chosen.iter().any(|c| *c == format!("g{group}")) {
                            reopen.extend(members.iter().copied());
                        }
                    }
                }
                None => {
                    ask = Some(json!({
                        "tightened": tightened,
                        "exceptions_removed": removed_exceptions,
                        "groups": groups.iter().map(|(group, members)| json!({
                            "group": format!("g{group}"),
                            "about": labels.get(group).cloned().unwrap_or_default(),
                            "memories": members.len(),
                        })).collect::<Vec<_>>(),
                        "ask": "The person's preferences are stricter than when these memories were \
                                reviewed. Say which groups the change could apply to, show the person \
                                each group with your verdict, then call review again with `recheck` \
                                naming those groups -- an empty list if none.",
                    }));
                }
            }
        }

        let store = self.store_for_mut(scope)?;
        store.reopen(&reopen).map_err(|error| error.to_string())?;
        if ask.is_none() {
            store
                .set_review_snapshot(&current)
                .map_err(|error| error.to_string())?;
        }
        Ok(ask.or_else(|| {
            (!reopen.is_empty() || !new_words.is_empty() || needs_judgement).then(
                || json!({"reopened": reopen.iter().map(ToString::to_string).collect::<Vec<_>>()}),
            )
        }))
    }

    /// Each subject group's label, keyed by its index in the assignment.
    fn group_labels(
        store: &Store,
        ids: &[MemoryId],
        assignment: &[usize],
    ) -> Result<std::collections::HashMap<usize, Vec<String>>, String> {
        let parts = assignment.iter().copied().max().map_or(0, |most| most + 1);
        let mut members: Vec<Vec<Vec<Keyword>>> = vec![Vec::new(); parts];
        for (id, group) in ids.iter().zip(assignment) {
            let keywords = store
                .get(*id)
                .map_err(|error| error.to_string())?
                .map(|memory| memory.keywords)
                .unwrap_or_default();
            members[*group].push(keywords);
        }
        // `label_clusters` orders its labels largest first, ties as given: follow the same order to
        // put each label back on its group.
        let present: Vec<usize> = (0..parts).filter(|g| !members[*g].is_empty()).collect();
        let clusters: Vec<crate::summary::Cluster> = present
            .iter()
            .map(|g| crate::summary::Cluster::new(members[*g].clone()))
            .collect();
        let mut order: Vec<usize> = (0..present.len()).collect();
        order.sort_by_key(|i| std::cmp::Reverse(clusters[*i].size()));
        let labels = crate::summary::label_clusters(&clusters, SUMMARY_LABELS);
        Ok(order
            .into_iter()
            .zip(labels)
            .map(|(i, label)| {
                (
                    present[i],
                    label
                        .keywords
                        .iter()
                        .map(|k| k.as_str().to_owned())
                        .collect(),
                )
            })
            .collect())
    }

    /// Drop from the memories awaiting the person any that were forgotten, or changed since they were
    /// shown.
    ///
    /// **The person decides about the text they were shown**, so a flagged memory amended since
    /// cannot be cleared by them, and goes back to the review instead: before this, `person_cleared`
    /// recorded the amended text as reviewed. And one that was forgotten has nothing left to decide,
    /// so it no longer counts as waiting. Found 2026-10-26, when a flagged memory was reworded on the
    /// review's first run on this project's own set and still showed as awaiting the person.
    fn prune_flagged(&mut self, scope: &Scope) -> Result<(), String> {
        let Some(session) = self.reviews.get(scope) else {
            return Ok(());
        };
        let store = self.stores.get(scope).ok_or("no such set")?;
        let mut stale = Vec::new();
        for id in &session.flagged {
            let current = store
                .get(*id)
                .map_err(|error| error.to_string())?
                .map(|memory| crate::checksum::of_memory(&memory));
            let shown = session.shown.get(id).map(|(sum, _)| *sum);
            if current.is_none() || current != shown {
                stale.push(*id);
            }
        }
        if let Some(session) = self.reviews.get_mut(scope) {
            for id in &stale {
                session.flagged.remove(id);
            }
        }
        Ok(())
    }

    /// `hoard_sets` action `clear`: record the session's answers, and the person's own decisions.
    ///
    /// The server routes each answer (`crate::review::route`): a memory is cleared or put to the
    /// person, and only the person clears one that was put to them.
    fn clear_reviewed(
        &mut self,
        scope: &Scope,
        answers: &[MemoryAnswer],
        person_cleared: &[MemoryId],
    ) -> Result<Value, String> {
        self.refuse_if_read_only(scope)?;
        self.prune_flagged(scope)?;
        let session = self.reviews.get(scope).cloned().ok_or_else(|| {
            format!(
                "nothing has been reviewed for {} in this session: run hoard_sets action `review` \
                 first, and answer the memories it shows",
                scope.as_key()
            )
        })?;
        // **A call that answers nothing is refused**, not taken as a review that found nothing: on
        // `gemma4:12b` a `clear` with no answers came back with every memory still waiting, and the
        // model told the person the review was complete.
        if answers.is_empty() && person_cleared.is_empty() {
            return Err(format!(
                "Nothing was cleared, because nothing was answered. Answer each memory the review \
                 showed in `answers`, one entry per numbered sentence: {} memories were shown in this \
                 session.",
                session.shown.len()
            ));
        }
        let person = self.person_preferences_text();
        let lists = ListedPreferences::parse(&person);
        let kind = Self::review_kind(scope, &lists);
        let store = self.stores.get(scope).ok_or("no such set")?;
        let authors = store
            .others_author_names()
            .map_err(|error| error.to_string())?;
        let project = match &session.destination {
            Some(Destination::Directory(directory)) => ProjectPreferences::load(directory)?,
            _ => None,
        }
        .unwrap_or_default();
        let mut fine: Vec<String> = lists
            .always_fine
            .written()
            .iter()
            .map(|w| (*w).to_owned())
            .collect();
        fine.extend(project.always_fine.iter().cloned());
        let always_fine = WordList::from_items(&fine);

        let mut to_clear: Vec<(MemoryId, u64)> = Vec::new();
        let mut flagged = Vec::new();
        let mut refused = Vec::new();
        for answer in answers {
            let Some((shown_sum, count)) = session.shown.get(&answer.id) else {
                refused.push(json!({"id": answer.id.to_string(), "why": "not shown by a review in this session"}));
                continue;
            };
            let Some(memory) = store.get(answer.id).map_err(|error| error.to_string())? else {
                refused.push(json!({"id": answer.id.to_string(), "why": "no longer in the set"}));
                continue;
            };
            let sum = crate::checksum::of_memory(&memory);
            if sum != *shown_sum {
                refused.push(json!({"id": answer.id.to_string(), "why": "changed since it was shown: review it again"}));
                continue;
            }
            let answered: HashSet<usize> = answer.sentences.iter().map(|s| s.n).collect();
            if (1..=*count).any(|n| !answered.contains(&n)) {
                refused.push(json!({"id": answer.id.to_string(), "why": format!("answer every sentence, 1 to {count}")}));
                continue;
            }
            // **A sentence about a person with no `says` is not refused**: `route` reads it as judging,
            // so it goes to the person. Decided by the human partner on 2026-10-26, after `gemma4:12b`
            // left it out of 39 answers in 177, mostly where it changed nothing.
            let hits = Self::review_hits(&memory, &lists, &always_fine, &authors);
            match route(kind, &answer.sentences, &lists, &hits) {
                Decision::Clear => to_clear.push((memory.id, sum)),
                Decision::Flag(reasons) => flagged.push((memory, reasons)),
            }
        }
        let mut by_person = Vec::new();
        for id in person_cleared {
            if !session.flagged.contains(id) {
                refused.push(json!({"id": id.to_string(), "why": "only a memory this review put to the person can be cleared by them"}));
                continue;
            }
            if let Some(memory) = store.get(*id).map_err(|error| error.to_string())? {
                by_person.push((memory.id, crate::checksum::of_memory(&memory)));
            }
        }

        let cleared_ids: Vec<String> = to_clear
            .iter()
            .chain(&by_person)
            .map(|(id, _)| id.to_string())
            .collect();
        let store = self.store_for_mut(scope)?;
        store
            .record_clearances(&to_clear)
            .and_then(|()| store.record_clearances(&by_person))
            .map_err(|error| error.to_string())?;
        let session = self.reviews.entry(scope.clone()).or_default();
        for (id, _) in &by_person {
            session.flagged.remove(id);
        }
        for (memory, _) in &flagged {
            session.flagged.insert(memory.id);
        }
        let destination = session.destination.clone();
        let awaiting = session.flagged.len();

        let published = match (kind, &destination) {
            (SetKind::Shared, Some(destination)) => self.published_records(scope, destination)?,
            _ => std::collections::HashMap::new(),
        };
        let remaining = self.unreviewed(scope, &published)?.len();
        let finished = remaining == 0;
        if finished {
            self.store_for_mut(scope)?
                .set_review_setting(ReviewSetting::FullReview, None)
                .map_err(|error| error.to_string())?;
        }

        let mut result = json!({
            "cleared": cleared_ids,
            "flagged": flagged.iter().map(|(memory, reasons)| json!({
                "id": memory.id.to_string(),
                "reasons": reasons,
                "content": memory.content.as_data(),
                "why": memory.why.as_ref().map(MemoryText::as_data),
                "context": memory.context.as_ref().map(MemoryText::as_data),
            })).collect::<Vec<_>>(),
            "remaining": remaining,
            "awaiting_the_person": awaiting,
            "finished": finished,
        });
        if !refused.is_empty() {
            result["refused"] = json!(refused);
        }
        if !finished {
            result["not_finished"] = json!(format!(
                "The review is not finished: {remaining} memories still wait, so it must not be \
                 reported as complete. Run `review` again for the next memories."
            ));
        }
        if !flagged.is_empty() {
            result["flag_guidance"] = json!(
                "Show the person each flagged memory in full, with its reasons. If they want it kept \
                 out, forget it with hoard_correct; if reworded, store a restatement that supersedes \
                 it and then forget it; if they say it is fine, pass its id in `person_cleared`. \
                 If a rewording replaces a name with a pseudonym, suggest the person add the \
                 pseudonym under *Always fine to share* in their sharing preferences, so later \
                 reviews do not flag it."
            );
        }
        Ok(result)
    }

    /// `hoard_sets` action `backup`: write the whole set where the machine's configuration says.
    ///
    /// `documents/design/export-review.md`, *a backup is its own action*. **Never gated**, because a
    /// safety copy refused when nobody has reviewed the set yet fails exactly when it is needed. The
    /// header marks the file as a backup, so its memories never count as reviewed if it is later put
    /// where the set is published. Nothing is recorded against the file: a backup is a snapshot, not a
    /// copy that reconciles. Each response says how many backups the set now has, and pruning is left
    /// to the person, since a safety mechanism should not delete copies on its own.
    fn backup_set(&mut self, scope: &Scope) -> Result<Value, String> {
        let directory = match (&self.backup_directory, &self.layout) {
            (Some(directory), _) => directory.clone(),
            (None, Some(layout)) => layout.common().join("backups"),
            (None, None) => {
                return Err(
                    "this server has no data directory, so it has nowhere to back up to".to_owned(),
                );
            }
        };
        let folder = directory.join(match scope {
            Scope::Local => "local".to_owned(),
            Scope::Shared(name) => name.as_str().to_owned(),
        });
        std::fs::create_dir_all(&folder)
            .map_err(|error| format!("could not create {}: {error}", folder.display()))?;
        let store = self
            .stores
            .get(scope)
            .ok_or_else(|| format!("no set {} is loaded", scope.as_key()))?;
        let set_id = store.set_id().map_err(|error| error.to_string())?;
        let mut body: Vec<u8> = Vec::new();
        memory_file::write_header_marked(scope, set_id, true, &mut body)
            .map_err(|error| error.to_string())?;
        let written = store
            .stream_records(|record| {
                memory_file::write_record(record, &mut body).expect("writing to a Vec cannot fail");
                Ok(())
            })
            .map_err(|error| format!("could not read the set: {error}"))?;
        let stamp = jiff::Timestamp::now()
            .strftime("%Y-%m-%dT%H%M%SZ")
            .to_string();
        let path = folder.join(format!("{stamp}.json"));
        let mut temporary = path.clone().into_os_string();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, &body)
            .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, &path)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        let held = std::fs::read_dir(&folder)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.path().extension().is_some_and(|e| e == "json"))
                    .count()
            })
            .unwrap_or(0);
        Ok(json!({
            "backed_up": path.display().to_string(),
            "records": written,
            "backups_held": held,
            "note": "A backup is never reviewed and is not a publication. Copying it off this \
                     machine is the person's own choice; older backups are removed by hand.",
        }))
    }

    /// Why an export of `scope` to `destination` must wait for the export review, if it must.
    ///
    /// `documents/design/export-review.md`, *the file on disk is the clearance record*. `local` is
    /// never gated; a set the person never publishes is refused outright; any other shared set is
    /// refused while it holds memories nobody has reviewed.
    fn export_review_refusal(
        &self,
        scope: &Scope,
        destination: &Destination,
    ) -> Result<Option<String>, String> {
        // A read-only set is someone else's published material, taken from their file and never
        // written here, so there is nothing of this person's in it to review.
        if self
            .stores
            .get(scope)
            .is_some_and(|store| matches!(store.load_mode(), Ok(LoadMode::ReadOnly)))
        {
            return Ok(None);
        }
        let lists = ListedPreferences::parse(&self.person_preferences_text());
        match Self::review_kind(scope, &lists) {
            SetKind::Local => Ok(None),
            SetKind::NeverPublished => Ok(Some(format!(
                "Nothing was exported: the person's sharing preferences list {} under *Sets I never \
                 publish*. It is reviewed like `local` and never exported. To keep a copy, use \
                 hoard_sets action `backup`.",
                scope.as_key()
            ))),
            SetKind::Shared => {
                let published = self.published_records(scope, destination)?;
                let waiting = self.unreviewed(scope, &published)?;
                if waiting.is_empty() {
                    return Ok(None);
                }
                let named: Vec<String> = waiting
                    .iter()
                    .take(8)
                    .map(|memory| memory.id.to_string())
                    .collect();
                Ok(Some(format!(
                    "Nothing was exported: {} memories in {} have not been through the export review, \
                     so export will not write them where the set is published ({}{}). Run hoard_sets \
                     action `review` for this set with the same `{}`, answer what it shows with \
                     `clear`, and export again. To keep a safety copy meanwhile, use `backup`.",
                    waiting.len(),
                    scope.as_key(),
                    named.join(", "),
                    if waiting.len() > named.len() {
                        ", ..."
                    } else {
                        ""
                    },
                    destination.form(),
                )))
            }
        }
    }

    fn export_set(&mut self, scope: &Scope, destination: &Destination) -> Result<Value, String> {
        if !self.stores.contains_key(scope) {
            return Err(missing_scope(&self.withheld, scope));
        }
        if let Some(refusal) = destination.refusal() {
            return Err(refusal);
        }
        if let Some(refusal) = self.export_review_refusal(scope, destination)? {
            return Err(refusal);
        }

        // Embedded first, so the description written into the file covers every memory in it rather
        // than whichever happened to have vectors. Every read and write runs the same catch-up; an
        // export is a read that had simply never asked for it.
        self.stores
            .get_mut(scope)
            .expect("just checked")
            .catch_up_vectors()
            .map_err(|error| format!("could not embed the set before describing it: {error}"))?;

        let store = self.stores.get(scope).expect("just checked");
        let known = store.known_targets().map_err(|error| error.to_string())?;
        let author = store.author_display().map_err(|error| {
            format!("could not read who this set attributes writes to: {error}")
        })?;
        let target = Store::normalise_target(&destination.target());

        // Serialise into memory first, then write once. The set is streamed out of the store rather
        // than collected as `Memory` values, so what is held is the text plus one checksum per
        // record — and writing only after every record has serialised means a failure partway leaves
        // no half-file at the destination.
        let mut bases: Vec<(MemoryId, u64)> = Vec::new();
        let mut prohibited_found: Vec<Value> = Vec::new();
        // **A set leaving this machine for the first time is given its identity here**, since
        // `set-settled-at-first-write`: a copy made without one would be taken on elsewhere by an
        // import that mints its own, and the two could never exchange records again. Proposed rather
        // than minted, so a refused export leaves the set as it found it -- an identity minted and
        // then refused would turn the import the refusal advises into a mismatch.
        let identity = store
            .set_id()
            .map_err(|error| format!("could not read this set's identity: {error}"))?;
        let declaring = identity.unwrap_or_else(SetId::generate);
        let rendered = Self::serialise_set_as(store, scope, Some(declaring), |record, line| {
            bases.push((record.id(), crate::checksum::of_record(record)));
            // Scanned as the serialised record rather than field by field, so a field added later
            // is covered without anyone remembering to add it here. Safe because every rule keys on
            // shape: the surrounding JSON contributes no prefixes, and the live sets scan clean
            // this way.
            let mut categories: Vec<String> = prohibited::scan(line)
                .into_iter()
                .map(|finding| serde_name(&finding.category))
                .collect();
            categories.sort_unstable();
            categories.dedup();
            for category in categories {
                prohibited_found.push(json!({
                    "memory": record.id().to_string(),
                    "category": category,
                }));
            }
        })?;

        let path = destination.file_to_write(scope, identity)?;

        // Read before the write, because the write is what destroys the file being compared against.
        let replaced = Replaced::find(&path, scope, &bases);
        // Identity first: the refusal below advises importing from this destination, and with the
        // identities apart that import is refused too -- advice leading only to another refusal.
        // Every identity the set holds is defended, records or not, since
        // `set-settled-at-first-write`: an import no longer replaces the identity an empty set holds.
        if let Some(refusal) = replaced.identity_refusal(&path, scope, identity) {
            return Err(refusal);
        }
        if let Some(refusal) = replaced.refusal(&path) {
            return Err(refusal);
        }
        // **A set with no identity meeting a file that declares one** has never been written to or
        // imported, and the file is very likely the set it was created to take on. Writing over it
        // would replace that identity with a new one, forking every copy taken from the file -- so
        // it is told to import, which is the step it skipped, and the import gives it the file's.
        if identity.is_none()
            && let Some(declared) = replaced.declared()
        {
            return Err(format!(
                "nothing was written: {} declares the set identity {declared}, and {} has none yet, \
                 having never been written to or imported. Exporting over the file would give it a \
                 new identity and fork every copy taken from it. Import from this destination first, \
                 which gives the set the file's identity, then export.",
                path.display(),
                json!(WireScope(scope))
            ));
        }
        if identity.is_none() {
            let adopted = store
                .adopt_set_id(declaring)
                .map_err(|error| format!("could not give the set an identity: {error}"))?;
            if !adopted {
                return Err(format!(
                    "{} was given an identity by another process while this export was being \
                     written, so nothing was written. Export again.",
                    json!(WireScope(scope))
                ));
            }
        }
        let path = Self::write_memory_file(destination, &path, &rendered.body)?;

        let store = self.stores.get_mut(scope).expect("checked above");
        store
            .set_bases(&target, &bases)
            .map_err(|error| format!("wrote {} but could not record the agreement: {error}. The next import will treat every record as changed.", path.display()))?;

        // Three numbers rather than one overloaded one. `written` is the count of *records*, which
        // is memories plus tombstones -- reporting that as `memories` is what made `status` and
        // `export` disagree about a name they shared, at the export gate of all places.
        // Both asked of the store rather than one derived from the other by subtraction: a
        // derived number cannot disagree with its source, so it could never report a real
        // inconsistency -- and being able to see one from a single response is the point.
        let live = store
            .count()
            .map_err(|error| format!("wrote {} but could not count it: {error}", path.display()))?;
        let identities = store.identity_count().map_err(|error| {
            format!(
                "wrote {} but could not count its identity records: {error}",
                path.display()
            )
        })?;
        let forgotten = store.forgotten_count().map_err(|error| {
            format!(
                "wrote {} but could not count what it forgot: {error}",
                path.display()
            )
        })?;
        // Shown at the gate the whole point of deriving it: this is the moment the whole set is in
        // view and the moment it becomes portable. It describes and does not judge -- see
        // `documents/design/golden-rule.md`, demand C.
        let described = rendered.summary.as_ref().map(describe);

        // **This export discharges a description an import still owed -- but only if it showed the
        // whole set.** An export with no model describes nothing, and one whose model does not match
        // the set's describes only the memories embedded before the mismatch; either would retire
        // the report without the description it stands in for ever having been shown. Asked as
        // coverage alone, because a set with nothing left unembedded has been described in full.
        let awaiting = store.awaiting_vectors().map_err(|error| {
            format!(
                "wrote {} but could not tell whether the set is embedded: {error}",
                path.display()
            )
        })?;
        if awaiting == 0 {
            store.discharge_description().map_err(|error| {
                format!(
                    "wrote {} but could not record that the set was described: {error}",
                    path.display()
                )
            })?;
        }

        let mut result = json!({
            "written": path.to_string_lossy(),
            // What this set amounts to, derived from its own contents rather than from what its
            // charter claims. Read it against the charter: they answer the same question from
            // different sides, and a set that has drifted from what it says it is for is the common
            // case this catches. The rarer one is the reason it exists.
            "describes": described,
            // Which of the two destination forms the caller used. `written` alone would leave a
            // reader to infer it from the path's shape, and an inference is not a statement -- a
            // set written where nothing will auto-discover it should say so rather than be
            // deducible.
            "path_form": destination.form(),
            "memories": live,
            "forgotten": forgotten,
            // Named rather than folded into `memories`, so `records` still reconciles as
            // `memories + forgotten + identities` from one response -- and so a person reading an
            // export report can see that the set's identity record travelled with it.
            "identities": identities,
            "records": rendered.written,
            // What this export did to the file it replaced: which records are new to it, which
            // differ, and which it dropped. One store serves every session on a machine, so another
            // session's work reaches this export with nothing marking it -- this is what marks it.
            // `export-reports-what-the-file-did-not-hold`.
            "replaced_file": replaced.report(),
            // Export is where attribution stops being private, and therefore the last cheap moment
            // to notice it is wrong -- `documents/design/store-registry.md`. Stated on every export
            // for the same reason `memorize-says-nothing-about-searchability` was filed: put the
            // fact where the caller can still act on it.
            "author": author.as_ref().map(Author::as_str),
            // A fact about the export that just ran, not a promise about the file. The scan is
            // mechanical and narrow by design — known vendor key prefixes, Luhn-passing digit
            // runs, PEM armour — so `true` means *those shapes were looked for*, which the note
            // below spells out. Passwords have no shape and are not covered at all.
            "prohibited_data_scanned": true,
            // **`false` is true here for two unrelated reasons and the boolean cannot tell them
            // apart**, which `set-lifecycle.md` anticipated: *"the response should say the gate was
            // skipped and why, so the difference stays visible where it matters -- in the result,
            // not in the surface."* A local export is never destined for a repository, so the gate
            // does not apply to it at all; a shared one is subject to the gate and skipped it only
            // because no scan exists yet. Conflating those matters more once the scan is built,
            // when a shared export that genuinely went unscanned would otherwise be
            // indistinguishable from a local one that never needed scanning.
            // `prohibited-data-scanned-conflates-skipped-with-unbuilt`.
            // **Not a clean bill of health, and worded so it cannot be read as one.** The
            // failure mode of a shape-based scan is misses rather than false alarms: an unknown
            // vendor's key format is not recognised, and a password cannot be recognised at all.
            // Saying only "scanned" would invite exactly the confidence the scan cannot support --
            // the same reason a link check that did not run reports that rather than staying
            // silent.
            "prohibited_data_note": if scope.exports_to().is_none() {
                "Scanned, though this set is local: it is written here as a backup rather than \
                 published to a repository, so the export-review gate does not apply to it. The \
                 scan matches shapes -- known vendor key prefixes, digit runs passing Luhn, PEM \
                 armour -- and not wording, so it finds neither an unfamiliar key format nor a \
                 password, which has no shape to match. Clean here means those shapes are absent, \
                 not that the set holds no secret."
            } else {
                "Scanned for shapes -- known vendor key prefixes, digit runs passing Luhn, PEM \
                 armour -- and not for wording, so writing about credentials is fine and only \
                 something formatted like one is reported. It finds neither an unfamiliar key \
                 format nor a password, which has no shape to match, so clean means those shapes \
                 are absent rather than that the set holds no secret. The gate's other half is \
                 the review in export-review.md, which runs before an export rather than inside it."
            },
        });

        // **Reported, never a refusal**, which is the decision this whole check turns on. A set can
        // hold memories that arrived by import, and `imported-memories-carry-no-arrival-provenance`
        // means the exporter cannot tell which those are -- so refusing would ask the caller to
        // amend or forget somebody else's record to unblock their own work. The scan also gains
        // rules over time, so refusing here would let a filter improvement close a set that
        // exported fine the day before, taking every memory written since with it. The write is
        // where refusal belongs: one memory, not yet stored, definitionally the caller's.
        if !prohibited_found.is_empty() {
            result["prohibited_data_found"] = json!(prohibited_found);
            result["prohibited_data_guidance"] = json!(
                "These memories carry text shaped like a credential, and the file has been \
                 written -- do not commit it until they are dealt with. Read each with \
                 `hoard_lookup`. If it is a real secret, treat it as compromised and rotate it, \
                 then `hoard_correct`'s forget removes the memory and writes a tombstone that \
                 reaches other copies on the next export; a memory that merely quotes something \
                 harmless can be reworded with a supersede. Nothing was blocked, because a set can \
                 hold memories that arrived from elsewhere and are not yours to rewrite."
            );
        }

        // A second checkout is legitimate and a typo is not, and only the caller can tell which — so
        // this is a remark, never a refusal.
        if !known.is_empty() && !known.contains(&target) {
            result["note"] = json!(format!(
                "this set has been exported to {} before; you named a directory it has not been \
                 written to. A second checkout is expected; a mistyped path is not.",
                known.join(", ")
            ));
        }

        Ok(result)
    }

    /// Reconcile a file into a store, keeping local work and surfacing genuine conflicts.
    ///
    /// Two streaming passes, in the order `documents/design/file-format.md` requires: validate the
    /// whole file first, then apply. A single pass that wrote as it read would journal work it might
    /// have to discard, and would open a window in which the store was half-updated from a file that
    /// turns out to be malformed on its last line.
    ///
    /// The reconciliation itself is [`crate::reconcile::resolve`], and the base it needs is keyed by
    /// this same directory — so two checkouts of one repository reconcile independently against the
    /// one store they share.
    ///
    /// **A conflict is reported, never resolved.** Last-writer-wins would silently discard one side's
    /// work, and a conflict here is exactly what `git` would have raised had both versions reached
    /// one file. Nothing is written for a conflicted record, and the response names it.
    /// `evaluate` -- read a file for review without adopting it.
    ///
    /// **No `create` step, and no store on disk.** Nothing is being taken on: the records go into an
    /// in-memory store under a name [`SetName::evaluation`] builds from the file's own set name and
    /// the caller's label, which is why this reads the file's header rather than being told which set
    /// to expect. The set the file names is untouched, and so is every other set here.
    ///
    /// The store adopts the file's identity, so a link naming that set can be reported against it --
    /// but an evaluation load is never a holder of it, which `settle_identity` enforces by leaving
    /// evaluation stores out.
    fn evaluate_set(
        &mut self,
        label: &str,
        source: &Destination,
        set: Option<&SetName>,
    ) -> Result<Value, String> {
        if let Some(refusal) = source.refusal() {
            return Err(refusal);
        }
        let started = std::time::Instant::now();
        let source_path = source.file_of_named(set)?;
        let file = std::fs::File::open(&source_path)
            .map_err(|error| format!("could not open {}: {error}", source_path.display()))?;
        // `AnyShared`: the file says which set it is, because nothing here is bound to that name --
        // the store this makes is named after the *review*, not after the set. Narrowed to exactly
        // the named set when the caller said which.
        let set = memory_file::read(&named_or_any(set), file)
            .map_err(|error| format!("{} is not usable: {error}", source_path.display()))?;
        let Scope::Shared(declared) = set.scope.clone() else {
            return Err(format!(
                "{} declares the local scope, which no file may: a local set is never published.",
                source_path.display()
            ));
        };
        let Some(identity) = set.set_id else {
            return Err(format!(
                "{} carries no set identity, so its memories cannot be told apart from another \
                 copy's. It was written before identities existed; the set it came from can give it \
                 one by exporting again.",
                source_path.display()
            ));
        };

        let named = SetName::evaluation(&declared, label)
            .map_err(|error| format!("{label:?} cannot name an evaluation load: {error}"))?;
        let scope = Scope::Shared(named);
        if self.stores.contains_key(&scope) || self.withheld.contains_key(&scope) {
            return Err(format!(
                "a copy of {declared} is already loaded for review under the label {label:?}. Give \
                 this one a label of its own, or drop that one first -- `hoard_sets` with action \
                 \"status\" lists what is loaded."
            ));
        }
        if let Scope::Shared(named) = &scope
            && let Some(held) = self.case_variant_of(named)
        {
            return Err(format!(
                "this machine already holds {:?}, and a review labelled {label:?} would differ from \
                 it only in case. Names that differ only in case would share one database file on \
                 macOS and Windows, so give this review a label that differs by more than case, or \
                 drop that one first.",
                held.as_str()
            ));
        }

        let layout = self.layout.as_ref().ok_or(
            "this server has no data directory, so it has nowhere to put a copy loaded for review. \
             That is startup wiring, not something a caller can supply.",
        )?;
        // **On disk, under the home.** A review copy is someone else's whole set, and a machine
        // running a model locally should not be asked to hold it in RAM beside them. The home is the
        // directory the user chose for stores and so is known to be sized for one;
        // `StoreLayout::reviews` says why not the system temp directory.
        let directory = layout.reviews();
        std::fs::create_dir_all(&directory).map_err(|error| {
            format!(
                "could not make a place for the review at {}: {error}",
                directory.display()
            )
        })?;
        let path = Store::path_for(&scope, &directory);
        // **A file already here is another server's review, or this one's from before a restart that
        // has not loaded it.** Either way the label is taken: opening it would put two servers on one
        // database, and `choose_evaluation` would refuse a store that already holds records anyway.
        if path.exists() {
            return Err(format!(
                "a copy of {declared} is already held for review under the label {label:?}, at {}. \
                 Give this one a label of its own, or end that review with `hoard_sets` action \
                 \"drop\".",
                path.display()
            ));
        }
        let store = Store::open_at(scope.clone(), &path)
            .map_err(|error| format!("could not open a store for the review: {error}"))?;
        store
            .choose_evaluation()
            .map_err(|error| format!("could not hold the copy for review: {error}"))?;
        store
            .adopt_set_id(identity)
            .map_err(|error| format!("could not record which set this is a copy of: {error}"))?;
        // Through `add_store`, so the review's store is embedded by this server's model like any
        // other -- a copy that cannot be searched by meaning would answer a review with nothing.
        self.add_store(store);

        let store = self
            .stores
            .get_mut(&scope)
            .ok_or_else(|| "the review's store did not load".to_owned())?;
        // One record at a time through the file door, rather than `Store::import`, which refuses a
        // file whose scope is not the store's -- and here it deliberately is not.
        for memory in &set.memories {
            store
                .accept_from_file(memory)
                .map_err(|error| format!("could not read {} for review: {error}", memory.id))?;
        }
        for tombstone in &set.tombstones {
            store
                .accept_tombstone(tombstone)
                .map_err(|error| format!("could not read a tombstone for review: {error}"))?;
        }

        Ok(json!({
            "loaded": WireScope(&scope),
            "copy_of": WireScope(&set.scope),
            "set_id": identity.to_string(),
            "memories": set.memories.len(),
            "forgotten": set.tombstones.len(),
            "read": source_path.to_string_lossy(),
            // Said rather than left to be found: the copy is a file under the home, loaded again at
            // the next start, and `drop` is what removes it.
            "held_at": path.to_string_lossy(),
            // **What the review cost on this machine.** An exported file carries no vectors, so this
            // embeds every memory before returning -- measured 2026-09-18 at about 0.7 ms each in
            // release and 6 ms in debug, which is why no size limit was built: the *looks like a
            // hang* failure needs a corpus a hundred times this project's. Reported rather than
            // capped, so the cost is visible on hardware nobody here can test.
            "took_ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "note": "Loaded for review and adopted by nothing. It is searched only when a recall \
                     names its scope, every memory it returns is marked as coming from a copy under \
                     review, and it lives in memory: dropping it, or restarting, ends the review \
                     with nothing saved and nothing changed in the set it copies.",
        }))
    }

    fn import_set(&mut self, scope: &Scope, source: &Destination) -> Result<Value, String> {
        if !self.stores.contains_key(scope) {
            return Err(missing_scope(&self.withheld, scope));
        }

        // Before the file is looked for: otherwise the failure reads as a missing memory file, which
        // sends the caller looking for the wrong thing entirely.
        if let Some(refusal) = source.refusal() {
            return Err(refusal);
        }

        let identity = self
            .stores
            .get(scope)
            .expect("just checked")
            .set_id()
            .map_err(|error| format!("could not read this set's identity: {error}"))?;
        let path = source.file_to_read(scope, identity)?;
        let expectation = memory_file::Expectation::Exactly(scope.clone());

        // Pass one. Holds the id set and nothing else, and a failure here leaves the store untouched.
        let validated = {
            let file = std::fs::File::open(&path)
                .map_err(|error| format!("could not open {}: {error}", path.display()))?;
            memory_file::validate(&expectation, file)
                .map_err(|error| format!("{} is not usable: {error}", path.display()))?
        };

        let target = Store::normalise_target(&source.target());
        let store = self.stores.get(scope).expect("just checked");

        // **Adopted before the records are reconciled, and it is not part of that reconciliation.**
        // An identity is a fact about the set rather than about any record in it, so it neither
        // takes part in the three-way decision nor depends on its outcome -- a file that turns out
        // to conflict on every memory still tells this store who it is. Filling an absence only:
        // `Store::adopt_set_id` will not displace an identity already held, and a mismatch is
        // reported by neither, because what set identity *gates* is an open question and a refusal
        // invented here would answer it by accident.
        // **Refused before anything is written** when another store answers to the file's identity:
        // adopting it would leave two sets that one identity resolves to, and whichever sorted first
        // would receive every link meant for the other (`two-sets-can-share-an-identity`).
        if let Some(offered) = validated.set_id
            && store.set_id().ok().flatten() != Some(offered)
            && let Some(holder) = self.other_holder_of(scope, offered)
        {
            return Err(format!(
                "{} belongs to the set identity {offered}, which {} already answers to on this \
                 machine. Importing it into {} would leave two sets with one identity. If this file \
                 is {} under another name, import it there instead. If it is a fork renamed by \
                 editing its header, the edit left the identity unchanged: the fork's renaming \
                 machine should delete `set_id` from the header as well, `create` the fork, import \
                 the edited copy and export it, and commit that export rather than the edited copy \
                 (`documents/design/set-lifecycle.md`, under renaming a fork).",
                path.display(),
                json!(WireScope(&holder)),
                json!(WireScope(scope)),
                json!(WireScope(&holder)),
            ));
        }
        let adopted = match validated.set_id {
            Some(offered) => store
                .adopt_set_id(offered)
                .map_err(|error| format!("could not record the set identity: {error}"))?
                .then(|| offered.to_string()),
            None => None,
        };
        // **Refused, since 2026-09-26**, where it had been reported and imported anyway
        // (`identity-mismatch-refusal`). Asked after adoption because adoption is what decides
        // whether there is a mismatch at all: a set with no records takes the file's identity, which
        // is `create` then `import` working as designed. Still differing afterwards means the set
        // kept its own because it has a history -- and adoption wrote nothing in that case, so
        // refusing here leaves the store exactly as it was.
        if let Some(declared) = validated.set_id
            && let Some(held) = store.set_id().map_err(|error| error.to_string())?
            && declared != held
        {
            return Err(identity_mismatch(
                &path,
                scope,
                declared,
                held,
                Crossing::Import,
            ));
        }
        // After adoption and before reconciling: a set with no identity yet -- one just created,
        // taking a file that declares none -- claims one here rather than at its first memory, so
        // tombstones applied ahead of any memory are held under it too. Only for a file with records
        // to write: an empty one is not the set's first write, and settles nothing.
        let held_before = store.set_id().map_err(|error| error.to_string())?;
        let held = if validated.ids.is_empty() {
            held_before
        } else {
            Some(
                store
                    .mint_set_id()
                    .map_err(|error| format!("could not give the set an identity: {error}"))?,
            )
        };
        // **Said when the file holds records and declares no identity**, the one disagreement about
        // who the set is that is not refused above: there is nothing to compare against.
        let identity_note = Self::identity_note_for_import(
            &path,
            scope,
            validated.set_id,
            held,
            held_before.is_none(),
            !validated.ids.is_empty(),
        );
        let known = store.known_targets().map_err(|error| error.to_string())?;
        let bases = store
            .bases_for(&target)
            .map_err(|error| format!("could not read the recorded agreement: {error}"))?;

        // The store's side of each decision, taken before anything is written so that every outcome
        // is judged against the same starting state.
        //
        // Streamed as *records*, so a tombstone counts as something the store holds. Reading only the
        // memory rows would make a memory this machine has already forgotten look absent, and the
        // matching tombstone in the file would then read as a record that vanished with no explanation
        // — an anomaly reported on every import, for a set that is perfectly in agreement.
        // The checksum decides; the timestamp only ever reports. See the note on the backwards check
        // below, and `documents/design/multi-set-sync.md` for why the clock never chooses a winner.
        let mut stored: std::collections::HashMap<MemoryId, (u64, jiff::Timestamp)> =
            std::collections::HashMap::new();
        // Which of them are tombstones, because a memory never replaces one at import: see
        // `reconcile::resolve_memory`.
        let mut forgotten_here: std::collections::HashSet<MemoryId> =
            std::collections::HashSet::new();
        store
            .stream_records(|record| {
                if matches!(record, memory_file::Record::Tombstone(_)) {
                    forgotten_here.insert(record.id());
                }
                stored.insert(
                    record.id(),
                    (crate::checksum::of_record(record), record.as_of()),
                );
                Ok(())
            })
            .map_err(|error| format!("could not read the store: {error}"))?;

        let in_file: std::collections::HashSet<MemoryId> = validated.ids.iter().copied().collect();

        // Pass two. Decisions are collected rather than applied inside the visitor, because applying
        // needs `&mut` on the store the visitor is reading from.
        let mut decisions: Vec<(memory_file::Record, reconcile::Outcome, u64)> = Vec::new();
        let mut backwards: Vec<String> = Vec::new();
        {
            let file = std::fs::File::open(&path)
                .map_err(|error| format!("could not reopen {}: {error}", path.display()))?;
            memory_file::stream(&expectation, file, |record| {
                let checksum = crate::checksum::of_record(record);
                let held = stored.get(&record.id()).copied();
                let base = bases.get(&record.id()).copied();
                let held_checksum = held.map(|(checksum, _)| checksum);
                // A tombstone can say which version it removed, which is the only thing that
                // answers a forget arriving with no ancestor to judge it by. Every other record
                // goes through the plain three-way decision.
                let outcome = match record {
                    memory_file::Record::Tombstone(tombstone) => reconcile::resolve_tombstone(
                        checksum,
                        tombstone.forgot,
                        base,
                        held_checksum,
                    ),
                    memory_file::Record::Memory(_) => reconcile::resolve_memory(
                        checksum,
                        base,
                        held_checksum,
                        forgotten_here.contains(&record.id()),
                    ),
                };
                // **A `TakeFile` whose incoming version is older than the one it replaces is never
                // what a pull means**, because every real edit bumps `modified` and forgetting
                // stamps its own moment. The reverse direction says the file is not a successor to
                // what this store holds but a *predecessor*, and taking it is a regression to
                // superseded content: `an-older-file-silently-resurrects-a-forgotten-memory`.
                //
                // Judged here rather than in `reconcile`, and deliberately: `resolve` decides by
                // checksum alone and stays a pure function of three checksums, which is what keeps
                // the clock out of the arithmetic. This changes nothing about what is written.
                //
                // It reads `as_of` over both record shapes, so a resurrection compares like any
                // other edit — the memory coming back is older than the tombstone that replaced it.
                if outcome == reconcile::Outcome::TakeFile
                    && let Some((_, held_at)) = held
                    && record.as_of() < held_at
                {
                    backwards.push(record.id().to_string());
                }
                decisions.push((record.clone(), outcome, checksum));
                Ok(())
            })
            .map_err(|error| format!("{} became unreadable: {error}", path.display()))?;
        }

        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        let mut conflicts: Vec<String> = Vec::new();
        let mut anomalies: Vec<String> = Vec::new();
        let mut kept_forgotten: Vec<String> = Vec::new();
        let mut agreed: Vec<(MemoryId, u64)> = Vec::new();
        let mut written: Vec<MemoryId> = Vec::new();

        let store = self.stores.get_mut(scope).expect("checked above");
        for (record, outcome, checksum) in decisions {
            *counts.entry(outcome_name(outcome)).or_default() += 1;
            let id = record.id();
            match outcome {
                reconcile::Outcome::TakeFile | reconcile::Outcome::NewFromFile => {
                    // A tombstone arriving from the file forgets the memory here too. That is how
                    // forgetting propagates: as an ordinary change to a record, through the same
                    // three-way decision as any other, rather than down a path of its own.
                    //
                    // **The record is written exactly as the file gives it, attribution included.**
                    // Stamping it with this server's identity -- or with the store's -- is a
                    // plausible-looking one-line change, since that is what `memorize` and `amend`
                    // correctly do, and it would be wrong twice: it would rewrite the authorship of
                    // everyone else's memories on every pull, and it would close the only route
                    // there is to re-attributing memories in bulk. That route is
                    // export-edit-import, and it is the sanctioned procedure precisely because it
                    // puts a human at a judgement software cannot make --
                    // `documents/design/store-registry.md`, *re-attributing existing memories*.
                    // Guarded by two tests in `tests/sets.rs` and by a recorded mutation.
                    match &record {
                        // `accept_from_file`, not `insert`: the cycle refusal on `insert` is right
                        // for a caller authoring one edge and wrong here, where it lands on
                        // whichever record a merge's cycle happened to close on and aborts the
                        // import with the store already partly written.
                        // `two-import-paths-disagree-about-supersession-cycles`.
                        memory_file::Record::Memory(memory) => {
                            // Written exactly as the file gives it. There is no receiver-side
                            // rewrite left on this path: absorbing a merged-away set's links is
                            // `hoard_sets` action "absorb", which authors a supersession instead.
                            store
                                .accept_from_file(memory)
                                .map_err(|error| format!("could not store memory {id}: {error}"))?;
                        }
                        // `accept_tombstone`, not `forget`: the arriving record carries the
                        // ancestry of the chain it was part of, and deriving it from this store
                        // instead would derive nothing on the machine that has never held the
                        // memory -- which is every machine adopting the set.
                        memory_file::Record::Tombstone(tombstone) => store
                            .accept_tombstone(tombstone)
                            .map_err(|error| format!("could not forget memory {id}: {error}"))?,
                    }
                    written.push(id);
                    agreed.push((id, checksum));
                }
                // Already identical on both sides, so nothing to write — but the agreement is now
                // established and recording it is what stops it being re-decided every import.
                reconcile::Outcome::Converged | reconcile::Outcome::Unchanged => {
                    agreed.push((id, checksum));
                }
                // No agreement, so the base must not move: it still records the last point the two
                // sides genuinely matched, which is what the next import needs.
                reconcile::Outcome::KeepLocal => {}
                // Nothing written and the base left where it was, as for `KeepLocal`: the two sides
                // do not agree, and the file will say the same thing next time until it moves on.
                reconcile::Outcome::KeptForgotten => kept_forgotten.push(id.to_string()),
                reconcile::Outcome::Conflict => conflicts.push(id.to_string()),
                reconcile::Outcome::Anomaly => anomalies.push(id.to_string()),
            }
        }

        // Records the store holds and the file does not. Absence means nothing on its own, which is
        // why forgetting is designed to write a tombstone — so this is either local work awaiting
        // export, or an anomaly if the two sides had previously agreed the record existed.
        for id in stored.keys() {
            if in_file.contains(id) {
                continue;
            }
            let outcome = reconcile::resolve_absent_from_file(bases.get(id).copied());
            *counts.entry(outcome_name(outcome)).or_default() += 1;
            if outcome == reconcile::Outcome::Anomaly {
                anomalies.push(id.to_string());
            }
        }

        // Asked of the store after every write, about the records this import wrote. A cycle is
        // reachable no other way now that the write no longer refuses one, and a stored cycle
        // nobody is told about is the silent half of the same defect: resolution reports such a
        // memory as `supersession_cycle` at recall, which is a strange thing to meet with no idea
        // where it came from. Scanning only what was written keeps the cost proportional to the
        // import rather than to the store.
        let mut cycles: Vec<String> = Vec::new();
        for id in written {
            match store.resolve_supersession(id) {
                Ok(Resolution::Cycle) => cycles.push(id.to_string()),
                Ok(_) => {}
                Err(error) => {
                    return Err(format!("imported, but could not resolve {id}: {error}"));
                }
            }
        }

        // Only the records that now agree. Written as a merge into the existing bases rather than a
        // replacement, because a conflicted or locally-changed record keeps the base it had.
        store
            .merge_bases(&target, &agreed)
            .map_err(|error| format!("imported, but could not record the agreement: {error}"))?;

        // **Derived here, from the memories this store now holds.** A file carries no description of
        // its own -- one it carried could be edited before it was shipped -- and one an older build
        // wrote is ignored on read. The file carries memories but no vectors, so the derivation is
        // only possible once they have been embedded here, and it describes the *result of the
        // merge*: the set as it now stands, which is what a session will be working with.
        //
        // **And where it cannot be recomputed, it is owed rather than skipped.** A memory with no
        // vector has no place in the space the description clusters, so while any arrived memory
        // lacks one -- no model here, or a model that does not match the set's -- a description
        // would silently leave the import out. Skipping it would let the gate be defeated by
        // importing first and installing the model afterwards, so the store records what is owed
        // and `hoard_conventions` pays it once the memories are embedded. The condition is coverage
        // rather than a missing model, because a mismatched model leaves exactly the same gap.
        let store = self.stores.get(scope).expect("checked at entry");
        let awaiting = store.awaiting_vectors().map_err(|error| {
            format!("imported, but could not tell whether the set is embedded: {error}")
        })?;
        let (describes, description_owed) = if awaiting > 0 {
            let record =
                serde_json::to_string(&OwedDescription {}).map_err(|error| error.to_string())?;
            store.owe_description(&record).map_err(|error| {
                format!(
                    "imported, but could not record that the set is owed a description: {error}"
                )
            })?;
            (
                Value::Null,
                Some(format!(
                    "not described yet: {awaiting} of the memories here have no vectors, so a \
                     description now would leave them out. It will be derived once they are embedded, \
                     and reported by `hoard_conventions` at the start of every session until this set \
                     is next exported."
                )),
            )
        } else {
            let derived = Self::derive_summary(store)?;
            (derived.as_ref().map_or(Value::Null, describe), None)
        };

        let mut result = json!({
            "read": path.to_string_lossy(),
            // What the set amounts to now, derived here rather than taken from the file.
            "describes": describes,
            // Said on both sides of the round trip for one reason: the two forms are two
            // reconciliation targets, so which one ran decides which recorded agreement this import
            // reconciled against.
            "path_form": source.form(),
            // Present only when this import actually handed the set an identity it did not have.
            // Null on every ordinary pull, which is what makes it worth reading when it is not.
            "adopted_set_id": adopted,
            "outcomes": counts,
        });
        if let Some(note) = identity_note {
            result["identity_note"] = json!(note);
        }
        // Absent when the import could describe the set, so its presence is the signal.
        if let Some(note) = description_owed {
            result["description_owed"] = json!(note);
        }

        // **The unrecognised target, said out loud on the reading side too.**
        //
        // `target-identity-is-an-unnormalised-path-string`. Export has remarked on this since it was
        // built; import read `bases_for`, got an empty map for a spelling it had never seen, and
        // reconciled the whole file **with no ancestor** -- the one case the per-record base exists
        // to prevent -- with nothing in the response to distinguish it from a genuine first import.
        //
        // **Normalising does not retire this**, which is why it is a remark rather than a thing the
        // key was going to fix. Canonicalisation collapses spellings; it does not survive a checkout
        // being renamed or moved, which orphans every base just as thoroughly and arrives looking
        // exactly like a directory never seen before. A second checkout is legitimate and a mistyped
        // path is not, and only the caller can tell -- so this is never a refusal.
        if !known.is_empty() && !known.contains(&target) {
            result["note"] = json!(format!(
                "this set has agreed with {} before; you named a location it has no record of, so \
                 every record in the file was reconciled with no common ancestor. A second checkout \
                 or a fresh clone is expected -- the next export records the agreement. A moved or \
                 renamed checkout looks the same from here and is also expected. A mistyped path is \
                 not: if these outcomes look wrong, check the location before acting on them.",
                known.join(", ")
            ));
        }
        if !conflicts.is_empty() {
            conflicts.sort();
            result["conflicts"] = json!(conflicts);
            result["conflict_guidance"] = json!(
                "Both sides changed these memories since they last agreed, and nothing here can \
                 choose between them. Nothing was written for them. Fetch each with `hoard_lookup` \
                 to see the store's version, read the file's version, and store the resolution."
            );
        }
        if !cycles.is_empty() {
            cycles.sort();
            result["supersession_cycles"] = json!(cycles);
            result["supersession_cycle_guidance"] = json!(
                "These memories supersede each other, directly or through a chain, so none of them \
                 is the current one and recall cannot stand on any of them. They were stored \
                 rather than refused, because the cycle is a property of the set that arrived and \
                 refusing it would have cost every other record in the file. No tool can resolve \
                 this for you: read the memories with `hoard_lookup`, decide which is current, and \
                 store that judgement. A merge that combined two supersession chains is the usual \
                 cause."
            );
        }
        if !backwards.is_empty() {
            backwards.sort();
            result["taken_backwards"] = json!(backwards);
            result["taken_backwards_guidance"] = json!(
                "These records arrived older than the versions they replaced, and were taken \
                 anyway. That is never what an ordinary pull looks like: every edit stamps a new \
                 `modified`, and forgetting stamps its own moment, so an incoming version that \
                 predates the one here is not a successor to it. The usual cause is a file that \
                 went backwards -- an earlier commit checked out, a revert of the commit that \
                 carried a change, a restored backup, or an older copy of the set read from a \
                 location this set has agreed with before. A memory here may have been reverted to \
                 an earlier form. Read each with `hoard_lookup`, and if the file was the wrong \
                 one, re-import the right one: nothing is lost that the correct file does not \
                 restore. Reported rather than refused, because a deliberate rollback looks \
                 exactly like this and only you can tell them apart."
            );
        }
        // **Said out loud, because a reverted file lands here too.** A forget is one-way at import
        // since 2026-09-28 (`merge-without-a-recorded-ancestor`), so the caller who meant to undo
        // one is told how, in the same breath as being told it did not happen.
        if !kept_forgotten.is_empty() {
            kept_forgotten.sort();
            result["kept_forgotten"] = json!(kept_forgotten);
            result["kept_forgotten_guidance"] = json!(
                "The file carries these memories, but this set has forgotten them, so they stayed \
                 forgotten: a forget is never undone by a file. The usual cause is a file that \
                 went backwards -- an older copy of the set, or an earlier commit checked out. If \
                 it was a revert meant to bring one back, store it again with a `supersedes` link \
                 to its id: that is a new record rather than an old one returning, and links that \
                 pointed at the forgotten id reach the new memory in recall."
            );
        }
        if !anomalies.is_empty() {
            anomalies.sort();
            result["anomalies"] = json!(anomalies);
            result["anomaly_guidance"] = json!(
                "These records were previously agreed with this file and are now missing from one \
                 side, with no tombstone saying they were forgotten. That is not a state any \
                 supported operation produces -- a hand-edited file or database is the likely cause. \
                 Nothing was written for them."
            );
        }
        Ok(result)
    }

    /// What an import says about identity when the file holds records and declares none.
    ///
    /// No export of a set with records writes such a header, so it was hand-edited or written
    /// elsewhere, and it is the first step of the one trap measured on 2026-09-22 -- a hand-renamed
    /// fork committed as edited, whose every clone keeps an identity of its own. A file declaring
    /// *another* identity is not a note but a refusal, made before this is asked: see
    /// [`identity_mismatch`].
    fn identity_note_for_import(
        path: &std::path::Path,
        scope: &Scope,
        declared: Option<SetId>,
        held: Option<SetId>,
        minted_now: bool,
        has_records: bool,
    ) -> Option<String> {
        let set = json!(WireScope(scope));
        match (declared, held) {
            (None, Some(held)) if has_records && minted_now => Some(format!(
                "{} declares no set identity, which an export of a set holding records always \
                 writes, so it was edited by hand or written elsewhere. {set} had none either, and \
                 was given {held} of its own: every machine importing this file does the same, so \
                 their copies are different sets. If the set's own export exists, import that \
                 instead.",
                path.display()
            )),
            (None, Some(held)) if has_records => Some(format!(
                "{} declares no set identity, which an export of a set holding records always \
                 writes, so it was edited by hand or written elsewhere. {set} keeps {held}. If it \
                 is a fork renamed by hand, the file to commit is the renaming machine's re-export, \
                 which carries the fork's own identity: a hand-edited copy gives each machine that \
                 imports it an identity of its own.",
                path.display()
            )),
            _ => None,
        }
    }

    /// Serialise a whole set into the exported format, one JSON object per line after the header.
    ///
    /// **One writer over the format, and that is the point rather than tidiness.** `export` and the
    /// safety copy that `drop` writes produce the same bytes for the same reason — a copy that is
    /// not readable by the ordinary reader is not a backup — and the way that goes wrong is two
    /// loops that agree today. `memory-file-read-silently-discards-tombstones` in `TRACKING.md` is
    /// the reading half of exactly this: a second reader over one format, adapted to compile when
    /// the format grew a record shape, and silently dropping it ever since.
    ///
    /// `per_record` sees each record and the line it serialised to, so a caller can collect
    /// checksums or scan the text without a second pass. Callers that want neither pass a no-op.
    fn serialise_set(
        store: &Store,
        scope: &Scope,
        per_record: impl FnMut(&memory_file::Record, &str),
    ) -> Result<Serialised, String> {
        let set_id = store
            .set_id()
            .map_err(|error| format!("could not read this set's identity: {error}"))?;
        Self::serialise_set_as(store, scope, set_id, per_record)
    }

    /// [`serialise_set`](Self::serialise_set), declaring `set_id` in the header rather than the
    /// identity the store holds -- for an export proposing the identity it will give the set only once
    /// nothing refuses it.
    fn serialise_set_as(
        store: &Store,
        scope: &Scope,
        set_id: Option<SetId>,
        mut per_record: impl FnMut(&memory_file::Record, &str),
    ) -> Result<Serialised, String> {
        let mut body: Vec<u8> = Vec::new();

        // Derived here rather than by the caller so there is exactly one derivation per write, which
        // the response reports. It is not written into the file: see [`memory_file::Header`].
        let summary = Self::derive_summary(store)?;
        memory_file::write_header(scope, set_id, &mut body).map_err(|error| error.to_string())?;

        let written = store
            .stream_records(|record| {
                let from = body.len();
                // Infallible: `io::Write` on a `Vec<u8>` has no failure mode. The destination file
                // is written once, afterwards, where a real error can occur and is reported.
                memory_file::write_record(record, &mut body).expect("writing to a Vec cannot fail");
                if let Ok(line) = std::str::from_utf8(&body[from..]) {
                    // Borrowed out of `body` before `per_record` may touch anything else.
                    let line = line.to_owned();
                    per_record(record, &line);
                }
                Ok(())
            })
            .map_err(|error| format!("could not read the set: {error}"))?;

        Ok(Serialised {
            body,
            written,
            summary,
        })
    }

    /// What the set amounts to, derived from its own contents.
    ///
    /// Returns nothing for a set with no vectors: there is no space to group memories in, so there
    /// is nothing to describe. The error case is a store that cannot be read at all, which the
    /// caller is already reporting on.
    fn derive_summary(store: &Store) -> Result<Option<SetSummary>, String> {
        let subjects = store
            .summary_subjects()
            .map_err(|error| format!("could not read the set to describe it: {error}"))?;
        if subjects.is_empty() {
            return Ok(None);
        }
        Ok(Some(SetSummary::derive(&subjects, SUMMARY_LABELS)))
    }

    /// Write the body to the file a [`Destination`] resolved to, atomically.
    ///
    /// Via a temporary file and a rename, which is atomic on POSIX: a crash partway leaves the
    /// previous export intact rather than a truncated file that no reader can parse and that `git`
    /// would happily commit.
    ///
    /// **Only the directory form creates anything.** It owns the reserved directory and may make it;
    /// the explicit form writes into a directory the caller already has, and `Destination::refusal`
    /// has said so before this runs. Creating a missing parent there would turn a mistyped path into
    /// a silently-made directory holding a set nobody can find.
    fn write_memory_file(
        destination: &Destination,
        path: &Path,
        body: &[u8],
    ) -> Result<PathBuf, String> {
        if let Destination::Directory(directory) = destination {
            let home = directory.join(RESERVED_DIRECTORY);
            let making_it = !home.exists();
            std::fs::create_dir_all(&home)
                .map_err(|error| format!("could not create {}: {error}", home.display()))?;
            // **Only when this export made the directory**, so a README is never added to one that
            // exists and never written over: a project may write its own, and a file appearing unasked
            // would turn up in its next diff. Decided 2026-10-17 by the human partner.
            if making_it {
                let readme = home.join(DIRECTORY_README);
                std::fs::write(&readme, DIRECTORY_README_TEXT)
                    .map_err(|error| format!("could not write {}: {error}", readme.display()))?;
            }
        }

        // Beside the destination rather than in a temporary directory, so the rename stays within
        // one filesystem and therefore stays atomic.
        let mut temporary = path.to_path_buf().into_os_string();
        temporary.push(".tmp");
        let temporary = std::path::PathBuf::from(temporary);
        std::fs::write(&temporary, body)
            .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| format!("could not replace {}: {error}", path.display()))?;
        Ok(path.to_path_buf())
    }

    /// Report the links whose target this machine cannot resolve, in the order they were written.
    ///
    /// `documents/design/cross-set-references.md`: **reported, never refused.** Links are apparatus,
    /// so refusing a write over one would make an apparatus error cost the claim — a sound memory
    /// not stored at all because of a field `amend` repairs cheaply. A report reaches the caller in
    /// the same turn a refusal would, and leaves the judgement where the relaxed stance puts it.
    ///
    /// Absence is not proof of a mistake. A set is a replica of a git-synced file, so a store can
    /// lag the repository it came from and an id merged upstream but not yet imported is a perfectly
    /// good target. That holds for a same-set target exactly as for a cross-set one, which is why
    /// neither is treated as the more suspicious.
    ///
    /// **Not in `Store::insert`.** That is shared with import, where a per-record judgement costs
    /// the whole file — `two-import-paths-disagree-about-supersession-cycles` in `TRACKING.md` records
    /// what happens when a write-time check reaches import by that route.
    fn unresolved_links(
        &self,
        scope: &Scope,
        links: &[Link],
    ) -> Result<Vec<UnresolvedLink>, String> {
        let mut reports = Vec::new();
        for link in links {
            let target_scope = match link.target.set() {
                Some(set) => Scope::Shared(set.clone()),
                None => scope.clone(),
            };
            let state = match self.stores.get(&target_scope) {
                // Not held here, so nothing was checked. Said out loud rather than omitted: silence
                // would be indistinguishable from a check that passed, which is the shape
                // `prohibited-data-scanned-conflates-skipped-with-unbuilt` was filed for.
                None => LinkState::SetNotLoaded,
                Some(store) => {
                    let held = store.holds_record(link.target.id()).map_err(|error| {
                        format!("could not check whether a link target exists: {error}")
                    })?;
                    if held {
                        continue;
                    } else if link.target.set().is_some() {
                        LinkState::NoSuchIdInThatSet
                    } else {
                        LinkState::NoSuchIdInThisSet
                    }
                }
            };
            reports.push(UnresolvedLink {
                label: link.label.clone(),
                target: link.target.id(),
                set: link.target.set().map(SetName::to_string),
                state,
            });
        }
        Ok(reports)
    }

    fn store_for_mut(&mut self, scope: &Scope) -> Result<&mut Store, String> {
        if self.stores.contains_key(scope) {
            Ok(self.stores.get_mut(scope).expect("just checked"))
        } else {
            Err(missing_scope(&self.withheld, scope))
        }
    }

    /// Who a write to this scope is attributed to.
    ///
    /// **The store answers, not this process.** `self.author` is only the default a store claims
    /// when it has none, which is what keeps one `local` store shared by two servers from recording
    /// the same person under two identities — `documents/design/store-registry.md`.
    ///
    /// Every write path goes through here rather than reading `self.author`, so a fourth one added
    /// later cannot reintroduce the defect by copying the older shape.
    /// Fill in the identity of every cross-set link target this machine holds a set for.
    ///
    /// **Written by the server, never by the caller, and this is [Decision Placement](documents/design/decision-placement.md)
    /// applied rather than a convenience.** A model writing a link knows an id and a set name; a
    /// set's identity is a fact this process holds and no caller could supply without being told it
    /// first. Adding it to the tool schema would be asking for information the asker does not have.
    ///
    /// **Absent is a real answer and stays one.** A link may name a set nobody here has ever held —
    /// the schema documents a dangling target as allowed — so there is often no identity to write,
    /// permanently rather than during a migration, and the name has to keep working alone.
    ///
    /// **Authoring only.** This must never run on a record arriving from a file: filling in an
    /// identity its author did not write is a receiver-side rewrite, which is the act
    /// `absorb-as-supersession` removed, and it would put one id under two contents again by way of
    /// helpfulness. Pinned by `importing_does_not_add_an_identity_the_author_did_not_write`.
    fn identify_link_targets(&self, links: &mut [Link]) -> Result<(), String> {
        for link in links {
            let LinkTarget::CrossSet { set, set_id, .. } = &mut link.target else {
                continue;
            };
            if set_id.is_some() {
                continue;
            }
            let named = Scope::Shared(set.clone());
            if let Some(store) = self.stores.get(&named) {
                *set_id = store
                    .set_id()
                    .map_err(|error| format!("could not read the identity of {set:?}: {error}"))?;
            }
        }
        Ok(())
    }

    /// The name a set's first write may claim it for, where nobody has said: `local`'s, and only
    /// `local`'s.
    ///
    /// **A shared set has none**, since `set-settled-at-first-write` (decided 2026-09-24): the name is
    /// how you are known where that set is published, which varies by set, so it is asked at the first
    /// write rather than taken from the machine -- which is how a corpus came to be attributed to a
    /// machine account. **`local` takes the name a store written by an older build recorded, then the
    /// account name**: it is never published, so a default costs nothing there, and refusing its first
    /// write would put a step in front of every new user. Where neither exists it has none either.
    fn default_author(&self, scope: &Scope) -> Result<Option<Author>, String> {
        let Scope::Local = scope else {
            return Ok(None);
        };
        let recorded = self
            .stores
            .get(scope)
            .ok_or_else(|| missing_scope(&self.withheld, scope))?
            .author()
            .map_err(|error| {
                format!("could not read who this set attributes writes to: {error}")
            })?;
        Ok(recorded.or_else(|| self.account_name.clone()))
    }

    /// Whether the human can be asked about `scope` on this request: the client shows forms, and the
    /// set is published. `local` is never published, so it has nothing to ask.
    fn asks_about(&self, scope: &Scope) -> bool {
        self.exchange.asks_human && matches!(scope, Scope::Shared(_))
    }

    /// What the human did with the question `key`, as the client attached it to this call.
    fn answer(&self, key: &str) -> Answer {
        let Some(response) = self.exchange.answers.get(key) else {
            return Answer::Unasked;
        };
        match response["action"].as_str() {
            Some("accept") => {
                Answer::Given(response["content"].as_object().cloned().unwrap_or_default())
            }
            Some("decline") => Answer::Declined,
            Some("cancel") => Answer::Cancelled,
            _ => Answer::Unasked,
        }
    }

    /// Whether the question `key` came back without an answer, declined or dismissed.
    ///
    /// **Both, because both are how a client says nobody answered**, and which word it uses is not
    /// the point: Claude Code in print mode declares form elicitation and declines every form unseen,
    /// observed 2026-10-16, and another client might dismiss instead. Keying the fallback on the one
    /// word seen would leave the other looping on the question forever.
    fn went_unanswered(&self, key: &str) -> bool {
        matches!(self.answer(key), Answer::Declined | Answer::Cancelled)
    }

    /// Stop this call to put `question` to the human. The returned text is never served: the call's
    /// result becomes the question, and the client retries with the answer.
    fn ask(&mut self, key: &'static str, question: Value) -> String {
        self.exchange.question = Some((key, question));
        "waiting for the human's answer".to_owned()
    }

    /// The name the human gives for themselves in `scope`, asking for it if they have not been asked.
    ///
    /// **Called only where [`Server::asks_about`] holds, and before anything is written**, so a call
    /// that asks has changed nothing. A decline is the human's answer and is reported as one; a
    /// cancel leaves the question open for the next call. An answer that cannot be a name is asked
    /// again, saying why, rather than refused -- the specification's advice, and the kinder of the
    /// two for someone who mistyped.
    fn name_from_human(&mut self, scope: &Scope, purpose: NamePurpose) -> Result<Author, String> {
        let set = scope.as_key();
        match self.answer(NAME_QUESTION) {
            Answer::Given(content) => {
                let given = content
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match Author::new(given) {
                    Ok(name) => Ok(name),
                    Err(error) => {
                        let problem = format!("{given:?} could not be used: {error}. ");
                        let question = name_question(&set, purpose, Some(&problem));
                        Err(self.ask(NAME_QUESTION, question))
                    }
                }
            }
            Answer::Declined => Err(unanswered_name(&set, purpose, "declined")),
            Answer::Cancelled => Err(unanswered_name(
                &set,
                purpose,
                "dismissed without an answer",
            )),
            Answer::Unasked => {
                let question = name_question(&set, purpose, None);
                Err(self.ask(NAME_QUESTION, question))
            }
        }
    }

    /// Which identity record the human says is theirs in `scope`, asking if they have not been
    /// asked; with `offer_new`, they may instead say they are new and give a name.
    ///
    /// **Offered by name, answered by id**: the form carries each record's id beside the name it
    /// shows, so the human's choice is never resolved by matching a name -- the guess that would fork
    /// one person into two identities.
    fn identity_from_human(&mut self, scope: &Scope, offer_new: bool) -> Result<Chosen, String> {
        let set = scope.as_key();
        let records = self
            .stores
            .get(scope)
            .ok_or_else(|| missing_scope(&self.withheld, scope))?
            .current_identity_records()
            .map_err(|error| format!("could not read the set's identity records: {error}"))?;
        if records.is_empty() && !offer_new {
            return Err(format!(
                "{set} holds no identity records to choose from: action \"mint\" says you are new \
                 to it."
            ));
        }
        match self.answer(IDENTITY_QUESTION) {
            Answer::Given(content) => {
                let chosen = content
                    .get("record")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if offer_new && chosen == NEW_IDENTITY {
                    let given = content
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    return match Author::new(given) {
                        Ok(name) => Ok(Chosen::New(name)),
                        Err(_) => {
                            let question = identity_question(
                                &set,
                                &records,
                                offer_new,
                                Some("To be added as new, give the name you are known by. "),
                            );
                            Err(self.ask(IDENTITY_QUESTION, question))
                        }
                    };
                }
                match records.iter().find(|(id, _)| id.to_string() == chosen) {
                    Some((id, _)) => Ok(Chosen::Record(*id)),
                    None => {
                        let question = identity_question(
                            &set,
                            &records,
                            offer_new,
                            Some("That choice is not one of the names below. "),
                        );
                        Err(self.ask(IDENTITY_QUESTION, question))
                    }
                }
            }
            Answer::Declined => Err(unanswered_identity(&set, "declined")),
            Answer::Cancelled => Err(unanswered_identity(&set, "dismissed without an answer")),
            Answer::Unasked => {
                let question = identity_question(&set, &records, offer_new, None);
                Err(self.ask(IDENTITY_QUESTION, question))
            }
        }
    }

    /// Point this machine at a record the human chose, or mint one for a newcomer, and remember it.
    fn settle_chosen_identity(
        &mut self,
        scope: &Scope,
        chosen: Chosen,
    ) -> Result<MemoryId, String> {
        let set = scope.as_key();
        let (record, note) = {
            let store = self.store_for_mut(scope)?;
            match chosen {
                Chosen::Record(record) => {
                    let name = store
                        .identify_author(record)
                        .map_err(|error| error.to_string())?;
                    let note = format!(
                        "The human was asked which identity is theirs in {set}, through a form \
                         their client showed them and you did not see, and chose \"{}\". \
                         Everything written there from this machine is attributed to it.",
                        name.as_str()
                    );
                    (record, note)
                }
                Chosen::New(name) => {
                    let record = store
                        .mint_author(&name, jiff::Timestamp::now())
                        .map_err(|error| error.to_string())?;
                    let note = format!(
                        "The human was asked which identity is theirs in {set}, through a form \
                         their client showed them and you did not see, and said they are new \
                         here, as \"{}\". It is recorded as the author of everything written \
                         there from this machine.",
                        name.as_str()
                    );
                    (record, note)
                }
            }
        };
        self.exchange.settled = Some(note);
        let set_id = self
            .stores
            .get(scope)
            .ok_or_else(|| missing_scope(&self.withheld, scope))?
            .set_id()
            .map_err(|error| format!("could not read the set's identity: {error}"))?;
        if let Some(set_id) = set_id {
            self.remember_author(scope, set_id, record)?;
        }
        Ok(record)
    }

    /// The author record this set attributes writes to, as an id.
    ///
    /// `&mut self` because resolving may *mint*: a set with no identity yet gets one on its first
    /// write, which is the one case where nothing can collide. Where the set already holds author
    /// records and this machine has not said which is you, this refuses rather than guessing --
    /// adopting on a name match would fork whoever has changed the name they publish under, and
    /// nothing downstream could report it. See `documents/design/memory-data-model.md`.
    fn author_for(&mut self, scope: &Scope) -> Result<MemoryId, String> {
        // **Asked only for a write, so a read-only set is refused here, first.** Otherwise a set
        // imported with its publishers' identity records reaches the question below -- *which of these
        // is you* -- which is meaningless for a set this machine never writes to, and the store's own
        // refusal is never reached.
        self.refuse_if_read_only(scope)?;
        // **Three layers, and only the first is on the write path.** The set's own `meta` answers
        // every ordinary write in one read of a store already open. The durable map in `local` is
        // consulted only on a miss — first contact with a set on this machine, or a re-add after a
        // `drop` destroyed the set's cache along with its database. The question is last, and only
        // where the other two cannot answer without guessing.
        // `documents/design/memory-data-model.md`, *which record is you is machine state*.
        let (cached, set_id) = {
            let store = self
                .stores
                .get(scope)
                .ok_or_else(|| missing_scope(&self.withheld, scope))?;
            let cached = store.author_record().map_err(|error| {
                format!("could not read who this set attributes writes to: {error}")
            })?;
            let set_id = store
                .set_id()
                .map_err(|error| format!("could not read the set's identity: {error}"))?;
            (cached, set_id)
        };
        if let Some(record) = cached {
            return Ok(record);
        }

        // Keyed on identity, never on the set's name: two checkouts can hold one set under
        // different names, and a name can change while an identity cannot. A set with no identity
        // of its own — one written before set ids existed — simply has no key here, and falls
        // through to the question rather than being matched by some weaker handle.
        if let Some(set_id) = set_id
            && let Some(row) = self
                .stores
                .get(&Scope::Local)
                .map(|local| local.author_map_get(set_id))
                .transpose()
                .map_err(|error| format!("could not read the author map: {error}"))?
                .flatten()
        {
            let store = self
                .stores
                .get_mut(scope)
                .ok_or_else(|| missing_scope(&self.withheld, scope))?;
            // **Checked against the set, not trusted.** A row can name a record this copy of the
            // set does not hold — an older export, a set restored from before the identity existed
            // — and adopting it would leave every write pointing at nothing. A row that does not
            // resolve is treated as no row at all, which lands on the question.
            let held = store
                .holds_record(row.record)
                .map_err(|error| format!("could not check the set for that identity: {error}"))?;
            if held {
                store.write_author_record(row.record).map_err(|error| {
                    format!("could not record which identity is yours here: {error}")
                })?;
                self.remember_author(scope, set_id, row.record)?;
                return Ok(row.record);
            }
        }

        // **Nobody has said who they are in this set on this machine.** Where the set already holds
        // identity records, that is the question below. Where it holds none, whether the write may
        // mint one depends on the set -- `set-settled-at-first-write`.
        let holds_records = self
            .stores
            .get(scope)
            .ok_or_else(|| missing_scope(&self.withheld, scope))?
            .holds_any_author_record()
            .map_err(|error| {
                format!("could not read who this set attributes writes to: {error}")
            })?;
        // **Where the human can be asked, they are**, and the model is never the source: the name,
        // or the choice among records already there, comes from a form only they fill in.
        // `documents/design/asking-the-human.md`.
        if holds_records && self.asks_about(scope) {
            let chosen = self.identity_from_human(scope, true)?;
            return self.settle_chosen_identity(scope, chosen);
        }
        let mut from_form = None;
        let name = if holds_records {
            None
        } else {
            match self.default_author(scope)? {
                Some(name) => Some(name),
                None if self.asks_about(scope) => {
                    let name = self.name_from_human(scope, NamePurpose::First)?;
                    from_form = Some(name.clone());
                    Some(name)
                }
                None => return Err(nobody_named_refusal(scope)),
            }
        };
        let minted = match name {
            None => None,
            Some(name) => self
                .stores
                .get_mut(scope)
                .ok_or_else(|| missing_scope(&self.withheld, scope))?
                .author_record_or_mint(&name, jiff::Timestamp::now())
                .map_err(|error| {
                    format!("could not read who this set attributes writes to: {error}")
                })?,
        };

        match minted {
            Some(record) => {
                // **Read again, not reused.** Minting the record was this set's first write if it had
                // no identity, and that write is what gave it one, since `set-settled-at-first-write`
                // -- so the identity read above can be stale, and the map row would go unwritten.
                let set_id = self
                    .stores
                    .get(scope)
                    .ok_or_else(|| missing_scope(&self.withheld, scope))?
                    .set_id()
                    .map_err(|error| format!("could not read the set's identity: {error}"))?;
                if let Some(set_id) = set_id {
                    self.remember_author(scope, set_id, record)?;
                }
                if let Some(name) = from_form {
                    self.exchange.settled = Some(format!(
                        "The human was asked for the name they write under in {}, through a form \
                         their client showed them and you did not see, and gave \"{}\". It is \
                         recorded as the author of everything written there from this machine.",
                        scope.as_key(),
                        name.as_str()
                    ));
                }
                Ok(record)
            }
            None => Err(format!(
                "{} already holds identity records, and this machine has not said which \
                 one is you. Recall the keyword `hoard-author` to see them, then say which is \
                 yours -- guessing from a matching name would fork one person into two \
                 identities, and nothing afterwards could tell that apart from two people.",
                serde_json::to_string(scope).unwrap_or_else(|_| "?".to_owned())
            )),
        }
    }

    /// Whether a write to this set would have to stop and ask which identity record is yours.
    ///
    /// The same three layers `author_for` consults, read-only and without the minting: the set's own
    /// cache, then the durable map, then whether there is anything to collide with. Reported by
    /// `status` so the answer is available before a caller meets the refusal mid-write, which is the
    /// worst moment to be asked a question about set membership.
    fn must_ask_who_you_are(&self, scope: &Scope) -> Result<bool, String> {
        let Some(store) = self.stores.get(scope) else {
            return Ok(false);
        };
        let cached = store.author_record().map_err(|error| {
            format!("could not read who this set attributes writes to: {error}")
        })?;
        if cached.is_some() {
            return Ok(false);
        }

        let set_id = store
            .set_id()
            .map_err(|error| format!("could not read the set's identity: {error}"))?;
        if let Some(set_id) = set_id
            && let Some(local) = self.stores.get(&Scope::Local)
            && let Some(row) = local
                .author_map_get(set_id)
                .map_err(|error| format!("could not read the author map: {error}"))?
            && store
                .holds_record(row.record)
                .map_err(|error| format!("could not check the set for that identity: {error}"))?
        {
            return Ok(false);
        }

        // With no identity records the next write mints, which collides with nothing and asks
        // nothing. It is only a set that already holds some that has a question to put.
        store
            .identity_count()
            .map(|held| held > 0)
            .map_err(|error| format!("could not count this set's identity records: {error}"))
    }

    /// Write the durable answer, and refresh the scope label while we are here.
    ///
    /// The label is for a person pruning rows for sets they no longer keep, which they do without
    /// the set being present — so it is rewritten on every touch of the map, and can only be stale
    /// for a set that is absent. That is exactly the case where a misread costs nothing: deleting a
    /// row is safe, because the fallback either mints where nothing can collide, or asks.
    fn remember_author(
        &self,
        scope: &Scope,
        set_id: SetId,
        record: MemoryId,
    ) -> Result<(), String> {
        let Some(local) = self.stores.get(&Scope::Local) else {
            // A server with no local store cannot remember, and that is not a reason to refuse the
            // write it was resolving for: the set's own cache still answers every later write here.
            return Ok(());
        };
        let label = match scope {
            Scope::Local => "local".to_owned(),
            Scope::Shared(name) => format!("shared:{name}"),
        };
        local
            .author_map_put(set_id, record, &label)
            .map_err(|error| format!("could not remember which identity is yours: {error}"))
    }
}

/// Arguments for `hoard_sets` action `create`.
#[derive(Debug, Deserialize)]
struct CreateSet {
    name: String,
    /// Absent means read-write: most sets are the machine's own, and asking a new user about it
    /// would only confuse them -- `documents/design/load-modes.md`.
    #[serde(default)]
    mode: Option<CreatableMode>,
}

/// The modes `create` can make, which is not every mode there is.
///
/// **A separate type so evaluation is unsayable here rather than refused.** An evaluation load has
/// no `create` step at all: it comes from a file, under a name only `SetName::evaluation` can build,
/// into a store that never touches the disk. Taking [`LoadMode`] directly would have let
/// `mode: "evaluation"` make an ordinary on-disk set claiming that mode -- which it did, for the
/// hour between the variant being added and this type existing.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum CreatableMode {
    ReadWrite,
    ReadOnly,
}

impl From<CreatableMode> for LoadMode {
    fn from(mode: CreatableMode) -> Self {
        match mode {
            CreatableMode::ReadWrite => Self::ReadWrite,
            CreatableMode::ReadOnly => Self::ReadOnly,
        }
    }
}

/// Arguments for `hoard_sets` action `mint`.
///
/// [`Author`] validates on the way in, so a value this struct holds is already one a store can
/// record — the *parse, don't validate* rule in `STANDARDS.md`. A blank or control-bearing identity
/// is refused by the deserialiser before any store is touched.
#[derive(Debug, Deserialize)]
struct MintAuthor {
    scope: Scope,
    /// Absent where the human is asked instead; see the `mint` arm of `sets`.
    #[serde(default)]
    author: Option<Author>,
}

/// Arguments for `hoard_sets` action `identify`.
///
/// **A record id, never a name.** A name can match several current records or none, and choosing
/// between them is the guess the whole design refuses to make; taking a name here would put that
/// guess behind a different verb. The id is one recall of `hoard-author` away.
#[derive(Debug, Deserialize)]
struct IdentifyAuthor {
    scope: Scope,
    /// Absent where the human is asked instead.
    #[serde(default)]
    record: Option<MemoryId>,
}

/// Arguments for `hoard_sets` action `rename`.
#[derive(Debug, Deserialize)]
struct RenameAuthor {
    scope: Scope,
    /// Absent where the human is asked instead.
    #[serde(default)]
    author: Option<Author>,
}

/// Arguments for `hoard_sets` action `export`.
///
/// Takes a full [`Scope`] rather than a set name, because a `local` set is exported too — that is
/// what backup is, and `documents/design/file-format.md` settles that both scopes use one writer.
///
/// **The destination is one field of an alternation, not two optional strings.** A pair of
/// `Option<String>` would make *both* and *neither* representable here, and the code downstream
/// would then owe a branch for two states the caller is already refused at dispatch — untestable
/// defence of the kind `STANDARDS.md` says to design out rather than write. Flattening
/// [`Destination`] puts the rule in the type: this struct cannot hold two destinations or none.
#[derive(Debug, Deserialize)]
struct ExportSet {
    scope: Scope,
    #[serde(flatten)]
    destination: Destination,
}

/// Arguments for `hoard_sets` action `review`.
///
/// The destination is needed only for a shared set that is published, whose file is what "already
/// reviewed" is measured against; `local` and the sets the person never publishes take none.
#[derive(Deserialize)]
struct ReviewRequest {
    scope: Scope,
    #[serde(default)]
    directory: Option<PathBuf>,
    #[serde(default)]
    file: Option<PathBuf>,
    #[serde(default)]
    full: bool,
    #[serde(default)]
    recheck: Option<Vec<String>>,
}

impl ReviewRequest {
    fn destination(&self) -> Result<Option<Destination>, String> {
        match (&self.directory, &self.file) {
            (Some(_), Some(_)) => Err("give `directory` or `file`, not both".to_owned()),
            (Some(directory), None) => Ok(Some(Destination::Directory(directory.clone()))),
            (None, Some(file)) => Ok(Some(Destination::File(file.clone()))),
            (None, None) => Ok(None),
        }
    }
}

/// Arguments for `hoard_sets` action `backup`: only the set. Where it goes is the machine's
/// configuration, never the call's.
#[derive(Deserialize)]
struct BackupRequest {
    scope: Scope,
}

/// One memory's answers in `hoard_sets` action `clear`.
#[derive(Deserialize)]
struct MemoryAnswer {
    id: MemoryId,
    sentences: Vec<crate::review::SentenceAnswer>,
}

/// Arguments for `hoard_sets` action `clear`.
#[derive(Deserialize)]
struct ClearRequest {
    scope: Scope,
    #[serde(default)]
    answers: Vec<MemoryAnswer>,
    #[serde(default)]
    person_cleared: Vec<MemoryId>,
}

/// What one set's export review has done in this session.
#[derive(Debug, Clone, Default)]
struct ReviewSession {
    /// Where the set is published, for a shared set that is.
    destination: Option<Destination>,
    /// Each memory shown, with the checksum it had and how many sentences it was shown as.
    shown: BTreeMap<MemoryId, (u64, usize)>,
    /// Memories put to the person and not yet decided.
    flagged: std::collections::BTreeSet<MemoryId>,
    /// The session this set's review was last briefed in: given the task in full, the person's
    /// preferences and the keep-out lists. **Once per review in a session, not once per page**,
    /// since from the second page on that text is already in the session's context, and a full
    /// review of a large set spent most of its context reading it again.
    briefed_in: Option<SessionId>,
    /// The set's memories grouped by subject, kept for the next page while nothing it was worked out
    /// from has changed.
    grouping: Option<ReviewGrouping>,
    /// Whether each memory has a pre-pass hit, kept for the next page in the same way.
    hits: ReviewHitMemo,
}

/// What a review page works out about the whole set, kept between pages because a page shows a
/// handful of memories and working it out again was most of each page's cost:
/// `each-review-page-recomputes-the-whole-set`. **Each is keyed by everything it was worked out
/// from**, rather than dropped on some signal that the set changed, so a write by this process or
/// another one is seen without anything having to announce it.
#[derive(Debug, Clone)]
struct ReviewGrouping {
    /// [`crate::summary::fingerprint`] of the subjects this was worked out from.
    fingerprint: u64,
    /// Every memory with a vector, in the order the subjects came.
    ids: Vec<MemoryId>,
    group_of: std::collections::HashMap<MemoryId, usize>,
    labels: std::collections::HashMap<usize, Vec<String>>,
}

/// Whether a memory has a pre-pass hit, by the checksum of the text that was checked.
#[derive(Debug, Clone, Default)]
struct ReviewHitMemo {
    /// A digest of the preferences, always-fine values and author names the hits were found
    /// against. A different one empties the memo.
    against: u64,
    found: std::collections::HashMap<MemoryId, (u64, bool)>,
}

/// How many memories one `review` call puts to the session at most, so a long review comes in
/// sittings.
const REVIEW_BATCH: usize = 10;

/// How much memory text one `review` call shows at most, counted in characters of the memories
/// themselves. **Sized for a small model's context**: on `gemma4:12b` with a 16,384-token window, a
/// review of thirty memories ran to 52,000 characters, and the model answered none of them and then
/// said it had cleared them all. At least one memory is always shown, however long.
const REVIEW_BATCH_CHARS: usize = 6_000;

/// A review set this small is shown with context: up to [`REVIEW_CONTEXT`] already-reviewed memories
/// from each memory's subject. On `gemma4:12b`, memories reviewed alone were missed where the same
/// memories in their subject group were not.
const REVIEW_ALONE: usize = 3;
const REVIEW_CONTEXT: usize = 4;

/// What every review asks of the session, stated once.
const REVIEW_TASK: &str = "Review each memory below before it is exported. For every numbered \
sentence, say who it is about -- \"project\", \"the person\" or \"someone else\" -- and which items \
from any list in the sharing preferences it mentions, quoted as the preferences word them, whatever \
list they are on. A sentence saying what the person decided or said about the work is about the \
project. For a sentence about the person or someone else, an LLM included, also say whether it \
`reports` what they did, said, wrote or decided, or `judges` what they are like or why they acted. \
For someone else, quote items from `others_keep_out`, read as about them. The name in \
`published_as` is the one the person publishes this set under, so that name alone mentions nothing. \
Leave an item out only \
where an exception allows it in this set and the set's audience is private, and never for someone \
else. Answer every memory with hoard_sets action `clear`, one answer per sentence. \
Context memories are already reviewed: do not answer for them. Show the person each memory `clear` \
flags, in full, with its reasons, and do what they decide: keep it out with hoard_correct forget, \
reword it by storing a restatement that supersedes it and then forgetting it, or pass its id in \
`person_cleared` if they say it is fine.";

/// The task on a review page after the first in a session. **It keeps the instruction to answer**,
/// which a reader needs on every page, and points back for the rest rather than restating it.
const REVIEW_TASK_AGAIN: &str = "Answer each memory below with hoard_sets action `clear`, one \
answer per sentence, as the task on this review's first page in this session says. The sharing \
preferences and keep-out lists given there still apply.";

/// What a session start says beside `review_due`. **At a natural pause, not ahead of what the person
/// came to do**: the session holds the notice and raises it when it suits.
const REVIEW_DUE_NOTE: &str = "These sets hold memories not yet reviewed against the person's \
sharing preferences. Offer the person a review (hoard_sets action `review`) at a natural pause in this \
session, not ahead of what they came to do.";

/// The caution every review ends with: what it cannot promise.
const REVIEW_CAUTION: &str = "Memories this review does not flag are cleared on the model's reading \
alone, and the review needs a capable model: a small one can miss details, or fail to finish it while \
saying it has. Only what `clear` records counts. If a missed detail would be costly for the person, \
they should read these memories themselves before the set is published. If this set is about to \
become public, its project's file should say \"audience\": \"public\".";

/// Arguments for `hoard_sets` action `restore`.
///
/// **A scope, not a name — and the asymmetry with `drop` is the point.** `drop` names a set because
/// it destroys the only copy, and `local` is the one set with no copy anywhere by construction.
/// `restore` replaces a set *from a copy the caller is holding*, and for `local` that file is the
/// only recovery route there will ever be, since a local set never reaches a repository. Refusing it
/// here would withhold the operation from the set that needs it most.
///
/// **Two paths, and this is the request that proves they had to be two arguments.** `file` (or
/// `directory`) is read from; `save_to` is written to, before anything is destroyed. The design is
/// explicit that they must differ — a safety copy written over the file being restored from would
/// *"destroy the good copy in the act of protecting the bad one"*.
#[derive(Debug, Deserialize)]
struct RestoreSet {
    scope: Scope,
    #[serde(flatten)]
    source: Destination,
    save_to: std::path::PathBuf,
}

/// Arguments for `hoard_sets` action `absorb`.
///
/// **The same three arguments `restore` takes, and it means the same thing by each of them** —
/// which is why it shares [`Destination`] and the `save_to` path rather than spelling either
/// afresh. What differs is the expectation the source is read under: `restore` requires the file to
/// declare the set named in `scope`, and this requires it to declare a different one.
///
/// A scope rather than a name, for the reason [`RestoreSet`] gives: `local` is a set memories can
/// legitimately be taken *into*, and it is `route-where`'s fallback. A local *file* is absorbed only
/// into `local`, to merge a personal store split across two directories; into a shared set the
/// reader refuses it, so personal memories never reach a set that is published.
#[derive(Debug, Deserialize)]
struct AbsorbSet {
    scope: Scope,
    #[serde(flatten)]
    source: Destination,
    /// The set to read, as its file's header names it. See [`Destination::file_of_named`].
    set: Option<SetName>,
    save_to: std::path::PathBuf,
}

/// Arguments for `hoard_sets` action `drop`.
///
/// **Takes a name, not a scope, exactly as `create` does — so `local` is unrepresentable.** The two
/// are a pair: one brings a shared set into existence and the other takes it out, and neither can
/// mean the local set. That is not politeness about a dangerous operation. `local` is the set
/// `route-where`'s fallback clause points at, guaranteed to exist by construction, and the one set
/// that never reaches a repository — so it is also the only set with no copy anywhere else by
/// construction. A `name` of `local` reaches the *shared* set of that name, which is a different
/// set and droppable like any other.
///
/// **`save_to`, not `file`, and the two are different decisions rather than one spelled twice.**
/// `file` says where a set *lives* — written there by `export`, read from there by `import`. This
/// says where a copy goes before the set is destroyed, which is somewhere the set does not live and
/// by design somewhere it never has. `restore` settles it beyond doubt: it reads a `file` *and*
/// writes a safety copy in the same call, so one name could not carry both. Sharing a name would
/// also hand this argument export's published description, which is written for a destination the
/// caller means to keep.
///
/// It is a plain path rather than a [`Destination`]: a safety copy is by definition a separate
/// location, never a checkout. Reading it as an explicit file inherits the reserved-directory
/// refusal, which is what makes *the copy cannot land on the set's own export path* true by
/// construction for every checkout rather than by a check somebody has to remember.
#[derive(Debug, Deserialize)]
struct DropSet {
    name: String,
    /// Optional, and required at run time for a set this machine could have written to. A read-only
    /// set is dropped without a copy -- not having one to write and then clean up is a reason to
    /// hold a set that way -- and honoured where it is given anyway.
    /// `documents/design/load-modes.md`.
    #[serde(default)]
    save_to: Option<std::path::PathBuf>,
}

/// Import takes exactly what export does.
///
/// A separate struct rather than a reuse of [`ExportSet`], because the two are the two ends of one
/// round trip and are free to diverge: this one is the reading end, and its arguments are checked
/// against a file that already exists.
///
/// **It used to carry `absorbing`**, the set names a merged file had taken in, and that field was
/// the receiver-side rewrite `hoard_sets` action `absorb` replaced — removed from the published
/// schema on 2026-08-31 rather than deprecated, because a merge is now recorded as authored
/// supersessions that travel in the file and nobody downstream has to be told about it again.
///
/// **The same [`Destination`] alternation, and it has to be the same one.** A target is where the
/// two replicas agreed, so an export that wrote `/backups/set.json` and an import that read it must
/// arrive at one key or the reconciliation finds no ancestor and reports every record as changed.
/// Sharing the type is what makes that true by construction rather than by two matching lines.
/// Arguments for `hoard_sets` action `evaluate`.
#[derive(Debug, Deserialize)]
struct EvaluateSet {
    /// Which copy this is -- a branch, an issue, a version. The user's word, since only they know
    /// what the file they are handing over represents.
    label: String,
    #[serde(flatten)]
    source: Destination,
    /// The set to read, as its file's header names it. See [`Destination::file_of_named`].
    set: Option<SetName>,
}

#[derive(Deserialize)]
struct ImportSet {
    scope: Scope,
    #[serde(flatten)]
    destination: Destination,
}

/// Which set to rebuild the vectors of.
///
/// One set at a time rather than all of them, because the caller is answering a question about one
/// store — the model it holds is not the model this server has — and a bulk rebuild would act on
/// stores nobody asked about.
#[derive(Debug, Deserialize)]
struct ReembedSet {
    scope: Scope,
}

#[derive(Deserialize)]
struct CalibrateSet {
    scope: Scope,
}

/// Naming a set this machine does not hold stays an error rather than becoming an invitation to
/// create one: inventing a store on a typo would silently split a set.
///
/// But the refusal has to name an action the caller can actually take, which is what
/// `no-tool-lists-loaded-scopes` was about. It used to say *"check the set name"* with nothing to
/// check against, and *"have to be imported"* with no import tool — two dead ends in one sentence.
/// What a recall managed to cover, kept together so the two questions cannot drift apart.
///
/// **One boolean answered both and gave the wrong answer to one of them.** *Did the semantic path
/// run* and *did it cover everything asked for* are different questions: a union where one set is
/// unsearchable answers the first yes and the second no, and the old field said only "no" —
/// attached to a response carrying a correct ranking. `semantic-search-available-conflates-partial-with-none`.
struct Coverage {
    /// Whether the semantic path ran at all.
    searched: bool,
    /// Whether it covered every set in scope.
    complete: bool,
    /// The sets left out, each with its own reason and therefore its own remedy.
    skipped: Vec<Value>,
    /// Whether the query itself held nothing the model could represent.
    query_unrepresentable: bool,
    /// Whether a floor removed every hit there was.
    silenced_by_floor: bool,
    /// The sets every hit came from, when every set with a floor declined. See
    /// [`crate::store::Recall::answered_only_without_a_floor`].
    answered_only_without_a_floor: Vec<Scope>,
}

impl Coverage {
    fn from_recall(recall: &crate::store::Recall) -> Self {
        Self {
            searched: recall.searched,
            complete: recall.complete,
            query_unrepresentable: recall.query_unrepresentable,
            silenced_by_floor: recall.silenced_by_floor,
            answered_only_without_a_floor: recall.answered_only_without_a_floor.clone(),
            skipped: recall
                .skipped
                .iter()
                .map(|set| json!({"scope": WireScope(&set.scope), "reason": set.reason}))
                .collect(),
        }
    }

    /// Keyword search never runs the semantic path, so it reports on the sets rather than on a
    /// search it did not do -- which is why the fields are filled in by the caller there.
    fn keyword_path() -> Self {
        Self {
            searched: false,
            complete: true,
            skipped: Vec::new(),
            query_unrepresentable: false,
            // A keyword search has no scores, so there is no floor for it to fall below.
            silenced_by_floor: false,
            answered_only_without_a_floor: Vec::new(),
        }
    }
}

/// The reserved directory name `export` and `import` append to whatever they are given.
///
/// Written once here rather than beside each `join`, because the guard below has to recognise
/// exactly what those calls append.
const RESERVED_DIRECTORY: &str = "scatterhoard_mem";

/// The one file in the reserved directory that is not a memory file and that Scatterhoard writes.
///
/// `memory-directory-readme`: a person meeting `scatterhoard_mem` for the first time found
/// `memories.json` and no explanation. Written when an export makes the directory, and recognised
/// by name -- [`MEMORY_FILE_EXTENSION`] is what is read here, so it is never taken for a set, and
/// the unknown-entry check `documents/design/trigger-and-loading.md` designs must count it as known.
/// A README a project writes for itself has the same name and the same standing.
const DIRECTORY_README: &str = "README.md";

/// What [`DIRECTORY_README`] says when Scatterhoard writes it.
///
/// **It describes and never directs.** Scatterhoard provides no location for instruction-shaped
/// text, and a markdown file in a directory that declares it holds none is exactly the shape the
/// design treats as a smuggling signal (`documents/design/trigger-and-loading.md`), so nothing here
/// tells a reader or a model what to do. It names where to read more, which
/// `readme-names-the-project-url` asked for.
const DIRECTORY_README_TEXT: &str = "# scatterhoard_mem

This directory holds memory sets for Scatterhoard, a memory system for AI agents. Each .json file is
one memory set, written when Scatterhoard exports its memories so that it can be committed and
reviewed beside the project it describes. A config directory, where there is one, holds the
project's settings for Scatterhoard.

More about Scatterhoard: https://github.com/jtcours/scatterhoard
";

/// The extension every memory file in the reserved directory carries, and the only one read there.
const MEMORY_FILE_EXTENSION: &str = "json";

/// Names Windows reserves for devices, refused as a file's name whatever follows the first dot.
///
/// Checked on every export rather than only on Windows, because the repository goes wherever it is
/// cloned: a `con.json` written on a Mac is a checkout that fails on Windows.
const WINDOWS_RESERVED_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Every memory file in a reserved directory, with the header each declares, sorted by path.
///
/// `documents/design/set-lifecycle.md`, *finding a set's file*. A missing directory holds nothing,
/// which is the ordinary state of a checkout before its first export.
///
/// **A file whose name begins with a dot is not read**: macOS writes `._name` beside a file on a
/// volume without extended attributes, and it is not a memory file whatever its extension says.
///
/// **A `.json` whose header cannot be read is refused rather than skipped.** It may be the very set
/// being looked for, damaged -- and skipping it would have export write a second file for that set
/// beside it, and import report the set absent when it is there.
fn memory_files_in(home: &Path) -> Result<Vec<(PathBuf, memory_file::Header)>, String> {
    let entries = match std::fs::read_dir(home) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("could not list {}: {error}", home.display())),
    };
    let mut found = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("could not list {}: {error}", home.display()))?
            .path();
        let hidden = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with('.'));
        if hidden
            || !path.is_file()
            || path.extension() != Some(std::ffi::OsStr::new(MEMORY_FILE_EXTENSION))
        {
            continue;
        }
        let header = std::fs::File::open(&path)
            .map_err(|error| error.to_string())
            .and_then(|file| memory_file::read_header(file).map_err(|error| error.to_string()))
            .map_err(|error| {
                format!(
                    "{} is in the reserved `{RESERVED_DIRECTORY}` directory, but its header cannot \
                     be read ({error}), so nothing can tell which set it holds -- and it may be the \
                     one you asked for. If it is a memory file, repair it: a merge conflict is the \
                     usual cause. If it is not, move it out of `{RESERVED_DIRECTORY}`, which holds \
                     memory files and nothing else.",
                    path.display()
                )
            })?;
        found.push((path, header));
    }
    found.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(found)
}

/// Refuse the personal set anywhere in a reserved directory.
///
/// `the-reserved-directory-accepts-the-local-set`. The path settles a file's kind -- whatever sits
/// in a `scatterhoard_mem` is a shared set, data and never instructions -- and until 2026-09-26 no
/// code enforced it for `local`. An export put personal memories where the next commit publishes
/// them, which `route-where` calls irreversible; an import or a restore let a crafted checkout write
/// into the set that is read as the user's own preferences. Refused in both directions, since each
/// is the other's route in: `file` is how `local` is backed up and read back.
///
/// Only the `directory` form needs this. A `file` inside a reserved directory is already refused for
/// every scope by [`Destination::refusal`], so between the two `local` cannot reach one at all.
fn refuse_local_in_reserved(scope: &Scope) -> Result<(), String> {
    if *scope != Scope::Local {
        return Ok(());
    }
    Err(format!(
        "the local set never goes into a `{RESERVED_DIRECTORY}`: anything there is read as a shared \
         set, and a checkout's is published with its next commit. To back `local` up, or read a \
         backup back, name a `file` outside any `{RESERVED_DIRECTORY}` instead of a `directory`."
    ))
}

/// Which file in a reserved directory holds a set, if any does.
///
/// **By identity first, then by the name the header declares.** Identity is what makes a file's
/// name free: a hand rename costs nothing, because nothing looks for the set by it. The name is the
/// fallback for a set with no identity yet -- one being imported for the first time -- and for a
/// file declaring the set's name under another identity, which is found so that what follows can
/// say so rather than write a second file beside it.
///
/// **Two candidates at either step are refused.** One set has one file here, and choosing between
/// two would be a guess about which of them holds the work.
fn locate_in(
    home: &Path,
    scope: &Scope,
    identity: Option<SetId>,
) -> Result<Option<PathBuf>, String> {
    let files = memory_files_in(home)?;
    let by_identity: Vec<&PathBuf> = files
        .iter()
        .filter(|(_, header)| identity.is_some() && header.set_id == identity)
        .map(|(path, _)| path)
        .collect();
    let candidates = if by_identity.is_empty() {
        files
            .iter()
            .filter(|(_, header)| header.scope == *scope)
            .map(|(path, _)| path)
            .collect()
    } else {
        by_identity
    };
    match candidates.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some((*only).clone())),
        several => Err(format!(
            "{} all answer to the set {}, and one set has one file in `{RESERVED_DIRECTORY}`: \
             choosing between them would be a guess about which holds the work. Keep the one that \
             is the set's and move the others out of the directory; one that holds work of its own \
             can then be read with `import`, naming it as a `file` where it now sits.",
            several
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            scope.as_key()
        )),
    }
}

/// Which way a set was crossing a file when the two disagreed about its identity.
#[derive(Clone, Copy)]
enum Crossing {
    Import,
    Export,
}

/// The refusal for a file whose identity is not the set's, naming both and the way out of each case
/// that produces one.
///
/// `documents/design/set-lifecycle.md`, *no trusted name for a checkout, and an identity mismatch is
/// refused*. A mismatch arises in three ways and the reader cannot be expected to know which this is,
/// so all three are named. **The collision route is not built**, and the message says so rather than
/// naming a call that does not exist: until `open-a-set-under-an-alias` ships, two sets that share a
/// name cannot both be loaded on one machine, and refusing is still right -- importing or exporting
/// across them is what merges two sets' histories by accident.
fn identity_mismatch(
    path: &Path,
    scope: &Scope,
    declared: SetId,
    held: SetId,
    crossing: Crossing,
) -> String {
    let set = json!(WireScope(scope));
    let consequence = match crossing {
        Crossing::Import => "so its records were not imported",
        Crossing::Export => {
            "so it was not written over: that would replace its identity with this \
                             set's"
        }
    };
    format!(
        "{} declares the set identity {declared}, but {set} answers to {held} on this machine, \
         {consequence}. Nothing was changed. Two identities under one name come about in three ways. \
         If this file is another set you mean to take into {set}, use action \"absorb\" with it. If \
         the two are copies of one set that have diverged -- a fork renamed without deleting its \
         `set_id`, or a clone taken from a file that declared none -- repair the copy on the wrong \
         identity: `drop` it (a safety copy is written first), `create` it again and import this \
         file, so the empty set adopts {declared}; then put back by hand anything the safety copy \
         holds that the file does not. If the file's copy is the wrong one, do that on the machine \
         that wrote it instead. If they are two different sets whose owners chose the same name, \
         they cannot both be loaded here under it: opening a set under an alias is not built yet, \
         so keep this file out of {set}'s imports and exports until it is.",
        path.display()
    )
}

/// What a file read by `absorb` or `evaluate` must declare: the named set, or any shared one.
fn named_or_any(set: Option<&SetName>) -> memory_file::Expectation {
    set.map_or(memory_file::Expectation::AnyShared, |set| {
        memory_file::Expectation::Exactly(Scope::Shared(set.clone()))
    })
}

/// The refusal for a set a reserved directory holds no file for, naming what it does hold.
///
/// Named, because the likeliest reasons are a set exported under another name or to another
/// checkout, and the listing is what tells those apart.
fn absent_from(home: &Path, scope: &Scope) -> String {
    let held = memory_files_in(home).unwrap_or_default();
    let holds = if held.is_empty() {
        "no memory files at all".to_owned()
    } else {
        held.iter()
            .map(|(path, header)| {
                format!(
                    "{} holding {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    header.scope.as_key()
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{} holds no file for the set {}: no header there carries its identity or declares its \
         name. It holds {holds}. If the set was exported to another checkout, name that one; if \
         its file is somewhere else, name it with `file`.",
        home.display(),
        scope.as_key()
    )
}

/// The name for a set's first file in a reserved directory: close to the set's name, and legal on
/// every operating system.
///
/// `documents/design/set-lifecycle.md`, *a file's name need not match its set's name*. Nothing
/// relies on the name after this, so it only has to be safe and close -- never exact.
///
/// - A Windows device name before the first dot takes `-set`, so `con` is written `con-set.json`.
/// - A name any entry here already has, **ignoring case**, takes a number: `notes-2.json`. A Mac
///   folds case, so `Notes` and `notes` would otherwise be two sets writing one file.
///
/// Set names are ASCII, so nothing here normalises Unicode; the design says what to add if that
/// ever widens.
fn new_file_in(home: &Path, scope: &Scope) -> Result<PathBuf, String> {
    let name = match scope {
        Scope::Local => "local",
        Scope::Shared(name) => name.as_str(),
    };
    let (first, rest) = name
        .split_once('.')
        .map_or((name, None), |(a, b)| (a, Some(b)));
    let stem = if WINDOWS_RESERVED_NAMES.contains(&first.to_ascii_lowercase().as_str()) {
        match rest {
            Some(rest) => format!("{first}-set.{rest}"),
            None => format!("{first}-set"),
        }
    } else {
        name.to_owned()
    };

    let taken: HashSet<String> = match std::fs::read_dir(home) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashSet::new(),
        Err(error) => return Err(format!("could not list {}: {error}", home.display())),
    };
    let mut candidate = format!("{stem}.{MEMORY_FILE_EXTENSION}");
    let mut number = 2;
    while taken.contains(&candidate.to_lowercase()) {
        candidate = format!("{stem}-{number}.{MEMORY_FILE_EXTENSION}");
        number += 1;
    }
    Ok(home.join(candidate))
}

/// Refuse a `directory` that is already the reserved one, rather than nesting inside it.
///
/// `export-appends-the-reserved-directory-to-its-own-argument`. Both actions take the **project
/// root** and append `scatterhoard_mem` themselves, so passing the memory directory wrote
/// `scatterhoard_mem/scatterhoard_mem/memories.json` and reported success with the doubled
/// path. Nothing warned, and the store's `bases` table shows an earlier session made the same
/// mistake unnoticed — a second identity for what a human means as one target.
///
/// **Refused rather than quietly corrected.** Stripping the segment would guess, and the guess is
/// wrong for anyone whose project root is genuinely named `scatterhoard_mem`; the refusal names the
/// remedy instead, which is the standing rule that a message telling its reader to fix something
/// must name an action its reader can take.
///
/// `Path::file_name` is what makes a trailing separator harmless: it reports the last real segment,
/// so `.../scatterhoard_mem` and `.../scatterhoard_mem/` are the same directory to this check, as
/// they are to everything else.
fn nested_reserved_directory(directory: &std::path::Path) -> Option<String> {
    if directory.file_name()? != std::ffi::OsStr::new(RESERVED_DIRECTORY) {
        return None;
    }
    Some(format!(
        "{} is already the `{RESERVED_DIRECTORY}` directory, and both `export` and `import` look \
         inside a `{RESERVED_DIRECTORY}` under what you give them -- so this would have written one \
         inside the other, where nothing would ever read it. Pass the parent instead: for a \
         project, the root of the checkout.",
        directory.display()
    ))
}

/// Where an export is to be written: a checkout to append the well-known path to, or a file to
/// write exactly.
///
/// `documents/design/set-lifecycle.md`, *the fixed export path is a property of the trigger, not of
/// the format*. The reserved directory exists so that **auto-discovery in a checkout** can settle
/// trust from the path before reading a byte of the contents. Nothing auto-discovers a backup or a
/// set kept in a synced folder: those are read by a caller naming the scope and the location, and
/// `check_scope` makes the file's header agree-or-refuse against what the caller stated. So for a
/// set that never sits in a checkout the reserved directory contributes no trust and only friction
/// — before several sets could share one, each unpublished set in a synced folder needed a wrapper
/// of its own, and every leaf file was named `memories.json`, indistinguishable in a listing, a
/// search result, or a sync-conflict notification, which is exactly where a person meets them.
///
/// **The caller decides, not the scope.** Varying the layout by scope would make publishing a set
/// into a file move and break *publishing later changes nothing structural*; the caller is the only
/// participant that knows whether it is writing into a checkout — the same reasoning that already
/// makes the directory come from the caller on every call rather than from stored state.
///
/// Externally tagged and flattened into the request, so `{"directory": …}` and `{"file": …}` each
/// name one variant and neither *both* nor *neither* is a value this type can hold.
/// **Public because the command line constructs one**, not because the tool surface needs it to be.
/// `export-subcommand` reaches [`Server::export_to`] with a destination it built from arguments, and
/// publishing this type is what lets it state which of the two forms it means. The alternative was
/// to let the binary infer the form from the path, which is the one thing only the caller knows.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    /// A checkout root, or any directory: the set's file is found in `scatterhoard_mem` under it.
    Directory(std::path::PathBuf),
    /// An exact file, written where it says and nowhere else.
    File(std::path::PathBuf),
}

impl Destination {
    /// The file an export writes: the set's own, if the directory holds one, or a new one named
    /// for it.
    fn file_to_write(&self, scope: &Scope, identity: Option<SetId>) -> Result<PathBuf, String> {
        match self {
            Self::Directory(directory) => {
                refuse_local_in_reserved(scope)?;
                let home = directory.join(RESERVED_DIRECTORY);
                match locate_in(&home, scope, identity)? {
                    Some(found) => Ok(found),
                    None => new_file_in(&home, scope),
                }
            }
            Self::File(file) => Ok(file.clone()),
        }
    }

    /// The file an import or a restore reads: the set's own, which has to be there.
    fn file_to_read(&self, scope: &Scope, identity: Option<SetId>) -> Result<PathBuf, String> {
        match self {
            Self::Directory(directory) => {
                refuse_local_in_reserved(scope)?;
                let home = directory.join(RESERVED_DIRECTORY);
                locate_in(&home, scope, identity)?.ok_or_else(|| absent_from(&home, scope))
            }
            Self::File(file) => Ok(file.clone()),
        }
    }

    /// The file to read for a caller that holds no set under the name it is reading -- `absorb`
    /// and `evaluate`, each of which reads another set's file by design.
    ///
    /// **In `directory` form the set has to be named**, because the header search is the whole of
    /// what that form does and there is nothing else to search for: the store is not theirs, so it
    /// has no identity to offer. The name is matched against what each header declares, as a first
    /// `import` does. Required even when the directory holds one file, so that a request which
    /// works today cannot start failing when a second set is exported beside it.
    ///
    /// In `file` form the path already says which file; a name, if given, is checked against the
    /// header by the reader rather than used to find anything.
    fn file_of_named(&self, set: Option<&SetName>) -> Result<PathBuf, String> {
        match self {
            Self::Directory(directory) => {
                let Some(set) = set else {
                    return Err(format!(
                        "reading from a `directory` needs `set`, the name of the set whose file to \
                         read: its `{RESERVED_DIRECTORY}` can hold several, and the headers are \
                         searched for that name. To read one exact file instead, pass `file`."
                    ));
                };
                let home = directory.join(RESERVED_DIRECTORY);
                let scope = Scope::Shared(set.clone());
                locate_in(&home, &scope, None)?.ok_or_else(|| absent_from(&home, &scope))
            }
            Self::File(file) => Ok(file.clone()),
        }
    }

    /// Which form the caller used, for the response.
    ///
    /// Reported because two callers can name the same bytes on disk by different arguments, and a
    /// transcript that cannot tell them apart cannot show that a set was written somewhere nothing
    /// will ever discover it.
    fn form(&self) -> &'static str {
        match self {
            Self::Directory(_) => "directory",
            Self::File(_) => "file",
        }
    }

    /// The reconciliation target this destination is, as the caller spelled it.
    ///
    /// **An explicit file is its own replica.** The directory form keys bases on the directory,
    /// which is right because a store holds one set: the directory and the store's identity
    /// together pick out one file there, whatever it is called.
    /// The explicit form breaks that correspondence on purpose — two sets backed up into one synced
    /// folder are two replicas — so keying on the shared parent would merge two files' agreements
    /// into one and supply each with the other's ancestor.
    ///
    /// Unnormalised, exactly as the directory form is: `target-identity-is-an-unnormalised-path-string`
    /// in `TRACKING.md` covers both, and normalising only this one would leave the two forms
    /// disagreeing about what a target is on top of the defect already filed.
    fn target(&self) -> String {
        match self {
            Self::Directory(directory) => directory.to_string_lossy().into_owned(),
            Self::File(file) => file.to_string_lossy().into_owned(),
        }
    }

    /// Refuse a destination that cannot be used, in this project's voice.
    ///
    /// **Shared by export and import, and worded for both.** They are the two ends of one round
    /// trip: a set is written to a place and read back from the same place, and a rule that held on
    /// only one end would let a file be written somewhere it could never be read from — which is
    /// the trap the two forms exist together to close.
    ///
    /// Both forms answer to the reserved-directory guard, and that is the constraint the design
    /// attaches to the explicit form: `nested_reserved_directory` applies to the directory itself,
    /// or to an explicit file's **parent**.
    fn refusal(&self) -> Option<String> {
        match self {
            Self::Directory(directory) => {
                if let Some(refusal) = nested_reserved_directory(directory) {
                    return Some(refusal);
                }
                if !directory.is_dir() {
                    return Some(format!(
                        "{} is not a directory that exists. `directory` names the directory the \
                         memory file sits in -- for a project, the root of the checkout -- and the \
                         set's file is found in `{RESERVED_DIRECTORY}` under it. To name an exact \
                         file instead, for a backup or a synced folder, pass `file`.",
                        directory.display()
                    ));
                }
                None
            }
            Self::File(file) => {
                if file.is_dir() {
                    return Some(format!(
                        "{} is a directory, and `file` names one exact file. Pass it as `directory` \
                         instead, to use the set's file in `{RESERVED_DIRECTORY}` inside it, or name \
                         a file within it.",
                        file.display()
                    ));
                }
                // The parent, not the file. `file_name` on the parent is the same recognition the
                // directory form uses, so a trailing separator is harmless here too.
                let parent = file
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty());
                let Some(parent) = parent else {
                    return Some(format!(
                        "{} names no directory to work in. `file` takes the whole path of the \
                         memory file.",
                        file.display()
                    ));
                };
                if parent.file_name() == Some(std::ffi::OsStr::new(RESERVED_DIRECTORY)) {
                    // **Refused on both sides, and the reason differs on each.** Writing into the
                    // reserved directory hand-places a file in the namespace that is trusted from
                    // its path alone, behind the unexpected-contents warning at the export gate.
                    // Reading from it is worse in a quieter way: `<root>/scatterhoard_mem/…` is
                    // reachable by both forms, and the two forms are two reconciliation targets, so
                    // one file named both ways would carry two independent records of agreement and
                    // read as settled by one spelling and diverged by the other.
                    return Some(format!(
                        "{} is inside the reserved `{RESERVED_DIRECTORY}` directory, which \
                         `directory` already reaches: anything there is read as this project's \
                         memory set from the path alone. Naming it as a `file` would make one file \
                         two reconciliation targets, each with its own record of what was last \
                         agreed. Pass the checkout root as `directory` instead, or -- for a backup \
                         -- name a `file` outside the reserved directory.",
                        file.display()
                    ));
                }
                if !parent.is_dir() {
                    return Some(format!(
                        "{} is not a directory that exists, so there is no {} to read or write. \
                         `file` names the whole path of the memory file, and the directory holding \
                         it has to be there already.",
                        parent.display(),
                        file.display()
                    ));
                }
                None
            }
        }
    }
}

/// Refuse a write whose prose carries prohibited data.
///
/// `documents/design/prohibited-data.md`. **Refused at the write and only warned about at export**,
/// which is not a lapse of nerve at the export end: a write is one memory that does not yet exist
/// and is definitionally the caller's to reword, while an export is a whole set that may hold
/// imported memories the caller cannot properly fix -- and because the filter gains rules over time,
/// blocking there would let a filter improvement retroactively close a set that exported fine
/// yesterday. Refuse where the caller holds the remedy; report where they may not.
///
/// **The refusal never quotes what matched.** A message travels into a transcript and possibly into
/// a log, and repeating the credential would spread the thing being contained.
fn prohibited_data_refusal(fields: &[(&str, &str)]) -> Option<String> {
    let mut found: Vec<String> = Vec::new();
    for (name, text) in fields {
        let mut categories: Vec<String> = prohibited::scan(text)
            .into_iter()
            .map(|finding| serde_name(&finding.category).to_owned())
            .collect();
        categories.sort();
        categories.dedup();
        for category in categories {
            found.push(format!("{category} in `{name}`"));
        }
    }
    if found.is_empty() {
        return None;
    }
    Some(format!(
        "Nothing was stored: this text carries what looks like prohibited data -- {}. \
         `documents/design/prohibited-data.md` keeps secret API keys, passwords and card numbers \
         out of memory sets entirely, because a memory set is written to a repository and a secret \
         that reaches one must be treated as compromised. Put the value in a file and record where \
         the file is, then store the memory referring to it. The match is on shape rather than on \
         wording, so writing *about* credentials is fine and only something formatted like one is \
         refused.",
        found.join(", ")
    ))
}

/// A link whose target this machine could not resolve, as reported to the caller.
///
/// Carries the label as well as the id because a dangling `SUPERSEDES` and a dangling `RELATED_TO`
/// are not equally bad — `documents/design/cross-set-references.md` leaves that severity question
/// open, and reporting the label is what lets it be answered later without a new mechanism.
#[derive(Debug, Serialize)]
struct UnresolvedLink {
    label: LinkLabel,
    target: MemoryId,
    /// The set named by the link, absent for a same-set target.
    #[serde(skip_serializing_if = "Option::is_none")]
    set: Option<String>,
    state: LinkState,
}

/// Why a link target did not resolve.
///
/// Three states rather than a boolean, because *checked and absent* and *not checked at all* are
/// different facts and a caller acts on them differently: the first is worth a second look at what
/// was written, the second says only that this machine does not hold the set.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum LinkState {
    /// Checked against the set being written to, which does not hold it.
    NoSuchIdInThisSet,
    /// Checked against the set the link names, which does not hold it.
    NoSuchIdInThatSet,
    /// Not checked: the named set is not loaded here.
    SetNotLoaded,
}

/// Attach a link report to a write's response, if there is anything to say.
///
/// One function rather than a projection at each call site: `hoard_memorize` and `amend` must report
/// this identically, and two hand-written copies of one response shape is the defect
/// `lookup-and-recall-disagree-on-shape` records.
/// Refuse a link that would assert authority over a memory in another set.
///
/// `documents/design/cross-set-references.md`: **no edge changes the standing of a memory in
/// another set.** Three labels change their target's standing when traversal acts on them, so a
/// cross-set one is an assertion of authority over a set whose maintainers never agreed to it, and
/// nothing can rank two shared sets against each other.
///
/// **This is the honest-client half, and it is not the half that protects anybody.** The file
/// carrying a hostile edge was never written by this client, so a refusal here would be security
/// theatre on its own; what protects a reader is `Store::edges` declining to act on such an edge at
/// resolution. This exists so our own tool cannot author one by accident, which is worth having for
/// the same reason a lint is worth having next to a test.
///
/// **`SUPERSEDED_BY` is deliberately exempt.** It names its own source, so it is a memory conceding
/// its own standing rather than reaching for someone else's -- and it is the only form that survives
/// the replacement's set being unavailable, which is the whole reason the label exists.
///
/// **At the tool surface, not in `Store::insert`**, which import shares: a per-record refusal there
/// costs the whole file, and an imported file is exactly where such an edge legitimately appears --
/// it must be storable and then ignored, not rejected at the door.
/// Refuse a supersession that does not say what the replacement is indexed under.
///
/// **Neither default is safe, which is why this refuses rather than choosing one.** Carrying the
/// superseded memory's keywords over silently is one failure — the replacement becomes findable
/// under terms nobody chose for it. Letting them default to nothing is the other, and it is the one
/// measured in a different memory system, where superseding silently dropped the record's keys.
/// `a-keyword-cannot-be-retired-once-a-memory-has-been-exported` in `TRACKING.md` records both.
///
/// **Absence, not emptiness.** `[]` is a replacement genuinely indexed under nothing, which is a
/// decision a caller is entitled to make; saying nothing at all is a call that did not consider the
/// question. Only `Option` can tell those apart, which is why `keywords` stopped being a defaulted
/// `Vec`. `swallowed_parameters` draws the same line for the same reason.
///
/// **Narrow on purpose.** `keywords` stays optional everywhere else. Requiring it throughout would
/// be a larger decision governed by `documents/design/decision-placement.md`, and the standing
/// reluctance to reach for required-ness — *"do not repair this by making `why` required, which
/// trades away a decided property"* — applies to it. What is required here is only that a caller
/// already deciding a memory's index says so, because they are deciding it either way.
/// How many ids each category of an export's `replaced_file` report lists, and how many a refusal
/// names. The count is always exact.
///
/// A cap because one format change can alter every record in a set, and a report listing all of them
/// would bury the few a reader needs to look at.
const REPLACED_IDS_LISTED: usize = 20;

/// What an export found at its destination before writing.
///
/// `export-reports-what-the-file-did-not-hold`, and `documents/design/export-review.md` under *What
/// the file being replaced did not hold*. **A question about files, not about authorship**: both
/// sides are checksummed by the canonical form reconciliation uses, so nothing is stored and no
/// session has to be identified.
///
/// Three cases rather than a boolean, because two of them call for opposite handling: nothing at
/// the destination is written freely, and a file that could not be read is refused.
enum Replaced {
    /// Nothing was at the destination, so every record the export writes is new.
    Absent {
        /// How many records the export writes.
        written: usize,
    },
    /// A file was there and could not be read as this set's memory file, for this reason.
    Unreadable(String),
    /// The file was read, and its records compared with what the export is about to write.
    Compared {
        new: Vec<MemoryId>,
        changed: Vec<MemoryId>,
        removed: Vec<MemoryId>,
        unchanged: usize,
        /// The set identity the replaced file's header declared, if any.
        declared: Option<SetId>,
    },
}

impl Replaced {
    /// Compare the records an export is about to write against the file at its destination.
    fn find(path: &std::path::Path, scope: &Scope, written: &[(MemoryId, u64)]) -> Self {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Self::Absent {
                    written: written.len(),
                };
            }
            Err(error) => return Self::Unreadable(error.to_string()),
        };

        let mut held: std::collections::HashMap<MemoryId, u64> = std::collections::HashMap::new();
        let expected = memory_file::Expectation::Exactly(scope.clone());
        let declared = match memory_file::stream(&expected, file, |record| {
            held.insert(record.id(), crate::checksum::of_record(record));
            Ok(())
        }) {
            Ok(header) => header.set_id,
            Err(error) => return Self::Unreadable(error.to_string()),
        };

        let mut new = Vec::new();
        let mut changed = Vec::new();
        let mut unchanged = 0usize;
        for (id, checksum) in written {
            match held.remove(id) {
                None => new.push(*id),
                Some(before) if before == *checksum => unchanged += 1,
                Some(_) => changed.push(*id),
            }
        }
        // What is left was in the file and is not being written.
        let mut removed: Vec<MemoryId> = held.into_keys().collect();
        removed.sort_unstable();

        Self::Compared {
            new,
            changed,
            removed,
            unchanged,
            declared,
        }
    }

    /// Why the export must not write over this file, if it declared another identity.
    ///
    /// **Refused since 2026-09-26, where it had been reported** (`identity-mismatch-refusal`). The
    /// case that decided it is a diverged copy: its export writes its own identity over the shared
    /// file's, after which every other collaborator mismatches and only the machine at fault was
    /// ever told. Refusing puts the cost on the machine that made the mistake.
    fn identity_refusal(
        &self,
        path: &Path,
        scope: &Scope,
        writing: Option<SetId>,
    ) -> Option<String> {
        let Self::Compared {
            declared: Some(declared),
            ..
        } = self
        else {
            return None;
        };
        let writing = writing?;
        (*declared != writing)
            .then(|| identity_mismatch(path, scope, *declared, writing, Crossing::Export))
    }

    /// Why the export must not write, if it must not.
    ///
    /// **Refused when writing would lose records, or when what it would lose cannot be told.**
    /// Decided 2026-09-14 by the human partner, after a first build that only reported: export
    /// replaces the file atomically and never checks the recorded bases against the file on disk, so
    /// records a pull brought in and nobody imported were overwritten with no sign but lines
    /// vanishing from a diff -- and a file with unresolved merge conflicts, overwritten, loses the
    /// other side of the merge.
    ///
    /// **The way through is a filesystem act, not a parameter.** Moving the file aside leaves a copy
    /// of what an intended overwrite replaced, and the decision stays out of the call, where a
    /// refused caller could simply set it. A force option was considered and left for later, if the
    /// friction proves real, because a parameter is hard to withdraw once copies of the code are in
    /// use. Nothing at the destination is never refused.
    fn refusal(&self, path: &std::path::Path) -> Option<String> {
        match self {
            Self::Absent { .. } => None,
            Self::Compared { removed, .. } if removed.is_empty() => None,
            Self::Compared { removed, .. } => Some(format!(
                "nothing was written: {} holds {} record(s) this set does not, and exporting over it \
                 would drop them: {}. They most likely arrived by a pull, or by another copy's \
                 export, and were never imported. To keep them, import from this same destination, \
                 then export again. If replacing the file is intended -- after a `restore`, say -- \
                 move it aside first, so the records it holds survive, or delete it if they are kept \
                 somewhere else; then export.",
                path.display(),
                removed.len(),
                named_ids(removed)
            )),
            Self::Unreadable(reason) => Some(format!(
                "nothing was written: {} is there but cannot be read as this set's memory file \
                 ({reason}), so what exporting over it would lose cannot be told. The likeliest \
                 cause is an unresolved merge conflict: resolve it, import from this destination, \
                 then export again. If replacing the file is intended, move it aside first, so \
                 whatever it holds survives, or delete it; then export.",
                path.display()
            )),
        }
    }

    /// The set identity the file at the destination declares, if it was read and declares one.
    fn declared(&self) -> Option<SetId> {
        match self {
            Self::Compared { declared, .. } => *declared,
            Self::Absent { .. } | Self::Unreadable(_) => None,
        }
    }

    /// The `replaced_file` report for an export that went ahead.
    fn report(&self) -> Value {
        match self {
            // Every record is new, and saying so is the point: a count of zero would read as
            // *nothing changed*, which is the opposite answer. The path is left to `written`, beside
            // this in the same result: repeating it would say nothing new, and would make the note
            // differ on every machine.
            Self::Absent { written } => json!({
                "state": "none",
                "note": format!(
                    "nothing was at the destination before this export, so all {written} records \
                     written are new."
                ),
            }),
            // Refused before any report is made; stated rather than left unreachable, so a caller
            // that reports without refusing still says something true.
            Self::Unreadable(reason) => json!({
                "state": "unreadable",
                "reason": reason,
            }),
            // `removed` is always empty on an export that went ahead, and is reported anyway: an
            // explicit zero is the statement that nothing was dropped.
            Self::Compared {
                new,
                changed,
                removed,
                unchanged,
                ..
            } => json!({
                "state": "compared",
                "new": listed_ids(new),
                "changed": listed_ids(changed),
                "removed": listed_ids(removed),
                "unchanged": unchanged,
            }),
        }
    }
}

fn listed_ids(ids: &[MemoryId]) -> Value {
    json!({
        "count": ids.len(),
        "ids": ids
            .iter()
            .take(REPLACED_IDS_LISTED)
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
    })
}

/// Ids for a sentence: the first [`REPLACED_IDS_LISTED`], and how many more there are.
fn named_ids(ids: &[MemoryId]) -> String {
    let shown = &ids[..ids.len().min(REPLACED_IDS_LISTED)];
    let mut text = shown
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if ids.len() > shown.len() {
        text.push_str(&format!(" and {} more", ids.len() - shown.len()));
    }
    text
}

/// Refuse a memory longer than a memory should be, and say what to do instead.
///
/// **Measured over what the embedder will receive**, via
/// [`embedding_input_of`](crate::embedding::embedding_input_of), so the length refused here is the
/// length that would have been embedded rather than a second count of the same text.
///
/// The message names the measured length as well as the limit, because *too long* without a number
/// leaves the caller guessing how much to cut — and names splitting, because the limit exists to
/// produce several linked memories rather than one shorter one. Cutting the text to fit and keeping
/// both claims in it satisfies the number and defeats the reason.
fn over_length_refusal(content: &str, why: Option<&str>, keywords: &[Keyword]) -> Option<String> {
    let input = crate::embedding::embedding_input_of(content, why, keywords);
    let length = input.chars().count();
    if length <= crate::embedding::MAX_EMBEDDING_INPUT_CHARS {
        return None;
    }
    Some(format!(
        "this memory is {length} characters of content, `why` and keywords together, and the \
         limit is {}. Split it: a memory that long is usually several claims, and the relationships \
         between them are what link labels are for -- buried in prose they cannot be followed, and a \
         later memory cannot point at one of them without pointing at all of them. Store each claim as \
         its own memory and link them. Shortening the text while keeping every claim in it meets the \
         number and misses the reason.",
        crate::embedding::MAX_EMBEDDING_INPUT_CHARS
    ))
}

fn unstated_keywords_refusal(links: &[Link], keywords: Option<&[Keyword]>) -> Option<String> {
    if keywords.is_some() {
        return None;
    }
    let superseded = links
        .iter()
        .find(|link| matches!(link.label, LinkLabel::Supersedes))?;

    Some(format!(
        "nothing was stored: this call supersedes `{}` and does not say what the new memory's \
         keywords are. They are not carried over, because the superseded memory's terms were \
         chosen for what it said, and they are not left empty, because a replacement nobody can \
         find by keyword is how a memory quietly stops being reachable. State the whole set the \
         replacement should be indexed under -- usually the old set, edited. If it genuinely \
         belongs under none, send `\"keywords\": []` and that is stored as written.",
        superseded.target.id()
    ))
}

/// Refuse a link whose label this build does not know, unless it restates one from `held`.
///
/// **Restated, never authored** -- `documents/design/memory-data-model.md`, *unknown labels are
/// carried opaquely*. A label from a newer build arrives in a file; to keep it across a supersession
/// or an amendment, both of which replace the whole link array, a caller has to be able to send it
/// back. Copying needs no knowledge of the label, so a pair already on the memory being replaced may
/// be restated: the same spelling, byte for byte, and the same target id -- the pair
/// [`link_deltas`] compares. Anything else unknown is refused, which is how a typo is still caught
/// now that the schema no longer publishes the vocabulary as an `enum`.
///
/// `held` is the links of the memory being superseded or amended, and nothing else: a label copied
/// from a bystander is being authored here, whatever memory it was copied from.
fn unknown_label_refusal(links: &[Link], held: &[Link]) -> Option<String> {
    let restates = |link: &Link| {
        held.iter()
            .any(|old| old.label == link.label && old.target.id() == link.target.id())
    };
    let authored: Vec<String> = links
        .iter()
        .filter(|link| !link.label.is_recognised() && !restates(link))
        .map(|link| format!("`{}` to {}", link.label.name(), link.target.id()))
        .collect();
    if authored.is_empty() {
        return None;
    }
    Some(format!(
        "nothing was written: {} not a link label this build knows. The labels are {}. A label \
         outside them is accepted only when it restates, spelled exactly the same and pointing at \
         the same target, a link the memory being superseded or amended already carries -- which \
         is how a label written by a newer build is kept. If this was meant as one of the labels \
         above, send it spelled that way.",
        match authored.as_slice() {
            [one] => format!("{one} is"),
            many => format!("{} are", many.join(", ")),
        },
        label_vocabulary(),
    ))
}

/// The vocabulary, as a sentence for the descriptions of the two tools that take `links`.
///
/// **In the tool's own description as well as the property's**, since 2026-09-28. Claude Desktop
/// was measured dropping every property description but `action`'s
/// (`documents/design/tool-surface.md`), and it kept the `enum` this replaced -- so without this, a
/// session there would see `label` as a bare string with no vocabulary at all. Generated from
/// [`LinkLabel::ALL`] like the property's text, so the two copies cannot disagree.
fn link_label_sentence() -> String {
    format!(
        "A link's `label` is one of {}; one copied from the memory being superseded or amended \
         keeps its label as written.",
        label_vocabulary()
    )
}

/// The vocabulary as prose: every known label, each in backticks.
fn label_vocabulary() -> String {
    LinkLabel::ALL
        .iter()
        .map(|label| format!("`{}`", label.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The links of every memory this call supersedes that this store holds -- the only ones a write
/// may restate an unknown label from. The same selection [`link_deltas`] makes, for the same
/// reason: a foreign target is not this store's to read.
fn superseded_links(
    store: &Store,
    links: &[Link],
    scope: &Scope,
    holder: Option<crate::memory::SetId>,
) -> Result<Vec<Link>, String> {
    let mut held = Vec::new();
    for link in links {
        if link.label != LinkLabel::Supersedes || link.target.is_foreign_to(scope, holder) {
            continue;
        }
        if let Some(old) = store
            .get(link.target.id())
            .map_err(|error| format!("could not read the memory being superseded: {error}"))?
        {
            held.extend(old.links);
        }
    }
    Ok(held)
}

fn cross_set_authority_refusal(
    links: &[Link],
    scope: &Scope,
    holder: Option<crate::memory::SetId>,
) -> Option<String> {
    let offending: Vec<&Link> = links
        .iter()
        .filter(|link| {
            // `is_foreign_to` rather than `set().is_some()`: a target may name the very set being
            // written into, which is a same-set correction spelled the long way and the commonest
            // thing this rule must *not* catch. See
            // `a-same-set-link-spelled-with-its-set-name-is-inert`. Given the set's identity so it
            // agrees with the write about which links are this set's own.
            link.target.is_foreign_to(scope, holder)
                && matches!(
                    link.label,
                    LinkLabel::Supersedes | LinkLabel::ExceptionTo | LinkLabel::Contradicts
                )
        })
        .collect();
    let first = offending.first()?;
    let set = first.target.set().expect("filtered on being cross-set");

    Some(format!(
        "a `{}` link cannot point into another set: it would change the standing of a memory in \
         `{set}`, and no set outranks another. Two ways forward, depending on what you meant. If \
         the memory being replaced is **yours**, record the obsolescence on it instead -- a \
         `superseded_by` link may cross a set boundary, because that is a memory conceding its own \
         standing rather than reaching for someone else's. If you are disagreeing with **someone \
         else's** memory, store your claim in your own set and link it with `related_to` or \
         `refers_to`, which associates the two without either one silencing the other.",
        serde_name(&first.label)
    ))
}

/// What a supersession did to the superseded memory's keywords.
///
/// Reported per superseded target, because one call may supersede several — the convention for
/// splitting a partly-stale memory produces exactly that shape.
#[derive(Serialize)]
struct KeywordDelta {
    /// The memory being superseded.
    superseded: MemoryId,
    /// Terms the replacement carries that the superseded memory did not.
    added: Vec<Keyword>,
    /// **Terms that stopped reaching anything.** The load-bearing half: after this call, nothing in
    /// the set answers to them unless another memory happens to carry them too.
    dropped: Vec<Keyword>,
    /// Terms both carry.
    retained: Vec<Keyword>,
}

/// Compare a replacement's keywords against each memory it supersedes.
///
/// **Only same-set targets this store holds.** A cross-set target's keywords are not ours to read
/// and may not be here at all, and reporting an empty delta for one would say *nothing was dropped*
/// where the truth is *nothing was checked* — the shape
/// `prohibited-data-scanned-conflates-skipped-with-unbuilt` was filed for.
fn keyword_deltas(store: &Store, links: &[Link], keywords: &[Keyword]) -> Vec<KeywordDelta> {
    let mut deltas = Vec::new();
    for link in links {
        if link.label != LinkLabel::Supersedes {
            continue;
        }
        let LinkTarget::SameSet(target) = link.target else {
            continue;
        };
        let Ok(Some(old)) = store.get(target) else {
            continue;
        };
        // Compared on the folded form, because that is what matching uses: two spellings that
        // differ only by case are one term, and reporting them as a drop and an addition would be
        // a change the search path cannot see.
        let folded: Vec<String> = keywords.iter().map(Keyword::folded).collect();
        let was: Vec<String> = old.keywords.iter().map(Keyword::folded).collect();
        deltas.push(KeywordDelta {
            superseded: target,
            added: keywords
                .iter()
                .filter(|k| !was.contains(&k.folded()))
                .cloned()
                .collect(),
            dropped: old
                .keywords
                .iter()
                .filter(|k| !folded.contains(&k.folded()))
                .cloned()
                .collect(),
            retained: keywords
                .iter()
                .filter(|k| was.contains(&k.folded()))
                .cloned()
                .collect(),
        });
    }
    deltas
}

/// What a supersession did to the superseded memory's out-links.
///
/// Step 1 of `out-links-stay-with-the-record-that-wrote-them`: an out-link is the claim of the record
/// that wrote it, so one the replacement does not restate stays with the superseded memory. Reported
/// before the walk stops inheriting such links, so that cutting inheritance removes nothing an
/// author did not see go.
#[derive(Serialize)]
struct LinkDelta {
    /// The memory being superseded.
    superseded: MemoryId,
    /// **Links the replacement does not restate.** The load-bearing half, and the same shape as
    /// `dropped` in [`KeywordDelta`]: a deliberate drop and a forgotten re-typing look identical.
    left_behind: Vec<Link>,
    /// Links the replacement restates.
    restated: Vec<Link>,
}

/// Compare a replacement's links against each memory it supersedes.
///
/// **On label and target id, not on the whole target.** Ids are unique across sets, and the rest of
/// a cross-set target is apparatus -- `target_keywords` exist to re-find a dangling reference -- so a
/// caller who re-typed them differently has still restated the claim. A changed label is a different
/// claim, and counts as left behind.
///
/// **The superseded memory's own `supersedes` and `superseded_by` are excluded.** They are its
/// history rather than claims about anything, and a replacement records the chain with its own new
/// link instead of copying the old one.
///
/// Same-set targets this store holds only, for the reason [`keyword_deltas`] gives. Unlike it, a
/// target spelled the long way with this set's own name is still this set's, which is what
/// [`LinkTarget::is_foreign_to`] answers.
fn link_deltas(
    store: &Store,
    links: &[Link],
    scope: &Scope,
    holder: Option<crate::memory::SetId>,
) -> Vec<LinkDelta> {
    let claims =
        |link: &&Link| !matches!(link.label, LinkLabel::Supersedes | LinkLabel::SupersededBy);
    let restates = |old: &Link| {
        links
            .iter()
            .any(|new| new.label == old.label && new.target.id() == old.target.id())
    };
    let mut deltas = Vec::new();
    for link in links {
        if link.label != LinkLabel::Supersedes || link.target.is_foreign_to(scope, holder) {
            continue;
        }
        let target = link.target.id();
        let Ok(Some(old)) = store.get(target) else {
            continue;
        };
        let (restated, left_behind) = old
            .links
            .iter()
            .filter(claims)
            .cloned()
            .partition(|old| restates(old));
        deltas.push(LinkDelta {
            superseded: target,
            left_behind,
            restated,
        });
    }
    deltas
}

/// The links into each memory a supersession replaces, which recall will now walk to the replacement.
///
/// **The in-link half of what a supersession does to the graph**, and the half nobody chose. The
/// walk resolves an edge into a superseded memory onto its tip, so `C SUPPORTS A` is followed as
/// `C SUPPORTS B` once B supersedes A: a claim C's author made about A's text and never about B's.
/// Both records stay intact, so the relationship exists only in the walk, with no diff or timestamp
/// for anyone to find. `supersession-silently-repoints-an-in-link-nobody-reauthored`.
///
/// Links from other loaded sets are included, because the walk re-points them too. Supersession
/// links are not: following a chain to its newest member is what they are for, and the line is the
/// one [`link_deltas`] draws.
fn repointed_in_links(
    stores: &[&Store],
    scope: &Scope,
    deltas: &[LinkDelta],
) -> Result<Vec<Value>, crate::store::StoreError> {
    let claims =
        |label: &LinkLabel| !matches!(label, LinkLabel::Supersedes | LinkLabel::SupersededBy);
    let Some(store) = stores.iter().find(|store| store.scope() == scope) else {
        return Ok(Vec::new());
    };
    let mut repointed = Vec::new();
    for delta in deltas {
        let superseded = delta.superseded;
        for edge in store.incoming_edges(superseded)? {
            if claims(&edge.label) {
                repointed.push(json!({
                    "superseded": superseded,
                    "source": edge.source,
                    "label": edge.label,
                }));
            }
        }
        for reference in referenced_from_other_sets(stores, scope, superseded)? {
            if claims(&reference.label) {
                repointed.push(json!({
                    "superseded": superseded,
                    "source": reference.source,
                    "label": reference.label,
                    "scope": WireScope(&reference.scope),
                }));
            }
        }
    }
    Ok(repointed)
}

/// Name the in-links a supersession re-points, and say what they mean. A remark, never a refusal:
/// the caller holds both texts and can act in the same turn, which is when this codebase remarks.
fn report_repointed_in_links(result: &mut Value, repointed: Vec<Value>) {
    if repointed.is_empty() {
        return;
    }
    result["repointed_in_links"] = Value::Array(repointed);
    result["repointed_in_links_note"] = json!(
        "Stored. These memories point at one this memory supersedes, and recall now walks each of \
         those links to this memory instead -- so a link written about the old text is read as a \
         claim about this one, though its author never saw it. Nothing was refused, and nothing \
         needs doing unless a label does not hold of this memory. Where one does not, the claim \
         belongs to the memory holding the link: correct its links with hoard_correct's amend, or \
         supersede it if the amend is refused. One from another set (`scope`) is that set's to \
         correct. Supersession links are left out, because following a chain to its newest member \
         is what they are for."
    );
}

/// State the link delta and say what to do about it. Never refuse: leaving a link behind is
/// legitimate, and is the only way to drop one once the memory holding it has been exported.
fn report_link_deltas(result: &mut Value, deltas: &[LinkDelta]) {
    if deltas.is_empty() {
        return;
    }
    let left_any = deltas.iter().any(|delta| !delta.left_behind.is_empty());
    result["link_delta"] = serde_json::to_value(deltas).unwrap_or(Value::Null);
    result["link_delta_note"] = json!(if left_any {
        "Stored. `left_behind` names links the superseded memory carries and this one does not \
         restate. A link is the claim of the memory that wrote it, so these stay with the \
         superseded memory rather than becoming this one's. That is a legitimate act -- it is how \
         a link that no longer holds is dropped -- and it is reported because it is also what an \
         incomplete re-typing looks like. If one still holds for this memory, correct it now with \
         hoard_correct's amend, which replaces the whole link array."
    } else {
        "Stored. Nothing was left behind: every link the superseded memory carried, other than \
         its own supersession history, is restated here."
    });
}

/// State the delta and say what to do about it. Never refuse: dropping a keyword is legitimate, and
/// is the only way to retire one.
fn report_keyword_deltas(result: &mut Value, deltas: &[KeywordDelta]) {
    if deltas.is_empty() {
        return;
    }
    let dropped_any = deltas.iter().any(|delta| !delta.dropped.is_empty());
    result["keyword_delta"] = serde_json::to_value(deltas).unwrap_or(Value::Null);
    result["keyword_delta_note"] = json!(if dropped_any {
        "Stored. `dropped` names keywords the superseded memory carried and this one does not, so \
         nothing answers to them here any more unless another memory happens to carry them. That \
         is a legitimate act -- it is the only way to retire a keyword, since amendment is refused \
         after export -- and it is reported because it is also what an incomplete re-typing looks \
         like. If a dropped term should still find this memory, correct it now with hoard_correct's \
         amend, which replaces the whole keyword array."
    } else {
        "Stored. Nothing was dropped: every keyword the superseded memory carried is carried here \
         too, so no term stopped reaching anything."
    });
}

fn report_unresolved(result: &mut Value, unresolved: &[UnresolvedLink]) {
    if unresolved.is_empty() {
        return;
    }
    result["unresolved_links"] = serde_json::to_value(unresolved).unwrap_or(Value::Null);
    result["unresolved_links_note"] = json!(
        "Stored. Some links could not be resolved here, which is reported rather than refused \
         because a link is apparatus and a refusal would have cost the memory itself. A target \
         may be absent because the id or the set name is wrong, or because this machine's copy of \
         that set predates the memory being pointed at. `set_not_loaded` means nothing was \
         checked. Fix a wrong one with hoard_correct's amend, which replaces the whole link array."
    );
}

/// Refuse a safety copy aimed at a path that already holds a file.
///
/// **Replaces a check against the set's recorded export targets, and is both simpler and stronger.**
/// The old rule asked *is this a path this set has been exported to*, which needed the per-target
/// base's target list to answer and could only ever see paths this store had written itself — it
/// was blind to a colleague's backup, to an unrelated file, and to a copy someone made by hand. The
/// harm being guarded against is destroying a file, not destroying a file of a particular
/// provenance, so *does something already live here* is the question the old rule was
/// approximating, and it needs no stored state at all.
///
/// It also refuses re-running a rescue onto a path used by an earlier one. That is correct rather
/// than incidental: the previous copy is precisely the thing you would work back from.
///
/// **Not race-free, and deliberately not made so.** The write is a temporary file plus a rename,
/// which is what makes it atomic; closing the window would mean giving that up. The caller names
/// this path in the same call, so the realistic failure is a path that already held something
/// rather than a concurrent writer.
fn occupied_safety_copy(path: &Path) -> Option<String> {
    path.exists().then(|| {
        format!(
            "{} already exists. The safety copy is written before anything is destroyed, so \
             overwriting it would lose whatever it holds in the act of making a backup -- and if it \
             is a copy of this set, that is the file you would work back from. Name a path that \
             does not exist yet.",
            path.display()
        )
    })
}

/// The refusal for a scope no loaded store answers to, naming the reason when it is withheld.
fn missing_scope(withheld: &BTreeMap<Scope, Store>, scope: &Scope) -> String {
    if withheld.contains_key(scope) {
        return withheld_scope(scope);
    }
    unknown_scope(scope)
}

fn withheld_scope(scope: &Scope) -> String {
    format!(
        "the memory set {} is not loaded, because another set on this machine answers to the same \
         set identity and nothing says which of the two is the original. Call `hoard_sets` with \
         action \"status\": both are listed under `withheld`. Drop the one you do not mean -- \
         `drop` writes a safety copy first -- and the other loads.",
        serde_json::to_string(scope).unwrap_or_else(|_| "?".to_owned())
    )
}

fn unknown_scope(scope: &Scope) -> String {
    format!(
        "no memory set is loaded for scope {}. Call `hoard_sets` with action \"status\" to see \
         which sets this machine holds, or action \"create\" to bring a new shared set into being.",
        serde_json::to_string(scope).unwrap_or_else(|_| "?".to_owned())
    )
}

fn parse<T: serde::de::DeserializeOwned>(arguments: &Value) -> Result<T, String> {
    serde_json::from_value(arguments.clone())
        .map_err(|error| format!("the arguments are not valid for this tool: {error}"))
}

fn tool_definitions(protocol: &str) -> Vec<Value> {
    let mut tools = tool_inputs(protocol);
    if !publishes_output_schemas(protocol) {
        return tools;
    }
    for tool in &mut tools {
        let name = tool["name"].as_str().unwrap_or_default().to_owned();
        if let Some(schema) = crate::output_schema::for_tool(&name) {
            tool["outputSchema"] = schema;
        }
    }
    tools
}

/// Whether a client on `protocol` is sent output schemas and `structuredContent`: only on the
/// revision that defines them.
fn publishes_output_schemas(protocol: &str) -> bool {
    protocol != "2024-11-05"
}

/// Each tool's name, description and `inputSchema`; [`tool_definitions`] adds what comes back.
fn tool_inputs(protocol: &str) -> Vec<Value> {
    vec![
        json!({
            "name": "hoard_conventions",
            "description": "Load Scatterhoard's operating conventions. Call this at the start of \
                            a session that uses Scatterhoard, before storing or recalling \
                            anything, and again after the conversation has been summarized or \
                            compacted. With no `max_chars` the whole catalog arrives at \
                            once. With one it arrives in pages, and each page's last line says \
                            either how to call for the rest or that every convention is loaded: \
                            keep calling until it says they are all loaded. Each page's first \
                            line is a slice of one JSON document; joined in order, the slices \
                            are the whole of it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cursor": {"type": "integer", "description": "Where the next page starts, as \
                        the previous page's last line gives it."},
                    "max_chars": {"type": "integer", "description": "The most characters a page \
                        may hold, everything on it counted. Omit it to receive the whole catalog \
                        in one page."},
                },
            },
        }),
        json!({
            "name": "hoard_memorize",
            "description": format!(
                "Store a memory. Write `&lt;` `&gt;` `&amp;` where you mean those \
                 characters -- they are decoded on the way in, so a memory *about* \
                 markup can be written without emitting any. A call whose parameter was \
                 closed with the wrong tag is refused rather than stored damaged. You \
                 must say which memory set it belongs to: \
                 Scatterhoard cannot work that out, and a personal memory committed to \
                 a shared repository cannot be taken back. Give every memory its \
                 reasoning in `why`. The response echoes `summary`: the one line this \
                 memory would show as in an index, cut from its opening and ending in \
                 an ellipsis if it was cut. Read it -- if it does not say what the \
                 memory is for, the opening is where to fix that, not the length. {}",
                link_label_sentence()
            ),
            "inputSchema": {
                "type": "object",
                "required": ["scope", "content", "confidence", "origin"],
                "properties": {
                    "scope": scope_schema(
                        "Which memory set this belongs to. Required, and never guessed.",
                    ),
                    "content": {"type": "string", "description": "The memory itself."},
                    "why": {"type": "string", "description": "The reasoning behind it."},
                    // The wording here is load-bearing, and was measured to be. The previous
                    // description -- "When and in response to what this came up. Not searched" --
                    // produced 98% adoption of an optional field and then returned its own
                    // vocabulary: dates in 89% of `scatterhoard` contexts, restating `created.at`,
                    // and checkability pointers in 72%, parked in the one field neither search nor
                    // any later session reads. What a human can use is the activity in progress,
                    // named as a person would say it. Human recall keys on the session's
                    // through-line rather than on dates or identifiers, so the exclusions matter as
                    // much as the instruction.
                    "context": {
                        "type": "string",
                        "description": "A handle to help a human place this memory later -- not \
                                        information about it. Write what was being worked on when \
                                        it came up, named the way a person would say it rather \
                                        than the way the repository does: \"Making the author \
                                        changeable. Documenting that `bg` is a memory ID.\", not \
                                        \"make-author-changeable step 4\". Include other work from \
                                        the same stretch even where it has nothing to do with this \
                                        memory -- a session runs as one thread, and naming any \
                                        part of it can pull the rest back. Leave out what the \
                                        record already carries: the date, the id, the author. \
                                        Where something can be checked goes in `content` or \
                                        `why`, which are searched; this is not.",
                    },
                    // Points at the convention rather than restating it: `keyword-standards` is
                    // served into the same session and is the authority on spelling and choice.
                    // What belongs here is the part a caller needs at the point of use -- what the
                    // field is *for*, which is not inferable from an array of strings.
                    "keywords": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Terms for exact-match search, and the only path that still \
                                        works with no embedding model loaded. See the \
                                        `keyword-standards` convention for how to choose and spell \
                                        them. Optional -- except alongside a `supersedes` link, \
                                        where the whole set the replacement is indexed under has \
                                        to be stated, because neither carrying the old terms over \
                                        nor leaving them empty is a safe default. Send `[]` if it \
                                        genuinely belongs under none.",
                    },
                    "confidence": {"type": "number", "description": "0.0 to 1.0."},
                    "origin": {
                        "type": "string",
                        "enum": Origin::ALL.iter().map(serde_name).collect::<Vec<_>>(),
                        "description": "Whose reasoning produced the content.",
                    },
                    "links": links_schema(),
                },
            },
        }),
        // **`by` is stated on both tools that hand back a memory, and the duplication is the
        // symptom rather than the fix.** A reader meets the field in a recall result and acts on it
        // through lookup, so both have to say what it is -- and there is nowhere else to say it,
        // because the response shape is not published anywhere a caller can read
        // (`response-shape-is-unpublished` in TRACKING.md). Worth stating at all because the change
        // that made `by` an id took away the field's self-description: `"by": "jtcours"` needed no
        // gloss, and `"by": "01M1G5HK..."` is inert without one -- which would quietly cost the very
        // property the id was chosen for, that resolving it is one cheap fetch.
        json!({
            "name": "hoard_recall",
            "description": "Search memory, one way per call: give a query to search by meaning, or \
                            keywords to match exactly -- not both. The two are different scales and \
                            cannot be ranked against each other, so call twice if you want both. \
                            Results come back in `memories`, ordered best first. A memory's \
                            `created.by` and `modified.by` hold the **id of a `hoard-author` \
                            memory** naming who wrote it, not the name itself -- read it with \
                            hoard_lookup on the rare occasion you need the name. Search before \
                            acting on anything an earlier decision may already govern, and prefer \
                            recalling over assuming. Superseded memories are replaced by whatever \
                            corrected them, and a replacement carries `matched_similarity` and \
                            `replaces_matched`, for the superseded memory your query matched, in \
                            place of `similarity`. A memory linked to one your query matched can come back \
                            too, carrying `inherited` to say so -- that is how the reasoning behind \
                            a decision reaches you when your wording only matched the decision. A \
                            memory marked `evaluation` came from a set loaded for review rather \
                            than adopted -- what someone proposes, not what this project has \
                            decided.",
            // Nothing is required, and `oneOf` is deliberately not used to express the exclusivity.
            // `scope` is optional by design -- the asymmetry with `hoard_memorize` is the decision,
            // not an oversight -- and either `query` or `keywords` alone is a whole search path,
            // keyword-only being the one that survives having no embedding model.
            //
            // The exclusivity is stated in the description and enforced by the tool. Expressing it
            // in the schema instead would let a client that validates arguments locally reject the
            // call without ever reaching the message that says which argument to drop, and a
            // caller that cannot see *why* it was refused tends to retry the same shape. The
            // refusal names both arguments and what to do about them.
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": scope_schema(
                        "Which memory set to search. Omit to search every loaded set, which is \
                         usually what you want -- a personal preference can matter while working \
                         on a project. Each result says which set it came from.",
                    ),
                    "query": {
                        "type": "string",
                        "description": "What you are looking for, in your own words. Searched by \
                                        meaning.",
                    },
                    "keywords": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Exact keywords to match. Cannot be combined with `query` in \
                                        one call, because a match count and a similarity score are \
                                        not the same scale. This is the path that works with no \
                                        embedding model installed, and the one to use for a precise \
                                        identifier -- an exact tag will not reliably be found by \
                                        meaning.",
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": format!(
                            "How many memories to return, best first; {DEFAULT_LIMIT} if you do not \
                             say. Honoured as given, with no ceiling: the size of the response is \
                             yours to manage, and `detail: summary` costs a small fraction of a \
                             full record per memory. A smaller limit returns the start of \
                             what a larger one would, so raising it only adds to the end -- and \
                             pass `already_known` to page past what you have rather than asking \
                             for it all again. Fewer come back when fewer clear the set's \
                             non-match floor."
                        ),
                    },
                    "already_known": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Ids of memories you already hold, exactly as an earlier \
                                        result gave them. They are left out of the results and out \
                                        of nothing else -- what you already hold still seeds the \
                                        search and is still traversed through, so this is how you \
                                        ask what surrounds what you know. Pass back the last \
                                        result's ids to get the next tranche instead of the same \
                                        memories again.",
                    },
                    // The vocabulary comes from the type, for the reason the link labels and the
                    // origins do: a hand-written copy is how a schema and its deserialiser fall out
                    // of step, and a level the server accepts but never mentions is unreachable.
                    "detail": {
                        "type": "string",
                        "enum": Detail::ALL.iter().map(serde_name).collect::<Vec<_>>(),
                        "description": "How much of each memory to return. `full`, the default, is \
                                        the whole record. `summary` is an id and one line derived \
                                        from the memory's opening, about a fifth of the size -- so \
                                        it is how you survey: ask a broad question at a large \
                                        `limit`, read thirty lines for the cost of six memories, \
                                        then fetch the ones worth having with hoard_lookup. A line \
                                        ending in an ellipsis was cut, and one without is the whole \
                                        memory. A line can only be cut short, never made to say the \
                                        opposite, so judge it as a name for a memory rather than as \
                                        its claim.",
                    },
                    "ignore_floor": {
                        "type": "boolean",
                        "description": "Search below the set's non-match floor. A set that has \
                                        answered enough questions calibrates a level from what it \
                                        has been able to answer, and returns nothing at all below \
                                        it -- which is how *nothing here answers that* is said, \
                                        where every other part of the ranking is relative and can \
                                        only say *this was the best of them*. When that happens you \
                                        are told so; pass this to ask again regardless, which is \
                                        worth doing when you have reason to think the set should \
                                        know something even though its own history says otherwise.",
                    },
                },
            },
        }),
        json!({
            "name": "hoard_lookup",
            "description": "Fetch a memory by its whole id, or search for one by a `fragment` of \
                            an id when a whole one is not to hand. Use this when you \
                            already have an id -- from a link, from a memory's `created.by`, or \
                            from a recall result that replaced something. Unlike hoard_recall, a superseded memory is \
                            returned rather than swapped for its replacement, so this is how you \
                            read history. `superseded_by` names the end of the chain; to walk it one \
                            link at a time, follow the `supersedes` edge in `incoming`. A memory \
                            marked `supersession_cycle` has a broken history -- two memories each \
                            claiming to replace the other -- which no tool can resolve for you and \
                            which usually means a merge went wrong.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "A whole memory id: 26 characters. A shortened one is \
                                        refused rather than guessed at -- use `fragment` for that.",
                    },
                    "fragment": {
                        "type": "string",
                        "description": "Part of an id, when a whole one is not to hand. Prefer a \
                                        piece from the *end*: the leading characters are a \
                                        timestamp, so memories written close together share them \
                                        and a fragment from the start matches all of them.",
                    },
                },
            },
        }),
        json!({
            "name": "hoard_correct",
            "description": format!(
                "Repair a memory that has never left this machine, or remove one \
                 entirely. Amend fixes anything about a memory nobody has seen yet -- a \
                 mistyped keyword, a link pointing at the wrong id, a confidence that \
                 was off, damaged text, the claim itself. Once a memory has been \
                 exported anywhere, including a backup, amending it is refused and the \
                 server says so: a second copy exists, so the change has to leave a \
                 trail, and you make it by storing the corrected memory with a \
                 `supersedes` link -- which keeps the old version readable as a record \
                 of what was believed. You do not have to work out which case you are \
                 in. Amending `content` echoes `summary`, the line the memory now shows \
                 as in an index -- so getting an opening to say what a memory is for is \
                 amend, read the line, amend again. Write `&lt;` `&gt;` `&amp;` where \
                 you mean those characters; a call whose parameter was closed with the \
                 wrong tag is refused rather than applied. {}",
                link_label_sentence()
            ),
            "inputSchema": correct_schema(),
        }),
        json!({
            "name": "hoard_sets",
            "description": "Manage memory sets themselves rather than their contents: find out \
                            which sets this machine holds, bring a new shared set into existence, \
                            or write one out to a file to be committed or kept as a backup. \
                            Setup-time and commit-time work -- storing and recalling do not need \
                            it.",
            "inputSchema": sets_schema(protocol),
        }),
    ]
}

/// One loaded set, as much of it as saying where a link lands requires.
struct LoadedSet {
    scope: Scope,
    identity: Option<SetId>,
    mode: &'static str,
}

/// What a memory under review points at in other sets, and where that lands on this machine.
///
/// **Reported, never resolved** -- `documents/design/load-modes.md`. A review copy carries its
/// source set's identity, so a link naming another set has two or more candidates here: the
/// reviewer's own copy of that set, and any other review copy of it. Choosing one would attach a
/// contributor's link to whichever memory this machine happens to hold, so instead the link is
/// handed over whole -- target, set name, identity -- with every loaded scope answering to that
/// identity and how it is held, for the caller to follow with `hoard_lookup` if it bears on the
/// review.
fn cross_set_targets(memory: &Memory, loaded: &[LoadedSet]) -> Vec<Value> {
    let mut surfaced = Vec::new();
    for link in &memory.links {
        let LinkTarget::CrossSet {
            set, id, set_id, ..
        } = &link.target
        else {
            continue;
        };
        let answering: Vec<Value> = loaded
            .iter()
            .filter(|held| match set_id {
                // By identity where the link carries one, which is the rule `holder_of` follows: a
                // name can be stale, and a review copy's name never matches the link at all.
                Some(wanted) => held.identity == Some(*wanted),
                None => held.scope.exports_to() == Some(set),
            })
            .map(|held| json!({"scope": WireScope(&held.scope), "mode": held.mode}))
            .collect();
        surfaced.push(json!({
            "label": link.label,
            "set": set.as_str(),
            "id": id.to_string(),
            "set_id": set_id.map(|id| id.to_string()),
            "loaded": answering,
        }));
    }
    surfaced
}

/// Put a memory's surfaced cross-set links on its view, for a memory under review.
///
/// Only there: an ordinary set resolves its links, so listing candidates for it would be noise. Only
/// on a full view, since a summary carries neither links nor room for them.
fn surface_cross_set_targets(
    view: &mut Value,
    reviewing: bool,
    memory: &Memory,
    loaded: &[LoadedSet],
) {
    if !reviewing || view.get("links").is_none() {
        return;
    }
    let targets = cross_set_targets(memory, loaded);
    if !targets.is_empty() {
        view["cross_set_targets"] = json!(targets);
    }
}

/// The line a response carries when it returned anything from a copy loaded for review.
///
/// **Only when one is present**, so an ordinary recall pays nothing for it, and it arrives at the
/// moment it bears on what the reader does next -- which is the argument for a note here rather than
/// a clause in the convention catalog. `documents/design/load-modes.md`.
const UNDER_REVIEW_NOTE: &str = "Memories marked `evaluation` come from a set loaded for review \
     rather than adopted: they are what someone proposes, not what this project has decided. Weigh \
     them as a reviewer would, and do not carry them into your own sets as settled.";

/// Attach [`UNDER_REVIEW_NOTE`] where any of `memories` carries the mark.
fn note_any_under_review(result: &mut Value, memories: &[Value]) {
    if memories
        .iter()
        .any(|memory| memory.get("evaluation") == Some(&json!(true)))
    {
        result["under_review_note"] = json!(UNDER_REVIEW_NOTE);
    }
}

/// Give an action-dispatched schema a flat top-level view, derived from its own branches.
///
/// **A `oneOf` alone is legal and was not enough.** Consumers vary in whether they honour it, and on
/// 2026-08-11 one presented `hoard_sets` as taking no arguments at all — stripping every one before
/// it reached the server, so the entire management surface answered *"needs an action"* and nothing
/// could call it. A second consumer merged the branches but kept the first branch's `const`, leaving
/// a schema that could only express `status`. Both read the same publication; neither was wrong to.
/// `oneof-tool-schemas-present-no-parameters` in `TRACKING.md`.
///
/// So the branches stay — they are the precise statement, and a consumer that honours them still
/// gets it — and this adds the fallback beneath: every argument at the top level, `action` as an
/// `enum` over all of them, and the per-action combinations spelled out in prose that survives any
/// flattening.
///
/// **Derived rather than written twice.** The flat view is built from the branches on every call, so
/// an action added below appears above without anyone remembering to, which is the failure this
/// codebase files more of than any other. The precision the branches buy is only *communicative*
/// anyway — neither shape forbids a stray `name` on a `status` call, since JSON Schema permits
/// unlisted properties unless told otherwise — so restating the combinations in words costs the
/// caller nothing they had.
fn with_flat_view(mut schema: Value) -> Value {
    let variants = schema["oneOf"].as_array().cloned().unwrap_or_default();

    let mut properties = serde_json::Map::new();
    let mut actions: Vec<Value> = Vec::new();
    let mut combinations: Vec<String> = Vec::new();

    for variant in &variants {
        let Some(action) = variant["properties"]["action"]["const"].as_str() else {
            continue;
        };
        actions.push(json!(action));

        let takes = branch_arguments(variant);
        let sentence = takes.sentence();
        combinations.push(if sentence.is_empty() {
            format!("`{action}` takes nothing else")
        } else {
            format!("`{action}` takes {sentence}")
        });

        if let Some(branch) = variant["properties"].as_object() {
            for (name, definition) in branch {
                if name == "action" {
                    continue;
                }
                // First branch wins. A property shared by two actions is the same argument with the
                // same meaning -- `scope` throughout -- and a later branch's wording would only
                // narrow it to that action.
                properties
                    .entry(name.clone())
                    .or_insert_with(|| definition.clone());
            }
        }
    }

    properties.insert(
        "action".to_owned(),
        json!({
            "type": "string",
            "enum": actions,
            "description": format!(
                "Which operation to perform. Each action takes only its own arguments: {}.",
                combinations.join("; ")
            ),
        }),
    );

    schema["properties"] = Value::Object(properties);
    schema["required"] = json!(["action"]);

    // **And the branches do not ship.** They were kept here for one afternoon as a courtesy to
    // consumers that honour them, which turned out to be a schema the Anthropic API refuses
    // outright: `input_schema does not support oneOf, allOf, or anyOf at the top level`. Publishing
    // one fails the *entire* tool list on every request -- every tool from every registered server,
    // for the rest of the session -- so this is not a cost paid by callers who ignore it.
    // `oneof-tool-schemas-present-no-parameters`, second half.
    //
    // Keeping `properties` alongside does not rescue it: the reported case published `anyOf` and
    // `properties` together and was still refused. The restriction is on the keyword's presence.
    //
    // Nested alternation is untouched and deliberately so -- `scope` and `links.target` keep theirs,
    // and the restriction is explicitly *at the top level*.
    if let Some(object) = schema.as_object_mut() {
        object.remove("oneOf");
    }
    schema
}

/// `a`, `a and b`, `a, b and c` — so a generated sentence reads as one.
///
/// Generic over anything string-like so a caller can pass borrowed names straight out of the schema
/// or phrases it built, without a second copy of this for owned strings.
fn and_list<S: AsRef<str>>(items: &[S]) -> String {
    match items {
        [] => String::new(),
        [only] => only.as_ref().to_owned(),
        [rest @ .., last] => format!(
            "{} and {}",
            rest.iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", "),
            last.as_ref()
        ),
    }
}

/// `either a or b` — the alternation's half of the same sentence.
///
/// Separate from [`and_list`] because the two say opposite things and a caller reading *scope and
/// directory and file* would supply all three, which is itself refused.
fn either_list<S: AsRef<str>>(items: &[S]) -> String {
    match items {
        [] => String::new(),
        [only] => only.as_ref().to_owned(),
        [rest @ .., last] => format!(
            "either {} or {}",
            rest.iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", "),
            last.as_ref()
        ),
    }
}

/// What one branch's action takes besides `action` itself.
///
/// **One derivation behind two things**: the flat schema's `action` description, and the argument
/// refusal below. An argument added to a branch reaches the published prose and the refusal without
/// anyone remembering to update either, which is the drift this codebase files more of than any
/// other kind.
///
/// Two kinds, because `export` takes a destination in one of two spellings and a flat list of names
/// cannot say *one of these*. Held together in one value rather than read separately at each site,
/// so a branch that grows an alternation cannot reach a site that still only looks at `required`.
#[derive(Debug, Default)]
struct BranchArguments<'a> {
    /// Every one of these has to arrive.
    required: Vec<&'a str>,
    /// Exactly one of these has to arrive. Empty for an action with no alternation.
    alternatives: Vec<&'a str>,
    /// May arrive or not. Named in the prose, or a consumer that flattens the schema never learns
    /// which action takes them: `amend`'s whole set of replaceable fields was invisible that way
    /// until 2026-09-16, found when `create` gained `mode`.
    optional: Vec<&'a str>,
}

impl BranchArguments<'_> {
    /// The whole of what the action takes, as one phrase: `scope and either directory or file`.
    fn sentence(&self) -> String {
        let mut parts: Vec<String> = self
            .required
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        if !self.alternatives.is_empty() {
            parts.push(either_list(&self.alternatives));
        }
        let taken = and_list(&parts);
        if self.optional.is_empty() {
            return taken;
        }
        let optional: Vec<String> = self
            .optional
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        if taken.is_empty() {
            format!("optionally {}", and_list(&optional))
        } else {
            format!("{taken}, and optionally {}", and_list(&optional))
        }
    }

    /// What this call did not bring. An unmet alternation names all its spellings, since any one of
    /// them settles it and naming only the first would teach the caller half the surface.
    fn absent(&self, arguments: &Value) -> Vec<String> {
        let mut absent: Vec<String> = self
            .required
            .iter()
            .filter(|name| !supplied(arguments, name))
            .map(|name| (*name).to_owned())
            .collect();
        if !self.alternatives.is_empty() && self.brought(arguments).is_empty() {
            absent.push(either_list(&self.alternatives));
        }
        absent
    }

    /// The spellings of an alternation this call brought more than one of.
    ///
    /// Refused rather than resolved by precedence. A precedence rule is invisible at the call site,
    /// so it would silently ignore half of what the caller asked for — and for a destination that
    /// means writing a set somewhere the caller did not mean it to go.
    fn conflicting(&self, arguments: &Value) -> Vec<&str> {
        let brought = self.brought(arguments);
        if brought.len() > 1 {
            brought
        } else {
            Vec::new()
        }
    }

    /// Which of the alternation's spellings this call actually carried.
    ///
    /// One derivation behind both questions above: *none of them* is what makes the call incomplete
    /// and *more than one* is what makes it ambiguous, and reading the arguments twice by two
    /// slightly different tests is how those two answers would come to disagree.
    fn brought(&self, arguments: &Value) -> Vec<&str> {
        self.alternatives
            .iter()
            .copied()
            .filter(|name| supplied(arguments, name))
            .collect()
    }
}

/// Whether an argument arrived at all.
///
/// `null` counts as absent: a client that fills every property of a flattened schema sends the ones
/// its action does not use as `null`, and treating that as *supplied* would refuse the call for
/// naming two destinations when it named one.
fn supplied(arguments: &Value, name: &str) -> bool {
    !matches!(arguments.get(name), None | Some(Value::Null))
}

fn branch_arguments(variant: &Value) -> BranchArguments<'_> {
    let names = |key: &str| -> Vec<&str> {
        variant[key]
            .as_array()
            .map(|listed| {
                listed
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|name| *name != "action")
                    .collect()
            })
            .unwrap_or_default()
    };
    let required = names("required");
    let alternatives = names("one_of");
    let optional = variant["properties"]
        .as_object()
        .map(|properties| {
            properties
                .keys()
                .map(String::as_str)
                .filter(|name| {
                    *name != "action" && !required.contains(name) && !alternatives.contains(name)
                })
                .collect()
        })
        .unwrap_or_default();
    BranchArguments {
        required,
        alternatives,
        optional,
    }
}

/// The branch pinning `action`, if these branches define it.
///
/// `None` for an action no branch declares, which leaves the unknown-action refusal to say so --
/// naming the actions that exist is a better answer than naming arguments for one that does not.
fn branch_for<'a>(branches: &'a Value, action: &str) -> Option<&'a Value> {
    branches["oneOf"]
        .as_array()?
        .iter()
        .find(|variant| variant["properties"]["action"]["const"].as_str() == Some(action))
}

/// Refuse a call whose arguments do not match what its action takes, in this project's voice rather
/// than serde's.
///
/// Two failures, one gate. An argument that did not arrive, and — where a branch declares an
/// alternation — two spellings of one argument that both did. Answered here together because both
/// are *this call's arguments do not describe an operation*, and a second gate beside this one is a
/// step every future dispatcher would have to remember.
///
/// `a-missing-argument-refusal-is-a-raw-serde-message`. The deserialiser's ``missing field `name` ``
/// is honest and actionable, which is why this never rated worse than `Costs`. What it was not is
/// the voice every other refusal here speaks in -- naming the action and what it takes.
///
/// **Checked ahead of the deserialiser rather than translated after it**, for two reasons. Reading
/// serde's text back would make a dependency's wording into an input to ours, and serde stops at
/// the *first* missing field where the branch knows all of them -- so this is what makes "one round
/// trip fixes it" true for a call that dropped two.
///
/// The flat schema is what makes this path common enough to be worth the code: at the top level
/// every argument now looks optional, and only the `action` description says which action needs
/// which, so a caller reading the published shape is *more* likely to arrive here than before.
fn argument_refusal(branches: &Value, action: &str, arguments: &Value) -> Option<String> {
    let takes = branch_arguments(branch_for(branches, action)?);

    let conflicting = takes.conflicting(arguments);
    if !conflicting.is_empty() {
        return Some(format!(
            "`{action}` takes {}, and this call included {}. They name different places to write, \
             so there is nothing here to choose between them.",
            takes.sentence(),
            and_list(&conflicting)
        ));
    }

    let absent = takes.absent(arguments);
    if absent.is_empty() {
        return None;
    }
    Some(format!(
        "`{action}` takes {}. This call did not include {}.",
        takes.sentence(),
        and_list(&absent)
    ))
}

/// The `hoard_sets` argument schema.
///
/// One `oneOf` branch per action, each pinning its own `action` value and listing only the arguments
/// that action takes. A flat schema with an `action` enum plus every optional argument would let a
/// caller construct `{"action": "status", "name": …}` and believe it meant something —
/// `schema-underspecifies-scope-and-links` was that defect, and the branches are what avoid it.
/// [`with_flat_view`] derives the published schema from them and then drops them, because the API
/// refuses a top-level `oneOf`. They remain the single source for the flat view, for the action
/// enum, and for the actions a refusal names — they are simply not on the wire.
fn sets_schema(protocol: &str) -> Value {
    with_flat_view(sets_branches_for(protocol))
}

/// [`sets_branches`] as served on `protocol`.
///
/// **On [`MODERN_PROTOCOL`] the human's own answers are optional arguments**: `mint`'s and
/// `rename`'s `author`, and `identify`'s `record`. A client there that shows forms has the human
/// asked directly, and the argument is set aside; one that does not still needs it from the model,
/// and whether a client shows forms is declared per request while the tool list is the same for
/// every caller. So the argument stays, optional, saying when it is used. On the handshake
/// revisions nothing can ask, and the arguments are required as before.
fn sets_branches_for(protocol: &str) -> Value {
    let mut branches = sets_branches();
    if protocol != MODERN_PROTOCOL {
        return branches;
    }
    let Some(variants) = branches["oneOf"].as_array_mut() else {
        return branches;
    };
    for variant in variants {
        let argument = match variant["properties"]["action"]["const"].as_str() {
            Some("mint" | "rename") => "author",
            Some("identify") => "record",
            _ => continue,
        };
        if let Some(required) = variant["required"].as_array_mut() {
            required.retain(|name| name != argument);
        }
        let described = variant["properties"][argument]["description"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        variant["properties"][argument]["description"] = json!(format!(
            "Leave this out where the human's client can show the server's own questions -- most \
             can -- and the human is asked directly, which nothing written here overrides. Give it \
             only where a refusal says this connection cannot ask. {described}"
        ));
    }
    branches
}

/// The per-action branches `hoard_sets` is defined by. Internal: see [`sets_schema`].
fn sets_branches() -> Value {
    let mut branches = every_other_branch();
    if let Some(variants) = branches["oneOf"].as_array_mut() {
        variants.extend(review_branches());
    }
    branches
}

/// The export review's two actions, added to [`sets_branches`] separately because one `json!` holding
/// every branch reaches the macro's recursion limit.
fn review_branches() -> Vec<Value> {
    let answer = json!({
        "type": "object",
        "required": ["n", "about", "mentions"],
        "properties": {
            "n": {"type": "integer"},
            "about": {"type": "string", "enum": ["project", "the person", "someone else"]},
            "says": {
                "type": "string",
                "enum": ["reports", "judges"],
                "description": "Only for a sentence about the person or someone else.",
            },
            "mentions": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Items from the sharing preferences the sentence mentions, quoted as \
                                listed -- not the sentence's own words.",
            },
        },
    });
    vec![
        json!({
            "title": "backup",
            "description": "Write a safety copy of the set where this machine keeps backups. Never \
                            refused for want of a review, and never a publication.",
            "required": ["action", "scope"],
            "properties": {
                "action": {"const": "backup"},
                "scope": scope_schema("Which set to back up."),
            },
        }),
        json!({
            "title": "review",
            "description": "Show the memories nobody has reviewed, which export refuses, with what \
                            the review asks of you. Answer them with `clear`.",
            "required": ["action", "scope"],
            "properties": {
                "action": {"const": "review"},
                "scope": scope_schema("Which set to review."),
                "directory": {
                    "type": "string",
                    "description": "For a shared set, the checkout it is exported to.",
                },
                "file": {
                    "type": "string",
                    "description": "For a shared set exported to one exact file, that file.",
                },
                "full": {
                    "type": "boolean",
                    "description": "Review every memory again, as before a first publication.",
                },
                "recheck": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "After a review reports stricter preferences: the groups the \
                                    change could apply to, or an empty list.",
                },
            },
        }),
        json!({
            "title": "clear",
            "description": "Answer what a `review` in this session showed, one answer per numbered \
                            sentence. Pass `person_cleared` for flagged memories the person says are \
                            fine.",
            "required": ["action", "scope"],
            "properties": {
                "action": {"const": "clear"},
                "scope": scope_schema("The set the review was for."),
                "answers": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["id", "sentences"],
                        "properties": {
                            "id": {"type": "string"},
                            "sentences": {"type": "array", "items": answer},
                        },
                    },
                },
                "person_cleared": {"type": "array", "items": {"type": "string"}},
            },
        }),
    ]
}

/// Every `hoard_sets` branch but the review's.
fn every_other_branch() -> Value {
    json!({
        "type": "object",
        "oneOf": [
            {
                "title": "status",
                "description": "List the sets this machine holds and how many memories each has.",
                "required": ["action"],
                "properties": {"action": {"const": "status"}},
            },
            {
                "title": "create",
                "description": "Create a new shared set, empty, and load it. The local set always \
                                exists and cannot be created. **Then give it a `hoard-scope-topics` \
                                memory saying what it is for**: what the set is about, and what \
                                sends a memory elsewhere, with a `hoard-lang-` keyword for each \
                                language its memories will be written in, such as `hoard-lang-en`. \
                                Lead with the subject. A list of the kinds \
                                of record it takes may illustrate that, but must never stand in its \
                                place -- routing matches a candidate against the description, so a \
                                list that leads decides what gets *written* and not only where it \
                                goes, and whatever it omits stops being recorded at all. You are \
                                writing it before the set holds anything, so treat it as a first \
                                draft: once there is enough in the set to sort, read what \
                                accumulated and correct the charter to match.",
                "required": ["action", "name"],
                "properties": {
                    "action": {"const": "create"},
                    "name": {
                        "type": "string",
                        "description": "The set's shared identity. It travels in the exported \
                                        file's header, so every copy of the set agrees on it. \
                                        Where the set is exported to is stated at export time, \
                                        not here.",
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["read-write", "read-only"],
                        "description": "How this machine holds the set, fixed at creation. Omit it \
                                        for a set of your own, which is read-write. Choose \
                                        `read-only` when taking on reference material someone \
                                        else publishes -- a product's documentation, a policy \
                                        set -- and then import their file: a read-only set is \
                                        refreshed from that file and never written here.",
                    },
                },
            },
            {
                "title": "reembed",
                "description": "Rebuild a set's vectors with this server's embedding model. Use it \
                                when recall reports that the set's vectors came from a different \
                                model: that is the only way back to search by meaning, because \
                                reading no longer rebuilds them. Every memory is re-embedded, which \
                                takes well under a second. If another server is running with its \
                                own model, this will be undone again -- the response says so, and \
                                the fix is to make the two configurations agree.",
                "required": ["action", "scope"],
                "properties": {
                    "action": {"const": "reembed"},
                    "scope": scope_schema(
                        "Which set to rebuild. One at a time: you are answering a question about \
                         one store, and the others have not been asked about.",
                    ),
                },
            },
            {
                "title": "calibrate",
                "description": "Set this set's **non-match floor** from its own query log: the \
                                level below which recall returns nothing at all, which is how \
                                *nothing here answers that* gets said. Every other part of the \
                                ranking is relative -- a gap, a rank -- so without it an \
                                unanswerable question comes back with a full window of \
                                confident-looking results. The level is read off the questions this \
                                set has been seen to answer, so it belongs to this set alone and no \
                                constant would do; it needs a few dozen such questions before one \
                                can be placed, and until then nothing is set and recall is \
                                unchanged. Run it when a set has been in use for a while, and again \
                                after a long stretch of work; it is also the only way to *raise* a \
                                floor, since one that silences a question naming this set gives way \
                                on its own.",
                "required": ["action", "scope"],
                "properties": {
                    "action": {"const": "calibrate"},
                    "scope": scope_schema(
                        "Which set to calibrate. One at a time: a floor is read off one set's own \
                         history and means nothing for another.",
                    ),
                },
            },
            {
                "title": "uncalibrate",
                "description": "Remove this set's non-match floor, so recall goes back to returning \
                                whatever it ranks however weakly it matched. Use it when the floor \
                                is silencing questions the set should be answering: it gives way on \
                                its own only to a query that *named this set*, and only at the next \
                                calibration, so a set searched without a scope has no other way \
                                back. Nothing recorded is lost -- the queries this set has answered \
                                are still there, and calibrating again sets a level from them.",
                "required": ["action", "scope"],
                "properties": {
                    "action": {"const": "uncalibrate"},
                    "scope": scope_schema("Which set to clear the floor from."),
                },
            },
            {
                "title": "mint",
                "description": "Say that you are **new to this set**, minting an identity record \
                                for the name you publish under here. Everything you write in this \
                                set afterwards refers to that record rather than repeating the \
                                name, so changing it later supersedes one record instead of \
                                rewriting every memory. Refused where this machine already writes \
                                to the set as somebody, or where a record already carries the name \
                                -- in both cases one of the other two actions is what was meant, \
                                and minting anyway would make a second identity for one person, \
                                which nothing afterwards can tell from two contributors.",
                "required": ["action", "scope", "author"],
                "properties": {
                    "action": {"const": "mint"},
                    "scope": scope_schema("Which set you are new to."),
                    "author": {
                        "type": "string",
                        "description": "The name to publish under in this set. Ask the human \
                                        rather than guessing: it is how they are known where this \
                                        set is published, which varies by set -- an account name \
                                        on one host, a full name and title somewhere academic -- \
                                        and is a fact about them rather than about the machine.",
                    },
                },
            },
            {
                "title": "identify",
                "description": "Say **which existing identity record is yours** in a set you have \
                                taken on -- after importing a set you already contribute to, most \
                                often. Changes nothing in the set: which record is you is a fact \
                                about this machine, remembered here and travelling nowhere. Takes \
                                the record's id rather than a name, because a name can match \
                                several records or none, and choosing between them is exactly the \
                                guess that would fork one person into two identities. Recall the \
                                keyword `hoard-author` in that set to see the records and their \
                                ids.",
                "required": ["action", "scope", "record"],
                "properties": {
                    "action": {"const": "identify"},
                    "scope": scope_schema("Which set to say who you are in."),
                    "record": {
                        "type": "string",
                        "description": "The id of the `hoard-author` record that is you in this \
                                        set. Refused if the set does not hold it, if it is an \
                                        ordinary memory, or if it has been superseded -- a \
                                        superseded record is a name somebody used to publish \
                                        under.",
                    },
                },
            },
            {
                "title": "rename",
                "description": "Change the name **you** publish under in a set. Supersedes your \
                                identity record with one carrying the new name and forgets the old \
                                one, so the name it held is gone from the set rather than merely \
                                retired -- a tombstone remains, saying an identity changed without \
                                saying from what. No memory is rewritten: they point at the \
                                superseded record, which leads to the new one, so a set of any size \
                                is re-attributed by this one change. Export afterwards to carry it \
                                to everyone else holding the set. Reach for this when `status` or \
                                an export shows a set attributed to a name the person is not known \
                                by there.",
                "required": ["action", "scope", "author"],
                "properties": {
                    "action": {"const": "rename"},
                    "scope": scope_schema("Which set to change your name in."),
                    "author": {
                        "type": "string",
                        "description": "The name to publish under here from now on. This replaces \
                                        your own identity in this set and nobody else's; it cannot \
                                        reach another contributor's record.",
                    },
                },
            },
            {
                "title": "export",
                "description": "Write a set to a file so it can be committed, shared, or kept as a \
                                backup. Any scope may be exported; for `local` this is a backup. \
                                Name a `directory` to write into a checkout, or a `file` to write \
                                exactly one path -- one of the two, never both.",
                "required": ["action", "scope"],
                "one_of": ["directory", "file"],
                "properties": {
                    "action": {"const": "export"},
                    "scope": scope_schema(
                        "Which set to write out. Any scope may be exported; for `local` this is a \
                         backup rather than a publication.",
                    ),
                    "directory": {
                        "type": "string",
                        "description": "The directory the memory file sits in -- for a project, \
                                        the root of the checkout. The headers in its \
                                        `scatterhoard_mem` are searched for the set's file: by \
                                        identity for a set this machine holds, by `set` for one it \
                                        is reading. An export with no file there writes one named \
                                        after the set. State it every time: one set can legitimately have several checkouts, and \
                                        only you know which one this session concerns.",
                    },
                    "file": {
                        "type": "string",
                        "description": "The whole path of the memory file, used exactly as given \
                                        with nothing appended -- written there by `export`, read \
                                        from there by `import`, `restore`, `absorb` and \
                                        `evaluate`. For a file outside any checkout: a backup, a \
                                        set kept in a synced folder, or a file you were handed to \
                                        read. Give an export and its import the same \
                                        path -- it is what the two sides record \
                                        their agreement against, so a set exported to one path and \
                                        imported from another reconciles against nothing. The \
                                        directory has to exist already and may not be a \
                                        `scatterhoard_mem`, which `directory` already reaches. Use \
                                        `directory` for a checkout.",
                    },
                },
            },
            {
                "title": "evaluate",
                "description": "Read someone else's memory file for review, without adopting it. \
                                The copy is loaded under a name of its own -- `eval-<set>-<label>` \
                                -- searched only when a recall names that scope, and every memory \
                                it returns is marked as coming from a copy under review rather than \
                                from this project's record. It lives in memory: dropping it, or \
                                restarting, ends the review, and the set it copies is untouched \
                                throughout. Use it for a contributor's branch, or any file you want \
                                to read before deciding whether to take it on -- importing is \
                                adopting, and cannot be undone by checking the branch back out. \
                                Several reviews can be open at once, told apart by their labels. \
                                Name a `directory` and the `set` to read from a checkout, or a \
                                `file` to read one exact path.",
                "required": ["action", "label"],
                "one_of": ["directory", "file"],
                "properties": {
                    "action": {"const": "evaluate"},
                    "set": {
                        "type": "string",
                        "description": "The set to read, as its file's header names it. Needed \
                                        with `directory`, whose `scatterhoard_mem` can hold \
                                        several sets: the headers are searched for this name. \
                                        With `file`, optional, and the header must then agree.",
                    },
                    "label": {
                        "type": "string",
                        "description": "Which copy this is, in the user's words -- a branch, an \
                                        issue number, a version. It becomes part of the name every \
                                        memory from this copy is reported under, so it is how the \
                                        user tells two open reviews apart. Ask them for it rather \
                                        than inventing one; the software makes whatever they say \
                                        into a legal name.",
                    },
                },
            },
            {
                "title": "import",
                "description": "Reconcile a memory file into a loaded set after pulling. Local \
                                memories the file has never carried are kept, and a memory both \
                                sides changed is reported rather than overwritten. Safe to run \
                                before or after exporting -- the order does not matter. Name a \
                                `directory` to read from a checkout, or a `file` to read one exact \
                                path -- one of the two, never both, and the same one the matching \
                                export wrote. \
                                \
                                To adopt a set for the first time -- a repository you have just \
                                cloned -- use action \"create\" with the set's name and then import. \
                                Creating it is what binds that name on this machine; the file's own \
                                header is then checked against it and can only agree, so naming the \
                                wrong set is refused rather than silently importing someone else's \
                                memories.",
                "required": ["action", "scope"],
                "one_of": ["directory", "file"],
                "properties": {
                    "action": {"const": "import"},
                    "scope": scope_schema(
                        "Which loaded set the file belongs to. The file's own header is checked \
                         against this and can only agree.",
                    ),
                    "directory": {
                        "type": "string",
                        "description": "The directory to read from -- for a project, the root of \
                                        the checkout you pulled into. The set's file in its \
                                        `scatterhoard_mem` is found by the identity in its header.",
                    },
                },
            },
            {
                "title": "restore",
                "description": "Replace a set's contents with a file's, discarding whatever the \
                                store holds that the file does not. This is not `import`: import \
                                reconciles and keeps local work, restore replaces and throws it \
                                away on purpose. Use it when the store is damaged or you want the \
                                file's version outright; use `import` after an ordinary pull. The \
                                set is written to `save_to` first and always, so what is discarded \
                                can be diffed back in.",
                "required": ["action", "scope", "save_to"],
                "one_of": ["directory", "file"],
                "properties": {
                    "action": {"const": "restore"},
                    "scope": scope_schema(
                        "Which loaded set to replace. The file's own header is checked against this \
                         and can only agree. `local` may be restored like any other set -- for a \
                         set that never reaches a repository, a file is the only copy there is.",
                    ),
                    "directory": {
                        "type": "string",
                        "description": "The directory to read from -- for a project, the root of \
                                        the checkout. The set's file in its `scatterhoard_mem` is \
                                        found by the identity in its header.",
                    },
                    "file": {
                        "type": "string",
                        "description": "The whole path of the memory file to read, used exactly as \
                                        given.",
                    },
                    "save_to": save_to_schema(),
                },
            },
            {
                "title": "absorb",
                "description": "Merge another set's file into this one. Read a file that declares a \
                                *different* set and take its memories into `scope`, keeping the ids \
                                they were written with. Links from this set into the absorbed one \
                                become internal by an authored supersession: each such memory stays \
                                where it is and a new one replaces it saying the same thing, so the \
                                merge travels in the exported file and nobody downstream has to be \
                                told about it. The arriving memories are written exactly as their \
                                author wrote them. An id both sides hold is refused before anything \
                                is written, and this set is written to `save_to` first and always. \
                                Absorbing the same file twice does nothing the second time. This \
                                does not remove the absorbed set from this machine -- use \"drop\" \
                                for that, once you are satisfied with what arrived. Into `local` it \
                                takes a `local` file of another identity, which is how a personal \
                                store split across two directories is merged back; a `local` file \
                                is never absorbed into a shared set.",
                "required": ["action", "scope", "save_to"],
                "one_of": ["directory", "file"],
                "properties": {
                    "action": {"const": "absorb"},
                    "scope": scope_schema(
                        "The set doing the absorbing -- the one the memories are taken *into*. The \
                         file must declare some other set; a file declaring this one is `import` or \
                         `restore` instead.",
                    ),
                    "directory": {
                        "type": "string",
                        "description": "The directory to read the absorbed set's file from -- for a \
                                        project, the root of its checkout. Name the set with `set`.",
                    },
                    "set": {
                        "type": "string",
                        "description": "The set to read, as its file's header names it. Needed \
                                        with `directory`, whose `scatterhoard_mem` can hold \
                                        several sets: the headers are searched for this name. \
                                        With `file`, optional, and the header must then agree.",
                    },
                    "file": {
                        "type": "string",
                        "description": "The whole path of the absorbed set's memory file, used \
                                        exactly as given.",
                    },
                    "save_to": save_to_schema(),
                },
            },
            {
                "title": "drop",
                "description": "Remove a shared set from this machine: write it to a file you name, \
                                then destroy its database. The copy is written first and always -- \
                                there is no way to drop a set without saving it somewhere, and no \
                                flag to skip that. The set's own exported file is left alone, which \
                                is what a set is restored from afterwards. Use this to undo an \
                                import you did not mean to keep. A read-only set is dropped \
                                without a copy unless you ask for one, since its memories came \
                                from its publisher's file. The local set cannot be dropped: \
                                this takes a set name, and `local` names a shared set of that name \
                                if one exists.",
                "required": ["action", "name"],
                "properties": {
                    "action": {"const": "drop"},
                    "name": {
                        "type": "string",
                        "description": "The shared set to remove, by name. `hoard_sets` with action \
                                        \"status\" lists what this machine holds.",
                    },
                    "save_to": save_to_schema(),
                },
            },
        ],
    })
}

/// The actions the schema advertises, read out of the branches the schema is built from.
///
/// Derived rather than kept beside them, so the list a refusal names and the list a caller can
/// discover cannot drift apart.
fn published_set_actions() -> Vec<String> {
    sets_branches()["oneOf"]
        .as_array()
        .map(|variants| {
            variants
                .iter()
                .filter_map(|variant| variant["properties"]["action"]["const"].as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The name an outcome reports as, so the response and the enum cannot drift.
fn outcome_name(outcome: reconcile::Outcome) -> &'static str {
    match outcome {
        reconcile::Outcome::Unchanged => "unchanged",
        reconcile::Outcome::TakeFile => "took_from_file",
        reconcile::Outcome::KeepLocal => "kept_local",
        reconcile::Outcome::Converged => "converged",
        reconcile::Outcome::Conflict => "conflict",
        reconcile::Outcome::NewFromFile => "new_from_file",
        reconcile::Outcome::Anomaly => "anomaly",
        reconcile::Outcome::KeptForgotten => "kept_forgotten",
    }
}

/// Deserialise into `Some(_)` whenever the field is *present*, including when its value is `null`.
///
/// Needed because `serde` collapses a `null` into the outer `None` of an `Option<Option<T>>`, which
/// makes "omitted" and "explicitly cleared" indistinguishable. `amend` has to tell them apart: the
/// first means leave the field alone, the second means remove a value recorded in error, and without
/// the distinction there would be no way to delete a wrong `context` — only to overwrite it with
/// different text.
///
/// `#[serde(default)]` supplies `None` when the key is absent; this runs only when it is present.
fn present_even_if_null<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// The `hoard_correct` argument schema, one branch per action.
///
/// Flattened by [`with_flat_view`] for the same reasons `hoard_sets` is: published as `oneOf` alone
/// this tool was uncallable from a client that reads only `properties`, and published *with* a
/// top-level `oneOf` at all it is refused by the API.
fn correct_schema() -> Value {
    with_flat_view(correct_branches())
}

/// The per-action branches `hoard_correct` is defined by. Internal: see [`correct_schema`].
fn correct_branches() -> Value {
    json!({
        "type": "object",
        "oneOf": [
            {
                "title": "amend",
                "description": "Repair a memory that has never been exported -- any part of it, \
                                content and why included. Omit a field to leave it alone. Once a \
                                memory has been exported to anywhere, including a backup, this is \
                                refused: a second copy of it exists, so the change has to carry \
                                ancestry and the instrument is supersession -- store the corrected \
                                memory with a `supersedes` link, which keeps the old version \
                                readable as a record of what was believed. The server decides which \
                                of the two applies; you do not have to work it out.",
                "required": ["action", "scope", "id"],
                "properties": {
                    "action": {"const": "amend"},
                    "scope": scope_schema("Which set holds the memory."),
                    "id": {
                        "type": "string",
                        "description": "The memory's full id. Not a prefix -- a prefix constrains \
                                        time rather than identity, so it could match a memory \
                                        written moments away from the one you mean.",
                    },
                    // Declared, and that is the point rather than a formality: an argument the
                    // schema does not name is stripped by a validating client before it is sent, so
                    // an amendable field left out here is amendable only in theory
                    // (`amending-a-claim-is-refused-where-no-client-can-reach-it`, which is the
                    // defect `content` and `why` below are declared to close).
                    "content": {
                        "type": "string",
                        "description": "Replaces the memory itself. Reaches only a memory that has \
                                        never been exported -- nobody has read that one, so there \
                                        is no belief to destroy.",
                    },
                    "why": {
                        "type": "string",
                        "description": "Replaces the reasoning. Send null to remove it. Same \
                                        condition as `content`.",
                    },
                    "origin": {
                        "type": "string",
                        "enum": Origin::ALL.iter().map(serde_name).collect::<Vec<_>>(),
                        "description": "Corrects whose reasoning produced the content, where it was \
                                        recorded wrongly. Nobody believed the wrong value, so this \
                                        is an amendment rather than a new claim.",
                    },
                    "keywords": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Replaces every keyword. An empty list removes them all.",
                    },
                    "links": {
                        "type": "array",
                        "items": links_schema()["items"].clone(),
                        "description": "Replaces every outbound link. An empty list removes them \
                                        all.",
                    },
                    // A single scalar type, not the union `["string", "null"]` this published
                    // until 2026-08-11. A union is an alternation like any other and was reduced to
                    // `{}` by the same client that empties a nested `oneOf`, leaving the field with
                    // no type at all -- so declaring one type carries *more* than declaring two.
                    // Null is still accepted and still clears the field; it is simply no longer
                    // advertised through a construct that does not arrive.
                    "context": {
                        "type": "string",
                        "description": "Replaces the context. Send null to remove it.",
                    },
                    "confidence": {"type": "number", "description": "0.0 to 1.0."},
                },
            },
            {
                "title": "forget",
                "description": "Remove a memory from the set entirely, leaving a tombstone that \
                                records only its id and who forgot it. Use this when the memory \
                                should not exist at all -- sensitive data reached it, or it \
                                characterises a person unfairly. Not for a claim that is merely \
                                wrong or out of date: that is superseded, which keeps the old \
                                version readable. Forgetting does not reach the history of a \
                                repository that already carries the memory.",
                "required": ["action", "scope", "id"],
                "properties": {
                    "action": {"const": "forget"},
                    "scope": scope_schema("Which set holds the memory."),
                    "id": {
                        "type": "string",
                        "description": "The memory's full id. Not a prefix -- a prefix constrains \
                                        time rather than identity, and this operation cannot be \
                                        undone.",
                    },
                },
            },
        ],
    })
}

fn unknown_correction(action: &str) -> String {
    let available: Vec<String> = correct_branches()["oneOf"]
        .as_array()
        .map(|variants| {
            variants
                .iter()
                .filter_map(|variant| variant["properties"]["action"]["const"].as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let available = available.join(", ");

    if action.is_empty() {
        format!("hoard_correct needs an action. The actions are: {available}.")
    } else {
        format!(
            "unknown action {action:?}. hoard_correct accepts: {available}. `amend` repairs a \
             memory that has never been exported; a memory that has been, you correct by storing \
             the corrected version with a `supersedes` link through hoard_memorize."
        )
    }
}

fn unknown_action(action: &str) -> String {
    let available = published_set_actions().join(", ");
    if action.is_empty() {
        format!("hoard_sets needs an action. The actions are: {available}.")
    } else {
        format!("unknown action {action:?}. hoard_sets accepts: {available}.")
    }
}

/// The published shape of a [`Scope`] argument.
///
/// **A scalar string, because an alternation does not survive the trip.**
///
/// This published a `oneOf` between a bare string and an object until 2026-08-11 — the shape serde's
/// external tagging actually accepts. At least one client reduces a nested alternation to `{}`, and
/// a property with no type then carries an object argument as *text*, so `{"shared": "x"}` arrives
/// as the string `{"shared":"x"}` and comes back as `unknown variant`. On that client no shared set
/// could be named at all, which is the client a global installation uses.
///
/// So the published form is the scalar one, `local` or `shared:<name>` — the spelling
/// [`Scope::as_key`] has always written into a store's `meta` row. Both forms still *parse*; see the
/// deserialiser on `Scope`, and `documents/design/tool-surface.md` for why publishing narrowly and
/// accepting widely is the right asymmetry when the server has no influence over its clients.
///
/// The examples are generated rather than written out. The literal `project` was published here
/// after the variant was renamed to `Shared`, advertising a key the deserialiser no longer took
/// while every test passed, and generating them is what stops that recurring.
/// The `save_to` argument both destructive actions take.
///
/// **Written once and shared, because it is one argument with one meaning**: where a set's current
/// contents go before they stop being its contents. The two actions differ in what happens next --
/// `drop` destroys the set, `restore` replaces it -- and that difference belongs to the actions
/// rather than to this. The flat schema publishes one description per property name anyway, taking
/// whichever branch it meets first, so two hand-written versions would mean one of them was
/// published and the other was decoration that could quietly go stale.
fn save_to_schema() -> Value {
    json!({
        "type": "string",
        "description": "The whole path to write the set's current contents to before they stop \
                        being the set's contents. This is the copy you would work back from if the \
                        operation turns out to have been a mistake, so it goes somewhere the set \
                        does not already live: a path this set is exported to is refused, as is \
                        anywhere inside a `scatterhoard_mem`, and for `restore` and `absorb` so is \
                        the file being read -- writing over that would destroy the good copy in the \
                        act of protecting the bad one. The directory has to exist already. For a \
                        set this machine could have written to it is written first and always -- \
                        there is no way to skip it and no flag to force one -- so `drop` requires \
                        it unless the set is read-only, where its memories came from a file its \
                        publisher holds and no copy is written unless you give this anyway. What lands there is an ordinary memory file, so it can be diffed \
                        against the result and individual memories put back.",
    })
}

fn scope_schema(description: &str) -> Value {
    let shared = Scope::Shared(SetName::new("example").expect("a valid probe name")).as_key();
    let local = Scope::Local.as_key();

    json!({
        "type": "string",
        "description": format!(
            "{description} Either `{local}` -- this machine only, never shared to a repository and \
             so never through export review, though it can still be exported, which for this scope \
             means writing a backup -- or `{shared}` naming a set that can be exported and shared, \
             written with the set's own name after the colon.",
        ),
    })
}

/// The published shape of a `links` argument.
///
/// The label vocabulary comes from [`LinkLabel::ALL`], so adding a label reaches clients without a
/// second edit here. A label the server accepts but never advertises is unreachable in practice,
/// which is the same defect as one it advertises and refuses.
///
/// **Named in the description, not published as an `enum`**, since 2026-09-28. A label written by a
/// newer build may be restated across a supersession or an amendment, and an `enum` would have a
/// validating client refuse that before the server saw it -- and a model reading the schema never
/// write it -- so the restating rule would guard nothing. Typos are refused by the server instead,
/// in [`unknown_label_refusal`], which names the vocabulary again.
fn links_schema() -> Value {
    json!({
        "type": "array",
        "description": "Labelled edges to other memories, written outbound from this one.",
        "items": {
            "type": "object",
            "required": ["label", "target"],
            "properties": {
                "label": {
                    "type": "string",
                    "description": format!(
                        "What this memory claims about the target: one of {}. A link copied \
                         from the memory being superseded or amended keeps its label as written, \
                         even one not listed here.",
                        label_vocabulary()
                    ),
                },
                "target": {
                    "type": "string",
                    "description": "The id of the memory this one points at. A target need not \
                                    resolve: a link to a set that is not loaded is allowed and \
                                    stays dangling.",
                },
                "target_set": {
                    "type": "string",
                    "description": "Only for a memory in *another* set: the set's bare name, \
                                    `coding-lessons` where `scope` takes `shared:coding-lessons`. \
                                    Omit it for the ordinary case of a link within this set.",
                },
                "target_keywords": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Only alongside `target_set`: the target's keywords, kept so a \
                                    reference that goes dangling has something to be re-found by.",
                },
            },
        },
    })
}

/// The wire form of [`ModelState`], as one projection applied in one place.
///
/// Written as a single function rather than inline at each call site because the read surface has
/// already produced one defect of exactly that kind — `lookup-and-recall-disagree-on-shape`, where
/// two call sites each chose their own fields. A caller comparing two servers' answers is the whole
/// point of reporting this, so the two answers have to be built the same way.
///
/// **Identities are named, not summarised to a boolean.** Two servers reported byte-identical status
/// for one store on 2026-08-09 while holding different models; a flag cannot express *whose* vectors
/// these are, which is the question a caller in that situation actually has.
fn describe_model_state(state: &ModelState) -> Value {
    match state {
        ModelState::Unembedded => json!({
            "state": "unembedded",
            "note": "Nothing has embedded into this set, so it holds no vectors. Keyword search \
                     and hoard_lookup are the search here, and are unaffected.",
        }),
        ModelState::ProvenanceUnknown { vectors } => json!({
            "state": "provenance_unknown",
            "vectors": vectors,
            "note": "This set holds vectors and nothing records which model produced them, which \
                     is what a store written before model identity was recorded looks like. They \
                     cannot be compared against anything until a model rebuilds them.",
        }),
        ModelState::Current { model } => json!({
            "state": "current",
            "recorded": model.as_str(),
        }),
        // **No model is not a second model**, so it gets its own note. The state is the same --
        // vectors this server cannot use -- but the remedy is opposite: there is nothing to rebuild
        // with and no other configuration to reconcile, only a model to make loadable again.
        // `a-missing-model-reports-as-a-mismatch`.
        ModelState::Mismatched {
            recorded,
            loaded: None,
        } => json!({
            "state": "mismatched",
            "recorded": recorded.as_str(),
            "loaded": null,
            "note": format!(
                "This set's vectors were built under `{recorded}`, and no embedding model is \
                 loaded in this server, so there is nothing to search them with or rebuild them \
                 from. Nothing is lost: the vectors are kept, and a memory written meanwhile is \
                 embedded once a model returns. Make `{recorded}` loadable again -- this server \
                 reads its model from `SCATTERHOARD_MODEL`, or from `model` in its data \
                 directory, and says on startup why loading failed. Keyword search and \
                 hoard_lookup are unaffected."
            ),
        }),
        ModelState::Mismatched {
            recorded,
            loaded: Some(loaded),
        } => json!({
            "state": "mismatched",
            "recorded": recorded.as_str(),
            "loaded": loaded.as_str(),
            "note": "This set's vectors were built under a different embedding identity than \
                     this server holds, so they cannot be compared against each other. The \
                     identity names the model's weights and how this server encodes text with \
                     them, and a difference in either is enough -- compare the two strings above \
                     to see which it is. Keyword search and hoard_lookup are unaffected. Rebuild \
                     the vectors with `hoard_sets` action \"reembed\" and this scope, or, if \
                     another server is running with its own model, fix the disagreement between \
                     the two configurations first.",
        }),
    }
}

/// Where a candidate for a shared set's publishing name is, said wherever that name is asked for.
///
/// `author-default-from-remote-url`. The owner part of a remote URL is a handle in the namespace the
/// set is published into, which is the shape the name wants. **The session looks it up rather than
/// the server**, decided 2026-10-10: the server cannot see which repository a session is in, and the
/// one place it records a set's location -- the paths in `bases` -- is empty for a new set at its
/// first write, which is exactly when the name is asked for. Said in terms of hosting rather than of
/// one version control system's commands, so a session in any of them knows what to look for, and one
/// in none skips it.
///
/// **A suggestion inside the question, never an answer.** The first form of this sentence offered the
/// owner as what the name "may be", and `gemma4:12b` minted the stranger's handle ten times in ten;
/// framed as something to put to the human, and placed after the instruction to stop and ask, it was
/// put to them six times in ten (`documents/design/asking-the-human.md`).
const PUBLISHING_NAME_HINT: &str = "If the set is published from a repository, you can suggest in \
     your question the account or organisation hosting it -- such as the owner part of its remote \
     URL -- but only they can confirm it, because whoever owns a repository is often not who writes \
     to it.";

/// Why a write cannot be attributed: nobody has said who they are in this set, and nothing supplies a
/// name. Names both ways through, because which one is right depends on whether the set already holds
/// the writer's record from another machine -- and only the writer knows that.
///
/// **A shared set's refusal stops the session before it says how to proceed.** Opening with how to
/// call `mint` read as a recipe with one blank, and a weak reader filled the blank with a name it made
/// up, every time; saying first that the name is the human's and to wait for it is the order that
/// worked (`documents/design/asking-the-human.md`). Where the server can ask the human itself, this
/// refusal will not be needed at all; until then it is the whole of the safeguard before the write,
/// and [`recorded_name_relay`] is the one after it.
fn nobody_named_refusal(scope: &Scope) -> String {
    let set = json!(WireScope(scope));
    match scope {
        Scope::Shared(_) => format!(
            "nothing written: no name has been given for whoever writes to {set} on this machine, \
             and it is a name only the human can give you -- how they are known where this set is \
             published, which varies by set. Ask them for it now, and do not call `hoard_sets` until \
             they have answered. {PUBLISHING_NAME_HINT} With their answer, `hoard_sets` action \
             \"mint\" with {set} and that name makes the identity record every write here refers \
             to. If the set already holds their record from another machine, action \"identify\" \
             with its id points at it instead: recall the keyword `hoard-author` in this set to see \
             what is there."
        ),
        Scope::Local => format!(
            "nothing written: nobody has said who writes to {set} on this machine, and the \
             operating system supplied no account name to default to. `hoard_sets` action \"mint\" \
             with {set} and the name to write under makes the identity record every write here \
             refers to. If the set already holds your record from another machine, action \
             \"identify\" with its id points at it instead: recall the keyword `hoard-author` in \
             this set to see what is there."
        ),
    }
}

/// Why an author action needs its argument from the model on this request: the human cannot be
/// asked directly, so the fallback decided 2026-10-11 applies.
fn model_must_name(action: &str, argument: &str) -> String {
    format!(
        "`{action}` needs `{argument}` here: this connection cannot ask the human directly, so ask \
         them what it is, wait for their answer, and pass it."
    )
}

/// The sentence a session relays to the human after recording a name for them in a shared set, or
/// nothing for `local`, which is never published.
///
/// **The safeguard after the write, where the one before it can fail.** Until the server can ask the
/// human directly, the name reaches `mint` and `rename` through the model, and a weak model sometimes
/// supplies one nobody gave. Decided 2026-10-11 by the human partner: rather than keep the name out of
/// the model at the cost of a terminal command, make a wrong one visible at once, in a sentence that
/// invites the correction `rename` makes cheap. **Verbatim and last** because that is what was
/// relayed: as prose, `gemma4:12b` named the author ten times in ten but offered the correction four;
/// as a sentence to end the reply with, twenty in twenty did both
/// (`documents/design/asking-the-human.md`).
fn recorded_name_relay(scope: &Scope, author: &Author) -> Option<String> {
    match scope {
        Scope::Shared(_) => Some(format!(
            "End your reply to the human with this sentence: I recorded \"{name}\" as the name you \
             write under in {set} -- if that is not how you are known where this set is published, \
             tell me the right name and I will change it.",
            name = author.as_str(),
            set = scope.as_key(),
        )),
        Scope::Local => None,
    }
}

/// Where a name an action records came from, which decides whether the human needs telling.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The human typed it into the server's own form, so they know what they gave.
    Human,
    /// The model supplied it, so the human is told what was recorded in their name.
    Model,
}

/// `note`, led by the sentence to relay where the model supplied the name.
fn with_relay(scope: &Scope, author: &Author, source: Source, note: &str) -> String {
    if source == Source::Human {
        return note.to_owned();
    }
    match recorded_name_relay(scope, author) {
        Some(relay) => format!("{relay} {note}"),
        None => note.to_owned(),
    }
}

/// What is worth saying about who a set attributes writes to, or nothing when nothing is.
///
/// **Silence is the common case and the right one**, and what counts as common changed. A set whose
/// identity is not this server's configuration is *expected* — identity follows a set's origin, so
/// one person differs from most of their own sets — and reporting that as a discrepancy fires
/// forever on healthy sets, which teaches a reader to skip the field.
///
/// What earns a line is what the caller is about to run into: a set holding identity records this
/// machine has not chosen between, where the next write is refused; an unclaimed set, where the next
/// write decides; and a store predating set-level attribution, whose silence would otherwise read as
/// *nothing has been written*.
///
/// **Reported and never acted on**, which is the resolution `documents/design/embeddings.md` reached
/// for the embedding model and the reason attribution is stable at all: a server that restamped what
/// it disagreed with would take the store back on every write, forever, and each one's local
/// experience would be success.
fn describe_author(
    claimed: bool,
    default: Option<&Author>,
    memories: usize,
    must_ask: bool,
    asks: bool,
) -> Option<String> {
    // **Where the human can be asked, the next write asks them**, and the note says so rather than
    // sending the model to `mint` or `identify` on their behalf. `asking-the-human`.
    if must_ask && asks {
        return Some(
            "This set holds identity records and this machine has not said which one is you. The \
             next write here asks the human, through their client, which of them is theirs -- or \
             that they are new -- so do not ask them yourself."
                .to_owned(),
        );
    }
    if must_ask {
        return Some(
            "This set holds identity records and this machine has not said which one is you, so the \
             next write here will be refused rather than guess. Recall the keyword `hoard-author` in \
             this set to see them, then `hoard_sets` with action \"identify\" and the id of yours -- \
             or action \"mint\" if you are new to this set and none of them is. Guessing by name \
             would fork one person into two identities, which nothing afterwards could tell from two \
             contributors."
                .to_owned(),
        );
    }
    // **A set that has been claimed says nothing**, whatever name it attributes to. Identity follows
    // a set's origin, so one person's name differs across their own sets -- permanently, and
    // correctly -- and a note that fires on a healthy set is one readers learn to skip.
    // `the-author-note-compares-against-the-wrong-thing`.
    if claimed {
        return None;
    }
    // Two states an unclaimed set can be in, and saying the wrong one is worse than saying nothing.
    // An empty set has genuinely had nothing written to it; one holding memories with no author is a
    // store written before set-level attribution. `author-note-denies-writes-to-a-set-that-has-them`.
    let state = if memories == 0 {
        "Nothing has been written to this set yet, so it records no author."
    } else {
        "This set holds memories but records no author, which is what a store written before \
         set-level attribution looks like. The memories keep the identity they were written with; \
         it is the set that has not been claimed."
    };
    Some(match default {
        Some(name) => format!(
            "{state} The next write claims it for {name}, this machine's account name, and \
             `hoard_sets` action \"rename\" changes that afterwards."
        ),
        None if asks => format!(
            "{state} The next write here asks the human, through their client, for the name they \
             publish under in it -- do not ask them for it yourself."
        ),
        None => format!(
            "{state} The next write here is refused until you say who you are in it: `hoard_sets` \
             action \"mint\" with the name to write under, or \"identify\" if it already holds \
             your record from another machine."
        ),
    })
}

/// Small ordinals in words, so a warning reads as a sentence rather than as a field dump.
fn ordinal(n: usize) -> String {
    match n {
        2 => "second".to_owned(),
        3 => "third".to_owned(),
        4 => "fourth".to_owned(),
        _ => format!("{n}th"),
    }
}

/// A `tools/call`'s envelope, described without its arguments: which era it is served under, the
/// `_meta` keys it carries, and the capabilities it declares. Arguments are left out because they
/// are the model's content, and this is written to a log.
fn call_shape(params: &Value) -> String {
    let meta = &params["_meta"];
    let era = if meta.get(META_PROTOCOL).is_some() {
        format!("modern {}", meta[META_PROTOCOL])
    } else {
        "handshake".to_owned()
    };
    let keys: Vec<&str> = meta
        .as_object()
        .map(|object| object.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let carries_answers = params.get("inputResponses").is_some();
    format!(
        "served as {era}; _meta keys {keys:?}; capabilities {}; answers attached: {carries_answers}",
        meta.get(META_CAPABILITIES).unwrap_or(&Value::Null),
    )
}

/// How this server names itself, in `initialize` and in every modern result.
fn server_info() -> Value {
    json!({"name": "scatterhoard", "version": env!("CARGO_PKG_VERSION")})
}

fn success_response(id: Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn error_response(id: Value, code: i32, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}
