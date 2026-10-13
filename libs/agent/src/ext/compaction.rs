//! Context compaction (F5): summarizes older complete turns when estimated tokens
//! exceed the role's `max_input_tokens` (AGT-17, AGT-18).
//!
//! A turn is never split between its tool calls and their results (AGT-17).
//! The session file records the summary, so `--continue` sends the summary (AGT-18).

use sjel_agent::{Extension, Message};

/// The name `agent.toml` and `--ext` use.
pub const NAME: &str = "compaction";

/// Default input token ceiling if the inference role specifies none.
pub const DEFAULT_MAX_INPUT_TOKENS: usize = 16_384;

pub struct Compaction {
    limit: usize,
}

impl Compaction {
    pub fn new(max_input_tokens: Option<u32>) -> Self {
        let limit = max_input_tokens
            .map(|t| t as usize)
            .unwrap_or(DEFAULT_MAX_INPUT_TOKENS);
        Self { limit }
    }

    #[allow(dead_code)]
    pub fn with_limit(limit: usize) -> Self {
        Self { limit }
    }
}

/// Conservative token estimation for a slice of messages (~4 chars per token + framing).
pub fn estimate_tokens(messages: &[Message]) -> usize {
    let mut total = 0;
    for m in messages {
        total += 4; // per-message framing overhead
        match m {
            Message::System { content } => total += content.len().div_ceil(4),
            Message::User { content } => total += content.len().div_ceil(4),
            Message::Assistant {
                content,
                tool_calls,
            } => {
                if let Some(c) = content {
                    total += c.len().div_ceil(4);
                }
                if let Some(calls) = tool_calls {
                    for call in calls {
                        total +=
                            (call.function.name.len() + call.function.arguments.len() + 10).div_ceil(4);
                    }
                }
            }
            Message::Tool { content, .. } => total += content.len().div_ceil(4),
        }
    }
    total
}

/// Identifies range boundaries for complete turns in `messages`.
/// Skips initial `Message::System` (index 0).
/// A complete turn starts at `Message::User` and includes all follow-up tool calls,
/// tool results, and final assistant replies up until the next `Message::User` or end.
/// Guarantees that a tool call and its tool results are never split across turn boundaries.
pub fn find_complete_turns(messages: &[Message]) -> Vec<std::ops::Range<usize>> {
    let start_idx = if matches!(messages.first(), Some(Message::System { .. })) {
        1
    } else {
        0
    };

    let mut user_indices = Vec::new();
    for (i, m) in messages.iter().enumerate().skip(start_idx) {
        if matches!(m, Message::User { .. }) {
            user_indices.push(i);
        }
    }

    let mut turns = Vec::new();
    for i in 0..user_indices.len() {
        let start = user_indices[i];
        let end = if i + 1 < user_indices.len() {
            user_indices[i + 1]
        } else {
            messages.len()
        };
        turns.push(start..end);
    }
    turns
}

/// Condenses a slice of messages from one or more turns into a concise summary string.
pub fn summarize_turns(turns: &[Message]) -> String {
    let mut summary = Vec::new();
    let mut user_prompt = String::new();
    let mut actions = Vec::new();
    let mut conclusion = String::new();

    for m in turns {
        match m {
            Message::User { content } => {
                if !user_prompt.is_empty() {
                    summary.push(format_turn_summary(&user_prompt, &actions, &conclusion));
                    actions.clear();
                    conclusion.clear();
                }
                user_prompt = content.clone();
            }
            Message::Assistant {
                content,
                tool_calls,
            } => {
                if let Some(calls) = tool_calls {
                    for call in calls {
                        actions.push(format!(
                            "{}({})",
                            call.function.name,
                            truncate_args(&call.function.arguments)
                        ));
                    }
                }
                if let Some(c) = content {
                    if !c.trim().is_empty() {
                        conclusion = c.clone();
                    }
                }
            }
            Message::Tool { .. } => {}
            Message::System { .. } => {}
        }
    }

    if !user_prompt.is_empty() {
        summary.push(format_turn_summary(&user_prompt, &actions, &conclusion));
    }

    summary.join("\n---\n")
}

fn truncate_args(args: &str) -> String {
    let trimmed = args.trim();
    if trimmed.len() > 60 {
        format!("{}...", &trimmed[..57])
    } else {
        trimmed.to_string()
    }
}

fn format_turn_summary(user: &str, actions: &[String], conclusion: &str) -> String {
    let mut out = format!("User asked: {}\n", user.trim());
    if !actions.is_empty() {
        out.push_str(&format!("Actions taken: {}\n", actions.join(", ")));
    }
    if !conclusion.is_empty() {
        out.push_str(&format!("Result: {}\n", conclusion.trim()));
    }
    out
}

impl Extension for Compaction {
    fn name(&self) -> &'static str {
        NAME
    }

    fn before_request(&self, messages: &mut Vec<Message>) {
        if estimate_tokens(messages) <= self.limit {
            return;
        }

        let turns = find_complete_turns(messages);
        if turns.len() < 2 {
            // Must keep at least the active current turn
            return;
        }

        // We can compact up to (turns.len() - 1) older complete turns into one summary message.
        let max_compact = turns.len() - 1;
        let mut count = 1;
        while count < max_compact {
            let start = turns[0].start;
            let end = turns[count - 1].end;
            let summary_text = summarize_turns(&messages[start..end]);
            let summary_tokens = summary_text.len().div_ceil(4) + 4;
            let turns_tokens = estimate_tokens(&messages[start..end]);
            let current_total = estimate_tokens(messages);
            let estimated_after = current_total.saturating_sub(turns_tokens).saturating_add(summary_tokens);
            if estimated_after <= self.limit {
                break;
            }
            count += 1;
        }

        let start = turns[0].start;
        let end = turns[count - 1].end;
        let summary_text = summarize_turns(&messages[start..end]);
        let summary_msg = Message::User {
            content: format!("[Summary of earlier conversation:\n{summary_text}]"),
        };

        messages.splice(start..end, std::iter::once(summary_msg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_agent::{FunctionCall, ToolCall};

    fn make_turn(user: &str, tool: &str, result: &str, answer: &str) -> Vec<Message> {
        vec![
            Message::User {
                content: user.into(),
            },
            Message::Assistant {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".into(),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: tool.into(),
                        arguments: r#"{"param":"val"}"#.into(),
                    },
                }]),
            },
            Message::Tool {
                tool_call_id: "call_1".into(),
                content: result.into(),
            },
            Message::Assistant {
                content: Some(answer.into()),
                tool_calls: None,
            },
        ]
    }

    #[test]
    fn compacts_oldest_turn_when_exceeding_token_limit_without_splitting_tool_results() {
        let mut messages = vec![Message::System {
            content: "You are a coding assistant.".into(),
        }];

        // Turn 1
        messages.extend(make_turn(
            "Fix test 1",
            "read",
            &"error line: mismatched types in test.rs\n".repeat(20),
            "I fixed test 1",
        ));
        // Turn 2
        messages.extend(make_turn(
            "Fix test 2",
            "edit",
            &"edited 5 lines successfully\n".repeat(10),
            "I fixed test 2",
        ));

        let initial_tokens = estimate_tokens(&messages);

        // Set a limit lower than initial tokens, but large enough for 1 turn
        let compactor = Compaction::with_limit(initial_tokens - 50);
        compactor.before_request(&mut messages);

        // AGT-17: The oldest turn is compacted into one summary message.
        // There must be no orphaned Tool messages without an assistant call!
        assert!(
            messages.len() < 9,
            "messages must be reduced, was {}",
            messages.len()
        );
        assert!(estimate_tokens(&messages) < initial_tokens);

        // Verify the summary message is present
        let first_user = messages
            .iter()
            .find(|m| matches!(m, Message::User { .. }))
            .unwrap();
        if let Message::User { content } = first_user {
            assert!(
                content.contains("Summary of earlier conversation"),
                "{content}"
            );
            assert!(content.contains("Fix test 1"), "{content}");
        }

        // AGT-17 invariant: Verify no tool result exists whose tool call was compacted away
        let mut in_flight_calls = Vec::new();
        for m in &messages {
            match m {
                Message::Assistant {
                    tool_calls: Some(calls),
                    ..
                } => {
                    for c in calls {
                        in_flight_calls.push(c.id.clone());
                    }
                }
                Message::Tool { tool_call_id, .. } => {
                    assert!(
                        in_flight_calls.contains(tool_call_id),
                        "orphaned tool result found for {tool_call_id}"
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn single_turn_is_never_compacted_away() {
        let mut messages = vec![
            Message::System {
                content: "sys".into(),
            },
            Message::User {
                content: "a very long prompt that exceeds the threshold by itself".into(),
            },
        ];

        let compactor = Compaction::with_limit(10);
        compactor.before_request(&mut messages);

        // Does not drop the only user prompt
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn session_file_records_summary_and_resumes_compacted_state() {
        let dir = std::env::temp_dir()
            .join(format!("sjel-agent-compaction-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = crate::session::new_path(&dir);

        let mut messages = vec![Message::System {
            content: "You are a coding assistant.".into(),
        }];
        messages.extend(make_turn(
            "Fix test 1",
            "read",
            &"lots of error output in test.rs\n".repeat(20),
            "I fixed test 1",
        ));
        messages.extend(make_turn(
            "Fix test 2",
            "edit",
            &"edited 5 lines successfully\n".repeat(10),
            "I fixed test 2",
        ));

        // Initial save to session
        crate::session::append(&path, &messages[1..]).unwrap();
        let uncompacted_file_size = std::fs::metadata(&path).unwrap().len();

        // Run compaction
        let initial_tokens = estimate_tokens(&messages);
        let compactor = Compaction::with_limit(initial_tokens - 50);
        compactor.before_request(&mut messages);

        // Overwrite session after compaction (AGT-18)
        crate::session::overwrite(&path, &messages).unwrap();

        // Resume session
        let loaded = crate::session::load(&path).unwrap();
        let compacted_tokens = estimate_tokens(&loaded);

        assert!(
            compacted_tokens < initial_tokens,
            "resumed conversation must be smaller than original uncompacted conversation"
        );
        let compacted_file_size = std::fs::metadata(&path).unwrap().len();
        assert!(
            compacted_file_size < uncompacted_file_size,
            "compacted file size ({compacted_file_size}) must be smaller than uncompacted ({uncompacted_file_size})"
        );
    }
}
