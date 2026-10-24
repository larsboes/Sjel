//! Which rung answers a prompt, decided without asking a model.
//!
//! Measured 2026-10-24 on this machine, same three prompts, same tools offered:
//! Apple Foundation Models (`foundation-models`, on-device, answered in 2.5 s) invented
//! `calendar_entries{"date": "2026-10-24", "status": "ready"}` for "reply with the single word:
//! ready" and returned a bare `502 Failed to deserialize a Generable type from model output` on
//! one of three; `Qwen3.5-4B-MLX-4bit` on oMLX (5–6.5 s) called `sjel_health` correctly on all
//! three. So the two rungs are not a cost choice, they are a correctness one.
//!
//! The router is a rule table over the prompt text, not a classifier model. Two consequences
//! follow, and both are deliberate:
//!
//! * **The fast rung is given no tools.** A 3B on-device model that cannot call a tool cannot
//!   invent one, so the whole class of malformed-call failures is removed rather than retried.
//! * **Misrouting is cheap to fix and cheap to justify.** A prompt that needed Sjel data and
//!   went to the fast rung gets a wrong answer, not a crash; the fix is a word in
//!   [`STATE_VOCABULARY`] plus a line in the fixture table below. A model-based router would
//!   make that fix a prompt-editing exercise and the failure a mystery.
//!
//! The bias is deliberately towards the strong rung: a chat prompt sent to the strong rung costs
//! about four seconds, while a Sjel question sent to the fast rung is answered wrongly and
//! confidently.

/// The rung that answers one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rung {
    /// The on-device model: no tools, no daemon, answers from the prompt alone.
    Fast,
    /// The strong local model: offered the read-only Sjel tools.
    Strong,
}

/// Words and stems that mean the answer has to come from Sjel's own data or from a clock the
/// fast rung does not have.
///
/// A token matches when it equals an entry, or begins with an entry of at least four characters,
/// so `capabilities`, `capability` and German compounds like `Kalendertermine` all land. Shorter
/// entries are matched exactly, because a three-letter prefix is a coincidence rather than a
/// word: `up` must not match `update`.
///
/// Entries were chosen to over-route rather than under-route. `up`, `down` and `week` are here
/// even though they appear in ordinary chat ("what's up"), because the cost of routing chat to
/// the strong rung is four seconds, and the cost of routing a state question to the fast rung is
/// a wrong answer stated with confidence.
const STATE_VOCABULARY: &[&str] = &[
    // Sjel itself.
    "sjel",
    "capabilit",
    "service",
    "health",
    "status",
    "up",
    "down",
    // Calendar and time.
    "calendar",
    "kalender",
    "event",
    "appointment",
    "termin",
    "schedule",
    "meeting",
    "today",
    "tomorrow",
    "yesterday",
    "tonight",
    "week",
    "woche",
    "heute",
    "morgen",
    "gestern",
    // Projects, records, trips.
    "project",
    "projekt",
    "record",
    "trip",
    "reise",
    "travel",
    "itinerary",
    "flight",
    "hotel",
    "booking",
    "plan",
    // Verbs that only make sense against live data.
    "running",
    "runs",
    "läuft",
    "laeuft",
    "laufen",
];

/// Weekday names, matched as whole words. Kept separate from the vocabulary because a
/// four-character prefix rule would make `Monday` match `money` and `March` match `market`; and
/// months are left out on purpose, since "may" and "march" are ordinary English too.
const CALENDAR_WORDS: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "mon",
    "tue",
    "tues",
    "wed",
    "thu",
    "thur",
    "thurs",
    "fri",
    "sat",
    "sun",
];

/// Answers the weak rung produced that are refusals rather than content.
///
/// Short by design: these are the exact strings measured on 2026-10-24 plus the wording the
/// system prompt steers towards. Anything else is treated as a real answer, because judging
/// whether prose is *good* needs a model call, and that call would cost what the ladder exists
/// to avoid.
const REFUSALS: &[&str] = &[
    "function is not available",
    "cannot determine",
    "can't determine",
    "do not have access",
    "don't have access",
    "the tool failed",
];

/// The rung a prompt should start on.
///
/// `has_history` forces the strong rung: a conversation that already has turns is one about
/// Sjel's own data in practice, and the fast rung would contradict what was said before it.
pub fn route(prompt: &str, has_history: bool) -> Rung {
    if has_history {
        return Rung::Strong;
    }
    let lowered = prompt.to_lowercase();
    if lowered
        .split(|c: char| !c.is_alphanumeric())
        .any(needs_state)
    {
        return Rung::Strong;
    }
    Rung::Fast
}

fn needs_state(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if CALENDAR_WORDS.contains(&token) {
        return true;
    }
    STATE_VOCABULARY.iter().any(|word| {
        if word.len() < 4 {
            token == *word
        } else {
            token.starts_with(word)
        }
    })
}

/// Whether a completed answer is a refusal the strong rung should replace.
pub fn refused(answer: &str) -> bool {
    let lowered = answer.to_lowercase();
    lowered.trim().is_empty() || REFUSALS.iter().any(|phrase| lowered.contains(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prompts and the rung they must start on. Every entry is a rule the router claims to
    /// implement, so a rule that stops working fails here rather than in the app.
    const CASES: &[(&str, Rung)] = &[
        ("start all capabilities", Rung::Strong),
        ("Which capabilities are up?", Rung::Strong),
        ("what is on my calendar tomorrow", Rung::Strong),
        ("Was steht heute an?", Rung::Strong),
        ("show me the Munich trip plan", Rung::Strong),
        ("list my projects", Rung::Strong),
        ("what did we decide about the planner", Rung::Strong),
        ("is everything running", Rung::Strong),
        ("check the health", Rung::Strong),
        ("any appointments on friday", Rung::Strong),
        ("Kalendertermine für morgen", Rung::Strong),
        ("läuft der Dienst?", Rung::Strong),
        ("say OK and nothing else", Rung::Fast),
        ("what is 2+2", Rung::Fast),
        ("rewrite this in a friendlier tone", Rung::Fast),
        ("thanks", Rung::Fast),
        ("summarize the paragraph I pasted", Rung::Fast),
        ("update the plan please", Rung::Strong),
        ("translate this to German", Rung::Fast),
        ("money market or equity", Rung::Fast),
        ("hello there", Rung::Fast),
        ("make it shorter", Rung::Fast),
    ];

    #[test]
    fn the_router_answers_every_fixture_as_documented() {
        for (prompt, expected) in CASES {
            assert_eq!(route(prompt, false), *expected, "routed {prompt:?} wrongly");
        }
    }

    #[test]
    fn history_forces_the_strong_rung() {
        assert_eq!(route("thanks", true), Rung::Strong);
    }

    #[test]
    fn a_refusal_is_recognised_and_a_real_answer_is_not() {
        assert!(refused(""));
        assert!(refused(
            "I cannot determine the live capabilities since the required function is not available."
        ));
        assert!(refused("The tool failed because the arguments were wrong"));
        assert!(!refused("All 28 capabilities are up."));
        assert!(!refused("4"));
    }
}
