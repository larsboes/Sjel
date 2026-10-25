//! Settings (⌘,): appearance, the connection to the shell, accounts, app updates, and shortcuts.
//!
//! A section rail styled like the sidebar's nav, then grouped rows on the same cards Home uses:
//! the label and one line about it on the left, the control on the right, as in macOS System
//! Settings. Controls write through `Workspace`; the view keeps no independent setting values.

use std::collections::BTreeSet;
use std::fs;

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, IconName, Selectable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, AsKeystroke, ClipboardItem, Context,
    Keystroke, SharedString, Window,
};

use crate::route::Page;
use crate::theme::{
    tint, AccentColor, AppearanceSettings, BackgroundTone, GlassStyle, Palette, ThemeMode,
    WindowOpacity,
};
use crate::ui::{list, note, section};
use crate::update::{RebuildState, BUILD_TIME, RUNNING_COMMIT};
use crate::workspace::{page_title, GoTo, OpenArea, Workspace};

fn recommendation_dismissals_path() -> Option<std::path::PathBuf> {
    AppearanceSettings::config_path()
        .map(|path| path.with_file_name("feature-recommendations.json"))
}

pub(crate) fn load_recommendation_dismissals() -> BTreeSet<String> {
    recommendation_dismissals_path()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub(crate) fn save_recommendation_dismissals(dismissed: &BTreeSet<String>) -> std::io::Result<()> {
    let Some(path) = recommendation_dismissals_path() else {
        return Err(std::io::Error::other(
            "Sjel config directory is unavailable",
        ));
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(dismissed).map_err(std::io::Error::other)?;
    let temp_path = path.with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&temp_path, bytes)?;
    fs::rename(temp_path, path)
}

fn recommendation_dismissal_key(feature: &str, topic: &str) -> String {
    format!("{feature}::{topic}")
}

/// A section of the page, in rail order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Appearance,
    Connection,
    Accounts,
    Features,
    Updates,
    Shortcuts,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FeatureView {
    #[default]
    Recommended,
    Explore,
    Installed,
}

impl FeatureView {
    fn title(self) -> &'static str {
        match self {
            Self::Recommended => "Recommended",
            Self::Explore => "Explore",
            Self::Installed => "Installed",
        }
    }

    const ALL: [Self; 3] = [Self::Recommended, Self::Explore, Self::Installed];
}

const SHORTCUT_GROUPS: [&str; 12] = [
    "WORKSPACE",
    "TODAY",
    "CALENDAR",
    "FINANCE",
    "PEOPLE",
    "BACKUP",
    "INVENTORY",
    "SOUNDSCAPE",
    "FEED",
    "MAIL",
    "INBOX",
    "NOTE EDITOR",
];

impl Section {
    const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Connection,
        Self::Accounts,
        Self::Features,
        Self::Updates,
        Self::Shortcuts,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Connection => "Connection & Sync",
            Self::Accounts => "Accounts & keys",
            Self::Features => "Features",
            Self::Updates => "Updates",
            Self::Shortcuts => "Shortcuts",
        }
    }

    /// The header's line under "Settings".
    pub fn summary(self) -> &'static str {
        match self {
            Self::Appearance => "Changes apply at once and are kept for the next launch",
            Self::Connection => "The Sjel shell this window reads from",
            Self::Accounts => "Provider keys and credentials, stored in the OS keychain",
            Self::Features => "Discover, install, and remove optional Sjel features",
            Self::Updates => "Sjel Desktop builds from your checkout of the Sjel repository",
            Self::Shortcuts => "Read from the key bindings this build registers",
        }
    }
}

pub(crate) fn search_section(query: &str, catalogue_has_match: bool) -> Option<Section> {
    let query = query.to_lowercase();
    if query.trim().is_empty() {
        return None;
    }
    let matches = |terms: &[&str]| terms.iter().any(|term| query.contains(term));
    if matches(&["appearance", "theme", "opacity", "accent"]) {
        Some(Section::Appearance)
    } else if matches(&[
        "account",
        "key",
        "credential",
        "provider",
        "google",
        "openai",
        "deepseek",
        "opencode",
        "nvidia",
        "gemini",
        "api",
    ]) {
        Some(Section::Accounts)
    } else if matches(&[
        "feature",
        "install",
        "capability",
        "capabilities",
        "package",
        "plugin",
        "extension",
        "installed",
        "remove",
        "recommend",
        "catalog",
        "browse",
    ]) {
        Some(Section::Features)
    } else if matches(&["connection", "sync", "phone", "endpoint"]) {
        Some(Section::Connection)
    } else if matches(&["update", "build", "version"]) {
        Some(Section::Updates)
    } else if matches(&[
        "shortcut",
        "key binding",
        "cmd",
        "command",
        "super",
        "shift",
        "option",
        "control",
        "ctrl",
        "alt",
        "escape",
        "esc",
        "enter",
        "return",
        "space",
        "backspace",
        "delete",
        "tab",
        "go to",
    ]) || query
        .chars()
        .any(|character| matches!(character, '⌘' | '⌥' | '⌃' | '⇧' | '⌫' | '⎋'))
    {
        Some(Section::Shortcuts)
    } else if catalogue_has_match {
        Some(Section::Features)
    } else {
        None
    }
}

pub(crate) fn provider_for_search(query: &str) -> Option<&'static str> {
    let query = query.to_lowercase();
    if query.contains("openai") || query.contains("open ai") {
        Some("openai")
    } else if query.contains("deepseek") || query.contains("deep seek") {
        Some("deepseek")
    } else if query.contains("opencode") || query.contains("open code") {
        Some("opencode-go")
    } else if query.contains("nvidia") {
        Some("nvidia-nim")
    } else if query.contains("gemini") || query.contains("google") {
        Some("gemini")
    } else {
        None
    }
}

pub(crate) fn feature_view_for_search(query: &str) -> Option<FeatureView> {
    let query = query.to_lowercase();
    if query.trim().is_empty() {
        return None;
    }
    if ["installed", "uninstall", "remove"]
        .iter()
        .any(|term| query.contains(term))
    {
        Some(FeatureView::Installed)
    } else if ["recommend", "suggestion"]
        .iter()
        .any(|term| query.contains(term))
    {
        Some(FeatureView::Recommended)
    } else if [
        "feature",
        "install",
        "package",
        "plugin",
        "extension",
        "catalog",
        "browse",
    ]
    .iter()
    .any(|term| query.contains(term))
    {
        Some(FeatureView::Explore)
    } else {
        None
    }
}

fn feature_content_query(query: &str) -> String {
    const NAVIGATION_WORDS: &[&str] = &[
        "settings",
        "setting",
        "feature",
        "features",
        "install",
        "installed",
        "uninstall",
        "remove",
        "package",
        "packages",
        "plugin",
        "plugins",
        "extension",
        "extensions",
        "catalog",
        "catalogue",
        "browse",
        "recommend",
        "recommendation",
        "recommendations",
        "recommended",
        "suggestion",
        "suggestions",
    ];
    query
        .to_lowercase()
        .split_whitespace()
        .filter(|word| !NAVIGATION_WORDS.contains(word))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn catalogue_matches(packages: &[crate::shell::AvailableFeature], query: &str) -> bool {
    let query = query.to_lowercase();
    !query.trim().is_empty()
        && packages.iter().any(|package| {
            package.name.to_lowercase().contains(&query)
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
}

pub fn render(
    view: &Workspace,
    palette: Palette,
    _: &mut Window,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let body = match view.settings_section {
        Section::Appearance => appearance(view.appearance_settings, palette, cx),
        Section::Connection => connection(view, palette, cx),
        Section::Accounts => vec![view.accounts.clone().into_any_element()],
        Section::Updates => updates(view, palette, cx),
        Section::Shortcuts => shortcuts(
            palette,
            cx,
            &view.settings_query,
            view.settings_shortcut_group,
        ),
        Section::Features => vec![features(view, palette, cx)],
    };
    let recommendation_count = recommendation_count(view, cx);
    div()
        .size_full()
        .flex()
        .gap_8()
        .when(view.settings_toc_visible, |layout| {
            layout.child(rail(
                view.settings_section,
                &view.settings_query,
                view.settings_shortcut_group,
                recommendation_count,
                palette,
                cx,
            ))
        })
        .child(
            div()
                .id("settings-body")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pb_3()
                        .child(if let Some((input, _)) = &view.settings_search {
                            Input::new(input).into_any_element()
                        } else {
                            div().into_any_element()
                        })
                        .child(
                            Button::new("settings-toc-toggle")
                                .ghost()
                                .small()
                                .label(if view.settings_toc_visible {
                                    "Hide contents"
                                } else {
                                    "Show contents"
                                })
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.settings_toc_visible = !view.settings_toc_visible;
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().flex().flex_col().gap_6().pb_8().children(body)),
        )
}

fn rail(
    current: Section,
    query: &str,
    selected_shortcut_group: Option<&'static str>,
    recommendation_count: usize,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(152.))
        .flex()
        .flex_col()
        .gap_1()
        .children(
            Section::ALL
                .into_iter()
                .filter(|section| {
                    query.trim().is_empty()
                        || *section == current
                        || *section == Section::Shortcuts
                        || section
                            .title()
                            .to_lowercase()
                            .contains(&query.to_lowercase())
                        || section
                            .summary()
                            .to_lowercase()
                            .contains(&query.to_lowercase())
                })
                .map(|section| {
                    let selected = section == current;
                    div()
                        .id(section.title())
                        .px_3()
                        .py(px(6.))
                        .rounded(px(7.))
                        .text_sm()
                        .when(selected, |row| row.bg(rgba(palette.raised)))
                        .text_color(rgb(if selected {
                            palette.text
                        } else {
                            palette.muted
                        }))
                        .cursor_pointer()
                        .hover(move |style| {
                            style.bg(rgba(palette.raised)).text_color(rgb(palette.text))
                        })
                        .on_click(
                            cx.listener(move |view, _, _, cx| {
                                view.open_settings(Some(section), cx)
                            }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(section.title())
                                .when(
                                    section == Section::Features && recommendation_count > 0,
                                    |row| {
                                        row.child(
                                            div()
                                                .px_1()
                                                .rounded(px(4.))
                                                .bg(rgba(palette.raised))
                                                .text_xs()
                                                .text_color(rgb(palette.accent))
                                                .child(recommendation_count.to_string()),
                                        )
                                    },
                                ),
                        )
                }),
        )
        .when(current == Section::Shortcuts, |rail| {
            rail.children(
                SHORTCUT_GROUPS
                    .into_iter()
                    .filter(|title| {
                        query.trim().is_empty()
                            || !SHORTCUT_GROUPS
                                .iter()
                                .any(|group| group.to_lowercase().contains(&query.to_lowercase()))
                            || title.to_lowercase().contains(&query.to_lowercase())
                    })
                    .map(|title| {
                        let selected = selected_shortcut_group == Some(title);
                        div()
                            .id(SharedString::from(format!("shortcut-group-{title}")))
                            .ml_2()
                            .px_3()
                            .py(px(5.))
                            .rounded(px(7.))
                            .text_xs()
                            .when(selected, |row| row.bg(rgba(palette.raised)))
                            .text_color(rgb(if selected {
                                palette.text
                            } else {
                                palette.muted
                            }))
                            .cursor_pointer()
                            .hover(move |style| {
                                style.bg(rgba(palette.raised)).text_color(rgb(palette.text))
                            })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.settings_shortcut_group = Some(title);
                                cx.notify();
                            }))
                            .child(title)
                    }),
            )
        })
}

fn recommendation_count(view: &Workspace, cx: &Context<Workspace>) -> usize {
    let Some(prompt) = view.ask.read(cx).latest_local_recommendation_prompt() else {
        return 0;
    };
    let Some(packages) = &view.available_features.value else {
        return 0;
    };
    let prompt = prompt.to_lowercase();
    packages
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
        .filter(|package| {
            package.feature.recommend_topics.iter().any(|topic| {
                topic_mentioned(&prompt, topic)
                    && !view
                        .dismissed_feature_recommendations
                        .contains(&recommendation_dismissal_key(&package.name, topic))
            })
        })
        .count()
}

/// One setting: what it is and what it does on the left, its control on the right.
fn row(
    title: impl Into<SharedString>,
    detail: Option<&str>,
    control: impl IntoElement,
    palette: Palette,
) -> AnyElement {
    // A narrow window wraps the control onto its own line, still on the right.
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x_6()
        .gap_y_2()
        .px_4()
        .py_3()
        .min_h(px(48.))
        .child(
            div()
                .flex_1()
                .min_w(px(200.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_sm().child(title.into()))
                .children(detail.map(|detail| {
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(SharedString::from(detail.to_owned()))
                })),
        )
        .child(div().flex_none().ml_auto().child(control))
        .into_any_element()
}

fn group(title: &'static str, rows: Vec<AnyElement>, palette: Palette) -> AnyElement {
    section(title, list(rows, palette), palette).into_any_element()
}

fn appearance(
    settings: AppearanceSettings,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Vec<AnyElement> {
    let theme = segmented(
        "theme-mode",
        ThemeMode::ALL.map(|mode| (mode.label(), settings.theme_mode == mode)),
        cx,
        move |ix| AppearanceSettings {
            theme_mode: ThemeMode::ALL[ix],
            ..settings
        },
    );
    let opacity = segmented(
        "window-opacity",
        WindowOpacity::ALL.map(|o| (o.short_label(), settings.opacity == o)),
        cx,
        move |ix| AppearanceSettings {
            opacity: WindowOpacity::ALL[ix],
            ..settings
        },
    );
    let sidebar_opacity = segmented(
        "sidebar-opacity",
        WindowOpacity::ALL.map(|o| (o.short_label(), settings.sidebar_opacity == o)),
        cx,
        move |ix| AppearanceSettings {
            sidebar_opacity: WindowOpacity::ALL[ix],
            ..settings
        },
    );
    let panel_opacity = segmented(
        "panel-opacity",
        WindowOpacity::ALL.map(|o| (o.short_label(), settings.panel_opacity == o)),
        cx,
        move |ix| AppearanceSettings {
            panel_opacity: WindowOpacity::ALL[ix],
            ..settings
        },
    );
    let tone = segmented(
        "background-tone",
        BackgroundTone::ALL.map(|t| (t.label(), settings.tone == t)),
        cx,
        move |ix| AppearanceSettings {
            tone: BackgroundTone::ALL[ix],
            ..settings
        },
    );
    let terminal_tone = segmented(
        "terminal-tone",
        BackgroundTone::ALL.map(|t| (t.label(), settings.terminal_tone == t)),
        cx,
        move |ix| AppearanceSettings {
            terminal_tone: BackgroundTone::ALL[ix],
            ..settings
        },
    );
    let terminal_opacity = segmented(
        "terminal-opacity",
        WindowOpacity::ALL.map(|o| (o.short_label(), settings.terminal_opacity == o)),
        cx,
        move |ix| AppearanceSettings {
            terminal_opacity: WindowOpacity::ALL[ix],
            ..settings
        },
    );
    let glass = Switch::new("liquid-glass")
        .checked(settings.glass == GlassStyle::LiquidGlass)
        .on_click(cx.listener(move |view, on: &bool, _, cx| {
            let glass = if *on {
                GlassStyle::LiquidGlass
            } else {
                GlassStyle::Flat
            };
            view.set_appearance(AppearanceSettings { glass, ..settings }, cx);
        }));
    vec![
        group(
            "THEME",
            vec![
                row(
                    "Mode",
                    Some("Follow macOS, or keep Sjel light or dark."),
                    theme,
                    palette,
                ),
                row(
                    "Accent colour",
                    Some("Selection, focus rings and primary buttons."),
                    accent_swatches(settings, cx),
                    palette,
                ),
            ],
            palette,
        ),
        group(
            "WINDOW",
            vec![
                row(
                    "Liquid Glass sidebar",
                    Some("Draws the sidebar on native glass. Off draws it solid."),
                    glass,
                    palette,
                ),
                row(
                    "Canvas opacity",
                    Some("How much of the desktop shows through the main window."),
                    opacity,
                    palette,
                ),
                row(
                    "Left sidebar opacity",
                    Some("Set the navigation surface separately from the canvas."),
                    sidebar_opacity,
                    palette,
                ),
                row(
                    "More panel opacity",
                    Some("Set the right-side panel surface separately."),
                    panel_opacity,
                    palette,
                ),
                row(
                    "Background tone",
                    Some("Tints the window. System uses the macOS colour."),
                    tone,
                    palette,
                ),
            ],
            palette,
        ),
        group(
            "TERMINAL",
            vec![
                row(
                    "Background",
                    Some("The terminal grid's colour. System follows the theme."),
                    terminal_tone,
                    palette,
                ),
                row(
                    "Background opacity",
                    Some("How much of the window shows through the grid."),
                    terminal_opacity,
                    palette,
                ),
            ],
            palette,
        ),
    ]
}

fn connection(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let (status_color, status) = crate::sidebar::status(&view.health, palette);
    let (read_path, read_color) = crate::sidebar::storage(view.has_direct_db(), palette);
    let read_detail = if view.has_direct_db() {
        "Reads the local database directly. Writes go through the shell."
    } else {
        "Reads and writes through the shell's HTTP API."
    };
    let capabilities = view.health.value.as_ref().map_or_else(
        || "Unknown".to_owned(),
        |health| {
            let total = health.capabilities.len();
            match health.down().len() {
                0 => format!("All {total} running"),
                down => format!("{} of {total} running", total - down),
            }
        },
    );
    vec![
        group(
            "SHELL",
            vec![
                row(
                    "Endpoint",
                    Some("Set with SJEL_STATUS_URL before launch."),
                    value(view.endpoint(), palette.text),
                    palette,
                ),
                row("Status", None, dot_value(status, status_color), palette),
                row(
                    "Capabilities",
                    None,
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(value(capabilities.into(), palette.text))
                        .child(
                            Button::new("show-capabilities")
                                .ghost()
                                .small()
                                .label("Show")
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.go(Page::Capabilities, cx)),
                                ),
                        ),
                    palette,
                ),
                row(
                    "Read path",
                    Some(read_detail),
                    dot_value(read_path.into(), read_color),
                    palette,
                ),
            ],
            palette,
        ),
        group(
            "PHONE SYNC",
            vec![row(
                "iPhone",
                Some(
                    "A paired iPhone keeps its own copy and queues writes offline. It syncs \
                     when it finds this Mac on the network; its queue lives on the phone.",
                ),
                Button::new("open-dashboard")
                    .outline()
                    .small()
                    .icon(IconName::ExternalLink)
                    .label("Dashboard")
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.open_dashboard("/".into(), window, cx)
                    })),
                palette,
            )],
            palette,
        ),
    ]
}

fn features(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
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

fn installed_features(
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
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.cancel_feature_uninstall(cx)),
                                ),
                            )
                            .into_any_element()
                    } else {
                        Button::new(SharedString::from(format!("feature-uninstall-{name}")))
                            .ghost()
                            .small()
                            .label("Uninstall…")
                            .loading(busy)
                            .disabled(busy || package.integrity != "verified")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.request_feature_uninstall(name.clone(), cx)
                            }))
                            .into_any_element()
                    };
                    row(
                        format!("{} · {}", package.feature.title, package.feature.category),
                        Some(&package.feature.summary),
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(value(
                                format!("{} · {}", package.version, integrity.0).into(),
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
        None if view.installed_features.loading => crate::ui::placeholder(72.),
        None => note(
            view.installed_features.error.as_ref().map_or_else(
                || "Package inventory is not available in this runtime.".to_owned(),
                ToString::to_string,
            ),
            palette.muted,
        ),
    };
    section("Installed features", installed_body, palette).into_any_element()
}

fn recommendations(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
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
    let body = if recommendations.is_empty() {
        return recommendation_empty(
            "No new catalogue feature matches the latest local Ask request. Nothing is installed automatically.",
            palette,
            cx,
        );
    } else {
        list(
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
        )
    };
    section("Recommended", body, palette).into_any_element()
}

fn recommendation_empty(
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

fn topic_mentioned(prompt: &str, topic: &str) -> bool {
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

fn explore_features(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
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
        None if view.available_features.loading => crate::ui::placeholder(96.),
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

fn updates(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let (repo, repo_color) = match &view.update_info {
        None => ("Checking…".to_owned(), palette.muted),
        Some(info) if info.commits_behind > 0 => (
            format!(
                "{} · {} newer commit{}",
                info.latest_commit,
                info.commits_behind,
                if info.commits_behind == 1 { "" } else { "s" }
            ),
            palette.amber,
        ),
        Some(info) if info.has_uncommitted_changes => (
            format!(
                "{} · up to date, with uncommitted changes",
                info.latest_commit
            ),
            palette.muted,
        ),
        Some(info) => (
            format!("{} · up to date", info.latest_commit),
            palette.green,
        ),
    };
    let building = view.rebuild_state == RebuildState::Building;
    let outcome = match &view.rebuild_state {
        RebuildState::Idle => None,
        RebuildState::Building => Some((
            "Building. Sjel restarts on the new build when it is done.".to_owned(),
            palette.accent,
        )),
        RebuildState::Success(message) => Some((message.clone(), palette.green)),
        RebuildState::Failed(failure) => Some((failure.headline(), palette.red)),
    };

    let mut rows = vec![
        row(
            "This build",
            Some(BUILD_TIME),
            value(RUNNING_COMMIT.into(), palette.text),
            palette,
        ),
        row(
            "Repository",
            None,
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(value(repo.into(), repo_color))
                .child(
                    Button::new("check-updates")
                        .ghost()
                        .small()
                        .icon(IconName::RotateCw)
                        .tooltip("Check now")
                        .on_click(cx.listener(|view, _, _, cx| view.check_updates(cx))),
                ),
            palette,
        ),
        row(
            "Rebuild & install",
            Some("Runs apps/gpui/install, then restarts Sjel on the new build."),
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    Button::new("copy-install")
                        .ghost()
                        .small()
                        .icon(IconName::Copy)
                        .tooltip("Copy the command")
                        .on_click(|_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                "apps/gpui/install".to_owned(),
                            ));
                        }),
                )
                .child(
                    Button::new("rebuild")
                        .primary()
                        .small()
                        .label("Rebuild")
                        .loading(building)
                        .disabled(building)
                        .on_click(
                            cx.listener(|view, _, window, cx| view.trigger_rebuild(window, cx)),
                        ),
                ),
            palette,
        ),
    ];
    if let Some((line, color)) = outcome {
        let mut block = div()
            .px_4()
            .py_3()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().text_color(rgb(color)).child(line));
        if let RebuildState::Failed(failure) = &view.rebuild_state {
            // The first error is what a person acts on; the log behind the button is for triage.
            if let Some(diagnostic) = &failure.first {
                block = block.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(diagnostic.message.clone()),
                );
            }
            block = block.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("rebuild-log")
                            .ghost()
                            .small()
                            .label(if view.show_rebuild_log {
                                "Hide full log"
                            } else {
                                "Full log"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.show_rebuild_log = !view.show_rebuild_log;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("rebuild-log-copy")
                            .ghost()
                            .small()
                            .icon(IconName::Copy)
                            .tooltip("Copy the log")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                let RebuildState::Failed(failure) = &view.rebuild_state else {
                                    return;
                                };
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    failure.log.clone(),
                                ));
                            })),
                    ),
            );
            if view.show_rebuild_log {
                block = block.child(
                    div()
                        .mt_1()
                        .max_h(px(320.))
                        .overflow_y_scrollbar()
                        .rounded_md()
                        .bg(rgba(tint(palette.raised, 0x40)))
                        .p_2()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(failure.log.clone()),
                );
            }
        }
        rows.push(block.into_any_element());
    }

    let mut groups = vec![group("SJEL DESKTOP", rows, palette)];
    if let Some(info) = view.update_info.as_ref().filter(|i| !i.commits.is_empty()) {
        let commits = info
            .commits
            .iter()
            .map(|commit| {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .text_sm()
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(rgb(palette.accent))
                            .child(commit.sha.clone()),
                    )
                    .child(div().min_w_0().truncate().child(commit.message.clone()))
                    .into_any_element()
            })
            .collect();
        groups.push(group("NEWER COMMITS", commits, palette));
    }
    groups
}

/// One row of the shortcuts list: the action's label and the strokes bound to it.
type ShortcutRow = (String, Vec<Keystroke>);

/// One group of them: the action-name prefix that selects it, its heading, and its rows.
type ShortcutGroup = (&'static str, &'static str, Vec<ShortcutRow>);

/// Every binding this app registers, read from the keymap itself so the list cannot drift from
/// `workspace::bind_keys` and `editor::bind_keys`.
fn shortcuts(
    palette: Palette,
    cx: &App,
    query: &str,
    selected_group: Option<&str>,
) -> Vec<AnyElement> {
    let shortcut_query = if ["shortcut", "shortcuts", "key binding", "key bindings"]
        .iter()
        .any(|term| query.trim().eq_ignore_ascii_case(term))
    {
        ""
    } else {
        query
    };
    let normalized_query = normalize_shortcut_query(shortcut_query);
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let docs = cx.action_documentation();
    let mut groups: [ShortcutGroup; 12] = [
        ("sjel::", "WORKSPACE", Vec::new()),
        ("context_page::", "TODAY", Vec::new()),
        ("calendar::", "CALENDAR", Vec::new()),
        ("finance_page::", "FINANCE", Vec::new()),
        ("people_page::", "PEOPLE", Vec::new()),
        ("backup::", "BACKUP", Vec::new()),
        ("inventory::", "INVENTORY", Vec::new()),
        ("soundscape::", "SOUNDSCAPE", Vec::new()),
        ("feed::", "FEED", Vec::new()),
        ("mail::", "MAIL", Vec::new()),
        ("inbox::", "INBOX", Vec::new()),
        ("editor::", "NOTE EDITOR", Vec::new()),
    ];
    for binding in keymap.bindings() {
        let name = binding.action().name();
        let Some((_, _, rows)) = groups.iter_mut().find(|(ns, _, _)| name.starts_with(ns)) else {
            continue;
        };
        let Some(stroke) = binding.keystrokes().first() else {
            continue;
        };
        // Every page shares `GoTo`, so its label comes from the page, not the action's doc.
        // Every page shares `GoTo` and every area `OpenArea`, so their labels come from the
        // table both read, never from the action's doc: `Sjel › Packs`, and `Go to Sjel`.
        let label = (binding.action().as_any().downcast_ref::<GoTo>())
            .map(|GoTo(page)| format!("Go to {}", page_title(*page)))
            .or_else(|| {
                (binding.action().as_any().downcast_ref::<OpenArea>())
                    .map(|OpenArea(area)| format!("Go to {}", area.label()))
            })
            .or_else(|| {
                docs.get(name)
                    .and_then(|doc| doc.lines().next())
                    .map(|doc| doc.trim_end_matches('.').to_owned())
            })
            .unwrap_or_else(|| humanize(name.rsplit("::").next().unwrap_or(name)));
        let stroke = stroke.as_keystroke().clone();
        match rows.iter_mut().find(|(l, _)| *l == label) {
            Some((_, strokes)) => strokes.push(stroke),
            None => rows.push((label, vec![stroke])),
        }
    }
    let matching: Vec<AnyElement> = groups
        .into_iter()
        .filter(|(_, title, rows)| {
            !rows.is_empty() && selected_group.map_or(true, |selected| selected == *title)
        })
        .filter_map(|(_, title, rows)| {
            let rows = rows
                .into_iter()
                .filter(|(label, strokes)| {
                    shortcut_query.trim().is_empty()
                        || label
                            .to_lowercase()
                            .contains(&shortcut_query.to_lowercase())
                        || strokes.iter().any(|stroke| {
                            normalize_shortcut_query(&stroke.to_string())
                                .to_lowercase()
                                .contains(&normalized_query)
                        })
                })
                .map(|(label, strokes)| {
                    row(
                        label,
                        None,
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .children(strokes.into_iter().map(Kbd::new)),
                        palette,
                    )
                })
                .collect::<Vec<_>>();
            (!rows.is_empty()).then(|| group(title, rows, palette))
        })
        .collect();
    if matching.is_empty() {
        vec![note("No shortcuts match this search.", palette.muted).into_any_element()]
    } else {
        matching
    }
}

fn normalize_shortcut_query(query: &str) -> String {
    let normalized = query
        .to_lowercase()
        .replace("command", "cmd")
        .replace("super", "cmd")
        .replace("option", "alt")
        .replace("control", "ctrl")
        .replace("delete", "backspace")
        .replace('⌘', "cmd-")
        .replace('⌥', "alt-")
        .replace('⌃', "ctrl-")
        .replace('⇧', "shift-")
        .replace('⌫', "backspace")
        .replace('⎋', "escape")
        .replace("cmd ", "cmd-")
        .replace("alt ", "alt-")
        .replace("ctrl ", "ctrl-")
        .replace("shift ", "shift-");
    let normalized = if normalized.trim() == "esc" {
        "escape".to_owned()
    } else {
        normalized.replace("esc ", "escape ")
    };
    normalized
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

/// `OpenInDashboard` → "Open in dashboard".
fn humanize(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn value(text: SharedString, color: u32) -> impl IntoElement {
    div().text_sm().text_color(rgb(color)).child(text)
}

fn dot_value(text: SharedString, color: u32) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().size(px(7.)).rounded_full().bg(rgb(color)))
        .child(div().text_sm().child(text))
}

/// A row of buttons where exactly one is selected, like a macOS segmented control. `next` builds
/// the settings the picked option stands for.
fn segmented<const N: usize>(
    id: &'static str,
    options: [(&'static str, bool); N],
    cx: &mut Context<Workspace>,
    next: impl Fn(usize) -> AppearanceSettings + 'static,
) -> impl IntoElement {
    ButtonGroup::new(id)
        .outline()
        .small()
        .children(
            options.iter().enumerate().map(|(ix, (label, selected))| {
                Button::new((id, ix)).label(*label).selected(*selected)
            }),
        )
        .on_click(cx.listener(move |view, picked: &Vec<usize>, _, cx| {
            if let Some(&ix) = picked.first() {
                view.set_appearance(next(ix), cx);
            }
        }))
}

/// One dot per accent, the current one ringed.
fn accent_swatches(settings: AppearanceSettings, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(AccentColor::ALL.map(|accent| {
            let selected = settings.accent == accent;
            div()
                .id(SharedString::from(format!("accent-{}", accent.label())))
                .size(px(24.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .border_2()
                .border_color(if selected {
                    rgb(accent.swatch_rgb())
                } else {
                    gpui_kit::transparent_black().into()
                })
                .cursor_pointer()
                .child(
                    div()
                        .size(px(14.))
                        .rounded_full()
                        .bg(rgb(accent.swatch_rgb())),
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.set_appearance(AppearanceSettings { accent, ..settings }, cx)
                }))
        }))
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use gpui_kit::component::Root;
    use gpui_kit::AppContext as _;

    use super::{humanize, Section};
    use crate::route::Page;
    use crate::workspace::{bind_keys, Workspace};

    /// ⌘, opens Settings, and every section renders.
    ///
    /// Building the workspace starts the pages' own reads against `http://example.invalid`. They fail
    /// at once, but the failure arrives on the HTTP runtime's thread, so the deterministic scheduler
    /// needs `allow_parking` to accept the test as one that touches real I/O.
    #[gpui::test]
    fn cmd_comma_opens_settings_and_every_section_renders(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            bind_keys(cx);
        });
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| Workspace::new("http://example.invalid", window, cx));
            found.replace(Some(view.clone()));
            Root::new(view, window, cx)
        });
        let view = slot.take().expect("workspace built");
        cx.simulate_keystrokes("cmd-,");
        view.read_with(cx, |view, _| assert_eq!(view.page, Page::Settings));
        for section in Section::ALL {
            view.update(cx, |view, cx| view.open_settings(Some(section), cx));
            cx.run_until_parked();
            view.read_with(cx, |view, _| assert_eq!(view.settings_section, section));
        }
    }

    #[test]
    fn action_names_read_as_sentences() {
        assert_eq!(humanize("GoHome"), "Go home");
        assert_eq!(humanize("OpenInDashboard"), "Open in dashboard");
        assert_eq!(humanize("Refresh"), "Refresh");
    }
}
