// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Approval tests: the **whole** response for a representative call, recorded and reviewed.
//!
//! # Why this exists beside the contract tests
//!
//! Sixteen defects have been found through this surface and none by the suite
//! (recorded in the `coding-lessons` set). Nearly every one is a property of a whole response rather than of any single
//! field — a score belonging to a different memory, a memory arriving with most of itself missing,
//! the same memory sent twice, `similarity` absent from one entry, a page eight times its budget.
//!
//! An assertion cannot find those, because writing one requires having already imagined the
//! failure. Recording the entire payload requires no imagination at all, only attention: the defect
//! is *visible* in the file whether or not anyone predicted it.
//!
//! # What keeps this from being the literal `STANDARDS.md` warns about
//!
//! *"A test that asserts against a literal instead of against the thing it describes"* is a real
//! hazard and a snapshot looks exactly like it. Three things separate them:
//!
//! - **The file is generated, never hand-written.** It is a recorded observation of what the code
//!   does, not a second hand-maintained copy of what someone believes it should do.
//! - **It asserts nothing in particular.** Its whole job is to make *any* change visible; the
//!   judgement stays with the reviewer of the diff rather than being frozen into an expectation.
//! - **Regeneration is explicit.** `UPDATE_SNAPSHOTS=1` rewrites the files and shows up in
//!   `git diff` like any other change.
//!
//! That last point is where approval testing usually dies: a snapshot accepted without reading it
//! records the bug as the expectation and is worse than no test. **A diff here is a request for
//! review, not a chore to clear.**
//!
//! # What is normalised, and what that costs
//!
//! Ids become `<id-N>` in first-seen order and timestamps become `<timestamp>`, because both vary
//! per run. The crate version becomes `<version>`, because it varies per release and a snapshot
//! rewritten by every release stops being read. Floats are rounded to four places. The cost is honest: **these snapshots cannot see a
//! timestamp defect**, since every timestamp normalises to the same token. They are for shape.

use std::collections::BTreeMap;
use std::sync::Arc;

use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::mcp::Server;
use scatterhoard::memory::{
    Attribution, Author, Confidence, Keyword, Memory, MemoryText, Origin, Scope, SetName,
};
use scatterhoard::store::Store;
use serde_json::{Map, Number, Value, json};

#[path = "support/review_all.rs"]
mod review_all;

#[path = "support/output_schema_check.rs"]
mod output_schema_check;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A second identity, for the one fixture that needs two sets to disagree about their model.
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
            .map(|text| {
                // The fixture charter is outside this model's vocabulary: see `MarkerEmbedder`.
                if text.contains(FIXTURE_CHARTER) {
                    return Ok(Embedded::Unrepresentable);
                }
                let count = |w: &str| text.matches(w).count() as f32;
                Vector::new(vec![0.01, count("alpha"), count("beta")])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

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
                // ranking of every fixture's two or three memories, because centring over so few moves
                // the query as much as the memories. See the same note in `tests/mcp.rs`.
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

/// What every fixture set's charter says. See [`MarkerEmbedder`] for why the model cannot place it.
const FIXTURE_CHARTER: &str = "This set holds the fixtures these tests write.";

/// A shared store as a set looks once it has its charter, which its first memory has to be since
/// `set-settled-at-first-write`.
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
    let mut charter = Memory::new(
        MemoryText::new(FIXTURE_CHARTER).expect("valid content"),
        Confidence::new(1.0).expect("in range"),
        Origin::Human,
        Attribution {
            by: author,
            at: jiff::Timestamp::now(),
        },
    );
    charter.keywords = vec![Keyword::new("hoard-scope-topics").expect("valid keyword")];
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

/// Looks like a memory id: 26 Crockford base32 characters.
///
/// **This detector is why the encoding change was not silent.** It matched `8-4-4-4-12` hex before
/// `crockford-base32-ids`, and the first regeneration afterwards wrote *raw ids* into every snapshot
/// — per-run values, which would have made these files change on every run. A snapshot that will not
/// settle is a finding, and here the finding was that the normaliser had stopped recognising the
/// thing it normalises.
fn is_memory_id(text: &str) -> bool {
    text.len() == 26
        && text
            .chars()
            .all(|c| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(c))
}

/// Sixteen lowercase hex characters: a 64-bit checksum as this project spells one.
///
/// Cannot collide with an id, which is 26 Crockford characters and uppercase.
fn is_checksum(text: &str) -> bool {
    text.len() == 16
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Looks like an RFC 3339 instant: `NNNN-` at the front, a `T`, and a trailing `Z`.
fn is_timestamp(text: &str) -> bool {
    text.len() >= 20
        && text.ends_with('Z')
        && text.contains('T')
        && text.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        && text.as_bytes()[4] == b'-'
}

/// Replace per-run values with stable tokens so a diff shows only shape and content.
///
/// Ids are numbered in first-seen order rather than blanked, so a reference such as
/// `replaces_matched` stays legible and a change in ordering stays visible.
fn normalise(value: &Value, ids: &mut BTreeMap<String, String>) -> Value {
    match value {
        Value::String(text) if is_memory_id(text) => {
            let next = ids.len() + 1;
            let token = ids
                .entry(text.clone())
                .or_insert_with(|| format!("<id-{next}>"));
            Value::String(token.clone())
        }
        Value::String(text) if is_timestamp(text) => Value::String("<timestamp>".to_owned()),
        // Anywhere in a string, since a conventions page is one string holding JSON lines.
        Value::String(text) if text.contains(env!("CARGO_PKG_VERSION")) => {
            Value::String(text.replace(env!("CARGO_PKG_VERSION"), "<version>"))
        }
        // A record's checksum covers its id and its timestamps, so it is per-run by construction.
        // **What this costs**: these recordings cannot see a wrong checksum *value* -- only that a
        // checksum is present and well-shaped. The value is asserted in tests/reconcile.rs and
        // driven end to end in tests/sets.rs, which is where a wrong one changes an outcome.
        Value::String(text) if is_checksum(text) => Value::String("<checksum>".to_owned()),
        Value::Number(number) => match number.as_f64() {
            Some(float) if !number.is_u64() && !number.is_i64() => {
                let rounded = (float * 10_000.0).round() / 10_000.0;
                Number::from_f64(rounded).map_or_else(|| value.clone(), Value::Number)
            }
            _ => value.clone(),
        },
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| normalise(item, ids)).collect())
        }
        Value::Object(fields) => {
            let mut out = Map::new();
            for (key, field) in fields {
                out.insert(key.clone(), normalise(field, ids));
            }
            Value::Object(out)
        }
        _ => value.clone(),
    }
}

/// The tool payload arrives as JSON inside a text block. Parse it so the snapshot shows structure
/// rather than one escaped string — a defect hidden inside an escaped blob is a defect nobody sees.
fn unwrap_payload(response: &Value) -> Value {
    // `structuredContent` is recorded as present rather than as a second copy of the payload: the
    // call helper has already checked it is identical to the text, and against the tool's schema.
    let mut response = response.clone();
    if response["result"].get("structuredContent").is_some() {
        response["result"]["structuredContent"] =
            json!("(present: identical to content[0].text, and checked against the output schema)");
    }
    let response = &response;
    let Some(text) = response["result"]["content"][0]["text"].as_str() else {
        return response.clone();
    };
    match serde_json::from_str::<Value>(text) {
        Ok(parsed) => {
            let mut cloned = response.clone();
            cloned["result"]["content"][0]["text"] = parsed;
            cloned
        }
        // A text payload of several lines -- a convention page -- is recorded line by line, each
        // line parsed where it is JSON, so the catalog in a whole page reads as structure and the
        // lines addressed to the reader read as themselves.
        Err(_) if text.contains('\n') => {
            let lines: Vec<Value> = text
                .split('\n')
                .map(|line| {
                    serde_json::from_str(line).unwrap_or_else(|_| Value::String(line.to_owned()))
                })
                .collect();
            let mut cloned = response.clone();
            cloned["result"]["content"][0]["text"] = Value::Array(lines);
            cloned
        }
        Err(_) => response.clone(),
    }
}

/// Compare a response against its recorded form, or rewrite it when `UPDATE_SNAPSHOTS=1`.
fn approve(name: &str, response: &Value) {
    let mut ids = BTreeMap::new();
    let normalised = normalise(&unwrap_payload(response), &mut ids);
    let actual = serde_json::to_string_pretty(&normalised).expect("serialises") + "\n";

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.json"));

    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().expect("has a parent"))
            .expect("creates the directory");
        std::fs::write(&path, &actual).expect("writes the snapshot");
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "no snapshot at {}\n\nrun with UPDATE_SNAPSHOTS=1, then read the new file before \
             committing it\n\nwould have recorded:\n{actual}",
            path.display()
        )
    });

    assert_eq!(
        actual, expected,
        "\n{name} no longer matches its recorded response.\n\nThis is a request for review, not a \
         chore: read the difference and decide whether the new shape is right. If it is, re-run \
         with UPDATE_SNAPSHOTS=1.\n"
    );
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

fn memorize(server: &mut Server, arguments: Value) -> Value {
    call(server, "hoard_memorize", arguments)
}

fn id_of(response: &Value) -> String {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("a text payload");
    let payload: Value = serde_json::from_str(text).expect("JSON payload");
    payload["id"].as_str().expect("an id").to_owned()
}

/// A store holding four memories: a direct semantic hit, an unrelated one that scores below zero,
/// and a superseded/replacement pair so the substituted result shape gets recorded.
fn a_populated_server() -> Server {
    let mut server = a_server();

    let routing = id_of(&memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha alpha alpha governs how requests are routed",
            "why": "the routing rule was decided before the code existed",
            "context": "recorded while building the approval fixture",
            "keywords": ["Alpha-Routing", "MemoryView"],
            "confidence": 0.9,
            "origin": "joint"
        }),
    ));

    // The reason behind the rule, in wording the rule's own query does not match. This is the memory
    // traversal exists to reach; `recall_already_known.json` is where the recorded surface shows it,
    // arriving with a deeply negative similarity of its own and an `inherited` score that explains
    // why it is there at all.
    memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the measurement that settled the routing rule",
            "why": "kept so the rule can be re-argued rather than only obeyed",
            "confidence": 0.8,
            "origin": "human",
            "links": [{"label": "supports", "target": routing}]
        }),
    );

    // Enough memories carrying the query's word to fill the walk's five-seed budget, so the memory
    // above is reached rather than seeded. **A fixture at or below the seed count cannot show
    // traversal at all** — every memory is a walk origin, and an origin never inherits — which is
    // what emptied `inherited` out of this recording when the walk was corrected on 2026-08-18.
    for (content, why) in [
        (
            "alpha alpha appear together in the shard table",
            "a second place the rule is felt",
        ),
        (
            "alpha alpha are counted before they are routed",
            "the counter predates the rule",
        ),
        (
            "alpha alpha alpha bound the retry path",
            "found while reading the router",
        ),
        (
            "alpha alpha alpha and beta share one transport",
            "noticed while tracing a request",
        ),
    ] {
        memorize(
            &mut server,
            json!({
                "scope": {"shared": "scatterhoard"},
                "content": content,
                "why": why,
                "confidence": 0.7,
                "origin": "llm"
            }),
        );
    }

    let superseded = id_of(&memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "beta beta describes the storage layout",
            "why": "an early guess at the layout",
            "confidence": 0.6,
            "origin": "llm"
        }),
    ));

    memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "gamma is unrelated to everything else here",
            "why": "present so a query has something to score badly against",
            "confidence": 0.5,
            "origin": "llm"
        }),
    );

    memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the storage layout was revised and now uses gamma",
            "why": "measured, where the earlier note was a guess",
            // Stated, not omitted: superseding requires saying what the replacement is indexed
            // under, and this fixture is about substitution rather than keywords. Empty is the
            // truthful answer here -- the memory it replaces carries none either -- and keeps the
            // recorded snapshots unchanged.
            "keywords": [],
            "confidence": 0.95,
            "origin": "joint",
            "links": [{"label": "supersedes", "target": superseded}]
        }),
    );

    server
}

// ---------------------------------------------------------------------------
// The published surface
// ---------------------------------------------------------------------------

#[test]
fn tools_list() {
    let mut server = a_server();
    let response = send(
        &mut server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}),
    );
    approve("tools_list", &response);
}

#[test]
fn conventions_default_page() {
    let mut server = a_server();
    approve(
        "conventions_default_page",
        &call(&mut server, "hoard_conventions", json!({})),
    );
}

/// `max_chars: 200` against a convention several times that size. Recorded so the overrun is a
/// number in a file rather than a claim in an issue.
#[test]
fn conventions_with_a_budget_smaller_than_one_convention() {
    let mut server = a_server();
    approve(
        "conventions_tiny_budget",
        &call(&mut server, "hoard_conventions", json!({"max_chars": 200})),
    );
}

// ---------------------------------------------------------------------------
// Recall
// ---------------------------------------------------------------------------

#[test]
fn recall_by_meaning() {
    let mut server = a_populated_server();
    approve(
        "recall_by_meaning",
        &call(
            &mut server,
            // Three, not one: scores are centred cosines, so a query *below* the corpus mean on the
            // only dimension that varies ranks the memories that do not mention it at all. That is
            // the cold-start problem in miniature, and a fixture is where it bites first.
            "hoard_recall",
            json!({"query": "alpha alpha alpha"}),
        ),
    );
}

/// A superseded memory is the match; the replacement is what comes back. This is the entry that
/// carries `matched_similarity` and no `similarity`.
#[test]
fn recall_that_substitutes_a_replacement() {
    let mut server = a_populated_server();
    approve(
        "recall_substituted",
        &call(&mut server, "hoard_recall", json!({"query": "beta"})),
    );
}

#[test]
fn recall_by_keyword() {
    let mut server = a_populated_server();
    approve(
        "recall_by_keyword",
        &call(
            &mut server,
            "hoard_recall",
            json!({"keywords": ["alpha-routing", "memoryview"]}),
        ),
    );
}

/// The reduced level, recorded whole for the reason every other response here is: what it *omits*
/// is the point of it, and an omission is only visible in the complete payload. Three shapes are in
/// this one recording -- a memory short enough to arrive intact, a line cut at a sentence, and a
/// line cut at a word -- so a change to the cut shows up here rather than in whichever case a
/// hand-written assertion happened to name.
#[test]
fn recall_in_summary_detail() {
    let mut server = a_populated_server();
    memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha alpha alpha are routed before they are counted. The order was \
                        settled by measurement rather than by argument, and the measurement is \
                        recorded beside the rule.",
            "why": "long enough that the line is a cut rather than a copy",
            "confidence": 0.9,
            "origin": "joint"
        }),
    );
    memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "alpha alpha alpha and the retry path share one counter, which is what \
                        makes the two of them impossible to tune separately without first taking \
                        the counter apart",
            "why": "one sentence, so the line has no sentence end to cut at",
            "confidence": 0.7,
            "origin": "llm"
        }),
    );

    approve(
        "recall_in_summary_detail",
        &call(
            &mut server,
            "hoard_recall",
            json!({"query": "alpha alpha alpha", "detail": "summary"}),
        ),
    );
}

/// The whole store in one response, which is what an unbounded limit buys: `recall-limit-is-unbounded`.
#[test]
fn recall_with_a_huge_limit() {
    let mut server = a_populated_server();
    approve(
        "recall_huge_limit",
        &call(
            &mut server,
            "hoard_recall",
            json!({"query": "alpha alpha alpha", "limit": 100_000}),
        ),
    );
}

/// Passing back what the last call returned is how a caller asks for the next tranche. The recorded
/// response is the one that shows the suppressed memory gone from the results while whatever it
/// points at is still here — output suppressed, participation untouched.
#[test]
fn recall_paging_past_what_the_caller_holds() {
    let mut server = a_populated_server();
    // Three of the eight, so there is a next tranche to record.
    let first = unwrap_payload(&call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha alpha alpha", "limit": 3}),
    ));
    let held: Vec<Value> = first["result"]["content"][0]["text"]["memories"]
        .as_array()
        .expect("a list of memories")
        .iter()
        .map(|memory| memory["id"].clone())
        .collect();

    approve(
        "recall_already_known",
        &call(
            &mut server,
            "hoard_recall",
            json!({"query": "alpha alpha alpha", "limit": 3, "already_known": held}),
        ),
    );
}

/// An id that is not one. The refusal names the argument and says what to pass instead, rather than
/// reporting whatever serde would say about a sequence element.
#[test]
fn recall_refuses_an_already_known_id_that_is_not_an_id() {
    let mut server = a_populated_server();
    approve(
        "refusal_already_known_prefix",
        &call(
            &mut server,
            "hoard_recall",
            json!({"query": "alpha alpha alpha", "already_known": ["1a2b3c4d"]}),
        ),
    );
}

#[test]
fn recall_with_a_zero_limit() {
    let mut server = a_populated_server();
    approve(
        "recall_zero_limit",
        &call(
            &mut server,
            "hoard_recall",
            json!({"query": "alpha alpha alpha", "limit": 0}),
        ),
    );
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// **This snapshot recorded the defect; now it records the refusal.**
///
/// It used to pass an eight-character prefix to `id` and capture what came back — every memory in
/// the store, because a UUIDv7's leading characters are a timestamp. An earlier version using
/// thirteen characters was *flaky*, since thirteen covers most of the millisecond and the match
/// count depended on how many memories happened to be written in the same one.
///
/// After `crockford-base32-ids` a shortened id is refused rather than resolved, so what is worth
/// recording is the refusal and whether it tells the caller what to do instead — which is the whole
/// of the improvement. `lookup-prefix-selectivity-is-time-correlated` closed by subtraction.
/// **The response nobody could see before**: a union where one set answers and another cannot.
///
/// Every other fixture here is complete, which is why `semantic-search-available-conflates-partial-with-none`
/// survived — with one loaded set, or two healthy ones, the partial case never arises. This records
/// the shape a caller actually meets: real hits, `semantic_search_available: false`, and
/// `skipped_sets` naming which set was left out and what to do about it.
#[test]
fn recall_where_one_set_could_not_be_searched() {
    let mut server = a_populated_server();
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));

    // Swapping the server's model is what a second checkout with its own configuration is: the set
    // already written now holds vectors this server cannot compare against, while a set written
    // afterwards is current. That is a partial union, and it is the ordinary case once a store owns
    // its model rather than inheriting the server's.
    server.set_embedder(Arc::new(OtherEmbedder));
    memorize(
        &mut server,
        json!({
            "scope": "local",
            "content": "alpha in the local set",
            "why": "so one set can answer while the other cannot",
            "confidence": 0.8,
            "origin": "llm"
        }),
    );

    approve(
        "recall_partial_coverage",
        &call(
            &mut server,
            "hoard_recall",
            json!({"query": "alpha", "limit": 5}),
        ),
    );
}

/// **A fixed literal here, deliberately, and the first version of this test was not.** It truncated
/// a freshly-minted id, and the refusal quotes what it was given — so the snapshot carried a
/// per-run value and would not settle. The normaliser could not catch it either: it recognises a
/// whole id, and this is by construction not one. A stable literal cannot expire the way
/// `prefix-test-literal-expired-with-the-clock` did, because nothing here *matches* on it — eight
/// characters is never twenty-six. That a real id is refused the same way is asserted in
/// `tests/mcp.rs`, which is where that linkage belongs.
#[test]
fn lookup_refuses_a_shortened_id() {
    let mut server = a_populated_server();
    approve(
        "lookup_shortened_id_refused",
        &call(&mut server, "hoard_lookup", json!({"id": "01M0G089"})),
    );
}

/// And the recovery it points at, with the fragment taken from the end as the message advises.
#[test]
fn lookup_by_a_fragment_from_the_end() {
    let mut server = a_populated_server();
    let all = call(
        &mut server,
        "hoard_recall",
        json!({"query": "alpha", "limit": 100}),
    );
    let text = all["result"]["content"][0]["text"]
        .as_str()
        .expect("a text payload");
    let payload: Value = serde_json::from_str(text).expect("JSON");
    let first = payload["memories"][0]["id"].as_str().expect("an id");
    let tail = &first[first.len() - 6..];

    approve(
        "lookup_by_fragment",
        &call(&mut server, "hoard_lookup", json!({"fragment": tail})),
    );
}

/// Reading history the way a client actually reaches it: recall hands back `replaces_matched`, and
/// that id is looked up. Records the shape of a superseded memory — and what a caller standing on
/// one can see of the edge pointing at it.
#[test]
fn lookup_of_a_superseded_memory() {
    let mut server = a_populated_server();
    let recalled = call(&mut server, "hoard_recall", json!({"query": "beta"}));
    let text = recalled["result"]["content"][0]["text"]
        .as_str()
        .expect("a text payload");
    let payload: Value = serde_json::from_str(text).expect("JSON");
    let replaced = payload["memories"][0]["replaces_matched"]
        .as_str()
        .expect("the first result should be a substitution");

    approve(
        "lookup_superseded",
        &call(&mut server, "hoard_lookup", json!({"id": replaced})),
    );
}

// ---------------------------------------------------------------------------
// Sets
// ---------------------------------------------------------------------------

/// `status` with two sets loaded. Records what a caller now gets in place of the dead end
/// `no-tool-lists-loaded-scopes` described — and records that there is no dirty flag yet.
///
/// Two sets rather than one deliberately: the stores live in a `HashMap`, so a missing sort would be
/// invisible with a single entry and flaky with two. One store also has no model, which is the
/// situation a server-wide searchability flag cannot express.
#[test]
fn sets_status() {
    let mut server = a_populated_server();
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    approve(
        "sets_status",
        &call(&mut server, "hoard_sets", json!({"action": "status"})),
    );
}

#[test]
fn sets_create_refuses_without_a_data_directory() {
    let mut server = a_populated_server();
    approve(
        "sets_create_no_home",
        &call(
            &mut server,
            "hoard_sets",
            json!({"action": "create", "name": "coding-lessons"}),
        ),
    );
}

/// A successful export. Records the path shape, the count, the standing statement that no
/// prohibited-data scan ran, and the absence of a note on a first export.
#[test]
fn sets_export() {
    let mut server = a_populated_server();
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("approval-export");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).expect("creates the scratch directory");

    let response = call(
        &mut server,
        "hoard_sets",
        json!({
            "action": "export",
            "scope": {"shared": "scatterhoard"},
            "directory": out.to_string_lossy(),
        }),
    );

    // The absolute path varies per machine, so the recorded response keeps everything but that.
    let mut redacted = response.clone();
    if let Some(text) = redacted["result"]["content"][0]["text"].as_str() {
        let mut payload: Value = serde_json::from_str(text).expect("JSON payload");
        // Only the directory is redacted, so the recording pins the file's name as well: it is
        // named for its set, and that is part of what the export decides.
        let written = payload["written"].as_str().expect("a path").to_owned();
        payload["written"] = json!(written.replace(&*out.to_string_lossy(), "<directory>"));
        redacted["result"]["content"][0]["text"] = json!(payload.to_string());
    }
    approve("sets_export", &redacted);
}

/// **The exported file itself**, which had no recording at all until 2026-08-30.
///
/// [STANDARDS.md](../STANDARDS.md) asks for an approval recording at every output boundary and names
/// two: the MCP surface and **the exported file format**. Only the first had one — `sets_export`
/// records the *response* to an export and never the bytes it wrote — so the format changed twice in
/// one day, gaining a tombstone's `ancestry` and then its `forgot`, with no recorded artifact
/// showing either. This records the file: the header, a memory with a link, and a tombstone
/// carrying both of the fields that were added blind.
///
/// **`forgot` is normalised, and the first version of this test got that wrong.** It was written
/// asserting the checksum was stable because the fixture fixes the content — which ignores that a
/// record's checksum covers its **id and its timestamps**, both per-run. The snapshot disagreed with
/// itself on the second run, which is the nondeterminism STANDARDS.md calls a finding rather than a
/// nuisance: here the finding is that the value is not a property of the format. What it costs is
/// stated beside the normaliser — this recording sees that a checksum is present and well-shaped,
/// never that it is right.
#[test]
fn exported_file_format() {
    let mut server = a_server();
    let kept = id_of(&memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the memory that stays",
            "why": "so the file has an ordinary record in it",
            "keywords": ["kept"],
            "confidence": 0.7,
            "origin": "llm"
        }),
    ));
    let doomed = id_of(&memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the memory that is forgotten",
            "why": "so the file has a tombstone in it",
            // Stated for the same reason as the fixture above: this record exists to become a
            // tombstone, and empty is what it was always stored with.
            "keywords": [],
            "confidence": 0.7,
            "origin": "llm",
            "links": [{"label": "supersedes", "target": kept}],
        }),
    ));
    call(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": {"shared": "scatterhoard"}, "id": doomed}),
    );

    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("approval-file-format");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).expect("creates the scratch directory");
    let exported = call(
        &mut server,
        "hoard_sets",
        json!({
            "action": "export",
            "scope": {"shared": "scatterhoard"},
            "directory": out.to_string_lossy(),
        }),
    );
    assert!(exported["result"]["isError"] != json!(true), "{exported:?}");

    let text = std::fs::read_to_string(out.join("scatterhoard_mem/scatterhoard.json"))
        .expect("reads the exported file");
    let mut lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("every line is JSON"))
        .collect();
    // **The records are recorded sorted by content, not in the order the file holds them.** A file
    // is ordered by `memory_file::file_position`, a hash of each id, and these ids are minted fresh
    // every run -- so the file's own order differs run to run, and a recording of it failed ten runs
    // in twelve. What this recording is for is the shape of each line; the order is asserted with
    // chosen ids in `tests/memory_file.rs` and `tests/store.rs`. A tombstone has no content and
    // sorts first.
    lines[1..].sort_by_key(|record| record["content"].as_str().map(str::to_owned));
    approve("exported_file_format", &json!({"lines": lines}));
}

/// An import that settles cleanly, and one that reports a conflict. Recorded together because the
/// shape a caller has to act on is the *difference* between them.
#[test]
fn sets_import() {
    let mut server = a_populated_server();
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("approval-import");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("creates the scratch directory");

    let scope = json!("shared:scatterhoard");
    let export = |server: &mut Server, at: &std::path::Path| {
        call(
            server,
            "hoard_sets",
            json!({"action": "export", "scope": scope, "directory": at.to_string_lossy()}),
        );
    };
    export(&mut server, &root);

    let response = call(
        &mut server,
        "hoard_sets",
        json!({"action": "import", "scope": scope, "directory": root.to_string_lossy()}),
    );

    let mut redacted = response.clone();
    if let Some(text) = redacted["result"]["content"][0]["text"].as_str() {
        let mut payload: Value = serde_json::from_str(text).expect("JSON payload");
        let read = payload["read"].as_str().expect("a path").to_owned();
        payload["read"] = json!(read.replace(&*root.to_string_lossy(), "<directory>"));
        redacted["result"]["content"][0]["text"] = json!(payload.to_string());
    }
    approve("sets_import_settled", &redacted);
}

/// `hoard_correct`'s response had no snapshot at all until 2026-08-30, which is a gap in a standing
/// requirement rather than an oversight about this feature: STANDARDS.md asks for a recording at
/// every output boundary, and this one carries prose telling a caller what to do about an edge whose
/// text has moved under it. Prose is what gets reworded without anyone re-reading what it now says.
///
/// The correction here touches `content` on a memory another memory `supports`, which is the shape
/// that raises the remark. An apparatus-only correction produces the plain response and is asserted
/// in tests/correct.rs rather than recorded, since there is nothing prose-shaped in it to drift.
#[test]
fn correct_response_with_an_in_link_remark() {
    let mut server = a_server();
    let target = id_of(&memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the claim another memory was written against",
            "why": "so there is something for an edge to have judged",
            "confidence": 0.7,
            "origin": "llm"
        }),
    ));
    memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the measurement that supports it",
            "why": "so the edge exists",
            "confidence": 0.7,
            "origin": "llm",
            "links": [{"label": "supports", "target": target}],
        }),
    );

    approve(
        "correct_in_link_remark",
        &call(
            &mut server,
            "hoard_correct",
            json!({
                "action": "amend",
                "scope": {"shared": "scatterhoard"},
                "id": target,
                "content": "the claim as it reads after the repair",
            }),
        ),
    );
}

#[test]
fn sets_unknown_action() {
    let mut server = a_server();
    approve(
        "sets_unknown_action",
        &call(&mut server, "hoard_sets", json!({"action": "obliterate"})),
    );
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn memorize_response() {
    let mut server = a_server();
    approve(
        "memorize_response",
        &memorize(
            &mut server,
            json!({
                "scope": {"shared": "scatterhoard"},
                "content": "a memory stored to record what the response looks like",
                "why": "the response shape is the thing under test",
                "confidence": 0.7,
                "origin": "llm"
            }),
        ),
    );
}

/// The supersession response, which carries a field and a paragraph of prose that nothing else
/// records.
///
/// Same argument as the sibling below: the note tells a caller what to do about keywords that
/// stopped reaching anything, and prose gets reworded without anyone re-reading what it now says. A
/// snapshot puts it in the diff. It also makes the `keyword_delta` and `link_delta` shapes a
/// checked-in fact rather than a claim, which is what `response-shape-is-unpublished` says every
/// response is missing. The earlier claim carries a link the replacement leaves behind, so the
/// note that fires is the one telling a caller what to do about it.
#[test]
fn memorize_response_for_a_supersession() {
    let mut server = a_server();
    let target = id_of(&memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "a memory the earlier claim points at",
            "why": "so there is a link to leave behind",
            "confidence": 0.7,
            "origin": "llm"
        }),
    ));
    let original = id_of(&memorize(
        &mut server,
        json!({
            "scope": {"shared": "scatterhoard"},
            "content": "the earlier claim",
            "why": "so there is something to supersede",
            "keywords": ["kept-key", "dropped-key"],
            "links": [{"label": "related_to", "target": target}],
            "confidence": 0.7,
            "origin": "llm"
        }),
    ));
    approve(
        "memorize_supersession",
        &memorize(
            &mut server,
            json!({
                "scope": {"shared": "scatterhoard"},
                "content": "the corrected claim",
                "why": "the response shape is the thing under test",
                "keywords": ["kept-key", "added-key"],
                "links": [{"label": "supersedes", "target": original}],
                "confidence": 0.9,
                "origin": "llm"
            }),
        ),
    );
}

/// The same call into a store with no model, recorded separately because it is a different
/// response and the interesting one.
///
/// Worth its own snapshot rather than an assertion on the flag: the payload carries prose telling
/// a caller what to do about an unfindable memory, and prose is exactly the kind of content that
/// gets reworded without anyone re-reading what it now says. A snapshot puts it in the diff.
#[test]
fn memorize_response_when_the_memory_cannot_be_found_by_meaning() {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.add_store(chartered(scope()));
    approve(
        "memorize_response_unfindable",
        &memorize(
            &mut server,
            json!({
                "scope": {"shared": "scatterhoard"},
                "content": "a memory stored where nothing can embed it",
                "why": "the response shape is the thing under test",
                "confidence": 0.7,
                "origin": "llm"
            }),
        ),
    );
}

#[test]
fn recall_refuses_a_query_and_keywords_together() {
    let mut server = a_populated_server();
    approve(
        "refusal_query_and_keywords",
        &call(
            &mut server,
            "hoard_recall",
            json!({"query": "alpha", "keywords": ["alpha-routing"]}),
        ),
    );
}

#[test]
fn memorize_refuses_an_unloaded_set() {
    let mut server = a_server();
    approve(
        "refusal_unloaded_set",
        &memorize(
            &mut server,
            json!({
                "scope": {"shared": "coding-lessons"},
                "content": "a memory for a set that does not exist",
                "why": "to record what the refusal says",
                "confidence": 0.5,
                "origin": "llm"
            }),
        ),
    );
}
