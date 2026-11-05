//! Native Scouting source discovery and opportunity review.

pub mod card;
pub mod promotion;
pub mod types;

pub use types::*;

use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, point, prelude::*, px, rgb, rgba, Context, Entity, FontWeight, IntoElement, Render,
    ScrollHandle, SharedString, Subscription, Window,
};
use sjel_gpui_core::scouting::{
    can_promote_to_calendar, CalendarCandidate, CalendarCandidateVerdict, DiscoverResponse,
    OpportunityStatus, ScoutingOpportunity, ScoutingSource,
};
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::Repo;
use crate::theme::Palette;

pub struct ScoutingPage {
    pub repo: Repo,
    pub palette: Option<Palette>,
    pub sources: Vec<ScoutingSource>,
    pub opportunities: Vec<ScoutingOpportunity>,
    pub store_total: usize,
    pub adapter: Option<String>,
    pub loading: bool,
    pub scanning: bool,
    pub busy: Option<String>,
    pub error: Option<SharedString>,
    pub notice: Option<SharedString>,
    pub scan: Option<DiscoverResponse>,
    pub calendar_verdicts: HashMap<String, CalendarCandidateVerdict>,
    pub(crate) verdict_request: u64,
    pub status_filter: StatusFilter,
    pub shown: usize,
    pub(crate) location: Option<(Entity<InputState>, Subscription)>,
    pub(crate) query: Option<(Entity<InputState>, Subscription)>,
    pub(crate) promotion: Option<PromotionForm>,
    pub(crate) scroll: ScrollHandle,
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

    pub(crate) fn load_calendar_verdicts(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn set_status(
        &mut self,
        id: String,
        status: OpportunityStatus,
        cx: &mut Context<Self>,
    ) {
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

    pub(crate) fn set_local_status(&mut self, id: &str, status: OpportunityStatus) {
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
            cards = cards.child(self.render_opportunity_card(opportunity, palette, cx));
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
