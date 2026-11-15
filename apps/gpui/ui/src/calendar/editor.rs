//! Application-owned choices and persistence around Ely's event editor.
use super::types::Form;
use super::{
    adapter,
    page::CalendarPage,
    types::{muted, CalendarIntent},
};
use crate::ely::calendar::{EventDraft, EventEditor, When};
use crate::Palette;
use gpui::Window;
use gpui::{div, prelude::*, AnyElement, Context, SharedString};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use sjel_gpui_core::{
    calendar::{COMMITMENTS, KINDS},
    shell::{self, CalendarEntry, ShellError},
};

impl CalendarPage {
    pub(super) fn open_form(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let start = format!("{}T09:00:00", self.day);
        let end = format!("{}T10:00:00", self.day);
        match (
            adapter::timestamp(&start, &self.zone),
            adapter::timestamp(&end, &self.zone),
        ) {
            (Ok(start), Ok(end)) => self.open_draft(When::Timed { start, end }, None, cx),
            _ => {
                self.write_error = Some(ShellError::Refused(
                    400,
                    Some("Choose a valid local time.".into()),
                ));
                cx.notify();
            }
        }
    }

    pub(super) fn open_draft(
        &mut self,
        when: When,
        editing: Option<CalendarEntry>,
        cx: &mut Context<Self>,
    ) {
        self.form_session = self.form_session.wrapping_add(1);
        let draft = match editing.as_ref() {
            Some(entry) => match adapter::draft(entry, &self.zone) {
                Ok(draft) => draft,
                Err(error) => {
                    self.write_error = Some(ShellError::Refused(400, Some(error)));
                    cx.notify();
                    return;
                }
            },
            None => EventDraft {
                title: "".into(),
                when,
                place: "".into(),
                hue: 0,
                repeat: None,
                reminders: vec![],
                notes: "".into(),
            },
        };
        self.form = Some(Form {
            kind: editing
                .as_ref()
                .map_or("event".into(), |entry| entry.kind.clone()),
            commitment: editing
                .as_ref()
                .map_or("planned".into(), |entry| entry.commitment.clone()),
            editing,
            draft,
            session: self.form_session,
        });
        self.write_error = None;
        cx.notify();
    }

    pub(super) fn edit_selected(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self
            .day_entries()
            .into_iter()
            .find(|entry| Some(&entry.id) == self.selected_entry.as_ref())
        {
            if let Ok(event) = adapter::event(&entry, &self.zone, None) {
                self.open_draft(event.when, Some(entry), cx);
            }
        }
    }

    pub(super) fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.changing_time {
            return;
        }
        self.form = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn save(&mut self, draft: &EventDraft, cx: &mut Context<Self>) {
        if self.changing_time {
            return;
        }
        let Some(form) = self.form.as_ref() else {
            return;
        };
        let original = form.editing.clone();
        let body = match adapter::patch(draft, &form.kind, &form.commitment, &self.zone) {
            Ok(body) => body,
            Err(reason) => {
                self.write_error = Some(ShellError::Refused(400, Some(reason)));
                cx.notify();
                return;
            }
        };
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.write_error = Some(error);
                cx.notify();
                return;
            }
        };
        self.write_error = None;
        self.changing_time = true;
        let previous = original.clone();
        let work = shell::run(async move {
            match original {
                Some(entry) => shell.update_calendar_entry(&entry.id, body).await,
                None => shell.create_calendar_entry(body).await,
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.changing_time = false;
                match result {
                    Ok(entry) => {
                        if let Some(before) = previous {
                            view.undo_time = Some((before, entry.clone()));
                        }
                        view.form = None;
                        view.focus_next = true;
                        view.selected_entry = Some(entry.id);
                        view.selected_detail = None;
                        view.refresh(cx);
                    }
                    Err(error) => view.write_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn form_panel(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = &self.form else {
            return div().into_any_element();
        };
        let kinds =
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(KINDS.iter().map(|(key, label, _)| {
                    let on = form.kind == *key;
                    Button::new(SharedString::from(format!("calendar-kind-{key}")))
                        .xsmall()
                        .label(*label)
                        .when(on, |b| b.primary())
                        .when(!on, |b| b.ghost())
                        .on_click(cx.listener(move |page, _, _, cx| {
                            if let Some(form) = page.form.as_mut() {
                                form.kind = (*key).into();
                            }
                            cx.notify();
                        }))
                }));
        let commitments = div()
            .flex()
            .gap_1()
            .children(COMMITMENTS.iter().map(|(key, label)| {
                let on = form.commitment == *key;
                Button::new(SharedString::from(format!("calendar-commitment-{key}")))
                    .xsmall()
                    .label(*label)
                    .when(on, |b| b.primary())
                    .when(!on, |b| b.ghost())
                    .on_click(cx.listener(move |page, _, _, cx| {
                        if let Some(form) = page.form.as_mut() {
                            form.commitment = (*key).into();
                        }
                        cx.notify();
                    }))
            }));
        let extra = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(muted("Kind", palette))
            .child(kinds)
            .child(muted("Commitment", palette))
            .child(commitments)
            .when(self.show_availability && form.editing.is_none(), |panel| {
                panel.child(
                    Button::new("calendar-gap-travel")
                        .small()
                        .ghost()
                        .label("Plan travel on this day")
                        .on_click(cx.listener(|page, _, _, cx| {
                            cx.emit(CalendarIntent::PlanTravel { day: page.day })
                        })),
                )
            });
        let cancel = cx.entity().downgrade();
        div()
            .key_context("CalendarForm")
            .child(
                EventEditor::new(
                    SharedString::from(format!("calendar-editor-{}", form.session)),
                    form.draft.clone(),
                )
                .zone(self.zone.clone())
                .autofocus(true)
                .scheduling(false)
                .clamp_end_date(true)
                .pending(self.changing_time)
                .error(
                    self.write_error
                        .as_ref()
                        .map(|error| error.to_string().into()),
                )
                .extra(extra)
                .on_save(cx.listener(|page, draft, _, cx| page.save(draft, cx)))
                .on_cancel(move |window, cx| {
                    let _ = cancel.update(cx, |page, cx| page.close_form(window, cx));
                }),
            )
            .into_any_element()
    }
}
