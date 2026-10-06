//! Which person each traveller name means (libs/links/ISA.md D7-D11, LNK-15).
//!
//! `travelers` stays text, as the operator wrote it; about a hundred readers take it that way.
//! Beside it, `{prefix}_people` holds one decision per name, keyed by `sjel_links::name_key`:
//! an `ent:` id, or null for "not a person". A name with no row is undecided.
//!
//! Entities decides what a name could mean (`POST /entities/api/resolve`). Trips only applies the
//! answer: an `exact` match is stored at once, everything else waits for the operator on
//! `/people`. If entities does not answer, the names stay undecided and nothing fails.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use rusqlite::params;
use sjel_links::{Link, Links, Match, ResolveAnswer, ResolveRequest};
use sjel_store::QueryAll;

use crate::store::{TripPlan, TripsStore};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

pub const DDL: &str = "
    -- One decision per traveller name (libs/links/ISA.md D7, D11). entity_id NULL records
    -- 'not a person'; a name with no row is undecided.
    CREATE TABLE IF NOT EXISTS {prefix}_people (
        name_key    TEXT PRIMARY KEY,
        entity_id   TEXT,
        decided_at  TEXT NOT NULL
    );
";

/// The id kinds trips answers `/api/links` for. Declared again as `links_to` in `service.toml`;
/// a test holds the two equal.
pub const LINKS_TO: &[&str] = &["ent"];

// ---- store ----------------------------------------------------------------------------------

/// Every decision, by name key.
pub fn decisions(store: &TripsStore) -> Fallible<HashMap<String, Option<String>>> {
    let prefix = store.prefix();
    let conn = store.borrow_connection()?;
    Ok(conn
        .query_all(
            &format!("SELECT name_key, entity_id FROM {prefix}_people"),
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )?
        .into_iter()
        .collect())
}

/// Records one decision, replacing an earlier one for the same name.
pub fn decide(store: &TripsStore, name: &str, entity_id: Option<&str>) -> Fallible<()> {
    let prefix = store.prefix();
    let conn = store.borrow_connection()?;
    conn.execute(
        &format!(
            "INSERT INTO {prefix}_people (name_key, entity_id, decided_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (name_key) DO UPDATE SET entity_id = excluded.entity_id,
                                                 decided_at = excluded.decided_at"
        ),
        params![sjel_links::name_key(name), entity_id, crate::store::stamp()],
    )?;
    Ok(())
}

// ---- pure -----------------------------------------------------------------------------------

/// Undecided names with the number of plans each one appears in, first spelling kept, in name
/// order.
pub fn undecided(
    plans: &[TripPlan],
    decided: &HashMap<String, Option<String>>,
) -> Vec<(String, usize)> {
    let mut by_key: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for plan in plans {
        let mut seen = std::collections::HashSet::new();
        for name in &plan.travelers {
            let key = sjel_links::name_key(name);
            if key.is_empty() || decided.contains_key(&key) || !seen.insert(key.clone()) {
                continue;
            }
            by_key
                .entry(key)
                .or_insert_with(|| (name.trim().to_string(), 0))
                .1 += 1;
        }
    }
    by_key.into_values().collect()
}

/// The plans whose travellers include a name decided as `entity_id`.
pub fn plans_with(
    plans: &[TripPlan],
    decided: &HashMap<String, Option<String>>,
    entity_id: &str,
) -> Links {
    let mut links: Vec<Link> = plans
        .iter()
        .filter(|plan| {
            plan.travelers.iter().any(|name| {
                decided
                    .get(&sjel_links::name_key(name))
                    .and_then(Option::as_deref)
                    == Some(entity_id)
            })
        })
        .map(|plan| Link {
            id: plan.id.clone(),
            kind: "trip:plan".into(),
            title: plan.title.clone(),
            at: Some(plan.date_start.clone()),
            meta: Some(format!("{} – {}", plan.date_start, plan.date_end)),
            via: "travelers",
        })
        .collect();
    links.sort_by(|a, b| b.at.cmp(&a.at));
    Links {
        to: entity_id.to_owned(),
        links,
        unlinkable: 0,
    }
}

// ---- entities -------------------------------------------------------------------------------

/// Where entities-server listens. The same sibling-port convention as `finance_base_url`.
pub fn entities_base_url() -> String {
    sjel_config::env_var("SJEL_ENTITIES_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8097".to_string())
}

const TIMEOUT: Duration = Duration::from_secs(3);

/// Asks entities what each name means. The error is a sentence a reader can act on.
pub fn resolve(names: &[String]) -> Result<ResolveAnswer, String> {
    if names.is_empty() {
        return Ok(ResolveAnswer::default());
    }
    let url = format!("{}/api/resolve", entities_base_url());
    let client = sjel_http::client(sjel_http::Purpose::new("trips-entities"), TIMEOUT)
        .map_err(|error| format!("entities client: {error}"))?;
    let body = ResolveRequest {
        names: names.to_vec(),
        emails: vec![],
    };
    let response =
        sjel_server::InboundAuth::with_loopback_auth(client.post(&url).json(&body), &url)
            .send()
            .map_err(|error| format!("entities is not reachable: {}", error.without_url()))?;
    if !response.status().is_success() {
        return Err(format!(
            "entities answered {} to a resolve",
            response.status()
        ));
    }
    response
        .json::<ResolveAnswer>()
        .map_err(|_| "entities answered an unexpected shape to a resolve".to_string())
}

/// Stores every exact match among the undecided names (D8). Returns how many it linked.
/// Anything entities is unsure of stays undecided for the operator.
pub fn link_exact(store: &TripsStore) -> Fallible<usize> {
    let names: Vec<String> = undecided(&store.list_plans()?, &decisions(store)?)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let answer = resolve(&names)?;
    let mut linked = 0;
    for found in answer.names {
        if let (Match::Exact, Some(id)) = (found.status, found.entity_id.as_deref()) {
            decide(store, &found.name, Some(id))?;
            linked += 1;
        }
    }
    Ok(linked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_links::{Candidate, NameAnswer};

    fn plan(id: &str, travelers: &[&str]) -> TripPlan {
        let mut plan: TripPlan = serde_json::from_value(serde_json::json!({
            "id": id, "title": id, "origin": {"id": "o", "name": "o"}, "destinations": [],
            "date_start": "2026-10-07", "date_end": "2026-10-13", "interests": "",
            "status": "saved", "travelers": [], "transport_modes": [], "stages": [],
            "created_at": "", "updated_at": ""
        }))
        .expect("a plan");
        plan.travelers = travelers.iter().map(|t| t.to_string()).collect();
        plan
    }

    #[test]
    fn a_name_counts_once_per_plan_and_decided_names_are_not_open() {
        let plans = [
            plan("a", &["Lucia", " lucia ", "Maria"]),
            plan("b", &["Lucia", "Jonas"]),
        ];
        let decided = HashMap::from([("jonas".to_string(), None)]);
        assert_eq!(
            undecided(&plans, &decided),
            [("Lucia".to_string(), 2), ("Maria".to_string(), 1)]
        );
    }

    #[test]
    fn the_open_list_survives_entities_being_down() {
        let open = sjel_links::open_names(
            "trips",
            vec![("Lucia".into(), 2)],
            Err("entities is not reachable".into()),
        );
        assert_eq!(open.names.len(), 1);
        assert!(open.names[0].status.is_none());
        assert!(open.error.is_some());

        let answer = ResolveAnswer {
            names: vec![NameAnswer {
                name: "lucia".into(),
                status: Match::First,
                entity_id: None,
                candidates: vec![Candidate {
                    id: "ent:1".into(),
                    name: "Lucia García".into(),
                }],
            }],
            emails: vec![],
        };
        let open = sjel_links::open_names("trips", vec![("Lucia".into(), 2)], Ok(answer));
        assert_eq!(open.names[0].status, Some(Match::First));
        assert_eq!(open.names[0].candidates.len(), 1);
    }

    #[test]
    fn a_person_finds_the_plans_their_name_is_on() {
        let plans = [
            plan("a", &["Lucia"]),
            plan("b", &["Maria"]),
            plan("c", &["LUCIA"]),
        ];
        let decided = HashMap::from([
            ("lucia".to_string(), Some("ent:1".to_string())),
            ("maria".to_string(), None),
        ]);
        let ids: Vec<_> = plans_with(&plans, &decided, "ent:1")
            .links
            .into_iter()
            .map(|l| l.id)
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(plans_with(&plans, &decided, "ent:2").links.is_empty());
    }

    #[test]
    fn links_to_matches_the_manifest() {
        let manifest = include_str!("../service.toml");
        let line = manifest
            .lines()
            .find(|line| line.starts_with("links_to"))
            .expect("service.toml declares links_to");
        for kind in LINKS_TO {
            assert!(
                line.contains(&format!("\"{kind}\"")),
                "{kind} missing from {line}"
            );
        }
    }
}
