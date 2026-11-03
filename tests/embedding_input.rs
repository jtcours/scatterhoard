// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the text that gets embedded.
//!
//! This is the one part of the embedder with a measured answer behind it: the spike scored four
//! candidate inputs on an identical corpus and metric, and content+why+keywords beat content alone
//! by 14 points at recall@5. These tests pin the composition that was measured, so a later tidy-up
//! cannot quietly change what the numbers in `documents/design/embeddings.md` refer to.

use std::str::FromStr;

use jiff::Timestamp;
use scatterhoard::embedding::embedding_input;
use scatterhoard::memory::{
    Attribution, Confidence, Keyword, Memory, MemoryId, MemoryText, Origin,
};

fn a_memory(content: &str) -> Memory {
    Memory::new(
        MemoryText::new(content).expect("valid content"),
        Confidence::new(0.8).expect("in range"),
        Origin::Llm,
        Attribution {
            by: MemoryId::generate(),
            at: Timestamp::from_str("2026-08-01T09:00:00Z").expect("valid timestamp"),
        },
    )
}

/// The composition the spike measured: content, then why, then keywords, space-separated.
#[test]
fn the_input_is_content_then_why_then_keywords() {
    let mut memory = a_memory("storage is partitioned by scope");
    memory.why = Some(MemoryText::new("a forgotten WHERE clause leaks personal content").unwrap());
    memory.keywords = vec![
        Keyword::new("storage").unwrap(),
        Keyword::new("scope").unwrap(),
    ];

    assert_eq!(
        embedding_input(&memory),
        "storage is partitioned by scope \
         a forgotten WHERE clause leaks personal content \
         storage scope"
    );
}

/// `why` is optional in the type, so the common-enough case of a memory without one must not
/// produce a gap or a stray separator that the model then has to see through.
#[test]
fn a_memory_without_reasoning_contributes_only_what_it_has() {
    let mut memory = a_memory("just the content");
    memory.keywords = vec![Keyword::new("solo").unwrap()];

    assert_eq!(embedding_input(&memory), "just the content solo");
}

#[test]
fn a_memory_with_nothing_but_content_embeds_its_content() {
    assert_eq!(embedding_input(&a_memory("bare")), "bare");
}

/// Context is deliberately excluded. It is an episodic anchor — when and in response to what a
/// memory arose — and including it would pull semantically unrelated memories together merely
/// because they happened around the same time. It belongs on the output side, glossing a memory
/// once recall has already found it.
#[test]
fn context_is_not_embedded() {
    let mut memory = a_memory("the content");
    memory.context = Some(MemoryText::new("during the outage postmortem").unwrap());

    assert_eq!(embedding_input(&memory), "the content");
    assert!(!embedding_input(&memory).contains("postmortem"));
}

/// Every memory has content, and content is non-empty by construction, so the input can never be
/// empty — which matters because an empty string embeds to a vector with no direction, and those
/// are refused.
#[test]
fn the_input_is_never_empty() {
    assert!(!embedding_input(&a_memory("x")).trim().is_empty());
}
