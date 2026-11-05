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
