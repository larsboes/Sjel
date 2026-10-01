//! Promotes saved Luma events into `capabilities/calendar` entries.
//!
//! This is the last leg of calendar's Phase A: discovery (scouting) →
//! availability annotation (calendar). It reads opportunities the operator
//! has already triaged to `saved`, and upserts each one through calendar's
//! `PUT /api/entries/external` with `source = "luma"` and the stable Luma
//! event id as `external_id`. Calendar's partial unique index on
//! `(source, external_id)` makes a repeat run update the same row instead of
//! adding a second one.
//!
//! Two boundaries this deliberately keeps:
//!
//! * **Scouting does not write calendar's database.** Everything goes through
//!   the HTTP contract, so calendar keeps ownership of its schema and of what
//!   a valid entry is (it rejects our payload if we get it wrong).
//! * **No date is guessed.** Calendar's README makes this a rule; an event
//!   missing a usable start or end, or carrying a non-UTC instant, is
//!   reported as skipped and left alone rather than promoted with an invented
//!   time. The operator sees the reason and can fix the source.
//!
//! `status = "saved"` is the promotion trigger because that is already the
//! operator's explicit "yes, this one" — reusing it avoids inventing a second
//! decision state alongside the one `store.rs` guarantees survives a refetch.

use serde::Serialize;
use serde_json::{json, Value};

use crate::event_route::{classify_ranked, EventRoute, EventRouteKind};
use crate::localtime::HomeTimezone;
use crate::store::{RankedRow, Store};

/// Calendar's default loopback address. Overridable via `scouting.json`'s
/// `calendar_base_url` or `--calendar-url`; the default matches
/// `capabilities/calendar`'s `SJEL_CALENDAR_PORT` default of 8087.
pub const DEFAULT_CALENDAR_BASE_URL: &str = "http://127.0.0.1:8087";

/// Calendar kind for a promoted event. Calendar treats kinds as open data,
/// but `event` is the one its correlation layer already understands.
const ENTRY_KIND: &str = "event";
const ENTRY_SOURCE: &str = "luma";

#[derive(Debug, Clone, Serialize)]
pub struct Promoted {
    pub opportunity_id: String,
    pub external_id: String,
    pub entry_id: String,
    pub title: String,
    pub starts_at: String,
    pub ends_at: String,
    pub event_route: EventRouteKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct Routed {
    pub opportunity_id: String,
    pub title: String,
    pub event_route: EventRouteKind,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Skipped {
    pub opportunity_id: String,
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Default, Serialize)]
pub struct PromotionReport {
    pub considered: usize,
    pub promoted: Vec<Promoted>,
    pub routed: Vec<Routed>,
    pub skipped: Vec<Skipped>,
    pub dry_run: bool,
}

/// Strips the `evt:luma:` namespace scouting adds, leaving Luma's own stable
/// event id. That bare id is what `external_id` means to calendar — the
/// provider's key, not ours — and it is what a second provider path (an ICS
/// import of the same event) would also produce.
fn luma_event_id(opportunity_id: &str) -> Option<&str> {
    opportunity_id
        .strip_prefix("evt:luma:")
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// Inert evidence snapshot. Deliberately carries no wall-clock "promoted at"
/// stamp: the payload has to be byte-identical across runs for a repeat
/// promotion to be a genuine no-op rather than a churning update.
///
/// It also carries **no score**. Ranking is how scouting decides what to *offer*
/// from an unbounded stream; once an opportunity is on the calendar the operator
/// has taken a position on it, and `commitment` is the only axis that means
/// anything. Copying the score across produced entries stamped `score: 0.0,
/// matched_focus: "Scholarship Profile"` — a verdict on an event the operator
/// had already accepted, rendered from a profile label that turned out to carry
/// no signal at all. `content-item-v2` now forbids it for `source = calendar`;
/// this is the write side of the same rule. The opportunity id stays, so the
/// score is one lookup away for anything that genuinely wants it.
fn evidence_payload(row: &RankedRow, tz: &HomeTimezone, event_route: &EventRoute) -> Value {
    json!({
        "promoted_from": "scouting",
        "opportunity_id": row.id,
        "url": row.url,
        "city": row.city,
        "starts_at_utc": row.starts_at,
        "ends_at_utc": row.ends_at,
        "home_timezone": tz.name(),
        "event_route": event_route.route,
        "event_route_basis": event_route.basis,
        "event_route_reason": event_route.reason,
        "distance_km": event_route.distance_km,
    })
}

fn build_entry(
    row: &RankedRow,
    tz: &HomeTimezone,
    event_route: &EventRoute,
) -> Result<(String, Value), String> {
    let external_id = luma_event_id(&row.id)
        .ok_or_else(|| format!("id '{}' is not a luma opportunity id", row.id))?
        .to_string();

    if row.starts_at.trim().is_empty() {
        return Err("no start time on the opportunity".into());
    }
    if row.ends_at.trim().is_empty() {
        return Err("no end time on the opportunity".into());
    }
    let starts_at = tz
        .wall_time(&row.starts_at)
        .map_err(|e| format!("start: {e}"))?;
    let ends_at = tz
        .wall_time(&row.ends_at)
        .map_err(|e| format!("end: {e}"))?;
    if ends_at <= starts_at {
        return Err(format!(
            "end {ends_at} is not after start {starts_at} (calendar ends are exclusive)"
        ));
    }

    let location = Some(row.location.trim())
        .filter(|l| !l.is_empty())
        .or(Some(row.city.trim()).filter(|c| !c.is_empty()))
        .map(str::to_string);

    let body = json!({
        "kind": ENTRY_KIND,
        "title": row.title,
        "starts_at": starts_at,
        "ends_at": ends_at,
        "all_day": false,
        "location": location,
        "notes": Value::Null,
        "source": ENTRY_SOURCE,
        "external_id": external_id,
        "payload": evidence_payload(row, tz, event_route),
    });
    Ok((external_id, body))
}

fn belongs_in_day_calendar(event_route: &EventRoute) -> bool {
    matches!(
        event_route.route,
        EventRouteKind::Local | EventRouteKind::Online
    )
}

/// Reads saved Luma opportunities and upserts each into calendar.
///
/// `limit` bounds the store read, not the promotion — the backlog query is
/// score-ordered, so this is the same window the ranked views show.
pub fn promote_saved_luma(
    store: &Store,
    calendar_base_url: &str,
    tz: &HomeTimezone,
    limit: usize,
    dry_run: bool,
    geo: Option<&crate::config::GeoPolicy>,
) -> Result<PromotionReport, Box<dyn std::error::Error>> {
    let rows = store.list_top(limit, false)?;
    let candidates: Vec<RankedRow> = rows
        .into_iter()
        .filter(|r| r.source == ENTRY_SOURCE && r.status == "saved")
        .collect();

    let mut report = PromotionReport {
        considered: candidates.len(),
        dry_run,
        ..Default::default()
    };

    // A local calendar POST, on the 20s calendar itself uses for its own
    // outbound calls. It had no timeout before.
    let client = sjel_http::client(
        sjel_http::Purpose::new("scouting-calendar"),
        std::time::Duration::from_secs(20),
    )?;
    let url = format!(
        "{}/api/entries/external",
        calendar_base_url.trim_end_matches('/')
    );

    for row in candidates {
        let Some(event_route) = classify_ranked(&row, geo) else {
            report.skipped.push(Skipped {
                opportunity_id: row.id,
                title: row.title,
                reason: format!(
                    "luma opportunity has non-event type {}",
                    row.opportunity_type
                ),
            });
            continue;
        };
        if !belongs_in_day_calendar(&event_route) {
            report.routed.push(Routed {
                opportunity_id: row.id,
                title: row.title,
                event_route: event_route.route,
                reason: event_route.reason,
            });
            continue;
        }
        let (external_id, body) = match build_entry(&row, tz, &event_route) {
            Ok(pair) => pair,
            Err(reason) => {
                report.skipped.push(Skipped {
                    opportunity_id: row.id,
                    title: row.title,
                    reason,
                });
                continue;
            }
        };

        if dry_run {
            report.promoted.push(Promoted {
                opportunity_id: row.id,
                external_id,
                entry_id: "(dry-run)".into(),
                title: row.title,
                starts_at: body["starts_at"].as_str().unwrap_or_default().into(),
                ends_at: body["ends_at"].as_str().unwrap_or_default().into(),
                event_route: event_route.route,
            });
            continue;
        }

        let request =
            sjel_server::InboundAuth::with_loopback_auth(client.put(&url).json(&body), &url);
        let response = request.send();
        match response {
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().unwrap_or_default();
                if !status.is_success() {
                    let snippet: String = text.chars().take(200).collect();
                    report.skipped.push(Skipped {
                        opportunity_id: row.id,
                        title: row.title,
                        reason: format!("calendar rejected the entry: HTTP {status}: {snippet}"),
                    });
                    continue;
                }
                let entry: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                report.promoted.push(Promoted {
                    opportunity_id: row.id,
                    external_id,
                    entry_id: entry["id"].as_str().unwrap_or("(no id in response)").into(),
                    title: row.title,
                    starts_at: entry["starts_at"].as_str().unwrap_or_default().into(),
                    ends_at: entry["ends_at"].as_str().unwrap_or_default().into(),
                    event_route: event_route.route,
                });
            }
            Err(error) => {
                report.skipped.push(Skipped {
                    opportunity_id: row.id,
                    title: row.title,
                    reason: format!("calendar unreachable at {url}: {error}"),
                });
            }
        }
    }

    Ok(report)
}

pub fn print_report(report: &PromotionReport, calendar_base_url: &str, tz: &HomeTimezone) {
    let mode = if report.dry_run {
        " (dry run — nothing written)"
    } else {
        ""
    };
    println!(
        "  calendar   : {calendar_base_url} · home timezone {}{mode}",
        tz.name()
    );
    println!(
        "  considered : {} saved luma opportunit{}\n",
        report.considered,
        if report.considered == 1 { "y" } else { "ies" }
    );

    if report.promoted.is_empty() && report.routed.is_empty() && report.skipped.is_empty() {
        println!("  nothing to promote — save a luma opportunity first (scout --save <id>)");
        return;
    }

    for p in &report.promoted {
        println!(
            "  promoted  {} — {} ({:?})",
            p.external_id, p.title, p.event_route
        );
        println!(
            "            {} → {}  entry {}",
            p.starts_at, p.ends_at, p.entry_id
        );
    }
    for routed in &report.routed {
        println!(
            "  routed    {} — {} ({:?})",
            routed.opportunity_id, routed.title, routed.event_route
        );
        println!("            {}", routed.reason);
    }
    for s in &report.skipped {
        println!("  skipped   {} — {}", s.opportunity_id, s.title);
        println!("            {}", s.reason);
    }
    println!(
        "\n  {} promoted, {} routed, {} skipped",
        report.promoted.len(),
        report.routed.len(),
        report.skipped.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, starts_at: &str, ends_at: &str) -> RankedRow {
        RankedRow {
            id: id.into(),
            opportunity_type: "event".into(),
            source: "luma".into(),
            title: "Berlin | Claude in the Wild".into(),
            city: "Berlin".into(),
            starts_at: starts_at.into(),
            ends_at: ends_at.into(),
            location: "Factory Berlin Mitte".into(),
            score: 0.42,
            matched_focus: "AI engineering".into(),
            rationale: "cosine 0.42".into(),
            url: "https://lu.ma/claude-fq5h".into(),
            vault_link: None,
            status: "saved".into(),
            country_code: Some("Germany".into()),
            latitude: Some(52.52),
            longitude: Some(13.405),
            raw: None,
        }
    }

    fn berlin() -> HomeTimezone {
        HomeTimezone::parse("Europe/Berlin").unwrap()
    }

    fn local_route() -> EventRoute {
        EventRoute {
            route: EventRouteKind::Local,
            basis: crate::event_route::EventRouteBasis::Coordinates,
            reason: "fixture is local".into(),
            distance_km: Some(10.0),
        }
    }

    #[test]
    fn only_local_and_online_routes_belong_in_the_day_calendar() {
        let mut route = local_route();
        assert!(belongs_in_day_calendar(&route));

        route.route = EventRouteKind::Online;
        assert!(belongs_in_day_calendar(&route));

        route.route = EventRouteKind::TravelCandidate;
        assert!(!belongs_in_day_calendar(&route));

        route.route = EventRouteKind::Unresolved;
        assert!(!belongs_in_day_calendar(&route));
    }

    #[test]
    fn external_id_is_lumas_own_id_not_the_namespaced_one() {
        assert_eq!(
            luma_event_id("evt:luma:evt-E8mj424DVKBXFb4"),
            Some("evt-E8mj424DVKBXFb4")
        );
        assert_eq!(luma_event_id("evt:obsidian:something"), None);
        assert_eq!(luma_event_id("evt:luma:"), None);
    }

    #[test]
    fn builds_a_calendar_entry_in_local_wall_time() {
        let (external_id, body) = build_entry(
            &row(
                "evt:luma:evt-E8mj424DVKBXFb4",
                "2026-07-30T16:00:00.000Z",
                "2026-07-30T19:00:00.000Z",
            ),
            &berlin(),
            &local_route(),
        )
        .unwrap();
        assert_eq!(external_id, "evt-E8mj424DVKBXFb4");
        assert_eq!(body["kind"], "event");
        assert_eq!(body["source"], "luma");
        assert_eq!(body["all_day"], false);
        assert_eq!(body["starts_at"], "2026-07-30T18:00:00");
        assert_eq!(body["ends_at"], "2026-07-30T21:00:00");
        assert_eq!(body["location"], "Factory Berlin Mitte");
        assert_eq!(body["payload"]["starts_at_utc"], "2026-07-30T16:00:00.000Z");
        assert_eq!(body["payload"]["home_timezone"], "Europe/Berlin");
    }

    /// A promoted event carries evidence, never a verdict.
    ///
    /// The regression: entries used to arrive stamped `score: 0.0` and
    /// `matched_focus: "Scholarship Profile"` — a judgement of an event the
    /// operator had already put on their calendar, from a profile label that
    /// carried no signal. `content-item-v2` forbids ranking on a calendar item;
    /// this is the write side of the same rule. `row()` deliberately sets all
    /// three fields, so a future re-add fails here.
    #[test]
    fn a_promoted_event_carries_no_ranking_fields() {
        let (_, body) = build_entry(
            &row(
                "evt:luma:evt-E8mj424DVKBXFb4",
                "2026-07-30T16:00:00.000Z",
                "2026-07-30T19:00:00.000Z",
            ),
            &berlin(),
            &local_route(),
        )
        .unwrap();
        let payload = &body["payload"];
        for field in ["score", "matched_focus", "rationale"] {
            assert!(
                payload.get(field).is_none(),
                "{field} is a ranking verdict and must not ride along on a calendar entry"
            );
        }
        // The link back is what survives, so the score stays one lookup away.
        assert_eq!(payload["opportunity_id"], "evt:luma:evt-E8mj424DVKBXFb4");
    }

    #[test]
    fn the_request_body_is_byte_stable_across_runs() {
        // Idempotency is only real if a repeat promotion sends the same bytes;
        // a timestamp in the payload would make every run an update.
        let r = row(
            "evt:luma:evt-A",
            "2026-07-30T16:00:00.000Z",
            "2026-07-30T19:00:00.000Z",
        );
        let first = build_entry(&r, &berlin(), &local_route()).unwrap().1;
        let second = build_entry(&r, &berlin(), &local_route()).unwrap().1;
        assert_eq!(first, second);
    }

    #[test]
    fn refuses_an_event_with_no_start() {
        let err = build_entry(
            &row("evt:luma:evt-A", "", "2026-07-30T19:00:00.000Z"),
            &berlin(),
            &local_route(),
        )
        .unwrap_err();
        assert!(err.contains("no start time"), "got: {err}");
    }

    #[test]
    fn refuses_an_event_with_no_end() {
        let err = build_entry(
            &row("evt:luma:evt-A", "2026-07-30T16:00:00.000Z", ""),
            &berlin(),
            &local_route(),
        )
        .unwrap_err();
        assert!(err.contains("no end time"), "got: {err}");
    }

    #[test]
    fn refuses_a_non_utc_instant_rather_than_assuming_a_zone() {
        let err = build_entry(
            &row(
                "evt:luma:evt-A",
                "2026-07-30T16:00:00",
                "2026-07-30T19:00:00",
            ),
            &berlin(),
            &local_route(),
        )
        .unwrap_err();
        assert!(err.contains("explicit UTC instant"), "got: {err}");
    }

    #[test]
    fn refuses_a_zero_length_or_inverted_window() {
        let err = build_entry(
            &row(
                "evt:luma:evt-A",
                "2026-07-30T19:00:00.000Z",
                "2026-07-30T16:00:00.000Z",
            ),
            &berlin(),
            &local_route(),
        )
        .unwrap_err();
        assert!(err.contains("not after"), "got: {err}");
    }

    #[test]
    fn falls_back_to_city_when_there_is_no_street_address() {
        let mut r = row(
            "evt:luma:evt-A",
            "2026-07-30T16:00:00.000Z",
            "2026-07-30T19:00:00.000Z",
        );
        r.location = "  ".into();
        let (_, body) = build_entry(&r, &berlin(), &local_route()).unwrap();
        assert_eq!(body["location"], "Berlin");
    }
}
