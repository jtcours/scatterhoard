// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The per-scope memory store.
//!
//! **One store per scope, one SQLite file per store** (`documents/design/embeddings.md`). A query
//! against one project's store structurally cannot return another scope's memories, because the
//! other scope's memories are in a different file — there is no `WHERE` clause to forget.
//!
//! # Edges live in their own table, and that table *is* the reverse index
//!
//! `documents/design/recall-mechanisms.md` requires that every stored edge be discoverable from
//! both endpoints, and leaves the mechanism open. A links table indexed on `target` as well as
//! `source` satisfies it without a second structure to build, maintain, and recompute: there is no
//! derived copy that can fall out of step, because there is no copy. The reverse direction is a
//! different index over the same rows.
//!
//! That also settles when the reverse becomes available — immediately, as part of the write that
//! created the edge, since it *is* the write. The failure the design warned about, where a
//! supersession recorded mid-session stays invisible until some later reload, cannot occur.
//!
//! # A project store is disposable; the local store is not
//!
//! Everything in a project's store can be rebuilt from its memory file, so the schema here carries
//! no migration burden — a change means dropping the file and re-importing. The local store is the
//! only copy of local memories, which is why backup and restore exist
//! (`documents/design/file-format.md`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

use crate::embedder::{Embedded, Embedder, ModelIdentity};
use crate::embedding::{Vector, VectorSet, combined_centroid, embedding_input};
use crate::memory::{
    Attribution, Author, Confidence, Keyword, Link, LinkLabel, LinkTarget, Memory, MemoryId,
    MemoryText, Origin, Scope, SetId, SetName, ValidationError, serde_name,
};
use crate::memory_file::{MemorySet, Record, Tombstone};
use crate::traversal::{self, Adjacency, Walk};

/// Guards against a supersession chain that is long enough to suggest corrupt data rather than a
/// genuine history. Resolution also detects cycles outright; this is the belt to that's braces.
const MAX_SUPERSESSION_DEPTH: usize = 64;

/// The reserved keyword marking a memory as an identity record.
///
/// `keyword-standards` reserves the `hoard-` prefix for Scatterhoard itself, and
/// `hoard-scope-topics` is the precedent: set-level information stored as an ordinary memory under
/// a reserved keyword, so it merges, supersedes and travels like everything else.
///
/// **The name lives in the record's `content`, not in a second keyword.** Keyword lookup folds
/// case, which would make `JDoe` and `jdoe` one identity — unacceptable for something whose whole
/// job is to tell two people apart.
pub const AUTHOR_KEYWORD: &str = "hoard-author";

/// The reserved keyword marking a set's charter: the memory saying what the set is for.
///
/// Until 2026-09-14 no code read it -- routing matches against charters by recall, which needs no
/// constant. What reads it now is [`Store::declared_languages`].
pub const CHARTER_KEYWORD: &str = "hoard-scope-topics";

/// The prefix of the keywords a charter declares its languages with: `hoard-lang-en`,
/// `hoard-lang-zh-hans`. `documents/design/memory-data-model.md`, *a set declares its languages on
/// its charter*.
pub const LANGUAGE_KEYWORD_PREFIX: &str = "hoard-lang-";

/// The prefix `keyword-standards` leaves to a project's own tooling.
pub const PROJECT_KEYWORD_PREFIX: &str = "hoard-proj-";

/// Every control mark this version of Scatterhoard knows, spelled in folded form.
///
/// **The list the authoring write path refuses by** (`reserved-keyword-block-list`), and the one
/// `tests/conventions.rs` holds the served catalog to, so that no convention tells a session to
/// write a mark the check would refuse. A mark added here is a mark sessions may write; one removed
/// is only no longer *introduced*, because a term a set already holds stays writable in it -- see
/// [`Store::insert`].
pub const CONTROL_MARKS: &[&str] = &[
    AUTHOR_KEYWORD,
    CHARTER_KEYWORD,
    SIGNPOST_MARK,
    COLLECT_MARK,
    INVESTIGATION_MARK,
    FIT_HUB_MARK,
    FIT_SENTENCES_MARK,
    FIT_MODEL_MARK,
];

/// A set's own model-fit material, as [`Store::fit_material`] reads it.
#[derive(Debug, Default)]
pub struct FitMaterial {
    /// The sentence sets the set's hub lists, keyed by language and script --
    /// [`crate::language_fit::language_and_script`], written `egy-latn` or `sm`.
    pub groups: BTreeMap<String, Vec<Memory>>,
    /// Live sentence sets no hub lists, which are scored for nothing.
    pub orphans: Vec<MemoryId>,
    /// The models the set recommends: live `hoard-fit-model` memories linking to its hub.
    pub recommendations: Vec<Recommendation>,
    /// The live hubs: one, ordinarily, and none until a set first carries material of its own.
    pub hubs: Vec<MemoryId>,
}

/// A model a set recommends, as status shows it.
///
/// **The summary line and not a parsed model name**: a recommendation is prose, so its opening has
/// to name the model, as every mark's opening has to say what the memory is for.
#[derive(Debug, Clone)]
pub struct Recommendation {
    /// The recommendation's id.
    pub id: MemoryId,
    /// Its one-line summary, derived from its opening.
    pub summary: String,
    /// The languages it was measured on, as tagged; empty where it concerns the whole set.
    pub tags: Vec<String>,
}

/// A set's model-fit hub: the memory that lists the set's own paraphrase test sentences and that
/// its model recommendations link to. One per set, separate from the charter.
/// `documents/design/memory-data-model.md`, *the model-fit report*.
///
/// **Three fixed marks rather than a `hoard-fit-` family**, so a mistyped one is refused like any
/// other unknown mark.
pub const FIT_HUB_MARK: &str = "hoard-fit-hub";

/// One paraphrase test triplet -- an anchor, a paraphrase and an unrelated sentence -- carried by a
/// memory the hub links to, tagged with its `hoard-lang-` language. Marked although the hub's links
/// identify it, so a reviewer can tell what one is on its own and status can report one no hub
/// lists.
pub const FIT_SENTENCES_MARK: &str = "hoard-fit-sentences";

/// A model recommended for this set, linking in to its hub, tagged with the languages it was
/// measured on or untagged where it concerns the whole set.
pub const FIT_MODEL_MARK: &str = "hoard-fit-model";

/// *Recall it before reasoning about its subject* -- the one watch mark that asks for no write, and
/// so the one a read-only set still pulls. See [`LoadMode::watch_marks`].
pub const SIGNPOST_MARK: &str = "hoard-signpost";

/// *Add findings that fit it.*
pub const COLLECT_MARK: &str = "hoard-collect";

/// *Add evidence for or against its hypothesis.*
pub const INVESTIGATION_MARK: &str = "hoard-investigation";

/// The prefixes whose every member is a control mark. **A family rather than a list** because what
/// follows the prefix is not this version's to enumerate: a language is whatever the user's chosen
/// model can represent, and a project's marks are the project's.
pub const CONTROL_MARK_FAMILIES: &[&str] = &[LANGUAGE_KEYWORD_PREFIX, PROJECT_KEYWORD_PREFIX];

/// What recall needs to know about a memory's validity before it ranks anything.
///
/// Supersession is a validity relation rather than a relevance one, so it is resolved before
/// ranking rather than as part of it — see `documents/design/recall-mechanisms.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Nothing in this store supersedes the memory.
    Current,

    /// The memory has been replaced.
    Superseded {
        /// The ends of the supersession chain, as far as this store can follow them. More than one
        /// when a memory was split into several, which the convention for correcting a partly-stale
        /// memory produces.
        tips: Vec<MemoryId>,
        /// True when the chain leaves this store — the memory is known to be superseded but the
        /// replacement is not available here. Reported rather than passed off as current.
        unresolved: bool,
        /// True when a chain ends on a replacement this store **forgot**: it holds the tombstone,
        /// so the replacement is not elsewhere but withdrawn. Kept apart from `unresolved` because
        /// the two send a reader in opposite directions -- load another set, or write the claim
        /// anew -- and only this one is answerable from the store in hand.
        /// `a-forgotten-replacement-reads-as-superseded-beyond-this-set`.
        forgotten: bool,
    },

    /// A supersession cycle. A data error rather than a history, so it is reported instead of
    /// resolved: write refuses to create one, but a hand-edited merge can still deliver one.
    Cycle,
}

/// How a supersession chain ended, with every reason it could not be followed kept apart.
///
/// [`Resolution`] folds *elsewhere* and *absent* into `unresolved`, which is what a single store's
/// reader needs. [`standing_within`] needs them apart: it can re-resolve a concession to another
/// set, and must keep an absent id out of reach rather than forget why the chain stopped -- which it
/// did until 2026-09-30, so a memory whose only replacement was forgotten looked up as current.
enum Walked {
    Current,
    Superseded {
        tips: Vec<MemoryId>,
        /// A `SUPERSEDED_BY` named a replacement in another set.
        elsewhere: bool,
        /// The chain ended on a same-set id this store has no record of at all.
        absent: bool,
        /// The chain ended on a same-set id this store holds only a tombstone for.
        forgotten: bool,
    },
    Cycle,
}

/// What a semantic recall asks for.
///
/// A struct rather than three arguments because the three are not peers: `query` is the search,
/// while `window` and `already_known` are the caller's budget and the caller's state. Bundling them
/// also keeps the decision about each where the stake is — see
/// `documents/design/decision-placement.md`.
#[derive(Debug, Clone)]
pub struct Question<'a> {
    /// What the caller is looking for, in their own words.
    pub query: &'a str,
    /// How many memories to return, best first.
    ///
    /// **A count, so a smaller limit returns the start of what a larger one would.** It was a window
    /// a largest-gap cut looked inside until 2026-10-02, when that cut was found to be choosing among
    /// noise; the cut survives only as an instrumentation stage, [`Stages::gap_cut`], off by default.
    pub limit: usize,
    /// Memories the caller already holds.
    ///
    /// Removed from the **results** and from nothing else. They still seed the walk and are still
    /// traversed through: a memory the caller holds and is querying around is the best available
    /// seed — known-relevant and already validated — and dropping it would disconnect whatever lies
    /// beyond it. See `documents/design/recall-mechanisms.md`.
    pub already_known: &'a [MemoryId],
    /// Who is asking, which is what decides whether the query log records this at all.
    pub asking: Asking,
    /// Whether the caller has waived this set's non-match floor.
    ///
    /// **A caller's choice, so it lives here and not in [`Stages`]**, which holds the scorer's own
    /// settled decisions. A floor is a level calibrated from past traffic, and the session asking
    /// is better placed than the store to know whether this particular question deserves an answer
    /// anyway — so when a floor silences everything, recall says so and the caller may ask again.
    pub ignore_floor: bool,
}

/// Whether a question is a session's own, and therefore evidence about what this store is asked.
///
/// **The default is [`Asking::Silently`], and the asymmetry is deliberate.** The probe suite scores
/// several hundred deliberately off-domain queries through this same path in a single pass, and a
/// log that recorded every recall would fill with the measurement rather than with the traffic —
/// leaving a floor calibrated partly against questions nobody asked. `tests/probes.rs` works on
/// *copies* of the live stores today, so nothing would reach the originals, but that is a property
/// of how those probes are written rather than of the log: the copy inherits the log, and
/// `tests/recall.rs` and the eval harness search stores of their own. Only a caller that knows a
/// session is at the other end can say a query is traffic, and that is the MCP tool surface.
///
/// The cost is the other way round: a future shipping path that forgets to say so logs nothing, and
/// nothing reports an under-collected log. `a_recall_through_the_server_is_logged` in
/// `tests/query_log.rs` is the guard on the one path that exists today.
/// One session, for as long as a server process lasts.
///
/// **It exists to bound the `used` label**, which is what keeps that label meaning anything: a
/// memory returned long ago by a query it did not answer would otherwise be labelled by any later
/// use of it, and a label that saturates calibrates a floor down toward the constant it replaced.
/// The same bound the transcript-based simulation used — `corpus/label_queries.py`, *mentioned in a
/// later call in the same session*.
///
/// **A session begins when the conventions are loaded, and deliberately not when the server starts.**
/// A process was tried first and is measurably wrong: on 2026-09-11 the servers on this machine had
/// been running two hours before the current session's transcript existed, and two other sessions
/// had written to their own transcripts in between. `hoard_conventions` is what a session does first
/// -- the catalog's own description says to call it once at the start of a session, and this
/// project's `AGENTS.md` requires it before any visible text -- so the server rolls this identity
/// when the first page is asked for.
///
/// **It answers to a convention rather than to the protocol**, which is its one weakness. A session
/// that never loads the conventions inherits the previous one's identity, which is the looser bound
/// a process gave and so a degradation rather than a break; a session that reloads them midway
/// splits in two, which loses labels rather than inventing them. Both errors leave the floor lower
/// or later than it should be, which is the over-recalling end and the safe one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionId(uuid::Uuid);

impl SessionId {
    /// A fresh identity for one server process.
    #[must_use]
    pub fn generate() -> Self {
        Self(uuid::Uuid::now_v7())
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Asking {
    /// Instrumentation: a probe, a sweep, a test. Nothing is recorded.
    #[default]
    Silently,
    /// A session's own question, recorded in each searched store's query log.
    InASession {
        /// Whether the caller named one set as its `scope`.
        ///
        /// **The one label a floor cannot suppress**, because it is an argument of the call rather
        /// than a consequence of the answer: a scoped query scoring below the floor is a
        /// contradiction sitting in the log, and it is what lowers a floor that was set too high.
        /// `documents/design/recall-mechanisms.md`.
        scoped: bool,
        /// When it was asked, supplied by the caller like every other time this store records.
        ///
        /// The rowid already orders the log, which is what walking it forward needs. This answers
        /// what the rowid cannot: how long a log has been running, which is the question anything
        /// that eventually bounds it has to ask.
        at: Timestamp,
        /// Who is asking, so that a memory used later in this same session labels this query and a
        /// use in some other session does not. See [`SessionId`].
        session: SessionId,
    },
}

impl<'a> Question<'a> {
    /// A query, and how many memories to return.
    #[must_use]
    pub const fn new(query: &'a str, limit: usize) -> Self {
        Self {
            query,
            limit,
            already_known: &[],
            asking: Asking::Silently,
            ignore_floor: false,
        }
    }

    /// Name what the caller already has in context.
    #[must_use]
    pub const fn already_holding(mut self, ids: &'a [MemoryId]) -> Self {
        self.already_known = ids;
        self
    }

    /// Ask without the floor, after being told it silenced everything.
    #[must_use]
    pub const fn without_the_floor(mut self) -> Self {
        self.ignore_floor = true;
        self
    }

    /// Say that a session is asking, so each searched store records the query. See [`Asking`].
    #[must_use]
    pub const fn in_a_session(mut self, scoped: bool, at: Timestamp, session: SessionId) -> Self {
        self.asking = Asking::InASession {
            scoped,
            at,
            session,
        };
        self
    }
}

/// Where in the labelled window the level is taken, before the margin.
///
/// **Not the minimum, and that is the whole of the redesign.** A minimum over a growing set can only
/// fall, so recalibration could never raise a floor and one mislabelled query set the level for
/// good. A percentile deliberately silences a few labelled queries, which is the tolerance that makes
/// a noisy-but-frequent signal like `scope` usable at all.
///
/// **Swept over both stores 2026-09-12** against the 601 off-domain probes: moving from the minimum
/// to the 5th percentile takes probe silencing from 39.8% to 74.4% on `scatterhoard` and 48.4% to
/// 83.2% on `coding-lessons`, at a cost of one labelled query in 65 and three in 43. Past 0.05 the
/// cost climbs faster than the benefit on both.
const FLOOR_PERCENTILE: f64 = 0.05;

/// How many labelled queries must arrive before a calibration will set anything.
///
/// Counted **since the last calibration**, so each level is read off evidence the previous one did
/// not consume. Swept 2026-09-12: recalibrating every 20 new labels loses 6.2% of the labelled
/// queries that follow, where every 40 loses 15.6%.
pub const LABELS_PER_CALIBRATION: usize = 20;

/// How many rows of each kind -- labelled and unlabelled -- the query log keeps.
///
/// The bound is what lets a level rise: read off everything a store was ever asked, a percentile
/// drifts toward the whole history rather than tracking a corpus that grows underneath it. The
/// simulation could not settle the size, because both stores hold fewer labelled queries than this
/// and their corpora were frozen while the real ones grew -- so 100 is chosen as *a few weeks of
/// traffic* at the measured rate of 19-20 labelled queries a week, not measured.
const QUERY_LOG_KEPT: usize = 100;

/// How far below the labelled percentile a calibrated floor sits.
///
/// **Measured, not chosen.** A margin of 0.05 below the minimum loses nothing on either of the two
/// stores simulated, from thirty labelled queries onward. A floor *at* the minimum would silence the
/// very query that proved the set could answer, the next time the same question is asked.
const FLOOR_MARGIN: f64 = 0.05;

/// What a calibration did, and on what evidence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Calibration {
    /// A floor was set, from this many labelled queries.
    Set {
        /// The level, a margin below the 5th percentile of the labelled scores.
        floor: f64,
        /// How many queries in the log carried a label.
        labelled: usize,
    },
    /// Too few labelled queries had arrived, so the floor was placed from the set's own memories:
    /// each current claim's index line asked against the rest. See [`Store::calibrate`].
    FromItsOwnMemories {
        /// The level, a margin below the 5th percentile of what those questions scored.
        floor: f64,
        /// How many current claims were asked.
        memories: usize,
    },
    /// The log does not yet hold enough labelled queries, so no floor was set and any existing one
    /// is left alone.
    NotEnoughEvidence {
        /// How many it does hold.
        labelled: usize,
    },
}

/// What a store's floor was read from, recorded beside the level.
///
/// **Reported because the two are not equal evidence.** Labels are what sessions did with real
/// answers; a floor from the set's own memories is what the set gives questions shaped like its own
/// index lines, which matched the label-derived floor within 0.012 on the two sets measured but is
/// an inference rather than an observation. `calibration-from-self-queries`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorSource {
    /// Read off labelled queries in the log.
    Labels,
    /// Placed from the set's own current memories, because too few labels had arrived.
    ItsOwnMemories,
}

impl FloorSource {
    /// The spelling stored in `meta` and reported by `hoard_sets status`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Labels => "labels",
            Self::ItsOwnMemories => "its-own-memories",
        }
    }
}

/// One recall, as the store that answered it recorded it. See [`Store::query_log`].
///
/// **No query text**, by construction rather than by redaction: it is never written, so there is
/// nothing here to leave out. What a calibration needs is the score and the labels.
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedQuery {
    /// When the session asked it.
    pub asked_at: Timestamp,
    /// The model whose scores these are. A floor calibrated under one model means nothing under
    /// another.
    pub model: ModelIdentity,
    /// Whether the caller named one set as its `scope`. See [`Asking::InASession`].
    pub scoped: bool,
    /// The best **uncentred** content similarity in this store, before any floor was applied.
    pub raw_top: f32,
    /// The best score the ranking saw, which is centred on the union of the searched stores.
    pub centred_top: f32,
    /// Whether a floor removed every hit this store had.
    pub silenced: bool,
    /// What the session received from this store, in the order it received it.
    pub returned: Vec<MemoryId>,
    /// Whether the session that asked went on to use one of those memories — looking it up, linking
    /// to it, or passing it back as already held. **This is the label that says the set could
    /// answer**, and the one a floor destroys: once a floor silences a query nothing comes back, so
    /// nothing can be used afterwards. See [`Store::note_used`].
    pub used: bool,
}

/// One store's links, read once and split by what each is for.
#[derive(Debug, Default)]
struct Edges {
    /// Endpoint pairs the walk may cross, with direction discarded, each carrying the set its
    /// target names — `None` for a same-set target, which is the store that produced the edge.
    ///
    /// **The set travels with the edge because the endpoint has to be located.** An id alone cannot
    /// say which store the far end is in once two sets may share one, and guessing by looking the id
    /// up across every store would connect an edge to a memory it never named.
    /// Source, target, the target's set name, and that set's identity where the link states one.
    /// The identity travels with the name for the reason `holder_of` gives: a name can be stale and
    /// a name can be shared, so it cannot say by itself which set an edge reaches.
    walkable: Vec<(MemoryId, MemoryId, Option<SetName>, Option<SetId>)>,
    /// Memories some other memory claims to have replaced.
    replaced: Vec<MemoryId>,
}

/// A memory recall returned, after validity has been resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Recall {
    /// The memories found, ranked.
    pub hits: Vec<Recalled>,
    /// Whether a floor removed every hit there was.
    ///
    /// **Silence and *nothing matched* are the same empty result**, and only the caller can tell
    /// whether the question deserved an answer — so this is reported rather than left to be
    /// inferred, and [`Question::without_the_floor`] is how the caller acts on it. False when
    /// nothing was silenced, including when some hits survived: a partial cut is the floor working,
    /// not the floor being wrong.
    pub silenced_by_floor: bool,
    /// The sets every hit came from, when some searched set has a floor and none of those did.
    ///
    /// **Empty unless that holds.** A set with no floor has nothing that separates a non-match
    /// from an answer, and when every set with one has declined a question, what remains is often
    /// that set's everyday background — the case `an-unanswerable-query-returns-confident-hits`
    /// records. Reported rather than withheld, because it may still be a real answer and the
    /// caller can re-ask with a `scope`. Empty when the caller waived the floors, since then
    /// nothing was declined. `note-when-only-uncalibrated-sets-answer`.
    pub answered_only_without_a_floor: Vec<Scope>,

    /// Whether the semantic path ran **at all**.
    ///
    /// False when no set in scope could be searched by meaning. Reported rather than left to be
    /// inferred from an empty result: a session that believes memory is being consulted when it is
    /// not is the failure the `recall-when` convention exists to prevent, and it is worse than no
    /// memory. Keyword search, direct lookup and graph traversal are unaffected.
    pub searched: bool,

    /// Whether it covered **everything asked for**.
    ///
    /// **A different question from [`searched`](Self::searched), and one field answering both is
    /// what `semantic-search-available-conflates-partial-with-none` was.** A union where one set is
    /// unsearchable returns real, correctly-scored hits from the rest — so `searched` is true and
    /// `complete` is false, and a caller told only *"nothing was searched"* would discard a good
    /// ranking. The scores are sound either way: an unsearchable set is left out of the scan rather
    /// than compared across vector spaces.
    pub complete: bool,

    /// The sets left out, each with why — so the caller need not ask a second time.
    ///
    /// Empty when `complete`. Named rather than counted because the remedy differs by set and by
    /// reason, and a count cannot say which store to act on.
    pub skipped: Vec<SkippedSet>,

    /// Whether the query itself held nothing the model could represent.
    ///
    /// **A reason nothing was searched by meaning that no set can be blamed for**: the sets are
    /// searchable, and the question is in a script the model has no tokens for. Reported beside
    /// [`searched`](Self::searched) rather than folded into it because the remedy differs — search
    /// by keyword, which is script-agnostic — and because until 2026-09-11 this was an error that
    /// failed the whole recall.
    pub query_unrepresentable: bool,
}

/// A set that a recall could not search by meaning, and the reason it could not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedSet {
    /// Which set was left out.
    pub scope: Scope,
    /// Why, in the project's own words, phrased for the caller rather than the log.
    pub reason: String,
}

/// An edge pointing *at* a memory.
///
/// Derived rather than authored: the same row as the source memory's outbound link, read through
/// the links table's index on `target`. It carries the label **as written** rather than a
/// synthesised inverse, for the reason [`Store::incoming`] documents.
///
/// This lives on the retrieval types rather than on [`Memory`] deliberately. Export must be
/// structurally unable to carry a derived reverse (`documents/design/recall-mechanisms.md`), and
/// `Memory` is the type that gets exported -- so putting it here is what makes omitting it from the
/// file something nobody has to remember to do.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct IncomingEdge {
    /// The memory the edge is written on, which is always the newer of the two.
    pub source: MemoryId,
    /// The label exactly as its author wrote it.
    pub label: LinkLabel,
}

/// A standing-changing edge in another set that names this memory as its target.
///
/// **Reported, never acted on.** `documents/design/cross-set-references.md` resolves that no edge
/// changes the standing of a memory in another set, so the claim below has no effect on the memory
/// it is attached to -- and saying so is what lets a session judge the dispute instead of merely
/// being protected from it.
///
/// **The edge, not the memory.** Only the claimant's id, its set and the label it wrote: the
/// claiming memory is neither fetched nor ranked. That is deliberate and it is the whole design.
/// Returning the other memory alongside every hit it attacks would let anyone buy a permanent slot
/// in every recall that matches their target -- weaker than suppression and still a free channel
/// into a stranger's context. An id and a set name are enough to go and read it on purpose, which
/// is the only time it is wanted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DisputedBy {
    /// The set the claim comes from. Never this memory's own set.
    pub set: SetName,
    /// The memory making the claim, unfetched.
    pub source: MemoryId,
    /// What it claims: `SUPERSEDES`, `EXCEPTION_TO` or `CONTRADICTS`.
    pub label: LinkLabel,
}

/// A memory in *another* loaded set that points at this one, whatever the label.
///
/// **Separate from `incoming`, for the reason [`DisputedBy`] is separate from it**: `incoming` is
/// what this memory's own set says about it, and folding a reference from elsewhere into the same
/// list would make the two indistinguishable. The edge, not the memory — the source is neither
/// fetched nor ranked, exactly as a claim's is not.
///
/// **Carries a [`Scope`] where [`DisputedBy`] carries a [`SetName`], and that difference is the
/// point.** A `SetName` cannot say `local`, which is why claims skip personal sets: naming `local`
/// as a claimant would travel to other holders meaning a different set on every machine. Nothing
/// here travels. This is derived at read time from the stores *this machine* holds and appears in no
/// export, so a personal memory pointing into a shared set is exactly the case it exists to show —
/// and it is the only direction that can occur, since a shared memory may not link into a personal
/// set without dangling for every other holder.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReferencedFrom {
    /// The set the reference comes from. Never this memory's own set; may be `local`.
    pub scope: Scope,
    /// The memory holding the link, unfetched.
    pub source: MemoryId,
    /// What it says about this memory.
    pub label: LinkLabel,
}

/// The labels whose cross-set use is a claim over another set's memory.
///
/// `SUPERSEDED_BY` is deliberately absent: pointing it at our memory concedes the claimant's own
/// standing rather than reaching for ours, so it is not a dispute. Descriptive labels are absent
/// because they assert nothing to report.
const DISPUTING_LABELS: [LinkLabel; 3] = [
    LinkLabel::Supersedes,
    LinkLabel::ExceptionTo,
    LinkLabel::Contradicts,
];

/// One recalled memory.
#[derive(Debug, Clone, PartialEq)]
pub struct Recalled {
    /// The memory itself, always the current version.
    pub memory: Memory,
    /// Which store it came from.
    ///
    /// Load-bearing once several stores are searched together: a local preference and an imported
    /// project decision arrive in the same list, and the trust difference between them is exactly
    /// what the scope partition exists to preserve.
    pub scope: Scope,
    /// The edges pointing at this memory. Empty when nothing cites it.
    pub incoming: Vec<IncomingEdge>,
    /// How closely the *matched* memory scored, with the corpus centroid removed.
    ///
    /// Its own similarity to the query, and only that. A memory the walk reached is ranked on this
    /// **plus** [`inherited`](Self::inherited), so this number alone does not explain the order —
    /// which is why the other one is reported beside it rather than folded into it.
    pub similarity: f32,
    /// What this memory inherited from a memory the query matched directly, decayed by distance.
    ///
    /// Zero when the walk did not reach it. Reported because ranking by a number the caller cannot
    /// see is the defect `similarity-describes-a-different-memory` was: a memory arriving above one
    /// with a better similarity is not an error, and the caller has no way to know that without
    /// this.
    pub inherited: f32,
    /// The id the query actually matched, when that is not the memory being returned.
    ///
    /// Set only when a superseded memory matched and its replacement is being returned instead.
    /// Nothing else about the superseded memory is reported: in practice supersession corrects a
    /// memory whose conclusion turned out to be wrong, so its substance is actively misleading
    /// rather than merely stale, and the id is a handle for fetching it if the history matters.
    pub matched: Option<MemoryId>,
    /// True when this memory sits in a supersession cycle, so its history cannot be resolved.
    ///
    /// Separate from [`matched`](Self::matched) rather than folded into it, because the two say
    /// different things: `matched` means *you asked for something else and this replaced it*, while
    /// this means *nothing here can say what replaced what*. A cycle is returned rather than
    /// withheld — dropping it would hide a memory because its apparatus is broken — but it is
    /// returned **named**, which is what the design means by reporting rather than silently picking
    /// a winner.
    pub in_cycle: bool,
    /// True when this memory is superseded by a replacement this store cannot produce.
    ///
    /// **Two shapes reach it, and the reader's situation is the same in both.** The one the name
    /// describes is a `SUPERSEDED_BY` naming another set: the replacement is elsewhere, and the
    /// link carries the set and the id, so a session that wants it can load that set and fetch it.
    /// The other is a chain that ends on an id this store holds no live memory for -- a same-set
    /// target nothing answers to, or one that was forgotten. There the link carries an id and no
    /// more, so there is nothing to go and load; what the flag buys is that the memory is returned
    /// at all. Substituting the tip instead dropped it from every search path
    /// (`a-superseded-by-naming-an-absent-id-hides-its-memory`).
    ///
    /// **Returned and flagged rather than withheld**, which is `report rather than resolve` applied
    /// to the last place recall was still deciding for the caller. Withholding it treated *the
    /// replacement is not available* as *this memory must not be seen*, and those are different
    /// claims: the replacement's set is usually merely unloaded rather than gone, so the ordinary
    /// case was being failed, and failed silently -- nothing told a session the memory existed.
    ///
    /// The flag does not name the set because the memory already does, and a second copy here would
    /// be one more thing free to drift.
    pub superseded_beyond_this_set: bool,
    /// Superseded by a replacement this store **forgot**: withdrawn here, not held elsewhere.
    ///
    /// The memory stays superseded -- forgetting a replacement does not reinstate what it replaced,
    /// which is what the tombstone's ancestry exists to keep true -- and the remedy is the opposite of
    /// `superseded_beyond_this_set`'s: there is no set to load, and a claim that should stand again
    /// is written anew superseding the forgotten id, which `incoming` names.
    /// `a-forgotten-replacement-reads-as-superseded-beyond-this-set`.
    pub replacement_forgotten: bool,
    /// Standing-changing edges in *other* sets that name this memory. Empty in the ordinary case.
    ///
    /// Empty rather than absent-when-quiet on purpose: a dispute is unusual, and a field always
    /// carrying something teaches a reader to skim past it.
    pub disputed_by: Vec<DisputedBy>,
    /// Memories in other loaded sets pointing at this one. See [`ReferencedFrom`].
    pub referenced_from: Vec<ReferencedFrom>,
}

/// A memory found by keyword search.
///
/// Carries a match count rather than a similarity. A count and a cosine score are different scales,
/// and flattening them into one number is precisely what made cross-store scores incomparable — so
/// keyword hits are reported alongside semantic ones, never merged into their ranking.
#[derive(Debug, Clone, PartialEq)]
pub struct KeywordHit {
    /// The memory, always the current version.
    pub memory: Memory,
    /// Which store it came from.
    pub scope: Scope,
    /// The edges pointing at this memory. Empty when nothing cites it.
    pub incoming: Vec<IncomingEdge>,
    /// How many of the requested keywords this memory carries.
    pub matched_keywords: usize,
    /// The id actually matched, when a superseded memory was replaced by this one.
    pub matched: Option<MemoryId>,
    /// True when this memory sits in a supersession cycle. Same meaning as on [`Recalled`].
    pub in_cycle: bool,
    /// Superseded by a replacement this store cannot produce. Same meaning as on [`Recalled`].
    pub superseded_beyond_this_set: bool,
    /// Superseded by a replacement this store forgot. Same meaning as on [`Recalled`].
    pub replacement_forgotten: bool,
    /// Claims from other sets against this memory. Same meaning as on [`Recalled`].
    pub disputed_by: Vec<DisputedBy>,
    /// Memories in other loaded sets pointing at this one. See [`ReferencedFrom`].
    pub referenced_from: Vec<ReferencedFrom>,
}

/// A memory retrieved by direct reference.
///
/// Retrieval, not search — which is why it has no score and, more importantly, why a superseded
/// memory is returned rather than substituted. See [`Store::lookup`].
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The memory that was asked for.
    pub memory: Memory,
    /// Which store it came from.
    pub scope: Scope,
    /// The edges pointing at this memory. Empty when nothing cites it.
    pub incoming: Vec<IncomingEdge>,
    /// Whether the memory is current, replaced, or sitting in a cycle.
    ///
    /// The whole [`Resolution`], not a slice of it. It was `Vec<MemoryId>` — empty for *current* —
    /// which forced both of the other answers into the same shape: a cycle read exactly like a
    /// memory nothing supersedes, and a chain that leaves this store read like one that ends here.
    /// The type already distinguishes all three, and `a-supersession-cycle-reads-as-current` was
    /// entirely the cost of discarding that at the boundary.
    pub validity: Resolution,
    /// Standing-changing edges in other sets that name this memory. Same meaning as on [`Recalled`].
    ///
    /// Filled by the caller, which is the only participant holding the other stores. Reported here
    /// as well as on recall because lookup is how a session reads a disputed memory *on purpose* --
    /// telling it about the claim only when it stumbled on the memory by search would be the
    /// stranger of the two.
    pub disputed_by: Vec<DisputedBy>,
    /// Memories in other loaded sets pointing at this one. See [`ReferencedFrom`].
    ///
    /// Filled by the caller for the same reason: only the caller holds the other stores.
    pub referenced_from: Vec<ReferencedFrom>,
}

/// Why a store operation failed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The underlying database failed.
    #[error("the memory store failed")]
    Database(#[from] rusqlite::Error),

    /// A value read back from the database is not well-formed.
    ///
    /// Means the file was written by something other than this code, or corrupted.
    #[error("the memory store holds a malformed value: {0}")]
    Malformed(#[from] ValidationError),

    /// Several current identity records carry one name, so naming it cannot say which is meant.
    ///
    /// Two records spelling one name are two identities, not one -- a person who renamed and minted
    /// instead of superseding. Which of them the caller is cannot be worked out from the name they
    /// just supplied, since it is the ambiguous half of exactly that question.
    #[error(
        "{count} identity records in this set are named {name:?}, so naming it cannot say which \
         one you mean. Recall the keyword `hoard-author` to see them and pick one by id -- two \
         records under one name usually means a rename minted a second identity instead of \
         superseding the first."
    )]
    AmbiguousAuthor {
        /// The name that matched more than once.
        name: String,
        /// How many current records carry it.
        count: usize,
    },

    /// `mint` on a store that already knows which identity record is this machine's user.
    ///
    /// Two records for one contributor look exactly like two contributors, so nothing downstream
    /// can report the mistake. The remedies are different acts: change the name with `rename`, or
    /// point at a different record with `identify`.
    #[error(
        "this machine already writes to this set as {record}. Minting again would make a second \
         identity for one person, which nothing afterwards can tell from two contributors -- use \
         \"rename\" to change the name you publish under here, or \"identify\" to point at a \
         different record."
    )]
    AlreadyIdentified {
        /// The record already in force.
        record: String,
    },

    /// `mint` where a current identity record already carries the name.
    #[error(
        "{record} already carries the name {name:?} in this set. If that record is you, say so with \
         \"identify\" and its id; if it is somebody else who happens to share the name, the name \
         cannot tell you apart and the id is the only thing that can."
    )]
    NameAlreadyHeld {
        /// The name that is already taken.
        name: String,
        /// The record carrying it.
        record: String,
    },

    /// `identify` pointed somewhere that cannot be an identity.
    #[error("{record} cannot be the identity you write as here: {why}")]
    NotAnIdentityRecord {
        /// The id that was offered.
        record: String,
        /// Why it cannot serve.
        why: String,
    },

    /// `rename` on a store with no identity to rename.
    #[error(
        "this machine has not said which identity record is yours in this set, so there is nothing \
         to rename. Use \"identify\" with the id of your record here, or \"mint\" if you are new \
         to this set."
    )]
    NotIdentifiedHere,

    /// The author map was asked of a store that is not `local`.
    ///
    /// *Which identity record is me in set X* is a fact about this machine, held in one place so
    /// there is no question which copy answers. A shared store holds no map, and reaching for one
    /// means the caller has the wrong store rather than that the answer is missing.
    #[error(
        "cannot {what} on {scope}: the author map is machine state and lives in the local store"
    )]
    NotTheLocalStore {
        /// What was attempted.
        what: String,
        /// The store it was attempted on.
        scope: String,
    },

    /// A recorded base checksum is not readable.
    ///
    /// Reported rather than treated as absent. A missing base means "we have never agreed about this
    /// record", which is a legitimate state with its own resolution — so silently substituting it for
    /// an unreadable one would re-baseline the record and could discard a real conflict.
    #[error("a recorded base checksum is not hexadecimal: {checksum:?}")]
    MalformedBase {
        /// The value found in the row.
        checksum: String,
    },

    /// A rebuild was asked for with no model to rebuild with.
    ///
    /// Its own variant rather than a database error, because the remedy is entirely different: this
    /// is a configuration to fix, not a failure to retry.
    #[error("no embedding model is loaded, so there is nothing to rebuild the vectors with")]
    NoEmbedder,

    /// The store on disk belongs to a different scope than the caller asked for.
    ///
    /// The same guard as a memory file's header, one layer down: what a store *is* comes from where
    /// it was opened, and the file can only agree.
    #[error("this store belongs to scope {found:?}, not the one it was opened as")]
    ScopeMismatch {
        /// The scope recorded in the store.
        found: Scope,
    },

    /// A record's stored file position disagrees with its id, or repeats the one before it.
    ///
    /// The position is derived from the id and stored so the database can order an export
    /// (`file-position-column`); derived state can drift, through a row written by a build with a
    /// different rule or edited by hand. **Refused rather than written**, because the alternative is
    /// a file silently out of order — which the next export reorders wholesale — or one carrying an
    /// id twice, which no reader will accept.
    #[error(
        "record {id} is out of file order in the store: its stored position does not match its id, \
         or it repeats the record before it"
    )]
    OutOfFileOrder {
        /// The record at which the order broke.
        id: MemoryId,
    },

    /// A stored enum or timestamp is not one this build recognises.
    ///
    /// Reported rather than panicked on: the store is a file on disk, and this code should defend
    /// itself against one it did not fully author.
    #[error("the memory store holds an unrecognised {field} value: {value:?}")]
    Unrecognised {
        /// What was being read.
        field: &'static str,
        /// The value found.
        value: String,
    },

    /// Embedding failed.
    #[error("embedding failed")]
    Embedding(#[from] crate::embedder::EmbedderError),

    /// A stored vector could not be read back.
    #[error("a stored vector is malformed")]
    Vector(#[from] crate::embedding::EmbeddingError),

    /// A rewrite would have destroyed an embedding nothing here could produce again.
    ///
    /// The message names the fix, because the caller can act on it and the alternative — reporting
    /// only that the write failed — sends its reader looking for setup instructions it should not
    /// need. Refusing is narrow by design: storing a *new* memory in the same store is allowed,
    /// since a memory with no vector loses nothing by being written.
    #[error(
        "memory {memory} already has an embedding and no model is loaded to rebuild one, so \
         rewriting it would remove it from search by meaning permanently. Storing new memories, \
         keyword search and hoard_lookup are unaffected. Load the embedding model and try again; \
         the store repairs itself once it can."
    )]
    CannotRebuildEmbedding {
        /// The memory whose rewrite was refused.
        memory: MemoryId,
    },

    /// A write would have created a supersession cycle.
    #[error("memory {memory} cannot supersede {target}: it is already superseded by it")]
    SupersessionCycle {
        /// The memory being written.
        memory: MemoryId,
        /// The memory it claimed to supersede.
        target: MemoryId,
    },

    /// A write would have given the set a second live charter.
    #[error(
        "memory {memory} carries `hoard-scope-topics`, and this set already has a live charter, \
         {charter}. A set has one charter. To replace it, supersede {charter}; if this is a memory \
         about charters rather than the charter itself, give it `about-hoard-scope-topics` instead."
    )]
    SecondLiveCharter {
        /// The memory being written.
        memory: MemoryId,
        /// A charter already live that the write does not supersede.
        charter: MemoryId,
    },

    /// A write would have given the set a second live model-fit hub.
    #[error(
        "memory {memory} carries `hoard-fit-hub`, and this set already has a live model-fit hub, \
         {hub}. A set has one. To replace it, supersede {hub}; if this is a memory about the hub \
         rather than the hub itself, give it `about-hoard-fit-hub` instead."
    )]
    SecondLiveFitHub {
        /// The memory being written.
        memory: MemoryId,
        /// A hub already live that the write does not supersede.
        hub: MemoryId,
    },

    /// A sentence-set memory that cannot be scored as written.
    #[error(
        "memory {memory} carries `hoard-fit-sentences`, but {problem}. A sentence set is one \
         paraphrase test for one language: a `hoard-lang-` keyword naming the language, and text \
         holding one JSON array of three sentences -- an anchor, a paraphrase of it that shares as \
         few words with it as the language allows, and a sentence about something else. Prose may \
         come before the array, for example: Paraphrase test 3 for Samoan. [\"anchor\", \
         \"paraphrase\", \"unrelated\"]"
    )]
    MalformedFitSentences {
        /// The memory being written.
        memory: MemoryId,
        /// What is wrong with it.
        problem: String,
    },

    /// A model recommendation that does not link to the set's hub.
    #[error("{}", unlinked_fit_model(*memory, *hub))]
    UnlinkedFitModel {
        /// The memory being written.
        memory: MemoryId,
        /// The set's live hub, where it has one: the memory to link to.
        hub: Option<MemoryId>,
    },

    /// A model-fit hub listing a number of sentence sets for some language other than zero or ten.
    #[error(
        "memory {memory} carries `hoard-fit-hub` and links to {counts}. A hub lists exactly ten \
         sentence sets for each language it supplies sentences for, or none: every language is \
         scored 0 to 10, each test against the other 29 sentences in its language, and a \
         different number would put that language on a different scale. Languages are counted by \
         language and script, ignoring region."
    )]
    IncompleteFitHub {
        /// The memory being written.
        memory: MemoryId,
        /// Each language whose count is wrong, and its count, as a phrase.
        counts: String,
    },

    /// A write would have introduced a `hoard-` keyword that is not a control mark.
    ///
    /// The marks are spelled out in the message rather than interpolated from [`CONTROL_MARKS`],
    /// because a format string cannot iterate; `tests/store.rs` holds the two together.
    #[error(
        "memory {memory} carries `{keyword}`, which this version of Scatterhoard does not know as a \
         control mark and which this set holds nowhere else. Keywords beginning `hoard-` are \
         reserved: the marks are `hoard-author`, `hoard-scope-topics`, `hoard-signpost`, \
         `hoard-collect`, `hoard-investigation`, `hoard-fit-hub`, `hoard-fit-sentences` and \
         `hoard-fit-model`, and every keyword beginning `hoard-lang-` (a \
         charter's languages) or `hoard-proj-` (a project's own tooling). If this is a memory about \
         a mark, give it `about-{keyword}` instead. If the mark comes from a newer Scatterhoard, \
         this one needs updating before it can introduce it."
    )]
    UnknownControlMark {
        /// The memory being written.
        memory: MemoryId,
        /// The keyword refused, as it was written.
        keyword: String,
    },

    /// A session tried to write to a set this machine holds read-only.
    ///
    /// Refused by the store rather than by each tool, because `drop` skips the safety copy for a
    /// read-only set on the strength of this refusal: a check every caller must remember is one the
    /// next caller forgets, and then the skipped copy loses writing. `documents/design/load-modes.md`.
    #[error(
        "the set {set} is read-only on this machine, chosen when it was created: its memories come \
         from the file it was imported from, so a change to them belongs in that file, made by \
         whoever publishes the set. Importing a newer file refreshes it."
    )]
    ReadOnly {
        /// The set's name.
        set: String,
    },

    /// A set that already holds records was asked to become read-only.
    ///
    /// **A mode is chosen at birth.** A read-only set is dropped without a safety copy because this
    /// machine never wrote to it, and that is true only if it was read-only before anything arrived.
    #[error(
        "a set's mode is chosen when it is created, and this one already holds records. To hold it \
         read-only, drop it with a `save_to` path, create it again with mode \"read-only\", and \
         import the saved copy."
    )]
    ModeIsChosenAtCreation,

    /// A record's link names a set loaded for review, which no stored link may.
    #[error(
        "this memory links to the set {set:?}, which is a copy loaded for review rather than a set \
         anyone has adopted. Such a copy lives in memory for one session, so the link would point at \
         nothing by the next start while still reading as a reference. Link to the set it is a copy \
         of instead."
    )]
    LinkNamesEvaluationSet {
        /// The name the link carried.
        set: String,
    },

    /// The database records a load mode this build does not know -- written by a newer build.
    #[error(
        "this set's database records the load mode {found:?}, which this build does not know. It \
         was probably written by a newer build of Scatterhoard; update this one."
    )]
    UnknownLoadMode {
        /// The recorded value.
        found: String,
    },
}

/// How this machine holds a set -- `documents/design/load-modes.md`.
///
/// **A value carrying a policy, not a flag.** Each thing that differs between modes is asked of the
/// mode, so a combination no mode has cannot be built. Evaluation is the third mode the design
/// describes, and arrives later as a variant here rather than as a second flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoadMode {
    /// The machine's own set: every set before load modes existed, and the default.
    ReadWrite,
    /// A set someone else publishes, refreshed from their file and never written here.
    ReadOnly,
    /// A copy loaded from a file for review, under a temporary name, adopted by nobody.
    ///
    /// Read-only with three additions it alone carries: it is searched only when its scope is
    /// named, every memory it returns is marked, and it is never a holder of its source set's
    /// identity. Those live where each is decided rather than here.
    Evaluation,
}

impl LoadMode {
    /// The spelling stored in `meta` and shown to callers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadWrite => "read-write",
            Self::ReadOnly => "read-only",
            Self::Evaluation => "evaluation",
        }
    }

    /// Whether a session may write to a set held this way.
    #[must_use]
    pub const fn accepts_session_writes(self) -> bool {
        match self {
            Self::ReadWrite => true,
            Self::ReadOnly | Self::Evaluation => false,
        }
    }

    /// The watch marks a session recalls for a set held this way, which `hoard_sets status` reports
    /// per set and the `recall-when` convention sends a session to read.
    ///
    /// **The mapping lives here, beside the mode, rather than in convention text.** Each mark names
    /// an action: a signpost asks for a recall, and collecting and investigating ask for writes a
    /// read-only set cannot take. Spelling that out in the catalog would cost characters and would
    /// need rewording for every mode added; deriving it from the mode needs neither.
    #[must_use]
    pub const fn watch_marks(self) -> &'static [&'static str] {
        match self {
            Self::ReadWrite => &[SIGNPOST_MARK, COLLECT_MARK, INVESTIGATION_MARK],
            Self::ReadOnly => &[SIGNPOST_MARK],
            // Nothing: a set nobody has adopted asks nothing of a session, and its collections and
            // investigations are its own project's, not this machine's.
            Self::Evaluation => &[],
        }
    }

    /// Whether dropping a set held this way owes a safety copy first: true exactly where this machine
    /// could have written to it.
    #[must_use]
    pub const fn owes_a_safety_copy(self) -> bool {
        self.accepts_session_writes()
    }

    /// Whether a set held this way is searched only when a recall names its scope, and every memory
    /// it returns marked as coming from a set under review.
    #[must_use]
    pub const fn is_evaluation(self) -> bool {
        matches!(self, Self::Evaluation)
    }
}

/// Whether this process can maintain the vectors a store already holds.
///
/// The states in `documents/design/embeddings.md`. Deliberately not a boolean: the question that
/// was being asked as one — *is there an embedder?* — cannot tell a store that legitimately has no
/// vectors from one whose model is missing, and those want opposite treatment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelState {
    /// Nothing has ever embedded into this store, so it is bound to no model.
    ///
    /// A complete state rather than a broken one: keyword search, direct lookup and traversal are
    /// the retrieval paths, all of them work, and there are no vectors for a write to invalidate.
    Unembedded,

    /// The store holds vectors, and nothing records which model produced them.
    ///
    /// Distinct from [`Unembedded`](ModelState::Unembedded), which it was reported as until
    /// 2026-08-09, and the distinction is the whole point: that variant promises *"there are no
    /// vectors for a write to invalidate"*, which is precisely false here. Every store written
    /// before the model identity was recorded is in this state, so it is a migration state rather
    /// than a corruption — but it is not a safe one, because a mismatch cannot be detected against
    /// a model nobody wrote down.
    ///
    /// The count is carried because it is the fact that makes the state actionable: *how much*
    /// would be rebuilt, or lost.
    ProvenanceUnknown {
        /// How many vectors are unaccounted for.
        vectors: usize,
    },

    /// The store's vectors came from the model this process holds.
    ///
    /// Carries the identity even though both sides agree on it, because the caller comparing two
    /// servers needs it *most* when nothing is wrong: that is the moment two configurations can
    /// still be told apart cheaply, before either has written anything the other must undo.
    Current {
        /// The model both the store and this process are using.
        model: ModelIdentity,
    },

    /// The store holds vectors from a model this process cannot provide.
    ///
    /// Not a degradation but a misconfiguration: reads stay correct, because none of the paths that
    /// still work needed a model, while any write touching the embedded text would destroy an
    /// invariant this process is not equipped to restore.
    Mismatched {
        /// The model the stored vectors came from.
        recorded: ModelIdentity,
        /// The model in hand, if any. `None` and a different model are the same *state* and want
        /// different *wording*, which is why this is kept rather than collapsed to a flag.
        loaded: Option<ModelIdentity>,
    },
}

/// How many displaced models a store remembers.
///
/// A signal, not a log. Only the recent past distinguishes an alternation from an ordinary run of
/// upgrades, and an unbounded list would eventually report every model as a repeat. Four covers the
/// two-server case with room to see a three-way cycle; whether it is enough is a question for the
/// multi-server environment rather than for argument.
const MODEL_HISTORY: usize = 4;

/// What a replacement put into a store.
///
/// **Two numbers rather than one, and they are the same two `export` reports.** A set is memories
/// plus what it has forgotten, and a single count of "records" is the shape that made `status` and
/// `export` disagree about a field they both called `memories`
/// (`memories-count-means-two-different-things`). Restore is the operation where the difference is
/// most worth seeing: a restore that silently dropped the tombstones would report a plausible
/// memory count and quietly resurrect everything the set had forgotten.
/// What a `rename` did: the record it replaced, the one now in force, and the name that was removed.
///
/// The old name is reported because the caller is the last reader who will see it — the record it
/// came from is a tombstone by the time this is returned, which is the point of the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    /// The record that was superseded and then forgotten.
    pub previous: MemoryId,
    /// The record now in force.
    pub replacement: MemoryId,
    /// The name the superseded record carried, if it could still be read.
    pub was: Option<Author>,
}

/// A row of the durable author map: which identity record is this machine's user, in one set.
///
/// `documents/design/memory-data-model.md`, *which record is you is machine state*. The map lives in
/// the `local` store because that is the only store with the right lifetime — machine-bound, reached
/// by every checkout, and out of reach of `drop`, which accepts only a shared set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorRow {
    /// The identity record this machine writes as, in the set the row is keyed by.
    pub record: MemoryId,
    /// The set's scope as last seen, for a person reading the row. **A label, never a key.**
    pub scope: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replaced {
    /// Live memories written.
    pub memories: usize,
    /// Tombstones written — what the restored set remembers having forgotten.
    pub forgotten: usize,
}

/// What an explicit rebuild did, and whether it looks like a turn in a tug-of-war.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rebuilt {
    /// How many memories were re-embedded.
    pub vectors: usize,
    /// How many memories the model had nothing to represent with.
    ///
    /// Reported beside `vectors` so a rebuild that leaves memories unfindable says so: without it,
    /// seven memories rebuilding to five vectors reads as two lost rather than two in a script the
    /// model does not cover.
    pub unrepresentable: usize,
    /// The model they now belong to.
    pub model: ModelIdentity,
    /// The model that held the store before this one, if any.
    pub displaced: Option<ModelIdentity>,
    /// How many times this model has *already* held the store and been displaced.
    ///
    /// Zero is the ordinary upgrade path. Anything higher means the store is being taken back, and
    /// two configurations are disagreeing rather than one being corrected.
    pub seen_before: usize,
}

/// How long a write waits for a lock another connection holds before giving up.
///
/// Two servers on one store overlap for milliseconds at a time, so anything that fails sooner than
/// this turns a momentary collision into a hard error. Set here rather than left to `rusqlite`,
/// which happens to default to the same five seconds: the requirement belongs to the store, and a
/// dependency's default is not a decision this project made.
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// A memory store for one scope.
///
/// # What is cached, and what makes it stale
///
/// Everything except [`Store::vectors`] is read from SQLite at the moment it is asked for, so it is
/// current by construction. The vector set is the one exception, and it is deliberate: re-deriving
/// it would put the embedder on the critical path of simply opening a store.
///
/// A cache is only safe while this connection is the only writer, and it is not — a server is
/// registered per checkout, so two checkouts already put two processes on one `local` store. The
/// staleness that follows is silent and asymmetric: the other process's memories are missing from
/// *semantic* recall while keyword search, which reads SQL, finds them, so the two search paths
/// disagree about what the store contains and neither response says so.
///
/// [`Store::refresh`] closes that, and the discipline this type keeps is that **every read of the
/// cache takes `&mut self`**. That is what makes forgetting to refresh a compile error rather than
/// a defect nobody sees. Adding cached state here means adding it to `refresh`, and giving its
/// readers `&mut` as well.
pub struct Store {
    connection: Connection,
    scope: Scope,
    /// Held in memory for scanning and persisted alongside the memories, so reopening a store does
    /// not put the embedder on the critical path of simply opening it. Refreshed under the rule in
    /// this type's own documentation; do not read it from a `&self` method.
    vectors: VectorSet,
    /// The value `PRAGMA data_version` had when `vectors` was last built. SQLite bumps it when
    /// *another* connection commits and never for this one's own writes, which is exactly the
    /// question being asked — writes made here keep the cache in step as they go.
    ///
    /// `None` until the first load, so "never built" is a state rather than a magic number that a
    /// real version could one day collide with.
    cache_built_at: Option<i64>,
    embedder: Option<Arc<dyn Embedder>>,
    /// Which file this store opened, as the filesystem identifies it, or `None` in memory.
    ///
    /// `a-set-dropped-by-another-server-stays-writable-here`: an open SQLite database survives its
    /// file being deleted, so a store whose set another server dropped -- or dropped and made again --
    /// would go on writing to a file nobody else can reach, and SQLite, reaching its `-wal` and
    /// `-shm` files by path, can damage the new set's. The path is shared state and the open
    /// connection is this process's private copy of it, so [`Store::file_is_gone`] compares the two.
    opened_file: Option<FileIdentity>,
}

/// A file as the filesystem identifies it: the device and inode on Unix, which change when a file is
/// deleted and another made at its path; elsewhere only that a file is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity(u64, u64);

/// The identity of the file at `path`, or `None` where there is none.
fn file_identity(path: &Path) -> Option<FileIdentity> {
    let metadata = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(FileIdentity(metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Some(FileIdentity(0, 0))
    }
}

/// What scanning a layout's directories found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// Each scope and the file it resolves to, in scope order. `local` is always present.
    pub stores: Vec<(Scope, PathBuf)>,
    /// Stores that lost to a copy in the common directory, and so are not loaded.
    ///
    /// Reported rather than deleted or refused. A set in both directories is a configuration to fix
    /// and only its owner can say which copy was meant — but leaving a session with no store at all
    /// would strand it, so one is chosen and the other is named.
    pub shadowed: Vec<(Scope, PathBuf)>,
}

/// The subdirectory of the home holding copies loaded for review.
pub const REVIEW_DIRECTORY: &str = "reviews";

/// The export review's own settings for a set, kept in its store's `meta` table and never exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewSetting {
    /// Present while a full review is under way: the file on disk no longer counts as reviewed.
    FullReview,
    /// The audience the project declared at the last review, so a change to public is noticed.
    Audience,
}

impl ReviewSetting {
    fn key(self) -> &'static str {
        match self {
            Self::FullReview => "review_full",
            Self::Audience => "review_audience",
        }
    }
}

/// Which directory holds each scope's store.
///
/// `documents/design/store-registry.md`, *checkout-bound and machine-bound sets*. Two directories,
/// and the rule between them is small enough to state here: **a store in the common directory is
/// machine-bound and every checkout on this machine reaches it; a store in the home belongs to that
/// home alone.** Where the file sits is what decides which it is.
///
/// It exists as a type rather than as lines of startup wiring because those lines have to agree with
/// each other, and nothing would have noticed if they stopped — the local store is opened, the
/// directories are scanned, and a created set is written, each choosing a directory separately. That
/// is the shape of `project-stores-get-no-embedder`, where startup wiring gave the model to one
/// store and not the others and no test could see it. Here the rule is one type with tests against
/// it, and `main.rs` supplies only the two paths.
///
/// **`local` needs no decision**, because its scope *type* already carries the answer: it is
/// personal and machine-wide, so it is always machine-bound. A shared set's type carries nothing —
/// `coding-lessons` and `scatterhoard` are both [`Scope::Shared`] and only intent separates them,
/// which is why that one has to be said rather than inferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreLayout {
    home: PathBuf,
    /// `None` means *not asked for*, which is deliberately not the same as *asked for, and equal to
    /// the home* — see [`StoreLayout::common_is_the_home`].
    common: Option<PathBuf>,
}

impl StoreLayout {
    /// A layout over a home, and optionally a separate directory for machine-bound stores.
    ///
    /// `None` is the ordinary case and **degenerates to today's behaviour exactly** — one directory
    /// holding everything. That property is what makes the split safe to introduce: someone who
    /// never asks for it cannot be affected by it.
    #[must_use]
    pub fn new(home: PathBuf, common: Option<PathBuf>) -> Self {
        Self { home, common }
    }

    /// The directory holding this home's own shared sets, the model, and anything else it owns.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where copies loaded for review are kept.
    ///
    /// **Under the home rather than the system temp directory.** The home is a directory the user
    /// chose and which already holds their stores, so it is known to be sized for them; `/tmp` is
    /// commonly a small root partition, and nothing portable can tell us how much room either has.
    /// Decided 2026-09-18 by the human partner.
    ///
    /// **Its own directory, and one for the machine rather than one per run**, because these files
    /// are loaded again at the next start: a review outlives the server that opened it, and `drop` is
    /// then the one way a copy goes — the same way any set goes — rather than a second cleanup
    /// mechanism nobody runs. Separate from the store directories all the same, so that what a
    /// session adopted and what it is merely reading never sit in one listing.
    #[must_use]
    pub fn reviews(&self) -> PathBuf {
        self.home.join(REVIEW_DIRECTORY)
    }

    /// The directory holding machine-bound stores. Equal to [`StoreLayout::home`] unless told
    /// otherwise.
    #[must_use]
    pub fn common(&self) -> &Path {
        self.common.as_deref().unwrap_or(&self.home)
    }

    /// Whether a common directory was named and turns out to *be* the home.
    ///
    /// Harmless — it is the default written out — but it is a stated intention that does nothing,
    /// and a setting that looks applied while having no effect is the shape this codebase keeps
    /// filing defects about. Worth reporting, which is why it is distinguishable at all: the
    /// unasked-for case answers `false`, because nobody asked for anything.
    ///
    /// Compares the directories rather than the strings, so a path that merely *reaches* the home
    /// still counts as the home — the same trap as
    /// `target-identity-is-an-unnormalised-path-string`.
    #[must_use]
    pub fn common_is_the_home(&self) -> bool {
        self.common
            .as_deref()
            .is_some_and(|common| same_directory(common, &self.home))
    }

    /// Whether there are genuinely two directories to look in.
    ///
    /// Deliberately not [`StoreLayout::common_is_the_home`] negated: that one is about whether
    /// someone *asked* for something, and answers `false` when nothing was asked for at all. This
    /// one is about the filesystem. Conflating them made every set shadow itself in the default
    /// configuration, which is the case with no common directory named.
    fn directories_differ(&self) -> bool {
        !same_directory(self.common(), &self.home)
    }

    /// Where a store for this scope is *created*.
    ///
    /// A new shared set is always made in the home, because nothing at creation time knows whether
    /// it will turn out to be machine-bound — that is a fact about how the set gets used, learned
    /// later. Moving it is how it becomes machine-bound.
    #[must_use]
    pub fn directory_for(&self, scope: &Scope) -> &Path {
        match scope {
            Scope::Local => self.common(),
            Scope::Shared(_) => &self.home,
        }
    }

    /// The file a store for this scope is created in. See [`StoreLayout::directory_for`] — for an
    /// existing shared set, [`StoreLayout::discover`] is the question to ask instead.
    #[must_use]
    pub fn path_for(&self, scope: &Scope) -> PathBuf {
        Store::path_for(scope, self.directory_for(scope))
    }

    /// Every store these directories hold, and which copy wins where both do.
    ///
    /// **The common copy wins.** The ways to reach a set living in both places all leave the *home*
    /// copy as the stale one: a move that left the original behind, or a `create` that made a fresh
    /// empty store for a set already common. Letting the home win would put an empty store in front
    /// of real memories, which is `moving-the-local-store-leaves-the-old-one-unmentioned` a second
    /// time, and this project has filed that shape often enough to design against it.
    ///
    /// Lenient about unreadable directories, which read as holding nothing. A directory that cannot
    /// be listed produces no stores rather than an error, in keeping with the startup path this
    /// replaces — and its absence is visible anyway, because `status` reports the file behind every
    /// set it does hold.
    #[must_use]
    pub fn discover(&self) -> Discovery {
        let mut stores: BTreeMap<Scope, PathBuf> = BTreeMap::new();
        let mut shadowed = Vec::new();

        // `local` first and from the common directory always: its scope type settles it, and it is
        // present whether or not the file exists yet, because the local store is created on demand.
        stores.insert(Scope::Local, self.path_for(&Scope::Local));

        // Common before home, so the home's insert is the one that finds a name taken. Scanned once
        // when both name one directory, or every set would shadow itself.
        let directories: Vec<&Path> = if self.directories_differ() {
            vec![self.common(), &self.home]
        } else {
            vec![&self.home]
        };

        for directory in directories {
            for scope in shared_scopes_in(directory) {
                let path = Store::path_for(&scope, directory);
                match stores.entry(scope.clone()) {
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(path);
                    }
                    std::collections::btree_map::Entry::Occupied(_) => shadowed.push((scope, path)),
                }
            }
        }

        // **Copies loaded for review, from their own directory.** They are loaded again at every
        // start so that a review survives a restart and `drop` stays the one way a copy goes. They
        // cannot collide with the stores above: every name here carries the reserved `eval-` prefix,
        // which no set anyone adopted may have.
        let reviews = self.reviews();
        for scope in shared_scopes_in(&reviews) {
            let path = Store::path_for(&scope, &reviews);
            stores.entry(scope).or_insert(path);
        }

        Discovery {
            stores: stores.into_iter().collect(),
            shadowed,
        }
    }
}

/// Whether two paths name one directory, resolving each where the filesystem can.
///
/// Falls back to comparing the paths when a side cannot be resolved — a directory that does not
/// exist has no canonical form, and saying "different" there is the answer that changes nothing.
fn same_directory(one: &Path, other: &Path) -> bool {
    match (one.canonicalize(), other.canonicalize()) {
        (Ok(one), Ok(other)) => one == other,
        _ => one == other,
    }
}

/// The shared sets whose databases sit directly in `directory`, recognised by the filename
/// [`Store::path_for`] writes.
fn shared_scopes_in(directory: &Path) -> Vec<Scope> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let set = name.strip_prefix("shared-")?.strip_suffix(".sqlite3")?;
            SetName::new(set).ok().map(Scope::Shared)
        })
        .collect()
}

impl Store {
    /// One spelling for a directory, whatever spelling the caller used.
    ///
    /// `target-identity-is-an-unnormalised-path-string`. A reconciliation target was the caller's
    /// path as typed, so `/p/proj` and `/p/proj/`, an absolute path and a relative one, and a
    /// symlinked route to one checkout were **four targets with four independent base sets** for one
    /// directory. Export remarked on the unfamiliar spelling; import reconciled the whole file
    /// against no ancestor and said nothing, which is the single case the per-record base exists to
    /// prevent.
    ///
    /// **Full resolution, symlinks included**, which is what [`Store::file`] already does to a
    /// database path and for the same reason: the key should name where the directory actually is
    /// rather than the route configured to reach it.
    ///
    /// **A path that cannot be resolved keeps the spelling it came with, and nothing fails.** Three
    /// things make that safe rather than a quiet reintroduction of the defect:
    ///
    /// - **A base is only ever consulted by exact key.** An unresolvable key can therefore only be
    ///   matched by the very path it names, so it can never supply an ancestor for a different
    ///   directory. It is inert, not wrong.
    /// - **Every operation that records a target has already established that the path exists** —
    ///   `Destination::refusal` for a write, opening the file for a read — so this can only fail
    ///   here by a race, and losing normalisation on that call costs one noisy reconciliation
    ///   rather than a wrong one.
    /// - **A target must stay nameable after its directory goes away.** An unmounted volume or a
    ///   checkout deleted and later restored keeps its agreement, where refusing or dropping would
    ///   discard ancestry that is still good. *This reason is the weakest of the three and is
    ///   stated as such: it assumes the path re-identifies the same volume, which is precisely what
    ///   removable media breaks — `a-target-path-can-come-to-mean-a-different-volume` in
    ///   `TRACKING.md`. The first reason carries the decision on its own.*
    ///
    /// The parent is tried when the path itself does not resolve, because an explicit `file` target
    /// names a file `export` is about to *create*: resolving only the whole path would key the first
    /// export raw and every later import canonically, which is the defect again with extra steps.
    ///
    /// Idempotent, so applying it at the store boundary and again at a caller's comparison costs
    /// nothing and neither site has to know about the other.
    #[must_use]
    pub fn normalise_target(target: &str) -> String {
        let path = Path::new(target);
        if let Ok(resolved) = path.canonicalize() {
            return resolved.to_string_lossy().into_owned();
        }
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
            && let Ok(resolved) = parent.canonicalize()
        {
            return resolved.join(name).to_string_lossy().into_owned();
        }
        target.to_owned()
    }

    /// The file a scope's store lives in, inside `directory`.
    ///
    /// Project stores are prefixed so that a project legitimately named `local` cannot collide with
    /// the local store. `SetName` already excludes path separators and cannot be `.` or `..`, so no
    /// escaping is needed here.
    #[must_use]
    pub fn path_for(scope: &Scope, directory: &Path) -> PathBuf {
        let name = match scope {
            Scope::Shared(set) => format!("shared-{set}.sqlite3"),
            Scope::Local => "local.sqlite3".to_owned(),
        };
        directory.join(name)
    }

    /// Open (or create) the store for `scope` inside `directory`.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn open(scope: Scope, directory: &Path) -> Result<Self, StoreError> {
        let path = Self::path_for(&scope, directory);
        Self::open_at(scope, &path)
    }

    /// Open (or create) the store for `scope` at an exact path.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ScopeMismatch`] if the file already holds a different scope.
    pub fn open_at(scope: Scope, path: &Path) -> Result<Self, StoreError> {
        let mut store = Self::from_connection(scope, Connection::open(path)?)?;
        store.opened_file = store.file().and_then(file_identity);
        Ok(store)
    }

    /// Whether the file this store opened is no longer the one at its path: deleted, or deleted and
    /// replaced -- as when another server sharing the directory drops the set, or drops it and makes
    /// it again. Always `false` in memory.
    ///
    /// **Compared by identity, not by existence**, because a set dropped and made again leaves a
    /// file at the path, and it is not this one. See the field note on `opened_file`.
    #[must_use]
    pub fn file_is_gone(&self) -> bool {
        let Some(opened) = self.opened_file else {
            return false;
        };
        self.file().and_then(file_identity) != Some(opened)
    }

    /// Open a store that exists only for the lifetime of this value.
    ///
    /// **Exactly like [`open`](Self::open), except in memory** -- and in particular it does not mint a
    /// set identity: its first write does. For a store that needs one before anything is written, to
    /// build a link naming it, use [`open_in_memory_and_mint_id`](Self::open_in_memory_and_mint_id).
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn open_in_memory(scope: Scope) -> Result<Self, StoreError> {
        Self::from_connection(scope, Connection::open_in_memory()?)
    }

    /// Open a store in memory and mint its set identity, as its first write would.
    ///
    /// **The minting is in the name on purpose.** The human partner's review, 2026-09-14: it is
    /// intuitive that `open_in_memory` acts like `open`, and not that it would also write an identity,
    /// so the constructor that does says so rather than hiding the difference behind a familiar name.
    ///
    /// Writing at open does not cross the rule that opening a store must never write to it. That rule
    /// exists because several processes open one database file, and a reader that wrote would make
    /// the identity a store settles on a race between their startups; nothing else can open an
    /// in-memory store.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn open_in_memory_and_mint_id(scope: Scope) -> Result<Self, StoreError> {
        let store = Self::open_in_memory(scope)?;
        store.mint_set_id()?;
        Ok(store)
    }

    fn from_connection(scope: Scope, connection: Connection) -> Result<Self, StoreError> {
        Self::configure_for_sharing(&connection)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memories (
                 id           TEXT PRIMARY KEY,
                 content      TEXT NOT NULL,
                 why          TEXT,
                 context      TEXT,
                 confidence   REAL NOT NULL,
                 origin       TEXT NOT NULL,
                 created_by   TEXT NOT NULL,
                 created_at   TEXT NOT NULL,
                 modified_by  TEXT NOT NULL,
                 modified_at  TEXT NOT NULL,
                 -- Nullable deliberately; see `fill_file_positions`.
                 file_position INTEGER
             );
             -- `keyword` is the record and `folded` is the index key, deliberately two columns.
             -- They were one, folded, which meant storing a keyword destroyed the spelling the
             -- caller wrote and no interface could show it back. Searching reads `folded`; reading
             -- a memory reads `keyword`.
             CREATE TABLE IF NOT EXISTS keywords (
                 memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                 keyword   TEXT NOT NULL,
                 folded    TEXT NOT NULL DEFAULT '',
                 position  INTEGER NOT NULL,
                 PRIMARY KEY (memory_id, position)
             );
             -- The index on `folded` is created by `add_folded_keywords`, not here: on a store
             -- written before that column existed this statement would run against a table that
             -- does not have it yet, and opening the store would fail outright.
             -- **No foreign key, and that is the whole of `forgetting-a-chains-middle-…`.**
             -- A supersession edge lives on the memory that authored it, so a cascade from
             -- `memories` deletes the ancestry along with the record and a chain silently loses its
             -- middle -- leaving a twice-replaced conclusion reporting itself as current. The rows
             -- that outlive their source are exactly the ones a tombstone carries; every other
             -- link for a deleted id is removed explicitly, by `forget` and by `write_memory`,
             -- which is where the intent is legible. Same reasoning as `bases` one table down:
             -- cascade derived data, never a record of what happened.
             CREATE TABLE IF NOT EXISTS links (
                 source          TEXT NOT NULL,
                 label           TEXT NOT NULL,
                 target          TEXT NOT NULL,
                 target_set      TEXT,
                 target_keywords TEXT,
                 -- The target set's own identity, where the authoring machine knew it. Nullable
                 -- permanently: a link may name a set nobody here has ever held.
                 target_set_id   TEXT,
                 -- `target_set` is in the key because `target` is a memory id alone, and ids repeat
                 -- across sets: a memory copied into another set keeps its id. Without it, two links
                 -- differing only by which set they name collide and `INSERT OR REPLACE` drops the
                 -- first -- `a-memory-linking-one-id-in-two-sets-keeps-one-link`.
                 PRIMARY KEY (source, label, target, target_set)
             );
             -- The reverse direction of the same rows. This index is the reverse index; there is no
             -- second structure that could fall out of step with the edges themselves.
             CREATE INDEX IF NOT EXISTS links_by_target ON links(target);
             -- What keeps the wider key from trading one silent fault for another. SQLite holds
             -- NULLs in a key distinct from each other, and a same-set link is exactly a row whose
             -- `target_set` is NULL, so the key above would stop collapsing two identical same-set
             -- links. This collapses them, over precisely the rows the key no longer covers.
             CREATE UNIQUE INDEX IF NOT EXISTS links_same_set
                 ON links(source, label, target) WHERE target_set IS NULL;
             -- Vectors are derived state, but re-deriving them on every process start would put the
             -- embedder on the critical path of simply opening a store. Stored as a raw f32 blob:
             -- the centroid is subtracted at scan time, never baked in, because it moves whenever a
             -- memory is added.
             CREATE TABLE IF NOT EXISTS vectors (
                 memory_id TEXT PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
                 vector    BLOB NOT NULL
             );
             -- The reconciliation base: the checksum a record had when this store and one particular
             -- file last agreed (documents/design/multi-set-sync.md). Keyed by target as well as by
             -- memory, because one store serves every checkout of a repository and two checkouts are
             -- two files at two commits -- a single base per record could not describe both.
             --
             -- Local derived state, never written to a memory file. Losing it is recoverable: every
             -- record reads as divergent until the next export re-establishes agreement.
             --
             -- **Deliberately no foreign key.** Every other child table here cascades from
             -- memories(id), and copying that was a defect: `write_memory` upserts with INSERT OR
             -- REPLACE, which deletes the old row, so a cascade would destroy the base every time a
             -- memory was rewritten -- turning a one-sided amendment into a conflict on the next
             -- import. A base is not a property of the memory row; it records an agreement between
             -- this store and one file, and it has to survive the memory being rewritten in place.
             --
             -- A base whose memory is gone entirely is not garbage either: it means the two sides
             -- once agreed a record existed and the store no longer holds it, which reconciliation
             -- reports as an anomaly rather than guessing (documents/design/multi-set-sync.md).
             -- A forgotten memory. The memory row is deleted outright -- cascading its keywords,
             -- links and vector -- because a principal reason to forget is that sensitive data
             -- reached the memory, and a flagged row that still holds the text removes nothing.
             -- What survives is the id and the attribution of the act.
             -- `forgot` is the checksum of the record this tombstone replaced, as hex, and is
             -- NULL where there was nothing to checksum: a forget of an id this store did not
             -- hold, or a tombstone written before the column existed. It is what lets an import
             -- tell a sequential forget from a concurrent one with no base to judge by
             -- (documents/design/multi-set-sync.md).
             CREATE TABLE IF NOT EXISTS tombstones (
                 id           TEXT PRIMARY KEY,
                 forgotten_by TEXT NOT NULL,
                 forgotten_at TEXT NOT NULL,
                 forgot       TEXT,
                 file_position INTEGER
             );
             CREATE TABLE IF NOT EXISTS bases (
                 target    TEXT NOT NULL,
                 memory_id TEXT NOT NULL,
                 checksum  TEXT NOT NULL,
                 PRIMARY KEY (target, memory_id)
             );
             -- What the export review has cleared: one row per memory, holding the checksum of the
             -- record as it stood when cleared (documents/design/export-review.md, *The procedure*).
             -- A memory changed since then no longer matches, so it is reviewed again; nothing
             -- outside the record can reopen it, since vectors are not part of the checksum. Like
             -- `bases`, it records an agreement rather than a property of the row, so it carries no
             -- foreign key and survives the memory being rewritten in place. Never exported.
             CREATE TABLE IF NOT EXISTS clearances (
                 memory_id TEXT PRIMARY KEY,
                 checksum  TEXT NOT NULL
             );
             -- Memories a change to the preferences has put back into review, whatever their
             -- clearance or the file says, until they are cleared again. Kept here rather than in a
             -- session so a review that stops part-way picks up where it left off.
             CREATE TABLE IF NOT EXISTS reopened (
                 memory_id TEXT PRIMARY KEY
             );
             -- What this store was asked, so a non-match floor can be calibrated from its own
             -- traffic rather than from a constant that is wrong on its face
             -- (documents/design/recall-mechanisms.md). Written by a session's recall and read by
             -- nothing yet: a log only answers questions about the period it was running, so it
             -- starts before the thing that needs it.
             --
             -- **No query text, ever.** The score is what calibration needs; the text is the part
             -- that could make a copy of this file a disclosure. Nothing here is exported -- export
             -- streams memories and tombstones, and this is neither -- so the absence is structural
             -- rather than a rule someone has to remember.
             --
             -- `raw_top` is the best *uncentred* content similarity and `centred_top` the best
             -- score the ranking saw. Both, because they answer different questions and the
             -- measurement that chose between them can be re-run only if neither was discarded.
             -- `model` is here because a floor belongs to one store **and one model**: every set's
             -- floor moved 0.10 to 0.16 under a different one.
             -- `session` is what bounds the `used` label: a memory returned months ago by a
             -- query it did not answer must not be labelled by any later use of it, or the label
             -- saturates and the floor it calibrates sinks back toward the constant it replaced.
             -- It identifies one server process, which stands in for one session.
             CREATE TABLE IF NOT EXISTS query_log (
                 id          INTEGER PRIMARY KEY,
                 asked_at    TEXT NOT NULL,
                 model       TEXT NOT NULL,
                 session     TEXT NOT NULL DEFAULT '',
                 scoped      INTEGER NOT NULL,
                 raw_top     REAL NOT NULL,
                 centred_top REAL NOT NULL,
                 silenced    INTEGER NOT NULL,
                 used        INTEGER NOT NULL DEFAULT 0
             );
             -- What the session actually received, so that a later use of one of these memories can
             -- label the query answerable. **Deliberately no foreign key to memories**: a memory
             -- that is later forgotten must not take the record of what was returned with it, for
             -- the same reason `tombstones` outlives its memory -- cascade derived state, never a
             -- record of what happened. An id carries no text, so nothing is retained by keeping
             -- it.
             CREATE TABLE IF NOT EXISTS query_log_hits (
                 query     INTEGER NOT NULL REFERENCES query_log(id) ON DELETE CASCADE,
                 position  INTEGER NOT NULL,
                 memory_id TEXT NOT NULL,
                 PRIMARY KEY (query, position)
             );
             -- Labelling asks *which queries returned this memory*, which is the reverse of how the
             -- rows are keyed.
             CREATE INDEX IF NOT EXISTS query_log_hits_by_memory ON query_log_hits(memory_id);",
        )?;

        // **Only the local store carries the author map, and the condition is the design rather
        // than an optimisation.** *Which identity record is me in set X* is a fact about this
        // machine, and `local` is the one store with the right lifetime for it: machine-bound by
        // scope type, reached by every checkout, and unreachable by `drop`, which takes only a
        // shared set. Creating the table in every store would leave several copies of a
        // machine-wide fact, and nothing to say which one answers.
        if matches!(scope, Scope::Local) {
            connection.execute_batch(
                "CREATE TABLE IF NOT EXISTS author_map (
                     set_id        TEXT PRIMARY KEY,
                     author_record TEXT NOT NULL,
                     -- A label for a person pruning rows, never a key. A set's local name can
                     -- change and its identity cannot, so matching on this would be the failure
                     -- `a-hand-merge-into-a-new-set-name-leaves-supersessions-inert` records.
                     scope         TEXT NOT NULL
                 );",
            )?;
        }

        let mut store = Self {
            connection,
            scope,
            vectors: VectorSet::new(0),
            cache_built_at: None,
            embedder: None,
            opened_file: None,
        };
        store.drop_cascading_bases()?;
        store.drop_cascading_links()?;
        store.add_forgotten_checksum()?;
        store.add_link_target_set_id()?;
        // After the column it carries over exists, and before `compact_own_set_links` below, which
        // depends on the unique index this creates to know what it is narrowing onto.
        store.rekey_links_by_target_set()?;
        // After the table is known to have the current shape, and before anything reads a base.
        store.normalise_base_targets()?;
        store.add_folded_keywords()?;
        store.add_query_log_labels()?;
        store.compact_own_set_links()?;
        store.refold_keywords()?;
        store.reencode_ids()?;
        // After ids are in their canonical spelling, since the position is a hash of that spelling.
        store.fill_file_positions()?;
        store.unbind_model_if_no_vectors()?;
        store.bind_scope()?;
        store.refresh()?;
        Ok(store)
    }

    /// Put a connection into the shape a store shared between processes needs.
    ///
    /// **WAL, so that a reader is not blocked by a writer.** The default rollback journal gives a
    /// writer an exclusive lock over the whole database, so with two servers on one store an export
    /// blocks every recall in the other session for as long as it runs — and then, when the wait
    /// runs out, fails it. WAL lets readers carry on against the last committed state throughout.
    ///
    /// It is a property of the *file*, not of the connection, so it survives being set once and is
    /// inherited by every other process that opens the store. That also means it can legitimately
    /// not apply: an in-memory database has no file to keep a log beside and stays on its own
    /// journal mode. Requiring `wal` here would make [`Store::open_in_memory`] fail, and it is a
    /// single-connection store by definition — the sharing this guards against cannot arise.
    fn configure_for_sharing(connection: &Connection) -> Result<(), StoreError> {
        connection.busy_timeout(LOCK_WAIT)?;
        // Returns the mode it settled on, which is why this is a query rather than a statement. The
        // answer is deliberately not checked: see above.
        connection.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })?;
        Ok(())
    }

    /// Bring everything this store holds in memory back into agreement with the file.
    ///
    /// Cheap when nothing has happened: `PRAGMA data_version` is one read of a counter SQLite keeps
    /// per connection, and it changes only when a *different* connection has committed. A store
    /// alone with its file, or one that made the last write itself, reloads nothing.
    ///
    /// The version is read **before** the reload rather than after. A commit landing in between then
    /// costs one redundant reload on the next call, whereas reading it afterwards would record a
    /// write as already seen and leave the cache wrong until the one after that.
    fn refresh(&mut self) -> Result<(), StoreError> {
        let version: i64 = self
            .connection
            .query_row("PRAGMA data_version", [], |row| row.get(0))?;
        if self.cache_built_at == Some(version) {
            return Ok(());
        }
        self.load_vectors()?;
        self.cache_built_at = Some(version);
        Ok(())
    }

    /// Bring this store's derived state into agreement before it is searched by meaning.
    ///
    /// Two different kinds of staleness, kept as two functions because they are two claims: the
    /// in-memory cache can lag another process's writes, and the vectors themselves can belong to a
    /// model that is no longer the one in hand. Both are "derived state that no longer matches what
    /// it was derived from", and both have to be settled before an answer means anything.
    fn prepare(&mut self) -> Result<(), StoreError> {
        self.refresh()?;
        self.reconcile_embeddings()
    }

    /// Make every vector in this store belong to the model currently loaded.
    ///
    /// The single mechanism behind three cases, which is the point rather than a coincidence — a
    /// store that has just gained a model, one whose model was swapped, and one holding memories
    /// written during a gap are all *memories with no vector for the model in hand*.
    ///
    /// Automatic and unasked-for because there is no decision in it: under
    /// `documents/design/decision-placement.md`, a choice whose answer is always "make them
    /// consistent" should not be a choice, and neither the LLM nor the human holds information that
    /// would justify leaving a store half-embedded.
    ///
    /// Deliberately **not** called from [`Store::open_at`]. Embedding at open is exactly what the
    /// in-memory vector cache exists to avoid, so this runs on first use instead. Where there is
    /// nothing to do — the overwhelmingly common case — it costs one query and never touches the
    /// model. It runs before a search by meaning and not on a write, since 2026-09-22.
    fn reconcile_embeddings(&mut self) -> Result<(), StoreError> {
        if self.embedder.is_none() {
            // Nothing to rebuild with. The store keeps whatever it has, and `insert` refuses the
            // writes that would destroy it.
            return Ok(());
        }

        // A mismatch is deliberately **not** resolved here, since 2026-08-09. Rebuilding on use is
        // what let two servers with different models each undo the other's work on every read,
        // indefinitely — measured, not predicted. Choosing which model should win needs the other
        // server's configuration, which neither process holds, so it goes to `rebuild_vectors`
        // where a caller asks for it. `documents/design/embeddings.md`.
        //
        // There is no branch for it here on purpose: with the delete below confined to unrecorded
        // vectors, a mismatched store falls through to `embed_all`, which refuses on the same
        // condition. A second check would be a second copy of one rule — and mutation testing found
        // exactly that, by removing an earlier guard here and changing nothing.

        // **Vectors of unknown provenance are no longer rebuilt here**, since 2026-08-20. They were,
        // and the argument was that unlike a mismatch it *converges*: whichever process arrives
        // first resolves it. What settles it the other way is that the state has **no live
        // producer** — the identity is recorded before the vectors that bind it, in the one place
        // vectors are written, and nothing deletes it — so only a store predating 2026-08-08 or a
        // hand-edited database can be in it. Keeping a read that rewrites the whole vector table,
        // permanently, to serve a population that can only shrink is the wrong trade; the way out
        // is `rebuild_vectors`, which a caller asks for. `documents/design/embeddings.md`.
        //
        // Rebuilt rather than adopted when that command does run: adopting means writing this
        // model's identity over vectors it did not produce, which makes a guess permanent and
        // unverifiable.

        let missing = self.memories_without_vectors()?;
        if missing.is_empty() {
            return Ok(());
        }

        // Batched through the same path an ordinary write takes, so there is one definition of what
        // embedding a memory means and the recorded model identity is updated by the same code.
        let memories: Vec<&Memory> = missing.iter().collect();
        self.embed_all(&memories)?;
        Ok(())
    }

    /// Every memory this store holds that has no vector and could have one.
    fn memories_without_vectors(&self) -> Result<Vec<Memory>, StoreError> {
        let mut memories = Vec::new();
        for id in self.gaps()? {
            if let Some(memory) = self.get(MemoryId::parse(&id)?)? {
                memories.push(memory);
            }
        }
        Ok(memories)
    }

    /// The ids of every memory with no vector row, identity records aside.
    ///
    /// **Identity records are excluded here, in the one query both callers share**, since
    /// 2026-09-22. They are never embedded (`embed_for`), so they never gain a row, and while this
    /// query counted them the sweep found one on every call, loaded it, and threw it away -- it never
    /// reached the empty result that lets it cost a single query. `awaiting_vectors` filtered them
    /// separately, after loading every gap, which was a second definition of the same thing. The
    /// test is structural, as [`Memory::is_identity_record`] is: a record attributed to itself.
    fn gaps(&self) -> Result<Vec<String>, StoreError> {
        // **No row, not no vector.** A model's verdict that a memory has nothing to represent is a
        // row, so the join counts it as done — which is what stops the sweep asking again about a
        // memory whose answer only a new text or a new model could change. A genuine gap, written
        // by a process that could not supply this store's model, still has no row and is found.
        let mut statement = self.connection.prepare(
            "SELECT m.id FROM memories m
             LEFT JOIN vectors v ON v.memory_id = m.id
             WHERE v.memory_id IS NULL AND m.created_by <> m.id
             ORDER BY m.id",
        )?;
        let mapped = statement.query_map([], |row| row.get::<_, String>(0))?;
        Ok(mapped.collect::<Result<_, _>>()?)
    }

    /// Whether rewriting this memory would discard a vector nothing here could rebuild.
    ///
    /// `write_memory` upserts with `INSERT OR REPLACE`, which deletes the memory row and cascades
    /// its vector away before reinserting. With a model that is invisible, because a replacement is
    /// produced in the same call. Without one it is permanent, silent, and unrecoverable from the
    /// tool surface — which is `amending-without-a-model-destroys-the-embedding`.
    /// Extended 2026-08-09 from *is there a model* to *can this store's own model be reproduced*.
    /// A mismatched process has an embedder, so the old condition let a rewrite through — and once
    /// the mismatch stopped resolving itself on use, that rewrite would have discarded a vector and
    /// replaced it with one from a different space, which is the mixture the design forbids.
    fn would_destroy_an_unrebuildable_vector(&self, memory: &Memory) -> Result<bool, StoreError> {
        if self.can_rebuild_in_this_stores_model()? {
            return Ok(false);
        }
        self.is_findable_by_meaning(memory.id)
    }

    /// Whether this process could produce a replacement vector belonging to the store's own model.
    ///
    /// Deliberately not *has an embedder*: a model that is not this store's cannot rebuild anything
    /// for it, however capable it is on its own terms.
    fn can_rebuild_in_this_stores_model(&self) -> Result<bool, StoreError> {
        if self.embedder.is_none() {
            return Ok(false);
        }
        Ok(!matches!(
            self.model_state()?,
            ModelState::Mismatched { .. }
        ))
    }

    /// Give every memory and tombstone its stored [`crate::memory_file::file_position`].
    ///
    /// **Stored so the database orders an export**, not the process: `stream_records` reads both
    /// tables in one `ORDER BY` and holds one record at a time (`file-position-column`). Before
    /// this, it collected every id and every whole tombstone and sorted them in memory.
    ///
    /// **Nullable, and filled on every open where it is missing**, which is the migration and a
    /// repair in one statement. `ALTER TABLE` cannot add a `NOT NULL` column without a default, and
    /// a default is worse than no value: a row given `0` would sort to a wrong place quietly, where a
    /// missing value is filled here or refused by `stream_records`, which checks every position it
    /// streams against the id it belongs to. The check at the reading end is what the constraint
    /// would have been at the writing end.
    ///
    /// The ids to fill are collected before any is written, rather than updated while the query
    /// walks the table — SQLite leaves a table's iteration undefined under updates to it. That
    /// holds sixteen bytes a missing row, once.
    fn fill_file_positions(&self) -> Result<(), StoreError> {
        for table in ["memories", "tombstones"] {
            let present: bool = self
                .connection
                .prepare(&format!(
                    "SELECT 1 FROM pragma_table_info('{table}') WHERE name = 'file_position'"
                ))?
                .exists([])?;
            if !present {
                self.connection.execute(
                    &format!("ALTER TABLE {table} ADD COLUMN file_position INTEGER"),
                    [],
                )?;
            }

            let missing: Vec<String> = {
                let mut statement = self.connection.prepare(&format!(
                    "SELECT id FROM {table} WHERE file_position IS NULL"
                ))?;
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<_, _>>()?
            };
            if !missing.is_empty() {
                let transaction = self.connection.unchecked_transaction()?;
                for key in missing {
                    // An id this build cannot parse keeps no position, and `stream_records` refuses
                    // the store naming nothing better than the order breaking. It would be refused
                    // by `get` in any case, so there is nothing a position could rescue.
                    let Ok(id) = MemoryId::parse(&key) else {
                        continue;
                    };
                    transaction.execute(
                        &format!("UPDATE {table} SET file_position = ?1 WHERE id = ?2"),
                        params![stored_position(id), key],
                    )?;
                }
                transaction.commit()?;
            }

            // Owned here rather than in the schema batch, for the reason `add_folded_keywords`
            // gives: on an older store the batch runs before the column exists.
            self.connection.execute(
                &format!(
                    "CREATE INDEX IF NOT EXISTS {table}_by_file_position \
                     ON {table}(file_position, id)"
                ),
                [],
            )?;
        }
        Ok(())
    }

    /// Bring a store written before keywords kept their case up to the current schema.
    ///
    /// `CREATE TABLE IF NOT EXISTS` cannot add a column to a table that already exists, so an older
    /// store opens with no `folded` column and every keyword search silently matches nothing. The
    /// backfill is a straight copy because it is *exactly* correct for such a store: every keyword
    /// in it went through the old constructor, so its stored spelling already is its folded form.
    ///
    /// Worth the migration rather than the documented drop-and-reimport, because the local store is
    /// the only copy of local memories and neither backup nor restore exists yet
    /// (`documents/design/set-lifecycle.md`).
    fn add_folded_keywords(&self) -> Result<(), StoreError> {
        let present: bool = self
            .connection
            .prepare("SELECT 1 FROM pragma_table_info('keywords') WHERE name = 'folded'")?
            .exists([])?;

        if !present {
            self.connection.execute(
                "ALTER TABLE keywords ADD COLUMN folded TEXT NOT NULL DEFAULT ''",
                [],
            )?;
            self.connection
                .execute("UPDATE keywords SET folded = keyword", [])?;
        }

        // Owned here for new and migrated stores alike, and deliberately not in the schema batch:
        // that runs first, and on an older store it would index a column that does not exist yet —
        // failing the open rather than the search.
        self.connection.execute(
            "CREATE INDEX IF NOT EXISTS keywords_by_folded ON keywords(folded)",
            [],
        )?;
        Ok(())
    }

    /// Rebuild the keyword lookup keys when the folding rule has changed.
    ///
    /// `folded` is derived state, and the rule deriving it moved from `to_lowercase` to
    /// `toNFKC_Casefold` (`folding-is-lowercase-not-casefold`). A store written under the old rule
    /// holds keys that the query path — which folds under the new rule — can no longer match, so
    /// every keyword search against it would silently return less than it should.
    ///
    /// **Guarded by a marker in `meta` rather than run unconditionally.** Recomputing is
    /// idempotent, so an unconditional pass would be correct; it would also fold every keyword in
    /// the store on every open, for a rule that changes about once. The marker names the rule the
    /// column was built under, which is the same thing `embedding_model` does for vectors and for
    /// the same reason: derived state should say what derived it.
    fn refold_keywords(&self) -> Result<(), StoreError> {
        const RULE: &str = "nfkc_casefold";

        let recorded: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'keyword_fold'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if recorded.as_deref() == Some(RULE) {
            return Ok(());
        }

        let rows: Vec<(String, String)> = {
            let mut statement = self
                .connection
                .prepare("SELECT DISTINCT keyword, folded FROM keywords")?;
            let mapped = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            mapped.collect::<Result<_, _>>()?
        };

        for (keyword, stored) in rows {
            // Through `Keyword` rather than folding the string here, so there is one rule and this
            // cannot drift from the one the query path uses -- which is the drift that made the
            // migration necessary in the first place.
            let Ok(parsed) = Keyword::new(keyword.clone()) else {
                continue;
            };
            let current = parsed.folded();
            if current != stored {
                self.connection.execute(
                    "UPDATE keywords SET folded = ?1 WHERE keyword = ?2",
                    params![current, keyword],
                )?;
            }
        }

        self.connection.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('keyword_fold', ?1)",
            params![RULE],
        )?;
        Ok(())
    }

    /// Rewrite links that name this store's own set into the compact same-set form.
    ///
    /// The data half of `a-same-set-link-spelled-with-its-set-name-is-inert`. Writes are
    /// canonicalised by [`write_memory`] from now on, but a store imported before that fix can hold
    /// rows in the long form, and those rows are **inert**: every lookup that decides standing keys
    /// on `target_set IS NULL`, so a supersession spelled the long way silently does nothing. A
    /// store is not re-imported often enough for the write-side fix to reach it on its own.
    ///
    /// Runs on every open rather than behind a schema check, because there is no column to test for
    /// -- the condition is a value, not a shape. It is cheap and idempotent: after the first pass
    /// the `WHERE` matches nothing, and on the overwhelmingly common store it matches nothing ever.
    ///
    /// **Only ever narrows `target_set` to null, and only where it equals this store's own name.**
    /// A genuinely foreign link is untouched, and the local store has no name for anything to
    /// equal, so this is a no-op there rather than a special case.
    fn compact_own_set_links(&self) -> Result<(), StoreError> {
        let Some(own) = self.scope.exports_to() else {
            return Ok(());
        };
        // **`OR REPLACE`, because narrowing can now land on a row that already exists.** Once
        // `target_set` is part of the key, a store may legitimately hold both the long spelling of a
        // same-set link and its compact form; narrowing the first onto the second violates
        // `links_same_set`, and a plain `UPDATE` would fail the open rather than the write. The two
        // rows say the same thing, so replacing is the meaning of the operation rather than a way
        // past the constraint. Before the rekey this could not arise: the narrower key had already
        // merged the two on the way in.
        self.connection.execute(
            "UPDATE OR REPLACE links SET target_set = NULL, target_keywords = NULL \
             WHERE target_set = ?1",
            params![own.as_str()],
        )?;
        Ok(())
    }

    /// Give this store an embedder, enabling search by meaning.
    ///
    /// Optional on purpose. A store without one still holds and returns memories, still traverses
    /// links, and still resolves supersession — [`Store::recall`] reports the missing capability
    /// rather than failing or, worse, quietly returning nothing.
    ///
    /// Shared rather than owned, because every store a session loads has to be embedded by the
    /// **same** model. [`recall_across`] embeds the query once, using whichever embedder it finds
    /// first, and scores that vector against every store's vectors; two different models would make
    /// those comparisons meaningless with nothing failing to say so. `Arc` also means one loaded
    /// model serves every store, which is what makes giving each of them one affordable.
    pub fn set_embedder(&mut self, embedder: Arc<dyn Embedder>) {
        self.embedder = Some(embedder);
    }

    /// Whether this store can be searched by meaning.
    ///
    /// Exists so a caller that is *not* running a semantic search can still report the capability
    /// honestly. Keyword-only recall used to hard-code the answer to `true`, because the value was
    /// computed inside the semantic path and the keyword path had nothing to read — telling a
    /// caller with no model installed that search by meaning was available, on the one call that
    /// still worked.
    /// Asked of the model state rather than of `self.embedder`, since 2026-08-09. Holding an
    /// embedder says a model exists, not that *this store* can be searched with it — which was the
    /// same non-answer as the server-wide flag this replaced, one level down, and reported `true`
    /// on a mismatched store where the embedder present is the wrong one.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the recorded model cannot be read.
    pub fn can_search_by_meaning(&self) -> Result<bool, StoreError> {
        if self.embedder.is_none() {
            return Ok(false);
        }
        // Neither a mismatch nor unknown provenance is searchable, and since 2026-08-20 neither is
        // resolved by a read either. A wrong model is at least a known space; an unrecorded one
        // cannot be compared against anything at all, so searching it would rank on numbers with no
        // meaning. Both are reported per set by `why_unsearchable`, with the remedy named.
        Ok(!matches!(
            self.model_state()?,
            ModelState::Mismatched { .. } | ModelState::ProvenanceUnknown { .. }
        ))
    }

    /// Why this store cannot be searched by meaning, phrased for the caller.
    ///
    /// **The reason differs by state and so does the remedy**, which is why a recall names it per
    /// set rather than reporting one sentence for the whole union: a store with no model needs one
    /// installed, and a store whose vectors came from a different model needs someone to decide
    /// which model should win. Reporting them alike is what sent a caller to `status` to find out
    /// something the failing call already knew.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn why_unsearchable(&self) -> Result<String, StoreError> {
        Ok(match self.model_state()? {
            ModelState::Current { .. } | ModelState::Unembedded if self.embedder.is_none() => {
                "no embedding model is loaded in this server, so this set has nothing to search \
                 with. Keyword search, hoard_lookup and traversal are unaffected."
                    .to_owned()
            }
            ModelState::Unembedded => {
                "this set holds no vectors yet, so there is nothing to search by meaning."
                    .to_owned()
            }
            ModelState::ProvenanceUnknown { vectors } => format!(
                "this set holds {vectors} vectors and no record of which model produced them, so \
                 they cannot be compared against anything. Rebuild them with `hoard_sets` action \
                 \"reembed\" and this scope."
            ),
            // Not a choice between two models, so not the remedy for one: `reembed` with nothing
            // loaded is refused for having nothing to rebuild with.
            ModelState::Mismatched {
                recorded,
                loaded: None,
            } => format!(
                "this set's vectors were produced by `{recorded}`, and no embedding model is loaded \
                 in this server, so there is nothing to search them with. Make `{recorded}` \
                 loadable again: this server reads its model from `SCATTERHOARD_MODEL`, or from \
                 `model` in its data directory. Keyword search, hoard_lookup and traversal are \
                 unaffected."
            ),
            ModelState::Mismatched {
                recorded,
                loaded: Some(loaded),
            } => format!(
                "this set's vectors were produced by {recorded}, and this server has {loaded}. \
                 Searching it would compare across two vector spaces and return confident \
                 nonsense. Decide which model should win: `hoard_sets` action \"reembed\" and this \
                 scope rebuilds with the model this server holds."
            ),
            ModelState::Current { .. } => {
                "this set is searchable; nothing should have skipped it.".to_owned()
            }
        })
    }

    /// Whether one particular memory can be found by meaning.
    ///
    /// Distinct from [`Store::can_search_by_meaning`], which is about the store's *capability*.
    /// This asks whether the vector actually exists, and the difference is the whole point: a
    /// memory written while no model was loaded stays unfindable by meaning afterwards, however
    /// capable the store becomes later.
    ///
    /// Answered from the `vectors` table rather than from `self.embedder`, so it reports the
    /// postcondition instead of restating the branch [`Store::embed_all`] took. Inferring it from
    /// the embedder would be the same mistake as a server-wide capability flag: correct on the
    /// path anyone thought about, and silent about every other way a vector could fail to arrive.
    /// That is what made `project-stores-get-no-embedder` invisible.
    ///
    /// Reads SQL, so it needs no refresh and takes `&self` — see the note on [`Store`].
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the query fails.
    pub fn is_findable_by_meaning(&self, id: MemoryId) -> Result<bool, StoreError> {
        // A row is not enough: a model's verdict that there is nothing to represent is a row too.
        let found: Option<i64> = self
            .connection
            .query_row(
                "SELECT 1 FROM vectors WHERE memory_id = ?1 AND length(vector) > 0",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Whether this store's model has looked at this memory and found nothing it can represent.
    ///
    /// **The other reason a memory is not findable by meaning, and the caller has to be told which.**
    /// With no model, the remedy is to install one. Here a model is installed and has no tokens for
    /// the memory's language — and the tempting remedy, adding keywords, would give the memory a
    /// vector built from those keywords alone, which is not a representation of what it says.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn is_unrepresentable(&self, id: MemoryId) -> Result<bool, StoreError> {
        let found: Option<i64> = self
            .connection
            .query_row(
                "SELECT 1 FROM vectors WHERE memory_id = ?1 AND length(vector) = 0",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Rebuild the in-memory vector set from the persisted one.
    ///
    /// **Discards what it holds first**, which it did not always do: it only ever inserted, so a
    /// vector whose memory had been deleted stayed behind, and `forget`'s reload — commented as a
    /// rebuild — removed nothing. Nothing leaked the content, because resolving a hit reads SQL and
    /// finds none; what leaked was scan width. Recall scans a fixed multiple of the requested limit
    /// and drops the hits that resolve to nothing, so enough departed vectors ranking ahead of a
    /// surviving memory pushed it off the end of the scan and recall answered with nothing at all.
    /// The same shape as `forget`, arrived at from outside, is a deletion by another process.
    fn load_vectors(&mut self) -> Result<(), StoreError> {
        let rows: Vec<(String, Vec<u8>)> = {
            let mut statement = self
                .connection
                .prepare("SELECT memory_id, vector FROM vectors")?;
            let mapped = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            mapped.collect::<Result<_, _>>()?
        };

        self.vectors = VectorSet::new(0);
        for (id, blob) in rows {
            // A model's verdict that the memory has nothing it can represent, not a vector.
            if blob.is_empty() {
                continue;
            }
            let components = decode_vector(&blob);
            if self.vectors.is_empty() {
                self.vectors = VectorSet::new(components.len());
            }
            self.vectors
                .insert(MemoryId::parse(&id)?, Vector::new(components)?)?;
        }
        Ok(())
    }

    /// Embed some memories and record what the model made of each, if an embedder is available.
    ///
    /// **Every outcome is recorded, not only vectors.** A memory the model has no tokens for gets a
    /// row too — [`UNREPRESENTABLE`] — because the gap sweep finds its work by looking for memories
    /// with *no* row. Without one it would ask again, on every read and write, about a memory whose
    /// answer cannot change: only a new text or a new model could change it, and each already
    /// re-embeds through its own path. For a store written mostly in a script its model lacks, that
    /// is re-embedding the whole store on every call. `one-unembeddable-memory-stops-the-store`.
    fn embed_all(&mut self, memories: &[&Memory]) -> Result<Outcomes, StoreError> {
        let Some(embeddings) = self.embed_for(memories)? else {
            return Ok(Outcomes::default());
        };
        self.persist(None, Some(embeddings))
    }

    /// Commit a written memory together with what the model made of it — or, for memories already
    /// stored, only the model's outcomes — as one transaction, then bring the in-memory set into line.
    ///
    /// **The one place a row enters `vectors`.** Every path comes through here, so the model is
    /// always recorded before the rows it binds, and a written memory never lands without the row it
    /// was embedded for.
    fn persist(
        &mut self,
        memory: Option<&Memory>,
        embeddings: Option<Embeddings>,
    ) -> Result<Outcomes, StoreError> {
        if let Some(embeddings) = &embeddings {
            // Recorded before the rows it binds, so no row ever exists under an unrecorded model.
            self.record_model(&embeddings.model)?;
        }
        // Read before the transaction borrows the connection. The set has already claimed its
        // identity by now, in `write`, so even its first write is canonicalised by identity.
        let holder = self.set_id()?;
        let transaction = self.connection.transaction()?;
        if let Some(memory) = memory {
            write_memory(&transaction, memory, &self.scope, holder)?;
        }
        if let Some(embeddings) = &embeddings {
            write_embeddings(&transaction, &embeddings.outcomes)?;
        }
        transaction.commit()?;
        match embeddings {
            Some(embeddings) => self.cache(embeddings),
            None => Ok(Outcomes::default()),
        }
    }

    /// What this store's model makes of some memories, without writing anything.
    ///
    /// `None` when there is nobody to ask or nothing to ask about — which is why a write can call
    /// this *before* committing, and keep the memory and its row in one transaction.
    ///
    /// **Identity records are never embedded, so they cannot surface in a search by meaning.**
    /// Semantic recall answers *what do I know about this*, and a record naming a contributor is
    /// not knowledge about anything -- it is set infrastructure. Left embedded it competes with
    /// real memories in every recall, and every set holds at least one, so the cost is universal
    /// rather than occasional. They stay reachable by the keyword `hoard-author`, which is how
    /// `documents/design/memory-data-model.md` says to find them, and `is_findable_by_meaning`
    /// honestly reports `false` for them.
    ///
    /// Excluded here rather than filtered at recall because this is one place instead of every
    /// read path, and because a vector that is never written cannot be returned by a path somebody
    /// adds later.
    fn embed_for(&self, memories: &[&Memory]) -> Result<Option<Embeddings>, StoreError> {
        let Some(embedder) = &self.embedder else {
            return Ok(None);
        };
        let memories: Vec<&Memory> = memories
            .iter()
            .copied()
            .filter(|memory| !memory.is_identity_record())
            .collect();
        if memories.is_empty() {
            return Ok(None);
        }

        // A mismatched store keeps its own model's vectors, so a new memory waits unembedded rather
        // than joining them from a different space. The narrowing the design already applies to a
        // missing model: the memory is stored, nothing is destroyed, and an explicit rebuild
        // resolves it. Checked here rather than at each caller so no write path can miss it.
        if matches!(self.model_state()?, ModelState::Mismatched { .. }) {
            return Ok(None);
        }

        let inputs: Vec<String> = memories.iter().map(|m| embedding_input(m)).collect();
        // `embed_each`, never `embed`: the strict form fails the whole batch on one text the model
        // cannot represent, and the sweep sends every gap through one batch.
        let outcomes = embedder.embed_each(&inputs)?;

        // Recorded by whoever writes these rather than when the embedder is set, because *holding* a
        // model is not what binds a store to one -- producing an outcome is. A store handed an
        // embedder that never embeds anything stays unbound, which keeps it freely writable.
        Ok(Some(Embeddings {
            model: embedder.identity(),
            dimension: embedder.dimension(),
            outcomes: memories
                .iter()
                .map(|memory| memory.id)
                .zip(outcomes)
                .collect(),
        }))
    }

    /// Bring the in-memory vector set into line with rows just written.
    ///
    /// **A verdict removes as well as adding nothing.** A rewrite whose new text the model cannot
    /// represent must not go on being found by the vector its old text had.
    fn cache(&mut self, embeddings: Embeddings) -> Result<Outcomes, StoreError> {
        if self.vectors.is_empty() {
            self.vectors = VectorSet::new(embeddings.dimension);
        }
        let mut outcomes = Outcomes::default();
        for (id, outcome) in embeddings.outcomes {
            match outcome {
                Embedded::Vector(vector) => {
                    self.vectors.insert(id, vector)?;
                    outcomes.vectors += 1;
                }
                Embedded::Unrepresentable => {
                    self.vectors.remove(id);
                    outcomes.unrepresentable += 1;
                }
            }
        }
        Ok(outcomes)
    }

    /// Release the recorded model when no vectors remain.
    ///
    /// **Decided 2026-08-20: if there are no vectors, there is no model.** The recorded identity is
    /// a property *of the vectors*, so with none there is nothing for it to be a property of, and a
    /// store claiming a model it holds no evidence for is the state that made `import` on a
    /// modelless server report `Mismatched` with nothing to mismatch — and then refuse to fill its
    /// own gap when a model returned.
    ///
    /// Called wherever vectors can be emptied, and again at open, which repairs any store already
    /// left in that state. Maintenance and repair are the same rule, so they are the same code.
    ///
    /// The history is deliberately left alone: it records what this store has held, and losing its
    /// last vectors does not unmake that.
    fn unbind_model_if_no_vectors(&self) -> Result<(), StoreError> {
        if self.vector_count()? == 0 {
            self.connection
                .execute("DELETE FROM meta WHERE key = 'embedding_model'", [])?;
        }
        Ok(())
    }

    /// Which model this store's vectors came from, against the one in hand.
    ///
    /// The three states `documents/design/embeddings.md` distinguishes. The useful question is not
    /// whether the store has vectors but whether this process can maintain what it holds, and that
    /// is a comparison rather than a property — which is why it needs the recorded identity and not
    /// just the presence of an embedder.
    ///
    /// Reads SQL, so it needs no refresh and takes `&self` — see the note on [`Store`].
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the query fails.
    pub fn model_state(&self) -> Result<ModelState, StoreError> {
        let recorded: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'embedding_model'",
                [],
                |row| row.get(0),
            )
            .optional()?;

        let Some(recorded) = recorded.map(ModelIdentity::new) else {
            // No recorded model is two different situations, and reporting them alike is what
            // `embedded-store-can-report-itself-unembedded` was: a store that has genuinely never
            // embedded anything, and one whose vectors predate the identity being written down.
            // Only the first has nothing to lose, so the vector count is what separates them.
            let vectors = self.vector_count()?;
            return Ok(if vectors == 0 {
                ModelState::Unembedded
            } else {
                ModelState::ProvenanceUnknown { vectors }
            });
        };
        let loaded = self.embedder.as_ref().map(|embedder| embedder.identity());
        if loaded.as_ref() == Some(&recorded) {
            return Ok(ModelState::Current { model: recorded });
        }
        Ok(ModelState::Mismatched { recorded, loaded })
    }

    /// Every embedded memory in this set, as the summariser needs to see it.
    ///
    /// A memory with no vector is left out — it has no position in the space the clustering works
    /// in. A memory with no keywords is kept, because it still counts toward a cluster's size even
    /// though it can contribute nothing to the label.
    ///
    /// **Both queries order explicitly, and that is load-bearing rather than tidy.** Clustering
    /// depends on the order its input arrives in, through the seeding draw and through every
    /// tie-break; unordered SQLite rows come back in whatever order the pages happen to hold, so two
    /// stores holding identical memories could summarise them differently. The export-versus-import
    /// comparison would then disagree on data nobody had touched.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the vectors or keywords cannot be read.
    pub fn summary_subjects(&self) -> Result<Vec<crate::summary::Subject>, StoreError> {
        Ok(self
            .summary_subjects_by_id()?
            .into_iter()
            .map(|(_, subject)| subject)
            .collect())
    }

    /// [`Store::summary_subjects`], each beside the id of the memory it came from, for the export
    /// review, which groups memories by subject and so has to know which is which.
    ///
    /// # Errors
    ///
    /// As for [`Store::summary_subjects`].
    pub fn summary_subjects_by_id(
        &self,
    ) -> Result<Vec<(MemoryId, crate::summary::Subject)>, StoreError> {
        let mut keywords: HashMap<String, Vec<crate::memory::Keyword>> = HashMap::new();
        {
            let mut statement = self
                .connection
                .prepare("SELECT memory_id, keyword FROM keywords ORDER BY memory_id, keyword")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, keyword) = row?;
                if let Ok(keyword) = crate::memory::Keyword::new(keyword) {
                    keywords.entry(id).or_default().push(keyword);
                }
            }
        }

        let mut statement = self
            .connection
            .prepare("SELECT memory_id, vector FROM vectors ORDER BY memory_id")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;

        let mut subjects = Vec::new();
        for row in rows {
            let (id, blob) = row?;
            let Ok(vector) = Vector::new(decode_vector(&blob)) else {
                continue;
            };
            let parsed = MemoryId::parse(&id)?;
            let subject = crate::summary::Subject::new(
                &parsed,
                vector,
                keywords.remove(&id).unwrap_or_default(),
            );
            subjects.push((parsed, subject));
        }
        Ok(subjects)
    }

    /// The identity of the embedding model this set's vectors were recorded under.
    ///
    /// [`Store::model_state`] answers a different question — whether *this process* can maintain
    /// them. This is the bare recorded identity, which is what a summary carries so that a later
    /// reader can tell whether its clusters are comparable with a fresh derivation.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the recorded model cannot be read.
    pub fn recorded_model(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'embedding_model'",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The languages this set's charter declares, or `None` where the set has no live charter.
    ///
    /// Read from every live charter rather than one, because nothing yet stops a set holding two
    /// (`one-live-charter-per-set`), and choosing between them would hide what the other declares.
    /// Keyword search returns only memories that carry the charter mark themselves, so a charter
    /// superseded by a memory without it has stopped declaring anything.
    ///
    /// Tags are reported in the caseless form keywords are matched by, so `hoard-lang-zh-Hans` and
    /// `hoard-lang-zh-hans` are one declaration.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the charter cannot be read.
    pub fn declared_languages(&self) -> Result<Option<Vec<String>>, StoreError> {
        let charters = self.search_keywords(&[Keyword::new(CHARTER_KEYWORD)?], usize::MAX)?;
        if charters.is_empty() {
            return Ok(None);
        }
        let mut tags = std::collections::BTreeSet::new();
        for charter in &charters {
            for keyword in &charter.memory.keywords {
                if let Some(tag) = keyword.folded().strip_prefix(LANGUAGE_KEYWORD_PREFIX)
                    && !tag.is_empty()
                {
                    tags.insert(tag.to_owned());
                }
            }
        }
        Ok(Some(tags.into_iter().collect()))
    }

    /// How many memories here have no vector yet, identity records aside.
    ///
    /// A derived description covers only memories with a position in the space it clusters, so this
    /// is how a caller knows whether one would describe the whole set. Identity records are left out
    /// because they are never embedded — see `Store::embed_for` — and counting them would leave
    /// every set permanently incomplete.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the memories cannot be read.
    pub fn awaiting_vectors(&self) -> Result<usize, StoreError> {
        Ok(self.gaps()?.len())
    }

    /// Embed whatever this store is missing, with the model it holds, without reading or writing
    /// anything else.
    ///
    /// The same catch-up every search by meaning runs first; exposed for a caller that needs the
    /// vectors to exist *before* its first search, which is otherwise the call that would make them.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn catch_up_vectors(&mut self) -> Result<(), StoreError> {
        self.prepare()
    }

    /// The description an import could not give, as the importer recorded it, if one is owed.
    ///
    /// Held as text the store does not interpret: what is owed, and how it is judged when it is paid,
    /// belong to the import gate, not to storage.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if it cannot be read.
    pub fn owed_description(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'owed_description'",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Record that this set is owed a description, replacing any earlier record.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if it cannot be written.
    pub fn owe_description(&self, record: &str) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('owed_description', ?1)",
            params![record],
        )?;
        Ok(())
    }

    /// Discharge an owed description.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if it cannot be written.
    pub fn discharge_description(&self) -> Result<(), StoreError> {
        self.connection
            .execute("DELETE FROM meta WHERE key = 'owed_description'", [])?;
        Ok(())
    }

    /// Record which model this store's vectors belong to, keeping the one it displaced.
    ///
    /// The history exists so a *return* to a model that already held this store can be recognised.
    /// That is the shape of two servers taking turns, and it is a claim about identity rather than
    /// about elapsed time — which matters because the reader is usually an LLM, for whom "four
    /// minutes ago" is a fact requiring interpretation and "this model held the store before" is
    /// not. `documents/design/embeddings.md`.
    ///
    /// Bounded, because it is a signal and not a log: only the recent past can tell an alternation
    /// from an ordinary sequence of upgrades, and an unbounded list would eventually call every
    /// model a repeat.
    fn record_model(&self, identity: &ModelIdentity) -> Result<(), StoreError> {
        let previous: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'embedding_model'",
                [],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(previous) = previous.filter(|previous| previous != identity.as_str()) {
            let mut history = self.model_history()?;
            history.push(previous);
            let excess = history.len().saturating_sub(MODEL_HISTORY);
            history.drain(..excess);
            self.connection.execute(
                "INSERT INTO meta (key, value) VALUES ('embedding_model_history', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![history.join("\n")],
            )?;
            // **Cleared where the vectors are, and for the same reason.** A floor's level is
            // entirely the model's -- every set's floor moved 0.10 to 0.16 under a different one --
            // so a level calibrated under the model being displaced describes nothing about the one
            // arriving. Only on an actual change: a rebuild under the same model displaces nothing
            // and the level still stands.
            self.clear_floor()?;
        }

        self.connection.execute(
            "INSERT INTO meta (key, value) VALUES ('embedding_model', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![identity.as_str()],
        )?;
        Ok(())
    }

    /// The models that have held this store before the current one, oldest first.
    fn model_history(&self) -> Result<Vec<String>, StoreError> {
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'embedding_model_history'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(raw
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    /// Rebuild every vector with the model this process holds, deliberately.
    ///
    /// The way out of a mismatch, and the only thing that resolves one now that reads do not. It
    /// **always proceeds**: the vectors it would protect are derived state rebuilt in well under a
    /// second, while refusing costs semantic recall itself and can strand a store its own tool
    /// surface cannot repair — decommission the other server and the model that remains is exactly
    /// the one a re-entry check would refuse.
    ///
    /// [`Rebuilt::seen_before`] carries the signal instead, so a caller can say the two
    /// configurations are fighting without anyone having to be stopped.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NoEmbedder`] if there is no model to rebuild with, and
    /// [`StoreError::Database`] if the rewrite fails.
    pub fn rebuild_vectors(&mut self) -> Result<Rebuilt, StoreError> {
        let Some(embedder) = self.embedder.clone() else {
            return Err(StoreError::NoEmbedder);
        };
        let model = embedder.identity();
        // **A reset displaces nothing.** Requesting the model the store already holds is legal and
        // rebuilds every vector, but reporting the same identity as *displaced* would describe a
        // change that did not happen -- and `record_model` agrees, keeping it out of the history for
        // the same reason. Without that agreement, resetting twice would accumulate into the
        // alternation count that exists to detect two servers fighting.
        let displaced = match self.model_state()? {
            ModelState::Current { model: held } | ModelState::Mismatched { recorded: held, .. } => {
                (held != model).then_some(held)
            }
            ModelState::Unembedded | ModelState::ProvenanceUnknown { .. } => None,
        };
        let seen_before = self
            .model_history()?
            .iter()
            .filter(|held| *held == model.as_str())
            .count();

        self.connection.execute("DELETE FROM vectors", [])?;
        self.vectors = VectorSet::new(0);
        // Before embedding, not after: `embed_all` refuses to write into a mismatched store, and
        // until the recorded model is this one the store still *is* mismatched. Recording it here
        // is what makes the rebuild the one sanctioned way through that guard.
        self.record_model(&model)?;

        // With every vector gone, "memories with no vector" is every memory. Reusing that query
        // rather than adding a second one keeps a single definition of what needs embedding.
        let memories = self.memories_without_vectors()?;
        let borrowed: Vec<&Memory> = memories.iter().collect();
        let embedded = self.embed_all(&borrowed)?;
        // A store with no memories rebuilds to nothing, and must not stay bound to a model on the
        // strength of a rebuild that produced no vectors.
        self.unbind_model_if_no_vectors()?;

        // **What this rebuild wrote, reported by the thing that wrote it.** Neither neighbour is
        // right: the input length is what the rebuild *intended*, and a memory the model cannot
        // represent is in it but gets a verdict rather than a vector, so it overstates by one for
        // each — while the table's own count includes anything a rebuild failed to clear, so it
        // would report a sweep that never happened as a success. Reporting an intention rather than an outcome is the shape that hid
        // `project-stores-get-no-embedder`; reporting a total rather than a delta hides a stale
        // vector surviving a model change, which is the mixture the design forbids.

        Ok(Rebuilt {
            vectors: embedded.vectors,
            unrepresentable: embedded.unrepresentable,
            model,
            displaced,
            seen_before,
        })
    }

    /// How many vectors this store holds, read from SQL rather than the in-memory set.
    ///
    /// **Verdicts count.** A row recording that the model had nothing to represent is that model's
    /// output as much as a vector is, so it binds the store to the model the same way. Counting
    /// only vectors would let a store holding nothing but such memories release its model under
    /// *no vectors, no model* — and a different model arriving later would then skip every one of
    /// them, because their rows exist.
    ///
    /// Deliberately the table and not `self.vectors`: this is asked while deciding whether the
    /// cached set can be trusted, so consulting the cache to answer it would be circular.
    fn vector_count(&self) -> Result<usize, StoreError> {
        let count: i64 = self
            .connection
            .query_row("SELECT count(*) FROM vectors", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Find the memories most like the question, walking this store's links outward from them.
    ///
    /// Supersession is a validity relation, not a relevance one, so a query matching a superseded
    /// memory returns the memory that replaced it — with the matched id reported, since otherwise
    /// the caller holds something it did not ask for and cannot tell why. **Resolution runs after
    /// the ranking rather than before it**, which is not what
    /// `documents/design/recall-mechanisms.md` describes; see the consequences recorded there and
    /// in `TRACKING.md`.
    ///
    /// **One store is the one-store case of [`recall_across`], not a separate path.** Centring a
    /// single store on the mean of its own vectors and centring it on the mean of a union of one
    /// are the same arithmetic, so delegating leaves nothing to drift: the walk, the cut and the
    /// suppression of already-known memories have exactly one implementation.
    ///
    /// The walk cannot leave this store, because a scoped recall's population is the memories it
    /// searched — the design's set-boundary rule falling out of [`adjacency_over`] rather than being
    /// stated a second time here.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    /// Takes `&mut self` because it reads the vector cache, which it must first bring up to date
    /// against writes made by other processes — see the note on [`Store`].
    pub fn recall(&mut self, question: &Question<'_>) -> Result<Recall, StoreError> {
        recall_across(&mut [self], question)
    }

    /// What this store was asked, oldest first.
    ///
    /// Nothing reads it yet: it is the evidence a non-match floor will be calibrated from, and a log
    /// can only answer questions about the period it was running, so it starts before the thing
    /// that needs it. `documents/design/recall-mechanisms.md`.
    ///
    /// Reads SQL, so it needs no refresh and takes `&self` — see the note on [`Store`].
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn query_log(&self) -> Result<Vec<LoggedQuery>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, asked_at, model, scoped, raw_top, centred_top, silenced, used
             FROM query_log ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, bool>(7)?,
            ))
        })?;

        let mut log = Vec::new();
        for row in rows {
            let (id, asked_at, model, scoped, raw_top, centred_top, silenced, used) = row?;
            log.push(LoggedQuery {
                asked_at: parse_timestamp(&asked_at)?,
                model: ModelIdentity::new(model),
                scoped,
                #[allow(clippy::cast_possible_truncation)]
                raw_top: raw_top as f32,
                #[allow(clippy::cast_possible_truncation)]
                centred_top: centred_top as f32,
                silenced,
                returned: self.logged_hits(id)?,
                used,
            });
        }
        Ok(log)
    }

    fn logged_hits(&self, query: i64) -> Result<Vec<MemoryId>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT memory_id FROM query_log_hits WHERE query = ?1 ORDER BY position")?;
        let rows = statement.query_map(params![query], |row| row.get::<_, String>(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(MemoryId::parse(&row?)?);
        }
        Ok(ids)
    }

    /// This store's non-match floor, if one has been calibrated.
    ///
    /// **A floor belongs to one store and one model.** The level is entirely the model's — every
    /// set's floor moved 0.10 to 0.16 under `potion-multilingual-128M` — so a model change clears
    /// it, exactly where the vectors are cleared and for the same reason.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn floor(&self) -> Result<Option<f64>, StoreError> {
        let recorded: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'non_match_floor'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        recorded
            .map(|value| {
                value.parse::<f64>().map_err(|_| StoreError::Unrecognised {
                    field: "non_match_floor",
                    value,
                })
            })
            .transpose()
    }

    /// Set this store's non-match floor. See [`Store::calibrate`], which is what decides the level.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn set_floor(&self, floor: f64) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('non_match_floor', ?1)",
            params![floor.to_string()],
        )?;
        Ok(())
    }

    /// Remove the level and the record of what it was read from, which go together: a source left
    /// behind would describe a floor that no longer exists.
    fn clear_floor(&self) -> Result<(), StoreError> {
        self.connection.execute(
            "DELETE FROM meta WHERE key IN ('non_match_floor', 'non_match_floor_source')",
            [],
        )?;
        Ok(())
    }

    fn record_floor_source(&self, source: FloorSource) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('non_match_floor_source', ?1)",
            params![source.as_str()],
        )?;
        Ok(())
    }

    /// What this store's floor was read from, or `None` where there is no floor or it was set
    /// directly rather than by a calibration.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn floor_source(&self) -> Result<Option<FloorSource>, StoreError> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'non_match_floor_source'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match value.as_deref() {
            Some("labels") => Some(FloorSource::Labels),
            Some("its-own-memories") => Some(FloorSource::ItsOwnMemories),
            _ => None,
        })
    }

    /// Remove this store's floor, and forget how far the last calibration had read.
    ///
    /// **The remedy the self-lowering rule cannot provide.** A floor gives way to a scope-labelled
    /// query that contradicts it only at the next calibration, and only a query that *names this
    /// set* is a contradiction at all — an unscoped search silenced by a bad level leaves no trace
    /// that corrects it. So there has to be a way out that does not require changing the embedding
    /// model, which is the only other thing that clears a floor.
    ///
    /// The read marker goes too: clearing means starting over, so every labelled query in the window
    /// is evidence the next calibration may use rather than evidence already spent.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn uncalibrate(&self) -> Result<Option<f64>, StoreError> {
        let had = self.floor()?;
        self.clear_floor()?;
        self.connection.execute(
            "DELETE FROM meta WHERE key = 'floor_calibrated_through'",
            [],
        )?;
        Ok(had)
    }

    /// Set this store's floor from its own query log, if the log holds enough to set one from.
    ///
    /// **A margin below the 5th percentile of what labelled queries reached.** A query is labelled
    /// when the session that asked it named this set as its `scope`, or went on to use a memory it
    /// returned — see [`Store::note_used`]. Nothing else is evidence about where the floor belongs:
    /// an unlabelled query is exactly the case a floor exists to silence.
    ///
    /// Explicit rather than automatic, in both directions: raising a floor is the direction that
    /// loses answers, and lowering it from recall made a ratchet, removed 2026-09-12. A
    /// scope-labelled query scoring below the floor is silenced and logged, and this reads it next
    /// time.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn calibrate(&mut self) -> Result<Calibration, StoreError> {
        let through: i64 = self.calibrated_through()?;
        let mut window: Vec<f64> = Vec::new();
        let mut fresh = 0usize;
        let mut highest = through;
        {
            let mut statement = self.connection.prepare(
                "SELECT id, raw_top FROM query_log WHERE used = 1 OR scoped = 1 ORDER BY id",
            )?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)))?;
            for row in rows {
                let (id, score) = row?;
                window.push(score);
                if id > through {
                    fresh += 1;
                }
                highest = highest.max(id);
            }
        }

        // Counted since the last calibration, so a level is never recomputed from evidence the
        // previous one already used -- which would fire on the same queries for ever and leave the
        // level standing still while the store moved.
        if fresh < LABELS_PER_CALIBRATION {
            // **Labels outrank the set's own memories.** A floor read from labels stands until new
            // ones arrive; only a set with no such floor is placed from what it holds.
            if self.floor_source()? != Some(FloorSource::Labels)
                && let Some((floor, memories)) = self.floor_from_its_own_memories()?
            {
                self.set_floor(floor)?;
                self.record_floor_source(FloorSource::ItsOwnMemories)?;
                return Ok(Calibration::FromItsOwnMemories { floor, memories });
            }
            return Ok(Calibration::NotEnoughEvidence { labelled: fresh });
        }

        let floor = percentile_of(&mut window, FLOOR_PERCENTILE) - FLOOR_MARGIN;
        self.set_floor(floor)?;
        self.record_floor_source(FloorSource::Labels)?;
        self.connection.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('floor_calibrated_through', ?1)",
            params![highest.to_string()],
        )?;
        Ok(Calibration::Set {
            floor,
            labelled: window.len(),
        })
    }

    /// A floor placed from this set's own current claims, for a set too little used to gather
    /// labels: each claim's index line asked against the rest with itself left out, and the level
    /// the same percentile and margin labels use. `None` below [`LABELS_PER_CALIBRATION`] claims,
    /// or with no model to ask with.
    ///
    /// **Current claims only.** A superseded memory's index line all but repeats its replacement's,
    /// which would score every chain as a near-perfect answer, and an identity record is a name
    /// rather than a claim. **The minimum is the label count**, the same amount of evidence either
    /// way: at eight memories the percentile is the set's weakest memory, and measured on
    /// `governance-docs` that placed no useful floor. `calibration-from-self-queries`, measured
    /// by `probe_a_floor_without_labelled_queries`.
    fn floor_from_its_own_memories(&mut self) -> Result<Option<(f64, usize)>, StoreError> {
        let Some(embedder) = self.embedder.clone() else {
            return Ok(None);
        };
        self.prepare()?;

        let mut claims = Vec::new();
        for id in self.memory_ids()? {
            if !matches!(self.resolve_supersession(id)?, Resolution::Current) {
                continue;
            }
            let Some(memory) = self.get(id)? else {
                continue;
            };
            if memory.is_identity_record() {
                continue;
            }
            claims.push(memory);
        }
        if claims.len() < LABELS_PER_CALIBRATION {
            return Ok(None);
        }

        let lines: Vec<String> = claims.iter().map(|m| m.content.summary_as_data()).collect();
        let asked = embedder.embed_each(&lines)?;
        let mut best: Vec<f64> = Vec::with_capacity(claims.len());
        for (index, question) in asked.iter().enumerate() {
            let Embedded::Vector(question) = question else {
                continue;
            };
            let top = claims
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .filter_map(|(_, memory)| self.vectors.raw_similarity(memory.id, question))
                .map(f64::from)
                .fold(f64::NEG_INFINITY, f64::max);
            if top.is_finite() {
                best.push(top);
            }
        }
        if best.len() < LABELS_PER_CALIBRATION {
            return Ok(None);
        }
        Ok(Some((
            percentile_of(&mut best, FLOOR_PERCENTILE) - FLOOR_MARGIN,
            claims.len(),
        )))
    }

    /// How many labelled queries have arrived since the last calibration, which is the evidence a
    /// calibration now would read.
    ///
    /// **Counted the way [`calibrate`](Self::calibrate) counts it**, against the same recorded
    /// watermark, so a report of *this set is ready* and the operation it offers cannot disagree.
    /// What a report cannot ask of `calibrate` is this number alone: calibrating to find out sets a
    /// floor, which is the direction that loses answers.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the query fails, and [`StoreError::Malformed`] if the
    /// recorded watermark is not a number.
    pub fn labels_since_calibration(&self) -> Result<usize, StoreError> {
        let through: i64 = self.calibrated_through()?;
        let fresh: i64 = self.connection.query_row(
            "SELECT count(*) FROM query_log WHERE (used = 1 OR scoped = 1) AND id > ?1",
            params![through],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(fresh).unwrap_or(0))
    }

    /// The highest query-log row the last calibration consumed, or 0 if there has never been one.
    fn calibrated_through(&self) -> Result<i64, StoreError> {
        let recorded: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'floor_calibrated_through'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(recorded.and_then(|value| value.parse().ok()).unwrap_or(0))
    }

    /// Keep the log bounded, one bound per kind.
    ///
    /// **Labelled rows are the calibration window**, and bounding them is what lets a level rise: read
    /// off everything a store was ever asked, a percentile drifts toward the whole history rather
    /// than tracking a corpus growing underneath it. **An unlabelled row from a finished session can
    /// never become labelled**, since [`Store::note_used`] is bounded to one session, so it is
    /// evidence about the labelling rate and nothing more — bounded on its own terms rather than
    /// competing with the rows calibration actually reads.
    fn prune_query_log(&self) -> Result<(), StoreError> {
        for labelled in [0, 1] {
            self.connection.execute(
                "DELETE FROM query_log WHERE (used = 1 OR scoped = 1) = ?1 AND id NOT IN (
                     SELECT id FROM query_log WHERE (used = 1 OR scoped = 1) = ?1
                     ORDER BY id DESC LIMIT ?2
                 )",
                params![labelled, i64::try_from(QUERY_LOG_KEPT).unwrap_or(i64::MAX)],
            )?;
        }
        Ok(())
    }

    /// Label every query this session asked that returned `memory`, because the session went on to
    /// use it.
    ///
    /// **What a session did with an answer is the evidence that the set could answer**, and it
    /// arrives from ordinary work rather than from anyone labelling anything: a memory is looked up,
    /// linked to, or passed back as already held. Bounded to one session — see [`SessionId`] — and
    /// naturally bounded to queries already asked, since a row written later starts unlabelled and
    /// no use can reach back for it.
    ///
    /// Silent when nothing matches, which is the ordinary case: most memories a session uses were
    /// not found by a recall in this store.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn note_used(&self, memory: MemoryId, session: SessionId) -> Result<(), StoreError> {
        self.connection.execute(
            "UPDATE query_log SET used = 1
             WHERE session = ?1
               AND id IN (SELECT query FROM query_log_hits WHERE memory_id = ?2)",
            params![session.to_string(), memory.to_string()],
        )?;
        // **Here as well as at write time, and that is not belt-and-braces.** A row is written
        // unlabelled and labelled afterwards, so pruning at write time can only ever bound the
        // unlabelled side; the labelled window is bounded exactly when a label arrives.
        self.prune_query_log()?;
        Ok(())
    }

    /// Record one recall of this store. See [`Asking`] for who may cause this to be called.
    ///
    /// A `&self` method, like [`Store::record_model`], because it writes SQL and touches nothing
    /// held in memory — which is what lets a recall write it while several stores are borrowed
    /// shared for scanning.
    fn log_query(&self, entry: &LoggedQuery, session: SessionId) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO query_log
                 (asked_at, model, session, scoped, raw_top, centred_top, silenced, used)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.asked_at.to_string(),
                entry.model.as_str(),
                session.to_string(),
                entry.scoped,
                f64::from(entry.raw_top),
                f64::from(entry.centred_top),
                entry.silenced,
                // Written rather than left to the column default, so that what the caller states is
                // what lands. A field the writer drops is one a later reader trusts and nothing
                // maintains -- found by the mutation that sets it and changed nothing.
                entry.used,
            ],
        )?;
        let id = self.connection.last_insert_rowid();
        self.prune_query_log()?;
        for (position, memory) in entry.returned.iter().enumerate() {
            self.connection.execute(
                "INSERT INTO query_log_hits (query, position, memory_id) VALUES (?1, ?2, ?3)",
                params![
                    id,
                    i64::try_from(position).unwrap_or(i64::MAX),
                    memory.to_string()
                ],
            )?;
        }
        Ok(())
    }

    /// This store's links, split into what the walk crosses and what has been replaced.
    ///
    /// One pass rather than two queries, because both answers come from the same rows and the walk
    /// needs them together: an edge is only useful once both its endpoints have been resolved to
    /// current memories.
    ///
    /// Direction is dropped from the walkable edges because expansion is symmetric — an edge is
    /// crossed the same way from either end. Which labels are crossed is decided by
    /// [`traversal::is_traversable`] rather than by a label list written into the SQL, so the rule
    /// lives in one place and a new label cannot become traversable by omission.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    fn edges(&self) -> Result<Edges, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT source, label, target, target_set, target_set_id FROM links")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;

        let mut edges = Edges::default();
        for row in rows {
            let (source, label, target, target_set, target_set_id) = row?;
            let label: LinkLabel = parse_enum("link label", &label)?;
            let source = MemoryId::parse(&source)?;
            let target = MemoryId::parse(&target)?;

            if traversal::is_traversable(&label) {
                // **A standing-changing edge reaching into another set is reported, never acted on
                // -- and being walked is being acted on, in the claimant's favour.**
                // `documents/design/cross-set-references.md` settles that what is reported is the
                // edge rather than the other memory, and that the claiming memory is "neither
                // fetched nor ranked". A walkable edge ranks it: one hop from the memory it
                // attacks, inheriting accordingly, which is the permanent slot in every recall that
                // the same section declined to give it -- "weaker than suppression and still a free
                // channel into a stranger's context". Measured at 0.192 inherited against a control
                // of 0 (`a-cross-set-dispute-carries-score-to-the-memory-making-it`).
                //
                // `SUPERSEDES` never reaches here, since `is_traversable` excludes it; the other
                // two standing-changing labels do, and did. Asked through `DISPUTING_LABELS` rather
                // than by naming them, so a label cannot become a dispute at resolution and stay
                // walkable here.
                //
                // **The same edge written within one set is ordinary and must keep working**, which
                // is what `target_set` distinguishes: a named set here means a foreign one, because
                // `write_memory` stores a target naming its own set in the compact form and
                // `compact_own_set_links` rewrites any that predate that, so the long spelling of a
                // same-set link never survives to be read.
                if !(DISPUTING_LABELS.contains(&label) && target_set.is_some()) {
                    let set = match target_set.as_deref() {
                        Some(name) => Some(SetName::new(name)?),
                        None => None,
                    };
                    let set_id = target_set_id.as_deref().map(SetId::parse).transpose()?;
                    edges.walkable.push((source, target, set, set_id));
                }
                continue;
            }
            // The two spellings of the same fact, from opposite ends: `A SUPERSEDES B` is written on
            // the replacement, and `B SUPERSEDED_BY A` on the memory acknowledging its own
            // obsolescence -- which is the only form available when the replacement is in a set this
            // machine cannot write. Either way it is `B` that has been replaced.
            //
            // **The two arms are asymmetric on purpose, and the asymmetry is the fix for
            // `a-set-can-suppress-another-sets-memory`.** `documents/design/cross-set-references.md`
            // resolves that no edge changes the standing of a memory in another set. `SUPERSEDES`
            // reaches *outward* at its target, so a cross-set one is dropped here: it is the only
            // way a foreign id entered `replaced`, and `Neighbourhood::over` then asked this store
            // to resolve a memory it does not hold, merging the answer into a validity map the whole
            // recall shares. `SUPERSEDED_BY` names its own source, which is always a memory of this
            // store, so it is a memory conceding its own standing rather than reaching for someone
            // else's -- legal across sets, and kept.
            match label {
                LinkLabel::Supersedes if target_set.is_none() => edges.replaced.push(target),
                LinkLabel::SupersededBy => edges.replaced.push(source),
                _ => {}
            }
        }
        Ok(edges)
    }

    /// Record this store's scope, or check it if one is already recorded.
    fn bind_scope(&self) -> Result<(), StoreError> {
        let recorded: Option<String> = self
            .connection
            .query_row("SELECT value FROM meta WHERE key = 'scope'", [], |row| {
                row.get(0)
            })
            .optional()?;

        match recorded {
            None => {
                self.connection.execute(
                    "INSERT INTO meta (key, value) VALUES ('scope', ?1)",
                    params![scope_key(&self.scope)],
                )?;
                Ok(())
            }
            Some(found) if found == scope_key(&self.scope) => Ok(()),
            Some(found) => Err(StoreError::ScopeMismatch {
                found: scope_from_key(&found)?,
            }),
        }
    }

    /// The scope this store holds.
    #[must_use]
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// The file this store is reading, when it has one.
    ///
    /// Asked of the connection rather than remembered from [`Store::open_at`], so it cannot disagree
    /// with the database actually open — the single-source-of-truth rule in `STANDARDS.md` applied to
    /// a fact that is otherwise very easy to keep a stale second copy of.
    ///
    /// **The path is SQLite's, so it is fully resolved** — absolute, with symlinks followed. It names
    /// where the data actually is rather than the route configured to reach it, which is the more
    /// useful of the two for the question this exists to answer, and is what makes two homes sharing
    /// one store through a symlink report the same file rather than two.
    ///
    /// `None` for an in-memory store. SQLite reports those as an empty path, which would travel to a
    /// caller as `""` and read like a filesystem answer rather than the absence of one.
    #[must_use]
    pub fn file(&self) -> Option<&Path> {
        self.connection
            .path()
            .filter(|path| !path.is_empty())
            .map(Path::new)
    }

    /// Who this store attributes new memories to, if it has been told.
    ///
    /// `None` is a real state and not a defect: a store that nothing has written to yet has no
    /// identity to report, and inventing one from the reading process would be the bug this whole
    /// mechanism exists to remove.
    ///
    /// Reads SQL, so it needs no refresh and takes `&self` — see the note on [`Store`]. Deliberately
    /// not cached: an author is read once per write and set by hand about never, so a cache would
    /// buy nothing and would owe the coherence discipline
    /// `documents/design/store-registry.md` requires of shared state.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Malformed`] if the recorded value is not a valid author, and
    /// [`StoreError::Database`] if the query fails.
    pub fn author(&self) -> Result<Option<Author>, StoreError> {
        let recorded: Option<String> = self
            .connection
            .query_row("SELECT value FROM meta WHERE key = 'author'", [], |row| {
                row.get(0)
            })
            .optional()?;

        recorded.map(Author::new).transpose().map_err(Into::into)
    }

    /// This set's own identity, if it has one.
    ///
    /// **Absent means *not yet minted*, never *unknown***, and the distinction is the whole
    /// migration. A store built before set identity existed has none, and the rules are that it
    /// adopts one from a file that carries it ([`adopt_set_id`](Self::adopt_set_id)) and is
    /// otherwise checked by name exactly as it always was. What it must not do is invent one:
    /// `TRACKING.md` records why, and the reason is that every store
    /// inventing independently manufactures the collision the identity exists to prevent.
    ///
    /// Stored in `meta` beside the author and the keyword-folding marker, and read the same way —
    /// SQL, no cache, `&self`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Malformed`] if the recorded value is not a valid set id, and
    /// [`StoreError::Database`] if the query fails.
    pub fn set_id(&self) -> Result<Option<SetId>, StoreError> {
        let recorded: Option<String> = self
            .connection
            .query_row("SELECT value FROM meta WHERE key = 'set_id'", [], |row| {
                row.get(0)
            })
            .optional()?;

        recorded
            .as_deref()
            .map(SetId::parse)
            .transpose()
            .map_err(Into::into)
    }

    /// How this machine holds the set. An absent row is read-write, which is every set created
    /// before load modes existed.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UnknownLoadMode`] for a value no mode spells, and [`StoreError::Database`]
    /// if the query fails.
    pub fn load_mode(&self) -> Result<LoadMode, StoreError> {
        let recorded: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'load_mode'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match recorded.as_deref() {
            None => Ok(LoadMode::ReadWrite),
            Some(spelled) if spelled == LoadMode::ReadOnly.as_str() => Ok(LoadMode::ReadOnly),
            Some(spelled) if spelled == LoadMode::Evaluation.as_str() => Ok(LoadMode::Evaluation),
            Some(other) => Err(StoreError::UnknownLoadMode {
                found: other.to_owned(),
            }),
        }
    }

    /// Hold this set read-only, for a set being created now.
    ///
    /// **Only while the set holds nothing**, tombstones included: that is what makes *this machine
    /// never wrote to it* true of every read-only set, which `drop` relies on to skip its safety
    /// copy. There is no way back to read-write here; promotion, if it is ever offered, is its own
    /// operation.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ModeIsChosenAtCreation`] if the set holds any record, and
    /// [`StoreError::Database`] if the write fails.
    pub fn choose_read_only(&self) -> Result<(), StoreError> {
        self.choose_mode(LoadMode::ReadOnly)
    }

    /// Hold this set as an evaluation load, for a store being filled from a file now.
    ///
    /// Same rule as [`choose_read_only`](Self::choose_read_only): only while the store holds
    /// nothing. An evaluation store is in memory and is never discovered at startup, so the recorded
    /// mode serves this process alone.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ModeIsChosenAtCreation`] if the set holds any record, and
    /// [`StoreError::Database`] if the write fails.
    pub fn choose_evaluation(&self) -> Result<(), StoreError> {
        self.choose_mode(LoadMode::Evaluation)
    }

    fn choose_mode(&self, mode: LoadMode) -> Result<(), StoreError> {
        if self.holds_any_record()? {
            return Err(StoreError::ModeIsChosenAtCreation);
        }
        self.connection.execute(
            "INSERT INTO meta (key, value) VALUES ('load_mode', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![mode.as_str()],
        )?;
        Ok(())
    }

    /// Refuse a session's write to a set held read-only.
    ///
    /// Called at the top of every entry point a session writes through -- [`insert`](Self::insert),
    /// [`carry_forward`](Self::carry_forward) and [`forget`](Self::forget) -- and at none a file
    /// writes through, which is how a read-only set still takes its records from an import. Public
    /// so an operation that would fail partway can ask before it starts.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ReadOnly`] for a read-only set, and whatever
    /// [`load_mode`](Self::load_mode) returns if the mode cannot be read.
    pub fn refuse_session_writes_if_read_only(&self) -> Result<(), StoreError> {
        if self.load_mode()?.accepts_session_writes() {
            return Ok(());
        }
        let set = match &self.scope {
            Scope::Shared(set) => set.as_str().to_owned(),
            Scope::Local => "local".to_owned(),
        };
        Err(StoreError::ReadOnly { set })
    }

    /// Mint this set's identity if it has none, and return the one it holds either way.
    ///
    /// **Called by every write, shared set or `local`, since `set-settled-at-first-write`** (decided
    /// 2026-09-24), so a set is given its identity where it is first written. A shared set used to be
    /// refused instead, because two holders of one set each minting would fork it -- but `create`
    /// minted just as blindly, and since `identity-mismatch-refusal` a fork is refused at the next
    /// import rather than merged. Minting here is also what lets `create` then `import` take on a
    /// published set with no identity to displace.
    ///
    /// **What it keeps is the invariant the refusal existed for**: a set holding records has an
    /// identity, which deciding whether a stored link is same-set relies on
    /// (`a-link-naming-its-own-set-by-an-old-name-is-inert`). **Only into an absence**, so the
    /// second write cannot rename the set. **On a write rather than at open**, which is
    /// [`author_record_or_mint`](Self::author_record_or_mint)'s rule and its reason: opening a store --
    /// or reading one -- must never write to it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the write fails.
    pub fn mint_set_id(&self) -> Result<SetId, StoreError> {
        // A shortcut, not the guard: two first writes can both read `None` here, which is why the
        // answer comes from the claim rather than from what this call generated.
        if let Some(existing) = self.set_id()? {
            return Ok(existing);
        }
        self.claim_set_id(SetId::generate())
    }

    /// Give this set `id` if it has no identity, **atomically**, and return the identity it holds
    /// afterwards -- which is `id` only if this call was first.
    ///
    /// `concurrent-first-writes-can-mint-two-identities`: minting read, found none, and wrote with
    /// an upsert, so two first writes on one file each wrote and the later one won, leaving the
    /// earlier writer's records stamped with an identity the set no longer held. The read and the
    /// write are one statement here, and the caller is told what won rather than assuming it did.
    /// Split out of [`mint_set_id`](Self::mint_set_id) so a test can interleave two connections'
    /// reads and claims by hand (`tests/concurrency.rs`).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the write fails.
    pub fn claim_set_id(&self, id: SetId) -> Result<SetId, StoreError> {
        self.fill_set_id(id)?;
        self.set_id()?
            .ok_or_else(|| StoreError::Database(rusqlite::Error::QueryReturnedNoRows))
    }

    /// Take a set identity from a file, for a store that has none.
    ///
    /// **Fills an absence; never displaces a value**, records or not, which is the same rule
    /// [`author_record_or_mint`](Self::author_record_or_mint) states and for the same reason: filling a gap
    /// converges, because every reader agrees on the value it finds, while overwriting a mismatch
    /// oscillates. Reports whether it took one, because a caller that just adopted an identity is
    /// telling a human something they did not know a moment ago.
    ///
    /// **A mismatch is not refused here.** Import refuses one, since 2026-09-26
    /// (`identity-mismatch-refusal`), by asking after adoption whether the store now carries the
    /// file's identity -- so this stays the one rule about when an identity may be taken, and the
    /// refusal is made by the layer that can say which file and which set.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the write fails.
    pub fn adopt_set_id(&self, offered: SetId) -> Result<bool, StoreError> {
        // **Held means held, since `set-settled-at-first-write`**, whether the identity held is this
        // one or another: an empty set used to be allowed to give up its identity, because `create`
        // minted one seconds before `import` could offer the file's
        // (`a-re-added-set-cannot-recover-its-identity`), and `create` no longer mints. **One
        // statement, with no read first**, so a mint on another connection cannot land between a
        // read and this write (`concurrent-first-writes-can-mint-two-identities`).
        self.fill_set_id(offered)
    }

    /// Whether this set holds anything at all — a memory or a tombstone.
    ///
    /// Tombstones count. A set that has forgotten something has been used, whatever it holds now,
    /// and *used* is what makes an identity worth keeping rather than the live record count.
    pub(crate) fn holds_any_record(&self) -> Result<bool, StoreError> {
        let any: i64 = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM memories UNION ALL SELECT 1 FROM tombstones)",
            [],
            |row| row.get(0),
        )?;
        Ok(any != 0)
    }

    /// Write `id` as this set's identity only if it has none, in one statement, and say whether it
    /// was written. **Never an upsert**: displacing an identity is what forked a set when two first
    /// writes met, and no caller has a reason to.
    fn fill_set_id(&self, id: SetId) -> Result<bool, StoreError> {
        let written = self.connection.execute(
            "INSERT INTO meta (key, value) VALUES ('set_id', ?1)
             ON CONFLICT(key) DO NOTHING",
            params![id.to_string()],
        )?;
        Ok(written == 1)
    }

    /// Mint an identity record for someone new to this set, and point this store at it.
    ///
    /// **The operation that can fork a person, so it is the one that refuses rather than infers.**
    /// Two identity records for one contributor are indistinguishable from two contributors, so
    /// nothing downstream can report the mistake — which is why *I am new here* has to be stated
    /// rather than deduced from a store that happens to have no cached answer.
    ///
    /// # Errors
    ///
    /// [`StoreError::AlreadyIdentified`] where this store already knows which record is you, and
    /// [`StoreError::NameAlreadyHeld`] where a current record carries the name — in which case the
    /// caller means `identify` and has the id one recall away.
    pub fn mint_author(
        &mut self,
        name: &Author,
        at: jiff::Timestamp,
    ) -> Result<MemoryId, StoreError> {
        if let Some(held) = self.author_record()? {
            return Err(StoreError::AlreadyIdentified {
                record: held.to_string(),
            });
        }
        if let [existing, ..] = self.author_records_named(name)?.as_slice() {
            return Err(StoreError::NameAlreadyHeld {
                name: name.as_str().to_owned(),
                record: existing.to_string(),
            });
        }

        let id = self.write_identity_record(name, at, None)?;
        self.write_author_record(id)?;
        Ok(id)
    }

    /// Point this store at an identity record it already holds.
    ///
    /// **Takes an id, never a name.** A name can match several current records or none, which is
    /// the ambiguous direction the resolution rules refuse to walk; reintroducing it here would put
    /// the same guess behind a different verb. Writes no set data at all — this is machine state.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotAnIdentityRecord`] where the id names nothing, names an ordinary memory, or
    /// names a record that has since been superseded.
    pub fn identify_author(&self, record: MemoryId) -> Result<Author, StoreError> {
        let refuse = |why: &str| StoreError::NotAnIdentityRecord {
            record: record.to_string(),
            why: why.to_owned(),
        };

        let Some(memory) = self.get(record)? else {
            return Err(refuse("this set does not hold it"));
        };
        if !memory.is_identity_record() {
            return Err(refuse("it is an ordinary memory, not an identity record"));
        }
        if !matches!(self.resolve_supersession(record)?, Resolution::Current) {
            return Err(refuse(
                "it has been superseded, so it is a name someone used to publish under",
            ));
        }

        self.write_author_record(record)?;
        Author::new(memory.content.as_data()).map_err(StoreError::from)
    }

    /// Change the name this machine publishes under in this set: supersede, then forget.
    ///
    /// **Both halves, because superseding alone is the answer everyone reaches for first and it is
    /// half of one.** A superseded record stays readable, carrying the old name, in every copy of
    /// the set — so where the motive is to stop publishing under a name, superseding achieves
    /// nothing on its own. Forgetting leaves a tombstone, which carries no content: the shape of the
    /// change survives and the name does not.
    ///
    /// **The cache is repointed before the old record is forgotten**, so no moment exists in which
    /// this store's answer to *which record is you* names a tombstone.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotIdentifiedHere`] where this store does not know which record is you — there
    /// is nothing to rename, and minting one under the new name is `mint`'s job to refuse or allow.
    pub fn rename_author(
        &mut self,
        name: &Author,
        at: jiff::Timestamp,
    ) -> Result<Renamed, StoreError> {
        let Some(previous) = self.author_record()? else {
            return Err(StoreError::NotIdentifiedHere);
        };
        let was = self.author_name(previous)?;

        let replacement = self.write_identity_record(name, at, Some(previous))?;
        self.write_author_record(replacement)?;
        // Attributed to the identity doing the forgetting, which is by now the new one -- so the
        // tombstone opens no trail back to the name being removed.
        self.forget(previous, replacement, at)?;
        self.write_author(name)?;

        Ok(Renamed {
            previous,
            replacement,
            was,
        })
    }

    /// Write an identity record, optionally superseding one.
    fn write_identity_record(
        &mut self,
        name: &Author,
        at: jiff::Timestamp,
        supersedes: Option<MemoryId>,
    ) -> Result<MemoryId, StoreError> {
        let mut record = Memory::self_attributed(
            MemoryText::new(name.as_str())?,
            Confidence::new(1.0)?,
            Origin::Human,
            at,
        );
        record.keywords = vec![Keyword::new(AUTHOR_KEYWORD)?];
        record.why = Some(MemoryText::new(
            "The name this set attributes writes to. Held as a memory so that changing it \
             supersedes one record rather than rewriting every record that names it.",
        )?);
        if let Some(previous) = supersedes {
            record.links = vec![Link {
                label: LinkLabel::Supersedes,
                target: LinkTarget::SameSet(previous),
            }];
        }
        let id = record.id;
        self.insert(&record)?;
        Ok(id)
    }

    /// Every current identity record in this set carrying exactly this name.
    fn author_records_named(&self, name: &Author) -> Result<Vec<MemoryId>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM memories WHERE created_by = id AND content = ?1")?;
        let rows = statement.query_map(params![name.as_str()], |row| row.get::<_, String>(0))?;
        let mut found = Vec::new();
        for row in rows {
            let id = MemoryId::parse(&row?)?;
            // A superseded record is a past name, not a candidate to write under again.
            if matches!(self.resolve_supersession(id)?, Resolution::Current) {
                found.push(id);
            }
        }
        Ok(found)
    }

    /// The name an author record carries, following supersession to the tip of its chain.
    ///
    /// This is what makes `by` worth holding as a reference: a memory pointing at a superseded
    /// author record reads as the *current* name without the memory being rewritten, which is the
    /// whole of `documents/design/memory-data-model.md`'s attribution proposal in one lookup.
    ///
    /// `None` where the id names nothing, names a memory that is not an author record, or resolves
    /// to more than one tip. The last is deliberate rather than a shrug: several tips means one
    /// identity was split into two, which is the duplicate-chain hazard the design says nothing
    /// downstream can detect — so picking one arbitrarily would be inventing the answer.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn author_name(&self, id: MemoryId) -> Result<Option<Author>, StoreError> {
        let tip = match self.resolve_supersession(id)? {
            Resolution::Superseded { tips, .. } if tips.len() == 1 => tips[0],
            Resolution::Current => id,
            _ => return Ok(None),
        };

        // The keyword is checked on the tip rather than on `id`, because the tip is what supplies
        // the name: an author record superseded by an ordinary memory would otherwise hand back
        // that memory's content as an identity.
        let Some(memory) = self.get(tip)? else {
            return Ok(None);
        };
        if !memory.is_identity_record() {
            return Ok(None);
        }

        Ok(Some(Author::new(memory.content.as_data())?))
    }

    /// The author record this store writes as, minting one where the set has no identity yet.
    ///
    /// `documents/design/memory-data-model.md`, *which record is you is machine state*. Three
    /// outcomes, and only one of them decides anything:
    ///
    /// - the cache in `meta` names a record — use it, which is the ordinary path and one read
    /// - the set holds no `hoard-author` records at all — mint one, because nothing can collide
    /// - the set holds author records and the cache does not say which is you — **`None`**, meaning
    ///   *ask*, because adopting on a name match would fork whoever has changed the name they
    ///   publish under, and two chains for one person are indistinguishable from two contributors.
    ///
    /// The minted record attributes itself: see [`Memory::self_attributed`].
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn author_record_or_mint(
        &mut self,
        name: &Author,
        at: jiff::Timestamp,
    ) -> Result<Option<MemoryId>, StoreError> {
        if let Some(cached) = self.author_record()? {
            return Ok(Some(cached));
        }

        if self.holds_any_author_record()? {
            return Ok(None);
        }

        // The name this set was created with wins over the process's configuration, which is the
        // precedence `author_note` already states: one store can be open in two servers, and
        // attribution that followed the process would record one person under whichever window
        // happened to be in front.
        let recorded = self.author()?;
        let name = recorded.as_ref().unwrap_or(name);

        let record = Memory::self_attributed(
            MemoryText::new(name.as_str())?,
            Confidence::new(1.0)?,
            Origin::Human,
            at,
        );
        let mut record = record;
        record.keywords = vec![Keyword::new(AUTHOR_KEYWORD)?];
        record.why = Some(MemoryText::new(
            "The name this set attributes writes to. Held as a memory so that changing it \
             supersedes one record rather than rewriting every record that names it.",
        )?);
        let id = record.id;
        self.insert(&record)?;
        self.write_author_record(id)?;
        Ok(Some(id))
    }

    /// The cached author record id, if this store has resolved one.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn author_record(&self) -> Result<Option<MemoryId>, StoreError> {
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'author_record'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        raw.map(|value| MemoryId::parse(&value))
            .transpose()
            .map_err(StoreError::from)
    }

    /// Point this store's cache at an author record.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn write_author_record(&self, id: MemoryId) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO meta (key, value) VALUES ('author_record', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// The name this set attributes writes to, for a person reading a report about the set.
    ///
    /// **Set-level, and deliberately unlike a memory's `by`.** A memory carries the author *record's
    /// id*, because a reader almost never wants the name and does want the handle when they do.
    /// This answers the other question — *who does this set attribute writes to* — which is asked
    /// by a human orienting themselves, once per set rather than once per record, so a name is what
    /// it should say.
    ///
    /// Resolves the identity record where there is one, and falls back to the legacy `meta.author`
    /// string for a store written before identity records existed.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn author_display(&self) -> Result<Option<Author>, StoreError> {
        if let Some(record) = self.author_record()?
            && let Some(name) = self.author_name(record)?
        {
            return Ok(Some(name));
        }
        self.author()
    }

    /// How many identity records this set holds.
    ///
    /// Reported alongside `memories` and `forgotten` so that `records = memories + forgotten +
    /// identities` still reconciles from one response. **That invariant is the export gate's
    /// check**, and excluding identity records from `memories` without naming them somewhere would
    /// have broken it silently — an off-by-one in the one arithmetic that exists to catch
    /// off-by-ones.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn identity_count(&self) -> Result<usize, StoreError> {
        let total: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM memories WHERE created_by = id",
            [],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(total).unwrap_or(usize::MAX))
    }

    /// The durable answer to *which identity record is me in this set*, if this machine has one.
    ///
    /// **Keyed on `set_id`, never on the set's name.** A local name can change, and two checkouts
    /// can hold one set under different names; identity is what both copies share. Matching on the
    /// name is the failure `a-hand-merge-into-a-new-set-name-leaves-supersessions-inert` records.
    ///
    /// # Errors
    ///
    /// See [`StoreError`]. Refuses on a store that is not `local`, which holds no map.
    pub fn author_map_get(&self, set: SetId) -> Result<Option<AuthorRow>, StoreError> {
        self.require_local("read the author map")?;
        self.connection
            .query_row(
                "SELECT author_record, scope FROM author_map WHERE set_id = ?1",
                params![set.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .map(|(record, scope)| {
                Ok(AuthorRow {
                    record: MemoryId::parse(&record)?,
                    scope,
                })
            })
            .transpose()
    }

    /// Record — or refresh — which identity record is this machine's user in one set.
    ///
    /// The scope label is rewritten on every call, so it can only be stale for a set that is not
    /// present. That is exactly the case where a person is reading it to decide whether to prune the
    /// row, and where a misread costs nothing: deleting a row is safe, because the fallback either
    /// mints where nothing can collide or asks.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn author_map_put(
        &self,
        set: SetId,
        record: MemoryId,
        scope: &str,
    ) -> Result<(), StoreError> {
        self.require_local("write the author map")?;
        self.connection.execute(
            "INSERT INTO author_map (set_id, author_record, scope) VALUES (?1, ?2, ?3)
             ON CONFLICT(set_id) DO UPDATE SET author_record = excluded.author_record,
                                               scope = excluded.scope",
            params![set.to_string(), record.to_string(), scope],
        )?;
        Ok(())
    }

    /// Forget one row. Safe by construction — see [`Store::author_map_put`].
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn author_map_forget(&self, set: SetId) -> Result<(), StoreError> {
        self.require_local("forget an author map row")?;
        self.connection.execute(
            "DELETE FROM author_map WHERE set_id = ?1",
            params![set.to_string()],
        )?;
        Ok(())
    }

    /// Every row, for a person deciding what to prune.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn author_map_rows(&self) -> Result<Vec<(SetId, AuthorRow)>, StoreError> {
        self.require_local("list the author map")?;
        let mut statement = self
            .connection
            .prepare("SELECT set_id, author_record, scope FROM author_map ORDER BY scope")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut found = Vec::new();
        for row in rows {
            let (set, record, scope) = row?;
            found.push((
                SetId::parse(&set)?,
                AuthorRow {
                    record: MemoryId::parse(&record)?,
                    scope,
                },
            ));
        }
        Ok(found)
    }

    /// The map is machine state and lives in one place; asking any other store for it is a bug.
    fn require_local(&self, what: &str) -> Result<(), StoreError> {
        if matches!(self.scope, Scope::Local) {
            return Ok(());
        }
        Err(StoreError::NotTheLocalStore {
            what: what.to_owned(),
            scope: format!("{:?}", self.scope),
        })
    }

    /// Whether this set holds any identity record at all, whoever it names.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn holds_any_author_record(&self) -> Result<bool, StoreError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM memories WHERE created_by = id",
            [],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Every current identity record in this set, with the name it carries, in id order.
    ///
    /// What the human is offered when asked which record is theirs -- by name, since a name is what
    /// they recognise, with the id carried beside it so the choice is never resolved by name. A
    /// superseded record is a name somebody used to publish under, so it is left out; so is
    /// anything self-attributed that is not an identity record.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn current_identity_records(&self) -> Result<Vec<(MemoryId, Author)>, StoreError> {
        let ids: Vec<String> = {
            let mut statement = self
                .connection
                .prepare("SELECT id FROM memories WHERE created_by = id ORDER BY id")?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };
        let mut records = Vec::new();
        for raw in ids {
            let id = MemoryId::parse(&raw)?;
            let Some(memory) = self.get(id)? else {
                continue;
            };
            if !memory.is_identity_record()
                || !matches!(self.resolve_supersession(id)?, Resolution::Current)
            {
                continue;
            }
            records.push((id, Author::new(memory.content.as_data())?));
        }
        Ok(records)
    }

    fn write_author(&self, author: &Author) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO meta (key, value) VALUES ('author', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![author.as_str()],
        )?;
        Ok(())
    }

    /// Visit every memory in id order, without holding the set in memory.
    ///
    /// `documents/design/file-format.md` requires that a large set can be exported without being
    /// materialised: at 1–1.5 KB a record, a long-lived project reaches tens of megabytes of file and
    /// two to three times that once parsed into owned strings. So this hands each memory to `visit`
    /// and drops it, and the caller writes as it goes.
    ///
    /// A callback rather than an iterator because a `rusqlite` row iterator borrows the prepared
    /// statement it came from, which cannot outlive this method without exposing that lifetime to
    /// every caller.
    ///
    /// **Id order comes from here**, because the format's ordering guarantee is what keeps an export
    /// from looking like a rewrite, and a streaming writer cannot sort what it has not yet seen. Ids
    /// are UUIDv7, so this is also creation order.
    ///
    /// The ids are collected first and the memories assembled one at a time through [`Store::get`].
    /// That costs a query per memory rather than one join, which is deliberate: `get` is the single
    /// place a row plus its links and keywords becomes a `Memory`, and a second assembly path here
    /// would be the duplication this codebase keeps filing defects about. The id list is the only
    /// thing held, at sixteen bytes each.
    ///
    /// # Errors
    ///
    /// Returns whatever `visit` returns, or [`StoreError::Sqlite`] if a query fails.
    pub fn stream_memories<F>(&self, mut visit: F) -> Result<usize, StoreError>
    where
        F: FnMut(&Memory) -> Result<(), StoreError>,
    {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM memories ORDER BY id")?;
        let ids: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<_, _>>()?;

        let mut written = 0;
        for key in ids {
            let Ok(id) = MemoryId::parse(&key) else {
                continue;
            };
            if let Some(memory) = self.get(id)? {
                visit(&memory)?;
                written += 1;
            }
        }
        Ok(written)
    }

    /// Record what this store and `target` now agree on, replacing any earlier agreement.
    ///
    /// Called at the end of an export, which is one of the two moments agreement is established —
    /// the other is import. Recording it only on import would make the next import read this
    /// machine's own exported changes as changes arriving from origin, and the whole file would
    /// present as a conflict.
    ///
    /// Replaces rather than merges, because an export writes the *whole* file: any record the store
    /// no longer holds is no longer in the file either, so its base is meaningless.
    ///
    /// Checksums are stored as hex text rather than as `INTEGER`. SQLite's integers are signed 64-bit,
    /// so half the checksum space would round-trip through a negative, and a reader that forgot the
    /// reinterpretation would compare wrongly for exactly half of all records — a defect that would
    /// look like a rare, unreproducible false conflict.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the write fails.
    pub fn set_bases(&mut self, target: &str, bases: &[(MemoryId, u64)]) -> Result<(), StoreError> {
        // Normalised here rather than only at the caller, so no path into this table can carry a
        // second spelling of one directory. Idempotent, so a caller that already normalised for its
        // own comparison pays nothing.
        let target = &Self::normalise_target(target);
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM bases WHERE target = ?1", params![target])?;
        for (id, checksum) in bases {
            transaction.execute(
                "INSERT INTO bases (target, memory_id, checksum) VALUES (?1, ?2, ?3)",
                params![target, id.to_string(), format!("{checksum:016x}")],
            )?;
        }
        // Written beside the bases it produced, so a build that cannot reproduce these checksums
        // detects that rather than comparing values it computed a different way.
        transaction.execute(
            "INSERT INTO meta (key, value) VALUES ('checksum_algorithm', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![crate::checksum::ALGORITHM],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Replace a `bases` table created with the cascading foreign key it should never have had.
    ///
    /// The first version of that table declared `memory_id ... REFERENCES memories(id) ON DELETE
    /// CASCADE`, copied from the sibling tables where cascading is correct. It is not correct here:
    /// `write_memory` upserts with `INSERT OR REPLACE`, which deletes the old row, so every
    /// amendment destroyed the base it should have preserved.
    ///
    /// **Dropping rather than rewriting the rows is the right repair**, because a base is local
    /// derived state that never reaches a memory file. Losing one means the next import reads that
    /// record as having no ancestor — noisy for one cycle, and not lossy, which is the property
    /// `documents/design/multi-set-sync.md` relies on when it says a change of checksum algorithm is
    /// recoverable. Preserving rows through a table rebuild would be more code defending a value that
    /// is cheaper to re-establish.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the migration fails.
    /// Bring every recorded target onto one spelling per directory.
    ///
    /// `target-identity-is-an-unnormalised-path-string`, the half that has to reach rows already
    /// written. [`Store::normalise_target`] fixes what arrives from now on; this fixes what a store
    /// carried before it existed.
    ///
    /// **A collision is not a merge, and that is the whole care of this function.**
    /// `documents/design/store-registry.md` settles it under *never keep a base you cannot prove*:
    /// two raw keys that normalise to one key carry two independent base sets, and wherever they
    /// disagree about a `memory_id` neither is the other's ancestor. Picking either supplies a wrong
    /// base for the file the other described -- which is a silent overwrite, the one outcome
    /// reconciliation exists to prevent. So both are dropped and the next export re-establishes
    /// agreement.
    ///
    /// **What dropping costs is stated in that document rather than overlooked**: a record the two
    /// sides once agreed on that the store no longer holds returns as `NewFromFile` instead of an
    /// `Anomaly`, so a silent resurrection replaces a report. A resurrection can be forgotten again;
    /// an overwritten memory cannot be recovered.
    ///
    /// A target that no longer resolves keeps its spelling and is left alone -- see
    /// [`Store::normalise_target`] for why that is inert rather than a defect, and why it matters
    /// that a checkout on an unmounted volume keeps its agreement.
    fn normalise_base_targets(&mut self) -> Result<(), StoreError> {
        let stored: Vec<String> = {
            let mut statement = self
                .connection
                .prepare("SELECT DISTINCT target FROM bases")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        // Grouped by what each spelling resolves to. A group of one is a rename; a group of two or
        // more is the collision above.
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for raw in stored {
            groups
                .entry(Self::normalise_target(&raw))
                .or_default()
                .push(raw);
        }

        let transaction = self.connection.transaction()?;
        for (normalised, raws) in groups {
            if raws.len() > 1 {
                for raw in raws {
                    transaction.execute("DELETE FROM bases WHERE target = ?1", params![raw])?;
                }
                continue;
            }
            let raw = &raws[0];
            if *raw != normalised {
                transaction.execute(
                    "UPDATE bases SET target = ?1 WHERE target = ?2",
                    params![normalised, raw],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn drop_cascading_bases(&mut self) -> Result<(), StoreError> {
        let cascades: bool = self
            .connection
            .prepare("SELECT COUNT(*) FROM pragma_foreign_key_list('bases')")?
            .query_row([], |row| row.get::<_, i64>(0))?
            > 0;

        if cascades {
            self.connection.execute_batch(
                "DROP TABLE bases;
                 CREATE TABLE bases (
                     target    TEXT NOT NULL,
                     memory_id TEXT NOT NULL,
                     checksum  TEXT NOT NULL,
                     PRIMARY KEY (target, memory_id)
                 );",
            )?;
        }
        Ok(())
    }

    /// Add `tombstones.forgot` to a store written before tombstones carried a checksum.
    ///
    /// The column is nullable and every existing row keeps `NULL`, which is the honest value: what
    /// those tombstones replaced was not recorded and cannot be recovered. They reconcile exactly as
    /// they did before — by base where one exists, and as a conflict where none does.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the migration fails.
    fn add_forgotten_checksum(&mut self) -> Result<(), StoreError> {
        let present: bool = self
            .connection
            .prepare("SELECT COUNT(*) FROM pragma_table_info('tombstones') WHERE name = 'forgot'")?
            .query_row([], |row| row.get::<_, i64>(0))?
            > 0;
        if !present {
            self.connection
                .execute_batch("ALTER TABLE tombstones ADD COLUMN forgot TEXT")?;
        }
        Ok(())
    }

    /// Add `target_set_id` to a `links` table created before set identity existed.
    ///
    /// **Every existing row keeps `NULL`, and that is the honest value** rather than a gap to fill:
    /// the authoring machine did not record an identity, and inferring one here from a set this
    /// machine happens to hold would be a receiver-side rewrite of somebody else's link — the exact
    /// act `absorb-as-supersession` removed. A link with only a name resolves as well as it ever
    /// did, which is what makes leaving them alone correct rather than merely cautious.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the migration fails.
    fn add_link_target_set_id(&mut self) -> Result<(), StoreError> {
        let present: bool = self
            .connection
            .prepare(
                "SELECT COUNT(*) FROM pragma_table_info('links') WHERE name = 'target_set_id'",
            )?
            .query_row([], |row| row.get::<_, i64>(0))?
            > 0;
        if !present {
            self.connection
                .execute_batch("ALTER TABLE links ADD COLUMN target_set_id TEXT")?;
        }
        Ok(())
    }

    /// Add the two columns the `used` label needs to a `query_log` written before it existed.
    ///
    /// The table is one commit old and holds nothing anyone would miss, so this could have dropped
    /// and recreated it. It does not, for the reason the neighbouring migrations exist: a store is
    /// opened by whatever binary is running, and a column that is merely *expected* to be there
    /// fails every session recall with `no such column` rather than degrading.
    fn add_query_log_labels(&mut self) -> Result<(), StoreError> {
        for (column, definition) in [
            ("session", "TEXT NOT NULL DEFAULT ''"),
            ("used", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            let present: bool = self
                .connection
                .prepare("SELECT COUNT(*) FROM pragma_table_info('query_log') WHERE name = ?1")?
                .query_row(params![column], |row| row.get::<_, i64>(0))?
                > 0;
            if !present {
                self.connection.execute_batch(&format!(
                    "ALTER TABLE query_log ADD COLUMN {column} {definition}"
                ))?;
            }
        }
        Ok(())
    }

    /// Rebuild a `links` table created with the cascading foreign key it should no longer have.
    ///
    /// Unlike [`Store::drop_cascading_bases`], the rows are **kept**: a base is local bookkeeping
    /// that can be re-established by the next export, while a link is authored content and losing
    /// one loses what somebody said. The table is rebuilt in place with the same columns and the
    /// same primary key, and only the foreign key goes.
    ///
    /// A store migrated here still holds no ancestry for memories forgotten *before* the migration
    /// — those edges were cascaded away when they were forgotten and nothing can recover them. The
    /// chain they were part of stays broken, which is worth knowing when reading an old set rather
    /// than assuming the fix is retroactive.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the migration fails.
    fn drop_cascading_links(&mut self) -> Result<(), StoreError> {
        let cascades: bool = self
            .connection
            .prepare("SELECT COUNT(*) FROM pragma_foreign_key_list('links')")?
            .query_row([], |row| row.get::<_, i64>(0))?
            > 0;

        if cascades {
            self.connection.execute_batch(
                "CREATE TABLE links_without_cascade (
                     source          TEXT NOT NULL,
                     label           TEXT NOT NULL,
                     target          TEXT NOT NULL,
                     target_set      TEXT,
                     target_keywords TEXT,
                     PRIMARY KEY (source, label, target)
                 );
                 INSERT INTO links_without_cascade
                     SELECT source, label, target, target_set, target_keywords FROM links;
                 DROP TABLE links;
                 ALTER TABLE links_without_cascade RENAME TO links;
                 CREATE INDEX IF NOT EXISTS links_by_target ON links(target);",
            )?;
        }
        Ok(())
    }

    /// Rekey `links` so the set a target lives in is part of a row's identity.
    ///
    /// `a-memory-linking-one-id-in-two-sets-keeps-one-link`: the key was `(source, label, target)`
    /// where `target` is a memory id alone, so a memory linking with one label to the same id in two
    /// different sets wrote two rows that collided, and `INSERT OR REPLACE` kept only the second.
    /// The loss was silent — `insert` returns `Ok`, and the link is simply not there afterwards.
    ///
    /// **The partial unique index is half the fix, not a tidy-up.** SQLite holds NULLs in a key
    /// distinct from one another, and a same-set link is a row whose `target_set` is NULL, so
    /// widening the key alone would stop collapsing two identical same-set links — swapping a silent
    /// loss for a silent duplicate. `links_same_set` covers exactly those rows.
    ///
    /// **Detection is the key's width**, three columns against four, read from `pragma_table_info`.
    /// Every row in an old table is unique under the old key, which is narrower than the new one, so
    /// the rebuild carries all of them and can neither drop nor merge anything.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the migration fails.
    fn rekey_links_by_target_set(&mut self) -> Result<(), StoreError> {
        let key_columns: i64 = self
            .connection
            .prepare("SELECT COUNT(*) FROM pragma_table_info('links') WHERE pk > 0")?
            .query_row([], |row| row.get(0))?;

        if key_columns >= 4 {
            return Ok(());
        }

        self.connection.execute_batch(
            "CREATE TABLE links_rekeyed (
                 source          TEXT NOT NULL,
                 label           TEXT NOT NULL,
                 target          TEXT NOT NULL,
                 target_set      TEXT,
                 target_keywords TEXT,
                 target_set_id   TEXT,
                 PRIMARY KEY (source, label, target, target_set)
             );
             INSERT INTO links_rekeyed
                 SELECT source, label, target, target_set, target_keywords, target_set_id
                 FROM links;
             DROP TABLE links;
             ALTER TABLE links_rekeyed RENAME TO links;
             CREATE INDEX IF NOT EXISTS links_by_target ON links(target);
             CREATE UNIQUE INDEX IF NOT EXISTS links_same_set
                 ON links(source, label, target) WHERE target_set IS NULL;",
        )?;
        Ok(())
    }

    /// Re-spell every id written before `crockford-base32-ids`.
    ///
    /// **Without this an older store opens healthy and answers nothing.** This build stringifies an
    /// id as 26 Crockford characters, so `WHERE id = ?` compares that against rows holding the
    /// previous 36-character hex spelling and matches none of them — every memory in the file
    /// becomes unreachable while nothing reports a fault. Same silent shape as
    /// [`Self::add_folded_keywords`], one column over.
    ///
    /// **Detection is by length**, which is exact rather than heuristic here: the two spellings are
    /// 36 and 26 characters and nothing else is ever written into these columns.
    ///
    /// **Foreign keys are off for the rewrite.** `memories.id` is the parent of three cascading
    /// children, and the schema declares no `ON UPDATE`, so updating a parent id with enforcement on
    /// fails against children that still hold the old spelling. Parent and child are rewritten
    /// together inside one transaction, so the window where they disagree never outlives it.
    ///
    /// A value that is the right length and not a parseable id is left alone rather than guessed at:
    /// that is corruption, and a migration is the wrong place to discover it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the migration fails.
    fn reencode_ids(&mut self) -> Result<(), StoreError> {
        /// Every column holding a memory id. `bases.target` is a directory path, not an id.
        const COLUMNS: &[(&str, &str)] = &[
            ("memories", "id"),
            ("keywords", "memory_id"),
            ("links", "source"),
            ("links", "target"),
            ("vectors", "memory_id"),
            ("tombstones", "id"),
            ("bases", "memory_id"),
        ];
        /// The previous spelling's length: `8-4-4-4-12` with its hyphens.
        const HEX_LEN: usize = 36;

        let mut pending: Vec<(&str, &str, String)> = Vec::new();
        for (table, column) in COLUMNS {
            let sql =
                format!("SELECT DISTINCT {column} FROM {table} WHERE length({column}) = {HEX_LEN}");
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                pending.push((table, column, row?));
            }
        }
        if pending.is_empty() {
            return Ok(());
        }

        // Outside the transaction deliberately: this pragma is a no-op inside one.
        self.connection.execute_batch("PRAGMA foreign_keys = OFF")?;
        let transaction = self.connection.transaction()?;
        for (table, column, previous) in &pending {
            let Ok(id) = MemoryId::parse(previous) else {
                continue;
            };
            transaction.execute(
                &format!("UPDATE {table} SET {column} = ?1 WHERE {column} = ?2"),
                params![id.to_string(), previous],
            )?;
        }
        transaction.commit()?;
        self.connection.execute_batch("PRAGMA foreign_keys = ON")?;
        Ok(())
    }

    /// Forget a memory: delete it, and leave a tombstone in its place.
    ///
    /// `documents/design/corrections.md`. Deleting rather than flagging is the point — the operation
    /// exists partly to remove sensitive data, so the content, the keywords and the vector all have
    /// to go. Several hundred floats reconstructed from a secret are still a copy of it.
    ///
    /// **What survives is the ancestry**, and only that: the memory's own `supersedes` and
    /// `superseded_by` edges, ids with no free text
    /// (`forgetting-a-chains-middle-makes-a-stale-conclusion-current-again`). Those edges live on
    /// the memory that authored them, so removing them removes a chain's middle and leaves a
    /// twice-replaced conclusion reporting itself as current. Every other link goes with the text,
    /// because a `SUPPORTS` edge is a judgement the memory made rather than the shape of the chain
    /// it sat in.
    ///
    /// Forgetting a memory that is not here is not an error: the outcome asked for is already the
    /// case, and a tombstone is still recorded so the fact propagates.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the write fails.
    pub fn forget(
        &mut self,
        id: MemoryId,
        by: MemoryId,
        at: jiff::Timestamp,
    ) -> Result<(), StoreError> {
        self.refuse_session_writes_if_read_only()?;
        // Read before the row goes, and narrowed by the constructor: what survives is the
        // supersession edges with their free text stripped. Everything else the memory authored --
        // its judgements about other memories, and any keywords carried on a cross-set target --
        // goes with the text, because forgetting is for removing what somebody wrote.
        // Checksummed as the whole record, which is what a receiver compares against: `stored` on
        // the other side is `checksum::of_record` over whatever they hold for this id. Forgetting an
        // id this store does not hold is not an error and leaves this `None` -- there is nothing to
        // say was removed, and saying so honestly is what keeps the field meaningful.
        let forgot = self.get(id)?.map(|memory| {
            crate::checksum::of_record(&crate::memory_file::Record::Memory(Box::new(memory)))
        });
        let tombstone = Tombstone::new(
            id,
            Attribution { by, at },
            self.links_of(&id.to_string())?,
            forgot,
        );
        self.entomb(&tombstone)
    }

    /// Apply a tombstone that arrived in a file.
    ///
    /// The record-at-a-time counterpart to [`accept_from_file`](Self::accept_from_file), and it
    /// exists for the same reason: **the ancestry has to come from the record, not from this
    /// store.** A machine adopting a set has never held the memory being forgotten, so deriving the
    /// edges locally derives none, and a chain that survived the export arrives with its middle
    /// missing anyway — the fix looking correct everywhere except across the boundary it exists to
    /// cross.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the write fails.
    pub fn accept_tombstone(&mut self, tombstone: &Tombstone) -> Result<(), StoreError> {
        // Rebuilt through the constructor even though the reader already did: this is a `pub` entry
        // point, so the rule about what a tombstone may carry has to hold for every caller rather
        // than for the ones that came through a file.
        let tombstone = Tombstone::new(
            tombstone.id,
            tombstone.forgotten.clone(),
            tombstone.ancestry.clone(),
            // Carried through rather than recomputed: it records what the *sender* removed, which
            // is the whole of its value. Recomputing it here would replace their answer with ours
            // and make every arriving tombstone agree with whatever this store happens to hold.
            tombstone.forgot,
        );
        self.entomb(&tombstone)
    }

    /// The write both forms share, so a locally forgotten memory and an arriving one cannot leave
    /// the store in two different shapes.
    fn entomb(&mut self, tombstone: &Tombstone) -> Result<(), StoreError> {
        let id = tombstone.id;
        let by = &tombstone.forgotten.by;
        let at = tombstone.forgotten.at;

        let holder = self.set_id()?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM memories WHERE id = ?1",
            params![id.to_string()],
        )?;
        // Explicit, because `links` no longer cascades. Deleting them all and writing back the
        // ancestry is deliberate over deleting selectively: one statement decides what a forgotten
        // memory leaves behind, and it is the same list the tombstone carries, so the store and the
        // file cannot come to disagree about it.
        transaction.execute(
            "DELETE FROM links WHERE source = ?1",
            params![id.to_string()],
        )?;
        write_links(
            &transaction,
            &id.to_string(),
            &tombstone.ancestry,
            &self.scope,
            holder,
        )?;
        transaction.execute(
            "INSERT INTO tombstones (id, forgotten_by, forgotten_at, forgot, file_position)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET forgotten_by = excluded.forgotten_by,
                                           forgotten_at = excluded.forgotten_at,
                                           forgot = excluded.forgot",
            params![
                id.to_string(),
                by.to_string(),
                at.to_string(),
                tombstone.forgot.map(|checksum| format!("{checksum:016x}")),
                stored_position(id),
            ],
        )?;
        transaction.commit()?;

        // Rebuilt rather than patched: the vector set is an ordered structure keyed by position, and
        // reloading is O(n) on an operation that is rare by design. Getting this wrong would leave a
        // forgotten memory's vector scoring against queries, which is the leak the deletion prevents.
        self.load_vectors()?;
        // Forgetting the last embedded memory cascades away the last vector, and a store holding
        // none is bound to no model.
        self.unbind_model_if_no_vectors()?;
        Ok(())
    }

    /// Every tombstone this store holds, in id order.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the query fails, or [`StoreError::Malformed`] if a stored
    /// value cannot be read back.
    /// The tombstone for one id, if this set forgot it.
    ///
    /// **Lookup needs this and `get` cannot supply it.** A forgotten memory has no row in
    /// `memories`, so asking for it by id answers *nothing here* — which is indistinguishable from
    /// an id this set never held, and wrong in a way that matters now that a memory's `by` can name
    /// a record `rename` forgot. What was forgotten is a different answer from what never existed.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn tombstone_for(&self, id: MemoryId) -> Result<Option<Tombstone>, StoreError> {
        let row = self
            .connection
            .query_row(
                "SELECT forgotten_by, forgotten_at, forgot FROM tombstones WHERE id = ?1",
                params![id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((by, at, forgot)) = row else {
            return Ok(None);
        };
        let forgot = forgot
            .map(|value| {
                u64::from_str_radix(&value, 16)
                    .map_err(|_| StoreError::MalformedBase { checksum: value })
            })
            .transpose()?;
        Ok(Some(Tombstone::new(
            id,
            Attribution {
                by: MemoryId::parse(&by)?,
                at: parse_timestamp(&at)?,
            },
            self.links_of(&id.to_string())?,
            forgot,
        )))
    }

    pub fn tombstones(&self) -> Result<Vec<Tombstone>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT id, forgotten_by, forgotten_at, forgot FROM tombstones ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;

        let mut tombstones = Vec::new();
        for row in rows {
            let (id, by, at, forgot) = row?;
            let forgot = forgot
                .map(|hex| {
                    u64::from_str_radix(&hex, 16)
                        .map_err(|_| StoreError::MalformedBase { checksum: hex })
                })
                .transpose()?;
            let id = MemoryId::parse(&id)?;
            // The edges are read from `links` like any other memory's, because that is where
            // `forget` left them -- a tombstone's ancestry is not a second copy of the graph but
            // the part of it whose source has stopped being a memory.
            tombstones.push(Tombstone::new(
                id,
                Attribution {
                    by: MemoryId::parse(&by)?,
                    at: at
                        .parse()
                        .map_err(|_| StoreError::MalformedBase { checksum: at })?,
                },
                self.links_of(&id.to_string())?,
                forgot,
            ));
        }
        Ok(tombstones)
    }

    /// Record agreement for some records without disturbing the rest.
    ///
    /// Unlike [`Store::set_bases`], which replaces a target's whole record because an export writes
    /// the whole file, an import agrees about only *some* records: a conflicted or locally-changed
    /// memory keeps the base it had, because that base still names the last point the two sides
    /// genuinely matched, and that is exactly what the next import needs in order to see the conflict
    /// again rather than silently accept one side.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the write fails.
    pub fn merge_bases(
        &mut self,
        target: &str,
        bases: &[(MemoryId, u64)],
    ) -> Result<(), StoreError> {
        let target = &Self::normalise_target(target);
        let transaction = self.connection.transaction()?;
        for (id, checksum) in bases {
            transaction.execute(
                "INSERT INTO bases (target, memory_id, checksum) VALUES (?1, ?2, ?3)
                 ON CONFLICT(target, memory_id) DO UPDATE SET checksum = excluded.checksum",
                params![target, id.to_string(), format!("{checksum:016x}")],
            )?;
        }
        transaction.execute(
            "INSERT INTO meta (key, value) VALUES ('checksum_algorithm', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![crate::checksum::ALGORITHM],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Record memories the export review cleared, each with the checksum of the record it cleared.
    ///
    /// Hex text, as [`Store::set_bases`] writes its checksums and for the same reason. A memory
    /// cleared again replaces its earlier clearance.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the write fails.
    pub fn record_clearances(&mut self, cleared: &[(MemoryId, u64)]) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        for (id, checksum) in cleared {
            transaction.execute(
                "INSERT INTO clearances (memory_id, checksum) VALUES (?1, ?2)
                 ON CONFLICT(memory_id) DO UPDATE SET checksum = excluded.checksum",
                params![id.to_string(), format!("{checksum:016x}")],
            )?;
            transaction.execute(
                "DELETE FROM reopened WHERE memory_id = ?1",
                params![id.to_string()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Every clearance the export review has recorded, by memory.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the query fails, or [`StoreError::Malformed`] if a stored
    /// value is not readable.
    pub fn clearances(&self) -> Result<std::collections::HashMap<MemoryId, u64>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT memory_id, checksum FROM clearances")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut cleared = std::collections::HashMap::new();
        for row in rows {
            let (key, checksum) = row?;
            let value = u64::from_str_radix(&checksum, 16)
                .map_err(|_| StoreError::MalformedBase { checksum })?;
            cleared.insert(MemoryId::parse(&key)?, value);
        }
        Ok(cleared)
    }

    /// Keep the preferences text this set's last finished review read, so the next can compare.
    ///
    /// Updated only when a review finishes, so one cut short is compared against the old text next
    /// time. It is the person's own text, kept in the store, which is never exported.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the write fails.
    pub fn set_review_snapshot(&mut self, text: &str) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO meta (key, value) VALUES ('review_preferences', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![text],
        )?;
        Ok(())
    }

    /// The preferences text this set's last finished review read, if one has finished.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the query fails.
    pub fn review_snapshot(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'review_preferences'",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Put memories back into review after a change to the preferences, until each is cleared again.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the write fails.
    pub fn reopen(&mut self, ids: &[MemoryId]) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        for id in ids {
            transaction.execute(
                "INSERT OR IGNORE INTO reopened (memory_id) VALUES (?1)",
                params![id.to_string()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// The memories a change to the preferences has put back into review.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the query fails, or [`StoreError::Malformed`] for an id
    /// this code did not write.
    pub fn reopened(&self) -> Result<HashSet<MemoryId>, StoreError> {
        let mut statement = self.connection.prepare("SELECT memory_id FROM reopened")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut found = HashSet::new();
        for row in rows {
            found.insert(MemoryId::parse(&row?)?);
        }
        Ok(found)
    }

    /// Forget every clearance, which is how a full review starts: every memory is reviewed again.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the write fails.
    pub fn forget_clearances(&mut self) -> Result<(), StoreError> {
        self.connection.execute("DELETE FROM clearances", [])?;
        Ok(())
    }

    /// One of the export review's own settings for this set, from `meta`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the query fails.
    pub fn review_setting(&self, key: ReviewSetting) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key.key()],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Set, or with `None` remove, one of the export review's own settings for this set.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the write fails.
    pub fn set_review_setting(
        &mut self,
        key: ReviewSetting,
        value: Option<&str>,
    ) -> Result<(), StoreError> {
        match value {
            Some(value) => self.connection.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key.key(), value],
            )?,
            None => self
                .connection
                .execute("DELETE FROM meta WHERE key = ?1", params![key.key()])?,
        };
        Ok(())
    }

    /// Every name this set has published under, current or past: the text of each identity record.
    ///
    /// For the export review's pre-pass, which flags a collaborator named in a memory's text. A past
    /// name identifies someone as surely as a current one, so superseded records count too.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the query fails.
    pub fn author_names(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT content FROM memories WHERE created_by = id ORDER BY id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The name this set currently publishes this store's author under: the
    /// [`Store::author_name`] of [`Store::author_record`], so a rename merged in from another copy
    /// is followed to its tip, and a record split in two names nobody.
    ///
    /// For the export review, which does not flag the person's own published name in their own
    /// set: it is their name as the author of their own work, already carried by every record's
    /// attribution. `the-review-flags-the-persons-own-published-name`.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn published_name(&self) -> Result<Option<String>, StoreError> {
        let Some(record) = self.author_record()? else {
            return Ok(None);
        };
        Ok(self
            .author_name(record)?
            .map(|name| name.as_str().to_owned()))
    }

    /// Every name in [`Store::author_names`] but the one this store's author currently publishes
    /// under: the names the export review's pre-pass flags.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn others_author_names(&self) -> Result<Vec<String>, StoreError> {
        let own = self.published_name()?;
        let mut names = self.author_names()?;
        names.retain(|name| Some(name) != own.as_ref());
        Ok(names)
    }

    /// The bases recorded for one target, by memory.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the query fails, or [`StoreError::Malformed`] if a stored
    /// value is not readable — which would mean the row was written by something other than this
    /// code.
    pub fn bases_for(
        &self,
        target: &str,
    ) -> Result<std::collections::HashMap<MemoryId, u64>, StoreError> {
        let target = &Self::normalise_target(target);
        let mut statement = self
            .connection
            .prepare("SELECT memory_id, checksum FROM bases WHERE target = ?1")?;
        let rows = statement.query_map(params![target], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut bases = std::collections::HashMap::new();
        for row in rows {
            let (key, checksum) = row?;
            let id = MemoryId::parse(&key)?;
            // Hex text, as `set_bases` writes it. A value that will not parse means the row came from
            // somewhere else, and treating it as absent would quietly re-baseline the record.
            let value = u64::from_str_radix(&checksum, 16)
                .map_err(|_| StoreError::MalformedBase { checksum })?;
            bases.insert(id, value);
        }
        Ok(bases)
    }

    /// Every memory id this store holds, in id order.
    ///
    /// Import needs it to find the records the store has and a file does not, which is a question
    /// about presence rather than content — so it is answered without assembling any memories.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the query fails.
    pub fn memory_ids(&self) -> Result<Vec<MemoryId>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM memories ORDER BY id")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<String>, _>>()?;
        ids.iter()
            .map(|key| MemoryId::parse(key).map_err(StoreError::from))
            .collect()
    }

    /// Whether any target has ever agreed about this record.
    ///
    /// This is the question `hoard_correct` asks to decide whether a record has **entered service**
    /// (`documents/design/corrections.md`): a base exists only where this store and some file have
    /// matched, so one means a second durable copy exists somewhere and a later merge may have to
    /// reason about this record without being able to ask anyone.
    ///
    /// Deliberately not `bases_for(target).contains_key(id)`: the property is *any* target, and a
    /// caller that had to name one would have to enumerate them, which turns a fact into a
    /// judgement about which targets count.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the query fails.
    pub fn has_base(&self, id: MemoryId) -> Result<bool, StoreError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM bases WHERE memory_id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// The targets this store has recorded an agreement with.
    ///
    /// Used to warn when an export names a directory this set has never been written to before —
    /// a second checkout is legitimate, a typo is not, and only the caller can tell which.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the query fails.
    pub fn known_targets(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT DISTINCT target FROM bases ORDER BY target")?;
        let targets = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<_, _>>()?;
        Ok(targets)
    }

    /// Visit every record — memory and tombstone alike — in file order, without materialising the set.
    ///
    /// Export writes what this yields, so tombstones travel with the memories and a forgotten memory
    /// propagates through the same reconciliation as any other change. **File order, not id order**,
    /// is [`crate::memory_file::file_position`], which each row stores, so the database does the
    /// ordering and this holds one record at a time (`file-position-column`).
    ///
    /// # Errors
    ///
    /// Returns whatever `visit` returns, or [`StoreError::Database`] if a query fails.
    pub fn stream_records<F>(&self, mut visit: F) -> Result<usize, StoreError>
    where
        F: FnMut(&Record) -> Result<(), StoreError>,
    {
        let mut statement = self.connection.prepare(
            "SELECT id, file_position, 0 FROM memories
             UNION ALL
             SELECT id, file_position, 1 FROM tombstones
             ORDER BY 2, 1",
        )?;
        let mut rows = statement.query([])?;

        let mut previous = None;
        let mut written = 0;
        while let Some(row) = rows.next()? {
            let id = MemoryId::parse(&row.get::<_, String>(0)?)?;
            let stored: Option<i64> = row.get(1)?;
            let is_tombstone = row.get::<_, i64>(2)? == 1;

            // The stored position is derived state, so it is checked against the id it came from
            // on the way out; and the order is checked against the record before, which also
            // refuses one id held as both a memory and a tombstone. See
            // [`StoreError::OutOfFileOrder`].
            let position = crate::memory_file::file_position(id);
            if stored != Some(stored_position(id)) || previous.is_some_and(|last| last >= position)
            {
                return Err(StoreError::OutOfFileOrder { id });
            }
            previous = Some(position);

            let record = if is_tombstone {
                match self.tombstone_for(id)? {
                    Some(tombstone) => Record::Tombstone(tombstone),
                    None => continue,
                }
            } else {
                match self.get(id)? {
                    Some(memory) => Record::Memory(Box::new(memory)),
                    None => continue,
                }
            };
            visit(&record)?;
            written += 1;
        }
        Ok(written)
    }

    /// How many memories this store holds, superseded ones included.
    ///
    /// Counts every row rather than only the currently-valid memories, because this answers "how
    /// much is in here" for a caller deciding whether a set is worth exporting or has loaded at all
    /// — a question about the contents of the database, not about what a recall would return.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the query fails.
    /// How many memories this store has forgotten: the tombstone count.
    ///
    /// **A sibling of [`Self::count`] rather than a field on it**, because they answer different
    /// questions and conflating them is `memories-count-means-two-different-things`: `count` is what
    /// the set holds, this is what it has removed, and a file written from the store carries both.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn forgotten_count(&self) -> Result<usize, StoreError> {
        let total: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM tombstones", [], |row| row.get(0))?;
        Ok(usize::try_from(total).unwrap_or(usize::MAX))
    }

    /// How many memories this set holds, **not counting identity records**.
    ///
    /// An identity record is a memory in the file and a record in every count that describes the
    /// *file* — `export` reports it, and a reader must round-trip it. This number describes what
    /// the caller has *stored*, which is a different question: a set reporting one more memory than
    /// anybody wrote is reporting infrastructure as content, and the person reading it did not put
    /// their own name there as a memory.
    ///
    /// The same line the embedder draws. See `embed_all`.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn count(&self) -> Result<usize, StoreError> {
        let total: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM memories WHERE created_by <> id",
            [],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(total).unwrap_or(usize::MAX))
    }

    /// Whether this set has never been written to, identity records aside: no memory and no tombstone.
    ///
    /// **What *the first write* means for a shared set's charter** (`set-settled-at-first-write`):
    /// its first memory has to be the charter. Identity records are left out because a write mints
    /// one before it stores anything, and a tombstone counts because forgetting is writing.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn holds_nothing_but_identities(&self) -> Result<bool, StoreError> {
        Ok(self.count()? == 0 && self.forgotten_count()? == 0)
    }

    /// Store a memory, replacing any earlier version of it.
    ///
    /// Replacement covers the memory's links: a withdrawn edge is gone from both directions,
    /// because there is only one copy of it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::SupersessionCycle`] if the memory's links would close a supersession
    /// cycle. Refusing here is cheaper than defending every use of validity resolution against
    /// non-termination — though resolution still detects cycles, because a hand-edited merge can
    /// deliver one this check never saw.
    ///
    /// Returns [`StoreError::UnknownControlMark`] if the memory carries a `hoard-` keyword that is
    /// not a control mark this version knows and that the set holds on no memory at all.
    ///
    /// Returns [`StoreError::SecondLiveCharter`] if the memory carries `hoard-scope-topics` while
    /// another live charter stands and the memory does not supersede it.
    pub fn insert(&mut self, memory: &Memory) -> Result<(), StoreError> {
        self.refuse_session_writes_if_read_only()?;
        self.check_no_cycle(memory)?;
        self.check_control_marks_known(memory)?;
        self.check_one_live_charter(memory)?;
        self.check_one_live_fit_hub(memory)?;
        self.check_fit_sentences(memory)?;
        self.check_fit_hub_complete(memory)?;
        self.check_fit_model_linked(memory)?;
        self.write(memory)
    }

    /// Store a replacement an operation carries forward from records the set already holds: the
    /// cycle refusal [`insert`](Self::insert) applies, without the one-live-charter refusal.
    ///
    /// **Absorbing is the caller.** It merges another set's records through the import path, so it
    /// can leave two live charters, and then supersedes this set's memories that linked into the
    /// absorbed set. When one of those is this set's charter, its replacement carries the charter mark
    /// beside the absorbed set's charter, and refusing it there would stop the absorb with the records
    /// already merged. The operation is carrying a charter forward, not authoring a second one, which
    /// is the reconciling side of the line `documents/design/conventions.md` draws. The two charters
    /// are then visible in `hoard_sets status`, and the next charter written has to supersede both.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::SupersessionCycle`] if the memory's links would close a supersession
    /// cycle.
    pub fn carry_forward(&mut self, memory: &Memory) -> Result<(), StoreError> {
        self.refuse_session_writes_if_read_only()?;
        self.check_no_cycle(memory)?;
        self.write(memory)
    }

    /// Refuse a record whose link names a set loaded for review.
    ///
    /// **The third boundary keeping the `eval-` prefix meaningful**, beside `create` and a file's
    /// header. It sits on the write both entry points share, so it covers a link a session authors
    /// and a link arriving in a file alike. Nothing legitimate names such a set: a review copy lives
    /// in one session's memory, so a stored link into one would dangle by the next start, while
    /// reading as a reference to a set that exists.
    fn refuse_links_into_an_evaluation_set(&self, memory: &Memory) -> Result<(), StoreError> {
        for link in &memory.links {
            if let LinkTarget::CrossSet { set, .. } = &link.target
                && set.is_evaluation()
            {
                return Err(StoreError::LinkNamesEvaluationSet {
                    set: set.as_str().to_owned(),
                });
            }
        }
        Ok(())
    }

    /// Refuse a write that introduces a `hoard-` keyword nobody knows: a typo, or an invented mark.
    ///
    /// `reserved-keyword-block-list`. A term is known if it is one of [`CONTROL_MARKS`], a member of
    /// one of [`CONTROL_MARK_FAMILIES`], or **already held by any memory in the set**, superseded ones
    /// included. The last is what keeps the check from painting keyword structure into a corner, and
    /// was decided 2026-09-14 by the human partner: copies of a set run different versions, so a mark
    /// a newer version introduced arrives at an older one by import and has to stay writable there,
    /// and a mark a later version retires -- as `hoard-watch` was -- must not strand the memories
    /// still carrying it. Amending or superseding a memory restates terms the set already holds, so
    /// neither is ever refused for a term it did not add.
    ///
    /// The price is that a mistyped term which arrived before this check, or by import, stays
    /// writable in that set. That is the same leniency import already needs.
    fn check_control_marks_known(&self, memory: &Memory) -> Result<(), StoreError> {
        for keyword in &memory.keywords {
            let folded = keyword.folded();
            if !folded.starts_with("hoard-") {
                continue;
            }
            let in_a_family = CONTROL_MARK_FAMILIES.iter().any(|family| {
                folded
                    .strip_prefix(family)
                    .is_some_and(|member| !member.is_empty())
            });
            if CONTROL_MARKS.contains(&folded.as_str()) || in_a_family {
                continue;
            }
            if self.holds_keyword(&folded)? {
                continue;
            }
            return Err(StoreError::UnknownControlMark {
                memory: memory.id,
                keyword: keyword.as_str().to_owned(),
            });
        }
        Ok(())
    }

    /// Whether any memory in the set, superseded or not, carries the keyword with this folded form.
    fn holds_keyword(&self, folded: &str) -> Result<bool, StoreError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM keywords WHERE folded = ?1)",
            params![folded],
            |row| row.get(0),
        )?)
    }

    /// Refuse a write that would leave this set with a second live charter.
    ///
    /// `one-live-charter-per-set`. Memories *about* charters were written carrying the charter mark
    /// three times in six days, in sets that already had one, and each was delivered to every session
    /// as though it described the set; the served `about-` rule did not stop it, because served text
    /// reaches a session before it writes and nothing stopped the write.
    ///
    /// A live charter is let stand beside the write only if it is the memory being written -- an
    /// amendment rewrites under its own id -- or one the write supersedes. Keyword search returns only
    /// memories that carry the mark themselves, so a charter already superseded by one without it has
    /// stopped counting.
    fn check_one_live_charter(&self, memory: &Memory) -> Result<(), StoreError> {
        if !memory
            .keywords
            .iter()
            .any(|keyword| keyword.folded() == CHARTER_KEYWORD)
        {
            return Ok(());
        }
        let replaced: HashSet<MemoryId> = memory
            .links
            .iter()
            .filter(|link| link.label == LinkLabel::Supersedes)
            .filter_map(|link| match link.target {
                LinkTarget::SameSet(target) => Some(target),
                LinkTarget::CrossSet { .. } => None,
            })
            .collect();
        for live in self.search_keywords(&[Keyword::new(CHARTER_KEYWORD)?], usize::MAX)? {
            let charter = live.memory.id;
            if charter != memory.id && !replaced.contains(&charter) {
                return Err(StoreError::SecondLiveCharter {
                    memory: memory.id,
                    charter,
                });
            }
        }
        Ok(())
    }

    /// Refuse a write that would leave this set with a second live model-fit hub, on the same terms
    /// as [`check_one_live_charter`](Self::check_one_live_charter): one per set, replaced by
    /// superseding it.
    fn check_one_live_fit_hub(&self, memory: &Memory) -> Result<(), StoreError> {
        if !carries(memory, FIT_HUB_MARK) {
            return Ok(());
        }
        let replaced = superseded_by(memory);
        for live in self.search_keywords(&[Keyword::new(FIT_HUB_MARK)?], usize::MAX)? {
            let hub = live.memory.id;
            if hub != memory.id && !replaced.contains(&hub) {
                return Err(StoreError::SecondLiveFitHub {
                    memory: memory.id,
                    hub,
                });
            }
        }
        Ok(())
    }

    /// Refuse a sentence-set memory that names no language, or several, or whose text holds no
    /// triplet. A set scored for the wrong language, or for none, would be a score about nothing.
    fn check_fit_sentences(&self, memory: &Memory) -> Result<(), StoreError> {
        if !carries(memory, FIT_SENTENCES_MARK) {
            return Ok(());
        }
        let malformed = |problem: String| StoreError::MalformedFitSentences {
            memory: memory.id,
            problem,
        };
        match language_tags(memory).len() {
            1 => {}
            0 => return Err(malformed("it has no `hoard-lang-` keyword".to_owned())),
            n => {
                return Err(malformed(format!(
                    "it has {n} `hoard-lang-` keywords, and a set tests one language"
                )));
            }
        }
        crate::language_fit::triplet_in(memory.content.as_data()).map_err(|problem| {
            malformed(format!("its text cannot be read as a triplet: {problem}"))
        })?;
        Ok(())
    }

    /// Refuse a hub listing a number of sentence sets other than zero or ten for any language.
    ///
    /// Counted per language and script, region ignored -- [`crate::language_fit::language_and_script`]
    /// -- and never as a total, which would accept seven of one language and thirteen of another.
    fn check_fit_hub_complete(&self, memory: &Memory) -> Result<(), StoreError> {
        if !carries(memory, FIT_HUB_MARK) {
            return Ok(());
        }
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for (tag, _) in self.fit_sentence_sets(memory)? {
            *counts.entry(tag).or_default() += 1;
        }
        let wrong: Vec<String> = counts
            .iter()
            .filter(|(_, count)| **count != crate::language_fit::TRIPLETS_PER_LANGUAGE)
            .map(|(tag, count)| format!("{count} for `{tag}`"))
            .collect();
        if wrong.is_empty() {
            return Ok(());
        }
        Err(StoreError::IncompleteFitHub {
            memory: memory.id,
            counts: format!("{} sentence sets", wrong.join(", ")),
        })
    }

    /// Refuse a recommendation that links to no live hub, which is the only way it is found.
    fn check_fit_model_linked(&self, memory: &Memory) -> Result<(), StoreError> {
        if !carries(memory, FIT_MODEL_MARK) || self.reaches_a_hub(memory)? {
            return Ok(());
        }
        let hub = self
            .search_keywords(&[Keyword::new(FIT_HUB_MARK)?], 1)?
            .first()
            .map(|hit| hit.memory.id);
        Err(StoreError::UnlinkedFitModel {
            memory: memory.id,
            hub,
        })
    }

    /// Whether any of `memory`'s links reaches a live hub, following a superseded hub to the one
    /// that replaced it -- so replacing the hub never strands what links to it.
    fn reaches_a_hub(&self, memory: &Memory) -> Result<bool, StoreError> {
        let hubs: HashSet<MemoryId> = self
            .search_keywords(&[Keyword::new(FIT_HUB_MARK)?], usize::MAX)?
            .into_iter()
            .map(|hit| hit.memory.id)
            .collect();
        for target in self.targets_in_this_set(memory)? {
            if self
                .current_versions(target)?
                .iter()
                .any(|id| hubs.contains(id))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The ids `memory` links to in this set, asked the way the write asks it.
    ///
    /// **A link naming this set is a same-set link** -- by identity where both have one, by name
    /// otherwise, `LinkTarget::is_foreign_to` -- and the write stores it as one. Checks that read
    /// only the bare spelling refused a recommendation the write would have stored correctly: a
    /// small model linked to the hub with `target_set` naming its own set and was refused five
    /// times running, each refusal naming the id it had already used (2026-10-04).
    fn targets_in_this_set(&self, memory: &Memory) -> Result<Vec<MemoryId>, StoreError> {
        let holder = self.set_id()?;
        Ok(memory
            .links
            .iter()
            .filter(|link| !link.target.is_foreign_to(self.scope(), holder))
            .map(|link| link.target.id())
            .collect())
    }

    /// What `target` now reads as: itself if current, its replacements if superseded, nothing if
    /// it sits in a cycle.
    fn current_versions(&self, target: MemoryId) -> Result<Vec<MemoryId>, StoreError> {
        Ok(match self.resolve_supersession(target)? {
            Resolution::Current => vec![target],
            Resolution::Superseded { tips, .. } => tips,
            Resolution::Cycle => Vec::new(),
        })
    }

    /// The sentence sets a hub lists, each with the language it tests, as language and script.
    ///
    /// **Followed to the current version**, so a set superseded since the hub was written counts
    /// as its replacement -- which is how one bad set is fixed without rewriting the hub. Only
    /// memories carrying `hoard-fit-sentences` count; a hub may link to anything else as well.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if a linked memory cannot be read.
    pub fn fit_sentence_sets(&self, hub: &Memory) -> Result<Vec<(String, Memory)>, StoreError> {
        let mut seen = HashSet::new();
        let mut sets = Vec::new();
        for target in self.targets_in_this_set(hub)? {
            for id in self.current_versions(target)? {
                if !seen.insert(id) {
                    continue;
                }
                let Some(set) = self.get(id)? else {
                    continue;
                };
                if !carries(&set, FIT_SENTENCES_MARK) {
                    continue;
                }
                if let [tag] = language_tags(&set).as_slice() {
                    let (language, script) = crate::language_fit::language_and_script(tag);
                    let key =
                        script.map_or(language.clone(), |script| format!("{language}-{script}"));
                    sets.push((key, set));
                }
            }
        }
        Ok(sets)
    }

    /// This set's own sentence sets, grouped by the language they test, and any no hub lists.
    ///
    /// Read from every live hub rather than one, for the reason [`Store::declared_languages`] reads
    /// every charter: a merge can leave two, and choosing would hide what the other lists.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] if the hub or its sets cannot be read.
    pub fn fit_material(&self) -> Result<FitMaterial, StoreError> {
        let mut material = FitMaterial::default();
        let mut listed = HashSet::new();
        for hub in self.search_keywords(&[Keyword::new(FIT_HUB_MARK)?], usize::MAX)? {
            material.hubs.push(hub.memory.id);
            for (key, set) in self.fit_sentence_sets(&hub.memory)? {
                if listed.insert(set.id) {
                    material.groups.entry(key).or_default().push(set);
                }
            }
        }
        for set in self.search_keywords(&[Keyword::new(FIT_SENTENCES_MARK)?], usize::MAX)? {
            if !listed.contains(&set.memory.id) {
                material.orphans.push(set.memory.id);
            }
        }
        for hit in self.search_keywords(&[Keyword::new(FIT_MODEL_MARK)?], usize::MAX)? {
            if self.reaches_a_hub(&hit.memory)? {
                material.recommendations.push(Recommendation {
                    id: hit.memory.id,
                    summary: hit.memory.content.summary_as_data(),
                    tags: language_tags(&hit.memory),
                });
            }
        }
        Ok(material)
    }

    /// Store a memory that arrived in a file, without the cycle refusal [`insert`](Self::insert)
    /// applies.
    ///
    /// **The difference is the caller, not the record.** Authoring one edge and reconciling a whole
    /// file are different situations, and the cycle refusal is right for exactly one of them: a
    /// caller writing a `SUPERSEDES` link holds both ends of it and can fix the one they meant,
    /// while a file is reconciled record by record and a refusal lands on whichever record the
    /// cycle happened to close on — aborting the import with the store already partly written, for
    /// an edge somebody else wrote in a merge. `two-import-paths-disagree-about-supersession-cycles`
    /// in `TRACKING.md` records that happening, and `documents/design/cross-set-references.md` records
    /// the general form: a per-record judgement during import costs the whole file.
    ///
    /// A cycle that arrives this way is detected by validity resolution and reported to the caller
    /// rather than resolved, which is what [`Resolution::Cycle`] exists to describe.
    ///
    /// # Errors
    ///
    /// See [`StoreError`]. Notably **not** [`StoreError::SupersessionCycle`], which is the point.
    pub fn accept_from_file(&mut self, memory: &Memory) -> Result<(), StoreError> {
        self.write(memory)
    }

    /// The write both entry points share, so they cannot drift apart in anything but the guard.
    fn write(&mut self, memory: &Memory) -> Result<(), StoreError> {
        self.refuse_links_into_an_evaluation_set(memory)?;
        // **No `prepare` here, since 2026-09-22** (`where-the-gap-sweep-runs` in `TRACKING.md`). A
        // write embeds its own memory below and needs no other memory's vector, so filling gaps
        // bought it nothing; the sweep runs before a search by meaning, which is what reads them.
        // Nor does it need `refresh`: everything it consults is read from the database, `cache`
        // only inserts, and a later `refresh` reloads if another connection committed meanwhile.
        // The comment this replaces said a mismatched model resolved itself here, which stopped
        // being true on 2026-08-09.
        self.mint_set_id()?;
        if self.would_destroy_an_unrebuildable_vector(memory)? {
            return Err(StoreError::CannotRebuildEmbedding { memory: memory.id });
        }
        // **Embedded before anything is committed**, since 2026-09-11. The memory used to be
        // committed first, so an embedding that failed left it stored behind an error saying it was
        // not — and, with no id returned, beyond the reach of any tool that could remove it.
        // `one-unembeddable-memory-stops-the-store`. Embedding is pure computation, so asking first
        // costs nothing, and the memory and its row then land together or not at all.
        let embeddings = self.embed_for(&[memory])?;
        self.persist(Some(memory), embeddings)?;
        Ok(())
    }

    /// Replace this store's contents with an imported memory set.
    ///
    /// Import **replaces** rather than merges: reconciliation happens in the file, by `git`, before
    /// it gets here (`documents/design/multi-set-sync.md`), so a memory absent from the file has
    /// been forgotten and must not survive in the store.
    ///
    /// Deliberately does *not* refuse a set containing a supersession cycle. A cycle can arrive
    /// from a hand-edited merge, and refusing the whole file would make one bad edge cost every
    /// memory in the set; resolution reports the cycle instead.
    ///
    /// [`accept_from_file`](Self::accept_from_file) is the record-at-a-time form of that same rule,
    /// and it exists because this comment was true only of this function: the reconciling import
    /// went through [`insert`](Self::insert) and did refuse, which is
    /// `two-import-paths-disagree-about-supersession-cycles` in `TRACKING.md`. Two write paths that
    /// disagree about a documented rule is worth more care than either path alone.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ScopeMismatch`] if the set belongs to a different scope.
    pub fn import(&mut self, set: &MemorySet) -> Result<(), StoreError> {
        // Both shapes, because a `MemorySet` is a whole file and a file's tombstones are part of
        // what it says. Taking only `memories` here is the store half of
        // `memory-file-read-silently-discards-tombstones`, and it would resurrect everything the
        // set had forgotten on the next export.
        let records: Vec<crate::memory_file::Record> = set
            .memories
            .iter()
            .map(|memory| crate::memory_file::Record::Memory(Box::new(memory.clone())))
            .chain(
                set.tombstones
                    .iter()
                    .map(|tombstone| crate::memory_file::Record::Tombstone(tombstone.clone())),
            )
            .collect();
        self.replace_with(&set.scope, &records)?;
        Ok(())
    }

    /// Replace everything this store holds with the records of a file.
    ///
    /// **This is `restore`**, and [`import`](Self::import) is now a thin caller
    /// of it — because two functions that each replace a store's contents is how the tombstone half
    /// of `memory-file-read-silently-discards-tombstones` came about, and one write path is the fix
    /// rather than two that agree today.
    ///
    /// **Records, not memories.** A tombstone is a record like any other and has to survive the
    /// round trip: it is what makes a link into a forgotten memory a *good* reference rather than a
    /// dangling one (`documents/design/cross-set-references.md`, and [`holds_record`](Self::holds_record)
    /// counts one for exactly that reason). Taking a `MemorySet` here cannot express that, since
    /// that type carries no tombstones.
    ///
    /// **Every table the set lives in is cleared, `tombstones` included.** Leaving them is not a
    /// smaller version of the same thing: a memory this store had forgotten and the file still
    /// carries live would end up holding *both* a memory row and a tombstone row under one id, and
    /// [`stream_records`](Self::stream_records) emits both — so the next export writes a file with a
    /// duplicate id, which this project's own reader refuses (`documents/design/file-format.md`).
    ///
    /// **`bases` is cleared too, and that is a decision rather than tidying.** A base says *this
    /// record's content agreed with that target at this checksum*, and replacing the contents makes
    /// every one of them unprovable. Keeping them is worse than losing them: a base for a record the
    /// restored set does not hold resolves as [`Anomaly`](crate::reconcile::Outcome::Anomaly) —
    /// *"a state no supported operation produces"* — so restore would manufacture the very report
    /// that exists to mean something went wrong outside the tools. `documents/design/store-registry.md`,
    /// *never keep a reconciliation base you cannot prove*. The cost is that the next import from
    /// another checkout reconciles with no ancestor, which degrades to `Converged` or a reported
    /// `Conflict` — never to a silent overwrite.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::ScopeMismatch`] if the records belong to a different scope.
    pub fn replace_with(
        &mut self,
        scope: &Scope,
        records: &[crate::memory_file::Record],
    ) -> Result<Replaced, StoreError> {
        if *scope != self.scope {
            return Err(StoreError::ScopeMismatch {
                found: scope.clone(),
            });
        }
        // Before any record: each is canonicalised against the identity of the set holding it.
        self.mint_set_id()?;

        let mut replaced = Replaced {
            memories: 0,
            forgotten: 0,
        };

        let holder = self.set_id()?;
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM memories", [])?;
        transaction.execute("DELETE FROM links", [])?;
        transaction.execute("DELETE FROM keywords", [])?;
        transaction.execute("DELETE FROM tombstones", [])?;
        transaction.execute("DELETE FROM bases", [])?;
        for record in records {
            match record {
                crate::memory_file::Record::Memory(memory) => {
                    // Written as the file gives it. `write_memory` canonicalises a target that is
                    // *this* set -- by identity where both have one -- and nothing else:
                    // unconditional and deterministic, so every holder of one file stores identical
                    // bytes. A merged-away set's links are made internal by `hoard_sets` action
                    // "absorb", which authors a supersession rather than editing anyone's record on
                    // the way in.
                    write_memory(&transaction, memory, &self.scope, holder)?;
                    // Counted as a memory only if it is one. An identity record is written like
                    // any other record -- it has to survive the round trip -- but `memories` here
                    // answers *how much did this set take on*, and nobody wrote their own name as
                    // a memory. The same line `count` and `embed_all` draw.
                    if !memory.is_identity_record() {
                        replaced.memories += 1;
                    }
                }
                crate::memory_file::Record::Tombstone(tombstone) => {
                    transaction.execute(
                        "INSERT INTO tombstones (id, forgotten_by, forgotten_at, forgot, file_position)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(id) DO UPDATE SET forgotten_by = excluded.forgotten_by,
                                                       forgotten_at = excluded.forgotten_at,
                                                       forgot = excluded.forgot",
                        params![
                            tombstone.id.to_string(),
                            tombstone.forgotten.by.to_string(),
                            tombstone.forgotten.at.to_string(),
                            tombstone.forgot.map(|checksum| format!("{checksum:016x}")),
                            stored_position(tombstone.id),
                        ],
                    )?;
                    // The ancestry arrives as edges like any other, because that is what it is: the
                    // part of the graph whose source has stopped being a memory. Written here as
                    // well as by `forget` so a chain survives the round trip through a file, which
                    // is the only way it reaches another machine at all.
                    write_links(
                        &transaction,
                        &tombstone.id.to_string(),
                        &tombstone.ancestry,
                        &self.scope,
                        holder,
                    )?;
                    replaced.forgotten += 1;
                }
            }
        }
        transaction.execute("DELETE FROM vectors", [])?;
        transaction.commit()?;

        self.vectors = VectorSet::new(0);
        let memories: Vec<Memory> = records
            .iter()
            .filter_map(|record| match record {
                crate::memory_file::Record::Memory(memory) => Some((**memory).clone()),
                crate::memory_file::Record::Tombstone(_) => None,
            })
            .collect();
        let borrowed: Vec<&Memory> = memories.iter().collect();
        self.embed_all(&borrowed)?;
        // With no model in hand `embed_all` is a no-op, so the store would otherwise be left
        // claiming a model while holding nothing it produced.
        self.unbind_model_if_no_vectors()?;
        Ok(replaced)
    }

    /// Whether this store holds any record under `id` — a memory or a tombstone.
    ///
    /// **A tombstone counts.** A link into a supersession chain whose middle was forgotten is a good
    /// reference, and answering only for live memories would report it as broken
    /// (`documents/design/cross-set-references.md`). This is the resolution side of the link check;
    /// it deliberately says nothing about *what* the record is, because a caller asking whether a
    /// target exists has no business being handed the target.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn holds_record(&self, id: MemoryId) -> Result<bool, StoreError> {
        let key = id.to_string();
        Ok(self
            .connection
            .prepare(
                "SELECT 1 FROM memories WHERE id = ?1
                 UNION ALL
                 SELECT 1 FROM tombstones WHERE id = ?1",
            )?
            .exists([&key])?)
    }

    /// Whether this store holds a **live memory** under `id` — a tombstone does not count.
    ///
    /// The narrower of the two questions [`holds_record`](Self::holds_record) answers, and the one
    /// resolution asks: not *is this reference good* but *can this be handed to a reader*. A tip
    /// that cannot be fetched is worse than no tip, since substituting it drops the memory that
    /// yielded to it.
    fn holds_memory(&self, id: MemoryId) -> Result<bool, StoreError> {
        Ok(self
            .connection
            .prepare("SELECT 1 FROM memories WHERE id = ?1")?
            .exists([&id.to_string()])?)
    }

    /// Fetch a memory by id.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn get(&self, id: MemoryId) -> Result<Option<Memory>, StoreError> {
        let key = id.to_string();
        let memory = self
            .connection
            .query_row(
                "SELECT content, why, context, confidence, origin,
                        created_by, created_at, modified_by, modified_at
                 FROM memories WHERE id = ?1",
                params![key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, f64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                },
            )
            .optional()?;

        let Some((content, why, context, confidence, origin, cby, cat, mby, mat)) = memory else {
            return Ok(None);
        };

        Ok(Some(Memory {
            id,
            content: MemoryText::new(content)?,
            why: why.map(MemoryText::new).transpose()?,
            context: context.map(MemoryText::new).transpose()?,
            keywords: self.keywords_of(&key)?,
            confidence: Confidence::new(confidence)?,
            origin: parse_enum("origin", &origin)?,
            created: Attribution {
                by: MemoryId::parse(&cby)?,
                at: parse_timestamp(&cat)?,
            },
            modified: Attribution {
                by: MemoryId::parse(&mby)?,
                at: parse_timestamp(&mat)?,
            },
            links: self.links_of(&key)?,
        }))
    }

    /// The edges pointing *at* this memory, as source and label.
    ///
    /// The other half of the both-endpoints guarantee. The label is the one that was written, not a
    /// synthesised inverse — a derived reverse must stay distinguishable from an authored one, or
    /// an authored `SUPERSEDED_BY` (the lower-authority side acknowledging replacement) becomes
    /// indistinguishable from the mechanical inverse of someone else's claim.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn incoming(&self, id: MemoryId) -> Result<Vec<(MemoryId, LinkLabel)>, StoreError> {
        // **`target_set IS NULL` is load-bearing, not tidiness.** Without it a row whose target
        // names *another* set matches when its id happens to equal one held here -- the id-collision
        // case in `intentional-edits-to-a-memory-file-are-largely-unprobed` -- and this is the
        // lookup `resolve_supersession` walks, so the collision would change a memory's standing.
        // Same rule as `edges`: an edge only speaks about a target in its own set.
        let mut statement = self.connection.prepare(
            "SELECT source, label FROM links WHERE target = ?1 AND target_set IS NULL \
             ORDER BY source",
        )?;
        let rows = statement.query_map(params![id.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut edges = Vec::new();
        for row in rows {
            let (source, label) = row?;
            edges.push((MemoryId::parse(&source)?, parse_enum("link label", &label)?));
        }
        Ok(edges)
    }

    /// The edges pointing at this memory, in the shape retrieval hands to a client.
    ///
    /// Deliberately unfiltered by label. Excluding `supersedes` because `superseded_by` already
    /// reports it would rebuild `incoming-edges-invisible-on-the-wire` -- a hard-coded slice of the
    /// reverse index -- and the two answer different questions: `superseded_by` reports validity,
    /// this reports edges.
    ///
    /// **Public because the write surface reports these too**, and must report them in the same
    /// shape: `hoard_correct` names what points at a memory whose claim it just rewrote. A second
    /// projection of one structure at a second call site is `lookup-and-recall-disagree-on-shape`,
    /// which is why the filtering a caller wants happens on the returned edges rather than here.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn incoming_edges(&self, id: MemoryId) -> Result<Vec<IncomingEdge>, StoreError> {
        Ok(self
            .incoming(id)?
            .into_iter()
            .map(|(source, label)| IncomingEdge { source, label })
            .collect())
    }

    /// The standing-changing edges in *this* store that name a target in `target_set`.
    ///
    /// The deliberate counterpart to [`Store::incoming`], which refuses cross-set rows outright so
    /// that none of them can change a memory's standing. Reporting needs to *see* exactly those
    /// rows, so this is the one lookup that reads them -- and it reads them into a report rather
    /// than into resolution, which is the whole distinction the design turns on.
    ///
    /// Filtered on label here rather than by the caller, so the set of labels that constitute a
    /// claim lives in one place ([`DISPUTING_LABELS`]) and a new label cannot become a dispute by
    /// omission.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    fn claims_against(
        &self,
        id: MemoryId,
        target_set: &SetName,
    ) -> Result<Vec<(MemoryId, LinkLabel)>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT source, label FROM links WHERE target = ?1 AND target_set = ?2 ORDER BY source",
        )?;
        let rows = statement.query_map(params![id.to_string(), target_set.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut claims = Vec::new();
        for row in rows {
            let (source, label) = row?;
            let label: LinkLabel = parse_enum("link label", &label)?;
            if DISPUTING_LABELS.contains(&label) {
                claims.push((MemoryId::parse(&source)?, label));
            }
        }
        Ok(claims)
    }

    /// Every edge in *this* store pointing at `id` in another set, whatever its label.
    ///
    /// **The reporting counterpart to [`Store::incoming`], which refuses cross-set rows outright.**
    /// That refusal is right and must stay: `incoming` is what `resolve_supersession` walks, so a
    /// cross-set row whose target id collides with one held here would let another set's edge change
    /// a memory's standing. This reads exactly those rows into a *report*, which is the distinction
    /// [`Store::claims_against`] already turns on — the difference being that claims are filtered to
    /// the labels that dispute, and a backlink is any edge at all.
    ///
    /// **Matched by identity where the link states one**, on the rule `holder_of` gives: a set's
    /// local name can be stale and can be shared, so a row naming *this* set by a name it no longer
    /// uses still points here, and a row naming some other set that happens to share the name does
    /// not. Where the row carries no identity the name is all there is, as before.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    fn edges_into_another_set(
        &self,
        id: MemoryId,
        target_set: &SetName,
        target_set_id: Option<SetId>,
    ) -> Result<Vec<IncomingEdge>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT source, label, target_set, target_set_id FROM links \
             WHERE target = ?1 AND target_set IS NOT NULL ORDER BY source",
        )?;
        let rows = statement.query_map(params![id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;

        let mut found = Vec::new();
        for row in rows {
            let (source, label, named, stated) = row?;
            let stated = stated.as_deref().map(SetId::parse).transpose()?;
            let points_here = match (stated, target_set_id) {
                // Both sides state an identity: that alone decides it, and the names are ignored.
                (Some(stated), Some(ours)) => stated == ours,
                // The row names an identity this set does not have, or the reverse. Fall back to the
                // name, which is the best either side can offer.
                _ => named.as_deref() == Some(target_set.as_str()),
            };
            if points_here {
                found.push(IncomingEdge {
                    source: MemoryId::parse(&source)?,
                    label: parse_enum("link label", &label)?,
                });
            }
        }
        Ok(found)
    }

    /// Resolve a memory's validity: is it current, replaced, or part of a cycle?
    ///
    /// Follows supersession forward to the end of the chain, so a memory replaced twice resolves to
    /// the newest version rather than to an intermediate one that was itself corrected.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn resolve_supersession(&self, id: MemoryId) -> Result<Resolution, StoreError> {
        Ok(match self.walk_supersession(id)? {
            Walked::Current => Resolution::Current,
            Walked::Cycle => Resolution::Cycle,
            Walked::Superseded {
                tips,
                elsewhere,
                absent,
                forgotten,
            } => Resolution::Superseded {
                tips,
                unresolved: elsewhere || absent,
                forgotten,
            },
        })
    }

    /// The walk behind [`Store::resolve_supersession`], with every way a chain can stop kept apart.
    fn walk_supersession(&self, id: MemoryId) -> Result<Walked, StoreError> {
        let mut tips = Vec::new();
        let (mut elsewhere, mut absent, mut forgotten) = (false, false, false);
        let mut seen = HashSet::from([id]);
        let mut frontier = vec![id];
        let mut superseded = false;
        let mut depth = 0;
        // Every same-set replacement the walk crossed, so a loop can be looked for once everything
        // reachable is known. **Reaching a memory twice is not a loop**: a fork that a later memory
        // heals reaches the healing memory once from each branch, and until 2026-09-14 the `seen`
        // set answered both questions and reported that diamond as a cycle --
        // `a-healed-fork-reports-its-ancestor-as-a-cycle`. `seen` now only stops a memory being
        // walked twice, and [`Store::closes_a_loop`] decides whether the edges go round.
        let mut edges: Vec<(MemoryId, MemoryId)> = Vec::new();

        while let Some(current) = frontier.pop() {
            depth += 1;
            if depth > MAX_SUPERSESSION_DEPTH {
                return Ok(Walked::Cycle);
            }

            let mut replacements = Vec::new();

            // An edge written on the newer memory: `A SUPERSEDES current`.
            for (source, label) in self.incoming(current)? {
                if label == LinkLabel::Supersedes {
                    replacements.push(Some(source));
                }
            }
            // An edge written on this memory: `current SUPERSEDED_BY A`. The only form that
            // survives the replacement's set being unavailable, which is why it is checked here
            // rather than left to the reverse direction.
            for link in self.links_of(&current.to_string())? {
                if link.label == LinkLabel::SupersededBy {
                    // A same-set target is followed whether or not this store holds it. Refusing to
                    // walk into an id nothing answers to was tried and removed: it decides at the
                    // wrong end. What the reader needs is a replacement that can be *fetched*, which
                    // is settled below where the chain stops, and an id this store holds no record
                    // of can still be superseded in turn by one it does.
                    replacements.push(match &link.target {
                        LinkTarget::SameSet(target) => Some(*target),
                        LinkTarget::CrossSet { .. } => None,
                    });
                }
            }

            if replacements.is_empty() && current != id {
                // Walkable and returnable are two questions, and the end of a chain is where the
                // second one is asked. A chain is followed through an id this store cannot produce
                // -- a forgotten memory, or one a hand-edited file names and nothing holds -- but a
                // chain that *ends* on one has no replacement to hand back. Naming it as a tip made
                // the fetch in `resolved_hit` fail and took the memory out of every search path
                // without a word: `a-superseded-by-naming-an-absent-id-hides-its-memory`, whose
                // commonest spelling is the malformation `documents/design/cross-set-references.md`
                // names -- a cross-set reference written without its set name. Reported instead,
                // which is the treatment a cross-set replacement already gets, and it leaves the
                // memory findable.
                if self.holds_memory(current)? {
                    tips.push(current);
                } else if self.holds_record(current)? {
                    // Held, but only as a tombstone: the replacement was withdrawn here, which is
                    // not the same thing as being held somewhere else.
                    forgotten = true;
                } else {
                    absent = true;
                }
            }
            if !replacements.is_empty() {
                superseded = true;
            }

            for replacement in replacements {
                let Some(replacement) = replacement else {
                    elsewhere = true;
                    continue;
                };
                edges.push((current, replacement));
                if seen.insert(replacement) {
                    frontier.push(replacement);
                }
            }
        }

        // Asked of everything reachable, not only of the path to a tip: a loop anywhere downstream
        // means no replacement can be trusted as the current one, which is what `Cycle` says.
        if Self::closes_a_loop(&seen, &edges) {
            return Ok(Walked::Cycle);
        }

        if !superseded {
            return Ok(Walked::Current);
        }

        tips.sort();
        tips.dedup();
        Ok(Walked::Superseded {
            tips,
            elsewhere,
            absent,
            forgotten,
        })
    }

    /// Whether these replacement edges, among these memories, go round.
    ///
    /// Kahn's algorithm: take away every memory nothing remaining replaces, and repeat. What cannot
    /// be taken away sits on a loop. A diamond empties completely -- the memory both branches reach
    /// becomes removable once both branches are gone -- which is the distinction a visited set
    /// cannot draw.
    fn closes_a_loop(memories: &HashSet<MemoryId>, edges: &[(MemoryId, MemoryId)]) -> bool {
        let mut replaced_by: HashMap<MemoryId, usize> =
            memories.iter().map(|memory| (*memory, 0)).collect();
        let mut replaces: HashMap<MemoryId, Vec<MemoryId>> = HashMap::new();
        for (older, newer) in edges {
            *replaced_by.entry(*newer).or_default() += 1;
            replaces.entry(*older).or_default().push(*newer);
        }

        let mut removable: Vec<MemoryId> = replaced_by
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(memory, _)| *memory)
            .collect();
        let mut removed = 0;
        while let Some(memory) = removable.pop() {
            removed += 1;
            for newer in replaces.get(&memory).into_iter().flatten() {
                let count = replaced_by
                    .get_mut(newer)
                    .expect("every edge's target was counted above");
                *count -= 1;
                if *count == 0 {
                    removable.push(*newer);
                }
            }
        }
        removed < replaced_by.len()
    }

    /// The replacements this memory concedes to **in other sets** -- its `SUPERSEDED_BY` targets
    /// that name a set rather than a bare id.
    ///
    /// Read by the cross-store layer, which is the only layer that can say whether the named set is
    /// among those being searched. A single store cannot: `resolve_supersession` maps every such
    /// target to *not available here* because, from where it stands, that is true.
    fn conceded_to_elsewhere(
        &self,
        id: MemoryId,
    ) -> Result<Vec<(SetName, Option<SetId>, MemoryId)>, StoreError> {
        let mut targets = Vec::new();
        for link in self.links_of(&id.to_string())? {
            if link.label != LinkLabel::SupersededBy {
                continue;
            }
            // The identity travels with the name, because the name alone cannot say *which* set --
            // it can be stale, and it can be shared. See `holder_of`.
            if let LinkTarget::CrossSet {
                set,
                id: target,
                set_id,
                ..
            } = link.target
            {
                targets.push((set, set_id, target));
            }
        }
        Ok(targets)
    }

    /// Find memories carrying any of these keywords, most matches first.
    ///
    /// One of the three recall paths in `documents/design/recall-mechanisms.md`, and one of the two
    /// that need no embedding model — which is what makes a missing model a degradation rather than
    /// a failure.
    ///
    /// This is *search*, so it resolves validity the same way semantic recall does: a superseded
    /// memory is replaced by whatever corrected it.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn search_keywords(
        &self,
        keywords: &[Keyword],
        limit: usize,
    ) -> Result<Vec<KeywordHit>, StoreError> {
        let mut cursor = KeywordCursor::new(self, keywords);
        let mut hits = Vec::new();
        while hits.len() < limit {
            match cursor.next_hit(&[])? {
                Some(hit) => hits.push(hit),
                None => break,
            }
        }
        Ok(hits)
    }

    /// Retrieve a memory by its whole id.
    ///
    /// **Prefix resolution used to live here and was removed**, not repaired: a prefix of a
    /// time-ordered id constrains *when* a memory was written rather than which one it is, measured
    /// at 82 collisions in 84 for a four-character head. Recovery from a partial id is
    /// [`find_by_fragment`](Self::find_by_fragment), which is a substring search the caller chooses
    /// deliberately rather than a truncation that resolves by accident. See
    /// `lookup-prefix-selectivity-is-time-correlated` in `TRACKING.md`.
    ///
    /// **Exempt from supersession substitution.** Asking for a memory by id means wanting *that*
    /// memory, so it comes back flagged rather than replaced. Without the exemption the prior
    /// version would be unreachable, which would make superseding equivalent to forgetting and
    /// defeat the choice to keep it.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn lookup(&self, id: MemoryId) -> Result<Option<Found>, StoreError> {
        self.found_for(id)
    }

    /// Find memories whose id *contains* `fragment` — the recovery path, never the default.
    ///
    /// `crockford-base32-ids`. This replaced prefix matching rather than joining it, and the reason
    /// is measured: with a UUIDv7's bits, the head of an id is the timestamp and the tail is the
    /// entropy, so **the front is the one cut position carrying no identity**. Across the live sets a
    /// six-character window is unique at every offset except the first two — nineteen of twenty-one
    /// windows identify a memory outright, and a prefix takes one of the two that do not.
    ///
    /// Ordering is by id, so results arrive oldest first like every other list here.
    ///
    /// # Errors
    ///
    /// See [`StoreError`].
    pub fn find_by_fragment(&self, fragment: &str) -> Result<Vec<Found>, StoreError> {
        let Some(needle) = MemoryId::normalise_fragment(fragment.trim()) else {
            return Ok(Vec::new());
        };

        let ids: Vec<String> = {
            // No index serves a leading wildcard, so this is a scan. That is affordable at a memory
            // set's size and is a further reason the path is opt-in rather than a fallback.
            let mut statement = self
                .connection
                .prepare("SELECT id FROM memories WHERE id LIKE ?1 ORDER BY id")?;
            let mapped = statement.query_map(params![format!("%{needle}%")], |row| {
                row.get::<_, String>(0)
            })?;
            mapped.collect::<Result<_, _>>()?
        };

        let mut found = Vec::new();
        for id in ids {
            if let Some(hit) = self.found_for(MemoryId::parse(&id)?)? {
                found.push(hit);
            }
        }
        Ok(found)
    }

    /// One memory as direct reference returns it: superseded memories are *returned*, not swapped.
    fn found_for(&self, id: MemoryId) -> Result<Option<Found>, StoreError> {
        let validity = self.resolve_supersession(id)?;
        let Some(memory) = self.get(id)? else {
            return Ok(None);
        };
        Ok(Some(Found {
            memory,
            scope: self.scope.clone(),
            incoming: self.incoming_edges(id)?,
            validity,
            disputed_by: Vec::new(),
            referenced_from: Vec::new(),
        }))
    }

    /// Apply validity resolution to one raw hit.
    ///
    /// Returns `None` when the memory is superseded by something this store cannot see: returning
    /// it as though current would be worse than omitting it, since it is known to be stale.
    fn resolved_hit(
        &self,
        id: MemoryId,
        similarity: f32,
        inherited: f32,
    ) -> Result<Option<Recalled>, StoreError> {
        let (id, matched, in_cycle, beyond, forgotten) = match self.resolve_supersession(id)? {
            Resolution::Current => (id, None, false, false, false),
            // Returned as itself and named. Treating a cycle as *current* was
            // `a-supersession-cycle-reads-as-current`: the one data error resolution detects, and no
            // caller could see it.
            Resolution::Cycle => (id, None, true, false, false),
            Resolution::Superseded {
                tips,
                unresolved,
                forgotten,
            } => match tips.first() {
                Some(tip) => (*tip, Some(id), false, false, false),
                None => (id, None, false, unresolved, forgotten),
            },
        };

        let incoming = self.incoming_edges(id)?;
        Ok(self.get(id)?.map(|memory| Recalled {
            memory,
            scope: self.scope.clone(),
            incoming,
            similarity,
            inherited,
            matched,
            in_cycle,
            superseded_beyond_this_set: beyond,
            replacement_forgotten: forgotten,
            // Filled by the across-functions, which are the only callers that hold the other
            // stores. A single store cannot see a claim against its own memory by construction.
            disputed_by: Vec::new(),
            referenced_from: Vec::new(),
        }))
    }

    /// Refuse a write whose `SUPERSEDES` edges would close a loop.
    fn check_no_cycle(&self, memory: &Memory) -> Result<(), StoreError> {
        for link in &memory.links {
            if link.label != LinkLabel::Supersedes {
                continue;
            }
            let LinkTarget::SameSet(target) = link.target else {
                continue;
            };
            // Does the target already supersede this memory, directly or through a chain? If so,
            // claiming to supersede it closes the loop.
            if self.supersedes_transitively(target, memory.id)? {
                return Err(StoreError::SupersessionCycle {
                    memory: memory.id,
                    target,
                });
            }
        }
        Ok(())
    }

    /// Whether `from` supersedes `goal`, directly or through a chain.
    ///
    /// Follows the edges *written on* each memory, since `SUPERSEDES` is stored on the memory doing
    /// the superseding — walking the reverse direction here would answer the opposite question.
    fn supersedes_transitively(&self, from: MemoryId, goal: MemoryId) -> Result<bool, StoreError> {
        let mut seen = HashSet::from([from]);
        let mut frontier = vec![from];

        while let Some(current) = frontier.pop() {
            for link in self.links_of(&current.to_string())? {
                if link.label != LinkLabel::Supersedes {
                    continue;
                }
                let LinkTarget::SameSet(target) = link.target else {
                    continue;
                };
                if target == goal {
                    return Ok(true);
                }
                if seen.insert(target) {
                    frontier.push(target);
                }
            }
        }
        Ok(false)
    }

    fn keywords_of(&self, id: &str) -> Result<Vec<Keyword>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT keyword FROM keywords WHERE memory_id = ?1 ORDER BY position")?;
        let rows = statement.query_map(params![id], |row| row.get::<_, String>(0))?;

        let mut keywords = Vec::new();
        for row in rows {
            keywords.push(Keyword::new(row?)?);
        }
        Ok(keywords)
    }

    fn links_of(&self, id: &str) -> Result<Vec<Link>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT label, target, target_set, target_keywords, target_set_id
             FROM links WHERE source = ?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;

        let mut links = Vec::new();
        for row in rows {
            let (label, target, target_set, target_keywords, target_set_id) = row?;
            let id = MemoryId::parse(&target)?;
            links.push(Link {
                label: parse_enum("link label", &label)?,
                target: match target_set {
                    None => LinkTarget::SameSet(id),
                    Some(set) => LinkTarget::CrossSet {
                        set: SetName::new(set)?,
                        id,
                        keywords: target_keywords
                            .map(|json| serde_json::from_str(&json).unwrap_or_default())
                            .unwrap_or_default(),
                        set_id: target_set_id.as_deref().map(SetId::parse).transpose()?,
                    },
                },
            });
        }
        Ok(links)
    }
}

/// A record's [`crate::memory_file::file_position`] as the database stores it.
///
/// **Sign-flipped**, because the hash is unsigned and SQLite's integers are signed: stored as-is,
/// every hash with its top bit set would read as negative and sort *before* the rest, so the
/// database's order would quietly differ from the file's. Flipping the top bit maps the unsigned
/// order onto the signed one exactly. The id that breaks a tie is the row's own `id`, ordered
/// second.
fn stored_position(id: MemoryId) -> i64 {
    let (hash, _) = crate::memory_file::file_position(id);
    i64::from_ne_bytes((hash ^ (1 << 63)).to_ne_bytes())
}

/// Write one memory's rows, canonicalising the spelling of its links.
///
/// **`scope` is what makes the store hold one spelling of a same-set link rather than two.** A
/// target naming the set it already lives in is written compactly, with `target_set` null, so every
/// lookup over the table can key on that null instead of each one remembering to compare set names
/// -- and the local store, which has no set name to compare against, needs no special case.
/// `a-same-set-link-spelled-with-its-set-name-is-inert` is what happens without this.
///
/// Normalising here rather than at the tool surface is deliberate: this is the one place every
/// write passes through, import included, and an import is exactly where the long spelling arrives.
/// It is a canonicalisation rather than a judgement, so it cannot fail, which is what makes it safe
/// on the shared path where a per-record refusal would cost the whole file.
///
/// The dropped `target_keywords` are not a loss: they exist to re-resolve a target in another set,
/// and a same-set reference has nothing to re-resolve against.
fn write_memory(
    connection: &Connection,
    memory: &Memory,
    scope: &Scope,
    holder: Option<SetId>,
) -> Result<(), StoreError> {
    let id = memory.id.to_string();

    connection.execute(
        "INSERT OR REPLACE INTO memories
             (id, content, why, context, confidence, origin,
              created_by, created_at, modified_by, modified_at, file_position)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id,
            memory.content.as_data(),
            memory.why.as_ref().map(MemoryText::as_data),
            memory.context.as_ref().map(MemoryText::as_data),
            memory.confidence.get(),
            serde_name(&memory.origin),
            memory.created.by.to_string(),
            memory.created.at.to_string(),
            memory.modified.by.to_string(),
            memory.modified.at.to_string(),
            stored_position(memory.id),
        ],
    )?;

    // Replaced wholesale rather than merged: a withdrawn edge or keyword must not survive a write.
    //
    // Deliberately redundant. `INSERT OR REPLACE` above deletes the old memory row, which already
    // cascades to these tables — verified by mutation: removing these statements changes nothing
    // until the foreign keys are also disabled. They are kept because the cascade depends on the
    // upsert being `INSERT OR REPLACE` specifically; rewriting it as `ON CONFLICT DO UPDATE`, which
    // is the more usual idiom and looks equivalent, updates the row in place and cascades nothing.
    // The explicit deletes make the intent survive that refactor.
    connection.execute("DELETE FROM keywords WHERE memory_id = ?1", params![id])?;
    connection.execute("DELETE FROM links WHERE source = ?1", params![id])?;

    // **An id is a memory or a tombstone, never both.** Not redundant, unlike the two above: nothing
    // cascades between these tables, and until 2026-08-26 nothing enforced the rule at all --
    // `an-id-can-hold-both-a-memory-and-a-tombstone`. `Store::forget` is this statement's mirror,
    // deleting the memory row as it writes the tombstone; without this one the pair is asymmetric
    // and a memory arriving for a tombstoned id leaves both rows behind.
    //
    // **The consequence was not confined to the store.** `stream_records` emits every row of both
    // tables, so an id in both is written to the file twice, and `documents/design/file-format.md`
    // refuses a duplicate id -- this project writing a file its own reader will not read, from a
    // sequence as ordinary as `git checkout` of an earlier commit followed by an import.
    //
    // Placed here rather than in the import path because every write goes through this function:
    // `insert`, `accept_from_file` and `replace_with` alike. The invariant is the table's, not any
    // one caller's, and `replace_with` having been fixed on its own is how this survived.
    connection.execute("DELETE FROM tombstones WHERE id = ?1", params![id])?;

    for (position, keyword) in memory.keywords.iter().enumerate() {
        connection.execute(
            "INSERT INTO keywords (memory_id, keyword, folded, position) VALUES (?1, ?2, ?3, ?4)",
            params![id, keyword.as_str(), keyword.folded(), position as i64],
        )?;
    }

    write_links(connection, &id, &memory.links, scope, holder)?;

    Ok(())
}

/// Write a source's outbound edges, canonicalising each target as it goes.
///
/// One function rather than a loop at each call site: `write_memory` writes a memory's links and
/// `Store::forget` writes the ancestry a tombstone keeps, and the two must agree exactly about how
/// a row is spelled -- particularly about a same-set target being stored in the compact form, which
/// `Store::edges` depends on to tell a foreign set from this one.
fn write_links(
    connection: &Connection,
    id: &str,
    links: &[Link],
    scope: &Scope,
    holder: Option<SetId>,
) -> Result<(), StoreError> {
    for link in links {
        let (target, set, keywords, set_id) = match &link.target {
            LinkTarget::SameSet(target) => (target.to_string(), None, None, None),
            // Foreign or not, asked once, so the two spellings of a same-set link converge here.
            LinkTarget::CrossSet {
                set,
                id: target,
                keywords,
                set_id,
            } if link.target.is_foreign_to(scope, holder) => (
                target.to_string(),
                Some(set.as_str().to_owned()),
                Some(serde_json::to_string(keywords).expect("keywords always serialise")),
                set_id.map(|id| id.to_string()),
            ),
            // A link that is *this* set -- by identity where both have one, by name otherwise --
            // canonicalises to a same-set one, and its identity goes with the name: a same-set link
            // needs neither, and keeping the id would leave the one spelling carrying two ways to say
            // the same thing.
            LinkTarget::CrossSet { id: target, .. } => (target.to_string(), None, None, None),
        };
        connection.execute(
            "INSERT OR REPLACE INTO links
                 (source, label, target, target_set, target_keywords, target_set_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, serde_name(&link.label), target, set, keywords, set_id],
        )?;
    }

    Ok(())
}

/// Enums are stored in the same form they take in the memory file, so the store and the file never
/// disagree about what a label is called.
fn parse_enum<T: serde::de::DeserializeOwned>(
    field: &'static str,
    value: &str,
) -> Result<T, StoreError> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|_| StoreError::Unrecognised {
        field,
        value: value.to_owned(),
    })
}

/// The value at `p` through a sorted copy of `values`, nearest-rank.
///
/// The same definition `tests/probes.rs` measures with, deliberately: a floor set by one rule and
/// swept under another is a number nobody can trace to its measurement.
fn percentile_of(values: &mut [f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let index = ((values.len() - 1) as f64 * p).round() as usize;
    values[index]
}

fn parse_timestamp(value: &str) -> Result<jiff::Timestamp, StoreError> {
    value.parse().map_err(|_| StoreError::Unrecognised {
        field: "timestamp",
        value: value.to_owned(),
    })
}

/// One memory competing in the ranking, after validity has been settled.
#[derive(Clone, Copy)]
struct Candidate {
    /// The best own-similarity any member of this memory's chain achieved.
    similarity: f32,
    /// What the walk carried to it.
    inherited: f32,
    /// The id that actually matched, when that was a memory this one replaced.
    matched: Option<MemoryId>,
    /// Whether its supersession history is a cycle.
    in_cycle: bool,
    /// Whether it yielded to a replacement in a set not held here.
    beyond: bool,
    /// Whether it yielded to a replacement this store forgot.
    forgotten: bool,
}

/// What the ranking should do with one scanned memory.
///
/// An enum rather than an `Option<MemoryId>` plus a flag, because the four answers are exclusive and
/// two of them are easy to conflate: a memory in a cycle and a memory whose replacement is missing
/// both *fail to resolve*, and they call for opposite treatment — one is returned and named, the
/// other is withheld.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ranked {
    /// Nothing supersedes it; it competes as itself.
    Itself,
    /// Superseded: this memory's score is evidence for the memory that replaced it, wherever that
    /// memory is -- a concession to another loaded set resolves into that set.
    Replaced(Located),
    /// In a supersession cycle. Competes as itself, and says so.
    InCycle,
    /// Superseded, with no replacement this search can produce: one held elsewhere, one forgotten
    /// here, or both. Competes as itself, and says which.
    ///
    /// It used to be dropped. See `superseded_beyond_this_set` on [`Recalled`] for why reporting it
    /// is the answer the design's own principle asks for.
    Unreplaced { beyond: bool, forgotten: bool },
}

/// A memory, and which of the searched stores it is in — **identity for the whole recall path**.
///
/// A bare [`MemoryId`] identifies a memory inside one store and nowhere else. Ids are unique within
/// a file, enforced when one is read, and nothing makes them unique *across* files: two loaded sets
/// may each hold one, in which case they are two memories with two contents and two scopes. Keying
/// the recall path on the id alone made them one identity before any edge was consulted, so a
/// memory was dropped with nothing said and the survivor was whichever store happened to be scanned
/// last — `two-loaded-sets-sharing-one-id-collapse-to-one-memory`.
///
/// **This is the model the rest of the system already used.** `hoard_lookup` collects a hit from
/// every store and returns both; keyword search does the same; a link is either same-set, resolved
/// inside one store, or carries the name of the set it points into. Semantic recall was the only
/// place that assumed an id names one memory everywhere, so this makes it agree rather than adding a
/// fourth rule.
///
/// The store is held as its index in the searched slice, which is what every caller here already
/// has, and is `Copy` where a `Scope` is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Located {
    /// Position in the slice `Neighbourhood::over` and `recall_across` were given.
    store: usize,
    id: MemoryId,
}

/// A memory's validity with its replacements *located*, which [`Resolution`] cannot express.
///
/// `Resolution` is a single store's answer and names its tips by id, which is right there: a
/// same-set tip is in the store that resolved it. Across stores a tip need not be — a concession to
/// a loaded set resolves to a memory in that set — so the recall path carries the store with each
/// tip rather than re-deriving it, which would be the same ambiguity in a different place.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Standing {
    Current,
    Superseded {
        tips: Vec<Located>,
        unresolved: bool,
        forgotten: bool,
    },
    Cycle,
}

impl Standing {
    /// The same answer with the stores dropped, for a caller reporting one memory's standing.
    fn flattened(self) -> Resolution {
        match self {
            Self::Current => Resolution::Current,
            Self::Cycle => Resolution::Cycle,
            Self::Superseded {
                tips,
                unresolved,
                forgotten,
            } => Resolution::Superseded {
                tips: tips.into_iter().map(|tip| tip.id).collect(),
                unresolved,
                forgotten,
            },
        }
    }
}

/// The graph the walk runs on, with every memory resolved to the one that is current.
///
/// **Validity is settled before the walk, not after it.** The design has always said so; recall did
/// not, and the cost was found on 2026-08-18 — see `validity-is-resolved-after-the-ranking` in
/// `TRACKING.md`. Two things follow from resolving here:
///
/// - **A superseded memory's edges belong to its tip.** The link was usually authored before the
///   correction was, so leaving it on the replaced memory makes the dead memory a conduit for
///   inherited score while the live one is short of routes — the only path between the two being the
///   `SUPERSEDES` edge the walk refuses to cross.
/// - **A supersession chain is one memory to the walk.** Its members resemble each other by
///   construction, so seeds chosen from unresolved ids would hand most of a five-seed budget to
///   memories that collapse to one, and the walk would explore far less than its parameters claim.
///
/// # The population is where the scope rule lives
///
/// An edge is walked when both endpoints are memories in the stores being searched — so an unscoped
/// recall, which loads every set, crosses a cross-set link and carries on, while a scoped recall
/// never has the other set's memories in its population and stops at the boundary. One rule, both
/// behaviours. The reason a scoped recall must stop is arithmetic rather than contractual: the
/// centroid was computed over the searched sets alone, so a memory from outside them has no
/// own-similarity on that scale.
///
/// Membership is by memory rather than by vector, so an unembedded memory still bridges the two
/// memories it links. It cannot be ranked or returned — it never appears in the scan — but the
/// relation either side of it is real.
struct Neighbourhood {
    adjacency: Adjacency<Located>,
    /// What every memory carrying a supersession edge resolves to. Only those appear, which is a
    /// small share of any set; anything absent is current.
    ///
    /// The whole [`Standing`] rather than the tip alone, because the ranking needs to tell
    /// *replaced* from *in a cycle* from *replaced by something this machine cannot see*, and those
    /// are three different answers with three different results.
    validity: HashMap<Located, Standing>,
    population: HashSet<Located>,
}

/// A memory's validity as the **whole call** can see it, rather than as its own store can.
///
/// **A concession naming a set that is being searched resolves like any other.**
/// [`Store::resolve_supersession`] cannot know that: it maps every cross-set target to *not
/// available here*, which is true of the store asking and false of the call it is part of. What came
/// back was a memory flagged `superseded_beyond_this_set` while its replacement sat in the same
/// result list, telling the reader to go and load a set they already hold --
/// `superseded-beyond-this-set-is-reported-when-the-set-is-loaded`.
///
/// **This stays on the right side of the rule it looks like it crosses.** No edge may change the
/// standing of a memory in another set; an authored `SUPERSEDED_BY` is the memory's own claim about
/// itself, so the standing that changes here is the conceding memory's own. Each store is asked only
/// about ids it holds, which is the restriction `Neighbourhood::over` enforces with `holds_record`
/// for the same reason.
///
/// Shared by every path that turns a resolution into a report, so the semantic path, the keyword
/// path and lookup cannot answer this differently -- which is the defect shape
/// `lookup-and-recall-disagree-on-shape` records.
///
/// **Returns the flattened answer**, naming tips by id alone, which is what a caller reporting one
/// memory's standing needs. The recall path uses [`standing_within`] instead and keeps the store
/// with each tip, because it has to *rank* the replacement and cannot look up which set it came
/// from without reintroducing the ambiguity this all exists to remove.
///
/// # Errors
///
/// See [`StoreError`].
pub fn resolve_within(
    stores: &[&Store],
    home: usize,
    id: MemoryId,
) -> Result<Resolution, StoreError> {
    Ok(standing_within(stores, home, id)?.flattened())
}

/// The same answer with every replacement located. See [`resolve_within`] for the reasoning; this is
/// the one implementation and that is the flattened view of it.
fn standing_within(stores: &[&Store], home: usize, id: MemoryId) -> Result<Standing, StoreError> {
    let Some(store) = stores.get(home) else {
        return Ok(Standing::Current);
    };
    let (local, elsewhere, absent, forgotten) = match store.walk_supersession(id)? {
        Walked::Current => return Ok(Standing::Current),
        Walked::Cycle => return Ok(Standing::Cycle),
        Walked::Superseded {
            tips,
            elsewhere,
            absent,
            forgotten,
        } => (tips, elsewhere, absent, forgotten),
    };

    let mut tips: Vec<Located> = local
        .into_iter()
        .map(|id| Located { store: home, id })
        .collect();
    if !elsewhere {
        return Ok(Standing::Superseded {
            tips,
            unresolved: absent,
            forgotten,
        });
    }

    // **An absent id stays out of reach whatever the concessions resolve to.** Only a concession
    // to another set can be re-resolved here; deriving `unresolved` from the concessions alone
    // dropped every other reason the chain stopped, and a memory whose replacement was forgotten
    // looked up as current.
    let mut out_of_reach = absent;
    for (set, set_id, target) in store.conceded_to_elsewhere(id)? {
        match tips_held_elsewhere(stores, &set, set_id, target)? {
            Some(found) => tips.extend(found),
            None => out_of_reach = true,
        }
    }
    tips.sort();
    tips.dedup();
    Ok(Standing::Superseded {
        tips,
        unresolved: out_of_reach,
        forgotten,
    })
}

/// What a concession to another set resolves to, when that set is among those being searched.
///
/// `None` when the reader is genuinely no better off: the named set is not loaded here, it does not
/// hold the id, or what it holds is itself out of reach -- superseded beyond its own set, or in a
/// cycle. In each of those the honest answer is the flag, which is what the caller does with `None`.
///
/// The chain is followed *inside* the named set rather than stopped at the target, so a replacement
/// that was itself corrected resolves to the correction, exactly as a same-set chain does.
/// The store holding a cross-set link's target, chosen by **identity** where the link states one.
///
/// **An identity, once stated, is not overridden by a name.** A set's local name can change, and two
/// sets can share one, which is exactly why a link records the target's `set_id` when the authoring
/// machine knew it. Matching on the name first defeats both purposes: a stale name resolves to
/// nothing while the identity sits unused in the same row, and a shared name resolves to the *wrong
/// set* -- which is worse than not resolving, because it attaches one set's concession to another
/// set's memory.
///
/// **No fallback from a stated identity to the name**, deliberately. If the identity names no loaded
/// set then the target set is not here, and a loaded set that merely shares the name is a different
/// set: taking it would be the ambiguity `set_id` exists to remove. Where the link carries no
/// identity -- written before the field existed, or by a machine that did not hold the target set --
/// the name is all there is and is used exactly as before.
///
/// **An identity chooses only among the stores a name could have chosen**, since 2026-09-28
/// (`a-link-can-reach-the-local-set-by-its-identity`). `local` is not a set name, so no link can
/// name it; but it has held an identity since 2026-08-31, and matching identities against every
/// store let a crafted file reach it anyway -- a shared memory's `superseded_by` resolved into a
/// personal memory and recall substituted it, and a shared edge made one a neighbour in the walk.
/// Keyed on whether the store is addressable at all rather than on `Scope::Local` by name, so the
/// two routes to a store are the same width by construction: a store no name reaches is one no
/// identity reaches.
fn holder_of(
    stores: &[&Store],
    set: &SetName,
    set_id: Option<SetId>,
) -> Result<Option<usize>, StoreError> {
    if let Some(wanted) = set_id {
        for (index, store) in stores.iter().enumerate() {
            if store.scope.exports_to().is_some() && store.set_id()? == Some(wanted) {
                return Ok(Some(index));
            }
        }
        return Ok(None);
    }
    Ok(stores
        .iter()
        .position(|store| store.scope.exports_to() == Some(set)))
}

fn tips_held_elsewhere(
    stores: &[&Store],
    set: &SetName,
    set_id: Option<SetId>,
    target: MemoryId,
) -> Result<Option<Vec<Located>>, StoreError> {
    let Some(holder) = holder_of(stores, set, set_id)? else {
        return Ok(None);
    };
    let store = stores[holder];
    if !store.holds_memory(target)? {
        return Ok(None);
    }
    let located = |id| Located { store: holder, id };
    match store.resolve_supersession(target)? {
        Resolution::Current => Ok(Some(vec![located(target)])),
        Resolution::Superseded { tips, .. } if !tips.is_empty() => {
            Ok(Some(tips.into_iter().map(located).collect()))
        }
        Resolution::Superseded { .. } | Resolution::Cycle => Ok(None),
    }
}

impl Neighbourhood {
    /// Read every searched store's memories and links, and resolve the graph onto current memories.
    ///
    /// Chains are resolved through [`Store::resolve_supersession`] rather than by following the
    /// edges collected here, so there is one implementation of *what replaced this* and the walk
    /// cannot disagree with the substitution the results report. It is asked only of memories some
    /// edge says were replaced, so the cost is a handful of chain walks rather than one per memory.
    fn over(stores: &[&Store]) -> Result<Self, StoreError> {
        let mut population: HashSet<Located> = HashSet::new();
        for (index, store) in stores.iter().enumerate() {
            for id in store.memory_ids()? {
                population.insert(Located { store: index, id });
            }
        }

        let mut validity: HashMap<Located, Standing> = HashMap::new();
        let mut walkable: Vec<(Located, Located)> = Vec::new();
        for (index, store) in stores.iter().enumerate() {
            let edges = store.edges()?;
            for (source, target, set, set_id) in edges.walkable {
                // **An edge reaches the store it named, and no other.** A same-set target is in the
                // store that produced the edge; a named one is in whichever store holds that set,
                // or nowhere this search can see. Looking the target id up across every store
                // instead would attach the edge to a memory it never pointed at, the moment two
                // sets share an id.
                let Some(target_store) = (match &set {
                    None => Some(index),
                    // By identity where the edge states one, for the reason `holder_of` gives: a
                    // stale name reaches nothing and a shared name reaches the wrong set, which
                    // would make a neighbour of a memory nothing ever linked to.
                    Some(name) => holder_of(stores, name, set_id)?,
                }) else {
                    continue;
                };
                walkable.push((
                    Located {
                        store: index,
                        id: source,
                    },
                    Located {
                        store: target_store,
                        id: target,
                    },
                ));
            }
            for replaced in edges.replaced {
                // **A store may only speak about memories it holds.** This is the structural half
                // of `a-set-can-suppress-another-sets-memory`, and the half the first repair missed:
                // filtering on `target_set` keyed on how the link was *spelled*, so an attacker who
                // wrote a bare id -- the easier thing to hand-write -- bypassed it entirely and
                // suppressed a memory in someone else's set.
                //
                // Locating the answer is what makes this hold even under a shared id. The check
                // says this store may rule on the id; the key says the ruling applies to *its* copy
                // and not to another store's, which is what a single validity map keyed by id alone
                // could not express.
                //
                // **The key now subsumes this check, and it is kept anyway.** With the ruling filed
                // under this store's own key, a store resolving an id it does not hold produces an
                // entry nothing in the population answers to, so removing the line changes no
                // observable behaviour -- its recorded mutation says `survives` and carries that
                // reasoning. It stays because it states the rule where the rule belongs, and
                // because it is what still holds if the key is ever loosened.
                if !store.holds_record(replaced)? {
                    continue;
                }
                let standing = standing_within(stores, index, replaced)?;
                if standing != Standing::Current {
                    validity.insert(
                        Located {
                            store: index,
                            id: replaced,
                        },
                        standing,
                    );
                }
            }
        }

        let mut neighbourhood = Self {
            adjacency: Adjacency::new(),
            validity,
            population,
        };
        for (source, target) in walkable {
            // **An out-link stays with the record that wrote it.** A superseded memory's own links
            // are its claims, and a replacement that did not restate one has dropped it -- the only
            // way to drop a link once the memory holding it has been exported. So an edge whose
            // source was replaced is not carried onto the tip. The *target* still moves: a link to
            // a superseded memory was written about the claim, and follows it.
            // `out-links-stay-with-the-record-that-wrote-them`, step 3. A superseded memory with no
            // tip this search can reach stands as itself, and keeps its links.
            if neighbourhood.tip_of(source) != source {
                continue;
            }
            let target = neighbourhood.tip_of(target);
            if neighbourhood.holds(source) && neighbourhood.holds(target) {
                // A self-edge is dropped by `add_edge`, which is what an edge between two links of
                // one supersession chain collapses to.
                neighbourhood.adjacency.add_edge(source, target);
            }
        }
        Ok(neighbourhood)
    }

    /// The memory a candidate should stand at: itself, or the first tip that replaced it.
    fn tip_of(&self, at: Located) -> Located {
        match self.validity.get(&at) {
            Some(Standing::Superseded { tips, .. }) => tips.first().copied().unwrap_or(at),
            Some(Standing::Current | Standing::Cycle) | None => at,
        }
    }

    /// Which memory a candidate should be ranked *as*, and whether it should be ranked at all.
    ///
    /// **This is validity resolved before the ranking rather than after it**, which is what the
    /// design always said happened and what `validity-is-resolved-after-the-ranking` records not
    /// happening. The ranking then works in current memories throughout: a chain contributes one
    /// candidate carrying the best evidence any of its members matched, and any per-record term a
    /// future ranking reads -- native confidence first among them -- reads the record that is
    /// actually being returned.
    fn rank_as(&self, at: Located) -> Ranked {
        match self.validity.get(&at) {
            None | Some(Standing::Current) => Ranked::Itself,
            Some(Standing::Cycle) => Ranked::InCycle,
            Some(Standing::Superseded {
                tips,
                unresolved,
                forgotten,
            }) => match tips.first() {
                Some(tip) => Ranked::Replaced(*tip),
                // Known to be superseded, with no replacement this search can produce. Returned so
                // the session learns it exists, flagged so it is not read as current.
                None => Ranked::Unreplaced {
                    beyond: *unresolved,
                    forgotten: *forgotten,
                },
            },
        }
    }

    /// Whether this memory is in the searched population.
    ///
    /// False for a memory in an unsearched set, and for one whose vector is still cached after it was
    /// forgotten — which is why this is asked of seeds as well as of edges.
    fn holds(&self, at: Located) -> bool {
        self.population.contains(&at)
    }
}

/// Every memory in another loaded set that points at this one.
///
/// **Reported, never resolved.** These rows are deliberately invisible to [`Store::incoming`], which
/// `resolve_supersession` walks — an id held in two sets would otherwise let one set's edge change
/// the other's standing. This reads them into a report, the same distinction [`claims_against_hit`]
/// turns on.
///
/// **Personal sets are included here and excluded there**, which is not an inconsistency. A claim
/// naming `local` would be reported to readers for whom `local` is a different set; this is derived
/// from the stores one machine holds, travels in no export, and exists precisely so a personal
/// memory pointing into a shared set is visible from the shared side.
///
/// # Errors
///
/// See [`StoreError`].
pub fn referenced_from_other_sets(
    stores: &[&Store],
    scope: &Scope,
    id: MemoryId,
) -> Result<Vec<ReferencedFrom>, StoreError> {
    let Some(target_set) = scope.exports_to() else {
        // A cross-set link names a `SetName`, and `local` is not one — so nothing can point *into*
        // a personal set from outside it, and there is no such row to find. Same-set edges are
        // already reported by `incoming`.
        return Ok(Vec::new());
    };
    // This set's own identity, so a row naming it by a name it no longer uses still matches, and one
    // naming a different set that shares the name does not.
    let target_set_id = match stores.iter().find(|store| &store.scope == scope) {
        Some(store) => store.set_id()?,
        None => None,
    };

    let mut found = Vec::new();
    for store in stores {
        if &store.scope == scope {
            continue;
        }
        for edge in store.edges_into_another_set(id, target_set, target_set_id)? {
            found.push(ReferencedFrom {
                scope: store.scope.clone(),
                source: edge.source,
                label: edge.label,
            });
        }
    }
    Ok(found)
}

/// Gather every other loaded set's standing-changing claims against one memory.
///
/// **Asked of the other stores, never of the memory's own.** A claim is defined by crossing a set
/// boundary, and a row inside the memory's own store that spells its own set name in `target_set` is
/// a different condition -- a same-set link written the long way -- which resolution currently
/// ignores and which is filed separately rather than reported here as a dispute.
///
/// **A memory in the local set can never be disputed, and that is structural rather than lucky.**
/// `LinkTarget::CrossSet` carries a `SetName`, and `Scope::Local` is not a named set, so no
/// `target_set` value addresses it -- and `holder_of` matches a `set_id` only among named sets, so
/// no identity does either. The early return says so out loud instead of letting the function
/// return empty for a reason no reader could see.
///
/// # Errors
///
/// See [`StoreError`].
pub fn claims_against_hit(
    stores: &[&Store],
    scope: &Scope,
    id: MemoryId,
) -> Result<Vec<DisputedBy>, StoreError> {
    let Some(target_set) = scope.exports_to() else {
        return Ok(Vec::new());
    };

    let mut claims = Vec::new();
    for store in stores {
        if &store.scope == scope {
            continue;
        }
        let Some(claimant) = store.scope.exports_to() else {
            // A local set can hold such a row, but reporting it would name `local` as the claimant
            // -- a set name that means a different set on every machine. Nothing useful travels.
            continue;
        };
        for (source, label) in store.claims_against(id, target_set)? {
            claims.push(DisputedBy {
                set: claimant.clone(),
                source,
                label,
            });
        }
    }
    Ok(claims)
}

/// How many candidate ids a keyword cursor reads from its store at a time.
///
/// **Ids and counts only, not memories**: a full record is loaded one at a time, as the merge pulls
/// it, so this bounds what a search holds rather than what it loads. Large enough that one batch
/// answers an ordinary recall, small enough that a keyword carried by thousands of memories is never
/// read whole to return ten of them.
const KEYWORD_BATCH: usize = 64;

/// One store's keyword matches, in rank order -- most matches first, then id -- read in bounded
/// batches and loaded only as they are taken.
///
/// `keyword-recall-ignores-already-known`, extended at the human partner's request into the shape
/// retrieval should have had: fetch a bounded batch, filter it, and fetch again only if more are
/// needed. The query it replaces had no `LIMIT`, so every matching id was read to answer any recall.
/// Each batch continues from the last `(matches, id)` seen rather than using an offset, so a batch
/// never re-reads the rows before it.
struct KeywordCursor<'s> {
    store: &'s Store,
    folded: Vec<String>,
    /// The last `(matches, id)` read, which the next batch continues after.
    after: Option<(i64, String)>,
    pending: std::collections::VecDeque<(String, i64)>,
    exhausted: bool,
}

impl<'s> KeywordCursor<'s> {
    fn new(store: &'s Store, keywords: &[Keyword]) -> Self {
        Self {
            store,
            folded: keywords.iter().map(Keyword::folded).collect(),
            after: None,
            pending: std::collections::VecDeque::new(),
            exhausted: keywords.is_empty(),
        }
    }

    fn fill(&mut self) -> Result<(), StoreError> {
        let placeholders = vec!["?"; self.folded.len()].join(", ");
        // `folded`, never `keyword`: matching ignores case, and the fold is applied to the query by
        // the same `Keyword::folded` that produced the stored key.
        let continuing = if self.after.is_some() {
            "HAVING COUNT(*) < ? OR (COUNT(*) = ? AND memory_id > ?)"
        } else {
            ""
        };
        let sql = format!(
            "SELECT memory_id, COUNT(*) AS matches FROM keywords
             WHERE folded IN ({placeholders})
             GROUP BY memory_id {continuing}
             ORDER BY matches DESC, memory_id ASC LIMIT {KEYWORD_BATCH}"
        );
        let mut values: Vec<rusqlite::types::Value> = self
            .folded
            .iter()
            .map(|folded| rusqlite::types::Value::Text(folded.clone()))
            .collect();
        if let Some((matches, id)) = &self.after {
            values.push(rusqlite::types::Value::Integer(*matches));
            values.push(rusqlite::types::Value::Integer(*matches));
            values.push(rusqlite::types::Value::Text(id.clone()));
        }
        let rows: Vec<(String, i64)> = {
            let mut statement = self.store.connection.prepare(&sql)?;
            let mapped = statement.query_map(rusqlite::params_from_iter(values), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            mapped.collect::<Result<_, _>>()?
        };
        self.exhausted = rows.len() < KEYWORD_BATCH;
        if let Some((id, matches)) = rows.last() {
            self.after = Some((*matches, id.clone()));
        }
        self.pending.extend(rows);
        Ok(())
    }

    /// The next hit in rank order, or `None` when the store has no more.
    ///
    /// **A held id is skipped before its memory is loaded**, which is what keeps paging cheap: the
    /// ids a caller already holds are the top of the ranking, so loading them only to discard them
    /// would cost the most exactly where a caller has paged the furthest.
    fn next_hit(&mut self, skip: &[MemoryId]) -> Result<Option<KeywordHit>, StoreError> {
        loop {
            let Some((id, matches)) = self.pending.pop_front() else {
                if self.exhausted {
                    return Ok(None);
                }
                self.fill()?;
                if self.pending.is_empty() {
                    return Ok(None);
                }
                continue;
            };
            let id = MemoryId::parse(&id)?;
            if skip.contains(&id) {
                continue;
            }
            // No similarity and no inherited score, because **keyword search does not traverse**:
            // an exact match returns a set with no scores, so there is no seed score for the walk to
            // inherit and decay -- the mechanism has nothing to operate on. Nothing is lost, since a
            // keyword hit carries its links like any other memory and a caller can follow one.
            let Some(recalled) = self.store.resolved_hit(id, 0.0, 0.0)? else {
                continue;
            };
            // **A keyword hit is never reached through supersession.** `resolved_hit` sets
            // `matched` exactly when it swapped a superseded memory for its replacement, and on
            // this path that swap is always redundant: if the replacement carries the term, its own
            // row matches it directly, so nothing is lost by dropping the superseded one. What is
            // gained is that a term the replacement *dropped* stops matching, which is the only way
            // an author can retire one — `a-keyword-cannot-be-retired-once-a-memory-has-been-
            // exported` in `TRACKING.md`.
            //
            // **Deliberately unlike the semantic path**, which still substitutes. The tip rule is
            // justified on a superseded memory's *substance* being misleading; a keyword is an
            // author's addressing decision rather than a claim, so that argument does not reach it.
            // Direct id lookup is exempt on the same ground. Explicit addressing is honoured as
            // written; inferred matching resolves to the tip.
            //
            // A memory superseded beyond this set has no tip to swap in, so `matched` is `None` and
            // it is returned flagged, as before.
            if recalled.matched.is_some() {
                continue;
            }
            return Ok(Some(KeywordHit {
                memory: recalled.memory,
                scope: recalled.scope,
                incoming: recalled.incoming,
                matched_keywords: usize::try_from(matches).unwrap_or(0),
                matched: recalled.matched,
                in_cycle: recalled.in_cycle,
                superseded_beyond_this_set: recalled.superseded_beyond_this_set,
                replacement_forgotten: recalled.replacement_forgotten,
                disputed_by: Vec::new(),
                referenced_from: Vec::new(),
            }));
        }
    }
}

/// Keyword search across several stores, ranked and cut as **one** list.
///
/// `keyword-recall-multiplies-the-limit-by-the-number-of-sets`. The caller loop this replaces asked
/// each store for `limit` hits and concatenated the results, which got both halves of the contract
/// wrong: the response held up to `limit × loaded sets`, and it was in store iteration order rather
/// than ranked, so a two-keyword match in the second set sorted below a one-keyword match in the
/// first. `hoard_recall` promises results "ordered best first", which was true of the semantic path
/// and false of this one. Both were invisible with a single loaded store, which is every test and
/// was every real configuration until a shared set existed alongside `local`.
///
/// No `prepare` pass, unlike [`recall_across`]: this path reads rows rather than the vector cache,
/// and SQLite already shows a fresh query another process's committed writes.
///
/// # Errors
///
/// See [`StoreError`].
pub fn search_keywords_across(
    stores: &[&Store],
    keywords: &[Keyword],
    limit: usize,
) -> Result<Vec<KeywordHit>, StoreError> {
    search_keywords_across_holding(stores, keywords, limit, &[])
}

/// [`search_keywords_across`], leaving out the memories a caller says it already holds.
///
/// **A merge that stops, not a union that is cut**, since 2026-09-29. Each store is a
/// [`KeywordCursor`] yielding its matches in rank order, a batch at a time; this takes the best head
/// among them one hit at a time, and stops at `limit`. Held ids, substitutions and duplicates are
/// dealt with as each hit is taken, so leaving something out simply means taking the next -- a page
/// is full whenever enough matches exist, with no over-fetch to size -- and claims are computed only
/// for what is returned. What it replaced read every matching id from every store, loaded up to
/// `limit` full records from each, and computed claims for all of them before cutting.
///
/// **Ranked by match count, then by the id of the memory that matched**, which is the id a cursor is
/// ordered by. After a cross-set substitution that is the conceding memory's id, not the
/// replacement's -- the count already stays with the memory that matched, as the similarity does on
/// the semantic path, and keying the tie-break on it is what lets the merge stream: ordering by the
/// replacement's id would make one hit's place unknowable until a whole tier had been read, and a
/// one-keyword query is a single tier. Equal counts are the common case, so the tie-break is a rule
/// rather than an accident of which store came first; ids are UUIDv7, so ascending is oldest first.
///
/// **Held ids are judged twice**: by each cursor before loading, which is the cheap case and nearly
/// every one, and here on the memory actually returned, for a hit substituted by a replacement the
/// caller holds.
///
/// # Errors
///
/// See [`StoreError`].
pub fn search_keywords_across_holding(
    stores: &[&Store],
    keywords: &[Keyword],
    limit: usize,
    already_known: &[MemoryId],
) -> Result<Vec<KeywordHit>, StoreError> {
    let mut cursors: Vec<KeywordCursor<'_>> = stores
        .iter()
        .map(|store| KeywordCursor::new(store, keywords))
        .collect();
    let mut heads: Vec<Option<KeywordHit>> = Vec::with_capacity(cursors.len());
    for cursor in &mut cursors {
        heads.push(if limit == 0 {
            None
        } else {
            cursor.next_hit(already_known)?
        });
    }

    // Substitution can land two stores' hits on one memory -- the conceding memory's, resolved to
    // the replacement, beside the replacement found on its own account. Best evidence wins, which is
    // the same rule the semantic path applies when a chain collapses into one candidate, and taking
    // hits best first makes it *first taken wins*.
    //
    // **Identity here is the memory *and* its set, and that is not pedantry.** Two loaded sets may
    // hold one id, in which case they are two memories with two contents and two scopes, and this
    // path is the one that still returns both -- `two-loaded-sets-sharing-one-id-collapse-to-one-
    // memory` records the semantic path collapsing them and names the keyword path as the contrast.
    // Keying this on the id alone reintroduced that collapse here, caught by
    // `a_colliding_id_leaves_the_keyword_path_alone`.
    let mut taken: HashSet<(String, MemoryId)> = HashSet::new();
    let mut hits: Vec<KeywordHit> = Vec::new();
    while hits.len() < limit {
        let Some(best) = heads
            .iter()
            .enumerate()
            .filter_map(|(index, head)| head.as_ref().map(|hit| (index, hit)))
            .min_by(|(_, left), (_, right)| {
                right
                    .matched_keywords
                    .cmp(&left.matched_keywords)
                    .then_with(|| left.memory.id.cmp(&right.memory.id))
            })
            .map(|(index, _)| index)
        else {
            break;
        };
        let Some(mut hit) = heads[best].take() else {
            break;
        };
        heads[best] = cursors[best].next_hit(already_known)?;

        substitute_beyond_its_set(stores, &mut hit)?;
        if already_known.contains(&hit.memory.id) {
            continue;
        }
        if !taken.insert((scope_key(&hit.scope), hit.memory.id)) {
            continue;
        }
        hits.push(hit);
    }

    // After any substitution, so the claims reported are the ones against the memory actually
    // returned -- and a claim comes from a store other than the one that found the memory, which is
    // why it is asked here rather than by the cursor. Only for what is returned.
    for hit in &mut hits {
        hit.disputed_by = claims_against_hit(stores, &hit.scope, hit.memory.id)?;
        hit.referenced_from = referenced_from_other_sets(stores, &hit.scope, hit.memory.id)?;
    }
    Ok(hits)
}

/// Replace a hit whose memory conceded to another set with that set's replacement, where the set is
/// among those searched.
fn substitute_beyond_its_set(stores: &[&Store], hit: &mut KeywordHit) -> Result<(), StoreError> {
    // A concession the semantic path honours must be honoured here too. Each [`KeywordCursor`]
    // resolved its hits against its own store, which cannot see whether the set a concession names
    // is loaded; `resolve_within` can, and this is the first point that holds every store. Only a
    // hit the per-store pass gave up on is reconsidered, so the ordinary hit costs nothing.
    if !hit.superseded_beyond_this_set {
        return Ok(());
    }
    let Some(home) = stores.iter().position(|store| store.scope == hit.scope) else {
        return Ok(());
    };
    let Resolution::Superseded {
        tips,
        unresolved,
        forgotten,
    } = resolve_within(stores, home, hit.memory.id)?
    else {
        return Ok(());
    };
    let Some(tip) = tips.first().copied() else {
        return Ok(());
    };
    let Some(holder) = stores
        .iter()
        .find(|store| store.holds_memory(tip).unwrap_or(false))
    else {
        return Ok(());
    };
    let Some(memory) = holder.get(tip)? else {
        return Ok(());
    };
    // The count stays with the memory that matched, as the similarity does on the semantic path:
    // it records how well the query found this neighbourhood, not what the replacement happens
    // to carry.
    hit.matched = Some(hit.memory.id);
    hit.memory = memory;
    hit.scope = holder.scope.clone();
    hit.incoming = holder.incoming_edges(tip)?;
    hit.superseded_beyond_this_set = unresolved;
    hit.replacement_forgotten = forgotten;
    Ok(())
}

/// Which centroid, if any, the scan removes from both sides.
///
/// The shipping choice is [`Centring::Combined`]. Per-store centroid removal puts each store's
/// scores on its own scale, which makes merging them by raw score unsound -- the property
/// `tests/cross_store.rs` pins -- so a multi-set recall centres every store on one centroid built
/// over all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Centring {
    /// One centroid over every store being searched. What ships.
    #[default]
    Combined,
    /// Each store centred on its own centroid.
    PerStore,
    /// No centroid removed: raw cosine.
    None,
}

/// Which of the scorer's decisions are in force.
///
/// **`Default` is exactly what ships**, and
/// `the_instrumented_scorer_at_its_defaults_matches_the_shipping_one` is the guard on that. This
/// exists so each decision can be turned off and its contribution to a real ranking measured, after
/// a set of measurements was found to have been taken with raw cosine against a scorer that centres
/// -- the numbers were not comparable and nothing said so.
///
/// Not reachable from the MCP surface, and deliberately: these are not choices a caller makes, they
/// are the scorer's own settled decisions. Instrumentation is the whole reason they are nameable.
#[derive(Debug, Clone, Copy)]
pub struct Stages {
    /// Which centroid the scan removes.
    pub centring: Centring,
    /// The walk's shape. [`Walk::none`] is the content-only baseline.
    pub walk: Walk,
    /// What share of a reached memory's score comes from the walk. See [`traversal::INHERITED_WEIGHT`].
    pub inherited_weight: f64,
    /// Whether the result set is truncated at the largest gap. **Off in what ships**, since
    /// 2026-10-02: on real queries the largest gap marked no boundary -- it appeared about as often
    /// in rankings for nonsense -- so it chose a count among noise, and the count followed the limit.
    /// Kept so that a measurement can still ask what it would have done.
    pub gap_cut: bool,
    /// Whether memories that have been superseded are scanned at all.
    ///
    /// `true` is what ships: a chain folds into one candidate carrying **the best evidence any
    /// member matched**, so a query matching an old claim's wording returns the correction. Setting
    /// this false scores a chain on its current text only, which is how the superseded bodies'
    /// contribution to a ranking is measured -- they are 10% of returned hits and lift their tips by
    /// a median 0.118, so the contribution is not obviously small.
    pub include_superseded: bool,
    /// Which floor is applied, below which a hit is not returned at all.
    pub floor: Floor,
}

/// Where a recall's floor comes from.
///
/// Every other stage of the scorer is *relative* — a gap, a rank, a ratio — so nothing in it can say
/// *nothing here answers this*, and an unanswerable query comes back with a full window of
/// confident-looking results. This is the one absolute question, and it is asked of **raw content
/// similarity** rather than of the blended score: inherited score is derived from some other
/// memory's relevance, so letting it carry a memory past a relevance floor would defeat the floor's
/// purpose, and the centred score has had the very direction removed that marks a query as out of
/// domain.
///
/// **A constant is wrong on its face** — the same probes put the floor at 0.089 on an eight-memory
/// set and 0.286 on a 429-memory one — which is why the shipping variant reads a level each store
/// calibrated for itself, and why the fixed one exists only for measurement.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Floor {
    /// Each store's own calibrated level, or none where it has not calibrated one. What ships.
    #[default]
    Calibrated,
    /// No floor anywhere. The baseline every measurement of what a floor contributes is taken
    /// against, and it is a distinct answer from *calibrated, and this store has none yet*.
    None,
    /// One level for every store searched.
    ///
    /// Instrumentation only. Stores centred together are comparable, but a level calibrated on one
    /// store is not a level for another, so this is a measurement instrument rather than a setting.
    Fixed(f64),
}

impl Default for Stages {
    fn default() -> Self {
        Self {
            centring: Centring::Combined,
            walk: Walk::measured(),
            inherited_weight: traversal::INHERITED_WEIGHT,
            gap_cut: false,
            include_superseded: true,
            floor: Floor::Calibrated,
        }
    }
}

/// Search several stores as one corpus.
///
/// Every store is centred on the mean of their union, which is what makes their scores comparable:
/// searching project ∪ local is one corpus, and scoring each store against its own mean would put
/// the results on two scales that cannot be ordered against each other.
///
/// # The order is resolve, then rank, then cut
///
/// Supersession is a **validity** relation, so it is settled before relevance is judged: a
/// superseded memory has already been replaced by its tip when the cut looks at the ranking. That is
/// what dissolves the collapsed-hit question the design left open — a memory and its tip never
/// occupy two slots, and a gap cut has no quota that a collapse could leave short.
///
/// # Errors
///
/// See [`StoreError`].
pub fn recall_across(
    stores: &mut [&mut Store],
    question: &Question<'_>,
) -> Result<Recall, StoreError> {
    recall_across_with(stores, question, Stages::default())
}

/// [`recall_across`] with each of the scorer's decisions selectable. See [`Stages`].
///
/// # Errors
///
/// See [`StoreError`].
pub fn recall_across_with(
    stores: &mut [&mut Store],
    question: &Question<'_>,
    stages: Stages,
) -> Result<Recall, StoreError> {
    // Every store's cache is brought up to date before any of them is scanned, for the same reason
    // `Store::recall` does it: a store shared with another process can be holding vectors from
    // before that process wrote. Done in its own pass so the rest of this function can work from
    // shared references, which is what lets one store's hits be resolved while another is read.
    for store in stores.iter_mut() {
        store.prepare()?;
    }
    let stores: Vec<&Store> = stores.iter().map(|store| &**store).collect();
    let stores = stores.as_slice();

    // Every store must be searchable for the answer to be complete. One without a model is not a
    // reason to silently answer from the rest.
    //
    // Asked of the model state since 2026-08-09, so a store holding another model's vectors counts
    // as unsearchable — and is then **left out of the scan entirely**, rather than merely making
    // the answer incomplete. The two cases differ in kind: a store with no model contributes no
    // vectors and the rest still answer honestly, which is the partial-result decision below. A
    // store with the wrong model would contribute vectors that are comparable in the arithmetic
    // sense and meaningless in every other, polluting both the ranking and the union centroid the
    // others are scored against — so a partial answer there is not partial, it is wrong.
    let mut searchable = Vec::with_capacity(stores.len());
    let mut skipped = Vec::new();
    for store in stores {
        if store.can_search_by_meaning()? {
            searchable.push(*store);
        } else {
            // Named here rather than counted, because the remedy differs by reason and the caller
            // should not need a second call to learn which set to act on.
            skipped.push(SkippedSet {
                scope: store.scope.clone(),
                reason: store.why_unsearchable()?,
            });
        }
    }
    let complete = !stores.is_empty() && skipped.is_empty();
    let stores = searchable.as_slice();

    let Some(embedder) = stores.iter().find_map(|store| store.embedder.as_ref()) else {
        return Ok(Recall {
            hits: Vec::new(),
            searched: false,
            complete,
            skipped,
            query_unrepresentable: false,
            silenced_by_floor: false,
            answered_only_without_a_floor: Vec::new(),
        });
    };

    // `embed_each`, so a query the model has no tokens for is answered rather than failed. It was
    // `recall failed: embedding failed` until 2026-09-11 — an error for a question in the wrong
    // script, which is a fact about the question and not a fault anywhere.
    let embedded = embedder.embed_each(&[question.query.to_owned()])?;
    let vector = match embedded.into_iter().next() {
        Some(Embedded::Vector(vector)) => vector,
        Some(Embedded::Unrepresentable) => {
            return Ok(Recall {
                hits: Vec::new(),
                searched: false,
                complete,
                skipped,
                query_unrepresentable: true,
                silenced_by_floor: false,
                answered_only_without_a_floor: Vec::new(),
            });
        }
        None => {
            return Ok(Recall {
                hits: Vec::new(),
                searched: true,
                complete,
                skipped,
                query_unrepresentable: false,
                silenced_by_floor: false,
                answered_only_without_a_floor: Vec::new(),
            });
        }
    };

    let sets: Vec<&VectorSet> = stores.iter().map(|store| &store.vectors).collect();
    let Some(centroid) = combined_centroid(&sets) else {
        return Ok(Recall {
            hits: Vec::new(),
            searched: true,
            complete,
            skipped,
            query_unrepresentable: false,
            silenced_by_floor: false,
            answered_only_without_a_floor: Vec::new(),
        });
    };

    // The whole population is scored rather than a top slice of it, because the walk can lift a
    // memory content similarity ranked anywhere — which is the case traversal exists for. The scan
    // already computes every similarity before it truncates, so this costs a sort rather than a
    // pass.
    let mut scored: Vec<(f32, Located)> = Vec::new();
    // The best score each store produced, kept here because this is the last point at which a score
    // still knows which store it came from -- afterwards they are one merged ranking. `None` for a
    // store that scored nothing, which is a store with no vectors rather than a store that answered
    // badly, and the two must not log alike.
    //
    // `session` carries the labels this query would be logged with, and is `None` when nobody
    // asked it. **One binding rather than two readings of `question.asking`**: the extra scan below
    // and the write at the end have to agree about whether this query is traffic, and asking the
    // same field twice makes each of them a gate that hides the other — a mutation flipping either
    // one alone changes nothing, so neither can be proved by a test.
    let session = match question.asking {
        Asking::InASession {
            scoped,
            at,
            session,
        } => Some((scoped, at, session)),
        Asking::Silently => None,
    };
    let mut centred_top: Vec<Option<f32>> = vec![None; stores.len()];
    let mut raw_top: Vec<Option<f32>> = vec![None; stores.len()];
    for (index, store) in stores.iter().enumerate() {
        let width = store.vectors.len();
        let scanned = match stages.centring {
            Centring::Combined => store.vectors.nearest_centred_on(&vector, width, &centroid),
            Centring::PerStore => store.vectors.nearest(&vector, width),
            Centring::None => store.vectors.nearest_uncentred(&vector, width),
        };
        // A second pass over this store's vectors, and only for a session's question. **The floor
        // has to read the uncentred score**: centring removes the corpus centroid, which is exactly
        // the direction that marks a query as out of domain, so the centred score cannot answer
        // whether anything here answers this at all. `documents/design/recall-mechanisms.md`.
        if session.is_some() {
            raw_top[index] = store
                .vectors
                .nearest_uncentred(&vector, 1)
                .first()
                .map(|hit| hit.similarity);
        }
        for hit in scanned {
            centred_top[index] = Some(match centred_top[index] {
                Some(best) if best >= hit.similarity => best,
                _ => hit.similarity,
            });
            scored.push((
                hit.similarity,
                Located {
                    store: index,
                    id: hit.id,
                },
            ));
        }
    }
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .expect("similarity is finite by construction")
            .then_with(|| a.1.cmp(&b.1))
    });

    let walk = stages.walk;
    let neighbourhood = Neighbourhood::over(stores)?;

    // The seeds are the best content matches, resolved to **distinct current memories**. Two things
    // are being kept out. A supersession chain would otherwise spend several of these few slots on
    // memories that collapse to one, since its members resemble each other by construction. And a
    // memory that has been forgotten can still have a vector in the cache, which would take a slot
    // it can reach nothing from.
    //
    // Memories the caller says it already holds are *not* kept out: the parameter suppresses output,
    // not participation, and a memory the caller is querying around is the best available seed.
    let mut seeds: Vec<(Located, f64)> = Vec::new();
    let mut seeded: HashSet<Located> = HashSet::new();
    for (similarity, at) in &scored {
        if seeds.len() == walk.seeds {
            break;
        }
        let tip = neighbourhood.tip_of(*at);
        if neighbourhood.holds(tip) && seeded.insert(tip) {
            // The score is the *matched* memory's, which is what says how well this neighbourhood
            // answered the query — and it is the best-scoring member of the chain, because this
            // walks the ranking in order.
            seeds.push((tip, f64::from(*similarity)));
        }
    }
    let inherited = traversal::reachable(&neighbourhood.adjacency, &seeds, &walk);

    // **Validity is settled here, before anything is ranked.** Every scanned memory is resolved to
    // the memory it should be ranked *as*, and a supersession chain folds into one candidate
    // carrying the best evidence any of its members matched — which is what keeps a query that
    // matched an old claim's wording returning the correction, while everything downstream works in
    // memories that are current. The order used to be the other way round; see
    // `validity-is-resolved-after-the-ranking` in TRACKING.md.
    let mut candidates: HashMap<Located, Candidate> = HashMap::new();
    for (similarity, at) in scored {
        // Instrumentation only: dropping the replaced members leaves a chain scored on its current
        // text. `Ranked::InCycle` and `Unreplaced` are deliberately kept -- neither has
        // a resolvable tip in this store, so dropping them would remove the memory from the ranking
        // entirely rather than scoring it on something else.
        if !stages.include_superseded && matches!(neighbourhood.rank_as(at), Ranked::Replaced(_)) {
            continue;
        }
        let (identity, matched, in_cycle, beyond, forgotten) = match neighbourhood.rank_as(at) {
            Ranked::Itself => (at, None, false, false, false),
            Ranked::InCycle => (at, None, true, false, false),
            Ranked::Replaced(tip) => (tip, Some(at.id), false, false, false),
            // Competes as itself: there is no tip to stand in for it, and the whole point is that
            // the session gets to see it.
            Ranked::Unreplaced { beyond, forgotten } => (at, None, false, beyond, forgotten),
        };

        let candidate = candidates.entry(identity).or_insert(Candidate {
            similarity,
            inherited: 0.0,
            matched,
            in_cycle,
            beyond,
            forgotten,
        });
        // `scored` arrives best first, so the first entry for an identity is already its best
        // evidence and nothing later may lower it.
        if identity == at {
            candidate.in_cycle = in_cycle;
            candidate.beyond = beyond;
            candidate.forgotten = forgotten;
        }
    }

    let mut ranked: Vec<(f64, Located, Candidate)> = candidates
        .into_iter()
        .map(|(identity, candidate)| {
            // Looked up by the identity being ranked, which is now the memory that will be returned.
            let carried = inherited
                .get(&identity)
                .copied()
                .filter(|score| score.is_finite())
                .unwrap_or(0.0);
            (
                traversal::combine_with(
                    f64::from(candidate.similarity),
                    carried,
                    stages.inherited_weight,
                ),
                identity,
                Candidate {
                    #[allow(clippy::cast_possible_truncation)]
                    inherited: carried as f32,
                    ..candidate
                },
            )
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    // No slack and no de-duplication pass: the candidates were collapsed before they were ranked, so
    // the window's worth of scores is the window's worth of answers.
    let mut hits: Vec<Recalled> = Vec::new();
    // Which store each hit came from, kept alongside rather than looked up afterwards: a hit's
    // `scope` names a set and the log is written per *store*, and the two are the same thing only
    // for as long as no two loaded stores share a scope.
    let mut from: Vec<usize> = Vec::new();
    for (_, identity, candidate) in ranked {
        if hits.len() >= question.limit {
            break;
        }
        // Suppressed **before** the cut, because a memory removed afterwards would spend a slot on
        // nothing -- and passing the last call's results back is what makes "give me more" mean
        // anything, which is what lets the default cut be this small.
        if question.already_known.contains(&identity.id) {
            continue;
        }
        // **The identity names its store, so there is nothing to look up and nothing to guess.**
        // This used to carry the store that scanned the memory, set only when the identity was
        // itself scanned, and a candidate that never was got dropped. That worked because an id was
        // assumed to name one memory; under a shared id it also meant the last store scanned
        // decided whose content came back. A located identity answers both questions by
        // construction.
        let store = stores[identity.store];
        let Some(memory) = store.get(identity.id)? else {
            continue;
        };
        hits.push(Recalled {
            memory,
            scope: store.scope.clone(),
            incoming: store.incoming_edges(identity.id)?,
            similarity: candidate.similarity,
            inherited: candidate.inherited,
            matched: candidate.matched,
            in_cycle: candidate.in_cycle,
            superseded_beyond_this_set: candidate.beyond,
            replacement_forgotten: candidate.forgotten,
            disputed_by: claims_against_hit(stores, &store.scope, identity.id)?,
            referenced_from: referenced_from_other_sets(stores, &store.scope, identity.id)?,
        });
        from.push(identity.store);
    }

    // Per store, because a floor is per store: one set answering nothing says nothing about
    // another. Counted before and after rather than inferred from the surviving hits, so a store
    // that had nothing to lose is not recorded as having been silenced.
    // **One level per store, because a floor belongs to one store.** A level calibrated on one
    // corpus says nothing about another, so the shipping path asks each store for its own rather
    // than applying a single number across a multi-set recall.
    let floors: Vec<Option<f64>> = if question.ignore_floor {
        // The caller has waived it, which is the answer to a floor that silenced a question it
        // should not have. Applied over every source of a level, instrumentation included: a waiver
        // is about this question rather than about where the level came from.
        vec![None; stores.len()]
    } else {
        match stages.floor {
            Floor::Calibrated => {
                let mut levels = Vec::with_capacity(stores.len());
                for store in stores {
                    levels.push(store.floor()?);
                }
                levels
            }
            Floor::None => vec![None; stores.len()],
            Floor::Fixed(level) => vec![Some(level); stores.len()],
        }
    };

    let mut silenced = vec![false; stores.len()];
    // Whether a floor emptied the answer entirely, which is what the caller is told. Per store for
    // the log, and once for the caller: a session asked one question and got nothing back.
    let mut silenced_by_floor = false;
    if floors.iter().any(Option::is_some) {
        let mut before = vec![0_usize; stores.len()];
        for store in &from {
            before[*store] += 1;
        }
        // **Raw, not the centred score beside it.** Centring removes the corpus centroid, which is
        // exactly the direction that marks a query as out of domain, so the ranking's own number
        // cannot answer the one absolute question there is.
        //
        // Asked of the memory whose score is being carried -- `matched`, where a supersession chain
        // folded an older member's evidence onto its tip -- so the two numbers describe the same
        // text. A memory whose vector has gone missing is kept: failing open lands on the
        // over-recalling end, which is the safe one.
        let keep: Vec<bool> = hits
            .iter()
            .zip(&from)
            .map(|(hit, store)| {
                let Some(floor) = floors[*store] else {
                    return true;
                };
                let scored = hit.matched.unwrap_or(hit.memory.id);
                stores[*store]
                    .vectors
                    .raw_similarity(scored, &vector)
                    .is_none_or(|raw| f64::from(raw) >= floor)
            })
            .collect();
        retain_by(&mut hits, &keep);
        retain_by(&mut from, &keep);
        let mut after = vec![0_usize; stores.len()];
        for store in &from {
            after[*store] += 1;
        }
        for (index, silenced) in silenced.iter_mut().enumerate() {
            *silenced = before[index] > 0 && after[index] == 0;
        }
        silenced_by_floor = hits.is_empty() && keep.iter().any(|kept| !kept);
    }

    if stages.gap_cut {
        let scores: Vec<f64> = hits
            .iter()
            .map(|hit| {
                traversal::combine_with(
                    f64::from(hit.similarity),
                    f64::from(hit.inherited),
                    stages.inherited_weight,
                )
            })
            .collect();
        let cut = traversal::cut_at_largest_gap(&scores, question.limit);
        hits.truncate(cut);
        from.truncate(cut);
    }

    // **Last, so that what is recorded is what the session actually received.** The scores are the
    // ones taken before the floor, which is the point: a floor that silenced a store leaves the
    // evidence that it did, where the hits alone would show only an empty answer.
    if let Some((scoped, at, asked_by)) = session {
        let model = embedder.identity();
        for (index, store) in stores.iter().enumerate() {
            let (Some(raw_top), Some(centred_top)) = (raw_top[index], centred_top[index]) else {
                // Nothing in this store was scored, so it has no evidence to offer about where a
                // floor belongs. An empty store, not an unanswered query.
                continue;
            };
            // **A copy loaded for review logs nothing, so it calibrates nothing.** Naming its scope
            // is the only way to search one, and a scoped query is exactly what calibration reads as
            // evidence that a set answered well -- so every question a reviewer asked while reading
            // someone else's branch would set that copy's floor. The copy also dies with the session,
            // so the level would describe a corpus nobody keeps. `documents/design/load-modes.md`.
            if store.load_mode().is_ok_and(LoadMode::is_evaluation) {
                continue;
            }
            // **Both ids of a substituted hit**, because the caller received both: the tip it was
            // handed, and the superseded member named as `replaces_matched`, which is how a session
            // reads what was claimed before. Recording only the tip made a lookup of that member
            // invisible as a use, on roughly 10% of returned hits.
            let returned = hits
                .iter()
                .zip(&from)
                .filter(|(_, at)| **at == index)
                .flat_map(|(hit, _)| std::iter::once(hit.memory.id).chain(hit.matched))
                .collect();
            store.log_query(
                &LoggedQuery {
                    asked_at: at,
                    model: model.clone(),
                    scoped,
                    raw_top,
                    centred_top,
                    silenced: silenced[index],
                    returned,
                    // A row is written unlabelled and labelled later, if the session uses one of
                    // the memories it carried. See `Store::note_used`.
                    used: false,
                },
                asked_by,
            )?;
        }
    }

    // Every hit from a set with no floor, while some searched set has one: those sets declined, or
    // were outranked by a set that cannot tell a non-match from an answer. `floors` is all `None`
    // when the caller waived them, so a waiver is never marked -- nothing was declined.
    let answered_only_without_a_floor = if !hits.is_empty()
        && floors.iter().any(Option::is_some)
        && from.iter().all(|store| floors[*store].is_none())
    {
        let mut sets: Vec<Scope> = Vec::new();
        for store in &from {
            let scope = stores[*store].scope();
            if !sets.contains(scope) {
                sets.push(scope.clone());
            }
        }
        sets
    } else {
        Vec::new()
    };

    Ok(Recall {
        hits,
        searched: true,
        complete,
        skipped,
        query_unrepresentable: false,
        silenced_by_floor,
        answered_only_without_a_floor,
    })
}

/// Keep the elements `keep` marks, which is `Vec::retain` for two vectors that must stay aligned.
fn retain_by<T>(values: &mut Vec<T>, keep: &[bool]) {
    let mut index = 0;
    values.retain(|_| {
        let kept = keep[index];
        index += 1;
        kept
    });
}

/// Raw little-endian `f32`s. No wire format to keep stable: vectors are local derived state and
/// never leave the machine.
/// The row recording a model's verdict that a memory's text has nothing it can represent.
///
/// **An empty blob rather than `NULL`**, because the column is `NOT NULL` and relaxing that means
/// rebuilding the table in every existing store. An empty blob needs no migration and cannot be
/// mistaken for a vector, since [`Vector`] refuses one with no components. Every reader of `vectors`
/// has to tell the two apart: `length(vector) > 0` in SQL, and an empty blob skipped when loading.
const UNREPRESENTABLE: &[u8] = &[];

/// What a model made of some memories, asked for and not yet written.
struct Embeddings {
    model: ModelIdentity,
    dimension: usize,
    outcomes: Vec<(MemoryId, Embedded)>,
}

/// How many rows an embedding pass wrote, of each kind.
#[derive(Debug, Default, Clone, Copy)]
struct Outcomes {
    vectors: usize,
    unrepresentable: usize,
}

/// Write what a model made of some memories: a vector, or its verdict that there is nothing to
/// represent.
///
/// A free function over a connection rather than a method, so a write can call it inside the same
/// transaction as the memory it belongs to.
fn write_embeddings(
    connection: &Connection,
    outcomes: &[(MemoryId, Embedded)],
) -> Result<(), StoreError> {
    for (id, outcome) in outcomes {
        let blob = match outcome {
            Embedded::Vector(vector) => encode_vector(vector),
            Embedded::Unrepresentable => UNREPRESENTABLE.to_vec(),
        };
        connection.execute(
            "INSERT OR REPLACE INTO vectors (memory_id, vector) VALUES (?1, ?2)",
            params![id.to_string(), blob],
        )?;
    }
    Ok(())
}

fn encode_vector(vector: &Vector) -> Vec<u8> {
    vector
        .components()
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// The encoding of a shared scope in `meta`, which is now [`Scope`]'s own.
///
/// It was spelled by hand in both directions here until 2026-08-04, when renaming the `Project`
/// variant changed the writer and left the reader matching a prefix nothing produced any more —
/// with a fully green suite, because the only caller of the reader is an error path. Collapsing the
/// pair into one constant fixed that.
///
/// **Moved onto `Scope` itself on 2026-08-11**, when the tool surface began publishing the same
/// spelling: a scope has to be expressible as a scalar string for clients that discard a nested
/// `oneOf` (`documents/design/tool-surface.md`). Keeping a private copy here would have restored
/// the two-writers defect across modules instead of within one — the same shape, one level out, and
/// harder to see.
fn scope_key(scope: &Scope) -> String {
    scope.as_key()
}

fn scope_from_key(key: &str) -> Result<Scope, ValidationError> {
    Scope::from_key(key)
}

/// Whether `memory` carries the keyword `mark`, compared caselessly as keywords are matched.
fn carries(memory: &Memory, mark: &str) -> bool {
    memory
        .keywords
        .iter()
        .any(|keyword| keyword.folded() == mark)
}

/// The same-set memories `memory` says it supersedes.
fn superseded_by(memory: &Memory) -> HashSet<MemoryId> {
    memory
        .links
        .iter()
        .filter(|link| link.label == LinkLabel::Supersedes)
        .filter_map(|link| match link.target {
            LinkTarget::SameSet(target) => Some(target),
            LinkTarget::CrossSet { .. } => None,
        })
        .collect()
}

/// The language tags a memory carries, in the caseless form, without the `hoard-lang-` prefix.
fn language_tags(memory: &Memory) -> Vec<String> {
    memory
        .keywords
        .iter()
        .filter_map(|keyword| {
            keyword
                .folded()
                .strip_prefix(LANGUAGE_KEYWORD_PREFIX)
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned)
        })
        .collect()
}

/// The refusal for a recommendation linking to no hub, which depends on whether there is one.
///
/// **With none, it shows the hub as a call would write it.** Told only to write the hub first, a
/// small local model linked to the charter instead and repeated the refused call four times
/// (2026-10-04); the missing-query refusal went from 0 to 10 of 10 on the same model by showing the
/// call. With one, it names it, since linking to it is the whole of the fix.
fn unlinked_fit_model(memory: MemoryId, hub: Option<MemoryId>) -> String {
    let opening = format!(
        "memory {memory} carries `hoard-fit-model` but links to no live `hoard-fit-hub` memory in \
         this set, and that link is how a recommendation is found -- without it the mark would do \
         nothing."
    );
    match hub {
        Some(hub) => {
            format!(
                "{opening} Add a `refers_to` link to this set's hub, {hub}, and store it again."
            )
        }
        None => format!(
            "{opening} This set has no hub yet, so store one first -- for example \
             {{\"content\": \"This set's model-fit hub.\", \"keywords\": [\"hoard-fit-hub\"]}} \
             with this set's scope -- and then store this memory again with a `refers_to` link to \
             the id that call returns."
        ),
    }
}
