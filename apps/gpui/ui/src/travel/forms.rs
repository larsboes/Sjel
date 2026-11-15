//! Writing a trip from the app: a new trip, a booking, a stage. One form card serves all three.
//!
//! Writes go through the Mac's shell only. A phone holds a read copy of trips and has no outbox
//! for them, so it offers no control the Mac would have to refuse.
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, Focusable, SharedString,
    Subscription, Window,
};
use sjel_gpui_core::{
    shell::{self, Shell, ShellError},
    travel::{
        edit::{
            stay_rank_body, BookingDraft, StageDraft, StageEdit, StayDraft, TripDraft,
            STAGE_STATUSES, TRANSPORT_MODES,
        },
        Item, Source, Stage, TravelRoute, TripSection,
    },
};

use super::Travel;
use crate::ely::{
    buttons::{Button, ButtonVariant, SegmentedControl},
    forms::{
        DatePicker, Form as ElyForm, FormError, FormField as ElyFormField, Input, InputEvent,
        TextInput,
    },
    theme::ControlSize,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FormKind {
    Trip(Option<String>),
    /// `None` adds a booking; `Some(item id)` edits that one.
    Booking(Option<String>),
    /// `None` adds a stay candidate; `Some(item id)` edits that one.
    Stay(Option<String>),
    /// `None` adds a stage at the end; `Some(stage id)` edits that one.
    Stage(Option<String>),
}

pub(super) enum FormField {
    Text(Entity<TextInput>),
    Date(String),
}

impl FormField {
    fn value(&self, cx: &gpui::App) -> String {
        match self {
            Self::Text(input) => input.read(cx).text().to_string(),
            Self::Date(date) => date.clone(),
        }
    }
}

pub(super) struct Form {
    kind: FormKind,
    fields: Vec<(&'static str, FormField)>,
    mode: &'static str,
    status: &'static str,
    error: Option<SharedString>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl Form {
    fn value(&self, index: usize, cx: &gpui::App) -> String {
        self.fields[index].1.value(cx)
    }
}

/// What a finished write asks the page to read again.
enum Saved {
    Trip(String),
    Deleted(String),
    Details,
}

impl Travel {
    /// Open a reviewed trip draft on the day selected in Calendar.
    pub fn plan_calendar_day(
        &mut self,
        day: chrono::NaiveDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_route(TravelRoute::Trips, cx);
        self.open_form(FormKind::Trip(None), window, cx);
        if let Some(form) = &mut self.form {
            for index in [3, 4] {
                match &mut form.fields[index].1 {
                    FormField::Date(date) => *date = day.format("%Y-%m-%d").to_string(),
                    FormField::Text(input) => {
                        let formatted = day.format("%Y-%m-%d").to_string();
                        input.update(cx, |input, cx| input.set_text(formatted, cx));
                    }
                }
            }
        }
        cx.notify();
    }

    /// The shell, when this page reads from the Mac. Writes are offered only then.
    pub(super) fn writer(&self) -> Option<Arc<Shell>> {
        match &self.source {
            Some(Source::Shell(shell)) => Some(shell.clone()),
            _ => None,
        }
    }

    pub(super) fn open_form(
        &mut self,
        kind: FormKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.writer().is_none() {
            return;
        }
        let plan = self.current().cloned();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let (labels, values, mode, status): (Vec<(&'static str, &'static str)>, Vec<String>, _, _) =
            match &kind {
                FormKind::Trip(destination) => (
                    vec![
                        ("Title", "Hamburg in November"),
                        ("From", "Bonn"),
                        ("To", "Hamburg, Lübeck"),
                        ("Start", "YYYY-MM-DD"),
                        ("End", "YYYY-MM-DD"),
                    ],
                    vec![
                        String::new(),
                        String::new(),
                        destination.clone().unwrap_or_default(),
                        today.clone(),
                        today,
                    ],
                    "",
                    "",
                ),
                FormKind::Booking(item) => {
                    let draft = item
                        .as_deref()
                        .and_then(|id| self.item(id))
                        .map(BookingDraft::from_item)
                        .unwrap_or_else(|| BookingDraft {
                            day: plan
                                .as_ref()
                                .map(|p| p.date_start.clone())
                                .unwrap_or_default(),
                            currency: "EUR".into(),
                            ..Default::default()
                        });
                    (
                        vec![
                            ("Provider", "Deutsche Bahn"),
                            ("Reference", "Order or confirmation code"),
                            ("Title", "Optional · ICE 578 Bonn → Hamburg"),
                            ("Day", "YYYY-MM-DD, optional"),
                            ("Amount", "Optional · 49,90"),
                            ("Currency", "EUR"),
                            ("Free cancellation until", "YYYY-MM-DD, optional"),
                        ],
                        vec![
                            draft.provider,
                            draft.order_ref,
                            draft.title,
                            draft.day,
                            draft.amount,
                            draft.currency,
                            draft.free_cancellation_until,
                        ],
                        "",
                        "",
                    )
                }
                FormKind::Stay(item) => {
                    let draft = item
                        .as_deref()
                        .and_then(|id| self.item(id))
                        .map(StayDraft::from_item)
                        .unwrap_or_else(|| {
                            let dest_coord =
                                plan.as_ref().and_then(|p| p.destinations.first()).and_then(
                                    |dest| dest.coordinate().or_else(|| self.coords.get(dest)),
                                );
                            StayDraft {
                                check_in: plan
                                    .as_ref()
                                    .map(|p| p.date_start.clone())
                                    .unwrap_or_default(),
                                check_out: plan
                                    .as_ref()
                                    .map(|p| p.date_end.clone())
                                    .unwrap_or_default(),
                                latitude: dest_coord
                                    .map(|c| format!("{:.5}", c.latitude))
                                    .unwrap_or_default(),
                                longitude: dest_coord
                                    .map(|c| format!("{:.5}", c.longitude))
                                    .unwrap_or_default(),
                                currency: "EUR".into(),
                                ..Default::default()
                            }
                        });
                    (
                        vec![
                            ("Stay name", "Hotel Hafen Hamburg"),
                            ("Check-in", "YYYY-MM-DD"),
                            ("Check-out", "YYYY-MM-DD"),
                            ("Latitude", "53.5451"),
                            ("Longitude", "9.9717"),
                            ("Rank", "Optional · 1, 2, 3…"),
                            ("Total amount", "Optional · 240,00"),
                            ("Currency", "EUR"),
                            ("Provider", "Optional · booking.com"),
                            ("Booking link", "Optional · https://…"),
                            ("Address", "Optional · Seewartenstraße 9"),
                            ("Booking reference", "Optional · sets stay as booked"),
                            ("Notes", "Optional · Harbour view, quiet wing"),
                        ],
                        vec![
                            draft.title,
                            draft.check_in,
                            draft.check_out,
                            draft.latitude,
                            draft.longitude,
                            draft.rank,
                            draft.amount,
                            draft.currency,
                            draft.provider,
                            draft.url,
                            draft.address,
                            draft.order_ref,
                            draft.notes,
                        ],
                        "",
                        "",
                    )
                }
                FormKind::Stage(stage) => {
                    let draft = stage
                        .as_deref()
                        .and_then(|id| self.stage(id))
                        .map(StageDraft::from_stage)
                        .unwrap_or_else(|| {
                            let last = plan
                                .as_ref()
                                .and_then(|p| p.stages.iter().max_by_key(|s| s.sequence));
                            StageDraft {
                                origin: last
                                    .map(|s| s.destination.name.clone())
                                    .unwrap_or_default(),
                                date: plan
                                    .as_ref()
                                    .map(|p| p.date_end.clone())
                                    .unwrap_or_default(),
                                ..Default::default()
                            }
                        });
                    (
                        vec![
                            ("From", "Hamburg"),
                            ("To", "Lübeck"),
                            ("Date", "YYYY-MM-DD, optional"),
                        ],
                        vec![draft.origin, draft.destination, draft.date],
                        draft.mode,
                        draft.status,
                    )
                }
            };
        let is_date_field = |label: &str| {
            matches!(
                label,
                "Start"
                    | "End"
                    | "Day"
                    | "Check-in"
                    | "Check-out"
                    | "Date"
                    | "Free cancellation until"
            )
        };
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for ((label, placeholder), value) in labels.into_iter().zip(values) {
            if is_date_field(label) {
                fields.push((label, FormField::Date(value)));
            } else {
                let input = cx.new(|cx| {
                    let mut state = TextInput::new(window, cx).placeholder(placeholder);
                    state.set_text(value, cx);
                    state
                });
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    |view, _, event, window, cx| {
                        if let InputEvent::Submit = event {
                            view.save_form(window, cx);
                        }
                    },
                ));
                fields.push((label, FormField::Text(input)));
            }
        }
        if let Some((_, FormField::Text(first))) = fields.first() {
            first.update(cx, |input, cx| window.focus(&input.focus_handle(cx), cx));
        }
        self.form = Some(Form {
            kind,
            fields,
            mode,
            status,
            error: None,
            saving: false,
            _subscriptions: subscriptions,
        });
        self.confirm_remove = None;
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) {
        self.form = None;
        cx.notify();
    }

    fn item(&self, id: &str) -> Option<&Item> {
        let plan = self.selected.as_ref()?;
        self.details
            .get(plan)?
            .value
            .as_ref()?
            .details
            .items
            .iter()
            .find(|item| item.id == id)
    }

    fn stage(&self, id: &str) -> Option<&Stage> {
        self.current()?.stages.iter().find(|stage| stage.id == id)
    }

    fn save_form(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let (Some(shell), Some(form)) = (self.writer(), self.form.as_ref()) else {
            return;
        };
        if form.saving {
            return;
        }
        let plan = self.selected.clone().unwrap_or_default();
        let v = |i| form.value(i, cx);
        let work: Result<_, String> = match form.kind.clone() {
            FormKind::Trip(_) => TripDraft {
                title: v(0),
                origin: v(1),
                destinations: v(2),
                date_start: v(3),
                date_end: v(4),
            }
            .body()
            .map(Work::Trip),
            FormKind::Booking(item) => {
                let draft = BookingDraft {
                    provider: v(0),
                    order_ref: v(1),
                    title: v(2),
                    day: v(3),
                    amount: v(4),
                    currency: v(5),
                    free_cancellation_until: v(6),
                };
                match item {
                    None => draft.create_body().map(|body| Work::AddItem(plan, body)),
                    Some(item) => draft
                        .edit_body()
                        .map(|(fields, day)| Work::EditBooking(plan, item, fields, day)),
                }
            }
            FormKind::Stay(item) => {
                let draft = StayDraft {
                    title: v(0),
                    check_in: v(1),
                    check_out: v(2),
                    latitude: v(3),
                    longitude: v(4),
                    rank: v(5),
                    amount: v(6),
                    currency: v(7),
                    provider: v(8),
                    url: v(9),
                    address: v(10),
                    order_ref: v(11),
                    notes: v(12),
                };
                Ok(Work::SaveStay(plan, item, Box::new(draft)))
            }

            FormKind::Stage(stage) => {
                let draft = StageDraft {
                    origin: v(0),
                    destination: v(1),
                    date: v(2),
                    mode: form.mode,
                    status: form.status,
                };
                // Checked here so a typing mistake is named before anything is read from the Mac.
                draft.check().map(|()| {
                    Work::Stages(
                        plan,
                        match stage {
                            None => StageEdit::Add(draft),
                            Some(id) => StageEdit::Change(id, draft),
                        },
                    )
                })
            }
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
        self.write(shell, work, cx);
    }

    /// Removes a trip, booking or stage. The first press asks; the second removes.
    pub(super) fn remove(&mut self, target: Remove, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(shell) = self.writer() else {
            return;
        };
        let key = target.key();
        if self.confirm_remove.as_deref() != Some(key.as_str()) {
            self.confirm_remove = Some(key);
            cx.notify();
            return;
        }
        self.confirm_remove = None;
        let plan = self.selected.clone().unwrap_or_default();
        let work = match &target {
            Remove::Plan(plan) => Work::DeletePlan(plan.clone()),
            Remove::Item(item) => Work::RemoveItem(plan, item.clone()),
            Remove::Stage(stage) => Work::Stages(plan, StageEdit::Remove(stage.clone())),
        };
        self.write(shell, work, cx);
    }

    pub(super) fn set_stay_rank(
        &mut self,
        item_id: String,
        rank: Option<i64>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shell) = self.writer() else {
            return;
        };
        let Some(plan) = self.selected.clone() else {
            return;
        };
        let Ok(body) = stay_rank_body(rank) else {
            return;
        };
        self.write(shell, Work::ItemFields(plan, item_id, body), cx);
    }

    pub(super) fn add_journey_to_trip(
        &mut self,
        journey: &sjel_gpui_core::travel::connections::Journey,
        cx: &mut Context<Self>,
    ) {
        let Some(plan) = self.selected.clone() else {
            return;
        };
        let Some(shell) = self.writer() else {
            return;
        };
        match sjel_gpui_core::travel::connections::hold_journey(journey) {
            Ok(body) => self.write(shell, Work::AddItem(plan, body), cx),
            Err(why) => {
                self.write_error = Some(why);
                cx.notify();
            }
        }
    }

    fn write(&mut self, shell: Arc<Shell>, work: Work, cx: &mut Context<Self>) {
        let task = shell::run(async move {
            match work {
                Work::SaveStay(plan, item, mut draft) => {
                    if draft.needs_geocode() {
                        let query = draft.geocode_query().ok_or_else(|| {
                            ShellError::Refused(
                                400,
                                Some("Give the stay a name or address.".into()),
                            )
                        })?;
                        let coordinate = shell.places_geocode(&query).await?.ok_or_else(|| {
                            ShellError::Refused(
                                422,
                                Some(
                                    "No coordinates found. Enter latitude and longitude manually."
                                        .into(),
                                ),
                            )
                        })?;
                        draft.with_resolved_coordinates(coordinate.latitude, coordinate.longitude);
                    }
                    let invalid = |reason: String| ShellError::Refused(400, Some(reason));
                    match item {
                        None => {
                            shell
                                .trip_item_add(&plan, draft.create_body().map_err(invalid)?)
                                .await?
                        }
                        Some(item) => {
                            let (fields, day) = draft.edit_body().map_err(invalid)?;
                            shell.trip_booking_edit(&plan, &item, fields, day).await?;
                        }
                    }
                    Ok(Saved::Details)
                }
                Work::Trip(body) => shell
                    .trip_create(body)
                    .await
                    .map(|plan| Saved::Trip(plan.id)),
                Work::DeletePlan(plan) => shell
                    .trip_plan_delete(&plan)
                    .await
                    .map(|()| Saved::Deleted(plan)),
                Work::AddItem(plan, body) => shell
                    .trip_item_add(&plan, body)
                    .await
                    .map(|()| Saved::Details),
                Work::EditBooking(plan, item, fields, day) => shell
                    .trip_booking_edit(&plan, &item, fields, day)
                    .await
                    .map(|()| Saved::Details),
                Work::ItemFields(plan, item, body) => shell
                    .trip_item_fields(&plan, &item, body)
                    .await
                    .map(|()| Saved::Details),
                Work::RemoveItem(plan, item) => shell
                    .trip_item_delete(&plan, &item)
                    .await
                    .map(|()| Saved::Details),
                Work::Stages(plan, edit) => shell
                    .trip_stages(&plan, &edit)
                    .await
                    .map(|()| Saved::Details),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(saved) => {
                        view.form = None;
                        view.write_error = None;
                        // The list carries stages and dates, so it is read again after any write.
                        view.loaded = false;
                        match saved {
                            Saved::Trip(id) => view.open_route(
                                TravelRoute::Trip {
                                    id,
                                    section: TripSection::Itinerary,
                                },
                                cx,
                            ),
                            Saved::Deleted(id) => {
                                view.details.remove(&id);
                                view.selected = None;
                                view.open_route(TravelRoute::Trips, cx);
                            }
                            Saved::Details => {
                                view.refresh(cx);
                                if let Some(id) = view.selected.clone() {
                                    view.read_details(id, true, cx);
                                }
                            }
                        }
                    }
                    Err(error) => match view.form.as_mut() {
                        Some(form) => {
                            form.saving = false;
                            form.error = Some(error.to_string().into());
                        }
                        None => view.write_error = Some(error.to_string()),
                    },
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The open form, when `kind` is the one open. Drawn where its button is.
    pub(super) fn form_card(&self, kind: &FormKind, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.form.as_ref().filter(|form| &form.kind == kind)?;
        let palette = self.palette;
        let title = match &form.kind {
            FormKind::Trip(Some(destination)) => format!("Plan a trip to {destination}"),
            FormKind::Trip(None) => "New trip".to_owned(),
            FormKind::Booking(None) => "Add a booking".to_owned(),
            FormKind::Booking(Some(_)) => "Edit booking".to_owned(),
            FormKind::Stay(None) => "Add stay candidate".to_owned(),
            FormKind::Stay(Some(_)) => "Edit stay candidate".to_owned(),
            FormKind::Stage(None) => "Add a stage".to_owned(),
            FormKind::Stage(Some(_)) => "Edit stage".to_owned(),
        };
        let mut card = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(10.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(title),
            );
        for (index, (label, field)) in form.fields.iter().enumerate() {
            let control = match field {
                FormField::Text(input) => {
                    Input::new(input).size(ControlSize::Sm).into_any_element()
                }
                FormField::Date(date) => {
                    let view = cx.weak_entity();
                    DatePicker::new(("travel-form-date", index), date.parse().ok())
                        .size(ControlSize::Sm)
                        .on_change(move |date, _, cx| {
                            let _ = view.update(cx, |view, cx| {
                                if let Some(form) = &mut view.form {
                                    if let FormField::Date(value) = &mut form.fields[index].1 {
                                        *value = date.to_string();
                                    }
                                }
                                cx.notify();
                            });
                        })
                        .into_any_element()
                }
            };
            card =
                card.child(ElyFormField::new(("travel-form-field", index), *label).child(control));
        }
        if matches!(form.kind, FormKind::Stage(_)) {
            card = card
                .child(self.chips(
                    "stage-mode",
                    TRANSPORT_MODES,
                    form.mode,
                    |form, key| form.mode = key,
                    cx,
                ))
                .child(self.chips(
                    "stage-status",
                    STAGE_STATUSES,
                    form.status,
                    |form, key| form.status = key,
                    cx,
                ));
        }
        if matches!(&form.kind, FormKind::Trip(_)) {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("One stage is made per destination, in the order typed. Dates and modes are set per stage afterwards."),
            );
        }
        Some(
            ElyForm::new("travel-form")
                .child(
                    card.when_some(form.error.clone(), |card, why| {
                        card.child(FormError::new("travel-form-error", why))
                    })
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new(
                                    "travel-form-save",
                                    if form.saving { "Saving…" } else { "Save" },
                                )
                                .primary()
                                .disabled(form.saving)
                                .on_click(
                                    cx.listener(|view, _, window, cx| view.save_form(window, cx)),
                                ),
                            )
                            .child(
                                Button::new("travel-form-cancel", "Cancel")
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(|view, _, _, cx| view.close_form(cx))),
                            ),
                    ),
                )
                .into_any_element(),
        )
    }

    fn chips(
        &self,
        id: &'static str,
        options: &'static [(&'static str, &'static str)],
        on: &'static str,
        set: fn(&mut Form, &'static str),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut control = SegmentedControl::new(id, on).size(ControlSize::Sm);
        for (key, label) in options {
            control = control.segment(*key, *label, None);
        }
        control
            .on_change(cx.listener(move |view, value: &SharedString, _, cx| {
                if let Some((key, _)) = options.iter().find(|(key, _)| *key == value.as_ref()) {
                    if let Some(form) = view.form.as_mut() {
                        set(form, key);
                    }
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    /// A button that opens `kind`'s form, absent when this page cannot write.
    pub(super) fn form_button(
        &self,
        id: impl Into<gpui::ElementId>,
        label: &'static str,
        kind: FormKind,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.writer()?;
        let open = self.form.as_ref().is_some_and(|form| {
            form.kind == kind
                || (matches!(&kind, FormKind::Trip(_)) && matches!(&form.kind, FormKind::Trip(_)))
        });
        let button = Button::new(id, label).disabled(open);
        let button = if matches!(&kind, FormKind::Trip(None)) {
            button.primary()
        } else {
            button
        };
        Some(
            button
                .on_click(
                    cx.listener(move |view, _, window, cx| {
                        view.open_form(kind.clone(), window, cx)
                    }),
                )
                .into_any_element(),
        )
    }

    /// The trip form is shown below the map when opened from a location row or marker.
    pub(super) fn active_trip_form_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let kind = self.form.as_ref()?.kind.clone();
        matches!(&kind, FormKind::Trip(_))
            .then(|| self.form_card(&kind, cx))
            .flatten()
    }

    /// `Remove`, then `Remove — press again` on the same row.
    pub(super) fn remove_button(
        &self,
        target: Remove,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.writer()?;
        let key = target.key();
        let armed = self.confirm_remove.as_deref() == Some(key.as_str());
        let (label, armed_label) = match &target {
            Remove::Plan(_) => ("Delete trip", "Press again to delete trip"),
            _ => ("Remove", "Press again to remove"),
        };
        Some(
            Button::new(
                SharedString::from(format!("remove-{key}")),
                if armed { armed_label } else { label },
            )
            .variant(ButtonVariant::Ghost)
            .on_click(
                cx.listener(move |view, _, window, cx| view.remove(target.clone(), window, cx)),
            )
            .into_any_element(),
        )
    }
}

#[derive(Clone, Debug)]
pub(super) enum Remove {
    Plan(String),
    Item(String),
    Stage(String),
}

impl Remove {
    fn key(&self) -> String {
        match self {
            Self::Plan(id) => format!("plan-{id}"),
            Self::Item(id) => format!("item-{id}"),
            Self::Stage(id) => format!("stage-{id}"),
        }
    }
}

enum Work {
    Trip(serde_json::Value),
    DeletePlan(String),
    AddItem(String, serde_json::Value),
    SaveStay(String, Option<String>, Box<StayDraft>),
    EditBooking(String, String, serde_json::Value, Option<String>),
    ItemFields(String, String, serde_json::Value),
    RemoveItem(String, String),
    Stages(String, StageEdit),
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{size, TestAppContext};

    #[gpui::test]
    fn trip_fields_keep_calendar_dates_and_submit_validation(cx: &mut TestAppContext) {
        let (view, cx) =
            crate::test_support::mount_root(cx, Some(size(px(1100.), px(900.))), |window, cx| {
                let shell = Arc::new(Shell::new("http://127.0.0.1:1").unwrap());
                let mut view = Travel::new(Some(Source::Shell(shell)), crate::Density::Desktop, cx);
                view.loaded = true;
                view.map_enabled = false;
                view.trip_drafts_loading = true;
                view.plan_calendar_day(
                    chrono::NaiveDate::from_ymd_opt(2026, 12, 10).unwrap(),
                    window,
                    cx,
                );
                view
            });
        view.read_with(cx, |view, cx| {
            let form = view.form.as_ref().unwrap();
            assert!(matches!(form.fields[3].1, FormField::Date(_)));
            assert!(matches!(form.fields[4].1, FormField::Date(_)));
            assert_eq!(form.value(3, cx), "2026-12-10");
            assert_eq!(form.value(4, cx), "2026-12-10");
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            let form = view.form.as_ref().unwrap();
            assert!(form.error.is_some());
            assert!(!form.saving);
        });
        cx.simulate_input("Winter trip");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.form.as_ref().unwrap().value(0, cx), "Winter trip");
        });
    }
}
