// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for `hoard_sets`, the lifecycle tool.
//!
//! Four actions so far. `status` and `create` between them close `no-tool-lists-loaded-scopes`: a
//! caller can discover which sets are live *and* bring a new one into existence, where before it was
//! told to "check the set name" against nothing. `export` writes a set to a file, which is what lets
//! one leave the machine at all, and `import` reconciles one back in.
//!
//! `documents/design/set-lifecycle.md` covers the rest (`restore`, `drop`).

use std::sync::Arc;

use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::mcp::Server;
use scatterhoard::memory::{
    Attribution, Author, Confidence, Memory, MemoryId, MemoryText, Origin, Scope, SetName,
};
use scatterhoard::memory_file::{self, Expectation};
use scatterhoard::store::{Store, StoreLayout};
use serde_json::{Value, json};

#[path = "support/review_all.rs"]
mod review_all;

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
                // a model that knows two words: placed in its three dimensions it would decide the
                // ranking of every test's handful of memories, because centring over so few moves the
                // query as much as the memories. See the same note in `tests/mcp.rs`.
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
    let mut path = std::env::temp_dir();
    path.push(format!("scatterhoard-sets-{}", uuid_like()));
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

/// Enough uniqueness for a scratch directory, without a dependency for it.
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_nanos();
    format!("{nanos}-{:?}", std::thread::current().id())
        .replace(['(', ')', ' '], "")
        .replace("ThreadId", "t")
}

/// A server holding only the local store, with a real directory so `create` has somewhere to work.
fn a_server(home: &TempDir) -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    server
}

/// Create a shared set and give it its charter, as its author would: since
/// `set-settled-at-first-write` a shared set's first memory has to be the charter. Not for a set
/// about to be taken on by `import`, which brings the file's charter and identity instead.
fn create_chartered(server: &mut Server, name: &str) {
    create_chartered_as(server, name, FIXTURE_AUTHOR);
}

/// [`create_chartered`], written under `author`: for a test about whose a set's records are.
fn create_chartered_as(server: &mut Server, name: &str, author: &str) {
    let created = sets(server, json!({"action": "create", "name": name}));
    assert!(!is_error(&created), "{created}");
    write_charter_as(server, name, author);
}

/// Say who this machine writes as in a shared set, as `hoard_sets` action `mint`.
fn name_author(server: &mut Server, set: &str, author: &str) {
    let minted = sets(
        server,
        json!({"action": "mint", "scope": {"shared": set}, "author": author}),
    );
    assert!(!is_error(&minted), "{}", message(&minted));
}

/// The name fixture sets are written under, where a test does not care whose they are.
const FIXTURE_AUTHOR: &str = "jtcours";

/// Write the fixture charter into a shared set that holds nothing yet, saying who writes it first
/// where nobody has: since `set-settled-at-first-write` step 3 a shared set's first write asks. A
/// `mint` refused because this machine already has an identity in the set is the answer already
/// given, so it is not an error here.
fn write_charter(server: &mut Server, name: &str) {
    write_charter_as(server, name, FIXTURE_AUTHOR);
}

/// [`write_charter`], naming `author` where nobody has been named yet.
fn write_charter_as(server: &mut Server, name: &str, author: &str) {
    let minted = sets(
        server,
        json!({"action": "mint", "scope": {"shared": name}, "author": author}),
    );
    assert!(
        !is_error(&minted) || message(&minted).contains("already"),
        "{}",
        message(&minted)
    );
    let charter = call(
        server,
        "hoard_memorize",
        json!({"scope": {"shared": name}, "content": FIXTURE_CHARTER, "why": "a fixture",
               "confidence": 1.0, "origin": "human",
               "keywords": ["hoard-scope-topics", "hoard-lang-en"]}),
    );
    assert!(!is_error(&charter), "{charter}");
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

fn sets(server: &mut Server, arguments: Value) -> Value {
    call(server, "hoard_sets", arguments)
}

/// Store a memory in the local set and return its id.
fn store_local(server: &mut Server, content: &str) -> String {
    let response = call(
        server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": content, "why": "a fixture",
            "confidence": 0.8, "origin": "llm"
        }),
    );
    payload(&response)["id"].as_str().expect("an id").to_owned()
}

/// The `local` entry from a `status` payload.
fn local_set(result: &Value) -> Value {
    result["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|set| set["scope"] == json!("local"))
        .expect("the local set")
        .clone()
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

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

#[test]
fn status_names_every_loaded_set() {
    let home = tempdir();
    let mut server = a_server(&home);

    let response = sets(&mut server, json!({"action": "status"}));
    assert!(!is_error(&response), "{}", message(&response));

    let listed = payload(&response);
    let scopes: Vec<&Value> = listed["sets"]
        .as_array()
        .expect("sets should be an array")
        .iter()
        .map(|set| &set["scope"])
        .collect();

    assert_eq!(scopes, vec![&json!("local")]);
}

#[test]
fn status_reports_how_many_memories_each_set_holds() {
    let home = tempdir();
    let mut server = a_server(&home);

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha is worth remembering",
            "why": "so the count is not zero",
            "confidence": 0.7,
            "origin": "llm"
        }),
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(listed["sets"][0]["memories"], json!(1));
}

/// A set created in this session shows up in `status` without a restart, because the same call that
/// creates it makes it reachable.
#[test]
fn status_includes_a_set_created_this_session() {
    let home = tempdir();
    let mut server = a_server(&home);

    sets(
        &mut server,
        json!({"action": "create", "name": "coding-lessons"}),
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let scopes: Vec<Value> = listed["sets"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|set| set["scope"].clone())
        .collect();

    assert!(
        scopes.contains(&json!("shared:coding-lessons")),
        "created set missing from status: {scopes:?}"
    );
}

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

/// The whole point of `create`: a set that did not exist can be written to afterwards. Before this,
/// `hoard_memorize` refused every shared scope and nothing could make one exist.
#[test]
fn a_created_set_can_be_memorized_into() {
    let home = tempdir();
    let mut server = a_server(&home);

    let created = sets(
        &mut server,
        json!({"action": "create", "name": "coding-lessons"}),
    );

    write_charter(&mut server, "coding-lessons");
    assert!(!is_error(&created), "{}", message(&created));

    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "coding-lessons"},
            "content": "a lesson worth keeping across projects",
            "why": "it is the first memory this set has ever held",
            "confidence": 0.8,
            "origin": "llm"
        }),
    );

    assert!(!is_error(&stored), "{}", message(&stored));
    assert!(payload(&stored)["id"].is_string());
}

/// A created set gets the server's model, so its memories are findable by meaning. This is
/// `project-stores-get-no-embedder` guarded at the one place that can now make a new store.
#[test]
fn a_created_set_is_searchable_by_meaning() {
    let home = tempdir();
    let mut server = a_server(&home);

    create_chartered(&mut server, "lessons");
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "lessons"},
            "content": "alpha alpha alpha is the thing to find",
            "why": "so a semantic query has a target",
            "confidence": 0.9,
            "origin": "llm"
        }),
    );

    let found = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": {"shared": "lessons"}}),
    ));

    assert_eq!(found["semantic_search_available"], json!(true));
    assert!(
        found["memories"][0]["similarity"].is_number(),
        "a created set's memory should carry a similarity: {found}"
    );
}

#[test]
fn create_writes_a_database_that_outlives_the_server() {
    let home = tempdir();
    {
        let mut server = a_server(&home);
        sets(&mut server, json!({"action": "create", "name": "durable"}));
    }

    let reopened = Store::open(
        Scope::Shared(SetName::new("durable").expect("valid")),
        home.path(),
    );
    assert!(reopened.is_ok(), "the created database should reopen");
}

#[test]
fn create_refuses_a_set_that_is_already_loaded() {
    let home = tempdir();
    let mut server = a_server(&home);

    sets(&mut server, json!({"action": "create", "name": "twice"}));
    let again = sets(&mut server, json!({"action": "create", "name": "twice"}));

    assert!(is_error(&again));
    assert!(
        message(&again).contains("twice"),
        "the refusal should name the set: {}",
        message(&again)
    );
}

#[test]
fn create_refuses_a_malformed_set_name() {
    let home = tempdir();
    let mut server = a_server(&home);

    let response = sets(
        &mut server,
        json!({"action": "create", "name": "not a name"}),
    );
    assert!(is_error(&response));
}

/// `create` needs somewhere to put the database. A server without one says so rather than failing
/// obscurely inside the store layer.
#[test]
fn create_without_a_data_directory_says_which_is_missing() {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));

    let response = sets(&mut server, json!({"action": "create", "name": "nowhere"}));
    assert!(is_error(&response));
    assert!(
        message(&response).contains("data directory"),
        "unhelpful refusal: {}",
        message(&response)
    );
}

/// `status` and `recall` must not disagree about whether a set can be searched by meaning.
///
/// They did. `status` first read the *server's* embedder while `recall` asks the *store's*, so a
/// server whose store was given a model directly reported `false` from one tool and `true` from the
/// other in the same session. Found by an approval snapshot on the first run, not by anything here —
/// this test is what keeps it fixed. The store-not-server wiring below is exactly the shape that
/// exposed it.
#[test]
fn status_and_recall_agree_on_searchability() {
    let mut store = Store::open_in_memory(Scope::Local).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(store);

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha is here to be recalled",
            "why": "so recall has something to answer with",
            "confidence": 0.7,
            "origin": "llm"
        }),
    );

    let recalled = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ));
    let listed = payload(&sets(&mut server, json!({"action": "status"})));

    let local = listed["sets"]
        .as_array()
        .expect("an array")
        .iter()
        .find(|set| set["scope"] == json!("local"))
        .expect("the local set should be listed")
        .clone();

    assert_eq!(
        local["semantic_search_available"], recalled["semantic_search_available"],
        "status and recall disagree: status {} vs recall {}",
        local["semantic_search_available"], recalled["semantic_search_available"]
    );
}

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

/// The one memory file under `home`: a shared set's in its reserved directory, whatever it is
/// called, or else `local`'s backup beside it.
///
/// Found rather than named, because a file's name is only an approximation of its set's and the
/// tests that care what it is called say so themselves. A second `.json` there is a panic rather
/// than a guess: a test holding two sets names the one it means.
fn memories_file(home: &TempDir) -> std::path::PathBuf {
    let files = json_files_in(home);
    match files.as_slice() {
        [only] => only.clone(),
        [] => local_backup(home),
        several => panic!("more than one memory file: {several:?}"),
    }
}

/// Every `.json` file in `home`'s reserved directory, sorted.
fn json_files_in(home: &TempDir) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(home.path().join("scatterhoard_mem")) else {
        return Vec::new();
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .map(|entry| entry.expect("reads the directory").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    files
}

fn store_one(server: &mut Server, content: &str) {
    call(
        server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": content,
            "why": "so there is something to export",
            "confidence": 0.7,
            "origin": "llm"
        }),
    );
}

#[test]
fn export_writes_the_well_known_file_under_the_named_directory() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_with_notes(&home, "alpha is worth exporting");

    let response = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    );
    assert!(!is_error(&response), "{}", message(&response));

    let path = memories_file(&out);
    assert!(path.is_file(), "no file at {}", path.display());
    // One more than the test's own writes: the set's charter, which its first memory has to be.
    assert_eq!(payload(&response)["memories"], json!(2));
}

/// The caller names a directory and the filename is appended, because trust is established by path
/// before anything reads the contents — a caller-chosen filename would make the header authoritative.
#[test]
fn export_refuses_a_directory_that_does_not_exist() {
    let home = tempdir();
    let mut server = a_server_with_notes(&home, "a memory bound nowhere");

    let response = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": "/nonexistent/nowhere"}),
    );
    assert!(is_error(&response));
    assert!(
        message(&response).contains("directory"),
        "unhelpful refusal: {}",
        message(&response)
    );
}

#[test]
fn an_exported_file_is_a_header_then_one_line_per_memory() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha comes first");
    store_one(&mut server, "beta comes second");

    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&out)}),
    );

    let text = std::fs::read_to_string(memories_file(&out)).expect("reads");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "header plus two memories: {text}");

    let header: Value = serde_json::from_str(lines[0]).expect("header parses");
    assert_eq!(header["scope"], json!("local"));
    assert!(header["format_version"].is_string());

    for line in &lines[1..] {
        let memory: Value = serde_json::from_str(line).expect("memory parses");
        assert!(memory["id"].is_string());
    }
}

/// Records are written in file order (`memory_file::file_position`), not id order. The streaming
/// writer cannot sort what it has not seen, so this is really a test that the store yields them
/// ordered. Four ids share both orders one time in twenty-four, so `tests/store.rs` pins the order
/// with chosen ids; this checks the export path end to end.
#[test]
fn exported_records_are_in_file_order() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    for content in ["first alpha", "second beta", "third gamma", "fourth delta"] {
        store_one(&mut server, content);
    }

    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&out)}),
    );

    let text = std::fs::read_to_string(memories_file(&out)).expect("reads");
    let ids: Vec<scatterhoard::memory::MemoryId> = text
        .lines()
        .skip(1)
        .map(|line| {
            let value = serde_json::from_str::<Value>(line).expect("parses");
            scatterhoard::memory::MemoryId::parse(value["id"].as_str().expect("an id"))
                .expect("a valid id")
        })
        .collect();

    let mut sorted = ids.clone();
    sorted.sort_by_key(|id| scatterhoard::memory_file::file_position(*id));
    assert_eq!(ids, sorted, "export is not in file order: {ids:?}");
}

/// Exporting twice must replace rather than append, and must stay byte-identical when nothing
/// changed — that stability is what keeps a re-export from looking like a rewrite in a diff.
#[test]
fn exporting_twice_produces_the_same_bytes() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is stable");

    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&out)}),
    );
    let first = std::fs::read_to_string(memories_file(&out)).expect("reads");

    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&out)}),
    );
    let second = std::fs::read_to_string(memories_file(&out)).expect("reads");

    assert_eq!(first, second);
}

/// No temporary file is left behind: the write goes through a rename so a crash cannot leave a
/// truncated file that `git` would happily commit.
#[test]
fn export_leaves_no_temporary_file() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is worth exporting");

    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&out)}),
    );

    assert!(
        !out.path()
            .join("scatterhoard_mem/memories.json.tmp")
            .exists(),
        "a temporary file survived the export"
    );
}

/// The file we write has to be a file we can read, with the memories intact. Without this, export and
/// import could each be internally consistent and disagree at the seam — which is the failure mode
/// this project has recorded four times.
#[test]
fn an_exported_file_reads_back_with_its_memories_intact() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha governs how requests are routed",
            "why": "the reasoning has to survive the round trip",
            "context": "and so does the context",
            "keywords": ["Alpha-Routing", "MemoryView"],
            "confidence": 0.9,
            "origin": "joint"
        }),
    );

    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&out)}),
    );

    let file = std::fs::File::open(memories_file(&out)).expect("opens");
    let set = memory_file::read(&Expectation::Exactly(Scope::Local), file).expect("reads back");

    assert_eq!(set.scope, Scope::Local);
    assert_eq!(set.memories.len(), 2);

    let memory = set
        .memories
        .iter()
        .find(|memory| !memory.keywords.iter().any(|k| k.as_str() == "hoard-author"))
        .expect("the stored memory, not the set's identity record");
    assert_eq!(
        memory.content.as_data(),
        "alpha governs how requests are routed"
    );
    assert!(memory.why.is_some(), "why did not survive the round trip");
    assert!(
        memory.context.is_some(),
        "context did not survive the round trip"
    );
    assert_eq!(
        memory
            .keywords
            .iter()
            .map(|keyword| keyword.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha-Routing", "MemoryView"],
        "keyword spelling must survive export, not just storage"
    );
}

#[test]
fn export_refuses_a_set_that_is_not_loaded() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    let response = sets(
        &mut server,
        json!({"action": "export", "scope": {"shared": "absent"}, "directory": out.path()}),
    );
    assert!(is_error(&response));
    assert!(message(&response).contains("hoard_sets"));
}

/// Every export says whether the prohibited-data scan ran, and now it does.
///
/// *This test asserted `false` until 2026-08-20, pinning the hardcoded literal that stood in for the
/// unbuilt scan. The literal is gone; what it was guarding — that the response states the fact
/// rather than leaving it to be assumed — is what survives here.*
#[test]
fn export_says_the_prohibited_data_scan_ran() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha might contain anything");

    let listed = payload(&sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&out)}),
    ));
    assert_eq!(listed["prohibited_data_scanned"], json!(true));
}

/// **The note carries what the boolean cannot, and what it has to carry has changed.**
///
/// `prohibited-data-scanned-conflates-skipped-with-unbuilt`, found 2026-08-11: a bare `false` meant
/// *this is local, the gate does not apply* and *the scan does not exist* alike. The second reason
/// is gone as of 2026-08-20, so the distinction the note must now draw is a different one — between
/// a set the gate does not govern and a set it does — and, in both cases, between *those shapes were
/// absent* and *this set holds no secret*. The second is the claim a shape-based scan cannot make,
/// and the one a reader will hear if nothing stops them.
#[test]
fn export_says_what_the_prohibited_data_scan_covers() {
    let home = tempdir();
    let out = tempdir();
    let shared_out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha might contain anything");
    create_chartered(&mut server, "team");
    store_into(&mut server, "team", "beta might contain anything");

    let local = payload(&sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&out)}),
    ));
    let local_note = local["prohibited_data_note"].as_str().unwrap_or_default();
    assert!(
        local_note.contains("does not apply"),
        "a local export is outside the gate by design and should still say so: {local}"
    );

    let shared = payload(&sets(
        &mut server,
        json!({"action": "export", "scope": {"shared": "team"}, "directory": shared_out.path()}),
    ));
    let shared_note = shared["prohibited_data_note"].as_str().unwrap_or_default();
    assert_ne!(
        local_note, shared_note,
        "the two situations are different and a caller has to be able to tell them apart"
    );

    for note in [local_note, shared_note] {
        assert!(
            !note.contains("exists yet"),
            "the scan exists; claiming a gap would be the old note surviving its own fix: {note}"
        );
        assert!(
            note.contains("password"),
            "the honest limit belongs in the note, not in a design document nobody reads at \
             export time: {note}"
        );
        assert!(
            note.contains("shape"),
            "matching shapes rather than wording is what makes clean mean something narrow: {note}"
        );
    }
}

/// A first export to a new directory says nothing; a second export elsewhere remarks on it. A second
/// checkout is legitimate and a typo is not, and only the caller can tell which — so it is a note,
/// never a refusal.
#[test]
fn exporting_somewhere_new_is_remarked_on_but_allowed() {
    let home = tempdir();
    let first = tempdir();
    let second = tempdir();
    let mut server = a_server_with_notes(&home, "alpha travels");

    let one = payload(&sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": first.path()}),
    ));
    assert!(
        one.get("note").is_none(),
        "a first export has nothing to compare against: {one}"
    );

    let two = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": second.path()}),
    );
    assert!(!is_error(&two), "a second target must not be refused");
    assert!(
        payload(&two)["note"].is_string(),
        "an unfamiliar target should be remarked on: {}",
        payload(&two)
    );
    assert!(memories_file(&second).is_file());
}

// ---------------------------------------------------------------------------
// import
// ---------------------------------------------------------------------------

/// A server holding a shared set, `notes`, with one memory in it -- for tests about the `directory`
/// form itself, which the personal set may not use (`the-reserved-directory-accepts-the-local-set`).
fn a_server_with_notes(home: &TempDir, content: &str) -> Server {
    let mut server = a_server(home);
    create_chartered(&mut server, "notes");
    store_into(&mut server, "notes", content);
    server
}

/// Back `local` up into `out`, as a `file`: the reserved directory never holds the personal set
/// (`the-reserved-directory-accepts-the-local-set`).
fn export_to(server: &mut Server, out: &TempDir) -> Value {
    sets(
        server,
        json!({"action": "export", "scope": "local", "file": local_backup(out)}),
    )
}

fn import_from(server: &mut Server, out: &TempDir) -> Value {
    sets(
        server,
        json!({"action": "import", "scope": "local", "file": local_backup(out)}),
    )
}

/// Where these tests back `local` up to.
fn local_backup(out: &TempDir) -> std::path::PathBuf {
    out.path().join("local.json")
}

/// Rewrite one memory's content directly in the file, as a pull from someone else would.
fn edit_in_file(out: &TempDir, from: &str, to: &str) {
    let path = memories_file(out);
    let text = std::fs::read_to_string(&path).expect("reads");
    assert!(text.contains(from), "{from:?} not in the file");
    std::fs::write(&path, text.replace(from, to)).expect("writes");
}

/// Export then import with nothing changed: every record is unchanged and nothing is written.
#[test]
fn importing_an_unchanged_file_changes_nothing() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is settled");

    export_to(&mut server, &out);
    let report = payload(&import_from(&mut server, &out));

    assert_eq!(report["outcomes"]["unchanged"], json!(2));
    assert!(report.get("conflicts").is_none(), "{report}");
    assert!(report.get("anomalies").is_none(), "{report}");
}

/// The file moved and the store did not, so the file's version wins — which is a pull arriving.
#[test]
fn a_change_only_in_the_file_is_taken() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha as first written");

    export_to(&mut server, &out);
    edit_in_file(
        &out,
        "alpha as first written",
        "alpha as someone else revised it",
    );

    let report = payload(&import_from(&mut server, &out));
    assert_eq!(report["outcomes"]["took_from_file"], json!(1));

    let found = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 10}),
    ));
    let contents: Vec<&str> = found["memories"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|memory| memory["content"].as_str().expect("content"))
        .collect();
    assert!(
        contents.contains(&"alpha as someone else revised it"),
        "the file's version did not reach the store: {contents:?}"
    );
}

/// **The defect this whole design exists to prevent.** Pull first, import second, with unexported
/// local work in the store: that work must survive.
#[test]
fn importing_after_a_pull_keeps_unexported_local_work() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha was exported");

    export_to(&mut server, &out);

    // Local work that has never been exported, so the file has never heard of it.
    store_one(&mut server, "beta was written after the export");

    let report = payload(&import_from(&mut server, &out));
    assert_eq!(
        report["outcomes"]["kept_local"],
        json!(1),
        "local-only work was not kept: {report}"
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(
        listed["sets"][0]["memories"],
        json!(2),
        "importing destroyed a memory the file did not carry"
    );
}

/// A record that differs between store and file with **no recorded agreement** is a conflict, because
/// there is no ancestor to judge by. Nothing is written and the id is named.
///
/// Reached through a second checkout, which is a real scenario rather than a contrivance: the store
/// has agreed with one directory and is handed a diverged file from another it has never reconciled
/// with. That matters, because the *other* route to a conflict — the same memory edited on both sides
/// after an agreement — **cannot currently be produced through the tool surface at all**, since
/// nothing can modify an existing memory. `hoard_memorize` always mints a new id. So that branch is
/// covered by the decision-table tests in `tests/reconcile.rs` and is unreachable end-to-end until
/// amendment exists; see the note on this in `TRACKING.md`.
#[test]
fn a_diverged_file_with_no_recorded_agreement_is_a_conflict() {
    let home = tempdir();
    let agreed = tempdir();
    let diverged = tempdir();
    let mut server = a_server(&home);

    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha as the store holds it",
            "why": "the original reasoning",
            "confidence": 0.7,
            "origin": "llm"
        }),
    );
    let id = payload(&stored)["id"].as_str().expect("an id").to_owned();

    // An agreement with one directory, so the store's own copy is settled there.
    export_to(&mut server, &agreed);

    // A second directory carrying a diverged copy of the same record, never reconciled with.
    std::fs::create_dir_all(diverged.path().join("scatterhoard_mem")).expect("creates");
    let text = std::fs::read_to_string(memories_file(&agreed)).expect("reads");
    std::fs::write(
        memories_file(&diverged),
        text.replace(
            "alpha as the store holds it",
            "alpha as the other copy has it",
        ),
    )
    .expect("writes");

    let report = payload(&import_from(&mut server, &diverged));

    assert_eq!(
        report["outcomes"]["conflict"],
        json!(1),
        "expected one conflict: {report}"
    );
    assert_eq!(report["conflicts"], json!([id]));
    assert!(
        report["conflict_guidance"].is_string(),
        "a conflict must come with guidance a caller can act on: {report}"
    );

    // Nothing was written for it: the store keeps its own version.
    let found = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 10}),
    ));
    let contents: Vec<&str> = found["memories"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|memory| memory["content"].as_str().expect("content"))
        .collect();
    assert_eq!(contents, vec!["alpha as the store holds it"]);
}

/// **The conflict a real user hits**, and it is reachable from a client through two checkouts: one
/// memory, changed on both sides since this store and *this* file last agreed. Nothing is written
/// for it and the id is named.
///
/// **The store's side of the divergence arrives by import**, not by amendment, and that is the
/// design rather than a convenience. Since `correction-gated-on-service` an exported record cannot
/// be corrected at all, so a store-side change to one can now only come from a `forget` or from an
/// import against a different target — which is exactly the second checkout modelled here. The
/// earlier version of this test amended after exporting, which the server now refuses.
///
/// The branch was unreachable from any client until `hoard_correct` existed
/// (`no-tool-can-modify-a-memory`); this keeps it reachable by the route that survived.
#[test]
fn a_memory_changed_on_both_sides_is_a_conflict() {
    let home = tempdir();
    let out = tempdir();
    let elsewhere = tempdir();
    let mut server = a_server(&home);

    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha as both sides first had it",
            "why": "the shared starting point",
            "keywords": ["agreed"],
            "confidence": 0.5,
            "origin": "llm"
        }),
    );
    let id = payload(&stored)["id"].as_str().expect("an id").to_owned();

    // Both checkouts now agree with the store, and a base records each agreement.
    export_to(&mut server, &out);
    export_to(&mut server, &elsewhere);

    // Our change: taken from the second checkout, which moves the store away from `out`'s base
    // without touching it.
    edit_in_file(&elsewhere, "\"agreed\"", "\"our-keyword\"");
    let ours = payload(&import_from(&mut server, &elsewhere));
    assert_eq!(ours["outcomes"]["took_from_file"], json!(1), "{ours}");

    // Their change, to the same memory, arriving in the first checkout as a pull would deliver it.
    edit_in_file(&out, "\"agreed\"", "\"their-keyword\"");

    let report = payload(&import_from(&mut server, &out));

    assert_eq!(
        report["outcomes"]["conflict"],
        json!(1),
        "both sides changed one memory and it was not reported as a conflict: {report}"
    );
    assert_eq!(report["conflicts"], json!([id]));

    // Nothing was written for it: our version stands until someone resolves it.
    let memory =
        payload(&call(&mut server, "hoard_lookup", json!({"id": &id})))["memories"][0].clone();
    assert_eq!(
        memory["keywords"],
        json!(["our-keyword"]),
        "a conflicted memory was overwritten: {memory}"
    );
}

/// **A tombstone says what it forgot, so an import can answer it without a base.**
/// `tombstone-carries-the-checksum-it-forgot`: matching what the receiver holds means the forget is
/// sequential and applies; differing means the sender forgot a version the receiver never had, so
/// the two are concurrent and it is a conflict.
///
/// Both tests below import from a directory this store has **never agreed with**, which is the case
/// the checksum exists for — with a base the three-way arithmetic already answers it, and *without*
/// one is exactly what goes missing in every defect filed against reconciliation. The tombstone is
/// produced by a second machine actually forgetting the memory rather than hand-written here, so
/// the checksum is computed by the code under test rather than by the test.
#[test]
fn a_tombstone_forgetting_the_version_we_hold_applies_without_a_base() {
    let home = tempdir();
    let ours = tempdir();
    let theirs = tempdir();
    let mut server = a_server(&home);
    let id = store_local(&mut server, "alpha will be forgotten elsewhere");
    export_to(&mut server, &ours);

    let forgotten_elsewhere = their_forget_of(&ours, &theirs, &id);
    assert!(
        !is_error(&forgotten_elsewhere),
        "{}",
        message(&forgotten_elsewhere)
    );

    let report = payload(&import_from(&mut server, &theirs));
    assert_eq!(
        report["outcomes"]["took_from_file"],
        json!(1),
        "a forget of the version we hold is sequential and should apply: {report}"
    );
    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(local_set(&listed)["memories"], json!(0));
    assert_eq!(local_set(&listed)["forgotten"], json!(1));
}

/// The other direction, and the one the entry names: the sender forgot a version this store never
/// had, so nothing here is the thing they removed.
#[test]
fn a_tombstone_forgetting_a_version_we_never_had_is_a_conflict() {
    let home = tempdir();
    let ours = tempdir();
    let theirs = tempdir();
    let mut server = a_server(&home);
    let id = store_local(&mut server, "alpha as we have it");
    export_to(&mut server, &ours);

    let forgotten_elsewhere = their_forget_of(&ours, &theirs, &id);
    assert!(
        !is_error(&forgotten_elsewhere),
        "{}",
        message(&forgotten_elsewhere)
    );

    // Our change to the memory they forgot, arriving from a third checkout. It has to come that way
    // rather than by amendment: since `correction-gated-on-service` an exported record cannot be
    // corrected, so an import against another target is the route a store-side change now takes.
    let third = tempdir();
    export_to(&mut server, &third);
    edit_in_file(
        &third,
        "alpha as we have it",
        "alpha as we have since revised it",
    );
    let ours_now = payload(&import_from(&mut server, &third));
    assert_eq!(
        ours_now["outcomes"]["took_from_file"],
        json!(1),
        "{ours_now}"
    );

    let report = payload(&import_from(&mut server, &theirs));
    assert_eq!(
        report["outcomes"]["conflict"],
        json!(1),
        "forgetting a version we never held is concurrent, not sequential: {report}"
    );
    assert_eq!(report["conflicts"], json!([id]));
    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(
        local_set(&listed)["memories"],
        json!(1),
        "a conflicted forget must not remove the memory"
    );
}

/// A second machine that adopts the set from `source`, forgets `id`, and exports to `destination`.
///
/// The tombstone that lands there is produced by the real `forget`, so what it says it forgot is
/// whatever the code computes rather than whatever a fixture asserts.
fn their_forget_of(source: &TempDir, destination: &TempDir, id: &str) -> Value {
    let their_home = tempdir();
    let mut theirs = a_server(&their_home);
    let imported = sets(
        &mut theirs,
        json!({"action": "import", "scope": "local", "file": local_backup(source)}),
    );
    assert!(!is_error(&imported), "{}", message(&imported));

    // **Saying who this machine is, before it writes to a set it has just taken on.** The import
    // brought the *other* holder's identity record with it, so writing without answering would
    // mean guessing which record is this user -- which the design refuses, because a wrong guess
    // forks one person into two identities and nothing downstream can report it. Spelled as the
    // `author` action for now; `identify` is what it becomes.
    let identified = sets(
        &mut theirs,
        json!({"action": "mint", "scope": "local", "author": "them"}),
    );
    assert!(!is_error(&identified), "{}", message(&identified));

    let forgotten = call(
        &mut theirs,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": id}),
    );
    assert!(!is_error(&forgotten), "{}", message(&forgotten));
    sets(
        &mut theirs,
        json!({"action": "export", "scope": "local", "file": local_backup(destination)}),
    )
}

// ---------------------------------------------------------------------------
// Can two contents live under one id? (`falsify-record-immutability`)
// ---------------------------------------------------------------------------

/// **The premise, and it now holds**: since `correction-gated-on-service`, one id never holds two
/// memory contents, because every post-export change is a supersession (new id) or a forget (a
/// tombstone). [The graph is the ancestor](documents/design/multi-set-sync.md) rests on it entirely.
///
/// **It was false until 2026-08-31, and `absorb` was the only route.** Two holders of one file, one
/// passing `import`'s `absorbing` argument and one not, landed on different content for one id with
/// no amendment anywhere — the receiver-side rewrite that made absorption an exception to the rule
/// every other change obeys. This test is what that one became: the same two holders, the same
/// file, and the merge now expressed as a record rather than a rewrite.
///
/// The absorbing holder's work reaches the other as an **addition** — a new memory with a new id —
/// so it imports with no conflict, and the two agree afterwards on every id either of them held.
#[test]
fn absorbing_reaches_the_other_holder_as_a_record_rather_than_a_conflict() {
    let author_home = tempdir();
    let repository = tempdir();
    let elsewhere = tempdir();
    let out = tempdir();
    let (other_id, other_file) = a_file_from_other(&author_home, &out);

    // One machine authors a memory in `main` pointing into `other`, and exports.
    let id = {
        let main_home = tempdir();
        let (mut authoring, id) = a_main_linking_into_other(&main_home, &other_id);
        sets(
            &mut authoring,
            json!({"action": "export", "scope": {"shared": "main"}, "directory": repository.path()}),
        );
        id
    };

    // Two holders adopt the same file. One then absorbs `other`; the other never hears of it.
    let absorbing_home = tempdir();
    let mut absorbing = adopt_main(&absorbing_home, &repository);
    let plain_home = tempdir();
    let mut plain = adopt_main(&plain_home, &repository);

    let safety = tempdir();
    let absorbed = absorb_into_main(
        &mut absorbing,
        &other_file,
        &safety.path().join("before.json"),
    );
    assert!(!is_error(&absorbed), "{}", message(&absorbed));

    // The record both hold is untouched on both sides. That is the claim the lattice needs.
    assert_eq!(
        look_up_in(&mut absorbing, &id)["links"],
        look_up_in(&mut plain, &id)["links"],
        "absorbing must not change a record the other holder also has"
    );

    sets(
        &mut absorbing,
        json!({"action": "export", "scope": {"shared": "main"}, "directory": elsewhere.path()}),
    );
    let report = payload(&sets(
        &mut plain,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": elsewhere.path()}),
    ));
    assert_eq!(
        report["outcomes"]["conflict"],
        json!(serde_json::Value::Null),
        "the merge arrives as new records, not as a conflict: {report}"
    );
    let tips = look_up_in(&mut plain, &id)["superseded_by"]
        .as_array()
        .expect("the supersession travelled")
        .len();
    assert_eq!(tips, 1, "the other holder now has the merge as history");
}

/// **And the paths that do hold**, so the finding above is isolated rather than assumed. Every other
/// way a memory reaches a store writes the file's bytes unchanged, so re-importing anywhere lands
/// the same content: adopting the set, restoring from the file, and importing again after both.
#[test]
fn every_other_write_path_lands_the_authors_bytes() {
    let author_home = tempdir();
    let repository = tempdir();
    let id = {
        let mut authoring = a_server(&author_home);
        create_chartered(&mut authoring, "main");
        let stored = call(
            &mut authoring,
            "hoard_memorize",
            json!({
                "scope": {"shared": "main"},
                "content": "alpha, authored once",
                "why": "so there is something to carry unchanged",
                "confidence": 0.7,
                "origin": "llm",
                "links": [{
                    "label": "related_to",
                    "target": "01M0K23W24FA1B2WVCNTZEC4M3",
                    "target_set": "other",
                }],
            }),
        );
        sets(
            &mut authoring,
            json!({"action": "export", "scope": {"shared": "main"}, "directory": repository.path()}),
        );
        payload(&stored)["id"].as_str().expect("an id").to_owned()
    };

    let home = tempdir();
    let mut server = adopt_main(&home, &repository);
    let adopted = look_up_in(&mut server, &id);

    // Restore from the same file, then import it again. Neither may change the record.
    let safety = tempdir();
    let restored = sets(
        &mut server,
        json!({
            "action": "restore",
            "scope": {"shared": "main"},
            "directory": repository.path(),
            "save_to": safety.path().join("before.json"),
        }),
    );
    assert!(!is_error(&restored), "{}", message(&restored));
    assert_eq!(
        look_up_in(&mut server, &id),
        adopted,
        "restore changed a record"
    );

    let reimported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": repository.path()}),
    );
    assert!(!is_error(&reimported), "{}", message(&reimported));
    assert_eq!(
        look_up_in(&mut server, &id),
        adopted,
        "a second import changed a record"
    );
}

// ---------------------------------------------------------------------------
// `absorb`: taking one set's file into another as an authored supersession
// ---------------------------------------------------------------------------

/// A file exported from a set called `other`, written by a machine that is not the one absorbing
/// it. That separation is the workflow rather than test convenience: the absorbing machine need
/// never have held the set, which is what makes **the file** the input and not a loaded store.
fn a_file_from_other(home: &TempDir, out: &TempDir) -> (String, std::path::PathBuf) {
    let mut authoring = a_server(home);
    sets(&mut authoring, json!({"action": "create", "name": "other"}));
    // A distinctive author, so "written as authored" is a claim the test can actually check: with
    // both servers defaulting to one name, an assertion about attribution would pass either way.
    // Minted before the charter, which would otherwise be written under the default name.
    sets(
        &mut authoring,
        json!({"action": "mint", "scope": {"shared": "other"}, "author": "someone-else"}),
    );
    write_charter(&mut authoring, "other");
    let stored = call(
        &mut authoring,
        "hoard_memorize",
        json!({
            "scope": {"shared": "other"},
            "content": "beta, authored in the set that gets absorbed",
            "why": "so absorption has a record to carry across",
            "confidence": 0.7,
            "origin": "llm",
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
    let file = out.path().join("other.json");
    let exported = sets(
        &mut authoring,
        json!({"action": "export", "scope": {"shared": "other"}, "file": &file}),
    );
    assert!(!is_error(&exported), "{}", message(&exported));
    (
        payload(&stored)["id"].as_str().expect("an id").to_owned(),
        file,
    )
}

/// A server holding `main`, with one memory whose link points into `other`.
fn a_main_linking_into_other(home: &TempDir, target: &str) -> (Server, String) {
    let mut server = a_server(home);
    create_chartered(&mut server, "main");
    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": "alpha, which points into the set being absorbed",
            "why": "so there is a link that absorption makes internal",
            "confidence": 0.7,
            "origin": "llm",
            "links": [{"label": "related_to", "target": target, "target_set": "other"}],
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
    (
        server,
        payload(&stored)["id"].as_str().expect("an id").to_owned(),
    )
}

fn absorb_into_main(
    server: &mut Server,
    file: &std::path::Path,
    save_to: &std::path::Path,
) -> Value {
    sets(
        server,
        json!({
            "action": "absorb",
            "scope": {"shared": "main"},
            "file": file,
            "save_to": save_to,
        }),
    )
}

/// The whole of the operation, asserted in one place: the arriving records land as their author
/// wrote them, and the links that have become internal are changed by **authoring a supersession**
/// rather than by rewriting what is already there.
///
/// `documents/design/multi-set-sync.md`, *absorption becomes a supersession*. The original keeping
/// its cross-set link is not a leftover — it is the history, and it is what makes this the same
/// rule as every other post-export change.
#[test]
fn absorb_takes_a_file_declaring_another_set_into_this_one() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);
    let home = tempdir();
    let (mut server, main_id) = a_main_linking_into_other(&home, &other_id);

    let safety = tempdir();
    let absorbed = absorb_into_main(&mut server, &file, &safety.path().join("before.json"));
    assert!(!is_error(&absorbed), "{}", message(&absorbed));

    // The arriving record is now in `main`, under its own id and with its own author's name on it.
    let arrived = look_up_in(&mut server, &other_id);
    // Resolved through the identity record, because `by` carries an id now -- and what the record
    // must keep is the *author it arrived with*, not one this server would have stamped on it.
    let arriving_identity = look_up_in(
        &mut server,
        arrived["created"]["by"]
            .as_str()
            .expect("an author record id"),
    );
    assert_eq!(
        arriving_identity["content"],
        json!("someone-else"),
        "an absorbed record must arrive as authored, not re-attributed: {arrived}"
    );

    // The original keeps the link it was written with. Anything else would be the rewrite this
    // operation exists to remove.
    let original = look_up_in(&mut server, &main_id);
    assert_eq!(
        original["links"][0]["target"]["set"],
        json!("other"),
        "the original must keep its cross-set link as history: {original}"
    );

    // And it has been superseded by one saying the same thing with the link made internal.
    let tips = original["superseded_by"]
        .as_array()
        .expect("a superseded memory names its tip");
    assert_eq!(tips.len(), 1, "one supersession, not several: {original}");
    let tip = look_up_in(&mut server, tips[0].as_str().expect("an id"));
    assert_eq!(
        tip["content"], original["content"],
        "the supersession makes the same claim: {tip}"
    );
    assert!(
        tip["links"]
            .as_array()
            .expect("links")
            .iter()
            .any(|link| link["target"] == json!(other_id)),
        "the superseding memory should link into the absorbed record as its own set's: {tip}"
    );
}

/// **An absorb that brings a second charter completes.** Both sets have charters, and this set's
/// charter links into the set being absorbed, so absorbing merges in the other charter and then
/// supersedes this one to make its link internal. That replacement stands beside the absorbed
/// charter; refused as a second charter, it would stop the absorb with the records already merged.
/// The two charters are left for the caller to reconcile, which `hoard_sets status` shows.
/// `one-live-charter-per-set`.
#[test]
fn absorbing_a_set_with_its_own_charter_completes_when_this_charter_links_into_it() {
    let author_home = tempdir();
    let out = tempdir();
    let mut authoring = a_server(&author_home);
    sets(&mut authoring, json!({"action": "create", "name": "other"}));
    sets(
        &mut authoring,
        json!({"action": "mint", "scope": {"shared": "other"}, "author": "someone-else"}),
    );
    let theirs = call(
        &mut authoring,
        "hoard_memorize",
        json!({
            "scope": {"shared": "other"},
            "content": "beta, what the set being absorbed is for",
            "why": "so the absorbed set brings a charter of its own",
            "confidence": 0.9,
            "origin": "human",
            "keywords": ["hoard-scope-topics"],
        }),
    );
    assert!(!is_error(&theirs), "{}", message(&theirs));
    let theirs = payload(&theirs)["id"].as_str().expect("an id").to_owned();
    let file = out.path().join("other.json");
    let exported = sets(
        &mut authoring,
        json!({"action": "export", "scope": {"shared": "other"}, "file": &file}),
    );
    assert!(!is_error(&exported), "{}", message(&exported));

    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "main"}));
    name_author(&mut server, "main", FIXTURE_AUTHOR);
    let ours = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": "alpha, what this set is for, pointing at the set it will absorb",
            "why": "so absorbing supersedes this charter to make its link internal",
            "confidence": 0.9,
            "origin": "human",
            "keywords": ["hoard-scope-topics"],
            "links": [{"label": "related_to", "target": theirs, "target_set": "other"}],
        }),
    );
    assert!(!is_error(&ours), "{}", message(&ours));
    let ours = payload(&ours)["id"].as_str().expect("an id").to_owned();

    let safety = tempdir();
    let absorbed = absorb_into_main(&mut server, &file, &safety.path().join("before.json"));
    assert!(
        !is_error(&absorbed),
        "an absorb must not stop on a charter it carries forward: {}",
        message(&absorbed)
    );
    let original = look_up_in(&mut server, &ours);
    assert!(
        original["superseded_by"].is_array(),
        "the charter linking into the absorbed set is superseded, as any such memory is: {original}"
    );
}

/// **The id check runs before anything is written**, safety copy included. Two distinct memories
/// under one id is the single thing set-scoped ids make possible and nothing else prevents, so it
/// is refused rather than reported afterwards.
#[test]
fn absorb_refuses_an_id_both_sides_hold_before_writing_anything() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);
    let home = tempdir();
    let (mut server, main_id) = a_main_linking_into_other(&home, &other_id);

    // Rewrite the arriving record's id to one `main` already holds. A hand-edited file is exactly
    // where this arrives from: merging two sets that each held a copy of one memory.
    let body = std::fs::read_to_string(&file).expect("the exported file");
    std::fs::write(&file, body.replace(&other_id, &main_id)).expect("rewrites the file");

    let safety = tempdir();
    let copy = safety.path().join("before.json");
    let refused = absorb_into_main(&mut server, &file, &copy);
    assert!(
        is_error(&refused),
        "a colliding id must be refused: {refused}"
    );
    let complaint = message(&refused);
    assert!(
        complaint.contains(&main_id),
        "the refusal should name the id that collided: {complaint}"
    );
    assert!(
        !copy.exists(),
        "nothing may be written before the check, the safety copy included"
    );

    // And the store is untouched: the memory `main` held still says what it said.
    let original = look_up_in(&mut server, &main_id);
    assert_eq!(
        original["links"][0]["target"]["set"],
        json!("other"),
        "a refused absorption must leave the set alone: {original}"
    );
    assert!(
        original["superseded_by"].is_null(),
        "a refused absorption must author nothing: {original}"
    );
}

/// The safety copy is written first and unconditionally, and holds the set as it was *before* the
/// absorption — which is what makes a bulk change that is tedious to unpick reversible at all.
#[test]
fn absorb_writes_a_safety_copy_of_the_set_as_it_was() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);
    let home = tempdir();
    let (mut server, main_id) = a_main_linking_into_other(&home, &other_id);

    let safety = tempdir();
    let copy = safety.path().join("before.json");
    let absorbed = absorb_into_main(&mut server, &file, &copy);
    assert!(!is_error(&absorbed), "{}", message(&absorbed));
    assert_eq!(
        payload(&absorbed)["safety_copy"],
        json!(copy.to_string_lossy()),
        "the response should say where the copy went: {}",
        payload(&absorbed)
    );

    let saved = std::fs::read_to_string(&copy).expect("the safety copy");
    assert!(
        saved.contains(&main_id),
        "the copy should hold what the set held: {saved}"
    );
    assert!(
        !saved.contains("beta, authored in the set that gets absorbed"),
        "the copy is of the set *before* the absorption: {saved}"
    );
    // Asserted on the arriving *content*, not its id: `main`'s own memory links to that id, so the
    // copy contains it and always would. The first version of this test asserted on the id and
    // passed for the wrong reason.
    assert_eq!(
        saved.lines().count(),
        4,
        "a header, the author's identity record, the set's charter and the one memory it held: \
         {saved}"
    );
}

/// A file declaring the very set it would be taken into is not an absorption, and reading it as one
/// would make `absorb` a second spelling of `import`. Refused with the two actions that do mean it.
#[test]
fn absorb_refuses_a_file_declaring_the_set_it_would_be_taken_into() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, _) = a_file_from_other(&author_home, &out);
    let home = tempdir();
    let (mut server, _) = a_main_linking_into_other(&home, &other_id);

    let own = out.path().join("main.json");
    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": {"shared": "main"}, "file": &own}),
    );
    assert!(!is_error(&exported), "{}", message(&exported));

    let safety = tempdir();
    let refused = absorb_into_main(&mut server, &own, &safety.path().join("before.json"));
    assert!(is_error(&refused), "absorbing a set into itself: {refused}");
    let complaint = message(&refused);
    assert!(
        complaint.contains("import") && complaint.contains("restore"),
        "the refusal should name what the caller meant instead: {complaint}"
    );
}

/// **`restore-silently-reverts-an-absorption`, closed.** The absorption is an authored memory, so
/// it is in the exported file like any other and survives every path that rebuilds the store from
/// one. Before this, restore reverted every rewritten link and nothing on the surface could put it
/// back.
#[test]
fn an_absorbed_link_survives_a_restore_of_the_absorbing_set() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);
    let home = tempdir();
    let (mut server, main_id) = a_main_linking_into_other(&home, &other_id);

    let safety = tempdir();
    let absorbed = absorb_into_main(&mut server, &file, &safety.path().join("before.json"));
    assert!(!is_error(&absorbed), "{}", message(&absorbed));
    let tip = look_up_in(&mut server, &main_id)["superseded_by"][0]
        .as_str()
        .expect("a tip")
        .to_owned();

    let checkout = tempdir();
    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": {"shared": "main"}, "directory": checkout.path()}),
    );
    assert!(!is_error(&exported), "{}", message(&exported));
    let restored = sets(
        &mut server,
        json!({
            "action": "restore",
            "scope": {"shared": "main"},
            "directory": checkout.path(),
            "save_to": safety.path().join("after.json"),
        }),
    );
    assert!(!is_error(&restored), "{}", message(&restored));

    let after = look_up_in(&mut server, &tip);
    assert!(
        after["links"]
            .as_array()
            .expect("links")
            .iter()
            .any(|link| link["target"] == json!(other_id)),
        "the absorption is authored, so a restore keeps it: {after}"
    );
}

/// Absorbing twice is absorbing once. The second pass finds no cross-set link left to make
/// internal, because the tips already carry same-set ones — idempotent by construction rather than
/// by a record of what has been absorbed.
#[test]
fn absorbing_the_same_file_twice_supersedes_nothing_the_second_time() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);
    let home = tempdir();
    let (mut server, main_id) = a_main_linking_into_other(&home, &other_id);

    let safety = tempdir();
    let first = absorb_into_main(&mut server, &file, &safety.path().join("first.json"));
    assert!(!is_error(&first), "{}", message(&first));
    let second = absorb_into_main(&mut server, &file, &safety.path().join("second.json"));
    assert!(!is_error(&second), "{}", message(&second));

    assert_eq!(
        payload(&second)["superseded"],
        json!([]),
        "the second pass has nothing left to make internal: {}",
        payload(&second)
    );
    let tips = look_up_in(&mut server, &main_id)["superseded_by"]
        .as_array()
        .expect("a tip")
        .len();
    assert_eq!(tips, 1, "one supersession after two absorptions");
}

/// A set name the file does not declare is not touched. Absorption is about **this** file's set,
/// and a link into a third set stays foreign — the allow-list property the old `absorbing`
/// argument had, kept now that the file itself says which set it is.
#[test]
fn absorb_leaves_a_link_into_a_third_set_alone() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);

    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": "alpha, pointing somewhere absorption does not reach",
            "why": "so the allow-list property has something to fail on",
            "confidence": 0.7,
            "origin": "llm",
            "links": [{"label": "related_to", "target": &other_id, "target_set": "elsewhere"}],
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
    let id = payload(&stored)["id"].as_str().expect("an id").to_owned();

    let safety = tempdir();
    let absorbed = absorb_into_main(&mut server, &file, &safety.path().join("before.json"));
    assert!(!is_error(&absorbed), "{}", message(&absorbed));

    let untouched = look_up_in(&mut server, &id);
    assert!(
        untouched["superseded_by"].is_null(),
        "a link into a set this file does not declare must not be absorbed: {untouched}"
    );
}

/// **A supersession arriving from the absorbed set acts once it is here.** That is the property
/// `a-hand-merge-into-a-new-set-name-leaves-supersessions-inert` was raised about, and it now holds
/// without an argument naming what was merged: a link naming *this* set canonicalises at the door,
/// unconditionally and identically for every holder of the file. Which is exactly why the other
/// direction is the only one that has to be authored — that one changes a memory already in service
/// here, and this one does not.
///
/// The `supersedes` label is edited into the file rather than authored, because a cross-set
/// `supersedes` is refused at the tool boundary — no set outranks another — so this edge can only
/// ever arrive in a file. Which is the hand-merge case it was reported from.
#[test]
fn a_supersession_arriving_from_the_absorbed_set_acts() {
    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    let old = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": "alpha as first claimed, in the set doing the absorbing",
            "why": "so there is something for an arriving memory to replace",
            "confidence": 0.7,
            "origin": "llm",
        }),
    );
    assert!(!is_error(&old), "{}", message(&old));
    let old = payload(&old)["id"].as_str().expect("an id").to_owned();

    let author_home = tempdir();
    let out = tempdir();
    let mut authoring = a_server(&author_home);
    create_chartered(&mut authoring, "other");
    let new = call(
        &mut authoring,
        "hoard_memorize",
        json!({
            "scope": {"shared": "other"},
            "content": "alpha as corrected, from the set being absorbed",
            "why": "so the arriving record has standing to change",
            "confidence": 0.9,
            "origin": "llm",
            "links": [{"label": "related_to", "target": &old, "target_set": "main"}],
        }),
    );
    assert!(!is_error(&new), "{}", message(&new));
    let new = payload(&new)["id"].as_str().expect("an id").to_owned();
    let file = out.path().join("other.json");
    sets(
        &mut authoring,
        json!({"action": "export", "scope": {"shared": "other"}, "file": &file}),
    );
    let body = std::fs::read_to_string(&file).expect("the exported file");
    std::fs::write(
        &file,
        body.replace("\"label\":\"related_to\"", "\"label\":\"supersedes\""),
    )
    .expect("rewrites the file");

    let safety = tempdir();
    let absorbed = absorb_into_main(&mut server, &file, &safety.path().join("before.json"));
    assert!(!is_error(&absorbed), "{}", message(&absorbed));

    let replaced = look_up_in(&mut server, &old);
    assert_eq!(
        replaced["superseded_by"],
        json!([new]),
        "the arriving supersession should act, not sit inert: {replaced}"
    );
}

/// One memory linking into **both** the absorbed set and a third one. It is superseded — it has a
/// link that has become internal — and the supersession must make *that* link internal and leave
/// the other foreign.
///
/// Written because a mutation survived: the guard inside the rewrite is unreachable for a memory
/// linking into only one set, since the collection step has already decided that case, so the two
/// guards look redundant until a memory carries both kinds of link. The mutation was the only thing
/// that said so.
#[test]
fn absorb_makes_only_the_absorbed_sets_link_internal_on_a_memory_with_both() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);

    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": "alpha, pointing into two sets at once",
            "why": "so the two guards are told apart",
            "confidence": 0.7,
            "origin": "llm",
            "links": [
                {"label": "related_to", "target": &other_id, "target_set": "other"},
                {"label": "refers_to", "target": "01M0K23W24FA1B2WVCNTZEC4M3", "target_set": "elsewhere"},
            ],
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
    let id = payload(&stored)["id"].as_str().expect("an id").to_owned();

    let safety = tempdir();
    let absorbed = absorb_into_main(&mut server, &file, &safety.path().join("before.json"));
    assert!(!is_error(&absorbed), "{}", message(&absorbed));

    let tip = look_up_in(&mut server, &id)["superseded_by"][0]
        .as_str()
        .expect("a tip")
        .to_owned();
    let tip = look_up_in(&mut server, &tip);
    let links = tip["links"].as_array().expect("links");
    assert!(
        links.iter().any(|link| link["target"] == json!(other_id)),
        "the absorbed set's link should be internal now: {tip}"
    );
    assert!(
        links
            .iter()
            .any(|link| link["target"]["set"] == json!("elsewhere")),
        "a link into a set this file does not declare must stay foreign: {tip}"
    );
}

/// An id `other`'s file does not carry: what a link looks like from here when the file holds only
/// part of the set.
const NOT_IN_THE_FILE: &str = "01M0K23W24FA1B2WVCNTZEC4M3";

/// **A link is made internal only if its target arrived.**
/// `absorb-internalises-links-to-memories-that-did-not-arrive`: for a whole set every target
/// arrives, but `absorb` reads a file carrying part of one without complaint, and rewriting a
/// working cross-set link to a same-set id this set lacks exchanges it for a dangling one -- under a
/// supersession that makes the broken version current. The link that did not arrive keeps its
/// cross-set form, and the result says so.
#[test]
fn absorb_keeps_a_link_whose_target_did_not_arrive_cross_set() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);

    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": "alpha, pointing at one memory that arrives and one that does not",
            "why": "so a partial file is told apart from a whole one",
            "confidence": 0.7,
            "origin": "llm",
            "links": [
                {"label": "related_to", "target": &other_id, "target_set": "other"},
                {"label": "refers_to", "target": NOT_IN_THE_FILE, "target_set": "other"},
            ],
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
    let id = payload(&stored)["id"].as_str().expect("an id").to_owned();

    let safety = tempdir();
    let absorbed = absorb_into_main(&mut server, &file, &safety.path().join("before.json"));
    assert!(!is_error(&absorbed), "{}", message(&absorbed));

    let tip = look_up_in(&mut server, &id)["superseded_by"][0]
        .as_str()
        .expect("the link that arrived is still made internal, by a supersession")
        .to_owned();
    let tip = look_up_in(&mut server, &tip);
    let links = tip["links"].as_array().expect("links");
    assert!(
        links.iter().any(|link| link["target"] == json!(other_id)),
        "the link whose target arrived should be internal now: {tip}"
    );
    assert!(
        links
            .iter()
            .any(|link| link["target"]["id"] == json!(NOT_IN_THE_FILE)
                && link["target"]["set"] == json!("other")),
        "the link whose target did not arrive must keep its cross-set form: {tip}"
    );
    assert_eq!(
        payload(&absorbed)["not_internalised"],
        json!([{"memory": id, "target": NOT_IN_THE_FILE}]),
        "the result must say which links were left cross-set: {absorbed}"
    );
}

/// **A move done in steps.** A target that arrived in an *earlier* absorb is held here already, and
/// it is the memory the link means: ids are generated to be universally unique, so one id in two
/// sets is one memory copied, and absorb refuses an arriving copy that differs from the one held.
/// Asking only whether the target is in *this* file left such a link cross-set, to dangle once the
/// absorbed set was dropped. Decided with the human partner 2026-09-28, from their scenario: C
/// absorbed first, B later, with a link to C written in between.
#[test]
fn absorb_internalises_a_link_whose_target_arrived_in_an_earlier_absorb() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);

    // The later file: the same set, without the memory the link points at.
    let whole = std::fs::read_to_string(&file).expect("reads the export");
    let marker = format!("\"id\":\"{other_id}\"");
    assert_eq!(
        whole.matches(&marker).count(),
        1,
        "the fixture carries it once"
    );
    let partial: String = whole
        .lines()
        .filter(|line| !line.contains(&marker))
        .map(|line| format!("{line}\n"))
        .collect();
    let later = out.path().join("other-without-it.json");
    std::fs::write(&later, partial).expect("writes the partial file");

    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    let safety = tempdir();
    let first = absorb_into_main(&mut server, &file, &safety.path().join("first.json"));
    assert!(!is_error(&first), "{}", message(&first));

    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": "alpha, written between the two absorbs",
            "why": "so its target arrived in the earlier file, not the later one",
            "confidence": 0.7,
            "origin": "llm",
            "links": [{"label": "related_to", "target": &other_id, "target_set": "other"}],
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
    let id = payload(&stored)["id"].as_str().expect("an id").to_owned();

    let second = absorb_into_main(&mut server, &later, &safety.path().join("second.json"));
    assert!(!is_error(&second), "{}", message(&second));

    assert!(
        payload(&second).get("not_internalised").is_none(),
        "the target is held here, so nothing was left cross-set: {second}"
    );
    let tip = look_up_in(&mut server, &id)["superseded_by"][0]
        .as_str()
        .expect("the link is made internal, by a supersession")
        .to_owned();
    let tip = look_up_in(&mut server, &tip);
    assert!(
        tip["links"]
            .as_array()
            .expect("links")
            .iter()
            .any(|link| link["target"] == json!(other_id)),
        "the link should be internal to the memory the earlier absorb brought: {tip}"
    );
}

/// **But not to a memory this set has since forgotten.** Its tombstone keeps the id, and making a
/// link internal to it would point at nothing just as surely as an id never held.
#[test]
fn absorb_does_not_internalise_a_link_to_a_memory_this_set_forgot() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);
    let whole = std::fs::read_to_string(&file).expect("reads the export");
    let marker = format!("\"id\":\"{other_id}\"");
    let partial: String = whole
        .lines()
        .filter(|line| !line.contains(&marker))
        .map(|line| format!("{line}\n"))
        .collect();
    let later = out.path().join("other-without-it.json");
    std::fs::write(&later, partial).expect("writes the partial file");

    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    let safety = tempdir();
    let first = absorb_into_main(&mut server, &file, &safety.path().join("first.json"));
    assert!(!is_error(&first), "{}", message(&first));
    let forgot = call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": {"shared": "main"}, "id": &other_id}),
    );
    assert!(!is_error(&forgot), "{}", message(&forgot));

    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": "alpha, pointing at a memory this set has forgotten",
            "why": "so a tombstone is told apart from a memory",
            "confidence": 0.7,
            "origin": "llm",
            "links": [{"label": "related_to", "target": &other_id, "target_set": "other"}],
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
    let id = payload(&stored)["id"].as_str().expect("an id").to_owned();

    let second = absorb_into_main(&mut server, &later, &safety.path().join("second.json"));
    assert!(!is_error(&second), "{}", message(&second));
    assert_eq!(
        payload(&second)["not_internalised"],
        json!([{"memory": id, "target": other_id}]),
        "a forgotten memory is not one a link can be made internal to: {second}"
    );
}

/// And a memory none of whose links into the absorbed set arrived is not superseded at all: a
/// replacement identical to the original would be a supersession that changes nothing.
#[test]
fn absorb_does_not_supersede_a_memory_whose_linked_targets_did_not_arrive() {
    let author_home = tempdir();
    let out = tempdir();
    let (_, file) = a_file_from_other(&author_home, &out);
    let home = tempdir();
    let (mut server, main_id) = a_main_linking_into_other(&home, NOT_IN_THE_FILE);

    let safety = tempdir();
    let absorbed = absorb_into_main(&mut server, &file, &safety.path().join("before.json"));
    assert!(!is_error(&absorbed), "{}", message(&absorbed));

    let original = look_up_in(&mut server, &main_id);
    assert!(
        original.get("superseded_by").is_none(),
        "nothing arrived for this memory to link to internally, so nothing replaces it: {original}"
    );
    assert_eq!(payload(&absorbed)["superseded"], json!([]));
    assert_eq!(
        payload(&absorbed)["not_internalised"],
        json!([{"memory": main_id, "target": NOT_IN_THE_FILE}]),
    );
}

/// **`import` no longer takes `absorbing`.** A published-schema removal, and the test is against the
/// schema rather than the behaviour because a silently-ignored argument is exactly what this must
/// not become.
#[test]
fn import_no_longer_publishes_an_absorbing_argument() {
    let home = tempdir();
    let mut server = a_server(&home);
    let listed = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}),
    );
    let schema = listed["result"]["tools"]
        .as_array()
        .expect("an array")
        .iter()
        .find(|tool| tool["name"] == json!("hoard_sets"))
        .map(|tool| tool["inputSchema"].clone())
        .expect("hoard_sets is published");
    assert!(
        schema["properties"]["absorbing"].is_null(),
        "`absorbing` is gone from the surface: {schema}"
    );
    assert!(
        !schema["properties"]["action"]["description"]
            .as_str()
            .unwrap_or_default()
            .contains("absorbing"),
        "the action description still mentions `absorbing`: {schema}"
    );
}

// ---------------------------------------------------------------------------
// The charter comes first: `set-settled-at-first-write`, step 2
// ---------------------------------------------------------------------------

/// What `status` says about one shared set.
fn status_of(server: &mut Server, set: &str) -> Value {
    payload(&sets(server, json!({"action": "status"})))["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .find(|entry| entry["scope"] == json!(format!("shared:{set}")))
        .unwrap_or_else(|| panic!("{set} is not loaded"))
        .clone()
}

/// **A shared set's first memory has to be its charter**, decided 2026-09-24 by the human partner.
/// Refused at the first write rather than at `create`, because `create` cannot tell a new set from one
/// about to be taken on by `import`, and the caller writing to a new set holds the remedy. The
/// refusal writes nothing -- not even the author record a write mints first -- and names the keyword.
#[test]
fn a_shared_sets_first_memory_must_be_its_charter() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "fresh"}));

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "fresh"}, "content": "alpha before any charter",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );

    assert!(is_error(&refused), "{refused}");
    assert!(
        message(&refused).contains("hoard-scope-topics"),
        "the refusal names the mark a charter carries: {}",
        message(&refused)
    );
    let fresh = status_of(&mut server, "fresh");
    assert_eq!(fresh["memories"], json!(0), "{fresh}");
    // No identity means nothing was written at all: the author record a write mints first would
    // have been the set's first write, and given it one.
    assert!(fresh["set_id"].is_null(), "and no identity: {fresh}");
}

/// The charter itself is the way through, and after it the set takes anything.
#[test]
fn a_shared_set_takes_its_charter_first_and_then_anything() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "fresh"}));
    name_author(&mut server, "fresh", FIXTURE_AUTHOR);

    let charter = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "fresh"}, "content": "This set holds notes about alpha.",
               "why": "a fixture", "confidence": 1.0, "origin": "human",
               "keywords": ["hoard-scope-topics", "hoard-lang-en"]}),
    );
    assert!(!is_error(&charter), "{}", message(&charter));
    let after = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "fresh"}, "content": "alpha after the charter",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );
    assert!(!is_error(&after), "{}", message(&after));
}

/// **Only the first write.** A set that already holds memories but no charter -- taken on from a file
/// whose author never wrote one -- keeps taking writes: there the remedy belongs to the set's author,
/// not to the caller, and `status` reports the missing charter rather than refusing.
#[test]
fn a_set_that_already_holds_memories_is_not_refused_for_lacking_a_charter() {
    let author_home = tempdir();
    let repository = tempdir();
    {
        let mut authoring = a_server(&author_home);
        let mut store = Store::open_in_memory(Scope::Shared(
            scatterhoard::memory::SetName::new("main").expect("valid set name"),
        ))
        .expect("opens");
        store
            .insert(&scatterhoard::memory::Memory::new(
                scatterhoard::memory::MemoryText::new("alpha, from a set with no charter")
                    .expect("valid"),
                scatterhoard::memory::Confidence::new(0.7).expect("valid"),
                scatterhoard::memory::Origin::Llm,
                scatterhoard::memory::Attribution {
                    by: scatterhoard::memory::MemoryId::generate(),
                    at: jiff::Timestamp::now(),
                },
            ))
            .expect("the store itself does not ask for a charter");
        authoring.add_store(store);
        sets(
            &mut authoring,
            json!({"action": "export", "scope": {"shared": "main"}, "directory": repository.path()}),
        );
    }
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "main"}));
    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": repository.path()}),
    );
    assert!(!is_error(&imported), "{}", message(&imported));

    name_author(&mut server, "main", FIXTURE_AUTHOR);
    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "main"}, "content": "beta, written after the import",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
}

/// **A set that has forgotten its charter has still been written to**, so it is not refused: the rule
/// is about a set's first write, and forgetting is a write. What it lacks is reported by `status`.
#[test]
fn a_set_whose_charter_was_forgotten_is_not_refused() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "fresh"}));
    name_author(&mut server, "fresh", FIXTURE_AUTHOR);
    let charter = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "fresh"}, "content": FIXTURE_CHARTER,
               "why": "a fixture", "confidence": 1.0, "origin": "human",
               "keywords": ["hoard-scope-topics", "hoard-lang-en"]}),
    );
    let id = payload(&charter)["id"].as_str().expect("an id").to_owned();
    let forgotten = call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": {"shared": "fresh"}, "id": id}),
    );
    assert!(!is_error(&forgotten), "{}", message(&forgotten));

    let after = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "fresh"}, "content": "alpha after the charter was forgotten",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );
    assert!(!is_error(&after), "{}", message(&after));
}

/// **An empty read-only set is told it is read-only**, not to write a charter it could never write:
/// the read-only refusal is asked first.
#[test]
fn an_empty_read_only_set_is_refused_as_read_only_not_for_its_charter() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(
        &mut server,
        json!({"action": "create", "name": "reference", "mode": "read-only"}),
    );

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "reference"}, "content": "alpha into a read-only set",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );

    assert!(is_error(&refused), "{refused}");
    assert!(
        message(&refused).contains("read-only") && !message(&refused).contains("charter"),
        "{}",
        message(&refused)
    );
}

/// `create` says what the first memory has to be, at the moment the set is made -- the promise
/// `route-where` makes when it says `hoard_sets` explains how to write a charter.
#[test]
fn create_says_the_first_memory_is_the_charter() {
    let home = tempdir();
    let mut server = a_server(&home);
    let created = payload(&sets(
        &mut server,
        json!({"action": "create", "name": "fresh"}),
    ));
    assert!(
        created["note"]
            .as_str()
            .is_some_and(|note| note.contains("hoard-scope-topics") && note.contains("\"mint\"")),
        "{created}"
    );
}

/// **A read-only set is told to import its publisher's file**, and nothing about `mint` or a charter:
/// it is never written from here, so both steps the writable note asks for would only be refused.
/// `a-read-only-set-is-told-to-mint`.
#[test]
fn create_tells_a_read_only_set_to_import_rather_than_mint_or_write_a_charter() {
    let home = tempdir();
    let mut server = a_server(&home);
    let created = payload(&sets(
        &mut server,
        json!({"action": "create", "name": "reference", "mode": "read-only"}),
    ));
    let note = created["note"].as_str().expect("a note");

    assert!(note.contains("\"import\""), "{note}");
    assert!(
        !note.contains("mint") && !note.contains("charter") && !note.contains("hoard-scope-topics"),
        "{note}"
    );
}

// ---------------------------------------------------------------------------
// Who writes: `set-settled-at-first-write`, step 3
// ---------------------------------------------------------------------------

/// A server with no account name at all -- what the startup wiring builds where the operating
/// system supplies none.
fn a_server_without_an_account_name(home: &TempDir) -> Server {
    let mut server = Server::new();
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    server
}

/// **A shared set's first write is refused until someone says who they are in it**, decided
/// 2026-09-24 by the human partner. The name is how you are known where the set is published, which
/// varies by set, so a machine-wide name is wrong for anyone publishing in more than one place --
/// which is how a corpus came to be attributed to a machine account. The refusal names both ways
/// through and writes nothing.
#[test]
fn a_shared_sets_first_write_is_refused_until_an_author_is_named() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "fresh"}));

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "fresh"}, "content": FIXTURE_CHARTER,
               "why": "a fixture", "confidence": 1.0, "origin": "human",
               "keywords": ["hoard-scope-topics", "hoard-lang-en"]}),
    );

    assert!(is_error(&refused), "{refused}");
    for way in ["\"mint\"", "\"identify\""] {
        assert!(
            message(&refused).contains(way),
            "the refusal names {way}: {}",
            message(&refused)
        );
    }
    let fresh = status_of(&mut server, "fresh");
    assert_eq!(fresh["memories"], json!(0), "{fresh}");
    assert!(fresh["set_id"].is_null(), "nothing was written: {fresh}");
    assert!(
        fresh["author_note"]
            .as_str()
            .is_some_and(|note| note.contains("refused") && note.contains("\"mint\"")),
        "status says the same before anyone tries: {fresh}"
    );

    let minted = sets(
        &mut server,
        json!({"action": "mint", "scope": {"shared": "fresh"}, "author": "published-name"}),
    );
    assert!(!is_error(&minted), "{}", message(&minted));
    let charter = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "fresh"}, "content": FIXTURE_CHARTER,
               "why": "a fixture", "confidence": 1.0, "origin": "human",
               "keywords": ["hoard-scope-topics", "hoard-lang-en"]}),
    );
    assert!(!is_error(&charter), "{}", message(&charter));
    assert_eq!(
        status_of(&mut server, "fresh")["author"],
        json!("published-name")
    );
}

/// **Where a shared set's name is asked for, the session is told where a candidate is**:
/// `author-default-from-remote-url`. The owner part of a repository's remote URL is a handle in the
/// namespace the set is published into, which is what the name wants -- but the server cannot see
/// which repository a session is in, and a new set has rarely been exported by its first write, so
/// the session looks it up and the human confirms it. Said in terms of hosting rather than of `git`,
/// so a session in another version control system still knows what to look for.
#[test]
fn a_shared_sets_refusal_says_where_a_candidate_name_is() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "fresh"}));

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "fresh"}, "content": FIXTURE_CHARTER,
               "why": "a fixture", "confidence": 1.0, "origin": "human",
               "keywords": ["hoard-scope-topics", "hoard-lang-en"]}),
    );

    let text = message(&refused);
    assert!(text.contains("remote URL"), "{text}");
    assert!(
        text.contains("only they can confirm it"),
        "offered, never written: {text}"
    );
    assert!(!text.contains("git"), "not tied to one VCS: {text}");
}

/// **The refusal stops the session before it says how to proceed**, because the order is what
/// worked: `documents/design/asking-the-human.md`. A refusal that opens with how to call `mint` reads
/// as a recipe with one blank, and a weak reader fills the blank itself -- an invented name every
/// time on `gemma4:12b`, where stopping first got six runs in ten to ask.
#[test]
fn a_shared_sets_refusal_says_to_ask_before_it_says_how_to_mint() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "fresh"}));

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "fresh"}, "content": FIXTURE_CHARTER,
               "why": "a fixture", "confidence": 1.0, "origin": "human",
               "keywords": ["hoard-scope-topics", "hoard-lang-en"]}),
    );

    let text = message(&refused);
    let ask = text
        .find("Ask them for it now, and do not call `hoard_sets` until they have answered")
        .unwrap_or_else(|| panic!("says to ask and wait: {text}"));
    let mint = text
        .find("action \"mint\"")
        .unwrap_or_else(|| panic!("names mint: {text}"));
    assert!(ask < mint, "asks before it says how to mint: {text}");
    assert!(text.contains("only the human can give you"), "{text}");
}

/// **Where the model supplied the name, the human is told what was recorded**, in a sentence the
/// session relays verbatim and that invites a correction: the fallback for a client that cannot be
/// asked directly, decided 2026-10-11. Relayed with the correction 20 of 20 times on `gemma4:12b`,
/// against none for the note it replaced.
#[test]
fn minting_in_a_shared_set_gives_the_session_a_sentence_to_relay() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "fresh"}));

    let minted = payload(&sets(
        &mut server,
        json!({"action": "mint", "scope": {"shared": "fresh"}, "author": "published-name"}),
    ));

    let note = minted["note"].as_str().expect("a note");
    assert!(
        note.starts_with(
            "End your reply to the human with this sentence: I recorded \"published-name\" as \
             the name you write under in shared:fresh -- if that is not how you are known where \
             this set is published, tell me the right name and I will change it."
        ),
        "{note}"
    );
}

/// **`rename` records a name the same way, so it is relayed the same way**, with the new name.
#[test]
fn renaming_in_a_shared_set_gives_the_session_a_sentence_to_relay() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "fresh"}));
    sets(
        &mut server,
        json!({"action": "mint", "scope": {"shared": "fresh"}, "author": "first-name"}),
    );

    let renamed = payload(&sets(
        &mut server,
        json!({"action": "rename", "scope": {"shared": "fresh"}, "author": "second-name"}),
    ));

    let note = renamed["note"].as_str().expect("a note");
    assert!(
        note.contains("I recorded \"second-name\" as the name you write under in shared:fresh"),
        "{note}"
    );
}

/// **`local` is never published**, so there is no publishing name to get wrong and nothing to relay.
#[test]
fn minting_in_local_has_nothing_to_relay() {
    let home = tempdir();
    let mut server = a_server_without_an_account_name(&home);

    let minted = payload(&sets(
        &mut server,
        json!({"action": "mint", "scope": "local", "author": "chosen-name"}),
    ));

    let note = minted["note"].as_str().expect("a note");
    assert!(!note.contains("End your reply"), "{note}");
}

/// **`local` is never published, so it has no publishing name to point at**, and the hint would send
/// a session looking for a repository that has nothing to do with the set.
#[test]
fn locals_refusal_says_nothing_about_a_remote() {
    let home = tempdir();
    let mut server = a_server_without_an_account_name(&home);

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": "local", "content": "alpha, a personal note",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );

    assert!(is_error(&refused), "{refused}");
    assert!(
        !message(&refused).contains("remote URL"),
        "{}",
        message(&refused)
    );
}

/// **`create` is the other moment the name is asked for**, and the first one a new set meets.
#[test]
fn create_says_where_a_candidate_name_is() {
    let home = tempdir();
    let mut server = a_server(&home);

    let writable = payload(&sets(
        &mut server,
        json!({"action": "create", "name": "fresh"}),
    ));
    let read_only = payload(&sets(
        &mut server,
        json!({"action": "create", "name": "reference", "mode": "read-only"}),
    ));

    assert!(
        writable["note"]
            .as_str()
            .is_some_and(|note| note.contains("remote URL")),
        "{writable}"
    );
    assert!(
        read_only["note"]
            .as_str()
            .is_some_and(|note| !note.contains("remote URL")),
        "a read-only set names nobody: {read_only}"
    );
}

/// **`local` takes the account name**: it is never published, so a default costs nothing there, and
/// refusing its first write would put a step in front of every new user.
#[test]
fn local_takes_its_author_from_the_account_name() {
    let home = tempdir();
    let mut server = a_server_without_an_account_name(&home);
    server.set_account_name(Author::new("account-name").expect("valid author"));

    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": "local", "content": "alpha, a personal note",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );

    assert!(!is_error(&stored), "{}", message(&stored));
    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let local = listed["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .find(|set| set["scope"] == json!("local"))
        .expect("local is loaded")
        .clone();
    assert_eq!(local["author"], json!("account-name"), "{local}");
}

/// **And a shared set does not**, however a server is configured: the account name is `local`'s
/// default and nothing else's.
#[test]
fn a_shared_set_does_not_take_the_account_name() {
    let home = tempdir();
    let mut server = a_server_without_an_account_name(&home);
    server.set_account_name(Author::new("account-name").expect("valid author"));
    sets(&mut server, json!({"action": "create", "name": "fresh"}));

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "fresh"}, "content": FIXTURE_CHARTER,
               "why": "a fixture", "confidence": 1.0, "origin": "human",
               "keywords": ["hoard-scope-topics", "hoard-lang-en"]}),
    );
    assert!(is_error(&refused), "{refused}");
}

/// **Where the system supplies no name, `local`'s first write is refused like any other set's**,
/// rather than recorded under a name nobody chose -- which is what `unknown` was.
#[test]
fn local_with_no_account_name_is_refused_until_one_is_minted() {
    let home = tempdir();
    let mut server = a_server_without_an_account_name(&home);

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": "local", "content": "alpha, a personal note",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );
    assert!(is_error(&refused), "{refused}");
    assert!(
        message(&refused).contains("\"mint\""),
        "{}",
        message(&refused)
    );

    let minted = sets(
        &mut server,
        json!({"action": "mint", "scope": "local", "author": "chosen-name"}),
    );
    assert!(!is_error(&minted), "{}", message(&minted));
    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": "local", "content": "alpha, a personal note",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
}

/// **`local` asks rather than minting beside a record it holds**, like any set: the account name is a
/// default for a set nobody has claimed, not a licence to mint a second identity for one person. The
/// state is `local` restored from a backup on a machine that has not said which record is its own.
#[test]
fn local_holding_an_identity_record_asks_rather_than_minting_another() {
    let home = tempdir();
    let path = home.path().join("local.sqlite3");
    {
        let mut local = Store::open_at(Scope::Local, &path).expect("opens");
        let mut record = scatterhoard::memory::Memory::self_attributed(
            scatterhoard::memory::MemoryText::new("jtcours").expect("valid"),
            scatterhoard::memory::Confidence::new(1.0).expect("valid"),
            scatterhoard::memory::Origin::Human,
            jiff::Timestamp::now(),
        );
        record.keywords = vec![scatterhoard::memory::Keyword::new("hoard-author").expect("valid")];
        // Through the file door, which is how a restored record arrives: no cache says it is ours.
        local.accept_from_file(&record).expect("takes the record");
    }
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.add_store(Store::open_at(Scope::Local, &path).expect("opens"));

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": "local", "content": "alpha, after the restore",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );

    assert!(is_error(&refused), "{refused}");
    assert!(
        message(&refused).contains("already holds identity records"),
        "{}",
        message(&refused)
    );
}

/// **A shared set taken on with its contributors' records asks which is you**, not who writes: the
/// set is not unclaimed, and the refusal has to say so, since the answer may be one of the records
/// already there.
#[test]
fn a_shared_set_holding_others_records_asks_which_is_yours() {
    let author_home = tempdir();
    let out = tempdir();
    let (_, file) = a_file_from_other(&author_home, &out);
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "other"}));
    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "other"}, "file": &file}),
    );
    assert!(!is_error(&imported), "{}", message(&imported));

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "other"}, "content": "alpha, written after taking it on",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );

    assert!(is_error(&refused), "{refused}");
    assert!(
        message(&refused).contains("already holds identity records"),
        "{}",
        message(&refused)
    );
}

/// **`local`'s first write is remembered too**, so losing its cached author -- restoring it from a
/// backup, say -- does not make it ask who you are. That first write mints both the author record and
/// the set's identity, so the durable map has to be keyed on the identity read *after* it.
#[test]
fn locals_first_write_is_remembered_when_its_cache_is_lost() {
    let home = tempdir();
    let path = home.path().join("local.sqlite3");
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.add_store(Store::open_at(Scope::Local, &path).expect("opens"));
    let first = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": "local", "content": "alpha, the first personal note",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );
    assert!(!is_error(&first), "{}", message(&first));

    rusqlite::Connection::open(&path)
        .expect("opens the store")
        .execute("DELETE FROM meta WHERE key = 'author_record'", [])
        .expect("clears the cached identity");

    let second = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": "local", "content": "beta, after the cache went",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );
    assert!(
        !is_error(&second),
        "the map should have answered without asking: {}",
        message(&second)
    );
}

/// **Minting into a new set is remembered, so a re-added copy of it does not ask again.** Minting is
/// now a new shared set's usual first write, and that write is what gives the set its identity -- so
/// `mint` has to key the durable map on the identity it has just created, not on the absent one it
/// could read beforehand.
#[test]
fn minting_into_a_new_set_is_remembered_when_the_set_is_re_added() {
    let home = tempdir();
    let out = tempdir();
    let safety = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "revenant");
    sets(
        &mut server,
        json!({"action": "export", "scope": {"shared": "revenant"}, "directory": out.path()}),
    );
    sets(
        &mut server,
        json!({"action": "drop", "name": "revenant",
               "save_to": safety.path().join("gone.json").to_string_lossy()}),
    );
    sets(&mut server, json!({"action": "create", "name": "revenant"}));
    sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "revenant"}, "directory": out.path()}),
    );

    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": {"shared": "revenant"}, "content": "alpha, after it came back",
               "why": "a fixture", "confidence": 0.7, "origin": "llm"}),
    );
    assert!(
        !is_error(&stored),
        "a set re-added on the machine that minted in it must not ask again: {}",
        message(&stored)
    );
}

// ---------------------------------------------------------------------------
// Set identity: minted at the first write, carried in the header, adopted from a file
// ---------------------------------------------------------------------------

/// **`create` settles no identity; the set's first write does**, since `set-settled-at-first-write`.
/// `create` cannot tell a new set from one about to be taken on by `import`, so an identity minted there
/// would be displaced by the file's or would fork it. `status` reports the identity once there is one,
/// because a human has to be able to read it.
#[test]
fn a_created_set_has_no_identity_until_it_is_first_written() {
    let home = tempdir();
    let mut server = a_server(&home);

    let created = payload(&sets(
        &mut server,
        json!({"action": "create", "name": "main"}),
    ));
    assert!(
        created.get("set_id").is_none(),
        "create settles no identity, so it reports none: {created}"
    );
    let status_of_main = |server: &mut Server| {
        payload(&sets(server, json!({"action": "status"})))["sets"]
            .as_array()
            .expect("sets")
            .iter()
            .find(|set| set["scope"] == json!("shared:main"))
            .expect("the set is loaded")["set_id"]
            .clone()
    };
    assert!(status_of_main(&mut server).is_null(), "nor does status");

    // The charter, which a shared set's first memory has to be, is the write that settles it.
    write_charter(&mut server, "main");
    let minted = status_of_main(&mut server);
    let minted = minted.as_str().expect("the first write gave it one");
    assert_eq!(minted.len(), 26, "the one id spelling: {minted}");
}

/// The identity travels in the header, which is what makes it reach anyone else at all.
#[test]
fn an_exported_file_carries_the_sets_identity() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    store_in_main(&mut server, "alpha in a set with an identity");
    let minted = identity_of(&mut server, "main");
    sets(
        &mut server,
        json!({"action": "export", "scope": {"shared": "main"}, "directory": out.path()}),
    );

    let text = std::fs::read_to_string(memories_file(&out)).expect("the exported file");
    let header: Value = serde_json::from_str(text.lines().next().expect("a header")).expect("json");
    assert_eq!(header["set_id"], json!(minted), "header: {header}");
}

/// **Adoption is what keeps the hard cut from being fragile.** Someone who has already built a store
/// from a file written before identities existed gets ours on their next import, rather than an
/// invented rival — which is the failure both rejected migration options produce.
#[test]
fn a_set_without_an_identity_adopts_the_one_a_file_offers() {
    let author_home = tempdir();
    let repository = tempdir();
    let minted = {
        let mut authoring = a_server(&author_home);
        create_chartered(&mut authoring, "main");
        store_in_main(
            &mut authoring,
            "alpha authored where the identity was minted",
        );
        sets(
            &mut authoring,
            json!({"action": "export", "scope": {"shared": "main"}, "directory": repository.path()}),
        );
        identity_of(&mut authoring, "main")
    };

    // **`create` then `import`, the sanctioned way to take on a set.** `create` settles no identity,
    // so the import fills an absence rather than displacing one minted seconds earlier
    // (`a-re-added-set-cannot-recover-its-identity`, then `set-settled-at-first-write`).
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "main"}));

    let imported = payload(&sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": repository.path()}),
    ));
    assert_eq!(
        imported["adopted_set_id"],
        json!(minted),
        "an import that hands a set its identity must say so: {imported}"
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let held = listed["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .find(|set| set["scope"] == json!("shared:main"))
        .and_then(|set| set["set_id"].as_str())
        .expect("an identity");
    assert_eq!(held, minted, "the set should now hold the file's identity");
}

/// **The different-set half of `a-target-path-can-come-to-mean-a-different-volume`, as the human
/// partner first told it**: export to a drive, swap in another drive that mounts at the same place
/// and carries a different set of the same name, and import. The directory is a target this set has
/// agreed with, so its base would be offered as the ancestor for the stranger's records -- and the
/// refusal has to come before that, whatever the base says. Checked 2026-09-28: refused since
/// `identity-mismatch-refusal`, and this pins the order against a base that exists.
#[test]
fn a_different_set_swapped_in_at_a_directory_this_set_agreed_with_is_refused() {
    let home = tempdir();
    let drive = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    store_in_main(&mut server, "alpha, exported to the first drive");
    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": {"shared": "main"}, "directory": drive.path()}),
    );
    assert!(!is_error(&exported), "{}", message(&exported));

    // Another person's set that happens to share the name, on a second drive.
    let stranger_home = tempdir();
    let other_drive = tempdir();
    let stranger_memory = {
        let mut stranger = a_server(&stranger_home);
        create_chartered(&mut stranger, "main");
        let stored = call(
            &mut stranger,
            "hoard_memorize",
            json!({
                "scope": {"shared": "main"},
                "content": "beta, from an unrelated set called main",
                "why": "so the stranger's set has a record that must not arrive",
                "confidence": 0.7,
                "origin": "llm"
            }),
        );
        assert!(!is_error(&stored), "{}", message(&stored));
        let id = payload(&stored)["id"].as_str().expect("an id").to_owned();
        let written = sets(
            &mut stranger,
            json!({"action": "export", "scope": {"shared": "main"}, "directory": other_drive.path()}),
        );
        assert!(!is_error(&written), "{}", message(&written));
        id
    };

    // The swap: the same mount point now holds the other drive's contents.
    let mount = drive.path().join("scatterhoard_mem");
    std::fs::remove_dir_all(&mount).expect("ejects the first drive");
    std::fs::create_dir_all(&mount).expect("mounts the second");
    for entry in std::fs::read_dir(other_drive.path().join("scatterhoard_mem")).expect("reads") {
        let entry = entry.expect("an entry");
        std::fs::copy(entry.path(), mount.join(entry.file_name())).expect("copies");
    }

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": drive.path()}),
    );
    assert!(
        is_error(&imported),
        "a different set at a familiar path must be refused, not reconciled against its base: \
         {imported}"
    );
    assert!(
        message(&imported).contains("declares the set identity"),
        "and refused as the identity mismatch it is: {}",
        message(&imported)
    );
    let looked = call(&mut server, "hoard_lookup", json!({"id": stranger_memory}));
    assert!(
        payload(&looked)["memories"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "nothing from the stranger's set may have arrived: {looked}"
    );
}

/// **A set that has written under an identity keeps it, whatever a file says.** Overwriting a
/// mismatch is what would let two servers restamp forever, where filling a gap converges because
/// every reader agrees on what it finds. The import is refused, since `identity-mismatch-refusal`.
#[test]
fn a_set_that_has_been_written_to_keeps_its_identity_when_a_file_offers_another() {
    let author_home = tempdir();
    let repository = tempdir();
    {
        let mut authoring = a_server(&author_home);
        create_chartered(&mut authoring, "main");
        store_in_main(&mut authoring, "alpha from a set with its own identity");
        sets(
            &mut authoring,
            json!({"action": "export", "scope": {"shared": "main"}, "directory": repository.path()}),
        );
    }

    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    store_in_main(
        &mut server,
        "beta, written here under this set's own identity",
    );
    let mine = identity_of(&mut server, "main");

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": repository.path()}),
    );
    assert!(
        is_error(&imported),
        "a mismatch is refused since `identity-mismatch-refusal`: {imported}"
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let held = listed["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .find(|set| set["scope"] == json!("shared:main"))
        .and_then(|set| set["set_id"].as_str())
        .expect("an identity");
    assert_eq!(
        held, mine,
        "an identity the set has written under must not be displaced"
    );
}

/// **A file with no identity is checked by name, exactly as it always was.** The legacy path, and
/// the reason the cut needs no flag day: such a file neither proves nor disproves identity, so it
/// degrades to the previous behaviour rather than to a refusal.
#[test]
fn a_file_carrying_no_identity_imports_as_it_always_did() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    store_in_main(&mut server, "alpha before identities existed");
    sets(
        &mut server,
        json!({"action": "export", "scope": {"shared": "main"}, "directory": out.path()}),
    );

    // Strip the header's identity, which is exactly what every file written before today looks like.
    let path = memories_file(&out);
    let text = std::fs::read_to_string(&path).expect("the exported file");
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut header: Value = serde_json::from_str(&lines[0]).expect("json");
    header.as_object_mut().expect("an object").remove("set_id");
    lines[0] = header.to_string();
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).expect("writes");

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": out.path()}),
    );
    assert!(
        !is_error(&imported),
        "a file from before identities must still import: {}",
        message(&imported)
    );
    assert!(
        payload(&imported)["adopted_set_id"].is_null(),
        "there was nothing to adopt"
    );
}

/// A shared set loaded with no identity -- the state `create` leaves one in, added directly so that
/// the test does not depend on a data directory.
fn add_a_set_with_no_identity(server: &mut Server, name: &str) {
    server.add_store(
        Store::open_in_memory(Scope::Shared(
            scatterhoard::memory::SetName::new(name).expect("valid set name"),
        ))
        .expect("opens"),
    );
}

/// **A shared set with no identity claims one on its first write**, since `set-settled-at-first-write`
/// (decided 2026-09-24). It was refused from 2026-09-14, on the ground that two holders of one set
/// minting would fork it; `create` minted just as early, and a fork is now refused at the next import
/// rather than merged.
#[test]
fn memorizing_into_a_shared_set_with_no_identity_claims_one() {
    let home = tempdir();
    let mut server = a_server(&home);
    add_a_set_with_no_identity(&mut server, "main");
    // Naming who writes is the set's first write now, and what claims its identity; the charter
    // after it has to find the identity already there.
    name_author(&mut server, "main", FIXTURE_AUTHOR);

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"}, "content": FIXTURE_CHARTER,
            "why": "a fixture", "confidence": 0.8, "origin": "llm",
            "keywords": ["hoard-scope-topics", "hoard-lang-en"]
        }),
    );

    assert!(!is_error(&response), "{response}");
    assert_eq!(identity_of(&mut server, "main").len(), 26);
}

/// A file carrying the set's identity gives a set without one that identity on import, before any
/// record is reconciled, and the file's records are then written.
///
/// Checked through `status` rather than by memorizing afterwards: the imported file brings its
/// author's identity record, and a write then waits for this machine to say which record is it --
/// a separate refusal, and not the one under test.
#[test]
fn importing_a_file_with_an_identity_into_a_set_without_one_adopts_it_and_proceeds() {
    let author_home = tempdir();
    let repository = tempdir();
    let minted = {
        let mut authoring = a_server(&author_home);
        create_chartered(&mut authoring, "main");
        store_in_main(
            &mut authoring,
            "alpha, authored where the identity was minted",
        );
        sets(
            &mut authoring,
            json!({"action": "export", "scope": {"shared": "main"}, "directory": repository.path()}),
        );
        identity_of(&mut authoring, "main")
    };

    let home = tempdir();
    let mut server = a_server(&home);
    add_a_set_with_no_identity(&mut server, "main");
    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": repository.path()}),
    );

    assert!(!is_error(&imported), "{}", message(&imported));
    assert_eq!(payload(&imported)["adopted_set_id"], json!(minted));
    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let main = listed["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .find(|set| set["scope"] == json!("shared:main"))
        .expect("the set is loaded")
        .clone();
    assert_eq!(
        main["memories"],
        // The file's memory and its set's charter.
        json!(2),
        "the file's memory was written once the set had an identity: {main}"
    );
}

/// **A file that carries no identity cannot supply one**, so importing its records into a set without
/// one makes the import the set's first write, which claims an identity of its own -- and says so,
/// since every machine importing that file does the same and their copies are different sets.
#[test]
fn importing_a_file_without_an_identity_into_a_set_without_one_claims_one_and_says_so() {
    let author_home = tempdir();
    let repository = tempdir();
    {
        let mut authoring = a_server(&author_home);
        create_chartered(&mut authoring, "main");
        store_in_main(
            &mut authoring,
            "alpha, from a file that will lose its identity",
        );
        sets(
            &mut authoring,
            json!({"action": "export", "scope": {"shared": "main"}, "directory": repository.path()}),
        );
    }
    let path = memories_file(&repository);
    let text = std::fs::read_to_string(&path).expect("the exported file");
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut header: Value = serde_json::from_str(&lines[0]).expect("json");
    header.as_object_mut().expect("an object").remove("set_id");
    lines[0] = header.to_string();
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).expect("writes");

    let home = tempdir();
    let mut server = a_server(&home);
    add_a_set_with_no_identity(&mut server, "main");
    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": repository.path()}),
    );

    assert!(!is_error(&imported), "{}", message(&imported));
    let minted = identity_of(&mut server, "main");
    let note = payload(&imported)["identity_note"]
        .as_str()
        .expect("an import that mints an identity nobody else holds should say so")
        .to_owned();
    assert!(
        note.contains(&minted) && note.contains("had none either"),
        "{note}"
    );
}

/// **The identity is claimed before reconciling, not at the first memory.** A file's tombstones are
/// applied through `accept_tombstone`, which does not mint, so a file of tombstones alone would
/// otherwise leave records in a set with no identity -- the state first-write minting exists to rule
/// out.
#[test]
fn importing_only_tombstones_into_a_set_without_an_identity_claims_one_first() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    add_a_set_with_no_identity(&mut server, "main");

    let path = reserved(&out, "main.json");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("creates");
    let header = json!({
        "format_version": scatterhoard::memory_file::FORMAT_VERSION,
        "scope": {"shared": "main"},
    });
    let tombstone = json!({
        "id": scatterhoard::memory::MemoryId::generate().to_string(),
        "forgotten": {
            "by": scatterhoard::memory::MemoryId::generate().to_string(),
            "at": "2026-09-14T09:00:00Z",
        },
    });
    std::fs::write(&path, format!("{header}\n{tombstone}\n")).expect("writes");

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": out.path()}),
    );

    assert!(!is_error(&imported), "{}", message(&imported));
    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let main = listed["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .find(|set| set["scope"] == json!("shared:main"))
        .expect("the set is still loaded")
        .clone();
    assert_eq!(
        main["forgotten"],
        json!(1),
        "the tombstone was written: {main}"
    );
    assert!(
        main["set_id"].is_string(),
        "under an identity claimed first: {main}"
    );
}

/// Absorbing takes another set's *memories*, never its identity. The absorbing set stays itself —
/// otherwise a merge would silently rename one of its participants.
#[test]
fn absorbing_does_not_take_the_absorbed_sets_identity() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);
    let home = tempdir();
    let (mut server, _) = a_main_linking_into_other(&home, &other_id);

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let before = listed["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .find(|set| set["scope"] == json!("shared:main"))
        .and_then(|set| set["set_id"].as_str())
        .expect("an identity")
        .to_owned();

    let safety = tempdir();
    let absorbed = absorb_into_main(&mut server, &file, &safety.path().join("before.json"));
    assert!(!is_error(&absorbed), "{}", message(&absorbed));

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let after = listed["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .find(|set| set["scope"] == json!("shared:main"))
        .and_then(|set| set["set_id"].as_str())
        .expect("an identity");
    assert_eq!(after, before, "absorbing must not change who the set is");
}

fn store_in_main(server: &mut Server, content: &str) {
    let stored = call(
        server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": content,
            "why": "so there is something to export",
            "confidence": 0.7,
            "origin": "llm"
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
}

/// **A cross-set link records the target set's identity when this machine knows it**, so a link
/// survives that set being renamed and stops being ambiguous between two sets sharing a name.
///
/// The caller does not supply it and has no way to: an LLM writing a link knows an id and a set
/// name, and the identity is a fact the server holds. [Decision Placement](documents/design/decision-placement.md)
/// — the decision goes where the information is.
#[test]
fn a_cross_set_link_records_the_target_sets_identity_when_it_is_known() {
    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "other");
    store_in(&mut server, "other", "gamma, so the set has an identity");
    let other_id = identity_of(&mut server, "other");
    create_chartered(&mut server, "main");

    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": "alpha pointing at a set this machine holds",
            "why": "so the identity is available to record",
            "confidence": 0.7,
            "origin": "llm",
            "links": [{
                "label": "related_to",
                "target": "01M0K23W24FA1B2WVCNTZEC4M3",
                "target_set": "other",
            }],
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));

    let found = look_up_in(&mut server, payload(&stored)["id"].as_str().expect("an id"));
    assert_eq!(
        found["links"][0]["target"]["set_id"],
        json!(other_id),
        "the link should carry the target set's identity: {found}"
    );
}

/// **And carries none when it cannot know it**, which is permanent rather than transitional: a link
/// may name a set this machine has never held — the tool documents a dangling target as allowed —
/// so there is often no identity to write, and the name has to keep working on its own.
#[test]
fn a_cross_set_link_into_an_unheld_set_carries_no_identity() {
    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "main");
    // Written to first, so a loaded set holds an identity that could be invented from: a set takes
    // one only at its first write, and with none held anywhere this test could not fail.
    store_in_main(&mut server, "beta, so this set has an identity");

    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "main"},
            "content": "alpha pointing somewhere this machine has never held",
            "why": "so the absent case is exercised",
            "confidence": 0.7,
            "origin": "llm",
            "links": [{
                "label": "related_to",
                "target": "01M0K23W24FA1B2WVCNTZEC4M3",
                "target_set": "stranger",
            }],
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));

    let found = look_up_in(&mut server, payload(&stored)["id"].as_str().expect("an id"));
    assert_eq!(
        found["links"][0]["target"]["set"],
        json!("stranger"),
        "the name still resolves it as far as it ever did: {found}"
    );
    assert!(
        found["links"][0]["target"]["set_id"].is_null(),
        "no identity may be invented for a set this machine does not hold: {found}"
    );
}

/// **An arriving record is not enriched on the way in.** Filling in an identity the author did not
/// write is a receiver-side rewrite, which is the exact act `absorb-as-supersession` removed — it
/// would put one id under two contents again, this time by helpfulness.
#[test]
fn importing_does_not_add_an_identity_the_author_did_not_write() {
    let author_home = tempdir();
    let repository = tempdir();
    let id = {
        let mut authoring = a_server(&author_home);
        create_chartered(&mut authoring, "main");
        let stored = call(
            &mut authoring,
            "hoard_memorize",
            json!({
                "scope": {"shared": "main"},
                "content": "alpha with a link into a set the author did not hold",
                "why": "so the importer has something it could be tempted to enrich",
                "confidence": 0.7,
                "origin": "llm",
                "links": [{
                    "label": "related_to",
                    "target": "01M0K23W24FA1B2WVCNTZEC4M3",
                    "target_set": "other",
                }],
            }),
        );
        assert!(!is_error(&stored), "{}", message(&stored));
        sets(
            &mut authoring,
            json!({"action": "export", "scope": {"shared": "main"}, "directory": repository.path()}),
        );
        payload(&stored)["id"].as_str().expect("an id").to_owned()
    };

    // The importing machine *does* hold `other`, so it could fill the identity in — and must not.
    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "other");
    sets(&mut server, json!({"action": "create", "name": "main"}));
    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": repository.path()}),
    );
    assert!(!is_error(&imported), "{}", message(&imported));

    let found = look_up_in(&mut server, &id);
    assert!(
        found["links"][0]["target"]["set_id"].is_null(),
        "an importer must write the author's bytes, not improve them: {found}"
    );
}

/// A machine that adopts `main` from a checkout: create the set, then import.
fn adopt_main(home: &TempDir, repository: &TempDir) -> Server {
    let mut server = a_server(home);
    sets(&mut server, json!({"action": "create", "name": "main"}));
    let imported = sets(
        &mut server,
        json!({
            "action": "import",
            "scope": {"shared": "main"},
            "directory": repository.path(),
        }),
    );
    assert!(!is_error(&imported), "{}", message(&imported));

    // **Adopting a set is not the same as being one of its contributors.** The import brought the
    // authoring holder's identity record in, so writing here has to say which record is this
    // machine's user rather than guess by name -- a wrong guess forks one person into two
    // identities and nothing downstream can report it. Spelled as the `author` action for now;
    // `identify` is what it becomes.
    let identified = sets(
        &mut server,
        json!({"action": "mint", "scope": {"shared": "main"}, "author": "adopter"}),
    );
    assert!(!is_error(&identified), "{}", message(&identified));
    server
}

fn look_up_in(server: &mut Server, id: &str) -> Value {
    payload(&call(server, "hoard_lookup", json!({"id": id})))["memories"][0].clone()
}

/// Only one side moving is not a conflict — it is a change to take or to keep, depending which side
/// moved. Paired with the test above so the difference between them is what is being asserted.
#[test]
fn a_memory_changed_on_one_side_only_is_not_a_conflict() {
    let home = tempdir();
    let out = tempdir();
    let elsewhere = tempdir();
    let mut server = a_server(&home);

    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha as agreed",
            "why": "the shared starting point",
            "keywords": ["agreed"],
            "confidence": 0.5,
            "origin": "llm"
        }),
    );
    let _ = payload(&stored)["id"].as_str().expect("an id");

    export_to(&mut server, &out);
    export_to(&mut server, &elsewhere);

    // The store moves, and `out` does not. Same route as the test above, one side short.
    edit_in_file(&elsewhere, "\"agreed\"", "\"ours-only\"");
    let ours = payload(&import_from(&mut server, &elsewhere));
    assert_eq!(ours["outcomes"]["took_from_file"], json!(1), "{ours}");

    let report = payload(&import_from(&mut server, &out));
    assert_eq!(
        report["outcomes"]["kept_local"],
        json!(1),
        "a change only this store has should be kept, not conflicted: {report}"
    );
    assert!(report.get("conflicts").is_none(), "{report}");
}

#[test]
fn import_refuses_a_file_that_is_not_there() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    let response = import_from(&mut server, &out);
    assert!(is_error(&response));
    assert!(
        message(&response).contains("local.json"),
        "the refusal should name the file it looked for: {}",
        message(&response)
    );
}

/// A malformed line must leave the store untouched, which is what validating before applying buys.
#[test]
fn a_malformed_file_is_refused_before_anything_is_written() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha survives");

    export_to(&mut server, &out);

    let path = memories_file(&out);
    let mut text = std::fs::read_to_string(&path).expect("reads");
    text.push_str("<<<<<<< HEAD\n");
    std::fs::write(&path, text).expect("writes");

    let response = import_from(&mut server, &out);
    assert!(is_error(&response), "a conflicted file must be refused");

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(
        listed["sets"][0]["memories"],
        json!(1),
        "the store changed despite the file being unusable"
    );
}

/// A file declaring a different scope is refused, because the header may only agree with what the
/// caller already believed.
#[test]
fn import_refuses_a_file_belonging_to_another_set() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is local");
    export_to(&mut server, &out);

    sets(
        &mut server,
        json!({"action": "create", "name": "elsewhere"}),
    );
    let response = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "elsewhere"}, "directory": out.path()}),
    );

    assert!(
        is_error(&response),
        "a local file must not import as shared"
    );
}

/// Adopting a set someone else exported: `create` with the name, then `import`.
///
/// This is the Vision's core scenario — clone a project, import its memories — and it already works
/// without `Expectation::AnyShared`, because `create` *is* the trust-on-first-use binding. The human
/// states the name they believe the set has, and the file's header can only agree. `AnyShared` would
/// save them typing it, not enable anything.
#[test]
fn a_set_exported_elsewhere_can_be_adopted_by_create_then_import() {
    let their_home = tempdir();
    let repository = tempdir();

    // Someone else's machine: a shared set with a memory in it, exported into a checkout.
    {
        let mut theirs = a_server(&their_home);
        create_chartered(&mut theirs, "coding-lessons");
        call(
            &mut theirs,
            "hoard_memorize",
            json!({
                "scope": {"shared": "coding-lessons"},
                "content": "alpha is a lesson worth sharing",
                "why": "it came up on their machine",
                "confidence": 0.8,
                "origin": "llm"
            }),
        );
        sets(
            &mut theirs,
            json!({
                "action": "export",
                "scope": {"shared": "coding-lessons"},
                "directory": repository.path(),
            }),
        );
    }

    // Our machine, which has never heard of the set.
    let our_home = tempdir();
    let mut ours = a_server(&our_home);
    sets(
        &mut ours,
        json!({"action": "create", "name": "coding-lessons"}),
    );

    let report = payload(&sets(
        &mut ours,
        json!({
            "action": "import",
            "scope": {"shared": "coding-lessons"},
            "directory": repository.path(),
        }),
    ));

    assert_eq!(
        report["outcomes"]["new_from_file"],
        // The memory, the author's identity record and the set's charter.
        json!(3),
        "the adopted memory did not arrive: {report}"
    );

    let found = payload(&call(
        &mut ours,
        "hoard_recall",
        json!({"query": "alpha", "scope": {"shared": "coding-lessons"}}),
    ));
    assert_eq!(
        found["memories"][0]["content"],
        json!("alpha is a lesson worth sharing")
    );
}

/// Naming the wrong set when adopting is a refusal, not a silent re-target. That is what makes
/// `create` a safe place for the binding to happen.
#[test]
fn adopting_under_the_wrong_name_is_refused() {
    let their_home = tempdir();
    let repository = tempdir();
    {
        let mut theirs = a_server(&their_home);
        sets(
            &mut theirs,
            json!({"action": "create", "name": "their-name"}),
        );
        sets(
            &mut theirs,
            json!({
                "action": "export",
                "scope": {"shared": "their-name"},
                "directory": repository.path(),
            }),
        );
    }

    let our_home = tempdir();
    let mut ours = a_server(&our_home);
    sets(&mut ours, json!({"action": "create", "name": "our-guess"}));

    let response = sets(
        &mut ours,
        json!({
            "action": "import",
            "scope": {"shared": "our-guess"},
            "directory": repository.path(),
        }),
    );
    assert!(
        is_error(&response),
        "a file declaring another set must not import: {}",
        message(&response)
    );
}

#[test]
fn import_refuses_a_set_that_is_not_loaded() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    let response = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "absent"}, "directory": out.path()}),
    );
    assert!(is_error(&response));
    assert!(message(&response).contains("hoard_sets"));
}

/// Importing twice in a row must be a no-op the second time. If the base were not advanced by the
/// first import, the second would see the same changes again.
#[test]
fn importing_twice_settles() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha as first written");

    export_to(&mut server, &out);
    edit_in_file(&out, "alpha as first written", "alpha as revised");

    let first = payload(&import_from(&mut server, &out));
    assert_eq!(first["outcomes"]["took_from_file"], json!(1));

    let second = payload(&import_from(&mut server, &out));
    assert_eq!(
        second["outcomes"]["unchanged"],
        json!(2),
        "the base did not advance, so the same change was seen twice: {second}"
    );
}

/// Importing twice with unexported local work must keep reporting it as local, not escalate it to a
/// conflict.
///
/// An import agrees about only *some* records, so it has to merge those bases and leave the rest
/// alone. Replacing the whole target's record instead would delete the base of a kept-local memory,
/// and the next import would see no ancestor, two differing sides, and call it a conflict — turning a
/// settled situation into one demanding human attention, for no reason.
///
/// **Written because a mutation survived.** Every other import test either exports in between, which
/// rewrites every base and hides this, or has nothing local to keep.
#[test]
fn importing_twice_does_not_escalate_local_work_to_a_conflict() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha was exported");

    export_to(&mut server, &out);
    store_one(&mut server, "beta was written after the export");

    let first = payload(&import_from(&mut server, &out));
    assert_eq!(first["outcomes"]["kept_local"], json!(1), "{first}");

    let second = payload(&import_from(&mut server, &out));
    assert_eq!(
        second["outcomes"]["kept_local"],
        json!(1),
        "the second import changed its mind about local work: {second}"
    );
    assert!(
        second.get("conflicts").is_none(),
        "local work became a conflict on re-import: {second}"
    );
}

/// Order must not matter, which is the whole point. Exporting after importing, then importing again,
/// settles rather than thrashing.
#[test]
fn export_and_import_can_run_in_either_order() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is first");

    export_to(&mut server, &out);
    store_one(&mut server, "beta is added locally");

    let after_pull = payload(&import_from(&mut server, &out));
    assert_eq!(after_pull["outcomes"]["kept_local"], json!(1));

    export_to(&mut server, &out);
    let settled = payload(&import_from(&mut server, &out));

    assert_eq!(
        settled["outcomes"]["unchanged"],
        json!(3),
        "after exporting, both records should be settled: {settled}"
    );
    assert!(settled.get("conflicts").is_none());
}

// ---------------------------------------------------------------------------
// drop
//
// `set-drop`. The set that can be created and imported but never removed -- demonstrated
// 2026-08-15, when a 56-memory test corpus imported as a set took all three top results on an
// unrelated query and the only way back was deleting a file by hand and restarting the server.
//
// `documents/design/set-lifecycle.md` settles the rule these tests are about: **a destructive
// operation takes a path and writes the set there before destroying it**, unconditionally, reading
// no state out of the database it is about to destroy.
// ---------------------------------------------------------------------------

/// The safety copy is written first, and the database goes.
#[test]
fn drop_writes_the_safety_copy_and_removes_the_database() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    create_chartered(&mut server, "throwaway");
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:throwaway", "content": "alpha was imported by mistake",
            "why": "a fixture", "confidence": 0.8, "origin": "llm"
        }),
    );

    let database = home.path().join("shared-throwaway.sqlite3");
    assert!(database.is_file(), "the fixture needs a real database");

    let copy = out.path().join("throwaway-before-drop.json");
    let response = sets(
        &mut server,
        json!({
            "action": "drop", "name": "throwaway",
            "save_to": copy.to_string_lossy()
        }),
    );
    assert!(!is_error(&response), "{}", message(&response));

    let report = payload(&response);
    assert_eq!(report["dropped"], json!("shared:throwaway"));
    assert_eq!(
        report["safety_copy"].as_str().map(std::path::Path::new),
        Some(copy.as_path()),
        "the report must say where the copy went: {report}"
    );

    // The copy holds the memories, which is what makes the drop recoverable at all.
    let file = std::fs::File::open(&copy).expect("the safety copy exists");
    let saved = memory_file::read(
        &Expectation::Exactly(Scope::Shared(
            SetName::new("throwaway").expect("valid name"),
        )),
        file,
    )
    .expect("reads back");
    // One more than the test's own writes: the set's charter, which its first memory has to be.
    assert_eq!(saved.memories.len(), 3);

    assert!(!database.exists(), "the database should be gone");

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let scopes: Vec<&Value> = listed["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|set| &set["scope"])
        .collect();
    assert!(
        !scopes.contains(&&json!("shared:throwaway")),
        "the dropped set should not still be loaded: {listed}"
    );
}

/// And the name is free again afterwards, which is what makes an accidental import undoable rather
/// than merely survivable.
#[test]
fn a_dropped_sets_name_can_be_created_again() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    sets(
        &mut server,
        json!({"action": "create", "name": "throwaway"}),
    );
    sets(
        &mut server,
        json!({
            "action": "drop", "name": "throwaway",
            "save_to": out.path().join("gone.json").to_string_lossy()
        }),
    );

    let again = sets(
        &mut server,
        json!({"action": "create", "name": "throwaway"}),
    );
    assert!(
        !is_error(&again),
        "creating over a dropped name should work: {}",
        message(&again)
    );
}

/// **`drop` leaves any exported file alone**, which is the other half of what makes it recoverable:
/// the set can be restored from its file afterwards. A drop that took the file with it would be a
/// deletion, not a drop.
#[test]
fn drop_leaves_the_sets_exported_file_alone() {
    let home = tempdir();
    let checkout = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    sets(
        &mut server,
        json!({"action": "create", "name": "published"}),
    );
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:published", "content": "alpha is committed",
            "why": "a fixture", "confidence": 0.8, "origin": "llm"
        }),
    );
    sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:published",
            "directory": checkout.path()
        }),
    );
    let exported = memories_file(&checkout);
    assert!(exported.is_file(), "the fixture needs an exported file");

    sets(
        &mut server,
        json!({
            "action": "drop", "name": "published",
            "save_to": out.path().join("safety.json").to_string_lossy()
        }),
    );

    assert!(
        exported.is_file(),
        "the exported file is what the set is restored from and must survive the drop"
    );
}

/// **The destination cannot be the set's own export path.**
///
/// `documents/design/set-lifecycle.md` states this as a correctness requirement rather than a
/// convenience: a safety copy written over the set's usual export file would destroy the good copy
/// in the act of protecting it. The reserved-directory refusal already covers a checkout, since
/// nothing may name a `file` inside `scatterhoard_mem`; what it cannot cover is a set exported to an
/// explicit file, so the recorded targets are what answers this.
#[test]
fn drop_refuses_a_safety_copy_aimed_at_a_file_this_set_is_exported_to() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    sets(
        &mut server,
        json!({"action": "create", "name": "backed-up"}),
    );
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:backed-up", "content": "alpha lives in a synced folder",
            "why": "a fixture", "confidence": 0.8, "origin": "llm"
        }),
    );

    let backup = out.path().join("backed-up.json");
    sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:backed-up",
            "file": backup.to_string_lossy()
        }),
    );
    let before = std::fs::read_to_string(&backup).expect("the export exists");

    let response = sets(
        &mut server,
        json!({
            "action": "drop", "name": "backed-up",
            "save_to": backup.to_string_lossy()
        }),
    );
    assert!(
        is_error(&response),
        "the safety copy must not be written over the file the set is restored from"
    );

    let complaint = message(&response);
    assert!(
        complaint.contains("backed-up.json"),
        "the refusal should name the path it recognised: {complaint}"
    );

    assert_eq!(
        std::fs::read_to_string(&backup).expect("still there"),
        before,
        "the export must be untouched"
    );
    assert!(
        home.path().join("shared-backed-up.sqlite3").is_file(),
        "and a refused drop must not have dropped anything"
    );
}

/// **The destination cannot be written: refuse.** The store is fine and the memories are
/// recoverable, so this is a real failure of the save rather than a reason to press on.
///
/// **The directory exists and is unwritable**, which is the whole difficulty of testing this. A
/// path under a directory that does not exist is refused *earlier*, by the destination check, so a
/// test written that way passes without the write ever being attempted — proving a different guard
/// while reading as proof of this one. Found by a mutation: turning the write's `?` into an `.ok()`
/// left the suite green.
#[test]
fn drop_refuses_when_the_safety_copy_cannot_be_written() {
    use std::os::unix::fs::PermissionsExt;

    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    sets(&mut server, json!({"action": "create", "name": "keepable"}));

    let sealed = out.path().join("sealed");
    std::fs::create_dir_all(&sealed).expect("creates");
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o555)).expect("seals");

    let response = sets(
        &mut server,
        json!({
            "action": "drop", "name": "keepable",
            "save_to": sealed.join("safety.json").to_string_lossy()
        }),
    );

    // Restored before the assertions, so a failure below does not leave an undeletable scratch
    // directory behind.
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755)).expect("unseals");

    assert!(
        is_error(&response),
        "a save that cannot happen is a refusal, not a drop: {}",
        message(&response)
    );

    assert!(
        home.path().join("shared-keepable.sqlite3").is_file(),
        "the set must survive a refused drop"
    );
    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let scopes: Vec<&Value> = listed["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|set| &set["scope"])
        .collect();
    assert!(
        scopes.contains(&&json!("shared:keepable")),
        "and must still be loaded: {listed}"
    );
}

/// **The store cannot be read: proceed, and say so unmissably.**
///
/// The corruption case, and the one the natural implementation — *if the export failed, stop* — gets
/// exactly backwards. The memories are already beyond saving, so refusing protects nothing and
/// blocks the repair. What the caller must not be able to miss is that no copy was made.
#[test]
fn drop_proceeds_when_the_store_cannot_be_read_and_says_no_copy_was_made() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    sets(&mut server, json!({"action": "create", "name": "broken"}));
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:broken", "content": "alpha is about to become unreadable",
            "why": "a fixture", "confidence": 0.8, "origin": "llm"
        }),
    );

    // Corruption, simulated at the only seam a test can reach it: a second connection to the same
    // file removes the table the set is read out of. A truncated or garbled file is the realistic
    // cause and is not reproducible from a test, but what `drop` sees is the same -- the read fails.
    let database = home.path().join("shared-broken.sqlite3");
    {
        let connection =
            rusqlite::Connection::open(&database).expect("opens the database directly");
        connection
            .execute("DROP TABLE memories", [])
            .expect("removes the table");
    }

    let copy = out.path().join("unwritten.json");
    let response = sets(
        &mut server,
        json!({
            "action": "drop", "name": "broken",
            "save_to": copy.to_string_lossy()
        }),
    );
    assert!(
        !is_error(&response),
        "refusing here protects nothing and blocks the repair: {}",
        message(&response)
    );

    let report = payload(&response);
    assert_eq!(report["dropped"], json!("shared:broken"));
    assert_eq!(
        report["safety_copy"],
        Value::Null,
        "there is no copy, and the field must not name a file that was never written: {report}"
    );
    let warning = report["no_safety_copy"]
        .as_str()
        .unwrap_or_else(|| panic!("the caller must not be able to miss this: {report}"));
    assert!(
        warning.contains("could not be read") || warning.contains("unreadable"),
        "and it must say why no copy exists: {warning}"
    );

    assert!(!database.exists(), "the drop should still have happened");
    assert!(
        !copy.exists(),
        "no half-written copy should be left claiming to be a backup"
    );
}

/// Dropping a set this machine does not hold is a refusal that points somewhere reachable.
#[test]
fn drop_refuses_a_set_that_is_not_loaded() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    let response = sets(
        &mut server,
        json!({
            "action": "drop", "name": "never-existed",
            "save_to": out.path().join("safety.json").to_string_lossy()
        }),
    );
    assert!(is_error(&response));

    let complaint = message(&response);
    assert!(
        complaint.contains("never-existed"),
        "the refusal should name the set asked for: {complaint}"
    );
}

/// **The local set is not reachable by `drop` at all**, and by the same construction that keeps it
/// out of `create`: the action takes a *name*, so `local` is not a value it can be given.
///
/// `local` is the one set guaranteed to exist -- it is the destination `route-where`'s fallback
/// clause points at, and the one set that never reaches a repository, so it is also the one with no
/// copy anywhere by construction. Naming `local` here reaches a *shared* set of that name, which is
/// a different set and legitimately droppable.
#[test]
fn drop_cannot_reach_the_local_set() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "a personal memory that must survive");

    let refused = sets(
        &mut server,
        json!({
            "action": "drop", "scope": "local",
            "save_to": out.path().join("safety.json").to_string_lossy()
        }),
    );
    assert!(
        is_error(&refused),
        "`drop` takes a name; a scope is not the argument it has"
    );

    // And the spelling that *is* accepted names a shared set, which this machine does not hold.
    let by_name = sets(
        &mut server,
        json!({
            "action": "drop", "name": "local",
            "save_to": out.path().join("safety.json").to_string_lossy()
        }),
    );
    assert!(is_error(&by_name), "no shared set called `local` is loaded");

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let local = local_set(&listed);
    assert_eq!(
        local["memories"],
        json!(1),
        "the local set must be untouched by either attempt: {listed}"
    );
}

// ---------------------------------------------------------------------------
// restore
//
// `set-restore`. The design's other destructive operation, and the one reached for at the worst
// moment: `documents/design/set-lifecycle.md` calls it *the only route back from a store that
// cannot be trusted*.
//
// **`restore` is not `import`, and the difference is the whole entry.** Import *reconciles* a file
// against a store that may have diverged from it, keeping local work and reporting genuine
// conflicts. Restore *replaces*: the file becomes the store and local divergence is discarded on
// purpose, because reconciling *against* a store you do not trust means merging against content
// nobody trusts.
// ---------------------------------------------------------------------------

/// The one the backlog entry asks for by name: **divergence is discarded, not merged.**
#[test]
fn restore_discards_local_divergence_rather_than_merging_it() {
    let home = tempdir();
    let out = tempdir();
    let safety = tempdir();
    let mut server = a_server(&home);

    store_one(&mut server, "alpha as the file has it");
    let file = out.path().join("good.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    );

    // The store now diverges from the file in both directions: one memory the file has never
    // carried, and an amendment to one it has.
    store_one(&mut server, "beta was written after the export");

    let response = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": file.to_string_lossy(),
            "save_to": safety.path().join("before-restore.json").to_string_lossy()
        }),
    );
    assert!(!is_error(&response), "{}", message(&response));

    let report = payload(&response);
    assert_eq!(
        report["memories"],
        json!(1),
        "the file's one memory: {report}"
    );

    // **The discriminating assertion.** An import here would have kept `beta`, because a memory the
    // file has never carried is local work awaiting export. Restore must not.
    let found = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "beta", "limit": 10}),
    ));
    let contents: Vec<String> = found["memories"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|memory| memory["content"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(
        !contents.iter().any(|content| content.contains("beta")),
        "local divergence must be discarded rather than merged: {contents:?}"
    );
}

/// **The safety copy runs first and unconditionally**, and it is what makes the discarding
/// survivable: the copy is the ordinary memory-file format, so it can be diffed against the restored
/// set and individual memories cherry-picked back.
#[test]
fn restore_writes_the_safety_copy_before_replacing_anything() {
    let home = tempdir();
    let out = tempdir();
    let safety = tempdir();
    let mut server = a_server(&home);

    store_one(&mut server, "alpha as the file has it");
    let file = out.path().join("good.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    );
    store_one(&mut server, "beta is about to be discarded");

    let copy = safety.path().join("before-restore.json");
    let response = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": file.to_string_lossy(),
            "save_to": copy.to_string_lossy()
        }),
    );
    assert!(!is_error(&response), "{}", message(&response));
    assert_eq!(
        payload(&response)["safety_copy"]
            .as_str()
            .map(std::path::Path::new),
        Some(copy.as_path())
    );

    // The copy holds what the store held *before* -- including the memory the restore discarded,
    // which is the only reason the discard is recoverable.
    let saved = memory_file::read(
        &Expectation::Exactly(Scope::Local),
        std::fs::File::open(&copy).expect("the copy exists"),
    )
    .expect("reads back");
    assert_eq!(saved.memories.len(), 3, "the copy is the pre-restore state");
    assert!(
        saved
            .memories
            .iter()
            .any(|memory| memory.content.as_data().contains("beta")),
        "the discarded memory must be in the copy"
    );
}

/// **The copy must not be written over the file being restored from.**
///
/// `documents/design/set-lifecycle.md` states this as a correctness requirement rather than a
/// convenience: it would destroy the good copy in the act of protecting the bad one. This is the
/// case the rule was written for, and it is reachable only because both paths are the caller's.
#[test]
fn restore_refuses_to_save_over_the_file_it_is_restoring_from() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    store_one(&mut server, "alpha is the good copy");
    let exported = out.path().join("good.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": exported.to_string_lossy()}),
    );

    // **Restored from a copy the set has never been exported to**, which is what makes this test
    // discriminate. Using the export itself would be refused by the *known target* check instead --
    // a different guard, passing under this test's name. Found by a mutation: disabling the
    // identity check below left the suite green.
    let file = out.path().join("handed-to-me.json");
    std::fs::copy(&exported, &file).expect("copies");
    let before = std::fs::read_to_string(&file).expect("the copy exists");
    store_one(&mut server, "beta diverges");

    let response = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": file.to_string_lossy(),
            "save_to": file.to_string_lossy()
        }),
    );
    assert!(
        is_error(&response),
        "saving over the source would destroy the good copy in the act of protecting the bad one"
    );

    // **Asserted on the wording, not merely on the refusal, and that is what makes this test worth
    // keeping.** Since the occupied-destination guard landed, the source file existing is enough to
    // refuse this call — so a test that only checked *that* it was refused would pass with this
    // guard deleted, and a mutation saying so survived on 2026-08-31. The two guards answer
    // different questions: one says a file is in the way, this one says **which** file, and naming
    // it is the difference between a fact and a diagnosis for someone mid-rescue.
    let complaint = message(&response);
    assert!(
        complaint.contains("which is the file being restored from"),
        "the refusal must name what the path collided with, not merely that it did: {complaint}"
    );

    assert_eq!(
        std::fs::read_to_string(&file).expect("still there"),
        before,
        "the file being restored from must be untouched"
    );

    // And nothing was restored, so the store still holds its divergence.
    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(local_set(&listed)["memories"], json!(2));
}

/// The second destination guard: the copy may not land on **any** file this set is exported to.
///
/// Distinct from refusing to save over the *source*, and only one of the two can catch this: here
/// the source and the destination are different files, so the identity check passes and the
/// recorded targets are what answer. Restoring from a copy someone handed you while saving over
/// your own backup is a plausible way to lose the backup.
#[test]
fn restore_refuses_to_save_over_a_file_this_set_is_exported_to() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    store_one(&mut server, "alpha is backed up here");
    let backup = out.path().join("my-backup.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": backup.to_string_lossy()}),
    );
    let before = std::fs::read_to_string(&backup).expect("the backup exists");

    // A different file to restore from, so the same-as-source check cannot be what refuses.
    let handed_over = out.path().join("from-a-colleague.json");
    std::fs::copy(&backup, &handed_over).expect("copies");
    store_one(&mut server, "beta diverges");

    let response = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": handed_over.to_string_lossy(),
            "save_to": backup.to_string_lossy()
        }),
    );
    assert!(
        is_error(&response),
        "the safety copy must not overwrite a good copy of the set"
    );
    assert_eq!(
        std::fs::read_to_string(&backup).expect("still there"),
        before,
        "the backup must be untouched"
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(
        local_set(&listed)["memories"],
        json!(2),
        "and nothing was restored: {listed}"
    );
}

/// A restore that cannot save refuses, exactly as a drop does: the store is fine and its memories
/// are recoverable, so the failure is the save's.
#[test]
fn restore_refuses_when_the_safety_copy_cannot_be_written() {
    use std::os::unix::fs::PermissionsExt;

    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    store_one(&mut server, "alpha is safe where it is");
    let file = out.path().join("good.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    );
    store_one(&mut server, "beta must survive a refused restore");

    let sealed = out.path().join("sealed");
    std::fs::create_dir_all(&sealed).expect("creates");
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o555)).expect("seals");

    let response = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": file.to_string_lossy(),
            "save_to": sealed.join("copy.json").to_string_lossy()
        }),
    );
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755)).expect("unseals");

    assert!(is_error(&response), "{}", message(&response));

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(
        local_set(&listed)["memories"],
        json!(2),
        "a refused restore must leave the store exactly as it was: {listed}"
    );
}

/// **Tombstones survive the round trip**, which is the defect
/// `memory-file-read-silently-discards-tombstones` set a trap for.
///
/// Two halves, and the second is the one that produces an unreadable file. The file's tombstones
/// must arrive, so the restored set still knows what it forgot; and the store's own tombstones must
/// be cleared, or a memory this store had forgotten and the file still carries live ends up holding
/// **both** a memory row and a tombstone row under one id. `stream_records` emits both, adjacent,
/// and `file-format.md` refuses duplicate ids -- so the next export writes a file this project's own
/// reader will not read.
#[test]
fn a_restored_set_holds_the_files_tombstones_and_not_the_stores() {
    let home = tempdir();
    let out = tempdir();
    let safety = tempdir();
    let mut server = a_server(&home);

    store_local(&mut server, "alpha is kept throughout");
    let forgotten_in_file = store_local(&mut server, "beta is forgotten in the file");
    let live_in_file = store_local(&mut server, "gamma is live in the file");

    // The file: beta forgotten, alpha and gamma live.
    call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": forgotten_in_file}),
    );
    let file = out.path().join("with-a-tombstone.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    );

    // The store diverges the other way: gamma is forgotten *here* and is live in the file. This is
    // the collision -- one id with a tombstone in the store and a memory in the file.
    call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": live_in_file}),
    );

    let response = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": file.to_string_lossy(),
            "save_to": safety.path().join("before.json").to_string_lossy()
        }),
    );
    assert!(!is_error(&response), "{}", message(&response));

    let report = payload(&response);
    assert_eq!(report["memories"], json!(2), "alpha and gamma: {report}");
    assert_eq!(report["forgotten"], json!(1), "beta's tombstone: {report}");

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(local_set(&listed)["forgotten"], json!(1));

    // The proof that no id holds both shapes: export the restored set and read it back. A duplicate
    // id is refused by the reader, so this fails if the store's tombstone survived beside the file's
    // memory.
    let after = out.path().join("after.json");
    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": after.to_string_lossy()}),
    );
    assert!(!is_error(&exported), "{}", message(&exported));
    memory_file::read(
        &Expectation::Exactly(Scope::Local),
        std::fs::File::open(&after).expect("the export exists"),
    )
    .expect("a restored set must export to a file its own reader accepts");

    // And gamma is live again, because the file said so.
    let found = payload(&call(
        &mut server,
        "hoard_lookup",
        json!({"id": live_in_file}),
    ));
    assert!(
        found["memories"][0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("gamma"),
        "the file's live memory must replace the store's tombstone: {found}"
    );
}

/// **A restore clears a tombstone the file never mentions**, which is the half of that rule the
/// per-id clearing cannot reach.
///
/// `write_memory` now removes any tombstone for an id it writes, so a memory the store had
/// forgotten and the file carries live is handled record by record. What that cannot touch is a
/// tombstone for an id the file does not mention **at all** — nothing writes that id, so nothing
/// clears it, and only `replace_with`'s wholesale delete does. Found by a mutation that stopped
/// being caught the moment the per-id rule landed: the guard did not become redundant, it became
/// uncovered.
#[test]
fn a_restore_clears_a_tombstone_for_an_id_the_file_never_mentions() {
    let home = tempdir();
    let out = tempdir();
    let safety = tempdir();
    let mut server = a_server(&home);

    store_one(&mut server, "alpha is in the file");
    let file = out.path().join("without-beta.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    );

    // Beta exists only here, and only as a tombstone: the file has never carried it in any form.
    let beta = store_local(
        &mut server,
        "beta was written and forgotten after the export",
    );
    call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": beta}),
    );
    assert_eq!(
        local_set(&payload(&sets(&mut server, json!({"action": "status"}))))["forgotten"],
        json!(1),
        "the fixture needs a tombstone the file does not mention"
    );

    sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": file.to_string_lossy(),
            "save_to": safety.path().join("before.json").to_string_lossy()
        }),
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(
        local_set(&listed)["forgotten"],
        json!(0),
        "restore replaces: a tombstone the file does not carry is local divergence and goes: \
         {listed}"
    );
    assert_eq!(local_set(&listed)["memories"], json!(1));
}

/// **A restored set agrees with the file it came from**, and with nothing else.
///
/// Recording the agreement is what stops the next import re-deciding every record. Clearing the
/// *other* targets' bases is the less obvious half and matters more: a base for a record the
/// restored set no longer holds resolves as `Anomaly` -- "a state no supported operation produces"
/// -- so keeping them would have restore manufacture the very report that means something went
/// wrong outside the tools.
#[test]
fn a_restored_set_agrees_with_its_source_and_with_no_other_target() {
    let home = tempdir();
    let checkout = tempdir();
    let out = tempdir();
    let safety = tempdir();
    let mut server = a_server(&home);

    store_one(&mut server, "alpha is in both places");

    let file = out.path().join("good.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    );

    // **Order matters, and getting it wrong makes this test prove nothing.** The checkout must be
    // exported to *after* `beta` exists, so it records an agreement about a record the restore is
    // about to discard -- that is the base that would resolve as an anomaly if it survived.
    // Exporting to the checkout first leaves it holding only agreements about records that outlive
    // the restore, and the assertion below then passes whether the bases are cleared or not.
    store_one(&mut server, "beta is about to be discarded");
    export_to(&mut server, &checkout);

    sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": file.to_string_lossy(),
            "save_to": safety.path().join("before.json").to_string_lossy()
        }),
    );

    // The source: settled, because restore recorded the agreement it just established.
    let from_source = payload(&sets(
        &mut server,
        json!({"action": "import", "scope": "local", "file": file.to_string_lossy()}),
    ));
    assert_eq!(
        from_source["outcomes"]["unchanged"],
        json!(2),
        "the restored set must agree with the file it came from: {from_source}"
    );

    // The other checkout: reconciled with no ancestor, so identical content is `converged` and the
    // discarded memory is a `keep_local` on the store side -- reported, never an anomaly.
    let from_checkout = payload(&import_from(&mut server, &checkout));
    assert!(
        from_checkout.get("anomalies").is_none(),
        "restore must not manufacture anomalies for another checkout: {from_checkout}"
    );
}

/// **A file that does not parse leaves the set exactly as it was.**
///
/// The guarantee `documents/design/file-format.md` asks for — the whole file is checked before any
/// of it is applied — and here it rests on the *shape* rather than on a validating pass: every
/// record is materialised before the store is touched, so a failure anywhere is a failure before
/// anything. A duplicate id is the case worth using, because it is what a mishandled merge
/// produces, it is detected only once the whole file has been read, and it is the one malformation
/// that looks perfectly well-formed line by line.
///
/// A validating pass was written here first and removed once a mutation proved it subsumed. This
/// test is what the guarantee now stands on, so it is the thing to check before changing `restore`
/// to write as it reads.
#[test]
fn a_file_that_does_not_parse_leaves_the_set_exactly_as_it_was() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    store_one(&mut server, "alpha must survive a bad file");
    store_one(&mut server, "beta must survive it too");

    let file = out.path().join("good.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    );

    // Duplicate the last record, which is what a hand-merge of two sets produces. Every line still
    // parses; only the whole file is wrong.
    let text = std::fs::read_to_string(&file).expect("reads");
    let last = text.lines().last().expect("a record").to_owned();
    let doubled = out.path().join("merged-badly.json");
    std::fs::write(&doubled, format!("{text}{last}\n")).expect("writes");

    let response = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": doubled.to_string_lossy(),
            "save_to": out.path().join("copy.json").to_string_lossy()
        }),
    );
    assert!(is_error(&response), "a duplicate id is not a usable file");

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(
        local_set(&listed)["memories"],
        json!(2),
        "a refused restore must not have emptied the set first: {listed}"
    );
}

/// **The old contents cannot be read: restore anyway, and say unmissably that no copy exists.**
///
/// The same rule as `drop`, and this is the operation it matters most for: a store too damaged to
/// read is what restore *exists* for, so refusing there would withhold the repair from the only
/// situation that needs it.
///
/// **Corrupted at the row rather than the table**, which is deliberate and is the difference from
/// the `drop` test. Removing a table makes the store unreadable *and* unwritable, so the
/// replacement would fail too and this branch would never be reached. A single unparseable value is
/// the realistic damage — a partly-written or hand-edited row — and it leaves the store perfectly
/// writable, which is exactly the state where restoring is both possible and worth doing.
///
/// **Nothing proved this branch until 2026-08-25.** Four mutations aimed at `drop` and `import`
/// were silently retargeted onto `restore`'s identical lines, which sit earlier in the file, and
/// survived there — reporting as defects in guards they had never touched. The anchors are now
/// unambiguous and restore's own copies have their own mutations; this test is what those needed.
#[test]
fn restore_proceeds_when_the_old_contents_cannot_be_read_and_says_no_copy_was_made() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    create_chartered(&mut server, "damaged");
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:damaged", "content": "alpha as the file has it",
            "why": "a fixture", "confidence": 0.8, "origin": "llm"
        }),
    );
    let file = out.path().join("good.json");
    sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:damaged",
            "file": file.to_string_lossy()
        }),
    );

    // One value the reader will not accept, written straight into the database. `Confidence::new`
    // refuses anything outside its range, so reading this row fails while every write against the
    // table still works.
    {
        let connection = rusqlite::Connection::open(home.path().join("shared-damaged.sqlite3"))
            .expect("opens the database directly");
        connection
            .execute("UPDATE memories SET confidence = 42.0", [])
            .expect("corrupts a row");
    }

    let copy = out.path().join("unwritten.json");
    let response = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "shared:damaged",
            "file": file.to_string_lossy(),
            "save_to": copy.to_string_lossy()
        }),
    );
    assert!(
        !is_error(&response),
        "a store too damaged to read is what restore is for: {}",
        message(&response)
    );

    let report = payload(&response);
    assert_eq!(
        report["safety_copy"],
        Value::Null,
        "the field must not name a file that was never written: {report}"
    );
    let warning = report["no_safety_copy"]
        .as_str()
        .unwrap_or_else(|| panic!("the caller must not be able to miss this: {report}"));
    assert!(
        warning.contains("could not be read") || warning.contains("unreadable"),
        "and it must say why no copy exists: {warning}"
    );
    assert!(
        !copy.exists(),
        "no half-written copy should be left claiming to be a backup"
    );

    // And the repair happened: the set is readable again, from the file.
    // One more than the test's own writes: the set's charter, which its first memory has to be.
    assert_eq!(report["memories"], json!(2));
    let found = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "shared:damaged", "limit": 5}),
    ));
    assert_eq!(
        found["memories"].as_array().map(Vec::len),
        Some(1),
        "the restored set must be usable: {found}"
    );
}

/// Restoring a set this machine does not hold is refused rather than silently creating one.
#[test]
fn restore_refuses_a_set_that_is_not_loaded() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    let response = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "shared:never-loaded",
            "file": out.path().join("anything.json").to_string_lossy(),
            "save_to": out.path().join("copy.json").to_string_lossy()
        }),
    );
    assert!(is_error(&response));
    assert!(
        message(&response).contains("never-loaded"),
        "{}",
        message(&response)
    );
}

/// A file that is not this set's is refused before anything is destroyed, by the same header check
/// `import` uses. The safety copy is beside the point if the source was never usable.
#[test]
fn restore_refuses_a_file_belonging_to_another_set_without_destroying_anything() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    sets(
        &mut server,
        json!({"action": "create", "name": "elsewhere"}),
    );
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:elsewhere", "content": "alpha belongs to another set",
            "why": "a fixture", "confidence": 0.8, "origin": "llm"
        }),
    );
    let foreign = out.path().join("elsewhere.json");
    sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:elsewhere",
            "file": foreign.to_string_lossy()
        }),
    );

    store_one(&mut server, "alpha is local and must survive");

    let response = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": foreign.to_string_lossy(),
            "save_to": out.path().join("copy.json").to_string_lossy()
        }),
    );
    assert!(is_error(&response), "{}", message(&response));

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(
        local_set(&listed)["memories"],
        json!(1),
        "the local set must be untouched: {listed}"
    );
}

// ---------------------------------------------------------------------------
// A forget is one-way at import
//
// `an-id-can-hold-both-a-memory-and-a-tombstone` and
// `an-older-file-silently-resurrects-a-forgotten-memory` were fixed as a pair on 2026-08-26: the
// resurrection was written and reported. Since 2026-09-28 it is not written at all
// (`merge-without-a-recorded-ancestor`, decided with the human partner): an older copy and a reverted
// file are the same object from the record's side, and the deliberate undo is a new memory
// superseding the forgotten id.
// ---------------------------------------------------------------------------

/// Put an earlier copy of the file back at the same path, as `git checkout` of an earlier commit,
/// a `git revert` of the commit carrying a tombstone, or a restored backup all do.
fn rewind_the_file(out: &TempDir, earlier: &str) {
    std::fs::write(memories_file(out), earlier).expect("writes");
}

/// **A file that went backwards cannot bring a forgotten memory back.** The base is the tombstone,
/// so the three-way arithmetic reads the older file as a change and would take it; the store keeps
/// the tombstone instead, and the set still exports as it was.
#[test]
fn an_import_keeps_a_forgotten_memory_forgotten() {
    let home = tempdir();
    let out = tempdir();
    let after = tempdir();
    let mut server = a_server(&home);

    let id = store_local(&mut server, "alpha will be forgotten and must stay so");
    export_to(&mut server, &out);
    let earlier = std::fs::read_to_string(memories_file(&out)).expect("reads");

    call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": id}),
    );
    export_to(&mut server, &out);

    rewind_the_file(&out, &earlier);
    let report = payload(&import_from(&mut server, &out));
    assert_eq!(report["outcomes"]["kept_forgotten"], json!(1), "{report}");
    assert!(
        report["outcomes"].get("took_from_file").is_none(),
        "nothing may be taken from a file that went backwards: {report}"
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(local_set(&listed)["forgotten"], json!(1), "{listed}");

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&after)}),
    );
    assert!(!is_error(&exported), "{}", message(&exported));
    let reread = memory_file::read(
        &Expectation::Exactly(Scope::Local),
        std::fs::File::open(memories_file(&after)).expect("opens"),
    )
    .expect("the set still exports something readable");
    assert_eq!(reread.tombstones.len(), 1, "the forget still stands");
}

/// **And the response says so, and says how to undo a forget on purpose**, because a reverted file
/// looks exactly like this and only the caller knows which it was.
#[test]
fn an_import_that_keeps_a_memory_forgotten_says_how_to_bring_it_back() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    let id = store_local(&mut server, "alpha will be forgotten and must stay so");
    export_to(&mut server, &out);
    let earlier = std::fs::read_to_string(memories_file(&out)).expect("reads");

    call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": id}),
    );
    export_to(&mut server, &out);

    rewind_the_file(&out, &earlier);
    let report = payload(&import_from(&mut server, &out));

    assert_eq!(
        report["kept_forgotten"],
        json!([id]),
        "the memory kept forgotten should be named: {report}"
    );
    assert!(
        report
            .get("taken_backwards")
            .is_none_or(|backwards| !backwards
                .as_array()
                .is_some_and(|ids| ids.contains(&json!(id)))),
        "and not reported as taken, because it was not: {report}"
    );
    let guidance = report["kept_forgotten_guidance"]
        .as_str()
        .unwrap_or_default();
    assert!(
        guidance.contains("supersedes"),
        "the guidance should name the way to undo a forget on purpose: {report}"
    );
}

/// The same shortfall on a memory that is still there: an older file quietly winds a live record
/// back, and the remark is the only thing that says so.
///
/// **Why the store's move arrives by import.** This test used to amend after exporting, which
/// `correction-gated-on-service` refuses — an exported record can no longer be changed in place at
/// all. The remaining routes are a `forget` (which the test above already covers) and an import
/// against another target, so this takes the second one. The remark under test is unchanged: it
/// fires on the *file* being older than what it replaces, whatever moved the store.
#[test]
fn an_import_that_reverts_an_earlier_change_says_so() {
    let home = tempdir();
    let out = tempdir();
    let elsewhere = tempdir();
    let mut server = a_server(&home);

    let id = store_local(&mut server, "alpha keeps its claim throughout");
    export_to(&mut server, &out);
    let earlier = std::fs::read_to_string(memories_file(&out)).expect("reads");

    // A later version of the same record, arriving from a second checkout: a changed field and a
    // `modified` stamp after ours, which is what any real edit leaves behind.
    export_to(&mut server, &elsewhere);
    let path = memories_file(&elsewhere);
    let text = std::fs::read_to_string(&path).expect("reads");
    // Line-aware rather than a blanket replace: the set's identity record is a line of this file
    // too, and moving *its* `modified` would drag it into the report this test is reading. The
    // subject is one content record going backwards.
    let theirs = text
        .lines()
        .map(|line| {
            if line.contains("hoard-author") {
                return line.to_owned();
            }
            line.replace("\"confidence\":0.8", "\"confidence\":0.95")
                .replace("\"modified\":{\"at\":\"2026", "\"modified\":{\"at\":\"2099")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_ne!(theirs, text, "the fixture must actually change the record");
    std::fs::write(&path, &theirs).expect("writes");
    let taken = payload(&import_from(&mut server, &elsewhere));
    assert_eq!(taken["outcomes"]["took_from_file"], json!(1), "{taken}");

    // And now the first checkout, still holding the version from before that change.
    export_to(&mut server, &out);
    rewind_the_file(&out, &earlier);
    let report = payload(&import_from(&mut server, &out));
    assert_eq!(
        report["taken_backwards"]
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec![id.as_str()]),
        "reverting a change is a record taken backwards: {report}"
    );
}

/// **And an ordinary pull is not remarked on**, which is what keeps the report worth reading.
///
/// **This test was written wrong first and is worth the note.** Its first version rolled the *store*
/// back and re-imported, which produces `unchanged` — no `TakeFile` at all, so it passed without
/// ever reaching the check it names. A mutation reversing the comparison caught the two positive
/// tests and not this one, which is what exposed it. The file has to be genuinely newer than the
/// store, which is what a colleague's edit arriving looks like.
#[test]
fn an_ordinary_pull_is_not_reported_as_taken_backwards() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    store_local(&mut server, "alpha as first written");
    export_to(&mut server, &out);

    // Someone else's later amendment, arriving in the file: a changed field and a `modified` stamp
    // after ours. Written into the file directly, because the alternative is a second server and
    // this test is about the reading side.
    let text = std::fs::read_to_string(memories_file(&out)).expect("reads");
    let theirs = text
        .replace("\"confidence\":0.8", "\"confidence\":0.95")
        .replace("\"modified\":{\"at\":\"2026", "\"modified\":{\"at\":\"2099");
    assert_ne!(theirs, text, "the fixture must actually change the record");
    std::fs::write(memories_file(&out), &theirs).expect("writes");

    let report = payload(&import_from(&mut server, &out));
    assert_eq!(
        report["outcomes"]["took_from_file"],
        json!(2),
        "the fixture must exercise a TakeFile, or it proves nothing: {report}"
    );
    assert!(
        report.get("taken_backwards").is_none(),
        "a pull that brings newer records must say nothing: {report}"
    );
}

// ---------------------------------------------------------------------------
// One directory is one reconciliation target, however it is spelled
//
// `target-identity-is-an-unnormalised-path-string`. A target was the caller's path as typed, so
// `/p/proj` and `/p/proj/`, an absolute path and a relative one, and a symlinked route to the same
// checkout were four targets with four independent base sets for one directory. Export remarked on
// the unrecognised spelling; import said nothing at all, reconciling the whole file with **no
// ancestor** -- the one case the per-record base exists to prevent.
// ---------------------------------------------------------------------------

/// A trailing separator is the same directory, and must be the same target.
#[test]
fn two_spellings_of_one_directory_are_one_target() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_with_notes(&home, "alpha is exported once");

    sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    );

    // The same directory, spelled with a trailing separator. Before normalisation this reconciled
    // against nothing, because the base it needed was filed under the other spelling.
    let report = payload(&sets(
        &mut server,
        json!({
            "action": "import", "scope": "shared:notes",
            "directory": format!("{}/", out.path().to_string_lossy())
        }),
    ));

    assert_eq!(
        report["outcomes"]["unchanged"],
        // The memory, the author's identity record and the set's charter.
        json!(3),
        "the same directory by another spelling must find the agreement the export recorded: \
         {report}"
    );
    assert!(
        report.get("note").is_none(),
        "and must not be remarked on as a directory this set has never been written to: {report}"
    );
}

/// A symlinked route to one checkout is that checkout, not a second one.
///
/// The same resolution `Store::file` already applies to a database path, for the same reason: it
/// names where the data actually is rather than the route configured to reach it, which is what
/// makes two homes sharing one store through a symlink report one file rather than two.
#[test]
fn a_symlinked_route_to_a_checkout_is_the_same_target() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_with_notes(&home, "alpha is reached two ways");

    let real = out.path().join("checkout");
    std::fs::create_dir_all(&real).expect("creates");
    let linked = out.path().join("by-another-name");
    std::os::unix::fs::symlink(&real, &linked).expect("links");

    sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": real.to_string_lossy()}),
    );
    let report = payload(&sets(
        &mut server,
        json!({"action": "import", "scope": "shared:notes", "directory": linked.to_string_lossy()}),
    ));

    assert_eq!(
        report["outcomes"]["unchanged"],
        // The memory, the author's identity record and the set's charter.
        json!(3),
        "a symlinked route is the same directory: {report}"
    );
}

/// **Import says the unrecognised target out loud**, which the design calls the load-bearing half.
///
/// No key removes the need for this: canonicalising collapses spellings, but it does not survive a
/// checkout being renamed or moved, which orphans every base just as thoroughly and reads as a
/// target never seen before. Export has remarked on this since it was built; import reconciled
/// against no ancestor in silence, and nothing in its response distinguished that from a genuine
/// first import.
#[test]
fn import_says_when_it_is_reconciling_against_no_agreement() {
    let home = tempdir();
    let known = tempdir();
    let elsewhere = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha lives in a known checkout");

    export_to(&mut server, &known);

    // A second directory holding a copy of the file, as a moved or re-cloned checkout would.
    std::fs::create_dir_all(elsewhere.path().join("scatterhoard_mem")).expect("creates");
    std::fs::copy(memories_file(&known), memories_file(&elsewhere)).expect("copies");

    let report = payload(&sets(
        &mut server,
        json!({"action": "import", "scope": "local", "file": local_backup(&elsewhere)}),
    ));

    let note = report["note"].as_str().unwrap_or_else(|| {
        panic!("import must not reconcile against nothing in silence: {report}")
    });
    assert!(
        note.contains(&known.path().to_string_lossy().to_string())
            || note.contains("scatterhoard-sets"),
        "the remark should name the target this set does know: {note}"
    );

    // And the first import of a set genuinely has nothing to remark on.
    let fresh = tempdir();
    let mut second = a_server(&fresh);
    store_one(&mut second, "alpha somewhere new");
    let first_time = tempdir();
    export_to(&mut second, &first_time);
    let quiet = payload(&import_from(&mut second, &first_time));
    assert!(
        quiet.get("note").is_none(),
        "a set's own checkout is not an unrecognised target: {quiet}"
    );
}

// ---------------------------------------------------------------------------
// One spelling for a scope, in both directions
// ---------------------------------------------------------------------------

/// Every place a scope appears in `value` spelled as a serde-derived object rather than as the
/// scalar the schema publishes.
///
/// Walks the whole payload rather than naming fields: a test listing the fields it knows about
/// would pass for a field added later, which is how the two spellings came to coexist.
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

/// **A scope on the wire is one spelling, and it is the one the schema publishes.**
///
/// `a-scope-in-a-response-is-not-the-spelling-the-schema-publishes`.
/// `oneof-tool-schemas-present-no-parameters` settled that `scope` publishes as `local` or
/// `shared:<name>` — the spelling `Scope::as_key` writes into a store's `meta` row — while the
/// object form keeps *parsing*, so nothing that worked stopped working. That covered arguments and
/// was never applied to responses, where `Scope`'s derived `Serialize` runs. Two spellings for one
/// concept, at the one boundary that was standardised to prevent exactly that.
///
/// **The file format is untouched and must stay so.** The header's object form is what every
/// exported `memories.json` on disk carries; this is about responses only.
#[test]
fn every_scope_hoard_sets_reports_is_the_spelling_the_schema_publishes() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    let created = payload(&sets(
        &mut server,
        json!({"action": "create", "name": "spelled"}),
    ));
    assert_eq!(created["created"], json!("shared:spelled"));
    write_charter(&mut server, "spelled");

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:spelled", "content": "alpha is in a shared set",
            "why": "a fixture", "confidence": 0.8, "origin": "llm"
        }),
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let entry = listed["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|set| set["scope"] == json!("shared:spelled"))
        .unwrap_or_else(|| panic!("status should publish the scalar spelling: {listed}"));
    // The charter and the memory.
    assert_eq!(entry["memories"], json!(2));

    let authored = payload(&sets(
        &mut server,
        json!({"action": "rename", "scope": "shared:spelled", "author": "someone"}),
    ));
    assert_eq!(authored["scope"], json!("shared:spelled"));

    let rebuilt = payload(&sets(
        &mut server,
        json!({"action": "reembed", "scope": "shared:spelled"}),
    ));
    assert_eq!(rebuilt["scope"], json!("shared:spelled"));

    let file = out.path().join("spelled.json");
    sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:spelled",
            "file": file.to_string_lossy()
        }),
    );
    let restored = payload(&sets(
        &mut server,
        json!({
            "action": "restore", "scope": "shared:spelled",
            "file": file.to_string_lossy(),
            "save_to": out.path().join("before.json").to_string_lossy()
        }),
    ));
    assert_eq!(restored["restored"], json!("shared:spelled"));

    let dropped = payload(&sets(
        &mut server,
        json!({
            "action": "drop", "name": "spelled",
            "save_to": out.path().join("gone.json").to_string_lossy()
        }),
    ));
    assert_eq!(dropped["dropped"], json!("shared:spelled"));

    // And nowhere in any of them, at any depth.
    let mut found = Vec::new();
    for (name, payload) in [
        ("create", &created),
        ("status", &listed),
        ("author", &authored),
        ("reembed", &rebuilt),
        ("restore", &restored),
        ("drop", &dropped),
    ] {
        object_form_scopes(payload, name, &mut found);
    }
    assert!(
        found.is_empty(),
        "the object form reached a response: {found:?}"
    );
}

/// **The exported file keeps the object form**, which is the half that must not move.
///
/// The header's `{"shared": …}` is what every `memories.json` already on disk carries, and
/// `documents/design/file-format.md` documents it. Changing responses is a surface fix; changing
/// the file would be a format change needing a version bump. This test is here so that a later
/// tidying pass that "makes them consistent" fails rather than rewrites every export.
#[test]
fn the_exported_file_header_keeps_the_object_form() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    sets(&mut server, json!({"action": "create", "name": "on-disk"}));
    let file = out.path().join("on-disk.json");
    sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:on-disk",
            "file": file.to_string_lossy()
        }),
    );

    let header = std::fs::read_to_string(&file).expect("reads");
    let header = header.lines().next().expect("a header line");
    let parsed: Value = serde_json::from_str(header).expect("the header is JSON");
    assert_eq!(
        parsed["scope"],
        json!({"shared": "on-disk"}),
        "the file format is not what this fix changes: {header}"
    );
}

// ---------------------------------------------------------------------------
// The surface itself
// ---------------------------------------------------------------------------

/// `no-tool-lists-loaded-scopes`, closed: the refusal has to name something the caller can do.
///
/// It previously said *"check the set name"*, with no way to list names, and *"have to be imported"*,
/// with no import tool. Both named actions no caller could take.
#[test]
fn the_unloaded_set_refusal_points_at_a_reachable_action() {
    let home = tempdir();
    let mut server = a_server(&home);

    let refusal = message(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "not-loaded"},
            "content": "a memory for a set that does not exist",
            "why": "to read the refusal",
            "confidence": 0.5,
            "origin": "llm"
        }),
    ));

    assert!(
        refusal.contains("hoard_sets"),
        "the refusal should name the tool that can fix it: {refusal}"
    );
}

/// The published schema and the deserialiser are two hand-written descriptions of one thing, which
/// `STANDARDS.md` says to test against each other rather than against a third list.
#[test]
fn every_action_the_schema_publishes_is_accepted() {
    let home = tempdir();
    let mut server = a_server(&home);

    let listed = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}),
    );
    let tools = listed["result"]["tools"].as_array().expect("an array");
    let schema = tools
        .iter()
        .find(|tool| tool["name"] == json!("hoard_sets"))
        .map(|tool| tool["inputSchema"].clone())
        .expect("hoard_sets should be published");

    // Read from the flat `action` enum, which is now the whole of what is published: the per-action
    // `oneOf` branches were removed on 2026-08-11 because the Anthropic API refuses a top-level
    // alternation and fails the entire tool list over it. The branches still exist inside `mcp.rs`
    // and still generate this enum, so this remains a test of the schema against the deserialiser
    // rather than against a third list — it just reads the schema where the schema now is.
    let published = schema["properties"]["action"]["enum"]
        .as_array()
        .expect("the published action enum");
    assert!(!published.is_empty(), "no actions published");

    for action in published {
        let action = action.as_str().expect("each action is a string");

        // `create` needs a name; supplying one for every action is harmless, because an action that
        // does not take one must still not reject the call as an unknown action.
        let response = sets(
            &mut server,
            json!({"action": action, "name": "schema-probe"}),
        );
        let complaint = message(&response);
        assert!(
            !complaint.contains("unknown action"),
            "the schema publishes {action:?} but the parser rejects it: {complaint}"
        );
    }
}

#[test]
fn an_unknown_action_names_the_ones_that_exist() {
    let home = tempdir();
    let mut server = a_server(&home);

    let response = sets(&mut server, json!({"action": "obliterate"}));
    assert!(is_error(&response));

    let complaint = message(&response);
    assert!(
        complaint.contains("status") && complaint.contains("create"),
        "an unknown action should list the available ones: {complaint}"
    );
}

// ---------------------------------------------------------------------------
// Who a set attributes writes to
// ---------------------------------------------------------------------------

/// A server whose process-wide default identity is `author` — the `SCATTERHOARD_AUTHOR` half.
fn a_server_as(home: &TempDir, author: &str) -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new(author).expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    server
}

fn local_status(server: &mut Server) -> Value {
    let status = payload(&sets(server, json!({"action": "status"})));
    status["sets"]
        .as_array()
        .expect("a list of sets")
        .iter()
        .find(|set| set["scope"] == json!("local"))
        .cloned()
        .unwrap_or_else(|| panic!("local should be listed, got {status}"))
}

#[test]
fn status_names_who_a_set_attributes_writes_to() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jtcours");
    store_one(&mut server, "alpha");

    assert_eq!(
        local_status(&mut server)["author"],
        json!("jtcours"),
        "status should report the identity the set records"
    );
}

/// A set nothing has written to has no author, and saying so is the honest answer — inventing one
/// from the asking process is the defect this mechanism removes.
#[test]
fn status_reports_no_author_for_a_set_nothing_has_claimed() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jtcours");

    assert_eq!(local_status(&mut server)["author"], json!(null));
}

/// **A set attributing to a name this server is not configured for is the expected state, and
/// saying so is noise.**
///
/// This used to report it as a disagreement worth a reader's attention. Identity is a function of a
/// set's *origin*, not of the person or the process — one contributor is `alice` on their
/// machine, `alicedev` on GitHub, `asmith` at work — so a server configured with one of those differs
/// from most sets it holds, permanently and correctly. A note that fires forever on a healthy set is
/// a note readers learn to skip, which costs the ones that mean something.
/// `the-author-note-compares-against-the-wrong-thing`.
#[test]
fn status_is_silent_when_a_set_attributes_to_a_name_this_server_is_not_configured_for() {
    let home = tempdir();
    let path = home.path().join("shared-team.sqlite3");

    let mut owner = Server::new();
    owner.set_account_name(Author::new("jeff").expect("valid author"));
    let store = Store::open_at(scope_team(), &path).expect("opens");
    store
        .mint_set_id()
        .expect("mints an identity, as create does");
    owner.add_store(store);
    write_charter_as(&mut owner, "team", "jeff");
    store_into(&mut owner, "team", "alpha");

    let mut visitor = Server::new();
    visitor.set_account_name(Author::new("jtcours@work").expect("valid author"));
    visitor.add_store(Store::open_at(scope_team(), &path).expect("opens"));

    let status = payload(&sets(&mut visitor, json!({"action": "status"})));
    let set = status["sets"]
        .as_array()
        .expect("a list of sets")
        .iter()
        .find(|set| set["scope"] != json!("local"))
        .cloned()
        .expect("the shared set should be listed");

    // The author is reported plainly, which is where a reader who cares can see it.
    assert_eq!(set["author"], json!("jeff"));
    assert!(
        set["author_note"].is_null(),
        "a set writing under its own identity is working correctly and needs no note: {set}"
    );
}

/// **What replaces it: the state where the next write cannot proceed.**
///
/// A set taken on by import arrives holding another contributor's identity record and no cached
/// answer to *which record is you* — `meta` does not travel in a memory file. The next write is
/// refused rather than guessing, which is correct and is exactly the kind of thing `status` should
/// say before the caller meets it mid-write.
#[test]
fn status_says_when_the_next_write_will_have_to_ask_which_identity_is_yours() {
    let home = tempdir();
    let repository = tempdir();
    {
        let author_home = tempdir();
        let mut authoring = a_server_as(&author_home, "someone-else");
        create_chartered_as(&mut authoring, "taken-on", "someone-else");
        let stored = call(
            &mut authoring,
            "hoard_memorize",
            json!({
                "scope": {"shared": "taken-on"}, "content": "alpha, authored elsewhere",
                "confidence": 0.8, "origin": "llm", "why": "so an identity record exists"
            }),
        );
        assert!(!is_error(&stored), "{}", message(&stored));
        sets(
            &mut authoring,
            json!({
                "action": "export", "scope": {"shared": "taken-on"},
                "directory": repository.path()
            }),
        );
    }

    let mut server = a_server_as(&home, "jtcours");
    sets(&mut server, json!({"action": "create", "name": "taken-on"}));
    let imported = sets(
        &mut server,
        json!({
            "action": "import", "scope": {"shared": "taken-on"}, "directory": repository.path()
        }),
    );
    assert!(!is_error(&imported), "{}", message(&imported));

    let status = payload(&sets(&mut server, json!({"action": "status"})));
    let set = status["sets"]
        .as_array()
        .expect("a list of sets")
        .iter()
        .find(|set| set["scope"] == json!("shared:taken-on"))
        .cloned()
        .expect("the set should be listed");

    let note = set["author_note"].as_str().unwrap_or_default();
    assert!(
        note.contains("identify"),
        "the note should name the action that answers it: {set}"
    );
}

/// **A store that predates author recording holds memories, and the note must not deny them.**
///
/// `author-note-denies-writes-to-a-set-that-has-them`, found 2026-08-11 running SETUP.md §5 item 13
/// against a real store: `author: null` with *"Nothing has been written to this set yet"* arrived
/// beside `memories: 27`, and an export of that same set showed all 27 records carrying an author.
/// The consequence was right and the explanation was false, which leaves a reader deciding which of
/// two fields in one object to disbelieve.
///
/// The sibling field already models the fix one line away: `embedding_model` reports
/// `provenance_unknown` for exactly this class of store, naming the migration without denying the
/// history.
#[test]
fn the_author_note_does_not_deny_writes_to_a_set_that_holds_memories() {
    let home = tempdir();
    let path = home.path().join("shared-team.sqlite3");

    // Inserted through the store rather than through a server, which is what a build predating
    // store-level author recording did: the record carries an author, the store does not.
    let mut unclaimed = Store::open_at(scope_team(), &path).expect("opens");
    unclaimed
        .mint_set_id()
        .expect("mints an identity, as create does");
    unclaimed
        .insert(&a_bare_memory("alpha worth keeping"))
        .expect("inserts");
    drop(unclaimed);

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(Store::open_at(scope_team(), &path).expect("opens"));

    let status = payload(&sets(&mut server, json!({"action": "status"})));
    let set = status["sets"]
        .as_array()
        .expect("a list of sets")
        .iter()
        .find(|set| set["scope"] != json!("local"))
        .cloned()
        .expect("the shared set should be listed");

    assert_eq!(set["author"], json!(null), "{set}");
    assert_eq!(set["memories"], json!(1), "{set}");

    let note = set["author_note"].as_str().unwrap_or_default();
    assert!(
        !note.contains("Nothing has been written"),
        "the set holds memories, so the note must not say nothing was written: {note}"
    );
    // And it says what the next write needs: a shared set is claimed by naming who writes, not by
    // the name this server runs as.
    assert!(
        note.contains("\"mint\"") && !note.contains("jtcours"),
        "it must say how the set is claimed: {note}"
    );
}

/// **The genuinely empty set is a different state and still says so.**
///
/// Kept beside the test above because the fix for it could easily collapse the two into one hedged
/// sentence that describes neither. An empty set really has had nothing written to it, and saying
/// so is the useful thing to say.
#[test]
fn the_author_note_still_reports_a_set_nothing_has_been_written_to() {
    let home = tempdir();
    let path = home.path().join("shared-team.sqlite3");

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(Store::open_at(scope_team(), &path).expect("opens"));

    let status = payload(&sets(&mut server, json!({"action": "status"})));
    let set = status["sets"]
        .as_array()
        .expect("a list of sets")
        .iter()
        .find(|set| set["scope"] != json!("local"))
        .cloned()
        .expect("the shared set should be listed");

    assert_eq!(set["memories"], json!(0), "{set}");
    let note = set["author_note"].as_str().unwrap_or_default();
    assert!(
        note.contains("Nothing has been written"),
        "an empty set has genuinely had nothing written to it: {note}"
    );
}

fn a_bare_memory(content: &str) -> Memory {
    Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: jiff::Timestamp::from_second(1_754_000_000).expect("valid timestamp"),
        },
    )
}

/// **Renaming re-attributes what is already stored, which inverts what this used to assert.**
///
/// Two tests stood here saying the opposite: that changing a set's author applied to future writes
/// only, that memories already stored kept the identity they were written under, and that the
/// response should say how many that was so the split became visible. All of it was true while `by`
/// held a name — a stored record could not change without being rewritten, so a name change left a
/// set speaking with two voices and the honest thing was to count them.
///
/// Holding `by` as a reference removes the split rather than reporting it. Nothing is rewritten and
/// everything re-attributes, because the memories point at a record and the record is what moved.
/// There is no *attributed otherwise* count any more, because there is no otherwise.
#[test]
fn renaming_re_attributes_memories_already_stored() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jtcours");
    store_one(&mut server, "alpha");

    let renamed = payload(&sets(
        &mut server,
        json!({"action": "rename", "scope": "local", "author": "jeff@example.com"}),
    ));
    assert_eq!(renamed["author"], json!("jeff@example.com"));
    assert_eq!(renamed["was"], json!("jtcours"));

    store_one(&mut server, "beta");
    assert_eq!(
        authors_of(&mut server),
        vec!["jeff@example.com", "jeff@example.com"],
        "the memory stored before the rename must read as the new name too, without having moved"
    );
}

#[test]
fn setting_the_author_of_a_set_that_is_not_loaded_is_refused() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jtcours");

    let response = sets(
        &mut server,
        json!({"action": "mint", "scope": {"shared": "absent"}, "author": "jeff"}),
    );
    assert!(is_error(&response), "got {response}");
}

#[test]
fn a_malformed_author_is_refused_rather_than_recorded() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jtcours");
    store_one(&mut server, "alpha");

    let response = sets(
        &mut server,
        json!({"action": "mint", "scope": "local", "author": "   "}),
    );
    assert!(is_error(&response), "got {response}");
    assert_eq!(
        local_status(&mut server)["author"],
        json!("jtcours"),
        "a refused change must leave the recorded identity alone"
    );
}

/// Export is the moment attribution stops being private, and therefore the last cheap moment to
/// notice it is wrong.
#[test]
fn export_reports_who_the_set_attributes_writes_to() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_as(&home, "jtcours");
    store_one(&mut server, "alpha");

    assert_eq!(
        payload(&export_to(&mut server, &out))["author"],
        json!("jtcours")
    );
}

/// **A created set records no author**, since `set-settled-at-first-write` step 3. Creation used to
/// record the server's configured name, which is how a corpus came to be attributed to a machine
/// account; now the first write asks who you are in the set, because that is how you are known where
/// it is published.
#[test]
fn a_created_set_records_no_author() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jeff");
    sets(&mut server, json!({"action": "create", "name": "fresh"}));

    let store = Store::open(scope_fresh(), home.path()).expect("opens");
    assert_eq!(store.author_display().expect("reads the author"), None);
}

fn scope_team() -> Scope {
    Scope::Shared(SetName::new("team").expect("valid set name"))
}

fn scope_fresh() -> Scope {
    Scope::Shared(SetName::new("fresh").expect("valid set name"))
}

fn store_into(server: &mut Server, set: &str, content: &str) {
    let response = call(
        server,
        "hoard_memorize",
        json!({"scope": {"shared": set}, "content": content, "confidence": 0.7, "origin": "llm"}),
    );
    assert!(!is_error(&response), "memorize should succeed: {response}");
}

/// Who the local set's memories are attributed to, sorted.
fn authors_of(server: &mut Server) -> Vec<String> {
    let found = payload(&call(
        server,
        "hoard_recall",
        json!({"query": "alpha beta"}),
    ));
    // `by` carries the identity record's id, so the name is one fetch away -- which is the trade
    // the design makes deliberately: the id is the handle, and going id-to-name is a lookup that
    // already exists where name-to-id would be a search that can answer twice or not at all.
    // The memories asked about by name rather than by where a cut fell: recall returns as many as
    // it is asked for, so anything else in the set comes back beside them.
    let ids: Vec<String> = found["memories"]
        .as_array()
        .expect("a list of memories")
        .iter()
        .filter(|memory| {
            memory["content"]
                .as_str()
                .is_some_and(|content| content.starts_with("alpha") || content.starts_with("beta"))
        })
        .filter_map(|memory| memory["created"]["by"].as_str())
        .map(ToOwned::to_owned)
        .collect();
    let mut authors: Vec<String> = ids
        .iter()
        .map(|id| {
            let record = payload(&call(server, "hoard_lookup", json!({"id": id})));
            if let Some(name) = record["memories"][0]["content"].as_str() {
                return name.to_owned();
            }
            // The record was superseded *and* forgotten, which is what `rename` does — so the
            // tombstone carries no name and says where the chain goes instead. One hop, which is
            // the walk `hoard_lookup` describes.
            let tip = record["forgotten"][0]["superseded_by"][0]
                .as_str()
                .map(ToOwned::to_owned);
            let Some(tip) = tip else {
                return "<unresolved>".to_owned();
            };
            let replacement = payload(&call(server, "hoard_lookup", json!({"id": tip})));
            replacement["memories"][0]["content"]
                .as_str()
                .unwrap_or("<unresolved>")
                .to_owned()
        })
        .collect();
    authors.sort();
    authors
}

// ---------------------------------------------------------------------------
// Which file a set is being read from
// ---------------------------------------------------------------------------

/// **`status` names the file each set lives in.**
///
/// A plain fact rather than a check, and it is what makes a misdirected `SCATTERHOARD_LOCAL`
/// diagnosable. Point that variable somewhere new and the server opens an empty store there, quite
/// correctly — the memories are safe in the old directory, and every symptom available to the caller
/// says `local`, `0 memories`, nothing wrong. The path is the one piece of information that
/// distinguishes *"you have no memories"* from *"you are looking in the wrong place"*, and neither
/// the LLM nor the human can supply it from anywhere else.
#[test]
fn status_names_the_file_each_set_is_read_from() {
    let home = tempdir();
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open(Scope::Local, home.path()).expect("opens"));

    // Canonicalised, because the reported path is SQLite's own and is fully resolved: on macOS the
    // temporary directory reaches us as `/var/...` and comes back as `/private/var/...`. That
    // resolution is a property worth having rather than one to work around — it names where the data
    // is rather than the route to it — so the test compares like with like instead of asking for the
    // spelling it happened to pass in.
    let expected = std::fs::canonicalize(Store::path_for(&Scope::Local, home.path()))
        .expect("the store file exists");
    assert_eq!(
        local_status(&mut server)["file"],
        json!(expected.to_string_lossy()),
        "status should say which file the local set is being read from"
    );
}

/// Two homes reaching one store through a symlink report **one file**, not two.
///
/// The configuration `SCATTERHOARD_LOCAL` exists to make ordinary, tested here through the symlink
/// route because that route is how it can be exercised without the variable. The resolved path is
/// what makes the sharing visible: two servers that disagree about whether they are sharing would
/// otherwise have no way to find out, and "am I looking at the same file as the other one" is
/// exactly the question two checkouts need answered.
#[test]
fn two_homes_sharing_one_store_through_a_symlink_report_the_same_file() {
    let real = tempdir();
    let alias = tempdir();

    let target = Store::path_for(&Scope::Local, real.path());
    Store::open_at(Scope::Local, &target).expect("creates the real store");
    std::os::unix::fs::symlink(&target, Store::path_for(&Scope::Local, alias.path()))
        .expect("links the second home at it");

    let direct = Store::open(Scope::Local, real.path()).expect("opens");
    let linked = Store::open(Scope::Local, alias.path()).expect("opens through the symlink");

    assert_eq!(
        direct.file(),
        linked.file(),
        "two routes to one store must report the one file they share"
    );
}

/// A store with no file says so, rather than reporting an empty string as though it were a path.
#[test]
fn a_set_with_no_file_reports_none() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jtcours");

    assert_eq!(local_status(&mut server)["file"], json!(null));
}

// ---------------------------------------------------------------------------
// Checkout-bound and machine-bound sets
// ---------------------------------------------------------------------------

/// A server over a real home and a real common directory, loading whatever discovery finds — the
/// startup path, assembled the way `main.rs` assembles it.
fn a_server_over(home: &TempDir, common: Option<&TempDir>) -> Server {
    let layout = StoreLayout::new(
        home.path().to_path_buf(),
        common.map(|directory| directory.path().to_path_buf()),
    );
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(layout.clone());
    for (scope, path) in layout.discover().stores {
        let store = Store::open_at(scope, &path).expect("opens");
        store
            .mint_set_id()
            .expect("mints an identity, as create does");
        server.add_store(store);
    }
    server
}

/// The configuration the whole variable exists for: two checkouts, each with its own `scatterhoard`
/// store, sharing one `coding-lessons`.
#[test]
fn two_homes_reach_their_own_project_set_and_one_common_set() {
    let common = tempdir();
    let development = tempdir();
    let review = tempdir();

    let mut first = a_server_over(&development, Some(&common));
    create_chartered(&mut first, "scatterhoard");
    store_into(
        &mut first,
        "scatterhoard",
        "alpha the dev checkout wrote this",
    );

    // `create` always writes to the home, so becoming machine-bound is a move. Done here as the
    // documented procedure does it: with nothing holding the file open.
    let mut second = a_server_over(&review, Some(&common));
    create_chartered(&mut second, "coding-lessons");
    drop(second);
    std::fs::rename(
        Store::path_for(&scope_named("coding-lessons"), review.path()),
        Store::path_for(&scope_named("coding-lessons"), common.path()),
    )
    .expect("moves the store to the common directory");

    let mut second = a_server_over(&review, Some(&common));
    store_into(
        &mut second,
        "coding-lessons",
        "beta a lesson learned in review",
    );

    let mut first = a_server_over(&development, Some(&common));

    assert_eq!(
        loaded_scopes(&mut first),
        vec![
            json!("shared:coding-lessons"),
            json!("shared:scatterhoard"),
            json!("local")
        ],
        "development reaches its own project set and the common one"
    );
    assert_eq!(
        loaded_scopes(&mut second),
        vec![json!("shared:coding-lessons"), json!("local")],
        "review reaches the common set and has no project set of its own -- the development \
         checkout's must not leak into it"
    );
    assert_eq!(
        contents_of_set(&mut first, "coding-lessons"),
        vec!["beta a lesson learned in review"],
        "and a lesson recorded while reviewing is readable while developing, which is the point"
    );
}

/// A set in both directories: the common copy is live, and the losing one is *named* rather than
/// silently ignored — otherwise a half-finished move looks exactly like a completed one.
#[test]
fn a_set_in_both_directories_is_reported_as_shadowed() {
    let home = tempdir();
    let common = tempdir();

    let mut server = a_server_over(&home, Some(&common));
    sets(
        &mut server,
        json!({"action": "create", "name": "coding-lessons"}),
    );
    drop(server);
    std::fs::copy(
        Store::path_for(&scope_named("coding-lessons"), home.path()),
        Store::path_for(&scope_named("coding-lessons"), common.path()),
    )
    .expect("copies rather than moves, which is the mistake being tested");

    let mut server = a_server_over(&home, Some(&common));
    let status = payload(&sets(&mut server, json!({"action": "status"})));

    assert_eq!(
        status["shadowed"][0]["scope"],
        json!("shared:coding-lessons"),
        "the copy that lost should be reported: {status}"
    );
    assert_eq!(
        status["shadowed"][0]["file"],
        json!(Store::path_for(&scope_named("coding-lessons"), home.path()).to_string_lossy()),
        "and named by path, since that is what the reader has to act on"
    );
}

/// Naming the home as the common directory is harmless and does nothing, which is exactly why it has
/// to be said: it is a setting that looks applied and is not.
#[test]
fn naming_the_home_as_the_common_directory_is_reported_in_status() {
    let home = tempdir();

    let mut declared = a_server_over(&home, Some(&home));
    let note = payload(&sets(&mut declared, json!({"action": "status"})))["configuration_note"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        note.contains("SCATTERHOARD_COMMON"),
        "the note should name the variable that is doing nothing: {note:?}"
    );

    let mut ordinary = a_server_over(&home, None);
    assert_eq!(
        payload(&sets(&mut ordinary, json!({"action": "status"})))["configuration_note"],
        json!(null),
        "the default configuration must say nothing: nobody asked for anything"
    );
}

fn scope_named(name: &str) -> Scope {
    Scope::Shared(SetName::new(name).expect("valid set name"))
}

/// Every scope a server has loaded, in the order `status` reports them.
fn loaded_scopes(server: &mut Server) -> Vec<Value> {
    payload(&sets(server, json!({"action": "status"})))["sets"]
        .as_array()
        .expect("a list of sets")
        .iter()
        .map(|set| set["scope"].clone())
        .collect()
}

/// What one shared set holds, so a test can say which memories a server can actually reach.
fn contents_of_set(server: &mut Server, set: &str) -> Vec<String> {
    let found = payload(&call(
        server,
        "hoard_recall",
        json!({"scope": {"shared": set}, "query": "alpha beta"}),
    ));
    found["memories"]
        .as_array()
        .expect("a list of memories")
        .iter()
        .filter_map(|memory| memory["content"].as_str())
        .map(ToOwned::to_owned)
        .collect()
}

// ---------------------------------------------------------------------------
// Attribution survives a round trip through a file
// ---------------------------------------------------------------------------

/// **A record arriving from a file keeps the attribution the file gives it.**
///
/// The narrow guard, and the easy thing to get wrong: stamping an imported record with the importing
/// server's identity is a one-line change that looks like housekeeping — it is what `memorize` and
/// `amend` both correctly do — and every other test in this file would stay green. It would also be
/// wrong twice over. An import would rewrite the authorship of everyone else's memories on every
/// pull, and it would foreclose the only route there is to re-attributing memories in bulk (see
/// below, and `documents/design/store-registry.md`).
#[test]
fn an_imported_memory_keeps_the_file_s_attribution_not_the_importing_server_s() {
    let source = tempdir();
    let repository = tempdir();
    let destination = tempdir();

    // One shared set on two machines. Ours takes it on first, so the import that matters arrives at
    // a set it already holds -- the only way two machines exchange records since
    // `identity-mismatch-refusal`, which refuses crossing between two machines' `local` sets.
    let mut theirs = a_server_as(&source, "someone-else");
    create_chartered_as(&mut theirs, "main", "someone-else");
    store_in(&mut theirs, "main", "gamma they wrote first");
    sets(
        &mut theirs,
        json!({"action": "export", "scope": "shared:main", "directory": repository.path()}),
    );
    let mut ours = a_server_as(&destination, "jtcours");
    sets(&mut ours, json!({"action": "create", "name": "main"}));
    sets(
        &mut ours,
        json!({"action": "import", "scope": "shared:main", "directory": repository.path()}),
    );
    sets(
        &mut ours,
        json!({"action": "mint", "scope": "shared:main", "author": "jtcours"}),
    );

    // Our set holds a memory of its own before the import that matters, so it has *claimed an
    // identity* first. Without that this test cannot see the mistake it exists for: a store with no
    // recorded author has nothing to re-stamp imported records with, so the defect would be a
    // silent no-op here and the test would pass while proving nothing.
    store_in(&mut ours, "main", "beta we wrote this");
    store_in(&mut theirs, "main", "alpha they wrote this");
    sets(
        &mut theirs,
        json!({"action": "export", "scope": "shared:main", "directory": repository.path()}),
    );

    let imported = payload(&sets(
        &mut ours,
        json!({"action": "import", "scope": "shared:main", "directory": repository.path()}),
    ));
    assert_eq!(
        imported["outcomes"]["new_from_file"],
        json!(1),
        "{imported}"
    );

    assert_eq!(
        authors_of(&mut ours),
        vec!["jtcours", "someone-else"],
        "an import must not re-attribute the memories it is given, and must not disturb our own"
    );
}

/// **The whole reason the guard above matters**: renaming an identity across a set is done by
/// exporting it, editing the file, and importing it back.
///
/// `documents/design/store-registry.md` leaves bulk re-attribution undesigned on purpose — software
/// cannot tell *"me under a name I no longer use"* from *"that was someone else"*. This route does
/// not evade that; it puts a human at exactly that judgement, with a reviewable diff as the artifact.
/// So the capability is real, is the sanctioned procedure, and is worth a test that fails if anyone
/// closes it off.
///
/// Every record reports `took_from_file` rather than a conflict, and that is not luck: the export
/// records the base, so the file has moved and the store has not, which is the same branch of
/// `reconcile::resolve` an ordinary pull takes.
#[test]
fn an_identity_can_be_renamed_across_a_set_by_editing_an_export() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_server_as(&home, "jtcours");

    store_one(&mut server, "alpha worth keeping");
    store_one(&mut server, "beta also worth keeping");
    export_to(&mut server, &repository);

    rewrite_authors(&repository, "jtcours", "jtcours@github");

    let imported = payload(&sets(
        &mut server,
        json!({"action": "import", "scope": "local", "file": local_backup(&repository)}),
    ));

    // **One record moved, and two memories changed hands.** That asymmetry is the point: the
    // memories reconcile as `unchanged` because their own records did not move at all, while the
    // identity they point at did. Before `by` became a reference this said `took_from_file: 2`,
    // and it would have said 155 on the real set.
    assert_eq!(
        imported["outcomes"]["took_from_file"],
        json!(1),
        "only the identity record should have moved: {imported}"
    );
    assert_eq!(
        imported["outcomes"]["unchanged"],
        json!(2),
        "the memories themselves must not have been rewritten: {imported}"
    );
    assert_eq!(imported["conflicts"], json!(null), "{imported}");
    assert_eq!(
        authors_of(&mut server),
        vec!["jtcours@github", "jtcours@github"],
        "both memories should now resolve to the new name without either being rewritten"
    );
}

/// **Trimming records out of an exported file is not how to share a subset**, and this pins the
/// report that says so.
///
/// The operationally dangerous part is that it looks exactly like the re-attribution procedure
/// `store-registry.md` blesses — export, edit the file, import it back. That one edits records in
/// place and removes none, which is the whole difference: absence in a file means nothing on its
/// own, so a record the two sides previously agreed on that has since vanished from the file is not
/// a deletion. It is the `Anomaly` case, reported and never acted on.
///
/// Loud rather than silent, which is the property worth keeping. The exchange between two parties
/// is whole-set export to whole-set import, in both directions.
#[test]
fn a_record_trimmed_out_of_an_exported_file_re_imports_as_an_anomaly() {
    let home = tempdir();
    let repository = tempdir();
    let mut server = a_server_as(&home, "jtcours");

    store_one(&mut server, "alpha worth keeping");
    store_one(&mut server, "beta worth keeping too");
    export_to(&mut server, &repository);

    let dropped = remove_first_record(&repository);

    let imported = payload(&sets(
        &mut server,
        json!({"action": "import", "scope": "local", "file": local_backup(&repository)}),
    ));

    assert_eq!(
        imported["outcomes"]["anomaly"],
        json!(1),
        "the trimmed record should be reported, not read as a deletion: {imported}"
    );
    assert_eq!(
        imported["anomalies"],
        json!([dropped]),
        "and named, so the operator can tell which one: {imported}"
    );
    assert_eq!(
        authors_of(&mut server).len(),
        2,
        "nothing should have been removed from the store"
    );
}

/// Delete the first record from an exported file, returning the id it carried.
fn remove_first_record(repository: &TempDir) -> String {
    let path = memories_file(repository);
    let original = std::fs::read_to_string(&path).expect("reads the exported file");

    let mut kept = String::new();
    let mut dropped = None;
    for line in original.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(line).expect("each line is one JSON object");
        // The header has no `id`; only a record does, and the first one is the one to drop.
        if dropped.is_none() && record["id"].is_string() {
            dropped = Some(record["id"].as_str().expect("just checked").to_owned());
            continue;
        }
        kept.push_str(&record.to_string());
        kept.push('\n');
    }

    std::fs::write(&path, kept).expect("writes the trimmed file");
    dropped.expect("the export held at least one record")
}

/// Rewrite `from` to `to` wherever a record carries it, leaving everything else byte-identical.
///
/// Deliberately touches `created`, `modified` **and** `forgotten`: a tombstone carries attribution
/// too, and a re-attribution that skipped it would leave one identity behind in exactly the records
/// nobody thinks to check.
/// Rename an identity in an exported file by editing **one record**.
///
/// This used to rewrite `by` on every record, because `by` held the name. It holds a reference now,
/// so the name lives in one `hoard-author` record and re-attributing a whole set is a single edit —
/// which is the entire argument of `documents/design/memory-data-model.md`, exercised rather than
/// asserted.
fn rewrite_authors(repository: &TempDir, from: &str, to: &str) {
    let path = memories_file(repository);
    let original = std::fs::read_to_string(&path).expect("reads the exported file");

    let mut rewritten = String::new();
    let mut edited = 0usize;
    for line in original.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut record: Value = serde_json::from_str(line).expect("each line is one JSON object");
        let is_identity = record["keywords"]
            .as_array()
            .is_some_and(|keywords| keywords.iter().any(|keyword| keyword == "hoard-author"));
        if is_identity && record["content"] == json!(from) {
            record["content"] = json!(to);
            edited += 1;
        }
        rewritten.push_str(&record.to_string());
        rewritten.push('\n');
    }
    assert_eq!(
        edited, 1,
        "exactly one identity record should carry {from:?}"
    );

    std::fs::write(&path, rewritten).expect("writes the edited file");
}

// ---------------------------------------------------------------------------
// Tombstones travelling
// ---------------------------------------------------------------------------

/// **The claim the whole reconciliation design rests on**: absence means nothing, deletion is a line.
/// Forgetting a memory here has to reach a machine that still holds it.
#[test]
fn forgetting_propagates_through_export_and_import() {
    let ours = tempdir();
    let theirs = tempdir();
    let repository = tempdir();

    // Both machines start from the same exported set.
    let mut us = a_server(&ours);
    store_one(&mut us, "alpha is worth keeping");
    let doomed = payload(&call(
        &mut us,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "beta should never have been recorded",
            "why": "it leaked something",
            "confidence": 0.5,
            "origin": "llm"
        }),
    ))["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    export_to(&mut us, &repository);

    let mut them = a_server(&theirs);
    let adopted = payload(&import_from(&mut them, &repository));
    assert_eq!(adopted["outcomes"]["new_from_file"], json!(3));

    // We forget it, and export.
    call(
        &mut us,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": &doomed}),
    );
    export_to(&mut us, &repository);

    // The file carries a tombstone rather than simply omitting the line.
    let text = std::fs::read_to_string(memories_file(&repository)).expect("reads");
    assert!(
        text.contains("\"forgotten\""),
        "the export dropped the line instead of writing a tombstone: {text}"
    );
    assert!(
        !text.contains("beta should never have been recorded"),
        "the tombstone carried the content it exists to remove: {text}"
    );

    // They import, and it is gone from their store too.
    let report = payload(&import_from(&mut them, &repository));
    assert_eq!(
        report["outcomes"]["took_from_file"],
        json!(1),
        "the tombstone was not applied: {report}"
    );

    let looked_up = payload(&call(&mut them, "hoard_lookup", json!({"id": &doomed})));
    assert_eq!(
        looked_up["memories"].as_array().map(Vec::len),
        Some(0),
        "the memory survived on the other machine: {looked_up}"
    );

    // And the memory that was not forgotten is untouched.
    let kept = payload(&call(
        &mut them,
        "hoard_recall",
        json!({"query": "alpha", "limit": 10}),
    ));
    assert_eq!(kept["memories"].as_array().map(Vec::len), Some(1));
}

/// Importing the same tombstone twice settles, rather than re-reporting a change every time.
#[test]
fn an_applied_tombstone_settles() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    let id = payload(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "beta will be forgotten",
            "why": "so a tombstone exists",
            "confidence": 0.5,
            "origin": "llm"
        }),
    ))["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": &id}),
    );
    export_to(&mut server, &out);

    let report = payload(&import_from(&mut server, &out));
    assert_eq!(
        report["outcomes"]["unchanged"],
        json!(2),
        "a tombstone the store already has should be settled: {report}"
    );
}

/// A second model, so a store can be made to disagree with the server holding it.
#[derive(Debug)]
struct OtherEmbedder;

impl Embedder for OtherEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("other-embedder")
    }
    fn dimension(&self) -> usize {
        3
    }
    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|_| {
                Vector::new(vec![1.0, 0.0, 0.0])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

/// `status` has to name the model a set's vectors came from, not just whether search is possible.
///
/// Measured 2026-08-09 driving two real servers at one store: both reported byte-identical status
/// while holding different models, so nothing a caller could see distinguished the server whose
/// rankings were meaningful from the one comparing across two embedding spaces. A boolean cannot
/// carry that, because the question is not *can this search* but *whose vectors are these*.
#[test]
fn status_reports_which_model_a_sets_vectors_came_from() {
    let home = tempdir();
    let mut server = a_server(&home);

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha alpha",
            "confidence": 0.9,
            "origin": "llm",
        }),
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let local = &listed["sets"][0];

    assert_eq!(
        local["embedding_model"]["state"], "current",
        "a store embedded by the model in hand is current"
    );
    assert_eq!(
        local["embedding_model"]["recorded"], "marker-embedder",
        "the recorded identity is named, so two servers can be told apart"
    );
}

/// A store bound to `marker-embedder`, reopened by a server holding no model at all -- what a
/// `SCATTERHOARD_MODEL` pointing at a path that no longer exists produces.
fn a_server_missing_the_model_its_store_was_built_with(home: &TempDir) -> Server {
    let path = home.path().join("local.sqlite3");
    let mut owner = Server::new();
    owner.set_account_name(Author::new("jtcours").expect("valid author"));
    owner.set_embedder(Arc::new(MarkerEmbedder));
    owner.add_store(Store::open_at(Scope::Local, &path).expect("opens"));
    let stored = call(
        &mut owner,
        "hoard_memorize",
        json!({"scope": "local", "content": "alpha alpha", "confidence": 0.9, "origin": "llm"}),
    );
    assert_eq!(payload(&stored)["findable_by_meaning"], true, "{stored}");

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(Store::open_at(Scope::Local, &path).expect("reopens"));
    server
}

/// **A missing model is not a disagreement, and must not be described as one.**
/// `a-missing-model-reports-as-a-mismatch`: with no model loaded, status called the store's model a
/// mismatch and blamed *another server running with its own model*, when no second server existed.
/// The remedy it named, reconciling two configurations, is the wrong one; making the model loadable
/// again is the right one -- so the test takes that remedy and requires it to work.
#[test]
fn status_with_no_model_names_the_missing_one_and_blames_no_other_server() {
    let home = tempdir();
    let mut server = a_server_missing_the_model_its_store_was_built_with(&home);

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let model = &listed["sets"][0]["embedding_model"];
    assert_eq!(model["state"], "mismatched", "{model}");
    assert_eq!(model["loaded"], Value::Null, "{model}");
    let note = model["note"].as_str().expect("a note");
    assert!(
        note.contains("marker-embedder"),
        "the note names the model to restore: {note}"
    );
    assert!(
        note.contains("SCATTERHOARD_MODEL"),
        "and where a model is read from: {note}"
    );
    assert!(
        !note.contains("another server"),
        "and blames no second server: {note}"
    );

    server.set_embedder(Arc::new(MarkerEmbedder));
    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert_eq!(
        listed["sets"][0]["embedding_model"]["state"], "current",
        "loading the model the note names is the whole fix"
    );
}

/// The same state as a recall reports it, per set. It recommended `reembed`, which with no model
/// loaded is refused for having nothing to rebuild with -- a remedy that cannot be taken.
#[test]
fn recall_with_no_model_names_the_missing_one_rather_than_a_rebuild() {
    let home = tempdir();
    let mut server = a_server_missing_the_model_its_store_was_built_with(&home);

    let recalled = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["alpha"]}),
    ));
    let reason = recalled["skipped_sets"][0]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("the set is reported as skipped: {recalled}"));
    assert!(
        reason.contains("marker-embedder") && reason.contains("SCATTERHOARD_MODEL"),
        "the reason names the model to restore and where it is read from: {reason}"
    );
    assert!(
        !reason.contains("Decide which model should win"),
        "with one model in play there is nothing to decide between: {reason}"
    );

    server.set_embedder(Arc::new(MarkerEmbedder));
    let recalled = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "local"}),
    ));
    assert_eq!(
        recalled["semantic_search_available"], true,
        "loading the model the reason names makes the set searchable again: {recalled}"
    );
}

/// The case the two-server run produced: the store's model is not the one this server holds.
#[test]
fn status_names_both_models_when_they_disagree() {
    let home = tempdir();
    let mut server = a_server(&home);

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha alpha",
            "confidence": 0.9,
            "origin": "llm",
        }),
    );

    // The same store, now held by a server with a different model -- which is exactly what a second
    // checkout with its own SCATTERHOARD_MODEL is.
    server.set_embedder(Arc::new(OtherEmbedder));

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let local = &listed["sets"][0];

    assert_eq!(local["embedding_model"]["state"], "mismatched");
    assert_eq!(
        local["embedding_model"]["recorded"], "marker-embedder",
        "the model the stored vectors actually came from"
    );
    assert_eq!(
        local["embedding_model"]["loaded"], "other-embedder",
        "and the one this server would search with"
    );

    // The note has to describe the identity rather than the model, because since 2026-09-19 the
    // identity carries how text is encoded as well as which weights encoded it -- and the ordinary
    // case of a mismatch is now the same weights encoded two ways, where "a different model" sends
    // a reader looking for a model change that did not happen.
    let note = local["embedding_model"]["note"]
        .as_str()
        .expect("the mismatch carries a note");
    assert!(
        note.contains("encod"),
        "the note must say that how the text was encoded is part of what disagrees: got {note}"
    );
    assert!(
        note.contains("reembed"),
        "the note must name the way out, which is the same one the write door names: got {note}"
    );
}

/// The whole recovery path, driven the way a session would: refuse, name the remedy, run it, work.
///
/// Written end to end rather than as three unit tests because the failure it guards against is a
/// seam. A refusal naming an action that does not exist, or an action whose name the refusal spells
/// differently, is exactly the defect that would survive three passing unit tests -- and the
/// conventions catalog already records the cost of guidance pointing at an absent tool.
#[test]
fn a_mismatched_set_refuses_recall_names_the_remedy_and_recovers() {
    let home = tempdir();
    let mut server = a_server(&home);

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha alpha",
            "confidence": 0.9,
            "origin": "llm",
        }),
    );

    // A second server with its own model is what a second checkout is.
    server.set_embedder(Arc::new(OtherEmbedder));

    let recalled = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "local"}),
    ));
    assert_eq!(
        recalled["semantic_search_available"], false,
        "searching would compare across two vector spaces, so it does not happen"
    );
    let note = recalled["note"]
        .as_str()
        .expect("a note explains the refusal");
    assert!(
        note.contains("skipped_sets"),
        "the note has to say where the per-set reason is, got: {note}"
    );

    // **The remedy moved but did not weaken.** It used to sit in the note, which could hold only
    // one sentence for the whole union; it now sits on the set it applies to, because two sets can
    // be skipped for different reasons and want different remedies. What must stay true is that the
    // *response* names an action the caller can take.
    let skipped = recalled["skipped_sets"].as_array().expect("a list");
    assert_eq!(skipped.len(), 1, "one set was skipped: {skipped:?}");
    assert_eq!(skipped[0]["scope"], json!("local"), "named, not counted");
    let reason = skipped[0]["reason"].as_str().expect("a reason");
    assert!(
        reason.contains("reembed"),
        "the reason has to name the action that fixes it, got: {reason}"
    );
    assert!(
        reason.contains("other-embedder"),
        "and which two models disagree, without a second call: {reason}"
    );
    assert!(
        recalled["memories"].as_array().expect("a list").is_empty(),
        "nothing is returned rather than a confidently wrong ranking"
    );

    let rebuilt = payload(&sets(
        &mut server,
        json!({"action": "reembed", "scope": "local"}),
    ));
    assert_eq!(rebuilt["vectors_rebuilt"], 1);
    assert_eq!(rebuilt["model"], "other-embedder");
    assert_eq!(rebuilt["displaced"], "marker-embedder");
    assert!(
        rebuilt.get("warning").is_none(),
        "a first rebuild is the ordinary upgrade path and says nothing"
    );

    let recalled = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "local"}),
    ));
    assert_eq!(
        recalled["semantic_search_available"], true,
        "and semantic recall is back"
    );
}

/// Taking the set back warns, and still does the work.
#[test]
fn rebuilding_back_to_a_displaced_model_warns_but_proceeds() {
    let home = tempdir();
    let mut server = a_server(&home);

    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "alpha alpha",
            "confidence": 0.9,
            "origin": "llm",
        }),
    );

    server.set_embedder(Arc::new(OtherEmbedder));
    sets(&mut server, json!({"action": "reembed", "scope": "local"}));

    // And the first server takes its turn back.
    server.set_embedder(Arc::new(MarkerEmbedder));
    let rebuilt = payload(&sets(
        &mut server,
        json!({"action": "reembed", "scope": "local"}),
    ));

    let warning = rebuilt["warning"]
        .as_str()
        .expect("returning to a displaced model is worth saying out loud");
    assert!(
        warning.contains("Raise it with the human"),
        "the warning has to say what to do, not just report a count: {warning}"
    );
    assert_eq!(
        rebuilt["vectors_rebuilt"], 1,
        "and it is a warning rather than a refusal -- the rebuild happened"
    );
}

/// **`local` is exportable, and the schema must not tell a model otherwise.**
///
/// `local-scope-description-says-it-is-never-exported`, found 2026-08-11 by exporting `local` during
/// SETUP.md §5 item 15 to see whether it would be refused. It was not, and it should not have been:
/// `set-lifecycle.md` settles that export and backup are one operation, on the grounds that *"a
/// memory set that cannot be backed up is one bad disk away from being gone."*
///
/// So describing local as *never exported* told a model the backup route for personal memories does
/// not exist. The true statement is about publication — local is never shared to a repository, which
/// is a fact about the review gate rather than about writing a file.
///
/// Paired with the export it used to deny, so this stays a defect about a real capability rather
/// than a wording preference.
#[test]
fn the_local_scope_is_not_described_as_unexportable() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    let response = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );
    // Read from `scope`'s own description, which is where this now lives: the per-variant
    // descriptions went with the `oneOf` when `scope` became a scalar, since a nested alternation
    // does not survive every client (`tool-surface.md`). The claim being tested is unchanged.
    let described = response["result"]["tools"]
        .as_array()
        .expect("a list of tools")
        .iter()
        .find(|tool| tool["name"] == json!("hoard_sets"))
        .expect("hoard_sets is published")["inputSchema"]["properties"]["scope"]["description"]
        .as_str()
        .expect("a description")
        .to_owned();

    assert!(
        !described.to_lowercase().contains("never exported"),
        "local can be exported as a backup; the schema says it cannot: {described}"
    );

    store_one(&mut server, "alpha worth keeping");
    let exported = payload(&sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&out)}),
    ));
    assert!(
        exported["written"].is_string(),
        "exporting local should write a file: {exported}"
    );
}

// ---------------------------------------------------------------------------
// A missing argument is refused in the project's own voice
// ---------------------------------------------------------------------------

/// `a-missing-argument-refusal-is-a-raw-serde-message`.
///
/// The deserialiser's `missing field \`name\`` is honest and actionable, which is why this was filed
/// as `Costs` rather than worse. What it is not is *this project's* refusal: every other one names
/// the action and says what it takes. The branches already compose that sentence for the schema, so
/// there is a derivation to reuse rather than a message to write.
#[test]
fn a_missing_argument_is_refused_in_the_projects_own_voice() {
    let home = tempdir();
    let mut server = a_server(&home);

    let complaint = message(&sets(&mut server, json!({"action": "create"})));
    assert!(
        !complaint.contains("missing field"),
        "the deserialiser's own words should not reach the caller: {complaint}"
    );
    assert!(
        complaint.contains("create") && complaint.contains("name"),
        "the refusal should name the action and the argument it wants: {complaint}"
    );
}

/// The multi-argument case, which is where a hand-written message would drift from the schema.
#[test]
fn a_missing_argument_names_every_argument_its_action_takes() {
    let home = tempdir();
    let mut server = a_server(&home);

    let complaint = message(&sets(
        &mut server,
        json!({"action": "export", "scope": "local"}),
    ));
    assert!(
        complaint.contains("directory"),
        "the refusal should name the argument that did not arrive: {complaint}"
    );
    assert!(
        complaint.contains("scope"),
        "and the whole set the action takes, so one round trip fixes it: {complaint}"
    );
}

/// The same voice on `hoard_correct`, whose branches are a separate set — proving the derivation is
/// shared rather than written once per tool.
#[test]
fn hoard_correct_refuses_a_missing_argument_in_the_same_voice() {
    let home = tempdir();
    let mut server = a_server(&home);

    let response = call(&mut server, "hoard_correct", json!({"action": "forget"}));
    let complaint = message(&response);
    assert!(
        !complaint.contains("missing field"),
        "the deserialiser's own words should not reach the caller here either: {complaint}"
    );
    assert!(
        complaint.contains("forget") && complaint.contains("id"),
        "the refusal should name the action and what it takes: {complaint}"
    );

    // The clause that earns the pre-deserialiser check its place. serde stops at the first missing
    // field; the branch knows both, and only this half of the sentence can show the difference --
    // the `takes` list names every argument whether it arrived or not.
    let (_, absent) = complaint
        .split_once("did not include")
        .expect("the refusal should say what did not arrive: {complaint}");
    assert!(
        absent.contains("scope") && absent.contains("id"),
        "both missing arguments should be named, so one round trip fixes it: {complaint}"
    );
}

// ---------------------------------------------------------------------------
// One name, one meaning: `memories` is live memories everywhere
// ---------------------------------------------------------------------------

/// `memories-count-means-two-different-things`. `status` counted live memories and `export` counted
/// records written, and both called the number `memories` — so a set holding one tombstone reported
/// 32 and 33 for the same set moments apart, with nothing in either response saying which
/// measurement it was.
///
/// **The moment it surfaced was the worst available one**: the export gate, where a human is asked
/// to scrutinise what travels into a repository. An unexplained extra record there is exactly what
/// that review exists to catch, so the tooling spent the reviewer's attention on a discrepancy that
/// was not one. It cost this session two reconciliations before being fixed.
#[test]
fn status_reports_forgotten_even_when_there_is_nothing_forgotten() {
    let home = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "a memory nobody forgot");

    let result = payload(&sets(&mut server, json!({"action": "status"})));
    let local = local_set(&result);

    assert_eq!(local["memories"], json!(1));
    assert_eq!(
        local["forgotten"],
        json!(0),
        "absent would make a reader work out whether it means none or means not-reported, which is \
         the whole defect one layer down: {local}"
    );
}

#[test]
fn a_forgotten_memory_leaves_the_live_count_alone_and_shows_up_as_forgotten() {
    let home = tempdir();
    let mut server = a_server(&home);
    let id = store_local(&mut server, "a memory about to go");
    store_local(&mut server, "a memory that stays");

    call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": id}),
    );

    let result = payload(&sets(&mut server, json!({"action": "status"})));
    let local = local_set(&result);
    assert_eq!(local["memories"], json!(1), "`memories` is live memories");
    assert_eq!(local["forgotten"], json!(1), "and the tombstone is counted");
}

/// The half that produced the defect. `export` writes memories *and* tombstones, so what it wrote
/// is a third number — and it has to be reconcilable against `status` from a single response.
#[test]
fn export_reports_records_written_and_reconciles_with_the_live_count() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    let id = store_local(&mut server, "a memory about to go");
    store_local(&mut server, "a memory that stays");
    call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": id}),
    );

    let result = payload(&sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&out).to_string_lossy()}),
    ));

    assert_eq!(result["memories"], json!(1), "live memories, as everywhere");
    assert_eq!(result["forgotten"], json!(1), "tombstones, named as such");
    assert_eq!(
        result["records"],
        json!(3),
        "and what was actually written, under a name that says so: {result}"
    );

    let (memories, forgotten, records) = (
        result["memories"].as_u64().expect("a number"),
        result["forgotten"].as_u64().expect("a number"),
        result["records"].as_u64().expect("a number"),
    );
    let identities = result["identities"].as_u64().expect("a number");
    assert_eq!(
        records,
        memories + forgotten + identities,
        "the four must reconcile from one response, which is what the export gate needs"
    );
}

// ---------------------------------------------------------------------------
// What an export replaced
// ---------------------------------------------------------------------------

/// The `replaced_file` report from an export payload.
fn replaced(result: &Value) -> Value {
    let report = result["replaced_file"].clone();
    assert!(
        report.is_object(),
        "an export must say what it replaced: {result}"
    );
    report
}

/// The ids a category of the report lists.
fn ids_of(category: &Value) -> Vec<String> {
    category["ids"]
        .as_array()
        .unwrap_or_else(|| panic!("a category lists ids: {category}"))
        .iter()
        .map(|id| id.as_str().expect("an id").to_owned())
        .collect()
}

/// **Where nothing was there, everything is new, and the report says so rather than counting zero.**
/// The two are different answers, and a zero would read as *nothing changed*.
/// `export-reports-what-the-file-did-not-hold`.
#[test]
fn a_first_export_says_there_was_no_file_to_compare() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "a memory");

    let report = replaced(&payload(&export_to(&mut server, &out)));

    assert_eq!(report["state"], json!("none"), "{report}");
    assert!(
        report["note"]
            .as_str()
            .is_some_and(|note| note.contains("new")),
        "the report should say every record is new: {report}"
    );
    assert!(
        report.get("new").is_none(),
        "no counts where nothing was compared: {report}"
    );
}

#[test]
fn exporting_an_unchanged_set_again_reports_nothing_new_changed_or_removed() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "one");
    store_local(&mut server, "two");
    export_to(&mut server, &out);

    let result = payload(&export_to(&mut server, &out));
    let report = replaced(&result);

    assert_eq!(report["state"], json!("compared"), "{report}");
    for category in ["new", "changed", "removed"] {
        assert_eq!(report[category]["count"], json!(0), "{category}: {report}");
        assert!(ids_of(&report[category]).is_empty(), "{category}: {report}");
    }
    assert_eq!(
        report["unchanged"], result["records"],
        "every record written was already in the file: {result}"
    );
}

/// **The incident the entry was filed for**: another session's memory, written to the shared store
/// since this checkout last exported, rides into the commit. Here it announces itself.
#[test]
fn a_memory_written_since_the_last_export_is_reported_as_new() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "already exported");
    export_to(&mut server, &out);
    let id = store_local(&mut server, "written since");

    let report = replaced(&payload(&export_to(&mut server, &out)));

    assert_eq!(report["new"]["count"], json!(1), "{report}");
    assert_eq!(ids_of(&report["new"]), [id], "{report}");
    assert_eq!(report["changed"]["count"], json!(0), "{report}");
    assert_eq!(report["removed"]["count"], json!(0), "{report}");
}

/// A record whose content differs between the file and the set -- here edited in the file, as a pull
/// would -- is changed, not new.
#[test]
fn a_record_that_differs_from_the_file_is_reported_as_changed() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    let id = store_local(&mut server, "the original wording");
    export_to(&mut server, &out);
    edit_in_file(&out, "the original wording", "a wording from elsewhere");

    let report = replaced(&payload(&export_to(&mut server, &out)));

    assert_eq!(ids_of(&report["changed"]), [id], "{report}");
    assert_eq!(report["new"]["count"], json!(0), "{report}");
    assert_eq!(report["removed"]["count"], json!(0), "{report}");
}

/// Forgetting turns a memory's line into a tombstone under the same id, which is a change to that
/// record rather than a removal: the tombstone is what carries the forgetting to other copies.
#[test]
fn a_memory_forgotten_since_the_last_export_is_reported_as_changed() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    let id = store_local(&mut server, "about to be forgotten");
    store_local(&mut server, "staying");
    export_to(&mut server, &out);
    call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": id}),
    );

    let report = replaced(&payload(&export_to(&mut server, &out)));

    assert_eq!(ids_of(&report["changed"]), [id], "{report}");
    assert_eq!(report["removed"]["count"], json!(0), "{report}");
}

/// A file holding a record this server's local set does not, written by another server's export.
/// Returns the foreign record's id.
/// One shared set, `notes`, held on two machines: ours imported it from `out`, and since then the
/// other machine wrote a memory and exported again -- so `out` holds a record ours has not imported,
/// as after a pull. Returns that record's id and our server.
///
/// A shared set rather than two machines' `local` sets, which it used to be: those are two sets with
/// an identity each, and since `identity-mismatch-refusal` crossing between them is refused before
/// the behaviour these tests are about is reached.
fn a_file_from_elsewhere(out: &TempDir) -> (String, Server) {
    let mut elsewhere = a_notes_set_exported_to(out);
    let home = tempdir();
    let mut ours = a_server(&home);
    std::mem::forget(home);
    sets(&mut ours, json!({"action": "create", "name": "notes"}));
    let adopted = import_notes(&mut ours, out);
    assert!(!is_error(&adopted), "{adopted}");
    // The set arrived holding the other machine's author record, so this one says it is someone
    // else before it writes.
    sets(
        &mut ours,
        json!({"action": "mint", "scope": "shared:notes", "author": "ours"}),
    );
    let theirs = store_in(&mut elsewhere, "notes", "written on another machine");
    let exported = export_notes(&mut elsewhere, out);
    assert!(!is_error(&exported), "{exported}");
    (theirs, ours)
}

fn export_notes(server: &mut Server, out: &TempDir) -> Value {
    sets(
        server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    )
}

fn import_notes(server: &mut Server, out: &TempDir) -> Value {
    sets(
        server,
        json!({"action": "import", "scope": "shared:notes", "directory": out.path()}),
    )
}

/// **The case most likely to cost work, refused.** The file holds a record this set does not -- it
/// arrived by a pull, or by another machine's export to a synced file, and was never imported -- so
/// writing over the file would drop it. Nothing is written, and the refusal names the record and both
/// ways on. Decided 2026-09-14 by the human partner.
#[test]
fn an_export_that_would_drop_records_the_file_holds_is_refused() {
    let out = tempdir();
    let (theirs, mut server) = a_file_from_elsewhere(&out);
    let before = std::fs::read(memories_file(&out)).expect("reads");
    store_into(&mut server, "notes", "written here");
    let response = export_notes(&mut server, &out);

    assert!(
        is_error(&response),
        "an export that would drop records must be refused: {response}"
    );
    let refusal = message(&response);
    assert!(
        refusal.contains(&theirs),
        "the refusal should name the record it would drop: {refusal}"
    );
    assert!(
        refusal.contains("import"),
        "the refusal should say that importing first keeps the records: {refusal}"
    );
    assert!(
        refusal.contains("move"),
        "the refusal should say how to overwrite on purpose: {refusal}"
    );
    assert_eq!(
        std::fs::read(memories_file(&out)).expect("reads"),
        before,
        "a refused export leaves the file exactly as it was"
    );
}

/// The remedy the refusal names first: once the file's records are imported, the set holds them and
/// the export drops nothing.
#[test]
fn importing_first_lets_the_export_keep_the_records() {
    let out = tempdir();
    let (theirs, mut server) = a_file_from_elsewhere(&out);
    store_into(&mut server, "notes", "written here");
    let imported = import_notes(&mut server, &out);
    assert!(!is_error(&imported), "{}", message(&imported));

    let response = export_notes(&mut server, &out);

    assert!(!is_error(&response), "{}", message(&response));
    assert!(
        std::fs::read_to_string(memories_file(&out))
            .expect("reads")
            .contains(&theirs),
        "the imported record is still in the file"
    );
}

/// **The way through for an overwrite that is intended** -- after a `restore`, say. Moving the file
/// aside keeps the records it held, and the export then finds nothing at the destination.
#[test]
fn moving_the_file_aside_lets_an_intended_overwrite_through() {
    let out = tempdir();
    let (_, mut server) = a_file_from_elsewhere(&out);
    store_into(&mut server, "notes", "written here");
    assert!(is_error(&export_notes(&mut server, &out)), "refused first");

    std::fs::rename(memories_file(&out), out.path().join("set-aside.json")).expect("moves");
    let response = export_notes(&mut server, &out);

    assert!(!is_error(&response), "{}", message(&response));
    assert_eq!(
        replaced(&payload(&response))["state"],
        json!("none"),
        "{response}"
    );
}

/// **An unreadable file is refused too**, because what writing over it would lose cannot be told.
/// Decided 2026-09-14 by the human partner; only a file that is not there at all is written freely.
#[test]
fn an_export_over_an_unreadable_file_is_refused() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "a memory");
    let path = memories_file(&out);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("creates");
    std::fs::write(&path, "not a memory file\n").expect("writes");

    let response = export_to(&mut server, &out);

    assert!(
        is_error(&response),
        "an export over an unreadable file must be refused: {response}"
    );
    assert!(
        message(&response).contains("move"),
        "the refusal should say how to overwrite on purpose: {}",
        message(&response)
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("reads"),
        "not a memory file\n",
        "a refused export leaves the file exactly as it was"
    );
}

/// The unreadable file this is most likely to meet: an export left with unresolved merge conflict
/// markers. Writing over it would throw away the other side of the merge.
#[test]
fn an_export_over_a_file_with_merge_conflict_markers_is_refused() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "a memory");
    export_to(&mut server, &out);
    let path = memories_file(&out);
    let text = std::fs::read_to_string(&path).expect("reads");
    std::fs::write(
        &path,
        format!("<<<<<<< HEAD\n{text}=======\n>>>>>>> theirs\n"),
    )
    .expect("writes");

    let response = export_to(&mut server, &out);

    assert!(is_error(&response), "{response}");
    assert!(
        message(&response).contains("conflict"),
        "the refusal should name the likeliest cause: {}",
        message(&response)
    );
}

/// Ids are capped per category, because one format change can alter every record in a set and a
/// report of eight hundred ids buries the few that matter. The count stays exact.
#[test]
fn a_long_category_lists_capped_ids_and_an_exact_count() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "the first");
    export_to(&mut server, &out);
    for n in 0..30 {
        store_local(&mut server, &format!("written since, number {n}"));
    }

    let report = replaced(&payload(&export_to(&mut server, &out)));

    assert_eq!(report["new"]["count"], json!(30), "{report}");
    let listed = ids_of(&report["new"]).len();
    assert!(
        listed > 0 && listed < 30,
        "a long category lists some ids, not all: {report}"
    );
}

/// The explicit form compares against the file it names, the same as the directory form.
#[test]
fn an_export_to_an_explicit_file_compares_against_that_file() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    let file = out.path().join("backup.json");
    store_local(&mut server, "already exported");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    );
    let id = store_local(&mut server, "written since");

    let report = replaced(&payload(&sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    )));

    assert_eq!(report["state"], json!("compared"), "{report}");
    assert_eq!(ids_of(&report["new"]), [id], "{report}");
}

// ---------------------------------------------------------------------------
// The reserved directory is appended, not accepted
// ---------------------------------------------------------------------------

/// `export-appends-the-reserved-directory-to-its-own-argument`. `directory` means the *project
/// root*: export appends `scatterhoard_mem/memories.json` itself. Passing the memory directory
/// therefore wrote `scatterhoard_mem/scatterhoard_mem/memories.json` — a nested reserved directory
/// nothing will ever read — and reported success with the doubled path in `written`.
///
/// **Not a first offence.** The store's `bases` table already held both spellings of one target
/// before the session that found this, so an earlier session made the same mistake unnoticed. The
/// check that catches it is trivial and local: the argument already ends in the reserved name.
#[test]
fn export_refuses_a_directory_that_is_already_the_reserved_one() {
    let home = tempdir();
    let out = tempdir();
    let nested = out.path().join("scatterhoard_mem");
    std::fs::create_dir_all(&nested).expect("creates");
    let mut server = a_server_with_notes(&home, "a memory to export");

    let response = sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:notes",
            "directory": nested.to_string_lossy()
        }),
    );
    assert!(is_error(&response), "silently nesting is the defect");

    let complaint = message(&response);
    assert!(
        complaint.contains("scatterhoard_mem"),
        "the refusal must name what it recognised: {complaint}"
    );
    assert!(
        complaint.contains("parent") || complaint.contains("root"),
        "and name the action the caller can take: {complaint}"
    );

    // And nothing was written, least of all a nested reserved directory.
    assert!(
        !nested.join("scatterhoard_mem").exists(),
        "a refused export must not leave the thing it refused to make"
    );
}

/// `import` takes the same argument and appends the same path, so it has the same trap.
#[test]
fn import_refuses_a_directory_that_is_already_the_reserved_one() {
    let home = tempdir();
    let out = tempdir();
    let nested = out.path().join("scatterhoard_mem");
    std::fs::create_dir_all(&nested).expect("creates");
    let mut server = a_server_with_notes(&home, "a memory to import into");

    let response = sets(
        &mut server,
        json!({
            "action": "import", "scope": "shared:notes",
            "directory": nested.to_string_lossy()
        }),
    );
    assert!(is_error(&response));
    let complaint = message(&response);
    // Asserting only that the message mentions `scatterhoard_mem` passes vacuously here: import
    // fails anyway on the missing file, and *that* error quotes the path. The discriminating part
    // is the guidance, which only the guard produces.
    assert!(
        complaint.contains("parent") || complaint.contains("root"),
        "the same refusal, from the other direction, naming the action: {complaint}"
    );
    assert!(
        !complaint.contains("could not open"),
        "the guard must fire before the file is reached, or it is not a guard: {complaint}"
    );
}

/// A trailing separator is the same directory, and a caller who typed one meant the same thing.
#[test]
fn the_reserved_directory_is_recognised_through_a_trailing_separator() {
    let home = tempdir();
    let out = tempdir();
    let nested = out.path().join("scatterhoard_mem");
    std::fs::create_dir_all(&nested).expect("creates");
    let mut server = a_server_with_notes(&home, "a memory to export");

    let response = sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:notes",
            "directory": format!("{}/", nested.to_string_lossy())
        }),
    );
    assert!(
        is_error(&response),
        "a trailing separator must not walk past the check"
    );
}

// ---------------------------------------------------------------------------
// Export may name a file, not only a directory
//
// `export-may-name-a-file`. The fixed `scatterhoard_mem/memories.json` path exists for
// auto-discovery in a checkout -- *a file found here is a project set no matter what its header
// claims* -- and nothing auto-discovers a backup or a synced folder, where trust comes from the
// caller's stated scope and `check_scope` instead. What the reserved directory contributes there is
// friction: several unpublished sets in one synced folder become several wrappers whose leaf files
// are all named `memories.json`, indistinguishable in a listing, a search result, or a
// sync-conflict notification.
// ---------------------------------------------------------------------------

/// The explicit form writes exactly where it was told, and nowhere else.
#[test]
fn export_to_an_explicit_file_writes_exactly_there() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "a memory bound for a backup");

    let destination = out.path().join("coding-lessons-backup.json");
    let response = sets(
        &mut server,
        json!({
            "action": "export", "scope": "local",
            "file": destination.to_string_lossy()
        }),
    );
    assert!(!is_error(&response), "{}", message(&response));

    assert!(
        destination.is_file(),
        "the file named by the caller should hold the set"
    );
    // The discriminating half: the reserved directory is not created *anywhere* near it, because
    // the whole point of this form is that nothing will auto-discover the result.
    assert!(
        !out.path().join("scatterhoard_mem").exists(),
        "the explicit form must not append the reserved directory it exists to avoid"
    );

    let file = std::fs::File::open(&destination).expect("opens");
    let set = memory_file::read(&Expectation::Exactly(Scope::Local), file).expect("reads back");
    assert_eq!(set.memories.len(), 2);

    let written = payload(&response)["written"]
        .as_str()
        .expect("export says where it wrote")
        .to_owned();
    assert_eq!(
        std::path::Path::new(&written),
        destination,
        "`written` should be the path the caller named, not a derived one"
    );
}

/// The directory form is unchanged, and stays the default. This is the regression guard on the
/// clause the backlog entry insists on: `export <checkout-root>` is exactly what it was.
#[test]
fn the_directory_form_still_appends_the_reserved_path() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_with_notes(&home, "a memory bound for a checkout");

    let response = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    );
    assert!(!is_error(&response), "{}", message(&response));
    assert!(
        memories_file(&out).is_file(),
        "the directory form appends the well-known path"
    );
}

/// **The response says which form was used.** Two callers can name the same bytes on disk by
/// different arguments, and a transcript that cannot tell them apart cannot show that a set was
/// written somewhere nothing will ever discover it.
#[test]
fn export_says_which_form_it_used() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_with_notes(&home, "a memory to export twice");

    let by_directory = payload(&sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    ));
    assert_eq!(by_directory["path_form"], json!("directory"));

    let by_file = payload(&sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:notes",
            "file": out.path().join("elsewhere.json").to_string_lossy()
        }),
    ));
    assert_eq!(by_file["path_form"], json!("file"));
}

/// Naming both is ambiguous about where the set lands, and ambiguity about a destination is the one
/// thing not to guess at. Refused rather than resolved by precedence: a precedence rule is invisible
/// at the call site and would silently ignore half of what the caller asked for.
#[test]
fn export_refuses_both_a_directory_and_a_file() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_with_notes(&home, "a memory with two destinations");

    let destination = out.path().join("ambiguous.json");
    let response = sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:notes",
            "directory": out.path(), "file": destination.to_string_lossy()
        }),
    );
    assert!(is_error(&response), "naming two destinations is not a call");

    let complaint = message(&response);
    assert!(
        complaint.contains("directory") && complaint.contains("file"),
        "the refusal should name both of what it was given: {complaint}"
    );

    // Neither destination was written, so a refused export leaves nothing to clean up and nothing
    // to mistake for a successful one.
    assert!(!destination.exists(), "the file form was not written");
    assert!(
        !out.path().join("scatterhoard_mem").exists(),
        "nor the directory form"
    );
}

/// Naming neither names no destination at all, and the refusal has to teach both forms -- a caller
/// told only about `directory` would never discover the one that suits a backup.
#[test]
fn export_with_no_destination_names_both_forms() {
    let home = tempdir();
    let mut server = a_server(&home);

    let complaint = message(&sets(
        &mut server,
        json!({"action": "export", "scope": "local"}),
    ));
    assert!(
        complaint.contains("directory") && complaint.contains("file"),
        "the refusal should name both forms, not just the older one: {complaint}"
    );
    assert!(
        complaint.contains("scope"),
        "and the whole set the action takes, so one round trip fixes it: {complaint}"
    );
}

/// A `null` is not a destination, and the flat schema is why this has to be true.
///
/// `with_flat_view` publishes every action's arguments at the top level, so a client filling in the
/// whole published shape sends the properties its action does not use as `null`. Reading that as
/// *supplied* would refuse an ordinary directory export for naming two destinations.
#[test]
fn a_null_destination_is_not_a_second_destination() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_with_notes(&home, "a memory from a thorough client");

    let response = sets(
        &mut server,
        json!({
            "action": "export", "scope": "shared:notes",
            "directory": out.path(), "file": Value::Null
        }),
    );
    assert!(!is_error(&response), "{}", message(&response));
    assert!(
        memories_file(&out).is_file(),
        "the directory form still ran"
    );
}

/// The reserved namespace cannot be hand-populated behind the unexpected-contents warning.
///
/// This is the constraint the design attaches to the explicit form. `export` and `import` append
/// `scatterhoard_mem/memories.json` and [Trigger and Loading] treats whatever it finds there as a
/// project set before reading a byte of it; the export gate's unexpected-contents warning is what
/// tells a human that something else is sitting in there. An explicit file path aimed *into* the
/// reserved directory would place a file in that namespace by a route the warning was not written
/// to cover, so the parent is checked exactly as the directory form's argument is.
#[test]
fn an_explicit_file_may_not_be_placed_inside_the_reserved_directory() {
    let home = tempdir();
    let out = tempdir();
    let reserved = out.path().join("scatterhoard_mem");
    std::fs::create_dir_all(&reserved).expect("creates");
    let mut server = a_server(&home);
    store_local(&mut server, "a memory aimed at the reserved namespace");

    let destination = reserved.join("something-else.json");
    let response = sets(
        &mut server,
        json!({
            "action": "export", "scope": "local",
            "file": destination.to_string_lossy()
        }),
    );
    assert!(
        is_error(&response),
        "the explicit form must not reach into the reserved directory"
    );

    let complaint = message(&response);
    assert!(
        complaint.contains("scatterhoard_mem"),
        "the refusal must name what it recognised: {complaint}"
    );
    assert!(
        !destination.exists(),
        "a refused export must not leave the file it refused to write"
    );
}

/// Import reads an explicit file, so a set written to one can be read back by the tool that wrote it.
#[test]
fn import_from_an_explicit_file_reads_exactly_there() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha arrived by an explicit path");

    let file = out.path().join("a-backup.json");
    let written = sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    );
    assert!(!is_error(&written), "{}", message(&written));

    let response = sets(
        &mut server,
        json!({"action": "import", "scope": "local", "file": file.to_string_lossy()}),
    );
    assert!(!is_error(&response), "{}", message(&response));

    let report = payload(&response);
    assert_eq!(
        report["read"].as_str().map(std::path::Path::new),
        Some(file.as_path()),
        "import should read the path the caller named: {report}"
    );
    assert_eq!(report["path_form"], json!("file"));
}

/// **The round trip settles**, which is the whole of why import had to gain this form too.
///
/// `unchanged` rather than `converged` is the discriminating assertion. Identical content with no
/// recorded base reconciles as `converged`; `unchanged` is only reachable when the import found the
/// base the *export* wrote, so it fails exactly when the two forms disagree about what the target
/// is. An export you cannot import from is the trap this pair exists to close, and a file that
/// reconciles against no ancestor is the quieter half of it.
#[test]
fn an_export_and_import_through_one_explicit_file_settle() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is settled");

    let file = out.path().join("round-trip.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file.to_string_lossy()}),
    );
    let report = payload(&sets(
        &mut server,
        json!({"action": "import", "scope": "local", "file": file.to_string_lossy()}),
    ));

    assert_eq!(
        report["outcomes"]["unchanged"],
        json!(2),
        "the import must find the agreement the export recorded: {report}"
    );
    assert!(report.get("conflicts").is_none(), "{report}");
    assert!(report.get("anomalies").is_none(), "{report}");
}

/// The two forms are two targets, and naming one file both ways would give it two agreements.
///
/// This is why the reserved-directory refusal binds import at least as hard as export. Reading
/// `<root>/scatterhoard_mem/memories.json` through `file` is a legitimate-looking spelling of what
/// `directory` already does -- and it would reconcile against a *different* base than the directory
/// form's, so the same file would be settled by one spelling and diverged by the other.
#[test]
fn an_explicit_file_may_not_be_read_from_inside_the_reserved_directory() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_with_notes(&home, "alpha lives at the well-known path");

    sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    );
    let well_known = memories_file(&out);
    assert!(well_known.is_file(), "the fixture needs the file to exist");

    let response = sets(
        &mut server,
        json!({
            "action": "import", "scope": "shared:notes",
            "file": well_known.to_string_lossy()
        }),
    );
    assert!(
        is_error(&response),
        "naming the well-known file explicitly would be a second target for one file"
    );

    let complaint = message(&response);
    assert!(
        complaint.contains("scatterhoard_mem"),
        "the refusal must name what it recognised: {complaint}"
    );
    assert!(
        complaint.contains("directory"),
        "and name the argument that already reaches this file: {complaint}"
    );
}

/// The alternation is one derivation, so import inherits every refusal export has.
#[test]
fn import_refuses_both_a_directory_and_a_file() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);

    let response = sets(
        &mut server,
        json!({
            "action": "import", "scope": "local",
            "file": local_backup(&out), "file": out.path().join("x.json").to_string_lossy()
        }),
    );
    assert!(is_error(&response));

    let complaint = message(&response);
    assert!(
        complaint.contains("directory") && complaint.contains("file"),
        "the refusal should name both of what it was given: {complaint}"
    );
}

/// And import's own missing-argument refusal teaches both forms, not just the older one.
#[test]
fn import_with_no_source_names_both_forms() {
    let home = tempdir();
    let mut server = a_server(&home);

    let complaint = message(&sets(
        &mut server,
        json!({"action": "import", "scope": "local"}),
    ));
    assert!(
        complaint.contains("directory") && complaint.contains("file"),
        "the refusal should name both forms: {complaint}"
    );
}

/// A path that is already a directory cannot be a file, and the refusal says so in this project's
/// voice rather than letting the operating system's `Is a directory` reach the caller.
#[test]
fn an_explicit_file_that_is_a_directory_is_refused_in_the_projects_voice() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "a memory aimed at a directory");

    let response = sets(
        &mut server,
        json!({
            "action": "export", "scope": "local",
            "file": out.path().to_string_lossy()
        }),
    );
    assert!(is_error(&response));

    let complaint = message(&response);
    assert!(
        complaint.contains("directory"),
        "the refusal should say what the path already is, and name the argument that takes one: \
         {complaint}"
    );
    // Without this the test passes vacuously: the missing-argument refusal also says "directory",
    // so the assertion above alone would hold even if the guard did not exist.
    assert!(
        !complaint.contains("did not include"),
        "a destination was given, so this is not a missing argument: {complaint}"
    );
}

/// An explicit file is its own replica, so it is its own reconciliation target.
///
/// The directory form keys bases on the directory as typed; the explicit form must key on the file,
/// because two sets backed up into one synced folder are two replicas and keying on their shared
/// parent would merge them. Observed through the unrecognised-target remark, which is the one place
/// the target list is visible from outside.
#[test]
fn an_explicit_file_is_its_own_reconciliation_target() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "a memory backed up twice");

    let first = out.path().join("first.json");
    let second = out.path().join("second.json");

    let opening = payload(&sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": first.to_string_lossy()}),
    ));
    assert!(
        opening.get("note").is_none(),
        "the first export of a set has no earlier target to remark on: {opening}"
    );

    let again = payload(&sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": first.to_string_lossy()}),
    ));
    assert!(
        again.get("note").is_none(),
        "the same file is the same target: {again}"
    );

    let sibling = payload(&sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": second.to_string_lossy()}),
    ));
    let note = sibling["note"].as_str().expect(
        "a sibling file in the same folder is a different replica, and must be remarked on",
    );
    assert!(
        note.contains("first.json"),
        "the remark should name the target this set does know: {note}"
    );
}

// ---------------------------------------------------------------------------
// A supersession cycle arriving in a file.
//
// `two-import-paths-disagree-about-supersession-cycles` in TRACKING.md. A cycle can arrive from a
// hand-edited merge, and refusing the whole file would make one bad edge cost every memory in the
// set -- so it is stored and reported, which is what resolution's `supersession_cycle` state exists
// to describe. Refusing stays correct for `hoard_memorize`, where a caller is authoring one edge.
// ---------------------------------------------------------------------------

/// Rewrite the exported file so its two memories supersede each other.
///
/// Written by hand because no tool will produce it: that is the point of the case.
fn make_the_two_records_supersede_each_other(out: &TempDir) -> (String, String) {
    let path = memories_file(out);
    let text = std::fs::read_to_string(&path).expect("reads");
    let mut lines: Vec<String> = text.lines().map(ToOwned::to_owned).collect();

    // The set's identity record is a line of the file too. Pairing it into the cycle would be
    // testing something else, so it is filtered rather than counted around -- which also keeps this
    // holding if the number of reserved records ever changes.
    let is_identity = |line: &str| {
        serde_json::from_str::<Value>(line).expect("a record")["keywords"]
            .as_array()
            .is_some_and(|keywords| keywords.iter().any(|keyword| keyword == "hoard-author"))
    };

    let content: Vec<usize> = (1..lines.len())
        .filter(|&index| !is_identity(&lines[index]))
        .collect();
    let ids: Vec<String> = content
        .iter()
        .map(|&index| {
            serde_json::from_str::<Value>(&lines[index]).expect("a record")["id"]
                .as_str()
                .expect("an id")
                .to_owned()
        })
        .collect();
    assert_eq!(ids.len(), 2, "this fixture wants exactly two records");

    for (offset, &index) in content.iter().enumerate() {
        let mut record: Value = serde_json::from_str(&lines[index]).expect("a record");
        let other = &ids[1 - offset];
        record["links"] = json!([{"label": "supersedes", "target": other}]);
        lines[index] = record.to_string();
    }

    std::fs::write(&path, lines.join("\n") + "\n").expect("writes");
    (ids[0].clone(), ids[1].clone())
}

#[test]
fn a_cycle_in_the_file_is_imported_rather_than_refused() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha as one side has it");
    store_one(&mut server, "alpha as the other side has it");
    export_to(&mut server, &out);
    let (first, second) = make_the_two_records_supersede_each_other(&out);

    // A fresh store, so both records are new to it and neither has an ancestor to judge against.
    let elsewhere = tempdir();
    let mut receiving = a_server(&elsewhere);
    let response = import_from(&mut receiving, &out);

    assert!(
        !is_error(&response),
        "one bad edge must not cost the whole file: {}",
        message(&response)
    );
    let report = payload(&response);
    assert_eq!(
        report["outcomes"]["new_from_file"],
        json!(3),
        "both records, and the set's identity record, should have been written: {report}"
    );

    for id in [&first, &second] {
        let found = call(&mut receiving, "hoard_lookup", json!({"id": id}));
        assert!(!is_error(&found), "{id} should be in the store: {found}");
    }
}

#[test]
fn a_cycle_in_the_file_is_reported_to_the_caller() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha as one side has it");
    store_one(&mut server, "alpha as the other side has it");
    export_to(&mut server, &out);
    let (first, second) = make_the_two_records_supersede_each_other(&out);

    let elsewhere = tempdir();
    let mut receiving = a_server(&elsewhere);
    let report = payload(&import_from(&mut receiving, &out));

    let cycles = report["supersession_cycles"].as_array().unwrap_or_else(|| {
        panic!("a stored cycle nobody is told about is the silent half: {report}")
    });
    let named: Vec<&str> = cycles.iter().filter_map(Value::as_str).collect();
    assert!(
        named.contains(&first.as_str()) && named.contains(&second.as_str()),
        "both ends belong in the report, since neither is the one at fault: {report}"
    );
}

// ---------------------------------------------------------------------------
// Prohibited data at export: warned about, loudly, and never blocking.
//
// The write refuses (tests/mcp.rs); the export does not. A set may hold memories that arrived by
// import and that the caller cannot properly fix -- and yesterday's finding is that the exporter
// cannot even tell which those are. The scan also gains rules over time, so blocking here would let
// a filter improvement close a set that exported fine the day before, taking every memory written
// since with it. See documents/design/export-review.md.
// ---------------------------------------------------------------------------

/// Fabricated. A real prefix, a body that is not a real key.
const A_FAKE_KEY_IN_A_SET: &str = "AKIAIOSFODNN7EXAMPLE";

/// Put a memory carrying a credential into the store behind the server's back.
///
/// Behind its back on purpose: `hoard_memorize` refuses this now, so the only ways a set acquires
/// one are an import, a hand-edited file, or a store written before the scan existed -- which are
/// exactly the cases the export-side warning is for.
fn a_server_holding_a_credential(home: &TempDir) -> Server {
    let mut store = Store::open_in_memory(Scope::Local).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    let memory = Memory::new(
        MemoryText::new(format!("the deploy key is {A_FAKE_KEY_IN_A_SET}")).expect("valid"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: "2026-08-02T09:00:00Z".parse().expect("valid timestamp"),
        },
    );
    store.insert(&memory).expect("inserts");

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(store);
    server
}

#[test]
fn an_export_carrying_a_credential_still_writes_the_file() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_holding_a_credential(&home);

    let response = export_to(&mut server, &out);

    assert!(
        !is_error(&response),
        "blocking here would let a filter change close a set retroactively: {}",
        message(&response)
    );
    assert!(
        memories_file(&out).exists(),
        "the file should have been written"
    );
}

#[test]
fn an_export_carrying_a_credential_says_so_loudly() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server_holding_a_credential(&home);

    let report = payload(&export_to(&mut server, &out));

    assert_eq!(
        report["prohibited_data_scanned"],
        json!(true),
        "the scan ran, so the flag must say so: {report}"
    );
    let found = report["prohibited_data_found"]
        .as_array()
        .unwrap_or_else(|| panic!("a warning nobody can act on is not a warning: {report}"));
    assert_eq!(found.len(), 1, "{report}");
    assert_eq!(found[0]["category"], json!("api_key"));
    assert!(
        found[0]["memory"].as_str().is_some(),
        "naming the memory is what makes it actionable: {report}"
    );
    assert!(
        !report.to_string().contains(A_FAKE_KEY_IN_A_SET),
        "the report must not repeat the credential: {report}"
    );
}

#[test]
fn a_clean_export_reports_the_scan_as_having_run() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha, carrying nothing of the kind");

    let report = payload(&export_to(&mut server, &out));

    assert_eq!(report["prohibited_data_scanned"], json!(true));
    assert!(report.get("prohibited_data_found").is_none(), "{report}");
}

/// **What retiring the per-target base would cost, measured rather than predicted.**
///
/// [The graph is the ancestor](documents/design/multi-set-sync.md) proposes dropping the base on the
/// ground that an exported record is immutable, so a record both sides hold can never differ. The
/// premise holds through the *tool* surface — `absorb-as-supersession` closed the last route on
/// 2026-08-31 — but not through **export-edit-import**, which [Store Registry](documents/design/store-registry.md)
/// keeps deliberately as the only way to re-attribute memories in bulk, precisely because it puts a
/// human at a judgement software cannot make.
///
/// This drives that procedure into a store that has **no base for the target it is reading**, which
/// is the exact state retirement would put every import into, and shows what the reconciler then
/// says: `resolve(file, None, Some(stored))` cannot tell *they changed it* from *we changed it*, so
/// a one-sided edit that `a_change_only_in_the_file_is_taken` resolves as `took_from_file` becomes a
/// **conflict**. Nothing is written, and the caller is asked to settle a disagreement that does not
/// exist.
///
/// Asserted as current behaviour, so that if the base is retired this test says what it cost.
#[test]
fn without_a_base_an_ordinary_edit_is_reported_as_a_conflict() {
    let home = tempdir();
    let agreed = tempdir();
    let elsewhere = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha as first written");

    // A target this store has agreed with: the base is recorded here.
    export_to(&mut server, &agreed);

    // The same bytes reached by a path this store has never agreed with -- one directory, one file,
    // no recorded agreement. A second checkout, a synced folder, or every import once the base is
    // gone.
    std::fs::create_dir_all(elsewhere.path().join("scatterhoard_mem")).expect("creates");
    std::fs::copy(memories_file(&agreed), memories_file(&elsewhere)).expect("copies");
    edit_in_file(
        &elsewhere,
        "alpha as first written",
        "alpha as someone else revised it",
    );

    let report = payload(&import_from(&mut server, &elsewhere));
    assert_eq!(
        report["outcomes"]["conflict"],
        json!(1),
        "with no ancestor, a one-sided edit is indistinguishable from a disagreement: {report}"
    );

    // And the edit did not land, which is the half that matters: the procedure silently stops
    // working rather than becoming noisy.
    let found = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 10}),
    ));
    let contents: Vec<&str> = found["memories"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|memory| memory["content"].as_str().expect("content"))
        .collect();
    assert!(
        contents.contains(&"alpha as first written"),
        "nothing is written for a conflicted record, so the store keeps its version: {contents:?}"
    );
}

/// **The case the old guard could not see, and the reason it was replaced.**
///
/// Until 2026-08-31 the rule was *the copy may not land on a path this set has been exported to*,
/// answered from the per-target base's target list. It could only ever see paths **this store had
/// written itself** — so a colleague's backup, an unrelated file, or a copy someone made by hand
/// were all invisible to it, and a rescue would have silently destroyed any of them. The harm is
/// destroying a file, not destroying a file of a particular provenance.
#[test]
fn a_safety_copy_will_not_overwrite_a_file_this_set_never_wrote() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is in the set");
    sets(&mut server, json!({"action": "create", "name": "doomed"}));

    // Nothing to do with Scatterhoard, and nothing this set has ever been exported to.
    let someone_elses = out.path().join("thesis-final-v3.txt");
    std::fs::write(&someone_elses, "years of work").expect("writes");

    let response = sets(
        &mut server,
        json!({
            "action": "drop", "name": "doomed",
            "save_to": someone_elses.to_string_lossy(),
        }),
    );
    assert!(is_error(&response), "an occupied path must be refused");
    assert_eq!(
        std::fs::read_to_string(&someone_elses).expect("still there"),
        "years of work",
        "the file must be untouched"
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let names: Vec<&str> = listed["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .filter_map(|set| set["scope"].as_str())
        .collect();
    assert!(
        names.contains(&"shared:doomed"),
        "and nothing was dropped: {names:?}"
    );
}

/// `absorb` writes a safety copy like the two destructive operations do, so it answers to the same
/// guard. Asserted separately because a shared helper's likeliest failure is one caller losing the
/// call while the others keep it — which is exactly what a surviving mutation here reported.
#[test]
fn absorb_will_not_overwrite_a_file_with_its_safety_copy() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);
    let home = tempdir();
    let (mut server, main_id) = a_main_linking_into_other(&home, &other_id);

    let occupied = out.path().join("already-here.json");
    std::fs::write(&occupied, "something that matters").expect("writes");

    let refused = absorb_into_main(&mut server, &file, &occupied);
    assert!(
        is_error(&refused),
        "an occupied path must be refused: {refused}"
    );
    assert_eq!(
        std::fs::read_to_string(&occupied).expect("still there"),
        "something that matters",
        "the file must be untouched"
    );
    assert!(
        look_up_in(&mut server, &main_id)["superseded_by"].is_null(),
        "and nothing was absorbed"
    );
}

/// **Backup-edit-restore does not avoid the conflict; it guarantees one.**
///
/// Export-edit-import propagates a bulk edit cleanly, because the import is against the *same*
/// target whose base it then advances — `a_change_only_in_the_file_is_taken`. Routing the same edit
/// through a backup and a restore breaks that, and for a reason `restore` states plainly:
/// `replace_with` clears **every** base, since a base for a record the restored set no longer holds
/// would resolve as an anomaly. Restore then records an agreement with the file it restored *from*
/// — the backup — and with nothing else.
///
/// So the store comes back holding edited records with **no ancestor relative to origin**, and the
/// next ordinary pull compares two versions with nothing to judge them by. Measured here rather
/// than argued: the edited record conflicts, and the unedited ones converge.
#[test]
fn a_backup_edit_restore_leaves_the_next_pull_with_no_ancestor() {
    let home = tempdir();
    let origin = tempdir();
    let backup = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha attributed to a name I no longer use");
    store_one(&mut server, "beta which nobody is editing");

    // The set as everyone else has it.
    export_to(&mut server, &origin);

    // The escape hatch, routed through a backup rather than through the export.
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": local_backup(&backup)}),
    );
    edit_in_file(
        &backup,
        "alpha attributed to a name I no longer use",
        "alpha attributed to the name I publish under",
    );
    let safety = tempdir();
    let restored = sets(
        &mut server,
        json!({
            "action": "restore", "scope": "local",
            "file": local_backup(&backup),
            "save_to": safety.path().join("before.json"),
        }),
    );
    assert!(!is_error(&restored), "{}", message(&restored));

    // The next ordinary pull from origin, which still carries the un-edited version.
    let report = payload(&import_from(&mut server, &origin));
    assert_eq!(
        report["outcomes"]["conflict"],
        json!(1),
        "the re-attributed record has no ancestor relative to origin: {report}"
    );
    assert_eq!(
        report["outcomes"]["converged"],
        json!(2),
        "the record nobody edited is identical on both sides, so it agrees: {report}"
    );
}

/// `absorb` names the collision too, and for `restore`'s reason: the occupied-destination guard
/// would refuse this call anyway — the file being absorbed plainly exists — so what this check adds
/// is **which** file it is. Asserted on the wording, because a test that only checked the refusal
/// would pass with the check deleted.
#[test]
fn absorb_refuses_to_save_over_the_file_it_is_absorbing() {
    let author_home = tempdir();
    let out = tempdir();
    let (other_id, file) = a_file_from_other(&author_home, &out);
    let before = std::fs::read_to_string(&file).expect("the file exists");
    let home = tempdir();
    let (mut server, main_id) = a_main_linking_into_other(&home, &other_id);

    let refused = absorb_into_main(&mut server, &file, &file);
    assert!(is_error(&refused), "saving over the source: {refused}");
    let complaint = message(&refused);
    assert!(
        complaint.contains("which is the file being absorbed"),
        "the refusal must name what the path collided with: {complaint}"
    );

    assert_eq!(
        std::fs::read_to_string(&file).expect("still there"),
        before,
        "the file being absorbed must be untouched"
    );
    assert!(
        look_up_in(&mut server, &main_id)["superseded_by"].is_null(),
        "and nothing was absorbed"
    );
}

// ---------------------------------------------------------------------------
// The durable author map: re-adding a set does not re-ask who you are
// ---------------------------------------------------------------------------

/// **The case the map exists for.** A set can lose the `meta` cache naming which identity record is
/// this machine's user — a `drop` destroys it with the database, and a set adopted on a second
/// checkout never had one. Writing again must not meet a question already answered, and must not
/// answer it by guessing, since the set holds an identity record and matching a name against it is
/// the adoption this design removed.
///
/// **The cache is cleared directly here, and that is now a narrowing rather than a workaround.** The
/// literal drop-and-re-add is covered by
/// `re_adding_a_dropped_set_recovers_its_identity_and_does_not_re_ask`, which could not be written
/// until a set could take back the identity in its own file. This one keeps the *other* route to the
/// same state — a cache lost without the set's identity changing, which is what a second checkout
/// adopting a set looks like — so the map is exercised on its own rather than only through the
/// identity recovery that now precedes it.
#[test]
fn a_set_that_lost_its_author_cache_resolves_from_the_map_without_asking() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jtcours");

    create_chartered(&mut server, "revenant");
    let first = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "revenant"}, "content": "alpha before the cache went",
            "confidence": 0.8, "origin": "llm", "why": "so the set has an identity record"
        }),
    );
    assert!(!is_error(&first), "{}", message(&first));

    forget_which_identity_is_yours(&home, "revenant");

    let second = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "revenant"}, "content": "beta after it went",
            "confidence": 0.8, "origin": "llm", "why": "so the write has to resolve an author"
        }),
    );
    assert!(
        !is_error(&second),
        "the durable map should have answered without asking: {}",
        message(&second)
    );

    // And under the identity already there rather than a second one: two identity records for one
    // person is the hazard nothing downstream can report.
    let identities = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["hoard-author"], "scope": {"shared": "revenant"}}),
    ));
    assert_eq!(
        identities["memories"].as_array().map(Vec::len),
        Some(1),
        "one person, one identity record: {identities}"
    );
}

/// Clear a set's cached answer to *which identity record is you*, leaving the record in place.
/// Stands in for a `drop`, which destroys the database and takes the cache with it.
fn forget_which_identity_is_yours(home: &TempDir, name: &str) {
    let path = home.path().join(format!("shared-{name}.sqlite3"));
    let connection = rusqlite::Connection::open(&path).expect("opens the store");
    connection
        .execute("DELETE FROM meta WHERE key = 'author_record'", [])
        .expect("clears the cached identity");
}

/// **A row is checked against the set, not trusted.** It can name a record this copy does not hold,
/// and adopting it would point every write at nothing — the one failure holding `by` as a reference
/// introduces. A row that does not resolve has to read as no row at all.
///
/// The scenario is ordinary: restore a set from a file taken before its identity record existed.
/// The set loses the record; the machine's map still names it.
#[test]
fn an_author_map_row_naming_a_record_the_set_lacks_is_not_adopted() {
    let home = tempdir();
    let before = tempdir();
    let mut server = a_server_as(&home, "jtcours");

    sets(&mut server, json!({"action": "create", "name": "revenant"}));
    // Exported while the set is empty — `create` records a name and mints no record, so this file
    // predates the identity entirely. Not chartered first: the charter is a write, and would put the
    // author record into this file, which is exactly the state the test needs the file not to hold.
    sets(
        &mut server,
        json!({
            "action": "export", "scope": {"shared": "revenant"}, "directory": before.path()
        }),
    );

    name_author(&mut server, "revenant", FIXTURE_AUTHOR);
    let first = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "revenant"}, "content": FIXTURE_CHARTER,
            "confidence": 0.8, "origin": "llm", "why": "so a record exists and the map names it",
            "keywords": ["hoard-scope-topics", "hoard-lang-en"]
        }),
    );
    assert!(!is_error(&first), "{}", message(&first));

    let restored = sets(
        &mut server,
        json!({
            "action": "restore", "scope": {"shared": "revenant"},
            "directory": before.path(),
            "save_to": home.path().join("before-restore.json").to_string_lossy()
        }),
    );
    assert!(!is_error(&restored), "{}", message(&restored));
    forget_which_identity_is_yours(&home, "revenant");

    let second = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "revenant"}, "content": FIXTURE_CHARTER,
            "confidence": 0.8, "origin": "llm", "why": "so the write has to resolve an author",
            // The restore emptied the set, so its first memory is again its charter.
            "keywords": ["hoard-scope-topics", "hoard-lang-en"]
        }),
    );

    // The set holds no identity records now, and a shared set is not given one by default -- so the
    // write is refused until someone says who they are. What must *not* happen is adopting the stale
    // row, which would let the write through attributed to a record no longer in the set.
    assert!(is_error(&second), "{second}");
    assert!(
        message(&second).contains("\"mint\""),
        "{}",
        message(&second)
    );

    name_author(&mut server, "revenant", FIXTURE_AUTHOR);
    let third = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "revenant"}, "content": FIXTURE_CHARTER,
            "confidence": 0.8, "origin": "llm", "why": "written once someone is named",
            "keywords": ["hoard-scope-topics", "hoard-lang-en"]
        }),
    );
    assert!(!is_error(&third), "{}", message(&third));
    let identities = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["hoard-author"], "scope": {"shared": "revenant"}}),
    ));
    let records: Vec<&str> = identities["memories"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|memory| memory["id"].as_str())
        .collect();
    assert_eq!(records.len(), 1, "one identity record: {identities}");
    let attributed = look_up_in(&mut server, records[0]);
    assert_eq!(
        attributed["id"], records[0],
        "the identity the writes point at must be one the set actually holds: {identities}"
    );
}

// ---------------------------------------------------------------------------
// `author` splits into `mint`, `identify` and `rename`
// ---------------------------------------------------------------------------

/// Store one memory in a fresh shared set, returning the server and the memory's id.
fn a_set_with_one_memory(home: &TempDir, name: &str) -> (Server, String) {
    let mut server = a_server_as(home, "jtcours");
    create_chartered(&mut server, name);
    let stored = payload(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": name}, "content": "alpha, written before anything moved",
            "confidence": 0.8, "origin": "llm", "why": "so there is something attributed"
        }),
    ));
    let id = stored["id"].as_str().expect("an id").to_owned();
    (server, id)
}

fn identities_in(server: &mut Server, set: &str) -> Vec<Value> {
    payload(&call(
        server,
        "hoard_recall",
        json!({"keywords": ["hoard-author"], "scope": {"shared": set}}),
    ))["memories"]
        .as_array()
        .expect("a list")
        .clone()
}

/// **`mint` says *I am new to this set*, and refuses where that is not true.** Minting a second
/// identity for one person is the hazard nothing downstream can report, so the operation that can
/// cause it is the one that has to refuse loudly rather than infer.
#[test]
fn mint_refuses_once_this_machine_already_has_an_identity_here() {
    let home = tempdir();
    let (mut server, _) = a_set_with_one_memory(&home, "hall");

    let again = sets(
        &mut server,
        json!({"action": "mint", "scope": {"shared": "hall"}, "author": "someone-new"}),
    );
    assert!(
        is_error(&again),
        "minting again would fork one person into two identities: {again}"
    );
    assert_eq!(identities_in(&mut server, "hall").len(), 1);
}

/// **`identify` says *I am this existing contributor*, and touches no set data at all.** It takes a
/// record id rather than a name: a name can match several records or none, which is the ambiguous
/// direction this design removed from resolution and must not reintroduce at the tool surface.
#[test]
fn identify_points_at_an_existing_record_and_refuses_one_the_set_lacks() {
    let home = tempdir();
    let (mut server, memory) = a_set_with_one_memory(&home, "hall");
    let existing = identities_in(&mut server, "hall")[0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let absent = sets(
        &mut server,
        json!({
            "action": "identify", "scope": {"shared": "hall"},
            "record": "01M1G5HKKZFBH9ANSXZZZZZZZZ"
        }),
    );
    assert!(
        is_error(&absent),
        "a record the set lacks must refuse: {absent}"
    );

    // A memory is not an identity record, and pointing at one would attribute every later write to
    // a piece of content.
    let not_an_identity = sets(
        &mut server,
        json!({"action": "identify", "scope": {"shared": "hall"}, "record": memory}),
    );
    assert!(
        is_error(&not_an_identity),
        "only a `hoard-author` record can be an identity: {not_an_identity}"
    );

    let good = sets(
        &mut server,
        json!({"action": "identify", "scope": {"shared": "hall"}, "record": existing}),
    );
    assert!(!is_error(&good), "{}", message(&good));
    assert_eq!(
        identities_in(&mut server, "hall").len(),
        1,
        "identify writes no set data, so nothing new appears"
    );
}

/// **`rename` supersedes *and* forgets, and the pairing is the whole point.** Superseding alone
/// leaves the old name readable by every holder, which is the half-answer everyone reaches for
/// first. What survives is the shape of the change, not the name.
#[test]
fn rename_supersedes_the_record_and_forgets_the_name_it_replaced() {
    let home = tempdir();
    let (mut server, memory) = a_set_with_one_memory(&home, "hall");
    let before = identities_in(&mut server, "hall")[0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let renamed = sets(
        &mut server,
        json!({"action": "rename", "scope": {"shared": "hall"}, "author": "jtcours@github"}),
    );
    assert!(!is_error(&renamed), "{}", message(&renamed));

    let now = identities_in(&mut server, "hall");
    assert_eq!(now.len(), 1, "one identity, not two: {now:?}");
    assert_eq!(now[0]["content"], json!("jtcours@github"));
    assert_ne!(
        now[0]["id"],
        json!(before),
        "a new record, not an edited one"
    );

    // The memory written beforehand was not touched, and still reaches the current name by one hop.
    let held = look_up_in(&mut server, &memory);
    assert_eq!(
        held["created"]["by"],
        json!(before),
        "the memory must not have been rewritten: {held}"
    );
    // The superseded record was forgotten as well, so looking it up reports a tombstone rather
    // than nothing: what was forgotten and what was never here are different answers, and after a
    // rename every earlier memory's `by` names one of these.
    let old = payload(&call(&mut server, "hoard_lookup", json!({"id": &before})));
    assert_eq!(
        old["memories"].as_array().map(Vec::len),
        Some(0),
        "a forgotten record is not a memory: {old}"
    );
    assert_eq!(
        old["forgotten"][0]["superseded_by"][0], now[0]["id"],
        "the tombstone must still say where the chain goes: {old}"
    );
    assert!(
        old["forgotten"][0]["content"].is_null(),
        "and must carry no name at all: {old}"
    );
}

/// The action it replaces is gone rather than left as a synonym: one string could not say which of
/// the three acts was meant, which is the whole reason for the split.
#[test]
fn the_author_action_no_longer_exists() {
    let home = tempdir();
    let (mut server, _) = a_set_with_one_memory(&home, "hall");
    let old = sets(
        &mut server,
        json!({"action": "author", "scope": {"shared": "hall"}, "author": "whoever"}),
    );
    assert!(is_error(&old), "`author` should be gone: {old}");
}

// ---------------------------------------------------------------------------
// A link that crosses a set boundary is visible from both ends
// ---------------------------------------------------------------------------

/// **A cross-set link used to be visible only from the side that wrote it.**
///
/// The edge lives in the *source's* database, and a lookup of the target asks the *target's* store,
/// so the row was never seen. That matters most where the legal direction is the one nobody would
/// search from: a personal memory may link into a shared set and a shared memory may not link back,
/// so a cluster spanning that boundary was reachable from the personal half alone.
///
/// Reported, never resolved — the same line `claims_against` draws. A cross-set row must not reach
/// `Store::incoming`, which resolution walks, because an id held in two sets would then let one
/// set's edge change the other's standing.
#[test]
fn a_link_from_another_set_is_reported_on_the_memory_it_points_at() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jtcours");
    create_chartered(&mut server, "shared-half");

    let target = payload(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "shared-half"}, "content": "the memory a personal note points at",
            "confidence": 0.8, "origin": "llm", "why": "so there is something to link to"
        }),
    ))["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let source = payload(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": "a personal note relating that to something",
            "confidence": 0.8, "origin": "llm", "why": "so the link crosses a boundary",
            "links": [{
                "label": "related_to", "target": &target, "target_set": "shared-half"
            }]
        }),
    ))["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let seen = look_up_in(&mut server, &target);

    // **Its own field, not folded into `incoming`.** `incoming` is what this memory's *own* set says
    // about it; a reference from elsewhere is a different fact, and merging them would lose the
    // distinction at the moment a reader needs it — which is the reasoning `disputed_by` already
    // encodes.
    assert!(
        seen["incoming"]
            .as_array()
            .is_none_or(|edges| !edges.iter().any(|edge| edge["source"] == json!(source))),
        "a reference from another set must not appear as a same-set edge: {seen}"
    );

    let references = seen["referenced_from"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let edge = references
        .iter()
        .find(|edge| edge["source"] == json!(source))
        .unwrap_or_else(|| panic!("the edge should be visible from the end it points at: {seen}"));
    assert_eq!(edge["label"], json!("related_to"));
    assert_eq!(
        edge["scope"],
        json!("local"),
        "and it has to name the set it came from, which `local` is exactly the case for: {seen}"
    );
}

/// **The same reference spelled the same way whichever tool returns it.** Found 2026-10-03 by
/// surveying every response shape the suite produces, while publishing output schemas
/// (`response-shape-is-unpublished`): lookup spelled `referenced_from[].scope` as the scalar the
/// schema publishes, and recall serialised `ReferencedFrom` with its derived `Serialize`, so the
/// same field arrived as `{"local": null}`-style objects. It is the
/// `a-scope-in-a-response-is-not-the-spelling-the-schema-publishes` drift again, at a site added
/// after `WireScope` fixed the others.
#[test]
fn recall_spells_a_reference_from_another_set_as_lookup_does() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jtcours");
    create_chartered(&mut server, "shared-half");
    create_chartered(&mut server, "other-half");
    let target = payload(&call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "shared-half"}, "content": "the memory another set points at",
            "confidence": 0.8, "origin": "llm", "why": "so there is something to link to"
        }),
    ))["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    // From a *shared* set: `Scope`'s derived form writes `local` as a plain string, so only a
    // shared source shows the object spelling.
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "other-half"}, "content": "a note in another set relating to that",
            "confidence": 0.8, "origin": "llm", "why": "so the link crosses a boundary",
            "links": [{"label": "related_to", "target": &target, "target_set": "shared-half"}]
        }),
    );

    let looked_up = look_up_in(&mut server, &target);
    assert_eq!(
        looked_up["referenced_from"][0]["scope"],
        json!("shared:other-half"),
        "{looked_up}"
    );
    let recalled = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "the memory another set points at"}),
    ));
    let hit = recalled["memories"]
        .as_array()
        .and_then(|hits| hits.iter().find(|hit| hit["id"] == json!(target)))
        .cloned()
        .unwrap_or_else(|| panic!("the target should be recalled: {recalled}"));
    assert_eq!(
        hit["referenced_from"][0]["scope"],
        json!("shared:other-half"),
        "recall must spell the scope as lookup does: {hit}"
    );
}

/// **A set that comes back after a `drop` comes back as itself.**
///
/// This is the test the durable author map was built for. It could not be written while `create`
/// minted a `set_id` that `adopt_set_id` would not displace: a re-added set arrived under a
/// *different* identity and a map row keyed on identity could not match. It stood in the suite with
/// its cache cleared by hand, naming `a-re-added-set-cannot-recover-its-identity`.
///
/// Two things have to hold together. The set must recover the identity in its own file, and writing
/// to it must not re-ask who you are — the second only follows from the first.
#[test]
fn re_adding_a_dropped_set_recovers_its_identity_and_does_not_re_ask() {
    let home = tempdir();
    let out = tempdir();
    let safety = tempdir();
    let mut server = a_server_as(&home, "jtcours");

    create_chartered(&mut server, "revenant");
    let first = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "revenant"}, "content": "alpha, from before the drop",
            "confidence": 0.8, "origin": "llm", "why": "so the set has an identity record"
        }),
    );
    assert!(!is_error(&first), "{}", message(&first));

    let identity = set_identity(&mut server, "revenant");

    sets(
        &mut server,
        json!({"action": "export", "scope": {"shared": "revenant"}, "directory": out.path()}),
    );
    let dropped = sets(
        &mut server,
        json!({
            "action": "drop", "name": "revenant",
            "save_to": safety.path().join("gone.json").to_string_lossy()
        }),
    );
    assert!(!is_error(&dropped), "{}", message(&dropped));

    sets(&mut server, json!({"action": "create", "name": "revenant"}));
    let imported = payload(&sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "revenant"}, "directory": out.path()}),
    ));
    assert_eq!(
        imported["adopted_set_id"], identity,
        "the set must take back the identity its own file carries: {imported}"
    );

    assert_eq!(
        set_identity(&mut server, "revenant"),
        identity,
        "and hold it afterwards"
    );

    // Which is what lets the durable map match, so this write is not met with the question the
    // first one already answered.
    let second = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "revenant"}, "content": "beta, after it came back",
            "confidence": 0.8, "origin": "llm", "why": "so the write has to resolve an author"
        }),
    );
    assert!(
        !is_error(&second),
        "re-adding a set must not re-ask who you are: {}",
        message(&second)
    );
    let identities = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["hoard-author"], "scope": {"shared": "revenant"}}),
    ));
    assert_eq!(
        identities["memories"].as_array().map(Vec::len),
        Some(1),
        "and must not have minted a second identity for one person: {identities}"
    );
}

/// The `set_id` `status` reports for one shared set.
fn set_identity(server: &mut Server, name: &str) -> Value {
    let status = payload(&sets(server, json!({"action": "status"})));
    status["sets"]
        .as_array()
        .expect("a list of sets")
        .iter()
        .find(|set| set["scope"] == json!(format!("shared:{name}")))
        .unwrap_or_else(|| panic!("{name} should be listed, got {status}"))["set_id"]
        .clone()
}

/// **A set whose cache is gone but whose identity this machine has already chosen stays quiet.**
///
/// The durable map answers before the question is put, so `status` must consult it rather than
/// reporting a question that will never be asked. This is the state a set is in between being
/// re-added and being written to: the identity records are back, `meta` is not, and the map is what
/// closes the gap.
#[test]
fn status_is_silent_when_the_map_already_knows_which_identity_is_yours() {
    let home = tempdir();
    let mut server = a_server_as(&home, "jtcours");
    create_chartered(&mut server, "remembered");
    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "remembered"}, "content": "alpha, which mints and remembers",
            "confidence": 0.8, "origin": "llm", "why": "so the map gains a row"
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));

    forget_which_identity_is_yours(&home, "remembered");

    let status = payload(&sets(&mut server, json!({"action": "status"})));
    let set = status["sets"]
        .as_array()
        .expect("a list of sets")
        .iter()
        .find(|set| set["scope"] == json!("shared:remembered"))
        .cloned()
        .expect("the set should be listed");

    assert!(
        set["author_note"].is_null(),
        "the map answers, so there is no question to warn about: {set}"
    );
}

// ---------------------------------------------------------------------------
// calibrate
// ---------------------------------------------------------------------------

/// **A floor needs evidence before it can be placed**, and saying so is the useful answer: set from
/// too few queries a floor sits too high and silences real questions, which is the one failure it
/// cannot report — a silenced query returns nothing, so nothing can be used, so nothing contradicts
/// it.
#[test]
fn calibrate_sets_no_floor_without_enough_labelled_queries() {
    let home = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is worth remembering");

    let response = sets(
        &mut server,
        json!({"action": "calibrate", "scope": "local"}),
    );
    assert!(!is_error(&response), "{}", message(&response));

    let result = payload(&response);
    assert_eq!(result["floor"], json!(null));
    assert_eq!(result["labelled_queries"], json!(0));
}

/// **The note on a floor that was set says what moves it, and it is not recall.** It used to say the
/// floor "lowers itself if a query naming this set ever scores under it", describing the rule removed
/// on 2026-09-12, the day the floor shipped: only `calibrate` writes the level, and a scoped query
/// under it is silenced and logged as evidence for the next calibration
/// (`a_scoped_query_the_floor_silences_is_still_logged_and_labelled` in `tests/floor.rs`).
/// `the-calibrate-note-promises-a-floor-that-lowers-itself`.
#[test]
fn a_floor_that_was_set_says_only_calibration_moves_it() {
    let home = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is worth remembering");
    for _ in 0..20 {
        let recalled = call(
            &mut server,
            "hoard_recall",
            json!({"scope": "local", "query": "alpha"}),
        );
        assert!(!is_error(&recalled), "{}", message(&recalled));
    }

    let result = payload(&sets(
        &mut server,
        json!({"action": "calibrate", "scope": "local"}),
    ));
    assert!(
        result["floor"].is_number(),
        "twenty scoped recalls should be enough to set a floor: {result}"
    );

    let note = result["note"].as_str().expect("a note");
    assert!(
        !note.contains("lowers itself"),
        "recall never writes the floor, so the note must not say it does: {note}"
    );
    assert!(
        note.contains("next calibration"),
        "the note should say where a contradicting query takes effect: {note}"
    );
}

/// `local`, holding `count` distinct memories and no labelled queries.
fn a_quiet_local_set(count: usize) -> (TempDir, Server) {
    let home = tempdir();
    let mut server = a_server(&home);
    let words = ["alpha", "beta", "gamma"];
    for index in 0..count {
        store_one(
            &mut server,
            &format!(
                "memory {index}: {} {} about {}",
                words[index % 3],
                words[(index + 1) % 3],
                words[(index / 3) % 3]
            ),
        );
    }
    (home, server)
}

/// **A set too little used to gather labels is calibrated from its own memories, and says so.**
/// `calibration-from-self-queries`. The source is in the response and in `status`, because a floor
/// inferred from the set's contents is weaker evidence than one read off queries it answered.
#[test]
fn calibrate_places_a_floor_from_a_quiet_sets_own_memories_and_says_so() {
    let (_home, mut server) = a_quiet_local_set(20);

    let result = payload(&sets(
        &mut server,
        json!({"action": "calibrate", "scope": "local"}),
    ));

    assert!(result["floor"].is_number(), "{result}");
    assert_eq!(
        result["floor_source"],
        json!("its-own-memories"),
        "{result}"
    );
    assert_eq!(result["memories_asked"], json!(20), "{result}");
    assert!(
        result["note"]
            .as_str()
            .is_some_and(|note| note.contains("own memories")),
        "{result}"
    );
    assert_eq!(
        local_status(&mut server)["floor_source"],
        json!("its-own-memories")
    );
}

/// **`status` offers the backup to a set big enough for it, and only to one.** Without the offer a
/// quiet set is never told it can have a floor; with it on a set too small, a session would calibrate
/// and be told there is not enough evidence.
#[test]
fn status_offers_a_floor_from_its_own_memories_only_to_a_set_big_enough() {
    let (_home, mut server) = a_quiet_local_set(20);
    let big = local_status(&mut server);
    assert!(
        big["calibration_note"]
            .as_str()
            .is_some_and(|note| note.contains("own memories")),
        "{big}"
    );

    let (_home, mut server) = a_quiet_local_set(5);
    let small = local_status(&mut server);
    assert!(small.get("calibration_note").is_none(), "{small}");
}

/// Calibration reads one set's own history, so it takes one set at a time — and a set nobody holds
/// is still an error rather than an invitation.
#[test]
fn calibrate_refuses_a_set_that_is_not_loaded() {
    let home = tempdir();
    let mut server = a_server(&home);

    let response = sets(
        &mut server,
        json!({"action": "calibrate", "scope": "shared:nowhere"}),
    );
    assert!(is_error(&response), "{}", message(&response));
}

/// **The remedy the self-lowering rule cannot provide.** A floor gives way only to a query that
/// named this set, and only at the next calibration — so an unscoped search silenced by a bad level
/// leaves no trace that corrects it, and without this the only other thing that clears a floor is
/// changing the embedding model.
#[test]
fn uncalibrate_removes_a_floor_and_says_what_it_removed() {
    let home = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is worth remembering");

    let cleared = payload(&sets(
        &mut server,
        json!({"action": "uncalibrate", "scope": "local"}),
    ));
    assert_eq!(
        cleared["cleared"],
        json!(null),
        "there was no floor to clear"
    );

    let response = sets(
        &mut server,
        json!({"action": "uncalibrate", "scope": "shared:nowhere"}),
    );
    assert!(is_error(&response), "{}", message(&response));
}

// ---------------------------------------------------------------------------
// Two loaded sets holding one identity -- `two-sets-can-share-an-identity`
// ---------------------------------------------------------------------------
//
// Written 2026-09-16 to reproduce the issue before fixing it. The route the issue first named -- the
// same file imported under two names -- is closed: import accepts only a file declaring the scope it
// is imported into (`import_refuses_a_file_belonging_to_another_set`). So is copying a database under
// another name, which refuses to open. What is left is an exported file whose declared name was
// edited before import, which `adopt_set_id` accepts into a set that holds no records yet -- and a
// machine that took that route loads both sets at every start, so the load needs the rule too.

/// Store one memory in `set` and return its id.
fn store_in(server: &mut Server, set: &str, content: &str) -> String {
    let response = call(
        server,
        "hoard_memorize",
        json!({
            "scope": format!("shared:{set}"), "content": content, "why": "a fixture",
            "confidence": 0.8, "origin": "llm"
        }),
    );
    payload(&response)["id"]
        .as_str()
        .unwrap_or_else(|| panic!("expected an id, got {response}"))
        .to_owned()
}

/// Which loaded sets report `identity`, by scope.
/// The loaded sets answering to `identity`, leaving out review copies -- which carry their source's
/// identity and hold it for nothing.
fn holders_of(server: &mut Server, identity: &str) -> Vec<Value> {
    payload(&sets(server, json!({"action": "status"})))["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .filter(|set| set["set_id"] == json!(identity) && set["mode"] != json!("evaluation"))
        .map(|set| set["scope"].clone())
        .collect()
}

/// The identity `set` reports in `status`.
fn identity_of(server: &mut Server, set: &str) -> String {
    payload(&sets(server, json!({"action": "status"})))["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|entry| entry["scope"] == json!(format!("shared:{set}")))
        .unwrap_or_else(|| panic!("{set} is not loaded"))["set_id"]
        .as_str()
        .expect("a set that has been written to has an identity")
        .to_owned()
}

/// **A server does not hold two sets answering to one identity**, however the stores came to share
/// it. Built through the store API because no tool should be able to produce it: this is the state a
/// machine is left in by an import that ran before the refusal existed, and it is loaded at every
/// start. A database copied under another name is not a route -- it refuses to open under a scope it
/// was not created with (`ScopeMismatch`).
#[test]
fn a_server_does_not_hold_two_sets_answering_to_one_identity() {
    let home = tempdir();
    let set = |name: &str| Scope::Shared(SetName::new(name).expect("valid set name"));
    let identity = {
        let original = Store::open(set("original"), home.path()).expect("opens");
        let identity = original.mint_set_id().expect("mints");
        let renamed = Store::open(set("renamed"), home.path()).expect("opens");
        assert!(
            renamed.adopt_set_id(identity).expect("adopts"),
            "the fixture needs the second store to take the first's identity"
        );
        identity.to_string()
    };

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    for name in ["renamed", "original"] {
        server.add_store(Store::open(set(name), home.path()).expect("opens"));
    }

    let holders = holders_of(&mut server, &identity);
    assert!(
        holders.is_empty(),
        "neither holder may load, since nothing says which is the original: {holders:?}"
    );

    let status = payload(&sets(&mut server, json!({"action": "status"})));
    let withheld: Vec<Value> = status["withheld"]
        .as_array()
        .unwrap_or_else(|| panic!("both must be reported as withheld: {status}"))
        .iter()
        .map(|entry| entry["scope"].clone())
        .collect();
    assert_eq!(
        withheld,
        vec![json!("shared:original"), json!("shared:renamed")],
        "a set that is not loaded must still be visible, or it reads as lost"
    );

    let refused = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:original", "content": "alpha", "why": "a fixture",
            "confidence": 0.8, "origin": "llm"
        }),
    );
    assert!(is_error(&refused));
    assert!(
        message(&refused).contains("same set identity"),
        "a write to a withheld set must say why it is not loaded: {}",
        message(&refused)
    );

    let copy = tempdir();
    let dropped = sets(
        &mut server,
        json!({"action": "drop", "name": "renamed", "save_to": copy.path().join("renamed.json")}),
    );
    assert!(!is_error(&dropped), "{}", message(&dropped));
    assert_eq!(
        holders_of(&mut server, &identity),
        vec![json!("shared:original")],
        "once its rival is dropped, the survivor loads without a restart"
    );
}

/// **An import cannot give a second loaded set an identity another loaded set holds**, even into a
/// set with none of its own, where adoption would otherwise fill the absence.
#[test]
fn an_import_does_not_give_a_second_loaded_set_an_identity_already_held() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "original");
    store_in(&mut server, "original", "alpha written before the export");
    let identity = identity_of(&mut server, "original");
    sets(
        &mut server,
        json!({"action": "export", "scope": "shared:original", "directory": out.path()}),
    );
    edit_in_file(
        &out,
        r#""scope":{"shared":"original"}"#,
        r#""scope":{"shared":"renamed"}"#,
    );

    sets(&mut server, json!({"action": "create", "name": "renamed"}));
    let response = sets(
        &mut server,
        json!({"action": "import", "scope": "shared:renamed", "directory": out.path()}),
    );

    assert!(
        is_error(&response),
        "the import must be refused: {}",
        message(&response)
    );
    assert!(
        message(&response).contains("shared:original"),
        "the refusal should name the set that holds the identity: {}",
        message(&response)
    );
    assert_eq!(
        holders_of(&mut server, &identity),
        vec![json!("shared:original")],
        "the refused import must leave the identity where it was"
    );
}

/// **A third holder of a withheld identity is withheld too**, rather than loading because the two
/// before it are no longer among the loaded sets.
#[test]
fn a_third_holder_of_a_withheld_identity_is_withheld_too() {
    let home = tempdir();
    let set = |name: &str| Scope::Shared(SetName::new(name).expect("valid set name"));
    let identity = {
        let first = Store::open(set("first"), home.path()).expect("opens");
        let identity = first.mint_set_id().expect("mints");
        for name in ["second", "third"] {
            let other = Store::open(set(name), home.path()).expect("opens");
            assert!(other.adopt_set_id(identity).expect("adopts"));
        }
        identity.to_string()
    };

    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    for name in ["first", "second", "third"] {
        server.add_store(Store::open(set(name), home.path()).expect("opens"));
    }

    assert_eq!(
        holders_of(&mut server, &identity),
        Vec::<Value>::new(),
        "the third holder must not load just because its rivals are withheld rather than loaded"
    );
}

// ---------------------------------------------------------------------------
// Read-only sets -- `documents/design/load-modes.md`
// ---------------------------------------------------------------------------

/// The mode `status` reports for `set`.
fn mode_of(server: &mut Server, set: &str) -> Value {
    payload(&sets(server, json!({"action": "status"})))["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|entry| entry["scope"] == json!(set))
        .unwrap_or_else(|| panic!("{set} is not loaded"))["mode"]
        .clone()
}

/// A file publishing the set `name` with one memory in it, and that memory's id.
fn a_published_set(name: &str) -> (TempDir, String) {
    let home = tempdir();
    let out = tempdir();
    let mut publisher = a_server(&home);
    create_chartered(&mut publisher, name);
    let id = store_in(&mut publisher, name, "alpha as the publisher wrote it");
    sets(
        &mut publisher,
        json!({"action": "export", "scope": format!("shared:{name}"), "directory": out.path()}),
    );
    (out, id)
}

/// A server that has taken on `name` read-only from `published`.
fn a_reader_of(home: &TempDir, name: &str, published: &TempDir) -> Server {
    let mut reader = a_server(home);
    let created = sets(
        &mut reader,
        json!({"action": "create", "name": name, "mode": "read-only"}),
    );
    assert!(!is_error(&created), "{}", message(&created));
    let imported = sets(
        &mut reader,
        json!({"action": "import", "scope": format!("shared:{name}"), "directory": published.path()}),
    );
    assert!(!is_error(&imported), "{}", message(&imported));
    reader
}

fn assert_refused_as_read_only(response: &Value, what: &str) {
    assert!(is_error(response), "{what} must be refused: {response}");
    assert!(
        message(response).contains("read-only"),
        "{what} should say the set is read-only: {}",
        message(response)
    );
}

#[test]
fn create_makes_a_read_write_set_unless_told_otherwise() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "ordinary"}));

    assert_eq!(mode_of(&mut server, "shared:ordinary"), json!("read-write"));
    assert_eq!(mode_of(&mut server, "local"), json!("read-write"));
}

#[test]
fn create_can_make_a_read_only_set() {
    let home = tempdir();
    let mut server = a_server(&home);
    let created = sets(
        &mut server,
        json!({"action": "create", "name": "reference", "mode": "read-only"}),
    );

    assert!(!is_error(&created), "{}", message(&created));
    assert_eq!(created_mode(&created), json!("read-only"));
    assert_eq!(mode_of(&mut server, "shared:reference"), json!("read-only"));
}

fn created_mode(response: &Value) -> Value {
    payload(response)["mode"].clone()
}

/// **`evaluation` is not a mode `create` can make**, and the refusal is the deserialiser's: an
/// evaluation load comes from a file under a name only `SetName::evaluation` builds, never from
/// `create`. It was briefly accepted here, the hour the variant was added.
#[test]
fn create_refuses_a_mode_it_does_not_know() {
    let home = tempdir();
    let mut server = a_server(&home);
    let created = sets(
        &mut server,
        json!({"action": "create", "name": "odd", "mode": "evaluation"}),
    );

    assert!(is_error(&created), "{}", message(&created));
    assert!(
        !Store::path_for(
            &Scope::Shared(SetName::new("odd").expect("valid")),
            home.path()
        )
        .exists(),
        "a refused create must not leave a database behind"
    );
}

/// **The mode is the machine's, and outlives the server that chose it.**
#[test]
fn a_read_only_set_is_still_read_only_after_a_restart() {
    let home = tempdir();
    {
        let mut server = a_server(&home);
        sets(
            &mut server,
            json!({"action": "create", "name": "reference", "mode": "read-only"}),
        );
    }
    let reopened = Store::open(
        Scope::Shared(SetName::new("reference").expect("valid")),
        home.path(),
    )
    .expect("reopens");
    let mut server = a_server(&home);
    server.add_store(reopened);

    assert_eq!(mode_of(&mut server, "shared:reference"), json!("read-only"));
}

#[test]
fn a_read_only_set_takes_its_memories_from_a_file_and_can_be_recalled() {
    let (published, id) = a_published_set("reference");
    let home = tempdir();
    let mut reader = a_reader_of(&home, "reference", &published);

    let found = payload(&call(
        &mut reader,
        "hoard_recall",
        json!({"query": "alpha", "scope": "shared:reference"}),
    ));
    let ids: Vec<&Value> = found["memories"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|memory| &memory["id"])
        .collect();
    assert_eq!(ids, vec![&json!(id)]);
}

#[test]
fn a_read_only_set_refuses_a_new_memory() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(
        &mut server,
        json!({"action": "create", "name": "reference", "mode": "read-only"}),
    );

    let response = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:reference", "content": "alpha", "why": "a fixture",
            "confidence": 0.8, "origin": "llm"
        }),
    );
    assert_refused_as_read_only(&response, "memorizing into a read-only set");
}

#[test]
fn a_read_only_set_refuses_amending_a_memory() {
    let (published, id) = a_published_set("reference");
    let home = tempdir();
    let mut reader = a_reader_of(&home, "reference", &published);

    let response = call(
        &mut reader,
        "hoard_correct",
        json!({"action": "amend", "scope": "shared:reference", "id": id, "content": "changed"}),
    );
    assert_refused_as_read_only(&response, "amending a read-only memory");
}

#[test]
fn a_read_only_set_refuses_forgetting_a_memory() {
    let (published, id) = a_published_set("reference");
    let home = tempdir();
    let mut reader = a_reader_of(&home, "reference", &published);

    let response = call(
        &mut reader,
        "hoard_correct",
        json!({"action": "forget", "scope": "shared:reference", "id": id}),
    );
    assert_refused_as_read_only(&response, "forgetting a read-only memory");
    let listed = payload(&sets(&mut reader, json!({"action": "status"})));
    let entry = listed["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|entry| entry["scope"] == json!("shared:reference"))
        .expect("loaded")
        .clone();
    // One more than the test's own writes: the set's charter, which its first memory has to be.
    assert_eq!(
        entry["memories"],
        json!(2),
        "the memory must still be there"
    );
}

/// **Refused before anything is written**, the safety copy included: absorbing writes that copy,
/// merges another set's records and carries replacements forward, and a refusal partway through any
/// of it would leave something behind.
#[test]
fn a_read_only_set_refuses_absorbing_another_set() {
    let (published, _) = a_published_set("reference");
    let (other, _) = a_published_set("other");
    let home = tempdir();
    let mut reader = a_reader_of(&home, "reference", &published);
    let copy = tempdir();

    let response = sets(
        &mut reader,
        json!({
            "action": "absorb", "scope": "shared:reference", "set": "other", "directory": other.path(),
            "save_to": copy.path().join("before.json")
        }),
    );
    assert_refused_as_read_only(&response, "absorbing into a read-only set");
    assert!(
        !copy.path().join("before.json").exists(),
        "a refused absorb must not have written its safety copy"
    );
    let listed = payload(&sets(&mut reader, json!({"action": "status"})));
    let entry = listed["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|entry| entry["scope"] == json!("shared:reference"))
        .expect("loaded")
        .clone();
    // One more than the test's own writes: the set's charter, which its first memory has to be.
    assert_eq!(entry["memories"], json!(2), "nothing may have been merged");
}

/// Identity records exist to attribute writes, and a read-only set takes none.
#[test]
fn a_read_only_set_refuses_the_author_actions() {
    let (published, id) = a_published_set("reference");
    let home = tempdir();
    let mut reader = a_reader_of(&home, "reference", &published);

    for (what, arguments) in [
        (
            "mint",
            json!({"action": "mint", "scope": "shared:reference", "author": "someone"}),
        ),
        (
            "rename",
            json!({"action": "rename", "scope": "shared:reference", "author": "someone"}),
        ),
        (
            "identify",
            json!({"action": "identify", "scope": "shared:reference", "record": id}),
        ),
    ] {
        let response = sets(&mut reader, arguments);
        assert_refused_as_read_only(&response, what);
    }
}

/// **A read-only set can reproduce the file it came from**, byte for byte, if its publisher wrote
/// that file with this build and model: the memories are the store's, the header's `set_id` and
/// `scope` came with them, and the summary is derived from the same records.
#[test]
fn a_read_only_set_re_exports_the_file_it_was_given() {
    let (published, _) = a_published_set("reference");
    let home = tempdir();
    let mut reader = a_reader_of(&home, "reference", &published);

    let out = tempdir();
    let exported = sets(
        &mut reader,
        json!({"action": "export", "scope": "shared:reference", "directory": out.path()}),
    );
    assert!(!is_error(&exported), "{}", message(&exported));

    let theirs = std::fs::read_to_string(memories_file(&published)).expect("reads");
    let ours = std::fs::read_to_string(memories_file(&out)).expect("reads");
    assert_eq!(
        ours, theirs,
        "a read-only set holds exactly what the file brought, so its export should reproduce it"
    );
}

/// **A read-only set is dropped without a safety copy**, which is one of the reasons to hold a set
/// that way: nothing is written that someone then has to clean up.
#[test]
fn dropping_a_read_only_set_needs_no_save_to() {
    let (published, _) = a_published_set("reference");
    let home = tempdir();
    let mut reader = a_reader_of(&home, "reference", &published);

    let dropped = sets(&mut reader, json!({"action": "drop", "name": "reference"}));
    assert!(!is_error(&dropped), "{}", message(&dropped));
    assert_eq!(payload(&dropped)["safety_copy"], json!(null));
    assert!(
        payload(&dropped)["safety_copy_skipped"].is_string(),
        "the response must say why no copy was written: {}",
        payload(&dropped)
    );
    assert!(
        !Store::path_for(
            &Scope::Shared(SetName::new("reference").expect("valid")),
            home.path()
        )
        .exists(),
        "the database should be gone"
    );
    assert!(
        memories_file(&published).is_file(),
        "the publisher's file is what the set is restored from, and is untouched"
    );
}

/// Given anyway -- out of habit, or by someone who wants the copy -- `save_to` is honoured rather
/// than refused.
#[test]
fn dropping_a_read_only_set_honours_a_save_to_that_is_given() {
    let (published, _) = a_published_set("reference");
    let home = tempdir();
    let mut reader = a_reader_of(&home, "reference", &published);
    let copy = tempdir();
    let path = copy.path().join("reference.json");

    let dropped = sets(
        &mut reader,
        json!({"action": "drop", "name": "reference", "save_to": path}),
    );
    assert!(!is_error(&dropped), "{}", message(&dropped));
    assert_eq!(
        payload(&dropped)["safety_copy"],
        json!(path.to_string_lossy())
    );
    assert!(path.is_file(), "the copy the caller asked for must exist");
}

/// The unconditional save survives where it was argued for: a set this machine could have written to.
#[test]
fn dropping_a_read_write_set_without_save_to_is_refused() {
    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "ours");
    store_in(&mut server, "ours", "alpha written here");

    let dropped = sets(&mut server, json!({"action": "drop", "name": "ours"}));
    assert!(is_error(&dropped));
    assert!(
        message(&dropped).contains("save_to"),
        "the refusal must name what is missing: {}",
        message(&dropped)
    );
    assert_eq!(
        mode_of(&mut server, "shared:ours"),
        json!("read-write"),
        "the set must still be loaded"
    );
}

// ---------------------------------------------------------------------------
// One copy of each set -- `documents/design/load-modes.md#one-copy-of-each-set`
// ---------------------------------------------------------------------------

/// A server over `home`, with every set on disk added as a start would add it.
fn a_server_loading(home: &TempDir, names: &[&str]) -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    for name in names {
        server.add_store(
            Store::open(
                Scope::Shared(SetName::new(*name).expect("valid set name")),
                home.path(),
            )
            .expect("opens"),
        );
    }
    server
}

/// Build the sets named by `read_only` and `read_write` on disk, all answering to one identity.
fn copies_of_one_identity(home: &TempDir, read_write: &[&str], read_only: &[&str]) -> String {
    let set = |name: &str| Scope::Shared(SetName::new(name).expect("valid set name"));
    let first = read_write
        .first()
        .or_else(|| read_only.first())
        .expect("a copy");
    let identity = Store::open(set(first), home.path())
        .expect("opens")
        .mint_set_id()
        .expect("mints");
    for name in read_write.iter().chain(read_only) {
        let store = Store::open(set(name), home.path()).expect("opens");
        store.adopt_set_id(identity).expect("adopts");
        if read_only.contains(name) {
            store.choose_read_only().expect("is empty");
        }
    }
    identity.to_string()
}

fn withheld_scopes(server: &mut Server) -> Vec<Value> {
    payload(&sets(server, json!({"action": "status"})))["withheld"]
        .as_array()
        .map(|entries| entries.iter().map(|entry| entry["scope"].clone()).collect())
        .unwrap_or_default()
}

/// **The machine's own copy wins.** A read-only copy of a set you also hold read-write is the copy
/// that steps aside, so a restart cannot lock you out of your own set.
#[test]
fn a_read_write_copy_loads_and_read_only_copies_of_it_are_withheld() {
    let home = tempdir();
    let identity = copies_of_one_identity(&home, &["ours"], &["theirs", "zzz-theirs-too"]);
    let mut server = a_server_loading(&home, &["ours", "theirs", "zzz-theirs-too"]);

    assert_eq!(
        holders_of(&mut server, &identity),
        vec![json!("shared:ours")]
    );
    assert_eq!(
        withheld_scopes(&mut server),
        vec![json!("shared:theirs"), json!("shared:zzz-theirs-too")]
    );
    write_charter(&mut server, "ours");
    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "shared:ours", "content": "alpha written to our own copy",
            "why": "a fixture", "confidence": 0.8, "origin": "llm"
        }),
    );
    assert!(!is_error(&stored), "{}", message(&stored));
}

/// With no read-write copy nothing says which is the original, so none loads.
#[test]
fn read_only_copies_of_one_identity_are_all_withheld() {
    let home = tempdir();
    let identity = copies_of_one_identity(&home, &[], &["one", "two"]);
    let mut server = a_server_loading(&home, &["one", "two"]);

    assert!(holders_of(&mut server, &identity).is_empty());
    assert_eq!(
        withheld_scopes(&mut server),
        vec![json!("shared:one"), json!("shared:two")]
    );
}

/// **Order does not decide it.** The read-write copy wins whether it was added first or last.
#[test]
fn the_read_write_copy_wins_whichever_order_the_stores_arrive_in() {
    let home = tempdir();
    let identity = copies_of_one_identity(&home, &["zzz-ours"], &["theirs"]);
    let mut server = a_server_loading(&home, &["theirs", "zzz-ours"]);

    assert_eq!(
        holders_of(&mut server, &identity),
        vec![json!("shared:zzz-ours")]
    );
}

/// Dropping the last rival leaves one copy, and one copy loads whatever its mode.
#[test]
fn dropping_the_read_write_copy_loads_the_read_only_one_that_is_left() {
    let home = tempdir();
    let identity = copies_of_one_identity(&home, &["ours"], &["theirs"]);
    let mut server = a_server_loading(&home, &["ours", "theirs"]);
    let copy = tempdir();

    let dropped = sets(
        &mut server,
        json!({"action": "drop", "name": "ours", "save_to": copy.path().join("ours.json")}),
    );
    assert!(!is_error(&dropped), "{}", message(&dropped));
    assert_eq!(
        holders_of(&mut server, &identity),
        vec![json!("shared:theirs")],
        "one copy is left, so it loads"
    );
}

/// **`status` names the marks to recall for each set**, so the mapping from mode to marks lives
/// beside the mode instead of in convention text -- `documents/design/load-modes.md`.
#[test]
fn status_names_the_watch_marks_for_each_set() {
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "ours"}));
    sets(
        &mut server,
        json!({"action": "create", "name": "theirs", "mode": "read-only"}),
    );

    let marks = |server: &mut Server, scope: &str| -> Value {
        payload(&sets(server, json!({"action": "status"})))["sets"]
            .as_array()
            .expect("a list")
            .iter()
            .find(|entry| entry["scope"] == json!(scope))
            .unwrap_or_else(|| panic!("{scope} is not loaded"))["watch_marks"]
            .clone()
    };

    assert_eq!(
        marks(&mut server, "shared:ours"),
        json!(["hoard-signpost", "hoard-collect", "hoard-investigation"])
    );
    assert_eq!(
        marks(&mut server, "shared:theirs"),
        json!(["hoard-signpost"]),
        "a read-only set pulls the mark that asks for a recall, and not the two that ask for writes"
    );
    assert_eq!(
        marks(&mut server, "local"),
        marks(&mut server, "shared:ours")
    );
}

/// **`status` reports each set's calibration state**: the floor it holds, and how much labelled
/// evidence has arrived since the last calibration. Without it, the only ways to ask change the
/// answer -- `calibrate` sets a floor and `uncalibrate` removes one.
#[test]
fn status_reports_a_sets_floor_and_its_evidence_since_calibration() {
    let home = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "alpha the only memory here");

    let entry = |server: &mut Server| -> Value {
        local_set(&payload(&sets(server, json!({"action": "status"}))))
    };
    let before = entry(&mut server);
    assert_eq!(
        before["non_match_floor"],
        json!(null),
        "a set with no floor says so rather than leaving the field out"
    );
    assert_eq!(before["labelled_since_calibration"], json!(0));
    assert!(
        before["calibration_note"].is_null(),
        "nothing to offer before there is evidence: {before}"
    );

    for _ in 0..20 {
        call(
            &mut server,
            "hoard_recall",
            json!({"query": "alpha", "scope": "local"}),
        );
    }

    let ready = entry(&mut server);
    assert_eq!(ready["labelled_since_calibration"], json!(20));
    let note = ready["calibration_note"].as_str().unwrap_or_else(|| {
        panic!("a set past the threshold should be offered calibration: {ready}")
    });
    assert!(
        note.contains("calibrate"),
        "the note should name the action: {note}"
    );

    sets(
        &mut server,
        json!({"action": "calibrate", "scope": "local"}),
    );
    let after = entry(&mut server);
    assert!(
        after["non_match_floor"].is_number(),
        "the floor it just set must be readable: {after}"
    );
    assert_eq!(after["labelled_since_calibration"], json!(0));
    assert!(
        after["calibration_note"].is_null(),
        "nothing more to offer once the evidence is consumed: {after}"
    );
}

// ---------------------------------------------------------------------------
// Evaluation loads -- `documents/design/load-modes.md#evaluation`
// ---------------------------------------------------------------------------

/// **A file is read for review without being adopted**: a store in memory, under a temporary name
/// the software builds from the file's own set name and the user's label.
#[test]
fn evaluate_loads_a_file_under_a_name_that_says_it_is_a_copy() {
    let (published, id) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut server = a_server(&home);

    let loaded = sets(
        &mut server,
        json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
    );
    assert!(!is_error(&loaded), "{}", message(&loaded));
    assert_eq!(
        payload(&loaded)["loaded"],
        json!("shared:eval-scatterhoard-issue-3")
    );
    // Two: the publisher's memory, and the identity record their first write minted.
    // And the publisher's charter, which their set's first memory had to be.
    assert_eq!(payload(&loaded)["memories"], json!(3));
    assert!(
        payload(&loaded)["took_ms"].is_number(),
        "a review says what it cost, since nothing else can measure this machine: {}",
        payload(&loaded)
    );
    assert_eq!(
        mode_of(&mut server, "shared:eval-scatterhoard-issue-3"),
        json!("evaluation")
    );

    let found = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "shared:eval-scatterhoard-issue-3"}),
    ));
    assert_eq!(
        found["memories"][0]["id"],
        json!(id),
        "the contributor's memory is readable under review: {found}"
    );
}

/// **Adopting nothing**: the set the file names is untouched, and the copy's database is kept apart
/// from the machine's own stores, under the home rather than in the system temp directory --
/// `documents/design/load-modes.md`.
#[test]
fn evaluate_leaves_the_named_set_alone_and_keeps_its_database_apart() {
    let (published, _) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut server = a_server(&home);
    sets(
        &mut server,
        json!({"action": "create", "name": "scatterhoard"}),
    );

    let loaded = sets(
        &mut server,
        json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
    );
    assert!(!is_error(&loaded), "{}", message(&loaded));

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let own = listed["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|set| set["scope"] == json!("shared:scatterhoard"))
        .expect("the machine's own set is still loaded");
    assert_eq!(own["memories"], json!(0), "the review must not reach it");
    assert_eq!(own["mode"], json!("read-write"));

    let held = std::path::PathBuf::from(
        payload(&loaded)["held_at"]
            .as_str()
            .unwrap_or_else(|| panic!("the response must say where it is: {}", payload(&loaded))),
    );
    assert!(held.is_file(), "the copy is a database on disk: {held:?}");
    assert!(
        held.starts_with(home.path()),
        "under the home, which is sized for stores, rather than wherever temp files go: {held:?}"
    );
    assert!(
        !Store::path_for(
            &Scope::Shared(SetName::new("eval-scatterhoard-issue-3").expect("a legal name")),
            home.path()
        )
        .exists(),
        "and not among the machine's own stores, where a restart would load it"
    );
}

/// **A restart keeps the review**, because the copy is a file the next start loads again. That is
/// what makes `drop` the one way a copy goes, rather than a second cleanup nobody runs -- and it
/// spares a reviewer re-embedding someone else's whole set after every restart.
#[test]
fn a_restart_loads_a_review_copy_again() {
    let (published, _) = a_published_set("scatterhoard");
    let home = tempdir();
    let held = {
        let mut server = a_server(&home);
        let loaded = sets(
            &mut server,
            json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
        );
        std::path::PathBuf::from(payload(&loaded)["held_at"].as_str().expect("a path"))
    };
    assert!(held.is_file());

    // As a start does it: everything the layout discovers, which is what `main.rs` loads.
    let mut restarted = Server::new();
    restarted.set_account_name(Author::new("jtcours").expect("valid author"));
    restarted.set_embedder(Arc::new(MarkerEmbedder));
    let layout = StoreLayout::new(home.path().to_path_buf(), None);
    restarted.set_layout(layout.clone());
    for (scope, path) in layout.discover().stores {
        restarted.add_store(Store::open_at(scope, &path).expect("opens"));
    }

    assert_eq!(
        mode_of(&mut restarted, "shared:eval-scatterhoard-issue-3"),
        json!("evaluation"),
        "the review is still open, and still marked as a review"
    );

    // And ending it takes the file with it.
    let dropped = sets(
        &mut restarted,
        json!({"action": "drop", "name": "eval-scatterhoard-issue-3"}),
    );
    assert!(!is_error(&dropped), "{}", message(&dropped));
    assert!(!held.exists(), "drop is the one way a copy goes: {held:?}");
}

/// The label is what tells two reviews apart, so the same label twice is refused rather than
/// silently replacing the first.
#[test]
fn evaluate_refuses_a_label_already_loaded() {
    let (published, _) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut server = a_server(&home);
    sets(
        &mut server,
        json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
    );

    let again = sets(
        &mut server,
        json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
    );
    assert!(is_error(&again));
    assert!(
        message(&again).contains("issue-3"),
        "the refusal should name the label: {}",
        message(&again)
    );
    // **"loaded", not merely an error.** Two guards refuse this call -- this server already holding
    // the label, and the file already existing because another server does -- and both messages
    // name the label, so asserting the label alone passes whichever one fires. That let the
    // mutation disabling the in-process guard survive a full pass on 2026-09-19: the on-disk guard
    // caught the same call by the other route and the test could not tell. The distinction is
    // operational, not cosmetic: *loaded* means drop it here, *held* means another server has it.
    assert!(
        message(&again).contains("already loaded for review"),
        "and should say this server holds it, not that the file exists: {}",
        message(&again)
    );
}

/// **Several reviews at once, of one set the machine also holds.** All three answer to one identity
/// — the copies carry their source's — and none withholds another: a review copy is never a holder,
/// or loading one would lock the reviewer out of their own set.
#[test]
fn two_evaluation_loads_and_the_real_set_can_be_open_together() {
    let (published, _) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut server = a_server(&home);
    sets(
        &mut server,
        json!({"action": "create", "name": "scatterhoard"}),
    );
    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "shared:scatterhoard", "directory": published.path()}),
    );
    assert!(!is_error(&imported), "{}", message(&imported));
    let identity = identity_of(&mut server, "scatterhoard");

    for label in ["issue-3", "issue-4"] {
        let loaded = sets(
            &mut server,
            json!({"action": "evaluate", "label": label, "set": "scatterhoard", "directory": published.path()}),
        );
        assert!(!is_error(&loaded), "{}", message(&loaded));
        assert_eq!(
            payload(&loaded)["set_id"],
            json!(identity),
            "a copy carries its source's identity"
        );
    }

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let evaluations: Vec<Value> = listed["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .filter(|set| set["mode"] == json!("evaluation"))
        .map(|set| set["scope"].clone())
        .collect();
    assert_eq!(
        evaluations,
        vec![
            json!("shared:eval-scatterhoard-issue-3"),
            json!("shared:eval-scatterhoard-issue-4")
        ]
    );
    assert_eq!(
        holders_of(&mut server, &identity),
        vec![json!("shared:scatterhoard")],
        "the machine's own set is the copy that holds the identity, and it stays loaded"
    );
    assert!(
        listed["withheld"].is_null(),
        "no copy withholds another: {listed}"
    );
}

/// A review is over when the copy is dropped, and nothing is saved: the file it came from is the
/// copy.
#[test]
fn an_evaluation_load_is_dropped_without_a_safety_copy() {
    let (published, _) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut server = a_server(&home);
    sets(
        &mut server,
        json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
    );

    let dropped = sets(
        &mut server,
        json!({"action": "drop", "name": "eval-scatterhoard-issue-3"}),
    );
    assert!(!is_error(&dropped), "{}", message(&dropped));
    assert_eq!(payload(&dropped)["safety_copy"], json!(null));
    let report = payload(&dropped);
    let removed = report["database_removed"].as_str().unwrap_or_else(|| {
        panic!(
            "ending a review takes its database with it: {}",
            payload(&dropped)
        )
    });
    assert!(
        !std::path::Path::new(removed).exists(),
        "the copy's database must be gone: {removed}"
    );
    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    assert!(
        !listed["sets"]
            .as_array()
            .expect("a list")
            .iter()
            .any(|set| set["mode"] == json!("evaluation")),
        "the review is over: {listed}"
    );
}

/// **`create` refuses the evaluation prefix**, one of the three boundaries that keep it meaning what
/// it says. The others are a file's header and a link written into a store.
#[test]
fn create_refuses_a_name_reserved_for_a_review_copy() {
    let home = tempdir();
    let mut server = a_server(&home);

    let refused = sets(
        &mut server,
        json!({"action": "create", "name": "eval-scatterhoard-issue-3"}),
    );
    assert!(is_error(&refused));
    assert!(
        message(&refused).contains("evaluate"),
        "the refusal should name the action that does make one: {}",
        message(&refused)
    );
}

/// **The other order, which is where the exclusion earns its place.** A review opened before the
/// machine's own copy of that set loads must not be withheld when it arrives: settling an identity
/// runs again on every arrival, and a review copy counted there would be set aside as the loser.
#[test]
fn a_review_opened_first_survives_its_own_set_loading_afterwards() {
    let (published, _) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut server = a_server(&home);

    sets(
        &mut server,
        json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
    );
    sets(
        &mut server,
        json!({"action": "create", "name": "scatterhoard"}),
    );
    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "shared:scatterhoard", "directory": published.path()}),
    );
    assert!(
        !is_error(&imported),
        "an open review of a set must not stop that set importing its own file: {}",
        message(&imported)
    );

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let loaded: Vec<Value> = listed["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|set| set["scope"].clone())
        .collect();
    assert!(
        loaded.contains(&json!("shared:eval-scatterhoard-issue-3")),
        "the review must survive its own set arriving: {listed}"
    );
    assert!(
        loaded.contains(&json!("shared:scatterhoard")),
        "and the machine's own set must load: {listed}"
    );
    assert!(
        listed["withheld"].is_null(),
        "neither withholds the other: {listed}"
    );
}

/// **Settling runs again when a copy goes**, and a review open at that moment must survive it: two
/// reviews of a set whose own copy is dropped would otherwise be counted as rival holders with no
/// read-write copy between them, and both set aside.
#[test]
fn dropping_a_set_does_not_withhold_the_reviews_of_it() {
    let (published, _) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut server = a_server(&home);
    sets(
        &mut server,
        json!({"action": "create", "name": "scatterhoard"}),
    );
    sets(
        &mut server,
        json!({"action": "import", "scope": "shared:scatterhoard", "directory": published.path()}),
    );
    for label in ["issue-3", "issue-4"] {
        sets(
            &mut server,
            json!({"action": "evaluate", "label": label, "set": "scatterhoard", "directory": published.path()}),
        );
    }

    let copy = tempdir();
    let dropped = sets(
        &mut server,
        json!({"action": "drop", "name": "scatterhoard", "save_to": copy.path().join("own.json")}),
    );
    assert!(!is_error(&dropped), "{}", message(&dropped));

    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let evaluations: Vec<Value> = listed["sets"]
        .as_array()
        .expect("a list")
        .iter()
        .filter(|set| set["mode"] == json!("evaluation"))
        .map(|set| set["scope"].clone())
        .collect();
    assert_eq!(
        evaluations,
        vec![
            json!("shared:eval-scatterhoard-issue-3"),
            json!("shared:eval-scatterhoard-issue-4")
        ],
        "both reviews must still be readable: {listed}"
    );
    assert!(
        listed["withheld"].is_null(),
        "and neither withheld: {listed}"
    );
}

/// **A review copy is searched only when a recall names it.** Excluded rather than filtered
/// afterwards: scores are computed against every store a recall searched, so a copy nobody adopted
/// would move the ranking of everything else even if its own memories were dropped from the result.
#[test]
fn an_unscoped_recall_does_not_search_a_review_copy() {
    let (published, id) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut server = a_server(&home);
    sets(
        &mut server,
        json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
    );

    let unscoped = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ));
    let ids: Vec<String> = unscoped["memories"]
        .as_array()
        .map(|found| {
            found
                .iter()
                .filter_map(|memory| memory["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        !ids.contains(&id),
        "a recall naming no set must not reach a copy under review: {unscoped}"
    );

    let named = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "shared:eval-scatterhoard-issue-3"}),
    ));
    assert_eq!(
        named["memories"][0]["id"],
        json!(id),
        "and naming it must reach it: {named}"
    );
}

/// **Every memory a review copy returns is marked, and the response says what the mark means.** The
/// mark comes from the store's load mode rather than from the `eval-` name, so no return path can
/// forget it by reading the wrong thing.
#[test]
fn memories_from_a_review_copy_are_marked_and_explained() {
    let (published, id) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut server = a_server(&home);
    store_local(&mut server, "alpha in a set of my own");
    sets(
        &mut server,
        json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
    );

    let reviewing = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "scope": "shared:eval-scatterhoard-issue-3"}),
    ));
    assert_eq!(reviewing["memories"][0]["evaluation"], json!(true));
    let note = reviewing["under_review_note"]
        .as_str()
        .unwrap_or_else(|| panic!("the response must say what the mark means: {reviewing}"));
    assert!(note.contains("proposes"), "{note}");

    // A summary index carries it too: an index that cannot tell a proposal from the record is an
    // index of claims the project has not made.
    let summarised = payload(&call(
        &mut server,
        "hoard_recall",
        json!({
            "query": "alpha", "scope": "shared:eval-scatterhoard-issue-3", "detail": "summary"
        }),
    ));
    assert_eq!(summarised["memories"][0]["evaluation"], json!(true));

    // And a lookup by id, which reaches every loaded set.
    let looked_up = payload(&call(&mut server, "hoard_lookup", json!({"id": id})));
    assert_eq!(looked_up["memories"][0]["evaluation"], json!(true));
    assert!(looked_up["under_review_note"].is_string());

    // The machine's own memories carry no mark, and their responses no note.
    let ordinary = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha"}),
    ));
    assert!(
        ordinary["memories"][0]["evaluation"].is_null(),
        "{ordinary}"
    );
    assert!(ordinary["under_review_note"].is_null(), "{ordinary}");
}

/// **A link out of a review copy is handed over, not followed** — with its target, the set it names,
/// that set's identity, and every loaded scope answering to it. The reviewer's own copy and a second
/// review of the same set are both listed, with how each is held, because choosing between them is
/// the caller's to do: `documents/design/load-modes.md`.
#[test]
fn a_review_copys_cross_set_links_are_surfaced_with_every_loaded_candidate() {
    // The contributor's machine: two sets, and a memory in one pointing into the other.
    let publisher_home = tempdir();
    let out = tempdir();
    let lessons_out = tempdir();
    let mut publisher = a_server(&publisher_home);
    create_chartered(&mut publisher, "coding-lessons");
    create_chartered(&mut publisher, "scatterhoard");
    let lesson = store_in(
        &mut publisher,
        "coding-lessons",
        "alpha the lesson it rests on",
    );
    let pointing = call(
        &mut publisher,
        "hoard_memorize",
        json!({
            "scope": "shared:scatterhoard",
            "content": "alpha the decision that rests on it",
            "why": "a fixture",
            "confidence": 0.8,
            "origin": "llm",
            "links": [{
                "label": "motivated_by",
                "target": lesson,
                "target_set": "coding-lessons",
            }],
        }),
    );
    assert!(!is_error(&pointing), "{}", message(&pointing));
    for (name, directory) in [("scatterhoard", &out), ("coding-lessons", &lessons_out)] {
        sets(
            &mut publisher,
            json!({"action": "export", "scope": format!("shared:{name}"), "directory": directory.path()}),
        );
    }

    // The reviewer's machine: their own copy of the lessons, and two reviews open.
    let home = tempdir();
    let mut reviewer = a_server(&home);
    sets(
        &mut reviewer,
        json!({"action": "create", "name": "coding-lessons"}),
    );
    sets(
        &mut reviewer,
        json!({"action": "import", "scope": "shared:coding-lessons", "directory": lessons_out.path()}),
    );
    for (label, set, directory) in [
        ("issue-3", "scatterhoard", &out),
        ("issue-3-lessons", "coding-lessons", &lessons_out),
    ] {
        let loaded = sets(
            &mut reviewer,
            json!({"action": "evaluate", "label": label, "set": set, "directory": directory.path()}),
        );
        assert!(!is_error(&loaded), "{}", message(&loaded));
    }

    let reviewing = payload(&call(
        &mut reviewer,
        "hoard_recall",
        json!({"query": "alpha the decision", "scope": "shared:eval-scatterhoard-issue-3"}),
    ));
    let surfaced = &reviewing["memories"][0]["cross_set_targets"][0];
    assert_eq!(surfaced["set"], json!("coding-lessons"), "{reviewing}");
    assert_eq!(surfaced["id"], json!(lesson));
    assert!(
        surfaced["set_id"].is_string(),
        "the identity it named: {surfaced}"
    );
    assert_eq!(
        surfaced["loaded"],
        json!([
            {"scope": "shared:coding-lessons", "mode": "read-write"},
            {"scope": "shared:eval-coding-lessons-issue-3-lessons", "mode": "evaluation"},
        ]),
        "every loaded scope answering to that identity, and how each is held: {surfaced}"
    );

    // And the reviewer's own copy of that same set -- holding the very same linking memory -- gets
    // no such list: an ordinary set resolves its links, so candidates there would be noise.
    sets(
        &mut reviewer,
        json!({"action": "create", "name": "scatterhoard"}),
    );
    let imported = sets(
        &mut reviewer,
        json!({"action": "import", "scope": "shared:scatterhoard", "directory": out.path()}),
    );
    assert!(!is_error(&imported), "{}", message(&imported));
    let ordinary = payload(&call(
        &mut reviewer,
        "hoard_recall",
        json!({"query": "alpha the decision"}),
    ));
    assert!(
        ordinary["memories"]
            .as_array()
            .is_some_and(|found| !found.is_empty()),
        "the reviewer's own copy should answer: {ordinary}"
    );
    assert!(
        ordinary["memories"].as_array().is_none_or(|found| {
            found.iter().all(|memory| {
                memory["evaluation"].is_null() && memory["cross_set_targets"].is_null()
            })
        }),
        "an ordinary set resolves its own links, so listing candidates there would be noise: \
         {ordinary}"
    );
}

/// **A label another server is already reviewing under is taken here too.** Servers share a home, so
/// the copy's database is the shared thing: opening it from two servers would put both on one file,
/// and the second would find records where an evaluation load expects an empty store.
#[test]
fn evaluate_refuses_a_label_another_server_already_holds() {
    let (published, _) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut first = a_server(&home);
    let loaded = sets(
        &mut first,
        json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
    );
    assert!(!is_error(&loaded), "{}", message(&loaded));

    // A second server over the same home, which has not loaded that review.
    let mut second = a_server(&home);
    let refused = sets(
        &mut second,
        json!({"action": "evaluate", "label": "issue-3", "set": "scatterhoard", "directory": published.path()}),
    );

    assert!(is_error(&refused));
    assert!(
        message(&refused).contains("issue-3") && message(&refused).contains("drop"),
        "the refusal should name the label and the way out: {}",
        message(&refused)
    );
}

// ---------------------------------------------------------------------------
// A file whose set identity is not the set's own
//
// `an-import-silently-keeps-a-different-set-identity`, then `identity-mismatch-refusal`. Until
// 2026-09-22 an import kept the store's identity whatever the file declared and said nothing; then
// it said so and imported anyway. Since 2026-09-26 a mismatch is refused wherever the two meet -- an
// import, and an export that would write over the file -- naming both identities and the way out of
// each case that produces one. A file with records and no identity keeps its note.
// ---------------------------------------------------------------------------

/// A set named `notes` created on its own machine, holding one memory, exported to `out`.
fn a_notes_set_exported_to(out: &TempDir) -> Server {
    let home = tempdir();
    let mut server = a_server(&home);
    std::mem::forget(home);
    create_chartered(&mut server, "notes");
    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": "shared:notes", "content": "alpha written here", "why": "a fixture",
               "confidence": 0.8, "origin": "llm"}),
    );
    assert!(!is_error(&stored), "{stored}");
    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    );
    assert!(!is_error(&exported), "{exported}");
    server
}

fn set_id_in_file(out: &TempDir) -> String {
    let text = std::fs::read_to_string(memories_file(out)).expect("reads");
    let header: Value = serde_json::from_str(text.lines().next().expect("a header")).expect("JSON");
    header["set_id"].as_str().expect("a set id").to_owned()
}

/// Two sets that share a name, each with its own identity. `theirs` holds the other `notes` in
/// `ours`'s checkout's place, so the directory form finds it by name and meets the mismatch.
fn two_notes_sets() -> (TempDir, String, TempDir, String, Server) {
    let theirs = tempdir();
    a_notes_set_exported_to(&theirs);
    let their_id = set_id_in_file(&theirs);
    let ours = tempdir();
    let server = a_notes_set_exported_to(&ours);
    let our_id = set_id_in_file(&ours);
    (theirs, their_id, ours, our_id, server)
}

/// The routes, named in the refusal: an intended merge, a diverged copy, and a genuine collision --
/// for which the alias is not built, and the refusal says so rather than naming a call that does not
/// exist.
fn names_the_ways_out(said: &str) {
    for route in ["absorb", "drop", "create", "alias"] {
        assert!(said.contains(route), "should name `{route}`: {said}");
    }
    assert!(
        said.contains("not built"),
        "should say the alias is not built yet: {said}"
    );
}

#[test]
fn an_import_refuses_a_file_declaring_another_identity() {
    let (theirs, their_id, _ours, our_id, mut server) = two_notes_sets();
    let before = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["alpha"], "scope": "shared:notes"}),
    ));

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "shared:notes", "directory": theirs.path()}),
    );

    assert!(is_error(&imported), "refused, not reported: {imported}");
    let said = message(&imported);
    assert!(
        said.contains(&their_id) && said.contains(&our_id),
        "both identities should be named: {said}"
    );
    names_the_ways_out(&said);
    let after = payload(&call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["alpha"], "scope": "shared:notes"}),
    ));
    assert_eq!(before, after, "nothing was imported");
}

#[test]
fn an_import_from_a_file_refuses_another_identity_too() {
    let (theirs, _, _ours, _, mut server) = two_notes_sets();
    let outside = tempdir();
    let file = outside.path().join("notes.json");
    std::fs::copy(reserved(&theirs, "notes.json"), &file).expect("copies");

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "shared:notes", "file": file}),
    );

    assert!(is_error(&imported), "{imported}");
}

#[test]
fn an_import_says_when_a_file_with_records_declares_no_identity() {
    let out = tempdir();
    a_notes_set_exported_to(&out);
    let path = memories_file(&out);
    let text = std::fs::read_to_string(&path).expect("reads");
    let (header, rest) = text.split_once('\n').expect("a header");
    let mut header: Value = serde_json::from_str(header).expect("JSON");
    header.as_object_mut().expect("an object").remove("set_id");
    std::fs::write(&path, format!("{header}\n{rest}")).expect("writes");

    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "notes");
    // Written to first, so the set holds an identity the file cannot confirm: the note is that it
    // keeps it. A set with none would mint one on this import, which is the other note.
    store_in(&mut server, "notes", "gamma, written here first");
    let held = identity_of(&mut server, "notes");
    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "shared:notes", "directory": out.path()}),
    );

    assert!(!is_error(&imported), "{imported}");
    let note = payload(&imported)["identity_note"]
        .as_str()
        .unwrap_or_else(|| panic!("no identity note: {imported}"))
        .to_owned();
    assert!(
        note.contains("no set identity") && note.contains(&format!("keeps {held}")),
        "{note}"
    );
}

#[test]
fn an_import_that_agrees_or_adopts_says_nothing_about_identity() {
    let out = tempdir();
    let mut origin = a_notes_set_exported_to(&out);
    let again = sets(
        &mut origin,
        json!({"action": "import", "scope": "shared:notes", "directory": out.path()}),
    );
    assert!(payload(&again).get("identity_note").is_none(), "{again}");

    let home = tempdir();
    let mut fresh = a_server(&home);
    sets(&mut fresh, json!({"action": "create", "name": "notes"}));
    let adopted = sets(
        &mut fresh,
        json!({"action": "import", "scope": "shared:notes", "directory": out.path()}),
    );
    assert!(payload(&adopted)["adopted_set_id"].is_string(), "{adopted}");
    assert!(
        payload(&adopted).get("identity_note").is_none(),
        "{adopted}"
    );
}

/// Rewrite the `set_id` in a memory file's header, leaving every record as it was -- so the file
/// differs from the set in identity alone, and no other refusal can be what stops an export.
fn with_identity(path: &std::path::Path, set_id: &str) {
    let text = std::fs::read_to_string(path).expect("reads");
    let (header, rest) = text.split_once('\n').expect("a header");
    let mut header: Value = serde_json::from_str(header).expect("JSON");
    header["set_id"] = json!(set_id);
    std::fs::write(path, format!("{header}\n{rest}")).expect("writes");
}

/// The case refusing was chosen for: a diverged copy's export writing its own identity over the
/// shared file, after which every other collaborator mismatches and only this machine was told.
#[test]
fn an_export_refuses_to_write_over_a_file_declaring_another_identity() {
    let (_theirs, their_id, ours, our_id, mut server) = two_notes_sets();
    let path = reserved(&ours, "notes.json");
    with_identity(&path, &their_id);
    let before = std::fs::read_to_string(&path).expect("reads");

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": ours.path()}),
    );

    assert!(is_error(&exported), "refused, not reported: {exported}");
    let said = message(&exported);
    assert!(
        said.contains(&their_id) && said.contains(&our_id),
        "both identities should be named: {said}"
    );
    names_the_ways_out(&said);
    assert_eq!(
        std::fs::read_to_string(&path).expect("reads"),
        before,
        "a refused export leaves the file as it was"
    );
    assert_eq!(
        json_files_in(&ours).len(),
        1,
        "and writes no second file beside it"
    );
}

#[test]
fn an_export_to_a_file_refuses_another_identity_too() {
    let (_theirs, their_id, ours, _, mut server) = two_notes_sets();
    let outside = tempdir();
    let file = outside.path().join("notes.json");
    std::fs::copy(reserved(&ours, "notes.json"), &file).expect("copies");
    with_identity(&file, &their_id);

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "file": file}),
    );

    assert!(is_error(&exported), "{exported}");
    let header = std::fs::read_to_string(&file).expect("reads");
    assert!(header.contains(&their_id), "the file keeps its identity");
}

/// When the file both declares another identity and holds records this set lacks, the identity is
/// what is refused. The dropped-records refusal advises importing from the destination, and that
/// import is refused too -- advice that leads only to a second refusal.
#[test]
fn an_identity_mismatch_is_refused_ahead_of_records_the_export_would_drop() {
    let (theirs, their_id, _ours, our_id, mut server) = two_notes_sets();

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": theirs.path()}),
    );

    assert!(is_error(&exported), "{exported}");
    let said = message(&exported);
    assert!(
        said.contains(&their_id) && said.contains(&our_id),
        "the identity refusal, not the dropped-records one: {said}"
    );
}

/// **A set with no identity is told to import a file that declares one, not allowed to write over
/// it.** It has never been written to or imported, so the file is almost certainly the set it was
/// created to take on; exporting would replace the file's identity with a new one and fork every
/// copy. And the refused export must leave it without one -- an identity minted and then refused
/// would turn the advised import into a mismatch.
#[test]
fn a_set_with_no_identity_is_told_to_import_a_file_that_declares_one() {
    // **An empty file**, exported by a set that holds nothing: a file with records is refused earlier,
    // for the records the export would drop, and would never reach the identity question.
    let theirs = tempdir();
    let their_home = tempdir();
    let mut origin = a_server(&their_home);
    sets(&mut origin, json!({"action": "create", "name": "notes"}));
    sets(
        &mut origin,
        json!({"action": "export", "scope": "shared:notes", "directory": theirs.path()}),
    );
    let their_id = set_id_in_file(&theirs);
    let home = tempdir();
    let mut fresh = a_server(&home);
    sets(&mut fresh, json!({"action": "create", "name": "notes"}));

    let exported = sets(
        &mut fresh,
        json!({"action": "export", "scope": "shared:notes", "directory": theirs.path()}),
    );

    assert!(is_error(&exported), "{exported}");
    assert!(
        message(&exported).contains(&their_id),
        "{}",
        message(&exported)
    );
    assert!(
        message(&exported).contains("import"),
        "{}",
        message(&exported)
    );
    let imported = sets(
        &mut fresh,
        json!({"action": "import", "scope": "shared:notes", "directory": theirs.path()}),
    );
    assert!(!is_error(&imported), "and its advice works: {imported}");
    assert_eq!(identity_of(&mut fresh, "notes"), their_id);
}

/// **An identity is defended whether or not the set has written under it**, since
/// `set-settled-at-first-write`: an import no longer displaces the identity an empty set holds, so an
/// export over another set's file has to be refused on identity, or it would replace that set's.
#[test]
fn an_empty_set_holding_an_identity_defends_it_on_export() {
    let (theirs, their_id, _ours, _, _server) = two_notes_sets();
    let home = tempdir();
    let own = tempdir();
    let mut fresh = a_server(&home);
    sets(&mut fresh, json!({"action": "create", "name": "notes"}));
    let first = sets(
        &mut fresh,
        json!({"action": "export", "scope": "shared:notes", "directory": own.path()}),
    );
    assert!(!is_error(&first), "{}", message(&first));
    let ours = identity_of(&mut fresh, "notes");

    let exported = sets(
        &mut fresh,
        json!({"action": "export", "scope": "shared:notes", "directory": theirs.path()}),
    );

    assert!(is_error(&exported), "{exported}");
    let said = message(&exported);
    assert!(
        said.contains(&their_id) && said.contains(&ours),
        "the identity refusal: {said}"
    );
}

/// **Importing an empty file is not a set's first write**, so it settles no identity -- otherwise the
/// set would be fixed under one nobody else holds before the file that carries its own arrived.
#[test]
fn importing_an_empty_file_declaring_no_identity_settles_none() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    add_a_set_with_no_identity(&mut server, "main");
    let path = reserved(&out, "main.json");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("creates");
    let header = json!({
        "format_version": scatterhoard::memory_file::FORMAT_VERSION,
        "scope": {"shared": "main"},
    });
    std::fs::write(&path, format!("{header}\n")).expect("writes");

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": {"shared": "main"}, "directory": out.path()}),
    );

    assert!(!is_error(&imported), "{}", message(&imported));
    let listed = payload(&sets(&mut server, json!({"action": "status"})));
    let main = listed["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .find(|set| set["scope"] == json!("shared:main"))
        .expect("the set is loaded")
        .clone();
    assert!(main["set_id"].is_null(), "{main}");
}

/// **An empty set's first export gives it its identity**, and the file carries it -- otherwise a
/// copy taken from that file mints its own on first write, and the two can never exchange records.
#[test]
fn exporting_a_set_with_no_identity_gives_it_one_the_file_carries() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "main"}));

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": {"shared": "main"}, "directory": out.path()}),
    );

    assert!(!is_error(&exported), "{}", message(&exported));
    let text = std::fs::read_to_string(memories_file(&out)).expect("the exported file");
    let header: Value = serde_json::from_str(text.lines().next().expect("a header")).expect("json");
    assert_eq!(header["set_id"], json!(identity_of(&mut server, "main")));
}

// ---------------------------------------------------------------------------
// Several sets in one directory
//
// `several-memory-files-per-directory`, `documents/design/set-lifecycle.md`, *several sets in one
// directory*. A `scatterhoard_mem` directory holds one `.json` per set. The `directory` form finds a
// set's file by the `set_id` in each header, falling back to the declared name, so a file may be
// called anything and a hand rename costs nothing. A new file is named after its set, in a form
// every operating system accepts.
// ---------------------------------------------------------------------------

/// A set called `name`, created on its own machine, holding one memory, exported to `out`.
fn a_set_exported_to(name: &str, out: &TempDir) -> Server {
    let home = tempdir();
    let mut server = a_server(&home);
    std::mem::forget(home);
    let created = sets(&mut server, json!({"action": "create", "name": name}));
    write_charter(&mut server, name);
    assert!(!is_error(&created), "{created}");
    let scope = format!("shared:{name}");
    let stored = call(
        &mut server,
        "hoard_memorize",
        json!({"scope": scope, "content": format!("a memory of {name}"), "why": "a fixture",
               "confidence": 0.8, "origin": "llm"}),
    );
    assert!(!is_error(&stored), "{stored}");
    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": scope, "directory": out.path()}),
    );
    assert!(!is_error(&exported), "{exported}");
    server
}

fn reserved(out: &TempDir, file: &str) -> std::path::PathBuf {
    out.path().join("scatterhoard_mem").join(file)
}

#[test]
fn a_new_file_is_named_after_its_set() {
    let out = tempdir();
    a_set_exported_to("notes", &out);

    assert_eq!(json_files_in(&out), vec![reserved(&out, "notes.json")]);
}

#[test]
fn two_sets_export_into_one_directory_side_by_side() {
    let out = tempdir();
    a_set_exported_to("notes", &out);
    a_set_exported_to("ideas", &out);

    assert_eq!(
        json_files_in(&out),
        vec![reserved(&out, "ideas.json"), reserved(&out, "notes.json")]
    );
}

#[test]
fn each_of_two_sets_in_one_directory_imports_its_own_file() {
    let out = tempdir();
    a_set_exported_to("notes", &out);
    a_set_exported_to("ideas", &out);

    let home = tempdir();
    let mut server = a_server(&home);
    for name in ["notes", "ideas"] {
        sets(&mut server, json!({"action": "create", "name": name}));
        let imported = sets(
            &mut server,
            json!({"action": "import", "scope": format!("shared:{name}"), "directory": out.path()}),
        );
        assert!(!is_error(&imported), "{imported}");
        assert!(
            payload(&imported)["read"]
                .as_str()
                .is_some_and(|read| read.ends_with(&format!("{name}.json"))),
            "{imported}"
        );
        // The memory and the set's identity record.
        assert_eq!(
            payload(&imported)["outcomes"]["new_from_file"],
            json!(3),
            "{imported}"
        );
    }
}

#[test]
fn export_writes_to_a_renamed_file_found_by_identity() {
    let out = tempdir();
    let mut server = a_set_exported_to("notes", &out);
    std::fs::rename(
        reserved(&out, "notes.json"),
        reserved(&out, "jottings.json"),
    )
    .expect("renames");
    store_into(&mut server, "notes", "a second memory");

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    );

    assert!(!is_error(&exported), "{exported}");
    assert_eq!(json_files_in(&out), vec![reserved(&out, "jottings.json")]);
    // One more than the test's own writes: the set's charter, which its first memory has to be.
    assert_eq!(payload(&exported)["memories"], json!(3));
}

#[test]
fn import_reads_a_renamed_file_found_by_identity() {
    let out = tempdir();
    let mut origin = a_set_exported_to("notes", &out);
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "notes"}));
    sets(
        &mut server,
        json!({"action": "import", "scope": "shared:notes", "directory": out.path()}),
    );
    store_into(&mut origin, "notes", "a second memory");
    sets(
        &mut origin,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    );
    std::fs::rename(
        reserved(&out, "notes.json"),
        reserved(&out, "jottings.json"),
    )
    .expect("renames");

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "shared:notes", "directory": out.path()}),
    );

    assert!(!is_error(&imported), "{imported}");
    assert_eq!(
        payload(&imported)["outcomes"]["new_from_file"],
        json!(1),
        "{imported}"
    );
}

/// A file declaring the set's name is not its file when another carries its identity. A rename
/// alone would not show this, since the renamed file's header still declares the name.
#[test]
fn the_file_carrying_the_sets_identity_wins_over_one_sharing_its_name() {
    let out = tempdir();
    let mut server = a_set_exported_to("notes", &out);
    let theirs = tempdir();
    a_set_exported_to("notes", &theirs);
    std::fs::copy(
        reserved(&theirs, "notes.json"),
        reserved(&out, "their-notes.json"),
    )
    .expect("copies");
    store_into(&mut server, "notes", "a second memory");

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    );

    assert!(!is_error(&exported), "{exported}");
    assert!(
        payload(&exported)["written"]
            .as_str()
            .is_some_and(|written| written.ends_with("/notes.json")),
        "{exported}"
    );
}

#[test]
fn import_says_when_the_directory_holds_no_file_for_the_set() {
    let out = tempdir();
    a_set_exported_to("ideas", &out);
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "notes"}));

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "shared:notes", "directory": out.path()}),
    );

    assert!(is_error(&imported), "{imported}");
    let said = message(&imported);
    assert!(
        said.contains("notes") && said.contains("ideas.json"),
        "should name the set sought and what the directory holds: {said}"
    );
}

#[test]
fn two_files_answering_to_one_set_are_refused_on_export_and_import() {
    let out = tempdir();
    let mut server = a_set_exported_to("notes", &out);
    std::fs::copy(reserved(&out, "notes.json"), reserved(&out, "copy.json")).expect("copies");

    for action in ["export", "import"] {
        let response = sets(
            &mut server,
            json!({"action": action, "scope": "shared:notes", "directory": out.path()}),
        );
        assert!(is_error(&response), "{action}: {response}");
        let said = message(&response);
        assert!(
            said.contains("notes.json") && said.contains("copy.json"),
            "{action} should name both files: {said}"
        );
    }
}

/// A `.json` whose header cannot be read might be this set's own file, damaged -- so it is named
/// rather than skipped, which would write a second file for the set beside it.
#[test]
fn an_unreadable_memory_file_is_named_rather_than_skipped() {
    let out = tempdir();
    let mut server = a_set_exported_to("notes", &out);
    std::fs::write(reserved(&out, "broken.json"), "<<<<<<< HEAD\n").expect("writes");

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    );

    assert!(is_error(&exported), "{exported}");
    assert!(message(&exported).contains("broken.json"), "{exported}");
}

/// macOS writes `._name` beside a file on a volume without extended attributes, and it is not a
/// memory file whatever its extension says.
#[test]
fn a_hidden_file_in_the_reserved_directory_is_not_a_memory_file() {
    let out = tempdir();
    let mut server = a_set_exported_to("notes", &out);
    std::fs::write(reserved(&out, "._notes.json"), [0u8, 5, 22, 7]).expect("writes");

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    );

    assert!(!is_error(&exported), "{exported}");
}

#[test]
fn a_windows_reserved_set_name_takes_a_suffix() {
    let out = tempdir();
    a_set_exported_to("con", &out);
    a_set_exported_to("NUL.notes", &out);

    assert_eq!(
        json_files_in(&out),
        vec![
            reserved(&out, "NUL-set.notes.json"),
            reserved(&out, "con-set.json")
        ]
    );
}

/// `Notes` and `notes` are two sets and one file on a case-insensitive filesystem, so the second
/// takes a number rather than overwrite the first on a Mac. The lowercase one goes first, so a
/// comparison that kept case would see `Notes.json` as free.
#[test]
fn a_file_name_taken_ignoring_case_takes_a_number() {
    let out = tempdir();
    a_set_exported_to("notes", &out);
    a_set_exported_to("Notes", &out);

    assert_eq!(
        json_files_in(&out),
        vec![reserved(&out, "Notes-2.json"), reserved(&out, "notes.json")]
    );
}

/// `absorb` and `evaluate` read a set this machine does not hold under that name, so in `directory`
/// form the request names it and the headers are searched for it.
#[test]
fn evaluate_reads_the_named_set_from_a_directory_holding_several() {
    let out = tempdir();
    a_set_exported_to("notes", &out);
    a_set_exported_to("ideas", &out);
    let home = tempdir();
    let mut server = a_server(&home);

    let evaluated = sets(
        &mut server,
        json!({"action": "evaluate", "label": "review", "set": "ideas", "directory": out.path()}),
    );

    assert!(!is_error(&evaluated), "{evaluated}");
    assert_eq!(
        payload(&evaluated)["copy_of"],
        json!("shared:ideas"),
        "{evaluated}"
    );
}

/// Required even with one file there, so a request that works today cannot start failing when a
/// second set is exported beside it.
#[test]
fn evaluate_from_a_directory_needs_the_set_named() {
    let out = tempdir();
    a_set_exported_to("notes", &out);
    let home = tempdir();
    let mut server = a_server(&home);

    let evaluated = sets(
        &mut server,
        json!({"action": "evaluate", "label": "review", "directory": out.path()}),
    );

    assert!(is_error(&evaluated), "{evaluated}");
    assert!(message(&evaluated).contains("`set`"), "{evaluated}");
}

#[test]
fn evaluate_says_when_the_directory_holds_no_file_for_the_named_set() {
    let out = tempdir();
    a_set_exported_to("notes", &out);
    let home = tempdir();
    let mut server = a_server(&home);

    let evaluated = sets(
        &mut server,
        json!({"action": "evaluate", "label": "review", "set": "ideas", "directory": out.path()}),
    );

    assert!(is_error(&evaluated), "{evaluated}");
    let said = message(&evaluated);
    assert!(
        said.contains("ideas") && said.contains("notes.json"),
        "{said}"
    );
}

/// With `file` the path says which file, and a name given as well has to agree with its header.
#[test]
fn evaluate_from_a_file_checks_a_named_set_against_the_header() {
    let out = tempdir();
    a_set_exported_to("notes", &out);
    let home = tempdir();
    let mut server = a_server(&home);

    let wrong = sets(
        &mut server,
        json!({"action": "evaluate", "label": "review", "set": "ideas",
               "file": reserved(&out, "notes.json").to_string_lossy()}),
    );
    // The file form still refuses a path inside `scatterhoard_mem`, so read a copy from outside.
    let outside = tempdir();
    std::fs::copy(
        reserved(&out, "notes.json"),
        outside.path().join("notes.json"),
    )
    .expect("copies");
    let mismatched = sets(
        &mut server,
        json!({"action": "evaluate", "label": "review", "set": "ideas",
               "file": outside.path().join("notes.json")}),
    );
    let matched = sets(
        &mut server,
        json!({"action": "evaluate", "label": "review", "set": "notes",
               "file": outside.path().join("notes.json")}),
    );

    assert!(is_error(&wrong), "inside the reserved directory: {wrong}");
    assert!(is_error(&mismatched), "{mismatched}");
    assert!(!is_error(&matched), "{matched}");
}

#[test]
fn absorb_reads_the_named_set_from_a_directory_holding_several() {
    let out = tempdir();
    a_set_exported_to("notes", &out);
    a_set_exported_to("ideas", &out);
    let home = tempdir();
    let mut server = a_server(&home);
    sets(&mut server, json!({"action": "create", "name": "main"}));
    // Absorbing authors supersessions, so it is a write, and a shared set asks who writes first.
    name_author(&mut server, "main", FIXTURE_AUTHOR);
    let copy = tempdir();

    let absorbed = sets(
        &mut server,
        json!({"action": "absorb", "scope": "shared:main", "set": "ideas", "directory": out.path(),
               "save_to": copy.path().join("before.json")}),
    );

    assert!(!is_error(&absorbed), "{absorbed}");
    assert_eq!(
        payload(&absorbed)["absorbed"],
        json!("shared:ideas"),
        "{absorbed}"
    );
    // One more than the test's own writes: the set's charter, which its first memory has to be.
    assert_eq!(payload(&absorbed)["memories"], json!(2), "{absorbed}");
}

// ---------------------------------------------------------------------------
// The reserved directory holds shared sets only
//
// `the-reserved-directory-accepts-the-local-set`. The path settles a file's kind: whatever sits in a
// `scatterhoard_mem` is a shared set, data and never instructions. So `local` is refused there in
// every direction -- an export would put personal memories where the next commit publishes them,
// and an import or restore would let a crafted checkout write into the personal set. `file` is how
// `local` is backed up.
// ---------------------------------------------------------------------------

#[test]
fn the_local_set_is_not_exported_into_a_reserved_directory() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is personal");

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "local", "directory": out.path()}),
    );

    assert!(is_error(&exported), "{exported}");
    assert!(message(&exported).contains("`file`"), "{exported}");
    assert!(
        !out.path().join("scatterhoard_mem").exists(),
        "nothing is written, not even the directory"
    );
}

/// The file a crafted checkout would carry: `local` declared in its reserved directory.
#[test]
fn the_local_set_is_not_imported_or_restored_from_a_reserved_directory() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is personal");
    let backup = out.path().join("local.json");
    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": backup}),
    );
    assert!(!is_error(&exported), "{exported}");
    std::fs::create_dir_all(out.path().join("scatterhoard_mem")).expect("creates");
    std::fs::copy(&backup, reserved(&out, "local.json")).expect("copies");
    let copy = tempdir();

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "local", "directory": out.path()}),
    );
    let restored = sets(
        &mut server,
        json!({"action": "restore", "scope": "local", "directory": out.path(),
               "save_to": copy.path().join("before.json")}),
    );

    assert!(is_error(&imported), "{imported}");
    assert!(is_error(&restored), "{restored}");
    assert!(
        !copy.path().join("before.json").exists(),
        "refused before the safety copy"
    );
}

#[test]
fn the_local_set_round_trips_through_a_file() {
    let home = tempdir();
    let out = tempdir();
    let mut server = a_server(&home);
    store_one(&mut server, "alpha is personal");
    let backup = out.path().join("local.json");

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": backup}),
    );
    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "local", "file": backup}),
    );

    assert!(!is_error(&exported), "{exported}");
    assert!(!is_error(&imported), "{imported}");
}

// ---------------------------------------------------------------------------
// Supersession says which in-links the walk will re-point
// ---------------------------------------------------------------------------
//
// `supersession-silently-repoints-an-in-link-nobody-reauthored`: recall walks an edge into a
// superseded memory as an edge into its replacement, so `C SUPPORTS A` is followed as `C SUPPORTS B`
// once B supersedes A -- a claim C's author made about A's text and never about B's. The records
// stay intact, so the relationship exists only in the walk, with nothing to inspect. The remedy is a
// remark at the one moment someone holding both texts can act.

/// Store a memory into `set` with these links, returning its id.
fn written_into(server: &mut Server, set: &str, content: &str, links: Value) -> String {
    let response = call(
        server,
        "hoard_memorize",
        json!({
            "scope": {"shared": set},
            "content": content,
            "confidence": 0.7,
            "origin": "llm",
            "keywords": [],
            "links": links,
        }),
    );
    assert!(!is_error(&response), "memorize should succeed: {response}");
    payload(&response)["id"].as_str().expect("an id").to_owned()
}

#[test]
fn superseding_a_memory_names_the_in_links_the_walk_will_repoint() {
    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "ours");
    create_chartered(&mut server, "theirs");

    let old = written_into(&mut server, "ours", "alpha governs routing", json!([]));
    let here = written_into(
        &mut server,
        "ours",
        "beta measured it",
        json!([{"label": "supports", "target": &old}]),
    );
    let there = written_into(
        &mut server,
        "theirs",
        "beta elsewhere relies on it",
        json!([{"label": "related_to", "target": &old, "target_set": "ours"}]),
    );

    let replaced = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "ours"},
            "content": "alpha governs recall",
            "confidence": 0.7,
            "origin": "llm",
            "keywords": [],
            "links": [{"label": "supersedes", "target": &old}],
        }),
    );
    assert!(!is_error(&replaced), "{replaced}");
    let result = payload(&replaced);

    let repointed = result["repointed_in_links"]
        .as_array()
        .unwrap_or_else(|| panic!("the in-links are named: {result}"));
    assert!(
        repointed.contains(&json!({"superseded": old, "source": here, "label": "supports"})),
        "the same-set in-link, with its label: {result}"
    );
    assert!(
        repointed.contains(&json!({
            "superseded": old,
            "source": there,
            "label": "related_to",
            "scope": "shared:theirs",
        })),
        "and the one from another loaded set, with its set: {result}"
    );
    let note = result["repointed_in_links_note"]
        .as_str()
        .expect("a note says what the list means");
    assert!(
        note.contains("this memory"),
        "the note says the links now reach the replacement: {note}"
    );
}

/// Chain structure is not a claim: an in-link that is itself a supersession is the history the walk
/// is supposed to follow, so a memory pointed at only by its chain is superseded quietly. Both
/// directions are present -- an older memory conceding to it with `superseded_by`, and a sibling
/// replacement written with `supersedes` -- so the filter is what keeps this quiet, not an absence.
#[test]
fn superseding_a_memory_only_chained_to_says_nothing_about_in_links() {
    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "ours");

    let current = written_into(&mut server, "ours", "alpha governs routing", json!([]));
    written_into(
        &mut server,
        "ours",
        "alpha governed routing once",
        json!([{"label": "superseded_by", "target": &current}]),
    );
    written_into(
        &mut server,
        "ours",
        "alpha governs recall, one half of a split",
        json!([{"label": "supersedes", "target": &current}]),
    );
    let other_half = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": {"shared": "ours"},
            "content": "alpha governs ranking, the other half",
            "confidence": 0.7,
            "origin": "llm",
            "keywords": [],
            "links": [{"label": "supersedes", "target": &current}],
        }),
    );
    let result = payload(&other_half);
    assert!(
        result.get("repointed_in_links").is_none()
            && result.get("repointed_in_links_note").is_none(),
        "only supersession links point at the superseded memory: {result}"
    );
}

// ---------------------------------------------------------------------------
// Set names differing only in case
// ---------------------------------------------------------------------------

/// **Names differing only in case are one name.** `set-names-differing-only-in-case-share-a-store-file`:
/// `notes` and `Notes` are distinct `SetName`s but one store file on a case-insensitive filesystem,
/// the default on macOS and Windows. Creating the second was refused on this machine as an unloaded
/// database -- with a remedy, restart, that loads the same file under the first spelling -- and
/// allowed on Linux, where the two sets then collide the moment the data directory moves. Refused
/// everywhere now, naming the set that holds the name, and tested by taking the remedy it gives.
#[test]
fn creating_a_set_differing_only_in_case_from_a_loaded_one_is_refused() {
    let home = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "notes");

    let refused = sets(&mut server, json!({"action": "create", "name": "Notes"}));
    assert!(is_error(&refused), "{refused}");
    let text = message(&refused);
    assert!(
        text.contains("\"notes\"") && text.contains("case"),
        "the refusal names the loaded set in its own spelling, and why: {text}"
    );
    assert!(
        !text.contains("not loaded"),
        "and does not send the caller looking for an unloaded database: {text}"
    );

    let accepted = sets(
        &mut server,
        json!({"action": "create", "name": "notes-two"}),
    );
    assert!(
        !is_error(&accepted),
        "a name differing by more than case is accepted: {accepted}"
    );
}

/// The same for a copy loaded for review, whose name is built from its label: two labels differing
/// only in case would put two reviews in one file.
#[test]
fn evaluate_refuses_a_label_differing_only_in_case_from_one_loaded() {
    let (published, _) = a_published_set("scatterhoard");
    let home = tempdir();
    let mut server = a_server(&home);
    let first = sets(
        &mut server,
        json!({"action": "evaluate", "label": "branch", "set": "scatterhoard", "directory": published.path()}),
    );
    assert!(!is_error(&first), "{first}");

    let refused = sets(
        &mut server,
        json!({"action": "evaluate", "label": "Branch", "set": "scatterhoard", "directory": published.path()}),
    );
    assert!(is_error(&refused), "{refused}");
    let text = message(&refused);
    assert!(
        text.contains("branch") && text.contains("case"),
        "the refusal names the loaded review and why: {text}"
    );
}

/// A withheld set still owns its file, so a name differing from it only in case is refused too --
/// rather than refused as an unloaded database on one platform and created beside it on another.
#[test]
fn creating_a_set_differing_only_in_case_from_a_withheld_one_is_refused() {
    let home = tempdir();
    let set = |name: &str| Scope::Shared(SetName::new(name).expect("valid set name"));
    {
        let original = Store::open(set("original"), home.path()).expect("opens");
        let identity = original.mint_set_id().expect("mints");
        let renamed = Store::open(set("renamed"), home.path()).expect("opens");
        assert!(renamed.adopt_set_id(identity).expect("adopts"));
    }
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    for name in ["renamed", "original"] {
        server.add_store(Store::open(set(name), home.path()).expect("opens"));
    }

    let refused = sets(&mut server, json!({"action": "create", "name": "Original"}));
    assert!(is_error(&refused), "{refused}");
    let text = message(&refused);
    assert!(
        text.contains("\"original\"") && text.contains("case") && !text.contains("not loaded"),
        "the withheld set holding the name is the one named: {text}"
    );
}

// ---------------------------------------------------------------------------
// A personal store split across two directories is merged back by absorb
// ---------------------------------------------------------------------------
//
// `a-split-local-store-cannot-be-merged-back`: setting `SCATTERHOARD_COMMON` without moving
// `local.sqlite3` leaves two `local` stores on one machine, each with its own identity. `import` of
// one into the other is refused for the identity and advises `absorb`, which refused every `local`
// file -- so the advice led to a second refusal and the split had no way back.

/// Write one memory into `local` and return its id.
fn into_local(server: &mut Server, content: &str) -> String {
    let response = call(
        server,
        "hoard_memorize",
        json!({"scope": "local", "content": content, "confidence": 0.9, "origin": "human"}),
    );
    assert!(!is_error(&response), "{response}");
    payload(&response)["id"].as_str().expect("an id").to_owned()
}

/// **Tested by taking the remedy the refusal names.** The stray store's export is imported as the
/// refusal's own route expects a caller to try first, and then absorbed as its advice says.
#[test]
fn a_split_local_store_is_merged_back_by_the_absorb_imports_refusal_names() {
    let stray_home = tempdir();
    let mut stray = a_server(&stray_home);
    let stray_memory = into_local(&mut stray, "alpha written into the wrong store");
    let backups = tempdir();
    let file = backups.path().join("stray-local.json");
    let exported = sets(
        &mut stray,
        json!({"action": "export", "scope": "local", "file": file}),
    );
    assert!(!is_error(&exported), "{exported}");

    let home = tempdir();
    let mut server = a_server(&home);
    let belongs = into_local(&mut server, "beta written where it belongs");
    // A same-set link in the receiving store. A `local` file names no set, and a same-set link names
    // none either, so a predicate that compared the two would read this as a link into the absorbed
    // set and supersede the memory holding it.
    let linking = call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local",
            "content": "beta again, pointing at the first",
            "confidence": 0.9,
            "origin": "human",
            "links": [{"label": "related_to", "target": belongs}],
        }),
    );
    assert!(!is_error(&linking), "{linking}");

    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "local", "file": file}),
    );
    assert!(is_error(&imported), "two identities: {imported}");
    assert!(
        message(&imported).contains("absorb"),
        "the refusal names absorb as the way to take this file in: {}",
        message(&imported)
    );

    let absorbed = sets(
        &mut server,
        json!({
            "action": "absorb",
            "scope": "local",
            "file": file,
            "save_to": backups.path().join("before-absorb.json"),
        }),
    );
    assert!(
        !is_error(&absorbed),
        "the remedy the refusal named works: {absorbed}"
    );
    let report = payload(&absorbed);
    assert_eq!(
        report["superseded"],
        json!([]),
        "nothing links into `local` from another set, so nothing is superseded: {report}"
    );
    assert!(
        report.get("still_loaded").is_none(),
        "`local` is not told to drop itself: {report}"
    );

    let found = payload(&call(
        &mut server,
        "hoard_lookup",
        json!({"id": stray_memory}),
    ));
    assert_eq!(
        found["memories"][0]["content"],
        json!("alpha written into the wrong store"),
        "the stray memory is back in the personal store: {found}"
    );
}

/// For `local` the same set is told by identity, since every `local` file declares the same scope:
/// absorbing a store's own export is reconciling it, which is import's job.
#[test]
fn absorbing_locals_own_export_into_local_is_refused_as_the_same_set() {
    let home = tempdir();
    let mut server = a_server(&home);
    into_local(&mut server, "alpha written where it belongs");
    let backups = tempdir();
    let file = backups.path().join("local.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file}),
    );

    let refused = sets(
        &mut server,
        json!({
            "action": "absorb",
            "scope": "local",
            "file": file,
            "save_to": backups.path().join("before-absorb.json"),
        }),
    );
    assert!(is_error(&refused), "{refused}");
    assert!(
        message(&refused).contains("import"),
        "the refusal points at reconciling instead: {}",
        message(&refused)
    );
}

/// Personal memories never go into a shared set this way: a `local` file is still refused there.
#[test]
fn a_local_file_is_not_absorbed_into_a_shared_set() {
    let home = tempdir();
    let mut server = a_server(&home);
    into_local(&mut server, "alpha a personal preference");
    let backups = tempdir();
    let file = backups.path().join("local.json");
    sets(
        &mut server,
        json!({"action": "export", "scope": "local", "file": file}),
    );
    create_chartered(&mut server, "team");

    let refused = sets(
        &mut server,
        json!({
            "action": "absorb",
            "scope": "shared:team",
            "file": file,
            "save_to": backups.path().join("before-absorb.json"),
        }),
    );
    assert!(
        is_error(&refused),
        "a personal file stays out of a shared set: {refused}"
    );
}

// ---------------------------------------------------------------------------
// Taking on a published set from a directory shared with another set's file
// ---------------------------------------------------------------------------
//
// `scatterhoard-cookbook` is published from this repository's `scatterhoard_mem`, beside
// `scatterhoard.json`, so a user reaches it two ways: by downloading its file alone, or by cloning a
// repository whose directory holds both. Each route ends in taking it on read-only -- `create` with
// `mode: read-only`, then `import` -- and neither was pinned with a neighbouring file present.

/// A publisher's directory holding two sets' files: `project`, and `cookbook` beside it. Returns
/// the directory and the id of the cookbook's one recipe.
fn a_directory_holding_two_sets() -> (TempDir, String) {
    let home = tempdir();
    let out = tempdir();
    let mut publisher = a_server(&home);
    for name in ["project", "cookbook"] {
        create_chartered(&mut publisher, name);
    }
    store_in(&mut publisher, "project", "alpha the project's own memory");
    let recipe = store_in(&mut publisher, "cookbook", "alpha a recipe for users");
    for name in ["project", "cookbook"] {
        let exported = sets(
            &mut publisher,
            json!({"action": "export", "scope": format!("shared:{name}"), "directory": out.path()}),
        );
        assert!(!is_error(&exported), "{exported}");
    }
    assert!(
        reserved(&out, "project.json").exists() && reserved(&out, "cookbook.json").exists(),
        "the fixture needs both files in one directory"
    );
    (out, recipe)
}

/// What a recall by meaning returns from `set`, by id.
fn recalled_ids(server: &mut Server, set: &str) -> Vec<Value> {
    let found = payload(&call(
        server,
        "hoard_recall",
        json!({"query": "alpha", "scope": format!("shared:{set}")}),
    ));
    found["memories"]
        .as_array()
        .unwrap_or_else(|| panic!("a list: {found}"))
        .iter()
        .map(|memory| memory["id"].clone())
        .collect()
}

/// **Downloaded**: the cookbook's file alone, copied out of the directory it was published in, is
/// taken on read-only by naming the file -- no checkout, no neighbour.
#[test]
fn a_downloaded_file_is_taken_on_read_only_by_naming_it() {
    let (published, recipe) = a_directory_holding_two_sets();
    let downloads = tempdir();
    let file = downloads.path().join("cookbook.json");
    std::fs::copy(reserved(&published, "cookbook.json"), &file).expect("downloads");

    let home = tempdir();
    let mut reader = a_server(&home);
    let created = sets(
        &mut reader,
        json!({"action": "create", "name": "cookbook", "mode": "read-only"}),
    );
    assert!(!is_error(&created), "{created}");
    let imported = sets(
        &mut reader,
        json!({"action": "import", "scope": "shared:cookbook", "file": file}),
    );
    assert!(
        !is_error(&imported),
        "a downloaded file is taken on: {imported}"
    );

    assert_eq!(recalled_ids(&mut reader, "cookbook"), vec![json!(recipe)]);
    let written = call(
        &mut reader,
        "hoard_memorize",
        json!({"scope": "shared:cookbook", "content": "a note of my own", "confidence": 0.5, "origin": "llm"}),
    );
    assert_refused_as_read_only(&written, "a write to a set taken on read-only");
}

/// **Cloned**: the same set taken on from the directory it was published in, with another set's file
/// beside it. The reader gets the cookbook and nothing of its neighbour.
#[test]
fn a_set_is_taken_on_from_a_directory_holding_another_sets_file() {
    let (published, recipe) = a_directory_holding_two_sets();

    let home = tempdir();
    let mut reader = a_reader_of(&home, "cookbook", &published);

    assert_eq!(
        recalled_ids(&mut reader, "cookbook"),
        vec![json!(recipe)],
        "the cookbook's own recipe, and not the project's memory beside it"
    );
}

// ---------------------------------------------------------------------------
// Sets another server creates: `a-set-created-by-another-server-is-unreachable-until-restart`
// ---------------------------------------------------------------------------

/// **A set another server creates in the shared home is picked up without a restart.** Several
/// servers share one home in ordinary use -- the desktop app's, Claude Code's, a chat's -- and until
/// 2026-10-16 a server learned its sets only at startup, so a set made in one was unreachable from
/// the others: every tool refused it as not loaded, and `create` refused because its file existed.
#[test]
fn a_set_another_server_creates_is_listed_and_usable_without_a_restart() {
    let home = tempdir();
    let mut first = a_server(&home);
    let mut second = a_server(&home);

    create_chartered(&mut first, "made-elsewhere");
    let status = payload(&sets(&mut second, json!({"action": "status"})));
    let found = call(
        &mut second,
        "hoard_recall",
        json!({"keywords": ["hoard-scope-topics"], "scope": "shared:made-elsewhere"}),
    );

    assert!(
        status["sets"]
            .as_array()
            .expect("a list")
            .iter()
            .any(|set| set["scope"] == json!("shared:made-elsewhere")),
        "{status}"
    );
    assert!(!is_error(&found), "{}", message(&found));
    assert_eq!(
        payload(&found)["memories"].as_array().map(Vec::len),
        Some(1),
        "{found}"
    );
}

/// **And it can be dropped from the server that did not create it**, which is where this was found.
#[test]
fn a_set_another_server_creates_can_be_dropped_here() {
    let home = tempdir();
    let mut first = a_server(&home);
    let mut second = a_server(&home);
    create_chartered(&mut first, "made-elsewhere");
    let save_to = home.path().join("made-elsewhere-safety.json");

    let dropped = sets(
        &mut second,
        json!({"action": "drop", "name": "made-elsewhere", "save_to": save_to}),
    );

    assert!(!is_error(&dropped), "{}", message(&dropped));
    assert!(save_to.exists(), "the safety copy was written");
}

/// **Creating a set another server has already made is refused as already loaded**, not as a file
/// to restart for: the rescan before the call has loaded it.
#[test]
fn creating_a_set_another_server_made_is_refused_as_loaded() {
    let home = tempdir();
    let mut first = a_server(&home);
    let mut second = a_server(&home);
    create_chartered(&mut first, "made-elsewhere");

    let refused = sets(
        &mut second,
        json!({"action": "create", "name": "made-elsewhere"}),
    );

    assert!(is_error(&refused), "{refused}");
    assert!(message(&refused).contains("already loaded"), "{refused}");
}

// ---------------------------------------------------------------------------
// Sets another server drops: `a-set-dropped-by-another-server-stays-writable-here`
// ---------------------------------------------------------------------------

/// A memory written into `set` from `server`, as an ordinary note.
fn note_into(server: &mut Server, set: &str, content: &str) -> Value {
    call(
        server,
        "hoard_memorize",
        json!({"scope": {"shared": set}, "content": content, "why": "a fixture",
               "confidence": 0.7, "origin": "llm"}),
    )
}

/// **A set another server drops is not written to here.** An open SQLite database survives its file
/// being deleted, so a server still holding a dropped set would report every write as stored while
/// putting it somewhere nobody can reach again -- silent loss. The file is the shared state, so the
/// server checks it before each call and lets the set go when it has gone.
#[test]
fn a_set_another_server_drops_is_not_written_to_here() {
    let home = tempdir();
    let mut first = a_server(&home);
    let mut second = a_server(&home);
    create_chartered(&mut first, "shared-one");
    let held = note_into(&mut second, "shared-one", "alpha, written while it existed");
    assert!(!is_error(&held), "{}", message(&held));

    let dropped = sets(
        &mut first,
        json!({"action": "drop", "name": "shared-one",
               "save_to": home.path().join("shared-one-safety.json")}),
    );
    assert!(!is_error(&dropped), "{}", message(&dropped));
    let afterwards = note_into(&mut second, "shared-one", "alpha, written after the drop");

    assert!(
        is_error(&afterwards),
        "a write into a dropped set must not report success: {afterwards}"
    );
}

/// **A set dropped and made again elsewhere is written to in its new file**, never the old one: the
/// server compares the file it opened with the one now at the path, not merely whether one is there.
#[test]
fn a_set_dropped_and_recreated_elsewhere_is_written_to_in_its_new_file() {
    let home = tempdir();
    let mut first = a_server(&home);
    let mut second = a_server(&home);
    create_chartered(&mut first, "shared-one");
    let held = note_into(&mut second, "shared-one", "alpha, before");
    assert!(!is_error(&held), "{}", message(&held));
    sets(
        &mut first,
        json!({"action": "drop", "name": "shared-one",
               "save_to": home.path().join("shared-one-safety.json")}),
    );
    create_chartered(&mut first, "shared-one");

    let written = note_into(
        &mut second,
        "shared-one",
        "beta, after the set was made again",
    );
    let mut third = a_server(&home);
    let found = call(
        &mut third,
        "hoard_recall",
        json!({"query": "beta", "scope": "shared:shared-one"}),
    );

    assert!(!is_error(&written), "{}", message(&written));
    assert!(!is_error(&found), "{}", message(&found));
    assert!(
        payload(&found)["memories"]
            .as_array()
            .is_some_and(|memories| memories.iter().any(|memory| memory["content"]
                .as_str()
                .is_some_and(|c| c.contains("beta")))),
        "the write reached the file everyone else reads: {found}"
    );
}

/// **Letting go of a stale set must not disturb the set made again in its place.** Closing the last
/// connection to a database in WAL mode checkpoints and deletes the `-wal` file by path, and after a
/// drop and re-create that path is the new set's, open in the other server. So the stale connection
/// is leaked rather than closed, and the new set's writes on either side of the moment it is let go
/// all reach its file.
#[test]
fn letting_go_of_a_stale_set_leaves_the_new_one_intact() {
    let home = tempdir();
    let mut first = a_server(&home);
    let mut second = a_server(&home);
    create_chartered(&mut first, "shared-one");
    let held = note_into(&mut second, "shared-one", "alpha, before");
    assert!(!is_error(&held), "{}", message(&held));
    sets(
        &mut first,
        json!({"action": "drop", "name": "shared-one",
               "save_to": home.path().join("shared-one-safety.json")}),
    );
    create_chartered(&mut first, "shared-one");
    let before = note_into(
        &mut first,
        "shared-one",
        "gamma, before the stale set is let go",
    );
    assert!(!is_error(&before), "{}", message(&before));

    // Any call makes the second server notice, and let go of, its stale copy.
    sets(&mut second, json!({"action": "status"}));
    let after = note_into(
        &mut first,
        "shared-one",
        "delta, after the stale set is let go",
    );
    assert!(!is_error(&after), "{}", message(&after));

    let mut third = a_server(&home);
    for word in ["gamma", "delta"] {
        let found = call(
            &mut third,
            "hoard_recall",
            json!({"query": word, "scope": "shared:shared-one"}),
        );
        assert!(!is_error(&found), "{}", message(&found));
        assert!(
            payload(&found)["memories"]
                .as_array()
                .is_some_and(|memories| memories
                    .iter()
                    .any(|memory| memory["content"].as_str().is_some_and(|c| c.contains(word)))),
            "{word} reached the new set's file: {found}"
        );
    }
}

// ---------------------------------------------------------------------------
// The default README: `memory-directory-readme`, `readme-names-the-project-url`
// ---------------------------------------------------------------------------

/// **Exporting into a checkout that has no memory directory writes a README beside the set**, so a
/// person meeting `scatterhoard_mem` for the first time finds what it is and where to read more. It
/// describes and never directs: the directory holds no instructions, by design.
#[test]
fn an_export_that_makes_the_memory_directory_writes_the_default_readme() {
    let home = tempdir();
    let checkout = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "documented");

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:documented", "directory": checkout.path()}),
    );

    assert!(!is_error(&exported), "{}", message(&exported));
    let readme = std::fs::read_to_string(checkout.path().join("scatterhoard_mem/README.md"))
        .expect("the README was written");
    assert!(readme.contains("memory sets for Scatterhoard"), "{readme}");
    assert!(
        readme.contains("https://github.com/jtcours/scatterhoard"),
        "{readme}"
    );
}

/// **A memory directory that already exists gets no README**, with one or without: it is the
/// project's, and a file appearing in it unasked would turn up in the next diff.
#[test]
fn an_export_into_an_existing_memory_directory_writes_no_readme() {
    let home = tempdir();
    let checkout = tempdir();
    std::fs::create_dir_all(checkout.path().join("scatterhoard_mem")).expect("makes the directory");
    let mut server = a_server(&home);
    create_chartered(&mut server, "documented");

    let exported = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:documented", "directory": checkout.path()}),
    );

    assert!(!is_error(&exported), "{}", message(&exported));
    assert!(!checkout.path().join("scatterhoard_mem/README.md").exists());
}

/// **A README a project wrote is never changed**, not even to add the project's URL -- decided
/// 2026-10-17 by the human partner.
#[test]
fn an_existing_readme_is_left_exactly_as_it_was() {
    let home = tempdir();
    let checkout = tempdir();
    let directory = checkout.path().join("scatterhoard_mem");
    std::fs::create_dir_all(&directory).expect("makes the directory");
    std::fs::write(directory.join("README.md"), "Our own words.\n").expect("writes ours");
    let mut server = a_server(&home);
    create_chartered(&mut server, "documented");

    sets(
        &mut server,
        json!({"action": "export", "scope": "shared:documented", "directory": checkout.path()}),
    );

    assert_eq!(
        std::fs::read_to_string(directory.join("README.md")).expect("still there"),
        "Our own words.\n"
    );
}

/// **The README is not a memory file**: exporting again and importing from the directory both
/// work with it there.
#[test]
fn the_readme_is_not_read_as_a_memory_file() {
    let home = tempdir();
    let checkout = tempdir();
    let mut server = a_server(&home);
    create_chartered(&mut server, "documented");
    let first = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:documented", "directory": checkout.path()}),
    );
    assert!(!is_error(&first), "{}", message(&first));

    let again = sets(
        &mut server,
        json!({"action": "export", "scope": "shared:documented", "directory": checkout.path()}),
    );
    let imported = sets(
        &mut server,
        json!({"action": "import", "scope": "shared:documented", "directory": checkout.path()}),
    );

    assert!(!is_error(&again), "{}", message(&again));
    assert!(!is_error(&imported), "{}", message(&imported));
}
