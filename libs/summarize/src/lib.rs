//! `libs/summarize` — one adaptive digest engine for every kind of observed content.
//!
//! A digest is what the local model wrote about a thing. It is deliberately a
//! different noun from `summary`: calendar's `summary` is what the *source* said
//! it is, and overwriting that with a generated paragraph would destroy the
//! only verbatim description an entry has. See `libs/content-item/src/lib.rs`.
//!
//! ## Why length decides the shape
//!
//! A summariser with one fixed prompt produces a three-bullet digest for a
//! forty-word mail and the same three bullets for a forty-page paper. The first
//! is longer than its source and the second throws away most of it. The shape
//! is therefore derived from how much source there actually is, and below a
//! floor no digest is produced at all — the source *is* the summary, and
//! storing a paraphrase of it is a second thing to read, not a shorter one.
//!
//! ## Why "more detailed" is a rung, not an adjective
//!
//! Asking a model to "be more detailed" gets you a longer version of the same
//! guess. [`Depth::Detailed`] instead moves the shape exactly one step up the
//! same ladder, which changes both the requested structure and the token
//! ceiling. That is inspectable after the fact — a stored digest records the
//! rung it was produced at — and an operator pressing refine twice does not get
//! two different unrelated documents.
//!
//! ## Dependency rule
//!
//! This is a normal workspace crate with an intentionally narrow dependency
//! surface: `serde_json` and blocking `reqwest`. It deliberately does **not**
//! name `libs/inference`'s types: a consumer builds a [`Target`] from whatever
//! it resolved, so a capability that has no inference dependency still compiles.

// Non-inline submodule: rustc resolves it against this file's own directory.
pub mod chart;

use std::time::Duration;

/// Bump with any change to [`digest_prompt`] or the shape ladder. It is part of
/// the stored producer string, so a change here is what makes an existing
/// digest legibly stale rather than silently mixed with new ones.
pub const DIGEST_PROMPT_REVISION: &str = "content-digest-v1-adaptive";

/// Bump with any change to [`diagram_prompt`] or the accepted diagram headers.
pub const DIAGRAM_PROMPT_REVISION: &str = "content-diagram-v1-mermaid";

/// Max characters of source text handed to the model. A one-hour transcript is
/// 100k+ characters and the local model's window is finite; the full text stays
/// stored unchanged either way.
pub const INPUT_CAP: usize = 15_000;

/// Below this, the source is already shorter than any honest digest of it.
///
/// Roughly a hundred words. A "digest" of a two-line mail is the same two lines
/// with different words in front of them, and it costs a model call to produce
/// something nobody gains by reading. The automatic pass skips these; an
/// explicit [`Depth::Detailed`] press overrides the skip, because the operator
/// looking at the item knows something the character count does not.
pub const SHORT_SOURCE_FLOOR: usize = 600;

/// How much structure the digest is asked for. Derived from source length,
/// never chosen by the caller directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Nothing worth digesting. Not a failure — a verdict.
    None,
    /// A few bullets, no framing sentence.
    Brief,
    /// Bullets plus one sentence of context. What every feed item got before
    /// this ladder existed.
    Standard,
    /// Grouped bullets under short headings, plus context. For papers and
    /// long transcripts, where a flat bullet list loses the argument's shape.
    Sectioned,
}

impl Shape {
    /// The rung a source of this length earns on its own.
    pub fn for_length(chars: usize) -> Self {
        match chars {
            0..=599 => Shape::None,
            600..=2_499 => Shape::Brief,
            2_500..=8_999 => Shape::Standard,
            _ => Shape::Sectioned,
        }
    }

    /// One rung up, saturating. This is the whole implementation of "more
    /// detailed, please".
    pub fn next(self) -> Self {
        match self {
            Shape::None => Shape::Brief,
            Shape::Brief => Shape::Standard,
            Shape::Standard | Shape::Sectioned => Shape::Sectioned,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Shape::None => "none",
            Shape::Brief => "brief",
            Shape::Standard => "standard",
            Shape::Sectioned => "sectioned",
        }
    }

    /// The ceiling that goes with the rung. A bigger structure needs room to
    /// finish; a smaller one must not be allowed to sprawl into the larger
    /// one's territory, or the ladder stops meaning anything.
    pub const fn max_tokens(self) -> u32 {
        match self {
            Shape::None => 0,
            Shape::Brief => 200,
            Shape::Standard => 500,
            Shape::Sectioned => 1_000,
        }
    }

    /// What the model is actually asked to produce at this rung.
    fn instruction(self) -> &'static str {
        match self {
            // Unreachable through `digest`, which returns SkippedShort first.
            Shape::None => "",
            Shape::Brief => {
                "Write two to three short bullet points covering only what the source \
                 actually establishes. Do not add a closing sentence."
            }
            Shape::Standard => {
                "Write the key points as short bullet points, then add exactly one sentence \
                 of context."
            }
            Shape::Sectioned => {
                "Group the key points under at most four short headings that follow the \
                 source's own structure, as bullet points under each. Then add exactly one \
                 sentence of context. Keep concrete numbers, names and results where the \
                 source gives them."
            }
        }
    }
}

/// Characters per token, for sizing a prompt against a model's context window.
///
/// Measured rather than guessed: the 14,064-character transcript that started
/// this work prefilled to 3,568 tokens, which is 3.94 characters per token.
/// Three is deliberately below that. This number decides whether a small model
/// is offered work it cannot hold, and being wrong in that direction produces a
/// hard context error, so it sizes for denser text than the sample rather than
/// for the sample.
pub const CHARS_PER_TOKEN: usize = 3;

/// Tokens to allow for the instruction, the focus terms and the chat envelope
/// wrapped around the source. The prompt preamble is about 60 words; the rest
/// is headroom.
pub const PROMPT_OVERHEAD_TOKENS: u32 = 400;

/// The largest prompt this module can construct, in tokens.
///
/// Every input is capped at [`INPUT_CAP`], every prompt carries
/// [`PROMPT_OVERHEAD_TOKENS`], and the widest reply is [`Shape::Sectioned`]. So the context a
/// summarization backend must offer is arithmetic, not a judgement — which matters because the
/// number has to be configured *outside* this repo, on whatever server holds the role, and a
/// number nobody can derive is one somebody eventually guesses.
///
/// Guessing it cost something measurable. Ollama loads a model at 4,096 tokens whatever window
/// the architecture declares, and its `/v1/chat/completions` — the endpoint an `api = "ollama"`
/// backend uses — accepts no per-request `num_ctx`. It does not refuse an over-window prompt
/// either: it drops the front and answers 200. Measured 2026-08-30, a 7,399-token prompt came
/// back reported as 2,050 with the opening silently gone. A digest of the tail of a document,
/// presented as a digest of the document.
pub const fn worst_case_prompt_tokens() -> u32 {
    (INPUT_CAP / CHARS_PER_TOKEN) as u32 + PROMPT_OVERHEAD_TOKENS + Shape::Sectioned.max_tokens()
}

/// Whether a source of this length, digested at this rung, fits a context
/// window of `context_tokens`.
///
/// Input **and** output together: a window is shared between the prompt and the
/// answer, and a model that accepts the prompt and then has no room to reply
/// has not helped. This is what keeps the cheap rungs on a small model and
/// sends the long ones to a large one, rather than discovering the ceiling by
/// hitting it.
pub fn fits_context(source_chars: usize, shape: Shape, context_tokens: u32) -> bool {
    fits_window(source_chars, shape.max_tokens(), context_tokens)
}

/// The same question for a caller whose reply ceiling is not a [`Shape`].
///
/// The feed-summary prefill in `capabilities/comms/src/media.rs` asks for a
/// fixed 800 tokens rather than climbing this ladder, and it has to decide
/// whether a small model can hold the job for exactly the same reason. Two
/// arithmetics for one question is how one of them ends up sizing only the
/// prompt and offering a 4,096-token model a request it cannot answer.
pub fn fits_window(source_chars: usize, reply_tokens: u32, context_tokens: u32) -> bool {
    let input = source_chars.min(INPUT_CAP) / CHARS_PER_TOKEN;
    let needed = (input as u64)
        .saturating_add(PROMPT_OVERHEAD_TOKENS as u64)
        .saturating_add(reply_tokens as u64);
    needed <= context_tokens as u64
}

/// The longest source, in characters, that [`fits_window`] accepts: its inverse, for a caller
/// that can shorten its input instead of refusing it. One arithmetic, read both ways.
pub fn window_chars(reply_tokens: u32, context_tokens: u32) -> usize {
    (context_tokens as usize)
        .saturating_sub(PROMPT_OVERHEAD_TOKENS as usize + reply_tokens as usize)
        .saturating_mul(CHARS_PER_TOKEN)
        .min(INPUT_CAP)
}

/// Whether this run is the automatic pass or an operator asking for more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Depth {
    #[default]
    Standard,
    Detailed,
}

impl Depth {
    pub fn as_str(self) -> &'static str {
        match self {
            Depth::Standard => "standard",
            Depth::Detailed => "detailed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "standard" => Some(Depth::Standard),
            "detailed" => Some(Depth::Detailed),
            _ => None,
        }
    }
}

/// At most this many focus terms, each at most this long. The operator types
/// these, so the bound is about keeping the prompt's shape predictable rather
/// than about trust — but a prompt whose instruction section can grow without
/// limit is one where the source text gets squeezed out by the request for it.
pub const MAX_FOCUS_TERMS: usize = 8;
pub const MAX_FOCUS_TERM_CHARS: usize = 40;

/// What the operator asked for, on top of what the length already decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Directive {
    pub depth: Depth,
    pub focus: Vec<String>,
}

impl Directive {
    /// Trim, drop empties and control characters, de-duplicate case-insensitively,
    /// and bound both the count and each term.
    pub fn new(depth: Depth, focus: impl IntoIterator<Item = String>) -> Self {
        let mut clean: Vec<String> = Vec::new();
        for term in focus {
            let term: String = term
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_FOCUS_TERM_CHARS)
                .collect();
            let term = term.trim().to_string();
            if term.is_empty() {
                continue;
            }
            if clean
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&term))
            {
                continue;
            }
            clean.push(term);
            if clean.len() == MAX_FOCUS_TERMS {
                break;
            }
        }
        Self {
            depth,
            focus: clean,
        }
    }

    /// The stored form. Comma-joined because it is display state read back into
    /// a text field, not something anything queries by.
    pub fn focus_text(&self) -> String {
        self.focus.join(", ")
    }

    /// Split a stored `focus` column back into terms.
    pub fn parse_focus(stored: &str) -> Vec<String> {
        stored
            .split(',')
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// The rung this directive lands on for a source of this length.
    pub fn shape_for(&self, source_chars: usize) -> Shape {
        let base = Shape::for_length(source_chars);
        match self.depth {
            Depth::Standard => base,
            Depth::Detailed => base.next(),
        }
    }
}

/// Admission to make one local request, released when dropped.
///
/// Drop rather than an explicit `release`, because [`complete`] returns from a
/// dozen places and a permit leaked on any one of them wedges every other
/// process for as long as the holder lives.
pub struct Admission(Option<Box<dyn FnOnce() + Send>>);

impl Admission {
    /// Hold a permit whose release needs work — unlocking, closing, decrementing.
    pub fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self(Some(Box::new(release)))
    }

    /// A permit that needs no cleanup. For gates that bound by counting an
    /// atomic, or for tests.
    pub fn free() -> Self {
        Self(None)
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            release();
        }
    }
}

impl std::fmt::Debug for Admission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Admission")
    }
}

/// Bounds how many local requests may be in flight at once.
///
/// The mechanism is the caller's, because this lib may only use `serde_json`
/// and blocking `reqwest` (see the dependency rule at the top of this file) and
/// so cannot hold a database handle, a socket, or a lock file. The *policy*
/// still lives here: [`complete`] will not make a loopback request without a
/// permit when a gate is configured, for the same reason the [`Reach`] refusal
/// lives here rather than in each caller.
///
/// Why this exists: on 2026-08-05 four concurrent prefills pushed oMLX past its
/// hard watermark and it aborted all four. Nothing in Axon knew the others were
/// running. An in-process semaphore would not have helped — the competing
/// consumers are separate processes.
pub trait LocalGate: Send + Sync {
    /// Wait for admission, or give up and say why. Giving up is not a failure
    /// of the request: the caller reports it as a capacity condition and the
    /// item is retried later.
    fn acquire(&self) -> Result<Admission, String>;
}

/// How far one payload is allowed to travel.
///
/// Deliberately not a `bool`. This is a verdict the caller's cloud gate reached
/// about one specific item's stored data class, and `digest(t, text, &d, false)`
/// records neither what was decided nor about what — the two literals that
/// stamped `public` on every feed item were exactly that shape, and both read as
/// valid Rust at the call site. A named variant does not make a verdict correct,
/// but it makes an unconsidered one visible in review.
///
/// [`Reach::CloudCleared`] is a claim about a *specific* endpoint: the gate was
/// asked whether this class, in this representation, may go to that provider's
/// tier. It is not a general permission and does not travel with the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Hardware the operator controls, and nothing else. A target that is
    /// neither loopback nor a declared trusted peer is refused outright —
    /// [`Outcome::RemoteRefused`], never a quiet downgrade to a local model.
    ///
    /// The name says `Loopback` because until PRD Q39 (2026-08-25) those were
    /// the same set. They are not any more: a trusted peer is the operator's own
    /// machine reached over the tailnet, and this verdict admits it. The variant
    /// is not renamed because 27 call sites spell it and the name is not the
    /// defect — [`Target::operator_owned`] is where the question is actually
    /// asked, and it is named for the question.
    LoopbackOnly,
    /// The caller's cloud gate admitted this payload for this endpoint.
    CloudCleared,
}

impl Reach {
    fn admits(self, target: &Target) -> bool {
        target.operator_owned || self == Self::CloudCleared
    }
}

/// Where to send the request. Built by the caller from whatever role it
/// resolved, so this lib never names `libs/inference`'s types.
#[derive(Clone)]
pub struct Target {
    pub backend_name: String,
    pub max_input_tokens: Option<u32>,
    /// A full chat-completions URL.
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    /// Whether the endpoint shares THIS machine's GPU. Only the admission gate
    /// below asks this; it is not a permission.
    ///
    /// Split from [`Self::operator_owned`] by PRD Q39 (2026-08-25). One `bool`
    /// answered both questions while loopback and trusted-hardware were the same
    /// set, and a trusted peer is the first target for which they differ: it may
    /// see any class, and it must NOT queue behind this host's gate, because it
    /// has its own GPU.
    pub loopback: bool,
    /// Whether the endpoint runs on hardware the operator controls — loopback,
    /// or a peer they declared as their own (PRD Q39). This is the permission
    /// question [`Reach::LoopbackOnly`] asks.
    pub operator_owned: bool,
    /// Admission control for targets on this machine. `None` means unbounded,
    /// which is what every caller did before this existed.
    ///
    /// Only consulted when `loopback` is true. A hosted provider does its own
    /// queueing and has no shared GPU to protect, so serialising against it
    /// would buy nothing and cost latency — and a trusted peer is a hosted
    /// provider in exactly that respect, whatever it is allowed to see.
    pub gate: Option<std::sync::Arc<dyn LocalGate>>,
}

// Hand-written because `Arc<dyn LocalGate>` is not Debug, and requiring that of
// implementors would leak this lib's logging choices into their types.
impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("loopback", &self.loopback)
            .field("operator_owned", &self.operator_owned)
            .field("gate", &self.gate.as_ref().map(|_| "<gate>"))
            .finish()
    }
}

/// Typed outcome, so a caller can tell "nothing to do" from "server down" from
/// "the model answered with nothing" and record the class for bounded retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok(String),
    /// The source is below the floor and no one forced it. Terminal and correct.
    SkippedShort,
    /// The item is Personal or Private and the resolved target is not loopback.
    /// A policy refusal, never retried into success by waiting.
    RemoteRefused,
    Unconfigured,
    HttpError(String),
    ModelError(String),
    /// Temporarily disallowed; preserve stored output and retry budgets.
    PolicyDeferred(String),
    /// The server took the request and then ran out of room for it. Separate
    /// from [`Outcome::ModelError`] because it is a fact about the machine
    /// rather than about the request: the identical prompt succeeds when
    /// something else is not holding the GPU.
    CapacityAborted(String),
    EmptyResponse,
    Timeout,
}

impl Outcome {
    /// Short, loggable state for the digest row.
    pub fn state(&self) -> &'static str {
        match self {
            Outcome::Ok(_) => "generated",
            Outcome::SkippedShort => "skipped_short",
            Outcome::RemoteRefused => "remote_refused",
            Outcome::Unconfigured => "unconfigured",
            Outcome::HttpError(_) => "http_error",
            Outcome::ModelError(_) => "model_error",
            Outcome::PolicyDeferred(_) => "policy_deferred",
            Outcome::CapacityAborted(_) => "capacity_aborted",
            Outcome::EmptyResponse => "empty_response",
            Outcome::Timeout => "timeout",
        }
    }

    /// Whether a later run could plausibly do better. `skipped_short` and
    /// `remote_refused` are verdicts, not transient failures.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Outcome::HttpError(_)
                | Outcome::ModelError(_)
                | Outcome::CapacityAborted(_)
                | Outcome::EmptyResponse
                | Outcome::Timeout
        )
    }

    /// The message worth showing a reader, if any.
    pub fn error_detail(&self) -> Option<&str> {
        match self {
            Outcome::HttpError(detail)
            | Outcome::ModelError(detail)
            | Outcome::CapacityAborted(detail)
            | Outcome::PolicyDeferred(detail) => Some(detail),
            _ => None,
        }
    }
}

/// The stored producer string: which backend, which model, which prompt.
/// Changing any of the three makes existing rows legibly stale.
pub fn producer(target_cache_key: &str, prompt_revision: &str) -> String {
    format!("{target_cache_key}:{prompt_revision}")
}

/// Cap the text handed to the model, marking the cut so a reader can tell a
/// digest of the whole thing from a digest of its first half.
pub fn truncate(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        text.to_string()
    } else {
        let head: String = text.chars().take(cap).collect();
        format!("{head}…[truncated]")
    }
}

/// The prompt. English output regardless of source language, no preamble, and
/// the focus terms named explicitly rather than folded into the instruction —
/// the operator should be able to read the stored `focus` and recognise what
/// they asked for.
pub fn digest_prompt(input: &str, shape: Shape, directive: &Directive) -> String {
    let mut prompt = String::from("Summarize the following content as a compact digest. ");
    prompt.push_str(shape.instruction());
    prompt.push_str(
        " Write in English, even when the source is in another language. Do not add a preamble, \
         and do not describe what you are about to do.",
    );
    if !directive.focus.is_empty() {
        prompt.push_str(
            "\n\nThe reader has asked you to pay particular attention to these topics. Cover what \
             the source says about each one; if the source says nothing about a topic, say so in \
             one short line rather than inventing material:\n",
        );
        for term in &directive.focus {
            prompt.push_str("- ");
            prompt.push_str(term);
            prompt.push('\n');
        }
    }
    prompt.push_str("\n\nContent:\n");
    prompt.push_str(input);
    prompt
}

/// Produce a digest of `text`.
///
/// [`Reach`] is the caller's data-class verdict: anything its cloud gate did not
/// admit arrives as [`Reach::LoopbackOnly`], and a non-loopback target is then
/// refused outright rather than quietly downgraded. That check lives here, at
/// the one place that makes the request, because a policy enforced by each
/// caller separately is a policy with as many holes as there are callers.
pub fn digest(target: Option<&Target>, text: &str, directive: &Directive, reach: Reach) -> Outcome {
    // Empty is not "short" — there is nothing for the operator to have seen
    // that the count missed, so the Detailed override does not apply. Asking a
    // model to summarize an empty string produces confident invention.
    if text.trim().is_empty() {
        return Outcome::SkippedShort;
    }
    let source_chars = text.chars().count();
    let shape = directive.shape_for(source_chars);
    if shape == Shape::None {
        return Outcome::SkippedShort;
    }
    let Some(target) = target else {
        return Outcome::Unconfigured;
    };
    if !reach.admits(target) {
        return Outcome::RemoteRefused;
    }
    let input = if target.backend_name == "foundation-models" { text.to_owned() } else { truncate(text, INPUT_CAP) };
    let prompt = digest_prompt(&input, shape, directive);
    complete(target, &prompt, shape.max_tokens())
}

/// Put one prompt to a resolved target and hand back what came back.
///
/// The caller owns the words and the budget; this owns the wire. It exists so
/// that a capability with a prompt of its own does not reimplement
/// [`complete`] — which is where the [`LocalGate`] is acquired and where the
/// per-backend admission lives — and so that [`Reach`] is checked in the same
/// one place [`digest`] checks it. A policy enforced by each caller separately
/// is a policy with as many holes as there are callers.
///
/// What it deliberately does NOT do: size the prompt against the model's
/// context window. [`digest`] truncates to `INPUT_CAP` and derives its reply
/// ceiling from a [`Shape`], but the *window* is a property of the resolved
/// role, and [`Target`] is deliberately plain data — this lib never learns what
/// an `InferenceConfig` is. So the budget check lives one frame up, in the
/// caller that resolved the role, and it is mandatory there: a caller that
/// reaches `ask` without having asked [`fits_window`] first will be told by the
/// server, as an error it then has to remember not to count.
///
/// `capabilities/comms` reaches this only through `quiet::Rung::Light`, which
/// `quiet::rung` returns only when `fits_window` said yes, and
/// `mail_model::tests::ask_is_only_reached_through_a_light_rung` pins it.
pub fn ask(target: Option<&Target>, prompt: &str, reply_tokens: u32, reach: Reach) -> Outcome {
    // Same reason `digest` refuses an empty source: a model asked about nothing
    // answers with confident invention, and the answer would be stored.
    if prompt.trim().is_empty() {
        return Outcome::SkippedShort;
    }
    let Some(target) = target else {
        return Outcome::Unconfigured;
    };
    if !reach.admits(target) {
        return Outcome::RemoteRefused;
    }
    complete(target, prompt, reply_tokens)
}

/// The diagram headers a renderer can actually draw. A model asked for "a
/// diagram" will happily answer with prose, and prose stored in a diagram
/// column is a render error at the reader rather than a failure here.
pub const MERMAID_HEADERS: [&str; 12] = [
    "flowchart",
    "graph",
    "sequenceDiagram",
    "classDiagram",
    "stateDiagram",
    "stateDiagram-v2",
    "erDiagram",
    "journey",
    "gantt",
    "pie",
    "mindmap",
    "timeline",
];

pub fn diagram_prompt(input: &str) -> String {
    format!(
        "Draw the structure of the following content as a single Mermaid diagram. Choose the \
         diagram type that fits what the content actually is: `flowchart` for a process or an \
         architecture, `sequenceDiagram` for an exchange between parties, `timeline` for dated \
         events, `mindmap` for a topic breakdown. Use at most fifteen nodes and keep every label \
         under six words. Answer with one fenced ```mermaid block and nothing else — no \
         explanation before or after it. Label text must not contain parentheses, quotes or \
         semicolons.\n\nContent:\n{input}"
    )
}

/// Pull the diagram out of a model answer and refuse anything a renderer would
/// choke on. Accepts a fenced block or a bare diagram, because models produce
/// both and the fence is presentation, not content.
pub fn extract_mermaid(answer: &str) -> Result<String, String> {
    let body = match answer.split_once("```mermaid") {
        Some((_, rest)) => rest
            .split_once("```")
            .map(|(inside, _)| inside)
            .ok_or_else(|| "unterminated ```mermaid block".to_string())?,
        // A bare answer may still be a diagram; a *differently* fenced one is a
        // model that ignored the format, and guessing which fence it meant is
        // how prose ends up in the column.
        None if answer.contains("```") => {
            return Err("answer used a fence that was not ```mermaid".into())
        }
        None => answer,
    };
    let body = body.trim();
    if body.is_empty() {
        return Err("empty diagram".into());
    }
    let header = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("%%"))
        .unwrap_or_default();
    let keyword = header.split_whitespace().next().unwrap_or_default();
    if !MERMAID_HEADERS.contains(&keyword) {
        return Err(format!("not a Mermaid diagram: starts with {keyword:?}"));
    }
    Ok(body.to_string())
}

/// Produce a Mermaid diagram of `text`. Same remote refusal as [`digest`]: a
/// diagram of a private mail is still that mail.
pub fn diagram(target: Option<&Target>, text: &str, reach: Reach) -> Outcome {
    let Some(target) = target else {
        return Outcome::Unconfigured;
    };
    if !reach.admits(target) {
        return Outcome::RemoteRefused;
    }
    if text.trim().is_empty() {
        return Outcome::SkippedShort;
    }
    let input = if target.backend_name == "foundation-models" { text.to_owned() } else { truncate(text, INPUT_CAP) };
    let prompt = diagram_prompt(&input);
    match complete(target, &prompt, 700) {
        Outcome::Ok(answer) => match extract_mermaid(&answer) {
            Ok(diagram) => Outcome::Ok(diagram),
            Err(reason) => Outcome::ModelError(reason),
        },
        other => other,
    }
}

fn runtime_admission(target: &Target, prompt: &str, reply_tokens: u32) -> Result<(), String> {
    let local = reqwest::Url::parse(&target.endpoint).ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host == "localhost" || host.trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()));
    sjel_runtime::admit_model(&target.backend_name, &target.model, local).map_err(|e| e.to_string())?;
    if local && target.backend_name == "foundation-models" && target.model == "apple-foundationmodel" {
        let window = target.max_input_tokens.unwrap_or(4096).min(4096) as usize;
        if prompt.len().saturating_add(reply_tokens as usize).saturating_add(256) > window {
            return Err("deferred by runtime profile: AFM input plus reply exceeds its context; no request was sent".into());
        }
        let base = target.endpoint.strip_suffix("/chat/completions").unwrap_or(&target.endpoint);
        if !sjel_inference::afm_available(base) {
            return Err("deferred by runtime profile: AFM model is unavailable".into());
        }
    }
    Ok(())
}

/// One OpenAI-compatible chat completion. The only place in this lib that
/// speaks HTTP.
pub(crate) fn complete(target: &Target, prompt: &str, max_tokens: u32) -> Outcome {
    if let Err(reason) = runtime_admission(target, prompt, max_tokens) {
        return Outcome::PolicyDeferred(reason);
    }
    // Held for the whole request and released by drop on every return path
    // below. Loopback only: a hosted provider queues for itself and shares no
    // GPU with anything here.
    let _admission = match &target.gate {
        Some(gate) if target.loopback => match gate.acquire() {
            Ok(admission) => Some(admission),
            // Not a failure of the request. The machine is busy, the same
            // prompt succeeds later, and the drain will bring it back.
            Err(reason) => return Outcome::CapacityAborted(reason),
        },
        _ => None,
    };
    let http = match sjel_http::client(
        sjel_http::Purpose::new("summarize"),
        Duration::from_secs(120),
    ) {
        Ok(client) => client,
        Err(error) => return Outcome::HttpError(error.to_string()),
    };
    let mut request = http.post(&target.endpoint).json(&serde_json::json!({
        "model": target.model,
        "messages": [{ "role": "user", "content": prompt }],
        "max_tokens": max_tokens,
        "stream": false,
    }));
    if let Some(key) = &target.api_key {
        request = request.bearer_auth(key);
    }
    if let Err(reason) = runtime_admission(target, prompt, max_tokens) {
        return Outcome::PolicyDeferred(reason);
    }
    let response = match request.send() {
        Ok(response) => response,
        Err(error) if error.is_timeout() => return Outcome::Timeout,
        Err(error) => return Outcome::HttpError(error.to_string()),
    };
    if !response.status().is_success() {
        return Outcome::ModelError(format!("status {}", response.status()));
    }
    let body: serde_json::Value = match response.json() {
        Ok(body) => body,
        Err(error) => return Outcome::ModelError(error.to_string()),
    };
    // Before `choices` — see `server_error` for why a 200 is not an answer.
    match server_error(&body) {
        Some(ServerError::Capacity(message)) => return Outcome::CapacityAborted(message),
        Some(ServerError::Other(message)) => return Outcome::ModelError(message),
        None => {}
    }
    let answer = body
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .map(str::trim)
        .unwrap_or_default();
    if answer.is_empty() {
        Outcome::EmptyResponse
    } else {
        Outcome::Ok(answer.to_string())
    }
}

/// What an OpenAI-shaped error body says, once it is known to be one.
///
/// Two variants because they mean different things to a reader. [`Capacity`]
/// says nothing about the request: the identical prompt succeeds on a quieter
/// machine, so it is worth retrying and worth reporting as a machine condition.
/// [`Other`] is a fact about the request.
///
/// [`Capacity`]: ServerError::Capacity
/// [`Other`]: ServerError::Other
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerError {
    Capacity(String),
    Other(String),
}

impl ServerError {
    pub fn message(&self) -> &str {
        match self {
            ServerError::Capacity(message) | ServerError::Other(message) => message,
        }
    }
}

/// Codes that mean the server ran out of room, not that the request was wrong.
///
/// Matched on `code`/`omlx_code` rather than on the message, because the
/// message is prose written for a human and carries live byte counts. Matching
/// it would be a string comparison against something designed to change.
const CAPACITY_CODES: [&str; 2] = ["prefill_memory_aborted", "prefill_memory_exceeded"];

/// Read an OpenAI-shaped error body, if that is what this response is.
///
/// **Call this before reading `choices`.** A 200 does not mean the server
/// answered: a server that streams keepalive padding while it works has already
/// committed its status line before it knows whether the request will finish,
/// so a failure after that point arrives as a successful response whose body
/// holds an error and no `choices` at all. Reading `choices` first turns that
/// into "the model answered with nothing", which names the wrong component.
///
/// Confirmed against oMLX on 2026-08-06: six concurrent large prefills, five
/// returned HTTP 200 with `{"error": {...,"code":"prefill_memory_aborted"}}`
/// and no `choices` key. Not oMLX-specific though — the error shape is the
/// OpenAI one, and any compatible server can reach for it late.
///
/// A body carrying both an error and usable content is not an error response.
/// Some servers attach a warning alongside a real answer, and discarding a
/// finished digest over it would throw away work the model already did.
pub fn server_error(body: &serde_json::Value) -> Option<ServerError> {
    let has_content = body
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .is_some_and(|content| !content.trim().is_empty());
    if has_content {
        return None;
    }
    let detail = body.get("error")?.as_object()?;
    let code = detail
        .get("omlx_code")
        .or_else(|| detail.get("code"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let message = detail
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("the server returned an error with no message")
        .trim()
        .to_string();
    Some(if CAPACITY_CODES.contains(&code) {
        ServerError::Capacity(message)
    } else {
        ServerError::Other(message)
    })
}

#[cfg(test)]
mod tests {
    /// The number a backend has to be configured to hold, pinned so a change to any of its three
    /// inputs shows up here rather than as a truncated digest on somebody's machine.
    ///
    /// 8,192 is what `axon-qwen3-4b:ctx8k` was built with on 2026-08-30, and this is the margin
    /// that made 8,192 the answer instead of the next power of two down.
    #[test]
    fn the_context_a_summarization_backend_must_offer_is_derivable() {
        assert_eq!(
            worst_case_prompt_tokens(),
            (INPUT_CAP / CHARS_PER_TOKEN) as u32 + PROMPT_OVERHEAD_TOKENS + 1_000
        );
        assert_eq!(worst_case_prompt_tokens(), 6_400);
        assert!(
            worst_case_prompt_tokens() > 4_096,
            "Ollama's default window cannot hold this, which is why a role on it needs a model \
             built with an explicit num_ctx"
        );
    }

    use super::*;

    #[test]
    fn the_shape_ladder_has_no_gaps_and_no_overlaps() {
        assert_eq!(Shape::for_length(0), Shape::None);
        assert_eq!(Shape::for_length(SHORT_SOURCE_FLOOR - 1), Shape::None);
        assert_eq!(Shape::for_length(SHORT_SOURCE_FLOOR), Shape::Brief);
        assert_eq!(Shape::for_length(2_499), Shape::Brief);
        assert_eq!(Shape::for_length(2_500), Shape::Standard);
        assert_eq!(Shape::for_length(8_999), Shape::Standard);
        assert_eq!(Shape::for_length(9_000), Shape::Sectioned);
        assert_eq!(Shape::for_length(400_000), Shape::Sectioned);
    }

    /// The ceiling has to rise with the rung, or a Sectioned digest gets cut
    /// off mid-section and reads as a worse Standard one.
    #[test]
    fn every_rung_raises_the_token_ceiling() {
        assert!(Shape::Brief.max_tokens() < Shape::Standard.max_tokens());
        assert!(Shape::Standard.max_tokens() < Shape::Sectioned.max_tokens());
        assert_eq!(Shape::None.max_tokens(), 0);
    }

    #[test]
    fn detailed_moves_exactly_one_rung_and_saturates() {
        let detailed = Directive::new(Depth::Detailed, []);
        let standard = Directive::new(Depth::Standard, []);
        assert_eq!(standard.shape_for(3_000), Shape::Standard);
        assert_eq!(detailed.shape_for(3_000), Shape::Sectioned);
        assert_eq!(detailed.shape_for(50_000), Shape::Sectioned);
    }

    /// The automatic pass skips a short source; an explicit press does not.
    /// That is the whole affordance — the operator can see something the
    /// character count cannot.
    #[test]
    fn an_explicit_detailed_press_overrides_the_short_skip() {
        let text = "Two lines. Nothing more.";
        assert_eq!(
            digest(None, text, &Directive::default(), Reach::LoopbackOnly),
            Outcome::SkippedShort
        );
        // Past the floor check, an absent target is what stops it — proving the
        // skip is no longer what returned.
        assert_eq!(
            digest(
                None,
                text,
                &Directive::new(Depth::Detailed, []),
                Reach::LoopbackOnly
            ),
            Outcome::Unconfigured
        );
    }

    /// The override is for a source the operator can see and the count
    /// under-rates. Nothing is not that: found live on a feed item whose
    /// transcript is zero characters, where forcing a digest would have asked
    /// the model to summarize an empty string and invent the answer.
    #[test]
    fn nothing_at_all_is_skipped_even_when_the_operator_insists() {
        for empty in ["", "   ", "\n\t "] {
            assert_eq!(
                digest(
                    None,
                    empty,
                    &Directive::new(Depth::Detailed, []),
                    Reach::LoopbackOnly
                ),
                Outcome::SkippedShort,
                "{empty:?} should stay skipped"
            );
        }
    }

    #[test]
    fn focus_terms_are_trimmed_deduplicated_and_bounded() {
        let directive = Directive::new(
            Depth::Standard,
            [
                "  benchmark ".into(),
                "BENCHMARK".into(),
                String::new(),
                "  ".into(),
                "a".repeat(200),
            ],
        );
        assert_eq!(directive.focus.len(), 2);
        assert_eq!(directive.focus[0], "benchmark");
        assert_eq!(directive.focus[1].chars().count(), MAX_FOCUS_TERM_CHARS);

        let many: Vec<String> = (0..50).map(|n| format!("term{n}")).collect();
        assert_eq!(
            Directive::new(Depth::Standard, many).focus.len(),
            MAX_FOCUS_TERMS
        );
    }

    #[test]
    fn focus_text_round_trips_through_storage() {
        let directive = Directive::new(Depth::Detailed, ["cost".into(), "latency".into()]);
        let stored = directive.focus_text();
        assert_eq!(stored, "cost, latency");
        assert_eq!(Directive::parse_focus(&stored), vec!["cost", "latency"]);
        assert!(Directive::parse_focus("").is_empty());
        assert!(Directive::parse_focus(" , ,").is_empty());
    }

    #[test]
    fn the_prompt_carries_the_rung_the_language_and_the_focus() {
        let directive = Directive::new(Depth::Standard, ["evaluation".into()]);
        let prompt = digest_prompt(
            "Ein deutschsprachiger Quelltext.",
            Shape::Sectioned,
            &directive,
        );
        assert!(prompt.contains("at most four short headings"));
        assert!(prompt.contains("Write in English"));
        assert!(prompt.contains("- evaluation"));
        assert!(prompt.contains("Ein deutschsprachiger Quelltext."));

        let plain = digest_prompt("x", Shape::Brief, &Directive::default());
        assert!(plain.contains("two to three short bullet points"));
        assert!(
            !plain.contains("particular attention"),
            "no focus section when nothing was asked for"
        );
    }

    /// Anything the caller's cloud gate did not admit arrives as
    /// `Reach::LoopbackOnly`. A cloud target must then be refused outright —
    /// not truncated, not downgraded.
    #[test]
    fn a_non_loopback_target_is_refused_for_restricted_content() {
        let cloud = Target {
            backend_name: "test".into(),
            max_input_tokens: None,
            endpoint: "https://api.example.com/v1/chat/completions".into(),
            model: "m".into(),
            api_key: None,
            loopback: false,
            operator_owned: false,
            gate: None,
        };
        let text = "x".repeat(1_000);
        assert_eq!(
            digest(
                Some(&cloud),
                &text,
                &Directive::default(),
                Reach::LoopbackOnly
            ),
            Outcome::RemoteRefused
        );
        assert_eq!(
            diagram(Some(&cloud), &text, Reach::LoopbackOnly),
            Outcome::RemoteRefused
        );
        assert_eq!(
            chart::chart(Some(&cloud), &text, Reach::LoopbackOnly),
            Outcome::RemoteRefused
        );
        assert_eq!(
            ask(Some(&cloud), "classify this", 256, Reach::LoopbackOnly),
            Outcome::RemoteRefused,
            "`ask` shares digest's refusal, which is the reason it exists here"
        );
    }

    /// `ask` refuses the two things it can decide without a request, and stops
    /// there. The window is the caller's, and its doc comment says so — a test
    /// that pinned a budget check here would be pinning a check this function
    /// deliberately does not have.
    #[test]
    fn ask_refuses_an_empty_prompt_and_an_absent_target() {
        assert_eq!(
            ask(None, "classify this", 256, Reach::LoopbackOnly),
            Outcome::Unconfigured
        );
        let local = Target {
            backend_name: "test".into(),
            max_input_tokens: None,
            endpoint: "http://127.0.0.1:9/v1/chat/completions".into(),
            model: "m".into(),
            api_key: None,
            loopback: true,
            operator_owned: true,
            gate: None,
        };
        for empty in ["", "   ", "\n\t "] {
            assert_eq!(
                ask(Some(&local), empty, 256, Reach::LoopbackOnly),
                Outcome::SkippedShort,
                "an empty prompt must not reach the wire"
            );
        }
    }

    /// A loopback target is reachable whatever the verdict says: `Reach` is
    /// about leaving the machine, and a local model is not leaving it. Getting
    /// this backwards would make every Personal digest `RemoteRefused` against
    /// the model that is supposed to serve it.
    #[test]
    fn a_loopback_target_is_never_refused_on_reach() {
        let local = Target {
            backend_name: "test".into(),
            max_input_tokens: None,
            endpoint: "http://127.0.0.1:8000/v1/chat/completions".into(),
            model: "m".into(),
            api_key: None,
            loopback: true,
            operator_owned: true,
            gate: None,
        };
        assert!(Reach::LoopbackOnly.admits(&local));
        assert!(Reach::CloudCleared.admits(&local));
    }

    /// The bodies below are verbatim from oMLX on 2026-08-06: six concurrent
    /// large prefills, five came back HTTP **200** carrying an error and no
    /// `choices`. Reading `choices` first classified them as `EmptyResponse`,
    /// so the reader was told "the local model answered with nothing" about a
    /// request the model never saw.
    #[test]
    fn a_two_hundred_carrying_an_error_is_not_an_empty_answer() {
        let aborted: serde_json::Value = serde_json::from_str(
            r#"{"error": {"message": "oMLX memory guard aborted this request mid-prefill: Request aborted: process memory limit exceeded (usage 18.2 GB, abort threshold (hard watermark) 18.0 GB, metal_cap ceiling 19.0 GB).", "type": "invalid_request_error", "param": null, "code": "prefill_memory_aborted", "omlx_code": "prefill_memory_aborted", "limit_bytes": 19381039923}, "type": "error"}"#,
        )
        .unwrap();
        let rejected: serde_json::Value = serde_json::from_str(
            r#"{"error": {"message": "oMLX prefill memory guard rejected this prompt: Prefill context too large for available memory (pre-chunk guard at 11264 tokens, kv_len=11264).", "type": "invalid_request_error", "code": "prefill_memory_exceeded", "omlx_code": "prefill_memory_exceeded"}, "type": "error"}"#,
        )
        .unwrap();

        for (body, label) in [(&aborted, "aborted"), (&rejected, "rejected")] {
            match server_error(body) {
                Some(ServerError::Capacity(message)) => {
                    assert!(message.contains("memory"), "{label}: {message}")
                }
                other => panic!("{label} should be a capacity error, got {other:?}"),
            }
        }
    }

    /// Anything else the server calls an error is still an error — just one
    /// about the request rather than about the machine.
    #[test]
    fn a_non_capacity_error_stays_a_model_error() {
        let body: serde_json::Value = serde_json::from_str(
            r#"{"error": {"message": "context length exceeded", "code": "context_length_exceeded"}}"#,
        )
        .unwrap();
        assert_eq!(
            server_error(&body),
            Some(ServerError::Other("context length exceeded".into()))
        );

        // No code at all is still not an empty answer.
        let bare: serde_json::Value =
            serde_json::from_str(r#"{"error": {"message": "upstream unavailable"}}"#).unwrap();
        assert!(matches!(server_error(&bare), Some(ServerError::Other(_))));

        // And an error with no message still says something.
        let mute: serde_json::Value = serde_json::from_str(r#"{"error": {}}"#).unwrap();
        assert!(server_error(&mute).is_some_and(|error| !error.message().is_empty()));
    }

    /// A real answer wins over a warning riding along beside it. Discarding a
    /// finished digest because the server attached a note would throw away work
    /// the model already did.
    #[test]
    fn an_answer_with_a_warning_beside_it_is_still_an_answer() {
        let body: serde_json::Value = serde_json::from_str(
            r#"{"choices": [{"message": {"content": "- A real point"}}],
                 "error": {"message": "deprecated parameter"}}"#,
        )
        .unwrap();
        assert_eq!(server_error(&body), None);

        // Whitespace-only content is not an answer, so the error wins.
        let blank: serde_json::Value = serde_json::from_str(
            r#"{"choices": [{"message": {"content": "   "}}],
                 "error": {"message": "truncated", "code": "prefill_memory_aborted"}}"#,
        )
        .unwrap();
        assert!(matches!(
            server_error(&blank),
            Some(ServerError::Capacity(_))
        ));
    }

    /// A body with neither an error nor content is the case `EmptyResponse`
    /// still exists for, and it must not be swallowed by the new branch.
    #[test]
    fn a_body_with_no_error_and_no_content_is_still_empty() {
        for text in [
            r#"{"choices": [{"message": {"content": ""}}]}"#,
            r#"{"choices": []}"#,
            r#"{}"#,
        ] {
            let body: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(server_error(&body), None, "{text} is not an error body");
        }
    }

    /// A capacity abort is a fact about the machine, so it retries; and it
    /// carries its detail to the reader rather than a generic line.
    #[test]
    fn a_capacity_abort_retries_and_explains_itself() {
        let outcome = Outcome::CapacityAborted("ran out of room".into());
        assert_eq!(outcome.state(), "capacity_aborted");
        assert!(outcome.retryable());
        assert_eq!(outcome.error_detail(), Some("ran out of room"));
    }

    /// A gate that refuses turns into a capacity condition rather than a
    /// failure of the request, and the request is never sent. The endpoint here
    /// is unroutable on purpose: if the gate leaked, this would hang or return
    /// an HttpError instead, and the assertion would say so.
    #[test]
    fn a_busy_gate_stops_a_local_request_before_it_is_sent() {
        struct AlwaysBusy;
        impl LocalGate for AlwaysBusy {
            fn acquire(&self) -> Result<Admission, String> {
                Err("another local inference request held the machine".into())
            }
        }
        let local = Target {
            backend_name: "test".into(),
            max_input_tokens: None,
            endpoint: "http://127.0.0.1:9/v1/chat/completions".into(),
            model: "m".into(),
            api_key: None,
            loopback: true,
            operator_owned: true,
            gate: Some(std::sync::Arc::new(AlwaysBusy)),
        };
        assert_eq!(
            digest(
                Some(&local),
                &"x".repeat(1_000),
                &Directive::default(),
                Reach::LoopbackOnly
            ),
            Outcome::CapacityAborted("another local inference request held the machine".into())
        );
    }

    /// Anti-claim: the gate exists to protect one shared GPU. A hosted provider
    /// has none, queues for itself, and must not be serialised behind local
    /// work — so a non-loopback target ignores the gate even when one is set.
    #[test]
    fn a_cloud_target_is_never_gated() {
        struct NeverCalled;
        impl LocalGate for NeverCalled {
            fn acquire(&self) -> Result<Admission, String> {
                panic!("a non-loopback target must not consult the gate");
            }
        }
        let cloud = Target {
            backend_name: "test".into(),
            max_input_tokens: None,
            endpoint: "https://api.example.com/v1/chat/completions".into(),
            model: "m".into(),
            api_key: None,
            loopback: false,
            operator_owned: false,
            gate: Some(std::sync::Arc::new(NeverCalled)),
        };
        // Refused for its data class before any gate question arises, which is
        // itself the ordering that matters: policy first, capacity second.
        assert_eq!(
            digest(
                Some(&cloud),
                &"x".repeat(1_000),
                &Directive::default(),
                Reach::LoopbackOnly
            ),
            Outcome::RemoteRefused
        );
        // And allowed through to the network without the gate panicking.
        assert!(matches!(
            digest(
                Some(&cloud),
                &"x".repeat(1_000),
                &Directive::default(),
                Reach::CloudCleared
            ),
            Outcome::HttpError(_) | Outcome::Timeout | Outcome::ModelError(_)
        ));
    }

    /// The permit is held for the whole request and released once, on the way
    /// out. A gate that hands out a permit per call and never gets it back
    /// deadlocks the second caller.
    #[test]
    fn a_permit_is_released_when_the_request_finishes() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        #[derive(Default)]
        struct Counting {
            taken: AtomicUsize,
            given_back: AtomicUsize,
        }
        impl LocalGate for Arc<Counting> {
            fn acquire(&self) -> Result<Admission, String> {
                self.taken.fetch_add(1, Ordering::SeqCst);
                let me = Arc::clone(self);
                Ok(Admission::new(move || {
                    me.given_back.fetch_add(1, Ordering::SeqCst);
                }))
            }
        }

        let counting = Arc::new(Counting::default());
        let target = Target {
            backend_name: "test".into(),
            max_input_tokens: None,
            endpoint: "http://127.0.0.1:9/v1/chat/completions".into(),
            model: "m".into(),
            api_key: None,
            loopback: true,
            operator_owned: true,
            gate: Some(std::sync::Arc::new(Arc::clone(&counting))),
        };
        // Fails at the socket, which is the point: the release must happen on
        // the error path too, not only on a clean answer.
        let _ = digest(
            Some(&target),
            &"x".repeat(1_000),
            &Directive::default(),
            Reach::LoopbackOnly,
        );
        assert_eq!(counting.taken.load(Ordering::SeqCst), 1);
        assert_eq!(counting.given_back.load(Ordering::SeqCst), 1);
    }

    /// The whole point of the sizing rule: Apple's on-device model is a 4,096
    /// token window, and the source that started this work needed more than
    /// that. It can serve the short rungs and structurally cannot serve the
    /// long one, which is what makes it a rung-selector rather than a downgrade.
    #[test]
    fn a_four_thousand_token_window_takes_the_short_rungs_and_not_the_long_one() {
        const APPLE: u32 = 4_096;

        // 2,000 characters is Brief: ~667 input + 400 overhead + 200 output.
        assert!(fits_context(2_000, Shape::Brief, APPLE));
        // 8,000 characters is Standard: ~2,667 + 400 + 500, still inside.
        assert!(fits_context(8_000, Shape::Standard, APPLE));
        // The 14,064-character transcript at Sectioned is not: ~4,688 + 400 + 1,000.
        assert!(!fits_context(14_064, Shape::Sectioned, APPLE));
        // Nor is anything at the input cap, whatever the rung.
        assert!(!fits_context(INPUT_CAP, Shape::Brief, APPLE));
    }

    /// Output counts against the window too. A model that accepts the prompt
    /// and then has no room to answer has not helped, and sizing on input alone
    /// is how you discover that at the point of use.
    #[test]
    fn the_answer_is_sized_against_the_window_as_well_as_the_prompt() {
        // Input alone fits this window at every rung; only the ceiling differs.
        let window = PROMPT_OVERHEAD_TOKENS + 1_000 + Shape::Standard.max_tokens();
        assert!(fits_context(3_000, Shape::Standard, window));
        assert!(
            !fits_context(3_000, Shape::Sectioned, window),
            "the larger rung asks for more room to finish and must not fit"
        );
    }

    /// A large window swallows everything, which is the strong model's case and
    /// must not accidentally exclude anything.
    #[test]
    fn a_large_window_fits_every_rung() {
        for shape in [Shape::Brief, Shape::Standard, Shape::Sectioned] {
            assert!(fits_context(INPUT_CAP, shape, 32_000), "{shape:?}");
        }
        assert!(
            !fits_context(1, Shape::Brief, 0),
            "a zero window fits nothing"
        );
    }

    #[test]
    fn a_refusal_or_a_skip_is_never_retried() {
        assert!(!Outcome::SkippedShort.retryable());
        assert!(!Outcome::RemoteRefused.retryable());
        assert!(!Outcome::Unconfigured.retryable());
        assert!(Outcome::Timeout.retryable());
        assert!(Outcome::EmptyResponse.retryable());
        assert!(Outcome::ModelError("boom".into()).retryable());
    }

    #[test]
    fn truncate_marks_only_when_it_cuts() {
        assert_eq!(truncate("short", 15_000), "short");
        let long = "x".repeat(20_000);
        let out = truncate(&long, 15_000);
        assert!(out.ends_with("…[truncated]"));
        assert_eq!(out.chars().count(), 15_000 + "…[truncated]".chars().count());
    }

    #[test]
    fn a_fenced_diagram_is_unwrapped() {
        let answer = "Here you go:\n```mermaid\nflowchart TD\n  A --> B\n```\nHope that helps.";
        assert_eq!(extract_mermaid(answer).unwrap(), "flowchart TD\n  A --> B");
    }

    #[test]
    fn a_bare_diagram_is_accepted() {
        assert_eq!(
            extract_mermaid("sequenceDiagram\n  A->>B: hi").unwrap(),
            "sequenceDiagram\n  A->>B: hi"
        );
        assert!(extract_mermaid("%% a comment\nmindmap\n  root").is_ok());
    }

    /// The gate that matters: prose stored in a diagram column is a render
    /// error at the reader, which is exactly where it is hardest to diagnose.
    #[test]
    fn prose_and_foreign_fences_are_rejected_rather_than_stored() {
        for answer in [
            "I'd be happy to help! Here is an overview of the paper.",
            "```python\nprint('hi')\n```",
            "```mermaid\nflowchart TD\n  A --> B",
            "```mermaid\n\n```",
            "",
        ] {
            assert!(
                extract_mermaid(answer).is_err(),
                "should have been rejected: {answer:?}"
            );
        }
    }

    #[test]
    fn the_producer_string_names_backend_model_and_prompt() {
        assert_eq!(
            producer("openai|http://127.0.0.1:8080|gemma", DIGEST_PROMPT_REVISION),
            "openai|http://127.0.0.1:8080|gemma:content-digest-v1-adaptive"
        );
    }

    #[test]
    fn depth_parses_only_the_two_stored_values() {
        assert_eq!(Depth::parse("standard"), Some(Depth::Standard));
        assert_eq!(Depth::parse("detailed"), Some(Depth::Detailed));
        assert_eq!(Depth::parse("deeper"), None);
        assert_eq!(Depth::parse(""), None);
    }

    #[test]
    fn window_chars_is_the_longest_source_fits_window_accepts() {
        for (reply, context) in [(700, 4_096), (800, 8_192), (700, 1_000), (700, 0)] {
            let room = window_chars(reply, context);
            if room > 0 {
                assert!(fits_window(room, reply, context), "{reply}/{context}");
            }
            // fits_window clamps a source to INPUT_CAP, so past the cap anything longer "fits".
            if room < INPUT_CAP {
                assert!(
                    !fits_window(room + CHARS_PER_TOKEN, reply, context),
                    "{reply}/{context}"
                );
            }
        }
        assert_eq!(window_chars(700, 4_096), 8_988);
    }
}
