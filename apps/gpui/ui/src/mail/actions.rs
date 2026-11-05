//! Mail triage actions and keyboard bindings.
use gpui_kit::{actions, App, KeyBinding};

actions!(
    mail,
    [
        /// Move to the next thread.
        Next,
        /// Move to the previous thread.
        Previous,
        /// Select or unselect the thread.
        ToggleSelect,
        /// Dismiss from Sjel. Gmail is not touched.
        Dismiss,
        /// Archive in Sjel and Gmail, after confirming.
        Archive,
        /// Move to Gmail Trash, after confirming.
        Trash,
        /// Add or remove Gmail's Waiting label.
        ToggleWaiting,
        /// Open the thread in Gmail.
        OpenInGmail,
        /// Confirm the action that asked.
        Confirm,
        /// Cancel the question, else clear the selection.
        Cancel,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("j", Next, Some("MailList")),
        KeyBinding::new("down", Next, Some("MailList")),
        KeyBinding::new("k", Previous, Some("MailList")),
        KeyBinding::new("up", Previous, Some("MailList")),
        KeyBinding::new("x", ToggleSelect, Some("MailList")),
        KeyBinding::new("d", Dismiss, Some("MailList")),
        KeyBinding::new("e", Archive, Some("MailList")),
        KeyBinding::new("#", Trash, Some("MailList")),
        KeyBinding::new("shift-3", Trash, Some("MailList")),
        KeyBinding::new("w", ToggleWaiting, Some("MailList")),
        KeyBinding::new("o", OpenInGmail, Some("MailList")),
        KeyBinding::new("enter", Confirm, Some("MailList")),
        KeyBinding::new("escape", Cancel, Some("MailList")),
    ]);
}
