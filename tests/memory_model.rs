// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the memory data model's value types.
//!
//! The types are built to make invalid memories unrepresentable (see the correctness-by-
//! construction section of `STANDARDS.md`), so these tests concentrate on the two things the
//! compiler cannot carry: that each constructor actually rejects what it claims to reject, and
//! that the guarantees the rest of the design leans on — export eligibility, stable serialised
//! names, creation-ordered identifiers — hold in practice.

use std::collections::HashSet;
use std::str::FromStr;

use jiff::Timestamp;
use scatterhoard::memory::{
    Attribution, Author, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId,
    MemoryText, Origin, SUMMARY_CAP, Scope, SetId, SetName, ValidationError,
};
use scatterhoard::memory_file::{Expectation, Record};

fn a_memory() -> Memory {
    Memory::new(
        MemoryText::new("a memory").expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: Timestamp::from_str("2026-08-01T09:00:00Z").expect("valid timestamp"),
        },
    )
}

// ---------------------------------------------------------------------------
// MemoryId
// ---------------------------------------------------------------------------

/// The previous spelling of an id, built from the id itself rather than written out, so this cannot
/// drift from what `generate` produces.
fn uuid_hex_of(id: &MemoryId) -> String {
    let raw = id.to_u128();
    let h = format!("{raw:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[test]
fn generated_ids_are_unique() {
    let ids: HashSet<_> = (0..10_000).map(|_| MemoryId::generate()).collect();
    assert_eq!(ids.len(), 10_000, "generated ids collided");
}

/// `file-format.md` wants a consistent export order, and names memory ID *or* creation time as the
/// key. Sorting by ID has to mean sorting by creation time for those to be the same order — that
/// is the whole reason the identifier is time-ordered rather than random.
#[test]
fn ids_sort_into_creation_order() {
    let generated: Vec<_> = (0..1_000).map(|_| MemoryId::generate()).collect();

    let mut sorted = generated.clone();
    sorted.sort();

    assert_eq!(sorted, generated, "ids did not sort into the order created");
}

/// `generated-ids-can-sort-out-of-creation-order`. `uuid`'s generator can hand back an id that sorts
/// *before* one it already issued, so ordering cannot be left to it: a candidate that does not sort
/// after the last id issued is replaced by the next id after that one. The bug needs a thread
/// pre-empted across a second boundary, which a test cannot arrange, so the decision is tested here
/// with ids chosen by hand.
fn hex_id(hex: &str) -> MemoryId {
    MemoryId::parse(hex).expect("a valid hex id")
}

#[test]
fn a_candidate_that_sorts_before_the_last_id_issued_is_stepped_past_it() {
    let last = hex_id("019a3b2c-1d00-7123-8000-000000000005");
    let earlier = hex_id("019a3b2c-1c00-7fff-bfff-ffffffffffff");

    assert_eq!(
        earlier.or_next_after(last),
        hex_id("019a3b2c-1d00-7123-8000-000000000006")
    );
}

#[test]
fn a_candidate_equal_to_the_last_id_issued_is_stepped_past_it() {
    let last = hex_id("019a3b2c-1d00-7123-8000-000000000005");

    assert_eq!(
        last.or_next_after(last),
        hex_id("019a3b2c-1d00-7123-8000-000000000006")
    );
}

#[test]
fn a_candidate_that_sorts_after_the_last_id_issued_is_kept() {
    let last = hex_id("019a3b2c-1d00-7123-8000-000000000005");
    let later = hex_id("019a3b2c-1d01-7000-8abc-000000000001");

    assert_eq!(later.or_next_after(last), later);
}

/// The step skips the version and variant bits rather than carrying into them, so the stepped id is
/// still a UUIDv7 — a plain `+ 1` on this value would write `11` into the variant.
#[test]
fn stepping_past_an_id_carries_around_its_version_and_variant() {
    let last = hex_id("019a3b2c-1d00-7123-bfff-ffffffffffff");
    let earlier = hex_id("019a3b2c-1c00-7000-8000-000000000000");

    assert_eq!(
        earlier.or_next_after(last),
        hex_id("019a3b2c-1d00-7124-8000-000000000000")
    );
}

#[test]
fn an_id_round_trips_through_its_string_form() {
    let id = MemoryId::generate();
    let parsed = MemoryId::parse(&id.to_string()).expect("an id's own string form must parse");
    assert_eq!(parsed, id);
}

/// `crockford-base32-ids`. The text form is ULID's: 26 characters, no separators, and an alphabet
/// that does not read as a hash. The point is not brevity — it is that nothing about this string
/// offers a cut point the way `8-4-4-4-12` offers its first group.
#[test]
fn an_id_displays_as_twenty_six_crockford_characters() {
    let shown = MemoryId::generate().to_string();

    assert_eq!(shown.len(), 26, "expected ULID's width: {shown}");
    assert!(
        !shown.contains('-'),
        "a separator is a cut point, which is the habit this encoding exists to remove: {shown}"
    );
    assert!(
        shown
            .chars()
            .all(|c| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(c)),
        "Crockford's alphabet only, upper case, without I L O or U: {shown}"
    );
}

/// **The compatibility window.** A set travels between machines that upgrade at different times, so
/// a reader must still accept the hex form it may already hold — see `file-format.md`.
#[test]
fn the_hex_form_is_still_accepted_and_means_the_same_id() {
    let id = MemoryId::generate();
    let hex = uuid_hex_of(&id);

    let from_hex = MemoryId::parse(&hex).expect("the previous form must still parse");
    assert_eq!(
        from_hex, id,
        "the two spellings name one identifier, not two"
    );
    assert_eq!(
        from_hex.to_string(),
        id.to_string(),
        "a hex id read in must be written back out in the new form"
    );
}

/// Crockford defines mappings for the characters people and models confuse. That is what makes it
/// the right alphabet for a *recovery* mechanism rather than merely a denser one.
#[test]
fn crockford_confusables_resolve_to_the_same_id() {
    let id = MemoryId::generate();
    let shown = id.to_string();

    // Substitute the confusables *into* the string: what a bad transcription looks like.
    let mangled = shown.replace('1', "I").replace('0', "O");
    assert_eq!(
        MemoryId::parse(&mangled).expect("a confusable transcription must resolve"),
        id,
        "I/L should read as 1 and O as 0: {mangled}"
    );

    let lower = shown.to_lowercase();
    assert_eq!(
        MemoryId::parse(&lower).expect("case must not matter"),
        id,
        "Crockford decoding is case-insensitive: {lower}"
    );
}

/// **Load-bearing, and the reason to assert it rather than trust it.** `file-format.md` sorts the
/// export by id to keep merge diffs small, and keyword recall breaks ties on ascending id. Crockford
/// is order-preserving over the same bits — but that is a property of the alphabet's ordering, and a
/// hand-rolled codec can lose it without anything else failing.
#[test]
fn the_text_form_sorts_into_the_same_order_as_the_id() {
    let ids: Vec<_> = (0..500).map(|_| MemoryId::generate()).collect();

    let mut by_value = ids.clone();
    by_value.sort();

    let mut by_text = ids.clone();
    by_text.sort_by_key(ToString::to_string);

    assert_eq!(
        by_value, by_text,
        "sorting the strings must agree with sorting the ids, or the export order and the \
         recall tie-break disagree with each other"
    );
}

#[test]
fn a_malformed_id_is_rejected() {
    for bad in ["", "not-a-uuid", "12345", "  "] {
        assert!(
            MemoryId::parse(bad).is_err(),
            "`{bad}` should not parse as a memory id"
        );
    }
}

// ---------------------------------------------------------------------------
// Confidence
// ---------------------------------------------------------------------------

#[test]
fn confidence_accepts_the_whole_declared_range() {
    for value in [0.0, 0.5, 1.0] {
        let confidence = Confidence::new(value).expect("value is in range");
        assert_eq!(confidence.get(), value);
    }
}

#[test]
fn confidence_rejects_values_outside_the_range() {
    for value in [-0.000_1, 1.000_1, -1.0, 2.0] {
        assert!(
            Confidence::new(value).is_err(),
            "{value} is outside 0.0..=1.0 and should be rejected"
        );
    }
}

/// Excluding NaN at construction is what makes the total ordering below sound.
///
/// The specific variant is asserted, not just that it fails: the range check alone would reject
/// these too (nothing compares inside a range against NaN), so testing only `is_err` leaves the
/// dedicated guard unexercised. The variants are documented as different diagnoses — an
/// out-of-range value is miscalibrated, a non-finite one usually means a broken computation
/// upstream — and that distinction is only worth having if it is actually reached.
#[test]
fn confidence_rejects_non_finite_values() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            Confidence::new(value),
            Err(ValidationError::ConfidenceNotFinite),
            "{value} is not finite and should be rejected as such"
        );
    }
}

/// Recall ranks by confidence among other things. Because NaN cannot be constructed, `Confidence`
/// is totally ordered, so ranking code sorts directly instead of unwrapping a partial comparison.
#[test]
fn confidences_sort_by_value() {
    let mut values: Vec<_> = [0.9, 0.1, 0.5, 1.0, 0.0]
        .into_iter()
        .map(|v| Confidence::new(v).expect("in range"))
        .collect();
    values.sort();

    let sorted: Vec<f64> = values.iter().map(|c| c.get()).collect();
    assert_eq!(sorted, vec![0.0, 0.1, 0.5, 0.9, 1.0]);
}

#[test]
fn confidence_round_trips_through_json() {
    let confidence = Confidence::new(0.75).expect("in range");
    let json = serde_json::to_string(&confidence).expect("serialises");
    assert_eq!(json, "0.75", "confidence should be a bare JSON number");

    let parsed: Confidence = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(parsed, confidence);
}

/// Parse, don't validate: an out-of-range number in an imported file must fail at the boundary,
/// not become a `Confidence` that later code trusts.
#[test]
fn confidence_validates_on_deserialisation() {
    for json in ["1.5", "-0.5"] {
        assert!(
            serde_json::from_str::<Confidence>(json).is_err(),
            "`{json}` should not deserialise into a Confidence"
        );
    }
}

// ---------------------------------------------------------------------------
// Scope and SetName
// ---------------------------------------------------------------------------

/// "Local memories never leave the machine" lives in the type: only `Project` has an export
/// target, so there is no separate check to forget at a call site.
#[test]
fn only_project_scope_has_an_export_target() {
    let set = SetName::new("scatterhoard").expect("valid set name");
    let project = Scope::Shared(set.clone());

    assert_eq!(project.exports_to(), Some(&set));
    assert_eq!(Scope::Local.exports_to(), None);
}

/// A set name keys a store on disk, so traversal is excluded by the character set rather than
/// filtered for. Dots are permitted *inside* a name, which is what makes these cases worth pinning:
/// `.` and `..` must stay unrepresentable even though the character is legal.
#[test]
fn set_names_reject_path_traversal_and_separators() {
    for bad in [
        "..",
        ".",
        "...",
        "../secrets",
        "a/b",
        "a\\b",
        "/etc/passwd",
        "a/../b",
    ] {
        assert!(
            SetName::new(bad).is_err(),
            "`{bad}` should be rejected: a set name keys a store on disk"
        );
    }
}

#[test]
fn set_names_reject_empty_values() {
    for bad in ["", "   ", "\t"] {
        assert!(
            SetName::new(bad).is_err(),
            "`{bad:?}` should not be a valid set name"
        );
    }
}

/// One rule rather than a list of special cases: punctuation is legal inside a name and nowhere
/// else. That is also what rules out a leading dot, and with it every hidden-file name.
#[test]
fn set_names_must_begin_and_end_with_an_alphanumeric() {
    for bad in [
        "-leading",
        "trailing-",
        "_leading",
        "trailing_",
        ".hidden",
        "trailing.",
    ] {
        assert!(
            SetName::new(bad).is_err(),
            "`{bad}` should be rejected: punctuation is only legal inside a name"
        );
    }
}

/// The natural set name is the project's own, so the rule has to accept names projects actually
/// have. Dotted names are common and were rejected outright by an earlier, more defensive rule.
#[test]
fn set_names_accept_names_real_projects_have() {
    for good in [
        "scatterhoard",
        "my_project",
        "web-app-2",
        "A",
        "Acme_Web-1",
        "next.js",
        "socket.io",
        "docs.example.com",
        "asp.net-core",
        "libc-0.2",
        "a.b.c.d",
        "3d-engine",
    ] {
        assert!(
            SetName::new(good).is_ok(),
            "`{good}` should be a valid set name"
        );
    }
}

#[test]
fn set_names_are_length_bounded() {
    let long = "a".repeat(SetName::MAX_LEN + 1);
    assert!(SetName::new(&long).is_err(), "over-long set name accepted");

    let at_limit = "a".repeat(SetName::MAX_LEN);
    assert!(
        SetName::new(&at_limit).is_ok(),
        "a set name at exactly the limit should be accepted"
    );
}

/// `Scope` deliberately has no `Default` (see `memory-steering.md`) — the serialised form must not
/// smuggle one back in by making the scope omissible.
#[test]
fn scope_round_trips_through_json_and_is_required() {
    let set = SetName::new("scatterhoard").expect("valid set name");
    for scope in [Scope::Shared(set), Scope::Local] {
        let json = serde_json::to_string(&scope).expect("serialises");
        let parsed: Scope = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(parsed, scope);
    }

    assert!(
        serde_json::from_str::<Scope>("null").is_err(),
        "a missing scope must not deserialise into a default"
    );
}

/// **Every spelling of a scope that any client might send has to parse.**
///
/// `tool-surface.md`, *publish the centre of MCP, accept the edges*. The published form is the
/// scalar string, because a nested `oneOf` is reduced to `{}` by at least one client and an object
/// argument then arrives as text. But the object form is what the file format writes, what earlier
/// callers send, and what a client that *does* preserve alternations will keep producing — so it
/// keeps parsing.
///
/// Being liberal here is not laxity. A strict server is normally right because strictness is what
/// stops clients drifting; that argument needs influence over the client, and this one has none.
/// Refusing the object form would not correct anybody, it would only make the memories unreachable.
///
/// The delimiter is safe by construction: `SetName` permits letters, digits, `-`, `_` and `.`, so a
/// set name can never contain a colon and the first one is always the separator.
#[test]
fn a_scope_parses_from_every_spelling_a_client_might_send() {
    let shared = Scope::Shared(SetName::new("scatterhoard").expect("valid set name"));

    // The published spelling, and the one the store's `meta` row has always used.
    assert_eq!(
        serde_json::from_str::<Scope>(r#""shared:scatterhoard""#).expect("string form"),
        shared
    );
    assert_eq!(
        serde_json::from_str::<Scope>(r#""local""#).expect("string form"),
        Scope::Local
    );

    // The form the file format writes and older callers send. Still accepted, deliberately.
    assert_eq!(
        serde_json::from_str::<Scope>(r#"{"shared":"scatterhoard"}"#).expect("object form"),
        shared
    );

    // What is written stays what it always was: the object form, so no memory file changes.
    assert_eq!(
        serde_json::to_string(&shared).expect("serialises"),
        r#"{"shared":"scatterhoard"}"#
    );
    assert_eq!(
        serde_json::to_string(&Scope::Local).expect("serialises"),
        r#""local""#
    );
}

/// Liberal about spelling is not liberal about meaning. A scope that names nothing valid is still
/// refused, and the refusal has to name both accepted forms — a caller reaching this has just been
/// told by the schema to send a string, and needs to know which strings work.
#[test]
fn an_unparseable_scope_is_refused_and_names_both_forms() {
    for bad in [
        r#""shared:""#,
        r#""shared:not a set name""#,
        r#""nonsense""#,
        "null",
        "42",
    ] {
        let error = serde_json::from_str::<Scope>(bad)
            .expect_err(&format!("{bad} should not parse as a scope"));
        let text = error.to_string();
        assert!(
            text.contains("local") && text.contains("shared"),
            "the refusal for {bad} should name both spellings, got: {text}"
        );
    }
}

// ---------------------------------------------------------------------------
// Keyword
// ---------------------------------------------------------------------------

/// A keyword keeps the case it was written with. It used to be lowercased at construction, which
/// destroyed the caller's spelling irreversibly and reported the altered value back as though it
/// were the original — `MemoryView` is a whole memory, so `MemoryView` was not what came back.
///
/// The neighbouring `MemoryText` already said why: normalising inside it *"would change what was
/// recorded."* A keyword is recorded content too, and only incidentally a lookup key. It also
/// reaches the embedding (see `embedding_input`), where case is information a tokeniser uses.
#[test]
fn keywords_keep_the_case_they_were_written_with() {
    for spelling in ["MemoryView", "STANDARDS", "main.rs", "issue-426"] {
        assert_eq!(
            Keyword::new(spelling).expect("valid keyword").as_str(),
            spelling,
            "`{spelling}` came back spelled differently from how it was stored"
        );
    }
}

/// Whitespace around a keyword is still trimmed, because it is not content — nobody means to write
/// a keyword with a leading tab, and a keyword may not contain interior whitespace at all.
#[test]
fn keywords_trim_surrounding_whitespace() {
    for spelling in ["  recall  ", "\trecall\n"] {
        assert_eq!(
            Keyword::new(spelling).expect("valid keyword").as_str(),
            "recall"
        );
    }
}

/// Matching stays case-insensitive: the *lookup key* is folded, while the stored value is not. That
/// separation is the whole fix — normalisation belonged to the index and was being applied to the
/// record.
#[test]
fn keywords_fold_to_one_lookup_key_regardless_of_case() {
    let canonical = Keyword::new("recall").expect("valid keyword");
    for spelling in ["Recall", "RECALL", "rEcAlL"] {
        assert_eq!(
            Keyword::new(spelling).expect("valid keyword").folded(),
            canonical.folded(),
            "`{spelling}` must look up the same memories as `recall`"
        );
    }
}

#[test]
fn keywords_reject_empty_and_multi_word_values() {
    for bad in ["", "   ", "two words", "a\tb"] {
        assert!(
            Keyword::new(bad).is_err(),
            "`{bad}` should not be a valid keyword"
        );
    }
}

// ---------------------------------------------------------------------------
// MemoryText
// ---------------------------------------------------------------------------

#[test]
fn memory_text_rejects_empty_and_whitespace_only_values() {
    for bad in ["", " ", "\n", "\t  \n"] {
        assert!(
            MemoryText::new(bad).is_err(),
            "`{bad:?}` carries no content and should be rejected"
        );
    }
}

/// Unlike a keyword, memory text is the memory. Trimming its edges is fine; touching its interior
/// would alter what was recorded.
#[test]
fn memory_text_preserves_its_interior_exactly() {
    let original = "Line one.\n\n  Indented line, with  double  spaces.";
    let text = MemoryText::new(format!("  {original}  ")).expect("valid text");
    assert_eq!(text.as_data(), original);
}

// ---------------------------------------------------------------------------
// Author and Origin
// ---------------------------------------------------------------------------

/// Each memory is one line of the exported file (see `multi-set-sync.md`), so a value carrying a
/// newline would break the format's line granularity.
#[test]
fn authors_reject_control_characters() {
    for bad in ["ada\nlovelace", "ada\rlovelace", "ada\0lovelace"] {
        assert!(
            Author::new(bad).is_err(),
            "`{bad:?}` should be rejected: it would break the one-memory-per-line format"
        );
    }
}

#[test]
fn authors_reject_empty_values() {
    for bad in ["", "   "] {
        assert!(Author::new(bad).is_err(), "`{bad:?}` is not an author");
    }
}

/// These names appear in every exported memory, so changing one is a format change. Pinning them
/// here makes that a test failure rather than a silent incompatibility.
#[test]
fn origin_has_stable_serialised_names() {
    let expected = [
        (Origin::Human, "\"human\""),
        (Origin::Llm, "\"llm\""),
        (Origin::Joint, "\"joint\""),
    ];

    for (origin, json) in expected {
        assert_eq!(serde_json::to_string(&origin).expect("serialises"), json);
        assert_eq!(
            serde_json::from_str::<Origin>(json).expect("deserialises"),
            origin
        );
    }
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

/// `created` and `modified` each pair a timestamp with the author responsible, so a revision
/// cannot bump the time and leave the previous author attached.
#[test]
fn a_new_memory_was_last_modified_when_it_was_created() {
    let memory = a_memory();
    assert_eq!(memory.created, memory.modified);
}

#[test]
fn revising_a_memory_moves_modified_and_leaves_created_alone() {
    let mut memory = a_memory();
    let created = memory.created.clone();

    let reviser = MemoryId::generate();
    let when = Timestamp::from_str("2026-08-02T11:00:00Z").expect("valid timestamp");
    memory.revise(reviser, when);

    assert_eq!(memory.created, created, "created must not move");
    assert_eq!(memory.modified.by, reviser);
    assert_eq!(memory.modified.at, when);
}

/// Two memories created in the same call must still be distinguishable.
#[test]
fn each_new_memory_gets_its_own_id() {
    assert_ne!(a_memory().id, a_memory().id);
}

/// `why` is deliberately optional in the type even though the conventions push hard for it. A type
/// that demanded reasoning would make the LLM invent some when it has none, which produces worse
/// memories than an honest omission — so this belongs in convention, not in the compiler.
#[test]
fn a_memory_can_be_stored_without_reasoning() {
    assert!(a_memory().why.is_none());
}

// ---------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------

/// The design says the canonical label set is defined by the software rather than left to
/// convention, so `ALL` has to actually be all of them. `LinkLabel::ordinal` is an exhaustive
/// match, which makes a new variant a compile error there; this test catches `ALL` going stale.
#[test]
fn every_link_label_appears_in_all_exactly_once() {
    let mut seen: Vec<usize> = LinkLabel::ALL
        .iter()
        .map(|l| l.ordinal().expect("every label in ALL is a known one"))
        .collect();
    seen.sort();
    seen.dedup();

    assert_eq!(
        seen.len(),
        LinkLabel::ALL.len(),
        "LinkLabel::ALL contains a duplicate"
    );
    assert_eq!(
        seen,
        (0..LinkLabel::ALL.len()).collect::<Vec<_>>(),
        "LinkLabel::ALL is missing a variant"
    );
}

/// These names appear in every exported memory that carries a link, so changing one is a format
/// migration rather than an edit. Pinning the whole vocabulary makes that a test failure instead of
/// a silent incompatibility — and the count is asserted so that adding a label is a deliberate act.
#[test]
fn link_labels_have_stable_serialised_names() {
    let expected = [
        (LinkLabel::RelatedTo, "related_to"),
        (LinkLabel::RefersTo, "refers_to"),
        (LinkLabel::Causes, "causes"),
        (LinkLabel::ResultsFrom, "results_from"),
        (LinkLabel::MotivatedBy, "motivated_by"),
        (LinkLabel::Supports, "supports"),
        (LinkLabel::Contradicts, "contradicts"),
        (LinkLabel::Supersedes, "supersedes"),
        (LinkLabel::SupersededBy, "superseded_by"),
        (LinkLabel::Resolves, "resolves"),
        (LinkLabel::DependsOn, "depends_on"),
        (LinkLabel::ExceptionTo, "exception_to"),
        (LinkLabel::Violates, "violates"),
        (LinkLabel::InstanceOf, "instance_of"),
    ];

    assert_eq!(
        expected.len(),
        LinkLabel::ALL.len(),
        "the vocabulary changed size; see documents/design/memory-data-model.md before updating"
    );

    for (label, name) in expected {
        let json = format!("\"{name}\"");
        assert_eq!(serde_json::to_string(&label).expect("serialises"), json);
        assert_eq!(
            serde_json::from_str::<LinkLabel>(&json).expect("deserialises"),
            label
        );
    }
}

/// `PRECEDES`/`FOLLOWS` are deferred pending a corpus that can test them fairly, so they must not
/// be quietly reachable through the serialised form in the meantime. Since 2026-09-25 they *parse*
/// -- an unknown label is carried rather than refused -- but only as `Unrecognised`, which nothing
/// walks and no call can author.
#[test]
fn deferred_link_labels_parse_only_as_unrecognised() {
    for name in ["precedes", "follows", "explains", "requires"] {
        let label: LinkLabel =
            serde_json::from_str(&format!("\"{name}\"")).expect("an unknown label is carried");
        assert!(
            matches!(&label, LinkLabel::Unrecognised(unknown) if unknown.as_str() == name),
            "{name} is not in the vocabulary and must not parse as a known label: {label:?}"
        );
    }
}

/// **An unknown label is written back exactly as it was read**, so a file from a newer build
/// round-trips through this one with its checksums intact. Spelling included: nothing here
/// normalises case or separators, because a normalised label is a different label to the build
/// that wrote it.
#[test]
fn an_unrecognised_label_round_trips_as_written() {
    for name in ["is_sketch", "Is_Sketch", "is sketch", "é"] {
        let json = serde_json::to_string(name).expect("a string serialises");
        let label: LinkLabel = serde_json::from_str(&json).expect("an unknown label is carried");
        assert_eq!(label.name(), name);
        assert_eq!(serde_json::to_string(&label).expect("serialises"), json);
    }
}

/// **A known spelling never becomes `Unrecognised`.** Otherwise a file could carry `supersedes`
/// opaquely and walk past every check that matches on `Supersedes`.
#[test]
fn a_known_name_always_parses_as_its_known_label() {
    for label in LinkLabel::ALL {
        assert!(label.is_recognised());
        assert_eq!(LinkLabel::from_name(label.name()), label);
    }
}

/// Within-set links are the common case, and the file format exists to keep merge diffs small, so
/// a same-set target serialises as a bare id with no wrapper.
#[test]
fn a_same_set_link_round_trips_as_a_bare_id() {
    let target = MemoryId::generate();
    let link = Link {
        label: LinkLabel::Supersedes,
        target: LinkTarget::SameSet(target),
    };

    let json = serde_json::to_string(&link).expect("serialises");
    assert!(
        json.contains(&format!("\"target\":\"{target}\"")),
        "a same-set target should serialise as a bare id, got {json}"
    );

    let parsed: Link = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(parsed, link);
}

#[test]
fn a_cross_set_link_carries_the_set_name_and_round_trips() {
    let link = Link {
        label: LinkLabel::RefersTo,
        target: LinkTarget::CrossSet {
            set: SetName::new("other-project").expect("valid set name"),
            id: MemoryId::generate(),
            keywords: vec![
                Keyword::new("Recall").expect("valid keyword"),
                Keyword::new("hubness").expect("valid keyword"),
            ],
            set_id: None,
        },
    };

    let json = serde_json::to_string(&link).expect("serialises");
    let parsed: Link = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(parsed, link);
}

/// A cross-set reference is id *plus set name*; the id alone cannot identify a memory once several
/// sets are loaded. Dropping the set name has to fail rather than resolve against the local set.
#[test]
fn a_cross_set_target_without_a_set_name_is_rejected() {
    let json = r#"{"label":"refers_to","target":{"id":"019fb4dc-d840-78df-bc3b-fcb5b45613bd"}}"#;
    assert!(
        serde_json::from_str::<Link>(json).is_err(),
        "a cross-set target missing its set name must not deserialise"
    );
}

/// Both target kinds resolve to an id without the caller matching on the variant, since most
/// traversal only needs the id.
#[test]
fn every_link_target_exposes_its_id() {
    let id = MemoryId::generate();

    assert_eq!(LinkTarget::SameSet(id).id(), id);
    assert_eq!(
        LinkTarget::CrossSet {
            set: SetName::new("other-project").expect("valid set name"),
            id,
            keywords: Vec::new(),
            set_id: None,
        }
        .id(),
        id
    );
}

// ---------------------------------------------------------------------------
// Link targets across the tool boundary
// ---------------------------------------------------------------------------

/// **A cross-set link has to be expressible without a nested alternation.**
///
/// `tool-surface.md`, *publish the centre of MCP, accept the edges*. `LinkTarget` is an untagged
/// enum — a bare string for a same-set id, an object for a cross-set one — which is the same shape
/// as `scope` and is reduced to `{}` by the same client, after which the object arrives as text.
///
/// It cannot take `scope`'s fix. A `"set:id"` string would drop `keywords`, and those exist *"so
/// that a dangling reference has something to be re-found by"* — precisely the field that makes a
/// cross-set link survive the target going missing, so a spelling that loses it is worse than the
/// problem. Instead the alternation moves out into siblings on the link: `target` is always an id,
/// with optional `target_set` and `target_keywords` beside it. Objects with named properties do
/// survive both clients; only the alternation inside them does not.
///
/// All three spellings parse, and the *structure* of what is written is unchanged — the nested
/// object survives. The id's own spelling did move, deliberately, with `crockford-base32-ids`.
#[test]
fn a_link_parses_from_every_spelling_a_client_might_send() {
    let id = "019fd978-6a2f-7a22-8742-deba83c94d5f";

    let same_set: Link =
        serde_json::from_str(&format!(r#"{{"label":"related_to","target":"{id}"}}"#))
            .expect("the bare id form");
    assert_eq!(same_set.target.set(), None);
    // Compared as an id rather than as text: a hex id read in is written back in the new form, so
    // asserting the spelling survives would assert the compatibility window shut.
    assert_eq!(
        same_set.target.id(),
        MemoryId::parse(id).expect("the previous spelling still parses")
    );

    // The nested form, which is what Serialize still writes and what the file format holds.
    let nested: Link = serde_json::from_str(&format!(
        r#"{{"label":"related_to","target":{{"set":"other","id":"{id}","keywords":["alpha"]}}}}"#
    ))
    .expect("the nested object form");

    // The flat form, which is what the schema now publishes.
    let flat: Link = serde_json::from_str(&format!(
        r#"{{"label":"related_to","target":"{id}","target_set":"other","target_keywords":["alpha"]}}"#
    ))
    .expect("the flat sibling form");

    assert_eq!(
        flat, nested,
        "the flat spelling must produce exactly the nested one, or the two forms mean different \
         things and the file format has quietly forked"
    );

    // And the link's structure is untouched: what goes out is still the nested object. The id
    // inside it is now Crockford, which is the `format_version` bump, not an accident here.
    let written = serde_json::to_string(&flat).expect("serialises");
    assert!(
        written.contains(r#""target":{"#),
        "serialising must still write the nested form, or every exported memory file changes: \
         {written}"
    );
}

/// `target_set` without a `target` is now constructible, which the nested form made impossible.
/// Accepted deliberately — the alternative is not a stricter schema but an emptier one, since the
/// client that would have enforced it is the client that discards it — so the parser has to be what
/// refuses, and it has to say what is missing.
#[test]
fn a_flat_link_target_without_an_id_is_refused() {
    let error = serde_json::from_str::<Link>(r#"{"label":"related_to","target_set":"other"}"#)
        .expect_err("a set with no id is not a target");
    let text = error.to_string();
    assert!(
        text.contains("target"),
        "the refusal should name the missing field: {text}"
    );
}

// ---------------------------------------------------------------------------
// `local` is reserved as a set name.
//
// `target-set-local-names-a-different-set`: the word means `Scope::Local` in a `scope` and, until
// this was reserved, a *shared set called local* in a link's `target_set` -- two different sets, one
// spelling, in one JSON object. Refusing at the constructor rather than at each field is what makes
// the ambiguity unrepresentable instead of merely detected: it closes `hoard_sets create`, the
// `shared:local` scalar form, a file header, and a link target in one place.
// ---------------------------------------------------------------------------

#[test]
fn local_is_not_a_legal_set_name() {
    let refused = SetName::new("local").expect_err("`local` must be reserved");
    let message = refused.to_string();
    assert!(
        message.contains("local"),
        "the refusal should name the word it reserved: {message}"
    );
}

/// Case-folded, because the confusion is about how the word *reads* rather than how it is spelled.
/// A set called `Local` collides with `Scope::Local` for every human who meets it.
#[test]
fn local_is_reserved_whatever_its_case() {
    for spelling in ["Local", "LOCAL", "LoCaL"] {
        assert!(
            SetName::new(spelling).is_err(),
            "{spelling} should be reserved too"
        );
    }
}

/// The reservation is one word, not a vocabulary. `shared` is unambiguous — the scalar form is
/// `shared:<name>`, and the first colon is always the separator, so `shared:shared` reads exactly
/// one way. Reserving more than is needed costs names for nothing.
#[test]
fn only_local_is_reserved() {
    for name in [
        "shared",
        "locale",
        "local-notes",
        "notlocal",
        "scatterhoard",
    ] {
        assert!(
            SetName::new(name).is_ok(),
            "{name} is not ambiguous and should stay available"
        );
    }
}

/// The scalar spelling that used to name a shared set called `local` is now refused outright, which
/// is the whole point: `local` and `shared:local` could not be told apart by a reader.
#[test]
fn the_shared_local_scalar_form_is_refused() {
    let shared_local: Result<Scope, _> = serde_json::from_str::<Scope>("\"shared:local\"");
    assert!(
        shared_local.is_err(),
        "`shared:local` should no longer parse"
    );
    let plain: Scope = serde_json::from_str("\"local\"").expect("plain local still parses");
    assert_eq!(plain, Scope::Local);
}

// ---------------------------------------------------------------------------
// The lookup key is `toNFKC_Casefold`, not lowercase.
//
// `folding-is-lowercase-not-casefold`. Lowercasing was a partial normalisation: enough that a
// caller could not treat matching as byte identity, not enough to unify what a non-English author
// actually writes. The served convention names the standard rather than describing the
// transformation, because one is a sentence and the other is a paragraph that stays incomplete.
// ---------------------------------------------------------------------------

/// Case folding proper, which lowercase mapping does not reach.
#[test]
fn the_lookup_key_folds_beyond_lowercase() {
    for (a, b, what) in [
        ("STRASSE", "Straße", "German sharp s: ß folds to ss"),
        ("ﬁle", "file", "the fi ligature decomposes"),
        (
            "\u{13F0}",
            "\u{13F8}",
            "Cherokee, which folds to upper case",
        ),
        // Pins the *order* of the composition rather than its parts. Folded before it is
        // normalised, `U+1D2C` stays `A` and never reaches `a`; 664 characters below U+30000
        // behave this way, so dropping the leading NFKC is a silent, wide loss.
        (
            "\u{1D2C}",
            "a",
            "modifier letter capital A, which needs normalising before folding",
        ),
    ] {
        assert_eq!(
            Keyword::new(a).expect("valid").folded(),
            Keyword::new(b).expect("valid").folded(),
            "{what}: `{a}` and `{b}` must share one lookup key"
        );
    }
}

/// Compatibility normalisation, which is what a CJK author actually needs: an input method emits
/// fullwidth Latin and halfwidth katakana, so the same keyword typed twice in two input modes would
/// otherwise be two keywords, invisibly.
#[test]
fn the_lookup_key_normalises_width() {
    for (a, b, what) in [
        ("日本語ＮＬＰ", "日本語NLP", "fullwidth versus ASCII Latin"),
        ("ｶﾀｶﾅ", "カタカナ", "halfwidth versus fullwidth katakana"),
    ] {
        assert_eq!(
            Keyword::new(a).expect("valid").folded(),
            Keyword::new(b).expect("valid").folded(),
            "{what}: `{a}` and `{b}` must share one lookup key"
        );
    }
}

/// **What must not be unified**, which is as much of the contract as what must. Each of these is a
/// distinction a reader means to draw, and a normalisation that erased it would lose memories
/// rather than find them.
#[test]
fn the_lookup_key_leaves_genuinely_different_words_apart() {
    for (a, b, what) in [
        (
            "かたかな",
            "カタカナ",
            "hiragana and katakana are different scripts",
        ),
        (
            "汉字",
            "漢字",
            "simplified and traditional Han are different characters",
        ),
        (
            "İSTANBUL",
            "istanbul",
            "the Turkish dotted I keeps its combining mark",
        ),
        (
            "budget",
            "context-budget",
            "different words, which no normalisation should merge",
        ),
    ] {
        assert_ne!(
            Keyword::new(a).expect("valid").folded(),
            Keyword::new(b).expect("valid").folded(),
            "{what}: `{a}` and `{b}` must stay separate"
        );
    }
}

/// A caseless script is untouched, so adopting this cost CJK keywords nothing.
#[test]
fn a_caseless_script_is_left_exactly_as_written() {
    for word in ["漢字", "ひらがな", "カタカナ", "한국어"] {
        assert_eq!(
            Keyword::new(word).expect("valid").folded(),
            word,
            "`{word}` is caseless and must be its own lookup key"
        );
    }
}

// ---------------------------------------------------------------------------
// Set identity
// ---------------------------------------------------------------------------

/// A set's identity is minted, spelled and read back in the one form this project spells ids in.
///
/// **Crockford base32, like a memory id, and for the same reason** — `documents/design/memory-data-model.md`
/// on the truncation instinct. A set id is read by people in a file header and in a refusal message,
/// which is exactly where a hex UUID's `8-4-4-4-12` grouping invites a cut at a boundary carrying no
/// identity.
#[test]
fn a_set_id_round_trips_through_its_text_form() {
    let id = SetId::generate();
    let text = id.to_string();

    assert_eq!(text.len(), 26, "26 characters, like every id here: {text}");
    assert_eq!(SetId::parse(&text).expect("its own spelling"), id);
}

/// Two mints are two identities. Trivial, and it is the property the whole scheme rests on.
#[test]
fn two_minted_set_ids_differ() {
    assert_ne!(SetId::generate(), SetId::generate());
}

/// **Only the one spelling is accepted, unlike `MemoryId`.** That type reads hex because sets in the
/// wild already carried ids written that way and a set travels between machines that upgrade at
/// different times. No set id has ever been written in any form, so accepting a second spelling
/// would be inventing compatibility with nothing.
#[test]
fn a_set_id_does_not_accept_the_hex_spelling() {
    let hex = "018f2c4a-1b3d-7e5f-8a9b-0c1d2e3f4a5b";
    assert!(
        SetId::parse(hex).is_err(),
        "a hex uuid is not a set id spelling"
    );
    assert!(SetId::parse("").is_err(), "empty");
    assert!(SetId::parse("not-an-id").is_err(), "nonsense");
}

/// It serialises as its text form, not as the `Uuid`'s. The header is a line people read in a diff.
#[test]
fn a_set_id_serialises_as_its_text_form() {
    let id = SetId::generate();
    let json = serde_json::to_value(id).expect("serialises");
    assert_eq!(json, serde_json::json!(id.to_string()));
    assert_eq!(
        serde_json::from_value::<SetId>(json).expect("round trips"),
        id
    );
}

// ---------------------------------------------------------------------------
// The derived summary line
// ---------------------------------------------------------------------------

/// The projection an index is built from: `documents/design/recall-mechanisms.md`, and
/// `recall-summary-output-mode` in `TRACKING.md` for the measurements behind the cap.

#[test]
fn a_memory_shorter_than_the_cap_is_its_own_summary() {
    let text = MemoryText::new("Push is the marked case.").expect("valid text");
    assert_eq!(text.summary_as_data(), "Push is the marked case.");
}

/// The cap is a bound, not a target. The check script that measured the corpus searched a window of
/// `CAP + 1` and produced a 121-character line, which is why this is asserted rather than assumed.
#[test]
fn a_summary_never_exceeds_the_cap() {
    for content in [
        // Nothing to cut at: one word longer than the whole cap.
        "x".repeat(SUMMARY_CAP * 2),
        // A sentence ending exactly at the cap, with the space that ends it just beyond.
        format!("{}. and then some more text after it", "word ".repeat(30)),
        // Ordinary prose, cut mid-word.
        "The fix is to push a derived index line and pull the watch on demand, one line per watch, \
         taken from the leading sentence of the content rather than from a new authored field."
            .to_owned(),
    ] {
        let summary = MemoryText::new(&content)
            .expect("valid text")
            .summary_as_data();
        assert!(
            summary.chars().count() <= SUMMARY_CAP,
            "{} characters is over the cap: {summary:?}",
            summary.chars().count()
        );
    }
}

/// A sentence that ends inside the cap is where the line ends, terminator included.
#[test]
fn a_summary_ends_at_the_last_sentence_that_fits() {
    let text = MemoryText::new(
        "Push is the marked case. Most accumulation stays passive. The rest is reached by ordinary \
         recall, which costs nothing until somebody asks.",
    )
    .expect("valid text");

    assert_eq!(
        text.summary_as_data(),
        "Push is the marked case. Most accumulation stays passive.\u{2026}"
    );
}

/// **The last sentence end that fits, not the first.** Taking the first would spend a quarter of
/// the budget here, and would truncate to nothing at all on an abbreviation near the front.
#[test]
fn an_early_sentence_end_does_not_cut_the_line_short() {
    let text = MemoryText::new(
        "No. The per-record envelope is 411 characters and does not shrink with the prose, so \
         terseness does not save it. Fifteen watches cost more than the catalog.",
    )
    .expect("valid text");

    assert_eq!(
        text.summary_as_data(),
        "No. The per-record envelope is 411 characters and does not shrink with the prose, so \
         terseness does not save it.\u{2026}"
    );
}

/// **A one-word sentence is not a boundary**, because cutting there leaves a line that names
/// nothing. `a-numbered-opening-summarises-to-its-number`: the parts of `coding-lessons`' adoption
/// guide opened with their number, and each summarised to `1.…` until its first sentence was
/// rewritten to end inside the cap. A one-word answer at the front is the same shape.
#[test]
fn a_one_word_sentence_is_not_where_the_line_ends() {
    for (opening, rest) in [
        (
            "1.",
            "Recall before writing: before each change that adds or alters behaviour, recall one \
             sentence describing it, and read what comes back before writing any code at all",
        ),
        (
            "No.",
            "The per-record envelope is four hundred and eleven characters wide and does not \
             shrink with the prose at all, which is what the arithmetic turns on in the end",
        ),
    ] {
        let content = format!("{opening} {rest}");
        let summary = MemoryText::new(&content)
            .expect("valid text")
            .summary_as_data();

        let text = summary
            .strip_suffix('\u{2026}')
            .expect("a cut line is marked");
        assert!(
            text.len() > opening.len() + 1 + rest.len() / 2,
            "a one-word sentence took the line: {summary:?}"
        );
        assert!(
            content.starts_with(text) && content[text.len()..].starts_with(' '),
            "it falls back to a word cut: {summary:?}"
        );
    }
}

/// The same, after an earlier sentence: the line ends at that sentence rather than carrying the
/// number of a list it never reaches.
#[test]
fn a_one_word_sentence_after_another_is_not_where_the_line_ends() {
    let text = MemoryText::new(
        "Six parts follow. 1. Recall before writing: before each change that adds or alters \
         behaviour, recall one sentence describing it and read what comes back",
    )
    .expect("valid text");

    assert_eq!(text.summary_as_data(), "Six parts follow.\u{2026}");
}

/// A decimal point and a dotted identifier are not sentence ends: a terminator only ends a sentence
/// when whitespace follows it.
#[test]
fn a_dot_inside_a_number_or_an_identifier_is_not_a_sentence_end() {
    let text = MemoryText::new(
        "0.5 is the recall floor and store.rs is where it lives, which together are long enough \
         that the line has to be cut somewhere after them.",
    )
    .expect("valid text");

    let summary = text.summary_as_data();
    assert!(
        summary.starts_with("0.5 is the recall floor and store.rs is where it lives"),
        "cut at a decimal or an identifier: {summary:?}"
    );
}

/// With no sentence to end on, the line ends at a word rather than in the middle of one.
#[test]
fn without_a_sentence_end_the_line_ends_at_a_word() {
    let content = "The per-record envelope is four hundred and eleven characters wide and does not \
                   shrink with the prose at all, which is what the arithmetic turns on";
    let summary = MemoryText::new(content)
        .expect("valid text")
        .summary_as_data();
    let text = summary
        .strip_suffix('\u{2026}')
        .expect("a cut line is marked");

    assert!(
        content.starts_with(text),
        "the summary is a prefix of the content: {summary:?}"
    );
    assert!(
        !text.ends_with(' '),
        "a trailing space is not a word: {summary:?}"
    );
    let next = content[text.len()..].chars().next().expect("more content");
    assert!(
        next.is_whitespace(),
        "cut mid-word, at {next:?}: {summary:?}"
    );
}

/// One word longer than the cap has no boundary to cut at, so the cap itself is the cut. The
/// alternative is an empty line, which names nothing.
#[test]
fn a_single_word_longer_than_the_cap_is_cut_at_the_cap() {
    let text = MemoryText::new("x".repeat(SUMMARY_CAP + 40)).expect("valid text");
    assert_eq!(
        text.summary_as_data(),
        "x".repeat(SUMMARY_CAP - 1) + "\u{2026}"
    );
}

/// **The marker answers one question — is there more memory beyond this line — so it has to answer
/// it every time.** Marking only the ragged cuts would leave an unmarked line meaning either
/// *complete* or *cut at a sentence*, which is the ambiguity the marker exists to remove.
#[test]
fn a_line_carries_a_marker_exactly_when_something_was_dropped() {
    let whole = "Push is the marked case.";
    let cut_at_a_sentence = "Push is the marked case. Most accumulation stays passive. The rest is \
                             reached by ordinary recall, which costs nothing until asked.";
    let cut_at_a_word = "The per-record envelope is four hundred and eleven characters wide and does \
                         not shrink with the prose at all, which is what the arithmetic turns on";

    for (content, marked) in [
        (whole, false),
        (cut_at_a_sentence, true),
        (cut_at_a_word, true),
    ] {
        let summary = MemoryText::new(content)
            .expect("valid text")
            .summary_as_data();
        assert_eq!(
            summary.ends_with('\u{2026}'),
            marked,
            "{summary:?} should {}be marked",
            if marked { "" } else { "not " }
        );
    }
}

/// The marker is part of the line, not an addition to it. A cap that the marker can push past is a
/// cap that stops binding on exactly the lines that are already at it.
#[test]
fn the_marker_is_inside_the_cap() {
    for content in [
        "x".repeat(SUMMARY_CAP * 2),
        format!("{}. and then some more text after it", "word ".repeat(30)),
        "The fix is to push a derived index line and pull the watch on demand, one line per watch, \
         taken from the leading sentence of the content rather than from a new authored field."
            .to_owned(),
    ] {
        let summary = MemoryText::new(&content)
            .expect("valid text")
            .summary_as_data();
        assert!(
            summary.ends_with('\u{2026}') && summary.chars().count() <= SUMMARY_CAP,
            "{} characters, marked {}: {summary:?}",
            summary.chars().count(),
            summary.ends_with('\u{2026}')
        );
    }
}

/// A memory's interior is preserved exactly, newlines included -- but a *line* has none, and an
/// index of lines that can each be two lines is not an index.
#[test]
fn the_summary_is_one_line() {
    let text = MemoryText::new("Two claims.\n\nOne memory,  double  spaced.").expect("valid text");
    assert_eq!(
        text.summary_as_data(),
        "Two claims. One memory, double spaced."
    );
}

/// Report how the summary line falls over a real memory file. **Not a parked test** — it asserts
/// nothing, and is `#[ignore]`d because it exists to produce a distribution rather than to check
/// one.
///
/// The cut was measured over the corpus before the cap was fixed, and two of the figures that came
/// out of that measurement are quoted in [`documents/design/recall-mechanisms.md`] and in
/// `EARLY-VALIDATION.md` — where one of them is a *prediction*, that the share cutting at a
/// sentence rises as authors write openings to fit. Re-measuring it needs the same instrument, and
/// this is that instrument running the shipped code rather than a second copy of the rule, which
/// is what the first measurement was.
///
/// ```text
/// cargo test --test memory_model report_summary_cuts -- --ignored --nocapture
/// SUMMARY_CORPUS=../other/scatterhoard_mem/coding-lessons.json cargo test ... --ignored --nocapture
/// ```
#[test]
#[ignore]
fn report_summary_cuts() {
    let path = std::env::var("SUMMARY_CORPUS")
        .unwrap_or_else(|_| "scatterhoard_mem/scatterhoard.json".to_owned());
    let file = std::fs::File::open(&path).unwrap_or_else(|error| panic!("{path}: {error}"));

    let (mut whole, mut sentence, mut word, mut longest) = (0, 0, 0, 0);
    scatterhoard::memory_file::stream(&Expectation::AnyShared, file, |record| {
        if let Record::Memory(memory) = record {
            let summary = memory.content.summary_as_data();
            let length = summary.chars().count();
            longest = longest.max(length);
            // Whole when nothing was cut: the same words, in the same order, whatever the
            // memory's own spacing was. Comparing lengths would need the collapse rule spelled out
            // a second time here, which is the copy this test exists to avoid.
            if summary
                .split_whitespace()
                .eq(memory.content.as_data().split_whitespace())
            {
                whole += 1;
            } else if summary
                .trim_end_matches('\u{2026}')
                .ends_with(['.', '!', '?'])
            {
                sentence += 1;
            } else {
                word += 1;
            }
        }
        Ok(())
    })
    .unwrap_or_else(|error| panic!("{path}: {error}"));

    let total = whole + sentence + word;
    println!(
        "SUMMARY CUTS over {path}: n={total} whole={whole} sentence={sentence} word={word} \
         longest={longest} cap={SUMMARY_CAP}"
    );
}

// ---------------------------------------------------------------------------
// The `eval-` prefix names an evaluation load
// ---------------------------------------------------------------------------
//
// A set loaded for review is named `eval-<set>-<label>`, and that name is what marks its memories as
// proposals rather than the project's record. `SetName::new` does **not** refuse the prefix: it is
// also how an already-loaded set is named -- `drop`'s argument, a scope in a tool call, the store
// reading its own scope back -- so refusing here refused an evaluation load the moment anything
// addressed it. The ban sits at the three boundaries where a *real* set could come to carry the
// prefix: `create` (tests/sets.rs), a file's header (tests/memory_file.rs) and a link written into a
// store (tests/store.rs). `documents/design/load-modes.md`.

#[test]
fn a_name_beginning_with_the_prefix_reads_as_an_evaluation_load() {
    for name in ["eval-scatterhoard-1234", "Eval-set", "EVAL-set", "eVaL-set"] {
        let named = SetName::new(name).expect("a legal set name, just not one create may make");
        assert!(
            named.is_evaluation(),
            "{name} should read as a review copy, case notwithstanding"
        );
    }
}

/// The prefix is the word with its hyphen, not the word: a set really called `evaluation` or `evals`
/// is not claiming to be a review copy.
#[test]
fn only_the_evaluation_prefix_reads_that_way() {
    for name in [
        "eval",
        "evaluation",
        "evals",
        "evaluation-harness",
        "my-eval-set",
    ] {
        assert!(
            !SetName::new(name).expect("valid").is_evaluation(),
            "{name} does not claim to be an evaluation copy"
        );
    }
}

/// **The one constructor that builds such a name**, used by the evaluation load and nothing else. It
/// prefixes the source set, appends the user's label, and makes the result a legal set name -- a
/// label like `feature/x` cannot be one as it stands.
#[test]
fn an_evaluation_name_is_built_from_its_source_and_label() {
    let source = SetName::new("scatterhoard").expect("valid");

    let named = SetName::evaluation(&source, "issue-3").expect("builds");
    assert_eq!(named.as_str(), "eval-scatterhoard-issue-3");
    assert!(named.is_evaluation());

    let awkward = SetName::evaluation(&source, "feature/x (draft)").expect("builds");
    assert_eq!(
        awkward.as_str(),
        "eval-scatterhoard-feature-x-draft",
        "a label the user types is made into a name rather than refused"
    );
}

/// A label that carries nothing usable is refused rather than silently dropped: the label is what
/// says *which copy this is*, so a name without one would defeat the point of having several open.
#[test]
fn an_evaluation_label_that_says_nothing_is_refused() {
    let source = SetName::new("scatterhoard").expect("valid");
    for label in ["", "   ", "///", "--"] {
        let refused = SetName::evaluation(&source, label)
            .expect_err(&format!("{label:?} leaves no label at all"));
        assert!(
            refused.to_string().contains("label"),
            "the refusal must name the label the caller gave, not the set name it would have \
             produced: {refused}"
        );
    }
}
