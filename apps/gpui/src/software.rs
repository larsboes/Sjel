//! The apps, leftovers and tools on this Mac from the `software` capability: what each costs on
//! disk, when it was last opened, and what the operator decided about it. Click a row to decide;
//! the note field carries why, and a decision button saves both. A row marked `remove` offers its
//! uninstall, which the capability runs through whoever installed it.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, IntoElement, SharedString, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::shell::{SoftwareApp, SoftwareApps};
use crate::theme::Palette;
use crate::ui::{dot, list, note, placeholder, row, section};

const SCAN: &str = "software:scan";

/// Which rows the page lists. Review holds every kind: a stale decision is one list to work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Apps,
    Leftovers,
    Tools,
    Review,
}

const FILTERS: [(Filter, &str); 4] = [
    (Filter::Apps, "Apps"),
    (Filter::Leftovers, "Leftovers"),
    (Filter::Tools, "Tools"),
    (Filter::Review, "Review"),
];

/// Present rows only: an uninstalled one keeps its history in the store, not on this page.
pub fn shows(filter: Filter, app: &SoftwareApp) -> bool {
    app.present
        && match filter {
            Filter::Apps => app.kind == "app" || app.kind.is_empty(),
            Filter::Leftovers => app.kind == "leftover",
            Filter::Tools => app.kind == "tool",
            Filter::Review => app.review.is_some(),
        }
}
const DECISIONS: [(&str, &str); 4] = [
    ("use", "Use"),
    ("remove", "Remove"),
    ("replace", "Replace"),
    ("rebuild", "Rebuild"),
];

pub struct Software {
    repo: Repo,
    apps: Remote<SoftwareApps>,
    busy: Busy,
    /// The bundle id whose decision row is open.
    open: Option<String>,
    note: Option<Entity<InputState>>,
    filter: Filter,
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

    fn scan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn toggle(&mut self, app: &SoftwareApp, window: &mut Window, cx: &mut Context<Self>) {
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

    fn decide(&mut self, decision: &'static str, window: &mut Window, cx: &mut Context<Self>) {
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

    fn uninstall(&mut self, bundle_id: String, window: &mut Window, cx: &mut Context<Self>) {
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

    fn app_row(&self, app: &SoftwareApp, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let open = self.open.as_deref() == Some(app.bundle_id.as_str());
        let (tone, label) = decision_look(app.decision.as_deref(), palette);
        let clicked = app.clone();
        let header = row()
            .id(SharedString::from(format!("app-{}", app.bundle_id)))
            .cursor_pointer()
            .hover(move |style| style.bg(rgba(palette.raised)))
            .on_click(cx.listener(move |view, _, window, cx| view.toggle(&clicked, window, cx)))
            .child(dot(tone))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().text_sm().child(app.name.clone()))
                    .children(app.review.clone().map(|r| {
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child(r)
                    }))
                    .children(
                        app.note
                            .clone()
                            .filter(|n| !n.is_empty() && !open)
                            .map(|n| {
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(n)
                            }),
                    ),
            )
            .child(
                div()
                    .w(px(72.))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(size(app.app_bytes + app.data_bytes)),
            )
            .child(
                div()
                    .w(px(80.))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(match app.kind.as_str() {
                        "tool" => app.source.clone(),
                        "leftover" => "no app".to_owned(),
                        _ => last_used(app.last_used.as_deref()),
                    }),
            )
            .child(
                div()
                    .w(px(64.))
                    .text_xs()
                    .text_color(rgb(tone))
                    .child(label),
            );
        if !open {
            return header.into_any_element();
        }
        let busy = self.busy.contains(&format!("decide:{}", app.bundle_id));
        let buttons = DECISIONS.iter().map(|(decision, title)| {
            let decision: &'static str = decision;
            Button::new(SharedString::from(format!("decide-{decision}")))
                .small()
                .when(app.decision.as_deref() != Some(decision), |b| b.ghost())
                .label(*title)
                .loading(busy)
                .on_click(cx.listener(move |view, _, window, cx| view.decide(decision, window, cx)))
        });
        let uninstalling = self.busy.contains(&format!("uninstall:{}", app.bundle_id));
        let target = app.bundle_id.clone();
        let uninstall = (app.decision.as_deref() == Some("remove")).then(|| {
            Button::new("software-uninstall")
                .small()
                .danger()
                .label(uninstall_label(app))
                .loading(uninstalling)
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.uninstall(target.clone(), window, cx)
                }))
        });
        let input = self.note.clone();
        div()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_3()
                    .pb_3()
                    .children(input.map(|input| Input::new(&input)))
                    .child(div().flex().gap_2().children(buttons).children(uninstall))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} · app {} · data {}",
                                app.bundle_id,
                                size(app.app_bytes),
                                size(app.data_bytes)
                            )),
                    ),
            )
            .into_any_element()
    }
}

impl Render for Software {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
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
            .map(|app| self.app_row(app, palette, cx))
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

/// Says what the button will do: Trash for an app or leftover, a manager's uninstall otherwise.
fn uninstall_label(app: &SoftwareApp) -> String {
    match (app.kind.as_str(), app.source.as_str()) {
        ("tool", manager) => format!("Uninstall ({manager})"),
        (_, source) if source.starts_with("cask:") => "Uninstall (brew cask)".to_owned(),
        _ => "Move to Trash".to_owned(),
    }
}

fn decision_look(decision: Option<&str>, palette: Palette) -> (u32, &'static str) {
    match decision {
        Some("use") => (palette.green, "use"),
        Some("remove") => (palette.red, "remove"),
        Some("replace") => (palette.amber, "replace"),
        Some("rebuild") => (palette.amber, "rebuild"),
        _ => (palette.muted, ""),
    }
}

/// The date part of Spotlight's stamp, or `never` when it holds none.
fn last_used(stamp: Option<&str>) -> String {
    match stamp.and_then(|s| s.get(..10)) {
        Some(date) => date.to_owned(),
        None => "never".to_owned(),
    }
}

fn size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.;
    if mb >= 1_024. {
        format!("{:.1} GB", mb / 1_024.)
    } else {
        format!("{mb:.0} MB")
    }
}

#[cfg(test)]
mod tests {
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
