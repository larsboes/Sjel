//! Calendar promotion form management and modal view.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, point, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString,
    Window,
};
use sjel_gpui_core::scouting::{
    calendar_entry_body, calendar_start_input, default_calendar_end, OpportunityStatus,
    ScoutingOpportunity,
};
use sjel_gpui_core::shell::{self, ShellError};

use crate::theme::Palette;

use super::types::{muted, PromotionForm};
use super::ScoutingPage;

impl ScoutingPage {
    pub(super) fn open_promotion(
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

    pub(super) fn close_promotion(&mut self, cx: &mut Context<Self>) {
        self.promotion = None;
        cx.notify();
    }

    pub(super) fn save_promotion(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn render_promotion(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
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
