use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Disableable, IconName, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, Anchor, Entity, FontWeight, IntoElement};

use crate::settings::Section;
use crate::workspace::Workspace;

use sjel_gpui_ui::theme::Palette;
use super::types::UpdateInfo;

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
        format!("{} new", info.commits_behind)
    };
    let info = info.clone();
    Popover::new("update-popover")
        .anchor(Anchor::TopRight)
        .trigger(
            Button::new("update-pill")
                .small()
                .outline()
                .icon(IconName::ArrowUp)
                .loading(building)
                .label(label)
                .tooltip("Sjel Desktop update"),
        )
        .content(move |_, _, cx| {
            let details = workspace.clone();
            let rebuild = workspace.clone();
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
                            Button::new("update-details")
                                .ghost()
                                .small()
                                .label("Details")
                                .on_click(cx.listener(move |state, _, window, cx| {
                                    state.dismiss(window, cx);
                                    details.update(cx, |view, cx| {
                                        view.open_settings(Some(Section::General), cx)
                                    });
                                })),
                        )
                        .child(
                            Button::new("update-rebuild")
                                .primary()
                                .small()
                                .label("Rebuild")
                                .disabled(building)
                                .tooltip("Builds, installs, and restarts Sjel Desktop")
                                .on_click(cx.listener(move |state, _, window, cx| {
                                    state.dismiss(window, cx);
                                    rebuild.update(cx, |view, cx| view.trigger_rebuild(window, cx));
                                })),
                        ),
                )
        })
}
