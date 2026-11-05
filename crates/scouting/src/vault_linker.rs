//! Matches scored opportunities against existing markdown event notes in a
//! configured events directory (`Config::events_dir` -- originally an
//! Obsidian vault's `Atlas/Events/` convention, generalized to "any directory
//! of markdown event notes").
//!
//! Annotate-only by design: this module never
//! writes or mutates notes, only returns the best-matching existing note path
//! so the caller can surface it as a cross-reference.

use std::path::Path;

use crate::opportunity::Opportunity;

/// Returns the path of the best-matching existing event note, if any.
pub fn link_to_vault(opp: &Opportunity, events_dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(events_dir).ok()?;
    let mut best: Option<(String, usize)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let stem = path.file_stem()?.to_str()?.to_lowercase();
        let score = match_score(opp, &stem);
        if score == 0 {
            continue;
        }
        match &best {
            None => best = Some((path.to_string_lossy().into_owned(), score)),
            Some((_, bs)) if score > *bs => {
                best = Some((path.to_string_lossy().into_owned(), score))
            }
            _ => {}
        }
    }

    best.map(|(p, _)| p)
}

fn match_score(opp: &Opportunity, note_stem_lower: &str) -> usize {
    let mut score = 0;
    let title_lower = opp.title.to_lowercase();

    if note_stem_lower.contains(&title_lower) || title_lower.contains(note_stem_lower) {
        score += 10;
    }
    let title_tokens: Vec<&str> = title_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 3)
        .collect();
    let token_matches = title_tokens
        .iter()
        .filter(|t| note_stem_lower.contains(*t))
        .count();
    score += token_matches * 2;

    if let Some(ref city) = opp.city {
        let city_lower = city.to_lowercase();
        if note_stem_lower.contains(&city_lower) {
            score += 3;
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opportunity::{Opportunity, OpportunityType, SourceKind};
    use std::fs;

    fn mk_opp(title: &str, city: Option<&str>) -> Opportunity {
        Opportunity {
            id: "x".into(),
            opportunity_type: OpportunityType::Event,
            source: "s".into(),
            source_kind: SourceKind::Api,
            url: "u".into(),
            title: title.into(),
            starts_at: None,
            ends_at: None,
            location: city.map(String::from),
            city: city.map(String::from),
            country_code: None,
            latitude: None,
            longitude: None,
            raw: serde_json::Value::Null,
            fetched_at: "t".into(),
        }
    }

    #[test]
    fn matches_by_title_subset() {
        let dir = std::env::temp_dir().join("sjel_scouting_linker_test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Community Meetup Night.md"), "").unwrap();
        fs::write(dir.join("Agent Olympics.md"), "").unwrap();

        let opp = mk_opp("Agent Olympics", None);
        let link = link_to_vault(&opp, &dir);
        assert!(link.is_some(), "should match Agent Olympics");
        assert!(link.unwrap().contains("Agent Olympics.md"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn matches_by_city_token_overlap() {
        let dir = std::env::temp_dir().join("sjel_scouting_linker_test2");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("2026-02 Munich Security & Hiking.md"), "").unwrap();

        let opp = mk_opp("Munich Hackathon", Some("Munich"));
        let link = link_to_vault(&opp, &dir);
        assert!(link.is_some(), "should match Munich note via token overlap");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_match_returns_none() {
        let dir = std::env::temp_dir().join("sjel_scouting_linker_test3");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Completely Unrelated Event.md"), "").unwrap();

        let opp = mk_opp("Berlin Tech Conference", Some("Berlin"));
        let link = link_to_vault(&opp, &dir);
        assert!(link.is_none());

        fs::remove_dir_all(&dir).ok();
    }
}
