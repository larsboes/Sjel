//! CV New application capture form and inputs.
use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::forms::{Form, FormField, Input, InputEvent, TextInput};
use crate::ely::theme::ControlSize;
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
    ) -> Entity<TextInput> {
        if let Some((input, _)) = &self.company_input {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("Company name…"));
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
    ) -> Entity<TextInput> {
        if let Some((input, _)) = &self.role_input {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("Role / Title…"));
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
    ) -> Entity<TextInput> {
        if let Some((input, _)) = &self.url_input {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("Posting URL (optional)…"));
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

    fn capture_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.busy.is_empty() {
            return;
        }
        let company = self
            .ensure_company_input(window, cx)
            .read(cx)
            .text()
            .trim()
            .to_owned();
        let role = self
            .ensure_role_input(window, cx)
            .read(cx)
            .text()
            .trim()
            .to_owned();
        if company.is_empty() || role.is_empty() {
            return;
        }
        let url = self
            .ensure_url_input(window, cx)
            .read(cx)
            .text()
            .trim()
            .to_owned();
        self.run_action(
            CvWriteRequest {
                action: "app-new".into(),
                slug: slug_for(&company, &role),
                company,
                role,
                url,
                profile: self.draft_profile.clone(),
                lang: self.draft_lang.clone(),
                ..Default::default()
            },
            window,
            cx,
        );
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

        let company_val = company_field.read(cx).text().trim().to_string();
        let role_val = role_field.read(cx).text().trim().to_string();

        let can_submit = !company_val.is_empty() && !role_val.is_empty();

        let draft_profile = self.draft_profile.clone();
        let draft_lang = self.draft_lang.clone();

        let profile_btns = view.profiles.iter().cloned().map(|prof| {
            let is_active = prof == draft_profile;
            let prof_c = prof.clone();
            Button::new(SharedString::from(format!("dprof-{}", prof)), prof)
                .size(ControlSize::Sm)
                .when(!is_active, |b| b.variant(ButtonVariant::Ghost))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.draft_profile = prof_c.clone();
                    cx.notify();
                }))
        });

        let dlang_en = Button::new("dlang-en", "English")
            .size(ControlSize::Sm)
            .when(draft_lang != "en", |b| b.variant(ButtonVariant::Ghost))
            .on_click(cx.listener(|this, _, _, cx| {
                this.draft_lang = "en".to_string();
                cx.notify();
            }));

        let dlang_de = Button::new("dlang-de", "German")
            .size(ControlSize::Sm)
            .when(draft_lang != "de", |b| b.variant(ButtonVariant::Ghost))
            .on_click(cx.listener(|this, _, _, cx| {
                this.draft_lang = "de".to_string();
                cx.notify();
            }));

        let submit_btn = Button::new("btn-submit-app", "Capture Application")
            .size(ControlSize::Sm)
            .primary()
            .disabled(is_busy || !can_submit)
            .on_click(cx.listener(|this, _, window, cx| {
                this.capture_application(window, cx);
            }));

        let page = cx.entity();
        let content = Form::new("cv-new-application")
            .on_submit(move |window, cx| {
                page.update(cx, |page, cx| page.capture_application(window, cx))
            })
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
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(
                        div().w(px(220.0)).child(
                            FormField::new("cv-company", "Company")
                                .required()
                                .child(Input::new(&company_field).size(ControlSize::Sm)),
                        ),
                    )
                    .child(
                        div().w(px(220.0)).child(
                            FormField::new("cv-role", "Role / Title")
                                .required()
                                .child(Input::new(&role_field).size(ControlSize::Sm)),
                        ),
                    )
                    .child(
                        div().flex_1().min_w(px(200.)).child(
                            FormField::new("cv-url", "Posting URL")
                                .child(Input::new(&url_field).size(ControlSize::Sm)),
                        ),
                    ),
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
                    .child(
                        div()
                            .debug_selector(|| "cv-capture-submit".into())
                            .child(submit_btn),
                    ),
            );

        card(content.into_any_element(), palette)
    }
}
