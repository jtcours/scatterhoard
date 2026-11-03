// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! What the prohibited-data scan recognises, and — as much of the file as anything — what it does
//! not.
//!
//! `documents/design/prohibited-data.md` names three categories. Two of them have shapes; the third
//! does not, and the tests saying so are load-bearing rather than decorative. A scan that claimed to
//! find passwords would be worse than one that says it cannot.

use scatterhoard::prohibited::{Category, scan};

/// Every credential here is fabricated. The prefixes are real vendor prefixes; the bodies are not.
const FAKE_AWS: &str = "AKIAIOSFODNN7EXAMPLE";
const FAKE_GITHUB: &str = "ghp_016c75a7f9b24e0d8c3aa15be92d4f7c1a8e";
const FAKE_ANTHROPIC: &str = "sk-ant-api03-Zx8Qw2Lm4Np7Rt1Yv6Bc3Fj9Hk5Ds0Ag";
const FAKE_CARD: &str = "4242424242424242";

#[test]
fn a_vendor_key_is_found_wherever_it_sits_in_the_text() {
    for secret in [FAKE_AWS, FAKE_GITHUB, FAKE_ANTHROPIC] {
        let text = format!("the deploy step needs {secret} exported first");
        let found = scan(&text);
        assert_eq!(
            found.iter().map(|f| f.category).collect::<Vec<_>>(),
            vec![Category::ApiKey],
            "missed {secret}"
        );
    }
}

#[test]
fn a_card_number_is_found_by_luhn_rather_than_by_length() {
    let found = scan(&format!("card on file is {FAKE_CARD}"));
    assert_eq!(
        found.iter().map(|f| f.category).collect::<Vec<_>>(),
        vec![Category::CardNumber]
    );
}

#[test]
fn a_long_digit_run_that_fails_luhn_is_not_a_card_number() {
    // A timestamp, an id, a phone number with an extension: all digit runs, none a card.
    for digits in ["4242424242424243", "1234567890123456", "20260820193000000"] {
        assert!(
            scan(&format!("the value was {digits}")).is_empty(),
            "{digits} should not read as a card number"
        );
    }
}

#[test]
fn separators_inside_a_card_number_do_not_hide_it() {
    for written in ["4242 4242 4242 4242", "4242-4242-4242-4242"] {
        assert_eq!(
            scan(written).len(),
            1,
            "{written} should still be recognised"
        );
    }
}

#[test]
fn a_private_key_block_is_found() {
    let text = "-----BEGIN RSA PRIVATE KEY-----\nMIIEow...\n-----END RSA PRIVATE KEY-----";
    assert_eq!(
        scan(text).iter().map(|f| f.category).collect::<Vec<_>>(),
        vec![Category::PrivateKey]
    );
}

// ---------------------------------------------------------------------------
// What it must not do. These are the tests that keep the scan usable.
// ---------------------------------------------------------------------------

/// **The defect this whole design is shaped around.** This project's own memories discuss API keys,
/// passwords and card numbers by name -- the `route-where` convention lists all three. A scan
/// matching vocabulary would refuse this repository's own export, which is the same failure the
/// corpus checker had when it searched for the words `links` and `target` in a corpus about links.
#[test]
fn writing_about_secrets_is_not_a_secret() {
    let prose = "secrets -- API keys, passwords, card numbers -- are never stored in any set, so \
                 put them in a file and record where the file is. Never paste a password here.";
    assert!(
        scan(prose).is_empty(),
        "matching vocabulary would fail this project's own export: {:?}",
        scan(prose)
    );
}

#[test]
fn ordinary_identifiers_are_not_keys() {
    // Memory ids, commit hashes, checksums and model identities are all long opaque tokens, and
    // every memory set is full of them.
    let text =
        "memory 01M0H8PSFNFA99SNN6SJ9E9C75 was fixed in ae13c22, model fnv1a64:6129f0a378649d02";
    assert!(scan(text).is_empty(), "{:?}", scan(text));
}

/// Stated as a test because the honest limit belongs where someone will meet it. A password has no
/// shape -- `hunter2` and `correct horse battery staple` are indistinguishable from prose -- so the
/// scan does not claim to find one, and the response must not imply it did.
#[test]
fn a_password_has_no_shape_and_is_not_claimed() {
    for text in [
        "my password is hunter2",
        "the passphrase is correct horse battery staple",
    ] {
        assert!(
            scan(text).is_empty(),
            "the scan must not pretend to a capability it does not have: {text}"
        );
    }
}

/// A prefix alone is not a credential, and this is the realistic case rather than a contrived one:
/// a memory set about secret handling discusses key *formats*, naming the prefixes. Without the
/// minimum-length requirement, writing down which prefix a vendor uses would be unstorable.
#[test]
fn naming_a_key_prefix_is_not_carrying_a_key() {
    let prose = "OpenAI keys start sk-, Anthropic's sk-ant-, GitHub tokens ghp_, and AWS AKIA";
    assert!(scan(prose).is_empty(), "{:?}", scan(prose));
}

/// A truncated or redacted key must not match either, which is the same rule from the other side:
/// people paste the first few characters of a key when discussing one.
#[test]
fn a_redacted_key_is_not_a_key() {
    for redacted in ["ghp_016c75...", "AKIAIOSF****", "sk-ant-api03-Zx8Q"] {
        assert!(
            scan(&format!("the token began {redacted}")).is_empty(),
            "{redacted} should not match"
        );
    }
}

/// **A vendor prefix inside an ordinary word is not a key.** `vendor-prefix-scan-matches-inside-words`:
/// `ask-`, `task-`, `risk-` and `desk-` all end in a vendor prefix, and a hyphenated slug long enough
/// after it was refused as an API key -- two tracking entries were, on 2026-10-11, for the slug
/// `ask-the-human-through-elicitation`.
#[test]
fn a_vendor_prefix_inside_a_word_is_not_a_key() {
    for text in [
        "ask-the-human-through-elicitation",
        "the task-scheduler-runs-every-night-at-two",
        "a risk-assessment-for-the-whole-release-train",
        "the desk-reservations-module-was-rewritten",
        "XAKIAIOSFODNN7EXAMPLE is a word that happens to contain one",
        "myghp_016c75a7f9b24e0d8c3aa15be92d4f7c1a8e",
    ] {
        assert!(
            scan(text).is_empty(),
            "{text:?} is not a key: {:?}",
            scan(text)
        );
    }
}

/// **A real key is still found wherever a key begins**, whatever comes before it -- the ways a
/// credential is actually written down, each of which a word-boundary rule must still catch.
#[test]
fn a_vendor_key_is_found_after_anything_that_can_precede_one() {
    for secret in [FAKE_AWS, FAKE_GITHUB, FAKE_ANTHROPIC] {
        for text in [
            secret.to_owned(),
            format!("export it: {secret}"),
            format!("OPENAI_API_KEY={secret}"),
            format!("token:{secret}"),
            format!("\"{secret}\""),
            format!("'{secret}'"),
            format!("`{secret}`"),
            format!("https://example.com/hook/{secret}"),
            format!("({secret})"),
            format!("first line\n{secret}"),
        ] {
            assert_eq!(
                scan(&text).iter().map(|f| f.category).collect::<Vec<_>>(),
                vec![Category::ApiKey],
                "missed {secret} in {text:?}"
            );
        }
    }
}
