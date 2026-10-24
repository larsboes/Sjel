//! `GET /api/sjel-status/search?q=` -- the server half of the app's finder.
//!
//! The finder ranks what the app already holds (pages, projects, tasks, notes) on every keystroke;
//! this answers for everything else, from the one shared database (`sjel_config::database_path`)
//! the way [`super::now`] reads it: read-only, one query per source, no call to a capability
//! process. A capability that is down still has its rows found.
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

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SearchHit {
    pub kind: &'static str,
    pub title: String,
    pub detail: String,
    /// Where the app goes: `sjel://calendar`, `sjel://travel/<id>/overview`, ...
    pub link: String,
}

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
        kind: "event",
        table: "calendar_entries",
        id: "id",
        title: "title",
        detail: "substr(starts_at, 1, 16) || coalesce(' · ' || location, '')",
        haystack: "title || ' ' || coalesce(location, '')",
        // Nearest to now first, ahead or behind: the event you mean is rarely the oldest.
        order: "abs(julianday(starts_at) - julianday('now'))",
        link: |_| "sjel://calendar".into(),
    },
    Source {
        kind: "person",
        table: "entities_entities",
        id: "id",
        title: "name",
        detail: "kind",
        haystack: "name",
        order: "length(name)",
        link: |_| "sjel://people".into(),
    },
    Source {
        kind: "document",
        table: "documents_items",
        id: "id",
        title: "coalesce(extracted_title, filename)",
        detail: "trim(coalesce(category, '') || ' ' || coalesce(project, ''))",
        haystack: "coalesce(extracted_title, '') || ' ' || filename || ' ' || coalesce(category, '') || ' ' || coalesce(project, '')",
        order: "imported_at DESC",
        link: |_| "sjel://documents".into(),
    },
    Source {
        kind: "item",
        table: "inventory_item",
        id: "id",
        title: "label",
        detail: "coalesce(category, kind)",
        haystack: "label || ' ' || coalesce(category, '') || ' ' || coalesce(notes, '')",
        order: "length(label)",
        link: |_| "sjel://inventory".into(),
    },
    Source {
        kind: "subscription",
        table: "finance_subscriptions",
        id: "id",
        title: "name",
        detail: "coalesce(category, '')",
        haystack: "name || ' ' || coalesce(category, '')",
        order: "name",
        link: |_| "sjel://finance".into(),
    },
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
            kind: source.kind,
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

pub(crate) async fn search_handler(
    axum::extract::Query(query): axum::extract::Query<SearchQuery>,
) -> Json<Value> {
    // A query is a few words; a longer one is not a search and is cut rather than run.
    let text: String = query.q.chars().take(200).collect();
    let db_path = sjel_config::database_path();
    let hits = tokio::task::spawn_blocking(move || search(&db_path, &text))
        .await
        .unwrap_or_default();
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
            "CREATE TABLE entities_entities (id TEXT, kind TEXT, name TEXT);
             INSERT INTO entities_entities VALUES ('1', 'person', 'Anna Berg'), ('2', 'person', 'Bob 100%');
             CREATE TABLE trips_plans (id TEXT, title TEXT, origin TEXT, destinations TEXT,
                                       date_start TEXT, date_end TEXT);
             INSERT INTO trips_plans VALUES ('t1', 'Lisbon week', 'Köln', 'Lisbon, Sintra',
                                             '2026-11-02', '2026-11-09');
             CREATE TABLE comms_triage_items (id TEXT, subject TEXT, from_addr TEXT,
                                              snippet TEXT, internal_date INTEGER);
             INSERT INTO comms_triage_items VALUES ('m1', 'Your Lisbon booking', 'tap@example.com',
                                                    'secret body text', 1);",
        )
        .unwrap();
        path
    }

    #[test]
    fn every_word_must_match_and_each_hit_links_to_its_page() {
        let db = scratch_db("words");
        let hits = search(&db, "lisbon");
        let kinds: Vec<&str> = hits.iter().map(|hit| hit.kind).collect();
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
        assert_eq!(
            search(&db, "ANNA")[0].title,
            "Anna Berg",
            "ASCII case folds"
        );
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
        assert_eq!(hits[0].title, "Bob 100%");
    }
}
