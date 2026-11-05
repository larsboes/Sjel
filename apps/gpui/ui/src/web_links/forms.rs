//! Add destination form for WebLinks.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, IntoElement, Window};

use super::constants::ADD;
use super::LinksPage;
use crate::primitives::card;
use crate::theme::Palette;

pub(crate) fn render_add_form(
    page: &mut LinksPage,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<LinksPage>,
) -> Option<AnyElement> {
    if !page.show_add {
        return None;
    }
    page.ensure_add_inputs(window, cx);
    let url = page.url_input.as_ref().map(|(input, _)| input.clone());
    let title = page.title_input.as_ref().map(|(input, _)| input.clone());
    let group = page.group_input.as_ref().map(|(input, _)| input.clone());
    let form = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(div().text_sm().child("Add a destination"))
        .children(url.map(|input| Input::new(&input)))
        .children(title.map(|input| Input::new(&input)))
        .children(group.map(|input| Input::new(&input)))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    Button::new("web-links-add-save")
                        .label("Save link")
                        .loading(page.busy.contains(ADD))
                        .on_click(cx.listener(|view, _, window, cx| view.add(window, cx))),
                )
                .child(
                    Button::new("web-links-add-cancel")
                        .ghost()
                        .label("Cancel")
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.show_add = false;
                            cx.notify();
                        })),
                ),
        );
    Some(card(form.into_any_element(), palette))
}
