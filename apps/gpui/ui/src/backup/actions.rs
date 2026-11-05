//! Actions, keybindings, and action constants for Backup views.
use gpui_kit::{actions, App, KeyBinding};

/// The `Busy` key the button marks while its requests are in flight.
pub const BACK_UP_ALL: &str = "backup:all";

actions!(
    backup,
    [ShowContracts, ShowTargets, ShowHistory, RefreshBackup,]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("1", ShowContracts, Some("BackupView")),
        KeyBinding::new("2", ShowTargets, Some("BackupView")),
        KeyBinding::new("3", ShowHistory, Some("BackupView")),
    ]);
}
