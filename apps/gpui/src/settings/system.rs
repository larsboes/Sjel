//! System and connectivity settings: Shell connection, macOS permissions, and app updates/rebuilding.

use std::path::PathBuf;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, ClipboardItem, Context, Div, ExternalDragPayload,
    FileDragPaths, IntoElement, Window,
};
use sjel_gpui_ui::ely::buttons::{Button, ButtonVariant};
use sjel_gpui_ui::ely::primitives::{Icon, IconName, Tooltip};
use sjel_gpui_ui::ely::theme::{ControlSize, IconSize};
use sjel_gpui_ui::theme::{tint, Palette};

use super::{dot_value, group, row, value, BUILD_TIME, RUNNING_COMMIT};
use crate::permissions::{HelperStatus, FULL_DISK_ACCESS_PANE};
use crate::route::Page;
use crate::update::RebuildState;
use crate::workspace::Workspace;

pub(super) fn connection(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Vec<AnyElement> {
    let (status_color, status) = crate::sidebar::status(&view.health, palette);
    let (read_path, read_color) = crate::sidebar::storage(palette);
    let dev = sjel_gpui_core::environment::current().dev;
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
                row(
                    "Node",
                    Some(if dev {
                        "The seeded demo node. Its data is synthetic."
                    } else {
                        "Open the app on the seeded demo node (sjel demo up)."
                    }),
                    Button::new(
                        "switch-node",
                        if dev {
                            "Back to live"
                        } else {
                            "Open on dev node"
                        },
                    )
                    .size(ControlSize::Sm)
                    .loading(view.busy.contains("switch-node"))
                    .on_click(cx.listener(|view, _, window, cx| view.switch_node(window, cx))),
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
                            Button::new("show-capabilities", "Show")
                                .variant(ButtonVariant::Ghost)
                                .size(ControlSize::Sm)
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.go(Page::Capabilities, cx)),
                                ),
                        ),
                    palette,
                ),
                row(
                    "Read path",
                    Some("Reads and writes through the shell's HTTP API."),
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
                Button::new("open-setup", "Pairing")
                    .variant(ButtonVariant::Outline)
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(|view, _, _, cx| view.go(Page::Setup, cx))),
                palette,
            )],
            palette,
        ),
    ]
}

pub(super) fn permissions(helper: &HelperStatus, palette: Palette) -> Vec<AnyElement> {
    if !cfg!(target_os = "macos") {
        return vec![row(
            "Full Disk Access",
            Some("This permission is only available on macOS."),
            div(),
            palette,
        )];
    }

    let (detail, control) = match helper {
        HelperStatus::Installed(path) => {
            let reveal = path.clone();
            (
                "Drag Sjel.app into the Full Disk Access list, then switch on Sjel.",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(helper_drag(path.clone(), palette))
                    .child(
                        Button::new("reveal-fda-helper", "Show in Finder")
                            .variant(ButtonVariant::Ghost)
                            .size(ControlSize::Sm)
                            .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
                    ),
            )
        }
        HelperStatus::Checking => ("Checking for the installed helper…", div()),
        HelperStatus::Unavailable => (
            "Sjel.app is not installed for background access. Run sjel fda-launcher install, then reopen this section.",
            div(),
        ),
    };
    vec![group(
        "FULL DISK ACCESS",
        vec![
            row(
                "Storage history",
                Some("One grant lets Sjel’s background job measure disk usage, even when both apps are closed."),
                Button::new("open-fda-pane", "Open Full Disk Access")
                    .variant(ButtonVariant::Primary)
                    .size(ControlSize::Sm)
                    .on_click(|_, _, cx| cx.open_url(FULL_DISK_ACCESS_PANE)),
                palette,
            ),
            row("Add Sjel", Some(detail), control, palette),
            row(
                "Switch existing jobs",
                Some("After enabling Sjel, run sjel fda-launcher migrate. Remove the old sjel-fda-launcher entry only after migration succeeds."),
                div(),
                palette,
            ),
        ],
        palette,
    )]
}

fn helper_drag(path: PathBuf, palette: Palette) -> impl IntoElement {
    helper_token(palette)
        .id("drag-fda-helper")
        .cursor_grab()
        .hover(move |style| style.bg(rgba(palette.raised)))
        .on_drag(path, move |_, _, _, cx| {
            cx.new(|_| HelperDragPreview(palette))
        })
        .external_drag_payload(|path: &PathBuf, _, _| {
            Some(ExternalDragPayload::Files(FileDragPaths::new([(
                path.clone(),
                true,
            )])))
        })
}

fn helper_token(palette: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .text_sm()
        .child(Icon::new(IconName::GripVertical).size(IconSize::Sm))
        .child("Drag Sjel.app")
}

struct HelperDragPreview(Palette);

impl Render for HelperDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        helper_token(self.0)
    }
}

pub(super) fn updates(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Vec<AnyElement> {
    let (repo, repo_color) = match &view.update_info {
        None => ("Checking…".to_owned(), palette.muted),
        Some(info) if !info.can_rebuild => (
            format!(
                "{} · checkout does not contain this build",
                info.latest_commit
            ),
            palette.amber,
        ),
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

    let behind = view
        .update_info
        .as_ref()
        .is_some_and(|info| info.commits_behind > 0);
    let rebuild_blocked = view
        .update_info
        .as_ref()
        .is_some_and(|info| !info.can_rebuild);
    let version_action = if behind {
        div()
            .id("update-now-tip")
            .tooltip(Tooltip::text("Builds, installs, and restarts Sjel Desktop"))
            .child(
                Button::new("update-now", "Update")
                    .variant(ButtonVariant::Primary)
                    .loading(building)
                    .disabled(building)
                    .on_click(cx.listener(|view, _, window, cx| view.trigger_rebuild(window, cx))),
            )
            .into_any_element()
    } else {
        Button::new("check-updates", "Check for updates")
            .variant(ButtonVariant::Outline)
            .loading(view.update_info.is_none())
            .on_click(cx.listener(|view, _, _, cx| view.check_updates(cx)))
            .into_any_element()
    };
    let mut rows = vec![
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_6()
            .gap_y_2()
            .px_4()
            .py_4()
            .child(
                div()
                    .flex_1()
                    .min_w(px(200.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().child(format!("Version {RUNNING_COMMIT}")))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("Built {BUILD_TIME}")),
                    )
                    .child(div().text_xs().text_color(rgb(repo_color)).child(repo)),
            )
            .child(div().flex_none().ml_auto().child(version_action))
            .into_any_element(),
        row(
            "Rebuild & install",
            Some("Runs apps/gpui/install, then restarts Sjel on the new build."),
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .id("copy-install-tip")
                        .tooltip(Tooltip::text("Copy the command"))
                        .child(
                            Button::new("copy-install", "Copy")
                                .variant(ButtonVariant::Ghost)
                                .size(ControlSize::Sm)
                                .disabled(rebuild_blocked || view.update_info.is_none())
                                .on_click(cx.listener(|view, _, _, cx| {
                                    let Some(info) = &view.update_info else {
                                        return;
                                    };
                                    let installer = info.repo_root.join("apps/gpui/install");
                                    let quoted = installer.to_string_lossy().replace('\'', "'\\''");
                                    cx.write_to_clipboard(ClipboardItem::new_string(format!(
                                        "'{quoted}'"
                                    )));
                                })),
                        ),
                )
                .child(
                    Button::new("rebuild", "Rebuild")
                        .variant(ButtonVariant::Primary)
                        .size(ControlSize::Sm)
                        .loading(building)
                        .disabled(building || rebuild_blocked)
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
                        Button::new(
                            "rebuild-log",
                            if view.show_rebuild_log {
                                "Hide full log"
                            } else {
                                "Full log"
                            },
                        )
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.show_rebuild_log = !view.show_rebuild_log;
                            cx.notify();
                        })),
                    )
                    .child(
                        div()
                            .id("rebuild-log-copy-tip")
                            .tooltip(Tooltip::text("Copy the log"))
                            .child(
                                Button::new("rebuild-log-copy", "Copy log")
                                    .variant(ButtonVariant::Ghost)
                                    .size(ControlSize::Sm)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        let RebuildState::Failed(failure) = &view.rebuild_state
                                        else {
                                            return;
                                        };
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            failure.log.clone(),
                                        ));
                                    })),
                            ),
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

#[cfg(test)]
mod tests {
    use gpui::{
        point, InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, TestAppContext,
    };

    use super::*;

    struct HelperDrag;

    impl Render for HelperDrag {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().child(helper_drag(
                PathBuf::from("/fixture/Sjel.app"),
                Palette::light(),
            ))
        }
    }

    #[gpui::test]
    fn helper_starts_drag_on_mouse_move(cx: &mut TestAppContext) {
        cx.update(sjel_gpui_ui::ely::init_for_tests);
        let window = cx.add_window(|_, _| HelperDrag);
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            window.dispatch_event(
                MouseDownEvent {
                    position: point(px(4.), px(8.)),
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            assert!(!cx.has_active_drag());
            window.dispatch_event(
                MouseMoveEvent {
                    position: point(px(20.), px(8.)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            assert!(cx.has_active_drag());
        })
        .unwrap();
    }
}
