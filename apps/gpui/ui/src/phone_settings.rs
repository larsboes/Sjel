//! The Swift host's phone settings state and actions.

use std::sync::{Mutex, OnceLock};

use tokio::sync::watch;

/// One row of mutually exclusive options, such as the theme mode.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    /// The key the shell applies a pick by.
    pub id: &'static str,
    pub title: &'static str,
    /// Each option's label and whether it is the current one.
    pub options: Vec<(String, bool)>,
}

/// A Mac this phone found on the network.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoundMac {
    pub name: String,
    pub address: String,
}

/// What the screen draws. The shell rebuilds it after every change; equal states are not resent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsState {
    pub status: String,
    pub synced: Option<String>,
    /// True once a capability answered, so the pairing card offers to pair again.
    pub paired: bool,
    /// True from a Scan or Join press until the owner decides.
    pub joining: bool,
    pub found: Vec<FoundMac>,
    /// The paired Mac's capabilities, and whether each one answered.
    pub capabilities: Vec<(String, bool)>,
    pub room_status: String,
    /// True when a Mac is in reach, so pending scans can be sent.
    pub room_can_sync: bool,
    pub health_status: String,
    pub health_last_synced: Option<String>,
    pub health_can_sync: bool,
    pub reminders_on: bool,
    pub reminders_sentence: String,
    /// iOS refused notifications, so the way back is the system's own page.
    pub reminders_denied: bool,
    /// Dates past the number iOS holds, said rather than hidden.
    pub reminders_dropped: Option<String>,
    /// "How early", shown while reminders are on.
    pub lead: Option<Choice>,
    pub appearance: Vec<Choice>,
    pub terminal: Vec<Choice>,
}

/// A press the shell answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    /// Back to Today.
    Back,
    /// Present the camera for the Mac's pairing code.
    Scan,
    /// Ask the found Mac at this index to admit the phone.
    Join(usize),
    CaptureRoom,
    SyncRoomScans,
    SyncHealth,
    ToggleReminders,
    OpenSystemSettings,
    /// Pick option `option` of the choice with this id.
    Choose {
        id: &'static str,
        option: usize,
    },
}

type Host = Box<dyn Fn(SettingsAction) + Send>;

struct Feed {
    state: watch::Sender<SettingsState>,
    host: Mutex<Option<Host>>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| Feed {
        state: watch::channel(SettingsState::default()).0,
        host: Mutex::new(None),
    })
}

/// What the shell does with a press.
pub fn set_host(host: impl Fn(SettingsAction) + Send + 'static) {
    *feed().host.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(host));
}

/// Hands the screen the shell's current state. A state equal to the last one is dropped.
pub fn set_state(next: SettingsState) {
    feed().state.send_if_modified(|current| {
        if *current == next {
            return false;
        }
        *current = next;
        true
    });
}

pub(super) fn send(action: SettingsAction) {
    if let Some(host) = feed()
        .host
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        host(action);
    }
}

pub(super) fn snapshots() -> &'static watch::Sender<SettingsState> {
    &feed().state
}

#[cfg(target_os = "ios")]
mod view;
#[cfg(target_os = "ios")]
pub use view::PhoneSettings;
