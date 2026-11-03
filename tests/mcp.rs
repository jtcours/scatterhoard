// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the MCP tool surface.
//!
//! The surface is a pure JSON request-to-response function, so these drive it directly rather than
//! through a transport. That is the point of keeping transport out: the tools are the part with
//! decisions in them, and they are testable without an MCP client or an async runtime.

use std::str::FromStr;
use std::sync::Arc;

use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::mcp::Server;
use scatterhoard::memory::{
    Attribution, Author, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId,
    MemoryText, Origin, Scope, SetName,
};
use scatterhoard::memory_file::MemorySet;
use scatterhoard::store::Store;
use serde_json::{Value, json};

#[path = "support/output_schema_check.rs"]
mod output_schema_check;

/// What every fixture set's charter says. See [`MarkerEmbedder`] for why the model cannot place it.
const FIXTURE_CHARTER: &str = "This set holds the fixtures these tests write.";

#[derive(Debug)]
struct MarkerEmbedder;

impl Embedder for MarkerEmbedder {
    fn identity(&self) -> ModelIdentity {
        // One identity for every test double, because these tests are about behaviour that assumes
        // a consistent model. The one place two models have to disagree is tests/model_identity.rs.
        ModelIdentity::new("marker-embedder")
    }

    fn dimension(&self) -> usize {
        3
    }
    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|text| {
                // **The fixture charter is outside this model's vocabulary**, which is the truth about
                // a model that knows two words. Placed anywhere in a three-dimensional space it
                // decides the ranking of every test's two or three memories, because centring over
                // so few moves the query as much as the memories -- an effect a real model's
                // dimensions dilute. Unrepresentable, it is stored and counted and left out of the
                // meaning search, which is what these tests are about.
                if text.contains(FIXTURE_CHARTER) {
                    return Ok(Embedded::Unrepresentable);
                }
                let count = |w: &str| text.matches(w).count() as f32;
                Vector::new(vec![count("alpha"), count("beta"), 0.01])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

fn scope() -> Scope {
    Scope::Shared(SetName::new("scatterhoard").expect("valid set name"))
}

/// Store a memory with a content and a why, returning its id.
fn memorize_full_with(server: &mut Server, content: &str, why: &str) -> String {
    let response = call(
        server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": content,
            "why": why,
            "confidence": 0.9,
            "origin": "llm"
        }),
    );
    assert!(!is_error(&response), "memorize failed: {response}");
    payload(&response)["id"].as_str().expect("an id").to_owned()
}

/// A memory built directly, for the cases that cannot go through the tool surface.
fn a_memory(content: &str) -> Memory {
    Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: jiff::Timestamp::from_str("2026-08-02T09:00:00Z").expect("valid timestamp"),
        },
    )
}

/// A charter, which a shared set's first memory has to be since `set-settled-at-first-write`.
///
/// Worded without `alpha` or `beta`, so the marker model places it nowhere near a test's queries.
fn a_charter() -> Memory {
    let mut charter = a_memory(FIXTURE_CHARTER);
    charter.keywords = vec![Keyword::new("hoard-scope-topics").expect("valid keyword")];
    charter
}

/// A shared store as a set looks once it has been given its charter, which its first memory has to be.
fn chartered(scope: Scope) -> Store {
    let mut store = Store::open_in_memory_and_mint_id(scope).expect("opens");
    // Named first, as a set's first write would: since `set-settled-at-first-write` a shared set
    // takes no write until someone has said who they are in it, and the charter is theirs.
    let author = store
        .author_record_or_mint(
            &Author::new("jtcours").expect("valid author"),
            jiff::Timestamp::now(),
        )
        .expect("records who writes here")
        .expect("the set holds no other identity record");
    let mut charter = a_charter();
    charter.created.by = author;
    charter.modified.by = author;
    store.insert(&charter).expect("takes its charter");
    store
}

fn a_server() -> Server {
    let mut store = chartered(scope());
    store.set_embedder(Arc::new(MarkerEmbedder));

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(store);
    server
}

/// Send a request and get the parsed response.
fn send(server: &mut Server, request: &Value) -> Value {
    let raw = server
        .handle(&request.to_string())
        .expect("a request with an id must be answered");
    serde_json::from_str(&raw).expect("the response should be JSON")
}

fn call(server: &mut Server, tool: &str, arguments: Value) -> Value {
    let response = send(
        server,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}
        }),
    );
    // Every response checked against its tool's published output schema, strictly:
    // `response-shape-is-unpublished`.
    output_schema_check::check_response(tool, &response);
    response
}

/// A tool result carries JSON as text; this is the payload.
fn payload(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a text result, got {response}"));
    serde_json::from_str(text).expect("the tool payload should be JSON")
}

/// A convention page split into its parts: the catalog's slice parsed as JSON where it is whole,
/// the aside the server said beside it (an empty object if none), and the page's last line.
fn conventions_page(response: &Value) -> (Value, Value, String) {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a text result, got {response}"));
    let lines: Vec<_> = text.split('\n').collect();
    let (slice, aside, last) = match lines.as_slice() {
        [slice, aside, last] => (
            *slice,
            serde_json::from_str(aside).expect("JSON aside"),
            *last,
        ),
        [slice, last] => (*slice, json!({}), *last),
        _ => panic!("a convention page is a slice, an optional aside and a last line: {text}"),
    };
    let catalog = serde_json::from_str(slice).unwrap_or(Value::Null);
    (catalog, aside, last.to_owned())
}

fn is_error(response: &Value) -> bool {
    response.get("error").is_some() || response["result"]["isError"] == json!(true)
}

/// The human-readable text of a refusal, wherever the transport put it.
fn message(response: &Value) -> String {
    response["result"]["content"][0]["text"]
        .as_str()
        .or_else(|| response["error"]["message"].as_str())
        .unwrap_or_else(|| panic!("expected a message, got {response}"))
        .to_owned()
}

/// Every place a scope appears in `value`, spelled as a serde-derived object rather than as the
/// scalar the schema publishes.
///
/// `a-scope-in-a-response-is-not-the-spelling-the-schema-publishes`. Walks the whole payload rather
/// than naming fields, because the point is that *no* response carries the object form — a test
/// listing the fields it knows about would pass for a field added later, which is how the two
/// spellings came to coexist in the first place.
fn object_form_scopes(value: &Value, path: &str, found: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            if fields.len() == 1 && fields.get("shared").is_some_and(Value::is_string) {
                found.push(format!("{path}: {value}"));
            }
            for (key, child) in fields {
                object_form_scopes(child, &format!("{path}.{key}"), found);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                object_form_scopes(item, &format!("{path}[{index}]"), found);
            }
        }
        _ => {}
    }
}

fn memorize(server: &mut Server, content: &str) -> Value {
    call(
        server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": content,
            "why": "because the test needs one",
            "confidence": 0.8,
            "origin": "llm"
        }),
    )
}

// ---------------------------------------------------------------------------
// Protocol
// ---------------------------------------------------------------------------

#[test]
fn initialize_reports_the_protocol_version_and_server() {
    let mut server = a_server();
    let response = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    );

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert!(response["result"]["protocolVersion"].is_string());
    assert_eq!(response["result"]["serverInfo"]["name"], "scatterhoard");
}

/// `response-shape-is-unpublished`. Output schemas and `structuredContent` arrived in the MCP
/// revision of 2025-06-18, so the server answers that revision when a client asks for it and the
/// older one when it asks for that, per the specification's rule: a version the server supports is
/// echoed, and any other gets the latest it supports.
#[test]
fn initialize_answers_the_protocol_version_the_client_asks_for_when_it_can() {
    for (asked, answered) in [
        (json!("2025-06-18"), "2025-06-18"),
        (json!("2024-11-05"), "2024-11-05"),
        (json!("2099-01-01"), "2025-06-18"),
        (Value::Null, "2025-06-18"),
    ] {
        let mut server = a_server();
        let mut params = json!({});
        if !asked.is_null() {
            params["protocolVersion"] = asked.clone();
        }
        let response = send(
            &mut server,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": params}),
        );
        assert_eq!(
            response["result"]["protocolVersion"],
            json!(answered),
            "asked {asked}"
        );
    }
}

/// An output schema's top level says `"type": "object"`, whatever else it says. Claude's client
/// refused the whole tool list on 2026-10-03 because two schemas opened with an `anyOf` and no
/// type, which took every tool offline -- so this is pinned on every tool that publishes one.
#[test]
fn every_output_schema_is_an_object_at_its_top_level() {
    let mut server = a_server();
    let listed = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );
    for tool in listed["result"]["tools"].as_array().expect("tools") {
        if let Some(schema) = tool.get("outputSchema") {
            assert_eq!(
                schema["type"],
                json!("object"),
                "{}'s output schema",
                tool["name"]
            );
        }
    }
}

/// The output schemas and `structuredContent` belong to the 2025-06-18 revision, so a client that
/// negotiated the older one gets neither -- which is also how a client avoids their cost: the
/// schemas nearly triple the tool list. Decided with the human partner on 2026-10-03.
#[test]
fn a_client_on_the_older_protocol_gets_no_output_schemas_or_structured_content() {
    for (version, published) in [("2024-11-05", false), ("2025-06-18", true)] {
        let mut server = a_server();
        send(
            &mut server,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {"protocolVersion": version}}),
        );
        let listed = send(
            &mut server,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        );
        let with_schema = listed["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter(|tool| tool.get("outputSchema").is_some())
            .count();
        assert_eq!(
            with_schema > 0,
            published,
            "{version}: output schemas published?"
        );

        let recalled = send(
            &mut server,
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": {"name": "hoard_recall", "arguments": {"keywords": ["alpha"]}}}),
        );
        assert_eq!(
            recalled["result"].get("structuredContent").is_some(),
            published,
            "{version}: structured content returned? {recalled}"
        );
    }
}

/// A notification has no id and must not be answered — replying to one is a protocol violation
/// that some clients treat as a hard error.
#[test]
fn a_notification_gets_no_reply() {
    let mut server = a_server();
    let request = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});

    assert!(server.handle(&request.to_string()).is_none());
}

#[test]
fn malformed_json_is_reported_as_a_parse_error() {
    let mut server = a_server();
    let response: Value =
        serde_json::from_str(&server.handle("{not json").expect("must answer")).expect("JSON");

    assert_eq!(response["error"]["code"], -32700);
    assert_eq!(
        response["id"],
        Value::Null,
        "a parse error has no id to echo"
    );
}

#[test]
fn an_unknown_method_is_reported_as_such() {
    let mut server = a_server();
    let response = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 7, "method": "nonsense/method"}),
    );

    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(response["id"], 7);
}

// ---------------------------------------------------------------------------
// The surface itself
// ---------------------------------------------------------------------------

/// Deliberately small. The design warns that a user may have several MCP servers loaded and that
/// too many tools confuse the model, so the surface is the Vision's store and recall, the convention
/// loading the trigger design requires, and lifecycle work — nothing else until it earns a place.
///
/// **Asserts properties rather than a list of names.** This test used to compare against a
/// hand-written vector of the four tool names, which is the literal `STANDARDS.md` warns about: the
/// list was a second copy of `tool_definitions`, so adding a fifth tool failed the test without
/// anything being wrong. What the design actually constrains is that the surface stays *small*, that
/// every tool is namespaced, and that each is callable from what it publishes.
#[test]
fn the_tool_surface_stays_small_and_self_describing() {
    let mut server = a_server();
    let response = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );

    let tools = response["result"]["tools"].as_array().expect("a list");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();

    // A bound, not a census. `tool-surface.md` argues the cost of a tool is permanent attention;
    // the number where that starts to bite is the thing worth pinning, and raising it should be a
    // deliberate edit with a reason.
    assert!(
        names.len() <= 6,
        "the surface is meant to stay small; it now offers {}: {names:?}",
        names.len()
    );

    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        names.len(),
        "duplicate tool name in {names:?}"
    );

    // Conventions first, because `trigger-and-loading.md` requires loading them at session start,
    // and the first thing listed is what a model reaches for first.
    assert_eq!(names.first(), Some(&"hoard_conventions"));

    for tool in tools {
        let name = tool["name"].as_str().expect("a name");
        assert!(
            name.starts_with("hoard_"),
            "{name} is not namespaced; it will collide with another server's tools"
        );
        assert!(
            tool["description"].as_str().is_some_and(|d| d.len() > 20),
            "{name} needs a description the model can act on"
        );

        // **Top-level `properties`, always** -- never `oneOf` alone.
        //
        // This assertion read `properties.is_object() || oneOf.is_array()` until 2026-08-11, on the
        // stated grounds that both shapes are constructible from what is published. A real client
        // disproved it: `hoard_sets` and `hoard_correct` published `oneOf` and no top-level
        // `properties`, the client presented them as taking no arguments, and every call was
        // stripped bare and refused. The whole management surface was uncallable while this test
        // was green. `oneof-tool-schemas-present-no-parameters`.
        let schema = &tool["inputSchema"];
        let properties = schema["properties"].as_object();
        assert!(
            properties.is_some_and(|published| !published.is_empty()),
            "{name} publishes no top-level properties, so a consumer that reads `properties` and \
             ignores `oneOf` sees a tool that takes nothing: {schema}"
        );

        // And where the tool is action-dispatched, the flat view has to reach *every* action. The
        // second failure mode was a client that merged the branches but kept the first branch's
        // `const`, leaving a schema that could only ever express `status`.
        //
        // This compared the flat enum against the published `oneOf` branches until 2026-08-11, when
        // the branches stopped being published at all — which turned the comparison into an
        // `if let` that never matched and an assertion that could no longer fail. Caught only
        // because removing them broke a *different* test in `tests/sets.rs`. The branches are not
        // visible from out here any more, so what is checkable is that the enum exists and is not
        // truncated to one; `every_action_the_schema_publishes_is_accepted` carries the other half
        // by driving every published action through the deserialiser.
        if let Some(action) = schema["properties"].get("action") {
            let offered: Vec<&str> = action["enum"]
                .as_array()
                .map(|values| values.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            assert!(
                offered.len() > 1,
                "{name} dispatches on an action but publishes {offered:?}; a single value is what \
                 a consumer that kept only the first branch's `const` would leave behind"
            );
            assert!(
                action["description"]
                    .as_str()
                    .is_some_and(|d| offered.iter().all(|a| d.contains(a))),
                "{name} must say what each action takes, since nothing else published does: \
                 {action}"
            );
        }
    }
}

/// **Every argument an action-dispatched tool publishes is named in its action prose**, optional
/// ones included. The prose is the only place a consumer that flattens the schema learns which action
/// takes what; an argument missing from it is invisible there. Written 2026-09-16 when `create`
/// gained `mode` and the prose -- built from required arguments and alternations only -- left it out,
/// which turned out to have hidden every optional field of `hoard_correct`'s `amend` all along.
#[test]
fn every_published_argument_is_named_in_the_action_prose() {
    let mut server = a_server();
    let listed = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );
    for tool in listed["result"]["tools"].as_array().expect("a list") {
        let schema = &tool["inputSchema"];
        let Some(prose) = schema["properties"]["action"]["description"].as_str() else {
            continue;
        };
        for argument in schema["properties"].as_object().expect("properties").keys() {
            if argument == "action" {
                continue;
            }
            assert!(
                prose.contains(argument.as_str()),
                "{} publishes `{argument}` but its action prose never names it: {prose}",
                tool["name"]
            );
        }
    }
}

#[test]
fn conventions_are_served_whole_without_a_budget() {
    let mut server = a_server();
    let (result, _, last) = conventions_page(&call(&mut server, "hoard_conventions", json!({})));

    assert!(last.ends_with("conventions are loaded."), "{last}");
    let conventions = result["conventions"].as_array().expect("a list");
    assert!(!conventions.is_empty());
    assert!(conventions[0]["id"].is_string());
    assert!(
        conventions[0]["reinforcement"].is_string(),
        "reinforcement is served with the convention, never separately"
    );
}

// ---------------------------------------------------------------------------
// Storing
// ---------------------------------------------------------------------------

#[test]
fn memorizing_returns_the_new_memory_id() {
    let mut server = a_server();
    let result = payload(&memorize(&mut server, "alpha alpha"));

    let id = result["id"].as_str().expect("an id");
    assert_eq!(
        id.len(),
        26,
        "a memory id is 26 Crockford characters, per `crockford-base32-ids`: {id}"
    );
    assert!(
        !id.contains('-'),
        "and carries no separator to truncate at: {id}"
    );
}

/// Storing has to say whether the thing just stored can be found by meaning.
///
/// This is the detector that was missing when `project-stores-get-no-embedder` happened: embedding
/// skips silently with no model, so memories were written with no vector — permanently unfindable
/// by meaning — and the surface reported plain success every time. That cause is fixed; without
/// this, the next cause would be just as invisible.
#[test]
fn memorizing_says_the_memory_can_be_found_by_meaning() {
    let mut server = a_server();
    let result = payload(&memorize(&mut server, "alpha alpha"));

    assert_eq!(
        result["findable_by_meaning"],
        json!(true),
        "a store with a model must report the memory as findable: {result}"
    );
}

/// The case that matters, and the reason the field is about *this write* rather than about the
/// server: a store with no model accepts the memory and cannot make it findable.
#[test]
fn memorizing_into_a_store_with_no_model_says_the_memory_cannot_be_found_by_meaning() {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(chartered(scope()));

    let result = payload(&memorize(&mut server, "alpha alpha"));

    assert!(
        result["id"].is_string(),
        "the memory is still stored -- this is not a refusal: {result}"
    );
    assert_eq!(
        result["findable_by_meaning"],
        json!(false),
        "with no model the memory has no vector and must say so: {result}"
    );
}

/// A caller told something is wrong has to be told what it can do about it, per the standing
/// lesson about error text naming an action its reader cannot take. At this moment the caller can
/// still add keywords, so the note has to say so rather than only naming the missing model.
#[test]
fn a_memory_that_cannot_be_found_by_meaning_comes_with_something_to_do_about_it() {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(chartered(scope()));

    let result = payload(&memorize(&mut server, "alpha alpha"));
    let note = result["note"].as_str().unwrap_or_default().to_lowercase();

    assert!(
        note.contains("keyword"),
        "the note must point at keywords, which still work: {result}"
    );
    assert!(
        note.contains("hoard_lookup") || note.contains("lookup"),
        "and at the retrieval paths that are unaffected: {result}"
    );
}

/// The flag is a fact about the write, checked against the store, not a restatement of which
/// branch the code took. A server-wide capability flag is the exact shape that lied in
/// `project-stores-get-no-embedder`, so one store having a model must not make another's memory
/// report as findable.
#[test]
fn findability_is_reported_per_store_not_per_server() {
    let mut with_model = chartered(scope());
    with_model.set_embedder(Arc::new(MarkerEmbedder));

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(with_model);
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));

    let shared = payload(&memorize(&mut server, "alpha alpha"));
    let local = payload(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha alpha",
            "confidence": 0.8,
            "origin": "llm"
        }),
    ));

    assert_eq!(shared["findable_by_meaning"], json!(true), "{shared}");
    assert_eq!(local["findable_by_meaning"], json!(false), "{local}");
}

/// Scope is a required choice that the software cannot infer, and the type carries that: `Scope`
/// has no `Default`, so a call that omits it cannot be deserialised into one. This asserts the
/// guarantee survives all the way to the tool boundary.
#[test]
fn memorizing_without_a_scope_is_refused() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({"content": "alpha", "confidence": 0.8, "origin": "llm"}),
    );

    assert!(is_error(&response), "scope must never be optional");
}

/// Parse-don't-validate reaching the boundary: an out-of-range confidence is refused by the type's
/// own constructor rather than by a check in the tool.
#[test]
fn memorizing_with_an_invalid_field_is_refused() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha", "confidence": 1.8, "origin": "llm"
        }),
    );

    assert!(is_error(&response), "confidence 1.8 must not be accepted");
}

/// Naming a set this machine does not hold is an error, not an invitation to create one — a store
/// comes into existence by import, and inventing one on a typo would silently split a set in two.
#[test]
fn memorizing_into_an_unknown_scope_is_refused() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "some-other-set"},
            "content": "alpha", "confidence": 0.8, "origin": "llm"
        }),
    );

    assert!(is_error(&response));
}

// ---------------------------------------------------------------------------
// Recalling
// ---------------------------------------------------------------------------

#[test]
fn recall_finds_a_memorized_memory() {
    let mut server = a_server();
    memorize(&mut server, "alpha alpha alpha");
    memorize(&mut server, "beta beta beta");

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"scope": {"shared": "scatterhoard"}, "query": "alpha", "limit": 5}),
    ));

    let memories = result["memories"].as_array().expect("a list");
    assert_eq!(memories.len(), 2);
    assert!(memories[0]["content"].as_str().unwrap().contains("alpha"));
    assert!(memories[0]["id"].is_string());
}

/// Adding a store to a server is what gives it the model — so startup wiring cannot forget to.
///
/// This is the structural form of the `project-stores-get-no-embedder` fix. That defect was
/// `main.rs` calling `set_embedder` on the local store and not on the project stores, which no test
/// could see because no test binary runs `main.rs`. Moving the assignment onto `add_store` puts the
/// invariant somewhere reachable: a store that a server can search is a store the server embedded.
///
/// Both orders are covered deliberately. A store added *before* the model has to be caught up when
/// the model arrives, and one added *after* has to be embedded on the way in; wiring code has no
/// reason to prefer one order and should not have to know which is safe.
#[test]
fn every_store_a_server_holds_gets_the_server_model() {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.add_store(chartered(scope()));

    for target in [json!("local"), json!("shared:scatterhoard")] {
        call(
            &mut server,
            "hoard_memorize",
            json!({
                "scope": target,
                "content": "alpha alpha alpha",
                "why": "because the test needs one",
                "confidence": 0.8,
                "origin": "llm"
            }),
        );
    }

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ));

    assert_eq!(
        result["semantic_search_available"],
        json!(true),
        "one unembedded store makes the whole union report itself unsearchable: {result}"
    );
    let scopes: Vec<&Value> = result["memories"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|hit| &hit["scope"])
        .collect();
    assert!(
        scopes.contains(&&json!("local")) && scopes.contains(&&json!("shared:scatterhoard")),
        "a store that never got the model stores memories with no vector, silently, and they are \
         unreachable by meaning forever: {result}"
    );
}

/// The degradation has to reach the model, not just the Rust API. A session that believes memory is
/// being consulted when it is not is worse than one with no memory at all.
#[test]
fn recall_tells_the_model_when_semantic_search_is_unavailable() {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(chartered(scope()));
    memorize(&mut server, "alpha");

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"scope": {"shared": "scatterhoard"}, "query": "alpha"}),
    ));

    assert_eq!(result["semantic_search_available"], json!(false));
    assert!(
        result["note"].as_str().is_some_and(|n| !n.is_empty()),
        "an unavailable capability needs saying in words the model will read"
    );
}

#[test]
fn recall_from_an_unknown_scope_is_refused() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_recall",
        json!({"scope": {"shared": "not-loaded"}, "query": "alpha"}),
    );

    assert!(is_error(&response));
}

/// Omitting the scope searches every loaded set. Optional here although required on
/// `hoard_memorize`, and the asymmetry is deliberate: storing into the wrong set is a one-way
/// door, while searching the wrong one is a read that costs nothing.
#[test]
fn recall_without_a_scope_searches_every_loaded_set() {
    let mut store = Store::open_in_memory(Scope::Local).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    let mut server = a_server();
    server.add_store(store);

    memorize(&mut server, "alpha alpha");
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": "alpha beta",
            "confidence": 0.8, "origin": "llm"
        }),
    );

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 5}),
    ));

    let memories = result["memories"].as_array().expect("a list");
    assert_eq!(memories.len(), 2, "both loaded sets should be searched");

    let scopes: Vec<&Value> = memories.iter().map(|m| &m["scope"]).collect();
    assert!(
        scopes.contains(&&json!("local")) && scopes.contains(&&json!("shared:scatterhoard")),
        "every result must say which set it came from: {scopes:?}"
    );
}

/// `the-missing-query-refusal-gives-no-example`. A recall with nothing to search for is refused with
/// the caller's own call, the missing part added, so the next call can be that one. Tested on
/// `gemma4:12b` before it shipped: the old wording -- *give hoard_recall a query … or keywords* --
/// was followed by a call still missing both, 10 times out of 10; this was followed by a call with a
/// query 10 times out of 10. It also says recall has no listing mode, which is what a caller asking
/// for a whole set was reaching for.
#[test]
fn a_recall_with_nothing_to_search_for_is_refused_with_the_call_to_make() {
    let mut server = a_server();
    memorize(&mut server, "alpha");

    let refused = call(
        &mut server,
        "hoard_recall",
        json!({"limit": 5, "scope": "shared:scatterhoard", "detail": "summary"}),
    );
    assert!(is_error(&refused), "{refused}");
    let said = message(&refused);
    assert!(said.contains("no mode that lists"), "{said}");
    assert!(
        said.contains("`keywords`"),
        "it names the other way to search: {said}"
    );

    // The suggested call is the caller's own, with a query added -- and taking it works.
    let start = said.find('{').expect("an example call");
    let end = said.rfind('}').expect("an example call") + 1;
    let mut suggested: Value =
        serde_json::from_str(&said[start..end]).expect("the example is JSON");
    assert_eq!(suggested["limit"], json!(5), "{suggested}");
    assert_eq!(
        suggested["scope"],
        json!("shared:scatterhoard"),
        "{suggested}"
    );
    assert_eq!(suggested["detail"], json!("summary"), "{suggested}");
    assert!(suggested["query"].is_string(), "{suggested}");
    suggested["query"] = json!("alpha");
    let taken = call(&mut server, "hoard_recall", suggested);
    assert!(
        !is_error(&taken),
        "the remedy the refusal names must work: {taken}"
    );
}

/// `recall-limit-is-unbounded`, the part of it that stood. A limit of zero returned an empty list,
/// which reads exactly like *nothing matched*, so a caller's slip passed for an answer. Zero, a
/// negative number and a fraction are refused alike, on both search paths, naming the argument and
/// what it takes -- rather than in serde's words, which said `usize`.
#[test]
fn a_limit_below_one_or_not_whole_is_refused_by_name() {
    let mut server = a_server();
    memorize(&mut server, "alpha");

    for limit in [json!(0), json!(-3), json!(2.5)] {
        for search in [json!({"query": "alpha"}), json!({"keywords": ["alpha"]})] {
            let mut arguments = search.clone();
            arguments["limit"] = limit.clone();
            let response = call(&mut server, "hoard_recall", arguments);
            assert!(is_error(&response), "limit {limit} on {search}: {response}");
            let said = message(&response);
            assert!(
                said.contains("`limit`") && said.contains("whole number of 1 or more"),
                "limit {limit} should be refused by name and say what it takes: {said}"
            );
        }
    }
    let schema = tool_schema(&mut server, "hoard_recall");
    assert_eq!(schema["properties"]["limit"]["minimum"], json!(1));
}

/// With no limit, recall returns eight: a predictable count, chosen 2026-10-02 when the largest-gap
/// cut came out, from the one measurement of result-count quality, which put a fixed count's best
/// F1 at eight to ten. The schema publishes the same number.
#[test]
fn recall_returns_eight_when_no_limit_is_given() {
    let mut server = a_server();
    for word in [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ] {
        memorize(&mut server, &format!("alpha {word}"));
    }

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ));

    assert_eq!(result["memories"].as_array().expect("a list").len(), 8);
    let schema = tool_schema(&mut server, "hoard_recall");
    let described = schema["properties"]["limit"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(
        described.contains('8'),
        "the schema should publish the default: {described}"
    );
}

/// `keyword-recall-multiplies-the-limit-by-the-number-of-sets`, from the position it was measured
/// from: two loaded sets, `limit: 1`, two memories back — one per store, and in store order rather
/// than ranked. Driven through the surface because that is the only place the defect was visible;
/// with a single loaded store, which is what every other test uses, both halves are invisible.
#[test]
fn keyword_recall_applies_the_limit_to_the_answer_not_to_each_set() {
    let mut store = Store::open_in_memory(Scope::Local).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    let mut server = a_server();
    server.add_store(store);

    for scope in [json!("local"), json!("shared:scatterhoard")] {
        call(
            &mut server,
            "hoard_memorize",
            json!({
                "scope": scope, "content": "a memory carrying the keyword",
                "keywords": ["routing"], "confidence": 0.8, "origin": "llm"
            }),
        );
    }

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["routing"], "limit": 1}),
    ));

    let memories = result["memories"].as_array().expect("a list");
    assert_eq!(
        memories.len(),
        1,
        "the limit bounds the response, not each set's share of it: {:?}",
        memories.iter().map(|m| &m["scope"]).collect::<Vec<_>>()
    );
}

/// And the ordering half. `hoard_recall` promises results "ordered best first"; concatenation made
/// that false across sets, so a two-keyword match sorted below a one-keyword match that happened to
/// live in the store iterated first.
#[test]
fn keyword_recall_ranks_across_sets_rather_than_concatenating_them() {
    let mut store = Store::open_in_memory(Scope::Local).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    let mut server = a_server();
    server.add_store(store);

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"}, "content": "one keyword only",
            "keywords": ["routing"], "confidence": 0.8, "origin": "llm"
        }),
    );
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": "both keywords",
            "keywords": ["routing", "scope"], "confidence": 0.8, "origin": "llm"
        }),
    );

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["routing", "scope"], "limit": 20}),
    ));

    let memories = result["memories"].as_array().expect("a list");
    assert_eq!(memories.len(), 2, "both memories carry a requested keyword");
    assert_eq!(
        memories[0]["matched_keywords"],
        json!(2),
        "the better match should lead whichever set it came from: {memories:?}"
    );
}

// ---------------------------------------------------------------------------
// Direct reference: a whole id, or a fragment said out loud
// ---------------------------------------------------------------------------

/// `crockford-base32-ids`. The linkage the approval snapshot deliberately does not carry: a real,
/// freshly-minted id, truncated the way a model truncates one, is **refused** rather than resolved.
/// That is the whole of the fix — truncation still happens, it just stops silently succeeding.
#[test]
fn a_truncated_real_id_is_refused_and_points_at_the_recovery() {
    let mut server = a_server();
    let id = payload(&memorize(&mut server, "alpha beta"))["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let response = call(&mut server, "hoard_lookup", json!({"id": &id[..8]}));
    assert!(is_error(&response), "a shortened id must not resolve");

    let complaint = message(&response);
    assert!(
        complaint.contains("fragment"),
        "the refusal must name the action the caller can take: {complaint}"
    );
    assert!(
        complaint.contains("end"),
        "and say which part of an id is worth quoting: {complaint}"
    );
}

#[test]
fn a_whole_id_returns_its_memory() {
    let mut server = a_server();
    let id = payload(&memorize(&mut server, "alpha beta"))["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let result = payload(&call(&mut server, "hoard_lookup", json!({"id": id})));
    let memories = result["memories"].as_array().expect("a list");
    assert_eq!(memories.len(), 1, "a whole id names exactly one memory");
}

#[test]
fn a_fragment_from_the_end_of_an_id_finds_the_memory() {
    let mut server = a_server();
    let id = payload(&memorize(&mut server, "alpha beta"))["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let tail = &id[id.len() - 6..];
    let result = payload(&call(
        &mut server,
        "hoard_lookup",
        json!({"fragment": tail}),
    ));
    assert_eq!(
        result["memories"].as_array().expect("a list").len(),
        1,
        "a six-character tail identifies a memory outright"
    );
    assert!(
        result["note"].is_null(),
        "one match is not ambiguous, so nothing should be remarked on: {result}"
    );
}

/// The head is the timestamp, so a fragment taken from the front matches everything written nearby.
/// That case is not an error — it is the one where the response has to say what to do differently.
#[test]
fn an_ambiguous_fragment_says_which_end_of_an_id_to_quote() {
    let mut server = a_server();
    for content in ["alpha one", "alpha two", "alpha three"] {
        memorize(&mut server, content);
    }
    let id = payload(&memorize(&mut server, "alpha four"))["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let result = payload(&call(
        &mut server,
        "hoard_lookup",
        json!({"fragment": &id[..4]}),
    ));
    assert!(
        result["memories"].as_array().expect("a list").len() > 1,
        "a fragment from the head should match several: {result}"
    );
    let note = result["note"].as_str().expect("a note when ambiguous");
    assert!(
        note.contains("end"),
        "the note must name the remedy, not just the fact: {note}"
    );
}

/// **`scope` and `target_set` spell one set two ways, and a refusal where they meet has to say so.**
/// `the-set-name-refusal-does-not-say-how-the-other-field-spells-it`: sending the `scope` spelling
/// as a link's `target_set` was refused with *"set name must not contain ':'"* -- true, naming no
/// field, and saying nothing about what to send instead, so the whole call had to be resent on a
/// guess. The refusal is tested by **taking its remedy**: the spelling it recommends is resent and
/// must be accepted.
#[test]
fn a_target_set_spelled_as_a_scope_is_refused_with_the_bare_name() {
    let mut server = a_server();
    let target = "01M0G0890000000000000000AA";
    let attempt = |server: &mut Server, target_set: &str| {
        call(
            server,
            "hoard_memorize",
            json!({
                "scope": "shared:scatterhoard",
                "content": "alpha: a link into another set",
                "confidence": 0.9,
                "origin": "llm",
                "links": [{"label": "related_to", "target": target, "target_set": target_set}],
            }),
        )
    };

    let refused = attempt(&mut server, "shared:coding-lessons");
    assert!(
        is_error(&refused),
        "the scope spelling is refused: {refused}"
    );
    let text = message(&refused);
    for (needle, why) in [
        ("`target_set`", "names the field"),
        ("`coding-lessons`", "gives the spelling to send"),
        ("`scope`", "says which field the prefix belongs to"),
    ] {
        assert!(text.contains(needle), "the refusal {why}: {text}");
    }

    let accepted = attempt(&mut server, "coding-lessons");
    assert!(
        !is_error(&accepted),
        "the spelling the refusal recommends is accepted: {accepted}"
    );
}

/// Every other refusal of `target_set` still says which field it is about, since a set name is
/// validated the same way wherever it appears and the bare message cannot say where.
#[test]
fn any_refused_target_set_names_the_field() {
    let mut server = a_server();
    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:scatterhoard",
            "content": "alpha: a link into another set",
            "confidence": 0.9,
            "origin": "llm",
            "links": [{
                "label": "related_to",
                "target": "01M0G0890000000000000000AA",
                "target_set": "has space",
            }],
        }),
    );
    assert!(is_error(&refused));
    assert!(
        message(&refused).contains("`target_set`"),
        "the refusal names the field: {}",
        message(&refused)
    );
}

#[test]
fn lookup_refuses_an_id_and_a_fragment_together() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_lookup",
        json!({"id": "01M0G0890000000000000000AA", "fragment": "ABC"}),
    );
    assert!(is_error(&response));
    assert!(
        message(&response).contains("not both"),
        "the refusal should say the two are alternatives: {}",
        message(&response)
    );
}

#[test]
fn lookup_with_neither_an_id_nor_a_fragment_is_refused() {
    let mut server = a_server();
    let response = call(&mut server, "hoard_lookup", json!({}));
    assert!(is_error(&response));
    assert!(
        message(&response).contains("fragment"),
        "and should name both ways in: {}",
        message(&response)
    );
}

// ---------------------------------------------------------------------------
// The advertised schema against what the server actually accepts
// ---------------------------------------------------------------------------

/// A schema that over-constrains is a defect no per-layer test can see: the deserialiser is right,
/// the tool is right, and a client that validates arguments before sending them still cannot make
/// the call. `required` is a promise about what the server *rejects*, so it is checked here against
/// the server rejecting it, never against a literal list.
fn required_fields(server: &mut Server, tool: &str) -> Vec<String> {
    let response = send(
        server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );
    let tools = response["result"]["tools"].as_array().expect("a list");
    let definition = tools
        .iter()
        .find(|t| t["name"] == json!(tool))
        .unwrap_or_else(|| panic!("{tool} should be advertised"));
    definition["inputSchema"]["required"]
        .as_array()
        .map(|fields| {
            fields
                .iter()
                .map(|f| f.as_str().expect("a field name").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn recall_does_not_advertise_scope_as_required_because_it_is_not() {
    let mut server = a_server();
    memorize(&mut server, "alpha beta");

    let response = call(&mut server, "hoard_recall", json!({"query": "alpha"}));
    assert!(
        !is_error(&response),
        "recall without a scope searches every loaded set: {response}"
    );
    assert!(
        !required_fields(&mut server, "hoard_recall").contains(&"scope".to_owned()),
        "the schema forbids a call the server answers, so a validating client cannot search \
         every loaded set -- the asymmetry with hoard_memorize is deliberate and must be visible \
         to the caller"
    );
}

#[test]
fn recall_does_not_advertise_query_as_required_because_keywords_alone_work() {
    let mut server = a_server();
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha beta",
            "why": "because the test needs one",
            "confidence": 0.8,
            "origin": "llm",
            "keywords": ["alpha"]
        }),
    );

    let response = call(&mut server, "hoard_recall", json!({"keywords": ["alpha"]}));
    assert!(
        !is_error(&response),
        "keyword search is the path that works with no model at all: {response}"
    );
    assert!(
        !required_fields(&mut server, "hoard_recall").contains(&"query".to_owned()),
        "keyword-only recall is what degradation without an embedding model rests on; a schema \
         that requires a query hides it from exactly the client that needs it"
    );
}

/// The converse direction: what the schema *does* still require must genuinely be refused.
#[test]
fn every_field_memorize_advertises_as_required_is_actually_refused_when_missing() {
    let mut server = a_server();
    let complete = json!({
        "scope": {"shared": "scatterhoard"},
        "content": "alpha beta",
        "why": "because the test needs one",
        "confidence": 0.8,
        "origin": "llm"
    });

    for field in required_fields(&mut server, "hoard_memorize") {
        let mut arguments = complete.clone();
        arguments
            .as_object_mut()
            .expect("an object")
            .remove(&field)
            .unwrap_or_else(|| panic!("{field} is advertised as required but is not in the call"));

        assert!(
            is_error(&call(&mut server, "hoard_memorize", arguments)),
            "hoard_memorize advertises {field} as required, so omitting it must be refused \
             rather than guessed"
        );
    }
}

// ---------------------------------------------------------------------------
// The empty corpus
// ---------------------------------------------------------------------------

/// The first call anyone makes against a fresh install, and the one boundary where the corpus
/// centroid has nothing to average. Finding nothing is an answer, not a failure: a model that gets
/// an error here learns that memory is broken, when what is true is that memory is empty.
#[test]
fn recall_against_an_empty_corpus_answers_that_it_found_nothing() {
    let mut server = a_server();

    for arguments in [
        json!({"query": "anything at all"}),
        json!({"keywords": ["absent"]}),
    ] {
        let response = call(&mut server, "hoard_recall", arguments.clone());
        assert!(
            !is_error(&response),
            "an empty corpus is not an error: {arguments} gave {response}"
        );

        let result = payload(&response);
        assert!(result["memories"].as_array().expect("a list").is_empty());
        assert_eq!(
            result["semantic_search_available"],
            json!(true),
            "the model is loaded -- there is simply nothing stored, and conflating the two would \
             send a caller looking for a setup problem that does not exist"
        );
    }
}

/// One search per call, deliberately. A combined call ran the two paths independently and stapled
/// the results together, which bought the caller nothing it could not get from two calls and cost a
/// whole class of shape defects: the memory returned twice, two scales loose in one response, and
/// an ordering rule with no honest answer. Refusing the combination makes all three unrepresentable
/// rather than fixed. See `documents/design/tool-surface.md`.
#[test]
fn recall_refuses_a_query_and_keywords_in_the_same_call() {
    let mut server = a_server();
    memorize_full(&mut server, "alpha one", json!(["alpha-key"]), json!([]));

    let response = call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "keywords": ["alpha-key"]}),
    );

    assert!(
        is_error(&response),
        "a query and keywords are two different searches on two different scales; answering both \
         in one response is what produced the duplication defect: {response}"
    );
    // A tool-level refusal travels as `isError` content, not a JSON-RPC error, so the text the
    // model actually reads is in the content block.
    let message = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        message.contains("query") && message.contains("keywords"),
        "the refusal has to name both arguments so the caller knows which to drop: {message}"
    );
}

/// The keyword path used to hard-code this to `true`, because the field was computed only on the
/// semantic path and keyword-only recall had no value to report. A caller on a machine with no
/// model was told semantic search was available by the one call that still worked.
#[test]
fn keyword_recall_reports_semantic_availability_truthfully() {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(chartered(scope()));
    memorize_full(&mut server, "alpha one", json!(["alpha-key"]), json!([]));

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["alpha-key"]}),
    ));

    assert_eq!(
        result["memories"].as_array().expect("a list").len(),
        1,
        "keyword search is the path that works without a model: {result}"
    );
    assert_eq!(
        result["semantic_search_available"],
        json!(false),
        "no model is loaded; saying otherwise on the one path that still works is the most \
         misleading place to say it: {result}"
    );
}

#[test]
fn lookup_against_an_empty_corpus_answers_that_it_found_nothing() {
    let mut server = a_server();

    // A whole id that names nothing, rather than a shortened one — since `crockford-base32-ids`
    // a partial id is a refusal, and this test is about an empty corpus, not about that.
    let response = call(
        &mut server,
        "hoard_lookup",
        json!({"id": "00000000000000000000000000"}),
    );
    assert!(!is_error(&response), "not an error: {response}");
    assert!(
        payload(&response)["memories"]
            .as_array()
            .expect("a list")
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// What a returned memory carries
// ---------------------------------------------------------------------------
//
// These assert the *whole* field set rather than picking out fields one at a time. An assertion
// that checks the fields someone thought of cannot detect the field nobody thought of, which is
// exactly how `keywords`, `links`, `origin`, `created` and `modified` went missing from every tool
// on the surface without a single test going red. See the "single source of truth" and
// "assertions that can only find what was anticipated" entries in TRACKING.md.

/// The keys of a JSON object, sorted, so a field set can be compared as a whole.
fn keys(value: &Value) -> Vec<String> {
    let mut found: Vec<String> = value
        .as_object()
        .unwrap_or_else(|| panic!("expected an object, got {value}"))
        .keys()
        .cloned()
        .collect();
    found.sort();
    found
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut owned: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    owned.sort();
    owned
}

/// Store a memory with every optional field populated, so an omission anywhere shows up.
/// The same, into whichever set the caller names. Two sets are what a cross-set concession needs.
fn memorize_full_in(server: &mut Server, scope: Value, content: &str) -> String {
    let response = call(
        server,
        "hoard_memorize",
        json!({
            "scope": scope,
            "content": content,
            "why": "the reasoning behind it",
            "confidence": 0.8,
            "origin": "joint"
        }),
    );
    assert!(!is_error(&response), "memorize failed: {response}");
    payload(&response)["id"].as_str().expect("an id").to_owned()
}

fn memorize_full(server: &mut Server, content: &str, keywords: Value, links: Value) -> String {
    let response = call(
        server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": content,
            "why": "the reasoning behind it",
            "context": "raised while writing this test",
            "keywords": keywords,
            "links": links,
            "confidence": 0.8,
            "origin": "joint"
        }),
    );
    assert!(!is_error(&response), "memorize failed: {response}");
    payload(&response)["id"].as_str().expect("an id").to_owned()
}

/// Every field the memory carries has to reach the caller. A field no tool can show is a field
/// whose corruption cannot be detected -- which is the reason this is a correctness requirement
/// and not a convenience one.
#[test]
fn recall_returns_every_field_of_the_memory() {
    let mut server = a_server();
    let first = memorize_full(&mut server, "alpha one", json!(["alpha-key"]), json!([]));
    memorize_full(
        &mut server,
        "alpha two",
        json!(["alpha-key", "second-key"]),
        json!([{"label": "related_to", "target": first}]),
    );

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 5}),
    ));
    let hit = result["memories"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|hit| hit["content"] == json!("alpha two"))
        .expect("the linked memory should be recalled")
        .clone();

    assert_eq!(
        keys(&hit),
        sorted(&[
            "id",
            "content",
            "why",
            "context",
            "keywords",
            "confidence",
            "origin",
            "created",
            "modified",
            "links",
            "scope",
            "similarity"
        ]),
        "recall dropped part of the memory: {hit}"
    );
    assert_eq!(hit["keywords"], json!(["alpha-key", "second-key"]));
    assert_eq!(hit["links"][0]["label"], json!("related_to"));
    assert_eq!(hit["links"][0]["target"], json!(first));
    assert_eq!(hit["origin"], json!("joint"));
    assert!(hit["created"]["by"].is_string());
}

/// The keyword path is the one that still works with no embedding model, so it is the path a
/// first-time user is most likely to be on. It must not be the impoverished one.
#[test]
fn keyword_hits_carry_the_same_memory_fields_as_semantic_hits() {
    let mut server = a_server();
    memorize_full(&mut server, "alpha one", json!(["alpha-key"]), json!([]));

    // Two calls, because one call cannot ask both any more. The point of the test is unchanged:
    // whichever path found it, the memory arrives whole.
    let semantic = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ))["memories"][0]
        .clone();
    let keyword = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["alpha-key"]}),
    ))["memories"][0]
        .clone();

    let memory_fields = |hit: &Value| {
        let mut names = keys(hit);
        names.retain(|name| !matches!(name.as_str(), "similarity" | "matched_keywords"));
        names
    };

    assert_eq!(
        memory_fields(&keyword),
        memory_fields(&semantic),
        "a keyword hit describes the same memory as a semantic one and must carry the same \
         fields; the keyword hit was {keyword}"
    );
    assert_eq!(keyword["why"], json!("the reasoning behind it"));
}

/// Two tools returning the same kind of thing in two shapes forces every caller to special-case
/// which tool produced it.
#[test]
fn lookup_returns_the_same_memory_fields_as_recall() {
    let mut server = a_server();
    let id = memorize_full(&mut server, "alpha one", json!(["alpha-key"]), json!([]));

    let recalled = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ))["memories"][0]
        .clone();
    let found =
        payload(&call(&mut server, "hoard_lookup", json!({"id": id})))["memories"][0].clone();

    let memory_fields = |hit: &Value| {
        let mut names = keys(hit);
        names.retain(|name| !matches!(name.as_str(), "similarity" | "superseded_by"));
        names
    };
    assert_eq!(
        memory_fields(&found),
        memory_fields(&recalled),
        "lookup and recall disagree about the shape of a memory; lookup was {found}"
    );
}

/// The corpus-integrity case. Superseding must not quietly drop the keywords or links of either
/// endpoint -- and the only way to know it hasn't is to be able to see them through the tools,
/// rather than by reading the database directly.
#[test]
fn supersession_preserves_the_keywords_and_links_of_both_endpoints() {
    let mut server = a_server();
    let anchor = memorize_full(&mut server, "beta anchor", json!(["anchor-key"]), json!([]));
    let original = memorize_full(
        &mut server,
        "alpha original",
        json!(["original-key", "shared-key"]),
        json!([{"label": "related_to", "target": anchor}]),
    );
    let replacement = memorize_full(
        &mut server,
        "alpha replacement",
        json!(["replacement-key", "shared-key"]),
        json!([{"label": "supersedes", "target": original}]),
    );

    let old =
        payload(&call(&mut server, "hoard_lookup", json!({"id": original})))["memories"][0].clone();
    assert_eq!(
        old["keywords"],
        json!(["original-key", "shared-key"]),
        "superseding a memory must not touch its keywords: {old}"
    );
    assert_eq!(
        old["links"][0]["target"],
        json!(anchor),
        "superseding a memory must not touch its links: {old}"
    );
    assert_eq!(old["superseded_by"], json!([replacement]));

    let new = payload(&call(
        &mut server,
        "hoard_lookup",
        json!({"id": replacement}),
    ))["memories"][0]
        .clone();
    assert_eq!(new["keywords"], json!(["replacement-key", "shared-key"]));
    assert_eq!(new["links"][0]["label"], json!("supersedes"));
    assert_eq!(new["links"][0]["target"], json!(original));
}

/// Superseding without stating a keyword set is refused, because neither default is safe.
///
/// Carrying the superseded memory's keywords over silently is one failure; letting the replacement
/// arrive with none is the other, and it is the one measured in a different memory system, where
/// superseding silently dropped the record's keys. `a-keyword-cannot-be-retired-once-a-memory-has-
/// been-exported` in TRACKING.md. The author is the only party who can say which terms the new memory
/// is indexed under, so the call has to say -- and absence is the one answer that cannot be
/// honoured.
///
/// Measured on the live sets before this shipped: three of three sampled supersession pairs had
/// byte-identical keyword sets, re-typed by hand each time. That is the operation this guards.
#[test]
fn superseding_without_stating_keywords_is_refused() {
    let mut server = a_server();
    let original = memorize_full(
        &mut server,
        "alpha original",
        json!(["original-key"]),
        json!([]),
    );

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha replacement",
            "why": "the reasoning behind it",
            "links": [{"label": "supersedes", "target": original}],
            "confidence": 0.8,
            "origin": "joint"
        }),
    );

    assert!(is_error(&response), "{response}");
    let message = response["result"]["content"][0]["text"]
        .as_str()
        .expect("a message");
    assert!(
        message.contains("keywords"),
        "the refusal names the parameter that has to be stated: {message}"
    );
}

/// An explicitly empty keyword list is a decision, and is stored.
///
/// The pair to the test above, and the reason the refusal keys on **absence** rather than on
/// emptiness. A replacement genuinely indexed under nothing is a legal thing to write; a call that
/// forgot to say is not. Only distinguishing the two makes the guard honest rather than a blanket
/// requirement -- and `swallowed_parameters` draws the same distinction for the same reason.
#[test]
fn superseding_with_an_explicitly_empty_keyword_list_is_accepted() {
    let mut server = a_server();
    let original = memorize_full(
        &mut server,
        "alpha original",
        json!(["original-key"]),
        json!([]),
    );

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha replacement",
            "why": "the reasoning behind it",
            "keywords": [],
            "links": [{"label": "supersedes", "target": original}],
            "confidence": 0.8,
            "origin": "joint"
        }),
    );

    assert!(!is_error(&response), "{response}");
    let replacement = payload(&response)["id"].as_str().expect("an id").to_owned();
    let stored = payload(&call(
        &mut server,
        "hoard_lookup",
        json!({"id": replacement}),
    ))["memories"][0]
        .clone();
    assert!(
        stored["keywords"].as_array().is_none_or(Vec::is_empty),
        "an explicitly empty list is stored as empty, not filled in from the superseded memory: \
         {stored}"
    );
}

/// Superseding reports what it did to reachability, while the caller can still act on it.
///
/// The third part of `a-keyword-cannot-be-retired-once-a-memory-has-been-exported`. Stating the
/// keyword set is now required, but a caller can still state a *wrong* one — most likely by
/// re-typing the old set and losing a term, which is the operation the live corpus shows being
/// performed by hand at every supersession. The delta is the cheapest possible check on that: the
/// call has just happened, the author still has the reasoning in hand, and `dropped` names exactly
/// the terms that stopped reaching anything.
///
/// **Reported, not refused.** Dropping a keyword is a legitimate act — it is the *only* way to
/// retire one — so the server states the fact and the caller judges, which is the criterion this
/// codebase applies: remark when the caller can still act, refuse when they cannot.
#[test]
fn superseding_reports_the_keyword_delta() {
    let mut server = a_server();
    let original = memorize_full(
        &mut server,
        "alpha original",
        json!(["kept-key", "dropped-key"]),
        json!([]),
    );

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha replacement",
            "why": "the reasoning behind it",
            "keywords": ["kept-key", "added-key"],
            "links": [{"label": "supersedes", "target": original}],
            "confidence": 0.8,
            "origin": "joint"
        }),
    );

    assert!(!is_error(&response), "{response}");
    let result = payload(&response);
    let delta = &result["keyword_delta"][0];

    assert_eq!(delta["superseded"], json!(original), "{result}");
    assert_eq!(delta["dropped"], json!(["dropped-key"]), "{result}");
    assert_eq!(delta["added"], json!(["added-key"]), "{result}");
    assert_eq!(delta["retained"], json!(["kept-key"]), "{result}");
    assert!(
        result["keyword_delta_note"].is_string(),
        "a delta the caller has to judge needs saying what to judge about it: {result}"
    );
}

/// No supersession, no delta — the field is absent rather than empty.
///
/// The paired must-not-fire. An empty delta on every ordinary store would be a standing cost paid
/// by the calls that have nothing to report, which is the shape `keywords` and `links` already
/// avoid by appearing only when non-empty.
#[test]
fn an_ordinary_memory_reports_no_keyword_delta() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha standing alone",
            "why": "the reasoning behind it",
            "keywords": ["a-key"],
            "confidence": 0.8,
            "origin": "joint"
        }),
    );
    assert!(!is_error(&response), "{response}");
    let result = payload(&response);
    assert!(
        result.get("keyword_delta").is_none(),
        "nothing was superseded, so there is no delta to report: {result}"
    );
}

/// A supersession that changes nothing still reports, and the report is what says so.
///
/// The case that matters most for the guard's honesty: re-typing the old set correctly is the
/// commonest outcome, and a caller who sees `dropped: []` learns their re-typing was faithful.
/// Suppressing the delta when nothing was dropped would remove exactly the confirmation.
#[test]
fn a_supersession_that_drops_nothing_still_reports_the_delta() {
    let mut server = a_server();
    let original = memorize_full(
        &mut server,
        "alpha original",
        json!(["same-key"]),
        json!([]),
    );

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha replacement",
            "why": "the reasoning behind it",
            "keywords": ["same-key"],
            "links": [{"label": "supersedes", "target": original}],
            "confidence": 0.8,
            "origin": "joint"
        }),
    );

    assert!(!is_error(&response), "{response}");
    let result = payload(&response);
    let delta = &result["keyword_delta"][0];
    assert_eq!(delta["dropped"], json!([]), "{result}");
    assert_eq!(delta["retained"], json!(["same-key"]), "{result}");
}

/// A supersession names the links it leaves behind, as it names the keywords it drops.
///
/// Step 1 of `out-links-stay-with-the-record-that-wrote-them`. An out-link is the claim of the
/// record that wrote it, so a replacement that does not restate one leaves it with the superseded
/// memory -- which is how a link that no longer holds gets dropped, and also what an incomplete
/// re-typing looks like. The report comes first so that cutting inheritance later removes nothing
/// an author did not see go.
///
/// **A changed label is a link left behind**, not a link kept: `related_to` and `supports` are
/// different claims about the same target, and restating one does not restate the other.
#[test]
fn superseding_reports_the_links_left_behind() {
    let mut server = a_server();
    let kept = memorize_full(&mut server, "kept target", json!([]), json!([]));
    let dropped = memorize_full(&mut server, "dropped target", json!([]), json!([]));
    let original = memorize_full(
        &mut server,
        "alpha original",
        json!(["a-key"]),
        json!([
            {"label": "related_to", "target": kept},
            {"label": "results_from", "target": dropped},
        ]),
    );

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha replacement",
            "why": "the reasoning behind it",
            "keywords": ["a-key"],
            "links": [
                {"label": "supersedes", "target": original},
                {"label": "related_to", "target": kept},
                {"label": "supports", "target": dropped},
            ],
            "confidence": 0.8,
            "origin": "joint"
        }),
    );

    assert!(!is_error(&response), "{response}");
    let result = payload(&response);
    let delta = &result["link_delta"][0];
    assert_eq!(delta["superseded"], json!(original), "{result}");
    assert_eq!(
        delta["left_behind"],
        json!([{"label": "results_from", "target": dropped}]),
        "{result}"
    );
    assert_eq!(
        delta["restated"],
        json!([{"label": "related_to", "target": kept}]),
        "{result}"
    );
    assert!(
        result["link_delta_note"].is_string(),
        "a link left behind needs saying what to judge about it: {result}"
    );
}

/// The superseded memory's own supersession links are its history, not claims to restate.
///
/// A replacement never restates `supersedes` on the memory it replaces -- the chain is recorded by
/// the new link, not by copying the old one -- so listing it would put a line in every report of a
/// chain three long that the caller can do nothing about. And a delta that restates everything
/// still reports, empty: that is the confirmation a faithful re-typing earns, and it is what tells
/// *nothing was left behind* apart from *nothing was checked*.
#[test]
fn a_supersession_restating_every_claim_leaves_nothing_behind() {
    let mut server = a_server();
    let target = memorize_full(&mut server, "a target", json!([]), json!([]));
    let first = memorize_full(&mut server, "alpha first", json!(["a-key"]), json!([]));
    let second = memorize_full(
        &mut server,
        "alpha second",
        json!(["a-key"]),
        json!([
            {"label": "supersedes", "target": first},
            {"label": "related_to", "target": target},
        ]),
    );

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha third",
            "why": "the reasoning behind it",
            "keywords": ["a-key"],
            "links": [
                {"label": "supersedes", "target": second},
                {"label": "related_to", "target": target},
            ],
            "confidence": 0.8,
            "origin": "joint"
        }),
    );

    assert!(!is_error(&response), "{response}");
    let result = payload(&response);
    let delta = &result["link_delta"][0];
    assert_eq!(delta["left_behind"], json!([]), "{result}");
    assert_eq!(
        delta["restated"],
        json!([{"label": "related_to", "target": target}]),
        "{result}"
    );
}

/// A cross-set link restated is restated, however its target was spelled.
///
/// Compared on label and target id rather than on the whole target, because ids are unique across
/// sets and the rest of a cross-set target is apparatus: `target_keywords` exist to re-find a
/// dangling reference, and a caller re-typing a link has no reason to reproduce them exactly.
#[test]
fn a_cross_set_link_restated_with_other_keywords_is_not_left_behind() {
    let mut server = a_server();
    let elsewhere = "01M1D3K649FX8V09X2KE5W8FAE";
    let original = memorize_full(
        &mut server,
        "alpha original",
        json!(["a-key"]),
        json!([{"label": "instance_of", "target": elsewhere,
                "target_set": "coding-lessons", "target_keywords": ["one-spelling"]}]),
    );

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha replacement",
            "why": "the reasoning behind it",
            "keywords": ["a-key"],
            "links": [
                {"label": "supersedes", "target": original},
                {"label": "instance_of", "target": elsewhere,
                 "target_set": "coding-lessons", "target_keywords": ["another-spelling"]},
            ],
            "confidence": 0.8,
            "origin": "joint"
        }),
    );

    assert!(!is_error(&response), "{response}");
    let result = payload(&response);
    assert_eq!(
        result["link_delta"][0]["left_behind"],
        json!([]),
        "{result}"
    );
}

/// No supersession, no link delta -- the paired must-not-fire.
#[test]
fn an_ordinary_memory_reports_no_link_delta() {
    let mut server = a_server();
    let target = memorize_full(&mut server, "a target", json!([]), json!([]));
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha standing alone",
            "why": "the reasoning behind it",
            "links": [{"label": "related_to", "target": target}],
            "confidence": 0.8,
            "origin": "joint"
        }),
    );
    assert!(!is_error(&response), "{response}");
    let result = payload(&response);
    assert!(
        result.get("link_delta").is_none(),
        "nothing was superseded, so there is no delta to report: {result}"
    );
}

/// The guard does not over-fire: an ordinary memory still needs no keywords.
///
/// Requiring them everywhere would be a different decision, and a larger one -- `keywords` is
/// optional throughout the surface, and `decision-placement.md` governs adding a required choice.
/// What is required here is narrow: a caller writing a `supersedes` link is deciding what the
/// replacement is indexed under, whether or not they realise it, so that call alone must say.
#[test]
fn storing_without_a_supersedes_link_does_not_require_keywords() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha standing alone",
            "why": "the reasoning behind it",
            "confidence": 0.8,
            "origin": "joint"
        }),
    );
    assert!(!is_error(&response), "{response}");
}

/// A keyword a replacement dropped stops matching. **This reverses what the suite asserted until
/// 2026-09-02**, and the reversal is the point.
///
/// The old test — `a_keyword_hit_reached_through_supersession_says_so` — pinned the substitution and
/// checked only that the payload *admitted* to it. That was the right fix for
/// `keyword-hit-swaps-silently`, which was about a caller being unable to tell. It left the harder
/// question untouched, and the entry said so: whether the keyword should still match at all was
/// inherited from the content rule rather than argued.
///
/// It should not. [Recall Mechanisms](documents/design/recall-mechanisms.md) justifies resolving to
/// the tip on the ground that a superseded memory's *substance* is actively misleading, which is
/// true of content and says nothing about an index term. A keyword is an author's addressing
/// decision, and dropping one on the replacement is that author saying *this is no longer about
/// that*. Direct id lookup is already exempt from substitution for the same reason. So: explicit
/// addressing is honoured as written, inferred matching resolves to the tip.
///
/// `a-keyword-cannot-be-retired-once-a-memory-has-been-exported` in TRACKING.md.
#[test]
fn a_keyword_dropped_by_a_replacement_stops_matching() {
    let mut server = a_server();
    let original = memorize_full(&mut server, "alpha original", json!(["old-key"]), json!([]));
    memorize_full(
        &mut server,
        "alpha replacement",
        json!(["new-key"]),
        json!([{"label": "supersedes", "target": original}]),
    );

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["old-key"]}),
    ));
    let hits = result["memories"].as_array().expect("a list");

    assert!(
        hits.is_empty(),
        "no live memory carries `old-key`, so nothing should match it: {result}"
    );
}

/// The paired case, and the reason the change is a narrowing rather than a loss: a term the
/// replacement **kept** still finds it, directly.
///
/// This is what makes the superseded row redundant rather than suppressed. If the tip carries the
/// term, the tip's own row matches it, so nothing has to be reached through supersession to find
/// it — which is why the fix skips the superseded row instead of filtering after resolving it.
#[test]
fn a_keyword_kept_by_a_replacement_matches_it_directly() {
    let mut server = a_server();
    let original = memorize_full(
        &mut server,
        "alpha original",
        json!(["old-key", "shared-key"]),
        json!([]),
    );
    let replacement = memorize_full(
        &mut server,
        "alpha replacement",
        json!(["new-key", "shared-key"]),
        json!([{"label": "supersedes", "target": original}]),
    );

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["shared-key"]}),
    ));
    let hits = result["memories"].as_array().expect("a list");

    assert_eq!(hits.len(), 1, "expected the replacement alone: {result}");
    assert_eq!(hits[0]["id"], json!(replacement));
    assert!(
        hits[0]["replaces_matched"].is_null(),
        "the replacement carries the term itself, so this is a direct hit and not a substitution: \
         {result}"
    );
}

/// A keyword hit **can** still be a substitution, across sets, and the payload must say so.
///
/// Same-set substitution left the keyword path on 2026-09-02, which took the only covered producer
/// of `replaces_matched` on this path with it — the recorded mutation `view: stop reporting the
/// substitution behind a keyword hit` began surviving, silently, because nothing exercised the arm
/// any more. That is the *adding a layer un-tests a guard* shape, so the guard is re-covered here
/// rather than the mutation being explained away.
///
/// The producer that remains is a memory conceding to a replacement in **another loaded set**.
/// `Store::search_keywords` never substitutes, but `search_keywords_across` honours the concession,
/// because a concession is the memory's own statement about its standing rather than an index term
/// being reached through.
#[test]
fn a_cross_set_concession_still_reports_the_keyword_substitution() {
    let mut server = a_server_with_two_sets();
    let replacement = memorize_full_in(
        &mut server,
        json!("shared:coding-lessons"),
        "the replacement, in the other set",
    );
    let conceding = memorize_full(
        &mut server,
        "conceding to a memory in the other set",
        json!(["concession-key"]),
        json!([{
            "label": "superseded_by",
            "target": replacement,
            "target_set": "coding-lessons"
        }]),
    );

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["concession-key"]}),
    ));
    let hits = result["memories"].as_array().expect("a list");

    assert_eq!(hits.len(), 1, "expected the replacement: {result}");
    assert_eq!(hits[0]["id"], json!(replacement));
    assert_eq!(
        hits[0]["replaces_matched"],
        json!(conceding),
        "the returned memory is in another set and does not carry the keyword, so the payload has \
         to say a substitution happened: {result}"
    );
}

// ---------------------------------------------------------------------------
// What the server publishes about itself
// ---------------------------------------------------------------------------
//
// The published schema is a second description of what the tools accept, and the deserialiser is
// the first. These tests compare it against that source rather than against a literal restatement
// -- a literal would be a third copy, and the failure this class produces is precisely two copies
// agreeing with themselves while disagreeing with each other. See STANDARDS.md, "single source of
// truth".

fn tool_schema(server: &mut Server, name: &str) -> Value {
    let response = send(
        server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );
    response["result"]["tools"]
        .as_array()
        .expect("a list of tools")
        .iter()
        .find(|tool| tool["name"] == json!(name))
        .unwrap_or_else(|| panic!("no tool named {name}"))["inputSchema"]
        .clone()
}

/// The names a serde enum actually accepts, taken from the enum itself.
fn accepted_names<T: serde::Serialize>(all: &[T]) -> Vec<String> {
    all.iter()
        .map(|variant| {
            serde_json::to_value(variant)
                .expect("a unit variant serialises")
                .as_str()
                .expect("a unit variant is a string")
                .to_owned()
        })
        .collect()
}

/// A property that says only what it means, and not what shape it is, cannot be constructed from
/// the schema. This is not hypothetical: the first session to drive these tools sent `scope` as a
/// string because nothing said it was an object, and got `unknown variant` back.
#[test]
fn every_advertised_property_says_what_shape_it_is() {
    let mut server = a_server();
    let describes_a_shape = ["type", "enum", "oneOf", "anyOf", "const"];

    for tool in [
        "hoard_memorize",
        "hoard_recall",
        "hoard_lookup",
        "hoard_conventions",
    ] {
        let schema = tool_schema(&mut server, tool);
        let properties = schema["properties"].as_object().expect("properties");
        for (name, property) in properties {
            assert!(
                describes_a_shape
                    .iter()
                    .any(|key| property.get(*key).is_some()),
                "{tool}.{name} advertises no shape, so a client has to guess it: {property}"
            );
        }
    }
}

/// The vocabulary is `LinkLabel::ALL`. A hand-written copy in the schema is how the two fall out
/// of step -- and a label the server accepts but never mentions is unreachable in practice.
///
/// Named in the description rather than as an `enum` since 2026-09-28, so that a label from a newer
/// build can be restated; `tests/unknown_labels.rs` holds why. Asserted as the exact list, in
/// order, so a label the prose omits or invents fails here.
#[test]
fn the_published_link_labels_are_exactly_the_ones_the_type_defines() {
    let mut server = a_server();
    let schema = tool_schema(&mut server, "hoard_memorize");

    assert_eq!(
        published_labels(&schema),
        accepted_names(&LinkLabel::ALL),
        "the published label list disagrees with LinkLabel::ALL"
    );
}

/// The labels a link's published description names, in order: every backticked term in it.
fn published_labels(schema: &Value) -> Vec<String> {
    schema["properties"]["links"]["items"]["properties"]["label"]["description"]
        .as_str()
        .expect("the label is described")
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_published_origins_are_exactly_the_ones_the_type_defines() {
    let mut server = a_server();
    let schema = tool_schema(&mut server, "hoard_memorize");

    assert_eq!(
        schema["properties"]["origin"]["enum"],
        json!(accepted_names(&Origin::ALL)),
        "the published origin list disagrees with Origin::ALL"
    );
}

/// Publishing a shape is only worth anything if the server accepts what it publishes.
#[test]
fn every_published_link_label_is_actually_accepted() {
    let mut server = a_server();
    let target = memorize_full(&mut server, "alpha target", json!([]), json!([]));
    let schema = tool_schema(&mut server, "hoard_memorize");
    let labels = published_labels(&schema);

    assert!(!labels.is_empty(), "no labels published at all");
    for label in labels {
        let response = call(
            &mut server,
            "hoard_memorize",
            json!({
                "scope": {"shared": "scatterhoard"},
                "content": "alpha linked",
                "confidence": 0.5,
                "origin": "llm",
                // The loop covers `supersedes` too, which requires a stated keyword set. Without
                // this the test would report the schema publishing a label the server refuses,
                // when what it actually refuses is the missing keywords.
                "keywords": [],
                "links": [{"label": label, "target": target}]
            }),
        );
        assert!(
            !is_error(&response),
            "the schema publishes label {label} but the server refuses it: {response}"
        );
    }
}

/// **Every spelling of a scope a client might send is accepted, whichever one is published.**
///
/// This asserted that `scope` published two `oneOf` alternatives and that both worked. It publishes
/// one scalar now — a nested alternation is reduced to `{}` by at least one client, and an object
/// argument then arrives as text — so the published half of the assertion is gone.
///
/// The accepting half matters more than it did. `tool-surface.md` settles that this server should be
/// liberal in what it takes precisely because it has no influence over its clients: the object form
/// is what memory files hold and what a client preserving alternations will keep sending, so
/// refusing it would strand memories rather than correct anyone. That makes this the test standing
/// between the two spellings and silent divergence.
#[test]
fn every_scope_spelling_is_accepted_however_it_is_published() {
    let mut server = a_server();
    let mut local = Store::open_in_memory(Scope::Local).expect("opens");
    local.set_embedder(Arc::new(MarkerEmbedder));
    server.add_store(local);

    let schema = tool_schema(&mut server, "hoard_memorize");
    assert_eq!(
        schema["properties"]["scope"]["type"],
        json!("string"),
        "scope must publish as a scalar, or an object argument arrives as text: {schema}"
    );

    for scope in [
        json!("local"),
        json!("shared:scatterhoard"),
        json!("shared:scatterhoard"),
    ] {
        let response = call(
            &mut server,
            "hoard_memorize",
            json!({
                "scope": scope,
                "content": "alpha scoped",
                "confidence": 0.5,
                "origin": "llm"
            }),
        );
        assert!(
            !is_error(&response),
            "scope {scope} is a spelling a client can produce and was refused: {response}"
        );
    }
}

/// On the substitution path the score belongs to the memory that *matched*, not the one being
/// returned -- so calling it `similarity` invites a caller to read it as describing the content
/// sitting beside it. A number has to name what it is about.
#[test]
fn a_substituted_hit_names_whose_score_it_reports() {
    let mut server = a_server();
    let original = memorize_full(&mut server, "alpha alpha alpha", json!([]), json!([]));
    memorize_full(
        &mut server,
        "beta beta beta",
        json!([]),
        json!([{"label": "supersedes", "target": original}]),
    );

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ));
    let hits = result["memories"].as_array().expect("a list");
    let substituted = hits
        .iter()
        .find(|hit| hit.get("replaces_matched").is_some())
        .unwrap_or_else(|| panic!("expected a substituted hit, got {result}"));

    assert!(
        substituted.get("similarity").is_none(),
        "this score is the superseded memory's, so it must not be presented as this memory's: \
         {substituted}"
    );
    assert!(
        substituted["matched_similarity"].is_number(),
        "a substituted hit should report the matched memory's score under a name that says so: \
         {substituted}"
    );
}

/// The ordinary case must stay ordinary: a hit that matched on its own merits reports its own
/// score, under the plain name.
#[test]
fn a_direct_hit_reports_its_own_similarity() {
    let mut server = a_server();
    memorize_full(&mut server, "alpha alpha", json!([]), json!([]));

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ));
    let hit = result["memories"][0].clone();

    assert!(
        hit["similarity"].is_number(),
        "expected a plain score: {hit}"
    );
    assert!(hit.get("matched_similarity").is_none());
    assert!(hit.get("replaces_matched").is_none());
}

/// **No tool may publish `oneOf`, `allOf` or `anyOf` at the top level of its input schema.**
///
/// Not a style rule. The Anthropic API rejects it outright:
///
/// ```text
/// 400 tools.N.custom.input_schema: input_schema does not support oneOf, allOf, or anyOf
///     at the top level
/// ```
///
/// **The blast radius is not this server.** A client that forwards MCP schemas to the API
/// unmodified sends the whole tool list on every request, so one offending schema fails *every*
/// tool from *every* registered server, on every call, until the session is restarted. Two public
/// reports of exactly that: anthropics/claude-code#45106 (a calendar connector) and
/// browser-use/browser-use#4211 (`browser_click`). In both, one server broke everything.
///
/// **Having `properties` as well does not help** — the calendar case published `anyOf` *and*
/// `properties` and was still refused. The keyword's presence at the top level is what is rejected,
/// so the branches cannot be kept alongside the flat view as a courtesy to consumers that read
/// them. They have to be gone.
///
/// Nested alternation is a separate matter and is not covered here: `scope` and `links.target` keep
/// their `oneOf`, the API accepts them, and the restriction is explicitly *at the top level*.
///
/// This existed for half a day between `oneof-tool-schemas-present-no-parameters` being fixed and
/// this test being written, because that fix added the flat view without removing what it was a
/// fallback for.
#[test]
fn no_tool_publishes_top_level_alternation() {
    let mut server = a_server();
    let response = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );

    for tool in response["result"]["tools"].as_array().expect("a list") {
        let name = tool["name"].as_str().expect("a name");
        let schema = &tool["inputSchema"];
        for keyword in ["oneOf", "allOf", "anyOf"] {
            assert!(
                schema.get(keyword).is_none(),
                "{name} publishes `{keyword}` at the top level of its input schema. The Anthropic \
                 API refuses the whole tool list for this, breaking every server in the session: \
                 {schema}"
            );
        }
    }
}

/// **The whole published surface stays inside the subset every client carried intact.**
///
/// `tool-surface.md`, *publish the centre of MCP, accept the edges*. `no_tool_publishes_top_level_alternation`
/// above covers the one case the API refuses outright; this covers the wider rule, at any depth,
/// and it exists because the top-level fix was written while three nested alternations sat
/// untouched a few lines away.
///
/// Each construct here was measured, not guessed. Two clients were given the same schema from the
/// same binary: `type`/`properties`/`required`/`items`/`enum` came through both intact, while every
/// construct below arrived from one of them as `{}` — and a property with no type carries an object
/// argument as *text*, which is how `{"shared": "x"}` became `unknown variant '{"shared":"x"}'` and
/// made shared sets unreachable.
///
/// Accepting these forms is a separate matter and deliberately wider: the parsers still take the
/// object spelling of a scope and the nested spelling of a link target. This is about what is
/// *published*.
#[test]
fn nothing_published_leaves_the_well_carried_subset() {
    // `const` and `title` are not themselves poorly carried; they only ever appeared inside the
    // branches, so their presence means an alternation has come back.
    const EDGE: [&str; 7] = [
        "oneOf",
        "anyOf",
        "allOf",
        "not",
        "additionalProperties",
        "const",
        "title",
    ];

    fn walk(node: &Value, path: &str, found: &mut Vec<String>) {
        match node {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if EDGE.contains(&key.as_str()) {
                        found.push(format!("{path}.{key}"));
                    }
                    if key == "type" && value.is_array() {
                        found.push(format!("{path}.type is a union: {value}"));
                    }
                    walk(value, &format!("{path}.{key}"), found);
                }
            }
            Value::Array(items) => {
                for (index, value) in items.iter().enumerate() {
                    walk(value, &format!("{path}[{index}]"), found);
                }
            }
            _ => {}
        }
    }

    let mut server = a_server();
    let response = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );

    let mut found = Vec::new();
    for tool in response["result"]["tools"].as_array().expect("a list") {
        let name = tool["name"].as_str().expect("a name");
        walk(&tool["inputSchema"], name, &mut found);
    }

    assert!(
        found.is_empty(),
        "these are published but are not reliably carried to a client, and a property that arrives \
         as `{{}}` takes an object argument as text: {found:#?}"
    );
}

// ---------------------------------------------------------------------------
// Transmission damage
// ---------------------------------------------------------------------------

/// The whole point of the check is that it is silent when nothing is wrong. A complaint on every
/// successful write would be noise, and noise is what makes a warning stop being read.
#[test]
fn an_undamaged_memory_says_nothing_about_markup() {
    let mut server = a_server();
    let result = payload(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "nothing wrong here",
            "why": "because the call was well formed",
            "confidence": 0.8,
            "origin": "llm"
        }),
    ));

    assert!(result.get("suspect_markup").is_none(), "{result}");
    assert!(result["id"].is_string(), "{result}");
}

/// **A memory past the length limit is refused, and the refusal says to split it.**
///
/// Decided 2026-09-19 by the human partner. The limit is not a property of the model — the encoding
/// stopped truncating the same day — but of what a memory *is*: a record long enough to need
/// splitting is usually several claims, and the relationships between them, which link labels would
/// have carried, are buried in prose instead. Measured over what the embedding is built from, which
/// is content, `why` and keywords together, because that is the text recall ranks.
#[test]
fn a_memory_past_the_length_limit_is_refused_with_advice_to_split() {
    let mut server = a_server();
    let before = payload(&call(
        &mut server,
        "hoard_sets",
        json!({"action": "status"}),
    ));

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha ".repeat(400),
            "why": "because it is long",
            "confidence": 0.8,
            "origin": "llm"
        }),
    );

    assert!(is_error(&response), "{response}");
    let message = response["result"]["content"][0]["text"]
        .as_str()
        .expect("a message");
    assert!(
        message.contains("2000"),
        "the refusal names the limit, so a caller can aim at it: {message}"
    );
    assert!(
        message.contains("2418"),
        "and names the length measured, which is what the caller has to change: {message}"
    );
    assert!(
        message.to_lowercase().contains("split"),
        "and says what to do about it, which is the whole reason for the limit: {message}"
    );

    let after = payload(&call(
        &mut server,
        "hoard_sets",
        json!({"action": "status"}),
    ));
    assert_eq!(
        after["sets"][0]["memories"], before["sets"][0]["memories"],
        "a refused write must leave the set exactly as it was"
    );
}

/// **Refused, and nothing stored.** This reverses what the tool used to do, on the human partner's
/// decision of 2026-08-16: it remarked and stored the damaged memory anyway, which left the caller
/// to repair it — and writing a repair is exactly the moment of emitting many parameters in
/// sequence, so four of the recorded occurrences were repairs for the one before. Refusing turns a
/// corruption plus a repair into a single failed call.
#[test]
fn a_swallowed_parameter_is_refused_and_leaves_nothing_behind() {
    let mut server = a_server();
    let before = payload(&call(
        &mut server,
        "hoard_sets",
        json!({"action": "status"}),
    ));

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha</content><parameter name=\"why\">swallowed",
            "confidence": 0.8,
            "origin": "llm"
        }),
    );

    assert!(is_error(&response), "{response}");
    let message = response["result"]["content"][0]["text"]
        .as_str()
        .expect("a message");
    assert!(
        message.contains("why"),
        "the refusal names the parameter that went missing: {message}"
    );

    let after = payload(&call(
        &mut server,
        "hoard_sets",
        json!({"action": "status"}),
    ));
    assert_eq!(
        after["sets"][0]["memories"], before["sets"][0]["memories"],
        "a refused write must leave the set exactly as it was"
    );
}

/// The field closed badly is not always `content` -- occurrences three and four were a mis-closed
/// `why`, which swallowed `keywords`. What names the damage is the *pairing*: framing naming a
/// parameter, and that parameter arriving empty.
#[test]
fn damage_in_why_is_caught_by_the_parameter_it_swallowed() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha",
            "why": "because</why><parameter name=\"keywords\">gone",
            "confidence": 0.8,
            "origin": "llm"
        }),
    );

    assert!(is_error(&response), "{response}");
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|message| message.contains("keywords")),
        "{response}"
    );
}

// ---------------------------------------------------------------------------
// Incoming edges
// ---------------------------------------------------------------------------

/// `incoming-edges-invisible-on-the-wire`: the store's links table is its own reverse index, and
/// the wire exposed one hard-coded slice of it. A caller arriving at the *target* of a link — the
/// direction anyone travels when following one — saw an unconnected memory with no way to learn
/// otherwise. `documents/design/recall-mechanisms.md` settles that these ship by default: an edge
/// sent only when asked for is invisible to a caller that does not know to ask.
#[test]
fn a_memory_reports_the_edges_pointing_at_it() {
    let mut server = a_server();
    let target = payload(&memorize(&mut server, "the claim being disputed"))["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let disputer = payload(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the claim that disputes it",
            "why": "because the test needs one",
            "confidence": 0.8,
            "origin": "llm",
            "links": [{"label": "contradicts", "target": target}]
        }),
    ))["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let fetched =
        payload(&call(&mut server, "hoard_lookup", json!({"id": target})))["memories"][0].clone();

    let incoming = fetched["incoming"].as_array().unwrap_or_else(|| {
        panic!("the disputed memory should report the edge pointing at it, got {fetched}")
    });

    assert_eq!(
        incoming.len(),
        1,
        "exactly one edge points at it: {fetched}"
    );
    assert_eq!(incoming[0]["source"], json!(disputer));
    assert_eq!(
        incoming[0]["label"],
        json!("contradicts"),
        "the label is the one that was written, not a synthesised inverse"
    );
}

/// The label carried inbound is *not* special-cased per label — excluding supersession because
/// `superseded_by` already reports it would be the same hard-coded slice, rebuilt. The two answer
/// different questions: `superseded_by` reports validity, `incoming` reports edges.
#[test]
fn incoming_edges_are_not_filtered_by_label() {
    let mut server = a_server();
    let original = payload(&memorize(&mut server, "the original claim"))["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the corrected claim",
            "why": "because the test needs one",
            "confidence": 0.8,
            "origin": "llm",
            // Superseding states the replacement's keywords; this fixture is not about indexing.
            "keywords": [],
            "links": [{"label": "supersedes", "target": original}]
        }),
    );

    let fetched =
        payload(&call(&mut server, "hoard_lookup", json!({"id": original})))["memories"][0].clone();

    let incoming = fetched["incoming"]
        .as_array()
        .unwrap_or_else(|| panic!("a superseded memory still reports the edge, got {fetched}"));
    assert_eq!(incoming[0]["label"], json!("supersedes"), "{fetched}");
}

// ---------------------------------------------------------------------------
// Traversal and paging, from a client's side of the wire
// ---------------------------------------------------------------------------

/// A memory reached by the walk arrives with `inherited` beside its own `similarity`, so a caller
/// handed a memory that plainly does not match its query can tell **why** it is here: because of
/// what it is linked to. Ranking on a number the caller cannot see is what made
/// `similarity-describes-a-different-memory` misleading rather than merely wrong.
#[test]
fn a_memory_reached_by_a_link_says_so() {
    let mut server = a_server();
    let matched = memorize_full(&mut server, "alpha alpha alpha", json!([]), json!([]));
    memorize_full(
        &mut server,
        "beta beta beta",
        json!([]),
        json!([{"label": "supports", "target": matched}]),
    );
    // The walk takes five seeds and a seed never inherits, so a corpus at or below that size cannot
    // show traversal at all: every memory is a walk origin. These fill the budget so the linked
    // memory is reached rather than seeded.
    for filler in [
        "alpha alpha counts the retries",
        "alpha alpha names the shard",
        "alpha alpha alpha bounds the transport",
        "alpha alpha alpha orders the queue",
    ] {
        memorize(&mut server, filler);
    }

    // Above the corpus mean on the dimension that varies, or centring ranks the memories that do
    // not mention the query at all.
    //
    // Paged rather than taken from one call: a memory the walk lifts need not clear the cut, and
    // here it does not, because the drop below the direct matches is the largest one there is.
    // Paging past what you already hold is how a caller reaches it, which is the pairing
    // `already_known` was built for.
    let mut held: Vec<Value> = Vec::new();
    let mut memories: Vec<Value> = Vec::new();
    for _ in 0..5 {
        let page = payload(&call(
            &mut server,
            "hoard_recall",
            json!({"query": "alpha alpha alpha", "already_known": held.clone()}),
        ));
        let page = page["memories"].as_array().expect("a list").clone();
        if page.is_empty() {
            break;
        }
        for memory in &page {
            held.push(memory["id"].clone());
        }
        memories.extend(page);
    }

    let reached = memories
        .iter()
        .find(|memory| {
            memory["content"]
                .as_str()
                .is_some_and(|content| content.contains("beta"))
        })
        .expect("the linked memory should have been reached");
    assert!(
        reached["inherited"]
            .as_f64()
            .is_some_and(|score| score > 0.0),
        "a memory the walk reached has to report what it inherited: {reached}"
    );
    assert!(
        reached["similarity"].as_f64().is_some_and(|own| own < 0.0),
        "and it is here despite its own wording, which is the whole point: {reached}"
    );
}

/// Nothing links to anything in most results, and `inherited` is absent rather than zero there. An
/// absent field and a zero say the same thing, and the zero costs a line in every memory returned.
#[test]
fn an_unreached_memory_carries_no_inherited_score() {
    let mut server = a_server();
    memorize(&mut server, "alpha alpha alpha");
    memorize(&mut server, "gamma gamma gamma");

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ));

    for memory in result["memories"].as_array().expect("a list") {
        assert!(
            memory.get("inherited").is_none(),
            "nothing was linked, so nothing should claim to have been reached: {memory}"
        );
    }
}

/// Passing back the ids from the last call is how a caller says *give me something else*. Without
/// it a second call against a deterministic ranking can only return what the first one did.
#[test]
fn already_known_memories_are_left_out_of_the_results() {
    let mut server = a_server();
    memorize(&mut server, "alpha alpha alpha");
    memorize(&mut server, "alpha alpha beta");
    memorize(&mut server, "alpha gamma");

    let first = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ));
    let held: Vec<Value> = first["memories"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|memory| memory["id"].clone())
        .collect();
    assert!(!held.is_empty());

    let second = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "already_known": held.clone()}),
    ));

    for memory in second["memories"].as_array().expect("a list") {
        assert!(
            !held.contains(&memory["id"]),
            "a memory the caller said it holds came back anyway: {memory}"
        );
    }
}

/// **The keyword path honours it too.** `keyword-recall-ignores-already-known`: the schema promises
/// held ids are left out, with no word about which search path, and only the semantic one read them
/// -- observed 2026-09-25 returning exactly the three ids a caller had passed. The parameter's tests
/// all went through `query`, so the other route to the same behaviour was never guarded.
#[test]
fn already_known_memories_are_left_out_of_a_keyword_recall() {
    let mut server = a_server();
    let mut ids: Vec<String> = ["alpha one", "alpha two", "alpha three"]
        .iter()
        .map(|content| memorize_full(&mut server, content, json!(["paging"]), json!([])))
        .collect();
    // Keyword ties order by id, oldest first, so this is the order a caller pages through.
    ids.sort();

    let second = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["paging"], "already_known": [&ids[0]]}),
    ));
    let returned: Vec<&str> = second["memories"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|memory| memory["id"].as_str())
        .collect();
    assert_eq!(
        returned,
        vec![ids[1].as_str(), ids[2].as_str()],
        "a memory the caller said it holds came back anyway, or the rest did not: {second}"
    );
}

/// **And leaving them out does not shrink the page.** Each store is asked for `limit` rows before
/// the merge, so removing held ids from what came back would return fewer than `limit` while more
/// matching memories wait below -- the next tranche arriving short, and reading as the end.
#[test]
fn leaving_out_known_memories_still_fills_a_keyword_page() {
    let mut server = a_server();
    let mut ids: Vec<String> = ["alpha one", "alpha two", "alpha three", "alpha four"]
        .iter()
        .map(|content| memorize_full(&mut server, content, json!(["paging"]), json!([])))
        .collect();
    ids.sort();

    let page = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["paging"], "limit": 2, "already_known": [&ids[0], &ids[1]]}),
    ));
    let returned: Vec<&str> = page["memories"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|memory| memory["id"].as_str())
        .collect();
    assert_eq!(
        returned,
        vec![ids[2].as_str(), ids[3].as_str()],
        "the page after two held memories should be the next two: {page}"
    );
}

/// An id prefix is not an id. The refusal has to name the argument and say what to pass instead --
/// the one refusal shape a caller can act on without guessing.
#[test]
fn an_already_known_id_that_is_a_prefix_is_refused_by_name() {
    let mut server = a_server();
    memorize(&mut server, "alpha alpha alpha");

    let response = call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "already_known": ["1a2b3c4d"]}),
    );

    assert!(is_error(&response));
    let message = response["result"]["content"][0]["text"]
        .as_str()
        .expect("a message");
    assert!(
        message.contains("already_known") && message.contains("prefix"),
        "the refusal has to name the argument and the mistake: {message}"
    );
}

/// **A supersession cycle reaches the client, by name.** Resolution has always detected it; both
/// callers discarded the answer, so a cycle arrived byte-identical to a memory nothing supersedes —
/// in the one situation where a caller is least able to work it out for itself.
///
/// The store is built by import rather than through the tool, because writing a cycle is refused:
/// only a hand-edit during a merge, or a file from elsewhere, can deliver one. That is exactly the
/// case the design says read-time reporting exists for.
#[test]
fn a_supersession_cycle_is_named_in_a_lookup() {
    let mut first = a_memory("one");
    let mut second = a_memory("two");
    first.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(second.id),
    }];
    second.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(first.id),
    }];

    let mut store = chartered(scope());
    store.set_embedder(Arc::new(MarkerEmbedder));
    store
        .import(&MemorySet {
            scope: scope(),
            memories: vec![first.clone(), second.clone()],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("a corrupt file still imports");

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(store);

    let found = payload(&call(
        &mut server,
        "hoard_lookup",
        json!({"id": first.id.to_string()}),
    ))["memories"][0]
        .clone();

    assert_eq!(
        found["supersession_cycle"],
        json!(true),
        "the caller has to be told the history is a loop: {found}"
    );
    assert!(
        found.get("superseded_by").is_none(),
        "and not handed a replacement, because there is no honest one to name: {found}"
    );
}

/// Lookup must not tell a caller to go and load a set they are already holding.
///
/// `superseded-beyond-this-set-is-reported-when-the-set-is-loaded`. Lookup is where a session reads
/// a memory *on purpose*, so it is exempt from substitution and returns the memory flagged -- which
/// makes it the surface where the flag's wording does all the work. The remedy the flag exists to
/// enable is *load that set and fetch the replacement*, and this caller has done the first half.
#[test]
fn lookup_names_a_replacement_in_a_loaded_set_rather_than_calling_it_out_of_reach() {
    let mut server = a_server_with_two_sets();

    let replacement = memorize_full_in(
        &mut server,
        json!("shared:coding-lessons"),
        "the replacement, in the other set",
    );
    let conceding = memorize_full(
        &mut server,
        "conceding to a memory in the other set",
        json!([]),
        json!([{
            "label": "superseded_by",
            "target": replacement,
            "target_set": "coding-lessons"
        }]),
    );

    let found = payload(&call(&mut server, "hoard_lookup", json!({"id": conceding})))["memories"]
        [0]
    .clone();

    assert!(
        found.get("superseded_beyond_this_set").is_none(),
        "flagged as replaced from beyond this machine, while the named set is loaded and its \
         memory is one call away: {found}"
    );
    assert_eq!(
        found["superseded_by"],
        json!([replacement]),
        "and the replacement has to be named, which is the whole of what lookup can offer here: \
         {found}"
    );
}

/// The flags are absent when they do not apply, which is what keeps them meaningful when they do.
#[test]
fn an_ordinary_lookup_carries_no_validity_flags() {
    let mut server = a_server();
    let id = memorize_full(&mut server, "still true", json!([]), json!([]));

    let found =
        payload(&call(&mut server, "hoard_lookup", json!({"id": id})))["memories"][0].clone();

    assert!(found.get("supersession_cycle").is_none());
    assert!(found.get("superseded_beyond_this_set").is_none());
    assert!(found.get("superseded_by").is_none());
}

// ---------------------------------------------------------------------------
// Writing markup on purpose, and having a swallowed parameter refused
// ---------------------------------------------------------------------------

/// **The escape has to exist before the refusal does.** A memory whose subject *is* tool-call
/// markup — which this project writes repeatedly, about this very defect — must have a way to say
/// *this is literal text*. Escaping works upstream of the damage: a caller writing `&lt;` emits no
/// markup at all, so there is nothing for a malformed close to swallow.
#[test]
fn escaped_markup_is_stored_as_the_characters_it_names() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "a mis-closed &lt;/why&gt; absorbs everything after it",
            "why": "so a memory about the defect can be written at all",
            "confidence": 0.9,
            "origin": "llm"
        }),
    );
    assert!(
        !is_error(&response),
        "this is a legitimate memory: {response}"
    );

    let id = payload(&response)["id"].as_str().expect("an id").to_owned();
    let stored =
        payload(&call(&mut server, "hoard_lookup", json!({"id": id})))["memories"][0].clone();
    assert_eq!(
        stored["content"],
        json!("a mis-closed </why> absorbs everything after it"),
        "the entities decode to the characters they name: {stored}"
    );
}

/// The ampersand is in the minimal set because the escape has to be able to describe itself: a
/// memory about escaping writes `&amp;lt;` and means `&lt;`.
#[test]
fn an_escaped_ampersand_lets_a_memory_write_about_escaping() {
    let mut server = a_server();
    let id = memorize_full_with(
        &mut server,
        "write &amp;lt; when you mean the escape rather than the character",
        "because the escape must be able to describe itself",
    );

    let stored =
        payload(&call(&mut server, "hoard_lookup", json!({"id": id})))["memories"][0].clone();
    assert_eq!(
        stored["content"],
        json!("write &lt; when you mean the escape rather than the character"),
        "one pass, and the output is not re-scanned: {stored}"
    );
}

/// Decoding is a boundary transformation over every prose field, not only `content`.
#[test]
fn the_escape_reaches_why_and_context_as_well_as_content() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "plain content",
            "why": "a why mentioning &lt;/context&gt; on purpose",
            "context": "a context mentioning &lt;/why&gt; on purpose",
            "confidence": 0.9,
            "origin": "llm"
        }),
    );
    let id = payload(&response)["id"].as_str().expect("an id").to_owned();
    let stored =
        payload(&call(&mut server, "hoard_lookup", json!({"id": id})))["memories"][0].clone();

    assert_eq!(
        stored["why"],
        json!("a why mentioning </context> on purpose")
    );
    assert_eq!(
        stored["context"],
        json!("a context mentioning </why> on purpose")
    );
}

/// **The conjunction, which is the whole point of the change.** A memory that merely *quotes* the
/// markup arrives with its other fields intact, and must be stored without complaint. The previous
/// check was a disjunction over substrings, so it flagged this — and TRACKING.md itself, which
/// discusses `</why>` at length, would have been unstorable as a memory.
#[test]
fn a_memory_that_quotes_markup_with_its_fields_intact_is_stored_quietly() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the damage looks like </why> arriving inside the text",
            "why": "this field arrived, which is what says the call was well formed",
            "confidence": 0.9,
            "origin": "llm"
        }),
    );

    assert!(!is_error(&response), "{response}");
    let result = payload(&response);
    assert!(
        result.get("suspect_markup").is_none(),
        "nothing was swallowed, so nothing should be remarked on: {result}"
    );
}

/// **The true positive: refused, not stored.** Content carrying a parameter tag that names a field
/// which then arrived empty is the signature of a swallowed parameter, and both halves are in hand.
/// Refusing turns a corruption plus a repair into one failed call -- the repair being itself a site
/// of the defect is what made the loop.
#[test]
fn a_write_whose_parameter_was_swallowed_is_refused_naming_what_it_recovered() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the real content</content>\n<parameter name=\"context\">the absorbed context",
            "why": "present, and not the field that was swallowed",
            "confidence": 0.9,
            "origin": "llm"
        }),
    );

    assert!(is_error(&response), "this must not be stored: {response}");
    let message = response["result"]["content"][0]["text"]
        .as_str()
        .expect("a message");
    assert!(
        message.contains("context"),
        "the refusal has to name the field that was swallowed: {message}"
    );
    assert!(
        message.contains("&lt;"),
        "and point at the escape, or a memory about markup has nowhere to go: {message}"
    );
}

// ---------------------------------------------------------------------------
// Link targets: reported when they cannot be resolved, never refused.
//
// `documents/design/cross-set-references.md`, *an unresolvable target is reported, never refused*.
// The relaxed stance leaves the decision to the caller, which needs the caller to be told.
// ---------------------------------------------------------------------------

/// A server holding two shared sets, so a cross-set link has somewhere real to point.
fn a_server_with_two_sets() -> Server {
    let mut server = a_server();
    let mut lessons = chartered(Scope::Shared(
        SetName::new("coding-lessons").expect("valid set name"),
    ));
    lessons.set_embedder(Arc::new(MarkerEmbedder));
    server.add_store(lessons);
    server
}

fn memorize_linking(server: &mut Server, links: Value) -> Value {
    call(
        server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha, with a link",
            "confidence": 0.9,
            "origin": "llm",
            // Stated because a caller may pass a `supersedes` link, which requires it. These
            // fixtures are about link handling, so empty is the truthful answer.
            "keywords": [],
            "links": links
        }),
    )
}

#[test]
fn a_link_to_an_id_this_set_does_not_hold_is_reported_and_still_stored() {
    let mut server = a_server();
    let absent = a_memory("never stored").id.to_string();

    let response = memorize_linking(
        &mut server,
        json!([{"label": "related_to", "target": absent}]),
    );

    assert!(
        !is_error(&response),
        "a link is apparatus, so it must not cost the claim: {response}"
    );
    let body = payload(&response);
    let unresolved = body["unresolved_links"]
        .as_array()
        .unwrap_or_else(|| panic!("expected unresolved_links, got {body}"));
    assert_eq!(unresolved.len(), 1, "one link, one report: {body}");
    assert_eq!(unresolved[0]["target"], json!(absent));
    assert_eq!(unresolved[0]["state"], json!("no_such_id_in_this_set"));

    // The memory itself is there.
    let stored = call(
        &mut server,
        "hoard_lookup",
        json!({"id": body["id"].as_str().expect("an id")}),
    );
    assert!(!is_error(&stored), "the memory should have been stored");
}

#[test]
fn a_link_to_a_forgotten_memory_resolves() {
    let mut server = a_server();
    let target = memorize_full_with(&mut server, "alpha target", "because");
    let forgotten = call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": {"shared": "scatterhoard"}, "id": target}),
    );
    assert!(!is_error(&forgotten), "forget failed: {forgotten}");

    let response = memorize_linking(
        &mut server,
        json!([{"label": "supersedes", "target": target}]),
    );

    let body = payload(&response);
    assert!(
        body.get("unresolved_links").is_none(),
        "a tombstone is a resolved target -- a supersession chain whose middle was forgotten still \
         holds a good reference: {body}"
    );
}

#[test]
fn a_cross_set_link_into_a_loaded_set_is_checked() {
    let mut server = a_server_with_two_sets();
    let absent = a_memory("never stored").id.to_string();

    let response = memorize_linking(
        &mut server,
        json!([{"label": "motivated_by", "target": absent, "target_set": "coding-lessons"}]),
    );

    let body = payload(&response);
    let unresolved = body["unresolved_links"]
        .as_array()
        .unwrap_or_else(|| panic!("expected unresolved_links, got {body}"));
    assert_eq!(unresolved[0]["set"], json!("coding-lessons"));
    assert_eq!(unresolved[0]["state"], json!("no_such_id_in_that_set"));
}

#[test]
fn a_cross_set_link_into_a_set_this_machine_does_not_hold_says_the_check_did_not_run() {
    let mut server = a_server();
    let elsewhere = a_memory("in another set").id.to_string();

    let response = memorize_linking(
        &mut server,
        json!([{"label": "related_to", "target": elsewhere, "target_set": "coding-lessons"}]),
    );

    let body = payload(&response);
    let unresolved = body["unresolved_links"]
        .as_array()
        .unwrap_or_else(|| panic!("expected unresolved_links, got {body}"));
    assert_eq!(
        unresolved[0]["state"],
        json!("set_not_loaded"),
        "silence here would be indistinguishable from a check that passed: {body}"
    );
}

#[test]
fn links_that_resolve_are_not_reported() {
    let mut server = a_server_with_two_sets();
    let target = memorize_full_with(&mut server, "alpha target", "because");

    let response = memorize_linking(
        &mut server,
        json!([{"label": "related_to", "target": target}]),
    );

    let body = payload(&response);
    assert!(
        body.get("unresolved_links").is_none(),
        "nothing to report should say nothing: {body}"
    );
}

#[test]
fn amend_checks_the_links_it_replaces() {
    let mut server = a_server();
    let id = memorize_full_with(&mut server, "alpha subject", "because");
    let absent = a_memory("never stored").id.to_string();

    let response = call(
        &mut server,
        "hoard_correct",
        json!({
            "action": "amend",
            "scope": {"shared": "scatterhoard"},
            "id": id,
            "links": [{"label": "related_to", "target": absent}]
        }),
    );

    assert!(!is_error(&response), "amend should not refuse: {response}");
    let body = payload(&response);
    let unresolved = body["unresolved_links"].as_array().unwrap_or_else(|| {
        panic!("amend replaces the whole array, so it needs the same check: {body}")
    });
    assert_eq!(unresolved[0]["state"], json!("no_such_id_in_this_set"));
}

// ---------------------------------------------------------------------------
// Prohibited data: refused at the write, where the caller owns the remedy.
//
// `documents/design/prohibited-data.md`. Refusing here and only warning at export is deliberate --
// see the reasoning recorded in export-review.md: a write is one memory that does not exist yet and
// is definitionally the caller's, while an export is a whole set that may hold imported memories the
// caller cannot properly fix, and a filter that gains a rule would retroactively block it.
// ---------------------------------------------------------------------------

/// Fabricated, and shaped like the real thing: a real prefix with a body that is not a real key.
const A_FAKE_KEY: &str = "ghp_016c75a7f9b24e0d8c3aa15be92d4f7c1a8e";

#[test]
fn a_key_in_the_content_is_refused_and_nothing_is_stored() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": format!("the deploy needs {A_FAKE_KEY} in the environment"),
            "confidence": 0.9,
            "origin": "llm"
        }),
    );

    assert!(
        is_error(&response),
        "a credential must not be stored: {response}"
    );
    let refusal = message(&response);
    assert!(
        refusal.contains("api_key") || refusal.contains("API key"),
        "the refusal should name what it recognised: {refusal}"
    );
    assert!(
        refusal.contains("file"),
        "and point at the remedy the conventions already prescribe: {refusal}"
    );
    assert!(
        !refusal.contains(A_FAKE_KEY),
        "and must not quote the credential back, which would spread it: {refusal}"
    );
}

#[test]
fn a_key_in_why_or_context_is_refused_too() {
    for field in ["why", "context"] {
        let mut server = a_server();
        let mut arguments = json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha, an ordinary memory",
            "confidence": 0.9,
            "origin": "llm"
        });
        arguments[field] = json!(format!("recorded while holding {A_FAKE_KEY}"));
        let response = call(&mut server, "hoard_memorize", arguments);
        assert!(
            is_error(&response),
            "{field} is prose a caller writes: {response}"
        );
    }
}

#[test]
fn amending_a_context_into_a_credential_is_refused() {
    let mut server = a_server();
    let id = memorize_full_with(&mut server, "alpha subject", "because");
    let response = call(
        &mut server,
        "hoard_correct",
        json!({
            "action": "amend",
            "scope": {"shared": "scatterhoard"},
            "id": id,
            "context": format!("found while testing with {A_FAKE_KEY}")
        }),
    );
    assert!(
        is_error(&response),
        "amend writes prose too, so it is the same door: {response}"
    );
}

#[test]
fn an_ordinary_memory_about_secrets_still_stores() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "secrets -- API keys, passwords, card numbers -- are never stored in any \
                        set; put them in a file and record where the file is",
            "confidence": 0.9,
            "origin": "llm"
        }),
    );
    assert!(
        !is_error(&response),
        "matching vocabulary would make this project unable to record its own rule: {response}"
    );
}

/// An id in the other loaded set, spelled the way a caller would write one.
const AN_ID_ELSEWHERE: &str = "01M0RQEYAEE10BM5K17SGCVN2A";

/// A `SUPERSEDES` reaching into another set is **refused at the write**.
///
/// `documents/design/cross-set-references.md`: no edge changes the standing of a memory in another
/// set. This is the honest-client half of that rule — it stops our own tool authoring the edge.
/// It is deliberately *not* the half that protects anyone: a hostile file was never written by this
/// client, which is why the edge is also ignored at resolution.
#[test]
fn a_cross_set_supersedes_is_refused() {
    let mut server = a_server_with_two_sets();
    let response = memorize_linking(
        &mut server,
        json!([{
            "label": "supersedes",
            "target": AN_ID_ELSEWHERE,
            "target_set": "coding-lessons"
        }]),
    );

    assert!(
        is_error(&response),
        "a memory in one set was allowed to claim authority over another set's: {response}"
    );
    let text = format!("{response}");
    assert!(
        text.contains("superseded_by") || text.contains("SUPERSEDED_BY"),
        "the refusal must name the move the caller can actually make, per the standing rule that a \
         message telling its reader to fix something names an action its reader can take: {text}"
    );
}

/// The carve-out, and it must not be lost: a memory may record **its own** obsolescence across a
/// set boundary. That is a claim about itself, not a reach into someone else's standing, and it is
/// the only form that survives the replacement's set being unavailable.
#[test]
fn a_cross_set_superseded_by_is_accepted() {
    let mut server = a_server_with_two_sets();
    let response = memorize_linking(
        &mut server,
        json!([{
            "label": "superseded_by",
            "target": AN_ID_ELSEWHERE,
            "target_set": "coding-lessons"
        }]),
    );

    assert!(
        !is_error(&response),
        "a memory was refused permission to record that it had been replaced: {response}"
    );
}

/// The refusal is about the set boundary, not about the label. A `SUPERSEDES` within one set is the
/// ordinary way a correction is recorded and must stay ordinary.
#[test]
fn a_same_set_supersedes_is_unaffected() {
    let mut server = a_server_with_two_sets();
    let existing = memorize_full_with(&mut server, "alpha the original claim", "because reasons");
    let response = memorize_linking(
        &mut server,
        json!([{"label": "supersedes", "target": existing}]),
    );

    assert!(
        !is_error(&response),
        "the everyday correction path was caught by a rule about set boundaries: {response}"
    );
}

/// And it is about *standing*, not about crossing. A descriptive cross-set link asserts no authority
/// and is exactly what the design invites — over-refusing here would take away the feature
/// `documents/design/cross-set-references.md` opens by contemplating.
#[test]
fn a_cross_set_descriptive_link_is_unaffected() {
    let mut server = a_server_with_two_sets();
    let response = memorize_linking(
        &mut server,
        json!([{
            "label": "related_to",
            "target": AN_ID_ELSEWHERE,
            "target_set": "coding-lessons"
        }]),
    );

    assert!(
        !is_error(&response),
        "a cross-set association was refused, which is the link type the design exists to allow: \
         {response}"
    );
}

/// `amend` replaces a memory's whole link array, so it is a second door onto the same write. The
/// refusal must be the identical one — a lighter check here would make the rule optional.
#[test]
fn amend_refuses_a_cross_set_supersedes_too() {
    let mut server = a_server_with_two_sets();
    let id = memorize_full_with(&mut server, "alpha a claim", "because reasons");

    let response = call(
        &mut server,
        "hoard_correct",
        json!({
            "action": "amend",
            "scope": {"shared": "scatterhoard"},
            "id": id,
            "links": [{
                "label": "supersedes",
                "target": AN_ID_ELSEWHERE,
                "target_set": "coding-lessons"
            }]
        }),
    );

    assert!(
        is_error(&response),
        "the rule was enforced at memorize and skipped at amend, so it is not a rule: {response}"
    );
}

/// The claim reaches the **tool response**, not merely the Rust struct.
///
/// Worth its own test: `disputed_by` is `skip_serializing_if`, so a wrong predicate would hide it
/// from every caller while the store-level tests stayed green.
#[test]
fn a_recall_hit_names_the_set_disputing_it() {
    let mut server = a_server_with_two_sets();
    let ours = memorize_full_with(
        &mut server,
        "alpha alpha ours and disputed",
        "because reasons",
    );

    // The rival's memory concerns something else entirely, so it cannot arrive on merit.
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "coding-lessons"},
            "content": "gamma gamma the rival rebuttal",
            "confidence": 0.9,
            "origin": "llm",
            "links": [{"label": "superseded_by", "target": ours, "target_set": "scatterhoard"}]
        }),
    );
    assert!(!is_error(&response), "setup failed: {response}");

    let recall = call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 5}),
    );
    let text = format!("{recall}");
    assert!(
        !text.contains("disputed_by"),
        "a `superseded_by` aimed at our memory concedes the claimant's own standing and is not a \
         dispute: {text}"
    );
}

/// And the real thing: a `supersedes` from another set is reported, names that set, and does not
/// drag the claiming memory into the results.
///
/// The rival memory is inserted into a store built here rather than through `hoard_memorize`,
/// because the tool surface refuses to author this edge — which is exactly the situation being
/// tested. The file an attacker controls was never written by our client, so the protection and the
/// reporting both have to work against a store that already contains the edge.
#[test]
fn a_cross_set_supersedes_written_by_hand_is_reported_not_obeyed() {
    let mut server = a_server_with_two_sets();
    let ours = memorize_full_with(
        &mut server,
        "alpha alpha ours and disputed",
        "because reasons",
    );
    let our_id = scatterhoard::memory::MemoryId::parse(&ours).expect("a real id");

    // Content deliberately unrelated to the query, so it cannot arrive on merit.
    let mut rebuttal = a_memory("gamma gamma the rebuttal");
    rebuttal.links = vec![Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::CrossSet {
            set: SetName::new("scatterhoard").expect("valid set name"),
            id: our_id,
            keywords: Vec::new(),
            set_id: None,
        },
    }];
    let mut rival = chartered(Scope::Shared(
        SetName::new("rival").expect("valid set name"),
    ));
    rival.set_embedder(Arc::new(MarkerEmbedder));
    rival.insert(&rebuttal).expect("inserts");
    server.add_store(rival);

    let recall = call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 5}),
    );
    let memories = payload(&recall)["memories"].clone();
    let hit = memories
        .as_array()
        .expect("memories is an array")
        .iter()
        .find(|memory| memory["id"] == json!(ours))
        .unwrap_or_else(|| panic!("our memory was suppressed: {recall}"));

    let claims = hit["disputed_by"].as_array().unwrap_or_else(|| {
        panic!("the dispute is not reported, so a session cannot judge it: {hit}")
    });
    assert_eq!(claims.len(), 1, "got {claims:?}");
    assert_eq!(claims[0]["set"], json!("rival"));
    assert_eq!(claims[0]["label"], json!("supersedes"));
    assert_eq!(claims[0]["source"], json!(rebuttal.id.to_string()));

    // **The edge, not the memory.** The claim carries an id, a set and a label -- and nothing that
    // would let a rival put words in front of a reader. Asserted on the claim's own keys rather
    // than on the whole response, because whether the rival's memory also arrives *on merit* is a
    // question about the ranking, not about this mechanism.
    let keys: Vec<&String> = claims[0]
        .as_object()
        .expect("a claim is an object")
        .keys()
        .collect();
    assert_eq!(
        keys,
        vec!["label", "set", "source"],
        "a claim grew a field; if it can carry content, disputing buys a voice"
    );
}

/// A `supersedes` whose `target_set` names **the set being written into** is a same-set correction
/// written the long way, and must be accepted.
///
/// The cross-set refusal used to ask `does this link carry a set name`, which caught this. The
/// question it means to ask is `is the target in a different set`, which does not — the same
/// conflation that made `a-same-set-link-spelled-with-its-set-name-is-inert` inert at resolution.
#[test]
fn a_supersedes_naming_the_writing_scope_is_not_refused() {
    let mut server = a_server_with_two_sets();
    let original = memorize_full_with(&mut server, "alpha the original", "because reasons");

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha the correction",
            "confidence": 0.9,
            "origin": "llm",
            // Superseding states the replacement's keywords; this fixture is not about indexing.
            "keywords": [],
            "links": [{
                "label": "supersedes",
                "target": original,
                "target_set": "scatterhoard"
            }]
        }),
    );

    assert!(
        !is_error(&response),
        "an ordinary same-set correction was refused for naming its own set: {response}"
    );

    // And it took effect, rather than being stored as an edge that changes nothing.
    let recall = call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 5}),
    );
    let text = format!("{recall}");
    assert!(
        !text.contains("\"id\":\"") || !text.contains(&format!("\"id\":\"{original}\"")),
        "the superseded memory came back as current, so the link was stored inert: {text}"
    );
}

/// **A scope on the wire is one spelling, and it is the one the schema publishes.**
///
/// `a-scope-in-a-response-is-not-the-spelling-the-schema-publishes`. `oneof-tool-schemas-present-no-parameters`
/// settled that `scope` publishes as `local` or `shared:<name>` — the spelling `Scope::as_key`
/// writes into a store's `meta` row — while the object form keeps *parsing*, so nothing that worked
/// stopped working. That covered arguments and was never applied to responses, where `Scope`'s
/// derived `Serialize` runs. A caller reading the published schema learned one spelling and met
/// another in every result.
///
/// This is the retrieval half; `hoard_sets` has its own in `tests/sets.rs`.
#[test]
fn a_recalled_memorys_scope_is_the_spelling_the_schema_publishes() {
    let mut server = a_server();
    let id = memorize_full_with(
        &mut server,
        "alpha governs routing",
        "so it can be recalled",
    );

    let recalled = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 5}),
    ));
    assert_eq!(
        recalled["memories"][0]["scope"],
        json!("shared:scatterhoard"),
        "recall should publish the scalar spelling: {recalled}"
    );

    let by_keyword = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["alpha"], "limit": 5}),
    ));
    let looked_up = payload(&call(&mut server, "hoard_lookup", json!({"id": id})));
    assert_eq!(
        looked_up["memories"][0]["scope"],
        json!("shared:scatterhoard")
    );

    // And nowhere in any of the three, at any depth. A named-field assertion would pass for a field
    // added later, which is exactly how the two spellings came to coexist.
    let mut found = Vec::new();
    object_form_scopes(&recalled, "recall", &mut found);
    object_form_scopes(&by_keyword, "keyword recall", &mut found);
    object_form_scopes(&looked_up, "lookup", &mut found);
    assert!(
        found.is_empty(),
        "the object form reached a response: {found:?}"
    );
}

// ---------------------------------------------------------------------------
// The summary detail level: an id and a derived line, on either recall path.
//
// `recall-summary-output-mode` in TRACKING.md carries the measurements. The choice is the caller's
// because the caller is the only party that knows whether it is surveying or reading -- the test in
// documents/design/decision-placement.md -- and the full level stays the default because a caller
// that has not chosen must not silently lose the record.
// ---------------------------------------------------------------------------

/// Long enough that its summary is a cut rather than a copy, and shaped like the memories this
/// index exists for: the claim first, the qualification after.
const A_LONG_MEMORY: &str = "Push is the marked case, and most accumulation stays passive. The rest \
                             is reached by ordinary recall, which is the budget story push had \
                             been missing all along.";

fn summary_recall(server: &mut Server, arguments: Value) -> Vec<Value> {
    let response = call(server, "hoard_recall", arguments);
    assert!(
        !is_error(&response),
        "recall failed: {}",
        message(&response)
    );
    payload(&response)["memories"]
        .as_array()
        .expect("memories")
        .clone()
}

#[test]
fn a_summary_recall_returns_an_id_and_a_line_and_nothing_else() {
    let mut server = a_server();
    memorize(&mut server, A_LONG_MEMORY);

    let memories = summary_recall(
        &mut server,
        json!({"query": "what push costs", "detail": "summary"}),
    );

    assert_eq!(memories.len(), 1, "one memory was stored: {memories:?}");
    let fields: Vec<&String> = memories[0]
        .as_object()
        .expect("an object")
        .keys()
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        vec!["id", "summary"],
        "a summary hit carries the id and the line, and nothing that would undo the saving"
    );
    assert_eq!(
        memories[0]["summary"],
        json!("Push is the marked case, and most accumulation stays passive.\u{2026}")
    );
}

/// The projection is on the read side rather than in the watch mechanism, so it applies to whichever
/// way the memory was found. A survey over a semantic recall is the use it was not designed for and
/// the reason it is general.
#[test]
fn the_keyword_path_summarises_too() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": A_LONG_MEMORY,
            "keywords": ["hoard-signpost"],
            "confidence": 0.9,
            "origin": "llm"
        }),
    );
    assert!(
        !is_error(&response),
        "memorize failed: {}",
        message(&response)
    );

    let memories = summary_recall(
        &mut server,
        json!({"keywords": ["hoard-signpost"], "detail": "summary"}),
    );

    assert_eq!(memories.len(), 1);
    assert_eq!(
        memories[0]["summary"],
        json!("Push is the marked case, and most accumulation stays passive.\u{2026}")
    );
    assert!(
        memories[0].get("matched_keywords").is_none(),
        "the match count is part of the record, not of the index: {}",
        memories[0]
    );
}

/// A caller that says nothing gets the whole record. The reduced form has to be asked for, because
/// a caller that did not choose it cannot know what it gave up.
#[test]
fn the_full_record_is_what_an_unasked_recall_returns() {
    let mut server = a_server();
    memorize(&mut server, A_LONG_MEMORY);

    for arguments in [
        json!({"query": "what push costs"}),
        json!({"query": "what push costs", "detail": "full"}),
    ] {
        let memories = summary_recall(&mut server, arguments.clone());
        assert!(
            memories[0].get("content").is_some() && memories[0].get("confidence").is_some(),
            "{arguments} lost the record: {}",
            memories[0]
        );
    }
}

#[test]
fn an_unknown_detail_level_is_refused_by_name() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_recall",
        json!({"query": "anything", "detail": "brief"}),
    );

    assert!(is_error(&response), "an invented level should be refused");
    let message = message(&response);
    assert!(
        message.contains("brief") && message.contains("summary"),
        "the refusal should name what was sent and what is accepted: {message}"
    );
}

/// The echo is what keeps the cap from repeating the mistake it exists to fix: a character cap is
/// invisible at drafting time, so the line is shown at the moment of writing rather than left to be
/// measured. Compared against a recall rather than against a literal -- the two must not be able to
/// disagree about what the line is.
#[test]
fn the_store_response_echoes_the_line_a_summary_recall_would_produce() {
    let mut server = a_server();
    let stored = payload(&memorize(&mut server, A_LONG_MEMORY));

    let memories = summary_recall(
        &mut server,
        json!({"query": "what push costs", "detail": "summary"}),
    );

    assert!(
        stored["summary"]
            .as_str()
            .is_some_and(|line| !line.is_empty()),
        "the store response carries no line: {stored}"
    );
    assert_eq!(stored["summary"], memories[0]["summary"]);
    assert_eq!(stored["id"], memories[0]["id"]);
}

/// Always, not only on a memory that will be indexed. Making it conditional on a keyword would
/// couple the response to `hoard-watch`, which is an open defect whose likely fix changes that
/// keyword -- and would confine the reflection to the one moment the author is already careful.
#[test]
fn an_ordinary_memory_gets_the_echo_as_well() {
    let mut server = a_server();
    let stored = payload(&memorize(&mut server, "Push is the marked case."));

    assert_eq!(stored["summary"], json!("Push is the marked case."));
}

// ---------------------------------------------------------------------------
// An identity record is recognised by its shape, not by a keyword anyone can write.
// ---------------------------------------------------------------------------

/// `about-hoard-author-as-a-subject-keyword-silently-disables-embedding`.
///
/// The embed filter used to ask *does this carry `hoard-author`*, so an ordinary memory that merely
/// **discussed** identity records was stored with no vector and left semantic search without a
/// word — and the note beside `findable_by_meaning: false` blamed a missing model, sending the
/// caller to check an installation that was fine.
#[test]
fn a_memory_that_merely_mentions_the_author_keyword_is_still_embedded() {
    let mut server = a_server();
    let stored = payload(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha alpha: identity records are never embedded, which is why a bare \
                        name matches nothing",
            "keywords": ["hoard-author", "embedding"],
            "confidence": 0.9,
            "origin": "llm"
        }),
    ));

    assert_eq!(
        stored["findable_by_meaning"],
        json!(true),
        "a memory about identity records is not one: {stored}"
    );
    let found = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha alpha", "limit": 5}),
    ));
    assert_eq!(
        found["memories"][0]["id"], stored["id"],
        "it should be reachable by meaning: {found}"
    );
}

// ---------------------------------------------------------------------------
// The non-match floor, at the tool surface
// ---------------------------------------------------------------------------

/// A server whose set already carries a calibrated floor, set before the store is handed over.
fn a_server_with_a_floor(floor: f64) -> Server {
    let mut store = chartered(scope());
    store.set_embedder(Arc::new(MarkerEmbedder));
    store.set_floor(floor).expect("sets the floor");

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(store);
    server
}

fn memorize_alpha(server: &mut Server) {
    call(
        server,
        "hoard_memorize",
        json!({
            "scope": "shared:scatterhoard", "content": "alpha alpha", "why": "a fixture",
            "confidence": 0.8, "origin": "llm"
        }),
    );
}

/// **Silence and *nothing matched* are the same empty result**, so the caller is told which it is —
/// and told with the way round it, since a report it cannot act on merely says the question failed.
#[test]
fn a_recall_silenced_by_the_floor_says_so_and_names_the_way_round_it() {
    let mut server = a_server_with_a_floor(0.9);
    memorize_alpha(&mut server);

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "gamma", "scope": "shared:scatterhoard"}),
    ));

    assert_eq!(result["memories"].as_array().expect("an array").len(), 0);
    let note = result["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("ignore_floor"),
        "a silenced recall must name the parameter that asks again: {note}"
    );
    // `the-silenced-recall-note-carries-source-indentation`: checked as served, beside the source
    // check in `tests/served_text.rs` that covers every literal.
    assert!(
        !note.contains("  "),
        "the note must read as one line of prose, not carry source indentation: {note:?}"
    );
}

#[test]
fn asking_again_without_the_floor_returns_what_it_silenced() {
    let mut server = a_server_with_a_floor(0.9);
    memorize_alpha(&mut server);

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "gamma", "scope": "shared:scatterhoard", "ignore_floor": true}),
    ));

    assert_eq!(
        result["memories"].as_array().expect("an array").len(),
        1,
        "asking without the floor returned nothing"
    );
    assert!(
        result.get("note").is_none(),
        "nothing was silenced this time"
    );
}

/// A query the set answers well is not silenced, and says nothing about a floor it cleared.
#[test]
fn a_recall_above_the_floor_reports_nothing() {
    let mut server = a_server_with_a_floor(0.9);
    memorize_alpha(&mut server);

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha alpha", "scope": "shared:scatterhoard"}),
    ));

    assert_eq!(result["memories"].as_array().expect("an array").len(), 1);
    assert!(result.get("note").is_none());
}

/// A server with one set carrying `floor` and holding *alpha*, and `local`, with no floor, holding
/// *gamma* -- so a question about gamma is one the floored set declines and only `local` answers.
fn a_floored_set_beside_one_without(floor: Option<f64>) -> Server {
    let mut shared = chartered(scope());
    shared.set_embedder(Arc::new(MarkerEmbedder));
    if let Some(floor) = floor {
        shared.set_floor(floor).expect("sets the floor");
    }
    let mut local = Store::open_in_memory(Scope::Local).expect("opens");
    local.set_embedder(Arc::new(MarkerEmbedder));

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(shared);
    server.add_store(local);
    memorize_alpha(&mut server);
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": "gamma gamma", "why": "a fixture",
            "confidence": 0.8, "origin": "llm"
        }),
    );
    server
}

/// **When every set with a floor declines a question, the hits that remain are marked.**
/// `note-when-only-uncalibrated-sets-answer`, the fix for `an-unanswerable-query-returns-confident-
/// hits`: a set with no floor has nothing that separates a non-match from an answer, and on an
/// unanswerable question its everyday background becomes the whole answer. Marked rather than
/// withheld, because the hit may be a real answer and the caller can re-ask with a `scope`.
#[test]
fn hits_only_from_sets_without_a_floor_are_marked() {
    let mut server = a_floored_set_beside_one_without(Some(0.9));

    let result = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "gamma"}),
    ));

    assert!(
        !result["memories"].as_array().expect("an array").is_empty(),
        "the set without a floor should still answer: {result}"
    );
    assert_eq!(
        result["answered_only_without_a_floor"],
        json!(["local"]),
        "{result}"
    );
    let note = result["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("scope"),
        "the note should name the way to search one set: {note}"
    );
}

/// The paired must-not-fire cases: a set with a floor answered too, no set has a floor, or the
/// caller waived the floors -- in each, nothing was declined by a floor that the remaining hits
/// stand in for.
#[test]
fn hits_are_not_marked_unless_every_floored_set_declined() {
    for (why, floor, arguments) in [
        (
            "a floored set answered",
            Some(0.9),
            json!({"query": "alpha alpha gamma"}),
        ),
        ("no set has a floor", None, json!({"query": "gamma"})),
        (
            "the floors were waived",
            Some(0.9),
            json!({"query": "gamma", "ignore_floor": true}),
        ),
    ] {
        let mut server = a_floored_set_beside_one_without(floor);
        let result = payload(&call(&mut server, "hoard_recall", arguments));
        let scopes: Vec<&str> = result["memories"]
            .as_array()
            .expect("an array")
            .iter()
            .filter_map(|hit| hit["scope"].as_str())
            .collect();
        assert!(
            scopes.contains(&"local"),
            "{why}: the case needs a hit from the set without a floor: {result}"
        );
        if floor.is_some() && why == "a floored set answered" {
            assert!(
                scopes.contains(&"shared:scatterhoard"),
                "{why}: the case needs a hit from the set with a floor: {result}"
            );
        }
        assert!(
            result.get("answered_only_without_a_floor").is_none(),
            "{why}: {result}"
        );
    }
}

// ---------------------------------------------------------------------------
// A server older than the binary it was launched from
// ---------------------------------------------------------------------------

/// A file standing in for the server's own binary, so a test can rebuild it.
fn a_fake_binary(content: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("scatterhoard-binary-{nanos}"));
    std::fs::write(&path, content).expect("writes");
    path
}

/// **A client serves whatever binary it launched with, so rebuilding changes nothing a running
/// session sees** — and the tell everyone reaches for first cannot fire. On 2026-09-07 a session
/// loaded a day-old catalog holding the right *number* of conventions and the wrong *text*, and
/// reasoned from it; a count moves only when a convention is added or removed, and edits are the
/// common case.
///
/// Reported on `hoard_conventions` because that is the call a session makes first, and because a
/// stale catalog is the worst of the failures: it answers every call successfully and simply
/// governs.
#[test]
fn a_server_launched_from_a_rebuilt_binary_says_so() {
    let binary = a_fake_binary("the build this server started from");
    let mut server = a_server();
    server.set_launched_from(&binary);

    let (_, before, _) = conventions_page(&call(&mut server, "hoard_conventions", json!({})));
    assert!(
        before.get("note").is_none(),
        "an unchanged binary should say nothing: {before}"
    );

    std::fs::write(&binary, "a newer build, of a different length entirely").expect("rebuilds");

    let (_, after, _) = conventions_page(&call(&mut server, "hoard_conventions", json!({})));
    let note = after["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("restart"),
        "a rebuilt binary should name the remedy, and said: {note:?}"
    );
    let _ = std::fs::remove_file(&binary);
}

/// **A session can say which build it is talking to**, for a bug report or to notice it differs from
/// a tester's. On the first page because that is the one call every session makes, and once rather
/// than on every page, as the other asides are.
#[test]
fn the_first_conventions_page_names_the_server_version() {
    let mut server = a_server();
    let (_, aside, _) = conventions_page(&call(&mut server, "hoard_conventions", json!({})));
    assert_eq!(
        aside["server_version"],
        json!(env!("CARGO_PKG_VERSION")),
        "{aside}"
    );
}

#[test]
fn a_continuation_page_does_not_repeat_the_server_version() {
    let mut server = a_server();
    let (_, _, last) = conventions_page(&call(
        &mut server,
        "hoard_conventions",
        json!({"max_chars": 3500}),
    ));
    let cursor: u64 = last
        .split("\"cursor\": ")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_else(|| panic!("the first page should say where the next starts: {last}"));
    let (_, aside, _) = conventions_page(&call(
        &mut server,
        "hoard_conventions",
        json!({"cursor": cursor, "max_chars": 3500}),
    ));
    assert!(aside.get("server_version").is_none(), "{aside}");
}

/// The stale-binary note names the version being served, so a session reporting it can say which.
#[test]
fn the_stale_binary_note_names_the_version_being_served() {
    let binary = a_fake_binary("the build this server started from");
    let mut server = a_server();
    server.set_launched_from(&binary);
    std::fs::write(&binary, "a newer build, of a different length entirely").expect("rebuilds");

    let (_, aside, _) = conventions_page(&call(&mut server, "hoard_conventions", json!({})));
    let note = aside["note"].as_str().unwrap_or_default();
    assert!(note.contains(env!("CARGO_PKG_VERSION")), "{note}");
    let _ = std::fs::remove_file(&binary);
}

/// The conventions still arrive. A session that cannot read them is worse off than one reading old
/// ones, so this reports beside the answer rather than in place of it.
#[test]
fn a_stale_server_still_serves_its_conventions() {
    let binary = a_fake_binary("the build this server started from");
    let mut server = a_server();
    server.set_launched_from(&binary);
    std::fs::write(&binary, "a newer build, of a different length entirely").expect("rebuilds");

    let (page, _, _) = conventions_page(&call(&mut server, "hoard_conventions", json!({})));
    assert!(
        !page["conventions"].as_array().expect("an array").is_empty(),
        "the catalog must still be served"
    );
    let _ = std::fs::remove_file(&binary);
}
