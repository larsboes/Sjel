//! Soundscape drawer UI controls: transport card, animated meter, preset chips, and 10-step level sliders.

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, MouseButton, SharedString,
};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::IconName;
use sjel_gpui_core::soundscape::{self, LayerName, Preset};

use super::Soundscape;
use crate::primitives::{mono_num, pill_tab, surface_card};
use crate::theme::tint;
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
        surface_card(palette)
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
                            .size(px(52.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(rgba(if playing {
                                tint(palette.accent, 0x2a)
                            } else {
                                palette.raised
                            }))
                            .border_1()
                            .border_color(if playing {
                                rgb(palette.accent)
                            } else {
                                rgba(tint(palette.border, 0x88))
                            })
                            .text_color(rgb(if playing {
                                palette.accent
                            } else {
                                palette.text
                            }))
                            .text_lg()
                            .child(if playing {
                                IconName::Pause
                            } else {
                                IconName::Play
                            })
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

    /// The six presets as pill tabs, wrapped cleanly in a surface card.
    pub(crate) fn presets(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let active = self.control.preset();
        let mut row = div().flex().flex_wrap().items_center().gap_1p5();
        for key in soundscape::music::PRESETS {
            let Some(preset) = Preset::from_key(key) else {
                continue;
            };
            let is_active = preset == active;
            row = row.child(pill_tab(
                SharedString::from(format!("soundscape-preset-{key}")),
                title_case(preset.key()),
                None,
                is_active,
                palette,
                cx.listener(move |view, _, _, cx| view.choose_preset(preset, cx)),
            ));
        }
        surface_card(palette)
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child("PRESET"),
            )
            .child(row)
            .into_any_element()
    }

    pub(crate) fn mixer(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut card = surface_card(palette)
            .p_3()
            .flex()
            .flex_col()
            .gap_2p5()
            .child(
                div()
                    .id("soundscape-mixer")
                    .flex()
                    .items_center()
                    .justify_between()
                    .min_h(px(28.))
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
                            .child(if self.prefs.mixer { "Hide" } else { "Show" }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| view.set_mixer(!view.prefs.mixer, cx)),
                    ),
            );
        if !self.prefs.mixer {
            return card.into_any_element();
        }
        for layer in LayerName::ALL {
            let value = self.control.layer(layer);
            let slider = self
                .layer_sliders
                .iter()
                .find(|(l, _)| *l == layer)
                .map(|(_, s)| s);
            card =
                card.child(
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
                                .child(div().text_xs().child(mono_num(
                                    format!("{:.0}%", value * 100.),
                                    palette.muted,
                                ))),
                        )
                        .when_some(slider, |el, s| {
                            el.child(Slider::new(s).accessibility_label(layer.label()))
                        }),
                );
        }
        card.into_any_element()
    }
}
