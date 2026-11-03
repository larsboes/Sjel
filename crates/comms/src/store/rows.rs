//! Database rows mapped into the store facade's domain types.

use super::*;

// -- row mappers ---------------------------------------------------------

pub(super) fn row_to_triage(r: &Row) -> rusqlite::Result<TriageItem> {
    Ok(TriageItem {
        id: r.get(0)?,
        from_addr: r.get(1)?,
        subject: r.get(2)?,
        snippet: r.get(3)?,
        internal_date_ms: None, // ms is the write-side field; reads carry text
        internal_date_text: r.get(4)?,
        stream: r.get(5)?,
        rationale: r.get(6)?,
        status: r.get(7)?,
        first_seen: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
        last_seen: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
        classification_method: r.get(10)?,
        classification_version: r.get(11)?,
        data_class: r.get(12)?,
        data_class_rationale: r.get(13)?,
        data_classification_method: r.get(14)?,
        data_classification_version: r.get(15)?,
        gmail_action: r.get(16)?,
        gmail_action_at: r.get(17)?,
        purge_after: r.get(18)?,
        gmail_location: r.get(19)?,
        gmail_observed_at: r.get(20)?,
        gmail_sync_status: r.get(21)?,
        gmail_sync_action: r.get(22)?,
        gmail_sync_error: r.get(23)?,
        waiting: r.get(24)?,
        waiting_since: r.get(25)?,
    })
}

pub(super) fn row_to_feed_list(r: &Row) -> rusqlite::Result<FeedItem> {
    Ok(FeedItem {
        id: r.get(0)?,
        stream: r.get(1)?,
        kind: r.get(2)?,
        title: r.get(3)?,
        url: r.get(4)?,
        author: r.get(5)?,
        summary: r.get(6)?,
        transcript: r.get(7)?, // the list query selects a bare NULL here
        day: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
        created_at: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
        status: r.get(10)?,
        content_status: r
            .get::<_, Option<String>>(11)?
            .unwrap_or_else(|| "unknown".into()),
        summary_attempts: r.get::<_, Option<i32>>(12)?.unwrap_or(0),
        summary_last_error: r.get(13)?,
        summary_next_attempt: r.get(14)?,
        captured_via: r.get(15)?,
        // By name, not by index: this column was appended to four SELECTs whose
        // trailing positions already differ (the list query carries created_at
        // after it, for ordering), and a positional read would have to agree
        // with all of them.
        transcript_source: r
            .get::<_, Option<String>>("transcript_source")?
            .unwrap_or_else(|| "unknown".into()),
        // Raw extraction output lives in its own table; `get_raw_content` is
        // the only reader, and only the renormalize path asks for it.
        raw_content: None,
        summary_provenance: None,
        // By name for the same reason as transcript_source, and NOT NULL in the
        // schema with a fail-closed DEFAULT -- so a read that somehow misses
        // the column falls back to c1 rather than to the empty string, which no
        // gate would recognise as a class at all.
        data_class: r
            .get::<_, Option<String>>("data_class")?
            .unwrap_or_else(|| "c1".into()),
        data_class_rationale: r
            .get::<_, Option<String>>("data_class_rationale")?
            .unwrap_or_else(|| content_item::UNDECLARED_RATIONALE.into()),
        data_classification_method: r
            .get::<_, Option<String>>("data_classification_method")?
            .unwrap_or_else(|| content_item::METHOD_LEGACY.into()),
        data_classification_version: r
            .get::<_, Option<String>>("data_classification_version")?
            .unwrap_or_else(|| content_item::LEGACY_CLASSIFIER_VERSION.into()),
    })
}

pub(super) fn row_to_feed_full(r: &Row) -> rusqlite::Result<FeedItem> {
    Ok(FeedItem {
        id: r.get(0)?,
        stream: r.get(1)?,
        kind: r.get(2)?,
        title: r.get(3)?,
        url: r.get(4)?,
        author: r.get(5)?,
        summary: r.get(6)?,
        transcript: r.get(7)?,
        day: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
        created_at: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
        status: r.get(10)?,
        content_status: r
            .get::<_, Option<String>>(11)?
            .unwrap_or_else(|| "unknown".into()),
        summary_attempts: r.get::<_, Option<i32>>(12)?.unwrap_or(0),
        summary_last_error: r.get(13)?,
        summary_next_attempt: r.get(14)?,
        captured_via: r.get(15)?,
        // By name, not by index: this column was appended to four SELECTs whose
        // trailing positions already differ (the list query carries created_at
        // after it, for ordering), and a positional read would have to agree
        // with all of them.
        transcript_source: r
            .get::<_, Option<String>>("transcript_source")?
            .unwrap_or_else(|| "unknown".into()),
        // Raw extraction output lives in its own table; `get_raw_content` is
        // the only reader, and only the renormalize path asks for it.
        raw_content: None,
        summary_provenance: None,
        // By name for the same reason as transcript_source, and NOT NULL in the
        // schema with a fail-closed DEFAULT -- so a read that somehow misses
        // the column falls back to c1 rather than to the empty string, which no
        // gate would recognise as a class at all.
        data_class: r
            .get::<_, Option<String>>("data_class")?
            .unwrap_or_else(|| "c1".into()),
        data_class_rationale: r
            .get::<_, Option<String>>("data_class_rationale")?
            .unwrap_or_else(|| content_item::UNDECLARED_RATIONALE.into()),
        data_classification_method: r
            .get::<_, Option<String>>("data_classification_method")?
            .unwrap_or_else(|| content_item::METHOD_LEGACY.into()),
        data_classification_version: r
            .get::<_, Option<String>>("data_classification_version")?
            .unwrap_or_else(|| content_item::LEGACY_CLASSIFIER_VERSION.into()),
    })
}

pub(super) fn epoch_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}
