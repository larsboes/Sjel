//! The soundscape drawer: transport, preset, volume and the mixer, in the width of a phone.
//!
//! The engine is platform-free (`sjel_gpui_core::soundscape`) and the output device belongs to the
//! host — `apps/gpui/src/audio.rs` on the Mac, `ios/host/Audio.swift` on the phone. What lives here is
//! the part both surfaces answer the same way: a conductor ([`Control`]) to write to, a report
//! ([`Visual`]) to read back, and the handful of decisions a drawer makes.
//!
//! Two of those decisions are the plan's: the drawer remembers whether it was open and whether the
//! mixer was showing, and neither is lost at the next launch ([`SoundscapePrefs`]). `playing` is not
//! remembered — a phone that starts making sound by itself is a different thing from one that
//! remembers its settings.
//!
//! The mixer is not the Mac's fourteen keyboard bars. On a phone the control is a thumb, so a value is
//! a row of ten cells: exact, tappable, and no measurement of a track's rectangle to get wrong.

use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, EventEmitter, FontWeight, MouseButton,
    Render, SharedString, Window,
};
use sjel_gpui_core::soundscape::{self, Control, LayerName, Playback, Preset, Visual};

use crate::theme::SoundscapePrefs;
use crate::{Density, Palette};

/// One cell of a level bar. Ten steps is a hundredth of a value without a keyboard.
const STEPS: usize = 10;

/// One frame asked for in five while something sounds: five times the analyser's own release rate is
/// more than the meter can show, and a phone pays for every one.
const METER_EVERY: usize = 5;

/// What the drawer tells its host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoundscapeEvent {
    /// The person closed it: the host puts it away and remembers that.
    Close,
}

/// The soundscape drawer.
pub struct Soundscape {
    /// The conductor the device plays. Writing to it is what makes sound; reading it says what was
    /// asked for.
    control: Arc<Control>,
    /// What is actually sounding, published by the audio thread on every block.
    visual: Arc<Visual>,
    /// Open state, mixer disclosure, preset and volume, as the last launch left them.
    prefs: SoundscapePrefs,
    /// The device's own name and rate, or why there is no sound. The host knows; the drawer says.
    device: Option<SharedString>,
    error: Option<SharedString>,
    palette: Palette,
    density: Density,
    /// Counts renders between two frames asked for, so the meter redraws at a fifth of the rate.
    tick: usize,
}

impl EventEmitter<SoundscapeEvent> for Soundscape {}

impl Soundscape {
    pub fn new(
        control: Arc<Control>,
        visual: Arc<Visual>,
        prefs: SoundscapePrefs,
        density: Density,
    ) -> Self {
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
            density,
            tick: 0,
        }
    }

    /// The settings as they stand, for a host that saves them when the app leaves.
    pub fn prefs(&self) -> &SoundscapePrefs {
        &self.prefs
    }

    pub fn is_open(&self) -> bool {
        self.prefs.open
    }

    /// Shows or puts away the drawer, and remembers it.
    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.prefs.open == open {
            return;
        }
        self.prefs.open = open;
        let _ = self.prefs.save();
        cx.notify();
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
        if self.palette != palette {
            self.palette = palette;
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

    /// Stops the sound without touching the preset, the volume or the mixer. What a host does when the
    /// drawer goes away: a panel that is not on screen is not a thing that should still be playing in
    /// a pocket.
    pub fn pause(&mut self, cx: &mut Context<Self>) {
        if self.control.playing() {
            self.control.set_playing(false);
            cx.notify();
        }
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        let playing = !self.control.playing();
        self.control.set_playing(playing);
        cx.notify();
    }

    fn choose_preset(&mut self, preset: Preset, cx: &mut Context<Self>) {
        self.control.set_preset(preset);
        preset.key().clone_into(&mut self.prefs.preset);
        let _ = self.prefs.save();
        cx.notify();
    }

    fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        let volume = volume.clamp(0., 1.);
        self.control.set_volume(volume);
        self.prefs.volume = volume;
        let _ = self.prefs.save();
        cx.notify();
    }

    fn set_layer(&mut self, layer: LayerName, value: f32, cx: &mut Context<Self>) {
        self.control.set_layer(layer, value.clamp(0., 1.));
        cx.notify();
    }

    fn set_mixer(&mut self, mixer: bool, cx: &mut Context<Self>) {
        self.prefs.mixer = mixer;
        let _ = self.prefs.save();
        cx.notify();
    }

    fn header(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child("SOUNDSCAPE"),
            )
            .child(
                div()
                    .id("soundscape-close")
                    .flex_none()
                    .size(px(36.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(palette.raised))
                    .text_color(rgb(palette.muted))
                    .child("✕")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.set_open(false, cx);
                            cx.emit(SoundscapeEvent::Close);
                        }),
                    ),
            )
            .into_any_element()
    }

    /// Play or pause, and one line about what is sounding.
    fn transport(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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
    fn meter(&self, palette: Palette) -> AnyElement {
        // The analyser's bands are already in `0.0..=1.0`, smoothed with a fast attack and a slow
        // release (`core/src/soundscape/analyser.rs`). Scaling them by the peak again drew every
        // band at full height, which is a picture of a bar chart rather than of the sound.
        let bands = self.visual.bands();
        let mut row = div().flex().items_end().gap(px(2.)).h(px(36.));
        for band in bands {
            let height = (band.clamp(0., 1.) * 36.).max(1.);
            row = row.child(
                div()
                    .flex_1()
                    .h(px(height))
                    .rounded(px(1.))
                    .bg(rgba(palette.accent)),
            );
        }
        row.into_any_element()
    }

    /// The six presets, three to a row: a thumb reaches the corner of a phone.
    fn presets(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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
                            chip.bg(rgba(palette.accent))
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
    fn level_bar(
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

    fn mixer(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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

/// `focus` → `Focus`: a preset's key is the wire form, not a label to draw.
fn title_case(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl Render for Soundscape {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
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
            .bg(rgb(palette.panel()))
            .text_color(rgb(palette.text))
            .overflow_y_scroll()
            .child(self.header(palette, cx))
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

    /// The drawer opens where the last launch left it, writes the remembered preset and volume into
    /// the conductor, and remembers the next thing the person does. The device is not started here:
    /// `Soundscape::new` only needs the two halves of the engine, which a test makes itself.
    #[gpui::test]
    fn the_drawer_carries_its_settings_and_writes_them(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let control = Arc::new(Control::new());
        let visual = Arc::new(Visual::new());
        let prefs = SoundscapePrefs {
            open: true,
            mixer: true,
            preset: "lofi".to_owned(),
            volume: 0.2,
        };
        let made = control.clone();
        let view = cx.new(|_| Soundscape::new(made, visual, prefs.clone(), Density::Mobile));
        cx.run_until_parked();
        view.read_with(cx, |drawer, _| assert!(drawer.is_open(), "remembered open"));
        assert_eq!(control.preset(), Preset::Lofi, "the conductor has it");
        assert!((control.volume() - 0.2).abs() < f32::EPSILON);

        view.update(cx, |drawer, cx| {
            drawer.choose_preset(Preset::Sleep, cx);
            drawer.set_mixer(false, cx);
            drawer.set_open(false, cx);
        });
        view.read_with(cx, |drawer, _| {
            assert_eq!(drawer.prefs().preset, "sleep");
            assert!(!drawer.prefs().mixer);
            assert!(!drawer.is_open(), "closing is remembered too");
        });
        assert_eq!(control.preset(), Preset::Sleep);
        // ...and a new drawer built from those settings starts where this one left off.
        let next = control.clone();
        let reopened = cx.new(|_| {
            Soundscape::new(
                next,
                Arc::new(Visual::new()),
                SoundscapePrefs::load(),
                Density::Mobile,
            )
        });
        reopened.read_with(cx, |drawer, _| {
            assert!(!drawer.is_open(), "a phone does not open sound by itself");
        });
    }

    #[test]
    fn a_preset_key_is_a_key_not_a_label() {
        assert_eq!(title_case("focus"), "Focus");
        assert_eq!(title_case("edm"), "Edm");
        assert_eq!(title_case(""), "");
    }
}
