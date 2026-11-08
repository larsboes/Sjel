//! Block rendering, formatting runs, gutter markers, and drag handles.

use std::sync::Arc;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Backspace, Escape, MoveDown, MoveUp, Textarea};
use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, relative, rgb, rgba, AnyElement, Context, FontStyle, FontWeight, HighlightStyle,
    MouseButton, MouseDownEvent, SharedString, StrikethroughStyle, StyledText, UnderlineStyle,
    Window,
};
use sjel_gpui_core::blocks::Kind;
use sjel_gpui_core::inline::{Inline, Mark};

use super::slash_menu::render_menu;
use super::types::{DraggedBlock, Shown, TEXTAREA};
use super::NoteEditor;
use crate::theme::{tint, Palette};

/// Marks as non-overlapping highlight runs; where marks nest, their styles combine.
pub(crate) fn runs(
    marks: &[(std::ops::Range<usize>, Mark)],
    palette: Palette,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let style = |mark: Mark| match mark {
        Mark::Bold => HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        },
        Mark::Italic => HighlightStyle {
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        },
        Mark::Code => HighlightStyle {
            background_color: Some(rgba(palette.raised).into()),
            ..Default::default()
        },
        Mark::Strike => HighlightStyle {
            strikethrough: Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            }),
            ..Default::default()
        },
        Mark::Link => HighlightStyle {
            color: Some(rgb(palette.accent).into()),
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            }),
            ..Default::default()
        },
        Mark::Highlight => HighlightStyle {
            background_color: Some(rgba(tint(palette.amber, 0x30)).into()),
            ..Default::default()
        },
    };
    let mut cuts: Vec<usize> = marks.iter().flat_map(|(r, _)| [r.start, r.end]).collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|pair| {
            let range = pair[0]..pair[1];
            marks
                .iter()
                .filter(|(r, _)| r.start <= range.start && range.end <= r.end)
                .map(|(_, mark)| style(*mark))
                .reduce(HighlightStyle::highlight)
                .map(|style| (range, style))
        })
        .collect()
}

pub(crate) fn render_block(
    editor: &mut NoteEditor,
    index: usize,
    window: &mut Window,
    cx: &mut Context<NoteEditor>,
) -> AnyElement {
    let Some(block) = editor.doc.blocks.get(index) else {
        return div().into_any_element();
    };
    // Read once, so the cache below may take `&mut self` without holding the block.
    let (kind, text, prefix, gap) = (
        block.kind,
        block.text.clone(),
        block.prefix.clone(),
        block.gap,
    );
    let palette = editor.palette(window);
    let mono = cx.theme().mono_font_family.clone();
    let indent = prefix.len() - prefix.trim_start().len();
    settle_shown(editor, index, &text, palette);
    let Some(row) = editor.rows.get(index) else {
        return div().into_any_element();
    };
    let gutter: AnyElement = match kind {
        Kind::Bullet => div()
            .text_sm()
            .text_color(rgb(palette.accent))
            .child("•")
            .into_any_element(),
        Kind::Numbered => div()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.muted))
            .child(prefix.trim().to_owned())
            .into_any_element(),
        Kind::Todo(done) => {
            let editor_entity = cx.entity().downgrade();
            Checkbox::new(("todo", index))
                .checked(done)
                .on_click(move |_, _, cx| {
                    let _ = editor_entity.update(cx, |editor, cx| editor.toggle(index, cx));
                })
                .into_any_element()
        }
        _ => div().into_any_element(),
    };
    // Code always shows its source; prose shows markdown only while the caret is in it.
    let live = editor
        .preview
        .as_ref()
        .filter(|preview| preview.read(cx).input == row.input && kind == Kind::Paragraph);
    let field = if let Some(live) = live.filter(|_| editor.active == Some(index)) {
        live.update(cx, |live, _| {
            live.palette = palette;
            live.readonly = !editor.editable;
        });
        live.clone().into_any_element()
    } else if editor.active == Some(index) || kind == Kind::Code {
        let field = Textarea::new(&row.input)
            .appearance(false)
            // Cancels the padding its editor gets from its size (`gpui-component`
            // input.rs `set_editor_paddings`), so a block keeps its box when it gains the
            // caret.
            .mx(-TEXTAREA.input_px())
            .my(-TEXTAREA.input_py())
            .line_height(relative(1.3))
            .readonly(!editor.editable);
        // The textarea sets its own text size, so a heading styles the textarea itself.
        match kind {
            Kind::Heading(1) => field.text_2xl().font_weight(FontWeight::BOLD),
            Kind::Heading(2) => field.text_xl().font_weight(FontWeight::SEMIBOLD),
            Kind::Heading(_) => field.text_lg().font_weight(FontWeight::SEMIBOLD),
            _ => field,
        }
        .into_any_element()
    } else {
        formatted(editor, index, cx)
    };
    let field = match kind {
        Kind::Heading(1) => div()
            .text_2xl()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(palette.text))
            .child(field),
        Kind::Heading(2) => div()
            .text_xl()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.text))
            .child(field),
        Kind::Heading(_) => div()
            .text_lg()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.text))
            .child(field),
        Kind::Quote => div()
            .border_l_3()
            .border_color(rgb(palette.accent))
            .rounded_r(px(8.))
            .bg(rgba(tint(palette.accent, 0x0c)))
            .px_3()
            .py(px(4.))
            .italic()
            .text_color(rgb(palette.text))
            .child(field),
        Kind::Todo(true) => div()
            .text_color(rgb(palette.muted))
            .line_through()
            .child(field),
        Kind::Code => div()
            .font_family(mono.clone())
            .text_sm()
            .rounded(px(8.))
            .bg(rgba(palette.raised))
            .border_1()
            .border_color(rgba(palette.border))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py(px(3.))
                    .bg(rgba(palette.surface | 0x88))
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("code")
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .opacity(0.6)
                            .child("snippet"),
                    ),
            )
            .child(div().p_3().child(field)),
        _ => div().child(field),
    };
    let label = editor
        .shown
        .get(index)
        .and_then(Option::as_ref)
        .map_or_else(SharedString::default, |shown| shown.label.clone());
    let handle = div()
        .id(("handle", index))
        .absolute()
        .left(px(-24.))
        .top(px(if gap > 0 { 8. } else { 0. }))
        .w(px(20.))
        .h(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_xs()
        .text_color(rgb(palette.muted))
        .cursor_grab()
        .opacity(0.)
        .group_hover("block", |style| style.opacity(1.))
        .hover(move |style| style.bg(rgba(palette.raised)).text_color(rgb(palette.text)))
        .child("⋮⋮")
        .on_click(cx.listener(move |editor, _, window, cx| editor.open_menu(index, window, cx)))
        .on_drag(
            DraggedBlock {
                index,
                label,
                palette,
            },
            |dragged, _, _, cx| cx.new(|_| dragged.clone()),
        );
    let menu = editor
        .menu
        .as_ref()
        .filter(|menu| menu.block == index)
        .map(|menu| render_menu(menu, palette, cx));
    div()
        .id(("block", index))
        .group("block")
        .relative()
        .w_full()
        .flex()
        .items_start()
        .gap_2()
        .pl(px(indent as f32 * 8.))
        .drag_over::<DraggedBlock>(move |style, _, _, _| style.bg(rgba(palette.raised)))
        .on_drop(
            cx.listener(move |editor, dragged: &DraggedBlock, window, cx| {
                editor.move_block(dragged.index, index, window, cx)
            }),
        )
        .child(handle)
        .pt(px(if gap > 0 { 10. } else { 2. }))
        .capture_action(cx.listener(move |editor, _: &Backspace, window, cx| {
            if editor.on_backspace(index, window, cx) {
                cx.stop_propagation();
            }
        }))
        .capture_action(cx.listener(move |editor, _: &MoveUp, window, cx| {
            if editor.step_menu(index, -1, cx) || editor.on_cross(index, true, window, cx) {
                cx.stop_propagation();
            }
        }))
        .capture_action(cx.listener(move |editor, _: &MoveDown, window, cx| {
            if editor.step_menu(index, 1, cx) || editor.on_cross(index, false, window, cx) {
                cx.stop_propagation();
            }
        }))
        .capture_action(cx.listener(move |editor, _: &Escape, _, cx| {
            if editor.menu.as_ref().is_some_and(|menu| menu.block == index) {
                editor.menu = None;
                cx.notify();
                cx.stop_propagation();
            }
        }))
        .child(
            div()
                .w(px(20.))
                .flex_none()
                .flex()
                .justify_end()
                // The text's line height, so the marker sits on the first line.
                .line_height(relative(1.3))
                .text_color(rgb(palette.muted))
                .child(gutter),
        )
        .child(div().flex_1().min_w_0().child(field).children(menu))
        .into_any_element()
}

pub(crate) fn settle_shown(editor: &mut NoteEditor, index: usize, text: &str, palette: Palette) {
    let current = editor.shown.get(index).and_then(Option::as_ref);
    if current.is_some_and(|shown| shown.text == text && shown.palette == palette) {
        return;
    }
    let inline = Arc::new(Inline::parse(text));
    // An empty block keeps one line's height.
    let label: SharedString = if inline.text.is_empty() {
        " ".into()
    } else {
        inline.text.clone().into()
    };
    let highlights = runs(&inline.marks, palette);
    if editor.shown.len() <= index {
        editor.shown.resize_with(index + 1, || None);
    }
    editor.shown[index] = Some(Shown {
        text: text.to_owned(),
        palette,
        label,
        inline,
        highlights,
    });
}

pub(crate) fn formatted(
    editor: &NoteEditor,
    index: usize,
    cx: &mut Context<NoteEditor>,
) -> AnyElement {
    let Some(shown) = editor.shown.get(index).and_then(Option::as_ref) else {
        return div().into_any_element();
    };
    if let Some(table) = super::table::parse_markdown_table(&shown.text) {
        return super::table::render_editor_table(&table, shown.palette, index, cx);
    }
    let styled = StyledText::new(shown.label.clone()).with_highlights(shown.highlights.clone());
    // An `Rc` handle: the layout fills in when the element is laid out, which this closure
    // reads after that. Kept per frame, but the work behind it is not.
    let layout = styled.layout().clone();
    let inline = shown.inline.clone();
    div()
        .id(("formatted", index))
        .debug_selector(move || format!("formatted-{index}"))
        .w_full()
        // The textarea's line height, so a block keeps its height when it gains the caret.
        .line_height(relative(1.3))
        .cursor_text()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |editor, event: &MouseDownEvent, window, cx| {
                let shown = layout
                    .index_for_position(event.position)
                    .unwrap_or_else(|nearest| nearest);
                editor.focus(index, inline.source_offset(shown), window, cx);
                cx.stop_propagation();
            }),
        )
        .child(styled)
        .into_any_element()
}
