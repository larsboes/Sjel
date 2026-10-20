//! F3 — the vault proposes: read-only per-field proposals extracted from TELOS
//! notes, naming the source file, admissible section, and quoted span.
//!
//! Nothing here writes to the store. A proposal is a diff between the current
//! profile and what an admissible section of `<overlay>/vault/TELOS/Personal`
//! states, and it only becomes part of the profile when the operator sends it
//! back through `PUT /api/profile` with `Provenance::Vault` (or `Stated`).
//!
//! ## Section gating
//!
//! `CURRENT_STATE` and draft/historical sections in TELOS notes record
//! point-in-time observations or outdated snapshots rather than durable
//! preferences. They are explicitly skipped and reported in `skipped_sections`
//! so a consumer can see that the file was read and why that block was refused.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model::{Anchor, Pace, Provenance, TravelProfile};

/// Admissible section identifiers that may propose profile fields.
pub const ADMISSIBLE_SECTIONS: &[&str] = &[
    "frontmatter:summary",
    "frontmatter:current_focus",
    "frontmatter:category_affinity",
    "callout:Charter",
    "heading:Interests",
    "heading:Preferences",
    "heading:Constraints",
    "heading:Travel",
];

/// Section headings that are explicitly excluded from proposing profile fields.
pub const EXCLUDED_HEADINGS: &[&str] = &["CURRENT_STATE", "Current State", "Draft", "History"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldProposal {
    /// Profile basis key (`interests`, `anchors`, `pace`,
    /// `hard.earliest_departure`, `hard.latest_arrival`, `hard.max_changes`,
    /// `hard.min_transfer_buffer_min`, `hard.avoid_overnight_travel`).
    pub field: String,
    pub current: Value,
    pub proposed: Value,
    pub source_file: String,
    pub section: String,
    pub quoted_span: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedSection {
    pub source_file: String,
    pub section: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultProposals {
    pub available: bool,
    pub scanned_files: Vec<String>,
    pub proposals: Vec<FieldProposal>,
    pub skipped_sections: Vec<SkippedSection>,
}

/// Propose profile field diffs from TELOS markdown files in `telos_dir`.
///
/// Fields already marked `Provenance::Stated` in `current` are never proposed
/// over: an explicit statement in the profile wins over a vault note.
pub fn propose_from_dir(telos_dir: Option<&Path>, current: &TravelProfile) -> VaultProposals {
    let Some(dir) = telos_dir.filter(|path| path.is_dir()) else {
        return VaultProposals {
            available: false,
            scanned_files: Vec::new(),
            proposals: Vec::new(),
            skipped_sections: Vec::new(),
        };
    };

    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("md"))
        .collect();
    entries.sort();

    let mut scanned_files = Vec::new();
    let mut skipped_sections = Vec::new();
    let mut interests: Vec<(String, String, String, String)> = Vec::new();
    let mut anchors: Vec<(Anchor, String, String, String)> = Vec::new();
    let mut scalar_candidates: Vec<FieldProposal> = Vec::new();

    for path in entries {
        let Some(file_name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        scanned_files.push(file_name.clone());
        scan_markdown_note(
            &file_name,
            &content,
            &mut interests,
            &mut anchors,
            &mut scalar_candidates,
            &mut skipped_sections,
        );
    }

    let mut proposals = Vec::new();

    if !is_stated(current, "interests") && !interests.is_empty() {
        let mut merged = current.interests.clone();
        let mut first_hit: Option<(String, String, String)> = None;
        for (item, file, section, span) in interests {
            if !merged
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&item))
            {
                if first_hit.is_none() {
                    first_hit = Some((file, section, span));
                }
                merged.push(item);
            }
        }
        if let Some((source_file, section, quoted_span)) = first_hit {
            proposals.push(FieldProposal {
                field: "interests".into(),
                current: json!(current.interests),
                proposed: json!(merged),
                source_file,
                section,
                quoted_span,
            });
        }
    }

    if !is_stated(current, "anchors") && !anchors.is_empty() {
        let mut merged = current.anchors.clone();
        let mut first_hit: Option<(String, String, String)> = None;
        for (anchor, file, section, span) in anchors {
            if !merged.contains(&anchor) {
                if first_hit.is_none() {
                    first_hit = Some((file, section, span));
                }
                merged.push(anchor);
            }
        }
        if let Some((source_file, section, quoted_span)) = first_hit {
            proposals.push(FieldProposal {
                field: "anchors".into(),
                current: json!(current.anchors),
                proposed: json!(merged),
                source_file,
                section,
                quoted_span,
            });
        }
    }

    for candidate in scalar_candidates {
        if is_stated(current, &candidate.field) {
            continue;
        }
        if proposals
            .iter()
            .any(|existing| existing.field == candidate.field)
        {
            continue;
        }
        let current_val = current_field_value(current, &candidate.field);
        if current_val != candidate.proposed {
            proposals.push(FieldProposal {
                current: current_val,
                ..candidate
            });
        }
    }

    VaultProposals {
        available: true,
        scanned_files,
        proposals,
        skipped_sections,
    }
}

fn is_stated(profile: &TravelProfile, key: &str) -> bool {
    profile.basis.get(key) == Some(&Provenance::Stated)
}

fn current_field_value(profile: &TravelProfile, key: &str) -> Value {
    match key {
        "hard.earliest_departure" => json!(profile.hard.earliest_departure),
        "hard.latest_arrival" => json!(profile.hard.latest_arrival),
        "hard.max_changes" => json!(profile.hard.max_changes),
        "hard.min_transfer_buffer_min" => json!(profile.hard.min_transfer_buffer_min),
        "hard.avoid_overnight_travel" => json!(profile.hard.avoid_overnight_travel),
        "pace" => json!(profile.pace),
        _ => Value::Null,
    }
}

fn scan_markdown_note(
    file_name: &str,
    content: &str,
    interests: &mut Vec<(String, String, String, String)>,
    anchors: &mut Vec<(Anchor, String, String, String)>,
    scalars: &mut Vec<FieldProposal>,
    skipped: &mut Vec<SkippedSection>,
) {
    let lines: Vec<&str> = content.lines().collect();
    let mut index = 0;

    // 1. Optional YAML frontmatter
    if lines.first().map(|line| line.trim()) == Some("---") {
        index = 1;
        let mut current_fm_key: Option<&str> = None;
        while index < lines.len() {
            let line = lines[index];
            if line.trim() == "---" {
                index += 1;
                break;
            }
            if let Some((raw_key, raw_val)) = line.split_once(':') {
                if !line.starts_with(' ') && !line.starts_with('\t') {
                    let key = raw_key.trim();
                    let val = strip_quotes(raw_val.trim());
                    current_fm_key = Some(key);
                    if matches!(key, "summary" | "current_focus") && !val.is_empty() {
                        let section = format!("frontmatter:{key}");
                        extract_line_signals(
                            file_name,
                            &section,
                            line.trim(),
                            &val,
                            interests,
                            anchors,
                            scalars,
                        );
                        if key == "current_focus" {
                            for item in val.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                                interests.push((
                                    strip_quotes(item),
                                    file_name.to_string(),
                                    section.clone(),
                                    line.trim().to_string(),
                                ));
                            }
                        }
                    } else if key == "category_affinity" && !val.is_empty() {
                        let section = "frontmatter:category_affinity".to_string();
                        for item in val
                            .trim_matches(['[', ']'])
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            let cleaned = strip_quotes(item);
                            if !cleaned.is_empty() {
                                interests.push((
                                    cleaned.clone(),
                                    file_name.to_string(),
                                    section.clone(),
                                    line.trim().to_string(),
                                ));
                                for anchor in anchors_in_text(&cleaned) {
                                    anchors.push((
                                        anchor,
                                        file_name.to_string(),
                                        section.clone(),
                                        line.trim().to_string(),
                                    ));
                                }
                            }
                        }
                    }
                    index += 1;
                    continue;
                }
            }
            if let Some("category_affinity") = current_fm_key {
                let trimmed = line.trim();
                if let Some(item) = trimmed.strip_prefix("- ") {
                    let cleaned = strip_quotes(item.trim());
                    if !cleaned.is_empty() {
                        let section = "frontmatter:category_affinity".to_string();
                        interests.push((
                            cleaned.clone(),
                            file_name.to_string(),
                            section.clone(),
                            trimmed.to_string(),
                        ));
                        for anchor in anchors_in_text(&cleaned) {
                            anchors.push((
                                anchor,
                                file_name.to_string(),
                                section.clone(),
                                trimmed.to_string(),
                            ));
                        }
                    }
                }
            }
            index += 1;
        }
    }

    // 2. Body headings & callout blocks
    let mut active_section: Option<String> = None;
    let mut in_excluded_heading = false;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();

        if let Some(heading) = parse_heading(trimmed) {
            if EXCLUDED_HEADINGS
                .iter()
                .any(|excluded| heading.eq_ignore_ascii_case(excluded))
            {
                in_excluded_heading = true;
                active_section = None;
                skipped.push(SkippedSection {
                    source_file: file_name.to_string(),
                    section: format!("heading:{heading}"),
                    reason: "excluded point-in-time or non-authoritative section".into(),
                });
            } else if let Some(canonical) = admissible_heading(heading) {
                in_excluded_heading = false;
                active_section = Some(format!("heading:{canonical}"));
            } else {
                in_excluded_heading = false;
                active_section = None;
            }
            index += 1;
            continue;
        }

        if trimmed.starts_with("> [!") {
            if trimmed.contains("Charter") {
                active_section = Some("callout:Charter".into());
                in_excluded_heading = false;
            }
            index += 1;
            continue;
        }

        if in_excluded_heading {
            index += 1;
            continue;
        }

        if active_section.as_deref() == Some("callout:Charter") && !trimmed.starts_with('>') {
            active_section = None;
        }

        if let Some(section) = &active_section {
            let body = trimmed.strip_prefix('>').map(str::trim).unwrap_or(trimmed);
            if !body.is_empty() {
                extract_line_signals(
                    file_name, section, trimmed, body, interests, anchors, scalars,
                );
            }
        }

        index += 1;
    }
}

fn parse_heading(trimmed: &str) -> Option<&str> {
    let stripped = trimmed.trim_start_matches('#');
    let hashes = trimmed.len() - stripped.len();
    if (1..=6).contains(&hashes) && stripped.starts_with(' ') {
        Some(stripped.trim())
    } else {
        None
    }
}

fn admissible_heading(heading: &str) -> Option<&'static str> {
    for candidate in ["Interests", "Preferences", "Constraints", "Travel"] {
        if heading.eq_ignore_ascii_case(candidate) {
            return Some(candidate);
        }
    }
    None
}

fn strip_quotes(raw: &str) -> String {
    raw.trim_matches('"').trim_matches('\'').trim().to_string()
}

fn extract_line_signals(
    file_name: &str,
    section: &str,
    quoted_span: &str,
    text: &str,
    interests: &mut Vec<(String, String, String, String)>,
    anchors: &mut Vec<(Anchor, String, String, String)>,
    scalars: &mut Vec<FieldProposal>,
) {
    for anchor in anchors_in_text(text) {
        anchors.push((
            anchor,
            file_name.to_string(),
            section.to_string(),
            quoted_span.to_string(),
        ));
    }

    if let Some(item) = text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")) {
        let item = item.trim();
        if let Some((key, value)) = item.split_once(':') {
            parse_key_value_directive(
                file_name,
                section,
                quoted_span,
                key.trim(),
                value.trim(),
                scalars,
            );
        } else if section == "heading:Interests" && !item.is_empty() {
            interests.push((
                strip_quotes(item),
                file_name.to_string(),
                section.to_string(),
                quoted_span.to_string(),
            ));
        }
    } else if let Some((key, value)) = text.split_once(':') {
        parse_key_value_directive(
            file_name,
            section,
            quoted_span,
            key.trim(),
            value.trim(),
            scalars,
        );
    }
}

fn parse_key_value_directive(
    file_name: &str,
    section: &str,
    quoted_span: &str,
    key: &str,
    value: &str,
    scalars: &mut Vec<FieldProposal>,
) {
    let normalized_key = key.to_lowercase().replace([' ', '-'], "_");
    let cleaned_val = strip_quotes(value);
    match normalized_key.as_str() {
        "earliest_departure" => {
            if is_clock(&cleaned_val) {
                scalars.push(FieldProposal {
                    field: "hard.earliest_departure".into(),
                    current: Value::Null,
                    proposed: json!(cleaned_val),
                    source_file: file_name.to_string(),
                    section: section.to_string(),
                    quoted_span: quoted_span.to_string(),
                });
            }
        }
        "latest_arrival" => {
            if is_clock(&cleaned_val) {
                scalars.push(FieldProposal {
                    field: "hard.latest_arrival".into(),
                    current: Value::Null,
                    proposed: json!(cleaned_val),
                    source_file: file_name.to_string(),
                    section: section.to_string(),
                    quoted_span: quoted_span.to_string(),
                });
            }
        }
        "max_changes" | "max_transfers" => {
            if let Ok(count) = cleaned_val.parse::<u32>() {
                scalars.push(FieldProposal {
                    field: "hard.max_changes".into(),
                    current: Value::Null,
                    proposed: json!(count),
                    source_file: file_name.to_string(),
                    section: section.to_string(),
                    quoted_span: quoted_span.to_string(),
                });
            }
        }
        "min_transfer_buffer_min" => {
            if let Ok(minutes) = cleaned_val.parse::<u32>() {
                scalars.push(FieldProposal {
                    field: "hard.min_transfer_buffer_min".into(),
                    current: Value::Null,
                    proposed: json!(minutes),
                    source_file: file_name.to_string(),
                    section: section.to_string(),
                    quoted_span: quoted_span.to_string(),
                });
            }
        }
        "avoid_overnight_travel" | "no_overnight" => {
            if let Some(flag) = match cleaned_val.to_lowercase().as_str() {
                "true" | "yes" => Some(true),
                "false" | "no" => Some(false),
                _ => None,
            } {
                scalars.push(FieldProposal {
                    field: "hard.avoid_overnight_travel".into(),
                    current: Value::Null,
                    proposed: json!(flag),
                    source_file: file_name.to_string(),
                    section: section.to_string(),
                    quoted_span: quoted_span.to_string(),
                });
            }
        }
        "pace" => {
            if let Some(pace) = match cleaned_val.to_lowercase().as_str() {
                "slow" => Some(Pace::Slow),
                "balanced" | "moderate" => Some(Pace::Balanced),
                "packed" => Some(Pace::Packed),
                _ => None,
            } {
                scalars.push(FieldProposal {
                    field: "pace".into(),
                    current: Value::Null,
                    proposed: json!(pace),
                    source_file: file_name.to_string(),
                    section: section.to_string(),
                    quoted_span: quoted_span.to_string(),
                });
            }
        }
        _ => {}
    }
}

fn is_clock(s: &str) -> bool {
    let Some((h, m)) = s.split_once(':') else {
        return false;
    };
    h.len() == 2
        && m.len() == 2
        && h.parse::<u8>().is_ok_and(|hh| hh < 24)
        && m.parse::<u8>().is_ok_and(|mm| mm < 60)
}

fn anchors_in_text(text: &str) -> Vec<Anchor> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    let pairs = [
        ("conference", Anchor::Event),
        ("hackathon", Anchor::Event),
        ("festival", Anchor::Event),
        ("show", Anchor::Event),
        ("visit", Anchor::Social),
        ("friends", Anchor::Social),
        ("family", Anchor::Social),
        ("trail", Anchor::Activity),
        ("hiking", Anchor::Activity),
        ("hike", Anchor::Activity),
        ("climbing", Anchor::Activity),
        ("bouldering", Anchor::Activity),
        ("kayak", Anchor::Activity),
        ("swim", Anchor::Activity),
        ("cowork", Anchor::Work),
        ("deep work", Anchor::Work),
        ("writing sprint", Anchor::Work),
        ("sauna", Anchor::Rest),
        ("thermal bath", Anchor::Rest),
        ("quiet", Anchor::Rest),
        ("retreat", Anchor::Rest),
    ];
    for (needle, anchor) in pairs {
        if lower.contains(needle) && !out.contains(&anchor) {
            out.push(anchor);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admissible_sections_propose_with_exact_quotes_and_current_state_is_skipped() {
        let dir = std::env::temp_dir().join(format!("axon-traveler-vault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join("INTERESTS.md"),
            "---\n\
             summary: \"Mountain trails and conferences\"\n\
             category_affinity:\n\
               - Alpine hiking\n\
               - Systems hackathon\n\
             ---\n\
             \n\
             > [!info] Charter\n\
             > earliest_departure: 08:30\n\
             > pace: slow\n\
             \n\
             ## CURRENT_STATE\n\
             - earliest_departure: 05:00\n\
             - Sauna every weekend\n\
             \n\
             ## Interests\n\
             - Specialty coffee\n",
        )
        .unwrap();

        let profile = TravelProfile::unstated();
        let report = propose_from_dir(Some(&dir), &profile);
        assert!(report.available);
        assert_eq!(report.scanned_files, vec!["INTERESTS.md".to_string()]);
        assert_eq!(report.skipped_sections.len(), 1);
        assert_eq!(report.skipped_sections[0].section, "heading:CURRENT_STATE");

        let interests = report
            .proposals
            .iter()
            .find(|p| p.field == "interests")
            .expect("interests proposal");
        assert_eq!(interests.source_file, "INTERESTS.md");
        assert_eq!(interests.section, "frontmatter:category_affinity");
        assert_eq!(interests.quoted_span, "- Alpine hiking");
        assert_eq!(
            interests.proposed,
            json!(["Alpine hiking", "Systems hackathon", "Specialty coffee"])
        );

        let dep = report
            .proposals
            .iter()
            .find(|p| p.field == "hard.earliest_departure")
            .expect("earliest_departure proposal");
        assert_eq!(
            dep.proposed,
            json!("08:30"),
            "CURRENT_STATE 05:00 must be ignored"
        );
        assert_eq!(dep.section, "callout:Charter");
        assert_eq!(dep.quoted_span, "> earliest_departure: 08:30");

        let pace = report
            .proposals
            .iter()
            .find(|p| p.field == "pace")
            .expect("pace proposal");
        assert_eq!(pace.proposed, json!("slow"));

        let anchors = report
            .proposals
            .iter()
            .find(|p| p.field == "anchors")
            .expect("anchors proposal");
        let proposed_anchors: Vec<String> =
            serde_json::from_value(anchors.proposed.clone()).unwrap();
        assert!(proposed_anchors.contains(&"activity".to_string()));
        assert!(proposed_anchors.contains(&"event".to_string()));
        assert!(
            !proposed_anchors.contains(&"rest".to_string()),
            "Sauna appeared only inside CURRENT_STATE and must not be proposed"
        );

        // Once a field is Provenance::Stated, the vault never proposes over it.
        let mut stated = TravelProfile::unstated();
        stated
            .basis
            .insert("hard.earliest_departure".into(), Provenance::Stated);
        let second = propose_from_dir(Some(&dir), &stated);
        assert!(second
            .proposals
            .iter()
            .all(|p| p.field != "hard.earliest_departure"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
