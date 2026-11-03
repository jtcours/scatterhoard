// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Run the export review on a set before a test exports it, standing in for the session and the
//! person: every sentence answered as about the project and mentioning nothing, and anything the
//! server flags cleared as the person would.
//!
//! **For tests about export itself**, which predate the review and are not about it. Since
//! 2026-10-24 export refuses a shared set's memories until they are reviewed
//! (`documents/design/export-review.md`); `tests/export_review.rs` tests that refusal and the
//! review, and these tests keep testing what they were written for.

use scatterhoard::mcp::Server;
use serde_json::{Value, json};

fn sets(server: &mut Server, arguments: Value) -> Value {
    let raw = server
        .handle(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": "hoard_sets", "arguments": arguments}})
            .to_string(),
        )
        .expect("answered");
    let response: Value = serde_json::from_str(&raw).expect("JSON");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if response["result"]["isError"] == json!(true) || response.get("error").is_some() {
        return Value::Null;
    }
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

/// Review and clear everything an export of `arguments` would refuse. Does nothing for an export
/// the review does not gate, and nothing a review refuses, so a test about refusals still sees its
/// own.
pub fn review_before_export(server: &mut Server, arguments: &Value) {
    if arguments["action"] != json!("export") || arguments["scope"] == json!("local") {
        return;
    }
    let mut request = json!({"action": "review", "scope": arguments["scope"]});
    for key in ["directory", "file"] {
        if !arguments[key].is_null() {
            request[key] = arguments[key].clone();
        }
    }
    for _ in 0..100 {
        let review = sets(server, request.clone());
        let memories: Vec<Value> = review["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|group| group["memories"].as_array().cloned().unwrap_or_default())
            .collect();
        if memories.is_empty() {
            return;
        }
        let answers: Vec<Value> = memories
            .iter()
            .map(|memory| {
                let count = memory["sentences"].as_array().map_or(0, Vec::len);
                json!({
                    "id": memory["id"],
                    "sentences": (1..=count)
                        .map(|n| json!({"n": n, "about": "project", "mentions": []}))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        let cleared = sets(
            server,
            json!({"action": "clear", "scope": arguments["scope"], "answers": answers}),
        );
        let flagged: Vec<Value> = cleared["flagged"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|flag| flag["id"].clone())
            .collect();
        if !flagged.is_empty() {
            sets(
                server,
                json!({"action": "clear", "scope": arguments["scope"], "person_cleared": flagged}),
            );
        }
    }
}
