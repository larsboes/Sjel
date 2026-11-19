//! Shared native synthesizer, listening modes and session controls.
//!
//! The conductor owns playback intent and the timed session. Each native audio
//! device drives this engine against its own sample clock.

pub mod analyser;
pub mod control;
pub mod engine;
pub mod fx;
pub mod music;
pub mod rng;
mod session;
pub mod steps;
pub mod synth;

pub use analyser::Analyser;
pub use control::{Control, Visual, BANDS};
pub use engine::Engine;
pub use music::{ArcPhase, Arrangement, LayerName, Layers, ParamName, Params, Preset};
pub use rng::{format_seed, parse_seed, Mulberry32};
pub use session::{ListeningContext, Scenario};
pub use steps::{play_step, Events, Sequence};
pub use synth::{VoiceSpec, Voices};

/// The window's own reading of a running session: everything the transport and the mixer need
/// without touching the conductor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    pub preset: Preset,
    pub playing: bool,
    pub phase: ArcPhase,
    pub energy: f32,
    pub voices: u32,
    pub step: u32,
}

impl Playback {
    /// Read one from the report the audio thread publishes.
    pub fn read(visual: &Visual, playing: bool) -> Self {
        Self {
            preset: visual.preset(),
            playing,
            phase: visual.phase(),
            energy: visual.energy(),
            voices: visual.voices(),
            step: visual.step(),
        }
    }

    /// "playing" is the operator's intent and "sounding" is the device's state; a transport that
    /// showed only one of them would lie whenever the two disagree.
    pub fn describe(&self) -> String {
        let state = if self.playing { "playing" } else { "paused" };
        if self.preset.scape().is_some() {
            return format!(
                "{} · {} · {} voices · {state}",
                self.preset.key(),
                self.phase.label(),
                self.voices
            );
        }
        format!("{} · {} voices · {state}", self.preset.key(), self.voices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playback_describes_what_is_actually_sounding() {
        let visual = Visual::new();
        // A scape reports its arc phase; a written performance has no arc to report.
        visual.write_state(Preset::Focus, ArcPhase::Ebb, 0.5, 4, 9);
        let playback = Playback::read(&visual, true);
        assert_eq!(playback.preset, Preset::Focus);
        assert!(
            playback.describe().contains("ebb"),
            "{}",
            playback.describe()
        );
        assert!(playback.describe().contains("4 voices"));
        assert!(playback.describe().contains("playing"));

        visual.write_state(Preset::Lofi, ArcPhase::Bloom, 0.5, 4, 9);
        let lofi = Playback::read(&visual, true);
        assert_eq!(lofi.describe(), "lofi · 4 voices · playing");

        // The transport says "paused" for the same synth when the operator stopped it.
        let paused = Playback::read(&visual, false);
        assert!(
            paused.describe().contains("paused"),
            "{}",
            paused.describe()
        );
    }
}
