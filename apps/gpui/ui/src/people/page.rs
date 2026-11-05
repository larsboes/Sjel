//! Native People page (`/people`): the contact registry, and the forms that write it.
//!
//! Reads go through the shell (`GET /entities/api/entities?kind=person`), writes through the same
//! shell. The field list is the registry's own (`GET /entities/api/fields?kind=person`), so a form
//! never offers a key the capability would refuse.

use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    actions, div, point, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FocusHandle,
    FontWeight, KeyBinding, ScrollHandle, SharedString, Subscription, Window,
};
use serde_json::Value;
use sjel_gpui_core::people::edit::{
    edit_fields, FactDraft, FieldDecl, NewPerson as NewPersonDraft, PersonEdit, FACT_PREDICATES,
};
use sjel_gpui_core::people::{filter_people, Person};
use sjel_gpui_core::shell::{self, DuplicatePair, EntityField, EntityRecord, Shell, ShellError};

use crate::data::{Remote, Repo};
use crate::primitives::virtual_list;
use crate::theme::{tint, Palette, ThemeContext};

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

/// The field types the registry accepts, in the order a form offers them.
const FIELD_TYPES: [&str; 10] = [
    "text", "longtext", "emails", "phones", "tags", "date", "enum", "number", "bool", "url",
];

/// Which write the open form is for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FormKind {
    /// A person nobody has added yet.
    New,
    /// The person with this id, changed.
    Edit(String),
    /// A dated fact on the person with this id.
    Fact(String),
    /// A field, so the next person can carry it.
    Field,
}

/// The open form: its inputs, and what a failed write said.
struct Form {
    kind: FormKind,
    /// (key, label, input). A fact form keys its inputs by what they mean: `place`, `from`, …
    fields: Vec<(String, String, Entity<InputState>)>,
    /// The name input, for the two person forms.
    name: Option<Entity<InputState>>,
    /// The chosen predicate for a fact, and the chosen type for a field.
    choice: String,
    error: Option<SharedString>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl Form {
    fn value(&self, key: &str, cx: &gpui::App) -> String {
        self.fields
            .iter()
            .find(|(field, _, _)| field == key)
            .map(|(_, _, input)| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
}

enum Work {
    Create(Value),
    Update(String, Value),
    Remove(String),
    AddFact(String, Value),
    RemoveFact(String, String),
    DeclareField(Value),
    Merge {
        keep: String,
        other: String,
        name: String,
    },
    Distinct(String, String),
}

enum Saved {
    Person(String),
    Removed,
    Fields,
    Pair,
}

#[derive(Default)]
struct PeopleDerived {
    people_at: Option<Option<Instant>>,
    parsed: Vec<Person>,
    filter_key: Option<(Option<Instant>, u64)>,
    filtered_indices: Vec<usize>,
}

pub struct PeoplePage {
    repo: Repo,
    palette: Option<Palette>,
    pub people: Remote<Vec<EntityRecord>>,
    /// The fields the registry declares for a person: what a form may offer.
    pub fields: Remote<Vec<EntityField>>,
    /// Probable duplicates, read when the Duplicates view opens.
    duplicates: Remote<Vec<DuplicatePair>>,
    show_duplicates: bool,
    /// The open form, when one is.
    form: Option<Form>,
    /// The person whose removal the next press confirms.
    confirm_remove: Option<String>,
    /// A write that failed with no form open to show it.
    write_error: Option<SharedString>,
    pub selected_id: Option<String>,
    pub search_query: String,
    filter_rev: u64,
    derived: PeopleDerived,
    people_list: gpui::ListState,
    search_input: Option<(Entity<InputState>, Subscription)>,
    form_scroll: ScrollHandle,
    detail_scroll: ScrollHandle,
    focus: FocusHandle,
    focus_next: bool,
}

impl PeoplePage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            people: Remote::default(),
            fields: Remote::default(),
            duplicates: Remote::default(),
            show_duplicates: false,
            form: None,
            confirm_remove: None,
            write_error: None,
            selected_id: None,
            search_query: String::new(),
            filter_rev: 0,
            derived: PeopleDerived::default(),
            people_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(600.)),
            search_input: None,
            form_scroll: ScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            focus_next: true,
        }
    }

    pub fn detail_scroll(&self) -> &ScrollHandle {
        &self.detail_scroll
    }

    pub fn bench_scroll(&mut self, fraction: f32, cx: &mut Context<Self>) {
        self.sync_derived();
        let rows = self.derived.filtered_indices.len();
        let item_ix = (rows.saturating_sub(1) as f32 * fraction).round() as usize;
        self.people_list.scroll_to(gpui::ListOffset {
            item_ix,
            offset_in_item: px(0.),
        });
        cx.notify();
    }

    fn sync_derived(&mut self) {
        if self.derived.people_at != Some(self.people.loaded_at) {
            self.derived.people_at = Some(self.people.loaded_at);
            self.derived.parsed = self
                .people
                .value
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(Person::from_record)
                .collect();
        }
        let filter_key = (self.people.loaded_at, self.filter_rev);
        if self.derived.filter_key != Some(filter_key) {
            self.derived.filter_key = Some(filter_key);
            let filtered = filter_people(&self.derived.parsed, &self.search_query);
            let base = self.derived.parsed.as_ptr() as usize;
            let stride = std::mem::size_of::<Person>();
            let indices: Vec<usize> = if stride > 0 {
                filtered
                    .into_iter()
                    .map(|p| ((p as *const Person as usize) - base) / stride)
                    .collect()
            } else {
                Vec::new()
            };
            self.people_list.reset(indices.len());
            self.derived.filtered_indices = indices;
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.focus_next = true;
        self.load(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };

        self.people.loading = true;
        cx.notify();

        let reading = shell.clone();
        let work = shell::run(async move { reading.people(None).await });
        cx.spawn(async move |this, cx| {
            let res = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if let Ok(records) = &res {
                    if page.selected_id.is_none() {
                        page.selected_id = records.first().map(|r| r.id.clone());
                    }
                }
                page.people.settle(res);
                cx.notify();
            });
        })
        .detach();

        // Read once: the field registry changes only when the operator declares a field, and that
        // write reads it again.
        if self.fields.value.is_none() && !self.fields.loading {
            self.load_fields(shell, cx);
        }
    }

    fn load_duplicates(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        self.duplicates.loading = true;
        let work = shell::run(async move { shell.entity_duplicates(20).await });
        cx.spawn(async move |this, cx| {
            let res = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.duplicates.settle(res);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_fields(&mut self, shell: Arc<Shell>, cx: &mut Context<Self>) {
        self.fields.loading = true;
        let work = shell::run(async move { shell.entity_fields("person").await });
        cx.spawn(async move |this, cx| {
            let res = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.fields.settle(res);
                cx.notify();
            });
        })
        .detach();
    }

    /// The shell, when this page can write at all.
    fn writer(&self) -> Option<Arc<Shell>> {
        self.repo.shell().ok()
    }

    fn person(&mut self, id: &str) -> Option<Person> {
        self.sync_derived();
        self.derived
            .parsed
            .iter()
            .find(|person| person.id == id)
            .cloned()
    }

    fn open_form(&mut self, kind: FormKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(_) = self.writer() else {
            return;
        };
        let declared = self.fields.value.clone().unwrap_or_default();
        self.form_scroll.set_offset(point(px(0.), px(0.)));
        let mut subscriptions = Vec::new();
        let mut input = |value: String,
                         placeholder: &str,
                         window: &mut Window,
                         cx: &mut Context<Self>| {
            let field = cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder(placeholder.to_owned());
                state.set_value(value, window, cx);
                state
            });
            subscriptions.push(
                cx.subscribe_in(&field, window, |view, _, event, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        view.save_form(window, cx);
                    }
                }),
            );
            field
        };

        let mut name = None;
        let mut fields = Vec::new();
        let mut choice = String::new();
        match &kind {
            FormKind::New => {
                name = Some(input(String::new(), "Ada Lovelace", window, cx));
                for field in edit_fields(&declared, &Default::default()) {
                    let held = String::new();
                    let placeholder = label_placeholder(&field.label);
                    fields.push((
                        field.key.clone(),
                        field.label.clone(),
                        input(held, &placeholder, window, cx),
                    ));
                }
            }
            FormKind::Edit(id) => {
                let Some(person) = self.person(id) else {
                    return;
                };
                name = Some(input(person.name.clone(), "Name", window, cx));
                for field in edit_fields(&declared, &person.values) {
                    let held = person.value(&field.key);
                    let placeholder = label_placeholder(&field.label);
                    fields.push((
                        field.key.clone(),
                        field.label.clone(),
                        input(held, &placeholder, window, cx),
                    ));
                }
            }
            FormKind::Fact(_) => {
                choice = FACT_PREDICATES[0].0.to_owned();
                fields.push((
                    "place".into(),
                    "Place".into(),
                    input(String::new(), "Lisbon", window, cx),
                ));
                fields.push((
                    "from".into(),
                    "From".into(),
                    input(
                        String::new(),
                        "YYYY-MM-DD, optional for a home base",
                        window,
                        cx,
                    ),
                ));
                fields.push((
                    "to".into(),
                    "To".into(),
                    input(
                        String::new(),
                        "YYYY-MM-DD, needed for an away period",
                        window,
                        cx,
                    ),
                ));
                fields.push((
                    "note".into(),
                    "Note".into(),
                    input(String::new(), "Optional", window, cx),
                ));
            }
            FormKind::Field => {
                choice = "text".to_owned();
                fields.push((
                    "key".into(),
                    "Key".into(),
                    input(String::new(), "favourite_drink", window, cx),
                ));
                fields.push((
                    "label".into(),
                    "Label".into(),
                    input(String::new(), "Favourite drink", window, cx),
                ));
                fields.push((
                    "options".into(),
                    "Options".into(),
                    input(String::new(), "Only for an enum: one, two", window, cx),
                ));
            }
        }
        if let Some(first) = name
            .clone()
            .or_else(|| fields.first().map(|(_, _, i)| i.clone()))
        {
            first.update(cx, |input, cx| input.focus(window, cx));
        }
        self.form = Some(Form {
            kind,
            fields,
            name,
            choice,
            error: None,
            saving: false,
            _subscriptions: subscriptions,
        });
        self.focus_next = false;
        self.confirm_remove = None;
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) {
        self.form = None;
        cx.notify();
    }

    fn save_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shell) = self.writer() else {
            return;
        };
        let Some(form) = self.form.as_ref() else {
            return;
        };
        if form.saving {
            return;
        }
        let kind = form.kind.clone();
        let choice = form.choice.clone();
        let place = form.value("place", cx);
        let from = form.value("from", cx);
        let to = form.value("to", cx);
        let note = form.value("note", cx);
        let field_key = form.value("key", cx);
        let field_label = form.value("label", cx);
        let field_options = form.value("options", cx);
        let name = form
            .name
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        let field_values: Vec<(String, String)> = form
            .fields
            .iter()
            .map(|(key, _, input)| (key.clone(), input.read(cx).value().to_string()))
            .collect();
        let types = self
            .fields
            .value
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|field| (field.key.clone(), field.field_type.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let work: Result<_, String> = match kind {
            FormKind::New => NewPersonDraft {
                name,
                values: field_values,
                types,
            }
            .body()
            .map(Work::Create),
            FormKind::Edit(id) => {
                let Some(person) = self.person(&id) else {
                    return;
                };
                PersonEdit {
                    id: id.clone(),
                    name,
                    original_name: person.name.clone(),
                    revision: person.revision,
                    before: person.values.clone(),
                    after: field_values.into_iter().collect(),
                    types,
                }
                .body()
                .map(|body| Work::Update(id, body))
            }
            FormKind::Fact(id) => FactDraft {
                predicate: choice,
                place,
                from,
                to,
                note,
            }
            .body()
            .map(|body| Work::AddFact(id, body)),
            FormKind::Field => FieldDecl {
                key: field_key,
                label: field_label,
                field_type: choice,
                options: field_options,
            }
            .body()
            .map(Work::DeclareField),
        };
        let work = match work {
            Ok(work) => work,
            Err(why) => {
                if let Some(form) = self.form.as_mut() {
                    form.error = Some(why.into());
                }
                cx.notify();
                return;
            }
        };
        if let Some(form) = self.form.as_mut() {
            form.saving = true;
            form.error = None;
        }
        self.write(shell, work, window, cx);
    }

    /// Removes the person on screen. The first press asks; the second removes.
    fn remove_person(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn remove_fact(&mut self, fact: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(shell), Some(id)) = (self.writer(), self.selected_id.clone()) else {
            return;
        };
        self.write(shell, Work::RemoveFact(id, fact.to_owned()), window, cx);
    }

    fn write(&mut self, shell: Arc<Shell>, work: Work, _: &mut Window, cx: &mut Context<Self>) {
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

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search people…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.search_query = input.read(cx).value().trim().to_lowercase();
                view.filter_rev = view.filter_rev.wrapping_add(1);
                cx.notify();
            }
        });
        self.search_input = Some((input.clone(), subscription));
        input
    }

    fn on_next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
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

    fn on_previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
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

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_search(window, cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }

    fn render_form(
        &mut self,
        palette: Palette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let form = self.form.as_ref()?;
        let mut card = div()
            .p_4()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .flex()
            .flex_col()
            .gap_3();
        let title = match form.kind {
            FormKind::New => "New person",
            FormKind::Edit(_) => "Edit person",
            FormKind::Fact(_) => "Add place fact",
            FormKind::Field => "Declare field",
        };
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    Button::new("people-form-close")
                        .small()
                        .ghost()
                        .label("Cancel")
                        .disabled(form.saving)
                        .on_click(cx.listener(|page, _, _, cx| page.close_form(cx))),
                ),
        );
        let mut fields = div().flex().flex_col().gap_3().flex_none().w_full();
        if let Some(name) = &form.name {
            fields = fields.child(Input::new(name).small());
        }
        for (key, label, input) in &form.fields {
            fields = fields.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(label.clone()),
                    )
                    .child(Input::new(input).small())
                    .id(SharedString::from(format!("people-field-{key}"))),
            );
        }
        let choices: Vec<(&str, &str)> = match &form.kind {
            FormKind::Fact(_) => FACT_PREDICATES.to_vec(),
            FormKind::Field => FIELD_TYPES.iter().map(|ty| (*ty, *ty)).collect(),
            _ => Vec::new(),
        };
        if !choices.is_empty() {
            let current = form.choice.clone();
            fields = fields.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(choices.iter().map(|(value, label)| {
                        let choice_value = (*value).to_owned();
                        let choice_label = (*label).to_owned();
                        Button::new(SharedString::from(format!("people-choice-{value}")))
                            .small()
                            .label(choice_label)
                            .when(current == *value, |button| button.primary())
                            .when(current != *value, |button| button.ghost())
                            .on_click(cx.listener(move |page, _, _, cx| {
                                if let Some(form) = page.form.as_mut() {
                                    form.choice = choice_value.clone();
                                }
                                cx.notify();
                            }))
                    })),
            );
        }
        if let Some(error) = &form.error {
            fields = fields.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        card = card.child(
            div()
                .id("people-form-fields")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scroll()
                .track_scroll(&self.form_scroll)
                .vertical_scrollbar(&self.form_scroll)
                .child(fields),
        );
        let saving = form.saving;
        card = card.child(
            div().flex_none().flex().justify_end().gap_2().child(
                Button::new("people-form-save")
                    .small()
                    .primary()
                    .label(if saving { "Saving…" } else { "Save" })
                    .disabled(saving)
                    .on_click(cx.listener(|page, _, window, cx| page.save_form(window, cx))),
            ),
        );
        Some(card.into_any_element())
    }

    fn render_person_detail(
        &self,
        person: Person,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        render_person_detail(
            person.clone(),
            palette,
            cx,
            &self.detail_scroll,
            self.confirm_remove.as_deref() == Some(person.id.as_str()),
        )
    }
}

impl PeoplePage {
    /// Probable duplicates with the registry's default merge: the side to keep and the name.
    fn render_duplicates(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let pairs = self.duplicates.value.clone().unwrap_or_default();
        let status = if self.duplicates.loading && pairs.is_empty() {
            Some("Looking for duplicates…".to_owned())
        } else if let Some(error) = self.duplicates.error.as_ref() {
            Some(format!("Could not read duplicates: {error}"))
        } else if pairs.is_empty() {
            Some("No probable duplicates.".to_owned())
        } else {
            None
        };
        let mut list = div().flex().flex_col().gap_2().w_full();
        for (i, pair) in pairs.into_iter().enumerate() {
            let (keep, other) = if pair.keep == "b" {
                (&pair.b, &pair.a)
            } else {
                (&pair.a, &pair.b)
            };
            let name_of = |side: &sjel_gpui_core::shell::DuplicateSide| {
                side.profile["name"].as_str().unwrap_or("?").to_owned()
            };
            let (keep_id, other_id, name) =
                (keep.id.clone(), other.id.clone(), pair.merged_name.clone());
            let (a, b) = (pair.a.id.clone(), pair.b.id.clone());
            let why = if pair.reasons.is_empty() {
                pair.strength.clone()
            } else {
                format!("{} · {}", pair.strength, pair.reasons.join(", "))
            };
            list = list.child(
                div()
                    .p_3()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.surface))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.text))
                            .child(format!("{} ↔ {}", name_of(&pair.a), name_of(&pair.b))),
                    )
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(why))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new(SharedString::from(format!("dup-merge-{i}")))
                                    .xsmall()
                                    .primary()
                                    .label(format!("Merge into \"{}\"", pair.merged_name))
                                    .on_click(cx.listener(move |page, _, window, cx| {
                                        let Some(shell) = page.writer() else { return };
                                        let work = Work::Merge {
                                            keep: keep_id.clone(),
                                            other: other_id.clone(),
                                            name: name.clone(),
                                        };
                                        page.write(shell, work, window, cx);
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("dup-distinct-{i}")))
                                    .xsmall()
                                    .ghost()
                                    .label("Different people")
                                    .on_click(cx.listener(move |page, _, window, cx| {
                                        let Some(shell) = page.writer() else { return };
                                        page.write(
                                            shell,
                                            Work::Distinct(a.clone(), b.clone()),
                                            window,
                                            cx,
                                        );
                                    })),
                            ),
                    ),
            );
        }
        div()
            .id("people-duplicates")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .overflow_y_scroll()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("The kept side has a note or more filled fields. Its values win where both have one."),
            )
            .when_some(status, |el, status| {
                el.child(div().text_sm().text_color(rgb(palette.muted)).child(status))
            })
            .child(list)
            .into_any_element()
    }
}

fn label_placeholder(label: &str) -> String {
    format!("Enter {}", label.to_lowercase())
}

impl Render for PeoplePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_next) {
            window.focus(&self.focus, cx);
        }

        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
        self.sync_derived();
        let total_contacts = self.derived.parsed.len();
        let is_empty = self.derived.filtered_indices.is_empty();

        let selected_person = self
            .selected_id
            .as_ref()
            .and_then(|id| self.derived.parsed.iter().find(|p| &p.id == id))
            .cloned();

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child("People"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(format!("{total_contacts} contacts")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("people-new")
                            .small()
                            .primary()
                            .label("New person")
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.open_form(FormKind::New, window, cx)
                            })),
                    )
                    .child(
                        Button::new("people-declare-field")
                            .small()
                            .ghost()
                            .label("Declare field")
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.open_form(FormKind::Field, window, cx)
                            })),
                    )
                    .child(
                        Button::new("people-duplicates")
                            .small()
                            .ghost()
                            .label(if self.show_duplicates {
                                "Close duplicates"
                            } else {
                                "Duplicates"
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.show_duplicates = !page.show_duplicates;
                                page.form = None;
                                if page.show_duplicates {
                                    page.load_duplicates(cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("people-refresh")
                            .small()
                            .ghost()
                            .label("↻ Refresh")
                            .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                    ),
            );

        let write_error = self.write_error.clone();

        let search_box = div()
            .w_full()
            .child(Input::new(&self.search_field(window, cx)).small());

        // Left List Pane
        let mut list_pane = div()
            .w(px(320.))
            .min_h_0()
            .min_w_0()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .child(search_box);

        if is_empty {
            list_pane = list_pane.child(
                div()
                    .p_6()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.surface))
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("No contacts found."),
            );
        } else {
            let entity = cx.entity();
            list_pane = list_pane.child(virtual_list(
                "people-list-scroll",
                &self.people_list,
                move |ix, _window, cx| {
                    entity.update(cx, |page, cx| {
                        let Some(&person_ix) = page.derived.filtered_indices.get(ix) else {
                            return div().into_any_element();
                        };
                        let Some(person) = page.derived.parsed.get(person_ix) else {
                            return div().into_any_element();
                        };
                        let chosen = page.selected_id.as_deref() == Some(person.id.as_str());
                        let pid = person.id.clone();
                        let initials = person.initials.clone();
                        let name = person.name.clone();
                        let subtitle = [person.company.as_deref(), person.role.as_deref()]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ");

                        div()
                            .pb_1()
                            .child(
                                div()
                                    .id(SharedString::from(format!("person-item-{pid}")))
                                    .flex_none()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .p_2p5()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(if chosen {
                                        rgb(palette.accent)
                                    } else {
                                        rgba(palette.border)
                                    })
                                    .bg(if chosen {
                                        rgba(palette.raised)
                                    } else {
                                        rgba(palette.surface)
                                    })
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.selected_id = Some(pid.clone());
                                        page.form = None;
                                        page.confirm_remove = None;
                                        page.detail_scroll.set_offset(point(px(0.), px(0.)));
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .w(px(32.))
                                            .h(px(32.))
                                            .rounded_full()
                                            .bg(rgba(tint(palette.accent, 0x22)))
                                            .border_1()
                                            .border_color(rgb(palette.accent))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(rgb(palette.accent))
                                            .child(initials),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap_0p5()
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(palette.text))
                                                    .child(name),
                                            )
                                            .when(!subtitle.is_empty(), |el| {
                                                el.child(
                                                    div()
                                                        .truncate()
                                                        .text_xs()
                                                        .text_color(rgb(palette.muted))
                                                        .child(subtitle),
                                                )
                                            }),
                                    ),
                            )
                            .into_any_element()
                    })
                },
            ));
        }

        // Right Detail Pane
        let detail_pane = if let Some(form) = self.render_form(palette, window, cx) {
            form
        } else if self.show_duplicates {
            self.render_duplicates(palette, cx)
        } else if let Some(p) = selected_person {
            self.render_person_detail(p, palette, cx)
        } else {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_8()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Select a contact to view details")
                .into_any_element()
        };

        div()
            .key_context("PeoplePage")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_previous))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(|page, _: &NewPerson, window, cx| {
                page.open_form(FormKind::New, window, cx)
            }))
            .on_action(cx.listener(|page, _: &EditPerson, window, cx| {
                if let Some(id) = page.selected_id.clone() {
                    page.open_form(FormKind::Edit(id), window, cx);
                }
            }))
            .on_action(cx.listener(|page, _: &AddFact, window, cx| {
                if let Some(id) = page.selected_id.clone() {
                    page.open_form(FormKind::Fact(id), window, cx);
                }
            }))
            .on_action(
                cx.listener(|page, _: &RemovePerson, window, cx| page.remove_person(window, cx)),
            )
            .on_action(cx.listener(|page, _: &DeclareField, window, cx| {
                page.open_form(FormKind::Field, window, cx)
            }))
            .on_action(cx.listener(|page, _: &CloseForm, _, cx| {
                if page.form.is_some() {
                    page.close_form(cx);
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(header)
            .when_some(write_error, |page, error| {
                page.child(
                    div()
                        .p_3()
                        .rounded(px(6.))
                        .bg(rgba(palette.surface))
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_4()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(list_pane)
                    .child(detail_pane),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pt_2()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(shortcut_hint("↓ / ↑ or j / k", "navigate", palette))
                    .child(shortcut_hint("/", "search", palette))
                    .child(shortcut_hint("⌘r", "refresh", palette)),
            )
    }
}

fn render_person_detail(
    person: Person,
    palette: Palette,
    cx: &mut Context<PeoplePage>,
    scroll: &ScrollHandle,
    confirming: bool,
) -> AnyElement {
    let company_role = [person.role.as_deref(), person.company.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" at ");

    let header_card = div()
        .p_6()
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .w(px(52.))
                .h(px(52.))
                .rounded_full()
                .bg(rgba(tint(palette.accent, 0x22)))
                .border_2()
                .border_color(rgb(palette.accent))
                .flex()
                .items_center()
                .justify_center()
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(palette.accent))
                .child(person.initials),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(palette.text))
                        .child(person.name),
                )
                .when(!company_role.is_empty(), |el| {
                    el.child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(company_role),
                    )
                }),
        );

    // Contact fields card
    let mut contact_fields = div().flex().flex_col().gap_2().flex_none().w_full();

    if let Some(email) = person.email {
        contact_fields = contact_fields.child(detail_row("EMAIL", email, palette));
    }
    if let Some(phone) = person.phone {
        contact_fields = contact_fields.child(detail_row("PHONE", phone, palette));
    }
    if let Some(location) = person.location {
        contact_fields = contact_fields.child(detail_row("LOCATION", location, palette));
    }
    if let Some(note) = person.note_ref {
        contact_fields = contact_fields.child(detail_row("NOTE", note, palette));
    }

    for (k, v) in person.custom_fields {
        contact_fields = contact_fields.child(detail_row(&k.to_uppercase(), v, palette));
    }

    div()
        .id("person-detail-scroll")
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .vertical_scrollbar(scroll)
        .flex()
        .flex_col()
        .gap_4()
        .child(header_card.flex_none().w_full())
        .child(
            div()
                .flex_none()
                .w_full()
                .flex()
                .gap_2()
                .child(
                    Button::new("person-edit")
                        .small()
                        .ghost()
                        .label("Edit")
                        .on_click(cx.listener(|page, _, window, cx| {
                            if let Some(id) = page.selected_id.clone() {
                                page.open_form(FormKind::Edit(id), window, cx);
                            }
                        })),
                )
                .child(
                    Button::new("person-delete")
                        .small()
                        .ghost()
                        .label(if confirming {
                            "Confirm delete"
                        } else {
                            "Delete"
                        })
                        .on_click(
                            cx.listener(|page, _, window, cx| page.remove_person(window, cx)),
                        ),
                )
                .child(
                    Button::new("person-add-fact")
                        .small()
                        .ghost()
                        .label("Add fact")
                        .on_click(cx.listener(|page, _, window, cx| {
                            if let Some(id) = page.selected_id.clone() {
                                page.open_form(FormKind::Fact(id), window, cx);
                            }
                        })),
                ),
        )
        .child(
            div()
                .flex_none()
                .w_full()
                .p_4()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("CONTACT & PROPERTIES"),
                )
                .child(contact_fields),
        )
        .when(!person.facts.is_empty(), |el| {
            let facts = person
                .facts
                .iter()
                .fold(div().flex().flex_col().gap_2(), |rows, fact| {
                    let summary = [
                        Some(fact.predicate.as_str()),
                        Some(fact.place.as_str()),
                        fact.valid_from.as_deref(),
                        fact.valid_to.as_deref(),
                        fact.note.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                    let fact_id = fact.id.clone();
                    rows.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .text_color(rgb(palette.text))
                                    .child(summary),
                            )
                            .child(
                                Button::new(SharedString::from(format!("remove-fact-{}", fact.id)))
                                    .small()
                                    .ghost()
                                    .label("Remove")
                                    .on_click(cx.listener(move |page, _, window, cx| {
                                        page.remove_fact(&fact_id, window, cx)
                                    })),
                            ),
                    )
                });
            el.child(
                div()
                    .flex_none()
                    .w_full()
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.surface))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("PLACE FACTS"),
                    )
                    .child(facts),
            )
        })
        .into_any_element()
}

fn detail_row(label: &str, val: String, palette: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .py_1p5()
        .border_b_1()
        .border_color(rgba(palette.border))
        .child(
            div()
                .w(px(120.))
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(label.to_owned()),
        )
        .child(
            div()
                .flex_1()
                .text_sm()
                .text_color(rgb(palette.text))
                .child(val),
        )
        .into_any_element()
}

fn shortcut_hint(key: &'static str, desc: &'static str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .px_1()
                .py(px(1.))
                .rounded(px(3.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.text))
                .child(key),
        )
        .child(div().child(desc))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn people_selection(cx: &mut TestAppContext) {
        let page = cx.new(|cx| PeoplePage::new(Repo::new(Err(ShellError::NotSetUp)), cx));
        page.update(cx, |page, cx| {
            assert!(page.selected_id.is_none());
            page.selected_id = Some("p-123".into());
            assert_eq!(page.selected_id.as_deref(), Some("p-123"));
            cx.notify();
        });
    }
}
