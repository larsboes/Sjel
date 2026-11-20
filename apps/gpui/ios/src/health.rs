//! Apple Health on the phone → the Mac's `health` capability. `host/Health.swift` reads HealthKit;
//! this crate only asks for a read, posts the answer with the paired device's signature, and tells
//! Swift how far the Mac now holds.

use serde::Deserialize;

#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // Declaring the host's symbols is unsafe; the host owns their definitions.
unsafe extern "C" {
    fn sjel_ios_health_collect(manual: bool);
    fn sjel_ios_health_finished(through: *const std::ffi::c_char, error: *const std::ffi::c_char);
}

/// The route the upload goes to, through the Mac's status proxy.
pub(crate) fn route() -> String {
    format!("{}{}", health_api::PROXY_PREFIX, health_api::ROUTE_DAYS)
}

/// Ask the host for one HealthKit read. Swift throttles and answers through
/// [`sjel_ios_health_upload`] when there is something to send.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
pub(crate) fn collect(manual: bool) {
    // SAFETY: Swift takes a C bool and queues the read on its main thread.
    unsafe { sjel_ios_health_collect(manual) };
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn collect(_manual: bool) {}

#[derive(Default)]
pub(crate) struct State {
    pub status: String,
    pub last_synced: Option<String>,
    pub busy: bool,
}

/// Updates Settings using display-only status, without passing HealthKit values.
///
/// # Safety
/// `status` must point to a NUL-terminated UTF-8 string; `last_synced` is either null or
/// another such string. Both buffers must stay alive for the call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_health_state(
    status: *const std::ffi::c_char,
    last_synced: *const std::ffi::c_char,
    busy: bool,
) {
    // SAFETY: Both pointers follow the contract; Swift keeps them live with `withCString`.
    let (status, last_synced) = unsafe {
        let status = std::ffi::CStr::from_ptr(status)
            .to_string_lossy()
            .into_owned();
        let last_synced = (!last_synced.is_null()).then(|| {
            std::ffi::CStr::from_ptr(last_synced)
                .to_string_lossy()
                .into_owned()
        });
        (status, last_synced)
    };
    crate::announce(crate::Event::HealthState(State {
        status,
        last_synced,
        busy,
    }));
}

/// The upload's outcome: stored through `through`, or `error`. Swift keeps it and, on a failure,
/// tries again at the next foreground.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
pub(crate) fn finished(outcome: Result<Option<String>, String>) {
    let (through, error) = match outcome {
        Ok(through) => (through, None),
        Err(error) => (None, Some(error)),
    };
    let through = through.and_then(|t| std::ffi::CString::new(t).ok());
    let error = error.and_then(|e| std::ffi::CString::new(e.replace('\0', " ")).ok());
    let ptr = |c: &Option<std::ffi::CString>| c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
    // SAFETY: both pointers are null or NUL-terminated strings alive for the call; Swift copies them.
    unsafe { sjel_ios_health_finished(ptr(&through), ptr(&error)) };
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn finished(_outcome: Result<Option<String>, String>) {}

/// What the Mac answers to a stored upload.
#[derive(Deserialize)]
pub(crate) struct Ack {
    stored: u64,
}

impl Ack {
    pub(crate) fn confirm(self, through: Option<String>) -> Result<Option<String>, String> {
        if self.stored > 0 {
            Ok(through)
        } else {
            Err("The Mac did not store any Health values from this upload.".into())
        }
    }
}

/// The newest day an upload body carries, which the host saves once the Mac stored it.
pub(crate) fn through(body: &str) -> Option<String> {
    let upload: health_api::DaysUpload = serde_json::from_str(body).ok()?;
    upload.values.iter().map(|value| value.day.clone()).max()
}

/// Called by `host/Health.swift` with one read, the `POST /api/days` body.
///
/// # Safety
///
/// `json` must be a NUL-terminated UTF-8 string valid for this call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_health_upload(json: *const std::ffi::c_char) {
    if json.is_null() {
        return;
    }
    // SAFETY: Swift passes `withCString`, keeping the NUL-terminated string alive for this call.
    let Ok(body) = unsafe { std::ffi::CStr::from_ptr(json) }.to_str() else {
        return;
    };
    crate::announce(crate::Event::HealthUpload(body.to_owned()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_upload_goes_through_the_proxy_and_reports_its_newest_day() {
        assert_eq!(route(), "/health/api/days");
        let body = r#"{"source":"HealthKit","from":"2026-11-17","values":[
            {"metric":"steps","day":"2026-11-19","value":10.0},
            {"metric":"steps","day":"2026-11-17","value":20.0}],"sleep":[]}"#;
        assert_eq!(through(body).as_deref(), Some("2026-11-19"));
        assert_eq!(through("not json"), None);
        let ack: Ack = serde_json::from_str(r#"{"stored":3}"#).unwrap();
        assert_eq!(
            ack.confirm(through(body)).unwrap().as_deref(),
            Some("2026-11-19")
        );
        assert!((Ack { stored: 0 }).confirm(through(body)).is_err());
        assert_eq!((Ack { stored: 1 }).confirm(None).unwrap(), None);
    }
}
