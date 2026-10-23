//! Native People page (`/people`): contacts registry, profiles,
//! notes references, and communication details.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, Entity, FocusHandle,
    FontWeight, KeyBinding, SharedString, Subscription, Window,
};
use sjel_gpui_core::people::{filter_people, Person};
use sjel_gpui_core::shell::{self, EntityRecord, ShellError};

use crate::data::{Remote, Repo};
use crate::theme::Palette;

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
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("j", Next, Some("PeoplePage")),
        KeyBinding::new("down", Next, Some("PeoplePage")),
        KeyBinding::new("k", Previous, Some("PeoplePage")),
        KeyBinding::new("up", Previous, Some("PeoplePage")),
        KeyBinding::new("/", FocusSearch, Some("PeoplePage")),
    ]);
}

pub struct PeoplePage {
    repo: Repo,
    palette: Option<Palette>,
    pub people: Remote<Vec<EntityRecord>>,
    pub selected_id: Option<String>,
    pub search_query: String,
    search_input: Option<(Entity<InputState>, Subscription)>,
    focus: FocusHandle,
    focus_next: bool,
}

impl PeoplePage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            people: Remote::default(),
            selected_id: None,
            search_query: String::new(),
            search_input: None,
            focus: cx.focus_handle(),
            focus_next: true,
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

        let work = shell::run(async move { shell.people(None).await });
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
    }

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.search_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search people…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.search_query = input.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        });
        self.search_input = Some((input.clone(), subscription));
        input
    }

    fn on_next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        let people: Vec<_> = self
            .people
            .value
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(Person::from_record)
            .collect();
        let filtered = filter_people(&people, &self.search_query);
        if filtered.is_empty() {
            return;
        }

        let at = self
            .selected_id
            .as_ref()
            .and_then(|id| filtered.iter().position(|p| &p.id == id));

        let next_idx = match at {
            Some(i) if i + 1 < filtered.len() => i + 1,
            _ => 0,
        };
        self.selected_id = Some(filtered[next_idx].id.clone());
        cx.notify();
    }

    fn on_previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        let people: Vec<_> = self
            .people
            .value
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(Person::from_record)
            .collect();
        let filtered = filter_people(&people, &self.search_query);
        if filtered.is_empty() {
            return;
        }

        let at = self
            .selected_id
            .as_ref()
            .and_then(|id| filtered.iter().position(|p| &p.id == id));

        let prev_idx = match at {
            Some(i) if i > 0 => i - 1,
            _ => filtered.len().saturating_sub(1),
        };
        self.selected_id = Some(filtered[prev_idx].id.clone());
        cx.notify();
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
}

impl Render for PeoplePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_next) {
            window.focus(&self.focus, cx);
        }

        let palette = self.palette.unwrap_or_else(Palette::dark);
        let records = self.people.value.clone().unwrap_or_default();
        let parsed_people: Vec<Person> = records.iter().map(Person::from_record).collect();
        let filtered = filter_people(&parsed_people, &self.search_query);

        let selected_person = self
            .selected_id
            .as_ref()
            .and_then(|id| parsed_people.iter().find(|p| &p.id == id))
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
                            .child(format!("{} contacts", parsed_people.len())),
                    ),
            )
            .child(
                Button::new("people-refresh")
                    .small()
                    .ghost()
                    .label("↻ Refresh")
                    .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
            );

        let search_box = div()
            .w_full()
            .child(Input::new(&self.search_field(window, cx)).small());

        // Left List Pane
        let mut list_pane = div()
            .w(px(320.))
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .child(search_box);

        if filtered.is_empty() {
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
            let mut items_list = div().flex().flex_col().gap_1();
            for person in filtered {
                let chosen = self.selected_id.as_deref() == Some(&person.id);
                let pid = person.id.clone();

                let subtitle = [person.company.as_deref(), person.role.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");

                items_list = items_list.child(
                    div()
                        .id(SharedString::from(format!("person-item-{}", person.id)))
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
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(32.))
                                .h(px(32.))
                                .rounded_full()
                                .bg(rgba((palette.accent & 0xffffff) | 0x22000000))
                                .border_1()
                                .border_color(rgba(palette.accent))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(palette.accent))
                                .child(person.initials.clone()),
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
                                        .child(person.name.clone()),
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
                );
            }
            list_pane = list_pane.child(
                div()
                    .id("people-list-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(items_list),
            );
        }

        // Right Detail Pane
        let detail_pane = if let Some(p) = selected_person {
            render_person_detail(p, palette)
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
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(header)
            .child(
                div()
                    .flex()
                    .gap_4()
                    .flex_1()
                    .min_h_0()
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

fn render_person_detail(person: Person, palette: Palette) -> AnyElement {
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
                .bg(rgba((palette.accent & 0xffffff) | 0x22000000))
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
    let mut contact_fields = div().flex().flex_col().gap_2();

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
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_4()
        .child(header_card)
        .child(
            div()
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
