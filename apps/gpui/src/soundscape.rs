//! The soundscape windows: the full player and the mini one, over a running synth.
//!
//! Why a window of its own: a soundscape is not a page you read, it is something you leave on.
//! Inside the main window it would compete with the note you are writing and lose, and its visual
//! would be clipped to a column. So it gets its own window, keeps its audio in a `Global` (so
//! closing the window does not cut the music) and writes what it does to the conductor, which stays
//! the one place that says what should be playing.
//!
//! Two sizes, one view. The mini is the same `Soundscape` with `Mode::Mini`, not a second player: it
//! shares the polling, the patch writes, the drag handling and the field, so the two windows cannot
//! drift apart in behaviour. They differ in layout and in what they show, and each can hand over to
//! the other.
//!
//! Every control here writes a *patch*, never a whole state: the dashboard, the phone and this
//! window drive the same session at once, and a whole-state write from a stale copy would silently
//! undo whatever another surface just changed.
//!
//! It also keeps sounding when the conductor is unreachable. That is not a second source of truth --
//! nothing is written, and the status line says the conductor is down -- it is the last known intent
//! continuing, because a music player that goes silent when a helper process dies is worse than one
//! that carries on and says so.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Bounds, Context, DragMoveEvent,
    FocusHandle, Focusable, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Render,
    SharedString, Window,
};
use sjel_gpui_core::data::{Remote, Repo};
use sjel_gpui_core::shell::{
    self, SoundscapeLayers, SoundscapeParams, SoundscapeSession, SoundscapeState,
};
use sjel_gpui_core::soundscape::{self, LayerName, ParamName, Preset, Visual};

use crate::data;
use crate::theme::Palette;
use crate::viz::{self, Field};

actions!(soundscape, [ToggleTransport, NextPreset, OpenMini]);

/// How often a poll reads the conductor, so another surface's change shows up here without the
/// operator doing anything.
const POLL: Duration = Duration::from_secs(3);
/// A drag writes at most this often while it moves; the last value always lands.
const WRITE_DEBOUNCE: Duration = Duration::from_millis(180);
/// A window that is not focused still animates, but at 30 Hz: enough to look alive behind another
/// app, cheap enough to leave running all day.
const INACTIVE_FRAME_INTERVAL: Duration = Duration::from_millis(33);

/// The six presets the conductor knows, in its own order.
const PRESETS: [Preset; 6] = [
    Preset::Edm,
    Preset::Ambient,
    Preset::Lofi,
    Preset::Focus,
    Preset::Relax,
    Preset::Sleep,
];

/// Which of the two windows this view is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The player: field, transport, mixer, parameters, scenarios.
    Full,
    /// The glance: a strip of the same field, play/pause, the preset, volume, and a way to expand.
    Mini,
}

impl Mode {
    fn size(self) -> gpui_kit::Size<Pixels> {
        match self {
            Self::Full => gpui_kit::size(px(980.), px(700.)),
            Self::Mini => gpui_kit::size(px(400.), px(190.)),
        }
    }

    fn options(self) -> gpui_kit::WindowOptions {
        gpui_kit::WindowOptions {
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some(match self {
                    Self::Full => "Soundscape".into(),
                    Self::Mini => "Soundscape Mini".into(),
                }),
                appears_transparent: true,
                ..Default::default()
            }),
            window_background: gpui_kit::WindowBackgroundAppearance::Opaque,
            window_min_size: Some(self.size()),
            // The mini is a fixed size on purpose: a glanceable strip that can be dragged to any
            // shape is a second full window with less in it.
            is_resizable: self == Self::Full,
            is_minimizable: self == Self::Full,
            inactive_frame_interval: Some(INACTIVE_FRAME_INTERVAL),
            ..Default::default()
        }
    }
}

/// One sliding value that is being dragged right now, so a poll does not fight the hand.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Knob {
    Volume,
    Energy,
    Layer(LayerName),
    Param(ParamName),
}

impl Knob {
    /// One slot per control, so the measured track rectangles are an array rather than a map.
    fn index(self) -> usize {
        match self {
            Self::Volume => 0,
            Self::Energy => 1,
            Self::Layer(layer) => {
                2 + LayerName::ALL
                    .iter()
                    .position(|candidate| *candidate == layer)
                    .unwrap_or(0)
            }
            Self::Param(name) => {
                7 + ParamName::ALL
                    .iter()
                    .position(|candidate| *candidate == name)
                    .unwrap_or(0)
            }
        }
    }
}

/// What one write sends. The conductor's update is a patch, so each one asserts only the fields the
/// operator touched.
enum Write {
    Volume(f32),
    Energy(f32),
    Layers(SoundscapeLayers),
    Params(SoundscapeParams),
}

/// The two channels the audio thread reads, and nothing else: the device itself lives in the app
/// global so that closing a window does not stop the music.
struct ControlHandles {
    control: Arc<soundscape::Control>,
    visual: Arc<Visual>,
}

pub struct Soundscape {
    mode: Mode,
    repo: Repo,
    /// What the conductor says, which is the intent every surface shares.
    state: Remote<SoundscapeState>,
    audio: Arc<ControlHandles>,
    /// The field's own motion, in a cell because the paint callback needs it while `render` only has
    /// `&self`.
    field: Rc<RefCell<Field>>,
    /// Where each bar's track was laid out last frame, measured during prepaint. A mouse event
    /// carries the window position and not the element's bounds, so the mapping from a click to a
    /// value needs the track's own rectangle and there is no other way to get it.
    tracks: Rc<RefCell<[Option<Bounds<Pixels>>; 14]>>,
    dragging: Option<Knob>,
    drag_span: Option<(f32, f32)>,
    last_write: Option<Instant>,
    /// The rate the device gave us, and why there is no sound when there is none.
    device: Option<String>,
    device_error: Option<String>,
    focus: FocusHandle,
    last_frame: Instant,
}

impl Soundscape {
    pub fn new(mode: Mode, cx: &mut Context<Self>) -> Self {
        let audio = crate::audio::audio(cx);
        let handles = ControlHandles {
            control: audio.control.clone(),
            visual: audio.visual.clone(),
        };
        let device = audio.device_name().map(|name| match audio.sample_rate() {
            Some(rate) => format!("{name} · {:.0} kHz", rate / 1000.),
            None => name.to_string(),
        });
        let device_error = audio.error().map(str::to_owned);
        let endpoint = std::env::var("SJEL_STATUS_URL")
            .unwrap_or_else(|_| sjel_gpui_core::shell::DEFAULT_ENDPOINT.to_owned());
        let repo = Repo::new(sjel_gpui_core::shell::Shell::new(&endpoint).map(std::sync::Arc::new));
        let mut view = Self {
            mode,
            repo,
            state: Remote::default(),
            audio: Arc::new(handles),
            field: Rc::new(RefCell::new(Field::new())),
            tracks: Rc::new(RefCell::new([None; 14])),
            dragging: None,
            drag_span: None,
            last_write: None,
            device,
            device_error,
            focus: cx.focus_handle(),
            last_frame: Instant::now(),
        };
        view.read_state(cx);
        view.start_polling(cx);
        view
    }

    /// Follow the conductor. Another surface -- the dashboard, the phone -- may change the preset or
    /// the mix at any time, and this window is one of the places that has to show it without the
    /// operator asking. The read is its own future rather than `data::read_repo`, because the answer
    /// has to be adopted as well as stored, and a poll that only stored it would draw a state the
    /// synth is not playing.
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            let Ok(shell) = repo.shell() else {
                continue;
            };
            let Some(answer) = shell::run(async move { shell.soundscape_state().await }).await
            else {
                return;
            };
            let still_open = this.update(cx, |view, cx| {
                match answer {
                    Ok(state) => {
                        view.state.value = Some(state);
                        view.state.error = None;
                    }
                    Err(error) => view.state.error = Some(error),
                }
                view.state.loading = false;
                view.adopt();
                cx.notify();
            });
            if still_open.is_err() {
                return;
            }
        })
        .detach();
    }

    /// Read the conductor and, unless the operator is mid-gesture, adopt what it says.
    fn read_state(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read_repo(
            cx,
            self,
            |view| &mut view.state,
            &repo,
            |repo| async move { repo.soundscape_state().await },
        );
    }

    /// Push the conductor's answer into the synth.
    fn adopt(&mut self) {
        let Some(state) = self.state.value.clone() else {
            return;
        };
        if self.dragging.is_some() {
            return;
        }
        // A write landing right now is newer than this poll: let the write win.
        if self
            .last_write
            .is_some_and(|at| at.elapsed() < WRITE_DEBOUNCE)
        {
            return;
        }
        self.push(&state);
    }

    fn push(&self, state: &SoundscapeState) {
        self.audio.control.set_preset(state.preset());
        self.audio.control.set_params(state.params());
        self.audio.control.set_layers(state.layers());
        self.audio.control.set_volume(state.volume);
        self.audio.control.set_energy(state.energy);
        self.audio.control.set_seed(state.seed);
        self.audio.control.set_playing(state.playing);
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let playing = self.current().map(|state| state.playing).unwrap_or(false);
        let next = !playing;
        // The transport answers on the keystroke: the conductor confirms afterwards.
        self.audio.control.set_playing(next);
        if let Some(state) = self.state.value.as_mut() {
            state.playing = next;
        }
        self.last_write = Some(Instant::now());
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            // A toast for a transport the operator just watched change would be noise, and the
            // mini window has no room for one.
            data::Press::quiet(""),
            move |shell| async move { shell.set_soundscape_playing(next).await },
            |view, updated, cx| {
                if let Some(state) = updated {
                    view.state.value = Some(state);
                }
                cx.notify();
            },
        );
    }

    fn choose_preset(&mut self, preset: Preset, window: &mut Window, cx: &mut Context<Self>) {
        self.audio.control.set_preset(preset);
        // A scape carries its own defaults, exactly as the dashboard's preset button does.
        let defaults = preset.scape().map(|scape| scape.defaults);
        if let Some(defaults) = defaults {
            self.audio.control.set_params(defaults);
        }
        self.last_write = Some(Instant::now());
        let repo = self.repo.clone();
        let key = preset.key().to_string();
        let params = defaults.map(SoundscapeParams::from);
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet(""),
            move |shell| async move {
                // The preset first: the conductor validates it, and a name it does not know must not
                // take the parameters down with it.
                let state = shell.set_soundscape_preset(&key).await?;
                match params {
                    Some(params) => shell.set_soundscape_params(params).await,
                    None => Ok(state),
                }
            },
            |view, updated, cx| {
                if let Some(state) = updated {
                    view.state.value = Some(state);
                }
                cx.notify();
            },
        );
    }

    /// The next preset in the strip, which is what a click on the preset label does.
    fn next_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .current()
            .map(|state| state.preset())
            .unwrap_or(Preset::Edm);
        let index = PRESETS
            .iter()
            .position(|candidate| *candidate == current)
            .unwrap_or(0);
        self.choose_preset(PRESETS[(index + 1) % PRESETS.len()], window, cx);
    }

    fn set_volume(&mut self, value: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.audio.control.set_volume(value);
        if let Some(state) = self.state.value.as_mut() {
            state.volume = value;
        }
        self.write_soon(window, cx, Write::Volume(value));
    }

    fn set_energy(&mut self, value: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.audio.control.set_energy(value);
        if let Some(state) = self.state.value.as_mut() {
            state.energy = value;
        }
        self.write_soon(window, cx, Write::Energy(value));
    }

    fn set_layer(
        &mut self,
        layer: LayerName,
        value: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.audio.control.set_layer(layer, value);
        if let Some(state) = self.state.value.as_mut() {
            state.layers.set(layer, value);
        }
        let layers = self.audio.control.layers();
        self.write_soon(window, cx, Write::Layers(layers.into()));
    }

    fn set_param(
        &mut self,
        name: ParamName,
        value: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.audio.control.set_param(name, value);
        if let Some(state) = self.state.value.as_mut() {
            state.params.set(name, value);
        }
        let params = self.audio.control.params();
        self.write_soon(window, cx, Write::Params(params.into()));
    }

    /// A drag writes at most every [`WRITE_DEBOUNCE`]; the value under the cursor is applied to the
    /// synth immediately either way, so the sound never lags the hand.
    fn write_soon(&mut self, window: &mut Window, cx: &mut Context<Self>, write: Write) {
        let due = self
            .last_write
            .map(|at| at.elapsed() >= WRITE_DEBOUNCE)
            .unwrap_or(true);
        if !due {
            return;
        }
        self.last_write = Some(Instant::now());
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet(""),
            move |shell| async move {
                match write {
                    Write::Volume(volume) => shell.set_soundscape_volume(volume).await,
                    Write::Energy(energy) => shell.set_soundscape_energy(energy).await,
                    Write::Layers(layers) => shell.set_soundscape_layers(layers).await,
                    Write::Params(params) => shell.set_soundscape_params(params).await,
                }
            },
            |view, updated, cx| {
                if let Some(state) = updated {
                    view.state.value = Some(state);
                }
                cx.notify();
            },
        );
    }

    fn start_session(
        &mut self,
        scenario: &'static str,
        minutes: u64,
        preset: Preset,
        energy: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = SoundscapeSession {
            scenario: scenario.to_string(),
            duration_ms: minutes * 60_000,
            elapsed_ms: 0,
            // The conductor counts the session down from this instant, so a started session has to
            // carry the moment it started rather than asking it to begin counting later.
            running_since_ms: Some(chrono::Utc::now().timestamp_millis().max(0) as u64),
        };
        self.audio.control.set_preset(preset);
        self.audio.control.set_energy(energy);
        self.audio.control.set_playing(true);
        self.last_write = Some(Instant::now());
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet("Session started."),
            move |shell| async move {
                shell.set_soundscape_preset(preset.key()).await?;
                shell.set_soundscape_energy(energy).await?;
                shell.set_soundscape_playing(true).await?;
                shell.set_soundscape_session(Some(session)).await
            },
            |view, updated, cx| {
                if let Some(state) = updated {
                    view.state.value = Some(state);
                }
                cx.notify();
            },
        );
    }

    fn clear_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.last_write = Some(Instant::now());
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet("Session cleared."),
            move |shell| async move { shell.set_soundscape_session(None).await },
            |view, updated, cx| {
                if let Some(state) = updated {
                    view.state.value = Some(state);
                }
                cx.notify();
            },
        );
    }

    fn current(&self) -> Option<&SoundscapeState> {
        self.state.value.as_ref()
    }

    /// How a bar control turns an x position into a value.
    fn value_at(position: f32, width: f32) -> f32 {
        if width <= 0. {
            return 0.;
        }
        (position / width).clamp(0., 1.)
    }

    /// Advance the field and decide whether the window needs another frame.
    ///
    /// `reduce_motion` is the system's own answer to "stop this": the picture then follows the sound
    /// on the frames the window already draws (a poll, a click) instead of driving a loop of its own.
    fn animate(&mut self, window: &mut Window, cx: &Context<Self>) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        let preset = self
            .current()
            .map(|state| state.preset())
            .unwrap_or_else(|| self.audio.visual.preset());
        let moving = self
            .field
            .borrow_mut()
            .advance(&self.audio.visual, preset, dt);
        if moving && !cx.reduce_motion() {
            window.request_animation_frame();
            return true;
        }
        moving
    }
}

impl Focusable for Soundscape {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Soundscape {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        self.adopt();
        self.animate(window, cx);

        let state = self.current();
        let playing = state.map(|state| state.playing).unwrap_or(false);
        let preset = state
            .map(|state| state.preset())
            .unwrap_or_else(|| self.audio.visual.preset());
        let playback = soundscape::Playback::read(&self.audio.visual, playing);

        let mut root = div()
            .key_context("Soundscape")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgba(palette.window_tint))
            .on_action(cx.listener(|view, _: &ToggleTransport, window, cx| {
                view.toggle(window, cx);
            }))
            .on_action(cx.listener(|view, _: &NextPreset, window, cx| {
                view.next_preset(window, cx);
            }));

        let field = div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(viz::frame(viz::canvas(
                self.field.clone(),
                self.audio.visual.clone(),
            )));

        root = match self.mode {
            Mode::Full => root.child(self.full_field(field, palette, preset, &playback, cx)),
            Mode::Mini => root.child(self.mini_field(field, palette, preset, &playback, cx)),
        };

        root = root.child(match self.mode {
            Mode::Full => self.full_controls(window, cx, palette, playing, preset),
            Mode::Mini => self.mini_controls(cx, palette, playing, preset),
        });
        root
    }
}

impl Soundscape {
    /// What sits over the field in the full window: the preset, the arrangement, the device.
    fn full_field(
        &self,
        field: gpui_kit::Div,
        palette: Palette,
        preset: Preset,
        playback: &soundscape::Playback,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        field
            .child(
                div()
                    .absolute()
                    .left(px(16.))
                    .bottom(px(12.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .child(preset.label()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(playback.describe()),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .right(px(16.))
                    .top(px(12.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .when_some(self.device.clone(), |element, device| {
                        element.child(div().text_xs().text_color(rgb(palette.muted)).child(device))
                    })
                    .when_some(self.device_error.clone(), |element, error| {
                        element.child(div().text_xs().text_color(rgb(palette.red)).child(error))
                    })
                    .child(icon_button(
                        "soundscape-to-mini",
                        "mini",
                        palette,
                        cx,
                        |view, window, cx| view.switch_to(Mode::Mini, window, cx),
                    )),
            )
    }

    /// What sits over the field in the mini window: the preset, how full it is, and the way out.
    fn mini_field(
        &self,
        field: gpui_kit::Div,
        palette: Palette,
        preset: Preset,
        playback: &soundscape::Playback,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        field
            .child(
                div()
                    .absolute()
                    .left(px(12.))
                    .bottom(px(8.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child(preset.label()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(playback.describe()),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .right(px(10.))
                    .top(px(8.))
                    .child(icon_button(
                        "soundscape-to-full",
                        "expand",
                        palette,
                        cx,
                        |view, window, cx| view.switch_to(Mode::Full, window, cx),
                    )),
            )
    }

    /// The strip under the field in the full window: transport, presets, then the two rows of values.
    fn full_controls(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        palette: Palette,
        playing: bool,
        preset: Preset,
    ) -> gpui_kit::Div {
        let state = self.current();
        let volume = state.map(|state| state.volume).unwrap_or(0.65);
        let energy = state.map(|state| state.energy).unwrap_or(0.7);
        let layers = state.map(|state| state.layers).unwrap_or_default();
        let params = state.map(|state| state.params).unwrap_or_default();
        let conductor_down = self.state.error.is_some();
        let pending = self.audio.visual.preset() != preset;

        let mut strip = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .px(px(16.))
            .py(px(12.))
            .bg(rgba(palette.surface))
            .border_t_1()
            .border_color(rgba(palette.border));

        strip =
            strip.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(transport(playing, palette, cx))
                    .child(div().flex().flex_wrap().gap_1().children(
                        PRESETS.map(|candidate| {
                            preset_chip(candidate, candidate == preset, palette, cx)
                        }),
                    ))
                    .when(pending, |element| {
                        // The synth changes on the next bar, so the picture saying one thing and the ears
                        // another for half a second is worth naming.
                        element.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.amber))
                                .child("on the next bar"),
                        )
                    })
                    .when(conductor_down, |element| {
                        element.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.amber))
                                .child("conductor unreachable · playing the last known state"),
                        )
                    }),
            );

        strip = strip.child(
            div()
                .flex()
                .gap_4()
                .child(self.bar(
                    "volume",
                    "volume",
                    volume,
                    Knob::Volume,
                    palette.accent,
                    palette,
                    cx,
                    |view, value, window, cx| view.set_volume(value, window, cx),
                ))
                .child(self.bar(
                    "energy",
                    "energy",
                    energy,
                    Knob::Energy,
                    palette.amber,
                    palette,
                    cx,
                    |view, value, window, cx| view.set_energy(value, window, cx),
                )),
        );

        strip = strip.child(div().flex().gap_3().children(LayerName::ALL.map(|layer| {
            self.bar(
                layer.label(),
                layer.label(),
                layers.get(layer),
                Knob::Layer(layer),
                palette.green,
                palette,
                cx,
                move |view, value, window, cx| view.set_layer(layer, value, window, cx),
            )
        })));

        strip = strip.child(div().flex().gap_3().children(ParamName::ALL.map(|name| {
            self.bar(
                name.label(),
                name.label(),
                params.get(name),
                Knob::Param(name),
                palette.muted,
                palette,
                cx,
                move |view, value, window, cx| view.set_param(name, value, window, cx),
            )
        })));

        let session = state.and_then(|state| state.session.clone());
        strip = strip.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(div().w(px(48.)).child("session"))
                .children(match session {
                    Some(session) => {
                        // The conductor stores elapsed time as a checkpoint plus the moment the
                        // session started; a window that showed only the checkpoint would always say
                        // "50 min left" and never count down.
                        let running = session
                            .running_since_ms
                            .map(|started| {
                                let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
                                now.saturating_sub(started)
                            })
                            .unwrap_or(0);
                        let remaining = session
                            .duration_ms
                            .saturating_sub(session.elapsed_ms.saturating_add(running))
                            / 60_000;
                        let label = format!("{} · {remaining} min left", session.scenario);
                        vec![
                            div()
                                .text_color(rgb(palette.text))
                                .child(label)
                                .into_any_element(),
                            icon_button(
                                "session-clear",
                                "clear",
                                palette,
                                cx,
                                |view, window, cx| view.clear_session(window, cx),
                            ),
                        ]
                    }
                    None => vec![
                        quick(
                            "deep-work",
                            "deep focus · 50 min",
                            palette,
                            cx,
                            |view, window, cx| {
                                view.start_session("deep-work", 50, Preset::Focus, 0.72, window, cx)
                            },
                        ),
                        quick(
                            "reading",
                            "reading · 25 min",
                            palette,
                            cx,
                            |view, window, cx| {
                                view.start_session("reading", 25, Preset::Focus, 0.42, window, cx)
                            },
                        ),
                        quick(
                            "reset",
                            "reset · 10 min",
                            palette,
                            cx,
                            |view, window, cx| {
                                view.start_session("reset", 10, Preset::Relax, 0.38, window, cx)
                            },
                        ),
                        quick(
                            "wind-down",
                            "wind down · 45 min",
                            palette,
                            cx,
                            |view, window, cx| {
                                view.start_session("wind-down", 45, Preset::Sleep, 0.24, window, cx)
                            },
                        ),
                    ],
                }),
        );

        // The seed, read off the screen rather than typed: it is what makes a session replayable.
        if let Some(seed) = state.map(|state| state.seed) {
            strip = strip.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!("seed {}", soundscape::format_seed(seed))),
            );
        }

        strip = strip.child(
            div()
                .flex()
                .gap_4()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("space play/pause")
                .child("⇧⌘N next preset")
                .child("⌘⇧M mini")
                .child("⌘W close"),
        );

        strip
    }

    /// The row under the field in the mini window: transport, the preset, volume, and no more.
    fn mini_controls(
        &self,
        cx: &mut Context<Self>,
        palette: Palette,
        playing: bool,
        preset: Preset,
    ) -> gpui_kit::Div {
        let volume = self.current().map(|state| state.volume).unwrap_or(0.65);
        let conductor_down = self.state.error.is_some();
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px(px(12.))
            .py(px(10.))
            .bg(rgba(palette.surface))
            .border_t_1()
            .border_color(rgba(palette.border))
            .child(transport(playing, palette, cx))
            .child(preset_chip(preset, true, palette, cx))
            .child(self.bar(
                "mini-volume",
                "volume",
                volume,
                Knob::Volume,
                palette.accent,
                palette,
                cx,
                |view, value, window, cx| view.set_volume(value, window, cx),
            ))
            .when(conductor_down, |element| {
                element.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.amber))
                        .child("offline"),
                )
            })
    }

    /// One bar: the row is the control, and a press anywhere on the track sets the value from the x
    /// position. A slider track with its own knob would cost another focus target per value and buy
    /// nothing for eleven values in a strip.
    ///
    /// The drag is GPUI's own (`on_drag` + `on_drag_move`) rather than a mouse-move handler on the
    /// track: a move handler stops firing the moment the pointer leaves a six-point-tall track, which
    /// is immediately, and a drag keeps reporting for as long as the button is down.
    #[allow(clippy::too_many_arguments)]
    fn bar<F>(
        &self,
        id: &'static str,
        label: &'static str,
        value: f32,
        knob: Knob,
        tone: u32,
        palette: Palette,
        cx: &mut Context<Self>,
        apply: F,
    ) -> impl IntoElement
    where
        F: Fn(&mut Self, f32, &mut Window, &mut Context<Self>) + Clone + 'static,
    {
        let active = self.dragging == Some(knob);
        let press = apply.clone();
        let moved = apply.clone();
        let released = apply;
        let value = value.clamp(0., 1.);
        div()
            .id(SharedString::from(format!("bar-{id}")))
            .flex_1()
            .min_w(px(76.))
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if active { palette.text } else { palette.muted }))
                            .child(label),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if active { palette.text } else { palette.muted }))
                            .child(format!("{:.0}", value * 100.)),
                    ),
            )
            .child(
                div()
                    // The cell's children are the label row and the track, so the track's own
                    // rectangle arrives here as the second painted child. No notify: this runs during
                    // prepaint, and writing a measured rectangle must not schedule a frame.
                    .on_children_prepainted({
                        let tracks = self.tracks.clone();
                        let slot = knob.index();
                        move |bounds: Vec<Bounds<Pixels>>, _, _| {
                            if let Some(track) = bounds.get(1).or_else(|| bounds.last()) {
                                tracks.borrow_mut()[slot] = Some(*track);
                            }
                        }
                    })
                    .id(SharedString::from(format!("track-{id}")))
                    .py(px(5.))
                    .cursor_pointer()
                    .child(
                        div()
                            .h(px(6.))
                            .w_full()
                            .rounded_full()
                            .bg(rgba(palette.raised))
                            .child(
                                div()
                                    .h_full()
                                    // A fraction of the track, not a pixel count: a fixed width
                                    // made every bar read short of its own value.
                                    .w(gpui_kit::relative(value))
                                    .rounded_full()
                                    .bg(rgba(tone)),
                            ),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                            let Some(bounds) = view.tracks.borrow()[knob.index()] else {
                                return;
                            };
                            view.dragging = Some(knob);
                            view.drag_span =
                                Some((f32::from(bounds.origin.x), f32::from(bounds.size.width)));
                            let value = Self::value_at(
                                f32::from(event.position.x) - f32::from(bounds.origin.x),
                                f32::from(bounds.size.width),
                            );
                            press(view, value, window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .on_drag(DragKnob(knob), |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| *drag)
                    })
                    .on_drag_move(cx.listener(
                        move |view, event: &DragMoveEvent<DragKnob>, window, cx| {
                            if event.drag(cx).0 != knob {
                                return;
                            }
                            let Some((origin, width)) = view.drag_span else {
                                return;
                            };
                            let value =
                                Self::value_at(f32::from(event.event.position.x) - origin, width);
                            moved(view, value, window, cx);
                        },
                    ))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |view, event: &MouseUpEvent, window, cx| {
                            let Some((origin, width)) = view.drag_span else {
                                return;
                            };
                            if view.dragging != Some(knob) {
                                return;
                            }
                            let value = Self::value_at(f32::from(event.position.x) - origin, width);
                            released(view, value, window, cx);
                            view.dragging = None;
                            view.drag_span = None;
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |view, _: &MouseUpEvent, _, cx| {
                            if view.dragging == Some(knob) {
                                view.dragging = None;
                                view.drag_span = None;
                                cx.notify();
                            }
                        }),
                    ),
            )
    }

    /// Hand over to the other window: open it, then close this one, so the music never stops and the
    /// operator never has two players open at once.
    fn switch_to(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if mode == self.mode {
            return;
        }
        open(cx, mode);
        window.remove_window();
    }
}

/// The drag payload of one bar. Its preview is nothing: a slider is not being carried anywhere, and
/// the drag exists only to keep reporting pointer movement.
#[derive(Clone, Copy)]
struct DragKnob(Knob);

impl Render for DragKnob {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_0()
    }
}

/// The play/pause button, shared by both windows.
fn transport(playing: bool, palette: Palette, cx: &mut Context<Soundscape>) -> AnyElement {
    div()
        .id("soundscape-transport")
        .flex_none()
        .w(px(76.))
        .h(px(28.))
        .rounded(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .cursor_pointer()
        .bg(rgba(if playing {
            palette.accent
        } else {
            palette.raised
        }))
        .text_color(rgb(if playing {
            palette.surface
        } else {
            palette.text
        }))
        .hover(move |style| style.text_color(rgb(palette.text)))
        .on_click(cx.listener(|view, _, window, cx| view.toggle(window, cx)))
        .child(if playing { "Pause" } else { "Play" })
        .into_any_element()
}

/// One preset chip. In the mini the chip is the current preset and a click moves to the next one,
/// which is the whole preset control a 400-point window can hold.
fn preset_chip(
    preset: Preset,
    chosen: bool,
    palette: Palette,
    cx: &mut Context<Soundscape>,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("preset-{}", preset.key())))
        .px_2()
        .py(px(4.))
        .rounded(px(6.))
        .text_xs()
        .cursor_pointer()
        .bg(rgba(if chosen {
            palette.accent << 8 | 0x2a
        } else {
            palette.raised
        }))
        .text_color(rgb(if chosen {
            palette.accent
        } else {
            palette.muted
        }))
        .hover(move |style| style.text_color(rgb(palette.text)))
        .on_click(cx.listener(move |view, _, window, cx| view.choose_preset(preset, window, cx)))
        .child(preset.key())
        .into_any_element()
}

/// A small text button: `clear`, `mini`, `expand`.
fn icon_button<F>(
    id: &'static str,
    label: &'static str,
    palette: Palette,
    cx: &mut Context<Soundscape>,
    apply: F,
) -> AnyElement
where
    F: Fn(&mut Soundscape, &mut Window, &mut Context<Soundscape>) + 'static,
{
    div()
        .id(SharedString::from(id.to_string()))
        .px_2()
        .py(px(3.))
        .rounded(px(5.))
        .text_xs()
        .cursor_pointer()
        .bg(rgba(palette.raised))
        .text_color(rgb(palette.muted))
        .hover(move |style| style.text_color(rgb(palette.text)))
        .on_click(cx.listener(move |view, _, window, cx| apply(view, window, cx)))
        .child(label)
        .into_any_element()
}

/// One of the timed scenarios.
fn quick<F>(
    id: &'static str,
    label: &'static str,
    palette: Palette,
    cx: &mut Context<Soundscape>,
    apply: F,
) -> AnyElement
where
    F: Fn(&mut Soundscape, &mut Window, &mut Context<Soundscape>) + 'static,
{
    icon_button(id, label, palette, cx, apply)
}

/// The two open windows, so `open` finds one instead of starting a second player.
#[derive(Default)]
struct Windows {
    full: Option<Open>,
    mini: Option<Open>,
}

struct Open {
    handle: gpui_kit::WindowHandle<gpui_kit::component::Root>,
    view: gpui_kit::WeakEntity<Soundscape>,
}

impl gpui_kit::Global for Windows {}

impl Windows {
    fn slot(&mut self, mode: Mode) -> &mut Option<Open> {
        match mode {
            Mode::Full => &mut self.full,
            Mode::Mini => &mut self.mini,
        }
    }

    /// The live window of that mode, forgetting one whose view has gone (its window was closed).
    fn live(&mut self, mode: Mode) -> Option<&Open> {
        let slot = self.slot(mode);
        match slot {
            Some(open) if open.view.upgrade().is_some() => slot.as_ref(),
            _ => {
                *slot = None;
                None
            }
        }
    }
}

/// Open one of the two windows, or raise it if it is already open.
pub fn open(cx: &mut App, mode: Mode) {
    if cx.try_global::<Windows>().is_none() {
        cx.set_global(Windows::default());
    }
    if let Some(handle) = cx
        .global_mut::<Windows>()
        .live(mode)
        .map(|open| open.handle)
    {
        handle
            .update(cx, |_, window, _| window.activate_window())
            .ok();
        return;
    }

    let size = mode.size();
    let options = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(
            gpui_kit::Bounds::centered(None, size, cx),
        )),
        ..mode.options()
    };
    let mut created = None;
    let Ok(handle) = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| Soundscape::new(mode, cx));
        created = Some(view.downgrade());
        window.activate_window();
        cx.new(|cx| gpui_kit::component::Root::new(view, window, cx).bg(rgba(0x00000000)))
    }) else {
        return;
    };
    if let Some(view) = created {
        *cx.global_mut::<Windows>().slot(mode) = Some(Open { handle, view });
    }
}

/// `⌘⇧M`: the mini player, on and off. A glanceable window has to be one keystroke from anywhere,
/// and the second press should put it away rather than raise it again.
pub fn toggle_mini(cx: &mut App) {
    if cx.try_global::<Windows>().is_none() {
        cx.set_global(Windows::default());
    }
    let existing = cx
        .global_mut::<Windows>()
        .live(Mode::Mini)
        .map(|open| open.handle);
    match existing {
        Some(handle) => {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
        None => open(cx, Mode::Mini),
    }
}

/// Bindings the windows carry. Registered once at launch, because a key binding belongs to the app
/// and the context it names ("Soundscape") is only carried by these windows' roots.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("space", ToggleTransport, Some("Soundscape")),
        gpui_kit::KeyBinding::new("shift-cmd-n", NextPreset, Some("Soundscape")),
        gpui_kit::KeyBinding::new("shift-cmd-m", OpenMini, Some("Soundscape")),
    ]);
    cx.on_action(|_: &OpenMini, cx| toggle_mini(cx));
}

/// Start playing through the conductor, for `SJEL_GPUI_SOUNDSCAPE=play`.
///
/// It goes the same way a click on Play goes -- a write to the conductor, which every other surface
/// reads -- rather than setting the local control, which the first poll would overwrite.
pub fn play_through_conductor(cx: &mut App) {
    let audio = crate::audio::audio(cx);
    audio.control.set_playing(true);
    let endpoint = std::env::var("SJEL_STATUS_URL")
        .unwrap_or_else(|_| sjel_gpui_core::shell::DEFAULT_ENDPOINT.to_owned());
    let Ok(shell) = sjel_gpui_core::shell::Shell::new(&endpoint) else {
        return;
    };
    let shell = Arc::new(shell);
    cx.spawn(async move |cx| {
        if let Some(Ok(state)) =
            shell::run(async move { shell.set_soundscape_playing(true).await }).await
        {
            cx.update(|cx| {
                let audio = crate::audio::audio(cx);
                audio.control.set_playing(state.playing);
            });
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bar_turns_a_position_into_a_fraction() {
        assert_eq!(Soundscape::value_at(0., 200.), 0.);
        assert_eq!(Soundscape::value_at(100., 200.), 0.5);
        assert_eq!(Soundscape::value_at(200., 200.), 1.);
        // A drag past the end clamps rather than wrapping.
        assert_eq!(Soundscape::value_at(260., 200.), 1.);
        assert_eq!(Soundscape::value_at(-20., 200.), 0.);
        // A zero-width track cannot divide.
        assert_eq!(Soundscape::value_at(10., 0.), 0.);
    }

    #[test]
    fn every_preset_is_reachable_from_the_strip() {
        assert_eq!(PRESETS.len(), 6);
        let keys: Vec<&str> = PRESETS.iter().map(|preset| preset.key()).collect();
        assert_eq!(
            keys,
            vec!["edm", "ambient", "lofi", "focus", "relax", "sleep"]
        );
    }

    /// Every control has its own slot in the measured-track array: two controls sharing one would
    /// map a click on one to a value on the other.
    #[test]
    fn every_control_has_its_own_track_slot() {
        let mut slots = vec![Knob::Volume.index(), Knob::Energy.index()];
        slots.extend(LayerName::ALL.map(|layer| Knob::Layer(layer).index()));
        slots.extend(ParamName::ALL.map(|name| Knob::Param(name).index()));
        slots.sort_unstable();
        let mut unique = slots.clone();
        unique.dedup();
        assert_eq!(unique.len(), slots.len(), "{slots:?}");
        assert!(
            *unique.last().expect("slots") < 14,
            "the array is sized for every control"
        );
    }

    #[test]
    fn the_mini_window_is_the_smaller_one_and_not_resizable() {
        let mini = Mode::Mini.options();
        assert!(!mini.is_resizable);
        assert!(Mode::Full.options().is_resizable);
        assert!(Mode::Mini.size().width < Mode::Full.size().width);
        assert!(Mode::Mini.size().height < Mode::Full.size().height);
    }
}
