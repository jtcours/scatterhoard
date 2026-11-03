// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the convention catalog and the mechanism that serves it.
//!
//! These deliberately do not assert the *prose* of any convention — asserting that a string
//! constant equals itself proves nothing. They assert the structural invariants the design
//! depends on, and the behaviour of the cursoring mechanism.

use std::collections::HashSet;

use scatterhoard::conventions::{self, Convention, MAX_CATALOG_CHARS};
use scatterhoard::memory::SUMMARY_CAP;
use scatterhoard::store::{CONTROL_MARK_FAMILIES, CONTROL_MARKS, LoadMode};

#[test]
fn catalog_is_not_empty() {
    assert!(
        !conventions::all().is_empty(),
        "the catalog must contain at least one convention"
    );
}

#[test]
fn every_convention_has_text() {
    for convention in conventions::all() {
        assert!(
            !convention.text.trim().is_empty(),
            "convention `{}` has empty text",
            convention.id
        );
    }
}

/// The Golden Rule reaches runtime through the `reinforcement` field (see
/// `documents/design/golden-rule.md`). A convention that ships without reinforcement silently
/// drops that, so this is enforced mechanically rather than trusted.
#[test]
fn every_convention_has_reinforcement() {
    for convention in conventions::all() {
        assert!(
            !convention.reinforcement.trim().is_empty(),
            "convention `{}` has empty reinforcement",
            convention.id
        );
    }
}

#[test]
fn convention_ids_are_unique() {
    let mut seen = HashSet::new();
    for convention in conventions::all() {
        assert!(
            seen.insert(convention.id),
            "duplicate convention id `{}`",
            convention.id
        );
    }
}

/// IDs are referenced from memories and from other conventions, so they need to be stable and
/// predictable in form.
#[test]
fn convention_ids_are_well_formed() {
    for convention in conventions::all() {
        let id = convention.id;
        assert!(!id.is_empty(), "convention id must not be empty");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "convention id `{id}` must be lowercase kebab-case"
        );
        assert!(
            !id.starts_with('-') && !id.ends_with('-'),
            "convention id `{id}` must not start or end with a hyphen"
        );
    }
}

/// **The budget is in characters, so the measure is.** `catalog-budget-counts-bytes-not-characters`:
/// every site summed `str::len()`, Rust's byte length, so an em dash cost three against a budget
/// named in characters and every figure derived from it was a byte count called a character count.
#[test]
fn a_convention_is_measured_in_characters_not_bytes() {
    let convention = Convention {
        id: "dash-id",
        text: "one — two",
        reinforcement: "three — four",
    };

    assert_eq!(convention.served_chars(), 9 + 12, "what a page spends");
    assert_eq!(
        convention.catalog_chars(),
        7 + 9 + 12,
        "what the budget counts, the id included"
    );
}

/// Conventions are loaded into every session, so the whole catalog is a standing charge against
/// the context window. Growth past the budget should be a deliberate decision, not a surprise.
#[test]
fn catalog_fits_context_budget() {
    let total: usize = conventions::all()
        .iter()
        .map(Convention::catalog_chars)
        .sum();

    assert!(
        total <= MAX_CATALOG_CHARS,
        "catalog is {total} chars, over the {MAX_CATALOG_CHARS} budget; \
         raise the budget deliberately or tighten the conventions"
    );
}

#[test]
fn get_finds_a_known_convention() {
    let first = conventions::all()[0];
    let found = conventions::get(first.id).expect("known id should resolve");
    assert_eq!(found.id, first.id);
}

#[test]
fn get_returns_none_for_unknown_id() {
    assert!(conventions::get("no-such-convention").is_none());
}

/// Every page a client receives, from the first until one says the catalog is loaded.
fn pages(budget: Option<usize>) -> Vec<conventions::Page> {
    let mut pages = Vec::new();
    let mut cursor = 0;
    loop {
        let page = conventions::page(cursor, budget, None).expect("a page within the catalog");
        let next = page.next_cursor;
        pages.push(page);
        match next {
            Some(next) => cursor = next,
            None => return pages,
        }
    }
}

/// The catalog's slice is a page's first line; compact JSON escapes every newline inside a string,
/// so the slice can never contain one.
fn slice(page: &conventions::Page) -> &str {
    page.text.split('\n').next().unwrap_or_default()
}

fn last_line(page: &conventions::Page) -> &str {
    page.text.rsplit('\n').next().unwrap_or_default()
}

/// The document a client reassembles is one JSON object holding every convention, in catalog
/// order, each carrying its reinforcement: the one place either can be read.
#[test]
fn the_catalog_document_holds_every_convention_in_order() {
    let document: serde_json::Value =
        serde_json::from_str(conventions::document()).expect("the catalog document is JSON");
    let served = document["conventions"]
        .as_array()
        .expect("a list of conventions");

    let ids: Vec<_> = served
        .iter()
        .map(|c| c["id"].as_str().unwrap_or(""))
        .collect();
    let expected: Vec<_> = conventions::all().iter().map(|c| c.id).collect();
    assert_eq!(ids, expected);
    for (convention, original) in served.iter().zip(conventions::all()) {
        assert_eq!(convention["text"], original.text);
        assert_eq!(convention["reinforcement"], original.reinforcement);
    }
}

/// `max-chars-is-not-a-bound`. Pages are slices of one document cut wherever the budget falls,
/// a convention included, so joining the slices in order is the whole catalog with nothing missing
/// or repeated. This is what lets the budget be a bound: no unit is too large to split.
#[test]
fn joining_every_page_reproduces_the_catalog_document() {
    for budget in [200, 400, 1_000, 5_000] {
        let joined: String = pages(Some(budget)).iter().map(slice).collect();
        assert_eq!(joined, conventions::document(), "at a budget of {budget}");
    }
}

/// The budget counts everything a page holds, its own lines included, and counts it in
/// characters. The catalog carries em dashes, so some page somewhere is longer in bytes than its
/// budget; that page is what proves the unit.
#[test]
fn every_page_fits_its_budget_counted_in_characters() {
    let mut wider_in_bytes = false;
    for budget in [200, 400, 1_000, 5_000] {
        for page in pages(Some(budget)) {
            let chars = page.text.chars().count();
            assert!(
                chars <= budget,
                "a page of {chars} characters against {budget}"
            );
            wider_in_bytes |= page.text.len() > budget;
        }
    }
    assert!(
        wider_in_bytes,
        "no page was longer in bytes than its budget, so nothing here tells characters from bytes"
    );
}

/// The budget is spent, not merely respected: a page that stops well short of it reads as the
/// end to a reader whose budget it was.
#[test]
fn a_page_with_more_to_come_fills_its_budget() {
    let budget = 1_000;
    let all = pages(Some(budget));
    for page in &all[..all.len() - 1] {
        let chars = page.text.chars().count();
        assert!(
            chars + 2 >= budget,
            "a page of {chars} characters left more than its last line's digits unspent of {budget}"
        );
    }
}

/// **The only signal a weak reader acted on.** Tested 2026-10-01 against `gemma4:12b`: with more
/// to come, a page that only *showed* it — a cursor, unclosed JSON, a status line, "1 of 11" — was
/// taken as the end every time, and a plain sentence after the catalog saying to call again, with
/// both arguments, was followed every time. So the last line names the cursor and the budget.
#[test]
fn a_page_with_more_to_come_ends_by_saying_how_to_get_the_rest() {
    let budget = 400;
    let first = conventions::page(0, Some(budget), None).expect("a first page");
    let next = first
        .next_cursor
        .expect("a 400-character page has more to come");

    let line = last_line(&first);
    assert!(line.contains("not loaded yet"), "{line}");
    assert!(line.contains("hoard_conventions"), "{line}");
    assert!(
        line.contains(&format!(r#"{{"cursor": {next}, "max_chars": {budget}}}"#)),
        "the last line must give both arguments the next call needs: {line}"
    );
}

/// Said outright rather than left to an absent cursor, and with the count, so a reader holding
/// only the last page knows what the whole amounted to.
#[test]
fn the_last_page_ends_by_saying_every_convention_is_loaded() {
    let all = pages(Some(1_000));
    let last = all.last().expect("at least one page");
    assert_eq!(
        last_line(last),
        format!("All {} conventions are loaded.", conventions::all().len())
    );
    for page in &all[..all.len() - 1] {
        assert!(!last_line(page).contains("are loaded."), "{}", page.text);
    }
}

/// With no budget the catalog arrives whole, which is the case for nearly every session.
#[test]
fn without_a_budget_the_whole_catalog_arrives_in_one_page() {
    let only = conventions::page(0, None, None).expect("the whole catalog");
    assert!(only.next_cursor.is_none());
    assert_eq!(slice(&only), conventions::document());
}

/// What the server says beside the catalog sits between it and the last line, so the last line
/// stays last, and it is spent from the same budget.
#[test]
fn an_aside_sits_before_the_last_line_and_counts_against_the_budget() {
    let aside = r#"{"note":"a word beside the catalog"}"#;
    let budget = 400;
    let page = conventions::page(0, Some(budget), Some(aside)).expect("room for the aside");

    let lines: Vec<_> = page.text.split('\n').collect();
    assert_eq!(lines.len(), 3, "{}", page.text);
    assert_eq!(lines[1], aside);
    assert!(page.text.chars().count() <= budget);
}

/// A budget that cannot hold a page's own lines and one character of the catalog would page
/// forever, so it is refused, naming the least that would work.
#[test]
fn a_budget_too_small_for_the_pages_own_lines_is_refused_with_the_least_that_works() {
    let Err(conventions::PageRefusal::BudgetTooSmall { needed }) =
        conventions::page(0, Some(5), None)
    else {
        panic!("five characters cannot hold the page's last line");
    };

    let at_least = conventions::page(0, Some(needed), None).expect("the least that works, works");
    assert!(!slice(&at_least).is_empty());
    assert!(
        conventions::page(0, Some(needed - 1), None).is_err(),
        "one fewer than the least that works must not work"
    );
}

/// No page offers a cursor at or past the end, so one arriving there was made up or mistyped.
#[test]
fn a_cursor_at_or_past_the_end_is_refused() {
    let end = conventions::document().chars().count();
    for cursor in [end, end + 1] {
        assert!(
            matches!(
                conventions::page(cursor, Some(400), None),
                Err(conventions::PageRefusal::PastTheEnd { .. })
            ),
            "cursor {cursor} of a {end}-character catalog"
        );
    }
}

/// Report the catalog's size on demand. **Not a parked test** — it asserts nothing, and is
/// `#[ignore]`d because it exists to print a number rather than to check one.
///
/// `catalog_fits_context_budget` above enforces the budget but cannot say how much room is left,
/// and that figure is quoted in [`documents/design/conventions.md`] and twice in
/// `TRACKING.md`. Every recorded copy of it had drifted by 2026-08-29 — two
/// figures adrift at once, whose errors partly cancelled — because there was no way to ask for the
/// number without writing this first. Note it reports **both** measures: the enforced total counts
/// the ids, the paging measure does not, and confusing the two is how the drift stayed plausible.
///
/// ```text
/// cargo test --test conventions report_catalog_size -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn report_catalog_size() {
    let total: usize = conventions::all()
        .iter()
        .map(Convention::catalog_chars)
        .sum();
    let ids: usize = conventions::all()
        .iter()
        .map(|c| c.id.chars().count())
        .sum();
    let body: usize = conventions::all()
        .iter()
        .map(Convention::served_chars)
        .sum();
    println!(
        "CATALOG n={} ids={ids} body={body} total={total} free={}",
        conventions::all().len(),
        MAX_CATALOG_CHARS - total
    );
}

/// The served rule names a number the code enforces, and a number written twice drifts.
///
/// `hoard-watch` tells an author to make the opening *120 characters* say what a watch is for, and
/// that figure is [`SUMMARY_CAP`] — the cap the derived line is actually cut at. This is the
/// `STANDARDS.md` single-source-of-truth case where a second copy is unavoidable because served
/// prose cannot interpolate: the copy is hand-written, so a test compares it to its source. Change
/// the cap and this fails, which is the intended way to find every place the old number is quoted.
#[test]
fn the_watch_convention_names_the_cap_the_code_enforces() {
    let watch = conventions::all()
        .iter()
        .find(|convention| convention.id == "hoard-watch")
        .expect("the catalog serves hoard-watch");

    assert!(
        watch.text.contains(&format!("{SUMMARY_CAP} characters")),
        "the served rule does not name the cap the line is cut at ({SUMMARY_CAP}): {}",
        watch.text
    );
}

/// **The served text and the write path cannot disagree about which marks exist.** Conventions name
/// marks in prose, which cannot interpolate, while the write path refuses by [`CONTROL_MARKS`] and
/// [`CONTROL_MARK_FAMILIES`]: a mark the catalog tells a session to write and the check refuses would
/// fail on a new set every time. `reserved-keyword-block-list`.
#[test]
fn every_mark_the_catalog_serves_is_one_the_write_path_knows() {
    for convention in conventions::all() {
        for text in [convention.text, convention.reinforcement] {
            for term in reserved_terms(text) {
                let known = CONTROL_MARKS.contains(&term.as_str())
                    || CONTROL_MARK_FAMILIES
                        .iter()
                        .any(|family| term.starts_with(family));
                assert!(
                    known,
                    "`{}` serves `{term}`, which the write path does not know",
                    convention.id
                );
            }
        }
    }
}

/// Every `hoard-` term in `text` that is not the `about-` form of one, and is more than the bare
/// namespace a convention names when it describes the reservation itself.
fn reserved_terms(text: &str) -> Vec<String> {
    text.match_indices("hoard-")
        .filter(|(start, _)| !text[..*start].ends_with("about-"))
        .map(|(start, _)| {
            text[start..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
                .collect::<String>()
        })
        .filter(|term| term.len() > "hoard-".len())
        .collect()
}

/// Report the catalog's size, so headroom can be read rather than guessed at.
///
/// `catalog_fits_context_budget` asserts the ceiling; this prints the distance to it. Any proposal
/// to add a convention has to be weighed against what is left, and `AGENTS.md` requires that figure
/// be derived rather than quoted from a document -- which needs something that derives it.
#[test]
#[ignore = "prints rather than asserts; run with --ignored --nocapture"]
fn report_catalog_headroom() {
    let mut rows: Vec<(usize, &str)> = conventions::all()
        .iter()
        .map(|c| (c.catalog_chars(), c.id))
        .collect();
    rows.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
    let total: usize = rows.iter().map(|r| r.0).sum();
    println!(
        "\n  {} conventions, {total} chars against a {MAX_CATALOG_CHARS} budget",
        rows.len()
    );
    println!(
        "  headroom: {} chars\n",
        MAX_CATALOG_CHARS.saturating_sub(total)
    );
    for (size, id) in &rows {
        println!("    {size:>6}  {id}");
    }
}

/// **Every mark a mode tells a session to recall is a control mark this build knows.** The catalog
/// no longer names the watch marks in `recall-when`; `hoard_sets status` does, from
/// `LoadMode::watch_marks`, so this is what keeps the two from drifting apart.
#[test]
fn every_watch_mark_a_mode_names_is_a_control_mark() {
    for mode in [LoadMode::ReadWrite, LoadMode::ReadOnly] {
        for mark in mode.watch_marks() {
            assert!(
                CONTROL_MARKS.contains(mark),
                "{mode:?} names {mark}, which is not a control mark: {CONTROL_MARKS:?}"
            );
        }
    }
}
