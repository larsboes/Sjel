//! Which person each `person_places.person` name means (libs/links/ISA.md D7-D11, LNK-16).
//!
//! The same contract as trips' `people.rs`: `person` stays text, and `{prefix}_people` holds one
//! decision per name, keyed by `sjel_links::name_key`: an `ent:` id, or null for "not a person".
//! A name with no row is undecided. Entities says what a name could mean; places stores exact
//! matches itself and leaves the rest to the operator.
//!
//! A dismissed person-place row is not a link and its name is not open: the operator already
//! said that place does not belong to that person (ISA PLC-7). Proposed and confirmed rows are
//! listed, and the link says which.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use rusqlite::params;
use sjel_links::{Link, Links, Match, ResolveAnswer, ResolveRequest};
use sjel_store::QueryAll;

use crate::store::PlacesStore;

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

pub const DDL: &str = "
    -- One decision per person name (libs/links/ISA.md D7, D11). entity_id NULL records
    -- 'not a person'; a name with no row is undecided.
    CREATE TABLE IF NOT EXISTS {prefix}_people (
        name_key    TEXT PRIMARY KEY,
        entity_id   TEXT,
        decided_at  TEXT NOT NULL
    );
";

/// One person-place row with the place it names, as the people contract reads it.
#[derive(Debug, Clone)]
pub struct Row {
    pub person: String,
    pub state: String,
    pub place_id: String,
    pub place_name: String,
    pub city: Option<String>,
}

// ---- store ----------------------------------------------------------------------------------

pub fn rows(store: &PlacesStore) -> Fallible<Vec<Row>> {
    let prefix = store.prefix();
    let conn = store.conn()?;
    Ok(conn.query_all(
        &format!(
            "SELECT pp.person, pp.state, p.id, p.name, p.city
             FROM {prefix}_person_places pp
             JOIN {prefix}_places p ON p.id = pp.place_id
             WHERE pp.state <> 'dismissed'
             ORDER BY pp.person, p.name"
        ),
        [],
        |row| {
            Ok(Row {
                person: row.get(0)?,
                state: row.get(1)?,
                place_id: row.get(2)?,
                place_name: row.get(3)?,
                city: row.get(4)?,
            })
        },
    )?)
}

pub fn decisions(store: &PlacesStore) -> Fallible<HashMap<String, Option<String>>> {
    let prefix = store.prefix();
    let conn = store.conn()?;
    Ok(conn
        .query_all(
            &format!("SELECT name_key, entity_id FROM {prefix}_people"),
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )?
        .into_iter()
        .collect())
}

pub fn decide(store: &PlacesStore, name: &str, entity_id: Option<&str>) -> Fallible<()> {
    let prefix = store.prefix();
    let conn = store.conn()?;
    conn.execute(
        &format!(
            "INSERT INTO {prefix}_people (name_key, entity_id, decided_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (name_key) DO UPDATE SET entity_id = excluded.entity_id,
                                                 decided_at = excluded.decided_at"
        ),
        params![sjel_links::name_key(name), entity_id, crate::today()],
    )?;
    Ok(())
}

// ---- pure -----------------------------------------------------------------------------------

/// Undecided names with how many rows each covers, first spelling kept, in name order.
pub fn undecided(rows: &[Row], decided: &HashMap<String, Option<String>>) -> Vec<(String, usize)> {
    let mut by_key: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for row in rows {
        let key = sjel_links::name_key(&row.person);
        if key.is_empty() || decided.contains_key(&key) {
            continue;
        }
        by_key
            .entry(key)
            .or_insert_with(|| (row.person.trim().to_string(), 0))
            .1 += 1;
    }
    by_key.into_values().collect()
}

/// The places a person is linked to, one link per place.
pub fn places_of(
    rows: &[Row],
    decided: &HashMap<String, Option<String>>,
    entity_id: &str,
) -> Links {
    let mut seen = HashSet::new();
    let links = rows
        .iter()
        .filter(|row| {
            decided
                .get(&sjel_links::name_key(&row.person))
                .and_then(Option::as_deref)
                == Some(entity_id)
        })
        .filter(|row| seen.insert(row.place_id.clone()))
        .map(|row| Link {
            id: row.place_id.clone(),
            kind: "place".into(),
            title: row.place_name.clone(),
            at: None,
            meta: Some(match &row.city {
                Some(city) if *city != row.place_name => format!("{} · {city}", row.state),
                _ => row.state.clone(),
            }),
            via: "person_places.person",
        })
        .collect();
    Links {
        to: entity_id.to_owned(),
        links,
        unlinkable: 0,
    }
}

// ---- entities -------------------------------------------------------------------------------

/// Where entities-server listens. The same sibling-port convention trips uses.
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
    let client = sjel_http::client(sjel_http::Purpose::new("places-entities"), TIMEOUT)
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
pub fn link_exact(store: &PlacesStore) -> Fallible<usize> {
    let names: Vec<String> = undecided(&rows(store)?, &decisions(store)?)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let mut linked = 0;
    for found in resolve(&names)?.names {
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

    fn row(person: &str, place: &str, state: &str) -> Row {
        Row {
            person: person.into(),
            state: state.into(),
            place_id: format!("place:{place}"),
            place_name: place.into(),
            city: Some("Bonn".into()),
        }
    }

    #[test]
    fn rows_of_one_name_are_one_open_entry() {
        let rows = [
            row("Lucia", "a", "proposed"),
            row("lucia ", "b", "proposed"),
            row("Jonas", "c", "proposed"),
        ];
        let decided = HashMap::from([("jonas".to_string(), None)]);
        assert_eq!(undecided(&rows, &decided), [("Lucia".to_string(), 2)]);
    }

    #[test]
    fn a_person_lists_each_place_once_with_its_state() {
        let rows = [
            row("Lucia", "Bonn", "proposed"),
            row("Lucia", "Bonn", "confirmed"),
            row("Lucia", "Cafe", "confirmed"),
            row("Maria", "Elsewhere", "proposed"),
        ];
        let decided = HashMap::from([("lucia".to_string(), Some("ent:1".to_string()))]);
        let links = places_of(&rows, &decided, "ent:1").links;
        let shown: Vec<_> = links
            .iter()
            .map(|l| (l.title.as_str(), l.meta.as_deref()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Bonn", Some("proposed")),
                ("Cafe", Some("confirmed · Bonn"))
            ]
        );
    }
}
