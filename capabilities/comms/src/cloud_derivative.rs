//! Local preparation of bounded documents for the reviewed cloud-processing queue.
//! Nothing in this module performs network I/O. A preview must be reviewed and
//! its exact hash approved before the derivative can be staged in the store.
//!
//! Two rules live here, and they are the same rule read from both ends:
//! [`prepare`] decides what representation of an item may exist at all, and
//! [`tier_allows`] decides which provider tier may receive that representation.
//! `c2` and `c3` get `Err` from the first and `false` from the second — values
//! the derivative table's own CHECK no longer accepts either.
//!
//! Neither of them is the *class* policy any more, and neither of them keeps a
//! copy of it. That policy is `content_item::cloud_admission`: [`prepare`] asks
//! it through `content_item::has_cloud_lane` — is there any tier and any
//! representation for this class at all — and [`tier_allows`] asks it about the
//! one tier and representation in hand. The wire label the dashboard prints is
//! derived from the same function, so the contract a reader is shown and the
//! answer a dispatch gets can no longer disagree (T3). What is still comms' own
//! is the redaction transformation and its version pin.

use serde::Serialize;
use sha2::{Digest, Sha256};

pub const PREVIEW_SCHEMA_VERSION: &str = "cloud-derivative-preview-v1";
pub const REDACTION_VERSION: &str = "deterministic-entity-redaction-v3";
pub const PSEUDONYMIZE_VERSION: &str = sjel_pseudonymize::PSEUDONYMIZE_VERSION;
pub const PASSTHROUGH_VERSION: &str = "bounded-public-v1";
/// `entity_detection` of a [`prepare_pseudonymized`] preview.
pub const PSEUDONYMIZE_DETECTION: &str = "local-reversible-v1";
const MAX_DOCUMENT_CHARS: usize = 16_000;

#[derive(Debug, Clone)]
pub struct CloudDocumentInput {
    pub source: String,
    pub id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub data_class: String,
}

impl CloudDocumentInput {
    /// The cloud input for one stored feed item.
    ///
    /// The dashboard's own path builds this from `ContentItemOut` (which for a
    /// feed row carries exactly these fields, unchanged), and the drain has a
    /// `FeedItem` in hand and no reason to assemble a wire contract first. The
    /// two must produce identical fields or the same item would hash to two
    /// different `preview_hash` values and each would call the other stale —
    /// `a_drain_and_the_reader_prepare_the_same_document` pins that.
    pub fn from_feed(item: &crate::store::FeedItem) -> Self {
        Self {
            source: "feed".into(),
            id: item.id.clone(),
            title: item.title.clone(),
            author: item.author.clone(),
            summary: item.summary.clone(),
            content: item.transcript.clone(),
            data_class: item.data_class.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RedactionFinding {
    pub entity_type: &'static str,
    pub marker: &'static str,
    pub count: usize,
}

/// A local-only document has no cloud preview. `c2` and `c3` are both local-only
/// (Q27): one holds other people's facts, the other holds credentials.
///
/// Not a stricter preview, not a preview flagged ineligible: none at all. A
/// preview is a hashable, approvable object — the whole point of it is that a
/// human can sign the exact bytes and a queue can pin them. Producing one for
/// content that may never leave the machine means the refusal has to be
/// remembered again at every later step, and it was: staging checked the class,
/// queueing did not, and the preview handler happily returned the document
/// itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalOnlyRefused;

/// The refusal's exact wording, because three handlers in
/// `capabilities/comms/src/server/cloud.rs` recognise it by string: the error
/// crosses a `spawn_blocking` boundary as a `String` and the type does not
/// survive. A constant rather than a literal in four places, since a reworded
/// sentence would silently turn a 400 into a 500.
pub const LOCAL_ONLY_REFUSAL: &str =
    "c2 and c3 content has no cloud derivative and cannot be prepared for one";

impl std::fmt::Display for LocalOnlyRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(LOCAL_ONLY_REFUSAL)
    }
}

impl std::error::Error for LocalOnlyRefused {}

/// Whether a provider tier admits this exact original-plus-representation pair.
///
/// **A thin wrapper over `content_item::cloud_admission` (T3).** The class-level
/// policy is not comms' any more: `libs/content-item` is a leaf crate every
/// capability already depends on, and holding the rule here meant an advisory
/// label in the contract and a real gate in this file, free to disagree — which
/// they did, about what an unrecognized class meant.
///
/// What stays comms-owned is the one thing content-item cannot know: the
/// *transformation version*. Those revisions belong to the redactor below, so
/// the version pin is asked here and the class question is asked there. A tier
/// that accepts pseudonymized personal content accepts the redacted derivative
/// at [`REDACTION_VERSION`]; the same c1 document sent verbatim is a different
/// question with a different answer.
pub fn tier_allows(
    tier: Option<&str>,
    original_data_class: &str,
    derivative_data_class: &str,
    transformation: &str,
) -> bool {
    let representation_pinned = match original_data_class {
        "c0" => transformation == PASSTHROUGH_VERSION,
        // Both c1 transformations clear the same recall floor on the frozen corpus (PRD
        // §6.2, `comms-redaction-eval` and `--pseudonymized`). Reviewed analysis and digest
        // jobs use `PSEUDONYMIZE_VERSION`; destructive redaction remains available for stored
        // review fields and its separate evaluation mode.
        "c1" => transformation == REDACTION_VERSION || transformation == PSEUDONYMIZE_VERSION,
        // Unreachable behind the admission check below, which refuses every
        // other class outright. Written as a refusal anyway: this is the arm a
        // vocabulary change lands in, and it has landed in one already.
        _ => false,
    };
    representation_pinned
        && crate::content_item::cloud_admission(original_data_class, derivative_data_class, tier)
            .is_ok()
}

/// Whether an item of this stored class may be sent to a provider tier **as it
/// stands** — no redaction, no derivative, the source text itself.
///
/// The question both prefill paths ask, because both send exactly that: the
/// digest ladder in `digest.rs` and the feed-summary drain in `media.rs`. Naming
/// it once means neither of them spells out the passthrough argument itself, and
/// there is one place to read to know what "verbatim" is allowed to mean.
///
/// Two questions, both delegated: which classes may travel unchanged at all
/// (`content_item::verbatim_cloud_allowed` — `c0` and nothing else), and whether
/// this tier admits that class (`content_item::cloud_admission`). `c1` has a
/// cloud lane, but it runs through [`prepare`] and the redaction transformation.
pub fn verbatim_send_allowed(cloud_data_tier: Option<&str>, data_class: &str) -> bool {
    crate::content_item::verbatim_cloud_allowed(data_class)
        && crate::content_item::cloud_admission(data_class, data_class, cloud_data_tier).is_ok()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloudDerivativePreview {
    pub schema_version: &'static str,
    pub source: String,
    pub id: String,
    pub source_revision: String,
    pub preview_hash: String,
    pub original_data_class: String,
    pub derivative_data_class: String,
    pub transformation: &'static str,
    pub document: String,
    pub redaction_count: usize,
    pub redactions: Vec<RedactionFinding>,
    /// Q9b's receipt, or `None` on a call nothing was removed from. Carried on
    /// the object rather than composed by each reader, so the CLI, the dashboard
    /// and any later surface state the same thing about the same call.
    pub redaction_receipt: Option<String>,
    pub entity_detection: &'static str,
    pub truncated: bool,
    pub approval_required: bool,
    pub provider_calls: u8,
    pub limitations: Vec<&'static str>,
}

fn build_document(
    input: &CloudDocumentInput,
    mut transform: impl FnMut(&str, bool) -> String,
) -> String {
    let mut sections = Vec::with_capacity(4);
    if let Some(value) = input
        .title
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        sections.push(format!("Title\n{}", transform(value, false)));
    }
    if let Some(value) = input
        .author
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        sections.push(format!("Author\n{}", transform(value, true)));
    }
    for (heading, value) in [
        ("Summary", input.summary.as_deref()),
        ("Source content", input.content.as_deref()),
    ] {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            sections.push(format!("{heading}\n{}", transform(value, false)));
        }
    }
    sections.join("\n\n")
}

fn build_preview(
    input: &CloudDocumentInput,
    transformation: &'static str,
    derivative_data_class: &'static str,
    entity_detection: &'static str,
    document: String,
    redactions: Vec<RedactionFinding>,
    limitations: Vec<&'static str>,
) -> CloudDerivativePreview {
    let source_revision = source_revision(input);
    let (document, truncated) = bounded_chars(&document, MAX_DOCUMENT_CHARS);
    let preview_hash = digest(&[
        PREVIEW_SCHEMA_VERSION,
        &source_revision,
        transformation,
        derivative_data_class,
        &document,
    ]);
    let redaction_count = redactions.iter().map(|finding| finding.count).sum();
    let receipt = redaction_receipt(&redactions);
    CloudDerivativePreview {
        schema_version: PREVIEW_SCHEMA_VERSION,
        source: input.source.clone(),
        id: input.id.clone(),
        source_revision,
        preview_hash,
        original_data_class: input.data_class.clone(),
        derivative_data_class: derivative_data_class.into(),
        transformation,
        document,
        redaction_count,
        redactions,
        redaction_receipt: receipt,
        entity_detection,
        truncated,
        approval_required: true,
        provider_calls: 0,
        limitations,
    }
}

/// Build the reviewable, hashable derivative for one stored item.
///
/// `Err(LocalOnlyRefused)` for everything that is not `c0` or `c1`: there is no
/// representation of `c2` or `c3` this module is willing to hand to an approval
/// flow, so the refusal is the return type rather than a flag on an object that
/// already exists. The redaction path below therefore only ever describes `c1`
/// — the local-only classes used to fall through it and produce a fully
/// approvable preview whose only protection was that three separate later call
/// sites remembered to look at `original_data_class` again.
///
/// The class question is `content_item::has_cloud_lane`, not a list written out
/// here. It used to be `matches!(.., "c0" | "c1")`, which is the admission
/// policy restated in a second place — and this is the *first* gate every cloud
/// lane passes (stage, queue, enqueue all reach it before any tier is chosen),
/// so a copy that narrowed later than the policy would keep building an
/// approvable, hashable, stageable preview of a document with no lane left. The
/// answer is the same for every value live today; what changes is that there is
/// now one function to change.
///
/// It still fails closed on an unrecognized class, because `has_cloud_lane`
/// does: it finds no admitted `(derivative, tier)` pair for a value outside the
/// vocabulary, exactly as every other unknown-class decision in the contract
/// refuses — `class_rank` answers `None`, `processing_policy` answers c3's
/// policy, `DataClass::new` labels it Secret.
pub fn prepare(input: &CloudDocumentInput) -> Result<CloudDerivativePreview, LocalOnlyRefused> {
    if !crate::content_item::has_cloud_lane(&input.data_class) {
        return Err(LocalOnlyRefused);
    }
    let needs_redaction = input.data_class != "c0";
    let transformation = if needs_redaction {
        REDACTION_VERSION
    } else {
        PASSTHROUGH_VERSION
    };
    let derivative_data_class = if needs_redaction { "c1" } else { "c0" };
    let mut redactions = Vec::new();
    let document = build_document(input, |value, is_author| {
        if is_author && needs_redaction {
            record_redaction(&mut redactions, "identity", "[identity removed]");
            "[identity removed]".into()
        } else if is_author {
            value.trim().into()
        } else {
            transform_text(value, needs_redaction, &mut redactions)
        }
    });
    let limitations = if needs_redaction {
        vec![
            "Only the bounded reader document is included; attachments and linked pages are excluded.",
            "Local deterministic entity detection removes recognized people after a salutation or a self-introduction, a person named as being from an organisation, login handles, email addresses, links, phone or account numbers, and token-like secrets; unrecognized names and contextual clues may remain.",
            "Human review is required before this derivative becomes cloud-eligible.",
        ]
    } else {
        vec![
            "Only the bounded reader document is included; attachments and linked pages are excluded.",
            "Public classification permits cloud use but does not select a provider or send the document.",
        ]
    };
    Ok(build_preview(
        input,
        transformation,
        derivative_data_class,
        if needs_redaction {
            "local-deterministic-v3"
        } else {
            "not-required"
        },
        document,
        redactions,
        limitations,
    ))
}

/// A reversible derivative and the symbol table that reverses it.
///
/// The session is the only way back from `<TRAVELER_01>` to a name, so it travels with the
/// preview it produced rather than being threaded in by the caller. It is personal data
/// (C2 by construction, PRD §6.1) and has no `Serialize`: it stays on this host.
#[derive(Debug, Clone)]
pub struct PseudonymizedPreview {
    pub preview: CloudDerivativePreview,
    pub session: sjel_pseudonymize::PseudonymizerSession,
}

/// Build the reviewable, reversible pseudonymized derivative for one stored item.
///
/// Unlike destructive masking (`[person]`), this assigns typed tokens (`<TRAVELER_01>`,
/// `<EMAIL_01>`) so a cloud evaluator can reason over which person is which, and a reply can
/// be rehydrated locally (PRD §6.2c, Q112).
///
/// The same gates as [`prepare`], held to the same floor:
/// - **Class.** `c2` and `c3` get `Err(LocalOnlyRefused)`, the same refusal as [`prepare`]
///   (Q27). The tokens are reversible, so the refusal matters more here, not less.
/// - **Recall.** Measured on the frozen corpus by `comms-redaction-eval --pseudonymized`
///   against the ratified 91.7% floor (PRD §6.2). Rung 0 is `registry`; comms callers pass
///   [`crate::people_registry::entity_registry`], the same names the destructive path
///   consults through `is_known_person`.
/// - **Author.** The whole field becomes one `<SENDER_nn>` token, as the destructive path
///   writes `[identity removed]` for the whole field. Tokenizing it word by word left the
///   display name: `Alice <alice@example.com>` became `Alice <EMAIL_01>`.
/// - **Receipt (Q9b).** Every call starts a fresh session, so `redactions` and
///   `redaction_receipt` describe this call only, counted in occurrences like [`prepare`].
/// - **Hash.** Token numbers depend on session state. A fresh session per call makes the
///   same input produce the same document and the same `preview_hash`, which the
///   approve-then-requeue comparison needs.
pub fn prepare_pseudonymized(
    input: &CloudDocumentInput,
    registry: &sjel_pseudonymize::EntityRegistry,
) -> Result<PseudonymizedPreview, LocalOnlyRefused> {
    if !crate::content_item::has_cloud_lane(&input.data_class) {
        return Err(LocalOnlyRefused);
    }
    let needs_redaction = input.data_class != "c0";
    let transformation = if needs_redaction {
        PSEUDONYMIZE_VERSION
    } else {
        PASSTHROUGH_VERSION
    };
    let derivative_data_class = if needs_redaction { "c1" } else { "c0" };
    let mut session = sjel_pseudonymize::PseudonymizerSession::new();
    let document = build_document(input, |value, is_author| {
        let rendered = if is_author && needs_redaction {
            session.tokenize_whole(value, sjel_pseudonymize::EntityType::Identity)
        } else if needs_redaction {
            session.tokenize_text(value, registry)
        } else {
            value.trim().to_string()
        };
        rendered
    });

    // The comms receipt vocabulary, not the library's, so both transformations read the
    // same way to the person approving them.
    let redactions: Vec<RedactionFinding> = session
        .findings()
        .iter()
        .map(|finding| RedactionFinding {
            entity_type: finding.entity_type.as_str(),
            marker: finding.entity_type.marker(),
            count: finding.count,
        })
        .collect();

    let limitations = if needs_redaction {
        vec![
            "Only the bounded reader document is included; attachments and linked pages are excluded.",
            "Local reversible pseudonymization replaces the author field, people the operator's registry names, people after a salutation or a self-introduction, a person named as being from an organisation, login handles, email addresses, links, phone or account numbers, long numbers and token-like secrets with typed tokens; unrecognized names and contextual clues may remain.",
            "The token table stays on this host and restores the original values in a reply.",
            "Human review is required before this derivative becomes cloud-eligible.",
        ]
    } else {
        vec![
            "Only the bounded reader document is included; attachments and linked pages are excluded.",
            "Public classification permits cloud use but does not select a provider or send the document.",
        ]
    };

    let preview = build_preview(
        input,
        transformation,
        derivative_data_class,
        if needs_redaction {
            PSEUDONYMIZE_DETECTION
        } else {
            "not-required"
        },
        document,
        redactions,
        limitations,
    );
    Ok(PseudonymizedPreview { preview, session })
}

/// Run one stored review field — a subject, a snippet — through the same
/// deterministic entity detection the cloud preview uses.
///
/// Exposed separately because the cloud boundary is not the first boundary.
/// A `c3` mail's subject line can itself be the secret, so this also runs
/// before the row is written (see `intake`), and again when an already-stored
/// row is remediated. Same detector, same version string, three call sites —
/// a second implementation is how the two drift apart.
pub fn redact_review_field(
    value: Option<&str>,
    redactions: &mut Vec<RedactionFinding>,
) -> Option<String> {
    let value = value?;
    if value.trim().is_empty() {
        return Some(value.to_string());
    }
    Some(transform_text(value, true, redactions))
}

/// The sentence a human reads on a call that was reduced (PRD Q9b, answered
/// 2026-08-23: *every reduced call leaves a visible receipt*).
///
/// `None` when nothing was removed. A receipt on an untouched call is noise,
/// and worse, it trains the reader to stop reading the ones that matter.
///
/// **What this can honestly say, and what Q9b's example sentence could not.**
/// The ruling's wording is *"reduced 3 facts about 2 people before this call"*.
/// The second number is not in the data: [`record_redaction`] aggregates by
/// `entity_type`, so `person: 5` means five mentions were replaced and says
/// nothing about how many distinct people they were — the detector never learns
/// that, because knowing it would mean keeping the names. Counting occurrences
/// and calling them occurrences is the honest version, and it costs the sentence
/// nothing a reader needs.
///
/// Kinds are named in a fixed order rather than by descending count, so two
/// consecutive calls that removed the same things read the same way and a
/// difference in the sentence is a difference in what was removed. A kind the
/// table does not know is named by its own literal instead of being dropped:
/// the leading total sums every finding, so an omission would produce a receipt
/// whose breakdown does not add up to its own number.
pub fn redaction_receipt(findings: &[RedactionFinding]) -> Option<String> {
    let total: usize = findings.iter().map(|finding| finding.count).sum();
    if total == 0 {
        return None;
    }

    // Every entity_type transform_text and stage_identity can record, in the
    // order a reader cares about: who, then how to reach them, then what
    // unlocks something. A kind missing from this table would be dropped from
    // the sentence silently, so the test below asserts the table is complete.
    const KINDS: [(&str, &str, &str); 9] = [
        ("person", "mention of a person", "mentions of people"),
        ("identity", "identity", "identities"),
        // Only the reversible path records places (a registry of places, PRD §6.2c).
        ("place", "mention of a place", "mentions of places"),
        ("email", "email address", "email addresses"),
        ("phone_number", "phone number", "phone numbers"),
        ("financial_identifier", "account number", "account numbers"),
        ("secret_token", "token-like secret", "token-like secrets"),
        ("long_number", "long number", "long numbers"),
        ("link", "link", "links"),
    ];

    let mut parts: Vec<String> = Vec::new();
    for (kind, one, many) in KINDS {
        let count: usize = findings
            .iter()
            .filter(|finding| finding.entity_type == kind)
            .map(|finding| finding.count)
            .sum();
        match count {
            0 => {}
            1 => parts.push(format!("1 {one}")),
            n => parts.push(format!("{n} {many}")),
        }
    }

    // A kind this table does not know is named by its own literal rather than
    // dropped. The count at the front of the sentence is the sum over ALL
    // findings, so a silently omitted kind would make the total disagree with
    // the breakdown — a receipt that does not add up is worse than none.
    for finding in findings {
        if KINDS
            .iter()
            .any(|(kind, _, _)| *kind == finding.entity_type)
        {
            continue;
        }
        parts.push(format!("{} {}", finding.count, finding.entity_type));
    }

    let detail = match parts.len() {
        0 => return None,
        1 => parts.remove(0),
        _ => {
            let last = parts.pop().unwrap_or_default();
            format!("{} and {last}", parts.join(", "))
        }
    };
    let noun = if total == 1 { "detail" } else { "details" };
    Some(format!(
        "Reduced {total} {noun} before this call: {detail}."
    ))
}

/// Stable identifier for what a redaction pass did, without carrying any of
/// the material it removed. Safe to log, store and show.
pub fn redaction_digest(findings: &[RedactionFinding]) -> String {
    let mut parts: Vec<String> = findings
        .iter()
        .map(|finding| format!("{}:{}", finding.entity_type, finding.count))
        .collect();
    parts.sort();
    let joined = parts.join(",");
    digest(&[REDACTION_VERSION, &joined])
}

pub fn source_revision(input: &CloudDocumentInput) -> String {
    digest(&[
        &input.source,
        &input.id,
        input.title.as_deref().unwrap_or_default(),
        input.author.as_deref().unwrap_or_default(),
        input.summary.as_deref().unwrap_or_default(),
        input.content.as_deref().unwrap_or_default(),
        &input.data_class,
    ])
}

/// Rung 1 and rung 2 of PRD §6.2, destructive form: each recognised word becomes a fixed
/// marker. The detectors are `sjel_pseudonymize::pattern` — the same functions the
/// reversible path calls — so the two transformations cannot disagree about what a phone
/// number or a self-introduction looks like. Their rationale (the D14 gaps, the `i'm`/`im`
/// apostrophe, the gated handle rule) is documented there.
fn transform_text(value: &str, redact: bool, redactions: &mut Vec<RedactionFinding>) -> String {
    use sjel_pseudonymize::pattern::{
        introduces_person, looks_like_email, looks_like_handle, looks_like_iban,
        looks_like_person_name, looks_like_phone, looks_like_sensitive_number, looks_like_token,
        looks_like_url, names_a_person_in_apposition,
    };
    if !redact {
        return value.trim().to_string();
    }

    let tokens = value.split_whitespace().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(tokens.len());
    let mut redact_person_tail = false;

    for (index, token) in tokens.iter().copied().enumerate() {
        let lowered = token.to_ascii_lowercase();
        // A cue reads backwards over the token slice rather than carrying a flag
        // forward, because "I am X" and "my name is X" are two tokens wide and a
        // one-token flag cannot see the word before the one that set it.
        let cued = redact_person_tail || introduces_person(&tokens, index);
        let finding = if looks_like_url(&lowered) {
            Some(("link", "[link]"))
        } else if looks_like_email(token) {
            Some(("email", "[email]"))
        } else if looks_like_iban(token) {
            Some(("financial_identifier", "[account]"))
        } else if looks_like_phone(token) {
            Some(("phone_number", "[phone]"))
        } else if looks_like_token(token) {
            Some(("secret_token", "[token]"))
        } else if looks_like_sensitive_number(token) {
            Some(("long_number", "[number]"))
        } else if crate::people_registry::is_known_person(token) {
            // Rung 0. The salutation gate below cannot see a bare first name, and
            // across 353 Journal notes it caught 2 of 2,802 known-person mentions.
            // A list the operator wrote by hand beats a heuristic here, so it is
            // consulted first.
            Some(("person", "[person]"))
        } else if names_a_person_in_apposition(&tokens, index) {
            Some(("person", "[person]"))
        } else if cued && (looks_like_person_name(token) || looks_like_handle(token)) {
            // A handle is only a person behind a cue. Ungated, `looks_like_handle`
            // fires on every "iPhone15" and order code in the corpus, which is the
            // over-redaction the Presidio trial rejected a whole runtime for.
            Some(("person", "[person]"))
        } else {
            None
        };

        if let Some((entity_type, marker)) = finding {
            record_redaction(redactions, entity_type, marker);
            output.push(marker);
            redact_person_tail = entity_type == "person";
        } else {
            output.push(token);
            redact_person_tail = false;
        }
    }

    output.join(" ")
}

fn record_redaction(
    redactions: &mut Vec<RedactionFinding>,
    entity_type: &'static str,
    marker: &'static str,
) {
    if let Some(finding) = redactions
        .iter_mut()
        .find(|finding| finding.entity_type == entity_type)
    {
        finding.count += 1;
    } else {
        redactions.push(RedactionFinding {
            entity_type,
            marker,
            count: 1,
        });
    }
}

fn bounded_chars(value: &str, limit: usize) -> (String, bool) {
    let mut chars = value.chars();
    let bounded: String = chars.by_ref().take(limit).collect();
    let truncated = chars.next().is_some();
    (bounded, truncated)
}

fn digest(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    // ── Q9b: every reduced call leaves a visible receipt ───────────────

    fn finding(entity_type: &'static str, count: usize) -> RedactionFinding {
        RedactionFinding {
            entity_type,
            marker: "[x]",
            count,
        }
    }

    #[test]
    fn an_untouched_call_gets_no_receipt() {
        assert_eq!(redaction_receipt(&[]), None);
        assert_eq!(redaction_receipt(&[finding("person", 0)]), None);
    }

    #[test]
    fn the_receipt_counts_occurrences_and_says_so() {
        // Q9b's own wording is "3 facts about 2 people". The second number is
        // not in the data and cannot be, so the sentence claims occurrences.
        let one = redaction_receipt(&[finding("person", 1)]).unwrap();
        assert_eq!(
            one,
            "Reduced 1 detail before this call: 1 mention of a person."
        );
        let many = redaction_receipt(&[finding("person", 5)]).unwrap();
        assert_eq!(
            many,
            "Reduced 5 details before this call: 5 mentions of people."
        );
    }

    #[test]
    fn kinds_read_in_a_fixed_order_whatever_order_they_were_found_in() {
        let a = redaction_receipt(&[finding("link", 2), finding("person", 1)]).unwrap();
        let b = redaction_receipt(&[finding("person", 1), finding("link", 2)]).unwrap();
        assert_eq!(a, b, "the same removals must read the same way");
        assert_eq!(
            a,
            "Reduced 3 details before this call: 1 mention of a person and 2 links."
        );
        let three = redaction_receipt(&[
            finding("email", 1),
            finding("person", 2),
            finding("link", 1),
        ])
        .unwrap();
        assert_eq!(
            three,
            "Reduced 4 details before this call: 2 mentions of people, 1 email address and 1 link."
        );
    }

    #[test]
    fn the_breakdown_always_adds_up_to_the_total() {
        // The failure this rules out: a new entity_type reaching the recorder
        // and being dropped from the sentence, leaving a receipt whose parts do
        // not sum to the number in front of them.
        let receipt =
            redaction_receipt(&[finding("person", 2), finding("something_new_entirely", 3)])
                .unwrap();
        assert!(receipt.starts_with("Reduced 5 details"), "{receipt}");
        assert!(receipt.contains("something_new_entirely"), "{receipt}");
    }

    #[test]
    fn every_kind_the_detector_records_is_named_in_words() {
        // The left column is every entity_type transform_text and
        // stage_identity can produce today; the right is the phrase a human
        // reads. A new kind reaching the recorder without a row here falls
        // through to its own literal, which the previous test allows and this
        // one is the reminder to fix.
        for (kind, phrase) in [
            ("person", "1 mention of a person"),
            ("identity", "1 identity"),
            ("place", "1 mention of a place"),
            ("email", "1 email address"),
            ("phone_number", "1 phone number"),
            ("financial_identifier", "1 account number"),
            ("secret_token", "1 token-like secret"),
            ("long_number", "1 long number"),
            ("link", "1 link"),
        ] {
            let receipt = redaction_receipt(&[finding(kind, 1)]).unwrap();
            assert_eq!(
                receipt,
                format!("Reduced 1 detail before this call: {phrase}."),
                "{kind}"
            );
        }
    }

    #[test]
    fn a_reduced_preview_carries_its_receipt_and_a_public_one_does_not() {
        let input = CloudDocumentInput {
            source: "mail".into(),
            id: "1".into(),
            title: Some("Hi from Herr Müller".into()),
            author: None,
            summary: None,
            content: Some("write to a@b.de".into()),
            data_class: "c1".into(),
        };
        let preview = prepare(&input).expect("c1 has a cloud lane");
        assert!(
            preview.redaction_count > 0,
            "the fixture must redact something"
        );
        let receipt = preview
            .redaction_receipt
            .as_deref()
            .expect("a reduced call carries a receipt");
        assert!(receipt.starts_with("Reduced "), "{receipt}");
    }

    use super::*;

    fn input(data_class: &str) -> CloudDocumentInput {
        CloudDocumentInput {
            source: "mail".into(),
            id: "thread-1".into(),
            title: Some("Security code 123456".into()),
            author: Some("Alice <alice@example.com>".into()),
            summary: None,
            content: Some(
                "Open https://example.com/a with token AbCd1234567890Ef or email alice@example.com"
                    .into(),
            ),
            data_class: data_class.into(),
        }
    }

    #[test]
    fn a_c1_preview_redacts_obvious_identifiers_without_provider_calls() {
        let preview = prepare(&input("c1")).unwrap();
        assert_eq!(preview.derivative_data_class, "c1");
        assert_eq!(preview.transformation, REDACTION_VERSION);
        assert!(preview.document.contains("[identity removed]"));
        assert!(preview.document.contains("[number]"));
        assert!(preview.document.contains("[link]"));
        assert!(preview.document.contains("[token]"));
        assert!(!preview.document.contains("alice@example.com"));
        assert_eq!(preview.entity_detection, "local-deterministic-v3");
        assert!(preview
            .redactions
            .iter()
            .any(|finding| finding.entity_type == "email"));
        assert_eq!(preview.provider_calls, 0);
    }

    #[test]
    fn a_c0_preview_is_bounded_but_not_pseudonymized() {
        let preview = prepare(&input("c0")).unwrap();
        assert_eq!(preview.derivative_data_class, "c0");
        assert_eq!(preview.transformation, PASSTHROUGH_VERSION);
        assert!(preview.document.contains("alice@example.com"));
        assert_eq!(preview.redaction_count, 0);
        assert!(preview.redactions.is_empty());
        assert_eq!(preview.entity_detection, "not-required");
    }

    #[test]
    fn preview_hash_changes_with_the_source() {
        let first = prepare(&input("c1")).unwrap();
        let mut changed = input("c1");
        changed.content = Some("different".into());
        let second = prepare(&changed).unwrap();
        assert_ne!(first.source_revision, second.source_revision);
        assert_ne!(first.preview_hash, second.preview_hash);
    }

    #[test]
    fn local_entity_detection_reports_people_phone_and_financial_identifiers() {
        let mut value = input("c1");
        value.content =
            Some("Hello Alice Example, call +49-170-1234567 or use DE89370400440532013000.".into());

        let preview = prepare(&value).unwrap();
        assert!(preview.document.contains("Hello [person] [person]"));
        assert!(preview.document.contains("[phone]"));
        assert!(preview.document.contains("[account]"));
        assert!(preview
            .redactions
            .iter()
            .any(|finding| finding.entity_type == "person" && finding.count == 2));
    }

    /// PRD D14, gap one. Three of the four labels Presidio caught and the
    /// incumbent missed were English self-introductions in the body of a mail,
    /// where a salutation-only gate has nothing to fire on. The entity form is
    /// the one the corpus actually holds: stored mail reaches this module with
    /// `&#39;` unresolved, so a matcher written against a bare apostrophe would
    /// pass this test and still miss every real mail.
    #[test]
    fn a_self_introduction_names_a_person_without_any_salutation() {
        for phrasing in [
            "Thanks for signing up. I&#39;m Josh, one of the co-founders.",
            "Thanks for signing up. I'm Josh, one of the co-founders.",
            "Thanks for signing up. I am Josh, one of the co-founders.",
            "Thanks for signing up. My name is Josh, one of the co-founders.",
            "Thanks for signing up. This is Josh, one of the co-founders.",
        ] {
            let mut value = input("c1");
            value.content = Some(phrasing.into());
            let preview = prepare(&value).unwrap();
            assert!(
                !preview.document.contains("Josh"),
                "self-introduction leaked the name: {phrasing}"
            );
        }
    }

    /// The negative that pays for the rule above. German `im` reduces to the
    /// same letters as the English cue `i'm`, and German capitalises every noun,
    /// so a cue test that dropped the apostrophe would redact a noun after every
    /// `im` in half the corpus. The corpus's German person recall was already the
    /// higher of the two; this rule must not spend that.
    #[test]
    fn the_german_preposition_im_is_not_a_self_introduction() {
        let mut value = input("c1");
        value.content = Some("Die Unterlagen finden Sie im Anhang dieser Nachricht.".into());

        let preview = prepare(&value).unwrap();
        assert!(preview.document.contains("im Anhang"));
    }

    /// PRD D14, gap two. `labo2764` passes every other recognizer here — too
    /// short for `looks_like_token`, too few digits for
    /// `looks_like_sensitive_number`, lowercase so not a name — and it identifies
    /// its owner as surely as the name on the account.
    #[test]
    fn a_login_handle_after_a_salutation_is_a_person() {
        let mut value = input("c1");
        value.content = Some("Hallo labo2764, wir haben eine neue Login-Aktivitaet.".into());

        let preview = prepare(&value).unwrap();
        assert!(!preview.document.contains("labo2764"));
        assert!(preview
            .redactions
            .iter()
            .any(|finding| finding.entity_type == "person"));
    }

    /// The handle rule is reachable only behind a cue. Ungated it fires on every
    /// product name and order code in the corpus, which is the over-redaction the
    /// Presidio trial rejected a whole Python runtime for.
    #[test]
    fn a_handle_shaped_word_with_no_cue_before_it_stays() {
        let mut value = input("c1");
        value.content = Some("Your order for the iPhone15 ships on Tuesday.".into());

        let preview = prepare(&value).unwrap();
        assert!(preview.document.contains("iPhone15"));
    }

    /// PRD D14, gap one again, in its other shape: a person named by the
    /// organisation they are from, mid-sentence, with no cue in front.
    #[test]
    fn a_person_named_as_being_from_an_organisation_is_redacted() {
        let mut value = input("c1");
        value.content = Some("A workshop co-hosted by Rayn from Scriptbee this Thursday.".into());

        let preview = prepare(&value).unwrap();
        assert!(!preview.document.contains("Rayn"));
        assert!(preview.document.contains("[person] from"));
    }

    /// The apposition needs a proper noun on both sides. `from` followed by a
    /// lowercase word is the ordinary preposition and must cost nothing.
    #[test]
    fn from_followed_by_a_common_noun_is_not_an_apposition() {
        let mut value = input("c1");
        value.content = Some("Download the report from our website before Friday.".into());

        let preview = prepare(&value).unwrap();
        assert!(preview
            .document
            .contains("Download the report from our website"));
    }

    /// This test used to prepare a local-only document and assert on the
    /// redacted preview it got back. That preview was a real, hashable,
    /// approvable object carrying the document itself; only the fact that three
    /// later call sites re-checked `original_data_class` kept it out of a cloud
    /// request. The refusal is the return type now, so there is nothing to
    /// approve — for c3, and for c2 as well since Q27 split them.
    #[test]
    fn local_only_content_has_no_approvable_preview_at_all() {
        for class in ["c2", "c3"] {
            assert_eq!(prepare(&input(class)), Err(LocalOnlyRefused));
        }

        let mut with_secrets = input("c3");
        with_secrets.content =
            Some("Hello Alice Example, call +49-170-1234567 or use DE89370400440532013000.".into());
        assert_eq!(prepare(&with_secrets), Err(LocalOnlyRefused));
    }

    /// The gate is "does this class have any cloud lane", so a value from
    /// outside the vocabulary is refused too — `has_cloud_lane` finds no
    /// admitted pair for it. This is the case a positive `c2 | c3` match got
    /// wrong: the class arrives as a stored string, and the previous vocabulary
    /// is still readable in backups, in a stale queued job and in any caller
    /// written against it.
    #[test]
    fn a_class_from_outside_the_vocabulary_is_refused_rather_than_prepared() {
        for unknown in ["vault", "personal", "private", "C1", "", "c4"] {
            assert_eq!(
                prepare(&input(unknown)),
                Err(LocalOnlyRefused),
                "{unknown} produced an approvable preview"
            );
        }
    }

    /// `prepare` refuses exactly the classes the policy gives no lane, because
    /// it is now the same question. Derived from `has_cloud_lane` rather than
    /// listed, so this is a statement about delegation and not a second copy of
    /// the answer it delegates.
    #[test]
    fn prepare_refuses_exactly_the_classes_with_no_cloud_lane() {
        for class in ["c0", "c1", "c2", "c3", "vault", "personal", "c4", ""] {
            assert_eq!(
                prepare(&input(class)).is_ok(),
                crate::content_item::has_cloud_lane(class),
                "{class}: prepare and the admission policy disagree about a lane"
            );
        }
    }

    /// `libs/inference` owns the tier vocabulary (Q26): a tier is what an
    /// operator reviewed one provider to receive, and a role is where that
    /// declaration lives, so `sjel_inference::CloudDataTier` is the original and
    /// its serde form is the config spelling.
    ///
    /// `libs/content-item` carries a copy in `CLOUD_DATA_TIERS` because it is
    /// the leaf crate every capability already depends on, and the admission
    /// policy that lives there cannot name the tiers it admits without them —
    /// depending back on inference would invert that. `comms` depends on both
    /// crates, so `comms` is the only place the copy can be pinned to the
    /// original.
    ///
    /// What this catches: rename a tier in `libs/inference` and
    /// `cloud_admission` goes on matching a string no role can declare. Every
    /// c1 dispatch stops, while `processing_policy` keeps printing
    /// `pseudonymization_required` to the dashboard — the label says a lane
    /// exists and the gate says none, which is the split T3 exists to end, one
    /// level up at the tier instead of the class.
    #[test]
    fn the_tier_vocabulary_is_spelled_the_same_way_in_both_crates() {
        use sjel_inference::CloudDataTier;
        let declared = [CloudDataTier::Public, CloudDataTier::PseudonymizedPersonal];
        // Exhaustive and armless on purpose: a variant added to
        // `libs/inference` stops this matching, which is the only signal a
        // leaf-crate copy can be given that the vocabulary grew. The array
        // above and `CLOUD_DATA_TIERS` then have to grow with it.
        for tier in declared {
            match tier {
                CloudDataTier::Public | CloudDataTier::PseudonymizedPersonal => {}
            }
        }
        assert_eq!(
            declared.map(CloudDataTier::as_str).as_slice(),
            crate::content_item::CLOUD_DATA_TIERS.as_slice(),
            "content-item's tier copy drifted from the vocabulary libs/inference owns"
        );
    }

    #[test]
    fn cloud_tiers_accept_only_the_exact_reviewed_representation() {
        assert!(tier_allows(Some("public"), "c0", "c0", PASSTHROUGH_VERSION));
        assert!(!tier_allows(Some("public"), "c1", "c1", REDACTION_VERSION));
        assert!(tier_allows(
            Some("pseudonymized_personal"),
            "c1",
            "c1",
            REDACTION_VERSION,
        ));
        assert!(!tier_allows(
            Some("pseudonymized_personal"),
            "c1",
            "c1",
            PASSTHROUGH_VERSION,
        ));
        for local_only in ["c2", "c3"] {
            assert!(!tier_allows(
                Some("pseudonymized_personal"),
                local_only,
                "c1",
                REDACTION_VERSION,
            ));
        }
    }

    /// The property both prefill paths depend on — `digest.rs` and the feed
    /// summary drain in `media.rs` — because both hand the model the item's own
    /// text with nothing removed. No configured tier admits that for anything
    /// but `c0`, and a role with no declared tier admits nothing at all: that
    /// covers every local role, and every https endpoint somebody points a
    /// summarization role at without a reviewed cloud policy on it.
    #[test]
    fn nothing_but_c0_may_be_sent_verbatim_and_an_undeclared_tier_sends_nothing() {
        for tier in [None, Some("public"), Some("pseudonymized_personal")] {
            for class in ["c1", "c2", "c3", "something-new"] {
                assert!(
                    !verbatim_send_allowed(tier, class),
                    "{tier:?} admitted {class} verbatim"
                );
            }
        }
        assert!(!verbatim_send_allowed(None, "c0"));
        assert!(verbatim_send_allowed(Some("public"), "c0"));
        assert!(verbatim_send_allowed(Some("pseudonymized_personal"), "c0"));
    }

    /// A c1 item has a cloud lane, and it is not this one. Worth its own
    /// assertion because the pseudonymized tier's whole purpose is personal
    /// content, and reading the tier name alone would suggest it applies here.
    #[test]
    fn a_c1_item_reaches_cloud_only_as_a_redacted_derivative() {
        assert!(!verbatim_send_allowed(Some("pseudonymized_personal"), "c1"));
        assert!(tier_allows(
            Some("pseudonymized_personal"),
            "c1",
            "c1",
            REDACTION_VERSION
        ));
    }

    fn pseudonymized_input() -> CloudDocumentInput {
        CloudDocumentInput {
            source: "mail".into(),
            id: "msg-123".into(),
            title: Some("Project update from Alice".into()),
            author: Some("Alice <alice@example.com>".into()),
            summary: Some("Call +49-170-1234567 regarding invoice".into()),
            content: Some("Transfer funds to DE89370400440532013000".into()),
            data_class: "c1".into(),
        }
    }

    fn alice_registry() -> sjel_pseudonymize::EntityRegistry {
        sjel_pseudonymize::EntityRegistry::builder()
            .add_person("Alice")
            .build()
    }

    #[test]
    fn prepare_pseudonymized_preserves_relational_tokens_and_receipt() {
        let PseudonymizedPreview { preview, session } =
            prepare_pseudonymized(&pseudonymized_input(), &alice_registry())
                .expect("c1 input produces pseudonymized derivative");

        assert_eq!(preview.transformation, PSEUDONYMIZE_VERSION);
        assert_eq!(preview.derivative_data_class, "c1");
        assert_eq!(preview.entity_detection, PSEUDONYMIZE_DETECTION);
        // The author field is one identity token, never "Alice <EMAIL_01>".
        assert!(
            preview.document.contains("Author\n<SENDER_01>"),
            "{}",
            preview.document
        );
        assert!(!preview.document.contains("Alice"), "{}", preview.document);
        assert!(!preview.document.contains("alice@example.com"));
        assert!(preview.document.contains("<TRAVELER_01>"));
        assert!(preview.document.contains("<PHONE_01>"));
        assert!(preview.document.contains("<ACCOUNT_01>"));
        assert!(preview.redaction_receipt.is_some());
        assert_eq!(
            session.rehydrate_text("<SENDER_01>"),
            "Alice <alice@example.com>"
        );
        assert!(tier_allows(
            Some("pseudonymized_personal"),
            "c1",
            "c1",
            PSEUDONYMIZE_VERSION
        ));
    }

    /// Q27 on the reversible path: the refusal is the return type here too, and no tier
    /// admits a local-only class under the reversible transformation.
    #[test]
    fn the_reversible_path_refuses_c2_and_c3_like_the_destructive_one() {
        for class in ["c2", "c3"] {
            let mut value = pseudonymized_input();
            value.data_class = class.into();
            assert!(
                matches!(
                    prepare_pseudonymized(&value, &alice_registry()),
                    Err(LocalOnlyRefused)
                ),
                "{class} produced a reversible preview"
            );
            for tier in [None, Some("public"), Some("pseudonymized_personal")] {
                for derivative in ["c0", "c1", class] {
                    assert!(
                        !tier_allows(tier, class, derivative, PSEUDONYMIZE_VERSION),
                        "{tier:?} admitted {class} as {derivative} under {PSEUDONYMIZE_VERSION}"
                    );
                }
            }
        }
        assert!(!tier_allows(
            Some("public"),
            "c1",
            "c1",
            PSEUDONYMIZE_VERSION
        ));
    }

    /// The approve-then-requeue check compares a fresh preparation's hash to the approved
    /// one. That only works when preparing twice gives the same bytes, and the receipt must
    /// describe one call, not the sum of every call before it (Q9b).
    #[test]
    fn a_reversible_preview_hashes_the_same_way_twice_and_its_receipt_is_per_call() {
        let registry = alice_registry();
        let first = prepare_pseudonymized(&pseudonymized_input(), &registry).unwrap();
        let second = prepare_pseudonymized(&pseudonymized_input(), &registry).unwrap();
        assert_eq!(first.preview.preview_hash, second.preview.preview_hash);
        assert_eq!(first.preview.document, second.preview.document);
        assert_eq!(
            first.preview.redaction_receipt,
            second.preview.redaction_receipt
        );
        assert_eq!(
            first.preview.redaction_count,
            second.preview.redaction_count
        );
    }

    /// Occurrences, like the destructive receipt says: two mentions of the same person are
    /// two, not one distinct entity.
    #[test]
    fn a_reversible_receipt_counts_occurrences() {
        let mut value = pseudonymized_input();
        value.title = None;
        value.author = None;
        value.summary = None;
        value.content = Some("Alice said Alice would call.".into());
        let preview = prepare_pseudonymized(&value, &alice_registry())
            .unwrap()
            .preview;
        assert_eq!(preview.redaction_count, 2);
        assert_eq!(
            preview.redaction_receipt.as_deref(),
            Some("Reduced 2 details before this call: 2 mentions of people.")
        );
        assert_eq!(preview.redactions[0].marker, "<TRAVELER_nn>");
    }

    /// The four probes the reversible path leaked before both paths shared
    /// `sjel_pseudonymize::pattern`. Both transformations must remove each of them.
    #[test]
    fn both_paths_remove_numbers_written_with_slashes_dots_or_a_hash() {
        for (text, secret) in [
            ("Ref #1234567", "1234567"),
            ("Tel. 0721/1234567", "1234567"),
            ("Konto 12.345.678", "12.345.678"),
            ("geb. 01.02.1990", "01.02.1990"),
        ] {
            let mut value = pseudonymized_input();
            value.content = Some(text.into());
            let destructive = prepare(&value).unwrap();
            assert!(
                !destructive.document.contains(secret),
                "prepare leaked {text}"
            );
            let reversible = prepare_pseudonymized(&value, &alice_registry())
                .unwrap()
                .preview;
            assert!(
                !reversible.document.contains(secret),
                "prepare_pseudonymized leaked {text}"
            );
        }
    }

    /// Serialized, a reversible preview stays inside `schemas/cloud-derivative-preview.schema.json`:
    /// the transformation, the detection label and the marker shape are values it lists.
    #[test]
    fn a_reversible_preview_uses_values_the_preview_schema_lists() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../schemas/cloud-derivative-preview.schema.json"
        ))
        .unwrap();
        let listed = |property: &str, value: &str| {
            schema["properties"][property]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == value)
        };
        let preview = prepare_pseudonymized(&pseudonymized_input(), &alice_registry())
            .unwrap()
            .preview;
        assert!(listed("transformation", preview.transformation));
        assert!(listed("entity_detection", preview.entity_detection));
        assert!(listed("transformation", REDACTION_VERSION));
        for finding in &preview.redactions {
            let marker = finding.marker;
            assert!(
                marker.starts_with('<') && marker.ends_with("_nn>"),
                "{marker}"
            );
        }
    }

    /// Product Rule 4 (answering Staab et al.): A cloud derivative carries only the bounded
    /// content fields required by the task (Title / Summary / Content). All ambient metadata,
    /// headers (Message-ID, Date, Sender email, IP, routing info) are completely excluded from
    /// the prepared cloud derivative.
    #[test]
    fn cloud_derivative_carries_only_fields_task_needs_answering_staab_et_al() {
        let mut triage = crate::store::db_tests::mk_triage("thread:msg-abc-123", "aktiv");
        triage.from_addr = Some("secret.boss@enterprise.com".into());
        triage.subject = Some("Quarterly review discussion".into());
        triage.snippet = Some("Drafting the report for next week.".into());

        let input = CloudDocumentInput {
            source: "triage".into(),
            id: triage.id.clone(),
            title: triage.subject.clone(),
            author: triage.from_addr.clone(),
            summary: triage.snippet.clone(),
            content: None,
            data_class: "c1".into(),
        };

        let prepared = prepare_pseudonymized(&input, &alice_registry())
            .unwrap()
            .preview;
        let doc = &prepared.document;

        // 1. Task-relevant fields exist
        assert!(doc.contains("Title\nQuarterly review discussion"));
        assert!(doc.contains("Summary\nDrafting the report for next week."));

        // 2. Ambient headers and metadata are completely omitted
        assert!(
            !doc.contains("secret.boss@enterprise.com"),
            "raw sender must not appear: {doc}"
        );
        assert!(
            !doc.contains("thread:msg-abc-123"),
            "thread ID must not appear in document: {doc}"
        );
        assert!(
            !doc.contains("Message-ID"),
            "protocol headers must not appear: {doc}"
        );
        assert!(
            !doc.contains("Received:"),
            "routing headers must not appear: {doc}"
        );
        assert!(
            !doc.contains("User-Agent:"),
            "client headers must not appear: {doc}"
        );
        assert!(
            !doc.contains("Date:"),
            "transport date header must not appear: {doc}"
        );
    }
}
