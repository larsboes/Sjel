//! CV: Profile variations, PDF builds, application ledger, and LinkedIn synchronization.
//!
//! Replaces the dashboard `/cv` route. Backed by `capabilities/cv/cv` via sjel-status.

pub mod applications;
pub mod builder;
pub mod form;

use gpui_kit::component::input::InputState;
use gpui_kit::{div, prelude::*, px, Context, Entity, IntoElement, Render, Subscription, Window};

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{note, placeholder, section};
use crate::theme::ThemeContext;
use sjel_gpui_core::cv::{CvView, CvWriteRequest};

pub struct CvPage {
    pub repo: Repo,
    pub cv: Remote<CvView>,
    pub busy: Busy,
    pub notice: Option<String>,
    pub write_error: Option<String>,
    pub output: Option<String>,

    pub selected_profile: String,
    pub selected_lang: String,
    pub filter_status: String,

    pub(crate) company_input: Option<(Entity<InputState>, Subscription)>,
    pub(crate) role_input: Option<(Entity<InputState>, Subscription)>,
    pub(crate) url_input: Option<(Entity<InputState>, Subscription)>,
    pub draft_profile: String,
    pub draft_lang: String,
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

    pub(crate) fn run_action(
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
}

impl Render for CvPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;

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
