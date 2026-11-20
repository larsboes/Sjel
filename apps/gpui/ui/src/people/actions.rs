//! Keyboard actions, key bindings, and entity mutation workflows for People.
use std::sync::Arc;

use gpui_kit::{actions, App, Context, Focusable, KeyBinding, Window};
use sjel_gpui_core::shell::{self, Shell, ShellError};

use crate::data::Remote;

use super::forms::{Saved, Work};
use super::PeoplePage;

actions!(
    people_page,
    [
        /// Move to the next person.
        Next,
        /// Move to the previous person.
        Previous,
        /// Focus the search field.
        FocusSearch,
        /// Refresh the people list.
        Refresh,
        /// Add a person.
        NewPerson,
        /// Change the person on screen.
        EditPerson,
        /// Add a dated place fact to the person on screen.
        AddFact,
        /// Remove the person on screen. The first press asks.
        RemovePerson,
        /// Declare a field, so the next person can carry it.
        DeclareField,
        /// Leave the current form without saving.
        CloseForm,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("j", Next, Some("PeoplePage")),
        KeyBinding::new("down", Next, Some("PeoplePage")),
        KeyBinding::new("k", Previous, Some("PeoplePage")),
        KeyBinding::new("up", Previous, Some("PeoplePage")),
        KeyBinding::new("/", FocusSearch, Some("PeoplePage")),
        KeyBinding::new("n", NewPerson, Some("PeoplePage")),
        KeyBinding::new("e", EditPerson, Some("PeoplePage")),
        KeyBinding::new("f", AddFact, Some("PeoplePage")),
        KeyBinding::new("x", RemovePerson, Some("PeoplePage")),
        KeyBinding::new("escape", CloseForm, Some("PeoplePage")),
    ]);
}

impl PeoplePage {
    /// The shell, when this page can write at all.
    pub(super) fn writer(&self) -> Option<Arc<Shell>> {
        self.repo.shell().ok()
    }

    /// Removes the person on screen. The first press asks; the second removes.
    pub(super) fn remove_person(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(shell), Some(id)) = (self.writer(), self.selected_id.clone()) else {
            return;
        };
        if self.person(&id).is_none() {
            return;
        }
        if self.confirm_remove.as_deref() != Some(id.as_str()) {
            self.confirm_remove = Some(id);
            cx.notify();
            return;
        }
        self.confirm_remove = None;
        self.write(shell, Work::Remove(id), window, cx);
    }

    pub(super) fn remove_fact(&mut self, fact: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(shell), Some(id)) = (self.writer(), self.selected_id.clone()) else {
            return;
        };
        self.write(shell, Work::RemoveFact(id, fact.to_owned()), window, cx);
    }

    /// Sets `last_contact` to today's date (`YYYY-MM-DD`) in one click.
    pub(super) fn mark_contacted_today(
        &mut self,
        id: &str,
        revision: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shell) = self.writer() else {
            return;
        };
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let body = serde_json::json!({
            "values": { "last_contact": today },
            "expected_revision": revision,
        });
        self.write(shell, Work::Update(id.to_owned(), body), window, cx);
    }

    /// Opens a file picker to attach a local profile picture to `id`.
    pub(super) fn pick_photo(
        &mut self,
        id: String,
        revision: u32,
        cx: &mut Context<Self>,
    ) {
        let Some(shell) = self.writer() else {
            return;
        };
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a profile picture".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let path_str = path.to_string_lossy().into_owned();
            let body = serde_json::json!({
                "values": { "photo": path_str },
                "expected_revision": revision,
            });
            let _ = shell.entity_update(&id, body).await;
            let _ = this.update(cx, |page, cx| {
                page.load(cx);
            });
        })
        .detach();
    }

    pub(super) fn write(
        &mut self,
        shell: Arc<Shell>,
        work: Work,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let task = shell::run(async move {
            match work {
                Work::Create(body) => shell
                    .entity_create(body)
                    .await
                    .map(|record| Saved::Person(record.id)),
                Work::Update(id, body) => shell
                    .entity_update(&id, body)
                    .await
                    .map(|_| Saved::Person(id)),
                Work::Remove(id) => shell.entity_delete(&id).await.map(|()| Saved::Removed),
                Work::AddFact(id, body) => shell
                    .entity_fact_add(&id, body)
                    .await
                    .map(|_| Saved::Person(id)),
                Work::Introduce { from, to, note } => shell
                    .people_link(&from, &to, "introduced", note.as_deref())
                    .await
                    .map(|()| Saved::Person(to)),
                Work::UnlinkIntroduction { from, to } => shell
                    .people_unlink(&from, &to, "introduced")
                    .await
                    .map(|()| Saved::Links),
                Work::RemoveFact(id, fact) => shell
                    .entity_fact_delete(&id, &fact)
                    .await
                    .map(|_| Saved::Person(id)),
                Work::DeclareField(body) => shell
                    .entity_declare_field(body)
                    .await
                    .map(|_| Saved::Fields),
                Work::Merge { keep, other, name } => shell
                    .entity_merge(&keep, &other, &name)
                    .await
                    .map(|()| Saved::Pair),
                Work::Distinct(a, b) => shell
                    .entity_mark_distinct(&a, &b)
                    .await
                    .map(|()| Saved::Pair),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(saved) => {
                        page.form = None;
                        page.write_error = None;
                        page.fields = Remote::default();
                        match saved {
                            Saved::Person(id) => page.selected_id = Some(id),
                            Saved::Removed => page.selected_id = None,
                            Saved::Fields | Saved::Links => {}
                            Saved::Pair => page.load_duplicates(cx),
                        }
                        page.load(cx);
                    }
                    Err(error) => match page.form.as_mut() {
                        Some(form) => {
                            form.saving = false;
                            form.error = Some(error.to_string().into());
                        }
                        None => page.write_error = Some(error.to_string().into()),
                    },
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn on_next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        self.step_selection(true, cx);
    }

    pub(super) fn on_previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        self.step_selection(false, cx);
    }

    pub(super) fn step_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.sync_derived();
        let indices = &self.derived.filtered_indices;
        let count = indices.len();
        if count == 0 {
            return;
        }
        let at = self.selected_id.as_deref().and_then(|id| {
            indices
                .iter()
                .position(|&i| self.derived.parsed[i].id == id)
        });
        let next = match (at, forward) {
            (Some(i), true) => (i + 1) % count,
            (Some(i), false) if i > 0 => i - 1,
            (_, true) => 0,
            (_, false) => count - 1,
        };
        let id = self.derived.parsed[indices[next]].id.clone();
        self.select_person(Some(id), cx);
        if !self.show_network {
            self.people_list.scroll_to_reveal_item(next);
        }
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.focus_handle(cx).focus(window, cx));
    }

    pub(super) fn on_focus_search(
        &mut self,
        _: &FocusSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_search(window, cx);
    }

    pub(super) fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }
}
