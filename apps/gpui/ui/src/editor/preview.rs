use std::ops::Range;

use gpui_kit::component::input::Textarea;
use gpui_kit::{
    canvas, fill, relative, size, Bounds, EntityInputHandler, MouseButton, MouseDownEvent,
    MouseMoveEvent, StyledText, Subscription, TextLayout, UnderlineStyle,
};

use super::*;

pub(crate) struct LiveBlock {
    pub input: Entity<TextareaState>,
    pub palette: Palette,
    pub readonly: bool,
    anchor: Option<usize>,
    pub(crate) layout: TextLayout,
    _observe: Subscription,
}

impl LiveBlock {
    pub fn new(input: Entity<TextareaState>, palette: Palette, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&input, |_, _, cx| cx.notify());
        Self {
            input,
            palette,
            readonly: false,
            anchor: None,
            layout: TextLayout::default(),
            _observe: observe,
        }
    }
}

impl Render for LiveBlock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.input.read(cx);
        let source = state.value().to_string();
        let cursor = state.cursor();
        let selected = state.selected_range();
        let marked = self
            .input
            .update(cx, |input, cx| input.marked_text_range(window, cx));
        let inline = Arc::new(Inline::preview(&source, cursor));
        let mut highlights = runs(&inline.marks, self.palette);
        let projected =
            |range: Range<usize>| inline.shown_offset(range.start)..inline.shown_offset(range.end);
        if !selected.is_empty() {
            highlights.push((
                projected(selected),
                HighlightStyle {
                    background_color: Some(rgba(tint(self.palette.accent, 0x50)).into()),
                    ..Default::default()
                },
            ));
        }
        if let Some(marked) = marked {
            highlights.push((
                projected(utf8_offset(&source, marked.start)..utf8_offset(&source, marked.end)),
                HighlightStyle {
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: false,
                    }),
                    ..Default::default()
                },
            ));
        }
        let label: SharedString = if inline.text.is_empty() {
            " ".into()
        } else {
            inline.text.clone().into()
        };
        let styled = StyledText::new(label).with_highlights(merge_highlights(highlights));
        let layout = styled.layout().clone();
        self.layout = layout.clone();
        let caret_layout = layout.clone();
        let drag_layout = layout.clone();
        let drag_inline = inline.clone();
        let caret = inline.shown_offset(cursor);
        let color = self.palette.text;
        let readonly = self.readonly;
        div()
            .id("live-block")
            .debug_selector(|| "live-block".into())
            .relative()
            .w_full()
            .line_height(relative(1.3))
            .cursor_text()
            .overflow_hidden()
            .child(
                div().absolute().inset_0().child(
                    Textarea::new(&self.input)
                        .appearance(false)
                        .mx(-TEXTAREA.input_px())
                        .my(-TEXTAREA.input_py())
                        .line_height(relative(1.3))
                        .readonly(readonly)
                        .opacity(0.),
                ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                            let shown = layout
                                .index_for_position(event.position)
                                .unwrap_or_else(|at| at);
                            let offset = inline.source_offset(shown);
                            let anchor = if event.modifiers.shift {
                                let input = view.input.read(cx);
                                let selected = input.selected_range();
                                if input.cursor() == selected.start {
                                    selected.end
                                } else {
                                    selected.start
                                }
                            } else {
                                offset
                            };
                            view.anchor = Some(anchor);
                            view.input.update(cx, |input, cx| {
                                input
                                    .set_selected_range(anchor.min(offset)..anchor.max(offset), cx);
                                input.focus(window, cx);
                            });
                            cx.stop_propagation();
                        }),
                    )
                    .on_mouse_move(cx.listener(move |view, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button != Some(MouseButton::Left) {
                            view.anchor = None;
                            return;
                        }
                        if let Some(anchor) = view.anchor {
                            let shown = drag_layout
                                .index_for_position(event.position)
                                .unwrap_or_else(|at| at);
                            let offset = drag_inline.source_offset(shown);
                            view.input.update(cx, |input, cx| {
                                input
                                    .set_selected_range(anchor.min(offset)..anchor.max(offset), cx);
                            });
                        }
                    }))
                    .child(styled)
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |_, _, window, _| {
                                if !readonly {
                                    if let Some(origin) = caret_layout.position_for_index(caret) {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                origin,
                                                size(px(1.), caret_layout.line_height()),
                                            ),
                                            rgb(color),
                                        ));
                                    }
                                }
                            },
                        )
                        .absolute()
                        .inset_0(),
                    ),
            )
    }
}

fn utf8_offset(source: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (offset, ch) in source.char_indices() {
        if units + ch.len_utf16() > utf16 {
            return offset;
        }
        units += ch.len_utf16();
    }
    source.len()
}

fn merge_highlights(
    highlights: Vec<(Range<usize>, HighlightStyle)>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut cuts: Vec<_> = highlights
        .iter()
        .flat_map(|(r, _)| [r.start, r.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|pair| {
            let range = pair[0]..pair[1];
            highlights
                .iter()
                .filter(|(r, _)| r.start <= range.start && range.end <= r.end)
                .map(|(_, style)| *style)
                .reduce(HighlightStyle::highlight)
                .map(|style| (range, style))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_offsets_count_utf16_not_bytes() {
        let source = "é🦀名前";
        assert_eq!(
            (0..=6)
                .map(|at| utf8_offset(source, at))
                .collect::<Vec<_>>(),
            vec![0, 2, 2, 6, 9, 12, 12]
        );
    }

    #[test]
    fn selection_and_preedit_combine_with_bold_without_overlapping_runs() {
        let highlights = merge_highlights(vec![
            (
                0..4,
                HighlightStyle {
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                },
            ),
            (
                2..6,
                HighlightStyle {
                    background_color: Some(rgb(0x123456).into()),
                    ..Default::default()
                },
            ),
            (
                2..3,
                HighlightStyle {
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: false,
                    }),
                    ..Default::default()
                },
            ),
        ]);
        assert_eq!(
            highlights
                .iter()
                .map(|(range, _)| range.clone())
                .collect::<Vec<_>>(),
            vec![0..2, 2..3, 3..4, 4..6]
        );
        assert_eq!(highlights[1].1.font_weight, Some(FontWeight::BOLD));
        assert!(highlights[1].1.background_color.is_some());
        assert!(highlights[1].1.underline.is_some());
        assert!(highlights[2].1.underline.is_none());
    }
}
