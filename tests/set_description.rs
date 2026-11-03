// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the derived set description at the import gate, and for the report that stands
//! in for it when the import could not describe what it brought.
//!
//! `derived-set-summary` in `TRACKING.md`. An import derives what the set amounts to from the
//! memories it received. **That needs vectors**, and a file carries none — so an import onto a
//! machine with no embedding model, or a mismatched one, could describe nothing. Left there, the gate is defeated by importing first and
//! installing the model afterwards. So the description is owed: it is reported at the start of every
//! session once the memories are embedded, until the set is next exported, which shows the
//! description itself at the moment the set becomes portable.
//!
//! **A file carries no description of its own**, since 2026-09-21. It used to, as a claim an import
//! compared its derivation against; the comparison caught only a sender who edited the claim rather
//! than deleting it, and every merge made it fire for no reason. What the reader is shown was always
//! the derivation, so nothing it sees changed (`how-a-merged-file-gets-a-current-summary`).

use std::sync::Arc;

use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::mcp::Server;
use scatterhoard::memory::{Author, Scope};
use scatterhoard::store::{Store, StoreLayout};
use serde_json::{Value, json};

/// A second model, for the mismatch: same space shape, different identity.
#[path = "support/review_all.rs"]
mod review_all;

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
        MarkerEmbedder.embed_each(texts)
    }
}

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
    let path = std::env::temp_dir().join(format!("scatterhoard-description-{unique}"));
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

/// A server holding an empty shared set, `notes`, with or without a model.
///
/// A shared set because these tests exchange it between two machines, and two machines' `local`
/// sets are two sets with an identity each: crossing between them is refused since
/// `identity-mismatch-refusal`. `create` settles no identity: the first machine's export gives its
/// `notes` one, and the other's adopts it on import, which is how the two become one set.
fn a_server(home: &TempDir, model: bool) -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    if model {
        server.set_embedder(Arc::new(MarkerEmbedder));
    }
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    call(
        &mut server,
        "hoard_sets",
        json!({"action": "create", "name": "notes"}),
    );
    server
}

fn call(server: &mut Server, tool: &str, arguments: Value) -> Value {
    if tool == "hoard_sets" {
        review_all::review_before_export(server, &arguments);
    }
    serde_json::from_str(&call_text(server, tool, arguments))
        .expect("the tool payload should be JSON")
}

/// The text a call returns, for `hoard_conventions`, whose pages are text rather than JSON.
fn call_text(server: &mut Server, tool: &str, arguments: Value) -> String {
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
    let response: Value = serde_json::from_str(&raw).expect("the response should be JSON");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a text result, got {response}"));
    assert_ne!(
        response["result"]["isError"],
        json!(true),
        "the call failed: {text}"
    );
    text.to_owned()
}

fn memorize(server: &mut Server, content: &str) {
    call(
        server,
        "hoard_memorize",
        json!({
            "scope": "shared:notes", "content": content, "why": "a fixture",
            "confidence": 0.7, "origin": "llm", "keywords": ["fixture-subject"]
        }),
    );
}

/// Give `notes` its charter, which a shared set's first memory has to be since
/// `set-settled-at-first-write`. Only on the machine that authors the set: the other takes it on by
/// import, which brings the charter with it.
fn write_charter(server: &mut Server) {
    // Named first: a shared set's first write asks who you are in it.
    call(
        server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:notes", "author": "jtcours"}),
    );
    call(
        server,
        "hoard_memorize",
        json!({
            "scope": "shared:notes", "content": "This set holds the notes these tests exchange.",
            "why": "a fixture", "confidence": 1.0, "origin": "human",
            "keywords": ["hoard-scope-topics", "hoard-lang-en"]
        }),
    );
}

fn export(server: &mut Server, out: &TempDir) -> Value {
    call(
        server,
        "hoard_sets",
        json!({"action": "export", "scope": "shared:notes", "directory": out.path()}),
    )
}

fn import(server: &mut Server, out: &TempDir) -> Value {
    call(
        server,
        "hoard_sets",
        json!({"action": "import", "scope": "shared:notes", "directory": out.path()}),
    )
}

fn session_start(server: &mut Server) -> Value {
    conventions_aside(&call_text(server, "hoard_conventions", json!({})))
}

/// What a convention page says beside the catalog: the JSON line between the catalog's slice and
/// the page's last line, or an empty object on a page that says nothing besides.
fn conventions_aside(page: &str) -> Value {
    let lines: Vec<_> = page.split('\n').collect();
    match lines.as_slice() {
        [_, aside, _] => serde_json::from_str(aside).expect("the aside is one JSON line"),
        [_, _] => json!({}),
        _ => panic!("a convention page is a slice, an optional aside and a last line: {page}"),
    }
}

/// The cursor a page's last line tells the reader to call again with, if it says to.
fn next_cursor(page: &str) -> Option<usize> {
    let last = page.rsplit('\n').next()?;
    let digits: String = last
        .split(r#""cursor": "#)
        .nth(1)?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// The same set, held on our machine before the other one writes to it: the other machine exports
/// `notes` while it is still empty, and ours adopts that identity. After this, ours can write and
/// then import what the other machine writes, which is the only order in which a set holds memories
/// of its own before an import brings more.
fn the_set_shared_with_elsewhere(ours: &mut Server) -> (TempDir, Server) {
    let their_home = tempdir();
    let out = tempdir();
    let mut theirs = a_server(&their_home, true);
    std::mem::forget(their_home);
    write_charter(&mut theirs);
    export(&mut theirs, &out);
    import(ours, &out);
    // The file brought the other machine's author record, written with the charter, so this
    // machine says who it is before it writes -- as a second contributor taking on a set would.
    call(
        ours,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:notes", "author": "ours"}),
    );
    (out, theirs)
}

/// Write two memories on the other machine and export them over the shared file, with a model
/// loaded so the export described the set.
fn written_elsewhere(theirs: &mut Server, out: &TempDir) {
    memorize(theirs, "alpha is how the routing works");
    memorize(theirs, "beta is why it was chosen");
    export(theirs, out);
}

/// Someone else's machine: a set with two memories, exported with a model loaded, so the export
/// described it.
fn a_file_written_elsewhere() -> TempDir {
    let their_home = tempdir();
    let out = tempdir();
    let mut theirs = a_server(&their_home, true);
    write_charter(&mut theirs);
    memorize(&mut theirs, "alpha is how the routing works");
    memorize(&mut theirs, "beta is why it was chosen");
    let exported = export(&mut theirs, &out);
    assert!(
        exported["describes"].is_object(),
        "the writing machine had a model, so its export should describe the set: {exported}"
    );
    out
}

fn header_of(out: &TempDir) -> Value {
    let path = out.path().join("scatterhoard_mem/notes.json");
    let text = std::fs::read_to_string(&path).expect("reads");
    let header = text.split_once('\n').expect("a header line").0;
    serde_json::from_str(header).expect("the header is JSON")
}

/// Give the file the description an older build wrote into its header, naming a subject none of its
/// memories is about -- so it can be told apart from anything derived from them.
fn add_a_description_as_an_older_build_did(out: &TempDir) {
    let path = out.path().join("scatterhoard_mem/notes.json");
    let text = std::fs::read_to_string(&path).expect("reads");
    let (header, rest) = text.split_once('\n').expect("a header line");
    let mut header: Value = serde_json::from_str(header).expect("the header is JSON");
    header["summary"] = json!({
        "derivation": "clusters/2",
        "model": "marker-embedder",
        "clusters": [{"size": 2, "keywords": ["claimed-by-the-sender"]}],
    });
    std::fs::write(&path, format!("{header}\n{rest}")).expect("writes");
}

/// The deferred reports a session-start response carries, keyed by nothing: this harness loads one
/// set, so there is at most one.
fn owed(started: &Value) -> Option<&Value> {
    let entries = started.get("imported_sets_described")?.as_array()?;
    assert!(entries.len() <= 1, "one set is loaded: {started}");
    entries.first()
}

fn owed_note(report: &Value) -> &str {
    report["description_owed"]
        .as_str()
        .unwrap_or_else(|| panic!("no note that the description is owed: {report}"))
}

/// The comparison with a file's own description is retired; a response still carrying its verdict
/// would be reporting on something no file now holds.
fn assert_no_verdict(value: &Value) {
    assert!(
        value.get("against_the_claim").is_none(),
        "the claim comparison is retired: {value}"
    );
}

// ---------------------------------------------------------------------------
// At the import itself
// ---------------------------------------------------------------------------

/// The header is the line two branches that both exported used to collide on, because the
/// description in it changed with every export.
#[test]
fn an_exported_file_carries_no_description() {
    let file = a_file_written_elsewhere();
    let header = header_of(&file);
    assert!(header.get("summary").is_none(), "{header}");
}

#[test]
fn an_import_describes_the_set_from_its_own_memories() {
    let file = a_file_written_elsewhere();
    let home = tempdir();
    let mut ours = a_server(&home, true);

    let report = import(&mut ours, &file);
    assert!(report["describes"].is_object(), "{report}");
    assert!(report.get("description_owed").is_none(), "{report}");
    assert_no_verdict(&report);
}

/// A file an older build wrote still imports, and what its header says is neither shown nor
/// compared: the description is derived from the memories that arrived.
#[test]
fn a_description_carried_by_an_older_file_is_ignored() {
    let file = a_file_written_elsewhere();
    add_a_description_as_an_older_build_did(&file);
    let home = tempdir();
    let mut ours = a_server(&home, true);

    let report = import(&mut ours, &file);
    assert!(report["describes"].is_object(), "{report}");
    assert!(
        !report.to_string().contains("claimed-by-the-sender"),
        "the file's own description reached the response: {report}"
    );
    assert_no_verdict(&report);
}

#[test]
fn an_import_that_could_describe_itself_owes_nothing_at_session_start() {
    let file = a_file_written_elsewhere();
    let home = tempdir();
    let mut ours = a_server(&home, true);
    import(&mut ours, &file);

    let started = session_start(&mut ours);
    assert!(owed(&started).is_none(), "{started}");
}

// ---------------------------------------------------------------------------
// Importing before there is a model to describe with
// ---------------------------------------------------------------------------

#[test]
fn an_import_with_no_model_describes_nothing_and_says_the_description_is_owed() {
    let file = a_file_written_elsewhere();
    let home = tempdir();
    let mut ours = a_server(&home, false);

    let report = import(&mut ours, &file);
    assert!(
        report["describes"].is_null(),
        "a description of memories that have no vectors would describe none of them: {report}"
    );
    assert!(
        owed_note(&report).starts_with("not described yet"),
        "{report}"
    );
}

#[test]
fn while_there_is_still_no_model_a_session_start_reports_nothing() {
    let file = a_file_written_elsewhere();
    let home = tempdir();
    let mut ours = a_server(&home, false);
    import(&mut ours, &file);

    let started = session_start(&mut ours);
    assert!(owed(&started).is_none(), "{started}");
}

#[test]
fn adding_a_model_after_the_import_describes_the_set_at_the_next_session_start() {
    let file = a_file_written_elsewhere();
    let home = tempdir();
    let mut ours = a_server(&home, false);
    import(&mut ours, &file);

    ours.set_embedder(Arc::new(MarkerEmbedder));
    let page = call_text(&mut ours, "hoard_conventions", json!({}));
    let started = conventions_aside(&page);
    let entry = owed(&started).unwrap_or_else(|| panic!("nothing was reported: {started}"));

    assert_eq!(entry["scope"], json!("shared:notes"), "{entry}");
    assert!(entry["describes"].is_object(), "{entry}");
    assert_no_verdict(entry);
    let catalog: Value = page
        .split('\n')
        .next()
        .and_then(|first| serde_json::from_str(first).ok())
        .unwrap_or_default();
    assert!(
        catalog["conventions"]
            .as_array()
            .is_some_and(|c| !c.is_empty()),
        "the report sits beside the catalog, never instead of it: {page}"
    );
}

#[test]
fn the_owed_description_is_reported_at_every_session_start_until_the_set_is_exported() {
    let file = a_file_written_elsewhere();
    let home = tempdir();
    let mut ours = a_server(&home, false);
    import(&mut ours, &file);
    ours.set_embedder(Arc::new(MarkerEmbedder));

    assert!(owed(&session_start(&mut ours)).is_some());
    let again = session_start(&mut ours);
    assert!(
        owed(&again).is_some(),
        "reported once and then dropped, a session that skimmed past it has discharged it: {again}"
    );

    let out = tempdir();
    export(&mut ours, &out);
    let after = session_start(&mut ours);
    assert!(
        owed(&after).is_none(),
        "the export showed the description itself, which discharges it: {after}"
    );
}

/// An export that could not describe the set has shown nothing, so it cannot discharge the report
/// that stands in for a description.
#[test]
fn an_export_that_describes_nothing_does_not_discharge_the_owed_description() {
    let file = a_file_written_elsewhere();
    let home = tempdir();
    let mut ours = a_server(&home, false);
    import(&mut ours, &file);

    let out = tempdir();
    let exported = export(&mut ours, &out);
    assert!(exported["describes"].is_null(), "{exported}");

    ours.set_embedder(Arc::new(MarkerEmbedder));
    let started = session_start(&mut ours);
    assert!(owed(&started).is_some(), "{started}");
}

#[test]
fn a_set_changed_after_the_import_is_still_described() {
    let file = a_file_written_elsewhere();
    let home = tempdir();
    let mut ours = a_server(&home, false);
    import(&mut ours, &file);
    // The file brought its author's identity record, and this machine must say which record is it
    // before it can write -- an unrelated guard, satisfied here so the write can happen.
    let identities = call(
        &mut ours,
        "hoard_recall",
        json!({"keywords": ["hoard-author"], "scope": "shared:notes"}),
    );
    let record = identities["memories"][0]["id"].clone();
    call(
        &mut ours,
        "hoard_sets",
        json!({"action": "identify", "scope": "shared:notes", "record": record}),
    );
    memorize(&mut ours, "beta written here after the import");

    ours.set_embedder(Arc::new(MarkerEmbedder));
    let started = session_start(&mut ours);
    let entry = owed(&started).unwrap_or_else(|| panic!("nothing was reported: {started}"));
    assert!(entry["describes"].is_object(), "{entry}");
    assert_no_verdict(entry);
}

/// **A model that does not match the set leaves the same gap as no model**, which is why the
/// condition is coverage. New memories wait unembedded rather than join a space they do not belong
/// to, so the description would silently leave the import out -- and an export in that state shows
/// only part of the set, so it must not discharge what is owed.
#[test]
fn a_mismatched_model_owes_the_description_as_a_missing_one_does() {
    let home = tempdir();
    let mut ours = a_server(&home, true);
    let (file, mut theirs) = the_set_shared_with_elsewhere(&mut ours);
    memorize(&mut ours, "gamma was here before the import");
    written_elsewhere(&mut theirs, &file);
    ours.set_embedder(Arc::new(OtherEmbedder));

    let report = import(&mut ours, &file);
    assert!(
        owed_note(&report).starts_with("not described yet"),
        "{report}"
    );

    let out = tempdir();
    let exported = export(&mut ours, &out);
    assert!(
        exported["describes"].is_object(),
        "the memories embedded before the mismatch are still described: {exported}"
    );
    assert!(
        owed(&session_start(&mut ours)).is_none(),
        "still unembedded, so still nothing to report"
    );

    call(
        &mut ours,
        "hoard_sets",
        json!({"action": "reembed", "scope": "shared:notes"}),
    );
    let started = session_start(&mut ours);
    let entry = owed(&started).unwrap_or_else(|| {
        panic!("a partial export must not discharge the owed description: {started}")
    });
    assert!(entry["describes"].is_object(), "{entry}");
}

/// An export is the other moment the whole set is in view, so it embeds what it can before it
/// describes -- otherwise an export made straight after the model arrives would describe nothing,
/// and the report it should discharge would stand.
#[test]
fn an_export_after_the_model_arrives_describes_the_whole_set_and_discharges_the_report() {
    let file = a_file_written_elsewhere();
    let home = tempdir();
    let mut ours = a_server(&home, false);
    import(&mut ours, &file);
    ours.set_embedder(Arc::new(MarkerEmbedder));

    let out = tempdir();
    let exported = export(&mut ours, &out);
    assert!(exported["describes"].is_object(), "{exported}");
    let started = session_start(&mut ours);
    assert!(owed(&started).is_none(), "{started}");
}

/// A set that already held memories when the file arrived still owes, and is paid, a description.
#[test]
fn a_set_that_held_memories_before_the_import_is_still_described() {
    let home = tempdir();
    let mut ours = a_server(&home, false);
    let (file, mut theirs) = the_set_shared_with_elsewhere(&mut ours);
    memorize(&mut ours, "gamma was here before the import");
    written_elsewhere(&mut theirs, &file);
    import(&mut ours, &file);

    ours.set_embedder(Arc::new(MarkerEmbedder));
    let started = session_start(&mut ours);
    let entry = owed(&started).unwrap_or_else(|| panic!("nothing was reported: {started}"));
    assert!(entry["describes"].is_object(), "{entry}");
    assert_no_verdict(entry);
}

/// Only the first page begins a session. A continuation is the same load, and repeating the report
/// on every page would make it the loudest thing in the catalog.
#[test]
fn a_continuation_page_does_not_repeat_the_report() {
    let file = a_file_written_elsewhere();
    let home = tempdir();
    let mut ours = a_server(&home, false);
    import(&mut ours, &file);
    ours.set_embedder(Arc::new(MarkerEmbedder));

    // Room for the report and some of the catalog, and well short of the whole of it.
    let budget = 6_000;
    let first = call_text(&mut ours, "hoard_conventions", json!({"max_chars": budget}));
    assert!(owed(&conventions_aside(&first)).is_some(), "{first}");
    let cursor = next_cursor(&first)
        .unwrap_or_else(|| panic!("a {budget}-character page should continue: {first}"));
    let next = call_text(
        &mut ours,
        "hoard_conventions",
        json!({"cursor": cursor, "max_chars": budget}),
    );
    assert!(owed(&conventions_aside(&next)).is_none(), "{next}");
}
