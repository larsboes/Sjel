//! Capture origins and collector-run projections.

use super::*;

impl Store {
    pub fn record_feed_origin(
        &self,
        feed_id: &str,
        source_id: &str,
        source_ref: &str,
        label: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        conn.execute(
            &format!(
                "INSERT INTO {prefix}_feed_origins
                    (feed_id, source_id, source_ref, label, first_seen, last_seen)
                 VALUES (?1,?2,?3,?4,{now},{now})
                 ON CONFLICT (feed_id, source_id, source_ref) DO UPDATE SET
                    label = COALESCE(excluded.label, {prefix}_feed_origins.label),
                    last_seen = {now}",
                prefix = self.prefix,
                now = sjel_store::NOW
            ),
            params![&feed_id, &source_id, &source_ref, &label],
        )?;
        Ok(())
    }

    pub fn feed_origins(
        &self,
        feed_id: &str,
    ) -> Result<Vec<FeedOrigin>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        Ok(conn.query_all(
            &format!(
                "SELECT source_id, source_ref, label
                 FROM {}_feed_origins WHERE feed_id = ?1 ORDER BY first_seen",
                self.prefix
            ),
            params![&feed_id],
            |row| {
                Ok(FeedOrigin {
                    source_id: row.get(0)?,
                    source_ref: row.get(1)?,
                    label: row.get(2)?,
                })
            },
        )?)
    }

    pub fn list_origin_summaries(&self) -> Result<Vec<OriginSummary>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // The `::text` casts are gone with the timestamp type: these columns are
        // TEXT, and the canonical stamp sorts as text (libs/sjel-store/README.md).
        Ok(conn.query_all(
            &format!(
                "SELECT source_id, COUNT(DISTINCT feed_id), MIN(first_seen), MAX(last_seen)
                 FROM {}_feed_origins
                 GROUP BY source_id
                 ORDER BY MAX(last_seen) DESC",
                self.prefix
            ),
            [],
            |r| {
                Ok(OriginSummary {
                    source_id: r.get(0)?,
                    item_count: r.get(1)?,
                    first_seen: r.get(2)?,
                    last_seen: r.get(3)?,
                })
            },
        )?)
    }

    /// Which items arrived together, derived at read time from `feed_origins`
    /// alone (#84). A "run" is a cluster of arrivals for one source: ordered by
    /// `first_seen`, a gap longer than `RUN_GAP_MINUTES` starts a new one.
    ///
    /// Nothing is stored for this — no run id on the item, no batch table. A
    /// collector that fetches each URL can take a while, so the gap threshold
    /// is generous; two genuine runs of the same source inside half an hour
    /// read as one, which is the failure this trades for never having to
    /// migrate a grouping decision.
    pub fn list_feed_runs(&self, days: i32) -> Result<Vec<FeedRun>, Box<dyn std::error::Error>> {
        let conn = self.conn()?;
        // Two translations here, and both depend on the canonical stamp being a
        // format SQLite's own date functions can read (libs/sjel-store/README.md):
        //
        //   CURRENT_DATE - $1::int         -> date('now', '-N days')
        //   a - b > interval 'N minutes'   -> julianday(a) - julianday(b) > N/1440
        //
        // julianday returns days as a float, so the gap is expressed in days.
        // Everything else -- LAG, SUM OVER ... ROWS UNBOUNDED PRECEDING, the CTEs --
        // is supported verbatim.
        Ok(conn.query_all(
            &format!(
                "WITH ordered AS (
                     SELECT o.feed_id, o.source_id, o.label, o.first_seen,
                            LAG(o.first_seen) OVER (
                                PARTITION BY o.source_id ORDER BY o.first_seen
                            ) AS prev_seen
                     FROM {prefix}_feed_origins o
                     JOIN {prefix}_feed_items f ON f.id = o.feed_id
                     WHERE f.day >= date('now', '-' || ?1 || ' days')
                 ),
                 marked AS (
                     SELECT *,
                            CASE
                                WHEN prev_seen IS NULL
                                  OR julianday(first_seen) - julianday(prev_seen)
                                     > {gap}.0 / 1440.0
                                THEN 1 ELSE 0
                            END AS starts_run
                     FROM ordered
                 ),
                 runs AS (
                     SELECT feed_id, source_id, label, first_seen,
                            SUM(starts_run) OVER (
                                PARTITION BY source_id ORDER BY first_seen
                                ROWS UNBOUNDED PRECEDING
                            ) AS run_seq
                     FROM marked
                 )
                 SELECT feed_id,
                        source_id,
                        label,
                        source_id || '#' || CAST(run_seq AS TEXT) AS run_key,
                        MIN(first_seen) OVER (PARTITION BY source_id, run_seq) AS run_started
                 FROM runs
                 ORDER BY run_started DESC, first_seen ASC",
                prefix = self.prefix,
                gap = RUN_GAP_MINUTES
            ),
            params![days],
            |r| {
                Ok(FeedRun {
                    feed_id: r.get(0)?,
                    source_id: r.get(1)?,
                    label: r.get(2)?,
                    run_key: r.get(3)?,
                    run_started: r.get(4)?,
                })
            },
        )?)
    }
}
