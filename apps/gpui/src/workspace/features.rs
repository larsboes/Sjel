//! Feature pack installation, uninstallation, catalogue discovery, and recommendation management.

use gpui_kit::Context;
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::Remote;
use super::Workspace;

impl Workspace {
    pub fn refresh_installed_features(&mut self, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.installed_features.settle(Err(error));
                cx.notify();
                return;
            }
        };
        if !self.installed_features.begin() {
            return;
        }
        cx.notify();
        let work = shell::run(async move { shell.installed_features().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.installed_features.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn refresh_feature_catalogue(&mut self, cx: &mut Context<Self>) {
        if !self.available_features.begin() {
            return;
        }
        cx.notify();
        let work = shell::run(async { sjel_gpui_core::shell::feature_catalogue().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.available_features.settle(result);
                let catalogue_has_match =
                    view.available_features
                        .value
                        .as_ref()
                        .is_some_and(|packages| {
                            crate::settings::catalogue_matches(packages, &view.settings_query)
                        });
                if let Some(section) =
                    crate::settings::search_section(&view.settings_query, catalogue_has_match)
                {
                    view.settings_section = section;
                    if section == crate::settings::Section::Features {
                        if let Some(feature_view) =
                            crate::settings::feature_view_for_search(&view.settings_query)
                        {
                            view.settings_feature_view = feature_view;
                        } else if catalogue_has_match {
                            view.settings_feature_view = crate::settings::FeatureView::Explore;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn request_feature_uninstall(&mut self, name: String, cx: &mut Context<Self>) {
        self.feature_uninstall_confirmation = Some(name);
        self.feature_manager_message = None;
        cx.notify();
    }

    pub fn dismiss_feature_recommendation(
        &mut self,
        feature: String,
        topic: String,
        cx: &mut Context<Self>,
    ) {
        let key = format!("{feature}::{topic}");
        self.dismissed_feature_recommendations.insert(key);
        self.feature_manager_message = None;
        if let Err(error) =
            crate::settings::save_recommendation_dismissals(&self.dismissed_feature_recommendations)
        {
            self.feature_manager_message = Some(format!("Could not save this preference: {error}"));
        }
        cx.notify();
    }

    pub fn request_feature_install(&mut self, name: String, cx: &mut Context<Self>) {
        self.feature_install_confirmation = Some(name);
        self.feature_manager_message = None;
        cx.notify();
    }

    pub fn cancel_feature_install(&mut self, cx: &mut Context<Self>) {
        self.feature_install_confirmation = None;
        cx.notify();
    }

    pub fn install_feature(
        &mut self,
        name: String,
        version: String,
        sha256: String,
        cx: &mut Context<Self>,
    ) {
        let key = format!("package-install:{name}");
        if !self.busy.start(&key) {
            return;
        }
        self.feature_install_confirmation = None;
        self.feature_manager_message = None;
        cx.notify();
        let work = shell::run(async move {
            sjel_gpui_core::shell::install_feature(&name, &version, &sha256).await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.busy.finish(&key);
                match result {
                    Ok(()) => {
                        view.feature_manager_message = Some(
                            "Feature installed. It is ready to enable in Capabilities.".into(),
                        );
                        view.installed_features = Remote::default();
                        view.refresh_installed_features(cx);
                        view.available_features = Remote::default();
                        view.refresh_feature_catalogue(cx);
                    }
                    Err(error) => view.feature_manager_message = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn cancel_feature_uninstall(&mut self, cx: &mut Context<Self>) {
        self.feature_uninstall_confirmation = None;
        cx.notify();
    }

    pub fn uninstall_feature(&mut self, name: String, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.feature_manager_message = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let key = format!("package-uninstall:{name}");
        if !self.busy.start(&key) {
            return;
        }
        self.feature_uninstall_confirmation = None;
        self.feature_manager_message = None;
        cx.notify();
        let work = shell::run(async move { shell.uninstall_feature(&name).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.busy.finish(&key);
                match result {
                    Ok(()) => {
                        view.feature_manager_message =
                            Some("Feature removed. Its capability data was retained.".into());
                        view.installed_features = Remote::default();
                        view.refresh_installed_features(cx);
                    }
                    Err(error) => view.feature_manager_message = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
