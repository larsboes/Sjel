//! The soundscape drawer: transport, preset, volume and the mixer, in the width of a phone.
//!
//! The engine is platform-free (`sjel_gpui_core::soundscape`) and the output device belongs to the
//! host — `apps/gpui/src/audio.rs` on the Mac, `ios/host/Audio.swift` on the phone. What lives here is
//! the part both surfaces answer the same way: a conductor ([`Control`]) to write to, a report
//! ([`Visual`]) to read back, and the handful of decisions a drawer makes.

pub mod controls;
pub mod slideover;

use std::sync::Arc;

use gpui::{div, prelude::*, px, rgb, rgba, Context, FontWeight, Render, SharedString, Window};
use sjel_gpui_core::soundscape::{Control, LayerName, Playback, Preset, Visual};

use crate::theme::{SoundscapePrefs, ThemeContext};
use crate::Palette;

pub use controls::{title_case, METER_EVERY, STEPS};
pub use slideover::{
    CloseDrawer, NextPreset, SoundscapeSlideover, ToggleMix, ToggleTransport, WIDTH,
};

/// The soundscape drawer: one seat of the right edge (`crate::sidebar`), which owns whether the edge
/// is showing at all and draws the title, the seat chips and the close button around this content.
pub struct Soundscape {
    /// The conductor the device plays. Writing to it is what makes sound; reading it says what was
    /// asked for.
    pub(crate) control: Arc<Control>,
    /// What is actually sounding, published by the audio thread on every block.
    pub(crate) visual: Arc<Visual>,
    /// Mixer disclosure, preset and volume, as the last launch left them.
    pub(crate) prefs: SoundscapePrefs,
    /// The device's own name and rate, or why there is no sound. The host knows; the drawer says.
    pub(crate) device: Option<SharedString>,
    pub(crate) error: Option<SharedString>,
    pub(crate) palette: Palette,
    pub(crate) palette_override: Option<Palette>,
    /// Counts renders between two frames asked for, so the meter redraws at a fifth of the rate.
    pub(crate) tick: usize,
}

impl Soundscape {
    pub fn new(control: Arc<Control>, visual: Arc<Visual>, prefs: SoundscapePrefs) -> Self {
        // The remembered preset and volume are written to the conductor here, not on the first
        // render: the device may already be open when the drawer is built.
        control.set_volume(prefs.volume.clamp(0., 1.));
        if let Some(preset) = Preset::from_key(&prefs.preset) {
            control.set_preset(preset);
        }
        Self {
            control,
            visual,
            prefs,
            device: None,
            error: None,
            palette: Palette::dark(),
            palette_override: None,
            tick: 0,
        }
    }

    /// The settings as they stand, for a host that saves them when the app leaves.
    pub fn prefs(&self) -> &SoundscapePrefs {
        &self.prefs
    }

    /// The host's word about the device: its name and rate, or why there is none.
    pub fn set_device(
        &mut self,
        device: Option<String>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.device = device.map(SharedString::from);
        self.error = error.map(SharedString::from);
        cx.notify();
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != palette || self.palette_override != Some(palette) {
            self.palette = palette;
            self.palette_override = Some(palette);
            cx.notify();
        }
    }

    /// What the transport and the mixer should show: the conductor's intent, and what the synth
    /// reports about it.
    pub fn playback(&self) -> Playback {
        Playback {
            preset: self.control.preset(),
            ..Playback::read(&self.visual, self.control.playing())
        }
    }

    /// Stops the sound without touching the preset, the volume or the mixer.
    pub fn pause(&mut self, cx: &mut Context<Self>) {
        if self.control.playing() {
            self.control.set_playing(false);
            cx.notify();
        }
    }

    pub(crate) fn toggle(&mut self, cx: &mut Context<Self>) {
        let playing = !self.control.playing();
        self.control.set_playing(playing);
        cx.notify();
    }

    pub(crate) fn choose_preset(&mut self, preset: Preset, cx: &mut Context<Self>) {
        self.control.set_preset(preset);
        preset.key().clone_into(&mut self.prefs.preset);
        let _ = self.prefs.save();
        cx.notify();
    }

    pub(crate) fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        let volume = volume.clamp(0., 1.);
        self.control.set_volume(volume);
        self.prefs.volume = volume;
        let _ = self.prefs.save();
        cx.notify();
    }

    pub(crate) fn set_layer(&mut self, layer: LayerName, value: f32, cx: &mut Context<Self>) {
        self.control.set_layer(layer, value.clamp(0., 1.));
        cx.notify();
    }

    pub(crate) fn set_mixer(&mut self, mixer: bool, cx: &mut Context<Self>) {
        self.prefs.mixer = mixer;
        let _ = self.prefs.save();
        cx.notify();
    }
}

impl Render for Soundscape {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette_override
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
        self.palette = palette;
        let playing = self.control.playing();
        // The meter is the only thing here that moves, and it moves because the audio thread wrote a
        // new report. A frame is asked for while something sounds and not one otherwise, and not on
        // every frame either: the analyser's release is a tenth of a second, so a fifth of the window's
        // frames is more than the picture can use and a fifth of the battery cost.
        if playing {
            self.tick = (self.tick + 1) % METER_EVERY;
            if self.tick == 0 {
                window.request_animation_frame();
            }
        }
        let volume = self.prefs.volume;

        div()
            .id("soundscape")
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .pt(px(12.))
            .bg(rgba(palette.panel()))
            .text_color(rgb(palette.text))
            .overflow_y_scroll()
            .child(self.transport(palette, cx))
            .child(self.presets(palette, cx))
            .child(
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
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child("VOLUME"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("{:.0}", volume * 100.)),
                            ),
                    )
                    .child(self.level_bar(
                        SharedString::from("volume"),
                        volume,
                        palette.accent,
                        palette,
                        cx,
                        |view, value, cx| view.set_volume(value, cx),
                    )),
            )
            .child(self.mixer(palette, cx))
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    /// The drawer carries its remembered settings into the conductor, and remembers the next thing the
    /// person does.
    #[gpui::test]
    fn the_drawer_carries_its_settings_and_writes_them(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let control = Arc::new(Control::new());
        let visual = Arc::new(Visual::new());
        let prefs = SoundscapePrefs {
            mixer: true,
            preset: "lofi".to_owned(),
            volume: 0.2,
        };
        let made = control.clone();
        let view = cx.new(|_| Soundscape::new(made, visual, prefs.clone()));
        cx.run_until_parked();
        assert_eq!(control.preset(), Preset::Lofi, "the conductor has it");
        assert!((control.volume() - 0.2).abs() < f32::EPSILON);

        view.update(cx, |drawer, cx| {
            drawer.choose_preset(Preset::Sleep, cx);
            drawer.set_mixer(false, cx);
        });
        view.read_with(cx, |drawer, _| {
            assert_eq!(drawer.prefs().preset, "sleep");
            assert!(!drawer.prefs().mixer);
        });
        assert_eq!(control.preset(), Preset::Sleep);
    }

    #[test]
    fn a_preset_key_is_a_key_not_a_label() {
        assert_eq!(title_case("focus"), "Focus");
        assert_eq!(title_case("edm"), "Edm");
        assert_eq!(title_case(""), "");
    }
}
