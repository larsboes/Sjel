//! Soundscape drawer UI controls: transport card, animated meter, preset chips, and 10-step level sliders.

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, MouseButton, SharedString,
};
use sjel_gpui_core::soundscape::{self, LayerName, Preset};

use super::Soundscape;
use crate::Palette;

/// One cell of a level bar. Ten steps is a hundredth of a value without a keyboard.
pub const STEPS: usize = 10;

/// One frame asked for in five while something sounds: five times the analyser's own release rate is
/// more than the meter can show, and a phone pays for every one.
pub const METER_EVERY: usize = 5;

/// `focus` → `Focus`: a preset's key is the wire form, not a label to draw.
pub fn title_case(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl Soundscape {
    /// Play or pause, and one line about what is sounding.
    pub(crate) fn transport(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let playing = self.control.playing();
        let playback = self.playback();
        let device = match (&self.error, &self.device) {
            (Some(error), _) => error.clone(),
            (None, Some(device)) => device.clone(),
            (None, None) => SharedString::from("No device yet"),
        };
        div()
            .rounded(px(12.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id("soundscape-transport")
                            .flex_none()
                            .size(px(56.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgba(if playing {
                                palette.accent
                            } else {
                                palette.raised
                            }))
                            .text_color(rgb(if playing { 0xffffff } else { palette.text }))
                            .text_lg()
                            .child(if playing { "❚❚" } else { "▶" })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, _, cx| view.toggle(cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(1.))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(palette.text))
                                    .child(playback.preset.label()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(playback.describe()),
                            ),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(if self.error.is_some() {
                        palette.red
                    } else {
                        palette.muted
                    }))
                    .child(device),
            )
            .when(playing, |card| card.child(self.meter(palette)))
            .into_any_element()
    }

    /// The spectrum the audio thread publishes, as ten bars. Something sounding is the one thing a
    /// paused drawer should not fake, so it is drawn only while playing.
    pub(crate) fn meter(&self, palette: Palette) -> AnyElement {
        let bands = self.visual.bands();
        let mut row = div().flex().items_end().gap(px(2.)).h(px(36.));
        for band in bands {
            let height = (band.clamp(0., 1.) * 36.).max(1.);
            row = row.child(
                div()
                    .flex_1()
                    .h(px(height))
                    .rounded(px(1.))
                    .bg(rgb(palette.accent)),
            );
        }
        row.into_any_element()
    }

    /// The six presets, three to a row: a thumb reaches the corner of a phone.
    pub(crate) fn presets(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let active = self.control.preset();
        let mut grid = div().flex().flex_col().gap_2();
        for chunk in soundscape::music::PRESETS.chunks(3) {
            let mut row = div().flex().items_center().gap_2();
            for key in chunk {
                let Some(preset) = Preset::from_key(key) else {
                    continue;
                };
                let is_active = preset == active;
                row = row.child(
                    div()
                        .id(SharedString::from(format!("soundscape-preset-{key}")))
                        .flex_1()
                        .min_h(px(40.))
                        .rounded(px(8.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .cursor_pointer()
                        .when(is_active, |chip| {
                            chip.bg(rgb(palette.accent))
                                .text_color(rgb(0xffffff))
                                .font_weight(FontWeight::SEMIBOLD)
                        })
                        .when(!is_active, |chip| {
                            chip.bg(rgba(palette.raised))
                                .text_color(rgb(palette.muted))
                                .border_1()
                                .border_color(rgba(palette.border))
                        })
                        .child(title_case(preset.key()))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, _, cx| view.choose_preset(preset, cx)),
                        ),
                );
            }
            grid = grid.child(row);
        }
        grid.into_any_element()
    }

    /// One value as ten cells: a tap on a cell sets the value to that cell's top. Exact, and no
    /// track rectangle to measure.
    pub(crate) fn level_bar(
        &self,
        id: SharedString,
        value: f32,
        tone: u32,
        palette: Palette,
        cx: &mut Context<Self>,
        apply: impl Fn(&mut Self, f32, &mut Context<Self>) + Clone + 'static,
    ) -> AnyElement {
        let filled = (value.clamp(0., 1.) * STEPS as f32).round() as usize;
        let mut row = div().flex().items_center().gap(px(2.)).h(px(32.));
        for index in 0..STEPS {
            let set = apply.clone();
            let target = (index + 1) as f32 / STEPS as f32;
            row = row.child(
                div()
                    .id(SharedString::from(format!("{id}-{index}")))
                    .flex_1()
                    .h_full()
                    .rounded(px(2.))
                    .cursor_pointer()
                    .bg(rgba(if index < filled { tone } else { palette.raised }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| set(view, target, cx)),
                    ),
            );
        }
        row.into_any_element()
    }

    pub(crate) fn mixer(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut column = div().flex().flex_col().gap_2().child(
            div()
                .id("soundscape-mixer")
                .flex()
                .items_center()
                .justify_between()
                .min_h(px(36.))
                .cursor_pointer()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("MIXER"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(if self.prefs.mixer { "▾" } else { "▸" }),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|view, _, _, cx| view.set_mixer(!view.prefs.mixer, cx)),
                ),
        );
        if !self.prefs.mixer {
            return column.into_any_element();
        }
        for layer in LayerName::ALL {
            let value = self.control.layer(layer);
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.text))
                                    .child(layer.label()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("{:.0}", value * 100.)),
                            ),
                    )
                    .child(self.level_bar(
                        SharedString::from(format!("layer-{}", layer.label())),
                        value,
                        palette.accent,
                        palette,
                        cx,
                        move |view, value, cx| view.set_layer(layer, value, cx),
                    )),
            );
        }
        column.into_any_element()
    }
}
