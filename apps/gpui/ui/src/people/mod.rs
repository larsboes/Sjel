//! Native People page (`/people`): the contact registry, and the forms that write it.
//!
//! Reads go through the shell (`GET /entities/api/entities?kind=person`), writes through the same
//! shell. The field list is the registry's own (`GET /entities/api/fields?kind=person`), so a form
//! never offers a key the capability would refuse.

mod actions;
mod detail;
mod forms;
mod network;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::forms::{Input, InputEvent, TextInput};
use crate::ely::layout::Sheet;
use crate::ely::primitives::Place;
use crate::ely::theme::ControlSize;
use gpui_kit::{
    div, point, prelude::*, px, rgb, rgba, Context, Entity, FocusHandle, FontWeight, IntoElement,
    Render, ScrollHandle, SharedString, Subscription, Window,
};
use sjel_gpui_core::people::graph::{company_key, Layout};
use sjel_gpui_core::people::{view_people, Order, Origin, Person};
use sjel_gpui_core::shell::{self, DuplicatePair, EntityField, EntityRecord, Shell, ShellError};

use crate::data::{Remote, Repo};
use crate::primitives::{key_bar, sub_tabs, virtual_list};
use crate::theme::{Density, Palette, ThemeContext};

use actions::*;
use forms::{Form, FormKind};

pub use actions::bind_keys;

pub mod page {
    pub use super::*;
}

#[derive(Default)]
pub(super) struct PeopleDerived {
    pub(super) people_at: Option<Option<Instant>>,
    pub(super) parsed: Vec<Person>,
    pub(super) filter_key: Option<(Option<Instant>, u64, chrono::NaiveDate)>,
    pub(super) filtered_indices: Vec<usize>,
    /// People under each `Origin::ALL` entry, before the search narrows them.
    pub(super) origin_counts: [usize; 4],
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
    pub(super) show_network: bool,
    pub(super) show_ego_graph: bool,
    pub(super) network_connected_only: bool,
    graph: Remote<Rc<Layout>>,
    graph_request: u64,
    network_bounds: Rc<Cell<Option<gpui_kit::Bounds<gpui_kit::Pixels>>>>,
    /// The open form, when one is.
    pub(super) form: Option<Form>,
    /// The person whose removal the next press confirms.
    pub(super) confirm_remove: Option<String>,
    confirm_unlink: Option<(String, String)>,
    /// A write that failed with no form open to show it.
    pub(super) write_error: Option<SharedString>,
    pub selected_id: Option<String>,
    pub search_query: String,
    pub(super) origin: Origin,
    pub(super) order: Order,
    filter_rev: u64,
    pub(super) derived: PeopleDerived,
    pub(super) people_list: gpui::ListState,
    search_input: Option<(Entity<TextInput>, Subscription)>,
    company_input: Option<(Entity<TextInput>, Subscription)>,
    company_query: String,
    overdue_only: bool,
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
            show_network: false,
            show_ego_graph: true,
            network_connected_only: false,
            graph: Remote::default(),
            graph_request: 0,
            network_bounds: Default::default(),
            form: None,
            confirm_remove: None,
            confirm_unlink: None,
            write_error: None,
            selected_id: None,
            search_query: String::new(),
            origin: Origin::All,
            order: Order::Name,
            filter_rev: 0,
            derived: PeopleDerived::default(),
            people_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(600.)),
            search_input: None,
            company_input: None,
            company_query: String::new(),
            overdue_only: false,
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
            let parsed = &self.derived.parsed;
            self.derived.origin_counts =
                Origin::ALL.map(|o| parsed.iter().filter(|p| p.has_origin(o)).count());
        }
        let today = chrono::Local::now().date_naive();
        let filter_key = (self.people.loaded_at, self.filter_rev, today);
        if self.derived.filter_key != Some(filter_key) {
            self.derived.filter_key = Some(filter_key);
            let filtered = view_people(
                &self.derived.parsed,
                &self.search_query,
                if self.show_network {
                    Origin::All
                } else {
                    self.origin
                },
                self.order,
            )
            .into_iter()
            .filter(|person| {
                self.show_network
                    || ((!self.overdue_only || person.is_overdue(today))
                        && (self.company_query.is_empty()
                            || person.company.as_deref().is_some_and(|company| {
                                company_key(company).contains(&self.company_query)
                            })))
            });
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

    /// Shows the people under `origin`, in `order`.
    pub(super) fn set_view(&mut self, origin: Origin, order: Order, cx: &mut Context<Self>) {
        if (self.origin, self.order) != (origin, order) {
            self.origin = origin;
            self.order = order;
            self.filter_rev = self.filter_rev.wrapping_add(1);
            cx.notify();
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

    pub fn active_tab_index(&self) -> usize {
        if self.show_duplicates {
            1
        } else if self.show_network {
            2
        } else {
            0
        }
    }

    pub fn badge_counts(&self) -> (usize, Option<usize>) {
        let total = self.derived.parsed.len();
        let dup = self.duplicates.value.as_ref().map(Vec::len);
        (total, dup)
    }

    pub fn set_tab_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        let (dup, net) = (ix == 1, ix == 2);
        if (self.show_duplicates, self.show_network) != (dup, net) {
            self.show_duplicates = dup;
            self.show_network = net;
            self.form = None;
            self.filter_rev = self.filter_rev.wrapping_add(1);
            if dup {
                self.load_duplicates(cx);
            }
            if net && !self.graph.loading && self.graph.value.is_none() {
                self.load_graph(cx);
            }
            cx.notify();
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.load_graph(cx);
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
                    if !page.show_network
                        && page
                            .selected_id
                            .as_ref()
                            .is_none_or(|id| !records.iter().any(|record| &record.id == id))
                    {
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

    pub(super) fn select_person(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if self.selected_id != id {
            self.selected_id = id;
            self.form = None;
            self.confirm_remove = None;
            self.confirm_unlink = None;
            self.detail_scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        }
    }

    fn company_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some((input, _)) = &self.company_input {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("Filter by company…"));
        let subscription = cx.subscribe_in(&input, window, |page, input, event, _, cx| {
            if matches!(event, InputEvent::Changed) {
                page.company_query = company_key(input.read(cx).text());
                page.filter_rev = page.filter_rev.wrapping_add(1);
                cx.notify();
            }
        });
        self.company_input = Some((input.clone(), subscription));
        input
    }

    pub(super) fn search_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new(window, cx).placeholder("Search people…"));
        let subscription =
            cx.subscribe_in(&input, window, |view, input, event, _, cx| match event {
                InputEvent::Changed => {
                    view.search_query = input.read(cx).text().trim().to_lowercase();
                    view.filter_rev = view.filter_rev.wrapping_add(1);
                    if view.show_network && !view.search_query.is_empty() {
                        view.sync_derived();
                        let first = view
                            .derived
                            .filtered_indices
                            .first()
                            .map(|&i| view.derived.parsed[i].id.clone());
                        view.select_person(first, cx);
                    }
                    cx.notify();
                }
                InputEvent::Submit if view.show_network => view.step_selection(true, cx),
                _ => {}
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

        let dup_count = self.duplicates.value.as_ref().map(Vec::len);
        let tab_entity = cx.entity();

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .when(cfg!(test), |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(palette.text))
                                .child("People"),
                        )
                        .child(sub_tabs(
                            "people-view-tabs",
                            vec![
                                (
                                    "Directory",
                                    Some(total_contacts),
                                    !self.show_duplicates && !self.show_network,
                                ),
                                ("Duplicates", dup_count, self.show_duplicates),
                                ("Network", None, self.show_network),
                            ],
                            move |ix, _, cx| {
                                tab_entity.update(cx, |page, cx| page.set_tab_index(ix, cx));
                            },
                        )),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("people-new", "New person")
                            .size(ControlSize::Sm)
                            .primary()
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.open_form(FormKind::New, window, cx)
                            })),
                    )
                    .child(
                        Button::new("people-declare-field", "Declare field")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.open_form(FormKind::Field, window, cx)
                            })),
                    )
                    .child(
                        Button::new("people-refresh", "↻ Refresh")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                    ),
            );

        let write_error = self.write_error.clone();

        let order = self.order;
        let today = chrono::Local::now().date_naive();
        let search = self.search_field(window, cx);
        let view_entity = cx.entity();
        let search_box = div()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&search).size(ControlSize::Sm)),
            )
            .child(
                Button::new("people-order", format!("↕ {}", order.label()))
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.set_view(page.origin, order.next(), cx)
                    })),
            );
        let origin_tabs = sub_tabs(
            "people-origin-tabs",
            Origin::ALL
                .iter()
                .zip(self.derived.origin_counts)
                .map(|(o, n)| (o.label(), Some(n), *o == self.origin))
                .collect(),
            move |ix, _, cx| {
                view_entity.update(cx, |page, cx| {
                    let origin = Origin::ALL.get(ix).copied().unwrap_or_default();
                    page.set_view(origin, page.order, cx);
                });
            },
        );

        // Left List Pane
        let mut list_pane =
            div()
                .w(px(320.))
                .min_h_0()
                .min_w_0()
                .flex_none()
                .flex()
                .flex_col()
                .gap_2()
                .child(search_box)
                .child(origin_tabs)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(
                            Input::new(&self.company_field(window, cx)).size(ControlSize::Sm),
                        ))
                        .child(
                            Button::new("people-overdue", "Overdue only")
                                .size(ControlSize::Sm)
                                .variant(if self.overdue_only {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Ghost
                                })
                                .on_click(cx.listener(|page, _, _, cx| {
                                    page.overdue_only = !page.overdue_only;
                                    page.filter_rev = page.filter_rev.wrapping_add(1);
                                    cx.notify();
                                })),
                        ),
                );

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
                        let photo = person.photo.clone();
                        let name = person.name.clone();
                        let subtitle = [person.company.as_deref(), person.role.as_deref()]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ");
                        let overdue = person.is_overdue(today);
                        let link_count = page
                            .graph
                            .value
                            .as_ref()
                            .and_then(|g| g.nodes.iter().position(|n| n.id == pid).map(|i| g.links_of(i).count()))
                            .unwrap_or(0);
                        let day = page
                            .order
                            .key()
                            .and_then(|key| person.values.get(key).cloned());

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
                                        page.select_person(Some(pid.clone()), cx);
                                    }))
                                    .child(avatar_badge(&initials, photo.as_deref(), 32.0, palette))
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
                                    )
                                    .when(link_count > 0 && !overdue, |el| {
                                        el.child(
                                            div()
                                                .flex_none()
                                                .px_1p5()
                                                .py(px(0.5))
                                                .rounded(px(4.))
                                                .bg(rgba(crate::tint(palette.accent, 0x18)))
                                                .text_xs()
                                                .text_color(rgb(palette.accent))
                                                .child(format!("{link_count}")),
                                        )
                                    })
                                    .when(overdue, |el| {
                                        el.child(
                                            div()
                                                .flex_none()
                                                .px_1p5()
                                                .py(px(0.5))
                                                .rounded(px(4.))
                                                .bg(rgba(crate::tint(palette.amber, 0x1e)))
                                                .text_xs()
                                                .text_color(rgb(palette.amber))
                                                .child("Overdue"),
                                        )
                                    })
                                    .when_some(day, |el, day| {
                                        el.child(
                                            div()
                                                .flex_none()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .child(day),
                                        )
                                    }),
                            )
                            .into_any_element()
                    })
                },
            ));
        }

        // Right Detail Pane
        let detail_pane = if self.show_duplicates {
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

        let form_sheet_title = self.form.as_ref().map(|f| match f.kind {
            FormKind::New => "New Person",
            FormKind::Edit(_) => "Edit Person",
            FormKind::Fact(_) => "Add Place Fact",
            FormKind::Introduction(_) => "Record Introduction",
            FormKind::Field => "Declare Registry Field",
        });
        let form_sheet = self
            .render_form(palette, window, cx)
            .zip(form_sheet_title)
            .map(|(form_el, title)| {
                let close_entity = cx.entity();
                Sheet::new("people-form-sheet", Place::Right, title, move |_, cx| {
                    close_entity.update(cx, |page, cx| page.close_form(cx));
                })
                .child(form_el)
            });

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
                } else if page.confirm_unlink.take().is_some() {
                    cx.notify();
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
                    .map(|body| {
                        if self.show_network {
                            body.child(self.render_network(&search, palette, cx))
                                .child(div().w(px(320.)).flex_none().flex().child(detail_pane))
                        } else {
                            body.child(list_pane).child(detail_pane)
                        }
                    }),
            )
            .children(form_sheet)
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
    fn network_node_click_and_keyboard_move_the_contact_inspector(cx: &mut TestAppContext) {
        use sjel_gpui_core::people::graph::{layout, EntityEdge, GraphNode, PeopleGraph};
        let (view, cx) =
            crate::test_support::mount_root(cx, Some(gpui::size(px(1100.), px(800.))), |_, cx| {
                let mut page = PeoplePage::new(Repo::new(Err(ShellError::NotSetUp)), cx);
                let records: Vec<_> = [("ada", "Ada Lovelace"), ("grace", "Grace Hopper")]
                    .into_iter()
                    .map(|(id, name)| EntityRecord {
                        id: id.into(),
                        name: name.into(),
                        kind: "person".into(),
                        ..Default::default()
                    })
                    .collect();
                let nodes = records
                    .iter()
                    .map(|p| GraphNode {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        kind: p.kind.clone(),
                        company: Some("Computing".into()),
                    })
                    .collect();
                let edges = vec![EntityEdge {
                    a: "ada".into(),
                    b: "grace".into(),
                    kind: "introduced".into(),
                    source: "operator".into(),
                    at: None,
                    note: None,
                }];
                page.people.settle(Ok(records));
                page.graph
                    .settle(Ok(Rc::new(layout(&PeopleGraph { nodes, edges }))));
                page.selected_id = Some("ada".into());
                page
            });
        cx.update(|_, cx| bind_keys(cx));
        let tabs = cx
            .debug_bounds("people-view-tabs-tabs")
            .expect("People tabs rendered");
        let network_tab = tabs.origin + point(tabs.size.width - px(20.), tabs.size.height / 2.);
        cx.simulate_click(network_tab, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let target = view.read_with(cx, |page, _| {
            assert!(page.show_network);
            let bounds = page.network_bounds.get().expect("canvas painted");
            let (cx, cy, side) = network::frame_of(&bounds);
            assert!(side > 50.);
            let grace = &page.graph.value.as_ref().unwrap().nodes[1];
            point(px(cx + grace.x * side), px(cy + grace.y * side))
        });
        cx.simulate_click(target, gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |page, _| {
            assert_eq!(page.selected_id.as_deref(), Some("grace"))
        });
        cx.simulate_keystrokes("k");
        cx.run_until_parked();
        view.read_with(cx, |page, _| {
            assert_eq!(page.selected_id.as_deref(), Some("ada"))
        });
    }

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

    #[test]
    fn person_photo_is_parsed_from_photo_or_avatar_fields() {
        let mut values = std::collections::BTreeMap::new();
        values.insert("photo".into(), serde_json::json!("/tmp/ada.png"));
        let record = EntityRecord {
            id: "p-ada".into(),
            kind: "person".into(),
            name: "Ada Lovelace".into(),
            values,
            ..Default::default()
        };
        let person = Person::from_record(&record);
        assert_eq!(person.photo.as_deref(), Some("/tmp/ada.png"));
        assert!(person.custom_fields.is_empty());
    }
}

/// A person's avatar: profile picture when `photo` is set, otherwise an initials circle.
pub fn avatar_badge(
    initials: &str,
    photo: Option<&str>,
    size_px: f32,
    palette: Palette,
) -> gpui::AnyElement {
    use gpui::{div, img, prelude::*, px, rgb, rgba, ObjectFit};
    if let Some(src) = photo.map(str::trim).filter(|s| !s.is_empty()) {
        let image_el = if src.starts_with("http://") || src.starts_with("https://") {
            img(src.to_owned())
        } else {
            let expanded = if let Some(rest) = src.strip_prefix("~/") {
                std::env::var("HOME")
                    .map(|home| format!("{home}/{rest}"))
                    .unwrap_or_else(|_| src.to_owned())
            } else {
                src.to_owned()
            };
            img(std::path::PathBuf::from(expanded))
        };
        return div()
            .flex_none()
            .size(px(size_px))
            .rounded_full()
            .overflow_hidden()
            .border_1()
            .border_color(rgba(crate::tint(palette.accent, 0x55)))
            .bg(rgba(palette.raised))
            .child(image_el.size_full().object_fit(ObjectFit::Cover))
            .into_any_element();
    }
    div()
        .flex_none()
        .size(px(size_px))
        .rounded_full()
        .bg(rgba(crate::tint(palette.accent, 0x22)))
        .border_1()
        .border_color(rgba(crate::tint(palette.accent, 0x44)))
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(rgb(palette.accent))
        .child(initials.to_owned())
        .into_any_element()
}

/// A person's initials in a round badge, as the Mac's list and card draw them.
pub fn initials_badge(initials: &str, palette: Palette) -> gpui::AnyElement {
    avatar_badge(initials, None, 32.0, palette)
}
