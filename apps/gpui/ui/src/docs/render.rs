//! Visual rendering of doc blocks, tables, inline highlights, and headings.

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, rgb, rgba, AnyElement, Context, FontStyle, FontWeight, HighlightStyle, IntoElement,
    MouseButton, MouseDownEvent, SharedString, StrikethroughStyle, StyledText, UnderlineStyle,
};
use sjel_gpui_core::blocks::Kind;
use sjel_gpui_core::inline::{Inline, Mark};

use super::diagrams::{mermaid_source, render_mermaid};
use super::types::{explicit_anchor, MarkdownBlock};
use super::DocsPage;
use crate::theme::{tint, Palette};

pub(crate) fn inline_highlights(
    marks: &[(std::ops::Range<usize>, Mark)],
    palette: Palette,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let style = |mark| match mark {
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
    let mut cuts: Vec<usize> = marks
        .iter()
        .flat_map(|(range, _)| [range.start, range.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|pair| {
            let range = pair[0]..pair[1];
            marks
                .iter()
                .filter(|(mark, _)| mark.start <= range.start && range.end <= mark.end)
                .map(|(_, mark)| style(*mark))
                .reduce(HighlightStyle::highlight)
                .map(|style| (range, style))
        })
        .collect()
}

pub(crate) fn render_inline_text(
    text: &str,
    file: String,
    palette: Palette,
    cx: &mut Context<DocsPage>,
) -> AnyElement {
    let inline = Inline::parse(text);
    let links = inline.links.clone();
    let shown: SharedString = if inline.text.is_empty() {
        " ".into()
    } else {
        inline.text.clone().into()
    };
    let styled = StyledText::new(shown).with_highlights(inline_highlights(&inline.marks, palette));
    let layout = styled.layout().clone();
    let mut element = div().child(styled);
    if !links.is_empty() {
        element = element.cursor_pointer().on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                let shown = layout
                    .index_for_position(event.position)
                    .unwrap_or_else(|nearest| nearest);
                if let Some((_, href)) = links.iter().find(|(range, _)| range.contains(&shown)) {
                    this.follow_link(href, &file, cx);
                    cx.stop_propagation();
                }
            }),
        );
    }
    element.into_any_element()
}

pub(crate) fn render_doc_block(
    index: usize,
    block: &MarkdownBlock,
    file: String,
    palette: Palette,
    cx: &mut Context<DocsPage>,
) -> AnyElement {
    let MarkdownBlock::Content(block) = block else {
        let MarkdownBlock::Table { headers, rows } = block else {
            unreachable!()
        };
        let columns = headers.len().max(1);
        let cell = |value: &str, heading: bool, cx: &mut Context<DocsPage>| {
            let mut item = div()
                .w(px(180.))
                .flex_none()
                .px_3()
                .py_2()
                .border_r_1()
                .border_color(rgba(palette.border))
                .child(render_inline_text(value, file.clone(), palette, cx));
            if heading {
                item = item
                    .bg(rgba(palette.raised))
                    .font_weight(FontWeight::SEMIBOLD);
            }
            item.into_any_element()
        };
        let header = div()
            .flex()
            .children(headers.iter().map(|value| cell(value, true, cx)));
        let mut table = div()
            .w(px(columns as f32 * 180.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .overflow_hidden()
            .child(header);
        for (row_index, row) in rows.iter().enumerate() {
            let mut rendered = div()
                .flex()
                .when(row_index % 2 == 1, |row| row.bg(rgba(palette.surface)));
            for column in 0..columns {
                rendered =
                    rendered.child(cell(row.get(column).map_or("", String::as_str), false, cx));
            }
            table = table.child(rendered);
        }
        return div().overflow_x_scrollbar().child(table).into_any_element();
    };
    if explicit_anchor(&block.text).is_some() {
        return div().h(px(1.)).into_any_element();
    }
    let content_text = render_inline_text(&block.text, file, palette, cx);
    let content = match block.kind {
        Kind::Heading(level) => {
            let size = match level {
                1 => px(23.),
                2 => px(20.),
                3 => px(17.),
                _ => px(15.),
            };
            div()
                .pt_2()
                .text_size(size)
                .font_weight(if level <= 2 {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(rgb(palette.text))
                .child(content_text)
        }
        Kind::Code => {
            if let Some(source) = mermaid_source(&block.text) {
                return match render_mermaid(&source, palette) {
                    Ok((svg, width, height)) => div()
                        .my_2()
                        .overflow_x_scrollbar()
                        .child(
                            gpui_kit::svg()
                                .data(svg.as_bytes())
                                .w(px(width))
                                .h(px(height)),
                        )
                        .into_any_element(),
                    Err(error) => div()
                        .p_3()
                        .rounded(px(8.))
                        .bg(rgb(0x1e1e1e))
                        .text_xs()
                        .text_color(rgb(palette.amber))
                        .child(format!("Could not render Mermaid diagram: {error}"))
                        .child(div().pt_2().text_color(rgb(0xd4d4d4)).child(source))
                        .into_any_element(),
                };
            }
            return div()
                .p_3()
                .rounded(px(8.))
                .bg(rgb(0x1e1e1e))
                .text_xs()
                .text_color(rgb(0xd4d4d4))
                .child(block.text.clone())
                .into_any_element();
        }
        Kind::Quote => div()
            .px_3()
            .border_l_2()
            .border_color(rgb(palette.accent))
            .text_sm()
            .text_color(rgb(palette.muted))
            .child(content_text),
        Kind::Bullet => div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(div().text_color(rgb(palette.muted)).child("•"))
            .child(div().flex_1().child(content_text)),
        Kind::Numbered => div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .child(block.prefix.trim().to_owned()),
            )
            .child(div().flex_1().child(content_text)),
        Kind::Todo(done) => div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .child(if done { "☑" } else { "☐" }),
            )
            .child(div().flex_1().child(content_text)),
        _ => div()
            .text_sm()
            .text_color(rgb(palette.text))
            .child(content_text),
    };
    if matches!(block.kind, Kind::Heading(_)) {
        content
            .id(SharedString::from(format!("doc-heading-{index}")))
            .into_any_element()
    } else {
        content.into_any_element()
    }
}
