//! The NSH trial (`docs/plans/gpui.md` §11, stage 3): the phone's Documents reader drawn with UIKit
//! views by NSH, mounted by `host/App.swift` in a view beside the GPUI one.
//!
//! Reads go through the paired phone's `DeviceClient` on core's runtime (`shell::spawn`). Their
//! results cross to NSH's engine thread over one channel, which [`Trial`] drains.

use std::sync::{Arc, Mutex, OnceLock};

use nsh_base::button;
use nsh_core::prelude::*;
use sjel_gpui_core::device::DeviceClient;
use sjel_gpui_core::shell::{self, DocumentDetail, DocumentSummary};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// What a read on core's runtime hands to the screen.
enum Update {
    Status(String),
    Rows(Vec<DocumentSummary>),
    Detail(Box<DocumentDetail>),
}

struct Feed {
    tx: UnboundedSender<Update>,
    rx: Mutex<Option<UnboundedReceiver<Update>>>,
    client: Mutex<Option<Arc<DeviceClient>>>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let (tx, rx) = unbounded_channel();
        Feed {
            tx,
            rx: Mutex::new(Some(rx)),
            client: Mutex::new(None),
        }
    })
}

fn send(update: Update) {
    let _ = feed().tx.send(update);
}

/// Called when the phone is paired: lists every document the Mac's library holds.
pub fn connect(client: Arc<DeviceClient>) {
    *feed().client.lock().unwrap_or_else(|e| e.into_inner()) = Some(client.clone());
    send(Update::Status("Reading the library on the Mac…".into()));
    shell::spawn(async move {
        let mut offset = 0;
        loop {
            match shell::device_documents_page(&client, offset).await {
                Ok(page) => {
                    let len = page.documents.len();
                    offset += len;
                    send(Update::Rows(page.documents));
                    if offset >= page.total || len == 0 {
                        send(Update::Status(format!("{offset} documents")));
                        return;
                    }
                }
                Err(reason) => {
                    send(Update::Status(format!("The Mac did not answer: {reason}")));
                    return;
                }
            }
        }
    });
}

fn open(id: String) {
    let client = feed()
        .client
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(client) = client else {
        return;
    };
    shell::spawn(async move {
        match shell::device_document_detail(&client, &id).await {
            Ok(detail) => send(Update::Detail(Box::new(detail))),
            Err(reason) => send(Update::Status(format!(
                "The document did not open: {reason}"
            ))),
        }
    });
}

fn title(doc: &DocumentSummary) -> String {
    doc.extracted_title
        .clone()
        .unwrap_or_else(|| doc.filename.clone())
}

fn subtitle(doc: &DocumentSummary) -> String {
    let day = doc.imported_at.get(..10).unwrap_or(&doc.imported_at);
    let mut parts = vec![day.to_string()];
    if doc.page_count > 0 {
        parts.push(format!("{} pages", doc.page_count));
    }
    if let Some(category) = &doc.category {
        parts.push(category.clone());
    }
    parts.join(" · ")
}

#[component]
pub fn Trial() -> impl IntoElement {
    let rows = signal(Vec::<DocumentSummary>::new());
    let status = signal(String::from(
        "Pair this phone with the Mac to read its documents.",
    ));
    let detail = signal(None::<DocumentDetail>);
    let receiver = feed().rx.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(mut rx) = receiver {
        spawn(async move {
            while let Some(update) = rx.recv().await {
                match update {
                    Update::Status(text) => status.set(text),
                    Update::Rows(batch) => rows.update(|all| all.extend(batch)),
                    Update::Detail(found) => detail.set(Some(*found)),
                }
            }
        });
    }
    view()
        .flex_1()
        .child(
            view()
                .px_4()
                .pt_4()
                .pb_2()
                .gap_1()
                .child(
                    text("Documents")
                        .text_2xl()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(text(move || status.get()).text_sm()),
        )
        .child(dynamic(move || match detail.with(|d| d.clone()) {
            Some(found) => reader(found, detail).into_element(),
            None => list(rows).into_element(),
        }))
}

fn list(rows: Signal<Vec<DocumentSummary>>) -> impl IntoElement {
    virtual_list(
        move || rows.with(|r| r.len()),
        move |at| {
            let at_row = move || rows.with(|r| r.get(at.index()).cloned().unwrap_or_default());
            button()
                .px_4()
                .py_3()
                .gap_1()
                .on_press(move || open(at_row().id))
                .child(text(move || title(&at_row())).text_base())
                .child(text(move || subtitle(&at_row())).text_sm())
        },
    )
    .estimated_item_size(px(64.0))
    .flex_1()
}

fn reader(found: DocumentDetail, detail: Signal<Option<DocumentDetail>>) -> impl IntoElement {
    let body = found
        .text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "No extracted text for this document.".into());
    view()
        .flex_1()
        .child(
            button()
                .px_4()
                .py_2()
                .on_press(move || detail.set(None))
                .child(text("‹ Documents").text_base()),
        )
        .child(
            scroll().flex_1().child(
                view()
                    .px_4()
                    .pb_4()
                    .gap_2()
                    .child(
                        text(title(&found.document))
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .select_text(),
                    )
                    .child(text(subtitle(&found.document)).text_sm())
                    .child(text(body).text_base().select_text()),
            ),
        )
}

#[cfg(target_os = "ios")]
thread_local! {
    /// Dropping the `AppView` stops the app, so the mounted one lives here.
    static MOUNTED: std::cell::RefCell<Option<nsh_platform_ios::AppView>> =
        const { std::cell::RefCell::new(None) };
}

/// Mounts [`Trial`] into `container`. Returns `false` off the main thread.
#[cfg(target_os = "ios")]
pub fn mount(container: &objc2_ui_kit::UIView) -> bool {
    use nsh_platform_ios::{set_ui_sets, AppView, Root, SurfaceOptions};
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return false;
    };
    set_ui_sets(vec![nsh_base::registry()]);
    let app = AppView::new(mtm, container.bounds(), SurfaceOptions::default());
    container.addSubview(app.view());
    app.run(Root::of::<Trial>());
    MOUNTED.with(|slot| *slot.borrow_mut() = Some(app));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_names_the_title_or_the_file_and_states_day_pages_and_category() {
        let mut doc = DocumentSummary {
            filename: "scan.pdf".into(),
            imported_at: "2026-11-07T14:00:00Z".into(),
            ..Default::default()
        };
        assert_eq!(title(&doc), "scan.pdf");
        assert_eq!(subtitle(&doc), "2026-11-07");
        doc.extracted_title = Some("Rental contract".into());
        doc.page_count = 3;
        doc.category = Some("housing".into());
        assert_eq!(title(&doc), "Rental contract");
        assert_eq!(subtitle(&doc), "2026-11-07 · 3 pages · housing");
    }
}
