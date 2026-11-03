//! The prompt outbox: drafts an agent wrote for the user to paste into another tool. comms holds
//! them and answers on the clip socket; the Today page shows them and Copy takes one out.

use serde_json::{json, Value};

/// One queued draft.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub id: i64,
    pub title: String,
    pub body: String,
}

fn parse(draft: &Value) -> Option<Draft> {
    Some(Draft {
        id: draft.get("id")?.as_i64()?,
        title: draft.get("title")?.as_str()?.to_string(),
        body: draft.get("body")?.as_str()?.to_string(),
    })
}

/// Every queued draft, oldest first.
///
/// # Errors
/// comms is not answering, or no overlay is configured.
pub fn drafts() -> Result<Vec<Draft>, String> {
    let answer = crate::clipper::ask(json!({ "type": "drafts" }))?;
    Ok(answer["drafts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(parse)
        .collect())
}

/// Remove a draft from the outbox after its text is on the clipboard.
///
/// # Errors
/// comms is not answering, or the draft was already taken.
pub fn take(id: i64) -> Result<(), String> {
    crate::clipper::ask(json!({ "type": "draft_take", "id": id })).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_without_a_field_is_skipped() {
        assert_eq!(
            parse(&json!({"id": 3, "title": "Watch", "body": "Compare"})),
            Some(Draft {
                id: 3,
                title: "Watch".into(),
                body: "Compare".into()
            })
        );
        assert_eq!(parse(&json!({"id": 3, "title": "Watch"})), None);
    }
}
