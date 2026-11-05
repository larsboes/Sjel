//! Event types for the Inbox quick capture view.

/// What the page asks its host to do. The host owns the sync and the keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InboxEvent {
    /// A capture was written here and is waiting for the Mac, so the host may sync now.
    Wrote,
    /// The field took or lost the keyboard, so the host can show or hide it.
    Editing(bool),
}
