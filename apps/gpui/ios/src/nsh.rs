//! Mounts the phone's NSH screens inside the UIKit host.

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
        match target {
            "today" => crate::host::announce(crate::Event::SelectTab(crate::Tab::Today)),
            "calendar" => crate::host::announce(crate::Event::SelectTab(crate::Tab::Calendar)),
            "travel" => crate::host::announce(crate::Event::SelectTab(crate::Tab::Travel)),
            "agent" => sjel_gpui_ui::nsh_edge::open(crate::Seat::Agent),
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
    let appearance = sjel_gpui_ui::AppearanceSettings::load();
    let _ = app.set_theme_overrides(Some(&sjel_gpui_ui::theme::nsh_theme_overrides(&appearance)));
    app.set_font_scale(Some(f32::from(appearance.ui_font_size) / 16.));
    apply_appearance(&appearance);
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
static APPEARANCE: std::sync::Mutex<Option<(String, Option<bool>, f32)>> =
    std::sync::Mutex::new(None);

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
unsafe extern "C" {
    fn sjel_ios_nsh_cover(style: i32, light: u32, dark: u32);
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
pub fn apply_appearance(settings: &sjel_gpui_ui::AppearanceSettings) {
    use sjel_gpui_ui::theme::{forced_dark, nsh_background, nsh_theme_overrides};
    let next = (
        nsh_theme_overrides(settings),
        forced_dark(settings),
        f32::from(settings.ui_font_size) / 16.,
    );
    let mut last = APPEARANCE.lock().expect("appearance lock");
    if last.as_ref() == Some(&next) {
        return;
    }
    let style = match next.1 {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    };
    *last = Some(next);
    drop(last);
    // SAFETY: the host copies these values and schedules the update on the main queue.
    unsafe {
        sjel_ios_nsh_cover(
            style,
            nsh_background(settings, false),
            nsh_background(settings, true),
        )
    };
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_nsh_apply_theme() {
    let Some(_) = objc2::MainThreadMarker::new() else {
        return;
    };
    let last = APPEARANCE.lock().expect("appearance lock");
    if let Some((theme, _, scale)) = last.as_ref() {
        MOUNTED.with(|slot| {
            if let Some(app) = slot.borrow().as_ref() {
                let _ = app.set_theme_overrides(Some(theme));
                app.set_font_scale(Some(*scale));
            }
        });
    }
}
