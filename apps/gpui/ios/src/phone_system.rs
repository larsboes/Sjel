//! Native phone appearance and protected caches.

/// Makes the protected mail cache available before the paired connection starts.
pub(crate) fn prepare() {
    sjel_gpui_ui::phone_mail::set_close(sjel_gpui_ui::phone_navigation::back);
    if let Some(offline) = mail_offline() {
        sjel_gpui_ui::phone_mail::set_offline(offline);
    }
    apply_appearance(&sjel_gpui_ui::AppearanceSettings::load());
}

static APPEARANCE: std::sync::Mutex<Option<sjel_gpui_ui::AppearanceSettings>> =
    std::sync::Mutex::new(None);

#[allow(unsafe_code)]
unsafe extern "C" {
    fn sjel_ios_phone_cover(style: i32, light: u32, dark: u32);
}

pub(crate) fn apply_appearance(settings: &sjel_gpui_ui::AppearanceSettings) {
    use sjel_gpui_ui::theme::{forced_dark, nsh_background};
    let mut current = APPEARANCE.lock().expect("appearance lock");
    if current.as_ref() == Some(settings) {
        return;
    }
    *current = Some(*settings);
    drop(current);
    let style = match forced_dark(settings) {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    };
    // SAFETY: the host copies these values and schedules the update on the main queue.
    #[allow(unsafe_code)]
    unsafe {
        sjel_ios_phone_cover(
            style,
            nsh_background(settings, false),
            nsh_background(settings, true),
        )
    };
}

pub(crate) fn appearance() -> sjel_gpui_ui::AppearanceSettings {
    APPEARANCE
        .lock()
        .expect("appearance lock")
        .unwrap_or_default()
}

#[allow(unsafe_code)]
unsafe extern "C" {
    fn sjel_ios_protected_folder(path: *const std::ffi::c_char) -> bool;
}

fn protected_folder(name: &str) -> Option<std::path::PathBuf> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
    let folder = home.join("Library/Application Support").join(name);
    let path = std::ffi::CString::new(folder.as_os_str().as_encoded_bytes()).ok()?;
    // SAFETY: path is a NUL-terminated string kept alive throughout the host call.
    #[allow(unsafe_code)]
    unsafe { sjel_ios_protected_folder(path.as_ptr()) }.then_some(folder)
}

pub(crate) fn originals() -> Option<sjel_gpui_core::originals::Originals> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?);
    let cache = home.join("Library/Caches/originals");
    let kept = protected_folder("kept-originals")?;
    Some(sjel_gpui_core::originals::Originals::new(cache, kept))
}

fn mail_offline() -> Option<sjel_gpui_core::mail::MailOffline> {
    protected_folder("kept-mail").map(sjel_gpui_core::mail::MailOffline::new)
}
