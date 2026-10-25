//! Native Scouting source discovery and opportunity review.

use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, point, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, ScrollHandle,
    SharedString, Subscription, Window,
};
use sjel_gpui_core::scouting::{
    calendar_entry_body, calendar_start_input, can_promote_to_calendar, default_calendar_end,
    CalendarCandidate, CalendarCandidateVerdict, DiscoverResponse, OpportunityStatus,
    ScoutingOpportunity, ScoutingSource,
};
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::Repo;
use crate::theme::Palette;

const PAGE_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusFilter {
    New,
    Saved,
    Dismissed,
    All,
}

impl StatusFilter {
    const ALL: [Self; 4] = [Self::New, Self::Saved, Self::Dismissed, Self::All];

    fn label(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Saved => "Saved",
            Self::Dismissed => "Dismissed",
            Self::All => "All",
        }
    }

    fn matches(self, opportunity: &ScoutingOpportunity) -> bool {
        self == Self::All
            || matches!(
                (self, opportunity.status),
                (Self::New, OpportunityStatus::New)
                    | (Self::Saved, OpportunityStatus::Saved)
                    | (Self::Dismissed, OpportunityStatus::Dismissed)
            )
    }
}

struct PromotionForm {
    opportunity: ScoutingOpportunity,
    fields: Vec<(String, String, Entity<InputState>)>,
    error: Option<SharedString>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl PromotionForm {
    fn value(&self, key: &str, cx: &gpui::App) -> String {
        self.fields
            .iter()
            .find(|(field, _, _)| field == key)
            .map(|(_, _, input)| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
}

pub struct ScoutingPage {
    repo: Repo,
    palette: Option<Palette>,
    sources: Vec<ScoutingSource>,
    opportunities: Vec<ScoutingOpportunity>,
    store_total: usize,
    adapter: Option<String>,
    loading: bool,
    scanning: bool,
    busy: Option<String>,
    error: Option<SharedString>,
    notice: Option<SharedString>,
    scan: Option<DiscoverResponse>,
    calendar_verdicts: HashMap<String, CalendarCandidateVerdict>,
    verdict_request: u64,
    status_filter: StatusFilter,
    shown: usize,
    location: Option<(Entity<InputState>, Subscription)>,
    query: Option<(Entity<InputState>, Subscription)>,
    promotion: Option<PromotionForm>,
    scroll: ScrollHandle,
}

impl ScoutingPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            palette: None,
            sources: Vec::new(),
            opportunities: Vec::new(),
            store_total: 0,
            adapter: None,
            loading: false,
            scanning: false,
            busy: None,
            error: None,
            notice: None,
            scan: None,
            calendar_verdicts: HashMap::new(),
            verdict_request: 0,
            status_filter: StatusFilter::New,
            shown: PAGE_SIZE,
            location: None,
            query: None,
            promotion: None,
            scroll: ScrollHandle::new(),
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            self.error = Some("Sjel is not available on this Mac.".into());
            cx.notify();
            return;
        };
        self.loading = true;
        self.error = None;
        cx.notify();
        let work = shell::run(async move {
            let _ = shell.set_running("scouting", true).await;
            let sources = shell.scouting_sources().await?;
            let opportunities = shell.scouting_opportunities().await?;
            Ok::<_, ShellError>((sources, opportunities))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.loading = false;
                match result {
                    Ok((sources, opportunities)) => {
                        page.sources = sources
                            .sources
                            .into_iter()
                            .filter(|source| source.enabled)
                            .collect();
                        page.store_total = opportunities.store_total;
                        page.opportunities = opportunities.opportunities;
                        if !page
                            .sources
                            .iter()
                            .any(|source| Some(&source.id) == page.adapter.as_ref())
                        {
                            page.adapter = page.sources.first().map(|source| source.id.clone());
                        }
                        page.load_calendar_verdicts(cx);
                    }
                    Err(error) => page.error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_calendar_verdicts(&mut self, cx: &mut Context<Self>) {
        self.verdict_request = self.verdict_request.wrapping_add(1);
        let request = self.verdict_request;
        let candidates: Vec<_> = self
            .opportunities
            .iter()
            .filter(|opportunity| can_promote_to_calendar(opportunity))
            .map(|opportunity| CalendarCandidate {
                id: opportunity.id.clone(),
                starts_at: opportunity.starts_at.clone(),
                ends_at: (!opportunity.ends_at.trim().is_empty())
                    .then(|| opportunity.ends_at.clone()),
            })
            .collect();
        if candidates.is_empty() {
            self.calendar_verdicts.clear();
            return;
        }
        self.calendar_verdicts.clear();
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let work = shell::run(async move {
            shell.set_running("calendar", true).await?;
            shell.calendar_candidate_verdicts(&candidates).await
        });
        cx.spawn(async move |this, cx| {
            if let Some(Ok(response)) = work.await {
                let _ = this.update(cx, |page, cx| {
                    if page.verdict_request != request {
                        return;
                    }
                    page.calendar_verdicts = response
                        .verdicts
                        .into_iter()
                        .map(|verdict| (verdict.id.clone(), verdict))
                        .collect();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn location_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.location {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Berlin"));
        let subscription = cx.subscribe_in(&input, window, |page, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            } else if matches!(event, InputEvent::PressEnter { .. }) {
                page.scan(cx);
            }
        });
        self.location = Some((input.clone(), subscription));
        input
    }

    fn query_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.query {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("AI, Security, Mobility…"));
        let subscription = cx.subscribe_in(&input, window, |page, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            } else if matches!(event, InputEvent::PressEnter { .. }) {
                page.scan(cx);
            }
        });
        self.query = Some((input.clone(), subscription));
        input
    }

    fn open_promotion(
        &mut self,
        opportunity: ScoutingOpportunity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(starts) = calendar_start_input(&opportunity.starts_at) else {
            self.error = Some("This opportunity has no usable date.".into());
            cx.notify();
            return;
        };
        let Some(ends) = default_calendar_end(&opportunity.starts_at, &opportunity.ends_at) else {
            self.error = Some("This opportunity has no usable end date.".into());
            cx.notify();
            return;
        };
        let mut subscriptions = Vec::new();
        let mut field = |key: &str, value: String, placeholder: &str| {
            let input = cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder(placeholder.to_owned());
                input.set_value(value, window, cx);
                input
            });
            subscriptions.push(cx.subscribe_in(&input, window, |page, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    page.save_promotion(cx);
                }
            }));
            (key.to_owned(), key.to_owned(), input)
        };
        let fields = vec![
            field("title", opportunity.title.clone(), "Event title"),
            field("starts_at", starts, "YYYY-MM-DD or YYYY-MM-DDTHH:MM"),
            field("ends_at", ends, "YYYY-MM-DD or YYYY-MM-DDTHH:MM"),
            field(
                "location",
                if opportunity.location.trim().is_empty() {
                    opportunity.city.clone()
                } else {
                    opportunity.location.clone()
                },
                "Location, optional",
            ),
        ];
        if let Some((_, _, first)) = fields.first() {
            first.update(cx, |input, cx| input.focus(window, cx));
        }
        self.promotion = Some(PromotionForm {
            opportunity,
            fields,
            error: None,
            saving: false,
            _subscriptions: subscriptions,
        });
        self.error = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn close_promotion(&mut self, cx: &mut Context<Self>) {
        self.promotion = None;
        cx.notify();
    }

    fn save_promotion(&mut self, cx: &mut Context<Self>) {
        let Some(shell) = self.repo.shell().ok() else {
            return;
        };
        let Some(form) = self.promotion.as_ref() else {
            return;
        };
        if form.saving {
            return;
        }
        let opportunity = form.opportunity.clone();
        let body = match calendar_entry_body(
            &opportunity,
            &form.value("title", cx),
            &form.value("starts_at", cx),
            &form.value("ends_at", cx),
            &form.value("location", cx),
        ) {
            Ok(body) => body,
            Err(error) => {
                if let Some(form) = self.promotion.as_mut() {
                    form.error = Some(error.into());
                }
                cx.notify();
                return;
            }
        };
        if let Some(form) = self.promotion.as_mut() {
            form.saving = true;
            form.error = None;
        }
        let promote_id = opportunity.id.clone();
        let promote_id_for_write = promote_id.clone();
        let status = opportunity.status;
        let work = shell::run(async move {
            shell.set_running("calendar", true).await?;
            shell.calendar_upsert_external(body).await?;
            if status != OpportunityStatus::Saved {
                shell
                    .scouting_set_status(promote_id_for_write.as_str(), OpportunityStatus::Saved)
                    .await?;
            }
            Ok::<_, ShellError>(())
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(()) => {
                        page.set_local_status(&promote_id, OpportunityStatus::Saved);
                        page.notice = Some("Added to Calendar.".into());
                        page.promotion = None;
                        page.load_calendar_verdicts(cx);
                    }
                    Err(error) => {
                        if let Some(form) = page.promotion.as_mut() {
                            form.saving = false;
                            form.error = Some(error.to_string().into());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn set_status(&mut self, id: String, status: OpportunityStatus, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(index) = self.opportunities.iter().position(|item| item.id == id) else {
            return;
        };
        let previous = self.opportunities[index].status;
        if previous == status {
            return;
        }
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        self.busy = Some(id.clone());
        self.error = None;
        self.notice = None;
        self.opportunities[index].status = status;
        cx.notify();
        let write_id = id.clone();
        let work = shell::run(async move { shell.scouting_set_status(&write_id, status).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.busy = None;
                if let Err(error) = result {
                    page.set_local_status(&id, previous);
                    page.error = Some(error.to_string().into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_local_status(&mut self, id: &str, status: OpportunityStatus) {
        if let Some(item) = self.opportunities.iter_mut().find(|item| item.id == id) {
            item.status = status;
        }
    }

    fn scan(&mut self, cx: &mut Context<Self>) {
        let Some(adapter) = self.adapter.clone() else {
            self.error = Some("Choose an enabled source first.".into());
            cx.notify();
            return;
        };
        if self.scanning {
            return;
        }
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let location = self
            .location
            .as_ref()
            .map(|(input, _)| input.read(cx).value().trim().to_owned())
            .filter(|value| !value.is_empty());
        let query = self
            .query
            .as_ref()
            .map(|(input, _)| input.read(cx).value().trim().to_owned())
            .filter(|value| !value.is_empty());
        self.scanning = true;
        self.error = None;
        self.notice = None;
        cx.notify();
        let work = shell::run(async move {
            shell.set_running("scouting", true).await?;
            let scan = shell
                .scouting_discover(&adapter, location.as_deref(), query.as_deref())
                .await?;
            let opportunities = shell.scouting_opportunities().await?;
            Ok::<_, ShellError>((scan, opportunities))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.scanning = false;
                match result {
                    Ok((scan, opportunities)) => {
                        page.scan = Some(scan);
                        page.store_total = opportunities.store_total;
                        page.opportunities = opportunities.opportunities;
                        page.status_filter = StatusFilter::New;
                        page.shown = PAGE_SIZE;
                        page.load_calendar_verdicts(cx);
                    }
                    Err(error) => page.error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn count(&self, filter: StatusFilter) -> usize {
        self.opportunities
            .iter()
            .filter(|opportunity| filter.matches(opportunity))
            .count()
    }

    fn render_promotion(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.promotion.as_ref()?;
        let mut fields = div().flex().flex_col().gap_2().flex_none().w_full();
        for (key, label, input) in &form.fields {
            fields = fields.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(label.clone()),
                    )
                    .child(Input::new(input).small())
                    .id(SharedString::from(format!("scout-calendar-{key}"))),
            );
        }
        if let Some(error) = &form.error {
            fields = fields.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        let saving = form.saving;
        Some(
            div()
                .p_4()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Review Calendar entry"),
                        )
                        .child(
                            Button::new("scout-calendar-cancel")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .disabled(saving)
                                .on_click(cx.listener(|page, _, _, cx| page.close_promotion(cx))),
                        ),
                )
                .child(muted(
                    "Check the date, time, and place before adding this opportunity to Calendar.",
                    palette,
                ))
                .child(fields)
                .child(
                    div().flex().justify_end().child(
                        Button::new("scout-calendar-save")
                            .small()
                            .primary()
                            .label(if saving {
                                "Saving…"
                            } else {
                                "Add to Calendar"
                            })
                            .disabled(saving)
                            .on_click(cx.listener(|page, _, _, cx| page.save_promotion(cx))),
                    ),
                )
                .into_any_element(),
        )
    }
}

fn source_label(source: &ScoutingSource) -> String {
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

fn type_label(value: &str) -> String {
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

fn rationale_text(value: &str) -> String {
    value
        .lines()
        .filter(|line| !line.contains("vault link:"))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned()
}

fn calendar_verdict_text(verdict: &CalendarCandidateVerdict) -> String {
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

fn muted(value: impl Into<SharedString>, palette: Palette) -> AnyElement {
    div()
        .text_xs()
        .text_color(rgb(palette.muted))
        .child(value.into())
        .into_any_element()
}

impl Render for ScoutingPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(Palette::dark);
        let location = self.location_input(window, cx);
        let query = self.query_input(window, cx);
        let selected = self.adapter.clone();
        let mut source_picker = div().flex().flex_wrap().gap_2();
        for source in &self.sources {
            let source_id = source.id.clone();
            let active = selected.as_deref() == Some(source_id.as_str());
            source_picker = source_picker.child(
                Button::new(SharedString::from(format!("scout-source-{source_id}")))
                    .small()
                    .label(source_label(source))
                    .when(active, |button| button.primary())
                    .when(!active, |button| button.ghost())
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.adapter = Some(source_id.clone());
                        page.scan = None;
                        cx.notify();
                    })),
            );
        }

        let mut visible: Vec<_> = self
            .opportunities
            .iter()
            .filter(|opportunity| self.status_filter.matches(opportunity))
            .collect();
        visible.truncate(self.shown);
        let visible_count = visible.len();
        let total_visible = self.count(self.status_filter);
        let store_total = self.store_total;
        let loading = self.loading;
        let scanning = self.scanning;
        let error = self.error.clone();
        let notice = self.notice.clone();
        let scan = self.scan.clone();

        let mut cards = div().flex().flex_col().gap_3().flex_none().w_full();
        for opportunity in visible {
            let id = opportunity.id.clone();
            let status = opportunity.status;
            let busy = self.busy.as_deref() == Some(id.as_str());
            let route = opportunity.event_route.as_ref();
            let calendar_verdict = self.calendar_verdicts.get(&id);
            let already_in_calendar =
                calendar_verdict.is_some_and(|verdict| verdict.already_in_calendar);
            let summary = rationale_text(&opportunity.rationale);
            let can_calendar = can_promote_to_calendar(opportunity);
            let original_url = opportunity.url.clone();
            let promote = opportunity.clone();
            let save_id = id.clone();
            let dismiss_id = id.clone();
            cards = cards.child(
                div()
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(if status == OpportunityStatus::Dismissed {
                        rgba(palette.surface)
                    } else {
                        rgba(palette.raised)
                    })
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().text_xs().text_color(rgb(palette.muted)).child(
                                        format!(
                                            "{} · {}{}",
                                            type_label(&opportunity.opportunity_type),
                                            opportunity.source,
                                            route.map_or(String::new(), |route| {
                                                format!(" · {}", route.route.label())
                                            })
                                        ),
                                    ))
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette.text))
                                            .child(opportunity.title.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(if opportunity.score >= 0.35 {
                                        palette.green
                                    } else {
                                        palette.muted
                                    }))
                                    .child(format!("Match {:.0}", opportunity.score * 100.)),
                            ),
                    )
                    .when(!summary.is_empty(), |card| {
                        card.child(
                            div()
                                .text_sm()
                                .text_color(rgb(palette.muted))
                                .child(summary),
                        )
                    })
                    .when_some(
                        route.filter(|route| !route.reason.trim().is_empty()),
                        |card, route| card.child(muted(route.reason.clone(), palette)),
                    )
                    .child(muted(
                        [
                            (!opportunity.starts_at.is_empty())
                                .then_some(opportunity.starts_at.get(..10).unwrap_or_default()),
                            (!opportunity.city.is_empty()).then_some(opportunity.city.as_str()),
                            (!opportunity.matched_focus.is_empty())
                                .then_some(opportunity.matched_focus.as_str()),
                        ]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · "),
                        palette,
                    ))
                    .when_some(calendar_verdict, |card, verdict| {
                        card.child(muted(calendar_verdict_text(verdict), palette))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new(SharedString::from(format!("scout-original-{id}")))
                                    .small()
                                    .ghost()
                                    .label("Open original ↗")
                                    .on_click(move |_, _, cx| cx.open_url(&original_url)),
                            )
                            .when(can_calendar, |row| {
                                row.child(
                                    Button::new(SharedString::from(format!("scout-calendar-{id}")))
                                        .small()
                                        .ghost()
                                        .label(if already_in_calendar {
                                            "Already in Calendar"
                                        } else {
                                            "Calendar…"
                                        })
                                        .disabled(busy || already_in_calendar)
                                        .on_click(cx.listener(move |page, _, window, cx| {
                                            page.open_promotion(promote.clone(), window, cx)
                                        })),
                                )
                            })
                            .child(
                                Button::new(SharedString::from(format!("scout-save-{id}")))
                                    .small()
                                    .when(status == OpportunityStatus::Saved, |button| {
                                        button.primary()
                                    })
                                    .when(status != OpportunityStatus::Saved, |button| {
                                        button.ghost()
                                    })
                                    .label(if busy {
                                        "Saving…"
                                    } else if status == OpportunityStatus::Saved {
                                        "Saved · Restore"
                                    } else {
                                        "Save"
                                    })
                                    .disabled(busy)
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        let next = if status == OpportunityStatus::Saved {
                                            OpportunityStatus::New
                                        } else {
                                            OpportunityStatus::Saved
                                        };
                                        page.set_status(save_id.clone(), next, cx);
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("scout-dismiss-{id}")))
                                    .small()
                                    .ghost()
                                    .label(if status == OpportunityStatus::Dismissed {
                                        "Restore"
                                    } else {
                                        "Dismiss"
                                    })
                                    .disabled(busy)
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        let next = if status == OpportunityStatus::Dismissed {
                                            OpportunityStatus::New
                                        } else {
                                            OpportunityStatus::Dismissed
                                        };
                                        page.set_status(dismiss_id.clone(), next, cx);
                                    })),
                            ),
                    ),
            );
        }

        let mut content = div()
            .key_context("ScoutingPage")
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Discover"),
                    )
                    .child(
                        Button::new("scout-refresh")
                            .small()
                            .ghost()
                            .label("↻ Refresh")
                            .disabled(loading || scanning)
                            .on_click(cx.listener(|page, _, _, cx| page.load(cx))),
                    ),
            )
            .child(source_picker)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(muted("Location · optional", palette))
                            .child(Input::new(&location).small()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(muted("Search term · optional", palette))
                            .child(Input::new(&query).small()),
                    )
                    .child(
                        Button::new("scout-run")
                            .small()
                            .primary()
                            .label(if scanning {
                                "Scanning…"
                            } else {
                                "Scan source"
                            })
                            .disabled(scanning || loading || self.adapter.is_none())
                            .on_click(cx.listener(|page, _, _, cx| page.scan(cx))),
                    ),
            );

        if let Some(form) = self.render_promotion(palette, cx) {
            content = content.child(form);
        }
        if let Some(error) = error {
            content = content.child(
                div()
                    .p_3()
                    .rounded(px(6.))
                    .bg(rgba(palette.surface))
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error),
            );
        }
        if let Some(notice) = notice {
            content = content.child(muted(notice, palette));
        }
        if let Some(scan) = scan {
            content = content.child(muted(
                format!(
                    "Last scan: {} scored · {} new · {} Obsidian links",
                    scan.total_scored, scan.new_count, scan.vault_links
                ),
                palette,
            ));
        }
        let filters = StatusFilter::ALL.into_iter().fold(
            div().flex().flex_wrap().items_center().gap_2(),
            |row, filter| {
                let active = self.status_filter == filter;
                row.child(
                    Button::new(SharedString::from(format!("scout-filter-{:?}", filter)))
                        .small()
                        .label(format!("{} · {}", filter.label(), self.count(filter)))
                        .when(active, |button| button.primary())
                        .when(!active, |button| button.ghost())
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.status_filter = filter;
                            page.shown = PAGE_SIZE;
                            page.scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                        })),
                )
            },
        );
        content = content.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(filters)
                .child(muted(
                    format!(
                        "{} loaded · {} stored",
                        self.opportunities.len(),
                        store_total
                    ),
                    palette,
                )),
        );
        if loading {
            content = content.child(muted("Loading sources and opportunities…", palette));
        } else if self.sources.is_empty() {
            content = content.child(muted("No active Scouting source is configured.", palette));
        } else if total_visible == 0 {
            content = content.child(muted(
                if self.status_filter == StatusFilter::New {
                    "No new opportunities. Scan a source or open saved entries."
                } else {
                    "No entries with this status."
                },
                palette,
            ));
        } else {
            content = content.child(
                div()
                    .id("scout-opportunities-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .vertical_scrollbar(&self.scroll)
                    .child(cards),
            );
            if visible_count < total_visible {
                content = content.child(
                    Button::new("scout-show-more")
                        .small()
                        .ghost()
                        .label(format!(
                            "Show more ({} remaining)",
                            total_visible - visible_count
                        ))
                        .on_click(cx.listener(|page, _, _, cx| {
                            page.shown = page.shown.saturating_add(PAGE_SIZE);
                            cx.notify();
                        })),
                );
            }
        }
        content
    }
}
