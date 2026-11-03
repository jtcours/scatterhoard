// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Labelled edges between memories.

use serde::{Deserialize, Serialize};

use super::{Keyword, MemoryId, Scope, SetId, SetName};

/// The canonical set of relationships a memory can have to another memory.
///
/// Defining the set here rather than in the conventions means the set traversal understands is the
/// one written down: a label outside it parses, but only as [`Unrecognised`](LinkLabel::Unrecognised),
/// which nothing walks.
///
/// Every variant reads **"this memory … that one"**, because every link is written outbound from
/// the memory being stored. That is not a style choice: it was measured across a corpus of real
/// memories, where 29 of 29 edges pointed outbound from the newer memory. Two consequences —
/// a label needs a reverse only where the new memory's side is genuinely free (causation, hence
/// [`Causes`](LinkLabel::Causes)/[`ResultsFrom`](LinkLabel::ResultsFrom)), and the served
/// vocabulary never has to explain direction, which is what pays for its context cost.
///
/// See `documents/design/memory-data-model.md` for the vocabulary's derivation and
/// `corpus/link-label-findings.md` for the evidence.
///
/// `PRECEDES` and `FOLLOWS` are deliberately absent — deferred, not rejected, pending a corpus that
/// can test them fairly (see `EARLY-VALIDATION.md`). Adding a label follows ordinary semantic
/// versioning, and whoever adds one decides whether older builds carrying it opaquely is safe —
/// `documents/design/memory-data-model.md`, *unknown labels are carried opaquely*.
///
/// **Not `Copy`**, because [`Unrecognised`](LinkLabel::Unrecognised) holds text. Serialised by
/// [`LinkLabel::name`] rather than by a derive, so the one exhaustive match there is the whole of
/// the vocabulary's spelling.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
// Deliberately not `#[non_exhaustive]`: that would force downstream matches to carry a catch-all
// arm, and adding a link label is exactly the change that should force every traversal site to be
// reconsidered. See the correctness-by-construction section of STANDARDS.md.
pub enum LinkLabel {
    /// …is associated with it; the fallback when nothing more specific applies.
    RelatedTo,
    /// …mentions or cites it.
    RefersTo,
    /// …is the cause of it.
    Causes,
    /// …is an effect of it.
    ///
    /// Named for the active reading rather than `CAUSED_BY`: the passive form was the only label in
    /// the source vocabulary with a near-identical active sibling, and was observed to cause
    /// mislabels.
    ResultsFrom,
    /// …was decided because of it.
    ///
    /// The commonest relation by a wide margin in a design corpus. Distinct from
    /// [`ResultsFrom`](LinkLabel::ResultsFrom), which is physical causation: this one is reasoning.
    MotivatedBy,
    /// …is evidence for it.
    ///
    /// The counterpart to [`Contradicts`](LinkLabel::Contradicts); a measurement corroborating a
    /// decision made on reasoning alone is the shape this almost always takes.
    Supports,
    /// …conflicts with it, with both still standing.
    Contradicts,
    /// …replaces it, which is kept as history.
    Supersedes,
    /// …is replaced by it.
    ///
    /// Exists for a scope reason rather than an authoring one: when a project memory supersedes a
    /// local one, writing [`Supersedes`](LinkLabel::Supersedes) on the project memory is both
    /// impossible and a trust inversion, so the lower-authority side records its own obsolescence.
    /// See `documents/design/cross-set-references.md`.
    SupersededBy,
    /// …answers it, an open question.
    Resolves,
    /// …needs it to hold.
    ///
    /// A dependency, **not** a reason — if the target is why this memory was decided, the label is
    /// [`MotivatedBy`](LinkLabel::MotivatedBy). Renamed from `REQUIRES`, which attracted that
    /// confusion badly enough to be worth renaming away from.
    DependsOn,
    /// …is a sanctioned carve-out from that rule.
    ExceptionTo,
    /// …breaks that rule, without sanction.
    ///
    /// The same shape as [`ExceptionTo`](LinkLabel::ExceptionTo), separated by whether the
    /// deviation is permitted. Without it, defects get recorded as approved exceptions, which is
    /// worse than recording nothing.
    Violates,
    /// …is a specific case of that general pattern.
    InstanceOf,
    /// A label this build does not know, carried exactly as it was read.
    ///
    /// **Carried, shown, never walked, and restated only by copying** — decided 2026-09-25, because
    /// refusing a file over one would make every label added after publication break every older
    /// reader, fixed on the day the first build ships. It arrives from a file written by a newer
    /// build, and a write may carry one only by restating a label and target the memory it
    /// supersedes or amends already holds; see `unknown_label_refusal` in `src/mcp.rs`.
    ///
    /// A variant of its own rather than a string field beside the known labels, so a check
    /// matching on specific variants — supersession, the disputing labels — cannot reach it by
    /// accident, and one written as a negation has to name it.
    Unrecognised(UnrecognisedLabel),
}

/// The spelling of a link label this build does not recognise.
///
/// **No public constructor.** The only way to get one is [`LinkLabel::from_name`] with a name
/// outside [`LinkLabel::ALL`], so an `Unrecognised` label never holds a known spelling: a newer
/// build cannot smuggle `supersedes` past the checks that match on it by writing it opaquely.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnrecognisedLabel(Box<str>);

impl UnrecognisedLabel {
    /// The label exactly as it was read.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl LinkLabel {
    /// Every label, in a stable order.
    pub const ALL: [Self; 14] = [
        Self::RelatedTo,
        Self::RefersTo,
        Self::Causes,
        Self::ResultsFrom,
        Self::MotivatedBy,
        Self::Supports,
        Self::Contradicts,
        Self::Supersedes,
        Self::SupersededBy,
        Self::Resolves,
        Self::DependsOn,
        Self::ExceptionTo,
        Self::Violates,
        Self::InstanceOf,
    ];

    /// This label's position in [`LinkLabel::ALL`], or `None` for one this build does not know.
    ///
    /// The exhaustive match is the point: adding a variant fails to compile here, and the
    /// accompanying test then catches `ALL` if it was not updated to match.
    #[must_use]
    pub fn ordinal(&self) -> Option<usize> {
        Some(match self {
            Self::RelatedTo => 0,
            Self::RefersTo => 1,
            Self::Causes => 2,
            Self::ResultsFrom => 3,
            Self::MotivatedBy => 4,
            Self::Supports => 5,
            Self::Contradicts => 6,
            Self::Supersedes => 7,
            Self::SupersededBy => 8,
            Self::Resolves => 9,
            Self::DependsOn => 10,
            Self::ExceptionTo => 11,
            Self::Violates => 12,
            Self::InstanceOf => 13,
            Self::Unrecognised(_) => return None,
        })
    }

    /// The label as it is written in a memory file and on the wire.
    ///
    /// These names are format-visible: they appear in every exported memory carrying a link, so
    /// changing one is a format migration rather than an edit. An unrecognised label is written
    /// back exactly as it was read, which is what keeps a file's checksums agreeing across a build
    /// that does not know it.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::RelatedTo => "related_to",
            Self::RefersTo => "refers_to",
            Self::Causes => "causes",
            Self::ResultsFrom => "results_from",
            Self::MotivatedBy => "motivated_by",
            Self::Supports => "supports",
            Self::Contradicts => "contradicts",
            Self::Supersedes => "supersedes",
            Self::SupersededBy => "superseded_by",
            Self::Resolves => "resolves",
            Self::DependsOn => "depends_on",
            Self::ExceptionTo => "exception_to",
            Self::Violates => "violates",
            Self::InstanceOf => "instance_of",
            Self::Unrecognised(label) => label.as_str(),
        }
    }

    /// The label a name spells: a known one where the name is in the vocabulary, and
    /// [`Unrecognised`](LinkLabel::Unrecognised) otherwise.
    ///
    /// **Never refuses.** Whether an unknown label may be *written* is a question about the call
    /// carrying it, not about the spelling, so it is asked where the call is — parsing one from a
    /// file is always allowed.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        Self::ALL
            .iter()
            .find(|label| label.name() == name)
            .cloned()
            .unwrap_or_else(|| Self::Unrecognised(UnrecognisedLabel(name.into())))
    }

    /// Whether this build knows the label.
    #[must_use]
    pub fn is_recognised(&self) -> bool {
        !matches!(self, Self::Unrecognised(_))
    }
}

impl Serialize for LinkLabel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for LinkLabel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = std::borrow::Cow::<'de, str>::deserialize(deserializer)?;
        Ok(Self::from_name(&name))
    }
}

/// What a link points at.
///
/// Two variants rather than an id with optional extras. A within-set link is the common case and
/// stays compact — repeating the set name on every one of them would work against the small merge
/// diffs the file format exists to produce. The supplementary keywords hang off the variant that
/// can use them: a same-set reference has nothing to re-resolve against if its target is forgotten,
/// so keywords there would be a field that is meaningless half the time.
///
/// See `documents/design/cross-set-references.md`.
///
/// **No link can reach the personal `local` set from outside it** -- neither by name, since `local`
/// is not a [`SetName`] and so no `set` spells it, nor by `set_id`, since resolution matches an
/// identity only among the sets a name could reach (`holder_of` in `src/store.rs`). Two behaviours
/// rest on this: a local memory cannot be disputed, and a claim from a local set is not reported,
/// because `local` names a different set on every machine. The identity half was missing from
/// 2026-08-31, when `local` gained an identity, to 2026-09-28, and nothing failed -- which is why
/// both halves are pinned by tests in `tests/cross_store.rs` and by mutations, rather than left to
/// follow from the types.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LinkTarget {
    /// A memory in the same set, identified by id alone.
    SameSet(MemoryId),
    /// A memory in another set.
    ///
    /// The id plus the set name is the canonical reference; `keywords` are supplementary context
    /// carried so that a dangling reference has something to be re-found by, and are never
    /// sufficient to resolve one on their own.
    CrossSet {
        /// The set the target lives in.
        set: SetName,
        /// The target's id.
        id: MemoryId,
        /// Keywords from the target, to help re-establish a reference that has gone dangling.
        keywords: Vec<Keyword>,
        /// The target set's own identity, when the authoring machine knew it.
        ///
        /// **Optional permanently, not just during the migration.** A link may name a set this
        /// machine has never held — a dangling target is documented as allowed and expected — so
        /// at authoring time there is often no identity to write, and there never will be. The
        /// name therefore keeps working on its own, and a link carrying only a name resolves
        /// exactly as well as it did before identities existed.
        ///
        /// **Skipped when absent**, which is what keeps this change off the checksum of every
        /// record that already holds a cross-set link: their bytes are unchanged, so they neither
        /// reconcile as modified nor need rewriting.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        set_id: Option<SetId>,
    },
}

impl LinkTarget {
    /// The target memory's id, whichever set it lives in.
    #[must_use]
    pub fn id(&self) -> MemoryId {
        match self {
            Self::SameSet(id) => *id,
            Self::CrossSet { id, .. } => *id,
        }
    }

    /// The set the target lives in, or `None` when it is the holding memory's own set.
    #[must_use]
    /// Whether this target really is in a different set from the one holding the link: `scope`, whose
    /// identity is `holder` where it has one.
    ///
    /// **Not the same question as "does it carry a set name".** A target may name the very set that
    /// holds the memory pointing at it, which is a same-set reference written the long way. Asking
    /// `set().is_some()` treats those as foreign, which is what made
    /// `a-same-set-link-spelled-with-its-set-name-is-inert` both inert at resolution and
    /// over-refused at the write.
    ///
    /// **Identity decides where both sides have one, and the name only where either does not**,
    /// since 2026-09-14 (`a-link-naming-its-own-set-by-an-old-name-is-inert`). A set's identity is
    /// minted once and never changes, where its name can be renamed or shared, and resolution already
    /// finds a set by identity -- so a target naming this set by an old name is this set, and one
    /// naming this set's current name with another set's identity is not.
    /// `documents/design/cross-set-references.md`, *which of the two a stored link is*.
    ///
    /// Without identities, a named target is always foreign to [`Scope::Local`], which has no name
    /// for anything to match: no `target_set` value addresses the local set, by construction.
    pub fn is_foreign_to(&self, scope: &Scope, holder: Option<SetId>) -> bool {
        match self {
            Self::SameSet(_) => false,
            Self::CrossSet {
                set_id: Some(target),
                ..
            } if holder.is_some() => holder != Some(*target),
            Self::CrossSet { set, .. } => scope.exports_to() != Some(set),
        }
    }

    pub fn set(&self) -> Option<&SetName> {
        match self {
            Self::SameSet(_) => None,
            Self::CrossSet { set, .. } => Some(set),
        }
    }
}

/// A labelled edge from the memory holding it to another memory.
///
/// The edge is stored on the source memory, so a link is one memory's own data and moves with it
/// through export and merge. A target is not guaranteed to resolve locally — it may name a set that
/// is not loaded, or a memory that was never merged — so traversal has to tolerate a dangling edge
/// rather than assume one cannot exist.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Link {
    /// What this memory claims about the target.
    pub label: LinkLabel,
    /// The memory pointed at.
    pub target: LinkTarget,
}

/// **Serialising is unchanged; parsing also accepts a flattened target.**
///
/// [`LinkTarget`] is an untagged enum — a bare string for a same-set id, an object for a cross-set
/// one — and at least one client reduces a nested alternation to `{}`, after which the object form
/// arrives as text and cannot be parsed at all. `documents/design/tool-surface.md`.
///
/// `Scope`'s fix does not transfer. A `"set:id"` string would drop `keywords`, which exist so a
/// dangling reference has something to be re-found by, so that spelling would lose exactly the
/// field that makes a cross-set link recoverable. Instead the alternation moves out into siblings
/// the schema can publish as scalars:
///
/// ```json
/// {"label": "related_to", "target": "<id>", "target_set": "other", "target_keywords": ["alpha"]}
/// ```
///
/// Objects with named properties survive every client observed; only the alternation inside them
/// does not. **`Serialize` stays derived**, so a memory file still holds the nested form and no
/// exported file changes — this is a widening at the tool boundary, not a format change.
///
/// `target_set` without `target` is constructible here where the nested form made it impossible.
/// That is accepted rather than solved, because the schema that would have forbidden it is the one
/// being discarded; the refusal moves into this parser instead, which is the only place left that
/// can make it.
impl<'de> Deserialize<'de> for Link {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            label: LinkLabel,
            #[serde(default)]
            target: Option<LinkTarget>,
            // Text rather than a `SetName`, so that a refusal can say which field it is about: a
            // set name is validated the same way wherever it appears, and its own message cannot.
            #[serde(default)]
            target_set: Option<String>,
            #[serde(default)]
            target_keywords: Option<Vec<Keyword>>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let Some(target) = wire.target else {
            return Err(D::Error::custom(
                "a link needs a `target`: the id of the memory it points at, optionally with \
                 `target_set` naming another set",
            ));
        };

        // No siblings supplied: the target is already whatever it parsed as, nested form included.
        if wire.target_set.is_none() && wire.target_keywords.is_none() {
            return Ok(Self {
                label: wire.label,
                target,
            });
        }

        let LinkTarget::SameSet(id) = target else {
            return Err(D::Error::custom(
                "a link gives its target either as a nested object or as `target` with \
                 `target_set` beside it, not both",
            ));
        };
        let Some(set) = wire.target_set else {
            return Err(D::Error::custom(
                "`target_keywords` describes a target in another set, so it needs `target_set` \
                 naming which one",
            ));
        };
        let set = SetName::new(set.as_str())
            .map_err(|error| D::Error::custom(target_set_refusal(&set, &error.to_string())))?;

        Ok(Self {
            label: wire.label,
            target: LinkTarget::CrossSet {
                set,
                id,
                keywords: wire.target_keywords.unwrap_or_default(),
                // Never from the caller. A set's identity is a fact the server holds and an LLM
                // writing a link does not, so it is filled in at the write if this machine holds
                // the target set and left absent otherwise -- see `Server::identify_link_targets`.
                set_id: None,
            },
        })
    }
}

/// Why a `target_set` was refused, naming the field and, where it can, the spelling to send.
///
/// **The two fields spell one set two ways, and both are right.** `scope` chooses between `local`
/// and a shared set, so it needs the `shared:` prefix to tell them apart; `target_set` only ever
/// names a shared set, so the prefix would be noise there. Sending the `scope` spelling is the
/// mistake the two invite, and the bare validation message -- *a set name must not contain ':'* --
/// names neither field nor the fix. `the-set-name-refusal-does-not-say-how-the-other-field-spells-it`.
fn target_set_refusal(text: &str, reason: &str) -> String {
    match text.strip_prefix("shared:") {
        Some(bare) if SetName::new(bare).is_ok() => format!(
            "`target_set` takes a shared set's bare name: send `{bare}`, not `{text}`. The \
             `shared:` prefix belongs to `scope`, where it tells a shared set from `local`; \
             `target_set` only ever names a shared set, so it takes the name alone."
        ),
        _ => format!("`target_set` {text:?} is not a set name: {reason}"),
    }
}
