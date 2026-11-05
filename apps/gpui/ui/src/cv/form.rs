//! CV New application capture form and inputs.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, Entity, SharedString, Window};
use sjel_gpui_core::cv::{slug_for, CvView, CvWriteRequest};

use crate::primitives::card;
use crate::theme::Palette;

use super::CvPage;

impl CvPage {
    pub(super) fn ensure_company_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.company_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Company name…"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.company_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn ensure_role_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.role_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Role / Title…"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.role_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn ensure_url_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.url_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Posting URL (optional)…"));
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

    pub(super) fn render_new_application_panel(
        &mut self,
        view: &CvView,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_busy = !self.busy.is_empty();

        let company_field = self.ensure_company_input(window, cx);
        let role_field = self.ensure_role_input(window, cx);
        let url_field = self.ensure_url_input(window, cx);

        let company_val = company_field.read(cx).value().trim().to_string();
        let role_val = role_field.read(cx).value().trim().to_string();
        let url_val = url_field.read(cx).value().trim().to_string();

        let can_submit = !company_val.is_empty() && !role_val.is_empty();

        let draft_profile = self.draft_profile.clone();
        let draft_lang = self.draft_lang.clone();

        let profile_btns = view.profiles.iter().cloned().map(|prof| {
            let is_active = prof == draft_profile;
            let prof_c = prof.clone();
            Button::new(SharedString::from(format!("dprof-{}", prof)))
                .small()
                .when(!is_active, |b| b.ghost())
                .label(prof)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.draft_profile = prof_c.clone();
                    cx.notify();
                }))
        });

        let dlang_en = Button::new("dlang-en")
            .small()
            .when(draft_lang != "en", |b| b.ghost())
            .label("en")
            .on_click(cx.listener(|this, _, _, cx| {
                this.draft_lang = "en".to_string();
                cx.notify();
            }));

        let dlang_de = Button::new("dlang-de")
            .small()
            .when(draft_lang != "de", |b| b.ghost())
            .label("de")
            .on_click(cx.listener(|this, _, _, cx| {
                this.draft_lang = "de".to_string();
                cx.notify();
            }));

        let submit_btn = Button::new("btn-submit-app")
            .small()
            .label("Capture Application")
            .disabled(is_busy || !can_submit)
            .on_click(cx.listener({
                let company = company_val.clone();
                let role = role_val.clone();
                let url = url_val.clone();
                let profile = draft_profile.clone();
                let lang = draft_lang.clone();
                let slug = slug_for(&company, &role);
                let comp_input_e = company_field.clone();
                let role_input_e = role_field.clone();
                let url_input_e = url_field.clone();

                move |this, _, window, cx| {
                    comp_input_e.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                    });
                    role_input_e.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                    });
                    url_input_e.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                    });

                    this.run_action(
                        CvWriteRequest {
                            action: "app-new".to_string(),
                            company: company.clone(),
                            role: role.clone(),
                            url: url.clone(),
                            profile: profile.clone(),
                            lang: lang.clone(),
                            slug: slug.clone(),
                            ..Default::default()
                        },
                        window,
                        cx,
                    );
                }
            }));

        let content = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Add New Application"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(div().w(px(220.0)).child(Input::new(&company_field)))
                    .child(div().w(px(220.0)).child(Input::new(&role_field)))
                    .child(div().flex_1().child(Input::new(&url_field))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Profile:"),
                            )
                            .children(profile_btns)
                            .child(
                                div()
                                    .ml(px(8.0))
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Lang:"),
                            )
                            .child(dlang_en)
                            .child(dlang_de),
                    )
                    .child(submit_btn),
            );

        card(content.into_any_element(), palette)
    }
}
