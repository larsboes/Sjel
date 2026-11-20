//! Shared controls and bounded snapshot subscriptions for the phone screens.
use nsh_base::{button, input};
use nsh_core::{element::SignalEquals, prelude::*};
use tokio::sync::watch;

pub(crate) fn snapshot<T: Clone + 'static>(tx: &watch::Sender<T>) -> Signal<T> {
    let mut rx = tx.subscribe();
    let value = signal_with(rx.borrow().clone(), SignalEquals::Never);
    spawn(async move {
        while rx.changed().await.is_ok() {
            let next = rx.borrow_and_update().clone();
            value.set(next);
        }
    });
    value
}

pub(crate) fn action(
    label: impl IntoReactive<SharedString>,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .min_h(px(44.))
        .px_3()
        .py_2()
        .on_press(press)
        .child(text(label).text_sm().number_of_lines(1u32))
}

pub(crate) fn field(label: &'static str, value: Signal<String>) -> impl IntoElement {
    view().gap_1().child(text(label).text_xs()).child(
        input()
            .test_id(label)
            .value(move || value.get())
            .on_change(move |next: &str| value.set(next.to_owned())),
    )
}

pub(crate) fn heading(label: impl Into<String>) -> impl IntoElement {
    text(label.into())
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
}

pub(crate) fn caption(label: impl Into<String>) -> impl IntoElement {
    text(label.into())
        .text_xs()
        .text_color(|| nsh_base::theme().muted_foreground)
}

pub(crate) fn section_heading(label: impl Into<String>) -> impl IntoElement {
    text(label.into())
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(|| nsh_base::theme().muted_foreground)
}

pub(crate) fn surface_card() -> View {
    view()
        .rounded_xl()
        .border(px(1.0))
        .border_color(move || nsh_base::theme().border)
        .bg(move || nsh_base::theme().card)
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn metric_tile(
    label: impl Into<String>,
    value: impl Into<String>,
    caption_text: impl Into<String>,
) -> impl IntoElement {
    surface_card()
        .flex_1()
        .px_4()
        .py_3()
        .gap_1()
        .child(section_heading(label))
        .child(text(value.into()).text_2xl().font_weight(FontWeight::BOLD))
        .child(caption(caption_text))
}

pub(crate) fn pill_tab(
    label: impl Into<String>,
    active: bool,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .px_3()
        .py_1()
        .rounded_full()
        .border(px(1.0))
        .border_color(move || {
            if active {
                nsh_base::theme().primary
            } else {
                nsh_base::theme().border
            }
        })
        .bg(move || {
            if active {
                nsh_base::theme().primary
            } else {
                nsh_base::theme().card
            }
        })
        .on_press(press)
        .child(
            text(label.into())
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if active {
                        nsh_base::theme().primary_foreground
                    } else {
                        nsh_base::theme().muted_foreground
                    }
                }),
        )
}

pub(crate) fn status_chip(label: impl Into<String>, highlight: bool) -> impl IntoElement {
    view()
        .px_2()
        .py_1()
        .rounded_full()
        .border(px(1.0))
        .border_color(move || {
            if highlight {
                nsh_base::theme().primary
            } else {
                nsh_base::theme().border
            }
        })
        .bg(move || nsh_base::theme().card)
        .child(
            text(label.into())
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if highlight {
                        nsh_base::theme().primary
                    } else {
                        nsh_base::theme().muted_foreground
                    }
                }),
        )
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn action_circle(
    glyph: &'static str,
    label: &'static str,
    active: bool,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .flex_1()
        .items_center()
        .gap_1()
        .py_1()
        .accessibility_label(label)
        .on_press(press)
        .child(
            view()
                .size(px(44.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .border(px(1.0))
                .border_color(move || {
                    if active {
                        nsh_base::theme().primary
                    } else {
                        nsh_base::theme().border
                    }
                })
                .bg(move || {
                    if active {
                        nsh_base::theme().primary
                    } else {
                        nsh_base::theme().card
                    }
                })
                .child(
                    text(glyph)
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || {
                            if active {
                                nsh_base::theme().primary_foreground
                            } else {
                                nsh_base::theme().foreground
                            }
                        }),
                ),
        )
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if active {
                        nsh_base::theme().primary
                    } else {
                        nsh_base::theme().muted_foreground
                    }
                }),
        )
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn progress_bar(ratio: f32) -> impl IntoElement {
    let clamped = ratio.clamp(0.0, 1.0);
    let width = 72.0_f32;
    view()
        .w(px(width))
        .h(px(4.0))
        .rounded_full()
        .bg(move || nsh_base::theme().border)
        .child(
            view()
                .h(px(4.0))
                .w(px(width * clamped))
                .rounded_full()
                .bg(move || nsh_base::theme().primary),
        )
}

pub(crate) fn nav_bar(
    title: impl Into<String>,
    back_label: Option<&'static str>,
    on_back: impl Fn() + 'static,
    trailing: Option<nsh_core::element::AnyElement>,
) -> impl IntoElement {
    let title = title.into();
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .px_4()
        .py_2()
        .gap_2()
        .border_b(px(1.0))
        .border_color(move || nsh_base::theme().border)
        .bg(move || nsh_base::theme().card)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(back_label.map(|label| {
                    button()
                        .min_h(px(36.0))
                        .px_2p5()
                        .py_1()
                        .rounded_lg()
                        .border(px(1.0))
                        .border_color(move || nsh_base::theme().border)
                        .bg(move || nsh_base::theme().background)
                        .on_press(on_back)
                        .child(
                            text(label)
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || nsh_base::theme().foreground),
                        )
                }))
                .child(
                    text(title)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || nsh_base::theme().foreground),
                ),
        )
        .child(trailing)
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn list_row(
    title: impl Into<String>,
    subtitle: impl Into<String>,
    trailing: Option<String>,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    let title = title.into();
    let subtitle = subtitle.into();
    surface_card().px_3().py_2p5().child(
        button()
            .min_h(px(44.0))
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .on_press(press)
            .child(
                view()
                    .flex_1()
                    .gap_0p5()
                    .child(
                        text(title)
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || nsh_base::theme().foreground),
                    )
                    .child((!subtitle.is_empty()).then(|| caption(subtitle))),
            )
            .child(trailing.map(|badge| status_chip(badge, false))),
    )
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn filter_drawer(
    title: impl Into<String>,
    summary: impl Into<String>,
    open: Signal<bool>,
    body: Option<impl IntoElement>,
) -> impl IntoElement {
    let title = title.into();
    let summary = summary.into();
    surface_card()
        .px_3()
        .py_2()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    view()
                        .flex_1()
                        .gap_0p5()
                        .child(section_heading(title))
                        .child(caption(summary)),
                )
                .child(
                    button()
                        .px_3()
                        .py_1p5()
                        .rounded_full()
                        .border(px(1.0))
                        .border_color(move || nsh_base::theme().border)
                        .bg(move || nsh_base::theme().background)
                        .on_press(move || open.update(|v| *v = !*v))
                        .child(
                            text(move || {
                                if open.get() {
                                    "Hide filters"
                                } else {
                                    "Filter & Sort"
                                }
                            })
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD),
                        ),
                ),
        )
        .child(body)
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn empty_state(title: impl Into<String>, detail: impl Into<String>) -> impl IntoElement {
    surface_card()
        .px_4()
        .py_4()
        .gap_1()
        .items_center()
        .child(
            text(title.into())
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD),
        )
        .child(caption(detail))
}
