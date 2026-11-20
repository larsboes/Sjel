//! NSH renderer for the phone Capture state.

use nsh_base::{button, input};
use nsh_core::prelude::*;
use sjel_gpui_core::{capture::TAGS, store::LocalCapture};

use crate::phone_capture::{headline, keep, load, preview_label, second_line, subscribe};

fn muted() -> AdaptiveColor {
    nsh_base::theme().muted_foreground
}

fn capture_row(capture: LocalCapture, refusal: Option<String>) -> impl IntoElement {
    let refused = refusal.is_some();
    view()
        .px_4()
        .py_3()
        .gap_1()
        .border_b(px(1.0))
        .border_b_color(move || nsh_base::theme().border)
        .child(text(headline(&capture)).text_base())
        .child(
            text(second_line(&capture, refusal.as_ref()))
                .text_xs()
                .text_color(move || {
                    if refused {
                        nsh_base::theme().destructive
                    } else {
                        muted()
                    }
                }),
        )
}

#[component]
pub fn CaptureScreen() -> impl IntoElement {
    let mut source = subscribe();
    let initial = source.borrow_and_update().clone();
    let seen_saves = initial.saved;
    let snap = signal(initial);
    let draft = signal(String::new());
    let tag = signal(None::<&'static str>);
    let last_save = signal(seen_saves);
    spawn(async move {
        while source.changed().await.is_ok() {
            let next = source.borrow_and_update().clone();
            // A new thought is not a correction of the last one: the field and chip clear.
            if next.saved != last_save.get() {
                last_save.set(next.saved);
                draft.set(String::new());
                tag.set(None);
            }
            snap.set(next);
        }
    });
    load();
    let submit = move || keep(draft.get(), tag.get());

    let chips = dynamic(move || {
        let active_tag = tag.get();
        let mut row = view().flex_row().flex_wrap().gap_1p5();
        for name in TAGS {
            row = row.child(crate::nsh_ui::pill_tab(
                format!("#{name}"),
                active_tag == Some(name),
                move || {
                    tag.update(|t| *t = if *t == Some(name) { None } else { Some(name) });
                },
            ));
        }
        row.into_element()
    });

    view()
        .flex_1()
        .child(
            view()
                .px_4()
                .pt_4()
                .pb_2()
                .gap_2()
                .child(text("Capture").text_2xl().font_weight(FontWeight::SEMIBOLD))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(
                            input()
                                .value(move || draft.get())
                                .on_change(move |value: &str| draft.set(value.to_owned()))
                                .on_submit(move |_: &str| submit())
                                .placeholder("Note, link or task…")
                                .flex_1()
                                .px_3()
                                .py_2()
                                .border(px(1.0))
                                .border_color(move || nsh_base::theme().input)
                                .rounded_lg(),
                        )
                        .child(
                            button()
                                .px_3()
                                .py_2()
                                .rounded_lg()
                                .bg(move || nsh_base::theme().primary)
                                .on_press(submit)
                                .child(
                                    text(move || {
                                        if snap.with(|s| s.saving) {
                                            "Saving…"
                                        } else {
                                            "Keep"
                                        }
                                    })
                                    .text_sm()
                                    .text_color(move || nsh_base::theme().primary_foreground),
                                ),
                        ),
                )
                .child(chips)
                .child(
                    text(move || preview_label(&draft.get(), tag.get()))
                        .text_xs()
                        .text_color(move || muted()),
                )
                .child(dynamic(move || {
                    snap.with(|s| s.error.clone()).map(|error| {
                        text(error)
                            .text_xs()
                            .text_color(move || nsh_base::theme().destructive)
                            .into_element()
                    })
                })),
        )
        .child(dynamic(move || {
            let (captures, refusals) = snap.with(|s| (s.captures.clone(), s.refusals.clone()));
            if captures.is_empty() {
                return view()
                    .px_4()
                    .py_3()
                    .child(
                        text("Nothing captured on this phone yet.")
                            .text_sm()
                            .text_color(move || muted()),
                    )
                    .into_element();
            }
            let waiting = captures.iter().filter(|c| !c.on_the_mac()).count();
            let mut list = view().child(
                text(match waiting {
                    0 => "Everything here is on the Mac.".to_owned(),
                    1 => "1 waiting for the Mac".to_owned(),
                    n => format!("{n} waiting for the Mac"),
                })
                .text_xs()
                .px_4()
                .pb_2()
                .text_color(move || muted()),
            );
            for capture in captures {
                let refusal = refusals
                    .iter()
                    .find(|(id, _)| id == &capture.id)
                    .map(|(_, reason)| reason.clone());
                list = list.child(capture_row(capture, refusal));
            }
            scroll().flex_1().child(list).into_element()
        }))
}
