//! Writing a trip from the app: a new trip, a booking, a stage. One form card serves all three.
//!
//! Writes go through the Mac's shell only. A phone holds a read copy of trips and has no outbox
//! for them, so it offers no control the Mac would have to refuse.
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, SharedString, Subscription, Window,
};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::{
    shell::{self, Shell, ShellError},
    travel::{
        edit::{BookingDraft, StageDraft, StageEdit, TripDraft, STAGE_STATUSES, TRANSPORT_MODES},
        Item, Source, Stage, TravelRoute, TripSection,
    },
};

use super::Travel;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FormKind {
    Trip,
    /// `None` adds a booking; `Some(item id)` edits that one.
    Booking(Option<String>),
    /// `None` adds a stage at the end; `Some(stage id)` edits that one.
    Stage(Option<String>),
}

pub(super) struct Form {
    kind: FormKind,
    fields: Vec<(&'static str, Entity<InputState>)>,
    mode: &'static str,
    status: &'static str,
    error: Option<SharedString>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl Form {
    fn value(&self, index: usize, cx: &gpui::App) -> String {
        self.fields[index].1.read(cx).value().to_string()
    }
}

/// What a finished write asks the page to read again.
enum Saved {
    Trip(String),
    Details,
}

impl Travel {
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
                FormKind::Trip => (
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
                        String::new(),
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
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for ((label, placeholder), value) in labels.into_iter().zip(values) {
            let input = cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder(placeholder);
                state.set_value(value, window, cx);
                state
            });
            subscriptions.push(
                cx.subscribe_in(&input, window, |view, _, event, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        view.save_form(window, cx);
                    }
                }),
            );
            fields.push((label, input));
        }
        if let Some((_, first)) = fields.first() {
            first.update(cx, |input, cx| input.focus(window, cx));
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

    fn save_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(shell), Some(form)) = (self.writer(), self.form.as_ref()) else {
            return;
        };
        if form.saving {
            return;
        }
        let plan = self.selected.clone().unwrap_or_default();
        let v = |i| form.value(i, cx);
        let work: Result<_, String> = match form.kind.clone() {
            FormKind::Trip => TripDraft {
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
                    None => draft.create_body().map(|body| Work::AddBooking(plan, body)),
                    Some(item) => draft
                        .edit_body()
                        .map(|(fields, day)| Work::EditBooking(plan, item, fields, day)),
                }
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
        self.write(shell, work, window, cx);
    }

    /// Removes a booking or a stage. The first press asks; the second removes.
    pub(super) fn remove(&mut self, target: Remove, window: &mut Window, cx: &mut Context<Self>) {
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
        let work = match target {
            Remove::Item(item) => Work::RemoveItem(plan, item),
            Remove::Stage(stage) => Work::Stages(plan, StageEdit::Remove(stage)),
        };
        self.write(shell, work, window, cx);
    }

    fn write(&mut self, shell: Arc<Shell>, work: Work, _: &mut Window, cx: &mut Context<Self>) {
        let task = shell::run(async move {
            match work {
                Work::Trip(body) => shell
                    .trip_create(body)
                    .await
                    .map(|plan| Saved::Trip(plan.id)),
                Work::AddBooking(plan, body) => shell
                    .trip_booking_add(&plan, body)
                    .await
                    .map(|()| Saved::Details),
                Work::EditBooking(plan, item, fields, day) => shell
                    .trip_booking_edit(&plan, &item, fields, day)
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
            FormKind::Trip => "New trip",
            FormKind::Booking(None) => "Add a booking",
            FormKind::Booking(Some(_)) => "Edit booking",
            FormKind::Stage(None) => "Add a stage",
            FormKind::Stage(Some(_)) => "Edit stage",
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
        for (label, input) in &form.fields {
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(*label))
                    .child(Input::new(input).small()),
            );
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
        if matches!(form.kind, FormKind::Trip) {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("One stage is made per destination, in the order typed. Dates and modes are set per stage afterwards."),
            );
        }
        Some(
            card.when_some(form.error.clone(), |card, why| {
                card.child(div().text_xs().text_color(rgb(palette.red)).child(why))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        self.button("travel-form-save")
                            .primary()
                            .label(if form.saving { "Saving…" } else { "Save" })
                            .disabled(form.saving)
                            .on_click(
                                cx.listener(|view, _, window, cx| view.save_form(window, cx)),
                            ),
                    )
                    .child(
                        self.button("travel-form-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|view, _, _, cx| view.close_form(cx))),
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
        let mut row = div().flex().flex_wrap().gap_1();
        for (key, label) in options {
            let key: &'static str = key;
            row = row.child(
                self.button(SharedString::from(format!("{id}-{key}")))
                    .label(*label)
                    .when(key == on, |button| button.primary())
                    .when(key != on, |button| button.ghost())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(form) = view.form.as_mut() {
                            set(form, key);
                        }
                        cx.notify();
                    })),
            );
        }
        row.into_any_element()
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
        let open = self.form.as_ref().is_some_and(|form| form.kind == kind);
        Some(
            self.button(id)
                .label(label)
                .disabled(open)
                .on_click(
                    cx.listener(move |view, _, window, cx| {
                        view.open_form(kind.clone(), window, cx)
                    }),
                )
                .into_any_element(),
        )
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
        Some(
            self.button(SharedString::from(format!("remove-{key}")))
                .ghost()
                .label(if armed {
                    "Press again to remove"
                } else {
                    "Remove"
                })
                .on_click(
                    cx.listener(move |view, _, window, cx| view.remove(target.clone(), window, cx)),
                )
                .into_any_element(),
        )
    }
}

#[derive(Clone, Debug)]
pub(super) enum Remove {
    Item(String),
    Stage(String),
}

impl Remove {
    fn key(&self) -> String {
        match self {
            Self::Item(id) => format!("item-{id}"),
            Self::Stage(id) => format!("stage-{id}"),
        }
    }
}

enum Work {
    Trip(serde_json::Value),
    AddBooking(String, serde_json::Value),
    EditBooking(String, String, serde_json::Value, Option<String>),
    RemoveItem(String, String),
    Stages(String, StageEdit),
}
