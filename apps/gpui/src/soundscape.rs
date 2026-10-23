//! The soundscape slideover: the player, sliding in over the page it belongs to.
//!
//! Why a drawer and not a window. A soundscape is something you leave on while you write, not a
//! place you go: a second window takes focus from the note, has to be found in the window list, and
//! at 980 by 700 it was never the size of the thing it contained. Between that window and nothing
//! there was no third option -- the sidebar strip can cycle a preset and start the transport, but
//! it cannot show what is playing. So the player is a panel that slides in from the right edge of
//! the main window, over the page, and slides away again. It is the same window, so nothing steals
//! focus; the audio lives in a `Global`, so closing the drawer does not cut the music.
//!
//! What the panel holds is chosen for a 400-point column rather than a wide window: the drawn
//! field at the top, the transport and the six presets under it, volume and energy as full-width
//! bars, and the twelve-value mixer folded behind one disclosure. Everything the old window showed
//! is still reachable; nothing is shown twice.
//!
//! Every control here writes a *patch*, never a whole state: the dashboard, the phone and this
//! panel drive the same session at once, and a whole-state write from a stale copy would silently
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

actions!(soundscape, [ToggleTransport, NextPreset, CloseDrawer]);

/// How wide the panel is. Narrow enough that the page behind it still reads as work in progress,
/// wide enough for two columns of labelled bars.
pub const WIDTH: f32 = 400.;

/// How often a poll reads the conductor, so another surface's change shows up here without the
/// operator doing anything.
const POLL: Duration = Duration::from_secs(3);
/// A drag writes at most this often while it moves; the last value always lands.
const WRITE_DEBOUNCE: Duration = Duration::from_millis(180);
/// How long the panel takes to arrive. One time constant, not a frame count: the motion is the same
/// at 60 Hz and 120 Hz, and short enough that the slide does not become something to wait for.
const SLIDE_TAU: f32 = 0.06;

/// The six presets the conductor knows, in its own order.
const PRESETS: [Preset; 6] = [
    Preset::Edm,
    Preset::Ambient,
    Preset::Lofi,
    Preset::Focus,
    Preset::Relax,
    Preset::Sleep,
];

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
/// global so that closing the drawer does not stop the music.
struct ControlHandles {
    control: Arc<soundscape::Control>,
    visual: Arc<Visual>,
}

pub struct Soundscape {
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
    /// Somewhere to put the focus on the next render: `set_open` has no window.
    refocus: bool,
    /// Whether the panel is asked to be on screen, and how far in it has slid (0..1).
    open: bool,
    slide: f32,
    /// The twelve-value mixer, folded away by default. A 400-point column cannot hold fourteen
    /// values and a picture without becoming a wall of small type.
    mix_open: bool,
}

impl Soundscape {
    /// Built with the workspace's repo: the panel talks to the same conductor through the same
    /// endpoint, and a second resolution of `SJEL_STATUS_URL` here would be a second answer to
    /// "which shell is this app talking to".
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        let audio = crate::audio::audio(cx);
        let handles = ControlHandles {
            control: audio.control.clone(),
            visual: audio.visual.clone(),
        };
        let device = audio.device_name().map(|name| match audio.sample_rate() {
            Some(rate) => format!("{name} · {rate:.0} Hz"),
            None => name.to_owned(),
        });
        let device_error = audio.error().map(str::to_owned);
        let mut view = Self {
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
            refocus: false,
            open: false,
            slide: 0.,
            mix_open: false,
        };
        view.read_state(cx);
        view.start_polling(cx);
        view
    }

    /// Slide the panel in, or away. The single place `open` changes, so the workspace, the sidebar,
    /// the menu and the keyboard cannot disagree about whether the player is on screen.
    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if open {
            // The panel is about to be looked at, so it takes the keys: space is play/pause. The
            // focus call needs the window, so it happens on the next render.
            self.refocus = true;
        }
        cx.notify();
    }

    pub fn toggle_open(&mut self, cx: &mut Context<Self>) {
        let open = self.open;
        self.set_open(!open, cx);
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

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

    /// How far the panel has to sit from the window's right edge at a given amount of arrival: 0
    /// when it is open, its own width when it is fully away.
    fn offset(slide: f32) -> f32 {
        -(1. - slide.clamp(0., 1.)) * WIDTH
    }

    /// Advance the field and the slide, and decide whether another frame is needed.
    ///
    /// `reduce_motion` is the system's own answer to "stop this": the picture then follows the sound
    /// on the frames the window already draws (a poll, a click) instead of driving a loop of its own,
    /// and the panel arrives at once rather than sliding.
    fn animate(&mut self, window: &mut Window, cx: &Context<Self>) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        let reduce_motion = cx.reduce_motion();

        // The slide is a time constant and not a frame count, the same rule the field follows: a
        // 120 Hz window and a 60 Hz one show the same gesture.
        let target = if self.open { 1. } else { 0. };
        if reduce_motion {
            self.slide = target;
        } else if self.slide != target {
            self.slide += (target - self.slide) * (1. - (-dt / SLIDE_TAU).exp());
            // Below a pixel and a half there is nothing left to see: land, so the loop can stop.
            if (target - self.slide).abs() < 0.004 {
                self.slide = target;
            }
        }
        let sliding = self.slide != target;

        let preset = self
            .current()
            .map(|state| state.preset())
            .unwrap_or_else(|| self.audio.visual.preset());
        let moving = self
            .field
            .borrow_mut()
            .advance(&self.audio.visual, preset, dt);
        if (moving || sliding) && !reduce_motion {
            window.request_animation_frame();
            return true;
        }
        moving || sliding
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
        if std::mem::take(&mut self.refocus) {
            window.focus(&self.focus, cx);
        }

        let state = self.current();
        let playing = state.map(|state| state.playing).unwrap_or(false);
        let preset = state
            .map(|state| state.preset())
            .unwrap_or_else(|| self.audio.visual.preset());
        let playback = soundscape::Playback::read(&self.audio.visual, playing);

        // Fully away and not wanted: draw nothing at all. The view stays alive because the audio
        // does, and the next `set_open` brings it back on the next frame.
        if !self.open && self.slide <= 0. {
            return div().into_any_element();
        }

        let field = div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(viz::frame(viz::canvas(
                self.field.clone(),
                self.audio.visual.clone(),
            )))
            .child(
                div()
                    .absolute()
                    .left(px(12.))
                    .bottom(px(10.))
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
            );

        div()
            .key_context("Soundscape")
            .track_focus(&self.focus)
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(Soundscape::offset(self.slide)))
            .w(px(WIDTH))
            .flex()
            .flex_col()
            .bg(rgb(palette.panel()))
            .border_l_1()
            .border_color(rgba(palette.border))
            // A page on white and a panel on near-white are one rectangle without this: the shadow
            // is what says the panel is in front of the page rather than beside it.
            .shadow(vec![gpui_kit::BoxShadow::new(
                px(-6.),
                px(0.),
                gpui_kit::hsla(0., 0., 0., 0.10),
            )
            .blur_radius(px(20.))
            .spread_radius(px(-4.))])
            .on_action(cx.listener(|view, _: &ToggleTransport, window, cx| {
                view.toggle(window, cx);
            }))
            .on_action(cx.listener(|view, _: &NextPreset, window, cx| {
                view.next_preset(window, cx);
            }))
            .on_action(cx.listener(|view, _: &CloseDrawer, _, cx| {
                view.set_open(false, cx);
            }))
            .child(self.header(palette, cx))
            .child(field)
            .child(self.controls(window, cx, palette, playing, preset))
            .into_any_element()
    }
}

impl Soundscape {
    /// The top of the panel: what this is, what it is playing through, and the way out.
    fn header(&self, palette: Palette, cx: &mut Context<Self>) -> gpui_kit::Div {
        let conductor_down = self.state.error.is_some();
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px(px(14.))
            .pt(px(12.))
            .pb(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .child("Soundscape"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(rgb(if conductor_down {
                                palette.amber
                            } else {
                                palette.muted
                            }))
                            .child(match (&self.device, &self.device_error, conductor_down) {
                                (_, Some(error), _) => error.clone(),
                                (Some(device), _, false) => device.clone(),
                                (Some(device), _, true) => {
                                    format!("{device} · conductor unreachable")
                                }
                                (None, _, _) => "no output device".to_owned(),
                            }),
                    ),
            )
            .child(icon_button(
                "soundscape-close",
                "×",
                palette,
                cx,
                |view, _, cx| view.set_open(false, cx),
            ))
    }

    /// The strip under the field: transport, presets, the two always-visible values, then the mixer
    /// folded behind one disclosure, then the session.
    fn controls(
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
            .px(px(14.))
            .py(px(12.))
            .bg(rgba(palette.surface))
            .border_t_1()
            .border_color(rgba(palette.border));

        strip = strip.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(transport(playing, palette, cx))
                .child(div().flex_1())
                .child(icon_button(
                    "soundscape-next",
                    "next",
                    palette,
                    cx,
                    |view, window, cx| view.next_preset(window, cx),
                )),
        );

        strip = strip.child(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(
                    PRESETS
                        .map(|candidate| preset_chip(candidate, candidate == preset, palette, cx)),
                )
                // Under the chips rather than among them: a label in the row pushed the sixth
                // preset onto a second line on its own.
                .when(pending, |element| {
                    // The synth changes on the next bar, so the panel saying one thing and the ears
                    // another for half a second is worth naming.
                    element.child(
                        div()
                            .px_1()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child("on the next bar"),
                    )
                }),
        );

        strip = strip.child(
            div()
                .flex()
                .gap_3()
                .child(
                    self.bar(
                        "volume",
                        "volume",
                        volume,
                        Knob::Volume,
                        palette.accent,
                        palette,
                        cx,
                        |view, value, window, cx| view.set_volume(value, window, cx),
                    )
                    .flex_1(),
                )
                .child(
                    self.bar(
                        "energy",
                        "energy",
                        energy,
                        Knob::Energy,
                        palette.amber,
                        palette,
                        cx,
                        |view, value, window, cx| view.set_energy(value, window, cx),
                    )
                    .flex_1(),
                ),
        );

        // The disclosure. Twelve more values do not belong on screen at once in a column this
        // narrow, and the six presets already cover the common change.
        strip = strip.child(
            div()
                .id("soundscape-mix-toggle")
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .rounded(px(5.))
                .when(self.mix_open, |element| element.bg(rgba(palette.raised)))
                .px(px(6.))
                .py(px(4.))
                .on_click(cx.listener(|view, _, _, cx| {
                    view.mix_open = !view.mix_open;
                    cx.notify();
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(if self.mix_open {
                            "mix  ▾"
                        } else {
                            "mix  ▸"
                        }),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("6 layers · 6 scene values"),
                ),
        );

        if self.mix_open {
            // Two explicit columns. A grid child sized to its content pushed the sixth bar out of
            // the panel and made the whole strip taller than the window.
            let mut layers_bars: Vec<gpui_kit::Stateful<gpui_kit::Div>> = LayerName::ALL
                .iter()
                .map(|layer| {
                    let layer = *layer;
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
                })
                .collect();
            let mut scene_bars: Vec<gpui_kit::Stateful<gpui_kit::Div>> = ParamName::ALL
                .iter()
                .map(|name| {
                    let name = *name;
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
                })
                .collect();
            strip = strip
                .child(section("layers", palette, columns(&mut layers_bars)))
                .child(section("scene", palette, columns(&mut scene_bars)));
        }

        let session = state.and_then(|state| state.session.clone());
        strip = strip.child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap_1()
                .text_xs()
                .text_color(rgb(palette.muted))
                .children(match session {
                    Some(session) => {
                        // The conductor stores elapsed time as a checkpoint plus the moment the
                        // session started; a panel that showed only the checkpoint would always say
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
                        vec![
                            div()
                                .text_color(rgb(palette.text))
                                .child(format!("{} · {remaining} min left", session.scenario))
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
        strip = strip.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(rgb(palette.muted))
                .when_some(state.map(|state| state.seed), |element, seed| {
                    element.child(format!("seed {}", soundscape::format_seed(seed)))
                })
                .when(conductor_down, |element| {
                    element.child(
                        div()
                            .text_color(rgb(palette.amber))
                            .child("playing the last known state"),
                    )
                }),
        );

        strip.child(
            div()
                .flex()
                .gap_3()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("space play/pause")
                .child("⇧⌘N next preset")
                .child("esc close"),
        )
    }

    /// One bar: the row is the control, and a press anywhere on the track sets the value from the x
    /// position. A slider track with its own knob would cost another focus target per value and buy
    /// nothing for fourteen values.
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
    ) -> gpui_kit::Stateful<gpui_kit::Div>
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
            .min_w(px(0.))
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
                            .h(px(5.))
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

/// Bars laid out two to a row.
///
/// Six values in one column make the panel taller than the window it slides over; two columns fit
/// inside it and stay readable at 400 points, where a bar is still about 180 wide.
fn columns(bars: &mut Vec<gpui_kit::Stateful<gpui_kit::Div>>) -> gpui_kit::Div {
    let mut column = div().flex().flex_col().gap_2();
    while !bars.is_empty() {
        let left = bars.remove(0).flex_1();
        let mut row = div().flex().gap_3().child(left);
        if !bars.is_empty() {
            row = row.child(bars.remove(0).flex_1());
        }
        column = column.child(row);
    }
    column
}

/// A labelled group inside the disclosure.
fn section(label: &'static str, palette: Palette, child: gpui_kit::Div) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(rgb(palette.muted)).child(label))
        .child(child)
}

/// The play/pause button.
fn transport(playing: bool, palette: Palette, cx: &mut Context<Soundscape>) -> AnyElement {
    div()
        .id("soundscape-transport")
        .flex_none()
        .w(px(88.))
        .h(px(30.))
        .rounded(px(15.))
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

/// One preset chip.
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

/// A small text button: `clear`, `next`, `×`.
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

/// The drag payload needs to carry the knob it belongs to; the write it applies comes from the
/// closure on each bar, which is why nothing here holds a `Write`.
impl Soundscape {
    /// Bindings the panel carries. Registered once at launch, because a key binding belongs to the
    /// app and the context it names ("Soundscape") is only carried by the panel's root.
    pub fn keys() -> [gpui_kit::KeyBinding; 3] {
        [
            gpui_kit::KeyBinding::new("space", ToggleTransport, Some("Soundscape")),
            gpui_kit::KeyBinding::new("shift-cmd-n", NextPreset, Some("Soundscape")),
            gpui_kit::KeyBinding::new("escape", CloseDrawer, Some("Soundscape")),
        ]
    }
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
    fn every_preset_is_reachable_from_the_panel() {
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

    /// The slide starts with the panel entirely off the right edge and ends flush to it, so the
    /// motion is one number and the two ends are the two states.
    #[test]
    fn the_panel_slides_from_off_the_edge_to_flush() {
        assert_eq!(Soundscape::offset(0.), -WIDTH);
        assert_eq!(Soundscape::offset(1.), 0.);
        assert!(Soundscape::offset(0.5) < 0., "halfway is still outside");
        // Clamped, so a bad value cannot push the panel over the page it sits on.
        assert_eq!(Soundscape::offset(2.), 0.);
        assert_eq!(Soundscape::offset(-1.), -WIDTH);
    }

    /// The panel has to be narrower than the window it slides over, or there is no page left to
    /// write in while it is open.
    #[test]
    fn the_panel_leaves_the_page_most_of_the_window() {
        assert!(WIDTH < 760., "the window's own minimum width");
        assert_eq!(Soundscape::keys().len(), 3);
    }
}
