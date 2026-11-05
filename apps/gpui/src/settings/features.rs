//! Feature package explorer, installed extensions management, and Ask topic recommendations.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::{Disableable, Selectable, Sizable};
use gpui_kit::{div, prelude::*, AnyElement, Context, IntoElement, SharedString};
use sjel_gpui_ui::primitives::{list, note, placeholder, section};
use sjel_gpui_ui::theme::Palette;

use crate::route::Page;
use crate::workspace::Workspace;
use super::{
    feature_content_query, group, recommendation_dismissal_key, row, value, FeatureView,
};

pub(super) fn features(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let tabs = ButtonGroup::new("feature-manager-view")
        .outline()
        .small()
        .children(FeatureView::ALL.into_iter().map(|feature_view| {
            Button::new(feature_view.title())
                .label(feature_view.title())
                .selected(view.settings_feature_view == feature_view)
        }))
        .on_click(cx.listener(|view, picked: &Vec<usize>, _, cx| {
            if let Some(&index) = picked.first() {
                if let Some(feature_view) = FeatureView::ALL.get(index).copied() {
                    view.settings_feature_view = feature_view;
                    view.settings_feature_detail = None;
                    cx.notify();
                }
            }
        }));
    let refresh = Button::new("feature-inventory-refresh")
        .ghost()
        .small()
        .label("Refresh inventory")
        .loading(view.installed_features.loading)
        .disabled(view.installed_features.loading)
        .on_click(cx.listener(|view, _, _, cx| {
            view.refresh_installed_features(cx);
            view.refresh_feature_catalogue(cx);
        }));
    let content = match view.settings_feature_view {
        FeatureView::Recommended => recommendations(view, palette, cx),
        FeatureView::Explore => explore_features(view, palette, cx),
        FeatureView::Installed => installed_features(view, palette, cx),
    };

    let manager_message = view.feature_manager_message.as_ref().map(|message| {
        note(
            message.clone(),
            if message.starts_with("Feature removed.") {
                palette.green
            } else {
                palette.red
            },
        )
    });
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(tabs)
                .child(refresh),
        )
        .children(manager_message)
        .child(content)
        .child(group(
            "CAPABILITIES",
            vec![row(
                "Runtime services and health",
                Some("Enable capabilities, inspect service health, and start or stop services."),
                Button::new("settings-open-capabilities")
                    .outline()
                    .small()
                    .label("Open Capabilities")
                    .on_click(cx.listener(|view, _, _, cx| view.go(Page::Capabilities, cx))),
                palette,
            )],
            palette,
        ))
        .into_any_element()
}

pub(super) fn installed_features(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let query = feature_content_query(&view.settings_query);
    let installed_body = match &view.installed_features.value {
        Some(packages) if !packages.is_empty() => {
            let rows: Vec<_> = packages
                .iter()
                .filter(|package| {
                    query.is_empty()
                        || package.name.to_lowercase().contains(&query)
                        || package.feature.title.to_lowercase().contains(&query)
                        || package.feature.summary.to_lowercase().contains(&query)
                        || package.feature.category.to_lowercase().contains(&query)
                        || package.version.to_lowercase().contains(&query)
                })
                .map(|package| {
                    let name = package.name.clone();
                    let busy_key = format!("package-uninstall:{name}");
                    let busy = view.busy.contains(&busy_key);
                    let integrity = match package.integrity.as_str() {
                        "verified" => ("files match receipt", palette.green),
                        _ => ("files differ from receipt", palette.red),
                    };
                    let action = if view.feature_uninstall_confirmation.as_deref()
                        == Some(package.name.as_str())
                    {
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!(
                                    "feature-uninstall-confirm-{name}"
                                )))
                                .small()
                                .label("Uninstall and keep data")
                                .loading(busy)
                                .disabled(busy)
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |view, _, _, cx| view.uninstall_feature(name.clone(), cx)
                                })),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "feature-uninstall-cancel-{name}"
                                )))
                                .ghost()
                                .small()
                                .label("Cancel")
                                .disabled(busy)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.cancel_feature_uninstall(cx)
                                })),
                            )
                    } else {
                        div().child(
                            Button::new(SharedString::from(format!("feature-uninstall-{name}")))
                                .ghost()
                                .small()
                                .label("Uninstall…")
                                .loading(busy)
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.request_feature_uninstall(name.clone(), cx)
                                })),
                        )
                    };
                    row(
                        package.feature.title.clone(),
                        Some(&package.feature.summary),
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(value(
                                format!("v{} · {}", package.version, integrity.0).into(),
                                integrity.1,
                            ))
                            .child(action),
                        palette,
                    )
                })
                .collect();
            if rows.is_empty() {
                note("No installed features match this search.", palette.muted)
            } else {
                list(rows, palette)
            }
        }
        Some(_) => note("No optional feature packages are installed.", palette.muted),
        None if view.installed_features.loading => placeholder(96.),
        None => note(
            view.installed_features.error.as_ref().map_or_else(
                || "Feature inventory has not been loaded.".to_owned(),
                ToString::to_string,
            ),
            palette.muted,
        ),
    };
    section("Installed features", installed_body, palette).into_any_element()
}

pub(super) fn recommendations(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let prompt = view.ask.read(cx).latest_local_recommendation_prompt();
    let Some(prompt) = prompt else {
        return recommendation_empty(
            "Recommendations use local Ask requests only. Browse the catalogue whenever you like.",
            palette,
            cx,
        );
    };
    let Some(packages) = &view.available_features.value else {
        return section(
            "Recommended",
            note(
                "Local request signals are ready. The official catalog is still loading.",
                palette.muted,
            ),
            palette,
        )
        .into_any_element();
    };
    let prompt = prompt.to_lowercase();
    let query = feature_content_query(&view.settings_query);
    let recommendations: Vec<_> = packages
        .iter()
        .filter(|package| {
            !view
                .installed_features
                .value
                .as_ref()
                .is_some_and(|installed| {
                    installed.iter().any(|feature| feature.name == package.name)
                })
        })
        .filter_map(|package| {
            package
                .feature
                .recommend_topics
                .iter()
                .find(|topic| {
                    topic_mentioned(&prompt, topic)
                        && !view
                            .dismissed_feature_recommendations
                            .contains(&recommendation_dismissal_key(&package.name, topic))
                })
                .map(|topic| (package, topic))
        })
        .filter(|(package, topic)| {
            query.is_empty()
                || package.name.to_lowercase().contains(&query)
                || package.feature.title.to_lowercase().contains(&query)
                || topic.to_lowercase().contains(&query)
        })
        .collect();
    if recommendations.is_empty() {
        recommendation_empty(
            "No new catalogue feature matches the latest local Ask request. Nothing is installed automatically.",
            palette,
            cx,
        )
    } else {
        let body = list(
            recommendations
                .into_iter()
                .map(|(package, topic)| {
                    let availability = package
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("available in Explore");
                    let feature_name = package.name.clone();
                    let topic_name = topic.clone();
                    row(
                        package.feature.title.clone(),
                        Some(&package.feature.summary),
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(value(
                                format!("Matches your local Ask topic ‘{topic}’ · {availability}")
                                    .into(),
                                if package.installable {
                                    palette.accent
                                } else {
                                    palette.muted
                                },
                            ))
                            .child(
                                Button::new(SharedString::from(format!(
                                    "feature-recommendation-dismiss-{}",
                                    package.name
                                )))
                                .ghost()
                                .small()
                                .label("Dismiss")
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.dismiss_feature_recommendation(
                                            feature_name.clone(),
                                            topic_name.clone(),
                                            cx,
                                        );
                                    },
                                )),
                            ),
                        palette,
                    )
                })
                .collect(),
            palette,
        );
        section("Recommended", body, palette).into_any_element()
    }
}

pub(super) fn recommendation_empty(
    message: &str,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    section(
        "Recommended",
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap_3()
            .child(note(message, palette.muted))
            .child(
                Button::new("recommended-browse-features")
                    .outline()
                    .small()
                    .label("Browse features")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.settings_feature_view = FeatureView::Explore;
                        cx.notify();
                    })),
            )
            .into_any_element(),
        palette,
    )
    .into_any_element()
}

pub(super) fn topic_mentioned(prompt: &str, topic: &str) -> bool {
    let prompt_words: Vec<_> = prompt
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let topic_words: Vec<_> = topic
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    !topic_words.is_empty()
        && prompt_words
            .windows(topic_words.len())
            .any(|window| window == topic_words)
}

pub(super) fn explore_features(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let body = match &view.available_features.value {
        Some(packages) => {
            let query = feature_content_query(&view.settings_query);
            let matches: Vec<_> = packages
                .iter()
                .filter(|package| {
                    query.is_empty()
                        || package.name.to_lowercase().contains(&query)
                        || package.feature.title.to_lowercase().contains(&query)
                        || package.feature.summary.to_lowercase().contains(&query)
                        || package.feature.category.to_lowercase().contains(&query)
                        || package
                            .feature
                            .recommend_topics
                            .iter()
                            .any(|topic| topic.to_lowercase().contains(&query))
                        || package
                            .feature
                            .data_access
                            .iter()
                            .any(|data| data.to_lowercase().contains(&query))
                })
                .collect();
            if matches.is_empty() {
                note(
                    if query.is_empty() {
                        "The official catalog has no feature packages for this platform."
                    } else {
                        "No catalog features match this search."
                    },
                    palette.muted,
                )
            } else {
                let cards = matches
                    .into_iter()
                    .map(|package| {
                        let name = package.name.clone();
                        let detail_name = name.clone();
                        let expanded = view.settings_feature_detail.as_deref() == Some(name.as_str());
                        let detail_button = Button::new(SharedString::from(format!(
                            "feature-details-{name}"
                        )))
                        .ghost()
                        .small()
                        .label(if expanded { "Hide details" } else { "Details" })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.settings_feature_detail = if expanded {
                                None
                            } else {
                                Some(detail_name.clone())
                            };
                            cx.notify();
                        }));
                        let status = package
                            .unavailable_reason
                            .as_deref()
                            .unwrap_or("Ready for this runtime");
                        let status_color = if package.installable {
                            palette.green
                        } else {
                            palette.amber
                        };
                        let heading = row(
                            format!("{} · {}", package.feature.title, package.feature.category),
                            Some(&package.feature.summary),
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(value(status.into(), status_color))
                                .child(detail_button),
                            palette,
                        );
                        let details = if expanded {
                            let requirements = if package.requires.is_empty() {
                                "None".to_owned()
                            } else {
                                package.requires.join(", ")
                            };
                            let data_access = if package.feature.data_access.is_empty() {
                                "No declared data access".to_owned()
                            } else {
                                package.feature.data_access.join(", ")
                            };
                            let topics = package.feature.recommend_topics.join(", ");
                            let status_detail = package.unavailable_reason.as_deref().unwrap_or(
                                "Release and runtime checks passed.",
                            );
                            let already_installed = view
                                .installed_features
                                .value
                                .as_ref()
                                .is_some_and(|installed| {
                                    installed.iter().any(|feature| feature.name == name)
                                });
                            let install_key = format!("package-install:{name}");
                            let install_busy = view.busy.contains(&install_key);
                            let package_version = package.version.clone();
                            let package_sha256 = package.sha256.clone();
                            let install_action = if already_installed {
                                Button::new(SharedString::from(format!("feature-installed-{name}")))
                                    .ghost()
                                    .small()
                                    .label("Installed · enable in Capabilities")
                                    .disabled(true)
                                    .into_any_element()
                            } else if !package.installable {
                                div().into_any_element()
                            } else if view.feature_install_confirmation.as_deref()
                                == Some(name.as_str())
                            {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "feature-install-confirm-{name}"
                                        )))
                                        .primary()
                                        .small()
                                        .label("Install package")
                                        .loading(install_busy)
                                        .disabled(install_busy)
                                        .on_click(cx.listener({
                                            let name = name.clone();
                                            let version = package_version.clone();
                                            let sha256 = package_sha256.clone();
                                            move |view, _, _, cx| {
                                                view.install_feature(
                                                    name.clone(),
                                                    version.clone(),
                                                    sha256.clone(),
                                                    cx,
                                                )
                                            }
                                        })),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "feature-install-cancel-{name}"
                                        )))
                                        .ghost()
                                        .small()
                                        .label("Cancel")
                                        .disabled(install_busy)
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.cancel_feature_install(cx)
                                        })),
                                    )
                                    .into_any_element()
                            } else {
                                Button::new(SharedString::from(format!("feature-install-{name}")))
                                    .primary()
                                    .small()
                                    .label("Install…")
                                    .loading(install_busy)
                                    .disabled(install_busy)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.request_feature_install(name.clone(), cx)
                                    }))
                                    .into_any_element()
                            };
                            div()
                                .ml_4()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(row(
                                    "Release",
                                    None,
                                    value(
                                        format!(
                                            "{} · {} · {:.1} MB",
                                            package.version,
                                            package.platform,
                                            package.bytes as f64 / (1024. * 1024.)
                                        )
                                        .into(),
                                        palette.muted,
                                    ),
                                    palette,
                                ))
                                .child(row(
                                    "Requires",
                                    None,
                                    value(requirements.into(), palette.text),
                                    palette,
                                ))
                                .child(row(
                                    "Data access",
                                    None,
                                    value(data_access.into(), palette.text),
                                    palette,
                                ))
                                .child(row(
                                    "Suggested for",
                                    None,
                                    value(topics.into(), palette.muted),
                                    palette,
                                ))
                                .child(row(
                                    "Availability",
                                    None,
                                    value(status_detail.into(), status_color),
                                    palette,
                                ))
                                .child(row(
                                    "Install",
                                    Some("The package starts disabled and does not enable or start a service."),
                                    install_action,
                                    palette,
                                ))
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        };
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(heading)
                            .child(details)
                            .into_any_element()
                    })
                    .collect();
                list(cards, palette)
            }
        }
        None if view.available_features.loading => placeholder(96.),
        None => note(
            view.available_features.error.as_ref().map_or_else(
                || "Official feature catalog has not been loaded.".to_owned(),
                ToString::to_string,
            ),
            palette.muted,
        ),
    };
    section("Explore features", body, palette).into_any_element()
}
