//! GPUI transport, presets and levels for the phone's native audio session.

use std::time::Duration;

use gpui::{div, prelude::*, px, rgb, rgba, Context, Div, FontWeight, Task, Window};
use sjel_gpui_core::soundscape::{LayerName, Preset};

use super::{feed, publish, save, state, Snapshot};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        forms::Slider,
        theme::ControlSize,
    },
    theme::{Palette, ThemeContext},
};

pub struct PhoneSoundscape {
    snapshot: Snapshot,
    _updates: Task<()>,
    save: Option<Task<()>>,
}

impl PhoneSoundscape {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut levels = feed().tx.subscribe();
        let snapshot = levels.borrow_and_update().clone();
        let updates = cx.spawn(async move |view, cx| {
            while levels.changed().await.is_ok() {
                let snapshot = levels.borrow_and_update().clone();
                if view
                    .update(cx, |view, cx| {
                        view.snapshot = snapshot;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            snapshot,
            _updates: updates,
            save: None,
        }
    }

    fn commit_volume(&mut self, value: f32, cx: &mut Context<Self>) {
        {
            let mut state = state();
            state.control.set_volume(value);
            state.prefs.volume = value;
            publish(&mut state);
        }
        self.save = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            save();
        }));
    }
}

impl Render for PhoneSoundscape {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let (volume, current, layers) = {
            let state = state();
            (
                state.control.volume(),
                state.control.preset(),
                LayerName::ALL.map(|layer| (layer, state.control.layer(layer))),
            )
        };
        let snapshot = &self.snapshot;
        let status = snapshot.error.clone().unwrap_or_else(|| {
            if snapshot.device.is_empty() {
                "Waiting for the audio device…".into()
            } else {
                format!("{} · {}", snapshot.report, snapshot.device)
            }
        });
        div()
            .id("soundscape-scroll")
            .size_full()
            .overflow_y_scroll()
            .bg(rgba(palette.window_tint))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Soundscape"),
                    )
                    .child(
                        button(
                            "soundscape-play",
                            if snapshot.playing { "Pause" } else { "Play" },
                            true,
                        )
                        .disabled(snapshot.device.is_empty() || snapshot.error.is_some())
                        .on_click(|_, _, _| {
                            let mut state = state();
                            if state.snap.error.is_none() && !state.snap.device.is_empty() {
                                state.control.set_playing(!state.control.playing());
                                publish(&mut state);
                            }
                        }),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(if snapshot.error.is_some() {
                                palette.red
                            } else {
                                palette.muted
                            }))
                            .child(status),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .children(Preset::ALL.map(|preset| {
                                button(preset.key(), preset.label(), preset == current).on_click(
                                    move |_, _, _| {
                                        {
                                            let mut state = state();
                                            state.control.set_preset(preset);
                                            state.prefs.preset = preset.key().into();
                                            publish(&mut state);
                                        }
                                        save();
                                    },
                                )
                            })),
                    )
                    .child(level("Volume", volume, palette, {
                        let view = cx.entity().downgrade();
                        move |value, _, cx| {
                            let _ =
                                view.update(cx, |view, cx| view.commit_volume(value as f32, cx));
                        }
                    }))
                    .when(snapshot.playing, |panel| {
                        panel.child(div().flex().items_end().h(px(36.)).gap(px(2.)).children(
                            snapshot.bands.map(|band| {
                                div()
                                    .flex_1()
                                    .h(px((band.clamp(0., 1.) * 36.).max(1.)))
                                    .rounded_sm()
                                    .bg(rgb(palette.accent))
                            }),
                        ))
                    })
                    .child(
                        button(
                            "soundscape-mixer",
                            if snapshot.mixer {
                                "Hide mixer"
                            } else {
                                "Show mixer"
                            },
                            false,
                        )
                        .on_click(|_, _, _| {
                            {
                                let mut state = state();
                                state.prefs.mixer = !state.prefs.mixer;
                                publish(&mut state);
                            }
                            save();
                        }),
                    )
                    .when(snapshot.mixer, |panel| {
                        panel.children(layers.map(|(layer, value)| {
                            level(layer.label(), value, palette, move |value, _, _| {
                                let mut state = state();
                                state.control.set_layer(layer, value as f32);
                                publish(&mut state);
                            })
                        }))
                    }),
            )
    }
}

fn button(id: &'static str, label: &'static str, active: bool) -> Button {
    Button::new(id, label)
        .size(ControlSize::Lg)
        .variant(if active {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Outline
        })
}

fn level(
    label: &'static str,
    value: f32,
    palette: Palette,
    change: impl Fn(f64, &mut Window, &mut gpui::App) + 'static,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_sm().text_color(rgb(palette.muted)).child(label))
        .child(
            Slider::new(label, f64::from(value))
                .range(0., 1.)
                .step(0.01)
                .on_change(change),
        )
}
