//! Contact editing, creation, fact attachment, and field declaration forms.
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::{
    div, point, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, Focusable, FontWeight,
    SharedString, Subscription, Window,
};
use serde_json::Value;
use sjel_gpui_core::people::edit::{
    edit_fields, FactDraft, FieldDecl, NewPerson as NewPersonDraft, PersonEdit, FACT_PREDICATES,
};

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::forms::{Form as ElyForm, FormError, FormField, Input, InputEvent, TextInput};
use crate::ely::theme::ControlSize;
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
    Introduction(String),
    /// A field, so the next person can carry it.
    Field,
}

/// The open form: its inputs, and what a failed write said.
pub(crate) struct Form {
    pub(super) kind: FormKind,
    /// (key, label, input). A fact form keys its inputs by what they mean: `place`, `from`, …
    pub(super) fields: Vec<(String, String, Entity<TextInput>)>,
    /// Per-field metadata `(field_type, enum_options, source)` for grouped rendering & quick pills.
    pub(super) field_meta: std::collections::HashMap<String, (String, Vec<String>, Option<String>)>,
    /// The name input, for the two person forms.
    pub(super) name: Option<Entity<TextInput>>,
    introducer: Option<Entity<SelectState<SearchableVec<Introducer>>>>,
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
            .map(|(_, _, input)| input.read(cx).text().to_owned())
            .unwrap_or_default()
    }
}

#[derive(Clone)]
struct Introducer {
    id: String,
    label: SharedString,
    aliases: String,
}

impl SearchableListItem for Introducer {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &String {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        self.label.to_lowercase().contains(&query) || self.aliases.to_lowercase().contains(&query)
    }
}

pub(super) enum Work {
    Create(Value),
    Update(String, Value),
    Remove(String),
    AddFact(String, Value),
    RemoveFact(String, String),
    Introduce {
        from: String,
        to: String,
        note: Option<String>,
    },
    UnlinkIntroduction {
        from: String,
        to: String,
    },
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
    Links,
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
        "photo" => "Image URL or local file path".into(),
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
                let mut state = TextInput::new(window, cx).placeholder(placeholder.to_owned());
                state.set_text(value, cx);
                state
            });
            subscriptions.push(
                cx.subscribe_in(&field, window, |view, _, event, window, cx| {
                    if let InputEvent::Submit = event {
                        view.save_form(window, cx);
                    }
                }),
            );
            field
        };

        let mut name = None;
        let mut introducer = None;
        let mut fields = Vec::new();
        let mut field_meta = std::collections::HashMap::new();
        let mut choice = String::new();
        match &kind {
            FormKind::New => {
                name = Some(input(String::new(), "Ada Lovelace", window, cx));
                for field in edit_fields(&declared, &Default::default()) {
                    let held = String::new();
                    let placeholder = label_placeholder(&field.label);
                    field_meta.insert(
                        field.key.clone(),
                        (field.field_type.clone(), field.options.clone(), None),
                    );
                    fields.push((
                        field.key.clone(),
                        field.label.clone(),
                        input(held, &placeholder, window, cx),
                    ));
                }
                if !fields.iter().any(|(k, _, _)| k == "photo") {
                    field_meta.insert("photo".into(), ("text".into(), Vec::new(), None));
                    fields.push((
                        "photo".into(),
                        "Photo".into(),
                        input(String::new(), "Image URL or local file path", window, cx),
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
                    let source = person.sources.get(&field.key).cloned();
                    field_meta.insert(
                        field.key.clone(),
                        (field.field_type.clone(), field.options.clone(), source),
                    );
                    fields.push((
                        field.key.clone(),
                        field.label.clone(),
                        input(held, &placeholder, window, cx),
                    ));
                }
                if !fields.iter().any(|(k, _, _)| k == "photo") {
                    let held = person.photo.clone().unwrap_or_default();
                    let source = person.sources.get("photo").cloned();
                    field_meta.insert("photo".into(), ("text".into(), Vec::new(), source));
                    fields.push((
                        "photo".into(),
                        "Photo".into(),
                        input(held, "Image URL or local file path", window, cx),
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
            FormKind::Introduction(id) => {
                self.sync_derived();
                let mut choices: Vec<_> = self
                    .derived
                    .parsed
                    .iter()
                    .filter(|person| &person.id != id)
                    .map(|person| Introducer {
                        id: person.id.clone(),
                        label: match &person.company {
                            Some(company) => format!("{} · {company}", person.name).into(),
                            None => person.name.clone().into(),
                        },
                        aliases: person.value("aliases"),
                    })
                    .collect();
                choices.sort_by_cached_key(|person| person.label.to_lowercase());
                introducer = Some(cx.new(|cx| {
                    SelectState::new(SearchableVec::from(choices), None, window, cx)
                        .searchable(true)
                }));
                fields.push((
                    "note".into(),
                    "Context".into(),
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
        if let Some(picker) = &introducer {
            picker.update(cx, |picker, cx| picker.focus(window, cx));
        } else if let Some(first) = name
            .clone()
            .or_else(|| fields.first().map(|(_, _, i)| i.clone()))
        {
            first.update(cx, |input, cx| input.focus_handle(cx).focus(window, cx));
        }
        self.form = Some(Form {
            kind,
            fields,
            field_meta,
            name,
            introducer,
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
        let introducer = form
            .introducer
            .as_ref()
            .and_then(|picker| picker.read(cx).selected_value().cloned());
        let field_key = form.value("key", cx);
        let field_label = form.value("label", cx);
        let field_options = form.value("options", cx);
        let name = form
            .name
            .as_ref()
            .map(|input| input.read(cx).text().to_owned())
            .unwrap_or_default();
        let field_values: Vec<(String, String)> = form
            .fields
            .iter()
            .map(|(key, _, input)| (key.clone(), input.read(cx).text().to_owned()))
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
            FormKind::Introduction(to) => introducer
                .map(|from| Work::Introduce {
                    from,
                    to,
                    note: (!note.trim().is_empty()).then(|| note.trim().to_owned()),
                })
                .ok_or_else(|| "Choose the person who made the introduction.".to_owned()),
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
            FormKind::Introduction(_) => "Record introduction",
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
                    Button::new("people-form-close", "Cancel")
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(form.saving)
                        .on_click(cx.listener(|page, _, _, cx| page.close_form(cx))),
                ),
        );
        let page_entity = cx.entity();
        let mut fields = ElyForm::new("people-form").on_submit(move |window, cx| {
            page_entity.update(cx, |page, cx| page.save_form(window, cx));
        });
        let mut form_body = div().flex().flex_col().gap_3().flex_none().w_full();
        if let Some(name) = &form.name {
            form_body = form_body.child(
                FormField::new("people-field-name", "Full name")
                    .child(Input::new(name).size(ControlSize::Sm)),
            );
        }
        if let Some(picker) = &form.introducer {
            form_body = form_body.child(
                FormField::new("people-introducer-field", "Introduced by").child(
                    Select::new(picker)
                        .id("people-introducer")
                        .placeholder("Choose a person")
                        .accessibility_label("Introduced by")
                        .search_placeholder("Search names or aliases…")
                        .disabled(form.saving),
                ),
            );
        }
        let render_field_row = |key: &String,
                                label: &String,
                                input: &Entity<TextInput>,
                                cx: &gpui::App|
         -> AnyElement {
            let meta = form.field_meta.get(key);
            let field_type = meta.map(|m| m.0.as_str()).unwrap_or("text");
            let options = meta.map(|m| m.1.as_slice()).unwrap_or(&[]);
            let source = meta.and_then(|m| m.2.as_deref());
            let current_val = input.read(cx).text().trim().to_string();
            let display_label = match source {
                Some(src) if src != "operator" => format!("{label} · {src}"),
                _ => label.clone(),
            };
            let mut field_el = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    FormField::new(
                        SharedString::from(format!("people-field-{key}")),
                        display_label,
                    )
                    .child(Input::new(input).size(ControlSize::Sm)),
                );
            if field_type == "enum" && !options.is_empty() {
                field_el = field_el.child(div().flex().flex_wrap().gap_1().children(
                    options.iter().map(|opt| {
                        let opt_val = opt.clone();
                        let input_ref = input.clone();
                        let active = current_val.eq_ignore_ascii_case(&opt_val);
                        Button::new(
                            SharedString::from(format!("people-enum-{key}-{opt_val}")),
                            opt.clone(),
                        )
                        .size(ControlSize::Sm)
                        .variant(if active {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Ghost
                        })
                        .on_click(move |_, _, cx| {
                            input_ref.update(cx, |state, cx| state.set_text(opt_val.clone(), cx));
                        })
                    }),
                ));
            } else if key == sjel_gpui_core::people::FOLLOW_UP_WEEKS {
                field_el = field_el.child(div().flex().flex_wrap().gap_1().children(
                    [("1", "1w"), ("2", "2w"), ("4", "1m"), ("12", "3m"), ("26", "6m")]
                        .into_iter()
                        .map(|(weeks, text)| {
                            let input_ref = input.clone();
                            let active = current_val == weeks;
                            Button::new(
                                SharedString::from(format!("people-followup-preset-{weeks}")),
                                text,
                            )
                            .size(ControlSize::Sm)
                            .variant(if active {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Ghost
                            })
                            .on_click(move |_, _, cx| {
                                input_ref.update(cx, |state, cx| state.set_text(weeks, cx));
                            })
                        }),
                ));
            }
            field_el.into_any_element()
        };

        if matches!(form.kind, FormKind::New | FormKind::Edit(_)) {
            let is_identity = |k: &str| {
                matches!(
                    k.to_ascii_lowercase().as_str(),
                    "email"
                        | "emails"
                        | "mail"
                        | "phone"
                        | "phones"
                        | "mobile"
                        | "tel"
                        | "company"
                        | "org"
                        | "organization"
                        | "role"
                        | "title"
                        | "position"
                        | "location"
                        | "city"
                        | "place"
                        | "photo"
                        | "avatar"
                        | "picture"
                        | "alias"
                        | "aliases"
                )
            };
            let is_relationship = |k: &str| {
                matches!(
                    k.to_ascii_lowercase().as_str(),
                    "linkedin" | "connected_on" | "met_at"
                ) || k == sjel_gpui_core::people::FOLLOW_UP_WEEKS
            };
            let identity_fields: Vec<_> = form
                .fields
                .iter()
                .filter(|(k, _, _)| is_identity(k))
                .collect();
            let rel_fields: Vec<_> = form
                .fields
                .iter()
                .filter(|(k, _, _)| is_relationship(k))
                .collect();
            let other_fields: Vec<_> = form
                .fields
                .iter()
                .filter(|(k, _, _)| !is_identity(k) && !is_relationship(k))
                .collect();
            let section_hdr = |title: &'static str| {
                div()
                    .pt_2()
                    .pb_1()
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(title)
            };
            if !identity_fields.is_empty() {
                form_body = form_body.child(section_hdr("IDENTITY & CONTACT"));
                for (key, label, input) in identity_fields {
                    form_body = form_body.child(render_field_row(key, label, input, cx));
                }
            }
            if !rel_fields.is_empty() {
                form_body = form_body.child(section_hdr("RELATIONSHIP & CADENCE"));
                for (key, label, input) in rel_fields {
                    form_body = form_body.child(render_field_row(key, label, input, cx));
                }
            }
            if !other_fields.is_empty() {
                form_body = form_body.child(section_hdr("TAGS & CUSTOM FIELDS"));
                for (key, label, input) in other_fields {
                    form_body = form_body.child(render_field_row(key, label, input, cx));
                }
            }
        } else {
            for (key, label, input) in &form.fields {
                form_body = form_body.child(render_field_row(key, label, input, cx));
            }
        }
        let choices: Vec<(&str, &str)> = match &form.kind {
            FormKind::Fact(_) => FACT_PREDICATES.to_vec(),
            FormKind::Field => FIELD_TYPES.iter().map(|ty| (*ty, *ty)).collect(),
            _ => Vec::new(),
        };
        if !choices.is_empty() {
            let current = form.choice.clone();
            form_body = form_body.child(div().flex().flex_wrap().gap_2().children(
                choices.iter().map(|(value, label)| {
                    let choice_value = (*value).to_owned();
                    let choice_label = (*label).to_owned();
                    Button::new(
                        SharedString::from(format!("people-choice-{value}")),
                        choice_label,
                    )
                    .size(ControlSize::Sm)
                    .variant(if current == *value {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .on_click(cx.listener(move |page, _, _, cx| {
                        if let Some(form) = page.form.as_mut() {
                            form.choice = choice_value.clone();
                        }
                        cx.notify();
                    }))
                }),
            ));
        }
        if let Some(error) = &form.error {
            form_body = form_body.child(FormError::new("people-form-error", error.clone()));
        }
        fields = fields.child(form_body);
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
                Button::new("people-form-save", if saving { "Saving…" } else { "Save" })
                    .size(ControlSize::Sm)
                    .primary()
                    .disabled(saving)
                    .on_click(cx.listener(|page, _, window, cx| page.save_form(window, cx))),
            ),
        );
        Some(card.into_any_element())
    }
}
