//! Phone layout for the Documents library, using signed reads and locally retained originals.

use std::{collections::HashMap, path::PathBuf};

use gpui::{
    div, prelude::*, px, rgb, rgba, uniform_list, Context, Entity, Focusable, FontWeight,
    Subscription, Task, Window,
};
use sjel_gpui_core::{
    device::Source,
    originals::Originals,
    shell::{self, DocumentDetail, DocumentSummary},
};
use tokio::sync::watch;

use super::{compare_documents, document_matches, document_title, DocSort, SortOrder};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        forms::{Input, InputEvent, TextInput},
        theme::ControlSize,
    },
    theme::{Density, ThemeContext},
};

/// The phone Documents view. UIKit presents originals from the path verified by core.
pub struct PhoneDocuments {
    source: Option<Source>,
    originals: Option<Originals>,
    show_pdf: fn(PathBuf),
    keyboard: fn(bool),
    back: fn(),
    rows: Vec<DocumentSummary>,
    order: Vec<usize>,
    order_by: DocSort,
    query: String,
    online_total: Option<usize>,
    offset: usize,
    detail: Option<DocumentDetail>,
    kept: bool,
    busy: bool,
    status: Option<String>,
    request: u64,
    search: Option<Entity<TextInput>>,
    subscriptions: Vec<Subscription>,
    _source_watch: Task<()>,
}

impl PhoneDocuments {
    /// Starts observing the phone's paired connection and reads its protected offline library.
    pub fn new(
        mut source: watch::Receiver<Option<Source>>,
        originals: Option<Originals>,
        show_pdf: fn(PathBuf),
        keyboard: fn(bool),
        back: fn(),
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_source = source.borrow_and_update().clone();
        let source_watch = cx.spawn(async move |this, cx| {
            let mut next = initial_source;
            loop {
                if this
                    .update(cx, |view, cx| {
                        view.source = next;
                        view.detail = None;
                        view.offset = 0;
                        view.online_total = None;
                        view.load_page(cx);
                    })
                    .is_err()
                {
                    break;
                }
                if source.changed().await.is_err() {
                    break;
                }
                next = source.borrow_and_update().clone();
            }
        });
        Self {
            source: None,
            originals,
            show_pdf,
            keyboard,
            back,
            rows: Vec::new(),
            order: Vec::new(),
            order_by: DocSort::Added,
            query: String::new(),
            online_total: None,
            offset: 0,
            detail: None,
            kept: false,
            busy: true,
            status: None,
            request: 0,
            search: None,
            subscriptions: Vec::new(),
            _source_watch: source_watch,
        }
    }

    fn begin(&mut self, cx: &mut Context<Self>) -> u64 {
        self.request = self.request.wrapping_add(1);
        self.busy = true;
        self.status = None;
        cx.notify();
        self.request
    }

    fn load_page(&mut self, cx: &mut Context<Self>) {
        let token = self.begin(cx);
        let offset = self.offset;
        let source = self.source.clone();
        let originals = self.originals.clone();
        let kept = originals.filter(|_| offset == 0).map(|store| {
            shell::run(async move {
                store
                    .kept()
                    .into_iter()
                    .map(|detail| detail.document)
                    .collect()
            })
        });
        let work = shell::run(async move {
            match source {
                Some(source) => source.documents_page(offset).await.map(Some),
                None => Ok(None),
            }
        });
        cx.spawn(async move |this, cx| {
            if offset == 0 {
                let rows = match kept {
                    Some(kept) => kept.await.unwrap_or_default(),
                    None => Vec::new(),
                };
                let _ = this.update(cx, |view, cx| {
                    if view.request == token {
                        view.rows = rows;
                        view.sort();
                        cx.notify();
                    }
                });
            }
            let result = work
                .await
                .unwrap_or_else(|| Err("The document request stopped.".into()));
            let _ = this.update(cx, |view, cx| {
                if view.request != token {
                    return;
                }
                match result {
                    Ok(Some(page)) => {
                        let received = page.documents.len();
                        let mut positions: HashMap<_, _> = view
                            .rows
                            .iter()
                            .enumerate()
                            .map(|(index, row)| (row.id.clone(), index))
                            .collect();
                        for row in page.documents {
                            let index = *positions.entry(row.id.clone()).or_insert(view.rows.len());
                            if index == view.rows.len() {
                                view.rows.push(row);
                            } else {
                                view.rows[index] = row;
                            }
                        }
                        view.offset = offset + received;
                        view.online_total = Some(if received == 0 {
                            view.offset
                        } else {
                            page.total
                        });
                    }
                    Ok(None) => {
                        view.status = Some("Pair with your Mac to read the library.".into());
                    }
                    Err(error) => {
                        view.status = Some(format!("The Mac did not answer: {error}"));
                    }
                }
                view.sort();
                view.busy = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn sort(&mut self) {
        let direction = match self.order_by {
            DocSort::Name | DocSort::Category => SortOrder::Ascending,
            _ => SortOrder::Descending,
        };
        self.order = (0..self.rows.len())
            .filter(|&index| document_matches(&self.rows[index], &self.query))
            .collect();
        self.order.sort_by(|&left, &right| {
            compare_documents(
                &self.rows[left],
                &self.rows[right],
                self.order_by,
                direction,
            )
        });
    }

    fn open(&mut self, id: String, cx: &mut Context<Self>) {
        (self.keyboard)(false);
        let token = self.begin(cx);
        let source = self.source.clone();
        let originals = self.originals.clone();
        let work = shell::run(async move {
            if let Some(detail) = originals
                .as_ref()
                .and_then(|store| store.kept_detail_of(&id))
            {
                return Ok((detail, true));
            }
            let source = source.ok_or_else(|| "The Mac is not reachable.".to_owned())?;
            source
                .document_detail(&id)
                .await
                .map(|detail| (detail, false))
        });
        cx.spawn(async move |this, cx| {
            let result = work
                .await
                .unwrap_or_else(|| Err("The document request stopped.".into()));
            let _ = this.update(cx, |view, cx| {
                if view.request != token {
                    return;
                }
                match result {
                    Ok((detail, kept)) => {
                        view.detail = Some(detail);
                        view.kept = kept;
                    }
                    Err(error) => view.status = Some(format!("The document did not open: {error}")),
                }
                view.busy = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn original(&mut self, cx: &mut Context<Self>) {
        let (Some(originals), Some(detail)) = (self.originals.clone(), &self.detail) else {
            return;
        };
        let document = detail.document.clone();
        let source = self.source.clone();
        let token = self.begin(cx);
        let work = shell::run(async move { originals.open(source.as_ref(), &document).await });
        cx.spawn(async move |this, cx| {
            let result = work
                .await
                .unwrap_or_else(|| Err("The document request stopped.".into()));
            let _ = this.update(cx, |view, cx| {
                if view.request != token {
                    return;
                }
                match result {
                    Ok(path) => (view.show_pdf)(path),
                    Err(error) => view.status = Some(format!("The original did not open: {error}")),
                }
                view.busy = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_kept(&mut self, cx: &mut Context<Self>) {
        let (Some(originals), Some(detail)) = (self.originals.clone(), self.detail.clone()) else {
            return;
        };
        let keep = !self.kept;
        let token = self.begin(cx);
        let work = shell::run(async move {
            if keep {
                originals.keep(&detail).map(drop)
            } else {
                originals.forget(&detail.document.id);
                Ok(())
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work
                .await
                .unwrap_or_else(|| Err("The document request stopped.".into()));
            let _ = this.update(cx, |view, cx| {
                if view.request != token {
                    return;
                }
                match result {
                    Ok(()) => {
                        view.kept = keep;
                        view.status = Some(
                            if keep {
                                "Kept on this phone."
                            } else {
                                "Removed from this phone."
                            }
                            .into(),
                        );
                    }
                    Err(error) => view.status = Some(error),
                }
                view.busy = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some(search) = &self.search {
            return search.clone();
        }
        let search = cx.new(|cx| TextInput::new(window, cx).placeholder("Search loaded documents"));
        let focus = search.read(cx).focus_handle(cx);
        self.subscriptions
            .push(cx.on_focus(&focus, window, |view, _, _| (view.keyboard)(true)));
        self.subscriptions
            .push(cx.on_blur(&focus, window, |view, _, _| (view.keyboard)(false)));
        self.subscriptions.push(
            cx.subscribe_in(&search, window, |view, search, event, _, cx| {
                if matches!(event, InputEvent::Changed) {
                    view.query = search.read(cx).text().trim().to_lowercase();
                    view.sort();
                    cx.notify();
                }
            }),
        );
        self.search = Some(search.clone());
        search
    }
}

impl Render for PhoneDocuments {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeContext::of(window, cx);
        let palette = theme.palette;
        let back_label = if self.detail.is_some() {
            "Library"
        } else {
            "Back"
        };
        let back = Button::new("documents-phone-back", back_label)
            .variant(ButtonVariant::Ghost)
            .size(ControlSize::Lg)
            .on_click(cx.listener(|view, _, _, cx| {
                if view.detail.take().is_some() {
                    view.request = view.request.wrapping_add(1);
                    view.busy = false;
                    view.status = None;
                    cx.notify();
                } else {
                    (view.keyboard)(false);
                    (view.back)();
                }
            }));
        let mut screen = div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .p_3()
            .gap_2()
            .bg(rgba(palette.surface))
            .text_color(rgb(palette.text))
            .child(
                div().flex().items_center().gap_2().child(back).child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Documents"),
                ),
            )
            .when_some(self.status.clone(), |screen, status| {
                screen.child(div().text_sm().text_color(rgb(palette.muted)).child(status))
            });

        if let Some(detail) = &self.detail {
            let can_open_pdf =
                self.originals.is_some() && detail.document.media_type == "application/pdf";
            let text = detail
                .text
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .unwrap_or("No extracted text for this document.");
            screen = screen
                .child(super::detail_panel(detail, palette))
                .when(can_open_pdf, |screen| {
                    screen.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("documents-phone-original", "Open original")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|view, _, _, cx| view.original(cx))),
                            )
                            .child(
                                Button::new(
                                    "documents-phone-kept",
                                    if self.kept {
                                        "Remove offline copy"
                                    } else {
                                        "Keep offline"
                                    },
                                )
                                .disabled(self.busy)
                                .on_click(cx.listener(|view, _, _, cx| view.toggle_kept(cx))),
                            ),
                    )
                })
                .child(
                    div()
                        .id("documents-phone-text")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(crate::doc_view::render_markdown(
                            text,
                            palette,
                            Density::Mobile,
                        )),
                );
        } else {
            let search = self.search_field(window, cx);
            let sort = [
                ("Newest", DocSort::Added),
                ("Name", DocSort::Name),
                ("Category", DocSort::Category),
                ("Pages", DocSort::Pages),
            ];
            let more = self.online_total.filter(|total| self.offset < *total);
            screen = screen
                .child(Input::new(&search))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(sort.into_iter().map(|(label, mode)| {
                            Button::new(label, label)
                                .variant(if self.order_by == mode {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Ghost
                                })
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.order_by = mode;
                                    view.sort();
                                    cx.notify();
                                }))
                        })),
                )
                .when(self.order.is_empty(), |screen| {
                    screen.child(div().text_sm().text_color(rgb(palette.muted)).child(
                        if self.busy {
                            "Reading documents…"
                        } else {
                            "No matching documents."
                        },
                    ))
                })
                .child(
                    uniform_list(
                        "documents-phone-rows",
                        self.order.len(),
                        cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|index| {
                                    let row = &view.rows[view.order[index]];
                                    let id = row.id.clone();
                                    let subtitle = format!(
                                        "{} · {} pages",
                                        row.imported_at.get(..10).unwrap_or("—"),
                                        row.page_count
                                    );
                                    div()
                                        .id(("documents-phone-row", index))
                                        .h(px(56.))
                                        .px_2()
                                        .flex()
                                        .flex_col()
                                        .justify_center()
                                        .border_b_1()
                                        .border_color(rgba(palette.border))
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.open(id.clone(), cx)
                                        }))
                                        .child(
                                            div().truncate().child(document_title(row).to_owned()),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(palette.muted))
                                                .truncate()
                                                .child(subtitle),
                                        )
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .flex_1()
                    .min_h_0(),
                )
                .when_some(more, |screen, total| {
                    screen.child(
                        Button::new(
                            "documents-phone-more",
                            format!("Load next page ({} of {total})", self.offset),
                        )
                        .disabled(self.busy)
                        .on_click(cx.listener(|view, _, _, cx| view.load_page(cx))),
                    )
                });
        }
        screen
    }
}
