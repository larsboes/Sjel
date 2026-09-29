//! Read-only travel context for Feed evaluation.
//!
//! Trips remains the owner of plans. Comms consumes its existing HTTP contract,
//! stores the last successful bounded snapshot, and keeps using that snapshot
//! while Trips is temporarily unavailable. Only public plan fields required for
//! ranking are retained; itinerary payloads and traveler names are not copied.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::time::Duration;

use crate::config::TravelContextConfig;
use crate::store::{FeedItem, Store, TravelContextSnapshot};

#[derive(Debug, Clone, Deserialize)]
struct TripPlanResponse {
    id: String,
    title: String,
    destinations: Vec<PlaceResponse>,
    date_start: String,
    date_end: String,
    interests: String,
    status: String,
    updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PlaceResponse {
    name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TravelContext {
    pub id: String,
    pub title: String,
    pub destinations: Vec<String>,
    pub date_start: String,
    pub date_end: String,
    pub interests: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TravelFactorContext {
    pub id: String,
    pub label: String,
    pub date_start: String,
    pub date_end: String,
    pub matched_terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TravelSignal {
    pub score: f64,
    pub rationale: String,
    pub context: Option<TravelFactorContext>,
}

#[derive(Debug, Clone)]
pub struct LoadedTravelContext {
    pub contexts: Vec<TravelContext>,
    pub revision: String,
    pub refreshed_at: String,
    pub reachable: bool,
    pub from_cache: bool,
}

pub fn load(store: &Store, config: &TravelContextConfig) -> LoadedTravelContext {
    if !config.enabled {
        return empty(false);
    }
    match fetch(config) {
        Ok(contexts) => {
            let revision = revision(&contexts);
            let payload = serde_json::to_string(&contexts).unwrap_or_else(|_| "[]".into());
            if let Err(error) = store.replace_travel_context_snapshot(&revision, &payload) {
                eprintln!("travel context: could not cache snapshot: {error}");
            }
            let cached = store.travel_context_snapshot().ok().flatten();
            LoadedTravelContext {
                contexts,
                revision,
                refreshed_at: cached
                    .map(|snapshot| snapshot.refreshed_at)
                    .unwrap_or_default(),
                reachable: true,
                from_cache: false,
            }
        }
        Err(error) => {
            eprintln!("travel context: Trips unavailable, using cache: {error}");
            cached(store).unwrap_or_else(|| empty(true))
        }
    }
}

pub fn cached(store: &Store) -> Option<LoadedTravelContext> {
    let snapshot = store.travel_context_snapshot().ok().flatten()?;
    decode_snapshot(snapshot)
}

fn decode_snapshot(snapshot: TravelContextSnapshot) -> Option<LoadedTravelContext> {
    let contexts = serde_json::from_str::<Vec<TravelContext>>(&snapshot.payload).ok()?;
    Some(LoadedTravelContext {
        contexts,
        revision: snapshot.revision,
        refreshed_at: snapshot.refreshed_at,
        reachable: false,
        from_cache: true,
    })
}

fn empty(from_cache: bool) -> LoadedTravelContext {
    LoadedTravelContext {
        contexts: Vec::new(),
        revision: revision(&[]),
        refreshed_at: String::new(),
        reachable: false,
        from_cache,
    }
}

fn fetch(config: &TravelContextConfig) -> Result<Vec<TravelContext>, Box<dyn std::error::Error>> {
    let url = format!("{}/api/plans", config.base_url.trim_end_matches('/'));
    let plans = sjel_http::client(
        sjel_http::Purpose::new("comms-travel"),
        Duration::from_millis(config.timeout_ms.clamp(250, 10_000)),
    )?
    .get(url)
    .send()?
    .error_for_status()?
    .json::<Vec<TripPlanResponse>>()?;
    let today = current_date();
    let mut contexts = plans
        .into_iter()
        .filter(|plan| plan.status != "archived" && plan.date_end.as_str() >= today.as_str())
        .map(|plan| TravelContext {
            id: plan.id,
            title: plan.title,
            destinations: plan
                .destinations
                .into_iter()
                .map(|place| place.name)
                .filter(|name| !name.trim().is_empty())
                .collect(),
            date_start: plan.date_start,
            date_end: plan.date_end,
            interests: plan.interests,
            updated_at: plan.updated_at,
        })
        .collect::<Vec<_>>();
    contexts.sort_by(|left, right| left.date_start.cmp(&right.date_start));
    contexts.truncate(config.max_plans.clamp(1, 50));
    Ok(contexts)
}

pub fn revision(contexts: &[TravelContext]) -> String {
    let mut rows = contexts
        .iter()
        .map(|context| {
            format!(
                "{}|{}|{}|{}|{}|{}",
                context.id,
                context.updated_at,
                context.date_start,
                context.date_end,
                context.destinations.join("\u{1f}"),
                context.interests
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    let mut hasher = Sha256::new();
    for row in rows {
        hasher.update((row.len() as u64).to_be_bytes());
        hasher.update(row.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

pub fn score_item(item: &FeedItem, contexts: &[TravelContext]) -> TravelSignal {
    let item_text = [
        item.title.as_deref().unwrap_or_default(),
        item.summary.as_deref().unwrap_or_default(),
        item.transcript.as_deref().unwrap_or_default(),
        &item.url,
    ]
    .join(" ")
    .to_lowercase();
    let item_tokens = tokens(&item_text);
    let mut best: Option<(f64, &TravelContext, Vec<String>)> = None;

    for context in contexts {
        let mut matched_terms = Vec::new();
        let destination_match = context
            .destinations
            .iter()
            .filter_map(|destination| {
                let normalized = destination.trim().to_lowercase();
                if normalized.len() >= 2 && item_text.contains(&normalized) {
                    Some(destination.clone())
                } else {
                    let destination_tokens = tokens(&normalized);
                    let overlap = destination_tokens
                        .iter()
                        .filter(|token| item_tokens.contains(*token))
                        .count();
                    (overlap > 0 && overlap == destination_tokens.len())
                        .then_some(destination.clone())
                }
            })
            .next();
        let has_destination_match = destination_match.is_some();
        if let Some(destination) = destination_match {
            matched_terms.push(destination);
        }

        let interest_tokens = tokens(&context.interests);
        let mut matching_interests = interest_tokens
            .iter()
            .filter(|token| item_tokens.contains(*token))
            .cloned()
            .collect::<Vec<_>>();
        matching_interests.sort();
        matching_interests.truncate(4);
        matched_terms.extend(matching_interests.iter().cloned());

        let location_score = if has_destination_match { 0.70 } else { 0.0 };
        let interest_score = if interest_tokens.is_empty() {
            0.0
        } else {
            (matching_interests.len() as f64 / interest_tokens.len() as f64).min(1.0) * 0.30
        };
        let score = (location_score + interest_score).clamp(0.0, 1.0);
        if score <= 0.0 {
            continue;
        }
        if best.as_ref().is_none_or(|(current, _, _)| score > *current) {
            best = Some((score, context, matched_terms));
        }
    }

    match best {
        Some((score, context, matched_terms)) => TravelSignal {
            score,
            rationale: format!(
                "Related to \"{}\" ({} to {}): {}",
                context.title,
                context.date_start,
                context.date_end,
                matched_terms.join(", ")
            ),
            context: Some(TravelFactorContext {
                id: context.id.clone(),
                label: context.title.clone(),
                date_start: context.date_start.clone(),
                date_end: context.date_end.clone(),
                matched_terms,
            }),
        },
        None => TravelSignal {
            score: 0.0,
            rationale: if contexts.is_empty() {
                "No upcoming trip in the cached Trips context".into()
            } else {
                format!(
                    "No clear location or interest match across {} upcoming trips",
                    contexts.len()
                )
            },
            context: None,
        },
    }
}

fn tokens(value: &str) -> HashSet<String> {
    const STOP_WORDS: &[&str] = &[
        "and", "der", "die", "das", "den", "des", "ein", "eine", "for", "from", "mit", "oder",
        "the", "und", "von", "zur",
    ];
    value
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2 && !STOP_WORDS.contains(token))
        .map(str::to_string)
        .collect()
}

fn current_date() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        / 86_400;
    civil_from_days(days)
}

fn civil_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str) -> FeedItem {
        let mut item = FeedItem::new("https://example.com", "news", "article");
        item.title = Some(text.into());
        item
    }

    fn trip() -> TravelContext {
        TravelContext {
            id: "trip:berlin".into(),
            title: "Berlin Rust Week".into(),
            destinations: vec!["Berlin".into()],
            date_start: "2026-09-10".into(),
            date_end: "2026-09-14".into(),
            interests: "Rust systems architecture".into(),
            updated_at: "2026-07-29T10:00:00Z".into(),
        }
    }

    #[test]
    fn destination_and_interest_produce_explainable_signal() {
        let signal = score_item(&item("Rust conference in Berlin"), &[trip()]);
        assert!(signal.score > 0.7);
        assert!(signal
            .rationale
            .starts_with("Related to \"Berlin Rust Week\""));
        let context = signal.context.unwrap();
        assert_eq!(context.id, "trip:berlin");
        assert!(context.matched_terms.contains(&"Berlin".to_string()));
        assert!(context.matched_terms.contains(&"rust".to_string()));
    }

    #[test]
    fn unrelated_item_has_no_trip_context() {
        let signal = score_item(&item("Ocean temperatures in Peru"), &[trip()]);
        assert_eq!(signal.score, 0.0);
        assert!(signal.context.is_none());
    }

    #[test]
    fn revision_is_order_independent() {
        let first = trip();
        let mut second = trip();
        second.id = "trip:two".into();
        assert_eq!(
            revision(&[first.clone(), second.clone()]),
            revision(&[second, first])
        );
    }
}
