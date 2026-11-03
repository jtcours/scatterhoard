// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The mechanical prohibited-data scan.
//!
//! `documents/design/prohibited-data.md` names what must not reach a repository: secret API keys,
//! passwords, and credit card numbers. `documents/design/export-review.md` settles where the check
//! runs -- in `export`, because it reads the whole set and needs nothing the pre-commit moment
//! provides.
//!
//! # It matches shapes, never vocabulary
//!
//! The tempting implementation looks for the words. It cannot work here, and the reason is specific
//! rather than fastidious: **a memory set about a memory system discusses its own prohibitions**.
//! This project's `route-where` convention names API keys, passwords and card numbers in a single
//! sentence, so a vocabulary scan refuses this repository's own export on its own rules. The corpus
//! checker made exactly this mistake once already, searching a corpus about links for the word
//! `links` (see `TRACKING.md`).
//!
//! So every rule below keys on structure: a vendor's own key prefix, a digit run that satisfies
//! Luhn, a PEM armour line. Those are things a document *about* secrets does not accidentally
//! contain, and things a leaked secret cannot avoid containing.
//!
//! # What it does not find, and says so
//!
//! **Passwords have no shape.** `hunter2` and a passphrase of four ordinary words are
//! indistinguishable from prose by any structural rule, and a scan that guessed would produce false
//! positives on ordinary memories while still missing most real passwords. This module does not
//! look for them, `tests/prohibited.rs` pins that, and any report it feeds must not imply otherwise
//! -- the design's word is *to the extent possible*, and this is the edge of possible.
//!
//! High-entropy tokens with no vendor prefix are also deliberately out. A memory set is full of
//! long opaque identifiers -- memory ids, commit hashes, model identities -- and an entropy
//! threshold that catches an unknown vendor's key catches those too.

use serde::Serialize;

/// What kind of prohibited data a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// A credential matching a known vendor's key format.
    ApiKey,
    /// A digit run that satisfies the Luhn checksum.
    CardNumber,
    /// PEM armour for a private key.
    PrivateKey,
}

/// One match, located well enough to find by eye without reproducing the secret.
///
/// **Deliberately carries no copy of what matched.** A finding travels into a tool response and may
/// be read, logged, or pasted somewhere else; putting the credential in it would spread the thing
/// the scan exists to contain. The category and the offset are enough to find it in the text the
/// caller already holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// What was recognised.
    pub category: Category,
    /// Byte offset into the scanned text where the match starts.
    pub at: usize,
}

/// Known vendor key prefixes, and the minimum number of key characters that must follow.
///
/// Each entry is a published format rather than a guess. `sk-ant-` is listed before `sk-` because
/// the scan takes the first prefix that matches and the longer one is the more precise report.
const VENDOR_PREFIXES: &[(&str, usize)] = &[
    ("sk-ant-", 20),
    ("sk-", 20),
    ("ghp_", 30),
    ("gho_", 30),
    ("ghu_", 30),
    ("ghs_", 30),
    ("ghr_", 30),
    ("github_pat_", 30),
    ("AKIA", 16),
    ("ASIA", 16),
    ("AIza", 30),
    ("xoxb-", 20),
    ("xoxp-", 20),
    ("xoxa-", 20),
    ("xoxr-", 20),
    ("xoxs-", 20),
];

/// PEM armour. Matching the opening line is enough; a key without it is not in PEM form anyway.
const PEM_MARKERS: &[&str] = &[
    "-----BEGIN RSA PRIVATE KEY-----",
    "-----BEGIN PRIVATE KEY-----",
    "-----BEGIN EC PRIVATE KEY-----",
    "-----BEGIN DSA PRIVATE KEY-----",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "-----BEGIN PGP PRIVATE KEY BLOCK-----",
];

/// Find prohibited data in one piece of text.
///
/// Findings come back in the order they appear. An empty result means nothing *this scan
/// recognises* is present, which is a narrower claim than "no secrets here" -- see the module note
/// on passwords.
#[must_use]
pub fn scan(text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(pem_findings(text));
    findings.extend(vendor_key_findings(text));
    findings.extend(card_findings(text));
    findings.sort_by_key(|finding| finding.at);
    findings
}

fn pem_findings(text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for marker in PEM_MARKERS {
        let mut from = 0;
        while let Some(offset) = text[from..].find(marker) {
            let at = from + offset;
            findings.push(Finding {
                category: Category::PrivateKey,
                at,
            });
            from = at + marker.len();
        }
    }
    findings
}

fn vendor_key_findings(text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (prefix, minimum) in VENDOR_PREFIXES {
        let mut from = 0;
        while let Some(offset) = text[from..].find(prefix) {
            let at = from + offset;
            let body = &text[at + prefix.len()..];
            let run = body.chars().take_while(|c| key_character(*c)).count();
            // **Only where a key could begin.** `vendor-prefix-scan-matches-inside-words`: `ask-`,
            // `task-` and `risk-` end in `sk-`, and a long enough hyphenated slug after one was
            // refused as an API key. A prefix preceded by a character that could itself be part of
            // a key is the middle of a word, not the start of a credential; anything else before it
            // -- a space, `=`, `:`, a quote, a slash, the start of the text -- still counts.
            let starts_a_word = !text[..at].chars().next_back().is_some_and(key_character);
            if starts_a_word && run >= *minimum {
                findings.push(Finding {
                    category: Category::ApiKey,
                    at,
                });
            }
            from = at + prefix.len();
        }
    }
    // A longer prefix and a shorter one can both match at the same place -- `sk-ant-` and `sk-`.
    // One credential is one finding, so the later duplicate at the same offset is dropped.
    findings.sort_by_key(|finding| finding.at);
    findings.dedup_by_key(|finding| finding.at);
    findings
}

/// Whether `c` can appear in a vendor key's body: letters, digits, `_` and `-`.
fn key_character(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Digit runs that satisfy Luhn, with spaces and hyphens treated as separators inside a run.
///
/// **Luhn is what makes this usable rather than the length test.** Memory sets are full of long
/// digit runs -- timestamps, counts, ids -- and roughly nine in ten of them fail the checksum, so
/// the check turns a rule that would fire constantly into one that rarely does.
fn card_findings(text: &str) -> Vec<Finding> {
    let bytes = text.as_bytes();
    let mut findings = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        // A run may contain separators, but must not end on one.
        let start = index;
        let mut end = index;
        let mut digits = Vec::new();
        let mut cursor = index;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'0'..=b'9' => {
                    digits.push(bytes[cursor] - b'0');
                    cursor += 1;
                    end = cursor;
                }
                b' ' | b'-' if cursor + 1 < bytes.len() && bytes[cursor + 1].is_ascii_digit() => {
                    cursor += 1;
                }
                _ => break,
            }
        }

        if (13..=19).contains(&digits.len()) && luhn(&digits) {
            findings.push(Finding {
                category: Category::CardNumber,
                at: start,
            });
        }
        index = end.max(start + 1);
    }
    findings
}

/// The Luhn checksum, as published. Digits are most-significant first.
fn luhn(digits: &[u8]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(position, digit)| {
            let value = u32::from(*digit);
            if position % 2 == 0 {
                value
            } else if value * 2 > 9 {
                value * 2 - 9
            } else {
                value * 2
            }
        })
        .sum();
    sum.is_multiple_of(10)
}
