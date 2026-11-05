//! Machine requirements and Turso replica configuration tab for Setup.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, Window};
use sjel_gpui_core::setup::{
    has_apply_input, merge_verification_probes, ProbeMark, SetupApplyRequest, SetupPlan,
};

use crate::data;
use crate::primitives::{card, dot, note};
use crate::theme::Palette;

use super::SetupPage;

impl SetupPage {
    pub(super) fn ensure_url_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.url_input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("libsql://your-database.turso.io"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.url_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn ensure_credential_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.credential_input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Primary auth token (piped to stdin, value hidden)")
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.credential_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn ensure_interval_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.interval_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("60"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.interval_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn verify_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        self.verifying = true;
        self.notice = None;
        self.write_error = None;

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(
                |s| &mut s.busy,
                "setup:verify",
                "Primary connection verified.",
            ),
            |shell| async move { shell.setup_verify().await },
            |this, result, cx| {
                this.verifying = false;
                match result {
                    Some(verification) => {
                        if let Some(view) = &mut this.setup.value {
                            view.plans = merge_verification_probes(&view.plans, &verification);
                        }
                        this.notice = Some("Primary connection verified.".to_string());
                    }
                    None => {
                        this.write_error =
                            Some("Connection check failed or could not reach primary.".to_string());
                    }
                }
                cx.notify();
            },
        );
    }

    pub(super) fn apply_setup(
        &mut self,
        component: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let url_field = self.ensure_url_input(window, cx);
        let cred_field = self.ensure_credential_input(window, cx);
        let int_field = self.ensure_interval_input(window, cx);

        let url = url_field.read(cx).value().trim().to_string();
        let token = cred_field.read(cx).value().trim().to_string();
        let interval_str = int_field.read(cx).value().trim().to_string();

        if !has_apply_input(&url, &token, &interval_str) {
            self.write_error = Some("Supply a URL, credential, or interval to apply.".to_string());
            cx.notify();
            return;
        }

        let interval_secs = interval_str.parse::<u64>().ok();

        let req = SetupApplyRequest {
            component: Some(component.clone()),
            url: if url.is_empty() { None } else { Some(url) },
            interval_secs,
            token: if token.is_empty() { None } else { Some(token) },
        };

        let repo = self.repo.clone();
        let key = format!("setup:apply:{}", component);

        self.notice = None;
        self.write_error = None;
        self.applying_component = Some(component);

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Setup steps applied."),
            move |shell| async move { shell.setup_apply(&req).await },
            |this, result, cx| {
                let component = this.applying_component.take();
                match result {
                    Some(applied) => {
                        let lines: Vec<String> = applied
                            .components
                            .into_iter()
                            .flat_map(|c| c.applied)
                            .collect();
                        this.applied_log = lines;
                        this.notice =
                            component.map(|name| format!("Applied setup settings for {name}."));
                        this.confirming_component = None;
                        // The credential is write-only: dropping the field is the only way this
                        // page can forget a value that must never be shown again.
                        this.credential_input = None;
                    }
                    None => {
                        this.write_error = Some("Failed to apply setup settings.".to_string());
                    }
                }
                this.load(cx);
            },
        );
    }

    pub(super) fn render_machine_tab(
        &mut self,
        plans: &[SetupPlan],
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if plans.is_empty() {
            return note(
                "Nothing to set up: no component has an active plan on this build.",
                palette.muted,
            );
        }

        let mut panels = Vec::new();

        // Action bar
        let action_bar = div()
            .flex()
            .justify_between()
            .items_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(dot(palette.accent))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child("What this machine still needs"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        Button::new("setup-re-read")
                            .small()
                            .ghost()
                            .label("Re-read")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load(cx);
                            })),
                    )
                    .child(
                        Button::new("setup-verify")
                            .small()
                            .ghost()
                            .label(if self.verifying {
                                "Checking…"
                            } else {
                                "Check the connection"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.verify_connection(window, cx);
                            })),
                    ),
            );

        panels.push(card(action_bar.into_any_element(), palette));

        // Applied log
        if !self.applied_log.is_empty() {
            let log_lines = self.applied_log.iter().map(|line| {
                div()
                    .text_size(px(11.0))
                    .font_family(".AppleSystemUIFontMonospaced")
                    .text_color(rgb(palette.text))
                    .child(line.clone())
                    .into_any_element()
            });

            let log_card = div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .p(px(8.0))
                .bg(rgba(palette.surface))
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(palette.green))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(palette.green))
                        .child("Recently Applied Settings:"),
                )
                .children(log_lines);

            panels.push(log_card.into_any_element());
        }

        // Each plan card
        for plan in plans {
            let comp = plan.component.clone();
            let is_confirming = self.confirming_component.as_deref() == Some(&comp);

            // Probes list
            let probe_rows = plan.probes.iter().map(|probe| {
                let parsed = probe.parsed_mark();
                let mark_color = match parsed {
                    ProbeMark::Ok => palette.green,
                    ProbeMark::Missing => palette.red,
                    ProbeMark::Warn => palette.amber,
                    ProbeMark::Blocked => palette.muted,
                };

                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .p(px(8.0))
                    .bg(rgba(palette.surface))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .w(px(20.0))
                            .text_size(px(13.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(mark_color))
                            .child(parsed.symbol()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette.text))
                                            .child(probe.label.clone()),
                                    )
                                    .child(
                                        div()
                                            .px(px(5.0))
                                            .py(px(1.0))
                                            .bg(rgba(palette.raised))
                                            .rounded(px(3.0))
                                            .text_size(px(9.0))
                                            .text_color(rgb(mark_color))
                                            .child(parsed.human_label()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .line_height(px(16.0))
                                    .child(probe.detail.clone()),
                            ),
                    )
                    .into_any_element()
            });

            // Steps list
            let step_rows = plan.steps.iter().map(|step| {
                let (sym, col) = if step.done {
                    ("✓", palette.green)
                } else {
                    ("·", palette.muted)
                };

                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .p(px(6.0))
                    .child(
                        div()
                            .w(px(16.0))
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(col))
                            .child(sym),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(if step.done {
                                        rgb(palette.muted)
                                    } else {
                                        rgb(palette.text)
                                    })
                                    .child(step.label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child(step.detail.clone()),
                            ),
                    )
                    .into_any_element()
            });

            // Form or trigger
            let form_or_button = if is_confirming {
                let url_field = self.ensure_url_input(window, cx);
                let cred_field = self.ensure_credential_input(window, cx);
                let int_field = self.ensure_interval_input(window, cx);
                let comp_for_apply = comp.clone();

                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .p(px(12.0))
                    .bg(rgba(palette.surface))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(palette.muted))
                            .line_height(px(17.0))
                            .child(
                                "Sjel writes the settings to the private overlay and restarts what follows the primary. \
                                Piped to tool stdin, never logged, never shown again.",
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Primary URL"),
                            )
                            .child(Input::new(&url_field)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Credential / Auth Token"),
                            )
                            .child(Input::new(&cred_field)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Pull Interval, seconds (default 60)"),
                            )
                            .child(Input::new(&int_field)),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .pt(px(4.0))
                            .child(
                                Button::new("cancel-apply")
                                    .small()
                                    .ghost()
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.confirming_component = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("submit-apply")
                                    .small()
                                    .label("Apply Settings")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.apply_setup(comp_for_apply.clone(), window, cx);
                                    })),
                            ),
                    )
                    .into_any_element()
            } else {
                let comp_to_open = comp.clone();
                div()
                    .flex()
                    .justify_start()
                    .pt(px(4.0))
                    .child(
                        Button::new(format!("btn-open-setup-{}", comp))
                            .small()
                            .label(format!("Configure {}", comp))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.confirming_component = Some(comp_to_open.clone());
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            };

            let plan_card = div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .border_b_1()
                        .border_color(rgba(palette.border))
                        .pb(px(8.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(15.0))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(rgb(palette.text))
                                        .child(plan.component.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(rgb(palette.muted))
                                        .child(plan.purpose.clone()),
                                ),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .bg(rgba(palette.surface))
                                .rounded(px(4.0))
                                .text_size(px(10.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.accent))
                                .child(format!("{} open steps", plan.applyable_steps())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child("Machine Probes"),
                        )
                        .children(probe_rows),
                )
                .when(!plan.steps.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child("Plan Steps"),
                            )
                            .children(step_rows),
                    )
                })
                .child(form_or_button);

            panels.push(card(plan_card.into_any_element(), palette));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .children(panels)
            .into_any_element()
    }
}
