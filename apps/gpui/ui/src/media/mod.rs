//! Media: Exact-byte ingest gate, volume integrity ledger, and run inspection.
//!
//! Replaces the dashboard `/media` route. Backed by `capabilities/media` via sjel-status.

pub mod inspector;
pub mod runs;
pub mod summary;

use gpui_kit::{div, prelude::*, px, Context, IntoElement, Window};
use sjel_gpui_core::media::{MediaIngestDetail, MediaView};

use crate::data::{self, Remote, Repo};
use crate::primitives::{note, placeholder, section};
use crate::theme::ThemeContext;

use self::inspector::render_run_detail_card;
use self::runs::render_ingest_runs_table;
use self::summary::{render_index_summary, render_volumes_table};

pub struct MediaPage {
    pub(crate) repo: Repo,
    pub(crate) media: Remote<MediaView>,
    pub(crate) notice: Option<String>,

    pub(crate) selected_run_id: Option<i64>,
    pub(crate) selected_run_detail: Remote<MediaIngestDetail>,
    pub(crate) filter_disposition: String,
}

impl MediaPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            media: Remote::default(),
            notice: None,

            selected_run_id: None,
            selected_run_detail: Remote::default(),
            filter_disposition: "all".to_string(),
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.media,
            &repo,
            |shell| async move { shell.media_view().await },
        );
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
        if let Some(run_id) = self.selected_run_id {
            self.load_run_detail(run_id, cx);
        }
    }

    fn load_run_detail(&mut self, id: i64, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.selected_run_detail,
            &repo,
            move |shell| async move { shell.media_ingest(id).await },
        );
    }

    pub(crate) fn select_run(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.selected_run_id == Some(id) {
            self.selected_run_id = None;
            self.selected_run_detail = Remote::default();
        } else {
            self.selected_run_id = Some(id);
            self.filter_disposition = "all".to_string();
            self.load_run_detail(id, cx);
        }
        cx.notify();
    }
}

impl Render for MediaPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;

        let Some(view) = self.media.value.clone() else {
            let body = match &self.media.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("Media", body, palette).into_any_element();
        };

        let mut panels = Vec::new();

        if let Some(notice) = &self.notice {
            panels.push(note(notice.as_str(), palette.green));
        }

        panels.push(render_index_summary(&view.status, palette, cx));
        panels.push(render_volumes_table(&view.status.volumes, palette));
        panels.push(render_ingest_runs_table(
            &view.runs,
            self.selected_run_id,
            palette,
            cx,
        ));

        if let Some(inspector) = render_run_detail_card(self, palette, cx) {
            panels.push(inspector);
        }

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
    fn test_media_page_initial_state() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = MediaPage::new(repo);
        assert!(page.selected_run_id.is_none());
        assert_eq!(page.filter_disposition, "all");
        assert!(page.notice.is_none());
    }
}
