// tools/claude-code-config/src/merge.rs — the two merge rules this tool has.
//
// `merge_defaults` is the one that runs on every apply: the baseline fills gaps and never
// changes a value that is already there, so a personal override survives every re-run. That
// is what makes re-applying safe, and it is why the verb is not called "enforce".
//
// `merge_force` is the same walk with the opposite leaf rule, and it exists because a floor
// you cannot restore is not a floor. An emptied deny list survives `merge_defaults` by design
// — the file belongs to the user — so restoring it has to be asked for by name.

use serde_json::{Map, Value};

fn is_plain_object(value: &Value) -> bool {
    value.is_object()
}

/// Fill gaps in `existing` from `baseline`, recording the dotted path of every key it
/// contributes. Arrays and scalars are leaves: an existing key is left exactly as it was.
pub fn merge_defaults(
    existing: &mut Value,
    baseline: &Value,
    prefix: &str,
    added: &mut Vec<String>,
) {
    let (Some(existing_map), Some(baseline_map)) = (existing.as_object_mut(), baseline.as_object())
    else {
        return;
    };
    for (key, baseline_value) in baseline_map {
        let path = join_path(prefix, key);
        match existing_map.get_mut(key) {
            None => {
                existing_map.insert(key.clone(), baseline_value.clone());
                added.push(path);
            }
            Some(existing_value) => {
                if is_plain_object(existing_value) && is_plain_object(baseline_value) {
                    merge_defaults(existing_value, baseline_value, &path, added);
                }
            }
        }
    }
}

/// Overwrite every leaf the baseline declares, recording each path whose value actually
/// changed. A key the baseline does not mention is still left alone: this restores what the
/// baseline states, it does not prune what it does not.
pub fn merge_force(
    existing: &mut Value,
    baseline: &Value,
    prefix: &str,
    changed: &mut Vec<String>,
) {
    let (Some(existing_map), Some(baseline_map)) = (existing.as_object_mut(), baseline.as_object())
    else {
        return;
    };
    for (key, baseline_value) in baseline_map {
        let path = join_path(prefix, key);
        match existing_map.get_mut(key) {
            None => {
                existing_map.insert(key.clone(), baseline_value.clone());
                changed.push(path);
            }
            Some(existing_value) => {
                if is_plain_object(existing_value) && is_plain_object(baseline_value) {
                    merge_force(existing_value, baseline_value, &path, changed);
                } else if existing_value != baseline_value {
                    *existing_value = baseline_value.clone();
                    changed.push(path);
                }
            }
        }
    }
}

fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// An object, for the empty case: a missing target starts as `{}`, not as `null`.
pub fn empty_object() -> Value {
    Value::Object(Map::new())
}

/// Drop `_`-prefixed keys at every level.
///
/// The convention comes from the fragment that fed the retired managed layer, and it earns its
/// keep: the deployment's own fragment opens with `_zone` and `_why`, which state why protected
/// paths are denied. That prose belongs beside the rules it explains, not inside the settings
/// file Claude Code parses.
pub fn strip_documentation(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let kept = map
                .iter()
                .filter(|(key, _)| !key.starts_with('_'))
                .map(|(key, inner)| (key.clone(), strip_documentation(inner)))
                .collect();
            Value::Object(kept)
        }
        Value::Array(items) => Value::Array(items.iter().map(strip_documentation).collect()),
        other => other.clone(),
    }
}

/// Lay `fragment` over `base`: the deployment's own rules win.
///
/// Arrays are concatenated rather than replaced, base entries first. That is the one rule the
/// retired managed layer cared most about, and it survives the move because the reason did: the
/// fragment ADDS this machine's protected paths to a shared floor, and replacing the array
/// would mean a fragment that names `permissions.deny` at all silently drops the 60-odd rules
/// it did not mention.
///
/// What did not survive is the guard that refused a fragment extending an allowlist. It was a
/// property of the managed layer — the overlay could not loosen a policy the operator deployed
/// — and there is no such layer now. The merged result lands in a file its owner can edit
/// directly, so a guard on the fragment would be theatre. The honest statement of the new
/// position is in README.md.
pub fn merge_over(base: &Value, fragment: &Value) -> Value {
    match (base, fragment) {
        (Value::Object(base_map), Value::Object(fragment_map)) => {
            let mut out = base_map.clone();
            for (key, fragment_value) in fragment_map {
                let combined = match (base_map.get(key), fragment_value) {
                    (Some(Value::Array(base_items)), Value::Array(fragment_items)) => {
                        let mut joined = base_items.clone();
                        for item in fragment_items {
                            if !joined.contains(item) {
                                joined.push(item.clone());
                            }
                        }
                        Value::Array(joined)
                    }
                    (Some(Value::Object(_)), Value::Object(_)) => {
                        merge_over(base_map.get(key).unwrap(), fragment_value)
                    }
                    (_, value) => value.clone(),
                };
                out.insert(key.clone(), combined);
            }
            Value::Object(out)
        }
        (_, other) => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_fill_gaps_and_never_overwrite() {
        let mut existing = json!({"model": "mine", "permissions": {"defaultMode": "plan"}});
        let baseline = json!({
            "model": "baseline",
            "permissions": {"defaultMode": "auto", "deny": ["Read(~/.ssh/**)"]}
        });
        let mut added = Vec::new();
        merge_defaults(&mut existing, &baseline, "", &mut added);
        assert_eq!(existing["model"], "mine");
        assert_eq!(existing["permissions"]["defaultMode"], "plan");
        assert_eq!(existing["permissions"]["deny"][0], "Read(~/.ssh/**)");
        assert_eq!(added, vec!["permissions.deny"]);
    }

    #[test]
    fn defaults_are_idempotent() {
        let baseline = json!({"a": {"b": 1}, "c": [1, 2]});
        let mut once = empty_object();
        let mut added = Vec::new();
        merge_defaults(&mut once, &baseline, "", &mut added);
        let snapshot = once.clone();
        let mut added_again = Vec::new();
        merge_defaults(&mut once, &baseline, "", &mut added_again);
        assert_eq!(once, snapshot);
        assert!(added_again.is_empty(), "a second apply changes nothing");
    }

    #[test]
    fn an_array_is_a_leaf_so_an_emptied_deny_list_survives() {
        let mut existing = json!({"permissions": {"deny": []}});
        let baseline = json!({"permissions": {"deny": ["Read(~/.ssh/**)"]}});
        let mut added = Vec::new();
        merge_defaults(&mut existing, &baseline, "", &mut added);
        assert_eq!(existing["permissions"]["deny"], json!([]));
        assert!(added.is_empty());
    }

    #[test]
    fn force_restores_exactly_what_the_baseline_declares() {
        let mut existing = json!({"model": "mine", "permissions": {"deny": []}});
        let baseline = json!({"permissions": {"deny": ["Read(~/.ssh/**)"]}});
        let mut changed = Vec::new();
        merge_force(&mut existing, &baseline, "", &mut changed);
        assert_eq!(
            existing["model"], "mine",
            "a key the baseline omits is untouched"
        );
        assert_eq!(existing["permissions"]["deny"], json!(["Read(~/.ssh/**)"]));
        assert_eq!(changed, vec!["permissions.deny"]);
    }

    #[test]
    fn force_records_nothing_when_values_already_match() {
        let baseline = json!({"permissions": {"deny": ["a"]}});
        let mut existing = baseline.clone();
        let mut changed = Vec::new();
        merge_force(&mut existing, &baseline, "", &mut changed);
        assert!(changed.is_empty());
    }

    #[test]
    fn a_wholly_absent_subtree_is_reported_once_at_its_top() {
        // The rule, made explicit rather than accidental: merge reports the highest level it
        // had to insert, because "+ sandbox" is both shorter and more accurate than forty
        // lines of rules that arrived together. `check` reports leaves instead, because there
        // the question is which rule changed.
        let mut existing = empty_object();
        let mut added = Vec::new();
        merge_defaults(
            &mut existing,
            &json!({"sandbox": {"credentials": {"envVars": [{"name": "GITHUB_TOKEN"}]}}}),
            "",
            &mut added,
        );
        assert_eq!(added, vec!["sandbox"]);
    }

    #[test]
    fn an_existing_subtree_reports_the_leaf_it_added() {
        let mut existing = json!({"sandbox": {"credentials": {}}});
        let mut added = Vec::new();
        merge_defaults(
            &mut existing,
            &json!({"sandbox": {"credentials": {"envVars": [{"name": "GITHUB_TOKEN"}]}}}),
            "",
            &mut added,
        );
        assert_eq!(added, vec!["sandbox.credentials.envVars"]);
    }

    #[test]
    fn documentation_keys_are_dropped_at_every_level() {
        let value = json!({
            "_why": "prose",
            "permissions": {"_zone": "protected", "deny": ["a"]},
            "list": [{"_note": "inner", "keep": 1}]
        });
        let stripped = strip_documentation(&value);
        assert!(stripped.get("_why").is_none());
        assert!(stripped["permissions"].get("_zone").is_none());
        assert_eq!(stripped["permissions"]["deny"][0], json!("a"));
        assert_eq!(stripped["list"][0]["keep"], json!(1));
        assert!(stripped["list"][0].get("_note").is_none());
    }

    #[test]
    fn a_fragment_adds_to_an_array_rather_than_replacing_it() {
        // The rule that matters: a fragment naming permissions.deny must not drop the shared
        // floor it did not mention.
        let base = json!({"permissions": {"deny": ["shared"]}});
        let fragment = json!({"permissions": {"deny": ["mine"]}});
        let merged = merge_over(&base, &fragment);
        assert_eq!(merged["permissions"]["deny"], json!(["shared", "mine"]));
    }

    #[test]
    fn a_fragment_wins_a_scalar_and_rejoins_its_own() {
        let base = json!({"permissions": {"defaultMode": "auto", "deny": ["shared"]}});
        let fragment = json!({"permissions": {"defaultMode": "plan"}});
        let merged = merge_over(&base, &fragment);
        assert_eq!(merged["permissions"]["defaultMode"], json!("plan"));
        assert_eq!(
            merged["permissions"]["deny"],
            json!(["shared"]),
            "untouched keys survive"
        );
    }

    #[test]
    fn a_repeated_array_entry_is_not_duplicated() {
        let base = json!({"permissions": {"deny": ["same"]}});
        let fragment = json!({"permissions": {"deny": ["same", "other"]}});
        assert_eq!(
            merge_over(&base, &fragment)["permissions"]["deny"],
            json!(["same", "other"])
        );
    }

    #[test]
    fn an_absent_fragment_leaves_the_base_alone() {
        let base = json!({"permissions": {"deny": ["shared"]}});
        assert_eq!(merge_over(&base, &empty_object()), base);
    }
}
