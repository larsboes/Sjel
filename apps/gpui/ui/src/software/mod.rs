//! The apps, leftovers and tools on this Mac from the `software` capability: what each costs on
//! disk, when it was last opened, and what the operator decided about it. Click a row to decide;
//! the note field carries why, and a decision button saves both. A row marked `remove` offers its
//! uninstall, which the capability runs through whoever installed it.

pub mod row;
pub mod types;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, rgb, AnyElement, Context, Entity, IntoElement, SharedString, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{list, note, placeholder, section};
use crate::theme::ThemeContext;
use sjel_gpui_core::shell::{SoftwareApp, SoftwareApps};

use types::size;
pub use types::{shows, Filter, FILTERS, SCAN};

use self::row::render_app_row;

pub struct Software {
    pub(crate) repo: Repo,
    pub(crate) apps: Remote<SoftwareApps>,
    pub(crate) busy: Busy,
    /// The bundle id whose decision row is open.
    pub(crate) open: Option<String>,
    pub(crate) note: Option<Entity<InputState>>,
    pub(crate) filter: Filter,
}

impl Software {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            apps: Remote::default(),
            busy: Busy::default(),
            open: None,
            note: None,
            filter: Filter::Apps,
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.apps,
            &repo,
            |shell| async move { shell.software_apps().await },
        );
    }

    pub(crate) fn scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, SCAN, "Scan recorded."),
            |shell| async move { shell.software_scan().await },
            |this, _, cx| this.load(cx),
        );
    }

    pub(crate) fn toggle(
        &mut self,
        app: &SoftwareApp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open.as_deref() == Some(app.bundle_id.as_str()) {
            self.open = None;
        } else {
            self.open = Some(app.bundle_id.clone());
            let text = app.note.clone().unwrap_or_default();
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Why: what you use it for, or what replaces it")
            });
            input.update(cx, |input, cx| input.set_value(text, window, cx));
            self.note = Some(input);
        }
        cx.notify();
    }

    pub(crate) fn decide(
        &mut self,
        decision: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bundle_id) = self.open.clone() else {
            return;
        };
        let note = self
            .note
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        let repo = self.repo.clone();
        let key = format!("decide:{bundle_id}");
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Decision saved."),
            move |shell| async move { shell.software_decide(&bundle_id, decision, &note).await },
            |this, saved, cx| {
                if saved.is_some() {
                    this.open = None;
                }
                this.load(cx);
            },
        );
    }

    pub(crate) fn uninstall(
        &mut self,
        bundle_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let key = format!("uninstall:{bundle_id}");
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Uninstalled."),
            move |shell| async move { shell.software_uninstall(&bundle_id).await },
            |this, done, cx| {
                if done.is_some() {
                    this.open = None;
                }
                this.load(cx);
            },
        );
    }
}

impl Render for Software {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let Some(apps) = self.apps.value.clone() else {
            let body = match &self.apps.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("Software", body, palette).into_any_element();
        };
        let filter = self.filter;
        let shown: Vec<&SoftwareApp> = apps.apps.iter().filter(|a| shows(filter, a)).collect();
        let total: u64 = shown.iter().map(|a| a.app_bytes + a.data_bytes).sum();
        let undecided = shown.iter().filter(|a| a.decision.is_none()).count();
        let summary = format!("{} · {undecided} undecided", size(total));
        let tabs: Vec<Button> = FILTERS
            .iter()
            .map(|(choice, title)| {
                let choice = *choice;
                let count = apps.apps.iter().filter(|a| shows(choice, a)).count();
                Button::new(SharedString::from(format!("software-filter-{title}")))
                    .small()
                    .when(choice != filter, |b| b.ghost())
                    .label(format!("{title} {count}"))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.filter = choice;
                        view.open = None;
                        cx.notify();
                    }))
            })
            .collect();
        let scan = Button::new("software-scan")
            .small()
            .ghost()
            .label("Scan now")
            .loading(self.busy.contains(SCAN))
            .on_click(cx.listener(|view, _, window, cx| view.scan(window, cx)));
        let rows: Vec<AnyElement> = shown
            .into_iter()
            .map(|app| render_app_row(self, app, palette, cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().flex().gap_1().children(tabs))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette.muted))
                                    .child(summary),
                            )
                            .child(scan),
                    ),
            )
            .child(list(rows, palette))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::types::{last_used, uninstall_label};
    use super::*;

    fn row(kind: &str, present: bool, review: Option<&str>) -> SoftwareApp {
        SoftwareApp {
            kind: kind.into(),
            source: "other".into(),
            bundle_id: "com.example.app".into(),
            name: "Example".into(),
            version: String::new(),
            app_bytes: 0,
            data_bytes: 0,
            last_used: None,
            present,
            decision: None,
            note: None,
            review: review.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn filters_list_present_rows_of_their_kind() {
        assert!(shows(Filter::Apps, &row("app", true, None)));
        assert!(
            !shows(Filter::Apps, &row("app", false, None)),
            "uninstalled rows stay off the page"
        );
        assert!(shows(Filter::Leftovers, &row("leftover", true, None)));
        assert!(!shows(Filter::Tools, &row("leftover", true, None)));
        assert!(shows(
            Filter::Review,
            &row("tool", true, Some("decided more than 180 days ago"))
        ));
        assert!(!shows(Filter::Review, &row("app", true, None)));
    }

    #[test]
    fn the_uninstall_button_names_what_it_does() {
        let mut tool = row("tool", true, None);
        tool.source = "brew".into();
        assert_eq!(uninstall_label(&tool), "Uninstall (brew)");
        let mut cask = row("app", true, None);
        cask.source = "cask:vorssaint".into();
        assert_eq!(uninstall_label(&cask), "Uninstall (brew cask)");
        assert_eq!(
            uninstall_label(&row("leftover", true, None)),
            "Move to Trash"
        );
    }

    #[test]
    fn last_used_and_size_read_at_a_glance() {
        assert_eq!(last_used(Some("2026-10-22 13:00:00 +0000")), "2026-10-22");
        assert_eq!(last_used(None), "never");
        assert_eq!(size(500 * 1_048_576), "500 MB");
        assert_eq!(size(4_404_019_200), "4.1 GB");
    }
}
