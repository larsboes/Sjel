//! Storage inspection, historical retention trend, and system package updates panels.
use chrono::Local;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, IntoElement, SharedString, Window,
};

use crate::data;
use crate::primitives::{list, note, row, section};
use crate::theme::{tint, Palette};
use sjel_gpui_core::systems::{
    apply_summary, audit_needs_attention, dated_absences, storage_bytes, version_label, Severity,
    StorageTrend, UpdateStatus, UpdateSurface,
};

use super::overview::bar;
use super::types::{APPLY_BEAT, APPLY_BEATS};
use super::{heading, SystemsPage};

impl SystemsPage {
    pub(super) fn read_storage(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read_quiet(
            cx,
            self,
            |s| &mut s.storage,
            &repo,
            |shell| async move { shell.storage().await },
        );
    }

    pub(super) fn read_history(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read_quiet(
            cx,
            self,
            |s| &mut s.history,
            &repo,
            |shell| async move { shell.storage_history().await },
        );
    }

    pub(super) fn read_updates(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read_quiet(
            cx,
            self,
            |s| &mut s.updates,
            &repo,
            |shell| async move { shell.updates().await },
        );
    }

    pub(super) fn apply_updates(
        &mut self,
        class: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let key = format!("updates:apply:{class}");
        let sent = class.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Apply started."),
            move |shell| async move { shell.apply_updates(&sent).await },
            |this, started, cx| {
                if started.is_some() {
                    this.watch_apply(cx);
                }
                this.read_updates(cx);
            },
        );
    }

    /// The report *is* the progress: the route answered `202` and the tool writes a receipt the
    /// next report carries. Bounded, so a shell that stops answering cannot leave a poll running.
    pub(super) fn watch_apply(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        cx.spawn(async move |weak, cx| {
            for _ in 0..APPLY_BEATS {
                cx.background_executor().timer(APPLY_BEAT).await;
                let running = weak
                    .update(cx, |page, cx| {
                        if page.generation != generation {
                            return false;
                        }
                        let running = page
                            .updates
                            .value
                            .as_ref()
                            .and_then(|report| report.last_apply.as_ref())
                            .and_then(|receipt| receipt.state.as_deref())
                            == Some("running");
                        if running {
                            page.read_updates(cx);
                        }
                        running
                    })
                    .unwrap_or(false);
                if !running {
                    return;
                }
            }
        })
        .detach();
    }

    pub(super) fn storage_panel(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        self.settle_derived();
        let body = match self.derived.storage_view.as_ref() {
            None => match &self.storage.error {
                // The report comes from sjel-status rather than macmon, so it answers even when
                // macmon is down — which is the state a full disk is hardest to see in anyway.
                Some(error) => note(error.to_string(), palette.red),
                None if self.storage.loading => {
                    note("Measuring policy-managed paths…", palette.muted)
                }
                None => note("Policy sizes are measured only on request.", palette.muted),
            },
            Some(view) => {
                let mut rows: Vec<AnyElement> = Vec::new();
                let tone = match view.severity {
                    Severity::Ok => palette.green,
                    Severity::Warn => palette.amber,
                    Severity::Critical => palette.red,
                };
                rows.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(bar(view.used_pct, tone, palette))
                                .child(
                                    div()
                                        .w(px(72.))
                                        .text_xs()
                                        .text_color(rgb(tone))
                                        .child(view.state_label.clone()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("{} on this volume · {} capacity", view.used, view.total))
                                .child(format!("{} free", view.free)),
                        )
                        .children(view.other_allocation_label.as_ref().map(|label| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("{label} outside this volume or reserved by filesystem"))
                        }))
                        .children(view.reconciliation.clone().map(|(named, unattributed)| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "Policy comparison only (classes can overlap; not a physical ledger): {named} named · {unattributed} unattributed"
                                ))
                        }))
                        .into_any_element(),
                );

                rows.push(trend_row(
                    self.derived.storage_trend.as_ref(),
                    self.history.value.as_ref().map_or(0, |h| h.retention_days),
                    palette,
                ));

                if view.classes.is_empty() {
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Nothing measured in any class.")
                            .into_any_element(),
                    );
                } else {
                    let deltas = &self.derived.class_deltas;
                    let largest = view.scale_bytes;
                    rows.push(heading("Policy-managed storage", palette));
                    let class_rows: Vec<AnyElement> = view
                        .classes
                        .iter()
                        .map(|class| {
                            let share = class.bytes as f32 / largest as f32;
                            let color = if class.flagged {
                                palette.amber
                            } else {
                                palette.accent
                            };
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_3()
                                .py_1()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(div().truncate().text_sm().child(class.name.clone()))
                                        .child(
                                            div()
                                                .flex()
                                                .gap_2()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .children(
                                                    (!class.applicable)
                                                        .then(|| "report-only".to_owned()),
                                                )
                                                .children(
                                                    class.flagged.then(|| "over flag".to_owned()),
                                                )
                                                .children(
                                                    (class.measured == Some(false))
                                                        .then(|| "partial".to_owned()),
                                                ),
                                        ),
                                )
                                .child(div().w(px(160.)).child(bar(share, color, palette)))
                                .child(
                                    div()
                                        .w(px(150.))
                                        .text_xs()
                                        .text_color(rgb(palette.text))
                                        .child(format!(
                                            "{}{}",
                                            storage_bytes(class.bytes),
                                            deltas
                                                .get(&class.name)
                                                .map(|delta| format!(" ({delta})"))
                                                .unwrap_or_default()
                                        )),
                                )
                                .into_any_element()
                        })
                        .collect();
                    rows.push(list(class_rows, palette));
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Classes can overlap. Review candidates with `sjel storage reclaim --dry-run`.")
                            .into_any_element(),
                    );
                }

                if !view.unmeasured.is_empty() {
                    let names: Vec<String> = view
                        .unmeasured
                        .iter()
                        .map(|class| class.name.clone())
                        .collect();
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child(format!(
                                "{} name a path that is here and could not be read, so no size is reported rather than a zero: {}",
                                names.len(),
                                names.join(", ")
                            ))
                            .into_any_element(),
                    );
                }
                if !view.classes_absent.is_empty() {
                    let names: Vec<String> = view
                        .classes_absent
                        .iter()
                        .map(|class| class.name.clone())
                        .collect();
                    let dated = dated_absences(&names, "class", self.history.value.as_ref());
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child(format!(
                                "The policy names a path that is no longer on this machine: {}",
                                dated.join(", ")
                            ))
                            .into_any_element(),
                    );
                }

                if !view.protected.is_empty() {
                    rows.push(heading("Protected — reported, never cleaned", palette));
                    let protected: Vec<AnyElement> = view
                        .protected
                        .iter()
                        .map(|entry| {
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .px_3()
                                .py_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_3()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_sm()
                                                .child(entry.path.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.text))
                                                .child(storage_bytes(entry.bytes)),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(entry.reason.clone()),
                                )
                                .into_any_element()
                        })
                        .collect();
                    rows.push(list(protected, palette));
                }

                if view.protected_absent > 0 {
                    let dated = dated_absences(
                        &view.protected_absent_paths,
                        "protected",
                        self.history.value.as_ref(),
                    );
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child(format!(
                                "{} of these name a path that is no longer on this machine — {} — so the reason beside it was written when it still existed.",
                                view.protected_absent,
                                dated.join(", ")
                            ))
                            .into_any_element(),
                    );
                }
                if view.protected_unreadable > 0 {
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} of these could not be read, so their size reads 0 MB.",
                                view.protected_unreadable
                            ))
                            .into_any_element(),
                    );
                }
                if !view.services_absent.is_empty() {
                    let dated = dated_absences(
                        &view.services_absent,
                        "service",
                        self.history.value.as_ref(),
                    );
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child(format!(
                                "{} declared service(s) not running — {} — so the policy expects something the machine is not running.",
                                view.services_absent.len(),
                                dated.join(", ")
                            ))
                            .into_any_element(),
                    );
                }

                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(rows)
                    .into_any_element()
            }
        };
        let explorer = self.explorer_panel(palette, cx);
        section(
            "Storage",
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(explorer)
                .child(
                    Button::new("storage-policy-measure")
                        .small()
                        .ghost()
                        .label(if self.storage.loading {
                            "Measuring policy paths…"
                        } else {
                            "Measure policy paths"
                        })
                        .disabled(self.storage.loading)
                        .on_click(cx.listener(|page, _, _, cx| page.read_storage(cx))),
                )
                .children(
                    self.storage
                        .value
                        .as_ref()
                        .and(self.storage.error.as_ref())
                        .map(|error| {
                            note(
                                format!("Policy refresh failed; previous report retained: {error}"),
                                palette.red,
                            )
                        }),
                )
                .child(body)
                .into_any_element(),
            palette,
        )
        .into_any_element()
    }

    pub(super) fn updates_panel(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        self.settle_derived();
        let view = self.derived.updates_view.clone();
        let body = match &view {
            None => match &self.updates.error {
                Some(error) => note(error.to_string(), palette.red),
                None => note("Asking three registries…", palette.muted),
            },
            Some(view) => {
                let summary = apply_summary(view, Local::now());
                let mut rows: Vec<AnyElement> = Vec::new();
                if !summary.is_empty() {
                    let attention = audit_needs_attention(
                        view.last_apply
                            .as_ref()
                            .and_then(|last| last.audit.as_deref()),
                    );
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(if attention {
                                palette.amber
                            } else {
                                palette.muted
                            }))
                            .child(summary)
                            .into_any_element(),
                    );
                }
                rows.push(
                    div()
                        .text_xs()
                        .text_color(rgb(if view.stale > 0 {
                            palette.amber
                        } else {
                            palette.muted
                        }))
                        .child(if view.stale > 0 {
                            format!("{} stale", view.stale)
                        } else {
                            "nothing stale".to_owned()
                        })
                        .into_any_element(),
                );

                for group in &view.groups {
                    let surface: UpdateSurface = group.surface.clone();
                    let actionable = group.actionable;
                    let busy =
                        view.busy || self.busy.contains(&format!("updates:apply:{}", surface.id));
                    rows.push(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .pt_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(surface.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(surface.owner_detail.clone()),
                                    ),
                            )
                            .when(actionable > 0 && !busy, |header| {
                                header.child(
                                    Button::new(SharedString::from(format!(
                                        "apply-{}",
                                        surface.id
                                    )))
                                    .small()
                                    .label(format!("Apply {actionable}"))
                                    .on_click(cx.listener(move |page, _, window, cx| {
                                        page.apply_updates(surface.id.clone(), window, cx);
                                    })),
                                )
                            })
                            .into_any_element(),
                    );

                    let update_rows: Vec<AnyElement> = group
                        .rows
                        .iter()
                        .map(|entry| {
                            let (tone, mark) = match entry.status() {
                                UpdateStatus::Stale => (palette.amber, "!"),
                                UpdateStatus::Current => (palette.green, "✓"),
                                UpdateStatus::Unknown => (palette.muted, "?"),
                                UpdateStatus::NotApplicable => (palette.muted, "·"),
                            };
                            row()
                                .text_xs()
                                .child(div().w(px(14.)).text_color(rgb(tone)).child(mark))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .child(entry.name.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(150.))
                                        .truncate()
                                        .text_color(rgb(palette.muted))
                                        .child(version_label(entry)),
                                )
                                .child(
                                    div()
                                        .w(px(240.))
                                        .truncate()
                                        .text_color(rgb(palette.muted))
                                        .child(entry.note.clone().unwrap_or_default()),
                                )
                                .into_any_element()
                        })
                        .collect();
                    rows.push(list(update_rows, palette));
                }

                if view.unknown > 0 {
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} row(s) were not checked — not stale, but not confirmed current either.",
                                view.unknown
                            ))
                            .into_any_element(),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(rows)
                    .into_any_element()
            }
        };
        section("Updates", body, palette).into_any_element()
    }
}

pub(super) fn trend_row(trend: Option<&StorageTrend>, days: u32, palette: Palette) -> AnyElement {
    let Some(trend) = trend else {
        return div().into_any_element();
    };
    if !trend.enough {
        return div()
            .text_xs()
            .text_color(rgb(palette.muted))
            .child(format!(
                "Growth history starts after the first hourly sample; {days} days are kept."
            ))
            .into_any_element();
    }
    let bars: Vec<AnyElement> = trend
        .bars
        .iter()
        .map(|share| {
            let height = (share * 24.0).max(2.0);
            div()
                .w(px(2.))
                .h(px(height))
                .rounded(px(1.))
                .bg(rgba(tint(palette.accent, 0xaa)))
                .into_any_element()
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_end()
                .gap(px(1.))
                .h(px(24.))
                .children(bars),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "{} used · {} unattributed over {}",
                    trend.delta_label, trend.unattributed_delta_label, trend.span_label
                )),
        )
        .into_any_element()
}
