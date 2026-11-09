//! Native soundscape controls over the same audio-thread atomics as the desktop drawer.
use crate::{
    nsh_ui::{action, caption, heading, snapshot},
    theme::SoundscapePrefs,
};
use nsh_core::prelude::*;
use sjel_gpui_core::{
    shell,
    soundscape::{Control, LayerName, Playback, Preset, Visual, BANDS},
};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::watch;

#[derive(Clone, Default)]
struct Snapshot {
    report: String,
    device: String,
    error: Option<String>,
    playing: bool,
    mixer: bool,
    bands: [f32; BANDS],
}
struct State {
    control: Arc<Control>,
    visual: Arc<Visual>,
    prefs: SoundscapePrefs,
    snap: Snapshot,
    visible: bool,
    generation: u64,
}
struct Feed {
    state: Mutex<State>,
    tx: watch::Sender<Snapshot>,
}
fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let (tx, _) = watch::channel(Snapshot::default());
        Feed {
            state: Mutex::new(State {
                control: Arc::new(Control::new()),
                visual: Arc::new(Visual::new()),
                prefs: SoundscapePrefs::default(),
                snap: Snapshot::default(),
                visible: false,
                generation: 0,
            }),
            tx,
        }
    })
}
fn state() -> std::sync::MutexGuard<'static, State> {
    feed().state.lock().unwrap_or_else(|e| e.into_inner())
}
fn publish(s: &mut State) {
    s.snap.report = Playback::read(&s.visual, s.control.playing()).describe();
    s.snap.playing = s.control.playing();
    s.snap.mixer = s.prefs.mixer;
    s.snap.bands = s.visual.bands();
    feed().tx.send_replace(s.snap.clone());
}
fn save() {
    let prefs = state().prefs.clone();
    shell::spawn(async move {
        let result = tokio::task::spawn_blocking(move || prefs.save()).await;
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(error) => Some(error.to_string()),
        };
        if let Some(error) = error {
            let mut s = state();
            s.snap.error = Some(error);
            publish(&mut s);
        }
    });
}
/// Shares the phone's audio device conductor and analyser, preserving saved preset and volume.
pub fn set_parts(control: Arc<Control>, visual: Arc<Visual>, prefs: SoundscapePrefs) {
    let mut s = state();
    control.set_volume(prefs.volume.clamp(0., 1.));
    if let Some(preset) = Preset::from_key(&prefs.preset) {
        control.set_preset(preset);
    }
    s.control = control;
    s.visual = visual;
    s.prefs = prefs;
    publish(&mut s);
}
/// Displays the native audio route, or why it could not open.
pub fn set_device(result: Result<String, String>) {
    let mut s = state();
    match result {
        Ok(device) => {
            s.snap.device = device;
            s.snap.error = None;
        }
        Err(error) => {
            s.snap.error = Some(error);
            s.control.set_playing(false);
        }
    }
    publish(&mut s);
}
/// Polls analyser data only while the soundscape seat is visible.
pub fn show(visible: bool) {
    let generation = {
        let mut s = state();
        s.visible = visible;
        s.generation += 1;
        publish(&mut s);
        s.generation
    };
    if visible {
        shell::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let mut s = state();
                if !s.visible || s.generation != generation {
                    return;
                }
                publish(&mut s);
            }
        });
    }
}
/// Closing the side panel pauses playback without ending the device's audio session.
pub fn pause() {
    let mut s = state();
    s.control.set_playing(false);
    publish(&mut s);
}
fn knob(
    label: &'static str,
    read: impl Fn(&Control) -> f32 + 'static,
    write: impl Fn(&Control, f32) + 'static,
) -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    view().gap_1().child(caption(label)).child(
        nsh_base::slider()
            .value(move || {
                snap.with(|_| ());
                Some(nsh_base::SliderValue::Single(f64::from(read(
                    &state().control,
                ))))
            })
            .min(0.)
            .max(1.)
            .step(0.01)
            .h(px(44.))
            .track_class(Some("h-2 w-full rounded-full bg-muted".into()))
            .range_class(Some("rounded-full bg-primary".into()))
            .thumb_class(Some("size-5 rounded-full bg-primary".into()))
            .on_change(move |value: &nsh_base::SliderValue| {
                let mut s = state();
                write(&s.control, value.start() as f32);
                if label == "Volume" {
                    s.prefs.volume = value.start() as f32;
                }
                publish(&mut s);
            })
            .on_change_end(|_: &nsh_base::SliderValue| save()),
    )
}
/// Local playback, six presets, volume, mixer layers and a real analyser meter.
#[component]
pub fn SoundscapeScreen() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let mixer = computed(move || snap.with(|s| s.mixer));
    let mut presets = view().flex_row().flex_wrap();
    for preset in Preset::ALL {
        presets = presets.child(action(preset.key(), move || {
            {
                let mut s = state();
                s.control.set_preset(preset);
                s.prefs.preset = preset.key().into();
                publish(&mut s);
            }
            save();
        }));
    }
    scroll().flex_1().child(
        view()
            .px_4()
            .gap_2()
            .child(heading("Soundscape"))
            .child(action("Play / pause", || {
                let mut s = state();
                if s.snap.error.is_none() && !s.snap.device.is_empty() {
                    s.control.set_playing(!s.control.playing());
                    publish(&mut s);
                }
            }))
            .child(
                text(move || {
                    snap.with(|s| {
                        s.error
                            .clone()
                            .unwrap_or_else(|| format!("{} · {}", s.report, s.device))
                    })
                })
                .text_sm(),
            )
            .child(presets)
            .child(knob("Volume", Control::volume, Control::set_volume))
            .child(
                canvas(move || {
                    let current = snap.get();
                    let mut p = Painter::new().fill_color(color(nsh_base::theme().primary));
                    if current.playing {
                        for (index, band) in current.bands.iter().enumerate() {
                            let height = band.clamp(0., 1.) * 40.;
                            p = p.fill_rect(index as f32 * 8., 40. - height, 6., height.max(1.));
                        }
                    }
                    p
                })
                .h(px(40.)),
            )
            .child(action("Show / hide mixer", || {
                {
                    let mut s = state();
                    s.prefs.mixer = !s.prefs.mixer;
                    publish(&mut s);
                }
                save();
            }))
            .child(dynamic(move || {
                mixer.get().then(|| {
                    let mut out = view();
                    for layer in LayerName::ALL {
                        out = out.child(knob(
                            layer.label(),
                            move |c| c.layer(layer),
                            move |c, v| c.set_layer(layer, v),
                        ));
                    }
                    out
                })
            })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_and_switching_seats_does_not_start_audio_and_closing_pauses() {
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;
        let control = Arc::new(Control::new());
        set_parts(
            control.clone(),
            Arc::new(Visual::new()),
            SoundscapePrefs::default(),
        );
        set_device(Ok("Synthetic audio device".into()));
        show(true);
        assert!(!control.playing());
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(nsh_core::element::Root::of::<SoundscapeScreen>())
        })
        .unwrap();
        app.wait_for("text:Soundscape").unwrap();
        app.press("text:Play / pause").unwrap();
        assert!(control.playing());
        show(false);
        assert!(control.playing(), "switching seats keeps playback");
        pause();
        assert!(!control.playing(), "closing the panel pauses playback");
        show(false);
    }
}
