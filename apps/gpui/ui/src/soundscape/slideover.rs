//! Desktop Soundscape player and conductor controls.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, Timelike, Utc};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, App, Bounds, Context, DragMoveEvent, FocusHandle,
    Focusable, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Render,
    SharedString, Window,
};
use sjel_gpui_core::data::{Remote, Repo};
use sjel_gpui_core::shell::{
    self, SoundscapeLayers, SoundscapeParams, SoundscapeSession, SoundscapeState,
};
use sjel_gpui_core::soundscape::{
    self, LayerName, ListeningContext, ParamName, Preset, Scenario, Visual,
};

use crate::data::{self, Busy};
use crate::ely::buttons::{Button, ButtonVariant, ToggleGroup, ToggleItem};
use crate::ely::forms::Switch;
use crate::ely::motion::ProgressBar;
use crate::ely::primitives::IconName;
use crate::ely::theme::ControlSize;
use crate::theme::{Palette, ThemeContext};
use crate::viz::{self, Field};

actions!(
    soundscape,
    [
        ToggleTransport,
        NextPreset,
        CloseDrawer,
        ToggleMix,
        WidenDrawer,
        NarrowDrawer
    ]
);

/// How wide the panel is. Narrow enough that the page behind it still reads as work in progress,
/// wide enough for two columns of labelled bars.
pub const WIDTH: f32 = 400.;
const MIN_WIDTH: f32 = 320.;
const MAX_WIDTH: f32 = 760.;
const KEY_RESIZE_STEP: f32 = 32.;

/// How often a poll reads the conductor, so another surface's change shows up here without the
/// operator doing anything.
const POLL: Duration = Duration::from_secs(3);
/// A drag writes at most this often while it moves; the last value always lands.
const WRITE_DEBOUNCE: Duration = Duration::from_millis(180);
/// How long the panel takes to arrive. One time constant, not a frame count: the motion is the same
/// at 60 Hz and 120 Hz, and short enough that the slide does not become something to wait for.
const SLIDE_TAU: f32 = 0.06;
/// The least height the drawn field keeps, enough for the preset label and the playback line.
const FIELD_HEIGHT: f32 = 190.;
const CONTEXT_POLL: Duration = Duration::from_secs(60);

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

pub struct SoundscapeSlideover {
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
    previous_focus: Option<FocusHandle>,
    /// Whether the panel is asked to be on screen, and how far in it has slid (0..1).
    open: bool,
    slide: f32,
    width: f32,
    measured_width: Rc<Cell<f32>>,
    resize_start: Option<(f32, f32)>,
    /// The twelve-value mixer, folded away by default. A 400-point column cannot hold fourteen
    /// values and a picture without becoming a wall of small type.
    mix_open: bool,
    listening_context: ListeningContext,
    last_context_read: Option<Instant>,
    session_write: Busy,
}

impl SoundscapeSlideover {
    /// Built with the workspace's repo: the panel talks to the same conductor through the same
    /// endpoint, and a second resolution of the endpoint here would be a second answer to
    /// "which shell is this app talking to".
    pub fn new_with_handles(
        repo: Repo,
        control: Arc<soundscape::Control>,
        visual: Arc<Visual>,
        device: Option<String>,
        device_error: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let handles = ControlHandles { control, visual };
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
            previous_focus: None,
            open: false,
            slide: 0.,
            width: WIDTH,
            measured_width: Rc::new(Cell::new(WIDTH)),
            resize_start: None,
            mix_open: false,
            listening_context: ListeningContext::at(Local::now().hour(), false),
            last_context_read: None,
            session_write: Busy::default(),
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
        self.refocus = open;
        if open {
            // The panel is about to be looked at, so it takes the keys: space is play/pause. The
            // focus call needs the window, so it happens on the next render.
            self.read_context(cx);
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
                return;
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
                if view.open || (view.audio.control.adapt() && view.audio.control.playing()) {
                    view.read_context(cx);
                }
                cx.notify();
            });
            if still_open.is_err() {
                return;
            }
        })
        .detach();
    }

    fn read_context(&mut self, cx: &mut Context<Self>) {
        if self
            .last_context_read
            .is_some_and(|last| last.elapsed() < CONTEXT_POLL)
        {
            return;
        }
        self.last_context_read = Some(Instant::now());
        let Ok(shell) = self.repo.shell() else {
            self.listening_context = ListeningContext::at(Local::now().hour(), false);
            self.audio
                .control
                .set_activity(self.listening_context.activity);
            return;
        };
        cx.spawn(async move |this, cx| {
            let context = shell::run(async move { shell.soundscape_context().await })
                .await
                .unwrap_or_else(|| ListeningContext::at(Local::now().hour(), false));
            let _ = this.update(cx, |view, cx| {
                view.listening_context = context;
                view.audio.control.set_activity(context.activity);
                if view.open {
                    cx.notify();
                }
            });
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
        let timing = state.session.as_ref().map(session_timing);
        self.audio
            .control
            .set_session_progress(timing.map(|(progress, _)| progress));
        self.audio
            .control
            .set_playing(state.playing && timing.is_none_or(|(_, remaining)| remaining > 0));
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.device.is_none() || self.session_write.contains("start") {
            return;
        }
        let playing = self.audio.control.playing();
        self.set_playing(!playing, window, cx);
    }

    /// The one write path for the transport. The sidebar, the palette and the global keys come here
    /// through the workspace too: two writers each with their own `last_write` guard could not see
    /// each other's write, so a poll could undo the one the other had just made.
    pub fn set_playing(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.session_write.contains("start") {
            return;
        }
        // The transport answers on the keystroke: the conductor confirms afterwards.
        self.audio.control.set_playing(next);
        if let Some(state) = self.state.value.as_mut() {
            state.playing = next;
            if let Some(session) = state.session.as_mut() {
                let now = wall_clock_ms();
                if next {
                    session.running_since_ms.get_or_insert(now);
                } else {
                    session.pause_at(now);
                }
            }
        }
        self.last_write = Some(Instant::now());
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            // A toast for a transport the operator just watched change would be noise.
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

    /// The one write path for a preset, for the same reason as [`Self::set_playing`].
    pub fn choose_preset(&mut self, preset: Preset, window: &mut Window, cx: &mut Context<Self>) {
        if self.session_write.contains("start") {
            return;
        }
        self.audio.control.set_preset(preset);
        // A scape carries its own defaults, exactly as the dashboard's preset button does.
        let defaults = preset.scape().map(|scape| scape.defaults);
        if let Some(defaults) = defaults {
            self.audio.control.set_params(defaults);
        }
        self.last_write = Some(Instant::now());
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::quiet(""),
            move |shell| async move { shell.select_soundscape_mode(preset).await },
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
        self.write_soon(window, cx, Write::Layers(layers));
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
        self.write_soon(window, cx, Write::Params(params));
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

    fn start_session(&mut self, scenario: Scenario, window: &mut Window, cx: &mut Context<Self>) {
        if self.device.is_none() {
            return;
        }
        self.last_write = Some(Instant::now());
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|view| &mut view.session_write, "start", ""),
            move |shell| async move { shell.start_soundscape_scenario(scenario).await },
            |view, updated, cx| {
                if let Some(state) = updated {
                    view.push(&state);
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
    fn offset(slide: f32, width: f32) -> f32 {
        -(1. - slide.clamp(0., 1.)) * width
    }

    fn resize(&mut self, width: f32, cx: &mut Context<Self>) {
        let width = width.clamp(MIN_WIDTH, MAX_WIDTH);
        if self.width != width {
            self.width = width;
            cx.notify();
        }
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
        if !self.open && !sliding {
            return false;
        }

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

impl Focusable for SoundscapeSlideover {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SoundscapeSlideover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        self.adopt();
        self.animate(window, cx);
        if std::mem::take(&mut self.refocus) {
            self.previous_focus = window.focused(cx).filter(|focus| focus != &self.focus);
            window.focus(&self.focus, cx);
        } else if !self.open {
            if let Some(previous) = self.previous_focus.take() {
                if self.focus.contains_focused(window, cx) {
                    window.focus(&previous, cx);
                }
            }
        }

        let state = self.current();
        let playing = self.audio.control.playing();
        let preset = state
            .map(|state| state.preset())
            .unwrap_or_else(|| self.audio.visual.preset());
        if !self.open && self.slide <= 0. {
            return div().into_any_element();
        }

        let field_height = FIELD_HEIGHT + (self.width - WIDTH).max(0.) * 0.2;
        let field = div()
            .h(px(field_height))
            .flex_none()
            .rounded(px(10.))
            .overflow_hidden()
            .when(!playing, |field| field.opacity(0.62))
            .child(viz::frame(viz::canvas(
                self.field.clone(),
                self.audio.visual.clone(),
            )));

        div()
            .on_children_prepainted({
                let measured_width = self.measured_width.clone();
                move |children, _, _| {
                    if let Some(header) = children.first() {
                        measured_width.set(f32::from(header.size.width) + 1.);
                    }
                }
            })
            .key_context("Soundscape")
            .track_focus(&self.focus)
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(Self::offset(self.slide, self.width)))
            .w(px(self.width))
            .max_w(gpui_kit::relative(0.9))
            .flex()
            .flex_col()
            .bg(rgba(palette.panel()))
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
            .on_action(cx.listener(|view, _: &ToggleMix, _, cx| {
                view.mix_open = !view.mix_open;
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &WidenDrawer, _, cx| {
                view.resize(view.measured_width.get() + KEY_RESIZE_STEP, cx);
            }))
            .on_action(cx.listener(|view, _: &NarrowDrawer, _, cx| {
                view.resize(view.measured_width.get() - KEY_RESIZE_STEP, cx);
            }))
            .child(self.header(palette, cx))
            .child(
                div()
                    .id("soundscape-controls")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .px(px(16.))
                            .pb(px(16.))
                            .child(field)
                            .child(self.controls(cx, palette, playing, preset)),
                    ),
            )
            .child(self.resize_grip(palette, cx))
            .into_any_element()
    }
}

impl SoundscapeSlideover {
    fn resize_grip(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("soundscape-resize")
            .debug_selector(|| "soundscape-resize".into())
            .absolute()
            .left_0()
            .top_0()
            .bottom_0()
            .w(px(8.))
            .cursor_col_resize()
            .hover(move |style| style.bg(rgba(crate::theme::tint(palette.accent, 0x26))))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        view.resize_start = None;
                        view.resize(WIDTH, cx);
                    } else {
                        view.resize_start =
                            Some((f32::from(event.position.x), view.measured_width.get()));
                    }
                    cx.stop_propagation();
                }),
            )
            .on_drag(ResizeDrawer, |_, _, _, cx| cx.new(|_| gpui_kit::EmptyView))
            .on_drag_move(
                cx.listener(|view, event: &DragMoveEvent<ResizeDrawer>, _, cx| {
                    if let Some((start_x, initial_width)) = view.resize_start {
                        view.resize(
                            initial_width + start_x - f32::from(event.event.position.x),
                            cx,
                        );
                    }
                }),
            )
    }

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
                                (_, Some(error), _) => device_line(error),
                                (Some(device), _, false) => device.clone(),
                                (Some(device), _, true) => {
                                    format!("{device} · conductor unreachable")
                                }
                                (None, _, _) => "no output device".to_owned(),
                            }),
                    ),
            )
            .child(
                quiet_button("soundscape-close", "Close", cx, |view, _, cx| {
                    view.set_open(false, cx)
                })
                .shortcut("escape"),
            )
    }

    /// Playback controls, timed sessions, and the disclosed mix.
    fn controls(
        &self,
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
        let pending = playing && self.audio.visual.preset() != preset;
        let session = state.and_then(|state| state.session.as_ref());
        let status = if pending {
            "Changing soundscape".to_owned()
        } else if playing && preset.scape().is_some() {
            super::controls::title_case(self.audio.visual.phase().label())
        } else if playing {
            "Playing".to_owned()
        } else {
            "Paused".to_owned()
        };

        let mut strip = div().flex().flex_col().gap(px(16.));
        let now_playing = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(28.))
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(preset.label()),
                            )
                            .child(div().text_sm().text_color(rgb(palette.muted)).child(status)),
                    )
                    .child(
                        Button::new(
                            "soundscape-transport",
                            if playing { "Pause" } else { "Play" },
                        )
                        .variant(ButtonVariant::Primary)
                        .icon(if playing {
                            IconName::Pause
                        } else {
                            IconName::Play
                        })
                        .disabled(self.device.is_none() || self.session_write.contains("start"))
                        .on_click(cx.listener(|view, _, window, cx| view.toggle(window, cx))),
                    ),
            )
            .when_some(session, |player, session| {
                let (progress, remaining_ms) = session_timing(session);
                let minutes = remaining_ms.div_ceil(60_000);
                let label = Scenario::ALL
                    .iter()
                    .find(|scenario| scenario.key == session.scenario)
                    .map_or("Timer", |scenario| scenario.label);
                player.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(palette.text))
                                        .child(format!("{label} · {minutes} min left")),
                                )
                                .child(quiet_button(
                                    "session-clear",
                                    "Remove timer",
                                    cx,
                                    |view, window, cx| {
                                        view.clear_session(window, cx);
                                    },
                                )),
                        )
                        .child(ProgressBar::new(
                            "soundscape-session-progress",
                            (progress * 100.).floor() / 100.,
                        )),
                )
            })
            .child(self.bar(
                "volume",
                "Volume",
                volume,
                Knob::Volume,
                palette.accent,
                palette,
                cx,
                |view, value, window, cx| view.set_volume(value, window, cx),
            ));
        strip = strip.child(now_playing);

        let modes = [Preset::Focus, Preset::Relax, Preset::Sleep]
            .into_iter()
            .fold(
                ToggleGroup::new("soundscape-modes").selected([preset.key()]),
                |control, mode| control.item(ToggleItem::new(mode.key()).label(mode.label())),
            );
        strip = strip.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .pt_3()
                .border_t_1()
                .border_color(rgba(palette.border))
                .child(
                    modes.on_change(cx.listener(|view, keys: &[SharedString], window, cx| {
                        if let Some(preset) =
                            keys.first().and_then(|key| Preset::from_key(key.as_ref()))
                        {
                            view.choose_preset(preset, window, cx);
                        }
                    })),
                )
                .child(div().flex().gap_1().children(
                    [Preset::Ambient, Preset::Lofi, Preset::Edm].map(|candidate| {
                        Button::new(candidate.key(), candidate.label())
                            .size(ControlSize::Sm)
                            .variant(if preset == candidate {
                                ButtonVariant::Subtle
                            } else {
                                ButtonVariant::Ghost
                            })
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.choose_preset(candidate, window, cx);
                            }))
                    }),
                )),
        );

        if session.is_none() {
            let suggestion = self.listening_context;
            strip = strip.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .child("Timed sessions"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} · try {}",
                                suggestion.reason, suggestion.scenario.label
                            )),
                    )
                    .child(
                        div()
                            .grid()
                            .grid_cols(2)
                            .gap_2()
                            .children(Scenario::ALL.map(|scenario| {
                                Button::new(
                                    scenario.key,
                                    format!("{} · {} min", scenario.label, scenario.minutes),
                                )
                                .size(ControlSize::Sm)
                                .full_width()
                                .variant(if scenario.key == suggestion.scenario.key {
                                    ButtonVariant::Subtle
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .disabled(
                                    self.device.is_none() || self.session_write.contains("start"),
                                )
                                .on_click(cx.listener(
                                    move |view, _, window, cx| {
                                        view.start_session(scenario, window, cx);
                                    },
                                ))
                            })),
                    ),
            );
        }

        let change_adapt = cx.listener(|view, on: &bool, _, cx| {
            view.audio.control.set_adapt(*on);
            cx.notify();
        });
        strip = strip.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .pt_3()
                .border_t_1()
                .border_color(rgba(palette.border))
                .when(preset.scape().is_some(), |strip| {
                    strip.child(
                        Switch::new("soundscape-adaptive-mix", self.audio.control.adapt())
                            .label("Adaptive mix")
                            .on_change(move |on, window, cx| change_adapt(&on, window, cx)),
                    )
                })
                .child(quiet_button(
                    "soundscape-mix-toggle",
                    if self.mix_open {
                        "Hide mixer"
                    } else {
                        "Show mixer"
                    },
                    cx,
                    |view, _, cx| {
                        view.mix_open = !view.mix_open;
                        cx.notify();
                    },
                )),
        );

        if self.mix_open {
            // Two explicit columns. A grid child sized to its content pushed the sixth bar out of
            // the panel and made the whole strip taller than the window.
            let mut layers_bars: Vec<gpui_kit::Stateful<gpui_kit::Div>> = LayerName::ALL
                .iter()
                .filter(|&&layer| {
                    layer != LayerName::Drums || !matches!(preset, Preset::Relax | Preset::Sleep)
                })
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
                .filter(|&&name| {
                    name != ParamName::Pulse || !matches!(preset, Preset::Relax | Preset::Sleep)
                })
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
                .child(self.bar(
                    "energy",
                    "Energy",
                    energy,
                    Knob::Energy,
                    palette.amber,
                    palette,
                    cx,
                    |view, value, window, cx| view.set_energy(value, window, cx),
                ))
                .child(section("Layers", palette, columns(&mut layers_bars)))
                .child(section("Scene", palette, columns(&mut scene_bars)))
                .when_some(state.map(|state| state.seed), |strip, seed| {
                    strip.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("Seed {}", soundscape::format_seed(seed))),
                    )
                });
        }

        strip
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
                                    .bg(rgb(tone)),
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

#[derive(Clone, Copy)]
struct ResizeDrawer;

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

fn session_timing(session: &SoundscapeSession) -> (f32, u64) {
    let remaining = session.remaining_at(wall_clock_ms());
    let elapsed = session.duration_ms.saturating_sub(remaining);
    (
        elapsed as f32 / session.duration_ms.max(1) as f32,
        remaining,
    )
}

fn wall_clock_ms() -> u64 {
    Utc::now().timestamp_millis().max(0) as u64
}

fn quiet_button<F>(
    id: &'static str,
    label: &'static str,
    cx: &mut Context<SoundscapeSlideover>,
    apply: F,
) -> Button
where
    F: Fn(&mut SoundscapeSlideover, &mut Window, &mut Context<SoundscapeSlideover>) + 'static,
{
    Button::new(id, label)
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Ghost)
        .on_click(cx.listener(move |view, _, window, cx| apply(view, window, cx)))
}

/// The drag payload needs to carry the knob it belongs to; the write it applies comes from the
/// closure on each bar, which is why nothing here holds a `Write`.
impl SoundscapeSlideover {
    /// Bindings the panel carries. Registered once at launch, because a key binding belongs to the
    /// app and the context it names ("Soundscape") is only carried by the panel's root.
    pub fn keys() -> [gpui_kit::KeyBinding; 6] {
        [
            gpui_kit::KeyBinding::new("space", ToggleTransport, Some("Soundscape")),
            gpui_kit::KeyBinding::new("shift-cmd-n", NextPreset, Some("Soundscape")),
            gpui_kit::KeyBinding::new("escape", CloseDrawer, Some("Soundscape")),
            gpui_kit::KeyBinding::new("m", ToggleMix, Some("Soundscape")),
            gpui_kit::KeyBinding::new("alt-left", WidenDrawer, Some("Soundscape")),
            gpui_kit::KeyBinding::new("alt-right", NarrowDrawer, Some("Soundscape")),
        ]
    }
}

/// The header's line for a device that did not open. The audio module's error names the cause for
/// a log; the panel says what it means and keeps the cause in brackets.
fn device_line(error: &str) -> String {
    if error.starts_with("SJEL_GPUI_NO_AUDIO") {
        return "Sound is off for this launch (SJEL_GPUI_NO_AUDIO)".to_owned();
    }
    format!("No sound output ({error})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_drawer_resizes_from_its_edge_and_closes_with_escape(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| cx.bind_keys(SoundscapeSlideover::keys()));
        let (player, cx) = crate::test_support::mount_root(
            cx,
            Some(gpui_kit::size(px(900.), px(740.))),
            |_, cx| {
                let mut player = SoundscapeSlideover::new_with_handles(
                    Repo::new(Err(shell::ShellError::NotSetUp)),
                    Arc::new(soundscape::Control::new()),
                    Arc::new(Visual::new()),
                    None,
                    None,
                    cx,
                );
                player.open = true;
                player.slide = 1.;
                player.refocus = true;
                player
            },
        );
        let left_edge = cx
            .debug_bounds("soundscape-resize")
            .expect("resize grip")
            .center();
        cx.simulate_mouse_down(left_edge, MouseButton::Left, Default::default());
        for offset in [16., 80.] {
            cx.simulate_mouse_move(
                gpui_kit::point(left_edge.x - px(offset), left_edge.y),
                MouseButton::Left,
                Default::default(),
            );
        }
        cx.simulate_mouse_up(
            gpui_kit::point(left_edge.x - px(80.), left_edge.y),
            MouseButton::Left,
            Default::default(),
        );
        cx.run_until_parked();
        assert_eq!(player.read_with(cx, |player, _| player.width), WIDTH + 80.);
        cx.simulate_keystrokes("alt-right");
        assert_eq!(
            player.read_with(cx, |player, _| player.width),
            WIDTH + 80. - KEY_RESIZE_STEP
        );
        cx.simulate_keystrokes("escape");
        assert!(!player.read_with(cx, |player, _| player.is_open()));
    }

    #[gpui::test]
    fn session_progress_renders_and_adaptation_keeps_the_preset(cx: &mut gpui_kit::TestAppContext) {
        let (player, cx) = crate::test_support::mount_root(
            cx,
            Some(gpui_kit::size(px(800.), px(740.))),
            |_, cx| {
                let mut player = SoundscapeSlideover::new_with_handles(
                    Repo::new(Err(shell::ShellError::NotSetUp)),
                    Arc::new(soundscape::Control::new()),
                    Arc::new(Visual::new()),
                    Some("Test output".into()),
                    None,
                    cx,
                );
                player.state.value = Some(SoundscapeState {
                    preset: Preset::Focus.key().into(),
                    session: Some(SoundscapeSession {
                        scenario: Scenario::DEEP_WORK.key.into(),
                        duration_ms: 3_000_000,
                        elapsed_ms: 1_500_000,
                        running_since_ms: None,
                    }),
                    ..SoundscapeState::default()
                });
                player.open = true;
                player.slide = 1.;
                player
            },
        );
        assert!(cx.debug_bounds("progress-fill-0").is_some());

        let toggle = cx
            .debug_bounds("switch-label")
            .expect("adaptive mix switch");
        cx.simulate_click(toggle.center(), gpui_kit::Modifiers::none());
        cx.run_until_parked();
        player.read_with(cx, |player, _| {
            assert!(player.audio.control.adapt());
            assert_eq!(player.audio.control.preset(), Preset::Focus);
            assert_eq!(player.audio.control.session_progress(), Some(0.5));
        });
    }

    #[test]
    fn a_disabled_device_reads_as_a_sentence() {
        assert_eq!(
            device_line("SJEL_GPUI_NO_AUDIO is set: no output device"),
            "Sound is off for this launch (SJEL_GPUI_NO_AUDIO)"
        );
        assert!(device_line("device busy").starts_with("No sound output"));
    }

    #[test]
    fn a_bar_turns_a_position_into_a_fraction() {
        assert_eq!(SoundscapeSlideover::value_at(0., 200.), 0.);
        assert_eq!(SoundscapeSlideover::value_at(100., 200.), 0.5);
        assert_eq!(SoundscapeSlideover::value_at(200., 200.), 1.);
        // A drag past the end clamps rather than wrapping.
        assert_eq!(SoundscapeSlideover::value_at(260., 200.), 1.);
        assert_eq!(SoundscapeSlideover::value_at(-20., 200.), 0.);
        // A zero-width track cannot divide.
        assert_eq!(SoundscapeSlideover::value_at(10., 0.), 0.);
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
        assert_eq!(SoundscapeSlideover::offset(0., WIDTH), -WIDTH);
        assert_eq!(SoundscapeSlideover::offset(1., WIDTH), 0.);
        assert!(
            SoundscapeSlideover::offset(0.5, WIDTH) < 0.,
            "halfway is still outside"
        );
        // Clamped, so a bad value cannot push the panel over the page it sits on.
        assert_eq!(SoundscapeSlideover::offset(2., WIDTH), 0.);
        assert_eq!(SoundscapeSlideover::offset(-1., WIDTH), -WIDTH);
    }

    /// The panel has to be narrower than the window it slides over, or there is no page left to
    /// write in while it is open.
    #[test]
    fn the_panel_leaves_the_page_most_of_the_window() {
        const { assert!(WIDTH < 760., "the window's own minimum width") };
        assert_eq!(SoundscapeSlideover::keys().len(), 6);
    }
}
