//! CV: Profile variations, PDF builds, application ledger, and LinkedIn synchronization.
//!
//! Replaces the dashboard `/cv` route. Backed by `capabilities/cv/cv` via sjel-status.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, AnyElement, Context, Entity, IntoElement, SharedString,
    Subscription, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::theme::Palette;
use crate::ui::{card, dot, note, placeholder, section};
use sjel_gpui_core::cv::{
    count_stale_pdfs, filter_applications, last_status_date, pdf_for_profile, slug_for,
    CvApplication, CvView, CvWriteRequest,
};

pub struct CvPage {
    repo: Repo,
    cv: Remote<CvView>,
    busy: Busy,
    notice: Option<String>,
    write_error: Option<String>,
    output: Option<String>,

    selected_profile: String,
    selected_lang: String,
    filter_status: String,

    company_input: Option<(Entity<InputState>, Subscription)>,
    role_input: Option<(Entity<InputState>, Subscription)>,
    url_input: Option<(Entity<InputState>, Subscription)>,
    draft_profile: String,
    draft_lang: String,
}

impl CvPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            cv: Remote::default(),
            busy: Busy::default(),
            notice: None,
            write_error: None,
            output: None,

            selected_profile: "default".to_string(),
            selected_lang: "en".to_string(),
            filter_status: "all".to_string(),

            company_input: None,
            role_input: None,
            url_input: None,
            draft_profile: "default".to_string(),
            draft_lang: "en".to_string(),
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.cv,
            &repo,
            |shell| async move { shell.cv().await },
        );
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    fn ensure_company_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.company_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Company name…"));
        let subscription = cx.subscribe_in(&input, window, |_view, _input, _event: &InputEvent, _, cx| {
            cx.notify();
        });
        self.company_input = Some((input.clone(), subscription));
        input
    }

    fn ensure_role_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.role_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Role / Title…"));
        let subscription = cx.subscribe_in(&input, window, |_view, _input, _event: &InputEvent, _, cx| {
            cx.notify();
        });
        self.role_input = Some((input.clone(), subscription));
        input
    }

    fn ensure_url_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.url_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Posting URL (optional)…"));
        let subscription = cx.subscribe_in(&input, window, |_view, _input, _event: &InputEvent, _, cx| {
            cx.notify();
        });
        self.url_input = Some((input.clone(), subscription));
        input
    }

    fn run_action(
        &mut self,
        req: CvWriteRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = format!("cv:{}:{}", req.action, req.slug);
        let repo = self.repo.clone();
        let req_clone = req.clone();

        self.notice = None;
        self.write_error = None;

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Executing CV action…"),
            move |shell| async move { shell.write_cv(&req_clone).await },
            |this, result, cx| {
                match result {
                    Some(res) => {
                        let combined = format!("{}\n\n{}", res.detail, res.warnings)
                            .trim()
                            .to_string();
                        if !combined.is_empty() {
                            this.output = Some(combined);
                        }
                        this.notice = Some(format!("Action completed: {}", res.action));
                    }
                    None => {
                        this.write_error = Some("Failed to execute CV action.".to_string());
                    }
                }
                this.load(cx);
            },
        );
    }

    // ── Components ────────────────────────────────────────────────────────────

    fn render_master_header(&self, view: &CvView, palette: Palette) -> AnyElement {
        let mtime_str = if view.master_mtime > 0 {
            chrono::DateTime::from_timestamp(view.master_mtime as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| format!("epoch {}", view.master_mtime))
        } else {
            "unknown".to_string()
        };

        let stale_count = count_stale_pdfs(view);
        let linkedin_date = view
            .linkedin_pulled
            .as_deref()
            .unwrap_or("Never");

        let header_content = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
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
                                    .child("Master Curriculum Vitae"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(palette.muted))
                                    .child(format!("({})", view.master)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(palette.muted))
                                    .child(format!("Last edited: {}", mtime_str)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(if stale_count > 0 {
                                        rgb(palette.amber)
                                    } else {
                                        rgb(palette.green)
                                    })
                                    .child(format!(
                                        "{} PDF(s) built ({} stale)",
                                        view.pdfs.len(),
                                        stale_count
                                    )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .text_size(px(11.0))
                    .text_color(rgb(palette.muted))
                    .child(format!("LinkedIn pulled: {}", linkedin_date))
                    .child("•")
                    .child("Single source of truth in master_cv.yaml"),
            );

        card(header_content.into_any_element(), palette)
    }

    fn render_builder_panel(
        &self,
        view: &CvView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_busy = !self.busy.is_empty();
        let selected_profile = self.selected_profile.clone();
        let selected_lang = self.selected_lang.clone();
        let pdf_match = pdf_for_profile(view, &selected_profile, &selected_lang);

        let pdf_status_element = match pdf_match {
            Some(pdf) if pdf.stale => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(dot(palette.amber))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(palette.amber))
                        .child("Stale (older than master)"),
                ),
            Some(_) => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(dot(palette.green))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(palette.green))
                        .child("Built and up to date"),
                ),
            None => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(dot(palette.muted))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(palette.muted))
                        .child("Not built yet"),
                ),
        };

        let profile_buttons = view.profiles.iter().cloned().map(|prof| {
            let is_selected = prof == selected_profile;
            let prof_clone = prof.clone();
            Button::new(SharedString::from(format!("prof-{}", prof)))
                .small()
                .when(!is_selected, |b| b.ghost())
                .label(prof)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_profile = prof_clone.clone();
                    cx.notify();
                }))
        });

        let lang_en = Button::new("lang-en")
            .small()
            .when(selected_lang != "en", |b| b.ghost())
            .label("English (en)")
            .on_click(cx.listener(|this, _, _, cx| {
                this.selected_lang = "en".to_string();
                cx.notify();
            }));

        let lang_de = Button::new("lang-de")
            .small()
            .when(selected_lang != "de", |b| b.ghost())
            .label("German (de)")
            .on_click(cx.listener(|this, _, _, cx| {
                this.selected_lang = "de".to_string();
                cx.notify();
            }));

        let btn_build = Button::new("btn-build")
            .small()
            .label("Build PDF")
            .disabled(is_busy)
            .on_click(cx.listener({
                let profile = selected_profile.clone();
                let lang = selected_lang.clone();
                move |this, _, window, cx| {
                    this.run_action(
                        CvWriteRequest {
                            action: "build".to_string(),
                            profile: profile.clone(),
                            lang: lang.clone(),
                            ..Default::default()
                        },
                        window,
                        cx,
                    );
                }
            }));

        let btn_build_all = Button::new("btn-build-all")
            .small()
            .ghost()
            .label("Build All")
            .disabled(is_busy)
            .on_click(cx.listener(|this, _, window, cx| {
                this.run_action(
                    CvWriteRequest {
                        action: "build-all".to_string(),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            }));

        let btn_li_pull = Button::new("btn-li-pull")
            .small()
            .ghost()
            .label("Pull LinkedIn")
            .disabled(is_busy)
            .on_click(cx.listener(|this, _, window, cx| {
                this.run_action(
                    CvWriteRequest {
                        action: "linkedin-pull".to_string(),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            }));

        let btn_li_diff = Button::new("btn-li-diff")
            .small()
            .ghost()
            .label("LinkedIn Diff")
            .disabled(is_busy || view.linkedin_pulled.is_none())
            .on_click(cx.listener(|this, _, window, cx| {
                this.run_action(
                    CvWriteRequest {
                        action: "linkedin-diff".to_string(),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            }));

        let btn_li_sheet = Button::new("btn-li-sheet")
            .small()
            .ghost()
            .label(format!("Paste Sheet ({})", selected_lang))
            .disabled(is_busy)
            .on_click(cx.listener({
                let lang = selected_lang.clone();
                move |this, _, window, cx| {
                    this.run_action(
                        CvWriteRequest {
                            action: "linkedin-sheet".to_string(),
                            lang: lang.clone(),
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
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child("Profile & PDF Builder"),
                    )
                    .child(pdf_status_element),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .w(px(70.0))
                                    .text_size(px(12.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Profile:"),
                            )
                            .child(div().flex().flex_wrap().gap(px(6.0)).children(profile_buttons)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .w(px(70.0))
                                    .text_size(px(12.0))
                                    .text_color(rgb(palette.muted))
                                    .child("Language:"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(6.0))
                                    .child(lang_en)
                                    .child(lang_de),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt(px(6.0))
                    .border_t_1()
                    .border_color(rgb(palette.border))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(btn_build)
                            .child(btn_build_all),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(btn_li_pull)
                            .child(btn_li_diff)
                            .child(btn_li_sheet),
                    ),
            );

        card(content.into_any_element(), palette)
    }

    fn render_output_card(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let output = self.output.as_ref()?;
        let dismiss_btn = Button::new("btn-dismiss-output")
            .small()
            .ghost()
            .label("Dismiss")
            .on_click(cx.listener(|this, _, _, cx| {
                this.output = None;
                cx.notify();
            }));

        let content = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child("Output / Diff"),
                    )
                    .child(dismiss_btn),
            )
            .child(
                div()
                    .p(px(8.0))
                    .rounded(px(4.0))
                    .bg(rgb(palette.surface))
                    .text_size(px(11.0))
                    .text_color(rgb(palette.text))
                    .font_family(".AppleSystemUIFontMonospaced")
                    .child(output.clone()),
            );

        Some(card(content.into_any_element(), palette))
    }

    fn render_applications_table(
        &self,
        view: &CvView,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_busy = !self.busy.is_empty();
        let filter = &self.filter_status;
        let apps: Vec<&CvApplication> = if filter == "all" {
            view.applications.iter().collect()
        } else {
            filter_applications(view, filter)
        };

        let mut status_counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for app in &view.applications {
            *status_counts.entry(&app.status).or_insert(0) += 1;
        }

        let tab_all = Button::new("tab-all")
            .small()
            .when(filter != "all", |b| b.ghost())
            .label(format!("All ({})", view.applications.len()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.filter_status = "all".to_string();
                cx.notify();
            }));

        let status_tabs = view.statuses.iter().cloned().map(|st| {
            let count = status_counts.get(st.as_str()).copied().unwrap_or(0);
            let is_active = &st == filter;
            let st_clone = st.clone();
            Button::new(SharedString::from(format!("tab-{}", st)))
                .small()
                .when(!is_active, |b| b.ghost())
                .label(format!("{} ({})", st, count))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.filter_status = st_clone.clone();
                    cx.notify();
                }))
        });

        let app_rows = if apps.is_empty() {
            div()
                .p(px(16.0))
                .text_size(px(12.0))
                .text_color(rgb(palette.muted))
                .child("No applications found matching this status.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .children(apps.into_iter().enumerate().map(|(idx, app)| {
                    let company = app.company.clone();
                    let role = app.role.clone();
                    let slug = app.slug.clone();
                    let status = app.status.clone();
                    let profile_lang = format!("{} · {}", app.profile, app.lang);
                    let last_date = last_status_date(app).to_string();
                    let url_opt = app.url.clone();

                    let next_statuses: Vec<String> = view
                        .statuses
                        .iter()
                        .filter(|s| *s != &status)
                        .cloned()
                        .collect();

                    let status_badge_color = match status.as_str() {
                        "offer" => rgb(palette.green),
                        "interviewing" | "interview" => rgb(palette.accent),
                        "rejected" => rgb(palette.red),
                        _ => rgb(palette.muted),
                    };

                    let next_btns = next_statuses.into_iter().take(2).map(|target_st| {
                        let slug_c = slug.clone();
                        let target_c = target_st.clone();
                        Button::new(SharedString::from(format!("st-{}-{}", slug, target_st)))
                            .small()
                            .ghost()
                            .disabled(is_busy)
                            .label(format!("→ {}", target_st))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run_action(
                                    CvWriteRequest {
                                        action: "app-status".to_string(),
                                        slug: slug_c.clone(),
                                        status: target_c.clone(),
                                        ..Default::default()
                                    },
                                    window,
                                    cx,
                                );
                            }))
                    });

                    let build_app_btn = Button::new(SharedString::from(format!("build-app-{}", slug)))
                        .small()
                        .ghost()
                        .disabled(is_busy)
                        .label("Build CV")
                        .on_click(cx.listener({
                            let slug_clone = slug.clone();
                            move |this, _, window, cx| {
                                this.run_action(
                                    CvWriteRequest {
                                        action: "app-build".to_string(),
                                        slug: slug_clone.clone(),
                                        ..Default::default()
                                    },
                                    window,
                                    cx,
                                );
                            }
                        }));

                    div()
                        .id(SharedString::from(format!("app-row-{}", idx)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .p(px(8.0))
                        .rounded(px(6.0))
                        .bg(if idx % 2 == 0 {
                            rgb(palette.surface)
                        } else {
                            rgb(palette.window_tint)
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .w(px(220.0))
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child(company),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.muted))
                                        .child(role),
                                ),
                        )
                        .child(
                            div()
                                .w(px(110.0))
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(profile_lang),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .px(px(6.0))
                                        .py(px(2.0))
                                        .rounded(px(4.0))
                                        .bg(status_badge_color)
                                        .text_size(px(11.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(0xffffff))
                                        .child(status.clone()),
                                )
                                .children(next_btns),
                        )
                        .child(
                            div()
                                .w(px(90.0))
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(if last_date.is_empty() {
                                    "-".to_string()
                                } else {
                                    last_date
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(build_app_btn)
                                .children(url_opt.map(|u| {
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.accent))
                                        .child(format!("🔗 {}", u.chars().take(24).collect::<String>()))
                                })),
                        )
                }))
                .into_any_element()
        };

        let content = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child("Applications Ledger"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.0))
                            .child(tab_all)
                            .children(status_tabs),
                    ),
            )
            .child(app_rows);

        card(content.into_any_element(), palette)
    }

    fn render_new_application_panel(
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

    fn render_pdfs_list(&self, view: &CvView, palette: Palette) -> AnyElement {
        let pdf_rows = view.pdfs.iter().map(|pdf| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .p(px(6.0))
                .rounded(px(4.0))
                .bg(rgb(palette.surface))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(palette.text))
                                .child(pdf.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(format!("({})", pdf.path)),
                        ),
                )
                .child(if pdf.stale {
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(3.0))
                        .bg(rgb(palette.amber))
                        .text_size(px(10.0))
                        .text_color(rgb(0xffffff))
                        .child("stale: older than master")
                } else {
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(3.0))
                        .bg(rgb(palette.green))
                        .text_size(px(10.0))
                        .text_color(rgb(0xffffff))
                        .child("current")
                })
        });

        let content = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Built PDF Files"),
            )
            .child(div().flex().flex_col().gap(px(4.0)).children(pdf_rows));

        card(content.into_any_element(), palette)
    }
}

impl Render for CvPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());

        let Some(view) = self.cv.value.clone() else {
            let body = match &self.cv.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("CV", body, palette).into_any_element();
        };

        let mut panels = Vec::new();

        if let Some(notice) = &self.notice {
            panels.push(note(notice.as_str(), palette.green));
        }
        if let Some(err) = &self.write_error {
            panels.push(note(err.as_str(), palette.red));
        }

        panels.push(self.render_master_header(&view, palette));
        panels.push(self.render_builder_panel(&view, palette, cx));

        if let Some(output) = self.render_output_card(palette, cx) {
            panels.push(output);
        }

        panels.push(self.render_applications_table(&view, palette, cx));
        panels.push(self.render_new_application_panel(&view, palette, window, cx));
        panels.push(self.render_pdfs_list(&view, palette));

        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .p(px(16.0))
            .children(panels)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cv_page_initial_state() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = CvPage::new(repo);
        assert_eq!(page.selected_profile, "default");
        assert_eq!(page.selected_lang, "en");
        assert_eq!(page.filter_status, "all");
        assert!(page.output.is_none());
        assert!(page.notice.is_none());
    }
}

