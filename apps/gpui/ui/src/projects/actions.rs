//! Keyboard actions and keybindings for Projects.

use gpui_kit::{actions, App, KeyBinding};

actions!(
    projects,
    [
        /// Go back or close the active note, task panel, or project.
        CloseProject,
        /// Switch to section 1 (Project Note) or tab 1 (Projects hub).
        SelectTab1,
        /// Switch to section 2 (Tasks) or tab 2 (All Tasks).
        SelectTab2,
        /// Switch to section 3 (Notes) or tab 3 (All Notes).
        SelectTab3,
        /// Switch to section 4 (Mail review).
        SelectTab4,
        /// Switch to section 5 (Inventory links).
        SelectTab5,
        /// Toggle between Table and Board views in Tasks.
        ToggleTaskViewMode,
        /// Cycle task filter (Open -> All -> Done).
        CycleTaskFilter,
        /// Focus the new task, note, or project input.
        NewItem,
        /// Focus the search field.
        FocusSearch,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseProject, Some("Projects")),
        KeyBinding::new("cmd-[", CloseProject, Some("Projects")),
        KeyBinding::new("1", SelectTab1, Some("Projects")),
        KeyBinding::new("2", SelectTab2, Some("Projects")),
        KeyBinding::new("3", SelectTab3, Some("Projects")),
        KeyBinding::new("4", SelectTab4, Some("Projects")),
        KeyBinding::new("5", SelectTab5, Some("Projects")),
        KeyBinding::new("cmd-1", SelectTab1, Some("Projects")),
        KeyBinding::new("cmd-2", SelectTab2, Some("Projects")),
        KeyBinding::new("cmd-3", SelectTab3, Some("Projects")),
        KeyBinding::new("cmd-4", SelectTab4, Some("Projects")),
        KeyBinding::new("cmd-5", SelectTab5, Some("Projects")),
        KeyBinding::new("b", ToggleTaskViewMode, Some("Projects")),
        KeyBinding::new("t", ToggleTaskViewMode, Some("Projects")),
        KeyBinding::new("f", CycleTaskFilter, Some("Projects")),
        KeyBinding::new("n", NewItem, Some("Projects")),
        KeyBinding::new("/", FocusSearch, Some("Projects")),
    ]);
}
