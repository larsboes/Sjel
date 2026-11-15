//! Generic registry object lists and read-only property views.

use std::rc::Rc;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{div, prelude::*, px, rgb, Context, SharedString, Window};
use serde_json::Value;
use sjel_gpui_core::record::registry::{self, View};

use crate::data::{self, Remote, Repo};
use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::{
    data_display::Tone,
    tables::{Cell, Column, DataTable, Row},
};
use crate::primitives::note;
use crate::theme::ThemeContext;

pub struct RegistryPage {
    repo: Repo,
    capability: String,
    data: Remote<View>,
    selected: Option<Value>,
    history: Vec<(View, Option<Value>)>,
}

impl RegistryPage {
    pub fn new(repo: Repo, capability: String, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            repo,
            capability,
            data: Remote::default(),
            selected: None,
            history: Vec::new(),
        };
        page.load(cx);
        page
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let capability = self.capability.clone();
        data::read(
            cx,
            self,
            |page| &mut page.data,
            &repo,
            move |shell| async move { registry::load(&shell, &capability).await },
        );
    }

    fn follow(&mut self, row: Value, index: usize, cx: &mut Context<Self>) {
        if self.data.loading {
            return;
        }
        let Some(view) = self.data.value.clone() else {
            return;
        };
        let binding = match view.link(&row, index) {
            Ok(binding) => binding,
            Err(error) => {
                self.data.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.history.push((view.clone(), self.selected.take()));
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |page| &mut page.data,
            &repo,
            move |shell| async move { registry::read(&shell, view.registry, binding).await },
        );
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if self.data.loading {
            return;
        }
        if self.selected.take().is_none() {
            if let Some((view, selected)) = self.history.pop() {
                self.data = Remote {
                    value: Some(view),
                    ..Remote::default()
                };
                self.selected = selected;
            }
        }
        cx.notify();
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        value => value.to_string(),
    }
}

fn row_key(view: &View, row: &Value) -> String {
    let identity = view
        .object()
        .identity
        .iter()
        .map(|key| row[key].clone())
        .collect::<Vec<_>>();
    format!("{}:{}", view.object().id, Value::Array(identity))
}

impl Render for RegistryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let mut body = div()
            .id("registry-browser")
            .size_full()
            .overflow_y_scrollbar()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.));
        let loading = self.data.loading;
        body = body.child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    div()
                        .debug_selector(|| "registry-back-control".into())
                        .child(
                            Button::new("registry-back", "Back")
                                .variant(ButtonVariant::Ghost)
                                .disabled(
                                    loading || (self.selected.is_none() && self.history.is_empty()),
                                )
                                .on_click(cx.listener(|page, _, _, cx| page.back(cx))),
                        ),
                )
                .child(
                    Button::new("registry-refresh", "Reload")
                        .variant(ButtonVariant::Ghost)
                        .disabled(loading)
                        .on_click(cx.listener(|page, _, _, cx| {
                            if let Some(view) = page.data.value.clone() {
                                let repo = page.repo.clone();
                                data::read(
                                    cx,
                                    page,
                                    |page| &mut page.data,
                                    &repo,
                                    move |shell| async move {
                                        registry::read(&shell, view.registry, view.binding).await
                                    },
                                );
                                page.selected = None;
                            } else {
                                page.load(cx);
                            }
                        })),
                ),
        );
        if loading {
            body = body.child(note("Loading objects…", palette.muted));
        }
        if let Some(error) = &self.data.error {
            body = body.child(note(error.to_string(), palette.red));
        }
        let Some(view) = self.data.value.as_ref() else {
            return body.into_any_element();
        };
        let object = view.object();
        let single = object
            .reads
            .iter()
            .find(|read| read.id == view.binding.read)
            .is_some_and(|read| !read.many);
        let selected =
            self.selected
                .as_ref()
                .or_else(|| if single { view.rows.first() } else { None });
        if let Some(row) = selected {
            body = body.child(
                div()
                    .text_xl()
                    .text_color(rgb(palette.text))
                    .child(text(&row[&object.title])),
            );
            if let Some(properties) = object.schema["properties"].as_object() {
                for (key, _) in properties {
                    if let Some(value) = row.get(key) {
                        body = body.child(
                            div()
                                .id(SharedString::from(format!("registry-property-{key}")))
                                .flex()
                                .gap(px(16.))
                                .child(
                                    div()
                                        .w(px(150.))
                                        .text_color(rgb(palette.muted))
                                        .child(key.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_color(rgb(palette.text))
                                        .child(text(value)),
                                ),
                        );
                    }
                }
            }
            for (index, link) in object.links.iter().enumerate() {
                let row = row.clone();
                body = body.child(
                    Button::new(
                        SharedString::from(format!("registry-link-{index}")),
                        link.label.clone(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .disabled(loading)
                    .on_click(
                        cx.listener(move |page, _, _, cx| page.follow(row.clone(), index, cx)),
                    ),
                );
            }
        } else {
            let properties = object.schema["properties"]
                .as_object()
                .expect("validated object properties");
            let keys = std::iter::once(&object.title)
                .chain(properties.keys().filter(|key| *key != &object.title))
                .collect::<Vec<_>>();
            let columns = keys
                .iter()
                .map(|key| {
                    let column = Column::new((*key).clone(), (*key).clone());
                    if matches!(
                        properties[*key]["type"].as_str(),
                        Some("number" | "integer")
                    ) {
                        column.end()
                    } else {
                        column
                    }
                })
                .collect::<Vec<_>>();
            let rows = view
                .rows
                .iter()
                .map(|row| {
                    let cells = keys
                        .iter()
                        .map(|key| match &row[*key] {
                            Value::Number(number) => Cell::Figure(
                                number.to_string().into(),
                                number.as_f64().unwrap_or_default(),
                                Tone::Neutral,
                            ),
                            value => Cell::Text(text(value).into()),
                        })
                        .collect::<Vec<_>>();
                    Row::new(row_key(view, row), cells)
                })
                .collect::<Vec<_>>();
            let entity = cx.entity();
            let values = view
                .rows
                .iter()
                .map(|row| (row_key(view, row), row.clone()))
                .collect::<Vec<_>>();
            body = body
                .child(
                    div()
                        .text_xl()
                        .text_color(rgb(palette.text))
                        .child(object.label.clone()),
                )
                .child(
                    DataTable::new("registry-objects", columns)
                        .rows(Rc::new(rows))
                        .on_open(move |key, _, cx| {
                            if loading {
                                return;
                            }
                            if let Some((_, row)) = values.iter().find(|(id, _)| id == key.as_ref())
                            {
                                entity.update(cx, |page, cx| {
                                    page.selected = Some(row.clone());
                                    cx.notify();
                                });
                            }
                        })
                        .empty("No objects"),
                );
        }
        body.into_any_element()
    }
}

#[cfg(test)]
mod tests;
