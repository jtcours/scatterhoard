// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The export review's judgement gate: what a review records, what it serves, and what export then
//! refuses. `documents/design/export-review.md`, *The procedure*.

use std::str::FromStr;

use jiff::Timestamp;
use scatterhoard::checksum;
use scatterhoard::memory::{
    Attribution, Confidence, Keyword, Memory, MemoryId, MemoryText, Origin, Scope, SetName,
};
use scatterhoard::store::Store;

fn a_store() -> Store {
    Store::open_in_memory_and_mint_id(Scope::Shared(SetName::new("projector").expect("valid")))
        .expect("opens")
}

fn a_memory(content: &str) -> Memory {
    Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: Timestamp::from_str("2026-10-23T09:00:00Z").expect("valid timestamp"),
        },
    )
}

// ---------------------------------------------------------------------------------------------
// The store: clearances by checksum, and the preferences the last review read.
// ---------------------------------------------------------------------------------------------

/// **Cleared means this exact record**: the checksum is the one an export would write, so any later
/// change to the memory reopens it, and nothing outside the record does.
#[test]
fn a_clearance_is_recorded_by_the_record_checksum() {
    let mut store = a_store();
    let memory = a_memory("The parser normalises tabs to spaces.");
    store.insert(&memory).expect("stores");
    let sum = checksum::of_memory(&memory);
    store
        .record_clearances(&[(memory.id, sum)])
        .expect("records");
    assert_eq!(
        store.clearances().expect("reads").get(&memory.id),
        Some(&sum)
    );
}

#[test]
fn a_later_clearance_of_the_same_memory_replaces_the_earlier() {
    let mut store = a_store();
    let id = MemoryId::generate();
    store.record_clearances(&[(id, 1)]).expect("records");
    store.record_clearances(&[(id, 2)]).expect("records again");
    let clearances = store.clearances().expect("reads");
    assert_eq!(clearances.len(), 1);
    assert_eq!(clearances.get(&id), Some(&2));
}

/// The text a set's last finished review read, so a change to the preferences reopens only what
/// it tightens. Absent until a review finishes.
#[test]
fn the_preferences_a_review_read_are_kept_for_the_next() {
    let mut store = a_store();
    assert_eq!(store.review_snapshot().expect("reads"), None);
    store
        .set_review_snapshot("## Keep out of every set, including private ones\n- my health\n")
        .expect("records");
    assert_eq!(
        store.review_snapshot().expect("reads").as_deref(),
        Some("## Keep out of every set, including private ones\n- my health\n")
    );
}

// ---------------------------------------------------------------------------------------------
// The mechanical pre-pass: shapes that go to the person whatever the model says.
// ---------------------------------------------------------------------------------------------

use scatterhoard::preferences::ListedPreferences;
use scatterhoard::review::{Shape, pre_pass};

/// The kinds of shape `text` holds, with the listed words and authors given.
fn shapes(text: &str, preferences: &str, authors: &[&str]) -> Vec<(Shape, String)> {
    let lists = ListedPreferences::parse(preferences);
    let authors: Vec<String> = authors.iter().map(|a| (*a).to_owned()).collect();
    pre_pass(text, &lists.never_store, &lists.always_fine, &authors)
        .into_iter()
        .map(|hit| (hit.shape, hit.matched))
        .collect()
}

fn kinds(text: &str) -> Vec<Shape> {
    shapes(text, "", &[])
        .into_iter()
        .map(|(shape, _)| shape)
        .collect()
}

#[test]
fn an_email_address_is_caught() {
    assert_eq!(
        shapes("Write to priya.raman@example.org about it.", "", &[]),
        vec![(Shape::Email, "priya.raman@example.org".to_owned())]
    );
}

/// The user-name part is what identifies someone; a path without one passes.
#[test]
fn a_home_path_carrying_a_user_name_is_caught() {
    assert_eq!(
        kinds("Stored under /Users/alice/Documents."),
        vec![Shape::HomePath]
    );
    assert_eq!(
        kinds("Stored under /home/bob/.config."),
        vec![Shape::HomePath]
    );
    assert_eq!(
        kinds(r"Stored under C:\Users\carol\AppData."),
        vec![Shape::HomePath]
    );
    assert!(kinds("Stored under ~/.local/share/scatterhoard.").is_empty());
    assert!(kinds("Shared files live in /Users/Shared/.").is_empty());
    assert!(kinds("The path is /Users/<name>/ on a Mac.").is_empty());
}

/// Private ranges and internal host names; public names and the `.local` directory pass.
#[test]
fn a_private_address_or_internal_host_is_caught() {
    assert_eq!(
        kinds("The runner is at 192.168.1.40."),
        vec![Shape::PrivateHost]
    );
    assert_eq!(
        kinds("The runner is at 10.0.3.7."),
        vec![Shape::PrivateHost]
    );
    assert_eq!(
        kinds("The runner is at 172.20.0.5."),
        vec![Shape::PrivateHost]
    );
    assert_eq!(
        kinds("Builds run on build-01.internal nightly."),
        vec![Shape::PrivateHost]
    );
    assert_eq!(
        kinds("Ask nas.lan for the archive."),
        vec![Shape::PrivateHost]
    );
    assert!(kinds("The project lives at github.com/example/projector.").is_empty());
    assert!(kinds("Data goes in ~/.local/share.").is_empty());
    assert!(kinds("The public resolver is 8.8.8.8.").is_empty());
}

/// Only shapes that read as phone numbers: ids, counts, dates and versions pass.
#[test]
fn a_phone_number_shape_is_caught() {
    assert_eq!(
        kinds("Call +44 20 7946 0958 after six."),
        vec![Shape::Phone]
    );
    assert_eq!(kinds("Call (555) 012-3456 after six."), vec![Shape::Phone]);
    assert_eq!(kinds("Call 555-012-3456 after six."), vec![Shape::Phone]);
    assert!(kinds("Released 2026-10-24 as version 0.1.0.").is_empty());
    assert!(kinds("Memory 01M5HVC7S0FC999JQR0T2HEGH2 holds 1458 memories.").is_empty());
    assert!(kinds("The catalog is 17915 characters, 85 to spare.").is_empty());
}

/// **A date with its time is not a phone number**, in any of the shapes this project writes one:
/// `the-pre-pass-reads-a-timestamp-as-a-phone-number`. A number written after a date is still
/// found, and a run that only looks like a date, with an impossible year, month or day, is still
/// read as a number.
#[test]
fn a_timestamp_is_not_a_phone_number() {
    assert!(kinds("Read checkpoints/2026-08-13-2255.md first.").is_empty());
    assert!(kinds("The 2026-08-25 06:38 checkpoint said so.").is_empty());
    assert!(kinds("Written 2026-10-29T05:05Z.").is_empty());
    assert_eq!(
        kinds("On 2026-08-13 call 555-012-3456."),
        vec![Shape::Phone]
    );
    assert_eq!(kinds("Logged 2026-08-13 555-012-3456."), vec![Shape::Phone]);
    for number in ["1234-12-01-2255", "2026-13-01-2255", "2026-12-32-2255"] {
        assert_eq!(kinds(&format!("Call {number} now.")), vec![Shape::Phone]);
    }
}

/// **An SSH remote is not an email address**: `git` is the hosting service's shared user, and
/// `git@host:path` names a repository, not anyone's mailbox —
/// `the-pre-pass-reads-an-ssh-remote-as-an-email`. Without the `:path`, `git@host` is still read as an
/// address, since nothing then says it is a remote.
#[test]
fn an_ssh_remote_is_not_an_email_address() {
    assert!(kinds("The remote is git@github.com:example/projector.git now.").is_empty());
    assert!(kinds("Push to `git@gitlab.example.org:team/repo.git`.").is_empty());
    assert_eq!(kinds("Mail git@example.org about it."), vec![Shape::Email]);
    assert_eq!(
        kinds("Write to gitte@example.org:support today."),
        vec![Shape::Email]
    );
}

#[test]
fn a_never_store_word_is_caught() {
    assert_eq!(
        shapes(
            "Thursday HIV treatment.",
            "## Never store these words\n- HIV\n",
            &[]
        ),
        vec![(Shape::NeverStore, "HIV".to_owned())]
    );
}

/// The names a set publishes under are exact, so a collaborator named in text is caught.
#[test]
fn an_author_name_is_caught() {
    assert_eq!(
        shapes(
            "As jbrown said, keep the interface stable.",
            "",
            &["jbrown"]
        ),
        vec![(Shape::Author, "jbrown".to_owned())]
    );
}

/// **Always fine silences the pre-pass for exact values**, so a work number is not put to the
/// person in every new memory; never-store wins where a value is in both.
#[test]
fn always_fine_silences_an_exact_value_but_never_a_never_store_word() {
    let fine = "## Always fine to share\n- me@example.com\n- HIV\n";
    assert!(shapes("Mail me@example.com.", fine, &[]).is_empty());
    assert_eq!(
        shapes("Mail you@example.com.", fine, &[]).len(),
        1,
        "only the listed value is silenced"
    );
    let both = format!("{fine}\n## Never store these words\n- HIV\n");
    assert_eq!(
        shapes("Thursday HIV treatment.", &both, &[]),
        vec![(Shape::NeverStore, "HIV".to_owned())]
    );
}

// ---------------------------------------------------------------------------------------------
// The project's file: what everyone exporting to a repository is checked against.
// ---------------------------------------------------------------------------------------------

use scatterhoard::review::{Audience, ProjectPreferences};

#[test]
fn the_project_file_holds_keep_outs_values_and_an_audience() {
    let project = ProjectPreferences::parse(
        r#"{"keep_out": ["client names"], "always_fine": ["noreply@example.com"], "audience": "private"}"#,
    )
    .expect("parses");
    assert_eq!(project.keep_out, vec!["client names".to_owned()]);
    assert_eq!(project.always_fine, vec!["noreply@example.com".to_owned()]);
    assert_eq!(project.audience, Audience::Private);
}

/// **Absent means public**, so forgetting to declare a private audience exposes nothing.
#[test]
fn an_undeclared_audience_is_public() {
    assert_eq!(
        ProjectPreferences::parse("{}").expect("parses").audience,
        Audience::Public
    );
    assert_eq!(ProjectPreferences::default().audience, Audience::Public);
}

/// **An unknown key is refused and named**: an ignored one would look applied and do nothing.
#[test]
fn an_unknown_key_in_the_project_file_is_refused_by_name() {
    let refusal = ProjectPreferences::parse(r#"{"keep_out": [], "exceptions": ["my employer"]}"#)
        .expect_err("refused");
    assert!(refusal.contains("exceptions"), "{refusal}");
}

#[test]
fn an_audience_other_than_public_or_private_is_refused() {
    assert!(ProjectPreferences::parse(r#"{"audience": "friends"}"#).is_err());
}

#[test]
fn the_project_file_is_read_from_the_checkout() {
    let checkout =
        std::env::temp_dir().join(format!("scatterhoard-project-{}", MemoryId::generate()));
    assert_eq!(ProjectPreferences::load(&checkout).expect("reads"), None);
    let config = checkout.join("scatterhoard_mem").join("config");
    std::fs::create_dir_all(&config).expect("creates");
    std::fs::write(
        config.join("sharing-preferences.json"),
        r#"{"audience": "private"}"#,
    )
    .expect("writes");
    let project = ProjectPreferences::load(&checkout)
        .expect("reads")
        .expect("present");
    assert_eq!(project.audience, Audience::Private);
    let _ = std::fs::remove_dir_all(&checkout);
}

// ---------------------------------------------------------------------------------------------
// The server's half of a review: numbering sentences, and routing a model's answers.
// ---------------------------------------------------------------------------------------------

use scatterhoard::review::{About, Decision, Says, SentenceAnswer, SetKind, numbered, route};

/// Text, reasoning and context are numbered as one run, because an export publishes all three.
#[test]
fn a_memory_is_numbered_across_its_text_reasoning_and_context() {
    let parts = numbered(
        "Recall pages default to eight. Larger ones crowded the context.",
        Some("Chosen while the person was off work after surgery."),
        Some("Tuning recall."),
    );
    let listed: Vec<(usize, &str, &str)> = parts
        .iter()
        .map(|p| (p.n, p.field, p.text.as_str()))
        .collect();
    assert_eq!(
        listed,
        vec![
            (1, "content", "Recall pages default to eight."),
            (2, "content", "Larger ones crowded the context."),
            (
                3,
                "why",
                "Chosen while the person was off work after surgery."
            ),
            (4, "context", "Tuning recall."),
        ]
    );
}

fn answer(about: About, mentions: &[&str]) -> SentenceAnswer {
    let says = (about != About::Project).then_some(Says::Reports);
    SentenceAnswer {
        n: 1,
        about,
        says,
        mentions: mentions.iter().map(|m| (*m).to_owned()).collect(),
    }
}

fn judging(about: About) -> SentenceAnswer {
    SentenceAnswer {
        says: Some(Says::Judges),
        ..answer(about, &[])
    }
}

fn routed(kind: SetKind, answers: &[SentenceAnswer]) -> Decision {
    route(
        kind,
        answers,
        &ListedPreferences::parse(TEMPLATE_FOR_ROUTING),
        &[],
    )
}

const TEMPLATE_FOR_ROUTING: &str = "## Keep out of shared sets\n- my employer\n- where I live, and how to contact me\n\n## Keep out of every set, including private ones\n- my health\n";

#[test]
fn a_shared_memory_about_the_project_mentioning_nothing_is_cleared() {
    assert_eq!(
        routed(SetKind::Shared, &[answer(About::Project, &[])]),
        Decision::Clear
    );
}

/// In a shared set, a memory about the person goes to them, and so does anything mentioned.
#[test]
fn a_shared_memory_about_the_person_or_mentioning_an_item_is_flagged() {
    assert!(matches!(
        routed(SetKind::Shared, &[answer(About::Person, &[])]),
        Decision::Flag(_)
    ));
    assert!(matches!(
        routed(SetKind::Shared, &[answer(About::Project, &["my employer"])]),
        Decision::Flag(_)
    ));
}

/// **The server applies the lists**: in `local`, and in a set the person never publishes, an item
/// kept out only of shared sets does not count, matched leniently.
#[test]
fn in_local_a_shared_only_item_does_not_count() {
    for kind in [SetKind::Local, SetKind::NeverPublished] {
        assert_eq!(
            routed(kind, &[answer(About::Person, &["my employer"])]),
            Decision::Clear
        );
        assert_eq!(
            routed(kind, &[answer(About::Person, &["Where I live"])]),
            Decision::Clear,
            "a paraphrased opening still names the item"
        );
        assert!(matches!(
            routed(kind, &[answer(About::Person, &["my health"])]),
            Decision::Flag(_)
        ));
    }
}

/// **It must be possible to report what someone did**, the human partner's words on 2026-10-25: a
/// report about someone else that mentions nothing kept out is cleared, in any set.
#[test]
fn a_report_about_someone_else_mentioning_nothing_is_cleared() {
    for kind in [SetKind::Local, SetKind::NeverPublished, SetKind::Shared] {
        assert_eq!(
            routed(kind, &[answer(About::SomeoneElse, &[])]),
            Decision::Clear,
            "{kind:?}"
        );
    }
}

/// A judgement about anyone goes to the person, in every set: the line the Golden Rule draws.
#[test]
fn a_judgement_about_a_person_is_flagged_in_every_set() {
    for kind in [SetKind::Local, SetKind::NeverPublished, SetKind::Shared] {
        for about in [About::SomeoneElse, About::Person] {
            assert!(
                matches!(routed(kind, &[judging(about)]), Decision::Flag(_)),
                "{kind:?} {about:?}"
            );
        }
    }
}

/// A sentence about a person that does not say whether it reports or judges is read as judging.
#[test]
fn an_unanswered_report_or_judgement_counts_as_judging() {
    let unsaid = SentenceAnswer {
        says: None,
        ..answer(About::SomeoneElse, &[])
    };
    assert!(matches!(
        routed(SetKind::Local, &[unsaid]),
        Decision::Flag(_)
    ));
}

/// **The template is the floor for what is said about someone else**: a person who drops an item
/// from their own file has loosened it for themselves alone.
#[test]
fn an_item_the_person_dropped_still_applies_to_someone_else() {
    let lists =
        ListedPreferences::parse("## Keep out of every set, including private ones\n- my health\n");
    let mentioned = [SentenceAnswer {
        mentions: vec!["my sexuality".to_owned()],
        ..answer(About::SomeoneElse, &[])
    }];
    assert!(matches!(
        route(SetKind::Local, &mentioned, &lists, &[]),
        Decision::Flag(_)
    ));
    let about_themself = [SentenceAnswer {
        mentions: vec!["my sexuality".to_owned()],
        ..answer(About::Person, &[])
    }];
    assert!(
        matches!(
            route(SetKind::Local, &about_themself, &lists, &[]),
            Decision::Flag(_)
        ),
        "an item on no list still goes to the person"
    );
}

/// The stricter placement wins for someone else: an item the template keeps out of shared sets
/// only, and the person keeps out of every set, is kept out of `local` for others too.
#[test]
fn the_stricter_placement_wins_for_someone_else() {
    let lists = ListedPreferences::parse(
        "## Keep out of every set, including private ones\n- my employer\n",
    );
    let employer = |about| {
        [SentenceAnswer {
            mentions: vec!["my employer".to_owned()],
            ..answer(about, &[])
        }]
    };
    assert!(matches!(
        route(SetKind::Local, &employer(About::SomeoneElse), &lists, &[]),
        Decision::Flag(_)
    ));
    let without =
        ListedPreferences::parse("## Keep out of every set, including private ones\n- my health\n");
    assert_eq!(
        route(SetKind::Local, &employer(About::SomeoneElse), &without, &[]),
        Decision::Clear,
        "in local the template allows someone else's employer, as it allows the person's"
    );
}

/// A person who loosens an item for themselves has not loosened it for anyone else: someone
/// else's health stays out of `local` although the person moved their own to shared-only.
#[test]
fn a_loosened_item_stays_strict_for_someone_else() {
    let lists = ListedPreferences::parse("## Keep out of shared sets\n- my health\n");
    let health = |about| {
        [SentenceAnswer {
            mentions: vec!["my health".to_owned()],
            ..answer(about, &[])
        }]
    };
    assert_eq!(
        route(SetKind::Local, &health(About::Person), &lists, &[]),
        Decision::Clear
    );
    assert!(matches!(
        route(SetKind::Local, &health(About::SomeoneElse), &lists, &[]),
        Decision::Flag(_)
    ));
}

/// A pre-pass hit goes to the person whatever the model answered.
#[test]
fn a_pre_pass_hit_flags_a_memory_the_model_would_clear() {
    use scatterhoard::review::Hit;
    let hit = Hit {
        shape: Shape::Email,
        matched: "a@example.org".to_owned(),
    };
    let decision = route(
        SetKind::Shared,
        &[answer(About::Project, &[])],
        &ListedPreferences::parse(TEMPLATE_FOR_ROUTING),
        &[hit],
    );
    assert!(matches!(decision, Decision::Flag(_)));
}

/// The project's own keep-outs count in its shared sets: any mention there goes to the person.
#[test]
fn a_project_keep_out_counts_in_a_shared_set() {
    let decision = route(
        SetKind::Shared,
        &[answer(About::Project, &["client names"])],
        &ListedPreferences::parse(TEMPLATE_FOR_ROUTING),
        &[],
    );
    assert!(matches!(decision, Decision::Flag(_)));
}

// ---------------------------------------------------------------------------------------------
// `review` and `clear` through the server.
// ---------------------------------------------------------------------------------------------

use std::sync::Arc;

use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::mcp::Server;
use scatterhoard::memory::Author;
use scatterhoard::preferences::FILE_NAME;
use scatterhoard::store::StoreLayout;
use serde_json::{Value, json};

#[path = "support/output_schema_check.rs"]
mod output_schema_check;

/// Two dimensions from two marker words, so tests can say which memories share a subject.
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
                Vector::new(vec![count("parser"), count("schedule"), 0.01])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tempdir(label: &str) -> TempDir {
    let path = std::env::temp_dir().join(format!(
        "scatterhoard-review-{label}-{}",
        MemoryId::generate()
    ));
    std::fs::create_dir_all(&path).expect("creates");
    TempDir(path)
}

/// A server whose personal preferences are `preferences`, holding `local` and `shared:projector`.
fn a_server(home: &TempDir, preferences: &str) -> Server {
    std::fs::write(home.0.join(FILE_NAME), preferences).expect("writes preferences");
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    server.set_embedder(Arc::new(MarkerEmbedder));
    server.set_layout(StoreLayout::new(home.0.clone(), None));
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    server.add_store(a_store());
    server
}

fn tool(server: &mut Server, name: &str, arguments: Value) -> (Value, bool) {
    let raw = server
        .handle(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": name, "arguments": arguments}})
            .to_string(),
        )
        .expect("answered");
    let response: Value = serde_json::from_str(&raw).expect("JSON");
    output_schema_check::check_response(name, &response);
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let refused = response["result"]["isError"] == json!(true) || response.get("error").is_some();
    (
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
        refused,
    )
}

fn remember(server: &mut Server, scope: &str, content: &str) -> String {
    let (result, refused) = tool(
        server,
        "hoard_memorize",
        json!({"scope": scope, "content": content, "confidence": 0.9, "origin": "human"}),
    );
    assert!(!refused, "{result}");
    result["id"].as_str().expect("an id").to_owned()
}

fn charter(server: &mut Server) {
    let (minted, refused) = tool(
        server,
        "hoard_sets",
        json!({"action": "mint", "scope": "shared:projector", "author": "jtcours"}),
    );
    assert!(!refused, "{minted}");
    let (result, refused) = tool(
        server,
        "hoard_memorize",
        json!({"scope": "shared:projector", "confidence": 0.9, "origin": "human",
               "content": "This set holds decisions about the projector project.",
               "keywords": ["hoard-scope-topics", "hoard-lang-en"]}),
    );
    assert!(!refused, "{result}");
}

fn review(server: &mut Server, arguments: Value) -> Value {
    let mut arguments = arguments;
    arguments["action"] = json!("review");
    let (result, refused) = tool(server, "hoard_sets", arguments);
    assert!(!refused, "{result}");
    result
}

/// Every memory a review showed, with how many sentences each was numbered into.
fn shown(result: &Value) -> Vec<(String, usize)> {
    result["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|group| group["memories"].as_array().cloned().unwrap_or_default())
        .map(|m| {
            (
                m["id"].as_str().unwrap_or_default().to_owned(),
                m["sentences"].as_array().map_or(0, Vec::len),
            )
        })
        .collect()
}

/// A `clear` answering every sentence of `id` the same way.
fn clear(
    server: &mut Server,
    scope: &str,
    id: &str,
    sentences: usize,
    about: &str,
    mentions: &[&str],
) -> (Value, bool) {
    let answers: Vec<Value> = (1..=sentences)
        .map(|n| {
            let mut answer = json!({"n": n, "about": about, "mentions": mentions});
            if about != "project" {
                answer["says"] = json!("reports");
            }
            answer
        })
        .collect();
    tool(
        server,
        "hoard_sets",
        json!({"action": "clear", "scope": scope, "answers": [{"id": id, "sentences": answers}]}),
    )
}

const PREFERENCES: &str = "## Keep out of shared sets\n- my employer\n\n## Keep out of every set, including private ones\n- my health\n";

#[test]
fn a_review_shows_unreviewed_memories_numbered_by_sentence() {
    let home = tempdir("shows");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(
        &mut server,
        "local",
        "The schedule moved. Reviews are on Fridays.",
    );
    let result = review(&mut server, json!({"scope": "local"}));
    assert_eq!(shown(&result), vec![(id, 2)]);
    assert_eq!(result["kind"], json!("personal"));
    assert!(
        result["caution"]
            .as_str()
            .unwrap_or_default()
            .contains("capable model")
    );
}

/// **The briefing comes once per review in a session**: the task in full, the person's preferences
/// and the keep-out lists on the first page, then a one-sentence task pointing back to it. The
/// caution stays on every page.
#[test]
fn a_later_review_page_in_the_same_session_is_not_briefed_again() {
    let home = tempdir("briefed");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    let first = review(&mut server, json!({"scope": "local"}));
    assert!(first.get("preferences").is_some(), "{first}");
    assert!(first.get("others_keep_out").is_some(), "{first}");
    let again = review(&mut server, json!({"scope": "local"}));
    assert!(again.get("preferences").is_none(), "{again}");
    assert!(again.get("others_keep_out").is_none(), "{again}");
    let task = again["task"].as_str().unwrap_or_default();
    assert!(task.contains("first page"), "{task}");
    assert!(task.len() < first["task"].as_str().unwrap_or_default().len());
    assert!(
        again["caution"]
            .as_str()
            .unwrap_or_default()
            .contains("capable model")
    );
}

/// **A new session is briefed again**, since what an earlier conversation was told is not in this
/// one's context. A session begins at `hoard_conventions`.
#[test]
fn a_new_session_is_briefed_again() {
    let home = tempdir("rebriefed");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    review(&mut server, json!({"scope": "local"}));
    let (_, refused) = tool(&mut server, "hoard_conventions", json!({}));
    assert!(!refused);
    let again = review(&mut server, json!({"scope": "local"}));
    assert!(again.get("preferences").is_some(), "{again}");
    assert!(again.get("others_keep_out").is_some(), "{again}");
}

/// **A change to the preferences is briefed again**, so the page that reports the change carries
/// the text it changed to.
#[test]
fn a_change_to_the_preferences_is_briefed_again() {
    let home = tempdir("rebriefed-change");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    review(&mut server, json!({"scope": "local"}));
    std::fs::write(
        home.0.join(FILE_NAME),
        format!("{PREFERENCES}- my hobbies\n"),
    )
    .expect("edits");
    let again = review(&mut server, json!({"scope": "local"}));
    assert!(
        again["preferences"]
            .as_str()
            .unwrap_or_default()
            .contains("my hobbies"),
        "{again}"
    );
}

#[test]
fn a_cleared_memory_is_not_shown_again() {
    let home = tempdir("cleared");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(&mut server, "local", "The schedule moved to Fridays.");
    review(&mut server, json!({"scope": "local"}));
    let (result, refused) = clear(&mut server, "local", &id, 1, "the person", &[]);
    assert!(!refused, "{result}");
    assert_eq!(result["cleared"], json!([id]));
    assert_eq!(result["finished"], json!(true));
    assert!(shown(&review(&mut server, json!({"scope": "local"}))).is_empty());
}

/// **Only what a review in this session showed can be cleared**: the server cannot hear the
/// person, but it can check that the memory was put in front of the session.
#[test]
fn a_memory_no_review_showed_cannot_be_cleared() {
    let home = tempdir("unshown");
    let mut server = a_server(&home, PREFERENCES);
    let shown_id = remember(&mut server, "local", "The schedule moved to Fridays.");
    review(&mut server, json!({"scope": "local"}));
    let unseen = remember(&mut server, "local", "Another schedule note.");
    let (result, _) = clear(&mut server, "local", &unseen, 1, "the person", &[]);
    assert_eq!(result["cleared"], json!([]), "{result}");
    assert!(
        result["refused"][0]["why"]
            .as_str()
            .unwrap_or_default()
            .contains("not shown")
    );
    let _ = shown_id;
}

/// The server requires an answer for every sentence it showed.
#[test]
fn an_answer_missing_a_sentence_is_refused() {
    let home = tempdir("incomplete");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(
        &mut server,
        "local",
        "The schedule moved. Reviews are on Fridays.",
    );
    review(&mut server, json!({"scope": "local"}));
    let (result, _) = clear(&mut server, "local", &id, 1, "the person", &[]);
    assert_eq!(result["cleared"], json!([]));
    assert!(
        result["refused"][0]["why"]
            .as_str()
            .unwrap_or_default()
            .contains("every sentence")
    );
}

/// **A sentence about a person with no answer for what it says goes to the person, and is not
/// refused**: on `gemma4:12b` a refusal cost a round trip in 39 answers of 177, mostly where the
/// answer changed nothing.
#[test]
fn an_answer_about_a_person_without_says_is_flagged_not_refused() {
    let home = tempdir("unsaid");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(&mut server, "local", "Sam rewrote the parser.");
    review(&mut server, json!({"scope": "local"}));
    let (result, _) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "clear", "scope": "local", "answers": [{"id": id, "sentences": [
            {"n": 1, "about": "someone else", "mentions": []}
        ]}]}),
    );
    assert_eq!(result["refused"], json!(null), "{result}");
    assert_eq!(result["flagged"][0]["id"], json!(id), "{result}");
}

/// **The review serves what applies to other people**: the template's keep-outs together with the
/// person's, so an item the person dropped can still be quoted for someone else.
#[test]
fn a_review_serves_the_keep_outs_for_other_people() {
    let home = tempdir("others");
    let mut server = a_server(
        &home,
        "## Keep out of every set, including private ones\n- my health\n- my pets\n",
    );
    remember(&mut server, "local", "The schedule moved to Fridays.");
    let result = review(&mut server, json!({"scope": "local"}));
    let others: Vec<&str> = result["others_keep_out"]
        .as_array()
        .expect("served")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for item in ["my pets", "my sexuality", "my employer", "my health"] {
        assert!(others.contains(&item), "{item} in {others:?}");
    }
    assert_eq!(
        others.iter().filter(|i| **i == "my health").count(),
        1,
        "an item on both lists is served once"
    );
}

/// A flagged memory goes to the person in full, and only the person clears it.
#[test]
fn a_flagged_memory_is_cleared_only_by_the_person() {
    let home = tempdir("flagged");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(
        &mut server,
        "local",
        "No sessions on Thursdays: a medical appointment.",
    );
    review(&mut server, json!({"scope": "local"}));
    let (result, _) = clear(&mut server, "local", &id, 1, "the person", &["my health"]);
    assert_eq!(result["flagged"][0]["id"], json!(id));
    assert!(
        result["flagged"][0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("medical")
    );
    assert_eq!(result["finished"], json!(false));
    let (result, refused) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "clear", "scope": "local", "person_cleared": [id]}),
    );
    assert!(!refused, "{result}");
    assert_eq!(result["cleared"], json!([id]));
    assert_eq!(result["finished"], json!(true));
}

/// **A rewording that brings in a pseudonym is the moment to offer it to the preferences**, so a later
/// review clears the pseudonym rather than flagging every memory that carries it. Decided 2026-10-30,
/// after a full review flagged about thirty memories for a set name the person had chosen as its
/// pseudonym eleven days earlier.
#[test]
fn the_flag_guidance_offers_a_pseudonym_to_the_always_fine_list() {
    let home = tempdir("flag-guidance-pseudonym");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(
        &mut server,
        "local",
        "No sessions on Thursdays: a medical appointment.",
    );
    review(&mut server, json!({"scope": "local"}));
    let (result, _) = clear(&mut server, "local", &id, 1, "the person", &["my health"]);
    let guidance = result["flag_guidance"].as_str().unwrap_or_default();
    assert!(guidance.contains("pseudonym"), "{result}");
    assert!(guidance.contains("Always fine to share"), "{result}");
}

/// **A flagged memory that is forgotten no longer awaits the person**: the review's own list must
/// not say a decision is waiting when the memory it concerned is gone. Found 2026-10-26, rewording a
/// flagged memory on the review's first run on this project's own set.
#[test]
fn a_forgotten_flagged_memory_no_longer_awaits_the_person() {
    let home = tempdir("forgotten-flag");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(
        &mut server,
        "local",
        "No sessions on Thursdays: a medical appointment.",
    );
    review(&mut server, json!({"scope": "local"}));
    let (result, _) = clear(&mut server, "local", &id, 1, "the person", &["my health"]);
    assert_eq!(result["awaiting_the_person"], json!(1), "{result}");
    let (forgot, refused) = tool(
        &mut server,
        "hoard_correct",
        json!({"action": "forget", "scope": "local", "id": id}),
    );
    assert!(!refused, "{forgot}");
    let result = review(&mut server, json!({"scope": "local"}));
    assert_eq!(result["awaiting_the_person"], json!(null), "{result}");
}

/// **A flagged memory changed since it was put to the person cannot be cleared by them**: they
/// decided about text they were shown, not about what it became.
#[test]
fn a_flagged_memory_changed_since_cannot_be_cleared_by_the_person() {
    let home = tempdir("amended-flag");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(
        &mut server,
        "local",
        "No sessions on Thursdays: a medical appointment.",
    );
    review(&mut server, json!({"scope": "local"}));
    clear(&mut server, "local", &id, 1, "the person", &["my health"]);
    let (amended, refused) = tool(
        &mut server,
        "hoard_correct",
        json!({"action": "amend", "scope": "local", "id": id,
               "content": "No sessions on Thursdays: HIV treatment at the clinic."}),
    );
    assert!(!refused, "{amended}");
    let (result, _) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "clear", "scope": "local", "person_cleared": [id]}),
    );
    assert_eq!(result["cleared"], json!([]), "{result}");
}

#[test]
fn the_person_cannot_clear_what_was_never_put_to_them() {
    let home = tempdir("not-flagged");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(&mut server, "local", "The schedule moved to Fridays.");
    review(&mut server, json!({"scope": "local"}));
    let (result, _) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "clear", "scope": "local", "person_cleared": [id]}),
    );
    assert_eq!(result["cleared"], json!([]));
}

/// In `local` the employer is allowed; in a shared set it goes to the person.
#[test]
fn the_server_applies_the_right_list_to_each_set() {
    let home = tempdir("lists");
    let mut server = a_server(&home, PREFERENCES);
    let local = remember(&mut server, "local", "The person works at Northwind Bank.");
    review(&mut server, json!({"scope": "local"}));
    let (result, _) = clear(
        &mut server,
        "local",
        &local,
        1,
        "the person",
        &["my employer"],
    );
    assert_eq!(result["cleared"], json!([local]));

    charter(&mut server);
    let checkout = tempdir("lists-checkout");
    let shared = remember(
        &mut server,
        "shared:projector",
        "Testing waits for weekends: the person works at Northwind Bank.",
    );
    let result = review(
        &mut server,
        json!({"scope": "shared:projector", "directory": checkout.0.display().to_string()}),
    );
    let count = shown(&result)
        .into_iter()
        .find(|(id, _)| *id == shared)
        .expect("shown")
        .1;
    let (result, _) = clear(
        &mut server,
        "shared:projector",
        &shared,
        count,
        "the person",
        &["my employer"],
    );
    assert_eq!(result["flagged"][0]["id"], json!(shared));
}

/// A set the person never publishes is reviewed like `local`.
#[test]
fn a_never_published_set_is_reviewed_like_local() {
    let home = tempdir("never");
    let mut server = a_server(
        &home,
        &format!("{PREFERENCES}\n## Sets I never publish\n- projector\n"),
    );
    charter(&mut server);
    let id = remember(
        &mut server,
        "shared:projector",
        "The person works at Northwind Bank.",
    );
    let result = review(&mut server, json!({"scope": "shared:projector"}));
    assert_eq!(result["kind"], json!("never published"));
    let count = shown(&result)
        .into_iter()
        .find(|(m, _)| *m == id)
        .expect("shown")
        .1;
    let (result, _) = clear(
        &mut server,
        "shared:projector",
        &id,
        count,
        "the person",
        &["my employer"],
    );
    assert_eq!(result["cleared"], json!([id]));
}

/// A shared set that is published names where, since its file is what counts as reviewed.
#[test]
fn reviewing_a_published_set_needs_its_destination() {
    let home = tempdir("destination");
    let mut server = a_server(&home, PREFERENCES);
    charter(&mut server);
    let (result, refused) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "review", "scope": "shared:projector"}),
    );
    assert!(refused, "{result}");
}

/// **The name the set publishes the person under is theirs as the author of their own work**, so it
/// is not a pre-pass hit, on the page or at `clear`, and the briefing names it for the model:
/// `the-review-flags-the-persons-own-published-name`.
#[test]
fn the_name_the_set_publishes_you_under_is_not_flagged() {
    let home = tempdir("published-name");
    let checkout = tempdir("published-name-checkout");
    let mut server = a_server(&home, PREFERENCES);
    charter(&mut server);
    let id = remember(
        &mut server,
        "shared:projector",
        "The repository is github.com/jtcours/projector.",
    );
    let page = review(
        &mut server,
        json!({"scope": "shared:projector", "directory": checkout.0.display().to_string()}),
    );
    assert_eq!(page["published_as"], json!("jtcours"), "{page}");
    assert!(
        page["pre_pass"].as_array().is_none_or(Vec::is_empty),
        "{page}"
    );
    let (result, refused) = clear(&mut server, "shared:projector", &id, 1, "project", &[]);
    assert!(!refused, "{result}");
    assert_eq!(result["cleared"][0], json!(id), "{result}");
}

/// Anyone else who publishes in the set is still caught, in the same sentence.
#[test]
fn a_collaborators_name_is_still_flagged_beside_yours() {
    let home = tempdir("collaborator-name");
    let checkout = tempdir("collaborator-name-checkout");
    let mut server = a_server(&home, PREFERENCES);
    let mut store = a_store();
    let mut collaborator = Memory::self_attributed(
        MemoryText::new("jbrown").expect("valid content"),
        Confidence::new(1.0).expect("in range"),
        Origin::Human,
        Timestamp::from_str("2026-10-23T09:00:00Z").expect("valid timestamp"),
    );
    collaborator.keywords = vec![Keyword::new("hoard-author").expect("valid keyword")];
    store.insert(&collaborator).expect("inserts");
    server.add_store(store);
    charter(&mut server);
    let id = remember(
        &mut server,
        "shared:projector",
        "As jbrown and jtcours agreed, keep the interface stable.",
    );
    let page = review(
        &mut server,
        json!({"scope": "shared:projector", "directory": checkout.0.display().to_string()}),
    );
    assert_eq!(page["pre_pass"][0]["id"], json!(id), "{page}");
    assert_eq!(
        page["pre_pass"][0]["hits"],
        json!([{"matched": "jbrown", "shape": "author"}]),
        "{page}"
    );
}

/// A pre-pass hit goes to the person whatever the model answers, and comes first.
#[test]
fn a_pre_pass_hit_is_listed_and_flags_the_memory() {
    let home = tempdir("prepass");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(
        &mut server,
        "local",
        "Send the schedule to priya@example.org.",
    );
    let result = review(&mut server, json!({"scope": "local"}));
    assert_eq!(result["pre_pass"][0]["id"], json!(id));
    let (result, _) = clear(&mut server, "local", &id, 1, "project", &[]);
    assert_eq!(result["flagged"][0]["id"], json!(id));
}

/// More memories than one review page shows, each clean, in id order.
fn a_page_and_one_more(server: &mut Server) -> Vec<String> {
    (0..=10)
        .map(|n| remember(server, "local", &format!("Schedule note {n} moved.")))
        .collect()
}

/// **What a review page works out about the whole set is kept between pages, and kept honest**:
/// `each-review-page-recomputes-the-whole-set`. A memory amended mid-review to carry a pre-pass hit
/// comes first on the next page, though it sorted last before.
#[test]
fn a_memory_amended_mid_review_to_carry_a_hit_comes_first() {
    let home = tempdir("prepass-amended");
    let mut server = a_server(&home, PREFERENCES);
    let ids = a_page_and_one_more(&mut server);
    let last = ids.last().expect("eleven").clone();
    let first = review(&mut server, json!({"scope": "local"}));
    assert!(shown(&first).iter().all(|(id, _)| *id != last), "{first}");
    let (amended, refused) = tool(
        &mut server,
        "hoard_correct",
        json!({"action": "amend", "scope": "local", "id": last,
               "content": "Send schedule note 10 to priya@example.org."}),
    );
    assert!(!refused, "{amended}");
    let again = review(&mut server, json!({"scope": "local"}));
    assert!(shown(&again).iter().any(|(id, _)| *id == last), "{again}");
}

/// The same when it is the preferences that change: a word added to the never-store list puts the
/// memory carrying it first.
#[test]
fn a_word_newly_listed_mid_review_puts_its_memory_first() {
    let home = tempdir("prepass-listed");
    let mut server = a_server(&home, PREFERENCES);
    let ids = a_page_and_one_more(&mut server);
    let last = ids.last().expect("eleven").clone();
    review(&mut server, json!({"scope": "local"}));
    std::fs::write(
        home.0.join(FILE_NAME),
        format!("{PREFERENCES}\n## Never store these words\n- note 10\n"),
    )
    .expect("writes preferences");
    let again = review(&mut server, json!({"scope": "local"}));
    assert!(shown(&again).iter().any(|(id, _)| *id == last), "{again}");
}

/// A memory stored mid-review is grouped by its subject on the next page, not reported as having no
/// vector because the grouping was worked out before it existed.
#[test]
fn a_memory_stored_mid_review_is_grouped_by_its_subject() {
    let home = tempdir("grouped-later");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    review(&mut server, json!({"scope": "local"}));
    let later = remember(&mut server, "local", "The schedule moved again.");
    let again = review(&mut server, json!({"scope": "local"}));
    let group = again["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|group| {
            group["memories"]
                .as_array()
                .is_some_and(|memories| memories.iter().any(|m| m["id"] == json!(later)))
        })
        .expect("shown");
    assert!(
        !group["about"].to_string().contains("not embedded"),
        "{again}"
    );
}

/// A memory changed after it was shown has to be shown again.
#[test]
fn a_memory_changed_since_it_was_shown_is_refused() {
    let home = tempdir("changed");
    let mut server = a_server(&home, PREFERENCES);
    let id = remember(&mut server, "local", "The schedule moved to Fridays.");
    review(&mut server, json!({"scope": "local"}));
    let (amended, refused) = tool(
        &mut server,
        "hoard_correct",
        json!({"action": "amend", "scope": "local", "id": id, "content": "The schedule moved to Mondays."}),
    );
    assert!(!refused, "{amended}");
    let (result, _) = clear(&mut server, "local", &id, 1, "the person", &[]);
    assert!(
        result["refused"][0]["why"]
            .as_str()
            .unwrap_or_default()
            .contains("changed")
    );
}

/// **A small review comes with context**: already-reviewed memories from each memory's subject.
#[test]
fn a_small_review_is_shown_beside_reviewed_memories_from_its_subject() {
    let home = tempdir("context");
    let mut server = a_server(&home, PREFERENCES);
    let mut ids = Vec::new();
    for n in 0..5 {
        ids.push(remember(
            &mut server,
            "local",
            &format!("The schedule note {n} moved."),
        ));
    }
    let result = review(&mut server, json!({"scope": "local"}));
    for (id, count) in shown(&result) {
        clear(&mut server, "local", &id, count, "the person", &[]);
    }
    let fresh = remember(&mut server, "local", "The schedule moved again.");
    let result = review(&mut server, json!({"scope": "local"}));
    assert_eq!(shown(&result).len(), 1);
    let context = result["groups"][0]["context"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(!context.is_empty(), "{result}");
    assert!(context.iter().all(|m| m["id"] != json!(fresh)));
}

/// Review everything in `local` and clear it, answering every sentence as about the person.
fn review_all_local(server: &mut Server) {
    let result = review(server, json!({"scope": "local"}));
    for (id, count) in shown(&result) {
        clear(server, "local", &id, count, "the person", &[]);
    }
}

/// **A word added to the never-store list reopens the reviewed memories containing it**, matched
/// by the server, and nothing else.
#[test]
fn a_new_never_store_word_reopens_only_the_memories_containing_it() {
    let home = tempdir("new-word");
    let mut server = a_server(&home, PREFERENCES);
    let migraine = remember(
        &mut server,
        "local",
        "Sessions start late after a migraine.",
    );
    remember(&mut server, "local", "The schedule moved to Fridays.");
    review_all_local(&mut server);
    std::fs::write(
        home.0.join(FILE_NAME),
        format!("{PREFERENCES}\n## Never store these words\n- migraine\n"),
    )
    .expect("edits");
    let result = review(&mut server, json!({"scope": "local"}));
    assert_eq!(
        shown(&result)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        vec![migraine]
    );
}

/// Loosening the preferences reopens nothing.
#[test]
fn loosening_the_preferences_reopens_nothing() {
    let home = tempdir("loosen");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    review_all_local(&mut server);
    std::fs::write(
        home.0.join(FILE_NAME),
        "## Keep out of shared sets\n- my employer\n",
    )
    .expect("edits");
    let result = review(&mut server, json!({"scope": "local"}));
    assert!(shown(&result).is_empty(), "{result}");
    assert!(result.get("preferences_changed").is_none(), "{result}");
}

/// **A tightened keep-out asks which groups it could apply to**, and reopens only those named.
#[test]
fn a_tightened_keep_out_reopens_only_the_groups_named() {
    let home = tempdir("tighten");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    review_all_local(&mut server);
    std::fs::write(
        home.0.join(FILE_NAME),
        format!("{PREFERENCES}- my hobbies\n"),
    )
    .expect("edits");
    let result = review(&mut server, json!({"scope": "local"}));
    let changed = &result["preferences_changed"];
    assert_eq!(changed["tightened"], json!(["my hobbies"]), "{result}");
    let group = changed["groups"][0]["group"]
        .as_str()
        .expect("a group")
        .to_owned();

    let result = review(&mut server, json!({"scope": "local", "recheck": [group]}));
    assert_eq!(shown(&result).len(), 1, "{result}");
}

/// Answering that no group is affected reopens nothing, and the question is not asked again.
#[test]
fn naming_no_group_reopens_nothing_and_settles_the_change() {
    let home = tempdir("none");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    review_all_local(&mut server);
    std::fs::write(
        home.0.join(FILE_NAME),
        format!("{PREFERENCES}- my hobbies\n"),
    )
    .expect("edits");
    review(&mut server, json!({"scope": "local"}));
    let result = review(&mut server, json!({"scope": "local", "recheck": []}));
    assert!(shown(&result).is_empty());
    let again = review(&mut server, json!({"scope": "local"}));
    assert!(again.get("preferences_changed").is_none(), "{again}");
}

/// A full review shows every memory again, whatever was reviewed before.
#[test]
fn a_full_review_shows_everything_again() {
    let home = tempdir("full");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    review_all_local(&mut server);
    let result = review(&mut server, json!({"scope": "local", "full": true}));
    assert_eq!(shown(&result).len(), 1, "{result}");
}

// ---------------------------------------------------------------------------------------------
// The gate: export refuses what nobody reviewed, and backup is never refused.
// ---------------------------------------------------------------------------------------------

fn export(server: &mut Server, scope: &str, checkout: &TempDir) -> (Value, bool) {
    tool(
        server,
        "hoard_sets",
        json!({"action": "export", "scope": scope, "directory": checkout.0.display().to_string()}),
    )
}

/// Review and clear everything waiting in `shared:projector` for `checkout`.
fn review_all_shared(server: &mut Server, checkout: &TempDir) -> Vec<String> {
    let result = review(
        server,
        json!({"scope": "shared:projector", "directory": checkout.0.display().to_string()}),
    );
    let mut ids = Vec::new();
    for (id, count) in shown(&result) {
        clear(server, "shared:projector", &id, count, "project", &[]);
        ids.push(id);
    }
    ids
}

#[test]
fn export_refuses_a_shared_set_nobody_reviewed() {
    let home = tempdir("gate");
    let checkout = tempdir("gate-checkout");
    let mut server = a_server(&home, PREFERENCES);
    charter(&mut server);
    remember(
        &mut server,
        "shared:projector",
        "The parser normalises tabs.",
    );
    let (refusal, refused) = export(&mut server, "shared:projector", &checkout);
    assert!(refused);
    assert!(
        refusal.as_str().unwrap_or_default().contains("review"),
        "{refusal}"
    );
}

/// **What is in the file counts as reviewed**, because export never wrote anything unreviewed there.
#[test]
fn after_an_export_only_new_memories_wait_for_review() {
    let home = tempdir("after");
    let checkout = tempdir("after-checkout");
    let mut server = a_server(&home, PREFERENCES);
    charter(&mut server);
    remember(
        &mut server,
        "shared:projector",
        "The parser normalises tabs.",
    );
    review_all_shared(&mut server, &checkout);
    let (result, refused) = export(&mut server, "shared:projector", &checkout);
    assert!(!refused, "{result}");

    let fresh = remember(
        &mut server,
        "shared:projector",
        "The parser counts columns from one.",
    );
    assert!(
        export(&mut server, "shared:projector", &checkout).1,
        "the new memory waits"
    );
    assert_eq!(review_all_shared(&mut server, &checkout), vec![fresh]);
    assert!(!export(&mut server, "shared:projector", &checkout).1);
}

#[test]
fn local_is_exported_without_a_review() {
    let home = tempdir("local-export");
    let out = tempdir("local-export-out");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "No sessions on Thursdays.");
    let (result, refused) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "export", "scope": "local", "file": out.0.join("local.json").display().to_string()}),
    );
    assert!(!refused, "{result}");
}

/// A set the person never publishes is refused by export, and backed up instead.
#[test]
fn a_never_published_set_is_refused_by_export_but_backed_up() {
    let home = tempdir("never-export");
    let checkout = tempdir("never-export-checkout");
    let mut server = a_server(
        &home,
        &format!("{PREFERENCES}\n## Sets I never publish\n- projector\n"),
    );
    charter(&mut server);
    let (refusal, refused) = export(&mut server, "shared:projector", &checkout);
    assert!(refused);
    assert!(
        refusal.as_str().unwrap_or_default().contains("never"),
        "{refusal}"
    );
    let (result, refused) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "backup", "scope": "shared:projector"}),
    );
    assert!(!refused, "{result}");
}

/// A backup goes where the machine says, is marked as a backup, and is counted.
#[test]
fn a_backup_is_written_where_configured_and_counted() {
    let home = tempdir("backup");
    let backups = tempdir("backup-folder");
    let mut server = a_server(&home, PREFERENCES);
    server.set_backup_directory(backups.0.clone());
    remember(&mut server, "local", "No sessions on Thursdays.");
    let (result, refused) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "backup", "scope": "local"}),
    );
    assert!(!refused, "{result}");
    let path = std::path::PathBuf::from(result["backed_up"].as_str().expect("a path"));
    assert!(
        path.starts_with(backups.0.join("local")),
        "{}",
        path.display()
    );
    let header = std::fs::read_to_string(&path).expect("reads");
    assert!(
        header
            .lines()
            .next()
            .unwrap_or_default()
            .contains("\"backup\":true"),
        "{header}"
    );
    assert_eq!(result["backups_held"], json!(1));
}

/// With nothing configured, backups go under the common directory.
#[test]
fn a_backup_defaults_to_the_common_directory() {
    let home = tempdir("backup-default");
    let mut server = a_server(&home, PREFERENCES);
    let (result, _) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "backup", "scope": "local"}),
    );
    let path = std::path::PathBuf::from(result["backed_up"].as_str().expect("a path"));
    assert!(
        path.starts_with(home.0.join("backups").join("local")),
        "{}",
        path.display()
    );
}

/// **A backup never counts as reviewed**, even put where the set is published.
#[test]
fn a_backup_copied_into_a_checkout_does_not_count_as_reviewed() {
    let home = tempdir("backup-copy");
    let checkout = tempdir("backup-copy-checkout");
    let mut server = a_server(&home, PREFERENCES);
    charter(&mut server);
    remember(
        &mut server,
        "shared:projector",
        "The parser normalises tabs.",
    );
    let (result, _) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "backup", "scope": "shared:projector"}),
    );
    let reserved = checkout.0.join("scatterhoard_mem");
    std::fs::create_dir_all(&reserved).expect("creates");
    std::fs::copy(
        result["backed_up"].as_str().expect("a path"),
        reserved.join("projector.json"),
    )
    .expect("copies");
    assert!(
        export(&mut server, "shared:projector", &checkout).1,
        "still waits for review"
    );
}

/// **A project that goes public reopens the whole set**, since its private exceptions lapse.
#[test]
fn a_project_turning_public_makes_the_next_review_full() {
    let home = tempdir("public");
    let checkout = tempdir("public-checkout");
    let mut server = a_server(&home, PREFERENCES);
    let config = checkout.0.join("scatterhoard_mem").join("config");
    std::fs::create_dir_all(&config).expect("creates");
    std::fs::write(
        config.join("sharing-preferences.json"),
        r#"{"audience": "private"}"#,
    )
    .expect("writes");
    charter(&mut server);
    remember(
        &mut server,
        "shared:projector",
        "The parser normalises tabs.",
    );
    review_all_shared(&mut server, &checkout);
    assert!(!export(&mut server, "shared:projector", &checkout).1);

    std::fs::write(
        config.join("sharing-preferences.json"),
        r#"{"audience": "public"}"#,
    )
    .expect("edits");
    let result = review(
        &mut server,
        json!({"scope": "shared:projector", "directory": checkout.0.display().to_string()}),
    );
    assert_eq!(result["audience"], json!("public"));
    assert!(
        !shown(&result).is_empty(),
        "everything is reviewed again: {result}"
    );
}

// ---------------------------------------------------------------------------------------------
// The reminder, at session start, where the agent is present to act on it.
// ---------------------------------------------------------------------------------------------

/// The JSON line `hoard_conventions` serves beside the catalog on its first page.
fn conventions_aside(server: &mut Server) -> Value {
    let raw = server
        .handle(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": "hoard_conventions", "arguments": {}}})
            .to_string(),
        )
        .expect("answered");
    let response: Value = serde_json::from_str(&raw).expect("JSON");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    let lines: Vec<&str> = text.split('\n').collect();
    match lines.as_slice() {
        [_, aside, _] => serde_json::from_str(aside).expect("a JSON aside"),
        _ => json!({}),
    }
}

/// **The car-light lesson**: a reminder given as the person signs off is forgotten by the next
/// session, so it comes at session start, counted per set.
#[test]
fn session_start_names_personal_sets_with_memories_to_review() {
    let home = tempdir("due");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    let aside = conventions_aside(&mut server);
    assert_eq!(
        aside["review_due"],
        json!([{"set": "local", "memories": 1}]),
        "{aside}"
    );
    assert!(
        aside["review_due_note"]
            .as_str()
            .unwrap_or_default()
            .contains("natural pause")
    );
}

#[test]
fn session_start_says_nothing_when_everything_is_reviewed() {
    let home = tempdir("not-due");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    review_all_local(&mut server);
    assert!(conventions_aside(&mut server).get("review_due").is_none());
}

/// A never-published set is reminded like `local`; a published one is gated at export instead.
#[test]
fn session_start_names_a_never_published_set_but_not_a_published_one() {
    let home = tempdir("due-shared");
    let mut server = a_server(
        &home,
        &format!("{PREFERENCES}\n## Sets I never publish\n- projector\n"),
    );
    charter(&mut server);
    let aside = conventions_aside(&mut server);
    let due = aside["review_due"].as_array().cloned().unwrap_or_default();
    assert!(
        due.iter()
            .any(|entry| entry["set"] == json!("shared:projector")),
        "{aside}"
    );

    let other = tempdir("due-published");
    let mut published = a_server(&other, PREFERENCES);
    charter(&mut published);
    let aside = conventions_aside(&mut published);
    let due = aside["review_due"].as_array().cloned().unwrap_or_default();
    assert!(
        due.iter()
            .all(|entry| entry["set"] != json!("shared:projector")),
        "{aside}"
    );
}

/// **A clear that answers nothing is refused**, since a small model took an empty one for a review
/// that found nothing and said so.
#[test]
fn a_clear_answering_nothing_is_refused() {
    let home = tempdir("empty-clear");
    let mut server = a_server(&home, PREFERENCES);
    remember(&mut server, "local", "The schedule moved to Fridays.");
    review(&mut server, json!({"scope": "local"}));
    let (result, refused) = tool(
        &mut server,
        "hoard_sets",
        json!({"action": "clear", "scope": "local"}),
    );
    assert!(refused, "{result}");
}

/// An unfinished review says so in words, not only in a count.
#[test]
fn an_unfinished_clear_says_the_review_is_not_finished() {
    let home = tempdir("unfinished");
    let mut server = a_server(&home, PREFERENCES);
    let first = remember(&mut server, "local", "The schedule moved to Fridays.");
    remember(&mut server, "local", "Reviews happen in the morning.");
    review(&mut server, json!({"scope": "local"}));
    let (result, _) = clear(&mut server, "local", &first, 1, "the person", &[]);
    assert!(
        result["not_finished"]
            .as_str()
            .unwrap_or_default()
            .contains("not finished"),
        "{result}"
    );
}
