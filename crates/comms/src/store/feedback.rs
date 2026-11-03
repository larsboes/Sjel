//! The interaction ledger and the labels a learned factor may train on.
//!
//! `{prefix}_feed_items.status` is a mutable enum: it says what the operator
//! decided, never when, and a second decision erases the first. That is enough
//! to render a library and not enough to learn anything — there is no decay, no
//! time-ordered holdout, and no way to tell a keep made this morning from one
//! made in August. This module owns the append-only ledger that fixes it.
//!
//! Two rules are enforced here rather than by convention:
//!
//! 1. **One writer per verb.** `kept`, `dismissed` and `unkept` are written
//!    only by [`Store::set_feed_status`], inside the same transaction as the
//!    UPDATE. [`Store::record_interaction`] — the HTTP path — refuses those
//!    three. Two paths writing one decision would double every count in a
//!    table whose whole justification is that it can be read by hand.
//! 2. **The class ladder governs training too.** An item that fails
//!    `content_item::local_prompt_allowed` contributes no label and no feature
//!    vector. Stating the rule for embeddings and leaving it unstated for
//!    training is the gap that let 11 c3 mails reach an embedding model.

use super::*;

/// The verbs the status route owns. Naming them once keeps the refusal message
/// and the label query from disagreeing about what "decisive" means.
pub(super) const DECISIVE_EVENTS: [&str; 3] = ["kept", "dismissed", "unkept"];

/// The verbs a client may post. `shared` is in the table's CHECK vocabulary and
/// has no writer yet, so it is deliberately absent here.
const CLIENT_EVENTS: [&str; 2] = ["opened", "reopened"];

/// Where a press can come from. One list, so the status route, the ledger
/// route and the CHECK constraint cannot disagree about the vocabulary.
pub(super) const SURFACES: [&str; 6] = ["inbox", "reader", "library", "home", "cli", "api"];

/// The refusal message both writers use, so a bad surface reads the same
/// whichever route received it.
pub(super) fn reject_surface(surface: &str) -> Option<String> {
    (!SURFACES.contains(&surface)).then(|| {
        format!(
            "invalid surface '{surface}' -- must be one of: {}",
            SURFACES.join(", ")
        )
    })
}

/// Append one row. Takes a connection rather than a `&Store` so the status
/// route can write the ledger inside its own transaction: a decision that
/// commits and a ledger row that does not is exactly the loss this table
/// exists to prevent.
pub(super) fn insert_interaction(
    conn: &Connection,
    prefix: &str,
    feed_id: &str,
    event: &str,
    surface: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        &format!(
            "INSERT INTO {prefix}_feed_interactions (feed_id, event, surface, occurred_at)
             VALUES (?1,?2,?3,{now})",
            now = sjel_store::NOW
        ),
        params![&feed_id, &event, &surface],
    )?;
    Ok(())
}

impl Store {
    /// Record an `opened` or `reopened` press.
    ///
    /// `Ok(false)` means the item does not exist, which the route answers 404.
    /// A decisive verb is an error naming the route that owns it, so a client
    /// that tries to become a second writer is told where the first one is.
    pub fn record_interaction(
        &self,
        feed_id: &str,
        event: &str,
        surface: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if DECISIVE_EVENTS.contains(&event) {
            return Err(format!(
                // `{{id}}` and not `{id}`: this is a format string, and the route pattern is
                // literal text in it since axum 0.8 renamed the pattern syntax (see the test
                // below, which matches on the rendered message).
                "'{event}' is written by POST /feed/{{id}}/status, not by this route"
            )
            .into());
        }
        if !CLIENT_EVENTS.contains(&event) {
            return Err(format!(
                "invalid interaction event '{event}' -- must be one of: {}",
                CLIENT_EVENTS.join(", ")
            )
            .into());
        }
        if let Some(refusal) = reject_surface(surface) {
            return Err(refusal.into());
        }
        let conn = self.conn()?;
        let exists = conn
            .query_row(
                &format!("SELECT 1 FROM {}_feed_items WHERE id = ?1", self.prefix),
                params![&feed_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .is_some();
        if !exists {
            return Ok(false);
        }
        insert_interaction(&conn, &self.prefix, feed_id, event, surface)?;
        Ok(true)
    }

    /// How many of each verb the ledger holds.
    ///
    /// `GET /feed/evaluation/status` reports it as `interactions`, beside the
    /// gate's own `samples`, so a reader can check the cold-start claim against
    /// the table itself -- and so `opened` and `reopened`, which no training
    /// label reads, reach a surface at all.
    pub fn interaction_counts(&self) -> Result<InteractionCounts, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let rows = conn.query_all(
            &format!(
                "SELECT event, COUNT(*) FROM {}_feed_interactions GROUP BY event",
                self.prefix
            ),
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )?;
        let mut counts = InteractionCounts::default();
        for (event, count) in rows {
            match event.as_str() {
                "opened" => counts.opened = count,
                "kept" => counts.kept = count,
                "dismissed" => counts.dismissed = count,
                "reopened" => counts.reopened = count,
                "unkept" => counts.unkept = count,
                "shared" => counts.shared = count,
                _ => {}
            }
            counts.total += count;
        }
        Ok(counts)
    }

    /// Every decision a model may learn from.
    ///
    /// The label rule, stated once in SQL and once here: an item's label is its
    /// most recent row whose event is decisive, and a most-recent `unkept` is a
    /// retraction rather than a label. `MAX(interaction_id)` rather than
    /// `MAX(occurred_at)` decides "most recent" because the column is
    /// AUTOINCREMENT over an append-only table, so it orders two rows written
    /// in the same millisecond and a text timestamp does not.
    ///
    /// Items decided before this ledger existed have a status and no row. They
    /// are seeded from `status` and marked `seeded_from_status`, because the
    /// decision is real while its date is not — the trainer weights those at
    /// the decay floor rather than pretending they happened at capture time.
    pub fn training_labels(&self) -> Result<TrainingLabels, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        let decided = conn.query_all(
            &format!(
                "SELECT item.id, item.data_class, ledger.event,
                        CAST(strftime('%s', ledger.occurred_at) AS INTEGER)
                 FROM {prefix}_feed_interactions ledger
                 JOIN {prefix}_feed_items item ON item.id = ledger.feed_id
                 WHERE ledger.event IN ('kept','dismissed','unkept')
                   AND ledger.interaction_id = (
                       SELECT MAX(latest.interaction_id)
                       FROM {prefix}_feed_interactions latest
                       WHERE latest.feed_id = ledger.feed_id
                         AND latest.event IN ('kept','dismissed','unkept'))",
                prefix = self.prefix
            ),
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                ))
            },
        )?;
        let seeded = conn.query_all(
            &format!(
                "SELECT item.id, item.data_class, item.status,
                        CAST(strftime('%s', item.created_at) AS INTEGER)
                 FROM {prefix}_feed_items item
                 WHERE item.status IN ('keeper','dismissed')
                   AND NOT EXISTS (
                       SELECT 1 FROM {prefix}_feed_interactions ledger
                       WHERE ledger.feed_id = item.id
                         AND ledger.event IN ('kept','dismissed','unkept'))",
                prefix = self.prefix
            ),
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                ))
            },
        )?;

        let mut labels = Vec::new();
        let mut skipped_class = 0usize;
        let mut seeded_from_status = 0usize;
        for (feed_id, data_class, event, decided_at) in decided {
            if !crate::content_item::local_prompt_allowed(&data_class) {
                skipped_class += 1;
                continue;
            }
            let kept = match event.as_str() {
                "kept" => true,
                "dismissed" => false,
                // A retraction. The operator took the decision back, so there
                // is nothing here to learn from.
                _ => continue,
            };
            labels.push(FeedbackLabel {
                feed_id,
                kept,
                decided_at,
                seeded_from_status: false,
            });
        }
        for (feed_id, data_class, status, captured_at) in seeded {
            if !crate::content_item::local_prompt_allowed(&data_class) {
                skipped_class += 1;
                continue;
            }
            seeded_from_status += 1;
            labels.push(FeedbackLabel {
                feed_id,
                kept: status == "keeper",
                decided_at: captured_at,
                seeded_from_status: true,
            });
        }
        labels.sort_by(|left, right| {
            left.decided_at
                .cmp(&right.decided_at)
                .then_with(|| left.feed_id.cmp(&right.feed_id))
        });
        Ok(TrainingLabels {
            labels,
            skipped_class,
            seeded_from_status,
        })
    }
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::store::db_tests::open_test_store;

    fn stored_item(store: &Store, url: &str, class: Option<&str>) -> String {
        let mut item = FeedItem::new(url, "news", "article");
        item.title = Some("A Title".into());
        if let Some(class) = class {
            item.data_class = class.into();
        }
        store.upsert_feed(&item).expect("the fixture stores");
        item.id
    }

    fn events(store: &Store, feed_id: &str) -> Vec<String> {
        let conn = store.conn().expect("a connection");
        conn.query_all(
            &format!(
                "SELECT event FROM {}_feed_interactions WHERE feed_id = ?1 ORDER BY interaction_id",
                store.prefix
            ),
            params![&feed_id],
            |row| row.get::<_, String>(0),
        )
        .expect("the ledger reads")
    }

    #[test]
    fn a_status_change_writes_exactly_one_ledger_row() {
        let store = open_test_store("feedback_one_row");
        let id = stored_item(&store, "https://example.com/one-row", None);

        assert!(store
            .set_feed_status(&id, "keeper", "inbox")
            .expect("the keep lands"));
        assert_eq!(events(&store, &id), vec!["kept".to_string()]);

        // The double-write objection: the decision produces one row, not one
        // per path that could have observed it.
        assert!(store
            .set_feed_status(&id, "dismissed", "reader")
            .expect("the dismiss lands"));
        assert_eq!(
            events(&store, &id),
            vec!["kept".to_string(), "dismissed".to_string()]
        );

        let conn = store.conn().unwrap();
        let surfaces = conn
            .query_all(
                &format!(
                    "SELECT surface FROM {}_feed_interactions WHERE feed_id = ?1 ORDER BY interaction_id",
                    store.prefix
                ),
                params![&id],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(surfaces, vec!["inbox".to_string(), "reader".to_string()]);

        // A refused status writes neither the UPDATE nor a ledger row.
        assert!(store.set_feed_status(&id, "banana", "api").is_err());
        assert_eq!(events(&store, &id).len(), 2);

        // An unknown item writes nothing either.
        assert!(!store
            .set_feed_status("missing", "keeper", "api")
            .expect("an unknown id is not an error"));
    }

    /// One row per status CHANGE, not per POST.
    ///
    /// The Inbox binds `u` and `d` unconditionally, so a second press on a row
    /// that is already in that status used to append a decision that never
    /// happened: an UPDATE that writes the value a column already holds still
    /// reports one row affected. `training_labels` survived it -- it takes the
    /// most recent decisive row -- but `interaction_counts`, which the status
    /// endpoint now reports, counted the press twice.
    #[test]
    fn a_repeated_identical_status_writes_no_second_ledger_row() {
        let store = open_test_store("feedback_repeat_no_row");
        let id = stored_item(&store, "https://example.com/repeat", None);

        assert!(store.set_feed_status(&id, "keeper", "inbox").unwrap());
        assert!(store.set_feed_status(&id, "keeper", "inbox").unwrap());
        assert!(store.set_feed_status(&id, "keeper", "reader").unwrap());
        assert_eq!(events(&store, &id), vec!["kept".to_string()]);

        // The status still reads back, and a real change still writes its row.
        assert_eq!(
            store.get_feed_status(&id).unwrap().as_deref(),
            Some("keeper")
        );
        assert!(store.set_feed_status(&id, "new", "inbox").unwrap());
        assert_eq!(
            events(&store, &id),
            vec!["kept".to_string(), "unkept".to_string()]
        );
        // A row that never moved from `new` records nothing at all.
        let untouched = stored_item(&store, "https://example.com/untouched", None);
        assert!(store.set_feed_status(&untouched, "new", "inbox").unwrap());
        assert!(events(&store, &untouched).is_empty());
    }

    #[test]
    fn the_ledger_route_refuses_a_decisive_verb() {
        let store = open_test_store("feedback_refuses_decisive");
        let id = stored_item(&store, "https://example.com/refusal", None);
        for event in DECISIVE_EVENTS {
            let error = store
                .record_interaction(&id, event, "inbox")
                .expect_err("a decisive verb has one writer");
            assert!(
                error.to_string().contains("POST /feed/{id}/status"),
                "the refusal must name the owner: {error}"
            );
        }
        assert!(store.record_interaction(&id, "opened", "reader").unwrap());
        assert!(store.record_interaction(&id, "shared", "reader").is_err());
        assert!(store.record_interaction(&id, "opened", "watch").is_err());
        assert!(!store
            .record_interaction("missing", "opened", "reader")
            .unwrap());
        assert_eq!(events(&store, &id), vec!["opened".to_string()]);
    }

    #[test]
    fn the_label_is_the_last_decisive_event() {
        let store = open_test_store("feedback_last_event");
        let flip = stored_item(&store, "https://example.com/flip", None);
        store.set_feed_status(&flip, "keeper", "inbox").unwrap();
        store.set_feed_status(&flip, "dismissed", "inbox").unwrap();
        store.set_feed_status(&flip, "keeper", "inbox").unwrap();

        let retracted = stored_item(&store, "https://example.com/retracted", None);
        store
            .set_feed_status(&retracted, "keeper", "inbox")
            .unwrap();
        store.set_feed_status(&retracted, "new", "inbox").unwrap();

        // Decided before the ledger existed: a status and no row.
        let seeded = stored_item(&store, "https://example.com/seeded", None);
        let conn = store.conn().unwrap();
        conn.execute(
            &format!(
                "UPDATE {}_feed_items SET status = 'keeper' WHERE id = ?1",
                store.prefix
            ),
            params![&seeded],
        )
        .unwrap();
        drop(conn);

        let training = store.training_labels().expect("labels read");
        let by_id = |id: &str| {
            training
                .labels
                .iter()
                .find(|label| label.feed_id == id)
                .cloned()
        };
        assert_eq!(by_id(&flip).map(|label| label.kept), Some(true));
        assert!(
            by_id(&retracted).is_none(),
            "a retracted decision is not a label"
        );
        let seed = by_id(&seeded).expect("an undated decision is still a decision");
        assert!(seed.kept);
        assert!(seed.seeded_from_status);
        assert_eq!(training.seeded_from_status, 1);

        let counts = store.interaction_counts().unwrap();
        assert_eq!(counts.kept, 3);
        assert_eq!(counts.dismissed, 1);
        assert_eq!(counts.unkept, 1);
        assert_eq!(counts.total, 5);
    }

    #[test]
    fn a_refused_class_contributes_no_label() {
        let store = open_test_store("feedback_class_gate");
        let refused = stored_item(&store, "https://example.com/private", Some("c3"));
        store.set_feed_status(&refused, "keeper", "inbox").unwrap();
        let allowed = stored_item(&store, "https://example.com/public", Some("c0"));
        store.set_feed_status(&allowed, "keeper", "inbox").unwrap();

        let training = store.training_labels().unwrap();
        assert_eq!(training.skipped_class, 1);
        assert_eq!(training.labels.len(), 1);
        assert_eq!(training.labels[0].feed_id, allowed);
    }
    #[test]
    fn an_id_older_than_the_page_is_still_selectable() {
        // The refresh route selected the newest page and THEN retained the
        // requested ids, so an explicitly named item outside that window was
        // silently dropped and could never be re-scored -- 172 of 372 items.
        let store = open_test_store("feed_ids_before_limit");
        let mut ids = Vec::new();
        for index in 0..250 {
            let mut item = FeedItem::new(
                &format!("https://example.com/paged/{index}"),
                "news",
                "article",
            );
            item.title = Some(format!("Item {index}"));
            store.upsert_feed(&item).expect("the fixture stores");
            ids.push(item.id);
        }
        let page = store
            .feed_for_relevance(3650, 200, 0)
            .expect("a bounded page");
        assert_eq!(page.len(), 200);
        let second_page = store
            .feed_for_relevance(3650, 200, 200)
            .expect("the next page");
        assert_eq!(second_page.len(), 50, "offset walks past the first page");

        let wanted = ids[239].clone();
        let selected = store
            .feed_items_by_ids(std::slice::from_ref(&wanted))
            .expect("selection by id");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, wanted);
    }
}
