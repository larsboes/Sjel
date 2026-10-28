//! Shared Block Editor / Document Viewer for desktop and mobile GPUI surfaces.
//!
//! Renders a parsed `Doc` (`sjel_gpui_core::blocks::Doc`) with uniform typography,
//! semantic palette colors, and density-aware touch targets for checklists.

use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, ClickEvent, FontStyle, FontWeight, HighlightStyle,
    SharedString, StrikethroughStyle, StyledText, UnderlineStyle, Window,
};
use sjel_gpui_core::blocks::{Block, Doc, Kind};
use sjel_gpui_core::inline::{Inline, Mark};

use crate::theme::{tint, Density, Palette};

/// Renders a raw markdown string into a styled block layout with inline markups.
pub fn render_markdown(markdown: &str, palette: Palette, density: Density) -> AnyElement {
    let doc = Doc::parse(markdown);
    render_doc(
        &doc,
        palette,
        density,
        None::<fn(usize, &ClickEvent, &mut Window, &mut gpui_kit::App)>,
    )
}

/// Renders a whole document's blocks into a vertical layout container.
pub fn render_doc<F>(
    doc: &Doc,
    palette: Palette,
    density: Density,
    on_toggle_todo: Option<F>,
) -> AnyElement
where
    F: Fn(usize, &ClickEvent, &mut Window, &mut gpui_kit::App) + 'static + Clone,
{
    let mut container = div().flex().flex_col().gap_2();

    for (idx, block) in doc.blocks.iter().enumerate() {
        let toggle = on_toggle_todo.clone();
        container = container.child(render_block(
            idx,
            block,
            palette,
            density,
            toggle.map(|cb| {
                move |e: &ClickEvent, w: &mut Window, a: &mut gpui_kit::App| cb(idx, e, w, a)
            }),
        ));
    }

    container.into_any_element()
}

fn inline_highlights(
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
    marks
        .iter()
        .map(|(range, mark)| (range.clone(), style(*mark)))
        .collect()
}

fn styled_inline(text: &str, palette: Palette) -> AnyElement {
    let inline = Inline::parse(text);
    if inline.text.is_empty() {
        return div().into_any_element();
    }
    let shown: SharedString = inline.text.into();
    let highlights = inline_highlights(&inline.marks, palette);
    if highlights.is_empty() {
        div().child(shown).into_any_element()
    } else {
        StyledText::new(shown)
            .with_highlights(highlights)
            .into_any_element()
    }
}

/// Renders one individual block with semantic styling and inline formatting.
pub fn render_block<F>(
    idx: usize,
    block: &Block,
    palette: Palette,
    density: Density,
    on_toggle: Option<F>,
) -> AnyElement
where
    F: Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
{
    match block.kind {
        Kind::Heading(1) => div()
            .pt_2()
            .text_xl()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(palette.text))
            .child(styled_inline(&block.text, palette))
            .into_any_element(),
        Kind::Heading(2) => div()
            .pt_2()
            .text_lg()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.text))
            .child(styled_inline(&block.text, palette))
            .into_any_element(),
        Kind::Heading(_) => div()
            .pt_1()
            .text_base()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.text))
            .child(styled_inline(&block.text, palette))
            .into_any_element(),
        Kind::Todo(done) => {
            let checkbox_size = if density.is_mobile() {
                px(22.)
            } else {
                px(16.)
            };
            let row_h = density.row_height();

            let mut row = div()
                .id(SharedString::from(format!("block-todo-{idx}")))
                .flex()
                .items_center()
                .gap_3()
                .px(density.pad_x())
                .py(density.pad_y())
                .min_h(row_h)
                .rounded(px(6.))
                .bg(if done {
                    rgba(palette.raised)
                } else {
                    rgba(palette.surface)
                });

            // Checkbox box
            let box_el = div()
                .size(checkbox_size)
                .rounded(px(4.))
                .border_1()
                .border_color(if done {
                    rgb(palette.green)
                } else {
                    rgb(palette.muted)
                })
                .bg(if done {
                    rgb(palette.green)
                } else {
                    rgba(palette.surface)
                })
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(palette.surface))
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .child(if done { "✓" } else { "" });

            // Text
            let text_el = div()
                .flex_1()
                .text_sm()
                .when(done, |el| el.line_through())
                .text_color(if done {
                    rgb(palette.muted)
                } else {
                    rgb(palette.text)
                })
                .child(styled_inline(&block.text, palette));

            row = row.child(box_el).child(text_el);

            if let Some(on_toggle) = on_toggle {
                row = row.cursor_pointer().on_click(on_toggle);
            }

            row.into_any_element()
        }
        Kind::Bullet => div()
            .px(density.pad_x())
            .py(px(2.))
            .flex()
            .items_start()
            .gap_2()
            .child(div().text_sm().text_color(rgb(palette.muted)).child("•"))
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(palette.text))
                    .child(styled_inline(&block.text, palette)),
            )
            .into_any_element(),
        Kind::Numbered => div()
            .px(density.pad_x())
            .py(px(2.))
            .flex()
            .items_start()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(block.prefix.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(palette.text))
                    .child(styled_inline(&block.text, palette)),
            )
            .into_any_element(),
        Kind::Quote => div()
            .px_3()
            .py(px(4.))
            .border_l_2()
            .border_color(rgb(palette.accent))
            .bg(rgba(palette.raised))
            .rounded_r(px(6.))
            .text_sm()
            .text_color(rgb(palette.muted))
            .child(styled_inline(&block.text, palette))
            .into_any_element(),
        Kind::Code => div()
            .p_3()
            .rounded(px(8.))
            .bg(rgb(0x1e1e1e))
            .border_1()
            .border_color(rgba(palette.border))
            .text_xs()
            .text_color(rgb(0xd4d4d4))
            .child(block.text.clone())
            .into_any_element(),
        Kind::Paragraph => div()
            .px(density.pad_x())
            .py(px(2.))
            .text_sm()
            .text_color(rgb(palette.text))
            .child(styled_inline(&block.text, palette))
            .into_any_element(),
    }
}
