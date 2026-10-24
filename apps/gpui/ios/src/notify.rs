//! The phone's lock screen, from this side of the bridge.
//!
//! The rule that decides what is worth announcing is platform-free
//! ([`sjel_gpui_core::notify`]); the notification centre itself is Apple's, and is the host's
//! (`host/Notifications.swift`) for the same reason the audio device is: `UNUserNotificationCenter`
//! is Swift's own API, and the permission prompt it can show is a screen GPUI does not paint.
//!
//! The contract is one list in each direction and no shared state: this side hands over the whole
//! set of requests to hold ([`apply`]), and the host answers with what the system actually holds
//! ([`State`]) — the count comes from `getPendingNotificationRequests`, never from this side, so a
//! line that says "4 reminders" is the system's answer and not this app's optimism.
//!
//! Three calls are the host's to make, because they start in its process: a tap on a delivered
//! reminder, and the state after any of the three asks above. Both arrive through a C entry point
//! and become events for the view, the way a Bonjour answer does.

// The macOS build of this crate is a test target: every function here is called from an iOS
// launch, where the host provides the other half of the call.
#![cfg_attr(not(target_os = "ios"), allow(dead_code))]

use std::sync::Mutex;

use sjel_gpui_core::notify::{Plan, Reminder};

/// The list last handed to the host, so a rebuild that changes nothing does not ask the notification
/// centre to replace sixty requests it already holds. A fresh process starts empty, so the first
/// rebuild after a launch always goes out.
static LAST: Mutex<Vec<Reminder>> = Mutex::new(Vec::new());

/// Whether the system will deliver what is asked for.
///
/// `NotAsked` is not a failure: iOS allows the prompt once, and this app asks only when the person
/// turns reminders on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Authorization {
    #[default]
    NotAsked,
    Allowed,
    Denied,
}

impl Authorization {
    pub fn from_key(key: &str) -> Self {
        match key {
            "allowed" => Self::Allowed,
            "denied" => Self::Denied,
            _ => Self::NotAsked,
        }
    }

    /// The words under the switch. A refusal says where to change it, because the app cannot ask
    /// again: the second prompt iOS would show does not exist.
    pub fn phrase(self) -> &'static str {
        match self {
            Self::NotAsked => "Not asked yet",
            Self::Allowed => "Allowed",
            Self::Denied => "Off in iOS Settings › Sjel › Notifications",
        }
    }
}

/// What the system holds, as the host counted it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub authorization: Authorization,
    /// Requests pending right now, counted by `UNUserNotificationCenter`.
    pub scheduled: usize,
    /// The next one: its moment and its title, for the line under the switch.
    pub next: Option<(i64, String)>,
}

impl State {
    /// The host's JSON answer, or `None` for a reply this build cannot read.
    pub fn from_json(text: &str) -> Option<Self> {
        #[derive(serde::Deserialize)]
        struct Reply {
            authorization: String,
            #[serde(default)]
            scheduled: usize,
            #[serde(default)]
            next: Option<Next>,
        }
        #[derive(serde::Deserialize)]
        struct Next {
            at: i64,
            title: String,
        }
        let reply: Reply = serde_json::from_str(text).ok()?;
        Some(Self {
            authorization: Authorization::from_key(&reply.authorization),
            scheduled: reply.scheduled,
            next: reply.next.map(|next| (next.at, next.title)),
        })
    }
}

/// `SJEL_GPUI_NO_NOTIFICATIONS=1` keeps the app away from the notification centre entirely: a
/// screenshot run has no business asking for a permission, and a check that says "this build cannot
/// remind you" is better than a silent one.
const NO_NOTIFICATIONS: &str = "SJEL_GPUI_NO_NOTIFICATIONS";

/// True when this launch may touch the notification centre.
pub fn allowed() -> bool {
    std::env::var_os(NO_NOTIFICATIONS).is_none()
}

/// Why reminders are off, when the switch above is the reason.
pub fn disabled_reason() -> Option<String> {
    (!allowed()).then(|| format!("{NO_NOTIFICATIONS} is set: no notifications"))
}

/// `SJEL_NOTIFY=1` turns reminders on at launch and asks for the permission. A phone never sets it;
/// `ios/run-sim` does, because a card a hand did not open still has to be photographable.
pub fn launch_enabled() -> bool {
    std::env::var("SJEL_NOTIFY")
        .map(|value| value != "0")
        .unwrap_or(false)
}

/// Asks the host for is permission. The answer is not returned here: it arrives as a
/// [`State`] once the person has answered the system prompt, which may be never.
pub fn authorize() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)] // One call into the host, which links this symbol by name.
    // SAFETY: it takes no arguments, returns nothing, and keeps nothing from this call
    // (`host/Notifications.swift`).
    unsafe {
        sjel_ios_notifications_authorize();
    }
}

/// Hands the host the whole set of requests to hold, replacing what it had.
///
/// The identifiers are stable ([`sjel_gpui_core::notify::plan`]), so a request already pending with
/// the same id is replaced rather than duplicated; the host cancels those the list no longer names.
pub fn apply(plan: &Plan) {
    if let Ok(mut last) = LAST.lock() {
        if *last == plan.reminders {
            return;
        }
        *last = plan.reminders.clone();
    }
    #[cfg(target_os = "ios")]
    if let Ok(json) = serde_json::to_string(&Body {
        reminders: &plan.reminders,
    }) {
        #[allow(unsafe_code)] // One call into the host, which links this symbol by name.
        // SAFETY: the buffer outlives the call, which is the host's contract (a `withCString`
        // argument piped into `String(cString:)`); nothing here is kept.
        unsafe {
            let text = std::ffi::CString::new(json).unwrap_or_default();
            sjel_ios_notifications_schedule(text.as_ptr());
        }
    }
    #[cfg(not(target_os = "ios"))]
    let _ = plan;
}

/// Takes every request away: what turning reminders off means.
pub fn clear() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)] // One call into the host, which links this symbol by name.
    // SAFETY: it takes no arguments and keeps nothing from this call.
    unsafe {
        sjel_ios_notifications_clear();
    }
}

/// Asks the host what it holds, so the card can show the system's answer rather than a memory of
/// what was last asked for. The answer arrives as [`crate::Event::NotifyState`].
pub fn report() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)] // One call into the host, which links this symbol by name.
    // SAFETY: it takes no arguments and keeps nothing from this call.
    unsafe {
        sjel_ios_notifications_report();
    }
}

/// Tells the host that this side's view exists, so a reminder tapped before the window was built
/// can be delivered now instead of dropped, and asks what the system already holds.
///
/// Called once, when the view is constructed: a tap launches the app, so the response arrives
/// while the window is still being made.
pub fn ready() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)] // One call into the host, which links this symbol by name.
    // SAFETY: it takes no arguments and keeps nothing from this call.
    unsafe {
        sjel_ios_notifications_ready();
    }
    report();
}

/// Opens this app's page in iOS Settings, which is the only way back from a denied permission:
/// the system allows the prompt once, and the second ask is a person's trip through Settings.
pub fn open_settings() {
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)] // One call into the host, which links this symbol by name.
    // SAFETY: it takes no arguments and keeps nothing from this call.
    unsafe {
        sjel_ios_open_settings();
    }
}

/// The body the host decodes: one list, which is the whole of what it should hold.
#[derive(serde::Serialize)]
struct Body<'a> {
    reminders: &'a [Reminder],
}

#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // Declaring the host's symbols is unsafe; the host owns their definitions.
unsafe extern "C" {
    fn sjel_ios_notifications_authorize();
    fn sjel_ios_notifications_schedule(json: *const std::ffi::c_char);
    fn sjel_ios_notifications_clear();
    fn sjel_ios_notifications_report();
    fn sjel_ios_notifications_ready();
    fn sjel_ios_open_settings();
}

/// Called by `host/Notifications.swift` with `{"authorization":"allowed","scheduled":3,"next":…}`
/// after any ask above, and once at launch.
///
/// # Safety
///
/// `json` must be a NUL-terminated UTF-8 string that stays valid for the call, which is what
/// `withCString` gives the Swift caller.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_notifications_state(json: *const std::ffi::c_char) {
    if json.is_null() {
        return;
    }
    // SAFETY: the header in `host/Embedding.h` is the caller's contract: a NUL-terminated string
    // that outlives the call, read here and not kept.
    let Ok(text) = unsafe { std::ffi::CStr::from_ptr(json) }.to_str() else {
        return;
    };
    if let Some(state) = State::from_json(text) {
        crate::announce(crate::Event::NotifyState(state));
    }
}

/// Called by `host/Notifications.swift` when the person taps a delivered reminder.
///
/// # Safety
///
/// `json` must be a NUL-terminated UTF-8 string that stays valid for the call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // `no_mangle` is an unsafe attribute; the host links this symbol by name.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_notification_opened(json: *const std::ffi::c_char) {
    if json.is_null() {
        return;
    }
    // SAFETY: as above: a NUL-terminated string owned by the caller for the call's duration.
    let Ok(text) = unsafe { std::ffi::CStr::from_ptr(json) }.to_str() else {
        return;
    };
    #[derive(serde::Deserialize)]
    struct Opened {
        #[serde(default)]
        link: String,
    }
    if let Ok(opened) = serde_json::from_str::<Opened>(text) {
        if !opened.link.is_empty() {
            crate::announce(crate::Event::NotifyOpen(opened.link));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host's answer is read as the system gave it, and an authorization this build does not
    /// know is "not asked" rather than a permission this app assumes it has.
    #[test]
    fn the_hosts_answer_reads_as_the_system_gave_it() {
        let state = State::from_json(
            r#"{"authorization":"allowed","scheduled":3,"next":{"at":1792828200,"title":"Zahnarzt"}}"#,
        )
        .expect("read");
        assert_eq!(state.authorization, Authorization::Allowed);
        assert_eq!(state.scheduled, 3);
        assert_eq!(state.next, Some((1792828200, "Zahnarzt".to_owned())));

        let empty = State::from_json(r#"{"authorization":"denied"}"#).expect("read");
        assert_eq!(empty.authorization, Authorization::Denied);
        assert_eq!(empty.scheduled, 0);
        assert_eq!(empty.next, None);

        assert_eq!(
            State::from_json(r#"{"authorization":"provisional"}"#)
                .expect("read")
                .authorization,
            Authorization::NotAsked,
            "an answer this build cannot place is not a yes"
        );
        assert!(State::from_json("not json").is_none());
        assert!(State::from_json("").is_none());
    }
}
