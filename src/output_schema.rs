// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! What each tool returns, published as an MCP `outputSchema`.
//!
//! `response-shape-is-unpublished`: every tool published what goes in and nothing about what comes
//! back, and the fields that vary are the ones a client sorts and thresholds on -- `similarity` is
//! absent exactly where a superseded memory was swapped for its replacement. A shape nothing
//! publishes drifts silently; one instance did, a `scope` spelled two ways for weeks.
//!
//! **What keeps these honest is the tests, not this file.** Every tool response in the approval,
//! MCP and sets suites is checked against its tool's schema, and checked strictly: a field the
//! schema does not declare fails the test, so a new field cannot arrive undocumented. The schemas
//! as published allow undeclared fields, so a client validating against them is never broken by a
//! field added after it was written. That asymmetry is deliberate: tests strict, contract lenient.
//!
//! **`required` names only what is present on every response.** A field that happens to appear in
//! every test is not thereby always present -- a summary-detail recall hit carries `id` and
//! `summary` and nothing else -- and a wrongly required field is the one mistake that would break a
//! strict client. Fields that appear only sometimes say when in their description.
//!
//! The schemas use a small subset of JSON Schema -- `type`, `properties`, `required`, `items`,
//! `anyOf`, `description` -- which is all the shapes need and all the tests check.

use serde_json::{Value, json};

/// The output schema for a tool, or `None` for one that returns text rather than JSON.
///
/// `hoard_conventions` is the `None`: a page is a slice of one document plus lines addressed to the
/// reader, and its description says how to read one.
#[must_use]
pub fn for_tool(name: &str) -> Option<Value> {
    match name {
        "hoard_recall" => Some(recall()),
        "hoard_lookup" => Some(lookup()),
        "hoard_memorize" => Some(memorize()),
        "hoard_correct" => Some(correct()),
        "hoard_sets" => Some(sets()),
        _ => None,
    }
}

/// Attach `description` unless it is empty. **Descriptions say only what a name and type cannot**:
/// when a field appears, and what an absence or a null means. Restating `id: the memory's id`
/// nearly tripled the tool list for nothing, and the list may sit in a model's context.
fn described(mut schema: Value, description: &str) -> Value {
    if !description.is_empty() {
        schema["description"] = json!(description);
    }
    schema
}

fn text(description: &str) -> Value {
    described(json!({"type": "string"}), description)
}

fn flag(description: &str) -> Value {
    described(json!({"type": "boolean"}), description)
}

fn count(description: &str) -> Value {
    described(json!({"type": "integer"}), description)
}

fn score(description: &str) -> Value {
    described(json!({"type": "number"}), description)
}

fn list(items: &Value, description: &str) -> Value {
    described(json!({"type": "array", "items": items}), description)
}

fn strings(description: &str) -> Value {
    list(&json!({"type": "string"}), description)
}

fn object(properties: &Value, required: &[&str], description: &str) -> Value {
    described(
        json!({"type": "object", "properties": properties, "required": required}),
        description,
    )
}

fn nullable(kind: &str, description: &str) -> Value {
    described(json!({"type": [kind, "null"]}), description)
}

/// Who wrote a memory and when: an author record's id, not a name.
fn stamp(description: &str) -> Value {
    object(
        &json!({
            "at": text("When, as an RFC 3339 timestamp."),
            "by": text("The id of the `hoard-author` memory naming who; read it with hoard_lookup for the name."),
        }),
        &["at", "by"],
        description,
    )
}

/// A link written outbound from a memory.
fn link() -> Value {
    object(
        &json!({
            "label": text(""),
            "target": {
                "anyOf": [
                    {"type": "string", "description": "A memory in the same set, by id."},
                    object(
                        &json!({
                            "id": text(""),
                            "set": text(""),
                            "set_id": text(""),
                            "keywords": strings(""),
                        }),
                        &["id", "set"],
                        "A memory in another set.",
                    ),
                ],
                "description": "An id for a memory in the same set, or an object naming another set.",
            },
        }),
        &["label", "target"],
        "",
    )
}

/// An edge arriving at a memory from another memory in the same set.
fn incoming_edge() -> Value {
    object(
        &json!({
            "label": text(""),
            "source": text(""),
        }),
        &["label", "source"],
        "",
    )
}

/// The memory as a client sees it, from recall or lookup.
fn memory_view() -> Value {
    object(
        &json!({
            "id": text(""),
            "scope": text("The set it came from: `local`, or `shared:<name>`. Absent on a summary-detail recall hit."),
            "summary": text("One line derived from the memory's opening. Present only on a summary-detail recall hit, which carries `id` and this and nothing else."),
            "content": text(""),
            "why": text(""),
            "context": text("A handle for placing it in time: what was being worked on. Present only when written."),
            "confidence": score("How far the memory's evidence reaches, 0.0 to 1.0."),
            "origin": text(""),
            "keywords": strings("Terms for exact-match search. Present only when it has any."),
            "links": list(&link(), "Edges written outbound from this memory. Present only when it has any."),
            "created": stamp(""),
            "modified": stamp(""),
            "superseded_by": strings("Ids of memories that replace this one; the last is the end of the chain. Present only on a superseded memory, which recall never returns and lookup does."),
            "incoming": list(&incoming_edge(), "Edges arriving from memories in the same set. Present only when there are any."),
            "referenced_from": list(&object(
                &json!({
                    "scope": text(""),
                    "source": text(""),
                    "label": text(""),
                }),
                &["scope", "source", "label"],
                "A link to this memory from a memory in another set this machine holds.",
            ), "Links to this memory from other sets this machine holds. Present only when there are any."),
            "disputed_by": list(&object(
                &json!({
                    "set": text(""),
                    "source": text(""),
                    "label": text(""),
                }),
                &["set", "source", "label"],
                "",
            ), "Claims another loaded set makes over this memory. Present only when there are any."),
            "cross_set_targets": list(&object(
                &json!({
                    "id": text(""),
                    "label": text(""),
                    "set": text(""),
                    "set_id": text(""),
                    "loaded": list(&object(
                        &json!({"scope": text(""), "mode": text("")}),
                        &["scope"],
                        "",
                    ), ""),
                }),
                &[],
                "",
            ), "Present only when a link names a set by identity and more than one loaded set answers to it."),
            "replacement_forgotten": flag("True when the memory that replaced this one has been forgotten, so this is superseded by something no longer readable. Present only then."),
            "supersession_cycle": flag("True when this memory's history is broken: two memories each claim to replace the other. Present only then."),
            "evaluation": flag("True when the memory came from a set loaded for review rather than adopted. Present only then."),
            "similarity": score("How closely the memory matched the query, on a semantic recall. Absent on a substituted hit, which carries `matched_similarity` instead."),
            "matched_similarity": score("On a semantic recall where a superseded memory matched and its replacement is returned instead: how closely the superseded one matched. Never appears beside `similarity`."),
            "replaces_matched": text("The id of the superseded memory the query actually matched, beside `matched_similarity`."),
            "inherited": score("Score the memory received through a link from one the query matched. Present only when the walk reached it."),
            "matched_keywords": count("How many of the query's keywords the memory carries, on a keyword recall. Present only on that path."),
        }),
        &["id"],
        "",
    )
}

fn recall() -> Value {
    object(
        &json!({
            "memories": list(&memory_view(), "The results, best first. Fewer than the limit when fewer clear the set's non-match floor."),
            "semantic_search_available": flag(""),
            "note": text("Something about this recall worth knowing, such as a floor that silenced a set. Present only then."),
            "skipped_sets": list(&object(
                &json!({"scope": text(""), "reason": text("")}),
                &["scope", "reason"],
                "",
            ), "Sets that could not be searched. Present only when there are any."),
            "answered_only_without_a_floor": strings("Sets whose floor silenced everything, which `ignore_floor` would ask again. Present only then."),
            "under_review_note": text("Present when results include memories from a set loaded for review."),
            "language_note": text("Present when a set searched by meaning declares a language its model serves poorly."),
        }),
        &["memories", "semantic_search_available"],
        "",
    )
}

fn lookup() -> Value {
    object(
        &json!({
            "memories": list(&memory_view(), "The memories found: one for a whole id, any number for a fragment."),
            "forgotten": list(&object(
                &json!({
                    "id": text(""),
                    "scope": text(""),
                    "forgotten": stamp(""),
                    "incoming": list(&incoming_edge(), ""),
                    "superseded_by": strings(""),
                }),
                &["id", "scope"],
                "A tombstone: a memory that was forgotten, which keeps its id but not its text.",
            ), "Forgotten memories the id or fragment matched. Present only when there are any."),
            "note": text("Something about this lookup worth knowing. Present only then."),
            "under_review_note": text("Present when a result came from a set loaded for review."),
        }),
        &["memories"],
        "",
    )
}

fn delta_target() -> Value {
    json!({"description": "A memory id, or an object naming a memory in another set."})
}

fn memorize() -> Value {
    object(
        &json!({
            "id": text(""),
            "summary": text(""),
            "findable_by_meaning": flag(""),
            "note": text("Something about this store worth knowing, such as why it is not findable by meaning. Present only then."),
            "author_note": text("Present only when this write settled who writes to the set from the human's own answer to a form you did not see: what they answered."),
            "keyword_delta": list(&object(
                &json!({
                    "superseded": text(""),
                    "added": strings(""),
                    "dropped": strings(""),
                    "retained": strings(""),
                }),
                &["superseded", "added", "dropped", "retained"],
                "",
            ), "Present only when this memory supersedes another."),
            "keyword_delta_note": text("What `keyword_delta` means for this store. Beside it."),
            "link_delta": list(&object(
                &json!({
                    "superseded": text(""),
                    "restated": list(&object(&json!({"label": text(""), "target": delta_target()}), &["label", "target"], ""), ""),
                    "left_behind": list(&object(&json!({"label": text(""), "target": delta_target()}), &["label", "target"], ""), ""),
                }),
                &["superseded", "restated", "left_behind"],
                "",
            ), "Present only when this memory supersedes another."),
            "link_delta_note": text("What `link_delta` means for this store. Beside it."),
            "repointed_in_links": list(&object(
                &json!({
                    "source": text(""),
                    "label": text(""),
                    "superseded": text(""),
                    "scope": text("The source's set, when it is another set."),
                }),
                &["source", "label", "superseded"],
                "",
            ), "Present only when other memories point at one this supersedes."),
            "repointed_in_links_note": text("What `repointed_in_links` means. Beside it."),
            "unresolved_links": list(&object(
                &json!({
                    "label": text(""),
                    "target": text(""),
                    "set": text(""),
                    "state": text(""),
                }),
                &["label", "target", "state"],
                "",
            ), "Present only when there are any."),
            "unresolved_links_note": text("What `unresolved_links` means. Beside it."),
        }),
        &["id", "summary", "findable_by_meaning"],
        "",
    )
}

fn correct() -> Value {
    let amend = object(
        &json!({
            "id": text(""),
            "amended": strings(""),
            "summary": text(""),
            "semantic_search_available": flag(""),
            "in_links": list(&incoming_edge(), "Links pointing at the memory, whose claims may need reviewing after a change to its content. Present only then."),
            "in_links_note": text("What `in_links` means. Beside it."),
            "unresolved_links": list(&object(
                &json!({"label": text(""), "target": text(""), "state": text("")}),
                &["label", "target", "state"],
                "",
            ), "Present only when there are any."),
            "unresolved_links_note": text("What `unresolved_links` means. Beside it."),
            "author_note": text("Present only when this write settled who writes to the set from the human's own answer to a form you did not see: what they answered."),
        }),
        &["id", "amended"],
        "",
    );
    let forget = object(
        &json!({
            "forgotten": text(""),
            "note": text("What forgetting kept: the id, as a tombstone."),
            "already_absent": flag("True when there was nothing left to forget. Present only then."),
            "author_note": text("Present only when this write settled who writes to the set from the human's own answer to a form you did not see: what they answered."),
        }),
        &["forgotten"],
        "",
    );
    // `"type": "object"` beside the `anyOf`, not only inside it: clients require an output
    // schema's top level to say `object` and refuse the whole tool list otherwise -- Claude's did,
    // on 2026-10-03, with this module's first draft.
    json!({
        "type": "object",
        "anyOf": [amend, forget],
        "description": "What a correction did: an `amend` or a `forget` result.",
    })
}

fn set_description() -> Value {
    json!({
        "type": ["object", "null"],
        "properties": {
            "clusters": list(&object(
                &json!({
                    "about": strings(""),
                    "memories": count(""),
                }),
                &["about", "memories"],
                "",
            ), ""),
            "derivation": text(""),
        },
        "description": "What the set amounts to as a whole, derived from its memories. Null when it could not be derived.",
    })
}

fn id_change(description: &str) -> Value {
    object(
        &json!({"count": count(""), "ids": strings("")}),
        &["count", "ids"],
        description,
    )
}

fn set_entry() -> Value {
    object(
        &json!({
            "scope": text(""),
            "mode": text("How this machine holds it: `read-write`, `read-only`, or a review copy."),
            "set_id": nullable("string", "The set's identity, or null before it has one."),
            "file": nullable("string", ""),
            "author": nullable("string", "The name this machine publishes under in the set, or null before one is minted."),
            "author_note": text("Present when the author needs attention."),
            "memories": count(""),
            "forgotten": count(""),
            "semantic_search_available": flag(""),
            "embedding_model": object(
                &json!({
                    "state": text(""),
                    "recorded": text(""),
                    "loaded": nullable("string", ""),
                    "note": text(""),
                }),
                &["state"],
                "",
            ),
            "non_match_floor": nullable("number", "The calibrated floor, or null when there is none."),
            "floor_source": nullable("string", ""),
            "labelled_since_calibration": count(""),
            "calibration_note": text("Present when calibration needs attention."),
            "languages": strings(""),
            "language_scores": described(
                json!({"type": "object", "additionalProperties": count("")}),
                "How well the loaded model represents each declared language, 0 to 10, keyed by tag. Present when the set can be searched by meaning and declares a language with built-in test sentences.",
            ),
            "language_note": text("Present when the languages need saying."),
            "recommended_models": list(
                &object(
                    &json!({
                        "id": text(""),
                        "summary": text("Its opening line, which names the model."),
                        "languages": strings("The languages it was measured on; empty where it concerns the whole set."),
                    }),
                    &["id", "summary", "languages"],
                    "",
                ),
                "Models this set's authors recommend for it, from `hoard-fit-model` memories linking to its hub. Present when there are any.",
            ),
            "watch_marks": strings(""),
        }),
        &["scope", "mode"],
        "",
    )
}

fn sets() -> Value {
    let mut shapes = every_other_set_action();
    if let Some(actions) = shapes["anyOf"].as_array_mut() {
        actions.extend(review_actions());
    }
    shapes
}

/// What `review` and `clear` return, kept apart from [`every_other_set_action`] so neither `json!`
/// reaches the macro's recursion limit.
fn review_actions() -> Vec<Value> {
    let sentence = object(
        &json!({"n": count(""), "field": text(""), "text": text("")}),
        &["n", "field", "text"],
        "",
    );
    let memory = object(
        &json!({"id": text(""), "sentences": list(&sentence, ""), "keywords": strings("")}),
        &["id", "sentences", "keywords"],
        "",
    );
    let hit = object(
        &json!({"shape": text(""), "matched": text("")}),
        &["shape", "matched"],
        "",
    );
    vec![
        object(
            &json!({
                "backed_up": text(""),
                "records": count(""),
                "backups_held": count(""),
                "note": text(""),
            }),
            &["backed_up", "records", "backups_held", "note"],
            "",
        ),
        object(
            &json!({
                "set": text(""),
                "kind": text(""),
                "audience": text(""),
                "to_review": count(""),
                "shown": count(""),
                "task": text(""),
                "preferences": text("On a review's first page in a session, and when they changed."),
                "others_keep_out": strings("What applies to a sentence about someone else. With `preferences`."),
                "project_keep_out": strings(""),
                "published_as": text("The name this set publishes the person under, which mentions nothing. With `preferences`."),
                "pre_pass": list(&object(&json!({"id": text(""), "hits": list(&hit, "")}), &["id", "hits"], ""), ""),
                "groups": list(&object(
                    &json!({"about": strings(""), "memories": list(&memory, ""), "context": list(&memory, "Already reviewed, shown for context only.")}),
                    &["about", "memories"],
                    "",
                ), ""),
                "awaiting_the_person": strings(""),
                "preferences_changed": object(
                    &json!({
                        "tightened": strings(""),
                        "exceptions_removed": strings(""),
                        "groups": list(&object(&json!({"group": text(""), "about": strings(""), "memories": count("")}), &["group", "about", "memories"], ""), ""),
                        "ask": text(""),
                        "reopened": strings(""),
                    }),
                    &[],
                    "Present when the preferences changed since the last review.",
                ),
                "caution": text(""),
            }),
            &[
                "set",
                "kind",
                "audience",
                "to_review",
                "shown",
                "task",
                "groups",
                "caution",
            ],
            "",
        ),
        object(
            &json!({
                "cleared": strings(""),
                "flagged": list(&object(
                    &json!({"id": text(""), "reasons": strings(""), "content": text(""), "why": nullable("string", ""), "context": nullable("string", "")}),
                    &["id", "reasons", "content", "why", "context"],
                    "",
                ), ""),
                "refused": list(&object(&json!({"id": text(""), "why": text("")}), &["id", "why"], ""), ""),
                "remaining": count(""),
                "awaiting_the_person": count(""),
                "finished": flag(""),
                "not_finished": text(""),
                "flag_guidance": text(""),
            }),
            &[
                "cleared",
                "flagged",
                "remaining",
                "awaiting_the_person",
                "finished",
            ],
            "",
        ),
    ]
}

/// Every `hoard_sets` action's shape but the review's.
fn every_other_set_action() -> Value {
    let action =
        |properties: Value, required: &[&str], what: &str| object(&properties, required, what);
    let safety = || {
        nullable(
            "string",
            "Where the set's previous contents were written first, or null when none was written.",
        )
    };
    json!({
        "type": "object",
        "anyOf": [
            action(json!({
                "sets": list(&set_entry(), ""),
                "withheld": list(&object(
                    &json!({"scope": text(""), "file": text(""), "set_id": text(""), "memories": count(""), "note": text("")}),
                    &["scope", "note"],
                    "",
                ), "Present only when there are any."),
                "shadowed": list(&object(
                    &json!({"scope": text(""), "file": text(""), "note": text("")}),
                    &["scope", "note"],
                    "A store file another set's name now reaches.",
                ), "Present only when there are any."),
                "configuration_note": text("Present only when the configuration needs attention."),
            }), &["sets"], ""),
            action(json!({
                "created": text(""), "mode": text(""),
                "memories": count(""), "note": text(""),
            }), &["created"], ""),
            action(json!({
                "written": text(""), "path_form": text(""),
                "memories": count(""), "forgotten": count(""),
                "identities": count(""), "records": count(""),
                "author": nullable("string", ""),
                "describes": set_description(),
                "replaced_file": object(
                    &json!({
                        "state": text(""),
                        "new": id_change("Records the file did not have."),
                        "changed": id_change("Records that changed."),
                        "removed": id_change("Records the file had and this export does not."),
                        "unchanged": count(""),
                        "note": text("Present when there was nothing to compare against."),
                    }),
                    &["state"],
                    "",
                ),
                "prohibited_data_scanned": flag(""),
                "prohibited_data_note": text(""),
                "prohibited_data_found": list(&object(
                    &json!({"memory": text(""), "category": text("")}),
                    &["memory", "category"],
                    "",
                ), "Present only when something was found."),
                "prohibited_data_guidance": text("What to do about it. Beside it."),
                "note": text("Present only when something about the export needs saying."),
            }), &["written"], ""),
            action(json!({
                "read": text(""), "path_form": text(""),
                "adopted_set_id": nullable("string", ""),
                "describes": set_description(),
                "outcomes": object(
                    &json!({
                        "new_from_file": count(""),
                        "unchanged": count(""),
                        "took_from_file": count("Records changed only in the file."),
                        "kept_local": count("Records changed only here."),
                        "converged": count(""),
                        "conflict": count(""),
                        "anomaly": count(""),
                        "kept_forgotten": count(""),
                    }),
                    &[],
                    "How many records fell into each outcome. An outcome nothing fell into is absent.",
                ),
                "conflicts": strings("Ids changed on both sides. Present only when there are any."),
                "conflict_guidance": text("What to do. Beside it."),
                "anomalies": strings("Present only when there are any."), "anomaly_guidance": text("Beside it."),
                "kept_forgotten": strings("Ids forgotten here that the file still holds. Present only when there are any."),
                "kept_forgotten_guidance": text("Beside it."),
                "taken_backwards": strings("Present only when there are any."), "taken_backwards_guidance": text("Beside it."),
                "supersession_cycles": strings("Present only when there are any."), "supersession_cycle_guidance": text("Beside it."),
                "description_owed": text("Present when the set could not be described at import."),
                "identity_note": text("Present when the set's identity needs saying."),
                "note": text("Present only when something about the import needs saying."),
            }), &["read", "outcomes"], ""),
            action(json!({
                "minted": text(""), "author": text(""),
                "scope": text(""), "note": text(""),
            }), &["minted"], ""),
            action(json!({
                "identified": text(""), "author": text(""),
                "scope": text(""), "note": text(""),
            }), &["identified"], ""),
            action(json!({
                "record": text(""), "superseded": text(""),
                "author": text(""), "was": text(""),
                "scope": text(""), "note": text(""),
            }), &["record"], ""),
            action(json!({
                "scope": text(""), "model": text(""),
                "vectors_rebuilt": count(""),
                "unrepresentable": count(""),
                "displaced": nullable("string", ""),
                "warning": text("Present when the rebuild needs a warning."),
            }), &["vectors_rebuilt"], ""),
            action(json!({
                "scope": text(""), "floor": nullable("number", "The new non-match floor, or null when there was not enough to set one."),
                "previous_floor": nullable("number", ""),
                "floor_source": text(""),
                "labelled_queries": count(""),
                "memories_asked": count(""),
                "note": text(""),
            }), &["floor"], ""),
            action(json!({
                "scope": text(""), "cleared": nullable("number", "The floor removed, or null if there was none."),
                "note": text(""),
            }), &["cleared"], ""),
            action(json!({
                "loaded": text(""), "copy_of": text(""),
                "set_id": text(""), "held_at": text(""), "read": text(""),
                "memories": count(""), "forgotten": count(""),
                "took_ms": count(""), "note": text(""),
            }), &["loaded"], ""),
            action(json!({
                "restored": text(""), "read": text(""), "path_form": text(""),
                "memories": count(""), "forgotten": count(""),
                "safety_copy": safety(), "no_safety_copy": text(""),
                "discarded_local_divergence": text(""),
            }), &["restored"], ""),
            action(json!({
                "absorbed": text(""), "into": text(""),
                "read": text(""), "path_form": text(""),
                "memories": count(""), "already_held": count(""),
                "forgotten": count(""),
                "superseded": list(&object(&json!({"memory": text(""), "superseded_by": text("")}), &["memory", "superseded_by"], ""), ""),
                "not_internalised": list(&object(&json!({"memory": text(""), "target": text("")}), &["memory", "target"], ""), "Present only when there are any."),
                "not_internalised_note": text("Beside it."),
                "still_loaded": text("Present when the absorbed set is still loaded, so its memories are in two sets."),
                "safety_copy": safety(), "note": text(""),
            }), &["absorbed", "into"], ""),
            action(json!({
                "dropped": text(""), "database_removed": text(""),
                "memories": count(""),
                "safety_copy": safety(), "no_safety_copy": text(""),
                "safety_copy_skipped": text("Why writing one was skipped, for a read-only set."),
            }), &["dropped"], ""),
        ],
        "description": "What the action did; each action returns its own shape.",
    })
}
