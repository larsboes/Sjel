//! Native People page (`/people`): the contact registry, and the forms that write it.
//!
//! Reads go through the shell (`GET /entities/api/entities?kind=person`), writes through the same
//! shell. The field list is the registry's own (`GET /entities/api/fields?kind=person`), so a form
//! never offers a key the capability would refuse.

mod actions;
mod detail;
mod forms;
mod register;

use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, point, prelude::*, px, rgb, rgba, Context, Entity, FocusHandle, FontWeight, IntoElement,
    Render, ScrollHandle, SharedString, Subscription, Window,
};
use sjel_gpui_core::people::{filter_people, Person};
use sjel_gpui_core::shell::{self, DuplicatePair, EntityField, EntityRecord, Shell, ShellError};

use crate::data::{Remote, Repo};
use crate::primitives::{key_bar, virtual_list};
use crate::theme::{Density, Palette, ThemeContext};

use actions::*;
use forms::{Form, FormKind};

pub use actions::bind_keys;
pub use register::*;

pub mod page {
    pub use super::*;
}

#[derive(Default)]
pub(super) struct PeopleDerived {
    pub(super) people_at: Option<Option<Instant>>,
    pub(super) parsed: Vec<Person>,
    pub(super) filter_key: Option<(Option<Instant>, u64)>,
    pub(super) filtered_indices: Vec<usize>,
}

pub struct PeoplePage {
    pub(super) repo: Repo,
    palette: Option<Palette>,
    density: Density,
    pub people: Remote<Vec<EntityRecord>>,
    /// The fields the registry declares for a person: what a form may offer.
    pub fields: Remote<Vec<EntityField>>,
    /// Probable duplicates, read when the Duplicates view opens.
    pub(super) duplicates: Remote<Vec<DuplicatePair>>,
    pub(super) show_duplicates: bool,
    /// The open form, when one is.
    pub(super) form: Option<Form>,
    /// The person whose removal the next press confirms.
    pub(super) confirm_remove: Option<String>,
    /// A write that failed with no form open to show it.
    pub(super) write_error: Option<SharedString>,
    pub selected_id: Option<String>,
    pub search_query: String,
    filter_rev: u64,
    pub(super) derived: PeopleDerived,
    pub(super) people_list: gpui::ListState,
    search_input: Option<(Entity<InputState>, Subscription)>,
    pub(super) form_scroll: ScrollHandle,
    pub(super) detail_scroll: ScrollHandle,
    focus: FocusHandle,
    focus_next: bool,
}

impl PeoplePage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self::with_density(repo, Density::Desktop, cx)
    }

    pub fn with_density(repo: Repo, density: Density, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            density,
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

    pub fn set_density(&mut self, density: Density, cx: &mut Context<Self>) {
        if self.density != density {
            self.density = density;
            cx.notify();
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

    pub(super) fn sync_derived(&mut self) {
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

    pub(super) fn load_duplicates(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn load_fields(&mut self, shell: Arc<Shell>, cx: &mut Context<Self>) {
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

    pub(super) fn person(&mut self, id: &str) -> Option<Person> {
        self.sync_derived();
        self.derived
            .parsed
            .iter()
            .find(|person| person.id == id)
            .cloned()
    }

    pub(super) fn search_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
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
                                    .child(initials_badge(&initials, palette))
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
            .when(!self.density.is_mobile(), |el| {
                el.child(key_bar(
                    &[
                        ("↓ / ↑ or j / k", "navigate"),
                        ("/", "search"),
                        ("⌘r", "refresh"),
                    ],
                    palette,
                ))
            })
    }
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
