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

use crate::ely::forms::TextInput;
use gpui_kit::{div, prelude::*, AnyElement, Context, Entity, Subscription, Window};

use crate::data::{self, Remote, Repo};
use crate::ely::data_display::Tone;
use crate::ely::tables::{Cell, Column, DataTable, Row};
use crate::primitives::{list, note, section};
use crate::theme::ThemeContext;
use sjel_gpui_core::accounts::CredentialRow;

pub use constants::KEY_PROVIDERS;

pub struct Accounts {
    selected: Option<String>,
    pub(crate) repo: Repo,
    pub(crate) rows: Remote<Vec<CredentialRow>>,
    /// The client JSON dropped for each credential ID, until Connect reads it.
    pub(crate) dropped: HashMap<String, PathBuf>,
    /// The credential ID whose consent flow is running.
    pub(crate) busy: Option<String>,
    /// The last outcome per credential ID: success, and the line shown.
    pub(crate) outcome: HashMap<String, (bool, String)>,
    pub(crate) key_provider: String,
    pub(crate) key_input: Option<(Entity<TextInput>, Subscription)>,
    pub(crate) key_status: Option<String>,
    pub(crate) confirm_key_removal: bool,
    pub(crate) key_presence: Remote<HashMap<String, bool>>,
}

impl Accounts {
    pub fn new(repo: Repo) -> Self {
        Self {
            selected: None,
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
                let selected = rows
                    .iter()
                    .find(|row| self.selected.as_deref() == Some(credential_key(row).as_str()))
                    .unwrap_or(&rows[0]);
                let page = cx.entity();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        DataTable::new(
                            "accounts-credentials",
                            [
                                Column::new("label", "ACCOUNT"),
                                Column::new("capability", "CAPABILITY"),
                                Column::new("state", "STATE"),
                            ],
                        )
                        .rows(rows.iter().map(credential_row).collect::<Vec<_>>())
                        .current(Some(credential_key(selected)))
                        .on_open(move |key, _, cx| {
                            page.update(cx, |page, cx| {
                                page.selected = Some(key.to_string());
                                cx.notify();
                            })
                        }),
                    )
                    .child(self.card(selected, palette, cx))
                    .into_any_element()
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

fn credential_key(row: &CredentialRow) -> String {
    format!("{}:{}", row.capability, row.id)
}

fn credential_row(row: &CredentialRow) -> Row {
    let state = row.error.clone().unwrap_or_else(|| row.state().to_owned());
    let tone = if row.error.is_some() {
        Tone::Danger
    } else if row.in_keychain {
        Tone::Success
    } else {
        Tone::Warning
    };
    Row::new(
        credential_key(row),
        [
            Cell::Text(row.label.clone().into()),
            Cell::Text(row.capability.clone().into()),
            Cell::Tag(state.into(), tone),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn opening_a_credential_keeps_its_connection_state(cx: &mut gpui::TestAppContext) {
        crate::test_support::init(cx);
        let (page, cx) = cx.add_window_view(|_, _| {
            let mut page =
                Accounts::new(Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp)));
            page.rows.value = Some(
                ["a", "b"]
                    .map(|id| CredentialRow {
                        capability: "calendar".into(),
                        id: id.into(),
                        label: id.into(),
                        kind: "google-oauth".into(),
                        scopes: vec![],
                        steps: vec![],
                        env_fallback: None,
                        in_keychain: false,
                        env_file: false,
                        error: None,
                    })
                    .to_vec(),
            );
            page.key_presence.value = Some(HashMap::new());
            page.dropped
                .insert("b".into(), PathBuf::from("/synthetic/client.json"));
            page.outcome.insert("b".into(), (false, "Try again".into()));
            page
        });
        cx.run_until_parked();
        let row = cx.debug_bounds("table-row calendar:b").unwrap();
        cx.simulate_click(row.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert_eq!(page.selected.as_deref(), Some("calendar:b"));
            assert_eq!(page.dropped["b"], PathBuf::from("/synthetic/client.json"));
            assert_eq!(page.outcome["b"].1, "Try again");
            assert!(page.busy.is_none(), "opening a row does not start consent");
        });
    }
}
