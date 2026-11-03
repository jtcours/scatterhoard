// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Globally unique, creation-ordered identifiers: one for a memory, one for a set.
//!
//! Both live here because they share the spelling and the reasoning behind it, and a second module
//! would either duplicate the encoder or export it for one caller.

use std::fmt;
use std::str::FromStr;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use super::ValidationError;

/// A memory's globally unique identifier.
///
/// Globally unique rather than per-set because Scatterhoard serves distributed development and
/// memories carry cross-references between sets (see `documents/design/cross-set-references.md`).
///
/// Identifiers are UUIDv7, so their sort order is their creation order. That is load-bearing
/// rather than incidental: `documents/design/file-format.md` asks for a consistent export order to
/// keep merge diffs small, and names memory ID or creation time as the key — with a time-ordered
/// identifier those are one order, not two. Keyword recall breaks ties on the same order.
///
/// **The bits are UUIDv7; the spelling is Crockford base32** — 26 characters, no separators, which
/// is ULID's text encoding. Decided 2026-08-20; see `documents/design/memory-data-model.md`. The
/// reason is not brevity but the truncation instinct: a hex UUID's `8-4-4-4-12` grouping offers a
/// pre-segmented first group, so cutting there feels like taking a natural boundary rather than
/// choosing a length, and every truncation recorded in this project landed on exactly those eight
/// characters. The entropy is at the *end*, so that is the one cut position carrying no identity.
/// This form offers no cut point and does not read as a hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemoryId(Uuid);

/// Crockford's alphabet: no `I`, `L`, `O` or `U`. Ordered, which is what keeps the text form's sort
/// equal to the value's — see `the_text_form_sorts_into_the_same_order_as_the_id`.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// How many characters 128 bits takes at five bits each: 26, with the leading character carrying
/// only three. A leading character above `7` therefore describes a value too large to be an id, and
/// the checked arithmetic in [`decode_crockford`] is what refuses it.
const ENCODED_LEN: usize = 26;

fn encode_crockford(value: u128) -> String {
    let mut out = String::with_capacity(ENCODED_LEN);
    for position in 0..ENCODED_LEN {
        // 125 rather than 128: the first character is the 3-bit remainder, not a full group.
        let shift = 125 - position * 5;
        let digit = usize::try_from((value >> shift) & 0x1f).unwrap_or(0);
        out.push(char::from(CROCKFORD[digit]));
    }
    out
}

/// One Crockford character's value, resolving the characters people and models confuse.
///
/// `O` reads as zero and `I`/`L` as one. **That mapping is the reason this alphabet was chosen over
/// a merely denser one**: a fragment transcribed slightly wrong still resolves, which is what a
/// recovery mechanism is for.
fn crockford_digit(ch: char) -> Option<u8> {
    let upper = u8::try_from(ch).ok()?.to_ascii_uppercase();
    match upper {
        b'O' => Some(0),
        b'I' | b'L' => Some(1),
        _ => CROCKFORD
            .iter()
            .position(|&c| c == upper)
            .and_then(|p| u8::try_from(p).ok()),
    }
}

/// The last id this process issued, memory and set ids alike, as its raw bits.
static LAST_ISSUED: Mutex<u128> = Mutex::new(0);

/// Mint a UUIDv7 that sorts after every one this process has minted before it.
///
/// **`Uuid::now_v7` alone does not promise that.** It reads the clock before taking its own lock,
/// and a reading from an earlier *second* that arrives late moves its stored time almost a second
/// ahead — it compares seconds and sub-second nanoseconds separately — after which the next
/// millisecond reseeds back to the true time, and every id in between sorts after the ones that
/// follow it. Reproduced on `uuid` 1.24.0 and 1.26.1; `generated-ids-can-sort-out-of-creation-order`
/// in `TRACKING.md`. Remembering the last id and stepping past it holds whatever the crate does.
fn issue() -> Uuid {
    let candidate = Uuid::now_v7().as_u128();
    let mut last = LAST_ISSUED.lock().unwrap_or_else(PoisonError::into_inner);
    *last = step_past(candidate, *last);
    Uuid::from_u128(*last)
}

/// The low 62 bits of a UUIDv7: `rand_b`, below the variant.
const RAND_B: u128 = (1 << 62) - 1;

/// `candidate` if it sorts after `last`, otherwise the next UUIDv7 after `last`.
///
/// **The step counts through the 122 bits that are neither version nor variant**, so a carry out of
/// `rand_b` lands in `rand_a` and one out of `rand_a` in the timestamp, and the result is still a
/// UUIDv7. A plain `last + 1` would carry into the variant instead. Exhausting all 122 would take a
/// timestamp past the year 10889.
fn step_past(candidate: u128, last: u128) -> u128 {
    if candidate > last {
        return candidate;
    }
    let free = ((last >> 80) << 74) | (((last >> 64) & 0xfff) << 62) | (last & RAND_B);
    let next = free + 1;
    ((next >> 74) << 80)
        | (0x7 << 76)
        | (((next >> 62) & 0xfff) << 64)
        | (0b10 << 62)
        | (next & RAND_B)
}

/// `None` for anything that is not exactly one encoded id.
///
/// **Hyphens are refused rather than skipped**, though Crockford's own specification says a decoder
/// should ignore them. Permitting them would put a separator back into the form, and the separator
/// is what this encoding exists to remove — a spelling that offers a tidy place to cut is the whole
/// defect, whatever the decoder is willing to accept afterwards.
fn decode_crockford(text: &str) -> Option<u128> {
    if text.chars().count() != ENCODED_LEN {
        return None;
    }
    let mut value: u128 = 0;
    for ch in text.chars() {
        let digit = crockford_digit(ch)?;
        // Overflows exactly when the leading character exceeds `7`, which is a 130-bit value
        // wearing an id's clothes.
        value = value.checked_mul(32)?.checked_add(u128::from(digit))?;
    }
    Some(value)
}

impl MemoryId {
    /// Mint a new identifier for a memory being created now.
    ///
    /// Named `generate` rather than `new` because it is not a pure conversion — two calls give two
    /// different values, and the ordering guarantee above depends on when it is called.
    #[must_use]
    pub fn generate() -> Self {
        Self(issue())
    }

    /// This id, or the next one after `last` if this one does not sort after it.
    ///
    /// It is the decision [`MemoryId::generate`] makes against the last id the process issued, made
    /// public so it can be tested with ids chosen by hand: the generator failure it guards against
    /// needs a thread pre-empted across a second boundary, which no test can arrange.
    #[must_use]
    pub fn or_next_after(self, last: Self) -> Self {
        Self(Uuid::from_u128(step_past(self.to_u128(), last.to_u128())))
    }

    /// Parse an identifier read from a memory file or a tool call.
    ///
    /// **Both spellings are accepted, and only one is written.** The hex form is still read because
    /// a set travels between machines that upgrade at different times, so a file or a store may
    /// already hold it (`documents/design/file-format.md`). Anything read in hex is written back in
    /// the new form, so a set converges rather than carrying two spellings indefinitely.
    ///
    /// The two cannot be confused: an encoded id is 26 characters and a hex UUID is 32 or 36.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::MalformedMemoryId`] if `value` is neither form.
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        if let Some(bits) = decode_crockford(value) {
            return Ok(Self(Uuid::from_u128(bits)));
        }
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| ValidationError::MalformedMemoryId {
                value: value.to_owned(),
            })
    }

    /// Fold a *fragment* of an id into the spelling ids are stored in.
    ///
    /// `None` if the fragment is empty or holds anything that is not a Crockford digit — which also
    /// keeps `%` and `_` out of the `LIKE` pattern the store builds from it.
    ///
    /// **This is where the alphabet earns its choice.** A fragment carrying `O` for `0` or `I` for
    /// `1` still finds its memory, which is the transcription error a recovery mechanism exists to
    /// absorb. Canonicalising here rather than at the query means the stored spelling stays the one
    /// true form and only the *input* is repaired.
    #[must_use]
    pub fn normalise_fragment(fragment: &str) -> Option<String> {
        if fragment.is_empty() {
            return None;
        }
        fragment
            .chars()
            .map(|ch| crockford_digit(ch).map(|digit| char::from(CROCKFORD[usize::from(digit)])))
            .collect()
    }

    /// The raw 128 bits, for callers that need the value rather than a spelling.
    #[must_use]
    pub fn to_u128(&self) -> u128 {
        self.0.as_u128()
    }
}

// Written and read as its own text form rather than the `Uuid`'s. `#[serde(transparent)]` would
// spell every id in a memory file as hex, which would leave the file format unchanged — the one
// place the decision most needs to land.
impl Serialize for MemoryId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for MemoryId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for MemoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Crockford base32, the form that goes into the file. `parse` also accepts the hex form
        // this replaced, but nothing writes it.
        write!(f, "{}", encode_crockford(self.0.as_u128()))
    }
}

impl FromStr for MemoryId {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// A memory set's own identity, minted once and never changed.
///
/// **A name is not an identity.** Two people can both call a set `notes`, and until this existed the
/// name in the file header was the only thing separating them — so `Expectation::Exactly` was
/// satisfied by an unrelated set, and importing one into the other produced a mass of unrelated
/// inserts plus memory ids living in two sets at once. `TRACKING.md`, *whether
/// a set carries a UUID*.
///
/// **UUIDv7 and Crockford base32, exactly as [`MemoryId`] is**, and the consistency is the argument:
/// this project spells ids one way, for the truncation reasoning recorded on that type, and a set id
/// is read by people in a file header and in refusals — which is precisely where a second spelling
/// would invite a cut at a boundary carrying no identity. The time-ordering v7 gives is unused here
/// and harmless; a set's creation moment is in the repository's history regardless.
///
/// **It has no legacy spelling to accept.** [`MemoryId::parse`] reads hex because sets in the wild
/// already carried ids written that way; no set id has ever been written in any form, so accepting a
/// second spelling would be inventing compatibility with nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SetId(Uuid);

impl SetId {
    /// Mint an identity for a set being created now.
    #[must_use]
    pub fn generate() -> Self {
        Self(issue())
    }

    /// Parse a set id read from a file header, a store, or a tool call.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::MalformedSetId`] if `value` is not the one spelling.
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        decode_crockford(value)
            .map(|bits| Self(Uuid::from_u128(bits)))
            .ok_or_else(|| ValidationError::MalformedSetId {
                value: value.to_owned(),
            })
    }
}

impl Serialize for SetId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SetId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for SetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", encode_crockford(self.0.as_u128()))
    }
}

impl FromStr for SetId {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}
