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

/// Where a bare key is a command. A focused text field inside the page takes the key as text.
const LETTERS: &str = "Projects && !Input && !ElyInput";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseProject, Some("Projects")),
        KeyBinding::new("cmd-[", CloseProject, Some("Projects")),
        KeyBinding::new("1", SelectTab1, Some(LETTERS)),
        KeyBinding::new("2", SelectTab2, Some(LETTERS)),
        KeyBinding::new("3", SelectTab3, Some(LETTERS)),
        KeyBinding::new("4", SelectTab4, Some(LETTERS)),
        KeyBinding::new("5", SelectTab5, Some(LETTERS)),
        KeyBinding::new("cmd-1", SelectTab1, Some("Projects")),
        KeyBinding::new("cmd-2", SelectTab2, Some("Projects")),
        KeyBinding::new("cmd-3", SelectTab3, Some("Projects")),
        KeyBinding::new("cmd-4", SelectTab4, Some("Projects")),
        KeyBinding::new("cmd-5", SelectTab5, Some("Projects")),
        KeyBinding::new("b", ToggleTaskViewMode, Some(LETTERS)),
        KeyBinding::new("t", ToggleTaskViewMode, Some(LETTERS)),
        KeyBinding::new("f", CycleTaskFilter, Some(LETTERS)),
        KeyBinding::new("n", NewItem, Some(LETTERS)),
        KeyBinding::new("/", FocusSearch, Some(LETTERS)),
        KeyBinding::new("j", SelectNext, Some(LETTERS)),
        KeyBinding::new("down", SelectNext, Some("Projects")),
        KeyBinding::new("k", SelectPrev, Some(LETTERS)),
        KeyBinding::new("up", SelectPrev, Some("Projects")),
        KeyBinding::new("enter", ActivateSelected, Some("Projects")),
        KeyBinding::new("l", ActivateSelected, Some(LETTERS)),
        KeyBinding::new("space", ToggleSelected, Some(LETTERS)),
        KeyBinding::new("e", EditTaskTitle, Some(LETTERS)),
        KeyBinding::new("]", MoveTaskNextStage, Some(LETTERS)),
        KeyBinding::new("[", MoveTaskPrevStage, Some(LETTERS)),
    ]);
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext, TestAppContext};

    use super::super::*;
    use gpui_kit::KeyBinding;

    gpui::actions!(projects_test, [Outer]);
    use sjel_gpui_core::shell::{ProjectRecord, ShellError};

    fn record() -> ProjectRecord {
        ProjectRecord {
            project: String::new(),
            path: "P/Plan.md".into(),
            kind: "note".into(),
            title: "Plan".into(),
            content: "---\ntype: note\n---\n# Plan\n".into(),
            done: false,
            due: None,
            priority: 2,
        }
    }
    #[gpui::test]
    fn the_note_title_field_takes_letters_the_page_binds(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        cx.update(actions::bind_keys);
        // The workspace's window-wide escape, as `apps/gpui/src/workspace/mod.rs` binds it.
        cx.update(|cx| {
            cx.bind_keys([KeyBinding::new(
                "escape",
                Outer,
                Some("!Terminal && !(Projects > Input)"),
            )]);
            cx.on_action(|_: &Outer, _| {});
        });
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
                let editor = cx.new(|cx| {
                    NoteEditor::new(
                        record(),
                        std::sync::Arc::new(|_| Box::pin(async { Ok(()) })),
                        window,
                        cx,
                    )
                });
                projects.editor = Some(editor);
                projects
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        let projects = slot.take().unwrap();
        cx.run_until_parked();
        cx.update(|window, cx| projects.update(cx, |view, cx| view.edit_title(window, cx)));
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-a k l t n space 1 e");
        cx.run_until_parked();
        projects.read_with(cx, |view, cx| {
            let (_, _, input, _) = view.title_edit.as_ref().expect("the title is a field");
            assert_eq!(input.read(cx).text(), "kltn 1e");
        });
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        projects.read_with(cx, |view, _| {
            assert!(view.title_edit.is_none(), "escape closes the title field");
            assert!(view.editor.is_some(), "escape leaves the note open");
        });
    }
}
