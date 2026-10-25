//! One path from a swept Gmail thread to a stored proposal.
//!
//! Both sweep entry points — the `comms` CLI and comms-server's HTTP sweep —
//! come through here, and that is the entire point of the module existing. The
//! redaction gate below is only a gate if there is no second way in; when the
//! two call sites each built their own `TriageItem` they had already drifted
//! apart once, and a gate added to one of them would have been a gate over
//! half the traffic.
//!
//! What the gate does: classify from metadata, then, for the classes whose
//! metadata is itself the payload, redact the stored review fields *before*
//! the row is written. A one-time code arrives in a subject line; persisting
//! that subject verbatim publishes it to a log, an API response and a
//! dashboard in one step, and no later deletion un-publishes it.

use crate::cloud_derivative::{redact_review_field, redaction_digest, RedactionFinding};
use crate::content_item::{self, DataClass};
use crate::google::ThreadMeta;
use crate::people_registry;
use crate::rules;
use crate::store::TriageItem;

/// Q27's wording for the named-person escalation. One constant because the
/// sweep and the refresh pass both write it, and a row must not read
/// differently depending on which one got there first.
pub const KNOWN_PERSON_RATIONALE: &str =
    "Names a person the vault knows; facts about them stay local.";

/// Classify a mail, then raise it to `c2` if it names someone the vault knows.
///
/// Q27's rule, and it is asked only of a `c1` result: `c2` and `c3` are already
/// local-only, and `c0` a mailbox never produces. Only the subject and the
/// snippet are read, never the sender — an address is the correspondent, and
/// this rule is about a third party the correspondence is *about*.
///
/// The tokens are `cloud_derivative::transform_text`'s tokens, because
/// `people_registry::is_known_person` is written against exactly those: split
/// on whitespace, punctuation stripped by the registry itself.
///
/// An absent registry escalates nothing, which is the existing degrade
/// behaviour and not a silent one: `POST /triage/data-class/refresh` reports
/// the registry's state, so a blind run is visible in the receipt.
pub fn classify_mail(stream: &str, from: &str, subject: &str, snippet: &str) -> DataClass {
    classify_mail_against(
        stream,
        from,
        subject,
        snippet,
        people_registry::is_known_person,
    )
}

/// The registry is a parameter here because it is process-wide state loaded
/// from the overlay: a test that wanted to pin this rule would otherwise be
/// pinning whichever names the machine happens to hold.
fn classify_mail_against(
    stream: &str,
    from: &str,
    subject: &str,
    snippet: &str,
    is_known_person: impl Fn(&str) -> bool,
) -> DataClass {
    let classification = DataClass::classify_mail(stream, from, subject);
    if classification.value != "c1" {
        return classification;
    }
    let names_someone = [subject, snippet]
        .into_iter()
        .flat_map(str::split_whitespace)
        .any(is_known_person);
    if !names_someone {
        return classification;
    }
    DataClass::new(
        "c2",
        KNOWN_PERSON_RATIONALE,
        content_item::METHOD_DETERMINISTIC,
        content_item::MAIL_CLASSIFIER_VERSION,
    )
}

/// A proposal that is safe to persist, plus the evidence of what was removed
/// to make it safe. The findings carry counts and entity types only — never
/// the matched text.
#[derive(Debug, Clone)]
pub struct Intake {
    pub item: TriageItem,
    /// Safe boolean computed from transient pre-redaction metadata. Never store the
    /// matching text: this signal only admits likely travel/event mail to local analysis.
    pub event_candidate: bool,
    pub redactions: Vec<RedactionFinding>,
    /// Which deterministic rung decided this thread's stream. Carried out of
    /// `rules::classify` because it cannot be re-derived from the stored row:
    /// the classifier reads `List-Unsubscribe`, a header no column holds.
    /// `Store::upsert_triage_with_rules` persists it beside the row.
    pub decided_by: rules::DecidedBy,
}

impl Intake {
    /// A digest identifying this redaction pass, for provenance. Empty when
    /// nothing was removed, so callers can tell "clean" from "cleaned".
    pub fn audit_digest(&self) -> Option<String> {
        (!self.redactions.is_empty()).then(|| redaction_digest(&self.redactions))
    }

    pub fn redaction_count(&self) -> usize {
        self.redactions.iter().map(|finding| finding.count).sum()
    }

    /// The deterministic verdict this intake was built from, as
    /// `Store::upsert_triage_with_rules` wants it.
    ///
    /// Rebuilt from the stored row rather than kept whole: `TriageItem` already
    /// owns `stream` and `rationale`, and a second copy on `Intake` would be a
    /// second place for them to disagree. The rung is the only field the item
    /// does not carry.
    pub fn verdict(&self) -> rules::Verdict {
        rules::Verdict {
            stream: self.item.stream.clone(),
            rationale: self.item.rationale.clone(),
            decided_by: self.decided_by,
        }
    }
}

/// Classify a swept thread and build the row to store, redacting first when
/// the class demands it.
pub fn from_thread(meta: ThreadMeta, config_rules: &[rules::Rule]) -> Intake {
    from_thread_against(meta, config_rules, people_registry::is_known_person)
}

/// `from_thread` with the registry as a parameter, for the same reason
/// `classify_mail_against` exists one level up.
///
/// Without it, every test of the redaction gate asserts against whichever names
/// this machine's overlay happens to hold: `ordinary_mail_is_stored_verbatim`
/// claims a c1 row keeps its subject, and that claim is only true while no word
/// in the fixture is a name the operator knows. The registry is a process-wide
/// `OnceLock` (`people_registry::registry`), so an environment variable cannot
/// pin it per test either — whichever test touches it first decides for the
/// whole binary.
fn from_thread_against(
    meta: ThreadMeta,
    config_rules: &[rules::Rule],
    is_known_person: impl Fn(&str) -> bool,
) -> Intake {
    let from = meta.from_addr.clone().unwrap_or_default();
    let subject = meta.subject.clone().unwrap_or_default();
    let facts = rules::MailFacts {
        from: &from,
        subject: &subject,
        has_list_unsubscribe: meta.has_list_unsubscribe(),
    };
    let verdict = rules::classify(&facts, config_rules);
    let classification = classify_mail_against(
        &verdict.stream,
        &from,
        &subject,
        meta.snippet.as_deref().unwrap_or_default(),
        is_known_person,
    );
    let event_candidate = crate::mail_events::is_candidate_metadata(
        &from,
        &subject,
        meta.snippet.as_deref().unwrap_or_default(),
    );

    let mut redactions = Vec::new();
    let (subject, snippet) = if content_item::redact_before_persistence(&classification.value) {
        (
            redact_review_field(meta.subject.as_deref(), &mut redactions),
            redact_review_field(meta.snippet.as_deref(), &mut redactions),
        )
    } else {
        (meta.subject.clone(), meta.snippet.clone())
    };

    Intake {
        event_candidate,
        item: TriageItem {
            id: meta.id,
            // The sender survives redaction deliberately. It is the one field a
            // human needs to recognize a proposal they cannot read the subject
            // of, and it is already personal rather than secret.
            from_addr: meta.from_addr,
            subject,
            snippet,
            internal_date_ms: meta.internal_date_ms,
            internal_date_text: None,
            stream: verdict.stream,
            rationale: verdict.rationale,
            classification_method: content_item::METHOD_DETERMINISTIC.into(),
            classification_version: rules::MAIL_RULES_VERSION.into(),
            data_class: classification.value,
            data_class_rationale: classification.rationale,
            data_classification_method: classification.method,
            data_classification_version: classification.version,
            status: "proposed".into(),
            gmail_action: None,
            gmail_action_at: None,
            purge_after: None,
            gmail_location: None,
            gmail_observed_at: None,
            gmail_sync_status: None,
            gmail_sync_action: None,
            // A freshly swept proposal is not waiting on anyone: the label is only
            // ever set by an explicit action, never inferred from a sweep.
            gmail_sync_error: None,
            waiting: false,
            waiting_since: None,
            first_seen: String::new(),
            last_seen: String::new(),
        },
        redactions,
        decided_by: verdict.decided_by,
    }
}

/// Redact an already-stored row's review fields. Returns `None` when the row's
/// class does not require redaction, so a caller can distinguish "left alone
/// on purpose" from "scanned and already clean".
pub fn remediate(
    data_class: &str,
    subject: Option<&str>,
    snippet: Option<&str>,
) -> Option<Remediation> {
    if !content_item::redact_before_persistence(data_class) {
        return None;
    }
    let mut redactions = Vec::new();
    let redacted_subject = redact_review_field(subject, &mut redactions);
    let redacted_snippet = redact_review_field(snippet, &mut redactions);
    Some(Remediation {
        changed: redacted_subject.as_deref() != subject || redacted_snippet.as_deref() != snippet,
        subject: redacted_subject,
        snippet: redacted_snippet,
        audit_digest: (!redactions.is_empty()).then(|| redaction_digest(&redactions)),
        redaction_count: redactions.iter().map(|finding| finding.count).sum(),
        redactions,
    })
}

#[derive(Debug, Clone)]
pub struct Remediation {
    pub subject: Option<String>,
    pub snippet: Option<String>,
    /// False when the pass ran and found nothing — which is the expected
    /// result on a second run, and is what makes remediation idempotent.
    pub changed: bool,
    pub audit_digest: Option<String>,
    pub redaction_count: usize,
    pub redactions: Vec<RedactionFinding>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(subject: &str, snippet: &str) -> ThreadMeta {
        ThreadMeta {
            id: "thread-1".into(),
            from_addr: Some("security@example.com".into()),
            subject: Some(subject.into()),
            date: None,
            list_unsubscribe: None,
            snippet: Some(snippet.into()),
            label_ids: Vec::new(),
            internal_date_ms: Some(1_754_000_000_000),
        }
    }

    /// A registry these tests own. `from_thread` reads the machine's — the file
    /// `people_registry`'s own module doc calls C2 data — and a gate asserted
    /// against the operator's contact list is a gate whose result changes when
    /// they meet someone. Every fixture below therefore states its registry,
    /// and the two that assert a verbatim subject state an empty one.
    fn knows_nobody(_: &str) -> bool {
        false
    }

    fn intake_of(subject: &str, snippet: &str) -> Intake {
        from_thread_against(meta(subject, snippet), &[], knows_nobody)
    }

    /// The case that produced Sjel#129: a live sweep stored a one-time code
    /// because it arrived in the subject line, where nothing was looking.
    #[test]
    fn a_one_time_code_never_reaches_the_stored_row() {
        let intake = intake_of(
            "Your verification code is 448215",
            "Use 448215 to sign in, or open https://example.com/verify?t=AbCd1234567890Ef",
        );

        assert_eq!(intake.item.data_class, "c3");
        let subject = intake.item.subject.clone().unwrap();
        let snippet = intake.item.snippet.clone().unwrap();
        assert!(!subject.contains("448215"), "the code survived: {subject}");
        assert!(!snippet.contains("448215"), "the code survived: {snippet}");
        assert!(!snippet.contains("AbCd1234567890Ef"), "token survived");
        assert!(subject.contains("[number]"));
        assert!(snippet.contains("[link]"));
        assert!(intake.redaction_count() >= 3);
        assert!(intake.audit_digest().is_some());
    }

    /// Redaction is scoped by class, not applied to everything: an ordinary
    /// mail stays readable, or the review list becomes useless and gets
    /// ignored, which is its own safety failure.
    #[test]
    fn ordinary_mail_is_stored_verbatim() {
        let intake = intake_of("Lunch on Tuesday?", "Half twelve at the usual place");
        assert_eq!(intake.item.data_class, "c1");
        assert_eq!(intake.item.subject.as_deref(), Some("Lunch on Tuesday?"));
        assert_eq!(
            intake.item.snippet.as_deref(),
            Some("Half twelve at the usual place")
        );
        assert!(intake.redactions.is_empty());
        assert!(intake.audit_digest().is_none());
    }

    #[test]
    fn event_candidate_survives_metadata_redaction_without_retaining_its_signal() {
        let knows_alice = |token: &str| token.trim_matches(',') == "Alice";
        let intake = from_thread_against(
            meta(
                "Fw: OpenAI DevDay Exchange alice@example.com",
                "Alice registered",
            ),
            &[],
            knows_alice,
        );
        assert_eq!(intake.item.data_class, "c2");
        assert_ne!(
            intake.item.subject.as_deref(),
            Some("Fw: OpenAI DevDay Exchange alice@example.com")
        );
        assert!(!intake
            .item
            .subject
            .as_deref()
            .unwrap()
            .contains("alice@example.com"));
        assert!(intake.event_candidate);
    }

    /// Q27's named-person rule, against a registry this test owns rather than
    /// the machine's. A c1 mail that names someone the vault knows becomes c2,
    /// and c2 is redacted before it is stored — so the escalation has to happen
    /// before the redaction gate, not after it.
    #[test]
    fn a_mail_that_names_a_known_person_is_others_and_gets_redacted() {
        let knows_mustermann = |token: &str| token.trim_matches(',') == "Mustermann";

        let escalated = classify_mail_against(
            "aktiv",
            "kollege@example.com",
            "Re: Mustermann, next week",
            "She asked about the schedule",
            knows_mustermann,
        );
        assert_eq!(escalated.value, "c2");
        assert_eq!(escalated.rationale, KNOWN_PERSON_RATIONALE);
        assert_eq!(escalated.method, content_item::METHOD_DETERMINISTIC);
        assert!(content_item::redact_before_persistence(&escalated.value));

        let from_the_snippet = classify_mail_against(
            "aktiv",
            "kollege@example.com",
            "Next week",
            "Mustermann asked about the schedule",
            knows_mustermann,
        );
        assert_eq!(from_the_snippet.value, "c2", "the snippet is read too");

        let unknown_registry = classify_mail_against(
            "aktiv",
            "kollege@example.com",
            "Re: Mustermann, next week",
            "She asked about the schedule",
            |_| false,
        );
        assert_eq!(
            unknown_registry.value, "c1",
            "an absent registry escalates nothing"
        );

        // And through the path that actually builds the stored row, so the
        // escalation is known to run before the redaction gate rather than
        // beside it. Only the class is asserted here: rung 1's own person
        // detector reads the process-wide registry
        // (`cloud_derivative.rs:351`), so which *tokens* come out is a property
        // of the machine even when the classifier's registry is injected.
        let swept = from_thread_against(
            meta("Re: Mustermann, next week", "She asked about the schedule"),
            &[],
            knows_mustermann,
        );
        assert_eq!(swept.item.data_class, "c2");
    }

    /// The sender is never read for the rule above: an address is the
    /// correspondent, not a third party the mail is about.
    #[test]
    fn a_known_person_in_the_sender_alone_does_not_escalate() {
        let classification = classify_mail_against(
            "aktiv",
            "mustermann@example.com",
            "Lunch on Tuesday?",
            "Half twelve at the usual place",
            |token| token == "mustermann",
        );
        assert_eq!(classification.value, "c1");
    }

    /// A credential outranks the registry: c3 is asked first, so a mail that is
    /// both never lands on c2.
    #[test]
    fn a_one_time_code_naming_a_known_person_stays_secret() {
        let classification = classify_mail_against(
            "aktiv",
            "security@example.com",
            "Mustermann, your verification code is 448215",
            "Use it to sign in",
            |token| token.trim_matches(',') == "Mustermann",
        );
        assert_eq!(classification.value, "c3");
    }

    /// The sender is what makes a redacted proposal reviewable at all.
    #[test]
    fn the_sender_survives_redaction() {
        let intake = intake_of("Security alert: new sign in", "Code 998877");
        assert_eq!(
            intake.item.from_addr.as_deref(),
            Some("security@example.com")
        );
    }

    #[test]
    fn remediation_is_idempotent_and_reports_the_second_pass_as_clean() {
        let first = remediate("c3", Some("Your code is 448215"), Some("expires soon"))
            .expect("c3 rows are in scope");
        assert!(first.changed);
        assert!(first.audit_digest.is_some());

        let second = remediate("c3", first.subject.as_deref(), first.snippet.as_deref())
            .expect("c3 rows are in scope");
        assert!(!second.changed, "a second pass must find nothing to remove");
        assert_eq!(second.subject, first.subject);
    }

    #[test]
    fn remediation_leaves_rows_it_does_not_own_alone() {
        assert!(remediate("c1", Some("Lunch on Tuesday?"), None).is_none());
        assert!(remediate("c0", Some("Release notes"), None).is_none());
    }

    /// Two passes that removed the same shapes must agree, or the digest is
    /// not evidence of anything.
    #[test]
    fn the_audit_digest_is_stable_for_the_same_findings() {
        let one = remediate("c3", Some("code 123456"), None).unwrap();
        let two = remediate("c3", Some("code 654321"), None).unwrap();
        assert_eq!(one.audit_digest, two.audit_digest);
        assert!(one.audit_digest.is_some());
    }
}
