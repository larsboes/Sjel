//! Signed Documents reads and the native PDF action for the phone.

use std::{
    ffi::{c_char, CString},
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use gpui_kit::Context;
use sjel_gpui_core::device::{DeviceClient, Source};
use sjel_gpui_ui::{documents::PhoneDocuments, phone_navigation::back as navigate_back};
use tokio::sync::watch;

fn source() -> &'static watch::Sender<Option<Source>> {
    static SOURCE: OnceLock<watch::Sender<Option<Source>>> = OnceLock::new();
    SOURCE.get_or_init(|| watch::channel(None).0)
}

pub(crate) fn connect(client: Arc<DeviceClient>) {
    source().send_replace(Some(Source::Device(client)));
}

#[allow(unsafe_code)]
unsafe extern "C" {
    fn sjel_ios_documents_pdf(path: *const c_char);
}

fn show_pdf(path: PathBuf) {
    if let Ok(path) = CString::new(path.as_os_str().as_encoded_bytes()) {
        // SAFETY: Swift copies the NUL-terminated path during the call.
        #[allow(unsafe_code)]
        unsafe {
            sjel_ios_documents_pdf(path.as_ptr())
        };
    }
}

pub(crate) fn view(cx: &mut Context<PhoneDocuments>) -> PhoneDocuments {
    PhoneDocuments::new(
        source().subscribe(),
        crate::phone_system::originals(),
        show_pdf,
        crate::gpui_host::keyboard,
        navigate_back,
        cx,
    )
}
