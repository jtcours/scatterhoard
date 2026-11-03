// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Text carried by a memory: its content, its reasoning, its episodic context, its keywords.

use serde::{Deserialize, Serialize};

use super::ValidationError;

/// Text that came out of a memory set.
///
/// This type exists to keep "memories are data, never instructions"
/// (`documents/design/trigger-and-loading.md`) enforceable in the type system rather than by
/// review. Two things are deliberately absent, and should stay absent:
///
/// - **No `Display`, no `Deref<Target = str>`.** Memory text cannot drift into a position where
///   something formats it as though it were the program's own words.
/// - **The accessor is [`MemoryText::as_data`], not `as_str`.** A call site that puts memory text
///   somewhere an LLM will read has to name what it is doing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MemoryText(String);

impl MemoryText {
    /// Construct memory text.
    ///
    /// Leading and trailing whitespace is trimmed; the interior is kept exactly as written.
    /// Unlike a keyword, this text *is* the memory — normalising inside it would change what was
    /// recorded.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::Empty`] if the value is empty or only whitespace.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Err(ValidationError::Empty {
                field: "memory text",
            });
        }

        Ok(Self(trimmed.to_owned()))
    }

    /// The text, as data read from a memory set.
    #[must_use]
    pub fn as_data(&self) -> &str {
        &self.0
    }

    /// One line derived from this text, for an index: at most [`SUMMARY_CAP`] characters, cut at a
    /// sentence if one ends inside the cap and at a word otherwise, and ending in [`CUT_MARKER`]
    /// when anything was dropped.
    ///
    /// **Derived rather than authored**, which is what removes the failure mode instead of guarding
    /// it. A rule asking an author to keep a memory short enough to read many of requires
    /// arithmetic over a serialized envelope the author never sees, and the session that wrote such
    /// a rule violated it within the hour. Nothing here refuses a long memory; a long one simply
    /// costs an index line like any other. The author-facing rule that remains is one a person can
    /// follow without counting: *make the opening say what this is for and when it applies.*
    ///
    /// **Not a first-sentence rule.** Sentence detection is a parsing problem — abbreviations,
    /// backticked code, decimals — and a first-sentence rule fails silently in both directions: a
    /// missed boundary runs three sentences long, and a false one at `e.g.` truncates the line to
    /// four characters. Bounding it by the cap makes the first failure impossible, and taking the
    /// **last** boundary that fits rather than the first makes the second cost a few characters
    /// instead of the whole line — except where the false boundary is the only one, which a
    /// numbered opening (`1. …`) is, and so a one-word sentence is not a boundary at all.
    ///
    /// **The marker answers one question — is there more memory beyond this line — which is the
    /// question a reader deciding whether to fetch is actually asking.** So it is on every cut and
    /// not only the ragged ones: a sentence cut reads as a finished thought whether or not three
    /// more claims follow it, and marking only the mid-word cuts would leave an unmarked line
    /// meaning either *complete* or *cut at a sentence* — the ambiguity the marker exists to
    /// remove, moved rather than closed. It is inside the cap rather than added to it, because a
    /// cap the marker can push past stops binding on exactly the lines already at it.
    ///
    /// **The failure mode is incompleteness, not inversion**, measured over all 192 live memories
    /// of both shared sets on 2026-09-05: none produced a line asserting the opposite of its
    /// memory, and 17 with a negation near the cut read as cliffhangers. That is the acceptable
    /// failure for an index — the subject stays recognisable, and pulling the memory is what
    /// supplies the point. It holds because this corpus leads with its claim, and is worth
    /// re-checking against a set that does not.
    ///
    /// Named `as_data` for the reason [`MemoryText::as_data`] is: this is memory text on its way to
    /// something that will read it.
    #[must_use]
    pub fn summary_as_data(&self) -> String {
        summarise(&self.0, SUMMARY_CAP)
    }
}

/// How many characters a derived summary line may run to.
///
/// **Measured rather than chosen**, against the convention catalog, which is the reference this
/// project already uses for context cost. The per-line JSON envelope including the 26-character id
/// is 49 characters, so a hundred lines at this cap cost 0.9x the catalog against 4.2x for a
/// hundred whole records. At 80 the saving over this is 0.2x and the second clause — *when it
/// applies* — gets squeezed out. The envelope is 29% of a line at this cap, which is the ceiling
/// this exposes: an index scales about five times better than whole records, not indefinitely.
pub const SUMMARY_CAP: usize = 120;

/// What a summary line ends with when the memory did not fit in it.
///
/// One character rather than three dots: it costs one of the cap's 120 where `...` costs three, and
/// it cannot be read as a sentence end followed by two more.
pub const CUT_MARKER: char = '\u{2026}';

/// The cut itself, with the cap a parameter so it can be exercised at sizes a test can read.
fn summarise(text: &str, cap: usize) -> String {
    // A line has no interior newlines, and a memory's does. `MemoryText` keeps the interior exactly
    // as written because that is what was recorded; a *projection* of it is free to be one line,
    // and has to be, or an index of lines has entries that are two.
    let line: Vec<char> = {
        let mut collapsed = String::with_capacity(text.len());
        for (index, word) in text.split_whitespace().enumerate() {
            if index > 0 {
                collapsed.push(' ');
            }
            collapsed.push_str(word);
        }
        collapsed.chars().collect()
    };

    if line.len() <= cap {
        return line.into_iter().collect();
    }

    // Everything below is a cut, so every path below ends in the marker -- which means the text has
    // one character less than the cap to work with. Taking it out of the budget here rather than
    // appending past the cap is what keeps the cap binding: the lines that would overrun are
    // precisely the ones already at it.
    let room = cap - 1;

    // The last sentence end that fits. A terminator counts only when whitespace follows it, which
    // is what keeps `0.5` and `store.rs` out of it — the common case in this corpus, and free to
    // exclude.
    //
    // **And only when the sentence it closes is more than one word.** A one-word sentence is a list
    // number or a bare answer — `1.`, `No.` — and cutting there leaves a line that names nothing,
    // which is `a-numbered-opening-summarises-to-its-number`. Passing over it costs a few characters
    // at worst, which is the bargain the last-fit rule already makes with a false boundary. A
    // length floor would catch more, and was declined: it needs a number, and every short sentence
    // ending in the live corpora was a heading worth keeping.
    let is_terminator = |at: usize| matches!(line[at], '.' | '!' | '?');
    let ends_a_sentence = |at: usize| {
        is_terminator(at)
            && line.get(at + 1).is_some_and(|next| *next == ' ')
            && line[..at]
                .iter()
                .rposition(|c| *c == ' ')
                .is_some_and(|space| !is_terminator(space - 1))
    };
    let cut = if let Some(end) = (0..room).rev().find(|at| ends_a_sentence(*at)) {
        end + 1
    } else if line[room] == ' ' {
        // A word ending exactly where the room runs out counts, which is the one case where the
        // character *after* the window decides — and the reason to look at it rather than to widen
        // the window: the check script that measured this corpus searched to `cap + 1` and returned
        // a line one character over.
        room
    } else {
        // No boundary at all means one word longer than the whole line. Cutting where the room runs
        // out is the only answer that still names something; an empty line names nothing.
        line[..room].iter().rposition(|c| *c == ' ').unwrap_or(room)
    };

    let mut summary: String = line[..cut].iter().collect();
    summary.push(CUT_MARKER);
    summary
}

impl TryFrom<String> for MemoryText {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<MemoryText> for String {
    fn from(text: MemoryText) -> Self {
        text.0
    }
}

/// A single keyword attached to a memory.
///
/// Trimmed at construction and otherwise **stored exactly as written**. Case-insensitive matching
/// is real and lives in [`Keyword::folded`], which is the lookup key — a separate thing from the
/// value, deliberately.
///
/// Folding the value itself is what this type used to do, and it was a defect: the caller's
/// spelling was destroyed with no way to recover or even observe it. Note the consequence for
/// `PartialEq` and `Hash`, which now distinguish `MemoryView` from `memoryview` — that is correct
/// for a *record*, and every place that needs matching semantics must go through `folded` rather
/// than comparing keywords directly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Keyword(String);

impl Keyword {
    /// Construct a keyword, keeping the case it was written with.
    ///
    /// Surrounding whitespace is trimmed, because that is not content — but the value is otherwise
    /// stored exactly as given. This used to end `to_lowercase()`, which was a defect
    /// (`keywords-silently-lowercased`): a keyword is **recorded content** that happens also to be
    /// a lookup key, and folding the record to suit the index destroyed the caller's spelling
    /// irreversibly, with no mode able to show what had been supplied. [`MemoryText`] above already
    /// stated the principle — normalising inside it "would change what was recorded" — and the same
    /// argument was simply not carried the few lines across.
    ///
    /// Case-insensitive matching is unaffected and lives in [`Keyword::folded`].
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::Empty`] if the value is empty or only whitespace, and
    /// [`ValidationError::NotASingleWord`] if it contains internal whitespace.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Err(ValidationError::Empty { field: "a keyword" });
        }

        if trimmed.chars().any(char::is_whitespace) {
            return Err(ValidationError::NotASingleWord {
                value: trimmed.to_owned(),
            });
        }

        Ok(Self(trimmed.to_owned()))
    }

    /// The keyword as it was written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The lookup key: `toNFKC_Casefold`, the operation Unicode defines for caseless matching.
    ///
    /// The single place normalisation happens. Both the write path and the query path derive their
    /// key from here, so a keyword can only ever be found by the same rule it was indexed under —
    /// the alternative is two spellings of the rule, which is the shape that has caused most of the
    /// defects in this project.
    ///
    /// **Why the standard rather than a transformation of our own.** This used to be
    /// `to_lowercase`, which is a *partial* normalisation: enough that a caller cannot treat
    /// matching as byte identity, not enough to unify what a non-English author writes. Naming a
    /// standard also costs one sentence of served convention text where describing a bespoke
    /// mapping costs a paragraph that stays incomplete — and the reader who needs it is a model
    /// deciding, at the moment it coins a keyword, whether two spellings are one subject.
    /// `folding-is-lowercase-not-casefold` in `TRACKING.md` records the measurements.
    ///
    /// **Composed rather than called.** No crate exposes the derived `NFKC_Casefold` property
    /// directly; ICU4X had it in 1.x and removed it in 2.x. `NFKC → fold → NFKC` agrees with it on
    /// every case tested. **The leading `NFKC` is load-bearing, not defensive**: 664 single
    /// characters below U+30000 fold differently with and without it. `U+1D2C` (modifier letter
    /// capital A) is the clearest — normalised first it reaches `a`, folded first it stays `A` and
    /// no second pass recovers the case.
    ///
    /// **What it deliberately does not merge**, because each is a distinction a reader means to
    /// draw: hiragana against katakana, simplified against traditional Han, and the Turkish dotted
    /// `İ`, which keeps its combining mark under the standard. Caseless scripts come through
    /// untouched, so this cost CJK keywords nothing.
    #[must_use]
    pub fn folded(&self) -> String {
        caseless_form(&self.0)
    }
}

/// `toNFKC_Casefold`, the form [`Keyword::folded`] compares in: the one definition of "the same
/// word, whatever its case or width" that every caseless comparison here should share.
///
/// **Exposed so the sharing preferences' never-store list folds the same way**, after it was first
/// written with `to_lowercase` -- which left full-width ＨＩＶ and HIV two words, the
/// `folding-is-lowercase-not-casefold` defect a second time.
#[must_use]
pub fn caseless_form(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;

    let normalised: String = text.nfkc().collect();
    caseless::default_case_fold_str(&normalised)
        .nfkc()
        .collect()
}

impl TryFrom<String> for Keyword {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Keyword> for String {
    fn from(keyword: Keyword) -> Self {
        keyword.0
    }
}
