//! Investment ideas: one thought about one instrument, held or not.
//!
//! Storage is append-only. Every create or edit records a full snapshot, and a note is its own
//! event, so the history of a thesis survives every edit. The current idea is its newest
//! snapshot. Plan: `docs/plans/finance.md`, section 1.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stance {
    Hold,
    Watch,
    Enter,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdeaStatus {
    #[default]
    Open,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Above,
    Below,
}

/// A price level. `price` is a decimal with a point, such as `1200.50`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trigger {
    pub direction: Direction,
    pub price: String,
    pub currency: String,
}

/// What a caller sends to create or replace an idea.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaDraft {
    pub label: String,
    /// The market-data symbol. Empty when no provider quotes the instrument.
    #[serde(default)]
    pub ticker: String,
    /// An `instruments` id when the idea is about a held position.
    #[serde(default)]
    pub instrument: Option<String>,
    pub stance: Stance,
    #[serde(default)]
    pub thesis: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub status: IdeaStatus,
    #[serde(default)]
    pub close_note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IdeaNote {
    pub text: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IdeaSnapshot {
    #[serde(flatten)]
    pub draft: IdeaDraft,
    pub recorded_at: String,
}

/// The current state of one idea, with its history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Idea {
    pub id: String,
    #[serde(flatten)]
    pub current: IdeaDraft,
    pub created_at: String,
    pub updated_at: String,
    pub history: Vec<IdeaSnapshot>,
    pub notes: Vec<IdeaNote>,
}

/// One stored event, in the order it was recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdeaEvent {
    Snapshot(IdeaDraft),
    Note(String),
}

/// Normalizes a draft and refuses one that cannot be stored as written.
pub fn validate(mut draft: IdeaDraft) -> Result<IdeaDraft, String> {
    draft.label = draft.label.trim().to_string();
    draft.ticker = draft.ticker.trim().to_ascii_uppercase();
    draft.instrument = draft
        .instrument
        .map(|instrument| instrument.trim().to_string())
        .filter(|instrument| !instrument.is_empty());
    draft.keywords = draft
        .keywords
        .into_iter()
        .map(|keyword| keyword.trim().to_string())
        .filter(|keyword| !keyword.is_empty())
        .collect();
    if draft.label.is_empty() {
        return Err("label must not be empty".into());
    }
    if !draft
        .ticker
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '^' | '='))
    {
        return Err(format!(
            "ticker {:?} has a character no market symbol uses",
            draft.ticker
        ));
    }
    for trigger in &mut draft.triggers {
        trigger.currency = trigger.currency.trim().to_ascii_uppercase();
        trigger.price = trigger.price.trim().to_string();
        if trigger.currency.len() != 3 || !trigger.currency.chars().all(|c| c.is_ascii_uppercase())
        {
            return Err(format!(
                "trigger currency {:?} is not a three-letter code",
                trigger.currency
            ));
        }
        let strict = trigger.price.split_once('.').map_or(
            !trigger.price.is_empty() && trigger.price.chars().all(|c| c.is_ascii_digit()),
            |(whole, fraction)| {
                !whole.is_empty()
                    && !fraction.is_empty()
                    && whole
                        .chars()
                        .chain(fraction.chars())
                        .all(|c| c.is_ascii_digit())
            },
        );
        let price = crate::price::parse_provider_decimal(&trigger.price)
            .ok()
            .filter(|_| strict)
            .ok_or_else(|| {
                format!(
                    "trigger price {:?} is not a decimal such as 1200.50",
                    trigger.price
                )
            })?;
        if price.mantissa <= 0 {
            return Err(format!(
                "trigger price {:?} must be above zero",
                trigger.price
            ));
        }
    }
    Ok(draft)
}

/// Folds one idea's events, oldest first, into its current state. `None` when no snapshot
/// exists, because a note alone does not make an idea.
pub fn fold(id: &str, events: &[(IdeaEvent, String)]) -> Option<Idea> {
    let mut history = Vec::new();
    let mut notes = Vec::new();
    for (event, recorded_at) in events {
        match event {
            IdeaEvent::Snapshot(draft) => history.push(IdeaSnapshot {
                draft: draft.clone(),
                recorded_at: recorded_at.clone(),
            }),
            IdeaEvent::Note(text) => notes.push(IdeaNote {
                text: text.clone(),
                recorded_at: recorded_at.clone(),
            }),
        }
    }
    let first = history.first()?;
    let last = history.last()?;
    Some(Idea {
        id: id.to_string(),
        current: last.draft.clone(),
        created_at: first.recorded_at.clone(),
        updated_at: last.recorded_at.clone(),
        history,
        notes,
    })
}

/// The instrument key a price for this idea is stored under: the held instrument when there
/// is one, else the ticker.
pub fn price_key(draft: &IdeaDraft) -> Option<&str> {
    draft
        .instrument
        .as_deref()
        .or_else(|| (!draft.ticker.is_empty()).then_some(draft.ticker.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(label: &str, ticker: &str) -> IdeaDraft {
        IdeaDraft {
            label: label.into(),
            ticker: ticker.into(),
            instrument: None,
            stance: Stance::Watch,
            thesis: String::new(),
            keywords: Vec::new(),
            triggers: Vec::new(),
            status: IdeaStatus::Open,
            close_note: String::new(),
        }
    }

    #[test]
    fn validation_normalizes_and_refuses_what_cannot_be_stored() {
        let mut ok = draft(" Synthetic Exchange ", " s68.si ");
        ok.keywords = vec![" SGX ".into(), " ".into()];
        ok.triggers = vec![Trigger {
            direction: Direction::Below,
            price: "8.40".into(),
            currency: "sgd".into(),
        }];
        let ok = validate(ok).expect("valid");
        assert_eq!(ok.label, "Synthetic Exchange");
        assert_eq!(ok.ticker, "S68.SI");
        assert_eq!(ok.keywords, ["SGX"]);
        assert_eq!(ok.triggers[0].currency, "SGD");

        assert!(validate(draft(" ", "")).is_err());
        assert!(validate(draft("A", "A B")).is_err());
        let mut bad_price = draft("A", "A");
        bad_price.triggers = vec![Trigger {
            direction: Direction::Above,
            price: "1.200,50".into(),
            currency: "EUR".into(),
        }];
        assert!(validate(bad_price).is_err());
    }

    #[test]
    fn an_edit_keeps_the_old_thesis_in_the_history() {
        let mut first = draft("Synthetic", "SYN");
        first.thesis = "cheap".into();
        let mut second = first.clone();
        second.thesis = "no longer cheap".into();
        let events = vec![
            (
                IdeaEvent::Snapshot(first),
                "2026-11-01T10:00:00Z".to_string(),
            ),
            (
                IdeaEvent::Note("read the report".into()),
                "2026-11-02T10:00:00Z".to_string(),
            ),
            (
                IdeaEvent::Snapshot(second),
                "2026-11-03T10:00:00Z".to_string(),
            ),
        ];
        let idea = fold("idea-1", &events).expect("an idea");
        assert_eq!(idea.current.thesis, "no longer cheap");
        assert_eq!(idea.history[0].draft.thesis, "cheap");
        assert_eq!(idea.created_at, "2026-11-01T10:00:00Z");
        assert_eq!(idea.updated_at, "2026-11-03T10:00:00Z");
        assert_eq!(idea.notes.len(), 1);
        assert!(fold("idea-2", &[(IdeaEvent::Note("x".into()), "t".into())]).is_none());
    }
}

/// A trigger's stable identity across edits that keep the same level.
pub fn trigger_key(trigger: &Trigger) -> String {
    let direction = match trigger.direction {
        Direction::Above => "above",
        Direction::Below => "below",
    };
    format!("{direction}:{}:{}", trigger.price, trigger.currency)
}

/// `amount` in EUR: as is when it is EUR, else through the newest published rate.
fn in_eur(
    amount: &crate::investment::Quantity,
    currency: &str,
    rates: &[crate::price::FxObservation],
) -> Option<crate::investment::Quantity> {
    if crate::money::is_declared(currency) {
        Some(amount.clone())
    } else {
        crate::money::unit_price_in_eur(amount, currency, rates).map(|(price, _)| price)
    }
}

/// `left` compared with `right`, exactly, at the wider scale.
fn compare(
    left: &crate::investment::Quantity,
    right: &crate::investment::Quantity,
) -> Option<std::cmp::Ordering> {
    let scale = left.scale.max(right.scale);
    let widen = |q: &crate::investment::Quantity| {
        i128::from(q.mantissa).checked_mul(10_i128.checked_pow(scale - q.scale)?)
    };
    Some(widen(left)?.cmp(&widen(right)?))
}

/// Whether `price` (quoted in `currency`) is past the trigger's level. `None` when the two
/// currencies differ and no published rate joins them.
pub fn is_past(
    trigger: &Trigger,
    price: &crate::investment::Quantity,
    currency: &str,
    rates: &[crate::price::FxObservation],
) -> Option<bool> {
    let level = crate::price::parse_provider_decimal(&trigger.price).ok()?;
    let (price, level) = if currency.eq_ignore_ascii_case(&trigger.currency) {
        (price.clone(), level)
    } else {
        (
            in_eur(price, currency, rates)?,
            in_eur(&level, &trigger.currency, rates)?,
        )
    };
    let ordering = compare(&price, &level)?;
    Some(match trigger.direction {
        Direction::Above => ordering.is_ge(),
        Direction::Below => ordering.is_le(),
    })
}

/// What one evaluation does to one trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerStep {
    /// Past the level and armed: notify once and disarm.
    Hit,
    /// Back on the near side after a hit: arm again.
    Rearm,
    Nothing,
}

/// `armed` is false after a hit until a rearm. A trigger with no history is armed.
pub fn step(past: bool, armed: bool) -> TriggerStep {
    match (past, armed) {
        (true, true) => TriggerStep::Hit,
        (false, false) => TriggerStep::Rearm,
        _ => TriggerStep::Nothing,
    }
}

#[cfg(test)]
mod trigger_tests {
    use super::*;
    use crate::investment::Quantity;

    fn q(mantissa: i64, scale: u32) -> Quantity {
        Quantity { mantissa, scale }
    }

    #[test]
    fn a_level_fires_once_per_crossing() {
        let trigger = Trigger {
            direction: Direction::Above,
            price: "100".into(),
            currency: "EUR".into(),
        };
        let mut armed = true;
        let mut hits = 0;
        for price in [9_000, 10_100, 10_500, 9_900, 10_200] {
            let past = is_past(&trigger, &q(price, 2), "EUR", &[]).expect("same currency");
            match step(past, armed) {
                TriggerStep::Hit => {
                    hits += 1;
                    armed = false;
                }
                TriggerStep::Rearm => armed = true,
                TriggerStep::Nothing => {}
            }
        }
        assert_eq!(hits, 2);
    }

    #[test]
    fn a_level_in_another_currency_compares_through_the_rate() {
        let rate = crate::price::FxObservation {
            base: "EUR".into(),
            quote: "USD".into(),
            observed_on: "2026-11-05".into(),
            rate: q(11_000, 4),
            source: "ecb".into(),
            fetched_at: "2026-11-05T00:00:00Z".into(),
        };
        let below_100_eur = Trigger {
            direction: Direction::Below,
            price: "100".into(),
            currency: "EUR".into(),
        };
        // 105.60 USD is 96.00 EUR.
        assert_eq!(
            is_past(
                &below_100_eur,
                &q(10_560, 2),
                "USD",
                std::slice::from_ref(&rate)
            ),
            Some(true)
        );
        // 115.50 USD is 105.00 EUR.
        assert_eq!(
            is_past(
                &below_100_eur,
                &q(11_550, 2),
                "USD",
                std::slice::from_ref(&rate)
            ),
            Some(false)
        );
        assert_eq!(is_past(&below_100_eur, &q(1, 0), "NOK", &[rate]), None);
    }
}

/// One trigger's result in a check run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerOutcome {
    pub idea_id: String,
    pub label: String,
    pub trigger_key: String,
    /// `hit`, `rearm`, `failed` or `skipped`.
    pub action: &'static str,
    pub detail: String,
}

pub fn quantity_text(quantity: &crate::investment::Quantity) -> String {
    let digits = quantity.mantissa.unsigned_abs().to_string();
    let scale = quantity.scale as usize;
    let sign = if quantity.mantissa < 0 { "-" } else { "" };
    if scale == 0 {
        return format!("{sign}{digits}");
    }
    let padded = format!("{digits:0>width$}", width = scale + 1);
    let (whole, fraction) = padded.split_at(padded.len() - scale);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}

/// Compares every open idea's triggers with its newest price. A hit creates a task due today
/// in `project` through the `projects` capability, and is recorded only when the task exists,
/// so a failed delivery is retried on the next run.
pub fn check_triggers(
    store: &crate::store::FinanceStore,
    projects_base_url: &str,
    project: &str,
    today: &str,
    recorded_at: &str,
) -> Result<Vec<TriggerOutcome>, String> {
    let ideas = store.ideas(None).map_err(|error| error.to_string())?;
    let prices = store.latest_prices().map_err(|error| error.to_string())?;
    let rates = store.latest_fx_rates().map_err(|error| error.to_string())?;
    let mut outcomes = Vec::new();
    for idea in ideas
        .iter()
        .filter(|idea| idea.current.status == IdeaStatus::Open)
    {
        let price = price_key(&idea.current)
            .and_then(|key| prices.iter().find(|price| price.instrument == key));
        for trigger in &idea.current.triggers {
            let key = trigger_key(trigger);
            let mut outcome = TriggerOutcome {
                idea_id: idea.id.clone(),
                label: idea.current.label.clone(),
                trigger_key: key.clone(),
                action: "skipped",
                detail: String::new(),
            };
            let Some(price) = price else {
                outcome.detail = "no price for this idea's instrument or ticker".into();
                outcomes.push(outcome);
                continue;
            };
            let Some(past) = is_past(trigger, &price.price, &price.currency, &rates) else {
                outcome.detail = format!(
                    "no published rate joins {} and {}",
                    price.currency, trigger.currency
                );
                outcomes.push(outcome);
                continue;
            };
            let armed = store
                .trigger_armed(&idea.id, &key)
                .map_err(|error| error.to_string())?;
            let price_text = quantity_text(&price.price);
            match step(past, armed) {
                TriggerStep::Nothing => continue,
                TriggerStep::Rearm => outcome.action = "rearm",
                TriggerStep::Hit => {
                    let direction = match trigger.direction {
                        Direction::Above => "above",
                        Direction::Below => "below",
                    };
                    let title = format!(
                        "{} {direction} {} {}, now {price_text} {} ({})",
                        idea.current.label,
                        trigger.price,
                        trigger.currency,
                        price.currency,
                        price.observed_on
                    );
                    let summary = format!(
                        "Price trigger of the finance idea {}. Thesis: {}",
                        idea.id, idea.current.thesis
                    );
                    match post_task(projects_base_url, project, &title, &summary, today) {
                        Ok(()) => {
                            outcome.action = "hit";
                            outcome.detail = title;
                        }
                        Err(error) => {
                            outcome.action = "failed";
                            outcome.detail = error;
                            outcomes.push(outcome);
                            continue;
                        }
                    }
                }
            }
            store
                .append_trigger_event(
                    &idea.id,
                    &key,
                    outcome.action == "hit",
                    &price_text,
                    &price.currency,
                    &price.observed_on,
                    recorded_at,
                )
                .map_err(|error| error.to_string())?;
            outcomes.push(outcome);
        }
    }
    Ok(outcomes)
}

fn post_task(
    projects_base_url: &str,
    project: &str,
    title: &str,
    summary: &str,
    due: &str,
) -> Result<(), String> {
    let client = crate::price::blocking_client()?;
    let url = format!("{}/api/tasks", projects_base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "project": project,
        "title": title,
        "summary": summary,
        "due": due,
        "priority": 1,
    });
    sjel_server::InboundAuth::with_loopback_auth(client.post(&url).json(&body), &url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map(|_| ())
        .map_err(|error| format!("projects /api/tasks: {error}"))
}

/// The terms research is matched on: the label, the ticker without its exchange suffix
/// (`RHM` for `RHM.DE`, so the suffix `DE` does not match every German headline), and the
/// keywords.
pub fn research_terms(draft: &IdeaDraft) -> Vec<String> {
    let symbol = draft
        .ticker
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string();
    std::iter::once(draft.label.clone())
        .chain(std::iter::once(symbol))
        .chain(draft.keywords.iter().cloned())
        .filter(|term| !term.trim().is_empty())
        .collect()
}

/// The most feed items one idea lists.
pub const MAX_RESEARCH_ITEMS: usize = 8;

/// Feed items whose title or summary names one of the idea's terms, newest first.
/// `items` pairs each item with its summary, which is matched but not returned.
pub fn match_research(
    draft: &IdeaDraft,
    items: &[(crate::decision::FeedEvidence, String)],
) -> Vec<crate::decision::FeedEvidence> {
    let terms = research_terms(draft);
    let mut matched: Vec<_> = items
        .iter()
        .filter(|(item, summary)| {
            terms.iter().any(|term| {
                crate::decision::mentions(&item.title, term)
                    || crate::decision::mentions(summary, term)
            })
        })
        .map(|(item, _)| item.clone())
        .collect();
    matched.sort_by(|a, b| b.day.cmp(&a.day).then(a.id.cmp(&b.id)));
    matched.truncate(MAX_RESEARCH_ITEMS);
    matched
}

#[cfg(test)]
mod research_tests {
    use super::*;
    use crate::decision::FeedEvidence;

    fn item(id: &str, title: &str, day: &str) -> (FeedEvidence, String) {
        (
            FeedEvidence {
                id: id.into(),
                title: title.into(),
                url: format!("https://example.org/{id}"),
                day: day.into(),
            },
            String::new(),
        )
    }

    fn idea(label: &str, ticker: &str, keywords: &[&str]) -> IdeaDraft {
        IdeaDraft {
            label: label.into(),
            ticker: ticker.into(),
            instrument: None,
            stance: Stance::Watch,
            thesis: String::new(),
            keywords: keywords.iter().map(|k| (*k).into()).collect(),
            triggers: Vec::new(),
            status: IdeaStatus::Open,
            close_note: String::new(),
        }
    }

    #[test]
    fn a_planted_item_shows_on_its_idea_and_on_no_other() {
        let items = vec![
            item("a", "Synthex raises its dividend", "2026-11-02"),
            item("b", "Markets in DE close higher", "2026-11-03"),
            (
                FeedEvidence {
                    id: "c".into(),
                    title: "Quarterly roundup".into(),
                    url: "https://example.org/c".into(),
                    day: "2026-11-04".into(),
                },
                "Volumes at SXG hit a record".into(),
            ),
        ];
        let exchange = idea("Synthex", "SYX.SI", &["SXG"]);
        let other = idea("Othercorp", "OTH.DE", &[]);
        let matched: Vec<_> = match_research(&exchange, &items)
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(matched, ["c", "a"]);
        assert!(match_research(&other, &items).is_empty());
    }
}
