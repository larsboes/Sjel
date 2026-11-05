//! Provider discovery and model selection logic for Ask sessions.

use gpui_kit::Context;
use sjel_gpui_core::shell;

use super::types::Model;
use super::Ask;

impl Ask {
    /// Read the configured cloud roles once. Later calls do nothing until [`Self::retry_providers`].
    pub fn discover_providers(&mut self, cx: &mut Context<Self>) {
        if self.provider_discovery_started {
            return;
        }
        self.provider_discovery_started = true;
        let Ok(shell_client) = self.repo.shell() else {
            self.provider_discovery_finished = true;
            self.provider_discovery_error = Some("Sjel is not connected".into());
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = shell::run(async move { shell_client.ask_providers().await }).await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Some(Ok(providers)) => {
                        view.providers = providers;
                        view.provider_discovery_error = None;
                    }
                    Some(Err(error)) => view.provider_discovery_error = Some(error.to_string()),
                    None => {
                        view.provider_discovery_error =
                            Some("the provider request did not complete".into())
                    }
                }
                view.provider_discovery_finished = true;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn retry_providers(&mut self, cx: &mut Context<Self>) {
        self.provider_discovery_started = false;
        self.provider_discovery_finished = false;
        self.provider_discovery_error = None;
        self.discover_providers(cx);
    }

    /// The model the next question goes to: `local`, or a cloud role as `provider · model`.
    pub fn model(&self) -> Model {
        let Some(role) = self.selected_provider.as_deref() else {
            return Model::Local {
                error: self.provider_discovery_error.clone(),
            };
        };
        let label = self
            .providers
            .iter()
            .find(|provider| provider.role == role)
            .map_or_else(
                || role.to_owned(),
                |provider| format!("{} · {}", provider.provider, provider.model),
            );
        Model::Cloud {
            label,
            confirmed: self.confirm_public_send,
        }
    }

    /// Step local → each available cloud role → local. A new choice is never confirmed.
    pub fn next_model(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let available: Vec<&str> = self
            .providers
            .iter()
            .filter(|provider| provider.available)
            .map(|provider| provider.role.as_str())
            .collect();
        let next = match self.selected_provider.as_deref() {
            None => available.first(),
            Some(role) => available
                .iter()
                .position(|candidate| *candidate == role)
                .and_then(|index| available.get(index + 1)),
        };
        self.selected_provider = next.map(|role| (*role).to_owned());
        self.confirm_public_send = false;
        cx.notify();
    }

    pub fn toggle_public(&mut self, cx: &mut Context<Self>) {
        if self.selected_provider.is_some() && !self.loading {
            self.confirm_public_send = !self.confirm_public_send;
            cx.notify();
        }
    }

    /// True while a run is going that [`Self::stop`] can end.
    pub fn can_stop(&self) -> bool {
        self.loading && self.active_run.is_some()
    }
}
