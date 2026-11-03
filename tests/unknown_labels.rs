// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for `unknown-labels-carried-opaquely`: a link label this build does not know is
//! carried through import, storage and export, shown as written, never walked, and restated only by
//! copying.
//!
//! `documents/design/memory-data-model.md`, *unknown labels are carried opaquely*. The label comes
//! from a newer build, so every fixture here gets one the only way a real set would: by reading a
//! file that carries it. Each write is sent the way a *validating client* sends it, because the
//! restating rule is a guard that is worthless if the published schema stops a client reaching it.

use scatterhoard::mcp::Server;
use scatterhoard::memory::{Author, LinkLabel, Scope};
use scatterhoard::store::Store;
use serde_json::{Value, json};

/// A label no build of this one knows.
#[path = "support/review_all.rs"]
mod review_all;

const UNKNOWN: &str = "is_sketch";

fn a_server() -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    server
}

fn call(server: &mut Server, tool: &str, arguments: Value) -> Value {
    if tool == "hoard_sets" {
        review_all::review_before_export(server, &arguments);
    }
    let request = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": tool, "arguments": arguments}
    });
    let raw = server
        .handle(&request.to_string())
        .expect("a request with an id must be answered");
    serde_json::from_str(&raw).expect("the response should be JSON")
}

fn payload(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a text result, got {response}"));
    serde_json::from_str(text).expect("the tool payload should be JSON")
}

fn is_error(response: &Value) -> bool {
    response.get("error").is_some() || response["result"]["isError"] == json!(true)
}

fn message(response: &Value) -> String {
    response["result"]["content"][0]["text"]
        .as_str()
        .or_else(|| response["error"]["message"].as_str())
        .unwrap_or_default()
        .to_owned()
}

fn input_schema(server: &mut Server, tool: &str) -> Value {
    let raw = server
        .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string())
        .expect("a request with an id must be answered");
    let listed: Value = serde_json::from_str(&raw).expect("JSON");
    listed["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|tool_entry| tool_entry["name"] == json!(tool))
        .unwrap_or_else(|| panic!("{tool} is published"))["inputSchema"]
        .clone()
}

/// The published shape of one link's `label`, wherever the tool puts its `links`.
fn label_schema(schema: &Value) -> Value {
    let direct = &schema["properties"]["links"]["items"]["properties"]["label"];
    if !direct.is_null() {
        return direct.clone();
    }
    // `hoard_correct` publishes its arguments flat, so `links` is a top-level property there too;
    // looked for rather than assumed, so a change of shape fails here rather than passing vacuously.
    panic!("no published label schema in {schema}")
}

/// Call a tool the way a *validating client* does: drop every argument the published schema does
/// not declare, and refuse -- before the server sees anything -- a link label the schema does not
/// admit. A test that builds its request by hand exercises a call no such client can make, which is
/// how `amending-a-claim-is-refused-where-no-client-can-reach-it` stayed green while unreachable.
fn as_a_client(server: &mut Server, tool: &str, arguments: Value) -> Value {
    let schema = input_schema(server, tool);
    let declared = schema["properties"].as_object().expect("properties");
    let label = label_schema(&schema);
    let filtered: serde_json::Map<String, Value> = arguments
        .as_object()
        .expect("arguments are an object")
        .iter()
        .filter(|(name, _)| declared.contains_key(*name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    if let Some(links) = filtered.get("links").and_then(Value::as_array) {
        for link in links {
            let admitted = match label["enum"].as_array() {
                Some(allowed) => allowed.contains(&link["label"]),
                None => label["type"] == json!("string") && link["label"].is_string(),
            };
            if !admitted {
                return json!({"error": {"message": format!(
                    "the client refused {} before sending it: the schema does not admit it",
                    link["label"]
                )}});
            }
        }
    }
    call(server, tool, Value::Object(filtered))
}

fn memorize(server: &mut Server, content: &str, links: Value) -> Value {
    as_a_client(
        server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": content,
            "why": "the fixture needs one",
            "keywords": [],
            "confidence": 0.6,
            "origin": "llm",
            "links": links,
        }),
    )
}

fn stored(response: &Value) -> String {
    assert!(!is_error(response), "{}", message(response));
    payload(response)["id"].as_str().expect("an id").to_owned()
}

fn amend(server: &mut Server, id: &str, links: Value) -> Value {
    as_a_client(
        server,
        "hoard_correct",
        json!({"action": "amend", "scope": "local", "id": id, "links": links}),
    )
}

fn look_up(server: &mut Server, id: &str) -> Value {
    payload(&call(server, "hoard_lookup", json!({"id": id})))["memories"][0].clone()
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn file(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Enough uniqueness for a scratch directory, without a dependency for it.
fn tempdir() -> TempDir {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_nanos();
    let mut path = std::env::temp_dir();
    path.push(
        format!(
            "scatterhoard-unknown-labels-{nanos}-{:?}",
            std::thread::current().id()
        )
        .replace(['(', ')', ' '], "")
        .replace("ThreadId", "t"),
    );
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

fn export(server: &mut Server, file: &std::path::Path) {
    let response = call(
        server,
        "hoard_sets",
        json!({"action": "export", "scope": "local", "file": file}),
    );
    assert!(!is_error(&response), "{}", message(&response));
}

/// Every line but the header, which carries the moment of writing and so differs between two
/// exports of the same records.
fn records(file: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .expect("reads the exported file")
        .lines()
        .skip(1)
        .map(str::to_owned)
        .collect()
}

/// A set in which `holder` links to `target` with a label this build does not know -- arrived at as
/// a newer build's set would arrive: `related_to` is exported, rewritten in the file to the unknown
/// label, and the file restored.
struct Fixture {
    server: Server,
    dir: TempDir,
    holder: String,
    target: String,
    other: String,
}

fn a_set_carrying_an_unknown_label() -> Fixture {
    let mut server = a_server();
    let target = stored(&memorize(&mut server, "alpha is the target", json!([])));
    let other = stored(&memorize(&mut server, "gamma is somewhere else", json!([])));
    let holder = stored(&memorize(
        &mut server,
        "beta holds the link",
        json!([{"label": "related_to", "target": &target}]),
    ));

    let dir = tempdir();
    let written = dir.file("as-written.json");
    export(&mut server, &written);
    let text = std::fs::read_to_string(&written).expect("reads the export");
    let known = "\"label\":\"related_to\"";
    assert_eq!(text.matches(known).count(), 1, "the fixture has one link");
    let newer = text.replace(known, &format!("\"label\":\"{UNKNOWN}\""));
    let from_a_newer_build = dir.file("from-a-newer-build.json");
    std::fs::write(&from_a_newer_build, newer).expect("writes the rewritten file");

    let restored = call(
        &mut server,
        "hoard_sets",
        json!({
            "action": "restore",
            "scope": "local",
            "file": &from_a_newer_build,
            "save_to": dir.file("before-restore.json"),
        }),
    );
    assert!(!is_error(&restored), "{}", message(&restored));

    Fixture {
        server,
        dir,
        holder,
        target,
        other,
    }
}

// ---------------------------------------------------------------------------
// Carried
// ---------------------------------------------------------------------------

/// **Byte-identical, record for record.** An export writes back what the import read, so the
/// checksums a newer build recorded still describe these bytes and a round trip through this build
/// changes nothing.
#[test]
fn a_file_carrying_an_unknown_label_round_trips_byte_identically() {
    let mut fixture = a_set_carrying_an_unknown_label();
    let again = fixture.dir.file("exported-again.json");
    export(&mut fixture.server, &again);

    assert_eq!(
        records(&again),
        records(&fixture.dir.file("from-a-newer-build.json")),
        "re-exporting changed a record carrying an unknown label"
    );
}

/// Shown as written: a caller sees exactly what the newer build wrote, which is also what it has to
/// copy to keep the link across a supersession.
#[test]
fn lookup_shows_an_unknown_label_as_written() {
    let mut fixture = a_set_carrying_an_unknown_label();
    let holder = fixture.holder.clone();
    let memory = look_up(&mut fixture.server, &holder);

    assert_eq!(
        memory["links"],
        json!([{"label": UNKNOWN, "target": fixture.target}])
    );
}

// ---------------------------------------------------------------------------
// Restated, never authored
// ---------------------------------------------------------------------------

/// The published label admits a label outside the vocabulary, and still names the vocabulary.
///
/// Decided 2026-09-28: with an `enum` published, a validating client refuses the restatement before
/// the server sees it, and a model reading the schema never writes it, so the restating rule would
/// guard nothing. Typos are the server's to refuse, one round trip later.
#[test]
fn the_published_label_admits_a_restated_unknown_label_and_names_the_vocabulary() {
    let mut server = a_server();
    for tool in ["hoard_memorize", "hoard_correct"] {
        let label = label_schema(&input_schema(&mut server, tool));
        assert!(
            label.get("enum").is_none(),
            "{tool} publishes an enum, which no restated unknown label can pass: {label}"
        );
        let description = label["description"].as_str().expect("a description");
        for known in LinkLabel::ALL {
            assert!(
                description.contains(&format!("`{}`", known.name())),
                "{tool}'s label description does not name `{}`: {description}",
                known.name()
            );
        }
    }
}

/// **The tool's own description names the vocabulary too**, because a property description is not
/// guaranteed to arrive: `documents/design/tool-surface.md` measured Claude Desktop dropping every
/// property description but `action`'s, and before 2026-09-28 the `enum` it kept was the only place
/// a session there saw the labels. Decided with the human partner the same day.
#[test]
fn the_tool_descriptions_name_the_vocabulary_where_every_client_keeps_it() {
    let mut server = a_server();
    let raw = server
        .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string())
        .expect("a request with an id must be answered");
    let listed: Value = serde_json::from_str(&raw).expect("JSON");
    for tool in ["hoard_memorize", "hoard_correct"] {
        let description = listed["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .find(|entry| entry["name"] == json!(tool))
            .unwrap_or_else(|| panic!("{tool} is published"))["description"]
            .as_str()
            .expect("a tool description")
            .to_owned();
        for known in LinkLabel::ALL {
            assert!(
                description.contains(&format!("`{}`", known.name())),
                "{tool}'s own description does not name `{}`: {description}",
                known.name()
            );
        }
    }
}

#[test]
fn a_supersession_may_restate_an_unknown_label() {
    let mut fixture = a_set_carrying_an_unknown_label();
    let response = memorize(
        &mut fixture.server,
        "beta holds the link, restated",
        json!([
            {"label": "supersedes", "target": &fixture.holder},
            {"label": UNKNOWN, "target": &fixture.target},
        ]),
    );
    let replacement = stored(&response);

    assert_eq!(
        payload(&response)["link_delta"][0]["left_behind"],
        json!([]),
        "the restated link was reported as left behind"
    );
    let memory = look_up(&mut fixture.server, &replacement);
    assert!(
        memory["links"]
            .as_array()
            .expect("links")
            .contains(&json!({"label": UNKNOWN, "target": fixture.target})),
        "the restated link was not kept: {memory}"
    );
}

/// **Authoring is still strict**: with nothing to copy from, an unknown label is a typo as far as
/// this build can tell, and the refusal is where the vocabulary is taught.
#[test]
fn a_new_link_with_an_unknown_label_is_refused_and_named() {
    let mut fixture = a_set_carrying_an_unknown_label();
    let target = fixture.target.clone();
    let response = memorize(
        &mut fixture.server,
        "delta invents a label",
        json!([{"label": UNKNOWN, "target": &target}]),
    );

    assert!(is_error(&response), "an authored unknown label was stored");
    let text = message(&response);
    assert!(
        text.contains(UNKNOWN),
        "the refusal does not name it: {text}"
    );
    assert!(
        text.contains("`related_to`"),
        "the refusal does not name the vocabulary: {text}"
    );
}

/// The pair is what is restated, not the label alone: the same unknown label aimed somewhere new is
/// a new claim nobody on this build can check.
#[test]
fn restating_needs_the_same_target() {
    let mut fixture = a_set_carrying_an_unknown_label();
    let response = memorize(
        &mut fixture.server,
        "beta holds the link, retargeted",
        json!([
            {"label": "supersedes", "target": &fixture.holder},
            {"label": UNKNOWN, "target": &fixture.other},
        ]),
    );
    assert!(is_error(&response), "a retargeted unknown label was stored");
}

/// Byte-for-byte: a normalised spelling is a different label to the build that wrote it.
#[test]
fn restating_needs_the_same_spelling() {
    let mut fixture = a_set_carrying_an_unknown_label();
    let response = memorize(
        &mut fixture.server,
        "beta holds the link, respelled",
        json!([
            {"label": "supersedes", "target": &fixture.holder},
            {"label": "Is_Sketch", "target": &fixture.target},
        ]),
    );
    assert!(is_error(&response), "a respelled unknown label was stored");
}

/// Only the memory being superseded counts as the source of a restatement. Copying a label from
/// some other memory in the set is authoring it here.
#[test]
fn restating_needs_the_label_on_the_memory_superseded() {
    let mut fixture = a_set_carrying_an_unknown_label();
    let response = memorize(
        &mut fixture.server,
        "gamma borrows beta's label",
        json!([
            {"label": "supersedes", "target": &fixture.other},
            {"label": UNKNOWN, "target": &fixture.target},
        ]),
    );
    assert!(
        is_error(&response),
        "a label borrowed from a bystander was stored"
    );
}

/// Amend replaces the whole link array, which is the same trap as supersession: the memory being
/// amended is the one whose pairs may be restated.
#[test]
fn an_amendment_may_restate_an_unknown_label_the_memory_holds() {
    let mut fixture = a_set_carrying_an_unknown_label();
    // The restored holder is in service and so not amendable; a replacement that restated the
    // label has never left this machine.
    let replacement = stored(&memorize(
        &mut fixture.server,
        "beta holds the link, restated",
        json!([
            {"label": "supersedes", "target": &fixture.holder},
            {"label": UNKNOWN, "target": &fixture.target},
        ]),
    ));

    let kept = amend(
        &mut fixture.server,
        &replacement,
        json!([
            {"label": "supersedes", "target": &fixture.holder},
            {"label": UNKNOWN, "target": &fixture.target},
            {"label": "related_to", "target": &fixture.other},
        ]),
    );
    assert!(!is_error(&kept), "{}", message(&kept));

    let retargeted = amend(
        &mut fixture.server,
        &replacement,
        json!([
            {"label": "supersedes", "target": &fixture.holder},
            {"label": UNKNOWN, "target": &fixture.other},
        ]),
    );
    assert!(
        is_error(&retargeted),
        "an amendment authored an unknown label"
    );
}
