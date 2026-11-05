//! The native Links launcher: saved destinations first, with history suggestions kept behind review.

use std::collections::BTreeMap;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, AnyElement, Context, Entity, IntoElement, SharedString, Subscription,
    Window,
};
use sjel_gpui_core::web_links::{WebLink, WebLinkCandidate, WebLinkHistoryReply, WebLinks};

use crate::data::{self, Busy, Remote, Repo};
use crate::primitives::{card, note, placeholder};
use crate::theme::{Palette, ThemeContext};

const ADD: &str = "web-links:add";
const SCAN: &str = "web-links:history-scan";

pub struct LinksPage {
    repo: Repo,
    links: Remote<WebLinks>,
    candidates: Remote<Vec<WebLinkCandidate>>,
    busy: Busy,
    query: String,
    query_input: Option<(Entity<InputState>, Subscription)>,
    url_input: Option<(Entity<InputState>, Subscription)>,
    title_input: Option<(Entity<InputState>, Subscription)>,
    group_input: Option<(Entity<InputState>, Subscription)>,
    edit_id: Option<String>,
    edit_title: Option<Entity<InputState>>,
    edit_group: Option<Entity<InputState>>,
    confirm_remove: Option<String>,
    history_days: u16,
    show_add: bool,
}

impl LinksPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            links: Remote::default(),
            candidates: Remote::default(),
            busy: Busy::default(),
            query: String::new(),
            query_input: None,
            url_input: None,
            title_input: None,
            group_input: None,
            edit_id: None,
            edit_title: None,
            edit_group: None,
            confirm_remove: None,
            history_days: 90,
            show_add: false,
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |state| &mut state.links,
            &repo,
            |shell| async move { shell.web_links().await },
        );
    }

    pub fn load_candidates(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |state| &mut state.candidates,
            &repo,
            |shell| async move { shell.web_link_history_candidates().await },
        );
    }

    fn ensure_query(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.query_input {
            return input.clone();
        }
        let input = cx
            .new(|cx| InputState::new(window, cx).placeholder("Search links, sites, communities…"));
        let subscription =
            cx.subscribe_in(&input, window, |view, input, _event: &InputEvent, _, cx| {
                view.query = input.read(cx).value().to_string();
                cx.notify();
            });
        self.query_input = Some((input.clone(), subscription));
        input
    }

    fn ensure_add_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.url_input.is_some() {
            return;
        }
        let make = |window: &mut Window, cx: &mut Context<Self>, hint: &'static str| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(hint));
            let subscription = cx.subscribe_in(
                &input,
                window,
                |_view, _input, _event: &InputEvent, _, cx| cx.notify(),
            );
            (input, subscription)
        };
        self.url_input = Some(make(window, cx, "https://reddit.com/r/rust"));
        self.title_input = Some(make(window, cx, "Name (optional)"));
        self.group_input = Some(make(window, cx, "Group (optional)"));
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) = self
            .url_input
            .as_ref()
            .map(|(input, _)| input.read(cx).value().trim().to_owned())
            .filter(|url| !url.is_empty())
        else {
            return;
        };
        let title = self
            .title_input
            .as_ref()
            .map(|(input, _)| input.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        let group = self
            .group_input
            .as_ref()
            .map(|(input, _)| input.read(cx).value().trim().to_owned())
            .filter(|group| !group.is_empty());
        let body = serde_json::json!({"url":url, "title":title, "group":group, "pinned":false});
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|state| &mut state.busy, ADD, "Link saved."),
            move |shell| async move { shell.web_link_add(body).await },
            Self::added,
        );
    }

    fn added(this: &mut Self, result: Option<WebLink>, cx: &mut Context<Self>) {
        if result.is_some() {
            this.show_add = false;
            this.url_input = None;
            this.title_input = None;
            this.group_input = None;
            this.load(cx);
        }
    }

    fn begin_edit(&mut self, link: &WebLink, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit_id.as_deref() == Some(link.id.as_str()) {
            self.edit_id = None;
            return;
        }
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        title.update(cx, |input, cx| {
            input.set_value(link.title.clone(), window, cx)
        });
        let group = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Group override (blank = automatic)")
        });
        group.update(cx, |input, cx| {
            input.set_value(link.group_override.clone().unwrap_or_default(), window, cx)
        });
        self.edit_id = Some(link.id.clone());
        self.edit_title = Some(title);
        self.edit_group = Some(group);
        cx.notify();
    }

    fn save_edit(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let title = self
            .edit_title
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        let group = self
            .edit_group
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        let body = serde_json::json!({"title":title, "group":group});
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(
                |state| &mut state.busy,
                format!("edit:{id}"),
                "Link updated.",
            ),
            move |shell| async move { shell.web_link_edit(&id, body).await },
            Self::edited,
        );
    }

    fn edited(this: &mut Self, result: Option<WebLink>, cx: &mut Context<Self>) {
        if result.is_some() {
            this.edit_id = None;
            this.edit_title = None;
            this.edit_group = None;
            this.load(cx);
        }
    }

    fn remove(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let key = format!("remove:{id}");
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|state| &mut state.busy, key, "Link removed."),
            move |shell| async move { shell.web_link_remove(&id).await },
            Self::removed,
        );
    }

    fn removed(this: &mut Self, result: Option<()>, cx: &mut Context<Self>) {
        if result.is_some() {
            this.edit_id = None;
            this.confirm_remove = None;
            this.load(cx);
        }
    }

    fn toggle_pin(&mut self, link: &WebLink, window: &mut Window, cx: &mut Context<Self>) {
        let id = link.id.clone();
        let pinned = !link.pinned;
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet(""),
            move |shell| async move {
                shell
                    .web_link_edit(&id, serde_json::json!({"pinned":pinned}))
                    .await
            },
            Self::edited,
        );
    }

    fn request_remove(&mut self, id: String, cx: &mut Context<Self>) {
        self.confirm_remove = Some(id);
        cx.notify();
    }

    fn open(&mut self, link: &WebLink, window: &mut Window, cx: &mut Context<Self>) {
        cx.open_url(&link.url);
        let id = link.id.clone();
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet(""),
            move |shell| async move { shell.web_link_open(&id).await },
            Self::opened,
        );
    }

    fn opened(this: &mut Self, result: Option<WebLink>, cx: &mut Context<Self>) {
        if let Some(link) = result {
            if let Some(links) = &mut this.links.value {
                if let Some(current) = links.links.iter_mut().find(|current| current.id == link.id)
                {
                    *current = link;
                }
            }
            cx.notify();
        }
    }

    fn scan_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let days = self.history_days;
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(
                |state| &mut state.busy,
                SCAN,
                "History suggestions ready for review.",
            ),
            move |shell| async move { shell.web_link_history_scan(days, 3).await },
            Self::scanned,
        );
    }

    fn scanned(this: &mut Self, result: Option<WebLinkHistoryReply>, cx: &mut Context<Self>) {
        if let Some(reply) = result {
            this.candidates.settle(Ok(reply.candidates));
            this.load(cx);
        }
    }

    fn history_decision(
        &mut self,
        candidate: WebLinkCandidate,
        decision: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = candidate.id;
        let repo = self.repo.clone();
        let key = format!("history:{decision}:{id}");
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(
                |state| &mut state.busy,
                key,
                if decision == "keep" {
                    "Destination saved."
                } else {
                    "Suggestion dismissed."
                },
            ),
            move |shell| async move { shell.web_link_history_decide(&id, decision).await },
            Self::history_decided,
        );
    }

    fn history_decided(this: &mut Self, result: Option<()>, cx: &mut Context<Self>) {
        if result.is_some() {
            this.load(cx);
            this.load_candidates(cx);
        }
    }

    fn filtered<'a>(&self, link: &'a WebLink) -> bool {
        let query = self.query.trim().to_lowercase();
        query.is_empty()
            || [
                link.title.as_str(),
                link.host.as_str(),
                link.group.as_str(),
                link.url.as_str(),
                link.tenant.as_deref().unwrap_or(""),
            ]
            .iter()
            .any(|value| value.to_lowercase().contains(&query))
    }

    fn link_row(
        &self,
        link: &WebLink,
        palette: Palette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let saved = link.clone();
        let saved_for_open = saved.clone();
        let saved_for_edit = saved.clone();
        let heading = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().truncate().text_sm().child(if link.pinned {
                        format!("★  {}", link.title)
                    } else {
                        link.title.clone()
                    }))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{}{} · {}",
                                link.host,
                                link.tenant
                                    .as_ref()
                                    .map(|tenant| format!(" · {tenant}"))
                                    .unwrap_or_default(),
                                link.url
                            )),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("open-{}", link.id)))
                    .small()
                    .label("Open")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.open(&saved_for_open, window, cx)
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("pin-{}", link.id)))
                    .small()
                    .ghost()
                    .label(if link.pinned { "Unpin" } else { "Pin" })
                    .on_click(
                        cx.listener(move |view, _, window, cx| view.toggle_pin(&saved, window, cx)),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("edit-{}", link.id)))
                    .small()
                    .ghost()
                    .label("Edit")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.begin_edit(&saved_for_edit, window, cx)
                    })),
            );
        if self.edit_id.as_deref() != Some(link.id.as_str()) {
            return heading.into_any_element();
        }
        let title = self.edit_title.clone();
        let group = self.edit_group.clone();
        let id = link.id.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(heading)
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .children(title.map(|input| Input::new(&input)))
                    .children(group.map(|input| Input::new(&input)))
                    .child(
                        Button::new("web-link-save-edit")
                            .small()
                            .label("Save")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.save_edit(id.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("web-link-cancel-edit")
                            .small()
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.edit_id = None;
                                view.confirm_remove = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        if self.confirm_remove.as_deref() == Some(link.id.as_str()) {
                            let remove_id = link.id.clone();
                            Button::new("web-link-remove-confirm")
                                .small()
                                .danger()
                                .label("Confirm remove")
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.remove(remove_id.clone(), window, cx)
                                }))
                        } else {
                            let remove_id = link.id.clone();
                            Button::new("web-link-remove")
                                .small()
                                .ghost()
                                .label("Remove…")
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.request_remove(remove_id.clone(), cx)
                                }))
                        },
                    ),
            )
            .into_any_element()
    }

    fn candidate_row(
        &self,
        candidate: &WebLinkCandidate,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let keep = candidate.clone();
        let dismiss = candidate.clone();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .child(if candidate.title.is_empty() {
                                candidate.host.clone()
                            } else {
                                candidate.title.clone()
                            }),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} · {} visits · {}",
                                candidate.browser, candidate.visits, candidate.url
                            )),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("keep-{}", candidate.id)))
                    .small()
                    .label("Keep")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.history_decision(keep.clone(), "keep", window, cx)
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("dismiss-{}", candidate.id)))
                    .small()
                    .ghost()
                    .label("Dismiss")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.history_decision(dismiss.clone(), "dismiss", window, cx)
                    })),
            )
            .into_any_element()
    }
}

impl Render for LinksPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let query = self.ensure_query(window, cx);
        let links = self
            .links
            .value
            .as_ref()
            .map(|value| value.links.clone())
            .unwrap_or_default();
        let mut groups: BTreeMap<String, Vec<WebLink>> = BTreeMap::new();
        for link in links.into_iter().filter(|link| self.filtered(link)) {
            groups.entry(link.group.clone()).or_default().push(link);
        }
        let group_views: Vec<AnyElement> = groups
            .into_iter()
            .map(|(name, links)| {
                let rows = links
                    .iter()
                    .map(|link| self.link_row(link, palette, window, cx))
                    .collect::<Vec<_>>();
                card(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(div().text_xs().text_color(rgb(palette.muted)).child(name))
                        .children(rows)
                        .into_any_element(),
                    palette,
                )
            })
            .collect();
        let content = if self.links.first_load() {
            placeholder(280.).into_any_element()
        } else if let Some(error) = &self.links.error {
            note(error.to_string(), palette.red)
        } else if group_views.is_empty() {
            note("No matching destinations. Add one, or import reviewed suggestions from browser history.", palette.muted)
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .children(group_views)
                .into_any_element()
        };
        let add_form = if self.show_add {
            self.ensure_add_inputs(window, cx);
            let url = self.url_input.as_ref().map(|(input, _)| input.clone());
            let title = self.title_input.as_ref().map(|(input, _)| input.clone());
            let group = self.group_input.as_ref().map(|(input, _)| input.clone());
            let form = div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_sm().child("Add a destination"))
                .children(url.map(|input| Input::new(&input)))
                .children(title.map(|input| Input::new(&input)))
                .children(group.map(|input| Input::new(&input)))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            Button::new("web-links-add-save")
                                .label("Save link")
                                .loading(self.busy.contains(ADD))
                                .on_click(cx.listener(|view, _, window, cx| view.add(window, cx))),
                        )
                        .child(
                            Button::new("web-links-add-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.show_add = false;
                                    cx.notify();
                                })),
                        ),
                );
            Some(card(form.into_any_element(), palette))
        } else {
            None
        };
        let candidates = self.candidates.value.as_deref().unwrap_or_default();
        let history_rows: Vec<AnyElement> = candidates
            .iter()
            .map(|candidate| self.candidate_row(candidate, palette, cx))
            .collect();
        let history = if candidates.is_empty() {
            None
        } else {
            let header = div()
                .flex()
                .items_center()
                .child(div().flex_1().text_sm().child(format!(
                    "History suggestions · {} to review",
                    candidates.len()
                )))
                .child(
                    Button::new("web-links-history-scan")
                        .small()
                        .ghost()
                        .label("Rescan")
                        .on_click(cx.listener(|view, _, window, cx| view.scan_history(window, cx))),
                );
            Some(card(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(header)
                    .children(history_rows)
                    .into_any_element(),
                palette,
            ))
        };
        div()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(20.))
                            .text_color(rgb(palette.text))
                            .child("Links"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .children([30_u16, 90, 180].into_iter().map(|days| {
                                Button::new(SharedString::from(format!("history-range-{days}")))
                                    .small()
                                    .when(self.history_days != days, |button| button.ghost())
                                    .label(format!("{days}d"))
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.history_days = days;
                                        cx.notify();
                                    }))
                            })),
                    )
                    .child(
                        Button::new("web-links-history-import")
                            .small()
                            .ghost()
                            .label("Import history")
                            .loading(self.busy.contains(SCAN))
                            .on_click(
                                cx.listener(|view, _, window, cx| view.scan_history(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("web-links-add-toggle")
                            .small()
                            .label(if self.show_add { "Close" } else { "+ Add" })
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.show_add = !view.show_add;
                                if view.show_add {
                                    view.ensure_add_inputs(window, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("web-links-refresh")
                            .small()
                            .ghost()
                            .label("Refresh")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.load(cx);
                                view.load_candidates(cx);
                            })),
                    ),
            )
            .child(Input::new(&query))
            .children(add_form)
            .children(history)
            .child(content)
            .into_any_element()
    }
}
