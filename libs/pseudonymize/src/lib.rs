//! Reversible session pseudonymization for cloud evaluators and agents.
//!
//! Provides bidirectional tokenization of personal and operational entities
//! (people, locations, transit stations, and syntactic PII) so that structured
//! requests can be dispatched to remote evaluators (such as Jev/TypeSafe AI or cloud LLMs)
//! without leaking raw C1 personal state off-device.

pub mod keyed;
pub mod pattern;
pub mod registry;
pub mod session;
pub mod types;
pub mod view;

pub use registry::{EntityRegistry, EntityRegistryBuilder};
pub use session::PseudonymizerSession;
pub use types::{
    compute_digest, format_receipt, EntityType, RedactionFinding, PSEUDONYMIZE_VERSION,
};

/// An immutable pseudonymizer holding entity dictionaries and pattern rules.
pub struct Pseudonymizer {
    registry: EntityRegistry,
}

impl Pseudonymizer {
    pub fn new(registry: EntityRegistry) -> Self {
        Self { registry }
    }

    pub fn builder() -> EntityRegistryBuilder {
        EntityRegistryBuilder::new()
    }

    pub fn registry(&self) -> &EntityRegistry {
        &self.registry
    }

    pub fn new_session(&self) -> PseudonymizerSession {
        PseudonymizerSession::new()
    }

    pub fn tokenize_text(&self, session: &mut PseudonymizerSession, text: &str) -> String {
        session.tokenize_text(text, &self.registry)
    }

    pub fn tokenize_json(&self, session: &mut PseudonymizerSession, value: &mut serde_json::Value) {
        session.tokenize_json(value, &self.registry);
    }

    pub fn rehydrate_text(&self, session: &PseudonymizerSession, text: &str) -> String {
        session.rehydrate_text(text)
    }

    pub fn rehydrate_json(&self, session: &PseudonymizerSession, value: &mut serde_json::Value) {
        session.rehydrate_json(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_engine() -> Pseudonymizer {
        let registry = EntityRegistry::builder()
            .add_people(["Lars", "Anna", "Dr. Müller"])
            .add_places([
                "Karlsruhe Hbf",
                "Berlin Hauptbahnhof",
                "München Hbf",
                "Bonn",
            ])
            .build();
        Pseudonymizer::new(registry)
    }

    #[test]
    fn roundtrip_fidelity_free_text() {
        let engine = test_engine();
        let mut session = engine.new_session();

        let original = "Can Lars and Anna meet at Karlsruhe Hbf before going to München Hbf?";
        let tokenized = engine.tokenize_text(&mut session, original);

        assert!(tokenized.contains("<TRAVELER_01>"));
        assert!(tokenized.contains("<TRAVELER_02>"));
        assert!(tokenized.contains("<PLACE_01>"));
        assert!(tokenized.contains("<PLACE_02>"));
        assert!(!tokenized.contains("Lars"));
        assert!(!tokenized.contains("Karlsruhe Hbf"));

        let rehydrated = engine.rehydrate_text(&session, &tokenized);
        assert_eq!(rehydrated, original);
    }

    #[test]
    fn consistent_tokens_across_multiple_mentions() {
        let engine = test_engine();
        let mut session = engine.new_session();

        let text = "Lars departs from Karlsruhe Hbf. Lars prefers Karlsruhe Hbf.";
        let tokenized = engine.tokenize_text(&mut session, text);

        assert_eq!(
            tokenized,
            "<TRAVELER_01> departs from <PLACE_01>. <TRAVELER_01> prefers <PLACE_01>."
        );
        assert_eq!(session.token_count(), 2);

        let rehydrated = engine.rehydrate_text(&session, &tokenized);
        assert_eq!(rehydrated, text);
    }

    #[test]
    fn longest_match_precedence() {
        let registry = EntityRegistry::builder()
            .add_places(["Berlin", "Berlin Hauptbahnhof"])
            .build();
        let engine = Pseudonymizer::new(registry);
        let mut session = engine.new_session();

        let text = "Arrival at Berlin Hauptbahnhof on platform 3.";
        let tokenized = engine.tokenize_text(&mut session, text);

        assert_eq!(tokenized, "Arrival at <PLACE_01> on platform 3.");
        assert_eq!(
            session.reverse_map().get("<PLACE_01>"),
            Some(&"Berlin Hauptbahnhof".to_string())
        );
    }

    #[test]
    fn syntactic_pii_detection_and_receipt() {
        let engine = test_engine();
        let mut session = engine.new_session();

        let text =
            "Send details to lars@example.com or call +491701234567. IBAN: DE89370400440532013000";
        let tokenized = engine.tokenize_text(&mut session, text);

        assert!(tokenized.contains("<EMAIL_01>"));
        assert!(tokenized.contains("<PHONE_01>"));
        assert!(tokenized.contains("<ACCOUNT_01>"));

        let receipt = session.receipt().expect("receipt should exist");
        assert!(receipt.contains("Reduced 3 details before this call:"));
        assert!(receipt.contains("1 email address"));
        assert!(receipt.contains("1 phone number"));
        assert!(receipt.contains("1 account number"));

        let rehydrated = engine.rehydrate_text(&session, &tokenized);
        assert_eq!(rehydrated, text);
    }

    #[test]
    fn contextual_person_cue() {
        let engine = test_engine();
        let mut session = engine.new_session();

        // "Fabian" is not in the registry, but cued by "Hello Fabian" and "meet Fabian from Acme"
        let text = "Hello Fabian, please meet Sophie from TechCorp at the station.";
        let tokenized = engine.tokenize_text(&mut session, text);

        assert!(tokenized.contains("<TRAVELER_01>")); // Fabian
        assert!(tokenized.contains("<TRAVELER_02>")); // Sophie

        let rehydrated = engine.rehydrate_text(&session, &tokenized);
        assert_eq!(rehydrated, text);
    }

    #[test]
    fn structured_json_tree_roundtrip() {
        let engine = test_engine();
        let mut session = engine.new_session();

        let mut data = json!({
            "intent": "plan_travel",
            "departure": {
                "station": "Karlsruhe Hbf",
                "passenger": "Lars"
            },
            "destinations": [
                {"name": "München Hbf", "contact": "anna@example.com"}
            ],
            "metadata": {
                "trip_id": "trip-123",
                "notes": "Lars traveling to München Hbf with Anna."
            }
        });

        let original_data = data.clone();
        engine.tokenize_json(&mut session, &mut data);

        // Keys must remain completely untouched
        assert_eq!(data["intent"], "plan_travel");
        assert_eq!(data["metadata"]["trip_id"], "trip-123");

        // Values must be tokenized
        assert_eq!(data["departure"]["station"], "<PLACE_01>");
        assert_eq!(data["departure"]["passenger"], "<TRAVELER_01>");
        assert_eq!(data["destinations"][0]["name"], "<PLACE_02>");
        assert_eq!(data["destinations"][0]["contact"], "<EMAIL_01>");
        assert_eq!(
            data["metadata"]["notes"],
            "<TRAVELER_01> traveling to <PLACE_02> with <TRAVELER_02>."
        );

        // Rehydrate JSON
        engine.rehydrate_json(&session, &mut data);
        assert_eq!(data, original_data);
    }

    #[test]
    fn digest_is_deterministic() {
        let engine = test_engine();
        let mut s1 = engine.new_session();
        let mut s2 = engine.new_session();

        s1.tokenize_text("Lars at Karlsruhe Hbf", engine.registry());
        s2.tokenize_text("Anna at München Hbf", engine.registry());

        // Both redacted 1 person and 1 place -> digests must match!
        assert_eq!(s1.digest(), s2.digest());
        assert_eq!(s1.digest().len(), 64);
    }

    #[test]
    fn rehydrate_case_insensitive_tokens() {
        let engine = test_engine();
        let mut session = engine.new_session();

        let tokenized = engine.tokenize_text(&mut session, "Lars goes to Karlsruhe Hbf.");
        assert_eq!(tokenized, "<TRAVELER_01> goes to <PLACE_01>.");

        // Simulate external LLM returning lowercase tokens
        let llm_reply = "Selected: <traveler_01> arrives at <place_01> at 10:00.";
        let rehydrated = engine.rehydrate_text(&session, llm_reply);
        assert_eq!(
            rehydrated,
            "Selected: Lars arrives at Karlsruhe Hbf at 10:00."
        );
    }

    #[test]
    fn empty_and_whitespace_text_handling() {
        let engine = test_engine();
        let mut session = engine.new_session();

        assert_eq!(engine.tokenize_text(&mut session, ""), "");
        assert_eq!(engine.tokenize_text(&mut session, "   \n\t  "), "   \n\t  ");
        assert_eq!(session.token_count(), 0);
        assert!(session.receipt().is_none());
    }
}
