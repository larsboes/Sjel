//! Contact editing, creation, fact attachment, and field declaration forms.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, point, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString,
    Subscription, Window,
};
use serde_json::Value;
use sjel_gpui_core::people::edit::{
    edit_fields, FactDraft, FieldDecl, NewPerson as NewPersonDraft, PersonEdit, FACT_PREDICATES,
};

use crate::theme::Palette;

use super::PeoplePage;

/// Which write the open form is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FormKind {
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
pub(crate) struct Form {
    pub(super) kind: FormKind,
    /// (key, label, input). A fact form keys its inputs by what they mean: `place`, `from`, …
    pub(super) fields: Vec<(String, String, Entity<InputState>)>,
    /// The name input, for the two person forms.
    pub(super) name: Option<Entity<InputState>>,
    /// The chosen predicate for a fact, and the chosen type for a field.
    pub(super) choice: String,
    pub(super) error: Option<SharedString>,
    pub(super) saving: bool,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl Form {
    pub(super) fn value(&self, key: &str, cx: &gpui::App) -> String {
        self.fields
            .iter()
            .find(|(field, _, _)| field == key)
            .map(|(_, _, input)| input.read(cx).value().to_string())
            .unwrap_or_default()
    }
}

pub(super) enum Work {
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

pub(super) enum Saved {
    Person(String),
    Removed,
    Fields,
    Pair,
}

/// The field types the registry accepts, in the order a form offers them.
pub(super) const FIELD_TYPES: [&str; 10] = [
    "text", "longtext", "emails", "phones", "tags", "date", "enum", "number", "bool", "url",
];

pub(super) fn label_placeholder(label: &str) -> String {
    match label.to_lowercase().as_str() {
        "email" => "ada@example.org".into(),
        "phone" => "+44 20 7946 0991".into(),
        "company" => "Analytical Engine Co.".into(),
        "role" => "Mathematician".into(),
        "location" => "London".into(),
        _ => String::new(),
    }
}

impl PeoplePage {
    pub(super) fn open_form(
        &mut self,
        kind: FormKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn close_form(&mut self, cx: &mut Context<Self>) {
        self.form = None;
        cx.notify();
    }

    pub(super) fn save_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn render_form(
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
}
