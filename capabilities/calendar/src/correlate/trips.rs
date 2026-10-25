use super::*;

/// A run of entries in one place, close enough in time to be one trip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TripDraft {
    /// The clustering key: the city, derived by `city_of` from whatever the
    /// source recorded. The venue lives on the member entries, where a trip to
    /// two places in one city can still show both.
    pub place: String,
    pub starts_on: String,
    /// Exclusive, like every other end in this capability.
    pub ends_before: String,
    pub entry_ids: Vec<String>,
    pub titles: Vec<String>,
    /// The strongest commitment among the members. A cluster of things you
    /// merely found is a possible trip; one booked ticket among them makes the
    /// whole journey real, because you are going either way.
    pub commitment: Commitment,
    /// The Trips plan a member already became, so a client offers "open" instead of "create".
    /// Filled by the server from the materialization ledger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
}

/// An entry that cannot be clustered, and why. Reported rather than dropped:
/// 13 of 62 events in the last live Luma sweep had no usable place, and a
/// silent filter is indistinguishable from a bug the first time something you
/// wanted goes missing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Unclustered {
    pub entry_id: String,
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TripDrafts {
    pub drafts: Vec<TripDraft>,
    pub unclustered: Vec<Unclustered>,
}

/// Where an entry is, as the sources actually record it.
///
/// Luma writes the city into `payload.city` and the street into `location`, so
/// `location` alone would cluster "Theresienstraße 6" separately from
/// "München". City first, `location` as the fallback for entries that carry
/// only that.
pub(super) fn place_of(entry: &Entry) -> Option<String> {
    let city = entry
        .payload
        .get("city")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|city| !city.is_empty());
    // Older mail proposals stored a model's JSON null as the string "null".
    let fallback = entry
        .location
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.eq_ignore_ascii_case("null"));
    city.or(fallback)
        .filter(|place| !is_online(place))
        .map(|place| place.to_string())
}

/// `Virtual`, `Online und live über CareerFairy`, `online via Microsoft Teams`: an event you
/// attend from wherever you are. It has no place, so three of them are not a trip to "Virtual".
pub(super) fn is_online(place: &str) -> bool {
    let first = place
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or_default();
    ["online", "virtual"]
        .iter()
        .any(|word| first.eq_ignore_ascii_case(word))
}

/// The departure city of a ticket leg, as Comms records it in `payload.from_city`.
pub(super) fn from_city_of(entry: &Entry) -> Option<&str> {
    entry
        .payload
        .get("from_city")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|city| !city.is_empty())
}

/// The longest stay a booked outward and return leg still bracket as one trip.
/// ponytail: fixed bound; make it a query parameter if month-long stays appear.
const TICKET_GAP_DAYS: i64 = 30;

/// Is this place the city the operator lives in, however the source spelled it?
///
/// `home` is a bare city. What sources actually write into `location`
/// is a venue line: `Nordwerk, Example City`, or `Beispiel Innovation Hub,
/// Musterdeich 15, 20097 Hamburg`. Comparing the whole line therefore never fired for
/// exactly the entries this exists for, and two committed days at the
/// operator's own employer, in the city they live in, were proposed as a
/// journey (found live 2026-08-04).
///
/// So the comparison runs per address segment, with a leading postal code
/// dropped. Segments rather than a substring search on purpose: a venue named
/// `Moxy Köln/Bonn Flughafen` is in Köln, and a rule that found `Bonn` inside
/// it would silently cancel a real trip — the failure that costs something.
pub(super) fn is_home(place: &str, home: &str) -> bool {
    let home = home.trim().to_lowercase();
    !home.is_empty()
        && place
            .split(',')
            .map(without_postal_code)
            .any(|segment| segment.to_lowercase() == home)
}

/// `20097 Hamburg` is the city Hamburg. Only a *leading* run of digits goes: a
/// house number trails its street (`Musterdeich 15`), so a segment ending in
/// digits is an address line and must not be mistaken for a city.
pub(super) fn without_postal_code(segment: &str) -> &str {
    let segment = segment.trim();
    match segment.split_once(' ') {
        Some((first, rest)) if !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()) => {
            rest.trim()
        }
        _ => segment,
    }
}

/// The city a free-text place is in — the key two events have to share before
/// they can be one journey.
///
/// Grouping on the raw string made the venue part of the identity, so a
/// conference at `Beispiel Innovation Hub, Musterdeich 15, 20097 Hamburg`
/// and a meetup elsewhere in Hamburg were two places, and neither ever reached
/// the two-event floor that makes a trip. You do not travel to a venue; you
/// travel to a city and then walk.
///
/// A postal code is a city's own routing key, so the segment carrying one is
/// the city segment wherever it sits. Failing that, the last segment: German
/// venue lines end on the city. That is the assumption, and its limit is
/// `Berlin, Germany`, which would yield `Germany` — it does not occur here
/// because Luma writes `payload.city`, which `place_of` prefers outright, and
/// the manual entries are addresses. If it ever does, the fix is to record
/// `payload.city`, not to teach this function geography.
pub(super) fn city_of(place: &str) -> String {
    let segments: Vec<&str> = place
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let city = segments
        .iter()
        .find(|segment| without_postal_code(segment) != **segment)
        .or_else(|| segments.last())
        .map(|segment| without_postal_code(segment))
        .unwrap_or("")
        .trim();
    if city.is_empty() {
        place.trim().to_string()
    } else {
        city.to_string()
    }
}

/// Groups entries into draft trips.
///
/// `max_gap_days` is how far apart two things in the same place can be and
/// still be one journey — the issue's own example is two Munich events five
/// days apart. `home` is the place that is never a trip; passing `None` means
/// every place clusters, including the one you live in, which is visible
/// rather than silently wrong.
pub fn cluster_trips(
    entries: &[Entry],
    max_gap_days: i64,
    home: Option<&str>,
) -> Result<TripDrafts, String> {
    let mut by_place: std::collections::BTreeMap<String, Vec<&Entry>> = Default::default();
    let mut returns = std::collections::HashSet::new();
    let mut unclustered = Vec::new();

    for entry in entries {
        if entry.kind != "event" && entry.kind != "nightlife" {
            continue;
        }
        match place_of(entry) {
            None => unclustered.push(Unclustered {
                entry_id: entry.id.clone(),
                title: entry.title.clone(),
                reason: "no place recorded, so it cannot be grouped with anything".into(),
            }),
            Some(place) => {
                // The home test reads the *raw* place, every segment of it,
                // and the clustering key reads the derived city. Deliberately
                // different: a forgiving home test can only ever exclude a day
                // the operator was at home anyway, while a forgiving grouping
                // key would merge two cities into one journey.
                // A ticket home from somewhere else ends the trip there.
                let place = match (home, from_city_of(entry)) {
                    (Some(home), Some(from)) if is_home(&place, home) && !is_home(from, home) => {
                        returns.insert(entry.id.as_str());
                        from.to_string()
                    }
                    _ => place,
                };
                if home.is_some_and(|home| is_home(&place, home)) {
                    unclustered.push(Unclustered {
                        entry_id: entry.id.clone(),
                        title: entry.title.clone(),
                        reason: format!("{place} is home, which is not a trip"),
                    });
                    continue;
                }
                by_place.entry(city_of(&place)).or_default().push(entry);
            }
        }
    }

    let mut drafts = Vec::new();
    for (place, mut members) in by_place {
        members.sort_by(|a, b| a.starts_at.cmp(&b.starts_at));
        let mut run: Vec<&Entry> = Vec::new();
        let mut run_end: Option<i64> = None;

        for entry in members {
            let (start, end) = entry_days(entry)?;
            // Only a return leg home bridges the long gap. Two outward tickets to one
            // city are two journeys.
            let gap = if returns.contains(entry.id.as_str()) {
                max_gap_days.max(TICKET_GAP_DAYS)
            } else {
                max_gap_days
            };
            let split = run_end.is_some_and(|previous_end| start - previous_end > gap);
            if split {
                push_trip_run(&mut drafts, &mut unclustered, &place, &run)?;
                run.clear();
                run_end = None;
            }
            run_end = Some(run_end.map_or(end, |previous| previous.max(end)));
            run.push(entry);
        }
        if !run.is_empty() {
            push_trip_run(&mut drafts, &mut unclustered, &place, &run)?;
        }
    }

    drafts.sort_by(|a, b| a.starts_on.cmp(&b.starts_on).then(a.place.cmp(&b.place)));
    Ok(TripDrafts {
        drafts,
        unclustered,
    })
}

/// A journey is evidence from multiple calendar anchors. A remote one-off is
/// still valuable, but it belongs in the ordinary Calendar proposal inbox;
/// rendering it as a trip would pressure the operator to create a plan merely
/// to acknowledge one event.
pub(super) fn push_trip_run(
    drafts: &mut Vec<TripDraft>,
    unclustered: &mut Vec<Unclustered>,
    place: &str,
    run: &[&Entry],
) -> Result<(), String> {
    if run.len() >= 2 {
        drafts.push(draft_from(place, run)?);
    } else if let Some(entry) = run.first() {
        unclustered.push(Unclustered {
            entry_id: entry.id.clone(),
            title: entry.title.clone(),
            reason: format!("only one event in {place}; review it as a calendar event, not a trip"),
        });
    }
    Ok(())
}

/// First and last day an entry touches, as day numbers. The stored end is
/// exclusive, so a same-day entry ends on the day it starts.
pub(super) fn entry_days(entry: &Entry) -> Result<(i64, i64), String> {
    let starts = date::parse_date(&entry.starts_at[..10.min(entry.starts_at.len())])
        .ok_or_else(|| format!("{}: unreadable starts_at", entry.id))?;
    let ends_raw = date::parse_date(&entry.ends_at[..10.min(entry.ends_at.len())])
        .ok_or_else(|| format!("{}: unreadable ends_at", entry.id))?;
    let ends = if entry.all_day {
        ends_raw - 1
    } else {
        ends_raw
    };
    Ok((starts, ends.max(starts)))
}

pub(super) fn draft_from(place: &str, run: &[&Entry]) -> Result<TripDraft, String> {
    let mut first = i64::MAX;
    let mut last = i64::MIN;
    for entry in run {
        let (start, end) = entry_days(entry)?;
        first = first.min(start);
        last = last.max(end);
    }
    Ok(TripDraft {
        place: place.to_string(),
        starts_on: date::format_date(first),
        ends_before: date::format_date(last + 1),
        entry_ids: run.iter().map(|e| e.id.clone()).collect(),
        titles: run.iter().map(|e| e.title.clone()).collect(),
        commitment: run
            .iter()
            .map(|e| e.commitment)
            .max()
            .unwrap_or(Commitment::Possible),
        plan_id: None,
    })
}
