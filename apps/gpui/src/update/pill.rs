use gpui_kit::{div, prelude::*, px, rgb, Entity, FontWeight, IntoElement};
use sjel_gpui_ui::ely::buttons::{Button, ButtonVariant};
use sjel_gpui_ui::ely::overlays::Popover;
use sjel_gpui_ui::ely::primitives::Tooltip;
use sjel_gpui_ui::ely::theme::ControlSize;

use crate::settings::Section;
use crate::workspace::Workspace;

use super::types::UpdateInfo;
use sjel_gpui_ui::theme::Palette;

/// The header's update control: a small pill that opens a popover with the newest commits,
/// Rebuild, and a link to Settings › General. The caller shows it only when commits are waiting.
pub fn header_pill(
    info: &UpdateInfo,
    building: bool,
    palette: Palette,
    workspace: Entity<Workspace>,
) -> impl IntoElement {
    let label = if building {
        "Building…".to_owned()
    } else {
        format!("↑ {} new", info.commits_behind)
    };
    let info = info.clone();
    Popover::with_opener(
        "update-popover",
        move |toggle| {
            div()
                .id("update-pill-tip")
                .tooltip(Tooltip::text("Sjel Desktop update"))
                .child(
                    Button::new("update-pill", label)
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .loading(building)
                        .on_click(move |_, window, cx| toggle(window, cx)),
                )
        },
        move |close, _, _cx| {
            let details = workspace.clone();
            let rebuild = workspace.clone();
            let details_close = close.clone();
            let rebuild_close = close;
            let behind = info.commits_behind;
            div()
                .w(px(320.))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Update available"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{behind} commit{} newer than this build ({})",
                                    if behind == 1 { "" } else { "s" },
                                    info.running_commit
                                )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(info.commits.iter().take(3).map(|commit| {
                            div()
                                .flex()
                                .gap_2()
                                .text_xs()
                                .child(
                                    div()
                                        .flex_none()
                                        .text_color(rgb(palette.accent))
                                        .child(commit.sha.clone()),
                                )
                                .child(div().min_w_0().truncate().child(commit.message.clone()))
                        })),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("update-details", "Details")
                                .variant(ButtonVariant::Ghost)
                                .size(ControlSize::Sm)
                                .on_click(move |_, window, cx| {
                                    details_close(window, cx);
                                    details.update(cx, |view, cx| {
                                        view.open_settings(Some(Section::General), cx)
                                    });
                                }),
                        )
                        .child(
                            div()
                                .id("update-rebuild-tip")
                                .tooltip(Tooltip::text(
                                    "Builds, installs, and restarts Sjel Desktop",
                                ))
                                .child(
                                    Button::new("update-rebuild", "Rebuild")
                                        .variant(ButtonVariant::Primary)
                                        .size(ControlSize::Sm)
                                        .disabled(building)
                                        .on_click(move |_, window, cx| {
                                            rebuild_close(window, cx);
                                            rebuild.update(cx, |view, cx| {
                                                view.trigger_rebuild(window, cx)
                                            });
                                        }),
                                ),
                        ),
                )
        },
    )
}
