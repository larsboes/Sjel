//! `GET /api/sjel-status/search?q=` -- the server half of the app's finder.
//!
//! The finder ranks what the app already holds (pages, projects, tasks, notes) on every keystroke;
//! this answers for everything else, from the one shared database (`sjel_config::database_path`)
//! for legacy sources. Calendar, Entities and Inventory supply their own results.
//!
//! Every query word must appear in a source's text (`LIKE`, bound as a parameter, wildcards
//! escaped), and each source returns at most [`PER_SOURCE`] rows. A hit carries a `sjel://` link,
//! which the app opens through its existing deep-link parser, so a new source needs no client
//! change.
//!
//! Mail returns its subject and sender and never the snippet: a result list is a place a body
//! should not be read from (data classes, `shared-data-boundaries`). SQLite's `LIKE` folds ASCII
//! case only, so "Ä" does not find "ä" -- ponytail: an FTS5 index when that matters.

use super::*;
use sjel_store::rusqlite;
use std::path::Path;

/// Rows per source. The finder shows a screenful; more is a page, not a result.
const PER_SOURCE: usize = 5;
/// Words per query. Every word is one `LIKE` per row, and nobody types more to find a thing.
const MAX_WORDS: usize = 6;

pub use status_api::SearchHit;

/// One searchable table. Every string here is a constant SQL fragment; only the words are bound.
struct Source {
    kind: &'static str,
    table: &'static str,
    id: &'static str,
    title: &'static str,
    detail: &'static str,
    /// The text the words are matched against.
    haystack: &'static str,
    order: &'static str,
    link: fn(&str) -> String,
}

const SOURCES: &[Source] = &[
    Source {
        kind: "trip",
        table: "trips_plans",
        id: "id",
        title: "title",
        detail: "coalesce(date_start, '') || ' – ' || coalesce(date_end, '')",
        haystack: "title || ' ' || coalesce(destinations, '') || ' ' || coalesce(origin, '')",
        order: "date_start DESC",
        link: |id| format!("sjel://travel/{id}/overview"),
    },
    Source {
        kind: "mail",
        table: "comms_triage_items",
        id: "id",
        title: "coalesce(subject, '(no subject)')",
        detail: "coalesce(from_addr, '')",
        haystack: "coalesce(subject, '') || ' ' || coalesce(from_addr, '')",
        order: "internal_date DESC",
        link: |_| "sjel://inbox".into(),
    },
];

/// The words of a query as `LIKE` patterns: lowercased, wildcards escaped, at most [`MAX_WORDS`].
fn patterns(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .take(MAX_WORDS)
        .map(|word| {
            let escaped = word
                .to_lowercase()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            format!("%{escaped}%")
        })
        .collect()
}

fn table_exists(conn: &rusqlite::Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| Ok(row.get::<_, i64>(0)? > 0),
    )
    .unwrap_or(false)
}

/// One source's hits. A missing table or a failed query is no hits, not an error: one broken
/// source must not empty the whole answer.
fn search_source(
    conn: &rusqlite::Connection,
    source: &Source,
    patterns: &[String],
) -> Vec<SearchHit> {
    if !table_exists(conn, source.table) {
        return Vec::new();
    }
    let condition = (1..=patterns.len())
        .map(|index| format!("lower({}) LIKE ?{index} ESCAPE '\\'", source.haystack))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!(
        "SELECT CAST({id} AS TEXT), {title}, {detail} FROM {table} WHERE {condition} ORDER BY {order} LIMIT {limit}",
        id = source.id,
        title = source.title,
        detail = source.detail,
        table = source.table,
        order = source.order,
        limit = PER_SOURCE,
    );
    let Ok(mut statement) = conn.prepare(&sql) else {
        return Vec::new();
    };
    let rows = statement.query_map(rusqlite::params_from_iter(patterns.iter()), |row| {
        let id: String = row.get::<_, Option<String>>(0)?.unwrap_or_default();
        Ok(SearchHit {
            kind: source.kind.to_string(),
            title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            detail: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            link: (source.link)(&id),
        })
    });
    match rows {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

/// Every source's hits for `query`, in source order.
pub(crate) fn search(db_path: &Path, query: &str) -> Vec<SearchHit> {
    let patterns = patterns(query);
    if patterns.is_empty() {
        return Vec::new();
    }
    let Ok(pool) = sjel_store::open_pool(db_path, "search", |_| Ok(())) else {
        return Vec::new();
    };
    let Ok(conn) = pool.get() else {
        return Vec::new();
    };
    SOURCES
        .iter()
        .flat_map(|source| search_source(&conn, source, &patterns))
        .collect()
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchQuery {
    #[serde(default)]
    q: String,
}

#[derive(Deserialize)]
struct InventoryHits {
    hits: Vec<SearchHit>,
}

pub(super) async fn owner_get<T: serde::de::DeserializeOwned>(
    services: &[Service],
    name: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Option<T> {
    let service = services
        .iter()
        .find(|service| service.name == name && !service.is_external())?;
    let port = service.port.parse::<u16>().ok()?;
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .ok()
        })
        .as_ref()?;
    let mut request = client
        .get(format!("http://127.0.0.1:{port}{path}"))
        .query(query)
        .timeout(Duration::from_millis(500));
    if let Some(bearer) = sjel_server::InboundAuth::from_deployment().bearer_header() {
        request = request.header(reqwest::header::AUTHORIZATION, bearer);
    }
    let mut upstream = request.try_clone()?.send().await;
    if matches!(&upstream, Err(error) if error.is_connect())
        && service.autostart != "true"
        && super::lifecycle(service.name.clone(), "start")
            .await
            .is_ok()
    {
        upstream = request.send().await;
    }
    upstream.ok()?.error_for_status().ok()?.json().await.ok()
}

async fn owner_hits(query: &str) -> Vec<SearchHit> {
    let Ok(services) = registry().await else {
        return Vec::new();
    };
    let parameters = [("q", query)];
    let (calendar, entities, documents, inventory, subscriptions) = tokio::join!(
        owner_get::<Vec<calendar_api::SearchHit>>(
            &services,
            "calendar",
            calendar_api::ROUTE_SEARCH,
            &parameters
        ),
        owner_get::<entities_api::EntitySearchResponse>(
            &services,
            "entities",
            entities_api::ROUTE_SEARCH,
            &parameters,
        ),
        owner_get::<Vec<documents_api::FinderHit>>(
            &services,
            "documents",
            documents_api::ROUTE_FINDER,
            &parameters,
        ),
        owner_get::<InventoryHits>(&services, "inventory", "/api/search", &parameters),
        owner_get::<Vec<finance_api::SubscriptionSearchHit>>(
            &services,
            "finance",
            finance_api::ROUTE_SUBSCRIPTION_SEARCH,
            &parameters,
        ),
    );
    let entities = entities
        .into_iter()
        .flat_map(|response| response.hits)
        .take(PER_SOURCE)
        .map(|entity| SearchHit {
            kind: "person".into(),
            title: entity.name,
            detail: entity.kind,
            link: "sjel://people".into(),
        });
    let inventory = inventory.map_or_else(Vec::new, |response| response.hits);
    let calendar = calendar
        .unwrap_or_default()
        .into_iter()
        .take(PER_SOURCE)
        .map(|entry| SearchHit {
            kind: "event".into(),
            title: entry.title,
            detail: entry.detail,
            link: "sjel://calendar".into(),
        });
    let subscriptions = subscriptions
        .unwrap_or_default()
        .into_iter()
        .take(PER_SOURCE)
        .map(|hit| SearchHit {
            kind: "subscription".into(),
            title: hit.title,
            detail: hit.detail,
            link: "sjel://finance".into(),
        });
    let documents = documents
        .unwrap_or_default()
        .into_iter()
        .take(PER_SOURCE)
        .map(|hit| SearchHit {
            kind: "document".into(),
            title: hit.title,
            detail: hit.detail,
            link: "sjel://documents".into(),
        });
    calendar
        .chain(entities)
        .chain(documents)
        .chain(inventory.into_iter().take(PER_SOURCE))
        .chain(subscriptions)
        .collect()
}

pub(crate) async fn search_handler(
    axum::extract::Query(query): axum::extract::Query<SearchQuery>,
) -> Json<Value> {
    // A query is a few words; a longer one is not a search and is cut rather than run.
    let text: String = query.q.chars().take(200).collect();
    if text.split_whitespace().next().is_none() {
        return Json(json!({ "hits": [] }));
    }
    let db_path = sjel_config::database_path();
    let local_query = text.clone();
    let (local, mut hits) = tokio::join!(
        tokio::task::spawn_blocking(move || search(&db_path, &local_query)),
        owner_hits(&text),
    );
    hits.extend(local.unwrap_or_default());
    Json(json!({ "hits": hits }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_db(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("sjel-search-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE trips_plans (id TEXT, title TEXT, origin TEXT, destinations TEXT,
                                       date_start TEXT, date_end TEXT);
             INSERT INTO trips_plans VALUES ('t1', 'Lisbon week', 'Köln', 'Lisbon, Sintra',
                                             '2026-11-02', '2026-11-09');
             CREATE TABLE comms_triage_items (id TEXT, subject TEXT, from_addr TEXT,
                                              snippet TEXT, internal_date INTEGER);
             INSERT INTO comms_triage_items VALUES ('m1', 'Your Lisbon booking', 'tap@example.com',
                                                    'secret body text', 1),
                                                   ('m2', 'Receipt 100%', NULL, NULL, 2);",
        )
        .unwrap();
        path
    }

    #[test]
    fn every_word_must_match_and_each_hit_links_to_its_page() {
        let db = scratch_db("words");
        let hits = search(&db, "lisbon");
        let kinds: Vec<&str> = hits.iter().map(|hit| hit.kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["trip", "mail"],
            "source order, missing tables skipped"
        );
        assert_eq!(hits[0].link, "sjel://travel/t1/overview");
        assert_eq!(hits[0].detail, "2026-11-02 – 2026-11-09");

        assert_eq!(
            search(&db, "lisbon sintra").len(),
            1,
            "both words, trip only"
        );
        assert!(search(&db, "lisbon zzz").is_empty());
        assert_eq!(search(&db, "LISBON").len(), 2, "ASCII case folds");
        assert!(search(&db, "   ").is_empty());
    }

    #[test]
    fn mail_never_returns_its_body_and_wildcards_are_literal() {
        let db = scratch_db("mail");
        // The snippet is not searched and not returned.
        assert!(search(&db, "secret").is_empty());
        let mail = &search(&db, "booking")[0];
        assert_eq!(mail.detail, "tap@example.com");
        // `%` is a character to find, not "anything".
        let hits = search(&db, "%");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Receipt 100%");
    }
}
