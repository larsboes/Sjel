//! Node Setup & Device Pairing: Probed machine requirements, Turso replica setup, and device pairing.
//!
//! Replaces the dashboard `/setup` route. Backed by `capabilities/sjel-status` and `capabilities/devices`.

pub mod devices;
pub mod machine;
pub mod types;

pub use types::*;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, AnyElement, Context, Entity, IntoElement, Subscription, Window,
};
use sjel_gpui_core::setup::{PairingChallenge, QrMatrix, SetupView};

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{note, placeholder, section};
use crate::theme::{Palette, ThemeContext};

pub struct SetupPage {
    pub(super) repo: Repo,
    pub(super) setup: Remote<SetupView>,
    pub(super) busy: Busy,
    pub(super) notice: Option<String>,
    pub(super) write_error: Option<String>,

    pub(super) active_tab: SetupTab,
    pub(super) verifying: bool,
    pub(super) confirming_component: Option<String>,
    /// The component whose apply is in flight, so the answer's sentence can name it: a `then`
    /// callback gets no captured state, only the page.
    pub(super) applying_component: Option<String>,
    pub(super) applied_log: Vec<String>,

    /// The one-time code this Mac is showing, and the picture of it. Both are transient: a code is
    /// single-use and lives ten minutes, so nothing here survives a reload.
    pub(super) challenge: Option<PairingChallenge>,
    pub(super) challenge_payload: Option<String>,
    pub(super) challenge_matrix: Option<QrMatrix>,
    pub(super) challenge_error: Option<String>,
    /// The host the code carries, one of `LanInfo::pairing_hosts`. `None` takes the first.
    pub(super) pairing_host: Option<String>,
    /// The device whose revoke is waiting for a second press.
    pub(super) confirming_revoke: Option<String>,

    pub(super) url_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) credential_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) interval_input: Option<(Entity<InputState>, Subscription)>,
}

impl SetupPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            setup: Remote::default(),
            busy: Busy::default(),
            notice: None,
            write_error: None,

            active_tab: SetupTab::Machine,
            verifying: false,
            confirming_component: None,
            applying_component: None,
            applied_log: Vec::new(),

            challenge: None,
            challenge_payload: None,
            challenge_matrix: None,
            challenge_error: None,
            pairing_host: None,
            confirming_revoke: None,

            url_input: None,
            credential_input: None,
            interval_input: None,
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.setup,
            &repo,
            |shell| async move { shell.setup_view().await },
        );
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn switch_tab(&mut self, tab: SetupTab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    pub(super) fn render_tabs(&self, _palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let tab_btn = |tab: SetupTab, label: &'static str, cx: &mut Context<Self>| {
            let active = self.active_tab == tab;
            Button::new(label)
                .small()
                .when(!active, |b| b.ghost())
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.switch_tab(tab, cx);
                }))
        };

        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(tab_btn(SetupTab::Machine, "Machine Setup", cx))
            .child(tab_btn(SetupTab::Devices, "Paired Devices", cx))
            .into_any_element()
    }
}

impl Render for SetupPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;

        let Some(view) = self.setup.value.clone() else {
            let body = match &self.setup.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("Setup", body, palette).into_any_element();
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
            SetupTab::Machine => self.render_machine_tab(&view.plans, palette, window, cx),
            SetupTab::Devices => self.render_devices_tab(&view, palette, cx),
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

    #[test]
    fn test_setup_page_initial_state() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = SetupPage::new(repo);
        assert_eq!(page.active_tab, SetupTab::Machine);
        assert!(page.confirming_component.is_none());
        assert!(!page.verifying);
    }
}
