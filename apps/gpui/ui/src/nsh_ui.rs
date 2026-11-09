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
