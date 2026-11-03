// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for a set declaring the languages it is written in.
//!
//! `charter-declares-set-languages` in `TRACKING.md`, and `documents/design/memory-data-model.md`
//! under *a set declares its languages on its charter*. A set's `hoard-scope-topics` charter carries
//! a `hoard-lang-` keyword per language. The declaration does two jobs. It is a standard a project
//! sets and a maintainer reviews, which is why it lives in the exported file. And it tells a session
//! which language to write memories and phrase queries in, because the loaded model may represent
//! only some languages: a memory in another one can be stored and never found by meaning.
//!
//! **`hoard_sets status` reports what each set declares, and notes every set that declares nothing**
//! -- `local` included. `local` never travels, so nobody reviews it, but it is recalled from as much
//! as any set and is never made through `create`, so nothing else would prompt its declaration.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::language_fit::{BUILT_IN, TRIPLETS_PER_LANGUAGE};
use scatterhoard::mcp::Server;
use scatterhoard::memory::{
    Attribution, Author, Confidence, Keyword, Memory, MemoryId, MemoryText, Origin, Scope,
};
use scatterhoard::memory_file::MemorySet;
use scatterhoard::store::{Store, StoreLayout};
use serde_json::{Value, json};

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

/// Separates the first `n` built-in triplets of each language it is given, and places every other
/// text on one shared point -- so a language's score is exactly the `n` the test chose. Counts the
/// texts it embeds, so a test can see whether a score was worked out again.
#[derive(Debug)]
struct FitEmbedder {
    separates: Vec<(&'static str, usize)>,
    embedded: Arc<AtomicUsize>,
}

impl FitEmbedder {
    fn new(separates: &[(&'static str, usize)]) -> Self {
        Self {
            separates: separates.to_vec(),
            embedded: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// The axis a text sits on: its triplet's own for an anchor or paraphrase the test separates,
    /// one past the triplets for its unrelated sentence, and `None` for everything else.
    ///
    /// A set's own sentences are written `pN …` for an anchor or paraphrase and `oN …` for the
    /// unrelated sentence, which this separates whatever their language tag; any other own sentence
    /// sits on the shared point.
    fn axis(&self, text: &str) -> Option<usize> {
        let first = text.split_whitespace().next().unwrap_or_default();
        if let Some(n) = first
            .strip_prefix('p')
            .and_then(|n| n.parse::<usize>().ok())
        {
            return Some(n);
        }
        if let Some(n) = first
            .strip_prefix('o')
            .and_then(|n| n.parse::<usize>().ok())
        {
            return Some(TRIPLETS_PER_LANGUAGE + n);
        }
        for (tag, n) in &self.separates {
            let language = BUILT_IN
                .iter()
                .find(|l| l.tag == *tag)
                .expect("a built-in tag");
            for (index, triplet) in language.triplets.iter().take(*n).enumerate() {
                if let Some(role) = triplet.iter().position(|s| *s == text) {
                    return Some(if role == 2 {
                        TRIPLETS_PER_LANGUAGE + index
                    } else {
                        index
                    });
                }
            }
        }
        None
    }
}

impl Embedder for FitEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("fit-embedder")
    }

    fn dimension(&self) -> usize {
        2 * TRIPLETS_PER_LANGUAGE + 1
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        self.embedded.fetch_add(texts.len(), Ordering::SeqCst);
        texts
            .iter()
            .map(|text| {
                let mut components = vec![0.0; self.dimension()];
                components[self.axis(text).unwrap_or(2 * TRIPLETS_PER_LANGUAGE)] = 1.0;
                Vector::new(components)
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
    let path = std::env::temp_dir().join(format!("scatterhoard-languages-{unique}"));
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

fn a_server(home: &TempDir) -> Server {
    a_server_with(home, Arc::new(MarkerEmbedder))
}

fn a_server_with(home: &TempDir, embedder: Arc<dyn Embedder>) -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(embedder);
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    server
}

fn call(server: &mut Server, tool: &str, arguments: Value) -> Value {
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
    serde_json::from_str(text).expect("the tool payload should be JSON")
}

/// Store a memory in `scope` carrying `keywords`, optionally superseding another, and return its id.
fn store(server: &mut Server, scope: &str, keywords: &[&str], supersedes: Option<&str>) -> String {
    let mut arguments = json!({
        "scope": scope, "content": "what this set is for", "why": "a fixture",
        "confidence": 0.9, "origin": "human", "keywords": keywords,
    });
    if let Some(target) = supersedes {
        arguments["links"] = json!([{"label": "supersedes", "target": target}]);
    }
    call(server, "hoard_memorize", arguments)["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

/// The `status` entry for one set.
fn status_of(server: &mut Server, scope: &str) -> Value {
    call(server, "hoard_sets", json!({"action": "status"}))["sets"]
        .as_array()
        .expect("a list of sets")
        .iter()
        .find(|set| set["scope"] == json!(scope))
        .unwrap_or_else(|| panic!("{scope} is not in status"))
        .clone()
}

fn languages(entry: &Value) -> Vec<&str> {
    entry["languages"]
        .as_array()
        .unwrap_or_else(|| panic!("no languages field: {entry}"))
        .iter()
        .map(|tag| tag.as_str().expect("a tag"))
        .collect()
}

#[test]
fn a_set_whose_charter_declares_languages_reports_them() {
    let home = tempdir();
    let mut server = a_server(&home);
    store(
        &mut server,
        "local",
        &["hoard-scope-topics", "hoard-lang-es", "hoard-lang-en"],
        None,
    );

    let entry = status_of(&mut server, "local");
    assert_eq!(languages(&entry), ["en", "es"], "{entry}");
}

/// **`local` is noted like any other set.** It is never reviewed, but it is recalled from, and a
/// memory written there in a language the model does not represent is as unfindable as anywhere.
#[test]
fn a_local_set_whose_charter_declares_no_language_is_noted() {
    let home = tempdir();
    let mut server = a_server(&home);
    store(&mut server, "local", &["hoard-scope-topics"], None);

    let entry = status_of(&mut server, "local");
    assert!(languages(&entry).is_empty(), "{entry}");
    let note = entry["language_note"]
        .as_str()
        .unwrap_or_else(|| panic!("an undeclared set must be noted: {entry}"));
    assert!(
        note.contains("hoard-lang-"),
        "the note should say what to add: {note}"
    );
}

#[test]
fn a_shared_set_whose_charter_declares_no_language_is_noted() {
    let home = tempdir();
    let mut server = a_server(&home);
    call(
        &mut server,
        "hoard_sets",
        json!({"action": "create", "name": "notes"}),
    );
    // Named first: a shared set's first write asks who you are in it.
    call(
        &mut server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:notes", "author": "jtcours"}),
    );
    store(&mut server, "shared:notes", &["hoard-scope-topics"], None);

    let entry = status_of(&mut server, "shared:notes");
    assert!(languages(&entry).is_empty(), "{entry}");
    assert!(entry["language_note"].is_string(), "{entry}");
}

/// A set with no charter declares nothing either, and the note has to say the charter is what is
/// missing -- telling a caller to add a keyword to a memory that does not exist sends them looking
/// for it.
#[test]
fn a_set_with_no_charter_is_noted_as_having_none() {
    let home = tempdir();
    let mut server = a_server(&home);

    let entry = status_of(&mut server, "local");
    assert!(languages(&entry).is_empty(), "{entry}");
    let note = entry["language_note"].as_str().expect("noted");
    assert!(note.contains("no `hoard-scope-topics` charter"), "{note}");
}

/// A language is retired by superseding the charter without it, which is how every other claim on
/// a charter changes.
#[test]
fn a_language_dropped_when_the_charter_is_superseded_is_no_longer_declared() {
    let home = tempdir();
    let mut server = a_server(&home);
    let first = store(
        &mut server,
        "local",
        &["hoard-scope-topics", "hoard-lang-en", "hoard-lang-es"],
        None,
    );
    store(
        &mut server,
        "local",
        &["hoard-scope-topics", "hoard-lang-en"],
        Some(&first),
    );

    assert_eq!(languages(&status_of(&mut server, "local")), ["en"]);
}

/// A charter declaring one language, as a file would carry it.
fn charter(language: &str) -> Memory {
    let mut memory = Memory::new(
        MemoryText::new("what this set is for").expect("valid content"),
        Confidence::new(0.9).expect("in range"),
        Origin::Human,
        Attribution {
            by: MemoryId::generate(),
            at: jiff::Timestamp::now(),
        },
    );
    memory.keywords = vec![
        Keyword::new("hoard-scope-topics").expect("valid keyword"),
        Keyword::new(format!("hoard-lang-{language}")).expect("valid keyword"),
    ];
    memory
}

/// **A set can still hold two live charters**: `one-live-charter-per-set` refuses a second one only
/// when it is authored, and an import or an absorb that brings one in is never refused. So status
/// reads every live charter rather than choosing, and reports what they declare between them. The
/// two are built by import because authoring the second is exactly what the guard stops.
#[test]
fn two_live_charters_report_every_language_either_declares() {
    let home = tempdir();
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    let mut local = Store::open_in_memory(Scope::Local).expect("opens");
    local
        .import(&MemorySet {
            scope: Scope::Local,
            memories: vec![charter("en"), charter("fr")],
            tombstones: Vec::new(),
            set_id: None,
        })
        .expect("a merge can leave two live charters");
    server.add_store(local);

    assert_eq!(languages(&status_of(&mut server, "local")), ["en", "fr"]);
}

/// Only the charter declares. A memory that merely carries a language tag -- one discussing the
/// declaration, say -- is not the set saying what it is written in.
#[test]
fn a_language_tag_on_a_memory_that_is_not_the_charter_declares_nothing() {
    let home = tempdir();
    let mut server = a_server(&home);
    store(
        &mut server,
        "local",
        &["hoard-scope-topics", "hoard-lang-en"],
        None,
    );
    store(&mut server, "local", &["hoard-lang-de", "languages"], None);

    assert_eq!(languages(&status_of(&mut server, "local")), ["en"]);
}

/// Keywords keep the spelling their author wrote and match without case, so `hoard-lang-zh-Hans` and
/// `hoard-lang-zh-hans` are one declaration, reported in the caseless form.
#[test]
fn a_language_tag_is_reported_without_case() {
    let home = tempdir();
    let mut server = a_server(&home);
    store(
        &mut server,
        "local",
        &[
            "hoard-scope-topics",
            "hoard-lang-zh-Hans",
            "hoard-lang-zh-hans",
        ],
        None,
    );

    assert_eq!(languages(&status_of(&mut server, "local")), ["zh-hans"]);
}

/// The charter declaring `tags`, stored in `local`, and that set's status entry.
fn declaring(server: &mut Server, tags: &[&str]) -> Value {
    let mut keywords = vec!["hoard-scope-topics".to_string()];
    keywords.extend(tags.iter().map(|tag| format!("hoard-lang-{tag}")));
    let keywords: Vec<&str> = keywords.iter().map(String::as_str).collect();
    store(server, "local", &keywords, None);
    status_of(server, "local")
}

/// **Each declared language is scored 0 to 10 against the loaded model**, which is how many of ten
/// paraphrases the model ranks first among everything else written in that language.
/// `recall-is-latin-script-only`.
#[test]
fn each_declared_language_is_scored_against_the_model() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10), ("ja", 5)])));

    let entry = declaring(&mut server, &["en", "ja", "ru"]);
    assert_eq!(
        entry["language_scores"],
        json!({"en": 10, "ja": 5, "ru": 0}),
        "{entry}"
    );
}

/// A language the model represents well needs nothing said about it.
#[test]
fn a_searchable_language_adds_no_note() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 7)])));

    let entry = declaring(&mut server, &["en"]);
    assert!(entry.get("language_note").is_none(), "{entry}");
}

#[test]
fn a_language_scoring_four_to_six_is_noted_as_unreliable() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10), ("ja", 4)])));

    let entry = declaring(&mut server, &["en", "ja"]);
    let note = entry["language_note"].as_str().expect("noted");
    assert!(
        note.contains("Japanese") && note.contains("unreliable"),
        "{note}"
    );
    assert!(!note.contains("English"), "{note}");
}

/// **The note has to say the memories are safe**: an agent getting noise from a set it trusts would
/// otherwise conclude the memories are wrong, which is the confusion this exists to prevent.
#[test]
fn a_language_scoring_three_or_less_is_noted_as_unsearchable() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10), ("ru", 3)])));

    let entry = declaring(&mut server, &["en", "ru"]);
    let note = entry["language_note"].as_str().expect("noted");
    assert!(
        note.contains("Russian") && note.contains("intact"),
        "{note}"
    );
}

#[test]
fn every_poorly_served_language_is_noted() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("ja", 5)])));

    let note = declaring(&mut server, &["ja", "ru"])["language_note"]
        .as_str()
        .expect("noted")
        .to_owned();
    assert!(
        note.contains("Japanese") && note.contains("Russian"),
        "{note}"
    );
}

/// **Worked out once per language for the life of the server**: the answer depends only on the
/// model and the language, and a model change restarts the server. A declaration is still read from
/// the charter every time, so a newly declared language is scored when it appears.
#[test]
fn a_language_is_scored_once_per_server() {
    let home = tempdir();
    let embedder = FitEmbedder::new(&[("en", 10)]);
    let embedded = Arc::clone(&embedder.embedded);
    let mut server = a_server_with(&home, Arc::new(embedder));
    declaring(&mut server, &["en"]);
    let after_first = embedded.load(Ordering::SeqCst);

    status_of(&mut server, "local");
    assert_eq!(embedded.load(Ordering::SeqCst), after_first);
}

#[test]
fn a_language_declared_later_is_scored_when_it_appears() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10), ("fr", 8)])));
    let first = store(
        &mut server,
        "local",
        &["hoard-scope-topics", "hoard-lang-en"],
        None,
    );
    store(
        &mut server,
        "local",
        &["hoard-scope-topics", "hoard-lang-en", "hoard-lang-fr"],
        Some(&first),
    );

    assert_eq!(
        status_of(&mut server, "local")["language_scores"],
        json!({"en": 10, "fr": 8})
    );
}

/// With no model there is nothing to score against, and saying a language scores zero would blame
/// the language for what is a missing model -- which `semantic_search_available` already reports.
#[test]
fn a_set_that_cannot_search_by_meaning_has_no_scores() {
    let home = tempdir();
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));

    let entry = declaring(&mut server, &["en"]);
    assert!(entry.get("language_scores").is_none(), "{entry}");
}

/// Loading another model clears what was worked out under the last one. A real server restarts for
/// that, but a server is not entitled to assume it was restarted, and a score kept across the change
/// would describe a model it no longer has.
#[test]
fn a_new_model_is_scored_afresh() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    declaring(&mut server, &["en"]);

    server.set_embedder(Arc::new(FitEmbedder::new(&[("en", 2)])));
    assert_eq!(
        status_of(&mut server, "local")["language_scores"],
        json!({"en": 2})
    );
}

/// **A set whose vectors came from another model is not scored**, though the server holds one: the
/// score would describe a model the set cannot be searched with, and `embedding_model` already says
/// why the set cannot be searched at all.
#[test]
fn a_set_embedded_by_another_model_has_no_scores() {
    let home = tempdir();
    let path = home.path().join("local.sqlite3");
    {
        let mut local = Store::open_at(Scope::Local, &path).expect("opens");
        local.set_embedder(Arc::new(MarkerEmbedder));
        local.insert(&charter("en")).expect("inserts");
    }
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(FitEmbedder::new(&[("en", 10)])));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_at(Scope::Local, &path).expect("reopens"));

    let entry = status_of(&mut server, "local");
    assert_eq!(entry["semantic_search_available"], json!(false), "{entry}");
    assert!(entry.get("language_scores").is_none(), "{entry}");
}

/// **Recall by meaning carries the note too**, because that is where the confusion it prevents
/// arises: an agent reading noise from a set it trusts, with status called once at the start of the
/// session and long out of view. `recall-is-latin-script-only`.
#[test]
fn recall_by_meaning_notes_a_set_whose_language_the_model_serves_poorly() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    declaring(&mut server, &["en", "ru"]);

    let result = call(
        &mut server,
        "hoard_recall",
        json!({"query": "what is this set for"}),
    );
    let note = result["language_note"]
        .as_str()
        .unwrap_or_else(|| panic!("a poorly served language must be noted: {result}"));
    assert!(note.contains("Russian") && note.contains("local"), "{note}");
}

#[test]
fn recall_by_meaning_says_nothing_where_every_language_is_searchable() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    declaring(&mut server, &["en"]);

    let result = call(
        &mut server,
        "hoard_recall",
        json!({"query": "what is this set for"}),
    );
    assert!(result.get("language_note").is_none(), "{result}");
}

/// Keyword recall matches spellings, which no model is involved in, so a poorly served language
/// does not affect it and the note would be noise.
#[test]
fn keyword_recall_carries_no_language_note() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    declaring(&mut server, &["en", "ru"]);

    let result = call(
        &mut server,
        "hoard_recall",
        json!({"keywords": ["hoard-scope-topics"]}),
    );
    assert!(result.get("language_note").is_none(), "{result}");
}

// ---------------------------------------------------------------------------
// A set's own sentences
// ---------------------------------------------------------------------------

/// Store one sentence set for `tag` and return its id: separated by the fit embedder when
/// `separated`, on the shared point otherwise.
fn sentence_set(server: &mut Server, tag: &str, n: usize, separated: bool) -> String {
    let (head, tail) = if separated { ("p", "o") } else { ("q", "q") };
    let content = format!(
        "Paraphrase test {n} for {tag}. [\"{head}{n} anchor\", \"{head}{n} paraphrase\", \"{tail}{n} other\"]"
    );
    call(
        server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": content, "why": "a fixture", "confidence": 0.9,
            "origin": "llm", "keywords": ["hoard-fit-sentences", format!("hoard-lang-{tag}")],
        }),
    )["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

/// Store `separated` separated sets and the rest unseparated, up to `count`, for `tag`.
fn sentence_sets(server: &mut Server, tag: &str, count: usize, separated: usize) -> Vec<String> {
    (0..count)
        .map(|n| sentence_set(server, tag, n, n < separated))
        .collect()
}

/// Store the set's hub listing `sets`, superseding `replacing` if given, and return its id.
fn hub(server: &mut Server, sets: &[String], replacing: Option<&str>) -> String {
    let mut links: Vec<Value> = sets
        .iter()
        .map(|id| json!({"label": "refers_to", "target": id}))
        .collect();
    if let Some(old) = replacing {
        links.push(json!({"label": "supersedes", "target": old}));
    }
    call(
        server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": "This set's model-fit hub.", "why": "a fixture",
            "confidence": 0.9, "origin": "llm", "keywords": ["hoard-fit-hub"], "links": links,
        }),
    )["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

fn note_of(entry: &Value) -> String {
    entry["language_note"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a language note: {entry}"))
        .to_owned()
}

/// **A language with no sentences asks for them**, saying how: the set is almost certainly being
/// written by an agent that can write in the language it declares.
#[test]
fn a_declared_language_with_no_sentences_asks_for_ten() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));

    let entry = declaring(&mut server, &["en", "sm"]);
    assert_eq!(entry["language_scores"], json!({"en": 10}), "{entry}");
    let note = note_of(&entry);
    for needed in [
        "`sm`",
        "hoard-fit-sentences",
        "hoard-lang-sm",
        "hoard-fit-hub",
        "ten",
    ] {
        assert!(
            note.contains(needed),
            "the request should name {needed}: {note}"
        );
    }
}

#[test]
fn a_language_with_its_own_ten_sets_is_scored_from_them() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[])));
    let sets = sentence_sets(&mut server, "sm", 10, 8);
    hub(&mut server, &sets, None);

    let entry = declaring(&mut server, &["sm"]);
    assert_eq!(entry["language_scores"], json!({"sm": 8}), "{entry}");
}

/// **A set's own sentences outrank the built-ins**, so a set can measure its own vocabulary --
/// English full of Italian cooking terms -- which everyday sentences cannot.
#[test]
fn a_sets_own_sentences_take_precedence_over_the_built_ins() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    let sets = sentence_sets(&mut server, "en", 10, 3);
    hub(&mut server, &sets, None);

    let entry = declaring(&mut server, &["en"]);
    assert_eq!(entry["language_scores"], json!({"en": 3}), "{entry}");
}

/// Region is ignored in matching too: a charter in American English and sentences in British.
#[test]
fn own_sentences_match_a_declaration_differing_only_in_region() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    let sets = sentence_sets(&mut server, "en-gb", 10, 4);
    hub(&mut server, &sets, None);

    let entry = declaring(&mut server, &["en-us"]);
    assert_eq!(entry["language_scores"], json!({"en-us": 4}), "{entry}");
}

/// **A declaration matching two scripts is reported, not resolved**: a guess would score the
/// wrong text.
#[test]
fn a_declaration_matching_two_scripts_is_left_unscored_and_named() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[])));
    let mut sets = sentence_sets(&mut server, "egy-latn", 10, 10);
    sets.extend(sentence_sets(&mut server, "egy-egyp", 10, 10));
    hub(&mut server, &sets, None);

    let entry = declaring(&mut server, &["egy"]);
    assert!(entry.get("language_scores").is_none(), "{entry}");
    let note = note_of(&entry);
    assert!(
        note.contains("egy-latn") && note.contains("egy-egyp"),
        "{note}"
    );
}

#[test]
fn sentences_for_a_language_the_charter_does_not_declare_are_noted() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    let sets = sentence_sets(&mut server, "mi", 10, 10);
    hub(&mut server, &sets, None);

    let entry = declaring(&mut server, &["en"]);
    assert!(note_of(&entry).contains("`mi`"), "{entry}");
}

/// A set superseded by a memory that is not a sentence set leaves its language one short, which
/// cannot be scored on the same scale -- so it is said, rather than scored from nine.
#[test]
fn a_language_left_with_fewer_than_ten_sets_is_noted_and_unscored() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[])));
    let sets = sentence_sets(&mut server, "sm", 10, 10);
    hub(&mut server, &sets, None);
    call(
        &mut server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": "withdrawn", "why": "a fixture", "confidence": 0.9,
            "origin": "llm", "keywords": [],
            "links": [{"label": "supersedes", "target": sets[0]}],
        }),
    );

    let entry = declaring(&mut server, &["sm"]);
    assert!(entry.get("language_scores").is_none(), "{entry}");
    let note = note_of(&entry);
    assert!(note.contains("`sm`") && note.contains('9'), "{note}");
}

/// A sentence set no hub lists is scored for nothing, and is the trace a mistaken supersession
/// leaves -- so it is said.
#[test]
fn a_sentence_set_no_hub_lists_is_noted() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    let orphan = sentence_set(&mut server, "sm", 0, true);

    let entry = declaring(&mut server, &["en"]);
    assert!(note_of(&entry).contains(&orphan), "{entry}");
}

/// Scored by its text: a fixed set is rescored, where a score kept by language would not be.
#[test]
fn changing_a_sentence_set_changes_the_score() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[])));
    let sets = sentence_sets(&mut server, "sm", 10, 9);
    let first = hub(&mut server, &sets, None);
    assert_eq!(
        declaring(&mut server, &["sm"])["language_scores"],
        json!({"sm": 9})
    );

    let mut fixed = sets.clone();
    fixed[9] = sentence_set(&mut server, "sm", 9, true);
    hub(&mut server, &fixed, Some(&first));
    assert_eq!(
        status_of(&mut server, "local")["language_scores"],
        json!({"sm": 10})
    );
}

/// The request for sentences is setup work, so it belongs in status; recall carries only what
/// changes how its results should be read.
#[test]
fn recall_does_not_ask_for_sentences() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    declaring(&mut server, &["en", "sm"]);

    let result = call(
        &mut server,
        "hoard_recall",
        json!({"query": "what is this set for"}),
    );
    assert!(result.get("language_note").is_none(), "{result}");
}

/// A declaration naming its script takes that script's sentences and no other's.
#[test]
fn a_declaration_naming_a_script_is_scored_from_that_scripts_sentences() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[])));
    let mut sets = sentence_sets(&mut server, "egy-latn", 10, 6);
    sets.extend(sentence_sets(&mut server, "egy-egyp", 10, 2));
    hub(&mut server, &sets, None);

    let entry = declaring(&mut server, &["egy-latn", "egy-egyp"]);
    assert_eq!(
        entry["language_scores"],
        json!({"egy-latn": 6, "egy-egyp": 2}),
        "{entry}"
    );
}

// ---------------------------------------------------------------------------
// Recommended models
// ---------------------------------------------------------------------------

/// Store a recommendation linking to `hub`, tagged with `tags`, and return its id.
fn recommendation(server: &mut Server, hub: &str, model: &str, tags: &[&str]) -> String {
    let mut keywords = vec!["hoard-fit-model".to_string()];
    keywords.extend(tags.iter().map(|tag| format!("hoard-lang-{tag}")));
    call(
        server,
        "hoard_memorize",
        json!({
            "scope": "local", "content": format!("{model} serves this set better."),
            "why": "a fixture", "confidence": 0.9, "origin": "llm", "keywords": keywords,
            "links": [{"label": "refers_to", "target": hub}],
        }),
    )["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

/// **Listed whatever the scores**: a set's authors pass a recommendation on so a new machine can
/// be set up right, before anything goes wrong.
#[test]
fn status_lists_every_recommendation_whatever_the_scores() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    let hub_id = hub(&mut server, &[], None);
    let id = recommendation(&mut server, &hub_id, "model-a", &["en"]);

    let entry = declaring(&mut server, &["en"]);
    assert_eq!(
        entry["recommended_models"],
        json!([{"id": id, "summary": "model-a serves this set better.", "languages": ["en"]}]),
        "{entry}"
    );
}

#[test]
fn a_low_score_names_the_recommendations_for_its_language() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    let hub_id = hub(&mut server, &[], None);
    recommendation(&mut server, &hub_id, "model-ja", &["ja"]);
    recommendation(&mut server, &hub_id, "model-whole-set", &[]);
    recommendation(&mut server, &hub_id, "model-ru", &["ru"]);

    let note = note_of(&declaring(&mut server, &["en", "ja"]));
    assert!(note.contains("model-ja"), "{note}");
    assert!(
        note.contains("model-whole-set"),
        "an untagged recommendation concerns every language: {note}"
    );
    assert!(!note.contains("model-ru"), "{note}");
}

/// Matched as sentences are: by language and script, region ignored.
#[test]
fn a_recommendation_matches_a_language_differing_only_in_region() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[])));
    let hub_id = hub(&mut server, &[], None);
    recommendation(&mut server, &hub_id, "model-en", &["en-gb"]);

    let note = note_of(&declaring(&mut server, &["en-us"]));
    assert!(note.contains("model-en"), "{note}");
}

/// A language scoring well needs no pointer to a better model.
#[test]
fn a_searchable_language_names_no_recommendation() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    let hub_id = hub(&mut server, &[], None);
    recommendation(&mut server, &hub_id, "model-en", &["en"]);

    let entry = declaring(&mut server, &["en"]);
    assert!(entry.get("language_note").is_none(), "{entry}");
}

/// Recall's note says the model is the cause; which model to load instead is status's to say.
#[test]
fn recall_does_not_repeat_recommendations() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    let hub_id = hub(&mut server, &[], None);
    recommendation(&mut server, &hub_id, "model-ru", &["ru"]);
    declaring(&mut server, &["en", "ru"]);

    let result = call(
        &mut server,
        "hoard_recall",
        json!({"query": "what is this set for"}),
    );
    let note = result["language_note"].as_str().expect("noted");
    assert!(!note.contains("model-ru"), "{note}");
}

/// **Listed with no model loaded**, which is when knowing which one to install matters most.
#[test]
fn recommendations_are_listed_with_no_model_loaded() {
    let home = tempdir();
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_layout(StoreLayout::new(home.path().to_path_buf(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    let hub_id = hub(&mut server, &[], None);
    recommendation(&mut server, &hub_id, "model-a", &[]);

    let entry = status_of(&mut server, "local");
    assert_eq!(
        entry["recommended_models"][0]["summary"],
        json!("model-a serves this set better."),
        "{entry}"
    );
}

// ---------------------------------------------------------------------------
// Saying how
// ---------------------------------------------------------------------------

/// **A low score says what a set can do about it**: record a model that serves it better, and
/// measure its own vocabulary where everyday sentences would not. Nothing else served tells a
/// session either is possible.
#[test]
fn a_low_score_says_how_to_record_a_model_and_measure_the_sets_own_vocabulary() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));

    let note = note_of(&declaring(&mut server, &["en", "ru"]));
    for needed in [
        "hoard-fit-model",
        "hoard-fit-sentences",
        "hoard-fit-hub",
        "refers_to",
    ] {
        assert!(
            note.contains(needed),
            "the note should name {needed}: {note}"
        );
    }
}

/// Said once per set however many languages score low, since the how-to is the same for each.
#[test]
fn the_how_to_is_said_once_however_many_languages_score_low() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[])));

    let note = note_of(&declaring(&mut server, &["en", "ru", "ja"]));
    assert_eq!(note.matches("hoard-fit-model").count(), 1, "{note}");
}

/// The how-to is setup work, so recall's note stays the bands alone.
#[test]
fn recall_does_not_carry_the_how_to() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    declaring(&mut server, &["en", "ru"]);

    let result = call(
        &mut server,
        "hoard_recall",
        json!({"query": "what is this set for"}),
    );
    let note = result["language_note"].as_str().expect("noted");
    assert!(!note.contains("hoard-fit"), "{note}");
}

/// A request for sentences says what *listing* them from the hub means: links.
#[test]
fn the_request_for_sentences_says_the_hub_links_to_them() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));

    let note = note_of(&declaring(&mut server, &["en", "sm"]));
    assert!(note.contains("refers_to"), "{note}");
}

/// **The note names the hub's id when there is one**: linking to it is the instruction, and an
/// agent told to link to a memory it cannot name has to go looking -- or writes a second hub.
#[test]
fn the_how_to_names_the_hub_when_the_set_has_one() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    let hub_id = hub(&mut server, &[], None);

    let note = note_of(&declaring(&mut server, &["en", "ru"]));
    assert!(note.contains(&hub_id), "{note}");
}

/// **A score from the set's own sentences is about the set's vocabulary, not the language**: the
/// same model can read everyday English and not this set's borrowed terms, and a note saying it
/// cannot search English would send the user after a model that does not help (2026-10-04).
#[test]
fn a_low_score_from_the_sets_own_sentences_names_the_sets_use_of_the_language() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));
    let sets = sentence_sets(&mut server, "en", 10, 1);
    hub(&mut server, &sets, None);

    let note = note_of(&declaring(&mut server, &["en"]));
    assert!(note.contains("English as this set writes it"), "{note}");
}

#[test]
fn a_low_score_from_built_in_sentences_names_the_language_alone() {
    let home = tempdir();
    let mut server = a_server_with(&home, Arc::new(FitEmbedder::new(&[("en", 10)])));

    let note = note_of(&declaring(&mut server, &["en", "ru"]));
    assert!(note.contains("search Russian by meaning"), "{note}");
    assert!(!note.contains("as this set writes it"), "{note}");
}
