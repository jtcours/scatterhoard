// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Check a tool response against the output schema its tool publishes, strictly.
//!
//! Included by path from each suite whose call helper checks responses
//! (`#[path = "support/output_schema_check.rs"] mod output_schema_check;`), so every tool response
//! those suites provoke is checked, not just a snapshot of each.
//!
//! **Strict where the published schema is lenient**: a field the schema does not declare fails
//! here, though a client validating against the published schema would accept it. That is the
//! guard against `response-shape-is-unpublished` happening again -- a new field fails a test rather
//! than arriving undocumented. A schema that lists no `properties` is opaque and not looked inside.
//!
//! **A small checker rather than a dependency**, because the schemas use seven keywords. It refuses
//! any other, so a schema can never use something this silently ignores. `additionalProperties` is
//! the seventh, for a map keyed by data rather than by name -- `language_scores`, keyed by language
//! tag -- and it checks every value, where an object listing no `properties` would check none.

use serde_json::{Value, json};

const KNOWN: [&str; 7] = [
    "type",
    "properties",
    "additionalProperties",
    "required",
    "items",
    "anyOf",
    "description",
];

/// Check one tool response: the `structuredContent` against the tool's schema, and the text copy
/// against the structured one. Text-only results, refusals, and tools without a schema pass.
pub fn check_response(tool: &str, response: &Value) {
    let result = &response["result"];
    if result["isError"] == json!(true) {
        return;
    }
    let Some(schema) = scatterhoard::output_schema::for_tool(tool) else {
        assert!(
            result.get("structuredContent").is_none(),
            "{tool} publishes no output schema, so it must return no structured content: {response}"
        );
        return;
    };
    let structured = result.get("structuredContent").unwrap_or_else(|| {
        panic!(
            "{tool} publishes an output schema, so it must return structured content: {response}"
        )
    });
    let text: Value = result["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(Value::Null);
    assert_eq!(
        &text, structured,
        "{tool}'s text copy must be the structured content"
    );
    if let Err(problem) = check(&schema, structured, "$") {
        panic!(
            "{tool}'s response does not match its published output schema: {problem}\nresponse: {structured}"
        );
    }
}

/// `Ok` when `instance` matches `schema` strictly; otherwise the first problem, with its path.
pub fn check(schema: &Value, instance: &Value, path: &str) -> Result<(), String> {
    let map = schema
        .as_object()
        .ok_or_else(|| format!("{path}: a schema must be an object"))?;
    if let Some(unknown) = map.keys().find(|key| !KNOWN.contains(&key.as_str())) {
        return Err(format!(
            "{path}: schema keyword `{unknown}` is not one this checker understands"
        ));
    }
    if let Some(branches) = map.get("anyOf") {
        let branches = branches
            .as_array()
            .ok_or_else(|| format!("{path}: anyOf must be a list"))?;
        let mut problems = Vec::new();
        for branch in branches {
            match check(branch, instance, path) {
                Ok(()) => return Ok(()),
                Err(problem) => problems.push(problem),
            }
        }
        return Err(format!(
            "{path}: matched no branch of anyOf: {}",
            problems.join(" | ")
        ));
    }
    if let Some(kind) = map.get("type") {
        let kinds: Vec<&str> = match kind {
            Value::String(one) => vec![one.as_str()],
            Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
            _ => return Err(format!("{path}: `type` must be a string or a list")),
        };
        if !kinds.iter().any(|kind| is_kind(instance, kind)) {
            return Err(format!("{path}: expected {kinds:?}, got {instance}"));
        }
    }
    if let (Some(properties), Value::Object(fields)) = (map.get("properties"), instance) {
        let properties = properties
            .as_object()
            .ok_or_else(|| format!("{path}: properties must be an object"))?;
        for (name, value) in fields {
            let declared = properties
                .get(name)
                .ok_or_else(|| format!("{path}.{name}: not declared in the schema"))?;
            check(declared, value, &format!("{path}.{name}"))?;
        }
        for required in map
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let required = required.as_str().unwrap_or_default();
            if !fields.contains_key(required) {
                return Err(format!("{path}.{required}: required, and absent"));
            }
        }
    }
    if let (Some(values), Value::Object(fields)) = (map.get("additionalProperties"), instance) {
        for (name, value) in fields {
            check(values, value, &format!("{path}.{name}"))?;
        }
    }
    if let (Some(items), Value::Array(elements)) = (map.get("items"), instance) {
        for (index, element) in elements.iter().enumerate() {
            check(items, element, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn is_kind(instance: &Value, kind: &str) -> bool {
    match kind {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "boolean" => instance.is_boolean(),
        "null" => instance.is_null(),
        "integer" => instance.is_i64() || instance.is_u64(),
        "number" => instance.is_number(),
        _ => false,
    }
}
