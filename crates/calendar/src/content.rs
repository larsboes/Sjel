//! Phase G: calendar entries as `content-item-v2`.
//!
//! Pure over one `Entry`. Nothing here queries, and the store is untouched —
//! this is a projection the reader asks for, not a second copy of the data.
//! `libs/content-item/README.md` has the argument for why the *contract* is
//! shared while the tables stay apart.
//!
//! Two things this deliberately does not do:
//!
//! - **It does not rank.** `relevance` and `evaluation` stay empty because a
//!   calendar entry is already decided; `commitment` is its triage axis and it
//!   surfaces as `status`. The schema enforces this for `source = calendar`.
//! - **It does not invent a summary.** `summary` is what the thing *is*, taken
//!   from whatever the source recorded; `notes` is why the operator cares, and
//!   stays in the calendar extension where no machine writes it.

use serde_json::Value;

use crate::content_item::{
    CalendarExtension, CloudProcessing, ContentItem, DataClass, Link, Origin, SCHEMA_VERSION,
};
use crate::date;
use crate::model::Entry;

/// Where an entry lives when it lives nowhere else. The dashboard serves
/// calendar and this API on one origin, so a root-relative path is
/// unambiguous — see the schema's note on `uri-reference`.
fn dashboard_link(entry: &Entry) -> String {
    format!(
        "/calendar?date={}&entry={}",
        &entry.starts_at[..10.min(entry.starts_at.len())],
        entry.id
    )
}

/// A string field out of `payload`, if it is actually there and non-empty.
fn payload_text(entry: &Entry, key: &str) -> Option<String> {
    entry
        .payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// The shared `links[]` vocabulary, read from `payload.links`.
///
/// Tolerant on purpose: an entry written before this existed simply has none,
/// and one malformed row drops that row rather than failing the projection.
/// A link is only useful if it has somewhere to go, so `url` is the one field
/// that must be present.
fn links_of(entry: &Entry) -> Vec<Link> {
    let mut links: Vec<Link> = entry
        .payload
        .get("links")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item.get("url").and_then(Value::as_str)?.trim();
                    if url.is_empty() {
                        return None;
                    }
                    Some(Link::new(
                        item.get("label")
                            .and_then(Value::as_str)
                            .filter(|label| !label.trim().is_empty())
                            .unwrap_or("Link"),
                        item.get("kind").and_then(Value::as_str).unwrap_or("source"),
                        url,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    // `payload.url` is what every provider adapter already writes for "the page
    // this came from". Promoting it into the shared vocabulary is what lets one
    // reader show a Luma event and a hand-entered ticket the same way, without
    // either side knowing about the other.
    if let Some(url) = payload_text(entry, "url") {
        if !links.iter().any(|link| link.url == url) {
            links.insert(0, Link::new("Source", "source", url));
        }
    }
    links
}

/// Which provider contributed this entry, when one did.
///
/// `external_id` is the provider's own key, so it is the `source_ref` — the
/// same value `upsert_external_entry` dedupes on.
fn origins_of(entry: &Entry) -> Vec<Origin> {
    match entry.external_id.as_deref() {
        Some(external_id) if !external_id.trim().is_empty() => vec![Origin {
            source_id: entry.source.clone(),
            source_ref: external_id.to_string(),
            label: payload_text(entry, "venue").or_else(|| payload_text(entry, "city")),
        }],
        _ => Vec::new(),
    }
}

/// Calendar stores `created_at` as unix seconds; feed and mail emit ISO-8601.
/// A reader that sorts a mixed list has to compare them, and `"1785414018"`
/// sorts before every ISO string ever written. Converting here is the whole
/// reason this is not just `entry.created_at.clone()`.
fn iso_utc(unix_seconds: &str) -> String {
    let Ok(seconds) = unix_seconds.trim().parse::<i64>() else {
        // Already ISO, or something we did not write. Pass it through rather
        // than fabricate a timestamp.
        return unix_seconds.to_string();
    };
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        date::format_date(days),
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// The operator's own schedule is personal by construction — a public concert
/// still says the flat is empty that evening. Stated as a rationale rather than
/// a bare enum so the reader can show *why* it is treated this way.
///
/// `pub` since 2026-09-08 (B50): the entry LIST states the same class, and it
/// states it by calling this rather than by repeating the literal. One
/// declaration, two readers — a second `"c1"` in `server.rs` would be a second
/// place deciding what a calendar entry is worth, which is what §6.1 forbids.
pub fn classification() -> DataClass {
    DataClass::personal_source_default(
        "Where the operator is and when is personal, whatever the event itself is.",
    )
}

pub fn from_entry(entry: &Entry) -> ContentItem {
    let summary = payload_text(entry, "about").or_else(|| payload_text(entry, "summary"));
    let content = entry
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|notes| !notes.is_empty())
        .map(str::to_string);
    let classification = classification();
    let policy = crate::content_item::processing_policy(&classification.value);

    ContentItem {
        schema_version: SCHEMA_VERSION,
        source: "calendar",
        id: entry.id.clone(),
        // The calendar kind *is* the type discriminator the reader switches on:
        // event, nightlife, work_onsite, away. Not flattened to "calendar",
        // because the kind is what tells you how to read the row.
        kind: entry.kind.clone(),
        title: Some(entry.title.clone()),
        url: dashboard_link(entry),
        // An entry has no author. Null rather than the source name: "who wrote
        // this" and "which adapter carried it" are different questions, and
        // `origins` already answers the second.
        author: None,
        summary,
        content_status: if content.is_some() { "full" } else { "none" },
        content,
        content_label: "Notes".into(),
        day: entry.starts_at[..10.min(entry.starts_at.len())].to_string(),
        created_at: iso_utc(&entry.created_at),
        // The triage axis, in the field the reader already reads for it.
        status: entry.commitment.as_str().to_string(),
        data_class: classification,
        processing_policy: policy,
        cloud_processing: CloudProcessing::not_prepared(),
        relevance: Vec::new(),
        evaluation: None,
        processing: Vec::new(),
        origins: origins_of(entry),
        links: links_of(entry),
        // Calendar does not generate digests: comms owns the digest engine and
        // reads an entry over HTTP to produce one, the same bounded
        // cross-capability read it already does against Trips. This projection
        // stays pure — it queries nothing — so the field is null here and the
        // reader fetches the digest from comms alongside it.
        digest: None,
        mail: None,
        calendar: Some(CalendarExtension {
            starts_at: entry.starts_at.clone(),
            ends_at: entry.ends_at.clone(),
            all_day: entry.all_day,
            commitment: entry.commitment.as_str().to_string(),
            location: entry.location.clone(),
            notes: entry.notes.clone(),
            entry_source: entry.source.clone(),
            rhythm_id: entry.rhythm_id.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Commitment;
    use serde_json::json;

    fn entry() -> Entry {
        Entry {
            id: "cal:entry:abc".into(),
            kind: "event".into(),
            commitment: Commitment::Committed,
            title: "Phantasialand".into(),
            starts_at: "2026-08-10T09:00:00".into(),
            ends_at: "2026-08-10T19:00:00".into(),
            all_day: false,
            location: Some("Phantasialand, 50321 Brühl".into()),
            notes: Some("Dated ticket, valid only on 10.08.".into()),
            attendees: Vec::new(),
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: json!({}),
            created_at: "1785414018".into(),
            updated_at: "1785414018".into(),
        }
    }

    #[test]
    fn an_entry_projects_into_the_contract() {
        let item = from_entry(&entry());
        assert_eq!(item.source, "calendar");
        assert_eq!(item.schema_version, "content-item-v2");
        assert_eq!(
            item.kind, "event",
            "the kind is the discriminator, not 'calendar'"
        );
        assert_eq!(item.day, "2026-08-10");
        assert_eq!(item.status, "committed");
        assert_eq!(item.url, "/calendar?date=2026-08-10&entry=cal:entry:abc");
        let extension = item
            .calendar
            .expect("a calendar item carries its extension");
        assert_eq!(extension.ends_at, "2026-08-10T19:00:00");
        assert!(!extension.all_day);
        assert_eq!(extension.commitment, "committed");
    }

    /// The constraint the schema also enforces. A decided item is not ranked,
    /// and the regression this prevents is real: a promoted Luma event used to
    /// arrive carrying `score: 0.0` and `matched_focus` set to one of the interest profiles.
    #[test]
    fn a_calendar_item_is_never_ranked_even_when_the_payload_has_a_score() {
        let mut noisy = entry();
        noisy.payload = json!({ "score": 0.0, "matched_focus": "some-interest-profile", "rationale": "low fit" });
        let item = from_entry(&noisy);
        assert!(
            item.relevance.is_empty(),
            "calendar ranks by commitment, never by score"
        );
        assert!(item.evaluation.is_none());
    }

    #[test]
    fn payload_url_and_links_become_one_shared_vocabulary() {
        let mut rich = entry();
        rich.payload = json!({
            "url": "https://www.phantasialand.de/",
            "links": [
                { "label": "E-Tickets", "kind": "mail", "url": "https://mail.google.com/mail/u/0/#all/abc" },
                { "label": "", "url": "https://example.com/order" },
                { "label": "broken", "kind": "mail" }
            ]
        });
        let item = from_entry(&rich);
        assert_eq!(
            item.links.len(),
            3,
            "the entry with no url is dropped, the rest survive"
        );
        assert_eq!(item.links[0].label, "Source", "payload.url leads");
        assert_eq!(item.links[0].url, "https://www.phantasialand.de/");
        assert_eq!(item.links[1].kind, "mail");
        assert_eq!(
            item.links[2].label, "Link",
            "a missing label falls back rather than dropping the link"
        );
        assert_eq!(item.links[2].kind, "source");
    }

    /// A source that already lists its own page must not get it twice.
    #[test]
    fn a_duplicated_source_url_is_listed_once() {
        let mut rich = entry();
        rich.payload = json!({
            "url": "https://lu.ma/x",
            "links": [{ "label": "Event page", "kind": "source", "url": "https://lu.ma/x" }]
        });
        assert_eq!(from_entry(&rich).links.len(), 1);
    }

    #[test]
    fn notes_are_the_content_and_about_is_the_summary() {
        let mut rich = entry();
        rich.payload = json!({ "about": "A theme park in Brühl." });
        let item = from_entry(&rich);
        assert_eq!(item.summary.as_deref(), Some("A theme park in Brühl."));
        assert_eq!(
            item.content.as_deref(),
            Some("Dated ticket, valid only on 10.08.")
        );
        assert_eq!(item.content_status, "full");

        // ...and an entry with neither says so rather than showing an empty box.
        let mut bare = entry();
        bare.notes = None;
        let item = from_entry(&bare);
        assert!(item.summary.is_none());
        assert_eq!(item.content_status, "none");
    }

    /// Which actions are honest depends on these two. An entry imported from
    /// Google must not offer to export back to Google, and a rhythm instance is
    /// never exported on its own — a reader cannot know either without them.
    #[test]
    fn the_extension_carries_what_a_reader_needs_to_pick_its_actions() {
        let plain = from_entry(&entry()).calendar.unwrap();
        assert_eq!(plain.entry_source, "manual");
        assert!(plain.rhythm_id.is_none());

        let mut imported = entry();
        imported.source = "google".into();
        imported.rhythm_id = Some("cal:rhythm:weekly".into());
        let extension = from_entry(&imported).calendar.unwrap();
        assert_eq!(extension.entry_source, "google");
        assert_eq!(extension.rhythm_id.as_deref(), Some("cal:rhythm:weekly"));
    }

    #[test]
    fn a_provider_contribution_records_its_origin() {
        let mut promoted = entry();
        promoted.source = "luma".into();
        promoted.external_id = Some("evt-abc".into());
        promoted.payload = json!({ "city": "München", "venue": "Theresienstraße 6" });
        let origins = from_entry(&promoted).origins;
        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0].source_id, "luma");
        assert_eq!(
            origins[0].source_ref, "evt-abc",
            "the provider's own dedupe key"
        );
        assert_eq!(origins[0].label.as_deref(), Some("Theresienstraße 6"));

        // A hand-made entry has no provider, so it claims none.
        assert!(from_entry(&entry()).origins.is_empty());
    }

    /// Unix seconds sort before every ISO string ever written, so a reader
    /// merging calendar with feed would put the whole calendar first.
    #[test]
    fn created_at_is_converted_to_iso_so_sources_sort_together() {
        assert_eq!(iso_utc("1785414018"), "2026-07-30T12:20:18Z");
        assert_eq!(iso_utc("0"), "1970-01-01T00:00:00Z");
        // Anything already ISO, or unparseable, passes through untouched rather
        // than becoming a fabricated timestamp.
        assert_eq!(iso_utc("2026-08-03T21:25:56Z"), "2026-08-03T21:25:56Z");
        assert!(from_entry(&entry()).created_at.ends_with('Z'));
    }

    /// The operator's schedule is Mine whatever the event is, and that must
    /// never come back cloud-eligible.
    #[test]
    fn a_calendar_item_is_mine_and_not_cloud_eligible() {
        let item = from_entry(&entry());
        assert_eq!(item.data_class.value, "c1");
        assert_eq!(item.data_class.label, "Mine");
        assert_eq!(
            item.processing_policy.cloud_handling,
            "pseudonymization_required"
        );
        assert_eq!(item.cloud_processing.provider_calls, 0);
    }

    #[test]
    fn an_all_day_entry_keeps_its_exclusive_end() {
        let mut all_day = entry();
        all_day.all_day = true;
        all_day.starts_at = "2026-09-15".into();
        all_day.ends_at = "2026-09-16".into();
        let item = from_entry(&all_day);
        assert_eq!(item.day, "2026-09-15");
        let extension = item.calendar.unwrap();
        assert!(extension.all_day);
        assert_eq!(
            extension.ends_at, "2026-09-16",
            "carried verbatim, never re-derived"
        );
    }
}
