//! The boundary with the Swift host (`host/`): the calls this crate makes into it, and the
//! entry points it calls back. Each answer arrives as an [`Event`] through [`announce`].

use super::*;

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
unsafe extern "C" {
    fn sjel_ios_mark_roomplan_uploaded(text: *const std::ffi::c_char);
    fn sjel_ios_preview_roomplan(text: *const std::ffi::c_char);
    fn sjel_ios_cache_preview_roomplan(
        draft_id: *const std::ffi::c_char,
        draft_json: *const std::ffi::c_char,
        bytes: *const u8,
        length: usize,
    );
}

/// What one browse pass reported: the nodes, and whether iOS refused local-network access.
#[cfg(target_os = "ios")]
#[derive(Deserialize)]
pub(crate) struct Browse {
    nodes: Vec<Found>,
    denied: bool,
}

/// The view's event sender, so the C entry point below can reach the view it feeds.
pub(crate) static EVENTS: OnceLock<Mutex<Option<UnboundedSender<Event>>>> = OnceLock::new();

// Opens the camera (`host/PairScanner.swift`). The screen is Swift's because a camera preview is an
// `AVCaptureVideoPreviewLayer`, and GPUI draws its own pixels; the answer comes back through
// `sjel_ios_scanned_pairing`.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // Declaring the host's symbol is unsafe; the host owns its definition.
unsafe extern "C" {
    fn sjel_ios_present_pair_scanner();
    fn sjel_ios_present_roomplan();
    fn sjel_ios_import_legacy_roomplan();
    fn sjel_ios_load_roomplan_status();
    fn sjel_ios_load_roomplan_upload();
    fn sjel_ios_load_roomplan_archive();
    // Face ID before a terminal on the Mac opens; the answer is `sjel_ios_unlocked`.
    fn sjel_ios_unlock();
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
pub(crate) fn unlock() {
    // SAFETY: no arguments; the host answers on the main queue through `sjel_ios_unlocked`.
    unsafe { sjel_ios_unlock() };
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn unlock() {}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
pub(crate) fn preview_roomplan(draft_id: &str) {
    if let Ok(text) = std::ffi::CString::new(draft_id) {
        // SAFETY: the Swift host copies this validated archive ID during the call.
        unsafe { sjel_ios_preview_roomplan(text.as_ptr()) };
    }
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
pub(crate) fn cache_preview_roomplan(draft_id: &str, draft_json: &str, bytes: Vec<u8>) {
    let (Ok(draft_id), Ok(draft_json)) = (
        std::ffi::CString::new(draft_id),
        std::ffi::CString::new(draft_json),
    ) else {
        return;
    };
    // SAFETY: Swift copies both strings and the byte slice before returning from this callback.
    unsafe {
        sjel_ios_cache_preview_roomplan(
            draft_id.as_ptr(),
            draft_json.as_ptr(),
            bytes.as_ptr(),
            bytes.len(),
        );
    }
}

/// Asks the host to open the scanner. Nothing happens on a build without a camera screen, which
/// is every non-iOS target of this crate.
pub(crate) fn present_pair_scanner() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)] // One call into the host, which links this symbol by name.
    // SAFETY: it takes no arguments and keeps nothing from this call (`host/PairScanner.swift`).
    unsafe {
        sjel_ios_present_pair_scanner();
    }
}

pub(crate) fn present_roomplan() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)]
    // SAFETY: the host function takes no arguments and owns the presented capture controller.
    unsafe {
        sjel_ios_present_roomplan();
    }
}

#[cfg(target_os = "ios")]
pub(crate) fn import_legacy_roomplan() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)]
    // SAFETY: the host presents its local Files picker and retains the import coordinator.
    unsafe {
        sjel_ios_import_legacy_roomplan();
    }
}

pub(crate) fn load_roomplan_status() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)]
    // SAFETY: the host reads its own app-support files and returns through the registered callback.
    unsafe {
        sjel_ios_load_roomplan_status();
    }
}

pub(crate) fn load_roomplan_upload() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)]
    // SAFETY: the host reads its own app-support files and returns the upload body through FFI.
    unsafe {
        sjel_ios_load_roomplan_upload();
    }
}

pub(crate) fn load_roomplan_archive() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)]
    // SAFETY: the host reads its app-support archive and returns JSON through the registered callback.
    unsafe {
        sjel_ios_load_roomplan_archive();
    }
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
pub(crate) fn mark_roomplan_uploaded(draft_id: &str) {
    if let Ok(text) = std::ffi::CString::new(draft_id) {
        // SAFETY: the host callback copies the NUL-terminated ID during this call.
        unsafe { sjel_ios_mark_roomplan_uploaded(text.as_ptr()) };
    }
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn mark_roomplan_uploaded(_draft_id: &str) {}

/// Sends one event to the live view, if there is one. Called from the main queue.
pub(crate) fn announce(event: Event) {
    let sender = EVENTS.get().and_then(|slot| slot.lock().ok()?.clone());
    if let Some(sender) = sender {
        let _ = sender.unbounded_send(event);
    }
}

pub(crate) fn keyboard(open: bool) {
    #[cfg(target_os = "ios")]
    if open {
        gpui_mobile::show_keyboard();
    } else {
        gpui_mobile::hide_keyboard();
    }
    #[cfg(not(target_os = "ios"))]
    let _ = open;
}

/// The software keyboard's height, so the open note's caret stays above it.
pub(crate) fn keyboard_inset() -> f32 {
    #[cfg(target_os = "ios")]
    return gpui_mobile::keyboard_height();
    #[cfg(not(target_os = "ios"))]
    0.0
}

/// Called by `host/Discovery.swift` with one browse answer, `{"nodes":[…],"denied":false}`.
///
/// The Swift side browses `_sjel._tcp` and passes every TXT record it can use; this entry point
/// only carries them to the view, because a browse answer arrives on the main queue and the
/// view lives on GPUI's foreground executor.
///
/// # Safety
///
/// `json` must be a NUL-terminated UTF-8 string that stays valid for the call, which is what
/// `withCString` gives the Swift caller.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_nodes(json: *const std::ffi::c_char) {
    if json.is_null() {
        return;
    }
    // SAFETY: the header in `host/Embedding.h` is the caller's contract: a NUL-terminated string
    // that outlives the call (`withCString`), read here and not kept.
    let Ok(text) = unsafe { std::ffi::CStr::from_ptr(json) }.to_str() else {
        return;
    };
    let Ok(browse) = serde_json::from_str::<Browse>(text) else {
        return;
    };
    if browse.denied {
        announce(Event::Status(LOCAL_NETWORK_DENIED.into()));
        return;
    }
    announce(Event::Found(browse.nodes));
}

/// Called by `host/PairScanner.swift` with the text of a pairing code it read.
///
/// The scanner is offered from this app's own Settings tab, so the view already exists by the
/// time a code can arrive: `EVENTS` is set in `Capabilities::new` before any button is drawn.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 string that stays valid for the call, which is what
/// `withCString` gives the Swift caller.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_scanned_pairing(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: the header in `host/Embedding.h` is the caller's contract: a NUL-terminated string
    // that outlives the call (`withCString`), read here and not kept.
    let Ok(code) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        announce(Event::ScanFailed("That code could not be read.".into()));
        return;
    };
    announce(Event::Scanned(code.trim().to_string()));
}

/// Called by `host/PairScanner.swift` when the camera will not open: a simulator has none, and a
/// person who refused it once is told where to change that instead of seeing a black rectangle.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 string that stays valid for the call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_scan_failed(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: as above: a NUL-terminated string owned by the caller for the call's duration.
    let Ok(reason) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        return;
    };
    announce(Event::ScanFailed(reason.to_string()));
}

/// Called by `host/RoomCapture.swift` with a saved capture summary or a user-facing failure.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 string valid for this call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_roomplan_result(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: Swift passes `withCString`, keeping the NUL-terminated string alive for this call.
    let Ok(result) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        announce(Event::RoomplanResult(
            "Room capture returned unreadable text.".into(),
        ));
        return;
    };
    announce(Event::RoomplanResult(result.to_owned()));
}

/// Called by `host/RoomCapture.swift` with a local, live semantic snapshot and coverage counters.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 JSON string valid for this call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_roomplan_live(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: Swift passes `withCString`, keeping the JSON alive for this call.
    let Ok(json) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        return;
    };
    let Ok(snapshot) = serde_json::from_str::<RoomPlanLiveUpdate>(json) else {
        return;
    };
    announce(Event::RoomplanLive(snapshot));
}

/// Called by `host/RoomCapture.swift` with the locally stored draft and USDZ upload envelope.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 string valid for this call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_roomplan_upload(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: Swift passes `withCString`, keeping the NUL-terminated string alive for this call.
    let Ok(body) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        announce(Event::RoomplanResult(
            "Room capture could not be read for upload.".into(),
        ));
        return;
    };
    announce(Event::RoomplanUpload(body.to_owned()));
}

/// Called by `host/RoomCapture.swift` with the phone's locally archived scan summaries.
///
/// # Safety
///
/// `text` must be a NUL-terminated UTF-8 string valid for this call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_roomplan_archive(text: *const std::ffi::c_char) {
    if text.is_null() {
        return;
    }
    // SAFETY: Swift passes `withCString`, keeping the NUL-terminated string alive for this call.
    let Ok(json) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
        return;
    };
    let Ok(records) = serde_json::from_str::<Vec<RoomPlanLocalRecord>>(json) else {
        return;
    };
    announce(Event::RoomplanArchive(records));
}

/// Called by `host/App.swift` when the scene becomes active: the phone may have been in a pocket
/// through several days, so the reminders are rebuilt from the replica the moment it is looked at.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_became_active() {
    announce(Event::Foreground);
}

/// Called by `host/App.swift` when the scene enters the background. A Face ID sheet only resigns
/// active, so this is the edge that ends a foreground session.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_entered_background() {
    announce(Event::Background);
}

/// Called by `host/App.swift` while a pinch moves, with its scale since it began.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_pinch(scale: f64, ended: bool) {
    announce(Event::Pinch(scale, ended));
}

/// Called by `host/App.swift` with the answer to `sjel_ios_unlock`, on the main queue.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_unlocked(ok: bool) {
    announce(Event::Unlocked(ok));
}

/// Called by `host/App.swift` before `gpui_ios_run_demo`: registers what the GPUI window shows.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_register_app() {
    use gpui::{App, WindowOptions};
    use gpui_kit::component::Root;

    gpui_mobile::ios::ffi::set_app_callback(Box::new(|cx: &mut App| {
        gpui_kit::init(cx);
        let _ = cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(Capabilities::new);
            cx.new(|cx| Root::new(view, window, cx))
        });
    }));
}

/// Called by `host/App.swift` when the app starts with `SJEL_NSH_TRIAL=1`: mounts the NSH trial
/// screen into `container`, a `UIView`. Returns `false` when the build has no `nsh-trial` feature.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_nsh_mount(container: *mut std::ffi::c_void) -> bool {
    #[cfg(feature = "nsh-trial")]
    {
        if container.is_null() {
            return false;
        }
        // SAFETY: the host passes an unretained `UIView` it keeps in its hierarchy for the call.
        let view = unsafe { &*container.cast::<objc2_ui_kit::UIView>() };
        crate::nsh_trial::mount(view)
    }
    #[cfg(not(feature = "nsh-trial"))]
    {
        let _ = container;
        false
    }
}
