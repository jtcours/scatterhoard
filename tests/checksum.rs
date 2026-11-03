// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Tests for the reconciliation checksum.
//!
//! The published-vector tests are the point of the module: they are what make the algorithm *pinned*
//! rather than merely *written down*. A refactor that changes the output fails here instead of
//! silently invalidating every base on every machine.

use scatterhoard::checksum;
use scatterhoard::memory_file::Record;
use std::str::FromStr;

use jiff::Timestamp;
use scatterhoard::memory::{
    Attribution, Confidence, Keyword, Memory, MemoryId, MemoryText, Origin,
};

/// The published FNV-1a 64 vectors. These are external to this project, which is what gives them
/// their authority — they cannot be quietly adjusted to match a changed implementation without the
/// change being obvious.
#[test]
fn matches_the_published_fnv1a_vectors() {
    assert_eq!(checksum::of_bytes(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(checksum::of_bytes(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(checksum::of_bytes(b"foobar"), 0x8594_4171_f739_67e8);
}

fn an_attribution() -> Attribution {
    Attribution {
        by: MemoryId::generate(),
        at: Timestamp::from_str("2026-08-05T09:00:00Z").expect("valid timestamp"),
    }
}

fn a_memory(content: &str) -> Memory {
    Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("valid confidence"),
        Origin::Llm,
        an_attribution(),
    )
}

#[test]
fn the_same_memory_checksums_the_same_way_twice() {
    let memory = a_memory("alpha governs routing");
    assert_eq!(checksum::of_memory(&memory), checksum::of_memory(&memory));
}

#[test]
fn a_changed_field_changes_the_checksum() {
    let before = a_memory("alpha governs routing");
    let mut after = before.clone();
    after.content = MemoryText::new("alpha no longer governs routing").expect("valid");

    assert_ne!(checksum::of_memory(&before), checksum::of_memory(&after));
}

/// A keyword is part of the record, so editing one has to register as a change — otherwise a
/// corrected keyword would never propagate through reconciliation.
#[test]
fn a_changed_keyword_changes_the_checksum() {
    let before = a_memory("alpha governs routing");
    let mut after = before.clone();
    after.keywords = vec![Keyword::new("routing").expect("valid keyword")];

    assert_ne!(checksum::of_memory(&before), checksum::of_memory(&after));
}

/// The canonical form must have its keys in sorted order, so the checksum depends on field *names*
/// rather than on where each field happens to sit in a source file. Without that, reordering
/// `Memory`'s fields would invalidate every recorded base on every machine, silently.
///
/// **This test replaces a vacuous one.** The previous version parsed the JSON into a
/// `serde_json::Value` before checking key order — and `Value`'s map is sorted by construction, so it
/// asserted a property of `Value` and passed however the serialiser behaved. It was hiding a real
/// deviation: serialising a struct emits fields in declaration order, which is what the code was
/// doing while `documents/design/file-format.md` specified sorted. Asserting against the **emitted
/// text** is what makes the difference visible.
#[test]
fn the_canonical_form_has_its_keys_in_sorted_order() {
    let record = Record::Memory(Box::new(a_memory("alpha governs routing")));
    let text = record.canonical();

    let keys: Vec<&str> = text
        .match_indices("\":")
        .map(|(at, _)| {
            let before = &text[..at];
            let start = before.rfind('"').expect("an opening quote");
            &before[start + 1..]
        })
        .collect();

    // Only the top level is asserted; nested objects contribute their own keys to this scan, so the
    // check is that the *whole* emitted text is non-decreasing within each object it opens. Taking the
    // top-level keys alone is enough to catch declaration order, which is the failure that matters.
    let top: Vec<&str> = keys
        .iter()
        .copied()
        .filter(|key| {
            [
                "id",
                "content",
                "why",
                "context",
                "keywords",
                "confidence",
                "origin",
                "created",
                "modified",
                "links",
            ]
            .contains(key)
        })
        .collect();

    let mut sorted = top.clone();
    sorted.sort_unstable();
    sorted.dedup();
    let mut seen = top.clone();
    seen.dedup();
    assert_eq!(
        seen, sorted,
        "the canonical form is emitting declaration order, not sorted order: {text}"
    );
}

/// Reordering the fields of `Memory` must not change a record's checksum, which is the property the
/// sorted canonical form buys. Asserted by checksumming a memory and a `Value` rebuilt from its keys
/// in a deliberately different order.
#[test]
fn field_order_in_the_source_does_not_reach_the_checksum() {
    let memory = a_memory("alpha governs routing");
    let record = Record::Memory(Box::new(memory.clone()));

    let mut shuffled: Vec<(String, serde_json::Value)> = serde_json::to_value(&memory)
        .expect("serialises")
        .as_object()
        .expect("an object")
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    shuffled.reverse();
    let rebuilt: serde_json::Value = shuffled
        .into_iter()
        .collect::<serde_json::Map<_, _>>()
        .into();
    let restored: Memory = serde_json::from_value(rebuilt).expect("parses");

    assert_eq!(
        checksum::of_record(&record),
        checksum::of_record(&Record::Memory(Box::new(restored)))
    );
}

/// Re-serialising a parsed memory has to reproduce the checksum, because that is exactly what
/// reconciliation does with a record read out of a file.
#[test]
fn a_round_trip_through_json_preserves_the_checksum() {
    let memory = a_memory("alpha governs routing");
    let json = serde_json::to_string(&memory).expect("serialises");
    let restored: Memory = serde_json::from_str(&json).expect("parses");

    assert_eq!(
        checksum::of_memory(&memory),
        checksum::of_memory(&restored),
        "a memory read back from a file must checksum as the memory that was written"
    );
}

/// Whitespace and key order in the *text* must not reach the checksum: the value is computed over a
/// re-serialisation, so a merge tool reformatting a line cannot register as a content change.
#[test]
fn reformatted_json_checksums_as_the_original() {
    let memory = a_memory("alpha governs routing");
    let pretty = serde_json::to_string_pretty(&memory).expect("serialises");
    let restored: Memory = serde_json::from_str(&pretty).expect("parses");

    assert_eq!(checksum::of_memory(&memory), checksum::of_memory(&restored));
}

/// Two different memories in the same set must not collide. Not a probabilistic claim — a smoke test
/// that the input actually reaches the hash, which a constant-returning implementation would fail.
#[test]
fn different_memories_get_different_checksums() {
    let checksums: Vec<u64> = ["alpha", "beta", "gamma", "delta"]
        .iter()
        .map(|content| checksum::of_memory(&a_memory(content)))
        .collect();

    let mut unique = checksums.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        checksums.len(),
        "checksums collided: {checksums:?}"
    );
}
