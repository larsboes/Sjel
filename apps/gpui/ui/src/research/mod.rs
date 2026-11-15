//! Research: Architecture, Local Decision Models, Upstreams & Benchmarks.
//!
//! Replaces the dashboard `/research` routes (`/research`, `/research/projects`, `/research/benchmarks`).
//! Backed by `core::research` and `POST /api/sjel-status/upstreams/watch`.

pub mod articles;
pub mod benchmarks;
pub mod projects;
pub mod types;

pub use types::*;

use crate::ely::forms::TextInput;
use gpui_kit::{
    div, prelude::*, px, AnyElement, Context, Entity, IntoElement, Subscription, Window,
};
use sjel_gpui_core::research::ResearchView;

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{note, placeholder, section};
use crate::theme::{Palette, ThemeContext};

pub struct ResearchPage {
    pub(super) repo: Repo,
    pub(super) research: Remote<ResearchView>,
    pub(super) busy: Busy,
    pub(super) notice: Option<String>,
    pub(super) write_error: Option<String>,

    pub(super) active_tab: ResearchTab,
    pub(super) selected_slug: String,
    pub(super) filter_query: String,
    pub(super) show_add_drawer: bool,

    pub(super) filter_input: Option<(Entity<TextInput>, Subscription)>,
    pub(super) add_url_input: Option<(Entity<TextInput>, Subscription)>,
    pub(super) add_summary_input: Option<(Entity<TextInput>, Subscription)>,
    pub(super) add_name_input: Option<(Entity<TextInput>, Subscription)>,
}

impl ResearchPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            research: Remote::default(),
            busy: Busy::default(),
            notice: None,
            write_error: None,

            active_tab: ResearchTab::Articles,
            selected_slug: "index".to_string(),
            filter_query: String::new(),
            show_add_drawer: false,

            filter_input: None,
            add_url_input: None,
            add_summary_input: None,
            add_name_input: None,
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.research,
            &repo,
            |shell| async move { shell.research_view().await },
        );
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn switch_tab(&mut self, tab: ResearchTab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    pub(super) fn render_tabs(&self, _palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let page = cx.entity();
        crate::primitives::sub_tabs(
            "research",
            vec![
                ("Articles", None, self.active_tab == ResearchTab::Articles),
                ("Projects", None, self.active_tab == ResearchTab::Projects),
                (
                    "Benchmarks",
                    None,
                    self.active_tab == ResearchTab::Benchmarks,
                ),
            ],
            move |index, _, cx| {
                let tabs = [
                    ResearchTab::Articles,
                    ResearchTab::Projects,
                    ResearchTab::Benchmarks,
                ];
                if let Some(tab) = tabs.get(index) {
                    page.update(cx, |page, cx| page.switch_tab(*tab, cx));
                }
            },
        )
    }
}

impl Render for ResearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;

        let Some(view) = self.research.value.clone() else {
            let body = match &self.research.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("Research", body, palette).into_any_element();
        };

        let mut panels = Vec::new();

        if let Some(notice) = &self.notice {
            panels.push(note(notice.as_str(), palette.green));
        }
        if let Some(error) = &self.write_error {
            panels.push(note(error.as_str(), palette.red));
        }

        panels.push(self.render_tabs(palette, cx));

        let active_content = match self.active_tab {
            ResearchTab::Articles => self.render_articles_tab(&view.articles, palette, cx),
            ResearchTab::Projects => {
                self.render_projects_tab(&view.systems, &view.upstreams, palette, window, cx)
            }
            ResearchTab::Benchmarks => self.render_benchmarks_tab(&view.suites, palette),
        };
        panels.push(active_content);

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .p(px(16.0))
            .children(panels)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn project_filter_updates_from_typing_and_survives_tab_changes(cx: &mut gpui::TestAppContext) {
        use gpui_kit::Focusable;

        let (page, cx) = crate::test_support::mount_root(cx, None, |_, _| {
            let mut page =
                ResearchPage::new(Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp)));
            page.research.value = Some(ResearchView::default());
            page
        });
        let tabs = cx.update(|window, cx| {
            window.blur(cx);
            window.focus_next(cx);
            window.focused(cx).unwrap()
        });
        cx.simulate_keystrokes("right");
        cx.run_until_parked();
        cx.update(|window, cx| {
            page.read(cx)
                .filter_input
                .as_ref()
                .unwrap()
                .0
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        });
        cx.simulate_input("sqlite");
        cx.run_until_parked();
        page.read_with(cx, |page, _| assert_eq!(page.filter_query, "sqlite"));
        cx.update(|window, cx| tabs.focus(window, cx));
        cx.simulate_keystrokes("right");
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert_eq!(page.active_tab, ResearchTab::Benchmarks)
        });
        cx.simulate_keystrokes("left");
        cx.run_until_parked();
        page.read_with(cx, |page, cx| {
            assert_eq!(page.active_tab, ResearchTab::Projects);
            assert_eq!(
                page.filter_input.as_ref().unwrap().0.read(cx).text(),
                "sqlite"
            );
        });
    }

    #[test]
    fn test_research_page_initial_state() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = ResearchPage::new(repo);
        assert_eq!(page.active_tab, ResearchTab::Articles);
        assert_eq!(page.selected_slug, "index");
        assert!(!page.show_add_drawer);
    }
}
