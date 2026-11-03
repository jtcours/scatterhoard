// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The export review's judgement gate: the parts the server does whatever model runs it.
//!
//! `documents/design/export-review.md`, *The procedure*. A model answers who each sentence of a
//! memory is about and which kept-out items it mentions; everything here is what does not depend on
//! how well it answers.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::preferences::WordList;

/// A shape the pre-pass found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// An email address.
    Email,
    /// A home directory path that carries a user name.
    HomePath,
    /// A private network address, or a host name with an internal suffix.
    PrivateHost,
    /// Digits grouped the way phone numbers are written.
    Phone,
    /// A word the person's file says is never stored, listed after the memory was written.
    NeverStore,
    /// A name the set publishes under, used in a memory's text.
    Author,
}

/// One thing the pre-pass found, as the caller wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    /// What kind of shape it is.
    pub shape: Shape,
    /// The text it matched.
    pub matched: String,
}

/// Internal host-name suffixes: names that only resolve inside someone's own network.
const INTERNAL_SUFFIXES: [&str; 6] = ["local", "lan", "internal", "corp", "home", "intranet"];

/// What in `text` goes to the person whatever the model concludes.
///
/// **Only shapes with a reliable form**, decided by the human partner on 2026-10-22, because each
/// false positive costs the person a glance and too many teach them to wave flags through. Other
/// personal names have no shape and are left to the model's answer to *who is this about*; the names
/// a set publishes under are exact, so they are matched.
///
/// **`always_fine` silences an exact value**, such as a work email, but never a never-store word:
/// where a value is in both lists the stricter promise wins.
#[must_use]
pub fn pre_pass(
    text: &str,
    never_store: &WordList,
    always_fine: &WordList,
    authors: &[String],
) -> Vec<Hit> {
    let chars: Vec<char> = text.chars().collect();
    let mut hits = Vec::new();
    hits.extend(emails(&chars).into_iter().map(|m| (Shape::Email, m)));
    hits.extend(home_paths(&chars).into_iter().map(|m| (Shape::HomePath, m)));
    hits.extend(
        private_hosts(&chars)
            .into_iter()
            .map(|m| (Shape::PrivateHost, m)),
    );
    hits.extend(phones(&chars).into_iter().map(|m| (Shape::Phone, m)));
    if let Some(found) = WordList::from_items(authors).find(text) {
        hits.push((Shape::Author, found.matched));
    }
    let mut kept: Vec<Hit> = hits
        .into_iter()
        .filter(|(_, matched)| always_fine.find(matched).is_none())
        .map(|(shape, matched)| Hit { shape, matched })
        .collect();
    if let Some(found) = never_store.find(text) {
        kept.push(Hit {
            shape: Shape::NeverStore,
            matched: found.matched,
        });
    }
    kept
}

/// The maximal run of characters satisfying `allowed` around `at`, as `(start, end)`.
fn run_around(chars: &[char], at: usize, allowed: impl Fn(char) -> bool) -> (usize, usize) {
    let mut start = at;
    while start > 0 && allowed(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at;
    while end < chars.len() && allowed(chars[end]) {
        end += 1;
    }
    (start, end)
}

fn emails(chars: &[char]) -> Vec<String> {
    let local = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let domain = |c: char| c.is_ascii_alphanumeric() || ".-".contains(c);
    let mut found = Vec::new();
    for (at, &c) in chars.iter().enumerate() {
        if c != '@' {
            continue;
        }
        let (start, _) = run_around(chars, at, local);
        let (_, mut end) = run_around(chars, at + 1, domain);
        while end > at + 1 && chars[end - 1] == '.' {
            end -= 1;
        }
        let host: String = chars[at + 1..end].iter().collect();
        let tld = host.rsplit('.').next().unwrap_or_default();
        // `git@host:path` is an SSH remote: `git` is the host's shared user, not anyone's mailbox.
        let ssh_remote = chars[start..at] == ['g', 'i', 't']
            && chars.get(end) == Some(&':')
            && chars.get(end + 1).is_some_and(|c| !c.is_whitespace());
        if start < at
            && !ssh_remote
            && host.contains('.')
            && tld.len() >= 2
            && tld.chars().all(|c| c.is_ascii_alphabetic())
        {
            found.push(chars[start..end].iter().collect());
        }
    }
    found
}

fn home_paths(chars: &[char]) -> Vec<String> {
    let lower: Vec<char> = chars.iter().flat_map(|c| c.to_lowercase()).collect();
    // Lower-casing ASCII never changes length, and these prefixes are ASCII; a text where it
    // changed length is compared position for position no worse than not at all.
    if lower.len() != chars.len() {
        return Vec::new();
    }
    let name = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
    let mut found = Vec::new();
    for prefix in ["/users/", "/home/", "c:\\users\\"] {
        let pattern: Vec<char> = prefix.chars().collect();
        for start in 0..lower.len().saturating_sub(pattern.len() - 1) {
            if lower[start..].starts_with(&pattern) {
                let from = start + pattern.len();
                let mut end = from;
                while end < chars.len() && name(chars[end]) {
                    end += 1;
                }
                let user: String = chars[from..end].iter().collect();
                if !user.is_empty() && !user.eq_ignore_ascii_case("shared") {
                    found.push(chars[start..end].iter().collect());
                }
            }
        }
    }
    found
}

fn private_hosts(chars: &[char]) -> Vec<String> {
    let host = |c: char| c.is_ascii_alphanumeric() || ".-".contains(c);
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if !host(chars[at]) {
            at += 1;
            continue;
        }
        let (start, mut end) = run_around(chars, at, host);
        at = end;
        while end > start && ".-".contains(chars[end - 1]) {
            end -= 1;
        }
        let token: String = chars[start..end].iter().collect();
        let labels: Vec<&str> = token.split('.').collect();
        let private_ip = labels.len() == 4
            && labels
                .iter()
                .all(|l| (1..=3).contains(&l.len()) && l.chars().all(|c| c.is_ascii_digit()))
            && {
                let octets: Vec<u32> = labels.iter().filter_map(|l| l.parse().ok()).collect();
                octets.iter().all(|&o| o <= 255)
                    && (octets[0] == 10
                        || (octets[0] == 192 && octets[1] == 168)
                        || (octets[0] == 172 && (16..=31).contains(&octets[1])))
            };
        let internal_name = labels.len() >= 2
            && labels[0].chars().any(|c| c.is_ascii_alphabetic())
            && INTERNAL_SUFFIXES.contains(&labels[labels.len() - 1].to_ascii_lowercase().as_str());
        if private_ip || internal_name {
            found.push(token);
        }
    }
    found
}

/// How long the ISO date `chars` begins with is, with the hour or `HHMM` stamp after it, or nothing
/// if it begins with no date. A time is two or four digits exactly, so a number written after a date
/// is not taken for one.
fn timestamp(chars: &[char]) -> Option<usize> {
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        let digits = chars.get(range)?;
        digits
            .iter()
            .all(char::is_ascii_digit)
            .then(|| digits.iter().collect::<String>().parse().ok())
            .flatten()
    };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let date = chars[4] == '-'
        && chars[7] == '-'
        && (1900..=2099).contains(&year)
        && (1..=12).contains(&month)
        && (1..=31).contains(&day);
    if !date {
        return None;
    }
    let time = chars.get(10).is_some_and(|c| "- T".contains(*c));
    let digits = chars
        .iter()
        .skip(11)
        .take_while(|c| c.is_ascii_digit())
        .count();
    Some(if time && (digits == 2 || digits == 4) {
        11 + digits
    } else {
        10
    })
}

fn phones(chars: &[char]) -> Vec<String> {
    let part = |c: char| c.is_ascii_digit() || " +()-.".contains(c);
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let begins = c.is_ascii_digit() || c == '+' || c == '(';
        let after_word = at > 0 && chars[at - 1].is_alphanumeric();
        if !begins || after_word {
            at += 1;
            continue;
        }
        // A date with the time after it reaches a phone number's digit count:
        // `the-pre-pass-reads-a-timestamp-as-a-phone-number`.
        if let Some(length) = timestamp(&chars[at..]) {
            at += length;
            continue;
        }
        let mut end = at;
        while end < chars.len() && part(chars[end]) {
            end += 1;
        }
        let next = end;
        while end > at && " .-(".contains(chars[end - 1]) {
            end -= 1;
        }
        // A run that runs straight into a letter is part of a word or an id, not a number.
        let into_word = next < chars.len() && chars[next].is_alphanumeric() && next == end;
        let run: String = chars[at..end].iter().collect();
        let groups: Vec<&str> = run
            .split(|c: char| !c.is_ascii_digit())
            .filter(|g| !g.is_empty())
            .collect();
        let digits: usize = groups.iter().map(|g| g.len()).sum();
        let international = run.starts_with('+') && (8..=15).contains(&digits) && groups.len() >= 2;
        let grouped = groups.len() >= 3
            && groups.iter().all(|g| (2..=4).contains(&g.len()))
            && (10..=15).contains(&digits);
        if !into_word && (international || grouped) {
            found.push(run.trim().to_owned());
        }
        at = next.max(at + 1);
    }
    found
}

/// Who can read a set's published file, as its project declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    /// Anyone. **The default**, so a project that never declares itself private exposes nothing
    /// through an exception meant for a private audience.
    #[default]
    Public,
    /// Only the people the repository is shared with. "While it is not public" exceptions apply.
    Private,
}

/// A project's additions to the export review, from `scatterhoard_mem/config/sharing-preferences.json`.
///
/// **Only what makes a review stricter, or quieter about the project's own exact values**, decided by
/// the human partner on 2026-10-22: the file arrives with every clone, so the worst a careless or
/// hostile one can do is make a review noisier. `audience` is the one fact it declares, and it can
/// only switch on exceptions the person wrote themselves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPreferences {
    /// Items every memory exported to the repository is checked against, beside the person's own.
    #[serde(default)]
    pub keep_out: Vec<String>,
    /// Exact values belonging to the project that the pre-pass does not flag.
    #[serde(default)]
    pub always_fine: Vec<String>,
    /// Who can read the published file.
    #[serde(default)]
    pub audience: Audience,
}

/// Where the project's file lives inside a checkout. A subdirectory, so no set name is taken: every
/// top-level `.json` in `scatterhoard_mem` is read as a memory set.
pub const PROJECT_FILE: &str = "scatterhoard_mem/config/sharing-preferences.json";

impl ProjectPreferences {
    /// Read the file's text.
    ///
    /// # Errors
    ///
    /// Why it is not a project file: **an unknown key is refused by name**, since an ignored one would
    /// look applied and do nothing, and so is a value of the wrong kind.
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|error| format!("{PROJECT_FILE} is not usable: {error}"))
    }

    /// The project file of the checkout at `checkout`, or `None` where there is none, which reads
    /// as a public project with nothing to add.
    ///
    /// # Errors
    ///
    /// As for [`ProjectPreferences::parse`], and when a file is there and cannot be read.
    pub fn load(checkout: &Path) -> Result<Option<Self>, String> {
        let path = checkout.join(PROJECT_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("could not read {}: {error}", path.display())),
        }
    }
}

/// One numbered sentence of a memory, as the review shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Part {
    /// Its number, counted across the memory's text, reasoning and context together.
    pub n: usize,
    /// Which field it comes from: `content`, `why` or `context`.
    pub field: &'static str,
    /// The sentence.
    pub text: String,
}

/// `text` split where a sentence ends: after `.`, `!`, `?` or `;` followed by white space.
#[must_use]
pub fn sentences(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current = String::new();
    let mut chars = text.trim().chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        if ".!?;".contains(c) && chars.peek().is_some_and(|next| next.is_whitespace()) {
            found.push(current.trim().to_owned());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        found.push(current.trim().to_owned());
    }
    found
}

/// A memory's text, reasoning and context as one numbered run of sentences.
///
/// **Numbered because asking about a memory as a whole let a detail after a harmless opening go**:
/// on `gemma4:12b` a memory reviewed alone was cleared in every run when its second sentence held
/// the detail, and in none once each sentence was asked about. All three fields, because an export
/// publishes the reasoning and context as surely as the text.
#[must_use]
pub fn numbered(content: &str, why: Option<&str>, context: Option<&str>) -> Vec<Part> {
    let mut parts = Vec::new();
    for (field, text) in [
        ("content", Some(content)),
        ("why", why),
        ("context", context),
    ] {
        for sentence in text.map(sentences).unwrap_or_default() {
            parts.push(Part {
                n: parts.len() + 1,
                field,
                text: sentence,
            });
        }
    }
    parts
}

/// Who a sentence is about, as the model answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum About {
    /// The work.
    #[serde(rename = "project")]
    Project,
    /// The person the session works with.
    #[serde(rename = "the person")]
    Person,
    /// Anyone else.
    #[serde(rename = "someone else")]
    SomeoneElse,
}

/// What a sentence about a person does, as the model answers: the line the Golden Rule's demands A
/// and D draw, which no shape in the text reveals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Says {
    /// What they did, said, wrote or decided.
    Reports,
    /// What they are like, or why they acted.
    Judges,
}

/// The model's answer for one numbered sentence.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SentenceAnswer {
    /// The sentence's number.
    pub n: usize,
    /// Who it is about.
    pub about: About,
    /// For a sentence about the person or someone else, whether it reports or judges.
    #[serde(default)]
    pub says: Option<Says>,
    /// The preference items it mentions, quoted as the file words them, from any list.
    #[serde(default)]
    pub mentions: Vec<String>,
}

/// How the review treats a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetKind {
    /// The personal set.
    Local,
    /// A shared set the person lists under *Sets I never publish*: reviewed like `local`.
    NeverPublished,
    /// Any other shared set.
    Shared,
}

/// What the server makes of a memory's answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Recorded as reviewed.
    Clear,
    /// Put to the person, with why.
    Flag(Vec<String>),
}

/// Text folded for comparing a quoted item with a listed one: case, width and punctuation aside.
fn folded_words(text: &str) -> String {
    let folded = crate::memory::caseless_form(text);
    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a quoted mention names a listed item: the same words once folded, or the item's opening
/// words. **Lenient because a small model paraphrases**: it quoted "where I live" for "where I
/// live, and how to contact me", which an exact match missed.
#[must_use]
pub fn names_item(mention: &str, item: &str) -> bool {
    let said = folded_words(mention);
    let listed = folded_words(item);
    !said.is_empty()
        && (said == listed || listed.starts_with(&format!("{said} ")) || said.starts_with(&listed))
}

/// The server's half of the review: what a memory's answers mean for its set.
///
/// **The model recognises; the server applies the lists.** On `gemma4:12b` the model reported the
/// items a memory mentioned reliably and did not work out which list applied to which set, so it
/// reports every item it recognises and this decides. In `local` and a never-published set an item
/// kept out only of shared sets does not count, unless the mention also names one kept out of every
/// set. In any other shared set a sentence about the person is misfiled, and any mention counts, so
/// a project's own keep-outs, shown to the model beside the person's, need nothing here.
///
/// **A report about someone else clears and a judgement never does**, decided by the human partner
/// on 2026-10-25, and a sentence about someone else is checked against
/// [`ListedPreferences::for_others`] rather than the person's own lists. A sentence about a person
/// with no answer for what it says counts as judging. Any pre-pass hit goes to the person.
#[must_use]
pub fn route(
    kind: SetKind,
    answers: &[SentenceAnswer],
    lists: &crate::preferences::ListedPreferences,
    hits: &[Hit],
) -> Decision {
    let mut reasons: Vec<String> = hits
        .iter()
        .map(|hit| format!("the pre-pass found {:?} \"{}\"", hit.shape, hit.matched))
        .collect();
    let personal = matches!(kind, SetKind::Local | SetKind::NeverPublished);
    let others = lists.for_others();
    for answer in answers {
        let judges = answer.says != Some(Says::Reports);
        match answer.about {
            About::SomeoneElse if judges => {
                reasons.push(format!("sentence {} judges someone else", answer.n));
            }
            About::Person => {
                if !personal {
                    reasons.push(format!("sentence {} is about the person", answer.n));
                }
                if judges {
                    reasons.push(format!("sentence {} judges the person", answer.n));
                }
            }
            _ => {}
        }
        let applying = if answer.about == About::SomeoneElse {
            &others
        } else {
            lists
        };
        for mention in &answer.mentions {
            let kept_everywhere = applying
                .keep_out_every
                .iter()
                .any(|item| names_item(mention, item));
            let shared_only = applying
                .keep_out_shared
                .iter()
                .any(|item| names_item(mention, item));
            if personal && shared_only && !kept_everywhere {
                continue;
            }
            let whose = if answer.about == About::SomeoneElse {
                " about someone else"
            } else {
                ""
            };
            reasons.push(format!(
                "sentence {} mentions \"{mention}\"{whose}",
                answer.n
            ));
        }
    }
    if reasons.is_empty() {
        Decision::Clear
    } else {
        Decision::Flag(reasons)
    }
}
