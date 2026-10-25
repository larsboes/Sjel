//! A typed record and the capabilities that refer to it.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, IntoElement, SharedString, Window};
use sjel_gpui_core::record::{self, Basis, View};

use crate::data::{self, Remote, Repo};
use crate::primitives::{card, note, placeholder, section};
use crate::theme::Palette;

pub struct RecordPage {
    repo: Repo,
    id: String,
    detail: Remote<View>,
}

impl RecordPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            id: String::new(),
            detail: Remote::default(),
        }
    }

    pub fn open(&mut self, id: String, cx: &mut Context<Self>) {
        self.id = id;
        self.detail = Remote::default();
        self.load(cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let id = self.id.clone();
        data::read(
            cx,
            self,
            |page| &mut page.detail,
            &repo,
            move |shell| async move { record::read(&shell, &id).await },
        );
    }
}

impl Render for RecordPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(_window.appearance());
        let Some(view) = self.detail.value.as_ref() else {
            let body = match (&self.detail.error, self.detail.loading) {
                (Some(error), _) => note(error.to_string(), palette.red),
                (None, _) => placeholder(260.),
            };
            return div().p(px(16.)).child(body).into_any_element();
        };

        let properties = view
            .properties
            .iter()
            .map(|property| {
                card(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(12.))
                        .child(
                            div()
                                .w(px(100.))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(property.label.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(rgb(palette.text))
                                .child(property.value.clone()),
                        )
                        .into_any_element(),
                    palette,
                )
                .into_any_element()
            })
            .collect::<Vec<_>>();
        let groups = view
            .groups
            .iter()
            .map(|group| {
                let mut rows = group
                    .records
                    .iter()
                    .map(|record| {
                        let id = record.id.clone();
                        let button_id = format!("record-{}", id);
                        Button::new(SharedString::from(button_id))
                            .ghost()
                            .label(format!("{}  {}", record.title, record.meta))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open(id.clone(), cx);
                            }))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>();
                if let Some(error) = &group.error {
                    rows.push(note(error.clone(), palette.red).into_any_element());
                }
                if group.unlinkable > 0 {
                    rows.push(
                        note(
                            format!("{} linked record(s) have no usable id", group.unlinkable),
                            palette.muted,
                        )
                        .into_any_element(),
                    );
                }
                if rows.is_empty() {
                    rows.push(note("No related records", palette.muted).into_any_element());
                }
                let heading = format!(
                    "{} · {}",
                    group.label,
                    match group.basis {
                        Basis::Reference => "linked",
                        Basis::Coincidence => "same dates",
                    }
                );
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(heading),
                    )
                    .child(card(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .children(rows)
                            .into_any_element(),
                        palette,
                    ))
                    .into_any_element()
            })
            .collect::<Vec<AnyElement>>();

        let title = view.title.clone();
        let id = view.id.clone();
        div()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(22.))
                            .text_color(rgb(palette.text))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("{} · {}", view.kind, id)),
                    ),
            )
            .child(section(
                "Properties",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children(properties)
                    .into_any_element(),
                palette,
            ))
            .child(section(
                "Connected",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .children(groups)
                    .into_any_element(),
                palette,
            ))
            .into_any_element()
    }
}
