//! Sjel block choices presented at the current caret without moving editor focus.

use gpui_kit::{component::input::TextareaState, AnyElement, Context, IntoElement, WeakEntity};
use sjel_gpui_core::blocks::Kind;

use super::{types::Menu, NoteEditor};
use crate::ely::{forms::Choice, menus::CaretMenu, primitives::IconName};

pub(crate) fn render_menu(
    menu: &Menu,
    input: WeakEntity<TextareaState>,
    cx: &mut Context<NoteEditor>,
) -> AnyElement {
    let index = menu.block;
    let choices = menu.choices();
    let kinds: Vec<_> = choices.iter().map(|choice| choice.kind).collect();
    let rows = choices.iter().map(|choice| {
        let icon = match choice.kind {
            Kind::Heading(1) => IconName::Heading1,
            Kind::Heading(2) => IconName::Heading2,
            Kind::Heading(3) => IconName::Heading3,
            Kind::Heading(_) => IconName::Heading,
            Kind::Bullet => IconName::List,
            Kind::Numbered => IconName::ListOrdered,
            Kind::Todo(_) => IconName::ListTodo,
            Kind::Quote => IconName::Quote,
            Kind::Code => IconName::Code,
            _ => IconName::FileText,
        };
        Choice::new(choice.label, choice.label)
            .icon(icon)
            .note(choice.shortcut)
    });
    let owner = cx.entity().downgrade();
    let dismiss = owner.clone();
    CaretMenu::new(
        ("editor-caret-menu", index),
        rows,
        move |_, cx| {
            let input = input.upgrade()?;
            let geometry = input.read(cx).prepaint_caret_geometry()?;
            let mut bounds = geometry.cursor_bounds();
            bounds.origin.y += geometry.scroll_offset().y;
            bounds.size.height = geometry.line_height();
            Some(bounds)
        },
        move |at, window, cx| {
            if let (Some(owner), Some(&kind)) = (owner.upgrade(), kinds.get(at)) {
                owner.update(cx, |editor, cx| editor.apply(index, kind, window, cx));
            }
        },
        move |_, cx| {
            if let Some(owner) = dismiss.upgrade() {
                owner.update(cx, |editor, cx| {
                    if editor.menu.as_ref().is_some_and(|menu| menu.block == index) {
                        editor.menu = None;
                        cx.notify();
                    }
                });
            }
        },
    )
    .selected(Some(menu.selected))
    .into_any_element()
}
