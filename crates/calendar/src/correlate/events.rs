use super::*;

/// A caller-owned date range to correlate against Calendar entries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub starts_at: String,
    #[serde(default)]
    pub ends_at: Option<String>,
}

/// One entry that overlapped, with what it did to the verdict. The UI reads
/// this to say "you have on-site work that day" instead of showing a badge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub entry_id: String,
    pub kind: String,
    /// Carried so the UI can say *why* something is weightless — "you noted
    /// this, you didn't book it" reads very differently from "this is free".
    pub commitment: Commitment,
    pub title: String,
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: bool,
    pub impact: Feasibility,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Verdict {
    pub id: String,
    pub verdict: Feasibility,
    /// The range actually checked, after normalization — a caller that sent a
    /// provider timestamp can see what it was read as.
    pub starts_at: String,
    pub ends_at: String,
    /// An entry with this candidate's `external_id` already exists (Phase A's
    /// "add to Calendar" promoted it). It is excluded from the verdict:
    /// correlating an opportunity against itself would make every saved
    /// opportunity conflict with itself.
    pub already_in_calendar: bool,
    /// Strongest impact first. Includes neutral overlaps — they explain the
    /// day even when they do not drive the verdict.
    pub evidence: Vec<Evidence>,
}

/// A maximal run of days where travel is possible at all: every day whose
/// verdict is not `conflicts`. The soft cost is named, not filtered —
/// `days_needing_travel_day` lists the days inside the run that would cost a
/// remote day or a dropped block, and `verdict` is the worst day in the run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeasibleWindow {
    pub starts_on: String,
    /// Exclusive, like every other end in this capability.
    pub ends_before: String,
    /// Every day in the run, in order — the shape transit's `plan --dates`
    /// takes, so the hand-off needs no date arithmetic on the way over.
    pub days: Vec<String>,
    pub verdict: Feasibility,
    pub days_needing_travel_day: Vec<String>,
}

pub(super) const MINUTES_PER_DAY: i64 = 1440;

/// Provider instants arrive in whatever shape the source emits — Luma's
/// `2026-07-10T18:00:00.000Z`, euro-hackathons' `2026-10-23T00:00:00+00:00`,
/// transit's naive `2026-08-01T08:00:00`. The zone designator and any
/// fractional seconds are dropped and the wall clock is read as local: that is
/// the single-home-timezone call the README's time model already makes, and
/// nothing here converts anything. Phase E (Google sync) is where offsets
/// become real.
pub(super) fn normalize_instant(text: &str) -> Option<String> {
    let text = text.trim();
    let Some((day, time)) = text.split_once('T') else {
        // Date-only: strict, exactly as the store stores it.
        return date::parse_date(text).map(|_| text.to_string());
    };
    // A '-' can only be an offset sign here — the date half is already split
    // off — so cutting at the first of these leaves the bare wall time.
    let cut = time.find(['Z', 'z', '+', '-', '.']).unwrap_or(time.len());
    let normalized = format!("{day}T{}", &time[..cut]);
    date::parse_instant(&normalized).map(|_| normalized)
}

pub(super) struct Resolved {
    pub(super) start: i64,
    pub(super) end: i64,
    starts_at: String,
    ends_at: String,
}

/// Resolves a candidate to a half-open `[start, end)` minute range.
///
/// A missing end, or an end at or before the start (a provider that emits one
/// instant twice, which euro-hackathons does for single-day entries), becomes
/// the whole start day. An event without an end is a day, not an instant —
/// and an empty range would overlap nothing and report `free` for a day the
/// operator is demonstrably away.
pub(super) fn resolve(candidate: &Candidate) -> Result<Resolved, String> {
    let starts_at = normalize_instant(&candidate.starts_at).ok_or_else(|| {
        format!(
            "candidate {}: starts_at is not a date or local time: {}",
            candidate.id, candidate.starts_at
        )
    })?;
    let start = date::instant_minutes(&starts_at)
        .ok_or_else(|| format!("candidate {}: unreadable starts_at", candidate.id))?;

    let end = match candidate
        .ends_at
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        Some(raw) => {
            let ends_at = normalize_instant(raw).ok_or_else(|| {
                format!(
                    "candidate {}: ends_at is not a date or local time: {raw}",
                    candidate.id
                )
            })?;
            let end = date::instant_minutes(&ends_at)
                .ok_or_else(|| format!("candidate {}: unreadable ends_at", candidate.id))?;
            (end > start).then_some((end, ends_at))
        }
        None => None,
    };

    Ok(match end {
        Some((end, ends_at)) => Resolved {
            start,
            end,
            starts_at,
            ends_at,
        },
        None => {
            let day = start.div_euclid(MINUTES_PER_DAY);
            Resolved {
                start: day * MINUTES_PER_DAY,
                end: (day + 1) * MINUTES_PER_DAY,
                starts_at: date::format_date(day),
                ends_at: date::format_date(day + 1),
            }
        }
    })
}

/// An entry's `[start, end)` in minutes. An unreadable instant is an error,
/// not a skip: the store validates every write, so a row that fails here is
/// corruption, and a verdict computed as if the entry were not there would be
/// confidently wrong about the operator's own time.
pub(super) fn entry_range(entry: &Entry) -> Result<(i64, i64), String> {
    let start = date::instant_minutes(&entry.starts_at).ok_or_else(|| {
        format!(
            "entry {}: unreadable starts_at {}",
            entry.id, entry.starts_at
        )
    })?;
    let end = date::instant_minutes(&entry.ends_at)
        .ok_or_else(|| format!("entry {}: unreadable ends_at {}", entry.id, entry.ends_at))?;
    Ok((start, end))
}

/// Half-open overlap. Ends are exclusive everywhere in this capability, so an
/// entry ending 2026-08-15 does not touch an event on 2026-08-15.
pub(super) fn overlaps(a: (i64, i64), b: (i64, i64)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// Is this entry the candidate itself, already promoted?
///
/// Two ids can name the same thing, and both are legitimate. `external_id` is
/// the *provider's* key — the bare `evt-…` Luma id — deliberately so, because
/// a second path to the same event (an ICS import rather than the JSON scrape)
/// produces that same key and must dedupe onto the same row. But the caller
/// asking for a verdict is Feed's Discover view, and what it holds is the
/// *scouting opportunity* id, `evt:luma:evt-…`. Matching only `external_id`
/// meant a saved opportunity came back `already_in_calendar: false` and
/// `conflicts` — with itself, because its own promoted entry is `kind = event`.
/// Verified against live data 2026-07-30: the bare id matched, the namespaced
/// one did not.
///
/// Promotion writes the opportunity id into `payload.opportunity_id`, so the
/// link exists; this reads it. Neither side had to give up its own key.
pub(super) fn is_same_thing(entry: &Entry, candidate_id: &str) -> bool {
    if entry.external_id.as_deref() == Some(candidate_id) {
        return true;
    }
    entry.payload.get("opportunity_id").and_then(Value::as_str) == Some(candidate_id)
}

/// Do two entries describe the same real-world event?
///
/// `is_same_thing` answers that by *key*, which is the right answer whenever
/// both sides hold one. But identity in this store is `(source, external_id)`,
/// so the same party scraped off the venue's own site and imported from Google
/// lands as two rows sharing no key at all. One of them is something the
/// operator already adopted; the other keeps showing up in the proposal inbox
/// announcing itself as new — while the month grid draws both.
///
/// So this matches on what independent sources do agree about: it happens at
/// the same time, and it is called the same thing once spelling is taken out of
/// the comparison. Place is deliberately *not* required — 13 of 62 events in
/// the last live Luma sweep carried no usable place, which would exempt exactly
/// the entries most likely to arrive twice.
pub fn is_same_event(a: &Entry, b: &Entry) -> Result<bool, String> {
    if !overlaps(entry_range(a)?, entry_range(b)?) {
        return Ok(false);
    }
    let title = comparable_title(&a.title);
    Ok(!title.is_empty() && title == comparable_title(&b.title))
}

/// The proposals that are not already sitting in the calendar under another
/// key. Order is preserved: this only ever removes.
///
/// Read-time rather than write-time on purpose. Merging at ingest would have to
/// pick a winner between two sources' versions of the same event and would be
/// irreversible; suppressing at read leaves both rows intact, so a match this
/// gets wrong costs a proposal that reappears the moment the rule is corrected,
/// not data.
pub fn without_already_adopted(
    proposals: Vec<Entry>,
    adopted: &[Entry],
) -> Result<Vec<Entry>, String> {
    let mut kept = Vec::with_capacity(proposals.len());
    for proposal in proposals {
        let mut duplicate = false;
        for entry in adopted {
            if is_same_event(&proposal, entry)? {
                duplicate = true;
                break;
            }
        }
        if !duplicate {
            kept.push(proposal);
        }
    }
    Ok(kept)
}

/// A title reduced to what two independent sources can be expected to agree on.
///
/// Case, diacritics and punctuation are where they differ without meaning
/// anything different: the only thing between `Bootshaus pres. BC173 (let’s get
/// loco)` and `Bootshaus pres BC173 (lets get loco)` is a typographic
/// apostrophe and a full stop.
///
/// Only whitespace separates words. Every other non-alphanumeric is *dropped*
/// rather than treated as a break, which is the difference between `let’s`
/// folding to `lets` and shattering into `let s` — the second reads as two
/// words the other source simply does not have. Verified against live data
/// 2026-08-04: splitting on punctuation left the Bootshaus twins unmatched.
pub(super) fn comparable_title(title: &str) -> String {
    title
        .split_whitespace()
        .map(|word| {
            word.to_lowercase()
                .chars()
                .filter_map(|ch| match fold(ch) {
                    Some(folded) => Some(folded.to_string()),
                    None if ch.is_alphanumeric() => Some(ch.to_string()),
                    None => None,
                })
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Latin-1 diacritics folded to the letter the other source probably typed.
/// `ß` is the one that is not one-to-one, which is why this returns a string.
/// Everything else is already comparable and passes through untouched.
pub(super) fn fold(ch: char) -> Option<&'static str> {
    Some(match ch {
        'ä' | 'à' | 'á' | 'â' | 'ã' | 'å' => "a",
        'ë' | 'è' | 'é' | 'ê' => "e",
        'ï' | 'ì' | 'í' | 'î' => "i",
        'ö' | 'ò' | 'ó' | 'ô' | 'õ' | 'ø' => "o",
        'ü' | 'ù' | 'ú' | 'û' => "u",
        'ñ' => "n",
        'ç' => "c",
        'ß' => "ss",
        _ => return None,
    })
}

/// The verdict for one candidate against the entries of a window that covers
/// it. Entries outside the candidate's range are ignored, so passing a wider
/// window than needed is free.
pub fn verdict_for(candidate: &Candidate, entries: &[Entry]) -> Result<Verdict, String> {
    let range = resolve(candidate)?;
    let mut verdict = Feasibility::Free;
    let mut already_in_calendar = false;
    let mut evidence = Vec::new();

    for entry in entries {
        if is_same_thing(entry, &candidate.id) {
            already_in_calendar = true;
            continue;
        }
        let entry_range = entry_range(entry)?;
        if !overlaps((range.start, range.end), entry_range) {
            continue;
        }
        let entry_impact = impact(&entry.kind, entry.commitment);
        verdict = verdict.max(entry_impact);
        evidence.push(Evidence {
            entry_id: entry.id.clone(),
            kind: entry.kind.clone(),
            commitment: entry.commitment,
            title: entry.title.clone(),
            starts_at: entry.starts_at.clone(),
            ends_at: entry.ends_at.clone(),
            all_day: entry.all_day,
            impact: entry_impact,
        });
    }

    evidence.sort_by(|a, b| {
        b.impact
            .cmp(&a.impact)
            .then_with(|| a.starts_at.cmp(&b.starts_at))
    });

    Ok(Verdict {
        id: candidate.id.clone(),
        verdict,
        starts_at: range.starts_at,
        ends_at: range.ends_at,
        already_in_calendar,
        evidence,
    })
}

/// Verdicts for one caller-supplied batch, in the caller's order.
///
/// The HTTP edge reads one Calendar window for the whole batch and delegates
/// here. Keeping the loop beside `verdict_for` makes the served batch contract
/// testable without a database or an Axum router.
pub fn verdicts_for(candidates: &[Candidate], entries: &[Entry]) -> Result<Vec<Verdict>, String> {
    candidates
        .iter()
        .map(|candidate| verdict_for(candidate, entries))
        .collect()
}
