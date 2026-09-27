//! `content-item-v2` — the canonical reader contract, in Rust.
//!
//! One shape for every kind of observed thing: a feed article, a mail, a
//! calendar entry. Source adapters own collection, storage and actions; the
//! dashboard owns **one** renderer for this contract. `schemas/content-item.schema.json`
//! is the normative artifact — this file exists so the capabilities that emit
//! it cannot drift from each other by hand.
//!
//! ## What this lib is not
//!
//! It is not a unified *storage* model. Each capability keeps its own tables
//! and its own invariants — calendar's exclusive `ends_at` and `(source,
//! external_id)` uniqueness, mail's retention window — because those are
//! genuinely different constraints and a merged table could not enforce any of
//! them. This is a **projection** the stores render themselves into on read.
//!
//! ## Ranking belongs to the source, not to the contract
//!
//! `relevance` and `evaluation` exist because feed is an unbounded inbox that
//! has to be ranked. A calendar entry is something the operator already decided
//! about — its triage axis is `commitment`, surfaced through `status`. A source
//! with no ranking leaves these empty rather than inventing a score; a `0.0`
//! sitting on a committed event is noise that reads as a judgement.
//!
//! Consumers link this as an ordinary crate. Capability boundaries still use
//! the serialized contract rather than reaching into another capability's
//! storage or implementation.

use serde::Serialize;
use serde_json::Value;

/// Bump only with the schema's `schema_version` const, and only for a change
/// readers cannot absorb — adding an optional field is not one.
///
/// `v1` → `v2` on 2026-09-02, for PRD Q27: `data_class.value` moved from
/// `[public, personal, vault]` to `[c0, c1, c2, c3]` and `label` from
/// `[Public, Personal, Private]` to `[Public, Mine, Others, Secret]`. The new
/// sets share no value with the old ones, so a reader that switches on
/// `personal` cannot absorb it — and without the bump it would have had no way
/// to tell the contract had moved. `local_processing` widened from a constant
/// to `allowed | blocked` in the same change.
pub const SCHEMA_VERSION: &str = "content-item-v2";

/// One reader shape for every kind of observed content.
///
/// Every field is always present. The schema is `additionalProperties: false`
/// with everything required, so absent data is an explicit `null` or an empty
/// collection — never a missing key a reader has to probe for.
#[derive(Debug, Clone, Serialize)]
pub struct ContentItem {
    pub schema_version: &'static str,
    pub source: &'static str,
    pub id: String,
    pub kind: String,
    pub title: Option<String>,
    pub url: String,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub content_label: String,
    pub day: String,
    pub created_at: String,
    pub status: String,
    pub content_status: &'static str,
    pub data_class: DataClass,
    pub processing_policy: ProcessingPolicy,
    pub cloud_processing: CloudProcessing,
    pub relevance: Vec<Relevance>,
    pub evaluation: Option<Evaluation>,
    pub processing: Vec<Processing>,
    pub origins: Vec<Origin>,
    pub links: Vec<Link>,
    pub digest: Option<Digest>,
    pub mail: Option<MailExtension>,
    pub calendar: Option<CalendarExtension>,
}

/// What the local model wrote about this thing, and what was asked of it.
///
/// Deliberately not `summary`. `summary` is what the *source* said it is —
/// calendar reads it from the entry's own description, and a generated
/// paragraph written over that destroys the only verbatim text an entry has. A
/// reader that wants "the short version" prefers `digest.text` and falls back to
/// `summary`; both being present is normal, not a conflict.
///
/// Every field is present so the reader never probes. `text` is null whenever
/// `state` is anything but `generated`, and `state` then says why — including
/// `skipped_short`, which is a verdict about the source rather than a failure.
#[derive(Debug, Clone, Serialize)]
pub struct Digest {
    pub text: Option<String>,
    /// `generated` · `skipped_short` · `remote_refused` · `local_refused` ·
    /// `unconfigured` · `http_error` · `model_error` · `capacity_aborted` ·
    /// `empty_response` · `timeout`.
    ///
    /// `local_refused` is the class verdict (T3): the item enters no prompt at
    /// all, so no model was asked and none will be. Producers may add a state a
    /// reader has no case for — a reader that switches on this needs a default
    /// arm, which is why a new state is not a [`SCHEMA_VERSION`] bump.
    pub state: String,
    /// The rung the ladder landed on: `none` · `brief` · `standard` · `sectioned`.
    pub shape: String,
    /// `standard` for the automatic pass, `detailed` when an operator asked for
    /// one rung more.
    pub depth: String,
    /// The operator's focus terms, as typed. Shown back to them so a
    /// differently-shaped digest is explained rather than mysterious.
    pub focus: Vec<String>,
    /// Backend, model and prompt revision. A change to any of the three makes
    /// this row legibly stale instead of silently mixed with newer ones.
    pub producer: String,
    /// How much source the ladder measured. Carrying it means a reader can see
    /// *why* a short item has no digest without re-deriving the length.
    pub source_chars: i64,
    /// How many entities the deterministic redactor removed before this text was
    /// written. Non-zero only for the classes that redact before persistence —
    /// c2 and c3, where the metadata is the payload and a digest could otherwise
    /// republish what the subject line was redacted for.
    pub redactions: i32,
    pub attempts: i32,
    pub last_error: Option<String>,
    /// Mermaid source, validated before it was stored — see `libs/summarize`.
    pub diagram: Option<String>,
    pub diagram_state: Option<String>,
    pub diagram_error: Option<String>,
    /// The chartable table pulled out of the source, as `chart-data` JSON, or
    /// null. Not a chart *spec*: the reader compiles one, so the model never
    /// reaches the rendering layer. Every value in it appeared verbatim in the
    /// source text before it was allowed in.
    pub chart: Option<Value>,
    /// `generated` · `skipped_short` (no comparable numbers, the answer for most
    /// prose) · a failure class.
    pub chart_state: Option<String>,
    pub chart_error: Option<String>,
    pub generated_at: String,
}

/// A named way out of this item: the source page, the mail that carried the
/// ticket, a map, a vault note.
///
/// The field every capability was about to invent separately. Deliberately not
/// typed as an enum of destinations — `kind` is a hint for the icon, and a
/// reader that meets an unknown one still renders a working link.
#[derive(Debug, Clone, Serialize)]
pub struct Link {
    pub label: String,
    pub kind: String,
    pub url: String,
}

impl Link {
    pub fn new(label: impl Into<String>, kind: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            kind: kind.into(),
            url: url.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DataClass {
    pub value: String,
    pub label: &'static str,
    pub rationale: String,
    pub method: String,
    pub version: String,
}

impl DataClass {
    /// `c0`–`c3` are stored; the label is what the product calls each one (Q27).
    pub fn new(
        value: impl Into<String>,
        rationale: impl Into<String>,
        method: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        let value = value.into();
        let label = match value.as_str() {
            "c0" => "Public",
            "c1" => "Mine",
            "c2" => "Others",
            "c3" => "Secret",
            // Every class is spelled out above, so `_` is a value from outside
            // the vocabulary. It reads back as the strictest label rather than
            // the loosest: a stale literal must not render as Public.
            _ => "Secret",
        };
        Self {
            value,
            label,
            rationale: rationale.into(),
            method: method.into(),
            version: version.into(),
        }
    }

    /// Read a stored classification back out of a row.
    ///
    /// The label is re-derived rather than stored, so a row written while the
    /// product called the same class something else still reads back correctly.
    pub fn stored(value: &str, rationale: &str, method: &str, version: &str) -> Self {
        Self::new(value, rationale, method, version)
    }

    /// What an item nobody classified is worth: Mine, decided by nobody.
    ///
    /// The fail-closed default, and the reason it is a real value rather than a
    /// NULL. `legacy` is rank 0, so any later decision — a collector's, a
    /// rule's, a human's — outranks it without a special case for "unset".
    pub fn undeclared() -> Self {
        Self::new(
            "c1",
            UNDECLARED_RATIONALE,
            METHOD_LEGACY,
            LEGACY_CLASSIFIER_VERSION,
        )
    }

    /// A class a collector positively declared for everything it fetches.
    ///
    /// `c0` is only reachable through here. A collector that declares nothing
    /// gets [`DataClass::undeclared`], which is Mine — that is the entire
    /// difference from the literal this replaced, which stamped the public class
    /// on every feed item whether or not anyone had ever looked at the source.
    pub fn declared_by_source(value: &str, rationale: &str) -> Self {
        Self::new(
            value,
            rationale,
            METHOD_DETERMINISTIC,
            SOURCE_CLASSIFIER_VERSION,
        )
    }

    /// The operator's own schedule. Where they are and when is theirs by
    /// construction, whatever the event itself is — a public concert still
    /// tells you the building is empty that evening.
    pub fn personal_source_default(rationale: impl Into<String>) -> Self {
        Self::new(
            "c1",
            rationale,
            METHOD_DETERMINISTIC,
            SOURCE_CLASSIFIER_VERSION,
        )
    }

    /// A class a human set by hand, carrying the reason they gave for it.
    pub fn set_by_human(value: &str, rationale: &str) -> Self {
        Self::new(value, rationale, METHOD_HUMAN, MANUAL_CLASSIFIER_VERSION)
    }

    /// Classify a mail from the metadata a read-only sweep has already
    /// admitted: the stream its mail rules picked, the sender and the subject.
    ///
    /// The body is deliberately not a parameter. Classification decides whether
    /// a body may be fetched at all, so it must not depend on having one.
    ///
    /// Never returns `c0`. A mailbox holds no public content — someone chose to
    /// write to *this* operator, and that choice is itself the operator's.
    ///
    /// Secret is asked first (Q27). A mail matching both rules carries a
    /// credential, and a credential is c3 whatever else it is about.
    pub fn classify_mail(stream: &str, from: &str, subject: &str) -> Self {
        let text = format!("{from} {subject}").to_ascii_lowercase();
        if let Some(rationale) = mail_secret_reason(&text) {
            return Self::new(
                "c3",
                rationale,
                METHOD_DETERMINISTIC,
                MAIL_CLASSIFIER_VERSION,
            );
        }
        if let Some(rationale) = mail_others_reason(stream, &text) {
            return Self::new(
                "c2",
                rationale,
                METHOD_DETERMINISTIC,
                MAIL_CLASSIFIER_VERSION,
            );
        }
        Self::new(
            "c1",
            "Mail metadata is Mine by default.",
            METHOD_DETERMINISTIC,
            MAIL_CLASSIFIER_VERSION,
        )
    }

    /// Classify one vault note from its vault-relative id and whatever its
    /// frontmatter `class:` key says (PRD Q9a, answered 2026-08-23:
    /// *folder default, frontmatter override*).
    ///
    /// `id` is slash-separated and relative to the vault root, which is the
    /// identity `vault::note::Note` already carries. The body is deliberately
    /// not a parameter, for the same reason [`DataClass::classify_mail`] refuses
    /// one: where a note sits is a fact the filesystem already states, and a
    /// classifier that has to read a note to place it cannot run on the walk.
    ///
    /// Never returns `c0`. Publishing is an act, not a location — §15 decides
    /// what leaves, and no folder in the vault means "already public".
    ///
    /// A declared class that is not one of [`DATA_CLASSES`] is **refused, not
    /// honoured and not escalated**, and the folder default answers instead. A
    /// typo is not evidence that a note is more sensitive than its folder, so
    /// inventing a stricter class from one would be a guess; the rationale names
    /// the rejected literal so a caller can report it rather than swallow it.
    pub fn classify_vault_note(id: &str, declared: Option<&str>) -> Self {
        if let Some(raw) = declared.map(str::trim).filter(|d| !d.is_empty()) {
            if valid(raw) {
                return Self::set_by_human(raw, "The note's frontmatter declares this class.");
            }
            let mut fallback = Self::vault_folder_default(id);
            fallback.rationale = format!(
                "Frontmatter declares `class: {raw}`, which is not one of {}; \
                 the folder default answers instead.",
                DATA_CLASSES.join(", ")
            );
            return fallback;
        }
        Self::vault_folder_default(id)
    }

    /// The folder half of Q9a, with no frontmatter in play.
    fn vault_folder_default(id: &str) -> Self {
        let lowered = id.to_ascii_lowercase();
        match vault_others_reason(&lowered) {
            Some(rationale) => Self::new(
                "c2",
                rationale,
                METHOD_DETERMINISTIC,
                VAULT_CLASSIFIER_VERSION,
            ),
            None => Self::new(
                "c1",
                "A vault note is Mine unless its folder says it holds someone else's facts.",
                METHOD_DETERMINISTIC,
                VAULT_CLASSIFIER_VERSION,
            ),
        }
    }
}

/// The stored classes, in the order a reader should offer them. The literal is
/// what a row stores; the label is the word the product uses (see
/// [`DataClass::new`]).
pub const DATA_CLASSES: [&str; 4] = ["c0", "c1", "c2", "c3"];

/// Stamped on every rules-produced classification, so a stored row records
/// which rule set decided it. Bump with any change to [`mail_secret_reason`] or
/// [`mail_others_reason`].
pub const MAIL_CLASSIFIER_VERSION: &str = "data-class-rules-v2";

/// Stamped by a collector that declared a class for what it fetches.
pub const SOURCE_CLASSIFIER_VERSION: &str = "data-class-source-v1";

/// Stamped on every folder-derived vault classification. Bump with any change
/// to [`vault_others_reason`], the same contract [`MAIL_CLASSIFIER_VERSION`]
/// carries for the mail rules.
pub const VAULT_CLASSIFIER_VERSION: &str = "data-class-vault-v1";

/// Stamped on a row nobody ever classified, including every row that predates
/// the class existing on its table.
pub const LEGACY_CLASSIFIER_VERSION: &str = "data-class-legacy-v1";

/// Stamped when a human set the class by hand.
pub const MANUAL_CLASSIFIER_VERSION: &str = "manual-v1";

/// Why an undeclared item is Mine. Kept next to the DB DEFAULT that writes the
/// same sentence, because the two have to agree for a backfilled row and a
/// freshly ingested one to read alike.
pub const UNDECLARED_RATIONALE: &str =
    "No collector declared a class for this item; Mine by default.";

pub const METHOD_LEGACY: &str = "legacy";
pub const METHOD_DETERMINISTIC: &str = "deterministic";
pub const METHOD_MODEL: &str = "model";
pub const METHOD_HUMAN: &str = "human";

/// Who decided, in order. One vocabulary for every classification and every
/// processing stage in the contract — `comms::provenance::tier_rank` is this
/// function, and the DB CHECK constraints spell out this list.
///
/// The order is the point. Without a rank there is no way to say "a rule may
/// not overwrite what a human decided" except by enumerating pairs, and the
/// enumeration is what drifts.
pub const CLASSIFICATION_METHODS: [&str; 4] = [
    METHOD_LEGACY,
    METHOD_DETERMINISTIC,
    METHOD_MODEL,
    METHOD_HUMAN,
];

pub fn method_rank(method: &str) -> Option<i16> {
    match method {
        METHOD_LEGACY => Some(0),
        METHOD_DETERMINISTIC => Some(10),
        METHOD_MODEL => Some(20),
        METHOD_HUMAN => Some(30),
        _ => None,
    }
}

/// How exposed the content is, ascending: Public < Mine < Others < Secret.
///
/// A higher rank is a stricter class, so "escalate" is "raise the rank" and the
/// whole escalation-only rule is one comparison rather than a table of cases.
/// The gaps of ten are the reason a fourth class cost no rule change (Q27).
pub fn class_rank(data_class: &str) -> Option<i16> {
    match data_class {
        "c0" => Some(0),
        "c1" => Some(10),
        "c2" => Some(20),
        "c3" => Some(30),
        _ => None,
    }
}

pub fn valid(data_class: &str) -> bool {
    DATA_CLASSES.contains(&data_class)
}

/// Whether a proposed reclassification may be written over the stored one.
///
/// The single home of the escalation rule, and the reason it is a function
/// rather than a check each caller performs: a gate every caller has to
/// remember to call is the shape that produced three disagreeing copies of the
/// cloud verdict already.
///
/// Escalation — proposing a stricter class — is always admitted, whoever
/// proposes it. De-escalation is admitted only for a human, and only with a
/// rationale they actually wrote. Both halves matter. A rule that can lower a
/// class is a declassification primitive: it runs over attacker-controlled feed
/// text, and "this looks like a public blog post" is a sentence an injected
/// page can arrange to be true of itself. And a de-escalation with no reason
/// recorded is indistinguishable afterwards from a mistake.
///
/// The third case is equality, and it is why the *stored* method is a parameter
/// rather than an implementation detail of whoever stored it. A write proposing
/// the class already on the row changes no class, so both rules above wave it
/// through — yet it still overwrites the method and the rationale. A machine
/// re-ingest arriving at `c1` on a row a human had classified `c1` erased the
/// human's record exactly that way: the class survived, the reason it was
/// chosen did not. So at equal class a non-human write over a stored human
/// decision writes nothing. There is no classification to make there, only a
/// record to lose.
///
/// Deliberately narrow: `human` is protected this way, method rank in general
/// is not. A rules resweep re-deciding at the same class is the classifier
/// doing its job and has to keep landing; only what a person wrote is
/// unreproducible.
pub fn admit_reclassification(
    stored_class: &str,
    stored_method: &str,
    proposed_class: &str,
    proposed_method: &str,
    rationale: &str,
) -> Result<(), &'static str> {
    let Some(stored_rank) = class_rank(stored_class) else {
        return Err("stored data class is not a known class");
    };
    let Some(proposed_rank) = class_rank(proposed_class) else {
        return Err("data class must be one of: c0, c1, c2, c3");
    };
    if method_rank(stored_method).is_none() {
        return Err("stored classification method is not a known method");
    }
    if method_rank(proposed_method).is_none() {
        return Err("classification method must be one of: legacy, deterministic, model, human");
    }
    if proposed_rank > stored_rank {
        return Ok(());
    }
    if proposed_rank == stored_rank {
        if stored_method == METHOD_HUMAN && proposed_method != METHOD_HUMAN {
            return Err("only a human may replace a human's classification at the same class");
        }
        return Ok(());
    }
    if proposed_method != METHOD_HUMAN {
        return Err("only a human may lower a data class");
    }
    if rationale.trim().is_empty() {
        return Err("lowering a data class requires a written rationale");
    }
    Ok(())
}

/// Whether a stored *review representation* of this class must be redacted
/// before it is persisted.
///
/// The distinction that matters: for c0 and c1 the sensitive material is in the
/// body, and keeping bodies transient is enough. For c2 and c3 the metadata is
/// the payload (Q27) — a one-time code arrives in the subject line, and storing
/// that subject verbatim puts it in a log, an API response and a dashboard at
/// once. A subject naming another person does the same for a fact that was
/// never the operator's to republish.
///
/// Written as the complement of the two classes that may be stored verbatim,
/// not as a list of the two that may not: an unrecognized value is redacted
/// rather than published. That matches every other unknown-class answer in this
/// module — `class_rank` returns `None`, `processing_policy` returns c3's
/// policy, `DataClass::new` labels it Secret — and the value reaching here is a
/// stored string, so the vocabulary it was written under is not guaranteed to
/// be this one.
pub fn redact_before_persistence(data_class: &str) -> bool {
    !matches!(data_class, "c0" | "c1")
}

/// Why a mail's metadata alone is enough to call it Secret, or `None`.
///
/// Credentials only. A match here means the text carries or announces an
/// authentication factor, and c3 never enters a prompt at all. The stream is not
/// a parameter: no mail stream is a credential by itself.
///
/// Conservative on purpose: a false `c3` costs a redacted subject line in a
/// review list, a false `c1` costs a leaked credential.
fn mail_secret_reason(lowercased_text: &str) -> Option<&'static str> {
    const AUTHENTICATION: [&str; 22] = [
        "verification code",
        "security code",
        "security alert",
        "account alert",
        "one-time code",
        "one-time access token",
        "access token",
        "recovery code",
        "new sign in",
        "new sign-in",
        "new login",
        "trusted device",
        "suspicious activity",
        "magic link",
        "secure mail",
        "securemail",
        "vertraulich",
        "bestätigungscode",
        "sicherheitscode",
        "einmalcode",
        "passwort",
        "password",
    ];

    // `2fa` and `otp` are matched as whole words: `otp` alone also sits inside
    // ordinary German words, and a substring match there classified unrelated
    // mail as Secret.
    let bounded_token = lowercased_text
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word == "2fa" || word == "otp");

    if bounded_token || contains_any(lowercased_text, &AUTHENTICATION) {
        return Some("Authentication or account-recovery metadata is Secret.");
    }
    None
}

/// Why a mail's metadata alone is enough to call it Others, or `None` for the
/// Mine default.
///
/// Facts that belong to somebody else. The `steuern` and `belege` streams land
/// here rather than in [`mail_secret_reason`] because tax and receipt mail
/// routinely names other people and carries their financial detail (Q27); it is
/// not a credential.
///
/// Conservative on purpose: a false `c2` costs a redacted subject line in a
/// review list, a false `c1` costs another person's money or health in a cloud
/// prompt.
fn mail_others_reason(stream: &str, lowercased_text: &str) -> Option<&'static str> {
    if stream == "steuern" {
        return Some("Tax-related mail is Others by default.");
    }
    if stream == "belege" {
        return Some("Receipts and invoices are Others by default.");
    }
    const FINANCIAL: [&str; 8] = [
        "bank statement",
        "kontoauszug",
        "rechnung",
        "invoice",
        "payment",
        "zahlung",
        "insurance",
        "versicherung",
    ];
    const HEALTH: [&str; 7] = [
        "diagnosis",
        "diagnose",
        "prescription",
        "rezept",
        "medical result",
        "befund",
        "krankenversicherung",
    ];

    if contains_any(lowercased_text, &FINANCIAL) {
        return Some("Financial or insurance metadata is Others.");
    }
    if contains_any(lowercased_text, &HEALTH) {
        return Some("Health-related metadata is Others.");
    }
    None
}

/// Which vault folders hold other people's facts (PRD Q9a's table, read
/// against §6.1's definition of C2: *facts about named people — health, money,
/// relationships, addresses, things told in confidence*).
///
/// `lowercased_id` is vault-relative and slash-separated. Matching is on whole
/// path SEGMENTS, not substrings: `Atlas/People/` is a folder that holds people,
/// `Knowledge/People-Analytics.md` is an article about a discipline, and a
/// substring match cannot tell them apart.
///
/// `Atlas/Personal/` is deliberately absent. C2 is other people's facts; the
/// operator's own are C1, which is what Q9a's *everything else* already says.
fn vault_others_reason(lowercased_id: &str) -> Option<&'static str> {
    // Health is named by Q9a as a rule of its own rather than a folder, because
    // `Atlas/Documents/Gesundheit/` is only where it happens to sit today. Both
    // spellings, because this vault is written in two languages.
    const HEALTH_SEGMENTS: [&str; 4] = ["gesundheit", "health", "medical", "arzt"];

    let segments: Vec<&str> = lowercased_id.split('/').collect();
    // The filename is excluded from the segment scan: a folder places a note,
    // a title describes it, and `Knowledge/Mental Health at Work.md` is an
    // article rather than somebody's diagnosis.
    let folders = segments.split_last().map(|(_, f)| f).unwrap_or(&[]);

    if folders.first() == Some(&"atlas") {
        match folders.get(1) {
            Some(&"people") => return Some("Atlas/People holds facts about named people."),
            Some(&"documents") => {
                return Some("Atlas/Documents holds scans and records about named people.")
            }
            Some(&"finance") => return Some("Atlas/Finance holds money, which §6.1 names as C2."),
            _ => {}
        }
    }
    if folders.iter().any(|f| HEALTH_SEGMENTS.contains(f)) {
        return Some("A health folder holds facts §6.1 names as C2 wherever it sits.");
    }
    None
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

/// The provider tiers a reviewed cloud role can declare, narrowest first.
///
/// `libs/inference` owns these names (Q26) and they are not class names — a
/// tier says what an operator reviewed one provider to receive. They are spelled
/// here because [`cloud_admission`] is the policy and a policy that cannot name
/// the tier it admits is not one. The lib stays a leaf crate: naming two strings
/// costs nothing, and depending on `libs/inference` for them would invert the
/// dependency every capability already has on this crate.
pub const CLOUD_DATA_TIERS: [&str; 2] = ["public", "pseudonymized_personal"];

/// Why no cloud role may receive this representation of this class.
///
/// Carries the class, the tier and the reason, and never the content: this is
/// formatted into logs, HTTP bodies and a dashboard, which is exactly where a
/// refusal that quoted the document it refused would republish it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudRefusal {
    pub original_data_class: String,
    pub derivative_data_class: String,
    pub tier: Option<String>,
    pub reason: &'static str,
}

/// The class has no cloud representation at all — c2, c3, and every value from
/// outside the vocabulary.
pub const NO_CLOUD_REPRESENTATION: &str = "this class reaches no cloud role in any representation";

/// The endpoint carries no reviewed cloud tier, so there is nothing to admit
/// against. Every loopback role lands here, and so does an https endpoint
/// nobody gave a cloud policy.
pub const NO_DECLARED_TIER: &str = "the endpoint declares no reviewed cloud tier";

/// The class has a cloud lane, and this is not it: the wrong derivative class,
/// or a tier narrower than the lane needs.
pub const REPRESENTATION_NOT_REVIEWED: &str =
    "this tier admits a different representation of this class";

impl std::fmt::Display for CloudRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} as {} to tier {}: {}",
            self.original_data_class,
            self.derivative_data_class,
            self.tier.as_deref().unwrap_or("none"),
            self.reason
        )
    }
}

impl std::error::Error for CloudRefusal {}

/// Whether a cloud role at `tier` may receive `derivative_class` derived from
/// `original_class`.
///
/// **The admission policy, and the only expression of it.** It used to have two:
/// a real gate inside comms and an advisory label here, free to disagree and
/// already disagreeing about what an unrecognized class meant. Everything else
/// in this file — [`processing_policy`]'s `cloud_handling`, its
/// `pseudonymization_required` — is now derived from this function rather than
/// written beside it.
///
/// Class-level only. The transformation *version* that produced the derivative
/// is comms' to pin, because comms owns the redactor whose revisions those are;
/// this answers which classes may travel where, which is the part every
/// capability needs and none of them should re-derive.
///
/// Two lanes exist and nothing else does:
/// - `c0` unchanged, to any declared tier. A tier reviewed for pseudonymized
///   personal content admits public content too — breadth, not equality, the
///   containment `sjel_inference::CloudDataTier::admits_at_least` states.
/// - `c1` as a `c1` derivative, to `pseudonymized_personal` only.
///
/// Everything else is refused, `c2` and `c3` first (Q27: one holds other
/// people's facts, the other holds credentials) and any unknown value with them.
/// Stated as an allow-list of admitted pairs rather than a deny-list of refused
/// classes, because the class arrives as a stored string and the previous
/// vocabulary is still readable in a backup and in a stale queued job.
pub fn cloud_admission(
    original_class: &str,
    derivative_class: &str,
    tier: Option<&str>,
) -> Result<(), CloudRefusal> {
    let refuse = |reason| {
        Err(CloudRefusal {
            original_data_class: original_class.to_string(),
            derivative_data_class: derivative_class.to_string(),
            tier: tier.map(str::to_string),
            reason,
        })
    };
    if !matches!(original_class, "c0" | "c1") {
        return refuse(NO_CLOUD_REPRESENTATION);
    }
    let Some(tier) = tier else {
        return refuse(NO_DECLARED_TIER);
    };
    match (original_class, derivative_class, tier) {
        ("c0", "c0", "public" | "pseudonymized_personal") => Ok(()),
        ("c1", "c1", "pseudonymized_personal") => Ok(()),
        _ => refuse(REPRESENTATION_NOT_REVIEWED),
    }
}

/// Whether this class may be sent to a cloud role **as it stands** — no
/// derivative, no redaction, the stored text itself.
///
/// `c0` and nothing else. `c1` has a cloud lane and it is the reviewed redacted
/// derivative; the same document handed over unchanged is a different question
/// with a different answer.
pub fn verbatim_cloud_allowed(data_class: &str) -> bool {
    data_class == "c0"
}

/// Whether this class may enter a prompt on this machine at all.
///
/// `false` for `c3` and for every value outside the vocabulary; `true` for c0,
/// c1 and c2. A credential belongs in no processing step, local or cloud (Q27),
/// and a local model is still a log, a context window and a cache.
///
/// This is what makes `processing_policy(..).local_processing` mechanical rather
/// than declared: `capabilities/comms`'s digest and media prompt-builders ask
/// this function before they build a prompt (T3).
pub fn local_prompt_allowed(data_class: &str) -> bool {
    matches!(data_class, "c0" | "c1" | "c2")
}

/// Whether any representation of this class reaches any reviewed cloud tier.
///
/// The first question in every cloud lane, and the coarsest: not *which* tier
/// and *which* transformation, but whether a cloud representation of this class
/// may be built at all. `c0` and `c1` have one; `c2`, `c3` and every value from
/// outside the vocabulary have none.
///
/// Searched, not tabulated — true iff some `(derivative, tier)` pair over
/// [`DATA_CLASSES`] × [`CLOUD_DATA_TIERS`] clears [`cloud_admission`]. That is
/// the whole point of exporting it: a caller who only needs this coarse answer
/// would otherwise write the two admitted class names out again beside the
/// policy, free to disagree with it. `capabilities/comms`'s
/// `cloud_derivative::prepare` did exactly that, and it is the *first* gate a
/// cloud lane passes, so its copy narrowing later than this one would build an
/// approvable preview of a document with no lane (T3).
///
/// Eight combinations; it is not a hot path.
pub fn has_cloud_lane(data_class: &str) -> bool {
    DATA_CLASSES.iter().any(|derivative| {
        CLOUD_DATA_TIERS
            .iter()
            .any(|tier| cloud_admission(data_class, derivative, Some(tier)).is_ok())
    })
}

/// The label for the cloud lane this class has, derived by asking
/// [`cloud_admission`] about every representation and tier that exist.
///
/// Searched rather than tabulated so the wire label cannot say `eligible` while
/// the gate refuses — which is the condition this whole module change exists to
/// end.
fn cloud_handling(data_class: &str) -> &'static str {
    if verbatim_cloud_allowed(data_class) {
        return "eligible";
    }
    if has_cloud_lane(data_class) {
        "pseudonymization_required"
    } else {
        "blocked"
    }
}

/// Derived permission boundary. Records eligibility; never evidence that a
/// pseudonymization step actually ran.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcessingPolicy {
    pub local_processing: &'static str,
    pub cloud_handling: &'static str,
    pub pseudonymization_required: bool,
    pub rationale: &'static str,
}

/// Policy follows from the stored class rather than being chosen per call.
///
/// Every field but the rationale is *derived from the gates themselves* —
/// [`local_prompt_allowed`], [`cloud_admission`], [`verbatim_cloud_allowed`] —
/// so the label a reader is shown and the answer a call site gets are one
/// expression. They were two, and two expressions of one policy are free to
/// disagree: this crate labelled `local_processing: "blocked"` for c3 while the
/// digest path, reading no field, prompted the local model with it anyway.
///
/// Only the rationale is a table, because it is prose. Every class is an
/// explicit arm so the catch-all means "outside the vocabulary" and nothing
/// else — and the derived fields answer for an unknown class exactly as they do
/// for c3, since each gate refuses it on its own.
pub fn processing_policy(data_class: &str) -> ProcessingPolicy {
    ProcessingPolicy {
        local_processing: if local_prompt_allowed(data_class) {
            "allowed"
        } else {
            "blocked"
        },
        cloud_handling: cloud_handling(data_class),
        pseudonymization_required: !verbatim_cloud_allowed(data_class),
        rationale: match data_class {
            "c0" => "Public content may be processed locally or by an approved cloud role.",
            "c1" => "The operator's own material stays local until an explicit pseudonymization step produces a reviewed derivative.",
            "c2" => "Facts about named people are local-only; no derivative carries them to a cloud role in any form.",
            "c3" => "A credential enters no prompt at all, local or cloud; there is no processing step it belongs in.",
            _ => "A class outside the vocabulary is handled as a secret until somebody classifies it.",
        },
    }
}

/// Local approval plus explicit cloud execution state. Only `running` or later
/// implies a provider was actually called.
#[derive(Debug, Clone, Serialize)]
pub struct CloudProcessing {
    pub status: String,
    pub preview_hash: Option<String>,
    pub approved_at: Option<String>,
    pub dispatch_status: String,
    pub job_id: Option<String>,
    pub provider_role: Option<String>,
    pub queued_at: Option<String>,
    pub provider_calls: u8,
    pub task: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub last_error: Option<String>,
    pub result: Option<Value>,
}

impl CloudProcessing {
    /// The honest state for a source with no cloud pipeline at all. Not a
    /// placeholder: "nothing was prepared and nothing was sent" is the claim.
    pub fn not_prepared() -> Self {
        Self {
            status: "not_prepared".into(),
            preview_hash: None,
            approved_at: None,
            dispatch_status: "not_queued".into(),
            job_id: None,
            provider_role: None,
            queued_at: None,
            provider_calls: 0,
            task: None,
            started_at: None,
            completed_at: None,
            last_error: None,
            result: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Relevance {
    pub profile_key: String,
    pub profile_label: String,
    pub score: f64,
    pub rationale: String,
    pub mode: String,
    pub profile_revision: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evaluation {
    pub overall_score: f64,
    pub explanation: String,
    pub mode: String,
    pub item_revision: String,
    pub context_revision: String,
    pub evaluator_revision: String,
    pub evaluated_at: String,
    pub factors: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Processing {
    pub stage: String,
    pub tier: String,
    pub revision: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Origin {
    pub source_id: String,
    pub source_ref: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MailExtension {
    pub category: String,
    pub rationale: String,
    pub classification_method: String,
    pub classification_version: String,
    pub gmail_action: Option<String>,
    pub gmail_action_at: Option<String>,
    pub purge_after: Option<String>,
    pub gmail_location: Option<String>,
    pub gmail_observed_at: Option<String>,
    pub gmail_sync_status: Option<String>,
    pub gmail_sync_action: Option<String>,
    pub gmail_sync_error: Option<String>,
}

/// What only a calendar entry has: it occupies time, and the operator has
/// taken a position on it.
///
/// `commitment` rather than a score is the whole point — see the module note on
/// ranking. `all_day` and the exclusive `ends_at` are carried verbatim so the
/// reader never re-derives them and gets the boundary wrong.
#[derive(Debug, Clone, Serialize)]
pub struct CalendarExtension {
    pub starts_at: String,
    /// Exclusive, as everywhere in calendar.
    pub ends_at: String,
    pub all_day: bool,
    pub commitment: String,
    pub location: Option<String>,
    /// The operator's own note. Distinct from `summary`, which describes what
    /// the thing *is* — this is why they care, and no machine writes it.
    pub notes: Option<String>,
    /// Which adapter contributed the entry — `manual`, `luma`, `google`. Not
    /// the item's `source`, which is always `calendar`: that says which
    /// capability serves this, this says where the row came from.
    ///
    /// A reader needs it to know which actions are honest. An entry imported
    /// *from* Google must not offer to export back to Google.
    pub entry_source: String,
    /// Set when this entry was materialized from a rhythm. Carried for the same
    /// reason: a rhythm instance is not exported individually, and any patch
    /// detaches it from its rhythm — both facts a surface has to know before it
    /// offers an action.
    pub rhythm_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal(source: &'static str) -> ContentItem {
        ContentItem {
            schema_version: SCHEMA_VERSION,
            source,
            id: "x".into(),
            kind: "event".into(),
            title: None,
            url: "/calendar".into(),
            author: None,
            summary: None,
            content: None,
            content_label: "Notes".into(),
            day: "2026-08-10".into(),
            created_at: "2026-08-04T00:00:00Z".into(),
            status: "committed".into(),
            content_status: "none",
            data_class: DataClass::declared_by_source("c0", "Test fixture."),
            processing_policy: processing_policy("c0"),
            cloud_processing: CloudProcessing::not_prepared(),
            relevance: Vec::new(),
            evaluation: None,
            processing: Vec::new(),
            origins: Vec::new(),
            links: Vec::new(),
            digest: None,
            mail: None,
            calendar: None,
        }
    }

    /// The schema is `additionalProperties: false` with every field required,
    /// so a reader may index straight in. An `Option` that skipped serializing
    /// would break that silently, on one source only.
    #[test]
    fn every_contract_field_is_emitted_even_when_empty() {
        let value = serde_json::to_value(minimal("calendar")).unwrap();
        let object = value.as_object().unwrap();
        for field in [
            "schema_version",
            "source",
            "id",
            "kind",
            "title",
            "url",
            "author",
            "summary",
            "content",
            "content_label",
            "day",
            "created_at",
            "status",
            "content_status",
            "data_class",
            "processing_policy",
            "cloud_processing",
            "relevance",
            "evaluation",
            "processing",
            "origins",
            "links",
            "digest",
            "mail",
            "calendar",
        ] {
            assert!(
                object.contains_key(field),
                "{field} missing from the wire shape"
            );
        }
        assert!(
            object["title"].is_null(),
            "an absent title is null, not omitted"
        );
        assert_eq!(
            object["links"],
            json!([]),
            "an empty collection is [], not null"
        );
    }

    /// The four classes and what each one obliges, as one table: the word the
    /// product shows, the rank the escalation rule compares, and whether the
    /// stored review representation is redacted before it is written.
    #[test]
    fn every_stored_class_carries_its_label_rank_and_redaction_duty() {
        for (value, label, rank, redacts) in [
            ("c0", "Public", 0, false),
            ("c1", "Mine", 10, false),
            ("c2", "Others", 20, true),
            ("c3", "Secret", 30, true),
        ] {
            let stored = DataClass::stored(value, "r", METHOD_HUMAN, "v1");
            assert_eq!(stored.label, label, "{value} is labelled {label}");
            assert_eq!(class_rank(value), Some(rank));
            assert_eq!(redact_before_persistence(value), redacts);
        }
        assert_eq!(DATA_CLASSES.len(), 4);
        assert_eq!(
            DataClass::declared_by_source("c0", "Declared.").label,
            "Public"
        );
        // The old vocabulary is gone, not accepted alongside the new one.
        assert!(!valid("personal"));
        assert!(!valid("vault"));
        // A value from outside the vocabulary reads back as the strictest
        // label, never as Public.
        assert_eq!(
            DataClass::stored("personal", "r", METHOD_LEGACY, "v1").label,
            "Secret"
        );
        // And it is redacted before it is stored, for the same reason. The
        // argument is a string read back out of a row, so a class written under
        // the previous vocabulary is a value this function really receives —
        // and the answer that costs a redacted subject line is the safe one.
        for unknown in ["personal", "vault", "private", "C0", "", "c4"] {
            assert!(
                redact_before_persistence(unknown),
                "{unknown} would have been stored verbatim"
            );
        }
    }

    /// A class must never come back more cloud-eligible than it is; that
    /// mapping is the whole reason policy is derived rather than passed in.
    #[test]
    fn policy_is_derived_from_the_class_and_never_widens_it() {
        assert_eq!(processing_policy("c0").cloud_handling, "eligible");
        assert_eq!(processing_policy("c0").local_processing, "allowed");
        assert!(!processing_policy("c0").pseudonymization_required);
        assert_eq!(
            processing_policy("c1").cloud_handling,
            "pseudonymization_required"
        );
        assert_eq!(processing_policy("c1").local_processing, "allowed");
        assert_eq!(processing_policy("c2").cloud_handling, "blocked");
        // Others is local-only, not local-forbidden: the operator still reads
        // and summarizes their own mail about other people.
        assert_eq!(processing_policy("c2").local_processing, "allowed");
        // Secret is the one class no prompt may hold, local model included.
        assert_eq!(processing_policy("c3").cloud_handling, "blocked");
        assert_eq!(processing_policy("c3").local_processing, "blocked");
        for class in DATA_CLASSES {
            assert_eq!(
                processing_policy(class).pseudonymization_required,
                class != "c0",
                "{class} must declare whether pseudonymization is owed"
            );
        }
        // An unknown class gets the strictest policy, not the loosest.
        assert_eq!(processing_policy("something-new").cloud_handling, "blocked");
        assert_eq!(
            processing_policy("something-new").local_processing,
            "blocked"
        );
        assert!(processing_policy("something-new").pseudonymization_required);
    }

    /// Six values from outside the vocabulary, chosen for the ways one actually
    /// arrives: the retired three-class names still readable in a backup and in
    /// a stale queued job, a plausible neighbour, a case error, and the empty
    /// string a missing column reads as.
    const OUTSIDE_THE_VOCABULARY: [&str; 6] = ["public", "personal", "vault", "c4", "C2", ""];

    /// T3's bar, stated over the whole input space rather than over the pairs
    /// someone thought of: no representation of a local-only class clears any
    /// tier. Eight originals — `c2`, `c3` and six unknowns — times ten
    /// derivative values times four tier values, and every one of the 320
    /// combinations must be an `Err`.
    ///
    /// The tier list includes a name no role declares today, because a tier
    /// arrives here as a string too and a fourth tier is a configuration change
    /// rather than a code change.
    #[test]
    fn no_representation_of_a_local_only_class_clears_any_tier() {
        let derivatives: Vec<&str> = DATA_CLASSES
            .iter()
            .chain(OUTSIDE_THE_VOCABULARY.iter())
            .copied()
            .collect();
        let tiers: Vec<Option<&str>> = std::iter::once(None)
            .chain(CLOUD_DATA_TIERS.iter().map(|tier| Some(*tier)))
            .chain(std::iter::once(Some("trusted_peer")))
            .collect();

        for original in ["c2", "c3"].iter().chain(OUTSIDE_THE_VOCABULARY.iter()) {
            assert!(
                !verbatim_cloud_allowed(original),
                "{original} was cleared to travel verbatim"
            );
            assert_eq!(
                processing_policy(original).cloud_handling,
                "blocked",
                "{original} is labelled with a cloud lane it does not have"
            );
            for derivative in &derivatives {
                for tier in &tiers {
                    let refusal = cloud_admission(original, derivative, *tier).expect_err(
                        &format!("{original} as {derivative} was admitted to tier {tier:?}"),
                    );
                    assert_eq!(refusal.reason, NO_CLOUD_REPRESENTATION);
                    assert_eq!(refusal.original_data_class, *original);
                }
            }
        }
    }

    /// The other half of the same property: the two lanes that do exist, and
    /// nothing beside them. Without this the test above passes on a function
    /// that refuses everything.
    #[test]
    fn the_two_cloud_lanes_are_the_only_ones() {
        assert!(cloud_admission("c0", "c0", Some("public")).is_ok());
        // Breadth, not equality: a tier reviewed for pseudonymized personal
        // content admits public content too.
        assert!(cloud_admission("c0", "c0", Some("pseudonymized_personal")).is_ok());
        assert!(cloud_admission("c1", "c1", Some("pseudonymized_personal")).is_ok());

        // The narrower tier does not reach up to the c1 lane.
        assert_eq!(
            cloud_admission("c1", "c1", Some("public"))
                .unwrap_err()
                .reason,
            REPRESENTATION_NOT_REVIEWED
        );
        // A c1 original may only become a c1 derivative; "redact it into a
        // public document" is not a step this policy knows.
        assert_eq!(
            cloud_admission("c1", "c0", Some("pseudonymized_personal"))
                .unwrap_err()
                .reason,
            REPRESENTATION_NOT_REVIEWED
        );
        // Every loopback role and every https endpoint with no reviewed policy.
        for class in ["c0", "c1"] {
            assert_eq!(
                cloud_admission(class, class, None).unwrap_err().reason,
                NO_DECLARED_TIER
            );
        }
    }

    /// The label is the gate. Both fields of the policy that used to be written
    /// by hand are re-derived here from the functions the call sites ask, so a
    /// class whose label and gate disagree fails this test rather than shipping.
    #[test]
    fn the_wire_label_says_exactly_what_the_gates_answer() {
        for class in DATA_CLASSES.iter().chain(OUTSIDE_THE_VOCABULARY.iter()) {
            let policy = processing_policy(class);
            assert_eq!(
                policy.local_processing == "allowed",
                local_prompt_allowed(class),
                "{class}: local_processing disagrees with local_prompt_allowed"
            );
            assert_eq!(
                policy.cloud_handling == "eligible",
                verbatim_cloud_allowed(class),
                "{class}: cloud_handling disagrees with verbatim_cloud_allowed"
            );
            assert_eq!(
                policy.cloud_handling == "blocked",
                CLOUD_DATA_TIERS.iter().all(|tier| DATA_CLASSES
                    .iter()
                    .all(|derivative| cloud_admission(class, derivative, Some(tier)).is_err())),
                "{class}: cloud_handling disagrees with cloud_admission"
            );
        }
        // c3 is the one class no prompt may hold, local model included.
        assert!(!local_prompt_allowed("c3"));
        for class in ["c0", "c1", "c2"] {
            assert!(local_prompt_allowed(class));
        }
    }

    /// `has_cloud_lane` is exported so a caller does not have to write the
    /// admitted class names out again, which only helps if it cannot drift from
    /// the function it summarizes.
    ///
    /// Derived rather than tabulated: the expectation is the
    /// `(derivative, tier)` search itself, run here over the same input space
    /// the predicate covers, so this fails if either side changes alone. The
    /// four anchors below stop it passing on a policy that had begun refusing
    /// everything, or admitting everything.
    #[test]
    fn the_cloud_lane_predicate_is_the_admission_search_itself() {
        for class in DATA_CLASSES.iter().chain(OUTSIDE_THE_VOCABULARY.iter()) {
            let searched = DATA_CLASSES.iter().any(|derivative| {
                CLOUD_DATA_TIERS
                    .iter()
                    .any(|tier| cloud_admission(class, derivative, Some(tier)).is_ok())
            });
            assert_eq!(
                has_cloud_lane(class),
                searched,
                "{class}: has_cloud_lane disagrees with cloud_admission"
            );
        }
        assert!(has_cloud_lane("c0"), "c0 lost its passthrough lane");
        assert!(has_cloud_lane("c1"), "c1 lost its reviewed redacted lane");
        for local_only in ["c2", "c3"] {
            assert!(
                !has_cloud_lane(local_only),
                "{local_only} was given a cloud lane"
            );
        }
    }

    /// A refusal is formatted into logs, HTTP bodies and a dashboard. It carries
    /// the class, the tier and the reason, and it must not be a place a document
    /// leaks — which is why `cloud_admission` never receives one.
    #[test]
    fn a_refusal_names_the_class_and_the_tier_and_nothing_else() {
        let refusal = cloud_admission("c3", "c1", Some("pseudonymized_personal")).unwrap_err();
        let rendered = refusal.to_string();
        assert!(rendered.contains("c3"), "{rendered}");
        assert!(rendered.contains("pseudonymized_personal"), "{rendered}");
        assert!(rendered.contains(NO_CLOUD_REPRESENTATION), "{rendered}");
        assert_eq!(
            cloud_admission("c2", "c1", None).unwrap_err().tier,
            None,
            "an undeclared tier is reported as absent, not as a name"
        );
    }

    /// The escalation rule over its whole input space, not over the three pairs
    /// someone thought of. Sixteen ordered class pairs times four stored methods
    /// times four proposing methods times three rationales: every one of the 768
    /// combinations is decided here, and the assertion is the rule restated
    /// independently rather than the implementation called twice. The fourth
    /// class widened this from 432 without touching the rule (Q27).
    #[test]
    fn no_machine_lowers_a_class_or_overwrites_a_human_and_no_human_lowers_silently() {
        for stored_class in DATA_CLASSES {
            for stored_method in CLASSIFICATION_METHODS {
                for proposed_class in DATA_CLASSES {
                    for proposed_method in CLASSIFICATION_METHODS {
                        for rationale in ["", "   ", "The paper is on arXiv."] {
                            let admitted = admit_reclassification(
                                stored_class,
                                stored_method,
                                proposed_class,
                                proposed_method,
                                rationale,
                            )
                            .is_ok();
                            let stored_rank = class_rank(stored_class);
                            let proposed_rank = class_rank(proposed_class);
                            let expected = if proposed_rank < stored_rank {
                                proposed_method == METHOD_HUMAN && !rationale.trim().is_empty()
                            } else if proposed_rank == stored_rank {
                                stored_method != METHOD_HUMAN || proposed_method == METHOD_HUMAN
                            } else {
                                true
                            };
                            assert_eq!(
                                admitted, expected,
                                "{stored_class}/{stored_method} -> {proposed_class} by \
                                 {proposed_method} with rationale {rationale:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The equal-class case stated on its own, because the property test above
    /// proves it holds everywhere and this one says what it is: a machine
    /// re-ingest of an item a human already classified keeps the human's
    /// record, and a genuine raise by that same machine still lands.
    #[test]
    fn a_machine_may_raise_a_humans_class_but_not_restate_it() {
        for machine in [METHOD_LEGACY, METHOD_DETERMINISTIC, METHOD_MODEL] {
            assert!(
                admit_reclassification("c1", METHOD_HUMAN, "c1", machine, "").is_err(),
                "{machine} restated a human's class and would have taken the record with it"
            );
            assert!(
                admit_reclassification("c1", METHOD_HUMAN, "c3", machine, "").is_ok(),
                "escalation is allowed to everyone, human-decided row or not"
            );
        }
        // A human revisiting their own decision is not a machine overwriting it.
        assert!(admit_reclassification("c1", METHOD_HUMAN, "c1", METHOD_HUMAN, "").is_ok());
        // And nothing here loosened the rules-over-rules case: a resweep that
        // re-decides at the same class still lands.
        assert!(
            admit_reclassification("c1", METHOD_DETERMINISTIC, "c1", METHOD_DETERMINISTIC, "")
                .is_ok()
        );
        // The named-person escalation comms runs after `classify_mail` is one
        // rank up, so a rule performs it without a human in the loop.
        assert!(
            admit_reclassification("c1", METHOD_DETERMINISTIC, "c2", METHOD_DETERMINISTIC, "")
                .is_ok()
        );
    }

    /// Rank is what the rule is written in terms of, so its order is part of
    /// the contract rather than an implementation detail of the match arm.
    #[test]
    fn a_stricter_class_and_a_more_authoritative_method_rank_higher() {
        assert!(class_rank("c0") < class_rank("c1"));
        assert!(class_rank("c1") < class_rank("c2"));
        assert!(class_rank("c2") < class_rank("c3"));
        assert_eq!(class_rank("something-new"), None);
        // The retired vocabulary ranks nowhere, so a missed call site is
        // refused rather than silently ranked at the bottom.
        assert_eq!(class_rank("vault"), None);
        assert_eq!(class_rank("personal"), None);
        assert!(method_rank(METHOD_LEGACY) < method_rank(METHOD_DETERMINISTIC));
        assert!(method_rank(METHOD_DETERMINISTIC) < method_rank(METHOD_MODEL));
        assert!(method_rank(METHOD_MODEL) < method_rank(METHOD_HUMAN));
        assert_eq!(method_rank("source-default"), None);
    }

    /// A class outside the vocabulary is refused rather than ranked, in both
    /// positions. Reaching the comparison with an unknown value would make
    /// `None < Some(_)` decide it, and `None` sorts lowest — an unknown stored
    /// class would then be silently overwritable by anything.
    #[test]
    fn an_unknown_class_is_refused_rather_than_ranked() {
        assert!(
            admit_reclassification("something-new", METHOD_HUMAN, "c0", METHOD_HUMAN, "why")
                .is_err()
        );
        assert!(
            admit_reclassification("c3", METHOD_HUMAN, "something-new", METHOD_HUMAN, "why")
                .is_err()
        );
        assert!(admit_reclassification("c3", METHOD_HUMAN, "c0", "source-default", "why").is_err());
        // Same reasoning one column over: an unranked *stored* method would
        // make `None == Some(30)` false and quietly leave a human's record
        // overwritable at equal class.
        assert!(admit_reclassification("c3", "source-default", "c0", METHOD_HUMAN, "why").is_err());
        // A row still holding a retired literal is refused in either position
        // rather than mapped to something that looks close enough.
        assert!(admit_reclassification("vault", METHOD_HUMAN, "c3", METHOD_HUMAN, "why").is_err());
        assert_eq!(
            admit_reclassification("c1", METHOD_HUMAN, "personal", METHOD_HUMAN, "why"),
            Err("data class must be one of: c0, c1, c2, c3")
        );
    }

    /// The fail-closed default is a value, not an absence: Mine, decided by
    /// nobody, and therefore replaceable by the first real decision of any kind
    /// without needing a special case for "unset".
    #[test]
    fn an_undeclared_item_is_mine_and_outranked_by_every_real_decision() {
        let undeclared = DataClass::undeclared();
        assert_eq!(undeclared.value, "c1");
        assert_eq!(undeclared.label, "Mine");
        assert_eq!(undeclared.method, METHOD_LEGACY);
        assert_eq!(
            processing_policy(&undeclared.value).cloud_handling,
            "pseudonymization_required"
        );
        for method in CLASSIFICATION_METHODS {
            assert!(method_rank(method) >= method_rank(&undeclared.method));
        }
        // Escalating it needs nobody's permission; lowering it to Public still
        // needs a human with a reason.
        assert!(admit_reclassification(
            &undeclared.value,
            &undeclared.method,
            "c3",
            METHOD_DETERMINISTIC,
            ""
        )
        .is_ok());
        assert!(admit_reclassification(
            &undeclared.value,
            &undeclared.method,
            "c0",
            METHOD_DETERMINISTIC,
            ""
        )
        .is_err());
    }

    #[test]
    fn ordinary_mail_is_mine_and_never_public() {
        let result = DataClass::classify_mail("aktiv", "erika@example.com", "Weekend plan");
        assert_eq!(result.value, "c1");
        assert_eq!(result.rationale, "Mail metadata is Mine by default.");
        assert_eq!(result.method, METHOD_DETERMINISTIC);
        assert_eq!(result.version, "data-class-rules-v2");
        assert_eq!(
            processing_policy(&result.value).cloud_handling,
            "pseudonymization_required"
        );
        assert!(!redact_before_persistence(&result.value));
    }

    /// The authentication rule owns c3 alone (Q27). Its output is the one class
    /// that reaches no prompt at all, so the policy assertion belongs here
    /// rather than in a table two files away.
    #[test]
    fn authentication_mail_is_secret() {
        for (from, subject) in [
            ("account@example.com", "Your verification code"),
            ("account@example.com", "Security alert: new sign in"),
            ("noreply@example.com", "Ihr Bestätigungscode"),
            ("noreply@example.com", "Passwort zurücksetzen"),
        ] {
            let result = DataClass::classify_mail("aktiv", from, subject);
            assert_eq!(result.value, "c3", "{subject} should be Secret");
            assert_eq!(
                result.rationale,
                "Authentication or account-recovery metadata is Secret."
            );
            assert!(redact_before_persistence(&result.value));
        }
        assert_eq!(processing_policy("c3").local_processing, "blocked");
        assert_eq!(processing_policy("c3").cloud_handling, "blocked");
    }

    /// Tax, receipts, money and health are somebody else's facts, not a
    /// credential: c2, redacted before it is stored, and never cloud-eligible.
    #[test]
    fn tax_receipt_financial_and_health_mail_is_others() {
        for (stream, from, subject) in [
            ("belege", "shop@example.com", "Your order"),
            ("steuern", "amt@example.com", "Bescheid"),
            ("aktiv", "bank@example.com", "Kontoauszug Juli"),
            ("aktiv", "praxis@example.com", "Ihr Befund"),
            ("aktiv", "versicherung@example.com", "Ihre Versicherung"),
        ] {
            let result = DataClass::classify_mail(stream, from, subject);
            assert_eq!(result.value, "c2", "{subject} should be Others");
            assert!(redact_before_persistence(&result.value));
        }
        assert_eq!(processing_policy("c2").cloud_handling, "blocked");
    }

    /// The split's whole ordering question, pinned: a receipt stream carrying a
    /// one-time code matches both rules, and a credential outranks a fact about
    /// somebody else (Q27).
    #[test]
    fn a_credential_in_a_receipt_stream_is_secret_not_others() {
        let result = DataClass::classify_mail("belege", "shop@example.com", "Your one-time code");
        assert_eq!(result.value, "c3");
        assert_eq!(
            result.rationale,
            "Authentication or account-recovery metadata is Secret."
        );
        // And the stream alone, with no credential in it, still lands on c2.
        assert_eq!(
            DataClass::classify_mail("belege", "shop@example.com", "Your order").value,
            "c2"
        );
    }

    /// The previous rule matched `" otp "` with literal surrounding spaces, so
    /// a subject that *began* with the code word slipped through as the default
    /// class — exactly the shape a one-time-code mail actually has.
    #[test]
    fn a_code_word_at_a_string_boundary_still_classifies_as_secret() {
        for subject in ["OTP for your login", "Login 2FA", "your otp"] {
            assert_eq!(
                DataClass::classify_mail("aktiv", "noreply@example.com", subject).value,
                "c3",
                "{subject} should be Secret"
            );
        }
    }

    /// Whole-word matching has to cut both ways, or the conservative default
    /// turns into "everything is Secret" and the class stops carrying signal.
    #[test]
    fn a_code_word_inside_an_unrelated_word_does_not_trigger() {
        assert_eq!(
            DataClass::classify_mail("aktiv", "erika@example.com", "Laptop-Adapter mitbringen?")
                .value,
            "c1"
        );
    }

    /// A skipped digest is a claim about the source, not a missing value: the
    /// reader has to be able to say "too short to be worth one" rather than
    /// showing an empty box that looks like a failure.
    #[test]
    fn a_skipped_digest_carries_its_verdict_and_no_text() {
        let mut item = minimal("mail");
        item.digest = Some(Digest {
            text: None,
            state: "skipped_short".into(),
            shape: "none".into(),
            depth: "standard".into(),
            focus: Vec::new(),
            producer: "openai|http://127.0.0.1:8080|gemma:content-digest-v1-adaptive".into(),
            source_chars: 148,
            redactions: 0,
            attempts: 0,
            last_error: None,
            diagram: None,
            diagram_state: None,
            diagram_error: None,
            chart: None,
            chart_state: None,
            chart_error: None,
            generated_at: "2026-08-05T12:00:00Z".into(),
        });
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["digest"]["state"], "skipped_short");
        assert!(value["digest"]["text"].is_null());
        assert_eq!(value["digest"]["source_chars"], 148);
        assert_eq!(value["digest"]["focus"], json!([]));
    }

    /// `summary` and `digest` are different nouns and both may be present:
    /// calendar's summary is the source's own description, and a generated
    /// digest must not be written over it.
    #[test]
    fn a_digest_never_replaces_the_sources_own_summary() {
        let mut item = minimal("calendar");
        item.summary = Some("A theme park in Brühl.".into());
        item.digest = Some(Digest {
            text: Some("- Opens 09:00\n- Ticket is dated".into()),
            state: "generated".into(),
            shape: "brief".into(),
            depth: "detailed".into(),
            focus: vec!["opening hours".into()],
            producer: "p".into(),
            source_chars: 1_400,
            redactions: 0,
            attempts: 1,
            last_error: None,
            diagram: None,
            diagram_state: None,
            diagram_error: None,
            chart: None,
            chart_state: None,
            chart_error: None,
            generated_at: "2026-08-05T12:00:00Z".into(),
        });
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["summary"], "A theme park in Brühl.");
        assert_eq!(value["digest"]["depth"], "detailed");
        assert_eq!(value["digest"]["focus"], json!(["opening hours"]));
    }

    #[test]
    fn not_prepared_claims_no_provider_was_called() {
        let state = CloudProcessing::not_prepared();
        assert_eq!(state.status, "not_prepared");
        assert_eq!(state.dispatch_status, "not_queued");
        assert_eq!(state.provider_calls, 0);
        assert!(state.result.is_none());
    }

    #[test]
    fn a_calendar_item_carries_commitment_and_no_score() {
        let mut item = minimal("calendar");
        item.calendar = Some(CalendarExtension {
            starts_at: "2026-08-10T09:00:00".into(),
            ends_at: "2026-08-10T19:00:00".into(),
            all_day: false,
            commitment: "committed".into(),
            location: Some("Brühl".into()),
            notes: Some("Dated ticket.".into()),
            entry_source: "manual".into(),
            rhythm_id: None,
        });
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["calendar"]["commitment"], "committed");
        assert_eq!(
            value["status"], "committed",
            "status mirrors the triage axis of the source"
        );
        assert_eq!(
            value["relevance"],
            json!([]),
            "a decided item is not ranked"
        );
        assert!(value["evaluation"].is_null());
        assert!(value["mail"].is_null(), "one extension at a time");
    }

    // ── Q9a: folder default, frontmatter override ──────────────────────

    #[test]
    fn the_three_named_folders_are_others() {
        for id in [
            "Atlas/People/Some Person.md",
            "Atlas/Documents/Mietvertrag.md",
            "Atlas/Finance/Depot.md",
        ] {
            let c = DataClass::classify_vault_note(id, None);
            assert_eq!(c.value, "c2", "{id}");
            assert_eq!(c.label, "Others");
            assert_eq!(c.method, METHOD_DETERMINISTIC);
            assert_eq!(c.version, VAULT_CLASSIFIER_VERSION);
        }
    }

    #[test]
    fn everything_else_is_mine_and_nothing_is_public() {
        for id in [
            "Journal/2026-09-07.md",
            "Projects/Axon/PRD Axon.md",
            "Atlas/Personal/Ziele.md",
            "Home.md",
        ] {
            let c = DataClass::classify_vault_note(id, None);
            assert_eq!(c.value, "c1", "{id}");
        }
        // c0 is unreachable from a location. Publishing is an act (§15).
        assert!(!DATA_CLASSES.iter().any(|_| DataClass::classify_vault_note(
            "Clippings/Public Post.md",
            None
        )
        .value
            == "c0"));
    }

    #[test]
    fn a_health_folder_is_others_wherever_it_sits() {
        for id in [
            "Atlas/Documents/Gesundheit/Befund.md",
            "Projects/Health/Plan.md",
            "Resources/Medical/Notes.md",
        ] {
            assert_eq!(DataClass::classify_vault_note(id, None).value, "c2", "{id}");
        }
    }

    #[test]
    fn a_title_is_not_a_folder() {
        // The failure this rules out: a substring or filename match sweeping
        // articles about a subject into the class meant for people's records.
        for id in [
            "Knowledge/Mental Health at Work.md",
            "Knowledge/People Analytics.md",
            "Knowledge/Personal Finance.md",
        ] {
            assert_eq!(DataClass::classify_vault_note(id, None).value, "c1", "{id}");
        }
    }

    #[test]
    fn frontmatter_overrides_in_both_directions() {
        // Down: a People note the operator says is not about anyone.
        let down = DataClass::classify_vault_note("Atlas/People/Method.md", Some("c1"));
        assert_eq!(down.value, "c1");
        assert_eq!(down.method, METHOD_HUMAN);
        // Up: an ordinary note that holds a credential.
        let up = DataClass::classify_vault_note("Journal/2026-09-07.md", Some("c3"));
        assert_eq!(up.value, "c3");
        assert_eq!(up.method, METHOD_HUMAN);
        // Whitespace and an empty key are not declarations.
        assert_eq!(
            DataClass::classify_vault_note("Journal/x.md", Some("  ")).method,
            METHOD_DETERMINISTIC
        );
        assert_eq!(
            DataClass::classify_vault_note("Atlas/People/x.md", Some(" c1 ")).value,
            "c1"
        );
    }

    #[test]
    fn a_typo_falls_back_to_the_folder_and_says_so() {
        let c = DataClass::classify_vault_note("Atlas/People/Some Person.md", Some("c22"));
        assert_eq!(c.value, "c2", "the folder answers, not the typo");
        assert_eq!(c.method, METHOD_DETERMINISTIC);
        assert!(c.rationale.contains("c22"), "the rejected literal is named");
        // And it does not silently loosen a note whose folder is Mine.
        let loose = DataClass::classify_vault_note("Journal/x.md", Some("public"));
        assert_eq!(loose.value, "c1");
        assert!(loose.rationale.contains("public"));
    }
}
