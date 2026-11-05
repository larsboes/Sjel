//! System and connectivity settings: Shell connection, macOS permissions, and app updates/rebuilding.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, IconName, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, ClipboardItem, Context, IntoElement};
use sjel_gpui_ui::theme::{tint, Palette};

use super::{dot_value, group, row, value, BUILD_TIME, RUNNING_COMMIT};
use crate::route::Page;
use crate::update::RebuildState;
use crate::workspace::Workspace;

pub(super) fn connection(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Vec<AnyElement> {
    let (status_color, status) = crate::sidebar::status(&view.health, palette);
    let (read_path, read_color) = crate::sidebar::storage(view.has_direct_db(), palette);
    let read_detail = if view.has_direct_db() {
        "Reads the local database directly. Writes go through the shell."
    } else {
        "Reads and writes through the shell's HTTP API."
    };
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
                    Button::new("switch-node")
                        .small()
                        .label(if dev {
                            "Back to live"
                        } else {
                            "Open on dev node"
                        })
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
                Button::new("open-setup")
                    .outline()
                    .small()
                    .label("Pairing")
                    .on_click(cx.listener(|view, _, _, cx| view.go(Page::Setup, cx))),
                palette,
            )],
            palette,
        ),
    ]
}

pub(super) fn permissions(palette: Palette) -> Vec<AnyElement> {
    use crate::permissions::{installed, open_pane, reveal, GRANTS};
    GRANTS
        .iter()
        .enumerate()
        .map(|(index, grant)| {
            let binary = installed(grant);
            let state = match &binary {
                Some(_) => value("Installed".into(), palette.muted),
                None => value(
                    "Not installed. Run tools/fda-launcher/install".into(),
                    palette.amber,
                ),
            };
            let mut control = div().flex().items_center().gap_2().child(state);
            if let Some(path) = binary {
                control = control.child(
                    Button::new(("reveal", index))
                        .ghost()
                        .small()
                        .label("Show in Finder")
                        .on_click(move |_, _, _| reveal(&path)),
                );
            }
            control = control.child(
                Button::new(("open-pane", index))
                    .primary()
                    .small()
                    .label("Open System Settings")
                    .on_click(move |_, _, _| open_pane(&GRANTS[index])),
            );
            row(grant.title, Some(grant.why), control, palette)
        })
        .collect()
}

pub(super) fn updates(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Vec<AnyElement> {
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

    let behind = view
        .update_info
        .as_ref()
        .is_some_and(|info| info.commits_behind > 0);
    let version_action = if behind {
        Button::new("update-now")
            .primary()
            .label("Update")
            .loading(building)
            .disabled(building)
            .tooltip("Builds, installs, and restarts Sjel Desktop")
            .on_click(cx.listener(|view, _, window, cx| view.trigger_rebuild(window, cx)))
    } else {
        Button::new("check-updates")
            .outline()
            .label("Check for updates")
            .loading(view.update_info.is_none())
            .on_click(cx.listener(|view, _, _, cx| view.check_updates(cx)))
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
