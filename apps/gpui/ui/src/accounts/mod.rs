//! Settings › Accounts: each credential a capability declares, where this Mac holds it, the
//! steps to obtain it, and a drop field for a Google client JSON.
//!
//! The rows come from sjel-status (`crate::accounts::CredentialRow`, presence only). Connect
//! runs the consent flow in this process (`sjel_gpui_core::accounts::connect_google`) and
//! writes the grant to the OS keychain; nothing secret is sent to the shell.

pub mod constants;
pub mod google;
pub mod provider_keys;

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::component::input::InputState;
use gpui_kit::{div, prelude::*, AnyElement, Context, Entity, Subscription, Window};

use crate::data::{self, Remote, Repo};
use crate::primitives::{list, note, section};
use crate::theme::ThemeContext;
use sjel_gpui_core::accounts::CredentialRow;

pub use constants::KEY_PROVIDERS;

pub struct Accounts {
    pub(crate) repo: Repo,
    pub(crate) rows: Remote<Vec<CredentialRow>>,
    /// The client JSON dropped for each credential ID, until Connect reads it.
    pub(crate) dropped: HashMap<String, PathBuf>,
    /// The credential ID whose consent flow is running.
    pub(crate) busy: Option<String>,
    /// The last outcome per credential ID: success, and the line shown.
    pub(crate) outcome: HashMap<String, (bool, String)>,
    pub(crate) key_provider: String,
    pub(crate) key_input: Option<(Entity<InputState>, Subscription)>,
    pub(crate) key_status: Option<String>,
    pub(crate) confirm_key_removal: bool,
    pub(crate) key_presence: Remote<HashMap<String, bool>>,
}

impl Accounts {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            rows: Remote::default(),
            dropped: HashMap::new(),
            busy: None,
            outcome: HashMap::new(),
            key_provider: "openai".into(),
            key_input: None,
            key_status: None,
            confirm_key_removal: false,
            key_presence: Remote::default(),
        }
    }

    pub fn refresh_if_missing(&mut self, cx: &mut Context<Self>) {
        if (self.rows.value.is_none() && self.rows.error.is_none() && !self.rows.loading)
            || (self.key_presence.value.is_none()
                && self.key_presence.error.is_none()
                && !self.key_presence.loading)
        {
            self.refresh(cx);
        }
    }

    /// Reads the declarations again. Called when the section opens and after a connect.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |this| &mut this.rows,
            &repo,
            |shell| async move { shell.credentials().await },
        );
        data::read(
            cx,
            self,
            |this| &mut this.key_presence,
            &repo,
            |shell| async move {
                let keys = shell.provider_key_status().await?;
                Ok(keys
                    .into_iter()
                    .map(|key| (key.provider, key.present))
                    .collect())
            },
        );
    }
}

impl Render for Accounts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let key_input = self.key_input(window, cx);
        self.refresh_if_missing(cx);
        let body: AnyElement = if let Some(rows) = self.rows.value.as_ref() {
            if rows.is_empty() {
                list(
                    vec![note("No capability declares a credential.", palette.muted)],
                    palette,
                )
            } else {
                let rows = rows.clone();
                list(
                    rows.iter().map(|row| self.card(row, palette, cx)).collect(),
                    palette,
                )
            }
        } else if let Some(error) = self.rows.error.as_ref() {
            list(
                vec![note(
                    format!("Could not read the credentials: {error}"),
                    palette.red,
                )],
                palette,
            )
        } else {
            list(
                vec![note("Reading the declared credentials…", palette.muted)],
                palette,
            )
        };
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.provider_keys(palette, &key_input, cx))
            .child(section("GOOGLE", body, palette))
            .child(note(
                "The client JSON is read once from the dropped file and never sent to the \
                 shell. Delete the download afterwards; Sjel keeps the grant in the keychain.",
                palette.muted,
            ))
    }
}
