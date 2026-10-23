//! Lazy detail reads retain a previous answer when a refresh fails.
use super::Travel;
use gpui::Context;
use sjel_gpui_core::{
    shell,
    travel::{self, SavedDetails},
};

#[derive(Default)]
pub(super) struct DetailState {
    pub value: Option<SavedDetails>,
    pub error: Option<String>,
    pub loading: bool,
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
                match travel::save_details(details.clone()).await {
                    Ok(saved) => Ok((saved, None)),
                    Err(error) => Ok((
                        SavedDetails {
                            saved_at: chrono::Utc::now().timestamp().max(0) as u64,
                            details,
                        },
                        Some(error),
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
                match result {
                    Ok((saved, warning)) => {
                        slot.value = Some(saved);
                        slot.error = warning;
                    }
                    Err(error) => slot.error = Some(error),
                }
                view.follow_detail(&id, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn follow_detail(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.selected.as_deref() != Some(id) {
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
        if self.fit_pending {
            self.fit_trip(cx);
        }
        self.resolve_coordinates(cx);
    }
}
