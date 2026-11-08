//! The Documents reader drawn by NSH (`docs/plans/gpui.md` §11): UIKit views on the phone, GPUI
//! Fast inside the Mac app.
//!
//! Reads run on core's runtime (`shell::spawn`). Their results cross to NSH's engine thread over
//! one channel, which [`DocumentsScreen`] drains. Originals go through
//! `sjel_gpui_core::originals`: checked against the library's SHA-256, and kept on the phone when
//! the reader asks.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use nsh_base::{button, text_view as nsh_markdown_view, TextViewLinks};
use nsh_core::prelude::*;
use serde_json::json;
use sjel_gpui_core::device::Source;
use sjel_gpui_core::originals::Originals;
use sjel_gpui_core::shell::{self, DocumentDetail, DocumentSummary};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::nsh_mail::mail_screen;

/// The native component that shows a PDF at a file path (`ios/src/nsh.rs` registers it).
pub const PDF_COMPONENT: &str = "sjel.pdf";

/// Which shared NSH screen [`NshHostScreen`] displays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NshPage {
    #[default]
    Documents,
    Mail,
}

struct PageRouter {
    current: Mutex<NshPage>,
    tx: Mutex<Option<UnboundedSender<NshPage>>>,
}

fn page_router() -> &'static PageRouter {
    static ROUTER: OnceLock<PageRouter> = OnceLock::new();
    ROUTER.get_or_init(|| PageRouter {
        current: Mutex::new(NshPage::Documents),
        tx: Mutex::new(None),
    })
}

/// Switches [`NshHostScreen`] to `page` (`Documents` or `Mail`).
pub fn set_nsh_page(page: NshPage) {
    *page_router()
        .current
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = page;
    if let Some(tx) = page_router()
        .tx
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        let _ = tx.send(page);
    }
}

/// Returns the active [`NshPage`].
pub fn nsh_page() -> NshPage {
    *page_router()
        .current
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// What a read on core's runtime hands to the screen.
enum Update {
    Status(String),
    Rows {
        rows: Vec<DocumentSummary>,
        replace: bool,
        next_offset: Option<usize>,
        total: usize,
    },
    Detail {
        detail: Box<DocumentDetail>,
        kept: bool,
    },
    Original(String),
    Kept(bool),
}

#[derive(Clone)]
struct Snapshot {
    rows: Vec<DocumentSummary>,
    next_page: Option<(usize, usize)>,
    status: String,
}

type SwitchTab = Box<dyn Fn(&str) + Send>;

struct Feed {
    tx: Mutex<Option<UnboundedSender<Update>>>,
    snapshot: Mutex<Snapshot>,
    source: Mutex<Option<Source>>,
    originals: Mutex<Option<Originals>>,
    close: Mutex<Option<Box<dyn Fn() + Send>>>,
    switch_tab: Mutex<Option<SwitchTab>>,
    pdf_viewer: AtomicBool,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| Feed {
        tx: Mutex::new(None),
        snapshot: Mutex::new(Snapshot {
            rows: Vec::new(),
            next_page: None,
            status: String::from("Waiting for the library."),
        }),
        source: Mutex::new(None),
        originals: Mutex::new(None),
        close: Mutex::new(None),
        switch_tab: Mutex::new(None),
        pdf_viewer: AtomicBool::new(false),
    })
}

fn locked<T: Clone>(slot: &Mutex<Option<T>>) -> Option<T> {
    slot.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn send(update: Update) {
    {
        let mut snap = feed().snapshot.lock().unwrap_or_else(|e| e.into_inner());
        match &update {
            Update::Status(text) => snap.status = text.clone(),
            Update::Rows {
                rows,
                replace,
                next_offset,
                total,
            } => {
                if *replace {
                    snap.rows.clear();
                }
                snap.rows.extend(rows.iter().cloned());
                snap.next_page = next_offset.map(|off| (off, *total));
            }
            Update::Detail { .. } | Update::Original(_) | Update::Kept(_) => {}
        }
    }
    if let Some(tx) = feed().tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(update);
    }
}

/// What "Back" does at the top of the list. Without it, the list has no "Back".
pub fn set_close(close: impl Fn() + Send + 'static) {
    *feed().close.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(close));
}

/// Allows host tab switching when navigating away from NSH screens.
pub fn set_switch_tab(switch: impl Fn(&str) + Send + 'static) {
    *feed().switch_tab.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(switch));
}

fn switch_tab_target(target: &str) {
    if let Some(sw) = feed()
        .switch_tab
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        sw(target);
    } else {
        close();
    }
}

fn close() {
    if let Some(close) = feed()
        .close
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        close();
    }
}

/// Says that the host registered [`PDF_COMPONENT`], so the reader offers the original.
pub fn set_pdf_viewer() {
    feed().pdf_viewer.store(true, Ordering::Relaxed);
}

/// Where originals live on this device. The documents kept here show at once, before any read.
pub fn set_originals(originals: Originals) {
    let kept: Vec<DocumentSummary> = originals.kept().into_iter().map(|d| d.document).collect();
    let count = kept.len();
    *feed().originals.lock().unwrap_or_else(|e| e.into_inner()) = Some(originals);
    if !kept.is_empty() {
        send(Update::Status(kept_status(count)));
        send(Update::Rows {
            rows: kept,
            replace: true,
            next_offset: None,
            total: count,
        });
    }
}

fn kept_status(count: usize) -> String {
    match count {
        1 => "1 document kept on this phone".into(),
        n => format!("{n} documents kept on this phone"),
    }
}

fn page_status(loaded: usize, total: usize) -> String {
    if loaded >= total {
        format!("{total} documents")
    } else {
        format!("Showing {loaded} of {total} documents")
    }
}

/// Lists the first page of documents from `source`: the Mac's shell, or a paired phone's client.
/// Additional pages load on demand through [`load_more`]. When the Mac does not answer, the
/// documents kept on this phone stay listed.
pub fn connect(source: Source) {
    *feed().source.lock().unwrap_or_else(|e| e.into_inner()) = Some(source);
    send(Update::Status("Reading the library on the Mac…".into()));
    fetch_page(0);
}

fn fetch_page(offset: usize) {
    let Some(source) = locked(&feed().source) else {
        return;
    };
    shell::spawn(async move {
        match source.documents_page(offset).await {
            Ok(page) => {
                let len = page.documents.len();
                let loaded = offset + len;
                let next_offset = (loaded < page.total && len > 0).then_some(loaded);
                send(Update::Rows {
                    rows: page.documents,
                    replace: offset == 0,
                    next_offset,
                    total: page.total,
                });
                send(Update::Status(page_status(loaded, page.total)));
            }
            Err(reason) => {
                let kept = locked(&feed().originals).map_or(0, |o| o.kept().len());
                let mut status = format!("The Mac did not answer: {reason}");
                if kept > 0 && offset == 0 {
                    status.push_str(&format!(". Showing the {}.", kept_status(kept)));
                }
                send(Update::Status(status));
            }
        }
    });
}

fn open(id: String) {
    if let Some(detail) = locked(&feed().originals).and_then(|o| o.kept_detail_of(&id)) {
        send(Update::Detail {
            detail: Box::new(detail),
            kept: true,
        });
        return;
    }
    let Some(source) = locked(&feed().source) else {
        return;
    };
    shell::spawn(async move {
        match source.document_detail(&id).await {
            Ok(detail) => send(Update::Detail {
                detail: Box::new(detail),
                kept: false,
            }),
            Err(reason) => send(Update::Status(format!(
                "The document did not open: {reason}"
            ))),
        }
    });
}

fn open_original(document: DocumentSummary) {
    let Some(originals) = locked(&feed().originals) else {
        return;
    };
    let source = locked(&feed().source);
    send(Update::Status(format!("Opening {}…", title(&document))));
    shell::spawn(async move {
        match originals.open(source.as_ref(), &document).await {
            Ok(path) => {
                send(Update::Status(
                    "Original checked against the library's SHA-256.".into(),
                ));
                send(Update::Original(path_text(&path)));
            }
            Err(reason) => send(Update::Status(format!(
                "The original did not open: {reason}"
            ))),
        }
    });
}

fn set_kept(detail: DocumentDetail, keep: bool) {
    let Some(originals) = locked(&feed().originals) else {
        return;
    };
    if keep {
        match originals.keep(&detail) {
            Ok(_) => {
                send(Update::Status("Kept on this phone.".into()));
                send(Update::Kept(true));
            }
            Err(reason) => send(Update::Status(reason)),
        }
    } else {
        originals.forget(&detail.document.id);
        send(Update::Status("Removed from this phone.".into()));
        send(Update::Kept(false));
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
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

fn is_pdf(doc: &DocumentSummary) -> bool {
    doc.media_type == "application/pdf"
}

/// The reader's state for the document on screen.
#[derive(Clone, Copy)]
struct Open {
    detail: Signal<Option<DocumentDetail>>,
    original: Signal<Option<String>>,
    kept: Signal<bool>,
}

#[component]
pub fn NshHostScreen() -> impl IntoElement {
    let page = signal(nsh_page());
    let (tx, mut rx) = unbounded_channel();
    *page_router().tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    spawn(async move {
        while let Some(next) = rx.recv().await {
            page.set(next);
        }
    });
    let closable = feed()
        .close
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    let page_chip = move |target: NshPage, label: &'static str| {
        button()
            .px_3()
            .py_1()
            .on_press(move || set_nsh_page(target))
            .child(
                text(move || {
                    if page.get() == target {
                        format!("{label} •")
                    } else {
                        label.to_owned()
                    }
                })
                .text_xs(),
            )
    };
    let tab_item =
        move |label: &'static str, target_page: Option<NshPage>, route_name: &'static str| {
            let is_active = move || target_page.map(|tp| page.get() == tp).unwrap_or(false);
            button()
                .flex_1()
                .py_2()
                .on_press(move || {
                    if let Some(tp) = target_page {
                        set_nsh_page(tp);
                    } else {
                        switch_tab_target(route_name);
                    }
                })
                .child(
                    text(move || {
                        if is_active() {
                            format!("{label} •")
                        } else {
                            label.to_owned()
                        }
                    })
                    .text_xs(),
                )
        };
    let bottom_tab_bar = view()
        .flex_row()
        .items_center()
        .px_2()
        .pt_1()
        .pb_2()
        .child(tab_item("Today", None, "today"))
        .child(tab_item("Mail", Some(NshPage::Mail), "mail"))
        .child(tab_item("Documents", Some(NshPage::Documents), "documents"))
        .child(tab_item("Calendar", None, "calendar"))
        .child(tab_item("Travel", None, "travel"))
        .child(tab_item("Agent", None, "agent"));

    view()
        .flex_1()
        .child((!closable).then(|| {
            view()
                .flex_row()
                .px_4()
                .pt_2()
                .gap_2()
                .child(page_chip(NshPage::Documents, "Documents"))
                .child(page_chip(NshPage::Mail, "Mail"))
        }))
        .child(view().flex_1().child(dynamic(move || match page.get() {
            NshPage::Documents => documents_screen().into_element(),
            NshPage::Mail => mail_screen().into_element(),
        })))
        .child(closable.then_some(bottom_tab_bar))
}

#[component]
pub fn DocumentsScreen() -> impl IntoElement {
    let initial = feed()
        .snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let rows = signal(initial.rows);
    let next_page = signal(initial.next_page);
    let status = signal(initial.status);
    let open_doc = Open {
        detail: signal(None::<DocumentDetail>),
        original: signal(None::<String>),
        kept: signal(false),
    };
    let (tx, mut rx) = unbounded_channel();
    *feed().tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    spawn(async move {
        while let Some(update) = rx.recv().await {
            match update {
                Update::Status(text) => status.set(text),
                Update::Rows {
                    rows: batch,
                    replace,
                    next_offset,
                    total,
                } => {
                    rows.update(|all| {
                        if replace {
                            all.clear();
                        }
                        all.extend(batch);
                    });
                    next_page.set(next_offset.map(|off| (off, total)));
                }
                Update::Detail { detail, kept } => {
                    open_doc.original.set(None);
                    open_doc.kept.set(kept);
                    open_doc.detail.set(Some(*detail));
                }
                Update::Original(path) => open_doc.original.set(Some(path)),
                Update::Kept(kept) => open_doc.kept.set(kept),
            }
        }
    });
    let closable = feed()
        .close
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    view()
        .flex_1()
        .child(closable.then(|| {
            button()
                .px_4()
                .pt_2()
                .on_press(close)
                .child(text("‹ Back").text_base())
        }))
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
        .child(dynamic(move || match open_doc.detail.with(|d| d.clone()) {
            Some(found) => reader(found, open_doc).into_element(),
            None => list(rows, next_page).into_element(),
        }))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NshDocSort {
    #[default]
    Newest,
    Name,
    Category,
    Pages,
}

impl NshDocSort {
    fn label(self) -> &'static str {
        match self {
            Self::Newest => "Newest",
            Self::Name => "Name A–Z",
            Self::Category => "Category",
            Self::Pages => "Pages",
        }
    }
}

fn sort_nsh_rows(rows: &mut [DocumentSummary], sort: NshDocSort) {
    rows.sort_by(|a, b| match sort {
        NshDocSort::Newest => b
            .imported_at
            .cmp(&a.imported_at)
            .then_with(|| title(a).to_lowercase().cmp(&title(b).to_lowercase())),
        NshDocSort::Name => title(a)
            .to_lowercase()
            .cmp(&title(b).to_lowercase())
            .then_with(|| b.imported_at.cmp(&a.imported_at)),
        NshDocSort::Category => a
            .category
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .cmp(&b.category.as_deref().unwrap_or("").to_lowercase())
            .then_with(|| b.imported_at.cmp(&a.imported_at)),
        NshDocSort::Pages => b
            .page_count
            .cmp(&a.page_count)
            .then_with(|| b.imported_at.cmp(&a.imported_at)),
    });
}

fn list(
    rows: Signal<Vec<DocumentSummary>>,
    next_page: Signal<Option<(usize, usize)>>,
) -> impl IntoElement {
    let sort_mode = signal(NshDocSort::Newest);
    let sort_chip = move |mode: NshDocSort| {
        button()
            .px_3()
            .py_1()
            .on_press(move || {
                sort_mode.set(mode);
                rows.update(|all| sort_nsh_rows(all, mode));
            })
            .child(
                text(move || {
                    if sort_mode.get() == mode {
                        format!("{} •", mode.label())
                    } else {
                        mode.label().to_owned()
                    }
                })
                .text_xs(),
            )
    };
    view()
        .flex_1()
        .child(
            view()
                .flex_row()
                .px_4()
                .pb_2()
                .gap_2()
                .child(sort_chip(NshDocSort::Newest))
                .child(sort_chip(NshDocSort::Name))
                .child(sort_chip(NshDocSort::Category))
                .child(sort_chip(NshDocSort::Pages)),
        )
        .child(
            virtual_list(
                move || rows.with(|r| r.len()),
                move |at| {
                    let at_row =
                        move || rows.with(|r| r.get(at.index()).cloned().unwrap_or_default());
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
            .flex_1(),
        )
        .child(dynamic(move || match next_page.get() {
            Some((offset, total)) => button()
                .px_4()
                .py_3()
                .on_press(move || fetch_page(offset))
                .child(text(format!("Load next page ({offset} of {total})")).text_sm())
                .into_element(),
            None => view().into_element(),
        }))
}

fn action(label: impl IntoReactive<SharedString>, press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .px_3()
        .py_2()
        .on_press(press)
        .child(text(label).text_base())
}

fn reader(found: DocumentDetail, open_doc: Open) -> impl IntoElement {
    let body = found
        .text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "No extracted text for this document.".into());
    let offers_original = feed().pdf_viewer.load(Ordering::Relaxed) && is_pdf(&found.document);
    let document = found.document.clone();
    let for_keep = found.clone();
    let actions = view()
        .flex_row()
        .px_1()
        .child(action("‹ Documents", move || open_doc.detail.set(None)))
        .child(offers_original.then(|| {
            action(
                move || {
                    if open_doc.original.with(Option::is_some) {
                        "Text"
                    } else {
                        "Original"
                    }
                },
                move || {
                    if open_doc.original.with(Option::is_some) {
                        open_doc.original.set(None);
                    } else {
                        open_original(document.clone());
                    }
                },
            )
        }))
        .child(offers_original.then(|| {
            action(
                move || {
                    if open_doc.kept.get() {
                        "Remove from phone"
                    } else {
                        "Keep offline"
                    }
                },
                move || set_kept(for_keep.clone(), !open_doc.kept.get()),
            )
        }));
    view().flex_1().child(actions).child(dynamic(move || {
        match open_doc.original.with(|p| p.clone()) {
            Some(path) => native(PDF_COMPONENT)
                .prop("path", json!(path))
                .flex_1()
                .into_element(),
            None => text_view(&found, &body).into_element(),
        }
    }))
}

fn text_view(found: &DocumentDetail, body: &str) -> impl IntoElement {
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
            .child(
                nsh_markdown_view()
                    .markdown(body.to_string())
                    .selectable(true)
                    .links(TextViewLinks::Open),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_STATE: Mutex<()> = Mutex::new(());

    #[test]
    fn a_row_names_the_title_or_the_file_and_states_day_pages_and_category() {
        let _state = TEST_STATE.lock().unwrap();
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
        assert_eq!(kept_status(1), "1 document kept on this phone");
        assert_eq!(page_status(500, 7154), "Showing 500 of 7154 documents");
        assert_eq!(page_status(7154, 7154), "7154 documents");

        let mut list = vec![
            DocumentSummary {
                id: "1".into(),
                filename: "z.pdf".into(),
                imported_at: "2026-11-01".into(),
                page_count: 2,
                ..Default::default()
            },
            DocumentSummary {
                id: "2".into(),
                filename: "a.pdf".into(),
                imported_at: "2026-11-05".into(),
                page_count: 10,
                ..Default::default()
            },
        ];
        sort_nsh_rows(&mut list, NshDocSort::Name);
        assert_eq!(list[0].id, "2");
        sort_nsh_rows(&mut list, NshDocSort::Pages);
        assert_eq!(list[0].page_count, 10);

        set_nsh_page(NshPage::Mail);
        assert_eq!(nsh_page(), NshPage::Mail);
        set_nsh_page(NshPage::Documents);
        assert_eq!(nsh_page(), NshPage::Documents);

        let switched = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let sw_clone = switched.clone();
        set_switch_tab(move |target| {
            *sw_clone.lock().unwrap() = target.to_string();
        });
        switch_tab_target("calendar");
        assert_eq!(*switched.lock().unwrap(), "calendar");
    }

    #[test]
    fn a_loaded_page_and_its_reader_render_in_the_phone_host() {
        use nsh_core::element::Root;
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;

        let _state = TEST_STATE.lock().unwrap();
        set_close(|| {});
        set_nsh_page(NshPage::Documents);
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<NshHostScreen>())
        })
        .expect("the phone's NSH root starts");
        app.wait_for("text:Documents").unwrap();

        let rows: Vec<_> = (0..500)
            .map(|at| DocumentSummary {
                id: format!("{at:064x}"),
                filename: format!("Document {at:03}.pdf"),
                page_count: 3,
                ..Default::default()
            })
            .collect();
        let first = rows[0].clone();
        send(Update::Rows {
            rows,
            replace: true,
            next_offset: Some(500),
            total: 1200,
        });
        app.wait_for("text:Document 000.pdf").unwrap();
        app.press("text:Name A–Z").unwrap();
        app.wait_for("text:Name A–Z •").unwrap();
        send(Update::Detail {
            detail: Box::new(DocumentDetail {
                document: first,
                text: Some("A synthetic document body.".into()),
                ..Default::default()
            }),
            kept: false,
        });
        app.wait_for("text:A synthetic document body.").unwrap();
    }
}
