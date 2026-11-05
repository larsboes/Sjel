//! Keyboard actions, key bindings, and entity mutation workflows for People.
use std::sync::Arc;

use gpui_kit::{
    actions, App, Context, KeyBinding, Window,
};
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

    pub(super) fn write(&mut self, shell: Arc<Shell>, work: Work, _: &mut Window, cx: &mut Context<Self>) {
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
                            Saved::Fields => {}
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
        self.sync_derived();
        if self.derived.filtered_indices.is_empty() {
            return;
        }
        let at = self.selected_id.as_ref().and_then(|id| {
            self.derived
                .filtered_indices
                .iter()
                .position(|&ix| self.derived.parsed.get(ix).is_some_and(|p| &p.id == id))
        });
        let next_idx = match at {
            Some(i) if i + 1 < self.derived.filtered_indices.len() => i + 1,
            _ => 0,
        };
        if let Some(&person_ix) = self.derived.filtered_indices.get(next_idx) {
            if let Some(person) = self.derived.parsed.get(person_ix) {
                self.selected_id = Some(person.id.clone());
                self.people_list.scroll_to_reveal_item(next_idx);
                cx.notify();
            }
        }
    }

    pub(super) fn on_previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        self.sync_derived();
        if self.derived.filtered_indices.is_empty() {
            return;
        }
        let at = self.selected_id.as_ref().and_then(|id| {
            self.derived
                .filtered_indices
                .iter()
                .position(|&ix| self.derived.parsed.get(ix).is_some_and(|p| &p.id == id))
        });
        let prev_idx = match at {
            Some(i) if i > 0 => i - 1,
            _ => self.derived.filtered_indices.len().saturating_sub(1),
        };
        if let Some(&person_ix) = self.derived.filtered_indices.get(prev_idx) {
            if let Some(person) = self.derived.parsed.get(person_ix) {
                self.selected_id = Some(person.id.clone());
                self.people_list.scroll_to_reveal_item(prev_idx);
                cx.notify();
            }
        }
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.search_field(window, cx);
        field.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_search(window, cx);
    }

    pub(super) fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }
}
