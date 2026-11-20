//! Device pairing and management on the Mac Setup page.

mod devices;

use gpui_kit::{div, prelude::*, px, Context, IntoElement, Window};
use sjel_gpui_core::setup::{PairingChallenge, QrMatrix, SetupView};

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{note, placeholder, section};
use crate::theme::ThemeContext;

pub struct SetupPage {
    pub(super) repo: Repo,
    pub(super) setup: Remote<SetupView>,
    pub(super) busy: Busy,
    pub(super) notice: Option<String>,

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
}

impl SetupPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            setup: Remote::default(),
            busy: Busy::default(),
            notice: None,

            challenge: None,
            challenge_payload: None,
            challenge_matrix: None,
            challenge_error: None,
            pairing_host: None,
            confirming_revoke: None,
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
        panels.push(self.render_devices(&view, palette, cx));

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
    fn setup_mounts_device_pairing_directly(cx: &mut gpui::TestAppContext) {
        let (_, cx) = crate::test_support::mount_root(cx, None, |_, _| {
            let mut page =
                SetupPage::new(Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp)));
            page.setup.value = Some(SetupView::default());
            page
        });
        assert!(cx.debug_bounds("setup-device-pairing").is_some());
    }
}
