//! Subagents extension (Wave 2): delegation to isolated child agent loops.
//!
//! Exposes the `subagent` tool to run bounded delegation tasks.
//! The subagent runs on its own Agent loop with isolated message history,
//! its own prompt, and returns the final synthesized response to the parent agent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::tools::coding;
use crate::{Agent, AgentError, Event, Extension, Message, Tool};
use serde_json::{json, Value};
use sjel_inference::InferenceConfig;

/// The name `agent.toml` and `--ext` use.
pub const NAME: &str = "subagents";

const DEFAULT_MAX_TURNS: usize = 10;
const MAX_ALLOWED_TURNS: usize = 20;

pub struct Subagents {
    stop: Arc<AtomicBool>,
    default_role: String,
}

impl Subagents {
    pub fn new(stop: &Arc<AtomicBool>, default_role: Option<&str>) -> Self {
        Self {
            stop: Arc::clone(stop),
            default_role: default_role.unwrap_or("coding").to_string(),
        }
    }
}

impl Extension for Subagents {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(SubagentTool {
            stop: Arc::clone(&self.stop),
            default_role: self.default_role.clone(),
        })]
    }
}

pub struct SubagentTool {
    stop: Arc<AtomicBool>,
    default_role: String,
}

impl Tool for SubagentTool {
    fn name(&self) -> &str {
        "subagent"
    }

    fn description(&self) -> &str {
        "Spawns an isolated subagent to perform a delegated task and returns the result."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Clear instruction for the subagent to perform"
                },
                "role": {
                    "type": "string",
                    "description": "Optional inference role (defaults to parent agent role)"
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Maximum tool turns (default 10, max 20)"
                }
            },
            "required": ["prompt"]
        })
    }

    fn run(&self, args: &Value) -> Result<String, String> {
        if self.stop.load(Ordering::Relaxed) {
            return Err("stopped by the user".to_string());
        }

        let prompt = match args.get("prompt").and_then(|p| p.as_str()) {
            Some(p) if !p.trim().is_empty() => p.to_string(),
            _ => return Err("missing required 'prompt' argument".to_string()),
        };

        let role_name = args
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or(&self.default_role);

        let max_turns = args
            .get("max_turns")
            .and_then(|t| t.as_u64())
            .map(|t| (t as usize).clamp(1, MAX_ALLOWED_TURNS))
            .unwrap_or(DEFAULT_MAX_TURNS);

        let role = InferenceConfig::load(sjel_config::overlay_config)
            .role(role_name)
            .ok_or_else(|| format!("cannot resolve inference role '{role_name}'"))?;

        let current_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let tools = coding(&current_dir, &self.stop);

        let mut child_agent = Agent::new(role, tools, vec![], Arc::clone(&self.stop))
            .map_err(|e| format!("cannot initialize subagent: {e}"))?;
        child_agent.max_turns = max_turns;

        let mut messages = vec![
            Message::System {
                content: "You are a focused subagent. Complete the assigned task concisely. Return the final finding or output directly.".to_string(),
            },
            Message::User { content: prompt },
        ];

        let mut final_text = String::new();
        let run_result = child_agent.run(&mut messages, |event| {
            if let Event::Text(fragment) = event {
                final_text.push_str(fragment);
            }
        });

        match run_result {
            Ok(()) => {
                if final_text.trim().is_empty() {
                    if let Some(Message::Assistant {
                        content: Some(c), ..
                    }) = messages.last()
                    {
                        return Ok(c.clone());
                    }
                    Ok("subagent finished with no output".to_string())
                } else {
                    Ok(final_text)
                }
            }
            Err(AgentError::Stopped) => Err("subagent stopped by user".to_string()),
            Err(e) => Err(format!("subagent error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagents_extension_properties() {
        let stop = Arc::new(AtomicBool::new(false));
        let ext = Subagents::new(&stop, Some("coding"));
        assert_eq!(ext.name(), "subagents");
        let tools = ext.tools();
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        assert_eq!(tool.name(), "subagent");
        assert!(!tool.description().is_empty());

        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["prompt"].is_object());
        assert!(params["properties"]["role"].is_object());
        assert!(params["properties"]["max_turns"].is_object());
    }

    #[test]
    fn subagent_tool_validates_prompt_and_stop() {
        let stop = Arc::new(AtomicBool::new(false));
        let tool = SubagentTool {
            stop: Arc::clone(&stop),
            default_role: "coding".into(),
        };

        // Missing prompt
        let err = tool.run(&json!({})).unwrap_err();
        assert!(err.contains("missing required 'prompt' argument"));

        // User stopped
        stop.store(true, Ordering::Relaxed);
        let err = tool.run(&json!({"prompt": "do something"})).unwrap_err();
        assert!(err.contains("stopped by the user"));
    }
}
