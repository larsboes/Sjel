//! The phone's soundscape audio: the two halves the drawer holds, and the render the host's device
//! pulls from.
//!
//! The engine is platform-free (`sjel_gpui_core::soundscape`) and is the same one the Mac plays
//! (`apps/gpui/src/audio.rs`); on the phone the device belongs to Swift, because `AVAudioEngine` and
//! the audio session are Apple's own and are what `host/Audio.swift` already knows how to set up. The
//! contract between them is one call: the host asks for N frames into two buffers, and this renders
//! exactly N. The device's callback is the clock, so nothing here needs a timer.
//!
//! Two halves are kept apart on purpose of lifetime, not of design: the conductor ([`Control`]) and
//! the report ([`Visual`]) exist from the first time the drawer is built, because the drawer must be
//! able to show a preset and a volume before there is any sound; the [`Engine`] is created on the
//! first render, because it needs the device's sample rate and nothing else.

use std::sync::{Arc, Mutex, OnceLock};

use sjel_gpui_core::soundscape::{Control, Engine, Visual};

/// `SJEL_GPUI_NO_AUDIO=1` keeps the app off the device, the way the Mac's own switch does: a
/// screenshot run has no business taking the speakers, and the drawer still opens and still writes to
/// the conductor.
const NO_AUDIO: &str = "SJEL_GPUI_NO_AUDIO";

/// The conductor and the report, created once and shared by the drawer and the device.
static PARTS: OnceLock<(Arc<Control>, Arc<Visual>)> = OnceLock::new();

/// The engine, which needs the device's rate and is therefore born on the first render.
static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

/// The drawer's two halves. Safe to call before any device exists.
pub fn parts() -> (Arc<Control>, Arc<Visual>) {
    PARTS
        .get_or_init(|| (Arc::new(Control::new()), Arc::new(Visual::new())))
        .clone()
}

/// True when a device may be opened. The host asks before it starts one.
pub fn audio_allowed() -> bool {
    std::env::var_os(NO_AUDIO).is_none()
}

/// Why there is no sound, when the switch above is the reason.
pub fn no_audio_reason() -> Option<String> {
    (!audio_allowed()).then(|| format!("{NO_AUDIO} is set: no output device"))
}

/// Asks the host to open its audio device and start pulling frames.
///
/// Answers the same sentence the drawer shows: the device's name and rate, or why there is none. The
/// host implements it (`host/Audio.swift`), because the session and the engine are its own.
///
/// # Safety
/// `sjel_ios_audio_start` is declared by the host and called with no arguments; it returns a
/// NUL-terminated string it owns for the duration of the call.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // Declaring the host's symbol is unsafe; the host owns its definition.
unsafe extern "C" {
    fn sjel_ios_audio_start() -> *const std::ffi::c_char;
}

/// Opens the device, if this build is allowed one, and answers what to say about it.
pub fn start() -> Result<String, String> {
    if let Some(reason) = no_audio_reason() {
        return Err(reason);
    }
    #[cfg(target_os = "ios")]
    #[allow(unsafe_code)] // One call into the host, which links this symbol by name.
    // SAFETY: the host's contract is a NUL-terminated string valid for the call
    // (`host/Audio.swift`); it is read here and not kept.
    let reply = unsafe {
        let text = sjel_ios_audio_start();
        if text.is_null() {
            return Err("The audio device did not answer.".to_owned());
        }
        std::ffi::CStr::from_ptr(text)
            .to_str()
            .unwrap_or_default()
            .to_owned()
    };
    #[cfg(not(target_os = "ios"))]
    let reply = String::from("err:this build has no audio device");

    // `ok:<device>:<rate>` or `err:<why>`: one string, so nothing has to be marshalled twice.
    match reply.split_once(':') {
        Some(("ok", rest)) => Ok(rest.replace(":", " · ")),
        Some(("err", why)) => Err(why.to_owned()),
        _ => Err(reply),
    }
}

/// Fills `frames` samples of each channel from the engine.
///
/// Called on the audio thread. A lock is taken for the length of one block: the drawer never holds it,
/// because it only writes to the atomics in [`Control`] and reads the report in [`Visual`].
///
/// # Safety
/// `left` and `right` must each point at `frames` writable `f32`s, which is what an
/// `AVAudioSourceNode`'s two buffers are (`host/Audio.swift`).
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // Writing caller-owned buffers is unsafe; the contract is stated above.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sjel_ios_soundscape_render(
    sample_rate: f64,
    left: *mut f32,
    right: *mut f32,
    frames: u32,
) {
    if left.is_null() || right.is_null() || frames == 0 {
        return;
    }
    let frames = frames as usize;
    let (control, visual) = parts();
    let Ok(mut guard) = ENGINE.lock() else {
        return;
    };
    let engine = guard.get_or_insert_with(|| Engine::new(sample_rate.max(8_000.) as f32));

    // The engine renders interleaved, which is what the device formats do on the Mac; the phone's
    // node hands over two separate buffers, so the block is rendered once and split once.
    let mut scratch = vec![0.0_f32; frames * 2];
    engine.render(&mut scratch, 2, &control, &visual);
    // SAFETY: the caller promised `frames` writable samples in each buffer.
    unsafe {
        let left = std::slice::from_raw_parts_mut(left, frames);
        let right = std::slice::from_raw_parts_mut(right, frames);
        for frame in 0..frames {
            left[frame] = scratch[frame * 2];
            right[frame] = scratch[frame * 2 + 1];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drawers_halves_exist_before_any_device_does() {
        let (control, visual) = parts();
        // A fresh conductor plays nothing and has a preset the drawer can name.
        assert!(!control.playing());
        assert!(control.volume() > 0.);
        assert_eq!(visual.voices(), 0);
        // The same halves come back, which is what lets the drawer and the engine agree.
        let (same_control, _) = parts();
        control.set_preset(sjel_gpui_core::soundscape::Preset::Sleep);
        assert_eq!(
            same_control.preset(),
            sjel_gpui_core::soundscape::Preset::Sleep
        );
    }
}
