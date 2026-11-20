//! Full-size PDF pages on either side of a proposed cut.

use std::sync::Arc;

use super::{split_picker::PAGES_PER_GRID, Documents};
use crate::ely::buttons::Button;
use crate::ely::theme::ControlSize;
use crate::pdf_kit::{page_view, zoom_controls, Zoom};
use gpui::{div, prelude::*, Context, RenderImage, Task};

pub(super) struct BoundaryPreview {
    before: usize,
    zoom: Zoom,
    pages: [Option<Result<Arc<RenderImage>, String>>; 2],
    _task: Task<()>,
}

#[cfg(test)]
impl BoundaryPreview {
    pub(super) fn rendered_pair(&self) -> Option<(usize, Zoom)> {
        self.pages
            .iter()
            .all(|page| matches!(page, Some(Ok(_))))
            .then_some((self.before, self.zoom))
    }
}

impl Documents {
    pub(super) fn inspect_boundary(&mut self, before: usize, zoom: Zoom, cx: &mut Context<Self>) {
        if before == 0 || before >= self.reader.pages {
            return;
        }
        let (Some(picker), Some(renderer), Some(bytes)) = (
            &mut self.split_picker,
            self.renderer.clone(),
            self.reader.bytes.clone(),
        ) else {
            return;
        };
        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            for (side, page) in [before - 1, before].into_iter().enumerate() {
                let renderer = renderer.clone();
                let bytes = bytes.clone();
                let image = executor
                    .spawn(async move {
                        renderer
                            .render(bytes, page, zoom.width())
                            .await
                            .map_err(|_| "The PDF renderer is unavailable.".to_owned())?
                            .map_err(|error| error.to_string())
                    })
                    .await;
                if this
                    .update(cx, |view, cx| {
                        if let Some(preview) = view
                            .split_picker
                            .as_mut()
                            .and_then(|picker| picker.preview.as_mut())
                        {
                            preview.pages[side] = Some(image);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
        });
        picker.preview = Some(BoundaryPreview {
            before,
            zoom,
            pages: [None, None],
            _task: task,
        });
        cx.notify();
    }

    pub(super) fn preview_boundary(
        &self,
        preview: &BoundaryPreview,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let before = preview.before;
        let zoom = preview.zoom;
        let cut = self
            .split_picker
            .as_ref()
            .is_some_and(|picker| picker.has_cut((before + 1) as u32));
        let matched = self
            .split_picker
            .as_ref()
            .and_then(|picker| picker.matched_span())
            .is_some_and(|span| span.first as usize == before + 1 || span.last as usize == before);
        let view = cx.entity().downgrade();
        let mut panes = div().flex().flex_1().min_h_0().gap_3();
        for (side, page) in preview.pages.iter().enumerate() {
            let body = match page {
                Some(Ok(image)) => {
                    page_view(("split-preview-page", before + side), image.clone(), zoom)
                }
                Some(Err(error)) => div().child(error.clone()).into_any_element(),
                None => div().child("Drawing page…").into_any_element(),
            };
            panes = panes.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(format!("Page {}", before + side))
                    .child(body),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .debug_selector(|| "split-preview-back-control".into())
                            .child(
                                Button::new("split-preview-back", "Back to thumbnails")
                                    .size(ControlSize::Sm)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if let Some(picker) = &mut view.split_picker {
                                            picker.preview = None;
                                            picker.offset =
                                                (before - 1) / PAGES_PER_GRID * PAGES_PER_GRID;
                                            view.request_thumbnails(cx);
                                            cx.notify();
                                        }
                                    })),
                            ),
                    )
                    .child(
                        Button::new("split-preview-prev", "Previous gap")
                            .size(ControlSize::Sm)
                            .disabled(before <= 1)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.inspect_boundary(before.saturating_sub(1), zoom, cx)
                            })),
                    )
                    .child(
                        Button::new("split-preview-next", "Next gap")
                            .size(ControlSize::Sm)
                            .disabled(before + 1 >= self.reader.pages)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.inspect_boundary(before + 1, zoom, cx)
                            })),
                    )
                    .child(
                        Button::new(
                            "split-preview-toggle",
                            if cut { "Remove cut" } else { "Add cut" },
                        )
                        .size(ControlSize::Sm)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.toggle_split_cut((before + 1) as u32, cx)
                        })),
                    )
                    .child(zoom_controls(
                        "split-preview-zoom",
                        zoom,
                        std::rc::Rc::new(move |zoom, _, cx| {
                            let _ =
                                view.update(cx, |view, cx| view.inspect_boundary(before, zoom, cx));
                        }),
                    )),
            )
            .children(
                self.detail
                    .value
                    .as_ref()
                    .and_then(|detail| detail.split_suggestions.as_ref())
                    .and_then(|suggestion| {
                        suggestion
                            .boundaries
                            .iter()
                            .find(|entry| entry.before_page as usize == before + 1)
                    })
                    .map(|entry| {
                        div()
                            .text_sm()
                            .child(format!("{} · {}", entry.marker, entry.quote))
                    }),
            )
            .when(matched, |preview| {
                preview.child(div().text_sm().child("Edge of a matching document span"))
            })
            .child(panes)
    }
}
