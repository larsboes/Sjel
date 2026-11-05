//! Settings (⌘,): version and updates, appearance, shortcuts, accounts, the connection to the
//! shell, permissions, and features.
//!
//! A rail with the search field on top and the sections under two headings, then grouped rows on
//! the same cards Home uses:
//! the label and one line about it on the left, the control on the right, as in macOS System
//! Settings. Controls write through `Workspace`; the view keeps no independent setting values.

use std::collections::BTreeSet;
use std::fs;

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString, Window};

pub(super) use crate::update::{BUILD_TIME, RUNNING_COMMIT};
use crate::workspace::Workspace;
use sjel_gpui_ui::primitives::{list, section};
use sjel_gpui_ui::theme::{AccentColor, AppearanceSettings, Palette};

mod appearance;
mod features;
mod shortcuts;
mod system;

#[cfg(test)]
pub(crate) use shortcuts::humanize;

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

pub(super) fn recommendation_dismissal_key(feature: &str, topic: &str) -> String {
    format!("{feature}::{topic}")
}

/// A section of the page, in rail order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    General,
    Appearance,
    Shortcuts,
    Accounts,
    Connection,
    Permissions,
    Features,
    Clipper,
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
    /// Rail order: the options every install has, then the features and their settings.
    const ALL: [Self; 8] = [
        Self::General,
        Self::Appearance,
        Self::Shortcuts,
        Self::Accounts,
        Self::Connection,
        Self::Permissions,
        Self::Features,
        Self::Clipper,
    ];

    /// The rail heading the section stands under.
    fn heading(self) -> &'static str {
        match self {
            Self::Features | Self::Clipper => "Features",
            _ => "Options",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Connection => "Connection & Sync",
            Self::Accounts => "Accounts & keys",
            Self::Clipper => "Clipper",
            Self::Features => "Features",
            Self::Permissions => "macOS permissions",
            Self::Shortcuts => "Shortcuts",
        }
    }

    /// The header's line under "Settings".
    pub fn summary(self) -> &'static str {
        match self {
            Self::Appearance => "Changes apply at once and are kept for the next launch",
            Self::Connection => "The Sjel shell this window reads from",
            Self::Accounts => "Provider keys and credentials, stored in the OS keychain",
            Self::Clipper => "Templates and the Interpreter model for the Sjel Clipper",
            Self::Features => "Discover, install, and remove optional Sjel features",
            Self::Permissions => {
                "Grants only you can give in System Settings, and why Sjel needs them"
            }
            Self::General => "Version and updates of Sjel Desktop, built from your checkout",
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
    } else if matches(&["clip", "template", "interpreter", "browser"]) {
        Some(Section::Clipper)
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
    } else if matches(&[
        "permission",
        "privacy",
        "photos",
        "music",
        "system settings",
    ]) {
        Some(Section::Permissions)
    } else if matches(&["connection", "sync", "phone", "endpoint"]) {
        Some(Section::Connection)
    } else if matches(&["general", "update", "build", "version", "rebuild"]) {
        Some(Section::General)
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

pub(super) fn feature_content_query(query: &str) -> String {
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
        Section::Appearance => appearance::appearance(view.appearance_settings, palette, cx),
        Section::Connection => system::connection(view, palette, cx),
        Section::Accounts => vec![view.accounts.clone().into_any_element()],
        Section::Clipper => vec![view.clipper_settings.clone().into_any_element()],
        Section::Permissions => system::permissions(palette),
        Section::General => system::updates(view, palette, cx),
        Section::Shortcuts => shortcuts::shortcuts(
            palette,
            cx,
            &view.settings_query,
            view.settings_shortcut_group,
        ),
        Section::Features => vec![features::features(view, palette, cx)],
    };
    let recommendation_count = recommendation_count(view, cx);
    let search = view
        .settings_search
        .as_ref()
        .map(|(input, _)| Input::new(input).into_any_element());
    div()
        .size_full()
        .flex()
        .gap_8()
        .child(rail(
            view.settings_section,
            &view.settings_query,
            view.settings_shortcut_group,
            recommendation_count,
            search,
            palette,
            cx,
        ))
        .child(
            div()
                .id("settings-body")
                .flex_1()
                .min_w_0()
                .h_full()
                .track_scroll(&view.selection.scroll)
                .overflow_y_scroll()
                .vertical_scrollbar(&view.selection.scroll)
                .child(div().flex().flex_col().gap_6().pb_8().children(body)),
        )
}

fn rail(
    current: Section,
    query: &str,
    selected_shortcut_group: Option<&'static str>,
    recommendation_count: usize,
    search: Option<AnyElement>,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let mut last_heading = None;
    div()
        .id("settings-rail")
        .flex_none()
        .w(px(200.))
        .h_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_1()
        .children(search.map(|search| div().pb_2().child(search)))
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
                .flat_map(|section| {
                    let heading = (last_heading != Some(section.heading())).then(|| {
                        last_heading = Some(section.heading());
                        div()
                            .px_3()
                            .pt_3()
                            .pb_1()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(section.heading())
                            .into_any_element()
                    });
                    let selected = section == current;
                    let item =
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
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.open_settings(Some(section), cx)
                            }))
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
                            );
                    heading.into_iter().chain([item.into_any_element()])
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
                features::topic_mentioned(&prompt, topic)
                    && !view
                        .dismissed_feature_recommendations
                        .contains(&recommendation_dismissal_key(&package.name, topic))
            })
        })
        .count()
}

/// One setting: what it is and what it does on the left, its control on the right.
pub(super) fn row(
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

pub(super) fn group(title: &'static str, rows: Vec<AnyElement>, palette: Palette) -> AnyElement {
    section(title, list(rows, palette), palette).into_any_element()
}

pub(super) fn value(text: SharedString, color: u32) -> impl IntoElement {
    div().text_sm().text_color(rgb(color)).child(text)
}

pub(super) fn dot_value(text: SharedString, color: u32) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().size(px(7.)).rounded_full().bg(rgb(color)))
        .child(div().text_sm().child(text))
}

/// A row of buttons where exactly one is selected, like a macOS segmented control. `next` builds
/// the settings the picked option stands for.
pub(super) fn segmented<L: Into<SharedString>>(
    id: &'static str,
    options: impl IntoIterator<Item = (L, bool)>,
    cx: &mut Context<Workspace>,
    next: impl Fn(usize) -> AppearanceSettings + 'static,
) -> impl IntoElement {
    // The chosen option is filled with the accent and the rest are outlined: gpui-kit draws a
    // selected outline button like a pressed one, which reads the same as the others.
    ButtonGroup::new(id)
        .small()
        .children(
            options
                .into_iter()
                .enumerate()
                .map(|(ix, (label, selected))| {
                    let button = Button::new((id, ix)).label(label).selected(selected);
                    if selected {
                        button.primary()
                    } else {
                        button.outline()
                    }
                }),
        )
        .on_click(cx.listener(move |view, picked: &Vec<usize>, _, cx| {
            if let Some(&ix) = picked.first() {
                view.set_appearance(next(ix), cx);
            }
        }))
}

/// One dot per accent, the current one ringed.
pub(super) fn accent_swatches(
    settings: AppearanceSettings,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
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
    fn privacy_words_find_the_permissions_section() {
        for query in ["privacy", "Photos", "music access", "permission"] {
            assert_eq!(
                super::search_section(query, false),
                Some(Section::Permissions),
                "{query}"
            );
        }
    }

    #[test]
    fn action_names_read_as_sentences() {
        assert_eq!(humanize("GoHome"), "Go home");
        assert_eq!(humanize("OpenSoundscape"), "Open soundscape");
        assert_eq!(humanize("Refresh"), "Refresh");
    }
}
