//! Settings (⌘,): appearance, the connection to the shell, app updates, and shortcuts.
//!
//! A section rail styled like the sidebar's nav, then grouped rows on the same cards Home uses:
//! the label and one line about it on the left, the control on the right, as in macOS System
//! Settings. Every control writes back through a `Workspace` method, so the page holds no state.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, IconName, Selectable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, AsKeystroke, ClipboardItem, Context,
    Keystroke, SharedString, Window,
};

use crate::route::Page;
use crate::theme::{
    AccentColor, AppearanceSettings, BackgroundTone, GlassStyle, Palette, ThemeMode, WindowOpacity,
};
use crate::ui::{list, section};
use crate::update::{RebuildState, BUILD_TIME, RUNNING_COMMIT};
use crate::workspace::{GoTo, Workspace};

/// A section of the page, in rail order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Appearance,
    Connection,
    Updates,
    Shortcuts,
}

impl Section {
    const ALL: [Self; 4] = [
        Self::Appearance,
        Self::Connection,
        Self::Updates,
        Self::Shortcuts,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Connection => "Connection & Sync",
            Self::Updates => "Updates",
            Self::Shortcuts => "Shortcuts",
        }
    }

    /// The header's line under "Settings".
    pub fn summary(self) -> &'static str {
        match self {
            Self::Appearance => "Changes apply at once and are kept for the next launch",
            Self::Connection => "The Sjel shell this window reads from",
            Self::Updates => "Sjel Desktop builds from your checkout of the Sjel repository",
            Self::Shortcuts => "Read from the key bindings this build registers",
        }
    }
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
        Section::Updates => updates(view, palette, cx),
        Section::Shortcuts => shortcuts(palette, cx),
    };
    div()
        .size_full()
        .flex()
        .gap_8()
        .child(rail(view.settings_section, palette, cx))
        .child(
            div()
                .id("settings-body")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .child(div().flex().flex_col().gap_6().pb_8().children(body)),
        )
}

fn rail(current: Section, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(152.))
        .flex()
        .flex_col()
        .gap_1()
        .children(Section::ALL.map(|section| {
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
                .hover(move |style| style.bg(rgba(palette.raised)).text_color(rgb(palette.text)))
                .on_click(cx.listener(move |view, _, _, cx| view.open_settings(Some(section), cx)))
                .child(section.title())
        }))
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
    let tone = segmented(
        "background-tone",
        BackgroundTone::ALL.map(|t| (t.label(), settings.tone == t)),
        cx,
        move |ix| AppearanceSettings {
            tone: BackgroundTone::ALL[ix],
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
                    "Opacity",
                    Some("How much of the desktop shows through."),
                    opacity,
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
        RebuildState::Failed(error) => Some((format!("Rebuild failed: {error}"), palette.red)),
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
        rows.push(
            div()
                .px_4()
                .py_3()
                .text_xs()
                .text_color(rgb(color))
                .child(line)
                .into_any_element(),
        );
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
fn shortcuts(palette: Palette, cx: &App) -> Vec<AnyElement> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let docs = cx.action_documentation();
    let mut groups: [ShortcutGroup; 12] = [
        ("sjel::", "WORKSPACE", Vec::new()),
        ("context_page::", "TODAY / CONTEXT", Vec::new()),
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
        let label = (binding.action().as_any().downcast_ref::<GoTo>())
            .map(|GoTo(page)| format!("Go to {}", page.label()))
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
    groups
        .into_iter()
        .filter(|(_, _, rows)| !rows.is_empty())
        .map(|(_, title, rows)| {
            let rows = rows
                .into_iter()
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
                .collect();
            group(title, rows, palette)
        })
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

    /// ⌘, opens Settings, and every section renders. The endpoint is not on loopback, so no
    /// request leaves the test.
    #[gpui::test]
    fn cmd_comma_opens_settings_and_every_section_renders(cx: &mut TestAppContext) {
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
