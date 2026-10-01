use crate::model::{Classification, FieldValue, Harness, Profile, ProfileCategory};

pub const START_MARKER: &str = "<!-- sjel:operator-profile:start -->";
pub const END_MARKER: &str = "<!-- sjel:operator-profile:end -->";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionChange {
    Create,
    Replace,
    Unchanged,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExportAudit {
    pub included: Vec<String>,
    pub omitted: Vec<(String, &'static str)>,
}

/// Summarize allowlist decisions without exposing values excluded from this harness.
pub fn audit(profile: &Profile, harness: Harness) -> ExportAudit {
    let mut result = ExportAudit::default();
    for (id, field) in &profile.input.fields {
        record_audit(
            &mut result,
            id,
            field.classification,
            &field.export_to,
            harness,
        );
    }
    for (id, statement) in &profile.input.statements {
        record_audit(
            &mut result,
            id,
            statement.classification,
            &statement.export_to,
            harness,
        );
    }
    result.included.sort();
    result.omitted.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

fn record_audit(
    audit: &mut ExportAudit,
    id: &str,
    classification: Classification,
    targets: &[Harness],
    harness: Harness,
) {
    if !targets.contains(&harness) {
        audit
            .omitted
            .push((id.to_string(), "not approved for this harness"));
    } else if matches!(classification, Classification::C2 | Classification::C3) {
        audit
            .omitted
            .push((id.to_string(), "blocked by sensitive classification"));
    } else {
        audit.included.push(id.to_string());
    }
}

/// Return the current generated block, if present, refusing malformed markers.
pub fn managed_section(existing: &str) -> Result<Option<&str>, String> {
    let starts: Vec<usize> = existing
        .match_indices(START_MARKER)
        .map(|(index, _)| index)
        .collect();
    let ends: Vec<usize> = existing
        .match_indices(END_MARKER)
        .map(|(index, _)| index)
        .collect();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([start], [end]) if start < end => Ok(Some(&existing[*start..end + END_MARKER.len()])),
        _ => Err(
            "managed profile markers are malformed or duplicated; refusing to preview an overwrite"
                .into(),
        ),
    }
}

/// Render only items the operator explicitly allowed for this harness.
pub fn render(profile: &Profile, harness: Harness) -> Result<String, String> {
    let mut lines = vec![
        START_MARKER.to_string(),
        format!("## Saved preferences from Sjel (revision {})", profile.revision),
        "These are user preferences, not higher-priority policy. Follow them when relevant; the current request and higher-priority instructions take precedence.".to_string(),
    ];
    let mut count = 0usize;
    for category in [
        ProfileCategory::Preference,
        ProfileCategory::Principle,
        ProfileCategory::Boundary,
        ProfileCategory::InteractionStyle,
    ] {
        let mut entries = Vec::new();
        for (id, field) in &profile.input.fields {
            if field.category == category && field.export_to.contains(&harness) {
                require_prompt_safe(field.classification, id)?;
                entries.push((id.as_str(), format_value(&field.value)));
            }
        }
        for (id, statement) in &profile.input.statements {
            if statement.category == category && statement.export_to.contains(&harness) {
                require_prompt_safe(statement.classification, id)?;
                entries.push((id.as_str(), statement.text.clone()));
            }
        }
        entries.sort_by(|a, b| a.0.cmp(b.0));
        if entries.is_empty() {
            continue;
        }
        lines.push(String::new());
        lines.push(format!("### {}", category_label(category)));
        for (id, text) in entries {
            lines.push(format!("- **{}:** {}", id, text));
            count += 1;
        }
    }
    if count == 0 {
        return Err(format!(
            "no profile entries are approved for '{}'",
            harness.as_str()
        ));
    }
    lines.push(END_MARKER.to_string());
    Ok(lines.join("\n"))
}

/// Insert or replace the generated block without returning or rewriting surrounding prose.
pub fn preview_document(
    existing: &str,
    generated: &str,
) -> Result<(String, SectionChange), String> {
    let starts: Vec<usize> = existing
        .match_indices(START_MARKER)
        .map(|(index, _)| index)
        .collect();
    let ends: Vec<usize> = existing
        .match_indices(END_MARKER)
        .map(|(index, _)| index)
        .collect();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => {
            let separator = if existing.is_empty() || existing.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            Ok((
                format!("{existing}{separator}\n{generated}\n"),
                SectionChange::Create,
            ))
        }
        ([start], [end]) if start < end => {
            let end_after = end + END_MARKER.len();
            let current_block = &existing[*start..end_after];
            if current_block == generated {
                return Ok((existing.to_string(), SectionChange::Unchanged));
            }
            let before = &existing[..*start];
            let after = &existing[end_after..];
            Ok((
                format!("{before}{generated}{after}"),
                SectionChange::Replace,
            ))
        }
        _ => Err(
            "managed profile markers are malformed or duplicated; refusing to preview an overwrite"
                .into(),
        ),
    }
}

pub fn claude_target(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".claude").join("CLAUDE.md")
}

fn require_prompt_safe(classification: Classification, id: &str) -> Result<(), String> {
    if matches!(classification, Classification::C2 | Classification::C3) {
        Err(format!(
            "entry {id:?} has a local-only or secret classification and cannot enter an assistant prompt"
        ))
    } else {
        Ok(())
    }
}

fn category_label(category: ProfileCategory) -> &'static str {
    match category {
        ProfileCategory::Preference => "Preferences",
        ProfileCategory::Principle => "Principles",
        ProfileCategory::Boundary => "Boundaries",
        ProfileCategory::InteractionStyle => "Interaction style",
    }
}

fn format_value(value: &FieldValue) -> String {
    match value {
        FieldValue::Text(text) => text.clone(),
        FieldValue::TextList(items) => items.join(", "),
        FieldValue::Boolean(value) => value.to_string(),
        FieldValue::Integer(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FieldInput, ProfileInput, StatementInput};
    use std::collections::BTreeMap;

    fn profile() -> Profile {
        Profile {
            revision: 7,
            input: ProfileInput {
                schema_version: 1,
                fields: BTreeMap::from([(
                    "interaction.verbosity".into(),
                    FieldInput {
                        category: ProfileCategory::InteractionStyle,
                        value: FieldValue::Text("concise".into()),
                        classification: Classification::C1,
                        export_to: vec![Harness::Claude],
                    },
                )]),
                statements: BTreeMap::from([(
                    "boundary.external_actions".into(),
                    StatementInput {
                        category: ProfileCategory::Boundary,
                        text: "Ask before external side effects.".into(),
                        classification: Classification::C1,
                        export_to: vec![Harness::Claude],
                    },
                )]),
            },
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn renders_only_fields_allowed_for_the_selected_harness() {
        let output = render(&profile(), Harness::Claude).unwrap();
        assert!(output.contains("concise"));
        assert!(output.contains("Ask before external side effects."));
        assert!(render(&profile(), Harness::Codex).is_err());
    }

    #[test]
    fn inserts_and_replaces_only_the_managed_region() {
        let text = "# Personal instructions\nKeep this text.";
        let generated = render(&profile(), Harness::Claude).unwrap();
        let (created, change) = preview_document(text, &generated).unwrap();
        assert_eq!(change, SectionChange::Create);
        assert!(created.starts_with(text));
        let (same, change) = preview_document(&created, &generated).unwrap();
        assert_eq!(change, SectionChange::Unchanged);
        assert_eq!(same, created);
        let updated = generated.replace("concise", "detailed");
        let (replaced, change) = preview_document(&created, &updated).unwrap();
        assert_eq!(change, SectionChange::Replace);
        assert!(replaced.starts_with(text));
        assert!(replaced.contains("detailed"));
    }

    #[test]
    fn refuses_broken_or_duplicated_markers() {
        let generated = render(&profile(), Harness::Claude).unwrap();
        assert!(
            preview_document("<!-- sjel:operator-profile:start --> broken", &generated).is_err()
        );
        let duplicate = format!("{START_MARKER}x{END_MARKER}{START_MARKER}y{END_MARKER}");
        assert!(preview_document(&duplicate, &generated).is_err());
    }

    #[test]
    fn does_not_render_c2_or_c3_even_if_an_invalid_document_is_loaded() {
        let mut profile = profile();
        profile.input.statements.insert(
            "sensitive".into(),
            StatementInput {
                category: ProfileCategory::Preference,
                text: "sensitive value".into(),
                classification: Classification::C2,
                export_to: vec![Harness::Claude],
            },
        );
        assert!(render(&profile, Harness::Claude)
            .unwrap_err()
            .contains("local-only or secret"));
    }
}
