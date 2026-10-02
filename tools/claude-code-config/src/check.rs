// tools/claude-code-config/src/check.rs — has the deployed file drifted from the baseline?
//
// This exists because retiring the managed layer gave something up. While the floor was
// root-owned, a session could not edit it; now the file belongs to the user, so a session
// can. Detection is the honest replacement for that — not enforcement, which is what the
// managed layer was and what this deliberately is not.
//
// The digest is FNV-1a over the canonical serialization, hand-rolled for the reason
// tools/storage/Cargo.toml gives about walkdir: a fingerprint of a few kilobytes is smaller
// than the argument for adding a hashing crate to carry it. It is not cryptographic and does
// not need to be — it answers "is this the same document" for a person reading a report.

use serde_json::Value;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Drift {
    /// Keys the baseline declares that the deployed file does not have.
    pub missing: Vec<String>,
    /// Keys both have, with different values.
    pub differing: Vec<String>,
}

impl Drift {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.differing.is_empty()
    }
}

/// Every dotted path where `deployed` fails to match what `baseline` declares.
pub fn drift(deployed: &Value, baseline: &Value) -> Drift {
    let mut found = Drift::default();
    walk(deployed, baseline, "", &mut found);
    found
}

fn walk(deployed: &Value, baseline: &Value, prefix: &str, found: &mut Drift) {
    let Some(baseline_map) = baseline.as_object() else {
        return;
    };
    let empty = serde_json::Map::new();
    let deployed_map = deployed.as_object().unwrap_or(&empty);
    for (key, baseline_value) in baseline_map {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match deployed_map.get(key) {
            None => found.missing.push(path),
            Some(deployed_value) => {
                if deployed_value.is_object() && baseline_value.is_object() {
                    walk(deployed_value, baseline_value, &path, found);
                } else if deployed_value != baseline_value {
                    found.differing.push(path);
                }
            }
        }
    }
}

/// The same document with every object's keys in sorted order, at every depth.
///
/// `digest` must not depend on key order, and `serde_json::Map` cannot be trusted to give it one.
/// The map is a `BTreeMap` only while no crate in the build enables `preserve_order`; that feature
/// is additive across the whole graph, and `libs/extraction` reaches it through `xberg`. So under
/// `cargo test --workspace` the map is an `IndexMap`, two literals with the same keys in a
/// different order hash differently, and `the_digest_is_key_order_independent` fails there while
/// passing in `cargo test -p sjel-claude-config`. Sorting here makes the answer the same either
/// way, and stops this tool depending on a feature flag it does not own.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// FNV-1a, 64-bit, over the canonical serialization of a document.
pub fn digest(value: &Value) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in canonical(value).to_string().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Short enough to read in a report, wide enough that a collision is not a practical worry.
/// Masked to 48 bits so the width is fixed: `{:012x}` alone is a minimum, and a full u64
/// prints sixteen digits, which would make the column ragged in a report.
pub fn short_digest(value: &Value) -> String {
    format!("{:012x}", digest(value) & 0x0000_ffff_ffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn identical_documents_have_no_drift() {
        let document = json!({"permissions": {"deny": ["a"]}, "model": "m"});
        assert!(drift(&document, &document).is_clean());
    }

    #[test]
    fn an_emptied_deny_list_is_reported_as_differing_not_missing() {
        // The exact case the managed layer used to make impossible: the key is there, the
        // rules are gone.
        let deployed = json!({"permissions": {"deny": []}});
        let baseline = json!({"permissions": {"deny": ["Read(~/.ssh/**)"]}});
        let found = drift(&deployed, &baseline);
        assert_eq!(found.differing, vec!["permissions.deny"]);
        assert!(found.missing.is_empty());
    }

    #[test]
    fn a_removed_key_is_reported_as_missing() {
        let deployed = json!({"permissions": {}});
        let baseline = json!({"permissions": {"deny": ["a"]}});
        assert_eq!(
            drift(&deployed, &baseline).missing,
            vec!["permissions.deny"]
        );
    }

    #[test]
    fn a_key_the_baseline_does_not_declare_is_not_drift() {
        // Personal additions are the point of a user-level file, not a problem to report.
        let deployed = json!({"model": "mine", "permissions": {"deny": ["a"]}});
        let baseline = json!({"permissions": {"deny": ["a"]}});
        assert!(drift(&deployed, &baseline).is_clean());
    }

    #[test]
    fn an_empty_deployment_reports_each_available_subtree_at_its_top() {
        // Same rule as merge: report the highest level that diverges. A subtree that is
        // absent entirely is one line, not one line per rule inside it. The counterpart — a
        // key missing beneath a parent that IS present, so the leaf is what is named — is
        // `a_removed_key_is_reported_as_missing` below.
        let baseline = json!({"a": 1, "b": {"c": 2}});
        let found = drift(&json!({}), &baseline);
        assert_eq!(found.missing, vec!["a", "b"]);
    }

    #[test]
    fn the_digest_is_key_order_independent() {
        let a = json!({"x": 1, "y": [1, 2]});
        let b = json!({"y": [1, 2], "x": 1});
        assert_eq!(digest(&a), digest(&b));
        assert_eq!(short_digest(&a).len(), 12);

        // Nested, because the recursion is the part a fix is likely to leave half done, and
        // because the feature that breaks this is enabled by another crate's dependency rather
        // than by anything declared here. `preserve_order` reaches the whole workspace through
        // `libs/extraction` -> `xberg`, so this passes under `-p sjel-claude-config` and fails
        // under `cargo test --workspace` unless the sort happens at every depth.
        let deep_a = json!({"outer": {"x": 1, "y": 2}, "list": [{"b": 1, "a": 2}]});
        let deep_b = json!({"list": [{"a": 2, "b": 1}], "outer": {"y": 2, "x": 1}});
        assert_eq!(digest(&deep_a), digest(&deep_b));
    }

    #[test]
    fn the_short_digest_keeps_its_width_for_any_value() {
        // A digest whose low bits are sparse must still print twelve characters, not fewer:
        // a ragged column is the thing the mask is for.
        for value in [json!(0), json!(u64::MAX), json!({"a": [1, 2, 3]})] {
            assert_eq!(short_digest(&value).len(), 12);
        }
    }

    #[test]
    fn a_different_value_changes_the_digest() {
        assert_ne!(digest(&json!({"x": 1})), digest(&json!({"x": 2})));
    }
}
