//! The agent run as the finder draws it: prompts, tool rows, and the call waiting for an answer.
//!
//! A run is not a request. It lives in the capability and emits events, so this holds what arrived
//! and nothing else — no promise, no timer, no conversation of its own. Close the panel and the
//! run keeps going; reopen it and the stream resumes from the last sequence seen.
//!
//! Drawing lives in `finder.rs`, because every approval button is a listener on that view.

use sjel_gpui_core::agent::{Event, Session};

/// One line of the transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// What was asked.
    Prompt(String),
    /// What came back, appended to as it streams.
    Answer(String),
    /// A tool call, its arguments, and its result once there is one.
    Tool {
        name: String,
        call_id: String,
        arguments: String,
        result: Option<String>,
    },
    /// An approval that was answered, in one line.
    Decided {
        name: String,
        allowed: bool,
        for_session: bool,
    },
    /// A reason the run stopped, or a fact worth a line: usage, a stop, a truncated stream.
    Notice(String),
}

/// The call the run is waiting on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub approval: String,
    pub name: String,
    /// The one-line summary: a command, or `path +3 −1`.
    pub preview: String,
    /// The lines that will change, when the call changes a file.
    pub diff: Option<String>,
}

#[derive(Default)]
pub struct AgentRun {
    /// The session this panel is attached to.
    pub session: Option<Session>,
    pub rows: Vec<Row>,
    pub pending: Option<Pending>,
    /// A reason nothing is running, when something went wrong before it started.
    pub error: Option<String>,
    pub running: bool,
    /// The last event applied, so a reconnect asks for what came after it.
    pub last_seq: u64,
    /// True once the stream reported that events this panel never saw are gone.
    pub truncated: bool,
}

impl AgentRun {
    /// Which session, and whether it is still the one on screen.
    pub fn id(&self) -> Option<&str> {
        self.session.as_ref().map(|session| session.id.as_str())
    }

    /// Everything that has to be said before a new prompt can start from here.
    pub fn busy(&self) -> bool {
        self.running || self.pending.is_some()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// One event from the stream.
    pub fn apply(&mut self, event: Event) {
        self.last_seq = self.last_seq.max(event.seq);
        match event.kind.as_str() {
            "turn" => {
                if !event.content.trim().is_empty() {
                    self.rows.push(Row::Prompt(event.content));
                }
            }
            "text" => match self.rows.last_mut() {
                // Text arrives in fragments: one answer is one row, not one row per token.
                Some(Row::Answer(answer)) => answer.push_str(&event.content),
                _ => self.rows.push(Row::Answer(event.content)),
            },
            // A model's thinking is not the answer and not a step. It is dropped rather than
            // shown as either, because a panel that mixes them is a panel you stop reading.
            "reasoning" => {}
            "tool_call" => {
                self.rows.push(Row::Tool {
                    name: event.name,
                    call_id: event.tool_id,
                    arguments: event.arguments,
                    result: None,
                });
            }
            "tool_result" => {
                // Match on the call id, so a batch of parallel calls keeps its own answers.
                let slot = self.rows.iter_mut().rev().find(|row| match row {
                    Row::Tool {
                        call_id, result, ..
                    } => result.is_none() && (call_id.is_empty() || *call_id == event.tool_id),
                    _ => false,
                });
                if let Some(Row::Tool { result, .. }) = slot {
                    *result = Some(event.content);
                } else {
                    self.rows.push(Row::Tool {
                        name: event.name,
                        call_id: event.tool_id,
                        arguments: String::new(),
                        result: Some(event.content),
                    });
                }
            }
            "needs_approval" => {
                if !event.approval.is_empty() {
                    self.pending = Some(Pending {
                        approval: event.approval,
                        name: event.name,
                        preview: event.preview,
                        diff: event.diff,
                    });
                }
            }
            "approved" => {
                self.pending = None;
                self.rows.push(Row::Decided {
                    name: event.name,
                    allowed: true,
                    for_session: false,
                });
            }
            "denied" => {
                self.pending = None;
                self.rows.push(Row::Decided {
                    name: event.name,
                    allowed: false,
                    for_session: false,
                });
            }
            "usage" => {
                if event.input_tokens > 0 || event.output_tokens > 0 {
                    self.rows.push(Row::Notice(format!(
                        "{} in · {} out",
                        event.input_tokens, event.output_tokens
                    )));
                }
            }
            "stopped" => {
                self.running = false;
                self.pending = None;
                self.rows.push(Row::Notice("stopped".into()));
            }
            "done" => {
                self.running = false;
                self.pending = None;
            }
            "error" => {
                self.running = false;
                self.pending = None;
                self.error = Some(event.message);
            }
            "truncated" => {
                self.truncated = true;
                self.running = false;
                // The card may be for a call that was already answered while this panel was
                // away. Clearing it beats offering an Allow for something that already ran.
                self.pending = None;
                self.rows.push(Row::Notice(
                    "This panel missed part of the run. Open the transcript to read it all.".into(),
                ));
            }
            other => {
                self.rows.push(Row::Notice(format!("unhandled event: {other}")));
            }
        }
    }

    /// What the footer says while a run is active.
    pub fn status(&self) -> String {
        if let Some(pending) = &self.pending {
            return format!("waiting on you · {}", pending.name);
        }
        if self.running {
            let tools = self
                .rows
                .iter()
                .filter(|row| matches!(row, Row::Tool { .. }))
                .count();
            return match tools {
                0 => "working…".to_owned(),
                count => format!("working · {count} call(s)"),
            };
        }
        match self.session.as_ref() {
            Some(session) => format!("{} · {} turn(s)", session.root_name(), session.turns),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str) -> Event {
        Event {
            kind: kind.into(),
            ..Event::default()
        }
    }

    #[test]
    fn streamed_fragments_become_one_answer() {
        let mut run = AgentRun::default();
        for fragment in ["The ", "test ", "fails."] {
            run.apply(Event {
                content: fragment.into(),
                ..event("text")
            });
        }
        assert_eq!(run.rows, [Row::Answer("The test fails.".into())]);
    }

    #[test]
    fn a_tool_result_lands_on_the_call_it_answers() {
        let mut run = AgentRun::default();
        for (id, name) in [("c1", "read"), ("c2", "bash")] {
            run.apply(Event {
                tool_id: id.into(),
                name: name.into(),
                arguments: "{}".into(),
                ..event("tool_call")
            });
        }
        run.apply(Event {
            tool_id: "c2".into(),
            name: "bash".into(),
            content: "ok".into(),
            ..event("tool_result")
        });
        let results: Vec<Option<String>> = run
            .rows
            .iter()
            .map(|row| match row {
                Row::Tool { result, .. } => result.clone(),
                _ => panic!("only tool rows here"),
            })
            .collect();
        assert_eq!(results, [None, Some("ok".into())]);
    }

    #[test]
    fn a_call_without_an_id_still_gets_its_result() {
        let mut run = AgentRun::default();
        run.apply(Event {
            name: "read".into(),
            ..event("tool_call")
        });
        run.apply(Event {
            name: "read".into(),
            content: "text".into(),
            ..event("tool_result")
        });
        assert!(matches!(
            &run.rows[0],
            Row::Tool { result: Some(result), .. } if result == "text"
        ));
    }

    #[test]
    fn an_approval_waits_and_is_cleared_by_its_answer() {
        let mut run = AgentRun::default();
        run.apply(Event {
            approval: "abc".into(),
            name: "bash".into(),
            preview: "cargo test".into(),
            diff: None,
            ..event("needs_approval")
        });
        let pending = run.pending.clone().expect("a call is waiting");
        assert_eq!(pending.preview, "cargo test");
        assert!(run.busy(), "a waiting call is a busy run");
        assert!(run.status().contains("waiting on you · bash"));

        run.apply(Event {
            approval: "abc".into(),
            name: "bash".into(),
            ..event("approved")
        });
        assert!(run.pending.is_none());
        assert_eq!(
            run.rows,
            [Row::Decided {
                name: "bash".into(),
                allowed: true,
                for_session: false
            }]
        );
    }

    #[test]
    fn a_denial_and_a_stop_both_end_the_wait() {
        let mut run = AgentRun::default();
        run.running = true;
        run.apply(Event {
            approval: "abc".into(),
            name: "write".into(),
            ..event("needs_approval")
        });
        run.apply(Event {
            approval: "abc".into(),
            name: "write".into(),
            ..event("denied")
        });
        // A denial is not an end: the model reads it and carries on, so the run is still busy.
        assert!(run.pending.is_none());
        assert!(run.running);
        assert!(!run.status().contains("waiting on you"), "{}", run.status());

        run.running = true;
        run.apply(event("stopped"));
        assert!(!run.running);
        assert!(matches!(run.rows.last(), Some(Row::Notice(text)) if text == "stopped"));
    }

    #[test]
    fn every_terminal_event_leaves_the_panel_idle() {
        for kind in ["done", "stopped", "error", "truncated"] {
            let mut run = AgentRun::default();
            run.running = true;
            run.apply(Event {
                approval: "abc".into(),
                ..event("needs_approval")
            });
            run.apply(event(kind));
            assert!(!run.running, "{kind} left the panel running");
            assert!(run.pending.is_none(), "{kind} left a call waiting");
        }
    }

    #[test]
    fn the_sequence_is_the_high_water_mark_so_a_reconnect_does_not_replay() {
        let mut run = AgentRun::default();
        run.apply(Event {
            seq: 4,
            ..event("text")
        });
        run.apply(Event {
            seq: 2,
            ..event("text")
        });
        assert_eq!(run.last_seq, 4);
    }

    #[test]
    fn reasoning_is_not_drawn_as_the_answer() {
        let mut run = AgentRun::default();
        run.apply(Event {
            content: "hmm".into(),
            ..event("reasoning")
        });
        run.apply(Event {
            content: "answer".into(),
            ..event("text")
        });
        assert_eq!(run.rows, [Row::Answer("answer".into())]);
    }

    #[test]
    fn an_unknown_event_is_named_rather_than_dropped_in_silence() {
        let mut run = AgentRun::default();
        run.apply(event("something-new"));
        assert!(matches!(run.rows.last(), Some(Row::Notice(text)) if text.contains("something-new")));
    }
}
