//! Popover menu for slash commands and block-type switching.

use gpui_kit::prelude::*;
use gpui_kit::{
    anchored, deferred, div, px, rgb, rgba, AnyElement, Context, FontWeight, MouseButton,
};

use super::types::Menu;
use super::NoteEditor;
use crate::theme::Palette;

pub(crate) fn render_menu(
    menu: &Menu,
    palette: Palette,
    cx: &mut Context<NoteEditor>,
) -> AnyElement {
    let index = menu.block;
    let items = menu.choices().into_iter().enumerate().map(|(at, choice)| {
        let kind = choice.kind;
        div()
            .id(("choice", at))
            .px_2()
            .py(px(5.))
            .rounded(px(6.))
            .text_sm()
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .when(at == menu.selected, |item| item.bg(rgba(palette.raised)))
            .hover(move |style| style.bg(rgba(palette.raised)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |editor, _, window, cx| {
                    editor.apply(index, kind, window, cx);
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(24.))
                            .h(px(20.))
                            .rounded(px(4.))
                            .bg(rgba(palette.surface))
                            .border_1()
                            .border_color(rgba(palette.border))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child(choice.icon),
                    )
                    .child(choice.label),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .opacity(0.7)
                    .child(choice.shortcut),
            )
    });
    let title = if menu.query.is_some() {
        "Insert Block"
    } else {
        "Turn into"
    };
    deferred(
        anchored().snap_to_window_with_margin(px(8.)).child(
            div()
                .mt_1()
                .w(px(250.))
                .p_1()
                .flex()
                .flex_col()
                .rounded(px(10.))
                .bg(rgba(palette.surface | 0xff))
                .border_1()
                .border_color(rgba(palette.border))
                .shadow_lg()
                .child(
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.muted))
                        .child(title),
                )
                .children(items),
        ),
    )
    .with_priority(1)
    .into_any_element()
}
