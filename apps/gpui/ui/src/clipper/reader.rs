//! Clip document reader view, citation audits, and paginated block rendering.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, Window};
use sjel_gpui_core::blocks::Doc;
use sjel_gpui_core::clipper::{audit_citations, markdown_to_turns, CitationStatus, ClipDocument};

use crate::doc_view::render_doc;
use crate::primitives::note;
use crate::theme::{tint, Density, Palette};

use super::types::CLIP_PAGE_BLOCKS;
use super::Clips;

impl Clips {
    pub(super) fn reader(
        &self,
        document: &ClipDocument,
        doc: &Doc,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page_count = doc.blocks.len().max(1).div_ceil(CLIP_PAGE_BLOCKS);
        let page = self.reader_page.min(page_count.saturating_sub(1));
        let start = page * CLIP_PAGE_BLOCKS;
        let end = (start + CLIP_PAGE_BLOCKS).min(doc.blocks.len());
        let page_doc = Doc {
            front: String::new(),
            blocks: doc.blocks[start..end].to_vec(),
            trailing: 0,
        };
        let page_markdown = page_doc.markdown();
        let url = document.url.clone();
        let title = if document.title.is_empty() {
            document.key.clone()
        } else {
            document.title.clone()
        };
        let doc_for_edit = document.clone();
        let key = document.key.clone();
        let key_for_delete = key.clone();
        let confirm_delete = self.confirm_delete.as_deref() == Some(key.as_str());
        let status = self.delete_status.as_ref().map(|(ok, message)| {
            note(
                message.clone(),
                if *ok { palette.muted } else { palette.red },
            )
        });

        let mut navigation = div().flex().items_center().gap_2();
        if page > 0 {
            navigation = navigation.child(
                Button::new("clip-page-previous")
                    .small()
                    .ghost()
                    .label("← Previous")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.reader_page = view.reader_page.saturating_sub(1);
                        cx.notify();
                    })),
            );
        }
        navigation = navigation.child(note(
            format!(
                "Page {} of {} · blocks {}–{} of {}",
                page + 1,
                page_count,
                start + 1,
                end,
                doc.blocks.len()
            ),
            palette.muted,
        ));
        if page + 1 < page_count {
            navigation = navigation.child(
                Button::new("clip-page-next")
                    .small()
                    .ghost()
                    .label("Next →")
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.reader_page = (view.reader_page + 1).min(page_count - 1);
                        cx.notify();
                    })),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_4()
            .children(status)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("clips-back")
                            .small()
                            .ghost()
                            .label("← Clips")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.open = None;
                                view.parsed_doc = None;
                                view.confirm_delete = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("clips-edit-composer")
                            .small()
                            .ghost()
                            .label("Edit in Composer")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                let turns = markdown_to_turns(&doc_for_edit.markdown);
                                view.start_composer(
                                    doc_for_edit.title.clone(),
                                    doc_for_edit.url.clone(),
                                    turns,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("clips-original")
                            .small()
                            .ghost()
                            .label("Open original")
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .child(
                        Button::new("clips-delete-document")
                            .small()
                            .ghost()
                            .label(if confirm_delete {
                                "Confirm delete"
                            } else {
                                "Delete clip"
                            })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if view.confirm_delete.as_deref() == Some(key_for_delete.as_str()) {
                                    view.delete_document(key_for_delete.clone(), cx);
                                } else {
                                    view.confirm_delete = Some(key_for_delete.clone());
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(note(
                format!(
                    "{} · {} version{} · {}",
                    document.key,
                    document.versions,
                    if document.versions == 1 { "" } else { "s" },
                    document.updated_at
                ),
                palette.muted,
            ))
            .child(navigation)
            .child(render_citation_audit(&page_markdown, palette))
            .child(render_doc(
                &page_doc,
                palette,
                Density::Desktop,
                None::<fn(usize, &gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App)>,
            ))
            .into_any_element()
    }
}

pub(super) fn render_citation_audit(text: &str, palette: Palette) -> AnyElement {
    let citations = audit_citations(text);
    if citations.is_empty() {
        return div().into_any_element();
    }
    let resolved_count = citations
        .iter()
        .filter(|c| c.status == CitationStatus::Resolved)
        .count();
    let unlinked_count = citations
        .iter()
        .filter(|c| c.status == CitationStatus::UnlinkedRef)
        .count();
    let malformed_count = citations
        .iter()
        .filter(|c| c.status == CitationStatus::MalformedUrl)
        .count();

    let chips: Vec<AnyElement> = citations
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let (bg, border, text_color, desc) = match c.status {
                CitationStatus::Resolved => (
                    rgba(tint(palette.accent, 0x18)),
                    rgb(palette.accent),
                    rgb(palette.accent),
                    c.url.clone().unwrap_or_default(),
                ),
                CitationStatus::UnlinkedRef => (
                    rgba(tint(palette.amber, 0x24)),
                    rgb(palette.amber),
                    rgb(palette.amber),
                    "Missing target URL (bare [N] from plain paste)".to_string(),
                ),
                CitationStatus::MalformedUrl => (
                    rgba(tint(palette.red, 0x24)),
                    rgb(palette.red),
                    rgb(palette.red),
                    format!("Invalid URL: {}", c.url.clone().unwrap_or_default()),
                ),
            };

            let target_url = c.url.clone();
            let mut chip = div()
                .id(("citation-chip", i))
                .flex()
                .items_center()
                .gap_1p5()
                .px_2()
                .py_0p5()
                .rounded_md()
                .border_1()
                .border_color(border)
                .bg(bg)
                .text_xs();

            if let Some(url) = target_url {
                chip = chip.cursor_pointer().on_click(move |_, _, cx| {
                    let clean = sjel_gpui_core::clipper::unwrap_google_redirect(&url);
                    if clean.contains("vertexaisearch.cloud.google.com/grounding-api-redirect/") {
                        let clean_clone = clean.clone();
                        cx.spawn(async move |cx| {
                            let resolved =
                                sjel_gpui_core::clipper::resolve_redirect_http(&clean_clone).await;
                            let _ = cx.update(|cx| cx.open_url(&resolved));
                        })
                        .detach();
                    } else {
                        cx.open_url(&clean);
                    }
                });
            }

            chip.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text_color)
                    .child(c.label),
            )
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .max_w(px(260.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(desc),
            )
            .into_any_element()
        })
        .collect();

    let summary_text = format!(
        "Citation & Link Audit: {} resolved{}, {} unlinked{}",
        resolved_count,
        if malformed_count > 0 {
            format!(", {} malformed", malformed_count)
        } else {
            String::new()
        },
        unlinked_count,
        if unlinked_count > 0 {
            " (paste rich HTML from AI Studio to preserve citation links)"
        } else {
            ""
        }
    );

    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_lg()
        .bg(rgba(tint(palette.surface, 0x30)))
        .border_1()
        .border_color(if unlinked_count > 0 {
            rgb(palette.amber)
        } else {
            rgba(palette.border)
        })
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(if unlinked_count > 0 {
                    rgb(palette.amber)
                } else {
                    rgb(palette.muted)
                })
                .child(summary_text),
        )
        .child(div().flex().flex_wrap().gap_2().children(chips))
        .into_any_element()
}
