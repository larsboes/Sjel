//! Types, status filters, promotion forms, and formatting helpers for Scouting.
use gpui_kit::component::input::InputState;
use gpui_kit::{div, prelude::*, rgb, AnyElement, Entity, SharedString, Subscription};
use sjel_gpui_core::scouting::{
    CalendarCandidateVerdict, OpportunityStatus, ScoutingOpportunity, ScoutingSource,
};

use crate::theme::Palette;

pub const PAGE_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusFilter {
    New,
    Saved,
    Dismissed,
    All,
}

impl StatusFilter {
    pub const ALL: [Self; 4] = [Self::New, Self::Saved, Self::Dismissed, Self::All];

    pub fn label(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Saved => "Saved",
            Self::Dismissed => "Dismissed",
            Self::All => "All",
        }
    }

    pub fn matches(self, opportunity: &ScoutingOpportunity) -> bool {
        self == Self::All
            || matches!(
                (self, opportunity.status),
                (Self::New, OpportunityStatus::New)
                    | (Self::Saved, OpportunityStatus::Saved)
                    | (Self::Dismissed, OpportunityStatus::Dismissed)
            )
    }
}

pub struct PromotionForm {
    pub opportunity: ScoutingOpportunity,
    pub fields: Vec<(String, String, Entity<InputState>)>,
    pub error: Option<SharedString>,
    pub saving: bool,
    pub _subscriptions: Vec<Subscription>,
}

impl PromotionForm {
    pub fn value(&self, key: &str, cx: &gpui::App) -> String {
        self.fields
            .iter()
            .find(|(field, _, _)| field == key)
            .map(|(_, _, input)| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
}

pub fn source_label(source: &ScoutingSource) -> String {
    let name = source
        .id
        .split(['-', '_'])
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("{name} · {}", type_label(&source.opportunity_type))
}

pub fn type_label(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "event" => "Event",
        "hackathon" => "Hackathon",
        "scholarship" => "Scholarship",
        "conference" => "Conference",
        "meetup" => "Meetup",
        "cfp" => "Call for Papers",
        _ => value,
    }
    .to_owned()
}

pub fn rationale_text(value: &str) -> String {
    value
        .lines()
        // `vault link:` is the marker rows stored before the rename carry.
        .filter(|line| !line.contains("note link:") && !line.contains("vault link:"))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned()
}

pub fn calendar_verdict_text(verdict: &CalendarCandidateVerdict) -> String {
    if verdict.already_in_calendar {
        return "Already in Calendar".to_owned();
    }
    let label = match verdict.verdict.as_str() {
        "free" => "Fits the calendar",
        "needs-travel-day" => "Needs coordination",
        "conflicts" => "Calendar conflict",
        _ => "Calendar checked",
    };
    let evidence = verdict
        .evidence
        .iter()
        .find(|entry| entry.impact == verdict.verdict);
    match (verdict.verdict.as_str(), evidence) {
        ("conflicts", Some(entry)) => format!("{label} · Conflicts with ‘{}’", entry.title),
        ("needs-travel-day", Some(entry)) => {
            format!("{label} · ‘{}’ requires travel or remote time", entry.title)
        }
        ("free", Some(entry)) => format!("{label} · ‘{}’ does not block this time", entry.title),
        ("free", None) => format!("{label} · No blocking overlap"),
        (_, None) => label.to_owned(),
        (_, Some(entry)) => format!("{label} · {}", entry.title),
    }
}

pub fn muted(value: impl Into<SharedString>, palette: Palette) -> AnyElement {
    div()
        .text_xs()
        .text_color(rgb(palette.muted))
        .child(value.into())
        .into_any_element()
}
