//! Standalone capability sites registered with `sjel-status`.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, IntoElement, SharedString, Window};
use sjel_gpui_core::shell::CapabilityPanel;

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{card, note, placeholder};
use crate::theme::Palette;

pub struct AppsPage {
    repo: Repo,
    panels: Remote<Vec<CapabilityPanel>>,
    busy: Busy,
}

impl AppsPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            panels: Remote::default(),
            busy: Busy::default(),
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.panels,
            &repo,
            |shell| async move { shell.capability_panels().await },
        );
    }

    fn start(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, name.clone(), "App started."),
            move |shell| async move { shell.set_running(&name, true).await },
            Self::started,
        );
    }

    fn started(this: &mut Self, result: Option<()>, cx: &mut Context<Self>) {
        if result.is_some() {
            this.load(cx);
        }
    }

    fn open(&self, panel: &CapabilityPanel, cx: &mut Context<Self>) {
        let Some(url) = panel.url(&self.repo.endpoint()) else {
            return;
        };
        cx.open_url(url.as_str());
    }
}

impl Render for AppsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        let Some(panels) = self.panels.value.as_ref() else {
            let body = match &self.panels.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(280.),
            };
            return div().p(px(16.)).child(body).into_any_element();
        };

        let rows: Vec<AnyElement> = panels
            .iter()
            .filter(|panel| !panel.panel_port.is_empty())
            .map(|panel| {
                let title = panel.name.replace('-', " ");
                let state = match panel.up {
                    Some(true) => "running",
                    Some(false) => "off",
                    None => "status unknown",
                };
                let action = if panel.up == Some(true) {
                    let panel = panel.clone();
                    Button::new(SharedString::from(format!("open-{}", panel.name)))
                        .small()
                        .label("Open site")
                        .on_click(cx.listener(move |this, _, _, cx| this.open(&panel, cx)))
                } else {
                    let name = panel.name.clone();
                    let busy = self.busy.contains(&name);
                    Button::new(SharedString::from(format!("start-{}", panel.name)))
                        .small()
                        .label("Start")
                        .loading(busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start(name.clone(), window, cx)
                        }))
                };
                card(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .child(div().text_sm().child(title))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("{} · {}", panel.kind, state)),
                                ),
                        )
                        .child(action)
                        .into_any_element(),
                    palette,
                )
            })
            .collect();

        let body = if rows.is_empty() {
            note(
                "No standalone sites are enabled on this machine.",
                palette.muted,
            )
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(rows)
                .into_any_element()
        };
        div()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(19.))
                            .text_color(rgb(palette.text))
                            .child("Standalone sites"),
                    )
                    .child(
                        Button::new("apps-refresh")
                            .small()
                            .ghost()
                            .label("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Capability interfaces start on demand and open in your browser."),
            )
            .child(body)
            .into_any_element()
    }
}
