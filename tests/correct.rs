// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for `hoard_correct`, which changes a memory that already exists.
//!
//! `documents/design/corrections.md` settles what belongs here, and the boundary moved on
//! 2026-08-29: it is no longer *which fields* but **whether the record ever entered service**. A
//! record that has never been exported to any target may be corrected entirely, `content` and `why`
//! included, because nobody has ever read it and there is no belief to destroy. Once it has been
//! exported a second durable copy exists, so every later change must carry ancestry and the
//! instrument is supersession.

use std::sync::Arc;

use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::mcp::Server;
use scatterhoard::memory::{Author, Scope};
use scatterhoard::store::Store;
use serde_json::{Value, json};

#[path = "support/review_all.rs"]
mod review_all;

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
                let count = |w: &str| text.matches(w).count() as f32;
                Vector::new(vec![count("alpha"), count("beta"), 0.01])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

fn a_server() -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
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
        .unwrap_or_default()
        .to_owned()
}

/// Store a memory and return its id.
fn store(server: &mut Server, content: &str, keywords: Value) -> String {
    let stored = call(
        server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": content,
            "why": "the reasoning that must survive an amendment",
            "context": "the branch this came up on",
            "keywords": keywords,
            "confidence": 0.6,
            "origin": "llm"
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
    payload(&stored)["id"].as_str().expect("an id").to_owned()
}

fn amend(server: &mut Server, mut arguments: Value) -> Value {
    arguments["action"] = json!("amend");
    arguments["scope"] = json!("local");
    call(server, "hoard_correct", arguments)
}

fn look_up(server: &mut Server, id: &str) -> Value {
    payload(&call(server, "hoard_lookup", json!({"id": id})))["memories"][0].clone()
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
            "scatterhoard-correct-{nanos}-{:?}",
            std::thread::current().id()
        )
        .replace(['(', ')', ' '], "")
        .replace("ThreadId", "t"),
    );
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

/// Put the set into service, which for a `local` set means a backup.
///
/// A backup counts, and that is the design's own clause rather than an approximation of it:
/// `import_set` places no restriction on scope, so a backup can be reconciled back in, which is the
/// whole of the test — a second durable copy exists and the machinery to merge it is reachable.
fn export_to(server: &mut Server, directory: &TempDir) {
    let response = call(
        server,
        "hoard_sets",
        json!({"action": "export", "scope": "local", "file": directory.path().join("local.json")}),
    );
    assert!(!is_error(&response), "{}", message(&response));
}

/// One record from the exported file, by id — the shape that actually travels between machines.
fn exported_record(directory: &TempDir, id: &str) -> Value {
    let path = directory.path().join("local.json");
    let text = std::fs::read_to_string(&path).expect("reads the exported file");
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|record| record["id"] == json!(id))
        .unwrap_or_else(|| panic!("no record for {id} in {}", path.display()))
}

/// The properties `hoard_correct` publishes, read from `tools/list` as a client receives them.
fn published_properties(server: &mut Server) -> Vec<String> {
    let raw = server
        .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string())
        .expect("a request with an id must be answered");
    let listed: Value = serde_json::from_str(&raw).expect("JSON");
    listed["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|tool| tool["name"] == "hoard_correct")
        .expect("hoard_correct is published")["inputSchema"]["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .cloned()
        .collect()
}

/// Amend the way a *validating client* does: drop every argument the published schema does not
/// declare, then send what is left.
///
/// This is the path `amending-a-claim-is-refused-where-no-client-can-reach-it` was found on. A test
/// that builds its arguments by hand exercises a request no such client can produce, so it stays
/// green while the guard it covers is unreachable — which is why the service refusal and the
/// widened reach are both provoked through here rather than through [`amend`].
/// Store a memory that links to `target`, and return its id.
fn store_linking(server: &mut Server, content: &str, label: &str, target: &str) -> String {
    let stored = call(
        server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": content,
            "why": "so there is an edge to report",
            "confidence": 0.6,
            "origin": "llm",
            // `label` may be `supersedes`, which requires a stated keyword set. These fixtures are
            // about edges and ancestry, so empty is the truthful answer.
            "keywords": [],
            "links": [{"label": label, "target": target}],
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
    payload(&stored)["id"].as_str().expect("an id").to_owned()
}

fn amend_as_a_client(server: &mut Server, arguments: Value) -> Value {
    let declared = published_properties(server);
    let filtered = arguments
        .as_object()
        .expect("arguments are an object")
        .iter()
        .filter(|(name, _)| declared.iter().any(|allowed| allowed == *name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>();
    amend(server, Value::Object(filtered))
}

// ---------------------------------------------------------------------------
// What amend changes
// ---------------------------------------------------------------------------

#[test]
fn amend_replaces_keywords_and_leaves_the_claim_alone() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["mispelled"]));

    let response = amend(
        &mut server,
        json!({"id": &id, "keywords": ["Alpha-Routing", "spelled-right"]}),
    );
    assert!(!is_error(&response), "{}", message(&response));

    let memory = look_up(&mut server, &id);
    assert_eq!(
        memory["keywords"],
        json!(["Alpha-Routing", "spelled-right"]),
        "keywords were not replaced"
    );
    assert_eq!(memory["content"], json!("alpha governs routing"));
    assert_eq!(
        memory["why"],
        json!("the reasoning that must survive an amendment")
    );
}

#[test]
fn amend_can_replace_context_and_confidence() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));

    amend(
        &mut server,
        json!({"id": &id, "context": "the branch it actually came up on", "confidence": 0.95}),
    );

    let memory = look_up(&mut server, &id);
    assert_eq!(
        memory["context"],
        json!("the branch it actually came up on")
    );
    assert_eq!(memory["confidence"], json!(0.95));
}

/// An empty array clears, which is distinct from omitting the field. A keyword recorded by mistake
/// has to be removable, not merely replaceable.
#[test]
fn an_empty_keyword_list_clears_them() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["wrong"]));

    amend(&mut server, json!({"id": &id, "keywords": []}));

    let memory = look_up(&mut server, &id);
    assert!(
        memory.get("keywords").is_none() || memory["keywords"] == json!([]),
        "keywords survived being cleared: {memory}"
    );
}

/// Null clears `context`, where omitting it leaves it. Without this there would be no way to remove a
/// context recorded in error, only to replace it with different wrong text.
#[test]
fn a_null_context_clears_it() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));

    amend(&mut server, json!({"id": &id, "context": null}));

    let memory = look_up(&mut server, &id);
    assert!(
        memory.get("context").is_none(),
        "context survived being cleared: {memory}"
    );
}

/// Omitting a field must leave it alone, or an amendment correcting one thing would silently wipe
/// everything else.
#[test]
fn omitted_fields_are_left_untouched() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["keep-me"]));

    amend(&mut server, json!({"id": &id, "confidence": 0.5}));

    let memory = look_up(&mut server, &id);
    assert_eq!(memory["keywords"], json!(["keep-me"]));
    assert_eq!(memory["context"], json!("the branch this came up on"));
}

/// Keywords reach the embedding, so amending them has to re-embed — otherwise a corrected keyword
/// would be searchable by the spelling it no longer has.
#[test]
fn amending_keywords_keeps_the_memory_findable() {
    let mut server = a_server();
    let id = store(&mut server, "beta describes storage", json!(["typo"]));

    amend(
        &mut server,
        json!({"id": &id, "keywords": ["Storage-Layout"]}),
    );

    let by_new = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["storage-layout"]}),
    ));
    assert_eq!(
        by_new["memories"][0]["id"],
        json!(id),
        "the corrected keyword does not find the memory: {by_new}"
    );

    let by_old = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["typo"]}),
    ));
    assert_eq!(
        by_old["memories"].as_array().map(Vec::len),
        Some(0),
        "the memory is still findable by the keyword that was removed: {by_old}"
    );
}

#[test]
fn amend_reports_whether_the_memory_is_still_findable_by_meaning() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));

    let report = payload(&amend(&mut server, json!({"id": &id, "confidence": 0.7})));
    assert_eq!(
        report["semantic_search_available"],
        json!(true),
        "amend should say whether the memory it rewrote can still be found: {report}"
    );
}

/// `modified` records who changed it and when, which is what makes an amendment visible even though
/// the previous value is not — and it is what the export review gate keys on.
#[test]
fn amend_updates_the_modified_attribution() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));
    let before = look_up(&mut server, &id);

    amend(&mut server, json!({"id": &id, "confidence": 0.9}));
    let after = look_up(&mut server, &id);

    assert_eq!(after["created"], before["created"], "created must not move");
    // `by` is a reference now, so the name is one hop away -- and following it is the property
    // worth asserting, since a bare id proves nothing about who the amendment was attributed to.
    let identity = look_up(
        &mut server,
        after["modified"]["by"]
            .as_str()
            .expect("an author record id"),
    );
    assert_eq!(identity["content"], json!("jtcours"));
    assert_ne!(
        after["modified"], before["modified"],
        "modified did not record the amendment"
    );
}

// ---------------------------------------------------------------------------
// What amend refuses
// ---------------------------------------------------------------------------

/// **Before service, correction reaches the claim.** A record that has never been exported holds no
/// belief anyone could have formed, so the field list that used to protect `content` and `why` has
/// nothing left to protect.
///
/// Sent through the published schema rather than by hand, because that is the path a client takes
/// and the one on which a capability declared nowhere is silently stripped.
#[test]
fn content_and_why_are_amendable_before_the_record_enters_service() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));

    let response = amend_as_a_client(
        &mut server,
        json!({"id": &id, "content": "alpha governs recall", "why": "the repaired reasoning"}),
    );
    assert!(!is_error(&response), "{}", message(&response));
    assert_eq!(payload(&response)["amended"], json!(["content", "why"]));

    let found = look_up(&mut server, &id);
    assert_eq!(found["content"], json!("alpha governs recall"));
    assert_eq!(found["why"], json!("the repaired reasoning"));
}

/// **After service, nothing is correctable — apparatus included.** The boundary is the record's
/// history, not the field being changed: once a second durable copy exists, a change that leaves
/// only a `modified` timestamp is a change a later merge cannot reason about.
#[test]
fn amending_a_record_that_has_entered_service_is_refused_and_says_to_supersede() {
    let mut server = a_server();
    let backup = tempdir();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));
    export_to(&mut server, &backup);

    for change in [
        json!({"content": "alpha governs recall"}),
        json!({"keywords": ["beta"]}),
        json!({"confidence": 0.9}),
    ] {
        let mut arguments = change.clone();
        arguments["id"] = json!(&id);
        let response = amend_as_a_client(&mut server, arguments);
        assert!(
            is_error(&response),
            "amending {change} after export should be refused: {}",
            message(&response)
        );
        let complaint = message(&response);
        assert!(
            complaint.contains("supersede"),
            "the refusal should name the instrument that does carry ancestry: {complaint}"
        );
    }

    let found = look_up(&mut server, &id);
    assert_eq!(
        found["content"],
        json!("alpha governs routing"),
        "a refused amendment must not have written anything"
    );
    assert_eq!(found["confidence"], json!(0.6));
}

/// **A rewrite can leave someone else's edge asserting a thing they never judged.** `B SUPPORTS A`
/// was authored against A's text; move that text and the label may be void, which only the caller
/// can tell -- so the response says what points here and with what label, and says nothing about
/// what to do with it.
#[test]
fn a_correction_to_content_names_what_points_at_the_memory() {
    let mut server = a_server();
    let target = store(&mut server, "alpha governs routing", json!(["alpha"]));
    let source = store_linking(&mut server, "beta measured alpha", "supports", &target);

    let response = amend_as_a_client(
        &mut server,
        json!({"id": &target, "content": "alpha governs nothing of the kind"}),
    );
    assert!(!is_error(&response), "{}", message(&response));

    let reported = payload(&response)["in_links"].clone();
    assert_eq!(
        reported,
        json!([{"source": source, "label": "supports"}]),
        "the remark should name the edge and its label: {}",
        payload(&response)
    );
    let note = payload(&response)["in_links_note"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        note.contains("supersed"),
        "the note should name the instrument for a judgement that did not survive: {note}"
    );

    // A remark, never a refusal: the amendment happened.
    assert_eq!(
        look_up(&mut server, &target)["content"],
        json!("alpha governs nothing of the kind")
    );
}

/// The same for `why`, which is the other half of the claim and moves under an edge exactly as
/// `content` does.
#[test]
fn a_correction_to_why_names_what_points_at_the_memory() {
    let mut server = a_server();
    let target = store(&mut server, "alpha governs routing", json!(["alpha"]));
    store_linking(
        &mut server,
        "beta is an instance of alpha",
        "instance_of",
        &target,
    );

    let response = amend_as_a_client(
        &mut server,
        json!({"id": &target, "why": "a different reason entirely"}),
    );
    assert!(!is_error(&response), "{}", message(&response));
    assert_eq!(
        payload(&response)["in_links"][0]["label"],
        json!("instance_of"),
        "{}",
        payload(&response)
    );
}

/// **And the half that keeps it worth reading.** Apparatus correction cannot invalidate a label,
/// because the text the edge's author judged has not moved. A remark that fired on every confidence
/// change would teach callers to skip it, which costs the cases it exists for.
#[test]
fn an_apparatus_correction_says_nothing_about_in_links() {
    let mut server = a_server();
    let target = store(&mut server, "alpha governs routing", json!(["alpha"]));
    store_linking(&mut server, "beta measured alpha", "supports", &target);

    let response = amend_as_a_client(&mut server, json!({"id": &target, "confidence": 0.9}));
    assert!(!is_error(&response), "{}", message(&response));
    assert!(
        payload(&response).get("in_links").is_none(),
        "an apparatus amendment must not raise the remark: {}",
        payload(&response)
    );
}

/// A `supersedes` edge is left out, and it is the one label that is *not* a judgement about the
/// text: it says this memory was replaced, which stays true however the replaced text reads.
#[test]
fn a_supersedes_in_link_is_not_remarked_on() {
    let mut server = a_server();
    let target = store(&mut server, "alpha governs routing", json!(["alpha"]));
    store_linking(
        &mut server,
        "alpha governs recall instead",
        "supersedes",
        &target,
    );

    let response = amend_as_a_client(
        &mut server,
        json!({"id": &target, "content": "alpha governed routing, once"}),
    );
    assert!(!is_error(&response), "{}", message(&response));
    assert!(
        payload(&response).get("in_links").is_none(),
        "a supersedes edge is not a judgement against the text: {}",
        payload(&response)
    );
}

/// Nothing points at it, so nothing is said. The field is absent rather than empty, for the reason
/// `links` is absent when a memory authors none.
#[test]
fn a_correction_with_no_in_links_says_nothing() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));

    let response = amend_as_a_client(&mut server, json!({"id": &id, "content": "alpha again"}));
    assert!(!is_error(&response), "{}", message(&response));
    assert!(
        payload(&response).get("in_links").is_none(),
        "{}",
        payload(&response)
    );
}

/// **A forget is undone by superseding it**: a new memory with a `supersedes` link to the forgotten
/// id is accepted, and the response says what it superseded. Asked 2026-09-28 under
/// `merge-without-a-recorded-ancestor`, where a forget becomes one-way at import and this is the
/// deliberate undo -- a new event, as a git revert is a new commit.
#[test]
fn a_forgotten_memory_can_be_superseded_to_undo_the_forget() {
    let mut server = a_server();
    let forgotten = store(&mut server, "alpha governs routing", json!(["alpha"]));
    let forgot = call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": &forgotten}),
    );
    assert!(!is_error(&forgot), "{}", message(&forgot));

    let undone = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha governs routing",
            "why": "the forget was a mistake",
            "keywords": ["alpha"],
            "confidence": 0.6,
            "origin": "llm",
            "links": [{"label": "supersedes", "target": &forgotten}],
        }),
    );
    assert!(!is_error(&undone), "{}", message(&undone));
    let replacement = payload(&undone)["id"].as_str().expect("an id").to_owned();
    assert_eq!(
        look_up(&mut server, &replacement)["links"],
        json!([{"label": "supersedes", "target": forgotten}]),
        "the undo is recorded as a supersession of the forgotten id"
    );
}

/// **The gate is per record, not per set.** A memory written after the export has never left this
/// machine even though its neighbours have, so it stays correctable — which is what distinguishes
/// asking the bases about *this id* from asking whether the set has ever been exported at all.
#[test]
fn a_record_written_after_the_export_is_still_correctable() {
    let mut server = a_server();
    let backup = tempdir();
    let exported = store(&mut server, "alpha governs routing", json!(["alpha"]));
    export_to(&mut server, &backup);
    let later = store(&mut server, "beta governs ranking", json!(["beta"]));

    let refused = amend_as_a_client(&mut server, json!({"id": &exported, "confidence": 0.9}));
    assert!(is_error(&refused), "{}", message(&refused));

    let allowed = amend_as_a_client(
        &mut server,
        json!({"id": &later, "content": "beta governs ordering"}),
    );
    assert!(!is_error(&allowed), "{}", message(&allowed));
    assert_eq!(
        look_up(&mut server, &later)["content"],
        json!("beta governs ordering")
    );
}

/// A restore establishes agreement with the file it read, so what it wrote is in service by the same
/// test — the records came *from* a durable copy, which is the property the gate is about.
#[test]
fn a_restored_record_is_in_service() {
    let mut server = a_server();
    let backup = tempdir();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));
    export_to(&mut server, &backup);

    let elsewhere = tempdir();
    let restored = call(
        &mut server,
        "hoard_sets",
        json!({
            "action": "restore",
            "scope": "local",
            "file": backup.path().join("local.json"),
            "save_to": elsewhere.path().join("before-restore.json"),
        }),
    );
    assert!(!is_error(&restored), "{}", message(&restored));

    let response = amend_as_a_client(&mut server, json!({"id": &id, "confidence": 0.9}));
    assert!(is_error(&response), "{}", message(&response));
}

#[test]
fn an_amendment_that_changes_nothing_is_refused() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));

    let response = amend(&mut server, json!({"id": &id}));
    assert!(is_error(&response));
    let complaint = message(&response);
    assert!(
        complaint.contains("nothing"),
        "unhelpful refusal: {complaint}"
    );
    // The refusal names what the caller could have sent, so it has to name the widened set rather
    // than the apparatus list it used to be -- a caller told to send `keywords` or `confidence`
    // when they meant to fix a typo has been sent the wrong way by an accurate-sounding message.
    for field in ["content", "why"] {
        assert!(
            complaint.contains(field),
            "the refusal should offer {field}: {complaint}"
        );
    }
}

#[test]
fn amending_a_memory_that_does_not_exist_is_refused() {
    let mut server = a_server();

    let response = amend(
        &mut server,
        json!({"id": "019fd012-321b-7db3-8716-e84fc06c147f", "confidence": 0.5}),
    );
    assert!(is_error(&response));
    assert!(
        message(&response).contains("no memory"),
        "unhelpful refusal: {}",
        message(&response)
    );
}

/// A prefix is not an identity — `lookup-prefix-selectivity-is-time-correlated` — so amending by one
/// would risk changing whichever same-millisecond sibling sorted first.
#[test]
fn amending_by_an_id_prefix_is_refused() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));

    let response = amend(&mut server, json!({"id": &id[..13], "confidence": 0.5}));
    assert!(
        is_error(&response),
        "a prefix must not be accepted where a memory is being changed"
    );
}

/// Amending links goes through the same cycle guard as storing them, because a hand-made supersession
/// cycle is a data error whichever operation creates it.
#[test]
fn an_amendment_that_would_close_a_supersession_cycle_is_refused() {
    let mut server = a_server();
    let first = store(&mut server, "alpha as first written", json!(["alpha"]));

    let second = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha as revised",
            "why": "a revision",
            "confidence": 0.8,
            "origin": "llm",
            // Superseding states the replacement's keywords; this fixture is not about indexing.
            "keywords": [],
            "links": [{"label": "supersedes", "target": &first}]
        }),
    );
    let second = payload(&second)["id"].as_str().expect("an id").to_owned();

    // Now make the superseded one supersede its replacement, closing the loop.
    let response = amend(
        &mut server,
        json!({"id": &first, "links": [{"label": "supersedes", "target": &second}]}),
    );
    assert!(is_error(&response), "a supersession cycle must be refused");
}

#[test]
fn amend_refuses_a_set_that_is_not_loaded() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_correct",
        json!({
            "action": "amend",
            "scope": {"shared": "absent"},
            "id": "019fd012-321b-7db3-8716-e84fc06c147f",
            "confidence": 0.5
        }),
    );
    assert!(is_error(&response));
    assert!(message(&response).contains("hoard_sets"));
}

#[test]
fn an_unknown_correction_action_names_the_ones_that_exist() {
    let mut server = a_server();
    let response = call(
        &mut server,
        "hoard_correct",
        json!({"action": "rewrite-history"}),
    );
    assert!(is_error(&response));
    assert!(
        message(&response).contains("amend"),
        "an unknown action should list what is available: {}",
        message(&response)
    );
}

// ---------------------------------------------------------------------------
// forget
// ---------------------------------------------------------------------------

fn forget(server: &mut Server, id: &str) -> Value {
    call(
        server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": id}),
    )
}

#[test]
fn a_forgotten_memory_is_gone_from_recall_and_returns_only_a_tombstone_from_lookup() {
    let mut server = a_server();
    let id = store(&mut server, "alpha holds a secret", json!(["alpha"]));

    let response = forget(&mut server, &id);
    assert!(!is_error(&response), "{}", message(&response));

    let found = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 10}),
    ));
    // **Nothing at all, not merely *not that memory*.** This was briefly weakened to check only for
    // the forgotten id, on the belief that a set's identity record would come back alongside it at
    // similarity 0.0. That was true when identity records were first written and false by the time
    // it was written down: they are never embedded, so they cannot be reached by meaning at all.
    assert_eq!(
        found["memories"].as_array().map(Vec::len),
        Some(0),
        "a forgotten memory came back from recall: {found}"
    );

    // **Gone as a memory, not gone without trace, and the difference is the whole contract.** A
    // tombstone is a record like any other -- it travels in the exported file, `status` counts it,
    // and it is what makes a link into a forgotten memory a good reference rather than a dangling
    // one. Lookup used to answer *nothing here*, which is also its answer for an id that never
    // existed, so the one surface that could tell a caller which of those they had did not.
    let looked_up = payload(&call(&mut server, "hoard_lookup", json!({"id": &id})));
    assert_eq!(
        looked_up["memories"].as_array().map(Vec::len),
        Some(0),
        "a forgotten memory came back from lookup as a memory: {looked_up}"
    );
    assert_eq!(
        looked_up["forgotten"][0]["id"],
        json!(&id),
        "but lookup must still say it was forgotten rather than never held: {looked_up}"
    );
    assert!(
        looked_up["forgotten"][0]["content"].is_null(),
        "carrying no content, which is what forgetting means: {looked_up}"
    );
}

/// The operation exists partly to remove sensitive data, so the keyword index has to go too — a
/// secret recorded as a keyword would otherwise stay exactly findable.
#[test]
fn a_forgotten_memory_is_gone_from_the_keyword_index() {
    let mut server = a_server();
    let id = store(&mut server, "alpha holds a secret", json!(["hunter2"]));

    forget(&mut server, &id);

    let by_keyword = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["hunter2"]}),
    ));
    assert_eq!(
        by_keyword["memories"].as_array().map(Vec::len),
        Some(0),
        "the forgotten memory is still findable by its keyword: {by_keyword}"
    );
}

/// Forgetting must say what it did *not* do. A caller who has just removed a credential and believes
/// the job is finished is the one most in need of hearing about the repository's history.
/// **`forgetting-a-chains-middle-makes-a-stale-conclusion-current-again`.** A `SUPERSEDES` edge
/// lives on the newer memory and points back at what it replaced, so forgetting that memory takes
/// the edge with it — and the record two steps back stops knowing it was ever replaced.
///
/// Driven from the tool surface rather than from `resolve_supersession`, because the damage is what
/// a caller sees: `superseded_by` is simply absent, so the stale conclusion comes back live and
/// unflagged with nothing about it looking broken.
#[test]
fn forgetting_the_middle_of_a_chain_keeps_the_oldest_superseded() {
    let mut server = a_server();
    let first = store(&mut server, "alpha governs routing", json!(["alpha"]));
    let middle = store_linking(&mut server, "alpha governs recall", "supersedes", &first);
    let last = store_linking(&mut server, "alpha governs ranking", "supersedes", &middle);

    assert_eq!(
        look_up(&mut server, &first)["superseded_by"],
        json!([last]),
        "the chain should resolve to its newest tip before anything is forgotten"
    );

    let forgotten = forget(&mut server, &middle);
    assert!(!is_error(&forgotten), "{}", message(&forgotten));

    assert_eq!(
        look_up(&mut server, &first)["superseded_by"],
        json!([last]),
        "forgetting the middle of a chain left the oldest memory reporting itself as current"
    );
}

/// **A replacement that was forgotten is not a replacement in another set.**
/// `a-forgotten-replacement-reads-as-superseded-beyond-this-set`: storing A, superseding it with B
/// and forgetting B left A flagged `superseded_beyond_this_set`, which sends a caller to load a set
/// that will never have B. A stays superseded -- forgetting a replacement does not reinstate what it
/// replaced -- but says why with the flag whose remedy applies.
#[test]
fn a_forgotten_replacement_is_reported_as_forgotten_not_as_elsewhere() {
    let mut server = a_server();
    let first = store(&mut server, "alpha governs routing", json!(["alpha"]));
    let replacement = store_linking(&mut server, "alpha governs recall", "supersedes", &first);
    assert!(!is_error(&forget(&mut server, &replacement)));

    let looked_up = look_up(&mut server, &first);
    assert_eq!(
        looked_up["replacement_forgotten"],
        json!(true),
        "lookup names the reason: {looked_up}"
    );
    assert!(
        looked_up.get("superseded_beyond_this_set").is_none(),
        "and does not send the caller to another set: {looked_up}"
    );

    let recalled = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["alpha"], "scope": "local"}),
    ));
    let hit = recalled["memories"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|hit| hit["id"] == json!(first))
        .unwrap_or_else(|| panic!("the superseded memory is still found: {recalled}"));
    assert_eq!(hit["replacement_forgotten"], json!(true), "{hit}");
    assert!(hit.get("superseded_beyond_this_set").is_none(), "{hit}");
}

/// The flag's remedy, taken: a memory superseding the forgotten id undoes the forget, and the chain
/// then resolves to it with neither flag.
#[test]
fn undoing_a_forgotten_replacement_resolves_the_chain_to_the_undo() {
    let mut server = a_server();
    let first = store(&mut server, "alpha governs routing", json!(["alpha"]));
    let replacement = store_linking(&mut server, "alpha governs recall", "supersedes", &first);
    assert!(!is_error(&forget(&mut server, &replacement)));
    let undo = store_linking(
        &mut server,
        "alpha governs recall",
        "supersedes",
        &replacement,
    );

    let looked_up = look_up(&mut server, &first);
    assert_eq!(looked_up["superseded_by"], json!([undo]), "{looked_up}");
    assert!(
        looked_up.get("replacement_forgotten").is_none(),
        "{looked_up}"
    );
    assert!(
        looked_up.get("superseded_beyond_this_set").is_none(),
        "{looked_up}"
    );
}

/// **What a tombstone may keep, exactly.** The ancestry exists so a chain survives losing a member;
/// everything else the forgotten memory authored goes with its text, because that is what forgetting
/// is for. A cross-set target keeps its set name — half the reference — and loses its keywords,
/// which are free text written by the same hand as the content being removed.
#[test]
fn a_tombstone_keeps_the_ancestry_and_nothing_else_the_memory_wrote() {
    let mut server = a_server();
    let out = tempdir();
    let replaced = store(&mut server, "alpha governs routing", json!(["alpha"]));
    let evidence = store(&mut server, "beta measured alpha", json!(["beta"]));

    let doomed = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha governs recall",
            "why": "the reasoning that goes with the text",
            "confidence": 0.6,
            "origin": "llm",
            // Superseding states the replacement's keywords; this fixture is not about indexing.
            "keywords": [],
            "links": [
                {"label": "supersedes", "target": &replaced},
                {"label": "supports", "target": &evidence},
                {
                    "label": "superseded_by",
                    "target": "01M0K23W24FA1B2WVCNTZEC4M3",
                    "target_set": "elsewhere",
                    "target_keywords": ["a-keyword-somebody-typed"],
                },
            ],
        }),
    );
    assert!(!is_error(&doomed), "{}", message(&doomed));
    let doomed = payload(&doomed)["id"].as_str().expect("an id").to_owned();

    assert!(!is_error(&forget(&mut server, &doomed)));
    export_to(&mut server, &out);

    let line = exported_record(&out, &doomed);
    assert_eq!(
        line["ancestry"],
        json!([
            {"label": "supersedes", "target": replaced},
            {"label": "superseded_by", "target": {
                "set": "elsewhere",
                "id": "01M0K23W24FA1B2WVCNTZEC4M3",
                "keywords": [],
            }},
        ]),
        "a tombstone kept the wrong things: {line}"
    );
    let text = line.to_string();
    for gone in [
        "alpha governs recall",
        "the reasoning that goes with the text",
        "a-keyword-somebody-typed",
        &evidence,
    ] {
        assert!(
            !text.contains(gone),
            "the tombstone still carries {gone:?}: {line}"
        );
    }
}

/// **And the chain survives the file**, which is the only way it reaches another machine. A fix that
/// held only in the store would look right here and break on every export.
#[test]
fn a_chain_forgotten_in_the_middle_survives_a_round_trip() {
    let mut server = a_server();
    let out = tempdir();
    let first = store(&mut server, "alpha governs routing", json!(["alpha"]));
    let middle = store_linking(&mut server, "alpha governs recall", "supersedes", &first);
    let last = store_linking(&mut server, "alpha governs ranking", "supersedes", &middle);

    assert!(!is_error(&forget(&mut server, &middle)));
    export_to(&mut server, &out);

    // A second machine, adopting the set from the file alone.
    let mut theirs = a_server();
    let imported = call(
        &mut theirs,
        "hoard_sets",
        json!({"action": "import", "scope": "local", "file": out.path().join("local.json")}),
    );
    assert!(!is_error(&imported), "{}", message(&imported));

    assert_eq!(
        look_up(&mut theirs, &first)["superseded_by"],
        json!([last]),
        "the chain did not survive the file"
    );
}

#[test]
fn forgetting_says_it_does_not_reach_a_repositorys_history() {
    let mut server = a_server();
    let id = store(&mut server, "alpha holds a secret", json!(["alpha"]));

    let report = payload(&forget(&mut server, &id));
    let note = report["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("compromised"),
        "forgetting should say a leaked secret must still be rotated: {note}"
    );
}

/// Not an error: the outcome asked for is already the case. Said, because a caller removing sensitive
/// data needs to know whether there was anything to remove.
#[test]
fn forgetting_a_memory_that_is_not_there_succeeds_and_says_so() {
    let mut server = a_server();

    let response = forget(&mut server, "019fd012-321b-7db3-8716-e84fc06c147f");
    assert!(!is_error(&response), "{}", message(&response));
    assert_eq!(payload(&response)["already_absent"], json!(true));
}

/// `origin` is apparatus, so a wrong one is amended rather than superseded.
///
/// `origin-cannot-be-corrected`: it had been named in neither list, so a value the caller sets at
/// store time and can plainly get wrong had no sanctioned repair. It belongs with the apparatus by
/// the corrections rule's own test — nobody ever believed the wrong value — and superseding would
/// have been wrong rather than verbose, since it keeps a misattribution of reasoning readable for
/// ever.
#[test]
fn origin_can_be_amended() {
    let mut server = a_server();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));

    let response = amend(&mut server, json!({"id": &id, "origin": "joint"}));
    assert!(!is_error(&response), "{}", message(&response));
    assert_eq!(payload(&response)["amended"], json!(["origin"]));

    let found = call(&mut server, "hoard_lookup", json!({"id": &id}));
    assert_eq!(
        payload(&found)["memories"][0]["origin"],
        json!("joint"),
        "the amendment did not reach the stored memory"
    );
}

/// **A field the published schema does not name is amendable only in theory.**
///
/// `amending-a-claim-is-refused-where-no-client-can-reach-it` is what this guards against, one door
/// over: a validating client strips arguments the schema does not declare, so shipping `origin` in
/// the handler and not in the schema would have produced a capability no client could reach — and
/// the test above would still have passed, because it builds the request itself.
#[test]
fn the_schema_declares_every_field_amend_accepts() {
    let mut server = a_server();
    // Read from the *published* schema, which is flattened -- one object with an `action` enum
    // rather than a `oneOf`, because at least one client reduces a `oneOf` to `{}`. Reading the
    // branch definitions instead would test a shape no client ever receives, which is the mistake
    // `amending-a-claim-is-refused-where-no-client-can-reach-it` is made of.
    let declared = published_properties(&mut server);

    for field in [
        "origin",
        "keywords",
        "links",
        "context",
        "confidence",
        "content",
        "why",
    ] {
        assert!(
            declared.iter().any(|name| name == field),
            "amend accepts {field} and the schema does not declare it, so a validating client \
             cannot send it: {declared:?}"
        );
    }
}

/// **A forgotten record in the middle of a chain can be walked through, not only jumped over.**
///
/// The two fields answer different questions and a caller needs both. `superseded_by` names the
/// *end* of the chain, which is what someone reading a stale reference wants — the current version,
/// in one hop. `incoming` carries the `supersedes` edge of whatever replaced this record directly,
/// which is what `hoard_lookup`'s own description sends a reader to when they want to walk a link
/// at a time.
///
/// **Reported for tombstones because otherwise the walk breaks exactly here.** A forgotten record
/// used to come back with `superseded_by` alone, so the record that replaced it was unreachable
/// going forward: you could reach the end of the chain or read backwards from it, but the middle
/// could not be stepped to. That is not hypothetical now — `rename` forgets the record it
/// supersedes, so every memory written before a rename cites one of these.
#[test]
fn a_forgotten_record_mid_chain_reports_both_its_successor_and_the_chains_end() {
    let mut server = a_server();

    let first = store(&mut server, "alpha, the original claim", json!(["alpha"]));
    let second = supersede(&mut server, "beta, which replaced alpha", &first);
    let third = supersede(&mut server, "gamma, which replaced beta", &second);
    let fourth = supersede(&mut server, "delta, which replaced gamma", &third);

    let forgotten = call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": &second}),
    );
    assert!(!is_error(&forgotten), "{}", message(&forgotten));

    let seen = payload(&call(&mut server, "hoard_lookup", json!({"id": &second})));
    let entry = &seen["forgotten"][0];

    assert_eq!(
        entry["superseded_by"],
        json!([fourth]),
        "the chain's end, so a stale reference reaches the current version in one hop: {seen}"
    );
    assert_eq!(
        entry["incoming"],
        json!([{"label": "supersedes", "source": third}]),
        "and the record that replaced this one directly, so the chain can be walked: {seen}"
    );
    assert!(
        entry["content"].is_null(),
        "carrying no content either way: {seen}"
    );
}

/// Store a memory that supersedes another, returning its id.
fn supersede(server: &mut Server, content: &str, target: &str) -> String {
    let stored = call(
        server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": content, "confidence": 0.8, "origin": "llm",
            "why": "so the chain has another link",
            // Superseding must state the replacement's keywords; these chain fixtures are about
            // ancestry rather than indexing, so empty is the truthful answer and is stated.
            "keywords": [],
            "links": [{"label": "supersedes", "target": target}]
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
    payload(&stored)["id"].as_str().expect("an id").to_owned()
}

// ---------------------------------------------------------------------------
// The echoed summary line, on the write that can still change it.
//
// The line is what a memory shows as in an index, and its first use is iterative: store, read the
// line, find the opening says nothing, rewrite the opening, read it again. `hoard_memorize` echoes
// on the first of those and `amend` is every one after it -- and amend is the right instrument for
// the whole loop, because the loop happens before export, which is exactly the window amend is
// open in.
// ---------------------------------------------------------------------------

/// Long enough that the line is a cut, and vague at the front, which is the case the loop exists
/// for: the memory is fine and its opening names nothing.
const A_VAGUE_OPENING: &str = "It turned out to be more complicated than expected, and after a good \
                               deal of measurement the answer was that the per-record envelope \
                               does not shrink with the prose.";

#[test]
fn amending_the_content_echoes_the_line_it_now_produces() {
    let mut server = a_server();
    let id = store(&mut server, A_VAGUE_OPENING, json!(["envelope"]));

    let rewritten = "The per-record envelope does not shrink with the prose, so writing shorter \
                     cannot control the cost. Measured at 411 characters.";
    let response = amend(&mut server, json!({"id": &id, "content": rewritten}));
    assert!(!is_error(&response), "{}", message(&response));

    assert_eq!(
        payload(&response)["summary"],
        json!(
            "The per-record envelope does not shrink with the prose, so writing shorter cannot \
               control the cost.\u{2026}"
        ),
        "the echo should show the line the *amended* content produces"
    );
}

/// **Only when the claim moved**, which is the rule the in-link remark beside it already follows: a
/// field that appears when nothing about it changed is a field callers learn to skip, and this one
/// is worth stopping for on the writes where it did.
#[test]
fn an_amendment_that_leaves_the_content_alone_echoes_no_line() {
    let mut server = a_server();
    let id = store(&mut server, A_VAGUE_OPENING, json!(["envelope"]));

    let response = amend(
        &mut server,
        json!({"id": &id, "confidence": 0.95, "keywords": ["record-envelope"]}),
    );
    assert!(!is_error(&response), "{}", message(&response));

    assert!(
        payload(&response).get("summary").is_none(),
        "the line did not change, so nothing should claim it did: {}",
        payload(&response)
    );
}

/// The whole loop, which is what the field is for: the first line names nothing, the second names
/// the memory, and the two come from the same projection so they can be compared at all.
#[test]
fn the_line_can_be_iterated_on_until_it_says_something() {
    let mut server = a_server();
    let id = store(&mut server, A_VAGUE_OPENING, json!(["envelope"]));
    let first = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["envelope"], "detail": "summary"}),
    ))["memories"][0]["summary"]
        .clone();

    let response = amend(
        &mut server,
        json!({
            "id": &id,
            "content": "The per-record envelope is 411 characters and does not shrink with the \
                        prose. It turned out to be more complicated than expected."
        }),
    );
    let echoed = payload(&response)["summary"].clone();

    assert_ne!(first, echoed, "the rewrite should change the line");
    let after = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["envelope"], "detail": "summary"}),
    ))["memories"][0]["summary"]
        .clone();
    assert_eq!(
        echoed, after,
        "the echo and the index must not be able to disagree about the line"
    );
}

/// **The gate routes; it does not deny.** The instrument is decided by a fact the caller cannot see,
/// so the refusal is the only place the caller learns which of two correct operations applies —
/// which makes it a signpost rather than a rejection, and it has to read as one.
///
/// This is guarded rather than left to prose because the earlier wording ("can no longer be
/// corrected", "correction is for a record that never entered service") measurably shaped how a
/// session regarded supersession: it arrived as the consolation for a refused first choice, and a
/// design question was filed on the strength of a cost the project nowhere claims. Nothing about the
/// mechanism changed; only what it sounded like.
#[test]
fn the_export_gate_names_the_instrument_rather_than_refusing() {
    let mut server = a_server();
    let backup = tempdir();
    let id = store(&mut server, "alpha governs routing", json!(["alpha"]));
    export_to(&mut server, &backup);

    let response = amend_as_a_client(&mut server, json!({"id": &id, "confidence": 0.9}));
    assert!(is_error(&response), "the gate should still hold");
    let text = message(&response).to_lowercase();

    assert!(
        text.contains("supersede"),
        "the refusal must name the instrument that applies: {text}"
    );
    assert!(
        text.contains("supersedes` link") || text.contains("supersedes link"),
        "it must say what to do, not only what to call it: {text}"
    );
    for denial in ["no longer", "cannot", "can not", "not allowed", "refused"] {
        assert!(
            !text.contains(denial),
            "the gate reads as a denial rather than a routing decision, on {denial:?}: {text}"
        );
    }
}

/// **An amendment cannot push a memory past the length limit either.**
///
/// The limit is a property of the memory rather than of the call that wrote it, so a door that did
/// not check it would not be a lenient door — it would be the way around the other one. Measured on
/// the memory as it would stand *after* the amendment, because amend replaces some fields and
/// leaves the rest, so neither the old text nor the new one is the length that matters.
#[test]
fn an_amendment_past_the_length_limit_is_refused() {
    let mut server = a_server();
    let stored = payload(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "short enough to store",
            "confidence": 0.8,
            "origin": "llm",
        }),
    ));
    let id = stored["id"].as_str().expect("an id").to_owned();

    let response = call(
        &mut server,
        "hoard_correct",
        json!({
            "action": "amend",
            "scope": "local",
            "id": id,
            "content": "alpha ".repeat(400),
        }),
    );

    assert!(is_error(&response), "{response}");
    let message = response["result"]["content"][0]["text"]
        .as_str()
        .expect("a message");
    assert!(
        message.contains("2000") && message.to_lowercase().contains("split"),
        "the same refusal the write door gives, for the same reason: {message}"
    );
}
