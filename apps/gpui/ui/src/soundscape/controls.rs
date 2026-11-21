//! Soundscape drawer UI controls: Endel-style audio-reactive hero, focus timer, mood preset cards, and mixer.

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, MouseButton, SharedString,
};
use sjel_gpui_core::soundscape::{self, LayerName, Preset};

use super::Soundscape;
use crate::ely::buttons::{ToggleButton, ToggleItem};
use crate::ely::data_display::{Badge, Tone};
use crate::ely::feedback::InlineMessage;
use crate::ely::forms::Slider;
use crate::ely::motion::ProgressBar;
use crate::ely::primitives::{Icon, IconName, Severity};
use crate::ely::theme::ControlSize;
use crate::primitives::{mono_num, surface_card};
use crate::theme::tint;
use crate::viz;
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

/// Short mood subtitle shown under a preset in both the shared drawer and desktop slideover.
pub fn preset_mood(preset: Preset) -> &'static str {
    match preset {
        Preset::Focus => "Deep flow",
        Preset::Relax => "Warm unwind",
        Preset::Sleep => "Delta drift",
        Preset::Lofi => "Dusty tape",
        Preset::Ambient => "Open horizon",
        Preset::Edm => "High pulse",
    }
}

impl Soundscape {
    /// Endel-style audio-reactive hero with floating Play/Pause orb, preset subtitle, and state badge.
    pub(crate) fn transport(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let playing = self.control.playing();
        let playback = self.playback();
        let field_hero = div()
            .relative()
            .h(px(172.))
            .w_full()
            .rounded(px(10.))
            .overflow_hidden()
            .border_1()
            .border_color(rgba(if playing {
                tint(palette.accent, 0x55)
            } else {
                palette.border
            }))
            .child(
                div()
                    .size_full()
                    .when(!playing, |canvas| canvas.opacity(0.62))
                    .child(viz::frame(viz::canvas(
                        self.field.clone(),
                        self.visual.clone(),
                    ))),
            )
            .child(
                div()
                    .absolute()
                    .top_2p5()
                    .left_2p5()
                    .right_2p5()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        Badge::new(if playing { "Live" } else { "Paused" })
                            .tone(if playing { Tone::Accent } else { Tone::Neutral })
                            .dot(),
                    )
                    .when_some(self.device.clone(), |row, device| {
                        row.child(Badge::new(device).tone(Tone::Neutral))
                    }),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        div()
                            .id("soundscape-transport")
                            .size(px(56.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(rgba(if playing {
                                tint(palette.accent, 0x38)
                            } else {
                                tint(palette.surface, 0xd8)
                            }))
                            .border_2()
                            .border_color(if playing {
                                rgb(palette.accent)
                            } else {
                                rgba(tint(palette.border, 0xaa))
                            })
                            .text_color(rgb(if playing {
                                palette.accent
                            } else {
                                palette.text
                            }))
                            .text_lg()
                            .child(Icon::new(if playing {
                                IconName::Pause
                            } else {
                                IconName::Play
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, _, cx| view.toggle(cx)),
                            ),
                    )
                    .child(
                        div()
                            .px_2p5()
                            .py(px(3.))
                            .rounded(px(999.))
                            .bg(rgba(tint(palette.panel(), 0xc8)))
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(playback.preset.label()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("· {}", playback.describe())),
                            ),
                    ),
            );

        let mut card = surface_card(palette)
            .p_2p5()
            .flex()
            .flex_col()
            .gap_2()
            .child(field_hero);

        if let Some(error) = &self.error {
            card = card.child(InlineMessage::new(Severity::Danger, error.clone()));
        } else if playing {
            card = card.child(self.meter(palette));
        }

        card.into_any_element()
    }

    /// Focus session countdown strip (`∞`, `15m`, `25m`, `45m`) with live progress bar.
    pub(crate) fn session_timer(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        const OPTIONS: [(Option<u32>, &str); 4] = [
            (None, "∞"),
            (Some(15), "15m"),
            (Some(25), "25m"),
            (Some(45), "45m"),
        ];
        let mut pills = div().flex().flex_wrap().items_center().gap_1p5();
        for (mins, label) in OPTIONS {
            let active = self.session_minutes == mins;
            let on_toggle = cx.listener(move |view, _: &(), _, cx| {
                view.set_session_timer(mins, cx);
            });
            pills = pills.child(
                ToggleButton::new(
                    SharedString::from(format!("soundscape-timer-{label}")),
                    ToggleItem::new(label).label(label),
                    active,
                )
                .size(ControlSize::Sm)
                .on_toggle(move |_, window, cx| on_toggle(&(), window, cx)),
            );
        }

        let progress_row = self.session_minutes.map(|mins| {
            let total = (f32::from(mins as u16) * 60.).max(1.);
            let elapsed = self
                .session_started
                .map(|s| s.elapsed().as_secs_f32())
                .unwrap_or(0.);
            let frac = (elapsed / total).clamp(0., 1.);
            let remaining_secs = (total - elapsed).max(0.) as u32;
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Remaining")
                        .child(mono_num(
                            format!("{:02}:{:02}", remaining_secs / 60, remaining_secs % 60),
                            palette.text,
                        )),
                )
                .child(ProgressBar::new("soundscape-session-progress", frac))
                .into_any_element()
        });

        surface_card(palette)
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("SESSION TIMER"),
                    )
                    .child(
                        Badge::new(match self.session_minutes {
                            None => "Continuous".to_string(),
                            Some(m) => format!("{m} min focus"),
                        })
                        .tone(if self.session_minutes.is_some() {
                            Tone::Accent
                        } else {
                            Tone::Neutral
                        }),
                    ),
            )
            .child(pills)
            .children(progress_row)
            .into_any_element()
    }

    /// The spectrum the audio thread publishes, as ten bars. Something sounding is the one thing a
    /// paused drawer should not fake, so it is drawn only while playing.
    pub(crate) fn meter(&self, palette: Palette) -> AnyElement {
        let bands = self.visual.bands();
        let mut row = div().flex().items_end().gap(px(2.)).h(px(22.));
        for band in bands {
            let height = (band.clamp(0., 1.) * 22.).max(1.);
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

    /// Two-column tactile mood tiles for the six conductor presets.
    pub(crate) fn presets(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let active = self.control.preset();
        let mut grid = div().grid().grid_cols(2).gap_2();
        for key in soundscape::music::PRESETS {
            let Some(preset) = Preset::from_key(key) else {
                continue;
            };
            let is_active = preset == active;
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("soundscape-preset-{key}")))
                    .cursor_pointer()
                    .px_2p5()
                    .py_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if is_active {
                        rgb(palette.accent)
                    } else {
                        rgba(palette.border)
                    })
                    .bg(rgba(if is_active {
                        tint(palette.accent, 0x1e)
                    } else {
                        palette.surface
                    }))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(if is_active {
                                        palette.accent
                                    } else {
                                        palette.text
                                    }))
                                    .child(title_case(preset.key())),
                            )
                            .when(is_active, |row| {
                                row.child(Badge::new("On").tone(Tone::Accent))
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(preset_mood(preset)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| view.choose_preset(preset, cx)),
                    ),
            );
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
                    .child("MOOD"),
            )
            .child(grid)
            .into_any_element()
    }

    pub(crate) fn mixer(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let active_layers = LayerName::ALL
            .iter()
            .filter(|layer| self.control.layer(**layer) > 0.01)
            .count();
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
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child("MIXER"),
                            )
                            .child(
                                Badge::new(format!("{active_layers}/{}", LayerName::ALL.len()))
                                    .tone(Tone::Neutral),
                            ),
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
            let layer_entity = cx.entity();
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
                        .child(
                            Slider::new(
                                SharedString::from(format!("soundscape-layer-{}", layer.label())),
                                f64::from(value.clamp(0., 1.)),
                            )
                            .range(0.0, 1.0)
                            .step(0.01)
                            .on_change(move |next, _, cx| {
                                layer_entity.update(cx, |view, cx| {
                                    view.set_layer(layer, next as f32, cx);
                                });
                            }),
                        ),
                );
        }
        card.into_any_element()
    }
}
