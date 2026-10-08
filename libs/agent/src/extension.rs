//! The extension contract: the one core change wave 1 needs (D1, D2).
//!
//! An official extension is a first-party Rust crate compiled into the binary, doing nothing
//! until config or a flag names it. These four hooks are the whole contract, because wave 1
//! needs exactly these and nothing else: tools (skills, MCP, diagnostics), the system prompt
//! (skills), request context (compaction) and the tool-call gate (guard). pi's event bus,
//! renderers and commands are not copied.
//!
//! A hook that does not exist here is a core change, and a core change is a decision in
//! `ISA.md` first.

use crate::{Message, Tool, ToolCall};

/// What an extension decides about one tool call, before the call runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// The call does not run. The text becomes the tool result, verbatim, so it is the
    /// extension's answer to the model and should say what was blocked and what to do instead.
    Deny(String),
}

/// A first-party extension, named by `<overlay>/config/agent.toml` or a `--ext` flag.
pub trait Extension: Send + Sync {
    /// The name config and flags use. Distinct from every other extension's.
    fn name(&self) -> &'static str;

    /// Tools added to the core set. Called once, when the agent is built.
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        Vec::new()
    }

    /// Appended to the system prompt once per run, before the first request.
    fn system(&self, _prompt: &mut String) {}

    /// Runs before every completion, and may rewrite the conversation. Compaction is the
    /// reason it takes `&mut`: it replaces old turns with one summary message.
    fn before_request(&self, _messages: &mut Vec<Message>) {}

    /// Runs before every tool call, after the call is known to name a tool and before its
    /// arguments are parsed — a guard blocks the call the model asked for, and a call whose
    /// arguments are unparseable is still a call it can name.
    fn tool_call(&self, _call: &ToolCall) -> Verdict {
        Verdict::Allow
    }
}

/// The extensions a run names, in the order they run: the config list first, then the `--ext`
/// flags in the order they were given.
///
/// Three flag forms and no fourth (AGT-8). `none` clears the set, `+name` adds one, `-name`
/// removes one. A bare name is an error rather than a guess about which of the two it meant.
/// Names are not checked against what the binary carries here — that is the front end's table
/// to check, because that is where the crates are linked.
pub fn select(config: &[String], flags: &[String]) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = config.to_vec();
    for flag in flags {
        match flag.as_str() {
            "none" => names.clear(),
            _ => {
                let (add, name) = match (flag.strip_prefix('+'), flag.strip_prefix('-')) {
                    (Some(rest), _) if !rest.is_empty() => (true, rest),
                    (_, Some(rest)) if !rest.is_empty() => (false, rest),
                    _ => {
                        return Err(format!(
                            "`--ext {flag}`: use `none` to clear the set, `+name` to add one, \
                             or `-name` to remove one"
                        ))
                    }
                };
                if add && !names.iter().any(|n| n == name) {
                    names.push(name.to_owned());
                } else if !add {
                    names.retain(|n| n != name);
                }
            }
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_flags_change_the_configured_set() {
        let config = names(&["a", "b"]);
        let flags = |list: &[&str]| select(&config, &names(list));
        assert_eq!(flags(&[]).unwrap(), names(&["a", "b"]), "config alone");
        assert_eq!(flags(&["+c"]).unwrap(), names(&["a", "b", "c"]));
        assert_eq!(flags(&["-a"]).unwrap(), names(&["b"]));
        assert_eq!(flags(&["none"]).unwrap(), names(&[]));
        assert_eq!(flags(&["none", "+c"]).unwrap(), names(&["c"]));
        assert_eq!(
            flags(&["+a"]).unwrap(),
            names(&["a", "b"]),
            "adding twice is one"
        );
        assert_eq!(flags(&["-b", "-a"]).unwrap(), names(&[]));
        assert_eq!(
            flags(&["none", "-a"]).unwrap(),
            names(&[]),
            "removing nothing is fine"
        );
        for bad in ["a", "typo", "+", "-", ""] {
            assert!(flags(&[bad]).is_err(), "`--ext {bad}` was accepted");
        }
    }
}
