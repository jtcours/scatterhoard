// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Hard-coded operating conventions, and the mechanism that serves them.
//!
//! Conventions act like memories but are hard-coded, and are loaded into any session that uses
//! Scatterhoard. See `documents/design/conventions.md`; the catalog itself lives in
//! [`catalog`](self::catalog).

mod catalog;

use std::sync::OnceLock;

pub use catalog::CONVENTIONS;

/// A single hard-coded operating convention.
///
/// The two text fields are held to different standards, and the distinction is load-bearing:
///
/// - `text` is the convention itself.
/// - `reinforcement` is served *with* the convention every time it is used, and must reinforce it
///   at the moment of use.
///
/// Background that only matters when *editing* a convention is not part of this struct at all — it
/// belongs in source comments beside the entry in the catalog, and is never served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Convention {
    /// Stable identifier, lowercase kebab-case. Referenced from memories and other conventions, so
    /// changing one is a breaking change.
    pub id: &'static str,
    /// The convention itself.
    pub text: &'static str,
    /// Text served alongside `text`, reinforcing the convention in the moment and clarifying when
    /// it applies.
    pub reinforcement: &'static str,
}

impl Convention {
    /// What serving this convention spends against a page's `max_chars`, in characters.
    ///
    /// **Characters, not `str::len()`**, which is bytes: the budget is named and argued in
    /// characters, and an em dash is three bytes but one character and about one token.
    /// `catalog-budget-counts-bytes-not-characters`. Defined once here because five sites each
    /// summed the fields themselves, which is how all five came to count the wrong unit together.
    #[must_use]
    pub fn served_chars(&self) -> usize {
        self.text.chars().count() + self.reinforcement.chars().count()
    }

    /// What this convention counts against [`MAX_CATALOG_CHARS`]: what it serves, and its id.
    #[must_use]
    pub fn catalog_chars(&self) -> usize {
        self.id.chars().count() + self.served_chars()
    }
}

/// Budget for the whole catalog, in characters, enforced by test.
///
/// Conventions are loaded into every session, so the catalog is a standing charge against the
/// context window. Characters are a proxy for tokens; the point is that growth past the budget has
/// to be a decision someone makes rather than a drift nobody notices.
///
/// The proxy is known to be uneven, and in the direction that matters. Measured 2026-08-01: prose
/// runs about 5.5 characters per token, bare identifiers about 4.1 — so a character budget
/// under-prices exactly the tables of `SCREAMING_SNAKE_CASE` labels the catalog is likeliest to
/// grow by, and gets more permissive in token terms the further the catalog drifts from prose.
/// (Measured on a WordPiece tokenizer rather than the one that will actually read this, so treat
/// the ratio as real and the absolute numbers as indicative.)
///
/// Tightening this into a real token count would mean carrying a tokenizer to build the crate,
/// which is not worth it while the catalog is small. Revisit if the vocabulary tables grow.
///
/// Raised from 8,000 to 16,000 on 2026-08-12, by the human partner, on the ratio the budget is
/// actually spending: 8,000 characters is around 1,950 tokens and 16,000 is around 3,900, against
/// context windows running from 128K on the smaller open-weight local models up to about 1M. Both
/// figures are small against either end of that range, so the earlier number was not buying much
/// and was about to force conventions to be shrunk to fit each other. The point of the budget is
/// unchanged — growth still has to be a decision someone makes — and 16,000 is a number to work
/// towards rather than a target to fill.
///
/// Raised again to 18,000 on 2026-08-13, by the human partner, decided on the same ratio rather
/// than on what the next convention needs. The catalog stood at 14,610 characters — about 3,560
/// tokens at the identifier rate, 2,660 at the prose rate — with 1,390 left and
/// `investigation-pipeline` still to draft, which was expected to run above the 1,461-character
/// average because it carries three linked concepts. The bar set for the raise was staying **well
/// under 4% of the smallest context window in range**: 18,000 characters is about 4,390 tokens,
/// which is 3.4% of 128K. The alternative was shrinking conventions against each other, which is
/// precisely what the previous raise was made to avoid.
///
/// The "work towards it, not fill it" instruction still attaches, and applies harder at each raise:
/// this is the second, and a budget raised whenever it binds is not a budget. What has kept it
/// honest so far is that both raises were argued from what the number *spends* rather than from
/// what would not fit.
///
/// **It is not a page size.** It once did double duty as the default `max_chars`, which is what
/// kept the catalog in one page; since 2026-10-01 a page with no budget holds the whole catalog
/// whatever its size, so this bounds what the catalog may grow to and nothing about how it is
/// served. `max-chars-is-not-a-bound`.
pub const MAX_CATALOG_CHARS: usize = 18_000;

/// Every convention, in catalog order.
pub fn all() -> &'static [Convention] {
    CONVENTIONS
}

/// Look up a convention by its identifier.
pub fn get(id: &str) -> Option<&'static Convention> {
    CONVENTIONS.iter().find(|c| c.id == id)
}

/// The catalog as one compact JSON document, `{"conventions":[…]}`, which is what pages are cut
/// from: each page's first line is a slice of it, and joining the slices in order gives it back.
///
/// Compact JSON escapes every newline inside a string, so the document has none of its own and a
/// page's lines can be told apart by splitting on them.
#[must_use]
pub fn document() -> &'static str {
    static DOCUMENT: OnceLock<String> = OnceLock::new();
    DOCUMENT.get_or_init(|| {
        let conventions: Vec<_> = all()
            .iter()
            .map(|convention| {
                serde_json::json!({
                    "id": convention.id,
                    "text": convention.text,
                    "reinforcement": convention.reinforcement,
                })
            })
            .collect();
        serde_json::json!({ "conventions": conventions }).to_string()
    })
}

/// [`document`] with the person's sharing preferences as its last field, which is what a session
/// with a preferences file is paged through.
///
/// **Spliced onto the catalog's own text rather than rebuilt**, so the catalog arrives first and
/// byte for byte as [`document`] has it, whatever order a JSON map keeps its keys in. After the
/// last convention because the catalog must arrive whole before anything a person can make any
/// length, and the framing refers to conventions the session has then read.
/// `documents/design/export-review.md`, *the preferences start from a template the person edits*.
#[must_use]
pub fn document_with(sharing_preferences: &serde_json::Value) -> String {
    let catalog = document();
    let open = catalog.strip_suffix('}').unwrap_or(catalog);
    format!("{open},\"sharing_preferences\":{sharing_preferences}}}")
}

/// One page of the catalog: the text to serve, and where the next page starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// A slice of [`document`], then the aside if there is one, then a last line saying either how
    /// to get the rest or that every convention is loaded — each on a line of its own.
    pub text: String,
    /// The character offset into [`document`] the next page starts at, or `None` on the last page.
    pub next_cursor: Option<usize>,
}

/// Why a page could not be served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageRefusal {
    /// The budget cannot hold the page's own lines and one character of the catalog besides.
    BudgetTooSmall {
        /// The least budget that would serve a page from this cursor.
        needed: usize,
    },
    /// The cursor is at or past the end of the catalog, which no page ever offers.
    PastTheEnd {
        /// How many characters the catalog document holds.
        length: usize,
    },
}

impl std::fmt::Display for PageRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BudgetTooSmall { needed } => write!(
                formatter,
                "max_chars is too small to serve any of the catalog: a page needs at least \
                 {needed} characters for its own lines and one of the conventions"
            ),
            Self::PastTheEnd { length } => write!(
                formatter,
                "the cursor is past the end of the catalog, which is {length} characters long; \
                 call hoard_conventions with no cursor to start again"
            ),
        }
    }
}

/// The last line of a page with more to come.
///
/// **Its wording is evidence, not style.** Tested 2026-10-01 against `gemma4:12b` with thinking
/// off: a page that only *showed* more was coming was taken as the end every time, and this
/// sentence, placed after the catalog, was followed every time, both arguments included. The same
/// sentence placed before the catalog was ignored, and "Not complete." in place of its opening
/// was ignored under this tool's name though it worked under another. Re-test before rewording:
/// `python3 scripts/local_model.py page --page-file <draft> --expect continue`.
fn more_follows(next: usize, max_chars: usize) -> String {
    format!(
        r#"The conventions are not loaded yet. Call hoard_conventions again with {{"cursor": {next}, "max_chars": {max_chars}}} for the rest."#
    )
}

/// The last line of the last page: said outright, with the count, so ending never rests on an
/// absent cursor and a reader holding only this page knows what the whole amounted to.
fn all_loaded() -> String {
    format!("All {} conventions are loaded.", all().len())
}

/// Serve the catalog from character `cursor` of [`document`], in at most `max_chars` characters.
///
/// **`max_chars` bounds the whole page**, its own lines included, counted in characters. Pages are
/// cut wherever the budget falls, a convention included, so no budget forces an overrun — the
/// whole-convention pager this replaced had to return one oversized convention rather than none,
/// and so overran by up to eight times. `max-chars-is-not-a-bound`. With no budget the whole
/// catalog arrives in one page.
///
/// `aside` is anything the server says beside the catalog. It sits between the slice and the last
/// line, so the last line stays last, and it is spent from the same budget.
///
/// # Errors
///
/// [`PageRefusal::PastTheEnd`] for a cursor at or past the end, and
/// [`PageRefusal::BudgetTooSmall`] for a budget that cannot hold the page's own lines and one
/// character of the catalog.
pub fn page(
    cursor: usize,
    max_chars: Option<usize>,
    aside: Option<&str>,
) -> Result<Page, PageRefusal> {
    page_of(document(), cursor, max_chars, aside)
}

/// [`page`], cut from a given document rather than the bare catalog: the one
/// [`document_with`] makes for a session that has sharing preferences.
///
/// # Errors
///
/// As for [`page`].
pub fn page_of(
    document: &str,
    cursor: usize,
    max_chars: Option<usize>,
    aside: Option<&str>,
) -> Result<Page, PageRefusal> {
    let length = document.chars().count();
    if cursor >= length {
        return Err(PageRefusal::PastTheEnd { length });
    }
    let rest = length - cursor;
    // One newline before the aside, if there is one, and one before the last line.
    let aside_chars = aside.map_or(0, |aside| aside.chars().count() + 1);
    let lines = |last: &str| aside_chars + 1 + last.chars().count();

    let take = match max_chars {
        None => rest,
        Some(budget) if rest + lines(&all_loaded()) <= budget => rest,
        Some(budget) => {
            // The last line names the next cursor, so its length depends on where this page ends;
            // the end of the catalog is the widest cursor it can name, so sizing for that never
            // overruns and leaves at most a digit or two unspent.
            let overhead = lines(&more_follows(length, budget));
            let take = budget.saturating_sub(overhead);
            if take == 0 {
                // The line quotes the budget too, so a larger budget lengthens it: the least that
                // works is found by trying, which settles within a digit or two.
                let needed = (budget + 1..)
                    .find(|&candidate| candidate > lines(&more_follows(length, candidate)))
                    .unwrap_or(usize::MAX);
                return Err(PageRefusal::BudgetTooSmall { needed });
            }
            take.min(rest - 1)
        }
    };

    let slice: String = document.chars().skip(cursor).take(take).collect();
    let next_cursor = (take < rest).then_some(cursor + take);
    let last = match (next_cursor, max_chars) {
        (Some(next), Some(budget)) => more_follows(next, budget),
        _ => all_loaded(),
    };
    let text = match aside {
        Some(aside) => format!("{slice}\n{aside}\n{last}"),
        None => format!("{slice}\n{last}"),
    };
    Ok(Page { text, next_cursor })
}
