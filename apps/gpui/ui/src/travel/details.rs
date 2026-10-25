//! Lazy detail reads retain a previous answer when a refresh fails.
use super::Travel;
use gpui::Context;
use sjel_gpui_core::{
    shell::{self, CalendarEntry},
    travel::{self, SavedDetails},
};

#[derive(Default)]
pub(super) struct DetailState {
    pub value: Option<SavedDetails>,
    pub error: Option<String>,
    pub loading: bool,
    pub calendar_events: Vec<CalendarEntry>,
    pub calendar_events_loaded: bool,
    pub calendar_events_loading: bool,
    pub calendar_events_error: Option<String>,
    pub calendar_links: Vec<sjel_gpui_core::record::Link>,
    pub calendar_links_loaded: bool,
    pub calendar_links_loading: bool,
    pub calendar_links_error: Option<String>,
    loaded: bool,
}
impl Travel {
    pub(super) fn read_details(&mut self, id: String, force: bool, cx: &mut Context<Self>) {
        let slot = self.details.entry(id.clone()).or_default();
        if slot.loading || (slot.loaded && !force) {
            return;
        }
        slot.loading = true;
        slot.error = None;
        slot.calendar_events_loading = true;
        slot.calendar_events_loaded = false;
        slot.calendar_events_error = None;
        slot.calendar_links_loading = true;
        slot.calendar_links_loaded = false;
        slot.calendar_links_error = None;
        let read_saved = slot.value.is_none();
        let source = self.source.clone();
        let cached_id = id.clone();
        let cached = shell::run(async move {
            if read_saved {
                travel::saved_details(&cached_id).await
            } else {
                Ok(None)
            }
        });
        cx.spawn(async move |this, cx| {
            let cached = cached.await;
            if this
                .update(cx, |view, cx| {
                    let slot = view.details.entry(id.clone()).or_default();
                    match cached {
                        Some(Ok(Some(saved))) => slot.value = Some(saved),
                        Some(Err(error)) => slot.error = Some(error),
                        None => slot.error = Some("Reading saved trip details stopped.".into()),
                        _ => {}
                    }
                    view.follow_detail(&id, cx);
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            let requested = id.clone();
            let work = shell::run(async move {
                let source = source.ok_or("Connect to your Mac to update this trip's details.")?;
                let details = source.details(&requested).await?;
                let calendar_events = source.related_calendar_events(&details.plan).await;
                let calendar_links = source.calendar_plan_links(&details.plan.id).await;
                match travel::save_details(details.clone()).await {
                    Ok(saved) => Ok((saved, None, calendar_events, calendar_links)),
                    Err(error) => Ok((
                        SavedDetails {
                            saved_at: chrono::Utc::now().timestamp().max(0) as u64,
                            details,
                        },
                        Some(error),
                        calendar_events,
                        calendar_links,
                    )),
                }
            });
            let result: Result<_, String> = work
                .await
                .unwrap_or_else(|| Err("Loading trip details stopped.".into()));
            let _ = this.update(cx, |view, cx| {
                let slot = view.details.entry(id.clone()).or_default();
                slot.loading = false;
                slot.loaded = true;
                slot.calendar_events_loading = false;
                slot.calendar_events_loaded = true;
                slot.calendar_links_loading = false;
                match result {
                    Ok((saved, warning, events, links)) => {
                        slot.value = Some(saved);
                        slot.error = warning;
                        match events {
                            Ok(events) => slot.calendar_events = events,
                            Err(error) => slot.calendar_events_error = Some(error),
                        }
                        slot.calendar_links_loaded = true;
                        match links {
                            Ok(links) => slot.calendar_links = links.links,
                            Err(error) => slot.calendar_links_error = Some(error),
                        }
                    }
                    Err(error) => {
                        slot.error = Some(error.clone());
                        slot.calendar_events_error = Some(error);
                        slot.calendar_links_loaded = true;
                        slot.calendar_links_error = slot.error.clone();
                    }
                }
                view.follow_detail(&id, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn follow_detail(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.selected.as_deref() != Some(id) && !matches!(self.route, super::TravelRoute::Map) {
            return;
        }
        // A detail read carries the plan's stages, so it is the first moment the trip's real
        // stops are known. Filling before fitting is what keeps the map off the world view.
        self.fill_known_coordinates();
        if self.center.is_none() {
            self.center = self
                .current()
                .and_then(|plan| plan.places().find_map(|p| p.coordinate()));
        }
        if self.fit_pending
            && (matches!(self.route, super::TravelRoute::Map)
                || self.selected.as_deref() == Some(id))
        {
            self.fit_trip(cx);
        }
        if matches!(self.route, super::TravelRoute::Map) {
            self.presence_loaded = false;
            self.load_people_presence(cx, false);
        }
        self.resolve_coordinates(cx);
    }
}
