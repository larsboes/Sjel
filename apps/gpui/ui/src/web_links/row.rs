//! Rendering saved destination link rows with inline editing and remove verification.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::Sizable;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, rgb, AnyElement, Context, IntoElement, SharedString, Window};
use sjel_gpui_core::web_links::WebLink;

use super::LinksPage;
use crate::theme::Palette;

pub(crate) fn render_link_row(
    page: &LinksPage,
    link: &WebLink,
    palette: Palette,
    _window: &mut Window,
    cx: &mut Context<LinksPage>,
) -> AnyElement {
    let saved = link.clone();
    let saved_for_open = saved.clone();
    let saved_for_edit = saved.clone();
    let heading = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().truncate().text_sm().child(if link.pinned {
                    format!("★  {}", link.title)
                } else {
                    link.title.clone()
                }))
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "{}{} · {}",
                            link.host,
                            link.tenant
                                .as_ref()
                                .map(|tenant| format!(" · {tenant}"))
                                .unwrap_or_default(),
                            link.url
                        )),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("open-{}", link.id)))
                .small()
                .label("Open")
                .on_click(
                    cx.listener(move |view, _, window, cx| view.open(&saved_for_open, window, cx)),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("pin-{}", link.id)))
                .small()
                .ghost()
                .label(if link.pinned { "Unpin" } else { "Pin" })
                .on_click(
                    cx.listener(move |view, _, window, cx| view.toggle_pin(&saved, window, cx)),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("edit-{}", link.id)))
                .small()
                .ghost()
                .label("Edit")
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.begin_edit(&saved_for_edit, window, cx)
                })),
        );
    if page.edit_id.as_deref() != Some(link.id.as_str()) {
        return heading.into_any_element();
    }
    let title = page.edit_title.clone();
    let group = page.edit_group.clone();
    let id = link.id.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(heading)
        .child(
            div()
                .flex()
                .gap(px(8.))
                .children(title.map(|input| Input::new(&input)))
                .children(group.map(|input| Input::new(&input)))
                .child(
                    Button::new("web-link-save-edit")
                        .small()
                        .label("Save")
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.save_edit(id.clone(), window, cx)
                        })),
                )
                .child(
                    Button::new("web-link-cancel-edit")
                        .small()
                        .ghost()
                        .label("Cancel")
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.edit_id = None;
                            view.confirm_remove = None;
                            cx.notify();
                        })),
                )
                .child(
                    if page.confirm_remove.as_deref() == Some(link.id.as_str()) {
                        let remove_id = link.id.clone();
                        Button::new("web-link-remove-confirm")
                            .small()
                            .danger()
                            .label("Confirm remove")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.remove(remove_id.clone(), window, cx)
                            }))
                    } else {
                        let remove_id = link.id.clone();
                        Button::new("web-link-remove")
                            .small()
                            .ghost()
                            .label("Remove…")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.request_remove(remove_id.clone(), cx)
                            }))
                    },
                ),
        )
        .into_any_element()
}
