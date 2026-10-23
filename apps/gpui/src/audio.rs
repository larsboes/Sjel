//! The audio device, and the one thread that owns the synth.
//!
//! Everything above this file is platform-free (`sjel_gpui_core::soundscape`); this is where the
//! Mac's own output is handed to it. The device callback *is* the clock: cpal asks for a block of
//! frames, the engine renders exactly that many, and the sequencer advances inside the loop. No
//! timer, no lookahead, no thread of our own.
//!
//! The stream is not `Send`, which is why the whole thing lives in a `Global` on the main thread
//! rather than inside a view: closing the soundscape window must not cut the music, and reopening
//! it must find the same session playing rather than start a second one.

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use gpui_kit::{App, Global};
use sjel_gpui_core::soundscape::{Control, Engine, Visual};

/// The device and its two channels. `output` is `None` when there is no device or the stream
/// could not be built, and `error` then says which -- a window that cannot make sound has to say
/// why rather than look broken.
struct Device {
    /// Held for its side effect: dropping the stream stops the device. Named with an underscore
    /// because nothing ever reads it, and a `Stream` that is dropped is silent.
    _stream: Stream,
    name: String,
    sample_rate: f32,
}

/// The app's soundscape audio, created the first time a window asks for it.
pub struct Audio {
    pub control: Arc<Control>,
    pub visual: Arc<Visual>,
    device: Option<Device>,
    error: Option<String>,
}

impl Global for Audio {}

impl Audio {
    fn open() -> Self {
        let control = Arc::new(Control::new());
        let visual = Arc::new(Visual::new());
        match start(control.clone(), visual.clone()) {
            Ok(device) => Self {
                control,
                visual,
                device: Some(device),
                error: None,
            },
            Err(error) => Self {
                control,
                visual,
                device: None,
                error: Some(error),
            },
        }
    }

    /// The device's own name and rate, for the window's status line.
    pub fn device_name(&self) -> Option<&str> {
        self.device.as_ref().map(|device| device.name.as_str())
    }

    pub fn sample_rate(&self) -> Option<f32> {
        self.device.as_ref().map(|device| device.sample_rate)
    }

    /// Why there is no sound, when there is no sound.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// The app's audio session, started on first use.
///
/// Not started at launch on purpose: taking the audio device is a visible act (the Mac switches
/// its output route, the menu bar shows it) and belongs to opening the soundscape, not to typing
/// in a note editor.
pub fn audio(cx: &mut App) -> &Audio {
    if cx.try_global::<Audio>().is_none() {
        cx.set_global(Audio::open());
    }
    cx.global::<Audio>()
}

fn start(control: Arc<Control>, visual: Arc<Visual>) -> Result<Device, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "This Mac reports no audio output device.".to_string())?;
    let name = device
        .name()
        .unwrap_or_else(|_| "the default device".into());
    let supported = device
        .default_output_config()
        .map_err(|error| format!("The device would not say what it can play: {error}"))?;
    let sample_format = supported.sample_format();
    let mut config: StreamConfig = supported.config();
    // A block of 512 frames is about 5 ms at 96 kHz. It is well inside every device's range on
    // this platform, and small enough that a slider move is heard as you drag it.
    if let cpal::SupportedBufferSize::Range { min, max } = supported.buffer_size() {
        config.buffer_size = cpal::BufferSize::Fixed((*min).max(512).min(*max));
    }
    let sample_rate = config.sample_rate.0 as f32;

    let engine = Engine::new(sample_rate);
    let stream = match sample_format {
        SampleFormat::F32 => build::<f32>(&device, &config, engine, control, visual),
        SampleFormat::I16 => build::<i16>(&device, &config, engine, control, visual),
        SampleFormat::U16 => build::<u16>(&device, &config, engine, control, visual),
        other => Err(format!(
            "The device plays {other}, which this build does not render."
        )),
    }?;
    stream.play().map_err(|error| {
        format!("The device accepted the stream but would not start it: {error}")
    })?;

    Ok(Device {
        _stream: stream,
        name,
        sample_rate,
    })
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut engine: Engine,
    control: Arc<Control>,
    visual: Arc<Visual>,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    // One scratch buffer for the whole session. It is grown once, on the first callback whose
    // block is larger than the last, so the steady state allocates nothing -- which is the whole
    // reason the engine renders f32 and the device's own format is converted afterwards.
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream::<T, _, _>(
            config,
            move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
                if scratch.len() != output.len() {
                    scratch.resize(output.len(), 0.);
                }
                engine.render(&mut scratch, channels, &control, &visual);
                for (slot, sample) in output.iter_mut().zip(scratch.iter()) {
                    *slot = T::from_sample(*sample);
                }
            },
            move |error| {
                // A device error is reported once by cpal and cannot re-enter the UI: say it on
                // stderr so a terminal run shows it, and let the window keep its own state.
                eprintln!("soundscape: the audio device reported {error}");
            },
            None,
        )
        .map_err(|error| format!("The device would not accept a stream: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A build of this app has to be able to open the default device on this machine; the
    /// stream is only opened, not played, so nothing is audible during a test run.
    #[test]
    fn the_default_device_can_be_opened() {
        let control = Arc::new(Control::new());
        let visual = Arc::new(Visual::new());
        match start(control, visual) {
            Ok(device) => {
                assert!(device.sample_rate > 8_000., "{}", device.sample_rate);
                assert!(!device.name.is_empty());
            }
            Err(error) => panic!("no output device: {error}"),
        }
    }
}
