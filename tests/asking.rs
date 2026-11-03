// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Asking the human: protocol revision 2026-07-28 served beside the handshake revisions, and the
//! elicitation form through which a shared set's publishing name reaches the server without the
//! model supplying it. `documents/design/asking-the-human.md`, `asking-the-human` in `TRACKING.md`.

use std::sync::Arc;

use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::mcp::Server;
use scatterhoard::memory::{Author, Scope};
use scatterhoard::store::{Store, StoreLayout};
use serde_json::{Value, json};

#[path = "support/review_all.rs"]
mod review_all;

#[path = "support/output_schema_check.rs"]
mod output_schema_check;

const FIXTURE_CHARTER: &str = "This set holds the fixtures these tests write.";

const MODERN: &str = "2026-07-28";

#[derive(Debug)]
struct MarkerEmbedder;

impl Embedder for MarkerEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("marker-embedder")
    }

    fn dimension(&self) -> usize {
        3
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|text| {
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

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tempdir() -> TempDir {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_nanos();
    let unique = format!("{nanos}-{:?}", std::thread::current().id())
        .replace(['(', ')', ' '], "")
        .replace("ThreadId", "t");
    let mut path = std::env::temp_dir();
    path.push(format!("scatterhoard-asking-{unique}"));
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

fn a_server(home: &TempDir) -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    server
}

fn send(server: &mut Server, request: &Value) -> Value {
    if request["params"]["name"] == json!("hoard_sets") {
        review_all::review_before_export(server, &request["params"]["arguments"]);
    }
    let raw = server
        .handle(&request.to_string())
        .expect("a request with an id must be answered");
    serde_json::from_str(&raw).expect("the response should be JSON")
}

/// A client that can show the human a form.
fn form() -> Value {
    json!({"elicitation": {"form": {}}})
}

/// A modern request: no handshake, the version and the client's capabilities on the request itself.
fn modern(server: &mut Server, method: &str, mut params: Value, capabilities: Value) -> Value {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientInfo": {"name": "asking-tests", "version": "0"},
        "io.modelcontextprotocol/clientCapabilities": capabilities,
    });
    send(
        server,
        &json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params}),
    )
}

/// A modern `tools/call`, carrying the human's answers where `responses` gives some.
fn modern_call(
    server: &mut Server,
    tool: &str,
    arguments: Value,
    capabilities: Value,
    responses: Option<Value>,
) -> Value {
    let mut params = json!({"name": tool, "arguments": arguments});
    if let Some(responses) = responses {
        params["inputResponses"] = responses;
    }
    let response = modern(server, "tools/call", params, capabilities);
    if response["result"]["resultType"] == json!("complete") {
        output_schema_check::check_response(tool, &response);
    }
    response
}

/// A legacy `tools/call`, as every client before 2026-07-28 sends one.
fn legacy_call(server: &mut Server, tool: &str, arguments: Value) -> Value {
    send(
        server,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}
        }),
    )
}

fn is_error(response: &Value) -> bool {
    response.get("error").is_some() || response["result"]["isError"] == json!(true)
}

fn message(response: &Value) -> String {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn payload(response: &Value) -> Value {
    serde_json::from_str(&message(response)).expect("the tool payload should be JSON")
}

fn create(server: &mut Server, name: &str) {
    let created = legacy_call(
        server,
        "hoard_sets",
        json!({"action": "create", "name": name}),
    );
    assert!(!is_error(&created), "{created}");
}

fn charter_arguments(set: &str) -> Value {
    json!({"scope": {"shared": set}, "content": FIXTURE_CHARTER, "why": "a fixture",
           "confidence": 1.0, "origin": "human",
           "keywords": ["hoard-scope-topics", "hoard-lang-en"]})
}

fn status_of(server: &mut Server, set: &str) -> Value {
    let status = payload(&legacy_call(
        server,
        "hoard_sets",
        json!({"action": "status"}),
    ));
    status["sets"]
        .as_array()
        .expect("a list of sets")
        .iter()
        .find(|entry| entry["scope"] == json!(format!("shared:{set}")))
        .unwrap_or_else(|| panic!("{set} is loaded: {status}"))
        .clone()
}

/// The human's answer to the name question.
fn named(name: &str) -> Value {
    json!({"publishing_name": {"action": "accept", "content": {"name": name}}})
}

// ---------------------------------------------------------------------------
// The two eras, side by side
// ---------------------------------------------------------------------------

/// **A handshake client is served exactly as before.** Every client that works today keeps working.
#[test]
fn initialize_still_selects_the_handshake_revisions() {
    let home = tempdir();
    let mut server = a_server(&home);

    let initialized = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {}}}),
    );

    assert_eq!(
        initialized["result"]["protocolVersion"],
        json!("2025-06-18")
    );
    assert!(
        initialized["result"].get("resultType").is_none(),
        "{initialized}"
    );
}

/// **`server/discover` is how a client that speaks both eras finds a modern server**, and a modern
/// server must answer it: its versions, its capabilities, and how long the answer stays fresh.
#[test]
fn discover_names_the_modern_revision_and_its_capabilities() {
    let home = tempdir();
    let mut server = a_server(&home);

    let discovered = modern(&mut server, "server/discover", json!({}), json!({}));

    let result = &discovered["result"];
    assert_eq!(result["resultType"], json!("complete"), "{discovered}");
    assert_eq!(result["supportedVersions"], json!([MODERN]), "{discovered}");
    assert!(result["capabilities"]["tools"].is_object(), "{discovered}");
    assert!(result["ttlMs"].as_u64().is_some(), "{discovered}");
    assert_eq!(result["cacheScope"], json!("public"), "{discovered}");
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        json!("scatterhoard"),
        "{discovered}"
    );
}

/// **Without the modern metadata, a probe reads as a legacy server's unknown method**, which is
/// what sends a client that speaks both eras back to `initialize`.
#[test]
fn discover_without_the_modern_metadata_is_an_unknown_method() {
    let home = tempdir();
    let mut server = a_server(&home);

    let probed = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {}}),
    );

    assert_eq!(probed["error"]["code"], json!(-32601), "{probed}");
}

/// **A modern version this server does not speak is refused with the versions it does**, so the
/// client can retry with one of them rather than fall back.
#[test]
fn an_unsupported_modern_version_is_refused_with_the_supported_ones() {
    let home = tempdir();
    let mut server = a_server(&home);

    let refused = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": {
            "io.modelcontextprotocol/protocolVersion": "2099-01-01",
            "io.modelcontextprotocol/clientCapabilities": {},
        }}}),
    );

    assert_eq!(refused["error"]["code"], json!(-32022), "{refused}");
    assert_eq!(
        refused["error"]["data"]["requested"],
        json!("2099-01-01"),
        "{refused}"
    );
    assert!(
        refused["error"]["data"]["supported"]
            .as_array()
            .is_some_and(|versions| versions.contains(&json!(MODERN))),
        "{refused}"
    );
}

/// **A modern request without the client's capabilities is malformed**: the server would otherwise
/// have to guess whether it may ask the human anything.
#[test]
fn a_modern_request_without_capabilities_is_invalid() {
    let home = tempdir();
    let mut server = a_server(&home);

    let refused = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": {
            "io.modelcontextprotocol/protocolVersion": MODERN,
        }}}),
    );

    assert_eq!(refused["error"]["code"], json!(-32602), "{refused}");
}

/// **The modern tool list carries its result type, its caching hints and its output schemas.**
#[test]
fn the_modern_tool_list_is_cacheable_and_publishes_output_schemas() {
    let home = tempdir();
    let mut server = a_server(&home);

    let listed = modern(&mut server, "tools/list", json!({}), json!({}));

    let result = &listed["result"];
    assert_eq!(result["resultType"], json!("complete"), "{listed}");
    assert!(result["ttlMs"].as_u64().is_some(), "{listed}");
    assert_eq!(result["cacheScope"], json!("public"), "{listed}");
    let tools = result["tools"].as_array().expect("a list of tools");
    let recall = tools
        .iter()
        .find(|tool| tool["name"] == json!("hoard_recall"))
        .expect("recall is listed");
    assert!(recall.get("outputSchema").is_some(), "{recall}");
}

/// **A modern request never reads what an earlier handshake set.** A handshake on 2024-11-05 turns
/// output schemas off for that client; a modern request on the same process still gets them.
#[test]
fn a_modern_request_ignores_an_earlier_handshake() {
    let home = tempdir();
    let mut server = a_server(&home);
    send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05", "capabilities": {}}}),
    );

    let listed = modern(&mut server, "tools/list", json!({}), json!({}));

    let recall = listed["result"]["tools"]
        .as_array()
        .expect("a list of tools")
        .iter()
        .find(|tool| tool["name"] == json!("hoard_recall"))
        .expect("recall is listed")
        .clone();
    assert!(recall.get("outputSchema").is_some(), "{recall}");
}

/// **The revision removes `ping`**; only the handshake side answers it.
#[test]
fn the_modern_revision_has_no_ping() {
    let home = tempdir();
    let mut server = a_server(&home);

    let pinged = modern(&mut server, "ping", json!({}), json!({}));

    assert_eq!(pinged["error"]["code"], json!(-32601), "{pinged}");
}

/// **A tool's results, including its failures, are complete results on the modern side.**
#[test]
fn a_modern_tool_call_returns_a_complete_result() {
    let home = tempdir();
    let mut server = a_server(&home);

    let status = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "status"}),
        json!({}),
        None,
    );
    let failed = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "status", "scope": "shared:nothing-here"}),
        json!({}),
        None,
    );

    assert_eq!(
        status["result"]["resultType"],
        json!("complete"),
        "{status}"
    );
    assert!(!is_error(&status), "{status}");
    assert_eq!(
        failed["result"]["resultType"],
        json!("complete"),
        "{failed}"
    );
}

// ---------------------------------------------------------------------------
// Asking for the name at a shared set's first write
// ---------------------------------------------------------------------------

/// **Where the client can show a form, the first write asks the human and writes nothing yet.**
#[test]
fn a_first_write_asks_the_human_for_their_name() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let asked = modern_call(
        &mut server,
        "hoard_memorize",
        charter_arguments("fresh"),
        form(),
        None,
    );

    let result = &asked["result"];
    assert_eq!(result["resultType"], json!("input_required"), "{asked}");
    let question = &result["inputRequests"]["publishing_name"];
    assert_eq!(question["method"], json!("elicitation/create"), "{asked}");
    assert_eq!(question["params"]["mode"], json!("form"), "{asked}");
    let field = &question["params"]["requestedSchema"]["properties"]["name"];
    assert_eq!(field["type"], json!("string"), "{asked}");
    assert!(
        field["description"]
            .as_str()
            .is_some_and(|text| text.contains("hosted under")),
        "the hint goes to the human: {asked}"
    );
    assert_eq!(
        question["params"]["requestedSchema"]["required"],
        json!(["name"])
    );
    assert!(
        result.get("ttlMs").is_none(),
        "an interim result is not cacheable"
    );
    assert_eq!(status_of(&mut server, "fresh")["memories"], json!(0));
}

/// **The retry carrying the human's answer mints their identity and completes the write.**
#[test]
fn the_answer_mints_the_identity_and_completes_the_write() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let written = modern_call(
        &mut server,
        "hoard_memorize",
        charter_arguments("fresh"),
        form(),
        Some(named("published-name")),
    );

    assert!(!is_error(&written), "{}", message(&written));
    assert_eq!(written["result"]["resultType"], json!("complete"));
    assert_eq!(
        status_of(&mut server, "fresh")["author"],
        json!("published-name")
    );
}

/// **A decline writes nothing and says it was the human's choice**, so the session does not ask
/// again in the same breath or reach for another name.
#[test]
fn a_declined_question_writes_nothing() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let declined = modern_call(
        &mut server,
        "hoard_memorize",
        charter_arguments("fresh"),
        form(),
        Some(json!({"publishing_name": {"action": "decline"}})),
    );

    assert!(is_error(&declined), "{declined}");
    let text = message(&declined);
    assert!(text.contains("declined"), "{declined}");
    assert!(
        text.contains("a client that cannot show forms"),
        "a decline is not blamed on the human, who may never have seen it: {text}"
    );
    assert!(
        text.contains("as `author` to `hoard_sets` action \"mint\""),
        "says how to proceed: {text}"
    );
    assert_eq!(status_of(&mut server, "fresh")["memories"], json!(0));
}

/// **A cancel writes nothing, and the next write asks again**, since nothing was answered.
#[test]
fn a_cancelled_question_writes_nothing_and_will_be_asked_again() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let cancelled = modern_call(
        &mut server,
        "hoard_memorize",
        charter_arguments("fresh"),
        form(),
        Some(json!({"publishing_name": {"action": "cancel"}})),
    );
    let again = modern_call(
        &mut server,
        "hoard_memorize",
        charter_arguments("fresh"),
        form(),
        None,
    );

    assert!(is_error(&cancelled), "{cancelled}");
    assert!(message(&cancelled).contains("dismissed"), "{cancelled}");
    assert_eq!(again["result"]["resultType"], json!("input_required"));
    assert_eq!(status_of(&mut server, "fresh")["memories"], json!(0));
}

/// **An empty answer is asked again rather than recorded**, saying why.
#[test]
fn an_unusable_answer_is_asked_again() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let again = modern_call(
        &mut server,
        "hoard_memorize",
        charter_arguments("fresh"),
        form(),
        Some(named("   ")),
    );

    assert_eq!(
        again["result"]["resultType"],
        json!("input_required"),
        "{again}"
    );
    assert!(
        again["result"]["inputRequests"]["publishing_name"]["params"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("could not be used")),
        "{again}"
    );
    assert_eq!(status_of(&mut server, "fresh")["memories"], json!(0));
}

/// **An answer is read only from where the client puts it**, never from the model's arguments: a
/// model that wrote one into its own call must not be mistaken for the human answering.
#[test]
fn an_answer_in_the_arguments_is_not_the_humans() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");
    let mut arguments = charter_arguments("fresh");
    arguments["inputResponses"] = named("acme-corp");

    let asked = modern_call(&mut server, "hoard_memorize", arguments, form(), None);

    assert_eq!(
        asked["result"]["resultType"],
        json!("input_required"),
        "{asked}"
    );
    assert_eq!(status_of(&mut server, "fresh")["memories"], json!(0));
}

/// **A client that cannot show a form gets the fallback refusal**: the specification forbids asking
/// a client for what it has not declared, and URL mode is not a form.
#[test]
fn a_client_without_form_elicitation_gets_the_refusal() {
    for capabilities in [json!({}), json!({"elicitation": {"url": {}}})] {
        let home = tempdir();
        let mut server = a_server(&home);
        create(&mut server, "fresh");

        let refused = modern_call(
            &mut server,
            "hoard_memorize",
            charter_arguments("fresh"),
            capabilities.clone(),
            None,
        );

        assert_eq!(refused["result"]["resultType"], json!("complete"));
        assert!(is_error(&refused), "{capabilities}: {refused}");
        assert!(
            message(&refused).contains("only the human can give you"),
            "{capabilities}: {refused}"
        );
    }
}

/// **An empty `elicitation` object declares form mode**, for backwards compatibility.
#[test]
fn an_empty_elicitation_capability_means_form() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let asked = modern_call(
        &mut server,
        "hoard_memorize",
        charter_arguments("fresh"),
        json!({"elicitation": {}}),
        None,
    );

    assert_eq!(
        asked["result"]["resultType"],
        json!("input_required"),
        "{asked}"
    );
}

/// **A handshake connection is never sent a form**, whatever it declared: on those revisions the
/// question would travel as a request from the server, which this server does not send.
#[test]
fn a_handshake_connection_is_never_asked() {
    let home = tempdir();
    let mut server = a_server(&home);
    send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18",
                           "capabilities": {"elicitation": {}}}}),
    );
    create(&mut server, "fresh");

    let refused = legacy_call(&mut server, "hoard_memorize", charter_arguments("fresh"));

    assert!(is_error(&refused), "{refused}");
    assert!(refused["result"].get("resultType").is_none(), "{refused}");
    assert!(
        message(&refused).contains("only the human can give you"),
        "{refused}"
    );
}

/// **`local` is never published and has nothing to ask.**
#[test]
fn local_is_not_asked() {
    let home = tempdir();
    let mut server = a_server(&home);

    let stored = modern_call(
        &mut server,
        "hoard_memorize",
        json!({"scope": "local", "content": "alpha, a personal note", "why": "a fixture",
               "confidence": 0.7, "origin": "llm"}),
        form(),
        None,
    );

    assert!(!is_error(&stored), "{stored}");
    assert_eq!(stored["result"]["resultType"], json!("complete"));
}

/// **Nor is `local` asked where the system supplies no account name**, which is the only route on
/// which it could be: with a name it takes that, and never reaches the question at all. It is
/// refused as before, and `mint` there takes its name as an argument.
#[test]
fn local_without_an_account_name_is_not_asked_either() {
    let home = tempdir();
    let mut server = Server::new();
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));

    let refused = modern_call(
        &mut server,
        "hoard_memorize",
        json!({"scope": "local", "content": "alpha, a personal note", "why": "a fixture",
               "confidence": 0.7, "origin": "llm"}),
        form(),
        None,
    );
    let minted = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "local", "author": "chosen-name"}),
        form(),
        None,
    );

    assert_eq!(
        refused["result"]["resultType"],
        json!("complete"),
        "{refused}"
    );
    assert!(is_error(&refused), "{refused}");
    assert!(!is_error(&minted), "{minted}");
    assert_eq!(payload(&minted)["author"], json!("chosen-name"));
}

/// **Where the human will be asked, nothing tells the model to ask them itself.** `create`'s note
/// and `status`'s author note say the first write asks; the handshake side still says to `mint`.
#[test]
fn the_notes_say_the_first_write_asks_where_it_will() {
    let home = tempdir();
    let mut server = a_server(&home);

    let created = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "create", "name": "fresh"}),
        form(),
        None,
    );
    let status = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "status"}),
        form(),
        None,
    );
    let legacy_status = payload(&legacy_call(
        &mut server,
        "hoard_sets",
        json!({"action": "status"}),
    ));

    let note = payload(&created)["note"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(note.contains("asks the human"), "{note}");
    assert!(!note.contains("\"mint\""), "{note}");
    let entry_of = |status: &Value| {
        status["sets"]
            .as_array()
            .expect("a list")
            .iter()
            .find(|set| set["scope"] == json!("shared:fresh"))
            .expect("the set")["author_note"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    let author_note = entry_of(&payload(&status));
    assert!(author_note.contains("asks the human"), "{author_note}");
    assert!(
        entry_of(&legacy_status).contains("\"mint\""),
        "{legacy_status}"
    );
}

// ---------------------------------------------------------------------------
// The author actions
// ---------------------------------------------------------------------------

/// **Where the human can be asked, `mint` asks, and a name the model supplied is not used.**
#[test]
fn mint_asks_the_human_and_sets_aside_the_models_name() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");
    let arguments = json!({"action": "mint", "scope": "shared:fresh", "author": "acme-corp"});

    let asked = modern_call(&mut server, "hoard_sets", arguments.clone(), form(), None);
    let minted = modern_call(
        &mut server,
        "hoard_sets",
        arguments,
        form(),
        Some(named("real-name")),
    );

    assert_eq!(
        asked["result"]["resultType"],
        json!("input_required"),
        "{asked}"
    );
    assert!(!is_error(&minted), "{}", message(&minted));
    let result = payload(&minted);
    assert_eq!(result["author"], json!("real-name"), "{result}");
    assert!(
        !result["note"]
            .as_str()
            .unwrap_or_default()
            .contains("End your reply"),
        "a name the human typed needs no relaying back to them: {result}"
    );
}

/// **`mint` needs no `author` on a modern connection**, since the human supplies it.
#[test]
fn mint_without_an_author_asks() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let asked = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:fresh"}),
        form(),
        None,
    );

    assert_eq!(
        asked["result"]["resultType"],
        json!("input_required"),
        "{asked}"
    );
}

/// **A `mint` that would be refused is refused before the human is asked**, so nobody fills in a
/// form for nothing.
#[test]
fn a_mint_that_would_be_refused_does_not_ask() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");
    modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:fresh"}),
        form(),
        Some(named("first-name")),
    );

    let again = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:fresh"}),
        form(),
        None,
    );

    assert!(is_error(&again), "{again}");
    assert_eq!(again["result"]["resultType"], json!("complete"), "{again}");
}

/// **`rename` asks for the new name the same way.**
#[test]
fn rename_asks_the_human_for_the_new_name() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");
    modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:fresh"}),
        form(),
        Some(named("first-name")),
    );
    let arguments = json!({"action": "rename", "scope": "shared:fresh"});

    let asked = modern_call(&mut server, "hoard_sets", arguments.clone(), form(), None);
    let renamed = modern_call(
        &mut server,
        "hoard_sets",
        arguments,
        form(),
        Some(named("second-name")),
    );

    assert_eq!(
        asked["result"]["resultType"],
        json!("input_required"),
        "{asked}"
    );
    assert!(!is_error(&renamed), "{}", message(&renamed));
    assert_eq!(payload(&renamed)["author"], json!("second-name"));
}

/// **A `rename` with nothing to rename is refused before asking.**
#[test]
fn a_rename_with_no_identity_does_not_ask() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let refused = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "rename", "scope": "shared:fresh"}),
        form(),
        None,
    );

    assert!(is_error(&refused), "{refused}");
    assert_eq!(refused["result"]["resultType"], json!("complete"));
}

/// **On a handshake connection the actions still take the name from the model**, as the fallback
/// decided 2026-10-11, and still relay it back.
#[test]
fn a_handshake_mint_still_takes_the_name_and_relays_it() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let missing = legacy_call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:fresh"}),
    );
    let minted = legacy_call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:fresh", "author": "given-name"}),
    );

    assert!(is_error(&missing), "{missing}");
    assert!(
        payload(&minted)["note"]
            .as_str()
            .is_some_and(|note| note.starts_with("End your reply")),
        "{minted}"
    );
}

/// **The modern schema leaves `author` and `record` optional, and says why**; the handshake
/// schema still requires them.
#[test]
fn the_modern_schema_makes_the_humans_answers_optional() {
    let home = tempdir();
    let mut server = a_server(&home);

    let modern_list = modern(&mut server, "tools/list", json!({}), json!({}));
    let legacy_list = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );

    let action_text = |listed: &Value| {
        listed["result"]["tools"]
            .as_array()
            .expect("a list of tools")
            .iter()
            .find(|tool| tool["name"] == json!("hoard_sets"))
            .expect("hoard_sets is listed")["inputSchema"]["properties"]["action"]["description"]
            .as_str()
            .expect("a description")
            .to_owned()
    };
    let modern_text = action_text(&modern_list);
    let legacy_text = action_text(&legacy_list);
    assert!(
        modern_text.contains("`mint` takes scope, and optionally author"),
        "{modern_text}"
    );
    assert!(
        modern_text.contains("`identify` takes scope, and optionally record"),
        "{modern_text}"
    );
    assert!(
        legacy_text.contains("`mint` takes scope and author"),
        "{legacy_text}"
    );
}

// ---------------------------------------------------------------------------
// Choosing among identity records already in the set
// ---------------------------------------------------------------------------

/// A set taken on from someone else: it holds their identity record, and this machine has not said
/// which record is it.
fn a_taken_on_set(home: &TempDir, repository: &TempDir) -> Server {
    {
        let author_home = tempdir();
        let mut authoring = a_server(&author_home);
        create(&mut authoring, "taken-on");
        let first = modern_call(
            &mut authoring,
            "hoard_memorize",
            charter_arguments("taken-on"),
            form(),
            Some(named("someone-else")),
        );
        assert!(!is_error(&first), "{first}");
        let exported = legacy_call(
            &mut authoring,
            "hoard_sets",
            json!({"action": "export", "scope": "shared:taken-on",
                   "directory": repository.path()}),
        );
        assert!(!is_error(&exported), "{exported}");
    }

    let mut server = a_server(home);
    create(&mut server, "taken-on");
    let imported = legacy_call(
        &mut server,
        "hoard_sets",
        json!({"action": "import", "scope": "shared:taken-on",
               "directory": repository.path()}),
    );
    assert!(!is_error(&imported), "{imported}");
    server
}

fn note_arguments(set: &str) -> Value {
    json!({"scope": {"shared": set}, "content": "alpha, a note", "why": "a fixture",
           "confidence": 0.7, "origin": "llm"})
}

/// **A write to a set that already holds identity records asks the human which is theirs**,
/// offering each record by name and a way to say they are new -- rather than having the model pick
/// which existing person the human is.
#[test]
fn a_write_to_a_taken_on_set_asks_which_identity_is_theirs() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_taken_on_set(&home, &repository);

    let asked = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("taken-on"),
        form(),
        None,
    );

    assert_eq!(
        asked["result"]["resultType"],
        json!("input_required"),
        "{asked}"
    );
    let schema = &asked["result"]["inputRequests"]["which_identity"]["params"]["requestedSchema"];
    let choices = schema["properties"]["record"]["oneOf"]
        .as_array()
        .expect("the records, offered by name");
    assert!(
        choices
            .iter()
            .any(|choice| choice["title"] == json!("someone-else")),
        "{asked}"
    );
    assert!(
        choices.iter().any(|choice| choice["const"] == json!("new")),
        "{asked}"
    );
}

/// **Choosing a record identifies this machine with it, and the write completes under it.**
#[test]
fn choosing_a_record_completes_the_write_under_it() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_taken_on_set(&home, &repository);
    let asked = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("taken-on"),
        form(),
        None,
    );
    let record = asked["result"]["inputRequests"]["which_identity"]["params"]["requestedSchema"]
        ["properties"]["record"]["oneOf"]
        .as_array()
        .expect("choices")
        .iter()
        .find(|choice| choice["title"] == json!("someone-else"))
        .expect("their record")["const"]
        .clone();

    let written = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("taken-on"),
        form(),
        Some(json!({"which_identity": {"action": "accept", "content": {"record": record}}})),
    );

    assert!(!is_error(&written), "{}", message(&written));
    assert_eq!(
        status_of(&mut server, "taken-on")["author"],
        json!("someone-else")
    );
}

/// **Saying they are new mints a record under the name they give**, in the same form.
#[test]
fn saying_they_are_new_mints_under_the_name_given() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_taken_on_set(&home, &repository);

    let written = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("taken-on"),
        form(),
        Some(json!({"which_identity": {"action": "accept",
                                       "content": {"record": "new", "name": "newcomer"}}})),
    );

    assert!(!is_error(&written), "{}", message(&written));
    assert_eq!(
        status_of(&mut server, "taken-on")["author"],
        json!("newcomer")
    );
}

/// **New without a name is asked again.**
#[test]
fn new_without_a_name_is_asked_again() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_taken_on_set(&home, &repository);

    let again = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("taken-on"),
        form(),
        Some(json!({"which_identity": {"action": "accept", "content": {"record": "new"}}})),
    );

    assert_eq!(
        again["result"]["resultType"],
        json!("input_required"),
        "{again}"
    );
}

/// **`identify` asks too**, offering only the records already there.
#[test]
fn identify_asks_which_record_is_theirs() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_taken_on_set(&home, &repository);

    let asked = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "identify", "scope": "shared:taken-on"}),
        form(),
        None,
    );

    assert_eq!(
        asked["result"]["resultType"],
        json!("input_required"),
        "{asked}"
    );
    let choices = asked["result"]["inputRequests"]["which_identity"]["params"]["requestedSchema"]
        ["properties"]["record"]["oneOf"]
        .as_array()
        .expect("choices")
        .clone();
    assert!(
        !choices.iter().any(|choice| choice["const"] == json!("new")),
        "identifying is choosing an existing record: {asked}"
    );
}

/// **`status` on a taken-on set says the next write asks which identity is theirs**, where it will.
#[test]
fn status_says_a_taken_on_set_will_ask_which_identity_is_theirs() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_taken_on_set(&home, &repository);

    let status = payload(&modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "status"}),
        form(),
        None,
    ));

    let note = status["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|set| set["scope"] == json!("shared:taken-on"))
        .expect("the set")["author_note"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(note.contains("which of them is theirs"), "{note}");
    assert!(!note.contains("\"identify\""), "{note}");
}

// ---------------------------------------------------------------------------
// When the form goes unanswered: the fallback, decided 2026-10-16
// ---------------------------------------------------------------------------

/// **A form the client answers for the human falls back to the model's name.** Claude Code in
/// print mode declares form elicitation and declines every form unseen, observed 2026-10-16, and the
/// server cannot tell that from a human's refusal. Strict, that host could never write to a new
/// shared set; so where the form goes unanswered and the model supplied a name, the name is used and
/// relayed back, the guarantee every handshake connection already has.
#[test]
fn an_unanswered_form_falls_back_to_the_models_name() {
    for unanswered in ["decline", "cancel"] {
        let home = tempdir();
        let mut server = a_server(&home);
        create(&mut server, "fresh");

        let minted = modern_call(
            &mut server,
            "hoard_sets",
            json!({"action": "mint", "scope": "shared:fresh", "author": "given-name"}),
            form(),
            Some(json!({"publishing_name": {"action": unanswered}})),
        );

        assert!(!is_error(&minted), "{unanswered}: {}", message(&minted));
        let result = payload(&minted);
        assert_eq!(
            result["author"],
            json!("given-name"),
            "{unanswered}: {result}"
        );
        assert!(
            result["note"]
                .as_str()
                .is_some_and(|note| note.starts_with("End your reply")),
            "a name the human did not type is relayed back to them: {result}"
        );
    }
}

/// **Without a name from the model, an unanswered `mint` says how to give one.**
#[test]
fn an_unanswered_mint_without_a_name_says_to_pass_one() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let refused = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:fresh"}),
        form(),
        Some(json!({"publishing_name": {"action": "decline"}})),
    );

    assert!(is_error(&refused), "{refused}");
    assert!(message(&refused).contains("`author`"), "{refused}");
}

/// **The human's answer still wins wherever there is one**: a name the model supplied never
/// overrides a form the human filled in.
#[test]
fn an_answered_form_still_wins_over_the_models_name() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let minted = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:fresh", "author": "model-name"}),
        form(),
        Some(named("human-name")),
    );

    assert_eq!(payload(&minted)["author"], json!("human-name"));
}

/// **`rename` falls back the same way.**
#[test]
fn an_unanswered_rename_falls_back_to_the_models_name() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");
    modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:fresh"}),
        form(),
        Some(named("first-name")),
    );

    let renamed = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "rename", "scope": "shared:fresh", "author": "second-name"}),
        form(),
        Some(json!({"publishing_name": {"action": "decline"}})),
    );

    assert!(!is_error(&renamed), "{}", message(&renamed));
    let result = payload(&renamed);
    assert_eq!(result["author"], json!("second-name"));
    assert!(
        result["note"]
            .as_str()
            .is_some_and(|note| note.starts_with("End your reply")),
        "{result}"
    );
}

/// **So does `identify`, with the record the model passed**, and an unanswered first write to a
/// taken-on set says how to pass one.
#[test]
fn an_unanswered_identify_falls_back_to_the_models_record() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_taken_on_set(&home, &repository);
    let asked = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("taken-on"),
        form(),
        None,
    );
    let record = asked["result"]["inputRequests"]["which_identity"]["params"]["requestedSchema"]
        ["properties"]["record"]["oneOf"][0]["const"]
        .clone();
    let unanswered_write = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("taken-on"),
        form(),
        Some(json!({"which_identity": {"action": "decline"}})),
    );

    let identified = modern_call(
        &mut server,
        "hoard_sets",
        json!({"action": "identify", "scope": "shared:taken-on", "record": record}),
        form(),
        Some(json!({"which_identity": {"action": "decline"}})),
    );

    assert!(is_error(&unanswered_write), "{unanswered_write}");
    assert!(
        message(&unanswered_write).contains("as `record` to `hoard_sets` action \"identify\""),
        "{unanswered_write}"
    );
    assert!(!is_error(&identified), "{}", message(&identified));
    assert_eq!(
        status_of(&mut server, "taken-on")["author"],
        json!("someone-else")
    );
}

// ---------------------------------------------------------------------------
// What the model learns: the question is invisible to it
// ---------------------------------------------------------------------------

/// **A write completed after the form says what the human gave.** A client that shows the form
/// retries the call itself, so the model sees one call and one result and never the question --
/// observed 2026-10-16, where a session found its set attributed to a name it had never seen and
/// could not account for. The result names the answer, and says the form was one the model did not
/// see.
#[test]
fn a_write_completed_after_the_form_names_what_the_human_gave() {
    let home = tempdir();
    let mut server = a_server(&home);
    create(&mut server, "fresh");

    let written = modern_call(
        &mut server,
        "hoard_memorize",
        charter_arguments("fresh"),
        form(),
        Some(named("Gimli")),
    );
    let later = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("fresh"),
        form(),
        None,
    );

    let note = payload(&written)["author_note"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(note.contains("gave \"Gimli\""), "{note}");
    assert!(note.contains("you did not see"), "{note}");
    assert!(
        payload(&later).get("author_note").is_none(),
        "only the write that settled it says so: {later}"
    );
}

/// **Choosing an existing record is reported the same way.**
#[test]
fn a_write_completed_after_choosing_a_record_names_the_choice() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_taken_on_set(&home, &repository);
    let asked = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("taken-on"),
        form(),
        None,
    );
    let record = asked["result"]["inputRequests"]["which_identity"]["params"]["requestedSchema"]
        ["properties"]["record"]["oneOf"][0]["const"]
        .clone();

    let written = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("taken-on"),
        form(),
        Some(json!({"which_identity": {"action": "accept", "content": {"record": record}}})),
    );

    let note = payload(&written)["author_note"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(note.contains("chose \"someone-else\""), "{note}");
}

/// **And saying they are new, with the name they gave.**
#[test]
fn a_write_completed_after_saying_they_are_new_names_them() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_taken_on_set(&home, &repository);

    let written = modern_call(
        &mut server,
        "hoard_memorize",
        note_arguments("taken-on"),
        form(),
        Some(json!({"which_identity": {"action": "accept",
                                       "content": {"record": "new", "name": "newcomer"}}})),
    );

    let note = payload(&written)["author_note"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(note.contains("new here, as \"newcomer\""), "{note}");
}
