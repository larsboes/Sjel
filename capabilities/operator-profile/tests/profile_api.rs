use operator_profile::{
    model::{
        Classification, FieldInput, FieldValue, Harness, ProfileCategory, ProfileInput,
        StatementInput,
    },
    store::OperatorProfileStore,
};
use std::collections::BTreeMap;

fn input() -> ProfileInput {
    ProfileInput {
        schema_version: 1,
        fields: BTreeMap::from([(
            "verbosity".into(),
            FieldInput {
                category: ProfileCategory::InteractionStyle,
                value: FieldValue::Text("concise".into()),
                classification: Classification::C1,
                export_to: vec![Harness::Claude],
            },
        )]),
        statements: BTreeMap::from([(
            "boundary".into(),
            StatementInput {
                category: ProfileCategory::Boundary,
                text: "Ask before external actions.".into(),
                classification: Classification::C1,
                export_to: vec![Harness::Claude],
            },
        )]),
    }
}

#[test]
fn profile_input_round_trips_as_json_without_logging_values() {
    let original = input();
    original.validate().unwrap();
    let encoded = serde_json::to_string(&original).unwrap();
    let decoded: ProfileInput = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, original);
    assert!(!format!("{original:?}").contains("concise"));
}

#[test]
fn profile_store_refuses_unconditional_stale_write() {
    let root = std::env::temp_dir().join(format!("sjel-profile-api-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let store = OperatorProfileStore::open(&root.join("profile.db")).unwrap();
    assert!(matches!(
        store.put(&input(), 0).unwrap(),
        operator_profile::PutOutcome::Stored(_)
    ));
    assert!(matches!(
        store.put(&input(), 0).unwrap(),
        operator_profile::PutOutcome::Stale {
            current_revision: 1
        }
    ));
    std::fs::remove_dir_all(root).unwrap();
}
