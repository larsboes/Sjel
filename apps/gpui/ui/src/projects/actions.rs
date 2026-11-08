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
        /// Move selection to the next item (project or task).
        SelectNext,
        /// Move selection to the previous item (project or task).
        SelectPrev,
        /// Open the currently selected project or task.
        ActivateSelected,
        /// Toggle completion of the selected task.
        ToggleSelected,
        /// Edit the title of the selected task in-place.
        EditTaskTitle,
        /// Advance the selected task to the next Kanban column.
        MoveTaskNextStage,
        /// Move the selected task to the previous Kanban column.
        MoveTaskPrevStage,
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
        KeyBinding::new("j", SelectNext, Some("Projects")),
        KeyBinding::new("down", SelectNext, Some("Projects")),
        KeyBinding::new("k", SelectPrev, Some("Projects")),
        KeyBinding::new("up", SelectPrev, Some("Projects")),
        KeyBinding::new("enter", ActivateSelected, Some("Projects")),
        KeyBinding::new("l", ActivateSelected, Some("Projects")),
        KeyBinding::new("space", ToggleSelected, Some("Projects")),
        KeyBinding::new("e", EditTaskTitle, Some("Projects")),
        KeyBinding::new("]", MoveTaskNextStage, Some("Projects")),
        KeyBinding::new("[", MoveTaskPrevStage, Some("Projects")),
    ]);
}
