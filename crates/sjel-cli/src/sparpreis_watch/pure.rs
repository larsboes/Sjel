//! The pure half of `tools/sparpreis-watch`: watch identity, watch discovery, history folding and
//! the drop rule, with no I/O.
//!
//! These are the helpers `tools/sparpreis-watch.ts` exported, ported one for one. Its
//! `tools/sparpreis-watch.test.ts` is the case list at the bottom of this file; the two cases it
//! held for `authorizedLoopbackRequest` stay in Rust, where the rule lives
//! (`sjel-server::InboundAuth::with_loopback_auth`, tested by
//! `shared_credentials_are_scoped_to_loopback_endpoints`). The manifest-port cases that file also
//! held moved with the reader, to `crate::paths::port_in_manifest`.

use serde_json::Value;

/// A stage status still worth re-pricing. `booked` and `completed` are not.
const WATCHED_STATUSES: [&str; 3] = ["open", "planning", "option_selected"];

/// A stage carries a date and no time, so its search starts here. An estimate of when a traveller
/// leaves, not a measurement; transit returns the next few journeys from it.
pub const STAGE_DEPARTURE: &str = "07:00:00";

/// One watched rail search: a station pair, a departure time and the fare context that changes
/// what the traveller would pay.
#[derive(Debug, Clone, PartialEq)]
pub struct RailWatch {
    pub plan_id: String,
    pub from: String,
    pub to: String,
    pub time: String,
    pub bc: Option<i64>,
    pub d_ticket: bool,
    pub first_class: bool,
    /// Set when the watch comes from a stage rather than from an option_set.
    pub stage_id: Option<String>,
}

/// A stable identity for one watched search, so observations land on one item.
pub fn watch_key(watch: &RailWatch) -> String {
    let mut fare: Vec<String> = Vec::new();
    // Falsy in the TypeScript, so a BahnCard class of 0 is absent rather than `bc0`.
    if let Some(bc) = watch.bc.filter(|bc| *bc != 0) {
        fare.push(format!("bc{bc}"));
    }
    if watch.d_ticket {
        fare.push("dt".to_owned());
    }
    if watch.first_class {
        fare.push("k1".to_owned());
    }
    let fare = fare.join("-");
    format!(
        "{}:{}:{}{}",
        watch.from,
        watch.to,
        watch.time,
        if fare.is_empty() {
            String::new()
        } else {
            format!(":{fare}")
        }
    )
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Place {
    #[serde(default)]
    pub name: Option<String>,
}

/// A plan stage, as trips' `/api/plans/{id}` returns one.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Stage {
    pub id: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub transport_modes: Option<Vec<String>>,
    #[serde(default)]
    pub origin: Option<Place>,
    #[serde(default)]
    pub destination: Option<Place>,
}

pub fn train_stages(stages: &[Stage]) -> Vec<&Stage> {
    stages
        .iter()
        .filter(|stage| {
            stage
                .transport_modes
                .as_ref()
                .is_some_and(|modes| modes.iter().any(|m| m == "train"))
        })
        .collect()
}

/// One watch per unbooked, dated, upcoming train stage, searched by place name. Transit resolves a
/// name to a station and answers 400 rather than guess.
pub fn stage_watches_of(plan_id: &str, stages: &[Stage], today: &str) -> Vec<RailWatch> {
    let mut watches = Vec::new();
    for stage in train_stages(stages) {
        if !stage
            .status
            .as_deref()
            .is_some_and(|status| WATCHED_STATUSES.contains(&status))
        {
            continue;
        }
        let Some(date) = stage.date.as_deref().filter(|d| *d >= today) else {
            continue;
        };
        let from = stage
            .origin
            .as_ref()
            .and_then(|p| p.name.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let to = stage
            .destination
            .as_ref()
            .and_then(|p| p.name.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        watches.push(RailWatch {
            plan_id: plan_id.to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            time: format!("{date}T{STAGE_DEPARTURE}"),
            bc: None,
            d_ticket: false,
            first_class: false,
            stage_id: Some(stage.id.clone()),
        });
    }
    watches
}

/// The rail searches a plan already records: option_set items whose query names two numeric
/// station ids and a departure time. Accommodation option_sets (coordinate queries) and anything
/// else fall through the numeric test.
pub fn rail_watches_of(plan_id: &str, items: &[Value]) -> Vec<RailWatch> {
    let mut watches = Vec::new();
    for item in items {
        if item.get("item_type").and_then(Value::as_str) != Some("option_set") {
            continue;
        }
        let external_id = item
            .get("external_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        // This job's own observations are option_sets too; re-watching them would multiply the
        // watch list every run.
        if external_id.starts_with("sparpreis-watch:") {
            continue;
        }
        let Some(query) = item.get("payload").and_then(|p| p.get("query")) else {
            continue;
        };
        let string = |key: &str| query.get(key).and_then(Value::as_str);
        let (Some(from), Some(to)) = (string("from"), string("to")) else {
            continue;
        };
        if !is_station_id(from) || !is_station_id(to) {
            continue;
        }
        let Some(time) = string("time").filter(|t| t.contains('T')) else {
            continue;
        };
        watches.push(RailWatch {
            plan_id: plan_id.to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            time: time.to_owned(),
            bc: query.get("bc").and_then(Value::as_i64),
            d_ticket: query.get("d_ticket").and_then(Value::as_bool) == Some(true),
            first_class: query.get("first_class").and_then(Value::as_bool) == Some(true),
            stage_id: None,
        });
    }
    watches
}

/// `^\d+$` — a station id, not a place name and not a coordinate.
fn is_station_id(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// The identity a station pair gets on one day, for matching option_sets to stages.
pub fn leg_key(from_eva: &str, to_eva: &str, time: &str) -> String {
    let day = time.get(..10).unwrap_or(time);
    format!("{from_eva}:{to_eva}:{day}")
}

/// Whether an option_set watch still describes a leg of the plan.
///
/// `stage_legs` holds the station pairs the unbooked stages resolved to in this run. A plan with
/// no train stage keeps every option_set watch, because there is nothing to compare it with. A
/// plan whose train stages are all booked watches nothing.
pub fn still_planned(watch: &RailWatch, has_train_stages: bool, stage_legs: &[String]) -> bool {
    if !has_train_stages {
        return true;
    }
    stage_legs.contains(&leg_key(&watch.from, &watch.to, &watch.time))
}

/// One day's prices for a watch.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub day: String,
    pub prices: Vec<f64>,
}

/// A per-day observation item written before 2026-09-25, with the day and prices read out of it.
#[derive(Debug, Clone)]
pub struct LegacyObservation {
    pub id: Option<String>,
    pub external_id: Option<String>,
    /// The item's payload, kept whole so `consolidate` can replay its `query` and `options`.
    pub payload: Value,
    pub day: String,
    pub prices: Vec<f64>,
}

/// `sparpreis-watch:<key>:<YYYY-MM-DD>`, split at the LAST colon so a key may itself carry one.
fn split_legacy_id(external_id: &str) -> Option<(&str, &str)> {
    let rest = external_id.strip_prefix("sparpreis-watch:")?;
    let (key, day) = rest.rsplit_once(':')?;
    if key.is_empty() || !is_iso_day(day) {
        return None;
    }
    Some((key, day))
}

fn is_iso_day(day: &str) -> bool {
    let b = day.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..].iter().all(u8::is_ascii_digit)
}

fn prices_of(options: Option<&Value>) -> Vec<f64> {
    options
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|o| o.get("total_price").and_then(Value::as_f64))
                .collect()
        })
        .unwrap_or_default()
}

/// The per-day observation items written before 2026-09-25, grouped by watch key, in encounter
/// order (a first-seen list rather than a sorted map, so the TypeScript's iteration order holds).
pub fn legacy_observations(items: &[Value]) -> Vec<(String, Vec<LegacyObservation>)> {
    let mut groups: Vec<(String, Vec<LegacyObservation>)> = Vec::new();
    for item in items {
        if item.get("item_type").and_then(Value::as_str) != Some("option_set") {
            continue;
        }
        let external_id = item
            .get("external_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let Some((key, day)) = split_legacy_id(external_id) else {
            continue;
        };
        let payload = item.get("payload").cloned().unwrap_or(Value::Null);
        let observation = LegacyObservation {
            id: item.get("id").and_then(Value::as_str).map(str::to_owned),
            external_id: item
                .get("external_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            prices: prices_of(payload.get("options")),
            payload,
            day: day.to_owned(),
        };
        match groups.iter_mut().find(|(k, _)| k == key) {
            Some((_, list)) => list.push(observation),
            None => groups.push((key.to_owned(), vec![observation])),
        }
    }
    groups
}

/// Every observation recorded for a watch: the single item's history plus any legacy per-day items
/// not folded in yet. One entry per day, the later write winning, ordered by day.
pub fn history_of(items: &[Value], key: &str) -> Vec<Observation> {
    let mut by_day: Vec<(String, Vec<f64>)> = Vec::new();
    for obs in legacy_observations(items)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, list)| list)
        .unwrap_or_default()
    {
        set_day(&mut by_day, &obs.day, obs.prices);
    }
    let single_id = format!("sparpreis-watch:{key}");
    let single = items.iter().find(|item| {
        item.get("item_type").and_then(Value::as_str) == Some("option_set")
            && item.get("external_id").and_then(Value::as_str) == Some(single_id.as_str())
    });
    if let Some(entries) = single
        .and_then(|s| s.get("payload"))
        .and_then(|p| p.get("history"))
        .and_then(Value::as_array)
    {
        for entry in entries {
            let Some(day) = entry.get("day").and_then(Value::as_str) else {
                continue;
            };
            let Some(prices) = entry.get("prices").and_then(Value::as_array) else {
                continue;
            };
            set_day(
                &mut by_day,
                day,
                prices.iter().filter_map(Value::as_f64).collect(),
            );
        }
    }
    by_day.sort_by(|a, b| a.0.cmp(&b.0));
    by_day
        .into_iter()
        .map(|(day, prices)| Observation { day, prices })
        .collect()
}

/// One day's prices, replacing an earlier entry for the same day.
fn set_day(by_day: &mut Vec<(String, Vec<f64>)>, day: &str, prices: Vec<f64>) {
    match by_day.iter_mut().find(|(d, _)| d == day) {
        Some((_, existing)) => *existing = prices,
        None => by_day.push((day.to_owned(), prices)),
    }
}

/// The lowest fare ever observed for a watch, or `None` before its first observation.
pub fn lowest_seen(history: &[Observation]) -> Option<f64> {
    history
        .iter()
        .flat_map(|o| o.prices.iter().copied())
        .reduce(f64::min)
}

/// A new low is a real one, not float noise and not a first observation.
pub fn dropped(previous_low: Option<f64>, current: f64) -> bool {
    previous_low.is_some_and(|low| current < low - 0.01)
}

/// Today's observation replaces an earlier one from the same day.
pub fn with_observation(history: Vec<Observation>, today: Observation) -> Vec<Observation> {
    let mut out: Vec<Observation> = history.into_iter().filter(|o| o.day != today.day).collect();
    out.push(today);
    out.sort_by(|a, b| a.day.cmp(&b.day));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_rail_option_set_becomes_a_watch_with_its_fare_context() {
        let item = json!({
            "item_type": "option_set",
            "external_id": "split:8000207:8000105",
            "payload": { "query": { "from": "8000207", "to": "8000105", "time": "2026-09-01T08:00:00", "bc": 25 } },
        });
        assert_eq!(
            rail_watches_of("p1", &[item]),
            vec![RailWatch {
                plan_id: "p1".to_owned(),
                from: "8000207".to_owned(),
                to: "8000105".to_owned(),
                time: "2026-09-01T08:00:00".to_owned(),
                bc: Some(25),
                d_ticket: false,
                first_class: false,
                stage_id: None,
            }]
        );
    }

    #[test]
    fn accommodation_queries_and_the_watchs_own_observations_are_not_watches() {
        let accommodation = json!({
            "item_type": "option_set",
            "external_id": "booking.com:berlin",
            "payload": { "query": { "from": "coordinate-anchor", "to": "52.52,13.40", "check_in": "2026-10-07" } },
        });
        let own_observation = json!({
            "item_type": "option_set",
            "external_id": "sparpreis-watch:8000207:8000105:2026-09-01T08:00:00:2026-08-11",
            "payload": { "query": { "from": "8000207", "to": "8000105", "time": "2026-09-01T08:00:00" } },
        });
        let stay = json!({ "item_type": "stay", "external_id": "booking.com:1", "payload": {} });
        assert!(rail_watches_of("p1", &[accommodation, own_observation, stay]).is_empty());
    }

    fn stage(over: Value) -> Stage {
        let mut base = json!({
            "id": "stage:a",
            "date": "2026-10-07",
            "status": "planning",
            "transport_modes": ["train"],
            "origin": { "name": "Bonn" },
            "destination": { "name": "Stuttgart" },
        });
        for (k, v) in over.as_object().expect("stage overrides are an object") {
            base[k] = v.clone();
        }
        serde_json::from_value(base).expect("stage parses")
    }

    #[test]
    fn an_unbooked_upcoming_train_stage_is_watched_by_place_name_on_its_date() {
        assert_eq!(
            stage_watches_of("p1", &[stage(json!({}))], "2026-09-25"),
            vec![RailWatch {
                plan_id: "p1".to_owned(),
                from: "Bonn".to_owned(),
                to: "Stuttgart".to_owned(),
                time: "2026-10-07T07:00:00".to_owned(),
                bc: None,
                d_ticket: false,
                first_class: false,
                stage_id: Some("stage:a".to_owned()),
            }]
        );
    }

    #[test]
    fn booked_completed_past_undated_and_non_train_stages_are_not_watched() {
        let stages = [
            stage(json!({ "status": "booked" })),
            stage(json!({ "status": "completed" })),
            stage(json!({ "date": "2026-09-01" })),
            stage(json!({ "date": null })),
            stage(json!({ "transport_modes": ["flight"] })),
            stage(json!({ "origin": {} })),
        ];
        assert!(stage_watches_of("p1", &stages, "2026-09-25").is_empty());
    }

    /// The Berlin plan, 2026-09-25: its option_set searched Bonn -> Berlin while its stages had
    /// become Bonn -> Stuttgart -> Berlin.
    #[test]
    fn an_option_set_whose_route_no_stage_resolved_to_is_not_watched() {
        let watch = RailWatch {
            plan_id: "p".to_owned(),
            from: "8000044".to_owned(),
            to: "8011160".to_owned(),
            time: "2026-10-07T08:00:00".to_owned(),
            bc: None,
            d_ticket: false,
            first_class: false,
            stage_id: None,
        };
        assert!(!still_planned(
            &watch,
            true,
            &["8000044:8000096:2026-10-07".to_owned()]
        ));
        assert!(still_planned(
            &watch,
            true,
            &["8000044:8011160:2026-10-07".to_owned()]
        ));
    }

    #[test]
    fn a_plan_with_no_train_stage_keeps_every_option_set_watch() {
        let watch = RailWatch {
            plan_id: "p".to_owned(),
            from: "8000044".to_owned(),
            to: "8011160".to_owned(),
            time: "2026-10-07T08:00:00".to_owned(),
            bc: None,
            d_ticket: false,
            first_class: false,
            stage_id: None,
        };
        assert!(still_planned(&watch, false, &[]));
    }

    const KEY: &str = "8000207:8000105:2026-09-01T08:00:00:bc25";

    fn legacy(day: &str, prices: &[Value]) -> Value {
        json!({
            "id": format!("i-{day}"),
            "item_type": "option_set",
            "external_id": format!("sparpreis-watch:{KEY}:{day}"),
            "payload": { "options": prices.iter().map(|p| json!({ "total_price": p })).collect::<Vec<_>>() },
        })
    }

    #[test]
    fn per_day_items_group_under_their_watch_key() {
        let groups = legacy_observations(&[
            legacy("2026-08-10", &[json!(29.99)]),
            legacy("2026-08-11", &[json!(35.99)]),
        ]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, KEY);
        let days: Vec<&str> = groups[0].1.iter().map(|o| o.day.as_str()).collect();
        assert_eq!(days, vec!["2026-08-10", "2026-08-11"]);
    }

    #[test]
    fn the_single_item_and_legacy_items_merge_one_entry_per_day() {
        let single = json!({
            "item_type": "option_set",
            "external_id": format!("sparpreis-watch:{KEY}"),
            "payload": { "history": [
                { "day": "2026-08-11", "prices": [33.0] },
                { "day": "2026-08-12", "prices": [40.0] },
            ] },
        });
        let history = history_of(
            &[
                legacy("2026-08-10", &[json!(29.99), json!(null)]),
                legacy("2026-08-11", &[json!(35.99)]),
                single,
            ],
            KEY,
        );
        assert_eq!(
            history,
            vec![
                Observation {
                    day: "2026-08-10".to_owned(),
                    prices: vec![29.99]
                },
                Observation {
                    day: "2026-08-11".to_owned(),
                    prices: vec![33.0]
                },
                Observation {
                    day: "2026-08-12".to_owned(),
                    prices: vec![40.0]
                },
            ]
        );
    }

    #[test]
    fn a_different_watch_key_does_not_bleed_in() {
        let other = json!({
            "id": "i",
            "item_type": "option_set",
            "external_id": "sparpreis-watch:8000000:8000001:2026-09-01T08:00:00:2026-08-11",
            "payload": { "options": [{ "total_price": 9.99 }] },
        });
        assert!(history_of(&[other], KEY).is_empty());
    }

    /// €67.99 -> €47.99 was reported as a drop on 2026-09-23 although €39.99 had been seen on
    /// 2026-08-15. The comparison is against the lowest, not the latest.
    #[test]
    fn a_new_low_is_measured_against_every_earlier_observation() {
        let history = vec![
            Observation {
                day: "2026-08-15".to_owned(),
                prices: vec![39.99],
            },
            Observation {
                day: "2026-09-22".to_owned(),
                prices: vec![67.99],
            },
        ];
        assert_eq!(lowest_seen(&history), Some(39.99));
        assert!(!dropped(lowest_seen(&history), 47.99));
        assert!(dropped(lowest_seen(&history), 34.99));
        assert_eq!(lowest_seen(&[]), None);
    }

    #[test]
    fn todays_observation_replaces_an_earlier_one_from_the_same_day() {
        let history = vec![Observation {
            day: "2026-09-25".to_owned(),
            prices: vec![50.0],
        }];
        assert_eq!(
            with_observation(
                history,
                Observation {
                    day: "2026-09-25".to_owned(),
                    prices: vec![45.0]
                }
            ),
            vec![Observation {
                day: "2026-09-25".to_owned(),
                prices: vec![45.0]
            }]
        );
    }

    #[test]
    fn a_real_drop_counts_float_noise_and_first_observations_do_not() {
        assert!(dropped(Some(35.99), 29.99));
        assert!(!dropped(Some(29.99), 29.985));
        assert!(!dropped(None, 29.99));
        assert!(!dropped(Some(29.99), 35.99));
    }

    #[test]
    fn fare_context_is_part_of_the_identity_and_absent_context_is_absent() {
        assert_eq!(
            watch_key(&RailWatch {
                plan_id: "p".to_owned(),
                from: "1".to_owned(),
                to: "2".to_owned(),
                time: "2026-09-01T08:00:00".to_owned(),
                bc: Some(25),
                d_ticket: false,
                first_class: false,
                stage_id: None,
            }),
            "1:2:2026-09-01T08:00:00:bc25"
        );
        assert_eq!(
            watch_key(&RailWatch {
                plan_id: "p".to_owned(),
                from: "1".to_owned(),
                to: "2".to_owned(),
                time: "2026-09-01T08:00:00".to_owned(),
                bc: None,
                d_ticket: false,
                first_class: false,
                stage_id: None,
            }),
            "1:2:2026-09-01T08:00:00"
        );
    }

    #[test]
    fn the_leg_key_is_the_station_pair_on_the_day() {
        assert_eq!(
            leg_key("8000207", "8000105", "2026-09-01T08:00:00"),
            "8000207:8000105:2026-09-01"
        );
        // Shorter than a day prefix: `slice(0, 10)` returns the whole string.
        assert_eq!(leg_key("1", "2", "2026-09-01"), "1:2:2026-09-01");
    }
}
