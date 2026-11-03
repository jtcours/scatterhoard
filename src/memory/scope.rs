// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Which memory set a memory belongs to.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::ValidationError;

/// The name of a shared memory set.
///
/// A set name identifies a store on disk, so path traversal is not something to filter for later —
/// it is something a `SetName` cannot express. `/` and `\` are excluded outright, and `.` is legal
/// only *inside* a name, which is what keeps `.`, `..` and every hidden-file name unrepresentable.
/// Set names arrive from the same untrusted channel as the memories themselves (see
/// `documents/design/trigger-and-loading.md`).
///
/// The permitted set is shaped by what projects are actually called, since the natural set name is
/// the project's own: dotted names like `next.js` or `docs.example.com` are common, and an earlier
/// rule that excluded `.` altogether rejected most of them. A project whose name still does not fit
/// can declare a different set name in its memory file's header — the name defaults to the
/// project's, rather than being defined as it, so this rule only has to cover the common case
/// rather than every host's naming rules at once.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SetName(String);

/// What an evaluation load's name begins with, and what no *real* set's name may.
///
/// **Not refused by [`SetName::new`]**, and that was tried first. `new` is also how a set already
/// loaded is named — `drop`'s argument, a scope in a tool call, the store reading its own scope back
/// out of `meta` — so refusing there refuses an evaluation load the moment anything addresses it.
/// The ban belongs at the boundaries where a *real* set could come to carry the prefix instead:
/// creating one, a file's header, and a link written into a store. See
/// [`SetName::is_evaluation`] and `documents/design/load-modes.md`.
pub const EVALUATION_PREFIX: &str = "eval-";

impl SetName {
    /// Longest permitted set name, in characters.
    ///
    /// Matches the limit common hosting platforms impose on repository names, so a project name
    /// that is legal where the project lives is legal here.
    pub const MAX_LEN: usize = 100;

    /// Construct a set name.
    ///
    /// Permitted: ASCII letters and digits, plus `-`, `_` and `.` **inside** the name only — the
    /// first and last characters must be alphanumeric. That is one rule rather than a list of
    /// special cases, and it is what excludes `.`, `..`, and leading-dot names.
    ///
    /// The value is taken as given rather than trimmed or lowercased — a set name keys a store and
    /// is compared for equality when resolving cross-set references, so two spellings must not
    /// silently become one.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] if the value is empty, longer than [`SetName::MAX_LEN`],
    /// contains a character outside the permitted set, or begins or ends with punctuation.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();

        Self::checked(value)
    }

    /// The name of an evaluation load of `source`, distinguished by the user's `label`.
    ///
    /// **The only way to make a name carrying [`EVALUATION_PREFIX`]**, and used by the evaluation
    /// load alone. The label says which copy this is -- a branch, an issue, a version -- so several
    /// reviews can be open at once; it is the user's word, made into a legal set name rather than
    /// refused, since `feature/x` is a reasonable thing to type and not a set name.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] if the label carries nothing a name can keep, or if the result
    /// is too long.
    pub fn evaluation(source: &Self, label: &str) -> Result<Self, ValidationError> {
        let kept: String = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let label: String = kept
            .split('-')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        if label.is_empty() {
            return Err(ValidationError::Empty {
                field: "an evaluation label",
            });
        }
        Self::checked(format!("{EVALUATION_PREFIX}{source}-{label}"))
    }

    /// Every rule but the reserved prefix, which only [`new`](Self::new) applies.
    fn checked(value: String) -> Result<Self, ValidationError> {
        const FIELD: &str = "a set name";

        if value.is_empty() {
            return Err(ValidationError::Empty { field: FIELD });
        }

        // **Reserved, so the ambiguity is unrepresentable rather than detected.** `local` is the
        // scalar spelling of `Scope::Local`, so a shared set of that name would be a second set
        // answering to the same word: `scope: "local"` would mean one and a link's
        // `target_set: "local"` the other, in the same JSON object. Refusing at the constructor
        // closes every route at once -- `hoard_sets create`, a file header, the `shared:local`
        // scalar form and a link target -- where a check per field would have to be remembered at
        // each. `target-set-local-names-a-different-set` in `TRACKING.md`.
        //
        // Case-folded because the confusion is about how the word reads, not how it is spelled.
        // Only this one word: `shared` needs no reservation, since the scalar form's first colon is
        // always the separator and `shared:shared` reads exactly one way.
        if value.eq_ignore_ascii_case("local") {
            return Err(ValidationError::ReservedName {
                field: FIELD,
                value,
            });
        }

        let length = value.chars().count();
        if length > Self::MAX_LEN {
            return Err(ValidationError::TooLong {
                field: FIELD,
                max: Self::MAX_LEN,
                actual: length,
            });
        }

        if let Some(character) = value
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.'))
        {
            return Err(ValidationError::IllegalCharacter {
                field: FIELD,
                character,
            });
        }

        // Punctuation is legal inside a name and nowhere else. This single rule is what makes `.`,
        // `..` and `.hidden` unrepresentable while still admitting `next.js`.
        for character in [
            value.chars().next().expect("non-empty"),
            value.chars().next_back().expect("non-empty"),
        ] {
            if !character.is_ascii_alphanumeric() {
                return Err(ValidationError::IllegalEdgeCharacter {
                    field: FIELD,
                    character,
                });
            }
        }

        Ok(Self(value))
    }

    /// Whether this names an evaluation load rather than a set anyone has adopted.
    ///
    /// Case-folded, because what a reader takes from the name is how it reads.
    #[must_use]
    pub fn is_evaluation(&self) -> bool {
        self.0.len() >= EVALUATION_PREFIX.len()
            && self.0[..EVALUATION_PREFIX.len()].eq_ignore_ascii_case(EVALUATION_PREFIX)
    }

    /// Whether this name and `other` would name one store file on a case-insensitive filesystem.
    ///
    /// **Refused at every platform, not only where it collides**, because a data directory moves:
    /// two sets a Linux machine holds side by side are one file the moment they reach a Mac, where
    /// case-insensitivity is the default, as it is on Windows. The memory-file naming rule compares
    /// ignoring case for the same reason. `set-names-differing-only-in-case-share-a-store-file`.
    ///
    /// **ASCII case-insensitivity is exact here only because [`SetName::new`] admits ASCII alone.**
    /// Over that alphabet full case folding and lower-casing agree. If set names are ever widened,
    /// this is the one place that has to change, and to what a filesystem means by the same name --
    /// canonical normalisation with case folding -- not to [`Keyword::folded`](super::Keyword::folded),
    /// whose compatibility mapping is right for search and would refuse names that do not collide.
    #[must_use]
    pub fn shares_a_file_with(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }

    /// The set name as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SetName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for SetName {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SetName> for String {
    fn from(name: SetName) -> Self {
        name.0
    }
}

/// Which memory set a memory belongs to.
///
/// Deliberately has **no `Default`**. `documents/design/memory-steering.md` requires scope to be
/// chosen explicitly at store time and never inferred, and a `Default` impl would quietly
/// reintroduce exactly the inference that decision rules out.
///
/// Scope also partitions storage — each scope gets its own store, so a query against one set
/// structurally cannot return another scope's memories. See
/// `documents/design/embeddings.md#storage-is-partitioned-by-scope`.
///
/// The two variants name what a set is *for*, not what has happened to it: the distinction is
/// private versus shareable, not stored versus published. `Shared` was called `Project` until
/// 2026-08-03, which wrongly implied a repository — a shared set needs a name and an export
/// destination, and nothing in `documents/design/set-lifecycle.md` requires it to belong to a
/// project at all. A cross-project set of coding lessons is the case that made the old name read
/// wrongly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Belongs to a named set that can be exported to a file and shared.
    Shared(SetName),
    /// Stays on this machine. Never exported, personal or otherwise.
    Local,
}

/// The delimiter in the scalar spelling of a shared scope.
///
/// Safe as a separator by construction rather than by convention: [`SetName`] permits letters,
/// digits, `-`, `_` and `.`, so a set name can never contain a colon and the first one is always
/// the separator. The same spelling is what `scope_key` writes into a store's `meta` row.
const SHARED_PREFIX: &str = "shared:";

impl Scope {
    /// The scalar spelling — `local`, or `shared:<name>`.
    ///
    /// This is what the tool surface publishes, because a `oneOf` between a string and an object is
    /// reduced to `{}` by at least one client, after which an object argument arrives as text and no
    /// shared set can be named at all. See `documents/design/tool-surface.md`.
    #[must_use]
    pub fn as_key(&self) -> String {
        match self {
            Self::Shared(set) => format!("{SHARED_PREFIX}{set}"),
            Self::Local => "local".to_owned(),
        }
    }

    /// The inverse of [`Scope::as_key`].
    ///
    /// Anything without the prefix is [`Scope::Local`], which is what the store's `meta` reader has
    /// always done — a value it cannot interpret is the local set rather than an error, because the
    /// only caller that predates this is an error path.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] if the text after the prefix is not a valid set name.
    pub fn from_key(key: &str) -> Result<Self, ValidationError> {
        match key.strip_prefix(SHARED_PREFIX) {
            Some(name) => Ok(Self::Shared(SetName::new(name)?)),
            None => Ok(Self::Local),
        }
    }
}

/// **Serialising is unchanged; only parsing is widened.**
///
/// `Serialize` stays derived, so a memory file's header and every stored record keep the object
/// form they have always had — this is a change at the *tool* boundary, and the file format is a
/// different boundary that no client transforms.
///
/// Parsing accepts three spellings, deliberately more than is published:
///
/// - `"shared:name"` and `"local"` — the scalar form, which is what the schema now advertises
///   because it is the only shape that survives every client intact.
/// - `{"shared": "name"}` — what the file format writes, what earlier callers send, and what a
///   client that preserves alternations will go on producing.
///
/// Being liberal here is not laxity. A strict server is normally right, because strictness is what
/// keeps clients from drifting into private dialects — but that argument needs influence over the
/// client, and this server has none. Refusing the object form would correct nobody; it would only
/// make the memories unreachable from wherever it arrived.
impl<'de> Deserialize<'de> for Scope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(ScopeVisitor)
    }
}

struct ScopeVisitor;

/// Named in every refusal, because a caller who reaches one has just been told by the schema to
/// send a string and needs to know which strings work.
const SCOPE_FORMS: &str = r#""local", "shared:<name>", or {"shared": "<name>"}"#;

impl<'de> serde::de::Visitor<'de> for ScopeVisitor {
    type Value = Scope;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "a memory set: {SCOPE_FORMS}")
    }

    fn visit_str<E>(self, value: &str) -> Result<Scope, E>
    where
        E: serde::de::Error,
    {
        if value == "local" {
            return Ok(Scope::Local);
        }
        match value.strip_prefix(SHARED_PREFIX) {
            Some(name) => SetName::new(name)
                .map(Scope::Shared)
                .map_err(|error| E::custom(format!("{error} -- a scope is {SCOPE_FORMS}"))),
            None => Err(E::custom(format!(
                "`{value}` does not name a memory set. A scope is {SCOPE_FORMS}"
            ))),
        }
    }

    /// The object form, which is what `Serialize` still produces.
    fn visit_map<M>(self, mut map: M) -> Result<Scope, M::Error>
    where
        M: serde::de::MapAccess<'de>,
    {
        use serde::de::Error;

        let Some(key) = map.next_key::<String>()? else {
            return Err(M::Error::custom(format!(
                "an empty object does not name a memory set. A scope is {SCOPE_FORMS}"
            )));
        };
        let scope = match key.as_str() {
            "shared" => Scope::Shared(map.next_value::<SetName>()?),
            // Externally tagged unit variants may also arrive as `{"local": null}`.
            "local" => {
                map.next_value::<serde::de::IgnoredAny>()?;
                Scope::Local
            }
            other => {
                return Err(M::Error::custom(format!(
                    "`{other}` does not name a memory set. A scope is {SCOPE_FORMS}"
                )));
            }
        };
        while map.next_entry::<String, serde::de::IgnoredAny>()?.is_some() {}
        Ok(scope)
    }
}

impl Scope {
    /// The set this memory is exported to, or `None` if it never leaves the machine.
    ///
    /// This is where "local memories are never exported" lives. Deriving export eligibility from
    /// the scope itself means there is no separate rule for a call site to forget.
    #[must_use]
    pub fn exports_to(&self) -> Option<&SetName> {
        match self {
            Self::Shared(name) => Some(name),
            Self::Local => None,
        }
    }
}
