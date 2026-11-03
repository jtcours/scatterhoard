// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the non-match floor: where it is kept, and how it is calibrated.
//!
//! The floor is the scorer's one absolute question — *does anything here answer this* — and every
//! other stage is relative, which is why an unanswerable query otherwise comes back with a full
//! window of confident-looking results. A constant is wrong on its face: the same probes put the
//! floor at 0.089 on an eight-memory set and 0.286 on a 429-memory one. So it is calibrated per
//! store, from that store's own query log, and only once there is enough of one.
//!
//! `documents/design/recall-mechanisms.md`, *the floor is calibrated from a local query log, and
//! reads raw similarity*.

use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use scatterhoard::embedder::{Embedded, Embedder, EmbedderError, ModelIdentity};
use scatterhoard::embedding::Vector;
use scatterhoard::memory::{
    Attribution, Confidence, Link, LinkLabel, LinkTarget, Memory, MemoryId, MemoryText, Origin,
    Scope, SetName,
};
use scatterhoard::store::{
    Calibration, Floor, FloorSource, LABELS_PER_CALIBRATION, Question, SessionId, Stages, Store,
    recall_across_with,
};

#[derive(Debug)]
struct MarkerEmbedder;

impl Embedder for MarkerEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("marker-embedder")
    }

    fn dimension(&self) -> usize {
        4
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|text| {
                let count = |word: &str| text.matches(word).count() as f32;
                Vector::new(vec![count("alpha"), count("beta"), count("gamma"), 0.01])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

/// A different model, so that a store can be made to change the one it is bound to.
#[derive(Debug)]
struct OtherEmbedder;

impl Embedder for OtherEmbedder {
    fn identity(&self) -> ModelIdentity {
        ModelIdentity::new("other-embedder")
    }

    fn dimension(&self) -> usize {
        4
    }

    fn embed_each(&self, texts: &[String]) -> Result<Vec<Embedded>, EmbedderError> {
        texts
            .iter()
            .map(|text| {
                let count = |word: &str| text.matches(word).count() as f32;
                Vector::new(vec![0.01, count("gamma"), count("beta"), count("alpha")])
                    .map(Embedded::Vector)
                    .map_err(EmbedderError::from)
            })
            .collect()
    }
}

fn shared() -> Scope {
    Scope::Shared(SetName::new("scatterhoard").expect("valid set name"))
}

fn a_store() -> Store {
    let mut store = Store::open_in_memory_and_mint_id(shared()).expect("opens");
    store.set_embedder(Arc::new(MarkerEmbedder));
    store
}

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

fn asked_at() -> Timestamp {
    Timestamp::from_str("2026-09-11T10:00:00Z").expect("valid timestamp")
}

// ---------------------------------------------------------------------------
// Where the floor is kept
// ---------------------------------------------------------------------------

#[test]
fn a_store_starts_with_no_floor() {
    assert_eq!(a_store().floor().expect("reads the floor"), None);
}

#[test]
fn a_floor_survives_being_written_and_read_back() {
    let store = a_store();
    store.set_floor(0.31).expect("sets the floor");
    assert_eq!(store.floor().expect("reads the floor"), Some(0.31));
}

/// **A floor belongs to one store and one model.** Every set's floor moved 0.10 to 0.16 under
/// `potion-multilingual-128M`, and between a third and a half of the spread between sets moved with
/// it — so a level calibrated under one model says nothing under another. It is cleared exactly
/// where the vectors are, and for the same reason.
#[test]
fn changing_the_model_clears_the_floor() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    store.set_floor(0.31).expect("sets the floor");

    store.set_embedder(Arc::new(OtherEmbedder));
    store.rebuild_vectors().expect("rebuilds");

    assert_eq!(
        store.floor().expect("reads the floor"),
        None,
        "a floor calibrated under the previous model outlived it"
    );
}

/// Rebuilding under the *same* model displaces nothing, so it must not throw the floor away either:
/// the level still describes this store scored by this model.
#[test]
fn rebuilding_under_the_same_model_keeps_the_floor() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    store.set_floor(0.31).expect("sets the floor");

    store.rebuild_vectors().expect("rebuilds");

    assert_eq!(store.floor().expect("reads the floor"), Some(0.31));
}

// ---------------------------------------------------------------------------
// Calibration
// ---------------------------------------------------------------------------

/// Ask `count` questions that this store can answer, labelling each the way a real session would.
fn ask_answerable(store: &mut Store, count: usize) {
    if store.count().expect("counts") == 0 {
        store.insert(&a_memory("alpha alpha")).expect("inserts");
    }
    for _ in 0..count {
        let session = SessionId::generate();
        let recall = store
            .recall(&Question::new("alpha", 1).in_a_session(false, asked_at(), session))
            .expect("recalls");
        // Whatever came back, rather than an id chosen in advance: two memories of identical text
        // are ranked by id, so naming one up front labels the query only by luck.
        for hit in &recall.hits {
            store.note_used(hit.memory.id, session).expect("marks");
        }
    }
}

/// **No floor until enough labelled queries have arrived since the last calibration.** Swept over
/// both stores 2026-09-12: recalibrating every 20 new labels loses 6.2% of later labelled queries
/// where every 40 loses 15.6%, because a floor set from fewer, staler labels tracks worse. Below the
/// trigger nothing is set, which is the over-recalling end and therefore the safe one.
#[test]
fn calibration_sets_no_floor_before_there_is_enough_evidence() {
    let mut store = a_store();
    ask_answerable(&mut store, 10);

    let calibration = store.calibrate().expect("calibrates");

    assert_eq!(calibration, Calibration::NotEnoughEvidence { labelled: 10 });
    assert_eq!(store.floor().expect("reads the floor"), None);
}

/// **The trigger counts labels since the last calibration, not labels in total.** Otherwise the
/// second calibration fires on evidence the first already consumed, and a level that should move
/// with the store is recomputed from the same queries.
#[test]
fn calibration_needs_new_labels_since_the_last_one() {
    let mut store = a_store();
    ask_answerable(&mut store, 20);
    assert!(matches!(
        store.calibrate().expect("calibrates"),
        Calibration::Set { .. }
    ));

    assert_eq!(
        store.calibrate().expect("calibrates"),
        Calibration::NotEnoughEvidence { labelled: 0 },
        "a second calibration fired on labels the first had already used"
    );

    ask_answerable(&mut store, 20);
    assert!(matches!(
        store.calibrate().expect("calibrates"),
        Calibration::Set { .. }
    ));
}

/// **The point of the whole redesign: the level moves up as well as down.** Under the old rule it
/// was the minimum over an ever-growing set, which can only fall — so one scoped question the set
/// could not answer destroyed the floor permanently. Measured on the real query history, a windowed
/// percentile runs 0.307 up to 0.469 and back down across three recalibrations.
#[test]
fn calibration_can_raise_the_floor_as_well_as_lower_it() {
    let mut store = a_store();
    let weak = a_memory("alpha beta gamma");
    store.insert(&weak).expect("inserts");

    // Twenty poor answers first: the query shares one marker with the memory, so it scores low.
    for _ in 0..20 {
        let session = SessionId::generate();
        store
            .recall(&Question::new("gamma gamma gamma", 1).in_a_session(false, asked_at(), session))
            .expect("recalls");
        store.note_used(weak.id, session).expect("marks");
    }
    store.calibrate().expect("calibrates");
    let low = store.floor().expect("reads").expect("a floor");

    // Then enough good ones to push the poor scores out of the bounded window entirely. That is
    // what makes a rise possible: while the old labels are still in the window a low percentile
    // keeps finding them, which is the whole reason the set is bounded rather than merely windowed
    // by time.
    for _ in 0..110 {
        let session = SessionId::generate();
        let recall = store
            .recall(&Question::new("alpha beta gamma", 1).in_a_session(false, asked_at(), session))
            .expect("recalls");
        for hit in &recall.hits {
            store.note_used(hit.memory.id, session).expect("marks");
        }
    }
    store.calibrate().expect("calibrates");
    let high = store.floor().expect("reads").expect("a floor");

    assert!(
        high > low,
        "a run of better-answered queries should raise the floor, and it went {low} -> {high}"
    );
}

/// **The labelled set is bounded**, so the level is read off recent traffic rather than everything
/// the store has ever been asked. An unbounded set is what made recalibration a ratchet.
#[test]
fn the_labelled_window_is_bounded() {
    let mut store = a_store();
    ask_answerable(&mut store, 120);

    let labelled = store
        .query_log()
        .expect("reads the log")
        .iter()
        .filter(|query| query.used || query.scoped)
        .count();
    assert!(
        labelled <= 100,
        "the labelled set should be bounded at 100 and holds {labelled}"
    );
}

/// An unlabelled row from a finished session can never become labelled — `note_used` is bounded to
/// one session — so it is evidence about the labelling rate and nothing else, and is the first thing
/// dropped when the log is pruned.
#[test]
fn unlabelled_rows_are_bounded_too() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    for _ in 0..120 {
        store
            .recall(&Question::new("alpha", 1).in_a_session(
                false,
                asked_at(),
                SessionId::generate(),
            ))
            .expect("recalls");
    }

    let rows = store.query_log().expect("reads the log").len();
    assert!(rows <= 100, "the log should be bounded and holds {rows}");
}

#[test]
fn calibration_sets_a_floor_once_there_is() {
    let mut store = a_store();
    ask_answerable(&mut store, 40);

    let calibration = store.calibrate().expect("calibrates");

    let Calibration::Set { floor, labelled } = calibration else {
        panic!("expected a floor to be set, got {calibration:?}");
    };
    assert_eq!(labelled, 40);
    assert_eq!(store.floor().expect("reads the floor"), Some(floor));
}

/// The floor sits a margin *below* the lowest labelled score, never at it. A floor at the minimum
/// would silence the very query that proved the set could answer, on the next run of the same
/// question.
#[test]
fn the_floor_sits_a_margin_below_the_lowest_labelled_score() {
    let mut store = a_store();
    ask_answerable(&mut store, 40);

    let Calibration::Set { floor, .. } = store.calibrate().expect("calibrates") else {
        panic!("expected a floor");
    };
    let lowest = store
        .query_log()
        .expect("reads the log")
        .iter()
        .filter(|query| query.used || query.scoped)
        .map(|query| f64::from(query.raw_top))
        .fold(f64::INFINITY, f64::min);

    assert!(
        floor < lowest,
        "floor {floor} should sit below the lowest labelled score {lowest}"
    );
    assert!(
        (lowest - floor - 0.05).abs() < 1e-9,
        "the margin should be 0.05, and is {}",
        lowest - floor
    );
}

/// **A scoped query counts even though nothing was used**, and it is the label a floor cannot
/// suppress: naming a set is an argument of the call rather than a consequence of the answer, so it
/// survives a floor that silenced everything.
#[test]
fn naming_the_scope_labels_a_query_on_its_own() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    for _ in 0..40 {
        store
            .recall(&Question::new("alpha", 1).in_a_session(
                true,
                asked_at(),
                SessionId::generate(),
            ))
            .expect("recalls");
    }

    assert!(matches!(
        store.calibrate().expect("calibrates"),
        Calibration::Set { labelled: 40, .. }
    ));
}

/// An unlabelled query is not evidence about where the floor belongs: nothing says the store could
/// answer it, which is exactly the case a floor exists to silence.
#[test]
fn an_unlabelled_query_is_not_evidence() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    for _ in 0..40 {
        store
            .recall(&Question::new("alpha", 1).in_a_session(
                false,
                asked_at(),
                SessionId::generate(),
            ))
            .expect("recalls");
    }

    assert_eq!(
        store.calibrate().expect("calibrates"),
        Calibration::NotEnoughEvidence { labelled: 0 }
    );
}

/// **A query the floor silences is still logged, still labelled, and still carries its raw score.**
///
/// This is the whole answer to a floor hiding the evidence that would correct it. Once a floor
/// silences a query nothing comes back, so no memory can be used and `used` can never arrive — but
/// `scope` is an argument of the call rather than a consequence of the answer, so it survives, and
/// the score recorded beside it is the one taken *before* the floor. The next calibration therefore
/// reads the contradiction as an ordinary label.
///
/// Until 2026-09-12 recall wrote the floor down to that score on the spot. That made the floor a
/// ratchet: a minimum over an ever-growing set can only fall, so a single scoped question the set
/// could not answer destroyed the level permanently and nothing could raise it again.
#[test]
fn a_scoped_query_the_floor_silences_is_still_logged_and_labelled() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    store.set_floor(0.9).expect("sets the floor");

    let recall = store
        .recall(&Question::new("gamma", 1).in_a_session(true, asked_at(), SessionId::generate()))
        .expect("recalls");
    assert!(
        recall.hits.is_empty(),
        "the fixture floor let something past"
    );

    let logged = store.query_log().expect("reads the log");
    assert_eq!(logged.len(), 1, "a silenced query was not logged");
    assert!(
        logged[0].scoped,
        "the label that survives silencing did not"
    );
    assert!(
        f64::from(logged[0].raw_top) < 0.9,
        "the score recorded is the one taken before the floor, and it should contradict it"
    );
    assert_eq!(
        store.floor().expect("reads the floor"),
        Some(0.9),
        "recall must not move the floor itself -- the contradiction is a label, not a write"
    );
}

// ---------------------------------------------------------------------------
// What a silenced query is told
// ---------------------------------------------------------------------------

/// **A floor that silences everything must say so.** Silence and *nothing matched* are the same
/// empty result, and the caller is the only one who can tell whether the question deserved an
/// answer — so the outcome is reported rather than left to be inferred.
#[test]
fn a_floor_that_silences_everything_is_reported() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    store.set_floor(0.9).expect("sets the floor");

    let recall = store
        .recall(&Question::new("gamma", 5).in_a_session(false, asked_at(), SessionId::generate()))
        .expect("recalls");

    assert!(recall.hits.is_empty());
    assert!(
        recall.silenced_by_floor,
        "a floor removed every hit and the recall did not report it"
    );
}

/// Nothing was silenced, so nothing is claimed. A query that genuinely matched nothing must not be
/// dressed up as one the floor suppressed — they call for different next moves.
#[test]
fn a_query_that_matched_nothing_is_not_reported_as_silenced() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");

    let recall = store
        .recall(&Question::new("gamma", 5).in_a_session(false, asked_at(), SessionId::generate()))
        .expect("recalls");

    assert!(!recall.silenced_by_floor);
}

/// **And the caller may ask again without it**, which is what keeps the mechanism honest at the
/// moment it costs something: the floor is a calibrated guess, and the session asking is better
/// placed than the store to know whether this particular question deserved an answer anyway.
#[test]
fn a_caller_may_ask_again_without_the_floor() {
    let mut store = a_store();
    store.insert(&a_memory("alpha alpha")).expect("inserts");
    store.set_floor(0.9).expect("sets the floor");

    let recall = store
        .recall(&Question::new("gamma", 5).without_the_floor().in_a_session(
            false,
            asked_at(),
            SessionId::generate(),
        ))
        .expect("recalls");

    assert_eq!(
        recall.hits.len(),
        1,
        "asking without the floor returned nothing"
    );
    assert!(
        !recall.silenced_by_floor,
        "nothing was silenced, because no floor was applied"
    );
}

/// **A floor belongs to one store, so a multi-set recall applies each store's own.** A level is read
/// off one corpus's history and means nothing for another — applying one across the union would
/// silence a set that had never calibrated anything, on a number belonging to somebody else.
///
/// The gap cut is off so that what is being measured is the floor alone: with it on, the weaker hit
/// might fall to the cut and the two outcomes would be indistinguishable.
#[test]
fn each_store_is_judged_against_its_own_floor() {
    let mut unfloored = Store::open_in_memory(Scope::Local).expect("opens");
    unfloored.set_embedder(Arc::new(MarkerEmbedder));
    unfloored.insert(&a_memory("alpha alpha")).expect("inserts");

    let mut floored = a_store();
    floored.insert(&a_memory("beta beta")).expect("inserts");
    floored.set_floor(0.9).expect("sets the floor");

    let recall = recall_across_with(
        &mut [&mut unfloored, &mut floored],
        &Question::new("alpha", 5),
        Stages {
            gap_cut: false,
            floor: Floor::Calibrated,
            ..Stages::default()
        },
    )
    .expect("recalls");

    assert_eq!(
        recall.hits.len(),
        1,
        "the unfloored set's answer should survive and the floored set's should not, and {} hits \
         came back",
        recall.hits.len()
    );
    assert_eq!(recall.hits[0].scope, Scope::Local);
}

/// **A set can say how much evidence has accrued since its last calibration**, which is what lets
/// anything report that calibrating would now set a floor. Counted the way `calibrate` counts it,
/// from one implementation, so the report and the operation cannot disagree.
#[test]
fn a_store_says_how_many_labels_have_arrived_since_its_last_calibration() {
    let mut store = a_store();
    assert_eq!(store.labels_since_calibration().expect("counts"), 0);

    ask_answerable(&mut store, LABELS_PER_CALIBRATION);
    assert_eq!(
        store.labels_since_calibration().expect("counts"),
        LABELS_PER_CALIBRATION,
        "every labelled query counts until a calibration consumes it"
    );

    store.calibrate().expect("calibrates");
    assert_eq!(
        store.labels_since_calibration().expect("counts"),
        0,
        "a calibration consumes the evidence it read"
    );

    ask_answerable(&mut store, 3);
    assert_eq!(store.labels_since_calibration().expect("counts"), 3);
}

/// **A copy loaded for review logs no queries, so it calibrates nothing.** Naming its scope is the
/// only way to search one, and a scoped query is what calibration reads as evidence a set answered
/// well -- so a reviewer's questions would set the floor of a copy nobody adopted and nothing keeps.
#[test]
fn an_evaluation_store_logs_no_queries() {
    let mut store = a_store();
    store
        .choose_evaluation()
        .expect("an empty store may be one");
    store
        .accept_from_file(&a_memory("alpha alpha"))
        .expect("takes the file's record");

    let session = SessionId::generate();
    store
        .recall(&Question::new("alpha", 1).in_a_session(true, asked_at(), session))
        .expect("recalls");

    assert_eq!(
        store.labels_since_calibration().expect("counts"),
        0,
        "a review asks questions of someone else's set, not evidence about this machine's"
    );
    assert!(store.query_log().expect("reads the log").is_empty());
}

// ---------------------------------------------------------------------------
// A floor from the set's own memories, for a set without enough labels
// ---------------------------------------------------------------------------

/// A set of `current` memories with no labelled queries, plus one more that the last supersedes when
/// `with_a_superseded_one` -- so the record count and the count of current claims differ by one.
fn a_quiet_set(current: usize, with_a_superseded_one: bool) -> Store {
    let mut store = a_store();
    let texts = [
        "alpha alpha beta",
        "beta beta gamma",
        "gamma gamma alpha",
        "alpha beta gamma",
    ];
    let mut last = None;
    for index in 0..current {
        let memory = a_memory(texts[index % texts.len()]);
        last = Some(memory.id);
        store.insert(&memory).expect("inserts");
    }
    if with_a_superseded_one {
        let mut replacement = a_memory("alpha alpha beta, corrected");
        replacement.links.push(Link {
            label: LinkLabel::Supersedes,
            target: LinkTarget::SameSet(last.expect("at least one memory")),
        });
        store.insert(&replacement).expect("inserts");
    }
    store
}

/// **A set too little used to gather labels still gets a floor, from its own memories.**
/// `calibration-from-self-queries`: each current memory's index line asked against the rest, with
/// itself left out, at the same percentile and margin as labels. Measured 2026-09-24 by
/// `probe_a_floor_without_labelled_queries`, it lands within 0.012 of the label-derived floor on
/// both sets that have one. The source is recorded, because it is weaker evidence than labels.
#[test]
fn a_set_without_labels_gets_a_floor_from_its_own_memories() {
    let mut store = a_quiet_set(LABELS_PER_CALIBRATION, false);

    let calibration = store.calibrate().expect("calibrates");

    let Calibration::FromItsOwnMemories { floor, memories } = calibration else {
        panic!("expected a floor from the set's own memories, got {calibration:?}");
    };
    assert_eq!(memories, LABELS_PER_CALIBRATION);
    assert_eq!(store.floor().expect("reads the floor"), Some(floor));
    assert_eq!(
        store.floor_source().expect("reads the source"),
        Some(FloorSource::ItsOwnMemories)
    );
}

/// **Only current claims count toward the minimum**, since a superseded memory's line all but
/// repeats its replacement's. Twenty records of which nineteen are current is too few: at that size
/// the percentile is the set's weakest memory, and on an eight-memory set that placed no useful floor.
#[test]
fn a_set_with_too_few_current_memories_gets_no_floor_from_them() {
    let mut store = a_quiet_set(LABELS_PER_CALIBRATION - 1, true);

    let calibration = store.calibrate().expect("calibrates");

    assert!(
        matches!(calibration, Calibration::NotEnoughEvidence { .. }),
        "nineteen current memories should not be enough, got {calibration:?}"
    );
    assert_eq!(store.floor().expect("reads the floor"), None);
}

/// **Labels outrank the set's own memories**, both ways round: a floor read from labels is never
/// replaced by one from memories while too few new labels have arrived, and a floor from memories
/// gives way to labels once they have.
#[test]
fn a_floor_from_labels_outranks_one_from_the_sets_own_memories() {
    let mut store = a_quiet_set(LABELS_PER_CALIBRATION, false);
    assert!(matches!(
        store.calibrate().expect("calibrates"),
        Calibration::FromItsOwnMemories { .. }
    ));

    // **Scoped, because a floor hides the uses that would label a query.** Under the floor just set,
    // an unscoped question this fixture's memories only partly answer is silenced, so nothing is
    // used and nothing labelled; `scope` is the label that survives silencing, which is how labels
    // reach a set with a floor at all.
    for _ in 0..LABELS_PER_CALIBRATION {
        store
            .recall(&Question::new("alpha", 1).in_a_session(
                true,
                asked_at(),
                SessionId::generate(),
            ))
            .expect("recalls");
    }
    assert!(
        matches!(
            store.calibrate().expect("calibrates"),
            Calibration::Set { .. }
        ),
        "labels should replace a floor from memories once enough have arrived"
    );
    assert_eq!(
        store.floor_source().expect("reads the source"),
        Some(FloorSource::Labels)
    );

    let from_labels = store.floor().expect("reads the floor");
    assert!(
        matches!(
            store.calibrate().expect("calibrates"),
            Calibration::NotEnoughEvidence { .. }
        ),
        "with no new labels, a floor from labels stands"
    );
    assert_eq!(store.floor().expect("reads the floor"), from_labels);
    assert_eq!(
        store.floor_source().expect("reads the source"),
        Some(FloorSource::Labels)
    );
}

/// **A memory does not answer its own question.** Each index line is asked against the *rest* of the
/// set: left in, every memory would find itself at a similarity of one and the floor would sit a bare
/// margin under a perfect match, silencing nearly everything. Twenty distinct mixes of the embedder's
/// three words, so no memory has a twin to find instead.
#[test]
fn a_floor_from_its_own_memories_leaves_each_memory_out_of_its_own_question() {
    let mut store = a_store();
    let mut mixes = Vec::new();
    for alpha in 1..=4 {
        for beta in 0..=4 {
            if mixes.len() < LABELS_PER_CALIBRATION {
                mixes.push((alpha, beta, 5 - beta));
            }
        }
    }
    for (alpha, beta, gamma) in mixes {
        let text = format!(
            "{}{}{}",
            "alpha ".repeat(alpha),
            "beta ".repeat(beta),
            "gamma ".repeat(gamma)
        );
        store.insert(&a_memory(text.trim())).expect("inserts");
    }

    let Calibration::FromItsOwnMemories { floor, .. } = store.calibrate().expect("calibrates")
    else {
        panic!("expected a floor from the set's own memories");
    };
    assert!(
        floor < 0.94,
        "a floor a bare margin under a perfect match means each memory answered itself: {floor}"
    );
}

/// **Removing a floor removes what it was read from**, or `status` would report a source for a
/// level that no longer exists.
#[test]
fn uncalibrating_forgets_what_the_floor_was_read_from() {
    let mut store = a_quiet_set(LABELS_PER_CALIBRATION, false);
    store.calibrate().expect("calibrates");
    assert!(store.floor_source().expect("reads").is_some());

    store.uncalibrate().expect("uncalibrates");

    assert_eq!(store.floor_source().expect("reads"), None);
}
