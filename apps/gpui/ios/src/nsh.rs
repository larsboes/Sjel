//! Mounts the NSH screens (`sjel_gpui_ui::nsh_documents::NshHostScreen`: Today, Documents, Mail)
//! into a view from `host/App.swift`, over the GPUI one (`docs/plans/gpui.md` §11). The view
//! shows at launch on Today and hides while a GPUI page is open. PDFs show in PDFKit through the `sjel.pdf` component, whose view
//! `host/Documents.swift` makes.

#[cfg(target_os = "ios")]
thread_local! {
    /// Dropping the `AppView` stops the app, so the mounted one lives here.
    static MOUNTED: std::cell::RefCell<Option<nsh_platform_ios::AppView>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // Declaring the host's symbols is unsafe; the host owns their definitions.
unsafe extern "C" {
    fn sjel_ios_pdf_view() -> *mut std::ffi::c_void;
    fn sjel_ios_pdf_load(view: *mut std::ffi::c_void, path: *const std::ffi::c_char);
    fn sjel_ios_protected_folder(path: *const std::ffi::c_char) -> bool;
}

/// A PDFKit view showing the file at its `path` prop.
#[cfg(target_os = "ios")]
struct Pdf {
    view: objc2::rc::Retained<objc2_ui_kit::UIView>,
}

#[cfg(target_os = "ios")]
impl nsh_platform_ios::NativeComponent for Pdf {
    fn view(&self) -> &objc2_ui_kit::UIView {
        &self.view
    }

    #[allow(unsafe_code)]
    fn set_prop(&mut self, name: &str, value: Option<&serde_json::Value>) {
        let Some(path) = value.and_then(|v| v.as_str()) else {
            return;
        };
        let (true, Ok(path)) = (name == "path", std::ffi::CString::new(path)) else {
            return;
        };
        let view = objc2::rc::Retained::as_ptr(&self.view).cast_mut().cast();
        // SAFETY: `view` is the live PDFView this component owns; `path` outlives the call.
        unsafe { sjel_ios_pdf_load(view, path.as_ptr()) }
    }
}

/// Where originals live: a cache the system may clear, and a kept folder whose files are
/// unreadable while the phone is locked. `None` when the kept folder cannot be protected.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
fn originals() -> Option<sjel_gpui_core::originals::Originals> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
    let cache = home.join("Library/Caches/originals");
    let kept = home.join("Library/Application Support/kept-originals");
    let kept_c = std::ffi::CString::new(kept.to_str()?).ok()?;
    // SAFETY: `kept_c` is a NUL-terminated path that outlives the call.
    unsafe { sjel_ios_protected_folder(kept_c.as_ptr()) }
        .then(|| sjel_gpui_core::originals::Originals::new(cache, kept))
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
fn mail_offline() -> Option<sjel_gpui_core::mail::MailOffline> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
    let kept = home.join("Library/Application Support/kept-mail");
    let kept_c = std::ffi::CString::new(kept.to_str()?).ok()?;
    // SAFETY: `kept_c` is a NUL-terminated path that outlives the call.
    unsafe { sjel_ios_protected_folder(kept_c.as_ptr()) }
        .then(|| sjel_gpui_core::mail::MailOffline::new(kept))
}

/// Mounts the reader into `container`. Returns `false` off the main thread.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
pub fn mount(container: &objc2_ui_kit::UIView) -> bool {
    use nsh_platform_ios::SurfaceOptions;
    use nsh_platform_ios::{set_ui_sets, AppView, ComponentContext, NativeComponent, Root};
    use sjel_gpui_ui::nsh_documents::{self as docs, NshHostScreen, PDF_COMPONENT};
    use sjel_gpui_ui::nsh_mail as mail;
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return false;
    };
    set_ui_sets(vec![nsh_base::registry()]);
    // "Back" from Documents or Mail returns to Today, which is the NSH screen under them.
    docs::set_close(|| docs::set_nsh_page(docs::NshPage::Today));
    mail::set_close(|| docs::set_nsh_page(docs::NshPage::Today));
    docs::set_switch_tab(|target| {
        sjel_gpui_ui::nsh_map::hide();
        crate::host::hide_nsh();
        match target {
            "today" => crate::host::announce(crate::Event::SelectTab(crate::Tab::Today)),
            "calendar" => crate::host::announce(crate::Event::SelectTab(crate::Tab::Calendar)),
            "travel" => crate::host::announce(crate::Event::SelectTab(crate::Tab::Travel)),
            "agent" => crate::host::announce(crate::Event::OpenEdge(crate::Seat::Agent)),
            _ => crate::host::announce(crate::Event::SelectTab(crate::Tab::Today)),
        }
    });
    if let Some(originals) = originals() {
        docs::set_originals(originals);
        docs::set_pdf_viewer();
    }
    if let Some(offline) = mail_offline() {
        mail::set_offline(offline);
    }
    let app = AppView::new(mtm, container.bounds(), SurfaceOptions::default());
    app.register_component(
        PDF_COMPONENT,
        std::rc::Rc::new(|_: ComponentContext| -> Box<dyn NativeComponent> {
            // SAFETY: the host returns a +1 retained `PDFView`, a `UIView`; `from_raw` takes it.
            let view = unsafe { objc2::rc::Retained::from_raw(sjel_ios_pdf_view().cast()) }
                .expect("the host made a PDF view");
            Box::new(Pdf { view })
        }),
    );
    container.addSubview(app.view());
    app.run(Root::of::<NshHostScreen>());
    MOUNTED.with(|slot| *slot.borrow_mut() = Some(app));
    true
}

#[cfg(target_os = "ios")]
thread_local! {
    /// The appearance last handed to the NSH view, so an unchanged one is not sent again.
    static APPLIED: std::cell::RefCell<Option<(String, Option<bool>)>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // Declaring the host's symbol is unsafe; the host owns its definition.
unsafe extern "C" {
    /// Paints the cover around the screens and forces a scheme (`host/App.swift`).
    fn sjel_ios_nsh_cover(style: i32, light: u32, dark: u32);
}

/// Hands the appearance settings to the NSH screens: the palette's tokens as theme overrides,
/// and light or dark when the settings force one (`sjel_gpui_ui::theme::nsh`).
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
pub fn apply_appearance(settings: &sjel_gpui_ui::AppearanceSettings) {
    use sjel_gpui_ui::theme::{forced_dark, nsh_background, nsh_theme_overrides};
    let next = (nsh_theme_overrides(settings), forced_dark(settings));
    if APPLIED.with(|last| last.borrow().as_ref() == Some(&next)) {
        return;
    }
    let mounted = MOUNTED.with(|slot| {
        let slot = slot.borrow();
        let Some(app) = slot.as_ref() else {
            return false;
        };
        let _ = app.set_theme_overrides(Some(&next.0));
        true
    });
    if !mounted {
        return;
    }
    let style = match next.1 {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    };
    // SAFETY: plain integers; the host paints on the main queue.
    unsafe {
        sjel_ios_nsh_cover(
            style,
            nsh_background(settings, false),
            nsh_background(settings, true),
        )
    };
    APPLIED.with(|last| *last.borrow_mut() = Some(next));
}
