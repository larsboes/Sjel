//! Native soundscape controls over the same audio-thread atomics as the desktop drawer.
use crate::theme::SoundscapePrefs;

mod phone;
pub use phone::PhoneSoundscape;
use sjel_gpui_core::{
    shell,
    soundscape::{Control, Playback, Preset, Visual, BANDS},
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_does_not_start_audio_and_closing_pauses() {
        let control = Arc::new(Control::new());
        set_parts(
            control.clone(),
            Arc::new(Visual::new()),
            SoundscapePrefs::default(),
        );
        set_device(Ok("Synthetic audio device".into()));
        show(true);
        assert!(!control.playing());
        control.set_playing(true);
        show(false);
        assert!(control.playing(), "switching seats keeps playback");
        pause();
        assert!(!control.playing(), "closing the panel pauses playback");
    }
}
