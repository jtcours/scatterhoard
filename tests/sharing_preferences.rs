// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! A person's sharing preferences: copied from a template once, then theirs, and served with the
//! conventions at the start of every session.
//!
//! `documents/design/export-review.md`, *the preferences start from a template the person edits*.
//! Like `tests/conventions.rs`, these assert what the design depends on rather than the prose: that
//! the file is written once and never again, that what is served is what the file says, and that it
//! arrives whole, after the catalog, whatever its length.

use std::path::{Path, PathBuf};

#[path = "support/output_schema_check.rs"]
mod output_schema_check;

use scatterhoard::conventions;
use scatterhoard::mcp::Server;
use scatterhoard::memory::{Author, Scope};
use scatterhoard::preferences::{FILE_NAME, ListedPreferences, TEMPLATE, Unread};
use scatterhoard::store::{Store, StoreLayout};
use serde_json::{Value, json};

struct TempDir(PathBuf);

impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tempdir(label: &str) -> TempDir {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_nanos();
    let unique = format!("{nanos}-{:?}", std::thread::current().id())
        .replace(['(', ')', ' '], "")
        .replace("ThreadId", "t");
    let mut path = std::env::temp_dir();
    path.push(format!("scatterhoard-preferences-{label}-{unique}"));
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

fn a_server(layout: Option<StoreLayout>) -> Server {
    let mut server = Server::new();
    server.set_account_name(Author::new("jtcours").expect("valid author"));
    if let Some(layout) = layout {
        server.set_layout(layout);
    }
    server.add_store(Store::open_in_memory(Scope::Local).expect("opens"));
    server
}

fn a_server_in(home: &TempDir) -> Server {
    a_server(Some(StoreLayout::new(home.path().to_path_buf(), None)))
}

/// A page's text, which `hoard_conventions` serves as written rather than as JSON.
fn page_text(server: &mut Server, arguments: Value) -> String {
    let raw = server
        .handle(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "hoard_conventions", "arguments": arguments}
            })
            .to_string(),
        )
        .expect("a request with an id is answered");
    let response: Value = serde_json::from_str(&raw).expect("the response is JSON");
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a page, got {response}"))
        .to_owned()
}

/// The cursor a page's last line names, if it names one.
fn next_cursor(page: &str) -> Option<u64> {
    let last = page.rsplit('\n').next().unwrap_or_default();
    last.split("\"cursor\": ")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
}

/// Every page from the first until one says the catalog is loaded, with the file optionally
/// changed after the first.
fn all_pages(
    server: &mut Server,
    max_chars: Option<usize>,
    after_first: impl FnOnce(),
) -> Vec<String> {
    let first = match max_chars {
        Some(budget) => json!({"max_chars": budget}),
        None => json!({}),
    };
    let mut pages = vec![page_text(server, first)];
    after_first();
    while let Some(cursor) = next_cursor(pages.last().expect("at least one page")) {
        let mut arguments = json!({"cursor": cursor});
        if let Some(budget) = max_chars {
            arguments["max_chars"] = json!(budget);
        }
        pages.push(page_text(server, arguments));
    }
    pages
}

/// The document the pages' first lines join into.
fn joined(pages: &[String]) -> String {
    pages
        .iter()
        .map(|page| page.split('\n').next().unwrap_or_default())
        .collect()
}

fn document(server: &mut Server) -> Value {
    let pages = all_pages(server, None, || {});
    serde_json::from_str(&joined(&pages)).expect("the pages join into one JSON document")
}

#[test]
fn a_missing_preferences_file_is_written_from_the_template() {
    let home = tempdir("missing");
    let mut server = a_server_in(&home);
    page_text(&mut server, json!({}));
    let written = std::fs::read_to_string(home.path().join(FILE_NAME)).expect("the file exists");
    assert_eq!(written, TEMPLATE);
}

/// **Once the file exists it is the person's**, and a later template reaches new users only.
#[test]
fn an_existing_preferences_file_is_never_changed() {
    let home = tempdir("existing");
    let theirs = "# Mine\n\n## Keep out of every set\n- my cat's name\n";
    std::fs::write(home.path().join(FILE_NAME), theirs).expect("writes");
    let mut server = a_server_in(&home);
    page_text(&mut server, json!({}));
    let after = std::fs::read_to_string(home.path().join(FILE_NAME)).expect("still there");
    assert_eq!(after, theirs);
}

/// An emptied file is still the person's choice, and is not refilled.
#[test]
fn an_emptied_preferences_file_is_not_refilled() {
    let home = tempdir("emptied");
    std::fs::write(home.path().join(FILE_NAME), "").expect("writes");
    let mut server = a_server_in(&home);
    page_text(&mut server, json!({}));
    let after = std::fs::read_to_string(home.path().join(FILE_NAME)).expect("still there");
    assert_eq!(after, "");
}

/// Beside the personal store, so every server on the machine reads one copy.
#[test]
fn the_preferences_file_lives_in_the_common_directory() {
    let home = tempdir("home");
    let common = tempdir("common");
    let mut server = a_server(Some(StoreLayout::new(
        home.path().to_path_buf(),
        Some(common.path().to_path_buf()),
    )));
    page_text(&mut server, json!({}));
    assert!(common.path().join(FILE_NAME).exists(), "written to common");
    assert!(!home.path().join(FILE_NAME).exists(), "not written to home");
}

#[test]
fn the_served_preferences_are_what_the_file_says() {
    let home = tempdir("served");
    let theirs = "# Mine\n\n## Keep out of every set\n- my cat's name\n";
    std::fs::write(home.path().join(FILE_NAME), theirs).expect("writes");
    let mut server = a_server_in(&home);
    let served = &document(&mut server)["sharing_preferences"];
    assert_eq!(served["text"], json!(theirs), "{served}");
    assert_eq!(
        served["file"],
        json!(home.path().join(FILE_NAME).display().to_string())
    );
    assert!(
        !served["framing"].as_str().unwrap_or_default().is_empty(),
        "the file is never served without Scatterhoard's framing: {served}"
    );
}

/// **After the last convention**, so the catalog arrives first and whole, and the framing follows
/// the conventions it refers to.
#[test]
fn the_preferences_follow_the_last_convention() {
    let home = tempdir("order");
    let mut server = a_server_in(&home);
    let text = joined(&all_pages(&mut server, None, || {}));
    let last_convention = conventions::all().last().expect("a catalog").id;
    let convention_at = text
        .find(&format!("\"id\":\"{last_convention}\""))
        .expect("the last convention is served");
    let preferences_at = text
        .find("\"sharing_preferences\"")
        .expect("the preferences are served");
    assert!(preferences_at > convention_at);
}

/// **Paging carries any length.** A file far longer than a page still arrives whole, and every
/// page stays within the budget it was asked for.
#[test]
fn a_long_preferences_file_is_paged_rather_than_refused() {
    let home = tempdir("long");
    let theirs: String = (0..2_000)
        .map(|n| format!("- keep out detail number {n}\n"))
        .collect();
    std::fs::write(home.path().join(FILE_NAME), &theirs).expect("writes");
    let mut server = a_server_in(&home);
    let budget = 3_500;
    let pages = all_pages(&mut server, Some(budget), || {});
    for page in &pages {
        assert!(page.chars().count() <= budget, "a page overran its budget");
    }
    let document: Value = serde_json::from_str(&joined(&pages)).expect("joins into JSON");
    assert_eq!(document["sharing_preferences"]["text"], json!(theirs));
    assert_eq!(
        document["conventions"].as_array().map(Vec::len),
        Some(conventions::all().len()),
        "the whole catalog still arrives"
    );
}

/// **A session keeps the text it was served on its first page**, so an edit made between pages
/// cannot shift a cursor and leave the joined pages unparseable.
#[test]
fn an_edit_between_pages_does_not_change_what_the_session_is_served() {
    let home = tempdir("edit");
    let path = home.path().join(FILE_NAME);
    let before = "- my cat's name\n";
    std::fs::write(&path, before).expect("writes");
    let mut server = a_server_in(&home);
    let pages = all_pages(&mut server, Some(3_500), || {
        std::fs::write(
            &path,
            "- something much longer than what was there before it\n",
        )
        .expect("edits");
    });
    let document: Value = serde_json::from_str(&joined(&pages)).expect("joins into JSON");
    assert_eq!(document["sharing_preferences"]["text"], json!(before));
}

/// The next session reads the file again, so an edit takes effect at the next session start.
#[test]
fn a_new_session_reads_the_file_again() {
    let home = tempdir("reread");
    let path = home.path().join(FILE_NAME);
    std::fs::write(&path, "- my cat's name\n").expect("writes");
    let mut server = a_server_in(&home);
    document(&mut server);
    std::fs::write(&path, "- my dog's name\n").expect("edits");
    let served = &document(&mut server)["sharing_preferences"];
    assert_eq!(served["text"], json!("- my dog's name\n"));
}

/// With nowhere to keep the file, nothing is written and the catalog is served as it always was.
#[test]
fn without_a_layout_the_catalog_is_served_unchanged() {
    let mut server = a_server(None);
    let text = joined(&all_pages(&mut server, None, || {}));
    assert_eq!(text, conventions::document());
}

/// **Beside the catalog, never instead of it**: a file that cannot be read costs the session its
/// preferences, says so, and leaves the conventions arriving as they always do.
#[test]
fn an_unreadable_preferences_file_is_reported_and_the_catalog_still_arrives() {
    let home = tempdir("unreadable");
    std::fs::create_dir(home.path().join(FILE_NAME)).expect("a directory where the file goes");
    let mut server = a_server_in(&home);
    let page = page_text(&mut server, json!({}));
    let lines: Vec<_> = page.split('\n').collect();
    let [slice, aside, _last] = lines.as_slice() else {
        panic!("expected a slice, an aside and a last line: {page}");
    };
    assert_eq!(*slice, conventions::document());
    let aside: Value = serde_json::from_str(aside).expect("a JSON aside");
    let problem = aside["sharing_preferences"].as_str().unwrap_or_default();
    assert!(problem.contains(FILE_NAME), "names the file: {aside}");
}

/// **Wrapped for a text editor**, since a person may read the file without anything rendering the
/// markdown. Decided by the human partner when the template was drafted.
#[test]
fn the_template_is_wrapped_for_a_text_editor() {
    for line in TEMPLATE.lines() {
        assert!(line.chars().count() <= 78, "too long: {line:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Never store these words: a list the server refuses whatever the model decides.
// ---------------------------------------------------------------------------------------------

/// A preferences file whose never-store list holds `words`.
fn never_storing(words: &[&str]) -> String {
    let list: String = words.iter().map(|word| format!("- {word}\n")).collect();
    format!("# Mine\n\n## Never store these words\n{list}\n## Exceptions\n- none\n")
}

/// The matching entry and the text it matched, for `text` against `words`.
fn found(words: &[&str], text: &str) -> Option<(String, String)> {
    ListedPreferences::parse(&never_storing(words))
        .never_store
        .find(text)
        .map(|found| (found.entry, found.matched))
}

#[test]
fn a_listed_word_matches_whatever_its_case() {
    assert_eq!(
        found(&["HIV"], "weekly hiv treatment"),
        Some(("HIV".to_owned(), "hiv".to_owned()))
    );
}

/// "HIV" must not refuse a memory about an archive.
#[test]
fn a_listed_word_does_not_match_inside_another_word() {
    assert_eq!(found(&["HIV"], "moved the archive"), None);
    assert_eq!(found(&["gay"], "Gayle reviewed it"), None);
}

#[test]
fn a_listed_phrase_matches_as_a_phrase() {
    assert!(found(&["hepatitis C"], "treated for Hepatitis C last year").is_some());
    assert_eq!(found(&["hepatitis C"], "hepatitis and C"), None);
}

/// **The only wildcard**: a trailing `*` matches every word starting with what precedes it.
#[test]
fn a_trailing_star_matches_every_word_that_starts_with_the_entry() {
    for text in [
        "a diagnosis",
        "was diagnosed",
        "two diagnoses",
        "diagnostic",
    ] {
        assert_eq!(
            found(&["diagnos*"], text).map(|(entry, _)| entry),
            Some("diagnos*".to_owned()),
            "{text}"
        );
    }
    assert_eq!(
        found(&["diagnos*"], "undiagnosed"),
        None,
        "still a word start"
    );
}

/// **Without the star, an entry is the whole word**: no hidden plurals.
#[test]
fn without_a_star_an_entry_does_not_match_a_longer_word() {
    assert_eq!(found(&["diagnosis"], "two diagnoses"), None);
}

/// **Smart boundaries.** Chinese is written without spaces, so a boundary check would mean an entry
/// there never matched. The human partner asked for this in the first build.
#[test]
fn an_entry_in_a_script_without_spaces_matches_anywhere_in_the_text() {
    assert!(found(&["艾滋病"], "他每周四去医院治疗艾滋病。").is_some());
    assert!(found(&["エイズ"], "毎週エイズの治療").is_some());
    assert!(found(&["เอชไอวี"], "รักษาเอชไอวีทุกวันพฤหัส").is_some());
}

/// A Latin-script entry beside Chinese text is still a word: the neighbour does not join it.
#[test]
fn a_spaced_script_entry_matches_beside_text_without_spaces() {
    assert!(found(&["HIV"], "我有HIV。").is_some());
    assert!(found(&["HIV"], "每周HIV治疗").is_some());
}

/// Accented letters are letters, so they join a word as any other letter does.
#[test]
fn an_accented_neighbour_still_joins_a_word() {
    assert_eq!(found(&["gay"], "gayé"), None);
    assert!(found(&["séropositif"], "Il est séropositif.").is_some());
}

/// The template's example sits in a comment, so it is not in effect.
#[test]
fn an_entry_inside_a_comment_is_not_in_effect() {
    let text = "## Never store these words\n<!-- An example, not in effect:\n- HIV\n-->\n";
    assert_eq!(
        ListedPreferences::parse(text)
            .never_store
            .find("HIV treatment"),
        None
    );
}

/// The list ends at the next heading, so another section's items are not taken for words.
#[test]
fn the_list_ends_at_the_next_heading() {
    let text = "## Never store these words\n- HIV\n\n## Keep out of every set\n- my health\n";
    assert!(
        ListedPreferences::parse(text)
            .never_store
            .find("about my health")
            .is_none()
    );
    assert!(
        ListedPreferences::parse(text)
            .never_store
            .find("HIV")
            .is_some()
    );
}

/// The template has the section, empty: which words matter is the person's to say.
#[test]
fn the_template_lists_no_words_but_has_the_section() {
    assert!(TEMPLATE.contains("## Never store these words"));
    assert!(ListedPreferences::parse(TEMPLATE).never_store.is_empty());
}

/// A tool call's result: its payload, and whether it was refused.
fn tool(server: &mut Server, name: &str, arguments: Value) -> (Value, bool) {
    let raw = server
        .handle(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            })
            .to_string(),
        )
        .expect("a request with an id is answered");
    let response: Value = serde_json::from_str(&raw).expect("the response is JSON");
    output_schema_check::check_response(name, &response);
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let refused = response["result"]["isError"] == json!(true) || response.get("error").is_some();
    let payload = serde_json::from_str(&text).unwrap_or(Value::String(text));
    (payload, refused)
}

fn memorize(server: &mut Server, fields: Value) -> (Value, bool) {
    let mut arguments = json!({"scope": "local", "confidence": 0.9, "origin": "human"});
    for (key, value) in fields.as_object().expect("fields") {
        arguments[key] = value.clone();
    }
    tool(server, "hoard_memorize", arguments)
}

fn local_memories(server: &mut Server) -> u64 {
    let (status, _) = tool(server, "hoard_sets", json!({"action": "status"}));
    status["sets"]
        .as_array()
        .and_then(|sets| sets.iter().find(|set| set["scope"] == json!("local")))
        .and_then(|set| set["memories"].as_u64())
        .unwrap_or_else(|| panic!("no local set in {status}"))
}

fn a_server_never_storing(home: &TempDir, words: &[&str]) -> Server {
    std::fs::write(home.path().join(FILE_NAME), never_storing(words)).expect("writes");
    a_server_in(home)
}

#[test]
fn a_memory_holding_a_listed_word_is_refused_and_nothing_is_stored() {
    let home = tempdir("refused");
    let mut server = a_server_never_storing(&home, &["HIV"]);
    let (refusal, refused) = memorize(
        &mut server,
        json!({"content": "Thursday HIV treatment, so no sessions then."}),
    );
    assert!(refused, "{refusal}");
    let said = refusal.as_str().unwrap_or_default();
    assert!(said.contains("\"HIV\""), "names what matched: {said}");
    assert!(said.contains("tell the person"), "{said}");
    assert_eq!(local_memories(&mut server), 0);
}

#[test]
fn every_field_a_caller_writes_is_checked() {
    let home = tempdir("fields");
    let mut server = a_server_never_storing(&home, &["HIV"]);
    for fields in [
        json!({"content": "No sessions on Thursdays.", "why": "HIV treatment."}),
        json!({"content": "No sessions on Thursdays.", "context": "Talking about HIV."}),
        json!({"content": "No sessions on Thursdays.", "keywords": ["hiv"]}),
    ] {
        let (_, refused) = memorize(&mut server, fields.clone());
        assert!(refused, "{fields}");
    }
}

#[test]
fn a_memory_without_a_listed_word_is_stored() {
    let home = tempdir("clean");
    let mut server = a_server_never_storing(&home, &["HIV"]);
    let (result, refused) = memorize(
        &mut server,
        json!({"content": "No sessions on Thursdays; the archive review moved to Fridays."}),
    );
    assert!(!refused, "{result}");
    assert_eq!(local_memories(&mut server), 1);
}

/// **Read at every store**, so the check holds in a session that never loaded the conventions,
/// and an edit protects at once.
#[test]
fn the_list_is_read_at_every_store() {
    let home = tempdir("fresh");
    let mut server = a_server_never_storing(&home, &["dentist"]);
    page_text(&mut server, json!({}));
    std::fs::write(home.path().join(FILE_NAME), never_storing(&["HIV"])).expect("edits");
    let (_, refused) = memorize(
        &mut server,
        json!({"content": "HIV treatment on Thursdays."}),
    );
    assert!(refused, "an edit made mid-session protects at once");

    let other = tempdir("unloaded");
    let mut unloaded = a_server_never_storing(&other, &["HIV"]);
    let (_, refused) = memorize(
        &mut unloaded,
        json!({"content": "HIV treatment on Thursdays."}),
    );
    assert!(refused, "without the conventions loaded");
}

/// An amendment is a way to write text too, so it passes the same check.
#[test]
fn an_amendment_holding_a_listed_word_is_refused() {
    let home = tempdir("amend");
    let mut server = a_server_never_storing(&home, &["HIV"]);
    let (stored, _) = memorize(&mut server, json!({"content": "No sessions on Thursdays."}));
    let id = stored["id"].as_str().expect("an id").to_owned();
    let (refusal, refused) = tool(
        &mut server,
        "hoard_correct",
        json!({"action": "amend", "scope": "local", "id": id,
               "content": "No sessions on Thursdays: HIV treatment."}),
    );
    assert!(refused, "{refusal}");
}

/// A file that cannot be read already costs the session its preferences and says so; it does not
/// also stop the session storing anything at all.
#[test]
fn an_unreadable_preferences_file_does_not_stop_stores() {
    let home = tempdir("unreadable-store");
    std::fs::create_dir(home.path().join(FILE_NAME)).expect("a directory where the file goes");
    let mut server = a_server_in(&home);
    let (result, refused) = memorize(&mut server, json!({"content": "No sessions on Thursdays."}));
    assert!(!refused, "{result}");
}

/// **Folded as keywords are, `toNFKC_Casefold`, not lower-cased**: Chinese text often writes Latin
/// letters full-width, and lower-casing leaves ＨＩＶ and HIV two different words with nothing to
/// report the miss. `folding-is-lowercase-not-casefold`, over again, until this.
#[test]
fn a_full_width_form_matches_its_ordinary_form() {
    assert!(found(&["HIV"], "我有ＨＩＶ。").is_some());
    assert!(found(&["ＨＩＶ"], "weekly HIV treatment").is_some());
}

#[test]
fn a_composed_and_a_decomposed_accent_are_the_same_letter() {
    assert!(found(&["séropositif"], "Il est se\u{301}ropositif.").is_some());
    assert!(found(&["se\u{301}ropositif"], "Il est séropositif.").is_some());
}

/// The refusal quotes what the caller wrote, not its folded form.
#[test]
fn a_match_is_quoted_as_it_was_written() {
    assert_eq!(
        found(&["HIV"], "我有ＨＩＶ。").map(|(_, matched)| matched),
        Some("ＨＩＶ".to_owned())
    );
}

/// **A space beside Chinese is optional**: "HIV 阳性" and "HIV阳性" are both ordinary Chinese.
#[test]
fn a_space_beside_a_script_without_spaces_is_optional() {
    assert!(found(&["HIV阳性"], "检测结果 HIV 阳性").is_some());
    assert!(found(&["HIV 阳性"], "检测结果HIV阳性").is_some());
}

/// Only there: between two words of a spaced script the space still separates them.
#[test]
fn a_space_between_words_of_a_spaced_script_is_not_optional() {
    assert_eq!(found(&["hepatitis C"], "hepatitisC"), None);
    assert!(
        found(&["hepatitis C"], "hepatitis  C").is_some(),
        "any run of spaces"
    );
    assert_eq!(
        found(&["hepatitis"], "hepa titis"),
        None,
        "nor inside a word"
    );
}

/// **Simplified and traditional characters are different characters**, so each form is listed
/// and matched as written. The template says so; a conversion table is a decision of its own.
#[test]
fn simplified_and_traditional_forms_are_matched_as_written() {
    assert_eq!(found(&["艾滋病"], "治疗愛滋病"), None);
    assert!(found(&["艾滋病", "愛滋病"], "治疗愛滋病").is_some());
}

#[test]
fn the_template_shows_both_character_forms_in_its_example() {
    assert!(TEMPLATE.contains("艾滋病") && TEMPLATE.contains("愛滋病"));
    assert!(TEMPLATE.contains("Simplified and traditional"));
}

// ---------------------------------------------------------------------------------------------
// Every list in the file, read by one parser: a heading, then `- ` items.
// ---------------------------------------------------------------------------------------------

/// The template's two keep-out lists, read by heading, which is what lets the server apply the
/// right one to each set rather than asking a small model to.
#[test]
fn the_keep_out_lists_are_read_by_heading() {
    let lists = ListedPreferences::parse(TEMPLATE);
    assert!(lists.keep_out_shared.contains(&"my employer".to_owned()));
    assert!(lists.keep_out_every.contains(&"my health".to_owned()));
    assert!(!lists.keep_out_every.contains(&"my employer".to_owned()));
}

/// **An unfamiliar list over-flags rather than going silently unenforced**: its items are kept
/// out of every set.
#[test]
fn an_unknown_heading_s_items_count_as_kept_out_of_every_set() {
    let lists = ListedPreferences::parse("## Keep out of my work sets\n- my visa status\n");
    assert_eq!(lists.keep_out_every, vec!["my visa status".to_owned()]);
}

/// Exceptions stay prose for the model, and are never mistaken for keep-outs.
#[test]
fn exceptions_are_kept_as_written_and_never_as_keep_outs() {
    let text = "## Exceptions\n- In the theology set, while it is not public: my religion.\n";
    let lists = ListedPreferences::parse(text);
    assert_eq!(
        lists.exceptions,
        vec!["In the theology set, while it is not public: my religion.".to_owned()]
    );
    assert!(lists.keep_out_every.is_empty());
}

/// Exact values, matched as never-store words are, so the two lists cannot behave differently.
#[test]
fn always_fine_values_are_matched_like_never_store_words() {
    let lists = ListedPreferences::parse("## Always fine to share\n- me@example.com\n");
    assert!(lists.always_fine.find("write to ME@example.com").is_some());
    assert!(lists.always_fine.find("write to you@example.com").is_none());
}

#[test]
fn sets_never_published_are_read() {
    let lists = ListedPreferences::parse("## Sets I never publish\n- my-household\n");
    assert_eq!(lists.never_publish, vec!["my-household".to_owned()]);
}

/// The template never relies on the unknown-heading rule: every heading in it is one the parser
/// knows, and none of its examples is in effect.
#[test]
fn the_template_is_read_without_surprises() {
    let lists = ListedPreferences::parse(TEMPLATE);
    assert_eq!(lists.keep_out_shared.len(), 5);
    assert_eq!(
        lists.keep_out_every.len(),
        6,
        "nothing unknown fell into it"
    );
    assert!(lists.never_store.is_empty() && lists.always_fine.is_empty());
    assert!(lists.never_publish.is_empty() && lists.exceptions.is_empty());
}

/// The caution the human partner asked for, near the top, for someone setting up their privacy.
#[test]
fn the_template_cautions_that_the_review_is_done_by_a_model() {
    let top: String = TEMPLATE.lines().take(16).collect::<Vec<_>>().join("\n");
    assert!(top.contains("AI model"), "{top}");
    assert!(TEMPLATE.contains("## Always fine to share"));
    assert!(TEMPLATE.contains("## Sets I never publish"));
}

// ---------------------------------------------------------------------------------------------
// What the parser does not read is reported, never dropped in silence.
// ---------------------------------------------------------------------------------------------

/// The lines of `text` the parser reports, as line number and text.
fn unread(text: &str) -> Vec<(usize, String)> {
    ListedPreferences::parse(text)
        .unread
        .into_iter()
        .map(|Unread { line, text, .. }| (line, text))
        .collect()
}

/// **The case that filed the issue**: two values added to a list as bare lines, the way the
/// section's instruction read, took no effect and nothing said so. Decided 2026-10-31 by the human
/// partner: such a line is reported, and the file still loads, rather than read as an item or
/// refused.
#[test]
fn a_bare_line_under_a_list_heading_is_reported_and_not_read() {
    let text = "## Never store these words\n- HIV\nhepatitis\n";
    let lists = ListedPreferences::parse(text);
    assert!(lists.never_store.find("hepatitis").is_none(), "not read");
    assert!(
        lists.never_store.find("HIV").is_some(),
        "the file still loads"
    );
    assert_eq!(unread(text), vec![(3, "hepatitis".to_owned())]);
}

/// **Wherever it is**: an unknown heading's items are kept out of every set, so a bare line under one
/// is the case where a silent drop costs most. Before the first heading counts too.
#[test]
fn a_bare_line_is_reported_under_any_heading_or_none() {
    let text = "a note\n# Mine\n## Things I always avoid\nmy visa status\n";
    assert_eq!(
        unread(text),
        vec![(1, "a note".to_owned()), (4, "my visa status".to_owned())]
    );
}

/// The three bullets CommonMark allows are all items.
#[test]
fn a_plus_marks_an_item_as_a_dash_and_an_asterisk_do() {
    let text = "## Never store these words\n- HIV\n* hepatitis\n+ chlamydia\n";
    let lists = ListedPreferences::parse(text);
    assert_eq!(
        lists.never_store.written(),
        vec!["HIV", "hepatitis", "chlamydia"]
    );
    assert!(lists.unread.is_empty());
}

/// **A numbered item is reported rather than read**: numbering says order matters, which nothing
/// here honours, so the syntax is left without a meaning -- free to be given one later. A marker
/// with no space after it is not a markdown item either.
#[test]
fn a_numbered_line_and_a_marker_without_a_space_are_reported() {
    let text = "## Never store these words\n1. HIV\n-hepatitis\n";
    assert_eq!(
        unread(text),
        vec![(2, "1. HIV".to_owned()), (3, "-hepatitis".to_owned())]
    );
    assert!(ListedPreferences::parse(text).never_store.is_empty());
}

/// An empty item carries nothing to lose, so there is nothing to report.
#[test]
fn an_empty_item_is_neither_read_nor_reported() {
    let text = "## Never store these words\n-\n- \n*\n";
    let lists = ListedPreferences::parse(text);
    assert!(lists.never_store.is_empty() && lists.unread.is_empty());
}

/// **An item written inside a comment is reported**, since to the eye it sits in the list, the way a
/// word added under the template's example would. Decided 2026-10-31 by the human partner, knowing it
/// reports the dash-style comments some people write: a note they do not need costs less than a
/// never-store word that never took effect.
#[test]
fn an_item_inside_a_comment_is_reported_as_not_in_effect() {
    let text = "## Never store these words\n<!--\n- syphilis\n - hepatitis\n-->\n";
    let lists = ListedPreferences::parse(text);
    assert!(lists.never_store.is_empty(), "still not in effect");
    assert_eq!(
        lists.unread,
        vec![
            Unread {
                line: 3,
                text: "- syphilis".to_owned(),
                in_comment: true
            },
            Unread {
                line: 4,
                text: "- hepatitis".to_owned(),
                in_comment: true
            },
        ]
    );
}

/// Prose inside a comment is what comments are for, and the template's own examples are meant to
/// be there.
#[test]
fn prose_and_the_template_s_examples_inside_a_comment_are_not_reported() {
    let text = "## Never store these words\n<!-- a note\nmore of it\n -\n- HIV\n- 艾滋病\n-->\n";
    assert!(unread(text).is_empty(), "{:?}", unread(text));
}

/// **A fresh file reports nothing**, which is what keeps the report worth reading: every line of
/// the template's own explanation sits inside a comment.
#[test]
fn the_template_reports_nothing() {
    assert_eq!(ListedPreferences::parse(TEMPLATE).unread, vec![]);
}

/// **Inside a template comment, a marker appears only on a line meant to be copied**: after the
/// comment's "An example" line, never on the explanation. Decided 2026-10-31 by the human partner,
/// since a comment style that carries a dash down every line would leave a person unable to tell
/// decoration from the shape their own lines should take.
#[test]
fn the_template_marks_only_its_examples() {
    let mut in_comment = false;
    let mut in_example = false;
    for line in TEMPLATE.lines() {
        let line = line.trim();
        if line.starts_with("<!--") {
            in_comment = !line.contains("-->");
            in_example = false;
        } else if in_comment && line.contains("-->") {
            in_comment = false;
        } else if in_comment && line.starts_with("An example") {
            in_example = true;
        } else if in_comment {
            let marked = ["- ", "* ", "+ "].iter().any(|m| line.starts_with(m));
            assert_eq!(marked, in_example, "{line:?}");
        }
    }
}

/// The instruction names the marker, in every section that asks for items: "one per line" alone
/// is how the issue's two values came to be written bare.
#[test]
fn the_template_names_the_marker_wherever_it_asks_for_items() {
    for section in TEMPLATE.split("\n## ").skip(1) {
        if section.contains("one per line") {
            assert!(section.contains("\"- \""), "{section}");
        }
    }
}

/// **Served with the preferences, so the session can tell the person**, naming each line so they
/// can find it.
#[test]
fn what_is_not_read_is_served_with_the_preferences() {
    let home = tempdir("unread");
    let theirs = "# Mine\n\n## Always fine to share\n- me@example.com\ngovernance-docs\n";
    std::fs::write(home.path().join(FILE_NAME), theirs).expect("writes");
    let mut server = a_server_in(&home);
    let served = &document(&mut server)["sharing_preferences"];
    assert_eq!(
        served["not_read"],
        json!([{"line": 5, "text": "governance-docs", "in_comment": false}]),
        "{served}"
    );
    assert!(
        !served["not_read_note"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "{served}"
    );
}

/// Nothing to report adds nothing to every session.
#[test]
fn a_file_with_nothing_unread_serves_no_report() {
    let home = tempdir("clean");
    std::fs::write(home.path().join(FILE_NAME), TEMPLATE).expect("writes");
    let mut server = a_server_in(&home);
    let served = &document(&mut server)["sharing_preferences"];
    assert!(served.get("not_read").is_none(), "{served}");
    assert!(served.get("not_read_note").is_none(), "{served}");
}
