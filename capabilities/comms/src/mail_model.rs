//! The second mail classification rung: a local model, on the rows the
//! deterministic rules did not decide.
//!
//! It runs after `rules::classify` and only on the threads that fell through to
//! the conservative `aktiv` default — `{prefix}_triage_rules.decided_by =
//! 'fallback'`. Everything it can do is bounded before it starts:
//!
//! - **A Secret mail is never prompted.** `content_item::local_prompt_allowed`
//!   is asked before the target is built and before the prompt is assembled,
//!   the same order `digest::write_digest` uses, and a `c3` thread gets a
//!   stored [`digest::LOCAL_REFUSED`] row rather than an absence. A row with no
//!   verdict reads as "not looked at yet"; this one never will be.
//! - **Only the light local rung answers.** `quiet::rung` decides, and
//!   `Rung::OverWindow` is terminal. The strong `summarization` role is not
//!   resolved anywhere in this file, deliberately: `quiet` states that the
//!   fallthrough "would put the strong local model back on the drain's path by
//!   the exact route that made the machine hot". One reachable rung also means
//!   one current producer, which is what makes the single-producer staleness
//!   check sound here where `digest::producer_revisions` needed a list of
//!   three.
//! - **Nothing leaves the machine.** Every call carries
//!   `summarize::Reach::LoopbackOnly`, so a non-loopback target is refused
//!   outright rather than quietly used.
//! - **Nothing moves by default.** The pass writes to
//!   `{prefix}_triage_model_verdicts` and changes no category until the overlay
//!   declares `mail_model.apply`, and even then it refuses any proposal that
//!   would raise the data class.
//!
//! The prompt carries the sender **domain** only. `from_addr` survives
//! redaction deliberately (see `intake`), so on a `c2` row the From header is
//! the last field still holding a person's name — and the domain is the part
//! that classifies anyway. Subject and snippet need no new redaction: `intake`
//! already ran `deterministic-entity-redaction-v3` over them before the row was
//! written. The model's own sentences are free text and go back through
//! `cloud_derivative::redact_review_field` before storage.

use std::collections::HashMap;

use serde::Deserialize;

use crate::cloud_derivative::{redact_review_field, RedactionFinding};
use crate::config::{Config, MailModelConfig};
use crate::content_item::{self, DataClass};
use crate::store::{ModelCandidate, ModelVerdict, Store};
use crate::summarize::{self, Outcome, Reach};
use crate::{digest, evaluation, quiet, rules};

/// The classifier version a model-written row carries, beside
/// `rules::MAIL_RULES_VERSION` on the deterministic ones.
pub const MAIL_MODEL_VERSION: &str = "mail-model-v1";

/// The prompt this rung puts. Two constants with two jobs: change the words and
/// every stored verdict is legibly stale; change the ladder and the version
/// moves instead.
pub const MAIL_MODEL_PROMPT_REVISION: &str = "mail-stream-v1-english";

/// The reply ceiling. Five short JSON fields; the window arithmetic in
/// `quiet::rung` counts it against the same 4,096 tokens as the prompt.
pub const REPLY_TOKENS: u32 = 256;

/// The model answered, but not with JSON this parser could find. A fact about
/// the prompt rather than about the machine, so it is retried a bounded number
/// of times and then left alone — if a pass shows a high rate here, the fix is
/// the prompt, not a longer retry.
pub const UNPARSEABLE: &str = "unparseable";

/// The model named a stream that is not one of the seven. Terminal: a second
/// identical prompt reaches the same answer.
pub const INVALID_STREAM: &str = "invalid_stream";

/// A model rationale is a line in a review card, not an essay.
pub const MAX_RATIONALE_CHARS: usize = 300;

/// How much of a model-supplied failure detail is worth storing in
/// `last_error`.
///
/// Shorter than a rationale on purpose. A category name is one word and a
/// transport error is a status line; anything longer is the model or the local
/// server having echoed the mail back, and `last_error` is the one column on
/// this table a reader does not read for content. It is redacted as well as
/// capped — see [`stored_error`].
pub const MAX_ERROR_CHARS: usize = 80;

/// The states a thread can only be in because a request reached the model.
///
/// `prompted` is a privacy receipt: the CLI prints it beside "refused as
/// Secret", so it has to count requests that were issued rather than answers
/// that were usable. A timeout, an HTTP error and a capacity abort are all
/// produced after `summarize::ask` has sent the request
/// (`libs/summarize/src/lib.rs`), so they belong here; `local_refused`,
/// `skipped_over_window`, `unconfigured` and `remote_refused` are all decided
/// before anything is sent, so they do not.
pub const PROMPTED_STATES: [&str; 8] = [
    "generated",
    "http_error",
    "model_error",
    "capacity_aborted",
    "empty_response",
    "timeout",
    UNPARSEABLE,
    INVALID_STREAM,
];

/// Whether a verdict in this state means a request was put to the model.
///
/// One predicate, because the pass receipt and the report's `by_data_class`
/// both answer the same question and a second copy is how they came to
/// disagree (review, 2026-09-05).
pub fn was_prompted(state: &str) -> bool {
    PROMPTED_STATES.contains(&state)
}

/// Every state this rung can write, which is the constraint the column does not
/// carry — `{prefix}_triage_model_verdicts.state` has no CHECK, following
/// `{prefix}_content_digests`. `every_stored_state_is_in_the_documented_set`
/// is what holds it.
pub const MODEL_VERDICT_STATES: [&str; 13] = [
    // The nine from `summarize::Outcome::state()`.
    "generated",
    "skipped_short",
    "remote_refused",
    "unconfigured",
    "http_error",
    "model_error",
    "capacity_aborted",
    "empty_response",
    "timeout",
    // Borrowed verbatim from digest, because they are the same two verdicts
    // about the same two questions.
    digest::LOCAL_REFUSED,
    digest::SKIPPED_OVER_WINDOW,
    // This rung's own two, both about the shape of the answer.
    UNPARSEABLE,
    INVALID_STREAM,
];

/// What the pass is allowed to do with what it finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Measure and store. Changes no category. The default, and the only mode
    /// available until the overlay says otherwise.
    Shadow,
    /// Write non-escalating disagreements onto the category axis.
    Apply,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shadow => "shadow",
            Self::Apply => "apply",
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "shadow" => Ok(Self::Shadow),
            "apply" => Ok(Self::Apply),
            other => Err(format!("mode must be 'shadow' or 'apply', got '{other}'")),
        }
    }
}

/// Whether this machine may run the requested mode.
///
/// A pure function over the config section rather than a read of `Config`, so
/// the refusal is testable without touching the operator's real overlay — and
/// so the test does not invert the day somebody sets the key. It manipulates no
/// environment variable, for the reason a var restored under a sibling test
/// mid-assertion is already on the record.
pub fn apply_allowed(
    section: Option<&MailModelConfig>,
    requested: Mode,
) -> std::result::Result<Mode, String> {
    match (requested, section) {
        (Mode::Shadow, _) => Ok(Mode::Shadow),
        // A floor of zero writes every disagreement the model reports at any
        // self-reported confidence, including zero — the opposite of what the
        // key's own doc comment promises. The field defaults to 0 because a
        // number invented here would be as arbitrary as that one, so the
        // operator states the floor in the same edit that turns writing on
        // (review, 2026-09-05).
        (Mode::Apply, Some(section)) if section.apply && section.min_confidence_bp == 0 => Err(
            "apply is refused: mail_model.apply is true but mail_model.min_confidence_bp is 0, \
             which would write every disagreement at any confidence — set the floor the frozen \
             corpus measured"
                .into(),
        ),
        (Mode::Apply, Some(section)) if section.apply => Ok(Mode::Apply),
        (Mode::Apply, Some(_)) => Err(
            "apply is refused: the overlay's comms.json declares mail_model.apply = false".into(),
        ),
        (Mode::Apply, None) => Err(
            "apply is refused: the overlay's comms.json declares no mail_model section, \
             so this rung runs in shadow only"
                .into(),
        ),
    }
}

/// How many threads one pass may act on.
///
/// The order is: what the caller asked for, else what the overlay's
/// `mail_model.limit` says, else 200. Read here rather than in each entry
/// point, because both of them hard-coded their own 200 and the declared
/// overlay key was read nowhere (review, 2026-09-05). Clamped, so neither an
/// unbounded pass nor a zero-length one is reachable from a request body.
pub fn pass_limit(section: Option<&MailModelConfig>, requested: Option<usize>) -> usize {
    requested
        .or_else(|| section.map(|section| section.limit))
        .unwrap_or(DEFAULT_PASS_LIMIT)
        .clamp(1, MAX_PASS_LIMIT)
}

/// The pass size an operator gets without asking for one.
pub const DEFAULT_PASS_LIMIT: usize = 200;

/// The most one pass may act on however it is asked. A local model at roughly
/// two seconds a thread makes a larger number a wait, not a feature.
pub const MAX_PASS_LIMIT: usize = 500;

/// The domain of a From header, lowercased, with the display name and the local
/// part dropped.
///
/// `None` for anything with no `@`, which reads as "the prompt gets no sender
/// line" rather than as a guess.
pub fn sender_domain(from: &str) -> Option<String> {
    // `Display Name <local@domain>` and a bare `local@domain` are both live
    // shapes; take the last angle-bracketed span when there is one.
    let inner = match (from.rfind('<'), from.rfind('>')) {
        (Some(open), Some(close)) if close > open + 1 => &from[open + 1..close],
        _ => from,
    };
    let domain = inner.rsplit_once('@')?.1;
    let domain = domain
        .trim()
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-')
        .to_ascii_lowercase();
    (!domain.is_empty()).then_some(domain)
}

/// The prompt, built from `rules::STREAM_DEFINITIONS` so the vocabulary the
/// model is told about is the vocabulary the column admits.
///
/// English, one prompt, matching the feed evaluator's own revision name and
/// Apple's declared support for both languages. The corpus carries a `language`
/// field per fixture precisely so the first measurement can answer whether
/// German mail is the gap.
pub fn prompt(domain: Option<&str>, subject: &str, snippet: &str) -> String {
    let mut prompt = String::from(
        "You sort one email into exactly one category for its recipient. \
         Answer with a single JSON object and nothing else.\n\n\
         The categories, and what each one means:\n",
    );
    for (name, definition) in rules::STREAM_DEFINITIONS {
        prompt.push_str("- ");
        prompt.push_str(name);
        prompt.push_str(": ");
        prompt.push_str(definition);
        prompt.push('\n');
    }
    prompt.push_str(
        "\nAlso rate how urgently this email needs the recipient's attention, from 0 \
         (nothing is asked of them) to 10000 (it needs an answer today). Urgency is about \
         what the email asks, not about how it is written: marketing that shouts is not \
         urgent.\n\n\
         Answer with exactly these five keys:\n\
         {\"stream\": \"<one category name from the list above>\", \
         \"confidence_bp\": <0-10000, how sure you are of the category>, \
         \"urgency_bp\": <0-10000>, \
         \"rationale\": \"<one English sentence, under 30 words, naming the signal you used>\", \
         \"urgency_rationale\": \"<one English sentence, under 20 words>\"}\n\n\
         Write in English even when the email is in another language. Some values may have \
         been replaced with markers such as [account] or [email]; that is deliberate, and a \
         marker is not a reason to lower your confidence.\n\n\
         The email:\n",
    );
    if let Some(domain) = domain {
        prompt.push_str("Sender domain: ");
        prompt.push_str(domain);
        prompt.push('\n');
    }
    prompt.push_str("Subject: ");
    prompt.push_str(subject.trim());
    prompt.push_str("\nPreview: ");
    prompt.push_str(snippet.trim());
    prompt.push('\n');
    prompt
}

/// One parsed answer, already validated against the vocabulary and clamped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub stream: String,
    pub confidence_bp: i64,
    pub urgency_bp: i64,
    pub rationale: String,
    pub urgency_rationale: String,
}

/// Why an answer produced no verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseFailure {
    /// No JSON object could be found, or it did not deserialize.
    Unparseable,
    /// A well-formed object naming a stream outside `rules::STREAMS`.
    InvalidStream(String),
}

impl ParseFailure {
    pub fn state(&self) -> &'static str {
        match self {
            Self::Unparseable => UNPARSEABLE,
            Self::InvalidStream(_) => INVALID_STREAM,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct RawAnswer {
    #[serde(default)]
    stream: String,
    #[serde(default)]
    confidence_bp: Option<f64>,
    #[serde(default)]
    urgency_bp: Option<f64>,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    urgency_rationale: String,
}

/// Pull the answer out of whatever the model wrapped it in.
///
/// Defensive by necessity: `afm-server` rejects unsupported request parameters
/// with a 400, so there is no `response_format` and no JSON mode to lean on.
/// The whole correctness of the structured output rests here.
///
/// Two candidate spans are tried, in this order: the first balanced brace span
/// of the answer as it arrived, then the one inside a fence. The raw answer
/// comes first because stripping the fence first threw away a well-formed
/// answer that happened to be FOLLOWED by a fenced example — the reply parsed
/// as `unparseable`, which is retryable, so it cost three prompts and left the
/// thread with no proposal (review, 2026-09-05). A fenced answer still parses
/// either way: a fence carries no braces of its own.
pub fn parse(answer: &str) -> std::result::Result<Answer, ParseFailure> {
    let raw = [brace_span(answer), brace_span(strip_fence(answer))]
        .into_iter()
        .flatten()
        .filter_map(|span| serde_json::from_str::<RawAnswer>(span).ok())
        // An object with no `stream` is not this rung's answer, whatever else
        // it deserialized into — so the second span still gets its turn.
        .find(|raw| !raw.stream.trim().is_empty())
        .ok_or(ParseFailure::Unparseable)?;

    let stream = raw.stream.trim().to_ascii_lowercase();
    if !rules::STREAMS.contains(&stream.as_str()) {
        return Err(ParseFailure::InvalidStream(stream));
    }
    Ok(Answer {
        stream,
        // A model that answers 0.8 rather than 8000 has answered a fraction;
        // scaling it is a guess, so it is clamped as given and the corpus is
        // what decides whether the number is worth anything at all.
        confidence_bp: clamp_bp(raw.confidence_bp),
        urgency_bp: clamp_bp(raw.urgency_bp),
        rationale: cap(&raw.rationale),
        urgency_rationale: cap(&raw.urgency_rationale),
    })
}

fn clamp_bp(value: Option<f64>) -> i64 {
    value.unwrap_or_default().round().clamp(0.0, 10_000.0) as i64
}

fn cap(value: &str) -> String {
    value.trim().chars().take(MAX_RATIONALE_CHARS).collect()
}

/// What `last_error` may hold: short, and redacted against the row's class.
///
/// The two values that reach this column are both model-derived. An invalid
/// stream is whatever word the model invented, and a transport detail is
/// whatever the local server said — a server that echoes the prompt back in an
/// error body puts mail text in it. Before this, `last_error` was the one
/// model-derived column that skipped `redact_review_field`, which contradicted
/// the DDL comment on the table (review, 2026-09-05).
fn stored_error(detail: &str, data_class: &str) -> String {
    let short: String = detail.trim().chars().take(MAX_ERROR_CHARS).collect();
    if !content_item::redact_before_persistence(data_class) {
        return short;
    }
    let mut findings: Vec<RedactionFinding> = Vec::new();
    redact_review_field(Some(&short), &mut findings).unwrap_or_default()
}

/// Drop a ```json (or ``` or ```JSON) fence, keeping the body.
fn strip_fence(answer: &str) -> &str {
    let Some((_, rest)) = answer.split_once("```") else {
        return answer;
    };
    // The opening fence may carry a language tag on the same line.
    let rest = rest.split_once('\n').map_or(rest, |(tag, body)| {
        if tag.trim().chars().all(char::is_alphanumeric) {
            body
        } else {
            rest
        }
    });
    rest.split_once("```").map_or(rest, |(inside, _)| inside)
}

/// The first balanced `{...}` span, so trailing prose costs nothing.
fn brace_span(body: &str) -> Option<&str> {
    let start = body.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, character) in body[start..].char_indices() {
        if in_string {
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&body[start..start + offset + character.len_utf8()]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The class a verdict's free text must be redacted against: the higher of the
/// class the row holds and the class the PROPOSED stream implies.
///
/// `steuern` and `belege` are Others by rule, so a verdict proposing one of
/// them on a `c1` row is already about `c2` material by the time it is stored.
/// Redacting up front against the higher of the two means a held verdict
/// carries `c2`-grade text before any human sees it, and the confirm path needs
/// no second narrowing write — `narrow_review_fields` only ever touches
/// `{prefix}_triage_items`, so a later escalation would never reach this row.
pub fn effective_class(stored: &str, proposed_stream: &str, from: &str, subject: &str) -> String {
    let implied = DataClass::classify_mail(proposed_stream, from, subject).value;
    if content_item::class_rank(&implied) > content_item::class_rank(stored) {
        implied
    } else {
        stored.to_string()
    }
}

/// The revision one verdict was reached against.
///
/// The same length-prefixed SHA-256 the feed evaluator uses, over exactly the
/// four fields the prompt is built from. A thread whose subject changed is a
/// different question, and its stored verdict is stale rather than wrong.
pub fn item_revision(
    domain: Option<&str>,
    subject: &str,
    snippet: &str,
    data_class: &str,
) -> String {
    evaluation::revision_hash(&[domain.unwrap_or_default(), subject, snippet, data_class])
}

/// The producer string of the one role that may answer this prompt, or
/// `"unconfigured"` when this machine has no light role.
///
/// Resolved before the class gate rather than after it, so a refusal row
/// records the truthful provenance of the model that WOULD have run instead of
/// claiming nothing was configured.
pub fn producer(cfg: &Config) -> String {
    quiet::light_role(&cfg.inference)
        .map(|role| summarize::producer(&role.cache_key(), MAIL_MODEL_PROMPT_REVISION))
        .unwrap_or_else(|| "unconfigured".into())
}

/// Ask the model about one thread and build the verdict, whatever happened.
///
/// Never `Err`: every path here is a verdict about the thread, and a verdict
/// with a state is the receipt. The order is `digest::write_digest`'s order,
/// and the first two steps are the ones that matter:
///
/// 1. resolve the light role, so the producer is truthful on every row;
/// 2. ask `content_item::local_prompt_allowed` — before the target is built and
///    before the prompt is assembled;
/// 3. `quiet::rung`, which answers `OverWindow` rather than truncating;
/// 4. `digest::to_target`, which is where the per-backend admission gate is
///    attached — reproducing it here would give foundation-models two admission
///    paths;
/// 5. `summarize::ask` with `Reach::LoopbackOnly`;
/// 6. redact the model's own sentences against [`effective_class`].
pub fn classify_one(cfg: &Config, candidate: &ModelCandidate) -> ModelVerdict {
    let producer = producer(cfg);
    let from = candidate.from_addr.as_deref().unwrap_or_default();
    let domain = sender_domain(from);
    let subject = candidate.subject.as_deref().unwrap_or_default();
    let snippet = candidate.snippet.as_deref().unwrap_or_default();
    let item_revision = item_revision(domain.as_deref(), subject, snippet, &candidate.data_class);

    let mut verdict = ModelVerdict {
        triage_id: candidate.id.clone(),
        mode: Mode::Shadow.as_str().into(),
        state: String::new(),
        rule_decided_by: candidate.rule_decided_by.clone(),
        rule_stream: candidate.rule_stream.clone(),
        model_stream: None,
        confidence_bp: None,
        urgency_bp: None,
        rationale: None,
        urgency_rationale: None,
        redactions: 0,
        data_class: candidate.data_class.clone(),
        redaction_class: candidate.data_class.clone(),
        producer,
        item_revision,
        prompt_revision: MAIL_MODEL_PROMPT_REVISION.into(),
        classification_version: MAIL_MODEL_VERSION.into(),
        attempts: 0,
        last_error: None,
        next_attempt: None,
        held_reason: None,
        applied_at: None,
    };

    // A credential belongs in no processing step, local or cloud, and a local
    // model is still a log, a context window and a cache. Checked here means no
    // prompt is assembled, no local model is woken, no request is made.
    if !content_item::local_prompt_allowed(&candidate.data_class) {
        verdict.state = digest::LOCAL_REFUSED.into();
        return verdict;
    }

    let prompt = prompt(domain.as_deref(), subject, snippet);
    // The budget check `summarize::ask` deliberately does not do. `ask` is
    // reached only through `Rung::Light`, and only `quiet::rung` produces one.
    let role = match quiet::rung(&cfg.inference, prompt.chars().count(), REPLY_TOKENS) {
        quiet::Rung::Light(role) => *role,
        // Terminal, and it is a verdict rather than a failure: no strong model
        // is woken, and a later pass on the same machine reaches the same
        // answer.
        quiet::Rung::OverWindow => {
            verdict.state = digest::SKIPPED_OVER_WINDOW.into();
            return verdict;
        }
        quiet::Rung::Unconfigured => {
            verdict.state = Outcome::Unconfigured.state().into();
            return verdict;
        }
    };

    let target = digest::to_target(cfg, &role);
    let outcome = summarize::ask(Some(&target), &prompt, REPLY_TOKENS, Reach::LoopbackOnly);
    let answer = match &outcome {
        Outcome::Ok(answer) => answer.clone(),
        other => {
            verdict.state = other.state().into();
            verdict.last_error = other
                .error_detail()
                .map(|detail| stored_error(detail, &candidate.data_class));
            verdict.attempts = i64::from(other.retryable());
            return verdict;
        }
    };

    match parse(&answer) {
        Err(failure) => {
            verdict.state = failure.state().into();
            if let ParseFailure::InvalidStream(named) = &failure {
                let named = stored_error(named, &candidate.data_class);
                verdict.last_error = Some(format!("the model named '{named}'"));
            }
            verdict.attempts = i64::from(failure == ParseFailure::Unparseable);
            verdict
        }
        Ok(answer) => {
            let redaction_class =
                effective_class(&candidate.data_class, &answer.stream, from, subject);
            let mut redactions: Vec<RedactionFinding> = Vec::new();
            let (rationale, urgency_rationale) =
                if content_item::redact_before_persistence(&redaction_class) {
                    (
                        redact_review_field(Some(&answer.rationale), &mut redactions),
                        redact_review_field(Some(&answer.urgency_rationale), &mut redactions),
                    )
                } else {
                    (
                        Some(answer.rationale.clone()),
                        Some(answer.urgency_rationale.clone()),
                    )
                };
            verdict.state = Outcome::Ok(String::new()).state().into();
            verdict.model_stream = Some(answer.stream);
            verdict.confidence_bp = Some(answer.confidence_bp);
            verdict.urgency_bp = Some(answer.urgency_bp);
            verdict.rationale = rationale;
            verdict.urgency_rationale = urgency_rationale;
            verdict.redactions = redactions
                .iter()
                .map(|finding| finding.count)
                .sum::<usize>() as i64;
            verdict.redaction_class = redaction_class;
            verdict
        }
    }
}

/// What one pass did. Counts only — no subject, no snippet, no rationale — so
/// the body is safe to log and to paste into a decision record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassReceipt {
    pub mode: String,
    pub reviewed: usize,
    pub eligible: usize,
    /// Threads carrying a current shadow verdict that disagrees with the rule,
    /// counted as this pass FOUND them. Counted in both modes: in shadow it is
    /// what the pass leaves for an operator to decide on, and in apply it is
    /// what this pass then acted on, up to `limit`.
    pub awaiting_apply: usize,
    pub prompted: usize,
    pub refused_c3: usize,
    pub over_window: usize,
    pub unparseable: usize,
    pub invalid_stream: usize,
    pub agreed_no_write: usize,
    pub disagreed: usize,
    pub applied: usize,
    pub held_class_escalation: usize,
    pub below_confidence: usize,
    pub errors: usize,
    pub redactions: i64,
    pub producer: String,
    pub prompt_revision: String,
}

/// The reason a proposal was held rather than applied.
fn held_reason(from_class: &str, to_class: &str) -> String {
    format!("applying this would raise the mail from {from_class} to {to_class}")
}

/// Run one bounded pass over the fallback rows.
///
/// Operator-pressed. No timer: `quiet` records what an unattended local-model
/// drain cost, and the category axis writes a decision rather than a derived
/// field.
///
/// `limit` bounds how many threads one pass ACTS on — prompts in the first
/// loop, writes in the second — not how many are read: the staleness comparison
/// needs the `item_revision` and SQL cannot compute it, so the coarse candidate
/// query is answered first and the cap applied after.
///
/// Two loops, because a pass has two kinds of work. The first asks the threads
/// that are due. The second writes the threads a PREVIOUS pass already
/// answered: `is_due` calls a current `generated` verdict done, and it is right
/// — the question was asked and answered — but what is still outstanding on
/// such a row in apply mode is the write. Without the second loop the whole
/// documented rollout (measure in shadow, read the corpus, set
/// `mail_model.apply`, run apply) moved nothing at all, because every row it
/// wanted to move had stopped being due (review, 2026-09-05).
pub fn run_pass(
    cfg: &Config,
    store: &Store,
    mode: Mode,
    limit: usize,
    min_confidence_bp: i64,
) -> std::result::Result<PassReceipt, String> {
    let producer = producer(cfg);
    let mut receipt = PassReceipt {
        mode: mode.as_str().into(),
        producer: producer.clone(),
        prompt_revision: MAIL_MODEL_PROMPT_REVISION.into(),
        ..PassReceipt::default()
    };

    let candidates = store
        .model_rung_candidates()
        .map_err(|error| error.to_string())?;
    receipt.reviewed = candidates.len();

    let mut due: Vec<ModelCandidate> = Vec::new();
    let mut answered: Vec<ModelCandidate> = Vec::new();
    for candidate in candidates {
        if is_due(&candidate, &producer) {
            due.push(candidate);
        } else if awaits_apply(&candidate) {
            answered.push(candidate);
        }
    }
    receipt.eligible = due.len();

    for candidate in due.into_iter().take(limit) {
        let mut verdict = classify_one(cfg, &candidate);
        // Carried forward rather than restarted: a retryable failure
        // accumulates across passes, and anything else says the previous
        // failures are no longer the state of this row.
        if verdict.attempts > 0 {
            if let Some(stored) = &candidate.stored {
                verdict.attempts = stored.attempts.saturating_add(1);
            }
        }
        // A request was issued, whatever came back. Counted from the state
        // rather than beside each branch, so the receipt and the report's
        // `by_data_class` answer this question the same way.
        if was_prompted(&verdict.state) {
            receipt.prompted += 1;
        }
        match verdict.state.as_str() {
            "generated" => {}
            state if state == digest::LOCAL_REFUSED => receipt.refused_c3 += 1,
            state if state == digest::SKIPPED_OVER_WINDOW => receipt.over_window += 1,
            UNPARSEABLE => receipt.unparseable += 1,
            INVALID_STREAM => receipt.invalid_stream += 1,
            _ => receipt.errors += 1,
        }
        receipt.redactions += verdict.redactions;

        settle(store, &mut verdict, mode, min_confidence_bp, &mut receipt)?;
        store
            .upsert_model_verdict(&verdict)
            .map_err(|error| error.to_string())?;
    }

    // The rows an earlier pass already answered. Nothing here is prompted: the
    // stored verdict IS the answer, and asking again would spend a second
    // prompt to reach the same one. Read in one query rather than per row, the
    // way the reader contract does it — and not read at all when there is
    // nothing to read it for, because this is the one query in the pass that
    // returns the model's own sentences.
    if answered.is_empty() {
        return Ok(receipt);
    }
    let stored = store.model_verdicts().map_err(|error| error.to_string())?;
    let mut written = 0usize;
    for candidate in answered {
        let Some(verdict) = stored.get(&candidate.id) else {
            continue;
        };
        let Some(proposed) = verdict.model_stream.as_deref() else {
            continue;
        };
        if proposed == verdict.rule_stream {
            // Agreement leaves nothing outstanding, in either mode.
            continue;
        }
        receipt.awaiting_apply += 1;
        if mode != Mode::Apply || written >= limit {
            continue;
        }
        written += 1;
        let mut verdict = verdict.clone();
        settle(store, &mut verdict, mode, min_confidence_bp, &mut receipt)?;
        store
            .upsert_model_verdict(&verdict)
            .map_err(|error| error.to_string())?;
    }
    Ok(receipt)
}

/// What one verdict does to the category axis, and what the receipt records.
///
/// Written once because two loops decide it: the verdicts a prompt just
/// produced, and the stored ones an apply pass acts on without prompting. A
/// second copy of the escalation guard is how one of them would come to be
/// missing it.
fn settle(
    store: &Store,
    verdict: &mut ModelVerdict,
    mode: Mode,
    min_confidence_bp: i64,
    receipt: &mut PassReceipt,
) -> std::result::Result<(), String> {
    let Some(proposed) = verdict.model_stream.clone() else {
        return Ok(());
    };
    if proposed == verdict.rule_stream {
        // Re-stamping an unchanged verdict as method='model' would erase the
        // true fact that a rule decided it.
        receipt.agreed_no_write += 1;
        return Ok(());
    }
    receipt.disagreed += 1;
    let escalates = content_item::class_rank(&verdict.redaction_class)
        > content_item::class_rank(&verdict.data_class);
    if escalates {
        verdict.mode = "held".into();
        verdict.held_reason = Some(held_reason(&verdict.data_class, &verdict.redaction_class));
        receipt.held_class_escalation += 1;
    } else if mode == Mode::Apply {
        if verdict.confidence_bp.unwrap_or_default() < min_confidence_bp {
            receipt.below_confidence += 1;
        } else {
            let write = store
                .apply_model_stream(verdict)
                .map_err(|error| error.to_string())?;
            if write.stream_changed {
                verdict.mode = "applied".into();
                receipt.applied += 1;
            }
        }
    }
    Ok(())
}

/// Whether an apply pass could write this thread without prompting anything.
///
/// A stored verdict that is not due is either terminal or waiting out a
/// backoff. Only one of those shapes has an outstanding write: a `generated`
/// verdict still marked `shadow`. `held` is excluded deliberately — that is a
/// class-raising proposal a machine may not apply, and `apply_model_stream`
/// refuses it anyway.
fn awaits_apply(candidate: &ModelCandidate) -> bool {
    candidate
        .stored
        .as_ref()
        .is_some_and(|stored| stored.state == "generated" && stored.mode == Mode::Shadow.as_str())
}

/// Whether this thread is due to be asked.
///
/// One current producer, so the staleness check is an equality rather than a
/// membership test — the property the OverWindow decision buys. A verdict from
/// a different producer, a different prompt revision or a different item
/// revision is stale. A terminal verdict from the current one is done. A
/// retryable one waits out its backoff, three attempts at most.
fn is_due(candidate: &ModelCandidate, producer: &str) -> bool {
    let Some(stored) = &candidate.stored else {
        return true;
    };
    if stored.producer != producer
        || stored.prompt_revision != MAIL_MODEL_PROMPT_REVISION
        || stored.item_revision
            != item_revision(
                sender_domain(candidate.from_addr.as_deref().unwrap_or_default()).as_deref(),
                candidate.subject.as_deref().unwrap_or_default(),
                candidate.snippet.as_deref().unwrap_or_default(),
                &candidate.data_class,
            )
    {
        return true;
    }
    crate::store::RETRYABLE_MODEL_VERDICT_STATES.contains(&stored.state.as_str())
        && stored.attempts < crate::store::MAX_MODEL_VERDICT_ATTEMPTS
        && stored.backoff_expired
}

/// The agreement table the report and the CLI both print, built from summaries
/// that cannot carry mail content.
pub fn agreement(
    summaries: &[crate::store::ModelVerdictSummary],
) -> Vec<(String, usize, usize, HashMap<String, usize>)> {
    let mut by_rule: HashMap<String, (usize, usize, HashMap<String, usize>)> = HashMap::new();
    for summary in summaries {
        let entry = by_rule.entry(summary.rule_stream.clone()).or_default();
        entry.0 += 1;
        if let Some(proposed) = &summary.model_stream {
            if proposed == &summary.rule_stream {
                entry.1 += 1;
            }
            *entry.2.entry(proposed.clone()).or_default() += 1;
        }
    }
    let mut rows: Vec<(String, usize, usize, HashMap<String, usize>)> = by_rule
        .into_iter()
        .map(|(stream, (n, agree, streams))| (stream, n, agree, streams))
        .collect();
    rows.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_inference::InferenceConfig;

    /// Apple's on-device model: 4,096 tokens shared between prompt and reply.
    const APPLE_WINDOW: u32 = 4_096;

    /// A light role pointed at a closed port. Any request would fail fast; a
    /// test that reaches one has failed its own point.
    fn inference(light_window: Option<u32>) -> InferenceConfig {
        let mut roles = serde_json::Map::new();
        // Present and never resolved: `no_strong_role_is_ever_resolved` is what
        // this is here for.
        roles.insert(
            "summarization".into(),
            serde_json::json!({ "backend": "ollama", "model": "big-local-model" }),
        );
        if let Some(window) = light_window {
            roles.insert(
                quiet::LIGHT_ROLE.into(),
                serde_json::json!({
                    "backend": "foundation-models",
                    "model": "apple-on-device",
                    "max_input_tokens": window,
                }),
            );
        }
        serde_json::from_value(serde_json::json!({
            "backends": {
                "ollama": { "api": "ollama", "base_url": "http://127.0.0.1:9/v1" },
                "foundation-models": { "api": "openai", "base_url": "http://127.0.0.1:9/v1" },
            },
            "roles": roles,
        }))
        .expect("the probe config is well formed")
    }

    fn candidate(id: &str, data_class: &str) -> ModelCandidate {
        ModelCandidate {
            id: id.into(),
            from_addr: Some("Someone <a.person@Example.COM>".into()),
            subject: Some("Re: the thing".into()),
            snippet: Some("A short preview.".into()),
            data_class: data_class.into(),
            rule_stream: "aktiv".into(),
            rule_rationale: "No rule matched; kept active as the conservative default.".into(),
            rule_decided_by: "fallback".into(),
            stored: None,
        }
    }

    /// A Secret mail never reaches a prompt, and the row that records the
    /// refusal names the model that WOULD have run — provenance that says
    /// "unconfigured" over a configured machine is provenance that lies.
    #[test]
    fn c3_is_refused_before_a_target_is_built() {
        let cfg = Config::with_inference(inference(Some(APPLE_WINDOW)));
        let verdict = classify_one(&cfg, &candidate("thread:secret", "c3"));
        assert_eq!(verdict.state, digest::LOCAL_REFUSED);
        assert!(verdict.model_stream.is_none());
        assert!(verdict.rationale.is_none());
        assert_eq!(verdict.attempts, 0, "a refusal is a verdict, not a failure");
        assert!(
            verdict.producer.starts_with("foundation-models"),
            "got {}",
            verdict.producer
        );
        assert_eq!(verdict.data_class, "c3");
    }

    /// Over the window is terminal, and the strong role is not woken. This is
    /// the budget invariant that lets `summarize::ask` own only the reach.
    #[test]
    fn ask_is_only_reached_through_a_light_rung() {
        let cfg = Config::with_inference(inference(Some(APPLE_WINDOW)));
        let mut over = candidate("thread:huge", "c1");
        over.snippet = Some("x".repeat(40_000));
        let verdict = classify_one(&cfg, &over);
        assert_eq!(verdict.state, digest::SKIPPED_OVER_WINDOW);
        assert!(verdict.model_stream.is_none());
        assert_eq!(verdict.attempts, 0, "over the window is not retryable");
        assert!(verdict.next_attempt.is_none());
    }

    /// A machine with no light role has nothing to run this on, and says so
    /// rather than quietly meaning "use the big one".
    #[test]
    fn a_machine_without_a_light_role_is_unconfigured() {
        let cfg = Config::with_inference(inference(None));
        let verdict = classify_one(&cfg, &candidate("thread:none", "c1"));
        assert_eq!(verdict.state, "unconfigured");
        assert_eq!(verdict.producer, "unconfigured");
    }

    /// The property the module exists to keep, stated as a property: no source
    /// length resolves the strong local role. `quiet` owns the mechanism; this
    /// pins that this file uses it and adds no second path.
    #[test]
    fn no_strong_role_is_ever_resolved() {
        let source = include_str!("mail_model.rs");
        let body = source
            .split_once("#[cfg(test)]")
            .map_or(source, |(body, _)| body);
        for forbidden in ["summarization_role", "role_for"] {
            assert!(
                !body.contains(forbidden),
                "the rung must not reach {forbidden}"
            );
        }
    }

    #[test]
    fn the_prompt_carries_the_domain_and_never_the_address() {
        for from in [
            "a.person@Example.COM",
            "Some Person <a.person@example.com>",
            "\"Person, A.\" <A.Person@EXAMPLE.com>",
            "<a.person@example.com>",
        ] {
            assert_eq!(
                sender_domain(from).as_deref(),
                Some("example.com"),
                "{from}"
            );
            let built = prompt(sender_domain(from).as_deref(), "Subject", "Preview");
            assert!(built.contains("example.com"), "{from}");
            assert!(!built.contains("a.person"), "{from} leaked the local part");
            assert!(!built.to_lowercase().contains("some person"), "{from}");
        }
        assert_eq!(sender_domain("not-an-address"), None);
        // No sender line at all rather than a guess.
        assert!(!prompt(None, "Subject", "Preview").contains("Sender domain"));
    }

    #[test]
    fn the_prompt_names_every_stream_it_admits() {
        let built = prompt(Some("example.com"), "Subject", "Preview");
        for (name, definition) in rules::STREAM_DEFINITIONS {
            assert!(built.contains(name), "the prompt never names {name}");
            assert!(built.contains(definition.split(',').next().unwrap_or(definition)));
        }
    }

    #[test]
    fn the_answer_is_parsed_out_of_whatever_the_model_wrapped_it_in() {
        let expected = Answer {
            stream: "issue".into(),
            confidence_bp: 8_200,
            urgency_bp: 6_000,
            rationale: "It asks for a reply by Friday.".into(),
            urgency_rationale: "A date is named.".into(),
        };
        let object = r#"{"stream":"issue","confidence_bp":8200,"urgency_bp":6000,"rationale":"It asks for a reply by Friday.","urgency_rationale":"A date is named."}"#;
        for wrapped in [
            object.to_string(),
            format!("```\n{object}\n```"),
            format!("```json\n{object}\n```"),
            format!("Here is my answer:\n{object}"),
            format!("{object}\n\nI hope that helps."),
            format!("Sure!\n```json\n{object}\n```\nDone."),
            object.replace(
                r#""urgency_rationale""#,
                r#""extra_key": {"nested": 1}, "urgency_rationale""#,
            ),
            // A fence AFTER the answer, and a fence in the prose before it.
            // Both used to read as `unparseable`, which is retryable — so a
            // well-formed answer cost three prompts and ended with no proposal.
            format!("{object}\n```\nfor example\n```"),
            format!("I would call this an ```issue```.\n{object}"),
        ] {
            assert_eq!(parse(&wrapped), Ok(expected.clone()), "failed on {wrapped}");
        }
        assert_eq!(
            parse("I think this is active."),
            Err(ParseFailure::Unparseable)
        );
        assert_eq!(parse(""), Err(ParseFailure::Unparseable));
    }

    /// `last_error` was the one model-derived column that skipped
    /// `redact_review_field`, which contradicted the DDL comment on the table.
    /// Both values that reach it come from outside this machine's code: a word
    /// the model invented, and a message the local server returned.
    #[test]
    fn a_stored_error_is_capped_and_redacted_against_the_class() {
        let long = "Rechnung von Erika Mustermann, Konto DE89370400440532013000, faellig heute";
        let capped = stored_error(long, "c1");
        assert!(
            capped.chars().count() <= MAX_ERROR_CHARS,
            "got {} chars",
            capped.chars().count()
        );

        let redacted = stored_error(long, "c2");
        assert!(
            !redacted.contains("DE89370400440532013000"),
            "a c2 row stored the account verbatim: {redacted}"
        );
        assert!(
            redacted.chars().count() <= MAX_ERROR_CHARS,
            "got {redacted}"
        );
        // c3 is refused before a prompt is built, so the only way text reaches
        // this column on one is a transport failure — redacted the same way.
        assert!(!stored_error(long, "c3").contains("DE89370400440532013000"));
    }

    #[test]
    fn a_stream_outside_the_vocabulary_is_refused_not_stored() {
        assert_eq!(
            parse(r#"{"stream":"urgent","confidence_bp":9000}"#),
            Err(ParseFailure::InvalidStream("urgent".into()))
        );
        assert_eq!(
            ParseFailure::InvalidStream("urgent".into()).state(),
            INVALID_STREAM
        );
        // A missing stream is a shape failure rather than a vocabulary one, so
        // it retries and the vocabulary one does not.
        assert_eq!(
            parse(r#"{"confidence_bp":9000}"#),
            Err(ParseFailure::Unparseable)
        );
    }

    #[test]
    fn the_numbers_are_clamped_and_the_sentences_are_capped() {
        let answer = parse(&format!(
            r#"{{"stream":"feed","confidence_bp":99999,"urgency_bp":-5,"rationale":"{}"}}"#,
            "x".repeat(500)
        ))
        .unwrap();
        assert_eq!(answer.confidence_bp, 10_000);
        assert_eq!(answer.urgency_bp, 0);
        assert_eq!(answer.rationale.chars().count(), MAX_RATIONALE_CHARS);
        assert!(answer.urgency_rationale.is_empty());
    }

    /// The escape from redacting against a class that is about to change under
    /// the row. A verdict proposing `belege` on a Mine row is already about
    /// Others material, and it is written that way.
    #[test]
    fn a_rationale_is_redacted_against_the_class_the_proposal_implies() {
        assert_eq!(
            effective_class("c1", "belege", "billing@vendor.example", "Invoice"),
            "c2"
        );
        assert_eq!(
            effective_class("c1", "steuern", "amt@example.gov", "Bescheid"),
            "c2"
        );
        assert_eq!(
            effective_class("c1", "feed", "news@example.com", "Weekly"),
            "c1",
            "an ordinary proposal must not invent an escalation"
        );
        assert_eq!(
            effective_class("c2", "feed", "news@example.com", "Weekly"),
            "c2",
            "and must never lower one"
        );

        let mut redactions: Vec<RedactionFinding> = Vec::new();
        let redacted = redact_review_field(
            Some("Names the account DE89370400440532013000."),
            &mut redactions,
        )
        .unwrap();
        assert!(!redacted.contains("DE89370400440532013000"));
        assert!(!redactions.is_empty());
        assert!(content_item::redact_before_persistence("c2"));
        assert!(!content_item::redact_before_persistence("c1"));
    }

    #[test]
    fn every_stored_state_is_in_the_documented_set() {
        for outcome in [
            Outcome::Ok(String::new()),
            Outcome::SkippedShort,
            Outcome::RemoteRefused,
            Outcome::Unconfigured,
            Outcome::HttpError(String::new()),
            Outcome::ModelError(String::new()),
            Outcome::CapacityAborted(String::new()),
            Outcome::EmptyResponse,
            Outcome::Timeout,
        ] {
            assert!(
                MODEL_VERDICT_STATES.contains(&outcome.state()),
                "{} is reachable and undocumented",
                outcome.state()
            );
        }
        for own in [
            digest::LOCAL_REFUSED,
            digest::SKIPPED_OVER_WINDOW,
            UNPARSEABLE,
            INVALID_STREAM,
        ] {
            assert!(MODEL_VERDICT_STATES.contains(&own));
        }
        assert_eq!(
            MODEL_VERDICT_STATES.len(),
            13,
            "a state was added without updating the documented set"
        );
        for retryable in crate::store::RETRYABLE_MODEL_VERDICT_STATES {
            assert!(MODEL_VERDICT_STATES.contains(&retryable));
        }
        for terminal in [
            digest::LOCAL_REFUSED,
            digest::SKIPPED_OVER_WINDOW,
            INVALID_STREAM,
        ] {
            assert!(
                !crate::store::RETRYABLE_MODEL_VERDICT_STATES.contains(&terminal),
                "{terminal} is a verdict, not a transient failure"
            );
        }
    }

    /// A pure function over a constructed section, so it does not invert the day
    /// the operator sets the key on this machine, and it manipulates no
    /// environment variable.
    #[test]
    fn apply_is_refused_without_the_overlay_key() {
        let off = MailModelConfig::default();
        let on = MailModelConfig {
            apply: true,
            min_confidence_bp: 7_000,
            ..MailModelConfig::default()
        };
        assert_eq!(apply_allowed(None, Mode::Shadow), Ok(Mode::Shadow));
        assert_eq!(apply_allowed(Some(&off), Mode::Shadow), Ok(Mode::Shadow));
        assert_eq!(apply_allowed(Some(&on), Mode::Apply), Ok(Mode::Apply));

        let error = apply_allowed(None, Mode::Apply).expect_err("no section refuses apply");
        assert!(error.contains("mail_model"), "got {error}");
        let error = apply_allowed(Some(&off), Mode::Apply).expect_err("apply=false refuses");
        assert!(error.contains("mail_model.apply"), "got {error}");
    }

    /// Turning writing on without naming a floor writes every disagreement at
    /// any self-reported confidence, including zero — which is the opposite of
    /// what the key's own doc comment promises. The operator states the floor in
    /// the same edit that turns writing on.
    #[test]
    fn apply_is_refused_until_the_confidence_floor_is_named() {
        let no_floor = MailModelConfig {
            apply: true,
            min_confidence_bp: 0,
            ..MailModelConfig::default()
        };
        let error =
            apply_allowed(Some(&no_floor), Mode::Apply).expect_err("a zero floor refuses apply");
        assert!(error.contains("min_confidence_bp"), "got {error}");
        // Shadow is unaffected: it writes no category at all.
        assert_eq!(
            apply_allowed(Some(&no_floor), Mode::Shadow),
            Ok(Mode::Shadow)
        );

        let floor = MailModelConfig {
            apply: true,
            min_confidence_bp: 1,
            ..MailModelConfig::default()
        };
        assert_eq!(apply_allowed(Some(&floor), Mode::Apply), Ok(Mode::Apply));
    }

    /// The overlay key is the default, the caller wins over it, and neither can
    /// ask for an unbounded pass.
    #[test]
    fn the_overlay_limit_is_the_default_and_the_caller_overrides_it() {
        let section = MailModelConfig {
            limit: 25,
            ..MailModelConfig::default()
        };
        assert_eq!(pass_limit(Some(&section), None), 25);
        assert_eq!(pass_limit(Some(&section), Some(3)), 3);
        assert_eq!(pass_limit(None, None), DEFAULT_PASS_LIMIT);
        assert_eq!(pass_limit(None, Some(0)), 1);
        assert_eq!(pass_limit(None, Some(100_000)), MAX_PASS_LIMIT);
    }

    /// One current producer, so staleness is an equality. A stored verdict from
    /// this producer and this revision is done; anything else is asked again.
    #[test]
    fn a_stale_verdict_is_asked_again_and_a_current_one_is_not() {
        let mut fresh = candidate("thread:due", "c1");
        assert!(is_due(&fresh, "p"), "no verdict at all is always due");

        let current = crate::store::StoredVerdictState {
            producer: "p".into(),
            prompt_revision: MAIL_MODEL_PROMPT_REVISION.into(),
            item_revision: item_revision(
                Some("example.com"),
                "Re: the thing",
                "A short preview.",
                "c1",
            ),
            state: "generated".into(),
            mode: "shadow".into(),
            attempts: 0,
            backoff_expired: true,
        };
        fresh.stored = Some(current.clone());
        assert!(!is_due(&fresh, "p"));

        for stale in [
            crate::store::StoredVerdictState {
                producer: "another-model".into(),
                ..current.clone()
            },
            crate::store::StoredVerdictState {
                prompt_revision: "mail-stream-v0".into(),
                ..current.clone()
            },
            crate::store::StoredVerdictState {
                item_revision: "the subject changed".into(),
                ..current.clone()
            },
        ] {
            fresh.stored = Some(stale);
            assert!(is_due(&fresh, "p"));
        }

        // A retryable failure comes back once its backoff has expired, and
        // stops at the cap.
        fresh.stored = Some(crate::store::StoredVerdictState {
            state: "timeout".into(),
            attempts: 1,
            backoff_expired: true,
            ..current.clone()
        });
        assert!(is_due(&fresh, "p"));
        fresh.stored = Some(crate::store::StoredVerdictState {
            state: "timeout".into(),
            attempts: crate::store::MAX_MODEL_VERDICT_ATTEMPTS,
            backoff_expired: true,
            ..current.clone()
        });
        assert!(!is_due(&fresh, "p"), "the attempt cap is not advisory");
        fresh.stored = Some(crate::store::StoredVerdictState {
            state: "timeout".into(),
            attempts: 1,
            backoff_expired: false,
            ..current
        });
        assert!(!is_due(&fresh, "p"), "the backoff is not advisory either");
    }

    #[test]
    fn the_agreement_table_counts_matches_per_rule_stream() {
        let summary = |rule: &str, model: Option<&str>| crate::store::ModelVerdictSummary {
            triage_id: "id".into(),
            mode: "shadow".into(),
            state: "generated".into(),
            rule_stream: rule.into(),
            model_stream: model.map(str::to_string),
            confidence_bp: Some(9_000),
            urgency_bp: Some(1_000),
            data_class: "c1".into(),
            held_reason: None,
            producer: "p".into(),
            prompt_revision: MAIL_MODEL_PROMPT_REVISION.into(),
        };
        let rows = agreement(&[
            summary("aktiv", Some("aktiv")),
            summary("aktiv", Some("werbung")),
            summary("aktiv", None),
            summary("feed", Some("feed")),
        ]);
        assert_eq!(rows[0].0, "aktiv");
        assert_eq!((rows[0].1, rows[0].2), (3, 1));
        assert_eq!(rows[0].3["werbung"], 1);
        assert_eq!((rows[1].0.as_str(), rows[1].1, rows[1].2), ("feed", 1, 1));
    }
}
